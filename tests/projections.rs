use serde_json::{Value, json};
use sm::{
    apply::{apply_writes, plan_apply},
    model::{ApplyPlan, ApplyStatus, Catalog, Client, Resource, ResourceContent, Scope, WriteBody},
    state::State,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};

fn resource(kind: &str, id: &str, entries: Value) -> Resource {
    serde_json::from_value(json!({
        "id":id,"kind":kind,"origin":"self","scope":"user","project":null,
        "files":{"root":format!("resources/{id}/files"),"entries":entries}
    }))
    .unwrap()
}
fn rule(entries: Value) -> Resource {
    resource("rule", "rule", entries)
}
fn render(
    resource: &Resource,
    payload: &Path,
    home: &Path,
) -> anyhow::Result<Vec<sm::model::OwnedWrite>> {
    sm::providers::render(resource, Client::Codex, payload, home)
}
fn apply(writes: Vec<sm::model::OwnedWrite>, state: &State) -> sm::model::ApplyReport {
    apply_writes(
        &ApplyPlan {
            writes,
            results: BTreeMap::new(),
        },
        state,
    )
    .unwrap()
}

#[test]
fn typed_projection_serializes_actual_kind_and_exact_source_files() {
    let resource = rule(json!([
        {"target":"AGENTS.md","source":"rule.md"},
        {"target":".codex/rules/rule.md","source":"rule.md"}
    ]));
    assert!(matches!(resource.content, ResourceContent::Files { .. }));
    assert_eq!(resource.targets().len(), 2);
    assert_eq!(
        resource.paths(),
        ["resources/rule/files/rule.md".into()].into()
    );
    let value = serde_json::to_value(&resource).unwrap();
    assert_eq!(value["kind"], "rule");
    assert!(value.get("payload").is_none());
    assert_eq!(
        serde_json::from_value::<Resource>(value.clone()).unwrap(),
        resource
    );
    for (key, field) in [
        ("kind", json!("bundle")),
        ("agents", json!(["codex"])),
        ("payload", json!("resources/rule")),
        ("installation", json!({})),
        ("unexpected", json!(true)),
    ] {
        let mut invalid = value.clone();
        invalid[key] = field;
        assert!(
            serde_json::from_value::<Resource>(invalid).is_err(),
            "{key}"
        );
    }
    let mut invalid = value;
    invalid["files"]["root"] = json!("resources/another/files");
    assert!(serde_json::from_value::<Resource>(invalid).is_err());
}

#[test]
fn one_resource_cannot_mix_actual_kinds_or_multiple_skills() {
    let base = json!({"id":"example","kind":"rule","origin":"self","scope":"user","files":{"root":"resources/example/files","entries":[]}});
    for (kind, entries) in [
        (
            "rule",
            json!([{"target":"AGENTS.md","source":"rule"},{"target":".codex/prompts/prompt.md","source":"prompt"}]),
        ),
        (
            "mcp",
            json!([{"target":".codex/config.toml","toml":{"mcp_servers":{},"hooks":{}}}]),
        ),
        (
            "hook",
            json!([{"target":".claude/settings.json","json":{"mcpServers":{}}}]),
        ),
        (
            "skill",
            json!([{"target":".agents/skills/one/SKILL.md","source":"one"},{"target":".agents/skills/two/SKILL.md","source":"two"}]),
        ),
        (
            "skill",
            json!([{"target":".agents/skills/one/script.sh","source":"script"}]),
        ),
        (
            "skill",
            json!([{"target":".agents/skills/one/docs/SKILL.md","source":"marker"}]),
        ),
        (
            "policy",
            json!([{"target":".codex/rules/policy.md","source":"policy"}]),
        ),
    ] {
        let mut invalid = base.clone();
        invalid["kind"] = json!(kind);
        invalid["files"]["entries"] = entries;
        assert!(
            serde_json::from_value::<Resource>(invalid).is_err(),
            "{kind}"
        );
    }
    resource(
        "skill",
        "example",
        json!([
            {"target":".agents/skills/one/SKILL.md","source":"SKILL.md"},
            {"target":".codex/skills/one/SKILL.md","source":"SKILL.md"},
            {"target":".agents/skills/one/AGENTS.md","source":"AGENTS.md"},
            {"target":".agents/skills/one/scripts/run.sh","source":"scripts/run.sh"}
        ]),
    );
    resource(
        "policy",
        "policy",
        json!([{"target":".codex/config.toml","toml":{"skills":{"config":[]}}}]),
    );
    resource(
        "script",
        "script",
        json!([{"target":".claude/scripts/lib/utils.js","source":"utils.js"}]),
    );
}

