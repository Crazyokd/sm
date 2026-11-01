use crate::{
    files::{atomic_write, reject_symlink_path},
    model::{Catalog, Client, valid_id},
    privacy::{redact, redact_text},
    state::State,
    wire::digest,
};
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Coverage {
    Observed,
    Partial,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    FileRead,
    NativeActivation,
    ToolExecution,
    Unknown,
    // Existing saved observations retain their original evidence labels.
    Execution,
    Inferred,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum ResourceSignal {
    Activation,
    Load,
    ToolExecution,
    Completion,
    Failure,
    #[default]
    Unknown,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum DetectionSource {
    Explicit,
    ActivePath,
    #[default]
    Unattributed,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
    #[default]
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    pub agent: Client,
    pub session_id: String,
    pub cwd: String,
    pub event: String,
    pub timestamp: String,
    pub resource_id: Option<String>,
    pub resource_content_hash: Option<String>,
    pub evidence_kind: EvidenceKind,
    pub signal: ResourceSignal,
    pub detected_from: DetectionSource,
    pub confidence: Confidence,
    pub turn_id: Option<String>,
    pub invocation_id: Option<String>,
    pub parent_invocation_id: Option<String>,
    pub facts: Map<String, Value>,
    pub coverage: Coverage,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub resource_id: String,
    pub observations: u64,
    pub activations: u64,
    pub loads: u64,
    pub tool_executions: u64,
    pub completions: u64,
    pub failures: u64,
    pub high_confidence: u64,
    pub medium_confidence: u64,
    pub low_confidence: u64,
    pub last_seen: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceUsageSummary {
    pub window_days: u64,
    pub since: String,
    pub observations: u64,
    pub attributed_observations: u64,
    pub unattributed_observations: u64,
    pub high_confidence: u64,
    pub medium_confidence: u64,
    pub low_confidence: u64,
    pub resources: Vec<ResourceUsage>,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EventStamp {
    pub device: u64,
    pub inode: u64,
    pub modified_seconds: i64,
    pub modified_nanoseconds: i64,
    pub changed_seconds: i64,
    pub changed_nanoseconds: i64,
}
pub(crate) fn event_stamp(metadata: &fs::Metadata) -> EventStamp {
    EventStamp {
        device: metadata.dev(),
        inode: metadata.ino(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    }
}
pub(crate) fn effective_offset(
    saved: u64,
    previous: Option<&EventStamp>,
    current: &EventStamp,
    bytes: u64,
) -> u64 {
    if saved > bytes
        || previous.is_none_or(|previous| {
            previous.device != current.device
                || previous.inode != current.inode
                || (previous != current && saved >= bytes)
        })
    {
        0
    } else {
        saved
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientCaptureHealth {
    pub received: u64,
    pub last_event: Option<String>,
    pub failures: u64,
    pub truncated: u64,
    pub gaps: u64,
    pub last_error: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CaptureHealth {
    pub clients: BTreeMap<Client, ClientCaptureHealth>,
}
pub fn capture_health(directory: &Path) -> Result<CaptureHealth> {
    let mut clients = BTreeMap::new();
    for client in Client::ALL {
        let path = directory.join("capture").join(format!("{client}.json"));
        reject_symlink_path(&path)?;
        let health = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("invalid capture health")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ClientCaptureHealth::default()
            }
            Err(error) => return Err(error.into()),
        };
        clients.insert(client, health);
    }
    Ok(CaptureHealth { clients })
}
fn update_health(
    agent: Client,
    directory: &Path,
    truncated: bool,
    gap: bool,
    error: Option<&str>,
) -> Result<()> {
    if internal_run() {
        return Ok(());
    }
    let folder = directory.join("capture");
    reject_symlink_path(&folder)?;
    fs::create_dir_all(&folder)?;
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o700))?;
    let lock_path = folder.join(format!("{agent}.lock"));
    reject_symlink_path(&lock_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(lock_path)?;
    lock.lock_exclusive()?;
    let path = folder.join(format!("{agent}.json"));
    reject_symlink_path(&path)?;
    let mut health: ClientCaptureHealth = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("invalid capture health")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            ClientCaptureHealth::default()
        }
        Err(error) => return Err(error.into()),
    };
    health.received = health.received.saturating_add(1);
    health.last_event = Some(chrono::Utc::now().to_rfc3339());
    health.truncated = health.truncated.saturating_add(u64::from(truncated));
    health.gaps = health
        .gaps
        .saturating_add(u64::from(gap || error.is_some()));
    if let Some(error) = error {
        health.failures = health.failures.saturating_add(1);
        health.last_error = Some(redact_text(error, Some(240)));
    }
    atomic_write(&path, &serde_json::to_vec(&health)?, None)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}
pub fn record_failure(agent: Client, directory: &Path, error: &str) -> Result<()> {
    update_health(agent, directory, false, true, Some(error))
}
fn internal_run() -> bool {
    std::env::var("SM_INTERNAL_RUN").as_deref() == Ok("1")
}
pub(crate) fn event_lock(directory: &Path) -> Result<fs::File> {
    let folder = directory.join("events");
    reject_symlink_path(&folder)?;
    fs::create_dir_all(&folder)?;
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o700))?;
    let path = folder.join(".lock");
    reject_symlink_path(&path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(path)?;
    lock.lock_exclusive()?;
    Ok(lock)
}

fn normalize_result(payload: &mut Map<String, Value>) -> Result<()> {
    let mut results = Map::new();
    for key in [
        "tool_output",
        "tool_response",
        "tool_result",
        "result",
        "output",
        "stdout",
        "stderr",
    ] {
        if let Some(value) = payload.remove(key) {
            results.insert(key.into(), value);
        }
    }
    let sources: Vec<_> = std::iter::once(&*payload)
        .chain(results.values().filter_map(Value::as_object))
        .collect();
    let field = |key: &str| sources.iter().find_map(|source| source.get(key));
    let status = field("status")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase);
    let exit_code = field("exit_code")
        .or_else(|| field("exitCode"))
        .and_then(Value::as_i64);
    let outcome = match status.as_deref() {
        Some("success" | "succeeded" | "completed" | "ok") => Some("success"),
        Some("failed" | "failure" | "error") => Some("failure"),
        Some("timeout" | "timed_out") => Some("timeout"),
        Some("cancelled" | "canceled") => Some("cancelled"),
        _ => field("success")
            .and_then(Value::as_bool)
            .map(|value| if value { "success" } else { "failure" })
            .or_else(|| {
                field("is_error")
                    .and_then(Value::as_bool)
                    .map(|value| if value { "failure" } else { "success" })
            })
            .or_else(|| exit_code.map(|code| if code == 0 { "success" } else { "failure" })),
    }
    .or_else(|| {
        (field("hook_event_name")
            .or_else(|| field("event"))
            .and_then(Value::as_str)
            == Some("PostToolUseFailure"))
        .then_some("failure")
    });
    let duration = field("duration_ms")
        .filter(|value| value.as_f64().is_some_and(|n| n >= 0.0))
        .cloned();
    let error = field("error")
        .and_then(|value| {
            value
                .as_str()
                .or_else(|| value.get("message").and_then(Value::as_str))
        })
        .or_else(|| field("stderr").and_then(Value::as_str))
        .or_else(|| results.get("stderr").and_then(Value::as_str))
        .filter(|text| !text.is_empty())
        .map(|text| redact_text(text, Some(240)));
    // Output bodies remain hash-only; only an explicitly supplied summary may carry text.
    let summary = field("summary")
        .and_then(Value::as_str)
        .map(|text| redact_text(text, Some(240)));
    for key in [
        "status",
        "success",
        "is_error",
        "exit_code",
        "exitCode",
        "duration_ms",
        "error",
        "summary",
        "outcome",
        "output_summary",
        "error_summary",
    ] {
        payload.remove(key);
    }
    if let Some(outcome) = outcome {
        payload.insert("outcome".into(), json!(outcome));
        if let Some(status) = status.filter(|status| {
            matches!(
                status.as_str(),
                "success"
                    | "succeeded"
                    | "completed"
                    | "ok"
                    | "failed"
                    | "failure"
                    | "error"
                    | "timeout"
                    | "timed_out"
                    | "cancelled"
                    | "canceled"
            )
        }) {
            payload.insert("status".into(), json!(status));
        }
    }
    if let Some(code) = exit_code {
        payload.insert("exit_code".into(), json!(code));
    }
    if let Some(duration) = duration {
        payload.insert("duration_ms".into(), duration);
    }
    if let Some(error) = error {
        payload.insert("error_summary".into(), json!(error));
    }
    if let Some(summary) = summary {
        payload.insert("output_summary".into(), json!(summary));
    }
    if !results.is_empty() {
        let bytes = if results.len() == 1 {
            serde_json::to_vec(results.values().next().context("missing result")?)?
        } else {
            serde_json::to_vec(&results)?
        };
        payload
            .entry("output_summary")
            .or_insert_with(|| json!(format!("output received ({} bytes)", bytes.len())));
        payload.insert("result_hash".into(), json!(digest(bytes)));
    }
    Ok(())
}
fn evidence_kind(payload: &HookPayload) -> EvidenceKind {
    let event = payload
        .hook_event_name
        .as_deref()
        .or(payload.event.as_deref())
        .unwrap_or_default();
    if matches!(
        event,
        "SkillActivated" | "SkillInvocation" | "skill_activated" | "skill_activation"
    ) {
        return EvidenceKind::NativeActivation;
    }
    if matches!(event, "FileRead" | "file_read") {
        return EvidenceKind::FileRead;
    }
    if matches!(
        event,
        "PostToolUse" | "PostToolUseFailure" | "ToolResult" | "tool_result"
    ) {
        return match payload.facts.get("tool_name").and_then(Value::as_str) {
            Some("Read" | "read_file" | "read") => EvidenceKind::FileRead,
            Some("Skill" | "skill") => EvidenceKind::NativeActivation,
            _ => EvidenceKind::ToolExecution,
        };
    }
    EvidenceKind::Unknown
}
fn fact_string(payload: &HookPayload, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| payload.facts.get(*key).and_then(Value::as_str))
        .filter(|value| !value.is_empty() && value.len() <= 240)
        .map(str::to_owned)
}
fn resource_signal(payload: &HookPayload) -> ResourceSignal {
    let event = payload
        .hook_event_name
        .as_deref()
        .or(payload.event.as_deref())
        .unwrap_or_default();
    if let Some(signal) = payload.facts.get("resource_event").and_then(Value::as_str) {
        return match signal {
            "activation" => ResourceSignal::Activation,
            "load" => ResourceSignal::Load,
            "tool" | "tool_execution" => ResourceSignal::ToolExecution,
            "completion" => ResourceSignal::Completion,
            "failure" => ResourceSignal::Failure,
            _ => ResourceSignal::Unknown,
        };
    }
    match event {
        "SkillActivated" | "SkillInvocation" | "skill_activated" | "skill_activation" => {
            ResourceSignal::Activation
        }
        "ResourceLoaded" | "resource_loaded" | "FileRead" | "file_read" => ResourceSignal::Load,
        "ResourceCompleted" | "resource_completed" => ResourceSignal::Completion,
        "PostToolUseFailure" | "post_tool_use_failure" => ResourceSignal::Failure,
        "PostToolUse" | "ToolResult" | "tool_result" => {
            if matches!(
                payload.facts.get("tool_name").and_then(Value::as_str),
                Some("Skill" | "skill")
            ) {
                ResourceSignal::Activation
            } else if payload.facts.get("outcome").and_then(Value::as_str) == Some("failure") {
                ResourceSignal::Failure
            } else {
                ResourceSignal::ToolExecution
            }
        }
        _ => ResourceSignal::Unknown,
    }
}
fn confidence(source: DetectionSource, signal: ResourceSignal) -> Confidence {
    match source {
        DetectionSource::Explicit => match signal {
            ResourceSignal::Activation
            | ResourceSignal::Load
            | ResourceSignal::Completion
            | ResourceSignal::Failure => Confidence::High,
            ResourceSignal::ToolExecution | ResourceSignal::Unknown => Confidence::Medium,
        },
        DetectionSource::ActivePath => Confidence::Low,
        DetectionSource::Unattributed => Confidence::Unknown,
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HookPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hook_event_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    resource_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource_content_hash: Option<String>,
    detected_from: DetectionSource,
}
pub fn record_event(agent: Client, payload: &Value, directory: &Path) -> Result<()> {
    if internal_run() {
        return Ok(());
    }
    let _: HookPayload =
        serde_json::from_value(payload.clone()).context("invalid recorder payload")?;
    let mut payload = payload
        .as_object()
        .context("event payload must be an object")?
        .clone();
    normalize_result(&mut payload)?;
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
    let safe_payload = redact(&Value::Object(payload.clone()), Some(4000));
    let mut truncated = payload
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || serde_json::to_vec(&safe_payload)?.len()
            < serde_json::to_vec(&redact(&Value::Object(payload.clone()), None))?.len();
    let mut envelope = Envelope {
        id: digest(identity.as_bytes()),
        agent,
        timestamp: chrono::Utc::now().to_rfc3339(),
        payload: serde_json::from_value(safe_payload)?,
        resource_id: payload
            .get("resource_id")
            .and_then(Value::as_str)
            .filter(|id| id.len() <= 240 && valid_id(id))
            .map(|id| redact_text(id, Some(240))),
        resource_content_hash: None,
        detected_from: DetectionSource::Unattributed,
    };
    if envelope.resource_id.is_some() {
        envelope.detected_from = DetectionSource::Explicit;
    }
    let active = directory.join("active.json");
    reject_symlink_path(&active)?;
    if active.is_file() {
        let index: ActiveIndex = serde_json::from_slice(&fs::read(active)?)?;
        let input = serde_json::to_string(payload.get("tool_input").unwrap_or(&Value::Null))?;
        envelope.resource_content_hash = envelope
            .resource_id
            .as_ref()
            .and_then(|id| index.hashes.get(id))
            .cloned();
        if envelope.resource_id.is_none() {
            let matches: std::collections::BTreeSet<_> = index
                .owned
                .iter()
                .filter(|owned| input.contains(owned.path.to_string_lossy().as_ref()))
                .map(|owned| &owned.resource_id)
                .collect();
            if matches.len() == 1 {
                let id = matches.into_iter().next().expect("one matched resource");
                envelope.resource_id = Some(id.clone());
                envelope.resource_content_hash = index.hashes.get(id).cloned();
                envelope.detected_from = DetectionSource::ActivePath;
            }
        }
    }
    let mut encoded = serde_json::to_vec(&envelope)?;
    if encoded.len() > 32768 {
        truncated = true;
        let original = serde_json::to_value(&envelope.payload)?;
        let mut limited = Map::new();
        for key in [
            "session_id",
            "cwd",
            "hook_event_name",
            "tool_name",
            "resource_id",
            "event",
            "outcome",
            "exit_code",
            "duration_ms",
            "output_summary",
            "error_summary",
            "result_hash",
        ] {
            if let Some(value) = original.get(key) {
                limited.insert(key.into(), redact(value, Some(256)));
            }
        }
        limited.insert("truncated".into(), Value::Bool(true));
        envelope.payload = serde_json::from_value(Value::Object(limited))?;
        encoded = serde_json::to_vec(&envelope)?;
    }
    encoded.push(b'\n');
    let _lock = event_lock(directory)?;
    let folder = directory.join("events");
    let path = folder.join(format!("{}.jsonl", chrono::Utc::now().date_naive()));
    reject_symlink_path(&path)?;
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(&path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    ensure!(
        file.write(&encoded)? == encoded.len(),
        "incomplete observation append"
    );
    drop(_lock);
    update_health(agent, directory, truncated, event == "CaptureGap", None)?;
    Ok(())
}
pub fn ingest_events(directory: &Path, state: &State) -> Result<Vec<Observation>> {
    let catalog: Catalog = state.get("catalog")?;
    let resources = catalog.resources;
    let resources: BTreeMap<_, _> = resources.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut offsets: BTreeMap<String, u64> = state.get("event_offsets")?;
    let mut stamps: BTreeMap<String, EventStamp> = state.get("event_stamps")?;
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
    db.execute_batch("CREATE TABLE IF NOT EXISTS retired_observations (id TEXT PRIMARY KEY)")?;
    for path in paths {
        reject_symlink_path(&path)?;
        let key = path.to_string_lossy().into_owned();
        let mut file = BufReader::new(fs::File::open(&path)?);
        let metadata = file.get_ref().metadata()?;
        let current = event_stamp(&metadata);
        let offset = effective_offset(
            offsets.get(&key).copied().unwrap_or_default(),
            stamps.get(&key),
            &current,
            metadata.len(),
        );
        if offset <= metadata.len() {
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
                let evidence_kind = evidence_kind(&envelope.payload);
                let signal = resource_signal(&envelope.payload);
                let detected_from = if observed {
                    envelope.detected_from
                } else {
                    DetectionSource::Unattributed
                };
                let confidence = confidence(detected_from, signal);
                let session_id = envelope.payload.session_id.clone().unwrap_or_default();
                let cwd = envelope.payload.cwd.clone().unwrap_or_default();
                let event = envelope
                    .payload
                    .hook_event_name
                    .clone()
                    .or_else(|| envelope.payload.event.clone())
                    .unwrap_or_else(|| "unknown".into());
                Ok(Some(Observation {
                    id: envelope.id,
                    agent: envelope.agent,
                    session_id,
                    cwd,
                    event,
                    timestamp: envelope.timestamp,
                    resource_id,
                    resource_content_hash: envelope.resource_content_hash,
                    evidence_kind,
                    signal,
                    detected_from,
                    confidence,
                    turn_id: fact_string(&envelope.payload, &["turn_id", "turnId"]),
                    invocation_id: fact_string(
                        &envelope.payload,
                        &[
                            "invocation_id",
                            "skill_invocation_id",
                            "tool_use_id",
                            "tool_call_id",
                        ],
                    ),
                    parent_invocation_id: fact_string(
                        &envelope.payload,
                        &["parent_invocation_id", "parent_id"],
                    ),
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
                        "INSERT OR IGNORE INTO observations(id,value) SELECT ?1,?2 WHERE NOT EXISTS (SELECT 1 FROM retired_observations WHERE id=?1)",
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
        offsets.insert(key.clone(), file.stream_position()?);
        stamps.insert(key, event_stamp(&file.get_ref().metadata()?));
    }
    if errors.len() > 100 {
        errors.drain(..errors.len() - 100);
    }
    state.set("event_offsets", &offsets)?;
    state.set("event_stamps", &stamps)?;
    state.set("recording_errors", &errors)?;
    Ok(added)
}
pub fn resource_usage_summary(
    state: &State,
    window: chrono::Duration,
) -> Result<ResourceUsageSummary> {
    let now = chrono::Utc::now();
    let cutoff = now - window;
    let mut summary = ResourceUsageSummary {
        window_days: window.num_days().max(0) as u64,
        since: cutoff.to_rfc3339(),
        ..Default::default()
    };
    let db = state.connect()?;
    let mut statement = db.prepare("SELECT value FROM observations")?;
    let mut resources = BTreeMap::<String, ResourceUsage>::new();
    for text in statement.query_map([], |row| row.get::<_, String>(0))? {
        let observation: Observation = serde_json::from_str(&text?)?;
        let timestamp = match chrono::DateTime::parse_from_rfc3339(&observation.timestamp) {
            Ok(timestamp) if timestamp.with_timezone(&chrono::Utc) >= cutoff => timestamp,
            _ => continue,
        };
        let signal = observation.signal;
        let observation_confidence = observation.confidence;
        summary.observations = summary.observations.saturating_add(1);
        match observation_confidence {
            Confidence::High => summary.high_confidence = summary.high_confidence.saturating_add(1),
            Confidence::Medium => {
                summary.medium_confidence = summary.medium_confidence.saturating_add(1)
            }
            Confidence::Low => summary.low_confidence = summary.low_confidence.saturating_add(1),
            Confidence::Unknown => {}
        }
        let Some(resource_id) = observation.resource_id else {
            continue;
        };
        summary.attributed_observations = summary.attributed_observations.saturating_add(1);
        let usage = resources
            .entry(resource_id.clone())
            .or_insert_with(|| ResourceUsage {
                resource_id,
                ..Default::default()
            });
        usage.observations = usage.observations.saturating_add(1);
        match signal {
            ResourceSignal::Activation => usage.activations = usage.activations.saturating_add(1),
            ResourceSignal::Load => usage.loads = usage.loads.saturating_add(1),
            ResourceSignal::ToolExecution | ResourceSignal::Failure => {
                usage.tool_executions = usage.tool_executions.saturating_add(1)
            }
            ResourceSignal::Completion => usage.completions = usage.completions.saturating_add(1),
            ResourceSignal::Unknown => {}
        }
        if signal == ResourceSignal::Failure
            || observation.facts.get("outcome").and_then(Value::as_str) == Some("failure")
        {
            usage.failures = usage.failures.saturating_add(1);
        }
        match observation_confidence {
            Confidence::High => usage.high_confidence = usage.high_confidence.saturating_add(1),
            Confidence::Medium => {
                usage.medium_confidence = usage.medium_confidence.saturating_add(1)
            }
            Confidence::Low => usage.low_confidence = usage.low_confidence.saturating_add(1),
            Confidence::Unknown => {}
        }
        if usage
            .last_seen
            .as_deref()
            .is_none_or(|last| last < observation.timestamp.as_str())
        {
            usage.last_seen = Some(timestamp.to_rfc3339());
        }
    }
    summary.unattributed_observations = summary
        .observations
        .saturating_sub(summary.attributed_observations);
    summary.resources = resources.into_values().collect();
    summary.resources.sort_by(|left, right| {
        right
            .activations
            .cmp(&left.activations)
            .then_with(|| right.observations.cmp(&left.observations))
            .then_with(|| left.resource_id.cmp(&right.resource_id))
    });
    Ok(summary)
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
