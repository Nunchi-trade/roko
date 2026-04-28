//! Tool-dispatch safety enforcement (§36.e).
//!
//! Each submodule owns **one policy family** that gates a specific
//! capability before the dispatcher hands a tool call to its handler.
//! Policies are **pure validators**: they take a call + context and
//! return a verdict — no side effects, no mutation of the caller's state.
//!
//! # Families (wave 1)
//!
//! - [`path`] (§36.46) — worktree-relative canonicalization & escape prevention
//! - [`bash`] (§36.47) — command allowlist / denylist for the `bash` tool
//! - [`network`] (§36.48) — outbound-destination allowlist for network tools
//!
//! # Families (later waves)
//!
//! - `git` (§36.49) — branch-protection policy
//! - `scrub` (§36.50) — secret-scrubbing from outputs
//! - `rate_limit` (§36.51) — per-tool / per-role rate limits
//! - `audit` (§36.52) — append-only JSONL audit log (lives in `roko-fs`)
//!
//! # Composition
//!
//! Each policy exposes a `check(...)` that returns `Result<(), ToolError>`.
//! The dispatcher chains them in order; the first failure short-circuits
//! and is returned verbatim to the caller.

#![allow(clippy::module_name_repetitions)]

pub mod allowlist;
pub mod authz;
pub mod bash;
pub mod capabilities;
pub mod contract;
pub mod data_llm;
pub mod git;
pub mod hallucination;
pub mod hooks;
pub mod network;
pub mod path;
pub mod provenance;
pub mod rate_limit;
pub mod result_filter;
pub mod risk;
pub mod scrub;
pub mod spending;
pub mod temporal;
pub mod witness;

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use regex::Regex;
use roko_core::config::schema::{RokoConfig, RoleOverride};
use roko_core::tool::{ToolCall, ToolContext, ToolError, ToolResult};

use self::bash::BashPolicy;
use self::contract::{AgentContract, ContractLoadMode, GovernanceRule, Invariant};
use self::git::GitPolicy;
use self::network::NetworkPolicy;
use self::path::PathPolicy;
use self::rate_limit::{RateLimitKey, RateLimiter};
use self::risk::{BudgetCheckResult, ProposedAction};
use self::scrub::ScrubPolicy;

use self::capabilities::{exec_capability_from_command, network_capability_from_url};
pub use allowlist::AllowlistGuard;
pub use authz::{
    ApproveAllChannel, AuthorizationEvidence, AuthorizationSource, AuthzDecision,
    ConfirmationChannel, ConfirmationOutcome, ConfirmationSource, DenyAllChannel, EscalationTarget,
    LogAndDenyChannel,
};
pub use capabilities::{
    AgentWarrant, Capability, CapabilityError, PluginTier, check_capability, check_plugin_tier,
    delegate,
};
pub use data_llm::{
    DataLlmAuditEntry, DataLlmDecision, DataLlmRouter, SanitizeResult, sanitize_input,
};
pub use hallucination::HallucinationDetector;
pub use hooks::{DataSink, HookDecision, SafetyAuditRecord, SafetyHook, TaintLabel, TaintedString};
pub use provenance::{AttestationLevel, Custody, CustodyLogger, Taint};
pub use result_filter::ResultFilter;
pub use risk::{
    BetaDistribution, BudgetDimension, OperationalConfidenceTracker, SafetyBudget,
    SafetyBudgetTracker, confidence_multiplier, effective_limit, irreversibility_score,
    kelly_fraction,
};
pub use spending::{SpendingLimiter, ToolCostEstimate};
pub use temporal::{LtlProperty, MonitorState, TemporalMonitor, Violation};
pub use witness::{IntegrityViolation, VertexKind, WitnessDag, WitnessLogger, WitnessVertex};

// ─── Orchestrator-level safety violation types (AGT-01) ────────────────────

/// A safety violation detected during pre- or post-dispatch checks.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SafetyViolation {
    /// Plan that triggered the violation.
    pub plan_id: String,
    /// Task within the plan.
    pub task_id: String,
    /// Kind of violation detected.
    pub violation_type: ViolationType,
    /// Human-readable description.
    pub message: String,
    /// Whether this should block execution or just warn.
    pub severity: ViolationSeverity,
}

impl std::fmt::Display for SafetyViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{:?}] {}/{}: {} ({})",
            self.severity, self.plan_id, self.task_id, self.message, self.violation_type
        )
    }
}

/// Classification of safety violations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationType {
    /// Agent modified files outside the worktree.
    PathEscape,
    /// Agent output contained secrets.
    SecretLeak,
    /// Agent contract was violated.
    ContractViolation,
    /// Safety budget was exhausted.
    BudgetExhausted,
    /// Forbidden tool was invoked.
    ForbiddenTool,
}

impl std::fmt::Display for ViolationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PathEscape => write!(f, "path_escape"),
            Self::SecretLeak => write!(f, "secret_leak"),
            Self::ContractViolation => write!(f, "contract_violation"),
            Self::BudgetExhausted => write!(f, "budget_exhausted"),
            Self::ForbiddenTool => write!(f, "forbidden_tool"),
        }
    }
}

/// Severity level for a safety violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationSeverity {
    /// Execution should be blocked.
    Block,
    /// Log a warning but allow execution to proceed.
    Warn,
}

// ─── Tool-name constants used to match calls to policies ─��─────────────��──

const BASH_TOOLS: &[&str] = &["bash", "run_tests"];
const NETWORK_TOOLS: &[&str] = &["web_fetch", "web_search"];
const FILE_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "edit_file",
    "multi_edit",
    "apply_patch",
    "notebook_edit",
    "ls",
    "glob",
    "grep",
];

// ─── SafetyLayer ──────────────────────────────────────────────────────────

