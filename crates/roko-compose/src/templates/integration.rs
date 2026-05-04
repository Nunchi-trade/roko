//! Integration tester prompt template.
//!
//! Roko-owned integration tester prompt template.
//!
//! The integration tester runs workspace-wide tests after batch merges and
//! reports failures without fixing them.

use super::common::{self, budget_for};
use super::{RolePromptTemplate, truncate};
use crate::prompt::{CacheLayer, Placement, PromptSection, SectionPriority};
use roko_core::AgentRole;

/// Typed input for the integration tester template. All fields are pre-read
/// strings — no filesystem access.
#[derive(Clone, Debug, Default)]
pub struct IntegrationInput {
    /// AGENTS.md content — coding conventions and behavioral rules.
    pub agents_md: String,
    /// The batch branch name (e.g. "batch/current").
    pub batch_branch: String,
    /// Names of plans that were merged into the batch branch.
    pub completed_plans: Vec<String>,
    /// Integration memo artifact content (integration.md).
    pub integration_memo: Option<String>,
    /// Fixture manifest (fixture-manifest.toml content).
    pub fixture_manifest: Option<String>,
    /// Dependency manifest (dependency-manifest.toml content).
    pub dependency_manifest: Option<String>,
}

/// Integration tester prompt template.
///
/// Drives workspace-wide verification after batch merges. The tester reports
/// failures but does not fix them.
pub struct IntegrationTemplate;

static INTEGRATION_ROLE_IDENTITY: &str = "\
You are the Integration Tester. Plans have been merged and you must verify \
the workspace compiles and all tests pass.\n\
\n\
Rules:\n\
1. Read integration.md, fixture-manifest.toml, and dependency-manifest.toml \
before widening to full plan prose.\n\
2. Run `cargo check --workspace` and report the result.\n\
3. Run `cargo test --workspace --no-fail-fast` and capture all output.\n\
4. If nextest is available, also run `cargo nextest run --workspace --no-fail-fast`.\n\
5. Run any Roko verification harness crates present in this workspace.\n\
6. For each test failure identify: crate, test function, likely source plan, \
error category (compile error, runtime panic, assertion failure).\n\
7. Write your report to `.roko/plans/reviews/integration-test-report.md`.\n\
8. Do NOT fix any failures. Only report them.\n\
9. Operate autonomously. Do not ask questions.";

impl RolePromptTemplate for IntegrationTemplate {
    type Input = IntegrationInput;

    fn sections(&self, input: &Self::Input) -> Vec<PromptSection> {
        let budget = budget_for(AgentRole::IntegrationTester);
        let manifest_cap = budget.context.min(2_000);
        let mut sections = Vec::with_capacity(6);

        // 1. agents_instructions — System / Critical / Start
        sections.push(common::agents_instructions_section(&input.agents_md));

        // 2. integration_context — Session / Critical / Start
        // Contains the batch branch and list of merged plans.
        let plans_list = if input.completed_plans.is_empty() {
            "(none)".to_string()
        } else {
            input.completed_plans.join(", ")
        };
        let context_text = format!(
            "## Integration Context\n\n\
             Batch branch: `{}`\n\
             Merged plans: {}\n",
            input.batch_branch, plans_list
        );
        sections.push(
            PromptSection::new("integration_context", context_text)
                .with_priority(SectionPriority::Critical)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Start),
        );

        // 3. integration_memo — Session / High / Middle / hard_cap 4k (only when present)
        if let Some(ref memo) = input.integration_memo {
            sections.push(
                PromptSection::new("integration_memo", truncate(memo, budget.brief))
                    .with_priority(SectionPriority::High)
                    .with_cache_layer(CacheLayer::Workspace)
                    .with_placement(Placement::Middle)
                    .with_hard_cap(budget.brief),
            );
        }

        // 4. fixture_manifest — Session / Normal / Middle / hard_cap 2k (only when present)
        if let Some(ref fixtures) = input.fixture_manifest {
            sections.push(
                PromptSection::new("fixture_manifest", truncate(fixtures, manifest_cap))
                    .with_priority(SectionPriority::Normal)
                    .with_cache_layer(CacheLayer::Workspace)
                    .with_placement(Placement::Middle)
                    .with_hard_cap(manifest_cap),
            );
        }

        // 5. dependency_manifest — Session / Normal / Middle / hard_cap 2k (only when present)
        if let Some(ref deps) = input.dependency_manifest {
            sections.push(
                PromptSection::new("dependency_manifest", truncate(deps, manifest_cap))
                    .with_priority(SectionPriority::Normal)
                    .with_cache_layer(CacheLayer::Workspace)
                    .with_placement(Placement::Middle)
                    .with_hard_cap(manifest_cap),
            );
        }

        // 6. instructions — System / Critical / End
        sections.push(
            PromptSection::new("instructions", INTEGRATION_INSTRUCTIONS)
                .with_priority(SectionPriority::Critical)
                .with_cache_layer(CacheLayer::Role)
                .with_placement(Placement::End),
        );

