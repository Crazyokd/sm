mod common;
use common::Repo;
use serde_json::json;
use sm::{
    adoption::{adopt, fingerprint},
    apply::{apply_writes, plan_apply},
    model::{Catalog, OwnedWrite, WriteBody},
    state::State,
};
use std::os::unix::fs::PermissionsExt;
use std::{collections::BTreeMap, fs};

#[test]
fn reviewed_handoff_moves_only_unchanged_named_file_ownership_without_writing() {
    for (previous, changed_mode, expected) in [
        ("former", false, true),
        ("unrelated", false, false),
        ("former", true, false),
    ] {
        let repo = Repo::new();
        let home = repo.temp.path().join("home");
        let path = home.join(".agents/skills/review/SKILL.md");
        let state = repo.state();
        let original = sm::model::ApplyPlan {
            writes: vec![OwnedWrite::new(
                "former",
                &path,
                WriteBody::File {
                    bytes: b"Original".to_vec(),
                    executable: false,
                },
            )],
            ..Default::default()
        };
        assert!(apply_writes(&original, &state).unwrap().ok());
        if changed_mode {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let catalog = Catalog::load(&repo.work).unwrap();
        let plan = plan_apply(
            &catalog,
            &BTreeMap::from([("review".into(), repo.work.clone())]),
            &home,
        );
        fs::write(repo.work.join("resources/adoption.json"), json!([{"resource":"review","previous_resource":previous,"target":".agents/skills/review/SKILL.md","sha256":fingerprint(b"Original"),"executable":false}]).to_string()).unwrap();
        adopt(&repo.work, &catalog, &home, &plan, &state).unwrap();
        let owned = state.owned().unwrap();
        assert_eq!(owned.len(), 1);
        assert_eq!(
            owned[0].resource_id,
            if expected { "review" } else { "former" }
        );
        assert_eq!(fs::read(&path).unwrap(), b"Original");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o100 != 0,
            changed_mode
        );
    }
}

#[test]
fn reviewed_handoff_splits_config_ownership_and_preserves_unknown_fields() {
    let repo = Repo::new();
    let home = repo.temp.path().join("home");
    fs::create_dir_all(home.join(".codex")).unwrap();
    let path = home.join(".codex/config.toml");
    fs::write(&path, "# local comment\n[other]\nvalue = 'kept'\n").unwrap();
    let state = repo.state();
    let whole = json!({"mcp_servers":{"alpha":{"command":"alpha"},"beta":{"command":"beta"}}})
        .as_object()
        .unwrap()
        .clone();
    assert!(
        apply_writes(
            &sm::model::ApplyPlan {
                writes: vec![OwnedWrite::new("former", &path, WriteBody::Toml(whole))],
                ..Default::default()
            },
            &state
        )
        .unwrap()
        .ok()
    );
    let before = fs::read(&path).unwrap();
    let resources: Vec<_> = ["alpha", "beta"].into_iter().map(|id| json!({"id":id,"kind":"mcp","origin":"self","scope":"user","project":null,"files":{"root":format!("resources/{id}"),"entries":[{"target":".codex/config.toml","toml":{"mcp_servers":{id:{"command":id}}}}]}})).collect();
    repo.catalog(json!({"projects":{},"upstreams":{},"resources":resources}));
    let catalog = Catalog::load(&repo.work).unwrap();
    let plan = plan_apply(
        &catalog,
        &BTreeMap::from([
            ("alpha".into(), repo.work.clone()),
            ("beta".into(), repo.work.clone()),
        ]),
        &home,
    );
    let adoptions: Vec<_> = plan.writes.iter().map(|w| { let WriteBody::Toml(fragment)=&w.body else {panic!("expected TOML")}; json!({"resource":w.resource_id,"previous_resource":"former","target":".codex/config.toml","sha256":fingerprint(&serde_json::to_vec(fragment).unwrap())}) }).collect();
    assert_eq!(adoptions.len(), 2);
    fs::write(
        repo.work.join("resources/adoption.json"),
        serde_json::to_vec(&adoptions).unwrap(),
    )
    .unwrap();
    adopt(&repo.work, &catalog, &home, &plan, &state).unwrap();
    let owned = state.owned().unwrap();
    assert_eq!(owned.len(), 2);
    assert!(owned.iter().all(|w| w.resource_id != "former"));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(apply_writes(&plan, &state).unwrap().ok());
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("# local comment") && text.contains("kept"));
}

#[test]
fn handoff_validation_failure_rolls_back_all_ownership_changes() {
    let repo = Repo::new();
    let home = repo.temp.path().join("home");
    let path = home.join(".agents/skills/review/SKILL.md");
    let state = repo.state();
    let original = sm::model::ApplyPlan {
        writes: vec![OwnedWrite::new(
            "former",
            &path,
            WriteBody::File {
                bytes: b"Original".to_vec(),
                executable: false,
            },
        )],
        ..Default::default()
    };
    assert!(apply_writes(&original, &state).unwrap().ok());
    let before = state.owned().unwrap();
    let catalog = Catalog::load(&repo.work).unwrap();
    let plan = plan_apply(
        &catalog,
        &BTreeMap::from([("review".into(), repo.work.clone())]),
        &home,
    );
    fs::write(repo.work.join("resources/adoption.json"),json!([{"resource":"review","previous_resource":"former","target":".agents/skills/review/SKILL.md","sha256":fingerprint(b"Original"),"executable":false},{"resource":"review","target":".agents/skills/review/other","sha256":"invalid"}]).to_string()).unwrap();
    assert!(adopt(&repo.work, &catalog, &home, &plan, &state).is_err());
    assert_eq!(state.owned().unwrap(), before);
    assert_eq!(fs::read(&path).unwrap(), b"Original");
}

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
    repo.catalog(json!({"projects":{},"upstreams":{"offline":{"git":"https://example.invalid/offline"}},"resources":[{"id":"review","kind":"skill","origin":"upstream","scope":"user","project":null,"payload":".","upstream":"offline"}]}));
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
