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
