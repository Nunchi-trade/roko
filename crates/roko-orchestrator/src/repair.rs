//! Plan repair engine — structured recovery from task failures.
//!
//! When a task fails or environment changes invalidate part of a plan, the
//! [`RepairEngine`] attempts **local repair** before falling back to full
//! re-planning. This preserves plan stability (minimizing changed tasks)
//! and reduces cost by avoiding unnecessary regeneration.
//!
//! # Three repair levels
//!
//! 1. **Task retry** — retry the failed task with a modified prompt
//!    (additional context from the failure, model escalation).
//! 2. **Subgraph replacement** — re-plan a subset of tasks (the failed
//!    task and its dependents) while keeping the rest of the plan intact.
//! 3. **Full replan** — regenerate the entire plan from scratch.
//!
//! # Meta-reasoning
//!
//! The engine selects the cheapest feasible repair level based on:
//! - Failure history (how many retries have been attempted)
//! - Estimated cost of each repair level
//! - Expected success probability at each level
//! - Plan stability metric (Fox et al. 2006): prefer repairs that
//!   minimize the number of changed tasks
//!
//! # Integration
//!
//! The repair engine is invoked during the executor's `AutoFixing` phase
//! transition. It produces a [`RepairAction`] that the runtime dispatches.

use serde::{Deserialize, Serialize};

use crate::replan::ReplanStrategy;

// ─── Failure context ────────────────────────────────────────────────────

/// Context describing a task failure, used by the repair engine to select
/// the appropriate repair level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureContext {
    /// The plan that contains the failed task.
    pub plan_id: String,
    /// The specific task that failed.
    pub task_id: String,
    /// How many times this task has been retried (0 = first failure).
    pub retry_count: u32,
    /// Whether the failure was a compilation error.
    pub is_compile_error: bool,
    /// Whether the failure was a test failure.
    pub is_test_failure: bool,
    /// Whether the failure was a timeout.
    pub is_timeout: bool,
    /// The gate that rejected the task, if any.
    pub failed_gate: Option<String>,
    /// Error summary from the last attempt.
    pub error_summary: String,
    /// Number of tasks in the plan that depend on this task.
    pub dependent_count: usize,
    /// Total number of tasks in the plan.
    pub total_tasks: usize,
    /// The current model used for the task, if known.
    pub current_model: Option<String>,
}

impl FailureContext {
    /// Create a minimal failure context for a task.
    #[must_use]
    pub fn new(plan_id: impl Into<String>, task_id: impl Into<String>) -> Self {
        Self {
            plan_id: plan_id.into(),
            task_id: task_id.into(),
            retry_count: 0,
            is_compile_error: false,
            is_test_failure: false,
            is_timeout: false,
            failed_gate: None,
            error_summary: String::new(),
            dependent_count: 0,
            total_tasks: 1,
            current_model: None,
        }
    }
}

// ─── Repair levels ──────────────────────────────────────────────────────

/// The three repair abstraction levels, ordered from cheapest to most
/// expensive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairLevel {
    /// Retry the same task with modified prompt / escalated model.
    TaskRetry,
    /// Re-plan the failed task and its transitive dependents.
    SubgraphReplacement,
    /// Regenerate the entire plan from scratch.
    FullReplan,
}

impl RepairLevel {
    /// Human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::TaskRetry => "task_retry",
            Self::SubgraphReplacement => "subgraph_replacement",
            Self::FullReplan => "full_replan",
        }
    }

    /// Estimated relative cost (0.0 = free, 1.0 = full replan cost).
    #[must_use]
    pub const fn relative_cost(self) -> f64 {
        match self {
            Self::TaskRetry => 0.1,
            Self::SubgraphReplacement => 0.4,
            Self::FullReplan => 1.0,
        }
    }
}

impl std::fmt::Display for RepairLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

// ─── RepairAction ───────────────────────────────────────────────────────

