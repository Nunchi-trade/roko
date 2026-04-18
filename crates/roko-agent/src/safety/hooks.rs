//! Safety hook and tainted-data primitives.

use std::collections::HashSet;
use std::fmt;

use async_trait::async_trait;
use roko_core::tool::{ToolContext, ToolDef, ToolError};
use serde::{Deserialize, Serialize};

/// A destination that tainted data may or may not be allowed to flow into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSink {
    /// Model-visible context supplied to an LLM backend.
    LlmContext,
    /// Runtime event stream used for telemetry and UI updates.
    EventBus,
    /// Shared collective knowledge or mesh storage.
    CollectiveMesh,
}

/// A sensitivity or provenance label attached to tainted text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaintLabel {
    /// Wallet private material that must never leave the process.
    WalletSecret,
    /// Owner credentials, such as API keys or session tokens.
    OwnerSecret,
    /// Proprietary strategy or planning material.
    StrategyConfidential,
    /// Personal data that requires care before collective storage.
    UserPII,
    /// Untrusted external input that should be validated before use.
    UntrustedExternal,
}

/// Sensitive text tagged with information-flow labels.
///
/// The bytes are overwritten with zeroes when the value is dropped. Callers
/// should use [`TaintedString::can_flow_to`] before placing the text in model
/// context, events, or shared knowledge stores.
#[derive(Clone, PartialEq, Eq)]
pub struct TaintedString {
    value: Vec<u8>,
    labels: HashSet<TaintLabel>,
}

impl TaintedString {
    /// Create a tainted string with the provided labels.
    #[must_use]
    pub fn new(value: impl Into<String>, labels: impl IntoIterator<Item = TaintLabel>) -> Self {
        Self {
            value: value.into().into_bytes(),
            labels: labels.into_iter().collect(),
        }
    }

    /// Borrow the contained text.
    ///
    /// # Panics
    ///
    /// Panics only if a `TaintedString` was constructed from invalid UTF-8,
    /// which cannot happen through the public constructors.
    #[must_use]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.value).expect("tainted string stores valid UTF-8")
    }

    /// Return the labels attached to this value.
    #[must_use]
    pub const fn labels(&self) -> &HashSet<TaintLabel> {
        &self.labels
    }

    /// Return `true` when this value is allowed to flow to `sink`.
    #[must_use]
    pub fn can_flow_to(&self, sink: DataSink) -> bool {
        match sink {
            DataSink::LlmContext => {
                !self.labels.contains(&TaintLabel::WalletSecret)
                    && !self.labels.contains(&TaintLabel::OwnerSecret)
            }
            DataSink::EventBus => !self.labels.contains(&TaintLabel::WalletSecret),
            DataSink::CollectiveMesh => {
                !self.labels.contains(&TaintLabel::StrategyConfidential)
                    && !self.labels.contains(&TaintLabel::UserPII)
                    && !self.labels.contains(&TaintLabel::WalletSecret)
                    && !self.labels.contains(&TaintLabel::OwnerSecret)
            }
        }
    }

    /// Return `true` if the value carries `label`.
    #[must_use]
    pub fn has_label(&self, label: TaintLabel) -> bool {
        self.labels.contains(&label)
    }
}

impl fmt::Debug for TaintedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaintedString")
            .field("value", &"<redacted>")
            .field("labels", &self.labels)
            .finish()
    }
}

impl Drop for TaintedString {
    fn drop(&mut self) {
        for byte in &mut self.value {
            *byte = 0;
        }
    }
}

/// Decision returned by a safety hook for a proposed tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision", content = "value")]
pub enum HookDecision {
    /// Allow the tool call to proceed unchanged.
    Allow,
    /// Allow the tool call with replacement parameters.
    AllowModified(serde_json::Value),
    /// Reject the tool call with a human-readable reason.
    Reject(String),
}

/// Audit record emitted for a safety-hook decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SafetyAuditRecord {
    /// Unix timestamp for the decision.
    pub timestamp: i64,
    /// Tool name being evaluated.
    pub tool_name: String,
    /// Hook implementation name that produced the decision.
    pub hook_name: String,
    /// Hook decision.
    pub decision: HookDecision,
    /// Hash of the input parameters, rather than the raw parameters.
    pub params_hash: String,
    /// Permit id created by the safety layer, when one exists.
    pub permit_id: Option<String>,
    /// Rejection or modification reason, when one exists.
    pub reason: Option<String>,
}

impl SafetyAuditRecord {
    /// Create a new safety audit record.
    #[must_use]
    pub fn new(
        timestamp: i64,
        tool_name: impl Into<String>,
        hook_name: impl Into<String>,
        decision: HookDecision,
        params_hash: impl Into<String>,
    ) -> Self {
        Self {
            timestamp,
            tool_name: tool_name.into(),
            hook_name: hook_name.into(),
            decision,
            params_hash: params_hash.into(),
            permit_id: None,
            reason: None,
        }
    }

    /// Attach a permit id to the audit record.
    #[must_use]
    pub fn with_permit_id(mut self, permit_id: impl Into<String>) -> Self {
        self.permit_id = Some(permit_id.into());
        self
    }

    /// Attach a reason to the audit record.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

/// Safety hook invoked before a tool call executes.
///
/// Hook implementations may approve the call, replace its parameters, or
/// reject it. This trait is intentionally independent of [`SafetyLayer`](super::SafetyLayer)
/// so domain-specific profiles can build hook chains without changing the
/// current dispatcher integration.
#[async_trait]
pub trait SafetyHook: Send + Sync {
    /// Evaluate a proposed tool call.
    async fn on_tool_call(
        &self,
        tool: &ToolDef,
        params: &serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<HookDecision, ToolError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tainted_string_blocks_wallet_secret_everywhere() {
        let secret = TaintedString::new("private key", [TaintLabel::WalletSecret]);

        assert!(!secret.can_flow_to(DataSink::LlmContext));
        assert!(!secret.can_flow_to(DataSink::EventBus));
        assert!(!secret.can_flow_to(DataSink::CollectiveMesh));
    }

    #[test]
    fn tainted_string_applies_sink_specific_rules() {
        let owner_secret = TaintedString::new("api-key", [TaintLabel::OwnerSecret]);
        let strategy = TaintedString::new(
            "alpha",
            [TaintLabel::StrategyConfidential, TaintLabel::UserPII],
        );

        assert!(!owner_secret.can_flow_to(DataSink::LlmContext));
        assert!(owner_secret.can_flow_to(DataSink::EventBus));
        assert!(!owner_secret.can_flow_to(DataSink::CollectiveMesh));

        assert!(strategy.can_flow_to(DataSink::LlmContext));
        assert!(strategy.can_flow_to(DataSink::EventBus));
        assert!(!strategy.can_flow_to(DataSink::CollectiveMesh));
    }

    #[test]
    fn audit_record_builders_attach_optional_fields() {
        let record = SafetyAuditRecord::new(
            42,
            "write_file",
            "policy_cage",
            HookDecision::Reject("readonly role".into()),
            "sha256:abc",
        )
        .with_reason("readonly role")
        .with_permit_id("permit-1");

        assert_eq!(record.timestamp, 42);
        assert_eq!(record.permit_id.as_deref(), Some("permit-1"));
        assert_eq!(record.reason.as_deref(), Some("readonly role"));
    }

    #[test]
    fn hook_decision_serializes_for_audit() {
        let decision = HookDecision::AllowModified(serde_json::json!({ "path": "safe.txt" }));
        let encoded = serde_json::to_string(&decision).unwrap();
        let decoded: HookDecision = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, decision);
    }
}
