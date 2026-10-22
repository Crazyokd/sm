use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
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
fn native_selection_copies_only_existing_requested_skills_and_preserves_bytes_and_modes() {
    let root = tempfile::tempdir().unwrap();
    let selected = root.path().join(".agents/skills/demo");
    fs::create_dir_all(&selected).unwrap();
    fs::write(selected.join("SKILL.md"), "Skill").unwrap();
    fs::write(selected.join("run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(selected.join("run.sh"), fs::Permissions::from_mode(0o750)).unwrap();
    fs::create_dir_all(root.path().join(".agents/skills/extra")).unwrap();
    fs::write(root.path().join(".agents/skills/extra/SKILL.md"), "extra").unwrap();
    let output = root.path().join("output");
    sm::skill_source::materialize_selected(
        root.path(),
        &output,
        &["demo".into(), "removed".into()],
    )
    .unwrap();
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
    let missing = root.path().join("missing-output");
    sm::skill_source::materialize_selected(root.path(), &missing, &["removed".into()]).unwrap();
    assert!(fs::read_dir(missing).unwrap().next().is_none());
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
