use crate::{
    analysis::{
        AgentRunner, analyze_observations, content_hash, default_runner, editable_text,
        to_changeset,
    },
    apply::{apply_writes, plan_apply},
    evaluation::evaluate_candidate,
    files::{atomic_write, contained, copy_tree, payload_files, reject_symlink_path},
    gitlab,
    model::{
        ApplyReport, ApplyStatus, Catalog, Client, ResourceResult, ValidationReport,
        ValidationStatus,
    },
    observe::{
        ActiveIndex, ActiveOwned, complete_observations, ingest_events, pending_observations,
    },
    privacy::redact_text,
    process::{available, run_command},
    proposals::{NoChanges, Proposal},
    repository::{fetch_approved, fetch_upstream},
    state::State,
};
use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct Runtime {
    pub repo: PathBuf,
    pub home: PathBuf,
    pub executable: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub repo: PathBuf,
}
impl Runtime {
    pub fn state(&self) -> Result<State> {
        State::open(self.home.join(".local/state/sm/state.sqlite3"))
    }
    pub fn discover(repo: Option<&Path>) -> Result<Self> {
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME is unavailable")?);
        let executable = std::env::current_exe()?;
        let config = home.join(".config/sm/config.json");
        let repo = if let Some(repo) = repo {
            repo.to_path_buf()
        } else if config.is_file() {
            serde_json::from_slice::<Config>(&fs::read(config)?)?.repo
        } else {
            let cwd = std::env::current_dir()?;
            cwd.ancestors()
                .find(|path| path.join("catalog.json").is_file())
                .map(Path::to_path_buf)
                .or_else(|| {
                    executable
                        .ancestors()
                        .find(|path| path.join("catalog.json").is_file())
                        .map(Path::to_path_buf)
                })
                .context("run ./sm init from the checkout or pass --repo")?
        };
        Ok(Self {
            repo: repo.canonicalize()?,
            home,
            executable,
        })
    }
}
#[derive(Debug, thiserror::Error)]
#[error("SM job already running")]
pub struct Busy;
pub struct JobGuard(File);
impl JobGuard {
    pub fn acquire(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory)?;
        let path = directory.join("job.lock");
        reject_symlink_path(&path)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Self(file)),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Err(Busy.into()),
            Err(error) => Err(error.into()),
        }
    }
}
impl Drop for JobGuard {
    fn drop(&mut self) {
        if let Err(error) = FileExt::unlock(&self.0) {
            eprintln!("SM lock release failed: {error}");
        }
    }
}
pub trait Systemd: Send + Sync {
    fn run(&self, args: &[&str]) -> Result<()>;
}
pub struct NativeSystemd;
impl Systemd for NativeSystemd {
    fn run(&self, args: &[&str]) -> Result<()> {
        let mut command = Command::new("systemctl");
        command.arg("--user").args(args);
        run_command(command, None, Duration::from_secs(30))?;
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitDetails {
    pub url: String,
    pub tools: BTreeMap<String, bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub services: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitReport {
    pub ok: bool,
    pub details: InitDetails,
}
fn unit_quote(text: &str) -> Result<String> {
    Ok(serde_json::to_string(text)?.replace('%', "%%"))
}
pub fn initialize(runtime: &Runtime) -> Result<InitReport> {
    initialize_with(runtime, &NativeSystemd)
}
pub fn initialize_with(runtime: &Runtime, systemd: &dyn Systemd) -> Result<InitReport> {
    let state = runtime.state()?;
    let tools: BTreeMap<_, _> = ["git", "codex", "kimi", "glab"]
        .into_iter()
        .map(|name| (name.to_owned(), available(name)))
        .collect();
    let mut details = InitDetails {
        url: "http://127.0.0.1:8765".into(),
        tools,
        services: None,
        error: None,
    };
    if !details.tools.get("git").copied().unwrap_or(false) {
        details.error = Some("Git is required".into());
        let report = InitReport { ok: false, details };
        state.add_run("init", &report)?;
        return Ok(report);
    }
    let config = Config {
        repo: runtime.repo.canonicalize()?,
    };
    atomic_write(
        &runtime.home.join(".config/sm/config.json"),
        &(serde_json::to_string_pretty(&config)? + "\n").into_bytes(),
        None,
    )?;
    let substitutions = BTreeMap::from([
        (
            "executable",
            unit_quote(&runtime.executable.to_string_lossy())?,
        ),
        ("repo", unit_quote(&config.repo.to_string_lossy())?),
        (
            "path",
            unit_quote(&format!(
                "PATH={}",
                std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
            ))?,
        ),
    ]);
    let tokens = regex::Regex::new(r"\{(executable|repo|path)\}")?;
    for (name, template) in [
        (
            "sm-web.service",
            include_str!("../assets/systemd/sm-web.service"),
        ),
        (
            "sm-cycle.service",
            include_str!("../assets/systemd/sm-cycle.service"),
        ),
        (
            "sm-cycle.timer",
            include_str!("../assets/systemd/sm-cycle.timer"),
        ),
    ] {
        let content = tokens.replace_all(template, |captures: &regex::Captures<'_>| {
            substitutions.get(&captures[1]).cloned().unwrap_or_default()
        });
        atomic_write(
            &runtime.home.join(".config/systemd/user").join(name),
            content.as_bytes(),
            None,
        )?;
    }
    let register = || -> Result<()> {
        systemd.run(&["daemon-reload"])?;
        systemd.run(&["enable", "sm-web.service", "sm-cycle.timer"])?;
        systemd.run(&["start", "sm-cycle.timer"])?;
        systemd.run(&["restart", "sm-web.service"])
    };
    let ok = match register() {
        Ok(()) => {
            details.services = Some("registered".into());
            true
        }
        Err(error) => {
            details.error = Some(redact_text(&format!("{error:#}"), Some(1200)));
            false
        }
    };
    let report = InitReport { ok, details };
    state.add_run("init", &report)?;
    Ok(report)
}
pub fn apply_source(
    source: &Path,
    runtime: &Runtime,
    state: &State,
) -> Result<(Catalog, ApplyReport)> {
    let catalog = Catalog::load(source)?;
    let temporary = tempfile::tempdir()?;
    let mut sources = BTreeMap::new();
    let mut errors = BTreeMap::new();
    for resource in &catalog.resources {
        let result = if resource.source.is_self() {
            Ok(source.to_path_buf())
        } else {
            fetch_upstream(resource, &temporary.path().join(&resource.id))
        };
        match result {
            Ok(path) => {
                sources.insert(resource.id.clone(), path);
            }
            Err(error) => {
                errors.insert(
                    resource.id.clone(),
                    ResourceResult::new(
                        ApplyStatus::Failed,
                        redact_text(&format!("{error:#}"), Some(1200)),
                    ),
                );
            }
        }
    }
    let mut plan = plan_apply(&catalog, &sources, &runtime.home);
    plan.results.extend(errors);
    let clients: BTreeSet<_> = catalog
        .resources
        .iter()
        .filter(|r| r.source.is_self())
        .flat_map(|r| r.targets().into_keys())
        .collect();
    for client in &clients {
        plan.writes.push(crate::providers::collector(
            *client,
            &runtime.home,
            &runtime.executable,
            state.directory()?,
        ));
    }
    let report = apply_writes(&plan, state)?;
    let mut contents: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut hashes = BTreeMap::new();
    for resource in &catalog.resources {
        let mut files = BTreeMap::new();
        if let Some(root) = sources.get(&resource.id) {
            for payload in resource.paths() {
                let payload = contained(root, &payload)?;
                if payload.is_dir() {
                    for (path, _) in payload_files(&payload)? {
                        if editable_text(&path) {
                            files.insert(
                                path.strip_prefix(root)?.to_string_lossy().into_owned(),
                                redact_text(&fs::read_to_string(path)?, None),
                            );
                        }
                    }
                }
            }
        }
        contents.insert(resource.id.clone(), files);
        if resource.source.is_self()
            && report
                .results
                .get(&resource.id)
                .is_some_and(|r| r.status == ApplyStatus::Applied)
        {
            hashes.insert(resource.id.clone(), content_hash(resource)?);
        }
    }
    state.set("catalog", &catalog)?;
    state.set("resource_contents", &contents)?;
    let owned = state
        .owned()?
        .into_iter()
        .filter(|w| hashes.contains_key(&w.resource_id))
        .map(|w| ActiveOwned {
            resource_id: w.resource_id,
            path: w.path,
        })
        .collect();
    atomic_write(
        &state.directory()?.join("active.json"),
        &serde_json::to_vec(&ActiveIndex { owned, hashes })?,
        None,
    )?;
    let collectors: BTreeMap<_, _> = clients
        .into_iter()
        .map(|client| {
            (
                client.to_string(),
                format!(
                    "已配置；以收到事件为准{}",
                    if client == Client::Codex {
                        "，Codex 需原生信任"
                    } else {
                        ""
                    }
                ),
            )
        })
        .collect();
    state.set("collector_status", &collectors)?;
    Ok((catalog, report))
}
#[derive(Serialize)]
struct Failure {
    error: String,
}
pub fn synchronize(runtime: &Runtime, state: &State) -> Result<ApplyReport> {
    let _guard = JobGuard::acquire(state.directory()?)?;
    let result = || -> Result<ApplyReport> {
        let temporary = tempfile::tempdir()?;
        let source = fetch_approved(&runtime.repo, temporary.path())?;
        let (_, report) = apply_source(&source, runtime, state)?;
        Ok(report)
    };
    match result() {
        Ok(report) => {
            state.add_run("sync", &report)?;
            Ok(report)
        }
        Err(error) => {
            state.add_run(
                "sync",
                &Failure {
                    error: redact_text(&format!("{error:#}"), Some(1600)),
                },
            )?;
            Err(error)
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub resource_id: String,
    pub issue: String,
    pub evidence: Vec<String>,
    pub validation: ValidationReport,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmittedProposal {
    pub key: String,
    pub url: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CycleDetails {
    pub application: BTreeMap<String, ResourceResult>,
    pub observations_processed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleReport {
    pub ok: bool,
    pub proposals: Vec<SubmittedProposal>,
    pub details: CycleDetails,
}
pub trait Submitter: Send + Sync {
    fn submit(&self, proposal: &mut Proposal, state: &State) -> Result<String>;
}
pub struct NativeSubmitter;
impl Submitter for NativeSubmitter {
    fn submit(&self, proposal: &mut Proposal, state: &State) -> Result<String> {
        gitlab::submit_mr(proposal, state)
    }
}
pub fn cycle(runtime: &Runtime, state: &State) -> Result<CycleReport> {
    cycle_with(runtime, state, None, &NativeSubmitter)
}
pub fn cycle_with(
    runtime: &Runtime,
    state: &State,
    runner: Option<&dyn AgentRunner>,
    submitter: &dyn Submitter,
) -> Result<CycleReport> {
    let _guard = JobGuard::acquire(state.directory()?)?;
    let mut result = CycleReport {
        ok: true,
        proposals: Vec::new(),
        details: CycleDetails::default(),
    };
    let work = |result: &mut CycleReport| -> Result<()> {
        ingest_events(state.directory()?, state)?;
        let temporary = tempfile::tempdir()?;
        let source = fetch_approved(&runtime.repo, &temporary.path().join("approved"))?;
        let (catalog, applied) = apply_source(&source, runtime, state)?;
        result.ok = applied.ok();
        result.details.application = applied.results;
        let observations = pending_observations(state, 1000)?;
        if !observations.is_empty() && catalog.resources.iter().any(|r| r.source.is_self()) {
            let fallback;
            let runner = match runner {
                Some(runner) => runner,
                None => {
                    fallback = default_runner()?;
                    &fallback
                }
            };
            let candidates = analyze_observations(&catalog, &observations, runner)?;
            let mut findings = Vec::new();
            for (index, candidate) in candidates.into_iter().enumerate() {
                let proposed = temporary.path().join(format!("candidate-{index}"));
                copy_tree(&source, &proposed)?;
                for (path, text) in &candidate.changes {
                    let path = contained(&proposed, Path::new(path))?;
                    fs::create_dir_all(path.parent().context("candidate target parent missing")?)?;
                    fs::write(path, text)?;
                }
                let validation = evaluate_candidate(&candidate, &source, &proposed)?;
                findings.push(Finding {
                    resource_id: candidate.resource_id.clone(),
                    issue: candidate.issue.clone(),
                    evidence: candidate.evidence.clone(),
                    validation: validation.clone(),
                });
                if validation.status == ValidationStatus::Failed {
                    continue;
                }
                let change = to_changeset(&candidate, &validation)?;
                let mut proposal =
                    match crate::proposals::create_proposal(&runtime.repo, &change, state) {
                        Ok(proposal) => proposal,
                        Err(error) if error.is::<NoChanges>() => continue,
                        Err(error) => return Err(error),
                    };
                let url = submitter.submit(&mut proposal, state)?;
                result.proposals.push(SubmittedProposal {
                    key: proposal.key,
                    url,
                });
            }
            state.set("findings", &findings)?;
        }
        complete_observations(
            state,
            &observations
                .iter()
                .map(|o| o.id.clone())
                .collect::<Vec<_>>(),
        )?;
        result.details.observations_processed = observations.len();
        Ok(())
    };
    if let Err(error) = work(&mut result) {
        result.ok = false;
        result.details.error = Some(redact_text(&format!("{error:#}"), Some(1600)));
    }
    state.add_run("cycle", &result)?;
    Ok(result)
}
