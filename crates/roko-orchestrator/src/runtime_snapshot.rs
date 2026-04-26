//! Aggregate snapshot for orchestrator-owned runtime state.
//!
//! [`OrchestratorSnapshot`] intentionally contains only state owned by this
//! crate: executor phases/queue, merge queue metadata, worktree registry, and
//! the optional event-log snapshot. Runner-owned projections, provider state,
//! learning caches, and dashboard state should be persisted separately.

use serde::{Deserialize, Serialize};

use crate::event_log::EventLogSnapshot;
use crate::executor::ExecutorSnapshot;
use crate::merge_queue::MergeQueueSnapshot;
use crate::worktree::WorktreeSnapshot;

/// Current schema version for [`OrchestratorSnapshot`].
pub const ORCHESTRATOR_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// Serializable checkpoint of all orchestrator-owned runtime metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorSnapshot {
    /// Version of this aggregate snapshot schema.
    #[serde(default = "orchestrator_snapshot_schema_version")]
    pub schema_version: u32,
    /// Executor state: plan phases, queue order, and speculative execution.
    pub executor: ExecutorSnapshot,
    /// Merge queue state, when a runner uses the queue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_queue: Option<MergeQueueSnapshot>,
    /// Worktree registry state, when worktree isolation is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktrees: Option<WorktreeSnapshot>,
    /// Optional tamper-evident event-log snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_log: Option<EventLogSnapshot>,
    /// Unix epoch milliseconds when the aggregate snapshot was produced.
    pub timestamp_ms: u64,
}

/// Return the current schema version for [`OrchestratorSnapshot`].
#[must_use]
pub const fn orchestrator_snapshot_schema_version() -> u32 {
    ORCHESTRATOR_SNAPSHOT_SCHEMA_VERSION
}

impl OrchestratorSnapshot {
    /// Create an aggregate snapshot with only executor state.
    #[must_use]
    pub fn new(executor: ExecutorSnapshot, timestamp_ms: u64) -> Self {
        Self {
            schema_version: orchestrator_snapshot_schema_version(),
            executor,
            merge_queue: None,
            worktrees: None,
            event_log: None,
            timestamp_ms,
        }
    }

    /// Attach merge queue metadata.
    #[must_use]
    pub fn with_merge_queue(mut self, merge_queue: MergeQueueSnapshot) -> Self {
        self.merge_queue = Some(merge_queue);
        self
    }

    /// Attach worktree registry metadata.
    #[must_use]
    pub fn with_worktrees(mut self, worktrees: WorktreeSnapshot) -> Self {
        self.worktrees = Some(worktrees);
        self
    }

    /// Attach event-log metadata.
    #[must_use]
    pub fn with_event_log(mut self, event_log: EventLogSnapshot) -> Self {
        self.event_log = Some(event_log);
        self
    }

    /// Serialize the aggregate snapshot as pretty JSON.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize an aggregate snapshot from JSON.
    ///
    /// # Errors
    ///
    /// Returns an error if the JSON is invalid for this schema.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Compute a deterministic BLAKE3 hash over the snapshot JSON value.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn compute_hash(&self) -> Result<[u8; 32], serde_json::Error> {
        let value = serde_json::to_value(self)?;
        let canonical = serde_json::to_vec(&value)?;
        Ok(*blake3::hash(&canonical).as_bytes())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::executor::ExecutorSnapshot;

    #[test]
    fn aggregate_snapshot_roundtrips() {
        let snapshot = OrchestratorSnapshot::new(ExecutorSnapshot::new(10), 10);
        let json = snapshot.to_json().unwrap();
        let restored = OrchestratorSnapshot::from_json(&json).unwrap();

        assert_eq!(
            restored.schema_version,
            ORCHESTRATOR_SNAPSHOT_SCHEMA_VERSION
        );
        assert_eq!(restored.timestamp_ms, 10);
        assert!(restored.merge_queue.is_none());
        assert!(restored.worktrees.is_none());
    }
}
