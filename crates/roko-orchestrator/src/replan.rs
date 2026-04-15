//! Re-planning strategy selection for failed tasks and plans.

use serde::{Deserialize, Serialize};

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
            } => write!(f, "decompose({plan_id}, {task_id}, {:?})", new_task_ids),
            Self::RegeneratePlan {
                plan_id,
                task_id,
                new_task_ids,
            } => write!(
                f,
                "regenerate_plan({plan_id}, {task_id}, {:?})",
                new_task_ids
            ),
            Self::Skip { plan_id, task_id } => write!(f, "skip({plan_id}, {task_id})"),
        }
    }
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
}
