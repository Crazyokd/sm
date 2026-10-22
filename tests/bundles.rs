use serde_json::{Value, json};
use sm::{
    apply::{apply_writes, plan_apply},
    model::{ApplyPlan, Catalog, OwnedWrite, Resource, ResourceContent, Scope, WriteBody},
    state::State,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};
use tempfile::tempdir;

fn bundle() -> Resource {
    serde_json::from_value(json!({
        "id":"workflow", "kind":"bundle", "origin":"self", "scope":"user",
        "payload":"resources/workflow"
    }))
    .unwrap()
}

fn manifest(payload: &Path, entries: Value) {
    fs::write(
        payload.join("bundle.json"),
        serde_json::to_vec(&json!({"entries": entries})).unwrap(),
    )
    .unwrap();
}

#[test]
fn bundle_has_one_shared_payload_without_client_classification() {
    let resource = bundle();
    assert!(matches!(resource.content, ResourceContent::Bundle { .. }));
    assert_eq!(resource.targets().len(), 2);
    assert_eq!(resource.paths().len(), 1);
    assert_eq!(serde_json::to_value(&resource).unwrap()["kind"], "bundle");
    for (key, value) in [
        ("agents", json!(["codex"])),
        ("payloads", json!({"codex":"resources/workflow"})),
        ("payload", json!("../outside")),
        ("payload", json!("outside")),
        ("scope", json!("system")),
    ] {
        let mut raw = serde_json::to_value(&resource).unwrap();
        raw[key] = value;
        assert!(serde_json::from_value::<Resource>(raw).is_err(), "{key}");
    }
}

#[test]
fn missing_private_mcp_binding_does_not_block_an_independent_rule() {
    let temp = tempdir().unwrap();
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    let root = temp.path().join("source");
    let rule = root.join("resources/rule");
    let mcp = root.join("resources/mcp");
    fs::create_dir_all(&rule).unwrap();
    fs::create_dir_all(&mcp).unwrap();
    fs::write(rule.join("rule.md"), "Reviewed rule").unwrap();
    manifest(&rule, json!([{"target":"AGENTS.md","source":"rule.md"}]));
    manifest(
        &mcp,
        json!([{"target":".codex/config.toml","toml":{"mcp_servers":{"private":{"command":"server","env":{"API_TOKEN":"${SM_LOCAL}"}}}}}]),
    );
    let mut rules = bundle();
    rules.id = "rule".into();
    rules.content = ResourceContent::Bundle {
        payload: "resources/rule".into(),
    };
    let mut private = bundle();
    private.id = "mcp".into();
    private.content = ResourceContent::Bundle {
        payload: "resources/mcp".into(),
    };
    let catalog = Catalog {
        projects: BTreeMap::new(),
        resources: vec![rules, private],
    };
    let sources = BTreeMap::from([("rule".into(), root.clone()), ("mcp".into(), root)]);
    let state = State::open(temp.path().join("state.sqlite3")).unwrap();
    let report = apply_writes(&plan_apply(&catalog, &sources, &home), &state).unwrap();
    assert_eq!(
        report.results["rule"].status,
        sm::model::ApplyStatus::Applied
    );
    assert_eq!(report.results["mcp"].status, sm::model::ApplyStatus::Failed);
    assert_eq!(
        fs::read_to_string(home.join("AGENTS.md")).unwrap(),
        "Reviewed rule"
    );
    assert!(!home.join(".codex/config.toml").exists());
}

#[test]
fn shared_bundle_keeps_kimi_project_hooks_unsupported() {
    let temp = tempdir().unwrap();
    let payload = temp.path().join("payload");
    fs::create_dir(&payload).unwrap();
    let mut resource = bundle();
    resource.scope = Scope::Project;
    resource.project = Some("app".into());
    manifest(
        &payload,
        json!([{"target":".kimi-code/config.toml","toml":{"hooks":[{"event":"Stop","command":"true"}]}}]),
    );
    let error = sm::bundles::render(&resource, &payload, temp.path()).unwrap_err();
    assert!(error.is::<sm::providers::UnsupportedResource>());
    assert!(!temp.path().join(".kimi-code").exists());
    manifest(
        &payload,
        json!([{"target":".kimi-code/hooks.json","json":{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"true"}]}]}}}]),
    );
    assert!(
        sm::bundles::render(&resource, &payload, temp.path())
            .unwrap_err()
            .is::<sm::providers::UnsupportedResource>()
    );
}

