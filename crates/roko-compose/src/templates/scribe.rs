//! Scribe prompt template.
//!
//! Roko-owned scribe, doc revision, and critic prompt templates. The Critic is
//! treated as a scribe-variant (same section set, different role identity).

use super::common::{self, REFERENCE_CONTEXT_WINDOW_TOKENS, adaptive_budget_for};
use super::{PlanSlice, RolePromptTemplate, truncate};
use crate::prompt::{CacheLayer, Placement, PromptSection, SectionPriority};
use roko_core::AgentRole;

/// A source file snippet for the scribe to document.
#[derive(Clone, Debug)]
pub struct FileSnippet {
    /// Relative file path (e.g. "crates/golem-core/src/mortality.rs").
    pub path: String,
    /// The source code content.
    pub content: String,
}

/// Which scribe variant to generate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScribeVariant {
    /// Initial documentation generation.
    #[default]
    Initial,
    /// Revision pass with critic feedback.
    Revision,
    /// Critic — reviews the scribe's output.
    Critic,
}

/// Typed input for the scribe template.
#[derive(Clone, Debug, Default)]
pub struct ScribeInput {
    /// AGENTS.md content.
    pub agents_md: String,
    /// Plan metadata + full content.
    pub plan: PlanSlice,
    /// PRD2 specification extract (up to 16k — the largest prd2 budget).
    pub prd2_extract: String,
    /// Strategist brief.
    pub brief: String,
    /// Source file snippets the scribe should document.
    pub source_snippets: Vec<FileSnippet>,
    /// Which scribe variant to produce.
    pub variant: ScribeVariant,
    /// Critic feedback (only for Revision variant).
    pub critic_feedback: Option<String>,
    /// Scribe's prior documentation output (only for Critic variant).
    pub prior_docs: Option<String>,
}

/// Scribe prompt template.
///
/// Generates documentation from plan + source code. The Critic variant
/// reviews documentation quality.
pub struct ScribeTemplate;

static SCRIBE_ROLE_IDENTITY: &str = "\
You are the Scribe. Write reference documentation for the implementation.\n\
\n\
Your reader has never seen this codebase. They do not know what the system \
does, why it exists, or what decisions shaped it. Everything must be \
self-contained.\n\
\n\
Rules:\n\
1. Open with the problem, not the solution. Never start with \"This module provides...\"\n\
2. Frame the plan as a single coherent story before documenting individual modules.\n\
3. Include a Mermaid graph TD diagram showing how modules relate.\n\
4. Every formula must cite the PRD2 file path, section, and academic source.\n\
5. Every state machine gets a stateDiagram-v2. Every multi-step flow gets a sequenceDiagram.\n\
6. Minimum 4 numbered, captioned Mermaid diagrams per plan.\n\
7. No diagram exceeds 15 nodes — split complex ones into sub-diagrams.\n\
8. All 7 required sections must be present: Context, Architecture, Concepts, API, \
Implementation, Cross-Module, Testing.\n\
9. Preserve the full depth of PRD2 source material. Do not truncate or simplify.\n\
10. Operate autonomously. Do not ask questions.";

static CRITIC_ROLE_IDENTITY: &str = "\
You are the Critic. Review the Scribe's documentation for quality and \
spec fidelity.\n\
\n\
Check each of these and report ALL failures:\n\
- Completeness: every public type, function, and trait is documented\n\
- Accuracy: type signatures, parameter ranges, return values match source\n\
- PRD2 Fidelity: every formula, threshold, constant appears with correct citation\n\
- Depth: docs explain WHY, not just WHAT\n\
- Cross-references: data flow, event sequences, type contracts documented\n\
- Holistic Narrative: plan-level overview present, not just a table of contents\n\
- Visual Documentation: minimum 4 Mermaid diagrams, numbered and captioned\n\
- Voice and Style: no AI writing patterns (delve, tapestry, robust, seamless)\n\
\n\
Be exhaustive. Report ALL issues in one pass. Only REVISE for missing public \
API documentation, wrong citations, missing required sections, or pervasive \
AI writing patterns.\n\
\n\
Operate autonomously. Do not ask questions.";

static REVISION_ROLE_IDENTITY: &str = "\
You are the Scribe (revision pass). Fix the issues identified by the Critic \
in your documentation.\n\
\n\
Read the critic's feedback carefully. Address every numbered item. Do not \
re-write from scratch — make targeted fixes to the existing documentation.\n\
\n\
Operate autonomously. Do not ask questions.";

