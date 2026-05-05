//! Common prompt utilities shared across all role templates.
//!
//! Contains per-role token budgets, reusable stanza constants, and
//! formatting helpers that multiple templates reference. This module owns
//! Roko's budget tables, context layout, MCP tools, and verdict format in a
//! typed, I/O-free form.

use crate::prompt::{CacheLayer, Placement, PromptSection, SectionPriority};
use roko_core::AgentRole;

// ─── Per-role budgets ────────────────────────────────────────────────────────

/// Per-section character caps for a given agent role.
///
/// Per-role prompt budget caps. Each field is a maximum character count — the
/// template truncates content to fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptBudget {
    /// Plan markdown content cap.
    pub plan: usize,
    /// Workspace map (file tree) cap.
    pub workspace_map: usize,
    /// PRD2 specification extract cap.
    pub prd2: usize,
    /// Cross-plan context (CONTEXT.md) cap.
    pub context: usize,
    /// Strategist brief cap.
    pub brief: usize,
    /// Prior review feedback cap.
    pub reviews: usize,
    /// Instruction block cap.
    pub instructions: usize,
    /// Inline file context cap.
    pub file_context: usize,
    /// Playbook / skill library context cap.
    pub skills: usize,
}

/// Reference model context window used by the built-in static budget table.
///
/// `budget_for()` values were tuned around 200k-token Claude-class models.
/// Adaptive callers should pass the actual model context window to
/// [`adaptive_budget_for`] instead of relying on this reference size.
pub const REFERENCE_CONTEXT_WINDOW_TOKENS: usize = 200_000;

/// Return the per-section character budget for a given agent role.
///
/// Budget values are Roko's built-in cold-start defaults. Model-specific
/// budget tuning can layer on top of this function.
#[must_use]
pub const fn budget_for(role: AgentRole) -> PromptBudget {
    match role {
        AgentRole::Implementer => PromptBudget {
            plan: 50_000,
            workspace_map: 20_000,
            prd2: 12_000,
            context: 4_000,
            brief: 8_000,
            reviews: 3_000,
            instructions: 4_000,
            file_context: 8_000,
            skills: 8_000,
        },
        AgentRole::Strategist => PromptBudget {
            plan: 50_000,
            workspace_map: 20_000,
            prd2: 12_000,
            context: 4_000,
            brief: 6_000,
            reviews: 3_000,
            instructions: 4_000,
            file_context: 0,
            skills: 4_000,
        },
        AgentRole::Architect | AgentRole::Auditor => PromptBudget {
            plan: 50_000,
            workspace_map: 6_000,
            prd2: 6_000,
            context: 2_000,
            brief: 4_000,
            reviews: 3_000,
            instructions: 4_000,
            file_context: 6_000,
            skills: 4_000,
        },
        AgentRole::Scribe | AgentRole::Critic => PromptBudget {
            plan: 50_000,
            workspace_map: 6_000,
            prd2: 16_000,
            context: 4_000,
            brief: 6_000,
            reviews: 3_000,
            instructions: 4_000,
            file_context: 6_000,
            skills: 4_000,
        },
        AgentRole::QuickReviewer => PromptBudget {
            plan: 50_000,
            workspace_map: 6_000,
            prd2: 0,
            context: 0,
            brief: 4_000,
            reviews: 3_000,
            instructions: 2_000,
            file_context: 0,
            skills: 0,
        },
        AgentRole::AutoFixer => PromptBudget {
            plan: 0,
            workspace_map: 0,
            prd2: 0,
            context: 0,
            brief: 0,
            reviews: 0,
            instructions: 2_000,
            file_context: 0,
            skills: 0,
        },
        _ => PromptBudget {
            plan: 50_000,
            workspace_map: 8_000,
            prd2: 6_000,
            context: 4_000,
            brief: 4_000,
            reviews: 2_000,
            instructions: 4_000,
            file_context: 6_000,
            skills: 4_000,
        },
    }
}

// ─── Adaptive budgets ───────────────────────────────────────────────────────

/// Model-aware budget that scales with available context window.
///
/// Rather than fixed character caps, an `AdaptiveBudget` expresses a section
/// budget as a fraction of the model's context window, clamped between an
/// absolute minimum and maximum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdaptiveBudget {
    /// Fraction of total context tokens to allocate (0.0..=1.0).
    pub fraction: f64,
    /// Minimum character budget regardless of model size.
    pub min_chars: usize,
    /// Maximum character budget regardless of model size.
    pub max_chars: usize,
}