#[test]
fn projections_only_read_selected_files_and_keep_modes() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("resources/rule/files");
    fs::create_dir_all(&payload).unwrap();
    fs::write(payload.join("rule.md"), "Reviewed rule\n").unwrap();
    fs::set_permissions(payload.join("rule.md"), fs::Permissions::from_mode(0o750)).unwrap();
    symlink("/etc/passwd", payload.join("unselected-secret")).unwrap();
    let home = temp.path().join("home");
    let catalog = Catalog {
        resources: vec![rule(json!([
            {"target":"AGENTS.md","source":"rule.md"},
            {"target":".codex/rules/exact.md","source":"rule.md"}
        ]))],
        ..Default::default()
    };
    let plan = plan_apply(
        &catalog,
        &BTreeMap::from([("rule".into(), temp.path().into())]),
        &home,
    );
    assert!(plan.results.is_empty(), "{:?}", plan.results);
    assert_eq!(plan.writes.len(), 2);
    let state = State::open(temp.path().join("state.sqlite3")).unwrap();
    assert!(apply_writes(&plan, &state).unwrap().ok());
    assert_eq!(
        fs::read(home.join("AGENTS.md")).unwrap(),
        b"Reviewed rule\n"
    );
    assert_ne!(
        fs::metadata(home.join("AGENTS.md"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert!(apply(Vec::new(), &state).ok());
    assert!(!home.join("AGENTS.md").exists());
}

#[test]
fn config_fragments_preserve_private_leaves_unrelated_fields_and_comments() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(home.join(".codex/config.toml"),"# keep comment\nmodel='local'\n[mcp_servers.managed]\ncommand='docs'\nargs=['private value']\nenabled=false\n[mcp_servers.local]\ncommand='keep'\n").unwrap();
    fs::write(home.join(".mcp.json"),json!({"mcpServers":{"managed":{"token":"private token"},"local":{"command":"keep"}},"other":"local"}).to_string()).unwrap();
    let projection = resource(
        "mcp",
        "managed",
        json!([
            {"target":".codex/config.toml","toml":{"mcp_servers":{"managed":{"command":"docs","args":["${SM_LOCAL}"],"enabled":"${SM_LOCAL}"}}}},
            {"target":".mcp.json","json":{"mcpServers":{"managed":{"token":"${SM_LOCAL}"}}}}
        ]),
    );
    let before = serde_json::to_vec(&projection).unwrap();
    let writes = render(&projection, temp.path(), &home).unwrap();
    let WriteBody::Toml(fragment) = &writes[0].body else {
        panic!("TOML expected")
    };
    assert_eq!(
        fragment["mcp_servers"]["managed"]["args"][0],
        "private value"
    );
    assert_eq!(fragment["mcp_servers"]["managed"]["enabled"], false);
    let state = State::open(temp.path().join("state.sqlite3")).unwrap();
    for write in &writes {
        state
            .replace_owned(&write.path, std::slice::from_ref(write))
            .unwrap();
    }
    let report = apply(writes.clone(), &state);
    assert!(report.ok(), "{report:?}");
    assert!(apply(writes, &state).ok());
    let toml = fs::read_to_string(home.join(".codex/config.toml")).unwrap();
    assert!(
        toml.contains("# keep comment")
            && toml.contains("model='local'")
            && toml.contains("mcp_servers.local")
    );
    let json: Value = serde_json::from_slice(&fs::read(home.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(json["other"], "local");
    assert_eq!(json["mcpServers"]["local"]["command"], "keep");
    assert_eq!(serde_json::to_vec(&projection).unwrap(), before);
    assert!(!String::from_utf8(before).unwrap().contains("private token"));
    let missing = resource(
        "mcp",
        "missing",
        json!([{"target":".mcp.json","json":{"mcpServers":{"absent":{"token":"${SM_LOCAL}"}}}}]),
    );
    let error = render(&missing, temp.path(), &home)
        .unwrap_err()
        .to_string();
    assert!(error.contains("local configuration value missing: mcpServers.absent.token"));
    assert!(!error.contains("private token"));
}

#[test]
fn missing_binding_fails_only_its_resource() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("resources/rule/files");
    fs::create_dir_all(&payload).unwrap();
    fs::write(payload.join("rule.md"), "Reviewed rule").unwrap();
    let catalog = Catalog {
        resources: vec![
            rule(json!([{"target":"AGENTS.md","source":"rule.md"}])),
            resource(
                "mcp",
                "private",
                json!([{"target":".codex/config.toml","toml":{"mcp_servers":{"private":{"command":"docs","env":{"API_TOKEN":"${SM_LOCAL}"}}}}}]),
            ),
        ],
        ..Default::default()
    };
    let home = temp.path().join("home");
    let sources = BTreeMap::from([
        ("rule".into(), temp.path().into()),
        ("private".into(), temp.path().into()),
    ]);
    let plan = plan_apply(&catalog, &sources, &home);
    let state = State::open(temp.path().join("state.sqlite3")).unwrap();
    let report = apply_writes(&plan, &state).unwrap();
    assert_eq!(report.results["rule"].status, ApplyStatus::Applied);
    assert_eq!(report.results["private"].status, ApplyStatus::Failed);
    assert_eq!(
        fs::read_to_string(home.join("AGENTS.md")).unwrap(),
        "Reviewed rule"
    );
}

#[test]
fn projection_rendering_precedes_native_client_checks_and_limits_kimi_hooks() {
    let temp = tempfile::tempdir().unwrap();
    let mut projection = resource(
        "hook",
        "hook",
        json!([{"target":".codex/hooks.json","json":{"hooks":{"Stop":[]}}}]),
    );
    projection.scope = Scope::Project;
    projection.project = Some("project".into());
    assert!(sm::providers::render(&projection, Client::Kimi, temp.path(), temp.path()).is_ok());
    for (target, fragment) in [
        (
            ".kimi-code/hooks.json",
            json!({"json":{"hooks":{"Stop":[]}}}),
        ),
        (
            ".kimi-code/config.toml",
            json!({"toml":{"hooks":[{"event":"Stop","command":"true"}]}}),
        ),
    ] {
        let mut entry = fragment;
        entry["target"] = json!(target);
        let mut projection = resource("hook", "hook", json!([entry]));
        projection.scope = Scope::Project;
        projection.project = Some("project".into());
        assert!(
            render(&projection, temp.path(), temp.path())
                .unwrap_err()
                .is::<sm::providers::UnsupportedResource>()
        );
    }
}

#[test]
fn declaration_shapes_and_targets_are_strict() {
    let valid =
        serde_json::to_value(rule(json!([{"target":"AGENTS.md","source":"rule.md"}]))).unwrap();
    for entry in [
        json!({"target":"AGENTS.md","source":"rule.md","extra":true}),
        json!({"target":"AGENTS.md","source":"rule.md","json":{}}),
        json!({"target":"AGENTS.md"}),
        json!({"target":"AGENTS.md","json":{}}),
        json!({"target":".codex/config.toml","source":"file"}),
        json!({"target":".codex/config.toml","json":{"mcp_servers":{}}}),
        json!({"target":".mcp.json","json":[]}),
    ] {
        let mut invalid = valid.clone();
        invalid["files"]["entries"] = json!([entry]);
        assert!(serde_json::from_value::<Resource>(invalid).is_err());
    }
    for target in [
        "../AGENTS.md",
        "/AGENTS.md",
        ".codex/auth.json",
        ".codex/history.jsonl",
        ".codex/sessions/data.json",
        ".ssh/config",
        "notes.md",
        ".codex/rules",
        ".codex/skills/.system/platform/SKILL.md",
    ] {
        let mut invalid = valid.clone();
        invalid["files"]["entries"][0]["target"] = json!(target);
        assert!(
            serde_json::from_value::<Resource>(invalid).is_err(),
            "{target}"
        );
    }
    for source in ["../outside", "/etc/passwd", ".git/config"] {
        let mut invalid = valid.clone();
        invalid["files"]["entries"][0]["source"] = json!(source);
        assert!(
            serde_json::from_value::<Resource>(invalid).is_err(),
            "{source}"
        );
    }
}

#[test]
fn selected_symlinks_directories_and_target_links_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload");
    fs::create_dir_all(&payload).unwrap();
    fs::write(temp.path().join("outside"), "private").unwrap();
    symlink(temp.path().join("outside"), payload.join("selected")).unwrap();
    let projection = rule(json!([{"target":"AGENTS.md","source":"selected"}]));
    assert!(render(&projection, &payload, &temp.path().join("home")).is_err());
    fs::remove_file(payload.join("selected")).unwrap();
    fs::create_dir(payload.join("selected")).unwrap();
    assert!(render(&projection, &payload, &temp.path().join("home")).is_err());
    fs::remove_dir(payload.join("selected")).unwrap();
    fs::write(payload.join("selected"), "safe").unwrap();
    fs::create_dir(temp.path().join("home")).unwrap();
    symlink(
        temp.path().join("outside"),
        temp.path().join("home/AGENTS.md"),
    )
    .unwrap();
    assert!(render(&projection, &payload, &temp.path().join("home")).is_err());
    assert_eq!(
        fs::read_to_string(temp.path().join("outside")).unwrap(),
        "private"
    );
}

#[test]
fn nested_rules_and_agent_definitions_keep_their_actual_kind() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("file"), "reviewed").unwrap();
    for target in [
        "simulation/AGENTS.md",
        "simulation/tests/modbus/CLAUDE.md",
        "agent-defs/reviewer/AGENTS.md",
    ] {
        let kind = if target.starts_with("agent-defs") {
            "agent"
        } else {
            "rule"
        };
        let mut projection = resource(
            "rule",
            "rule",
            json!([{"target":"AGENTS.md","source":"file"}]),
        );
        projection.scope = Scope::Project;
        projection.project = Some("project".into());
        let mut value = serde_json::to_value(&projection).unwrap();
        value["kind"] = json!(kind);
        value["files"]["entries"][0]["target"] = json!(target);
        let projection: Resource = serde_json::from_value(value).unwrap();
        assert!(render(&projection, temp.path(), &temp.path().join("home")).is_ok());
    }
    for target in [
        ".git/AGENTS.md",
        "vendor/CLAUDE.md",
        "third_party/lib/AGENTS.md",
        "target/AGENTS.md",
        ".worktrees/other/AGENTS.md",
        "resources/imported/AGENTS.md",
    ] {
        let mut projection = rule(json!([{"target":"AGENTS.md","source":"file"}]));
        projection.scope = Scope::Project;
        projection.project = Some("project".into());
        let mut value = serde_json::to_value(projection).unwrap();
        value["files"]["entries"][0]["target"] = json!(target);
        assert!(
            serde_json::from_value::<Resource>(value).is_err(),
            "{target}"
        );
    }
}

