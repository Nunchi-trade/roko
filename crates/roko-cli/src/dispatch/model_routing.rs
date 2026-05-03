//! Model routing — turn a task + dispatch context into a [`ModelSpec`].
//!
//! ## Decision pipeline
//!
//! 1. **Manual override**. `force_backend` from CLI / config wins
//!    unconditionally. This preserves the operator's ability to pin a
//!    backend during incidents — and the choice is recorded so the
//!    feedback loop can learn from operator preferences.
//! 2. **Task hint**. `task_def.model_hint` (if any). Hints are author
//!    intent — not learned policy — and always beat the router.
//! 3. **CascadeRouter**. Only consulted when neither override nor hint
//!    applies. Returns a [`CascadeModel`] whose `primary` slug is used.
//! 4. **Safe default**. With no router and no hint, fall back to the
//!    `RunConfig.model` default. The router will eventually populate
//!    itself from observations.
//!
//! Every choice is wrapped in [`ModelChoice`] which records *why* the
//! model was picked. Feedback writers ([`runtime_feedback`]) use this to
//! tag observations: a forced override is recorded with
//! `forced = true` so the router doesn't conflate operator intent with
//! its own bandit signal.
//!
//! [`runtime_feedback`]: crate::runtime_feedback

use std::sync::Arc;

use roko_core::agent::ModelSpec;
use roko_core::task::TaskComplexityBand;
use roko_learn::cascade_router::CascadeRouter;
use roko_learn::model_router::RoutingContext;

use super::DispatchContext;
use super::outcome::DispatchError;
use crate::task_parser::TaskDef;

// ─── Inputs ────────────────────────────────────────────────────────────

/// All inputs the router needs from the runner.
///
/// Constructed from a `TaskDef` + `DispatchContext`. Pre-extracting these
/// fields keeps the router pure and makes it trivial to test without
/// holding live runner state.
#[derive(Debug, Clone)]
pub struct RoutingInputs {
    /// Task domain (`"rust"`, `"docs"`, `"frontend"`, ...). Used by the
    /// router to bias toward domain-strong models.
    pub task_domain: Option<String>,
    /// Task tier (`"focused"`, `"deep"`, ...). Higher tiers can spend
    /// more per call.
    pub task_tier: String,
    /// Author-provided model hint (`task.model_hint`).
    pub task_model_hint: Option<String>,
    /// Operator override (`force_backend` from config).
    pub force_backend: Option<String>,
    /// Remaining USD budget for the plan.
    pub budget_remaining_usd: f64,
    /// Attempt number (0 = first try).
    pub attempt: u32,
    /// Role label.
    pub role: String,
    /// Full routing context for the CascadeRouter. When `Some`, the router
    /// calls `CascadeRouter::route()` instead of falling back to the default.
    pub routing_context: Option<RoutingContext>,
}

impl RoutingInputs {
    /// Extract router inputs from a task + per-call context.
    #[must_use]
    pub fn from_task(task: &TaskDef, ctx: &DispatchContext) -> Self {
        Self {
            task_domain: task.domain.as_ref().map(|d| d.label().to_string()),
            task_tier: task.tier.clone(),
            task_model_hint: task.model_hint.clone().or_else(|| ctx.model_hint.clone()),
            force_backend: ctx.force_backend.clone(),
            budget_remaining_usd: ctx.budget_remaining_usd,
            attempt: ctx.attempt,
            role: ctx.role.clone(),
            routing_context: ctx.routing_context.clone(),
        }
    }
}

// ─── Outputs ───────────────────────────────────────────────────────────

/// Why the router picked this model — preserved for feedback writers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelChoiceSource {
    /// `force_backend` override from CLI / config.
    Override,
    /// Author intent (`task.model_hint`).
    TaskHint,
    /// Returned by [`CascadeRouter`].
    Router,
    /// Fallback when no other signal was available.
    Default,
}

/// A picked model and the reason it was picked.
#[derive(Debug, Clone)]
pub struct ModelChoice {
    /// The resolved model spec (slug + backend + effort).
    pub model: ModelSpec,
    /// Why this model was picked.
    pub source: ModelChoiceSource,
}

impl ModelChoice {
    /// `true` if the choice came from an explicit operator override.
    #[must_use]
    pub fn forced(&self) -> bool {
        matches!(self.source, ModelChoiceSource::Override)
    }
}

