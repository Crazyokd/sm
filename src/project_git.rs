use crate::{
    files::{atomic_write, reject_symlink_path, relative_path},
    model::{Catalog, ResourceKind, Scope},
    privacy::redact_text,
    process::run_command,
    state::State,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const START: &[u8] = b"# SM managed project resources\n";
const END: &[u8] = b"# /SM managed project resources\n";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProtectedWrite {
    resource_id: String,
    path: PathBuf,
    skill: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    project: PathBuf,
    writes: Vec<ProtectedWrite>,
    repositories: BTreeSet<PathBuf>,
}

fn git_command(project: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("--literal-pathspecs")
        .arg("-C")
        .arg(project)
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("SM_INTERNAL_RUN", "1");
    command
}

fn git_paths(project: &Path) -> Result<Option<(PathBuf, PathBuf)>> {
    let Some(dot_git) = project
        .ancestors()
        .map(|path| path.join(".git"))
        .find(|path| path.exists() || path.is_symlink())
    else {
        return Ok(None);
    };
    reject_symlink_path(&dot_git)?;
    let mut command = git_command(project);
    command.args([
        "rev-parse",
        "--path-format=absolute",
        "--show-toplevel",
        "--git-common-dir",
        "--git-path",
        "info/exclude",
    ]);
    let output = run_command(command, None, Duration::from_secs(15))?;
    let output = String::from_utf8(output.stdout)?;
    let paths: Vec<_> = output
        .strip_suffix('\n')
        .unwrap_or(&output)
        .split('\n')
        .collect();
    ensure!(
        paths.len() == 3 && paths.iter().all(|path| !path.contains('\r')),
        "invalid project Git paths"
    );
    let root = PathBuf::from(paths[0]);
    let common = PathBuf::from(paths[1]);
    let exclude = PathBuf::from(paths[2]);
    ensure!(
        root.is_absolute() && project.starts_with(&root),
        "project is outside its Git worktree"
    );
    ensure!(
        common.is_absolute() && exclude == common.join("info/exclude"),
        "Git exclude is outside its metadata directory"
    );
    reject_symlink_path(&exclude)?;
    Ok(Some((root, exclude)))
}

fn resolve_git(
    path: &Path,
    cache: &mut BTreeMap<PathBuf, Option<(PathBuf, PathBuf)>>,
) -> Result<Option<(PathBuf, PathBuf)>> {
    let Some(directory) = path.ancestors().find(|directory| {
        let marker = directory.join(".git");
        marker.exists() || marker.is_symlink()
    }) else {
        return Ok(None);
    };
    if !cache.contains_key(directory) {
        cache.insert(directory.to_path_buf(), git_paths(directory)?);
    }
    Ok(cache[directory].clone())
}

fn pattern(path: &Path, directory: bool) -> Result<String> {
    relative_path(path)?;
    let path = path.to_str().context("project Git path must be UTF-8")?;
    ensure!(
        !path.contains(['\n', '\r']),
        "project Git path contains a line break"
    );
    let mut pattern = String::from("/");
    for character in path.chars() {
        if matches!(
            character,
            '\\' | '*' | '?' | '[' | ']' | '#' | '!' | ' ' | '\t'
        ) {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    if directory {
        pattern.push('/');
    }
    Ok(pattern)
}

fn marker(bytes: &[u8], marker: &[u8]) -> Option<usize> {
    bytes
        .windows(marker.len())
        .enumerate()
        .find_map(|(index, part)| {
            (part == marker && (index == 0 || bytes[index - 1] == b'\n')).then_some(index)
        })
}

fn update_exclude(path: &Path, patterns: &BTreeSet<String>) -> Result<()> {
    reject_symlink_path(path)?;
    let before = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let mut after = before.clone();
    if let Some(start) = marker(&after, START) {
        let tail = start + START.len();
        let end = marker(&after[tail..], END).context("unterminated SM Git exclude block")?
            + tail
            + END.len();
        after.drain(start..end);
        ensure!(
            marker(&after, START).is_none(),
            "duplicate SM Git exclude blocks"
        );
    }
    if !patterns.is_empty() {
        if !after.is_empty() && !after.ends_with(b"\n") {
            after.push(b'\n');
        }
        after.extend_from_slice(START);
        for pattern in patterns {
            after.extend_from_slice(pattern.as_bytes());
            after.push(b'\n');
        }
        after.extend_from_slice(END);
    }
    if after != before {
        atomic_write(path, &after, None)?;
    }
    Ok(())
}

pub fn protect_owned(catalog: &Catalog, home: &Path, state: &State) -> Result<()> {
    let owned = state.owned()?;
    let previous: Catalog = state.get("catalog")?;
    let receipts: Vec<Receipt> = state.get("project_git_protections")?;
    let mut projects = BTreeMap::<PathBuf, BTreeMap<(String, PathBuf), bool>>::new();
    let mut previous_repositories = BTreeSet::new();
    // Retained ownership outlives catalog changes and both apply phases.
    for receipt in receipts {
        previous_repositories.extend(receipt.repositories);
        let writes = projects.entry(receipt.project).or_default();
        for write in receipt.writes {
            if owned.iter().any(|current| {
                current.resource_id == write.resource_id && current.path == write.path
            }) {
                writes.insert((write.resource_id, write.path), write.skill);
            }
        }
    }
    for catalog in [&previous, catalog] {
        for resource in catalog
            .resources
            .iter()
            .filter(|resource| resource.scope == Scope::Project)
        {
            let root = catalog.scope_root(resource, home)?;
            let writes = projects.entry(root.clone()).or_default();
            for write in owned
                .iter()
                .filter(|write| write.resource_id == resource.id && write.path.starts_with(&root))
            {
                writes.insert(
                    (write.resource_id.clone(), write.path.clone()),
                    resource.content.kind() == ResourceKind::Skill,
                );
            }
        }
    }
    // A scope change may also retain user files, which are not project exclusions.
    for write in &owned {
        if catalog
            .resources
            .iter()
            .any(|resource| resource.id == write.resource_id && resource.scope == Scope::Project)
            && !write.path.starts_with(home)
        {
            ensure!(
                projects
                    .values()
                    .any(|writes| writes
                        .contains_key(&(write.resource_id.clone(), write.path.clone()))),
                "owned project path is outside its known scopes"
            );
        }
    }
    let mut cache = BTreeMap::new();
    let mut repositories = BTreeMap::<PathBuf, (PathBuf, BTreeSet<(PathBuf, bool)>)>::new();
    let mut retained = Vec::new();
    for (project, writes) in projects {
        let mut protected = Vec::new();
        let mut project_repositories = BTreeSet::new();
        for ((resource_id, path), skill) in writes {
            relative_path(
                path.strip_prefix(&project)
                    .context("owned project path is outside its scope")?,
            )?;
            reject_symlink_path(&path)?;
            let parent = path.parent().context("project file has no parent")?;
            if let Some((root, exclude)) = resolve_git(parent, &mut cache)? {
                project_repositories.insert(root.clone());
                let directory = skill
                    && path.file_name().is_some_and(|name| name == "SKILL.md")
                    && parent != root;
                let paths = &mut repositories
                    .entry(root)
                    .or_insert_with(|| (exclude, BTreeSet::new()))
                    .1;
                paths.insert((
                    if directory {
                        parent.to_path_buf()
                    } else {
                        path.clone()
                    },
                    directory,
                ));
            }
            protected.push(ProtectedWrite {
                resource_id,
                path,
                skill,
            });
        }
        if !protected.is_empty() {
            retained.push(Receipt {
                project: project.clone(),
                writes: protected,
                repositories: project_repositories,
            });
        }
    }
    for previous in previous_repositories {
        if let Some((root, exclude)) = resolve_git(&previous, &mut cache)? {
            repositories
                .entry(root)
                .or_insert_with(|| (exclude, BTreeSet::new()));
        }
    }
    let mut excludes = BTreeMap::<PathBuf, BTreeSet<String>>::new();
    for (root, (exclude, paths)) in &repositories {
        let patterns = excludes.entry(exclude.clone()).or_default();
        for (path, directory) in paths {
            if !paths.iter().any(|(other, is_directory)| {
                *is_directory && path != other && path.starts_with(other)
            }) {
                patterns.insert(pattern(path.strip_prefix(root)?, *directory)?);
            }
        }
    }
    for (path, patterns) in excludes {
        update_exclude(&path, &patterns)?;
    }
    let mut warnings = BTreeMap::<PathBuf, Vec<PathBuf>>::new();
    for (root, (_, paths)) in repositories {
        if paths.is_empty() {
            continue;
        }
        let mut command = git_command(&root);
        command
            .args(["ls-files", "--others", "--exclude-standard", "-z", "--"])
            .args(paths.iter().map(|(path, _)| path));
        let output = run_command(command, None, Duration::from_secs(15))?;
        if !output.stdout.is_empty() {
            warnings.insert(
                root,
                String::from_utf8(output.stdout)?
                    .split_terminator('\0')
                    .map(PathBuf::from)
                    .collect(),
            );
        }
    }
    if state.get::<BTreeMap<PathBuf, Vec<PathBuf>>>("project_git_protection_warnings")? != warnings
    {
        let report = serde_json::json!({
            "warnings": warnings,
            "reason": "Project Git rules may override local exclusions; listed files remain exposed to stash -u"
        });
        state.add_run("project-git-protection", &report)?;
        eprintln!(
            "SM project Git protection: {}",
            redact_text(&report.to_string(), Some(1200))
        );
    }
    state.set("project_git_protection_warnings", &warnings)?;
    state.set("project_git_protections", &retained)
}
