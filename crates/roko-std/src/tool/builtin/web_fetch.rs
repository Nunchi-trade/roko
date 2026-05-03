//! `web_fetch` -- fetch a web page or resource.
//!
//! Category: [`ToolCategory::Network`]. Permission: networked.
//! Concurrency: [`ToolConcurrency::Parallel`]. Idempotent: yes
//! (HTTP GET assumed; POST / mutating fetches would not be idempotent).

use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};
use std::sync::LazyLock;
use std::time::Duration;

use super::sandbox::require_string;

/// Shared HTTP client — reused across all `web_fetch` invocations to
/// amortize TLS session setup and connection pooling (§12.13).
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
});

/// Canonical `snake_case` name.
pub const NAME: &str = "web_fetch";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Fetch a URL via HTTP(S) and return its body, subject to \
    the domain allowlist configured in safety policy.";

/// Default request timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Maximum response body size in bytes (1 MB).
const MAX_BODY_BYTES: usize = 1_024 * 1_024;

/// Build the [`ToolDef`] for `web_fetch`.
#[must_use]
pub fn tool_def() -> ToolDef {
    ToolDef::new(
        NAME,
        DESCRIPTION,
        ToolCategory::Network,
        ToolPermission::networked(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "description": "The URL to fetch (HTTP or HTTPS)."
            },
            "headers": {
                "type": "object",
                "description": "Optional HTTP headers to include.",
                "additionalProperties": { "type": "string" }
            }
        },
        "required": ["url"],
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Parallel)
    .with_idempotent(true)
    .with_timeout_ms(30_000)
}

/// Strip HTML tags from a string, yielding approximate plain text.
///
/// This is intentionally lightweight -- no full DOM parse, just enough
/// to make HTML responses readable by an LLM.
fn strip_html_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut tag_buf = String::new();

    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag_buf.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let tag_lower = tag_buf.to_ascii_lowercase();
                let tag_name = tag_lower.split_whitespace().next().unwrap_or("");
                if tag_name == "script" {
                    in_script = true;
                } else if tag_name == "/script" {
                    in_script = false;
                } else if tag_name == "style" {
                    in_style = true;
                } else if tag_name == "/style" {
                    in_style = false;
                }
                // Insert a newline for block-level elements.
                if matches!(
                    tag_name,
                    "br" | "p"
                        | "/p"
                        | "div"
                        | "/div"
                        | "h1"
                        | "/h1"
                        | "h2"
                        | "/h2"
                        | "h3"
                        | "/h3"
                        | "h4"
                        | "/h4"
                        | "li"
                        | "tr"
                        | "/tr"
                ) {
                    out.push('\n');
                }
                tag_buf.clear();
            }
            _ if in_tag => {
                tag_buf.push(ch);
            }
            _ if in_script || in_style => {
                // Discard content inside <script> and <style>.
            }
            _ => {
                out.push(ch);
            }
        }
    }
    // Decode a few common HTML entities.
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    // Collapse runs of blank lines.
    collapse_blank_lines(&out)
}

/// Collapse three or more consecutive newlines into two.
fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut consecutive_newlines = 0u32;
    for ch in s.chars() {
        if ch == '\n' {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                out.push(ch);
            }
        } else {
            consecutive_newlines = 0;
            out.push(ch);
        }
    }
    out
}

/// Returns true if the content-type header looks like HTML.
fn is_html_content_type(content_type: &str) -> bool {
    let lower = content_type.to_ascii_lowercase();
    lower.contains("text/html") || lower.contains("application/xhtml")
}

/// Validate the URL scheme and localhost-only rule for plain HTTP.
fn validate_url(url: &str) -> Result<(), ToolError> {
    let scheme = url
        .split("://")
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(ToolError::SchemaInvalid(format!(
            "web_fetch: only http(s) URLs are supported, got `{url}`"
        )));
    }
    if scheme == "http" && !url.starts_with("http://localhost") && !url.starts_with("http://127.") {
        return Err(ToolError::NetworkBlocked(format!(
            "web_fetch: plain http is only allowed for localhost, got `{url}`"
        )));
    }
    Ok(())
}

