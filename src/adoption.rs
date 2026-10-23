use crate::{
    apply::merge_entries,
    files::{contained, reject_symlink_path},
    model::{ApplyPlan, Catalog, ResourceContent, StoredBody, StoredWrite, WriteBody, valid_id},
    state::State,
};
use anyhow::{Context, Result, ensure};
use rusqlite::params;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Adoption {
    resource: String,
    target: PathBuf,
    sha256: String,
    executable: Option<bool>,
    previous_resource: Option<String>,
    #[serde(default)]
    previous_resources: Vec<String>,
    shared_with: Option<String>,
}

pub fn fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn matches_current(body: &StoredBody, bytes: &[u8], executable: bool) -> Result<bool> {
    Ok(match body {
        StoredBody::FileFingerprint {
            sha256,
            executable: previous,
        } => *sha256 == <[u8; 32]>::from(Sha256::digest(bytes)) && *previous == executable,
        StoredBody::Json(fragment) | StoredBody::Toml(fragment) => {
            let mut document: Value = if matches!(body, StoredBody::Toml(_)) {
                toml_edit::de::from_str(std::str::from_utf8(bytes)?)?
            } else {
                serde_json::from_slice(bytes)?
            };
            merge_entries(&mut document, fragment, true).is_ok()
        }
        StoredBody::Markdown(_) => false,
    })
}

fn empty_fragment(value: &Value) -> bool {
    match value {
        Value::Object(values) => values.values().all(|value| match value {
            Value::Object(entries) => entries.is_empty(),
            Value::Array(entries) => entries.is_empty(),
            _ => false,
        }),
        Value::Array(values) => values.is_empty(),
        _ => false,
    }
}

