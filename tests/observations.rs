use serde_json::json;
use sm::{
    model::{Catalog, Client, Resource},
    observe::{capture_health, ingest_events, pending_observations, record_event, record_failure},
    privacy::{redact, redact_text},
    state::State,
};
use std::{collections::BTreeMap, fs, io::Write};
use tempfile::tempdir;
fn state(root: &std::path::Path) -> State {
    let state = State::open(root.join("state.sqlite3")).unwrap();
    let resource:Resource=serde_json::from_value(json!({"id":"review","kind":"skill","origin":"self","scope":"user","project":null,"payload":"resources/review"})).unwrap();
    state
        .set(
            "catalog",
            &Catalog {
                resources: vec![resource],
                ..Default::default()
            },
        )
        .unwrap();
    state
}
#[test]
fn secrets_are_removed_from_nested_and_free_text_records() {
    for secret in [
        "OPENAI_API_KEY=dummy-sensitive",
        "password=\"dummy-sensitive\"",
        "Authorization: Basic dummy-sensitive",
        "https://user:dummy-sensitive@example.invalid",
    ] {
        assert!(!redact_text(secret, None).contains("dummy-sensitive"));
    }
    assert_eq!(
        redact(
            &json!({"api_key":"private","tool_input":{"command":"API_KEY=dummy-sensitive curl example.invalid"}}),
            None
        )["api_key"],
        "[redacted]"
    );
    assert_eq!(
        redact_text("api_key=\"$LOCAL_API_KEY\"", None),
        "api_key=\"$LOCAL_API_KEY\""
    );
    assert_eq!(redact_text("中文字符", Some(2)), "中文");
}
#[test]
fn native_ids_deduplicate_but_legitimate_repeats_remain() {
    let root = tempdir().unwrap();
    let state = state(root.path());
    let event = json!({"session_id":"session","event_id":"event","hook_event_name":"PostToolUse","resource_id":"review"});
    record_event(Client::Codex, &event, root.path()).unwrap();
    record_event(Client::Codex, &event, root.path()).unwrap();
    assert_eq!(ingest_events(root.path(), &state).unwrap().len(), 1);
    assert!(ingest_events(root.path(), &state).unwrap().is_empty());
    for _ in 0..2 {
        record_event(
            Client::Codex,
            &json!({"hook_event_name":"Stop"}),
            root.path(),
        )
        .unwrap();
    }
    assert_eq!(ingest_events(root.path(), &state).unwrap().len(), 2);
}
#[test]
fn complete_bad_records_are_skipped_and_partial_record_is_retained() {
    let root = tempdir().unwrap();
    let state = state(root.path());
    let events = root.path().join("events");
    fs::create_dir(&events).unwrap();
    let file = events.join("0000.jsonl");
    fs::write(&file, "[]\n{\"payload\":[]}\n{bad}\n").unwrap();
    record_event(
        Client::Kimi,
        &json!({"hook_event_name":"Stop"}),
        root.path(),
    )
    .unwrap();
    assert_eq!(ingest_events(root.path(), &state).unwrap().len(), 1);
    let errors: Vec<sm::observe::RecordingError> = state.get("recording_errors").unwrap();
    assert_eq!(errors.len(), 3);
    let mut stream = fs::OpenOptions::new().append(true).open(&file).unwrap();
    stream
        .write_all(
            b"{\"id\":\"partial\",\"agent\":\"codex\",\"timestamp\":\"2026-10-21\",\"payload\":{}",
        )
        .unwrap();
    assert!(ingest_events(root.path(), &state).unwrap().is_empty());
    stream.write_all(b"}\n").unwrap();
    assert_eq!(ingest_events(root.path(), &state).unwrap().len(), 1);
    assert!(ingest_events(root.path(), &state).unwrap().is_empty());
}
#[test]
fn upstream_events_never_enter_optimization_records() {
    let root = tempdir().unwrap();
    let state = state(root.path());
    let upstream:Resource=serde_json::from_value(json!({"id":"external","kind":"skill","origin":"upstream","scope":"user","project":null,"payload":".","upstream":{"git":"https://example.invalid/skill"}})).unwrap();
    state
        .set(
            "catalog",
            &Catalog {
                resources: vec![upstream],
                ..Default::default()
            },
        )
        .unwrap();
    record_event(
        Client::Codex,
        &json!({"resource_id":"external","hook_event_name":"Stop"}),
        root.path(),
    )
    .unwrap();
    assert!(ingest_events(root.path(), &state).unwrap().is_empty());
    assert!(pending_observations(&state, 100).unwrap().is_empty());
}
#[test]
fn outputs_are_digest_only_with_deterministic_unicode_hashes() {
    let root = tempdir().unwrap();
    let state = state(root.path());
    let mut hashes = Vec::new();
    for (index, output) in [
        json!({"status":"完成 🚀","secret":"raw-sensitive-output"}),
        json!({"secret":"raw-sensitive-output","status":"完成 🚀"}),
        json!({"status":"改变","secret":"raw-sensitive-output"}),
    ]
    .into_iter()
    .enumerate()
    {
        let event = json!({"session_id":"session","event_id":index,"resource_id":"review","hook_event_name":"PostToolUse","tool_output":output});
        record_event(Client::Codex, &event, root.path()).unwrap();
        let row = ingest_events(root.path(), &state).unwrap().remove(0);
        assert!(!row.facts.contains_key("tool_output"));
        assert!(
            !serde_json::to_string(&row)
                .unwrap()
                .contains("raw-sensitive-output")
        );
        hashes.push(row.facts["result_hash"].as_str().unwrap().to_owned());
    }
    assert_eq!(hashes[0], hashes[1]);
    assert_ne!(hashes[0], hashes[2]);
    assert!(hashes.iter().all(|hash| hash.len() == 64));
}
#[test]
fn active_projection_paths_attach_content_hash_without_claiming_hidden_behavior() {
    let root = tempdir().unwrap();
    let state = state(root.path());
    let path = "/tmp/managed/SKILL.md";
    fs::write(root.path().join("active.json"),json!({"owned":[{"resource_id":"review","path":path}],"hashes":{"review":"old-content-hash"}}).to_string()).unwrap();
    record_event(
        Client::Codex,
        &json!({"hook_event_name":"PostToolUse","tool_input":{"command":format!("cat {path}")}}),
        root.path(),
    )
    .unwrap();
    let row = ingest_events(root.path(), &state).unwrap().remove(0);
    assert_eq!(row.resource_id.as_deref(), Some("review"));
    assert_eq!(
        row.resource_content_hash.as_deref(),
        Some("old-content-hash")
    );
    assert_eq!(serde_json::to_value(row.coverage).unwrap(), "observed");
    let offsets: BTreeMap<String, u64> = state.get("event_offsets").unwrap();
    assert!(offsets.values().all(|n| *n > 0));
}

