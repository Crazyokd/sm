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
    repo.catalog(json!({"projects":{},"upstreams":{"external":{"argv":[script,runtime.home.join(".agents/skills/new/SKILL.md"),runtime.home.join(".agents/skills/review/SKILL.md"),state.directory().unwrap().join("active.json")]}},"resources":[{"id":"new","kind":"skill","origin":"self","scope":"user","project":null,"payload":"resources/new"},{"id":"external","kind":"skill","origin":"upstream","scope":"user","project":null,"payload":".","upstream":"external"}]}));
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
    catalog["upstreams"] = json!({"upstream":{"argv":["cp","-R","resources"]}});
    catalog["resources"].as_array_mut().unwrap().push(json!({
        "id": "upstream", "kind": "skill", "origin": "upstream", "scope": "user",
        "project": null, "payload": "resources/review",
        "upstream": "upstream"
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

    catalog["upstreams"]["upstream"]["argv"] = json!(["false"]);
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
    let details = serde_json::to_value(&report.details).unwrap();
    assert_eq!(details["sync_ok"], true);
    assert_eq!(details["analysis_ok"], false);
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
fn nightly_analysis_drains_the_backlog_and_checkpoints_completed_selections() {
    struct Runner {
        calls: std::sync::atomic::AtomicUsize,
        fail_second: bool,
    }
    impl AgentRunner for Runner {
        fn run(&self, _: &str) -> Result<AnalysisResponse> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.fail_second && call == 1 {
                bail!("next selection unavailable");
            }
            Ok(AnalysisResponse {
                candidates: Vec::new(),
            })
        }
    }
    let repo = Repo::new();
    let runtime = runtime(&repo);
    let state = runtime.state().unwrap();
    let mut db = state.connect().unwrap();
    let transaction = db.transaction().unwrap();
    for index in 0..2005 {
        let row = json!({"id":format!("event-{index}"),"agent":"codex","session_id":"session","cwd":"/repo","event":"Stop","timestamp":"2026-11-06T00:00:00Z","resource_id":"review","resource_content_hash":null,"evidence_kind":"unknown","signal":"completion","detected_from":"explicit","confidence":"low","turn_id":null,"invocation_id":null,"parent_invocation_id":null,"facts":{},"coverage":"observed"});
        transaction
            .execute(
                "INSERT INTO observations(id,value) VALUES (?1,?2)",
                [row["id"].as_str().unwrap(), &row.to_string()],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
    let failing = Runner {
        calls: Default::default(),
        fail_second: true,
    };
    let report = cycle_with(
        &runtime,
        &state,
        Some(&failing),
        &FakeSubmitter(FakeMrApi::default()),
    )
    .unwrap();
    assert!(!report.ok);
    assert_eq!(report.details.observations_processed, 1000);
    assert_eq!(pending_observations(&state, 3000).unwrap().len(), 1005);
    assert_eq!(pending_observations(&state, 1).unwrap()[0].id, "event-1000");
    let retry = Runner {
        calls: Default::default(),
        fail_second: false,
    };
    let report = cycle_with(
        &runtime,
        &state,
        Some(&retry),
        &FakeSubmitter(FakeMrApi::default()),
    )
    .unwrap();
    assert!(report.ok, "{report:?}");
    assert_eq!(report.details.observations_processed, 1005);
    assert!(pending_observations(&state, 1).unwrap().is_empty());
    assert!(report.proposals.is_empty());
}

#[test]
fn synchronization_conflicts_do_not_hide_successful_analysis() {
    struct Empty;
    impl AgentRunner for Empty {
        fn run(&self, _: &str) -> Result<AnalysisResponse> {
            Ok(AnalysisResponse {
                candidates: Vec::new(),
            })
        }
    }
    let repo = Repo::new();
    let runtime = runtime(&repo);
    let state = runtime.state().unwrap();
    synchronize(&runtime, &state).unwrap();
    fs::write(
        runtime.home.join(".agents/skills/review/SKILL.md"),
        "Local edit",
    )
    .unwrap();
    record_event(
        sm::model::Client::Codex,
        &json!({"resource_id":"review","hook_event_name":"Stop"}),
        state.directory().unwrap(),
    )
    .unwrap();
    let report = cycle_with(
        &runtime,
        &state,
        Some(&Empty),
        &FakeSubmitter(FakeMrApi::default()),
    )
    .unwrap();
    assert!(!report.ok);
    assert_eq!(report.details.observations_processed, 1);
    let details = serde_json::to_value(&report.details).unwrap();
    assert_eq!(details["sync_ok"], false);
    assert_eq!(details["analysis_ok"], true);
    assert_eq!(
        fs::read_to_string(runtime.home.join(".agents/skills/review/SKILL.md")).unwrap(),
        "Local edit"
    );
}

#[test]
fn later_analysis_batch_failure_keeps_the_selection_pending_for_a_complete_retry() {
    struct BatchRunner {
        fail_second: bool,
        batches: Mutex<Vec<Vec<String>>>,
    }
    impl AgentRunner for BatchRunner {
        fn run(&self, prompt: &str) -> Result<AnalysisResponse> {
            assert!(prompt.len() <= 512_000);
            let input: serde_json::Value = serde_json::from_str(prompt.lines().last().unwrap())?;
            let mut batches = self.batches.lock().unwrap();
            batches.push(
                input["observations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|row| row["id"].as_str().unwrap().to_owned())
                    .collect(),
            );
            if self.fail_second && batches.len() == 2 {
                bail!("second analysis batch unavailable");
            }
            Ok(AnalysisResponse {
                candidates: Vec::new(),
            })
        }
    }
    let repo = Repo::new();
    let runtime = runtime(&repo);
    let state = runtime.state().unwrap();
    let db = state.connect().unwrap();
    let ids: Vec<_> = (0..8).map(|index| format!("batch-event-{index}")).collect();
    for id in &ids {
        let row = json!({"id":id,"agent":"codex","session_id":"session","cwd":"/repo","event":"PostToolUse","timestamp":"2026-10-26T00:00:00Z","resource_id":"review","resource_content_hash":null,"evidence_kind":"tool_execution","signal":"tool_execution","detected_from":"explicit","confidence":"medium","turn_id":null,"invocation_id":null,"parent_invocation_id":null,"facts":{"text":"x".repeat(180_000)},"coverage":"observed"});
        db.execute(
            "INSERT INTO observations(id,value,processed) VALUES (?1,?2,0)",
            [id, &row.to_string()],
        )
        .unwrap();
    }
    let failing = BatchRunner {
        fail_second: true,
        batches: Mutex::new(Vec::new()),
    };
    let report = cycle_with(
        &runtime,
        &state,
        Some(&failing),
        &FakeSubmitter(FakeMrApi::default()),
    )
    .unwrap();
    assert!(!report.ok);
    assert!(
        report
            .details
            .error
            .unwrap()
            .contains("second analysis batch unavailable")
    );
    assert_eq!(report.details.observations_processed, 0);
    assert_eq!(pending_observations(&state, 1000).unwrap().len(), ids.len());

    let retry = BatchRunner {
        fail_second: false,
        batches: Mutex::new(Vec::new()),
    };
    let report = cycle_with(
        &runtime,
        &state,
        Some(&retry),
        &FakeSubmitter(FakeMrApi::default()),
    )
    .unwrap();
    assert!(report.ok, "{report:?}");
    assert_eq!(report.details.observations_processed, ids.len());
    assert!(pending_observations(&state, 1000).unwrap().is_empty());
    assert_eq!(
        retry
            .batches
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>(),
        ids
    );
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

#[test]
fn shared_upstream_is_fetched_once_and_missing_payload_fails_only_its_resource() {
    let repo = Repo::new();
    let counter = repo.temp.path().join("fetches");
    fs::write(repo.work.join("resources/generate.sh"), "set -eu\nprintf 'fetch\\n' >> \"$1\"\nmkdir -p \"$2/.agents/skills/one\" \"$2/.agents/skills/two\"\nprintf 'One' > \"$2/.agents/skills/one/SKILL.md\"\nprintf 'Two' > \"$2/.agents/skills/two/SKILL.md\"\nprintf 'private unrelated' > \"$2/unselected.txt\"\n").unwrap();
    let mut catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.work.join("catalog.json")).unwrap()).unwrap();
    catalog["upstreams"] = json!({"shared":{"argv":["sh","resources/generate.sh",counter]}});
    for id in ["one", "two", "missing"] {
        catalog["resources"].as_array_mut().unwrap().push(json!({
            "id":id,"kind":"skill","origin":"upstream","upstream":"shared","scope":"user","project":null,
            "files":{"root":".","entries":[{"target":format!(".agents/skills/{id}/SKILL.md"),"source":format!(".agents/skills/{id}/SKILL.md")}]}
        }));
    }
    repo.catalog(catalog.clone());
    repo.publish("Independent resources from one shared upstream");
    let runtime = runtime(&repo);
    let state = runtime.state().unwrap();
    let report = synchronize(&runtime, &state).unwrap();
    assert_eq!(fs::read_to_string(&counter).unwrap(), "fetch\n");
    assert_eq!(report.results["review"].status, ApplyStatus::Applied);
    assert_eq!(report.results["one"].status, ApplyStatus::Applied);
    assert_eq!(report.results["two"].status, ApplyStatus::Applied);
    assert_eq!(report.results["missing"].status, ApplyStatus::Failed);
    let contents: serde_json::Value = state.get("resource_contents").unwrap();
    assert_eq!(contents["one"][".agents/skills/one/SKILL.md"], "One");
    assert_eq!(contents["one"].as_object().unwrap().len(), 1);
    assert_eq!(contents["two"].as_object().unwrap().len(), 1);
    assert!(contents["missing"].as_object().unwrap().is_empty());
    assert!(!contents.to_string().contains("private unrelated"));

    catalog["upstreams"]["shared"]["argv"] = json!([
        "sh",
        "-c",
        "printf 'failed\\n' >> \"$1\"; exit 1",
        "source",
        counter
    ]);
    repo.catalog(catalog);
    repo.publish("Failed shared upstream remains one attempt");
    let report = synchronize(&runtime, &state).unwrap();
    assert_eq!(fs::read_to_string(counter).unwrap(), "fetch\nfailed\n");
    for id in ["one", "two", "missing"] {
        assert_eq!(report.results[id].status, ApplyStatus::Failed);
    }
    assert_eq!(report.results["review"].status, ApplyStatus::Applied);
    assert_eq!(
        fs::read_to_string(runtime.home.join(".agents/skills/one/SKILL.md")).unwrap(),
        "One"
    );
    let retained: serde_json::Value = state.get("resource_contents").unwrap();
    assert_eq!(retained["one"], contents["one"]);
}

#[test]
fn upstream_skill_tree_follows_added_removed_helpers_and_isolates_unsafe_members() {
    let repo = Repo::new();
    let runtime = runtime(&repo);
    let state = runtime.state().unwrap();
    let mut catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.work.join("catalog.json")).unwrap()).unwrap();
    catalog["upstreams"] = json!({"shared":{"argv":["sh","resources/skill-tree.sh"]}});
    catalog["resources"].as_array_mut().unwrap().push(json!({
        "id":"selected","kind":"skill","origin":"upstream","upstream":"shared","scope":"user",
        "files":{"root":".","entries":[{"target":".agents/skills/selected","tree":".agents/skills/selected"}]}
    }));
    repo.catalog(catalog);
    for (revision, helpers) in [
        ("initial", vec!["old.sh"]),
        ("added", vec!["old.sh", "new.sh"]),
        ("removed", vec!["new.sh"]),
    ] {
        let mut generator = String::from(
            "set -eu\nmkdir -p \"$1/.agents/skills/selected\" \"$1/.agents/skills/other\"\nprintf 'Selected Skill' > \"$1/.agents/skills/selected/SKILL.md\"\nprintf 'Other Skill' > \"$1/.agents/skills/other/SKILL.md\"\n",
        );
        for helper in &helpers {
            generator.push_str(&format!("printf 'helper' > \"$1/.agents/skills/selected/{helper}\"\nchmod +x \"$1/.agents/skills/selected/{helper}\"\n"));
        }
        fs::write(repo.work.join("resources/skill-tree.sh"), generator).unwrap();
        repo.publish(revision);
        let report = synchronize(&runtime, &state).unwrap();
        assert!(report.ok(), "{report:?}");
        let installed = runtime.home.join(".agents/skills/selected");
        assert_eq!(
            installed.join("old.sh").exists(),
            helpers.contains(&"old.sh")
        );
        assert_eq!(
            installed.join("new.sh").exists(),
            helpers.contains(&"new.sh")
        );
        assert!(!runtime.home.join(".agents/skills/other").exists());
        let contents: serde_json::Value = state.get("resource_contents").unwrap();
        assert_eq!(
            contents["selected"].as_object().unwrap().len(),
            helpers.len() + 1
        );
        assert!(contents["selected"][".agents/skills/selected/SKILL.md"].is_string());
    }
    let previous: serde_json::Value = state.get("resource_contents").unwrap();
    fs::write(repo.work.join("resources/skill-tree.sh"),"set -eu\nmkdir -p \"$1/.agents/skills/selected\"\nprintf 'Changed Skill' > \"$1/.agents/skills/selected/SKILL.md\"\nln -s /etc/passwd \"$1/.agents/skills/selected/unsafe\"\n").unwrap();
    repo.publish("unsafe selected tree");
    let report = synchronize(&runtime, &state).unwrap();
    assert_eq!(report.results["selected"].status, ApplyStatus::Failed);
    assert_eq!(report.results["review"].status, ApplyStatus::Applied);
    assert_eq!(
        fs::read_to_string(runtime.home.join(".agents/skills/selected/SKILL.md")).unwrap(),
        "Selected Skill"
    );
    assert!(runtime.home.join(".agents/skills/selected/new.sh").exists());
    let contents: serde_json::Value = state.get("resource_contents").unwrap();
    assert_eq!(contents["selected"], previous["selected"]);
}

#[test]
fn complete_consumers_wait_for_shared_upstream_support_and_preserve_it_on_failure() {
    let repo = Repo::new();
    let runtime = runtime(&repo);
    let state = runtime.state().unwrap();
    let counter = repo.temp.path().join("fetch-count");
    let generator = repo.work.join("resources/support.sh");
    fs::write(&generator,"set -eu\nprintf 'fetch\\n' >> \"$1\"\nif [ \"$2\" != initial ]; then grep -q old-a \"$3\"; ! grep -q next-a \"$3\"; fi\nmkdir -p \"$4/.claude/scripts\" \"$4/.agents/skills/external\"\nprintf 'runtime' > \"$4/.claude/scripts/runtime.js\"\nprintf 'Upstream Skill' > \"$4/.agents/skills/external/SKILL.md\"\n").unwrap();
    let mut catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.work.join("catalog.json")).unwrap()).unwrap();
    catalog["upstreams"] = json!({"shared":{"argv":["sh","resources/support.sh",counter,"initial",runtime.home.join(".codex/hooks.json")]}});
    catalog["dependencies"] = json!({"runtime":{"origin":"upstream","upstream":"shared","files":{"root":".","entries":[{"target":".claude/scripts/runtime.js","source":".claude/scripts/runtime.js"}]}}});
    for id in ["a", "b"] {
        catalog["resources"].as_array_mut().unwrap().push(json!({
            "id":id,"kind":"hook","origin":"self","scope":"user","dependencies":["runtime"],
            "files":{"root":format!("resources/{id}"),"entries":[{"target":".codex/hooks.json","json":{"hooks":{"Stop":[{"hooks":[{"type":"command","command":format!("old-{id}")}]}]}}}]}
        }));
    }
    catalog["resources"].as_array_mut().unwrap().push(json!({"id":"external","kind":"skill","origin":"upstream","upstream":"shared","scope":"user","files":{"root":".","entries":[{"target":".agents/skills/external","tree":".agents/skills/external"}]}}));
    repo.catalog(catalog.clone());
    repo.publish("Complete hooks with referenced upstream support");
    let report = synchronize(&runtime, &state).unwrap();
    assert!(report.ok(), "{report:?}");
    assert_eq!(fs::read_to_string(&counter).unwrap(), "fetch\n");
    let shared = runtime.home.join(".claude/scripts/runtime.js");
    assert_eq!(fs::read(&shared).unwrap(), b"runtime");
    assert_eq!(
        state
            .owned()
            .unwrap()
            .iter()
            .filter(|write| write.path == shared)
            .count(),
        2
    );
    for id in ["a", "b"] {
        assert!(
            catalog["resources"]
                .as_array()
                .unwrap()
                .iter()
                .find(|resource| resource["id"] == id)
                .unwrap()["origin"]
                == "self"
        );
    }
    let mut resources = catalog["resources"].as_array_mut().unwrap().iter_mut();
    for resource in resources
        .by_ref()
        .filter(|resource| resource["id"] == "a" || resource["id"] == "b")
    {
        let id = resource["id"].as_str().unwrap().to_owned();
        resource["files"]["entries"][0]["json"]["hooks"]["Stop"][0]["hooks"][0]["command"] =
            json!(format!("next-{id}"));
    }
    catalog["upstreams"]["shared"]["argv"][3] = json!("updated");
    repo.catalog(catalog.clone());
    repo.publish("Update authored hook while dependency fetch verifies old body");
    let report = synchronize(&runtime, &state).unwrap();
    assert!(report.ok(), "{report:?}");
    assert_eq!(fs::read_to_string(&counter).unwrap(), "fetch\nfetch\n");
    let config = fs::read(runtime.home.join(".codex/hooks.json")).unwrap();
    assert!(
        String::from_utf8(config.clone())
            .unwrap()
            .contains("next-a")
    );
    let contents: serde_json::Value = state.get("resource_contents").unwrap();
    assert!(!contents["a"].to_string().contains("runtime.js"));
    catalog["upstreams"]["shared"]["argv"] = json!(["false"]);
    for resource in catalog["resources"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .filter(|resource| resource["id"] == "a" || resource["id"] == "b")
    {
        resource["files"]["entries"][0]["json"]["hooks"]["Stop"][0]["hooks"][0]["command"] =
            json!("unavailable");
    }
    fs::write(
        repo.work.join("resources/review/SKILL.md"),
        "Independent update",
    )
    .unwrap();
    repo.catalog(catalog);
    repo.publish("Dependency source fails while independent resource updates");
    let report = synchronize(&runtime, &state).unwrap();
    assert_eq!(report.results["a"].status, ApplyStatus::Failed);
    assert_eq!(report.results["b"].status, ApplyStatus::Failed);
    assert_eq!(report.results["review"].status, ApplyStatus::Applied);
    assert_eq!(fs::read(&shared).unwrap(), b"runtime");
    assert_eq!(
        fs::read(runtime.home.join(".codex/hooks.json")).unwrap(),
        config
    );
    assert_eq!(
        fs::read_to_string(runtime.home.join(".agents/skills/review/SKILL.md")).unwrap(),
        "Independent update"
    );
}

#[test]
fn missing_support_payload_blocks_the_complete_consumer_and_keeps_previous_files() {
    let repo = Repo::new();
    let runtime = runtime(&repo);
    let state = runtime.state().unwrap();
    let support = repo.work.join("resources/dependencies/runtime/files");
    fs::create_dir_all(&support).unwrap();
    fs::write(support.join("runtime.js"), "runtime").unwrap();
    let mut catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.work.join("catalog.json")).unwrap()).unwrap();
    catalog["dependencies"] = json!({"runtime":{"origin":"self","files":{"root":"resources/dependencies/runtime/files","entries":[{"target":".claude/scripts/runtime.js","source":"runtime.js"}]}}});
    catalog["resources"].as_array_mut().unwrap().push(json!({"id":"hook","kind":"hook","origin":"self","scope":"user","dependencies":["runtime"],"files":{"root":"resources/hook","entries":[{"target":".codex/hooks.json","json":{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"before"}]}]}}}]}}));
    repo.catalog(catalog.clone());
    repo.publish("Self complete Hook");
    assert!(synchronize(&runtime, &state).unwrap().ok());
    let config = fs::read(runtime.home.join(".codex/hooks.json")).unwrap();
    fs::remove_file(support.join("runtime.js")).unwrap();
    catalog["resources"][1]["files"]["entries"][0]["json"]["hooks"]["Stop"][0]["hooks"][0]["command"] =
        json!("after");
    repo.catalog(catalog);
    repo.publish("Missing dependency file");
    let report = synchronize(&runtime, &state).unwrap();
    assert_eq!(report.results["hook"].status, ApplyStatus::Failed);
    assert_eq!(
        fs::read(runtime.home.join(".codex/hooks.json")).unwrap(),
        config
    );
    assert_eq!(
        fs::read(runtime.home.join(".claude/scripts/runtime.js")).unwrap(),
        b"runtime"
    );
}

