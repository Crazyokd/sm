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
fn reviewed_shared_support_handoff_requires_a_named_consumer_and_keeps_last_owner() {
    for (shared_with, expected_owners) in [("alpha", 2), ("unrelated", 1)] {
        let repo = Repo::new();
        let home = repo.temp.path().join("home");
        let path = home.join(".codex/hooks/shared.py");
        let state = repo.state();
        let file = |id: &str| {
            OwnedWrite::new(
                id,
                &path,
                WriteBody::File {
                    bytes: b"print('shared')".to_vec(),
                    executable: true,
                },
            )
        };
        assert!(
            apply_writes(
                &sm::model::ApplyPlan {
                    writes: vec![file("former-script")],
                    ..Default::default()
                },
                &state
            )
            .unwrap()
            .ok()
        );
        let resources: Vec<_> = ["alpha", "beta"].into_iter().map(|id| json!({"id":id,"kind":"hook","origin":"self","scope":"user","project":null,"dependencies":["runtime"],"files":{"root":format!("resources/{id}"),"entries":[{"target":".codex/hooks.json","json":{"hooks":{"Stop":[{"hooks":[{"type":"command","command":id}]}]}}}]}})).collect();
        repo.catalog(json!({"projects":{},"upstreams":{"remote":{"git":"https://example.invalid/runtime"}},"dependencies":{"runtime":{"origin":"upstream","upstream":"remote","files":{"root":".","entries":[{"target":".codex/hooks/shared.py","source":"shared.py"}]}}},"resources":resources}));
        let catalog = Catalog::load(&repo.work).unwrap();
        let manifest = json!([
            {"resource":"alpha","previous_resource":"former-script","target":".codex/hooks/shared.py","sha256":fingerprint(b"print('shared')"),"executable":true},
            {"resource":"beta","shared_with":shared_with,"target":".codex/hooks/shared.py","sha256":fingerprint(b"print('shared')"),"executable":true}
        ]);
        fs::write(
            repo.work.join("resources/adoption.json"),
            manifest.to_string(),
        )
        .unwrap();
        adopt(&repo.work, &catalog, &home, &Default::default(), &state).unwrap();
        let owned = state.owned().unwrap();
        assert_eq!(owned.len(), expected_owners);
        assert!(
            owned
                .iter()
                .all(|write| write.resource_id != "former-script")
        );
        assert_eq!(fs::read(&path).unwrap(), b"print('shared')");
        if expected_owners == 2 {
            assert!(
                apply_writes(
                    &sm::model::ApplyPlan {
                        writes: vec![file("alpha"), file("beta")],
                        ..Default::default()
                    },
                    &state
                )
                .unwrap()
                .ok()
            );
            assert!(
                apply_writes(
                    &sm::model::ApplyPlan {
                        writes: vec![file("beta")],
                        ..Default::default()
                    },
                    &state
                )
                .unwrap()
                .ok()
            );
            assert!(path.exists());
            assert_eq!(state.owned().unwrap()[0].resource_id, "beta");
            assert!(apply_writes(&Default::default(), &state).unwrap().ok());
            assert!(!path.exists());
        }
    }
}

