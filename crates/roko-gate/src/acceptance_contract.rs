//! Typed done-gate contract for self-hosting Roko tasks.
//!
//! The contract is intentionally narrow: it describes the evidence a task must
//! produce before it can be marked done. Missing or malformed evidence is a
//! blocking validation issue, so callers fail closed instead of treating absent
//! data as success.

use serde::{Deserialize, Serialize};

/// Terminal and actionable states for a done-gate decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceOutcome {
    /// All required evidence is present and passing.
    Passed,
    /// Required evidence failed or is malformed.
    Failed,
    /// Work cannot proceed with the current external state.
    Blocked,
    /// A required gate exceeded its time budget.
    TimedOut,
    /// The run was cancelled before a terminal verdict.
    Cancelled,
    /// Evidence points to a bounded retry of the same task.
    NeedsRetry,
    /// Evidence points to changing the plan before retrying.
    NeedsReplan,
    /// Evidence requires human review or approval.
    NeedsHuman,
    /// Required evidence is incomplete and the task needs more implementation work.
    NeedsWork,
}

/// A typed acceptance contract for one self-hosting task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceContract {
    /// Contract schema version. Only version 1 is accepted.
    pub version: u32,
    /// Compile/test/lint or custom commands that must produce gate evidence.
    #[serde(default)]
    pub gates: Vec<GateRequirement>,
    /// Requirement that production paths are not satisfied by stubs/noops.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_stub: Option<NoStubRequirement>,
    /// Requirement for structurally parseable agent output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_output: Option<StructuredAgentOutputRequirement>,
    /// Requirement for a structured reviewer verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_verdict: Option<ReviewVerdictRequirement>,
    /// Requirement to record retry/reflection/replan signals after failures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<RecoveryRequirement>,
    /// Requirement to attach doc parity evidence rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parity_ledger: Option<ParityLedgerRequirement>,
}

