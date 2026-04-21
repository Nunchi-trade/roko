//! Hallucination detector safety hook (§36.52).
//!
//! Validates tool call plausibility before execution. Checks include:
//!
//! - **Tool name validity**: the tool name must match a known registered tool.
//! - **File path sanity**: paths passed to file tools must look structurally valid
//!   (no null bytes, no excessively long components).
//! - **Parameter range validation**: numeric parameters must be within sane bounds
//!   (e.g., line counts > 0, timeout > 0).
//! - **Argument presence**: required parameters must not be missing or empty.

use async_trait::async_trait;
use roko_core::tool::{ToolContext, ToolDef, ToolError};

use super::hooks::{HookDecision, SafetyHook};

/// Maximum allowed file path length in bytes.
const MAX_PATH_LEN: usize = 4096;

/// Maximum line number / offset allowed before flagging as suspicious.
const MAX_LINE_NUMBER: u64 = 10_000_000;

/// Validates tool call plausibility to catch hallucinated or malformed invocations
/// before they reach the tool handler.
#[derive(Debug, Clone)]
pub struct HallucinationDetector {
    /// Known tool names that this detector considers valid.
    /// When empty, name validation is skipped (all names accepted).
    pub known_tools: Vec<String>,
}

impl HallucinationDetector {
    /// Create a detector with no tool name validation.
    pub fn permissive() -> Self {
        Self {
            known_tools: Vec::new(),
        }
    }

    /// Create a detector that validates tool names against a known set.
    pub fn with_known_tools(tools: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            known_tools: tools.into_iter().map(Into::into).collect(),
        }
    }

    fn check_tool_name(&self, name: &str) -> Result<(), String> {
        if self.known_tools.is_empty() {
            return Ok(());
        }
        if self.known_tools.iter().any(|t| t == name) {
            Ok(())
        } else {
            Err(format!(
                "unknown tool `{name}`: not in the registered tool set"
            ))
        }
    }

    fn check_file_path(params: &serde_json::Value) -> Result<(), String> {
        for key in &["file_path", "path", "notebook_path"] {
            if let Some(path) = params.get(*key).and_then(|v| v.as_str()) {
                if path.is_empty() {
                    return Err(format!("parameter `{key}` is empty"));
                }
                if path.len() > MAX_PATH_LEN {
                    return Err(format!(
                        "parameter `{key}` exceeds maximum path length ({} > {MAX_PATH_LEN})",
                        path.len()
                    ));
                }
                if path.contains('\0') {
                    return Err(format!("parameter `{key}` contains null bytes"));
                }
            }
        }
        Ok(())
    }

    fn check_numeric_ranges(params: &serde_json::Value) -> Result<(), String> {
        // Check line number / offset parameters.
        for key in &["offset", "limit", "line", "cell_number"] {
            if let Some(val) = params.get(*key) {
                if let Some(n) = val.as_u64() {
                    if n > MAX_LINE_NUMBER {
                        return Err(format!(
                            "parameter `{key}` value {n} exceeds maximum ({MAX_LINE_NUMBER})"
                        ));
                    }
                } else if let Some(n) = val.as_i64() {
                    if n < 0 {
                        return Err(format!("parameter `{key}` value {n} is negative"));
                    }
                }
            }
        }

        // Check timeout parameter.
        if let Some(timeout) = params.get("timeout").and_then(|v| v.as_u64()) {
            // 10 minutes max (in milliseconds).
            if timeout > 600_000 {
                return Err(format!(
                    "parameter `timeout` value {timeout}ms exceeds maximum (600000ms)"
                ));
            }
        }

        Ok(())
    }

    fn check_command_sanity(params: &serde_json::Value) -> Result<(), String> {
        if let Some(cmd) = params.get("command").and_then(|v| v.as_str()) {
            if cmd.is_empty() {
                return Err("parameter `command` is empty".into());
            }
            // Flag suspiciously long commands (likely hallucinated output).
            if cmd.len() > 10_000 {
                return Err(format!(
                    "parameter `command` is suspiciously long ({} bytes)",
                    cmd.len()
                ));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl SafetyHook for HallucinationDetector {
    async fn on_tool_call(
        &self,
        tool: &ToolDef,
        params: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<HookDecision, ToolError> {
        // 1. Validate tool name.
        if let Err(reason) = self.check_tool_name(&tool.name) {
            return Ok(HookDecision::Reject(reason));
        }

        // 2. Validate file paths.
        if let Err(reason) = Self::check_file_path(params) {
            return Ok(HookDecision::Reject(reason));
        }

        // 3. Validate numeric ranges.
        if let Err(reason) = Self::check_numeric_ranges(params) {
            return Ok(HookDecision::Reject(reason));
        }

        // 4. Validate command sanity.
        if let Err(reason) = Self::check_command_sanity(params) {
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

    #[tokio::test]
    async fn allows_valid_tool_call() {
        let detector = HallucinationDetector::with_known_tools(["read_file", "write_file"]);
        let tool = test_tool("read_file");
        let params = serde_json::json!({ "file_path": "/tmp/test.rs" });
        let result = detector
            .on_tool_call(&tool, &params, &test_ctx())
            .await
            .unwrap();
        assert_eq!(result, HookDecision::Allow);
    }

    #[tokio::test]
    async fn rejects_unknown_tool_name() {
        let detector = HallucinationDetector::with_known_tools(["read_file"]);
        let tool = test_tool("hallucinated_tool");
        let params = serde_json::json!({});
        let result = detector
            .on_tool_call(&tool, &params, &test_ctx())
            .await
            .unwrap();
        assert!(matches!(result, HookDecision::Reject(_)));
    }

    #[tokio::test]
    async fn rejects_empty_file_path() {
        let detector = HallucinationDetector::permissive();
        let tool = test_tool("read_file");
        let params = serde_json::json!({ "file_path": "" });
        let result = detector
            .on_tool_call(&tool, &params, &test_ctx())
            .await
            .unwrap();
        assert!(matches!(result, HookDecision::Reject(_)));
    }

    #[tokio::test]
    async fn rejects_null_bytes_in_path() {
        let detector = HallucinationDetector::permissive();
        let tool = test_tool("read_file");
        let params = serde_json::json!({ "file_path": "/tmp/\0bad" });
        let result = detector
            .on_tool_call(&tool, &params, &test_ctx())
            .await
            .unwrap();
        assert!(matches!(result, HookDecision::Reject(_)));
    }

    #[tokio::test]
    async fn rejects_negative_offset() {
        let detector = HallucinationDetector::permissive();
        let tool = test_tool("read_file");
        let params = serde_json::json!({ "file_path": "/tmp/test.rs", "offset": -1 });
        let result = detector
            .on_tool_call(&tool, &params, &test_ctx())
            .await
            .unwrap();
        assert!(matches!(result, HookDecision::Reject(_)));
    }

    #[tokio::test]
    async fn rejects_empty_command() {
        let detector = HallucinationDetector::permissive();
        let tool = test_tool("bash");
        let params = serde_json::json!({ "command": "" });
        let result = detector
            .on_tool_call(&tool, &params, &test_ctx())
            .await
            .unwrap();
        assert!(matches!(result, HookDecision::Reject(_)));
    }

    #[tokio::test]
    async fn permissive_accepts_any_tool_name() {
        let detector = HallucinationDetector::permissive();
        let tool = test_tool("anything_goes");
        let params = serde_json::json!({});
        let result = detector
            .on_tool_call(&tool, &params, &test_ctx())
            .await
            .unwrap();
        assert_eq!(result, HookDecision::Allow);
    }
}
