//! Projection contract hardening (SURF-GAP-05).
//!
//! Defines versioning, invalidation rules, and restart recovery behavior
//! for StateHub projections consumed by TUI and dashboard surfaces.
//!
//! Every projection response can be wrapped in a [`ProjectionEnvelope`] that
//! carries schema version, monotonic cursor, staleness information, and a
//! flag indicating whether the data was recovered from disk after a restart.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Envelope
// ---------------------------------------------------------------------------

/// Metadata envelope for a projection response.
///
/// Wraps the actual projection data with versioning and freshness metadata
/// so that consumers can detect stale data, handle schema changes, and
/// distinguish recovered (post-restart) state from live state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionEnvelope<T> {
    /// Projection name (e.g., `"marketplace_jobs"`, `"active_tasks"`).
    pub name: String,
    /// Schema version for this projection format.
    ///
    /// Bumped when the shape of the inner `data` changes in a breaking way.
    /// Consumers should reject versions they do not understand.
    pub version: u32,
    /// Monotonic sequence number from the StateHub event bus.
    pub cursor: u64,
    /// When this snapshot was computed (RFC 3339).
    pub computed_at: String,
    /// Whether this projection was loaded from recovery (restart) state
    /// rather than built from a live event stream.
    pub recovered: bool,
    /// The actual projection data.
    pub data: T,
}

// ---------------------------------------------------------------------------
// Invalidation policy
// ---------------------------------------------------------------------------

/// Cache invalidation policy for a projection.
///
/// Describes how long a projection's cached value remains valid and which
/// events trigger eager invalidation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvalidationPolicy {
    /// Maximum age in seconds before the projection is considered stale.
    pub max_age_secs: u64,
    /// Whether the projection supports incremental (delta) updates.
    pub incremental: bool,
    /// Event types that trigger invalidation of this projection.
    pub invalidation_triggers: Vec<String>,
}

/// Catalog entry returned by `GET /api/projections/catalog`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionCatalogEntry {
    /// Projection name.
    pub name: String,
    /// Current schema version.
    pub version: u32,
    /// Invalidation policy.
    pub policy: InvalidationPolicy,
}

// ---------------------------------------------------------------------------
// Known projections
// ---------------------------------------------------------------------------

/// Returns the known projection names with their current schema versions
/// and invalidation policies.
pub fn projection_policies() -> Vec<ProjectionCatalogEntry> {
    vec![
        ProjectionCatalogEntry {
            name: "dashboard".into(),
            version: 1,
            policy: InvalidationPolicy {
                max_age_secs: 5,
                incremental: true,
                invalidation_triggers: vec!["*".into()],
            },
        },
        ProjectionCatalogEntry {
            name: "agent_state".into(),
            version: 1,
            policy: InvalidationPolicy {
                max_age_secs: 5,
                incremental: true,
                invalidation_triggers: vec![
                    "agent_spawned".into(),
                    "agent_output".into(),
                    "agent_completed".into(),
                ],
            },
        },
        ProjectionCatalogEntry {
            name: "plan_state".into(),
            version: 1,
            policy: InvalidationPolicy {
                max_age_secs: 5,
                incremental: true,
                invalidation_triggers: vec![
                    "plan_started".into(),
                    "plan_completed".into(),
                    "task_started".into(),
                    "task_completed".into(),
                    "task_phase_changed".into(),
                    "phase_transition".into(),
                ],
            },
        },
        ProjectionCatalogEntry {
            name: "gate_state".into(),
            version: 1,
            policy: InvalidationPolicy {
                max_age_secs: 10,
                incremental: true,
                invalidation_triggers: vec!["gate_result".into(), "gate_thresholds_updated".into()],
            },
        },
        ProjectionCatalogEntry {
            name: "learning_policy_state".into(),
            version: 1,
            policy: InvalidationPolicy {
                max_age_secs: 30,
                incremental: true,
                invalidation_triggers: vec![
                    "experiment_winners_updated".into(),
                    "cfactor_trend_updated".into(),
                    "efficiency_event".into(),
                    "episode_recorded".into(),
                    "cascade_router_updated".into(),
                    "gate_thresholds_updated".into(),
                ],
            },
        },
        ProjectionCatalogEntry {
            name: "cohort_health".into(),
            version: 1,
            policy: InvalidationPolicy {
                max_age_secs: 10,
                incremental: true,
                invalidation_triggers: vec![
                    "plan_started".into(),
                    "plan_completed".into(),
                    "task_started".into(),
                    "task_completed".into(),
                    "agent_spawned".into(),
                    "efficiency_event".into(),
                    "cfactor_trend_updated".into(),
                    "diagnosis".into(),
                    "error".into(),
                ],
            },
        },
        ProjectionCatalogEntry {
            name: "active_tasks".into(),
            version: 1,
            policy: InvalidationPolicy {
                max_age_secs: 5,
                incremental: true,
                invalidation_triggers: vec![
                    "task_started".into(),
                    "task_completed".into(),
                    "task_phase_changed".into(),
                ],
            },
        },
        ProjectionCatalogEntry {
            name: "gate_pipeline".into(),
            version: 1,
            policy: InvalidationPolicy {
                max_age_secs: 10,
                incremental: true,
                invalidation_triggers: vec!["gate_result".into()],
            },
        },
        ProjectionCatalogEntry {
            name: "agent_trails".into(),
            version: 1,
            policy: InvalidationPolicy {
                max_age_secs: 5,
                incremental: true,
                invalidation_triggers: vec!["agent_spawned".into(), "agent_output".into()],
            },
        },
        ProjectionCatalogEntry {
            name: "alerts".into(),
            version: 1,
            policy: InvalidationPolicy {
                max_age_secs: 15,
                incremental: true,
                invalidation_triggers: vec![
                    "diagnosis".into(),
                    "gate_result".into(),
                    "error".into(),
                ],
            },
        },
        ProjectionCatalogEntry {
            name: "plans_list".into(),
            version: 1,
            policy: InvalidationPolicy {
                max_age_secs: 10,
                incremental: true,
                invalidation_triggers: vec![
                    "plan_started".into(),
                    "plan_completed".into(),
                    "phase_transition".into(),
                ],
            },
        },
        ProjectionCatalogEntry {
            name: "recent_episodes".into(),
            version: 1,
            policy: InvalidationPolicy {
                max_age_secs: 30,
                incremental: true,
                invalidation_triggers: vec!["episode_recorded".into()],
            },
        },
        ProjectionCatalogEntry {
            name: "marketplace_jobs".into(),
            version: 2, // v2: includes `source` field (SURF-GAP-04)
            policy: InvalidationPolicy {
                max_age_secs: 30,
                incremental: true,
                invalidation_triggers: vec!["marketplace_jobs_updated".into()],
            },
        },
        ProjectionCatalogEntry {
            name: "prds".into(),
            version: 1,
            policy: InvalidationPolicy {
                max_age_secs: 60,
                incremental: true,
                invalidation_triggers: vec!["prds_updated".into()],
            },
        },
        ProjectionCatalogEntry {
            name: "knowledge".into(),
            version: 1,
            policy: InvalidationPolicy {
                max_age_secs: 60,
                incremental: true,
                invalidation_triggers: vec!["knowledge_entries_updated".into()],
            },
        },
    ]
}

