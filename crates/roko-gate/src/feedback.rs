//! Filtered gate output for agent consumption.
//!
//! Raw gate output (compiler stderr, test logs, linter JSON) is verbose
//! and full of noise that wastes agent context tokens. [`feedback_for_agent`]
//! parses the raw output into a structured [`GateFeedback`] that contains
//! only actionable items, classified by severity.

use serde::{Deserialize, Serialize};

// ─── Severity ───────────────────────────────────────────────────────────

/// Severity level for a feedback item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Severity {
    /// Informational: context that may help but requires no action.
    Info,
    /// Warning: the code works but should be improved.
    Warning,
    /// Error: the code is broken and must be fixed.
    Error,
}

// ─── FeedbackItem ───────────────────────────────────────────────────────

/// A single actionable item extracted from gate output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackItem {
    /// Severity classification.
    pub severity: Severity,
    /// The extracted message text.
    pub message: String,
}

impl FeedbackItem {
    /// Construct a new feedback item.
    #[must_use]
    pub fn new(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            severity,
            message: message.into(),
        }
    }
}

// ─── GateFeedback ───────────────────────────────────────────────────────

/// Structured, filtered feedback from a gate execution.
///
/// Designed to be serialized and injected into an agent's context window
/// without wasting tokens on noise lines like progress bars, download
/// messages, or repeated blank lines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateFeedback {
    /// Which rung produced this feedback.
    pub rung: u8,
    /// Whether the gate passed.
    pub passed: bool,
    /// Error-level items (must fix).
    pub errors: Vec<String>,
    /// Warning-level items (should fix).
    pub warnings: Vec<String>,
    /// Actionable suggestions extracted from the output.
    pub suggestions: Vec<String>,
}

impl GateFeedback {
    /// Total number of items across all categories.
    #[must_use]
    pub fn item_count(&self) -> usize {
        self.errors.len() + self.warnings.len() + self.suggestions.len()
    }

    /// Returns `true` if there are no actionable items at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty() && self.warnings.is_empty() && self.suggestions.is_empty()
    }

    /// Collect all items as [`FeedbackItem`]s, errors first, then warnings,
    /// then suggestions.
    #[must_use]
    pub fn items(&self) -> Vec<FeedbackItem> {
        let mut out = Vec::with_capacity(self.item_count());
        for e in &self.errors {
            out.push(FeedbackItem::new(Severity::Error, e.clone()));
        }
        for w in &self.warnings {
            out.push(FeedbackItem::new(Severity::Warning, w.clone()));
        }
        for s in &self.suggestions {
            out.push(FeedbackItem::new(Severity::Info, s.clone()));
        }
        out
    }
}

// ─── Classifier ─────────────────────────────────────────────────────────

/// Classify a single line into a severity, or `None` if the line is noise.
fn classify_line(line: &str) -> Option<(Severity, &str)> {
    let trimmed = line.trim();

    // Skip empty lines
    if trimmed.is_empty() {
        return None;
    }

    // Skip noise patterns
    if is_noise(trimmed) {
        return None;
    }

    // Error patterns
    if is_error_line(trimmed) {
        return Some((Severity::Error, trimmed));
    }

    // Warning patterns
    if is_warning_line(trimmed) {
        return Some((Severity::Warning, trimmed));
    }

    // Suggestion / help patterns
    if is_suggestion_line(trimmed) {
        return Some((Severity::Info, trimmed));
    }

    // Lines that carry useful context (e.g. file paths, test names)
    // but don't match a specific severity are dropped to avoid noise.
    None
}

/// Returns `true` for lines that are pure noise (progress, download, blank).
fn is_noise(line: &str) -> bool {
    // Cargo download / compile progress
    if line.starts_with("Downloading")
        || line.starts_with("Downloaded")
        || line.starts_with("Compiling")
        || line.starts_with("Checking")
        || line.starts_with("Finished")
        || line.starts_with("Running")
        || line.starts_with("Documenting")
        || line.starts_with("Fresh")
        || line.starts_with("Packaging")
    {
        return true;
    }

    // npm progress
    if line.starts_with("npm WARN") && line.contains("deprecated") {
        return true;
    }

    // Generic progress bars / spinners
    if line.contains("━") || line.contains("▓") || line.contains("░") {
        return true;
    }

    false
}

/// Returns `true` if the line looks like an error diagnostic.
fn is_error_line(line: &str) -> bool {
    line.starts_with("error")
        || line.starts_with("Error:")
        || line.starts_with("ERROR:")
        || line.starts_with("FAILED")
        || line.starts_with("FAIL ")
        || line.contains("error[E")
        || line.contains("panicked at")
        || line.starts_with("thread '") && line.contains("panicked")
}

/// Returns `true` if the line looks like a warning diagnostic.
fn is_warning_line(line: &str) -> bool {
    line.starts_with("warning")
        || line.starts_with("Warning:")
        || line.starts_with("WARNING:")
        || line.starts_with("warn[")
}

/// Returns `true` if the line looks like a suggestion / help message.
fn is_suggestion_line(line: &str) -> bool {
    line.starts_with("help:")
        || line.contains("= help:")
        || line.starts_with("note:")
        || line.contains("= note:")
        || line.starts_with("suggestion:")
        || line.starts_with("hint:")
        || line.starts_with("--> ")
        || line.contains("--> ")
}

// ─── Public API ─────────────────────────────────────────────────────────

