//! Gate verdicts, router selections, and router feedback outcomes.
//!
//! These are the three "output" types of the verification and routing verbs:
//!
//! - [`Verdict`] — a [`Gate`](crate::Gate) passed or failed a signal
//! - [`Selection`] — a [`Router`](crate::Router) chose one candidate
//! - [`Outcome`] — feedback about what happened after a selection was acted on

use crate::ContentHash;
use serde::{Deserialize, Serialize};

/// Structured test counts from a test gate (passed/failed/ignored).
///
/// Matches Mori's `TestCount` in `apps/mori/src/orchestrator/gates.rs`. Used
/// by policies to classify "mostly passing" failures (see
/// [`Verdict::is_mostly_passing`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestCount {
    /// Tests that passed.
    pub passed: u32,
    /// Tests that failed.
    pub failed: u32,
    /// Tests that were ignored/skipped.
    pub ignored: u32,
}

impl TestCount {
    /// Construct a new `TestCount`.
    #[must_use]
    pub const fn new(passed: u32, failed: u32, ignored: u32) -> Self {
        Self { passed, failed, ignored }
    }

    /// Total tests seen (passed + failed + ignored).
    #[must_use]
    pub const fn total(&self) -> u32 {
        self.passed + self.failed + self.ignored
    }
}

/// The result of a [`Gate`](crate::Gate) verifying a signal.
///
/// Verdicts include evidence — why did the gate pass or fail? — so downstream
/// policies can make intelligent decisions (retry with different input,
/// escalate to a human, etc.).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    /// Did the signal pass the gate?
    pub passed: bool,
    /// Human-readable reason (used for logs, error messages).
    pub reason: String,
    /// Identifier of the gate that rendered this verdict.
    pub gate: String,
    /// Numeric score in `[0..1]` — useful for thresholding (e.g. judge gates).
    pub score: f32,
    /// Optional detail string (stdout, error output, diagnostic).
    pub detail: Option<String>,
    /// Structured test counts (populated by test gates).
    pub test_count: Option<TestCount>,
    /// Structured error digest for feeding back to agents — unique errors
    /// with file/line info, not raw output. See
    /// `apps/mori/src/orchestrator/gates.rs::extract_error_digest`.
    pub error_digest: Option<String>,
    /// Wall-clock duration the gate took, in milliseconds.
    pub duration_ms: u64,
}

impl Verdict {
    /// A passing verdict.
    #[must_use]
    pub fn pass(gate: impl Into<String>) -> Self {
        Self {
            passed: true,
            reason: String::new(),
            gate: gate.into(),
            score: 1.0,
            detail: None,
            test_count: None,
            error_digest: None,
            duration_ms: 0,
        }
    }

    /// A failing verdict with a reason.
    #[must_use]
    pub fn fail(gate: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            passed: false,
            reason: reason.into(),
            gate: gate.into(),
            score: 0.0,
            detail: None,
            test_count: None,
            error_digest: None,
            duration_ms: 0,
        }
    }

    /// Override the verdict's numeric score (clamped to `[0..1]`).
    #[must_use]
    pub const fn with_score(mut self, score: f32) -> Self {
        self.score = score.clamp(0.0, 1.0);
        self
    }

    /// Attach a detail string (stdout, diagnostic).
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Record how long the gate took (milliseconds).
    #[must_use]
    pub const fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }

    /// Attach structured test counts (populated by test gates).
    #[must_use]
    pub const fn with_test_count(mut self, tc: TestCount) -> Self {
        self.test_count = Some(tc);
        self
    }

    /// Attach a structured error digest (unique errors with file/line refs).
    #[must_use]
    pub fn with_error_digest(mut self, digest: impl Into<String>) -> Self {
        self.error_digest = Some(digest.into());
        self
    }

    /// True when tests mostly pass (>90% pass rate, >20 total, ≥1 failure).
    ///
    /// Only meaningful for failed test gates. A passing gate returns false
    /// because there is nothing to classify. Mirrors Mori's
    /// `GateResult::is_mostly_passing`.
    #[must_use]
    pub fn is_mostly_passing(&self) -> bool {
        if self.passed {
            return false;
        }
        let Some(tc) = self.test_count.as_ref() else {
            return false;
        };
        let total = tc.total();
        total > 20 && tc.failed > 0 && (f64::from(tc.passed) / f64::from(total)) > 0.9
    }
}

/// The result of a [`Router`](crate::Router) picking one signal from candidates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    /// The content hash of the chosen signal.
    pub chosen: ContentHash,
    /// Router's confidence in this choice, `[0..1]`.
    pub confidence: f32,
    /// Identifier of the router that made this selection.
    pub router: String,
    /// Optional reasoning trace (for debugging/observability).
    pub reasoning: Option<String>,
}

impl Selection {
    /// Construct a selection with full confidence.
    #[must_use]
    pub fn new(chosen: ContentHash, router: impl Into<String>) -> Self {
        Self {
            chosen,
            confidence: 1.0,
            router: router.into(),
            reasoning: None,
        }
    }

    /// Set the router's confidence in this selection (clamped to `[0..1]`).
    #[must_use]
    pub const fn with_confidence(mut self, c: f32) -> Self {
        self.confidence = c.clamp(0.0, 1.0);
        self
    }

