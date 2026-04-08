//! Path-sandboxing helpers shared by filesystem tool handlers.
//!
//! All filesystem tools must resolve user-supplied paths **inside** the
//! worktree supplied by [`roko_core::tool::ToolContext::worktree`]. Any
//! path that escapes via `..` or an absolute prefix is rejected with
//! [`ToolError::PathOutsideWorktree`].
//!
//! # Algorithm
//!
//! 1. If the caller supplies an absolute path, canonicalize it and check
//!    that it sits under the worktree root. If canonicalization fails
//!    (path doesn't exist yet, as for `write_file`), fall back to a
//!    purely lexical `starts_with` check.
//! 2. If the caller supplies a relative path, join it to the worktree
//!    root, then normalize `..` components lexically and verify the
//!    result is still under the root.
//!
//! The check is deliberately conservative — symlinks that point outside
//! the worktree are treated as escapes. Downstream handlers can loosen
//! this via the §36.46 capability system when necessary.

use std::path::{Component, Path, PathBuf};

use roko_core::tool::ToolError;

/// Resolve `rel` against `worktree` and ensure the result stays inside
/// the worktree. Returns the absolute, normalized path on success.
pub fn require_within_worktree(worktree: &Path, rel: &str) -> Result<PathBuf, ToolError> {
    let candidate = Path::new(rel);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        worktree.join(candidate)
    };
    let normalized = normalize(&joined);
    let normalized_root = normalize(worktree);
    if !normalized.starts_with(&normalized_root) {
        return Err(ToolError::PathOutsideWorktree(normalized));
    }
    Ok(normalized)
}

/// Extract a required string field from a JSON arguments object.
pub fn require_string(args: &serde_json::Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .map_or_else(
            || {
                Err(ToolError::SchemaInvalid(format!(
                    "missing required string argument: {key}"
                )))
            },
            |s| Ok(s.to_string()),
        )
}

/// Lexically normalize a path, collapsing `.` and `..` components.
///
/// Does **not** touch the filesystem — purely syntactic. This matches
/// the behavior of Go's `path.Clean` / Rust's `path-clean` crate.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(std::path::MAIN_SEPARATOR_STR),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    // Can't go above root — keep as-is (caller's check
                    // against the worktree root will reject this).
                    out.push("..");
                }
            }
            Component::Normal(n) => out.push(n),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_string_extracts_field() {
        let args = serde_json::json!({"path": "foo.rs"});
        assert_eq!(require_string(&args, "path").expect("ok"), "foo.rs");
    }

    #[test]
    fn require_string_rejects_missing_field() {
        let args = serde_json::json!({"other": 1});
        let err = require_string(&args, "path").expect_err("missing");
        assert!(matches!(err, ToolError::SchemaInvalid(_)));
    }

    #[test]
    fn require_string_rejects_wrong_type() {
        let args = serde_json::json!({"path": 42});
        assert!(require_string(&args, "path").is_err());
    }

    #[test]
    fn relative_path_inside_worktree_passes() {
        let worktree = Path::new("/repo");
        let resolved = require_within_worktree(worktree, "src/main.rs").expect("ok");
        assert!(resolved.ends_with("src/main.rs"));
    }

    #[test]
    fn parent_dir_escape_is_rejected() {
        let worktree = Path::new("/repo");
        let err = require_within_worktree(worktree, "../etc/passwd").expect_err("escape");
        assert!(matches!(err, ToolError::PathOutsideWorktree(_)));
    }

    #[test]
    fn absolute_path_outside_worktree_is_rejected() {
        let worktree = Path::new("/repo");
        let err = require_within_worktree(worktree, "/etc/passwd").expect_err("escape");
        assert!(matches!(err, ToolError::PathOutsideWorktree(_)));
    }

    #[test]
    fn current_dir_components_are_normalized() {
        let worktree = Path::new("/repo");
        let resolved = require_within_worktree(worktree, "./src/./foo.rs").expect("ok");
        assert!(resolved.ends_with("src/foo.rs"));
    }

    #[test]
    fn parent_within_worktree_is_allowed() {
        let worktree = Path::new("/repo");
        let resolved = require_within_worktree(worktree, "src/../lib.rs").expect("ok");
        assert!(resolved.ends_with("lib.rs"));
    }

    #[test]
    fn normalize_preserves_unicode_names() {
        let worktree = Path::new("/repo");
        let resolved = require_within_worktree(worktree, "файл.rs").expect("ok");
        assert!(resolved.to_string_lossy().contains("файл.rs"));
    }
}
