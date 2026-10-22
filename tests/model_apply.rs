use serde_json::{Value, json};
use sm::{
    apply::{apply_writes, plan_apply},
    model::{Catalog, Client, OwnedWrite, Resource, ResourceContent, Scope, WriteBody},
    state::State,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};
use tempfile::tempdir;

fn resource(value: Value) -> Resource {
    serde_json::from_value(value).unwrap()
}
fn skill() -> Resource {
    resource(
        json!({"id":"review","kind":"skill","origin":"self","scope":"user","project":null,"payload":"resources/review"}),
    )
}
fn owned(path: &Path, bytes: &[u8]) -> OwnedWrite {
    OwnedWrite::new(
        "review",
        path,
        WriteBody::File {
            bytes: bytes.to_vec(),
            executable: false,
        },
    )
}
fn apply(writes: Vec<OwnedWrite>, state: &State) -> sm::model::ApplyReport {
    apply_writes(
        &sm::model::ApplyPlan {
            writes,
            results: BTreeMap::new(),
        },
        state,
    )
    .unwrap()
}

fn migrated_state(path: &Path, writes: &[OwnedWrite]) -> State {
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch(
        "CREATE TABLE ownership (resource TEXT,path TEXT,value TEXT,PRIMARY KEY(resource,path));",
    )
    .unwrap();
    for write in writes {
        db.execute(
            "INSERT INTO ownership VALUES (?1,?2,?3)",
            rusqlite::params![
                write.resource_id,
                write.path.to_string_lossy(),
                serde_json::to_string(write).unwrap()
            ],
        )
        .unwrap();
    }
    drop(db);
    State::open(path.to_path_buf()).unwrap()
}

#[test]
fn migrated_files_allow_updates_mode_changes_and_removal() {
    let temp = tempdir().unwrap();
    let target = temp.path().join("script");
    fs::write(&target, [0, 255, 128]).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    let state = migrated_state(
        &temp.path().join("state.db"),
        &[owned(&target, &[0, 255, 128])],
    );
    let saved = serde_json::to_value(state.owned().unwrap()).unwrap();
    assert_eq!(saved[0]["body"]["kind"], "file_fingerprint");
    for (bytes, executable) in [(vec![0, 255, 128], true), (b"changed".to_vec(), false)] {
        let write = OwnedWrite::new(
            "review",
            &target,
            WriteBody::File {
                bytes: bytes.clone(),
                executable,
            },
        );
        assert!(apply(vec![write], &state).ok());
        assert_eq!(fs::read(&target).unwrap(), bytes);
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o111 != 0,
            executable
        );
    }
    assert!(apply(vec![], &state).ok());
    assert!(!target.exists());
    assert!(state.owned().unwrap().is_empty());
}

#[test]
fn migrated_files_preserve_local_changes_missing_targets_and_symlinks() {
    for change in ["content", "missing", "symlink"] {
        let temp = tempdir().unwrap();
        let target = temp.path().join("file");
        let outside = temp.path().join("outside");
        fs::write(&target, "managed").unwrap();
        let state = migrated_state(&temp.path().join("state.db"), &[owned(&target, b"managed")]);
        match change {
            "content" => fs::write(&target, "local edit").unwrap(),
            "missing" => fs::remove_file(&target).unwrap(),
            _ => {
                fs::remove_file(&target).unwrap();
                fs::write(&outside, "outside data").unwrap();
                symlink(&outside, &target).unwrap();
            }
        }
        let before = serde_json::to_value(state.owned().unwrap()).unwrap();
        assert!(
            !apply(vec![owned(&target, b"new")], &state).ok(),
            "{change}"
        );
        assert!(!apply(vec![], &state).ok(), "{change}");
        assert_eq!(
            serde_json::to_value(state.owned().unwrap()).unwrap(),
            before
        );
        match change {
            "content" => assert_eq!(fs::read(&target).unwrap(), b"local edit"),
            "missing" => assert!(!target.exists()),
            _ => {
                assert!(
                    fs::symlink_metadata(&target)
                        .unwrap()
                        .file_type()
                        .is_symlink()
                );
                assert_eq!(fs::read(outside).unwrap(), b"outside data");
            }
        }
    }
}

