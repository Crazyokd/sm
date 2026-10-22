use crate::{files::reject_symlink_path, model::OwnedWrite};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
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
        let db = state.connect()?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS ownership (resource TEXT,path TEXT,value TEXT,PRIMARY KEY(resource,path)); CREATE TABLE IF NOT EXISTS observations (id TEXT PRIMARY KEY,value TEXT NOT NULL,processed INTEGER DEFAULT 0);")?;
        fs::set_permissions(&state.path, fs::Permissions::from_mode(0o600))?;
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
    pub fn owned(&self) -> Result<Vec<OwnedWrite>> {
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
                    serde_json::to_string(write)?
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
    pub fn add_run<T: Serialize>(&self, operation: &str, result: &T) -> Result<()> {
        let mut records: Vec<RunRecord> = self.get("runs")?;
        records.push(RunRecord {
            operation: operation.into(),
            time: chrono::Utc::now().to_rfc3339(),
            result: serde_json::to_value(result)?,
        });
        if records.len() > 100 {
            records.drain(..records.len() - 100);
        }
        self.set("runs", &records)
    }
}