#[test]
fn fragment_file_edits_merge_native_config_and_preserve_local_values() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("resources/managed");
    fs::create_dir_all(&payload).unwrap();
    let home = temp.path().join("home");
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(home.join(".codex/config.toml"), "# local comment\nmodel='local'\n[mcp_servers.managed]\ncommand='old'\nargs=['private value']\n[mcp_servers.local]\ncommand='keep'\n").unwrap();
    fs::write(home.join(".mcp.json"), json!({"other":"local","mcpServers":{"managed":{"command":"old","token":"private token"},"local":{"command":"keep"}}}).to_string()).unwrap();
    let initial = resource(
        "mcp",
        "managed",
        json!([
            {"target":".codex/config.toml","toml":{"mcp_servers":{"managed":{"command":"old","args":["${SM_LOCAL}"]}}}},
            {"target":".mcp.json","json":{"mcpServers":{"managed":{"command":"old","token":"${SM_LOCAL}"}}}}
        ]),
    );
    let state = State::open(temp.path().join("state.sqlite3")).unwrap();
    for write in render(&initial, &payload, &home).unwrap() {
        state.replace_owned(&write.path, &[write.clone()]).unwrap();
    }
    let projection: Resource = serde_json::from_value(json!({
        "id":"managed","kind":"mcp","origin":"self","scope":"user",
        "files":{"root":"resources/managed","entries":[
            {"target":".codex/config.toml","fragment":"config.json"},
            {"target":".mcp.json","fragment":"mcp.json"}
        ]}
    }))
    .unwrap();
    assert_eq!(
        projection.paths(),
        [
            "resources/managed/config.json".into(),
            "resources/managed/mcp.json".into()
        ]
        .into()
    );
    for command in ["new", "edited"] {
        fs::write(
            payload.join("config.json"),
            json!({"mcp_servers":{"managed":{"command":command,"args":["${SM_LOCAL}"]}}})
                .to_string(),
        )
        .unwrap();
        fs::write(
            payload.join("mcp.json"),
            json!({"mcpServers":{"managed":{"command":command,"token":"${SM_LOCAL}"}}}).to_string(),
        )
        .unwrap();
        let writes = render(&projection, &payload, &home).unwrap();
        assert!(matches!(writes[0].body, WriteBody::Toml(_)));
        assert!(matches!(writes[1].body, WriteBody::Json(_)));
        let report = apply(writes, &state);
        assert!(report.ok(), "{report:?}");
        let text = fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        assert!(text.contains("# local comment"));
        let document: Value = toml_edit::de::from_str(&text).unwrap();
        assert_eq!(document["model"], "local");
        assert_eq!(document["mcp_servers"]["local"]["command"], "keep");
        assert_eq!(document["mcp_servers"]["managed"]["command"], command);
        assert_eq!(
            document["mcp_servers"]["managed"]["args"][0],
            "private value"
        );
        let document: Value =
            serde_json::from_slice(&fs::read(home.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(document["other"], "local");
        assert_eq!(document["mcpServers"]["local"]["command"], "keep");
        assert_eq!(document["mcpServers"]["managed"]["command"], command);
        assert_eq!(document["mcpServers"]["managed"]["token"], "private token");
        assert!(
            !fs::read_to_string(payload.join("mcp.json"))
                .unwrap()
                .contains("private token")
        );
    }
}

#[test]
fn fragment_sources_and_native_targets_are_strict_and_invalid_bodies_fail_independently() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("resources/config");
    fs::create_dir_all(&payload).unwrap();
    let raw = json!({
        "id":"config","kind":"mcp","origin":"self","scope":"user",
        "files":{"root":"resources/config","entries":[{"target":".codex/config.toml","fragment":"config.json"}]}
    });
    let projection: Resource = serde_json::from_value(raw.clone()).unwrap();
    for source in ["../outside.json", "/etc/passwd", ".git/config"] {
        let mut invalid = raw.clone();
        invalid["files"]["entries"][0]["fragment"] = json!(source);
        assert!(
            serde_json::from_value::<Resource>(invalid).is_err(),
            "{source}"
        );
    }
    for entry in [
        json!({"target":"AGENTS.md","fragment":"config.json"}),
        json!({"target":".codex/config.toml","source":"config.json"}),
        json!({"target":".codex/config.toml","fragment":"config.json","json":{}}),
    ] {
        let mut invalid = raw.clone();
        invalid["files"]["entries"] = json!([entry]);
        assert!(serde_json::from_value::<Resource>(invalid).is_err());
    }
    for body in [
        "invalid JSON",
        "[]",
        "{}",
        "{\"hooks\":{}}",
        "{\"mcp_servers\":{},\"hooks\":{}}",
    ] {
        fs::write(payload.join("config.json"), body).unwrap();
        assert!(
            render(&projection, &payload, &temp.path().join("home")).is_err(),
            "{body}"
        );
    }
    fs::remove_file(payload.join("config.json")).unwrap();
    symlink("/etc/passwd", payload.join("config.json")).unwrap();
    assert!(render(&projection, &payload, &temp.path().join("home")).is_err());
    fs::remove_file(payload.join("config.json")).unwrap();
    fs::create_dir(payload.join("config.json")).unwrap();
    assert!(render(&projection, &payload, &temp.path().join("home")).is_err());
    fs::remove_dir(payload.join("config.json")).unwrap();
    fs::write(payload.join("config.json"), "malformed").unwrap();
    fs::create_dir_all(temp.path().join("resources/rule/files")).unwrap();
    fs::write(
        temp.path().join("resources/rule/files/rule.md"),
        "Reviewed rule",
    )
    .unwrap();
    let catalog = Catalog {
        resources: vec![
            projection,
            rule(json!([{"target":"AGENTS.md","source":"rule.md"}])),
        ],
        ..Default::default()
    };
    let sources = BTreeMap::from([
        ("config".into(), temp.path().into()),
        ("rule".into(), temp.path().into()),
    ]);
    let plan = plan_apply(&catalog, &sources, &temp.path().join("home"));
    let state = State::open(temp.path().join("state.sqlite3")).unwrap();
    let report = apply_writes(&plan, &state).unwrap();
    assert_eq!(report.results["config"].status, ApplyStatus::Failed);
    assert_eq!(report.results["rule"].status, ApplyStatus::Applied);
}

