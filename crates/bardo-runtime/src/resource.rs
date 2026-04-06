//! Resource accounting: token, cost, and time budgets.
//!
//! Tracks per-plan and per-task resource consumption against budgets.
//! When a budget is exceeded, the system can throttle, warn, or halt.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// A resource budget with usage tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceAccount {
    /// Token budget.
    pub tokens: BudgetEntry<u64>,
    /// Cost budget (USD).
    pub cost: BudgetEntry<f64>,
    /// Time budget.
    #[serde(with = "duration_serde")]
    pub time_limit: Duration,
    /// When tracking started.
    #[serde(skip)]
    pub started_at: Option<Instant>,
    /// Label for logging.
    pub label: String,
}

/// A single budget entry with limit and usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetEntry<T> {
    /// Maximum allowed.
    pub limit: T,
    /// Current usage.
    pub used: T,
}

impl ResourceAccount {
    /// Create a new account with the given limits.
    pub fn new(label: impl Into<String>, token_limit: u64, cost_limit: f64, time_limit: Duration) -> Self {
        Self {
            tokens: BudgetEntry { limit: token_limit, used: 0 },
            cost: BudgetEntry { limit: cost_limit, used: 0.0 },
            time_limit,
            started_at: Some(Instant::now()),
            label: label.into(),
        }
    }

    /// Record token usage.
    pub const fn record_tokens(&mut self, input: u64, output: u64) {
        self.tokens.used += input + output;
    }

    /// Record cost.
    pub fn record_cost(&mut self, usd: f64) {
        self.cost.used += usd;
    }

    /// Whether the token budget is exceeded.
    pub const fn tokens_exceeded(&self) -> bool {
        self.tokens.used > self.tokens.limit
    }

    /// Whether the cost budget is exceeded.
    pub fn cost_exceeded(&self) -> bool {
        self.cost.used > self.cost.limit
    }

    /// Whether the time budget is exceeded.
    pub fn time_exceeded(&self) -> bool {
        self.started_at
            .is_some_and(|s| s.elapsed() > self.time_limit)
    }

    /// Whether any budget is exceeded.
    pub fn any_exceeded(&self) -> bool {
        self.tokens_exceeded() || self.cost_exceeded() || self.time_exceeded()
    }

    /// Token utilisation as a fraction (0.0 to 1.0).
    #[allow(clippy::cast_precision_loss)]
    pub fn token_utilisation(&self) -> f64 {
        if self.tokens.limit == 0 { return 0.0; }
        self.tokens.used as f64 / self.tokens.limit as f64
    }

    /// Cost utilisation as a fraction (0.0–1.0).
    pub fn cost_utilisation(&self) -> f64 {
        if self.cost.limit == 0.0 { return 0.0; }
        self.cost.used / self.cost.limit
    }

    /// Time utilisation as a fraction (0.0–1.0).
    pub fn time_utilisation(&self) -> f64 {
        let elapsed = self.started_at.map(|s| s.elapsed()).unwrap_or_default();
        if self.time_limit.is_zero() { return 0.0; }
        elapsed.as_secs_f64() / self.time_limit.as_secs_f64()
    }

    /// Remaining token budget.
    pub const fn tokens_remaining(&self) -> u64 {
        self.tokens.limit.saturating_sub(self.tokens.used)
    }

    /// Remaining cost budget.
    pub fn cost_remaining(&self) -> f64 {
        (self.cost.limit - self.cost.used).max(0.0)
    }
}

/// Default budgets for different plan complexity tiers.
impl ResourceAccount {
    /// Budget for a trivial plan.
    pub fn trivial(label: impl Into<String>) -> Self {
        Self::new(label, 50_000, 0.50, Duration::from_secs(5 * 60))
    }

    /// Budget for a simple plan.
    pub fn simple(label: impl Into<String>) -> Self {
        Self::new(label, 200_000, 2.00, Duration::from_secs(15 * 60))
    }

    /// Budget for a standard plan.
    pub fn standard(label: impl Into<String>) -> Self {
        Self::new(label, 500_000, 5.00, Duration::from_secs(30 * 60))
    }

    /// Budget for a complex plan.
    pub fn complex(label: impl Into<String>) -> Self {
        Self::new(label, 2_000_000, 20.00, Duration::from_secs(60 * 60))
    }
}

mod duration_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;
    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_secs())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_secs(u64::deserialize(d)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_tracking() {
        let mut acct = ResourceAccount::new("plan-01", 1000, 5.0, Duration::from_secs(3600));
        acct.record_tokens(400, 100);
        acct.record_cost(1.5);

        assert_eq!(acct.tokens.used, 500);
        assert!(!acct.tokens_exceeded());
        assert!(!acct.cost_exceeded());
        assert!((acct.token_utilisation() - 0.5).abs() < 0.01);
        assert!((acct.cost_utilisation() - 0.3).abs() < 0.01);
    }

    #[test]
    fn budget_exceeded() {
        let mut acct = ResourceAccount::new("plan-02", 100, 1.0, Duration::from_secs(1));
        acct.record_tokens(80, 30);
        assert!(acct.tokens_exceeded());
        assert!(acct.any_exceeded());
    }

    #[test]
    fn tier_budgets() {
        let t = ResourceAccount::trivial("t");
        let s = ResourceAccount::standard("s");
        let c = ResourceAccount::complex("c");
        assert!(t.tokens.limit < s.tokens.limit);
        assert!(s.tokens.limit < c.tokens.limit);
        assert!(t.cost.limit < c.cost.limit);
    }
}