// Compatibility shorthand expected by [`super::RunnerDispatchPlan`].
impl ModelChoice {
    /// Reuse the public `forced` flag exposed via the dispatcher facade.
    #[must_use]
    pub fn is_forced(&self) -> bool {
        self.forced()
    }
}

// ─── Router ────────────────────────────────────────────────────────────

/// Thin facade over [`CascadeRouter`] that applies the override / hint /
/// router / default precedence rules.
///
/// Holds an `Option<Arc<CascadeRouter>>` so callers without a configured
/// router (CI, smoke tests) still work. When the router is absent the
/// pipeline degrades to override → hint → default — never panics.
#[derive(Clone)]
pub struct ModelRouter {
    cascade: Option<Arc<CascadeRouter>>,
    /// Default model slug used when override / hint / router all decline.
    /// Configurable so tests can inject a deterministic baseline.
    default_slug: String,
}

impl std::fmt::Debug for ModelRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelRouter")
            .field("cascade", &self.cascade.as_ref().map(|_| ".."))
            .field("default_slug", &self.default_slug)
            .finish()
    }
}

impl ModelRouter {
    /// Construct a router. Callers without a `CascadeRouter` pass `None`.
    pub fn new(cascade: Option<Arc<CascadeRouter>>) -> Self {
        Self {
            cascade,
            default_slug: "claude-sonnet-4-6".to_string(),
        }
    }

    /// Clone the inner cascade router `Arc` (for factory cache swap).
    #[must_use]
    pub fn cascade_arc(&self) -> Option<Arc<CascadeRouter>> {
        self.cascade.clone()
    }

    /// Override the default-fallback slug.
    pub fn with_default_slug(mut self, slug: impl Into<String>) -> Self {
        self.default_slug = slug.into();
        self
    }

    /// Apply the precedence pipeline.
    pub fn route(&self, inputs: &RoutingInputs) -> Result<ModelChoice, DispatchError> {
        if let Some(slug) = inputs.force_backend.as_ref() {
            return Ok(ModelChoice {
                model: ModelSpec::from_slug(slug),
                source: ModelChoiceSource::Override,
            });
        }
        if let Some(slug) = inputs.task_model_hint.as_ref() {
            return Ok(ModelChoice {
                model: ModelSpec::from_slug(slug),
                source: ModelChoiceSource::TaskHint,
            });
        }
        if let Some(router) = self.cascade.as_ref() {
            if let Some(ctx) = &inputs.routing_context {
                let cascade_model = router.route(ctx);
                return Ok(ModelChoice {
                    model: cascade_model.primary,
                    source: ModelChoiceSource::Router,
                });
            }
            // No RoutingContext → degrade to default (CI, smoke tests).
            return Ok(ModelChoice {
                model: ModelSpec::from_slug(&self.default_slug),
                source: ModelChoiceSource::Default,
            });
        }
        Ok(ModelChoice {
            model: ModelSpec::from_slug(&self.default_slug),
            source: ModelChoiceSource::Default,
        })
    }
}