/// The concrete repair action selected by the engine.
///
/// The runtime dispatches this action to the appropriate subsystem
/// (agent pool for retries, plan generator for re-planning).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RepairAction {
    /// Retry the task with additional failure context injected into the
    /// prompt and optionally a stronger model.
    RetryTask {
        /// Plan containing the failed task.
        plan_id: String,
        /// The failed task to retry.
        task_id: String,
        /// Additional context to inject into the retry prompt.
        additional_context: String,
        /// If set, escalate to this model for the retry.
        escalated_model: Option<String>,
    },

    /// Re-plan a subgraph: the failed task and its transitive dependents.
    ReplaceSubgraph {
        /// Plan containing the failed task.
        plan_id: String,
        /// The root task whose subgraph should be replaced.
        root_task_id: String,
        /// Task IDs in the subgraph (root + transitive dependents).
        affected_task_ids: Vec<String>,
        /// Hint for the planner: error context to avoid repeating.
        failure_hint: String,
    },

    /// Regenerate the entire plan from scratch.
    FullReplan {
        /// Plan to regenerate.
        plan_id: String,
        /// Why the full replan was chosen.
        reason: String,
    },

    /// Skip the failed task and continue with the rest of the plan.
    ///
    /// Chosen when the task is not on the critical path and has no
    /// dependents, making it safe to skip.
    SkipTask {
        /// Plan containing the task.
        plan_id: String,
        /// The task to skip.
        task_id: String,
        /// Why skipping was chosen.
        reason: String,
    },
}

impl RepairAction {
    /// The repair level this action corresponds to.
    #[must_use]
    pub const fn level(&self) -> RepairLevel {
        match self {
            Self::RetryTask { .. } => RepairLevel::TaskRetry,
            Self::ReplaceSubgraph { .. } => RepairLevel::SubgraphReplacement,
            Self::FullReplan { .. } => RepairLevel::FullReplan,
            Self::SkipTask { .. } => RepairLevel::TaskRetry, // skip is cheapest
        }
    }

    /// The plan this action targets.
    #[must_use]
    pub fn plan_id(&self) -> &str {
        match self {
            Self::RetryTask { plan_id, .. }
            | Self::ReplaceSubgraph { plan_id, .. }
            | Self::FullReplan { plan_id, .. }
            | Self::SkipTask { plan_id, .. } => plan_id,
        }
    }

    /// Convert to the corresponding [`ReplanStrategy`] for compatibility
    /// with the existing replan module.
    #[must_use]
    pub fn as_replan_strategy(&self) -> ReplanStrategy {
        match self {
            Self::RetryTask {
                escalated_model: Some(_),
                ..
            } => ReplanStrategy::RetryWithEscalation,
            Self::RetryTask { .. } => ReplanStrategy::RetrySame,
            Self::ReplaceSubgraph { .. } => ReplanStrategy::Decompose,
            Self::FullReplan { .. } => ReplanStrategy::RegeneratePlan,
            Self::SkipTask { .. } => ReplanStrategy::Skip,
        }
    }
}

impl std::fmt::Display for RepairAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RetryTask {
                plan_id,
                task_id,
                escalated_model,
                ..
            } => {
                if let Some(model) = escalated_model {
                    write!(f, "retry({plan_id}:{task_id}, model={model})")
                } else {
                    write!(f, "retry({plan_id}:{task_id})")
                }
            }
            Self::ReplaceSubgraph {
                plan_id,
                root_task_id,
                affected_task_ids,
                ..
            } => {
                write!(
                    f,
                    "replace_subgraph({plan_id}:{root_task_id}, {} tasks)",
                    affected_task_ids.len()
                )
            }
            Self::FullReplan { plan_id, reason } => {
                write!(f, "full_replan({plan_id}: {reason})")
            }
            Self::SkipTask {
                plan_id, task_id, ..
            } => {
                write!(f, "skip({plan_id}:{task_id})")
            }
        }
    }
}

// ─── RepairConfig ───────────────────────────────────────────────────────

/// Configuration knobs for the repair engine's meta-reasoning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairConfig {
    /// Maximum retries at the task level before escalating to subgraph
    /// replacement.
    pub max_task_retries: u32,
    /// Maximum subgraph replacement attempts before escalating to full
    /// replan.
    pub max_subgraph_attempts: u32,
    /// Fraction of plan tasks that must be in the affected subgraph
    /// before we skip subgraph replacement and go straight to full
    /// replan (0.0..=1.0).
    pub subgraph_fraction_threshold: f64,
    /// Model to escalate to on the second task retry (if any).
    pub escalation_model: Option<String>,
    /// Whether to allow skipping non-critical tasks with no dependents.
    pub allow_skip: bool,
}

