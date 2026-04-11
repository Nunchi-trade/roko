//! Budget guardrails for routing and orchestration decisions.
//!
//! This module tracks cumulative spend at the task, session, and day level
//! and converts budget usage into coarse routing actions.

/// Budget enforcement state across multiple scopes.
#[derive(Debug, Clone)]
pub struct BudgetGuardrail {
    /// Maximum spend allowed per task, in USD.
    pub per_task_limit_usd: f64,
    /// Maximum spend allowed per session, in USD.
    pub per_session_limit_usd: f64,
    /// Maximum spend allowed per day, in USD.
    pub per_day_limit_usd: f64,
    /// Warning threshold expressed as a fraction in the range `0.0..=1.0`.
    pub warn_at_percent: f64,
    task_spent: f64,
    session_spent: f64,
    day_spent: f64,
}

/// Action to take once a budget threshold is crossed.
#[derive(Debug, Clone, PartialEq)]
pub enum BudgetAction {
    /// No budget issue detected.
    Ok,
    /// Warn while returning the percent of budget consumed.
    Warn {
        /// Fraction of the budget consumed.
        percent_used: f64,
        /// Level used for the warning.
        level: &'static str,
    },
    /// Spend is high enough that routing should favor cheaper models.
    RouteToCheaper,
    /// New sessions should be blocked to limit further exposure.
    BlockNewSessions,
    /// The budget has been exhausted.
    Block,
}

impl BudgetGuardrail {
    /// Create a new guardrail with explicit limits.
    #[must_use]
    pub const fn new(
        per_task_limit_usd: f64,
        per_session_limit_usd: f64,
        per_day_limit_usd: f64,
        warn_at_percent: f64,
    ) -> Self {
        Self {
            per_task_limit_usd,
            per_session_limit_usd,
            per_day_limit_usd,
            warn_at_percent,
            task_spent: 0.0,
            session_spent: 0.0,
            day_spent: 0.0,
        }
    }

    /// Record a cost against a budget level and return the resulting action.
    pub fn record_cost(&mut self, cost_usd: f64, level: &str) -> BudgetAction {
        match level {
            "task" => {
                self.task_spent += cost_usd;
                self.check_budget(self.task_spent, self.per_task_limit_usd)
            }
            "session" => {
                self.session_spent += cost_usd;
                self.check_budget(self.session_spent, self.per_session_limit_usd)
            }
            "day" => {
                self.day_spent += cost_usd;
                self.check_budget(self.day_spent, self.per_day_limit_usd)
            }
            _ => BudgetAction::Ok,
        }
    }

    /// Check a spend amount against a limit and map it to a routing action.
    fn check_budget(&self, spent: f64, limit: f64) -> BudgetAction {
        if limit <= 0.0 {
            return BudgetAction::Ok;
        }

        let pct = spent / limit;
        if pct >= 1.0 {
            BudgetAction::Block
        } else if pct >= 0.95 {
            BudgetAction::BlockNewSessions
        } else if pct >= 0.80 {
            BudgetAction::RouteToCheaper
        } else if pct >= self.warn_at_percent {
            BudgetAction::Warn {
                percent_used: pct,
                level: "budget",
            }
        } else {
            BudgetAction::Ok
        }
    }

    /// Reset accumulated task spend.
    pub fn reset_task(&mut self) {
        self.task_spent = 0.0;
    }

    /// Reset accumulated session spend.
    pub fn reset_session(&mut self) {
        self.session_spent = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::{BudgetAction, BudgetGuardrail};

    fn guardrail() -> BudgetGuardrail {
        BudgetGuardrail {
            per_task_limit_usd: 100.0,
            per_session_limit_usd: 100.0,
            per_day_limit_usd: 100.0,
            warn_at_percent: 0.75,
            task_spent: 0.0,
            session_spent: 0.0,
            day_spent: 0.0,
        }
    }

    #[test]
    fn budget_guardrail_thresholds() {
        let mut budget = guardrail();

        match budget.record_cost(79.0, "task") {
            BudgetAction::Warn {
                percent_used,
                level,
            } => {
                assert!((percent_used - 0.79).abs() < 1e-12);
                assert_eq!(level, "budget");
            }
            other => panic!("unexpected action: {other:?}"),
        }

        let mut budget = guardrail();
        assert_eq!(budget.record_cost(81.0, "task"), BudgetAction::RouteToCheaper);

        let mut budget = guardrail();
        assert_eq!(budget.record_cost(96.0, "task"), BudgetAction::BlockNewSessions);

        let mut budget = guardrail();
        assert_eq!(budget.record_cost(100.0, "task"), BudgetAction::Block);
    }
}