/// Map a task tier string to a [`TaskComplexityBand`].
pub(crate) fn tier_to_complexity(tier: &str) -> TaskComplexityBand {
    match tier {
        "focused" | "quick" | "trivial" => TaskComplexityBand::Fast,
        "deep" | "architectural" | "complex" => TaskComplexityBand::Complex,
        _ => TaskComplexityBand::Standard,
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn task() -> TaskDef {
        TaskDef {
            id: "t".into(),
            title: "t".into(),
            description: None,
            role: Some("implementer".into()),
            status: "ready".into(),
            tier: "focused".into(),
            frequency: None,
            model_hint: None,
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
            domain: Some(roko_core::task::TaskDomain::Code),
        }
    }

    fn ctx() -> DispatchContext {
        DispatchContext {
            plan_id: "p".into(),
            role: "implementer".into(),
            workdir: PathBuf::from("/tmp"),
            model_hint: None,
            force_backend: None,
            budget_remaining_usd: 5.0,
            attempt: 0,
            gate_feedback: None,
            routing_context: None,
        }
    }

    #[test]
    fn override_wins_over_everything() {
        let mut t = task();
        t.model_hint = Some("claude-sonnet-4-6".into());
        let mut c = ctx();
        c.force_backend = Some("gpt-5".into());
        let inputs = RoutingInputs::from_task(&t, &c);
        let router = ModelRouter::new(None);
        let choice = router.route(&inputs).unwrap();
        assert_eq!(choice.model.slug, "gpt-5");
        assert_eq!(choice.source, ModelChoiceSource::Override);
        assert!(choice.forced());
    }

    #[test]
    fn task_hint_beats_router_when_no_override() {
        let mut t = task();
        t.model_hint = Some("claude-haiku-4-5".into());
        let inputs = RoutingInputs::from_task(&t, &ctx());
        let router = ModelRouter::new(None);
        let choice = router.route(&inputs).unwrap();
        assert_eq!(choice.model.slug, "claude-haiku-4-5");
        assert_eq!(choice.source, ModelChoiceSource::TaskHint);
        assert!(!choice.forced());
    }

    #[test]
    fn default_fallback_when_router_absent() {
        let inputs = RoutingInputs::from_task(&task(), &ctx());
        let router = ModelRouter::new(None).with_default_slug("custom-default");
        let choice = router.route(&inputs).unwrap();
        assert_eq!(choice.model.slug, "custom-default");
        assert_eq!(choice.source, ModelChoiceSource::Default);
    }

    fn routing_context() -> RoutingContext {
        use roko_core::task::TaskCategory;
        use roko_core::{AgentRole, BehavioralState, DaimonPolicy};

        RoutingContext {
            task_category: TaskCategory::Implementation,
            complexity: TaskComplexityBand::Standard,
            iteration: 0,
            role: AgentRole::Implementer,
            crate_familiarity: 0.5,
            has_prior_failure: false,
            conductor_load: 0.0,
            active_agents: 0,
            ready_queue_depth: 0,
            max_queue_wait_hours: 0.0,
            daimon_policy: DaimonPolicy::new(0.5, BehavioralState::Engaged),
            thinking_level: None,
            temperament: None,
            previous_model: None,
            plan_context_tokens: None,
            tier_thresholds: None,
        }
    }

    #[test]
    fn cascade_router_called_when_context_present() {
        let cascade = Arc::new(CascadeRouter::new(vec![
            "claude-sonnet-4-6".into(),
            "gpt-5".into(),
        ]));
        let router = ModelRouter::new(Some(cascade));
        let mut inputs = RoutingInputs::from_task(&task(), &ctx());
        inputs.routing_context = Some(routing_context());
        let choice = router.route(&inputs).unwrap();
        assert_eq!(choice.source, ModelChoiceSource::Router);
        // CascadeRouter picks from the configured slugs.
        assert!(
            choice.model.slug == "claude-sonnet-4-6" || choice.model.slug == "gpt-5",
            "expected one of the configured slugs, got {:?}",
            choice.model.slug,
        );
    }

    #[test]
    fn no_context_degrades_to_default() {
        let cascade = Arc::new(CascadeRouter::new(vec![
            "claude-sonnet-4-6".into(),
            "gpt-5".into(),
        ]));
        let router = ModelRouter::new(Some(cascade)).with_default_slug("fallback-model");
        let inputs = RoutingInputs::from_task(&task(), &ctx());
        // routing_context is None via from_task()
        let choice = router.route(&inputs).unwrap();
        assert_eq!(choice.model.slug, "fallback-model");
        assert_eq!(choice.source, ModelChoiceSource::Default);
    }

    #[test]
    fn tier_to_complexity_mapping() {
        assert_eq!(tier_to_complexity("focused"), TaskComplexityBand::Fast);
        assert_eq!(tier_to_complexity("quick"), TaskComplexityBand::Fast);
        assert_eq!(tier_to_complexity("trivial"), TaskComplexityBand::Fast);
        assert_eq!(tier_to_complexity("deep"), TaskComplexityBand::Complex);
        assert_eq!(
            tier_to_complexity("architectural"),
            TaskComplexityBand::Complex
        );
        assert_eq!(tier_to_complexity("complex"), TaskComplexityBand::Complex);
        assert_eq!(tier_to_complexity("standard"), TaskComplexityBand::Standard);
        assert_eq!(tier_to_complexity("anything"), TaskComplexityBand::Standard);
    }
}