#[test]
fn renamed_complete_hook_keeps_old_installation_until_dependency_retry_succeeds() {
    let repo = Repo::new();
    let runtime = runtime(&repo);
    let state = runtime.state().unwrap();
    let payload = repo.work.join("resources/old-hook");
    fs::create_dir_all(payload.join("files")).unwrap();
    fs::write(
        payload.join("config.json"),
        json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":"old-command"}]}]}})
            .to_string(),
    )
    .unwrap();
    fs::write(payload.join("files/hook.sh"), "old authored helper").unwrap();
    fs::write(repo.work.join("resources/runtime.sh"),"set -eu\nmkdir -p \"$1/.claude/scripts\"\nprintf 'shared runtime' > \"$1/.claude/scripts/runtime.js\"\n").unwrap();
    let mut catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.work.join("catalog.json")).unwrap()).unwrap();
    let upstream = json!({"argv":["sh","resources/runtime.sh"]});
    catalog["upstreams"] = json!({"runtime":upstream});
    catalog["dependencies"] = json!({"runtime":{"origin":"upstream","upstream":"runtime","files":{"root":".","entries":[{"target":".claude/scripts/runtime.js","source":".claude/scripts/runtime.js"}]}}});
    catalog["resources"].as_array_mut().unwrap().push(json!({"id":"old-hook","kind":"hook","origin":"self","scope":"user","dependencies":["runtime"],"files":{"root":"resources/old-hook","entries":[{"target":".codex/hooks.json","fragment":"config.json"},{"target":".codex/hooks/complete.sh","source":"files/hook.sh"}]}}));
    repo.catalog(catalog.clone());
    repo.publish("Install complete Hook before rename");
    assert!(synchronize(&runtime, &state).unwrap().ok());
    let config = runtime.home.join(".codex/hooks.json");
    let helper = runtime.home.join(".codex/hooks/complete.sh");
    let shared = runtime.home.join(".claude/scripts/runtime.js");
    let before_config = fs::read(&config).unwrap();
    let before_helper = fs::read(&helper).unwrap();
    let before_shared = fs::read(&shared).unwrap();
    let before_owned = state.owned().unwrap();
    fs::rename(&payload, repo.work.join("resources/new-hook")).unwrap();
    fs::write(
        repo.work.join("resources/new-hook/config.json"),
        json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":"new-command"}]}]}})
            .to_string(),
    )
    .unwrap();
    fs::write(
        repo.work.join("resources/new-hook/files/hook.sh"),
        "new authored helper",
    )
    .unwrap();
    catalog["resources"][1]["id"] = json!("new-hook");
    catalog["resources"][1]["files"]["root"] = json!("resources/new-hook");
    catalog["upstreams"]["runtime"] = json!({"argv":["false"]});
    repo.catalog(catalog.clone());
    repo.publish("Rename complete Hook while upstream dependency is unavailable");
    let report = synchronize(&runtime, &state).unwrap();
    assert_eq!(report.results["new-hook"].status, ApplyStatus::Failed);
    assert_eq!(report.results["old-hook"].status, ApplyStatus::Pending);
    assert_eq!(fs::read(&config).unwrap(), before_config);
    assert_eq!(fs::read(&helper).unwrap(), before_helper);
    assert_eq!(fs::read(&shared).unwrap(), before_shared);
    assert_eq!(state.owned().unwrap(), before_owned);
    catalog["upstreams"]["runtime"] = upstream;
    repo.catalog(catalog);
    repo.publish("Retry complete renamed Hook with available support");
    let report = synchronize(&runtime, &state).unwrap();
    assert!(report.ok(), "{report:?}");
    assert_eq!(report.results["new-hook"].status, ApplyStatus::Applied);
    assert!(fs::read_to_string(&config).unwrap().contains("new-command"));
    assert!(!fs::read_to_string(&config).unwrap().contains("old-command"));
    assert_eq!(fs::read(&helper).unwrap(), b"new authored helper");
    assert_eq!(fs::read(&shared).unwrap(), before_shared);
    let owned = state.owned().unwrap();
    assert!(!owned.iter().any(|write| write.resource_id == "old-hook"));
    for path in [&config, &helper, &shared] {
        assert!(
            owned
                .iter()
                .any(|write| write.resource_id == "new-hook" && &write.path == path)
        );
    }
}

