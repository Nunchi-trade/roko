//! Per-task implementer prompt template.
//!
//! Roko-owned task implementer prompt template.
//! Unlike the plan-wide [`ImplementerTemplate`](super::ImplementerTemplate),
//! this template is scoped to a single task: it includes assignment details,
//! file context, sibling task awareness for parallel execution, and per-task
//! learning packs.

use super::common::{self, budget_for};
use super::{PlanSlice, RolePromptTemplate, TaskEnhancements, format_enhancements, truncate};
use crate::prompt::{CacheLayer, Placement, PromptSection, SectionPriority};
use roko_core::AgentRole;

/// A sibling task running in parallel — for file-exclusion awareness.
#[derive(Clone, Debug)]
pub struct SiblingTask {
    /// Task identifier (e.g. "T3").
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Files this sibling owns — the current task must NOT touch these.
    pub files: Vec<String>,
}

/// Typed input for the per-task implementer template. All fields are pre-read
/// strings — no filesystem access.
#[derive(Clone, Debug, Default)]
pub struct TaskImplInput {
    /// AGENTS.md content — coding conventions and behavioral rules.
    pub agents_md: String,
    /// Plan metadata + full content.
    pub plan: PlanSlice,
    /// Task identifier (e.g. "T2").
    pub task_id: String,
    /// Human-readable task title.
    pub task_title: String,
    /// Files this task should create or modify.
    pub task_files: Vec<String>,
    /// Acceptance criteria (one per entry).
    pub acceptance_criteria: Vec<String>,
    /// Strategist brief.
    pub brief: String,
    /// Filtered workspace map (only crates relevant to this task).
    pub workspace_map: String,
    /// PRD2 specification extract.
    pub prd2_extract: String,
    /// Cross-plan context (completed plan registry, etc).
    pub cross_plan_context: String,
    /// Ignored tests ledger.
    pub ignored_tests: String,
    /// Compressed output from prior tasks in the same plan (None on first task).
    pub prior_task_outputs: Option<String>,
    /// Verify chain script content (None if no verify script).
    pub verify_chain: Option<String>,
    /// Per-task typed enhancements from the enrichment pipeline.
    pub task_enhancements: Option<TaskEnhancements>,
    /// Sibling tasks running in parallel — for file-exclusion awareness.
    pub sibling_tasks: Vec<SiblingTask>,
    /// Inline file context (pre-read contents of task files).
    pub file_context: Option<String>,
    /// Learning context pack (playbook + research prepass).
    pub learning_pack: Option<String>,
}

/// Per-task implementer prompt template.
///
/// Generates a focused prompt for a single task within a plan. Includes
/// assignment details, file context, and parallel-task awareness.
pub struct TaskImplTemplate;

static TASK_IMPL_ROLE_IDENTITY: &str = "\
You are the Implementer (single-task mode). Your job is to implement exactly \
one task from the plan.\n\
\n\
Rules:\n\
1. Implement ONLY the task assigned below. Do not touch files outside your list.\n\
2. Read the plan and PRD2 extract for exact values, formulas, and type signatures.\n\
3. If `context/in/implementer-pack.md` exists, read it first for execution guidance.\n\
4. Audit the assigned files first — if work is already present, keep it and fix gaps.\n\
5. Write tests for all public items.\n\
6. No unwrap() in library crates — use ?, ok_or(), or map_err().\n\
7. Every new pub type, function, and field must have a doc comment.\n\
8. No hardcoded absolute paths in committed files.\n\
9. No upward dependencies — leaf crates must have zero workspace-internal deps.\n\
10. Self-validate: cargo check and cargo test on affected crates before signaling done.\n\
11. If sibling tasks are listed, do NOT touch their files — they run in parallel.\n\
12. Operate autonomously. Do not ask questions. Complete all work and end your turn.";

impl RolePromptTemplate for TaskImplTemplate {
    type Input = TaskImplInput;

    fn sections(&self, input: &Self::Input) -> Vec<PromptSection> {
        let mut sections = Vec::with_capacity(14);
        push_base_sections(&mut sections, input);
        push_optional_sections(&mut sections, input);
        sections
    }

    fn role_identity(&self) -> &'static str {
        TASK_IMPL_ROLE_IDENTITY
    }
}

