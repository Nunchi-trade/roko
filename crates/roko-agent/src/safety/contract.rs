//! Formal behavioral contracts for role-scoped agent governance.
//!
//! These types define invariants, governance rules, and recovery actions that
//! higher-level orchestration can evaluate with a low-latency policy check.

use serde::{Deserialize, Serialize};

/// Behavioral contract for a specific agent role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContract {
    /// Role this contract applies to.
    pub role: String,
    /// Behavioral invariants checked during execution.
    pub invariants: Vec<Invariant>,
    /// Governance rules constraining agent behavior within a turn.
    pub governance: Vec<GovernanceRule>,
    /// Recovery actions for soft invariant violations or policy triggers.
    pub recovery: Vec<RecoveryAction>,
}

/// Declarative invariant attached to an [`AgentContract`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invariant {
    /// Stable human-readable identifier.
    pub name: String,
    /// Whether a violation aborts immediately or attempts recovery.
    pub kind: InvariantKind,
    /// Predicate evaluated by the contract runtime.
    pub predicate: String,
    /// How often the predicate should be checked.
    pub check_frequency: CheckFreq,
}

/// Severity of invariant enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvariantKind {
    /// Violation aborts immediately.
    Hard,
    /// Violation attempts configured recovery.
    Soft,
}

/// Frequency for contract evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckFreq {
    /// Evaluate after every action.
    PerAction,
    /// Evaluate once per turn.
    PerTurn,
    /// Evaluate once per task.
    PerTask,
}

/// Governance rules constraining agent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GovernanceRule {
    /// Hard cap on tool calls in a single turn.
    MaxToolCallsPerTurn(u32),
    /// Tool names that the role may never invoke.
    ForbiddenTools(Vec<String>),
    /// Maximum spend per turn in USD.
    MaxCostPerTurn(f64),
    /// Abort after too many consecutive failures.
    MaxConsecutiveFailures(u32),
    /// Require one tool to appear before another action.
    RequireToolBeforeEdit(String),
}

/// Recovery action triggered by a soft violation or other policy condition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryAction {
    /// Trigger expression evaluated by the contract runtime.
    pub trigger: String,
    /// Action taken when the trigger fires.
    pub action: RecoveryKind,
}

/// Recovery strategies for soft invariant violations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryKind {
    /// Retry the action or turn.
    Retry,
    /// Downgrade to a safer or cheaper execution mode.
    Downgrade,
    /// Abort the current execution.
    Abort,
    /// Emit an alert for external handling.
    Alert,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_contract_round_trips_through_json() {
        let contract = AgentContract {
            role: "implementer".into(),
            invariants: vec![
                Invariant {
                    name: "cost-cap".into(),
                    kind: InvariantKind::Hard,
                    predicate: "cost_usd < 2.0".into(),
                    check_frequency: CheckFreq::PerAction,
                },
                Invariant {
                    name: "failure-budget".into(),
                    kind: InvariantKind::Soft,
                    predicate: "consecutive_failures < 3".into(),
                    check_frequency: CheckFreq::PerTurn,
                },
            ],
            governance: vec![
                GovernanceRule::MaxToolCallsPerTurn(8),
                GovernanceRule::ForbiddenTools(vec!["network".into(), "bash".into()]),
                GovernanceRule::RequireToolBeforeEdit("read_file".into()),
            ],
            recovery: vec![
                RecoveryAction {
                    trigger: "consecutive_failures >= 3".into(),
                    action: RecoveryKind::Downgrade,
                },
                RecoveryAction {
                    trigger: "tool_calls > 8".into(),
                    action: RecoveryKind::Abort,
                },
            ],
        };

        let encoded = serde_json::to_string(&contract).expect("serialize contract");
        let decoded: AgentContract = serde_json::from_str(&encoded).expect("deserialize contract");

        assert_eq!(decoded.role, "implementer");
        assert_eq!(decoded.invariants.len(), 2);
        assert!(matches!(decoded.invariants[0].kind, InvariantKind::Hard));
        assert!(matches!(
            decoded.governance[1],
            GovernanceRule::ForbiddenTools(_)
        ));
        assert!(matches!(
            decoded.recovery[0].action,
            RecoveryKind::Downgrade
        ));
    }

    #[test]
    fn agent_contract_supports_role_scoped_governance() {
        let reviewer = AgentContract {
            role: "reviewer".into(),
            invariants: vec![Invariant {
                name: "review-pass".into(),
                kind: InvariantKind::Hard,
                predicate: "edits == 0".into(),
                check_frequency: CheckFreq::PerTask,
            }],
            governance: vec![
                GovernanceRule::MaxToolCallsPerTurn(4),
                GovernanceRule::ForbiddenTools(vec!["edit_file".into(), "apply_patch".into()]),
            ],
            recovery: vec![RecoveryAction {
                trigger: "attempted_edit".into(),
                action: RecoveryKind::Alert,
            }],
        };

        assert_eq!(reviewer.role, "reviewer");
        assert!(matches!(
            reviewer.governance[0],
            GovernanceRule::MaxToolCallsPerTurn(4)
        ));
        assert!(matches!(reviewer.recovery[0].action, RecoveryKind::Alert));
    }
}
