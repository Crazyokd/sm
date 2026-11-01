use crate::{
    files::reject_symlink_path,
    model::ResourceResult,
    observe::{
        CaptureHealth, RecordingError, ResourceUsageSummary, capture_health, resource_usage_summary,
    },
    privacy::{redact, redact_text},
    process::run_command,
    retention::{RetentionHealth, summary},
    service::Runtime,
    state::{RunRecord, State},
    telemetry::{RunSnapshot, snapshot},
};
use anyhow::{Result, ensure};
use chrono::Utc;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    time::Duration,
};

#[derive(Debug, Serialize)]
pub struct Storage {
    pub database_bytes: u64,
    pub events_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct Health {
    pub runs: RunSnapshot,
    pub capture: CaptureHealth,
    pub retention: RetentionHealth,
    pub pending_raw_bytes: u64,
    pub pending_observations: u64,
    pub processed_observations: u64,
    pub parsing_errors: Vec<RecordingError>,
    pub storage: Storage,
    pub resource_usage: ResourceUsageSummary,
}

pub fn health(state: &State) -> Result<Health> {
    let directory = state.directory()?;
    let retention = summary(state)?;
    let mut total_bytes = 0;
    let mut database_bytes = 0;
    for entry in walkdir::WalkDir::new(directory).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let bytes = entry.metadata()?.len();
        total_bytes += bytes;
        if entry.file_name() == "state.sqlite3" {
            database_bytes = bytes;
        }
    }
    Ok(Health {
        runs: snapshot(state)?,
        capture: capture_health(directory)?,
        pending_raw_bytes: retention.pending_raw_bytes,
        pending_observations: retention.pending_observations,
        processed_observations: retention.observations - retention.pending_observations,
        parsing_errors: state.get("recording_errors")?,
        storage: Storage {
            database_bytes,
            events_bytes: retention.raw_bytes,
            total_bytes,
        },
        resource_usage: resource_usage_summary(
            state,
            chrono::Duration::days(retention.retention_days as i64),
        )?,
        retention,
    })
}

#[derive(Debug, Serialize)]
pub struct Probe {
    pub available: bool,
    pub detail: String,
}

fn probe(program: &str, args: &[&str]) -> Probe {
    let mut command = Command::new(program);
    command.args(args);
    match run_command(command, None, Duration::from_secs(5)) {
        Ok(output) => Probe {
            available: true,
            detail: redact_text(String::from_utf8_lossy(&output.stdout).trim(), Some(4000)),
        },
        Err(error) => Probe {
            available: false,
            detail: redact_text(&format!("{error:#}"), Some(600)),
        },
    }
}

#[derive(Debug, Serialize)]
pub struct RunError {
    pub operation: String,
    pub time: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub generated_at: String,
    pub manager_version: &'static str,
    pub approved_revision: Probe,
    pub client_versions: BTreeMap<&'static str, Probe>,
    pub services: Probe,
    pub health: Health,
    pub resource_results: BTreeMap<String, ResourceResult>,
    pub recent_errors: Vec<RunError>,
    pub journal: Probe,
}

pub fn collect(runtime: &Runtime, state: &State) -> Result<Report> {
    let mut results: BTreeMap<String, ResourceResult> = state.get("resource_results")?;
    for result in results.values_mut() {
        result.message = redact_text(&result.message, Some(1200));
    }
    let runs: Vec<RunRecord> = state.get("runs")?;
    let recent_errors = runs
        .into_iter()
        .rev()
        .filter_map(|run| {
            run.result
                .get("error")
                .or_else(|| run.result.get("details").and_then(|d| d.get("error")))
                .and_then(serde_json::Value::as_str)
                .map(|error| RunError {
                    operation: redact_text(&run.operation, Some(80)),
                    time: run.time,
                    error: redact_text(error, Some(1200)),
                })
        })
        .take(20)
        .collect();
    let repo = runtime.repo.to_string_lossy();
    Ok(Report {
        generated_at: Utc::now().to_rfc3339(),
        manager_version: env!("CARGO_PKG_VERSION"),
        approved_revision: probe("git", &["-C", &repo, "rev-parse", "origin/main"]),
        client_versions: BTreeMap::from([
            ("codex", probe("codex", &["--version"])),
            ("kimi", probe("kimi", &["--version"])),
        ]),
        services: probe(
            "systemctl",
            &[
                "--user",
                "show",
                "sm-web.service",
                "sm-cycle.service",
                "sm-cycle.timer",
                "--property=Id,ActiveState,SubState,Result,MainPID,ExecMainStatus,NextElapseUSecRealtime",
            ],
        ),
        health: health(state)?,
        resource_results: results,
        recent_errors,
        journal: probe(
            "journalctl",
            &[
                "--user",
                "-u",
                "sm-web.service",
                "-u",
                "sm-cycle.service",
                "-p",
                "warning",
                "-n",
                "20",
                "--no-pager",
                "--output=short-iso",
            ],
        ),
    })
}

pub fn bytes(report: &Report) -> Result<Vec<u8>> {
    let mut value = redact(&serde_json::to_value(report)?, Some(4000));
    let mut counts = BTreeMap::<String, u64>::new();
    for result in report.resource_results.values() {
        let status = serde_json::to_value(result.status)?;
        if let Some(status) = status.as_str() {
            *counts.entry(status.to_owned()).or_default() += 1;
        }
    }
    value["resource_status_counts"] = serde_json::to_value(counts)?;
    value["omitted"] =
        serde_json::json!({"runs":0,"resource_results":0,"recent_errors":0,"parsing_errors":0});
    loop {
        let mut bytes = serde_json::to_vec_pretty(&value)?;
        if bytes.len() < 256 * 1024 {
            bytes.push(b'\n');
            return Ok(bytes);
        }
        ensure!(
            omit_detail(&mut value),
            "diagnostic metadata exceeds 256 KiB"
        );
    }
}

fn omit_detail(value: &mut serde_json::Value) -> bool {
    for minimum in [10, 0] {
        for (key, path) in [
            ("runs", "/health/runs/recent"),
            ("resource_results", "/resource_results"),
            ("recent_errors", "/recent_errors"),
            ("parsing_errors", "/health/parsing_errors"),
        ] {
            let removed = match value.pointer_mut(path) {
                Some(serde_json::Value::Array(items)) if items.len() > minimum => {
                    items.pop();
                    true
                }
                Some(serde_json::Value::Object(items)) if items.len() > minimum => {
                    let key = items.keys().next_back().cloned();
                    if let Some(key) = key {
                        items.remove(&key);
                    }
                    true
                }
                _ => false,
            };
            if removed {
                value["omitted"][key] =
                    serde_json::json!(value["omitted"][key].as_u64().unwrap_or_default() + 1);
                return true;
            }
        }
    }
    false
}

pub fn write(path: &Path, report: &Report) -> Result<()> {
    reject_symlink_path(path)?;
    let bytes = bytes(report)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    reject_symlink_path(path)?;
    temporary.persist(path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub fn latest_success(health: &Health) -> Option<&str> {
    health
        .runs
        .recent
        .iter()
        .find(|run| run.status == crate::telemetry::RunStatus::Succeeded)
        .and_then(|run| run.finished_at.as_deref())
}
