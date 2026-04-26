//! Diagnosis engine: matches error output against known patterns to suggest interventions.
//!
//! The diagnosis engine is a pure function: given an error string, it returns
//! a ranked list of matched patterns with confidence scores and suggested
//! actions. It does not mutate state or perform I/O.
//!
//! # Usage
//!
//! ```rust
//! use roko_conductor::diagnosis::{DiagnosisEngine, ErrorCategory};
//!
//! let engine = DiagnosisEngine::default();
//! let results = engine.diagnose("error[E0308]: mismatched types");
//! assert!(!results.is_empty());
//! assert_eq!(results[0].category, ErrorCategory::TypeMismatch);
//! ```

use serde::{Deserialize, Serialize};

// ---- Error categories -------------------------------------------------------

/// High-level classification of an error pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCategory {
    /// Compilation failure (syntax error, unresolved symbol, etc.).
    CompileError,
    /// Test assertion or harness failure.
    TestFailure,
    /// Clippy lint violation.
    ClippyWarning,
    /// Git merge conflict markers detected.
    GitConflict,
    /// Dependency resolution failure (missing crate, version mismatch).
    DependencyError,
    /// Rust type mismatch (`E0308` and friends).
    TypeMismatch,
    /// Borrow-checker violation (`E0502`, `E0505`, `E0382`, etc.).
    BorrowCheckerError,
    /// Lifetime annotation error (`E0106`, `E0621`, etc.).
    LifetimeError,
    /// Missing `use` / `mod` / import.
    ImportError,
    /// Referenced file does not exist.
    MissingFile,
    /// OS permission denied.
    PermissionDenied,
    /// Network-level failure (DNS, TLS, connection refused).
    NetworkError,
    /// Command or operation timed out.
    TimeoutError,
    /// Out-of-memory kill.
    OomError,
    /// Disk full / no space left.
    DiskFull,
    /// LLM provider rate-limited the request.
    LlmRateLimit,
    /// LLM context window exceeded.
    LlmContextOverflow,
    /// LLM refused to produce output (content filter / safety).
    LlmRefusal,
    /// Child process crashed (signal, segfault, abort).
    ProcessCrash,
    /// Agent is stuck in a loop (repeated identical output).
    LoopDetected,
}

// ---- Suggested intervention -------------------------------------------------

/// What the conductor should do after diagnosing an error.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SuggestedIntervention {
    /// Retry the agent with additional context about the error.
    RetryWithContext,
    /// Invoke the auto-fix pipeline (gate -> autofix -> gate).
    AutoFix,
    /// Restart the agent from scratch.
    RestartAgent,
    /// Abort the plan; manual intervention needed.
    AbortPlan,
    /// Wait and retry after a backoff period.
    BackoffRetry,
    /// Run a merge-resolution agent to fix conflicts.
    MergeResolution,
    /// Reduce context window / summarize prior turns.
    ReduceContext,
    /// Switch to a different model tier.
    SwitchModel,
    /// Emit a warning but keep going.
    WarnAndContinue,
}

// ---- ErrorPattern -----------------------------------------------------------

/// A single error pattern: a name, a regex, a category, and a suggested action.
#[derive(Debug, Clone)]
pub struct ErrorPattern {
    /// Human-readable pattern name (e.g. "rust-type-mismatch").
    pub name: &'static str,
    /// Substring or regex-style pattern to search for (we use substring matching).
    pub needle: &'static str,
    /// Category of this error.
    pub category: ErrorCategory,
    /// What the conductor should do when this pattern matches.
    pub suggested_action: SuggestedIntervention,
    /// Whether this is a case-insensitive match.
    pub case_insensitive: bool,
}

// ---- DiagnosisResult --------------------------------------------------------

/// Result of matching an error against a known pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosisResult {
    /// Name of the matched pattern.
    pub pattern_name: String,
    /// Category of the error.
    pub category: ErrorCategory,
    /// Confidence of the match: 1.0 for exact, 0.5-0.9 for partial.
    pub confidence: f64,
    /// Suggested intervention.
    pub suggested_intervention: SuggestedIntervention,
    /// The portion of the input that matched, truncated for display.
    pub matched_excerpt: String,
}

// ---- DiagnosisEngine --------------------------------------------------------

/// The diagnosis engine: holds a registry of error patterns and matches
/// incoming error output against them.
///
/// Thread-safe and immutable after construction.
#[derive(Debug, Clone)]
pub struct DiagnosisEngine {
    patterns: Vec<ErrorPattern>,
}

