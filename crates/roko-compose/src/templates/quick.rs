//! Quick-pass prompt templates.
//!
//! Two lightweight templates for fast review/fix cycles:
//!
//! - [`QuickReviewerTemplate`] — focused single-pass review checking only
//!   correctness, API alignment, compilation, and blocking omissions. Skips
//!   docs, style, naming, and performance (~5k tokens vs ~60k for full review).
//!
//! - [`QuickFixTemplate`] — minimal fix-only prompt. Does not re-read the plan
//!   or workspace map; only includes the compressed feedback and fix directives.
//!
//! Ports Mori's `quick_reviewer_prompt` and `quick_fix_prompt` from
//! `prompts.rs:3468` and `prompts.rs:3622`.

use super::common::{budget_for, format_prior_review, format_verdict_instructions};
use super::{PlanSlice, RolePromptTemplate, truncate};
use crate::prompt::{CacheLayer, Placement, PromptSection, SectionPriority};
use roko_core::AgentRole;

// ─── Quick Reviewer ──────────────────────────────────────────────────────────

/// Typed input for the quick reviewer template.
#[derive(Clone, Debug, Default)]
pub struct QuickReviewerInput {
    /// AGENTS.md content — coding conventions and behavioral rules.
    pub agents_md: String,
    /// Plan metadata + full content.
    pub plan: PlanSlice,
    /// Workspace map (file tree, filtered to relevant crates).
    pub workspace_map: String,
    /// Strategist brief.
    pub brief: String,
    /// Current iteration number (1-based).
    pub iteration: u32,
    /// Prior review feedback (only for iteration 2+).
    pub prior_review: Option<String>,
}

/// Quick reviewer prompt template.
///
/// Drives a focused single-pass review. Checks only: correctness, API
/// alignment, compilation, blocking omissions. Produces a TOML verdict
/// in the same structured format as the architect review. Keeps the
/// prompt under ~5k tokens for fast turnaround.
pub struct QuickReviewerTemplate;

static QUICK_REVIEWER_ROLE_IDENTITY: &str = "\
You are the Quick Reviewer. Do a focused single-pass review of this implementation.\n\
\n\
## Scope (check ONLY these)\n\
\n\
1. **Correctness** — Does the implementation satisfy every acceptance criterion \
in the plan? Are there logic errors, off-by-ones, missing cases?\n\
2. **API alignment** — Do all cross-crate type signatures match what other plans expect?\n\
3. **Compilation** — Would `cargo check --workspace` pass? (Check imports, missing \
derives, type mismatches.)\n\
4. **Blocking omissions** — Are any required files entirely missing?\n\
\n\
Do NOT comment on: code style, docs, naming conventions, performance, or non-blocking nits.\n\
\n\
Keep the entire review under 500 words.\n\
Operate autonomously. Do not ask questions.";

impl RolePromptTemplate for QuickReviewerTemplate {
    type Input = QuickReviewerInput;

    fn sections(&self, input: &Self::Input) -> Vec<PromptSection> {
        let budget = budget_for(AgentRole::QuickReviewer);
        let mut sections = Vec::with_capacity(6);

        // 1. agents_instructions — System / Critical / Start
        sections.push(
            PromptSection::new("agents_instructions", &input.agents_md)
                .with_priority(SectionPriority::Critical)
                .with_cache_layer(CacheLayer::Role)
                .with_placement(Placement::Start),
        );

        // 2. plan_spec — Session / Critical / Start / hard_cap 50k
        sections.push(
            PromptSection::new("plan_spec", truncate(&input.plan.content, budget.plan))
                .with_priority(SectionPriority::Critical)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Start)
                .with_hard_cap(budget.plan),
        );

        // 3. workspace_map — Session / High / Middle / hard_cap 6k
        sections.push(
            PromptSection::new(
                "workspace_map",
                truncate(&input.workspace_map, budget.workspace_map),
            )
            .with_priority(SectionPriority::High)
            .with_cache_layer(CacheLayer::Workspace)
            .with_placement(Placement::Middle)
            .with_hard_cap(budget.workspace_map),
        );

