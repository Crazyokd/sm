use crate::{
    apply::merge_entries,
    files::{contained, reject_symlink_path},
    model::{ApplyPlan, Catalog, StoredBody, StoredWrite, WriteBody},
    state::State,
};
use anyhow::{Context, Result, ensure};
use rusqlite::params;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
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
}

pub fn fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
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
    let owned = state.owned()?;
    let mut consumed: BTreeSet<String> = state.get("adopted_targets")?;
    let mut claims = Vec::new();
    for entry in entries {
        ensure!(
            entry.sha256.len() == 64 && entry.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid adoption fingerprint"
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
        if consumed.contains(&key)
            || owned
                .iter()
                .any(|w| w.resource_id == entry.resource && w.path == path)
        {
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
        } else if !resource.source.is_self()
            && crate::bundles::adoptable_file_target(&entry.target, resource.scope)
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
        let mut claim = StoredWrite::from(write);
        let digest = match &write.body {
            WriteBody::File { .. } => {
                claim.body = StoredBody::FileFingerprint {
                    sha256: Sha256::digest(&bytes).into(),
                    executable: fs::metadata(&path)?.permissions().mode() & 0o111 != 0,
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
        // Whole files cannot transfer between owners; fragments may share a config.
        if matches!(&claim.body, StoredBody::FileFingerprint { .. })
            && owned.iter().any(|w| w.path == path)
        {
            continue;
        }
        claims.push(claim);
        consumed.insert(key);
    }
    let mut db = state.connect()?;
    let tx = db.transaction()?;
    for claim in claims {
        tx.execute(
            "INSERT INTO ownership VALUES (?1,?2,?3)",
            params![
                claim.resource_id,
                claim.path.to_string_lossy(),
                serde_json::to_string(&claim)?
            ],
        )?;
    }
    tx.execute(
        "INSERT OR REPLACE INTO kv VALUES ('adopted_targets',?1)",
        [serde_json::to_string(&consumed)?],
    )?;
    tx.commit()?;
    Ok(())
}
