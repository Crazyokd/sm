use crate::{
    bundles::excluded_rule_tree,
    model::{Catalog, Scope, StoredBody, StoredWrite},
    privacy::redact_text,
    state::State,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Take},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

const MAX_ENTRIES: usize = 20_000;
const MAX_DEPTH: usize = 16;
const MAX_CONFIG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_HASH_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub generated_at: String,
    pub entries: Vec<InventoryEntry>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryEntry {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub kind: String,
    pub path: PathBuf,
    pub scope: Scope,
    pub project: Option<String>,
    pub owner: Option<String>,
    pub status: String,
    pub enabled: Option<bool>,
    #[serde(default)]
    pub enabled_source: Option<String>,
    pub alias_of: Option<PathBuf>,
}

impl Default for InventoryEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: String::new(),
            path: PathBuf::new(),
            scope: Scope::User,
            project: None,
            owner: None,
            status: String::new(),
            enabled: None,
            enabled_source: None,
            alias_of: None,
        }
    }
}

#[derive(Deserialize)]
struct ExternalInstallation {
    id: String,
    paths: Vec<PathBuf>,
}

struct Scanner {
    home: PathBuf,
    owned: Vec<StoredWrite>,
    native: Vec<(PathBuf, String)>,
    enabled: BTreeMap<PathBuf, bool>,
    skill_paths: BTreeMap<PathBuf, bool>,
    skill_names: BTreeMap<(PathBuf, String), bool>,
    entries: BTreeMap<String, InventoryEntry>,
    errors: BTreeSet<String>,
    visited: usize,
}

/// Reads resource locations and configuration identities without returning resource contents.
pub fn scan(catalog: &Catalog, state: &State, home: &Path) -> Result<Inventory> {
    let home = home.canonicalize()?;
    let mut scanner = Scanner {
        home: home.clone(),
        owned: state.owned()?,
        native: state
            .get::<Vec<ExternalInstallation>>("external_installations")?
            .into_iter()
            .flat_map(|installation| {
                installation
                    .paths
                    .into_iter()
                    .map(move |path| (path, installation.id.clone()))
            })
            .collect(),
        enabled: BTreeMap::new(),
        skill_paths: BTreeMap::new(),
        skill_names: BTreeMap::new(),
        entries: BTreeMap::new(),
        errors: BTreeSet::new(),
        visited: 0,
    };
    let projects: Vec<_> = catalog
        .projects
        .iter()
        .map(|(name, path)| {
            let path = path
                .strip_prefix("~/")
                .map(|path| home.join(path))
                .unwrap_or_else(|| PathBuf::from(path));
            (name, path)
        })
        .collect();
    scanner.root(&home, Scope::User, None);
    for (project, path) in &projects {
        if !path.is_dir() {
            scanner.error(path, "registered project is missing or unreadable");
            continue;
        }
        scanner.root(path, Scope::Project, Some(project));
        scanner.project_rules(path, project, &projects, 0);
    }
    // Retain missing projections as drift instead of silently dropping their ownership.
    for write in scanner.owned.clone() {
        let represented = scanner.entries.values().any(|entry| {
            entry.owner.as_deref() == Some(&write.resource_id)
                && (entry.path == write.path
                    || (entry.kind == "skill" && write.path.starts_with(&entry.path)))
        });
        let missing_fragment = !scanner.matches(&write)
            && !scanner.entries.values().any(|entry| {
                entry.owner.as_deref() == Some(&write.resource_id)
                    && entry.status == "drift"
                    && (entry.path == write.path
                        || (entry.kind == "skill" && write.path.starts_with(&entry.path)))
            });
        if !represented || missing_fragment {
            let (scope, project) = projects
                .iter()
                .filter(|(_, root)| write.path.starts_with(root))
                .max_by_key(|(_, root)| root.components().count())
                .map_or((Scope::User, None), |(name, _)| {
                    (Scope::Project, Some(name.as_str()))
                });
            let kind = match write.body {
                StoredBody::Markdown(_) => "rule",
                StoredBody::Json(_) | StoredBody::Toml(_) => "config",
                StoredBody::FileFingerprint { .. } => "file",
            };
            let id = format!("{}#owned/{}", write.path.display(), write.resource_id);
            let base = redact_text(&write.path.to_string_lossy(), None);
            let existing = scanner.entries.contains_key(&base);
            scanner.entry(&write.path, kind, scope, project, None, None);
            if let Some(mut entry) = scanner.entries.get(&base).cloned() {
                if !existing {
                    scanner.entries.remove(&base);
                }
                entry.id = id.clone();
                entry.owner = Some(write.resource_id.clone());
                entry.status = if scanner.matches(&write) {
                    "managed"
                } else {
                    "drift"
                }
                .into();
                scanner.entries.insert(id, entry);
            }
        }
    }
    let enabled: Vec<_> = scanner
        .entries
        .iter()
        .filter_map(|(id, entry)| {
            let path = entry.alias_of.as_ref().unwrap_or(&entry.path);
            scanner
                .skill_enablement(path, &entry.kind)
                .map(|(enabled, source)| (id.clone(), enabled, source))
        })
        .collect();
    for (id, enabled, source) in enabled {
        let entry = scanner
            .entries
            .get_mut(&id)
            .expect("existing inventory entry");
        entry.enabled = Some(enabled);
        entry.enabled_source = source;
    }
    Ok(Inventory {
        generated_at: chrono::Utc::now().to_rfc3339(),
        entries: scanner.entries.into_values().collect(),
        errors: scanner.errors.into_iter().collect(),
    })
}