#[test]
fn reviewed_config_consolidation_requires_the_exact_union_of_named_owners() {
    for (prior, expected) in [
        (vec!["first", "second"], true),
        (vec!["first", "missing"], false),
        (vec!["first"], false),
    ] {
        let repo = Repo::new();
        let home = repo.temp.path().join("home");
        fs::create_dir_all(home.join(".codex")).unwrap();
        let path = home.join(".codex/hooks.json");
        fs::write(&path, "{\"local\":true}").unwrap();
        let state = repo.state();
        let first =
            json!({"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"recall"}]}]}})
                .as_object()
                .unwrap()
                .clone();
        let second = json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":"retain"}]}]}})
            .as_object()
            .unwrap()
            .clone();
        assert!(
            apply_writes(
                &sm::model::ApplyPlan {
                    writes: vec![
                        OwnedWrite::new("first", &path, WriteBody::Json(first.clone())),
                        OwnedWrite::new("second", &path, WriteBody::Json(second.clone()))
                    ],
                    ..Default::default()
                },
                &state
            )
            .unwrap()
            .ok()
        );
        let before = fs::read(&path).unwrap();
        let mut combined = json!({});
        sm::apply::merge_entries(&mut combined, &first, false).unwrap();
        sm::apply::merge_entries(&mut combined, &second, false).unwrap();
        repo.catalog(json!({"projects":{},"resources":[{"id":"memory","kind":"hook","origin":"self","scope":"user","project":null,"files":{"root":"resources/memory","entries":[{"target":".codex/hooks.json","json":combined}]}}]}));
        let catalog = Catalog::load(&repo.work).unwrap();
        let plan = plan_apply(
            &catalog,
            &BTreeMap::from([("memory".into(), repo.work.clone())]),
            &home,
        );
        fs::write(repo.work.join("resources/adoption.json"), json!([{"resource":"memory","previous_resources":prior,"target":".codex/hooks.json","sha256":fingerprint(&serde_json::to_vec(combined.as_object().unwrap()).unwrap())}]).to_string()).unwrap();
        adopt(&repo.work, &catalog, &home, &plan, &state).unwrap();
        let owned = state.owned().unwrap();
        assert_eq!(owned.len(), if expected { 1 } else { 2 });
        assert_eq!(fs::read(&path).unwrap(), before);
        if expected {
            assert_eq!(owned[0].resource_id, "memory");
            assert!(apply_writes(&plan, &state).unwrap().ok());
        }
    }
}

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
fn reviewed_missing_file_restore_requires_the_exact_old_ownership() {
    let repo = Repo::new();
    let home = repo.temp.path().join("home");
    let target = home.join(".agents/skills/review/SKILL.md");
    let state = repo.state();
    let catalog = Catalog::load(&repo.work).unwrap();
    let plan = plan_apply(
        &catalog,
        &BTreeMap::from([("review".into(), repo.work.clone())]),
        &home,
    );
    assert!(apply_writes(&plan, &state).unwrap().ok());
    fs::remove_file(&target).unwrap();
    for digest in [fingerprint(b"Wrong old content"), fingerprint(b"Approved")] {
        fs::write(repo.work.join("resources/adoption.json"),json!([{"resource":"review","previous_resource":"review","target":".agents/skills/review/SKILL.md","sha256":digest,"executable":false,"restore_missing":true}]).to_string()).unwrap();
        adopt(&repo.work, &catalog, &home, &plan, &state).unwrap();
        let report = apply_writes(&plan, &state).unwrap();
        assert_eq!(report.ok(), digest == fingerprint(b"Approved"));
    }
    assert_eq!(fs::read_to_string(&target).unwrap(), "Approved");
}

#[test]
fn reviewed_same_owner_reconciliation_accepts_only_the_new_exact_fingerprint_once() {
    let repo = Repo::new();
    let home = repo.temp.path().join("home");
    let target = home.join(".agents/skills/review/SKILL.md");
    let state = repo.state();
    let catalog = Catalog::load(&repo.work).unwrap();
    let plan = plan_apply(
        &catalog,
        &BTreeMap::from([("review".into(), repo.work.clone())]),
        &home,
    );
    assert!(apply_writes(&plan, &state).unwrap().ok());
    fs::write(&target, "Reviewed local edit").unwrap();
    fs::write(
        repo.work.join("resources/review/SKILL.md"),
        "Reviewed local edit",
    )
    .unwrap();
    fs::write(repo.work.join("resources/adoption.json"),json!([{"resource":"review","previous_resource":"review","target":".agents/skills/review/SKILL.md","sha256":fingerprint(b"Reviewed local edit"),"executable":false}]).to_string()).unwrap();
    let plan = plan_apply(
        &catalog,
        &BTreeMap::from([("review".into(), repo.work.clone())]),
        &home,
    );
    adopt(&repo.work, &catalog, &home, &plan, &state).unwrap();
    assert!(apply_writes(&plan, &state).unwrap().ok());
    assert_eq!(fs::read_to_string(&target).unwrap(), "Reviewed local edit");
    fs::write(&target, "Later unreviewed edit").unwrap();
    adopt(&repo.work, &catalog, &home, &plan, &state).unwrap();
    assert!(!apply_writes(&plan, &state).unwrap().ok());
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "Later unreviewed edit"
    );
    fs::write(&target, "Reviewed local edit").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(
        repo.work.join("resources/review/SKILL.md"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::write(repo.work.join("resources/adoption.json"),json!([{"resource":"review","previous_resource":"review","target":".agents/skills/review/SKILL.md","sha256":fingerprint(b"Reviewed local edit"),"executable":true}]).to_string()).unwrap();
    let plan = plan_apply(
        &catalog,
        &BTreeMap::from([("review".into(), repo.work.clone())]),
        &home,
    );
    adopt(&repo.work, &catalog, &home, &plan, &state).unwrap();
    assert!(apply_writes(&plan, &state).unwrap().ok());
    fs::remove_file(&target).unwrap();
    fs::write(repo.work.join("resources/adoption.json"),json!([{"resource":"review","previous_resource":"review","target":".agents/skills/review/SKILL.md","sha256":fingerprint(b"Reviewed local edit"),"executable":true,"restore_missing":true}]).to_string()).unwrap();
    adopt(&repo.work, &catalog, &home, &plan, &state).unwrap();
    assert!(apply_writes(&plan, &state).unwrap().ok());
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
