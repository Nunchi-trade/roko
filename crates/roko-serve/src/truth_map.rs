//! Backend truth-source registry.
//!
//! Every operator-visible entity has exactly one authoritative source.
//! This module documents and enforces those ownership rules so that
//! surfaces (CLI, TUI, HTTP, WebSocket) never diverge on state.

use std::fmt;

use serde::Serialize;

// ---------------------------------------------------------------------------
// TruthSource — where the authoritative data lives
// ---------------------------------------------------------------------------

/// Where the authoritative copy of an entity lives at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum TruthSource {
    /// Materialized snapshot inside [`SharedStateHub`](roko_core::SharedStateHub).
    /// Updated via `DashboardEvent` and served by the watch channel.
    StateHub,
    /// Durable files under `.roko/` (JSONL, JSON, TOML).
    /// Read on demand; bootstrap seeds StateHub at startup.
    Filesystem,
    /// Ephemeral in-memory maps on [`AppState`](crate::state::AppState)
    /// (active runs, discovered agents, aggregator cache, etc.).
    InMemory,
    /// Live process state managed by
    /// [`ProcessSupervisor`](roko_runtime::process::ProcessSupervisor)
    /// or the cancellation-token tree.
    Runtime,
}

impl fmt::Display for TruthSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StateHub => write!(f, "StateHub"),
            Self::Filesystem => write!(f, "Filesystem"),
            Self::InMemory => write!(f, "InMemory"),
            Self::Runtime => write!(f, "Runtime"),
        }
    }
}

// ---------------------------------------------------------------------------
// EntityKind — every operator-visible entity type
// ---------------------------------------------------------------------------

/// Exhaustive list of entity kinds surfaced to operators through any of the
/// four consumer surfaces (CLI, TUI, HTTP API, WebSocket).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum EntityKind {
    /// Marketplace job (`.roko/jobs/*.json` -> StateHub snapshot).
    Job,
    /// Plan definition (`.roko/plans/` or `plans/`).
    Plan,
    /// Active plan execution handle.
    PlanExecution,
    /// Discovered or supervised agent entry.
    Agent,
    /// Recorded episode (agent turn + gate result).
    Episode,
    /// Persisted signal / engram.
    Signal,
    /// Gate check result.
    GateResult,
    /// Cascade router state (model routing weights).
    CascadeRouter,
    /// Prompt A/B experiment store.
    Experiment,
    /// Per-turn efficiency events.
    Efficiency,
    /// Rolling C-Factor history.
    CFactorHistory,
    /// Adaptive gate threshold state.
    GateThresholds,
    /// PRD document (idea / draft / published).
    Prd,
    /// Cloud deployment record.
    Deployment,
    /// Provider health / liveness tracker.
    ProviderHealth,
    /// Prometheus-style metric.
    Metric,
}

impl fmt::Display for EntityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Job => "Job",
            Self::Plan => "Plan",
            Self::PlanExecution => "PlanExecution",
            Self::Agent => "Agent",
            Self::Episode => "Episode",
            Self::Signal => "Signal",
            Self::GateResult => "GateResult",
            Self::CascadeRouter => "CascadeRouter",
            Self::Experiment => "Experiment",
            Self::Efficiency => "Efficiency",
            Self::CFactorHistory => "CFactorHistory",
            Self::GateThresholds => "GateThresholds",
            Self::Prd => "Prd",
            Self::Deployment => "Deployment",
            Self::ProviderHealth => "ProviderHealth",
            Self::Metric => "Metric",
        };
        write!(f, "{label}")
    }
}

// ---------------------------------------------------------------------------
// EntityOwnership — the truth record for one entity kind
// ---------------------------------------------------------------------------

