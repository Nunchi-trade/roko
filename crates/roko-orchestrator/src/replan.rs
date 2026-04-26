//! Re-planning strategy selection for failed tasks and plans.

use serde::{Deserialize, Serialize};

/// Structured disposition for a gate/task failure after retries are considered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureDisposition {
    /// The same task should be retried or deterministically remediated first.
    Retry,
    /// The current plan is the wrong shape; request planner revision.
    NeedsReplan,
    /// The run is blocked by an external condition and should not spin retries.
    Blocked,
    /// Human input is required before execution can safely continue.
    NeedsHuman,
}

impl FailureDisposition {
    /// Stable label for records, prompts, and metrics.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Retry => "retry",
            Self::NeedsReplan => "needs_replan",
            Self::Blocked => "blocked",
            Self::NeedsHuman => "needs_human",
        }
    }
}

impl std::fmt::Display for FailureDisposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// One piece of structured evidence supporting a plan revision request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRevisionEvidence {
    /// Verify, review, or dispatcher source that produced the evidence.
    pub source: String,
    /// Optional failure classification, such as `type_error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    /// Stable failure pattern ids linked to this evidence.
    #[serde(default)]
    pub failure_pattern_ids: Vec<String>,
    /// Blocking findings that make retry unsafe or insufficient.
    #[serde(default)]
    pub blocking_findings: Vec<String>,
    /// Bounded details for a planner prompt; not a full raw log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl PlanRevisionEvidence {
    /// Build evidence for a gate verdict or gate-derived failure classification.
    #[must_use]
    pub fn gate(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            classification: None,
            failure_pattern_ids: Vec::new(),
            blocking_findings: Vec::new(),
            detail: None,
        }
    }

    /// Attach an optional failure classification.
    #[must_use]
    pub fn with_classification(mut self, classification: Option<String>) -> Self {
        self.classification = classification;
        self
    }

    /// Attach stable failure pattern ids.
    #[must_use]
    pub fn with_failure_pattern_ids(mut self, ids: Vec<String>) -> Self {
        self.failure_pattern_ids = ids;
        self
    }

    /// Attach blocking findings.
    #[must_use]
    pub fn with_blocking_findings(mut self, findings: Vec<String>) -> Self {
        self.blocking_findings = findings;
        self
    }

    /// Attach bounded detail.
    #[must_use]
    pub fn with_detail(mut self, detail: Option<String>) -> Self {
        self.detail = detail;
        self
    }
}

/// Durable request record for a future planner agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRevisionRequest {
    /// Stable request id derived from structured evidence.
    pub request_id: String,
    /// Plan being revised.
    pub plan_id: String,
    /// Task that triggered the request.
    pub task_id: String,
    /// Machine-readable disposition.
    pub disposition: FailureDisposition,
    /// Reason label, for example `gate_failure_limit`.
    pub reason: String,
    /// Verify/task attempt count at the time the request was issued.
    pub attempts: u32,
    /// Evidence items used to choose replan over retry.
    pub evidence: Vec<PlanRevisionEvidence>,
    /// De-duplicated pattern ids across all evidence.
    pub failure_pattern_ids: Vec<String>,
    /// De-duplicated blocking findings across all evidence.
    pub blocking_findings: Vec<String>,
    /// Hash of the old plan/task/failure evidence for resume and dedupe.
    pub resume_token: String,
    /// UTC timestamp for the request.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl PlanRevisionRequest {
    /// Build a durable gate-failure request from structured evidence.
    #[must_use]
    pub fn gate_failure_limit(
        plan_id: impl Into<String>,
        task_id: impl Into<String>,
        attempts: u32,
        evidence: Vec<PlanRevisionEvidence>,
    ) -> Self {
        let plan_id = plan_id.into();
        let task_id = task_id.into();
        let failure_pattern_ids = unique_flatten(
            evidence
                .iter()
                .flat_map(|item| item.failure_pattern_ids.iter().cloned()),
        );
        let blocking_findings = unique_flatten(
            evidence
                .iter()
                .flat_map(|item| item.blocking_findings.iter().cloned()),
        );
        let disposition = if blocking_findings
            .iter()
            .any(|finding| finding.contains("permission") || finding.contains("human"))
        {
            FailureDisposition::NeedsHuman
        } else {
            FailureDisposition::NeedsReplan
        };
        let resume_token = revision_hash(
            &plan_id,
            &task_id,
            "gate_failure_limit",
            attempts,
            &failure_pattern_ids,
            &blocking_findings,
            &evidence,
        );
        let request_id = format!("replan-{resume_token}");

        Self {
            request_id,
            plan_id,
            task_id,
            disposition,
            reason: "gate_failure_limit".to_string(),
            attempts,
            evidence,
            failure_pattern_ids,
            blocking_findings,
            resume_token,
            created_at: chrono::Utc::now(),
        }
    }
}