impl Default for RepairConfig {
    fn default() -> Self {
        Self {
            max_task_retries: 2,
            max_subgraph_attempts: 1,
            subgraph_fraction_threshold: 0.5,
            escalation_model: None,
            allow_skip: true,
        }
    }
}

// ─── StabilityMetric ────────────────────────────────────────────────────

/// Plan stability metric (Fox et al. 2006).
///
/// Measures the fraction of the plan that remains unchanged after a
/// repair. Higher is better: a stability of 1.0 means nothing changed
/// (pure retry), 0.0 means full replan.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StabilityMetric {
    /// Number of tasks unchanged by the repair.
    pub unchanged_tasks: usize,
    /// Total number of tasks in the plan.
    pub total_tasks: usize,
}

impl StabilityMetric {
    /// Compute the stability ratio (0.0..=1.0).
    #[must_use]
    pub fn ratio(self) -> f64 {
        if self.total_tasks == 0 {
            return 1.0;
        }
        self.unchanged_tasks as f64 / self.total_tasks as f64
    }
}

impl std::fmt::Display for StabilityMetric {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{} unchanged ({:.0}%)",
            self.unchanged_tasks,
            self.total_tasks,
            self.ratio() * 100.0,
        )
    }
}

// ─── RepairDecision ─────────────────────────────────────────────────────

/// The engine's decision, bundling the chosen action with reasoning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairDecision {
    /// The repair action to execute.
    pub action: RepairAction,
    /// The repair level selected.
    pub level: RepairLevel,
    /// Why this level was chosen.
    pub reasoning: String,
    /// Expected stability if this repair succeeds.
    pub expected_stability: StabilityMetric,
}

// ─── RepairEngine ───────────────────────────────────────────────────────

/// Structured plan repair engine.
///
/// Implements three-level repair with meta-reasoning: task retry,
/// subgraph replacement, and full replan. Selects the cheapest feasible
/// repair based on failure context and configuration.
pub struct RepairEngine {
    config: RepairConfig,
}

