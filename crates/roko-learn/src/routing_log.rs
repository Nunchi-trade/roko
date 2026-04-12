//! Append-only JSONL persistence for routing decisions.
//!
//! Each routing decision is written once when the decision is made and may be
//! written again with outcome fields populated after the task completes. The
//! latest record for a given `trace_id` is therefore the canonical view.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Persisted routing-decision record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingDecisionLog {
    /// RFC 3339 timestamp for when the record was written.
    pub timestamp: String,
    /// Deterministic trace identifier for the task dispatch.
    pub trace_id: String,
    /// Stable task identifier within the plan.
    pub task_id: String,
    /// Originally requested model before routing/override logic.
    pub requested_model: String,
    /// Agent role requesting the model.
    pub role: String,
    /// Task complexity label used for routing.
    pub task_complexity: String,
    /// Final provider selected for dispatch.
    pub selected_provider: String,
    /// Final model selected for dispatch.
    pub selected_model: String,
    /// Routing stage responsible for the base decision.
    pub routing_stage: String,
    /// Human-readable machine-parsable reason for the final decision.
    pub routing_reason: String,
    /// Candidate set considered during routing.
    pub candidates: Vec<CandidateEntry>,
    /// Whether the routed turn ultimately succeeded.
    pub outcome_success: Option<bool>,
    /// Final observed turn cost in USD.
    pub outcome_cost_usd: Option<f64>,
    /// Final observed turn latency in milliseconds.
    pub outcome_latency_ms: Option<u64>,
}

impl RoutingDecisionLog {
    /// Return a clone with terminal outcome fields populated.
    #[must_use]
    pub fn with_outcome(mut self, success: bool, cost_usd: f64, latency_ms: u64) -> Self {
        self.outcome_success = Some(success);
        self.outcome_cost_usd = Some(cost_usd);
        self.outcome_latency_ms = Some(latency_ms);
        self
    }
}

/// One candidate model score from the routing decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateEntry {
    /// Candidate model slug.
    pub model: String,
    /// Provider backing the model.
    pub provider: String,
    /// Stage-specific candidate score.
    pub score: f64,
    /// Optional reason the candidate could not be selected.
    pub disqualified: Option<String>,
}

/// Append-only JSONL log for [`RoutingDecisionLog`] values.
#[derive(Debug, Clone)]
pub struct RoutingDecisionLogStore {
    path: PathBuf,
    fsync: bool,
}

impl RoutingDecisionLogStore {
    /// Construct a log at `path`.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            fsync: true,
        }
    }

    /// Create parent directories and return a log at `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when parent directories cannot be created.
    pub async fn open_creating(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        Ok(Self { path, fsync: true })
    }

    /// Path to the underlying JSONL file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Disable fsync after appends.
    #[must_use]
    pub const fn without_fsync(mut self) -> Self {
        self.fsync = false;
        self
    }

    /// Append one [`RoutingDecisionLog`] as one JSON line.
    ///
    /// # Errors
    ///
    /// Returns an error for serialization or file I/O failures.
    pub async fn append(&self, record: &RoutingDecisionLog) -> io::Result<()> {
        let mut line = serde_json::to_string(record)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(line.as_bytes()).await?;
        if self.fsync {
            file.sync_data().await?;
        }
        Ok(())
    }

    /// Read all valid records; malformed lines are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error only for file open/read failures.
    pub async fn read_all(&self) -> io::Result<Vec<RoutingDecisionLog>> {
        let file = match tokio::fs::File::open(&self.path).await {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut out = Vec::new();
        while let Some(line) = lines.next_line().await? {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(record) = serde_json::from_str::<RoutingDecisionLog>(trimmed) {
                out.push(record);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{CandidateEntry, RoutingDecisionLog, RoutingDecisionLogStore};
    use tempfile::TempDir;

    fn record() -> RoutingDecisionLog {
        RoutingDecisionLog {
            timestamp: "2026-04-12T08:30:00Z".to_string(),
            trace_id: "trace-123".to_string(),
            task_id: "task-2m13".to_string(),
            requested_model: "kimi-k2.5".to_string(),
            role: "implementer".to_string(),
            task_complexity: "architectural".to_string(),
            selected_provider: "zai".to_string(),
            selected_model: "glm-5.1".to_string(),
            routing_stage: "ucb".to_string(),
            routing_reason: "highest_ucb_score".to_string(),
            candidates: vec![
                CandidateEntry {
                    model: "glm-5.1".to_string(),
                    provider: "zai".to_string(),
                    score: 0.91,
                    disqualified: None,
                },
                CandidateEntry {
                    model: "kimi-k2.5".to_string(),
                    provider: "moonshot".to_string(),
                    score: 0.77,
                    disqualified: Some("provider_unhealthy".to_string()),
                },
            ],
            outcome_success: None,
            outcome_cost_usd: None,
            outcome_latency_ms: None,
        }
    }

    #[tokio::test]
    async fn routing_decision_log_append_and_read_roundtrip() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("routing.jsonl");
        let log = RoutingDecisionLogStore::at(&path).without_fsync();
        let pending = record();
        let completed = pending.clone().with_outcome(true, 0.42, 1_250);

        log.append(&pending).await.expect("append pending");
        log.append(&completed).await.expect("append completed");

        let all = log.read_all().await.expect("read all");
        assert_eq!(all, vec![pending, completed]);
    }

    #[tokio::test]
    async fn routing_decision_log_read_all_skips_malformed_lines() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("routing.jsonl");
        let record = record();

        tokio::fs::write(
            &path,
            format!(
                "{}\n{}\n{}\n",
                serde_json::to_string(&record).expect("serialize"),
                "{ bad json",
                serde_json::to_string(&record.with_outcome(false, 0.1, 250)).expect("serialize"),
            ),
        )
        .await
        .expect("write log");

        let log = RoutingDecisionLogStore::at(&path).without_fsync();
        let all = log.read_all().await.expect("read all");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].trace_id, "trace-123");
        assert_eq!(all[1].outcome_success, Some(false));
    }
}