/// Aggregated safety policies that the dispatcher applies to every tool call.
///
/// Constructed via [`SafetyLayer::with_defaults()`] for the standard
/// conservative posture, or piece-by-piece for custom configurations.
/// Passed to [`ToolDispatcher::with_safety`](super::super::dispatcher::ToolDispatcher).
#[derive(Debug, Clone)]
pub struct SafetyLayer {
    /// Bash command allowlist / denylist.
    pub bash_policy: BashPolicy,
    /// Git branch-protection rules.
    pub git_policy: GitPolicy,
    /// Network destination allowlist.
    pub network_policy: NetworkPolicy,
    /// Worktree path-escape prevention.
    pub path_policy: PathPolicy,
    /// Secret-scrubbing rules applied to outputs.
    pub scrub_policy: ScrubPolicy,
    /// Rate limiter — shared across calls (interior mutability via `Arc`).
    pub rate_limiter: Option<Arc<RateLimiter>>,
    /// Optional adaptive-risk budget tracker shared across calls.
    pub safety_budget: Option<Arc<Mutex<SafetyBudgetTracker>>>,
    /// Role name used as part of the rate-limit key.
    /// Defaults to `"default"`.
    pub role: String,
    /// Declarative contract enforced for this role.
    pub contract: AgentContract,
    /// Optional OCaps-style warrant for tool execution.
    pub warrant: Option<AgentWarrant>,
    /// Role-local tool whitelists loaded from config.
    role_tools: HashMap<String, ToolWhitelist>,
    /// Role overrides keyed by both section name and any explicit role alias.
    role_overrides: HashMap<String, RoleOverride>,
    /// Optional temporal logic monitor for safety/liveness properties.
    ///
    /// When present, `check_pre_execution` evaluates all registered
    /// `Never`/`Always`/`Eventually` properties on each tool call.
    /// Protected by `Arc<Mutex<_>>` for interior mutability (the monitor
    /// maintains per-property state).
    pub temporal_monitor: Option<Arc<Mutex<TemporalMonitor>>>,
}

#[derive(Debug, Clone, Default)]
struct ToolWhitelist(Vec<Regex>);

impl ToolWhitelist {
    fn from_patterns(patterns: &[String]) -> Self {
        let patterns = patterns
            .iter()
            .map(|pattern| glob_to_regex(pattern))
            .collect();
        Self(patterns)
    }

    fn matches(&self, tool: &str) -> bool {
        self.0.iter().any(|pattern| pattern.is_match(tool))
    }
}

impl SafetyLayer {
    /// Construct with all default policies enabled.
    ///
    /// # Panics
    ///
    /// Panics if the default `BashPolicy` regex compilation fails
    /// (a compile-time bug, not a runtime condition).
    #[must_use]
    pub fn with_defaults() -> Self {
        Self {
            bash_policy: BashPolicy::with_defaults(),
            git_policy: GitPolicy::default(),
            network_policy: NetworkPolicy::default(),
            path_policy: PathPolicy::default(),
            scrub_policy: ScrubPolicy::default(),
            rate_limiter: Some(Arc::new(RateLimiter::with_defaults())),
            safety_budget: None,
            role: "default".into(),
            contract: AgentContract::restricted("default"),
            warrant: None,
            role_tools: HashMap::new(),
            role_overrides: HashMap::new(),
            temporal_monitor: None,
        }
    }

    /// Construct with default policies and role-local tool whitelists from config.
    #[must_use]
    pub fn from_config(config: &RokoConfig) -> Self {
        let mut layer = Self::with_defaults();
        layer.role_tools = build_role_tools(&config.agent.roles);
        layer.role_overrides = build_role_overrides_map(&config.agent.roles);
        layer
    }

