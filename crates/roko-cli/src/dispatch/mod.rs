//! Dispatch — the runner's single entry point for agent invocation.
//!
//! ## Architectural role
//!
//! In the unified Roko model the runner is a [`Compose → Route → Act`] loop
//! over `Signal`s. The dispatch module owns the **Route → Act** seam:
//!
//! - **Route**: pick a `ModelSpec` for the task by consulting
//!   [`CascadeRouter`] + any task / config overrides
//!   ([`model_routing`]).
//! - **Compose**: assemble a fully structured prompt
//!   ([`PromptAssembler`] in [`prompt_builder`]).
//! - **Act**: launch the resolved provider through
//!   [`AgentDispatcherV2`] / `roko-agent` and return a normalized
//!   [`AgentOutcome`] ([`outcome`]).
//!
//! This file is intentionally small: it composes the four submodules into
//! a [`Dispatcher`] facade that the runner can call, and a
//! [`DispatchContext`] value object that carries all the per-call inputs.
//!
//! Importantly, no provider-specific logic lives here. Every
//! provider concern (CLI args, stream parsing, http transport) is owned by
//! [`roko_agent::provider`]. The dispatcher only sees a `ModelSpec` and a
//! prompt; the act-step calls into provider code through the shared
//! [`AgentDispatcherV2`] resolver.
//!
//! ## Test seam
//!
//! [`Dispatcher::dispatch`] is async-trait based on a thin
//! [`AgentResultBridge`] that hides the provider for testing. Production
//! callers wire in [`AgentDispatcherV2`]; tests can plug in a stub bridge.

pub mod factory;
pub mod model_routing;
pub mod outcome;
pub mod prompt_builder;
pub mod prompt_cache;
pub mod warm_pool;

use std::sync::Arc;

use roko_agent::AgentRuntimeEvent;
use roko_core::agent::ModelSpec;
use roko_core::config::schema::RokoConfig;
use roko_learn::cascade_router::CascadeRouter;
use roko_learn::model_router::RoutingContext;
use tokio::sync::mpsc;

pub use factory::SharedAgentFactory;
pub use model_routing::{ModelChoice, ModelChoiceSource, ModelRouter, RoutingInputs};
pub use outcome::{AgentOutcome, DispatchError};
pub use prompt_builder::{
    AssembledPrompt, GateFeedback, PromptAssembler, PromptContext, PromptDiagnostics,
};
pub use prompt_cache::PromptCache;
pub use warm_pool::{WarmPool, WarmPoolStats};

pub use crate::dispatch_v2::AgentDispatchRequest;
use crate::dispatch_v2::ProviderRuntime;
use crate::dispatch_v2::{AgentDispatcherV2, CliProviderConfig, ProviderDispatchResolver};
use crate::task_parser::TaskDef;

// ─── Per-call value objects ────────────────────────────────────────────

/// Inputs the runner already has available for a single dispatch call.
///
/// `DispatchContext` is a *value object*: read-only, cheap to construct,
/// and carries the per-task knobs the dispatcher needs without requiring
/// the runner to thread through opaque references.
#[derive(Debug, Clone)]
pub struct DispatchContext {
    /// Plan id this task belongs to.
    pub plan_id: String,
    /// Logical role name (`"implementer"`, `"reviewer"`, ...).
    pub role: String,
    /// Working directory for the agent.
    pub workdir: std::path::PathBuf,
    /// Optional explicit model override from CLI / config (`task.model_hint`).
    pub model_hint: Option<String>,
    /// Optional `force_backend` override (manual operator decision).
    pub force_backend: Option<String>,
    /// Remaining USD budget for the plan; the router uses this to bias
    /// toward cheaper models when the budget is nearly exhausted.
    pub budget_remaining_usd: f64,
    /// Attempt number for this task (0 = first try, > 0 = retry).
    pub attempt: u32,
    /// Optional structured feedback from a previous gate failure.
    pub gate_feedback: Option<GateFeedback>,
    /// Routing context for the CascadeRouter. Built at the dispatch site
    /// from task + runner state, threaded through to `RoutingInputs`.
    pub routing_context: Option<RoutingContext>,
    /// Output files from each completed dependency task.
    /// Each entry is `(task_id, files)`. Injected into the system prompt
    /// so the agent knows what its predecessors already produced.
    pub dependency_outputs: Vec<(String, Vec<String>)>,
}

// ─── Dispatcher facade ─────────────────────────────────────────────────

/// Single-entry agent dispatch facade.
///
/// Owns:
/// - a model router (cascade-aware),
/// - a prompt assembler (queries playbooks + neuro store),
/// - a warm pool for fast role transitions.
///
/// The dispatcher does *not* own the agent runtime itself —
/// [`Dispatcher::dispatch`] receives an [`AgentResultBridge`] and calls
/// through to it. Production wiring constructs the bridge from
/// [`AgentDispatcherV2`]; tests use a stub.
#[derive(Debug)]
pub struct Dispatcher {
    router: ModelRouter,
    prompt_assembler: PromptAssembler,
    warm_pool: WarmPool,
}

