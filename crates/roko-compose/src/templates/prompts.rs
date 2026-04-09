//! Prompt templates for enrichment steps.
//!
//! Ported from `apps/mori/src/support_enrich/prompts.rs`. Each step has a
//! system prompt constant and a function that builds the user message from
//! input file contents.
//!
//! Anti-pattern #8: **no `std::fs` in this module**. All file content arrives
//! via function parameters.
#![allow(clippy::needless_raw_string_hashes, clippy::format_push_string)]

/// Maximum characters for plan content in prompts.
pub const PLAN_BUDGET: usize = 30_000;
/// Maximum characters for supporting document content.
pub const SUPPORT_BUDGET: usize = 8_000;

/// Truncate content to a character budget, appending a note if truncated.
fn truncate_to_budget(content: &str, budget: usize) -> String {
    if content.len() <= budget {
        content.to_string()
    } else {
        let end = budget.saturating_sub(40);
        let end = if end >= content.len() {
            content.len()
        } else if content.is_char_boundary(end) {
            end
        } else {
            (0..end)
                .rev()
                .find(|idx| content.is_char_boundary(*idx))
                .unwrap_or(0)
        };
        format!(
            "{}\n\n[... truncated at {}/{} chars]",
            &content[..end],
            end,
            content.len()
        )
    }
}

// ── PRD Extract ────────────────────────────────────────────────────

/// System prompt for PRD context extraction.
pub const PRD_SYSTEM: &str = r#"You are a context engineer extracting relevant specification sections for an implementation plan.

Given a plan that references specification documents (PRD, RFC, design docs), extract the sections most relevant to implementation. For each referenced document:

1. Include the section title and path.
2. Extract the specific paragraphs, requirements, or constraints referenced.
3. Truncate long sections but preserve structure.
4. Note any cross-references to other specification sections.

Output format:
```markdown
# PRD Context for Plan {name}

## {source_path}

<prd-file path="{path}">
{extracted content}
[... truncated at N/M chars]
</prd-file>
```

Budget: aim for 8000-15000 characters total. Prioritize sections directly referenced by the plan over tangentially related content."#;

/// Build the user message for PRD extraction.
pub fn prd_user(plan_content: &str, prd_sections: &[(&str, &str)]) -> String {
    let plan = truncate_to_budget(plan_content, PLAN_BUDGET);
    let mut msg = format!(
        "Extract relevant PRD context for this plan:\n\n{plan}\n\n---\n\nAvailable PRD sections:\n"
    );
    for (path, content) in prd_sections {
        let c = truncate_to_budget(content, SUPPORT_BUDGET);
        msg.push_str(&format!("\n## {path}\n\n{c}\n"));
    }
    msg
}

// ── Briefs ─────────────────────────────────────────────────────────

/// System prompt for brief generation.
pub const BRIEF_SYSTEM: &str = r#"You are a technical writer creating implementation briefs for software development plans.

Given a plan document, produce a concise brief with these sections:

## Artifact Pointers
Table mapping artifact names to their file paths within the plan directory.

## Authority Chain
Which documents take precedence (plan > PRD > brief for implementation details).

## Dependencies
What must exist before this plan can be implemented. List prerequisite plans, modules, or packages.

## Imports
Types, traits, or interfaces imported from other plans or modules.

## Task Map
A table of implementation units with their files and acceptance criteria.

## Key Types and Interfaces
The most important types, traits, or interfaces this plan introduces or modifies.

## Verification Checklist
What must pass for this plan to be considered complete (compile, test, lint, review).

## Conflict Scan
Note any files that overlap with other plans or modules.

## Execution Order
Summarize the recommended implementation order from the decomposition (if available).

Keep the brief under 3000 words. Focus on what an implementer needs to start coding."#;

/// Build the user message for brief generation.
pub fn brief_user(plan_content: &str, decomposition: Option<&str>) -> String {
    let plan = truncate_to_budget(plan_content, PLAN_BUDGET);
    let mut msg = format!("Generate an implementation brief for this plan:\n\n{plan}");
    if let Some(decomp) = decomposition {
        let d = truncate_to_budget(decomp, SUPPORT_BUDGET);
        msg.push_str(&format!(
            "\n\n---\n\nDecomposition (for execution order):\n\n{d}"
        ));
    }
    msg
}

