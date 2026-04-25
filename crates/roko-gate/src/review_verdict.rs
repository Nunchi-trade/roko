//! Structured review verdict types for the gate pipeline and reviewer agents.
//!
//! Enriches the raw [`Verdict`] with structured issue classification
//! and actionable suggestions that agents can consume programmatically.

use crate::acceptance_contract::{AcceptanceOutcome, RequiredNextAction, ReviewVerdictEvidence};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

/// Metadata supplied by the caller when parsing a reviewer agent response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewVerdictContext {
    /// Stable verdict id.
    pub verdict_id: String,
    /// Batch or task family id.
    pub batch_id: String,
    /// Task id.
    pub task_id: String,
    /// Reviewer role/profile id.
    pub reviewer_role_id: String,
    /// Raw reviewer output path, signal id, or artifact id.
    pub raw_output_ref: String,
    /// Creation timestamp.
    pub created_at: String,
}

/// Where a reviewer verdict was parsed from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewParseSource {
    /// Entire response was valid JSON.
    Json,
    /// A fenced `json` code block was valid JSON.
    JsonCodeBlock,
    /// A fenced `toml` code block was valid TOML.
    TomlCodeBlock,
    /// Parsing or validation failed; the returned verdict is fail-closed.
    FailClosed,
}

/// Structured parser result that keeps the raw reviewer text for audit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParsedReviewVerdict {
    /// Typed evidence consumed by acceptance contracts and orchestration.
    pub evidence: ReviewVerdictEvidence,
    /// Raw reviewer output preserved for audit/debugging.
    pub raw_output: String,
    /// Parser source used to produce `evidence`.
    pub source: ReviewParseSource,
    /// Parse or validation error when fail-closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
}

impl ParsedReviewVerdict {
    /// True only when the parsed verdict unambiguously approves the review.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.evidence.status == AcceptanceOutcome::Passed
            && self.evidence.confidence.is_finite()
            && (0.0..=1.0).contains(&self.evidence.confidence)
            && self.evidence.blocking_findings.is_empty()
            && self.evidence.required_next_action == RequiredNextAction::None
            && self.parse_error.is_none()
    }
}

#[derive(Debug, Deserialize)]
struct AgentReviewPayload {
    #[serde(
        alias = "verdict",
        alias = "decision",
        deserialize_with = "deserialize_status"
    )]
    status: AcceptanceOutcome,
    confidence: f32,
    blocking_findings: Vec<String>,
    non_blocking_findings: Vec<String>,
    required_next_action: RequiredNextAction,
    evidence_refs: Vec<String>,
}

/// Parse reviewer agent output into typed evidence.
///
/// The parser accepts, in order, a full JSON object, a fenced `json` code block,
/// or a fenced `toml` code block. Any missing, ambiguous, contradictory, or
/// unparsable verdict returns a structured `needs_human` result instead of
/// approving free text.
#[must_use]
pub fn parse_structured_review_verdict(
    output: &str,
    ctx: ReviewVerdictContext,
) -> ParsedReviewVerdict {
    match parse_agent_payload(output) {
        Ok((payload, source)) => evidence_from_payload(payload, output, ctx, source),
        Err(error) => fail_closed(output, ctx, error),
    }
}

fn parse_agent_payload(output: &str) -> Result<(AgentReviewPayload, ReviewParseSource), String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Err("review output is empty".to_string());
    }

    if let Ok(payload) = serde_json::from_str::<AgentReviewPayload>(trimmed) {
        return Ok((payload, ReviewParseSource::Json));
    }

    if let Some(block) = extract_code_block(trimmed, "json") {
        let payload = serde_json::from_str::<AgentReviewPayload>(&block)
            .map_err(|error| format!("json code block did not match review schema: {error}"))?;
        return Ok((payload, ReviewParseSource::JsonCodeBlock));
    }

    if let Some(block) = extract_code_block(trimmed, "toml") {
        let payload = toml::from_str::<AgentReviewPayload>(&block)
            .map_err(|error| format!("toml code block did not match review schema: {error}"))?;
        return Ok((payload, ReviewParseSource::TomlCodeBlock));
    }

    Err("review output did not contain a structured JSON or TOML verdict".to_string())
}

