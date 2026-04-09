//! Daimon subsystem crate.
//!
//! This crate re-exports the existing scaffolded daimon engine from
//! `roko-golem` so the workspace can depend on a dedicated daimon package
//! without duplicating the placeholder implementation.

pub use roko_golem::{
    AffectEngine, DaimonEngine, GolemSubsystemId, GolemSubsystemSummary, ScaffoldEngine,
};
