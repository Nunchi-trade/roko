//! Knowledge ingestion sink — turns successful task completions into
//! candidate observations for the durable knowledge store.
//!
//! ## What it owns
//!
//! When a task completes successfully, the sink emits a
//! [`KnowledgeCandidate`] describing what to ingest. By default the sink
//! writes those candidates to a JSONL file under `.roko/learn/` so they
//! can be picked up by a separate ingestion pass without coupling the
//! runner to neuro-store internals.
//!
//! When a [`KnowledgeIngestor`] is supplied, the sink calls into it
//! synchronously instead. This split keeps the sink fast (the runner
//! never blocks on durable knowledge writes during execution) while
//! allowing tests / smoke runs to verify the full path.
//!
//! ## Architectural note
//!
//! The knowledge subsystem is large and its full ingestion API is not
//! stabilized. This sink is the runtime seam — it lets the runner
//! produce candidates today without blocking on the store evolution.
//! The `.roko/learn/knowledge-candidates.jsonl` file is consumed by an
//! offline reinforcement pass; see `.roko/GAPS.md` for the remaining
//! wiring work.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use super::{FeedbackEvent, FeedbackSink};

/// Candidate observation written to disk for downstream ingestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeCandidate {
    pub plan_id: String,
    pub task_id: String,
    pub model: String,
    pub provider: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub duration_ms: u64,
    pub kind: KnowledgeCandidateKind,
}

/// Kind of observation. Currently only successful runs become candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeCandidateKind {
    /// Task ran cleanly to completion.
    Success,
    /// A specific gate produced a falsifier the store should remember.
    GateFalsifier,
}

/// Optional in-process ingestor — passes candidates into a live store.
#[async_trait]
pub trait KnowledgeIngestor: Send + Sync + std::fmt::Debug {
    async fn ingest(&self, candidate: &KnowledgeCandidate) -> Result<(), anyhow::Error>;
}

/// Sink that writes ingestion candidates to disk and / or hands them to
/// a live ingestor.
#[derive(Debug)]
pub struct KnowledgeIngestionSink {
    candidates_path: PathBuf,
    file: Mutex<Option<tokio::fs::File>>,
    ingestor: Option<Arc<dyn KnowledgeIngestor>>,
}

impl KnowledgeIngestionSink {
    /// Construct a sink writing candidates to `candidates_path`.
    #[must_use]
    pub fn at(candidates_path: impl Into<PathBuf>) -> Self {
        Self {
            candidates_path: candidates_path.into(),
            file: Mutex::new(None),
            ingestor: None,
        }
    }

    /// Attach a live ingestor (called in addition to the JSONL write).
    #[must_use]
    pub fn with_ingestor(mut self, ingestor: Arc<dyn KnowledgeIngestor>) -> Self {
        self.ingestor = Some(ingestor);
        self
    }

    async fn write(&self, candidate: &KnowledgeCandidate) -> Result<(), anyhow::Error> {
        if let Some(parent) = self.candidates_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        let mut guard = self.file.lock().await;
        if guard.is_none() {
            let file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.candidates_path)
                .await?;
            *guard = Some(file);
        }
        let line = serde_json::to_string(candidate)?;
        let bytes = format!("{line}\n");
        let file = guard.as_mut().unwrap();
        file.write_all(bytes.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }
}

#[async_trait]
impl FeedbackSink for KnowledgeIngestionSink {
    fn name(&self) -> &'static str {
        "knowledge"
    }

    fn interested(&self, event: &FeedbackEvent) -> bool {
        matches!(
            event,
            FeedbackEvent::TaskCompleted { .. } | FeedbackEvent::GateOutcome { .. }
        )
    }

    async fn on_event(&self, event: &FeedbackEvent) -> Result<(), anyhow::Error> {
        let candidate = match event {
            FeedbackEvent::TaskCompleted {
                plan_id,
                task_id,
                outcome,
                succeeded: true,
                ..
            } => KnowledgeCandidate {
                plan_id: plan_id.clone(),
                task_id: task_id.clone(),
                model: outcome.model.clone(),
                provider: outcome.provider.clone(),
                tokens_in: outcome.tokens_in,
                tokens_out: outcome.tokens_out,
                cost_usd: outcome.cost_usd,
                duration_ms: outcome.duration_ms,
                kind: KnowledgeCandidateKind::Success,
            },
            FeedbackEvent::GateOutcome {
                plan_id,
                task_id,
                rung: _,
                passed: false,
                duration_ms,
            } => KnowledgeCandidate {
                plan_id: plan_id.clone(),
                task_id: task_id.clone(),
                model: String::new(),
                provider: String::new(),
                tokens_in: 0,
                tokens_out: 0,
                cost_usd: 0.0,
                duration_ms: *duration_ms,
                kind: KnowledgeCandidateKind::GateFalsifier,
            },
            _ => return Ok(()),
        };

        self.write(&candidate).await?;
        if let Some(ingestor) = &self.ingestor {
            ingestor.ingest(&candidate).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::{AgentOutcome, ModelChoiceSource};
    use tempfile::tempdir;

    fn outcome() -> AgentOutcome {
        AgentOutcome {
            task_id: "t".into(),
            plan_id: "p".into(),
            model: "claude-sonnet-4-6".into(),
            provider: "claude_cli".into(),
            output: "".into(),
            tokens_in: 100,
            tokens_out: 50,
            cost_usd: 0.001,
            duration_ms: 42,
            exit_code: Some(0),
            is_error: false,
        }
    }

    #[tokio::test]
    async fn successful_task_writes_success_candidate() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("kc.jsonl");
        let sink = KnowledgeIngestionSink::at(&path);
        sink.on_event(&FeedbackEvent::TaskCompleted {
            plan_id: "p".into(),
            task_id: "t".into(),
            outcome: outcome(),
            model_source: ModelChoiceSource::Router,
            succeeded: true,
        })
        .await
        .unwrap();
        let txt = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(txt.contains("\"kind\":\"success\""));
        assert!(txt.contains("\"model\":\"claude-sonnet-4-6\""));
    }

    #[tokio::test]
    async fn failed_task_writes_no_candidate() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("kc.jsonl");
        let sink = KnowledgeIngestionSink::at(&path);
        sink.on_event(&FeedbackEvent::TaskCompleted {
            plan_id: "p".into(),
            task_id: "t".into(),
            outcome: outcome(),
            model_source: ModelChoiceSource::Router,
            succeeded: false,
        })
        .await
        .unwrap();
        assert!(!path.exists() || tokio::fs::read(&path).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn gate_failure_writes_falsifier_candidate() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("kc.jsonl");
        let sink = KnowledgeIngestionSink::at(&path);
        sink.on_event(&FeedbackEvent::GateOutcome {
            plan_id: "p".into(),
            task_id: "t".into(),
            rung: 2,
            passed: false,
            duration_ms: 1000,
        })
        .await
        .unwrap();
        let txt = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(txt.contains("\"kind\":\"gate_falsifier\""));
    }
}
