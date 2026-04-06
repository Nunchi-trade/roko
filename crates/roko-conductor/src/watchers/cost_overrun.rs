//! Cost overrun watcher: fires when plan cost exceeds budget.
//!
//! Monitors `Metric` signals tagged `name=plan_cost` and compares
//! against the budget set via `name=plan_budget`. Fires when cost
//! exceeds budget.

use roko_core::{Body, Context, Kind, Policy, Signal};

/// Tag key marking signals from this watcher.
pub const WATCHER_NAME: &str = "cost-overrun";

/// Tag key identifying the metric name on `Metric` signals.
pub const METRIC_NAME_TAG: &str = "name";
/// Metric name for accumulated plan cost.
pub const PLAN_COST_METRIC: &str = "plan_cost";
/// Metric name for plan budget.
pub const PLAN_BUDGET_METRIC: &str = "plan_budget";
/// Tag key for the numeric value.
pub const METRIC_VALUE_TAG: &str = "value";

/// Default budget when no budget metric is found (USD).
pub const DEFAULT_BUDGET: f64 = 10.0;

/// Fires when accumulated plan cost exceeds the plan budget.
///
/// Looks for the most recent `plan_cost` and `plan_budget` metric
/// signals. If cost exceeds budget, fires a warning.
#[derive(Debug, Clone)]
pub struct CostOverrunWatcher {
    /// Fallback budget if no budget metric exists.
    default_budget: f64,
}

impl Default for CostOverrunWatcher {
    fn default() -> Self {
        Self {
            default_budget: DEFAULT_BUDGET,
        }
    }
}

impl CostOverrunWatcher {
    /// Create with a custom default budget.
    #[must_use]
    pub const fn new(default_budget: f64) -> Self {
        Self { default_budget }
    }
}

/// Find the most recent metric value by name.
fn latest_metric(stream: &[Signal], name: &str) -> Option<f64> {
    stream
        .iter()
        .rev()
        .find(|s| s.kind == Kind::Metric && s.tag(METRIC_NAME_TAG) == Some(name))
        .and_then(|s| s.tag(METRIC_VALUE_TAG))
        .and_then(|v| v.parse().ok())
}

impl Policy for CostOverrunWatcher {
    fn decide(&self, stream: &[Signal], _ctx: &Context) -> Vec<Signal> {
        let Some(cost) = latest_metric(stream, PLAN_COST_METRIC) else {
            return Vec::new();
        };

        let budget = latest_metric(stream, PLAN_BUDGET_METRIC)
            .unwrap_or(self.default_budget);

        if budget <= 0.0 {
            return Vec::new();
        }

        if cost > budget {
            vec![Signal::builder(Kind::Custom(
                "conductor.intervention".into(),
            ))
            .body(Body::text(format!(
                "plan cost ${cost:.2} exceeds budget ${budget:.2}"
            )))
            .tag("watcher", WATCHER_NAME)
            .tag("severity", "warning")
            .tag("cost", format!("{cost:.2}"))
            .tag("budget", format!("{budget:.2}"))
            .build()]
        } else {
            Vec::new()
        }
    }

    fn name(&self) -> &str {
        WATCHER_NAME
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost_signal(cost: f64) -> Signal {
        Signal::builder(Kind::Metric)
            .body(Body::text("cost"))
            .tag(METRIC_NAME_TAG, PLAN_COST_METRIC)
            .tag(METRIC_VALUE_TAG, &format!("{cost}"))
            .build()
    }

    fn budget_signal(budget: f64) -> Signal {
        Signal::builder(Kind::Metric)
            .body(Body::text("budget"))
            .tag(METRIC_NAME_TAG, PLAN_BUDGET_METRIC)
            .tag(METRIC_VALUE_TAG, &format!("{budget}"))
            .build()
    }

    #[test]
    fn empty_stream_no_fire() {
        let w = CostOverrunWatcher::default();
        assert!(w.decide(&[], &Context::at(0)).is_empty());
    }

    #[test]
    fn no_cost_signal_no_fire() {
        let w = CostOverrunWatcher::default();
        let stream = vec![budget_signal(100.0)];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn below_budget_no_fire() {
        let w = CostOverrunWatcher::default();
        let stream = vec![budget_signal(20.0), cost_signal(15.0)];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn above_budget_fires() {
        let w = CostOverrunWatcher::default();
        let stream = vec![budget_signal(10.0), cost_signal(12.0)];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
    }

    #[test]
    fn uses_default_budget_when_no_budget_signal() {
        let w = CostOverrunWatcher::new(5.0);
        let stream = vec![cost_signal(6.0)]; // 6 > 5 default
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn below_default_budget_no_fire() {
        let w = CostOverrunWatcher::new(5.0);
        let stream = vec![cost_signal(3.0)]; // 3 < 5 default
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn uses_most_recent_cost() {
        let w = CostOverrunWatcher::default();
        let stream = vec![
            budget_signal(10.0),
            cost_signal(15.0), // over budget
            cost_signal(5.0),  // most recent — under budget
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }
}