/// Parse raw gate output into structured, filtered feedback for an agent.
///
/// Scans each line of `gate_output`, classifies it by severity, and
/// collects actionable items into the appropriate bucket. Noise lines
/// (progress messages, download indicators, etc.) are dropped.
#[must_use]
pub fn feedback_for_agent(gate_output: &str, rung: u8) -> GateFeedback {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut suggestions = Vec::new();
    let mut has_errors = false;

    for line in gate_output.lines() {
        if let Some((severity, text)) = classify_line(line) {
            match severity {
                Severity::Error => {
                    has_errors = true;
                    errors.push(text.to_string());
                }
                Severity::Warning => {
                    warnings.push(text.to_string());
                }
                Severity::Info => {
                    suggestions.push(text.to_string());
                }
            }
        }
    }

    // If there are no classified lines but the output is non-empty and
    // non-trivial, assume it is an unrecognized error format and include
    // a summary as a single error.
    let passed = !has_errors;

    GateFeedback {
        rung,
        passed,
        errors,
        warnings,
        suggestions,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_empty_output_passes() {
        let fb = feedback_for_agent("", 0);
        assert!(fb.passed);
        assert!(fb.errors.is_empty());
        assert!(fb.warnings.is_empty());
        assert!(fb.suggestions.is_empty());
        assert_eq!(fb.rung, 0);
    }

    #[test]
    fn feedback_extracts_errors() {
        let output = "Compiling foo v0.1.0\nerror[E0425]: cannot find value `x`\nerror: aborting due to previous error\n";
        let fb = feedback_for_agent(output, 0);
        assert!(!fb.passed);
        assert_eq!(fb.errors.len(), 2);
        assert!(fb.errors[0].contains("E0425"));
        assert!(fb.errors[1].contains("aborting"));
    }

    #[test]
    fn feedback_extracts_warnings() {
        let output = "warning: unused variable `x`\nwarning: 1 warning emitted\n";
        let fb = feedback_for_agent(output, 1);
        assert!(fb.passed);
        assert_eq!(fb.warnings.len(), 2);
        assert!(fb.warnings[0].contains("unused variable"));
    }

    #[test]
    fn feedback_extracts_suggestions() {
        let output = "help: consider using `let _x` instead\n  = note: `#[warn(unused_variables)]` on by default\n";
        let fb = feedback_for_agent(output, 1);
        assert!(fb.passed);
        assert_eq!(fb.suggestions.len(), 2);
    }

    #[test]
    fn feedback_filters_noise() {
        let output = "Downloading crates ...\nDownloaded serde v1.0.0\nCompiling serde v1.0.0\nChecking mylib v0.1.0\nFinished dev [unoptimized] in 2.3s\n";
        let fb = feedback_for_agent(output, 0);
        assert!(fb.passed);
        assert!(fb.is_empty());
    }

    #[test]
    fn feedback_mixed_output() {
        let output = "\
Compiling foo v0.1.0
warning: unused import `bar`
  --> src/lib.rs:1:5
error[E0433]: failed to resolve
help: use `crate::baz` instead
Finished dev [unoptimized]
";
        let fb = feedback_for_agent(output, 0);
        assert!(!fb.passed);
        assert_eq!(fb.errors.len(), 1);
        assert_eq!(fb.warnings.len(), 1);
        assert_eq!(fb.suggestions.len(), 2); // --> line + help: line
    }

    #[test]
    fn feedback_item_count() {
        let output = "error: one\nwarning: two\nhelp: three\n";
        let fb = feedback_for_agent(output, 2);
        assert_eq!(fb.item_count(), 3);
        assert!(!fb.is_empty());
    }

    #[test]
    fn feedback_items_ordering() {
        let output = "warning: w1\nerror: e1\nhelp: s1\n";
        let fb = feedback_for_agent(output, 0);
        let items = fb.items();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].severity, Severity::Error);
        assert_eq!(items[1].severity, Severity::Warning);
        assert_eq!(items[2].severity, Severity::Info);
    }

    #[test]
    fn feedback_rung_preserved() {
        for rung in 0..=6 {
            let fb = feedback_for_agent("", rung);
            assert_eq!(fb.rung, rung);
        }
    }

    #[test]
    fn feedback_test_failure_detected() {
        let output = "FAILED tests::my_test\nthread 'tests::my_test' panicked at 'assertion failed'\n";
        let fb = feedback_for_agent(output, 2);
        assert!(!fb.passed);
        assert_eq!(fb.errors.len(), 2);
    }

    #[test]
    fn feedback_severity_ordering() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
    }

    #[test]
    fn feedback_classify_line_none_for_empty() {
        assert!(classify_line("").is_none());
        assert!(classify_line("   ").is_none());
    }

    #[test]
    fn feedback_npm_deprecation_is_noise() {
        let line = "npm WARN deprecated stable@0.1.0: deprecated package";
        assert!(is_noise(line));
    }

    #[test]
    fn feedback_progress_bars_are_noise() {
        assert!(is_noise("████████░░░░ 60%"));
        assert!(is_noise("━━━━━━━━━━ progress"));
    }

    #[test]
    fn feedback_serde_roundtrip() {
        let fb = feedback_for_agent("error: bad\nwarning: meh\nhelp: try this\n", 1);
        let json = serde_json::to_string(&fb).unwrap();
        let parsed: GateFeedback = serde_json::from_str(&json).unwrap();
        assert_eq!(fb, parsed);
    }
}