fn resource_name(path: &Path, kind: &str, logical: Option<(&str, &Value, &[String])>) -> String {
    let fallback = || {
        path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    };
    let name = logical
        .map(|(label, value, _)| match kind {
            "skill-config" => value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    value.get("path").and_then(Value::as_str).map(|path| {
                        Path::new(path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    })
                })
                .unwrap_or_else(fallback),
            "hook" => value
                .get("event")
                .and_then(Value::as_str)
                .or_else(|| {
                    label
                        .split('/')
                        .find(|part| *part != "hooks" && !part.bytes().all(|b| b.is_ascii_digit()))
                })
                .map(str::to_owned)
                .unwrap_or_else(fallback),
            "mcp" | "plugin" | "agent" => label
                .split_once('/')
                .map(|(_, name)| name.to_owned())
                .unwrap_or_else(fallback),
            _ => fallback(),
        })
        .unwrap_or_else(fallback);
    redact_text(&name, Some(180))
}

impl Scanner {
    fn project_rules(
        &mut self,
        path: &Path,
        project: &str,
        projects: &[(&String, PathBuf)],
        depth: usize,
    ) {
        if path.is_symlink() || self.resolve(path).is_none() || !self.budget(path, depth) {
            return;
        }
        for name in ["AGENTS.md", "CLAUDE.md"] {
            let rule = path.join(name);
            if !rule.is_symlink() {
                self.optional_file(&rule, "rule", Scope::Project, Some(project));
            }
        }
        for child in self.children(path) {
            if self.visited >= MAX_ENTRIES {
                self.error(path, "inventory scan limit reached; coverage is incomplete");
                break;
            }
            if excluded_rule_tree(&child)
                || child.is_symlink()
                || !child.is_dir()
                || projects
                    .iter()
                    .any(|(name, root)| name.as_str() != project && child == *root)
            {
                continue;
            }
            self.project_rules(&child, project, projects, depth + 1);
        }
    }