impl AdaptiveBudget {
    /// Create a new adaptive budget with the given fraction and clamp bounds.
    #[must_use]
    pub const fn new(fraction: f64, min_chars: usize, max_chars: usize) -> Self {
        Self {
            fraction,
            min_chars,
            max_chars,
        }
    }

    /// Compute the effective character budget for a model with `context_tokens` capacity.
    ///
    /// Assumes ~4 characters per token for the conversion.
    #[must_use]
    pub fn compute(&self, context_tokens: usize) -> usize {
        let chars_from_fraction = (context_tokens as f64 * 4.0 * self.fraction) as usize;
        chars_from_fraction.clamp(self.min_chars, self.max_chars)
    }
}

/// Return a role-aware adaptive budget scaled to the model's context window.
///
/// This complements [`budget_for`] by providing model-sensitive budgets. The
/// returned budget represents the total prompt section allocation for the role.
#[must_use]
pub fn adaptive_budget_for(role: AgentRole, model_context_tokens: usize) -> PromptBudget {
    let base = budget_for(role);

    // Scale each field proportionally based on model context size.
    // Reference model: 200k tokens (800k chars). Budgets are designed for that baseline.
    let reference_chars: f64 = REFERENCE_CONTEXT_WINDOW_TOKENS as f64 * 4.0;
    let model_chars = model_context_tokens as f64 * 4.0;
    let scale = (model_chars / reference_chars).clamp(0.25, 2.0);

    let scaled = |v: usize| -> usize {
        let raw = (v as f64 * scale) as usize;
        // Never go below 25% of original or above 200%
        raw.clamp(v / 4, v * 2)
    };

    PromptBudget {
        plan: scaled(base.plan),
        workspace_map: scaled(base.workspace_map),
        prd2: scaled(base.prd2),
        context: scaled(base.context),
        brief: scaled(base.brief),
        reviews: scaled(base.reviews),
        instructions: scaled(base.instructions),
        file_context: scaled(base.file_context),
        skills: scaled(base.skills),
    }
}

// ─── Utility sections ───────────────────────────────────────────────────────

/// Build the standard `agents_instructions` section used by all role templates.
///
/// This is the canonical constructor -- all templates should use this instead
/// of manually building the section to avoid drift in priority/cache/placement.
pub fn agents_instructions_section(agents_md: &str) -> PromptSection {
    PromptSection::new("agents_instructions", agents_md)
        .with_priority(SectionPriority::Critical)
        .with_cache_layer(CacheLayer::Role)
        .with_placement(Placement::Start)
}

// ─── Reusable stanza constants ───────────────────────────────────────────────

/// Describes the canonical plans context layout for agents.
///
/// Injected into prompts so agents know where to find plan artifacts
/// without relying on `find`/`ls`.
pub const CONTEXT_LAYOUT_STANZA: &str = "\
## Plans context layout

- `prd/` — canonical product-spec root; use this for source PRDs and specs by default.
- `.roko/plans/` — canonical plan-artifact root; use this for plan files, reviews, and caches.
- `.roko/plans/workspace-map.md` — crate file tree; use this instead of `find`/`ls` on `crates/`.
- `.roko/plans/preflight-snapshot.md` — ambient compile/test baseline when present.
- `.roko/plans/CONTEXT.md` — cross-plan registry (types, boundaries, decisions).
- `.roko/plans/ignored-tests.md` — ledger of `#[ignore]` tests.
- `.roko/plans/<plan-base>/prd-extract.md` — PRD extracts per plan (optional).
- `.roko/plans/<plan-base>/decomposition.md` — step breakdown (optional).
- `.roko/plans/<plan-base>/tasks.toml` — task checklists.
- `.roko/plans/<plan-base>/research.md`, `integration.md` — execution artifacts.
- `.roko/plans/<plan-base>/verify.sh` — invariant runner when generated (optional).
- `.roko/plans/<plan-base>/brief.md` — implementation brief when present.
- `.roko/plans/<plan-base>/reviews/` — per-plan review outputs when present.
";

/// MCP tools stanza — describes the free tools available to agents.
///
/// Injected into prompts so agents prefer MCP tools over shelling out.
/// Roko-owned MCP tools guidance.
pub const MCP_TOOLS_STANZA: &str = "\
## MCP Tools (free, instant)

You have MCP server tools. Use them for file reading, searching, and navigation \
instead of shelling out. They are faster and do not consume subprocess budget.
";

