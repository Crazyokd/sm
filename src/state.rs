use crate::{
    files::reject_symlink_path,
    model::{OwnedWrite, StoredWrite},
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct State {
    path: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub operation: String,
    pub time: String,
    pub result: Value,
}
impl State {
    pub fn open(path: PathBuf) -> Result<Self> {
        reject_symlink_path(&path)?;
        fs::create_dir_all(path.parent().context("state path has no parent")?)?;
        let state = Self { path };
        let mut db = state.connect()?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS ownership (resource TEXT,path TEXT,value TEXT,PRIMARY KEY(resource,path)); CREATE TABLE IF NOT EXISTS observations (id TEXT PRIMARY KEY,value TEXT NOT NULL,processed INTEGER DEFAULT 0);")?;
        fs::set_permissions(&state.path, fs::Permissions::from_mode(0o600))?;
        if migrate_ownership(&mut db)? {
            db.execute_batch("VACUUM")?;
        }
        Ok(state)
    }
    pub fn directory(&self) -> Result<&Path> {
        self.path.parent().context("state path has no parent")
    }
    pub fn connect(&self) -> Result<Connection> {
        let db = Connection::open(&self.path)?;
        db.busy_timeout(Duration::from_secs(5))?;
        Ok(db)
    }
    pub fn get<T: DeserializeOwned + Default>(&self, key: &str) -> Result<T> {
        let text: Option<String> = self
            .connect()?
            .query_row("SELECT value FROM kv WHERE key=?1", [key], |row| row.get(0))
            .optional()?;
        match text {
            Some(text) => {
                serde_json::from_str(&text).with_context(|| format!("invalid saved state: {key}"))
            }
            None => Ok(T::default()),
        }
    }
    pub fn set<T: Serialize + ?Sized>(&self, key: &str, value: &T) -> Result<()> {
        self.connect()?.execute(
            "INSERT OR REPLACE INTO kv VALUES (?1,?2)",
            params![key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }
    pub fn owned(&self) -> Result<Vec<StoredWrite>> {
        let db = self.connect()?;
        let mut query = db.prepare("SELECT value FROM ownership ORDER BY resource,path")?;
        query
            .query_map([], |row| row.get::<_, String>(0))?
            .map(|text| serde_json::from_str(&text?).context("invalid saved ownership"))
            .collect()
    }
    pub fn replace_owned(&self, path: &Path, writes: &[OwnedWrite]) -> Result<()> {
        let mut db = self.connect()?;
        let transaction = db.transaction()?;
        transaction.execute(
            "DELETE FROM ownership WHERE path=?1",
            [path.to_string_lossy().as_ref()],
        )?;
        for write in writes {
            transaction.execute(
                "INSERT INTO ownership VALUES (?1,?2,?3)",
                params![
                    write.resource_id,
                    path.to_string_lossy().as_ref(),
                    serde_json::to_string(&StoredWrite::from(write))?
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
    pub fn add_run<T: Serialize>(&self, operation: &str, result: &T) -> Result<()> {
        let result = serde_json::to_value(result)?;
        let mut db = self.connect()?;
        let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let text: Option<String> = transaction
            .query_row("SELECT value FROM kv WHERE key='runs'", [], |row| {
                row.get(0)
            })
            .optional()?;
        let mut records: Vec<RunRecord> = text
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .context("invalid saved state: runs")?
            .unwrap_or_default();
        records.push(RunRecord {
            operation: operation.into(),
            time: chrono::Utc::now().to_rfc3339(),
            result,
        });
        if records.len() > 100 {
            records.drain(..records.len() - 100);
        }
        transaction.execute(
            "INSERT OR REPLACE INTO kv VALUES ('runs',?1)",
            [serde_json::to_string(&records)?],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

fn migrate_ownership(db: &mut Connection) -> Result<bool> {
    let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if transaction.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))? >= 1 {
        return Ok(false);
    }
    let rows = {
        let mut query = transaction
            .prepare("SELECT resource,path,value FROM ownership ORDER BY resource,path")?;
        query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut converted = false;
    for (resource, path, text) in rows {
        let value: Value = serde_json::from_str(&text).context("invalid saved ownership")?;
        let legacy = value["body"]["kind"] == "file";
        let write: StoredWrite = if legacy {
            let write: OwnedWrite =
                serde_json::from_value(value).context("invalid saved ownership")?;
            StoredWrite::from(&write)
        } else {
            serde_json::from_value(value).context("invalid saved ownership")?
        };
        ensure!(
            write.resource_id == resource && write.path == Path::new(&path),
            "invalid saved ownership identity"
        );
        if legacy {
            transaction.execute(
                "UPDATE ownership SET value=?1 WHERE resource=?2 AND path=?3",
                params![serde_json::to_string(&write)?, resource, path],
            )?;
            converted = true;
        }
    }
    transaction.pragma_update(None, "user_version", 1)?;
    transaction.commit()?;
    Ok(converted)
}
