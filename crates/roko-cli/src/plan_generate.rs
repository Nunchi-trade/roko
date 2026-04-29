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

use crate::repo_context::RepoContextPack;

const NAMING_GLOSSARY_RELATIVE_PATH: &str = "docs/00-architecture/01-naming-and-glossary.md";
const NAMING_GLOSSARY_MAX_LINES: usize = 160;
const CLAUDE_MD_RELATIVE_PATH: &str = "CLAUDE.md";
const CLAUDE_MD_MAX_LINES: usize = 120;

/// Built-in plan generation template presets.
///
/// The PRD frontmatter selects one of these presets. Each preset controls the
/// generator's default model tier, gate strictness guidance, and total task
/// budget. Unknown or missing template names fall back to [`Default`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanTemplateKind {
    /// Current behavior: balanced defaults.
    Default,
    /// Smaller, tighter plans with fewer tasks.
    Compact,
    /// More conservative plans with stricter gates.
    Strict,
}

impl PlanTemplateKind {
    /// Resolve a template name from PRD frontmatter.
    #[must_use]
    pub(crate) fn resolve(name: Option<&str>) -> Self {
        let Some(name) = name else {
            return Self::Default;
        };
        if name.eq_ignore_ascii_case("compact") || name.eq_ignore_ascii_case("small") {
            Self::Compact
        } else if name.eq_ignore_ascii_case("strict") {
            Self::Strict
        } else {
            Self::Default
        }
    }

    /// Template label used in prompts.
    #[must_use]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Compact => "compact",
            Self::Strict => "strict",
        }
    }

    /// Default model tier for the template.
    #[must_use]
    pub(crate) const fn default_model_tier(self) -> &'static str {
        match self {
            Self::Default => "focused",
            Self::Compact => "mechanical",
            Self::Strict => "integrative",
        }
    }

    /// Verify strictness guidance for the template.
    #[must_use]
    pub(crate) const fn gate_strictness(self) -> &'static str {
        match self {
            Self::Default => "standard",
            Self::Compact => "standard",
            Self::Strict => "strict",
        }
    }

    /// Maximum total task count the generator should target.
    #[must_use]
    pub(crate) const fn max_task_count(self) -> usize {
        match self {
            Self::Default => 20,
            Self::Compact => 12,
            Self::Strict => 8,
        }
    }
}

/// Render the selected plan template as prompt guidance.
#[must_use]
pub(crate) fn render_plan_template_guidance(template: PlanTemplateKind) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "## Plan template");
    let _ = writeln!(out, "- name: {}", template.label());
    let _ = writeln!(
        out,
        "- default model tier: {}",
        template.default_model_tier()
    );
    let _ = writeln!(out, "- gate strictness: {}", template.gate_strictness());
    let _ = writeln!(out, "- max task count: {}", template.max_task_count());
    let _ = writeln!(
        out,
        "- Keep the plan within this budget unless the PRD explicitly requires more tasks."
    );
    out
}

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
5. **Model hints**: Assign the cheapest model that can handle each task. Imports → `claude-haiku-4-5`. Single function → `claude-sonnet-4-6`. Multi-module wiring → `claude-opus-4-6`.

## Task tiers

| Tier | Name | Max LOC | Model | Examples |
|------|------|---------|-------|----------|
| 0 | Mechanical | 20 | `claude-haiku-4-5` | Add import, add struct field, rename function |
| 1 | Focused | 50 | `claude-sonnet-4-6` | Implement function body, write single test |
| 2 | Integrative | 150 | `claude-sonnet-4-6` | Wire module A→B, implement trait for type |
| 3 | Architectural | 300 | `claude-opus-4-6` | Design new API, decompose complex feature |

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
description = "<short outcome description>"
status = "ready"
tier = "mechanical"       # mechanical | focused | integrative | architectural
model_hint = "claude-haiku-4-5"  # FULL model name required: claude-haiku-4-5 | claude-sonnet-4-6 | claude-opus-4-6
max_loc = 20              # maximum lines of change
files = ["<path>"]        # files this task modifies
allowed_tools = ["read_file", "grep"]
denied_tools = []
mcp_servers = ["filesystem"] # MCP servers this task needs
depends_on = []
role = "implementer"      # REQUIRED: implementer | architect | researcher | strategist | quick-reviewer | scribe

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