#[test]
fn native_outcomes_are_normalized_before_full_output_is_discarded() {
    let root = tempdir().unwrap();
    let state = state(root.path());
    record_event(
        Client::Codex,
        &json!({"hook_event_name":"PostToolUse", "resource_id":"review", "tool_name":"Bash",
            "tool_response":{"status":"failed", "exit_code":23, "duration_ms":14,
                "stdout":"private-business-output".repeat(100), "stderr":"API_KEY=dummy-sensitive failed"}}),
        root.path(),
    ).unwrap();
    let row = ingest_events(root.path(), &state).unwrap().remove(0);
    assert_eq!(row.facts["outcome"], "failure");
    assert_eq!(row.facts["exit_code"], 23);
    assert_eq!(row.facts["duration_ms"], 14);
    assert_eq!(
        serde_json::to_value(row.evidence_kind).unwrap(),
        "tool_execution"
    );
    let saved = serde_json::to_string(&row).unwrap();
    assert!(!saved.contains("private-business-output"));
    assert!(!saved.contains("dummy-sensitive"));
    assert!(!row.facts.contains_key("tool_response"));
    assert!(row.facts["error_summary"].as_str().unwrap().chars().count() <= 240);
}

#[test]
fn evidence_names_only_the_signal_the_client_reported() {
    let root = tempdir().unwrap();
    let state = state(root.path());
    for (payload, kind) in [
        (
            json!({"event":"PostToolUse", "tool_name":"Read", "tool_input":{"path":"/tmp/SKILL.md"}}),
            "file_read",
        ),
        (
            json!({"event":"SkillActivated", "resource_id":"review"}),
            "native_activation",
        ),
        (
            json!({"event":"PostToolUse", "tool_name":"Bash", "resource_id":"review"}),
            "tool_execution",
        ),
        (json!({"event":"Stop", "resource_id":"review"}), "unknown"),
        (
            json!({"event":"PreToolUse", "tool_name":"Read", "resource_id":"review"}),
            "unknown",
        ),
    ] {
        record_event(Client::Codex, &payload, root.path()).unwrap();
        let row = ingest_events(root.path(), &state).unwrap().remove(0);
        assert_eq!(serde_json::to_value(row.evidence_kind).unwrap(), kind);
        assert!(!row.facts.contains_key("outcome"));
        assert!(!row.facts.contains_key("exit_code"));
        assert!(!row.facts.contains_key("duration_ms"));
    }
}

