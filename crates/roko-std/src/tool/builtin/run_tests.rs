//! `run_tests` — run the project's tests and return structured results.
//!
//! Category: [`ToolCategory::Exec`]. Permission: read + exec.
//! Concurrency: [`ToolConcurrency::Serial`] (tests often share fixtures).
//! Idempotent: no.

use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};
use std::time::Duration;

/// Canonical `snake_case` name.
pub const NAME: &str = "run_tests";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Run the project's test suite (or a filtered subset) \
    and return structured pass/fail results.";

/// Build the [`ToolDef`] for `run_tests`.
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
            "build": {
                "type": "string",
                "enum": ["cargo", "npm", "go", "pytest", "forge", "make"],
                "description": "Build system to use for running tests (default: 'cargo')."
            },
            "filter": {
                "type": "string",
                "description": "Test name filter / pattern to run a subset of tests."
            },
            "timeout_ms": {
                "type": "integer",
                "description": "Optional timeout override in milliseconds."
            }
        },
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Serial)
    .with_idempotent(false)
    .with_timeout_ms(600_000)
}

/// Handler for `run_tests` (§36.29).
///
/// Wraps `bash` by invoking the build-system-appropriate test command
/// (`cargo test`, `npm test`, `go test ./...`, …) selected by the
/// `build` argument. Falls back to `cargo test` when unspecified.
///
/// The result structure is:
///
/// ```json
/// { "build": "...", "status": "ok"|"failed", "passed": N, "failed": N, "ignored": N }
/// ```
///
/// Full stdout is included when the tests fail.
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
                "run_tests requires exec capability".into(),
            ));
        }
        let build = call
            .arguments
            .get("build")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("cargo");
        let filter = call
            .arguments
            .get("filter")
            .and_then(serde_json::Value::as_str);
        let (program, args) = match build {
            "cargo" => ("cargo", vec!["test", "--workspace"]),
            "npm" => ("npm", vec!["test"]),
            "go" => ("go", vec!["test", "./..."]),
            "pytest" => ("python3", vec!["-m", "pytest"]),
            "forge" => ("forge", vec!["test"]),
            "make" => ("make", vec!["test"]),
            other => {
                return ToolResult::Err(ToolError::Other(format!(
                    "run_tests: unknown build system `{other}`"
                )));
            }
        };
        let mut cmd = tokio::process::Command::new(program);
        for arg in &args {
            cmd.arg(arg);
        }
        if let Some(f) = filter {
            cmd.arg(f);
        }
        cmd.current_dir(ctx.worktree());
        cmd.kill_on_drop(true);
        let effective_timeout = if ctx.timeout.is_zero() {
            Duration::from_secs(600)
        } else {
            ctx.timeout
        };
        let output = match tokio::time::timeout(effective_timeout, cmd.output()).await {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => {
                return ToolResult::Err(ToolError::Other(format!("run_tests: spawn failed: {e}")));
            }
            Err(_) => {
                return ToolResult::Err(ToolError::Timeout {
                    after_ms: u64::try_from(effective_timeout.as_millis()).unwrap_or(u64::MAX),
                });
            }
        };
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let combined = format!("{stdout}\n{stderr}");
        let counts = parse_counts(&combined);
        let payload = serde_json::json!({
            "build": build,
            "status": if output.status.success() { "ok" } else { "failed" },
            "passed": counts.passed,
            "failed": counts.failed,
            "ignored": counts.ignored,
            "output": if output.status.success() { serde_json::Value::Null } else { serde_json::Value::String(combined.clone()) },
        });
        if output.status.success() {
            ToolResult::structured(payload.to_string())
        } else {
            ToolResult::Err(ToolError::Other(format!(
                "run_tests: {} tests failed — {}",
                counts.failed,
                truncate(&combined, 400)
            )))
        }
    }
}

#[derive(Default)]
struct Counts {
    passed: u32,
    failed: u32,
    ignored: u32,
}

fn parse_counts(output: &str) -> Counts {
    let mut c = Counts::default();
    for line in output.lines() {
        let t = line.trim();
        if t.starts_with("test result:") {
            c.passed += extract(t, "passed");
            c.failed += extract(t, "failed");
            c.ignored += extract(t, "ignored");
        }
    }
    c
}

fn extract(line: &str, label: &str) -> u32 {
    for part in line.split(';') {
        let p = part.trim();
        if let Some(rest) = p.strip_suffix(label).map(str::trim_end) {
            if let Some(num) = rest.split_whitespace().last() {
                if let Ok(n) = num.parse::<u32>() {
                    return n;
                }
            }
        }
    }
    0
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_counts_reads_summary_line() {
        let out = "test result: ok. 5 passed; 2 failed; 1 ignored; 0 measured";
        let c = parse_counts(out);
        assert_eq!(c.passed, 5);
        assert_eq!(c.failed, 2);
        assert_eq!(c.ignored, 1);
    }

    #[test]
    fn parse_counts_aggregates_multiple_lines() {
        let out = "test result: ok. 3 passed; 0 failed; 0 ignored\n\
                   test result: FAILED. 2 passed; 1 failed; 0 ignored";
        let c = parse_counts(out);
        assert_eq!(c.passed, 5);
        assert_eq!(c.failed, 1);
    }

    #[test]
    fn parse_counts_zero_on_empty() {
        let c = parse_counts("");
        assert_eq!(c.passed, 0);
        assert_eq!(c.failed, 0);
        assert_eq!(c.ignored, 0);
    }

    #[test]
    fn truncate_adds_ellipsis_when_long() {
        let long = "a".repeat(500);
        let short = truncate(&long, 10);
        assert!(short.ends_with('…'));
        assert!(short.chars().count() <= 12);
    }

    #[test]
    fn truncate_passthrough_when_short() {
        assert_eq!(truncate("short", 400), "short");
    }
}