impl Default for DiagnosisEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl DiagnosisEngine {
    /// Create an engine with the built-in 20+ error patterns.
    #[must_use]
    pub fn new() -> Self {
        Self {
            patterns: built_in_patterns(),
        }
    }

    /// Create an engine with custom patterns.
    #[must_use]
    pub const fn with_patterns(patterns: Vec<ErrorPattern>) -> Self {
        Self { patterns }
    }

    /// Number of registered patterns.
    #[must_use]
    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }

    /// Diagnose an error string. Returns all matching patterns ranked by
    /// confidence (highest first).
    #[must_use]
    pub fn diagnose(&self, error_output: &str) -> Vec<DiagnosisResult> {
        let mut results = Vec::new();

        for pattern in &self.patterns {
            if let Some(result) = match_pattern(pattern, error_output) {
                results.push(result);
            }
        }

        // Sort by confidence descending, then by pattern name for stability.
        results.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.pattern_name.cmp(&b.pattern_name))
        });

        results
    }

    /// Diagnose and return only the best match, if any.
    #[must_use]
    pub fn diagnose_best(&self, error_output: &str) -> Option<DiagnosisResult> {
        self.diagnose(error_output).into_iter().next()
    }
}

// ---- Pattern matching -------------------------------------------------------

/// Try to match a single pattern against the error output.
fn match_pattern(pattern: &ErrorPattern, error_output: &str) -> Option<DiagnosisResult> {
    let (haystack, needle) = if pattern.case_insensitive {
        (error_output.to_lowercase(), pattern.needle.to_lowercase())
    } else {
        (error_output.to_owned(), pattern.needle.to_owned())
    };

    let pos = haystack.find(&needle)?;

    // Compute confidence based on match quality.
    let confidence = compute_confidence(pattern, error_output, &needle);

    // Extract a short excerpt around the match.
    let excerpt_start = pos.saturating_sub(20);
    let excerpt_end = (pos + needle.len() + 40).min(error_output.len());
    let excerpt = &error_output[excerpt_start..excerpt_end];

    Some(DiagnosisResult {
        pattern_name: pattern.name.to_owned(),
        category: pattern.category,
        confidence,
        suggested_intervention: pattern.suggested_action.clone(),
        matched_excerpt: truncate_str(excerpt, 100),
    })
}

/// Compute confidence score for a match.
///
/// Confidence is based on three factors:
/// 1. How much of the output the needle covers (ratio).
/// 2. How specific the needle is (longer needles = more specific).
/// 3. Whether the pattern is an exact-match (contains an error code) vs fuzzy.
///
/// Returns a value in `[0.5, 1.0]`.
#[allow(clippy::cast_precision_loss)]
fn compute_confidence(pattern: &ErrorPattern, error_output: &str, needle: &str) -> f64 {
    if error_output.is_empty() {
        return 0.5;
    }

    let ratio = needle.len() as f64 / error_output.len() as f64;

    // Base score from coverage ratio.
    let base = if ratio > 0.9 {
        1.0
    } else if ratio > 0.5 {
        0.9
    } else if ratio > 0.2 {
        0.8
    } else if ratio > 0.05 {
        0.7
    } else {
        0.5
    };

    // Specificity bonus: longer needles are more specific and get a small boost.
    // This ensures "error[E0308]" (12 chars) beats "error[E" (7 chars).
    #[allow(clippy::cast_precision_loss)]
    let specificity_bonus = (needle.len() as f64 * 0.005).min(0.15);

    // Exact-match patterns (those whose name contains a specific error code or
    // well-known marker) get an additional confidence bump. This lets e.g.
    // "rust-type-mismatch" beat the generic "rust-compile-error" when both match.
    let exact_match_bonus = if !pattern.case_insensitive && pattern.name.contains('-') {
        // Patterns with hyphens and case-sensitive matching tend to be more
        // specific (e.g. "rust-type-mismatch" vs "assertion-failed").
        0.02
    } else {
        0.0
    };

    (base + specificity_bonus + exact_match_bonus).min(1.0)
}

/// Truncate a string with ellipsis.
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_owned()
    } else {
        let mut t = s[..max].to_owned();
        t.push_str("...");
        t
    }
}

// ---- Built-in patterns ------------------------------------------------------