#[test]
fn hook_subset_bindings_select_the_matching_group_and_handler() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    fs::create_dir_all(home.join(".codex")).unwrap();
    let target = home.join(".codex/hooks.json");
    let current = json!({"hooks":{"Stop":[
        {"matcher":"first","hooks":[{"type":"command","command":"first","token":"first-private"}]},
        {"matcher":"selected","hooks":[
            {"type":"command","command":"other","token":"other-private"},
            {"type":"command","command":"selected","token":"selected-private"}
        ]}
    ]},"unknown":"keep"});
    fs::write(&target, current.to_string()).unwrap();
    let projection = resource(
        "hook",
        "selected",
        json!([{"target":".codex/hooks.json","json":{"hooks":{"Stop":[
            {"matcher":"selected","hooks":[{"type":"command","command":"selected","token":"${SM_LOCAL}"}]}
        ]}}}]),
    );
    let writes = render(&projection, temp.path(), &home).unwrap();
    let WriteBody::Json(fragment) = &writes[0].body else {
        panic!("JSON expected")
    };
    assert_eq!(
        fragment["hooks"]["Stop"][0]["hooks"][0]["token"],
        "selected-private"
    );
    assert!(
        !serde_json::to_string(&writes)
            .unwrap()
            .contains("first-private")
    );
    assert!(
        !serde_json::to_string(&writes)
            .unwrap()
            .contains("other-private")
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(target).unwrap()).unwrap(),
        current
    );
}