/// Only reviewed fingerprints can transfer existing content into SM ownership.
pub fn adopt(
    source: &Path,
    catalog: &Catalog,
    home: &Path,
    plan: &ApplyPlan,
    state: &State,
) -> Result<()> {
    let manifest = source.join("resources/adoption.json");
    if !manifest.exists() {
        return Ok(());
    }
    let entries: Vec<Adoption> = serde_json::from_slice(&fs::read(manifest)?)?;
    let mut owned: BTreeMap<_, _> = state
        .owned()?
        .into_iter()
        .map(|write| ((write.resource_id.clone(), write.path.clone()), write))
        .collect();
    let mut consumed: BTreeSet<String> = state.get("adopted_targets")?;
    let mut changed = BTreeSet::new();
    for entry in entries {
        ensure!(
            entry.sha256.len() == 64 && entry.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid adoption fingerprint"
        );
        ensure!(
            entry.previous_resource.as_deref().is_none_or(valid_id),
            "invalid previous owner"
        );
        ensure!(
            entry.shared_with.as_deref().is_none_or(valid_id)
                && entry.previous_resources.iter().all(|id| valid_id(id))
                && entry
                    .previous_resources
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len()
                    == entry.previous_resources.len()
                && usize::from(entry.previous_resource.is_some())
                    + usize::from(!entry.previous_resources.is_empty())
                    + usize::from(entry.shared_with.is_some())
                    <= 1,
            "shared adoption requires one valid named owner"
        );
        let resource = catalog
            .resources
            .iter()
            .find(|r| r.id == entry.resource)
            .context("adoption resource missing")?;
        let scope = catalog.scope_root(resource, home)?;
        if !scope.is_dir() {
            continue;
        }
        let path = contained(&scope, &entry.target)?;
        reject_symlink_path(&path)?;
        let key = format!("{}:{}", entry.resource, entry.target.display());
        if consumed.contains(&key) || owned.contains_key(&(entry.resource.clone(), path.clone())) {
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let write = plan
            .writes
            .iter()
            .find(|w| w.resource_id == entry.resource && w.path == path);
        // A reviewed existing upstream file is governable even while its refresh is offline.
        let baseline;
        let write = if let Some(write) = write {
            write
        } else if (!resource.source.is_self()
            || resource.dependencies.iter().any(|id| {
                catalog.dependencies.get(id).is_some_and(|dependency| {
                    !dependency.source.is_self()
                        && dependency.files.entries.iter().any(|projection| {
                            matches!(projection, crate::model::ProjectionEntry::File { target, .. } if target == &entry.target)
                        })
                })
            }))
            && crate::projections::adoptable_file_target(&entry.target, resource.scope)
            && !entry
                .target
                .components()
                .any(|part| part.as_os_str() == ".git")
            && (resource.dependencies.iter().any(|id| {
                catalog.dependencies.get(id).is_some_and(|dependency| {
                    dependency.files.entries.iter().any(|projection| {
                        matches!(projection, crate::model::ProjectionEntry::File { target, .. } if target == &entry.target)
                    })
                })
            }) || match &resource.content {
                ResourceContent::Files { files, .. } => {
                    files.entries.iter().any(|projection| match projection {
                        crate::model::ProjectionEntry::File { target, .. } => {
                            target == &entry.target
                        }
                        crate::model::ProjectionEntry::Tree { target, .. } => {
                            entry.target != *target && entry.target.starts_with(target)
                        }
                        _ => false,
                    })
                }
                ResourceContent::Skill { .. } => entry
                    .target
                    .starts_with(Path::new(".agents/skills").join(&resource.id)),
                _ => false,
            })
        {
            baseline = crate::model::OwnedWrite::new(
                &entry.resource,
                path.clone(),
                WriteBody::File {
                    bytes: fs::read(&path)?,
                    executable: fs::metadata(&path)?.permissions().mode() & 0o111 != 0,
                },
            );
            &baseline
        } else {
            continue;
        };
        let bytes = fs::read(&path)?;
        let executable = fs::metadata(&path)?.permissions().mode() & 0o111 != 0;
        if entry
            .executable
            .is_some_and(|expected| expected != executable)
        {
            continue;
        }
        let mut claim = StoredWrite::from(write);
        let digest = match &write.body {
            WriteBody::File { .. } => {
                claim.body = StoredBody::FileFingerprint {
                    sha256: Sha256::digest(&bytes).into(),
                    executable,
                };
                fingerprint(&bytes)
            }
            WriteBody::Json(fragment) | WriteBody::Toml(fragment) => {
                let mut document: Value = if matches!(&write.body, WriteBody::Toml(_)) {
                    toml_edit::de::from_str(std::str::from_utf8(&bytes)?)?
                } else {
                    serde_json::from_slice(&bytes)?
                };
                if merge_entries(&mut document, fragment, true).is_err() {
                    continue;
                }
                fingerprint(&serde_json::to_vec(fragment)?)
            }
            WriteBody::Markdown(_) => continue,
        };
        if digest != entry.sha256 {
            continue;
        }
        if !entry.previous_resources.is_empty() {
            let mut combined = Value::Object(Default::default());
            let mut keys = Vec::new();
            let mut valid = true;
            for previous in entry.previous_resources {
                let key = (previous, path.clone());
                let Some(old) = owned.get(&key) else {
                    valid = false;
                    break;
                };
                let fragment = match (&old.body, &claim.body) {
                    (StoredBody::Json(fragment), StoredBody::Json(_))
                    | (StoredBody::Toml(fragment), StoredBody::Toml(_)) => fragment,
                    _ => {
                        valid = false;
                        break;
                    }
                };
                if !matches_current(&old.body, &bytes, executable)?
                    || merge_entries(&mut combined, fragment, false).is_err()
                {
                    valid = false;
                    break;
                }
                keys.push(key);
            }
            let expected = match &claim.body {
                StoredBody::Json(fragment) | StoredBody::Toml(fragment) => {
                    Value::Object(fragment.clone())
                }
                _ => continue,
            };
            if !valid || combined != expected {
                continue;
            }
            for key in keys {
                owned.remove(&key);
                changed.insert(key);
            }
        } else if let Some(previous) = entry.previous_resource {
            let previous_key = (previous, path.clone());
            let Some(old) = owned.get(&previous_key) else {
                continue;
            };
            if !matches_current(&old.body, &bytes, executable)? {
                continue;
            }
            let remaining = match (&old.body, &claim.body) {
                (StoredBody::FileFingerprint { .. }, StoredBody::FileFingerprint { .. }) => {
                    if entry.executable.is_none()
                        || owned.values().filter(|w| w.path == path).count() != 1
                    {
                        continue;
                    }
                    None
                }
                (StoredBody::Json(old), StoredBody::Json(fragment))
                | (StoredBody::Toml(old), StoredBody::Toml(fragment)) => {
                    let mut rest = Value::Object(old.clone());
                    if merge_entries(&mut rest, fragment, true).is_err() {
                        continue;
                    }
                    if empty_fragment(&rest) {
                        None
                    } else {
                        let Value::Object(rest) = rest else {
                            unreachable!("fragment remains an object")
                        };
                        let mut previous = owned
                            .get(&previous_key)
                            .expect("checked previous owner")
                            .clone();
                        previous.body = if matches!(&claim.body, StoredBody::Toml(_)) {
                            StoredBody::Toml(rest)
                        } else {
                            StoredBody::Json(rest)
                        };
                        Some(previous)
                    }
                }
                _ => continue,
            };
            if let Some(remaining) = remaining {
                owned.insert(previous_key.clone(), remaining);
            } else {
                owned.remove(&previous_key);
            }
            changed.insert(previous_key);
        } else if let Some(shared) = entry.shared_with {
            let Some(other) = catalog.resources.iter().find(|r| r.id == shared) else {
                continue;
            };
            let shared_dependency = resource.dependencies.iter().any(|id| {
                other.dependencies.contains(id)
                    && catalog.dependencies.get(id).is_some_and(|dependency| {
                        dependency.files.entries.iter().any(|projection| {
                            matches!(projection, crate::model::ProjectionEntry::File { target, .. } if target == &entry.target)
                        })
                    })
            });
            let Some(old) = owned.get(&(shared, path.clone())) else {
                continue;
            };
            if entry.executable.is_none()
                || !matches!(claim.body, StoredBody::FileFingerprint { .. })
                || !shared_dependency
                || catalog.scope_root(other, home)? != scope
                || old.body != claim.body
                || !matches_current(&old.body, &bytes, executable)?
                || owned
                    .values()
                    .any(|w| w.path == path && w.body != claim.body)
            {
                continue;
            }
        } else if matches!(&claim.body, StoredBody::FileFingerprint { .. })
            && owned.values().any(|w| w.path == path)
        {
            continue;
        }
        let claim_key = (claim.resource_id.clone(), claim.path.clone());
        changed.insert(claim_key.clone());
        owned.insert(claim_key, claim);
        consumed.insert(key);
    }
    let mut db = state.connect()?;
    let tx = db.transaction()?;
    for (resource, path) in changed {
        if let Some(claim) = owned.get(&(resource.clone(), path.clone())) {
            tx.execute(
                "INSERT OR REPLACE INTO ownership VALUES (?1,?2,?3)",
                params![
                    resource,
                    path.to_string_lossy(),
                    serde_json::to_string(claim)?
                ],
            )?;
        } else {
            tx.execute(
                "DELETE FROM ownership WHERE resource=?1 AND path=?2",
                params![resource, path.to_string_lossy()],
            )?;
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO kv VALUES ('adopted_targets',?1)",
        [serde_json::to_string(&consumed)?],
    )?;
    tx.commit()?;
    Ok(())
}