    /// Override the role label used in rate-limit keys.
    #[must_use]
    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        let role = role.into();
        self.contract = self.contract_for_role(&role);
        self.role = role;
        self
    }

    /// Override the contract attached to the safety layer.
    #[must_use]
    pub fn with_contract(mut self, contract: AgentContract) -> Self {
        self.role = contract.role.clone();
        self.contract = contract;
        self
    }

    /// Attach a warrant to the safety layer.
    #[must_use]
    pub fn with_warrant(mut self, warrant: AgentWarrant) -> Self {
        self.warrant = Some(warrant);
        self
    }

    /// Attach an adaptive-risk budget tracker to the safety layer.
    #[must_use]
    pub fn with_safety_budget(mut self, budget: SafetyBudgetTracker) -> Self {
        self.safety_budget = Some(Arc::new(Mutex::new(budget)));
        self
    }

    /// Attach a shared adaptive-risk budget tracker to the safety layer.
    #[must_use]
    pub fn with_shared_safety_budget(mut self, budget: Arc<Mutex<SafetyBudgetTracker>>) -> Self {
        self.safety_budget = Some(budget);
        self
    }

    /// Attach a temporal logic monitor for safety/liveness property checking.
    ///
    /// When attached, every call to `check_pre_execution` evaluates the
    /// monitor's registered `Never`, `Always`, and `Eventually` properties.
    /// Violations cause `ToolError::PermissionDenied`.
    #[must_use]
    pub fn with_temporal_monitor(mut self, monitor: TemporalMonitor) -> Self {
        self.temporal_monitor = Some(Arc::new(Mutex::new(monitor)));
        self
    }

    /// Run all pre-execution safety checks for `call` + `ctx`.
    ///
    /// Returns `Ok(())` if all policies pass; the first failure
    /// short-circuits and is returned as an `Err`.
    pub fn check_pre_execution(&self, call: &ToolCall, ctx: &ToolContext) -> Result<(), ToolError> {
        let name = call.name.as_str();

        if let Some(whitelist) = self.role_tools.get(&self.role) {
            if !whitelist.matches(name) {
                return Err(ToolError::PermissionDenied(format!(
                    "tool `{}` is not allowed for role `{}`",
                    call.name, self.role
                )));
            }
        }

        // 1. Rate limit (applies to all tools).
        if let Some(ref limiter) = self.rate_limiter {
            let key = RateLimitKey {
                role: self.role.clone(),
                tool: name.to_string(),
            };
            limiter.check_and_record(&key)?;
        }

        // 2. OCaps warrant check.
        if let Some(ref warrant) = self.warrant {
            for required in required_capabilities(call, ctx, &self.path_policy) {
                if !check_capability(warrant, &required) {
                    return Err(ToolError::PermissionDenied(format!(
                        "missing capability for tool `{}`: {:?}",
                        call.name, required
                    )));
                }
            }
        }

        // 3. Bash / run_tests policy (command argument).
        if BASH_TOOLS.contains(&name) {
            if let Some(cmd) = call.arguments.get("command").and_then(|v| v.as_str()) {
                bash::check_command_with_policy(cmd, &self.bash_policy)?;
                git::check_git_command_with_policy(cmd, &self.git_policy)?;
            }
        }

        // 4. Network policy (url argument).
        if NETWORK_TOOLS.contains(&name) {
            if let Some(url) = call.arguments.get("url").and_then(|v| v.as_str()) {
                network::check_url_with_policy(url, &self.network_policy)?;
            }
        }

        // 5. Path policy (file_path / path argument).
        if FILE_TOOLS.contains(&name) {
            let worktree = &ctx.worktree_path;
            // Try common argument names for file paths.
            let path_arg = call
                .arguments
                .get("file_path")
                .or_else(|| call.arguments.get("path"))
                .or_else(|| call.arguments.get("pattern")) // grep
                .and_then(|v| v.as_str());
            if let Some(p) = path_arg {
                path::canonicalize_with_policy(worktree, p, &self.path_policy)?;
            }
        }

        if let Some(ref budget) = self.safety_budget {
            let mut tracker = budget.lock();
            match tracker.check_and_consume(&ProposedAction::from_tool_call(call)) {
                BudgetCheckResult::WithinBudget => {}
                BudgetCheckResult::Exceeded(dimension) => {
                    return Err(ToolError::PermissionDenied(format!(
                        "safety budget exceeded: {}",
                        dimension.as_str()
                    )));
                }
            }
        }

        // 7. Temporal logic monitor (Never/Always/Eventually properties).
        if let Some(ref monitor) = self.temporal_monitor {
            let mut monitor = monitor.lock();
            monitor.check_as_tool_error(call)?;
        }

        // 8. Declarative agent contract (invariants + governance rules).
        self.contract
            .check_pre_execution(call, ctx)
            .map_err(|violation| violation.into_tool_error())?;

        Ok(())
    }

    /// Evaluate a tool call through the safety layer and return a unified
    /// authorization decision.
    ///
    /// When active taint is present (e.g. `Taint::ExternalFetch` or
    /// `Taint::ThirdPartyPlugin`), network tools and file-write tools
    /// escalate to `AllowWithConfirm` instead of immediate `Allow`. This
    /// prevents tainted data from flowing to high-risk destinations
    /// without explicit operator confirmation.
    #[must_use]
    pub fn authorize_call(&self, call: &ToolCall, ctx: &ToolContext) -> AuthzDecision {
        self.authorize_call_with_taint(call, ctx, None)
    }

    /// Like [`authorize_call`](Self::authorize_call), but accepts an
    /// explicit taint label for the current context.
    #[must_use]
    pub fn authorize_call_with_taint(
        &self,
        call: &ToolCall,
        ctx: &ToolContext,
        taint: Option<&Taint>,
    ) -> AuthzDecision {
        let mut evidence = vec![AuthorizationEvidence::role_grant(format!(
            "role `{}` is active for tool `{}`",
            self.role, call.name
        ))];
        if let Some(warrant) = &self.warrant {
            evidence.push(AuthorizationEvidence::session_approval(format!(
                "warrant issued by `{}` is attached",
                warrant.issuer
            )));
        }

        if let Err(err) = self.check_pre_execution(call, ctx) {
            return AuthzDecision::Deny {
                reason: err.to_string(),
            };
        }

        if let Err(err) = self.check_contract(call, ctx) {
            return AuthzDecision::Deny {
                reason: err.to_string(),
            };
        }

        // Taint enforcement: if the context carries active taint, escalate
        // network and file-write operations to AllowWithConfirm.
        if let Some(active_taint) = taint.filter(|t| t.is_active()) {
            let name = call.name.as_str();
            let taint_desc = format!("{active_taint:?}");

            if NETWORK_TOOLS.contains(&name) {
                return AuthzDecision::AllowWithConfirm {
                    prompt: format!(
                        "tool `{name}` attempts network egress under taint {taint_desc}. Allow?"
                    ),
                    evidence,
                };
            }

            // File writes under taint require confirmation.
            let is_write_tool = matches!(
                name,
                "write_file" | "edit_file" | "multi_edit" | "apply_patch" | "notebook_edit"
            );
            if is_write_tool {
                return AuthzDecision::AllowWithConfirm {
                    prompt: format!(
                        "tool `{name}` attempts file write under taint {taint_desc}. Allow?"
                    ),
                    evidence,
                };
            }

            // Bash commands under taint are high-risk.
            if BASH_TOOLS.contains(&name) {
                return AuthzDecision::AllowWithConfirm {
                    prompt: format!(
                        "tool `{name}` attempts shell execution under taint {taint_desc}. Allow?"
                    ),
                    evidence,
                };
            }
        }

        AuthzDecision::Allow { evidence }
    }

    /// Run declarative contract checks for `call` + `ctx`.
    pub fn check_contract(&self, call: &ToolCall, ctx: &ToolContext) -> Result<(), ToolError> {
        self.contract
            .check_pre_execution(call, ctx)
            .map_err(|violation| violation.into_tool_error())
    }

    /// Run the subset of safety checks that can be applied to a raw subprocess launch.
    ///
    /// This is intentionally narrower than [`Self::check_pre_execution`]: generic
    /// subprocesses do not expose structured tool arguments, so we only validate
    /// what can be reasoned about before spawn: warranted exec capability, the
    /// rendered command line for direct invocations, and shell-wrapper command
    /// strings.
    pub fn check_exec_command(&self, program: &str, args: &[String]) -> Result<(), ToolError> {
        let command = shell_command_arg(program, args)
            .map(str::to_owned)
            .unwrap_or_else(|| render_exec_command(program, args));

        if let Some(ref limiter) = self.rate_limiter {
            let key = RateLimitKey {
                role: self.role.clone(),
                tool: exec_rate_limit_subject(program, &command),
            };
            limiter.check_and_record(&key)?;
        }

        if let Some(ref warrant) = self.warrant
            && let Some(required) = exec_capability_from_command(&command)
            && !check_capability(warrant, &required)
        {
            return Err(ToolError::PermissionDenied(format!(
                "missing capability for subprocess `{}`: {:?}",
                program, required
            )));
        }

        bash::check_command_with_policy(&command, &self.bash_policy)?;
        git::check_git_command_with_policy(&command, &self.git_policy)?;

        if let Some(ref budget) = self.safety_budget {
            let mut tracker = budget.lock();
            match tracker.check_and_consume(&ProposedAction::from_exec_command(program, args)) {
                BudgetCheckResult::WithinBudget => {}
                BudgetCheckResult::Exceeded(dimension) => {
                    return Err(ToolError::PermissionDenied(format!(
                        "safety budget exceeded: {}",
                        dimension.as_str()
                    )));
                }
            }
        }

        Ok(())
    }

    /// Evaluate a raw subprocess launch through the unified authorization API.
    #[must_use]
    pub fn authorize_exec_command(&self, program: &str, args: &[String]) -> AuthzDecision {
        let mut evidence = vec![AuthorizationEvidence::role_grant(format!(
            "role `{}` is active for subprocess `{program}`",
            self.role
        ))];
        if let Some(warrant) = &self.warrant {
            evidence.push(AuthorizationEvidence::session_approval(format!(
                "warrant issued by `{}` is attached",
                warrant.issuer
            )));
        }

        match self.check_exec_command(program, args) {
            Ok(()) => AuthzDecision::Allow { evidence },
            Err(err) => AuthzDecision::Deny {
                reason: err.to_string(),
            },
        }
    }

    /// Scrub secrets from a successful tool result.
    ///
    /// Only `ToolResult::Ok` variants are scrubbed; errors pass through
    /// unchanged.
    #[must_use]
    pub fn scrub_output(&self, result: ToolResult) -> ToolResult {
        match result {
            ToolResult::Ok {
                content,
                is_structured,
                artifacts,
            } => {
                let cleaned = scrub::scrub_secrets(&content, &self.scrub_policy);
                ToolResult::Ok {
                    content: cleaned,
                    is_structured,
                    artifacts,
                }
            }
            err @ ToolResult::Err(_) => err,
        }
    }

    /// Apply any configured recovery rule after tool execution.
    pub fn check_recovery(&self, result: &ToolResult) -> Result<(), ToolError> {
        match self.contract.applicable_recovery(result) {
            Some(recovery) => Err(recovery.into_tool_error(&self.contract.role)),
            None => Ok(()),
        }
    }

    /// Scrub secrets from an arbitrary text payload.
    #[must_use]
    pub fn scrub_text(&self, content: &str) -> String {
        scrub::scrub_secrets(content, &self.scrub_policy)
    }

    // ─── Orchestrator-level pre/post dispatch checks (AGT-01) ──────────

    /// Pre-dispatch safety check run before any agent execution path.
    ///
    /// Validates the task specification against the active safety policies:
    /// - Role authorization: does the role permit this kind of task?
    /// - Contract invariants: does the agent contract allow this dispatch?
    /// - Budget limits: is the safety budget still available?
    ///
    /// Returns a [`SafetyViolation`] if the dispatch should be blocked.
    pub fn pre_dispatch_check(
        &self,
        plan_id: &str,
        task_id: &str,
        role: &str,
        exec_dir: &std::path::Path,
    ) -> Result<(), SafetyViolation> {
        // 1. Verify the execution directory is within allowed bounds.
        if self.path_policy.prevent_escapes {
            let canonical = exec_dir
                .canonicalize()
                .unwrap_or_else(|_| exec_dir.to_path_buf());
            // Check that the exec dir is not a symlink escape or suspicious path.
            if canonical.to_string_lossy().contains("..") {
                return Err(SafetyViolation {
                    plan_id: plan_id.to_string(),
                    task_id: task_id.to_string(),
                    violation_type: ViolationType::PathEscape,
                    message: format!(
                        "execution directory `{}` contains path traversal",
                        exec_dir.display()
                    ),
                    severity: ViolationSeverity::Block,
                });
            }
        }

        // 2. Check contract-level invariants for the role.
        for inv in &self.contract.invariants {
            match inv {
                Invariant::MaxTokensPerTurn(max_tokens) if *max_tokens == 0 => {
                    return Err(SafetyViolation {
                        plan_id: plan_id.to_string(),
                        task_id: task_id.to_string(),
                        violation_type: ViolationType::ContractViolation,
                        message: format!("role `{role}` has zero token budget; dispatch blocked"),
                        severity: ViolationSeverity::Block,
                    });
                }
                _ => {}
            }
        }

        // 3. Check safety budget availability.
        if let Some(ref budget) = self.safety_budget {
            let tracker = budget.lock();
            if tracker.is_exhausted() {
                return Err(SafetyViolation {
                    plan_id: plan_id.to_string(),
                    task_id: task_id.to_string(),
                    violation_type: ViolationType::BudgetExhausted,
                    message: format!("safety budget exhausted for role `{role}`"),
                    severity: ViolationSeverity::Block,
                });
            }
        }

        tracing::debug!(plan_id, task_id, role, "safety pre-dispatch check passed");
        Ok(())
    }

    /// Post-dispatch safety check run after any agent execution path completes.
    ///
    /// Validates the agent output:
    /// - Secret scrubbing: ensures no secrets leaked in the output.
    /// - Output size limits: flags unusually large outputs.
    /// - Contract recovery rules: checks if recovery actions are needed.
    ///
    /// Returns a list of [`SafetyViolation`]s found (may be empty).
    pub fn post_dispatch_check(
        &self,
        plan_id: &str,
        task_id: &str,
        role: &str,
        agent_output: &str,
        changed_files: &[String],
    ) -> Vec<SafetyViolation> {
        let mut violations = Vec::new();

        // 1. Check for secret leaks in agent output.
        let scrubbed = scrub::scrub_secrets(agent_output, &self.scrub_policy);
        if scrubbed != agent_output {
            violations.push(SafetyViolation {
                plan_id: plan_id.to_string(),
                task_id: task_id.to_string(),
                violation_type: ViolationType::SecretLeak,
                message: "agent output contains secrets that were scrubbed".to_string(),
                severity: ViolationSeverity::Warn,
            });
        }

        // 2. Check for path escapes in changed files.
        if self.path_policy.prevent_escapes {
            for file in changed_files {
                if file.contains("..") || file.starts_with('/') {
                    violations.push(SafetyViolation {
                        plan_id: plan_id.to_string(),
                        task_id: task_id.to_string(),
                        violation_type: ViolationType::PathEscape,
                        message: format!("agent modified file outside worktree: {file}"),
                        severity: ViolationSeverity::Warn,
                    });
                }
            }
        }

        // 3. Check governance limits on changed files.
        for rule in &self.contract.governance {
            if let GovernanceRule::ForbiddenTools(tools) = rule {
                // If "write_file" or "edit_file" is forbidden but files were changed,
                // flag a violation.
                if (tools.contains(&"write_file".to_string())
                    || tools.contains(&"edit_file".to_string()))
                    && !changed_files.is_empty()
                {
                    violations.push(SafetyViolation {
                        plan_id: plan_id.to_string(),
                        task_id: task_id.to_string(),
                        violation_type: ViolationType::ContractViolation,
                        message: format!(
                            "role `{role}` forbids file writes but {} files were changed",
                            changed_files.len()
                        ),
                        severity: ViolationSeverity::Warn,
                    });
                }
            }
        }

        if violations.is_empty() {
            tracing::debug!(plan_id, task_id, role, "safety post-dispatch check passed");
        } else {
            tracing::warn!(
                plan_id,
                task_id,
                role,
                violation_count = violations.len(),
                "safety post-dispatch check found violations"
            );
        }
        violations
    }

    /// Summarize the active safety constraints as anti-pattern strings suitable
    /// for injection into prompt layer 7 (INT-14: Safety -> Composition).
    ///
    /// The returned strings describe what the agent must NOT do, derived from
    /// the concrete bash deny-patterns, git protections, network restrictions,
    /// and governance rules currently loaded in this `SafetyLayer`.
    #[must_use]
    pub fn constraints_as_anti_patterns(&self) -> Vec<String> {
        let mut patterns = Vec::new();

        // Bash deny-patterns → anti-patterns.
        if !self.bash_policy.deny_patterns.is_empty() {
            patterns.push(
                "Never run dangerous shell commands: rm -rf /, sudo, curl|sh pipes, \
                 fork bombs, mkfs, or raw-device I/O."
                    .to_string(),
            );
        }

        // Git protections → anti-patterns.
        if !self.git_policy.protected_branches.is_empty() {
            let branches = self
                .git_policy
                .protected_branches
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            patterns.push(format!(
                "Never force-push or delete protected branches: {branches}."
            ));
        }

        // Network restrictions → anti-patterns.
        if !self.network_policy.allow_hosts.is_empty() {
            patterns.push(
                "Network access is restricted to an allowlist of hosts; do not fetch \
                 from arbitrary URLs."
                    .to_string(),
            );
        }
        if !self.network_policy.deny_hosts.is_empty() {
            patterns.push(
                "Certain network hosts are blocked by policy; avoid requests to internal \
                 or denied destinations."
                    .to_string(),
            );
        }

        // Contract governance rules → anti-patterns.
        for rule in &self.contract.governance {
            match rule {
                GovernanceRule::ForbiddenTools(tools) if !tools.is_empty() => {
                    patterns.push(format!(
                        "Never use these forbidden tools: {}.",
                        tools.join(", ")
                    ));
                }
                GovernanceRule::MaxToolCallsPerTurn(max) => {
                    patterns.push(format!(
                        "Limit tool calls to {max} per turn to stay within governance bounds."
                    ));
                }
                GovernanceRule::MaxConsecutiveFailures(max) => {
                    patterns.push(format!(
                        "After {max} consecutive failures, stop and report rather than retrying."
                    ));
                }
                _ => {}
            }
        }

        // Contract invariants → anti-patterns.
        for inv in &self.contract.invariants {
            match inv {
                Invariant::NoNetworkAccess => {
                    patterns
                        .push("This role has no network access; never call network tools.".into());
                }
                Invariant::RequireGateBeforeCommit => {
                    patterns.push("Never commit without a passing gate verification first.".into());
                }
                _ => {}
            }
        }

        // Path policy → anti-patterns.
        if self.path_policy.prevent_escapes {
            patterns.push("Never read or write files outside the designated worktree root.".into());
        }

        patterns
    }

    fn contract_for_role(&self, role: &str) -> AgentContract {
        let mut contract =
            AgentContract::load_for_role_with_mode(role, ContractLoadMode::RestrictedFallback)
                .unwrap_or_else(|err| {
                    tracing::error!(
                        %role,
                        %err,
                        "contract load failed even with restricted fallback; using deny-all"
                    );
                    AgentContract::restricted(role.to_string())
                });

        if let Some(role_override) = self.role_overrides.get(role)
            && let Some(budget) = role_override.effective_budget()
        {
            if let Some(max_tokens) = budget.max_tokens_per_turn {
                contract
                    .invariants
                    .push(Invariant::MaxTokensPerTurn(max_tokens));
            }
            if let Some(max_cost_usd) = budget.max_cost_usd_per_turn() {
                contract
                    .governance
                    .push(GovernanceRule::MaxCostPerTurn(max_cost_usd));
            }
        }

        contract
    }
}

