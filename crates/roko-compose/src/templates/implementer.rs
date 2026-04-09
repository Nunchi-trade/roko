//! Implementer prompt template.
//!
//! Ports Mori's `implementer_sections` + `implementer_prompt` into a typed,
//! I/O-free API. The most context-heavy template in the set.

use super::{PlanSlice, RolePromptTemplate, TaskEnhancements, format_enhancements, truncate};
use crate::prompt::{CacheLayer, Placement, PromptSection, SectionPriority};

/// Typed input for the implementer template. All fields are pre-read strings.
#[derive(Clone, Debug, Default)]
pub struct ImplementerInput {
    /// AGENTS.md content — coding conventions and behavioral rules.
    pub agents_md: String,
    /// Plan metadata + full content.
    pub plan: PlanSlice,
    /// Strategist brief (may be empty if brief step hasn't run).
    pub brief: String,
    /// Tasks TOML content — the task checklist.
    pub tasks: String,
    /// Workspace map (tree of crates and modules).
    pub workspace_map: String,
    /// Preflight snapshot (build/repo health info).
    pub preflight: String,
    /// Completed plan registry snapshot.
    pub registry_snapshot: String,
    /// Prior iteration review feedback (None on first iteration).
    pub prev_reviews: Option<String>,
    /// Verify chain script content (None if no verify script).
    pub verify_chain: Option<String>,
    /// INV-NN invariant blocks (None if no invariants).
    pub invariants: Option<String>,
    /// Per-task typed enhancements from the enrichment pipeline.
    pub task_enhancements: Option<TaskEnhancements>,
}

/// Implementer prompt template.
///
/// Drives code generation. Emits the richest section set of any role.
pub struct ImplementerTemplate;

static IMPLEMENTER_ROLE_IDENTITY: &str = "\
You are the Implementer. Your job is to write production-quality code that \
satisfies the plan specification exactly.\n\
\n\
Rules:\n\
1. Read the plan carefully. Implement each unit of work in sequence.\n\
2. For each unit: implement the code, write tests, create/update documentation.\n\
3. Verify exports, doc comments, and unwrap() usage.\n\
4. Treat the current repository state as real. Do not assume a blank starting point.\n\
5. When current code is newer or broader than the plan, keep the newer behavior and \
document the deviation.\n\
6. Never add unwrap() in library crates — use ?, ok_or(), or map_err().\n\
7. Every new pub type, function, and field in a library crate must have a doc comment.\n\
8. No hardcoded absolute paths in any committed file.\n\
9. No upward dependencies — leaf crates must have zero workspace-internal deps.\n\
10. All tests from the plan's Verification section must pass.\n\
11. Self-validate before signaling done: cargo check, cargo test on affected crates.\n\
12. Operate autonomously. Do not ask questions. Complete all work and end your turn.";

impl RolePromptTemplate for ImplementerTemplate {
    type Input = ImplementerInput;

    fn sections(&self, input: &Self::Input) -> Vec<PromptSection> {
        let mut sections = Vec::with_capacity(10);

        // 1. agents_instructions — System / Critical
        sections.push(
            PromptSection::new("agents_instructions", &input.agents_md)
                .with_priority(SectionPriority::Critical)
                .with_cache_layer(CacheLayer::System)
                .with_placement(Placement::Start),
        );

        // 2. plan_spec — Session / Critical / hard_cap 50k
        sections.push(
            PromptSection::new("plan_spec", truncate(&input.plan.content, 50_000))
                .with_priority(SectionPriority::Critical)
                .with_cache_layer(CacheLayer::Session)
                .with_placement(Placement::Start)
                .with_hard_cap(50_000),
        );

        // 3. brief — Session / High
        sections.push(
            PromptSection::new("brief", &input.brief)
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Session)
                .with_placement(Placement::Start),
        );

