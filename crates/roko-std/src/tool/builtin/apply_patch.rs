//! `apply_patch` — apply a simple unified-diff patch to the worktree.
//!
//! Category: [`ToolCategory::Write`]. Permission: read + write.
//! Concurrency: [`ToolConcurrency::Serial`]. Idempotent: no.
//!
//! # Scope
//!
//! Day-one implementation supports **single-file patches** with one or
//! more hunks. Each hunk's context lines must match the current file
//! exactly (no fuzz). Multi-file patches are rejected with a clear error
//! pointing at the first unexpected file header.

use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};

use super::sandbox::{require_string, require_within_worktree};

/// Canonical `snake_case` name.
pub const NAME: &str = "apply_patch";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Apply a unified-diff patch to a single file in the worktree.";

/// Build the [`ToolDef`] for `apply_patch`.
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
    .with_timeout_ms(60_000)
}

/// Handler for `apply_patch` (§36.28).
#[derive(Debug, Clone, Copy, Default)]
pub struct Handler;

#[async_trait]
impl ToolHandler for Handler {
    fn name(&self) -> &str {
        NAME
    }

    async fn execute(&self, call: ToolCall, ctx: &ToolContext) -> ToolResult {
        let patch = match require_string(&call.arguments, "patch") {
            Ok(s) => s,
            Err(e) => return ToolResult::Err(e),
        };
        let parsed = match parse_single_file_patch(&patch) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(ToolError::Other(format!("apply_patch: {e}"))),
        };
        let absolute = match require_within_worktree(ctx.worktree(), &parsed.path) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        let original = match tokio::fs::read_to_string(&absolute).await {
            Ok(s) => s,
            Err(e) => {
                return ToolResult::Err(ToolError::Other(format!(
                    "apply_patch: failed to read {}: {e}",
                    absolute.display()
                )));
            }
        };
        let patched = match apply_hunks(&original, &parsed.hunks) {
            Ok(s) => s,
            Err(e) => return ToolResult::Err(ToolError::Other(format!("apply_patch: {e}"))),
        };
        match tokio::fs::write(&absolute, patched.as_bytes()).await {
            Ok(()) => ToolResult::text(format!(
                "patched {} ({} hunk(s))",
                absolute.display(),
                parsed.hunks.len()
            )),
            Err(e) => ToolResult::Err(ToolError::Other(format!(
                "apply_patch: write failed ({}): {e}",
                absolute.display()
            ))),
        }
    }
}

#[derive(Debug)]
struct ParsedPatch {
    path: String,
    hunks: Vec<Hunk>,
}

#[derive(Debug)]
struct Hunk {
    old_start: usize,
    lines: Vec<HunkLine>,
}

#[derive(Debug)]
enum HunkLine {
    Context(String),
    Removed(String),
    Added(String),
}

fn parse_single_file_patch(diff: &str) -> Result<ParsedPatch, String> {
    let mut path: Option<String> = None;
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut lines = diff.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            let p = rest.trim_start_matches("b/").trim();
            if let Some(ref existing) = path {
                if existing != p {
                    return Err(format!(
                        "multi-file patch not supported (saw {existing} and {p})"
                    ));
                }
            } else {
                path = Some(p.to_string());
            }
            continue;
        }
        if line.starts_with("--- ") || line.starts_with("diff ") || line.starts_with("index ") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("@@ ") {
            let header = rest.trim_start_matches('-');
            let old_start = header
                .split([',', ' ', '@'])
                .next()
                .and_then(|n| n.parse::<usize>().ok())
                .ok_or_else(|| format!("malformed hunk header: {line}"))?;
            let mut hunk = Hunk {
                old_start,
                lines: Vec::new(),
            };
            while let Some(peek) = lines.peek() {
                if peek.starts_with("@@ ") || peek.starts_with("--- ") || peek.starts_with("+++ ") {
                    break;
                }
                let Some(next) = lines.next() else { break };
                let hline = if let Some(rest) = next.strip_prefix('+') {
                    HunkLine::Added(rest.to_string())
                } else if let Some(rest) = next.strip_prefix('-') {
                    HunkLine::Removed(rest.to_string())
                } else if let Some(rest) = next.strip_prefix(' ') {
                    HunkLine::Context(rest.to_string())
                } else if next.is_empty() {
                    HunkLine::Context(String::new())
                } else {
                    // Unrecognized line inside a hunk — bail out.
                    return Err(format!("unexpected line in hunk: {next:?}"));
                };
                hunk.lines.push(hline);
            }
            hunks.push(hunk);
        }
    }
    let path = path.ok_or_else(|| "no `+++ ` header found".to_string())?;
    if hunks.is_empty() {
        return Err("no hunks found".into());
    }
    Ok(ParsedPatch { path, hunks })
}

