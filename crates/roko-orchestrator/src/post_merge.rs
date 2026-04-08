//! Post-merge regression testing (§14.8).
//!
//! After a plan branch is merged into the batch branch, the
//! [`PostMergeRunner`] runs gate checks (compile, test, clippy, etc.)
//! against the merged state and produces a [`PostMergeResult`]
//! indicating whether the merge introduced regressions.
//!
//! If regressions are detected, the result carries the list of failing
//! tests so the conductor can decide whether to revert or retry.

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::Mutex;
use roko_core::Verdict;
use serde::{Deserialize, Serialize};

// ─── PostMergeResult ───────────────────────────────────────────────────

/// Outcome of a post-merge regression check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PostMergeResult {
    /// All gates passed — no regression introduced.
    Clean,
    /// One or more gates failed after merging.
    RegressionDetected {
        /// Identifiers or descriptions of failing tests/gates.
        failing_tests: Vec<String>,
        /// Whether the merge was automatically reverted.
        reverted: bool,
    },
}

impl PostMergeResult {
    /// Whether this result represents a clean merge.
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        matches!(self, Self::Clean)
    }

    /// Whether a regression was detected.
    #[must_use]
    pub const fn is_regression(&self) -> bool {
        matches!(self, Self::RegressionDetected { .. })
    }
}

// ─── PostMergeCheck ────────────────────────────────────────────────────

/// Record of a post-merge check run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostMergeCheck {
    /// Plan that was merged.
    pub plan_id: String,
    /// Unix millisecond timestamp of when the merge completed.
    pub merged_at_ms: i64,
    /// Result of the post-merge regression check.
    pub result: PostMergeResult,
}

/// Actionable follow-up produced from a post-merge check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostMergeFollowUp {
    /// Plan this follow-up belongs to.
    pub plan_id: String,
    /// Whether the merged branch should be reverted automatically.
    pub should_revert: bool,
    /// Human-readable failing gate/test descriptions.
    pub failing_tests: Vec<String>,
}

impl PostMergeFollowUp {
    /// Create a follow-up from a result payload.
    #[must_use]
    pub fn from_result(plan_id: &str, result: &PostMergeResult) -> Self {
        match result {
            PostMergeResult::Clean => Self {
                plan_id: plan_id.to_string(),
                should_revert: false,
                failing_tests: Vec::new(),
            },
            PostMergeResult::RegressionDetected { failing_tests, .. } => Self {
                plan_id: plan_id.to_string(),
                should_revert: true,
                failing_tests: failing_tests.clone(),
            },
        }
    }

    /// Whether the follow-up requests a revert.
    #[must_use]
    pub const fn needs_revert(&self) -> bool {
        self.should_revert
    }
}

// ─── PostMergeRunner ───────────────────────────────────────────────────

/// Runs post-merge regression checks and maintains a history of results.
///
/// The runner does not execute actual shell commands — it evaluates
/// gate [`Verdict`]s provided by the caller (the orchestrator runs
/// the gates, the runner interprets the results).
#[derive(Debug, Clone, Default)]
pub struct PostMergeRunner {
    /// History of post-merge checks, keyed by `plan_id`.
    history: Arc<Mutex<BTreeMap<String, PostMergeCheck>>>,
}

impl PostMergeRunner {
    /// Create a new runner with empty history.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluate gate results after a merge and produce a
    /// [`PostMergeResult`].
    ///
    /// The caller provides the set of gate verdicts that were run
    /// against the merged worktree. If any verdict failed, a
    /// [`PostMergeResult::RegressionDetected`] is returned. The
    /// `reverted` flag is initially `false` — the caller should set
    /// it if an automatic revert is performed.
    #[must_use]
    pub fn check_regression(gate_results: &[Verdict]) -> PostMergeResult {
        let failing: Vec<String> = gate_results
            .iter()
            .filter(|v| !v.passed)
            .map(|v| {
                if v.reason.is_empty() {
                    format!("{} (score={:.2})", v.gate, v.score)
                } else {
                    format!("{}: {}", v.gate, v.reason)
                }
            })
            .collect();

        if failing.is_empty() {
            PostMergeResult::Clean
        } else {
            PostMergeResult::RegressionDetected {
                failing_tests: failing,
                reverted: false,
            }
        }
    }