#[test]
fn migrated_file_conflicts_block_every_shared_owner_before_cleanup() {
    let temp = tempdir().unwrap();
    let a_old = temp.path().join("a-old");
    let b_old = temp.path().join("b-old");
    let config = temp.path().join("hooks.json");
    fs::write(&a_old, "managed").unwrap();
    fs::write(&b_old, "managed").unwrap();
    let mut a = owned(&a_old, b"managed");
    a.resource_id = "a".into();
    let mut b = owned(&b_old, b"managed");
    b.resource_id = "b".into();
    let fragment = |id: &str| {
        json!({"hooks":{"Stop":[{"command":id}]}})
            .as_object()
            .unwrap()
            .clone()
    };
    let hook = |id: &str| OwnedWrite::new(id, &config, WriteBody::Json(fragment(id)));
    fs::write(
        &config,
        serde_json::to_vec(&json!({"hooks":{"Stop":[{"command":"a"},{"command":"b"}]}})).unwrap(),
    )
    .unwrap();
    let state = migrated_state(&temp.path().join("state.db"), &[a, b, hook("a"), hook("b")]);
    fs::write(&a_old, "local edit").unwrap();
    let before = fs::read(&config).unwrap();
    let mut a = owned(&temp.path().join("a-new"), b"new");
    a.resource_id = "a".into();
    let mut b = owned(&temp.path().join("b-new"), b"new");
    b.resource_id = "b".into();
    let report = apply(vec![a, b, hook("a"), hook("b")], &state);
    assert!(!report.ok());
    for id in ["a", "b"] {
        assert_eq!(report.results[id].status, sm::model::ApplyStatus::Conflict);
    }
    assert_eq!(fs::read(&a_old).unwrap(), b"local edit");
    assert_eq!(fs::read(&b_old).unwrap(), b"managed");
    assert_eq!(fs::read(&config).unwrap(), before);
    assert!(!temp.path().join("a-new").exists());
    assert!(!temp.path().join("b-new").exists());
    assert_eq!(state.owned().unwrap().len(), 4);
}

#[test]
fn skill_is_client_independent_and_installed_once() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("resources/review")).unwrap();
    fs::write(root.join("resources/review/SKILL.md"), "Portable").unwrap();
    let entry = skill();
    assert!(matches!(entry.content, ResourceContent::Skill { .. }));
    assert_eq!(entry.targets().len(), 2);
    let catalog = Catalog {
        projects: BTreeMap::new(),
        upstreams: BTreeMap::new(),
        resources: vec![entry],
    };
    let plan = plan_apply(
        &catalog,
        &BTreeMap::from([("review".into(), root.to_path_buf())]),
        &root.join("home"),
    );
    assert!(plan.results.is_empty());
    assert_eq!(plan.writes.len(), 1);
    assert_eq!(
        plan.writes[0].path,
        root.join("home/.agents/skills/review/SKILL.md")
    );
}

#[test]
fn invalid_metadata_cannot_enter_the_domain() {
    let base = json!({"id":"review","kind":"skill","origin":"self","scope":"user","project":null,"payload":"resources/review"});
    for (key, value) in [
        ("scope", json!("system")),
        ("id", json!("../escape")),
        ("kind", json!("plugin")),
        ("kind", json!("bundle")),
        ("payload", json!("../outside")),
        ("payload", json!("/absolute")),
        ("agents", json!(["codex"])),
    ] {
        let mut input = base.clone();
        input[key] = value;
        assert!(serde_json::from_value::<Resource>(input).is_err(), "{key}");
    }
    let mut input = base;
    input["scope"] = json!("project");
    assert!(serde_json::from_value::<Resource>(input).is_err());
}

#[test]
fn project_directory_expands_under_each_home() {
    let entry = resource(
        json!({"id":"review","kind":"skill","origin":"self","scope":"project","project":"app","payload":"resources/review"}),
    );
    let catalog = Catalog {
        projects: BTreeMap::from([("app".into(), "~/repo/app".into())]),
        upstreams: BTreeMap::new(),
        resources: vec![entry.clone()],
    };
    for home in [Path::new("/tmp/one"), Path::new("/tmp/two")] {
        assert_eq!(
            catalog.scope_root(&entry, home).unwrap(),
            home.join("repo/app")
        );
    }
    assert_eq!(entry.scope, Scope::Project);
}

