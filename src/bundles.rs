use crate::{
    files::{contained, payload_files, reject_symlink_path, relative_path},
    model::{OwnedWrite, Resource, Scope, WriteBody},
};
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum Entry {
    File {
        target: PathBuf,
        source: PathBuf,
    },
    Json {
        target: PathBuf,
        json: Map<String, Value>,
    },
    Toml {
        target: PathBuf,
        toml: Map<String, Value>,
    },
}

fn config_kind(target: &Path) -> Option<bool> {
    match target.to_str()? {
        ".mcp.json"
        | ".codex/hooks.json"
        | ".kimi-code/hooks.json"
        | ".kimi-code/mcp.json"
        | ".claude/settings.json"
        | ".claude.json" => Some(false),
        ".codex/config.toml" | ".kimi-code/config.toml" => Some(true),
        _ => None,
    }
}

pub(crate) fn excluded_rule_tree(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                ".git"
                    | "build"
                    | "install"
                    | "target"
                    | "node_modules"
                    | "vendor"
                    | "third_party"
                    | "third-party"
                    | ".worktrees"
                    | "cache"
                    | "caches"
                    | ".cache"
                    | ".caches"
                    | "log"
                    | "logs"
                    | "dist"
                    | "out"
                    | "__pycache__"
                    | ".venv"
                    | "venv"
                    | ".pytest_cache"
                    | ".mypy_cache"
                    | "generated"
                    | ".generated"
                    | "artifacts"
                    | "resources"
                    | "archive"
                    | "archives"
                    | "task-archives"
                    | "task_archives"
                    | "task_archive"
                    | ".agents"
                    | ".codex"
                    | ".kimi-code"
                    | ".claude"
                    | ".cursor"
                    | "agent-defs"
                    | "role"
            )
        })
}

fn allowed_target(target: &Path, scope: Scope) -> Result<()> {
    relative_path(target)?;
    ensure!(
        !target.starts_with(".codex/skills/.system"),
        "platform skills cannot be managed by a bundle"
    );
    ensure!(
        target != Path::new(".claude.json") || scope == Scope::User,
        "global Claude configuration requires user scope"
    );
    let nested_rule = scope == Scope::Project
        && target
            .file_name()
            .is_some_and(|name| name == "AGENTS.md" || name == "CLAUDE.md")
        && target.parent().is_some_and(|parent| {
            parent
                .components()
                .all(|component| !excluded_rule_tree(Path::new(component.as_os_str())))
        });
    let allowed = [
        "AGENTS.md",
        "CLAUDE.md",
        ".codex/AGENTS.md",
        ".kimi-code/AGENTS.md",
        ".claude/AGENTS.md",
        ".claude/CLAUDE.md",
    ]
    .iter()
    .any(|path| target == Path::new(path))
        || nested_rule
        || config_kind(target).is_some()
        || [
            ".agents/skills",
            ".claude/skills",
            ".claude/agents",
            ".claude/commands",
            ".claude/prompts",
            ".claude/rules",
            ".claude/hooks",
            ".claude/scripts",
            ".cursor/rules",
            ".codex/skills",
            ".codex/agents",
            ".codex/prompts",
            ".codex/rules",
            ".codex/hooks",
            ".kimi-code/skills",
            ".kimi-code/agents",
            ".kimi-code/prompts",
            ".kimi-code/rules",
            ".kimi-code/hooks",
        ]
        .into_iter()
        .chain((scope == Scope::Project).then_some("agent-defs"))
        .any(|prefix| target.starts_with(prefix) && target != Path::new(prefix));
    ensure!(allowed, "unsupported bundle target: {}", target.display());
    Ok(())
}

pub(crate) fn adoptable_file_target(target: &Path, scope: Scope) -> bool {
    allowed_target(target, scope).is_ok() && config_kind(target).is_none()
}

fn bind_local(value: &mut Value, current: Option<&Value>, path: &str) -> Result<()> {
    match value {
        Value::String(marker) if marker == "${SM_LOCAL}" => {
            *value = current
                .ok_or_else(|| anyhow::anyhow!("local configuration value missing: {path}"))?
                .clone();
        }
        Value::Object(values) => {
            for (key, value) in values {
                bind_local(
                    value,
                    current.and_then(|value| value.get(key)),
                    &format!("{path}.{key}"),
                )?;
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter_mut().enumerate() {
                bind_local(
                    value,
                    current.and_then(|value| value.get(index)),
                    &format!("{path}[{index}]"),
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn fragment(
    mut values: Map<String, Value>,
    target: &Path,
    toml: bool,
) -> Result<Map<String, Value>> {
    let current: Value = if target.exists() {
        let text = fs::read_to_string(target)?;
        if toml {
            toml_edit::de::from_str(&text)
                .map_err(|_| anyhow::anyhow!("local configuration is not valid TOML"))?
        } else {
            serde_json::from_str(&text)
                .map_err(|_| anyhow::anyhow!("local configuration is not valid JSON"))?
        }
    } else {
        Value::Null
    };
    for (key, value) in &mut values {
        bind_local(value, current.get(key), key)?;
    }
    Ok(values)
}

pub fn render(resource: &Resource, payload: &Path, scope: &Path) -> Result<Vec<OwnedWrite>> {
    let files = payload_files(payload)?;
    let manifest: Manifest = serde_json::from_slice(&fs::read(payload.join("bundle.json"))?)?;
    manifest
        .entries
        .into_iter()
        .map(|entry| {
            let relative = match &entry {
                Entry::File { target, .. }
                | Entry::Json { target, .. }
                | Entry::Toml { target, .. } => target,
            };
            allowed_target(relative, resource.scope)?;
            if resource.scope == Scope::Project && relative == Path::new(".kimi-code/hooks.json") {
                return Err(crate::providers::UnsupportedResource.into());
            }
            let config = config_kind(relative);
            let kimi_project_config =
                resource.scope == Scope::Project && relative == Path::new(".kimi-code/config.toml");
            let target = scope.join(relative);
            reject_symlink_path(&target)?;
            let body = match entry {
                Entry::File { source, .. } => {
                    ensure!(
                        config.is_none(),
                        "native configuration requires a bundle fragment"
                    );
                    let path = contained(payload, &source)?;
                    ensure!(
                        files.iter().any(|(file, _)| *file == path),
                        "bundle source is not a payload file: {}",
                        source.display()
                    );
                    WriteBody::File {
                        bytes: fs::read(&path)?,
                        executable: fs::metadata(&path)?.permissions().mode() & 0o111 != 0,
                    }
                }
                Entry::Json { json, .. } => {
                    ensure!(
                        config == Some(false),
                        "JSON fragment requires a native JSON configuration target"
                    );
                    WriteBody::Json(fragment(json, &target, false)?)
                }
                Entry::Toml { toml, .. } => {
                    if kimi_project_config && toml.contains_key("hooks") {
                        return Err(crate::providers::UnsupportedResource.into());
                    }
                    ensure!(
                        config == Some(true),
                        "TOML fragment requires a native TOML configuration target"
                    );
                    WriteBody::Toml(fragment(toml, &target, true)?)
                }
            };
            Ok(OwnedWrite::new(&resource.id, target, body))
        })
        .collect()
}