impl Dispatcher {
    /// Construct a new dispatcher.
    pub fn new(
        cascade: Option<Arc<CascadeRouter>>,
        prompt_assembler: PromptAssembler,
        warm_pool: WarmPool,
    ) -> Self {
        Self {
            router: ModelRouter::new(cascade),
            prompt_assembler,
            warm_pool,
        }
    }

    /// Read-only access to the warm pool — exposed for diagnostics and
    /// admin endpoints (`/agents/warm-pool`) without leaking mutability.
    #[must_use]
    pub fn warm_pool(&self) -> &WarmPool {
        &self.warm_pool
    }

    /// Clone the cascade router `Arc` for use when reconstructing the
    /// dispatcher with an updated prompt assembler.
    #[must_use]
    pub fn cascade_router_arc(&self) -> Option<Arc<CascadeRouter>> {
        self.router.cascade_arc()
    }

    /// Resolve the model + prompt for `task` without dispatching.
    ///
    /// Used by tests, dry-run flows, and the prompt cache to materialize
    /// the dispatch decision without paying for an agent run.
    pub fn plan(
        &self,
        task: &TaskDef,
        ctx: &DispatchContext,
    ) -> Result<RunnerDispatchPlan, DispatchError> {
        let inputs = RoutingInputs::from_task(task, ctx);
        let choice = self.router.route(&inputs)?;
        let prompt_ctx = PromptContext::from_task(task, ctx);
        let assembled = self.prompt_assembler.assemble(task, &prompt_ctx)?;
        Ok(RunnerDispatchPlan {
            model: choice.model.clone(),
            forced: choice.forced(),
            prompt: assembled,
        })
    }

    /// Dispatch `task` through the supplied bridge and normalize the
    /// outcome.
    ///
    /// The bridge isolates provider state so this method stays pure
    /// orchestration: route + compose + act + normalize.
    pub async fn dispatch<B: AgentResultBridge>(
        &self,
        task: &TaskDef,
        ctx: &DispatchContext,
        bridge: &B,
    ) -> Result<AgentOutcome, DispatchError> {
        let plan = self.plan(task, ctx)?;
        let result = bridge
            .run_agent(&plan, ctx)
            .await
            .map_err(|err| DispatchError::SpawnFailed(err.to_string()))?;
        Ok(result)
    }

    /// Launch a streaming CLI agent through the dispatch facade.
    ///
    /// Runner v2 still owns the returned process handle so it can enforce
    /// cancellation and orphan cleanup, but provider invocation construction and
    /// runtime event normalization are below dispatch/roko-agent.
    pub async fn spawn_streaming_cli_agent(
        &self,
        config: &crate::runner::agent_stream::AgentSpawnConfig,
        event_tx: mpsc::Sender<AgentRuntimeEvent>,
    ) -> anyhow::Result<crate::runner::agent_stream::AgentHandle> {
        crate::runner::agent_stream::spawn_agent(config, event_tx).await
    }
}

/// Materialized runner dispatch plan — what `Dispatcher::plan` resolves to.
#[derive(Debug, Clone)]
pub struct RunnerDispatchPlan {
    /// Selected model + backend.
    pub model: ModelSpec,
    /// `true` if the model came from a `force_backend` override rather
    /// than the router. Recorded so feedback writers can downstream-tag
    /// observations as overrides.
    pub forced: bool,
    /// Assembled prompt, allowlist, diagnostics.
    pub prompt: AssembledPrompt,
}

// ─── Provider bridge trait (async-trait friendly) ──────────────────────

/// Minimal async trait implemented by provider runtimes.
///
/// Production: a thin shim around `AgentDispatcherV2::run_agent_result_bridge`.
/// Tests: a stub returning canned outcomes.
#[async_trait::async_trait]
pub trait AgentResultBridge: Send + Sync {
    /// Run the agent for `plan` and return a normalized outcome.
    async fn run_agent(
        &self,
        plan: &RunnerDispatchPlan,
        ctx: &DispatchContext,
    ) -> Result<AgentOutcome, anyhow::Error>;
}

// ─── Runtime Launch Facade ─────────────────────────────────────────────

/// Runtime selected for a resolved model.
#[derive(Debug, Clone)]
pub enum ResolvedAgentRuntime {
    /// Streaming CLI subprocess with provider-specific invocation metadata.
    Cli {
        /// Concrete model slug sent to the CLI.
        model: String,
        /// Resolved CLI provider. `None` preserves legacy runner defaults when
        /// no `roko.toml` provider graph has been loaded.
        cli_provider: Option<CliProviderConfig>,
    },
    /// API/provider-backed agent bridged through `AgentDispatcherV2`.
    Bridge {
        /// Concrete model slug sent to the provider.
        model: String,
        /// Provider registry id for diagnostics.
        provider_id: String,
        /// Effective config used to create the provider-backed agent.
        roko_config: Arc<RokoConfig>,
    },
}

