//! Per-role prompt templates.
//!
//! Each template takes a typed input struct (pre-read strings, no I/O) and
//! emits a `Vec<PromptSection>` ready for [`PromptComposer`](crate::PromptComposer).
//!
//! Anti-pattern #8: **no `std::fs` in this module tree**. All file content
//! arrives via the `*Input` structs.

pub mod assembly;
pub mod common;
pub mod implementer;
pub mod integration;
pub mod prompts;
pub mod quick;
pub mod reviewer;
pub mod scribe;
pub mod strategist;
pub mod task_impl;

pub use assembly::PromptAssembler;
pub use common::{PromptBudget, budget_for};
pub use implementer::{ImplementerInput, ImplementerTemplate};
pub use integration::{IntegrationInput, IntegrationTemplate};
pub use quick::{QuickFixInput, QuickFixTemplate, QuickReviewerInput, QuickReviewerTemplate};
pub use reviewer::{Reviewer, ReviewerInput, ReviewerTemplate};
pub use scribe::{FileSnippet, ScribeInput, ScribeTemplate};
pub use strategist::{StrategistInput, StrategistTemplate};
pub use task_impl::{SiblingTask, TaskImplInput, TaskImplTemplate};

use crate::PromptSection;

/// Shared input fragment for plan metadata.
#[derive(Clone, Debug, Default)]
pub struct PlanSlice {
    /// Plan number (e.g. "042").
    pub num: String,
    /// Plan base name (e.g. "golem-mortality").
    pub base: String,
    /// Plan title for display.
    pub title: String,
    /// Full plan content (markdown).
    pub content: String,
}

/// Typed task-level enhancements from the enrichment pipeline.
#[derive(Clone, Debug, Default)]
pub struct TaskEnhancements {
    /// Types the implementer should define.
    pub types_to_define: Vec<String>,
    /// Formulas the implementer should implement.
    pub formulas: Vec<String>,
    /// Import paths the implementer needs.
    pub imports: Vec<String>,
    /// An example pattern to follow.
    pub example_pattern: Option<String>,
    /// Invariants that tests should verify.
    pub test_invariants: Vec<String>,
}

impl TaskEnhancements {
    /// True when every sub-vec/option is empty — signals omit the section.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.types_to_define.is_empty()
            && self.formulas.is_empty()
            && self.imports.is_empty()
            && self.example_pattern.is_none()
            && self.test_invariants.is_empty()
    }
}

/// A role-specific prompt template.
///
/// Every template follows the same contract: typed input in, section list out.
/// No filesystem reads, no network calls.
pub trait RolePromptTemplate {
    /// Typed input struct — the only interface. No filesystem reads.
    type Input;

    /// Produce a set of prompt sections from the input.
    ///
    /// The caller hands these to [`PromptComposer`](crate::PromptComposer)
    /// along with a budget.
    fn sections(&self, input: &Self::Input) -> Vec<PromptSection>;

    /// The role identity opening text (layer 1 of the system prompt).
    fn role_identity(&self) -> &'static str;
}

/// Truncate `s` to at most `max_chars`, cutting at the last newline boundary
/// before the limit. Appends a truncation marker.
///
/// Port of Mori's `truncate()` helper (prompts.rs:744).
pub fn truncate(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        return s.to_string();
    }
    // Walk back to a char boundary, then to a newline.
    let mut boundary = max_chars;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    // Find last newline at or before boundary.
    if let Some(nl) = s[..boundary].rfind('\n') {
        boundary = nl + 1;
    }
    let dropped = s.len() - boundary;
    format!("{}…[truncated {} chars]", &s[..boundary], dropped)
}

/// Truncate from the *tail* — keeps the last `max_chars` of `s`.
///
/// Port of Mori's `truncate_tail()` helper (prompts.rs:760).
pub fn truncate_tail(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        return s.to_string();
    }
    let start = s.len() - max_chars;
    // Walk forward to a char boundary, then to a newline.
    let mut boundary = start;
    while boundary < s.len() && !s.is_char_boundary(boundary) {
        boundary += 1;
    }
    if let Some(nl) = s[boundary..].find('\n') {
        boundary += nl + 1;
    }
    let dropped = boundary;
    format!("…[truncated {} chars]\n{}", dropped, &s[boundary..])
}

/// Format an enhanced-sections block from [`TaskEnhancements`].
///
/// Returns an empty string when all enhancement fields are empty.
pub fn format_enhancements(e: &TaskEnhancements) -> String {
    use std::fmt::Write;

    if e.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    if !e.types_to_define.is_empty() {
        out.push_str("### Types to Define\n");
        for t in &e.types_to_define {
            let _ = writeln!(out, "- {t}");
        }
        out.push('\n');
    }
    if !e.formulas.is_empty() {
        out.push_str("### Formulas\n");
        for f in &e.formulas {
            let _ = writeln!(out, "- {f}");
        }
        out.push('\n');
    }
    if !e.imports.is_empty() {
        out.push_str("### Imports\n");
        for i in &e.imports {
            let _ = writeln!(out, "- {i}");
        }
        out.push('\n');
    }
    if let Some(ref pat) = e.example_pattern {
        let _ = write!(out, "### Example Pattern\n{pat}\n\n");
    }
    if !e.test_invariants.is_empty() {
        out.push_str("### Test Invariants\n");
        for inv in &e.test_invariants {
            let _ = writeln!(out, "- {inv}");
        }
        out.push('\n');
    }
    out
}

/// Format a list of files changed.
pub fn format_files_changed(files: &[String]) -> String {
    use std::fmt::Write;

    if files.is_empty() {
        return String::new();
    }
    let mut out = String::from("Files changed:\n");
    for f in files {
        let _ = writeln!(out, "- {f}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_noop_when_under_limit() {
        let s = "hello\nworld\n";
        assert_eq!(truncate(s, 100), s);
    }

    #[test]
    fn truncate_cuts_at_newline() {
        let s = "line1\nline2\nline3\nline4\n";
        let t = truncate(s, 14);
        assert!(t.contains("line1\nline2\n"));
        assert!(t.contains("truncated"));
        assert!(!t.contains("line3"));
    }

    #[test]
    fn truncate_tail_noop_when_under_limit() {
        let s = "hello\nworld\n";
        assert_eq!(truncate_tail(s, 100), s);
    }

    #[test]
    fn truncate_tail_keeps_end() {
        let s = "line1\nline2\nline3\nline4\n";
        let t = truncate_tail(s, 14);
        assert!(t.contains("line4"));
        assert!(t.contains("truncated"));
    }

    #[test]
    fn task_enhancements_empty_check() {
        assert!(TaskEnhancements::default().is_empty());
        assert!(
            !TaskEnhancements {
                types_to_define: vec!["Foo".into()],
                ..Default::default()
            }
            .is_empty()
        );
    }

    #[test]
    fn format_enhancements_empty_returns_empty_string() {
        assert!(format_enhancements(&TaskEnhancements::default()).is_empty());
    }

    #[test]
    fn format_enhancements_renders_all_fields() {
        let e = TaskEnhancements {
            types_to_define: vec!["Foo".into()],
            formulas: vec!["x = y + z".into()],
            imports: vec!["use crate::Foo".into()],
            example_pattern: Some("match x { ... }".into()),
            test_invariants: vec!["INV-001".into()],
        };
        let out = format_enhancements(&e);
        assert!(out.contains("Types to Define"));
        assert!(out.contains("Foo"));
        assert!(out.contains("Formulas"));
        assert!(out.contains("Example Pattern"));
        assert!(out.contains("Test Invariants"));
    }
}
