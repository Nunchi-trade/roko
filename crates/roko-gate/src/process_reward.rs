//! Process Reward Model — step-level verification for agent trajectories.
//!
//! Two cybernetic signals derived from gate verdicts at each agent turn:
//!
//! - **Promise**: predicts probability of eventual task success given the
//!   current trajectory (ratchet progression rate, pass history, diff trends).
//! - **Progress**: measures trajectory delta between turns (rung advancement,
//!   error reduction, coverage increase).
//!
//! Together they enable early termination (low Promise -> abandon) and
//! intervention (stalling Progress -> change model/strategy).
//!
//! References: Lightman et al. 2023 (PRM800K), AgentPRM (arXiv:2502.10325).

use roko_core::Verdict;
use serde::{Deserialize, Serialize};

/// A snapshot of the gate pipeline state after a single agent turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurnSnapshot {
    /// Highest rung reached at this turn.
    pub rung: u32,
    /// All verdicts produced by the gate pipeline at this turn.
    pub verdicts: Vec<Verdict>,
    /// Number of distinct errors reported by gate feedback.
    pub error_count: u32,
    /// Number of lines changed in the diff at this turn.
    pub diff_lines: u32,
}

/// Aggregate method for combining per-step scores into a final verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AggregateMethod {
    /// Worst step determines the aggregate score.
    Min,
    /// Average of all step scores.
    Mean,
    /// Later steps receive higher weight (linearly increasing).
    Weighted,
}

impl Default for AggregateMethod {
    fn default() -> Self {
        Self::Mean
    }
}

/// Process Reward Model — tracks per-turn gate snapshots and derives
/// Promise/Progress signals for the orchestrator.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProcessRewardModel {
    /// Ordered history of turn snapshots (oldest first).
    pub history: Vec<TurnSnapshot>,
    /// Minimum per-step score for heuristic validation.
    pub step_threshold: f64,
    /// How to aggregate per-step scores.
    pub aggregate: AggregateMethod,
}