/// Strategy to apply after a gate failure or plan-level breakdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplanStrategy {
    /// Retry the same task with the current model and context.
    RetrySame,
    /// Retry the same task after upgrading to a stronger model.
    RetryWithEscalation,
    /// Split the failed task into smaller subtasks before retrying.
    Decompose,
    /// Mark the task skipped and continue with the rest of the plan.
    Skip,
    /// Rebuild the plan from scratch and restart execution.
    RegeneratePlan,
}

impl ReplanStrategy {
    /// Human-readable label for logs and metrics.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::RetrySame => "retry_same",
            Self::RetryWithEscalation => "retry_with_escalation",
            Self::Decompose => "decompose",
            Self::Skip => "skip",
            Self::RegeneratePlan => "regenerate_plan",
        }
    }
}

impl std::fmt::Display for ReplanStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Structured outcome produced by a live re-plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ReplanResult {
    /// Retry the same task with the current model/context.
    RetrySame {
        /// The plan that was re-planned.
        plan_id: String,
        /// The failed task that triggered the re-plan.
        task_id: String,
    },
    /// Retry the same task after escalating to a stronger model.
    RetryWithEscalation {
        /// The plan that was re-planned.
        plan_id: String,
        /// The failed task that triggered the re-plan.
        task_id: String,
        /// The stronger model chosen for the retry.
        escalated_model: String,
    },
    /// Replace the failed task with multiple subtasks and restart the plan.
    Decompose {
        /// The plan that was re-planned.
        plan_id: String,
        /// The failed task that triggered the re-plan.
        task_id: String,
        /// New task IDs introduced by the decomposition.
        new_task_ids: Vec<String>,
    },
    /// Rebuild the plan from scratch and restart execution.
    RegeneratePlan {
        /// The plan that was re-planned.
        plan_id: String,
        /// The failed task that triggered the re-plan.
        task_id: String,
        /// New task IDs introduced by the regenerated plan.
        new_task_ids: Vec<String>,
    },
    /// Skip the failed task and continue with the current plan.
    Skip {
        /// The plan that was re-planned.
        plan_id: String,
        /// The failed task that was skipped.
        task_id: String,
    },
}

impl ReplanResult {
    /// Return the strategy represented by this re-plan outcome.
    #[must_use]
    pub const fn strategy(&self) -> ReplanStrategy {
        match self {
            Self::RetrySame { .. } => ReplanStrategy::RetrySame,
            Self::RetryWithEscalation { .. } => ReplanStrategy::RetryWithEscalation,
            Self::Decompose { .. } => ReplanStrategy::Decompose,
            Self::RegeneratePlan { .. } => ReplanStrategy::RegeneratePlan,
            Self::Skip { .. } => ReplanStrategy::Skip,
        }
    }

    /// The plan that was re-planned.
    #[must_use]
    pub fn plan_id(&self) -> &str {
        match self {
            Self::RetrySame { plan_id, .. }
            | Self::RetryWithEscalation { plan_id, .. }
            | Self::Decompose { plan_id, .. }
            | Self::RegeneratePlan { plan_id, .. }
            | Self::Skip { plan_id, .. } => plan_id,
        }
    }

    /// The task that triggered the re-plan.
    #[must_use]
    pub fn task_id(&self) -> &str {
        match self {
            Self::RetrySame { task_id, .. }
            | Self::RetryWithEscalation { task_id, .. }
            | Self::Decompose { task_id, .. }
            | Self::RegeneratePlan { task_id, .. }
            | Self::Skip { task_id, .. } => task_id,
        }
    }