// ── Tasks ──────────────────────────────────────────────────────────

/// System prompt for task TOML generation.
pub const TASKS_SYSTEM: &str = r#"You are a build planner that converts implementation plans into structured task files.

Given a plan document, produce a TOML file with this exact format:

```toml
[meta]
plan = "plan-name"
iteration = 1
total = N
done = 0
max_parallel = N
estimated_total_minutes = N

[[task]]
id = "T1"
title = "descriptive title"
status = "pending"
files = ["path/to/file.ext"]
acceptance = ["criterion 1", "criterion 2"]
allowed_tools = ["read_file", "grep"]
denied_tools = []
mcp_servers = ["filesystem", "git"]
depends_on = []
parallel_group = "A"
exclusive_files = true
estimated_seconds = 600
```

Rules:
- Extract implementation units from ## headings in the plan.
- Each task maps to a logical unit of work with specific files.
- Acceptance criteria come from the plan's requirements, checkpoints, and verification steps.
- Use `mcp_servers` to list the MCP server names a task needs before it runs.
- Group independent tasks into parallel groups (A, B, C...).
- Tasks that modify the same files must NOT be in the same parallel group.
- Estimate seconds conservatively (300-1800 per task).
- Output ONLY the TOML content, no markdown fences or explanation."#;

/// Build the user message for task generation.
pub fn tasks_user(plan_content: &str) -> String {
    let plan = truncate_to_budget(plan_content, PLAN_BUDGET);
    format!("Generate a tasks.toml for this plan:\n\n{plan}")
}

// ── Decompose ──────────────────────────────────────────────────────

/// System prompt for task decomposition.
pub const DECOMPOSE_SYSTEM: &str = r#"You are a senior engineer creating step-by-step implementation instructions from a plan.

Produce a markdown decomposition with:
- A preamble summarizing constraints and key decisions.
- Numbered steps, each with:
  - **Files:** which files to create or modify
  - **Creates/Modifies:** what the step produces
  - **Imports:** what the step depends on from earlier steps
  - **Action:** numbered sub-steps with exact instructions
  - **Checkpoint:** a concrete command to verify the step succeeded

Format:
```markdown
# Decomposition: Plan {name}

## Preamble
{constraints, key decisions, existing state notes}

---

## Step 1: {title}
**Files:** `path/to/file`
**Creates:** {what this step produces}
**Imports:** {dependencies from earlier steps or external}
**Action:**
1. {specific instruction}
2. {specific instruction}

### Checkpoint after Step 1
```bash
{verification command}
# Expected: {expected output}
```
```

Rules:
- Steps must be ordered so each depends only on prior steps.
- Checkpoints must be concrete, runnable commands.
- Include both creation and modification instructions.
- Note when existing files need merging rather than overwriting.
- Be specific about types, function signatures, and field names when the plan defines them."#;

/// Build the user message for decomposition.
pub fn decompose_user(plan_content: &str, brief: Option<&str>) -> String {
    let plan = truncate_to_budget(plan_content, PLAN_BUDGET);
    let mut msg = format!("Create a step-by-step decomposition for this plan:\n\n{plan}");
    if let Some(b) = brief {
        let br = truncate_to_budget(b, SUPPORT_BUDGET);
        msg.push_str(&format!("\n\n---\n\nBrief (for context):\n\n{br}"));
    }
    msg
}

// ── Research ───────────────────────────────────────────────────────

/// System prompt for research memo generation.
pub const RESEARCH_SYSTEM: &str = r#"You are a context engineer creating a narrow research memo for a software implementation plan.

Prefer concrete file paths, existing repo patterns, likely risks, and the smallest useful verification checklist. Keep the output dense and operational."#;