#[test]
fn native_installation_pending_does_not_defer_ready_payload_retirement() {
    let repo = Repo::new();
    let runtime = runtime(&repo);
    let state = runtime.state().unwrap();
    assert!(synchronize(&runtime, &state).unwrap().ok());
    let old = runtime.home.join(".agents/skills/review/SKILL.md");
    assert!(old.is_file());
    fs::create_dir_all(runtime.home.join(".codex/cache/sample")).unwrap();
    fs::write(runtime.home.join(".codex/config.toml"),"[marketplaces.market]\nsource_type='git'\nsource='https://example.invalid/plugins'\n[plugins.'sample@market']\nenabled=false\n").unwrap();
    fs::rename(
        repo.work.join("resources/review"),
        repo.work.join("resources/replacement"),
    )
    .unwrap();
    let mut catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.work.join("catalog.json")).unwrap()).unwrap();
    catalog["resources"][0]["id"] = json!("replacement");
    catalog["resources"][0]["payload"] = json!("resources/replacement");
    catalog["resources"].as_array_mut().unwrap().push(json!({"id":"native-policy","kind":"policy","origin":"self","scope":"user","installation":{"agent":"codex","source":"https://example.invalid/plugins","selector":"sample@market","enabled":false,"paths":["~/.codex/cache/sample"],"update":[["true"]]}}));
    repo.catalog(catalog);
    repo.publish("Replace payload while native policy awaits verification");
    let loaded = sm::model::Catalog::load(&repo.work).unwrap();
    let sources = loaded
        .resources
        .iter()
        .map(|resource| (resource.id.clone(), repo.work.clone()))
        .collect();
    assert_eq!(
        sm::apply::plan_apply(&loaded, &sources, &runtime.home).results["native-policy"].status,
        ApplyStatus::Pending
    );
    let report = synchronize(&runtime, &state).unwrap();
    assert!(report.ok(), "{report:?}");
    assert_eq!(report.results["native-policy"].status, ApplyStatus::Applied);
    assert_eq!(report.results["replacement"].status, ApplyStatus::Applied);
    assert!(!old.exists());
    assert!(
        runtime
            .home
            .join(".agents/skills/replacement/SKILL.md")
            .is_file()
    );
    assert!(
        !state
            .owned()
            .unwrap()
            .iter()
            .any(|write| write.resource_id == "review")
    );
}
