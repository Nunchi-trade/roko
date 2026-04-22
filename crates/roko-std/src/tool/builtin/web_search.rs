//! `web_search` -- query a web search provider via Perplexity sonar.
//!
//! Category: [`ToolCategory::Network`]. Permission: networked.
//! Concurrency: [`ToolConcurrency::Parallel`]. Idempotent: yes.
//!
//! Uses the Perplexity chat/completions API with the `sonar` model to
//! perform search-grounded generation. Requires the `PERPLEXITY_API_KEY`
//! environment variable to be set.

use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};
use std::fmt::Write as _;
use std::time::Duration;

use super::sandbox::require_string;

/// Canonical `snake_case` name.
pub const NAME: &str = "web_search";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Query a configured web search provider and return top results.";

/// Perplexity API base URL.
const PERPLEXITY_API_URL: &str = "https://api.perplexity.ai/chat/completions";

/// Default model slug for search queries.
const DEFAULT_MODEL: &str = "sonar";

/// Request timeout in seconds.
const SEARCH_TIMEOUT_SECS: u64 = 30;

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

/// Format the Perplexity response into a readable text block for the agent.
fn format_response(parsed: &serde_json::Value) -> String {
    let mut out = String::new();

    // Extract the main answer.
    let content = parsed
        .pointer("/choices/0/message/content")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    if !content.is_empty() {
        out.push_str(content);
    }

    // Append citations if present.
    if let Some(citations) = parsed.get("citations").and_then(serde_json::Value::as_array) {
        if !citations.is_empty() {
            out.push_str("\n\nSources:\n");
            for (i, cite) in citations.iter().enumerate() {
                if let Some(url) = cite.as_str() {
                    let _ = writeln!(out, "[{}] {}", i + 1, url);
                }
            }
        }
    }

    // Append search result snippets if present.
    if let Some(results) = parsed
        .get("search_results")
        .and_then(serde_json::Value::as_array)
    {
        if !results.is_empty() {
            out.push_str("\nSearch results:\n");
            for result in results {
                let title = result
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("(untitled)");
                let url = result
                    .get("url")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let snippet = result
                    .get("content")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let _ = writeln!(out, "- {title}\n  {url}");
                if !snippet.is_empty() {
                    let truncated = if snippet.len() > 200 {
                        format!("{}...", &snippet[..200])
                    } else {
                        snippet.to_string()
                    };
                    let _ = writeln!(out, "  {truncated}");
                }
            }
        }
    }

    out
}

/// Send the search query to the Perplexity API and return the raw response text.
async fn call_perplexity(query: &str, api_key: &str) -> ToolResult {
    let body = serde_json::json!({
        "model": DEFAULT_MODEL,
        "messages": [
            {
                "role": "system",
                "content": "You are a search assistant. Answer the query concisely \
                    with citations. Be precise and factual."
            },
            {
                "role": "user",
                "content": query
            }
        ],
        "return_related_questions": false,
    });

    let body_bytes = match serde_json::to_vec(&body) {
        Ok(b) => b,
        Err(e) => {
            return ToolResult::Err(ToolError::Other(format!(
                "web_search: failed to serialize request: {e}"
            )));
        }
    };

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(SEARCH_TIMEOUT_SECS))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return ToolResult::Err(ToolError::Other(format!(
                "web_search: failed to build HTTP client: {e}"
            )));
        }
    };

    let response = match client
        .post(PERPLEXITY_API_URL)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .body(body_bytes)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            if e.is_timeout() {
                return ToolResult::Err(ToolError::Timeout {
                    after_ms: SEARCH_TIMEOUT_SECS * 1_000,
                });
            }
            return ToolResult::Err(ToolError::Other(format!(
                "web_search: request to Perplexity failed: {e}"
            )));
        }
    };

    parse_perplexity_response(response).await
}

