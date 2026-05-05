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
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "command": {
                "type": "string",
                "description": "The shell command to execute via `bash -c`."
            },
            "timeout_ms": {
                "type": "integer",
                "description": "Optional wall-clock timeout in milliseconds (default: 120000)."
            }
        },
        "required": ["command"],
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Serial)
    .with_idempotent(false)
    .with_timeout_ms(120_000)
}

// Command-level safety (denylist, path confinement) is enforced by the
// SafetyLayer's `BashPolicy` before this handler is invoked. No second-
// tier check here — a single authoritative policy avoids divergence.

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
        let effective_timeout = if ctx.timeout.is_zero() {
            Duration::from_secs(120)
        } else {
            ctx.timeout
        };
        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c").arg(&command);
        cmd.current_dir(ctx.worktree());
        cmd.kill_on_drop(true);

        // Scrub secrets from the child environment. Inheriting the full parent
        // env exposes API keys, SSH agent sockets, and other credentials to
        // agent-controlled commands.
        cmd.env_clear();
        let safe_env_keys = [
            "PATH", "HOME", "TMPDIR", "TEMP", "TMP", "TERM", "LANG", "LC_ALL", "USER", "LOGNAME",
            "SHELL",
        ];
        for key in &safe_env_keys {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }

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

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use roko_core::tool::ToolContext;

    /// Verify that the env scrubbing drops secret-containing env vars while
    /// preserving the safe set (PATH, HOME, etc.).
    #[tokio::test]
    async fn env_scrubbing_hides_secrets_preserves_safe_keys() {
        // Temporarily set secret env vars in this process. The handler
        // should NOT propagate them to the child process.
        unsafe {
            std::env::set_var("OPENAI_API_KEY", "sk-secret-openai-test-key");
            std::env::set_var("MY_SECRET_TOKEN", "tok-super-secret");
        }

        let handler = Handler;
        let call = ToolCall::new("c", NAME, serde_json::json!({"command": "env"}));
        let mut ctx = ToolContext::testing("/tmp");
        ctx.capabilities.exec = true;

        let result = handler.execute(call, &ctx).await;
        match result {
            ToolResult::Ok { content, .. } => {
                // Secrets must NOT appear.
                assert!(
                    !content.contains("sk-secret-openai-test-key"),
                    "OPENAI_API_KEY leaked into child env"
                );
                assert!(
                    !content.contains("tok-super-secret"),
                    "MY_SECRET_TOKEN leaked into child env"
                );
                // Safe keys SHOULD be present (PATH is almost certainly set).
                assert!(
                    content.contains("PATH="),
                    "PATH should be inherited by the child process"
                );
            }
            ToolResult::Err(e) => {
                panic!("bash handler failed unexpectedly: {e}");
            }
        }

        // Clean up the test env vars.
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("MY_SECRET_TOKEN");
        }
    }
}
