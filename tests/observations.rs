use serde_json::json;
use sm::{
    model::{Catalog, Client, Resource},
    observe::{ingest_events, pending_observations, record_event},
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
