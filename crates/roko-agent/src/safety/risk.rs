//! Adaptive-risk primitives for the safety layer.
//!
//! These structs implement the deterministic parts of the documented adaptive
//! risk model: confidence tracking, safety budgets, and lightweight
//! pre-execution scope sizing. They are intentionally dependency-light so the
//! existing [`crate::safety::SafetyLayer`] can apply them on the current
//! runtime path.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use roko_core::tool::ToolCall;

/// Tracks confidence for one dimension of agent competence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BetaDistribution {
    /// Success pseudo-count.
    pub alpha: f64,
    /// Failure pseudo-count.
    pub beta: f64,
}

impl BetaDistribution {
    /// Weakly pessimistic prior with mean `0.25`.
    #[must_use]
    pub const fn pessimistic_prior() -> Self {
        Self {
            alpha: 1.0,
            beta: 3.0,
        }
    }

    /// Posterior mean.
    #[must_use]
    pub fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Posterior variance.
    #[must_use]
    pub fn variance(&self) -> f64 {
        let sum = self.alpha + self.beta;
        (self.alpha * self.beta) / (sum * sum * (sum + 1.0))
    }

    /// Conservative lower bound for the 95% credible interval.
    #[must_use]
    pub fn lower_95(&self) -> f64 {
        (self.mean() - 1.96 * self.variance().sqrt()).max(0.0)
    }

    /// Record a success.
    pub fn record_success(&mut self) {
        self.alpha += 1.0;
    }

    /// Record a failure with the provided weight.
    pub fn record_failure(&mut self, weight: f64) {
        self.beta += weight.max(0.0);
    }
}

impl Default for BetaDistribution {
    fn default() -> Self {
        Self::pessimistic_prior()
    }
}

/// Confidence tracker over multiple competence dimensions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationalConfidenceTracker {
    /// Per-dimension posterior distributions.
    pub dimensions: HashMap<String, BetaDistribution>,
    /// Failure multiplier used for asymmetric learning.
    pub failure_weight: f64,
}

impl OperationalConfidenceTracker {
    /// Create a new tracker with the documented default failure weight.
    #[must_use]
    pub fn new() -> Self {
        Self {
            dimensions: HashMap::new(),
            failure_weight: 1.5,
        }
    }

    /// Register a dimension if it does not already exist.
    pub fn register_dimension(&mut self, name: &str) {
        self.dimensions
            .entry(name.to_string())
            .or_insert_with(BetaDistribution::pessimistic_prior);
    }

    /// Record a success for a dimension, creating it if needed.
    pub fn record_success(&mut self, dimension: &str) {
        self.register_dimension(dimension);
        if let Some(distribution) = self.dimensions.get_mut(dimension) {
            distribution.record_success();
        }
    }

    /// Record a failure for a dimension, creating it if needed.
    pub fn record_failure(&mut self, dimension: &str) {
        self.register_dimension(dimension);
        if let Some(distribution) = self.dimensions.get_mut(dimension) {
            distribution.record_failure(self.failure_weight);
        }
    }

    /// Composite confidence as the geometric mean of lower 95% bounds.
    #[must_use]
    pub fn composite_confidence(&self) -> f64 {
        if self.dimensions.is_empty() {
            return 0.0;
        }

        let product: f64 = self
            .dimensions
            .values()
            .map(|distribution| distribution.lower_95().max(0.001))
            .product();

        product.powf(1.0 / self.dimensions.len() as f64)
    }
}

impl Default for OperationalConfidenceTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Sigmoid confidence multiplier used for adaptive budget sizing.
#[must_use]
pub fn confidence_multiplier(confidence: f64) -> f64 {
    let clamped = confidence.clamp(0.0, 1.0);
    let sigmoid = 1.0 / (1.0 + (-10.0 * (clamped - 0.5)).exp());
    0.1 + 0.4 * sigmoid
}

/// Compute an effective operating limit under confidence and context.
#[must_use]
pub fn effective_limit(
    hard_shield_limit: f64,
    confidence: f64,
    failure_rate: f64,
    task_complexity: f64,
    domain_risk: f64,
) -> f64 {
    let base_multiplier = 0.2 + 0.8 * confidence.clamp(0.0, 1.0);
    let failure_factor = 1.0 - (failure_rate.clamp(0.0, 1.0) * 0.5).min(0.8);
    let complexity_factor = 1.0 - (task_complexity.clamp(0.0, 1.0) * 0.3).min(0.6);
    let risk_factor = 1.0 - (domain_risk.clamp(0.0, 1.0) * 0.4).min(0.7);
    hard_shield_limit * base_multiplier * failure_factor * complexity_factor * risk_factor
}

/// Hard limits on autonomous risk consumption for a session or task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SafetyBudget {
    /// Total irreversibility score that may be consumed.
    pub irreversibility_limit: f64,
    /// Maximum unique files touched.
    pub blast_radius_file_limit: usize,
    /// Maximum number of tool-like external actions.
    pub footprint_limit: usize,
    /// Number of low-confidence decisions that may proceed.
    pub uncertainty_tokens: usize,
    /// Estimated or actual dollar-cost ceiling.
    pub cost_limit_usd: f64,
}

