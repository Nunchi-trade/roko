//! Per-role, complexity-adaptive prompt budgets with cache alignment markers.
//!
//! Extends the existing [`PromptBudget`](crate::templates::common::PromptBudget)
//! and [`budget_for`](crate::templates::common::budget_for) with:
//!
//! - Complexity-based budget scaling (Fast/Standard/Complex)
//! - Section dropping for trivial tasks (PRD, research, decomposition)
//! - Prefix cache alignment markers (hints for LLM cache break points)
//!
//! The base per-role budgets live in `templates/common.rs` and are re-used
//! here. This module adds the complexity dimension on top.
//!
//! Anti-pattern #8: **no `std::fs`**. All content arrives via parameters.

use crate::templates::common::{PromptBudget, adaptive_budget_for, budget_for};
use roko_core::AgentRole;

/// Which complexity band the current plan/task falls into.
///
/// This is a local enum mirroring `TaskComplexityBand` from roko-core but
/// oriented toward prompt budget decisions rather than task routing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Complexity {
    /// Single-file, trivial change. Drop PRD, research, decomposition sections.
    Trivial,
    /// Standard multi-file task. Full budget at role defaults.
    #[default]
    Standard,
    /// Cross-crate or architectural work. Inflated budgets for context.
    Complex,
}

/// A complexity-adjusted budget derived from the base per-role budget.
///
/// Contains the 9 section caps from [`PromptBudget`] plus metadata about
/// which sections were dropped and where cache breaks should go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdjustedBudget {
    /// The per-section character caps (possibly scaled or zeroed).
    pub budget: PromptBudget,
    /// Sections that were dropped entirely for this complexity level.
    pub dropped_sections: Vec<&'static str>,
    /// Suggested cache break points (section names after which a cache
    /// layer boundary should be inserted). Aligns with the 4-tier
    /// `CacheLayer` model in `prompt.rs`.
    pub cache_breaks: Vec<&'static str>,
    /// The complexity band that produced this budget.
    pub complexity: Complexity,
    /// The role that produced the base budget.
    pub role: AgentRole,
}

/// Compute a complexity-adjusted budget for a given role.
///
/// # Algorithm
///
/// 1. Start with the base per-role budget from `budget_for(role)`.
/// 2. Apply complexity adjustments:
///    - **Trivial**: zero out `prd2`, `context`, `skills`; halve `workspace_map`
///      and `brief`. These sections add noise to simple single-file tasks.
///    - **Standard**: use the base budget as-is.
///    - **Complex**: inflate `workspace_map` by 50%, `context` by 100%,
///      `file_context` by 50%. Complex tasks need more spatial awareness.
/// 3. Compute cache break hints based on section stability tiers.
#[must_use]
pub fn adjusted_budget_for(role: AgentRole, complexity: Complexity) -> AdjustedBudget {
    adjusted_budget_from_base(role, budget_for(role), complexity)
}

/// Compute a context-window-aware, complexity-adjusted budget for a role.
#[must_use]
pub fn adjusted_adaptive_budget_for(
    role: AgentRole,
    complexity: Complexity,
    context_window_tokens: usize,
) -> AdjustedBudget {
    adjusted_budget_from_base(
        role,
        adaptive_budget_for(role, context_window_tokens),
        complexity,
    )
}

fn adjusted_budget_from_base(
    role: AgentRole,
    mut budget: PromptBudget,
    complexity: Complexity,
) -> AdjustedBudget {
    let mut dropped = Vec::new();

    match complexity {
        Complexity::Trivial => {
            // Drop heavy context sections that add noise for trivial tasks.
            if budget.prd2 > 0 {
                budget.prd2 = 0;
                dropped.push("prd2");
            }
            if budget.context > 0 {
                budget.context = 0;
                dropped.push("context");
            }
            if budget.skills > 0 {
                budget.skills = 0;
                dropped.push("skills");
            }
            // Halve remaining large sections.
            budget.workspace_map /= 2;
            budget.brief /= 2;
        }
        Complexity::Standard => {
            // Use base budget as-is.
        }
        Complexity::Complex => {
            // Inflate sections that help with cross-crate awareness.
            budget.workspace_map = budget.workspace_map.saturating_mul(3) / 2;
            budget.context = budget.context.saturating_mul(2);
            budget.file_context = budget.file_context.saturating_mul(3) / 2;
        }
    }

    // Cache break hints: insert breaks after stable layers so the LLM
    // prefix cache can reuse the system/session prefix across turns.
    //
    // Layer boundaries (from CacheLayer enum):
    //   System  → role identity, agents.md, conventions
    //   Session → plan, workspace_map, brief
    //   Task    → tasks, file_context, enhancements
    //   Dynamic → reviews, error digest
    let cache_breaks = vec![
        "conventions",   // end of System layer
        "workspace_map", // end of Session layer
        "file_context",  // end of Task layer
    ];

    AdjustedBudget {
        budget,
        dropped_sections: dropped,
        cache_breaks,
        complexity,
        role,
    }
}