impl RepairEngine {
    /// Create a repair engine with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: RepairConfig::default(),
        }
    }

    /// Create a repair engine with the given configuration.
    #[must_use]
    pub fn with_config(config: RepairConfig) -> Self {
        Self { config }
    }

    /// Access the engine's configuration.
    #[must_use]
    pub const fn config(&self) -> &RepairConfig {
        &self.config
    }

    /// Select the cheapest feasible repair for the given failure.
    ///
    /// This is the main entry point. The engine examines the failure
    /// context and walks up the repair levels (task retry -> subgraph ->
    /// full replan) until it finds one that is feasible.
    #[must_use]
    pub fn repair(&self, ctx: &FailureContext) -> RepairDecision {
        // Level 0: Can we skip the task entirely?
        if let Some(decision) = self.try_skip(ctx) {
            return decision;
        }

        // Level 1: Task retry.
        if let Some(decision) = self.try_task_retry(ctx) {
            return decision;
        }

        // Level 2: Subgraph replacement.
        if let Some(decision) = self.try_subgraph_replacement(ctx) {
            return decision;
        }

        // Level 3: Full replan (always feasible as last resort).
        self.full_replan(ctx)
    }

    /// Attempt to skip the task if it has no dependents and is not
    /// critical.
    fn try_skip(&self, ctx: &FailureContext) -> Option<RepairDecision> {
        if !self.config.allow_skip {
            return None;
        }
        if ctx.dependent_count > 0 {
            return None;
        }
        // Don't skip compile errors (they indicate structural problems).
        if ctx.is_compile_error {
            return None;
        }
        // Only skip after at least one retry.
        if ctx.retry_count == 0 {
            return None;
        }

        Some(RepairDecision {
            action: RepairAction::SkipTask {
                plan_id: ctx.plan_id.clone(),
                task_id: ctx.task_id.clone(),
                reason: "no dependents and not a compile error".into(),
            },
            level: RepairLevel::TaskRetry,
            reasoning: format!(
                "task {} has no dependents and has been retried {} times; skipping",
                ctx.task_id, ctx.retry_count,
            ),
            expected_stability: StabilityMetric {
                unchanged_tasks: ctx.total_tasks.saturating_sub(1),
                total_tasks: ctx.total_tasks,
            },
        })
    }

    /// Attempt a task-level retry with additional context and optional
    /// model escalation.
    fn try_task_retry(&self, ctx: &FailureContext) -> Option<RepairDecision> {
        if ctx.retry_count >= self.config.max_task_retries {
            return None;
        }

        let escalated_model = if ctx.retry_count > 0 {
            self.config.escalation_model.clone()
        } else {
            None
        };

        let additional_context = build_retry_context(ctx);

        Some(RepairDecision {
            action: RepairAction::RetryTask {
                plan_id: ctx.plan_id.clone(),
                task_id: ctx.task_id.clone(),
                additional_context,
                escalated_model: escalated_model.clone(),
            },
            level: RepairLevel::TaskRetry,
            reasoning: format!(
                "retry {}/{} for task {} ({})",
                ctx.retry_count + 1,
                self.config.max_task_retries,
                ctx.task_id,
                if escalated_model.is_some() {
                    "with model escalation"
                } else {
                    "same model"
                },
            ),
            expected_stability: StabilityMetric {
                unchanged_tasks: ctx.total_tasks,
                total_tasks: ctx.total_tasks,
            },
        })
    }

    /// Attempt subgraph replacement if the affected subgraph is small
    /// enough relative to the total plan.
    fn try_subgraph_replacement(&self, ctx: &FailureContext) -> Option<RepairDecision> {
        // If the subgraph is too large a fraction of the plan, skip
        // straight to full replan.
        let affected_count = ctx.dependent_count + 1; // task + dependents
        let fraction = if ctx.total_tasks > 0 {
            affected_count as f64 / ctx.total_tasks as f64
        } else {
            1.0
        };
        if fraction > self.config.subgraph_fraction_threshold {
            return None;
        }

        // Build a list of affected task IDs. In practice, the runtime
        // would compute the actual transitive closure from the DAG; here
        // we provide a placeholder list that the runtime will expand.
        let affected_task_ids = vec![ctx.task_id.clone()];

        Some(RepairDecision {
            action: RepairAction::ReplaceSubgraph {
                plan_id: ctx.plan_id.clone(),
                root_task_id: ctx.task_id.clone(),
                affected_task_ids,
                failure_hint: ctx.error_summary.clone(),
            },
            level: RepairLevel::SubgraphReplacement,
            reasoning: format!(
                "subgraph replacement for task {} ({} of {} tasks affected, {:.0}%)",
                ctx.task_id,
                affected_count,
                ctx.total_tasks,
                fraction * 100.0,
            ),
            expected_stability: StabilityMetric {
                unchanged_tasks: ctx.total_tasks.saturating_sub(affected_count),
                total_tasks: ctx.total_tasks,
            },
        })
    }

    /// Full replan as the last resort.
    fn full_replan(&self, ctx: &FailureContext) -> RepairDecision {
        RepairDecision {
            action: RepairAction::FullReplan {
                plan_id: ctx.plan_id.clone(),
                reason: format!(
                    "task {} exhausted retries ({}) and subgraph too large",
                    ctx.task_id, ctx.retry_count,
                ),
            },
            level: RepairLevel::FullReplan,
            reasoning: format!(
                "full replan for plan {} after {} failed retries on task {}",
                ctx.plan_id, ctx.retry_count, ctx.task_id,
            ),
            expected_stability: StabilityMetric {
                unchanged_tasks: 0,
                total_tasks: ctx.total_tasks,
            },
        }
    }
}

impl Default for RepairEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────