impl Default for SafetyBudget {
    fn default() -> Self {
        Self {
            irreversibility_limit: 10.0,
            blast_radius_file_limit: 50,
            footprint_limit: 500,
            uncertainty_tokens: 10,
            cost_limit_usd: 50.0,
        }
    }
}

/// Running usage counters for a [`SafetyBudget`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SafetyBudgetUsage {
    /// Irreversibility consumed so far.
    pub irreversibility_consumed: f64,
    /// Unique files touched so far.
    pub files_touched: HashSet<String>,
    /// Count of external actions or tool calls.
    pub footprint_count: usize,
    /// Low-confidence decisions already spent.
    pub uncertainty_tokens_used: usize,
    /// Estimated or actual cost already consumed.
    pub cost_consumed_usd: f64,
}

/// Budget dimensions tracked by the safety layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetDimension {
    /// Budget on irreversible or costly-to-revert changes.
    Irreversibility,
    /// Budget on the number of files or artifacts touched.
    BlastRadius,
    /// Budget on action count and runtime footprint.
    Footprint,
    /// Budget on low-confidence decisions.
    Uncertainty,
    /// Budget on direct cost.
    Cost,
}

impl BudgetDimension {
    /// Stable string label for diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Irreversibility => "irreversibility",
            Self::BlastRadius => "blast_radius",
            Self::Footprint => "footprint",
            Self::Uncertainty => "uncertainty",
            Self::Cost => "cost",
        }
    }
}

/// Result of a budget admission check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetCheckResult {
    /// The action is within budget.
    WithinBudget,
    /// The action exceeds the specified budget dimension.
    Exceeded(BudgetDimension),
}

/// Estimated pre-execution impact of a proposed action.
#[derive(Debug, Clone, PartialEq)]
pub struct ProposedAction {
    /// Irreversibility score for the action.
    pub irreversibility_score: f64,
    /// Files expected to be modified by the action.
    pub files_modified: Vec<String>,
    /// Number of tool-call units the action consumes.
    pub tool_calls: usize,
    /// Confidence for the action. Values below the tracker's threshold spend
    /// an uncertainty token.
    pub confidence: f64,
    /// Estimated dollar cost for the action.
    pub estimated_cost: f64,
}

impl ProposedAction {
    /// Build a conservative estimate from an inbound tool call.
    #[must_use]
    pub fn from_tool_call(call: &ToolCall) -> Self {
        let files_modified = modified_files(call);
        let estimated_cost = call
            .arguments
            .get("estimated_cost_usd")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0);
        let confidence = call
            .arguments
            .get("confidence")
            .and_then(|value| value.as_f64())
            .unwrap_or(1.0);

        Self {
            irreversibility_score: irreversibility_score(&call.name, &call.arguments),
            files_modified,
            tool_calls: 1,
            confidence,
            estimated_cost,
        }
    }

    /// Build a conservative estimate from a raw subprocess launch.
    #[must_use]
    pub fn from_exec_command(program: &str, args: &[String]) -> Self {
        let command = if args.is_empty() {
            program.to_string()
        } else {
            format!("{program} {}", args.join(" "))
        };
        Self {
            irreversibility_score: score_command_irreversibility(&command),
            files_modified: Vec::new(),
            tool_calls: 1,
            confidence: 1.0,
            estimated_cost: 0.0,
        }
    }
}

/// Observed post-execution impact of an action.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletedAction {
    /// Irreversibility score consumed by the action.
    pub irreversibility_score: f64,
    /// Files actually modified.
    pub files_modified: Vec<String>,
    /// Number of tool-call units consumed.
    pub tool_calls: usize,
    /// Confidence recorded for the action.
    pub confidence: f64,
    /// Actual cost charged to the action.
    pub actual_cost: f64,
}

impl From<&ProposedAction> for CompletedAction {
    fn from(action: &ProposedAction) -> Self {
        Self {
            irreversibility_score: action.irreversibility_score,
            files_modified: action.files_modified.clone(),
            tool_calls: action.tool_calls,
            confidence: action.confidence,
            actual_cost: action.estimated_cost,
        }
    }
}

/// Stateful budget tracker used by the safety layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SafetyBudgetTracker {
    /// Budget caps for the current run.
    pub budget: SafetyBudget,
    /// Current usage counters.
    pub usage: SafetyBudgetUsage,
    /// Threshold below which confidence consumes an uncertainty token.
    pub uncertainty_threshold: f64,
}

impl SafetyBudgetTracker {
    /// Create a tracker with empty usage.
    #[must_use]
    pub fn new(budget: SafetyBudget) -> Self {
        Self {
            budget,
            usage: SafetyBudgetUsage::default(),
            uncertainty_threshold: 0.5,
        }
    }

