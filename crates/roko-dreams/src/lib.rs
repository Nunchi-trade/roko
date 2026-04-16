//! Dreams subsystem crate.
//!
//! This package owns the dream-cycle runtime, heartbeat helpers, and the
//! remaining placeholder subsystem types that still live in the dreams domain.

pub mod cycle;
pub mod hypnagogia;
pub mod imagination;
pub mod replay;
pub mod runner;
pub mod threat;

pub use cycle::{AgentDispatcher, DreamCycle, DreamCycleReport};
pub use hypnagogia::{
    DaliInterrupt, ExecutiveLoosener, HomuncularObserver, HypnagogiaEngine, ThalamicGate,
};
pub use imagination::{
    CausalModel, CounterfactualQuery, ImaginationMode, ImaginationOutcome, counterfactual_episode,
    imagine, synthesize_hypotheses,
};
pub use replay::{DreamReplayBatch, DreamReplayMode, DreamReplayPolicy, select_replay_episodes};
pub use runner::{
    DreamAgentConfig, DreamBudget, DreamConfig, DreamEngine, DreamHeartbeatPolicy,
    DreamHeartbeatReport, DreamLoopConfig, DreamReport, DreamRunner, DreamRuntimeControls,
    DreamSchedulePolicy, DreamTrigger, Episode, Insight, build_dream_review_dispatcher,
};
pub use threat::{ThreatScenario, enumerate_threats, threat_warning_entries};

/// Stable subsystem identifiers still surfaced by the dreams crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DreamsSubsystemId {
    /// Replay and imagination loop.
    Dreams,
    /// Liminal interrupt / handoff state.
    Hypnagogia,
}

/// Summary metadata for one dreams subsystem placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DreamsSubsystemSummary {
    /// Stable subsystem identifier.
    pub id: DreamsSubsystemId,
    /// Human-readable label.
    pub label: &'static str,
    /// Static marker string describing placeholder behavior.
    pub marker: &'static str,
}

impl DreamsSubsystemSummary {
    /// Construct a subsystem summary.
    #[must_use]
    pub const fn new(id: DreamsSubsystemId, label: &'static str, marker: &'static str) -> Self {
        Self { id, label, marker }
    }
}

/// Dreams engine facade for replay, scheduling, and consolidation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DreamsEngine;

impl DreamsEngine {
    /// Stable subsystem id.
    pub const ID: DreamsSubsystemId = DreamsSubsystemId::Dreams;
    /// Human-readable subsystem label.
    pub const LABEL: &'static str = "Dreams";
    /// Static marker string for compatibility with older summaries.
    pub const MARKER: &'static str = "roko-dreams subsystem: dreams";

    /// Construct a dreams engine facade.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Summary metadata for this subsystem placeholder.
    #[must_use]
    pub const fn summary(self) -> DreamsSubsystemSummary {
        DreamsSubsystemSummary::new(Self::ID, Self::LABEL, Self::MARKER)
    }

    /// Returns a static marker describing the subsystem.
    #[must_use]
    pub const fn replay(self) -> &'static str {
        Self::MARKER
    }
}
