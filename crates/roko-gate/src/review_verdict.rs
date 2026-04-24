//! Structured review verdict types for the gate pipeline.
//!
//! Enriches the raw [`Verdict`] with structured issue classification
//! and actionable suggestions that agents can consume programmatically.

use serde::{Deserialize, Serialize};

/// High-level review decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    /// All gates passed — safe to merge.
    Approve,
    /// Some gates failed — needs rework with feedback.
    Revise,
    /// Skipped (e.g. not applicable, deferred).
    Skip,
}

/// Category of issue found during gate evaluation.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCategory {
    /// Code does not compile.
    CompileError,
    /// Tests fail.
    TestFailure,
    /// Lint violations (clippy, eslint, etc.).
    LintViolation,
    /// Missing or incomplete implementation (vacuous impl).
    IncompleteImpl,
    /// Security vulnerability detected.
    SecurityIssue,
    /// Performance regression detected.
    PerformanceRegression,
    /// Formatting issues.
    FormatViolation,
    /// Symbol or API contract violation.
    SymbolMissing,
    /// Integration test failure.
    IntegrationFailure,
    /// Review required by human (LLM judge flagged).
    NeedsHumanReview,
}

/// A single issue found during review.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewIssue {
    /// Issue category.
    pub category: IssueCategory,
    /// Which gate found this issue.
    pub gate: String,
    /// Which rung (0-6) this came from.
    pub rung: u8,
    /// Human-readable description.
    pub message: String,
    /// File path if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Line number if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Actionable suggestion for the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// Whether this issue is blocking (must fix vs. should fix).
    pub blocking: bool,
}

/// A structured review verdict produced from the gate pipeline.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewVerdict {
    /// Overall decision.
    pub decision: ReviewDecision,
    /// Summary line for humans.
    pub summary: String,
    /// All issues found.
    pub issues: Vec<ReviewIssue>,
    /// Blocking issue count.
    pub blocking_count: usize,
    /// Non-blocking issue count.
    pub advisory_count: usize,
    /// Per-rung pass/fail results.
    pub rung_results: Vec<RungResult>,
}

/// Pass/fail result for a single rung.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RungResult {
    /// Rung index (0-6).
    pub rung: u8,
    /// Rung name.
    pub name: String,
    /// Whether this rung passed.
    pub passed: bool,
    /// Score (0.0-1.0).
    pub score: f32,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

impl ReviewVerdict {
    /// Build a verdict from a set of gate verdicts.
    pub fn from_verdicts(verdicts: &[(u8, &str, &roko_core::Verdict)]) -> Self {
        let mut issues = Vec::new();
        let mut rung_results = Vec::new();

        for &(rung, name, verdict) in verdicts {
            rung_results.push(RungResult {
                rung,
                name: name.to_string(),
                passed: verdict.passed,
                score: verdict.score,
                duration_ms: verdict.duration_ms,
            });

            if !verdict.passed {
                let category = match rung {
                    0 => IssueCategory::CompileError,
                    1 => IssueCategory::LintViolation,
                    2 | 4 | 5 => IssueCategory::TestFailure,
                    3 => IssueCategory::SymbolMissing,
                    _ => IssueCategory::NeedsHumanReview,
                };

                issues.push(ReviewIssue {
                    category,
                    gate: name.to_string(),
                    rung,
                    message: verdict.reason.clone(),
                    file: None,
                    line: None,
                    suggestion: verdict.detail.as_ref().and_then(|d| {
                        // Extract first suggestion-like line from detail.
                        d.lines()
                            .find(|l| {
                                let t = l.trim_start();
                                t.starts_with("help:") || t.starts_with("suggestion:")
                            })
                            .map(|l| l.trim().to_string())
                    }),
                    blocking: rung <= 2, // compile, lint, test are blocking
                });
            }
        }

        let blocking_count = issues.iter().filter(|i| i.blocking).count();
        let advisory_count = issues.iter().filter(|i| !i.blocking).count();

        let decision = if blocking_count == 0 && advisory_count == 0 {
            ReviewDecision::Approve
        } else if blocking_count == 0 {
            // Only non-blocking issues — still approve with warnings.
            ReviewDecision::Approve
        } else {
            ReviewDecision::Revise
        };

        let summary = if decision == ReviewDecision::Approve {
            if advisory_count > 0 {
                format!(
                    "Approved with {advisory_count} advisory issue{}",
                    if advisory_count == 1 { "" } else { "s" }
                )
            } else {
                "All gates passed".to_string()
            }
        } else {
            format!(
                "{blocking_count} blocking issue{} found",
                if blocking_count == 1 { "" } else { "s" }
            )
        };

        Self {
            decision,
            summary,
            issues,
            blocking_count,
            advisory_count,
            rung_results,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::Verdict;

    #[test]
    fn all_passing_verdicts_approve() {
        let compile = Verdict::pass("compile");
        let test = Verdict::pass("test");
        let verdicts = vec![(0, "compile", &compile), (2, "test", &test)];
        let review = ReviewVerdict::from_verdicts(&verdicts);

        assert_eq!(review.decision, ReviewDecision::Approve);
        assert_eq!(review.blocking_count, 0);
        assert!(review.issues.is_empty());
    }

    #[test]
    fn compile_failure_requires_revise() {
        let compile = Verdict::fail("compile", "error[E0433]: unresolved import");
        let verdicts = vec![(0, "compile", &compile)];
        let review = ReviewVerdict::from_verdicts(&verdicts);

        assert_eq!(review.decision, ReviewDecision::Revise);
        assert_eq!(review.blocking_count, 1);
        assert_eq!(review.issues[0].category, IssueCategory::CompileError);
        assert!(review.issues[0].blocking);
    }

    #[test]
    fn non_blocking_issues_still_approve() {
        let compile = Verdict::pass("compile");
        let judge = Verdict::fail("llm_judge", "code quality could improve");
        let verdicts = vec![(0, "compile", &compile), (6, "llm_judge", &judge)];
        let review = ReviewVerdict::from_verdicts(&verdicts);

        assert_eq!(review.decision, ReviewDecision::Approve);
        assert_eq!(review.advisory_count, 1);
        assert!(!review.issues[0].blocking);
    }

    #[test]
    fn rung_results_capture_scores() {
        let compile = Verdict::pass("compile").with_score(1.0).with_duration(42);
        let verdicts = vec![(0, "compile", &compile)];
        let review = ReviewVerdict::from_verdicts(&verdicts);

        assert_eq!(review.rung_results.len(), 1);
        assert_eq!(review.rung_results[0].score, 1.0);
        assert_eq!(review.rung_results[0].duration_ms, 42);
    }
}