## Role selection

Every `[[task]]` MUST include a `role` field. Choose the most specific role:

| Role | Use when |
|------|----------|
| `"implementer"` | Writing code, adding fields, modifying functions, creating files |
| `"architect"` | Designing APIs, planning module structure, major refactors |
| `"researcher"` | Gathering information, analyzing existing code, reading docs |
| `"strategist"` | Decomposing requirements, planning approach, making design decisions |
| `"scribe"` | Writing documentation, updating comments, generating markdown |
| `"quick-reviewer"` | Code review tasks, auditing for correctness |

Missing or misspelled roles will be rejected by `roko plan validate`. The `role` field is REQUIRED.

## Model names

Use FULL model identifiers in the `model_hint` and `model` fields. Never use short aliases.

| Alias (WRONG) | Full name (CORRECT) |
|---------------|---------------------|
| `"haiku"` | `"claude-haiku-4-5"` |
| `"sonnet"` | `"claude-sonnet-4-6"` |
| `"opus"` | `"claude-opus-4-6"` |

Using aliases like `"sonnet"` will cause `PLAN_009` warnings in `roko plan validate` and may fail at execution.

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

/// Build the shared system prompt for plan generation and regeneration.
#[must_use]
pub fn build_generator_system_prompt(workdir: &Path) -> String {
    let mut prompt = String::new();
    let _ = writeln!(prompt, "{PLAN_GENERATOR_SYSTEM_PROMPT}");
    append_naming_glossary_prompt(&mut prompt, workdir);
    append_claude_md_prompt(&mut prompt, workdir);
    prompt
}

/// Build the full prompt for plan generation from a source input.
#[must_use]
pub fn build_generation_prompt(workdir: &Path, source: &str, source_type: &str) -> String {
    build_generation_prompt_with_context(workdir, source, source_type, None)
}

/// Build the full prompt for plan generation from a source input and optional
/// repository context.
#[must_use]
pub fn build_generation_prompt_with_context(
    workdir: &Path,
    source: &str,
    source_type: &str,
    repo_context: Option<&RepoContextPack>,
) -> String {
    let mut prompt = build_generator_system_prompt(workdir);
    let _ = writeln!(prompt, "\n---\n");
    let _ = writeln!(prompt, "## Workspace: {}\n", workdir.display());
    let _ = writeln!(
        prompt,
        "## Source type: {source_type}\n\n## Source content:\n\n{source}"
    );
    append_repo_context_prompt(&mut prompt, repo_context);
    prompt
}

/// Build a prompt for regenerating an existing plan in place (§11).
///
/// Strips the existing tasks to just `id`/`title`/`depends_on` and asks the
/// agent to fill in `tier`, `model_hint`, `read_files`, `verify`, `context`,
/// and `max_loc`.
#[must_use]
pub fn build_regeneration_prompt(workdir: &Path, existing_tasks_toml: &str) -> String {
    build_regeneration_prompt_with_context(workdir, existing_tasks_toml, None, None)
}

