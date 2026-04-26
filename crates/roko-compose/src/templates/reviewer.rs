//! Reviewer prompt template.
//!
//! Roko-owned architect, auditor, and combined-reviewer prompts in a single
//! template with enum dispatch. All three share a common context prefix (plan,
//! workspace map, prd2, brief) and differ only in role identity and instructions.

use super::common::budget_for;
use super::{PlanSlice, RolePromptTemplate, truncate};
use crate::prompt::{CacheLayer, Placement, PromptSection, SectionPriority};
use roko_core::AgentRole;

/// Which reviewer variant to generate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reviewer {
    /// Structural review — code quality, patterns, module structure.
    Architect,
    /// Spec compliance — exports, formulas, invariant coverage.
    Auditor,
    /// Combined pass — both code quality and spec compliance in one agent.
    Combined,
}

/// Typed input for the reviewer template. All fields are pre-read strings.
#[derive(Clone, Debug, Default)]
pub struct ReviewerInput {
    /// AGENTS.md content.
    pub agents_md: String,
    /// Plan metadata + full content.
    pub plan: PlanSlice,
    /// Filtered workspace map (only crates touched by this plan).
    pub filtered_workspace_map: String,
    /// PRD2 specification extract.
    pub prd2_extract: String,
    /// Strategist brief.
    pub brief: String,
    /// List of files changed by the implementer.
    pub files_changed: Vec<String>,
    /// Prior review findings from a previous iteration (None on first pass).
    pub prior_findings: Option<String>,
}

/// Reviewer prompt template (Architect / Auditor / Combined).
///
/// One template, three variants. The shared context prefix is identical for
/// cache-hit optimization; only the role identity and instruction block differ.
pub struct ReviewerTemplate {
    variant: Reviewer,
}

impl ReviewerTemplate {
    /// Create a reviewer template for the given variant.
    #[must_use]
    pub const fn new(variant: Reviewer) -> Self {
        Self { variant }
    }
}

static ARCHITECT_ROLE_IDENTITY: &str = "\
You are the Architect. Review the implementation for code quality.\n\
\n\
Your focus:\n\
- cargo check --workspace passes with zero errors\n\
- No unwrap() calls in library crates — use ?, ok_or(), map_err()\n\
- Every new pub type, function, and field has a doc comment\n\
- No hardcoded absolute paths in committed files\n\
- No upward dependencies in leaf crates\n\
- All tests from the plan's Verification section pass\n\
- Diff-focused: review what changed, not pre-existing code\n\
\n\
Be exhaustive. Report ALL issues in one pass. The implementer fixes everything \
you report in a single cycle — incomplete reviews cause unnecessary re-review. \
Only REVISE for genuine functional bugs that would cause runtime failures or \
data corruption. Style, naming, and clippy warnings are nits, not blockers.\n\
\n\
Operate autonomously. Do not ask questions.";

static AUDITOR_ROLE_IDENTITY: &str = "\
You are the Auditor. Verify the implementation matches the specification.\n\
\n\
Your focus:\n\
- Every export listed in the plan exists with the exact visibility stated\n\
- All formula constants match PRD2 values exactly — no rounding\n\
- All INV-NNN invariant tests exist and pass\n\
- All behavioral rules (states, transitions, lifecycle) are implemented\n\
- Type signatures match the plan's Quick Reference\n\
- Cargo dependencies match the plan's specification\n\
\n\
Be exhaustive. Walk through the entire plan specification and verify every \
requirement. Compile a COMPLETE list so everything can be fixed in one pass. \
Only REVISE for genuine spec violations that cause functional problems. Minor \
deviations from the plan are not blocking if core functionality works.\n\
\n\
Operate autonomously. Do not ask questions.";

static COMBINED_ROLE_IDENTITY: &str = "\
You are the Combined Reviewer. Perform both code quality review (Architect) \
and spec compliance verification (Auditor) in a single pass.\n\
\n\
Code quality checks:\n\
- Compilation, unwrap() usage, doc comments, dependency direction, tests\n\
\n\
Spec compliance checks:\n\
- Exports, formula constants, invariant tests, behavioral rules, type signatures\n\
\n\
Be exhaustive in both dimensions. Report ALL issues in one pass. Only REVISE \
for genuine blocking issues — functional bugs or spec violations that break \
the code. Style preferences and minor deviations are nits.\n\
\n\
Operate autonomously. Do not ask questions.";

impl RolePromptTemplate for ReviewerTemplate {
    type Input = ReviewerInput;