#[test]
fn rules_preserve_unmanaged_text_and_remove_only_own_block() {
    let temp = tempdir().unwrap();
    let target = temp.path().join("AGENTS.md");
    let state = State::open(temp.path().join("state.db")).unwrap();
    fs::write(&target, "Existing rules\n").unwrap();
    assert!(
        apply(
            vec![OwnedWrite::new(
                "review",
                &target,
                WriteBody::Markdown(b"New rules".to_vec())
            )],
            &state
        )
        .ok()
    );
    assert!(
        fs::read_to_string(&target)
            .unwrap()
            .starts_with("Existing rules\n")
    );
    assert!(apply(vec![], &state).ok());
    assert_eq!(fs::read_to_string(target).unwrap(), "Existing rules\n");
}

#[test]
fn mcp_merge_preserves_credentials_comments_and_unowned_entries() {
    let temp = tempdir().unwrap();
    let target = temp.path().join("config.toml");
    let state = State::open(temp.path().join("state.db")).unwrap();
    fs::write(&target,"# Keep this comment\n[model_providers.local]\napi_key=\"local-secret\"\n[mcp_servers.keep]\ncommand=\"keep\"\n").unwrap();
    let body = json!({"mcp_servers":{"docs":{"command":"docs"}}})
        .as_object()
        .unwrap()
        .clone();
    assert!(
        apply(
            vec![OwnedWrite::new("review", &target, WriteBody::Toml(body))],
            &state
        )
        .ok()
    );
    let text = fs::read_to_string(&target).unwrap();
    assert!(text.contains("# Keep this comment"));
    assert!(text.contains("local-secret"));
    let parsed: toml_edit::DocumentMut = text.parse().unwrap();
    assert_eq!(
        parsed["mcp_servers"]["docs"]["command"].as_str(),
        Some("docs")
    );
    assert!(apply(vec![], &state).ok());
    let text = fs::read_to_string(target).unwrap();
    assert!(text.contains("mcp_servers.keep"));
    assert!(!text.contains("mcp_servers.docs"));
}

#[test]
fn hooks_merge_without_adopting_other_handlers() {
    let temp = tempdir().unwrap();
    let target = temp.path().join("hooks.json");
    let state = State::open(temp.path().join("state.db")).unwrap();
    let keep = json!({"hooks":[{"type":"command","command":"keep"}]});
    fs::write(
        &target,
        json!({"hooks":{"Stop":[keep.clone()]}}).to_string(),
    )
    .unwrap();
    let own = json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":"managed"}]}]}})
        .as_object()
        .unwrap()
        .clone();
    assert!(
        apply(
            vec![OwnedWrite::new("review", &target, WriteBody::Json(own))],
            &state
        )
        .ok()
    );
    assert!(apply(vec![], &state).ok());
    let final_value: Value = serde_json::from_str(&fs::read_to_string(target).unwrap()).unwrap();
    assert_eq!(final_value["hooks"]["Stop"], json!([keep]));
}

#[test]
fn local_edits_and_unmanaged_targets_are_preserved() {
    let temp = tempdir().unwrap();
    let target = temp.path().join("file");
    let state = State::open(temp.path().join("state.db")).unwrap();
    fs::write(&target, "mine").unwrap();
    assert!(!apply(vec![owned(&target, b"managed")], &state).ok());
    assert_eq!(fs::read(&target).unwrap(), b"mine");
    fs::remove_file(&target).unwrap();
    assert!(apply(vec![owned(&target, b"managed")], &state).ok());
    fs::write(&target, "local edit").unwrap();
    assert!(!apply(vec![owned(&target, b"new")], &state).ok());
    assert!(!apply(vec![], &state).ok());
    assert_eq!(fs::read(target).unwrap(), b"local edit");
}

#[test]
fn move_conflicts_keep_old_projection_and_success_removes_it() {
    let temp = tempdir().unwrap();
    let old = temp.path().join("old");
    let new = temp.path().join("new");
    let state = State::open(temp.path().join("state.db")).unwrap();
    assert!(apply(vec![owned(&old, b"managed")], &state).ok());
    fs::write(&new, "someone else").unwrap();
    assert!(!apply(vec![owned(&new, b"new")], &state).ok());
    assert_eq!(fs::read(&old).unwrap(), b"managed");
    fs::remove_file(&new).unwrap();
    assert!(apply(vec![owned(&new, b"new")], &state).ok());
    assert!(!old.exists());
    assert_eq!(fs::read(new).unwrap(), b"new");
}

