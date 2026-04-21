//! Unified authorization decisions for the safety spine.
//!
//! The live runtime still enforces safety through [`crate::safety::SafetyLayer`]
//! checks. These types provide the documented higher-level decision surface so
//! callers can reason about permit, deny, and escalation outcomes without
//! flattening everything into a bare [`roko_core::tool::ToolError`].

use serde::{Deserialize, Serialize};

use roko_core::tool::ToolError;

/// Where a blocked or uncertain action should be escalated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationTarget {
    /// Ask the current user for an explicit review decision.
    UserReview,
    /// Escalate to an operator or deployment owner.
    Operator,
    /// Escalate to a stronger safety or security workflow.
    SecurityPolicy,
}

/// Durable explanation for why an authorization decision was reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationSource {
    /// Standing role-based grant.
    RoleGrant,
    /// Session-scoped approval or warrant.
    SessionApproval,
    /// One-off approval tied to a single action.
    OneShotApproval,
    /// Result of an escalation workflow.
    Escalation,
}

/// Evidence attached to an authorization decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationEvidence {
    /// The source of authority that justified the decision.
    pub source: AuthorizationSource,
    /// Human-readable scope or explanation.
    pub detail: String,
}

impl AuthorizationEvidence {
    /// Create evidence describing a standing role grant.
    #[must_use]
    pub fn role_grant(detail: impl Into<String>) -> Self {
        Self {
            source: AuthorizationSource::RoleGrant,
            detail: detail.into(),
        }
    }

    /// Create evidence describing session-scoped approval.
    #[must_use]
    pub fn session_approval(detail: impl Into<String>) -> Self {
        Self {
            source: AuthorizationSource::SessionApproval,
            detail: detail.into(),
        }
    }

    /// Create evidence describing a one-shot human approval.
    #[must_use]
    pub fn one_shot_approval(detail: impl Into<String>) -> Self {
        Self {
            source: AuthorizationSource::OneShotApproval,
            detail: detail.into(),
        }
    }

    /// Create evidence describing an escalation decision.
    #[must_use]
    pub fn escalation(detail: impl Into<String>) -> Self {
        Self {
            source: AuthorizationSource::Escalation,
            detail: detail.into(),
        }
    }
}

/// High-level authorization result for a proposed action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthzDecision {
    /// The action is permitted as-is.
    Allow {
        /// Evidence explaining why the action is allowed.
        evidence: Vec<AuthorizationEvidence>,
    },
    /// The action is allowed, but only after explicit user confirmation.
    AllowWithConfirm {
        /// Prompt that should be shown to the operator.
        prompt: String,
        /// Evidence explaining why confirmation is required.
        evidence: Vec<AuthorizationEvidence>,
    },
    /// The action may proceed once under a narrow approval scope.
    AllowOnce {
        /// Evidence explaining the one-shot scope.
        evidence: Vec<AuthorizationEvidence>,
    },
    /// The action is denied.
    Deny {
        /// Human-readable denial reason.
        reason: String,
    },
    /// The action cannot proceed without escalation.
    Escalate {
        /// Destination for the escalation.
        to: EscalationTarget,
        /// Human-readable escalation reason.
        reason: String,
    },
}

impl AuthzDecision {
    /// Returns `true` if the decision permits execution without more work.
    #[must_use]
    pub const fn is_immediately_allowed(&self) -> bool {
        matches!(self, Self::Allow { .. } | Self::AllowOnce { .. })
    }

    /// Convert a decision into a dispatcher-style result.
    ///
    /// Confirmation and escalation paths fail closed until an outer workflow
    /// provides the additional approval.
    pub fn into_tool_result(self) -> Result<(), ToolError> {
        match self {
            Self::Allow { .. } | Self::AllowOnce { .. } => Ok(()),
            Self::AllowWithConfirm { prompt, .. } => Err(ToolError::PermissionDenied(format!(
                "confirmation required: {prompt}"
            ))),
            Self::Deny { reason } => Err(ToolError::PermissionDenied(reason)),
            Self::Escalate { to, reason } => Err(ToolError::PermissionDenied(format!(
                "escalation to {to:?} required: {reason}"
            ))),
        }
    }

