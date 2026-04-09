//! Strategist prompt template.
//!
//! Ports Mori's `strategist_prompt` into a typed, I/O-free API.
//! The strategist analyzes a plan and produces a brief + structured TOML
//! task checklist. On iteration 2+, it also processes prior review feedback
//! and generates remediation instructions.

use super::{PlanSlice, RolePromptTemplate, truncate};
use crate::prompt::{CacheLayer, Placement, PromptSection, SectionPriority};

/// Typed input for the strategist template. All fields are pre-read strings —
/// no filesystem access.
#[derive(Clone, Debug, Default)]
pub struct StrategistInput {
    /// AGENTS.md content — coding conventions and behavioral rules.
    pub agents_md: String,
    /// Plan metadata + full content.
    pub plan: PlanSlice,
    /// Full workspace map.
    pub workspace_map: String,
    /// Cross-plan context (completed plan registry).
    pub cross_plan_context: String,
    /// PRD2 specification extract.
    pub prd2_extract: String,
    /// Current iteration number (1-based).
    pub iteration: u32,
    /// Prior review feedback (only for iteration 2+).
    pub prior_reviews: Option<String>,
    /// Existing decomposition content (from prior strategist run).
    pub decomposition: Option<String>,
    /// Preflight snapshot (build/repo health info).
    pub preflight: Option<String>,
    /// Ignored tests ledger.
    pub ignored_tests: Option<String>,
    /// Relative path where the brief should be written.
    pub brief_write_path: String,
    /// Relative path where the tasks TOML should be written.
    pub tasks_write_path: String,
}

/// Strategist prompt template.
///
/// Drives plan analysis and task decomposition. On iteration 1, produces a
/// fresh brief and task checklist. On iteration 2+, incorporates review
/// feedback and generates remediation instructions.
pub struct StrategistTemplate;

static STRATEGIST_ROLE_IDENTITY: &str = "\
You are the Strategist. Your job is to analyze the plan and produce a brief + \
structured task checklist.\n\
\n\
Rules:\n\
1. Read the PRD2 files listed in the prd2 context before writing the brief.\n\
2. Verify that your task breakdown covers all spec requirements.\n\
3. Flag any plan requirements that don't match the PRD2 spec.\n\
4. If a decomposition section is present, align your execution order with those \
steps — do not contradict them without calling out why.\n\
5. Be concrete: reference specific files, types, and line numbers.\n\
6. On iteration 2+, include a Remediation Plan section that addresses each \
blocking issue from prior reviews by ID.\n\
7. Preserve enriched TOML when tasks file already exists — do not replace an \
enriched TOML with a skeletal one.\n\
8. Tasks in the same parallel group must NOT touch the same files when \
exclusive_files is true.\n\
9. Operate autonomously. Do not ask questions.";

impl RolePromptTemplate for StrategistTemplate {
    type Input = StrategistInput;

    fn sections(&self, input: &Self::Input) -> Vec<PromptSection> {
        let mut sections = Vec::with_capacity(10);

        // 1. agents_instructions — System / Critical / Start
        sections.push(
            PromptSection::new("agents_instructions", &input.agents_md)
                .with_priority(SectionPriority::Critical)
                .with_cache_layer(CacheLayer::System)
                .with_placement(Placement::Start),
        );

        // 2. plan_spec — Session / Critical / Start / hard_cap 50k
        sections.push(
            PromptSection::new("plan_spec", truncate(&input.plan.content, 50_000))
                .with_priority(SectionPriority::Critical)
                .with_cache_layer(CacheLayer::Session)
                .with_placement(Placement::Start)
                .with_hard_cap(50_000),
        );

        // 3. workspace_map — Session / High / Middle / hard_cap 12k
        // Strategist gets a generous workspace budget for full plan analysis.
        sections.push(
            PromptSection::new("workspace_map", truncate(&input.workspace_map, 12_000))
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Session)
                .with_placement(Placement::Middle)
                .with_hard_cap(12_000),
        );

