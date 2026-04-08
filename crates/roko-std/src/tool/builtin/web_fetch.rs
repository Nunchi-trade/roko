//! `web_fetch` — fetch a web page or resource.
//!
//! Category: [`ToolCategory::Network`]. Permission: networked.
//! Concurrency: [`ToolConcurrency::Parallel`]. Idempotent: yes
//! (HTTP GET assumed; POST / mutating fetches would not be idempotent).
//!
//! # Day-one stub
//!
//! The handler validates arguments and enforces the network capability
//! guard, but does **not** yet perform an actual HTTP request — the
//! concrete client (e.g. `reqwest` + `rustls`) lands with the broader
//! §36 network-safety pass. For now, a call with the `network`
//! capability granted returns a structured "not wired" error so agents
//! can differentiate "tool unavailable" from "permission denied".

use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};

use super::sandbox::require_string;

/// Canonical `snake_case` name.
pub const NAME: &str = "web_fetch";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Fetch a URL via HTTP(S) and return its body, subject to \
    the domain allowlist configured in safety policy.";

/// Build the [`ToolDef`] for `web_fetch`.
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

/// Handler for `web_fetch` (§36.22).
///
/// Validates the `url` argument and the network capability, then
/// returns a structured "not yet wired" error. The real HTTP client
/// will land in a follow-up commit together with the domain
/// allowlist wiring.
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
                "web_fetch requires network capability".into(),
            ));
        }
        let url = match require_string(&call.arguments, "url") {
            Ok(u) => u,
            Err(e) => return ToolResult::Err(e),
        };
        let scheme = url
            .split("://")
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return ToolResult::Err(ToolError::SchemaInvalid(format!(
                "web_fetch: only http(s) URLs are supported, got `{url}`"
            )));
        }
        if scheme == "http"
            && !url.starts_with("http://localhost")
            && !url.starts_with("http://127.")
        {
            return ToolResult::Err(ToolError::NetworkBlocked(format!(
                "web_fetch: plain http is only allowed for localhost, got `{url}`"
            )));
        }
        ToolResult::Err(ToolError::Other(
            "web_fetch: HTTP client not yet wired (day-one stub)".into(),
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
        let call = ToolCall::new(
            "c",
            NAME,
            serde_json::json!({ "url": "https://example.com" }),
        );
        let res = Handler.execute(call, &ctx).await;
        assert!(matches!(
            res,
            ToolResult::Err(ToolError::PermissionDenied(_))
        ));
    }

    #[tokio::test]
    async fn missing_url_is_schema_invalid() {
        let ctx = testing_ctx_with_net();
        let call = ToolCall::new("c", NAME, serde_json::json!({}));
        let res = Handler.execute(call, &ctx).await;
        assert!(matches!(res, ToolResult::Err(ToolError::SchemaInvalid(_))));
    }

    #[tokio::test]
    async fn non_http_scheme_rejected() {
        let ctx = testing_ctx_with_net();
        let call = ToolCall::new(
            "c",
            NAME,
            serde_json::json!({ "url": "file:///etc/passwd" }),
        );
        let res = Handler.execute(call, &ctx).await;
        assert!(matches!(res, ToolResult::Err(ToolError::SchemaInvalid(_))));
    }

    #[tokio::test]
    async fn plain_http_only_allowed_for_localhost() {
        let ctx = testing_ctx_with_net();
        let call = ToolCall::new(
            "c",
            NAME,
            serde_json::json!({ "url": "http://evil.example.com" }),
        );
        let res = Handler.execute(call, &ctx).await;
        assert!(matches!(res, ToolResult::Err(ToolError::NetworkBlocked(_))));
    }

    #[tokio::test]
    async fn localhost_http_passes_through_to_stub_error() {
        let ctx = testing_ctx_with_net();
        let call = ToolCall::new(
            "c",
            NAME,
            serde_json::json!({ "url": "http://localhost:8080/x" }),
        );
        let res = Handler.execute(call, &ctx).await;
        // Falls through to the "not yet wired" error, which is Other(..).
        assert!(matches!(res, ToolResult::Err(ToolError::Other(_))));
    }

    #[tokio::test]
    async fn https_passes_through_to_stub_error() {
        let ctx = testing_ctx_with_net();
        let call = ToolCall::new(
            "c",
            NAME,
            serde_json::json!({ "url": "https://example.com" }),
        );
        let res = Handler.execute(call, &ctx).await;
        assert!(matches!(res, ToolResult::Err(ToolError::Other(_))));
    }

    #[test]
    fn handler_name_matches_tool_def() {
        assert_eq!(Handler.name(), NAME);
    }
}