impl AcceptanceContract {
    /// Validate contract shape before any evidence is evaluated.
    #[must_use]
    pub fn validate_contract(&self) -> AcceptanceDecision {
        let mut issues = Vec::new();

        if self.version != 1 {
            issues.push(AcceptanceIssue::blocking(
                "ACCEPT_001",
                format!("unsupported acceptance contract version {}", self.version),
            ));
        }

        for gate in &self.gates {
            if gate.id.trim().is_empty() {
                issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_002",
                    "gate requirement is missing id",
                ));
            }
            if gate.command.as_deref().is_none_or(str::is_empty) {
                issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_003",
                    format!("gate '{}' is missing command", gate.id),
                ));
            }
        }

        if let Some(no_stub) = &self.no_stub
            && no_stub.required
            && no_stub.production_paths.is_empty()
        {
            issues.push(AcceptanceIssue::blocking(
                "ACCEPT_004",
                "no-stub requirement must name at least one production path",
            ));
        }

        if let Some(agent_output) = &self.agent_output
            && agent_output.required
            && agent_output.schema.trim().is_empty()
        {
            issues.push(AcceptanceIssue::blocking(
                "ACCEPT_005",
                "structured agent output requirement is missing schema",
            ));
        }

        if let Some(review) = &self.review_verdict {
            if review.required && review.reviewer_role_id.trim().is_empty() {
                issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_006",
                    "review verdict requirement is missing reviewer_role_id",
                ));
            }
            if !(0.0..=1.0).contains(&review.min_confidence) {
                issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_007",
                    "review verdict min_confidence must be in 0.0..=1.0",
                ));
            }
        }

        if let Some(recovery) = &self.recovery
            && recovery.required
            && !recovery.retry
            && !recovery.reflection
            && !recovery.replan
        {
            issues.push(AcceptanceIssue::blocking(
                "ACCEPT_008",
                "recovery requirement must require retry, reflection, or replan evidence",
            ));
        }

        if let Some(parity) = &self.parity_ledger
            && parity.required
            && parity.rows.is_empty()
        {
            issues.push(AcceptanceIssue::blocking(
                "ACCEPT_009",
                "parity ledger requirement must declare at least one row",
            ));
        }

        for row in self
            .parity_ledger
            .as_ref()
            .into_iter()
            .flat_map(|parity| &parity.rows)
        {
            if row.requirement_id.trim().is_empty() {
                issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_010",
                    "parity ledger row is missing requirement_id",
                ));
            }
            if row.evidence_ref.trim().is_empty()
                && row
                    .implementation_refs
                    .iter()
                    .all(|value| value.trim().is_empty())
            {
                issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_011",
                    format!(
                        "parity ledger row '{}' is missing implementation evidence",
                        row.requirement_id
                    ),
                ));
            }
            if row.source_ref.trim().is_empty() {
                issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_012",
                    format!(
                        "parity ledger row '{}' is missing source_ref",
                        row.requirement_id
                    ),
                ));
            }
        }

        decision_from_issues(issues)
    }

    /// Validate a completed evidence packet against this contract.
    #[must_use]
    pub fn validate_evidence(&self, evidence: &AcceptanceEvidence) -> AcceptanceDecision {
        let contract_decision = self.validate_contract();
        if !contract_decision.passed() {
            return contract_decision;
        }

        let mut issues = Vec::new();

        for gate in self.gates.iter().filter(|gate| gate.required) {
            match evidence
                .gates
                .iter()
                .find(|result| result.gate_id == gate.id)
            {
                Some(result) if result.outcome == AcceptanceOutcome::Passed => {}
                Some(result) => issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_020",
                    format!(
                        "required gate '{}' did not pass: {:?}",
                        gate.id, result.outcome
                    ),
                )),
                None => issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_021",
                    format!("required gate '{}' has no evidence", gate.id),
                )),
            }
        }

        if self.no_stub.as_ref().is_some_and(|req| req.required) {
            match &evidence.no_stub {
                Some(scan)
                    if scan.outcome == AcceptanceOutcome::Passed && scan.findings.is_empty() => {}
                Some(scan) => issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_022",
                    format!("no-stub evidence did not pass: {:?}", scan.outcome),
                )),
                None => issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_023",
                    "required no-stub evidence is missing",
                )),
            }
        }

        if self.agent_output.as_ref().is_some_and(|req| req.required) {
            match &evidence.agent_output {
                Some(output) if output.parsed && output.schema_valid => {}
                Some(_) => issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_024",
                    "structured agent output did not parse against its schema",
                )),
                None => issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_025",
                    "required structured agent output evidence is missing",
                )),
            }
        }

        if let Some(review_req) = self.review_verdict.as_ref().filter(|req| req.required) {
            match &evidence.review_verdict {
                Some(review)
                    if review.reviewer_role_id == review_req.reviewer_role_id
                        && review.status == AcceptanceOutcome::Passed
                        && review.confidence >= review_req.min_confidence
                        && review.blocking_findings.is_empty()
                        && !review.raw_output_ref.trim().is_empty()
                        && review.confidence.is_finite()
                        && review.required_next_action == RequiredNextAction::None => {}
                Some(review) => issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_026",
                    format!(
                        "review verdict did not satisfy contract: status={:?}, confidence={}, reviewer_role_id={}, required_next_action={:?}",
                        review.status,
                        review.confidence,
                        review.reviewer_role_id,
                        review.required_next_action
                    ),
                )),
                None => issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_027",
                    "required review verdict evidence is missing",
                )),
            }
        }

        if let Some(recovery_req) = self.recovery.as_ref().filter(|req| req.required) {
            let recovery = evidence.recovery.as_ref();
            if recovery_req.retry && !recovery.is_some_and(|item| item.retry_recorded) {
                issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_028",
                    "required retry evidence is missing",
                ));
            }
            if recovery_req.reflection && !recovery.is_some_and(|item| item.reflection_recorded) {
                issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_029",
                    "required reflection evidence is missing",
                ));
            }
            if recovery_req.replan && !recovery.is_some_and(|item| item.replan_recorded) {
                issues.push(AcceptanceIssue::blocking(
                    "ACCEPT_030",
                    "required replan evidence is missing",
                ));
            }
        }

        if let Some(parity_req) = self.parity_ledger.as_ref().filter(|req| req.required) {
            for row in &parity_req.rows {
                match evidence
                    .parity_ledger_rows
                    .iter()
                    .find(|candidate| candidate.requirement_id == row.requirement_id)
                {
                    Some(evidence_row)
                        if evidence_row.outcome == AcceptanceOutcome::Passed
                            && evidence_row.status == ParityLedgerStatus::Verified
                            && !evidence_row.effective_source_ref(row).trim().is_empty()
                            && !evidence_row.implementation_evidence_refs().is_empty()
                            && !evidence_row.test_evidence_refs.is_empty() => {}
                    Some(evidence_row) => issues.push(AcceptanceIssue::blocking(
                        "ACCEPT_031",
                        format!(
                            "parity ledger row '{}' did not close: outcome={:?}, status={:?}, implementation_refs={}, test_evidence_refs={}",
                            row.requirement_id,
                            evidence_row.outcome,
                            evidence_row.status,
                            evidence_row.implementation_evidence_refs().len(),
                            evidence_row.test_evidence_refs.len()
                        ),
                    )),
                    None => issues.push(AcceptanceIssue::blocking(
                        "ACCEPT_032",
                        format!(
                            "required parity ledger row '{}' is missing",
                            row.requirement_id
                        ),
                    )),
                }
            }
        }

        if issues.iter().any(|issue| issue.blocking) {
            return decision_from_evidence_issues(issues);
        }

        AcceptanceDecision {
            outcome: evidence.outcome,
            issues,
        }
    }
}