    /// Run regression check and record the result in history.
    ///
    /// Returns the [`PostMergeCheck`] that was recorded.
    pub fn run_and_record(
        &self,
        plan_id: &str,
        merged_at_ms: i64,
        gate_results: &[Verdict],
    ) -> PostMergeCheck {
        let result = Self::check_regression(gate_results);
        let check = PostMergeCheck {
            plan_id: plan_id.to_string(),
            merged_at_ms,
            result,
        };
        self.history
            .lock()
            .insert(plan_id.to_string(), check.clone());
        check
    }

    /// Run regression checks, persist history, and return the immediate
    /// orchestration follow-up decision.
    pub fn run_record_and_follow_up(
        &self,
        plan_id: &str,
        merged_at_ms: i64,
        gate_results: &[Verdict],
    ) -> (PostMergeCheck, PostMergeFollowUp) {
        let check = self.run_and_record(plan_id, merged_at_ms, gate_results);
        let follow_up = PostMergeFollowUp::from_result(plan_id, &check.result);
        (check, follow_up)
    }

    /// Look up the most recent check for a given plan.
    #[must_use]
    pub fn get_check(&self, plan_id: &str) -> Option<PostMergeCheck> {
        self.history.lock().get(plan_id).cloned()
    }

    /// Build a follow-up decision from the latest stored check for a plan.
    #[must_use]
    pub fn follow_up_for(&self, plan_id: &str) -> Option<PostMergeFollowUp> {
        self.get_check(plan_id)
            .map(|check| PostMergeFollowUp::from_result(&check.plan_id, &check.result))
    }

    /// Number of checks in history.
    #[must_use]
    pub fn history_len(&self) -> usize {
        self.history.lock().len()
    }

    /// Return all checks that detected regressions.
    #[must_use]
    pub fn regressions(&self) -> Vec<PostMergeCheck> {
        self.history
            .lock()
            .values()
            .filter(|c| c.result.is_regression())
            .cloned()
            .collect()
    }

    /// Return all clean checks.
    #[must_use]
    pub fn clean_merges(&self) -> Vec<PostMergeCheck> {
        self.history
            .lock()
            .values()
            .filter(|c| c.result.is_clean())
            .cloned()
            .collect()
    }

    /// Mark a previously detected regression as reverted.
    ///
    /// Returns `true` if the check was found and updated.
    #[allow(clippy::significant_drop_tightening)]
    pub fn mark_reverted(&self, plan_id: &str) -> bool {
        let mut guard = self.history.lock();
        let Some(check) = guard.get_mut(plan_id) else {
            return false;
        };
        match &mut check.result {
            PostMergeResult::RegressionDetected { reverted, .. } => {
                *reverted = true;
                true
            }
            PostMergeResult::Clean => false,
        }
    }

