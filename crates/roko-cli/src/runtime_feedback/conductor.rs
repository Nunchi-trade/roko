//! Conductor sink — surfaces gate failures and retries to the conductor /
//! affect engine.
//!
//! ## What the conductor expects
//!
//! `roko-conductor` and `roko-daimon` model "how the run is feeling" via
//! affect signals. The two events that drive their state most directly
//! are gate outcomes (especially failures) and retry decisions
//! (frequency / backoff growth signal pressure).
//!
//! Today the conductor reads JSONL files written by `runner/persist.rs`.
//! This sink **mirrors** the events into a dedicated
//! `.roko/conductor/observations.jsonl` so the conductor doesn't have to
//! parse the much larger generic events file. Once the conductor adopts
//! a streaming subscriber, this sink will switch to in-process delivery.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use super::{FeedbackEvent, FeedbackSink};

/// One conductor-observable record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConductorObservation {
    /// Plan id this observation belongs to.
    pub plan_id: String,
    /// Task id the observation belongs to.
    pub task_id: String,
    /// Stable category for downstream filtering.
    pub kind: ConductorObservationKind,
    /// Wall-clock timestamp in milliseconds since the Unix epoch.
    pub timestamp_ms: u64,
    /// Optional structured detail (gate name, attempt count, ...).
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConductorObservationKind {
    GatePassed,
    GateFailed,
    RetryStarted,
}

#[derive(Debug)]
pub struct ConductorObservationSink {
    path: PathBuf,
    file: Mutex<Option<tokio::fs::File>>,
}

impl ConductorObservationSink {
    /// Construct a sink that writes observations to `path`.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            file: Mutex::new(None),
        }
    }

    async fn append(&self, observation: &ConductorObservation) -> Result<(), anyhow::Error> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        let mut guard = self.file.lock().await;
        if guard.is_none() {
            let file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .await?;
            *guard = Some(file);
        }
        let line = serde_json::to_string(observation)?;
        let bytes = format!("{line}\n");
        let file = guard.as_mut().unwrap();
        file.write_all(bytes.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }
}

#[async_trait]
impl FeedbackSink for ConductorObservationSink {
    fn name(&self) -> &'static str {
        "conductor"
    }

    fn interested(&self, event: &FeedbackEvent) -> bool {
        matches!(
            event,
            FeedbackEvent::GateOutcome { .. } | FeedbackEvent::RetryDecision { .. }
        )
    }

    async fn on_event(&self, event: &FeedbackEvent) -> Result<(), anyhow::Error> {
        let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let observation = match event {
            FeedbackEvent::GateOutcome {
                plan_id,
                task_id,
                rung,
                passed,
                duration_ms,
            } => ConductorObservation {
                plan_id: plan_id.clone(),
                task_id: task_id.clone(),
                kind: if *passed {
                    ConductorObservationKind::GatePassed
                } else {
                    ConductorObservationKind::GateFailed
                },
                timestamp_ms: now,
                detail: serde_json::json!({
                    "rung": rung,
                    "duration_ms": duration_ms,
                }),
            },
            FeedbackEvent::RetryDecision {
                plan_id,
                task_id,
                attempt,
                backoff_secs,
            } => ConductorObservation {
                plan_id: plan_id.clone(),
                task_id: task_id.clone(),
                kind: ConductorObservationKind::RetryStarted,
                timestamp_ms: now,
                detail: serde_json::json!({
                    "attempt": attempt,
                    "backoff_secs": backoff_secs,
                }),
            },
            _ => return Ok(()),
        };
        self.append(&observation).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn gate_failure_recorded_with_rung_detail() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("conductor.jsonl");
        let sink = ConductorObservationSink::at(&path);
        sink.on_event(&FeedbackEvent::GateOutcome {
            plan_id: "p".into(),
            task_id: "t".into(),
            rung: 3,
            passed: false,
            duration_ms: 5_000,
        })
        .await
        .unwrap();
        let txt = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(txt.contains("\"kind\":\"gate_failed\""));
        assert!(txt.contains("\"rung\":3"));
    }

    #[tokio::test]
    async fn retry_decision_recorded_with_backoff_detail() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("conductor.jsonl");
        let sink = ConductorObservationSink::at(&path);
        sink.on_event(&FeedbackEvent::RetryDecision {
            plan_id: "p".into(),
            task_id: "t".into(),
            attempt: 2,
            backoff_secs: 4,
        })
        .await
        .unwrap();
        let txt = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(txt.contains("\"kind\":\"retry_started\""));
        assert!(txt.contains("\"backoff_secs\":4"));
    }

    #[tokio::test]
    async fn turn_completed_is_ignored() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("conductor.jsonl");
        let sink = ConductorObservationSink::at(&path);
        let event = FeedbackEvent::TurnCompleted {
            plan_id: "p".into(),
            task_id: "t".into(),
            attempt: 0,
            tokens_in: 1,
            tokens_out: 1,
            cost_usd: 0.0,
        };
        assert!(!sink.interested(&event));
        sink.on_event(&event).await.unwrap();
        assert!(!path.exists());
    }
}
