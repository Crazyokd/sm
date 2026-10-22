use chrono::{Duration, Utc};
use serde_json::json;
use sm::{
    model::{Catalog, Resource},
    observe::{complete_observations, ingest_events, pending_observations},
    retention::{self, UsageTotal},
    state::State,
};
use std::{
    collections::BTreeMap,
    fs,
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use tempfile::tempdir;

fn state(root: &Path) -> State {
    let state = State::open(root.join("state.sqlite3")).unwrap();
    let resource: Resource = serde_json::from_value(json!({"id":"review","kind":"skill","origin":"self","scope":"user","project":null,"payload":"resources/review"})).unwrap();
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
fn envelope(id: &str, timestamp: &str) -> String {
    json!({"id":id,"agent":"codex","timestamp":timestamp,"payload":{"resource_id":"review","event":"Stop"}}).to_string() + "\n"
}
fn day(days: i64) -> String {
    (Utc::now() - Duration::days(days)).date_naive().to_string()
}
fn raw(root: &Path, days: i64, contents: &str) -> PathBuf {
    let folder = root.join("events");
    fs::create_dir_all(&folder).unwrap();
    let path = folder.join(format!("{}.jsonl", day(days)));
    fs::write(&path, contents).unwrap();
    path
}
fn consumed_sparse(root: &Path, state: &State, days: i64, bytes: u64) -> PathBuf {
    use std::os::unix::fs::MetadataExt;
    let path = raw(root, days, "");
    let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(bytes).unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    file.write_all(b"\n").unwrap();
    let mut offsets: BTreeMap<String, u64> = state.get("event_offsets").unwrap();
    offsets.insert(path.to_string_lossy().into_owned(), bytes);
    state.set("event_offsets", &offsets).unwrap();
    let metadata = file.metadata().unwrap();
    let mut stamps: BTreeMap<String, serde_json::Value> = state.get("event_stamps").unwrap();
    stamps.insert(
        path.to_string_lossy().into_owned(),
        json!({"device":metadata.dev(),"inode":metadata.ino(),
        "modified_seconds":metadata.mtime(),"modified_nanoseconds":metadata.mtime_nsec(),
        "changed_seconds":metadata.ctime(),"changed_nanoseconds":metadata.ctime_nsec()}),
    );
    state.set("event_stamps", &stamps).unwrap();
    path
}

#[test]
fn old_detail_is_retired_but_pending_and_both_evidence_sources_survive() {
    let root = tempdir().unwrap();
    let state = state(root.path());
    let old = day(30);
    let path = raw(
        root.path(),
        30,
        &[
            "retire",
            "pending",
            "proposal-pin",
            "finding-pin",
            "unknown",
        ]
        .map(|id| envelope(id, if id == "unknown" { "unknown" } else { &old }))
        .concat(),
    );
    assert_eq!(ingest_events(root.path(), &state).unwrap().len(), 5);
    complete_observations(
        &state,
        &["retire", "proposal-pin", "finding-pin", "unknown"].map(str::to_owned),
    )
    .unwrap();
    state.set("proposals", &json!({"one":{"key":"one","directory":"/unused","branch":"sm/one","change":{"resource_ids":["review"],"reason":"review","evidence":["Captured event `proposal-pin` · codex · PostToolUse · observed: {}"]},"status":"submitted","mr_url":"https://example.invalid/mr/1"}})).unwrap();
    state.set("findings", &json!([{"resource_id":"review","issue":"review","evidence":["finding-pin"],"validation":{}}])).unwrap();
    let report = retention::maintain(&state).unwrap();
    assert_eq!(report.removed_observations, 1);
    assert_eq!(report.removed_raw_files, 1);
    assert!(!path.exists());
    assert_eq!(report.after.observations, 4);
    assert_eq!(report.after.pending_observations, 1);
    assert_eq!(report.after.pinned_observations, 2);
    assert_eq!(report.after.unknown_timestamp_observations, 1);
    let totals: BTreeMap<String, UsageTotal> = state.get("usage_totals").unwrap();
    assert_eq!(totals["review"].events, 1);
    assert_eq!(totals["review"].last_seen.as_deref(), Some(old.as_str()));
    assert_eq!(retention::maintain(&state).unwrap().removed_observations, 0);

    // Recreated or replayed files cannot resurrect retired IDs, while old unseen events still ingest.
    raw(
        root.path(),
        30,
        &(envelope("retire", &old) + &envelope("late", &old)),
    );
    state
        .set("event_offsets", &BTreeMap::<String, u64>::new())
        .unwrap();
    let added = ingest_events(root.path(), &state).unwrap();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].id, "late");
    assert_eq!(pending_observations(&state, 100).unwrap().len(), 2);
    complete_observations(&state, &["late".into()]).unwrap();
    assert_eq!(retention::maintain(&state).unwrap().removed_observations, 1);
    let totals: BTreeMap<String, UsageTotal> = state.get("usage_totals").unwrap();
    assert_eq!(totals["review"].events, 2);
}