fn evidence_from_payload(
    payload: AgentReviewPayload,
    output: &str,
    ctx: ReviewVerdictContext,
    source: ReviewParseSource,
) -> ParsedReviewVerdict {
    let mut evidence = ReviewVerdictEvidence {
        verdict_id: ctx.verdict_id,
        batch_id: ctx.batch_id,
        task_id: ctx.task_id,
        reviewer_role_id: ctx.reviewer_role_id,
        status: payload.status,
        confidence: payload.confidence,
        blocking_findings: payload.blocking_findings,
        non_blocking_findings: payload.non_blocking_findings,
        required_next_action: payload.required_next_action,
        evidence_refs: payload.evidence_refs,
        raw_output_ref: ctx.raw_output_ref,
        created_at: ctx.created_at,
    };

    if let Some(error) = validate_evidence_semantics(&evidence) {
        evidence.status = AcceptanceOutcome::NeedsHuman;
        evidence.required_next_action = RequiredNextAction::Human;
        evidence.blocking_findings.push(error.clone());
        return ParsedReviewVerdict {
            evidence,
            raw_output: output.to_string(),
            source: ReviewParseSource::FailClosed,
            parse_error: Some(error),
        };
    }

    ParsedReviewVerdict {
        evidence,
        raw_output: output.to_string(),
        source,
        parse_error: None,
    }
}

fn fail_closed(output: &str, ctx: ReviewVerdictContext, error: String) -> ParsedReviewVerdict {
    let raw_excerpt = output
        .split_whitespace()
        .take(80)
        .collect::<Vec<_>>()
        .join(" ");
    let mut blocking_findings = vec![format!("structured review verdict parse failed: {error}")];
    if !raw_excerpt.is_empty() {
        blocking_findings.push(format!("raw review excerpt: {raw_excerpt}"));
    }

    ParsedReviewVerdict {
        evidence: ReviewVerdictEvidence {
            verdict_id: ctx.verdict_id,
            batch_id: ctx.batch_id,
            task_id: ctx.task_id,
            reviewer_role_id: ctx.reviewer_role_id,
            status: AcceptanceOutcome::NeedsHuman,
            confidence: 0.0,
            blocking_findings,
            non_blocking_findings: Vec::new(),
            required_next_action: RequiredNextAction::Human,
            evidence_refs: Vec::new(),
            raw_output_ref: ctx.raw_output_ref,
            created_at: ctx.created_at,
        },
        raw_output: output.to_string(),
        source: ReviewParseSource::FailClosed,
        parse_error: Some(error),
    }
}

fn validate_evidence_semantics(evidence: &ReviewVerdictEvidence) -> Option<String> {
    if !evidence.confidence.is_finite() || !(0.0..=1.0).contains(&evidence.confidence) {
        return Some("review confidence must be in 0.0..=1.0".to_string());
    }

    if evidence.status == AcceptanceOutcome::Passed {
        if !evidence.blocking_findings.is_empty() {
            return Some("passed review contains blocking findings".to_string());
        }
        if evidence.required_next_action != RequiredNextAction::None {
            return Some("passed review requires a follow-up action".to_string());
        }
    }

    if evidence.status != AcceptanceOutcome::Passed
        && evidence.required_next_action == RequiredNextAction::None
    {
        return Some("non-passing review omitted required next action".to_string());
    }

    None
}

fn deserialize_status<'de, D>(deserializer: D) -> Result<AcceptanceOutcome, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let Some(raw) = value.as_str() else {
        return Err(serde::de::Error::custom("status must be a string"));
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "pass" | "passed" | "approve" | "approved" => Ok(AcceptanceOutcome::Passed),
        "fail" | "failed" | "revise" | "reject" | "rejected" => Ok(AcceptanceOutcome::Failed),
        "blocked" => Ok(AcceptanceOutcome::Blocked),
        "timed_out" | "timeout" => Ok(AcceptanceOutcome::TimedOut),
        "cancelled" | "canceled" => Ok(AcceptanceOutcome::Cancelled),
        "needs_retry" | "retry" => Ok(AcceptanceOutcome::NeedsRetry),
        "needs_replan" | "replan" => Ok(AcceptanceOutcome::NeedsReplan),
        "needs_human" | "human" | "needs-human" => Ok(AcceptanceOutcome::NeedsHuman),
        other => Err(serde::de::Error::custom(format!(
            "unsupported review status '{other}'"
        ))),
    }
}

