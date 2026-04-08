//! Per-task brief generation.
//!
//! Generates a concise brief for each task that answers three questions:
//!
//! 1. **What**: Task title, files to modify, acceptance criteria.
//! 2. **Why**: Why this task exists — extracted from the plan document by
//!    finding paragraphs that reference this task's files or title.
//! 3. **How**: Existing patterns in the codebase to follow, sibling task
//!    context (what runs before/after).
//!
//! This replaces mori's approach of dumping the full plan brief into every
//! task prompt. Instead, each task gets only the *slice* of the plan that's
//! relevant to it.
//!
//! For mechanical tasks: skip entirely (the task prompt IS the spec).
//! For focused tasks: generate (mostly extraction, no LLM needed).
//! For architectural tasks: same extraction, but could be enriched by LLM
//! in the future for cross-cutting concern analysis.

use std::fmt::Write;
use std::path::Path;

use crate::context_provider::{SiblingTask, TaskInput};

/// Generates per-task briefs.
pub struct TaskBriefGenerator {
    /// Maximum lines from the plan to include in the "Why" section.
    max_plan_lines: usize,
}

impl TaskBriefGenerator {
    /// Create a new brief generator with default settings.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_plan_lines: 30,
        }
    }

    /// Override the maximum plan lines to include.
    #[must_use]
    pub const fn with_max_plan_lines(mut self, n: usize) -> Self {
        self.max_plan_lines = n;
        self
    }

    /// Generate a task brief.
    ///
    /// Returns the formatted brief text, or an empty string if the task
    /// doesn't need one (e.g., mechanical tasks with no plan context).
    #[must_use]
    pub fn generate(
        &self,
        task: &TaskInput,
        plan_doc: Option<&str>,
        siblings: &[SiblingTask],
    ) -> String {
        let mut brief = String::new();

        // ── Section 1: What ────────────────────────────────────────
        brief.push_str("## Task brief\n\n");
        brief.push_str("### What\n");
        let _ = writeln!(brief, "**{}**: {}", task.id, task.title);

        if !task.files.is_empty() {
            brief.push_str("\nFiles to modify:\n");
            for f in &task.files {
                let _ = writeln!(brief, "- `{f}`");
            }
        }

        if let Some(max) = task.max_loc {
            let _ = writeln!(brief, "\nMax lines of change: {max}");
        }

        if !task.acceptance.is_empty() {
            brief.push_str("\nAcceptance criteria:\n");
            for a in &task.acceptance {
                let _ = writeln!(brief, "- {a}");
            }
        }

        // ── Section 2: Why (extracted from plan) ───────────────────
        if let Some(plan) = plan_doc {
            let why = extract_why(plan, task, self.max_plan_lines);
            if !why.is_empty() {
                brief.push_str("\n### Why\n");
                brief.push_str(&why);
                brief.push('\n');
            }
        }

        // ── Section 3: How (sibling context + patterns) ────────────
        let how = generate_how(task, siblings);
        if !how.is_empty() {
            brief.push_str("\n### How\n");
            brief.push_str(&how);
            brief.push('\n');
        }

        brief
    }
}

