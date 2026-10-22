mod common;
use anyhow::{Result, bail};
use clap::Parser;
use common::{FakeMrApi, Repo};
use serde_json::json;
use sm::{
    analysis::{AgentRunner, AnalysisResponse},
    cli::{Args, CommandOutcome},
    model::ApplyStatus,
    observe::{pending_observations, record_event},
    service::{JobGuard, Runtime, Submitter, Systemd, cycle_with, initialize_with, synchronize},
    state::State,
};
use std::{fs, path::PathBuf, process::Command, sync::Mutex};
#[derive(Default)]
struct FakeSystemd {
    calls: Mutex<Vec<Vec<String>>>,
    offline: bool,
}
impl Systemd for FakeSystemd {
    fn run(&self, args: &[&str]) -> Result<()> {
        self.calls
            .lock()
            .unwrap()
            .push(args.iter().map(|s| (*s).to_owned()).collect());
        if self.offline {
            bail!("no user bus");
        }
        Ok(())
    }
}
struct NoModel;
impl AgentRunner for NoModel {
    fn run(&self, _: &str) -> Result<AnalysisResponse> {
        bail!("model unavailable")
    }
}
struct FakeSubmitter(FakeMrApi);
impl Submitter for FakeSubmitter {
    fn submit(&self, proposal: &mut sm::proposals::Proposal, state: &State) -> Result<String> {
        sm::gitlab::submit_mr_with(proposal, state, &self.0)
    }
}
fn runtime(repo: &Repo) -> Runtime {
    Runtime {
        repo: repo.work.clone(),
        home: repo.temp.path().join("home"),
        executable: PathBuf::from(env!("CARGO_BIN_EXE_sm")),
    }
}

#[test]
fn local_apply_and_capture_are_ready_before_upstream_fetch_and_old_retirement_waits() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new();
    let runtime = runtime(&repo);
    let state = runtime.state().unwrap();
    assert!(synchronize(&runtime, &state).unwrap().ok());
    let path = repo.work.join("resources/new");
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("SKILL.md"), "New approved content").unwrap();
    let script = repo.temp.path().join("check-local");
    fs::write(&script,"#!/bin/sh\nset -eu\ntest -f \"$1\"\ntest -f \"$2\"\ngrep -q new \"$3\"\nmkdir -p \"$4\"\nprintf 'Upstream ready' > \"$4/SKILL.md\"\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    repo.catalog(json!({"projects":{},"resources":[{"id":"new","kind":"skill","origin":"self","scope":"user","project":null,"payload":"resources/new"},{"id":"external","kind":"skill","origin":"upstream","scope":"user","project":null,"payload":".","upstream":{"argv":[script,runtime.home.join(".agents/skills/new/SKILL.md"),runtime.home.join(".agents/skills/review/SKILL.md"),state.directory().unwrap().join("active.json")]}}]}));
    repo.publish("Replace local skill and fetch upstream");
    let report = synchronize(&runtime, &state).unwrap();
    assert!(report.ok(), "{report:?}");
    assert!(runtime.home.join(".agents/skills/new/SKILL.md").is_file());
    assert!(!runtime.home.join(".agents/skills/review/SKILL.md").exists());
    assert_eq!(
        fs::read_to_string(runtime.home.join(".agents/skills/external/SKILL.md")).unwrap(),
        "Upstream ready"
    );
}
#[test]
fn repeated_initialization_registers_only_same_native_units() {
    let repo = Repo::new();
    let runtime = runtime(&repo);
    let systemd = FakeSystemd::default();
    assert!(initialize_with(&runtime, &systemd).unwrap().ok);
    let path = runtime.home.join(".config/systemd/user/sm-web.service");
    let first = fs::read_to_string(&path).unwrap();
    assert!(first.contains(runtime.executable.to_str().unwrap()));
    assert!(!first.contains("-m sm"));
    assert!(initialize_with(&runtime, &systemd).unwrap().ok);
    assert_eq!(first, fs::read_to_string(path).unwrap());
    assert_eq!(
        fs::read_dir(runtime.home.join(".config/systemd/user"))
            .unwrap()
            .count(),
        3
    );
    assert!(!runtime.home.join(".codex/hooks.json").exists());
    assert!(
        fs::read_to_string(runtime.home.join(".config/systemd/user/sm-cycle.timer"))
            .unwrap()
            .contains("02:00:00")
    );
}
#[test]
fn missing_user_bus_is_a_recorded_initialization_failure() {
    let repo = Repo::new();
    let runtime = runtime(&repo);
    let report = initialize_with(
        &runtime,
        &FakeSystemd {
            offline: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!report.ok);
    assert!(report.details.error.unwrap().contains("no user bus"));
    let records: Vec<sm::state::RunRecord> = runtime.state().unwrap().get("runs").unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].result["run"]["status"], "failed");
    assert_eq!(records[0].result["run"]["stage"], "register_services");
}