#[test]
fn indistinguishable_or_missing_hook_binding_candidates_fail_without_writes() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    fs::create_dir_all(home.join(".codex")).unwrap();
    let target = home.join(".codex/hooks.json");
    let current = json!({"hooks":{"Stop":[
        {"matcher":"same","hooks":[{"type":"command","command":"same","token":"first-private"}]},
        {"matcher":"same","hooks":[{"type":"command","command":"same","token":"second-private"}]}
    ]}})
    .to_string();
    fs::write(&target, &current).unwrap();
    for command in ["same", "missing"] {
        let projection = resource(
            "hook",
            "selected",
            json!([{"target":".codex/hooks.json","json":{"hooks":{"Stop":[
                {"matcher":"same","hooks":[{"type":"command","command":command,"token":"${SM_LOCAL}"}]}
            ]}}}]),
        );
        let error = render(&projection, temp.path(), &home)
            .unwrap_err()
            .to_string();
        assert!(!error.contains("first-private") && !error.contains("second-private"));
        let catalog = Catalog {
            resources: vec![projection],
            ..Default::default()
        };
        let plan = plan_apply(
            &catalog,
            &BTreeMap::from([("selected".into(), temp.path().into())]),
            &home,
        );
        assert!(plan.writes.is_empty());
        let state = State::open(temp.path().join(format!("{command}.sqlite3"))).unwrap();
        assert_eq!(
            apply_writes(&plan, &state).unwrap().results["selected"].status,
            ApplyStatus::Failed
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), current);
        assert!(state.owned().unwrap().is_empty());
    }
}

