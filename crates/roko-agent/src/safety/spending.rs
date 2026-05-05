//! Spending limiter safety hook (TOOL-02, hook #3).
//!
//! Checks per-turn and daily cost budgets before permitting a tool call.
//! This hook wraps a shared [`BudgetTracker`] and rejects tool invocations
//! when the budget is exhausted or critically constrained.
//!
//! The limiter also enforces a per-tool cost estimate: tools tagged with
//! estimated costs (e.g., LLM calls, chain transactions) are checked against
//! the remaining budget before execution.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use roko_core::tool::{ToolContext, ToolDef, ToolError};

use crate::lifecycle::{BudgetStatus, BudgetTracker};

use super::hooks::{HookDecision, SafetyHook};

/// Per-tool cost estimate used by the spending limiter to pre-check
/// whether a tool call would exceed the remaining budget.
#[derive(Debug, Clone)]
pub struct ToolCostEstimate {
    /// Tool name pattern (exact match).
    pub tool_name: String,
    /// Estimated cost in USD per invocation.
    pub estimated_cost_usd: f64,
}

/// Cost budget enforcement hook.
///
/// Place this after `AllowlistGuard` in the hook chain. It checks:
///
/// 1. Whether the daily/lifetime budget is already exhausted.
/// 2. Whether the estimated cost of the tool call would exceed the
///    remaining budget.
///
/// When budget is exhausted, the hook rejects with a clear reason that
/// can be surfaced to the operator or used for degradation decisions.
#[derive(Debug, Clone)]
pub struct SpendingLimiter {
    /// Shared budget tracker (same instance used by the tool loop).
    budget: Arc<Mutex<BudgetTracker>>,
    /// Per-tool cost estimates for pre-flight budget checks.
    cost_estimates: Vec<ToolCostEstimate>,
    /// Whether to reject on Warning status (conservative mode).
    /// When false, only Exhausted status triggers rejection.
    reject_on_critical: bool,
}

impl SpendingLimiter {
    /// Create a spending limiter from a shared budget tracker.
    pub fn new(budget: Arc<Mutex<BudgetTracker>>) -> Self {
        Self {
            budget,
            cost_estimates: Vec::new(),
            reject_on_critical: false,
        }
    }

    /// Enable rejection on Critical status (not just Exhausted).
    ///
    /// When enabled, tool calls are blocked at 90% budget utilization
    /// instead of waiting for full exhaustion.
    #[must_use]
    pub fn reject_on_critical(mut self, enabled: bool) -> Self {
        self.reject_on_critical = enabled;
        self
    }

    /// Register per-tool cost estimates for pre-flight checks.
    #[must_use]
    pub fn with_cost_estimates(mut self, estimates: Vec<ToolCostEstimate>) -> Self {
        self.cost_estimates = estimates;
        self
    }

    /// Look up the estimated cost for a tool.
    fn estimated_cost(&self, tool_name: &str) -> Option<f64> {
        self.cost_estimates
            .iter()
            .find(|e| e.tool_name == tool_name)
            .map(|e| e.estimated_cost_usd)
    }
}

