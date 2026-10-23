use sm::{
    apply::{apply_writes, plan_apply},
    model::{ApplyStatus, Catalog, ProjectionEntry, ResourceContent},
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
    // Local bindings stay local; tests supply fixture values rather than reading the real home.
    fn bind_fixture(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(text) if text == "${SM_LOCAL}" => {
                *text = "fixture-local".into()
            }
            serde_json::Value::Object(values) => values.values_mut().for_each(bind_fixture),
            serde_json::Value::Array(values) => values.iter_mut().for_each(bind_fixture),
            _ => {}
        }
    }
    let mut documents = BTreeMap::new();
    for resource in &catalog.resources {
        if let ResourceContent::Files { files, .. } = &resource.content {
            for entry in &files.entries {
                let (relative, fragment, toml) = match entry {
                    ProjectionEntry::Json { target, json } => (target, json.clone(), false),
                    ProjectionEntry::Toml { target, toml } => (target, toml.clone(), true),
                    ProjectionEntry::Fragment { target, fragment } => (
                        target,
                        serde_json::from_slice(
                            &fs::read(root.join(&files.root).join(fragment)).unwrap(),
                        )
                        .unwrap(),
                        target
                            .extension()
                            .is_some_and(|extension| extension == "toml"),
                    ),
                    _ => continue,
                };
                let target = catalog.scope_root(resource, &home).unwrap().join(relative);
                let (document, _) = documents
                    .entry(target)
                    .or_insert((serde_json::json!({}), toml));
                let mut values = serde_json::Value::Object(fragment);
                bind_fixture(&mut values);
                sm::apply::merge_entries(document, values.as_object().unwrap(), false).unwrap();
            }
        }
    }
    for (path, (document, toml)) in documents {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text = if toml {
            toml_edit::ser::to_string_pretty(&document).unwrap()
        } else {
            serde_json::to_string_pretty(&document).unwrap()
        };
        fs::write(path, text).unwrap();
    }
    let mut sources: BTreeMap<_, _> = catalog
        .resources
        .iter()
        .map(|r| (r.id.clone(), root.to_path_buf()))
        .collect();
    for (name, dependency) in &catalog.dependencies {
        let source = if dependency.source.is_self() {
            root.to_path_buf()
        } else {
            let source = temp.path().join("dependency-sources").join(name);
            for entry in &dependency.files.entries {
                let ProjectionEntry::File { source: file, .. } = entry else {
                    panic!("support file required")
                };
                let target = source.join(&dependency.files.root).join(file);
                fs::create_dir_all(target.parent().unwrap()).unwrap();
                fs::write(target, "fixture supporting runtime").unwrap();
            }
            source
        };
        sources.insert(format!("@dependency/{name}"), source);
    }
    let state = State::open(temp.path().join("state.sqlite3")).unwrap();
    let plan = plan_apply(&catalog, &sources, &home);
    let mut initial = BTreeMap::<_, Vec<_>>::new();
    for write in &plan.writes {
        if matches!(
            write.body,
            sm::model::WriteBody::Json(_) | sm::model::WriteBody::Toml(_)
        ) {
            initial
                .entry(write.path.clone())
                .or_default()
                .push(write.clone());
        }
    }
    for (path, writes) in initial {
        state.replace_owned(&path, &writes).unwrap();
    }
    let report = apply_writes(&plan, &state).unwrap();
    assert!(report.ok(), "{report:?}");
    for resource in &catalog.resources {
        let expected = if matches!(resource.content, ResourceContent::Installation { .. }) {
            ApplyStatus::Pending
        } else {
            ApplyStatus::Applied
        };
        assert_eq!(
            report.results[&resource.id].status, expected,
            "{}",
            resource.id
        );
    }
    for write in &plan.writes {
        if let sm::model::WriteBody::File { bytes, .. } = &write.body {
            assert_eq!(
                fs::read(&write.path).unwrap(),
                *bytes,
                "{}",
                write.resource_id
            );
        }
    }
}

#[test]
fn upstream_installer_uses_disposable_staging_and_materializes_only_selected_skills() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let npx = bin.join("npx");
    fs::write(
        &npx,
        "#!/bin/sh\nset -eu\n[ \"$DISABLE_TELEMETRY\" = 1 ]\n[ \"$DO_NOT_TRACK\" = 1 ]\n[ \"$SM_INTERNAL_RUN\" = 1 ]\nprintf '%s\\n' \"$PWD\" \"$@\" > \"$SM_TEST_RECORD\"\nmkdir -p .agents/skills/demo .agents/skills/extra\nprintf 'Selected Skill' > .agents/skills/demo/SKILL.md\nprintf 'Unselected' > .agents/skills/extra/SKILL.md\n",
    ).unwrap();
    fs::set_permissions(&npx, fs::Permissions::from_mode(0o700)).unwrap();
    let selection = temp.path().join("selection.json");
    fs::write(
        &selection,
        r#"{"source":"https://example.invalid","skills":["demo"]}"#,
    )
    .unwrap();
    let output = temp.path().join("staging with spaces");
    let record = temp.path().join("invocation");
    let result = Command::new(env!("CARGO_BIN_EXE_sm"))
        .arg("source-skills")
        .arg(&selection)
        .arg(&output)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("SM_TEST_RECORD", &record)
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let invocation = fs::read_to_string(record).unwrap();
    let (staging, arguments) = invocation.split_once('\n').unwrap();
    assert_ne!(Path::new(staging), output);
    assert!(
        !Path::new(staging).exists(),
        "disposable native staging must be cleaned"
    );
    assert_eq!(
        arguments,
        "--yes\nskills@1.7.2\nadd\nhttps://example.invalid\n--skill\n*\n--agent\ncodex\n--yes\n--copy\n"
    );
    assert_eq!(
        fs::read_to_string(output.join(".agents/skills/demo/SKILL.md")).unwrap(),
        "Selected Skill"
    );
    assert!(!output.join(".agents/skills/extra").exists());
}
