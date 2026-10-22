use crate::{
    files::reject_symlink_path,
    observe::{EventStamp, effective_offset, event_lock, event_stamp},
    proposals::Proposal,
    service::Finding,
    state::State,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
};

pub const RETENTION_DAYS: u64 = 14;
pub const RAW_LIMIT_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageTotal {
    pub events: u64,
    pub last_seen: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionHealth {
    pub retention_days: u64,
    pub raw_limit_bytes: u64,
    pub raw_files: u64,
    pub raw_bytes: u64,
    pub protected_raw_files: u64,
    pub protected_raw_bytes: u64,
    pub pending_raw_files: u64,
    pub pending_raw_bytes: u64,
    pub unknown_raw_files: u64,
    pub observations: u64,
    pub pending_observations: u64,
    pub pinned_observations: u64,
    pub unknown_timestamp_observations: u64,
    pub retired_observations: u64,
    pub over_budget: bool,
    pub over_budget_bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionReport {
    pub before: RetentionHealth,
    pub after: RetentionHealth,
    pub removed_raw_files: u64,
    pub removed_raw_bytes: u64,
    pub removed_observations: u64,
}
struct RawFile {
    path: PathBuf,
    bytes: u64,
    date: Option<NaiveDate>,
    protected: bool,
}
struct StoredEvent {
    id: String,
    processed: bool,
    resource_id: Option<String>,
    timestamp: Option<String>,
}
struct Inventory {
    health: RetentionHealth,
    raw: Vec<RawFile>,
    rows: Vec<StoredEvent>,
    pins: BTreeSet<String>,
}
fn saved<T: DeserializeOwned + Default>(db: &Connection, key: &str) -> Result<T> {
    let text: Option<String> = db
        .query_row("SELECT value FROM kv WHERE key=?1", [key], |row| row.get(0))
        .optional()?;
    text.map_or_else(
        || Ok(T::default()),
        |text| serde_json::from_str(&text).with_context(|| format!("invalid saved state: {key}")),
    )
}
fn pinned(db: &Connection) -> Result<BTreeSet<String>> {
    let proposals: BTreeMap<String, Proposal> = saved(db, "proposals")?;
    let findings: Vec<Finding> = saved(db, "findings")?;
    Ok(proposals
        .into_values()
        .flat_map(|proposal| proposal.change.evidence)
        .chain(findings.into_iter().flat_map(|finding| finding.evidence))
        .flat_map(|text| {
            text.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_')
                .filter(|token| !token.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect())
}
fn timestamp(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.with_timezone(&Utc))
        .or_else(|| {
            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .ok()?
                .and_hms_opt(0, 0, 0)
                .map(|time| time.and_utc())
        })
}
fn inventory(state: &State, db: &Connection) -> Result<Inventory> {
    let offsets: BTreeMap<String, u64> = saved(db, "event_offsets")?;
    let stamps: BTreeMap<String, EventStamp> = saved(db, "event_stamps")?;
    let pins = pinned(db)?;
    let mut health = RetentionHealth {
        retention_days: RETENTION_DAYS,
        raw_limit_bytes: RAW_LIMIT_BYTES,
        raw_files: 0,
        raw_bytes: 0,
        protected_raw_files: 0,
        protected_raw_bytes: 0,
        pending_raw_files: 0,
        pending_raw_bytes: 0,
        unknown_raw_files: 0,
        observations: 0,
        pending_observations: 0,
        pinned_observations: 0,
        unknown_timestamp_observations: 0,
        retired_observations: 0,
        over_budget: false,
        over_budget_bytes: 0,
    };
    let mut raw = Vec::new();
    let folder = state.directory()?.join("events");
    reject_symlink_path(&folder)?;
    if folder.is_dir() {
        for entry in fs::read_dir(folder)? {
            let path = entry?.path();
            if path
                .extension()
                .is_none_or(|extension| extension != "jsonl")
            {
                continue;
            }
            reject_symlink_path(&path)?;
            let metadata = fs::metadata(&path)?;
            if !metadata.is_file() {
                continue;
            }
            let bytes = metadata.len();
            let date = path
                .file_stem()
                .and_then(|name| name.to_str())
                .and_then(|name| NaiveDate::parse_from_str(name, "%Y-%m-%d").ok());
            let closed = date.is_some_and(|date| date < Utc::now().date_naive());
            let mut newline = bytes == 0;
            if bytes > 0 {
                let mut file = fs::File::open(&path)?;
                file.seek(SeekFrom::End(-1))?;
                let mut tail = [0];
                file.read_exact(&mut tail)?;
                newline = tail == [b'\n'];
            }
            let key = path.to_string_lossy();
            let current = event_stamp(&metadata);
            let offset = effective_offset(
                offsets.get(key.as_ref()).copied().unwrap_or_default(),
                stamps.get(key.as_ref()),
                &current,
                bytes,
            );
            let pending = offset != bytes || !newline || stamps.get(key.as_ref()) != Some(&current);
            let protected = !closed || pending;
            health.raw_files += 1;
            health.raw_bytes += bytes;
            health.pending_raw_files += u64::from(pending);
            health.pending_raw_bytes += bytes - offset;
            health.unknown_raw_files +=
                u64::from(date.is_none_or(|date| date > Utc::now().date_naive()));
            if protected {
                health.protected_raw_files += 1;
                health.protected_raw_bytes += bytes;
            }
            raw.push(RawFile {
                path,
                bytes,
                date,
                protected,
            });
        }
    }
    raw.sort_by_key(|file| file.date);
    let mut query = db.prepare("SELECT id,processed,json_extract(value,'$.resource_id'),json_extract(value,'$.timestamp') FROM observations")?;
    let rows: Vec<_> = query
        .query_map([], |row| {
            Ok(StoredEvent {
                id: row.get(0)?,
                processed: row.get::<_, i64>(1)? != 0,
                resource_id: row.get(2)?,
                timestamp: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    for row in &rows {
        health.observations += 1;
        health.pending_observations += u64::from(!row.processed);
        health.pinned_observations += u64::from(pins.contains(&row.id));
        health.unknown_timestamp_observations +=
            u64::from(row.timestamp.as_deref().and_then(timestamp).is_none());
    }
    let retired_exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='retired_observations')", [], |row| row.get(0))?;
    if retired_exists {
        health.retired_observations =
            db.query_row("SELECT COUNT(*) FROM retired_observations", [], |row| {
                row.get(0)
            })?;
    }
    health.over_budget_bytes = health.raw_bytes.saturating_sub(RAW_LIMIT_BYTES);
    health.over_budget = health.over_budget_bytes > 0;
    Ok(Inventory {
        health,
        raw,
        rows,
        pins,
    })
}
pub fn summary(state: &State) -> Result<RetentionHealth> {
    Ok(inventory(state, &state.connect()?)?.health)
}
pub fn maintain(state: &State) -> Result<RetentionReport> {
    let mut db = state.connect()?;
    let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction
        .execute_batch("CREATE TABLE IF NOT EXISTS retired_observations (id TEXT PRIMARY KEY)")?;
    let before = inventory(state, &transaction)?;
    let cutoff = Utc::now() - Duration::days(RETENTION_DAYS as i64);
    let mut totals: BTreeMap<String, UsageTotal> = saved(&transaction, "usage_totals")?;
    let mut removed_observations = 0;
    for row in before.rows {
        if !row.processed
            || before.pins.contains(&row.id)
            || row
                .timestamp
                .as_deref()
                .and_then(timestamp)
                .is_none_or(|time| time >= cutoff)
        {
            continue;
        }
        // Compact ID tombstones prevent replayed or recreated raw files from restoring retired events.
        transaction.execute(
            "INSERT OR IGNORE INTO retired_observations(id) VALUES (?1)",
            [&row.id],
        )?;
        transaction.execute("DELETE FROM observations WHERE id=?1", [&row.id])?;
        removed_observations += 1;
        if let Some(id) = row.resource_id {
            let total = totals.entry(id).or_default();
            total.events = total.events.saturating_add(1);
            if total.last_seen.as_deref().and_then(timestamp)
                < row.timestamp.as_deref().and_then(timestamp)
            {
                total.last_seen = row.timestamp;
            }
        }
    }
    transaction.execute(
        "INSERT OR REPLACE INTO kv(key,value) VALUES ('usage_totals',?1)",
        [serde_json::to_string(&totals)?],
    )?;
    let mut raw_bytes = before.health.raw_bytes;
    let mut removed_raw_files = 0;
    let mut removed_raw_bytes = 0;
    let _raw_lock = event_lock(state.directory()?)?;
    let stamps: BTreeMap<String, EventStamp> = saved(&transaction, "event_stamps")?;
    for raw in before.raw {
        if !raw.protected
            && (raw.date.is_some_and(|date| date < cutoff.date_naive())
                || raw_bytes > RAW_LIMIT_BYTES)
            && stamps.get(raw.path.to_string_lossy().as_ref())
                == Some(&event_stamp(&fs::metadata(&raw.path)?))
        {
            fs::remove_file(raw.path)?;
            removed_raw_files += 1;
            removed_raw_bytes += raw.bytes;
            raw_bytes -= raw.bytes;
        }
    }
    drop(_raw_lock);
    transaction.commit()?;
    if removed_observations > 0 {
        db.execute_batch("VACUUM")?;
    }
    Ok(RetentionReport {
        before: before.health,
        after: inventory(state, &db)?.health,
        removed_raw_files,
        removed_raw_bytes,
        removed_observations,
    })
}