    /// Whether the executor should restart the plan queue entry.
    #[must_use]
    pub const fn requires_restart(&self) -> bool {
        matches!(self, Self::Decompose { .. } | Self::RegeneratePlan { .. })
    }

    /// Any task IDs introduced by the re-plan.
    #[must_use]
    pub fn new_task_ids(&self) -> &[String] {
        match self {
            Self::Decompose { new_task_ids, .. } | Self::RegeneratePlan { new_task_ids, .. } => {
                new_task_ids
            }
            _ => &[],
        }
    }
}

impl std::fmt::Display for ReplanResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RetrySame { plan_id, task_id } => {
                write!(f, "retry_same({plan_id}, {task_id})")
            }
            Self::RetryWithEscalation {
                plan_id,
                task_id,
                escalated_model,
            } => write!(
                f,
                "retry_with_escalation({plan_id}, {task_id}, {escalated_model})"
            ),
            Self::Decompose {
                plan_id,
                task_id,
                new_task_ids,
            } => write!(f, "decompose({plan_id}, {task_id}, {new_task_ids:?})"),
            Self::RegeneratePlan {
                plan_id,
                task_id,
                new_task_ids,
            } => write!(f, "regenerate_plan({plan_id}, {task_id}, {new_task_ids:?})"),
            Self::Skip { plan_id, task_id } => write!(f, "skip({plan_id}, {task_id})"),
        }
    }
}

fn unique_flatten(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut unique = Vec::new();
    for value in values {
        if !value.trim().is_empty() && !unique.contains(&value) {
            unique.push(value);
        }
    }
    unique
}

fn revision_hash(
    plan_id: &str,
    task_id: &str,
    reason: &str,
    attempts: u32,
    failure_pattern_ids: &[String],
    blocking_findings: &[String],
    evidence: &[PlanRevisionEvidence],
) -> String {
    let payload = serde_json::json!({
        "plan_id": plan_id,
        "task_id": task_id,
        "reason": reason,
        "attempts": attempts,
        "failure_pattern_ids": failure_pattern_ids,
        "blocking_findings": blocking_findings,
        "evidence": evidence,
    });
    blake3::hash(payload.to_string().as_bytes())
        .to_hex()
        .to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn strategy_labels_are_stable() {
        assert_eq!(
            ReplanStrategy::RetryWithEscalation.label(),
            "retry_with_escalation"
        );
        assert_eq!(ReplanStrategy::Decompose.to_string(), "decompose");
    }

    #[test]
    fn replan_result_reports_restart_requirement() {
        let result = ReplanResult::Decompose {
            plan_id: "plan-1".into(),
            task_id: "task-1".into(),
            new_task_ids: vec!["a".into(), "b".into()],
        };

        assert!(result.requires_restart());
        assert_eq!(result.strategy(), ReplanStrategy::Decompose);
        assert_eq!(result.plan_id(), "plan-1");
        assert_eq!(result.task_id(), "task-1");
        assert_eq!(result.new_task_ids(), &["a".to_string(), "b".to_string()]);
        assert!(result.to_string().contains("decompose"));
    }

    #[test]
    fn replan_result_non_restart_is_not_structural() {
        let result = ReplanResult::RetryWithEscalation {
            plan_id: "plan-2".into(),
            task_id: "task-9".into(),
            escalated_model: "claude-sonnet".into(),
        };

        assert!(!result.requires_restart());
        assert!(result.new_task_ids().is_empty());
    }

    #[test]
    fn plan_revision_request_preserves_structured_evidence() {
        let request = PlanRevisionRequest::gate_failure_limit(
            "plan-1",
            "T1",
            3,
            vec![
                PlanRevisionEvidence::gate("compile:cargo")
                    .with_classification(Some("architectural_conflict_requires_replan".into()))
                    .with_failure_pattern_ids(vec!["E0425::src/lib.rs".into()])
                    .with_blocking_findings(vec![
                        "failure requires plan shape or dependency revision".into(),
                    ]),
            ],
        );

        assert_eq!(request.disposition, FailureDisposition::NeedsReplan);
        assert_eq!(request.failure_pattern_ids, vec!["E0425::src/lib.rs"]);
        assert_eq!(request.blocking_findings.len(), 1);
        assert!(request.request_id.starts_with("replan-"));
        assert!(!request.resume_token.is_empty());
    }
}
