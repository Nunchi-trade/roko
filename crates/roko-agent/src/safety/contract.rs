//! Formal behavioral contracts for role-scoped agent governance.
//!
//! These types define invariants, governance rules, and recovery actions that
//! higher-level orchestration can evaluate with a low-latency policy check.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use roko_core::tool::{ToolCall, ToolContext, ToolError, ToolResult};

const CONTRACT_DIR: &str = "src/safety/contracts";
const NETWORK_TOOLS: &[&str] = &["web_fetch", "web_search"];
const EDIT_TOOLS: &[&str] = &[
    "write_file",
    "edit_file",
    "multi_edit",
    "apply_patch",
    "notebook_edit",
];

/// Behavioral contract for a specific agent role.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentContract {
    /// Role this contract applies to.
    #[serde(default)]
    pub role: String,
    /// Behavioral invariants checked during execution.
    #[serde(default)]
    pub invariants: Vec<Invariant>,
    /// Governance rules constraining agent behavior within a turn.
    #[serde(default)]
    pub governance: Vec<GovernanceRule>,
    /// Recovery actions for soft invariant violations or policy triggers.
    #[serde(default)]
    pub recovery: Vec<RecoveryAction>,
}

impl AgentContract {
    /// Build a permissive contract for `role`.
    ///
    /// This is used as a safe fallback when contract assets are missing or
    /// malformed.
    #[must_use]
    pub fn permissive(role: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            ..Self::default()
        }
    }

    /// Load the bundled contract asset for `role`.
    ///
    /// The loader reads `src/safety/contracts/<role>.yaml` relative to the
    /// `roko-agent` crate root. The files use a YAML-compatible JSON subset,
    /// so they can be parsed without adding a new dependency.
    pub fn load_for_role(role: impl AsRef<str>) -> Result<Self, ContractLoadError> {
        let role = role.as_ref().trim();
        validate_role(role)?;

        let path = contract_asset_path(role);
        let source = fs::read_to_string(&path).map_err(|source| match source.kind() {
            std::io::ErrorKind::NotFound => ContractLoadError::MissingAsset {
                role: role.to_owned(),
                path: path.clone(),
            },
            _ => ContractLoadError::ReadAsset {
                path: path.clone(),
                source,
            },
        })?;

        let mut contract: Self =
            serde_json::from_str(&source).map_err(|source| ContractLoadError::ParseAsset {
                path: path.clone(),
                source,
            })?;

        if !contract.role.is_empty() && contract.role != role {
            return Err(ContractLoadError::RoleMismatch {
                expected: role.to_owned(),
                found: contract.role,
                path,
            });
        }

        contract.role = role.to_owned();
        Ok(contract)
    }

    /// Validate this contract against an inbound tool invocation.
    pub fn check_pre_execution(
        &self,
        call: &ToolCall,
        ctx: &ToolContext,
    ) -> Result<(), ContractViolation> {
        for invariant in &self.invariants {
            invariant.check(&self.role, call, ctx)?;
        }
        for rule in &self.governance {
            rule.check(&self.role, call, ctx)?;
        }
        Ok(())
    }

    /// Return the first configured recovery action that applies to `result`.
    #[must_use]
    pub fn applicable_recovery(&self, result: &ToolResult) -> Option<RecoveryAction> {
        let ToolResult::Err(err) = result else {
            return None;
        };

        self.recovery
            .iter()
            .find(|action| action.matches(err))
            .cloned()
    }
}