/// Documents the authoritative source, filesystem path, WebSocket event, and
/// projection key for a single entity kind.
///
/// The `truth_map()` function returns one entry per [`EntityKind`]. Route
/// handlers and TUI views should consult this registry to decide where to
/// read from, avoiding dual-source divergence.
#[derive(Debug, Clone, Serialize)]
pub struct EntityOwnership {
    /// Which entity this record describes.
    pub kind: EntityKind,
    /// Where the authoritative copy lives.
    pub source: TruthSource,
    /// Filesystem path pattern (relative to workdir) that holds the durable
    /// copy, or an `AppState` field name for in-memory entities.
    pub read_path: &'static str,
    /// WebSocket / SSE event name that notifies consumers of mutations.
    pub ws_event: Option<&'static str>,
    /// StateHub projection key (field on `DashboardSnapshot`) when the entity
    /// is materialized into the snapshot, or `None` if not projected.
    pub projection: Option<&'static str>,
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Return the complete truth-source registry.
///
/// One entry per [`EntityKind`]. The ordering matches the enum definition for
/// deterministic iteration.
#[must_use]
pub fn truth_map() -> Vec<EntityOwnership> {
    vec![
        EntityOwnership {
            kind: EntityKind::Job,
            source: TruthSource::StateHub,
            read_path: ".roko/jobs/*.json",
            ws_event: Some("job_created | job_updated"),
            projection: Some("marketplace_jobs"),
        },
        EntityOwnership {
            kind: EntityKind::Plan,
            source: TruthSource::Filesystem,
            read_path: ".roko/plans/ | plans/",
            ws_event: Some("plan_started | plan_completed"),
            projection: Some("plans"),
        },
        EntityOwnership {
            kind: EntityKind::PlanExecution,
            source: TruthSource::InMemory,
            read_path: "AppState::active_plans",
            ws_event: Some("execution"),
            projection: Some("plans"),
        },
        EntityOwnership {
            kind: EntityKind::Agent,
            source: TruthSource::InMemory,
            read_path: "AppState::discovered_agents",
            ws_event: Some("agent_spawned"),
            projection: Some("agents"),
        },
        EntityOwnership {
            kind: EntityKind::Episode,
            source: TruthSource::Filesystem,
            read_path: ".roko/episodes.jsonl",
            ws_event: Some("episode"),
            projection: Some("episodes"),
        },
        EntityOwnership {
            kind: EntityKind::Signal,
            source: TruthSource::Filesystem,
            read_path: ".roko/engrams.jsonl",
            ws_event: Some("webhook_received"),
            projection: Some("gates"),
        },
        EntityOwnership {
            kind: EntityKind::GateResult,
            source: TruthSource::StateHub,
            read_path: ".roko/engrams.jsonl",
            ws_event: Some("gate_result"),
            projection: Some("gates"),
        },
        EntityOwnership {
            kind: EntityKind::CascadeRouter,
            source: TruthSource::Filesystem,
            read_path: ".roko/learn/cascade-router.json",
            ws_event: None,
            projection: Some("cascade_router_json"),
        },
        EntityOwnership {
            kind: EntityKind::Experiment,
            source: TruthSource::Filesystem,
            read_path: ".roko/learn/experiments.json",
            ws_event: None,
            projection: Some("experiment_winners"),
        },
        EntityOwnership {
            kind: EntityKind::Efficiency,
            source: TruthSource::Filesystem,
            read_path: ".roko/learn/efficiency.jsonl",
            ws_event: Some("efficiency_event"),
            projection: Some("stats.tokens_total / stats.cost_usd_total"),
        },
        EntityOwnership {
            kind: EntityKind::CFactorHistory,
            source: TruthSource::Filesystem,
            read_path: ".roko/learn/c-factor.jsonl",
            ws_event: None,
            projection: Some("cfactor_trend"),
        },
        EntityOwnership {
            kind: EntityKind::GateThresholds,
            source: TruthSource::Filesystem,
            read_path: ".roko/learn/gate-thresholds.json",
            ws_event: None,
            projection: Some("gate_thresholds_json"),
        },
        EntityOwnership {
            kind: EntityKind::Prd,
            source: TruthSource::StateHub,
            read_path: ".roko/prd/",
            ws_event: None,
            projection: Some("prds / prd_tasks"),
        },
        EntityOwnership {
            kind: EntityKind::Deployment,
            source: TruthSource::InMemory,
            read_path: "AppState::deployments",
            ws_event: Some("deployment_created | deployment_ready | deployment_failed"),
            projection: None,
        },
        EntityOwnership {
            kind: EntityKind::ProviderHealth,
            source: TruthSource::InMemory,
            read_path: "AppState::provider_health",
            ws_event: None,
            projection: None,
        },
        EntityOwnership {
            kind: EntityKind::Metric,
            source: TruthSource::InMemory,
            read_path: "AppState::metrics",
            ws_event: None,
            projection: None,
        },
    ]
}

/// Look up the authoritative [`TruthSource`] for a given entity kind.
#[must_use]
pub fn entity_source(kind: EntityKind) -> TruthSource {
    // The map is small and fixed-size; a linear scan is fine.
    truth_map()
        .into_iter()
        .find(|entry| entry.kind == kind)
        .map(|entry| entry.source)
        .unwrap_or(TruthSource::Filesystem)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn truth_map_covers_all_entity_kinds() {
        let map = truth_map();
        let covered: HashSet<EntityKind> = map.iter().map(|entry| entry.kind).collect();

        let all_kinds = [
            EntityKind::Job,
            EntityKind::Plan,
            EntityKind::PlanExecution,
            EntityKind::Agent,
            EntityKind::Episode,
            EntityKind::Signal,
            EntityKind::GateResult,
            EntityKind::CascadeRouter,
            EntityKind::Experiment,
            EntityKind::Efficiency,
            EntityKind::CFactorHistory,
            EntityKind::GateThresholds,
            EntityKind::Prd,
            EntityKind::Deployment,
            EntityKind::ProviderHealth,
            EntityKind::Metric,
        ];

        for kind in &all_kinds {
            assert!(
                covered.contains(kind),
                "EntityKind::{kind} missing from truth_map()"
            );
        }
        assert_eq!(
            map.len(),
            all_kinds.len(),
            "truth_map has extra entries beyond known EntityKind variants"
        );
    }

    #[test]
    fn entity_source_returns_correct_source_for_known_kinds() {
        assert_eq!(entity_source(EntityKind::Job), TruthSource::StateHub);
        assert_eq!(entity_source(EntityKind::Plan), TruthSource::Filesystem);
        assert_eq!(
            entity_source(EntityKind::PlanExecution),
            TruthSource::InMemory
        );
        assert_eq!(entity_source(EntityKind::Agent), TruthSource::InMemory);
        assert_eq!(entity_source(EntityKind::Episode), TruthSource::Filesystem);
        assert_eq!(entity_source(EntityKind::Signal), TruthSource::Filesystem);
        assert_eq!(entity_source(EntityKind::GateResult), TruthSource::StateHub);
        assert_eq!(entity_source(EntityKind::Deployment), TruthSource::InMemory);
        assert_eq!(
            entity_source(EntityKind::ProviderHealth),
            TruthSource::InMemory
        );
        assert_eq!(entity_source(EntityKind::Metric), TruthSource::InMemory);
    }

    #[test]
    fn no_duplicate_entity_kinds_in_map() {
        let map = truth_map();
        let mut seen = HashSet::new();
        for entry in &map {
            assert!(
                seen.insert(entry.kind),
                "duplicate EntityKind::{} in truth_map()",
                entry.kind
            );
        }
    }

    #[test]
    fn all_entries_have_non_empty_read_path() {
        for entry in truth_map() {
            assert!(
                !entry.read_path.is_empty(),
                "EntityKind::{} has empty read_path",
                entry.kind
            );
        }
    }
}