    fn skill_enablement(&self, path: &Path, kind: &str) -> Option<(bool, Option<String>)> {
        if kind == "skill" {
            if let Some((_, enabled)) = self
                .skill_paths
                .iter()
                .filter(|(root, _)| path.starts_with(root))
                .max_by_key(|(root, _)| root.components().count())
            {
                return Some((*enabled, Some("codex:skills.config".into())));
            }
            let name = path.file_name()?.to_str()?;
            if let Some((_, enabled)) = self
                .skill_names
                .iter()
                .filter(|((root, configured), _)| {
                    if let Ok(relative) = path.strip_prefix(root.join(".codex/plugins/cache")) {
                        let plugin = relative
                            .components()
                            .nth(1)
                            .and_then(|part| part.as_os_str().to_str());
                        plugin.is_some_and(|plugin| *configured == format!("{plugin}:{name}"))
                    } else {
                        (path.starts_with(root.join(".agents/skills"))
                            || path.starts_with(root.join(".codex/skills")))
                            && configured == name
                    }
                })
                .max_by_key(|((root, _), _)| root.components().count())
            {
                return Some((*enabled, Some("codex:skills.config".into())));
            }
        }
        self.enabled
            .iter()
            .filter(|(root, _)| path.starts_with(root))
            .max_by_key(|(root, _)| root.components().count())
            .map(|(_, enabled)| (*enabled, None))
    }
    fn error(&mut self, path: &Path, message: &str) {
        if self.errors.len() < 100 {
            self.errors.insert(redact_text(
                &format!("{}: {message}", path.display()),
                Some(800),
            ));
        }
    }

    fn budget(&mut self, path: &Path, depth: usize) -> bool {
        self.visited += 1;
        if self.visited > MAX_ENTRIES || depth > MAX_DEPTH {
            self.error(path, "inventory scan limit reached; coverage is incomplete");
            return false;
        }
        true
    }

    fn present(&mut self, path: &Path) -> bool {
        match fs::symlink_metadata(path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => {
                self.error(path, "resource location could not be inspected");
                false
            }
        }
    }

    fn resolve(&mut self, path: &Path) -> Option<PathBuf> {
        match path.canonicalize() {
            Ok(resolved) => {
                let linked = path.ancestors().any(Path::is_symlink);
                if linked && !resolved.starts_with(&self.home) {
                    self.error(path, "symlink target outside home was not scanned");
                    None
                } else {
                    Some(resolved)
                }
            }
            Err(_) => {
                self.error(path, "resource is missing, unreadable, or a cyclic link");
                None
            }
        }
    }

    fn children(&mut self, path: &Path) -> Vec<PathBuf> {
        let children = match fs::read_dir(path) {
            Ok(children) => children,
            Err(_) => {
                self.error(path, "resource directory could not be read");
                return Vec::new();
            }
        };
        let mut result = Vec::new();
        for child in children.take(MAX_ENTRIES + 1) {
            match child {
                Ok(child) => result.push(child.path()),
                Err(_) => self.error(path, "resource directory entry could not be read"),
            }
        }
        if result.len() > MAX_ENTRIES {
            self.error(path, "inventory scan limit reached; coverage is incomplete");
            result.truncate(MAX_ENTRIES);
        }
        result.sort();
        result
    }

    fn root(&mut self, root: &Path, scope: Scope, project: Option<&str>) {
        if self.resolve(root).is_none() {
            return;
        }
        for file in ["AGENTS.md", "CLAUDE.md"] {
            self.optional_file(&root.join(file), "rule", scope, project);
        }
        for base in [".codex", ".kimi-code", ".claude"] {
            let base = root.join(base);
            self.optional_file(&base.join("AGENTS.md"), "rule", scope, project);
            if base.file_name().is_some_and(|name| name == ".claude") {
                self.optional_file(&base.join("CLAUDE.md"), "rule", scope, project);
                if scope == Scope::User {
                    self.directory(
                        &base.join("scripts"),
                        "hook-support",
                        scope,
                        project,
                        0,
                        &mut BTreeSet::new(),
                    );
                }
            }
            for (folder, kind) in [
                ("skills", "skill"),
                ("agents", "agent"),
                ("agent-defs", "agent"),
                ("role", "agent"),
                ("prompts", "prompt"),
                ("commands", "prompt"),
                ("rules", "rule"),
                ("hooks", "hook"),
            ] {
                self.directory(
                    &base.join(folder),
                    kind,
                    scope,
                    project,
                    0,
                    &mut BTreeSet::new(),
                );
            }
            for filename in ["config.toml", "mcp.json", "hooks.json", "settings.json"] {
                self.config(&base.join(filename), scope, project);
            }
        }
        self.directory(
            &root.join(".agents/skills"),
            "skill",
            scope,
            project,
            0,
            &mut BTreeSet::new(),
        );
        for folder in ["agent-defs", "role"] {
            self.directory(
                &root.join(folder),
                "agent",
                scope,
                project,
                0,
                &mut BTreeSet::new(),
            );
        }
        self.directory(
            &root.join(".cursor/rules"),
            "rule",
            scope,
            project,
            0,
            &mut BTreeSet::new(),
        );
        self.config(&root.join(".mcp.json"), scope, project);
        if scope == Scope::User {
            self.config(&root.join(".claude.json"), scope, project);
        }
        self.plugins(root, scope, project);
    }

