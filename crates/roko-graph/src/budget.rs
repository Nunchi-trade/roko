//! Budget and deadline enforcement for graph execution.
//!
//! [`BudgetTracker`] monitors token usage, cost, and elapsed time against
//! configured limits. The graph engine checks the tracker before each node
//! execution and short-circuits with `BudgetExceeded` when any limit is hit.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::error::{GraphError, Result};
use crate::types::GraphConfig;

/// Tracks resource consumption during graph execution and enforces limits.
#[derive(Debug)]
pub struct BudgetTracker {
    /// Total tokens consumed so far.
    tokens_used: AtomicU64,
    /// Total cost in USD (stored as microdollars for atomics).
    cost_microdollars: AtomicU64,
    /// When execution started.
    start_time: Instant,
    /// Configured limits.
    limits: BudgetLimits,
    /// Detailed per-node cost breakdown (for reporting).
    breakdown: Mutex<Vec<NodeCost>>,
}

/// Configured budget limits extracted from [`GraphConfig`].
#[derive(Debug, Clone)]
pub struct BudgetLimits {
    /// Maximum total tokens allowed.
    pub max_tokens: Option<u64>,
    /// Maximum total cost in USD.
    pub max_cost_usd: Option<f64>,
    /// Maximum wall-clock time.
    pub deadline: Option<Duration>,
}

/// Per-node cost record.
#[derive(Debug, Clone)]
pub struct NodeCost {
    /// Node ID.
    pub node_id: String,
    /// Tokens consumed by this node.
    pub tokens: u64,
    /// Cost in USD for this node.
    pub cost_usd: f64,
    /// Wall-clock time for this node.
    pub duration: Duration,
}

impl BudgetTracker {
    /// Create a new tracker from graph configuration.
    #[must_use]
    pub fn from_config(config: &GraphConfig) -> Self {
        Self {
            tokens_used: AtomicU64::new(0),
            cost_microdollars: AtomicU64::new(0),
            start_time: Instant::now(),
            limits: BudgetLimits {
                max_tokens: config.max_tokens,
                max_cost_usd: config.max_cost_usd,
                deadline: config.deadline,
            },
            breakdown: Mutex::new(Vec::new()),
        }
    }

    /// Create a tracker with explicit limits (useful for testing).
    #[must_use]
    pub fn with_limits(limits: BudgetLimits) -> Self {
        Self {
            tokens_used: AtomicU64::new(0),
            cost_microdollars: AtomicU64::new(0),
            start_time: Instant::now(),
            limits,
            breakdown: Mutex::new(Vec::new()),
        }
    }

    /// Check if budget allows another node execution. Returns `Ok(())` if within
    /// limits, or `Err(GraphError::BudgetExceeded)` if any limit is breached.
    pub fn check(&self) -> Result<()> {
        // Check token limit.
        if let Some(max) = self.limits.max_tokens {
            let used = self.tokens_used.load(Ordering::Relaxed);
            if used >= max {
                return Err(GraphError::BudgetExceeded {
                    reason: format!("token limit reached: {used}/{max}"),
                });
            }
        }

        // Check cost limit.
        if let Some(max) = self.limits.max_cost_usd {
            let used_usd = self.cost_usd();
            if used_usd >= max {
                return Err(GraphError::BudgetExceeded {
                    reason: format!("cost limit reached: ${used_usd:.4}/{max:.4}"),
                });
            }
        }

        // Check deadline.
        if let Some(deadline) = self.limits.deadline {
            let elapsed = self.elapsed();
            if elapsed >= deadline {
                return Err(GraphError::BudgetExceeded {
                    reason: format!(
                        "deadline exceeded: {:.1}s/{:.1}s",
                        elapsed.as_secs_f64(),
                        deadline.as_secs_f64()
                    ),
                });
            }
        }

        Ok(())
    }

    /// Record resource consumption from a completed node.
    pub fn record(&self, node_id: &str, tokens: u64, cost_usd: f64, duration: Duration) {
        self.tokens_used.fetch_add(tokens, Ordering::Relaxed);
        // Store cost as microdollars (1 USD = 1_000_000 microdollars).
        let microdollars = (cost_usd * 1_000_000.0) as u64;
        self.cost_microdollars
            .fetch_add(microdollars, Ordering::Relaxed);

        self.breakdown.lock().push(NodeCost {
            node_id: node_id.to_string(),
            tokens,
            cost_usd,
            duration,
        });
    }

