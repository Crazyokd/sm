use crate::{files::reject_symlink_path, process::run_command, skill_source::materialize_selected};
use anyhow::{Result, ensure};
use std::{fs, path::Path, process::Command, time::Duration};

pub fn generate(output: &Path) -> Result<()> {
    reject_symlink_path(output)?;
    ensure!(
        !output.exists() || (output.is_dir() && fs::read_dir(output)?.next().is_none()),
        "upstream output must be an empty directory"
    );
    let temporary = tempfile::tempdir()?;
    let staging = temporary.path();
    let mut native = Command::new("npx");
    native
        .args([
            "--yes",
            "@fission-ai/openspec@latest",
            "init",
            "--tools",
            "agents",
            "--profile",
            "core",
            "--force",
        ])
        .current_dir(staging)
        .env("XDG_CONFIG_HOME", staging.join("config"))
        .env("XDG_DATA_HOME", staging.join("data"))
        .env("CODEX_HOME", staging.join("codex"))
        .env("OPENSPEC_TELEMETRY", "0")
        .env("DO_NOT_TRACK", "1")
        .env("SM_INTERNAL_RUN", "1");
    run_command(native, None, Duration::from_secs(480))?;
    materialize_selected(
        staging,
        output,
        &[
            "openspec-apply-change".into(),
            "openspec-archive-change".into(),
            "openspec-explore".into(),
            "openspec-propose".into(),
        ],
    )
}
