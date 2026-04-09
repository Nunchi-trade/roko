//! `web_search` — query a web search provider.
//!
//! Category: [`ToolCategory::Network`]. Permission: networked.
//! Concurrency: [`ToolConcurrency::Parallel`]. Idempotent: yes.
//!
//! # Day-one stub
//!
//! Validates arguments and enforces the network capability guard.
//! An actual search-provider client (e.g. a user-configured engine)
//! ships with the broader §36 network-safety pass. Until then, calls
//! with the `network` capability return a structured "not wired"
//! error so agents can distinguish "no provider yet" from "permission
//! denied".

use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};

use super::sandbox::require_string;

/// Canonical `snake_case` name.
pub const NAME: &str = "web_search";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Query a configured web search provider and return top results.";

/// Build the [`ToolDef`] for `web_search`.
#[must_use]
pub fn tool_def() -> ToolDef {
    ToolDef::new(
        NAME,
        DESCRIPTION,
        ToolCategory::Network,
        ToolPermission::networked(),
    )
    .with_parameters(ToolSchema::any_object())
    .with_concurrency(ToolConcurrency::Parallel)
    .with_idempotent(true)
    .with_timeout_ms(30_000)
}

/// Handler for `web_search` (§36.23).
///
/// Day-one stub — enforces `network` capability and returns a
/// "not yet wired" [`ToolError::Other`] when the gate lets the call
/// through. The concrete provider client lands in a follow-up commit.
#[derive(Debug, Clone, Copy, Default)]
pub struct Handler;

#[async_trait]
impl ToolHandler for Handler {
    fn name(&self) -> &str {
        NAME
    }

    async fn execute(&self, call: ToolCall, ctx: &ToolContext) -> ToolResult {
        if !ctx.capabilities.network {
            return ToolResult::Err(ToolError::PermissionDenied(
                "web_search requires network capability".into(),
            ));
        }
        let query = match require_string(&call.arguments, "query") {
            Ok(q) => q,
            Err(e) => return ToolResult::Err(e),
        };
        if query.trim().is_empty() {
            return ToolResult::Err(ToolError::SchemaInvalid(
                "web_search: `query` must be non-empty".into(),
            ));
        }
        ToolResult::Err(ToolError::Other(
            "web_search: no search provider wired (day-one stub)".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::ToolContext;

    fn testing_ctx_no_net() -> ToolContext {
        let mut ctx = ToolContext::testing("/tmp/work");
        ctx.capabilities.network = false;
        ctx
    }

    fn testing_ctx_with_net() -> ToolContext {
        let mut ctx = ToolContext::testing("/tmp/work");
        ctx.capabilities.network = true;
        ctx
    }

    #[tokio::test]
    async fn network_capability_gate_denies_when_off() {
        let ctx = testing_ctx_no_net();
        let call = ToolCall::new("c", NAME, serde_json::json!({ "query": "claude code" }));
        let res = Handler.execute(call, &ctx).await;
        assert!(matches!(
            res,
            ToolResult::Err(ToolError::PermissionDenied(_))
        ));
    }

    #[tokio::test]
    async fn missing_query_is_schema_invalid() {
        let ctx = testing_ctx_with_net();
        let call = ToolCall::new("c", NAME, serde_json::json!({}));
        let res = Handler.execute(call, &ctx).await;
        assert!(matches!(res, ToolResult::Err(ToolError::SchemaInvalid(_))));
    }

    #[tokio::test]
    async fn blank_query_is_schema_invalid() {
        let ctx = testing_ctx_with_net();
        let call = ToolCall::new("c", NAME, serde_json::json!({ "query": "   " }));
        let res = Handler.execute(call, &ctx).await;
        assert!(matches!(res, ToolResult::Err(ToolError::SchemaInvalid(_))));
    }

    #[tokio::test]
    async fn valid_query_passes_through_to_stub_error() {
        let ctx = testing_ctx_with_net();
        let call = ToolCall::new("c", NAME, serde_json::json!({ "query": "ripgrep" }));
        let res = Handler.execute(call, &ctx).await;
        assert!(matches!(res, ToolResult::Err(ToolError::Other(_))));
    }

    #[test]
    fn handler_name_matches_tool_def() {
        assert_eq!(Handler.name(), NAME);
    }
}