    /// Convert a decision into a result, using the given confirmation channel
    /// for `AllowWithConfirm` and `Escalate` decisions.
    ///
    /// When the channel approves, the decision is converted to `Ok(())` and
    /// a [`ConfirmationOutcome`] is returned alongside. When the channel
    /// denies, the decision is converted to `Err(PermissionDenied)`.
    pub fn resolve_with_channel(
        self,
        channel: &dyn ConfirmationChannel,
    ) -> (Result<(), ToolError>, Option<ConfirmationOutcome>) {
        match self {
            Self::Allow { .. } | Self::AllowOnce { .. } => (Ok(()), None),
            Self::AllowWithConfirm { prompt, .. } => {
                let approved = channel.prompt(&prompt);
                let outcome = ConfirmationOutcome {
                    prompt: prompt.clone(),
                    approved,
                    source: if approved {
                        ConfirmationSource::Interactive
                    } else {
                        ConfirmationSource::FailClosed
                    },
                };
                let result = if approved {
                    Ok(())
                } else {
                    Err(ToolError::PermissionDenied(format!(
                        "confirmation denied: {prompt}"
                    )))
                };
                (result, Some(outcome))
            }
            Self::Deny { reason } => (Err(ToolError::PermissionDenied(reason)), None),
            Self::Escalate { to, reason } => {
                let (approved, source) = match &to {
                    EscalationTarget::UserReview => {
                        let escalation_prompt =
                            format!("escalation to user review: {reason}. Approve?");
                        let approved = channel.prompt(&escalation_prompt);
                        (approved, ConfirmationSource::Escalation)
                    }
                    EscalationTarget::Operator | EscalationTarget::SecurityPolicy => {
                        tracing::warn!(
                            ?to,
                            %reason,
                            "escalation target requires out-of-band handling; denying"
                        );
                        (false, ConfirmationSource::FailClosed)
                    }
                };
                let outcome = ConfirmationOutcome {
                    prompt: reason.clone(),
                    approved,
                    source,
                };
                let result = if approved {
                    Ok(())
                } else {
                    Err(ToolError::PermissionDenied(format!(
                        "escalation to {to:?} denied: {reason}"
                    )))
                };
                (result, Some(outcome))
            }
        }
    }
}

// ─── Confirmation channel ───────────────────────────────────────────────

/// Trait for collecting human approval when `AllowWithConfirm` or `Escalate`
/// decisions are reached.
///
/// Implementors present the prompt to the operator and return `true` for
/// approval, `false` for denial.
pub trait ConfirmationChannel: Send + Sync {
    /// Present a confirmation prompt and return whether the operator approved.
    fn prompt(&self, prompt: &str) -> bool;
}

/// A confirmation channel that always denies (fail-closed default).
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyAllChannel;

impl ConfirmationChannel for DenyAllChannel {
    fn prompt(&self, _prompt: &str) -> bool {
        false
    }
}

/// A confirmation channel that always approves.
///
/// Useful for testing or non-interactive batch modes where the operator
/// has pre-authorized all actions.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApproveAllChannel;

impl ConfirmationChannel for ApproveAllChannel {
    fn prompt(&self, _prompt: &str) -> bool {
        true
    }
}

/// A confirmation channel that logs the prompt and denies.
///
/// Suitable for daemon mode where no interactive terminal is available
/// but the prompt should be recorded for later review.
#[derive(Debug, Clone, Copy, Default)]
pub struct LogAndDenyChannel;

impl ConfirmationChannel for LogAndDenyChannel {
    fn prompt(&self, prompt: &str) -> bool {
        tracing::warn!(
            prompt,
            "confirmation required but no interactive channel; denying"
        );
        false
    }
}

