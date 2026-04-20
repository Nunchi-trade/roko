//! Dreams subsystem crate.
//!
//! This package owns the shipping dream-cycle runtime, heartbeat helpers, and
//! compatibility metadata for the subsystem surfaces that still live in the
//! dreams domain.

#![allow(
    clippy::assigning_clones,
    clippy::bool_to_int_with_if,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::clone_on_copy,
    clippy::cloned_ref_to_slice_refs,
    clippy::derivable_impls,
    clippy::derive_partial_eq_without_eq,
    clippy::double_must_use,
    clippy::expect_used,
    clippy::format_push_string,
    clippy::if_not_else,
    clippy::iter_with_drain,
    clippy::manual_let_else,
    clippy::manual_midpoint,
    clippy::map_unwrap_or,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_value,
    clippy::option_if_let_else,
    clippy::or_fun_call,
    clippy::redundant_closure_for_method_calls,
    clippy::similar_names,
    clippy::suboptimal_flops,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::unnecessary_wraps,
    clippy::unused_async
)]

pub mod cycle;
pub mod hypnagogia;
pub mod imagination;
pub mod phase2;
pub mod rehearsal;
pub mod replay;
pub mod runner;
pub mod staging;
pub mod threat;

pub use cycle::{AgentDispatcher, DreamCycle, DreamCycleReport, PhaseBudgetSummary, StagingBufferStats};
pub use hypnagogia::{
    DaliInterrupt, ExecutiveLoosener, HomuncularObserver, HypnagogiaEngine, ThalamicGate,
};
pub use imagination::{
    CausalModel, CounterfactualQuery, ImaginationMode, ImaginationOutcome, counterfactual_episode,
    imagine, synthesize_hypotheses,
};
pub use replay::{
    DreamReplayBatch, DreamReplayMode, DreamReplayPolicy, MattarDawConfig, ReplayUtility,
    compute_replay_utility, select_replay_episodes, select_replay_episodes_with_affect,
};
pub use staging::{ConfidenceStage, StagingBuffer, StagingEntry};
pub use runner::{
    BusPulseTriggerConfig, DreamAgentConfig, DreamBudget, DreamConfig, DreamEngine,
    DreamHeartbeatPolicy, DreamHeartbeatReport, DreamLoopConfig, DreamReport, DreamRunner,
    DreamRuntimeControls, DreamSchedulePolicy, DreamTrigger, Episode, Insight, IntensiveMode,
    build_dream_review_dispatcher,
};
pub use rehearsal::{RehearsalOutcome, RehearsalReport, rehearse_threats};
pub use threat::{ThreatScenario, enumerate_threats, threat_warning_entries};

/// Stable subsystem identifiers still surfaced by the dreams crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DreamsSubsystemId {
    /// Replay and imagination loop.
    Dreams,
    /// Liminal interrupt / handoff state.
    Hypnagogia,
}

/// Summary metadata for one dreams subsystem compatibility surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DreamsSubsystemSummary {
    /// Stable subsystem identifier.
    pub id: DreamsSubsystemId,
    /// Human-readable label.
    pub label: &'static str,
    /// Static marker string describing compatibility behavior.
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

    /// Summary metadata for this subsystem compatibility surface.
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
