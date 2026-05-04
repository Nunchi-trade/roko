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
5. **Model hints**: NEVER set `model_hint`. The runtime selects the right model based on the task `tier`. Hardcoded model names break across providers.

## Task tiers

| Tier | Name | Max LOC | Examples |
|------|------|---------|----------|
| 0 | Mechanical | 20 | Add import, add struct field, rename function |
| 1 | Focused | 50 | Implement function body, write single test |
| 2 | Integrative | 150 | Wire module A→B, implement trait for type |
| 3 | Architectural | 300 | Design new API, decompose complex feature |

## Output format

Create plan directories with these files:

### tasks.toml
```toml
[meta]
plan = "add-funding-rate"  # MUST match the PRD slug exactly
total = 3
done = 0
status = "ready"
max_parallel = 1  # default to 1 for safety; only increase when tasks are truly independent

[[task]]
id = "T1"
title = "Add FundingRate struct to core types"
description = "Define the FundingRate data structure in roko-core for storing funding rate observations."
status = "ready"
tier = "mechanical"       # mechanical | focused | integrative | architectural
# model_hint omitted — runtime picks the best model automatically
max_loc = 20              # maximum lines of change
files = ["crates/roko-core/src/types.rs"]   # REAL file paths only, never <path> or <crate>
allowed_tools = ["read_file", "grep"]
denied_tools = []
mcp_servers = ["filesystem"] # MCP servers this task needs
depends_on = []
role = "implementer"      # REQUIRED: implementer | architect | researcher | strategist | quick-reviewer | scribe

# SURGICAL CONTEXT: exactly what the agent needs to read
[task.context]
read_files = [
    { path = "crates/roko-core/src/types.rs", lines = "1-50", why = "Find existing type definitions to follow naming conventions." },
]
symbols = [
    "Signal — existing base type to reference",
]
anti_patterns = [
    "Do NOT create new files. Modify crates/roko-core/src/types.rs only.",
]

# EXECUTABLE VERIFICATION
[[task.verify]]
phase = "structural"
command = "grep -q 'pub struct FundingRate' crates/roko-core/src/types.rs"
fail_msg = "FundingRate struct not found"

[[task.verify]]
phase = "compile"
command = "cargo check -p roko-core"

[[task.verify]]
phase = "test"
command = "cargo test -p roko-core"

[[task]]
id = "T2"
title = "Wire FundingRate display into CLI status output"
description = "Import FundingRate from roko-core and add it to the status command output."
status = "ready"
tier = "focused"
# model_hint omitted — runtime selects automatically
max_loc = 40
files = ["crates/roko-cli/src/commands/status.rs"]
allowed_tools = ["read_file", "grep", "write_file"]
denied_tools = []
mcp_servers = ["filesystem"]
depends_on = ["T1"]
role = "implementer"

[task.context]
read_files = [
    { path = "crates/roko-cli/src/commands/status.rs", lines = "1-80", why = "Understand current status output format." },
    { path = "crates/roko-core/src/types.rs", lines = "1-30", why = "Import the new FundingRate type." },
]
symbols = [
    "StatusOutput — struct that collects status display fields",
]
anti_patterns = [
    "Do NOT modify roko-core. Only change the CLI crate.",
]

[[task.verify]]
phase = "structural"
command = "grep -q 'FundingRate' crates/roko-cli/src/commands/status.rs"
fail_msg = "FundingRate not referenced in status command"
[[task.verify]]
phase = "compile"
command = "cargo check -p roko-cli"
[[task.verify]]
phase = "test"
command = "cargo test -p roko-cli"
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

## Model hints

**NEVER set `model_hint`.** The runtime's model-selection chain (cascade router, project config, budget pressure) picks the right model automatically. Setting model_hint hardcodes a provider-specific model name that breaks when users run non-Claude providers.

Always omit the `model_hint` field entirely. The task `tier` field (mechanical/focused/integrative/architectural) already tells the runtime what capability level is needed.

## Before generating tasks, you MUST:

1. Search the codebase to understand what exists:
   `grep -rn 'TypeName' crates/ --include='*.rs' | grep -v target/ | head -20`

2. Read the specific files you're generating tasks for — understand the current code.

3. Check if the feature already exists (partially or fully):
   `grep -rn 'feature_keyword' crates/ --include='*.rs' | grep -v target/`

4. For each task, verify the context files actually exist:
   `test -f crates/roko-core/src/types.rs && echo "exists" || echo "MISSING"`

## Language detection

Detect the project language and use the right commands:
- Cargo.toml → Rust: `cargo check`, `cargo test`, `cargo clippy`
- package.json → TypeScript: `npx tsc`, `npx jest`, `npx eslint`
- go.mod → Go: `go build`, `go test`, `golangci-lint`
- pyproject.toml/setup.py → Python: `python -m py_compile`, `pytest`, `ruff`

## Verify steps by role

- **implementer/architect**: MUST have at least 1 structural check + 1 compile check (e.g. `cargo check`)
- **researcher/strategist**: MUST have only structural checks (e.g. `test -f path/to/output.md`, `grep -q ...`). Do NOT add compile/test verify steps — researcher tasks do not modify code.
- **scribe/quick-reviewer**: structural checks only (verify docs exist, verify reviewed files haven't changed)

## Quality gates for YOUR output

Before finalizing, verify your tasks against:
- [ ] `meta.plan` matches the PRD slug exactly (e.g. slug "add-funding-rate" → `plan = "add-funding-rate"`)
- [ ] `meta.max_parallel` is 1 unless tasks are truly independent (shared files = not independent)
- [ ] Every task has ≤ max_loc lines of change for its tier
- [ ] Implementer/architect tasks have at least 1 structural + 1 compile verify step
- [ ] Researcher/strategist tasks have ONLY structural verify steps (no cargo check, no cargo test)
- [ ] No task requires reading more than 3 files
- [ ] Anti-patterns are specific (not generic "be careful")
- [ ] Dependencies form a DAG (no cycles)
- [ ] `model_hint` is NEVER set — runtime selects models from `tier`

CRITICAL RULES for `files` field:
- Use CONCRETE file paths: `"crates/my-crate/src/lib.rs"` NOT `"crates/"` or `"crates/*/src/*.rs"`
- Never use bare directory references like `"crates/"` or `"src/"`
- Never use glob patterns like `*` in file paths
- If a task creates a NEW crate, list the specific files: `"crates/new-crate/src/lib.rs"`, `"crates/new-crate/Cargo.toml"`
- Researcher tasks that only READ files should still list specific file paths they will inspect

Use CONCRETE file paths and crate names from the repository context below.
Never output angle-bracket placeholders like <path>, <crate>, <file>, <module>, or <relevant-lib>.
Every `files` entry, every `path` in `read_files`, and every `cargo` command must reference
actual files and crates that exist in the workspace or that the plan explicitly creates.
If the PRD describes a new crate to create, use the PRD's slug as the crate name
(e.g., for slug "btc-funding-alert", use "crates/btc-funding-alert/src/lib.rs").
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
    let mut prompt = build_generator_system_prompt(workdir);
    let _ = writeln!(prompt, "\n---\n");
    let _ = writeln!(prompt, "## Workspace: {}\n", workdir.display());
    let _ = writeln!(
        prompt,
        "## Source type: {source_type}\n\n## Source content:\n\n{source}"
    );
    prompt
}

#[cfg(test)]
mod template_tests {
    use super::*;

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

/// Build a prompt for regenerating an existing plan in place (§11).
///
/// Strips the existing tasks to just `id`/`title`/`depends_on` and asks the
/// agent to fill in `tier`, `model_hint`, `read_files`, `verify`, `context`,
/// and `max_loc`.
#[must_use]
pub fn build_regeneration_prompt(workdir: &Path, existing_tasks_toml: &str) -> String {
    let mut prompt = build_generator_system_prompt(workdir);
    let _ = writeln!(prompt, "\n---\n");
    let _ = writeln!(prompt, "## Workspace: {}\n", workdir.display());
    let _ = writeln!(prompt, "## Task: Regenerate plan\n");
    let _ = writeln!(
        prompt,
        "The following tasks.toml exists but is missing full metadata (description, tier, \
         read_files, verify, context, max_loc, mcp_servers). Your job is to read the codebase and fill in \
         every field for each task. Keep the existing id, title, description, and depends_on. Add:\n\
         - `tier` (mechanical/focused/integrative/architectural)\n\
         - `max_loc` (estimated lines of change)\n\
         - `allowed_tools`, `denied_tools`, and `mcp_servers` (per-task tool/MCP constraints)\n\
         - `[task.context]` with read_files, symbols, anti_patterns\n\
         - `[[task.verify]]` with at least compile + test checks\n\
         Do NOT set `model_hint` — the runtime selects models automatically from the task tier.\n\n\
         ## Existing tasks.toml:\n\n```toml\n{existing_tasks_toml}\n```"
    );
    prompt
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
    fn build_generator_system_prompt_never_suggests_model_names() {
        let prompt = build_generator_system_prompt(std::path::Path::new("/test"));

        assert!(prompt.contains("## Model hints"));
        assert!(prompt.contains("NEVER set `model_hint`"));
        // Must NOT contain hardcoded model names that break non-Claude providers.
        assert!(!prompt.contains("claude-haiku-4-5"));
        assert!(!prompt.contains("claude-sonnet-4-6"));
        assert!(!prompt.contains("claude-opus-4-6"));
        // Tier table is still present.
        assert!(prompt.contains("| 0 | Mechanical | 20 |"));
    }
}