impl RolePromptTemplate for ScribeTemplate {
    type Input = ScribeInput;

    fn sections(&self, input: &Self::Input) -> Vec<PromptSection> {
        self.sections_with_context_window(input, REFERENCE_CONTEXT_WINDOW_TOKENS)
    }

    fn sections_with_context_window(
        &self,
        input: &Self::Input,
        context_window_tokens: usize,
    ) -> Vec<PromptSection> {
        let budget = match input.variant {
            ScribeVariant::Critic => adaptive_budget_for(AgentRole::Critic, context_window_tokens),
            ScribeVariant::Initial | ScribeVariant::Revision => {
                adaptive_budget_for(AgentRole::Scribe, context_window_tokens)
            }
        };
        let mut sections = Vec::with_capacity(8);

        // 1. agents_instructions — System / Critical / Start
        sections.push(common::agents_instructions_section(&input.agents_md));

        // 2. plan_spec — Session / Critical / hard_cap 50k
        sections.push(
            PromptSection::new("plan_spec", truncate(&input.plan.content, budget.plan))
                .with_priority(SectionPriority::Critical)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Start)
                .with_hard_cap(budget.plan),
        );

        // 3. prd2_extract — Session / High / hard_cap 16k (scribe gets the largest prd2 budget)
        sections.push(
            PromptSection::new("prd2_extract", truncate(&input.prd2_extract, budget.prd2))
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Middle)
                .with_hard_cap(budget.prd2),
        );

        // 4. brief — Session / High
        sections.push(
            PromptSection::new("brief", &input.brief)
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Middle),
        );

        // 5. file_context — Task / High (source snippets concatenated)
        if !input.source_snippets.is_empty() {
            let text = format_snippets(&input.source_snippets);
            sections.push(
                PromptSection::new("file_context", text)
                    .with_priority(SectionPriority::High)
                    .with_cache_layer(CacheLayer::Plan)
                    .with_placement(Placement::End),
            );
        }

        // 6. critic_feedback — Dynamic / High (only for Revision variant)
        if input.variant == ScribeVariant::Revision {
            if let Some(ref feedback) = input.critic_feedback {
                sections.push(
                    PromptSection::new("critic_feedback", feedback.as_str())
                        .with_priority(SectionPriority::High)
                        .with_cache_layer(CacheLayer::Volatile)
                        .with_placement(Placement::End),
                );
            }
        }

        // 7. prior_docs — Task / High (only for Critic variant — the docs to review)
        if input.variant == ScribeVariant::Critic {
            if let Some(ref docs) = input.prior_docs {
                sections.push(
                    PromptSection::new("prior_docs", docs.as_str())
                        .with_priority(SectionPriority::High)
                        .with_cache_layer(CacheLayer::Plan)
                        .with_placement(Placement::End),
                );
            }
        }

        sections
    }

    fn role_identity(&self) -> &'static str {
        // Default to Initial — callers using Revision/Critic should check
        // `ScribeVariant` on the input. This method returns the initial identity
        // for trait compliance; variant-specific identity is available via
        // `role_identity_for_variant`.
        SCRIBE_ROLE_IDENTITY
    }
}

impl ScribeTemplate {
    /// Get the role identity for a specific scribe variant.
    #[must_use]
    pub fn role_identity_for_variant(variant: ScribeVariant) -> &'static str {
        match variant {
            ScribeVariant::Initial => SCRIBE_ROLE_IDENTITY,
            ScribeVariant::Revision => REVISION_ROLE_IDENTITY,
            ScribeVariant::Critic => CRITIC_ROLE_IDENTITY,
        }
    }
}

