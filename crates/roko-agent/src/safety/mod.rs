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

pub mod bash;
pub mod authz;
pub mod capabilities;
pub mod contract;
pub mod git;
pub mod network;
pub mod path;
pub mod provenance;
pub mod rate_limit;
pub mod risk;
pub mod scrub;

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use regex::Regex;
use roko_core::config::schema::{RokoConfig, RoleOverride};
use roko_core::tool::{ToolCall, ToolContext, ToolError, ToolResult};

use self::bash::BashPolicy;
use self::contract::{AgentContract, GovernanceRule, Invariant};
use self::git::GitPolicy;
use self::network::NetworkPolicy;
use self::path::PathPolicy;
use self::rate_limit::{RateLimitKey, RateLimiter};
use self::risk::{BudgetCheckResult, ProposedAction};
use self::scrub::ScrubPolicy;

use self::capabilities::{exec_capability_from_command, network_capability_from_url};
pub use authz::{AuthzDecision, AuthorizationEvidence, AuthorizationSource, EscalationTarget};
pub use capabilities::{AgentWarrant, Capability, CapabilityError, check_capability, delegate};
pub use provenance::{AttestationLevel, Custody, Taint};
pub use risk::{
    BetaDistribution, BudgetDimension, OperationalConfidenceTracker, SafetyBudget,
    SafetyBudgetTracker, confidence_multiplier, effective_limit, irreversibility_score,
};

// ─── Tool-name constants used to match calls to policies ──────────────────

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
            contract: AgentContract::permissive("default"),
            warrant: None,
            role_tools: HashMap::new(),
            role_overrides: HashMap::new(),
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
    pub fn with_shared_safety_budget(
        mut self,
        budget: Arc<Mutex<SafetyBudgetTracker>>,
    ) -> Self {
        self.safety_budget = Some(budget);
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

        Ok(())
    }

    /// Evaluate a tool call through the safety layer and return a unified
    /// authorization decision.
    #[must_use]
    pub fn authorize_call(&self, call: &ToolCall, ctx: &ToolContext) -> AuthzDecision {
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

    fn contract_for_role(&self, role: &str) -> AgentContract {
        let mut contract = AgentContract::load_for_role(role).unwrap_or_else(|err| {
            tracing::warn!(
                %role,
                %err,
                "no contract for role; using permissive default"
            );
            AgentContract::permissive(role.to_string())
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

    #[test]
    fn safety_layer_blocks_dangerous_bash() {
        let layer = SafetyLayer::with_defaults();
        let ctx = test_ctx();
        let call = bash_call("rm -rf /");
        assert!(layer.check_pre_execution(&call, &ctx).is_err());
    }

    #[test]
    fn safety_layer_allows_safe_bash() {
        let layer = SafetyLayer::with_defaults();
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
        let layer = SafetyLayer::with_defaults();
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
        let mut layer = SafetyLayer::with_defaults();
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

    #[test]
    fn safety_budget_blocks_after_limit_is_spent() {
        let layer = SafetyLayer::with_defaults().with_safety_budget(SafetyBudgetTracker::new(
            risk::SafetyBudget {
                footprint_limit: 1,
                ..risk::SafetyBudget::default()
            },
        ));
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
}
