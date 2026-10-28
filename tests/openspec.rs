use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    process::Command,
};

#[test]
fn openspec_native_generation_is_isolated_and_exports_only_selected_skills() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let npx = bin.join("npx");
    fs::write(&npx, r#"#!/bin/sh
set -eu
[ "$1" = --yes ]
[ "$2" = @fission-ai/openspec@latest ]
[ "$3" = init ]
[ "$4" = --tools ]
[ "$5" = agents ]
[ "$6" = --profile ]
[ "$7" = core ]
[ "$8" = --force ]
[ "$SM_INTERNAL_RUN" = 1 ]
[ "$OPENSPEC_TELEMETRY" = 0 ]
[ "$XDG_CONFIG_HOME" = "$PWD/config" ]
[ "$XDG_DATA_HOME" = "$PWD/data" ]
[ "$CODEX_HOME" = "$PWD/codex" ]
printf '%s' "$PWD" > "$SM_TEST_RECORD"
for name in openspec-apply-change openspec-archive-change openspec-explore openspec-propose extra; do
  mkdir -p ".agents/skills/$name/references"
  printf '%s' "$name" > ".agents/skills/$name/SKILL.md"
  printf 'upstream support\n' > ".agents/skills/$name/references/guide.md"
done
mkdir -p "$CODEX_HOME/prompts" openspec
printf 'temporary' > "$CODEX_HOME/prompts/opsx-apply.md"
printf 'temporary' > openspec/config.yaml
"#).unwrap();
    fs::set_permissions(&npx, fs::Permissions::from_mode(0o700)).unwrap();
    let output = temp.path().join("output with spaces");
    let record = temp.path().join("record");
    let result = Command::new(env!("CARGO_BIN_EXE_sm"))
        .arg("source-openspec")
        .arg(&output)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("SM_TEST_RECORD", &record)
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let staging = fs::read_to_string(record).unwrap();
    assert!(!std::path::Path::new(&staging).exists());
    for name in [
        "openspec-apply-change",
        "openspec-archive-change",
        "openspec-explore",
        "openspec-propose",
    ] {
        let skill = output.join(".agents/skills").join(name);
        assert_eq!(fs::read_to_string(skill.join("SKILL.md")).unwrap(), name);
        assert_eq!(
            fs::read_to_string(skill.join("references/guide.md")).unwrap(),
            "upstream support\n"
        );
    }
    for path in [
        ".agents/skills/extra",
        "codex",
        "openspec",
        "config",
        "data",
    ] {
        assert!(!output.join(path).exists(), "{path}");
    }
}

#[test]
fn openspec_rejects_existing_or_symlink_output_before_running_installer() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("output");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("local"), "keep").unwrap();
    let alias = temp.path().join("alias");
    symlink(&output, &alias).unwrap();
    for (path, message) in [(&output, "empty directory"), (&alias, "symlink")] {
        let result = Command::new(env!("CARGO_BIN_EXE_sm"))
            .arg("source-openspec")
            .arg(path)
            .env("PATH", "/nonexistent")
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8(result.stderr).unwrap().contains(message));
    }
    assert_eq!(fs::read_to_string(output.join("local")).unwrap(), "keep");
}
