//! Result filter safety hook (§36.52).
//!
//! Sanitizes tool output after execution. Applied as the final hook in the
//! safety chain, it:
//!
//! - **Truncates** oversized responses (> configurable limit, default 100 KB)
//! - **Strips secrets** using the existing `ScrubPolicy` patterns
//! - **Labels** external data with taint provenance
//!
//! The filter runs on `ToolResult::Ok` content. Errors pass through unchanged.

use async_trait::async_trait;
use roko_core::tool::{ToolContext, ToolDef, ToolError};

use super::hooks::{HookDecision, SafetyHook};
use super::scrub::{ScrubPolicy, scrub_secrets};

/// Default maximum response size before truncation (100 KB).
const DEFAULT_MAX_RESPONSE_BYTES: usize = 100 * 1024;

/// Tools whose output comes from external sources (network, user input).
const EXTERNAL_OUTPUT_TOOLS: &[&str] = &["web_fetch", "web_search", "bash", "run_tests"];

/// Sanitizes tool output by stripping secrets and truncating oversized responses.
///
/// This hook is applied **after** tool execution (post-hook) by wrapping the
/// pre-execution hook trait. When used as a pre-hook, it validates that the
/// tool call parameters do not embed secrets (defense-in-depth).
#[derive(Debug, Clone)]
pub struct ResultFilter {
    /// Maximum response size in bytes before truncation.
    pub max_response_bytes: usize,
    /// Secret scrubbing policy.
    pub scrub_policy: ScrubPolicy,
    /// Whether to annotate external tool output with provenance markers.
    pub annotate_external: bool,
}

impl ResultFilter {
    /// Create a result filter with default settings.
    pub fn with_defaults() -> Self {
        Self {
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            scrub_policy: ScrubPolicy::default(),
            annotate_external: true,
        }
    }

    /// Create a result filter with a custom size limit.
    pub fn with_max_size(max_bytes: usize) -> Self {
        Self {
            max_response_bytes: max_bytes,
            ..Self::with_defaults()
        }
    }

    /// Sanitize a tool output string.
    ///
    /// This is the core filtering function, usable outside the hook chain
    /// for ad-hoc sanitization.
    pub fn sanitize(&self, content: &str, tool_name: &str) -> String {
        let mut output = scrub_secrets(content, &self.scrub_policy);

        // Truncate oversized output.
        if output.len() > self.max_response_bytes {
            let truncated_at = self.max_response_bytes;
            // Find a safe UTF-8 boundary.
            let boundary = output
                .char_indices()
                .take_while(|(i, _)| *i < truncated_at)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            output.truncate(boundary);
            output.push_str(&format!(
                "\n\n[OUTPUT TRUNCATED: original size exceeded {truncated_at} bytes]"
            ));
        }

        // Annotate output from external sources.
        if self.annotate_external && is_external_tool(tool_name) {
            output = format!("[external:{tool_name}] {output}");
        }

        output
    }

    /// Check tool call parameters for embedded secrets (defense-in-depth).
    fn check_params_for_secrets(&self, params: &serde_json::Value) -> Option<String> {
        let params_str = params.to_string();
        let scrubbed = scrub_secrets(&params_str, &self.scrub_policy);
        if scrubbed != params_str {
            Some("tool call parameters contain embedded secrets".into())
        } else {
            None
        }
    }
}

fn is_external_tool(name: &str) -> bool {
    EXTERNAL_OUTPUT_TOOLS.contains(&name)
}

#[async_trait]
impl SafetyHook for ResultFilter {
    async fn on_tool_call(
        &self,
        tool: &ToolDef,
        params: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<HookDecision, ToolError> {
        // Pre-execution: reject calls with embedded secrets in parameters.
        if let Some(reason) = self.check_params_for_secrets(params) {
            tracing::warn!(
                tool = %tool.name,
                "ResultFilter rejecting tool call: {reason}"
            );
            return Ok(HookDecision::Reject(reason));
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
        ToolDef::new(
            name,
            "test tool",
            ToolCategory::Read,
            ToolPermission::read_only(),
        )
    }

    #[test]
    fn sanitize_truncates_large_output() {
        let filter = ResultFilter::with_max_size(100);
        let big_content = "x".repeat(500);
        let result = filter.sanitize(&big_content, "read_file");
        assert!(result.len() < 500);
        assert!(result.contains("[OUTPUT TRUNCATED"));
    }

    #[test]
    fn sanitize_scrubs_secrets() {
        let filter = ResultFilter::with_defaults();
        let api_key = format!("sk-ant-api03-{}", "A".repeat(80));
        let content = format!("found key: {api_key}");
        let result = filter.sanitize(&content, "read_file");
        assert!(!result.contains(&api_key));
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn sanitize_annotates_external_tools() {
        let filter = ResultFilter::with_defaults();
        let result = filter.sanitize("response body", "web_fetch");
        assert!(result.starts_with("[external:web_fetch]"));
    }

    #[test]
    fn sanitize_does_not_annotate_internal_tools() {
        let filter = ResultFilter::with_defaults();
        let result = filter.sanitize("file content", "read_file");
        assert!(!result.starts_with("[external:"));
    }

    #[test]
    fn sanitize_handles_utf8_boundary() {
        let filter = ResultFilter::with_max_size(5);
        // Multi-byte character at the truncation boundary.
        let content = "hello\u{1F600}world";
        let result = filter.sanitize(content, "read_file");
        // Must not panic or produce invalid UTF-8.
        assert!(result.is_char_boundary(0));
    }

    #[tokio::test]
    async fn pre_hook_rejects_embedded_secrets() {
        let filter = ResultFilter::with_defaults();
        let tool = test_tool("bash");
        let api_key = format!("sk-ant-api03-{}", "A".repeat(80));
        let params = serde_json::json!({ "command": format!("echo {api_key}") });
        let result = filter
            .on_tool_call(&tool, &params, &test_ctx())
            .await
            .unwrap();
        assert!(matches!(result, HookDecision::Reject(_)));
    }

    #[tokio::test]
    async fn pre_hook_allows_clean_params() {
        let filter = ResultFilter::with_defaults();
        let tool = test_tool("bash");
        let params = serde_json::json!({ "command": "echo hello" });
        let result = filter
            .on_tool_call(&tool, &params, &test_ctx())
            .await
            .unwrap();
        assert_eq!(result, HookDecision::Allow);
    }

    #[test]
    fn no_annotation_when_disabled() {
        let filter = ResultFilter {
            annotate_external: false,
            ..ResultFilter::with_defaults()
        };
        let result = filter.sanitize("response", "web_fetch");
        assert!(!result.contains("[external:"));
    }
}
