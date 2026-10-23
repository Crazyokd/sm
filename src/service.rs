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
        ApplyReport, ApplyStatus, Catalog, Client, ResourceResult, Source, ValidationReport,
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
    telemetry::RunTracker,
};
use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
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
            let _ = writeln!(
                std::io::stderr().lock(),
                "{}",
                serde_json::json!({"event":"job_unlock_failed","error":redact_text(&error.to_string(),Some(1200))})
            );
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
    let mut run = RunTracker::start(&state, "init")?;
    match initialize_tracked(runtime, systemd, &mut run) {
        Ok(report) => {
            if let Err(error) = run.finish(&report, report.ok, report.details.error.as_deref()) {
                return Err(match report.details.error {
                    Some(summary) => error.context(summary),
                    None => error,
                });
            }
            Ok(report)
        }
        Err(error) => {
            record_run_error(&mut run, &error);
            Err(error)
        }
    }
}
fn initialize_tracked(
    runtime: &Runtime,
    systemd: &dyn Systemd,
    run: &mut RunTracker,
) -> Result<InitReport> {
    run.stage("configure", None, 0, 1)?;
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
    run.stage("configure", None, 1, 1)?;
    run.stage("register_services", None, 0, 1)?;
    let register = || -> Result<()> {
        systemd.run(&["daemon-reload"])?;
        systemd.run(&["enable", "sm-web.service", "sm-cycle.timer"])?;
        systemd.run(&["start", "sm-cycle.timer"])?;
        systemd.run(&["restart", "sm-web.service"])
    };
    let ok = match register() {
        Ok(()) => {
            details.services = Some("registered".into());
            run.stage("register_services", None, 1, 1)?;
            true
        }
        Err(error) => {
            details.error = Some(redact_text(&format!("{error:#}"), Some(1200)));
            false
        }
    };
    let report = InitReport { ok, details };
    Ok(report)
}
pub fn apply_source(
    source: &Path,
    runtime: &Runtime,
    state: &State,
) -> Result<(Catalog, ApplyReport)> {
    apply_source_tracked(source, runtime, state, None)
}
fn apply_source_tracked(
    source: &Path,
    runtime: &Runtime,
    state: &State,
    mut run: Option<&mut RunTracker>,
) -> Result<(Catalog, ApplyReport)> {
    if let Some(run) = &mut run {
        run.stage("catalog", None, 0, 1)?;
    }
    let catalog = Catalog::load(source)?;
    // A reviewed existing resource stays governable while its network refresh is unavailable.
    let mut local_sources: BTreeMap<_, _> = catalog
        .resources
        .iter()
        .filter(|r| r.source.is_self())
        .map(|r| (r.id.clone(), source.to_path_buf()))
        .collect();
    for (name, dependency) in &catalog.dependencies {
        if dependency.source.is_self() {
            local_sources.insert(format!("@dependency/{name}"), source.to_path_buf());
        }
    }
    let clients: BTreeSet<_> = catalog
        .resources
        .iter()
        .filter(|r| r.source.is_self())
        .flat_map(|r| r.targets().into_keys())
        .collect();
    let mut baseline = plan_apply(&catalog, &local_sources, &runtime.home);
    for resource in catalog.resources.iter().filter(|resource| {
        !resource.source.is_self()
            || resource
                .dependencies
                .iter()
                .any(|name| !catalog.dependencies[name].source.is_self())
    }) {
        baseline.results.insert(
            resource.id.clone(),
            ResourceResult::new(ApplyStatus::Pending, "本轮上游刷新待完成；保留现有本地内容"),
        );
    }
    for client in &clients {
        baseline.writes.push(crate::providers::collector(
            *client,
            &runtime.home,
            &runtime.executable,
            state.directory()?,
        ));
    }
    // Delay unrelated retirements until the complete upstream plan is available.
    let ready: BTreeSet<_> = baseline
        .writes
        .iter()
        .map(|w| w.resource_id.clone())
        .chain(baseline.results.keys().cloned())
        .collect();
    for owned in state.owned()? {
        if !ready.contains(&owned.resource_id) {
            baseline.results.insert(
                owned.resource_id,
                ResourceResult::new(ApplyStatus::Pending, "完整同步完成后再清理旧投放"),
            );
        }
    }
    if let Some(run) = &mut run {
        run.stage("apply_existing", None, 0, catalog.resources.len())?;
    }
    crate::adoption::adopt(source, &catalog, &runtime.home, &baseline, state)?;
    let mut local_report = apply_writes(&baseline, state)?;
    local_report
        .results
        .extend(crate::aliases::synchronize(source, &runtime.home, state)?);
    state.set("resource_results", &local_report.results)?;
    cache_resources(
        &catalog,
        &local_sources,
        &local_report,
        &runtime.home,
        state,
    )?;
    state.set(
        "installation_manifest",
        &crate::installations::load(source)?,
    )?;
    crate::installations::register_existing(source, &runtime.home, state)?;
    state.set(
        "inventory",
        &crate::inventory::scan(&catalog, state, &runtime.home)?,
    )?;
    let temporary = tempfile::tempdir()?;
    let mut sources = BTreeMap::new();
    let mut errors = BTreeMap::new();
    let mut fetched: BTreeMap<String, std::result::Result<PathBuf, String>> = BTreeMap::new();
    let fetch_source =
        |origin: &Source, fetched: &mut BTreeMap<String, std::result::Result<PathBuf, String>>| {
            match origin {
                Source::SelfOwned => Ok(source.to_path_buf()),
                Source::Upstream { upstream } => fetched
                    .entry(upstream.clone())
                    .or_insert_with(|| {
                        fetch_upstream(
                            &catalog.upstreams[upstream],
                            source,
                            &temporary.path().join(upstream),
                        )
                        .map_err(|error| redact_text(&format!("{error:#}"), Some(1200)))
                    })
                    .clone(),
            }
        };
    let mut dependency_errors = BTreeMap::new();
    for (name, dependency) in &catalog.dependencies {
        match fetch_source(&dependency.source, &mut fetched) {
            Ok(path) => {
                sources.insert(format!("@dependency/{name}"), path);
            }
            Err(error) => {
                dependency_errors.insert(name.clone(), error);
            }
        }
    }
    for (index, resource) in catalog.resources.iter().enumerate() {
        if let Some(run) = &mut run {
            run.stage(
                "fetch_resource",
                Some(&resource.id),
                index,
                catalog.resources.len(),
            )?;
        }
        let result = fetch_source(&resource.source, &mut fetched);
        match result {
            Ok(path) => {
                if let Some(run) = &run {
                    run.resource_result(&resource.id, "fetched", "Source ready");
                }
                sources.insert(resource.id.clone(), path);
            }
            Err(error) => {
                if let Some(run) = &run {
                    run.resource_result(&resource.id, "failed", &format!("{error:#}"));
                }
                errors.insert(
                    resource.id.clone(),
                    ResourceResult::new(
                        ApplyStatus::Failed,
                        redact_text(&format!("{error:#}"), Some(1200)),
                    ),
                );
            }
        }
        for name in &resource.dependencies {
            if let Some(error) = dependency_errors.get(name) {
                errors.entry(resource.id.clone()).or_insert_with(|| {
                    ResourceResult::new(
                        ApplyStatus::Failed,
                        format!("supporting files {name}: {error}"),
                    )
                });
            }
        }
        if let Some(run) = &mut run {
            run.stage(
                "fetch_resource",
                Some(&resource.id),
                index + 1,
                catalog.resources.len(),
            )?;
        }
    }
    if let Some(run) = &mut run {
        run.stage("apply", None, 0, catalog.resources.len())?;
    }
    let mut plan = plan_apply(&catalog, &sources, &runtime.home);
    plan.results.extend(errors);
    for client in &clients {
        plan.writes.push(crate::providers::collector(
            *client,
            &runtime.home,
            &runtime.executable,
            state.directory()?,
        ));
    }
    if catalog
        .resources
        .iter()
        .filter(|resource| {
            !matches!(
                resource.content,
                crate::model::ResourceContent::Installation { .. }
            )
        })
        .any(|resource| {
            plan.results.get(&resource.id).is_some_and(|result| {
                matches!(
                    result.status,
                    ApplyStatus::Pending
                        | ApplyStatus::Failed
                        | ApplyStatus::Unsupported
                        | ApplyStatus::Conflict
                )
            })
        })
    {
        let current: BTreeSet<_> = catalog
            .resources
            .iter()
            .map(|resource| resource.id.as_str())
            .chain(plan.writes.iter().map(|write| write.resource_id.as_str()))
            .collect();
        // A renamed consumer may still need its entire old installation while support is unavailable.
        for owned in state.owned()? {
            if !current.contains(owned.resource_id.as_str()) {
                plan.results.insert(
                    owned.resource_id,
                    ResourceResult::new(ApplyStatus::Pending, "完整同步成功后再清理旧投放"),
                );
            }
        }
    }
    crate::adoption::adopt(source, &catalog, &runtime.home, &plan, state)?;
    let mut report = apply_writes(&plan, state)?;
    report
        .results
        .extend(crate::aliases::synchronize(source, &runtime.home, state)?);
    if let Some(run) = &mut run {
        run.stage("native_installations", None, 0, 1)?;
    }
    report.results.extend(crate::installations::synchronize(
        source,
        &runtime.home,
        state,
    )?);
    state.set("resource_results", &report.results)?;
    if let Some(run) = &mut run {
        for (index, (resource, result)) in report.results.iter().enumerate() {
            let status = serde_json::to_value(result.status)?;
            run.resource_result(
                resource,
                status.as_str().unwrap_or("unknown"),
                &result.message,
            );
            run.stage("apply", Some(resource), index + 1, report.results.len())?;
        }
        run.stage("cache_metadata", None, 0, 1)?;
    }
    cache_resources(&catalog, &sources, &report, &runtime.home, state)?;
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
    if let Some(run) = &mut run {
        run.stage("cache_metadata", None, 1, 1)?;
    }
    Ok((catalog, report))
}
fn cache_resources(
    catalog: &Catalog,
    sources: &BTreeMap<String, PathBuf>,
    report: &ApplyReport,
    home: &Path,
    state: &State,
) -> Result<()> {
    let mut previous_contents: BTreeMap<String, BTreeMap<String, String>> =
        state.get("resource_contents")?;
    let mut contents: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut hashes = BTreeMap::new();
    for resource in &catalog.resources {
        let mut files = BTreeMap::new();
        let readable = !report.results.get(&resource.id).is_some_and(|result| {
            matches!(
                result.status,
                ApplyStatus::Failed | ApplyStatus::Unsupported | ApplyStatus::Pending
            )
        });
        if let Some(root) = sources.get(&resource.id).filter(|_| readable) {
            for payload in resource.paths() {
                let payload = contained(root, &payload)?;
                let paths = if payload.is_file() {
                    vec![payload]
                } else if payload.is_dir() {
                    payload_files(&payload)?
                        .into_iter()
                        .map(|(path, _)| path)
                        .collect()
                } else {
                    Vec::new()
                };
                for path in paths {
                    if editable_text(&path) {
                        files.insert(
                            path.strip_prefix(root)?.to_string_lossy().into_owned(),
                            redact_text(&fs::read_to_string(path)?, None),
                        );
                    }
                }
            }
        } else if let Some(previous) = previous_contents.remove(&resource.id) {
            files = previous;
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
    state.set("catalog", catalog)?;
    state.set("inventory", &crate::inventory::scan(catalog, state, home)?)?;
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
    Ok(())
}
#[derive(Serialize)]
struct Failure {
    error: String,
}
fn record_run_error(run: &mut RunTracker, error: &anyhow::Error) {
    let summary = redact_text(&format!("{error:#}"), Some(1600));
    if let Err(error) = run.finish(
        &Failure {
            error: summary.clone(),
        },
        false,
        Some(&summary),
    ) {
        let _ = writeln!(
            std::io::stderr().lock(),
            "{}",
            serde_json::json!({"event":"run_persistence_failed","id":run.details().id,"error":redact_text(&format!("{error:#}"),Some(1600))})
        );
    }
}
fn application_error(results: &BTreeMap<String, ResourceResult>) -> Option<String> {
    let failures: Vec<_> = results
        .iter()
        .filter(|(_, result)| matches!(result.status, ApplyStatus::Failed | ApplyStatus::Conflict))
        .take(10)
        .map(|(resource, result)| format!("{resource}: {}", result.message))
        .collect();
    (!failures.is_empty()).then(|| redact_text(&failures.join("; "), Some(1600)))
}
pub fn synchronize(runtime: &Runtime, state: &State) -> Result<ApplyReport> {
    let _guard = JobGuard::acquire(state.directory()?)?;
    let mut run = RunTracker::start(state, "sync")?;
    let mut result = || -> Result<ApplyReport> {
        run.stage("fetch_approved", None, 0, 1)?;
        let temporary = tempfile::tempdir()?;
        let source = fetch_approved(&runtime.repo, temporary.path())?;
        let (_, report) = apply_source_tracked(&source, runtime, state, Some(&mut run))?;
        Ok(report)
    };
    match result() {
        Ok(report) => {
            let summary = application_error(&report.results);
            if let Err(error) = run.finish(&report, report.ok(), summary.as_deref()) {
                return Err(match summary {
                    Some(summary) => error.context(summary),
                    None => error,
                });
            }
            Ok(report)
        }
        Err(error) => {
            record_run_error(&mut run, &error);
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
    pub retention: Option<crate::retention::RetentionReport>,
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
    let mut run = RunTracker::start(state, "cycle")?;
    let mut result = CycleReport {
        ok: true,
        proposals: Vec::new(),
        details: CycleDetails::default(),
    };
    let mut work = |result: &mut CycleReport| -> Result<()> {
        run.stage("ingest", None, 0, 1)?;
        ingest_events(state.directory()?, state)?;
        run.stage("fetch_approved", None, 0, 1)?;
        let temporary = tempfile::tempdir()?;
        let source = fetch_approved(&runtime.repo, &temporary.path().join("approved"))?;
        let (catalog, applied) = apply_source_tracked(&source, runtime, state, Some(&mut run))?;
        result.ok = applied.ok();
        result.details.application = applied.results;
        let observations = pending_observations(state, 1000)?;
        if !observations.is_empty() && catalog.resources.iter().any(|r| r.source.is_self()) {
            run.stage("analysis", None, 0, observations.len())?;
            let fallback;
            let runner = match runner {
                Some(runner) => runner,
                None => {
                    fallback = default_runner()?;
                    &fallback
                }
            };
            let candidates = analyze_observations(&catalog, &observations, runner)?;
            run.stage("analysis", None, observations.len(), observations.len())?;
            let total = candidates.len();
            let mut findings = Vec::new();
            for (index, candidate) in candidates.into_iter().enumerate() {
                run.stage("validation", Some(&candidate.resource_id), index, total)?;
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
                let validation_status = serde_json::to_value(validation.status)?;
                run.resource_result(
                    &candidate.resource_id,
                    validation_status.as_str().unwrap_or("unknown"),
                    "Candidate validation finished",
                );
                run.stage("validation", Some(&candidate.resource_id), index + 1, total)?;
                if validation.status == ValidationStatus::Failed {
                    continue;
                }
                run.stage("proposal", Some(&candidate.resource_id), index, total)?;
                let change = to_changeset(&candidate, &validation)?;
                let mut proposal =
                    match crate::proposals::create_proposal(&runtime.repo, &change, state) {
                        Ok(proposal) => proposal,
                        Err(error) if error.is::<NoChanges>() => continue,
                        Err(error) => return Err(error),
                    };
                run.stage("submit_mr", Some(&candidate.resource_id), index, total)?;
                let url = submitter.submit(&mut proposal, state)?;
                run.resource_result(
                    &candidate.resource_id,
                    "submitted",
                    "Merge request submitted for review",
                );
                run.stage("submit_mr", Some(&candidate.resource_id), index + 1, total)?;
                result.proposals.push(SubmittedProposal {
                    key: proposal.key,
                    url,
                });
            }
            state.set("findings", &findings)?;
        }
        run.stage("complete_observations", None, 0, observations.len())?;
        complete_observations(
            state,
            &observations
                .iter()
                .map(|o| o.id.clone())
                .collect::<Vec<_>>(),
        )?;
        result.details.observations_processed = observations.len();
        run.stage(
            "complete_observations",
            None,
            observations.len(),
            observations.len(),
        )?;
        Ok(())
    };
    let failed_stage = if let Err(error) = work(&mut result) {
        result.ok = false;
        result.details.error = Some(redact_text(&format!("{error:#}"), Some(1600)));
        Some(run.details().clone())
    } else {
        None
    };
    let maintenance = (|| -> Result<_> {
        run.stage("retention", None, 0, 1)?;
        let report = crate::retention::maintain(state)?;
        run.stage("retention", None, 1, 1)?;
        Ok(report)
    })();
    match maintenance {
        Ok(report) => result.details.retention = Some(report),
        Err(error) => {
            let summary = redact_text(&format!("{error:#}"), Some(1600));
            run.resource_result("retention", "failed", &summary);
            result.ok = false;
            if result.details.error.is_none() {
                result.details.error = Some(summary);
            }
        }
    }
    if let Some(failed) = failed_stage {
        if let Err(error) = run.stage(
            &failed.stage,
            failed.resource.as_deref(),
            failed.completed,
            failed.total,
        ) {
            let _ = writeln!(
                std::io::stderr().lock(),
                "{}",
                serde_json::json!({"event":"run_persistence_failed","id":run.details().id,"error":redact_text(&format!("{error:#}"),Some(1600))})
            );
        }
    }
    let error = result
        .details
        .error
        .clone()
        .or_else(|| application_error(&result.details.application));
    if let Err(persistence) = run.finish(&result, result.ok, error.as_deref()) {
        if let Some(error) = error {
            return Err(persistence.context(error));
        }
        return Err(persistence);
    }
    Ok(result)
}
