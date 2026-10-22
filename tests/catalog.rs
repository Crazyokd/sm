use sm::{
    apply::{apply_writes, plan_apply},
    model::{ApplyStatus, Catalog},
    state::State,
};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

#[test]
fn shipped_self_resources_apply_to_declared_scopes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut catalog = Catalog::load(root).unwrap();
    catalog.resources.retain(|r| r.source.is_self());
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    for (index, directory) in catalog.projects.values_mut().enumerate() {
        *directory = format!("~/project-{index}");
    }
    for resource in &catalog.resources {
        fs::create_dir_all(catalog.scope_root(resource, &home).unwrap()).unwrap();
    }
    let sources: BTreeMap<_, _> = catalog
        .resources
        .iter()
        .map(|r| (r.id.clone(), root.to_path_buf()))
        .collect();
    let state = State::open(temp.path().join("state.sqlite3")).unwrap();
    let report = apply_writes(&plan_apply(&catalog, &sources, &home), &state).unwrap();
    assert!(report.ok(), "{report:?}");
    for resource in &catalog.resources {
        assert_eq!(report.results[&resource.id].status, ApplyStatus::Applied);
        if resource.content.kind() != sm::model::ResourceKind::Skill {
            continue;
        }
        let skill = catalog
            .scope_root(resource, &home)
            .unwrap()
            .join(".agents/skills")
            .join(&resource.id)
            .join("SKILL.md");
        assert!(fs::read_to_string(skill).unwrap().starts_with("---\n"));
    }
}

#[test]
fn upstream_installer_uses_only_the_supplied_staging_project() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let npx = bin.join("npx");
    fs::write(
        &npx,
        "#!/bin/sh\nset -eu\n[ \"$DISABLE_TELEMETRY\" = 1 ]\n[ \"$DO_NOT_TRACK\" = 1 ]\nprintf '%s\\n' \"$PWD\" \"$@\" > invocation\n",
    )
    .unwrap();
    fs::set_permissions(&npx, fs::Permissions::from_mode(0o700)).unwrap();
    let output = temp.path().join("staging with spaces");
    let result = Command::new("bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/source-providers/skills.sh"))
        .args(["https://example.invalid", "demo"])
        .arg(&output)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        fs::read_to_string(output.join("invocation")).unwrap(),
        format!(
            "{}\n--yes\nskills@1.7.2\nadd\nhttps://example.invalid\n--skill\ndemo\n--agent\ncodex\n--yes\n--copy\n",
            output.display()
        )
    );
}
