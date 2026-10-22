use serde_json::{Value, json};
use sm::{
    inventory::{Inventory, scan},
    model::{Catalog, OwnedWrite, Scope, WriteBody},
    state::State,
};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};
use tempfile::{TempDir, tempdir};

fn fixture() -> (TempDir, State) {
    let home = tempdir().unwrap();
    let state = State::open(home.path().join("state/state.db")).unwrap();
    (home, state)
}

fn put(home: &Path, relative: &str, content: &str) -> PathBuf {
    let path = home.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, content).unwrap();
    path
}

fn own_file(state: &State, path: &Path, id: &str) {
    state
        .replace_owned(
            path,
            &[OwnedWrite::new(
                id,
                path,
                WriteBody::File {
                    bytes: fs::read(path).unwrap(),
                    executable: fs::metadata(path).unwrap().permissions().mode() & 0o111 != 0,
                },
            )],
        )
        .unwrap();
}

fn at<'a>(inventory: &'a Inventory, path: &Path) -> &'a sm::inventory::InventoryEntry {
    inventory
        .entries
        .iter()
        .find(|entry| entry.path == path)
        .unwrap_or_else(|| panic!("missing {}", path.display()))
}

#[test]
fn discovers_new_skills_and_aliases_without_reading_payloads() {
    let (home, state) = fixture();
    let marker = put(
        home.path(),
        ".agents/skills/managed/SKILL.md",
        "managed skill",
    );
    own_file(&state, &marker, "managed");
    let unmanaged = put(home.path(), ".codex/skills/local/SKILL.md", "local skill");
    put(
        home.path(),
        ".agents/skills/managed/scripts/private.sh",
        "PRIVATE_SCRIPT_BODY",
    );
    fs::create_dir_all(home.path().join(".claude/skills")).unwrap();
    let alias = home.path().join(".claude/skills/managed");
    symlink(marker.parent().unwrap(), &alias).unwrap();
    let inventory = scan(&Catalog::default(), &state, home.path()).unwrap();
    assert!(inventory.errors.is_empty(), "{:?}", inventory.errors);
    assert_eq!(at(&inventory, marker.parent().unwrap()).status, "managed");
    assert_eq!(
        at(&inventory, unmanaged.parent().unwrap()).status,
        "unmanaged"
    );
    assert_eq!(at(&inventory, &alias).alias_of.as_deref(), marker.parent());
    assert_eq!(at(&inventory, &alias).owner.as_deref(), Some("managed"));
    assert!(
        !serde_json::to_string(&inventory)
            .unwrap()
            .contains("PRIVATE_SCRIPT_BODY")
    );
    assert!(
        !inventory
            .entries
            .iter()
            .any(|entry| entry.path.ends_with("private.sh"))
    );
    let novel = put(
        home.path(),
        ".agents/skills/after-scan/SKILL.md",
        "new skill",
    );
    let refreshed = scan(&Catalog::default(), &state, home.path()).unwrap();
    assert_eq!(at(&refreshed, novel.parent().unwrap()).status, "unmanaged");
}

#[test]
fn exact_config_fragments_preserve_disabled_state_and_hide_secrets() {
    let (home, state) = fixture();
    let config = put(
        home.path(),
        ".codex/config.toml",
        "[mcp_servers.codegraph]\ncommand='PRIVATE_COMMAND'\nenabled=false\nenv={API_TOKEN='PRIVATE_TOKEN'}\n[mcp_servers.hindsight]\ncommand='HINDSIGHT_COMMAND'\n",
    );
    let mut fragment = serde_json::Map::new();
    fragment.insert("mcp_servers".into(), json!({"codegraph":{"command":"PRIVATE_COMMAND","enabled":false,"env":{"API_TOKEN":"PRIVATE_TOKEN"}}}));
    state
        .replace_owned(
            &config,
            &[OwnedWrite::new("graph", &config, WriteBody::Toml(fragment))],
        )
        .unwrap();
    let inventory = scan(&Catalog::default(), &state, home.path()).unwrap();
    let graph = inventory
        .entries
        .iter()
        .find(|entry| entry.id.ends_with("#mcp_servers/codegraph"))
        .unwrap();
    let hindsight = inventory
        .entries
        .iter()
        .find(|entry| entry.id.ends_with("#mcp_servers/hindsight"))
        .unwrap();
    assert_eq!(graph.status, "managed");
    assert_eq!(graph.enabled, Some(false));
    assert_eq!(hindsight.status, "unmanaged");
    let serialized = serde_json::to_string(&inventory).unwrap();
    for secret in [
        "PRIVATE_COMMAND",
        "PRIVATE_TOKEN",
        "HINDSIGHT_COMMAND",
        "API_TOKEN",
    ] {
        assert!(!serialized.contains(secret), "inventory exposed {secret}");
    }
    fs::write(
        &config,
        "[mcp_servers.codegraph]\ncommand='changed'\nenabled=false\n",
    )
    .unwrap();
    let inventory = scan(&Catalog::default(), &state, home.path()).unwrap();
    assert_eq!(
        inventory
            .entries
            .iter()
            .find(|entry| entry.id.ends_with("#mcp_servers/codegraph"))
            .unwrap()
            .status,
        "drift"
    );
}

