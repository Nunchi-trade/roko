//! Prompt assembly — combines role template sections into a final [`PromptBuild`].
//!
//! The [`PromptAssembler`] bridges individual role templates (which produce
//! `Vec<PromptSection>`) and the final prompt text + metadata. It handles:
//!
//! - Role identity injection as a leading section
//! - Optional common stanza injection (context layout, MCP tools)
//! - Budget enforcement with priority-based dropping
//! - U-shaped placement ordering (Start → Middle → End)
//! - Metadata tracking (sections kept/dropped, token count, strategy)
//!
//! Roko-owned reusable prompt assembler for role sections.

use super::RolePromptTemplate;
use super::common::{CONTEXT_LAYOUT_STANZA, MCP_TOOLS_STANZA};
use crate::prompt::{
    CacheLayer, ContextStrategy, Placement, PromptBuild, PromptSection, SectionPriority,
    estimate_tokens,
};

/// High-level prompt assembler.
///
/// Takes sections from any [`RolePromptTemplate`], optionally injects common
/// stanzas, applies a token budget, and returns a [`PromptBuild`] with full
/// metadata. This is the convenience layer between role templates and the
/// final prompt text.
///
/// # Example
///
/// ```ignore
/// let assembler = PromptAssembler::new();
/// let template = ImplementerTemplate;
/// let sections = template.sections(&input);
/// let build = assembler.assemble(
///     template.role_identity(),
///     sections,
///     Some(100_000),
///     ContextStrategy::Full,
/// );
/// assert!(!build.prompt.is_empty());
/// ```
pub struct PromptAssembler {
    inject_context_layout: bool,
    inject_mcp_tools: bool,
    include_headers: bool,
}

impl Default for PromptAssembler {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptAssembler {
    /// Create a default assembler with all injections enabled.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inject_context_layout: true,
            inject_mcp_tools: true,
            include_headers: false,
        }
    }

    /// Disable context layout stanza injection.
    #[must_use]
    pub const fn without_context_layout(mut self) -> Self {
        self.inject_context_layout = false;
        self
    }

    /// Disable MCP tools stanza injection.
    #[must_use]
    pub const fn without_mcp_tools(mut self) -> Self {
        self.inject_mcp_tools = false;
        self
    }

    /// Enable section header markers (`--- name ---`) in the output.
    #[must_use]
    pub const fn with_headers(mut self) -> Self {
        self.include_headers = true;
        self
    }

    /// Assemble a prompt from a role template's output.
    ///
    /// 1. Prepends the role identity as a Critical/System/Start section.
    /// 2. Optionally injects context layout and MCP tools stanzas.
    /// 3. Applies the token budget: Critical sections are always kept;
    ///    optional sections are included in priority order until budget
    ///    is exhausted.
    /// 4. Orders sections by placement (Start → Middle → End).
    /// 5. Returns a [`PromptBuild`] with the assembled text and metadata.
    #[must_use]
    pub fn assemble(
        &self,
        role_identity: &str,
        mut sections: Vec<PromptSection>,
        budget_tokens: Option<usize>,
        strategy: ContextStrategy,
    ) -> PromptBuild {
        // Prepend role identity.
        sections.insert(
            0,
            PromptSection::new("role_identity", role_identity)
                .with_priority(SectionPriority::Critical)
                .with_cache_layer(CacheLayer::Role)
                .with_placement(Placement::Start),
        );

        // Inject common stanzas.
        if self.inject_context_layout {
            sections.push(
                PromptSection::new("context_layout", CONTEXT_LAYOUT_STANZA)
                    .with_priority(SectionPriority::Low)
                    .with_cache_layer(CacheLayer::Role)
                    .with_placement(Placement::Middle),
            );
        }
        if self.inject_mcp_tools {
            sections.push(
                PromptSection::new("mcp_tools", MCP_TOOLS_STANZA)
                    .with_priority(SectionPriority::Low)
                    .with_cache_layer(CacheLayer::Role)
                    .with_placement(Placement::Middle),
            );
        }

        let total_sections = sections.len();
        let all_sections = sections.clone();

        // Enforce per-section hard caps, then split critical vs optional.
        let (critical, mut optional): (Vec<_>, Vec<_>) = sections
            .into_iter()
            .map(PromptSection::enforce_hard_cap)
            .partition(|s| s.priority == SectionPriority::Critical);

        let critical_tokens: usize = critical.iter().map(|s| estimate_tokens(&s.content)).sum();

        // Sort optional by priority DESC (High first, Low last).
        optional.sort_by_key(|s| std::cmp::Reverse(s.priority as u8));

        // Greedy inclusion under budget.
        let mut kept = critical;
        let mut token_total = critical_tokens;

        for section in optional {
            let toks = estimate_tokens(&section.content);
            if let Some(max) = budget_tokens {
                if token_total.saturating_add(toks) > max {
                    continue;
                }
            }
            token_total += toks;
            kept.push(section);
        }

        let sections_kept = kept.len();
        let sections_dropped = total_sections - sections_kept;
        let mut kept_ids = kept
            .iter()
            .map(PromptSection::stable_section_id)
            .collect::<Vec<_>>();
        let mut section_metadata = Vec::with_capacity(total_sections);
        for section in &all_sections {
            let capped = section.clone().enforce_hard_cap();
            let section_id = capped.stable_section_id();
            if let Some(index) = kept_ids.iter().position(|id| id == &section_id) {
                kept_ids.remove(index);
                section_metadata.push(capped.audit_row(
                    true,
                    capped.estimated_tokens(),
                    "included_by_prompt_assembler",
                ));
            } else {
                section_metadata.push(capped.audit_row(false, 0, "dropped_by_token_budget"));
            }
        }

        // Order by placement (U-shape): Start → Middle → End.
        kept.sort_by_key(|s| placement_order(s.placement));

        // Concatenate.
        let prompt = render(&kept, self.include_headers);

        PromptBuild::new(prompt)
            .with_strategy(strategy)
            .with_section_counts(sections_kept, sections_dropped)
            .with_section_metadata(section_metadata)
    }

    /// Convenience: call `template.sections(input)` then `assemble()`.
    #[must_use]
    pub fn assemble_from<T: RolePromptTemplate>(
        &self,
        template: &T,
        input: &T::Input,
        budget_tokens: Option<usize>,
        strategy: ContextStrategy,
    ) -> PromptBuild {
        let sections = template.sections(input);
        self.assemble(template.role_identity(), sections, budget_tokens, strategy)
    }
}