    fn optional_file(&mut self, path: &Path, kind: &str, scope: Scope, project: Option<&str>) {
        if self.present(path) && self.resolve(path).is_some() && path.is_file() {
            self.entry(path, kind, scope, project, None, None);
        }
    }

    fn directory(
        &mut self,
        path: &Path,
        kind: &str,
        scope: Scope,
        project: Option<&str>,
        depth: usize,
        ancestors: &mut BTreeSet<PathBuf>,
    ) {
        if !self.present(path) || !self.budget(path, depth) {
            return;
        }
        let Some(resolved) = self.resolve(path) else {
            return;
        };
        if !ancestors.insert(resolved.clone()) {
            self.error(path, "cyclic directory alias was not scanned");
            return;
        }
        if path.is_dir() {
            if kind == "skill" && path.join("SKILL.md").is_file() {
                self.entry(path, kind, scope, project, None, None);
            } else {
                for child in self.children(path) {
                    if excluded(&child) {
                        continue;
                    }
                    self.directory(&child, kind, scope, project, depth + 1, ancestors);
                }
            }
        } else if kind != "skill" && path.is_file() {
            self.entry(path, kind, scope, project, None, None);
        }
        ancestors.remove(&resolved);
    }

    fn entry(
        &mut self,
        path: &Path,
        kind: &str,
        scope: Scope,
        project: Option<&str>,
        logical: Option<(&str, &Value, &[String])>,
        enabled: Option<bool>,
    ) {
        let resolved = path.canonicalize().ok();
        let lookup = resolved.as_deref().unwrap_or(path);
        let id = redact_text(
            &format!(
                "{}{}",
                path.display(),
                logical.map_or(String::new(), |(label, _, _)| format!("#{label}"))
            ),
            None,
        );
        if self.entries.contains_key(&id) {
            return;
        }
        if self.entries.len() >= MAX_ENTRIES {
            self.error(path, "inventory scan limit reached; coverage is incomplete");
            return;
        }
        if fs::metadata(path).is_ok_and(|metadata| metadata.len() > MAX_HASH_BYTES) {
            self.error(
                path,
                "file exceeds fingerprint limit; coverage is incomplete",
            );
        }
        let mut owner = None;
        let mut status = "unmanaged";
        for write in &self.owned {
            let file = write.path == lookup || write.path == path;
            let skill = kind == "skill" && write.path.starts_with(lookup);
            if !file && !skill {
                continue;
            }
            let matches = if let Some((_, value, selector)) = logical {
                match &write.body {
                    StoredBody::Json(fragment) | StoredBody::Toml(fragment) => {
                        let fragment = Value::Object(fragment.clone());
                        let Some(matches) = owned_at(&fragment, selector, value) else {
                            continue;
                        };
                        matches
                    }
                    _ => self.matches(write),
                }
            } else {
                self.matches(write)
            };
            if owner.is_none() || !matches {
                owner = Some(write.resource_id.clone());
            }
            if !matches {
                status = "drift";
            } else if status != "drift" {
                status = "managed";
            }
        }
        let native_owner = self.native_owner(lookup).or_else(|| {
            logical.and_then(|(_, _, selector)| {
                if kind == "plugin" && selector.first().is_some_and(|part| part == "plugins") {
                    self.plugin_path(selector.get(1)?)
                        .and_then(|path| self.native_owner(&path))
                } else {
                    None
                }
            })
        });
        if let Some(id) = native_owner {
            if status == "unmanaged" || id == "platform" {
                status = "native";
                owner = Some(id);
            }
        }
        let enabled = enabled.or_else(|| {
            self.enabled
                .iter()
                .filter(|(path, _)| lookup.starts_with(path))
                .max_by_key(|(path, _)| path.components().count())
                .map(|(_, enabled)| *enabled)
        });
        self.entries.insert(
            id.clone(),
            InventoryEntry {
                id,
                name: resource_name(path, kind, logical),
                kind: kind.into(),
                path: path.to_path_buf(),
                scope,
                project: project.map(str::to_owned),
                owner,
                status: status.into(),
                enabled,
                enabled_source: None,
                alias_of: resolved.filter(|resolved| resolved != path),
            },
        );
    }

