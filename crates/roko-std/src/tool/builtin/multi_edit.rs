//! `multi_edit` — apply multiple exact-string replacements to a file atomically.
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
pub const NAME: &str = "multi_edit";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str =
    "Apply multiple exact-string replacements to a single file atomically.";

/// Build the [`ToolDef`] for `multi_edit`.
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
                "description": "Relative path within the worktree to edit."
            },
            "edits": {
                "type": "array",
                "description": "Array of edit operations applied atomically in order.",
                "items": {
                    "type": "object",
                    "properties": {
                        "old_string": { "type": "string", "description": "Exact string to find." },
                        "new_string": { "type": "string", "description": "Replacement string." },
                        "replace_all": { "type": "boolean", "description": "Replace all occurrences (default: false)." }
                    },
                    "required": ["old_string", "new_string"]
                }
            }
        },
        "required": ["path", "edits"],
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Serial)
    .with_idempotent(false)
    .with_timeout_ms(30_000)
}

/// Handler for `multi_edit` (§36.17).
///
/// Accepts an array `edits` of `{old_string, new_string, replace_all?}`
/// objects and applies them to `path` in order. If **any** edit fails
/// (ambiguous match without `replace_all`, `old_string` absent after a
/// prior edit rewrote it, etc.), the entire operation is aborted and
/// the file is left unchanged — "atomic" in the all-or-nothing sense.
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
        let Some(edits) = call
            .arguments
            .get("edits")
            .and_then(serde_json::Value::as_array)
        else {
            return ToolResult::Err(ToolError::SchemaInvalid(
                "multi_edit: missing required array argument `edits`".into(),
            ));
        };
        let absolute = match require_within_worktree(ctx.worktree(), &rel_path) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        let original = match tokio::fs::read_to_string(&absolute).await {
            Ok(s) => s,
            Err(e) => {
                return ToolResult::Err(ToolError::Other(format!(
                    "multi_edit: failed to read {}: {e}",
                    absolute.display()
                )));
            }
        };
        let mut working = original;
        for (idx, edit) in edits.iter().enumerate() {
            let Some(old_string) = edit.get("old_string").and_then(serde_json::Value::as_str)
            else {
                return ToolResult::Err(ToolError::SchemaInvalid(format!(
                    "multi_edit: edits[{idx}].old_string is required"
                )));
            };
            let Some(new_string) = edit.get("new_string").and_then(serde_json::Value::as_str)
            else {
                return ToolResult::Err(ToolError::SchemaInvalid(format!(
                    "multi_edit: edits[{idx}].new_string is required"
                )));
            };
            let replace_all = edit
                .get("replace_all")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let count = working.matches(old_string).count();
            if count == 0 {
                return ToolResult::Err(ToolError::Other(format!(
                    "multi_edit: edits[{idx}].old_string not found"
                )));
            }
            if count > 1 && !replace_all {
                return ToolResult::Err(ToolError::Other(format!(
                    "multi_edit: edits[{idx}].old_string occurs {count} times — set replace_all:true"
                )));
            }
            working = if replace_all {
                working.replace(old_string, new_string)
            } else {
                working.replacen(old_string, new_string, 1)
            };
        }
        match tokio::fs::write(&absolute, working.as_bytes()).await {
            Ok(()) => ToolResult::text(format!(
                "applied {} edits to {}",
                edits.len(),
                absolute.display()
            )),
            Err(e) => ToolResult::Err(ToolError::Other(format!(
                "multi_edit: failed to write {}: {e}",
                absolute.display()
            ))),
        }
    }
}
