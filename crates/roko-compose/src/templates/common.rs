//! Common prompt utilities shared across all role templates.
//!
//! Contains per-role token budgets, reusable stanza constants, and
//! formatting helpers that multiple templates reference. Ports Mori's
//! shared prompt infrastructure (budget tables, context layout, MCP
//! tools, verdict format) into a typed, I/O-free module.

use roko_core::AgentRole;

// ─── Per-role budgets ────────────────────────────────────────────────────────

/// Per-section character caps for a given agent role.
///
/// Matches Mori's `PromptBudget` struct from `prompts.rs:46`. Each field
/// is a maximum character count — the template truncates content to fit.
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

/// Return the per-section character budget for a given agent role.
///
/// Budget values match Mori's `budget_for()` (`prompts.rs:62`). The
/// model parameter is accepted for API compatibility but does not
/// currently affect budgets (all models get the same caps per role).
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

// ─── Reusable stanza constants ───────────────────────────────────────────────

/// Describes the canonical plans context layout for agents.
///
/// Injected into prompts so agents know where to find plan artifacts
/// without relying on `find`/`ls`. Matches Mori's `CONTEXT_LAYOUT_STANZA`.
pub const CONTEXT_LAYOUT_STANZA: &str = "\
## Plans context layout

- `prd/` — canonical product-spec root; use this for source PRDs and specs by default.
- `.mori/plans/` — canonical plan-artifact root; use this for plan files, reviews, and caches.
- `.mori/plans/workspace-map.md` — crate file tree; use this instead of `find`/`ls` on `crates/`.
- `.mori/plans/preflight-snapshot.md` — ambient compile/test baseline when present.
- `.mori/plans/CONTEXT.md` — cross-plan registry (types, boundaries, decisions).
- `.mori/plans/ignored-tests.md` — ledger of `#[ignore]` tests.
- `.mori/plans/<plan-base>/prd-extract.md` — PRD extracts per plan (optional).
- `.mori/plans/<plan-base>/decomposition.md` — step breakdown (optional).
- `.mori/plans/<plan-base>/tasks.toml` — task checklists.
- `.mori/plans/<plan-base>/research.md`, `integration.md` — execution artifacts.
- `.mori/plans/<plan-base>/verify.sh` — invariant runner when generated (optional).
- `.mori/plans/<plan-base>/brief.md` — implementation brief when present.
- `.mori/plans/<plan-base>/reviews/` — per-plan review outputs when present.
";

/// MCP tools stanza — describes the free tools available to agents.
///
/// Injected into prompts so agents prefer MCP tools over shelling out.
/// Matches Mori's `MCP_TOOLS_STANZA`.
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
        assert!(CONTEXT_LAYOUT_STANZA.contains("`.mori/plans/`"));
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