#[test]
fn cap_removes_consumed_closed_files_even_within_the_age_window() {
    let root = tempdir().unwrap();
    let state = state(root.path());
    let old = consumed_sparse(root.path(), &state, 2, 30 * 1024 * 1024);
    let active = consumed_sparse(root.path(), &state, 0, 25 * 1024 * 1024);
    let report = retention::maintain(&state).unwrap();
    assert!(report.before.over_budget);
    assert_eq!(report.removed_raw_bytes, 30 * 1024 * 1024);
    assert!(!old.exists());
    assert!(active.exists());
    assert!(!report.after.over_budget);
    assert_eq!(report.after.protected_raw_files, 1);
}

#[test]
fn partial_unprocessed_and_unknown_files_remain_and_protected_overage_is_visible() {
    let root = tempdir().unwrap();
    let state = state(root.path());
    let partial = raw(
        root.path(),
        31,
        &(envelope("old", &day(31)) + "{\"id\":\"tail\""),
    );
    assert_eq!(ingest_events(root.path(), &state).unwrap().len(), 1);
    complete_observations(&state, &["old".into()]).unwrap();
    let pending = raw(root.path(), 30, &envelope("not-ingested", &day(30)));
    let unknown = root.path().join("events/unknown.jsonl");
    fs::write(&unknown, "{}\n").unwrap();
    let active = consumed_sparse(root.path(), &state, 0, retention::RAW_LIMIT_BYTES + 1);
    let report = retention::maintain(&state).unwrap();
    assert_eq!(report.removed_raw_files, 0);
    assert_eq!(report.after.raw_files, 4);
    assert_eq!(report.after.protected_raw_files, 4);
    assert_eq!(report.after.pending_raw_files, 3);
    assert_eq!(report.after.unknown_raw_files, 1);
    assert!(report.after.over_budget);
    assert!(report.after.over_budget_bytes > 0);
    assert_eq!(report.after.protected_raw_bytes, report.after.raw_bytes);
    for path in [&partial, &pending, &unknown, &active] {
        assert!(path.exists());
    }
}

#[test]
fn current_raw_file_stays_and_health_reads_do_not_change_state() {
    let root = tempdir().unwrap();
    let state = state(root.path());
    let first = retention::summary(&state).unwrap();
    assert_eq!(first.retention_days, 14);
    assert_eq!(first.raw_limit_bytes, 50 * 1024 * 1024);
    assert!(!root.path().join("events").exists());
    assert_eq!(
        state
            .connect()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM kv", [], |row| row.get::<_, u64>(0))
            .unwrap(),
        1
    );
    let path = raw(root.path(), 0, &envelope("current", &day(0)));
    ingest_events(root.path(), &state).unwrap();
    complete_observations(&state, &["current".into()]).unwrap();
    assert_eq!(retention::maintain(&state).unwrap().removed_raw_files, 0);
    assert!(path.exists());
    assert_eq!(retention::summary(&state).unwrap().observations, 1);
}

#[test]
fn stale_offsets_do_not_authorize_deleting_a_replaced_same_length_file() {
    let root = tempdir().unwrap();
    let state = state(root.path());
    let old = day(30);
    let path = raw(root.path(), 30, &envelope("old", &old));
    ingest_events(root.path(), &state).unwrap();
    complete_observations(&state, &["old".into()]).unwrap();
    let replacement = root.path().join("replacement");
    fs::write(&replacement, envelope("new", &old)).unwrap();
    fs::rename(replacement, &path).unwrap();
    let report = retention::maintain(&state).unwrap();
    assert_eq!(report.removed_raw_files, 0);
    assert_eq!(report.after.pending_raw_files, 1);
    assert!(path.exists());
    let rows = ingest_events(root.path(), &state).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "new");
}
