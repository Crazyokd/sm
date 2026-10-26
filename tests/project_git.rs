mod common;

use common::{Repo, git};
use serde_json::json;
use sm::{
    apply::{apply_writes, plan_apply},
    model::{Catalog, OwnedWrite, WriteBody},
    service::{Runtime, apply_source},
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
};

fn project(repo: &Repo) -> PathBuf {
    let project = repo.temp.path().join("project");
    initialize_project(project)
}

fn initialize_project(project: PathBuf) -> PathBuf {
    fs::create_dir_all(&project).unwrap();
    git(&project, &["init", "-b", "main"]);
    git(&project, &["config", "user.name", "SM Test"]);
    git(&project, &["config", "user.email", "sm@example.invalid"]);
    fs::write(project.join("README.md"), "Tracked project\n").unwrap();
    git(&project, &["add", "README.md"]);
    git(&project, &["commit", "-m", "Initial project"]);
    project
}

#[test]
fn outer_scope_protects_and_retires_a_file_inside_an_independent_child_repository() {
    let repo = Repo::new();
    let outer = project(&repo);
    fs::create_dir(outer.join("src")).unwrap();
    let child = initialize_project(outer.join("src/child"));
    let runtime = configure(&repo, &outer);
    let mut catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.work.join("catalog.json")).unwrap()).unwrap();
    catalog["resources"][1]["files"]["entries"][0]["target"] = json!("src/child/AGENTS.md");
    repo.catalog(catalog);
    let state = runtime.state().unwrap();
    let exclude = child.join(".git/info/exclude");
    let original = fs::read(&exclude).unwrap();
    assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
    git(&child, &["stash", "push", "-u"]);
    assert!(
        child.join("AGENTS.md").is_file(),
        "the inner repository owns its stash behavior"
    );
    assert!(
        !fs::read_to_string(outer.join(".git/info/exclude"))
            .unwrap()
            .contains("/src/child/AGENTS.md")
    );

    repo.catalog(json!({"projects":{},"resources":[]}));
    assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
    assert_eq!(fs::read(exclude).unwrap(), original);
    assert!(
        state
            .get::<Vec<serde_json::Value>>("project_git_protections")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn skill_directory_protection_respects_independent_git_boundaries() {
    for boundary in ["", "helpers"] {
        let repo = Repo::new();
        let project = project(&repo);
        let runtime = configure(&repo, &project);
        fs::create_dir_all(repo.work.join("resources/review/helpers")).unwrap();
        fs::write(
            repo.work.join("resources/review/helpers/tool.txt"),
            "Skill helper",
        )
        .unwrap();
        let state = runtime.state().unwrap();
        assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
        let skill = project.join(".agents/skills/review");
        let independent = initialize_project(skill.join(boundary));
        assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
        git(&independent, &["stash", "push", "-u"]);
        assert!(skill.join("SKILL.md").is_file());
        assert_eq!(
            fs::read_to_string(skill.join("helpers/tool.txt")).unwrap(),
            "Skill helper"
        );
        let local_rules = fs::read_to_string(independent.join(".git/info/exclude")).unwrap();
        assert!(!local_rules.contains("\n/\n"));
        if boundary.is_empty() {
            assert!(local_rules.contains("/SKILL.md\n"));
        } else {
            assert!(local_rules.contains("/tool.txt\n"));
        }
    }
}

fn configure(repo: &Repo, project: &std::path::Path) -> Runtime {
    fs::create_dir_all(repo.work.join("resources/rule/files")).unwrap();
    fs::write(
        repo.work.join("resources/rule/files/AGENTS.md"),
        "Project rule\n",
    )
    .unwrap();
    repo.catalog(json!({"projects":{"project":project},"resources":[
        {"id":"review","kind":"skill","origin":"self","scope":"project","project":"project","payload":"resources/review"},
        {"id":"rule","kind":"rule","origin":"self","scope":"project","project":"project","files":{"root":"resources/rule","entries":[{"target":"AGENTS.md","source":"files/AGENTS.md"}]}}
    ]}));
    Runtime {
        repo: repo.work.clone(),
        home: repo.temp.path().join("home"),
        executable: PathBuf::from(env!("CARGO_BIN_EXE_sm")),
    }
}

#[test]
fn explicit_project_reinclusion_does_not_block_resource_application() {
    for ignore in ["!AGENTS.md\n", "*\n!AGENTS.md\n"] {
        let repo = Repo::new();
        let project = project(&repo);
        let runtime = configure(&repo, &project);
        let state = runtime.state().unwrap();
        fs::write(project.join(".gitignore"), ignore).unwrap();
        let index = fs::read(project.join(".git/index")).unwrap();
        assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
        assert_eq!(
            fs::read_to_string(project.join("AGENTS.md")).unwrap(),
            "Project rule\n"
        );
        assert_eq!(
            fs::read_to_string(project.join(".gitignore")).unwrap(),
            ignore
        );
        assert_eq!(fs::read(project.join(".git/index")).unwrap(), index);
        let warnings: BTreeMap<PathBuf, Vec<PathBuf>> =
            state.get("project_git_protection_warnings").unwrap();
        assert_eq!(warnings[&project], vec![PathBuf::from("AGENTS.md")]);
        fs::write(project.join(".gitignore"), "*\n").unwrap();
        assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
        assert!(
            state
                .get::<BTreeMap<PathBuf, Vec<PathBuf>>>("project_git_protection_warnings")
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn unprotected_managed_skill_is_removed_by_git_stash_u() {
    let repo = Repo::new();
    let project = project(&repo);
    let runtime = configure(&repo, &project);
    let state = runtime.state().unwrap();
    let catalog = Catalog::load(&repo.work).unwrap();
    let sources = BTreeMap::from([
        ("review".into(), repo.work.clone()),
        ("rule".into(), repo.work.clone()),
    ]);
    let report = apply_writes(&plan_apply(&catalog, &sources, &runtime.home), &state).unwrap();
    assert!(report.ok());
    let marker = project.join(".agents/skills/review/SKILL.md");
    assert!(marker.is_file());

    git(&project, &["stash", "push", "-u"]);
    assert!(
        !marker.exists(),
        "reproduce the data-loss boundary before protection"
    );
}

#[test]
fn service_install_protects_only_managed_paths_from_stash_u_and_is_idempotent() {
    let repo = Repo::new();
    let project = project(&repo);
    let runtime = configure(&repo, &project);
    let state = runtime.state().unwrap();
    let exclude = project.join(".git/info/exclude");
    let original = "# Existing local rules\n*.log\n!keep.log";
    fs::write(&exclude, original).unwrap();
    fs::set_permissions(&exclude, fs::Permissions::from_mode(0o640)).unwrap();
    assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
    let protected = fs::read(&exclude).unwrap();
    assert!(protected.starts_with(original.as_bytes()));
    assert_eq!(
        fs::metadata(&exclude).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
    assert_eq!(fs::read(&exclude).unwrap(), protected);
    fs::write(project.join("scratch.txt"), "stash this").unwrap();
    fs::create_dir_all(project.join(".agents/skills/unmanaged")).unwrap();
    fs::write(
        project.join(".agents/skills/unmanaged/SKILL.md"),
        "stash this too",
    )
    .unwrap();
    fs::write(project.join("README.md"), "tracked edit").unwrap();

    git(&project, &["stash", "push", "-u"]);
    assert!(project.join(".agents/skills/review/SKILL.md").is_file());
    assert!(project.join("AGENTS.md").is_file());
    assert!(!project.join("scratch.txt").exists());
    assert!(!project.join(".agents/skills/unmanaged/SKILL.md").exists());
    assert_eq!(
        fs::read_to_string(project.join("README.md")).unwrap(),
        "Tracked project\n"
    );
    assert!(!project.join(".gitignore").exists());

    git(&project, &["add", "-f", "AGENTS.md"]);
    git(&project, &["commit", "-m", "Track project rule"]);
    fs::write(project.join("AGENTS.md"), "Tracked rule edit\n").unwrap();
    assert!(!git(&project, &["diff", "--", "AGENTS.md"]).is_empty());
    git(&project, &["stash", "push", "-u"]);
    assert_eq!(
        fs::read_to_string(project.join("AGENTS.md")).unwrap(),
        "Project rule\n"
    );
}

#[test]
fn non_git_projects_are_installed_without_creating_git_metadata() {
    let repo = Repo::new();
    let project = repo.temp.path().join("plain-project");
    fs::create_dir(&project).unwrap();
    let runtime = configure(&repo, &project);
    assert!(
        apply_source(&repo.work, &runtime, &runtime.state().unwrap())
            .unwrap()
            .1
            .ok()
    );
    assert!(project.join(".agents/skills/review/SKILL.md").is_file());
    assert!(!project.join(".git").exists());
}

#[test]
fn user_resources_inside_a_git_home_are_not_project_exclusions() {
    let repo = Repo::new();
    let project_root = project(&repo);
    let mut runtime = configure(&repo, &project_root);
    let home_repo = Repo::new();
    runtime.home = project(&home_repo);
    let exclude = runtime.home.join(".git/info/exclude");
    let original = fs::read(&exclude).unwrap();
    let mut catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.work.join("catalog.json")).unwrap()).unwrap();
    catalog["resources"].as_array_mut().unwrap().push(json!({"id":"user-skill","kind":"skill","origin":"self","scope":"user","project":null,"payload":"resources/review"}));
    repo.catalog(catalog);
    assert!(
        apply_source(&repo.work, &runtime, &runtime.state().unwrap())
            .unwrap()
            .1
            .ok()
    );
    assert_eq!(fs::read(exclude).unwrap(), original);
    assert_eq!(
        git(
            &runtime.home,
            &[
                "ls-files",
                "--others",
                "--exclude-standard",
                "--",
                ".agents/skills/user-skill/SKILL.md"
            ]
        ),
        ".agents/skills/user-skill/SKILL.md"
    );
}

#[test]
fn unused_catalog_projects_do_not_require_git_or_create_exclusions() {
    let repo = Repo::new();
    let project = project(&repo);
    let runtime = configure(&repo, &project);
    let plain = repo.temp.path().join("plain");
    fs::create_dir(&plain).unwrap();
    let exclude = project.join(".git/info/exclude");
    let before = fs::read(&exclude).unwrap();
    repo.catalog(json!({"projects":{"git":project,"plain":plain,"missing":project.join("missing")},"resources":[]}));
    let state = runtime.state().unwrap();
    assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
    assert_eq!(fs::read(&exclude).unwrap(), before);
    assert!(!plain.join(".git").exists());
    assert!(!project.join("missing").exists());

    let outside = repo.temp.path().join("unused-exclude");
    fs::write(&outside, "unmanaged metadata\n").unwrap();
    fs::remove_file(&exclude).unwrap();
    symlink(&outside, &exclude).unwrap();
    assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
    assert_eq!(fs::read_to_string(outside).unwrap(), "unmanaged metadata\n");
}

#[test]
fn baseline_is_protected_before_fetch_and_new_upstream_install_is_protected_after_fetch() {
    let repo = Repo::new();
    let project = project(&repo);
    let runtime = configure(&repo, &project);
    let script = repo.temp.path().join("fetch-skill");
    fs::write(&script, "#!/bin/sh\nset -eu\ngit -C \"$1\" check-ignore -q .agents/skills/review/SKILL.md\nprintf 'Fetched Skill' > \"$2/SKILL.md\"\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let mut catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.work.join("catalog.json")).unwrap()).unwrap();
    catalog["upstreams"] = json!({"external":{"argv":[script,project]}});
    catalog["resources"].as_array_mut().unwrap().push(json!({"id":"external","kind":"skill","origin":"upstream","upstream":"external","scope":"project","project":"project","payload":"."}));
    repo.catalog(catalog);
    assert!(
        apply_source(&repo.work, &runtime, &runtime.state().unwrap())
            .unwrap()
            .1
            .ok()
    );
    git(&project, &["stash", "push", "-u"]);
    for id in ["review", "external"] {
        assert!(
            project
                .join(format!(".agents/skills/{id}/SKILL.md"))
                .is_file()
        );
    }
}

#[test]
fn worktree_uses_common_exclude_and_escapes_exact_nested_project_paths() {
    let repo = Repo::new();
    let main = project(&repo);
    let worktree = repo.temp.path().join("worktree");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-b",
            "protected",
            worktree.to_str().unwrap(),
        ],
    );
    assert!(worktree.join(".git").is_file());
    let project = worktree.join("nested [x]*? #!\\ folder ");
    fs::create_dir(&project).unwrap();
    let runtime = configure(&repo, &project);
    assert!(
        apply_source(&repo.work, &runtime, &runtime.state().unwrap())
            .unwrap()
            .1
            .ok()
    );
    let exclude = fs::read_to_string(main.join(".git/info/exclude")).unwrap();
    assert!(
        exclude.contains("/nested\\ \\[x\\]\\*\\?\\ \\#\\!\\\\\\ folder\\ /AGENTS.md\n"),
        "{exclude}"
    );
    let sibling = worktree.join("nested x-other #!\\ folder ");
    fs::create_dir(&sibling).unwrap();
    fs::write(sibling.join("AGENTS.md"), "unmanaged lookalike").unwrap();
    git(&worktree, &["stash", "push", "-u"]);
    assert!(project.join(".agents/skills/review/SKILL.md").is_file());
    assert!(project.join("AGENTS.md").is_file());
    assert!(!sibling.join("AGENTS.md").exists());
}

#[test]
fn symlinked_exclude_fails_without_writing_its_target() {
    let repo = Repo::new();
    let project = project(&repo);
    let runtime = configure(&repo, &project);
    let exclude = project.join(".git/info/exclude");
    let outside = repo.temp.path().join("outside-exclude");
    fs::write(&outside, "keep outside\n").unwrap();
    fs::remove_file(&exclude).unwrap();
    symlink(&outside, &exclude).unwrap();
    assert!(apply_source(&repo.work, &runtime, &runtime.state().unwrap()).is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), "keep outside\n");
}

#[test]
fn unsafe_owned_paths_cannot_expand_exclusions() {
    for symlinked in [false, true] {
        let repo = Repo::new();
        let project = project(&repo);
        let runtime = configure(&repo, &project);
        let state = runtime.state().unwrap();
        assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
        let exclude = project.join(".git/info/exclude");
        let before = fs::read(&exclude).unwrap();
        let outside = repo.temp.path().join("outside");
        fs::write(&outside, "outside\n").unwrap();
        if symlinked {
            let marker = project.join(".agents/skills/review/SKILL.md");
            fs::remove_file(&marker).unwrap();
            symlink(&outside, marker).unwrap();
        } else {
            state
                .replace_owned(
                    &outside,
                    &[OwnedWrite::new(
                        "review",
                        &outside,
                        WriteBody::File {
                            bytes: b"outside\n".to_vec(),
                            executable: false,
                        },
                    )],
                )
                .unwrap();
        }
        let catalog = Catalog::load(&repo.work).unwrap();
        assert!(sm::project_git::protect_owned(&catalog, &runtime.home, &state).is_err());
        assert_eq!(fs::read(&exclude).unwrap(), before);
        assert_eq!(fs::read_to_string(outside).unwrap(), "outside\n");
    }
}

#[test]
fn resource_retirement_removes_only_sm_exclusions() {
    let repo = Repo::new();
    let project = project(&repo);
    let runtime = configure(&repo, &project);
    let state = runtime.state().unwrap();
    let exclude = project.join(".git/info/exclude");
    let original = fs::read_to_string(&exclude).unwrap();
    assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
    let updated = fs::read_to_string(&exclude).unwrap() + "# User addition\nmanual.tmp\n";
    fs::write(&exclude, updated).unwrap();
    repo.catalog(json!({"projects":{"project":project},"resources":[]}));
    assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
    assert_eq!(
        fs::read_to_string(exclude).unwrap(),
        original + "# User addition\nmanual.tmp\n"
    );
}

#[test]
fn retirement_conflict_keeps_retained_ownership_protected_across_syncs() {
    let repo = Repo::new();
    let project = project(&repo);
    let runtime = configure(&repo, &project);
    let state = runtime.state().unwrap();
    assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
    let marker = project.join(".agents/skills/review/SKILL.md");
    fs::write(&marker, "Retain my local edits").unwrap();
    repo.catalog(json!({"projects":{"project":project},"resources":[]}));
    for iteration in 0..2 {
        let report = apply_source(&repo.work, &runtime, &state).unwrap().1;
        assert_eq!(
            report.results["review"].status,
            sm::model::ApplyStatus::Conflict
        );
        assert!(
            state
                .owned()
                .unwrap()
                .iter()
                .any(|write| write.path == marker)
        );
        if iteration == 0 {
            repo.catalog(json!({"projects":{},"resources":[]}));
        }
    }
    git(&project, &["stash", "push", "-u"]);
    assert_eq!(fs::read_to_string(marker).unwrap(), "Retain my local edits");
}

#[test]
fn failed_project_relocation_protects_old_scope_until_ownership_moves() {
    let repo = Repo::new();
    let old_project = project(&repo);
    let runtime = configure(&repo, &old_project);
    let state = runtime.state().unwrap();
    assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
    state
        .set("project_git_protections", &Vec::<serde_json::Value>::new())
        .unwrap();
    let old_marker = old_project.join(".agents/skills/review/SKILL.md");
    fs::write(&old_marker, "Retain my local edits").unwrap();
    let new_repo = Repo::new();
    let new_project = project(&new_repo);
    configure(&repo, &new_project);
    for _ in 0..2 {
        let report = apply_source(&repo.work, &runtime, &state).unwrap().1;
        assert_eq!(
            report.results["review"].status,
            sm::model::ApplyStatus::Conflict
        );
        assert!(
            state
                .owned()
                .unwrap()
                .iter()
                .any(|write| write.path == old_marker)
        );
    }
    git(&old_project, &["stash", "push", "-u"]);
    assert_eq!(
        fs::read_to_string(&old_marker).unwrap(),
        "Retain my local edits"
    );

    fs::write(&old_marker, "Approved").unwrap();
    assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
    assert!(!old_marker.exists());
    assert!(
        !fs::read_to_string(old_project.join(".git/info/exclude"))
            .unwrap()
            .contains("# SM managed project resources")
    );
    git(&new_project, &["stash", "push", "-u"]);
    assert!(new_project.join(".agents/skills/review/SKILL.md").is_file());
}

#[test]
fn removing_last_resource_and_project_clears_only_released_exclusions() {
    let repo = Repo::new();
    let project = project(&repo);
    let runtime = configure(&repo, &project);
    let state = runtime.state().unwrap();
    let exclude = project.join(".git/info/exclude");
    let original = fs::read(&exclude).unwrap();
    assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
    repo.catalog(json!({"projects":{},"resources":[]}));
    assert!(apply_source(&repo.work, &runtime, &state).unwrap().1.ok());
    assert!(
        state
            .owned()
            .unwrap()
            .iter()
            .all(|write| !write.path.starts_with(&project))
    );
    assert_eq!(fs::read(exclude).unwrap(), original);
    assert!(
        state
            .get::<Vec<serde_json::Value>>("project_git_protections")
            .unwrap()
            .is_empty()
    );
}