#[test]
fn initialization_restart_can_start_synchronization_without_recovering_init_as_interrupted() {
    struct RestartSync<'a>(&'a Runtime);
    impl Systemd for RestartSync<'_> {
        fn run(&self, args: &[&str]) -> Result<()> {
            if args == ["restart", "sm-web.service"] {
                let state = self.0.state()?;
                assert!(
                    state
                        .get::<Option<serde_json::Value>>("active_run")?
                        .is_none()
                );
                assert!(synchronize(self.0, &state)?.ok());
            }
            Ok(())
        }
    }
    let repo = Repo::new();
    let runtime = runtime(&repo);
    assert!(
        initialize_with(&runtime, &RestartSync(&runtime))
            .unwrap()
            .ok
    );
    let records: Vec<sm::state::RunRecord> = runtime.state().unwrap().get("runs").unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].operation, "sync");
    assert_eq!(records[1].operation, "init");
    assert!(
        records
            .iter()
            .all(|record| record.result["run"]["status"] == "succeeded")
    );
}
#[test]
fn synchronization_projects_only_approved_content_and_replaces_collector_once() {
    let repo = Repo::new();
    let mut runtime = runtime(&repo);
    let state = runtime.state().unwrap();
    assert!(synchronize(&runtime, &state).unwrap().ok());
    let records: Vec<sm::state::RunRecord> = state.get("runs").unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].result["results"]["review"]["status"], "applied");
    assert_eq!(records[0].result["run"]["status"], "succeeded");
    assert_eq!(records[0].result["run"]["operation"], "sync");
    assert!(records[0].result["run"]["id"].as_str().is_some());
    assert!(
        state
            .get::<Option<serde_json::Value>>("active_run")
            .unwrap()
            .is_none()
    );
    let first = fs::read(runtime.home.join(".codex/hooks.json")).unwrap();
    assert!(synchronize(&runtime, &state).unwrap().ok());
    assert_eq!(
        first,
        fs::read(runtime.home.join(".codex/hooks.json")).unwrap()
    );
    assert_eq!(
        fs::read_to_string(runtime.home.join(".agents/skills/review/SKILL.md")).unwrap(),
        "Approved"
    );
    runtime.executable = repo.temp.path().join("native-sm-updated");
    assert!(synchronize(&runtime, &state).unwrap().ok());
    let current: serde_json::Value =
        serde_json::from_slice(&fs::read(runtime.home.join(".codex/hooks.json")).unwrap()).unwrap();
    assert_eq!(current["hooks"]["Stop"].as_array().unwrap().len(), 1);
    assert!(current.to_string().contains("native-sm-updated"));
}
#[test]
fn failed_upstream_refresh_preserves_files_and_cached_content_until_retirement() {
    let repo = Repo::new();
    let mut catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.work.join("catalog.json")).unwrap()).unwrap();
    catalog["resources"].as_array_mut().unwrap().push(json!({
        "id": "upstream", "kind": "skill", "origin": "upstream", "scope": "user",
        "project": null, "payload": "resources/review",
        "upstream": {"argv": ["cp", "-R", "resources"]}
    }));
    repo.catalog(catalog.clone());
    repo.publish("Add upstream generator");
    let runtime = runtime(&repo);
    let state = runtime.state().unwrap();
    assert!(synchronize(&runtime, &state).unwrap().ok());
    let target = runtime.home.join(".agents/skills/upstream/SKILL.md");
    assert_eq!(fs::read_to_string(&target).unwrap(), "Approved");
    let contents: serde_json::Value = state.get("resource_contents").unwrap();
    assert_eq!(
        contents["upstream"]["resources/review/SKILL.md"],
        "Approved"
    );

    catalog["resources"][1]["upstream"]["argv"] = json!(["false"]);
    repo.catalog(catalog.clone());
    fs::write(
        repo.work.join("resources/review/SKILL.md"),
        "New approved content",
    )
    .unwrap();
    repo.publish("Fail upstream generator while updating self-owned content");
    let report = synchronize(&runtime, &state).unwrap();
    assert!(!report.ok());
    assert_eq!(report.results["upstream"].status, ApplyStatus::Failed);
    assert_eq!(report.results["review"].status, ApplyStatus::Applied);
    assert_eq!(fs::read_to_string(&target).unwrap(), "Approved");
    let contents: serde_json::Value = state.get("resource_contents").unwrap();
    assert_eq!(
        contents["upstream"]["resources/review/SKILL.md"],
        "Approved"
    );
    assert_eq!(
        contents["review"]["resources/review/SKILL.md"],
        "New approved content"
    );

    catalog["resources"].as_array_mut().unwrap().pop();
    repo.catalog(catalog);
    repo.publish("Retire upstream resource");
    assert!(synchronize(&runtime, &state).unwrap().ok());
    assert!(!target.exists());
    let contents: serde_json::Value = state.get("resource_contents").unwrap();
    assert!(contents.get("upstream").is_none());
}
#[test]
fn job_lock_rejects_overlap_and_releases_on_drop() {
    let root = tempfile::tempdir().unwrap();
    let first = JobGuard::acquire(root.path()).unwrap();
    assert!(JobGuard::acquire(root.path()).is_err());
    drop(first);
    assert!(JobGuard::acquire(root.path()).is_ok());
}
#[test]
fn one_invalid_native_payload_does_not_hide_other_resources() {
    let repo = Repo::new();
    fs::create_dir(repo.work.join("resources/bad")).unwrap();
    fs::write(
        repo.work.join("resources/bad/note.txt"),
        "Missing Skill header",
    )
    .unwrap();
    let mut catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.work.join("catalog.json")).unwrap()).unwrap();
    let mut bad = catalog["resources"][0].clone();
    bad["id"] = json!("bad");
    bad["payload"] = json!("resources/bad");
    catalog["resources"].as_array_mut().unwrap().push(bad);
    repo.catalog(catalog);
    repo.publish("Bad payload");
    let runtime = runtime(&repo);
    let report = synchronize(&runtime, &runtime.state().unwrap()).unwrap();
    assert_eq!(report.results["review"].status, ApplyStatus::Applied);
    assert_eq!(report.results["bad"].status, ApplyStatus::Failed);
}
#[test]
fn empty_cycle_does_not_call_model_or_publish_proposals() {
    let repo = Repo::new();
    let runtime = runtime(&repo);
    let report = cycle_with(
        &runtime,
        &runtime.state().unwrap(),
        Some(&NoModel),
        &FakeSubmitter(FakeMrApi::default()),
    )
    .unwrap();
    assert!(report.ok);
    assert!(report.proposals.is_empty());
}
#[test]
fn model_failure_keeps_observations_pending_and_records_cycle_failure() {
    let repo = Repo::new();
    let runtime = runtime(&repo);
    let state = runtime.state().unwrap();
    synchronize(&runtime, &state).unwrap();
    record_event(
        sm::model::Client::Codex,
        &json!({"resource_id":"review","hook_event_name":"Stop"}),
        state.directory().unwrap(),
    )
    .unwrap();
    let report = cycle_with(
        &runtime,
        &state,
        Some(&NoModel),
        &FakeSubmitter(FakeMrApi::default()),
    )
    .unwrap();
    assert!(!report.ok);
    assert!(report.details.retention.is_some());
    assert!(report.details.error.unwrap().contains("model unavailable"));
    assert_eq!(pending_observations(&state, 100).unwrap().len(), 1);
    let records: Vec<sm::state::RunRecord> = state.get("runs").unwrap();
    let run = &records.last().unwrap().result["run"];
    assert_eq!(run["status"], "failed");
    assert_eq!(run["stage"], "analysis");
    assert!(run["error"].as_str().unwrap().contains("model unavailable"));
}

