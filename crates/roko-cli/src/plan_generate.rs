//! `roko plan generate` — intelligent task decomposition from any input source.
//!
//! Takes a PRD, prompt, file, or checklist and produces plan directories
//! with surgically-scoped tasks, executable verification, and model hints.
//!
//! Key principles (from Meta-Harness [Lee et al. 2026]):
//! - Right context, not more context
//! - Tasks ≤50 LOC for Tier 1, ≤20 LOC for Tier 0
//! - Every acceptance criterion is a runnable command
//! - Feedback from failures feeds into retry context

use std::fmt::Write as _;
use std::path::Path;

/// Task tier determines minimum model and maximum scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskTier {
    /// Mechanical: imports, renames, field additions. ≤20 LOC. Haiku-capable.
    Mechanical,
    /// Focused: single function, single test. ≤50 LOC. Sonnet-capable.
    Focused,
    /// Integrative: multi-module connection. ≤150 LOC. Sonnet/Opus.
    Integrative,
    /// Architectural: API design, decomposition. ≤300 LOC. Opus only.
    Architectural,
}

impl TaskTier {
    /// Suggested model for this tier.
    #[must_use]
    pub const fn model_hint(&self) -> &'static str {
        match self {
            Self::Mechanical => "claude-haiku-4-5",
            Self::Focused | Self::Integrative => "claude-sonnet-4-6",
            Self::Architectural => "claude-opus-4-6",
        }
    }

    /// Maximum lines of code change for this tier.
    #[must_use]
    pub const fn max_loc(&self) -> u32 {
        match self {
            Self::Mechanical => 20,
            Self::Focused => 50,
            Self::Integrative => 150,
            Self::Architectural => 300,
        }
    }

    /// Label for TOML output.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Mechanical => "mechanical",
            Self::Focused => "focused",
            Self::Integrative => "integrative",
            Self::Architectural => "architectural",
        }
    }
}

/// The system prompt for the plan generator agent.
///
/// This prompt produces tasks with surgical context, executable verification,
/// and model-adaptive tier hints. It's designed to produce tasks that even
/// the smallest models can execute successfully.
pub const PLAN_GENERATOR_SYSTEM_PROMPT: &str = r#"You are a task decomposition engine for software projects. Your job is to take a feature description and produce a set of tasks that are so precisely scoped that even the smallest, cheapest LLM can execute them correctly.

## Core principles

1. **Surgical scope**: Each task touches 1-2 files, changes ≤50 lines. If a change requires more, split it.
2. **Precise context**: For each task, specify EXACTLY which files and line ranges to read. Not "read the crate" — "read lines 40-80 of src/lib.rs".
3. **Executable verification**: Every acceptance criterion is a shell command that exits 0 on success, 1 on failure. No subjective criteria.
4. **Dependency ordering**: Types before implementations. Implementations before wiring. Wiring before tests.
5. **Model hints**: Assign the cheapest model that can handle each task. Imports → Haiku. Single function → Sonnet. Multi-module wiring → Opus.

## Task tiers

| Tier | Name | Max LOC | Model | Examples |
|------|------|---------|-------|----------|
| 0 | Mechanical | 20 | haiku | Add import, add struct field, rename function |
| 1 | Focused | 50 | sonnet | Implement function body, write single test |
| 2 | Integrative | 150 | sonnet/opus | Wire module A→B, implement trait for type |
| 3 | Architectural | 300 | opus | Design new API, decompose complex feature |

## Output format

Create plan directories with these files:

### tasks.toml
```toml
[meta]
plan = "<slug>"
total = <N>
done = 0
status = "ready"
max_parallel = <N>  # how many can run concurrently

[[task]]
id = "T1"
title = "<imperative verb phrase>"
status = "ready"
tier = "mechanical"       # mechanical | focused | integrative | architectural
model_hint = "haiku"      # cheapest model for this tier
max_loc = 20              # maximum lines of change
files = ["<path>"]        # files this task modifies
allowed_tools = ["read_file", "grep"]
denied_tools = []
mcp_servers = ["filesystem"] # MCP servers this task needs
depends_on = []

# SURGICAL CONTEXT: exactly what the agent needs to read
[task.context]
read_files = [
    { path = "<file>", lines = "40-80", why = "<reason>" },
]
symbols = [
    "<TypeName>::<method> — <brief signature description>",
]
anti_patterns = [
    "Do NOT create new files. Modify <file> only.",
]

# EXECUTABLE VERIFICATION
[[task.verify]]
phase = "structural"
command = "grep -q 'pattern' path/to/file"
fail_msg = "Pattern not found in file"

[[task.verify]]
phase = "compile"
command = "cargo check -p <crate>"

[[task.verify]]
phase = "test"
command = "cargo test -p <crate> -- <test_name>"
```

