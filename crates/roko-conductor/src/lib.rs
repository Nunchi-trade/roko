//! Roko conductor — the reactive intelligence layer.
//!
//! The conductor watches signal streams and decides when to intervene:
//! restart an agent, change model, or abort a plan. It is composed of:
//!
//! - **State machine** — phase timeouts and transition records
//! - **Interventions** — severity classification and escalation policies
//! - **Circuit breaker** — per-plan failure budget tracking
//! - **Watchers** — 10 specialized anomaly detectors, each a [`Policy`] impl
//! - **Conductor** — composite Policy that runs all watchers
//!
//! # Architecture
//!
//! Every watcher is a pure function: `&[Engram] -> Vec<Engram>`. Watchers
//! have no side effects. The conductor collects watcher outputs, maps them
//! through an [`InterventionPolicy`](interventions::InterventionPolicy),
//! and produces a single [`ConductorDecision`](roko_core::ConductorDecision).
//!
//! # Re-exports
//!
//! The canonical [`PlanPhase`], [`PhaseKind`], and [`ConductorDecision`]
//! types live in `roko-core`. This crate re-exports them for convenience.

#![allow(
    clippy::assigning_clones,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::derive_partial_eq_without_eq,
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::for_kv_map,
    clippy::if_not_else,
    clippy::manual_let_else,
    clippy::missing_const_for_fn,
    clippy::needless_borrows_for_generic_args,
    clippy::redundant_closure_for_method_calls,
    clippy::significant_drop_tightening,
    clippy::single_match,
    clippy::suboptimal_flops,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::unnecessary_literal_bound,
    clippy::unnested_or_patterns
)]

pub mod circuit_breaker;
pub mod conductor;
pub mod diagnosis;
/// Federated conductor hierarchy — L1 turn → L4 fleet (COND-05).
pub mod federation;
pub mod health;
pub mod interventions;
/// Complex event pattern detection with temporal hysteresis (COND-07).
pub mod pattern_detector;
/// Self-healing conductor recovery strategies (COND-06).
pub mod self_healing;
pub mod state_machine;
pub mod stuck_detection;
pub mod threshold_learner;
pub mod watchers;
/// Yerkes-Dodson pressure-performance framework (COND-04).
pub mod yerkes_dodson;

// Re-export core types for convenience.
pub use roko_core::{
    CognitiveSignal, ConductorDecision, ConductorEvaluation, PhaseKind, PlanPhase,
};

// Re-export primary types from this crate.
pub use circuit_breaker::{
    CircuitBreaker, CircuitBreakerState, FailureRecord, HoltForecaster, ProactiveTripSignal,
};
pub use conductor::{Conductor, RoutingBias};
pub use diagnosis::{
    Diagnosis, DiagnosisEngine, DiagnosisResult, ErrorCategory, ErrorPattern, SuggestedIntervention,
};
pub use health::{HealthCheckResult, HealthMonitor, HealthStatus, SystemSnapshot};
pub use interventions::{
    BanditPolicy, InterventionPolicy, Severity, WatcherOutput, WorstSeverityPolicy,
};
pub use pattern_detector::{CompoundPattern, PatternDetector, WatcherFamily};
pub use self_healing::{HealingAction, SelfHealingPolicy, SelfHealingState};
pub use state_machine::{PhaseTransition, phase_timeout};
pub use stuck_detection::{
    ActivityEntry, MetaCognitionAction, MetaCognitionAssessment, MetaCognitionHook, StuckDetector,
    StuckKind, StuckSignal, StuckThresholds,
};
pub use threshold_learner::{AdaptiveThreshold, InterventionOutcome, ThresholdLearner};
pub use yerkes_dodson::YerkesDodson;