#[test]
fn synchronization_fetch_error_is_recorded_with_its_failed_stage() {
    let repo = Repo::new();
    let mut runtime = runtime(&repo);
    runtime.repo = repo.temp.path().join("missing-repository");
    let state = runtime.state().unwrap();
    let error = synchronize(&runtime, &state).unwrap_err();
    assert!(error.to_string().contains("git"));
    let records: Vec<sm::state::RunRecord> = state.get("runs").unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].result["run"]["status"], "failed");
    assert_eq!(records[0].result["run"]["stage"], "fetch_approved");
    assert!(records[0].result["error"].as_str().unwrap().contains("git"));
}

#[test]
fn original_sync_error_survives_failed_history_persistence() {
    let repo = Repo::new();
    let mut runtime = runtime(&repo);
    runtime.repo = repo.temp.path().join("missing-repository");
    let state = runtime.state().unwrap();
    state.set("runs", &json!({"not":"run history"})).unwrap();
    let error = synchronize(&runtime, &state).unwrap_err();
    assert!(error.to_string().contains("git"), "{error:#}");
    let active = state
        .get::<Option<serde_json::Value>>("active_run")
        .unwrap()
        .unwrap();
    assert_eq!(active["status"], "failed");
    assert_eq!(active["stage"], "fetch_approved");
}

