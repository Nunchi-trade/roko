//! Test failure budget watcher: fires when test failures increase beyond
//! the baseline observed earlier in the signal stream.
//!
//! Monitors test gate verdict signals for structured failure counts and
//! emits an intervention when the latest failure count exceeds the
//! baseline failure count.

use std::collections::HashMap;

use roko_core::{Body, Context, Kind, Policy, Signal};

/// Minimum increase in test failures required to fire.
pub const MIN_FAILURE_INCREASE: u32 = 1;

/// Tag key marking signals from this watcher.
pub const WATCHER_NAME: &str = "test-failure-budget";

/// JSON field containing structured test counts on conductor gate signals.
pub const TEST_COUNT_FIELD: &str = "test_count";
/// JSON field containing the plan identifier.
pub const PLAN_ID_FIELD: &str = "plan_id";
/// JSON field containing the gate name.
pub const GATE_FIELD: &str = "gate";
/// JSON field containing the number of failed tests.
pub const FAILED_FIELD: &str = "failed";

/// Fires when the latest test failure count exceeds the baseline.
///
/// The watcher scans the stream for test gate verdicts, remembers the
/// earliest failure count it sees for each plan, and compares the latest
/// count against that baseline.
#[derive(Debug, Clone)]
pub struct TestFailureBudgetWatcher {
    /// Minimum failure increase before firing.
    min_failure_increase: u32,
}

impl Default for TestFailureBudgetWatcher {
    fn default() -> Self {
        Self {
            min_failure_increase: MIN_FAILURE_INCREASE,
        }
    }
}

impl TestFailureBudgetWatcher {
    /// Create with a custom failure increase threshold.
    #[must_use]
    pub const fn new(min_failure_increase: u32) -> Self {
        Self {
            min_failure_increase,
        }
    }
}

impl Policy for TestFailureBudgetWatcher {
    fn decide(&self, stream: &[Signal], _ctx: &Context) -> Vec<Signal> {
        let mut baselines: HashMap<String, u32> = HashMap::new();
        let mut latest: HashMap<String, u32> = HashMap::new();

        for s in stream {
            if s.kind != Kind::GateVerdict {
                continue;
            }

            let Ok(body) = s.body.as_json::<serde_json::Value>() else {
                continue;
            };
            let Some(test_count) = body.get(TEST_COUNT_FIELD).and_then(|v| v.as_object()) else {
                continue;
            };
            let Some(plan_id) = body.get(PLAN_ID_FIELD).and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(failed) = test_count
                .get(FAILED_FIELD)
                .and_then(|v| v.as_u64())
                .and_then(|n| u32::try_from(n).ok())
            else {
                continue;
            };

            let plan_id = plan_id.to_owned();
            baselines.entry(plan_id.clone()).or_insert(failed);
            latest.insert(plan_id, failed);
        }

        let mut signals = Vec::new();
        for (plan_id, current_failed) in latest {
            let baseline_failed = match baselines.get(&plan_id).copied() {
                Some(baseline) => baseline,
                None => continue,
            };
            if current_failed.saturating_sub(baseline_failed) < self.min_failure_increase {
                continue;
            }

            let delta = current_failed.saturating_sub(baseline_failed);
            signals.push(
                Signal::builder(Kind::Custom("conductor.intervention".into()))
                    .body(Body::text(format!(
                        "test failures increased for {plan_id}: {baseline_failed} -> {current_failed}"
                    )))
                    .tag("watcher", WATCHER_NAME)
                    .tag("severity", "warning")
                    .tag("plan_id", plan_id)
                    .tag("baseline_failures", baseline_failed.to_string())
                    .tag("current_failures", current_failed.to_string())
                    .tag("failure_delta", delta.to_string())
                    .build(),
            );
        }

        signals
    }

    fn name(&self) -> &str {
        WATCHER_NAME
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_signal(plan_id: &str, failed: u32) -> Signal {
        Signal::builder(Kind::GateVerdict)
            .body(Body::Json(serde_json::json!({
                PLAN_ID_FIELD: plan_id,
                GATE_FIELD: "test",
                TEST_COUNT_FIELD: {
                    "passed": 10u32.saturating_sub(failed),
                    "failed": failed,
                    "ignored": 0,
                    "total": 10,
                }
            })))
            .build()
    }

    #[test]
    fn empty_stream_no_fire() {
        let w = TestFailureBudgetWatcher::default();
        assert!(w.decide(&[], &Context::at(0)).is_empty());
    }

    #[test]
    fn single_test_result_no_fire() {
        let w = TestFailureBudgetWatcher::default();
        let stream = vec![test_signal("plan-1", 1)];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn unchanged_failure_count_no_fire() {
        let w = TestFailureBudgetWatcher::default();
        let stream = vec![test_signal("plan-1", 2), test_signal("plan-1", 2)];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn increased_failure_count_fires() {
        let w = TestFailureBudgetWatcher::default();
        let stream = vec![test_signal("plan-1", 1), test_signal("plan-1", 3)];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
        assert_eq!(out[0].tag("plan_id"), Some("plan-1"));
        assert_eq!(out[0].tag("baseline_failures"), Some("1"));
        assert_eq!(out[0].tag("current_failures"), Some("3"));
    }

    #[test]
    fn multiple_plans_independent() {
        let w = TestFailureBudgetWatcher::default();
        let stream = vec![
            test_signal("plan-a", 0),
            test_signal("plan-b", 1),
            test_signal("plan-a", 2),
            test_signal("plan-b", 1),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("plan_id"), Some("plan-a"));
    }

    #[test]
    fn custom_threshold_requires_larger_increase() {
        let w = TestFailureBudgetWatcher::new(3);
        let stream = vec![test_signal("plan-1", 1), test_signal("plan-1", 3)];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn custom_threshold_fires_when_met() {
        let w = TestFailureBudgetWatcher::new(2);
        let stream = vec![test_signal("plan-1", 1), test_signal("plan-1", 3)];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }
}
