//! Scenario trait + registry.
//!
//! Each concrete scenario module implements [`Scenario`] which owns its
//! scripted spine (the deterministic backbone) and any `rust` fixture handlers.
//!
//! Scenarios spawn their worker/poster/validator "agents" as **tokio tasks**
//! inside the `roko-demo` process — no subprocess fan-out. The LLM "leaves"
//! are pluggable via a [`LlmProvider`] trait; the in-process default is a
//! deterministic stub that produces bounded-random structured output so
//! scenarios run headless in CI.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::chain_ctx::ChainCtx;
use crate::events::EventEmitter;
use crate::fixtures::FixtureRegistry;
use crate::manifest::Scenario as ScenarioManifest;

pub mod consortium;
pub mod defi_routing;
pub mod flywheel;
pub mod job_board;
pub mod llm;
pub mod yield_routing;

pub use llm::{LlmProvider, StubLlm, create_provider};

/// Runtime values that are shared across scenario entrypoints.
pub struct ScenarioRuntime {
    /// LLM backend used by the scenario.
    pub llm: Arc<dyn LlmProvider>,
    /// Event sink used by the scenario.
    pub events: Arc<dyn EventEmitter>,
    /// Runtime artifact directory from the CLI.
    pub runtime_dir: PathBuf,
    /// Whether worker reputation snapshots should be restored/saved.
    pub persist_reputation: bool,
}

/// Scripted-spine lifecycle.
#[async_trait]
pub trait Scenario: Send + Sync {
    /// Canonical name (matches manifest entry).
    fn name(&self) -> &'static str;
    /// Register rust-kind fixture handlers.
    fn register_fixtures(&self, _registry: &mut FixtureRegistry) {}
    /// Run the scripted spine: spawn agents, drive actions, wait for success criteria.
    async fn spine(
        &self,
        ctx: Arc<ChainCtx>,
        manifest: &ScenarioManifest,
        runtime: Arc<ScenarioRuntime>,
    ) -> anyhow::Result<()>;
}

/// Look up a scenario implementation by name.
pub fn all() -> Vec<Box<dyn Scenario>> {
    vec![
        Box::new(job_board::JobBoard),
        Box::new(consortium::Consortium),
        Box::new(defi_routing::DefiRouting),
        Box::new(flywheel::Flywheel),
        Box::new(yield_routing::YieldRouting),
    ]
}

/// Find a concrete scenario by name.
pub fn find(name: &str) -> Option<Box<dyn Scenario>> {
    all().into_iter().find(|s| s.name() == name)
}
