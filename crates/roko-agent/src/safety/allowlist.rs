//! Allowlist/denylist guard safety hook (TOOL-02, hook #2).
//!
//! Filters tool calls based on explicit allow and deny lists. The guard
//! checks tool names against two sets:
//!
//! - **allowlist**: if non-empty, only these tools are permitted
//! - **denylist**: these tools are always rejected, even if on the allowlist
//!
//! Denylist takes precedence over allowlist. When the allowlist is empty,
//! all tools not on the denylist are permitted.

use std::collections::HashSet;

use async_trait::async_trait;
use roko_core::tool::{ToolContext, ToolDef, ToolError};

use super::hooks::{HookDecision, SafetyHook};

/// Tool-level allowlist/denylist filter.
///
/// Place this early in the hook chain (after PolicyCage) to gate which
/// tools an agent is permitted to invoke. Configuration comes from
/// `[tools.profile]` in `roko.toml` or from domain/role profile composition.
#[derive(Debug, Clone)]
pub struct AllowlistGuard {
    /// Tools explicitly allowed. Empty means "allow all not denied".
    pub allowed: HashSet<String>,
    /// Tools explicitly denied. Takes precedence over `allowed`.
    pub denied: HashSet<String>,
}

impl AllowlistGuard {
    /// Create a guard that allows everything (no filtering).
    pub fn permissive() -> Self {
        Self {
            allowed: HashSet::new(),
            denied: HashSet::new(),
        }
    }

    /// Create a guard from explicit allow and deny lists.
    pub fn new(
        allowed: impl IntoIterator<Item = impl Into<String>>,
        denied: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            allowed: allowed.into_iter().map(Into::into).collect(),
            denied: denied.into_iter().map(Into::into).collect(),
        }
    }

    /// Create a guard that only allows the specified tools.
    pub fn allow_only(tools: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            allowed: tools.into_iter().map(Into::into).collect(),
            denied: HashSet::new(),
        }
    }

    /// Create a guard that denies the specified tools (allow all others).
    pub fn deny_only(tools: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            allowed: HashSet::new(),
            denied: tools.into_iter().map(Into::into).collect(),
        }
    }

    /// Check whether a tool name is permitted.
    pub fn is_permitted(&self, tool_name: &str) -> bool {
        // Denylist always takes precedence.
        if self.denied.contains(tool_name) {
            return false;
        }
        // If allowlist is empty, permit everything not denied.
        if self.allowed.is_empty() {
            return true;
        }
        // Otherwise, tool must be on the allowlist.
        self.allowed.contains(tool_name)
    }
}

#[async_trait]
impl SafetyHook for AllowlistGuard {
    async fn on_tool_call(
        &self,
        tool: &ToolDef,
        _params: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<HookDecision, ToolError> {
        if self.denied.contains(&tool.name) {
            return Ok(HookDecision::Reject(format!(
                "tool `{}` is on the denylist",
                tool.name
            )));
        }

        if !self.allowed.is_empty() && !self.allowed.contains(&tool.name) {
            return Ok(HookDecision::Reject(format!(
                "tool `{}` is not on the allowlist",
                tool.name
            )));
        }

        Ok(HookDecision::Allow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::{ToolCategory, ToolPermission};

    fn test_ctx() -> ToolContext {
        ToolContext::testing("/tmp/worktree")
    }

    fn test_tool(name: &str) -> ToolDef {
        ToolDef::new(name, "test tool", ToolCategory::Read, ToolPermission::read_only())
    }

    #[tokio::test]
    async fn permissive_allows_all() {
        let guard = AllowlistGuard::permissive();
        let tool = test_tool("anything");
        let params = serde_json::json!({});
        let result = guard.on_tool_call(&tool, &params, &test_ctx()).await.unwrap();
        assert_eq!(result, HookDecision::Allow);
    }

    #[tokio::test]
    async fn allowlist_permits_listed_tools() {
        let guard = AllowlistGuard::allow_only(["read_file", "write_file"]);
        let tool = test_tool("read_file");
        let params = serde_json::json!({});
        let result = guard.on_tool_call(&tool, &params, &test_ctx()).await.unwrap();
        assert_eq!(result, HookDecision::Allow);
    }

    #[tokio::test]
    async fn allowlist_rejects_unlisted_tools() {
        let guard = AllowlistGuard::allow_only(["read_file"]);
        let tool = test_tool("bash");
        let params = serde_json::json!({});
        let result = guard.on_tool_call(&tool, &params, &test_ctx()).await.unwrap();
        assert!(matches!(result, HookDecision::Reject(_)));
    }

    #[tokio::test]
    async fn denylist_rejects_listed_tools() {
        let guard = AllowlistGuard::deny_only(["bash", "write_file"]);
        let tool = test_tool("bash");
        let params = serde_json::json!({});
        let result = guard.on_tool_call(&tool, &params, &test_ctx()).await.unwrap();
        assert!(matches!(result, HookDecision::Reject(_)));
    }

    #[tokio::test]
    async fn denylist_allows_unlisted_tools() {
        let guard = AllowlistGuard::deny_only(["bash"]);
        let tool = test_tool("read_file");
        let params = serde_json::json!({});
        let result = guard.on_tool_call(&tool, &params, &test_ctx()).await.unwrap();
        assert_eq!(result, HookDecision::Allow);
    }

    #[tokio::test]
    async fn deny_takes_precedence_over_allow() {
        let guard = AllowlistGuard::new(["bash", "read_file"], ["bash"]);
        let tool = test_tool("bash");
        let params = serde_json::json!({});
        let result = guard.on_tool_call(&tool, &params, &test_ctx()).await.unwrap();
        assert!(matches!(result, HookDecision::Reject(_)));
    }

    #[test]
    fn is_permitted_logic() {
        let guard = AllowlistGuard::new(["read_file", "write_file"], ["write_file"]);
        assert!(guard.is_permitted("read_file"));
        assert!(!guard.is_permitted("write_file")); // denied
        assert!(!guard.is_permitted("bash")); // not on allowlist
    }

    #[test]
    fn empty_allowlist_permits_non_denied() {
        let guard = AllowlistGuard::deny_only(["bash"]);
        assert!(guard.is_permitted("read_file"));
        assert!(guard.is_permitted("write_file"));
        assert!(!guard.is_permitted("bash"));
    }
}