## Before generating tasks, you MUST:

1. Search the codebase to understand what exists:
   `grep -rn 'TypeName' crates/ --include='*.rs' | grep -v target/ | head -20`

2. Read the specific files you're generating tasks for — understand the current code.

3. Check if the feature already exists (partially or fully):
   `grep -rn 'feature_keyword' crates/ --include='*.rs' | grep -v target/`

4. For each task, verify the context files actually exist:
   `test -f <path> && echo "exists" || echo "MISSING"`

## Language detection

Detect the project language and use the right commands:
- Cargo.toml → Rust: `cargo check`, `cargo test`, `cargo clippy`
- package.json → TypeScript: `npx tsc`, `npx jest`, `npx eslint`
- go.mod → Go: `go build`, `go test`, `golangci-lint`
- pyproject.toml/setup.py → Python: `python -m py_compile`, `pytest`, `ruff`

## Quality gates for YOUR output

Before finalizing, verify your tasks against:
- [ ] Every task has ≤ max_loc lines of change for its tier
- [ ] Every task has at least 1 structural check + 1 compile check
- [ ] No task requires reading more than 3 files
- [ ] Anti-patterns are specific (not generic "be careful")
- [ ] Dependencies form a DAG (no cycles)
- [ ] The cheapest possible model is assigned to each task
"#;

/// Build the full prompt for plan generation from a source input.
pub fn build_generation_prompt(workdir: &Path, source: &str, source_type: &str) -> String {
    let mut prompt = String::new();
    let _ = writeln!(prompt, "{PLAN_GENERATOR_SYSTEM_PROMPT}");
    let _ = writeln!(prompt, "\n---\n");
    let _ = writeln!(prompt, "## Workspace: {}\n", workdir.display());
    let _ = writeln!(
        prompt,
        "## Source type: {source_type}\n\n## Source content:\n\n{source}"
    );
    prompt
}

/// Build a prompt for regenerating an existing plan in place (§11).
///
/// Strips the existing tasks to just `id`/`title`/`depends_on` and asks the
/// agent to fill in `tier`, `model_hint`, `read_files`, `verify`, `context`,
/// and `max_loc`.
pub fn build_regeneration_prompt(workdir: &Path, existing_tasks_toml: &str) -> String {
    let mut prompt = String::new();
    let _ = writeln!(prompt, "{PLAN_GENERATOR_SYSTEM_PROMPT}");
    let _ = writeln!(prompt, "\n---\n");
    let _ = writeln!(prompt, "## Workspace: {}\n", workdir.display());
    let _ = writeln!(prompt, "## Task: Regenerate plan\n");
    let _ = writeln!(
        prompt,
        "The following tasks.toml exists but is missing full metadata (tier, model_hint, \
         read_files, verify, context, max_loc, mcp_servers). Your job is to read the codebase and fill in \
         every field for each task. Keep the existing id, title, and depends_on. Add:\n\
         - `tier` (mechanical/focused/integrative/architectural)\n\
         - `model_hint` (the cheapest model for that tier)\n\
         - `max_loc` (estimated lines of change)\n\
         - `allowed_tools`, `denied_tools`, and `mcp_servers` (per-task tool/MCP constraints)\n\
         - `[task.context]` with read_files, symbols, anti_patterns\n\
         - `[[task.verify]]` with at least compile + test checks\n\n\
         ## Existing tasks.toml:\n\n```toml\n{existing_tasks_toml}\n```"
    );
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_model_hints() {
        assert_eq!(TaskTier::Mechanical.model_hint(), "claude-haiku-4-5");
        assert_eq!(TaskTier::Focused.model_hint(), "claude-sonnet-4-6");
        assert_eq!(TaskTier::Architectural.model_hint(), "claude-opus-4-6");
    }

    #[test]
    fn tier_max_loc() {
        assert_eq!(TaskTier::Mechanical.max_loc(), 20);
        assert_eq!(TaskTier::Focused.max_loc(), 50);
        assert_eq!(TaskTier::Integrative.max_loc(), 150);
        assert_eq!(TaskTier::Architectural.max_loc(), 300);
    }

    #[test]
    fn build_prompt_includes_source() {
        let prompt = build_generation_prompt(
            std::path::Path::new("/test"),
            "Add a logging system",
            "prompt",
        );
        assert!(prompt.contains("Add a logging system"));
        assert!(prompt.contains("Surgical scope"));
        assert!(prompt.contains("/test"));
    }
}
