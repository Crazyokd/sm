use crate::{
    files::contained,
    model::{Resource, Source, UpstreamSource},
    process::run_command,
};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::Cursor,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
pub fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo).args(args);
    Ok(
        String::from_utf8(run_command(command, None, Duration::from_secs(120))?.stdout)?
            .trim()
            .to_owned(),
    )
}
pub fn fetch_approved(repo: &Path, output: &Path) -> Result<PathBuf> {
    git(repo, &["fetch", "origin", "main"])?;
    let mut command = Command::new("git");
    command.arg("-C").arg(repo).args(["archive", "origin/main"]);
    let archive = run_command(command, None, Duration::from_secs(120))?.stdout;
    fs::create_dir_all(output)?;
    for entry in tar::Archive::new(Cursor::new(archive)).entries()? {
        let mut entry = entry?;
        let relative = entry.path()?.into_owned();
        if relative != Path::new("catalog.json") && !relative.starts_with("resources") {
            continue;
        }
        let target = contained(output, &relative)?;
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            fs::create_dir_all(target)?;
        } else if kind.is_file() {
            fs::create_dir_all(target.parent().context("archive target has no parent")?)?;
            let mut file = fs::File::create(&target)?;
            std::io::copy(&mut entry, &mut file)?;
            fs::set_permissions(
                target,
                fs::Permissions::from_mode(if entry.header().mode()? & 0o111 != 0 {
                    0o700
                } else {
                    0o600
                }),
            )?;
        } else {
            bail!(
                "source link or special file is not accepted: {}",
                relative.display()
            );
        }
    }
    Ok(output.to_path_buf())
}
pub fn fetch_upstream(resource: &Resource, output: &Path) -> Result<PathBuf> {
    let Source::Upstream { upstream } = &resource.source else {
        bail!("upstream source required");
    };
    let mut command = match upstream {
        UpstreamSource::Git { git } => {
            let mut command = Command::new("git");
            command
                .args(["clone", "--depth", "1", "--", git])
                .arg(output);
            command
        }
        UpstreamSource::Generator { argv } => {
            let (program, args) = argv.split_first().context("upstream generator is empty")?;
            fs::create_dir_all(output)?;
            let mut command = Command::new(program);
            command
                .args(args)
                .arg(output.canonicalize()?)
                .current_dir(&resource.root);
            command
        }
    };
    command.env("SM_INTERNAL_RUN", "1");
    run_command(command, None, Duration::from_secs(120))?;
    Ok(output.to_path_buf())
}