/// Push the 8 always-present base sections.
fn push_base_sections(sections: &mut Vec<PromptSection>, input: &TaskImplInput) {
    let budget = budget_for(AgentRole::Implementer);
    // 1. agents_instructions — System / Critical / Start
    sections.push(common::agents_instructions_section(&input.agents_md));
    // 2. plan_spec — Session / Critical / Start / hard_cap 50k
    sections.push(
        PromptSection::new("plan_spec", truncate(&input.plan.content, budget.plan))
            .with_priority(SectionPriority::Critical)
            .with_cache_layer(CacheLayer::Workspace)
            .with_placement(Placement::Start)
            .with_hard_cap(budget.plan),
    );
    // 3. workspace_map — Session / High / Middle / hard_cap 1.5k
    sections.push(
        PromptSection::new("workspace_map", truncate(&input.workspace_map, 1_500))
            .with_priority(SectionPriority::High)
            .with_cache_layer(CacheLayer::Workspace)
            .with_placement(Placement::Middle)
            .with_hard_cap(1_500),
    );
    // 4. brief — Session / High / Middle / hard_cap 2k
    sections.push(
        PromptSection::new("brief", truncate(&input.brief, 2_000))
            .with_priority(SectionPriority::High)
            .with_cache_layer(CacheLayer::Workspace)
            .with_placement(Placement::Middle)
            .with_hard_cap(2_000),
    );
    // 5. prd2_extract — Session / High / Middle / hard_cap 3k
    sections.push(
        PromptSection::new("prd2_extract", truncate(&input.prd2_extract, 3_000))
            .with_priority(SectionPriority::High)
            .with_cache_layer(CacheLayer::Workspace)
            .with_placement(Placement::Middle)
            .with_hard_cap(3_000),
    );
    // 6. cross_plan_context — Session / Normal / Middle / hard_cap 1k
    sections.push(
        PromptSection::new(
            "cross_plan_context",
            truncate(&input.cross_plan_context, 1_000),
        )
        .with_priority(SectionPriority::Normal)
        .with_cache_layer(CacheLayer::Workspace)
        .with_placement(Placement::Middle)
        .with_hard_cap(1_000),
    );
    // 7. ignored_tests — Session / Low / Middle / hard_cap 500
    if !input.ignored_tests.is_empty() {
        sections.push(
            PromptSection::new("ignored_tests", truncate(&input.ignored_tests, 500))
                .with_priority(SectionPriority::Low)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Middle)
                .with_hard_cap(500),
        );
    }
    // 8. assignment — Task / Critical / End
    sections.push(
        PromptSection::new("assignment", format_assignment(input))
            .with_priority(SectionPriority::Critical)
            .with_cache_layer(CacheLayer::Plan)
            .with_placement(Placement::End),
    );
}

/// Push optional sections that are only present when their inputs are non-empty.
fn push_optional_sections(sections: &mut Vec<PromptSection>, input: &TaskImplInput) {
    if let Some(ref prior) = input.prior_task_outputs {
        sections.push(
            PromptSection::new("prior_task_outputs", truncate(prior, 2_000))
                .with_priority(SectionPriority::Normal)
                .with_cache_layer(CacheLayer::Volatile)
                .with_placement(Placement::Middle)
                .with_hard_cap(2_000),
        );
    }
    if let Some(ref chain) = input.verify_chain {
        sections.push(
            PromptSection::new("verify_chain", truncate(chain, 2_000))
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::End)
                .with_hard_cap(2_000),
        );
    }
    if let Some(ref enh) = input.task_enhancements {
        let text = format_enhancements(enh);
        if !text.is_empty() {
            sections.push(
                PromptSection::new("enhanced_sections", text)
                    .with_priority(SectionPriority::High)
                    .with_cache_layer(CacheLayer::Plan)
                    .with_placement(Placement::End),
            );
        }
    }
    if !input.sibling_tasks.is_empty() {
        sections.push(
            PromptSection::new("sibling_tasks", format_siblings(&input.sibling_tasks))
                .with_priority(SectionPriority::Normal)
                .with_cache_layer(CacheLayer::Plan)
                .with_placement(Placement::End),
        );
    }
    if let Some(ref ctx) = input.file_context {
        sections.push(
            PromptSection::new("file_context", ctx.as_str())
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Plan)
                .with_placement(Placement::End),
        );
    }
    if let Some(ref pack) = input.learning_pack {
        sections.push(
            PromptSection::new("learning_pack", truncate(pack, 3_000))
                .with_priority(SectionPriority::Normal)
                .with_cache_layer(CacheLayer::Volatile)
                .with_placement(Placement::Middle)
                .with_hard_cap(3_000),
        );
    }
}

