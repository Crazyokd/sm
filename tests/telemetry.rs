use serde_json::json;
use sm::{
    service::JobGuard,
    state::{RunRecord, State},
    telemetry::{RunDetails, RunStatus, RunTracker, snapshot},
};

fn state() -> (tempfile::TempDir, State) {
    let root = tempfile::tempdir().unwrap();
    let state = State::open(root.path().join("state.sqlite3")).unwrap();
    (root, state)
}

#[test]
fn start_and_progress_are_persisted_and_success_keeps_business_fields() {
    let (_root, state) = state();
    let _guard = JobGuard::acquire(state.directory().unwrap()).unwrap();
    let mut tracker = RunTracker::start(&state, "sync").unwrap();
    let current = snapshot(&state).unwrap().current.unwrap();
    assert_eq!(current.status, RunStatus::Running);
    assert_eq!(current.operation, "sync");
    assert!(uuid::Uuid::parse_str(&current.id).is_ok());
    tracker
        .stage("fetch_resource", Some("review"), 1, 3)
        .unwrap();
    let current = snapshot(&state).unwrap().current.unwrap();
    assert_eq!(current.stage, "fetch_resource");
    assert_eq!(current.resource.as_deref(), Some("review"));
    assert_eq!((current.completed, current.total), (1, 3));
    tracker
        .finish(&json!({"results":{"review":"applied"}}), true, None)
        .unwrap();
    assert!(snapshot(&state).unwrap().current.is_none());
    let records: Vec<RunRecord> = state.get("runs").unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].result["results"]["review"], "applied");
    let saved: RunDetails = serde_json::from_value(records[0].result["run"].clone()).unwrap();
    assert_eq!(saved.id, current.id);
    assert_eq!(saved.status, RunStatus::Succeeded);
    assert!(saved.finished_at.is_some());
    assert!(saved.error.is_none());
}

#[test]
fn failure_is_bounded_and_redacted_and_is_not_appended_twice_on_drop() {
    let (_root, state) = state();
    let _guard = JobGuard::acquire(state.directory().unwrap()).unwrap();
    let mut tracker = RunTracker::start(&state, "cycle").unwrap();
    tracker.stage("analysis", None, 0, 1).unwrap();
    let error = format!("api_key=private-secret {}", "x".repeat(10000));
    tracker
        .finish(&json!({"ok":false}), false, Some(&error))
        .unwrap();
    drop(tracker);
    let recent = snapshot(&state).unwrap().recent;
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].status, RunStatus::Failed);
    let error = recent[0].error.as_ref().unwrap();
    assert!(error.len() <= 1600);
    assert!(!error.contains("private-secret"));
    assert!(error.contains("[redacted]"));
}

#[test]
fn dropped_tracker_records_interruption() {
    let (_root, state) = state();
    let _guard = JobGuard::acquire(state.directory().unwrap()).unwrap();
    {
        let mut tracker = RunTracker::start(&state, "sync").unwrap();
        tracker.stage("fetch_approved", None, 0, 1).unwrap();
    }
    let data = snapshot(&state).unwrap();
    assert!(data.current.is_none());
    assert_eq!(data.recent.len(), 1);
    assert_eq!(data.recent[0].status, RunStatus::Interrupted);
    assert_eq!(data.recent[0].stage, "fetch_approved");
}

#[test]
fn health_reports_lock_free_stale_snapshot_without_writing_until_next_run() {
    let (_root, state) = state();
    let guard = JobGuard::acquire(state.directory().unwrap()).unwrap();
    let tracker = RunTracker::start(&state, "cycle").unwrap();
    std::mem::forget(tracker);
    drop(guard);
    let data = snapshot(&state).unwrap();
    assert_eq!(data.current.unwrap().status, RunStatus::Interrupted);
    assert!(data.recent.is_empty());
    assert_eq!(
        state
            .get::<Option<RunDetails>>("active_run")
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Running
    );
    let _guard = JobGuard::acquire(state.directory().unwrap()).unwrap();
    let mut next = RunTracker::start(&state, "sync").unwrap();
    next.finish(&json!({"ok":true}), true, None).unwrap();
    let data = snapshot(&state).unwrap();
    assert_eq!(data.recent.len(), 2);
    assert_eq!(data.recent[0].operation, "sync");
    assert_eq!(data.recent[1].operation, "cycle");
    assert_eq!(data.recent[1].status, RunStatus::Interrupted);
}

#[test]
fn restart_does_not_duplicate_history_if_snapshot_was_not_cleared() {
    let (_root, state) = state();
    let _guard = JobGuard::acquire(state.directory().unwrap()).unwrap();
    let mut tracker = RunTracker::start(&state, "sync").unwrap();
    let old: Option<RunDetails> = state.get("active_run").unwrap();
    tracker.finish(&json!({"ok":true}), true, None).unwrap();
    state.set("active_run", &old).unwrap();
    assert!(snapshot(&state).unwrap().current.is_none());
    let mut next = RunTracker::start(&state, "sync").unwrap();
    next.finish(&json!({"ok":true}), true, None).unwrap();
    assert_eq!(snapshot(&state).unwrap().recent.len(), 2);
}

#[test]
fn failed_history_persistence_does_not_replace_failed_outcome_with_interruption_on_drop() {
    let (_root, state) = state();
    let _guard = JobGuard::acquire(state.directory().unwrap()).unwrap();
    let mut tracker = RunTracker::start(&state, "sync").unwrap();
    state.set("runs", &json!({"invalid":"history"})).unwrap();
    let error = tracker.finish(&json!({"ok":false}), false, Some("original failure"));
    assert!(error.is_err());
    state.set("runs", &Vec::<RunRecord>::new()).unwrap();
    drop(tracker);
    let current = snapshot(&state).unwrap().current.unwrap();
    assert_eq!(current.status, RunStatus::Failed);
    assert_eq!(current.error.as_deref(), Some("original failure"));
    assert!(snapshot(&state).unwrap().recent.is_empty());
}
