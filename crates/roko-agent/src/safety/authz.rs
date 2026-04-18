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
}

