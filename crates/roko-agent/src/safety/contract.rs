//! Formal behavioral contracts for role-scoped agent governance.
//!
//! These types define invariants, governance rules, and recovery actions that
//! higher-level orchestration can evaluate with a low-latency policy check.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use roko_core::tool::{ExternalAction, ToolCall, ToolContext, ToolError, ToolResult};

const CONTRACT_DIR: &str = "src/safety/contracts";
const NETWORK_TOOLS: &[&str] = &["web_fetch", "web_search"];
const EDIT_TOOLS: &[&str] = &[
    "write_file",
    "edit_file",
    "multi_edit",
    "apply_patch",
    "notebook_edit",
];

/// Process-wide cache of parsed agent contracts, keyed by role name.
///
/// Contract assets are baked into the crate via `env!("CARGO_MANIFEST_DIR")`
/// at build time and never change during a process lifetime. Caching avoids
/// redundant disk reads and JSON parses on every tool dispatch check.
///
/// Uses `RwLock` for thread-safe concurrent reads with exclusive writes on
/// cache misses. Only successful loads are cached; errors always re-read.
static CONTRACT_CACHE: LazyLock<RwLock<HashMap<String, AgentContract>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// How to handle a missing or invalid bundled contract asset.
///
/// Used by [`AgentContract::load_for_role_with_mode`] to choose between a
/// hard error and a deny-everything fallback.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ContractLoadMode {
    /// Treat a missing or malformed asset as a fatal error.
    ///
    /// Callers that prefer to fail fast on bootstrap should use this mode
    /// so the workspace is forced to ship explicit contracts for every role.
    Strict,
    /// Substitute a deny-everything restricted contract when the asset is
    /// missing or malformed.
    ///
    /// The fallback contract has zero allowed tools, no governance, and no
    /// invariants beyond the implicit deny-by-default. This keeps the
    /// dispatcher safe when an unfamiliar role is requested without
    /// breaking the orchestrator.
    #[default]
    RestrictedFallback,
}

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
    /// Optional explicit allowlist of tool names this role may invoke.
    ///
    /// When `Some(_)`, the dispatcher enforces capability intersection: any
    /// dispatch request whose tool is not in this list is rejected before
    /// the handler runs. When `None`, the role is gated only by the
    /// `ForbiddenTools` denylist in `governance`.
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
}