/// A single required verification command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GateRequirement {
    /// Stable identifier used by evidence packets.
    pub id: String,
    /// The kind of gate this command represents.
    pub kind: GateRequirementKind,
    /// Shell command or named gate invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Whether this gate blocks completion.
    #[serde(default = "default_true")]
    pub required: bool,
}

/// Verify categories understood by the done-gate contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateRequirementKind {
    /// A compile/build gate such as `cargo check`.
    Compile,
    /// A test gate such as `cargo test`.
    Test,
    /// A lint/static-analysis gate such as `cargo clippy`.
    Lint,
    /// A structured review gate.
    Review,
    /// A caller-defined gate category.
    Custom,
}

/// No-stub production path requirement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoStubRequirement {
    /// Whether no-stub evidence blocks completion.
    #[serde(default = "default_true")]
    pub required: bool,
    /// Paths that must be covered by non-stub evidence.
    #[serde(default)]
    pub production_paths: Vec<String>,
}

/// Structured output requirement for the implementing agent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StructuredAgentOutputRequirement {
    /// Whether output parsing blocks completion.
    #[serde(default = "default_true")]
    pub required: bool,
    /// Schema id, JSON schema path, or manifest id.
    pub schema: String,
}

/// Structured review verdict requirement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewVerdictRequirement {
    /// Whether review verdict evidence blocks completion.
    #[serde(default = "default_true")]
    pub required: bool,
    /// Reviewer role/profile id expected to produce the verdict.
    pub reviewer_role_id: String,
    /// Minimum accepted confidence in `[0.0, 1.0]`.
    #[serde(default)]
    pub min_confidence: f32,
}

/// Required recovery signals after failed gates.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecoveryRequirement {
    /// Whether recovery evidence blocks completion.
    #[serde(default = "default_true")]
    pub required: bool,
    /// Require a retry signal.
    #[serde(default)]
    pub retry: bool,
    /// Require a reflection signal.
    #[serde(default)]
    pub reflection: bool,
    /// Require a replan signal.
    #[serde(default)]
    pub replan: bool,
}