    fn sections(&self, input: &Self::Input) -> Vec<PromptSection> {
        let budget = budget_for(match self.variant {
            Reviewer::Architect => AgentRole::Architect,
            Reviewer::Auditor | Reviewer::Combined => AgentRole::Auditor,
        });
        let mut sections = Vec::with_capacity(8);

        // 1. agents_instructions — System / Critical
        sections.push(
            PromptSection::new("agents_instructions", &input.agents_md)
                .with_priority(SectionPriority::Critical)
                .with_cache_layer(CacheLayer::Role)
                .with_placement(Placement::Start),
        );

        // 2. plan_spec — Session / Critical / hard_cap 50k
        sections.push(
            PromptSection::new("plan_spec", truncate(&input.plan.content, budget.plan))
                .with_priority(SectionPriority::Critical)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Start)
                .with_hard_cap(budget.plan),
        );

        // 3. workspace_map — Session / High / hard_cap 6k (reviewer budget is smaller)
        sections.push(
            PromptSection::new(
                "workspace_map",
                truncate(&input.filtered_workspace_map, budget.workspace_map),
            )
            .with_priority(SectionPriority::High)
            .with_cache_layer(CacheLayer::Workspace)
            .with_placement(Placement::Middle)
            .with_hard_cap(budget.workspace_map),
        );

        // 4. prd2_extract — Session / High / hard_cap 6k
        sections.push(
            PromptSection::new("prd2_extract", truncate(&input.prd2_extract, budget.prd2))
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Middle)
                .with_hard_cap(budget.prd2),
        );

        // 5. brief — Session / High / hard_cap 4k
        sections.push(
            PromptSection::new("brief", truncate(&input.brief, budget.brief))
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Middle)
                .with_hard_cap(budget.brief),
        );

        // 6. reviewer_criteria — System / Normal
        // Content varies by variant.
        let criteria = match self.variant {
            Reviewer::Architect => ARCHITECT_CRITERIA,
            Reviewer::Auditor => AUDITOR_CRITERIA,
            Reviewer::Combined => COMBINED_CRITERIA,
        };
        sections.push(
            PromptSection::new("reviewer_criteria", criteria)
                .with_priority(SectionPriority::Normal)
                .with_cache_layer(CacheLayer::Role)
                .with_placement(Placement::End),
        );

        // 7. files_changed — Task / High (only when non-empty)
        if !input.files_changed.is_empty() {
            let text = super::format_files_changed(&input.files_changed);
            sections.push(
                PromptSection::new("files_changed", text)
                    .with_priority(SectionPriority::High)
                    .with_cache_layer(CacheLayer::Plan)
                    .with_placement(Placement::End),
            );
        }

        // 8. prior_findings — Dynamic / High / hard_cap 15k (only on iteration 2+)
        if let Some(ref findings) = input.prior_findings {
            sections.push(
                PromptSection::new("prior_findings", truncate(findings, budget.reviews))
                    .with_priority(SectionPriority::High)
                    .with_cache_layer(CacheLayer::Volatile)
                    .with_placement(Placement::End)
                    .with_hard_cap(budget.reviews),
            );
        }

        sections
    }

    fn role_identity(&self) -> &'static str {
        match self.variant {
            Reviewer::Architect => ARCHITECT_ROLE_IDENTITY,
            Reviewer::Auditor => AUDITOR_ROLE_IDENTITY,
            Reviewer::Combined => COMBINED_ROLE_IDENTITY,
        }
    }
}

static ARCHITECT_CRITERIA: &str = "\
Architect review criteria:\n\
- cargo check --workspace passes\n\
- No unwrap() in library crates\n\
- Doc comments on all new pub items\n\
- No hardcoded absolute paths\n\
- No upward dependencies\n\
- All plan Verification tests pass\n\
- Review the diff, not pre-existing code\n\
- Emit structured JSON findings: issue_id, severity, file, line, description, fix_hint";

static AUDITOR_CRITERIA: &str = "\
Auditor review criteria:\n\
- All plan exports exist with exact visibility\n\
- Formula constants match PRD2 exactly\n\
- All INV-NNN invariant tests exist and pass\n\
- Behavioral rules implemented (states, transitions, lifecycle)\n\
- Type signatures match plan Quick Reference\n\
- Cargo dependencies match plan specification\n\
- Emit structured JSON findings: issue_id, severity, file, line, description, fix_hint";

static COMBINED_CRITERIA: &str = "\
Combined review criteria:\n\
Code quality: compilation, unwrap() usage, doc comments, dependency direction, tests.\n\
Spec compliance: exports, formula constants, invariant tests, behavioral rules, types.\n\
- Emit structured JSON findings: issue_id, severity, file, line, description, fix_hint";

#[cfg(test)]
mod tests {
    use super::*;