fn extract_code_block(output: &str, language: &str) -> Option<String> {
    let fence = format!("```{language}");
    let start = output.find(&fence)?;
    let after_fence = output[start + fence.len()..].strip_prefix('\r').unwrap_or(
        output[start + fence.len()..]
            .strip_prefix('\n')
            .unwrap_or(&output[start + fence.len()..]),
    );
    let after_fence = after_fence.strip_prefix('\n').unwrap_or(after_fence);
    let end = after_fence.find("```")?;
    Some(after_fence[..end].trim().to_string())
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

    fn parse_ctx() -> ReviewVerdictContext {
        ReviewVerdictContext {
            verdict_id: "verdict-1".to_string(),
            batch_id: "RT01".to_string(),
            task_id: "task-1".to_string(),
            reviewer_role_id: "quick-reviewer".to_string(),
            raw_output_ref: ".roko/runs/review.raw".to_string(),
            created_at: "2026-04-25T12:43:56Z".to_string(),
        }
    }

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

    #[test]
    fn parses_valid_json_reviewer_verdict() {
        let raw = r#"{
            "status": "passed",
            "confidence": 0.91,
            "blocking_findings": [],
            "non_blocking_findings": ["small naming nit"],
            "required_next_action": "none",
            "evidence_refs": ["crates/roko-gate/src/review_verdict.rs"]
        }"#;

        let parsed = parse_structured_review_verdict(raw, parse_ctx());

        assert!(parsed.passed(), "{parsed:?}");
        assert_eq!(parsed.source, ReviewParseSource::Json);
        assert_eq!(parsed.evidence.confidence, 0.91);
        assert_eq!(parsed.evidence.raw_output_ref, ".roko/runs/review.raw");
        assert_eq!(parsed.raw_output, raw);
    }

    #[test]
    fn parses_valid_json_code_block() {
        let raw = r#"Here is the verdict:
```json
{
  "verdict": "approve",
  "confidence": 0.82,
  "blocking_findings": [],
  "non_blocking_findings": [],
  "required_next_action": "none",
  "evidence_refs": ["artifact://diff"]
}
```"#;

        let parsed = parse_structured_review_verdict(raw, parse_ctx());

        assert!(parsed.passed(), "{parsed:?}");
        assert_eq!(parsed.source, ReviewParseSource::JsonCodeBlock);
    }

    #[test]
    fn parses_valid_toml_code_block() {
        let raw = r#"```toml
status = "failed"
confidence = 0.7
blocking_findings = ["missing test coverage"]
non_blocking_findings = []
required_next_action = "retry"
evidence_refs = ["artifact://review"]
```"#;

        let parsed = parse_structured_review_verdict(raw, parse_ctx());

        assert!(!parsed.passed());
        assert_eq!(parsed.source, ReviewParseSource::TomlCodeBlock);
        assert_eq!(parsed.evidence.status, AcceptanceOutcome::Failed);
        assert_eq!(
            parsed.evidence.required_next_action,
            RequiredNextAction::Retry
        );
    }

    #[test]
    fn invalid_free_text_fails_closed() {
        let parsed = parse_structured_review_verdict("LGTM, looks good to me", parse_ctx());

        assert!(!parsed.passed());
        assert_eq!(parsed.source, ReviewParseSource::FailClosed);
        assert_eq!(parsed.evidence.status, AcceptanceOutcome::NeedsHuman);
        assert_eq!(
            parsed.evidence.required_next_action,
            RequiredNextAction::Human
        );
        assert!(parsed.parse_error.is_some());
    }

    #[test]
    fn missing_required_fields_fail_closed() {
        let raw = r#"{"status":"passed","blocking_findings":[]}"#;

        let parsed = parse_structured_review_verdict(raw, parse_ctx());

        assert!(!parsed.passed());
        assert_eq!(parsed.evidence.status, AcceptanceOutcome::NeedsHuman);
        assert!(
            parsed
                .evidence
                .blocking_findings
                .iter()
                .any(|finding| finding.contains("parse failed"))
        );
    }

    #[test]
    fn contradictory_passed_verdict_fails_closed() {
        let raw = r#"{
            "status": "passed",
            "confidence": 0.95,
            "blocking_findings": ["compile still fails"],
            "non_blocking_findings": [],
            "required_next_action": "none",
            "evidence_refs": []
        }"#;

        let parsed = parse_structured_review_verdict(raw, parse_ctx());

        assert!(!parsed.passed());
        assert_eq!(parsed.source, ReviewParseSource::FailClosed);
        assert_eq!(parsed.evidence.status, AcceptanceOutcome::NeedsHuman);
        assert!(
            parsed
                .evidence
                .blocking_findings
                .iter()
                .any(|finding| finding.contains("blocking findings"))
        );
    }
}
