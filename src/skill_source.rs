use crate::{
    files::{atomic_write, contained, payload_files, reject_symlink_path},
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
    let installed = tempfile::tempdir()?;
    let mut command = Command::new("npx");
    command
        .args([
            "--yes",
            "skills@1.7.2",
            "add",
            &selection.source,
            "--skill",
            "*",
        ])
        .args(["--agent", "codex", "--yes", "--copy"])
        .current_dir(installed.path())
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