    fn native_owner(&self, path: &Path) -> Option<String> {
        if path.starts_with(self.home.join(".codex/skills/.system"))
            || path.starts_with(self.home.join(".codex/plugins/cache/openai-bundled"))
            || path.starts_with(
                self.home
                    .join(".codex/plugins/cache/openai-primary-runtime"),
            )
        {
            Some("platform".into())
        } else {
            self.native
                .iter()
                .find(|(root, _)| path.starts_with(root))
                .map(|(_, id)| id.clone())
        }
    }

    fn plugin_path(&self, name: &str) -> Option<PathBuf> {
        let (plugin, market) = name.split_once('@')?;
        if [plugin, market].iter().any(|name| {
            name.is_empty() || Path::new(name).components().count() != 1 || name == &".."
        }) {
            return None;
        }
        Some(
            self.home
                .join(".codex/plugins/cache")
                .join(market)
                .join(plugin),
        )
    }

    fn matches(&self, write: &StoredWrite) -> bool {
        if write.path.ancestors().any(Path::is_symlink) {
            return false;
        }
        match &write.body {
            StoredBody::FileFingerprint { sha256, executable } => {
                let Ok(metadata) = fs::metadata(&write.path) else {
                    return false;
                };
                if !metadata.is_file()
                    || metadata.len() > MAX_HASH_BYTES
                    || (metadata.permissions().mode() & 0o111 != 0) != *executable
                {
                    return false;
                }
                let Ok(file) = fs::File::open(&write.path) else {
                    return false;
                };
                let mut hash = Sha256::new();
                let mut reader = file.take(MAX_HASH_BYTES + 1);
                let mut buffer = [0; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => return <[u8; 32]>::from(hash.finalize()) == *sha256,
                        Ok(count) => hash.update(&buffer[..count]),
                        Err(_) => return false,
                    }
                }
            }
            StoredBody::Markdown(bytes) => {
                let Some(text) = read_text(&write.path) else {
                    return false;
                };
                let Ok(body) = std::str::from_utf8(bytes) else {
                    return false;
                };
                let block = format!(
                    "<!-- SM:{} -->\n{}\n<!-- /SM:{} -->\n",
                    write.resource_id,
                    body.trim_end(),
                    write.resource_id
                );
                text.matches(&block).count() == 1
            }
            StoredBody::Json(fragment) | StoredBody::Toml(fragment) => read_config(&write.path)
                .is_some_and(|value| contains(&value, &Value::Object(fragment.clone()))),
        }
    }

    fn config(&mut self, path: &Path, scope: Scope, project: Option<&str>) {
        if !self.present(path) || self.resolve(path).is_none() {
            return;
        }
        let Some(value) = read_config(path) else {
            self.error(
                path,
                "configuration could not be parsed within the size limit",
            );
            return;
        };
        for (section, kind) in [
            ("mcp_servers", "mcp"),
            ("mcpServers", "mcp"),
            ("plugins", "plugin"),
            ("agents", "agent"),
        ] {
            if let Some(entries) = value.get(section).and_then(Value::as_object) {
                for (name, entry) in entries {
                    let selector = vec![section.into(), name.clone()];
                    self.entry(
                        path,
                        kind,
                        scope,
                        project,
                        Some((&format!("{section}/{name}"), entry, &selector)),
                        is_enabled(entry),
                    );
                    if section == "plugins" {
                        if let Some(root) = self.plugin_path(name) {
                            if let Some(enabled) = is_enabled(entry) {
                                self.enabled.insert(root, enabled);
                            }
                        }
                    }
                }
            }
        }
        if let Some(skills) = value
            .get("skills")
            .and_then(|skills| skills.get("config"))
            .and_then(Value::as_array)
        {
            for (index, skill) in skills.iter().enumerate() {
                let selector = vec!["skills".into(), "config".into(), index.to_string()];
                self.entry(
                    path,
                    "skill-config",
                    scope,
                    project,
                    Some((&format!("skills/config/{index}"), skill, &selector)),
                    is_enabled(skill),
                );
                let codex = path
                    .parent()
                    .is_some_and(|parent| parent.file_name().is_some_and(|name| name == ".codex"));
                if !codex {
                    continue;
                }
                if let (Some(name), Some(enabled)) =
                    (skill.get("name").and_then(Value::as_str), is_enabled(skill))
                {
                    if let Some(root) = path.parent().and_then(Path::parent) {
                        self.skill_names
                            .insert((root.to_path_buf(), name.to_owned()), enabled);
                    }
                }
                if let (Some(path), Some(enabled)) =
                    (skill.get("path").and_then(Value::as_str), is_enabled(skill))
                {
                    let path = path
                        .strip_prefix("~/")
                        .map(|path| self.home.join(path))
                        .unwrap_or_else(|| PathBuf::from(path));
                    if path.is_absolute() {
                        let path = if path.file_name().is_some_and(|name| name == "SKILL.md") {
                            path.parent().unwrap().to_path_buf()
                        } else {
                            path
                        };
                        self.skill_paths.insert(path, enabled);
                    }
                }
            }
        }
        if let Some(hooks) = value.get("hooks") {
            self.hooks(path, hooks, vec!["hooks".into()], scope, project);
        }
    }

    fn hooks(
        &mut self,
        path: &Path,
        value: &Value,
        selector: Vec<String>,
        scope: Scope,
        project: Option<&str>,
    ) {
        match value {
            Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    let mut selector = selector.clone();
                    selector.push(index.to_string());
                    self.hooks(path, value, selector, scope, project);
                }
            }
            Value::Object(object) => {
                if object.contains_key("command")
                    || object.contains_key("type")
                    || object.contains_key("prompt")
                {
                    let event = object
                        .get("event")
                        .and_then(Value::as_str)
                        .map(|event| format!("/{event}"))
                        .unwrap_or_default();
                    self.entry(
                        path,
                        "hook",
                        scope,
                        project,
                        Some((&format!("{}{event}", selector.join("/")), value, &selector)),
                        is_enabled(value),
                    );
                } else {
                    for (key, value) in object {
                        let mut selector = selector.clone();
                        selector.push(key.clone());
                        self.hooks(path, value, selector, scope, project);
                    }
                }
            }
            _ => {}
        }
    }

    fn plugins(&mut self, root: &Path, scope: Scope, project: Option<&str>) {
        let installed = root.join(".kimi-code/plugins/installed.json");
        if installed.exists() {
            if let Some(value) = read_config(&installed) {
                if let Some(plugins) = value.get("plugins").and_then(Value::as_array) {
                    for plugin in plugins {
                        let Some(path) = plugin
                            .get("root")
                            .and_then(Value::as_str)
                            .map(PathBuf::from)
                        else {
                            continue;
                        };
                        if !path.is_absolute()
                            || !path.starts_with(&self.home)
                            || path
                                .components()
                                .any(|component| component == std::path::Component::ParentDir)
                            || self.resolve(&path).is_none()
                        {
                            self.error(&installed, "plugin root outside home was not scanned");
                            continue;
                        }
                        if let Some(enabled) = is_enabled(plugin) {
                            self.enabled.insert(path.clone(), enabled);
                        }
                        self.entry(&path, "plugin", scope, project, None, is_enabled(plugin));
                        self.plugin_resources(&path, scope, project, 0);
                    }
                }
            } else {
                self.error(
                    &installed,
                    "plugin registry could not be parsed within the size limit",
                );
            }
        }
        let cache = root.join(".codex/plugins/cache");
        if cache.is_dir() && self.resolve(&cache).is_some() {
            for market in self.children(&cache) {
                if !self.budget(&market, 0) {
                    return;
                }
                if !market.is_dir() || self.resolve(&market).is_none() {
                    continue;
                }
                for package in self.children(&market) {
                    if !self.budget(&package, 0) {
                        return;
                    }
                    if !package.is_dir() || self.resolve(&package).is_none() {
                        continue;
                    }
                    self.entry(&package, "plugin", scope, project, None, None);
                    self.plugin_resources(&package, scope, project, 0);
                }
            }
        }
        // Configuration is read before files so disabled package/skill flags apply to every alias.
        for entry in self.entries.values_mut() {
            if entry.enabled.is_none() {
                let path = entry.alias_of.as_ref().unwrap_or(&entry.path);
                entry.enabled = self
                    .enabled
                    .iter()
                    .filter(|(root, _)| path.starts_with(root))
                    .max_by_key(|(root, _)| root.components().count())
                    .map(|(_, enabled)| *enabled);
            }
        }
    }

    fn plugin_resources(&mut self, path: &Path, scope: Scope, project: Option<&str>, depth: usize) {
        if !self.budget(path, depth) || self.resolve(path).is_none() {
            return;
        }
        for child in self.children(path) {
            if excluded(&child) || !child.is_dir() {
                continue;
            }
            let kind = match child.file_name().and_then(|name| name.to_str()) {
                Some("skills") => Some("skill"),
                Some("agents" | "agent-defs" | "role") => Some("agent"),
                Some("hooks") => Some("hook"),
                Some("rules") => Some("rule"),
                Some("prompts" | "commands") => Some("prompt"),
                _ => None,
            };
            if let Some(kind) = kind {
                self.directory(
                    &child,
                    kind,
                    scope,
                    project,
                    depth + 1,
                    &mut BTreeSet::new(),
                );
            } else if depth < 3 {
                self.plugin_resources(&child, scope, project, depth + 1);
            }
        }
        self.optional_file(&path.join("AGENTS.md"), "rule", scope, project);
        self.config(&path.join("hooks/hooks.json"), scope, project);
        self.config(&path.join(".mcp.json"), scope, project);
    }
}

