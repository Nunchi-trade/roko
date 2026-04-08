//! Test failure budget watcher: fires when pass rate drops below threshold.
//!
//! Monitors `TestResult` signals for pass/fail counts and fires when the
//! overall pass rate drops below [`MIN_PASS_RATE`].

use roko_core::{Body, Context, Kind, Policy, Signal};

/// Minimum acceptable test pass rate (0.0 to 1.0).
pub const MIN_PASS_RATE: f64 = 0.90;

/// Tag key marking signals from this watcher.
pub const WATCHER_NAME: &str = "test-failure-budget";

/// Tag key on test-result signals indicating pass/fail.
pub const TEST_OUTCOME_TAG: &str = "outcome";
/// Tag value for passed tests.
pub const TEST_PASSED: &str = "pass";
/// Tag value for failed tests.
pub const TEST_FAILED: &str = "fail";

/// Fires when the test pass rate drops below [`MIN_PASS_RATE`].
///
/// Examines all `TestResult` signals in the stream, counts passes vs
/// failures, and fires if the pass rate is below the threshold.
#[derive(Debug, Clone)]
pub struct TestFailureBudgetWatcher {
    /// Minimum pass rate before firing.
    min_pass_rate: f64,
}

impl Default for TestFailureBudgetWatcher {
    fn default() -> Self {
        Self {
            min_pass_rate: MIN_PASS_RATE,
        }
    }
}

impl TestFailureBudgetWatcher {
    /// Create with a custom pass rate threshold.
    #[must_use]
    pub const fn new(min_pass_rate: f64) -> Self {
        Self { min_pass_rate }
    }
}

impl Policy for TestFailureBudgetWatcher {
    fn decide(&self, stream: &[Signal], _ctx: &Context) -> Vec<Signal> {
        let mut passed = 0u64;
        let mut failed = 0u64;

        for s in stream {
            if s.kind != Kind::TestResult {
                continue;
            }
            match s.tag(TEST_OUTCOME_TAG) {
                Some(TEST_PASSED) => passed += 1,
                Some(TEST_FAILED) => failed += 1,
                _ => {}
            }
        }

        let total = passed + failed;
        if total == 0 {
            return Vec::new();
        }

        #[allow(clippy::cast_precision_loss)]
        let pass_rate = passed as f64 / total as f64;

        if pass_rate < self.min_pass_rate {
            vec![
                Signal::builder(Kind::Custom("conductor.intervention".into()))
                    .body(Body::text(format!(
                        "test pass rate {pass_rate:.1}% ({passed}/{total}) below threshold {:.0}%",
                        self.min_pass_rate * 100.0
                    )))
                    .tag("watcher", WATCHER_NAME)
                    .tag("severity", "warning")
                    .tag("pass_rate", format!("{pass_rate:.3}"))
                    .build(),
            ]
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

    fn test_signal(outcome: &str) -> Signal {
        Signal::builder(Kind::TestResult)
            .body(Body::text("test output"))
            .tag(TEST_OUTCOME_TAG, outcome)
            .build()
    }

    #[test]
    fn empty_stream_no_fire() {
        let w = TestFailureBudgetWatcher::default();
        assert!(w.decide(&[], &Context::at(0)).is_empty());
    }

    #[test]
    fn all_pass_no_fire() {
        let w = TestFailureBudgetWatcher::default();
        let stream: Vec<Signal> = (0..10).map(|_| test_signal(TEST_PASSED)).collect();
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn above_threshold_no_fire() {
        let w = TestFailureBudgetWatcher::default();
        // 9 pass, 1 fail = 90% — exactly at threshold
        let mut stream: Vec<Signal> = (0..9).map(|_| test_signal(TEST_PASSED)).collect();
        stream.push(test_signal(TEST_FAILED));
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn below_threshold_fires() {
        let w = TestFailureBudgetWatcher::default();
        // 8 pass, 2 fail = 80% < 90%
        let mut stream: Vec<Signal> = (0..8).map(|_| test_signal(TEST_PASSED)).collect();
        stream.push(test_signal(TEST_FAILED));
        stream.push(test_signal(TEST_FAILED));
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
    }

    #[test]
    fn all_fail_fires() {
        let w = TestFailureBudgetWatcher::default();
        let stream = vec![test_signal(TEST_FAILED), test_signal(TEST_FAILED)];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn non_test_signals_ignored() {
        let w = TestFailureBudgetWatcher::default();
        let stream = vec![
            Signal::builder(Kind::AgentOutput)
                .body(Body::text("hi"))
                .tag(TEST_OUTCOME_TAG, TEST_FAILED)
                .build(),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn custom_threshold() {
        let w = TestFailureBudgetWatcher::new(0.50);
        // 4 pass, 6 fail = 40% < 50%
        let mut stream: Vec<Signal> = (0..4).map(|_| test_signal(TEST_PASSED)).collect();
        for _ in 0..6 {
            stream.push(test_signal(TEST_FAILED));
        }
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }
}
