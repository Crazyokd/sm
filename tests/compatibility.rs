mod common;
use common::{FakeMrApi, Repo, git};
use serde_json::{Value, json};
use sm::{
    model::{Resource, WriteBody},
    observe::{ingest_events, pending_observations, record_event},
    proposals::{Proposal, create_proposal},
    service::{Runtime, synchronize},
    state::State,
};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::Path};
fn legacy(root: &Path) -> State {
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/legacy/expected.json")).unwrap();
    let source = include_str!("fixtures/legacy/state.sql")
        .replace("/legacy/home", root.join("home").to_str().unwrap())
        .replace(
            expected["old_state_dir"].as_str().unwrap(),
            root.to_str().unwrap(),
        );
    let path = root.join("state.sqlite3");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(&source).unwrap();
    drop(db);
    State::open(path).unwrap()
}
#[test]
fn previous_database_preserves_ownership_observations_offsets_and_pending_proposals() {
    let root = tempfile::tempdir().unwrap();
    let state = legacy(root.path());
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/legacy/expected.json")).unwrap();
    let owned = state.owned().unwrap();
    assert_eq!(owned.len(), 3);
    let file = owned.iter().find(|w| w.resource_id == "review").unwrap();
    assert_eq!(
        file.body,
        WriteBody::File {
            bytes: b"Approved legacy content\n".to_vec(),
            executable: true
        }
    );
    let resources: Vec<Resource> = state.get("resources").unwrap();
    assert_eq!(resources.len(), 1);
    let proposals: BTreeMap<String, Proposal> = state.get("proposals").unwrap();
    let key = expected["proposal_key"].as_str().unwrap();
    assert_eq!(proposals[key].change.key().unwrap(), key);
    let events = pending_observations(&state, 100).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, expected["event_id"].as_str().unwrap());
    let offsets: BTreeMap<String, u64> = state.get("event_offsets").unwrap();
    assert_eq!(offsets.len(), 1);
    assert!(offsets.values().all(|n| *n > 0));
}
#[test]
fn existing_projection_and_old_python_collectors_continue_without_duplicate_hooks() {
    let repo = Repo::new();
    let state = legacy(repo.temp.path());
    let home = repo.temp.path().join("home");
    let live = home.join(".agents/skills/review/SKILL.md");
    fs::create_dir_all(live.parent().unwrap()).unwrap();
    fs::write(&live, "Approved legacy content\n").unwrap();
    fs::set_permissions(&live, fs::Permissions::from_mode(0o700)).unwrap();
    for (folder, name, text) in [
        (
            ".codex",
            "hooks.json",
            include_str!("fixtures/legacy/hooks.json"),
        ),
        (
            ".kimi-code",
            "config.toml",
            include_str!("fixtures/legacy/config.toml"),
        ),
    ] {
        fs::create_dir_all(home.join(folder)).unwrap();
        fs::write(
            home.join(folder).join(name),
            text.replace("/legacy/home", home.to_str().unwrap()),
        )
        .unwrap();
    }
    fs::write(
        repo.work.join("resources/review/SKILL.md"),
        "Approved legacy content\n",
    )
    .unwrap();
    fs::set_permissions(
        repo.work.join("resources/review/SKILL.md"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    repo.publish("Match installed legacy content");
    let runtime = Runtime {
        repo: repo.work.clone(),
        home: home.clone(),
        executable: env!("CARGO_BIN_EXE_sm").into(),
    };
    let report = synchronize(&runtime, &state).unwrap();
    assert!(report.ok(), "{report:?}");
    let hooks = fs::read_to_string(home.join(".codex/hooks.json")).unwrap();
    assert!(!hooks.contains(".venv"));
    assert!(hooks.contains("preserved-local-hook"));
    let data: Value = serde_json::from_str(&hooks).unwrap();
    assert_eq!(data["hooks"]["Stop"].as_array().unwrap().len(), 2);
    let kimi = fs::read_to_string(home.join(".kimi-code/config.toml")).unwrap();
    assert!(kimi.contains("existing-model") && kimi.contains("retained local configuration"));
    assert!(!kimi.contains(".venv"));
    assert_eq!(pending_observations(&state, 100).unwrap().len(), 1);
    assert!(synchronize(&runtime, &state).unwrap().ok());
    assert_eq!(
        hooks,
        fs::read_to_string(home.join(".codex/hooks.json")).unwrap()
    );
}
#[test]
fn old_event_offsets_do_not_reingest_old_bytes_and_new_events_continue() {
    let root = tempfile::tempdir().unwrap();
    let state = legacy(root.path());
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/legacy/expected.json")).unwrap();
    fs::create_dir(root.path().join("events")).unwrap();
    fs::write(
        root.path()
            .join("events")
            .join(expected["old_event_file"].as_str().unwrap()),
        include_str!("fixtures/legacy/event.jsonl"),
    )
    .unwrap();
    assert!(ingest_events(root.path(), &state).unwrap().is_empty());
    record_event(sm::model::Client::Codex,&json!({"event_id":"new-event","session_id":"new-session","hook_event_name":"Stop","resource_id":"review"}),root.path()).unwrap();
    assert_eq!(ingest_events(root.path(), &state).unwrap().len(), 1);
    assert_eq!(pending_observations(&state, 100).unwrap().len(), 2);
}
#[test]
fn a_previous_pending_git_branch_is_updated_in_place() {
    let repo = Repo::new();
    let state = legacy(repo.temp.path());
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/legacy/expected.json")).unwrap();
    let key = expected["proposal_key"].as_str().unwrap();
    let directory = state.directory().unwrap().join("proposals").join(key);
    fs::create_dir_all(directory.parent().unwrap()).unwrap();
    git(
        &repo.work,
        &[
            "clone",
            "--branch",
            "main",
            repo.remote.to_str().unwrap(),
            directory.to_str().unwrap(),
        ],
    );
    git(&directory, &["config", "user.name", "SM Test"]);
    git(&directory, &["config", "user.email", "sm@example.invalid"]);
    let branch = format!("sm/{key}");
    git(&directory, &["switch", "-c", &branch]);
    fs::write(directory.join("resources/review/SKILL.md"), "Candidate").unwrap();
    git(&directory, &["commit", "-am", "减少重复步骤"]);
    git(&directory, &["push", "origin", &branch]);
    let mut proposals: BTreeMap<String, Proposal> = state.get("proposals").unwrap();
    let old = proposals.get_mut(key).unwrap();
    old.directory = directory.clone();
    state.set("proposals", &proposals).unwrap();
    let mut change = proposals[key].change.clone();
    change.writes.insert(
        "resources/review/SKILL.md".into(),
        "Candidate updated".into(),
    );
    let mut proposal = create_proposal(&repo.work, &change, &state).unwrap();
    assert_eq!(proposal.directory, directory);
    assert_eq!(proposal.key, key);
    let api = FakeMrApi::default();
    api.mrs.lock().unwrap().push(sm::gitlab::Mr {
        iid: 7,
        web_url: "https://git.example.invalid/mr/7".into(),
        source_branch: branch,
        target_branch: "main".into(),
        title: "Draft: legacy".into(),
        description: "Legacy evidence".into(),
    });
    assert_eq!(
        sm::gitlab::submit_mr_with(&mut proposal, &state, &api).unwrap(),
        "https://git.example.invalid/mr/7"
    );
    assert_eq!(api.mrs.lock().unwrap().len(), 1);
}
#[test]
fn malformed_saved_data_fails_visibly_without_clearing_it() {
    let root = tempfile::tempdir().unwrap();
    let state = legacy(root.path());
    state
        .connect()
        .unwrap()
        .execute(
            "UPDATE ownership SET value='broken' WHERE resource='review'",
            [],
        )
        .unwrap();
    assert!(state.owned().is_err());
    let count: u64 = state
        .connect()
        .unwrap()
        .query_row("SELECT count(*) FROM ownership", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 3);
}
#[test]
fn gitlab_nullable_description_is_accepted_at_the_external_boundary() {
    let mr:sm::gitlab::Mr=serde_json::from_value(json!({"iid":1,"web_url":"https://git.example.invalid/mr/1","source_branch":"sm/one","target_branch":"main","title":"Draft: update","description":null})).unwrap();
    assert!(mr.description.is_empty());
}
