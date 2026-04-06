//! Prompt dispatch for all 13 enrichment steps.
//!
//! Ported from `apps/mori/src/support_enrich/mod.rs` lines 756-837 and the
//! `apps/mori/src/support_enrich/prompts.rs` module.
//!
//! Every function here takes and returns `String` — no filesystem access.
//! This is anti-pattern #8: I/O at boundary only.

use std::collections::BTreeMap;

use super::step::EnrichStep;

/// Collected input text for a step.
///
/// The pipeline reads files from disk and populates this struct before calling
/// [`build_prompt`]. Prompt builders receive only strings.
///
/// Ported from Mori `StepInputs` (lines 200-212).
pub struct StepInputs {
    /// Full content of `plan.md`.
    pub plan_content: String,
    /// Content of `tasks.toml`, if it exists.
    pub tasks_content: Option<String>,
    /// Content of `brief.md`, if it exists.
    pub brief_content: Option<String>,
    /// Content of `decomposition.md`, if it exists.
    pub decomposition_content: Option<String>,
    /// Content of `verify-tasks.toml`, if it exists.
    pub verify_content: Option<String>,
    /// Content of `review-tasks.toml`, if it exists.
    pub review_content: Option<String>,
    /// Content of `research.md`, if it exists.
    pub research_content: Option<String>,
    /// Content of `dependency-manifest.toml`, if it exists.
    pub dependency_manifest: Option<String>,
    /// Content of `fixture-manifest.toml`, if it exists.
    pub fixture_manifest: Option<String>,
}

/// Build the (system, user) prompt pair for a given step.
///
/// Dispatches to step-specific prompt builders using the Mori prompt templates
/// that already live in `crate::templates::*` (the enrichment prompts module).
///
/// Ported from Mori `build_prompt` (lines 756-837).
pub fn build_prompt(step: EnrichStep, inputs: &StepInputs) -> (String, String) {
    // We re-use the prompt templates from the Mori prompts module, which was
    // already ported into this crate at `crate::templates::prompts`. The
    // templates are constants and pure functions taking string slices.
    use crate::templates::prompts as p;

    match step {
        EnrichStep::Prd => (
            p::PRD_SYSTEM.to_string(),
            p::prd_user(&inputs.plan_content, &[]),
        ),
        EnrichStep::Briefs => (
            p::BRIEF_SYSTEM.to_string(),
            p::brief_user(
                &inputs.plan_content,
                inputs.decomposition_content.as_deref(),
            ),
        ),
        EnrichStep::Tasks => (
            p::TASKS_SYSTEM.to_string(),
            p::tasks_user(&inputs.plan_content),
        ),
        EnrichStep::Decompose => (
            p::DECOMPOSE_SYSTEM.to_string(),
            p::decompose_user(&inputs.plan_content, inputs.brief_content.as_deref()),
        ),
        EnrichStep::Research => (
            p::RESEARCH_SYSTEM.to_string(),
            p::research_user(
                &inputs.plan_content,
                inputs.tasks_content.as_deref(),
                inputs.brief_content.as_deref(),
                inputs.decomposition_content.as_deref(),
                inputs.verify_content.as_deref(),
                inputs.review_content.as_deref(),
            ),
        ),
        EnrichStep::Dependencies => (
            p::DEPENDENCIES_SYSTEM.to_string(),
            p::dependencies_user(
                &inputs.plan_content,
                inputs.tasks_content.as_deref(),
                inputs.brief_content.as_deref(),
            ),
        ),
        EnrichStep::Fixtures => (
            p::FIXTURES_SYSTEM.to_string(),
            p::fixtures_user(
                &inputs.plan_content,
                inputs.tasks_content.as_deref(),
                inputs.dependency_manifest.as_deref(),
                inputs.research_content.as_deref(),
            ),
        ),
        EnrichStep::Integration => (
            p::INTEGRATION_SYSTEM.to_string(),
            p::integration_user(
                &inputs.plan_content,
                inputs.tasks_content.as_deref(),
                inputs.verify_content.as_deref(),
                inputs.review_content.as_deref(),
                inputs.research_content.as_deref(),
                inputs.dependency_manifest.as_deref(),
                inputs.fixture_manifest.as_deref(),
            ),
        ),
        EnrichStep::Verify => (
            p::VERIFY_SYSTEM.to_string(),
            p::verify_user(&inputs.plan_content, inputs.tasks_content.as_deref()),
        ),
        EnrichStep::Reviews => (
            p::REVIEW_SYSTEM.to_string(),
            p::review_user(&inputs.plan_content),
        ),
        EnrichStep::Tests => (
            p::TESTS_SYSTEM.to_string(),
            p::tests_user(&inputs.plan_content, inputs.tasks_content.as_deref()),
        ),
        EnrichStep::Invariants => (
            p::INVARIANTS_SYSTEM.to_string(),
            p::invariants_user(&inputs.plan_content),
        ),
        EnrichStep::Scribe => (
            p::SCRIBE_SYSTEM.to_string(),
            p::scribe_user(&inputs.plan_content),
        ),
    }
}