/// Total character budget across all 9 sections.
///
/// Useful for estimating whether a prompt will fit in a model's context window
/// when fully populated.
#[must_use]
pub const fn total_budget(budget: &PromptBudget) -> usize {
    budget.plan
        + budget.workspace_map
        + budget.prd2
        + budget.context
        + budget.brief
        + budget.reviews
        + budget.instructions
        + budget.file_context
        + budget.skills
}

/// Check whether a given section name is at a cache break boundary.
#[must_use]
pub fn is_cache_break(adjusted: &AdjustedBudget, section_name: &str) -> bool {
    adjusted.cache_breaks.contains(&section_name)
}

/// Format a cache alignment marker for prompt insertion.
///
/// Returns a string like `<!-- cache:session -->` that downstream prompt
/// renderers can detect and use to set `cache_control` on API calls.
#[must_use]
pub fn cache_marker(layer_name: &str) -> String {
    format!("<!-- cache:{layer_name} -->")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trivial_drops_prd_context_skills() {
        let adj = adjusted_budget_for(AgentRole::Implementer, Complexity::Trivial);
        assert_eq!(adj.budget.prd2, 0);
        assert_eq!(adj.budget.context, 0);
        assert_eq!(adj.budget.skills, 0);
        assert!(adj.dropped_sections.contains(&"prd2"));
        assert!(adj.dropped_sections.contains(&"context"));
        assert!(adj.dropped_sections.contains(&"skills"));
    }

    #[test]
    fn trivial_halves_workspace_and_brief() {
        let base = budget_for(AgentRole::Implementer);
        let adj = adjusted_budget_for(AgentRole::Implementer, Complexity::Trivial);
        assert_eq!(adj.budget.workspace_map, base.workspace_map / 2);
        assert_eq!(adj.budget.brief, base.brief / 2);
    }

    #[test]
    fn standard_matches_base() {
        let base = budget_for(AgentRole::Implementer);
        let adj = adjusted_budget_for(AgentRole::Implementer, Complexity::Standard);
        assert_eq!(adj.budget, base);
        assert!(adj.dropped_sections.is_empty());
    }

    #[test]
    fn complex_inflates_workspace_and_context() {
        let base = budget_for(AgentRole::Implementer);
        let adj = adjusted_budget_for(AgentRole::Implementer, Complexity::Complex);
        assert!(adj.budget.workspace_map > base.workspace_map);
        assert!(adj.budget.context > base.context);
        assert!(adj.budget.file_context > base.file_context);
        // Specifically: 50% inflation on workspace_map.
        assert_eq!(adj.budget.workspace_map, base.workspace_map * 3 / 2);
    }

    #[test]
    fn auto_fixer_trivial_drops_nothing_already_zero() {
        let adj = adjusted_budget_for(AgentRole::AutoFixer, Complexity::Trivial);
        // AutoFixer already has prd2=0, context=0, skills=0.
        assert!(adj.dropped_sections.is_empty());
        // Plan and brief are already 0 for AutoFixer.
        assert_eq!(adj.budget.plan, 0);
    }

    #[test]
    fn cache_breaks_present() {
        let adj = adjusted_budget_for(AgentRole::Implementer, Complexity::Standard);
        assert_eq!(adj.cache_breaks.len(), 3);
        assert!(is_cache_break(&adj, "conventions"));
        assert!(is_cache_break(&adj, "workspace_map"));
        assert!(is_cache_break(&adj, "file_context"));
        assert!(!is_cache_break(&adj, "plan"));
    }

    #[test]
    fn total_budget_sums_all_sections() {
        let b = budget_for(AgentRole::Implementer);
        let total = total_budget(&b);
        assert_eq!(
            total,
            b.plan
                + b.workspace_map
                + b.prd2
                + b.context
                + b.brief
                + b.reviews
                + b.instructions
                + b.file_context
                + b.skills
        );
    }

    #[test]
    fn cache_marker_format() {
        let marker = cache_marker("session");
        assert_eq!(marker, "<!-- cache:session -->");
    }

    #[test]
    fn per_role_budgets_differ() {
        let impl_budget = adjusted_budget_for(AgentRole::Implementer, Complexity::Standard);
        let rev_budget = adjusted_budget_for(AgentRole::QuickReviewer, Complexity::Standard);
        // Implementer gets more file_context than QuickReviewer.
        assert!(impl_budget.budget.file_context > rev_budget.budget.file_context);
        // QuickReviewer has no prd2 or context.
        assert_eq!(rev_budget.budget.prd2, 0);
    }

    #[test]
    fn reviewer_gets_reviews_budget() {
        let rev = adjusted_budget_for(AgentRole::QuickReviewer, Complexity::Standard);
        assert!(rev.budget.reviews > 0);
    }

    #[test]
    fn complexity_stored_in_result() {
        let adj = adjusted_budget_for(AgentRole::Strategist, Complexity::Complex);
        assert_eq!(adj.complexity, Complexity::Complex);
        assert_eq!(adj.role, AgentRole::Strategist);
    }

    #[test]
    fn scribe_complex_has_inflated_context() {
        let base = budget_for(AgentRole::Scribe);
        let adj = adjusted_budget_for(AgentRole::Scribe, Complexity::Complex);
        assert_eq!(adj.budget.context, base.context * 2);
    }
}
