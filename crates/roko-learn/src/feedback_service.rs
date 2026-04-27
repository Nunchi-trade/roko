//! FeedbackService — concrete implementation of `FeedbackSink`.
//!
//! Records model call feedback, gate results, and workflow outcomes into the
//! existing learning infrastructure as append-only efficiency JSONL events.

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
}

impl FeedbackService {
    /// Create a new service writing to the given data directory.
    #[must_use]
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            buffer: Mutex::new(Vec::with_capacity(64)),
            buffer_capacity: 64,
        }
    }

    /// Create a service from the standard `.roko` directory.
    #[must_use]
    pub fn from_roko_dir(roko_dir: &Path) -> Self {
        Self::new(roko_dir.join("learn"))
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
            self.flush()?;
        }

        Ok(())
    }
}

impl Drop for FeedbackService {
    fn drop(&mut self) {
        let _ = self.flush();
    }
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
}