#[test]
fn runtime_failure_blocks_shared_configuration_and_old_asset_cleanup() {
    for broken in ["a", "b"] {
        let temp = tempdir().unwrap();
        let home = temp.path().join("home");
        let state = State::open(temp.path().join("state.db")).unwrap();
        let a_old = home.join(".codex/hooks/sm/a/old.sh");
        let b_old = home.join(".codex/hooks/sm/b/old.sh");
        let writes = |a: &Path, b: &Path| {
            [("a", a), ("b", b)]
                .into_iter()
                .flat_map(|(id, path)| {
                    let mut script = owned(path, b"managed");
                    script.resource_id = id.into();
                    let mut hook =
                        sm::providers::collector(Client::Codex, &home, path, temp.path());
                    hook.resource_id = id.into();
                    [script, hook]
                })
                .collect()
        };
        assert!(apply(writes(&a_old, &b_old), &state).ok());
        let config = fs::read(home.join(".codex/hooks.json")).unwrap();
        let blocker = home.join(format!(".codex/hooks/sm/{broken}/block"));
        fs::write(&blocker, "local file").unwrap();
        let a_new = if broken == "a" {
            blocker.join("run.sh")
        } else {
            home.join(".codex/hooks/sm/a/new.sh")
        };
        let b_new = if broken == "b" {
            blocker.join("run.sh")
        } else {
            home.join(".codex/hooks/sm/b/new.sh")
        };
        let report = apply(writes(&a_new, &b_new), &state);
        assert!(!report.ok());
        assert_eq!(fs::read(&a_old).unwrap(), b"managed");
        assert_eq!(fs::read(&b_old).unwrap(), b"managed");
        assert_ne!(report.results["b"].status, sm::model::ApplyStatus::Applied);
        assert_ne!(report.results["a"].status, sm::model::ApplyStatus::Applied);
        assert_eq!(fs::read(home.join(".codex/hooks.json")).unwrap(), config);
        assert!(
            state
                .owned()
                .unwrap()
                .iter()
                .any(|write| write.path == b_old)
        );
        fs::remove_file(blocker).unwrap();
        assert!(apply(writes(&a_new, &b_new), &state).ok());
        assert!(!a_old.exists());
        assert!(!b_old.exists());
        assert_eq!(fs::read(b_new).unwrap(), b"managed");
    }
}

#[test]
fn malformed_shared_configuration_does_not_block_unrelated_resources() {
    let temp = tempdir().unwrap();
    let target = temp.path().join("hooks.json");
    let state = State::open(temp.path().join("state.db")).unwrap();
    fs::write(&target, "{\"hooks\":[]}").unwrap();
    let body = json!({"hooks":{"Stop":[]}}).as_object().unwrap().clone();
    let good = temp.path().join("good");
    let mut second = owned(&good, b"good");
    second.resource_id = "other".into();
    assert!(
        !apply(
            vec![
                OwnedWrite::new("review", &target, WriteBody::Json(body)),
                second
            ],
            &state
        )
        .ok()
    );
    assert_eq!(fs::read(good).unwrap(), b"good");
    assert_eq!(fs::read_to_string(target).unwrap(), "{\"hooks\":[]}");
}

#[test]
fn managed_scripts_apply_mode_changes_even_when_bytes_match() {
    let temp = tempdir().unwrap();
    let target = temp.path().join("script");
    let state = State::open(temp.path().join("state.db")).unwrap();
    assert!(apply(vec![owned(&target, b"body")], &state).ok());
    for (bytes, executable) in [
        (b"body".as_slice(), true),
        (b"changed".as_slice(), false),
        (b"again".as_slice(), true),
    ] {
        assert!(
            apply(
                vec![OwnedWrite::new(
                    "review",
                    &target,
                    WriteBody::File {
                        bytes: bytes.to_vec(),
                        executable
                    }
                )],
                &state
            )
            .ok()
        );
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o100 != 0,
            executable
        );
    }
}

#[test]
fn target_and_source_symlinks_never_escape() {
    let temp = tempdir().unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let link = temp.path().join("link");
    symlink(&outside, &link).unwrap();
    let state = State::open(temp.path().join("state.db")).unwrap();
    assert!(!apply(vec![owned(&link.join("file"), b"bad")], &state).ok());
    assert!(!outside.join("file").exists());
    let payload = temp.path().join("resources/review");
    fs::create_dir_all(&payload).unwrap();
    symlink("/etc/passwd", payload.join("SKILL.md")).unwrap();
    assert!(
        sm::providers::render(&skill(), Client::Codex, &payload, &temp.path().join("home"))
            .is_err()
    );
}