/// Parity ledger evidence requirement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParityLedgerRequirement {
    /// Whether parity ledger rows block completion.
    #[serde(default = "default_true")]
    pub required: bool,
    /// Required ledger rows for implemented doc requirements.
    #[serde(default)]
    pub rows: Vec<ParityLedgerRequirementRow>,
}

/// A required parity ledger row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParityLedgerRequirementRow {
    /// Stable doc requirement id.
    pub requirement_id: String,
    /// Source document path or requirement reference.
    pub source_ref: String,
    /// Legacy implementation evidence artifact path or structured ledger key.
    #[serde(default)]
    pub evidence_ref: String,
    /// Implementation evidence artifact paths or structured ledger keys.
    #[serde(default)]
    pub implementation_refs: Vec<String>,
    /// Declared test or gate evidence refs for this requirement, when known at plan time.
    #[serde(default)]
    pub test_evidence_refs: Vec<String>,
}

/// Completed evidence packet for one task/run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceEvidence {
    /// Overall task outcome reported by the executor.
    pub outcome: AcceptanceOutcome,
    /// Verify results keyed by [`GateRequirement::id`].
    #[serde(default)]
    pub gates: Vec<GateEvidence>,
    /// No-stub scan evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_stub: Option<NoStubEvidence>,
    /// Structured output parse evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_output: Option<StructuredOutputEvidence>,
    /// Structured review verdict evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_verdict: Option<ReviewVerdictEvidence>,
    /// Retry/reflection/replan evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<RecoveryEvidence>,
    /// Parity rows actually recorded for the task.
    #[serde(default)]
    pub parity_ledger_rows: Vec<ParityLedgerEvidenceRow>,
}

/// Evidence for one gate requirement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GateEvidence {
    /// Requirement id this result satisfies.
    pub gate_id: String,
    /// Verify outcome.
    pub outcome: AcceptanceOutcome,
    /// Evidence path, command log, or content-addressed artifact id.
    pub evidence_ref: String,
}

/// Evidence that no production path was satisfied by a stub/noop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoStubEvidence {
    /// Scan outcome.
    pub outcome: AcceptanceOutcome,
    /// Paths scanned.
    #[serde(default)]
    pub scanned_paths: Vec<String>,
    /// Stub/noop findings. Must be empty for a passing required scan.
    #[serde(default)]
    pub findings: Vec<String>,
}

/// Evidence that agent output was parsed structurally.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredOutputEvidence {
    /// Whether output was parseable.
    pub parsed: bool,
    /// Whether parsed output matched the required schema.
    pub schema_valid: bool,
    /// Raw output path or artifact id.
    pub raw_output_ref: String,
}

/// Review verdict evidence with enough structure for orchestration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewVerdictEvidence {
    /// Stable verdict id.
    pub verdict_id: String,
    /// Batch or task family id.
    pub batch_id: String,
    /// Task id.
    pub task_id: String,
    /// Reviewer role/profile id.
    pub reviewer_role_id: String,
    /// Structured review outcome.
    pub status: AcceptanceOutcome,
    /// Reviewer confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Blocking findings.
    #[serde(default)]
    pub blocking_findings: Vec<String>,
    /// Non-blocking findings.
    #[serde(default)]
    pub non_blocking_findings: Vec<String>,
    /// Required next action for the orchestrator.
    pub required_next_action: RequiredNextAction,
    /// Evidence paths or artifact ids.
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    /// Raw reviewer output path or artifact id.
    pub raw_output_ref: String,
    /// Creation timestamp supplied by the caller.
    pub created_at: String,
}

/// Required next action from a structured review verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequiredNextAction {
    /// No follow-up action is required.
    None,
    /// Retry the task with bounded new evidence.
    Retry,
    /// Record reflection before continuing.
    Reflect,
    /// Replan before retrying.
    Replan,
    /// Escalate to a human reviewer.
    Human,
}

