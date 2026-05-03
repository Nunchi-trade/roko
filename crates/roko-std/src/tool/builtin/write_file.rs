//! `write_file` — write a file to the worktree, creating or replacing it.
//!
//! Category: [`ToolCategory::Write`]. Permission: read + write.
//! Concurrency: [`ToolConcurrency::Serial`]. Idempotent: no.

use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};

use super::sandbox::{require_string, require_within_worktree};

/// Canonical `snake_case` name.
pub const NAME: &str = "write_file";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Write the provided content to a file in the worktree, \
    creating it if it does not exist or replacing its contents.";

/// Build the [`ToolDef`] for `write_file`.
#[must_use]
pub fn tool_def() -> ToolDef {
    ToolDef::new(
        NAME,
        DESCRIPTION,
        ToolCategory::Write,
        ToolPermission::writes(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Relative path within the worktree to write."
            },
            "content": {
                "type": "string",
                "description": "The full file content to write."
            }
        },
        "required": ["path", "content"],
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Serial)
    .with_idempotent(false)
    .with_timeout_ms(30_000)
}

/// Handler for `write_file` (§36.15).
#[derive(Debug, Clone, Copy, Default)]
pub struct Handler;

#[async_trait]
impl ToolHandler for Handler {
    fn name(&self) -> &str {
        NAME
    }

    async fn execute(&self, call: ToolCall, ctx: &ToolContext) -> ToolResult {
        let rel_path = match require_string(&call.arguments, "path") {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        let content = match require_string(&call.arguments, "content") {
            Ok(s) => s,
            Err(e) => return ToolResult::Err(e),
        };
        let absolute = match require_within_worktree(ctx.worktree(), &rel_path) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        if let Some(parent) = absolute.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ToolResult::Err(ToolError::Other(format!(
                    "write_file: failed to create parent {}: {e}",
                    parent.display()
                )));
            }
        }
        match tokio::fs::write(&absolute, content.as_bytes()).await {
            Ok(()) => ToolResult::text(format!(
                "wrote {} bytes to {}",
                content.len(),
                absolute.display()
            )),
            Err(e) => ToolResult::Err(ToolError::Other(format!(
                "write_file failed ({}): {e}",
                absolute.display()
            ))),
        }
    }
}