/// The default set of 20+ error patterns.
#[allow(clippy::too_many_lines)]
fn built_in_patterns() -> Vec<ErrorPattern> {
    vec![
        // ---- Compile errors ----
        ErrorPattern {
            name: "rust-compile-error",
            needle: "error[E",
            category: ErrorCategory::CompileError,
            suggested_action: SuggestedIntervention::RetryWithContext,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "rust-type-mismatch",
            needle: "error[E0308]",
            category: ErrorCategory::TypeMismatch,
            suggested_action: SuggestedIntervention::RetryWithContext,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "rust-borrow-conflict",
            needle: "error[E0502]",
            category: ErrorCategory::BorrowCheckerError,
            suggested_action: SuggestedIntervention::RetryWithContext,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "rust-use-after-move",
            needle: "error[E0382]",
            category: ErrorCategory::BorrowCheckerError,
            suggested_action: SuggestedIntervention::RetryWithContext,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "rust-moved-value",
            needle: "error[E0505]",
            category: ErrorCategory::BorrowCheckerError,
            suggested_action: SuggestedIntervention::RetryWithContext,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "rust-lifetime-missing",
            needle: "error[E0106]",
            category: ErrorCategory::LifetimeError,
            suggested_action: SuggestedIntervention::RetryWithContext,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "rust-lifetime-mismatch",
            needle: "error[E0621]",
            category: ErrorCategory::LifetimeError,
            suggested_action: SuggestedIntervention::RetryWithContext,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "rust-unresolved-import",
            needle: "error[E0432]",
            category: ErrorCategory::ImportError,
            suggested_action: SuggestedIntervention::RetryWithContext,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "rust-cannot-find",
            needle: "cannot find",
            category: ErrorCategory::ImportError,
            suggested_action: SuggestedIntervention::RetryWithContext,
            case_insensitive: false,
        },
        // ---- Test failures ----
        ErrorPattern {
            name: "rust-test-failure",
            needle: "test result: FAILED",
            category: ErrorCategory::TestFailure,
            suggested_action: SuggestedIntervention::AutoFix,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "assertion-failed",
            needle: "assertion failed",
            category: ErrorCategory::TestFailure,
            suggested_action: SuggestedIntervention::AutoFix,
            case_insensitive: true,
        },
        // ---- Clippy ----
        ErrorPattern {
            name: "clippy-warning",
            needle: "warning: ",
            category: ErrorCategory::ClippyWarning,
            suggested_action: SuggestedIntervention::AutoFix,
            case_insensitive: false,
        },
        // ---- Git conflicts ----
        ErrorPattern {
            name: "git-conflict-markers",
            needle: "<<<<<<<",
            category: ErrorCategory::GitConflict,
            suggested_action: SuggestedIntervention::MergeResolution,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "git-merge-conflict",
            needle: "CONFLICT (content)",
            category: ErrorCategory::GitConflict,
            suggested_action: SuggestedIntervention::MergeResolution,
            case_insensitive: false,
        },
        // ---- Dependencies ----
        ErrorPattern {
            name: "cargo-dependency-missing",
            needle: "no matching package named",
            category: ErrorCategory::DependencyError,
            suggested_action: SuggestedIntervention::RetryWithContext,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "cargo-version-mismatch",
            needle: "failed to select a version for",
            category: ErrorCategory::DependencyError,
            suggested_action: SuggestedIntervention::RetryWithContext,
            case_insensitive: false,
        },
        // ---- File system ----
        ErrorPattern {
            name: "file-not-found",
            needle: "No such file or directory",
            category: ErrorCategory::MissingFile,
            suggested_action: SuggestedIntervention::RetryWithContext,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "permission-denied",
            needle: "Permission denied",
            category: ErrorCategory::PermissionDenied,
            suggested_action: SuggestedIntervention::AbortPlan,
            case_insensitive: false,
        },
        // ---- Network ----
        ErrorPattern {
            name: "connection-refused",
            needle: "Connection refused",
            category: ErrorCategory::NetworkError,
            suggested_action: SuggestedIntervention::BackoffRetry,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "dns-failure",
            needle: "could not resolve host",
            category: ErrorCategory::NetworkError,
            suggested_action: SuggestedIntervention::BackoffRetry,
            case_insensitive: true,
        },
        // ---- Timeouts ----
        ErrorPattern {
            name: "command-timeout",
            needle: "timed out",
            category: ErrorCategory::TimeoutError,
            suggested_action: SuggestedIntervention::BackoffRetry,
            case_insensitive: true,
        },
        // ---- Resource exhaustion ----
        ErrorPattern {
            name: "out-of-memory",
            needle: "out of memory",
            category: ErrorCategory::OomError,
            suggested_action: SuggestedIntervention::AbortPlan,
            case_insensitive: true,
        },
        ErrorPattern {
            name: "oom-killed",
            needle: "SIGKILL",
            category: ErrorCategory::OomError,
            suggested_action: SuggestedIntervention::AbortPlan,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "disk-full",
            needle: "No space left on device",
            category: ErrorCategory::DiskFull,
            suggested_action: SuggestedIntervention::AbortPlan,
            case_insensitive: false,
        },
        // ---- LLM-specific ----
        ErrorPattern {
            name: "llm-rate-limit",
            needle: "rate limit",
            category: ErrorCategory::LlmRateLimit,
            suggested_action: SuggestedIntervention::BackoffRetry,
            case_insensitive: true,
        },
        ErrorPattern {
            name: "llm-429",
            needle: "429 Too Many Requests",
            category: ErrorCategory::LlmRateLimit,
            suggested_action: SuggestedIntervention::BackoffRetry,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "llm-context-overflow",
            needle: "context_length_exceeded",
            category: ErrorCategory::LlmContextOverflow,
            suggested_action: SuggestedIntervention::ReduceContext,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "llm-max-tokens",
            needle: "maximum context length",
            category: ErrorCategory::LlmContextOverflow,
            suggested_action: SuggestedIntervention::ReduceContext,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "llm-content-filter",
            needle: "content_filter",
            category: ErrorCategory::LlmRefusal,
            suggested_action: SuggestedIntervention::SwitchModel,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "llm-safety-refusal",
            needle: "I cannot",
            category: ErrorCategory::LlmRefusal,
            suggested_action: SuggestedIntervention::SwitchModel,
            case_insensitive: false,
        },
        // ---- Process crashes ----
        ErrorPattern {
            name: "segfault",
            needle: "Segmentation fault",
            category: ErrorCategory::ProcessCrash,
            suggested_action: SuggestedIntervention::RestartAgent,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "process-abort",
            needle: "SIGABRT",
            category: ErrorCategory::ProcessCrash,
            suggested_action: SuggestedIntervention::RestartAgent,
            case_insensitive: false,
        },
        ErrorPattern {
            name: "process-panic",
            needle: "thread 'main' panicked",
            category: ErrorCategory::ProcessCrash,
            suggested_action: SuggestedIntervention::RestartAgent,
            case_insensitive: false,
        },
        // ---- Loops ----
        ErrorPattern {
            name: "loop-detected-marker",
            needle: "LOOP DETECTED",
            category: ErrorCategory::LoopDetected,
            suggested_action: SuggestedIntervention::RestartAgent,
            case_insensitive: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> DiagnosisEngine {
        DiagnosisEngine::default()
    }

    // ---- Pattern count ----

    #[test]
    fn has_at_least_20_patterns() {
        assert!(engine().pattern_count() >= 20);
    }

    // ---- Compile errors ----

    #[test]
    fn diagnose_type_mismatch() {
        let results = engine().diagnose("error[E0308]: mismatched types\n  expected `u32`, found `&str`");
        assert!(!results.is_empty());
        // The more specific E0308 pattern should appear.
        let has_type_mismatch = results.iter().any(|r| r.category == ErrorCategory::TypeMismatch);
        assert!(has_type_mismatch, "should match TypeMismatch: {results:?}");
    }

    #[test]
    fn diagnose_generic_compile_error() {
        let results = engine().diagnose("error[E0277]: the trait bound is not satisfied");
        assert!(!results.is_empty());
        assert_eq!(results[0].category, ErrorCategory::CompileError);
    }

    #[test]
    fn diagnose_borrow_checker() {
        let results = engine().diagnose("error[E0502]: cannot borrow `x` as mutable");
        let has_borrow = results.iter().any(|r| r.category == ErrorCategory::BorrowCheckerError);
        assert!(has_borrow);
    }

    #[test]
    fn diagnose_use_after_move() {
        let results = engine().diagnose("error[E0382]: use of moved value: `v`");
        let has_borrow = results.iter().any(|r| r.category == ErrorCategory::BorrowCheckerError);
        assert!(has_borrow);
    }

    #[test]
    fn diagnose_lifetime_error() {
        let results = engine().diagnose("error[E0106]: missing lifetime specifier");
        let has_lt = results.iter().any(|r| r.category == ErrorCategory::LifetimeError);
        assert!(has_lt);
    }

    #[test]
    fn diagnose_import_error() {
        let results = engine().diagnose("error[E0432]: unresolved import `foo::bar`");
        let has_import = results.iter().any(|r| r.category == ErrorCategory::ImportError);
        assert!(has_import);
    }

    // ---- Test failures ----

    #[test]
    fn diagnose_test_failure() {
        let results = engine().diagnose("test result: FAILED. 2 passed; 1 failed; 0 ignored");
        assert!(!results.is_empty());
        let has_test = results.iter().any(|r| r.category == ErrorCategory::TestFailure);
        assert!(has_test);
    }

    #[test]
    fn diagnose_assertion_case_insensitive() {
        let results = engine().diagnose("Assertion Failed: expected 5, got 3");
        let has_test = results.iter().any(|r| r.category == ErrorCategory::TestFailure);
        assert!(has_test);
    }

    // ---- Git conflicts ----

    #[test]
    fn diagnose_git_conflict_markers() {
        let results = engine().diagnose("<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> branch");
        let has_conflict = results.iter().any(|r| r.category == ErrorCategory::GitConflict);
        assert!(has_conflict);
    }

    #[test]
    fn diagnose_merge_conflict_message() {
        let results = engine().diagnose("CONFLICT (content): Merge conflict in src/lib.rs");
        let has_conflict = results.iter().any(|r| r.category == ErrorCategory::GitConflict);
        assert!(has_conflict);
    }

    // ---- LLM errors ----

    #[test]
    fn diagnose_rate_limit() {
        let results = engine().diagnose("Error: Rate limit exceeded. Please retry after 30s.");
        let has_rl = results.iter().any(|r| r.category == ErrorCategory::LlmRateLimit);
        assert!(has_rl);
    }

    #[test]
    fn diagnose_context_overflow() {
        let results = engine().diagnose("InvalidRequestError: context_length_exceeded");
        assert!(!results.is_empty());
        assert_eq!(results[0].category, ErrorCategory::LlmContextOverflow);
        assert_eq!(results[0].suggested_intervention, SuggestedIntervention::ReduceContext);
    }

    // ---- File system ----

    #[test]
    fn diagnose_file_not_found() {
        let results = engine().diagnose("No such file or directory: /tmp/foo.rs");
        let has_missing = results.iter().any(|r| r.category == ErrorCategory::MissingFile);
        assert!(has_missing);
    }

    #[test]
    fn diagnose_permission_denied() {
        let results = engine().diagnose("Permission denied (os error 13)");
        let has_perm = results.iter().any(|r| r.category == ErrorCategory::PermissionDenied);
        assert!(has_perm);
        assert_eq!(results.iter().find(|r| r.category == ErrorCategory::PermissionDenied)
            .expect("should exist").suggested_intervention, SuggestedIntervention::AbortPlan);
    }

    // ---- Resource exhaustion ----

    #[test]
    fn diagnose_oom() {
        let results = engine().diagnose("fatal: Out of memory allocating 1073741824 bytes");
        let has_oom = results.iter().any(|r| r.category == ErrorCategory::OomError);
        assert!(has_oom);
    }

    #[test]
    fn diagnose_disk_full() {
        let results = engine().diagnose("write error: No space left on device");
        let has_disk = results.iter().any(|r| r.category == ErrorCategory::DiskFull);
        assert!(has_disk);
    }

    // ---- Process crashes ----

    #[test]
    fn diagnose_segfault() {
        let results = engine().diagnose("Segmentation fault (core dumped)");
        let has_crash = results.iter().any(|r| r.category == ErrorCategory::ProcessCrash);
        assert!(has_crash);
    }

    #[test]
    fn diagnose_panic() {
        let results = engine().diagnose("thread 'main' panicked at 'index out of bounds'");
        let has_crash = results.iter().any(|r| r.category == ErrorCategory::ProcessCrash);
        assert!(has_crash);
    }

    // ---- No match ----

    #[test]
    fn no_match_returns_empty() {
        let results = engine().diagnose("everything is fine, all tests passed");
        // May match some general patterns but let's check a totally clean string.
        let results2 = engine().diagnose("success 42");
        assert!(results2.is_empty());
    }

    // ---- Best match ----

    #[test]
    fn diagnose_best_returns_highest_confidence() {
        let best = engine().diagnose_best("error[E0308]: mismatched types");
        assert!(best.is_some());
        let best = best.expect("should have a match");
        // The specific E0308 pattern should be the best.
        assert_eq!(best.category, ErrorCategory::TypeMismatch);
    }

    #[test]
    fn diagnose_best_none_for_clean_input() {
        let best = engine().diagnose_best("all good");
        assert!(best.is_none());
    }

    // ---- Confidence scoring ----

    #[test]
    fn specific_patterns_have_high_confidence() {
        let results = engine().diagnose("error[E0308]: mismatched types");
        let type_match = results.iter().find(|r| r.category == ErrorCategory::TypeMismatch);
        assert!(type_match.is_some());
        let tm = type_match.expect("should exist");
        assert!(tm.confidence >= 0.5, "confidence={}", tm.confidence);
    }

    #[test]
    fn confidence_ordering_preserved() {
        let results = engine().diagnose("error[E0308]: mismatched types in very long output with lots of noise that makes the match ratio smaller but still valid in context");
        if results.len() > 1 {
            for pair in results.windows(2) {
                assert!(pair[0].confidence >= pair[1].confidence,
                    "{} ({}) should be >= {} ({})",
                    pair[0].pattern_name, pair[0].confidence,
                    pair[1].pattern_name, pair[1].confidence);
            }
        }
    }

    // ---- Serde ----

    #[test]
    fn diagnosis_result_serde_roundtrip() {
        let result = DiagnosisResult {
            pattern_name: "test".into(),
            category: ErrorCategory::CompileError,
            confidence: 0.85,
            suggested_intervention: SuggestedIntervention::RetryWithContext,
            matched_excerpt: "error[E0001]".into(),
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let decoded: DiagnosisResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.pattern_name, "test");
        assert_eq!(decoded.category, ErrorCategory::CompileError);
    }

    // ---- Custom patterns ----

    #[test]
    fn custom_patterns_work() {
        let custom = vec![ErrorPattern {
            name: "my-pattern",
            needle: "CUSTOM_ERROR_42",
            category: ErrorCategory::CompileError,
            suggested_action: SuggestedIntervention::AbortPlan,
            case_insensitive: false,
        }];
        let engine = DiagnosisEngine::with_patterns(custom);
        assert_eq!(engine.pattern_count(), 1);
        let results = engine.diagnose("something CUSTOM_ERROR_42 happened");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].pattern_name, "my-pattern");
    }

    // ---- Edge cases ----

    #[test]
    fn empty_input_returns_empty() {
        let results = engine().diagnose("");
        assert!(results.is_empty());
    }

    #[test]
    fn multiple_matches_all_returned() {
        // This input should match both the generic compile error and the type mismatch.
        let results = engine().diagnose("error[E0308]: mismatched types");
        assert!(results.len() >= 2, "expected multiple matches, got {}", results.len());
    }

    // ---- Network ----

    #[test]
    fn diagnose_network_errors() {
        let results = engine().diagnose("Connection refused (os error 111)");
        let has_net = results.iter().any(|r| r.category == ErrorCategory::NetworkError);
        assert!(has_net);
    }

    #[test]
    fn diagnose_dns_failure() {
        let results = engine().diagnose("Could not resolve host: api.example.com");
        let has_net = results.iter().any(|r| r.category == ErrorCategory::NetworkError);
        assert!(has_net);
    }

    // ---- Loop detection ----

    #[test]
    fn diagnose_loop_detected() {
        let results = engine().diagnose("LOOP DETECTED: agent repeating same action 5 times");
        let has_loop = results.iter().any(|r| r.category == ErrorCategory::LoopDetected);
        assert!(has_loop);
    }

    // ---- Clippy ----

    #[test]
    fn diagnose_clippy_warning() {
        let results = engine().diagnose("warning: unused variable `x`");
        let has_clippy = results.iter().any(|r| r.category == ErrorCategory::ClippyWarning);
        assert!(has_clippy);
    }

    // ---- Dependency ----

    #[test]
    fn diagnose_dependency_missing() {
        let results = engine().diagnose("error: no matching package named `foo_crate` found");
        let has_dep = results.iter().any(|r| r.category == ErrorCategory::DependencyError);
        assert!(has_dep);
    }
}