    /// Return the plan ids that currently have unresolved regressions.
    #[must_use]
    pub fn unresolved_regressions(&self) -> Vec<String> {
        self.history
            .lock()
            .iter()
            .filter_map(|(plan_id, check)| match &check.result {
                PostMergeResult::RegressionDetected { reverted, .. } if !reverted => {
                    Some(plan_id.clone())
                }
                _ => None,
            })
            .collect()
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use roko_core::{TestCount, Verdict};

    // ── 1. All gates pass -> Clean ───────────────────────────────────

    #[test]
    fn all_gates_pass_is_clean() {
        let verdicts = vec![
            Verdict::pass("compile"),
            Verdict::pass("test"),
            Verdict::pass("clippy"),
        ];
        let result = PostMergeRunner::check_regression(&verdicts);
        assert!(result.is_clean());
        assert!(!result.is_regression());
    }

    // ── 2. One failing gate -> RegressionDetected ────────────────────

    #[test]
    fn one_failure_is_regression() {
        let verdicts = vec![
            Verdict::pass("compile"),
            Verdict::fail("test", "3 tests failed"),
            Verdict::pass("clippy"),
        ];
        let result = PostMergeRunner::check_regression(&verdicts);
        assert!(result.is_regression());
        match &result {
            PostMergeResult::RegressionDetected {
                failing_tests,
                reverted,
            } => {
                assert_eq!(failing_tests.len(), 1);
                assert!(failing_tests[0].contains("test"));
                assert!(failing_tests[0].contains("3 tests failed"));
                assert!(!reverted);
            }
            PostMergeResult::Clean => panic!("expected regression"),
        }
    }

    // ── 3. Multiple failures listed ──────────────────────────────────

    #[test]
    fn multiple_failures_listed() {
        let verdicts = vec![
            Verdict::fail("compile", "syntax error"),
            Verdict::fail("test", "assertion failed"),
        ];
        let result = PostMergeRunner::check_regression(&verdicts);
        match &result {
            PostMergeResult::RegressionDetected { failing_tests, .. } => {
                assert_eq!(failing_tests.len(), 2);
            }
            PostMergeResult::Clean => panic!("expected regression"),
        }
    }

    // ── 4. Empty verdicts -> Clean ───────────────────────────────────

    #[test]
    fn empty_verdicts_is_clean() {
        let result = PostMergeRunner::check_regression(&[]);
        assert!(result.is_clean());
    }

    // ── 5. run_and_record stores history ─────────────────────────────

    #[test]
    fn run_and_record_stores_history() {
        let runner = PostMergeRunner::new();
        let verdicts = vec![Verdict::pass("test")];
        let check = runner.run_and_record("plan-a", 1_000_000, &verdicts);
        assert!(check.result.is_clean());
        assert_eq!(check.plan_id, "plan-a");
        assert_eq!(check.merged_at_ms, 1_000_000);

        assert_eq!(runner.history_len(), 1);
        let stored = runner.get_check("plan-a").unwrap();
        assert_eq!(stored, check);
    }

    // ── 6. regressions() filters correctly ───────────────────────────

    #[test]
    fn regressions_filter() {
        let runner = PostMergeRunner::new();
        runner.run_and_record("clean-plan", 100, &[Verdict::pass("test")]);
        runner.run_and_record("bad-plan", 200, &[Verdict::fail("test", "boom")]);

        let regs = runner.regressions();
        assert_eq!(regs.len(), 1);
        assert_eq!(regs[0].plan_id, "bad-plan");

        let clean = runner.clean_merges();
        assert_eq!(clean.len(), 1);
        assert_eq!(clean[0].plan_id, "clean-plan");
    }

    // ── 7. mark_reverted flips the flag ──────────────────────────────

    #[test]
    fn mark_reverted_flips_flag() {
        let runner = PostMergeRunner::new();
        runner.run_and_record("bad-plan", 200, &[Verdict::fail("test", "boom")]);
        assert!(runner.mark_reverted("bad-plan"));

        let check = runner.get_check("bad-plan").unwrap();
        match &check.result {
            PostMergeResult::RegressionDetected { reverted, .. } => {
                assert!(reverted);
            }
            PostMergeResult::Clean => panic!("expected regression"),
        }
    }

    // ── 8. mark_reverted returns false for unknown ───────────────────

    #[test]
    fn mark_reverted_unknown_returns_false() {
        let runner = PostMergeRunner::new();
        assert!(!runner.mark_reverted("nonexistent"));
    }

    // ── 9. mark_reverted returns false for clean merge ───────────────

    #[test]
    fn mark_reverted_clean_returns_false() {
        let runner = PostMergeRunner::new();
        runner.run_and_record("clean-plan", 100, &[Verdict::pass("test")]);
        assert!(!runner.mark_reverted("clean-plan"));
    }

    // ── 10. Verdict with empty reason formats with score ─────────────

    #[test]
    fn verdict_empty_reason_formats_with_score() {
        let mut v = Verdict::fail("lint", "");
        v.reason = String::new();
        let verdicts = vec![v];
        let result = PostMergeRunner::check_regression(&verdicts);
        match &result {
            PostMergeResult::RegressionDetected { failing_tests, .. } => {
                assert!(failing_tests[0].contains("lint"));
                assert!(failing_tests[0].contains("score="));
            }
            PostMergeResult::Clean => panic!("expected regression"),
        }
    }

    // ── 11. PostMergeCheck with test counts ──────────────────────────

    #[test]
    fn regression_with_test_counts() {
        let v = Verdict::fail("test", "regression").with_test_count(TestCount::new(90, 5, 2));
        let verdicts = vec![Verdict::pass("compile"), v];
        let result = PostMergeRunner::check_regression(&verdicts);
        assert!(result.is_regression());
        match &result {
            PostMergeResult::RegressionDetected { failing_tests, .. } => {
                assert_eq!(failing_tests.len(), 1);
                assert!(failing_tests[0].contains("regression"));
            }
            PostMergeResult::Clean => panic!("expected regression"),
        }
    }

    // ── 12. History overwrite on re-record ────────────────────────────

    #[test]
    fn history_overwrite_on_rerecord() {
        let runner = PostMergeRunner::new();
        runner.run_and_record("plan-a", 100, &[Verdict::fail("test", "first")]);
        assert!(runner.get_check("plan-a").unwrap().result.is_regression());

        // Re-run after fix -> clean.
        runner.run_and_record("plan-a", 200, &[Verdict::pass("test")]);
        assert!(runner.get_check("plan-a").unwrap().result.is_clean());
        // History should still have 1 entry, not 2.
        assert_eq!(runner.history_len(), 1);
    }

    #[test]
    fn follow_up_from_result_matches_clean_and_regression() {
        let clean = PostMergeFollowUp::from_result("p-clean", &PostMergeResult::Clean);
        assert!(!clean.needs_revert());
        assert!(clean.failing_tests.is_empty());

        let regression = PostMergeFollowUp::from_result(
            "p-bad",
            &PostMergeResult::RegressionDetected {
                failing_tests: vec!["test: boom".to_string()],
                reverted: false,
            },
        );
        assert!(regression.needs_revert());
        assert_eq!(regression.failing_tests, vec!["test: boom".to_string()]);
    }

    #[test]
    fn run_record_and_follow_up_returns_matching_decision() {
        let runner = PostMergeRunner::new();
        let (_check, follow_up) =
            runner.run_record_and_follow_up("plan-x", 1_000, &[Verdict::fail("test", "oops")]);
        assert!(follow_up.needs_revert());
        assert_eq!(follow_up.plan_id, "plan-x");
        assert!(follow_up.failing_tests[0].contains("oops"));
    }

    #[test]
    fn follow_up_for_uses_latest_recorded_check() {
        let runner = PostMergeRunner::new();
        runner.run_and_record("plan-y", 100, &[Verdict::fail("test", "first")]);
        runner.run_and_record("plan-y", 200, &[Verdict::pass("test")]);
        let follow_up = runner.follow_up_for("plan-y").unwrap();
        assert!(!follow_up.needs_revert());
        assert!(follow_up.failing_tests.is_empty());
    }

    #[test]
    fn unresolved_regressions_excludes_reverted_entries() {
        let runner = PostMergeRunner::new();
        runner.run_and_record("bad-a", 100, &[Verdict::fail("test", "A")]);
        runner.run_and_record("bad-b", 200, &[Verdict::fail("test", "B")]);
        runner.run_and_record("clean-c", 300, &[Verdict::pass("test")]);

        let unresolved = runner.unresolved_regressions();
        assert_eq!(unresolved.len(), 2);
        assert!(unresolved.contains(&"bad-a".to_string()));
        assert!(unresolved.contains(&"bad-b".to_string()));

        assert!(runner.mark_reverted("bad-a"));
        let unresolved = runner.unresolved_regressions();
        assert_eq!(unresolved, vec!["bad-b".to_string()]);
    }
}