        // 4. prd2_extract — Session / High / Middle / hard_cap 8k
        sections.push(
            PromptSection::new("prd2_extract", truncate(&input.prd2_extract, 8_000))
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Session)
                .with_placement(Placement::Middle)
                .with_hard_cap(8_000),
        );

        // 5. cross_plan_context — Session / Normal / Middle / hard_cap 4k
        sections.push(
            PromptSection::new(
                "cross_plan_context",
                truncate(&input.cross_plan_context, 4_000),
            )
            .with_priority(SectionPriority::Normal)
            .with_cache_layer(CacheLayer::Session)
            .with_placement(Placement::Middle)
            .with_hard_cap(4_000),
        );

        // 6. decomposition — Session / Normal / Middle / hard_cap 12k (only when present)
        if let Some(ref decomp) = input.decomposition {
            sections.push(
                PromptSection::new("decomposition", truncate(decomp, 12_000))
                    .with_priority(SectionPriority::Normal)
                    .with_cache_layer(CacheLayer::Session)
                    .with_placement(Placement::Middle)
                    .with_hard_cap(12_000),
            );
        }

        // 7. preflight — Session / Normal / Middle / hard_cap 5k (only when present)
        if let Some(ref pf) = input.preflight {
            sections.push(
                PromptSection::new("preflight", truncate(pf, 5_000))
                    .with_priority(SectionPriority::Normal)
                    .with_cache_layer(CacheLayer::Session)
                    .with_placement(Placement::Middle)
                    .with_hard_cap(5_000),
            );
        }

        // 8. ignored_tests — Session / Low / Middle / hard_cap 3k (only when present)
        if let Some(ref tests) = input.ignored_tests {
            sections.push(
                PromptSection::new("ignored_tests", truncate(tests, 3_000))
                    .with_priority(SectionPriority::Low)
                    .with_cache_layer(CacheLayer::Session)
                    .with_placement(Placement::Middle)
                    .with_hard_cap(3_000),
            );
        }

        // 9. prior_reviews — Dynamic / High / End / hard_cap 10k (only on iteration 2+)
        if input.iteration > 1 {
            if let Some(ref reviews) = input.prior_reviews {
                sections.push(
                    PromptSection::new("prior_reviews", truncate(reviews, 10_000))
                        .with_priority(SectionPriority::High)
                        .with_cache_layer(CacheLayer::Dynamic)
                        .with_placement(Placement::End)
                        .with_hard_cap(10_000),
                );
            }
        }

        // 10. output_paths — System / High / End
        // Tells the strategist where to write brief + tasks TOML.
        let output_text = format_output_instructions(input);
        sections.push(
            PromptSection::new("output_paths", output_text)
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::System)
                .with_placement(Placement::End),
        );

        sections
    }

    fn role_identity(&self) -> &'static str {
        STRATEGIST_ROLE_IDENTITY
    }
}

