//! `ExecutorAction` — the vocabulary of side-effects the executor can request.
//!
//! Each [`tick()`](super::ParallelExecutor::tick) call returns a `Vec<ExecutorAction>`.
//! The caller (runtime harness) is responsible for dispatching these actions to
//! the appropriate subsystem (agent pool, gate runner, git merge, etc.).

use roko_core::AgentRole;
use serde::{Deserialize, Serialize};

/// An action the executor wants the runtime to perform.
///
/// Actions are *requests*, not effects — the executor is a pure state machine
/// that never performs I/O itself. The runtime dispatches each action and
/// feeds results back as events on the next tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ExecutorAction {
    /// Begin executing a plan that was queued.
    DispatchPlan {
        /// Which plan to start.
        plan_id: String,
    },

    /// Spawn an agent process for a specific task within a plan.
    SpawnAgent {
        /// The plan this agent works on.
        plan_id: String,
        /// The role the agent should assume.
        role: AgentRole,
        /// The task identifier within the plan.
        task: String,
    },

    /// Run a verification gate (compile, test, clippy, etc.) at a given rung.
    RunGate {
        /// The plan whose worktree to verify.
        plan_id: String,
        /// The gate rung to execute (0 = compile, 1 = test, etc.).
        rung: u32,
    },

    /// Merge a plan's worktree branch into the batch branch.
    MergeBranch {
        /// The plan to merge.
        plan_id: String,
    },

    /// Mark a plan as terminally failed.
    FailPlan {
        /// The plan that failed.
        plan_id: String,
        /// Human-readable failure reason.
        reason: String,
    },

    /// Mark a plan as successfully completed.
    CompletePlan {
        /// The plan that completed.
        plan_id: String,
    },

    /// Move a plan to a different position in the execution queue.
    Reorder {
        /// The plan to reposition.
        plan_id: String,
        /// New zero-based position in the queue.
        new_position: usize,
    },

    /// Pause a running plan (e.g. due to resource contention).
    PausePlan {
        /// The plan to pause.
        plan_id: String,
    },

    /// Resume a previously paused plan.
    ResumePlan {
        /// The plan to resume.
        plan_id: String,
    },
}

impl std::fmt::Display for ExecutorAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DispatchPlan { plan_id } => write!(f, "dispatch({plan_id})"),
            Self::SpawnAgent {
                plan_id,
                role,
                task,
            } => {
                write!(f, "spawn({plan_id}, {role}, {task})")
            }
            Self::RunGate { plan_id, rung } => write!(f, "gate({plan_id}, rung={rung})"),
            Self::MergeBranch { plan_id } => write!(f, "merge({plan_id})"),
            Self::FailPlan { plan_id, reason } => {
                write!(f, "fail({plan_id}: {reason})")
            }
            Self::CompletePlan { plan_id } => write!(f, "complete({plan_id})"),
            Self::Reorder {
                plan_id,
                new_position,
            } => {
                write!(f, "reorder({plan_id} -> {new_position})")
            }
            Self::PausePlan { plan_id } => write!(f, "pause({plan_id})"),
            Self::ResumePlan { plan_id } => write!(f, "resume({plan_id})"),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn action_display_formats() {
        let a = ExecutorAction::DispatchPlan {
            plan_id: "p1".into(),
        };
        assert_eq!(a.to_string(), "dispatch(p1)");

        let b = ExecutorAction::SpawnAgent {
            plan_id: "p2".into(),
            role: AgentRole::Implementer,
            task: "t1".into(),
        };
        assert!(b.to_string().contains("implementer"));

        let c = ExecutorAction::RunGate {
            plan_id: "p3".into(),
            rung: 2,
        };
        assert_eq!(c.to_string(), "gate(p3, rung=2)");
    }

    #[test]
    fn action_serde_roundtrip() {
        let action = ExecutorAction::SpawnAgent {
            plan_id: "plan-42".into(),
            role: AgentRole::Auditor,
            task: "t7".into(),
        };
        let json = serde_json::to_string(&action).unwrap();
        let decoded: ExecutorAction = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, action);
    }

    #[test]
    fn all_variants_serialize() {
        let variants: Vec<ExecutorAction> = vec![
            ExecutorAction::DispatchPlan {
                plan_id: "a".into(),
            },
            ExecutorAction::SpawnAgent {
                plan_id: "b".into(),
                role: AgentRole::Implementer,
                task: "t1".into(),
            },
            ExecutorAction::RunGate {
                plan_id: "c".into(),
                rung: 0,
            },
            ExecutorAction::MergeBranch {
                plan_id: "d".into(),
            },
            ExecutorAction::FailPlan {
                plan_id: "e".into(),
                reason: "boom".into(),
            },
            ExecutorAction::CompletePlan {
                plan_id: "f".into(),
            },
            ExecutorAction::Reorder {
                plan_id: "g".into(),
                new_position: 5,
            },
            ExecutorAction::PausePlan {
                plan_id: "h".into(),
            },
            ExecutorAction::ResumePlan {
                plan_id: "i".into(),
            },
        ];
        for v in &variants {
            let json = serde_json::to_string(v).unwrap();
            let back: ExecutorAction = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, v);
        }
    }
}
