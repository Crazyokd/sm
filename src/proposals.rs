use crate::{
    files::{contained, relative_path},
    model::{Catalog, Scope, ValidationReport, valid_id},
    privacy::redact_text,
    process::run_command,
    repository::git,
    state::State,
    wire::{digest, truncate},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Debug, thiserror::Error)]
#[error("proposal has no changes against approved content")]
pub struct NoChanges;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeSet {
    pub resource_ids: Vec<String>,
    pub reason: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub writes: BTreeMap<String, String>,
    #[serde(default)]
    pub deletions: Vec<String>,
    #[serde(default)]
    pub validation: ValidationReport,
    #[serde(default)]
    pub renames: BTreeMap<String, String>,
}
impl ChangeSet {
    pub fn validate(&self) -> Result<()> {
        let ids: BTreeSet<_> = self.resource_ids.iter().collect();
        ensure!(
            !ids.is_empty()
                && ids.len() == self.resource_ids.len()
                && self.resource_ids.iter().all(|id| valid_id(id)),
            "valid unique resource ids are required"
        );
        ensure!(
            !self.reason.trim().is_empty()
                && (!self.writes.is_empty()
                    || !self.deletions.is_empty()
                    || !self.renames.is_empty()),
            "change requires reason and changes"
        );
        for path in self.writes.keys().chain(&self.deletions) {
            relative_path(Path::new(path))?;
            if path != "catalog.json" {
                let parts: Vec<_> = Path::new(path).components().collect();
                ensure!(
                    parts.len() >= 3
                        && parts[0].as_os_str() == "resources"
                        && parts[1].as_os_str().to_str().is_some_and(|id| self
                            .resource_ids
                            .iter()
                            .any(|selected| selected == id)),
                    "path is outside selected resources: {path}"
                );
            }
            ensure!(
                !(self.writes.contains_key(path) && self.deletions.contains(path)),
                "a path cannot be written and deleted"
            );
        }
        let mut destinations = BTreeSet::new();
        for (old, new) in &self.renames {
            ensure!(
                old != new
                    && ids.contains(old)
                    && ids.contains(new)
                    && valid_id(old)
                    && valid_id(new)
                    && destinations.insert(new),
                "rename requires unique selected source/destination ids"
            );
        }
        for text in std::iter::once(&self.reason)
            .chain(&self.evidence)
            .chain(self.writes.values())
        {
            ensure!(
                redact_text(text, None) == *text,
                "credential value must remain local"
            );
        }
        let validation = serde_json::to_string(&self.validation)?;
        ensure!(
            redact_text(&validation, None) == validation,
            "credential value must remain local"
        );
        Ok(())
    }
    pub fn key(&self) -> Result<String> {
        let mut ids = self.resource_ids.clone();
        ids.sort();
        Ok(digest(serde_json::to_vec(&json!([ids, self.reason]))?)[..16].into())
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProposalStatus {
    Draft,
    Submitted,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub key: String,
    pub directory: PathBuf,
    pub branch: String,
    pub change: ChangeSet,
    pub status: ProposalStatus,
    pub mr_url: Option<String>,
}
impl Proposal {
    pub fn save(&self, state: &State) -> Result<()> {
        let mut values: BTreeMap<String, Self> = state.get("proposals")?;
        values.insert(self.key.clone(), self.clone());
        state.set("proposals", &values)
    }
    pub fn description(&self) -> Result<String> {
        let evidence = if self.change.evidence.is_empty() {
            "- 用户主动修改".into()
        } else {
            self.change
                .evidence
                .iter()
                .map(|e| format!("- {e}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(format!(
            "## 问题与证据\n\n{}\n\n{evidence}\n\n## 修改依据\n\n纳管资源：{}\n\n## 修改内容\n\n参见 MR 文件差异；原有运行位置未直接修改。\n\n## 对比验证与未验证项\n\n```json\n{}\n```\n\n## 影响范围\n\n资源归属及客户端原生适配以 catalog.json 为准。\n",
            self.change.reason,
            self.change.resource_ids.join(", "),
            serde_json::to_string_pretty(&self.change.validation)?
        ))
    }
}
pub fn create_proposal(repo: &Path, change: &ChangeSet, state: &State) -> Result<Proposal> {
    change.validate()?;
    let key = change.key()?;
    let previous: BTreeMap<String, Proposal> = state.get("proposals")?;
    let previous = previous.get(&key);
    let directory = state.directory()?.join("proposals").join(&key);
    let branch = format!("sm/{key}");
    let parent = directory.parent().context("proposal parent missing")?;
    fs::create_dir_all(parent)?;
    if previous.is_some() {
        ensure!(
            directory.join(".git").is_dir(),
            "saved proposal directory is missing"
        );
    }
    let remote = git(repo, &["remote", "get-url", "origin"])?;
    let staging = tempfile::Builder::new()
        .prefix(".prepare-")
        .tempdir_in(parent)?;
    let prepared = staging.path().join("repo");
    let mut command = Command::new("git");
    command.args([
        "clone",
        "--branch",
        if previous.is_some() { &branch } else { "main" },
        "--",
    ]);
    if previous.is_some() {
        command.arg(&directory);
    } else {
        command.arg(&remote);
    }
    command.arg(&prepared);
    run_command(command, None, Duration::from_secs(120))?;
    git(&prepared, &["remote", "set-url", "origin", &remote])?;
    for setting in ["user.name", "user.email"] {
        git(
            &prepared,
            &["config", setting, &git(repo, &["config", setting])?],
        )?;
    }
    if previous.is_some() {
        git(&prepared, &["fetch", "origin", "main"])?;
        git(&prepared, &["merge", "--no-edit", "origin/main"])?;
    } else {
        git(&prepared, &["switch", "-c", &branch])?;
    }
    let original = Catalog::load(&prepared)?;
    let before: BTreeMap<_, _> = original
        .resources
        .iter()
        .map(|r| (r.id.clone(), r))
        .collect();
    for (old, new) in &change.renames {
        let source = before.get(old).context("rename source missing")?;
        ensure!(
            !before.contains_key(new),
            "rename destination already exists"
        );
        let old = prepared.join("resources").join(old);
        let new = prepared.join("resources").join(new);
        ensure!(!new.exists(), "rename destination already exists");
        if source.source.is_self() {
            fs::rename(old, new)?;
        }
    }
    for path in change.writes.keys().chain(&change.deletions) {
        if path != "catalog.json" {
            let id = Path::new(path)
                .components()
                .nth(1)
                .and_then(|c| c.as_os_str().to_str())
                .context("resource path is missing id")?;
            let original_id = change
                .renames
                .iter()
                .find_map(|(old, new)| (new == id).then_some(old.as_str()))
                .unwrap_or(id);
            ensure!(
                before.get(original_id).is_none_or(|r| r.source.is_self()),
                "upstream content is not a local optimization target"
            );
        }
        contained(&prepared, Path::new(path))?;
    }
    for (path, text) in &change.writes {
        let target = contained(&prepared, Path::new(path))?;
        fs::create_dir_all(target.parent().context("write target has no parent")?)?;
        fs::write(target, text)?;
    }
    for path in &change.deletions {
        let target = contained(&prepared, Path::new(path))?;
        if target.exists() {
            fs::remove_file(target)?;
        }
    }
    let updated = Catalog::load(&prepared)?;
    let after: BTreeMap<_, _> = updated
        .resources
        .iter()
        .map(|r| (r.id.clone(), r))
        .collect();
    let ids: BTreeSet<_> = before.keys().chain(after.keys()).collect();
    for id in ids {
        if !change.resource_ids.contains(id) {
            ensure!(
                before.get(id) == after.get(id),
                "catalog edits affect resources outside selected ids"
            );
        }
    }
    for resource in &original.resources {
        if resource.scope == Scope::Project && !change.resource_ids.contains(&resource.id) {
            let project = resource.project.as_ref().context("project missing")?;
            ensure!(
                original.projects.get(project) == updated.projects.get(project),
                "project mapping affects resources outside selected ids"
            );
        }
    }
    for (old, new) in &change.renames {
        ensure!(
            !after.contains_key(old) && after.contains_key(new),
            "catalog must reflect resource rename"
        );
    }
    git(&prepared, &["add", "--all"])?;
    if git(&prepared, &["diff", "origin/main", "--name-only"])?.is_empty() {
        return Err(NoChanges.into());
    }
    if !git(&prepared, &["diff", "--cached", "--name-only"])?.is_empty() {
        git(
            &prepared,
            &[
                "commit",
                "-m",
                &truncate(change.reason.lines().next().unwrap_or_default(), 150),
            ],
        )?;
    }
    let backup = staging.path().join("previous");
    if directory.exists() {
        fs::rename(&directory, &backup)?;
    }
    if let Err(error) = fs::rename(&prepared, &directory) {
        if backup.exists() {
            fs::rename(backup, &directory).context("restore previous proposal")?;
        }
        return Err(error.into());
    }
    let proposal = Proposal {
        key,
        directory,
        branch,
        change: change.clone(),
        status: previous.map_or(ProposalStatus::Draft, |p| p.status),
        mr_url: previous.and_then(|p| p.mr_url.clone()),
    };
    proposal.save(state)?;
    Ok(proposal)
}