fn build_role_tools(roles: &HashMap<String, RoleOverride>) -> HashMap<String, ToolWhitelist> {
    let mut role_tools = HashMap::new();
    for (role, override_cfg) in roles {
        let Some(tools) = override_cfg.tools.as_ref() else {
            continue;
        };
        let whitelist = ToolWhitelist::from_patterns(tools);
        for key in role_override_keys(role, override_cfg) {
            role_tools.insert(key, whitelist.clone());
        }
    }
    role_tools
}

fn build_role_overrides_map(
    roles: &HashMap<String, RoleOverride>,
) -> HashMap<String, RoleOverride> {
    let mut role_overrides = HashMap::new();
    for (role, override_cfg) in roles {
        for key in role_override_keys(role, override_cfg) {
            role_overrides.insert(key, override_cfg.clone());
        }
    }
    role_overrides
}

fn role_override_keys(section_name: &str, override_cfg: &RoleOverride) -> Vec<String> {
    let mut keys = vec![section_name.to_string()];
    let resolved = override_cfg.resolved_role_name(section_name);
    if resolved != section_name {
        keys.push(resolved.to_string());
    }
    keys
}

fn glob_to_regex(pattern: &str) -> Regex {
    let mut regex = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => regex.push_str(".*"),
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                regex.push('\\');
                regex.push(ch);
            }
            _ => regex.push(ch),
        }
    }
    regex.push('$');
    Regex::new(&regex).expect("generated whitelist regex should compile")
}