/// Standard TOML format for nits (minor observations that are not blocking).
///
/// Agents write nits to `plans/context/nits/<plan-num>-nits.toml`.
pub const NITS_FORMAT: &str = r#"```toml
[[nit]]
reviewer = "quick-reviewer"     # or architect / auditor / critic
file = "crates/foo/src/lib.rs"  # relative to repo root; omit if not file-specific
line = 42                       # optional
description = "variable name `x` could be more descriptive"
category = "style"              # style | naming | docs | spec_deviation | other
```"#;

// ─── Formatting helpers ──────────────────────────────────────────────────────

/// Wrap prior review text in an XML section with a "do not re-raise" instruction.
///
/// Returns an empty string when the review is empty.
#[must_use]
pub fn format_prior_review(review: &str) -> String {
    if review.is_empty() {
        return String::new();
    }
    format!(
        "\n## Prior Review\n\n\
         <prior-review>\n{review}\n</prior-review>\n\n\
         Do NOT re-raise issues that have been fixed.\n"
    )
}

/// Standard verdict TOML format instructions for reviewer agents.
///
/// The `plan_num` is included so nits can be written to the right file.
#[must_use]
pub fn format_verdict_instructions(plan_num: &str) -> String {
    format!(
        r#"## Verdict Format

Output your verdict in this exact format:

```toml
[verdict]
overall = "approve"  # or "revise"
code = "approve"     # or "revise" — mirrors overall for quick reviews
docs = "skip"        # quick-reviewer does not check docs