/// Build the repair prompt for invalid TOML output.
///
/// Ported from Mori `repair_toml_output` (lines 872-890).
pub fn build_repair_prompt(
    step: EnrichStep,
    raw_output: &str,
    error_message: &str,
) -> (String, String) {
    let system = "You repair invalid TOML generated by an enrichment pipeline. Return only \
        valid TOML. Preserve the original intent and content, but fix syntax, quoting, arrays, \
        and table structure as needed. Do not add markdown fences or commentary."
        .to_string();

    let user = format!(
        "Artifact step: {step}\n\
        Validation error:\n{error_message}\n\n\
        Invalid candidate TOML:\n```toml\n{raw_output}\n```\n\n\
        Return only corrected TOML."
    );

    (system, user)
}

/// Generate content for a non-LLM step via pure extraction.
///
/// These steps do not call any model — they extract and restructure data from
/// existing artifacts. In the current implementation, all non-LLM steps return
/// a placeholder extraction; a richer extraction is a future enhancement.
///
/// Ported from Mori `generate_without_llm` (lines 893-903).
pub fn generate_without_llm(step: EnrichStep, inputs: &StepInputs) -> Result<String, String> {
    match step {
        EnrichStep::Prd => Ok(extract_prd(&inputs.plan_content)),
        EnrichStep::Briefs => Ok(extract_brief(&inputs.plan_content)),
        EnrichStep::Tasks => Ok(extract_tasks(&inputs.plan_content)),
        EnrichStep::Research => Ok(generate_research(inputs)),
        EnrichStep::Dependencies => Ok(generate_dependency_manifest(inputs)),
        EnrichStep::Fixtures => Ok(generate_fixture_manifest(inputs)),
        EnrichStep::Integration => Ok(generate_integration(inputs)),
        _ => Err(format!("step {step} requires an LLM call")),
    }
}

// ── Pure extraction helpers ─────────────────────────────────────────────────

/// Extract PRD references from plan content.
fn extract_prd(plan: &str) -> String {
    // Simple extraction: pull lines that reference PRD, RFC, or spec documents.
    let mut out = String::from("# PRD Context\n\n");
    let mut found = false;
    for line in plan.lines() {
        let lower = line.to_lowercase();
        if lower.contains("prd")
            || lower.contains("rfc")
            || lower.contains("spec")
            || lower.contains("requirement")
        {
            out.push_str(line);
            out.push('\n');
            found = true;
        }
    }
    if !found {
        out.push_str("No PRD references found in plan.\n");
    }
    out
}