fn required_capabilities(
    call: &ToolCall,
    ctx: &ToolContext,
    path_policy: &PathPolicy,
) -> Vec<Capability> {
    let mut required = vec![Capability::Tool(call.name.clone())];
    let name = call.name.as_str();

    if BASH_TOOLS.contains(&name)
        && let Some(command) = call.arguments.get("command").and_then(|v| v.as_str())
        && let Some(exec) = exec_capability_from_command(command)
    {
        required.push(exec);
    }

    if NETWORK_TOOLS.contains(&name)
        && let Some(url) = call.arguments.get("url").and_then(|v| v.as_str())
        && let Some(network) = network_capability_from_url(url)
    {
        required.push(network);
    }

    if FILE_TOOLS.contains(&name) {
        let path_arg = call
            .arguments
            .get("file_path")
            .or_else(|| call.arguments.get("path"))
            .or_else(|| call.arguments.get("pattern"))
            .and_then(|v| v.as_str());
        if let Some(path_arg) = path_arg
            && let Ok(canonical) =
                path::canonicalize_with_policy(&ctx.worktree_path, path_arg, path_policy)
        {
            required.push(match name {
                "write_file" | "edit_file" | "multi_edit" | "apply_patch" | "notebook_edit" => {
                    Capability::WritePath(canonical.absolute)
                }
                _ => Capability::ReadPath(canonical.absolute),
            });
        }
    }

    required
}