[[issues]]
id = "B1"
severity = "blocking"
file = "path/to/file.rs"
description = "What is wrong and what the fix should be"
```

If there are no blocking issues, output `overall = "approve"` with no issues.

## Nits

If you notice something minor — style, naming, cosmetic, missed doc comments, trivial clippy suggestions
that don't indicate bugs — write it to `plans/context/nits/{plan_num}-nits.toml` rather than listing it
in this review. Minor observations are NOT grounds for REVISE.

{NITS_FORMAT}

Write as many `[[nit]]` entries as needed. If the file doesn't exist yet, create it."#,
    )
}

/// Format a list of completed plans as a bullet list.
///
/// Returns `"(none)"` when the list is empty.
#[must_use]
pub fn format_plan_list(plans: &[String]) -> String {
    if plans.is_empty() {
        return "(none)".to_string();
    }
    plans.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_for_implementer_has_largest_caps() {
        let b = budget_for(AgentRole::Implementer);
        assert_eq!(b.plan, 50_000);
        assert_eq!(b.workspace_map, 20_000);
        assert_eq!(b.prd2, 12_000);
        assert_eq!(b.file_context, 8_000);
        assert_eq!(b.skills, 8_000);
    }

    #[test]
    fn budget_for_quick_reviewer_is_minimal() {
        let b = budget_for(AgentRole::QuickReviewer);
        assert_eq!(b.prd2, 0);
        assert_eq!(b.context, 0);
        assert_eq!(b.file_context, 0);
        assert_eq!(b.skills, 0);
        // Still gets plan and brief
        assert_eq!(b.plan, 50_000);
        assert_eq!(b.brief, 4_000);
    }

    #[test]
    fn budget_for_auto_fixer_is_bare_minimum() {
        let b = budget_for(AgentRole::AutoFixer);
        assert_eq!(b.plan, 0);
        assert_eq!(b.workspace_map, 0);
        assert_eq!(b.prd2, 0);
        assert_eq!(b.brief, 0);
        assert_eq!(b.instructions, 2_000);
    }

    #[test]
    fn budget_for_scribe_has_large_prd2() {
        let b = budget_for(AgentRole::Scribe);
        assert_eq!(b.prd2, 16_000);
    }

    #[test]
    fn budget_for_default_role_returns_balanced_caps() {
        let b = budget_for(AgentRole::Researcher);
        assert_eq!(b.plan, 50_000);
        assert_eq!(b.workspace_map, 8_000);
        assert_eq!(b.prd2, 6_000);
        assert_eq!(b.context, 4_000);
        assert_eq!(b.brief, 4_000);
    }

    #[test]
    fn context_layout_stanza_contains_key_paths() {
        assert!(CONTEXT_LAYOUT_STANZA.contains("`prd/`"));
        assert!(CONTEXT_LAYOUT_STANZA.contains("`.roko/plans/`"));
        assert!(!CONTEXT_LAYOUT_STANZA.contains(".mori"));
        assert!(CONTEXT_LAYOUT_STANZA.contains("workspace-map.md"));
        assert!(CONTEXT_LAYOUT_STANZA.contains("CONTEXT.md"));
    }

    #[test]
    fn mcp_tools_stanza_mentions_mcp() {
        assert!(MCP_TOOLS_STANZA.contains("MCP"));
        assert!(MCP_TOOLS_STANZA.contains("free"));
    }

    #[test]
    fn nits_format_contains_toml_structure() {
        assert!(NITS_FORMAT.contains("[[nit]]"));
        assert!(NITS_FORMAT.contains("reviewer"));
        assert!(NITS_FORMAT.contains("category"));
    }

    #[test]
    fn format_prior_review_empty_returns_empty() {
        assert!(format_prior_review("").is_empty());
    }

    #[test]
    fn format_prior_review_wraps_in_xml() {
        let out = format_prior_review("Fix the bug in module X.");
        assert!(out.contains("<prior-review>"));
        assert!(out.contains("</prior-review>"));
        assert!(out.contains("Fix the bug in module X."));
        assert!(out.contains("Do NOT re-raise"));
    }

    #[test]
    fn format_verdict_instructions_includes_plan_num() {
        let out = format_verdict_instructions("042");
        assert!(out.contains("042-nits.toml"));
        assert!(out.contains("[verdict]"));
        assert!(out.contains("overall"));
        assert!(out.contains("[[issues]]"));
    }

    #[test]
    fn format_plan_list_empty() {
        assert_eq!(format_plan_list(&[]), "(none)");
    }

    #[test]
    fn format_plan_list_multiple() {
        let plans = vec!["plan-041".into(), "plan-042".into()];
        let out = format_plan_list(&plans);
        assert!(out.contains("plan-041"));
        assert!(out.contains("plan-042"));
    }
}

#[cfg(test)]
mod adaptive_tests {
    use super::*;

    #[test]
    fn adaptive_budget_compute_clamps_min() {
        let ab = AdaptiveBudget::new(0.1, 5_000, 100_000);
        // Tiny model: 1k tokens = 4k chars, fraction gives 400 chars, clamp to min
        assert_eq!(ab.compute(1_000), 5_000);
    }

    #[test]
    fn adaptive_budget_compute_clamps_max() {
        let ab = AdaptiveBudget::new(0.5, 1_000, 50_000);
        // Huge model: 1M tokens = 4M chars, fraction gives 2M chars, clamp to max
        assert_eq!(ab.compute(1_000_000), 50_000);
    }

    #[test]
    fn adaptive_budget_compute_mid_range() {
        let ab = AdaptiveBudget::new(0.1, 1_000, 200_000);
        // 200k tokens = 800k chars, 10% = 80k chars
        assert_eq!(ab.compute(200_000), 80_000);
    }

    #[test]
    fn adaptive_budget_for_scales_with_context() {
        let baseline = budget_for(AgentRole::Implementer);
        // 200k tokens is the reference model — should return approximately baseline
        let scaled = adaptive_budget_for(AgentRole::Implementer, 200_000);
        assert_eq!(scaled.plan, baseline.plan);
        assert_eq!(scaled.workspace_map, baseline.workspace_map);
    }

    #[test]
    fn adaptive_budget_for_small_model_reduces() {
        let baseline = budget_for(AgentRole::Implementer);
        // 50k tokens = 200k chars, scale = 200k/800k = 0.25
        let scaled = adaptive_budget_for(AgentRole::Implementer, 50_000);
        assert!(scaled.plan <= baseline.plan);
        assert!(scaled.workspace_map <= baseline.workspace_map);
    }

    #[test]
    fn adaptive_budget_for_large_model_increases() {
        let baseline = budget_for(AgentRole::Implementer);
        // 400k tokens = 1.6M chars, scale = 1.6M/800k = 2.0
        let scaled = adaptive_budget_for(AgentRole::Implementer, 400_000);
        assert!(scaled.plan >= baseline.plan);
    }

    #[test]
    fn adaptive_budget_for_zero_fields_stay_zero() {
        // AutoFixer has many zero fields — they should remain zero after scaling
        let scaled = adaptive_budget_for(AgentRole::AutoFixer, 200_000);
        assert_eq!(scaled.plan, 0);
        assert_eq!(scaled.workspace_map, 0);
        assert_eq!(scaled.prd2, 0);
    }
}
