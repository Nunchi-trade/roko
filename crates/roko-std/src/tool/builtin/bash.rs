//! `bash` — execute a shell command.
//!
//! Category: [`ToolCategory::Exec`]. Permission: read + exec.
//! Concurrency: [`ToolConcurrency::Serial`] (shared shell state).
//! Idempotent: no.

use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};
use std::time::Duration;

use super::sandbox::require_string;

/// Canonical `snake_case` name.
pub const NAME: &str = "bash";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Execute a shell command via `bash -c` and return its output.";

/// Build the [`ToolDef`] for `bash`.
#[must_use]
pub fn tool_def() -> ToolDef {
    ToolDef::new(
        NAME,
        DESCRIPTION,
        ToolCategory::Exec,
        ToolPermission::executes(),
    )
    .with_parameters(ToolSchema::any_object())
    .with_concurrency(ToolConcurrency::Serial)
    .with_idempotent(false)
    .with_timeout_ms(120_000)
}

/// Default blocklist: patterns this handler refuses outright. Agents can
/// expand/narrow this via the safety-policy layer in §36.46; here we only
/// stop the most obviously dangerous invocations.
const DEFAULT_DENY_SUBSTRINGS: &[&str] = &[
    "rm -rf /",
    "sudo ",
    "dd if=/dev/",
    ":(){ :|:& };:", // fork bomb
    "mkfs.",
];

/// Handler for `bash` (§36.20).
///
/// Spawns `bash -c <command>` in the worktree with
/// [`ToolContext::timeout`] as the wall-clock budget. The command's
/// combined stdout+stderr is returned as the tool result's content.
///
/// Per the sandbox contract, the handler only runs when
/// [`ToolPermission::exec`] is granted; safety policies layer additional
/// allowlist/blocklist checks on top.
#[derive(Debug, Clone, Copy, Default)]
pub struct Handler;

#[async_trait]
impl ToolHandler for Handler {
    fn name(&self) -> &str {
        NAME
    }

    async fn execute(&self, call: ToolCall, ctx: &ToolContext) -> ToolResult {
        if !ctx.capabilities.exec {
            return ToolResult::Err(ToolError::PermissionDenied(
                "bash requires exec capability".into(),
            ));
        }
        let command = match require_string(&call.arguments, "command") {
            Ok(c) => c,
            Err(e) => return ToolResult::Err(e),
        };
        for needle in DEFAULT_DENY_SUBSTRINGS {
            if command.contains(needle) {
                return ToolResult::Err(ToolError::CommandNotAllowed(command));
            }
        }
        let effective_timeout = if ctx.timeout.is_zero() {
            Duration::from_secs(120)
        } else {
            ctx.timeout
        };
        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c").arg(&command);
        cmd.current_dir(ctx.worktree());
        cmd.kill_on_drop(true);
        let output = match tokio::time::timeout(effective_timeout, cmd.output()).await {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => {
                return ToolResult::Err(ToolError::Other(format!("bash: spawn failed: {e}")));
            }
            Err(_) => {
                return ToolResult::Err(ToolError::Timeout {
                    after_ms: u64::try_from(effective_timeout.as_millis()).unwrap_or(u64::MAX),
                });
            }
        };
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let combined = if stderr.is_empty() {
            stdout
        } else {
            format!("{stdout}{stderr}")
        };
        if output.status.success() {
            ToolResult::text(combined)
        } else {
            ToolResult::Err(ToolError::Other(format!(
                "bash: exited with status {:?}: {combined}",
                output.status.code()
            )))
        }
    }
}
