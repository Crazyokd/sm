use crate::{
    files::{atomic_write, contained, payload_files, reject_symlink_path, relative_path},
    model::valid_id,
    process::run_command,
};
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::{
    collections::BTreeSet, fs, os::unix::fs::PermissionsExt, path::Path, process::Command,
    time::Duration,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    source: String,
    skills: Vec<String>,
    #[serde(default)]
    subpath: Option<String>,
}

/// Materialize selected upstream Skills with their native installer, never into the real home.
pub fn generate(selection: &Path, output: &Path) -> Result<()> {
    reject_symlink_path(output)?;
    ensure!(
        !output.exists() || (output.is_dir() && fs::read_dir(output)?.next().is_none()),
        "upstream output must be an empty directory"
    );
    let selection: Selection = serde_json::from_slice(&fs::read(selection)?)?;
    let url = url::Url::parse(&selection.source)?;
    ensure!(
        url.scheme() == "https" && url.username().is_empty() && url.password().is_none(),
        "upstream requires a public HTTPS source"
    );
    ensure!(
        !selection.skills.is_empty()
            && selection.skills.iter().all(|id| valid_id(id))
            && selection.skills.iter().collect::<BTreeSet<_>>().len() == selection.skills.len(),
        "invalid upstream Skill selection"
    );
    if let Some(subpath) = &selection.subpath {
        relative_path(Path::new(subpath))?;
        ensure!(
            subpath
                .split('/')
                .all(|part| !matches!(part, "" | "." | ".."))
                && !subpath
                    .chars()
                    .any(|c| c.is_control() || "*?[]\\".contains(c)),
            "invalid upstream Skill subpath"
        );
    }
    let installed = tempfile::tempdir()?;
    let local_source = if let Some(subpath) = &selection.subpath {
        let checkout = installed.path().join("upstream");
        let mut clone = Command::new("git");
        clone
            .args([
                "clone",
                "--filter=blob:none",
                "--no-checkout",
                "--depth",
                "1",
                "-c",
                "filter.lfs.required=false",
                "-c",
                "filter.lfs.smudge=",
                "-c",
                "filter.lfs.clean=",
                "-c",
                "filter.lfs.process=",
                "--",
            ])
            .arg(&selection.source)
            .arg(&checkout)
            .env_remove("GIT_DIR")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env("GIT_TERMINAL_PROMPT", "0");
        run_command(clone, None, Duration::from_secs(480))?;
        let mut sparse = Command::new("git");
        sparse
            .args(["sparse-checkout", "set", "--no-cone", "--"])
            .arg(format!("/{subpath}/"))
            .current_dir(&checkout)
            .env_remove("GIT_DIR")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE");
        run_command(sparse, None, Duration::from_secs(480))?;
        let mut checkout_head = Command::new("git");
        checkout_head
            .args(["checkout", "HEAD"])
            .current_dir(&checkout)
            .env_remove("GIT_DIR")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env("GIT_TERMINAL_PROMPT", "0");
        run_command(checkout_head, None, Duration::from_secs(480))?;
        let root = contained(&checkout, Path::new(subpath))?;
        payload_files(&root)?;
        ensure!(
            root.join("SKILL.md").is_file(),
            "upstream Skill subpath marker missing"
        );
        Some(root)
    } else {
        None
    };
    let mut command = Command::new("npx");
    command.args(["--yes", "skills@1.7.2", "add"]);
    if let Some(root) = &local_source {
        command.arg(root);
    } else {
        command.arg(&selection.source);
    }
    command
        .args(["--skill", "*"])
        .args(["--agent", "codex", "--yes", "--copy"])
        .current_dir(installed.path())
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("INSTALL_INTERNAL_SKILLS", "1")
        .env("DISABLE_TELEMETRY", "1")
        .env("DO_NOT_TRACK", "1")
        .env("SM_INTERNAL_RUN", "1");
    run_command(command, None, Duration::from_secs(480))?;
    materialize_selected(installed.path(), output, &selection.skills)
}

pub fn materialize_selected(installed: &Path, output: &Path, skills: &[String]) -> Result<()> {
    reject_symlink_path(output)?;
    ensure!(
        !output.exists() || (output.is_dir() && fs::read_dir(output)?.next().is_none()),
        "upstream output must be an empty directory"
    );
    let mut files = Vec::new();
    let mut missing = Vec::new();
    for skill in skills {
        ensure!(valid_id(skill), "invalid Skill name");
        let relative = Path::new(".agents/skills").join(skill);
        let root = contained(installed, &relative)?;
        reject_symlink_path(&root)?;
        if !root.exists() {
            missing.push(skill.as_str());
            continue;
        }
        let payload = payload_files(&root)?;
        ensure!(
            root.join("SKILL.md").is_file(),
            "selected upstream Skill marker missing: {skill}"
        );
        for (path, file) in payload {
            files.push((
                relative.join(file),
                fs::read(&path)?,
                fs::metadata(&path)?.permissions().mode() & 0o111 != 0,
            ));
        }
    }
    if !missing.is_empty() {
        eprintln!(
            "upstream source omitted {} selected Skills: {}",
            missing.len(),
            missing.into_iter().take(8).collect::<Vec<_>>().join(", ")
        );
    }
    fs::create_dir_all(output)?;
    for (target, bytes, executable) in files {
        atomic_write(&output.join(target), &bytes, Some(executable))?;
    }
    Ok(())
}
