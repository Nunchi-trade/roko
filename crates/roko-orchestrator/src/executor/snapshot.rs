//! Crash-recovery snapshot for the executor.
//!
//! [`ExecutorSnapshot`] captures the full mutable state of a
//! [`ParallelExecutor`](super::ParallelExecutor) so it can be serialized
//! to disk and restored after a crash or restart. The snapshot is designed
//! to be written atomically (write-to-temp + rename) by the persistence
//! layer.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use roko_core::PlanPhase;

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
    /// Falls back to a legacy `tasks`-based schema if the current
    /// `plan_states` layout is unavailable.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        // Peek at the raw value: if it has a `tasks` key but no
        // `plan_states`, it is a legacy snapshot and should use the
        // compat loader even though the primary path would succeed
        // (all fields are `#[serde(default)]`).
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(json) {
            if value.get("tasks").is_some() && value.get("plan_states").is_none() {
                return Self::from_legacy_json(json);
            }
        }
        match serde_json::from_str(json) {
            Ok(snapshot) => Ok(snapshot),
            Err(primary) => Self::from_legacy_json(json).or(Err(primary)),
        }
    }

    /// Number of plans in the snapshot.
    #[must_use]
    pub fn plan_count(&self) -> usize {
        self.plan_states.len()
    }

    fn from_legacy_json(json: &str) -> Result<Self, serde_json::Error> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        let Some(tasks) = value.get("tasks").and_then(|tasks| tasks.as_array()) else {
            return Err(serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "missing plan_states or tasks",
            )));
        };

        let timestamp_ms = value
            .get("timestamp_ms")
            .and_then(|timestamp| timestamp.as_u64())
            .unwrap_or(0);

        let mut plan_states: HashMap<String, PlanState> = HashMap::new();
        let mut queue_order: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        let mut plan_stats: HashMap<String, (usize, usize, bool)> = HashMap::new();

        for task in tasks {
            let plan_id = task
                .get("plan")
                .or_else(|| task.get("plan_id"))
                .and_then(|plan| plan.as_str())
                .unwrap_or_default();
            if plan_id.is_empty() {
                continue;
            }

            if seen.insert(plan_id.to_string()) {
                queue_order.push(plan_id.to_string());
            }

            let status = task
                .get("status")
                .and_then(|status| status.as_str())
                .map(|status| status.to_ascii_lowercase())
                .unwrap_or_default();

            let entry = plan_stats
                .entry(plan_id.to_string())
                .or_insert((0usize, 0usize, false));
            entry.0 += 1;
            if matches!(status.as_str(), "done" | "complete" | "completed") {
                entry.1 += 1;
            } else {
                entry.2 = true;
            }
        }

        for (plan_id, (total, done, has_active)) in plan_stats {
            let mut plan_state = PlanState::new(plan_id.clone());
            if total > 0 && done == total {
                plan_state.current_phase = PlanPhase::Complete;
            } else if done > 0 || has_active {
                plan_state.current_phase = PlanPhase::Implementing;
            }
            plan_states.insert(plan_id, plan_state);
        }

        if let Some(order) = value.get("queue_order").and_then(|order| order.as_array()) {
            let legacy_order = order
                .iter()
                .filter_map(|entry| entry.as_str().map(String::from))
                .collect::<Vec<_>>();
            if !legacy_order.is_empty() {
                queue_order = legacy_order;
            }
        }

        Ok(Self {
            plan_states,
            queue_order,
            timestamp_ms,
        })
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
    fn snapshot_with_partial_plan_state_uses_defaults() {
        // PlanPhase uses `#[serde(tag = "kind", rename_all = "kebab-case")]`,
        // so it is internally tagged: `{"kind": "queued"}` not `"Queued"`.
        let json = r#"
        {
            "plan_states": {
                "plan-1": {
                    "plan_id": "plan-1",
                    "current_phase": {"kind": "queued"}
                }
            },
            "queue_order": ["plan-1"]
        }
        "#;

        let restored = ExecutorSnapshot::from_json(json).unwrap();
        let ps = &restored.plan_states["plan-1"];
        assert_eq!(ps.plan_id, "plan-1");
        assert_eq!(ps.current_phase, PlanPhase::Queued);
        assert!(ps.assigned_agents.is_empty());
        assert!(ps.gate_results.is_empty());
        assert_eq!(ps.iteration, 1);
        assert_eq!(ps.started_at_ms, 0);
        assert!(ps.files_changed.is_empty());
        assert_eq!(ps.merge_attempts, 0);
        assert!(ps.last_error.is_none());
        assert!(!ps.paused);
        assert_eq!(ps.priority, 0);
    }

    #[test]
    fn legacy_task_snapshot_falls_back_to_compat_loader() {
        let json = r#"
        {
            "tasks": [
                { "id": "task-1", "status": "done", "plan": "plan-a" },
                { "id": "task-2", "status": "running", "plan": "plan-a" },
                { "id": "task-3", "status": "complete", "plan": "plan-b" }
            ],
            "queue_order": ["plan-b", "plan-a"],
            "timestamp_ms": 42
        }
        "#;

        let restored = ExecutorSnapshot::from_json(json).unwrap();
        assert_eq!(restored.timestamp_ms, 42);
        assert_eq!(restored.queue_order, vec!["plan-b", "plan-a"]);
        assert_eq!(restored.plan_states.len(), 2);
        assert_eq!(
            restored.plan_states["plan-a"].current_phase,
            PlanPhase::Implementing
        );
        assert_eq!(
            restored.plan_states["plan-b"].current_phase,
            PlanPhase::Complete
        );
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