fn format_snippets(snippets: &[FileSnippet]) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    for snippet in snippets {
        let _ = write!(
            out,
            "### {}\n```\n{}\n```\n\n",
            snippet.path, snippet.content
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_input() -> ScribeInput {
        ScribeInput {
            agents_md: "# AGENTS.md\nConventions.".into(),
            plan: PlanSlice {
                num: "042".into(),
                base: "golem-mortality".into(),
                title: "Mortality model".into(),
                content: "## Plan\nDocument the mortality model.".into(),
            },
            prd2_extract: "## PRD2\nGompertz mortality: lambda(t) = ae^(bt).".into(),
            brief: "Brief about mortality module.".into(),
            source_snippets: vec![
                FileSnippet {
                    path: "crates/golem-core/src/mortality.rs".into(),
                    content: "pub fn compute_rate() -> f64 { 0.0 }".into(),
                },
                FileSnippet {
                    path: "crates/golem-core/src/lifecycle.rs".into(),
                    content: "pub struct Lifecycle;".into(),
                },
            ],
            variant: ScribeVariant::Initial,
            critic_feedback: None,
            prior_docs: None,
        }
    }

    #[test]
    fn render_golden_initial_scribe() {
        let template = ScribeTemplate;
        let sections = template.sections(&full_input());

        // 5 sections: agents_instructions, plan_spec, prd2_extract, brief, file_context
        assert_eq!(sections.len(), 5);

        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            &[
                "agents_instructions",
                "plan_spec",
                "prd2_extract",
                "brief",
                "file_context",
            ]
        );

        // Cache layers
        assert_eq!(sections[0].cache_layer, CacheLayer::Role);
        assert_eq!(sections[1].cache_layer, CacheLayer::Workspace);
        assert_eq!(sections[2].cache_layer, CacheLayer::Workspace);
        assert_eq!(sections[4].cache_layer, CacheLayer::Plan);

        // Hard caps — scribe gets 16k for prd2
        assert_eq!(sections[1].hard_cap, Some(50_000));
        assert_eq!(sections[2].hard_cap, Some(16_000));

        // file_context contains both snippets
        let fc = &sections[4].content;
        assert!(fc.contains("mortality.rs"));
        assert!(fc.contains("lifecycle.rs"));
        assert!(fc.contains("compute_rate"));
    }

    #[test]
    fn render_golden_critic_variant() {
        let template = ScribeTemplate;
        let mut input = full_input();
        input.variant = ScribeVariant::Critic;
        input.prior_docs = Some("# Documentation\nPrior scribe output.".into());
        let sections = template.sections(&input);

        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"prior_docs"));
        assert!(!names.contains(&"critic_feedback"));

        let docs_section = sections.iter().find(|s| s.name == "prior_docs").unwrap();
        assert!(docs_section.content.contains("Prior scribe output"));
    }

    #[test]
    fn render_golden_revision_variant() {
        let template = ScribeTemplate;
        let mut input = full_input();
        input.variant = ScribeVariant::Revision;
        input.critic_feedback = Some("1. Missing API docs for compute_rate.".into());
        let sections = template.sections(&input);

        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"critic_feedback"));
        assert!(!names.contains(&"prior_docs"));
    }

    #[test]
    fn budget_capped_render_truncates_oversized_prd2() {
        let template = ScribeTemplate;
        let mut input = full_input();
        input.prd2_extract = "x".repeat(50_000);
        let sections = template.sections(&input);
        let prd2 = sections.iter().find(|s| s.name == "prd2_extract").unwrap();
        // Should be truncated to ~16k + marker
        assert!(prd2.content.len() < 17_000);
        assert!(prd2.content.contains("truncated"));
    }

    #[test]
    fn empty_ctx_omits_optional_sections() {
        let template = ScribeTemplate;
        let input = ScribeInput {
            agents_md: "agents".into(),
            plan: PlanSlice {
                content: "plan".into(),
                ..Default::default()
            },
            prd2_extract: "prd2".into(),
            brief: String::new(),
            source_snippets: vec![],
            variant: ScribeVariant::Initial,
            critic_feedback: None,
            prior_docs: None,
        };
        let sections = template.sections(&input);

        // 4 sections: agents_instructions, plan_spec, prd2_extract, brief
        // No file_context (empty snippets), no critic_feedback, no prior_docs
        assert_eq!(sections.len(), 4);
        let names: Vec<&str> = sections.iter().map(|s| s.name.as_str()).collect();
        assert!(!names.contains(&"file_context"));
        assert!(!names.contains(&"critic_feedback"));
        assert!(!names.contains(&"prior_docs"));
    }

    #[test]
    fn determinism_identical_input_identical_output() {
        let template = ScribeTemplate;
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
    fn role_identity_variants_are_distinct() {
        let initial = ScribeTemplate::role_identity_for_variant(ScribeVariant::Initial);
        let revision = ScribeTemplate::role_identity_for_variant(ScribeVariant::Revision);
        let critic = ScribeTemplate::role_identity_for_variant(ScribeVariant::Critic);
        assert!(initial.contains("Scribe"));
        assert!(revision.contains("revision"));
        assert!(critic.contains("Critic"));
        assert_ne!(initial, revision);
        assert_ne!(initial, critic);
        // All substantial
        assert!(initial.len() >= 500);
        assert!(critic.len() >= 500);
    }
}
