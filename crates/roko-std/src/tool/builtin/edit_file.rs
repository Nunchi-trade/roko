//! `edit_file` — replace an exact string in a file.
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
pub const NAME: &str = "edit_file";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Replace an exact string in a file with a new string.";

/// Build the [`ToolDef`] for `edit_file`.
#[must_use]
pub fn tool_def() -> ToolDef {
    ToolDef::new(
        NAME,
        DESCRIPTION,
        ToolCategory::Write,
        ToolPermission::writes(),
    )
    .with_parameters(ToolSchema::any_object())
    .with_concurrency(ToolConcurrency::Serial)
    .with_idempotent(false)
    .with_timeout_ms(30_000)
}

/// Handler for `edit_file` (§36.16).
///
/// Replaces `old_string` with `new_string` in the file at `path`. If
/// `replace_all` is `false` (default), the edit fails when `old_string`
/// occurs more than once (ambiguity guard, matching Claude's `Edit` tool
/// semantics).
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
        let old_string = match require_string(&call.arguments, "old_string") {
            Ok(s) => s,
            Err(e) => return ToolResult::Err(e),
        };
        let new_string = match require_string(&call.arguments, "new_string") {
            Ok(s) => s,
            Err(e) => return ToolResult::Err(e),
        };
        let replace_all = call
            .arguments
            .get("replace_all")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let absolute = match require_within_worktree(ctx.worktree(), &rel_path) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        let existing = match tokio::fs::read_to_string(&absolute).await {
            Ok(s) => s,
            Err(e) => {
                return ToolResult::Err(ToolError::Other(format!(
                    "edit_file: failed to read {}: {e}",
                    absolute.display()
                )));
            }
        };
        let count = existing.matches(old_string.as_str()).count();
        if count == 0 {
            return ToolResult::Err(ToolError::Other(format!(
                "edit_file: old_string not found in {}",
                absolute.display()
            )));
        }
        if count > 1 && !replace_all {
            return ToolResult::Err(ToolError::Other(format!(
                "edit_file: old_string occurs {count} times in {} — set replace_all:true to replace all, or disambiguate",
                absolute.display()
            )));
        }
        let replaced = if replace_all {
            existing.replace(old_string.as_str(), &new_string)
        } else {
            existing.replacen(old_string.as_str(), &new_string, 1)
        };
        match tokio::fs::write(&absolute, replaced.as_bytes()).await {
            Ok(()) => {
                ToolResult::text(format!("edited {} ({count} match(es))", absolute.display()))
            }
            Err(e) => ToolResult::Err(ToolError::Other(format!(
                "edit_file: failed to write {}: {e}",
                absolute.display()
            ))),
        }
    }
}
