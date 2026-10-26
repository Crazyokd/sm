mod common;

use common::{Repo, git};
use sm::{
    apply::{apply_writes, plan_apply},
    model::Catalog,
};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

#[test]
fn installed_force_sync_is_self_contained_and_survives_stash() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = Repo::new();
    let child = Repo::new();
    let fallback = Repo::new();
    git(&fallback.work, &["branch", "-m", "release"]);
    git(&fallback.work, &["push", "origin", "release"]);
    git(
        &fallback.remote,
        &["symbolic-ref", "HEAD", "refs/heads/release"],
    );
    git(&fallback.remote, &["update-ref", "-d", "refs/heads/main"]);
    fs::create_dir(root.work.join("src")).unwrap();
    for (name, remote) in [("child", &child.remote), ("fallback", &fallback.remote)] {
        git(
            &root.work,
            &["clone", remote.to_str().unwrap(), &format!("src/{name}")],
        );
    }
    let wrapper = root.work.join("src/not-a-repository");
    fs::create_dir(&wrapper).unwrap();
    fs::write(wrapper.join("keep.txt"), "keep wrapper").unwrap();
    let home = root.temp.path().join("home");
    fs::create_dir(&home).unwrap();
    let mut catalog = Catalog::load(source).unwrap();
    catalog.resources.retain(|r| r.id == "easygo-force-sync");
    catalog
        .projects
        .insert("easygo".into(), root.work.to_str().unwrap().into());
    let state = root.state();
    let plan = plan_apply(
        &catalog,
        &BTreeMap::from([("easygo-force-sync".into(), source.into())]),
        &home,
    );
    assert!(apply_writes(&plan, &state).unwrap().ok());
    let installed = root.work.join(".agents/skills/easygo-force-sync");
    let script = installed.join("scripts/force-sync-origin-main.sh");
    assert!(
        script.is_file(),
        "the installed Skill must include its executable dependency"
    );
    let instructions = fs::read_to_string(installed.join("SKILL.md")).unwrap();
    assert!(
        instructions
            .contains("bash .agents/skills/easygo-force-sync/scripts/force-sync-origin-main.sh")
    );
    assert!(!root.work.join("script/force-sync-origin-main.sh").exists());
    fs::write(
        root.work.join(".git/info/exclude"),
        "/.agents/skills/easygo-force-sync/\n",
    )
    .unwrap();
    for repo in [
        &root.work,
        &root.work.join("src/child"),
        &root.work.join("src/fallback"),
    ] {
        fs::write(repo.join("catalog.json"), "local tracked edit").unwrap();
        fs::write(repo.join("keep-untracked.txt"), "keep untracked").unwrap();
        git(
            repo,
            &[
                "-c",
                "user.name=SM Test",
                "-c",
                "user.email=sm@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "Local commit",
            ],
        );
    }
    let output = Command::new("bash")
        .arg(&script)
        .current_dir(&root.work)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    for (repo, target) in [
        (&root.work, "origin/main"),
        (&root.work.join("src/child"), "origin/main"),
        (&root.work.join("src/fallback"), "origin/release"),
    ] {
        assert_eq!(
            git(repo, &["rev-parse", "HEAD"]),
            git(repo, &["rev-parse", target])
        );
        assert_eq!(
            fs::read_to_string(repo.join("catalog.json"))
                .unwrap()
                .trim(),
            git(repo, &["show", &format!("{target}:catalog.json")])
        );
        assert_eq!(
            fs::read_to_string(repo.join("keep-untracked.txt")).unwrap(),
            "keep untracked"
        );
    }
    assert_eq!(
        fs::read_to_string(wrapper.join("keep.txt")).unwrap(),
        "keep wrapper"
    );
    fs::write(root.work.join("catalog.json"), "another local edit").unwrap();
    git(
        &root.work,
        &[
            "stash",
            "push",
            "--include-untracked",
            "-m",
            "switch branch",
        ],
    );
    assert!(
        script.is_file(),
        "Git must leave the ignored SM installation in place"
    );
    assert!(installed.join("SKILL.md").is_file());
    assert!(installed.join("agents/openai.yaml").is_file());
}

#[test]
fn force_sync_blocks_untracked_and_ignored_path_collisions() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources/easygo-force-sync/scripts/force-sync-origin-main.sh");
    for ignored in [false, true] {
        for (local_path, remote_path) in [
            ("local-config", "local-config"),
            ("settings", "settings/config"),
            ("settings/config", "settings"),
        ] {
            let repo = Repo::new();
            let checkout = repo.temp.path().join("checkout");
            git(
                &repo.work,
                &[
                    "clone",
                    repo.remote.to_str().unwrap(),
                    checkout.to_str().unwrap(),
                ],
            );
            let before = git(&checkout, &["rev-parse", "HEAD"]);
            let local = checkout.join(local_path);
            fs::create_dir_all(local.parent().unwrap()).unwrap();
            fs::write(&local, "unique local data").unwrap();
            if ignored {
                fs::write(
                    checkout.join(".git/info/exclude"),
                    format!("/{local_path}\n"),
                )
                .unwrap();
            }
            let remote = repo.work.join(remote_path);
            fs::create_dir_all(remote.parent().unwrap()).unwrap();
            fs::write(remote, "remote file").unwrap();
            repo.publish("Add conflicting remote path");
            let output = Command::new("bash")
                .arg(&script)
                .current_dir(&checkout)
                .output()
                .unwrap();
            assert!(
                !output.status.success(),
                "must block {local_path} -> {remote_path}, ignored={ignored}"
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("BLOCKED"));
            assert_eq!(fs::read_to_string(local).unwrap(), "unique local data");
            assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), before);
        }
    }
}
