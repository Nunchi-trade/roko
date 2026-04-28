//! FeedbackService — concrete implementation of `FeedbackSink`.
//!
//! Records model call feedback, gate results, and workflow outcomes into the
//! existing learning infrastructure as append-only efficiency JSONL events.

use crate::cascade_router::CascadeRouter;
use crate::episode_logger::{Episode, EpisodeLogger, Usage};
use crate::model_router::CONTEXT_DIM;
use async_trait::async_trait;
use chrono::Utc;
use roko_core::foundation::{FeedbackEvent, FeedbackSink};
use roko_core::{Result, RokoError};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const KNOWLEDGE_FEEDBACK_FILE: &str = "knowledge-feedback.jsonl";

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
    /// Optional cascade router for eager model-call reward observations.
    cascade_router: Option<Arc<CascadeRouter>>,
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
            cascade_router: None,
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

    /// Attach a cascade router for bandit reward observations.
    ///
    /// On each `ModelCall` event, the service will call `router.observe()`
    /// with a success/failure reward signal so the bandit can update its
    /// model selection policy.
    #[must_use]
    pub fn with_cascade_router(mut self, router: Arc<CascadeRouter>) -> Self {
        self.cascade_router = Some(router);
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

    /// Record which knowledge entries influenced a routing/prompt decision
    /// and whether the subsequent gate check passed.
    ///
    /// # Errors
    ///
    /// Returns an error if the data directory cannot be created, the feedback
    /// file cannot be opened, or the JSONL record cannot be written.
    pub fn record_knowledge_usage(
        &self,
        run_id: &str,
        knowledge_ids: Vec<String>,
        gate_passed: bool,
        model_slug: &str,
    ) -> Result<()> {
        if knowledge_ids.is_empty() {
            return Ok(());
        }

        std::fs::create_dir_all(&self.data_dir)?;
        let path = self.data_dir.join(KNOWLEDGE_FEEDBACK_FILE);
        let timestamp = Utc::now().to_rfc3339();
        let json = serde_json::json!({
            "type": "knowledge_usage",
            "run_id": run_id,
            "knowledge_ids": knowledge_ids,
            "gate_passed": gate_passed,
            "model_slug": model_slug,
            "timestamp": timestamp,
        });

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(file, "{json}")?;

        tracing::debug!(
            run_id,
            model_slug,
            gate_passed,
            knowledge_count = json
                .get("knowledge_ids")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len),
            "recorded knowledge usage feedback"
        );

        Ok(())
    }

    /// Read the knowledge feedback log and compute per-entry success rates.
    ///
    /// Returns a map of knowledge_id -> (successes, total_uses).
    ///
    /// # Errors
    ///
    /// Returns an error if the feedback file cannot be read or a non-empty
    /// JSONL line cannot be parsed as JSON.
    pub fn compute_knowledge_scores(&self) -> Result<HashMap<String, (u32, u32)>> {
        let path = self.data_dir.join(KNOWLEDGE_FEEDBACK_FILE);
        if !path.exists() {
            return Ok(HashMap::new());
        }

        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let mut scores: HashMap<String, (u32, u32)> = HashMap::new();

        for line in reader.lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let value: serde_json::Value = serde_json::from_str(trimmed)?;
            let gate_passed = value
                .get("gate_passed")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let Some(knowledge_ids) = value
                .get("knowledge_ids")
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };

            for knowledge_id in knowledge_ids
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter(|id| !id.is_empty())
            {
                let (successes, total_uses) =
                    scores.entry(knowledge_id.to_string()).or_insert((0, 0));
                *total_uses += 1;
                if gate_passed {
                    *successes += 1;
                }
            }
        }

        Ok(scores)
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

    fn observe_model_call(&self, model: &str, success: bool, role: &str, latency_ms: u64) {
        let Some(ref router) = self.cascade_router else {
            return;
        };

        let context_vec = model_call_context_vec(role, latency_ms);
        let Some(model_idx) = router.model_index_for_slug(model) else {
            tracing::debug!("model {model} not in cascade router slug list, skipping observe");
            return;
        };

        let reward = if success { 1.0 } else { 0.0 };
        router.observe(context_vec, model_idx, reward);
    }
}