/// Evidence that the executor recorded recovery signals.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryEvidence {
    /// Retry signal/action id was recorded.
    #[serde(default)]
    pub retry_recorded: bool,
    /// Reflection signal/action id was recorded.
    #[serde(default)]
    pub reflection_recorded: bool,
    /// Replan signal/action id was recorded.
    #[serde(default)]
    pub replan_recorded: bool,
}

/// Evidence row recorded in a parity ledger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParityLedgerEvidenceRow {
    /// Stable doc requirement id.
    pub requirement_id: String,
    /// Row outcome.
    pub outcome: AcceptanceOutcome,
    /// Row closure status.
    #[serde(default)]
    pub status: ParityLedgerStatus,
    /// Source document path or requirement reference.
    #[serde(default)]
    pub source_ref: String,
    /// Legacy implementation evidence path or artifact id.
    #[serde(default)]
    pub evidence_ref: String,
    /// Implementation evidence paths or artifact ids.
    #[serde(default)]
    pub implementation_refs: Vec<String>,
    /// Test or gate evidence paths or artifact ids.
    #[serde(default)]
    pub test_evidence_refs: Vec<String>,
}

impl ParityLedgerEvidenceRow {
    /// Implementation evidence refs, including the legacy `evidence_ref` field.
    #[must_use]
    pub fn implementation_evidence_refs(&self) -> Vec<&str> {
        self.implementation_refs
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(self.evidence_ref.as_str()))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect()
    }

    fn effective_source_ref<'a>(&'a self, requirement: &'a ParityLedgerRequirementRow) -> &'a str {
        if self.source_ref.trim().is_empty() {
            &requirement.source_ref
        } else {
            &self.source_ref
        }
    }
}

/// Closure state for a doc parity ledger row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParityLedgerStatus {
    /// Implementation evidence exists but runtime evidence has not closed it.
    Implemented,
    /// Implementation and test/gate evidence both exist.
    #[default]
    Verified,
    /// Completion is blocked by missing external state.
    Blocked,
    /// More work is required before this doc requirement can close.
    NeedsWork,
}

/// Done-gate validation decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceDecision {
    /// Final outcome after fail-closed validation.
    pub outcome: AcceptanceOutcome,
    /// Structured validation issues.
    #[serde(default)]
    pub issues: Vec<AcceptanceIssue>,
}

impl AcceptanceDecision {
    /// True only when the decision has no blocking issue and the outcome passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.outcome == AcceptanceOutcome::Passed && self.issues.iter().all(|issue| !issue.blocking)
    }
}

/// Validation issue for a contract or evidence packet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceIssue {
    /// Stable machine-readable code.
    pub code: String,
    /// Human-readable diagnostic.
    pub message: String,
    /// Whether this issue blocks completion.
    pub blocking: bool,
}

impl AcceptanceIssue {
    fn blocking(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            blocking: true,
        }
    }
}

fn decision_from_issues(issues: Vec<AcceptanceIssue>) -> AcceptanceDecision {
    let outcome = if issues.iter().any(|issue| issue.blocking) {
        AcceptanceOutcome::Failed
    } else {
        AcceptanceOutcome::Passed
    };
    AcceptanceDecision { outcome, issues }
}

