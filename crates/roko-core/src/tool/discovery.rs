//! Progressive tool discovery (§36.77 + §36.78).
//!
//! Small models perform strictly worse when presented with too many
//! tools (Qwen3-coder: format-switches above 5 tools; Vercel: cut 80%
//! of tools and got **better** results). This module exposes the
//! registry entry-point for returning the top-`k` tools by relevance
//! to a task description, with model-aware capping via
//! [`profile_for_model`].
//!
//! The real impl extends [`KeywordOverlapScorer`] via the
//! [`ToolRelevanceScorer`] trait — embedding-backed scorers will plug
//! into the same entry-point once `roko-index` exists.
//!
//! # Entry-points
//!
//! | Function | When to use |
//! |---|---|
//! | [`for_call`] | Model-agnostic: you own the limit |
//! | [`for_call_with_scorer`] | Bring your own scorer |
//! | [`for_call_model_aware`] | Clamps limit to model's degradation threshold |
//! | [`for_call_with_summary`] | Full observability (dropped count, effective limit) |

use super::def::ToolDef;
use super::format::profile_for_model;
use super::registry::ToolRegistry;
use super::relevance::{KeywordOverlapScorer, ToolRelevanceScorer};

// ─── for_call ────────────────────────────────────────────────────────────────

/// Return the top-`limit` tools from a registry, ranked by relevance to
/// the given task description.
///
/// Uses [`KeywordOverlapScorer`] internally. Equivalent to
/// `KeywordOverlapScorer.top_n(task, registry.all(), limit)` but hides the
/// scorer from the caller.
///
/// When `limit == 0` this returns an empty slice. When `limit` exceeds the
/// registry size all tools are returned (the scorer's `top_n` already
/// handles truncation).
#[must_use]
pub fn for_call<'a>(
    registry: &'a dyn ToolRegistry,
    task_description: &str,
    limit: usize,
) -> Vec<&'a ToolDef> {
    let scorer = KeywordOverlapScorer;
    scorer.top_n(task_description, registry.all(), limit)
}

// ─── for_call_with_scorer ────────────────────────────────────────────────────

/// Return the top-`limit` tools using a caller-provided scorer.
///
/// Identical to [`for_call`] but lets the caller inject any
/// [`ToolRelevanceScorer`] implementation — useful for tests and for
/// swapping in embedding-backed scorers without changing the call site.
#[must_use]
pub fn for_call_with_scorer<'a>(
    registry: &'a dyn ToolRegistry,
    task_description: &str,
    limit: usize,
    scorer: &dyn ToolRelevanceScorer,
) -> Vec<&'a ToolDef> {
    scorer.top_n(task_description, registry.all(), limit)
}

// ─── for_call_model_aware ────────────────────────────────────────────────────

/// Return the top tools for a task, capped by the model's degradation
/// threshold (§36.78).
///
/// Small models hallucinate or format-switch when given too many tools;
/// this function consults [`profile_for_model`] and clamps the effective
/// limit to `profile.max_tools_before_degrade`, preventing the caller from
/// accidentally flooding a small model with a 16-tool menu.
///
/// # Algorithm
///
/// 1. `profile = profile_for_model(model_slug)`
/// 2. `effective_limit = user_limit.unwrap_or(usize::MAX)`
///    `.min(profile.max_tools_before_degrade as usize)`
///    `.max(1)` — floor of 1 so we always return at least one tool
/// 3. `for_call(registry, task_description, effective_limit)`
///
/// If `model_slug` is unknown, [`profile_for_model`] returns the
/// conservative `unknown_default()` profile (cap = 3, stream disabled).
#[must_use]
pub fn for_call_model_aware<'a>(
    registry: &'a dyn ToolRegistry,
    task_description: &str,
    model_slug: &str,
    user_limit: Option<usize>,
) -> Vec<&'a ToolDef> {
    let profile = profile_for_model(model_slug);
    let effective_limit = user_limit
        .unwrap_or(usize::MAX)
        .min(profile.max_tools_before_degrade as usize)
        .max(1);
    for_call(registry, task_description, effective_limit)
}

// ─── DiscoverySummary ────────────────────────────────────────────────────────

/// Observability wrapper returned by [`for_call_with_summary`].
///
/// Carries everything a dispatcher or TUI needs to log (or display) what
/// happened during discovery: how many tools were in the registry, what
/// the caller asked for, what the model cap forced it down to, and which
/// tools were ultimately selected.
#[derive(Debug)]
pub struct DiscoverySummary<'a> {
    /// Tools selected for the call, ordered by descending relevance.
    pub selected: Vec<&'a ToolDef>,
    /// Total tools registered at query time.
    pub total_in_registry: usize,
    /// The limit the caller passed in (`user_limit` argument).
    pub requested_limit: usize,
    /// The actual limit applied after model-aware clamping.
    pub effective_limit: usize,
    /// Model slug used to look up the profile, if one was provided.
    pub model_slug: Option<String>,
}

