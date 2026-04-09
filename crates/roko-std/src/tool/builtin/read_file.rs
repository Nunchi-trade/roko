//! `read_file` — read a UTF-8 file from the worktree.
//!
//! Category: [`ToolCategory::Read`]. Permission: read-only.
//! Concurrency: [`ToolConcurrency::Parallel`]. Idempotent: yes.

use async_trait::async_trait;
use roko_core::tool::{
    Artifact, ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError,
    ToolHandler, ToolPermission, ToolResult, ToolSchema,
};

use super::sandbox::{require_string, require_within_worktree};

/// Canonical `snake_case` name.
pub const NAME: &str = "read_file";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Read the contents of a UTF-8 file from the worktree.";

/// Build the [`ToolDef`] for `read_file`.
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
    .with_timeout_ms(30_000)
}

/// Handler for `read_file` (§36.14).
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
        let absolute = match require_within_worktree(ctx.worktree(), &rel_path) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        match tokio::fs::read_to_string(&absolute).await {
            Ok(content) => ToolResult::with_artifacts(
                content,
                vec![Artifact::new(
                    absolute.display().to_string(),
                    "text/plain",
                    roko_core::Body::text(String::new()),
                )],
            ),
            Err(e) => ToolResult::Err(ToolError::Other(format!(
                "read_file failed ({}): {e}",
                absolute.display()
            ))),
        }
    }
}
