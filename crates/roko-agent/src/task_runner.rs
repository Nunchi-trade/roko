//! Task-runner composition types for the model-routing pipeline.
//!
//! This module defines the top-level runtime shape that will coordinate agent
//! execution, events, anomaly detection, budget enforcement, conductor actions,
//! and cost accounting across task iterations.

use crate::{Agent, Usage};
use roko_core::Signal;

/// Composition point for the task execution pipeline.
pub struct TaskRunner {
    /// The task-facing agent implementation, such as Claude CLI or the tool loop.
    pub agent: Box<dyn Agent>,
    /// Event stream publisher for runtime feedback.
    pub event_bus: EventBus,
    /// Session-local anomaly detector.
    pub anomaly: AnomalyDetector,
    /// Budget guardrail applied across task iterations.
    pub budget: BudgetGuardrail,
    /// Learned conductor policy for intervention decisions.
    pub conductor: ConductorBandit,
    /// Pricing table used to compute aggregate cost.
    pub cost_table: CostTable,
    /// Requested model slug for this task.
    pub model_slug: String,
    /// Provider identifier for this task.
    pub provider_id: String,
    /// Maximum task-loop iterations before aborting.
    pub max_iterations: u32,
}

/// Aggregated outcome from a task runner execution.
pub struct TaskResult {
    /// Final output signal emitted by the agent pipeline.
    pub output: Signal,
    /// Total usage accumulated across all iterations.
    pub total_usage: Usage,
    /// Total cost accumulated across all iterations.
    pub total_cost_usd: f64,
    /// Number of iterations consumed by the task run.
    pub iterations: u32,
    /// Whether the downstream gate accepted the result.
    pub gate_passed: bool,
    /// Conductor actions taken while processing the task.
    pub conductor_actions: Vec<ConductorAction>,
}

/// Runtime event bus placeholder owned by the task runner.
#[derive(Debug, Clone, Default)]
pub struct EventBus;

/// Session-local anomaly detector placeholder owned by the task runner.
#[derive(Debug, Clone, Default)]
pub struct AnomalyDetector;

/// Budget guardrail placeholder owned by the task runner.
#[derive(Debug, Clone, Default)]
pub struct BudgetGuardrail;

/// Learned conductor policy placeholder owned by the task runner.
#[derive(Debug, Clone, Default)]
pub struct ConductorBandit;

/// Per-model pricing table placeholder owned by the task runner.
#[derive(Debug, Clone, Default)]
pub struct CostTable;

/// Targeted hint variants available to conductor interventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HintType {
    /// Inject an enriched error digest into the next retry.
    #[default]
    ErrorDigest,
    /// Suggest a relevant reusable skill or recipe.
    SkillSuggestion,
    /// Request a simpler implementation path.
    SimplifyApproach,
}

/// Intervention selected by the conductor while a task is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConductorAction {
    /// Continue without extra intervention.
    #[default]
    Continue,
    /// Inject a targeted hint before the next retry.
    InjectHint(HintType),
    /// Switch to a different model tier.
    SwitchModel,
    /// Restart the task from a cleaner state.
    Restart,
    /// Abort the task.
    Abort,
}
