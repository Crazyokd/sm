use crate::{
    files::{contained, payload_files, reject_symlink_path, relative_path},
    model::{
        FileProjection, OwnedWrite, ProjectionEntry, Resource, ResourceContent, ResourceKind,
        Scope, WriteBody,
    },
};
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value};
use std::{collections::BTreeSet, fs, os::unix::fs::PermissionsExt, path::Path};

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
        "platform skills cannot be managed"
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
    ensure!(allowed, "unsupported resource target: {}", target.display());
    Ok(())
}

pub fn support_target(path: &Path) -> bool {
    relative_path(path).is_ok()
        && config_kind(path).is_none()
        && [
            ".claude/scripts",
            ".claude/hooks",
            ".codex/hooks",
            ".kimi-code/hooks",
        ]
        .into_iter()
        .any(|prefix| path.starts_with(prefix) && path != Path::new(prefix))
}

pub fn validate_support(files: &FileProjection) -> Result<()> {
    relative_path(&files.root)?;
    ensure!(
        !files.entries.is_empty(),
        "supporting files require entries"
    );
    let mut targets = BTreeSet::new();
    for entry in &files.entries {
        let ProjectionEntry::File { target, source } = entry else {
            anyhow::bail!("supporting dependency permits only file entries");
        };
        ensure!(
            support_target(target),
            "unsupported supporting file target: {}",
            target.display()
        );
        relative_path(source)?;
        ensure!(
            !source.components().any(|part| part.as_os_str() == ".git"),
            "Git internals are not supporting files"
        );
        ensure!(targets.insert(target), "duplicate supporting file target");
    }
    Ok(())
}

pub fn render_support(
    id: &str,
    files: &FileProjection,
    payload: &Path,
    scope: &Path,
) -> Result<Vec<OwnedWrite>> {
    validate_support(files)?;
    files
        .entries
        .iter()
        .map(|entry| {
            let ProjectionEntry::File { target, source } = entry else {
                unreachable!("validated file entries")
            };
            let target = scope.join(target);
            reject_symlink_path(&target)?;
            let path = selected_file(payload, source)?;
            Ok(OwnedWrite::new(
                id,
                target,
                WriteBody::File {
                    bytes: fs::read(&path)?,
                    executable: fs::metadata(&path)?.permissions().mode() & 0o111 != 0,
                },
            ))
        })
        .collect()
}

pub(crate) fn adoptable_file_target(target: &Path, scope: Scope) -> bool {
    allowed_target(target, scope).is_ok() && config_kind(target).is_none()
}

fn has_local(value: &Value) -> bool {
    match value {
        Value::String(marker) => marker == "${SM_LOCAL}",
        Value::Object(values) => values.values().any(has_local),
        Value::Array(values) => values.iter().any(has_local),
        _ => false,
    }
}