        // 4. brief — Session / High / Middle / hard_cap 4k
        sections.push(
            PromptSection::new("brief", truncate(&input.brief, budget.brief))
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Middle)
                .with_hard_cap(budget.brief),
        );

        // 5. prior_review — Dynamic / High / End / hard_cap 3k (only on iteration 2+)
        if input.iteration > 1 {
            if let Some(ref review) = input.prior_review {
                let formatted = format_prior_review(&truncate(review, budget.reviews));
                sections.push(
                    PromptSection::new("prior_review", formatted)
                        .with_priority(SectionPriority::High)
                        .with_cache_layer(CacheLayer::Volatile)
                        .with_placement(Placement::End)
                        .with_hard_cap(budget.reviews),
                );
            }
        }

        // 6. verdict_instructions — System / Critical / End
        let verdict = format_verdict_instructions(&input.plan.num);
        sections.push(
            PromptSection::new("verdict_instructions", verdict)
                .with_priority(SectionPriority::Critical)
                .with_cache_layer(CacheLayer::Role)
                .with_placement(Placement::End),
        );

        sections
    }

    fn role_identity(&self) -> &'static str {
        QUICK_REVIEWER_ROLE_IDENTITY
    }
}

// ─── Quick Fix ───────────────────────────────────────────────────────────────

/// Typed input for the quick fix template.
#[derive(Clone, Debug, Default)]
pub struct QuickFixInput {
    /// Plan number (for writing selfcheck artifacts).
    pub plan_num: String,
    /// Compressed feedback listing the specific issues to fix.
    pub compressed_feedback: String,
}

/// Quick fix prompt template.
///
/// Minimal prompt for fixing specific issues identified by a reviewer.
/// Does NOT re-read the plan, workspace map, or PRD2 — only includes
/// the compressed feedback and fix directives. This keeps the prompt
/// under ~1k tokens for maximum speed.
pub struct QuickFixTemplate;

static QUICK_FIX_ROLE_IDENTITY: &str = "\
You are the Quick-Fixer. Your ONLY job is to fix the specific issues listed below.\n\
Do NOT re-read the plan. Do NOT re-implement anything. Do NOT add features.\n\
\n\
Instructions:\n\
1. For each issue listed, open the file and fix it.\n\
2. Run `cargo check --workspace` after all fixes.\n\
3. Run `cargo fmt` on any files you touched.\n\
4. Write results to the selfcheck artifact.\n\
\n\
That's it. Fix, check, done.\n\
\n\
IMPORTANT: This is a fully autonomous pipeline. Do NOT ask questions. \
Just fix the listed issues and end your turn.";

impl RolePromptTemplate for QuickFixTemplate {
    type Input = QuickFixInput;

    fn sections(&self, input: &Self::Input) -> Vec<PromptSection> {
        let mut sections = Vec::with_capacity(3);

        // 1. fix_directive — Task / Critical / Start
        let directive = format!("## Fix Directive\n\n{}\n", input.compressed_feedback);
        sections.push(
            PromptSection::new("fix_directive", directive)
                .with_priority(SectionPriority::Critical)
                .with_cache_layer(CacheLayer::Plan)
                .with_placement(Placement::Start),
        );

        // 2. selfcheck_instructions — System / High / End
        let selfcheck = format!(
            "## Output\n\n\
             Write results to `.mori/plans/completion/{plan_num}-selfcheck.toml` \
             (fallback: `plans/context/completion/{plan_num}-selfcheck.toml`).\n",
            plan_num = input.plan_num,
        );
        sections.push(
            PromptSection::new("selfcheck_instructions", selfcheck)
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Role)
                .with_placement(Placement::End),
        );