        // 4. tasks — Task / High
        sections.push(
            PromptSection::new("tasks", &input.tasks)
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Task)
                .with_placement(Placement::Middle),
        );

        // 5. workspace_map — Session / High / hard_cap 20k
        sections.push(
            PromptSection::new("workspace_map", truncate(&input.workspace_map, 20_000))
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Session)
                .with_placement(Placement::Middle)
                .with_hard_cap(20_000),
        );

        // 6. preflight — Session / Normal / hard_cap 5k
        sections.push(
            PromptSection::new("preflight", truncate(&input.preflight, 5_000))
                .with_priority(SectionPriority::Normal)
                .with_cache_layer(CacheLayer::Session)
                .with_placement(Placement::Middle)
                .with_hard_cap(5_000),
        );

        // 7. registry — Dynamic / Normal / hard_cap 8k
        sections.push(
            PromptSection::new("registry", truncate(&input.registry_snapshot, 8_000))
                .with_priority(SectionPriority::Normal)
                .with_cache_layer(CacheLayer::Dynamic)
                .with_placement(Placement::Middle)
                .with_hard_cap(8_000),
        );

        // 8. prev_reviews — Dynamic / High / hard_cap 15k (only when present)
        if let Some(ref reviews) = input.prev_reviews {
            sections.push(
                PromptSection::new("prev_reviews", truncate(reviews, 15_000))
                    .with_priority(SectionPriority::High)
                    .with_cache_layer(CacheLayer::Dynamic)
                    .with_placement(Placement::End)
                    .with_hard_cap(15_000),
            );
        }

        // 9. verify_chain — Session / High / hard_cap 4k (only when present)
        if let Some(ref chain) = input.verify_chain {
            sections.push(
                PromptSection::new("verify_chain", truncate(chain, 4_000))
                    .with_priority(SectionPriority::High)
                    .with_cache_layer(CacheLayer::Session)
                    .with_placement(Placement::End)
                    .with_hard_cap(4_000),
            );
        }

        // 10. invariants — Session / High / hard_cap 4k (only when present)
        if let Some(ref inv) = input.invariants {
            sections.push(
                PromptSection::new("invariants", truncate(inv, 4_000))
                    .with_priority(SectionPriority::High)
                    .with_cache_layer(CacheLayer::Session)
                    .with_placement(Placement::End)
                    .with_hard_cap(4_000),
            );
        }

        // 11. enhanced_sections — Task / High (only when non-empty)
        if let Some(ref enh) = input.task_enhancements {
            let text = format_enhancements(enh);
            if !text.is_empty() {
                sections.push(
                    PromptSection::new("enhanced_sections", text)
                        .with_priority(SectionPriority::High)
                        .with_cache_layer(CacheLayer::Task)
                        .with_placement(Placement::End),
                );
            }
        }

        sections
    }

    fn role_identity(&self) -> &'static str {
        IMPLEMENTER_ROLE_IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_input() -> ImplementerInput {
        ImplementerInput {
            agents_md: "# AGENTS.md\nFollow conventions.".into(),
            plan: PlanSlice {
                num: "042".into(),
                base: "golem-mortality".into(),
                title: "Implement mortality model".into(),
                content: "## Plan\nBuild the mortality model.".into(),
            },
            brief: "Strategist brief content.".into(),
            tasks: "[task]\nname = \"implement mortality\"".into(),
            workspace_map: "crates/golem-core/src/lib.rs".into(),
            preflight: "all green".into(),
            registry_snapshot: "plan-041: done".into(),
            prev_reviews: Some("Fix the error handling in module X.".into()),
            verify_chain: Some("#!/bin/bash\ncargo test".into()),
            invariants: Some("INV-001: mortality rate >= 0".into()),
            task_enhancements: Some(TaskEnhancements {
                types_to_define: vec!["MortalityRate".into()],
                formulas: vec!["lambda(t) = a * e^(b*t)".into()],
                imports: vec!["use golem_core::mortality::*".into()],
                example_pattern: Some("match rate { .. }".into()),
                test_invariants: vec!["INV-001".into()],
            }),
        }
    }

    #[test]
    fn render_golden_full_input() {
        let template = ImplementerTemplate;
        let sections = template.sections(&full_input());

        // Expect all 11 sections: 7 base + prev_reviews + verify_chain + invariants + enhanced_sections
        assert_eq!(sections.len(), 11);

        // Verify section names
        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            &[
                "agents_instructions",
                "plan_spec",
                "brief",
                "tasks",
                "workspace_map",
                "preflight",
                "registry",
                "prev_reviews",
                "verify_chain",
                "invariants",
                "enhanced_sections",
            ]
        );

        // Critical sections
        assert_eq!(sections[0].priority, SectionPriority::Critical); // agents_instructions
        assert_eq!(sections[1].priority, SectionPriority::Critical); // plan_spec

        // Cache layers match spec
        assert_eq!(sections[0].cache_layer, CacheLayer::System);
        assert_eq!(sections[1].cache_layer, CacheLayer::Session);
        assert_eq!(sections[3].cache_layer, CacheLayer::Task); // tasks
        assert_eq!(sections[6].cache_layer, CacheLayer::Dynamic); // registry

        // Hard caps match Mori
        assert_eq!(sections[1].hard_cap, Some(50_000)); // plan_spec
        assert_eq!(sections[4].hard_cap, Some(20_000)); // workspace_map
        assert_eq!(sections[5].hard_cap, Some(5_000)); // preflight
        assert_eq!(sections[6].hard_cap, Some(8_000)); // registry
    }

    #[test]
    fn budget_capped_render_truncates_oversized_plan() {
        let template = ImplementerTemplate;
        let mut input = full_input();
        input.plan.content = "x".repeat(100_000);
        let sections = template.sections(&input);
        let plan_section = sections.iter().find(|s| s.name == "plan_spec").unwrap();
        // Content should be truncated to ~50k + truncation marker
        assert!(plan_section.content.len() < 55_000);
        assert!(plan_section.content.contains("truncated"));
    }

    #[test]
    fn empty_ctx_omits_optional_sections() {
        let template = ImplementerTemplate;
        let input = ImplementerInput {
            agents_md: "agents".into(),
            plan: PlanSlice {
                content: "plan".into(),
                ..Default::default()
            },
            brief: String::new(),
            tasks: "tasks".into(),
            workspace_map: "map".into(),
            preflight: "ok".into(),
            registry_snapshot: "reg".into(),
            prev_reviews: None,
            verify_chain: None,
            invariants: None,
            task_enhancements: None,
        };
        let sections = template.sections(&input);

        // Should have 7 base sections, no optional ones
        assert_eq!(sections.len(), 7);
        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert!(!names.contains(&"prev_reviews"));
        assert!(!names.contains(&"verify_chain"));
        assert!(!names.contains(&"invariants"));
        assert!(!names.contains(&"enhanced_sections"));
    }

    #[test]
    fn empty_enhancements_omitted() {
        let template = ImplementerTemplate;
        let input = ImplementerInput {
            agents_md: "a".into(),
            plan: PlanSlice {
                content: "p".into(),
                ..Default::default()
            },
            tasks: "t".into(),
            workspace_map: "m".into(),
            preflight: "ok".into(),
            registry_snapshot: "r".into(),
            task_enhancements: Some(TaskEnhancements::default()),
            ..Default::default()
        };
        let sections = template.sections(&input);
        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert!(!names.contains(&"enhanced_sections"));
    }

    #[test]
    fn determinism_identical_input_identical_output() {
        let template = ImplementerTemplate;
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
        let template = ImplementerTemplate;
        let id = template.role_identity();
        assert!(id.len() >= 500);
        assert!(id.len() <= 1500);
        assert!(id.contains("Implementer"));
    }
}
