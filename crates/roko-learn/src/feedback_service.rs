//! FeedbackService — concrete implementation of `FeedbackSink`.
//!
//! Records model call feedback, gate results, and workflow outcomes into the
//! existing learning infrastructure as append-only efficiency JSONL events.

use crate::episode_logger::{Episode, EpisodeLogger, Usage};
use async_trait::async_trait;
use chrono::Utc;
use roko_core::foundation::{FeedbackEvent, FeedbackSink};
use roko_core::{Result, RokoError};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Service that records feedback events for the learning subsystem.
///
/// This is the canonical way to record workflow feedback. It logs model call
/// metrics, gate results, and workflow outcomes into `.roko/learn`-style data
/// files so downstream learning components can consume them.
pub struct FeedbackService {
    /// Directory for feedback data files.
    data_dir: PathBuf,
    /// In-memory buffer of recent events for batched writes.
    buffer: Mutex<Vec<FeedbackEvent>>,
    /// Maximum buffer size before flushing.
    buffer_capacity: usize,
    /// Optional episode logger for workflow outcome records.
    episode_logger: Option<EpisodeLogger>,
}

impl FeedbackService {
    /// Create a new service writing to the given data directory.
    #[must_use]
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            buffer: Mutex::new(Vec::with_capacity(64)),
            buffer_capacity: 64,
            episode_logger: None,
        }
    }

    /// Create a service from the standard `.roko` directory.
    #[must_use]
    pub fn from_roko_dir(roko_dir: &Path) -> Self {
        Self::new(roko_dir.join("learn"))
    }

    /// Attach an episode logger to record workflow outcomes as episodes.
    ///
    /// When a `WorkflowComplete` event is flushed, the service will also
    /// append an `Episode` record to the logger's JSONL file.
    #[must_use]
    pub fn with_episode_logger(mut self, logger: EpisodeLogger) -> Self {
        self.episode_logger = Some(logger);
        self
    }

    /// Create a service from the `.roko` directory with episode recording enabled.
    #[must_use]
    pub fn from_roko_dir_with_episodes(roko_dir: &Path) -> Self {
        let episodes_path = roko_dir.join("episodes.jsonl");
        let logger = EpisodeLogger::new(episodes_path);
        Self::from_roko_dir(roko_dir).with_episode_logger(logger)
    }

    /// Flush buffered events to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the buffer lock is poisoned, the data directory
    /// cannot be created, or the JSONL file cannot be written.
    pub fn flush(&self) -> Result<()> {
        let events = {
            let mut buf = self
                .buffer
                .lock()
                .map_err(|error| RokoError::Invalid(format!("lock poisoned: {error}")))?;
            std::mem::take(&mut *buf)
        };

        if events.is_empty() {
            return Ok(());
        }

        std::fs::create_dir_all(&self.data_dir)?;
        let efficiency_path = self.data_dir.join("efficiency.jsonl");

        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&efficiency_path)?;

        for event in &events {
            let ts = Utc::now().to_rfc3339();
            let json = match event {
                FeedbackEvent::ModelCall {
                    run_id,
                    model,
                    role,
                    input_tokens,
                    output_tokens,
                    cost_usd,
                    latency_ms,
                    success,
                } => serde_json::json!({
                    "kind": "model_call",
                    "run_id": run_id,
                    "model": model,
                    "role": role,
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens,
                    "cost_usd": cost_usd,
                    "latency_ms": latency_ms,
                    "success": success,
                    "ts": ts,
                }),
                FeedbackEvent::GateResult {
                    run_id,
                    gate_name,
                    passed,
                    duration_ms,
                } => serde_json::json!({
                    "kind": "gate_result",
                    "run_id": run_id,
                    "gate_name": gate_name,
                    "passed": passed,
                    "duration_ms": duration_ms,
                    "ts": ts,
                }),
                FeedbackEvent::WorkflowComplete {
                    run_id,
                    outcome,
                    total_cost_usd,
                    total_tokens,
                    duration_ms,
                } => serde_json::json!({
                    "kind": "workflow_complete",
                    "run_id": run_id,
                    "outcome": outcome,
                    "total_cost_usd": total_cost_usd,
                    "total_tokens": total_tokens,
                    "duration_ms": duration_ms,
                    "ts": ts,
                }),
            };
            writeln!(file, "{json}")?;
        }

        Ok(())
    }

    /// Flush buffered events to disk and append workflow-complete episodes.
    ///
    /// The synchronous [`Self::flush`] path remains suitable for `Drop` and
    /// callers without an async runtime; this async path adds best-effort
    /// episode recording after the efficiency JSONL write succeeds.
    ///
    /// # Errors
    ///
    /// Returns any error from the synchronous efficiency JSONL flush. Episode
    /// append errors are logged and do not fail the flush.
    pub async fn flush_async(&self) -> Result<()> {
        let episodes = if self.episode_logger.is_some() {
            self.pending_workflow_episodes()?
        } else {
            Vec::new()
        };

        self.flush()?;

        if let Some(ref logger) = self.episode_logger {
            for episode in episodes {
                if let Err(err) = logger.append(&episode).await {
                    tracing::warn!("failed to append episode: {err}");
                }
            }
        }

        Ok(())
    }

    fn pending_workflow_episodes(&self) -> Result<Vec<Episode>> {
        let buf = self
            .buffer
            .lock()
            .map_err(|error| RokoError::Invalid(format!("lock poisoned: {error}")))?;

        Ok(buf
            .iter()
            .filter_map(|event| {
                if let FeedbackEvent::WorkflowComplete {
                    run_id,
                    outcome,
                    total_cost_usd,
                    total_tokens,
                    duration_ms,
                } = event
                {
                    Some(build_episode_from_workflow(
                        run_id,
                        outcome,
                        *total_cost_usd,
                        *total_tokens,
                        *duration_ms,
                    ))
                } else {
                    None
                }
            })
            .collect())
    }
}

