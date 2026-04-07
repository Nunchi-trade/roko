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
pub mod git;
pub mod network;
pub mod path;
pub mod rate_limit;
pub mod scrub;

use std::sync::Arc;

use roko_core::tool::{ToolCall, ToolContext, ToolError, ToolResult};

use self::bash::BashPolicy;
use self::git::GitPolicy;
use self::network::NetworkPolicy;
use self::path::PathPolicy;
use self::rate_limit::{RateLimitKey, RateLimiter};
use self::scrub::ScrubPolicy;

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
    /// Role name used as part of the rate-limit key.
    /// Defaults to `"default"`.
    pub role: String,
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
            role: "default".into(),
        }
    }

    /// Override the role label used in rate-limit keys.
    #[must_use]
    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.role = role.into();
        self
    }

    /// Run all pre-execution safety checks for `call` + `ctx`.
    ///
    /// Returns `Ok(())` if all policies pass; the first failure
    /// short-circuits and is returned as an `Err`.
    pub fn check_pre_execution(
        &self,
        call: &ToolCall,
        ctx: &ToolContext,
    ) -> Result<(), ToolError> {
        let name = call.name.as_str();

        // 1. Rate limit (applies to all tools).
        if let Some(ref limiter) = self.rate_limiter {
            let key = RateLimitKey {
                role: self.role.clone(),
                tool: name.to_string(),
            };
            limiter.check_and_record(&key)?;
        }

        // 2. Bash / run_tests policy (command argument).
        if BASH_TOOLS.contains(&name) {
            if let Some(cmd) = call.arguments.get("command").and_then(|v| v.as_str()) {
                bash::check_command_with_policy(cmd, &self.bash_policy)?;
                git::check_git_command_with_policy(cmd, &self.git_policy)?;
            }
        }

        // 3. Network policy (url argument).
        if NETWORK_TOOLS.contains(&name) {
            if let Some(url) = call.arguments.get("url").and_then(|v| v.as_str()) {
                network::check_url_with_policy(url, &self.network_policy)?;
            }
        }

        // 4. Path policy (file_path / path argument).
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

        Ok(())
    }

    /// Scrub secrets from a successful tool result.
    ///
    /// Only `ToolResult::Ok` variants are scrubbed; errors pass through
    /// unchanged.
    #[must_use]
    pub fn scrub_output(&self, result: ToolResult) -> ToolResult {
        match result {
            ToolResult::Ok { content, is_structured, artifacts } => {
                let cleaned = scrub::scrub_secrets(&content, &self.scrub_policy);
                ToolResult::Ok { content: cleaned, is_structured, artifacts }
            }
            err @ ToolResult::Err(_) => err,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::{ToolCall, ToolContext};

    fn test_ctx() -> ToolContext {
        ToolContext::testing("/tmp/worktree")
    }

    fn bash_call(cmd: &str) -> ToolCall {
        ToolCall::new(
            "test-id",
            "bash",
            serde_json::json!({ "command": cmd }),
        )
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
        let result =
            ToolResult::text("key is sk-ant-api03-abcdefghij1234567890abcdefghij1234567890abcdefghij1234567890abcdefghij1234-AAAAAA");
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
        let call = ToolCall::new(
            "test-id",
            "exit_plan_mode",
            serde_json::json!({}),
        );
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
        layer.rate_limiter = Some(Arc::new(RateLimiter::new(
            rate_limit::RateLimitPolicy {
                max_calls_per_window: 2,
                window_duration: std::time::Duration::from_secs(60),
            },
        )));
        let ctx = test_ctx();
        let call = bash_call("echo hi");
        assert!(layer.check_pre_execution(&call, &ctx).is_ok());
        assert!(layer.check_pre_execution(&call, &ctx).is_ok());
        assert!(layer.check_pre_execution(&call, &ctx).is_err());
    }
}