/// Errors returned while loading a contract asset.
#[derive(Debug, Error)]
pub enum ContractLoadError {
    /// The requested role name is not safe to map to a file name.
    #[error("unsupported contract role `{role}`")]
    InvalidRole {
        /// Offending role label.
        role: String,
    },
    /// No bundled asset exists for the requested role.
    #[error("missing contract asset for role `{role}` at {path}")]
    MissingAsset {
        /// Requested role label.
        role: String,
        /// Expected asset path.
        path: PathBuf,
    },
    /// The bundled asset could not be read.
    #[error("failed to read contract asset `{path}`: {source}")]
    ReadAsset {
        /// Asset path that failed to read.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The bundled asset could not be parsed.
    #[error("failed to parse contract asset `{path}`: {source}")]
    ParseAsset {
        /// Asset path that failed to parse.
        path: PathBuf,
        /// Underlying parser failure.
        #[source]
        source: serde_json::Error,
    },
    /// The asset declared a different role than the caller requested.
    #[error("contract asset role mismatch for `{path}`: expected `{expected}`, found `{found}`")]
    RoleMismatch {
        /// Role requested by the caller.
        expected: String,
        /// Role declared in the asset.
        found: String,
        /// Asset path that produced the mismatch.
        path: PathBuf,
    },
}

/// Declarative invariant attached to an [`AgentContract`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Invariant {
    /// Cap the number of tokens consumed in a turn.
    MaxTokensPerTurn(u32),
    /// Require a gate pass before the agent may commit changes.
    RequireGateBeforeCommit,
    /// Block any contract that permits network access.
    NoNetworkAccess,
}

/// Governance rules constraining agent execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// Concrete pre-execution contract violation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("contract violation for role `{role}` ({rule}): {detail}")]
pub struct ContractViolation {
    /// Role whose contract was violated.
    pub role: String,
    /// Stable rule label for the violated invariant or governance rule.
    pub rule: &'static str,
    /// Human-readable detail for the failure.
    pub detail: String,
}

impl ContractViolation {
    fn new(role: &str, rule_name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            role: role.to_owned(),
            rule: rule_name,
            detail: detail.into(),
        }
    }

    /// Convert the violation into a dispatcher-visible tool error.
    #[must_use]
    pub fn into_tool_error(self) -> ToolError {
        ToolError::PermissionDenied(self.to_string())
    }
}

impl Invariant {
    fn check(
        &self,
        role: &str,
        call: &ToolCall,
        ctx: &ToolContext,
    ) -> Result<(), ContractViolation> {
        match self {
            Self::MaxTokensPerTurn(max) => {
                if let Some(estimated_tokens) = estimated_tokens(call)
                    && estimated_tokens > *max
                {
                    return Err(ContractViolation::new(
                        role,
                        "MaxTokensPerTurn",
                        format!("{estimated_tokens} > {max}"),
                    ));
                }
            }
            Self::RequireGateBeforeCommit => {
                if is_commit_like_call(call) && !has_gate_approval(call, ctx) {
                    return Err(ContractViolation::new(
                        role,
                        "RequireGateBeforeCommit",
                        format!("tool `{}` attempted commit without a gate pass", call.name),
                    ));
                }
            }
            Self::NoNetworkAccess => {
                if is_network_like_call(call) {
                    return Err(ContractViolation::new(
                        role,
                        "NoNetworkAccess",
                        format!("tool `{}` requires network access", call.name),
                    ));
                }
            }
        }

        Ok(())
    }
}

impl GovernanceRule {
    fn check(
        &self,
        role: &str,
        call: &ToolCall,
        ctx: &ToolContext,
    ) -> Result<(), ContractViolation> {
        match self {
            Self::MaxToolCallsPerTurn(max) => {
                let observed_calls = u32::try_from(ctx.external_actions.read().len())
                    .unwrap_or(u32::MAX)
                    .saturating_add(1);
                if observed_calls > *max {
                    return Err(ContractViolation::new(
                        role,
                        "MaxToolCallsPerTurn",
                        format!("{observed_calls} > {max}"),
                    ));
                }
            }
            Self::ForbiddenTools(tools) => {
                if tools.iter().any(|tool| tool == &call.name) {
                    return Err(ContractViolation::new(
                        role,
                        "ForbiddenTools",
                        format!("tool `{}` is forbidden for this contract", call.name),
                    ));
                }
            }
            Self::MaxCostPerTurn(_) | Self::MaxConsecutiveFailures(_) => {}
            Self::RequireToolBeforeEdit(required_tool) => {
                if EDIT_TOOLS.contains(&call.name.as_str()) && !has_prior_tool(ctx, required_tool) {
                    return Err(ContractViolation::new(
                        role,
                        "RequireToolBeforeEdit",
                        format!("tool `{required_tool}` must run before `{}`", call.name),
                    ));
                }
            }
        }

        Ok(())
    }
}

