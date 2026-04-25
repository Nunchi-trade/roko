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
/// Patterns are keyed by [`ErrorPattern::key`]. Older digest-only rows are
/// repaired on load by using the digest as the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorPattern {
    /// Stable key used for de-duplication. New gate failure observations use
    /// normalized gate/parser keys such as `E0425::src/lib.rs`.
    #[serde(default)]
    pub key: String,
    /// Normalized error signature (first line of error, stripped of file
    /// paths and line numbers).
    pub digest: String,
    /// Gate that emitted the pattern, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
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
    /// Task IDs that have hit this error.
    #[serde(default)]
    pub task_ids: Vec<String>,
    /// Whether this pattern has been resolved.
    #[serde(default)]
    pub resolved: bool,
    /// What fixed the error (filled in from reflection or manual annotation).
    pub resolution: Option<String>,
    /// Auto-fix hint extracted from rustc output.
    pub suggestion: Option<String>,
}

/// A structured gate failure observation emitted by gates, review parsing, or
/// retry classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateFailureObservation {
    /// Stable key used to merge repeated observations.
    pub key: String,
    /// Plan that observed the failure.
    pub plan_id: String,
    /// Task that observed the failure, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Gate or parser source that observed the failure.
    pub gate: String,
    /// Coarse failure class.
    pub classification: String,
    /// Compact, bounded signature. Raw logs should not be stored here.
    pub digest: String,
    /// Optional suggested fix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// Source subsystem that produced the observation.
    pub source: GateFailureSource,
    /// ISO 8601 timestamp for the observation.
    pub observed_at: String,
}

impl GateFailureObservation {
    /// Build an observation and stamp it with the current time.
    #[must_use]
    pub fn new(
        key: impl Into<String>,
        plan_id: impl Into<String>,
        task_id: Option<String>,
        gate: impl Into<String>,
        classification: impl Into<String>,
        digest: impl Into<String>,
        source: GateFailureSource,
    ) -> Self {
        Self {
            key: key.into(),
            plan_id: plan_id.into(),
            task_id,
            gate: gate.into(),
            classification: classification.into(),
            digest: truncate_chars(&digest.into(), 200),
            suggestion: None,
            source,
            observed_at: Utc::now().to_rfc3339(),
        }
    }

    /// Attach an optional suggestion to this observation.
    #[must_use]
    pub fn with_suggestion(mut self, suggestion: Option<String>) -> Self {
        self.suggestion = suggestion;
        self
    }
}

/// Subsystem that produced a gate failure observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateFailureSource {
    /// Compile/test/lint gate classification.
    GateClassification,
    /// Structured review verdict parsing.
    ReviewVerdict,
    /// Agent dispatch/retry error classification.
    RetryClassifier,
}

/// Result of upserting a failure observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailurePatternUpdate {
    /// Whether this observation created a new pattern.
    pub inserted: bool,
    /// Occurrence count after the update.
    pub occurrences: u32,
}

/// Query used to select relevant failure patterns for retry context.
#[derive(Debug, Clone, Copy, Default)]
pub struct FailurePatternQuery<'a> {
    /// Plan to prefer.
    pub plan_id: Option<&'a str>,
    /// Task to prefer.
    pub task_id: Option<&'a str>,
    /// Gate to prefer.
    pub gate: Option<&'a str>,
    /// Failure class to prefer.
    pub classification: Option<&'a str>,
}

/// A bounded prompt/context summary for failure memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailurePatternSummary {
    /// Selected patterns in display order.
    pub patterns: Vec<FailurePatternSummaryItem>,
    /// Number of candidate patterns considered before bounding.
    pub total_candidates: usize,
}

impl FailurePatternSummary {
    /// Render the summary as retry-context text.
    #[must_use]
    pub fn format_for_prompt(&self) -> String {
        if self.patterns.is_empty() {
            return String::new();
        }

        let mut out = String::from("## Prior Gate Failure Patterns\n");
        out.push_str(
            "Use these concise prior failures as constraints; do not treat them as full logs.\n",
        );
        for (index, pattern) in self.patterns.iter().enumerate() {
            let repeated = if pattern.repeated {
                "repeated"
            } else {
                "one-off"
            };
            let _ = writeln!(
                out,
                "{}. [{}] {} (seen {} time{}, {repeated})",
                index + 1,
                pattern.classification,
                pattern.digest,
                pattern.occurrences,
                if pattern.occurrences == 1 { "" } else { "s" },
            );
            if let Some(gate) = &pattern.gate {
                let _ = writeln!(out, "   Gate: {gate}");
            }
            if let Some(resolution) = &pattern.resolution {
                let _ = writeln!(out, "   Fix: {resolution}");
            }
            if let Some(suggestion) = &pattern.suggestion {
                let _ = writeln!(out, "   Hint: {suggestion}");
            }
        }
        out
    }
}