/// Build a prompt for regenerating an existing plan in place (§11) with
/// optional validation feedback and repository context.
#[must_use]
pub fn build_regeneration_prompt_with_context(
    workdir: &Path,
    existing_tasks_toml: &str,
    validation_errors: Option<&str>,
    repo_context: Option<&RepoContextPack>,
) -> String {
    let mut prompt = build_generator_system_prompt(workdir);
    let _ = writeln!(prompt, "\n---\n");
    let _ = writeln!(prompt, "## Workspace: {}\n", workdir.display());
    let _ = writeln!(prompt, "## Task: Regenerate plan\n");
    let _ = writeln!(
        prompt,
        "The following tasks.toml exists but is missing full metadata (description, tier, model_hint, \
         read_files, verify, context, max_loc, mcp_servers). Your job is to read the codebase and fill in \
         every field for each task. Keep the existing id, title, description, and depends_on. Add:\n\
         - `tier` (mechanical/focused/integrative/architectural)\n\
         - `model_hint` (the cheapest model for that tier)\n\
         - `max_loc` (estimated lines of change)\n\
         - `allowed_tools`, `denied_tools`, and `mcp_servers` (per-task tool/MCP constraints)\n\
         - `[task.context]` with read_files, symbols, anti_patterns\n\
         - `[[task.verify]]` with at least compile + test checks\n\n\
         ## Existing tasks.toml:\n\n```toml\n{existing_tasks_toml}\n```"
    );
    if let Some(validation_errors) = validation_errors.filter(|errors| !errors.trim().is_empty()) {
        let _ = writeln!(
            prompt,
            "\n## Previous Plan Validation Errors\nThe previous plan had the following validation issues that MUST be fixed:\n\n{}\n\nFix ALL errors. Warnings should be addressed if possible.",
            validation_errors
        );
    }
    append_repo_context_prompt(&mut prompt, repo_context);
    prompt
}

fn append_repo_context_prompt(prompt: &mut String, repo_context: Option<&RepoContextPack>) {
    let Some(repo_context) = repo_context else {
        return;
    };

    let _ = writeln!(prompt, "\n---\n\n{}", repo_context.to_prompt_section());
}

#[cfg(test)]
mod template_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn build_generator_system_prompt_includes_naming_glossary_excerpt_when_present() {
        let temp = tempfile::tempdir().expect("tempdir");
        let glossary_dir = temp.path().join("docs").join("00-architecture");
        std::fs::create_dir_all(&glossary_dir).expect("create glossary dir");
        std::fs::write(
            glossary_dir.join("01-naming-and-glossary.md"),
            "# Naming Map\n\nSignal -> Engram\n",
        )
        .expect("write glossary");

        let prompt = build_generator_system_prompt(temp.path());
        assert!(prompt.contains("## Naming glossary"));
        assert!(prompt.contains("Signal -> Engram"));
    }

    #[test]
    fn build_generator_system_prompt_includes_claude_rules_when_present() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("CLAUDE.md"),
            "# Rules\n\nNEVER reimplement what already exists.\n",
        )
        .expect("write claude");

        let prompt = build_generator_system_prompt(temp.path());

        assert!(prompt.contains("## Workspace rules"));
        assert!(prompt.contains("NEVER reimplement what already exists."));
    }

    #[test]
    fn build_generation_prompt_with_context_includes_repo_context() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_context = RepoContextPack {
            root: temp.path().to_path_buf(),
            project_kind: crate::repo_context::ProjectKind::Rust,
            workspace_members: vec!["roko-compose".to_string()],
            key_files: vec![PathBuf::from("crates/roko-compose/src/lib.rs")],
            matching_symbols: Vec::new(),
            related_prds: Vec::new(),
            related_plans: Vec::new(),
            do_not_create: vec!["roko-compose".to_string()],
            keywords: vec!["compose".to_string()],
            context_root_verified: true,
        };

        let prompt = build_generation_prompt_with_context(
            temp.path(),
            "Compose a grounded plan",
            "prompt",
            Some(&repo_context),
        );

        assert!(prompt.contains("Compose a grounded plan"));
        assert!(prompt.contains("## Repository Context"));
        assert!(prompt.contains("roko-compose"));
    }

    #[test]
    fn build_regeneration_prompt_with_context_includes_validation_and_context() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_context = RepoContextPack {
            root: temp.path().to_path_buf(),
            project_kind: crate::repo_context::ProjectKind::Rust,
            workspace_members: vec!["roko-core".to_string()],
            key_files: vec![PathBuf::from("crates/roko-core/src/lib.rs")],
            matching_symbols: Vec::new(),
            related_prds: Vec::new(),
            related_plans: Vec::new(),
            do_not_create: vec!["roko-core".to_string()],
            keywords: vec!["core".to_string()],
            context_root_verified: true,
        };

        let prompt = build_regeneration_prompt_with_context(
            temp.path(),
            "[[task]]\nid = \"T1\"\n",
            Some("- **ERROR** [PLAN_003]: task 'T1' is missing required field 'role'"),
            Some(&repo_context),
        );

        assert!(prompt.contains("Previous Plan Validation Errors"));
        assert!(prompt.contains("PLAN_003"));
        assert!(prompt.contains("## Repository Context"));
    }

    #[test]
    fn resolves_missing_template_to_default() {
        let template = PlanTemplateKind::resolve(None);
        assert_eq!(template.label(), "default");
        assert_eq!(template.default_model_tier(), "focused");
        assert_eq!(template.gate_strictness(), "standard");
        assert_eq!(template.max_task_count(), 20);
    }

    #[test]
    fn resolves_strict_template() {
        let template = PlanTemplateKind::resolve(Some("strict"));
        assert_eq!(template.label(), "strict");
        assert_eq!(template.default_model_tier(), "integrative");
        assert_eq!(template.gate_strictness(), "strict");
        assert_eq!(template.max_task_count(), 8);
    }

    #[test]
    fn template_guidance_includes_selected_settings() {
        let guidance = render_plan_template_guidance(PlanTemplateKind::Compact);
        assert!(guidance.contains("name: compact"));
        assert!(guidance.contains("default model tier: mechanical"));
        assert!(guidance.contains("gate strictness: standard"));
        assert!(guidance.contains("max task count: 12"));
    }
}