impl ProcessRewardModel {
    /// Create a new PRM with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            step_threshold: 0.5,
            aggregate: AggregateMethod::Mean,
        }
    }

    /// Create a PRM with custom threshold and aggregation method.
    #[must_use]
    pub fn with_config(step_threshold: f64, aggregate: AggregateMethod) -> Self {
        Self {
            history: Vec::new(),
            step_threshold,
            aggregate,
        }
    }

    /// Record a new turn snapshot.
    pub fn record_turn(&mut self, snapshot: TurnSnapshot) {
        self.history.push(snapshot);
    }

    /// Promise score: predicts probability of eventual task success.
    ///
    /// Computed from:
    /// 1. Ratchet progression rate (are we advancing through rungs?)
    /// 2. Historical pass rate (fraction of verdicts that passed)
    /// 3. Diff size trend (are diffs getting smaller, suggesting convergence?)
    ///
    /// Returns a value in `[0.0, 1.0]`.
    #[must_use]
    pub fn promise(&self) -> f64 {
        if self.history.is_empty() {
            return 0.5; // no data => neutral prior
        }

        let pass_rate = self.historical_pass_rate();
        let progression_rate = self.ratchet_progression_rate();
        let convergence = self.diff_convergence();

        // Weighted combination: pass rate is most important, progression
        // shows forward movement, convergence shows we are narrowing.
        let raw = pass_rate * 0.5 + progression_rate * 0.3 + convergence * 0.2;
        raw.clamp(0.0, 1.0)
    }

    /// Progress score: trajectory delta between the two most recent turns.
    ///
    /// Positive means improvement, negative means regression.
    /// Returns a value in `[-1.0, 1.0]`.
    #[must_use]
    pub fn progress(&self) -> f64 {
        if self.history.len() < 2 {
            return 0.0;
        }

        let prev = &self.history[self.history.len() - 2];
        let curr = &self.history[self.history.len() - 1];

        let mut delta = 0.0;

        // Rung advancement: +0.4 per rung gained, -0.4 per rung lost.
        let rung_delta = curr.rung as f64 - prev.rung as f64;
        delta += (rung_delta * 0.4).clamp(-0.4, 0.4);

        // Error reduction: going from 5 errors to 0 is good.
        if prev.error_count > 0 {
            let error_reduction =
                (prev.error_count as f64 - curr.error_count as f64) / prev.error_count as f64;
            delta += error_reduction * 0.3;
        } else if curr.error_count == 0 {
            delta += 0.3; // still clean
        }

        // Pass rate improvement between turns.
        let prev_pass_rate = turn_pass_rate(prev);
        let curr_pass_rate = turn_pass_rate(curr);
        delta += (curr_pass_rate - prev_pass_rate) * 0.3;

        delta.clamp(-1.0, 1.0)
    }

    /// Whether the task should be terminated early due to low promise.
    #[must_use]
    pub fn should_terminate(&self, min_promise: f64) -> bool {
        // Need at least 2 turns of data before deciding to terminate.
        if self.history.len() < 2 {
            return false;
        }
        self.promise() < min_promise
    }

    /// Verify a sequence of reasoning steps using heuristic scoring.
    ///
    /// Each step is scored based on: length (non-trivial content), presence
    /// of code blocks, and presence of assertions/checks. This is a stub
    /// for future LLM-based scoring.
    #[must_use]
    pub fn verify_steps(&self, steps: &[ReasoningStep]) -> StepVerdict {
        if steps.is_empty() {
            return StepVerdict {
                passed: false,
                step_scores: Vec::new(),
                aggregate_score: 0.0,
            };
        }

        let step_scores: Vec<f64> = steps.iter().map(|s| score_step(s)).collect();
        let aggregate_score = match self.aggregate {
            AggregateMethod::Min => step_scores.iter().copied().fold(f64::INFINITY, f64::min),
            AggregateMethod::Mean => step_scores.iter().sum::<f64>() / step_scores.len() as f64,
            AggregateMethod::Weighted => {
                let n = step_scores.len() as f64;
                let total_weight = n * (n + 1.0) / 2.0;
                step_scores
                    .iter()
                    .enumerate()
                    .map(|(i, &s)| s * (i as f64 + 1.0))
                    .sum::<f64>()
                    / total_weight
            }
        };

        StepVerdict {
            passed: aggregate_score >= self.step_threshold,
            step_scores,
            aggregate_score,
        }
    }

    // ── Private helpers ──────────────────────────────────────────────────

    fn historical_pass_rate(&self) -> f64 {
        let mut total = 0u64;
        let mut passed = 0u64;
        for snap in &self.history {
            for v in &snap.verdicts {
                total += 1;
                if v.passed {
                    passed += 1;
                }
            }
        }
        if total == 0 {
            return 0.5;
        }
        passed as f64 / total as f64
    }

    fn ratchet_progression_rate(&self) -> f64 {
        if self.history.len() < 2 {
            return 0.5;
        }
        let first_rung = self.history.first().map_or(0, |s| s.rung);
        let last_rung = self.history.last().map_or(0, |s| s.rung);
        let max_rung = self
            .history
            .iter()
            .map(|s| s.rung)
            .max()
            .unwrap_or(1)
            .max(1);

        // Normalize progression against the highest rung seen.
        let delta = last_rung as f64 - first_rung as f64;
        (0.5 + delta / (2.0 * max_rung as f64)).clamp(0.0, 1.0)
    }

    fn diff_convergence(&self) -> f64 {
        if self.history.len() < 2 {
            return 0.5;
        }
        let first_diff = self.history.first().map_or(1, |s| s.diff_lines).max(1);
        let last_diff = self.history.last().map_or(1, |s| s.diff_lines).max(1);

        // Shrinking diffs suggest convergence.
        if last_diff <= first_diff {
            let ratio = last_diff as f64 / first_diff as f64;
            // ratio 1.0 => 0.5, ratio 0.0 => 1.0
            0.5 + (1.0 - ratio) * 0.5
        } else {
            // Growing diffs suggest divergence.
            let ratio = first_diff as f64 / last_diff as f64;
            ratio * 0.5
        }
    }
}

/// A single reasoning step to be scored by the PRM.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReasoningStep {
    /// The text content of this reasoning step.
    pub content: String,
}

/// Result of verifying a sequence of reasoning steps.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepVerdict {
    /// Whether the overall sequence passed verification.
    pub passed: bool,
    /// Per-step scores in `[0.0, 1.0]`.
    pub step_scores: Vec<f64>,
    /// Aggregate score after combining per-step scores.
    pub aggregate_score: f64,
}

fn turn_pass_rate(snap: &TurnSnapshot) -> f64 {
    if snap.verdicts.is_empty() {
        return 0.0;
    }
    let passed = snap.verdicts.iter().filter(|v| v.passed).count();
    passed as f64 / snap.verdicts.len() as f64
}

