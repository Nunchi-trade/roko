//! Persistent storage for error patterns discovered during plan execution.
//!
//! Agents accumulate compiler and test errors across tasks. This module
//! normalizes each error into a stable digest, upserts it into an
//! append-friendly store, and exposes the most frequent patterns so that
//! agent prompts can include "known pitfalls" context — enabling agents to
//! learn from each other's failures.
//!
//! The store is a single JSON file. Writes use atomic tmp-rename to avoid
//! corruption on crash.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use serde::{Deserialize, Serialize};

/// A single normalized error pattern with occurrence tracking.
///
/// Patterns are keyed by [`ErrorPattern::digest`] — a normalized first-line
/// error signature stripped of file paths, line numbers, and ANSI codes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorPattern {
    /// Normalized error signature (first line of error, stripped of file
    /// paths and line numbers).
    pub digest: String,
    /// Error category (e.g. `"unresolved_import"`, `"type_mismatch"`,
    /// `"lifetime"`).
    pub category: String,
    /// How many times this pattern has been seen.
    pub occurrences: u32,
    /// ISO 8601 timestamp of the first occurrence.
    pub first_seen_at: String,
    /// ISO 8601 timestamp of the most recent occurrence.
    pub last_seen_at: String,
    /// Plan IDs that have hit this error.
    pub plan_ids: Vec<String>,
    /// What fixed the error (filled in from reflection or manual annotation).
    pub resolution: Option<String>,
    /// Auto-fix hint extracted from rustc output.
    pub suggestion: Option<String>,
}

/// Persistent store of [`ErrorPattern`] records backed by a JSON file.
///
/// The store de-duplicates patterns by digest: calling [`append`](Self::append)
/// with the same digest increments the occurrence counter and merges
/// metadata rather than creating a new entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPatternStore {
    patterns: Vec<ErrorPattern>,
}

impl ErrorPatternStore {
    /// Load patterns from a JSON file at `path`.
    ///
    /// Returns an empty store if the file does not exist or cannot be
    /// parsed (e.g. after a crash that left a partial write).
    pub fn load(path: &Path) -> Self {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => return Self::empty(),
        };
        serde_json::from_slice::<Self>(&bytes).unwrap_or_else(|_| Self::empty())
    }

    /// Persist the store to `path` as pretty-printed JSON.
    ///
    /// Uses atomic write (tmp file + rename) so readers never see a
    /// partially-written file.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`std::io::Error`] if the parent directory
    /// cannot be created, serialization fails, or the filesystem write fails.
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = unique_tmp_path(path);
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Upsert an error pattern by digest.
    ///
    /// If a pattern with the same `digest` already exists, its occurrence
    /// counter is incremented, `last_seen_at` is updated, and `plan_id` and
    /// `suggestion` are merged. Otherwise a new pattern is created.
    pub fn append(
        &mut self,
        digest: &str,
        category: &str,
        plan_id: &str,
        suggestion: Option<&str>,
    ) {
        let now = Utc::now().to_rfc3339();

        if let Some(existing) = self.patterns.iter_mut().find(|p| p.digest == digest) {
            existing.occurrences = existing.occurrences.saturating_add(1);
            existing.last_seen_at = now;
            if !existing.plan_ids.contains(&plan_id.to_string()) {
                existing.plan_ids.push(plan_id.to_string());
            }
            if existing.suggestion.is_none() {
                if let Some(s) = suggestion {
                    existing.suggestion = Some(s.to_string());
                }
            }
            return;
        }

        self.patterns.push(ErrorPattern {
            digest: digest.to_string(),
            category: category.to_string(),
            occurrences: 1,
            first_seen_at: now.clone(),
            last_seen_at: now,
            plan_ids: vec![plan_id.to_string()],
            resolution: None,
            suggestion: suggestion.map(|s| s.to_string()),
        });
    }

    /// Return the most frequent patterns, sorted by descending occurrence
    /// count.
    pub fn top_patterns(&self, limit: usize) -> Vec<&ErrorPattern> {
        let mut sorted: Vec<&ErrorPattern> = self.patterns.iter().collect();
        sorted.sort_by_key(|p| std::cmp::Reverse(p.occurrences));
        sorted.truncate(limit);
        sorted
    }

    /// Return all patterns matching the given `category`.
    pub fn patterns_for_category(&self, category: &str) -> Vec<&ErrorPattern> {
        self.patterns
            .iter()
            .filter(|p| p.category == category)
            .collect()
    }

    /// Format the top patterns as a markdown-ish block suitable for
    /// injection into an agent system prompt.
    ///
    /// Each entry shows the digest, category, occurrence count, and any
    /// known resolution or suggestion. Output is capped at `limit` entries.
    pub fn format_for_prompt(&self, limit: usize) -> String {
        let top = self.top_patterns(limit);
        if top.is_empty() {
            return String::new();
        }

        let mut out = String::from("Known error patterns (learn from prior failures):\n");
        for (i, pattern) in top.iter().enumerate() {
            let _ = writeln!(
                out,
                "{}. [{}] {} (seen {} time{})",
                i + 1,
                pattern.category,
                pattern.digest,
                pattern.occurrences,
                if pattern.occurrences == 1 { "" } else { "s" },
            );
            if let Some(resolution) = &pattern.resolution {
                let _ = writeln!(out, "   Fix: {resolution}");
            }
            if let Some(suggestion) = &pattern.suggestion {
                let _ = writeln!(out, "   Hint: {suggestion}");
            }
        }
        out
    }

    /// Return the number of distinct patterns in the store.
    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    /// Return `true` if the store contains no patterns.
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    fn empty() -> Self {
        Self {
            patterns: Vec::new(),
        }
    }
}