#[test]
fn bundle_projects_exact_rule_prompt_and_script_paths_once() {
    let temp = tempdir().unwrap();
    let payload = temp.path().join("resources/workflow");
    fs::create_dir_all(&payload).unwrap();
    fs::write(payload.join("rule.md"), "Exact rule\n").unwrap();
    fs::write(payload.join("prompt.md"), "Exact prompt\n").unwrap();
    fs::write(payload.join("run.sh"), "#!/bin/sh\ntrue\n").unwrap();
    fs::set_permissions(payload.join("run.sh"), fs::Permissions::from_mode(0o750)).unwrap();
    manifest(
        &payload,
        json!([
            {"target":".codex/rules/exact.md", "source":"rule.md"},
            {"target":".kimi-code/prompts/exact.md", "source":"prompt.md"},
            {"target":".codex/hooks/run.sh", "source":"run.sh"}
        ]),
    );
    let home = temp.path().join("home");
    let plan = plan_apply(
        &Catalog {
            projects: BTreeMap::new(),
            resources: vec![bundle()],
        },
        &BTreeMap::from([("workflow".into(), temp.path().to_path_buf())]),
        &home,
    );
    assert!(plan.results.is_empty(), "{:?}", plan.results);
    assert_eq!(plan.writes.len(), 3);
    let state = State::open(temp.path().join("state.db")).unwrap();
    assert!(apply_writes(&plan, &state).unwrap().ok());
    assert_eq!(
        fs::read(home.join(".codex/rules/exact.md")).unwrap(),
        b"Exact rule\n"
    );
    assert_eq!(
        fs::read(home.join(".kimi-code/prompts/exact.md")).unwrap(),
        b"Exact prompt\n"
    );
    assert_ne!(
        fs::metadata(home.join(".codex/hooks/run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert!(!home.join(".codex/rules/bundle.json").exists());
    assert!(apply_writes(&ApplyPlan::default(), &state).unwrap().ok());
    assert!(!home.join(".codex/rules/exact.md").exists());
}

#[test]
fn bundle_fragments_preserve_unrelated_native_configuration() {
    let temp = tempdir().unwrap();
    let payload = temp.path().join("payload");
    fs::create_dir(&payload).unwrap();
    manifest(
        &payload,
        json!([
            {"target":".mcp.json", "json":{"mcpServers":{"managed":{"command":"docs"}}}},
            {"target":".codex/config.toml", "toml":{"mcp_servers":{"managed":{"command":"docs"}}}}
        ]),
    );
    let home = temp.path().join("home");
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(
        home.join(".mcp.json"),
        r#"{"mcpServers":{"local":{"command":"keep"}},"other":"local"}"#,
    )
    .unwrap();
    fs::write(
        home.join(".codex/config.toml"),
        "# local comment\nmodel='local-model'\n[mcp_servers.local]\ncommand='keep'\n",
    )
    .unwrap();
    let writes = sm::bundles::render(&bundle(), &payload, &home).unwrap();
    assert!(matches!(writes[0].body, WriteBody::Json(_)));
    assert!(matches!(writes[1].body, WriteBody::Toml(_)));
    let state = State::open(temp.path().join("state.db")).unwrap();
    let plan = ApplyPlan {
        writes,
        results: BTreeMap::new(),
    };
    let report = apply_writes(&plan, &state).unwrap();
    assert!(report.ok(), "{report:?}");
    let json: Value = serde_json::from_slice(&fs::read(home.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(json["other"], "local");
    assert_eq!(json["mcpServers"]["local"]["command"], "keep");
    assert_eq!(json["mcpServers"]["managed"]["command"], "docs");
    let toml = fs::read_to_string(home.join(".codex/config.toml")).unwrap();
    assert!(toml.contains("# local comment"));
    let toml: toml_edit::DocumentMut = toml.parse().unwrap();
    assert_eq!(toml["model"].as_str(), Some("local-model"));
    assert_eq!(
        toml["mcp_servers"]["local"]["command"].as_str(),
        Some("keep")
    );
    assert_eq!(
        toml["mcp_servers"]["managed"]["command"].as_str(),
        Some("docs")
    );
    assert!(apply_writes(&ApplyPlan::default(), &state).unwrap().ok());
    let json: Value = serde_json::from_slice(&fs::read(home.join(".mcp.json")).unwrap()).unwrap();
    assert!(json["mcpServers"].get("managed").is_none());
    assert_eq!(json["mcpServers"]["local"]["command"], "keep");
}

#[test]
fn bundle_binds_private_values_from_the_same_native_document_only() {
    let temp = tempdir().unwrap();
    let payload = temp.path().join("payload");
    fs::create_dir(&payload).unwrap();
    let home = temp.path().join("home");
    let target = home.join(".codex/config.toml");
    let json_target = home.join(".mcp.json");
    let state = State::open(temp.path().join("state.db")).unwrap();
    let initial = json!({"mcp_servers":{"managed":{"command":"old", "args":["private-value"], "enabled":true}}});
    let plan = ApplyPlan {
        writes: vec![
            OwnedWrite::new(
                "workflow",
                &target,
                WriteBody::Toml(initial.as_object().unwrap().clone()),
            ),
            OwnedWrite::new(
                "workflow",
                &json_target,
                WriteBody::Json(
                    json!({"mcpServers":{"managed":{"token":"json-private"}}})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            ),
        ],
        results: BTreeMap::new(),
    };
    assert!(apply_writes(&plan, &state).unwrap().ok());
    manifest(
        &payload,
        json!([
            {
                "target":".codex/config.toml",
                "toml":{"mcp_servers":{"managed":{"command":"new", "args":["${SM_LOCAL}"], "enabled":"${SM_LOCAL}"}}}
            },
            {"target":".mcp.json", "json":{"mcpServers":{"managed":{"token":"${SM_LOCAL}"}}}}
        ]),
    );
    let manifest_bytes = fs::read(payload.join("bundle.json")).unwrap();
    let writes = sm::bundles::render(&bundle(), &payload, &home).unwrap();
    let WriteBody::Toml(fragment) = &writes[0].body else {
        panic!("expected TOML fragment")
    };
    assert_eq!(
        fragment["mcp_servers"]["managed"]["args"][0],
        "private-value"
    );
    assert_eq!(fragment["mcp_servers"]["managed"]["enabled"], true);
    let report = apply_writes(
        &ApplyPlan {
            writes,
            results: BTreeMap::new(),
        },
        &state,
    )
    .unwrap();
    assert!(report.ok(), "{report:?}");
    let applied: Value = toml_edit::de::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(applied["mcp_servers"]["managed"]["command"], "new");
    assert_eq!(
        applied["mcp_servers"]["managed"]["args"][0],
        "private-value"
    );
    let applied: Value = serde_json::from_slice(&fs::read(&json_target).unwrap()).unwrap();
    assert_eq!(applied["mcpServers"]["managed"]["token"], "json-private");
    assert_eq!(
        fs::read(payload.join("bundle.json")).unwrap(),
        manifest_bytes
    );
    assert!(
        !String::from_utf8(manifest_bytes)
            .unwrap()
            .contains("private-value")
    );

    manifest(
        &payload,
        json!([{
            "target":".mcp.json", "json":{"mcpServers":{"missing":{"token":"${SM_LOCAL}"}}}
        }]),
    );
    let error = sm::bundles::render(&bundle(), &payload, &home)
        .unwrap_err()
        .to_string();
    assert!(error.contains("local configuration value missing: mcpServers.missing.token"));
    assert!(!error.contains("private-value"));
    fs::write(&target, "token = local-private-value\n").unwrap();
    manifest(
        &payload,
        json!([{"target":".codex/config.toml", "toml":{"mcp_servers":{}}}]),
    );
    let error = sm::bundles::render(&bundle(), &payload, &home)
        .unwrap_err()
        .to_string();
    assert_eq!(error, "local configuration is not valid TOML");
}

#[test]
fn bundle_rejects_arbitrary_state_and_traversal_targets() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join("file"), "data").unwrap();
    for target in [
        "../AGENTS.md",
        "/AGENTS.md",
        ".codex/rules/../../auth.json",
        ".codex/auth.json",
        ".codex/history.jsonl",
        ".codex/sessions/data.json",
        ".codex/cache/data",
        ".kimi-code/credentials.json",
        ".claude/settings.json",
        ".ssh/config",
        "notes.md",
        ".codex/rules",
        "agent-defs/role.md",
    ] {
        manifest(temp.path(), json!([{"target": target, "source":"file"}]));
        assert!(
            sm::bundles::render(&bundle(), temp.path(), &temp.path().join("home")).is_err(),
            "{target}"
        );
    }
    let mut resource = bundle();
    resource.scope = Scope::Project;
    resource.project = Some("app".into());
    manifest(
        temp.path(),
        json!([{"target":"agent-defs/role.md", "source":"file"}]),
    );
    assert!(sm::bundles::render(&resource, temp.path(), &temp.path().join("project")).is_ok());
}

#[test]
fn bundle_requires_strict_entry_shapes_and_config_fragments() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join("file"), "data").unwrap();
    for entry in [
        json!({"target":"AGENTS.md", "source":"file", "extra":true}),
        json!({"target":"AGENTS.md", "source":"file", "json":{}}),
        json!({"target":"AGENTS.md"}),
        json!({"target":"AGENTS.md", "json":{}}),
        json!({"target":".codex/config.toml", "source":"file"}),
        json!({"target":".codex/hooks.json", "source":"file"}),
        json!({"target":".codex/config.toml", "json":{}}),
        json!({"target":".mcp.json", "toml":{}}),
        json!({"target":".mcp.json", "json":[]}),
    ] {
        manifest(temp.path(), json!([entry.clone()]));
        assert!(
            sm::bundles::render(&bundle(), temp.path(), &temp.path().join("home")).is_err(),
            "{entry}"
        );
    }
    fs::write(
        temp.path().join("bundle.json"),
        r#"{"entries":[],"extra":true}"#,
    )
    .unwrap();
    assert!(sm::bundles::render(&bundle(), temp.path(), &temp.path().join("home")).is_err());
}

#[test]
fn bundle_rejects_traversal_and_symlink_sources_and_targets() {
    let temp = tempdir().unwrap();
    let payload = temp.path().join("payload");
    fs::create_dir(&payload).unwrap();
    fs::write(temp.path().join("outside"), "secret").unwrap();
    fs::write(payload.join("file"), "safe").unwrap();
    for source in ["../outside", "/etc/passwd", ".git/hidden"] {
        manifest(&payload, json!([{"target":"AGENTS.md", "source":source}]));
        assert!(sm::bundles::render(&bundle(), &payload, &temp.path().join("home")).is_err());
    }
    manifest(&payload, json!([{"target":"AGENTS.md", "source":"file"}]));
    symlink("file", payload.join("alias")).unwrap();
    assert!(sm::bundles::render(&bundle(), &payload, &temp.path().join("home")).is_err());
    fs::remove_file(payload.join("alias")).unwrap();
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    symlink(temp.path().join("outside"), home.join("AGENTS.md")).unwrap();
    assert!(sm::bundles::render(&bundle(), &payload, &home).is_err());
    assert_eq!(fs::read(temp.path().join("outside")).unwrap(), b"secret");
}

#[test]
fn nested_rules_are_project_only_and_cannot_target_excluded_trees() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join("rule"), "project rule").unwrap();
    let mut resource = bundle();
    for target in ["simulation/AGENTS.md", "simulation/tests/modbus/CLAUDE.md"] {
        manifest(temp.path(), json!([{"target":target,"source":"rule"}]));
        assert!(sm::bundles::render(&resource, temp.path(), &temp.path().join("home")).is_err());
    }
    resource.scope = Scope::Project;
    resource.project = Some("project".into());
    for target in ["simulation/AGENTS.md", "simulation/tests/modbus/CLAUDE.md"] {
        manifest(temp.path(), json!([{"target":target,"source":"rule"}]));
        assert!(sm::bundles::render(&resource, temp.path(), &temp.path().join("home")).is_ok());
    }
    for target in [
        "../AGENTS.md",
        "simulation/../../AGENTS.md",
        ".git/AGENTS.md",
        "vendor/CLAUDE.md",
        "third_party/lib/AGENTS.md",
        "target/AGENTS.md",
        ".worktrees/other/AGENTS.md",
        "caches/AGENTS.md",
        "resources/imported/AGENTS.md",
        "task_archives/CLAUDE.md",
        "simulation/auth.json",
    ] {
        manifest(temp.path(), json!([{"target":target,"source":"rule"}]));
        assert!(
            sm::bundles::render(&resource, temp.path(), &temp.path().join("home")).is_err(),
            "{target}"
        );
    }
}