const fn placement_order(p: Placement) -> u8 {
    match p {
        Placement::Start => 0,
        Placement::Middle => 1,
        Placement::End => 2,
    }
}

fn render(sections: &[PromptSection], headers: bool) -> String {
    let mut out = String::new();
    for section in sections {
        if headers {
            out.push_str("--- ");
            out.push_str(&section.name);
            out.push_str(" ---\n");
        }
        out.push_str(&section.content);
        if !section.content.ends_with('\n') {
            out.push('\n');
        }
        if headers {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::{PlanSlice, QuickReviewerInput, QuickReviewerTemplate};
    use super::*;

    #[test]
    fn render_golden_full_assembly() {
        let assembler = PromptAssembler::new();
        let template = QuickReviewerTemplate;
        let input = QuickReviewerInput {
            agents_md: "# AGENTS.md\nFollow conventions.".into(),
            plan: PlanSlice {
                num: "042".into(),
                base: "golem-mortality".into(),
                title: "Implement mortality model".into(),
                content: "## Plan\nBuild the mortality model.".into(),
            },
            workspace_map: "crates/golem-core/src/lib.rs".into(),
            brief: "Strategist brief content.".into(),
            iteration: 1,
            prior_review: None,
        };

        let build = assembler.assemble_from(&template, &input, None, ContextStrategy::Full);

        // Prompt contains role identity, plan content, and common stanzas
        assert!(build.prompt.contains("Quick Reviewer"));
        assert!(build.prompt.contains("Build the mortality model"));
        assert!(build.prompt.contains("Plans context layout"));
        assert!(build.prompt.contains("MCP"));

        // Metadata
        assert_eq!(build.context_strategy, ContextStrategy::Full);
        assert!(build.tokens > 0);
        // 5 template sections + 1 role identity + 2 common stanzas = 8
        assert_eq!(build.sections_kept, 8);
        assert_eq!(build.sections_dropped, 0);
    }

    #[test]
    fn assembly_without_common_stanzas() {
        let assembler = PromptAssembler::new()
            .without_context_layout()
            .without_mcp_tools();

        let sections =
            vec![PromptSection::new("task", "implement X").with_priority(SectionPriority::High)];

        let build = assembler.assemble(
            "You are a test agent.",
            sections,
            None,
            ContextStrategy::Full,
        );

        assert!(build.prompt.contains("You are a test agent"));
        assert!(build.prompt.contains("implement X"));
        assert!(!build.prompt.contains("Plans context layout"));
        assert!(!build.prompt.contains("MCP"));
        // 1 role identity + 1 task = 2
        assert_eq!(build.sections_kept, 2);
    }

    #[test]
    fn assembly_drops_low_priority_under_budget() {
        let assembler = PromptAssembler::new()
            .without_context_layout()
            .without_mcp_tools();

        let sections = vec![
            PromptSection::new("important", "keep this").with_priority(SectionPriority::Critical),
            PromptSection::new("filler", &"x".repeat(4000)).with_priority(SectionPriority::Low),
        ];

        // Budget of 30 tokens (~120 chars) — role identity + "keep this" fits,
        // but the 4000-char filler does not.
        let build = assembler.assemble("Agent", sections, Some(30), ContextStrategy::Trimmed);

        assert!(build.prompt.contains("keep this"));
        assert!(!build.prompt.contains("xxxx"));
        assert_eq!(build.context_strategy, ContextStrategy::Trimmed);
        // role_identity (Critical) + important (Critical) kept; filler dropped
        assert_eq!(build.sections_kept, 2);
        assert_eq!(build.sections_dropped, 1);
        let filler = build
            .section_metadata
            .iter()
            .find(|section| section.section_name == "filler")
            .expect("filler section metadata should be recorded");
        assert!(!filler.included);
        assert_eq!(filler.tokens_used, 0);
        assert_eq!(filler.action_id, "prompt_section:filler");
        assert!(
            !serde_json::to_string(&build.section_metadata)
                .expect("metadata serializes")
                .contains(&"x".repeat(100))
        );
    }

    #[test]
    fn assembly_preserves_u_shape_ordering() {
        let assembler = PromptAssembler::new()
            .without_context_layout()
            .without_mcp_tools();

        let sections = vec![
            PromptSection::new("end_section", "I am end").with_placement(Placement::End),
            PromptSection::new("middle_section", "I am middle").with_placement(Placement::Middle),
        ];

        let build = assembler.assemble("Role start", sections, None, ContextStrategy::Full);

        // Role identity is Start, then middle, then end
        let start_pos = build.prompt.find("Role start").unwrap();
        let middle_pos = build.prompt.find("I am middle").unwrap();
        let end_pos = build.prompt.find("I am end").unwrap();
        assert!(start_pos < middle_pos);
        assert!(middle_pos < end_pos);
    }

    #[test]
    fn assembly_with_headers() {
        let assembler = PromptAssembler::new()
            .without_context_layout()
            .without_mcp_tools()
            .with_headers();

        let sections =
            vec![PromptSection::new("task", "do the thing").with_priority(SectionPriority::High)];

        let build = assembler.assemble("Agent role.", sections, None, ContextStrategy::Full);

        assert!(build.prompt.contains("--- role_identity ---"));
        assert!(build.prompt.contains("--- task ---"));
    }

    #[test]
    fn assembly_enforces_hard_caps() {
        let assembler = PromptAssembler::new()
            .without_context_layout()
            .without_mcp_tools();

        let sections = vec![
            PromptSection::new("bounded", "a".repeat(400))
                .with_priority(SectionPriority::High)
                .with_hard_cap(5),
        ];

        let build = assembler.assemble("Role", sections, None, ContextStrategy::Full);

        // The 400-char section should be truncated to ~20 bytes + marker
        assert!(build.prompt.contains("[truncated"));
        // Total prompt is well under 400 chars
        assert!(build.prompt.len() < 100);
    }

    #[test]
    fn assembly_determinism() {
        let assembler = PromptAssembler::new();
        let sections1 = vec![
            PromptSection::new("a", "content a").with_priority(SectionPriority::High),
            PromptSection::new("b", "content b").with_priority(SectionPriority::Normal),
        ];
        let sections2 = sections1.clone();

        let b1 = assembler.assemble("Role", sections1, None, ContextStrategy::Full);
        let b2 = assembler.assemble("Role", sections2, None, ContextStrategy::Full);
        assert_eq!(b1.prompt, b2.prompt);
        assert_eq!(b1.tokens, b2.tokens);
        assert_eq!(b1.sections_kept, b2.sections_kept);
    }

    #[test]
    fn assembly_retry_strategy() {
        let assembler = PromptAssembler::new()
            .without_context_layout()
            .without_mcp_tools();

        let sections = vec![
            PromptSection::new("error_digest", "Previous errors: ...")
                .with_priority(SectionPriority::High),
        ];

        let build = assembler.assemble("Retry agent.", sections, None, ContextStrategy::Retry);

        assert_eq!(build.context_strategy, ContextStrategy::Retry);
        assert!(build.prompt.contains("Previous errors"));
    }
}