    /// Check whether a proposed action fits inside the remaining budget.
    #[must_use]
    pub fn check(&self, action: &ProposedAction) -> BudgetCheckResult {
        let usage = &self.usage;
        let budget = &self.budget;

        if usage.irreversibility_consumed + action.irreversibility_score
            > budget.irreversibility_limit
        {
            return BudgetCheckResult::Exceeded(BudgetDimension::Irreversibility);
        }

        let mut files = usage.files_touched.clone();
        files.extend(action.files_modified.iter().cloned());
        if files.len() > budget.blast_radius_file_limit {
            return BudgetCheckResult::Exceeded(BudgetDimension::BlastRadius);
        }

        if usage.footprint_count + action.tool_calls > budget.footprint_limit {
            return BudgetCheckResult::Exceeded(BudgetDimension::Footprint);
        }

        if action.confidence < self.uncertainty_threshold
            && usage.uncertainty_tokens_used >= budget.uncertainty_tokens
        {
            return BudgetCheckResult::Exceeded(BudgetDimension::Uncertainty);
        }

        if usage.cost_consumed_usd + action.estimated_cost > budget.cost_limit_usd {
            return BudgetCheckResult::Exceeded(BudgetDimension::Cost);
        }

        BudgetCheckResult::WithinBudget
    }

    /// Record a completed action against the usage counters.
    pub fn consume(&mut self, action: &CompletedAction) {
        self.usage.irreversibility_consumed += action.irreversibility_score;
        self.usage
            .files_touched
            .extend(action.files_modified.iter().cloned());
        self.usage.footprint_count += action.tool_calls;
        if action.confidence < self.uncertainty_threshold {
            self.usage.uncertainty_tokens_used += 1;
        }
        self.usage.cost_consumed_usd += action.actual_cost;
    }

    /// Atomically check and consume a proposed action using its estimate.
    #[must_use]
    pub fn check_and_consume(&mut self, action: &ProposedAction) -> BudgetCheckResult {
        let result = self.check(action);
        if matches!(result, BudgetCheckResult::WithinBudget) {
            self.consume(&CompletedAction::from(action));
        }
        result
    }
}

impl Default for SafetyBudgetTracker {
    fn default() -> Self {
        Self::new(SafetyBudget::default())
    }
}

/// Compute the irreversibility score for a tool call.
#[must_use]
pub fn irreversibility_score(tool: &str, args: &serde_json::Value) -> f64 {
    match tool {
        "read_file" | "glob" | "grep" | "ls" => 0.0,
        "write_file" => 0.2,
        "edit_file" | "multi_edit" | "apply_patch" | "notebook_edit" => 0.3,
        "bash" | "run_tests" => score_bash_irreversibility(args),
        "git_commit" => 0.3,
        "git_push" => 0.6,
        "web_fetch" | "web_search" => 0.0,
        _ => 0.5,
    }
}

fn score_bash_irreversibility(args: &serde_json::Value) -> f64 {
    let command = args
        .get("command")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    score_command_irreversibility(command)
}

fn score_command_irreversibility(command: &str) -> f64 {
    let normalized = command.trim().to_ascii_lowercase();
    if normalized.starts_with("ls")
        || normalized.starts_with("cat")
        || normalized.starts_with("echo")
        || normalized.starts_with("grep")
    {
        0.0
    } else if normalized.contains("rm ") || normalized.contains("rmdir") {
        0.8
    } else if normalized.starts_with("cargo build") || normalized.starts_with("cargo test") {
        0.1
    } else if normalized.starts_with("cargo publish") {
        0.9
    } else if normalized.contains("git push") {
        0.6
    } else {
        0.4
    }
}

fn modified_files(call: &ToolCall) -> Vec<String> {
    let mut files = Vec::new();
    for key in ["file_path", "path", "target_file"] {
        if let Some(path) = call.arguments.get(key).and_then(|value| value.as_str()) {
            files.push(path.to_string());
        }
    }
    if let Some(paths) = call
        .arguments
        .get("paths")
        .and_then(|value| value.as_array())
    {
        for path in paths.iter().filter_map(|value| value.as_str()) {
            files.push(path.to_string());
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_tracker_recovers_after_successes() {
        let mut tracker = OperationalConfidenceTracker::new();
        tracker.record_failure("tool_success");
        let after_failure = tracker.composite_confidence();
        tracker.record_success("tool_success");
        tracker.record_success("tool_success");
        assert!(tracker.composite_confidence() > after_failure);
    }

    #[test]
    fn budget_tracker_blocks_footprint_exhaustion() {
        let mut tracker = SafetyBudgetTracker::new(SafetyBudget {
            footprint_limit: 1,
            ..SafetyBudget::default()
        });
        let action = ProposedAction {
            irreversibility_score: 0.0,
            files_modified: Vec::new(),
            tool_calls: 1,
            confidence: 1.0,
            estimated_cost: 0.0,
        };
        assert_eq!(
            tracker.check_and_consume(&action),
            BudgetCheckResult::WithinBudget
        );
        assert_eq!(
            tracker.check_and_consume(&action),
            BudgetCheckResult::Exceeded(BudgetDimension::Footprint)
        );
    }

    #[test]
    fn irreversibility_scores_rm_as_high_risk() {
        let args = serde_json::json!({ "command": "rm -rf tmp" });
        assert_eq!(irreversibility_score("bash", &args), 0.8);
    }
}