/// Parse the HTTP response from Perplexity into a formatted `ToolResult`.
async fn parse_perplexity_response(response: reqwest::Response) -> ToolResult {
    let status = response.status();
    let response_text = match response.text().await {
        Ok(t) => t,
        Err(e) => {
            return ToolResult::Err(ToolError::Other(format!(
                "web_search: reading Perplexity response failed: {e}"
            )));
        }
    };

    if !status.is_success() {
        let truncated = if response_text.len() > 500 {
            format!("{}...", &response_text[..500])
        } else {
            response_text
        };
        return ToolResult::Err(ToolError::Other(format!(
            "web_search: Perplexity returned HTTP {status}: {truncated}"
        )));
    }

    let parsed: serde_json::Value = match serde_json::from_str(&response_text) {
        Ok(v) => v,
        Err(e) => {
            return ToolResult::Err(ToolError::Other(format!(
                "web_search: malformed response from Perplexity: {e}"
            )));
        }
    };

    if let Some(err) = parsed.get("error") {
        let msg = err
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown API error");
        return ToolResult::Err(ToolError::Other(format!(
            "web_search: Perplexity API error: {msg}"
        )));
    }

    let formatted = format_response(&parsed);
    if formatted.trim().is_empty() {
        return ToolResult::Err(ToolError::Other(
            "web_search: Perplexity returned an empty response".into(),
        ));
    }

    ToolResult::text(formatted)
}

/// Handler for `web_search` (section 36.23).
///
/// Calls the Perplexity sonar model via their chat/completions API to
/// perform search-grounded generation. Returns the answer text plus
/// citations and search result snippets.
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

        let api_key = match std::env::var("PERPLEXITY_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => {
                return ToolResult::Err(ToolError::Other(
                    "web_search: PERPLEXITY_API_KEY environment variable is not set. \
                     Set it to your Perplexity API key to enable web search."
                        .into(),
                ));
            }
        };

        call_perplexity(&query, &api_key).await
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
    async fn missing_api_key_returns_clear_error() {
        // This test only exercises the API-key-missing path when the
        // env var is genuinely absent (the typical CI / test case).
        // We cannot safely unset env vars in a multi-threaded test, so
        // we skip gracefully when the key happens to be present.
        if std::env::var("PERPLEXITY_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .is_some()
        {
            // Key is present; nothing to test here.
            return;
        }
        let ctx = testing_ctx_with_net();
        let call = ToolCall::new("c", NAME, serde_json::json!({ "query": "test search" }));
        let res = Handler.execute(call, &ctx).await;
        match res {
            ToolResult::Err(ToolError::Other(msg)) => {
                assert!(
                    msg.contains("PERPLEXITY_API_KEY"),
                    "error should mention PERPLEXITY_API_KEY, got: {msg}"
                );
            }
            other => panic!("expected Other error about API key, got: {other:?}"),
        }
    }

    #[test]
    fn handler_name_matches_tool_def() {
        assert_eq!(Handler.name(), NAME);
    }

    // ── format_response unit tests ───────────────────────────────────────

    #[test]
    fn format_response_with_content_only() {
        let parsed = serde_json::json!({
            "choices": [{
                "message": { "content": "Rust is a systems language." }
            }]
        });
        let out = format_response(&parsed);
        assert!(out.contains("Rust is a systems language."));
    }

    #[test]
    fn format_response_with_citations() {
        let parsed = serde_json::json!({
            "choices": [{
                "message": { "content": "Answer text." }
            }],
            "citations": ["https://example.com/1", "https://example.com/2"]
        });
        let out = format_response(&parsed);
        assert!(out.contains("Answer text."));
        assert!(out.contains("[1] https://example.com/1"));
        assert!(out.contains("[2] https://example.com/2"));
    }

    #[test]
    fn format_response_with_search_results() {
        let parsed = serde_json::json!({
            "choices": [{
                "message": { "content": "Here is the info." }
            }],
            "search_results": [{
                "title": "Result Title",
                "url": "https://example.com/result",
                "content": "A snippet of text"
            }]
        });
        let out = format_response(&parsed);
        assert!(out.contains("Here is the info."));
        assert!(out.contains("Result Title"));
        assert!(out.contains("https://example.com/result"));
        assert!(out.contains("A snippet of text"));
    }

    #[test]
    fn format_response_empty_content() {
        let parsed = serde_json::json!({
            "choices": [{
                "message": { "content": "" }
            }]
        });
        let out = format_response(&parsed);
        assert!(out.trim().is_empty());
    }
}