    fn full_input() -> ReviewerInput {
        ReviewerInput {
            agents_md: "# AGENTS.md\nConventions here.".into(),
            plan: PlanSlice {
                num: "042".into(),
                base: "golem-mortality".into(),
                title: "Mortality model".into(),
                content: "## Plan\nImplement mortality.".into(),
            },
            filtered_workspace_map: "crates/golem-core/src/lib.rs".into(),
            prd2_extract: "## PRD2\nMortality formula.".into(),
            brief: "Brief content.".into(),
            files_changed: vec!["crates/golem-core/src/mortality.rs".into()],
            prior_findings: Some("Fix error handling in compute_rate.".into()),
        }
    }

    #[test]
    fn render_golden_architect() {
        let template = ReviewerTemplate::new(Reviewer::Architect);
        let sections = template.sections(&full_input());

        // 8 sections: 6 base + files_changed + prior_findings
        assert_eq!(sections.len(), 8);

        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            &[
                "agents_instructions",
                "plan_spec",
                "workspace_map",
                "prd2_extract",
                "brief",
                "reviewer_criteria",
                "files_changed",
                "prior_findings",
            ]
        );

        // Cache layers match spec
        assert_eq!(sections[0].cache_layer, CacheLayer::Role); // agents_instructions
        assert_eq!(sections[1].cache_layer, CacheLayer::Workspace); // plan_spec
        assert_eq!(sections[2].cache_layer, CacheLayer::Workspace); // workspace_map
        assert_eq!(sections[5].cache_layer, CacheLayer::Role); // reviewer_criteria

        // Hard caps match spec — reviewer budgets are smaller
        assert_eq!(sections[1].hard_cap, Some(50_000)); // plan_spec
        assert_eq!(sections[2].hard_cap, Some(6_000)); // workspace_map
        assert_eq!(sections[3].hard_cap, Some(6_000)); // prd2_extract
        assert_eq!(sections[4].hard_cap, Some(4_000)); // brief

        // Criteria content matches variant
        assert!(sections[5].content.contains("Architect"));
    }

    #[test]
    fn render_golden_auditor() {
        let template = ReviewerTemplate::new(Reviewer::Auditor);
        let sections = template.sections(&full_input());
        let criteria = sections
            .iter()
            .find(|s| s.name == "reviewer_criteria")
            .unwrap();
        assert!(criteria.content.contains("Auditor"));
        assert!(criteria.content.contains("Formula constants"));
    }

    #[test]
    fn render_golden_combined() {
        let template = ReviewerTemplate::new(Reviewer::Combined);
        let sections = template.sections(&full_input());
        let criteria = sections
            .iter()
            .find(|s| s.name == "reviewer_criteria")
            .unwrap();
        assert!(criteria.content.contains("Combined"));
    }

    #[test]
    fn budget_capped_render_truncates_oversized_prd2() {
        let template = ReviewerTemplate::new(Reviewer::Architect);
        let mut input = full_input();
        input.prd2_extract = "x".repeat(20_000);
        let sections = template.sections(&input);
        let prd2 = sections.iter().find(|s| s.name == "prd2_extract").unwrap();
        assert!(prd2.content.len() < 7_000);
        assert!(prd2.content.contains("truncated"));
    }

    #[test]
    fn empty_ctx_omits_optional_sections() {
        let template = ReviewerTemplate::new(Reviewer::Architect);
        let input = ReviewerInput {
            agents_md: "agents".into(),
            plan: PlanSlice {
                content: "plan".into(),
                ..Default::default()
            },
            filtered_workspace_map: "map".into(),
            prd2_extract: "prd2".into(),
            brief: "brief".into(),
            files_changed: vec![],
            prior_findings: None,
        };
        let sections = template.sections(&input);

        // 6 base sections, no files_changed, no prior_findings
        assert_eq!(sections.len(), 6);
        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert!(!names.contains(&"files_changed"));
        assert!(!names.contains(&"prior_findings"));
    }

    #[test]
    fn determinism_identical_input_identical_output() {
        let template = ReviewerTemplate::new(Reviewer::Auditor);
        let input = full_input();
        let s1 = template.sections(&input);
        let s2 = template.sections(&input);
        assert_eq!(s1.len(), s2.len());
        for (a, b) in s1.iter().zip(s2.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.content, b.content);
            assert_eq!(a.priority, b.priority);
            assert_eq!(a.cache_layer, b.cache_layer);
        }
    }

    #[test]
    fn role_identity_varies_by_variant() {
        let arch = ReviewerTemplate::new(Reviewer::Architect);
        let aud = ReviewerTemplate::new(Reviewer::Auditor);
        let comb = ReviewerTemplate::new(Reviewer::Combined);
        assert!(arch.role_identity().contains("Architect"));
        assert!(aud.role_identity().contains("Auditor"));
        assert!(comb.role_identity().contains("Combined"));
        // All substantial (500-1500 chars range)
        assert!(arch.role_identity().len() >= 500);
        assert!(aud.role_identity().len() >= 500);
        assert!(comb.role_identity().len() >= 500);
    }
}