/// Build the user message for research memo generation.
pub fn research_user(
    plan_content: &str,
    tasks: Option<&str>,
    brief: Option<&str>,
    decomposition: Option<&str>,
    verify: Option<&str>,
    review: Option<&str>,
) -> String {
    let mut msg = format!(
        "Generate a dense research memo for this plan:\n\n{}",
        truncate_to_budget(plan_content, PLAN_BUDGET)
    );
    for (label, value) in [
        ("Tasks", tasks),
        ("Brief", brief),
        ("Decomposition", decomposition),
        ("Verify tasks", verify),
        ("Review tasks", review),
    ] {
        if let Some(value) = value {
            msg.push_str(&format!(
                "\n\n---\n\n{label}:\n\n{}",
                truncate_to_budget(value, SUPPORT_BUDGET)
            ));
        }
    }
    msg
}

// ── Dependencies ───────────────────────────────────────────────────

/// System prompt for dependency manifest generation.
pub const DEPENDENCIES_SYSTEM: &str = r#"You are a verification planner extracting machine-readable dependency manifests from a plan.

Return TOML only with `[[dependency]]` entries. Focus on reusable dependencies, mocks, fixtures, and downstream impact."#;

/// Build the user message for dependency manifest generation.
pub fn dependencies_user(plan_content: &str, tasks: Option<&str>, brief: Option<&str>) -> String {
    let mut msg = format!(
        "Generate a dependency manifest for this plan:\n\n{}",
        truncate_to_budget(plan_content, PLAN_BUDGET)
    );
    if let Some(tasks) = tasks {
        msg.push_str(&format!(
            "\n\n---\n\nTasks:\n\n{}",
            truncate_to_budget(tasks, SUPPORT_BUDGET)
        ));
    }
    if let Some(brief) = brief {
        msg.push_str(&format!(
            "\n\n---\n\nBrief:\n\n{}",
            truncate_to_budget(brief, SUPPORT_BUDGET)
        ));
    }
    msg
}

// ── Fixtures ───────────────────────────────────────────────────────

/// System prompt for fixture manifest generation.
pub const FIXTURES_SYSTEM: &str = r#"You are a local-test infrastructure planner.

Return TOML only with `[[fixture]]` entries describing reusable local fixtures, sidecars, commands, and healthchecks."#;

/// Build the user message for fixture manifest generation.
pub fn fixtures_user(
    plan_content: &str,
    tasks: Option<&str>,
    dependency_manifest: Option<&str>,
    research: Option<&str>,
) -> String {
    let mut msg = format!(
        "Generate a fixture manifest for this plan:\n\n{}",
        truncate_to_budget(plan_content, PLAN_BUDGET)
    );
    for (label, value) in [
        ("Tasks", tasks),
        ("Dependency manifest", dependency_manifest),
        ("Research", research),
    ] {
        if let Some(value) = value {
            msg.push_str(&format!(
                "\n\n---\n\n{label}:\n\n{}",
                truncate_to_budget(value, SUPPORT_BUDGET)
            ));
        }
    }
    msg
}

// ── Integration ────────────────────────────────────────────────────

/// System prompt for integration guidance generation.
pub const INTEGRATION_SYSTEM: &str = r#"You are an integration-test planner.

Return markdown only. Focus on executable local integration/test flows, fixtures to start, likely commands, and the narrowest realistic surfaces to validate."#;

/// Build the user message for integration guidance generation.
pub fn integration_user(
    plan_content: &str,
    tasks: Option<&str>,
    verify: Option<&str>,
    review: Option<&str>,
    research: Option<&str>,
    dependency_manifest: Option<&str>,
    fixture_manifest: Option<&str>,
) -> String {
    let mut msg = format!(
        "Generate an integration plan for this plan:\n\n{}",
        truncate_to_budget(plan_content, PLAN_BUDGET)
    );
    for (label, value) in [
        ("Tasks", tasks),
        ("Verify tasks", verify),
        ("Review tasks", review),
        ("Research", research),
        ("Dependency manifest", dependency_manifest),
        ("Fixture manifest", fixture_manifest),
    ] {
        if let Some(value) = value {
            msg.push_str(&format!(
                "\n\n---\n\n{label}:\n\n{}",
                truncate_to_budget(value, SUPPORT_BUDGET)
            ));
        }
    }
    msg
}