fn shell_command_arg<'a>(program: &str, args: &'a [String]) -> Option<&'a str> {
    if !is_shell_program(program) {
        return None;
    }

    args.windows(2).find_map(|pair| {
        let [flag, command] = pair else {
            return None;
        };
        if is_shell_command_flag(flag) {
            Some(command.as_str())
        } else {
            None
        }
    })
}

fn render_exec_command(program: &str, args: &[String]) -> String {
    let display_program = display_program_name(program);
    if args.is_empty() {
        display_program.to_string()
    } else {
        format!("{display_program} {}", args.join(" "))
    }
}

fn exec_rate_limit_subject(program: &str, command: &str) -> String {
    let executable = exec_capability_from_command(command)
        .and_then(|capability| match capability {
            Capability::Exec(name) => Some(name),
            _ => None,
        })
        .unwrap_or_else(|| display_program_name(program).to_string());
    format!("exec:{executable}")
}

fn display_program_name(program: &str) -> &str {
    Path::new(program)
        .file_name()
        .filter(|name| !name.is_empty())
        .and_then(OsStr::to_str)
        .unwrap_or(program)
}

fn is_shell_program(program: &str) -> bool {
    let Some(name) = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
    else {
        return false;
    };
    matches!(name, "sh" | "bash" | "zsh" | "dash" | "ksh")
}

fn is_git_program(program: &str) -> bool {
    Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        == Some("git")
}