#[test]
fn concurrent_capture_counts_every_attempt_with_private_bounded_health() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempdir().unwrap();
    let path = root.path().to_owned();
    let threads: Vec<_> = (0..12)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || {
                for _ in 0..4 {
                    record_event(Client::Codex, &json!({"event":"Stop"}), &path).unwrap();
                    record_failure(
                        Client::Kimi,
                        &path,
                        "API_KEY=dummy-sensitive capture failed",
                    )
                    .unwrap();
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    record_event(
        Client::Codex,
        &json!({"event":"Stop", "prompt":"x".repeat(10000)}),
        &path,
    )
    .unwrap();
    let health = capture_health(&path).unwrap();
    assert_eq!(health.clients[&Client::Codex].received, 49);
    assert_eq!(health.clients[&Client::Codex].truncated, 1);
    assert_eq!(health.clients[&Client::Kimi].received, 48);
    assert_eq!(health.clients[&Client::Kimi].failures, 48);
    assert_eq!(health.clients[&Client::Kimi].gaps, 48);
    assert!(health.clients[&Client::Kimi].last_event.is_some());
    let bytes = fs::read(path.join("capture/kimi.json")).unwrap();
    assert!(bytes.len() < 1000);
    assert!(
        !String::from_utf8(bytes)
            .unwrap()
            .contains("dummy-sensitive")
    );
    for file in [
        path.join("capture/kimi.json"),
        path.join("capture/codex.json"),
    ] {
        assert_eq!(
            fs::metadata(file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let events = fs::read_dir(path.join("events"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .unwrap();
    assert_eq!(fs::read_to_string(events).unwrap().lines().count(), 49);
}

#[test]
fn upstream_explicit_id_cannot_be_overridden_by_a_self_resource_path() {
    let root = tempdir().unwrap();
    let state = state(root.path());
    let mut catalog: Catalog = state.get("catalog").unwrap();
    catalog.resources.push(serde_json::from_value(json!({"id":"external", "kind":"skill", "origin":"upstream", "scope":"user", "project":null, "payload":".", "upstream":{"git":"https://example.invalid/external"}})).unwrap());
    state.set("catalog", &catalog).unwrap();
    fs::write(
        root.path().join("active.json"),
        json!({"owned":[{"resource_id":"review","path":"/tmp/self/SKILL.md"}],"hashes":{}})
            .to_string(),
    )
    .unwrap();
    record_event(Client::Codex, &json!({"resource_id":"external", "event":"PostToolUse", "tool_input":{"path":"/tmp/self/SKILL.md"}}), root.path()).unwrap();
    assert!(ingest_events(root.path(), &state).unwrap().is_empty());
}

#[test]
fn reading_empty_capture_health_does_not_create_files() {
    let root = tempdir().unwrap();
    let health = capture_health(root.path()).unwrap();
    assert!(
        health
            .clients
            .values()
            .all(|client| client.received == 0 && client.last_event.is_none())
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn unicode_metadata_stays_within_the_byte_cap_and_resource_labels_are_redacted() {
    let root = tempdir().unwrap();
    let metadata = "🚀".repeat(4000);
    record_event(
        Client::Codex,
        &json!({"session_id":metadata, "cwd":metadata,
        "hook_event_name":metadata,"event":metadata,"tool_name":metadata,
        "resource_id":"API_KEY=dummy-sensitive"}),
        root.path(),
    )
    .unwrap();
    let path = fs::read_dir(root.path().join("events"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .unwrap();
    let bytes = fs::read(path).unwrap();
    assert!(
        bytes.len() <= 32769,
        "{} bytes exceeded capture cap",
        bytes.len()
    );
    assert!(
        !String::from_utf8(bytes)
            .unwrap()
            .contains("dummy-sensitive")
    );
}

#[test]
fn internal_hook_runs_skip_events_and_health_even_for_invalid_input() {
    use std::process::{Command, Stdio};
    let root = tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_sm"))
        .args(["record", "--agent", "codex", "--state-dir"])
        .arg(root.path())
        .env("SM_INTERNAL_RUN", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"{bad}").unwrap();
    let _ = child.wait().unwrap();
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn explicit_capture_gaps_and_native_failures_remain_visible() {
    let root = tempdir().unwrap();
    let state = state(root.path());
    record_event(
        Client::Kimi,
        &json!({"event":"CaptureGap", "truncated":true}),
        root.path(),
    )
    .unwrap();
    let health = capture_health(root.path()).unwrap();
    assert_eq!(health.clients[&Client::Kimi].gaps, 1);
    assert_eq!(health.clients[&Client::Kimi].truncated, 1);
    record_event(Client::Codex, &json!({"hook_event_name":"PostToolUseFailure", "tool_name":"Bash", "error":"API_KEY=dummy-sensitive failed"}), root.path()).unwrap();
    let rows = ingest_events(root.path(), &state).unwrap();
    let failed = rows.iter().find(|row| row.agent == Client::Codex).unwrap();
    assert_eq!(failed.facts["outcome"], "failure");
    assert!(
        !serde_json::to_string(failed)
            .unwrap()
            .contains("dummy-sensitive")
    );
}
