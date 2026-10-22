mod common;

use common::Repo;
use serde_json::json;
use sm::{diagnostics, model::Client, observe::record_event, service::Runtime};
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn diagnostic_redaction_covers_native_token_and_secret_names() {
    for text in [
        "GH_TOKEN=private-marker",
        "ONES_AUTH_TOKEN=private-marker",
        "app_secret=private-marker",
    ] {
        assert!(!sm::privacy::redact_text(text, None).contains("private-marker"));
    }
    let value = sm::privacy::redact(
        &json!({"token":"private-marker","secret":"private-marker","token_count":42}),
        None,
    );
    assert_eq!(value["token"], "[redacted]");
    assert_eq!(value["secret"], "[redacted]");
    assert_eq!(value["token_count"], 42);
}

#[test]
fn appended_raw_backlog_counts_only_new_bytes() {
    let repo = Repo::new();
    let state = repo.state();
    let directory = state.directory().unwrap();
    record_event(Client::Codex, &json!({"hook_event_name":"Stop"}), directory).unwrap();
    sm::observe::ingest_events(directory, &state).unwrap();
    let before = diagnostics::health(&state).unwrap().storage.events_bytes;
    record_event(Client::Codex, &json!({"hook_event_name":"Stop"}), directory).unwrap();
    let health = diagnostics::health(&state).unwrap();
    assert_eq!(
        health.pending_raw_bytes,
        health.storage.events_bytes - before
    );
}