/// Build additional context for a retry prompt based on the failure.
fn build_retry_context(ctx: &FailureContext) -> String {
    let mut parts: Vec<String> = Vec::new();

    parts.push(format!(
        "Previous attempt failed (attempt {}).",
        ctx.retry_count + 1,
    ));

    if !ctx.error_summary.is_empty() {
        parts.push(format!("Error: {}", ctx.error_summary));
    }

    if ctx.is_compile_error {
        parts
            .push("The code did not compile. Fix all compilation errors before proceeding.".into());
    }

    if ctx.is_test_failure {
        parts.push(
            "Tests failed. Ensure all existing tests pass and add tests for new functionality."
                .into(),
        );
    }

    if ctx.is_timeout {
        parts.push("The previous attempt timed out. Simplify the approach or break the work into smaller steps.".into());
    }

    if let Some(gate) = &ctx.failed_gate {
        parts.push(format!("The '{gate}' gate rejected the output."));
    }

    parts.join("\n")
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn simple_failure(retries: u32) -> FailureContext {
        FailureContext {
            plan_id: "plan-1".into(),
            task_id: "t3".into(),
            retry_count: retries,
            is_compile_error: false,
            is_test_failure: true,
            is_timeout: false,
            failed_gate: Some("test".into()),
            error_summary: "3 test failures in module X".into(),
            dependent_count: 2,
            total_tasks: 10,
            current_model: Some("claude-sonnet".into()),
        }
    }

    #[test]
    fn first_failure_retries_same_task() {
        let engine = RepairEngine::new();
        let decision = engine.repair(&simple_failure(0));

        assert_eq!(decision.level, RepairLevel::TaskRetry);
        assert!(matches!(decision.action, RepairAction::RetryTask { .. }));
        assert_eq!(decision.expected_stability.total_tasks, 10);
        assert_eq!(decision.expected_stability.unchanged_tasks, 10);
    }

    #[test]
    fn second_retry_escalates_model_if_configured() {
        let engine = RepairEngine::with_config(RepairConfig {
            escalation_model: Some("claude-opus".into()),
            ..RepairConfig::default()
        });
        let decision = engine.repair(&simple_failure(1));

        assert_eq!(decision.level, RepairLevel::TaskRetry);
        match &decision.action {
            RepairAction::RetryTask {
                escalated_model, ..
            } => {
                assert_eq!(escalated_model.as_deref(), Some("claude-opus"));
            }
            other => panic!("expected RetryTask, got {other:?}"),
        }
    }

    #[test]
    fn exhausted_retries_escalates_to_subgraph() {
        let engine = RepairEngine::new();
        let decision = engine.repair(&simple_failure(2));

        // retry_count=2 >= max_task_retries=2, so TaskRetry is skipped.
        // dependent_count=2, total_tasks=10 -> 3/10=30% < 50% threshold.
        assert_eq!(decision.level, RepairLevel::SubgraphReplacement);
        assert!(matches!(
            decision.action,
            RepairAction::ReplaceSubgraph { .. }
        ));
        // 10 - 3 (root + 2 dependents) = 7 unchanged.
        assert_eq!(decision.expected_stability.unchanged_tasks, 7);
    }

    #[test]
    fn large_subgraph_goes_straight_to_full_replan() {
        let engine = RepairEngine::new();
        let ctx = FailureContext {
            plan_id: "plan-2".into(),
            task_id: "t1".into(),
            retry_count: 3,
            is_compile_error: true,
            is_test_failure: false,
            is_timeout: false,
            failed_gate: Some("compile".into()),
            error_summary: "cannot find module".into(),
            dependent_count: 8, // 9/10 = 90% > 50%
            total_tasks: 10,
            current_model: None,
        };
        let decision = engine.repair(&ctx);

        assert_eq!(decision.level, RepairLevel::FullReplan);
        assert!(matches!(decision.action, RepairAction::FullReplan { .. }));
        assert_eq!(decision.expected_stability.unchanged_tasks, 0);
    }

    #[test]
    fn no_dependents_allows_skip_after_retry() {
        let engine = RepairEngine::new();
        let ctx = FailureContext {
            plan_id: "plan-3".into(),
            task_id: "t-doc".into(),
            retry_count: 1,
            is_compile_error: false,
            is_test_failure: false,
            is_timeout: true,
            failed_gate: None,
            error_summary: "timeout after 600s".into(),
            dependent_count: 0,
            total_tasks: 10,
            current_model: None,
        };
        let decision = engine.repair(&ctx);

        assert!(matches!(decision.action, RepairAction::SkipTask { .. }));
        assert_eq!(decision.expected_stability.unchanged_tasks, 9);
    }

    #[test]
    fn skip_not_allowed_on_first_attempt() {
        let engine = RepairEngine::new();
        let ctx = FailureContext {
            plan_id: "plan-4".into(),
            task_id: "t-leaf".into(),
            retry_count: 0,
            is_compile_error: false,
            is_test_failure: false,
            is_timeout: false,
            failed_gate: None,
            error_summary: "some error".into(),
            dependent_count: 0,
            total_tasks: 5,
            current_model: None,
        };
        let decision = engine.repair(&ctx);

        // First attempt: should retry, not skip.
        assert_eq!(decision.level, RepairLevel::TaskRetry);
        assert!(matches!(decision.action, RepairAction::RetryTask { .. }));
    }

    #[test]
    fn compile_error_never_skipped() {
        let engine = RepairEngine::new();
        let ctx = FailureContext {
            plan_id: "plan-5".into(),
            task_id: "t-core".into(),
            retry_count: 1,
            is_compile_error: true,
            is_test_failure: false,
            is_timeout: false,
            failed_gate: Some("compile".into()),
            error_summary: "unresolved import".into(),
            dependent_count: 0,
            total_tasks: 5,
            current_model: None,
        };
        let decision = engine.repair(&ctx);

        // Compile errors are never skipped even with 0 dependents.
        assert!(
            !matches!(decision.action, RepairAction::SkipTask { .. }),
            "compile errors should not be skipped"
        );
    }

    #[test]
    fn skip_disabled_by_config() {
        let engine = RepairEngine::with_config(RepairConfig {
            allow_skip: false,
            ..RepairConfig::default()
        });
        let ctx = FailureContext {
            plan_id: "plan-6".into(),
            task_id: "t-optional".into(),
            retry_count: 5,
            is_compile_error: false,
            is_test_failure: false,
            is_timeout: false,
            failed_gate: None,
            error_summary: "some issue".into(),
            dependent_count: 0,
            total_tasks: 10,
            current_model: None,
        };
        let decision = engine.repair(&ctx);

        // Skip disabled, retries exhausted, no dependents -> full replan.
        assert!(!matches!(decision.action, RepairAction::SkipTask { .. }));
    }

    #[test]
    fn repair_action_as_replan_strategy() {
        let retry = RepairAction::RetryTask {
            plan_id: "p".into(),
            task_id: "t".into(),
            additional_context: String::new(),
            escalated_model: None,
        };
        assert_eq!(retry.as_replan_strategy(), ReplanStrategy::RetrySame);

        let escalated = RepairAction::RetryTask {
            plan_id: "p".into(),
            task_id: "t".into(),
            additional_context: String::new(),
            escalated_model: Some("opus".into()),
        };
        assert_eq!(
            escalated.as_replan_strategy(),
            ReplanStrategy::RetryWithEscalation
        );

        let subgraph = RepairAction::ReplaceSubgraph {
            plan_id: "p".into(),
            root_task_id: "t".into(),
            affected_task_ids: vec![],
            failure_hint: String::new(),
        };
        assert_eq!(subgraph.as_replan_strategy(), ReplanStrategy::Decompose);

        let full = RepairAction::FullReplan {
            plan_id: "p".into(),
            reason: "test".into(),
        };
        assert_eq!(full.as_replan_strategy(), ReplanStrategy::RegeneratePlan);

        let skip = RepairAction::SkipTask {
            plan_id: "p".into(),
            task_id: "t".into(),
            reason: "test".into(),
        };
        assert_eq!(skip.as_replan_strategy(), ReplanStrategy::Skip);
    }

    #[test]
    fn repair_action_display() {
        let action = RepairAction::RetryTask {
            plan_id: "p1".into(),
            task_id: "t1".into(),
            additional_context: "ctx".into(),
            escalated_model: Some("opus".into()),
        };
        assert!(action.to_string().contains("retry"));
        assert!(action.to_string().contains("opus"));

        let action = RepairAction::ReplaceSubgraph {
            plan_id: "p1".into(),
            root_task_id: "t1".into(),
            affected_task_ids: vec!["t1".into(), "t2".into(), "t3".into()],
            failure_hint: String::new(),
        };
        assert!(action.to_string().contains("3 tasks"));

        let action = RepairAction::FullReplan {
            plan_id: "p1".into(),
            reason: "too many failures".into(),
        };
        assert!(action.to_string().contains("full_replan"));
    }

    #[test]
    fn stability_metric_display_and_ratio() {
        let metric = StabilityMetric {
            unchanged_tasks: 8,
            total_tasks: 10,
        };
        assert_eq!(metric.ratio(), 0.8);
        assert!(metric.to_string().contains("80%"));

        let empty = StabilityMetric {
            unchanged_tasks: 0,
            total_tasks: 0,
        };
        assert_eq!(empty.ratio(), 1.0);
    }

    #[test]
    fn repair_level_labels_and_costs() {
        assert_eq!(RepairLevel::TaskRetry.label(), "task_retry");
        assert_eq!(
            RepairLevel::SubgraphReplacement.label(),
            "subgraph_replacement"
        );
        assert_eq!(RepairLevel::FullReplan.label(), "full_replan");
        assert!(
            RepairLevel::TaskRetry.relative_cost()
                < RepairLevel::SubgraphReplacement.relative_cost()
        );
        assert!(
            RepairLevel::SubgraphReplacement.relative_cost()
                < RepairLevel::FullReplan.relative_cost()
        );
    }

    #[test]
    fn repair_config_default_values() {
        let config = RepairConfig::default();
        assert_eq!(config.max_task_retries, 2);
        assert_eq!(config.max_subgraph_attempts, 1);
        assert_eq!(config.subgraph_fraction_threshold, 0.5);
        assert!(config.allow_skip);
        assert!(config.escalation_model.is_none());
    }

    #[test]
    fn failure_context_minimal_constructor() {
        let ctx = FailureContext::new("plan-x", "task-y");
        assert_eq!(ctx.plan_id, "plan-x");
        assert_eq!(ctx.task_id, "task-y");
        assert_eq!(ctx.retry_count, 0);
        assert!(!ctx.is_compile_error);
        assert!(!ctx.is_test_failure);
        assert!(!ctx.is_timeout);
    }

    #[test]
    fn build_retry_context_includes_error_info() {
        let ctx = FailureContext {
            plan_id: "p".into(),
            task_id: "t".into(),
            retry_count: 1,
            is_compile_error: true,
            is_test_failure: true,
            is_timeout: false,
            failed_gate: Some("compile".into()),
            error_summary: "unresolved import foo".into(),
            dependent_count: 0,
            total_tasks: 1,
            current_model: None,
        };
        let context = build_retry_context(&ctx);
        assert!(context.contains("unresolved import foo"));
        assert!(context.contains("compilation errors"));
        assert!(context.contains("Tests failed"));
        assert!(context.contains("compile"));
    }

    #[test]
    fn serde_roundtrip_repair_action() {
        let action = RepairAction::ReplaceSubgraph {
            plan_id: "p1".into(),
            root_task_id: "t1".into(),
            affected_task_ids: vec!["t1".into(), "t2".into()],
            failure_hint: "compile error".into(),
        };
        let json = serde_json::to_string(&action).unwrap();
        let back: RepairAction = serde_json::from_str(&json).unwrap();
        assert_eq!(action, back);
    }

    #[test]
    fn serde_roundtrip_repair_decision() {
        let decision = RepairDecision {
            action: RepairAction::FullReplan {
                plan_id: "p".into(),
                reason: "test".into(),
            },
            level: RepairLevel::FullReplan,
            reasoning: "exhausted retries".into(),
            expected_stability: StabilityMetric {
                unchanged_tasks: 0,
                total_tasks: 10,
            },
        };
        let json = serde_json::to_string(&decision).unwrap();
        let back: RepairDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(back.level, RepairLevel::FullReplan);
        assert_eq!(back.expected_stability.total_tasks, 10);
    }
}