#[test]
fn detects_file_content_mode_and_missing_projection_drift() {
    let (home, state) = fixture();
    let script = put(home.path(), ".codex/hooks/check.sh", "original");
    own_file(&state, &script, "check");
    assert_eq!(
        at(
            &scan(&Catalog::default(), &state, home.path()).unwrap(),
            &script
        )
        .status,
        "managed"
    );
    fs::write(&script, "modified").unwrap();
    assert_eq!(
        at(
            &scan(&Catalog::default(), &state, home.path()).unwrap(),
            &script
        )
        .status,
        "drift"
    );
    fs::write(&script, "original").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        at(
            &scan(&Catalog::default(), &state, home.path()).unwrap(),
            &script
        )
        .status,
        "drift"
    );
    fs::remove_file(&script).unwrap();
    assert_eq!(
        at(
            &scan(&Catalog::default(), &state, home.path()).unwrap(),
            &script
        )
        .status,
        "drift"
    );
}

#[test]
fn native_ownership_requires_an_explicit_installation_except_platform() {
    let (home, state) = fixture();
    let package = home.path().join(".codex/plugins/cache/market/plugin");
    let marker = put(
        home.path(),
        ".codex/plugins/cache/market/plugin/1/skills/tool/SKILL.md",
        "tool",
    );
    let system = put(
        home.path(),
        ".codex/skills/.system/system/SKILL.md",
        "platform",
    );
    let bundled = put(
        home.path(),
        ".codex/plugins/cache/openai-bundled/core/1/skills/bundled/SKILL.md",
        "bundled",
    );
    put(
        home.path(),
        ".codex/config.toml",
        "[plugins.'plugin@market']\nenabled=false\n",
    );
    let before = scan(&Catalog::default(), &state, home.path()).unwrap();
    assert_eq!(at(&before, marker.parent().unwrap()).status, "unmanaged");
    assert_eq!(at(&before, system.parent().unwrap()).status, "native");
    assert_eq!(
        at(&before, bundled.parent().unwrap()).owner.as_deref(),
        Some("platform")
    );
    state
        .set(
            "external_installations",
            &json!([{"id":"plugin-package","paths":[package]}]),
        )
        .unwrap();
    let inventory = scan(&Catalog::default(), &state, home.path()).unwrap();
    let skill = at(&inventory, marker.parent().unwrap());
    assert_eq!(skill.status, "native");
    assert_eq!(skill.owner.as_deref(), Some("plugin-package"));
    assert_eq!(skill.enabled, Some(false));
}

#[test]
fn kimi_registry_only_exposes_resource_locations_and_enabled_state() {
    let (home, state) = fixture();
    let root = home.path().join(".kimi-code/plugins/managed/ponytail");
    let marker = put(
        home.path(),
        ".kimi-code/plugins/managed/ponytail/skills/ponytail/SKILL.md",
        "skill",
    );
    put(home.path(), ".kimi-code/plugins/installed.json", &json!({"plugins":[{"id":"ponytail","root":root,"enabled":false,"source":"REGISTRY_SECRET","originalSource":{"token":"REGISTRY_TOKEN"}}]}).to_string());
    state
        .set(
            "external_installations",
            &json!([{"id":"ponytail","paths":[root]}]),
        )
        .unwrap();
    let inventory = scan(&Catalog::default(), &state, home.path()).unwrap();
    assert_eq!(at(&inventory, marker.parent().unwrap()).status, "native");
    assert_eq!(
        at(&inventory, marker.parent().unwrap()).enabled,
        Some(false)
    );
    let serialized = serde_json::to_string(&inventory).unwrap();
    assert!(!serialized.contains("REGISTRY_SECRET"));
    assert!(!serialized.contains("REGISTRY_TOKEN"));
}