/// Heuristic step scorer (stub for future LLM-based PRM).
///
/// Scores based on:
/// - Content length (non-trivial steps score higher)
/// - Presence of code blocks (```...```)
/// - Presence of assertion keywords (assert, verify, check, test)
fn score_step(step: &ReasoningStep) -> f64 {
    let content = &step.content;
    let mut score: f64 = 0.0;

    // Length score: 0-50 chars = 0.1, 50-200 = 0.3, 200+ = 0.4
    let len = content.len();
    score += if len > 200 {
        0.4
    } else if len > 50 {
        0.3
    } else if len > 10 {
        0.1
    } else {
        0.0
    };

    // Code block presence.
    if content.contains("```") || content.contains("    ") {
        score += 0.3;
    }

    // Assertion/verification keywords.
    let lower = content.to_ascii_lowercase();
    let check_words = ["assert", "verify", "check", "test", "ensure", "confirm"];
    if check_words.iter().any(|w| lower.contains(w)) {
        score += 0.3;
    }

    score.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::Verdict;

    fn pass_verdict() -> Verdict {
        Verdict::pass("compile")
    }

    fn fail_verdict() -> Verdict {
        Verdict::fail("compile", "error[E0308]: mismatched types")
    }

    fn snapshot(rung: u32, verdicts: Vec<Verdict>, errors: u32, diff: u32) -> TurnSnapshot {
        TurnSnapshot {
            rung,
            verdicts,
            error_count: errors,
            diff_lines: diff,
        }
    }

    #[test]
    fn promise_neutral_when_empty() {
        let prm = ProcessRewardModel::new();
        assert!((prm.promise() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn promise_increases_with_passing_verdicts() {
        let mut prm = ProcessRewardModel::new();
        prm.record_turn(snapshot(1, vec![pass_verdict(), pass_verdict()], 0, 100));
        prm.record_turn(snapshot(2, vec![pass_verdict(), pass_verdict()], 0, 50));

        assert!(prm.promise() > 0.7);
    }

    #[test]
    fn promise_decreases_with_failing_verdicts() {
        let mut prm = ProcessRewardModel::new();
        prm.record_turn(snapshot(1, vec![fail_verdict(), fail_verdict()], 5, 100));
        prm.record_turn(snapshot(1, vec![fail_verdict(), fail_verdict()], 8, 200));

        assert!(prm.promise() < 0.4);
    }

    #[test]
    fn progress_positive_on_improvement() {
        let mut prm = ProcessRewardModel::new();
        prm.record_turn(snapshot(1, vec![fail_verdict()], 5, 100));
        prm.record_turn(snapshot(3, vec![pass_verdict()], 0, 50));

        assert!(prm.progress() > 0.0);
    }

    #[test]
    fn progress_negative_on_regression() {
        let mut prm = ProcessRewardModel::new();
        prm.record_turn(snapshot(3, vec![pass_verdict()], 0, 50));
        prm.record_turn(snapshot(1, vec![fail_verdict()], 5, 200));

        assert!(prm.progress() < 0.0);
    }

    #[test]
    fn should_terminate_requires_min_turns() {
        let mut prm = ProcessRewardModel::new();
        prm.record_turn(snapshot(0, vec![fail_verdict()], 10, 500));
        // Only 1 turn — should not terminate regardless of promise.
        assert!(!prm.should_terminate(0.9));
    }

    #[test]
    fn should_terminate_on_low_promise() {
        let mut prm = ProcessRewardModel::new();
        prm.record_turn(snapshot(0, vec![fail_verdict(), fail_verdict()], 10, 200));
        prm.record_turn(snapshot(0, vec![fail_verdict(), fail_verdict()], 12, 300));

        // All failures, no progression, growing diffs => low promise.
        assert!(prm.should_terminate(0.4));
    }

    #[test]
    fn verify_steps_min_aggregate() {
        let prm = ProcessRewardModel::with_config(0.5, AggregateMethod::Min);
        let steps = vec![
            ReasoningStep {
                content: "First I'll check the error output and verify the assertion holds."
                    .to_string(),
            },
            ReasoningStep {
                content: "ok".to_string(), // too short
            },
        ];
        let result = prm.verify_steps(&steps);
        // The short step should drag the min down.
        assert!(!result.passed);
        assert_eq!(result.step_scores.len(), 2);
    }

    #[test]
    fn verify_steps_mean_aggregate() {
        let prm = ProcessRewardModel::with_config(0.3, AggregateMethod::Mean);
        let steps = vec![
            ReasoningStep {
                content: "Reading the compile output to check for type errors. The assert macro should verify correctness.".to_string(),
            },
            ReasoningStep {
                content: "```rust\nfn main() { assert!(true); }\n```".to_string(),
            },
        ];
        let result = prm.verify_steps(&steps);
        assert!(result.passed);
        assert!(result.aggregate_score >= 0.3);
    }

    #[test]
    fn verify_steps_weighted_favors_later() {
        let prm = ProcessRewardModel::with_config(0.3, AggregateMethod::Weighted);
        let steps = vec![
            ReasoningStep {
                content: "x".to_string(), // poor first step
            },
            ReasoningStep {
                content: "Now I will verify correctness with ```assert_eq!(a, b)``` and ensure all tests pass.".to_string(),
            },
        ];
        let result = prm.verify_steps(&steps);
        // Weighted should give more weight to the better second step.
        let mean_prm = ProcessRewardModel::with_config(0.3, AggregateMethod::Mean);
        let mean_result = mean_prm.verify_steps(&steps);
        assert!(result.aggregate_score >= mean_result.aggregate_score);
    }

    #[test]
    fn verify_steps_empty() {
        let prm = ProcessRewardModel::new();
        let result = prm.verify_steps(&[]);
        assert!(!result.passed);
        assert_eq!(result.aggregate_score, 0.0);
    }
}