#[test]
fn replaced_raw_file_remains_visible_in_byte_backlog() {
    let repo = Repo::new();
    let state = repo.state();
    let directory = state.directory().unwrap();
    record_event(Client::Codex, &json!({"hook_event_name":"Stop"}), directory).unwrap();
    sm::observe::ingest_events(directory, &state).unwrap();
    assert_eq!(diagnostics::health(&state).unwrap().pending_raw_bytes, 0);
    let path = fs::read_dir(directory.join("events"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .unwrap();
    let original = fs::read(&path).unwrap();
    let mut event: serde_json::Value = serde_json::from_slice(&original).unwrap();
    event["id"] = json!("f".repeat(64));
    let mut bytes = serde_json::to_vec(&event).unwrap();
    bytes.push(b'\n');
    assert_eq!(bytes.len(), original.len());
    let replacement = directory.join("replacement");
    fs::write(&replacement, bytes).unwrap();
    fs::rename(replacement, path).unwrap();
    let health = diagnostics::health(&state).unwrap();
    assert_eq!(health.retention.pending_raw_files, 1);
    assert!(health.pending_raw_bytes > 0);
}

#[test]
fn diagnostic_export_still_works_after_many_failures() {
    let repo = Repo::new();
    let runtime = Runtime {
        repo: repo.work.clone(),
        home: repo.temp.path().join("home"),
        executable: env!("CARGO_BIN_EXE_sm").into(),
    };
    let state = runtime.state().unwrap();
    let error = "同步失败".repeat(320);
    let records:Vec<_>=(0..100).map(|i|json!({"operation":"sync","time":"2026-01-01T00:00:01Z","result":{"error":error,"run":{
        "id":format!("run-{i}"),"operation":"sync","trigger":"manual","stage":"fetch_resource","resource":"review","completed":0,"total":79,
        "started_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:01Z","finished_at":"2026-01-01T00:00:01Z","duration_ms":1000,"status":"failed","error":error
    }}})).collect();
    state.set("runs", &records).unwrap();
    let results: std::collections::BTreeMap<_, _> = (0..79)
        .map(|i| {
            (
                format!("resource-{i}"),
                json!({"status":"failed","message":error}),
            )
        })
        .collect();
    state.set("resource_results", &results).unwrap();
    let bytes = diagnostics::bytes(&diagnostics::collect(&runtime, &state).unwrap()).unwrap();
    assert!(bytes.len() <= 256 * 1024);
    let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(report["omitted"]["runs"].as_u64().unwrap() > 0);
    assert_eq!(
        state
            .get::<Vec<sm::state::RunRecord>>("runs")
            .unwrap()
            .len(),
        100
    );
}

#[test]
fn native_diagnose_command_writes_a_private_report() {
    let repo = Repo::new();
    let task_home = repo.temp.path().join("home");
    let output = repo.temp.path().join("diagnostics.json");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_sm"))
        .arg("--repo")
        .arg(&repo.work)
        .args(["diagnose", "--output"])
        .arg(&output)
        .env("HOME", &task_home)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(report["manager_version"], env!("CARGO_PKG_VERSION"));
    assert!(report["health"]["capture"]["clients"]["codex"]["received"].is_number());
    assert_eq!(
        fs::metadata(output).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn health_exposes_capture_and_backlog_without_ingesting_or_reading_resource_bodies() {
    let repo = Repo::new();
    let state = repo.state();
    record_event(
        Client::Codex,
        &json!({"hook_event_name":"PostToolUse","tool_input":{"command":"true"}}),
        state.directory().unwrap(),
    )
    .unwrap();
    state
        .set(
            "resource_contents",
            &json!({"private":"resource-body-marker"}),
        )
        .unwrap();
    let report = diagnostics::health(&state).unwrap();
    assert_eq!(report.capture.clients[&Client::Codex].received, 1);
    assert!(report.pending_raw_bytes > 0);
    assert_eq!(report.pending_observations, 0);
    assert!(report.storage.events_bytes > 0);
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("resource-body-marker")
    );
    let count: u64 = state
        .connect()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM observations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn diagnostic_export_redacts_results_and_excludes_payloads_and_ownership() {
    let repo = Repo::new();
    let runtime = Runtime {
        repo: repo.work.clone(),
        home: repo.temp.path().join("home"),
        executable: env!("CARGO_BIN_EXE_sm").into(),
    };
    let state = runtime.state().unwrap();
    state.set("resource_results",&json!({"review":{"status":"failed","message":"Authorization: Bearer secret-result-marker"}})).unwrap();
    state
        .set(
            "resource_contents",
            &json!({"private":"resource-body-marker"}),
        )
        .unwrap();
    state.add_run("sync",&json!({"error":"password=secret-history-marker","unexpected":"untrusted-extra-marker"})).unwrap();
    let report = diagnostics::collect(&runtime, &state).unwrap();
    let text = serde_json::to_string(&report).unwrap();
    for secret in [
        "secret-result-marker",
        "secret-history-marker",
        "resource-body-marker",
        "untrusted-extra-marker",
    ] {
        assert!(!text.contains(secret), "{secret}");
    }
    let output = repo.temp.path().join("diagnostics.json");
    diagnostics::write(&output, &report).unwrap();
    assert_eq!(
        fs::metadata(&output).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::set_permissions(&output, fs::Permissions::from_mode(0o644)).unwrap();
    diagnostics::write(&output, &report).unwrap();
    assert_eq!(
        fs::metadata(&output).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(fs::read_to_string(output).unwrap().contains("[redacted]"));
}

#[test]
fn diagnostic_export_rejects_symlink_targets() {
    let repo = Repo::new();
    let runtime = Runtime {
        repo: repo.work.clone(),
        home: repo.temp.path().join("home"),
        executable: env!("CARGO_BIN_EXE_sm").into(),
    };
    let report = diagnostics::collect(&runtime, &runtime.state().unwrap()).unwrap();
    let unrelated = repo.temp.path().join("unrelated");
    fs::write(&unrelated, "keep").unwrap();
    let output = repo.temp.path().join("diagnostics.json");
    std::os::unix::fs::symlink(&unrelated, &output).unwrap();
    assert!(diagnostics::write(&output, &report).is_err());
    assert_eq!(fs::read_to_string(unrelated).unwrap(), "keep");
}