fn matches_local_template(template: &Value, current: &Value) -> bool {
    match (template, current) {
        (Value::String(marker), _) if marker == "${SM_LOCAL}" => true,
        (Value::Object(template), Value::Object(current)) => {
            template.iter().all(|(key, template)| {
                current
                    .get(key)
                    .is_some_and(|current| matches_local_template(template, current))
            })
        }
        (Value::Array(template), Value::Array(current)) if template.iter().any(has_local) => {
            if template.len() == current.len() {
                template
                    .iter()
                    .zip(current)
                    .all(|(template, current)| matches_local_template(template, current))
            } else {
                template.len() < current.len()
                    && template.iter().all(|template| {
                        current
                            .iter()
                            .any(|current| matches_local_template(template, current))
                    })
            }
        }
        _ => template == current,
    }
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
        Value::Array(values) if values.iter().any(has_local) => {
            let current = current
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow::anyhow!("local configuration array missing: {path}"))?;
            // Scalar arguments have explicit slots; structured handlers need a unique identity.
            let positional = values.len() == current.len()
                && values.iter().enumerate().all(|(index, value)| {
                    !has_local(value)
                        || (matches_local_template(value, &current[index])
                            && ((!current[index].is_object() && !current[index].is_array())
                                || current
                                    .iter()
                                    .filter(|candidate| matches_local_template(value, candidate))
                                    .count()
                                    == 1))
                });
            let mut selected = BTreeSet::new();
            for (index, value) in values
                .iter_mut()
                .enumerate()
                .filter(|(_, value)| has_local(value))
            {
                let candidate = if positional {
                    index
                } else {
                    let mut candidates = current
                        .iter()
                        .enumerate()
                        .filter(|(_, current)| matches_local_template(value, current));
                    let (candidate, _) = candidates.next().ok_or_else(|| {
                        anyhow::anyhow!("local configuration array item missing: {path}[{index}]")
                    })?;
                    ensure!(
                        candidates.next().is_none(),
                        "local configuration array item ambiguous: {path}[{index}]"
                    );
                    candidate
                };
                ensure!(
                    selected.insert(candidate),
                    "local configuration array item reused: {path}[{index}]"
                );
                bind_local(
                    value,
                    Some(&current[candidate]),
                    &format!("{path}[{index}]"),
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn fragment(
    kind: ResourceKind,
    mut values: Map<String, Value>,
    target: &Path,
    toml: bool,
    mcp_names: &mut BTreeSet<String>,
) -> Result<Map<String, Value>> {
    validate_fragment(kind, &values, mcp_names)?;
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

fn file_kind(target: &Path, scope: Scope) -> Option<ResourceKind> {
    if target.starts_with("agent-defs") && scope == Scope::Project {
        return Some(ResourceKind::Agent);
    }
    for (directory, kind) in [
        ("skills", ResourceKind::Skill),
        ("agents", ResourceKind::Agent),
        ("rules", ResourceKind::Rule),
        ("prompts", ResourceKind::Prompt),
        ("commands", ResourceKind::Prompt),
        ("hooks", ResourceKind::Hook),
        ("scripts", ResourceKind::Hook),
    ] {
        for client in [".agents", ".codex", ".kimi-code", ".claude", ".cursor"] {
            let prefix = Path::new(client).join(directory);
            if target.starts_with(prefix) {
                return Some(kind);
            }
        }
    }
    let name = target.file_name()?;
    if name == "AGENTS.md" || name == "CLAUDE.md" {
        return Some(ResourceKind::Rule);
    }
    None
}

fn validate_fragment(
    kind: ResourceKind,
    values: &Map<String, Value>,
    mcp_names: &mut BTreeSet<String>,
) -> Result<()> {
    ensure!(
        !values.is_empty(),
        "configuration fragment must not be empty"
    );
    for section in values.keys() {
        let actual = match section.as_str() {
            "hooks" => ResourceKind::Hook,
            "mcp_servers" | "mcpServers" => ResourceKind::Mcp,
            "skills" => ResourceKind::Policy,
            _ => anyhow::bail!("unsupported resource configuration section: {section}"),
        };
        ensure!(
            actual == kind,
            "configuration fragment mixes resource kinds"
        );
        if kind == ResourceKind::Mcp {
            let servers = values[section]
                .as_object()
                .context("MCP services must be an object")?;
            ensure!(
                servers.len() == 1 && servers.keys().all(|name| !name.trim().is_empty()),
                "MCP fragment must contain one server identity"
            );
            mcp_names.extend(servers.keys().cloned());
            ensure!(
                mcp_names.len() == 1,
                "projection contains different MCP server identities"
            );
        }
    }
    Ok(())
}

pub fn validate(kind: ResourceKind, files: &FileProjection, scope: Scope) -> Result<()> {
    relative_path(&files.root)?;
    ensure!(
        !files.entries.is_empty(),
        "file projection requires entries"
    );
    let mut targets = BTreeSet::new();
    let mut mcp_names = BTreeSet::new();
    let mut skill_names = BTreeSet::new();
    let mut skill_marker = false;
    for entry in &files.entries {
        let target = match entry {
            ProjectionEntry::File { target, .. }
            | ProjectionEntry::Tree { target, .. }
            | ProjectionEntry::Fragment { target, .. }
            | ProjectionEntry::Json { target, .. }
            | ProjectionEntry::Toml { target, .. } => target,
        };
        allowed_target(target, scope)?;
        ensure!(
            targets.insert(target),
            "duplicate projection target: {}",
            target.display()
        );
        if let ProjectionEntry::File { source, .. }
        | ProjectionEntry::Tree { tree: source, .. }
        | ProjectionEntry::Fragment {
            fragment: source, ..
        } = entry
        {
            relative_path(source)?;
            ensure!(
                !source.components().any(|part| part.as_os_str() == ".git"),
                "Git internals are not resource payloads"
            );
        }
        match entry {
            ProjectionEntry::File { .. } => {
                ensure!(
                    config_kind(target).is_none(),
                    "native configuration requires a resource fragment"
                );
                ensure!(
                    file_kind(target, scope) == Some(kind),
                    "file projection mixes resource kinds"
                );
                if kind == ResourceKind::Skill {
                    let mut components = target.components();
                    components.next();
                    components.next();
                    let name = components.next().context("Skill target requires a name")?;
                    ensure!(
                        !components.as_path().as_os_str().is_empty(),
                        "Skill target requires a file"
                    );
                    skill_names.insert(name.as_os_str());
                    skill_marker |= components.as_path() == Path::new("SKILL.md");
                }
            }
            ProjectionEntry::Tree { .. } => {
                ensure!(
                    kind == ResourceKind::Skill,
                    "tree projection requires Skill kind"
                );
                let name = [
                    ".agents/skills",
                    ".codex/skills",
                    ".kimi-code/skills",
                    ".claude/skills",
                ]
                .into_iter()
                .find_map(|prefix| {
                    target
                        .strip_prefix(prefix)
                        .ok()
                        .filter(|relative| relative.components().count() == 1)
                        .and_then(Path::file_name)
                })
                .context("tree target must be one Skill directory")?;
                skill_names.insert(name);
                skill_marker = true;
            }
            ProjectionEntry::Fragment { .. } => {
                ensure!(
                    config_kind(target).is_some(),
                    "configuration fragment requires a native configuration target"
                );
            }
            ProjectionEntry::Json { json, .. } => {
                ensure!(
                    config_kind(target) == Some(false),
                    "JSON fragment requires a native JSON configuration target"
                );
                validate_fragment(kind, json, &mut mcp_names)?;
            }
            ProjectionEntry::Toml { toml, .. } => {
                ensure!(
                    config_kind(target) == Some(true),
                    "TOML fragment requires a native TOML configuration target"
                );
                validate_fragment(kind, toml, &mut mcp_names)?;
            }
        }
    }
    if kind == ResourceKind::Skill {
        ensure!(
            skill_names.len() == 1 && skill_marker,
            "Skill projection must contain one Skill name and SKILL.md"
        );
    }
    Ok(())
}

fn selected_file(payload: &Path, source: &Path) -> Result<std::path::PathBuf> {
    reject_symlink_path(payload)?;
    let path = contained(payload, source)?;
    reject_symlink_path(&path)?;
    ensure!(
        path.is_file(),
        "projection source is not a regular file: {}",
        source.display()
    );
    Ok(path)
}

pub fn render(resource: &Resource, payload: &Path, scope: &Path) -> Result<Vec<OwnedWrite>> {
    let ResourceContent::Files { kind, files } = &resource.content else {
        anyhow::bail!("file projection required");
    };
    validate(*kind, files, resource.scope)?;
    let mut mcp_names = BTreeSet::new();
    let mut writes = Vec::new();
    for entry in &files.entries {
        let relative = match entry {
            ProjectionEntry::File { target, .. }
            | ProjectionEntry::Tree { target, .. }
            | ProjectionEntry::Fragment { target, .. }
            | ProjectionEntry::Json { target, .. }
            | ProjectionEntry::Toml { target, .. } => target,
        };
        if resource.scope == Scope::Project
            && *kind == ResourceKind::Hook
            && matches!(
                relative.to_str(),
                Some(".kimi-code/hooks.json" | ".kimi-code/config.toml")
            )
        {
            return Err(crate::providers::UnsupportedResource.into());
        }
        let target = scope.join(relative);
        reject_symlink_path(&target)?;
        let body = match entry {
            ProjectionEntry::File { source, .. } => {
                let path = selected_file(payload, source)?;
                WriteBody::File {
                    bytes: fs::read(&path)?,
                    executable: fs::metadata(&path)?.permissions().mode() & 0o111 != 0,
                }
            }
            ProjectionEntry::Tree { tree, .. } => {
                reject_symlink_path(payload)?;
                let root = contained(payload, tree)?;
                reject_symlink_path(&root)?;
                let files = payload_files(&root)?;
                ensure!(
                    files
                        .iter()
                        .any(|(_, relative)| relative == Path::new("SKILL.md")),
                    "selected Skill marker missing"
                );
                for (path, relative) in files {
                    writes.push(OwnedWrite::new(
                        &resource.id,
                        target.join(relative),
                        WriteBody::File {
                            bytes: fs::read(&path)?,
                            executable: fs::metadata(&path)?.permissions().mode() & 0o111 != 0,
                        },
                    ));
                }
                continue;
            }
            ProjectionEntry::Fragment {
                fragment: source, ..
            } => {
                let path = selected_file(payload, source)?;
                let values: Map<String, Value> =
                    serde_json::from_slice(&fs::read(path)?).map_err(|_| {
                        anyhow::anyhow!("resource configuration fragment must be a JSON object")
                    })?;
                let toml = config_kind(relative).context("configuration target required")?;
                let values = fragment(*kind, values, &target, toml, &mut mcp_names)?;
                if toml {
                    WriteBody::Toml(values)
                } else {
                    WriteBody::Json(values)
                }
            }
            ProjectionEntry::Json { json, .. } => WriteBody::Json(fragment(
                *kind,
                json.clone(),
                &target,
                false,
                &mut mcp_names,
            )?),
            ProjectionEntry::Toml { toml, .. } => WriteBody::Toml(fragment(
                *kind,
                toml.clone(),
                &target,
                true,
                &mut mcp_names,
            )?),
        };
        writes.push(OwnedWrite::new(&resource.id, target, body));
    }
    Ok(writes)
}
