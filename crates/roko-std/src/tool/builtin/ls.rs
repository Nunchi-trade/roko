//! `ls` — list directory contents.
//!
//! Category: [`ToolCategory::Read`]. Permission: read-only.
//! Concurrency: [`ToolConcurrency::Parallel`]. Idempotent: yes.

use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};

use super::sandbox::require_within_worktree;

/// Canonical `snake_case` name.
pub const NAME: &str = "ls";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "List the contents of a directory in the worktree.";

/// Build the [`ToolDef`] for `ls`.
#[must_use]
pub fn tool_def() -> ToolDef {
    ToolDef::new(
        NAME,
        DESCRIPTION,
        ToolCategory::Read,
        ToolPermission::read_only(),
    )
    .with_parameters(ToolSchema::any_object())
    .with_concurrency(ToolConcurrency::Parallel)
    .with_idempotent(true)
    .with_timeout_ms(10_000)
}

/// Handler for `ls` (§36.21).
///
/// Lists entries in the directory named by `path` (default: ".").
/// Each line is `d|f|l\tNAME\tSIZE_BYTES` (type marker, name, size).
#[derive(Debug, Clone, Copy, Default)]
pub struct Handler;

#[async_trait]
impl ToolHandler for Handler {
    fn name(&self) -> &str {
        NAME
    }

    async fn execute(&self, call: ToolCall, ctx: &ToolContext) -> ToolResult {
        let rel_path = call
            .arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(".");
        let absolute = match require_within_worktree(ctx.worktree(), rel_path) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        let mut entries = match tokio::fs::read_dir(&absolute).await {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::Err(ToolError::Other(format!(
                    "ls: read_dir({}) failed: {e}",
                    absolute.display()
                )));
            }
        };
        let mut lines: Vec<String> = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            let marker = if meta.is_dir() {
                'd'
            } else if meta.is_symlink() {
                'l'
            } else {
                'f'
            };
            lines.push(format!("{marker}\t{name}\t{}", meta.len()));
        }
        lines.sort();
        ToolResult::text(lines.join("\n"))
    }
}