fn decision_from_evidence_issues(issues: Vec<AcceptanceIssue>) -> AcceptanceDecision {
    let outcome = if issues.iter().any(|issue| issue.blocking) {
        if issues
            .iter()
            .filter(|issue| issue.blocking)
            .all(|issue| matches!(issue.code.as_str(), "ACCEPT_031" | "ACCEPT_032"))
        {
            AcceptanceOutcome::NeedsWork
        } else {
            AcceptanceOutcome::Failed
        }
    } else {
        AcceptanceOutcome::Passed
    };
    AcceptanceDecision { outcome, issues }
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_contract() -> AcceptanceContract {
        AcceptanceContract {
            version: 1,
            gates: vec![
                GateRequirement {
                    id: "compile".to_string(),
                    kind: GateRequirementKind::Compile,
                    command: Some("cargo check -p roko-gate".to_string()),
                    required: true,
                },
                GateRequirement {
                    id: "test".to_string(),
                    kind: GateRequirementKind::Test,
                    command: Some("cargo test -p roko-gate --lib --no-run".to_string()),
                    required: true,
                },
            ],
            no_stub: Some(NoStubRequirement {
                required: true,
                production_paths: vec!["crates/roko-gate/src".to_string()],
            }),
            agent_output: Some(StructuredAgentOutputRequirement {
                required: true,
                schema: "roko.acceptance.agent_output.v1".to_string(),
            }),
            review_verdict: Some(ReviewVerdictRequirement {
                required: true,
                reviewer_role_id: "quick-reviewer".to_string(),
                min_confidence: 0.6,
            }),
            recovery: Some(RecoveryRequirement {
                required: true,
                retry: true,
                reflection: true,
                replan: true,
            }),
            parity_ledger: Some(ParityLedgerRequirement {
                required: true,
                rows: vec![ParityLedgerRequirementRow {
                    requirement_id: "RT00.done-gate".to_string(),
                    source_ref: "tmp/architecture-plans/08-end-to-end-acceptance.md".to_string(),
                    evidence_ref: "crates/roko-gate/src/acceptance_contract.rs".to_string(),
                    implementation_refs: Vec::new(),
                    test_evidence_refs: Vec::new(),
                }],
            }),
        }
    }

    fn full_evidence() -> AcceptanceEvidence {
        AcceptanceEvidence {
            outcome: AcceptanceOutcome::Passed,
            gates: vec![
                GateEvidence {
                    gate_id: "compile".to_string(),
                    outcome: AcceptanceOutcome::Passed,
                    evidence_ref: ".roko/runs/compile.log".to_string(),
                },
                GateEvidence {
                    gate_id: "test".to_string(),
                    outcome: AcceptanceOutcome::Passed,
                    evidence_ref: ".roko/runs/test.log".to_string(),
                },
            ],
            no_stub: Some(NoStubEvidence {
                outcome: AcceptanceOutcome::Passed,
                scanned_paths: vec!["crates/roko-gate/src".to_string()],
                findings: Vec::new(),
            }),
            agent_output: Some(StructuredOutputEvidence {
                parsed: true,
                schema_valid: true,
                raw_output_ref: ".roko/runs/agent-output.json".to_string(),
            }),
            review_verdict: Some(ReviewVerdictEvidence {
                verdict_id: "verdict-1".to_string(),
                batch_id: "RT00".to_string(),
                task_id: "RT00".to_string(),
                reviewer_role_id: "quick-reviewer".to_string(),
                status: AcceptanceOutcome::Passed,
                confidence: 0.9,
                blocking_findings: Vec::new(),
                non_blocking_findings: Vec::new(),
                required_next_action: RequiredNextAction::None,
                evidence_refs: vec!["crates/roko-gate/src/acceptance_contract.rs".to_string()],
                raw_output_ref: ".roko/runs/review.json".to_string(),
                created_at: "2026-04-25T12:43:56Z".to_string(),
            }),
            recovery: Some(RecoveryEvidence {
                retry_recorded: true,
                reflection_recorded: true,
                replan_recorded: true,
            }),
            parity_ledger_rows: vec![ParityLedgerEvidenceRow {
                requirement_id: "RT00.done-gate".to_string(),
                outcome: AcceptanceOutcome::Passed,
                status: ParityLedgerStatus::Verified,
                source_ref: "tmp/architecture-plans/08-end-to-end-acceptance.md".to_string(),
                evidence_ref: "crates/roko-gate/src/acceptance_contract.rs".to_string(),
                implementation_refs: Vec::new(),
                test_evidence_refs: vec![".roko/runs/test.log".to_string()],
            }],
        }
    }

    #[test]
    fn valid_contract_and_evidence_pass() {
        let contract = full_contract();
        let evidence = full_evidence();

        let decision = contract.validate_evidence(&evidence);

        assert!(decision.passed(), "{decision:?}");
    }

    #[test]
    fn missing_required_gate_fails_closed() {
        let contract = full_contract();
        let mut evidence = full_evidence();
        evidence.gates.retain(|gate| gate.gate_id != "compile");

        let decision = contract.validate_evidence(&evidence);

        assert_eq!(decision.outcome, AcceptanceOutcome::Failed);
        assert!(
            decision
                .issues
                .iter()
                .any(|issue| issue.code == "ACCEPT_021")
        );
    }

    #[test]
    fn malformed_contract_fails_closed() {
        let mut contract = full_contract();
        contract.version = 99;

        let decision = contract.validate_contract();

        assert_eq!(decision.outcome, AcceptanceOutcome::Failed);
        assert!(
            decision
                .issues
                .iter()
                .any(|issue| issue.code == "ACCEPT_001")
        );
    }

    #[test]
    fn unparsable_outcome_is_rejected_by_serde() {
        let raw = r#"{"outcome":"done","gates":[],"parity_ledger_rows":[]}"#;

        let parsed = serde_json::from_str::<AcceptanceEvidence>(raw);

        assert!(parsed.is_err());
    }

    #[test]
    fn review_verdict_with_next_action_fails_closed() {
        let contract = full_contract();
        let mut evidence = full_evidence();
        let review = evidence.review_verdict.as_mut().expect("review evidence");
        review.required_next_action = RequiredNextAction::Retry;

        let decision = contract.validate_evidence(&evidence);

        assert_eq!(decision.outcome, AcceptanceOutcome::Failed);
        assert!(
            decision
                .issues
                .iter()
                .any(|issue| issue.code == "ACCEPT_026")
        );
    }

    #[test]
    fn review_verdict_wrong_reviewer_fails_closed() {
        let contract = full_contract();
        let mut evidence = full_evidence();
        let review = evidence.review_verdict.as_mut().expect("review evidence");
        review.reviewer_role_id = "unexpected-reviewer".to_string();

        let decision = contract.validate_evidence(&evidence);

        assert_eq!(decision.outcome, AcceptanceOutcome::Failed);
        assert!(
            decision
                .issues
                .iter()
                .any(|issue| issue.code == "ACCEPT_026")
        );
    }

    #[test]
    fn missing_parity_test_evidence_needs_work() {
        let contract = full_contract();
        let mut evidence = full_evidence();
        evidence.parity_ledger_rows[0].test_evidence_refs.clear();

        let decision = contract.validate_evidence(&evidence);

        assert_eq!(decision.outcome, AcceptanceOutcome::NeedsWork);
        assert!(
            decision
                .issues
                .iter()
                .any(|issue| issue.code == "ACCEPT_031")
        );
    }

    #[test]
    fn parity_row_status_must_be_verified() {
        let contract = full_contract();
        let mut evidence = full_evidence();
        evidence.parity_ledger_rows[0].status = ParityLedgerStatus::NeedsWork;

        let decision = contract.validate_evidence(&evidence);

        assert_eq!(decision.outcome, AcceptanceOutcome::NeedsWork);
        assert!(
            decision
                .issues
                .iter()
                .any(|issue| issue.message.contains("status=NeedsWork"))
        );
    }

    #[test]
    fn parity_requirement_accepts_structured_implementation_refs() {
        let mut contract = full_contract();
        let row = &mut contract
            .parity_ledger
            .as_mut()
            .expect("parity requirement")
            .rows[0];
        row.evidence_ref.clear();
        row.implementation_refs = vec!["crates/roko-gate/src/acceptance_contract.rs".to_string()];

        let decision = contract.validate_contract();

        assert!(decision.passed(), "{decision:?}");
    }
}