impl RecoveryAction {
    fn matches(&self, err: &ToolError) -> bool {
        match self.trigger.as_str() {
            "contract_violation" => matches!(
                err,
                ToolError::PermissionDenied(message) if message.contains("contract violation")
            ),
            "attempted_edit" => matches!(
                err,
                ToolError::PermissionDenied(message)
                    if message.contains("RequireToolBeforeEdit") || message.contains("ForbiddenTools")
            ),
            "tool_budget_exhausted" => matches!(
                err,
                ToolError::Other(message) if message.to_ascii_lowercase().contains("budget")
            ),
            _ => false,
        }
    }

    /// Convert the recovery action into a fail-closed tool error.
    #[must_use]
    pub fn into_tool_error(self, role: &str) -> ToolError {
        ToolError::PermissionDenied(format!(
            "contract recovery for role `{role}` requested {:?} after trigger `{}`",
            self.action, self.trigger
        ))
    }
}

fn contract_asset_path(role: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(CONTRACT_DIR)
        .join(format!("{role}.yaml"))
}

fn validate_role(role: &str) -> Result<(), ContractLoadError> {
    if role.is_empty()
        || !role
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Err(ContractLoadError::InvalidRole {
            role: role.to_owned(),
        });
    }

    Ok(())
}

fn estimated_tokens(call: &ToolCall) -> Option<u32> {
    call.arguments
        .get("estimated_tokens")
        .and_then(as_u32)
        .or_else(|| call.arguments.get("max_tokens").and_then(as_u32))
        .or_else(|| string_token_estimate(call.arguments.get("prompt")))
        .or_else(|| string_token_estimate(call.arguments.get("input")))
}

fn as_u32(value: &serde_json::Value) -> Option<u32> {
    value.as_u64().and_then(|value| u32::try_from(value).ok())
}

fn string_token_estimate(value: Option<&serde_json::Value>) -> Option<u32> {
    let text = value?.as_str()?;
    let chars = text.chars().count();
    let estimate = chars.div_ceil(4);
    u32::try_from(estimate).ok()
}

fn is_commit_like_call(call: &ToolCall) -> bool {
    if call.name.contains("commit") {
        return true;
    }

    call.arguments
        .get("command")
        .and_then(|value| value.as_str())
        .is_some_and(|command| {
            let lower = command.to_ascii_lowercase();
            lower.contains("git commit") || lower.contains("git push")
        })
}

fn is_network_like_call(call: &ToolCall) -> bool {
    if NETWORK_TOOLS.contains(&call.name.as_str()) {
        return true;
    }

    call.arguments
        .get("command")
        .and_then(|value| value.as_str())
        .is_some_and(|command| {
            let lower = command.to_ascii_lowercase();
            lower.contains("curl ")
                || lower.contains("wget ")
                || lower.contains("http://")
                || lower.contains("https://")
        })
}

fn has_gate_approval(call: &ToolCall, ctx: &ToolContext) -> bool {
    if call
        .arguments
        .get("gate_passed")
        .and_then(|value| value.as_bool())
        == Some(true)
    {
        return true;
    }

    if call
        .arguments
        .get("verified")
        .and_then(|value| value.as_bool())
        == Some(true)
    {
        return true;
    }

    ctx.external_actions.read().iter().any(|action| {
        action.action_type == "gate_passed"
            || (action.action_type == "run_gate"
                && action
                    .metadata
                    .get("passed")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false))
    })
}