fn excluded(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                ".git"
                    | "node_modules"
                    | "target"
                    | "cache"
                    | "caches"
                    | "sessions"
                    | "archived_sessions"
                    | "__pycache__"
                    | ".venv"
                    | "venv"
                    | "site-packages"
                    | "site_packages"
                    | ".cache"
            )
        })
}

fn is_enabled(value: &Value) -> Option<bool> {
    value
        .as_bool()
        .or_else(|| value.get("enabled").and_then(Value::as_bool))
        .or_else(|| {
            value
                .get("disabled")
                .and_then(Value::as_bool)
                .map(|disabled| !disabled)
        })
}

fn read_text(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES {
        return None;
    }
    let file = fs::File::open(path).ok()?;
    let mut text = String::new();
    let mut reader: Take<fs::File> = file.take(MAX_CONFIG_BYTES + 1);
    reader.read_to_string(&mut text).ok()?;
    (text.len() as u64 <= MAX_CONFIG_BYTES).then_some(text)
}

fn read_config(path: &Path) -> Option<Value> {
    let text = read_text(path)?;
    if path
        .extension()
        .is_some_and(|extension| extension == "toml")
    {
        toml_edit::de::from_str(&text).ok()
    } else {
        serde_json::from_str(&text).ok()
    }
}

fn owned_at(value: &Value, selector: &[String], actual: &Value) -> Option<bool> {
    let Some((key, rest)) = selector.split_first() else {
        return Some(value == actual);
    };
    match value {
        // Array entries merge by exact value; their stored positions are not ownership identities.
        Value::Array(values) if key.parse::<usize>().is_ok() => values
            .iter()
            .any(|value| owned_at(value, rest, actual) == Some(true))
            .then_some(true),
        _ => owned_at(value.get(key)?, rest, actual),
    }
}

fn contains(value: &Value, owned: &Value) -> bool {
    match (value, owned) {
        (Value::Object(value), Value::Object(owned)) => owned
            .iter()
            .all(|(key, owned)| value.get(key).is_some_and(|value| contains(value, owned))),
        (Value::Array(value), Value::Array(owned)) => {
            owned.iter().all(|owned| value.contains(owned))
        }
        _ => value == owned,
    }
}