#[test]
fn complete_argument_arrays_keep_their_explicit_local_slots() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(home.join(".codex/config.toml"),"[mcp_servers.managed]\ncommand='docs'\nargs=['--token','first-private','second-private']\n").unwrap();
    let projection = resource(
        "mcp",
        "managed",
        json!([{"target":".codex/config.toml","toml":{"mcp_servers":{"managed":{"command":"docs","args":["--token","${SM_LOCAL}","${SM_LOCAL}"]}}}}]),
    );
    let writes = render(&projection, temp.path(), &home).unwrap();
    let WriteBody::Toml(fragment) = &writes[0].body else {
        panic!("TOML expected")
    };
    assert_eq!(
        fragment["mcp_servers"]["managed"]["args"],
        json!(["--token", "first-private", "second-private"])
    );
}

#[test]
fn a_projection_rejects_multiple_mcp_identities_and_duplicate_targets() {
    let base = json!({"id":"managed","kind":"mcp","origin":"self","scope":"user","files":{"root":"resources/managed/files","entries":[]}});
    for entries in [
        json!([{"target":".codex/config.toml","toml":{"mcp_servers":{"one":{"command":"a"},"two":{"command":"b"}}}}]),
        json!([{"target":".codex/config.toml","toml":{"mcp_servers":{"one":{"command":"a"}}}},{"target":".mcp.json","json":{"mcpServers":{"two":{"command":"b"}}}}]),
        json!([{"target":".codex/config.toml","fragment":"one.json"},{"target":".codex/config.toml","fragment":"two.json"}]),
    ] {
        let mut invalid = base.clone();
        invalid["files"]["entries"] = entries;
        assert!(serde_json::from_value::<Resource>(invalid).is_err());
    }
    resource(
        "mcp",
        "managed",
        json!([
            {"target":".codex/config.toml","toml":{"mcp_servers":{"one":{"command":"a"}}}},
            {"target":".mcp.json","json":{"mcpServers":{"one":{"command":"a"}}}}
        ]),
    );
    let duplicate_rule = json!({"id":"rule","kind":"rule","origin":"self","scope":"user","files":{"root":"resources/rule/files","entries":[
        {"target":".codex/rules/rule.md","source":"one.md"},
        {"target":".codex/rules/./rule.md","source":"two.md"}
    ]}});
    assert!(serde_json::from_value::<Resource>(duplicate_rule).is_err());
    let temp = tempfile::tempdir().unwrap();
    let projection = resource(
        "mcp",
        "managed",
        json!([
            {"target":".codex/config.toml","fragment":"one.json"},
            {"target":".mcp.json","fragment":"two.json"}
        ]),
    );
    fs::write(
        temp.path().join("one.json"),
        json!({"mcp_servers":{"one":{"command":"a"}}}).to_string(),
    )
    .unwrap();
    fs::write(
        temp.path().join("two.json"),
        json!({"mcpServers":{"two":{"command":"b"}}}).to_string(),
    )
    .unwrap();
    assert!(render(&projection, temp.path(), &temp.path().join("home")).is_err());
    fs::write(
        temp.path().join("one.json"),
        json!({"mcp_servers":{"one":{"command":"a"},"two":{"command":"b"}}}).to_string(),
    )
    .unwrap();
    assert!(render(&projection, temp.path(), &temp.path().join("home")).is_err());
}