impl AgentContract {
    /// Build a permissive contract for `role`.
    ///
    /// This is retained for tests and adapter shims. New code should prefer
    /// either [`AgentContract::load_for_role`] or
    /// [`AgentContract::restricted`] so missing-role fallbacks fail closed.
    #[must_use]
    pub fn permissive(role: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            ..Self::default()
        }
    }

    /// Build a deny-everything restricted contract for `role`.
    ///
    /// The contract sets `allowed_tools = Some(vec![])` and an empty
    /// `ForbiddenTools` rule (the allowlist intersection is the binding
    /// constraint). Used as the [`ContractLoadMode::RestrictedFallback`]
    /// substitute when no bundled YAML exists for a role.
    #[must_use]
    pub fn restricted(role: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            invariants: vec![Invariant::NoNetworkAccess],
            governance: Vec::new(),
            recovery: Vec::new(),
            allowed_tools: Some(Vec::new()),
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

        // Check cache first (read lock — cheap concurrent path).
        if let Ok(guard) = CONTRACT_CACHE.read() {
            if let Some(cached) = guard.get(role) {
                return Ok(cached.clone());
            }
        }

        // Cache miss: load from disk.
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

        // Store in cache on success (write lock — only on first load per role).
        // Ignore lock poisoning (don't fail the load just because cache is broken).
        if let Ok(mut guard) = CONTRACT_CACHE.write() {
            guard.insert(role.to_owned(), contract.clone());
        }

        Ok(contract)
    }

    /// Load the bundled contract for `role` using `mode` to handle missing
    /// or malformed assets.
    ///
    /// In [`ContractLoadMode::Strict`] mode the underlying load error is
    /// surfaced directly. In [`ContractLoadMode::RestrictedFallback`] mode a
    /// deny-everything contract is substituted and a warning is emitted via
    /// `tracing::warn!`.
    pub fn load_for_role_with_mode(
        role: impl AsRef<str>,
        mode: ContractLoadMode,
    ) -> Result<Self, ContractLoadError> {
        let role_ref = role.as_ref();
        match Self::load_for_role(role_ref) {
            Ok(contract) => Ok(contract),
            Err(err) => match mode {
                ContractLoadMode::Strict => Err(err),
                ContractLoadMode::RestrictedFallback => {
                    tracing::warn!(
                        role = %role_ref,
                        %err,
                        "no contract for role; using restricted (deny-all) fallback"
                    );
                    Ok(Self::restricted(role_ref))
                }
            },
        }
    }

    /// Clear the contract cache. Used in tests to ensure each test loads fresh
    /// contracts without interference from previous test runs.
    ///
    /// Only available in test builds.
    #[cfg(test)]
    pub fn invalidate_contract_cache() {
        if let Ok(mut guard) = CONTRACT_CACHE.write() {
            guard.clear();
        }
    }

    /// Returns `true` if the given tool name is permitted by this contract's
    /// allowlist + denylist intersection.
    ///
    /// - When `allowed_tools` is `Some(_)`, the tool must appear in the
    ///   allowlist *and* must not appear in any `ForbiddenTools` rule.
    /// - When `allowed_tools` is `None`, the tool only needs to avoid the
    ///   `ForbiddenTools` denylist.
    #[must_use]
    pub fn permits_tool(&self, tool_name: &str) -> bool {
        if let Some(ref allowed) = self.allowed_tools {
            if !allowed.iter().any(|allowed_name| allowed_name == tool_name) {
                return false;
            }
        }

        for rule in &self.governance {
            if let GovernanceRule::ForbiddenTools(forbidden) = rule {
                if forbidden.iter().any(|name| name == tool_name) {
                    return false;
                }
            }
        }

        true
    }

    /// Validate this contract against an inbound tool invocation.
    pub fn check_pre_execution(
        &self,
        call: &ToolCall,
        ctx: &ToolContext,
    ) -> Result<(), ContractViolation> {
        // Capability intersection — `allowed_tools` is the binding allowlist
        // when set; reject before invariants/governance run so a denied tool
        // never observes any contract side effects.
        if !self.permits_tool(&call.name) {
            return Err(ContractViolation::new(
                &self.role,
                "AllowedTools",
                format!(
                    "tool `{}` is not in the role's allowed_tools list",
                    call.name
                ),
            ));
        }
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
            Self::MaxCostPerTurn(max) => {
                if let Some(estimated_cost_usd) = estimated_cost_usd(call)
                    && estimated_cost_usd > *max
                {
                    return Err(ContractViolation::new(
                        role,
                        "MaxCostPerTurn",
                        format!("{estimated_cost_usd:.4} > {max:.4}"),
                    ));
                }
                // TODO(UX26): enforce cumulative per-turn spend once tool-cost
                // accounting is threaded into ToolContext.
            }
            Self::MaxConsecutiveFailures(max) => {
                let consecutive = count_trailing_failures(&ctx.external_actions.read());
                if consecutive >= *max {
                    return Err(ContractViolation::new(
                        role,
                        "MaxConsecutiveFailures",
                        format!("{consecutive} consecutive failures >= limit {max}"),
                    ));
                }
            }
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

fn estimated_cost_usd(call: &ToolCall) -> Option<f64> {
    call.arguments
        .get("estimated_cost_usd")
        .and_then(|value| value.as_f64())
        .or_else(|| {
            call.arguments
                .get("cost_usd")
                .and_then(|value| value.as_f64())
        })
        .or_else(|| {
            call.arguments
                .get("estimated_cost_usd_cents")
                .and_then(|value| value.as_u64())
                .and_then(|cents| u32::try_from(cents).ok())
                .map(|cents| f64::from(cents) / 100.0)
        })
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

/// Count trailing consecutive failures from the external actions buffer.
///
/// An action is considered a failure if its metadata contains `"success": false`
/// or `"error": true`, or if its `action_type` contains "error" or "fail".
/// We walk backwards from the most recent action; the first success (or non-failure)
/// resets the count.
fn count_trailing_failures(actions: &[ExternalAction]) -> u32 {
    let mut count: u32 = 0;
    for action in actions.iter().rev() {
        if is_failure_action(action) {
            count = count.saturating_add(1);
        } else {
            break;
        }
    }
    count
}

/// Heuristic: does this external action represent a tool failure?
fn is_failure_action(action: &ExternalAction) -> bool {
    // Explicit success=false marker.
    if action.metadata.get("success").and_then(|v| v.as_bool()) == Some(false) {
        return true;
    }
    // Explicit error=true marker.
    if action.metadata.get("error").and_then(|v| v.as_bool()) == Some(true) {
        return true;
    }
    // action_type heuristic (e.g. "tool_error", "execution_failed").
    let at = action.action_type.to_ascii_lowercase();
    at.contains("error") || at.contains("fail")
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
        AgentContract::invalidate_contract_cache();
        let implementer = AgentContract::load_for_role("implementer").expect("load implementer");
        let reviewer = AgentContract::load_for_role("reviewer").expect("load reviewer");
        let researcher = AgentContract::load_for_role("researcher").expect("load researcher");
        let architect = AgentContract::load_for_role("architect").expect("load architect");
        let auditor = AgentContract::load_for_role("auditor").expect("load auditor");
        let scribe = AgentContract::load_for_role("scribe").expect("load scribe");
        let auto_fixer = AgentContract::load_for_role("auto-fixer").expect("load auto-fixer");

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

        assert_eq!(architect.role, "architect");
        assert!(!architect.governance.is_empty());
        assert_eq!(auditor.role, "auditor");
        assert!(auditor.invariants.contains(&Invariant::NoNetworkAccess));
        assert_eq!(scribe.role, "scribe");
        assert!(scribe.invariants.contains(&Invariant::NoNetworkAccess));
        assert_eq!(auto_fixer.role, "auto-fixer");
        assert!(
            auto_fixer
                .invariants
                .contains(&Invariant::RequireGateBeforeCommit)
        );
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
            allowed_tools: None,
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

    #[test]
    fn max_consecutive_failures_blocks_after_threshold() {
        let contract = AgentContract {
            role: "implementer".into(),
            invariants: Vec::new(),
            governance: vec![GovernanceRule::MaxConsecutiveFailures(3)],
            recovery: Vec::new(),
            allowed_tools: None,
        };

        // Build 3 consecutive failure actions.
        let failure_actions: Vec<ExternalAction> = (0..3)
            .map(|i| ExternalAction {
                service: "tool_dispatcher".into(),
                action_type: format!("tool_call_{i}"),
                resource_id: String::new(),
                metadata: serde_json::json!({ "success": false }),
                performed_at: chrono::Utc::now(),
            })
            .collect();

        let actions = Arc::new(RwLock::new(failure_actions));
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

        let call = ToolCall::new("call-1", "bash", serde_json::json!({"command": "ls"}));
        let err = contract
            .check_pre_execution(&call, &ctx)
            .expect_err("should deny after 3 consecutive failures");
        assert_eq!(err.rule, "MaxConsecutiveFailures");
    }

    #[test]
    fn max_consecutive_failures_resets_on_success() {
        let contract = AgentContract {
            role: "implementer".into(),
            invariants: Vec::new(),
            governance: vec![GovernanceRule::MaxConsecutiveFailures(3)],
            recovery: Vec::new(),
            allowed_tools: None,
        };

        // 2 failures, then 1 success, then 2 more failures = 2 trailing failures.
        let actions = Arc::new(RwLock::new(vec![
            ExternalAction {
                service: "tool_dispatcher".into(),
                action_type: "call_0".into(),
                resource_id: String::new(),
                metadata: serde_json::json!({ "success": false }),
                performed_at: chrono::Utc::now(),
            },
            ExternalAction {
                service: "tool_dispatcher".into(),
                action_type: "call_1".into(),
                resource_id: String::new(),
                metadata: serde_json::json!({ "success": false }),
                performed_at: chrono::Utc::now(),
            },
            ExternalAction {
                service: "tool_dispatcher".into(),
                action_type: "call_2".into(),
                resource_id: String::new(),
                metadata: serde_json::json!({ "success": true }),
                performed_at: chrono::Utc::now(),
            },
            ExternalAction {
                service: "tool_dispatcher".into(),
                action_type: "call_3".into(),
                resource_id: String::new(),
                metadata: serde_json::json!({ "success": false }),
                performed_at: chrono::Utc::now(),
            },
            ExternalAction {
                service: "tool_dispatcher".into(),
                action_type: "call_4".into(),
                resource_id: String::new(),
                metadata: serde_json::json!({ "success": false }),
                performed_at: chrono::Utc::now(),
            },
        ]));
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

        let call = ToolCall::new("call-5", "bash", serde_json::json!({"command": "ls"}));
        // Only 2 trailing failures, limit is 3, so this should pass.
        assert!(contract.check_pre_execution(&call, &ctx).is_ok());
    }

    #[test]
    fn count_trailing_failures_with_error_action_type() {
        let actions = vec![
            ExternalAction {
                service: "tool_dispatcher".into(),
                action_type: "read_file".into(),
                resource_id: String::new(),
                metadata: serde_json::json!({}),
                performed_at: chrono::Utc::now(),
            },
            ExternalAction {
                service: "tool_dispatcher".into(),
                action_type: "tool_error".into(),
                resource_id: String::new(),
                metadata: serde_json::json!({}),
                performed_at: chrono::Utc::now(),
            },
            ExternalAction {
                service: "tool_dispatcher".into(),
                action_type: "execution_failed".into(),
                resource_id: String::new(),
                metadata: serde_json::json!({}),
                performed_at: chrono::Utc::now(),
            },
        ];
        assert_eq!(super::count_trailing_failures(&actions), 2);
    }

    #[test]
    fn allowed_tools_blocks_disallowed_call_in_check_pre_execution() {
        let contract = AgentContract {
            role: "auditor".into(),
            invariants: Vec::new(),
            governance: Vec::new(),
            recovery: Vec::new(),
            allowed_tools: Some(vec!["read_file".into(), "grep".into()]),
        };
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
        );

        // `write_file` is not in the allowlist — must be rejected.
        let call = ToolCall::new("c1", "write_file", serde_json::json!({}));
        let err = contract
            .check_pre_execution(&call, &ctx)
            .expect_err("write_file must be blocked");
        assert_eq!(err.rule, "AllowedTools");
        assert!(err.detail.contains("write_file"));

        // `read_file` is in the allowlist — must pass.
        let call = ToolCall::new("c2", "read_file", serde_json::json!({"path": "."}));
        contract.check_pre_execution(&call, &ctx).unwrap();
    }

    #[test]
    fn restricted_contract_denies_every_tool() {
        let contract = AgentContract::restricted("unknown");
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
        );
        for tool in ["read_file", "write_file", "grep", "bash"] {
            let call = ToolCall::new("x", tool, serde_json::json!({}));
            assert!(
                contract.check_pre_execution(&call, &ctx).is_err(),
                "restricted contract should deny `{tool}`",
            );
        }
    }

    #[test]
    fn load_for_role_with_mode_strict_errors_on_missing() {
        AgentContract::invalidate_contract_cache();
        let err =
            AgentContract::load_for_role_with_mode("totally-not-a-role", ContractLoadMode::Strict)
                .expect_err("missing role must error in strict mode");
        match err {
            ContractLoadError::MissingAsset { .. } => {}
            other => panic!("expected MissingAsset, got {other:?}"),
        }
    }

    #[test]
    fn load_for_role_with_mode_fallback_returns_restricted() {
        AgentContract::invalidate_contract_cache();
        let contract = AgentContract::load_for_role_with_mode(
            "totally-not-a-role",
            ContractLoadMode::RestrictedFallback,
        )
        .expect("fallback contract must load");
        assert_eq!(contract.role, "totally-not-a-role");
        assert_eq!(contract.allowed_tools.as_deref(), Some(&[][..]));
        assert!(!contract.permits_tool("read_file"));
    }
}