fn has_prior_tool(ctx: &ToolContext, required_tool: &str) -> bool {
    ctx.external_actions.read().iter().any(|action| {
        action.action_type == required_tool
            || action.service == required_tool
            || action.metadata.get("tool").and_then(|value| value.as_str()) == Some(required_tool)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    use parking_lot::RwLock;
    use roko_core::tool::{
        ExternalAction, NoopAuditSink, NoopMetricsSink, NoopTraceSink, ToolPermission,
    };

    #[test]
    fn permissive_contract_is_empty_except_for_role() {
        let contract = AgentContract::permissive("reviewer");

        assert_eq!(contract.role, "reviewer");
        assert!(contract.invariants.is_empty());
        assert!(contract.governance.is_empty());
        assert!(contract.recovery.is_empty());
    }

    #[test]
    fn bundled_contracts_load_from_assets() {
        let implementer = AgentContract::load_for_role("implementer").expect("load implementer");
        let reviewer = AgentContract::load_for_role("reviewer").expect("load reviewer");
        let researcher = AgentContract::load_for_role("researcher").expect("load researcher");

        assert_eq!(implementer.role, "implementer");
        assert!(matches!(
            implementer.invariants.as_slice(),
            [
                Invariant::MaxTokensPerTurn(_),
                Invariant::RequireGateBeforeCommit,
            ]
        ));

        assert_eq!(reviewer.role, "reviewer");
        assert!(matches!(
            reviewer.invariants.as_slice(),
            [Invariant::NoNetworkAccess]
        ));

        assert_eq!(researcher.role, "researcher");
        assert!(matches!(
            researcher.invariants.as_slice(),
            [Invariant::MaxTokensPerTurn(_)]
        ));
    }

    #[test]
    fn max_tokens_contract_violation_surfaces_permission_error() {
        let contract = AgentContract::load_for_role("implementer").expect("load implementer");
        let call = ToolCall::new(
            "call-1",
            "bash",
            serde_json::json!({
                "command": "echo hi",
                "estimated_tokens": 12_001_u32,
            }),
        );
        let ctx = ToolContext::testing("/tmp/contract-tests");

        let err = contract
            .check_pre_execution(&call, &ctx)
            .expect_err("token budget should be enforced");
        assert_eq!(err.rule, "MaxTokensPerTurn");
        assert!(matches!(
            err.clone().into_tool_error(),
            ToolError::PermissionDenied(message) if message.contains("MaxTokensPerTurn")
        ));
    }

    #[test]
    fn require_tool_before_edit_reads_external_actions() {
        let contract = AgentContract {
            role: "implementer".into(),
            invariants: Vec::new(),
            governance: vec![GovernanceRule::RequireToolBeforeEdit("read_file".into())],
            recovery: Vec::new(),
        };
        let actions = Arc::new(RwLock::new(vec![ExternalAction {
            service: "tool_dispatcher".into(),
            action_type: "read_file".into(),
            resource_id: "src/lib.rs".into(),
            metadata: serde_json::json!({ "tool": "read_file" }),
            performed_at: chrono::Utc::now(),
        }]));
        let ctx = ToolContext::new(
            "/tmp/contract-tests",
            Duration::from_secs(5),
            ToolPermission {
                read: true,
                write: true,
                exec: true,
                git: true,
                network: true,
            },
            Arc::new(NoopAuditSink),
            Arc::new(NoopTraceSink),
            Arc::new(NoopMetricsSink),
            Arc::new(roko_core::tool::NeverCancel),
        )
        .with_external_actions(actions);
        let call = ToolCall::new(
            "call-2",
            "edit_file",
            serde_json::json!({ "path": "src/lib.rs" }),
        );

        assert!(contract.check_pre_execution(&call, &ctx).is_ok());
    }
}