    /// Total tokens consumed so far.
    #[must_use]
    pub fn tokens_used(&self) -> u64 {
        self.tokens_used.load(Ordering::Relaxed)
    }

    /// Total cost in USD consumed so far.
    #[must_use]
    pub fn cost_usd(&self) -> f64 {
        let microdollars = self.cost_microdollars.load(Ordering::Relaxed);
        microdollars as f64 / 1_000_000.0
    }

    /// Elapsed wall-clock time since execution started.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Remaining budget as USD, if a cost limit is configured.
    #[must_use]
    pub fn remaining_cost_usd(&self) -> Option<f64> {
        self.limits.max_cost_usd.map(|max| max - self.cost_usd())
    }

    /// Remaining time before deadline, if configured.
    #[must_use]
    pub fn remaining_time(&self) -> Option<Duration> {
        self.limits
            .deadline
            .map(|d| d.saturating_sub(self.elapsed()))
    }

    /// Get the cost breakdown for all recorded nodes.
    #[must_use]
    pub fn breakdown(&self) -> Vec<NodeCost> {
        self.breakdown.lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_starts_empty() {
        let tracker = BudgetTracker::with_limits(BudgetLimits {
            max_tokens: Some(1000),
            max_cost_usd: Some(1.0),
            deadline: None,
        });
        assert_eq!(tracker.tokens_used(), 0);
        assert!((tracker.cost_usd() - 0.0).abs() < f64::EPSILON);
        assert!(tracker.check().is_ok());
    }

    #[test]
    fn token_limit_exceeded() {
        let tracker = BudgetTracker::with_limits(BudgetLimits {
            max_tokens: Some(100),
            max_cost_usd: None,
            deadline: None,
        });
        tracker.record("n1", 80, 0.01, Duration::from_millis(100));
        assert!(tracker.check().is_ok());

        tracker.record("n2", 30, 0.01, Duration::from_millis(50));
        let err = tracker.check().unwrap_err();
        assert!(matches!(err, GraphError::BudgetExceeded { .. }));
    }

    #[test]
    fn cost_limit_exceeded() {
        let tracker = BudgetTracker::with_limits(BudgetLimits {
            max_tokens: None,
            max_cost_usd: Some(0.50),
            deadline: None,
        });
        tracker.record("n1", 100, 0.30, Duration::from_millis(100));
        assert!(tracker.check().is_ok());

        tracker.record("n2", 100, 0.25, Duration::from_millis(50));
        let err = tracker.check().unwrap_err();
        assert!(matches!(err, GraphError::BudgetExceeded { .. }));
    }

    #[test]
    fn deadline_exceeded() {
        let tracker = BudgetTracker::with_limits(BudgetLimits {
            max_tokens: None,
            max_cost_usd: None,
            deadline: Some(Duration::from_millis(0)), // Already expired.
        });
        let err = tracker.check().unwrap_err();
        assert!(matches!(err, GraphError::BudgetExceeded { .. }));
    }

    #[test]
    fn no_limits_always_passes() {
        let tracker = BudgetTracker::with_limits(BudgetLimits {
            max_tokens: None,
            max_cost_usd: None,
            deadline: None,
        });
        tracker.record("n1", 999_999, 999.0, Duration::from_secs(9999));
        assert!(tracker.check().is_ok());
    }

    #[test]
    fn remaining_cost_computed_correctly() {
        let tracker = BudgetTracker::with_limits(BudgetLimits {
            max_tokens: None,
            max_cost_usd: Some(1.0),
            deadline: None,
        });
        tracker.record("n1", 0, 0.35, Duration::ZERO);
        let remaining = tracker.remaining_cost_usd().unwrap();
        assert!((remaining - 0.65).abs() < 0.001);
    }

    #[test]
    fn breakdown_records_all_nodes() {
        let tracker = BudgetTracker::with_limits(BudgetLimits {
            max_tokens: None,
            max_cost_usd: None,
            deadline: None,
        });
        tracker.record("n1", 50, 0.01, Duration::from_millis(10));
        tracker.record("n2", 75, 0.02, Duration::from_millis(20));
        let bd = tracker.breakdown();
        assert_eq!(bd.len(), 2);
        assert_eq!(bd[0].node_id, "n1");
        assert_eq!(bd[1].node_id, "n2");
    }
}