/// Normalize raw error text into a stable digest.
///
/// The normalization pipeline:
/// 1. Takes the first non-empty line of `raw`.
/// 2. Strips ANSI escape codes.
/// 3. Replaces file-path-with-line-number tokens (e.g.
///    `/path/to/file.rs:42:10`) with `<file>`.
/// 4. Collapses runs of whitespace into single spaces.
/// 5. Truncates to 200 characters.
pub fn normalize_error_digest(raw: &str) -> String {
    let first_line = raw
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();

    let stripped = strip_ansi(first_line);
    let no_paths = replace_file_paths(&stripped);
    let collapsed = collapse_whitespace(&no_paths);

    truncate_chars(&collapsed, 200)
}

/// Strip ANSI escape sequences (CSI and OSC) from `text`.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // CSI sequence: ESC [ ... final byte
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                while let Some(&next) = chars.peek() {
                    chars.next();
                    // CSI terminates at an ASCII letter or '~'.
                    if next.is_ascii_alphabetic() || next == '~' {
                        break;
                    }
                }
            } else {
                // OSC or other: consume until BEL or ST.
                while let Some(&next) = chars.peek() {
                    if next == '\x07' {
                        chars.next();
                        break;
                    }
                    if next == '\x1b' {
                        break;
                    }
                    chars.next();
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Replace file-path tokens like `/foo/bar.rs:42:10` or `src/lib.rs:7`
/// with `<file>`.
fn replace_file_paths(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for token in text.split_whitespace() {
        if !result.is_empty() {
            result.push(' ');
        }
        if is_file_path_token(token) {
            result.push_str("<file>");
        } else {
            result.push_str(token);
        }
    }
    result
}

/// Check whether `token` looks like a file path with a line number,
/// e.g. `crates/roko-learn/src/lib.rs:42:10` or `-->
/// src/main.rs:7:1`.
fn is_file_path_token(token: &str) -> bool {
    let cleaned = token
        .trim_start_matches("-->")
        .trim_start()
        .trim_end_matches([',', ';', ':']);

    // Must contain a `.rs:` or `.ts:` or `.go:` etc followed by digits.
    for ext in [".rs:", ".ts:", ".go:", ".py:", ".js:", ".toml:", ".json:"] {
        if let Some((_prefix, tail)) = cleaned.rsplit_once(ext) {
            let first_part = tail.split(':').next().unwrap_or("");
            if !first_part.is_empty() && first_part.chars().all(|ch| ch.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Generate a unique temporary file path next to `path` for atomic writes.
fn unique_tmp_path(path: &Path) -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("error-patterns.json");
    parent.join(format!(".{stem}.tmp-{stamp}-{seq}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn append_upserts_by_digest() {
        let mut store = ErrorPatternStore::empty();
        store.append(
            "error[E0433]: unresolved import",
            "unresolved_import",
            "plan-1",
            None,
        );
        store.append(
            "error[E0433]: unresolved import",
            "unresolved_import",
            "plan-2",
            Some("did you mean `std::io`?"),
        );
        store.append(
            "error[E0433]: unresolved import",
            "unresolved_import",
            "plan-2",
            None,
        );

        assert_eq!(store.len(), 1);
        let pattern = &store.patterns[0];
        assert_eq!(pattern.occurrences, 3);
        assert_eq!(pattern.plan_ids, vec!["plan-1", "plan-2"]);
        assert_eq!(
            pattern.suggestion.as_deref(),
            Some("did you mean `std::io`?")
        );
    }

    #[test]
    fn top_patterns_sorts_by_occurrence() {
        let mut store = ErrorPatternStore::empty();
        store.append("rare error", "misc", "p1", None);

        store.append("common error", "misc", "p1", None);
        store.append("common error", "misc", "p2", None);
        store.append("common error", "misc", "p3", None);

        store.append("medium error", "misc", "p1", None);
        store.append("medium error", "misc", "p2", None);

        let top = store.top_patterns(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].digest, "common error");
        assert_eq!(top[0].occurrences, 3);
        assert_eq!(top[1].digest, "medium error");
        assert_eq!(top[1].occurrences, 2);
    }

    #[test]
    fn normalize_strips_paths() {
        let raw = "error[E0433]: failed to resolve: use of undeclared crate or module `foo` --> crates/roko-learn/src/lib.rs:42:10";
        let digest = normalize_error_digest(raw);
        assert!(
            !digest.contains("crates/roko-learn/src/lib.rs:42:10"),
            "file path should be replaced, got: {digest}"
        );
        assert!(
            digest.contains("<file>"),
            "should contain <file> placeholder, got: {digest}"
        );
        assert!(
            digest.contains("error[E0433]"),
            "error code should be preserved, got: {digest}"
        );
    }

    #[test]
    fn normalize_strips_ansi_codes() {
        let raw = "\x1b[1m\x1b[38;5;9merror[E0308]\x1b[0m: mismatched types";
        let digest = normalize_error_digest(raw);
        assert_eq!(digest, "error[E0308]: mismatched types");
    }

    #[test]
    fn normalize_collapses_whitespace_and_truncates() {
        let raw = format!("error:   too   many   spaces   {}", "x".repeat(300));
        let digest = normalize_error_digest(&raw);
        assert!(!digest.contains("  "), "should collapse whitespace");
        assert!(
            digest.chars().count() <= 200,
            "should truncate to 200 chars, got {}",
            digest.chars().count()
        );
    }

    #[test]
    fn save_load_roundtrip() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("error-patterns.json");

        let mut store = ErrorPatternStore::empty();
        store.append("digest-a", "type_mismatch", "plan-1", Some("try Into"));
        store.append("digest-b", "lifetime", "plan-2", None);

        store.save(&path).expect("save");
        let loaded = ErrorPatternStore::load(&path);

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.patterns[0].digest, "digest-a");
        assert_eq!(loaded.patterns[0].category, "type_mismatch");
        assert_eq!(loaded.patterns[0].suggestion.as_deref(), Some("try Into"));
        assert_eq!(loaded.patterns[1].digest, "digest-b");
        assert_eq!(loaded.patterns[1].category, "lifetime");
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("nonexistent.json");
        let store = ErrorPatternStore::load(&path);
        assert!(store.is_empty());
    }

    #[test]
    fn format_for_prompt_limits_output() {
        let mut store = ErrorPatternStore::empty();
        for i in 0..10 {
            for _ in 0..(10 - i) {
                store.append(&format!("error-{i}"), "misc", &format!("plan-{i}"), None);
            }
        }

        let formatted = store.format_for_prompt(3);
        let lines: Vec<&str> = formatted.lines().collect();

        // Header + 3 entries = 4 lines minimum.
        assert!(
            lines.len() >= 4,
            "expected at least 4 lines, got {}",
            lines.len()
        );
        assert!(formatted.contains("error-0"), "most frequent should appear");
        assert!(formatted.contains("error-1"));
        assert!(formatted.contains("error-2"));
        assert!(
            !formatted.contains("error-9"),
            "least frequent should be excluded"
        );
    }

    #[test]
    fn format_for_prompt_empty_store() {
        let store = ErrorPatternStore::empty();
        let formatted = store.format_for_prompt(5);
        assert!(formatted.is_empty());
    }

    #[test]
    fn format_for_prompt_includes_resolution_and_suggestion() {
        let mut store = ErrorPatternStore::empty();
        store.append("digest-r", "type_mismatch", "p1", Some("use `.into()`"));
        store.patterns[0].resolution = Some("Added explicit type annotation".to_string());

        let formatted = store.format_for_prompt(5);
        assert!(
            formatted.contains("Added explicit type annotation"),
            "resolution should appear"
        );
        assert!(
            formatted.contains("use `.into()`"),
            "suggestion should appear"
        );
    }

    #[test]
    fn patterns_for_category_filters() {
        let mut store = ErrorPatternStore::empty();
        store.append("error A", "type_mismatch", "p1", None);
        store.append("error B", "lifetime", "p1", None);
        store.append("error C", "type_mismatch", "p2", None);
        store.append("error D", "unresolved_import", "p3", None);

        let type_errors = store.patterns_for_category("type_mismatch");
        assert_eq!(type_errors.len(), 2);
        assert!(type_errors.iter().all(|p| p.category == "type_mismatch"));

        let lifetime_errors = store.patterns_for_category("lifetime");
        assert_eq!(lifetime_errors.len(), 1);
        assert_eq!(lifetime_errors[0].digest, "error B");

        let empty = store.patterns_for_category("nonexistent");
        assert!(empty.is_empty());
    }

    #[test]
    fn append_preserves_first_seen_timestamp() {
        let mut store = ErrorPatternStore::empty();
        store.append("same-digest", "misc", "p1", None);
        let first_seen = store.patterns[0].first_seen_at.clone();

        // Simulate a later occurrence.
        store.append("same-digest", "misc", "p2", None);
        assert_eq!(
            store.patterns[0].first_seen_at, first_seen,
            "first_seen_at must not change on upsert"
        );
        assert_ne!(
            store.patterns[0].last_seen_at, first_seen,
            "last_seen_at should be updated (unless test runs in < 1ms)"
        );
    }

    #[test]
    fn append_does_not_overwrite_existing_suggestion() {
        let mut store = ErrorPatternStore::empty();
        store.append("d", "misc", "p1", Some("original hint"));
        store.append("d", "misc", "p2", Some("new hint"));

        assert_eq!(
            store.patterns[0].suggestion.as_deref(),
            Some("original hint"),
            "first suggestion wins"
        );
    }
}