#[async_trait]
impl SafetyHook for SpendingLimiter {
    async fn on_tool_call(
        &self,
        tool: &ToolDef,
        _params: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<HookDecision, ToolError> {
        let guard = self.budget.lock();

        let status = guard.check();

        match status {
            BudgetStatus::Exhausted => {
                return Ok(HookDecision::Reject(format!(
                    "budget exhausted: daily ${:.2} / ${:.2} (lifetime ${:.2})",
                    guard.daily_cost_usd,
                    guard.config.max_daily_inference_usd,
                    guard.lifetime_cost_usd,
                )));
            }
            BudgetStatus::Critical if self.reject_on_critical => {
                return Ok(HookDecision::Reject(format!(
                    "budget critically constrained ({:.0}% of daily limit): ${:.2} / ${:.2}",
                    guard.daily_utilization() * 100.0,
                    guard.daily_cost_usd,
                    guard.config.max_daily_inference_usd,
                )));
            }
            _ => {}
        }

        // Pre-flight check: would this tool call exceed remaining budget?
        if let Some(estimated) = self.estimated_cost(&tool.name) {
            let remaining = guard.remaining_daily_usd();
            if estimated > remaining && remaining > 0.0 {
                return Ok(HookDecision::Reject(format!(
                    "tool `{}` estimated cost ${:.4} exceeds remaining daily budget ${:.4}",
                    tool.name, estimated, remaining,
                )));
            }
        }

        Ok(HookDecision::Allow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::{BudgetConfig, TurnCostRecord};
    use roko_core::tool::{ToolCategory, ToolPermission};

    fn test_ctx() -> ToolContext {
        ToolContext::testing("/tmp/worktree")
    }

    fn test_tool(name: &str) -> ToolDef {
        ToolDef::new(
            name,
            "test tool",
            ToolCategory::Read,
            ToolPermission::read_only(),
        )
    }

    fn make_budget(max_daily: f64) -> Arc<parking_lot::Mutex<BudgetTracker>> {
        Arc::new(parking_lot::Mutex::new(BudgetTracker::new(BudgetConfig {
            max_daily_inference_usd: max_daily,
            ..BudgetConfig::default()
        })))
    }

    fn spend(budget: &Arc<parking_lot::Mutex<BudgetTracker>>, amount: f64) {
        let mut guard = budget.lock();
        guard.record_turn(&TurnCostRecord {
            turn_id: "test".into(),
            model: "test".into(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            estimated_cost_usd: amount,
            cognitive_tier: crate::lifecycle::CognitiveTier::Gamma,
            t0_suppressed: false,
            timestamp: 0,
        });
    }

    #[tokio::test]
    async fn allows_within_budget() {
        let budget = make_budget(10.0);
        let limiter = SpendingLimiter::new(budget);
        let tool = test_tool("read_file");
        let params = serde_json::json!({});
        let result = limiter
            .on_tool_call(&tool, &params, &test_ctx())
            .await
            .unwrap();
        assert_eq!(result, HookDecision::Allow);
    }

    #[tokio::test]
    async fn rejects_when_exhausted() {
        let budget = make_budget(1.0);
        spend(&budget, 1.5); // Over budget
        let limiter = SpendingLimiter::new(budget);
        let tool = test_tool("bash");
        let params = serde_json::json!({});
        let result = limiter
            .on_tool_call(&tool, &params, &test_ctx())
            .await
            .unwrap();
        assert!(matches!(result, HookDecision::Reject(_)));
    }

    #[tokio::test]
    async fn allows_critical_by_default() {
        let budget = make_budget(10.0);
        spend(&budget, 9.5); // 95% - critical
        let limiter = SpendingLimiter::new(budget);
        let tool = test_tool("read_file");
        let params = serde_json::json!({});
        let result = limiter
            .on_tool_call(&tool, &params, &test_ctx())
            .await
            .unwrap();
        assert_eq!(result, HookDecision::Allow);
    }

    #[tokio::test]
    async fn rejects_critical_when_enabled() {
        let budget = make_budget(10.0);
        spend(&budget, 9.5); // 95% - critical
        let limiter = SpendingLimiter::new(budget).reject_on_critical(true);
        let tool = test_tool("read_file");
        let params = serde_json::json!({});
        let result = limiter
            .on_tool_call(&tool, &params, &test_ctx())
            .await
            .unwrap();
        assert!(matches!(result, HookDecision::Reject(_)));
    }

    #[tokio::test]
    async fn rejects_when_tool_cost_exceeds_remaining() {
        let budget = make_budget(10.0);
        spend(&budget, 9.8); // $0.20 remaining
        let limiter = SpendingLimiter::new(budget).with_cost_estimates(vec![ToolCostEstimate {
            tool_name: "expensive_tool".into(),
            estimated_cost_usd: 0.50,
        }]);
        let tool = test_tool("expensive_tool");
        let params = serde_json::json!({});
        let result = limiter
            .on_tool_call(&tool, &params, &test_ctx())
            .await
            .unwrap();
        assert!(matches!(result, HookDecision::Reject(_)));
    }

    #[tokio::test]
    async fn allows_cheap_tool_with_sufficient_remaining() {
        let budget = make_budget(10.0);
        spend(&budget, 5.0); // $5.00 remaining
        let limiter = SpendingLimiter::new(budget).with_cost_estimates(vec![ToolCostEstimate {
            tool_name: "cheap_tool".into(),
            estimated_cost_usd: 0.01,
        }]);
        let tool = test_tool("cheap_tool");
        let params = serde_json::json!({});
        let result = limiter
            .on_tool_call(&tool, &params, &test_ctx())
            .await
            .unwrap();
        assert_eq!(result, HookDecision::Allow);
    }
}