// ── Verify ─────────────────────────────────────────────────────────

/// System prompt for verification task TOML generation.
pub const VERIFY_SYSTEM: &str = r#"You are a CI/CD engineer creating verification task files for software plans.

Given a plan and its tasks, produce a TOML file of verification steps:

```toml
[meta]
plan = "plan-name"
role = "verifier"
total = N

# COMPILE GATES - blocking, must pass first
[[task]]
id = "CG1"
title = "Workspace Compilation"
type = "compile"
command = "cargo check --workspace"
blocking = true
status = "pending"

# TEST TASKS
[[task]]
id = "VT1"
title = "Unit Tests"
type = "test"
command = "cargo test -p package_name"
blocking = false
status = "pending"

# LINT TASKS
[[task]]
id = "LT1"
title = "Clippy Lint"
type = "lint"
command = "cargo clippy -p package_name -- -D warnings"
blocking = false
status = "pending"
```

Rules:
- Start with compile gates (CG prefix) -- these block everything.
- Add per-package test tasks (VT prefix).
- Add lint checks (LT prefix).
- Add format checks if applicable.
- Commands should be concrete and runnable.
- Detect the project's build system from the plan (cargo, npm, make, etc.).
- Output ONLY the TOML content."#;

/// Build the user message for verify task generation.
pub fn verify_user(plan_content: &str, tasks_content: Option<&str>) -> String {
    let plan = truncate_to_budget(plan_content, PLAN_BUDGET);
    let mut msg = format!("Generate verification tasks for this plan:\n\n{plan}");
    if let Some(tasks) = tasks_content {
        let t = truncate_to_budget(tasks, SUPPORT_BUDGET);
        msg.push_str(&format!("\n\n---\n\nExisting tasks.toml:\n\n{t}"));
    }
    msg
}

// ── Reviews ────────────────────────────────────────────────────────

/// System prompt for review task TOML generation.
pub const REVIEW_SYSTEM: &str = r#"You are a code review architect creating review checklists for implementation plans.

Given a plan, produce a TOML file of review tasks:

```toml
[meta]
plan = "plan-name"
role = "reviewer"
review_type = "architect+auditor"
total = N

# BLOCKING GATES
[[task]]
id = "R1"
title = "Compilation Gate"
type = "gate"
severity = "blocking"
check = ["specific thing to verify"]
files = ["files to inspect"]
verdict = "pending"
notes = ""

# INVARIANTS
[[task]]
id = "R3"
title = "Invariant Name"
type = "invariant"
severity = "blocking"
check = ["invariant condition to verify"]
files = ["relevant files"]
verdict = "pending"
notes = ""

# CONTRACT CHECKS
[[task]]
id = "R10"
title = "API Contract"
type = "contract"
severity = "blocking"
check = ["exported API matches plan spec"]
files = ["public API files"]
verdict = "pending"
notes = ""

# ACCEPTANCE CHECKS (non-blocking)
[[task]]
id = "R20"
title = "Documentation"
type = "acceptance"
severity = "non-blocking"
check = ["docs exist for public items"]
files = []
verdict = "pending"
notes = ""
```

Rules:
- Extract invariants (INV- blocks) from the plan.
- Extract contract requirements (exports, public API shapes).
- Add compilation and test gates.
- Severity is "blocking" for invariants and contracts, "non-blocking" for style/docs.
- Output ONLY the TOML content."#;

/// Build the user message for review task generation.
pub fn review_user(plan_content: &str) -> String {
    let plan = truncate_to_budget(plan_content, PLAN_BUDGET);
    format!("Generate review tasks for this plan:\n\n{plan}")
}

// ── Tests ──────────────────────────────────────────────────────────

/// System prompt for test suggestion generation.
pub const TESTS_SYSTEM: &str = r#"You are a test engineer creating a testing backlog for an implementation plan.

Produce a markdown document listing test suggestions organized by task. Include:

## Task {id} -- {title}
- [ ] {test description}: what to test, expected behavior, edge cases
- [ ] {another test}

