mod common;
use common::Repo;
use serde_json::json;
use sm::{
    adoption::{adopt, fingerprint},
    apply::{apply_writes, plan_apply},
    model::{Catalog, OwnedWrite, WriteBody},
    state::State,
};
use std::{collections::BTreeMap, fs};

#[test]
fn reviewed_fingerprint_adopts_exact_existing_files_once_and_preserves_later_edits() {
    let repo = Repo::new();
    let home = repo.temp.path().join("home");
    let target = home.join(".agents/skills/review/SKILL.md");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "Original").unwrap();
    let catalog = Catalog::load(&repo.work).unwrap();
    let state = repo.state();
    let plan = plan_apply(
        &catalog,
        &BTreeMap::from([("review".into(), repo.work.clone())]),
        &home,
    );
    assert!(!apply_writes(&plan, &state).unwrap().ok());
    fs::write(repo.work.join("resources/adoption.json"),json!([{"resource":"review","target":".agents/skills/review/SKILL.md","sha256":fingerprint(b"Original")}]).to_string()).unwrap();
    adopt(&repo.work, &catalog, &home, &plan, &state).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"Original");
    assert!(apply_writes(&plan, &state).unwrap().ok());
    assert_eq!(fs::read(&target).unwrap(), b"Approved");
    fs::write(&target, "Locally changed").unwrap();
    adopt(&repo.work, &catalog, &home, &plan, &state).unwrap();
    assert!(!apply_writes(&plan, &state).unwrap().ok());
    assert_eq!(fs::read(&target).unwrap(), b"Locally changed");
}

#[test]
fn fingerprint_mismatch_never_takes_ownership() {
    let repo = Repo::new();
    let home = repo.temp.path().join("home");
    let target = home.join(".agents/skills/review/SKILL.md");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "Private edit").unwrap();
    let catalog = Catalog::load(&repo.work).unwrap();
    let state = repo.state();
    let plan = plan_apply(
        &catalog,
        &BTreeMap::from([("review".into(), repo.work.clone())]),
        &home,
    );
    fs::write(repo.work.join("resources/adoption.json"),json!([{"resource":"review","target":".agents/skills/review/SKILL.md","sha256":fingerprint(b"Original")}]).to_string()).unwrap();
    adopt(&repo.work, &catalog, &home, &plan, &state).unwrap();
    assert!(state.owned().unwrap().is_empty());
    assert!(!apply_writes(&plan, &state).unwrap().ok());
}

#[test]
fn reviewed_upstream_baseline_is_governed_when_refresh_fails_without_changing_content() {
    let repo = Repo::new();
    repo.catalog(json!({"projects":{},"resources":[{"id":"review","kind":"skill","origin":"upstream","scope":"user","project":null,"payload":".","upstream":{"git":"https://example.invalid/offline"}}]}));
    let home = repo.temp.path().join("home");
    let target = home.join(".agents/skills/review/SKILL.md");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "Reviewed existing upstream").unwrap();
    fs::write(repo.work.join("resources/adoption.json"),json!([{"resource":"review","target":".agents/skills/review/SKILL.md","sha256":fingerprint(b"Reviewed existing upstream")}]).to_string()).unwrap();
    let catalog = Catalog::load(&repo.work).unwrap();
    let state = repo.state();
    let plan = plan_apply(&catalog, &BTreeMap::new(), &home);
    adopt(&repo.work, &catalog, &home, &plan, &state).unwrap();
    assert_eq!(state.owned().unwrap().len(), 1);
    assert!(!apply_writes(&plan, &state).unwrap().ok());
    assert_eq!(
        fs::read_to_string(target).unwrap(),
        "Reviewed existing upstream"
    );
    assert_eq!(state.owned().unwrap().len(), 1);
}

#[test]
fn exact_config_fragment_adoption_retains_local_credentials_and_other_owners() {
    let repo = Repo::new();
    let home = repo.temp.path().join("home");
    fs::create_dir_all(home.join(".codex")).unwrap();
    let path = home.join(".codex/config.toml");
    fs::write(&path,"# local comment\n[mcp_servers.private]\ncommand = 'private'\n[other]\nsecret = 'local-only'\n").unwrap();
    let fragment = json!({"mcp_servers":{"private":{"command":"private"}}})
        .as_object()
        .unwrap()
        .clone();
    let plan = sm::model::ApplyPlan {
        writes: vec![OwnedWrite::new(
            "review",
            path.clone(),
            WriteBody::Toml(fragment.clone()),
        )],
        ..Default::default()
    };
    let catalog = Catalog::load(&repo.work).unwrap();
    let state = State::open(repo.temp.path().join("adoption.sqlite3")).unwrap();
    fs::write(repo.work.join("resources/adoption.json"),json!([{"resource":"review","target":".codex/config.toml","sha256":fingerprint(&serde_json::to_vec(&fragment).unwrap())}]).to_string()).unwrap();
    adopt(&repo.work, &catalog, &home, &plan, &state).unwrap();
    assert!(apply_writes(&plan, &state).unwrap().ok());
    let text = fs::read_to_string(path).unwrap();
    assert!(text.contains("# local comment") && text.contains("local-only"));
    assert!(
        !serde_json::to_string(&state.owned().unwrap())
            .unwrap()
            .contains("local-only")
    );
}