#[test]
fn legacy_ai_file_paths_are_explicit_and_private_configs_require_fragments() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join("file"), "reviewed resource").unwrap();
    for target in [
        ".claude/agents/reviewer.md",
        ".claude/commands/review.md",
        ".claude/prompts/review.md",
        ".claude/rules/common.md",
        ".claude/hooks/check.sh",
        ".claude/scripts/hooks/start.js",
        ".claude/AGENTS.md",
        ".claude/CLAUDE.md",
        ".cursor/rules/project.mdc",
    ] {
        manifest(temp.path(), json!([{"target":target,"source":"file"}]));
        assert!(
            sm::bundles::render(&bundle(), temp.path(), &temp.path().join("home")).is_ok(),
            "{target}"
        );
    }
    for target in [
        ".claude/auth.json",
        ".claude/settings.json",
        ".claude.json",
        ".claude/history.jsonl",
        ".claude/projects/session.jsonl",
        ".codex/skills/.system/platform/SKILL.md",
        ".codex/scripts/unregistered.js",
    ] {
        manifest(temp.path(), json!([{"target":target,"source":"file"}]));
        assert!(
            sm::bundles::render(&bundle(), temp.path(), &temp.path().join("home")).is_err(),
            "{target}"
        );
    }
}

#[test]
fn legacy_native_fragments_bind_private_leaves_without_importing_other_settings() {
    let temp = tempdir().unwrap();
    let home = temp.path().join("home");
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::write(home.join(".claude/settings.json"), json!({"permissions":{"allow":["local permission"]},"mcpServers":{"graph":{"headers":{"Authorization":"private bearer value"}}}}).to_string()).unwrap();
    manifest(
        temp.path(),
        json!([{"target":".claude/settings.json","json":{"mcpServers":{"graph":{"headers":{"Authorization":"${SM_LOCAL}"}}}}}]),
    );
    let writes = sm::bundles::render(&bundle(), temp.path(), &home).unwrap();
    let WriteBody::Json(fragment) = &writes[0].body else {
        panic!("expected JSON fragment")
    };
    assert_eq!(
        fragment["mcpServers"]["graph"]["headers"]["Authorization"],
        "private bearer value"
    );
    assert!(!fragment.contains_key("permissions"));
    fs::write(
        home.join(".claude.json"),
        json!({"mcpServers":{"graph":{"token":"private token"}},"auth":"local profile"})
            .to_string(),
    )
    .unwrap();
    manifest(
        temp.path(),
        json!([{"target":".claude.json","json":{"mcpServers":{"graph":{"token":"${SM_LOCAL}"}}}}]),
    );
    assert!(sm::bundles::render(&bundle(), temp.path(), &home).is_ok());
    let mut project = bundle();
    project.scope = Scope::Project;
    project.project = Some("project".into());
    assert!(sm::bundles::render(&project, temp.path(), &home).is_err());
}