Categories of tests to consider:
- Unit tests for individual functions/methods
- Integration tests across modules
- Property-based tests for invariants
- Edge cases and error paths
- Regression anchors for known tricky behavior

For each test, note whether it is:
- **blocking**: must pass for the plan to be complete
- **non-blocking**: nice to have, can be deferred

Include concrete code examples where helpful (use the project's language).
Output markdown only."#;

/// Build the user message for test generation.
pub fn tests_user(plan_content: &str, tasks_content: Option<&str>) -> String {
    let plan = truncate_to_budget(plan_content, PLAN_BUDGET);
    let mut msg = format!("Generate a testing backlog for this plan:\n\n{plan}");
    if let Some(tasks) = tasks_content {
        let t = truncate_to_budget(tasks, SUPPORT_BUDGET);
        msg.push_str(&format!("\n\n---\n\nTasks:\n\n{t}"));
    }
    msg
}

// ── Invariants ─────────────────────────────────────────────────────

/// System prompt for invariant/rubric extraction.
pub const INVARIANTS_SYSTEM: &str = r#"You are a quality engineer creating a shared review rubric from an implementation plan.

Extract all invariants, contracts, and acceptance gates into a structured rubric. Format:

```markdown
## Shared Review Rubric -- Plan {name}

Single source of truth for implementer self-check and reviewer blocking scope.

| Role | Rule |
|------|------|
| Implementer | Confirm every box before declaring the plan done. |
| Reviewer | Raise blocking issues only when they violate an item below. |

## Blocking checklist (N items)

### Compilation
- [ ] {build command} completes with zero errors.

### Exports contract
- [ ] Every path listed in the plan Exports section exists on disk.

### Invariants
- [ ] INV-{N}: {invariant description}; oracle: {how to verify}

### Quick Reference fidelity
- [ ] If the plan defines public API shapes, the implementation matches verbatim.

### Verification chain
- [ ] {verification script} exits 0.
```

Rules:
- Extract INV- blocks from the plan.
- Extract export contracts and public API requirements.
- Include concrete verification commands.
- Distinguish blocking from non-blocking items.
- Output markdown only."#;

/// Build the user message for invariant extraction.
pub fn invariants_user(plan_content: &str) -> String {
    let plan = truncate_to_budget(plan_content, PLAN_BUDGET);
    format!("Extract invariants and build a review rubric for this plan:\n\n{plan}")
}

// ── Scribe ─────────────────────────────────────────────────────────

/// System prompt for scribe (documentation) task TOML generation.
pub const SCRIBE_SYSTEM: &str = r#"You are a documentation planner creating documentation task files for implementation plans.

Given a plan, produce a TOML file of documentation tasks:

```toml
[meta]
plan = "plan-name"
role = "scribe"
total = N

[[task]]
id = "D1"
type = "module_doc"
title = "Document {module}: {description}"
output_file = "path/to/doc.md"
sections = ["context", "architecture", "api", "examples", "testing"]

[[task]]
id = "D10"
type = "api_doc"
title = "API reference for {module}"
output_file = "path/to/api-doc.md"
sections = ["types", "functions", "traits", "examples"]

[[task]]
id = "D20"
type = "guide"
title = "Getting started with {feature}"
output_file = "path/to/guide.md"
sections = ["overview", "setup", "usage", "troubleshooting"]
```

Rules:
- One module_doc task per module/crate/package touched by the plan.
- Add api_doc tasks for public interfaces.
- Add guide tasks for user-facing features.
- Output paths should be relative to the plan directory.
- Output ONLY the TOML content."#;

/// Build the user message for scribe task generation.
pub fn scribe_user(plan_content: &str) -> String {
    let plan = truncate_to_budget(plan_content, PLAN_BUDGET);
    format!("Generate documentation tasks for this plan:\n\n{plan}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_within_budget_returns_original() {
        let content = "short content";
        assert_eq!(truncate_to_budget(content, 100), content);
    }

    #[test]
    fn truncate_over_budget_adds_note() {
        let content = "a".repeat(200);
        let result = truncate_to_budget(&content, 100);
        assert!(result.contains("[... truncated at"));
        assert!(result.len() < 200);
    }
}
