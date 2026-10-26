use sm::{
    apply::{apply_writes, plan_apply},
    model::{ApplyStatus, Catalog},
    state::State,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    process::Command,
};

#[test]
fn source_generation_rejects_escaping_or_pattern_subpaths_before_fetching() {
    let temp = tempfile::tempdir().unwrap();
    let selection = temp.path().join("selection.json");
    let output = temp.path().join("output");
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let sentinel = temp.path().join("fetch-invoked");
    for program in ["git", "npx"] {
        let path = bin.join(program);
        fs::write(
            &path,
            "#!/bin/sh\n: > \"$SM_TEST_FETCH_SENTINEL\"\nexit 99\n",
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    for (subpath, expected) in [
        ("../outside", "SM: invalid content path: ../outside"),
        ("/outside", "SM: invalid content path: /outside"),
        ("", "SM: invalid content path:"),
        ("skills/*", "SM: invalid upstream Skill subpath"),
        ("skills\nother", "SM: invalid upstream Skill subpath"),
    ] {
        fs::write(
            &selection,
            serde_json::to_vec(&serde_json::json!({
                "source": "https://example.invalid/source.git",
                "skills": ["demo"],
                "subpath": subpath,
            }))
            .unwrap(),
        )
        .unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_sm"))
            .arg("source-skills")
            .arg(&selection)
            .arg(&output)
            .env("PATH", &bin)
            .env("SM_TEST_FETCH_SENTINEL", &sentinel)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{result:?}");
        assert_eq!(String::from_utf8(result.stderr).unwrap().trim(), expected);
        assert!(!sentinel.exists(), "invalid path invoked a fetch command");
        assert!(!output.exists());
    }
}
#[test]
fn source_generation_rejects_nonempty_or_symlink_outputs_before_invoking_installer() {
    let temp = tempfile::tempdir().unwrap();
    let selection = temp.path().join("selection.json");
    fs::write(
        &selection,
        r#"{"source":"https://example.invalid/source","skills":["sample"]}"#,
    )
    .unwrap();
    let output = temp.path().join("existing");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("private"), "unchanged").unwrap();
    assert!(
        sm::skill_source::generate(&selection, &output)
            .unwrap_err()
            .to_string()
            .contains("empty directory")
    );
    let alias = temp.path().join("alias");
    symlink(&output, &alias).unwrap();
    assert!(
        sm::skill_source::generate(&selection, &alias)
            .unwrap_err()
            .to_string()
            .contains("symlink")
    );
    assert_eq!(
        fs::read_to_string(output.join("private")).unwrap(),
        "unchanged"
    );
}
#[test]
fn native_selection_copies_only_requested_skills_and_preserves_bytes_and_modes() {
    let root = tempfile::tempdir().unwrap();
    let selected = root.path().join(".agents/skills/demo");
    fs::create_dir_all(&selected).unwrap();
    fs::write(selected.join("SKILL.md"), "Skill").unwrap();
    fs::write(selected.join("run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(selected.join("run.sh"), fs::Permissions::from_mode(0o750)).unwrap();
    fs::create_dir_all(root.path().join(".agents/skills/extra")).unwrap();
    fs::write(root.path().join(".agents/skills/extra/SKILL.md"), "extra").unwrap();
    let output = root.path().join("output");
    sm::skill_source::materialize_selected(root.path(), &output, &["demo".into()]).unwrap();
    assert!(!output.join("bundle.json").exists());
    assert!(!output.join(".agents/skills/extra").exists());
    assert!(!output.join(".agents/skills/removed").exists());
    assert_eq!(
        fs::read(output.join(".agents/skills/demo/SKILL.md")).unwrap(),
        b"Skill"
    );
    assert_eq!(
        fs::read(output.join(".agents/skills/demo/run.sh")).unwrap(),
        b"#!/bin/sh\nexit 0\n"
    );
    assert_ne!(
        fs::metadata(output.join(".agents/skills/demo/run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
}

#[test]
fn missing_selected_skill_does_not_block_other_members_of_shared_source() {
    let root = tempfile::tempdir().unwrap();
    let selected = root.path().join(".agents/skills/demo");
    fs::create_dir_all(&selected).unwrap();
    fs::write(selected.join("SKILL.md"), "Skill").unwrap();
    let output = root.path().join("output");
    sm::skill_source::materialize_selected(
        root.path(),
        &output,
        &["demo".into(), "removed".into()],
    )
    .unwrap();
    let catalog: Catalog = serde_json::from_value(serde_json::json!({
        "projects": {},
        "upstreams": {"shared": {"git": "https://example.invalid/source.git"}},
        "resources": (["demo", "removed"].map(|id| serde_json::json!({
            "id": id, "kind": "skill", "origin": "upstream", "scope": "user",
            "upstream": "shared", "payload": format!(".agents/skills/{id}")
        })))
    }))
    .unwrap();
    let home = root.path().join("home");
    let sources = BTreeMap::from([("demo".into(), output.clone()), ("removed".into(), output)]);
    let plan = plan_apply(&catalog, &sources, &home);
    let state = State::open(root.path().join("state.sqlite3")).unwrap();
    let report = apply_writes(&plan, &state).unwrap();
    assert_eq!(report.results["demo"].status, ApplyStatus::Applied);
    assert_eq!(report.results["removed"].status, ApplyStatus::Failed);
    assert!(!report.ok());
    assert_eq!(
        fs::read(home.join(".agents/skills/demo/SKILL.md")).unwrap(),
        b"Skill"
    );
    assert!(!home.join(".agents/skills/removed").exists());
}

#[test]
fn native_selection_rejects_escaping_symlink_or_malformed_members_before_copying() {
    let root = tempfile::tempdir().unwrap();
    let selected = root.path().join(".agents/skills/demo");
    fs::create_dir_all(&selected).unwrap();
    fs::write(selected.join("SKILL.md"), "Skill").unwrap();
    let output = root.path().join("output");
    assert!(
        sm::skill_source::materialize_selected(root.path(), &output, &["../escape".into()])
            .is_err()
    );
    symlink("SKILL.md", selected.join("link")).unwrap();
    assert!(
        sm::skill_source::materialize_selected(root.path(), &output, &["demo".into()]).is_err()
    );
    assert!(!output.exists());
    fs::remove_file(selected.join("link")).unwrap();
    fs::remove_file(selected.join("SKILL.md")).unwrap();
    assert!(
        sm::skill_source::materialize_selected(root.path(), &output, &["demo".into()]).is_err()
    );
    assert!(!output.exists());
}