impl DiscoverySummary<'_> {
    /// Number of registry tools that were **not** selected.
    #[must_use]
    pub fn dropped_count(&self) -> usize {
        self.total_in_registry.saturating_sub(self.selected.len())
    }
}

// ─── for_call_with_summary ───────────────────────────────────────────────────

/// Model-aware discovery with full observability metadata.
///
/// Identical to [`for_call_model_aware`] but returns a [`DiscoverySummary`]
/// instead of a bare `Vec`, so callers (dispatcher, TUI) can log or display
/// how many tools were dropped and why.
///
/// Pass `model_slug = None` to skip model-aware capping; `requested_limit`
/// is used as-is in that case (with a floor of 1).
#[must_use]
pub fn for_call_with_summary<'a>(
    registry: &'a dyn ToolRegistry,
    task_description: &str,
    model_slug: Option<&str>,
    requested_limit: usize,
) -> DiscoverySummary<'a> {
    let total_in_registry = registry.all().len();

    let effective_limit = model_slug.map_or_else(
        || requested_limit.max(1),
        |slug| {
            let profile = profile_for_model(slug);
            requested_limit
                .min(profile.max_tools_before_degrade as usize)
                .max(1)
        },
    );

    let selected = for_call(registry, task_description, effective_limit);

    DiscoverySummary {
        selected,
        total_in_registry,
        requested_limit,
        effective_limit,
        model_slug: model_slug.map(str::to_owned),
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::registry::VecToolRegistry;
    use crate::tool::{ToolCategory, ToolPermission};

    // ── Fixtures ─────────────────────────────────────────────────────────────

    fn td(name: &str, description: &str) -> ToolDef {
        ToolDef::new(
            name,
            description,
            ToolCategory::Read,
            ToolPermission::read_only(),
        )
    }

    /// Registry with 5 tools whose descriptions have clear keyword differentiation.
    fn five_tool_registry() -> VecToolRegistry {
        VecToolRegistry::from_tools(vec![
            td(
                "read_file",
                "Read a UTF-8 text file with optional line range",
            ),
            td("write_file", "Write or overwrite a text file"),
            td("grep", "Search file contents using regex pattern matching"),
            td("bash", "Execute a shell command and capture stdout stderr"),
            td("web_fetch", "Fetch a URL over HTTPS and return the body"),
        ])
    }

    /// Registry with 16 tools (simulates full built-in menu).
    fn sixteen_tool_registry() -> VecToolRegistry {
        VecToolRegistry::from_tools(
            (0..16)
                .map(|i| {
                    td(
                        &format!("tool_{i}"),
                        &format!("tool number {i} does something unique"),
                    )
                })
                .collect(),
        )
    }

    // ── A minimal scorer that returns the same constant for every tool ────────

    struct ConstScorer(f32);

    impl ToolRelevanceScorer for ConstScorer {
        fn score(&self, _task: &str, _tool: &ToolDef) -> f32 {
            self.0
        }
    }

    // ── 1. for_call_returns_top_n_by_relevance ───────────────────────────────

    #[test]
    fn for_call_returns_top_n_by_relevance() {
        let reg = five_tool_registry();
        let results = for_call(&reg, "read a file from disk", 2);
        assert_eq!(results.len(), 2, "should return exactly 2 tools");
        // "read_file" must be first — its name+description has the best keyword overlap
        assert_eq!(
            results[0].name, "read_file",
            "read_file should rank first for a read-file task"
        );
    }

    // ── 2. for_call_limit_larger_than_registry_returns_all ───────────────────

    #[test]
    fn for_call_limit_larger_than_registry_returns_all() {
        let reg = five_tool_registry();
        let results = for_call(&reg, "do anything", 100);
        assert_eq!(
            results.len(),
            5,
            "should return all tools when limit > registry size"
        );
    }

    // ── 3. for_call_limit_zero_returns_empty ─────────────────────────────────

    #[test]
    fn for_call_limit_zero_returns_empty() {
        let reg = five_tool_registry();
        let results = for_call(&reg, "read a file", 0);
        assert!(results.is_empty(), "limit=0 must return empty");
    }

    // ── 4. for_call_empty_registry_returns_empty ─────────────────────────────

    #[test]
    fn for_call_empty_registry_returns_empty() {
        let reg = VecToolRegistry::new();
        let results = for_call(&reg, "read a file", 5);
        assert!(results.is_empty(), "empty registry must return empty");
    }

    // ── 5. for_call_empty_task_description_still_returns_tools_deterministically

    #[test]
    fn for_call_empty_task_description_still_returns_tools_deterministically() {
        let reg = five_tool_registry();
        // All scores will be 0.0 → stable order (rank uses partial_cmp which
        // yields Equal on ties, and sort_by is stable).
        let first = for_call(&reg, "", 5);
        let second = for_call(&reg, "", 5);
        let names_first: Vec<&str> = first.iter().map(|t| t.name.as_str()).collect();
        let names_second: Vec<&str> = second.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names_first, names_second,
            "tie-broken order must be deterministic across identical calls"
        );
        assert_eq!(names_first.len(), 5, "should still return all 5 tools");
    }

    // ── 6. for_call_with_scorer_uses_caller_scorer ───────────────────────────

    #[test]
    fn for_call_with_scorer_uses_caller_scorer() {
        let reg = five_tool_registry();
        let scorer = ConstScorer(0.5); // all tools score identically
        let results = for_call_with_scorer(&reg, "anything", 3, &scorer);
        // ConstScorer gives every tool 0.5, so `top_n` with limit=3 should
        // return 3 tools (the first 3 in stable order after the tie-sort).
        assert_eq!(
            results.len(),
            3,
            "ConstScorer with limit=3 should return exactly 3 tools"
        );
    }

    // ── 7. for_call_model_aware_caps_by_profile ──────────────────────────────

    #[test]
    fn for_call_model_aware_caps_by_profile() {
        let reg = sixteen_tool_registry();
        // qwen3-32b has max_tools_before_degrade = 5
        let results = for_call_model_aware(&reg, "search patterns", "qwen3-32b", None);
        assert!(
            results.len() <= 5,
            "qwen3 profile cap=5 must be respected; got {}",
            results.len()
        );
    }

    // ── 8. for_call_model_aware_respects_user_lower_limit ────────────────────

    #[test]
    fn for_call_model_aware_respects_user_lower_limit() {
        let reg = sixteen_tool_registry();
        // profile cap=5, user wants only 2 → effective = min(2, 5).max(1) = 2
        let results = for_call_model_aware(&reg, "something", "qwen3-32b", Some(2));
        assert_eq!(
            results.len(),
            2,
            "user_limit=2 must win when it is lower than the profile cap"
        );
    }

    // ── 9. for_call_model_aware_handles_unknown_slug_gracefully ──────────────

    #[test]
    fn for_call_model_aware_handles_unknown_slug_gracefully() {
        let reg = sixteen_tool_registry();
        // mystery-model-xyz → unknown_default() profile: cap = 3
        let results = for_call_model_aware(&reg, "do a task", "nonexistent-model-xyz", None);
        // unknown_default cap is 3, so we must get ≤ 3
        assert!(
            results.len() <= 3,
            "unknown model must fall back to unknown_default cap=3; got {}",
            results.len()
        );
    }

    // ── 10. for_call_model_aware_never_returns_zero_tools ────────────────────

    #[test]
    fn for_call_model_aware_never_returns_zero_tools() {
        let reg = five_tool_registry();
        // Even if the profile somehow had cap=0, our floor of .max(1) prevents
        // returning an empty slice. We use a slug whose profile has cap=3 and
        // pass user_limit=Some(0); floor bumps it to 1.
        // Note: the floor applies to the clamped value not to user_limit itself,
        // so Some(0).min(3).max(1) = max(0, 1) = 1.
        // But wait — our algorithm: user_limit.unwrap_or(MAX).min(cap).max(1)
        //   = 0.min(3).max(1) = 0.max(1) = 1.  Good.
        let results = for_call_model_aware(&reg, "read a file", "llama3-8b", Some(0));
        assert!(
            !results.is_empty(),
            "effective limit must never be 0 — floor is 1"
        );
    }

    // ── 11. for_call_with_summary_populates_all_fields ───────────────────────

    #[test]
    fn for_call_with_summary_populates_all_fields() {
        let reg = five_tool_registry();
        let summary = for_call_with_summary(
            &reg,
            "grep for regex patterns",
            Some("claude-sonnet-4-5"),
            3,
        );
        // claude profile has cap=32, so effective_limit = min(3,32).max(1) = 3
        assert_eq!(
            summary.total_in_registry, 5,
            "total must reflect registry size"
        );
        assert_eq!(
            summary.requested_limit, 3,
            "requested_limit must match argument"
        );
        assert_eq!(
            summary.effective_limit, 3,
            "effective_limit should be 3 (profile cap≫3)"
        );
        assert_eq!(
            summary.model_slug.as_deref(),
            Some("claude-sonnet-4-5"),
            "model_slug must be stored"
        );
        assert_eq!(summary.selected.len(), 3, "selected must have 3 tools");
    }

    // ── 12. for_call_with_summary_tracks_dropped_count ───────────────────────

    #[test]
    fn for_call_with_summary_tracks_dropped_count() {
        let reg = sixteen_tool_registry();
        // requested_limit=10, qwen3 profile cap=5 → effective=5
        // total=16, selected=5 → dropped=11
        let summary = for_call_with_summary(&reg, "search for patterns", Some("qwen3-32b"), 10);
        assert_eq!(
            summary.effective_limit, 5,
            "qwen3 profile cap must clamp to 5"
        );
        assert_eq!(
            summary.selected.len(),
            5,
            "exactly 5 tools should be selected"
        );
        assert_eq!(
            summary.dropped_count(),
            11,
            "16 - 5 = 11 tools should be dropped"
        );
    }
}
