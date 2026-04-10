//! Standard trait implementations for Roko.
//!
//! This crate provides:
//!
//! - [`MemorySubstrate`] — an in-memory `Substrate` for tests and ephemeral state
//! - **`NoOp` impls** of all six traits (useful as defaults and testing scaffolds)
//! - **Composite scorers** — `SumScorer`, `MulScorer`, `ConstScorer`
//! - **Simple routers** — `FirstRouter`, `HighestScoreRouter`, `RoundRobinRouter`
//!
//! These are the "kernel-adjacent" implementations that every Roko deployment
//! needs. Concrete domain impls (gates, agents, prompt composers) live in
//! their own crates.

#![allow(clippy::module_name_repetitions)]

pub mod memory;
pub mod noop;
pub mod roles;
pub mod router;
pub mod scorer;
pub mod tool;
pub mod trace_sink;

pub use memory::MemorySubstrate;
pub use noop::{NoOpComposer, NoOpGate, NoOpPolicy, NoOpRouter, NoOpScorer};
pub use roles::{
    IMPLEMENTER_TOOL_PROFILE, RESEARCHER_TOOL_PROFILE, REVIEWER_TOOL_PROFILE, RoleToolProfile,
    RoleToolProfileKind, SCRIBE_TOOL_PROFILE, STRATEGIST_TOOL_PROFILE, denied_tools_for_role,
};
pub use router::{FirstRouter, HighestScoreRouter, RoundRobinRouter};
pub use scorer::{ConstScorer, MulScorer, SumScorer};
pub use tool::{MockToolDispatcher, ROKO_BUILTIN_TOOLS, StaticToolRegistry, TOOL_COUNT};
pub use trace_sink::InMemoryTraceSink;