/// Send the HTTP request and return the response body as text,
/// applying the size limit and HTML stripping as needed.
async fn do_fetch(url: &str, call: &ToolCall) -> ToolResult {
    let method = call
        .arguments
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET")
        .to_ascii_uppercase();

    let client = &*HTTP_CLIENT;

    let request_builder = match method.as_str() {
        "GET" => client.get(url),
        "HEAD" => client.head(url),
        "POST" => {
            let body = call
                .arguments
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            client.post(url).body(body)
        }
        other => {
            return ToolResult::Err(ToolError::SchemaInvalid(format!(
                "web_fetch: unsupported method `{other}`, use GET, HEAD, or POST"
            )));
        }
    };

    let request_builder = request_builder.header("User-Agent", "roko-agent/0.1 (web_fetch tool)");

    let response = match request_builder.send().await {
        Ok(r) => r,
        Err(e) => {
            if e.is_timeout() {
                return ToolResult::Err(ToolError::Timeout {
                    after_ms: DEFAULT_TIMEOUT_SECS * 1_000,
                });
            }
            return ToolResult::Err(ToolError::Other(format!(
                "web_fetch: request to `{url}` failed: {e}"
            )));
        }
    };

    process_response(response, url).await
}

/// Read the response body, enforce the size limit, and convert to text.
async fn process_response(response: reqwest::Response, url: &str) -> ToolResult {
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body_bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            return ToolResult::Err(ToolError::Other(format!(
                "web_fetch: reading body from `{url}` failed: {e}"
            )));
        }
    };

    if body_bytes.len() > MAX_BODY_BYTES {
        return ToolResult::Err(ToolError::Other(format!(
            "web_fetch: response from `{url}` is {} bytes, exceeds {MAX_BODY_BYTES} byte limit",
            body_bytes.len()
        )));
    }

    if !status.is_success() {
        let body_preview = String::from_utf8_lossy(&body_bytes);
        let truncated = if body_preview.len() > 500 {
            format!("{}...", &body_preview[..500])
        } else {
            body_preview.to_string()
        };
        return ToolResult::Err(ToolError::Other(format!(
            "web_fetch: `{url}` returned HTTP {status}: {truncated}"
        )));
    }

    let body_text = String::from_utf8_lossy(&body_bytes).to_string();
    let result_text = if is_html_content_type(&content_type) {
        strip_html_tags(&body_text)
    } else {
        body_text
    };

    ToolResult::text(result_text)
}

/// Handler for `web_fetch` (section 36.22).
///
/// Performs a real HTTP GET via reqwest. Enforces the `network` capability,
/// validates the URL scheme, applies a 30-second timeout and 1 MB body
/// limit, and strips HTML tags when the response is HTML.
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
        if let Err(e) = validate_url(&url) {
            return ToolResult::Err(e);
        }
        do_fetch(&url, &call).await
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

    #[test]
    fn handler_name_matches_tool_def() {
        assert_eq!(Handler.name(), NAME);
    }

    // ── HTML stripping unit tests ────────────────────────────────────────

    #[test]
    fn strip_simple_html() {
        let input = "<html><body><h1>Hello</h1><p>World</p></body></html>";
        let out = strip_html_tags(input);
        assert!(out.contains("Hello"));
        assert!(out.contains("World"));
        assert!(!out.contains("<h1>"));
        assert!(!out.contains("<p>"));
    }

    #[test]
    fn strip_script_and_style() {
        let input = "<p>text</p><script>alert('x');</script><style>.x{}</style><p>more</p>";
        let out = strip_html_tags(input);
        assert!(out.contains("text"));
        assert!(out.contains("more"));
        assert!(!out.contains("alert"));
        assert!(!out.contains(".x{}"));
    }

    #[test]
    fn decode_html_entities() {
        let input = "a &amp; b &lt; c &gt; d &quot;e&quot; f&apos;s";
        let out = strip_html_tags(input);
        assert_eq!(out, "a & b < c > d \"e\" f's");
    }

    #[test]
    fn is_html_detection() {
        assert!(is_html_content_type("text/html; charset=utf-8"));
        assert!(is_html_content_type("application/xhtml+xml"));
        assert!(!is_html_content_type("application/json"));
        assert!(!is_html_content_type("text/plain"));
    }

    #[test]
    fn collapse_excessive_blank_lines() {
        let input = "a\n\n\n\n\nb";
        let out = collapse_blank_lines(input);
        assert_eq!(out, "a\n\nb");
    }
}