#[test]
fn full_hook_arrays_require_distinguishable_object_identities() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    fs::create_dir_all(home.join(".codex")).unwrap();
    for identical in [false, true] {
        let second = if identical { "first" } else { "second" };
        let current = json!({"hooks":{"Stop":[
            {"matcher":"first","hooks":[{"type":"command","command":"run","token":"first-private"}]},
            {"matcher":second,"hooks":[{"type":"command","command":"run","token":"second-private"}]}
        ]}});
        fs::write(home.join(".codex/hooks.json"), current.to_string()).unwrap();
        let projection = resource(
            "hook",
            "complete",
            json!([{"target":".codex/hooks.json","json":{"hooks":{"Stop":[
                {"matcher":"first","hooks":[{"type":"command","command":"run","token":"${SM_LOCAL}"}]},
                {"matcher":second,"hooks":[{"type":"command","command":"run","token":"${SM_LOCAL}"}]}
            ]}}}]),
        );
        let rendered = render(&projection, temp.path(), &home);
        if identical {
            assert!(rendered.is_err());
        } else {
            let writes = rendered.unwrap();
            let WriteBody::Json(fragment) = &writes[0].body else {
                panic!("JSON expected")
            };
            assert_eq!(Value::Object(fragment.clone()), current);
        }
    }
}