/// Format the core assignment block: task id, title, files, acceptance criteria.
fn format_assignment(input: &TaskImplInput) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    let _ = writeln!(
        out,
        "## Your Assignment\n\nImplement task {} of plan {}: {}",
        input.task_id, input.plan.base, input.task_title
    );
    let _ = writeln!(out, "\n### Files to Modify");
    for f in &input.task_files {
        let _ = writeln!(out, "- {f}");
    }
    if !input.acceptance_criteria.is_empty() {
        let _ = writeln!(out, "\n### Acceptance Criteria");
        for c in &input.acceptance_criteria {
            let _ = writeln!(out, "- {c}");
        }
    }
    out
}

/// Format the sibling tasks awareness section.
fn format_siblings(siblings: &[SiblingTask]) -> String {
    use std::fmt::Write;

    let mut out = String::from(
        "## Sibling Tasks (running in parallel)\n\n\
         These agents are working simultaneously. Do NOT touch their files:\n",
    );
    for s in siblings {
        let _ = writeln!(
            out,
            "- {}: {} → files: {}",
            s.id,
            s.title,
            s.files.join(", ")
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_input() -> TaskImplInput {
        TaskImplInput {
            agents_md: "# AGENTS.md\nFollow conventions.".into(),
            plan: PlanSlice {
                num: "042".into(),
                base: "golem-mortality".into(),
                title: "Implement mortality model".into(),
                content: "## Plan\nBuild the mortality model.".into(),
            },
            task_id: "T2".into(),
            task_title: "Implement Gompertz formula".into(),
            task_files: vec![
                "crates/golem-core/src/mortality.rs".into(),
                "crates/golem-core/src/lib.rs".into(),
            ],
            acceptance_criteria: vec![
                "compute_rate returns correct Gompertz values".into(),
                "All tests pass".into(),
            ],
            brief: "Strategist brief content.".into(),
            workspace_map: "crates/golem-core/src/lib.rs".into(),
            prd2_extract: "## PRD2\nGompertz: lambda(t) = ae^(bt).".into(),
            cross_plan_context: "plan-041: done".into(),
            ignored_tests: "test_old_feature: reason".into(),
            prior_task_outputs: Some("T1 completed: defined MortalityRate type.".into()),
            verify_chain: Some("#!/bin/bash\ncargo test".into()),
            task_enhancements: Some(TaskEnhancements {
                types_to_define: vec!["MortalityRate".into()],
                formulas: vec!["lambda(t) = a * e^(b*t)".into()],
                imports: vec!["use golem_core::mortality::*".into()],
                example_pattern: Some("match rate { .. }".into()),
                test_invariants: vec!["INV-001".into()],
            }),
            sibling_tasks: vec![SiblingTask {
                id: "T3".into(),
                title: "Implement lifecycle hooks".into(),
                files: vec!["crates/golem-core/src/lifecycle.rs".into()],
            }],
            file_context: Some("// mortality.rs\npub struct MortalityRate;".into()),
            learning_pack: Some("Playbook: use Gompertz from Finkelstein 2008.".into()),
        }
    }

    #[test]
    fn render_golden_full_input() {
        let template = TaskImplTemplate;
        let sections = template.sections(&full_input());

        // All 14 sections present
        assert_eq!(sections.len(), 14);

        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            &[
                "agents_instructions",
                "plan_spec",
                "workspace_map",
                "brief",
                "prd2_extract",
                "cross_plan_context",
                "ignored_tests",
                "assignment",
                "prior_task_outputs",
                "verify_chain",
                "enhanced_sections",
                "sibling_tasks",
                "file_context",
                "learning_pack",
            ]
        );

        // Critical sections
        assert_eq!(sections[0].priority, SectionPriority::Critical); // agents_instructions
        assert_eq!(sections[1].priority, SectionPriority::Critical); // plan_spec
        assert_eq!(sections[7].priority, SectionPriority::Critical); // assignment

        // Cache layers
        assert_eq!(sections[0].cache_layer, CacheLayer::Role);
        assert_eq!(sections[1].cache_layer, CacheLayer::Workspace);
        assert_eq!(sections[7].cache_layer, CacheLayer::Plan);
        assert_eq!(sections[8].cache_layer, CacheLayer::Volatile);

        // Hard caps — task impl uses tighter budgets
        assert_eq!(sections[1].hard_cap, Some(50_000)); // plan_spec
        assert_eq!(sections[2].hard_cap, Some(1_500)); // workspace_map
        assert_eq!(sections[3].hard_cap, Some(2_000)); // brief
        assert_eq!(sections[4].hard_cap, Some(3_000)); // prd2_extract
        assert_eq!(sections[5].hard_cap, Some(1_000)); // cross_plan_context

        // Assignment section includes task id and files
        let assignment = &sections[7].content;
        assert!(assignment.contains("T2"));
        assert!(assignment.contains("golem-mortality"));
        assert!(assignment.contains("Implement Gompertz formula"));
        assert!(assignment.contains("mortality.rs"));
        assert!(assignment.contains("Acceptance Criteria"));
    }

    #[test]
    fn empty_ctx_omits_optional_sections() {
        let template = TaskImplTemplate;
        let input = TaskImplInput {
            agents_md: "agents".into(),
            plan: PlanSlice {
                content: "plan".into(),
                ..Default::default()
            },
            task_id: "T1".into(),
            task_title: "First task".into(),
            task_files: vec!["src/lib.rs".into()],
            brief: "brief".into(),
            workspace_map: "map".into(),
            prd2_extract: "prd2".into(),
            cross_plan_context: "ctx".into(),
            ..Default::default()
        };
        let sections = template.sections(&input);

        // 7 base sections: agents_instructions, plan_spec, workspace_map, brief,
        // prd2_extract, cross_plan_context, assignment
        // ignored_tests is empty → omitted
        assert_eq!(sections.len(), 7);
        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert!(!names.contains(&"prior_task_outputs"));
        assert!(!names.contains(&"verify_chain"));
        assert!(!names.contains(&"enhanced_sections"));
        assert!(!names.contains(&"sibling_tasks"));
        assert!(!names.contains(&"file_context"));
        assert!(!names.contains(&"learning_pack"));
        assert!(!names.contains(&"ignored_tests"));
    }

    #[test]
    fn sibling_tasks_formatted_correctly() {
        let template = TaskImplTemplate;
        let mut input = TaskImplInput {
            agents_md: "a".into(),
            plan: PlanSlice {
                content: "p".into(),
                base: "plan-x".into(),
                ..Default::default()
            },
            task_id: "T1".into(),
            task_title: "My task".into(),
            task_files: vec!["a.rs".into()],
            brief: "b".into(),
            workspace_map: "m".into(),
            prd2_extract: "p".into(),
            cross_plan_context: "c".into(),
            ..Default::default()
        };
        input.sibling_tasks = vec![
            SiblingTask {
                id: "T2".into(),
                title: "Other task".into(),
                files: vec!["b.rs".into(), "c.rs".into()],
            },
            SiblingTask {
                id: "T3".into(),
                title: "Third task".into(),
                files: vec!["d.rs".into()],
            },
        ];
        let sections = template.sections(&input);
        let sibling = sections.iter().find(|s| s.name == "sibling_tasks").unwrap();
        assert!(sibling.content.contains("T2: Other task"));
        assert!(sibling.content.contains("b.rs, c.rs"));
        assert!(sibling.content.contains("T3: Third task"));
        assert!(sibling.content.contains("Do NOT touch their files"));
    }

    #[test]
    fn budget_capped_render_truncates_oversized_workspace_map() {
        let template = TaskImplTemplate;
        let mut input = full_input();
        input.workspace_map = "x".repeat(10_000);
        let sections = template.sections(&input);
        let ws = sections.iter().find(|s| s.name == "workspace_map").unwrap();
        assert!(ws.content.len() < 2_000);
        assert!(ws.content.contains("truncated"));
    }

    #[test]
    fn determinism_identical_input_identical_output() {
        let template = TaskImplTemplate;
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
        let template = TaskImplTemplate;
        let id = template.role_identity();
        assert!(id.len() >= 500);
        assert!(id.len() <= 1500);
        assert!(id.contains("Implementer"));
        assert!(id.contains("single-task"));
    }
}
