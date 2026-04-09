//! Crash-recovery snapshot for the executor.
//!
//! [`ExecutorSnapshot`] captures the full mutable state of a
//! [`ParallelExecutor`](super::ParallelExecutor) so it can be serialized
//! to disk and restored after a crash or restart. The snapshot is designed
//! to be written atomically (write-to-temp + rename) by the persistence
//! layer.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::plan_state::PlanState;

/// Serializable snapshot of the entire executor state.
///
/// The runtime writes this periodically (or on every significant event)
/// to `.roko/state/executor.json`. On startup, if the file exists, the
/// executor restores from it and resumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorSnapshot {
    /// Per-plan mutable state, keyed by `plan_id`.
    #[serde(default)]
    pub plan_states: HashMap<String, PlanState>,
    /// Queue order: `plan_id`s in execution priority order.
    #[serde(default)]
    pub queue_order: Vec<String>,
    /// Unix millisecond timestamp when the snapshot was taken.
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl ExecutorSnapshot {
    /// Create an empty snapshot at the given timestamp.
    #[must_use]
    pub fn new(timestamp_ms: u64) -> Self {
        Self {
            plan_states: HashMap::new(),
            queue_order: Vec::new(),
            timestamp_ms,
        }
    }

    /// Serialize to JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails (should not happen for these types).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the JSON is malformed or missing required fields.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Number of plans in the snapshot.
    #[must_use]
    pub fn plan_count(&self) -> usize {
        self.plan_states.len()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use roko_core::PlanPhase;

    #[test]
    fn empty_snapshot_roundtrips() {
        let snap = ExecutorSnapshot::new(1000);
        let json = snap.to_json().unwrap();
        let restored = ExecutorSnapshot::from_json(&json).unwrap();
        assert_eq!(restored.timestamp_ms, 1000);
        assert!(restored.plan_states.is_empty());
        assert!(restored.queue_order.is_empty());
    }

    #[test]
    fn snapshot_with_plans_roundtrips() {
        let mut snap = ExecutorSnapshot::new(42_000);
        let mut ps = PlanState::new("plan-1");
        ps.current_phase = PlanPhase::Implementing;
        ps.iteration = 2;
        snap.plan_states.insert("plan-1".into(), ps);

        let mut ps2 = PlanState::new("plan-2");
        ps2.current_phase = PlanPhase::Gating;
        snap.plan_states.insert("plan-2".into(), ps2);
        snap.queue_order = vec!["plan-1".into(), "plan-2".into()];

        let json = snap.to_json().unwrap();
        let restored = ExecutorSnapshot::from_json(&json).unwrap();
        assert_eq!(restored.plan_count(), 2);
        assert_eq!(restored.queue_order.len(), 2);
        assert_eq!(
            restored.plan_states["plan-1"].current_phase,
            PlanPhase::Implementing
        );
        assert_eq!(restored.plan_states["plan-1"].iteration, 2);
        assert_eq!(
            restored.plan_states["plan-2"].current_phase,
            PlanPhase::Gating
        );
    }

    #[test]
    fn snapshot_preserves_queue_order() {
        let mut snap = ExecutorSnapshot::new(0);
        snap.queue_order = vec!["c".into(), "a".into(), "b".into()];
        let json = snap.to_json().unwrap();
        let restored = ExecutorSnapshot::from_json(&json).unwrap();
        assert_eq!(restored.queue_order, vec!["c", "a", "b"]);
    }

    #[test]
    fn plan_count_matches_states() {
        let mut snap = ExecutorSnapshot::new(0);
        assert_eq!(snap.plan_count(), 0);
        snap.plan_states.insert("a".into(), PlanState::new("a"));
        snap.plan_states.insert("b".into(), PlanState::new("b"));
        assert_eq!(snap.plan_count(), 2);
    }

    #[test]
    fn from_json_rejects_garbage() {
        assert!(ExecutorSnapshot::from_json("not json").is_err());
    }

    #[test]
    fn snapshot_with_terminal_plan() {
        let mut snap = ExecutorSnapshot::new(99);
        let mut ps = PlanState::new("done-plan");
        ps.current_phase = PlanPhase::Complete;
        snap.plan_states.insert("done-plan".into(), ps);
        snap.queue_order = vec!["done-plan".into()];

        let json = snap.to_json().unwrap();
        let restored = ExecutorSnapshot::from_json(&json).unwrap();
        assert!(restored.plan_states["done-plan"].is_terminal());
    }
}