#[test]
fn trigger_is_whitelisted_and_journal_events_are_json_summaries() {
    let repo = Repo::new();
    let runtime = runtime(&repo);
    for (trigger, expected) in [
        ("startup", "startup"),
        ("nightly", "nightly"),
        ("private-secret", "manual"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_sm"))
            .args(["--repo", runtime.repo.to_str().unwrap(), "sync"])
            .env("HOME", &runtime.home)
            .env("SM_TRIGGER", trigger)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let records: Vec<sm::state::RunRecord> = runtime.state().unwrap().get("runs").unwrap();
        assert_eq!(records.last().unwrap().result["run"]["trigger"], expected);
        let events: Vec<serde_json::Value> = String::from_utf8(output.stderr)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(events.iter().any(|event| event["event"] == "run_started"));
        assert!(
            events
                .iter()
                .any(|event| event["event"] == "resource_result")
        );
        assert!(events.iter().any(|event| event["event"] == "run_finished"));
        assert!(
            !serde_json::to_string(&events)
                .unwrap()
                .contains("private-secret")
        );
        assert!(!serde_json::to_string(&events).unwrap().contains("Approved"));
    }
}
#[test]
fn cli_routes_all_original_commands_with_typed_arguments() {
    for argv in [
        vec!["sm", "init"],
        vec!["sm", "serve", "--port", "8765"],
        vec!["sm", "sync"],
        vec!["sm", "cycle"],
        vec!["sm", "status"],
        vec!["sm", "propose", "change.json"],
        vec!["sm", "record", "--agent", "codex", "--state-dir", "/tmp/sm"],
    ] {
        assert!(Args::try_parse_from(argv).is_ok());
    }
    assert!(
        Args::try_parse_from([
            "sm",
            "record",
            "--agent",
            "unknown",
            "--state-dir",
            "/tmp/sm"
        ])
        .is_err()
    );
    let repo = Repo::new();
    let runtime = runtime(&repo);
    assert!(matches!(
        sm::cli::run(Args::try_parse_from(["sm", "sync"]).unwrap(), &runtime).unwrap(),
        CommandOutcome::Exit(0)
    ));
    assert!(runtime.home.join(".agents/skills/review/SKILL.md").exists());
    assert!(matches!(
        sm::cli::run(
            Args::try_parse_from(["sm", "serve", "--port", "8766"]).unwrap(),
            &runtime
        )
        .unwrap(),
        CommandOutcome::Serve(8766)
    ));
}
#[test]
fn native_service_units_pass_systemd_syntax_validation() {
    let repo = Repo::new();
    for home in ["home", "home with spaces"] {
        let mut runtime = runtime(&repo);
        runtime.home = repo.temp.path().join(home);
        initialize_with(&runtime, &FakeSystemd::default()).unwrap();
        let directory = runtime.home.join(".config/systemd/user");
        let output = Command::new("systemd-analyze")
            .arg("verify")
            .args([
                directory.join("sm-web.service"),
                directory.join("sm-cycle.service"),
                directory.join("sm-cycle.timer"),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let warnings = String::from_utf8_lossy(&output.stderr);
        assert!(!warnings.contains("EnvironmentFile="), "{warnings}");
    }
}
