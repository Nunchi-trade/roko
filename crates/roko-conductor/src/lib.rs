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
//! Every watcher is a pure function: `&[Signal] -> Vec<Signal>`. Watchers
//! have no side effects. The conductor collects watcher outputs, maps them
//! through an [`InterventionPolicy`](interventions::InterventionPolicy),
//! and produces a single [`ConductorDecision`](roko_core::ConductorDecision).
//!
//! # Re-exports
//!
//! The canonical [`PlanPhase`], [`PhaseKind`], and [`ConductorDecision`]
//! types live in `roko-core`. This crate re-exports them for convenience.

pub mod circuit_breaker;
pub mod conductor;
pub mod diagnosis;
pub mod health;
pub mod interventions;
pub mod state_machine;
pub mod stuck_detection;
pub mod watchers;

// Re-export core types for convenience.
pub use roko_core::{ConductorDecision, PhaseKind, PlanPhase};

// Re-export primary types from this crate.
pub use circuit_breaker::CircuitBreaker;
pub use conductor::Conductor;
pub use interventions::{InterventionPolicy, Severity, WatcherOutput, WorstSeverityPolicy};
pub use state_machine::{PhaseTransition, phase_timeout};