#[test]
fn skill_trees_project_only_the_selected_complete_skill_with_mirrors() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload");
    fs::create_dir_all(payload.join("selected/scripts")).unwrap();
    fs::create_dir_all(payload.join("other")).unwrap();
    fs::write(payload.join("selected/SKILL.md"), "Selected Skill").unwrap();
    fs::write(payload.join("selected/scripts/run.sh"), "#!/bin/sh\ntrue\n").unwrap();
    fs::set_permissions(
        payload.join("selected/scripts/run.sh"),
        fs::Permissions::from_mode(0o750),
    )
    .unwrap();
    symlink("/etc/passwd", payload.join("other/unselected")).unwrap();
    let projection = resource(
        "skill",
        "selected",
        json!([
            {"target":".agents/skills/selected","tree":"selected"},
            {"target":".codex/skills/selected","tree":"selected"}
        ]),
    );
    assert_eq!(
        projection.paths(),
        ["resources/selected/files/selected".into()].into()
    );
    let writes = render(&projection, &payload, &temp.path().join("home")).unwrap();
    assert_eq!(writes.len(), 4);
    for target in [".agents/skills/selected", ".codex/skills/selected"] {
        assert!(
            writes
                .iter()
                .any(|write| write.path == temp.path().join("home").join(target).join("SKILL.md"))
        );
        assert!(writes.iter().any(|write| write.path
            == temp.path().join("home").join(target).join("scripts/run.sh")
            && matches!(
                write.body,
                WriteBody::File {
                    executable: true,
                    ..
                }
            )));
    }
    assert!(
        !serde_json::to_string(&writes)
            .unwrap()
            .contains("unselected")
    );
}

#[test]
fn tree_selectors_require_one_skill_directory_and_a_safe_complete_payload() {
    let base = json!({"id":"selected","kind":"skill","origin":"self","scope":"user","files":{"root":"resources/selected/files","entries":[{"target":".agents/skills/selected","tree":"selected"}]}});
    for (kind, target, tree) in [
        ("rule", ".agents/skills/selected", "selected"),
        ("skill", ".agents/skills", "selected"),
        ("skill", ".agents/skills/selected/subdir", "selected"),
        ("skill", ".codex/config.toml", "selected"),
        ("skill", ".codex/skills/.system", "selected"),
        ("skill", ".agents/skills/selected", "../outside"),
        ("skill", ".agents/skills/selected", ".git"),
    ] {
        let mut invalid = base.clone();
        invalid["kind"] = json!(kind);
        invalid["files"]["entries"] = json!([{"target":target,"tree":tree}]);
        assert!(
            serde_json::from_value::<Resource>(invalid).is_err(),
            "{kind}/{target}/{tree}"
        );
    }
    let mut different = base.clone();
    different["files"]["entries"]
        .as_array_mut()
        .unwrap()
        .push(json!({"target":".codex/skills/other","tree":"other"}));
    assert!(serde_json::from_value::<Resource>(different).is_err());
    let projection: Resource = serde_json::from_value(base).unwrap();
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("selected")).unwrap();
    fs::write(temp.path().join("selected/helper.sh"), "helper").unwrap();
    assert!(render(&projection, temp.path(), &temp.path().join("home")).is_err());
    fs::write(temp.path().join("selected/SKILL.md"), "Selected Skill").unwrap();
    symlink("/etc/passwd", temp.path().join("selected/link")).unwrap();
    assert!(render(&projection, temp.path(), &temp.path().join("home")).is_err());
    fs::remove_file(temp.path().join("selected/link")).unwrap();
    nix::unistd::mkfifo(
        &temp.path().join("selected/fifo"),
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    assert!(render(&projection, temp.path(), &temp.path().join("home")).is_err());
}