#[async_trait]
impl FeedbackSink for FeedbackService {
    async fn record(&self, event: FeedbackEvent) -> Result<()> {
        if let FeedbackEvent::ModelCall {
            ref model,
            ref role,
            latency_ms,
            success,
            ..
        } = event
        {
            self.observe_model_call(model, success, role, latency_ms);
        }

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

fn model_call_context_vec(role: &str, latency_ms: u64) -> Vec<f64> {
    let role_feature = simple_role_hash(role);
    let latency_feature = (latency_ms as f64 / 60_000.0).min(1.0);
    let mut context_vec = vec![0.0; CONTEXT_DIM];

    // Preserve the requested minimal model-call signals while using the
    // router's fixed raw context width so LinUCB accepts the observation.
    context_vec[0] = role_feature;
    context_vec[1] = latency_feature;
    context_vec[16] = 1.0;

    context_vec
}

fn simple_role_hash(role: &str) -> f64 {
    let hash: u32 = role.bytes().fold(0u32, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(u32::from(b))
    });
    f64::from(hash % 1000) / 1000.0
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

    #[test]
    fn records_knowledge_usage() {
        let dir = tempfile::tempdir().unwrap();
        let svc = FeedbackService::new(dir.path().to_path_buf());

        svc.record_knowledge_usage(
            "r1",
            vec!["knowledge-a".into(), "knowledge-b".into()],
            true,
            "sonnet",
        )
        .unwrap();

        let content = std::fs::read_to_string(dir.path().join(KNOWLEDGE_FEEDBACK_FILE)).unwrap();
        let value: serde_json::Value = serde_json::from_str(content.trim()).unwrap();

        assert_eq!(value["type"], "knowledge_usage");
        assert_eq!(value["run_id"], "r1");
        assert_eq!(value["gate_passed"], true);
        assert_eq!(value["model_slug"], "sonnet");
        assert_eq!(
            value["knowledge_ids"],
            serde_json::json!(["knowledge-a", "knowledge-b"])
        );
        assert!(value["timestamp"].as_str().is_some());
    }

    #[test]
    fn skips_empty_knowledge_usage() {
        let dir = tempfile::tempdir().unwrap();
        let svc = FeedbackService::new(dir.path().to_path_buf());

        svc.record_knowledge_usage("r1", Vec::new(), true, "sonnet")
            .unwrap();

        assert!(!dir.path().join(KNOWLEDGE_FEEDBACK_FILE).exists());
    }

    #[test]
    fn computes_knowledge_scores() {
        let dir = tempfile::tempdir().unwrap();
        let svc = FeedbackService::new(dir.path().to_path_buf());

        svc.record_knowledge_usage(
            "r1",
            vec!["knowledge-a".into(), "knowledge-b".into()],
            true,
            "sonnet",
        )
        .unwrap();
        svc.record_knowledge_usage("r2", vec!["knowledge-a".into()], false, "haiku")
            .unwrap();

        let scores = svc.compute_knowledge_scores().unwrap();

        assert_eq!(scores.get("knowledge-a"), Some(&(1, 2)));
        assert_eq!(scores.get("knowledge-b"), Some(&(1, 1)));
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

    #[tokio::test]
    async fn observes_model_call_to_cascade_router() {
        let dir = tempfile::tempdir().unwrap();
        let router = Arc::new(CascadeRouter::new(vec!["sonnet".into(), "opus".into()]));
        let svc =
            FeedbackService::new(dir.path().to_path_buf()).with_cascade_router(Arc::clone(&router));

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

        assert_eq!(router.total_observations(), 1);
    }

    #[tokio::test]
    async fn skips_observation_for_unknown_model() {
        let dir = tempfile::tempdir().unwrap();
        let router = Arc::new(CascadeRouter::new(vec!["sonnet".into()]));
        let svc =
            FeedbackService::new(dir.path().to_path_buf()).with_cascade_router(Arc::clone(&router));

        svc.record(FeedbackEvent::ModelCall {
            run_id: "r1".into(),
            model: "unknown-model".into(),
            role: "implementer".into(),
            input_tokens: 1000,
            output_tokens: 500,
            cost_usd: 0.01,
            latency_ms: 2000,
            success: true,
        })
        .await
        .unwrap();

        assert_eq!(router.total_observations(), 0);
    }
}