fn apply_hunks(original: &str, hunks: &[Hunk]) -> Result<String, String> {
    let src_lines: Vec<&str> = original.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut src_idx: usize = 0; // 0-based
    for (h_idx, hunk) in hunks.iter().enumerate() {
        // old_start is 1-based; move src_idx forward to that line,
        // copying verbatim.
        let target_idx = hunk.old_start.saturating_sub(1);
        if target_idx < src_idx {
            return Err(format!("hunk {h_idx} overlaps with a prior hunk"));
        }
        while src_idx < target_idx {
            if src_idx >= src_lines.len() {
                return Err(format!("hunk {h_idx} starts past end of file"));
            }
            out.push(src_lines[src_idx].to_string());
            src_idx += 1;
        }
        for line in &hunk.lines {
            match line {
                HunkLine::Context(text) | HunkLine::Removed(text) => {
                    let actual = src_lines.get(src_idx).copied().unwrap_or("");
                    if actual != text {
                        return Err(format!(
                            "hunk {h_idx}: context mismatch at line {} (expected {:?}, got {:?})",
                            src_idx + 1,
                            text,
                            actual
                        ));
                    }
                    if matches!(line, HunkLine::Context(_)) {
                        out.push(text.clone());
                    }
                    src_idx += 1;
                }
                HunkLine::Added(text) => {
                    out.push(text.clone());
                }
            }
        }
    }
    // Tail of unchanged source.
    while src_idx < src_lines.len() {
        out.push(src_lines[src_idx].to_string());
        src_idx += 1;
    }
    let trailing_newline = original.ends_with('\n');
    let mut result = out.join("\n");
    if trailing_newline {
        result.push('\n');
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_file_with_one_hunk() {
        let patch = "--- a/x.txt\n+++ b/x.txt\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n";
        let p = parse_single_file_patch(patch).expect("parse");
        assert_eq!(p.path, "x.txt");
        assert_eq!(p.hunks.len(), 1);
        assert_eq!(p.hunks[0].old_start, 1);
    }

    #[test]
    fn parse_rejects_multi_file_patch() {
        let patch = "+++ b/x.txt\n@@ -1 +1 @@\n-a\n+b\n+++ b/y.txt\n";
        let err = parse_single_file_patch(patch).expect_err("multi-file");
        assert!(err.contains("multi-file"));
    }

    #[test]
    fn parse_rejects_missing_header() {
        let patch = "@@ -1 +1 @@\n-a\n+b\n";
        assert!(parse_single_file_patch(patch).is_err());
    }

    #[test]
    fn apply_single_hunk_replacement() {
        let original = "a\nb\nc\n";
        let patch = "--- a/x.txt\n+++ b/x.txt\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n";
        let p = parse_single_file_patch(patch).expect("parse");
        let applied = apply_hunks(original, &p.hunks).expect("apply");
        assert_eq!(applied, "a\nB\nc\n");
    }

    #[test]
    fn apply_rejects_context_mismatch() {
        let original = "a\nb\nc\n";
        let patch = "--- a/x.txt\n+++ b/x.txt\n@@ -1,3 +1,3 @@\n a\n-WRONG\n+B\n c\n";
        let p = parse_single_file_patch(patch).expect("parse");
        let err = apply_hunks(original, &p.hunks).expect_err("mismatch");
        assert!(err.contains("context mismatch"));
    }

    #[test]
    fn apply_addition_at_end() {
        let original = "a\nb\n";
        let patch = "--- a/x.txt\n+++ b/x.txt\n@@ -1,2 +1,3 @@\n a\n b\n+c\n";
        let p = parse_single_file_patch(patch).expect("parse");
        let applied = apply_hunks(original, &p.hunks).expect("apply");
        assert_eq!(applied, "a\nb\nc\n");
    }

    #[test]
    fn apply_preserves_no_trailing_newline() {
        let original = "a\nb";
        let patch = "--- a/x.txt\n+++ b/x.txt\n@@ -1,2 +1,2 @@\n a\n-b\n+B";
        let p = parse_single_file_patch(patch).expect("parse");
        let applied = apply_hunks(original, &p.hunks).expect("apply");
        assert_eq!(applied, "a\nB");
    }
}
