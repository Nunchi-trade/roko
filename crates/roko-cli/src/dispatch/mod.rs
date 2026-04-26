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

pub mod model_routing;
pub mod outcome;
pub mod prompt_builder;
pub mod warm_pool;

use std::sync::Arc;

use roko_core::agent::ModelSpec;
use roko_learn::cascade_router::CascadeRouter;

pub use model_routing::{ModelChoice, ModelChoiceSource, ModelRouter, RoutingInputs};
pub use outcome::{AgentOutcome, DispatchError};
pub use prompt_builder::{
    AssembledPrompt, GateFeedback, PromptAssembler, PromptContext, PromptDiagnostics,
};
pub use warm_pool::{WarmPool, WarmPoolStats};

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

    /// Resolve the model + prompt for `task` without dispatching.
    ///
    /// Used by tests, dry-run flows, and the prompt cache to materialize
    /// the dispatch decision without paying for an agent run.
    pub fn plan(
        &self,
        task: &TaskDef,
        ctx: &DispatchContext,
    ) -> Result<DispatchPlan, DispatchError> {
        let inputs = RoutingInputs::from_task(task, ctx);
        let choice = self.router.route(&inputs)?;
        let prompt_ctx = PromptContext::from_task(task, ctx);
        let assembled = self.prompt_assembler.assemble(task, &prompt_ctx)?;
        Ok(DispatchPlan {
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
}

/// Materialized dispatch plan — what `Dispatcher::plan` resolves to.
#[derive(Debug, Clone)]
pub struct DispatchPlan {
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
        plan: &DispatchPlan,
        ctx: &DispatchContext,
    ) -> Result<AgentOutcome, anyhow::Error>;
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
        }
    }

    struct StubBridge;

    #[async_trait::async_trait]
    impl AgentResultBridge for StubBridge {
        async fn run_agent(
            &self,
            plan: &DispatchPlan,
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
        let dispatcher = Dispatcher::new(
            None,
            PromptAssembler::minimal(),
            WarmPool::new(0),
        );
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
        let dispatcher = Dispatcher::new(
            None,
            PromptAssembler::minimal(),
            WarmPool::new(0),
        );
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
