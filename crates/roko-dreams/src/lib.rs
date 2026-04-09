//! Dreams subsystem crate.
//!
//! This crate re-exports the scaffolded dreams engine from `roko-golem` so
//! the workspace can depend on a dedicated dreams package without duplicating
//! the placeholder implementation.

pub mod cycle;

pub use roko_golem::{
    DreamsEngine, GolemSubsystemId, GolemSubsystemSummary, ScaffoldEngine,
};
pub use cycle::{AgentDispatcher, DreamCycle, DreamCycleReport};
