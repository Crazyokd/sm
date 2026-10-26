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

#[test]
fn upstream_subpath_installs_the_fresh_complete_tree_and_rejects_symlink_selection() {
    let temp = tempfile::tempdir().unwrap();
    let upstream = temp.path().join("upstream");
    let skill = upstream.join("skills/demo");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "Selected Skill").unwrap();
    fs::write(skill.join(".data"), [0, 255, 1]).unwrap();
    fs::write(skill.join("run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(skill.join("run.sh"), fs::Permissions::from_mode(0o750)).unwrap();
    fs::write(upstream.join("unrelated"), "must not be fetched").unwrap();
    std::os::unix::fs::symlink("demo", upstream.join("skills/alias")).unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("private"), "external repository worktree").unwrap();
    for root in [&upstream, &outside] {
        for args in [
            vec!["init", "-q"],
            vec!["config", "uploadpack.allowFilter", "true"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
        ] {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(root)
                    .status()
                    .unwrap()
                    .success()
            );
        }
    }
    let snapshot = || {
        walkdir::WalkDir::new(&outside)
            .into_iter()
            .map(Result::unwrap)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| {
                (
                    entry.path().strip_prefix(&outside).unwrap().to_path_buf(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let outside_before = snapshot();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::write(
        bin.join("git"),
        "#!/bin/bash\nset -eu\nargs=(\"$@\")\nfor i in \"${!args[@]}\"; do\n  if [ \"${args[$i]}\" = https://example.invalid/source.git ]; then args[$i]=\"$SM_TEST_UPSTREAM\"; fi\ndone\nexec /usr/bin/git \"${args[@]}\"\n",
    ).unwrap();
    fs::write(
        bin.join("npx"),
        "#!/bin/sh\nset -eu\n[ -f \"$4/SKILL.md\" ]\n[ ! -e \"$4/../../unrelated\" ]\n[ -z \"${GIT_DIR+x}\" ]\n[ -z \"${GIT_COMMON_DIR+x}\" ]\n[ -z \"${GIT_WORK_TREE+x}\" ]\n[ -z \"${GIT_INDEX_FILE+x}\" ]\ngit -C \"$4\" config --local sm.native-install true\nprintf '%s' \"$4\" > \"$SM_TEST_RECORD\"\nmkdir -p .agents/skills\ncp -a \"$4\" .agents/skills/demo\n",
    ).unwrap();
    for program in ["git", "npx"] {
        fs::set_permissions(bin.join(program), fs::Permissions::from_mode(0o700)).unwrap();
    }
    let selection = temp.path().join("selection.json");
    let record = temp.path().join("native-source");
    let routing = BTreeMap::from([
        ("GIT_DIR", outside.join(".git")),
        ("GIT_COMMON_DIR", outside.join(".git")),
        ("GIT_WORK_TREE", outside.clone()),
        ("GIT_INDEX_FILE", outside.join(".git/index")),
    ]);
    let all_routing = [
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
    ];
    for (index, (subpath, variables)) in [
        ("skills/demo", &["GIT_DIR"][..]),
        ("skills/demo", &all_routing[..]),
        ("skills/alias", &all_routing[..]),
    ]
    .into_iter()
    .enumerate()
    {
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
        let output = temp.path().join(format!("output-{index}"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_sm"));
        command
            .args(["source-skills"])
            .arg(&selection)
            .arg(&output)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("SM_TEST_UPSTREAM", format!("file://{}", upstream.display()))
            .env("SM_TEST_RECORD", &record);
        for variable in variables {
            command.env(variable, &routing[variable]);
        }
        let result = command.output().unwrap();
        let outside_after = snapshot();
        let changed: Vec<_> = outside_after
            .keys()
            .chain(outside_before.keys())
            .filter(|path| outside_after.get(*path) != outside_before.get(*path))
            .collect();
        assert!(
            changed.is_empty(),
            "source fetch changed external Git state: {changed:?}"
        );
        if subpath == "skills/alias" {
            assert!(!result.status.success(), "{result:?}");
            assert!(!output.exists());
            assert!(
                !record.exists(),
                "invalid selection reached native installer"
            );
            continue;
        }
        assert!(result.status.success(), "{result:?}");
        let native = fs::read_to_string(&record).unwrap();
        assert_ne!(Path::new(&native), skill);
        assert!(
            !Path::new(&native).exists(),
            "fresh staging was not cleaned"
        );
        for file in ["SKILL.md", ".data", "run.sh"] {
            assert_eq!(
                fs::read(output.join(".agents/skills/demo").join(file)).unwrap(),
                fs::read(skill.join(file)).unwrap()
            );
        }
        assert_ne!(
            fs::metadata(output.join(".agents/skills/demo/run.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
        fs::remove_file(&record).unwrap();
    }
}
