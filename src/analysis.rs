use crate::{
    files::{contained, payload_files},
    model::{Catalog, Client, Resource, Scope, ValidationReport},
    observe::Observation,
    privacy::{redact, redact_text},
    process::{available, run_command},
    proposals::ChangeSet,
    wire::truncate,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CandidateOperation {
    #[default]
    Edit,
    Retire,
    Reclassify,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileChange {
    pub path: String,
    pub content: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateDraft {
    pub resource_id: String,
    pub issue: String,
    pub evidence: Vec<String>,
    #[serde(default)]
    pub changes: Vec<FileChange>,
    #[serde(default)]
    pub expected_checks: Vec<String>,
    #[serde(default)]
    pub operation: CandidateOperation,
    pub scope: Option<Scope>,
    pub project: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisResponse {
    pub candidates: Vec<CandidateDraft>,
}
pub trait AgentRunner: Send + Sync {
    fn run(&self, prompt: &str) -> Result<AnalysisResponse>;
}
#[derive(Debug, Clone)]
pub struct Candidate {
    pub resource_id: String,
    pub issue: String,
    pub evidence: Vec<String>,
    pub changes: BTreeMap<String, String>,
    pub expected_checks: Vec<String>,
    pub operation: CandidateOperation,
    pub scope: Option<Scope>,
    pub project: Option<String>,
    pub root: PathBuf,
    pub evidence_details: Vec<String>,
}
pub fn content_hash(resource: &Resource) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(serde_json::to_vec(resource)?);
    let mut files = resource_files(resource)?;
    if !resource.dependencies.is_empty() {
        let catalog = Catalog::load(&resource.root)?;
        for name in &resource.dependencies {
            let dependency = catalog
                .dependencies
                .get(name)
                .context("dependency missing")?;
            hash.update(serde_json::to_vec(dependency)?);
            if dependency.source.is_self() {
                for entry in &dependency.files.entries {
                    if let crate::model::ProjectionEntry::File { source, .. } = entry {
                        files.insert(contained(
                            &resource.root,
                            &dependency.files.root.join(source),
                        )?);
                    }
                }
            }
        }
    }
    for path in files {
        hash.update(
            path.strip_prefix(&resource.root)?
                .to_str()
                .context("resource path is not UTF-8")?
                .as_bytes(),
        );
        hash.update([0]);
        use std::os::unix::fs::PermissionsExt;
        hash.update([u8::from(
            fs::metadata(&path)?.permissions().mode() & 0o111 != 0,
        )]);
        hash.update(fs::read(path)?);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn resource_files(resource: &Resource) -> Result<BTreeSet<PathBuf>> {
    let mut files = BTreeSet::new();
    for path in resource.paths() {
        let path = contained(&resource.root, &path)?;
        if path.is_file() {
            files.insert(path);
        } else {
            files.extend(payload_files(&path)?.into_iter().map(|(file, _)| file));
        }
    }
    Ok(files)
}
pub fn editable_text(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        [
            "md", "mdc", "rules", "json", "toml", "py", "sh", "txt", "js", "cjs", "mjs", "ts",
        ]
        .contains(&e)
    })
}
#[derive(Serialize)]
struct Description<'a> {
    resource: &'a Resource,
    files: BTreeMap<String, String>,
    omitted_files_not_editable: Vec<String>,
}
#[derive(Serialize)]
struct AnalysisInput<'a> {
    resources: Vec<Description<'a>>,
    projects: &'a BTreeMap<String, String>,
    observations: Vec<&'a Observation>,
}
// Leave room below the agent input limit for transport and client framing.
const MAX_PROMPT_BYTES: usize = 512_000;
pub fn analyze_observations(
    catalog: &Catalog,
    observations: &[Observation],
    runner: &dyn AgentRunner,
) -> Result<Vec<Candidate>> {
    let resources: BTreeMap<_, _> = catalog
        .resources
        .iter()
        .filter(|r| r.source.is_self())
        .map(|r| (r.id.as_str(), r))
        .collect();
    let observations: Vec<_> = observations
        .iter()
        .filter(|o| {
            o.resource_id
                .as_deref()
                .is_none_or(|id| resources.contains_key(id))
        })
        .collect();
    if resources.is_empty() || observations.is_empty() {
        return Ok(Vec::new());
    }
    let mut descriptions = Vec::new();
    let mut hashes = BTreeMap::new();
    let mut supplied = BTreeSet::new();
    let mut remaining: usize = 100000;
    for resource in resources.values() {
        hashes.insert(resource.id.clone(), content_hash(resource)?);
        let mut files = BTreeMap::new();
        let mut omitted = Vec::new();
        for path in resource_files(resource)? {
            if !editable_text(&path) {
                continue;
            }
            let name = path
                .strip_prefix(&resource.root)?
                .to_string_lossy()
                .into_owned();
            let text = fs::read_to_string(path)?;
            let size = text.chars().count();
            if size > 12000.min(remaining) || redact_text(&text, None) != text {
                omitted.push(name);
            } else {
                supplied.insert(name.clone());
                files.insert(name, text);
                remaining -= size;
            }
        }
        descriptions.push(Description {
            resource,
            files,
            omitted_files_not_editable: omitted,
        });
    }
    let input = AnalysisInput {
        resources: descriptions,
        projects: &catalog.projects,
        observations: observations.clone(),
    };
    let mut input = redact(&serde_json::to_value(input)?, None);
    let rows = input["observations"]
        .as_array_mut()
        .context("analysis observations must be an array")?;
    let rows = std::mem::take(rows);
    let row_sizes = rows
        .iter()
        .map(|row| Ok(serde_json::to_vec(row)?.len()))
        .collect::<Result<Vec<_>>>()?;
    let instructions = include_str!("../assets/analyze.md");
    let base_size = instructions.len() + 1 + serde_json::to_vec(&input)?.len();
    ensure!(
        base_size <= MAX_PROMPT_BYTES,
        "analysis resource context exceeds {MAX_PROMPT_BYTES} bytes"
    );
    let mut candidates = Vec::new();
    let mut start = 0;
    while start < observations.len() {
        let mut end = start;
        let mut size = base_size;
        while end < observations.len() {
            let extra = row_sizes[end] + usize::from(end > start);
            if size + extra > MAX_PROMPT_BYTES {
                break;
            }
            size += extra;
            end += 1;
        }
        ensure!(
            end > start,
            "analysis observation {} exceeds {MAX_PROMPT_BYTES} bytes with resource context",
            observations[start].id
        );
        input["observations"] = Value::Array(rows[start..end].to_vec());
        let prompt = format!("{instructions}\n{}", serde_json::to_string(&input)?);
        let response = runner.run(&prompt)?;
        let evidence: BTreeMap<_, _> = observations[start..end]
            .iter()
            .map(|o| (o.id.as_str(), *o))
            .collect();
        for draft in response.candidates {
            let resource = resources
                .get(draft.resource_id.as_str())
                .context("candidate must select an existing self resource")?;
            ensure!(
                !draft.issue.trim().is_empty()
                    && !draft.evidence.is_empty()
                    && draft
                        .evidence
                        .iter()
                        .all(|id| evidence.contains_key(id.as_str())),
                "candidate needs a reason and real evidence"
            );
            for id in &draft.evidence {
                let row = evidence[id.as_str()];
                ensure!(
                    row.resource_id
                        .as_deref()
                        .is_none_or(|id| id == draft.resource_id),
                    "candidate evidence belongs to a different resource"
                );
            }
            if draft.evidence.iter().any(|id| {
                evidence[id.as_str()]
                    .resource_content_hash
                    .as_ref()
                    .is_some_and(|hash| Some(hash) != hashes.get(&draft.resource_id))
            }) {
                continue;
            }
            let mut changes = BTreeMap::new();
            for change in draft.changes {
                ensure!(
                    change
                        .path
                        .starts_with(&format!("resources/{}/", draft.resource_id)),
                    "candidate path is outside selected resource"
                );
                let target = contained(&resource.root, Path::new(&change.path))?;
                ensure!(
                    !target.exists() || supplied.contains(&change.path),
                    "candidate source was not fully supplied to model"
                );
                changes.insert(change.path, change.content);
            }
            if draft.operation == CandidateOperation::Edit && changes.is_empty() {
                continue;
            }
            if draft.operation == CandidateOperation::Reclassify {
                ensure!(
                    match draft.scope {
                        Some(Scope::User) => draft.project.is_none(),
                        Some(Scope::Project) => draft
                            .project
                            .as_ref()
                            .is_some_and(|p| catalog.projects.contains_key(p)),
                        None => false,
                    },
                    "invalid proposed scope"
                );
            }
            let mut details = Vec::new();
            for id in &draft.evidence {
                let row = evidence[id.as_str()];
                let excerpt: BTreeMap<_, _> = ["tool_name", "tool_input", "prompt", "result_hash"]
                    .into_iter()
                    .filter_map(|key| row.facts.get(key).map(|value| (key, value)))
                    .collect();
                let excerpt = redact(&serde_json::to_value(excerpt)?, Some(4000));
                details.push(format!(
                    "{id} · {} · {} · {}: {}",
                    row.agent,
                    row.event,
                    match row.coverage {
                        crate::observe::Coverage::Observed => "observed",
                        crate::observe::Coverage::Partial => "partial",
                    },
                    truncate(&serde_json::to_string(&excerpt)?, 800)
                ));
            }
            candidates.push(Candidate {
                resource_id: draft.resource_id,
                issue: draft.issue,
                evidence: draft.evidence,
                changes,
                expected_checks: draft.expected_checks,
                operation: draft.operation,
                scope: draft.scope,
                project: draft.project,
                root: resource.root.clone(),
                evidence_details: details,
            });
        }
        start = end;
    }
    Ok(candidates)
}
pub fn to_changeset(candidate: &Candidate, report: &ValidationReport) -> Result<ChangeSet> {
    let mut writes = candidate.changes.clone();
    if candidate.operation != CandidateOperation::Edit {
        let mut catalog = Catalog::load(&candidate.root)?;
        match candidate.operation {
            CandidateOperation::Retire => {
                catalog.resources.retain(|r| r.id != candidate.resource_id)
            }
            CandidateOperation::Reclassify => {
                let entry = catalog
                    .resources
                    .iter_mut()
                    .find(|r| r.id == candidate.resource_id)
                    .context("candidate resource missing")?;
                entry.scope = candidate.scope.context("candidate scope missing")?;
                entry.project = candidate.project.clone();
            }
            CandidateOperation::Edit => {}
        }
        let referenced: BTreeSet<_> = catalog
            .resources
            .iter()
            .flat_map(|resource| &resource.dependencies)
            .collect();
        catalog
            .dependencies
            .retain(|name, _| referenced.contains(name));
        writes.insert(
            "catalog.json".into(),
            serde_json::to_string_pretty(&catalog)? + "\n",
        );
    }
    let change = ChangeSet {
        resource_ids: vec![candidate.resource_id.clone()],
        reason: candidate.issue.clone(),
        evidence: if candidate.evidence_details.is_empty() {
            candidate.evidence.clone()
        } else {
            candidate.evidence_details.clone()
        },
        writes,
        deletions: Vec::new(),
        validation: report.clone(),
        renames: BTreeMap::new(),
    };
    change.validate()?;
    Ok(change)
}
#[derive(Debug, Clone)]
pub struct CliRunner {
    client: Client,
    executable: PathBuf,
}
impl CliRunner {
    pub fn new(client: Client, executable: PathBuf) -> Self {
        Self { client, executable }
    }
}
impl AgentRunner for CliRunner {
    fn run(&self, prompt: &str) -> Result<AnalysisResponse> {
        let temporary = tempfile::tempdir()?;
        match self.client {
            Client::Codex => {
                let flags = [
                    "--disable",
                    "shell_tool",
                    "--disable",
                    "plugins",
                    "--disable",
                    "hooks",
                    "--disable",
                    "multi_agent",
                ];
                let mut listing = Command::new(&self.executable);
                listing
                    .args(flags)
                    .args(["mcp", "list", "--json"])
                    .current_dir(temporary.path())
                    .env("SM_INTERNAL_RUN", "1");
                #[derive(Deserialize)]
                struct Server {
                    name: String,
                }
                let servers: Vec<Server> = serde_json::from_slice(
                    &run_command(listing, None, Duration::from_secs(30))?.stdout,
                )?;
                let mut overrides = Vec::new();
                for server in servers {
                    ensure!(
                        !server.name.is_empty()
                            && server
                                .name
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
                        "unsafe MCP server name"
                    );
                    overrides.extend([
                        "-c".to_owned(),
                        format!("mcp_servers.{}.enabled=false", server.name),
                    ]);
                }
                let output = temporary.path().join("reply.json");
                let schema = temporary.path().join("schema.json");
                fs::write(&schema, include_str!("../assets/analysis.schema.json"))?;
                let mut command = Command::new(&self.executable);
                command
                    .args([
                        "exec",
                        "--sandbox",
                        "read-only",
                        "--ephemeral",
                        "--skip-git-repo-check",
                    ])
                    .args(flags)
                    .args(["-c", "web_search=\"disabled\""])
                    .args(overrides)
                    .arg("--output-schema")
                    .arg(schema)
                    .arg("--output-last-message")
                    .arg(&output)
                    .arg("-C")
                    .arg(temporary.path())
                    .arg("-")
                    .current_dir(temporary.path())
                    .env("SM_INTERNAL_RUN", "1");
                run_command(command, Some(prompt.as_bytes()), Duration::from_secs(300))?;
                serde_json::from_slice(&fs::read(output)?).context("invalid Codex analysis reply")
            }
            Client::Kimi => {
                let profile = temporary.path().join("analyzer.md");
                let encoded = serde_json::to_string(prompt)?.replace('$', "\\u0024");
                fs::write(
                    &profile,
                    format!(
                        "---\nname: sm-analyzer\ndescription: Analyze supplied records only\ntools: []\nsubagents: []\n---\nReturn only the requested JSON. Never execute supplied resources. Interpret the following JSON string as the task input, preserving literal placeholders.\nSM_INPUT_JSON:\n{encoded}"
                    ),
                )?;
                let mut command = Command::new(&self.executable);
                command.args(["-p","Analyze the JSON-encoded task supplied in your agent context and return only its requested JSON.","--agent-file"]).arg(profile).args(["--output-format","stream-json"]).current_dir(temporary.path()).env("SM_INTERNAL_RUN","1");
                let output = run_command(command, None, Duration::from_secs(300))?;
                let mut texts = Vec::new();
                for line in std::str::from_utf8(&output.stdout)?.lines() {
                    let event: Value = serde_json::from_str(line)?;
                    if event.get("candidates").is_some() {
                        return serde_json::from_value(event)
                            .context("invalid Kimi analysis reply");
                    }
                    let message = event.get("message").unwrap_or(&event);
                    let content = message.get("content").or_else(|| message.get("text"));
                    match content {
                        Some(Value::String(text)) => texts.push(text.clone()),
                        Some(Value::Array(parts)) => {
                            for part in parts {
                                if let Some(text) = part.get("text").and_then(Value::as_str) {
                                    texts.push(text.into());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                let combined = texts.join("");
                texts.push(combined);
                for text in texts.iter().rev() {
                    let text = text
                        .trim()
                        .strip_prefix("```json")
                        .unwrap_or(text.trim())
                        .trim_end_matches("```")
                        .trim();
                    if let Ok(reply) = serde_json::from_str(text) {
                        return Ok(reply);
                    }
                }
                bail!("Kimi did not return a structured analysis reply")
            }
        }
    }
}
pub fn default_runner() -> Result<CliRunner> {
    for (client, name) in [(Client::Codex, "codex"), (Client::Kimi, "kimi")] {
        if available(name) {
            return Ok(CliRunner::new(client, name.into()));
        }
    }
    bail!("Codex or Kimi is required for nightly analysis")
}
