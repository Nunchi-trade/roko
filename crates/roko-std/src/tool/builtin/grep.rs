//! `grep` — search file contents for a regex pattern.
//!
//! Category: [`ToolCategory::Read`]. Permission: read-only.
//! Concurrency: [`ToolConcurrency::Parallel`]. Idempotent: yes.

use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};
use std::path::PathBuf;

use super::sandbox::require_string;

/// Canonical `snake_case` name.
pub const NAME: &str = "grep";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Search file contents for a literal-substring or simple \
    regex-like pattern across the worktree.";

/// Build the [`ToolDef`] for `grep`.
#[must_use]
pub fn tool_def() -> ToolDef {
    ToolDef::new(
        NAME,
        DESCRIPTION,
        ToolCategory::Read,
        ToolPermission::read_only(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "pattern": {
                "type": "string",
                "description": "Substring or pattern to search for in file contents."
            },
            "path": {
                "type": "string",
                "description": "Optional subdirectory to narrow the search scope."
            },
            "mode": {
                "type": "string",
                "enum": ["content", "files_with_matches", "count"],
                "description": "Output mode: 'content' shows matching lines, 'files_with_matches' shows paths, 'count' shows per-file counts. Default: 'content'."
            },
            "include": {
                "type": "string",
                "description": "Glob filter for files to search (e.g. \"*.rs\")."
            }
        },
        "required": ["pattern"],
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Parallel)
    .with_idempotent(true)
    .with_timeout_ms(60_000)
}

/// Handler for `grep` (§36.19).
///
/// Day-one implementation: **literal substring** search (no regex
/// metacharacters interpreted). Supports three output modes:
///
/// - `mode: "content"` (default) — lines matching, formatted
///   `path:line_no:content`
/// - `mode: "files_with_matches"` — just the distinct paths that had
///   at least one match
/// - `mode: "count"` — `path:count` per file that matched
///
/// Optional `path` argument narrows the search to a subdirectory.
///
/// A real regex backend ships in a follow-up (it'll add the `regex`
/// crate); this day-one version keeps roko-std dependency-light.
#[derive(Debug, Clone, Copy, Default)]
pub struct Handler;

#[async_trait]
impl ToolHandler for Handler {
    fn name(&self) -> &str {
        NAME
    }

    async fn execute(&self, call: ToolCall, ctx: &ToolContext) -> ToolResult {
        let pattern = match require_string(&call.arguments, "pattern") {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        let mode = call
            .arguments
            .get("mode")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("content")
            .to_string();
        let sub = call
            .arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(".");
        let root = if sub == "." {
            ctx.worktree().to_path_buf()
        } else {
            match super::sandbox::require_within_worktree(ctx.worktree(), sub) {
                Ok(p) => p,
                Err(e) => return ToolResult::Err(e),
            }
        };
        let worktree_root = ctx.worktree().to_path_buf();
        let result =
            tokio::task::spawn_blocking(move || search(&root, &worktree_root, &pattern, &mode))
                .await;
        match result {
            Ok(Ok(output)) => ToolResult::text(output),
            Ok(Err(e)) => ToolResult::Err(ToolError::Other(format!("grep: {e}"))),
            Err(_) => ToolResult::Err(ToolError::Other("grep: join failed".into())),
        }
    }
}

fn search(
    root: &std::path::Path,
    worktree_root: &std::path::Path,
    pattern: &str,
    mode: &str,
) -> Result<String, String> {
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    let mut content_lines: Vec<String> = Vec::new();
    let mut files_matched: Vec<String> = Vec::new();
    let mut per_file_counts: Vec<(String, usize)> = Vec::new();
    while let Some(dir) = stack.pop() {
        let rd =
            std::fs::read_dir(&dir).map_err(|e| format!("read_dir({}): {e}", dir.display()))?;
        for entry in rd.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue;
                }
                stack.push(path);
                continue;
            }
            // Skip files that look binary (naive: any null byte in first 1024 bytes).
            let Ok(contents) = std::fs::read(&path) else {
                continue;
            };
            if contents.iter().take(1024).any(|b| *b == 0) {
                continue;
            }
            let Ok(text) = std::str::from_utf8(&contents) else {
                continue;
            };
            let rel_path = path.strip_prefix(worktree_root).unwrap_or(&path);
            let rel_str = rel_path.to_string_lossy().replace('\\', "/");
            let mut count = 0usize;
            for (idx, line) in text.lines().enumerate() {
                if line.contains(pattern) {
                    count += 1;
                    if mode == "content" {
                        content_lines.push(format!("{rel_str}:{}:{line}", idx + 1));
                    }
                }
            }
            if count > 0 {
                files_matched.push(rel_str.clone());
                per_file_counts.push((rel_str, count));
            }
        }
    }
    let output = match mode {
        "content" => content_lines.join("\n"),
        "files_with_matches" => {
            files_matched.sort();
            files_matched.dedup();
            files_matched.join("\n")
        }
        "count" => {
            per_file_counts.sort();
            per_file_counts
                .into_iter()
                .map(|(p, c)| format!("{p}:{c}"))
                .collect::<Vec<_>>()
                .join("\n")
        }
        other => {
            return Err(format!(
                "unknown mode `{other}` (expected content|files_with_matches|count)"
            ));
        }
    };
    Ok(output)
}