/// The outcome of processing a confirmation decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmationOutcome {
    /// The prompt that was shown to the operator.
    pub prompt: String,
    /// Whether the operator approved.
    pub approved: bool,
    /// The source of the decision.
    pub source: ConfirmationSource,
}

/// Where the confirmation was collected from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationSource {
    /// Interactive confirmation from the CLI.
    Interactive,
    /// Automatic denial (no channel available).
    FailClosed,
    /// Automatic approval (batch/pre-authorized mode).
    PreAuthorized,
    /// Escalation path.
    Escalation,
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_all_channel_rejects() {
        let channel = DenyAllChannel;
        assert!(!channel.prompt("allow?"));
    }

    #[test]
    fn approve_all_channel_accepts() {
        let channel = ApproveAllChannel;
        assert!(channel.prompt("allow?"));
    }

    #[test]
    fn allow_decision_resolves_immediately() {
        let decision = AuthzDecision::Allow { evidence: vec![] };
        let (result, outcome) = decision.resolve_with_channel(&DenyAllChannel);
        assert!(result.is_ok());
        assert!(outcome.is_none());
    }

    #[test]
    fn allow_with_confirm_approved_by_channel() {
        let decision = AuthzDecision::AllowWithConfirm {
            prompt: "tainted write".into(),
            evidence: vec![],
        };
        let (result, outcome) = decision.resolve_with_channel(&ApproveAllChannel);
        assert!(result.is_ok());
        let outcome = outcome.expect("should have outcome");
        assert!(outcome.approved);
        assert_eq!(outcome.source, ConfirmationSource::Interactive);
    }

    #[test]
    fn allow_with_confirm_denied_by_channel() {
        let decision = AuthzDecision::AllowWithConfirm {
            prompt: "dangerous action".into(),
            evidence: vec![],
        };
        let (result, outcome) = decision.resolve_with_channel(&DenyAllChannel);
        assert!(result.is_err());
        let outcome = outcome.expect("should have outcome");
        assert!(!outcome.approved);
        assert_eq!(outcome.source, ConfirmationSource::FailClosed);
    }

    #[test]
    fn deny_decision_resolves_to_error() {
        let decision = AuthzDecision::Deny {
            reason: "nope".into(),
        };
        let (result, outcome) = decision.resolve_with_channel(&ApproveAllChannel);
        assert!(result.is_err());
        assert!(outcome.is_none());
    }

    #[test]
    fn escalate_user_review_can_be_approved() {
        let decision = AuthzDecision::Escalate {
            to: EscalationTarget::UserReview,
            reason: "needs review".into(),
        };
        let (result, outcome) = decision.resolve_with_channel(&ApproveAllChannel);
        assert!(result.is_ok());
        let outcome = outcome.expect("should have outcome");
        assert!(outcome.approved);
        assert_eq!(outcome.source, ConfirmationSource::Escalation);
    }

    #[test]
    fn escalate_operator_always_denied() {
        let decision = AuthzDecision::Escalate {
            to: EscalationTarget::Operator,
            reason: "needs operator".into(),
        };
        let (result, outcome) = decision.resolve_with_channel(&ApproveAllChannel);
        assert!(result.is_err());
        let outcome = outcome.expect("should have outcome");
        assert!(!outcome.approved);
    }

    #[test]
    fn into_tool_result_backwards_compat() {
        let allow = AuthzDecision::Allow { evidence: vec![] };
        assert!(allow.into_tool_result().is_ok());

        let deny = AuthzDecision::Deny {
            reason: "no".into(),
        };
        assert!(deny.into_tool_result().is_err());
    }

    #[test]
    fn one_shot_approval_evidence_round_trips() {
        let evidence = AuthorizationEvidence::one_shot_approval("user clicked yes");
        let json = serde_json::to_string(&evidence).unwrap();
        let decoded: AuthorizationEvidence = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.source, AuthorizationSource::OneShotApproval);
    }
}