#[test]
fn hook_handlers_are_sanitized_and_owned_by_exact_array_entry() {
    let (home, state) = fixture();
    let handler = json!({"type":"command","command":"PRIVATE_HOOK_COMMAND"});
    let hooks = put(home.path(), ".codex/hooks.json", &json!({"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"LOCAL_COMMAND"},handler]}]}}).to_string());
    let fragment = json!({"hooks":{"SessionStart":[{"hooks":[handler]}]}})
        .as_object()
        .unwrap()
        .clone();
    state
        .replace_owned(
            &hooks,
            &[OwnedWrite::new(
                "observer",
                &hooks,
                WriteBody::Json(fragment),
            )],
        )
        .unwrap();
    put(
        home.path(),
        ".kimi-code/config.toml",
        "[[hooks]]\nevent='Stop'\ncommand='PRIVATE_KIMI_COMMAND'\n",
    );
    let inventory = scan(&Catalog::default(), &state, home.path()).unwrap();
    let rows: Vec<_> = inventory
        .entries
        .iter()
        .filter(|entry| entry.path == hooks && entry.kind == "hook")
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .filter(|entry| entry.status == "managed")
            .count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|entry| entry.status == "unmanaged")
            .count(),
        1
    );
    assert!(
        inventory
            .entries
            .iter()
            .any(|entry| entry.id.ends_with("#hooks/0/Stop"))
    );
    let serialized = serde_json::to_string(&inventory).unwrap();
    for secret in [
        "PRIVATE_HOOK_COMMAND",
        "LOCAL_COMMAND",
        "PRIVATE_KIMI_COMMAND",
    ] {
        assert!(!serialized.contains(secret));
    }
}

#[test]
fn scope_is_exact_and_missing_projects_and_unsafe_links_are_reported() {
    let (home, state) = fixture();
    let outside = tempdir().unwrap();
    put(outside.path(), "secret/SKILL.md", "outside");
    fs::create_dir_all(home.path().join(".agents/skills")).unwrap();
    symlink(
        outside.path().join("secret"),
        home.path().join(".agents/skills/escape"),
    )
    .unwrap();
    symlink("cycle", home.path().join(".agents/skills/cycle")).unwrap();
    let rule = put(home.path(), "projects/registered/AGENTS.md", "project rule");
    put(
        home.path(),
        "projects/not-registered/AGENTS.md",
        "not registered",
    );
    let catalog = Catalog {
        projects: [
            ("registered".into(), "~/projects/registered".into()),
            ("missing".into(), "~/projects/missing".into()),
        ]
        .into(),
        resources: vec![],
    };
    let inventory = scan(&catalog, &state, home.path()).unwrap();
    assert_eq!(at(&inventory, &rule).scope, Scope::Project);
    assert_eq!(at(&inventory, &rule).project.as_deref(), Some("registered"));
    assert!(!inventory.entries.iter().any(|entry| {
        entry.path.to_string_lossy().contains("not-registered")
            || entry.path.to_string_lossy().contains("escape")
    }));
    assert!(
        inventory
            .errors
            .iter()
            .any(|error| error.contains("registered project"))
    );
    assert!(
        inventory
            .errors
            .iter()
            .any(|error| error.contains("outside home"))
    );
    assert!(
        inventory
            .errors
            .iter()
            .any(|error| error.contains("cyclic link"))
    );
}

#[test]
fn malformed_configuration_errors_do_not_include_source_text() {
    let (home, state) = fixture();
    put(
        home.path(),
        ".codex/config.toml",
        "api_token = 'SECRET_MALFORMED_TOKEN",
    );
    let inventory = scan(&Catalog::default(), &state, home.path()).unwrap();
    assert!(!inventory.errors.is_empty());
    assert!(
        !serde_json::to_string(&inventory)
            .unwrap()
            .contains("SECRET_MALFORMED_TOKEN")
    );
    let _: Value = serde_json::from_str(&serde_json::to_string(&inventory).unwrap()).unwrap();
}

#[test]
fn removed_owned_fragment_remains_visible_as_drift() {
    let (home, state) = fixture();
    let config = put(
        home.path(),
        ".kimi-code/mcp.json",
        r#"{"mcpServers":{"local":{"command":"local"}}}"#,
    );
    let fragment = json!({"mcpServers":{"managed":{"command":"private"}}})
        .as_object()
        .unwrap()
        .clone();
    state
        .replace_owned(
            &config,
            &[OwnedWrite::new(
                "missing",
                &config,
                WriteBody::Json(fragment),
            )],
        )
        .unwrap();
    let inventory = scan(&Catalog::default(), &state, home.path()).unwrap();
    assert!(
        inventory
            .entries
            .iter()
            .any(|entry| entry.owner.as_deref() == Some("missing") && entry.status == "drift")
    );
    assert!(
        inventory
            .entries
            .iter()
            .any(|entry| entry.id.ends_with("#mcpServers/local") && entry.status == "unmanaged")
    );
}

#[test]
fn multiple_markdown_owners_remain_individually_visible() {
    let (home, state) = fixture();
    let rule = put(
        home.path(),
        "AGENTS.md",
        "local prose\n<!-- SM:first -->\none\n<!-- /SM:first -->\n<!-- SM:second -->\ntwo\n<!-- /SM:second -->\n",
    );
    state
        .replace_owned(
            &rule,
            &[
                OwnedWrite::new("first", &rule, WriteBody::Markdown(b"one".to_vec())),
                OwnedWrite::new("second", &rule, WriteBody::Markdown(b"two".to_vec())),
            ],
        )
        .unwrap();
    let inventory = scan(&Catalog::default(), &state, home.path()).unwrap();
    for owner in ["first", "second"] {
        assert!(inventory.entries.iter().any(|entry| entry.path == rule
            && entry.owner.as_deref() == Some(owner)
            && entry.status == "managed"));
    }
}

#[test]
fn discovers_nested_rules_once_in_the_longest_registered_project_scope() {
    let (home, state) = fixture();
    let rule = put(
        home.path(),
        "projects/app/simulation/tests/modbus/AGENTS.md",
        "nested rule",
    );
    let child = put(
        home.path(),
        "projects/app/component/deep/CLAUDE.md",
        "child rule",
    );
    for directory in [
        ".git",
        "build",
        "install",
        "target",
        "node_modules",
        "vendor",
        "third_party",
        "third-party",
        ".worktrees",
        "caches",
        ".cache",
        "log",
        "resources",
        "task_archives",
    ] {
        put(
            home.path(),
            &format!("projects/app/{directory}/ignored/AGENTS.md"),
            "excluded",
        );
    }
    put(
        home.path(),
        "unregistered/nested/AGENTS.md",
        "not a registered project",
    );
    let outside = tempdir().unwrap();
    put(outside.path(), "hidden/AGENTS.md", "outside home");
    symlink(outside.path(), home.path().join("projects/app/alias")).unwrap();
    let catalog = Catalog {
        projects: [
            ("app".into(), "~/projects/app".into()),
            ("component".into(), "~/projects/app/component".into()),
        ]
        .into(),
        resources: vec![],
    };
    let inventory = scan(&catalog, &state, home.path()).unwrap();
    assert!(inventory.errors.is_empty(), "{:?}", inventory.errors);
    assert_eq!(at(&inventory, &rule).project.as_deref(), Some("app"));
    assert_eq!(at(&inventory, &child).project.as_deref(), Some("component"));
    assert_eq!(
        inventory
            .entries
            .iter()
            .filter(|entry| entry.path == child)
            .count(),
        1
    );
    assert!(!inventory.entries.iter().any(
        |entry| entry.path.to_string_lossy().contains("ignored")
            || entry.path.to_string_lossy().contains("unregistered")
            || entry.path.to_string_lossy().contains("alias")
    ));
}

#[test]
fn nested_rule_depth_limit_reports_incomplete_coverage() {
    let (home, state) = fixture();
    put(
        home.path(),
        &format!("project/{}/AGENTS.md", vec!["deep"; 18].join("/")),
        "deep rule",
    );
    let catalog = Catalog {
        projects: [("app".into(), "~/project".into())].into(),
        resources: vec![],
    };
    let inventory = scan(&catalog, &state, home.path()).unwrap();
    assert!(
        inventory
            .errors
            .iter()
            .any(|error| error.contains("scan limit reached"))
    );
}

#[test]
fn codex_skill_names_override_package_flags_and_paths_override_names() {
    let (home, state) = fixture();
    let raw = put(home.path(), ".agents/skills/cavecrew/SKILL.md", "shared");
    let raw_ponytail = put(
        home.path(),
        ".agents/skills/ponytail/SKILL.md",
        "shared same name",
    );
    let kimi = put(home.path(), ".kimi-code/skills/cavecrew/SKILL.md", "kimi");
    let plugin = put(
        home.path(),
        ".codex/plugins/cache/market/ponytail/1/skills/ponytail/SKILL.md",
        "plugin",
    );
    let other = put(
        home.path(),
        ".codex/plugins/cache/market/other/1/skills/ponytail/SKILL.md",
        "other plugin",
    );
    let specific = put(
        home.path(),
        ".codex/plugins/cache/market/ponytail/1/skills/specific/SKILL.md",
        "specific path",
    );
    put(
        home.path(),
        ".codex/config.toml",
        &format!(
            "[plugins.'ponytail@market']\nenabled=true\n[plugins.'other@market']\nenabled=true\n[[skills.config]]\nname='cavecrew'\nenabled=false\n[[skills.config]]\nname='ponytail:ponytail'\nenabled=false\n[[skills.config]]\nname='foreign:ponytail'\nenabled=false\n[[skills.config]]\nname='ponytail:specific'\nenabled=false\n[[skills.config]]\npath={}\nenabled=true\n",
            serde_json::to_string(&specific).unwrap()
        ),
    );
    let inventory = scan(&Catalog::default(), &state, home.path()).unwrap();
    assert_eq!(at(&inventory, raw.parent().unwrap()).enabled, Some(false));
    assert_eq!(
        at(&inventory, raw.parent().unwrap())
            .enabled_source
            .as_deref(),
        Some("codex:skills.config")
    );
    assert_eq!(
        at(&inventory, plugin.parent().unwrap()).enabled,
        Some(false)
    );
    assert_eq!(at(&inventory, other.parent().unwrap()).enabled, Some(true));
    assert_eq!(
        at(&inventory, specific.parent().unwrap()).enabled,
        Some(true)
    );
    assert_eq!(at(&inventory, kimi.parent().unwrap()).enabled, None);
    assert_eq!(at(&inventory, raw_ponytail.parent().unwrap()).enabled, None);
}

#[test]
fn legacy_commands_and_claude_rules_and_global_mcp_are_inventoried_without_private_values() {
    let (home, state) = fixture();
    let command = put(home.path(), ".claude/commands/review.md", "command body");
    let instructions = put(home.path(), ".claude/CLAUDE.md", "legacy rules");
    let runtime = put(
        home.path(),
        ".claude/scripts/hooks/start.js",
        "runtime body",
    );
    for directory in ["node_modules", "venv", "site-packages", ".cache"] {
        put(
            home.path(),
            &format!(".claude/scripts/{directory}/ignored.js"),
            "not a resource",
        );
    }
    put(
        home.path(),
        ".claude.json",
        r#"{"mcpServers":{"graph":{"headers":{"Authorization":"PRIVATE_PROFILE_HEADER"}}},"auth":"PRIVATE_AUTH_STATE"}"#,
    );
    let inventory = scan(&Catalog::default(), &state, home.path()).unwrap();
    assert_eq!(at(&inventory, &command).kind, "prompt");
    assert_eq!(at(&inventory, &instructions).kind, "rule");
    assert_eq!(at(&inventory, &runtime).kind, "hook-support");
    assert!(
        !inventory
            .entries
            .iter()
            .any(|entry| entry.path.ends_with("ignored.js"))
    );
    assert!(
        inventory
            .entries
            .iter()
            .any(|entry| entry.id.ends_with(".claude.json#mcpServers/graph"))
    );
    let serialized = serde_json::to_string(&inventory).unwrap();
    assert!(
        !serialized.contains("PRIVATE_PROFILE_HEADER")
            && !serialized.contains("PRIVATE_AUTH_STATE")
    );
}