        sections
    }

    fn role_identity(&self) -> &'static str {
        QUICK_FIX_ROLE_IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Quick Reviewer tests ─────────────────────────────────────────────

    fn full_reviewer_input() -> QuickReviewerInput {
        QuickReviewerInput {
            agents_md: "# AGENTS.md\nFollow conventions.".into(),
            plan: PlanSlice {
                num: "042".into(),
                base: "golem-mortality".into(),
                title: "Implement mortality model".into(),
                content: "## Plan\nBuild the mortality model.".into(),
            },
            workspace_map: "crates/golem-core/src/lib.rs".into(),
            brief: "Strategist brief content.".into(),
            iteration: 2,
            prior_review: Some("[B-1] Missing error handling.".into()),
        }
    }

    #[test]
    fn render_golden_quick_reviewer_full() {
        let template = QuickReviewerTemplate;
        let sections = template.sections(&full_reviewer_input());

        // 6 sections: agents, plan_spec, workspace_map, brief, prior_review, verdict
        assert_eq!(sections.len(), 6);

        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            &[
                "agents_instructions",
                "plan_spec",
                "workspace_map",
                "brief",
                "prior_review",
                "verdict_instructions",
            ]
        );

        // Critical sections
        assert_eq!(sections[0].priority, SectionPriority::Critical);
        assert_eq!(sections[1].priority, SectionPriority::Critical);
        assert_eq!(sections[5].priority, SectionPriority::Critical);

        // Cache layers
        assert_eq!(sections[0].cache_layer, CacheLayer::Role);
        assert_eq!(sections[1].cache_layer, CacheLayer::Workspace);
        assert_eq!(sections[4].cache_layer, CacheLayer::Volatile);
        assert_eq!(sections[5].cache_layer, CacheLayer::Role);

        // Hard caps — quick reviewer uses tight budgets
        assert_eq!(sections[1].hard_cap, Some(50_000));
        assert_eq!(sections[2].hard_cap, Some(6_000));
        assert_eq!(sections[3].hard_cap, Some(4_000));
        assert_eq!(sections[4].hard_cap, Some(3_000));

        // Prior review contains the feedback
        assert!(sections[4].content.contains("Missing error handling"));
        assert!(sections[4].content.contains("Do NOT re-raise"));

        // Verdict instructions include plan number
        assert!(sections[5].content.contains("042"));
    }

    #[test]
    fn quick_reviewer_iteration_1_omits_prior_review() {
        let template = QuickReviewerTemplate;
        let mut input = full_reviewer_input();
        input.iteration = 1;
        input.prior_review = Some("Should be ignored on iter 1.".into());
        let sections = template.sections(&input);

        // 5 sections (no prior_review)
        assert_eq!(sections.len(), 5);
        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert!(!names.contains(&"prior_review"));
    }

    #[test]
    fn quick_reviewer_empty_ctx_minimal_sections() {
        let template = QuickReviewerTemplate;
        let input = QuickReviewerInput {
            agents_md: "agents".into(),
            plan: PlanSlice {
                num: "001".into(),
                content: "plan".into(),
                ..Default::default()
            },
            workspace_map: "map".into(),
            brief: String::new(),
            iteration: 1,
            prior_review: None,
        };
        let sections = template.sections(&input);

        // 5 base sections: agents, plan_spec, workspace_map, brief, verdict
        assert_eq!(sections.len(), 5);
        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert!(!names.contains(&"prior_review"));
    }

    #[test]
    fn quick_reviewer_truncates_oversized_workspace() {
        let template = QuickReviewerTemplate;
        let mut input = full_reviewer_input();
        input.workspace_map = "x".repeat(20_000);
        let sections = template.sections(&input);
        let ws = sections.iter().find(|s| s.name == "workspace_map").unwrap();
        assert!(ws.content.len() < 7_000);
        assert!(ws.content.contains("truncated"));
    }

    #[test]
    fn quick_reviewer_determinism() {
        let template = QuickReviewerTemplate;
        let input = full_reviewer_input();
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
    fn quick_reviewer_role_identity_is_substantial() {
        let template = QuickReviewerTemplate;
        let id = template.role_identity();
        assert!(id.len() >= 500);
        assert!(id.contains("Quick Reviewer"));
        assert!(id.contains("Correctness"));
        assert!(id.contains("Do NOT comment on"));
    }

    // ── Quick Fix tests ──────────────────────────────────────────────────

    fn full_fix_input() -> QuickFixInput {
        QuickFixInput {
            plan_num: "042".into(),
            compressed_feedback: "[B-1] Fix error handling in compute_rate (mortality.rs:42).\n\
                                  [B-2] Add missing derive(Clone) on MortalityRate."
                .into(),
        }
    }

    #[test]
    fn render_golden_quick_fix_full() {
        let template = QuickFixTemplate;
        let sections = template.sections(&full_fix_input());

        // 2 sections: fix_directive, selfcheck_instructions
        assert_eq!(sections.len(), 2);

        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, &["fix_directive", "selfcheck_instructions"]);

        // fix_directive is Critical (must not be dropped)
        assert_eq!(sections[0].priority, SectionPriority::Critical);
        assert_eq!(sections[0].cache_layer, CacheLayer::Plan);

        // Contains the feedback
        assert!(sections[0].content.contains("compute_rate"));
        assert!(sections[0].content.contains("MortalityRate"));

        // Selfcheck instructions contain plan number
        assert!(sections[1].content.contains("042-selfcheck.toml"));
    }

    #[test]
    fn quick_fix_determinism() {
        let template = QuickFixTemplate;
        let input = full_fix_input();
        let s1 = template.sections(&input);
        let s2 = template.sections(&input);
        assert_eq!(s1.len(), s2.len());
        for (a, b) in s1.iter().zip(s2.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.content, b.content);
            assert_eq!(a.priority, b.priority);
        }
    }

    #[test]
    fn quick_fix_role_identity_is_substantial() {
        let template = QuickFixTemplate;
        let id = template.role_identity();
        assert!(id.len() >= 300);
        assert!(id.contains("Quick-Fixer"));
        assert!(id.contains("Do NOT re-read the plan"));
        assert!(id.contains("autonomous"));
    }
}