/// Extract a brief from plan content.
fn extract_brief(plan: &str) -> String {
    // Extract headings and first paragraph under each as a brief.
    let mut out = String::from("# Implementation Brief\n\n");
    let mut in_heading = false;
    let mut paragraph_lines = 0;
    for line in plan.lines() {
        if line.starts_with('#') {
            out.push_str(line);
            out.push('\n');
            in_heading = true;
            paragraph_lines = 0;
        } else if in_heading {
            if line.trim().is_empty() {
                if paragraph_lines > 0 {
                    in_heading = false;
                }
                out.push('\n');
            } else {
                out.push_str(line);
                out.push('\n');
                paragraph_lines += 1;
            }
        }
    }
    out
}

/// Extract tasks TOML from plan content.
fn extract_tasks(plan: &str) -> String {
    // Build a minimal tasks.toml from plan headings.
    let mut tasks = Vec::new();
    let mut task_id = 0;

    for line in plan.lines() {
        if line.starts_with("## ") {
            task_id += 1;
            let title = line.trim_start_matches('#').trim();
            tasks.push((format!("T{task_id}"), title.to_string()));
        }
    }

    let mut out = String::from("[meta]\nplan = \"extracted\"\niteration = 1\n");
    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!("total = {}\ndone = 0\n\n", tasks.len()),
    );

    for (id, title) in &tasks {
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!(
                "[[task]]\nid = \"{id}\"\ntitle = \"{title}\"\nstatus = \"pending\"\nfiles = []\n\
                 acceptance = []\ndepends_on = []\n\n"
            ),
        );
    }
    out
}

