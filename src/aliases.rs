use crate::{
    files::reject_symlink_path,
    model::{ApplyStatus, ResourceResult, valid_id},
    state::State,
};
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetiredAlias {
    path: PathBuf,
    previous: PathBuf,
}

/// Retires only reviewed, unchanged dangling aliases left by an earlier migration.
pub fn synchronize(
    source: &Path,
    home: &Path,
    _state: &State,
) -> Result<BTreeMap<String, ResourceResult>> {
    let manifest = source.join("resources/native-installations/retired-aliases.json");
    if !manifest.exists() {
        return Ok(BTreeMap::new());
    }
    reject_symlink_path(&manifest)
        .map_err(|_| anyhow::anyhow!("alias manifest contains a symlink"))?;
    let aliases: Vec<RetiredAlias> = serde_json::from_slice(&fs::read(manifest)?)
        .map_err(|_| anyhow::anyhow!("invalid retired alias manifest"))?;
    let mut names = BTreeSet::new();
    for alias in &aliases {
        let name = alias.path.file_name().and_then(|name| name.to_str());
        ensure!(
            alias.path.parent() == Some(Path::new(".claude/skills"))
                && alias.path.components().count() == 3
                && alias
                    .path
                    .components()
                    .all(|part| matches!(part, Component::Normal(_)))
                && name.is_some_and(valid_id)
                && names.insert(name)
                && !alias.previous.as_os_str().is_empty(),
            "invalid or duplicate retired alias path"
        );
    }
    let home = home.canonicalize()?;
    Ok(aliases
        .into_iter()
        .map(|alias| {
            let id = format!(
                "@alias/{}",
                alias.path.file_name().unwrap().to_string_lossy()
            );
            (id, retire(&home.join(alias.path), &alias.previous))
        })
        .collect())
}

fn retire(path: &Path, previous: &Path) -> ResourceResult {
    let conflict = || {
        ResourceResult::new(
            ApplyStatus::Conflict,
            "Alias changed, is active, or cannot be verified; preserved",
        )
    };
    if reject_symlink_path(path.parent().expect("validated alias parent")).is_err() {
        return conflict();
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return ResourceResult::new(ApplyStatus::Applied, "Alias already absent");
        }
        Err(_) => return conflict(),
    };
    if !metadata.file_type().is_symlink() || fs::read_link(path).ok().as_deref() != Some(previous) {
        return conflict();
    }
    if !matches!(fs::metadata(path), Err(error) if error.kind() == ErrorKind::NotFound) {
        return conflict();
    }
    match fs::remove_file(path) {
        Ok(()) => ResourceResult::new(ApplyStatus::Applied, "Reviewed dangling alias retired"),
        Err(_) => ResourceResult::new(ApplyStatus::Failed, "Alias could not be retired"),
    }
}