fn append_naming_glossary_prompt(prompt: &mut String, workdir: &Path) {
    let glossary_path = workdir.join(NAMING_GLOSSARY_RELATIVE_PATH);
    let Ok(glossary) = std::fs::read_to_string(&glossary_path) else {
        return;
    };

    let excerpt = glossary
        .lines()
        .take(NAMING_GLOSSARY_MAX_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    if excerpt.trim().is_empty() {
        return;
    }

    let _ = writeln!(
        prompt,
        "\n## Naming glossary\nUse the canonical names and renames below when generating plans. This excerpt comes from `{}`.\n\n```md\n{}\n```",
        NAMING_GLOSSARY_RELATIVE_PATH, excerpt
    );
}

fn append_claude_md_prompt(prompt: &mut String, workdir: &Path) {
    let claude_path = workdir.join(CLAUDE_MD_RELATIVE_PATH);
    let Ok(claude_md) = std::fs::read_to_string(&claude_path) else {
        return;
    };

    let excerpt = claude_md
        .lines()
        .take(CLAUDE_MD_MAX_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    if excerpt.trim().is_empty() {
        return;
    }

    let _ = writeln!(
        prompt,
        "\n## Workspace rules\nFollow the project-specific operating rules below from `{}` when generating plans.\n\n```md\n{}\n```",
        CLAUDE_MD_RELATIVE_PATH, excerpt
    );
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

    #[test]
    fn build_generator_system_prompt_uses_full_model_names() {
        let prompt = build_generator_system_prompt(std::path::Path::new("/test"));

        assert!(prompt.contains("## Model names"));
        assert!(prompt.contains("| 0 | Mechanical | 20 | `claude-haiku-4-5` |"));
        assert!(prompt.contains("| 1 | Focused | 50 | `claude-sonnet-4-6` |"));
        assert!(prompt.contains("| 2 | Integrative | 150 | `claude-sonnet-4-6` |"));
        assert!(prompt.contains("| 3 | Architectural | 300 | `claude-opus-4-6` |"));
        assert!(prompt.contains("model_hint = \"claude-haiku-4-5\""));
        assert!(!prompt.contains("| 0 | Mechanical | 20 | haiku |"));
    }
}