    /// Attach a reasoning trace (for observability).
    #[must_use]
    pub fn with_reasoning(mut self, r: impl Into<String>) -> Self {
        self.reasoning = Some(r.into());
        self
    }
}

/// Feedback about a prior router selection — did acting on it work out?
///
/// Routers use outcomes to learn (bandit algorithms, ELO updates, etc.).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    /// Which selection does this outcome evaluate?
    pub selection: Selection,
    /// Did the downstream action succeed?
    pub success: bool,
    /// Numeric reward signal, typically `[0..1]` or `[-1..1]`.
    pub reward: f32,
    /// Optional cost incurred (for cost-aware bandits).
    pub cost: Option<f32>,
    /// Optional latency in milliseconds.
    pub latency_ms: Option<u64>,
}

impl Outcome {
    /// A successful outcome with full reward (1.0).
    #[must_use]
    pub const fn success(selection: Selection) -> Self {
        Self {
            selection,
            success: true,
            reward: 1.0,
            cost: None,
            latency_ms: None,
        }
    }

    /// A failure outcome with zero reward.
    #[must_use]
    pub const fn failure(selection: Selection) -> Self {
        Self {
            selection,
            success: false,
            reward: 0.0,
            cost: None,
            latency_ms: None,
        }
    }

    /// Set the reward value (scalar feedback for bandit learning).
    #[must_use]
    pub const fn with_reward(mut self, r: f32) -> Self {
        self.reward = r;
        self
    }

    /// Attach a cost metric (for cost-aware bandits).
    #[must_use]
    pub const fn with_cost(mut self, c: f32) -> Self {
        self.cost = Some(c);
        self
    }

    /// Attach observed latency in milliseconds.
    #[must_use]
    pub const fn with_latency(mut self, ms: u64) -> Self {
        self.latency_ms = Some(ms);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ContentHash;

    #[test]
    fn pass_verdict_has_score_1() {
        let v = Verdict::pass("test_gate");
        assert!(v.passed);
        assert_eq!(v.score, 1.0);
    }

    #[test]
    fn fail_verdict_captures_reason() {
        let v = Verdict::fail("compile", "undefined symbol");
        assert!(!v.passed);
        assert_eq!(v.reason, "undefined symbol");
        assert_eq!(v.score, 0.0);
    }

    #[test]
    fn verdict_builders_chain() {
        let v = Verdict::fail("lint", "style")
            .with_score(0.3)
            .with_detail("lots of warnings")
            .with_duration(1500);
        assert_eq!(v.score, 0.3);
        assert_eq!(v.duration_ms, 1500);
        assert_eq!(v.detail.as_deref(), Some("lots of warnings"));
    }

    #[test]
    fn selection_clamps_confidence() {
        let h = ContentHash::of(b"x");
        let s = Selection::new(h, "r").with_confidence(1.5);
        assert_eq!(s.confidence, 1.0);
    }

    #[test]
    fn outcome_success_full_reward() {
        let h = ContentHash::of(b"x");
        let sel = Selection::new(h, "r");
        let o = Outcome::success(sel);
        assert!(o.success);
        assert_eq!(o.reward, 1.0);
    }

    #[test]
    fn test_count_total_sums_fields() {
        let tc = TestCount::new(10, 2, 3);
        assert_eq!(tc.total(), 15);
    }

    #[test]
    fn verdict_records_test_count_and_digest() {
        let v = Verdict::fail("test", "one failed")
            .with_test_count(TestCount::new(40, 1, 2))
            .with_error_digest("E0599: no method `foo` on type `Bar`");
        assert_eq!(v.test_count, Some(TestCount::new(40, 1, 2)));
        assert_eq!(v.error_digest.as_deref(), Some("E0599: no method `foo` on type `Bar`"));
    }

    #[test]
    fn is_mostly_passing_true_above_threshold() {
        let v = Verdict::fail("test", "some failed")
            .with_test_count(TestCount::new(95, 4, 1));
        // 100 total, 4 failed, 95/100 = 95% pass → mostly passing
        assert!(v.is_mostly_passing());
    }

    #[test]
    fn is_mostly_passing_false_when_few_tests() {
        let v = Verdict::fail("test", "some failed")
            .with_test_count(TestCount::new(18, 1, 0));
        // Total ≤ 20 → not mostly passing
        assert!(!v.is_mostly_passing());
    }

    #[test]
    fn is_mostly_passing_false_when_no_failures() {
        // Contract: a failed verdict with zero failures is degenerate;
        // only counts that show ≥1 failure are candidates.
        let v = Verdict::fail("test", "spurious failure")
            .with_test_count(TestCount::new(100, 0, 5));
        assert!(!v.is_mostly_passing());
    }

    #[test]
    fn is_mostly_passing_false_on_passing_verdict() {
        let v = Verdict::pass("test").with_test_count(TestCount::new(100, 0, 0));
        assert!(!v.is_mostly_passing());
    }

    #[test]
    fn outcome_captures_cost_latency() {
        let h = ContentHash::of(b"x");
        let sel = Selection::new(h, "r");
        let o = Outcome::success(sel)
            .with_reward(0.8)
            .with_cost(0.05)
            .with_latency(200);
        assert_eq!(o.reward, 0.8);
        assert_eq!(o.cost, Some(0.05));
        assert_eq!(o.latency_ms, Some(200));
    }
}
