use crate::{
    files::reject_symlink_path,
    model::{Client, Resource},
    privacy::{redact, redact_text},
    state::State,
    wire::{digest, legacy_json},
};
use anyhow::{Context, Result, ensure};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Seek, SeekFrom, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Coverage {
    Observed,
    #[default]
    Partial,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceKind {
    Execution,
    #[default]
    Inferred,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    pub agent: Client,
    pub session_id: String,
    pub cwd: String,
    pub event: String,
    pub timestamp: String,
    #[serde(default)]
    pub resource_id: Option<String>,
    #[serde(default)]
    pub resource_content_hash: Option<String>,
    #[serde(default)]
    pub evidence_kind: EvidenceKind,
    #[serde(default)]
    pub facts: Map<String, Value>,
    #[serde(default)]
    pub coverage: Coverage,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActiveIndex {
    pub owned: Vec<ActiveOwned>,
    pub hashes: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveOwned {
    pub resource_id: String,
    pub path: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingError {
    pub file: String,
    pub offset: u64,
    pub error: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HookPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hook_event_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resource_id: Option<String>,
    #[serde(flatten)]
    facts: Map<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Envelope {
    id: String,
    agent: Client,
    timestamp: String,
    payload: HookPayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resource_content_hash: Option<String>,
}
fn ascii_json(value: &Value) -> Result<String> {
    let value = legacy_json(value)?;
    let mut encoded = String::new();
    for character in value.chars() {
        if character.is_ascii() {
            encoded.push(character);
        } else {
            let mut units = [0; 2];
            for unit in character.encode_utf16(&mut units) {
                encoded.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    Ok(encoded)
}
pub fn record_event(agent: Client, payload: &Value, directory: &Path) -> Result<()> {
    if std::env::var("SM_INTERNAL_RUN").as_deref() == Ok("1") {
        return Ok(());
    }
    let _: HookPayload =
        serde_json::from_value(payload.clone()).context("invalid recorder payload")?;
    let mut payload = payload
        .as_object()
        .context("event payload must be an object")?
        .clone();
    for key in ["tool_output", "tool_response"] {
        if let Some(output) = payload.remove(key) {
            payload.insert("result_hash".into(), json!(digest(ascii_json(&output)?)));
        }
    }
    let event = payload
        .get("hook_event_name")
        .or_else(|| payload.get("event"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let native = payload
        .get("event_id")
        .or_else(|| payload.get("tool_call_id"))
        .or_else(|| payload.get("tool_use_id"));
    let identity = match native {
        Some(native) if !native.is_null() => format!(
            "{agent}:{}:{event}:{}",
            payload
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            native
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| native.to_string())
        ),
        _ => uuid::Uuid::new_v4().to_string(),
    };
    let mut envelope = Envelope {
        id: digest(identity.as_bytes()),
        agent,
        timestamp: chrono::Utc::now().to_rfc3339(),
        payload: serde_json::from_value(redact(&Value::Object(payload.clone()), Some(4000)))?,
        resource_id: None,
        resource_content_hash: None,
    };
    let active = directory.join("active.json");
    if active.is_file() {
        let index: ActiveIndex = serde_json::from_slice(&fs::read(active)?)?;
        let input = serde_json::to_string(payload.get("tool_input").unwrap_or(&Value::Null))?;
        for owned in index.owned {
            if input.contains(owned.path.to_string_lossy().as_ref()) {
                envelope.resource_id = Some(owned.resource_id.clone());
                envelope.resource_content_hash = index.hashes.get(&owned.resource_id).cloned();
                break;
            }
        }
    }
    let mut encoded = serde_json::to_vec(&envelope)?;
    if encoded.len() > 32768 {
        let original = serde_json::to_value(&envelope.payload)?;
        let mut limited = Map::new();
        for key in [
            "session_id",
            "cwd",
            "hook_event_name",
            "tool_name",
            "resource_id",
        ] {
            if let Some(value) = original.get(key) {
                limited.insert(key.into(), value.clone());
            }
        }
        limited.insert("truncated".into(), Value::Bool(true));
        envelope.payload = serde_json::from_value(Value::Object(limited))?;
        encoded = serde_json::to_vec(&envelope)?;
    }
    encoded.push(b'\n');
    let folder = directory.join("events");
    reject_symlink_path(&folder)?;
    fs::create_dir_all(&folder)?;
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o700))?;
    let path = folder.join(format!("{}.jsonl", chrono::Utc::now().date_naive()));
    reject_symlink_path(&path)?;
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)?;
    ensure!(
        file.write(&encoded)? == encoded.len(),
        "incomplete observation append"
    );
    Ok(())
}
pub fn ingest_events(directory: &Path, state: &State) -> Result<Vec<Observation>> {
    let resources: Vec<Resource> = state.get("resources")?;
    let resources: BTreeMap<_, _> = resources.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut offsets: BTreeMap<String, u64> = state.get("event_offsets")?;
    let mut errors: Vec<RecordingError> = state.get("recording_errors")?;
    let mut added = Vec::new();
    let folder = directory.join("events");
    let mut paths = Vec::new();
    if folder.is_dir() {
        for entry in fs::read_dir(folder)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "jsonl") {
                paths.push(path);
            }
        }
    }
    paths.sort();
    let db = state.connect()?;
    for path in paths {
        reject_symlink_path(&path)?;
        let key = path.to_string_lossy().into_owned();
        let offset = offsets.get(&key).copied().unwrap_or_default();
        let mut file = BufReader::new(fs::File::open(&path)?);
        if offset <= file.get_ref().metadata()?.len() {
            file.seek(SeekFrom::Start(offset))?;
        }
        loop {
            let before = file.stream_position()?;
            let mut line = Vec::new();
            if file.read_until(b'\n', &mut line)? == 0 {
                break;
            }
            if line.last() != Some(&b'\n') {
                file.seek(SeekFrom::Start(before))?;
                break;
            }
            let parse = || -> Result<Option<Observation>> {
                let envelope: Envelope = serde_json::from_slice(&line)?;
                let mut resource_id = envelope
                    .resource_id
                    .clone()
                    .or_else(|| envelope.payload.resource_id.clone());
                if resource_id
                    .as_deref()
                    .and_then(|id| resources.get(id))
                    .is_some_and(|r| !r.source.is_self())
                {
                    return Ok(None);
                }
                if !resource_id
                    .as_deref()
                    .is_some_and(|id| resources.contains_key(id))
                {
                    resource_id = None;
                }
                let facts = serde_json::to_value(&envelope.payload)?
                    .as_object()
                    .context("event facts must be an object")?
                    .clone();
                let observed = resource_id.is_some();
                Ok(Some(Observation {
                    id: envelope.id,
                    agent: envelope.agent,
                    session_id: envelope.payload.session_id.unwrap_or_default(),
                    cwd: envelope.payload.cwd.unwrap_or_default(),
                    event: envelope
                        .payload
                        .hook_event_name
                        .or(envelope.payload.event)
                        .unwrap_or_else(|| "unknown".into()),
                    timestamp: envelope.timestamp,
                    resource_id,
                    resource_content_hash: envelope.resource_content_hash,
                    evidence_kind: if observed {
                        EvidenceKind::Execution
                    } else {
                        EvidenceKind::Inferred
                    },
                    coverage: if observed {
                        Coverage::Observed
                    } else {
                        Coverage::Partial
                    },
                    facts,
                }))
            };
            match parse() {
                Ok(Some(row)) => {
                    if db.execute(
                        "INSERT OR IGNORE INTO observations(id,value) VALUES (?1,?2)",
                        params![row.id, serde_json::to_string(&row)?],
                    )? > 0
                    {
                        added.push(row);
                    }
                }
                Ok(None) => {}
                Err(error) => errors.push(RecordingError {
                    file: path
                        .file_name()
                        .context("event filename missing")?
                        .to_string_lossy()
                        .into_owned(),
                    offset: before,
                    error: redact_text(&error.to_string(), Some(300)),
                }),
            }
        }
        offsets.insert(key, file.stream_position()?);
    }
    if errors.len() > 100 {
        errors.drain(..errors.len() - 100);
    }
    state.set("event_offsets", &offsets)?;
    state.set("recording_errors", &errors)?;
    Ok(added)
}
pub fn pending_observations(state: &State, limit: usize) -> Result<Vec<Observation>> {
    let db = state.connect()?;
    let mut statement =
        db.prepare("SELECT value FROM observations WHERE processed=0 ORDER BY rowid LIMIT ?1")?;
    statement
        .query_map([i64::try_from(limit)?], |row| row.get::<_, String>(0))?
        .map(|text| Ok(serde_json::from_str(&text?)?))
        .collect()
}
pub fn complete_observations(state: &State, ids: &[String]) -> Result<()> {
    let mut db = state.connect()?;
    let transaction = db.transaction()?;
    for id in ids {
        transaction.execute("UPDATE observations SET processed=1 WHERE id=?1", [id])?;
    }
    transaction.commit()?;
    Ok(())
}
