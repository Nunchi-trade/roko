//! `glob` — find files matching a glob pattern.
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
pub const NAME: &str = "glob";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Find files in the worktree matching a glob pattern.";

/// Build the [`ToolDef`] for `glob`.
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
                "description": "Glob pattern to match (supports *, ?, **, [...]). Example: \"src/**/*.rs\""
            },
            "path": {
                "type": "string",
                "description": "Optional subdirectory to search within (default: worktree root)."
            }
        },
        "required": ["pattern"],
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Parallel)
    .with_idempotent(true)
    .with_timeout_ms(30_000)
}

/// Handler for `glob` (§36.18).
///
/// Walks the worktree recursively, matches each relative path against
/// the supplied glob `pattern` using a small shell-style matcher
/// (supports `*`, `?`, `**`, and character classes `[...]` at the
/// per-segment level; `**` matches any number of path components).
/// Returns matches one per line, sorted by modification-time descending
/// (newest first) with ties broken lexicographically.
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
        let root = ctx.worktree().to_path_buf();
        let matches = tokio::task::spawn_blocking(move || walk_and_match(&root, &pattern))
            .await
            .ok();
        let mut entries = match matches {
            Some(Ok(m)) => m,
            Some(Err(e)) => return ToolResult::Err(ToolError::Other(e)),
            None => return ToolResult::Err(ToolError::Other("glob: join failed".into())),
        };
        entries.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.rel.cmp(&b.rel)));
        let lines: Vec<String> = entries.into_iter().map(|e| e.rel).collect();
        ToolResult::text(lines.join("\n"))
    }
}

struct Match {
    rel: String,
    mtime: std::time::SystemTime,
}

fn walk_and_match(root: &std::path::Path, pattern: &str) -> Result<Vec<Match>, String> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd =
            std::fs::read_dir(&dir).map_err(|e| format!("read_dir({}): {e}", dir.display()))?;
        for entry in rd.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                // Skip hidden / vcs dirs.
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if glob_matches(pattern, &rel_str) {
                let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                out.push(Match {
                    rel: rel_str,
                    mtime,
                });
            }
        }
    }
    Ok(out)
}

/// Minimal shell-style glob matcher supporting `*`, `?`, `[...]`, `**`.
///
/// Segments (separated by `/`) are matched independently except for
/// `**`, which matches zero or more complete path segments.
fn glob_matches(pattern: &str, path: &str) -> bool {
    let pattern_parts: Vec<&str> = pattern.split('/').collect();
    let input_parts: Vec<&str> = path.split('/').collect();
    match_segments(&pattern_parts, &input_parts)
}

fn match_segments(pat: &[&str], path: &[&str]) -> bool {
    match (pat.first(), path.first()) {
        (None, None) => true,
        // `**` at the end matches any remaining segments (including none).
        (Some(&"**"), None) => match_segments(&pat[1..], path),
        (Some(&"**"), Some(_)) => {
            // Option 1: consume zero segments. Option 2: consume one and retry.
            match_segments(&pat[1..], path) || match_segments(pat, &path[1..])
        }
        (None, Some(_)) | (Some(_), None) => false,
        (Some(p), Some(t)) => {
            segment_match(p.as_bytes(), t.as_bytes()) && match_segments(&pat[1..], &path[1..])
        }
    }
}

fn segment_match(pat: &[u8], text: &[u8]) -> bool {
    let mut pi = 0;
    let mut ti = 0;
    let mut star: Option<(usize, usize)> = None;
    while ti < text.len() {
        if pi < pat.len() {
            if pat[pi] == b'*' {
                star = Some((pi + 1, ti));
                pi += 1;
                continue;
            }
            if pat[pi] == b'?' {
                pi += 1;
                ti += 1;
                continue;
            }
            if pat[pi] == b'[' {
                if let Some((end, matched)) = class_match(&pat[pi..], text[ti]) {
                    if matched {
                        pi += end;
                        ti += 1;
                        continue;
                    }
                }
            } else if pat[pi] == text[ti] {
                pi += 1;
                ti += 1;
                continue;
            }
        }
        if let Some((sp, st)) = star {
            pi = sp;
            ti = st + 1;
            star = Some((sp, ti));
            continue;
        }
        return false;
    }
    while pi < pat.len() && pat[pi] == b'*' {
        pi += 1;
    }
    pi == pat.len()
}

fn class_match(pat: &[u8], c: u8) -> Option<(usize, bool)> {
    let mut i = 1;
    let negate = pat.get(1) == Some(&b'!') || pat.get(1) == Some(&b'^');
    if negate {
        i += 1;
    }
    let mut matched = false;
    while i < pat.len() && pat[i] != b']' {
        if i + 2 < pat.len() && pat[i + 1] == b'-' && pat[i + 2] != b']' {
            if c >= pat[i] && c <= pat[i + 2] {
                matched = true;
            }
            i += 3;
        } else {
            if pat[i] == c {
                matched = true;
            }
            i += 1;
        }
    }
    if i >= pat.len() {
        return None;
    }
    Some((i + 1, matched ^ negate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_matches_within_segment() {
        assert!(glob_matches("*.rs", "main.rs"));
        assert!(glob_matches("*.rs", "lib.rs"));
        assert!(!glob_matches("*.rs", "src/main.rs")); // no cross-segment
        assert!(!glob_matches("*.rs", "main.txt"));
    }

    #[test]
    fn question_matches_single_char() {
        assert!(glob_matches("a?c", "abc"));
        assert!(!glob_matches("a?c", "ac"));
        assert!(!glob_matches("a?c", "abcd"));
    }

    #[test]
    fn doublestar_matches_cross_segment() {
        assert!(glob_matches("**/*.rs", "src/main.rs"));
        assert!(glob_matches("**/*.rs", "a/b/c/x.rs"));
        assert!(glob_matches("**/*.rs", "main.rs"));
        assert!(!glob_matches("**/*.rs", "main.txt"));
    }

    #[test]
    fn character_class_matches() {
        assert!(glob_matches("[abc]x", "ax"));
        assert!(glob_matches("[abc]x", "bx"));
        assert!(!glob_matches("[abc]x", "dx"));
    }

    #[test]
    fn character_class_negation() {
        assert!(glob_matches("[!abc]x", "dx"));
        assert!(!glob_matches("[!abc]x", "ax"));
    }

    #[test]
    fn character_class_range() {
        assert!(glob_matches("[a-z]", "m"));
        assert!(!glob_matches("[a-z]", "M"));
    }
}