/// Resolve the runtime that should execute `requested_model`.
pub fn resolve_agent_runtime(
    roko_config: Option<&Arc<RokoConfig>>,
    requested_model: &str,
) -> Result<ResolvedAgentRuntime, String> {
    let Some(roko_config) = roko_config else {
        return Ok(ResolvedAgentRuntime::Cli {
            model: requested_model.to_string(),
            cli_provider: None,
        });
    };

    let resolver = ProviderDispatchResolver::new(Arc::clone(roko_config));
    let spec = resolver.resolve(requested_model);
    match spec.runtime {
        ProviderRuntime::Cli(provider) => Ok(ResolvedAgentRuntime::Cli {
            model: spec.model_slug,
            cli_provider: Some(provider),
        }),
        ProviderRuntime::AgentResultBridge { .. } => Ok(ResolvedAgentRuntime::Bridge {
            model: spec.model_slug,
            provider_id: spec.provider_id,
            roko_config: Arc::clone(roko_config),
        }),
        ProviderRuntime::Unsupported(unsupported) => Err(format!(
            "model `{requested_model}` resolved to unsupported provider `{}`: {}",
            spec.provider_id, unsupported.detail
        )),
    }
}

/// Spawn an API/provider-backed agent and forward streaming runtime events.
///
/// Events are forwarded in real time as `StreamChunk`s arrive from the agent,
/// rather than batched after the full run completes.
pub fn spawn_agent_result_bridge(
    roko_config: Arc<RokoConfig>,
    request: AgentDispatchRequest,
    event_tx: mpsc::Sender<AgentRuntimeEvent>,
) {
    tokio::spawn(async move {
        let dispatcher = AgentDispatcherV2::new(roko_config);
        if let Err(err) = dispatcher
            .run_agent_streaming(request, event_tx.clone())
            .await
        {
            let _ = event_tx
                .send(AgentRuntimeEvent::Error {
                    message: err.to_string(),
                })
                .await;
            let _ = event_tx
                .send(AgentRuntimeEvent::Exited { exit_code: Some(1) })
                .await;
        }
    });
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_task(id: &str) -> TaskDef {
        TaskDef {
            id: id.into(),
            title: id.into(),
            description: None,
            role: Some("implementer".into()),
            status: "ready".into(),
            tier: "focused".into(),
            frequency: None,
            model_hint: Some("claude-sonnet-4-6".into()),
            replan_strategy: None,
            max_loc: None,
            files: vec![],
            allowed_tools: None,
            denied_tools: None,
            mcp_servers: None,
            depends_on: vec![],
            depends_on_plan: vec![],
            split_into: None,
            context: None,
            verify: vec![],
            timeout_secs: 60,
            max_retries: 1,
            acceptance: vec![],
            acceptance_contract: None,
            domain: None,
            sequence: 0,
        }
    }

    fn make_ctx() -> DispatchContext {
        DispatchContext {
            plan_id: "p1".into(),
            role: "implementer".into(),
            workdir: PathBuf::from("/tmp"),
            model_hint: None,
            force_backend: None,
            budget_remaining_usd: 5.0,
            attempt: 0,
            gate_feedback: None,
            routing_context: None,
            dependency_outputs: Vec::new(),
        }
    }

    struct StubBridge;

    #[async_trait::async_trait]
    impl AgentResultBridge for StubBridge {
        async fn run_agent(
            &self,
            plan: &RunnerDispatchPlan,
            ctx: &DispatchContext,
        ) -> Result<AgentOutcome, anyhow::Error> {
            Ok(AgentOutcome {
                task_id: "t".into(),
                plan_id: ctx.plan_id.clone(),
                model: plan.model.slug.clone(),
                provider: format!("{:?}", plan.model.backend).to_lowercase(),
                output: "ok".into(),
                tokens_in: 10,
                tokens_out: 20,
                cost_usd: 0.001,
                duration_ms: 42,
                exit_code: Some(0),
                is_error: false,
            })
        }
    }

    #[tokio::test]
    async fn dispatcher_plan_and_dispatch_round_trip() {
        let dispatcher = Dispatcher::new(None, PromptAssembler::minimal(), WarmPool::new(0));
        let task = make_task("t-1");
        let ctx = make_ctx();
        let plan = dispatcher.plan(&task, &ctx).expect("plan");
        assert_eq!(plan.model.slug, "claude-sonnet-4-6");
        assert!(!plan.prompt.system_prompt.is_empty());

        let outcome = dispatcher
            .dispatch(&task, &ctx, &StubBridge)
            .await
            .expect("dispatch");
        assert_eq!(outcome.model, "claude-sonnet-4-6");
        assert_eq!(outcome.tokens_in, 10);
    }

    #[tokio::test]
    async fn force_backend_overrides_router() {
        let dispatcher = Dispatcher::new(None, PromptAssembler::minimal(), WarmPool::new(0));
        let task = make_task("t-2");
        let ctx = DispatchContext {
            force_backend: Some("gpt-5".into()),
            ..make_ctx()
        };
        let plan = dispatcher.plan(&task, &ctx).expect("plan");
        assert_eq!(plan.model.slug, "gpt-5");
        assert!(plan.forced, "force_backend must mark plan as forced");
    }
}