impl Default for TaskBriefGenerator {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Plan text extraction ──────────────────────────────────────────────────

/// Extract the "Why" section from a plan document.
///
/// Strategy:
/// 1. Find the heading/section that most closely matches the task's files or title.
/// 2. Extract paragraphs that reference the task's files.
/// 3. Include surrounding context (the heading and first paragraph of each section).
/// 4. Cap at `max_lines`.
fn extract_why(plan: &str, task: &TaskInput, max_lines: usize) -> String {
    let mut relevant_lines = Vec::new();

    // Build a set of search terms: filenames + basenames + title words
    let mut search_terms: Vec<String> = task.files.iter()
        .map(|f| f.to_ascii_lowercase())
        .collect();

    // Add basenames
    for f in &task.files {
        if let Some(basename) = Path::new(f).file_name().and_then(|n| n.to_str()) {
            let lower = basename.to_ascii_lowercase();
            if !search_terms.contains(&lower) {
                search_terms.push(lower);
            }
        }
    }

    // Add significant words from the task title (skip common words)
    let skip_words: &[&str] = &[
        "the", "a", "an", "in", "to", "for", "and", "or", "of", "with", "from",
        "into", "on", "at", "by", "is", "are", "was", "were", "be", "been",
        "add", "wire", "implement", "create", "update", "fix", "remove",
    ];
    for word in task.title.split_whitespace() {
        let lower = word.to_ascii_lowercase();
        // Only include words that are meaningful (3+ chars, not common)
        if lower.len() >= 3 && !skip_words.contains(&lower.as_str())
            && !search_terms.contains(&lower)
        {
            search_terms.push(lower);
        }
    }

    if search_terms.is_empty() {
        return String::new();
    }

    // Walk the plan document, scoring each paragraph by relevance
    let paragraphs: Vec<&str> = plan.split("\n\n").collect();
    let mut scored: Vec<(usize, &str)> = Vec::new();

    for para in &paragraphs {
        let lower = para.to_ascii_lowercase();
        let mut score = 0usize;

        for term in &search_terms {
            if lower.contains(term.as_str()) {
                // File path matches are worth more than title word matches
                if term.contains('/')
                    || Path::new(term).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
                    || Path::new(term).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
                {
                    score += 3;
                } else {
                    score += 1;
                }
            }
        }

        if score > 0 {
            scored.push((score, para));
        }
    }

    // Sort by score descending
    scored.sort_by(|a, b| b.0.cmp(&a.0));

    // Take the most relevant paragraphs up to max_lines
    let mut total_lines = 0;
    for (_, para) in &scored {
        let para_lines = para.lines().count();
        if total_lines + para_lines > max_lines {
            break;
        }
        relevant_lines.push(*para);
        total_lines += para_lines;
    }

    relevant_lines.join("\n\n")
}

/// Generate the "How" section: sibling context and pattern hints.
fn generate_how(task: &TaskInput, siblings: &[SiblingTask]) -> String {
    let mut how = String::new();

    // Dependency chain context
    if !task.depends_on.is_empty() {
        how.push_str("**Dependencies** (must complete before this task):\n");
        for dep_id in &task.depends_on {
            if let Some(sibling) = siblings.iter().find(|s| s.id == *dep_id) {
                let status_marker = match sibling.status.as_str() {
                    "done" | "completed" => " (completed)",
                    "running" | "in_progress" => " (running)",
                    _ => "",
                };
                let _ = writeln!(how, "- **{}**: {}{}", sibling.id, sibling.title, status_marker);
            } else {
                let _ = writeln!(how, "- **{dep_id}** (external)");
            }
        }
        how.push('\n');
    }

    // Downstream tasks (tasks that depend on this one)
    // Note: downstream tasks (tasks that depend on this one) are not shown
    // here because we don't have their depends_on info. The sibling list
    // only provides id/title/status.

    // Pattern hint: if modifying existing files, suggest looking at existing patterns
    if !task.files.is_empty() {
        how.push_str("**Patterns**: Look at existing code in the files you're modifying. ");
        how.push_str("Follow the same style, error handling, and test patterns.\n");
    }

    how
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_task(id: &str, title: &str, files: &[&str]) -> TaskInput {
        TaskInput {
            id: id.into(),
            title: title.into(),
            tier: "focused".into(),
            files: files.iter().map(|s| (*s).to_string()).collect(),
            read_files: vec![],
            symbols: vec![],
            anti_patterns: vec![],
            prior_failures: vec![],
            verify_commands: vec![],
            acceptance: vec!["It should compile".into()],
            depends_on: vec![],
            max_loc: Some(50),
        }
    }

    #[test]
    fn generate_brief_includes_what_section() {
        let generator = TaskBriefGenerator::new();
        let task = make_task("T1", "Wire context provider", &["src/orchestrate.rs"]);
        let brief = generator.generate(&task, None, &[]);

        assert!(brief.contains("### What"));
        assert!(brief.contains("T1"));
        assert!(brief.contains("Wire context provider"));
        assert!(brief.contains("orchestrate.rs"));
        assert!(brief.contains("Max lines of change: 50"));
        assert!(brief.contains("It should compile"));
    }

    #[test]
    fn generate_brief_extracts_why_from_plan() {
        let generator = TaskBriefGenerator::new();
        let task = make_task("T1", "Wire context provider", &["src/orchestrate.rs"]);

        let plan = "# Implementation Plan\n\n\
                     This plan implements the context provider system.\n\n\
                     ## Phase 1: Core types\n\n\
                     First, create the core types in context_provider.rs.\n\n\
                     ## Phase 2: Wire into orchestrate.rs\n\n\
                     The orchestrate.rs file needs to call the context provider\n\
                     during dispatch_agent, between task parsing and prompt composition.\n\n\
                     ## Phase 3: Testing\n\n\
                     Write unit tests for all modules.";

        let brief = generator.generate(&task, Some(plan), &[]);

        assert!(brief.contains("### Why"));
        assert!(brief.contains("orchestrate.rs"));
    }

    #[test]
    fn generate_brief_includes_sibling_deps() {
        let generator = TaskBriefGenerator::new();
        let mut task = make_task("T2", "Wire ContextProvider", &["src/orchestrate.rs"]);
        task.depends_on = vec!["T1".into()];

        let siblings = vec![
            SiblingTask {
                id: "T1".into(),
                title: "Create ContextProvider types".into(),
                status: "completed".into(),
            },
            SiblingTask {
                id: "T3".into(),
                title: "Add tests".into(),
                status: "ready".into(),
            },
        ];

        let brief = generator.generate(&task, None, &siblings);

        assert!(brief.contains("### How"));
        assert!(brief.contains("Dependencies"));
        assert!(brief.contains("T1"));
        assert!(brief.contains("Create ContextProvider types"));
        assert!(brief.contains("(completed)"));
    }

    #[test]
    fn extract_why_scores_file_paths_higher() {
        let task = make_task("T1", "Update config", &["crates/roko-cli/src/config.rs"]);

        let plan = "This paragraph mentions unrelated things and the word config.\n\n\
                     This paragraph specifically references crates/roko-cli/src/config.rs\n\
                     and explains why it needs to change.\n\n\
                     Another paragraph about other stuff.";

        let why = extract_why(plan, &task, 30);
        // The paragraph with the full path should be included
        assert!(why.contains("crates/roko-cli/src/config.rs"));
    }

    #[test]
    fn extract_why_respects_max_lines() {
        let task = make_task("T1", "Update config", &["config.rs"]);

        let plan = "Config paragraph one with config.rs reference.\n\n\
                     Config paragraph two also mentioning config.rs. This one is longer\n\
                     and has multiple lines that will push us over the limit.\n\
                     Line 3 of the paragraph.\n\
                     Line 4 of the paragraph.\n\
                     Line 5 of the paragraph.\n\n\
                     Config paragraph three with config.rs.";

        let why = extract_why(plan, &task, 3);
        let line_count = why.lines().count();
        assert!(line_count <= 3, "got {line_count} lines, expected <= 3");
    }

    #[test]
    fn generate_brief_empty_for_no_context() {
        let generator = TaskBriefGenerator::new();
        let task = make_task("T1", "Do something", &[]);
        let brief = generator.generate(&task, None, &[]);

        // Should still have the What section at minimum
        assert!(brief.contains("### What"));
        assert!(brief.contains("T1"));
    }
}