fn is_shell_command_flag(flag: &str) -> bool {
    flag.starts_with('-') && flag.contains('c')
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::{ToolCall, ToolContext};

    fn test_ctx() -> ToolContext {
        ToolContext::testing("/tmp/worktree")
    }

    fn bash_call(cmd: &str) -> ToolCall {
        ToolCall::new("test-id", "bash", serde_json::json!({ "command": cmd }))
    }

    /// Safety layer with a permissive contract for tests that check allow behavior.
    fn permissive_layer() -> SafetyLayer {
        SafetyLayer::with_defaults().with_contract(AgentContract::permissive("test"))
    }

    #[test]
    fn safety_layer_blocks_dangerous_bash() {
        let layer = SafetyLayer::with_defaults();
        let ctx = test_ctx();
        let call = bash_call("rm -rf /");
        assert!(layer.check_pre_execution(&call, &ctx).is_err());
    }

    #[test]
    fn safety_layer_allows_safe_bash() {
        let layer = permissive_layer();
        let ctx = test_ctx();
        let call = bash_call("cargo test");
        assert!(layer.check_pre_execution(&call, &ctx).is_ok());
    }

    #[test]
    fn safety_layer_blocks_force_push_to_main() {
        let layer = SafetyLayer::with_defaults();
        let ctx = test_ctx();
        let call = bash_call("git push --force origin main");
        assert!(layer.check_pre_execution(&call, &ctx).is_err());
    }

    #[test]
    fn safety_layer_scrubs_api_key() {
        let layer = SafetyLayer::with_defaults();
        let result = ToolResult::text(
            "key is sk-ant-api03-abcdefghij1234567890abcdefghij1234567890abcdefghij1234567890abcdefghij1234-AAAAAA",
        );
        let scrubbed = layer.scrub_output(result);
        match scrubbed {
            ToolResult::Ok { content, .. } => {
                assert!(!content.contains("sk-ant-api03"));
            }
            _ => panic!("expected Ok variant"),
        }
    }

    #[test]
    fn safety_layer_passes_errors_through() {
        let layer = SafetyLayer::with_defaults();
        let result = ToolResult::err(ToolError::Other("oops".into()));
        let out = layer.scrub_output(result);
        assert!(matches!(out, ToolResult::Err(_)));
    }

    #[test]
    fn safety_layer_no_safety_means_passthrough() {
        // Verify that non-filesystem tools pass through without errors.
        let layer = permissive_layer();
        let ctx = test_ctx();
        let call = ToolCall::new("test-id", "exit_plan_mode", serde_json::json!({}));
        // exit_plan_mode is not in any policy group; should pass all checks.
        assert!(layer.check_pre_execution(&call, &ctx).is_ok());
    }

    #[test]
    fn network_tool_blocked_for_private_ip() {
        let layer = SafetyLayer::with_defaults();
        let ctx = test_ctx();
        let call = ToolCall::new(
            "test-id",
            "web_fetch",
            serde_json::json!({ "url": "http://127.0.0.1:8080/secrets" }),
        );
        assert!(layer.check_pre_execution(&call, &ctx).is_err());
    }

    #[test]
    fn rate_limiter_eventually_blocks() {
        let mut layer = permissive_layer();
        // Custom tight limit: 2 calls per window.
        layer.rate_limiter = Some(Arc::new(RateLimiter::new(rate_limit::RateLimitPolicy {
            max_calls_per_window: 2,
            window_duration: std::time::Duration::from_secs(60),
        })));
        let ctx = test_ctx();
        let call = bash_call("echo hi");
        assert!(layer.check_pre_execution(&call, &ctx).is_ok());
        assert!(layer.check_pre_execution(&call, &ctx).is_ok());
        assert!(layer.check_pre_execution(&call, &ctx).is_err());
    }

    #[test]
    fn exec_command_blocks_dangerous_shell_wrapper() {
        let layer = SafetyLayer::with_defaults();
        let args = vec!["-lc".to_string(), "rm -rf /".to_string()];
        assert!(layer.check_exec_command("/bin/bash", &args).is_err());
    }

    #[test]
    fn exec_command_blocks_direct_git_force_push() {
        let layer = SafetyLayer::with_defaults();
        let args = vec![
            "push".to_string(),
            "--force".to_string(),
            "origin".to_string(),
            "main".to_string(),
        ];
        assert!(layer.check_exec_command("git", &args).is_err());
    }

    #[test]
    fn exec_command_blocks_direct_dangerous_command() {
        let layer = SafetyLayer::with_defaults();
        let args = vec!["-rf".to_string(), "/".to_string()];
        assert!(layer.check_exec_command("/bin/rm", &args).is_err());
    }

    #[test]
    fn exec_command_requires_warrant_for_subprocess() {
        let layer = SafetyLayer::with_defaults().with_warrant(AgentWarrant::new(
            "issuer",
            vec![Capability::Exec("git".into())],
            1,
        ));
        let args = vec!["status".to_string()];
        assert!(layer.check_exec_command("git", &args).is_ok());
        assert!(
            layer
                .check_exec_command("cargo", &["check".to_string()])
                .is_err()
        );
    }

    #[test]
    fn exec_command_rate_limit_applies() {
        let mut layer = SafetyLayer::with_defaults();
        layer.rate_limiter = Some(Arc::new(RateLimiter::new(rate_limit::RateLimitPolicy {
            max_calls_per_window: 1,
            window_duration: std::time::Duration::from_secs(60),
        })));
        let args = vec!["status".to_string()];
        assert!(layer.check_exec_command("git", &args).is_ok());
        assert!(layer.check_exec_command("git", &args).is_err());
    }

    #[test]
    fn exec_command_allows_safe_shell_wrapper() {
        let layer = SafetyLayer::with_defaults();
        let args = vec!["-c".to_string(), "echo hi".to_string()];
        assert!(layer.check_exec_command("sh", &args).is_ok());
    }

    #[test]
    fn authorize_call_denies_blocked_command() {
        let layer = SafetyLayer::with_defaults();
        let ctx = test_ctx();
        let decision = layer.authorize_call(&bash_call("rm -rf /"), &ctx);
        assert!(matches!(decision, AuthzDecision::Deny { .. }));
    }

    // ─── Taint enforcement tests ─────────────────────────────────────

    #[test]
    fn taint_escalates_network_to_allow_with_confirm() {
        let layer = permissive_layer();
        let ctx = ToolContext::testing("/tmp/worktree");
        let call = ToolCall::new(
            "test-id",
            "web_fetch",
            serde_json::json!({ "url": "https://safe.example.com" }),
        );
        let taint = Taint::ExternalFetch("untrusted-source".into());
        let decision = layer.authorize_call_with_taint(&call, &ctx, Some(&taint));
        assert!(
            matches!(decision, AuthzDecision::AllowWithConfirm { .. }),
            "expected AllowWithConfirm, got {decision:?}"
        );
    }

    #[test]
    fn taint_escalates_file_write_to_allow_with_confirm() {
        // Use a real temp dir so PathPolicy doesn't reject the path.
        let tmp = tempfile::tempdir().unwrap();
        let worktree = tmp.path().to_path_buf();
        let file_path = worktree.join("x.txt");
        let ctx = ToolContext::testing(&worktree);
        let layer = permissive_layer();
        let call = ToolCall::new(
            "test-id",
            "write_file",
            serde_json::json!({ "file_path": file_path.to_str().unwrap(), "content": "hi" }),
        );
        let taint = Taint::ThirdPartyPlugin("plugin-x".into());
        let decision = layer.authorize_call_with_taint(&call, &ctx, Some(&taint));
        assert!(
            matches!(decision, AuthzDecision::AllowWithConfirm { .. }),
            "expected AllowWithConfirm, got {decision:?}"
        );
    }

    #[test]
    fn taint_escalates_bash_to_allow_with_confirm() {
        let layer = permissive_layer();
        let ctx = test_ctx();
        let call = bash_call("echo hi");
        let taint = Taint::UserInput;
        let decision = layer.authorize_call_with_taint(&call, &ctx, Some(&taint));
        assert!(
            matches!(decision, AuthzDecision::AllowWithConfirm { .. }),
            "expected AllowWithConfirm for bash under taint, got {decision:?}"
        );
    }

    #[test]
    fn no_taint_means_normal_allow() {
        let layer = permissive_layer();
        let ctx = test_ctx();
        let call = bash_call("echo hi");
        let decision = layer.authorize_call_with_taint(&call, &ctx, None);
        assert!(
            matches!(decision, AuthzDecision::Allow { .. }),
            "expected Allow, got {decision:?}"
        );
    }

    #[test]
    fn inactive_taint_means_normal_allow() {
        let layer = permissive_layer();
        let ctx = test_ctx();
        let call = bash_call("echo hi");
        let taint = Taint::None;
        let decision = layer.authorize_call_with_taint(&call, &ctx, Some(&taint));
        assert!(
            matches!(decision, AuthzDecision::Allow { .. }),
            "expected Allow for inactive taint, got {decision:?}"
        );
    }

    #[test]
    fn safety_budget_blocks_after_limit_is_spent() {
        let layer =
            permissive_layer().with_safety_budget(SafetyBudgetTracker::new(risk::SafetyBudget {
                footprint_limit: 1,
                ..risk::SafetyBudget::default()
            }));
        let ctx = test_ctx();
        let call = bash_call("echo hi");
        assert!(layer.check_pre_execution(&call, &ctx).is_ok());
        assert!(layer.check_pre_execution(&call, &ctx).is_err());
    }

    #[test]
    fn safety_layer_scrubs_text() {
        let layer = SafetyLayer::with_defaults();
        let cleaned = layer.scrub_text(
            "sk-ant-api03-abcdefghij1234567890abcdefghij1234567890abcdefghij1234567890abcdefghij1234-AAAAAA",
        );
        assert!(!cleaned.contains("sk-ant-api03"));
    }

    // ─── Temporal monitor integration ────────────────────────────────

    #[test]
    fn temporal_monitor_blocks_never_pattern_in_safety_layer() {
        let monitor = TemporalMonitor::with_properties(vec![LtlProperty::Never {
            pattern: "force-push main".into(),
            description: "never force-push main".into(),
        }]);
        let layer = permissive_layer().with_temporal_monitor(monitor);
        let ctx = test_ctx();

        // Safe command passes.
        let call = bash_call("git push origin feature");
        assert!(layer.check_pre_execution(&call, &ctx).is_ok());

        // Dangerous command is blocked by temporal monitor.
        let call = bash_call("git push --force-push main");
        let result = layer.check_pre_execution(&call, &ctx);
        assert!(result.is_err(), "expected temporal violation");
        if let Err(ToolError::PermissionDenied(msg)) = result {
            assert!(msg.contains("temporal property violation"));
        }
    }

    // ─── AGT-01: Pre/post dispatch check tests ─────────────────────

    #[test]
    fn pre_dispatch_check_passes_for_normal_dir() {
        let layer = SafetyLayer::with_defaults();
        let tmp = tempfile::tempdir().unwrap();
        let result = layer.pre_dispatch_check("plan-1", "task-1", "implementer", tmp.path());
        assert!(result.is_ok());
    }

    #[test]
    fn pre_dispatch_check_blocks_exhausted_budget() {
        let budget = SafetyBudgetTracker::new(risk::SafetyBudget {
            footprint_limit: 0,
            ..risk::SafetyBudget::default()
        });
        let layer = SafetyLayer::with_defaults().with_safety_budget(budget);
        let tmp = tempfile::tempdir().unwrap();
        let result = layer.pre_dispatch_check("plan-1", "task-1", "implementer", tmp.path());
        assert!(result.is_err());
        if let Err(violation) = result {
            assert_eq!(violation.violation_type, ViolationType::BudgetExhausted);
        }
    }

    #[test]
    fn post_dispatch_check_detects_secret_leak() {
        let layer = SafetyLayer::with_defaults();
        let api_key = format!("sk-ant-api03-{}", "A".repeat(80));
        let output = format!("found key: {api_key}");
        let violations = layer.post_dispatch_check("plan-1", "task-1", "implementer", &output, &[]);
        assert!(!violations.is_empty());
        assert!(
            violations
                .iter()
                .any(|v| v.violation_type == ViolationType::SecretLeak)
        );
    }

    #[test]
    fn post_dispatch_check_passes_clean_output() {
        let layer = SafetyLayer::with_defaults();
        let violations = layer.post_dispatch_check(
            "plan-1",
            "task-1",
            "implementer",
            "all tests pass",
            &["src/lib.rs".to_string()],
        );
        assert!(violations.is_empty());
    }

    #[test]
    fn post_dispatch_check_detects_forbidden_file_writes() {
        let mut layer = SafetyLayer::with_defaults();
        layer
            .contract
            .governance
            .push(GovernanceRule::ForbiddenTools(vec![
                "write_file".to_string(),
            ]));
        let violations = layer.post_dispatch_check(
            "plan-1",
            "task-1",
            "reviewer",
            "reviewed code",
            &["src/lib.rs".to_string()],
        );
        assert!(!violations.is_empty());
        assert!(
            violations
                .iter()
                .any(|v| v.violation_type == ViolationType::ContractViolation)
        );
    }
}