/// Format the output instructions with write paths and task TOML schema.
fn format_output_instructions(input: &StrategistInput) -> String {
    let remediation = if input.iteration > 1 {
        "\n6. **Remediation Plan**: For each `[B-N]` issue in the Prior Review section \
         above, provide specific fix instructions. Reference the exact ID (e.g., `[B-1]`) \
         so the implementer can cross-reference directly.\n"
    } else {
        ""
    };
    format!(
        r#"## Output Instructions

Write a brief to `{brief_path}` with these sections:

1. **Dependency Verification**: Check that all imported types/traits from prior plans exist.
2. **Conflict Scan**: Identify potential conflicts with existing code.
3. **Execution Order**: Optimal sequence for implementing the plan's units.
4. **Pattern Alignment**: Ensure the plan follows established patterns.
5. **Risk Flags**: Anything that might cause compilation or test failures.
6. **Verification Completeness**: Check that ## Verification exists and covers every formula, state machine, and boundary condition.
{remediation}
After the brief, write a TOML task checklist to `{tasks_path}`:

```toml
[meta]
plan = "{plan_base}"
iteration = {iteration}
total = <number of tasks>
done = 0

[[task]]
id = "T1"
title = "First task title"
status = "pending"
files = ["path/to/file.rs"]
acceptance = ["Acceptance criterion 1"]
allowed_tools = ["read_file", "grep"]
denied_tools = []
mcp_servers = ["filesystem"]
depends_on = []
parallel_group = "A"
exclusive_files = true
```

Parallel group rules: tasks sharing a group run simultaneously; same-group tasks must NOT touch the same files when exclusive_files is true."#,
        brief_path = input.brief_write_path,
        tasks_path = input.tasks_write_path,
        plan_base = input.plan.base,
        iteration = input.iteration,
        remediation = remediation,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_input() -> StrategistInput {
        StrategistInput {
            agents_md: "# AGENTS.md\nConventions.".into(),
            plan: PlanSlice {
                num: "042".into(),
                base: "golem-mortality".into(),
                title: "Implement mortality model".into(),
                content: "## Plan\nDesign the mortality model.".into(),
            },
            workspace_map: "crates/golem-core/src/lib.rs\ncrates/golem-core/src/mortality.rs"
                .into(),
            cross_plan_context: "plan-041: done\nplan-040: done".into(),
            prd2_extract: "## PRD2\nGompertz: lambda(t) = ae^(bt).".into(),
            iteration: 2,
            prior_reviews: Some(
                "[B-1] Missing error handling in compute_rate.\n\
                 [B-2] Type MortalityRate not exported."
                    .into(),
            ),
            decomposition: Some("Step 1: Define types.\nStep 2: Implement formulas.".into()),
            preflight: Some("all green, 142 tests passing".into()),
            ignored_tests: Some("test_old_feature: reason".into()),
            brief_write_path: ".mori/plans/golem-mortality/brief.md".into(),
            tasks_write_path: ".mori/plans/golem-mortality/042-tasks.toml".into(),
        }
    }

    #[test]
    fn render_golden_full_input() {
        let template = StrategistTemplate;
        let sections = template.sections(&full_input());

        // All 10 sections present
        assert_eq!(sections.len(), 10);

        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            &[
                "agents_instructions",
                "plan_spec",
                "workspace_map",
                "prd2_extract",
                "cross_plan_context",
                "decomposition",
                "preflight",
                "ignored_tests",
                "prior_reviews",
                "output_paths",
            ]
        );

        // Critical sections
        assert_eq!(sections[0].priority, SectionPriority::Critical);
        assert_eq!(sections[1].priority, SectionPriority::Critical);

        // Cache layers
        assert_eq!(sections[0].cache_layer, CacheLayer::System);
        assert_eq!(sections[1].cache_layer, CacheLayer::Session);
        assert_eq!(sections[8].cache_layer, CacheLayer::Dynamic); // prior_reviews
        assert_eq!(sections[9].cache_layer, CacheLayer::System); // output_paths

        // Hard caps — strategist gets generous budgets
        assert_eq!(sections[1].hard_cap, Some(50_000)); // plan_spec
        assert_eq!(sections[2].hard_cap, Some(12_000)); // workspace_map
        assert_eq!(sections[3].hard_cap, Some(8_000)); // prd2_extract
        assert_eq!(sections[4].hard_cap, Some(4_000)); // cross_plan_context
        assert_eq!(sections[5].hard_cap, Some(12_000)); // decomposition
        assert_eq!(sections[6].hard_cap, Some(5_000)); // preflight
        assert_eq!(sections[7].hard_cap, Some(3_000)); // ignored_tests
        assert_eq!(sections[8].hard_cap, Some(10_000)); // prior_reviews

        // Output paths section contains write paths and remediation instruction
        let output = &sections[9].content;
        assert!(output.contains("golem-mortality"));
        assert!(output.contains("brief.md"));
        assert!(output.contains("042-tasks.toml"));
        assert!(output.contains("Remediation Plan"));
    }

    #[test]
    fn iteration_1_omits_prior_reviews_and_remediation() {
        let template = StrategistTemplate;
        let mut input = full_input();
        input.iteration = 1;
        input.prior_reviews = Some("Should be ignored on iter 1.".into());
        let sections = template.sections(&input);

        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert!(!names.contains(&"prior_reviews"));

        // Output paths should NOT contain remediation on iteration 1
        let output = sections.iter().find(|s| s.name == "output_paths").unwrap();
        assert!(!output.content.contains("Remediation Plan"));
    }

    #[test]
    fn empty_ctx_omits_optional_sections() {
        let template = StrategistTemplate;
        let input = StrategistInput {
            agents_md: "agents".into(),
            plan: PlanSlice {
                content: "plan".into(),
                base: "test-plan".into(),
                ..Default::default()
            },
            workspace_map: "map".into(),
            cross_plan_context: "ctx".into(),
            prd2_extract: "prd2".into(),
            iteration: 1,
            brief_write_path: "brief.md".into(),
            tasks_write_path: "tasks.toml".into(),
            ..Default::default()
        };
        let sections = template.sections(&input);

        // 6 base sections: agents, plan_spec, workspace_map, prd2, cross_plan_context, output_paths
        assert_eq!(sections.len(), 6);
        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert!(!names.contains(&"decomposition"));
        assert!(!names.contains(&"preflight"));
        assert!(!names.contains(&"ignored_tests"));
        assert!(!names.contains(&"prior_reviews"));
    }

    #[test]
    fn budget_capped_render_truncates_oversized_workspace_map() {
        let template = StrategistTemplate;
        let mut input = full_input();
        input.workspace_map = "x".repeat(50_000);
        let sections = template.sections(&input);
        let ws = sections.iter().find(|s| s.name == "workspace_map").unwrap();
        assert!(ws.content.len() < 13_000);
        assert!(ws.content.contains("truncated"));
    }

    #[test]
    fn determinism_identical_input_identical_output() {
        let template = StrategistTemplate;
        let input = full_input();
        let s1 = template.sections(&input);
        let s2 = template.sections(&input);
        assert_eq!(s1.len(), s2.len());
        for (a, b) in s1.iter().zip(s2.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.content, b.content);
            assert_eq!(a.priority, b.priority);
            assert_eq!(a.cache_layer, b.cache_layer);
            assert_eq!(a.placement, b.placement);
            assert_eq!(a.hard_cap, b.hard_cap);
        }
    }

    #[test]
    fn role_identity_is_substantial() {
        let template = StrategistTemplate;
        let id = template.role_identity();
        assert!(id.len() >= 500);
        assert!(id.len() <= 1500);
        assert!(id.contains("Strategist"));
    }
}