        sections
    }

    fn role_identity(&self) -> &'static str {
        INTEGRATION_ROLE_IDENTITY
    }
}

static INTEGRATION_INSTRUCTIONS: &str = "\
## Instructions\n\
\n\
1. Start with `context/in/integration-tester-pack.md` when present \
(otherwise `context/in/execution-pack.md`).\n\
2. Read integration.md, fixture-manifest.toml, and dependency-manifest.toml \
before widening to full plan prose.\n\
3. Run `cargo check --workspace` and report the result.\n\
4. Run `cargo test --workspace --no-fail-fast` and capture all output.\n\
5. If nextest is available, run `cargo nextest run --workspace --no-fail-fast`.\n\
6. Run any Roko verification harness crates present in this workspace.\n\
7. For each test failure:\n\
   - Identify which crate and test function failed\n\
   - Check `git log --oneline -5` to identify which plan's merge likely caused it\n\
   - Note whether it's a compile error, runtime panic, or assertion failure\n\
\n\
Write your report with:\n\
- Workspace compile status (PASS/FAIL)\n\
- Total tests run, passed, failed, ignored\n\
- Per-crate breakdown of failures\n\
- For each failure: test name, error, likely source plan\n\
\n\
Do NOT fix any failures. Only report them.";

#[cfg(test)]
mod tests {
    use super::*;

    fn full_input() -> IntegrationInput {
        IntegrationInput {
            agents_md: "# AGENTS.md\nConventions.".into(),
            batch_branch: "batch/current".into(),
            completed_plans: vec!["plan-041".into(), "plan-042".into(), "plan-043".into()],
            integration_memo: Some("## Integration\nCross-crate wiring notes.".into()),
            fixture_manifest: Some("[fixture]\nname = \"anvil\"\nport = 8545".into()),
            dependency_manifest: Some("[dep]\ncrate = \"golem-core\"\nversion = \"0.1\"".into()),
        }
    }

    #[test]
    fn render_golden_full_input() {
        let template = IntegrationTemplate;
        let sections = template.sections(&full_input());

        // All 6 sections present
        assert_eq!(sections.len(), 6);

        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            &[
                "agents_instructions",
                "integration_context",
                "integration_memo",
                "fixture_manifest",
                "dependency_manifest",
                "instructions",
            ]
        );

        // Critical sections
        assert_eq!(sections[0].priority, SectionPriority::Critical);
        assert_eq!(sections[1].priority, SectionPriority::Critical);
        assert_eq!(sections[5].priority, SectionPriority::Critical);

        // Cache layers
        assert_eq!(sections[0].cache_layer, CacheLayer::Role);
        assert_eq!(sections[1].cache_layer, CacheLayer::Workspace);
        assert_eq!(sections[2].cache_layer, CacheLayer::Workspace);
        assert_eq!(sections[5].cache_layer, CacheLayer::Role);

        // Hard caps
        assert_eq!(sections[2].hard_cap, Some(4_000)); // integration_memo
        assert_eq!(sections[3].hard_cap, Some(2_000)); // fixture_manifest
        assert_eq!(sections[4].hard_cap, Some(2_000)); // dependency_manifest

        // Context contains plan list
        let ctx = &sections[1].content;
        assert!(ctx.contains("batch/current"));
        assert!(ctx.contains("plan-041"));
        assert!(ctx.contains("plan-042"));
        assert!(ctx.contains("plan-043"));
    }

    #[test]
    fn empty_ctx_omits_optional_sections() {
        let template = IntegrationTemplate;
        let input = IntegrationInput {
            agents_md: "agents".into(),
            batch_branch: "batch/test".into(),
            completed_plans: vec![],
            integration_memo: None,
            fixture_manifest: None,
            dependency_manifest: None,
        };
        let sections = template.sections(&input);

        // 3 base sections: agents_instructions, integration_context, instructions
        assert_eq!(sections.len(), 3);
        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert!(!names.contains(&"integration_memo"));
        assert!(!names.contains(&"fixture_manifest"));
        assert!(!names.contains(&"dependency_manifest"));

        // Empty plans list shows "(none)"
        let ctx = &sections[1].content;
        assert!(ctx.contains("(none)"));
    }

    #[test]
    fn budget_capped_render_truncates_oversized_memo() {
        let template = IntegrationTemplate;
        let mut input = full_input();
        input.integration_memo = Some("x".repeat(20_000));
        let sections = template.sections(&input);
        let memo = sections
            .iter()
            .find(|s| s.name == "integration_memo")
            .unwrap();
        assert!(memo.content.len() < 5_000);
        assert!(memo.content.contains("truncated"));
    }

    #[test]
    fn determinism_identical_input_identical_output() {
        let template = IntegrationTemplate;
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
        let template = IntegrationTemplate;
        let id = template.role_identity();
        assert!(id.len() >= 500);
        assert!(id.len() <= 1500);
        assert!(id.contains("Integration Tester"));
    }
}
