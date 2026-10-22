use anyhow::{Context, Result, bail, ensure};
use std::{
    fs::{self, File},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Component, Path, PathBuf},
};
use tempfile::NamedTempFile;
use walkdir::WalkDir;

pub fn relative_path(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty()
            && !path.is_absolute()
            && !path.components().any(|p| matches!(
                p,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )),
        "invalid content path: {}",
        path.display()
    );
    Ok(())
}
pub fn contained(root: &Path, relative: &Path) -> Result<PathBuf> {
    relative_path(relative)?;
    let resolved = root
        .canonicalize()
        .with_context(|| format!("source root missing: {}", root.display()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        if current.exists() || current.is_symlink() {
            ensure!(
                current.canonicalize()?.starts_with(&resolved),
                "path escapes source: {}",
                relative.display()
            );
        }
    }
    Ok(root.join(relative))
}
pub fn reject_symlink_path(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        ensure!(
            !ancestor.is_symlink(),
            "target path contains a symlink: {}",
            ancestor.display()
        );
    }
    Ok(())
}
pub fn payload_files(root: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    ensure!(
        root.is_dir() && !root.is_symlink(),
        "payload must be a regular directory"
    );
    let mut result = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
    {
        let entry = entry?;
        ensure!(
            !entry.file_type().is_symlink(),
            "payload symlink is not accepted: {}",
            entry.path().display()
        );
        if entry.file_type().is_file() {
            result.push((
                entry.path().to_path_buf(),
                entry.path().strip_prefix(root)?.to_path_buf(),
            ));
        } else if !entry.file_type().is_dir() {
            bail!("special payload file is not accepted");
        }
    }
    result.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(result)
}
pub fn atomic_write(path: &Path, bytes: &[u8], executable: Option<bool>) -> Result<()> {
    reject_symlink_path(path)?;
    let parent = path.parent().context("target has no parent")?;
    fs::create_dir_all(parent)?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    let mut mode = if path.exists() {
        fs::metadata(path)?.permissions().mode() & 0o777
    } else {
        0o600
    };
    if let Some(executable) = executable {
        mode = if executable {
            mode | 0o100
        } else {
            mode & !0o111
        };
    }
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(mode))?;
    temporary.persist(path).map_err(|e| e.error)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
pub fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&b))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}
pub fn shell_join<I, S>(values: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    values
        .into_iter()
        .map(|s| shell_quote(s.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for (path, relative) in payload_files(source)? {
        let target = destination.join(relative);
        atomic_write(
            &target,
            &fs::read(&path)?,
            Some(fs::metadata(path)?.permissions().mode() & 0o111 != 0),
        )?;
    }
    Ok(())
}