#[async_trait]
impl FeedbackSink for FeedbackService {
    async fn record(&self, event: FeedbackEvent) -> Result<()> {
        let should_flush = {
            let mut buf = self
                .buffer
                .lock()
                .map_err(|error| RokoError::Invalid(format!("lock poisoned: {error}")))?;
            buf.push(event);
            buf.len() >= self.buffer_capacity
        };

        if should_flush {
            self.flush_async().await?;
        }

        Ok(())
    }
}

impl Drop for FeedbackService {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

fn build_episode_from_workflow(
    run_id: &str,
    outcome: &str,
    total_cost_usd: f64,
    total_tokens: u64,
    duration_ms: u64,
) -> Episode {
    let mut episode = Episode::new("workflow", run_id);
    episode.kind = "workflow_complete".to_string();
    episode.id = format!("ep-{run_id}");
    episode.episode_id = episode.id.clone();
    episode.trigger_kind = "workflow_complete".to_string();
    episode.success = outcome == "success";
    episode.duration_secs = duration_ms as f64 / 1000.0;
    episode.tokens_used = total_tokens;
    episode.usage = Usage {
        cost_usd: total_cost_usd,
        cost_usd_without_cache: total_cost_usd,
        wall_ms: duration_ms,
        ..Usage::default()
    };
    if !episode.success {
        episode.failure_reason = Some(outcome.to_string());
    }
    episode
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_model_call() {
        let dir = tempfile::tempdir().unwrap();
        let svc = FeedbackService::new(dir.path().to_path_buf());

        svc.record(FeedbackEvent::ModelCall {
            run_id: "r1".into(),
            model: "sonnet".into(),
            role: "implementer".into(),
            input_tokens: 1000,
            output_tokens: 500,
            cost_usd: 0.01,
            latency_ms: 2000,
            success: true,
        })
        .await
        .unwrap();

        svc.flush().unwrap();

        let content = std::fs::read_to_string(dir.path().join("efficiency.jsonl")).unwrap();
        assert!(content.contains("model_call"));
        assert!(content.contains("sonnet"));
    }

    #[tokio::test]
    async fn records_gate_result() {
        let dir = tempfile::tempdir().unwrap();
        let svc = FeedbackService::new(dir.path().to_path_buf());

        svc.record(FeedbackEvent::GateResult {
            run_id: "r1".into(),
            gate_name: "compile".into(),
            passed: true,
            duration_ms: 3000,
        })
        .await
        .unwrap();

        svc.flush().unwrap();

        let content = std::fs::read_to_string(dir.path().join("efficiency.jsonl")).unwrap();
        assert!(content.contains("gate_result"));
    }

    #[tokio::test]
    async fn records_episode_on_workflow_complete() {
        let dir = tempfile::tempdir().unwrap();
        let episodes_path = dir.path().join("episodes.jsonl");
        let logger = EpisodeLogger::new(&episodes_path);
        let svc = FeedbackService::new(dir.path().join("learn")).with_episode_logger(logger);

        svc.record(FeedbackEvent::WorkflowComplete {
            run_id: "r1".into(),
            outcome: "success".into(),
            total_cost_usd: 0.02,
            total_tokens: 1200,
            duration_ms: 2500,
        })
        .await
        .unwrap();

        svc.flush_async().await.unwrap();

        let episodes = EpisodeLogger::read_all(&episodes_path).await.unwrap();
        assert!(
            episodes
                .iter()
                .any(|episode| episode.kind == "workflow_complete")
        );
    }

    #[tokio::test]
    async fn sync_flush_still_works_without_episodes() {
        let dir = tempfile::tempdir().unwrap();
        let svc = FeedbackService::new(dir.path().to_path_buf());

        svc.record(FeedbackEvent::ModelCall {
            run_id: "r1".into(),
            model: "sonnet".into(),
            role: "implementer".into(),
            input_tokens: 1000,
            output_tokens: 500,
            cost_usd: 0.01,
            latency_ms: 2000,
            success: true,
        })
        .await
        .unwrap();

        svc.flush().unwrap();

        let efficiency_path = dir.path().join("efficiency.jsonl");
        assert!(efficiency_path.exists());
        let content = std::fs::read_to_string(efficiency_path).unwrap();
        assert!(content.contains("model_call"));
    }
}
