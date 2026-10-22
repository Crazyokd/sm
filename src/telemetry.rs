use crate::{
    files::reject_symlink_path,
    privacy::redact_text,
    state::{RunRecord, State},
};
use anyhow::{Context, Result};
use chrono::Utc;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs::File, io::Write, time::Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunTrigger {
    Startup,
    Nightly,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunDetails {
    pub id: String,
    pub operation: String,
    pub trigger: RunTrigger,
    pub stage: String,
    pub resource: Option<String>,
    pub completed: usize,
    pub total: usize,
    pub started_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
    pub duration_ms: u64,
    pub status: RunStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSnapshot {
    pub current: Option<RunDetails>,
    pub recent: Vec<RunDetails>,
}

fn elapsed_ms(start: &str, end: &str) -> u64 {
    chrono::DateTime::parse_from_rfc3339(start)
        .and_then(|start| chrono::DateTime::parse_from_rfc3339(end).map(|end| end - start))
        .map_or(0, |duration| duration.num_milliseconds().max(0) as u64)
}

fn emit(event: &str, details: &RunDetails, extra: Value) {
    let mut value = json!({"event": event, "run": details});
    if let (Some(object), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
        object.extend(extra.clone());
    }
    let _ = writeln!(std::io::stderr().lock(), "{value}");
}

fn job_locked(state: &State) -> Result<bool> {
    let path = state.directory()?.join("job.lock");
    reject_symlink_path(&path)?;
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    match file.try_lock_exclusive() {
        Ok(()) => {
            FileExt::unlock(&file)?;
            Ok(false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(true),
        Err(error) => Err(error.into()),
    }
}

pub fn snapshot(state: &State) -> Result<RunSnapshot> {
    let mut current: Option<RunDetails> = state.get("active_run")?;
    if let Some(run) = &mut current {
        if run.status == RunStatus::Running {
            if job_locked(state)? {
                run.duration_ms = elapsed_ms(&run.started_at, &Utc::now().to_rfc3339());
            } else {
                run.status = RunStatus::Interrupted;
                run.error = Some("Job lock was released before this run finished".into());
            }
        }
    }
    let records: Vec<RunRecord> = state.get("runs")?;
    let recent: Vec<RunDetails> = records
        .into_iter()
        .rev()
        .filter_map(|record| record.result.get("run").cloned())
        .map(|value| serde_json::from_value(value).context("invalid saved run details"))
        .collect::<Result<_>>()?;
    if current
        .as_ref()
        .is_some_and(|current| recent.iter().any(|run| run.id == current.id))
    {
        current = None;
    }
    Ok(RunSnapshot { current, recent })
}

pub struct RunTracker {
    state: State,
    details: RunDetails,
    started: Instant,
    stage_started: Instant,
    finished: bool,
    active: bool,
}

impl RunTracker {
    /// Sync/cycle callers hold the job lock; init has a separate lifecycle while restarting services.
    pub fn start(state: &State, operation: &str) -> Result<Self> {
        let active = operation != "init";
        if let Some(mut old) = if active {
            state.get::<Option<RunDetails>>("active_run")?
        } else {
            None
        } {
            let records: Vec<RunRecord> = state.get("runs")?;
            if !records
                .iter()
                .any(|record| record.result["run"]["id"].as_str() == Some(&old.id))
            {
                if old.status == RunStatus::Running {
                    old.status = RunStatus::Interrupted;
                    old.updated_at = Utc::now().to_rfc3339();
                    old.finished_at = Some(old.updated_at.clone());
                    old.duration_ms = elapsed_ms(&old.started_at, &old.updated_at);
                    old.error = Some(
                        "Previous run was interrupted; detected after acquiring the job lock"
                            .into(),
                    );
                }
                state.add_run(&old.operation, &json!({"error":old.error,"run":old}))?;
                emit("run_recovered", &old, json!({}));
            }
        }
        let started = Instant::now();
        let now = Utc::now().to_rfc3339();
        let details = RunDetails {
            id: uuid::Uuid::new_v4().to_string(),
            operation: redact_text(operation, Some(80)),
            trigger: match std::env::var("SM_TRIGGER").as_deref() {
                Ok("startup") => RunTrigger::Startup,
                Ok("nightly") => RunTrigger::Nightly,
                _ => RunTrigger::Manual,
            },
            stage: "starting".into(),
            resource: None,
            completed: 0,
            total: 0,
            started_at: now.clone(),
            updated_at: now,
            finished_at: None,
            duration_ms: 0,
            status: RunStatus::Running,
            error: None,
        };
        if active {
            state.set("active_run", &details)?;
        }
        emit("run_started", &details, json!({}));
        emit("stage_started", &details, json!({}));
        Ok(Self {
            state: state.clone(),
            details,
            started,
            stage_started: started,
            finished: false,
            active,
        })
    }

    pub fn details(&self) -> &RunDetails {
        &self.details
    }

    pub fn stage(
        &mut self,
        stage: &str,
        resource: Option<&str>,
        completed: usize,
        total: usize,
    ) -> Result<()> {
        let stage = redact_text(stage, Some(80));
        let resource = resource.map(|resource| redact_text(resource, Some(160)));
        let changed = self.details.stage != stage || self.details.resource != resource;
        if changed {
            emit(
                "stage_finished",
                &self.details,
                json!({"stage_duration_ms": self.stage_started.elapsed().as_millis()}),
            );
        }
        self.details.stage = stage;
        self.details.resource = resource;
        self.details.completed = completed;
        self.details.total = total;
        self.details.updated_at = Utc::now().to_rfc3339();
        self.details.duration_ms = self.started.elapsed().as_millis() as u64;
        if self.active {
            self.state.set("active_run", &self.details)?;
        }
        if changed {
            self.stage_started = Instant::now();
        }
        emit(
            if changed {
                "stage_started"
            } else {
                "run_progress"
            },
            &self.details,
            json!({}),
        );
        Ok(())
    }

    pub fn resource_result(&self, resource: &str, status: &str, message: &str) {
        emit(
            "resource_result",
            &self.details,
            json!({
                "resource": redact_text(resource, Some(160)),
                "status": redact_text(status, Some(40)),
                "message": redact_text(message, Some(1200)),
            }),
        );
    }

    pub fn finish<T: Serialize>(
        &mut self,
        business: &T,
        success: bool,
        error: Option<&str>,
    ) -> Result<()> {
        self.finish_as(
            business,
            if success {
                RunStatus::Succeeded
            } else {
                RunStatus::Failed
            },
            error,
        )
    }

    fn finish_as<T: Serialize>(
        &mut self,
        business: &T,
        status: RunStatus,
        error: Option<&str>,
    ) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        let mut result = serde_json::to_value(business)?;
        let object = result
            .as_object_mut()
            .context("run business result must be an object")?;
        self.finished = true;
        self.details.status = status;
        self.details.updated_at = Utc::now().to_rfc3339();
        self.details.finished_at = Some(self.details.updated_at.clone());
        self.details.duration_ms = self.started.elapsed().as_millis() as u64;
        self.details.error = error.map(|error| redact_text(error, Some(1600)));
        object.insert("run".into(), serde_json::to_value(&self.details)?);
        emit(
            "stage_finished",
            &self.details,
            json!({"stage_duration_ms": self.stage_started.elapsed().as_millis()}),
        );
        emit("run_finished", &self.details, json!({}));
        if self.active {
            self.state.set("active_run", &self.details)?;
        }
        self.state.add_run(&self.details.operation, &result)?;
        if self.active {
            self.state.set("active_run", &Option::<RunDetails>::None)?;
        }
        Ok(())
    }
}

impl Drop for RunTracker {
    fn drop(&mut self) {
        if !self.finished {
            if let Err(error) = self.finish_as(
                &json!({}),
                RunStatus::Interrupted,
                Some("Run ended before completion"),
            ) {
                let _ = writeln!(
                    std::io::stderr().lock(),
                    "{}",
                    json!({"event":"run_persistence_failed","id":self.details.id,"error":redact_text(&format!("{error:#}"),Some(1600))})
                );
            }
        }
    }
}
