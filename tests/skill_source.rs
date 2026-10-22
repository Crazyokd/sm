use std::{fs, os::unix::fs::symlink};
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
fn native_installed_selection_projects_original_names_and_rejects_missing_or_escaping_payloads() {
    let root = tempfile::tempdir().unwrap();
    let selected = root.path().join(".agents/skills/demo");
    fs::create_dir_all(&selected).unwrap();
    fs::write(selected.join("SKILL.md"), "Skill").unwrap();
    fs::create_dir_all(root.path().join(".agents/skills/extra")).unwrap();
    fs::write(root.path().join(".agents/skills/extra/SKILL.md"), "extra").unwrap();
    sm::skill_source::manifest(root.path(), &["demo".into()]).unwrap();
    let manifest = fs::read_to_string(root.path().join("bundle.json")).unwrap();
    assert!(manifest.contains(".agents/skills/demo/SKILL.md") && !manifest.contains("extra"));
    assert!(sm::skill_source::manifest(root.path(), &["missing".into()]).is_err());
    assert!(sm::skill_source::manifest(root.path(), &["../escape".into()]).is_err());
    symlink("SKILL.md", selected.join("link")).unwrap();
    assert!(sm::skill_source::manifest(root.path(), &["demo".into()]).is_err());
}
