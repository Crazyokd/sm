use crate::{
    files::{contained, payload_files, reject_symlink_path},
    model::valid_id,
    process::run_command,
};
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeSet, fs, path::Path, process::Command, time::Duration};

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
    fs::create_dir_all(output)?;
    let mut command = Command::new("npx");
    command
        .args(["--yes", "skills@1.7.2", "add", &selection.source, "--skill"])
        .args(&selection.skills)
        .args(["--agent", "codex", "--yes", "--copy"])
        .current_dir(output)
        .env("DISABLE_TELEMETRY", "1")
        .env("DO_NOT_TRACK", "1")
        .env("SM_INTERNAL_RUN", "1");
    run_command(command, None, Duration::from_secs(480))?;
    manifest(output, &selection.skills)
}

pub fn manifest(output: &Path, skills: &[String]) -> Result<()> {
    let mut entries = Vec::new();
    for skill in skills {
        ensure!(valid_id(skill), "invalid Skill name");
        let relative = Path::new(".agents/skills").join(skill);
        let root = contained(output, &relative)?;
        ensure!(
            root.join("SKILL.md").is_file(),
            "selected upstream Skill missing: {skill}"
        );
        for (_, file) in payload_files(&root)? {
            let source = relative.join(file);
            entries.push(json!({"target":source,"source":source}));
        }
    }
    fs::write(
        output.join("bundle.json"),
        serde_json::to_vec_pretty(&json!({"entries":entries}))?,
    )?;
    Ok(())
}