/// Look up the schema version for a named projection.
///
/// Returns `None` for unknown projections.
pub fn projection_version(name: &str) -> Option<u32> {
    // Normalise aliases.
    let canonical = match name {
        "dashboard_snapshot" => "dashboard",
        "agents" | "agent_trails" => "agent_state",
        "plans" | "plans_list" => "plan_state",
        "gates" | "gate_pipeline" => "gate_state",
        "learning" | "learning_policy" => "learning_policy_state",
        "jobs" => "marketplace_jobs",
        "atelier" => "prds",
        "knowledge_entries" => "knowledge",
        _ => name,
    };
    projection_policies()
        .iter()
        .find(|entry| entry.name == canonical)
        .map(|entry| entry.version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_policies_have_non_zero_max_age() {
        for entry in projection_policies() {
            assert!(
                entry.policy.max_age_secs > 0,
                "projection '{}' has zero max_age_secs",
                entry.name
            );
        }
    }

    #[test]
    fn all_policies_have_at_least_one_trigger() {
        for entry in projection_policies() {
            assert!(
                !entry.policy.invalidation_triggers.is_empty(),
                "projection '{}' has no invalidation triggers",
                entry.name
            );
        }
    }

    #[test]
    fn version_lookup_resolves_aliases() {
        assert_eq!(projection_version("marketplace_jobs"), Some(2));
        assert_eq!(projection_version("jobs"), Some(2));
        assert_eq!(projection_version("dashboard"), Some(1));
        assert_eq!(projection_version("dashboard_snapshot"), Some(1));
        assert_eq!(projection_version("agent_state"), Some(1));
        assert_eq!(projection_version("agent_trails"), Some(1));
        assert_eq!(projection_version("plan_state"), Some(1));
        assert_eq!(projection_version("plans_list"), Some(1));
        assert_eq!(projection_version("gate_state"), Some(1));
        assert_eq!(projection_version("gate_pipeline"), Some(1));
        assert_eq!(projection_version("learning_policy_state"), Some(1));
        assert_eq!(projection_version("learning_policy"), Some(1));
        assert_eq!(projection_version("atelier"), Some(1));
        assert_eq!(projection_version("knowledge_entries"), Some(1));
        assert_eq!(projection_version("nonexistent"), None);
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let envelope = ProjectionEnvelope {
            name: "test".into(),
            version: 1,
            cursor: 42,
            computed_at: "2026-04-21T00:00:00Z".into(),
            recovered: false,
            data: serde_json::json!({"items": []}),
        };
        let json = serde_json::to_string(&envelope).expect("serialize");
        let decoded: ProjectionEnvelope<serde_json::Value> =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.name, "test");
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.cursor, 42);
        assert!(!decoded.recovered);
    }
}