/// Generate a research memo from available inputs.
fn generate_research(inputs: &StepInputs) -> String {
    let mut out = String::from("# Research Memo\n\n");
    out.push_str("## Plan Summary\n\n");

    // Summarize plan headings.
    for line in inputs.plan_content.lines() {
        if line.starts_with('#') {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push('\n');

    if let Some(ref tasks) = inputs.tasks_content {
        out.push_str("## Task Overview\n\n");
        // Count tasks.
        let task_count = tasks.matches("[[task]]").count();
        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{task_count} tasks defined.\n\n"));
    }

    if let Some(ref brief) = inputs.brief_content {
        out.push_str("## Brief Highlights\n\n");
        for line in brief.lines().take(20) {
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }

    out
}

/// Generate a dependency manifest from available inputs.
fn generate_dependency_manifest(inputs: &StepInputs) -> String {
    let mut entries: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // Scan plan for dependency indicators.
    for line in inputs.plan_content.lines() {
        let lower = line.to_lowercase();
        if lower.contains("depends on")
            || lower.contains("dependency")
            || lower.contains("requires")
        {
            entries
                .entry("plan".to_string())
                .or_default()
                .push(line.trim().to_string());
        }
    }

    let mut out = String::from("# Auto-extracted dependency manifest\n\n");
    if entries.is_empty() {
        out.push_str("# No dependencies detected.\n");
        out.push_str("[meta]\nsource = \"extracted\"\n");
    } else {
        out.push_str("[meta]\nsource = \"extracted\"\n\n");
        for lines in entries.values() {
            for line in lines {
                let _ = std::fmt::Write::write_fmt(
                    &mut out,
                    format_args!("[[dependency]]\nnote = \"{}\"\n\n", line.replace('"', "'")),
                );
            }
        }
    }
    out
}

/// Generate a fixture manifest from available inputs.
fn generate_fixture_manifest(inputs: &StepInputs) -> String {
    let mut out = String::from("[meta]\nsource = \"extracted\"\n\n");

    // Scan for fixture-like references.
    let mut found = false;
    for line in inputs.plan_content.lines() {
        let lower = line.to_lowercase();
        if lower.contains("fixture")
            || lower.contains("mock")
            || lower.contains("stub")
            || lower.contains("sidecar")
            || lower.contains("test data")
        {
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!(
                    "[[fixture]]\nnote = \"{}\"\n\n",
                    line.trim().replace('"', "'")
                ),
            );
            found = true;
        }
    }

    if !found {
        out.push_str("# No fixtures detected.\n");
    }
    out
}

/// Generate integration guidance from available inputs.
fn generate_integration(inputs: &StepInputs) -> String {
    let mut out = String::from("# Integration Guidance\n\n");

    out.push_str("## Plan\n\n");
    // Include first 30 lines of plan for context.
    for line in inputs.plan_content.lines().take(30) {
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');

    if let Some(ref tasks) = inputs.tasks_content {
        out.push_str("## Tasks\n\n");
        for line in tasks.lines().take(30) {
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }

    if let Some(ref research) = inputs.research_content {
        out.push_str("## Research Notes\n\n");
        for line in research.lines().take(20) {
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_prompt_returns_non_empty_for_all_steps() {
        let inputs = StepInputs {
            plan_content: "# Plan\n\n## Step 1\nDo something.".to_string(),
            tasks_content: Some("[meta]\nplan = \"test\"\n\n[[task]]\nid = \"T1\"\n".to_string()),
            brief_content: Some("Brief content.".to_string()),
            decomposition_content: Some("Decomposition content.".to_string()),
            verify_content: Some("Verify content.".to_string()),
            review_content: Some("Review content.".to_string()),
            research_content: Some("Research content.".to_string()),
            dependency_manifest: Some("[meta]\nsource = \"test\"\n".to_string()),
            fixture_manifest: Some("[meta]\nsource = \"test\"\n".to_string()),
        };

        for step in super::super::step::ALL_ORDERED {
            let (system, user) = build_prompt(*step, &inputs);
            assert!(!system.is_empty(), "system prompt empty for {step}");
            assert!(!user.is_empty(), "user message empty for {step}");
        }
    }

    #[test]
    fn generate_without_llm_succeeds_for_non_llm_steps() {
        let inputs = StepInputs {
            plan_content: "# Plan\n\n## Step 1\nDo something.".to_string(),
            tasks_content: Some("[meta]\nplan = \"test\"\n\n[[task]]\nid = \"T1\"\n".to_string()),
            brief_content: Some("Brief content.".to_string()),
            decomposition_content: None,
            verify_content: None,
            review_content: None,
            research_content: Some("Research.".to_string()),
            dependency_manifest: Some("[meta]\n".to_string()),
            fixture_manifest: None,
        };

        let non_llm = [
            EnrichStep::Prd,
            EnrichStep::Briefs,
            EnrichStep::Tasks,
            EnrichStep::Research,
            EnrichStep::Dependencies,
            EnrichStep::Fixtures,
            EnrichStep::Integration,
        ];

        for step in non_llm {
            let result = generate_without_llm(step, &inputs);
            assert!(result.is_ok(), "generate_without_llm failed for {step}");
            assert!(
                !result.as_ref().map_or(true, String::is_empty),
                "empty output for {step}"
            );
        }
    }

    #[test]
    fn generate_without_llm_rejects_llm_steps() {
        let inputs = StepInputs {
            plan_content: "plan".to_string(),
            tasks_content: None,
            brief_content: None,
            decomposition_content: None,
            verify_content: None,
            review_content: None,
            research_content: None,
            dependency_manifest: None,
            fixture_manifest: None,
        };

        let llm_steps = [
            EnrichStep::Decompose,
            EnrichStep::Verify,
            EnrichStep::Reviews,
            EnrichStep::Tests,
            EnrichStep::Invariants,
            EnrichStep::Scribe,
        ];

        for step in llm_steps {
            assert!(
                generate_without_llm(step, &inputs).is_err(),
                "should reject LLM step {step}"
            );
        }
    }

    #[test]
    fn repair_prompt_includes_step_and_error() {
        let (system, user) = build_repair_prompt(
            EnrichStep::Tasks,
            "bad toml content",
            "expected `=` at line 3",
        );
        assert!(system.contains("TOML"));
        assert!(user.contains("tasks"));
        assert!(user.contains("expected `=` at line 3"));
        assert!(user.contains("bad toml content"));
    }
}