#[test]
fn native_types_render_expected_user_targets_and_project_limit() {
    let cases = [
        ("rule", "codex", "AGENTS.md", "Rules", ".codex/AGENTS.md"),
        ("rule", "kimi", "AGENTS.md", "Rules", ".kimi-code/AGENTS.md"),
        (
            "agent",
            "codex",
            "agent.toml",
            "name='review'",
            ".codex/agents/review.toml",
        ),
        (
            "agent",
            "kimi",
            "agent.md",
            "Role",
            ".kimi-code/agents/review.md",
        ),
        (
            "mcp",
            "codex",
            "config.toml",
            "[mcp_servers.docs]\ncommand='docs'",
            ".codex/config.toml",
        ),
        (
            "mcp",
            "kimi",
            "mcp.json",
            "{\"mcpServers\":{\"docs\":{\"command\":\"docs\"}}}",
            ".kimi-code/mcp.json",
        ),
        (
            "hook",
            "codex",
            "hooks.json",
            "{\"hooks\":{\"Stop\":[{\"hooks\":[{\"type\":\"command\",\"command\":\"true\"}]}]}}",
            ".codex/hooks.json",
        ),
        (
            "hook",
            "kimi",
            "config.toml",
            "[[hooks]]\nevent='Stop'\ncommand='true'",
            ".kimi-code/config.toml",
        ),
    ];
    for (kind, client, name, text, target) in cases {
        let temp = tempdir().unwrap();
        fs::write(temp.path().join(name), text).unwrap();
        let home = temp.path().join("home");
        let entry = resource(
            json!({"id":"review","kind":kind,"origin":"self","scope":"user","project":null,"agents":[client],"payloads":{client:"resources/review"}}),
        );
        let client: Client = serde_json::from_value(json!(client)).unwrap();
        let writes = sm::providers::render(&entry, client, temp.path(), &home).unwrap();
        assert!(
            writes.iter().any(|w| w.path == home.join(target)),
            "{kind}/{client:?}"
        );
    }
    let entry = resource(
        json!({"id":"review","kind":"hook","origin":"self","scope":"project","project":"app","agents":["kimi"],"payloads":{"kimi":"resources/review"}}),
    );
    let temp = tempdir().unwrap();
    assert!(sm::providers::render(&entry, Client::Kimi, temp.path(), temp.path()).is_err());
}

#[test]
fn kimi_hook_update_retains_other_handler_comments() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let state = State::open(temp.path().join("state.db")).unwrap();
    fs::write(
        &path,
        "# local hook\n[[hooks]]\nevent='Stop'\ncommand='keep' # handler comment\n",
    )
    .unwrap();
    let config = json!({"hooks":[{"event":"Stop","command":"managed"}]})
        .as_object()
        .unwrap()
        .clone();
    let report = apply(
        vec![OwnedWrite::new("review", &path, WriteBody::Toml(config))],
        &state,
    );
    assert!(report.ok(), "{report:?}");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("handler comment"));
    assert!(text.contains("keep"));
    assert!(apply(vec![], &state).ok());
    let text = fs::read_to_string(path).unwrap();
    assert!(text.contains("handler comment"));
    assert!(!text.contains("managed"));
}

#[test]
fn native_mcp_payloads_contain_one_server_identity() {
    for (client, filename, text) in [
        (
            "codex",
            "config.toml",
            "[mcp_servers.one]\ncommand='a'\n[mcp_servers.two]\ncommand='b'\n",
        ),
        (
            "kimi",
            "mcp.json",
            "{\"mcpServers\":{\"one\":{\"command\":\"a\"},\"two\":{\"command\":\"b\"}}}",
        ),
    ] {
        let temp = tempdir().unwrap();
        fs::write(temp.path().join(filename), text).unwrap();
        let entry = resource(
            json!({"id":"review","kind":"mcp","origin":"self","scope":"user","agents":[client],"payloads":{client:"resources/review"}}),
        );
        let client = serde_json::from_value(json!(client)).unwrap();
        assert!(
            sm::providers::render(&entry, client, temp.path(), &temp.path().join("home")).is_err()
        );
    }
}