/// One selected failure pattern for prompt/context use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailurePatternSummaryItem {
    /// Stable key for the pattern.
    pub key: String,
    /// Gate that emitted the pattern, if known.
    pub gate: Option<String>,
    /// Coarse failure class.
    pub classification: String,
    /// Compact signature.
    pub digest: String,
    /// Occurrence count.
    pub occurrences: u32,
    /// Whether this is a repeated pattern rather than a one-off failure.
    pub repeated: bool,
    /// Known resolution from reflection or manual annotation.
    pub resolution: Option<String>,
    /// Optional suggested fix.
    pub suggestion: Option<String>,
}

/// Persistent store of [`ErrorPattern`] records backed by a JSON file.
///
/// The store de-duplicates patterns by key: structured observations use
/// normalized keys, while the legacy [`append`](Self::append) path uses the
/// digest as the key.
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
        serde_json::from_slice::<Self>(&bytes)
            .map(|mut store| {
                store.repair_loaded_patterns();
                store
            })
            .unwrap_or_else(|_| Self::empty())
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
        let observation = GateFailureObservation::new(
            digest,
            plan_id,
            None,
            "unknown",
            category,
            digest,
            GateFailureSource::RetryClassifier,
        )
        .with_suggestion(suggestion.map(str::to_string));
        let _ = self.observe_gate_failure(observation);
    }

    /// Upsert a structured gate failure observation.
    ///
    /// Repeated observations with the same key increment evidence on the
    /// existing pattern instead of producing duplicate prompt noise.
    pub fn observe_gate_failure(
        &mut self,
        observation: GateFailureObservation,
    ) -> FailurePatternUpdate {
        let now = Utc::now().to_rfc3339();
        let key = observation.key.trim().to_string();
        if key.is_empty() {
            return FailurePatternUpdate {
                inserted: false,
                occurrences: 0,
            };
        }

        if let Some(existing) = self.patterns.iter_mut().find(|p| p.key == key) {
            existing.occurrences = existing.occurrences.saturating_add(1);
            existing.last_seen_at = now;
            push_unique(&mut existing.plan_ids, observation.plan_id);
            if let Some(task_id) = observation.task_id {
                push_unique(&mut existing.task_ids, task_id);
            }
            if existing.gate.is_none() && !observation.gate.trim().is_empty() {
                existing.gate = Some(observation.gate);
            }
            if !observation.digest.trim().is_empty() {
                existing.digest = truncate_chars(&observation.digest, 200);
            }
            if existing.suggestion.is_none() {
                existing.suggestion = observation.suggestion;
            }
            return FailurePatternUpdate {
                inserted: false,
                occurrences: existing.occurrences,
            };
        }

        self.patterns.push(ErrorPattern {
            key,
            digest: truncate_chars(&observation.digest, 200),
            gate: (!observation.gate.trim().is_empty()).then_some(observation.gate),
            category: observation.classification,
            occurrences: 1,
            first_seen_at: now.clone(),
            last_seen_at: now,
            plan_ids: vec![observation.plan_id],
            task_ids: observation.task_id.into_iter().collect(),
            resolved: false,
            resolution: None,
            suggestion: observation.suggestion,
        });
        FailurePatternUpdate {
            inserted: true,
            occurrences: 1,
        }
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

    /// Return a bounded, relevance-ranked summary for retry prompt context.
    #[must_use]
    pub fn bounded_summary(
        &self,
        query: FailurePatternQuery<'_>,
        limit: usize,
        max_chars: usize,
    ) -> FailurePatternSummary {
        let mut candidates: Vec<(usize, &ErrorPattern)> = self
            .patterns
            .iter()
            .filter(|pattern| !pattern.resolved)
            .map(|pattern| (pattern.relevance_score(query), pattern))
            .filter(|(score, _)| *score > 0 || query.is_empty())
            .collect();
        candidates.sort_by(|(score_a, a), (score_b, b)| {
            score_b
                .cmp(score_a)
                .then_with(|| b.occurrences.cmp(&a.occurrences))
                .then_with(|| b.last_seen_at.cmp(&a.last_seen_at))
        });

        let total_candidates = candidates.len();
        let mut used_chars = 0usize;
        let mut patterns = Vec::new();
        for (_, pattern) in candidates.into_iter().take(limit) {
            let item = FailurePatternSummaryItem {
                key: pattern.key.clone(),
                gate: pattern.gate.clone(),
                classification: pattern.category.clone(),
                digest: truncate_chars(&pattern.digest, 200),
                occurrences: pattern.occurrences,
                repeated: pattern.occurrences > 1,
                resolution: pattern.resolution.clone(),
                suggestion: pattern.suggestion.clone(),
            };
            let projected = item.digest.chars().count()
                + item.resolution.as_ref().map_or(0, |s| s.chars().count())
                + item.suggestion.as_ref().map_or(0, |s| s.chars().count())
                + item.gate.as_ref().map_or(0, |s| s.chars().count())
                + 80;
            if !patterns.is_empty() && used_chars.saturating_add(projected) > max_chars {
                break;
            }
            used_chars = used_chars.saturating_add(projected);
            patterns.push(item);
        }

        FailurePatternSummary {
            patterns,
            total_candidates,
        }
    }

    /// Format the top patterns as a markdown-ish block suitable for
    /// injection into an agent system prompt.
    ///
    /// Each entry shows the digest, category, occurrence count, and any
    /// known resolution or suggestion. Output is capped at `limit` entries.
    pub fn format_for_prompt(&self, limit: usize) -> String {
        self.bounded_summary(FailurePatternQuery::default(), limit, 2_000)
            .format_for_prompt()
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

    fn repair_loaded_patterns(&mut self) {
        for pattern in &mut self.patterns {
            if pattern.key.is_empty() {
                pattern.key = pattern.digest.clone();
            }
            pattern.digest = truncate_chars(&pattern.digest, 200);
        }
    }
}

impl ErrorPattern {
    fn relevance_score(&self, query: FailurePatternQuery<'_>) -> usize {
        let mut score = 0usize;
        if let Some(task_id) = query.task_id
            && self.task_ids.iter().any(|seen| seen == task_id)
        {
            score += 8;
        }
        if let Some(plan_id) = query.plan_id
            && self.plan_ids.iter().any(|seen| seen == plan_id)
        {
            score += 4;
        }
        if let Some(gate) = query.gate
            && self.gate.as_deref() == Some(gate)
        {
            score += 2;
        }
        if let Some(classification) = query.classification
            && self.category == classification
        {
            score += 1;
        }
        score
    }
}

impl FailurePatternQuery<'_> {
    fn is_empty(self) -> bool {
        self.plan_id.is_none()
            && self.task_id.is_none()
            && self.gate.is_none()
            && self.classification.is_none()
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
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

    #[test]
    fn structured_observations_dedupe_by_key() {
        let mut store = ErrorPatternStore::empty();
        let first = GateFailureObservation::new(
            "E0425::src/lib.rs",
            "plan-a",
            Some("task-a".to_string()),
            "compile:cargo",
            "unresolved_import",
            "E0425: cannot find value `foo` [src/lib.rs]",
            GateFailureSource::GateClassification,
        );
        let second = GateFailureObservation::new(
            "E0425::src/lib.rs",
            "plan-b",
            Some("task-b".to_string()),
            "compile:cargo",
            "unresolved_import",
            "E0425: cannot find value `bar` [src/lib.rs]",
            GateFailureSource::GateClassification,
        );
        let different_file = GateFailureObservation::new(
            "E0425::src/other.rs",
            "plan-a",
            Some("task-a".to_string()),
            "compile:cargo",
            "unresolved_import",
            "E0425: cannot find value `foo` [src/other.rs]",
            GateFailureSource::GateClassification,
        );

        assert!(store.observe_gate_failure(first).inserted);
        let update = store.observe_gate_failure(second);
        assert!(!update.inserted);
        assert_eq!(update.occurrences, 2);
        assert!(store.observe_gate_failure(different_file).inserted);

        assert_eq!(store.len(), 2);
        assert_eq!(store.patterns[0].plan_ids, vec!["plan-a", "plan-b"]);
        assert_eq!(store.patterns[0].task_ids, vec!["task-a", "task-b"]);
    }

    #[test]
    fn bounded_summary_limits_patterns_and_size() {
        let mut store = ErrorPatternStore::empty();
        for i in 0..8 {
            let observation = GateFailureObservation::new(
                format!("E04{i:02}::src/lib.rs"),
                "plan-a",
                Some("task-a".to_string()),
                "compile:cargo",
                "type_error",
                format!("E04{i:02}: {}", "x".repeat(500)),
                GateFailureSource::GateClassification,
            );
            store.observe_gate_failure(observation);
        }

        let summary = store.bounded_summary(
            FailurePatternQuery {
                plan_id: Some("plan-a"),
                task_id: Some("task-a"),
                gate: Some("compile:cargo"),
                classification: Some("type_error"),
            },
            5,
            500,
        );

        assert!(summary.patterns.len() <= 5);
        assert!(
            summary
                .patterns
                .iter()
                .all(|p| p.digest.chars().count() <= 200)
        );
        let prompt = summary.format_for_prompt();
        assert!(prompt.contains("Prior Gate Failure Patterns"));
        assert!(!prompt.contains(&"x".repeat(500)));
    }

    #[test]
    fn summary_distinguishes_repeated_from_one_off() {
        let mut store = ErrorPatternStore::empty();
        let observation = GateFailureObservation::new(
            "test::panic::snapshot",
            "plan-a",
            Some("task-a".to_string()),
            "test:cargo",
            "test_expectation_failure",
            "snapshot mismatch",
            GateFailureSource::GateClassification,
        );
        store.observe_gate_failure(observation.clone());
        store.observe_gate_failure(observation);

        let summary = store.bounded_summary(
            FailurePatternQuery {
                gate: Some("test:cargo"),
                ..FailurePatternQuery::default()
            },
            3,
            1_000,
        );

        assert_eq!(summary.patterns.len(), 1);
        assert!(summary.patterns[0].repeated);
        assert!(summary.format_for_prompt().contains("repeated"));
    }
}
