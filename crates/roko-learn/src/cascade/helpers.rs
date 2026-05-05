//! Free functions used by the cascade router: slug matching, tier mapping,
//! cost/latency helpers, thinking preference filtering, cache affinity,
//! hysteresis, shadow evaluation, and the static role-to-model table.

use roko_agent::AgentResult;
use roko_core::agent::{AgentRole, ModelSpec, ModelTier};
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_core::{BehavioralState, DaimonPolicy, Temperament};
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::costs_db::CostTable;
use crate::model_router::RoutingContext;

use super::types::{
    CACHE_AFFINITY_BONUS, CONFIDENCE_TO_UCB_THRESHOLD, CascadeStage, HYSTERESIS_THRESHOLD,
};
use crate::model_router::COLD_START_THRESHOLD;

// ─── Static role -> model table ─────────────────────────────────────────────

/// Build the default static role-to-model mapping.
///
/// Fast-tier roles prefer Gemini Flash-Lite, Standard-tier roles prefer
/// Gemini Flash, and Premium-tier roles prefer Opus with Gemini Pro Preview
/// as the premium fallback.
///
/// Candidate lists intentionally include older slugs (e.g. `claude-sonnet-4-5`)
/// as trailing fallbacks. `pick_static_slug` only returns a candidate when it
/// appears in `model_slugs` (the configured model set), so stale candidates
/// harmlessly fall through to the next entry and do not cause mis-routing.
pub(crate) fn default_role_model_table(model_slugs: &[String]) -> HashMap<AgentRole, String> {
    let mut table = HashMap::new();

    // Research role -> Perplexity Sonar when available, standard-tier fallback.
    table.insert(
        AgentRole::Researcher,
        pick_static_slug(
            model_slugs,
            &[
                "sonar-pro",
                "sonar",
                "gemini-2.5-flash",
                "gemini-2.5-pro",
                "kimi-k2.5",
                "claude-sonnet-4-6",
                "claude-sonnet-4-5",
            ],
        ),
    );

    let all_roles: Vec<AgentRole> = std::iter::once(AgentRole::Conductor)
        .chain(AgentRole::ALL_AGENTS.iter().copied())
        .collect();
    for role in all_roles {
        if table.contains_key(&role) {
            continue;
        }
        let slug = match role.model_tier() {
            ModelTier::Fast => {
                pick_static_slug(model_slugs, &["gemini-2.5-flash-lite", "claude-haiku-4-5"])
            }
            ModelTier::Premium => pick_static_slug(
                model_slugs,
                &[
                    "claude-opus-4-6",
                    "gemini-3.1-pro-preview",
                    "gemini-2.5-pro",
                ],
            ),
            // Standard and forward-compat
            _ => pick_static_slug(
                model_slugs,
                &[
                    "gemini-2.5-flash",
                    "gemini-2.5-pro",
                    "kimi-k2.5",
                    "kimi-k2-thinking",
                    "claude-sonnet-4-6",
                    "claude-sonnet-4-5",
                ],
            ),
        };
        table.insert(role, slug);
    }
    table
}

pub(crate) fn pick_static_slug(model_slugs: &[String], candidates: &[&str]) -> String {
    for candidate in candidates {
        if let Some(slug) = model_slugs
            .iter()
            .find(|slug| slugs_match(slug, candidate))
            .cloned()
        {
            return slug;
        }
    }
    candidates[0].to_string()
}

pub(crate) fn pick_available_static_slug(model_slugs: &[String], candidates: &[&str]) -> String {
    for candidate in candidates {
        if let Some(slug) = model_slugs
            .iter()
            .find(|slug| slugs_match(slug, candidate))
            .cloned()
        {
            return slug;
        }
    }

    model_slugs
        .first()
        .cloned()
        .unwrap_or_else(|| candidates[0].to_string())
}

/// Default latency SLA for a model tier (milliseconds).
pub(crate) const fn default_latency_sla(tier: ModelTier) -> u64 {
    match tier {
        ModelTier::Fast => 10_000,
        ModelTier::Premium => 120_000,
        // Standard and forward-compat
        _ => 30_000,
    }
}

/// Resolve a model's tier using a config-sourced tier map with heuristic fallback.
///
/// When `tier_map` contains the slug, returns the config-sourced tier.
/// Otherwise falls back to [`slug_to_tier_heuristic`].
///
/// The tier map is built from `ModelProfile.tier` fields at `CascadeRouter`
/// construction time via [`CascadeRouter::with_model_tiers`].
pub(crate) fn slug_to_tier(slug: &str, tier_map: &HashMap<String, ModelTier>) -> ModelTier {
    if let Some(&tier) = tier_map.get(slug) {
        return tier;
    }
    slug_to_tier_heuristic(slug)
}

/// Substring-based tier heuristic (legacy fallback).
///
/// Intentionally conservative — only matches well-known patterns.
/// For precise routing, set the `tier` field in `roko.toml` model profiles
/// and call [`CascadeRouter::with_model_tiers`] at construction.
pub(crate) fn slug_to_tier_heuristic(slug: &str) -> ModelTier {
    if slug.contains("flash-lite") || slug.contains("haiku") {
        ModelTier::Fast
    } else if slug.contains("opus") || slug.contains("-pro-preview") {
        ModelTier::Premium
    } else {
        ModelTier::Standard
    }
}

/// Build the ordered fallback chain for a routed primary model.
pub(crate) fn fallback_chain_for_model(
    model_slugs: &[String],
    primary_slug: &str,
    tier_map: &HashMap<String, ModelTier>,
) -> Vec<ModelSpec> {
    let primary_tier = slug_to_tier(primary_slug, tier_map);

    if matches!(primary_tier, ModelTier::Fast) {
        return Vec::new();
    }

    let mut grouped = [Vec::new(), Vec::new(), Vec::new()];

    for slug in model_slugs {
        if slugs_match(slug, primary_slug) {
            continue;
        }

        let bucket = match primary_tier {
            ModelTier::Standard => match slug_to_tier(slug, tier_map) {
                ModelTier::Fast => 0,
                ModelTier::Standard => 1,
                ModelTier::Premium => 2,
                _ => 1,
            },
            ModelTier::Premium => match slug_to_tier(slug, tier_map) {
                ModelTier::Standard => 0,
                ModelTier::Fast => 1,
                ModelTier::Premium => 2,
                _ => 0,
            },
            _ => 0,
        };

        grouped[bucket].push(ModelSpec::from_slug(slug));
    }

    grouped.into_iter().flatten().collect()
}

/// Find a stronger model to use when the selected model overflows context.
pub(crate) fn context_overflow_fallback_for_model(
    model_slugs: &[String],
    primary_slug: &str,
    tier_map: &HashMap<String, ModelTier>,
) -> Option<ModelSpec> {
    let primary_rank = model_tier_rank(slug_to_tier(primary_slug, tier_map));

    model_slugs
        .iter()
        .find(|slug| model_tier_rank(slug_to_tier(slug, tier_map)) > primary_rank)
        .map(ModelSpec::from_slug)
}

pub(crate) fn low_confidence_tier_bonus(tier: ModelTier) -> f64 {
    match tier {
        ModelTier::Premium => 0.15,
        ModelTier::Standard => 0.05,
        ModelTier::Fast => 0.0,
        _ => 0.05,
    }
}

pub(crate) fn routing_tier_bias_factor(tier: ModelTier) -> f64 {
    match tier {
        ModelTier::Fast => 1.10,
        ModelTier::Standard => 1.0,
        ModelTier::Premium => 0.85,
        _ => 1.0,
    }
}

pub(crate) fn cost_pressure_factor(tier: ModelTier) -> f64 {
    match tier {
        ModelTier::Fast => 1.20,
        ModelTier::Standard => 0.90,
        ModelTier::Premium => 0.0,
        _ => 0.90,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ProviderHealthSnapshotKey {
    pub(crate) state_rank: u8,
    pub(crate) consecutive_failures: u32,
    pub(crate) total_failures: u64,
    pub(crate) last_failure_at: i64,
}

impl From<&crate::provider_health::ProviderHealth> for ProviderHealthSnapshotKey {
    fn from(health: &crate::provider_health::ProviderHealth) -> Self {
        let state_rank = match health.state {
            crate::provider_health::CircuitState::Closed => 0,
            crate::provider_health::CircuitState::HalfOpen => 1,
            crate::provider_health::CircuitState::Open => 2,
        };
        Self {
            state_rank,
            consecutive_failures: health.consecutive_failures,
            total_failures: health.total_failures,
            last_failure_at: health.last_failure_at.unwrap_or(i64::MIN),
        }
    }
}

pub(crate) fn behavioral_state_tier_shift(ctx: &RoutingContext) -> i8 {
    // When affect-adjusted tier thresholds are available, derive the shift
    // from prediction error (`1.0 - affect_confidence`) against the per-state
    // ceilings.  High prediction error (above t1_ceiling) pushes toward
    // Premium (+1); low error (within t0_ceiling) pulls toward Fast (-1).
    if let Some(thresholds) = &ctx.tier_thresholds {
        let prediction_error = 1.0 - ctx.daimon_policy.affect_confidence.clamp(0.0, 1.0);
        return if prediction_error > thresholds.t1_ceiling {
            1 // exceed Standard ceiling -> escalate to Premium
        } else if prediction_error <= thresholds.t0_ceiling {
            -1 // within Fast ceiling -> save cost
        } else {
            0 // within Standard band -> no shift
        };
    }

    // Fallback: hardcoded per-state shift when no thresholds are supplied.
    match ctx.daimon_policy.behavioral_state {
        BehavioralState::Struggling => 1,
        BehavioralState::Coasting | BehavioralState::Resting | BehavioralState::Focused => -1,
        BehavioralState::Exploring => {
            if matches!(ctx.complexity, TaskComplexityBand::Complex)
                || matches!(ctx.task_category, TaskCategory::Research)
                || ctx.has_prior_failure
            {
                1
            } else {
                0
            }
        }
        BehavioralState::Engaged => 0,
    }
}

pub(crate) fn conductor_load_tier_shift(ctx: &RoutingContext) -> i8 {
    let load = ctx.conductor_load.clamp(0.0, 1.0);
    if load >= 0.9 {
        -2
    } else if load >= 0.65 {
        -1
    } else {
        0
    }
}

pub(crate) fn temperament_tier_shift(ctx: &RoutingContext) -> i8 {
    ctx.temperament
        .map(Temperament::routing_tier_shift)
        .unwrap_or(0)
}

pub(crate) fn temperament_exploration_multiplier(ctx: &RoutingContext) -> f64 {
    ctx.temperament
        .map(Temperament::exploration_multiplier)
        .unwrap_or(1.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThinkingPreference {
    Neutral,
    PreferThinking,
    PreferNonThinking,
}

pub(crate) fn thinking_preference(ctx: &RoutingContext) -> ThinkingPreference {
    let Some(level) = ctx.thinking_level.as_deref() else {
        return ThinkingPreference::Neutral;
    };

    let level = level.trim().to_ascii_lowercase();
    match level.as_str() {
        "high" | "max" if ctx.complexity == TaskComplexityBand::Complex => {
            ThinkingPreference::PreferThinking
        }
        "minimal" | "none" | "disabled" | "off" | "false" => ThinkingPreference::PreferNonThinking,
        _ => ThinkingPreference::Neutral,
    }
}

pub(crate) fn model_supports_thinking(slug: &str) -> bool {
    let slug = slug.to_ascii_lowercase();
    if slug.contains("gemini-2.5-flash-lite")
        || slug.starts_with("sonar")
        || slug.starts_with("perplexity/")
    {
        return false;
    }

    slug.contains("gemini-2.5-flash")
        || slug.contains("gemini-2.5-pro")
        || slug.contains("gemini-3")
        || slug.starts_with("kimi-k2")
        || slug.starts_with("glm")
        || slug.contains("gpt-5")
        || slug.starts_with("o1")
        || slug.starts_with("o3")
        || slug.starts_with("o4")
        || slug.contains("thinking")
        || slug.contains("reasoning")
}

pub(crate) fn thinking_filtered_candidates(
    candidates: &[String],
    ctx: &RoutingContext,
) -> Vec<String> {
    let wants_thinking = match thinking_preference(ctx) {
        ThinkingPreference::PreferThinking => Some(true),
        ThinkingPreference::PreferNonThinking => Some(false),
        ThinkingPreference::Neutral => None,
    };
    let Some(wants_thinking) = wants_thinking else {
        return candidates.to_vec();
    };

    let filtered: Vec<String> = candidates
        .iter()
        .filter(|slug| model_supports_thinking(slug) == wants_thinking)
        .cloned()
        .collect();
    if filtered.is_empty() {
        candidates.to_vec()
    } else {
        filtered
    }
}

pub(crate) fn pick_tier_extreme(
    candidates: &[String],
    prefer_strongest: bool,
    tier_map: &HashMap<String, ModelTier>,
) -> Option<String> {
    let mut iter = candidates.iter();
    let first = iter.next()?.clone();
    let mut best = first;
    let mut best_rank = model_tier_rank(slug_to_tier(&best, tier_map));

    for slug in iter {
        let rank = model_tier_rank(slug_to_tier(slug, tier_map));
        let better = if prefer_strongest {
            rank > best_rank
        } else {
            rank < best_rank
        };
        if better {
            best = slug.clone();
            best_rank = rank;
        }
    }

    Some(best)
}

pub(crate) fn apply_cache_affinity(scores: &mut [(String, f64)], previous_model: Option<&str>) {
    if let Some(prev) = previous_model {
        for (slug, score) in scores.iter_mut() {
            if slug == prev {
                *score += CACHE_AFFINITY_BONUS;
            }
        }
    }
}

pub(crate) fn select_with_hysteresis(
    candidates: &[(String, f64)],
    previous_model: Option<&str>,
) -> String {
    let best = candidates
        .iter()
        .max_by(|lhs, rhs| lhs.1.total_cmp(&rhs.1))
        .expect("CascadeRouter: score-based routing requires at least one candidate");

    if let Some(previous_model) = previous_model
        && let Some(previous_score) = candidates
            .iter()
            .find(|(slug, _)| slug == previous_model)
            .map(|(_, score)| *score)
        && best.0 != previous_model
        && best.1 - previous_score < HYSTERESIS_THRESHOLD
    {
        return previous_model.to_string();
    }

    best.0.clone()
}

pub(crate) fn model_tier_rank(tier: ModelTier) -> u8 {
    match tier {
        ModelTier::Premium => 2,
        ModelTier::Standard => 1,
        ModelTier::Fast => 0,
        _ => 1,
    }
}

pub(crate) fn target_tier_rank(current_rank: u8, shift: i8) -> u8 {
    if shift.is_negative() {
        current_rank.saturating_sub(shift.unsigned_abs())
    } else {
        current_rank.saturating_add(shift as u8).min(2)
    }
}

/// Test whether two model slugs refer to the same model family.
pub(crate) fn slugs_match(lhs: &str, rhs: &str) -> bool {
    lhs == rhs || slug_family(lhs).is_some_and(|family| slug_family(rhs) == Some(family))
}

pub(crate) fn parse_agent_role(raw: &str) -> Option<AgentRole> {
    if let Ok(role) = serde_json::from_str::<AgentRole>(&format!("\"{raw}\"")) {
        return Some(role);
    }

    std::iter::once(AgentRole::Conductor)
        .chain(AgentRole::ALL_AGENTS.iter().copied())
        .find(|role| raw == role.label() || raw == format!("{role:?}"))
}

/// Classify a model slug into a known model family.
///
/// This is the canonical family classifier used by both `cascade_router` and
/// `model_router` for slug matching, Pareto cost proxies, and static routing.
pub fn slug_family(slug: &str) -> Option<&'static str> {
    if slug.starts_with("kimi-k2") {
        Some("kimi-k2")
    } else if slug.contains("gemini-3.1-pro-preview") {
        Some("gemini-3.1-pro-preview")
    } else if slug.contains("gemini-3.1-flash-lite-preview") {
        Some("gemini-3.1-flash-lite-preview")
    } else if slug.contains("gemini-3-flash-preview") {
        Some("gemini-3-flash-preview")
    } else if slug.contains("gemini-2.5-pro") {
        Some("gemini-2.5-pro")
    } else if slug.contains("gemini-2.5-flash-lite") {
        Some("gemini-2.5-flash-lite")
    } else if slug.contains("gemini-2.5-flash") {
        Some("gemini-2.5-flash")
    } else if slug.contains("haiku") {
        Some("haiku")
    } else if slug.contains("sonnet") {
        Some("sonnet")
    } else if slug.contains("opus") {
        Some("opus")
    } else if slug.contains("glm") {
        Some("glm")
    } else if slug.starts_with("gpt-") {
        Some("gpt")
    } else if slug.starts_with("o1") {
        Some("o1")
    } else if slug.starts_with("o3") {
        Some("o3")
    } else if slug.starts_with("deepseek") {
        Some("deepseek")
    } else if slug.starts_with("gemini") {
        Some("gemini")
    } else {
        None
    }
}

pub(crate) fn default_cost_table() -> &'static CostTable {
    static COST_TABLE: OnceLock<CostTable> = OnceLock::new();
    COST_TABLE.get_or_init(CostTable::default)
}

pub(crate) fn estimate_total_cost_usd(
    model_slug: &str,
    input_tokens: u64,
    output_tokens: u64,
) -> f64 {
    default_cost_table()
        .lookup(model_slug)
        .map(|pricing| pricing.estimate_total(input_tokens, output_tokens))
        .unwrap_or(0.0)
}

/// Determine the cascade stage from observation count.
pub(crate) const fn stage_for_observations(obs: u64) -> CascadeStage {
    if obs < COLD_START_THRESHOLD {
        CascadeStage::Static
    } else if obs < CONFIDENCE_TO_UCB_THRESHOLD {
        CascadeStage::Confidence
    } else {
        CascadeStage::Ucb
    }
}

pub(crate) fn pareto_adjusted_alpha(base_alpha: f64, slug: &str, frontier: &[String]) -> f64 {
    if frontier.iter().any(|frontier_slug| frontier_slug == slug) {
        base_alpha
    } else {
        base_alpha * 0.1
    }
}

pub(crate) fn pareto_cost_proxy(slug: &str, tier_map: &HashMap<String, ModelTier>) -> f64 {
    match slug_family(slug) {
        Some("gemini-3.1-flash-lite-preview") => 0.9,
        Some("gemini-3-flash-preview") => 1.5,
        Some("haiku") => 1.0,
        Some("sonnet") => 3.0,
        Some("opus") => 9.0,
        Some("kimi-k2") => 2.5,
        _ => match slug_to_tier(slug, tier_map) {
            ModelTier::Fast => 1.0,
            ModelTier::Premium => 9.0,
            _ => 3.0,
        },
    }
}

pub(crate) fn pareto_latency_proxy(slug: &str, tier_map: &HashMap<String, ModelTier>) -> f64 {
    default_latency_sla(slug_to_tier(slug, tier_map)) as f64
}

pub(crate) fn is_free_tier_gemini_model(slug: &str) -> bool {
    let slug = slug.to_ascii_lowercase();
    slug.contains("gemini-2.5-flash")
        || slug.contains("gemini-2.5-flash-lite")
        || slug.contains("gemini-3-flash-preview")
        || slug.contains("gemini-3.1-flash-lite-preview")
}

// ─── Shadow evaluation helpers ──────────────────────────────────────────────

pub(crate) fn infer_shadow_routing_context(
    prompt: &str,
    primary_result: &AgentResult,
) -> RoutingContext {
    let lower = prompt.to_ascii_lowercase();
    let task_category = infer_task_category(&lower);
    let complexity = infer_task_complexity(prompt, &lower);
    let role = infer_shadow_role(task_category, complexity, &lower);

    RoutingContext {
        task_category,
        complexity,
        iteration: 0,
        role,
        crate_familiarity: 0.5,
        has_prior_failure: !primary_result.success,
        conductor_load: 0.0,
        active_agents: 0,
        ready_queue_depth: 0,
        max_queue_wait_hours: 0.0,
        // Shadow evaluation should compare alternate models against a neutral
        // routing baseline. Reusing the primary outcome's affective state here
        // would leak live dispatch bias into offline evaluation.
        daimon_policy: DaimonPolicy::new(0.5, BehavioralState::Engaged),
        thinking_level: None,
        temperament: None,
        previous_model: primary_result.output.tag("model").map(str::to_string),
        plan_context_tokens: Some((prompt.len() as u64).div_ceil(4)),
        tier_thresholds: None,
    }
}

fn infer_task_category(lower_prompt: &str) -> TaskCategory {
    if contains_any(
        lower_prompt,
        &["research", "investigate", "why", "citation", "source"],
    ) {
        TaskCategory::Research
    } else if contains_any(
        lower_prompt,
        &["test", "verify", "assert", "failing", "regression"],
    ) {
        TaskCategory::Verification
    } else if contains_any(
        lower_prompt,
        &["integrate", "integration", "wire up", "hook up", "connect"],
    ) {
        TaskCategory::Integration
    } else if contains_any(lower_prompt, &["refactor", "cleanup", "rename", "extract"]) {
        TaskCategory::Refactor
    } else if contains_any(lower_prompt, &["doc", "readme", "documentation", "explain"]) {
        TaskCategory::Docs
    } else if contains_any(lower_prompt, &["ci", "cargo", "build", "deploy", "infra"]) {
        TaskCategory::Infra
    } else {
        TaskCategory::Implementation
    }
}

fn infer_task_complexity(prompt: &str, lower_prompt: &str) -> TaskComplexityBand {
    let word_count = prompt.split_whitespace().count();

    if contains_any(
        lower_prompt,
        &[
            "architecture",
            "cross-crate",
            "multi-crate",
            "end-to-end",
            "system design",
            "migration",
        ],
    ) || word_count > 250
    {
        TaskComplexityBand::Complex
    } else if contains_any(
        lower_prompt,
        &[
            "typo",
            "format",
            "lint",
            "rename",
            "small fix",
            "single file",
        ],
    ) || word_count < 40
    {
        TaskComplexityBand::Fast
    } else {
        TaskComplexityBand::Standard
    }
}

fn infer_shadow_role(
    task_category: TaskCategory,
    complexity: TaskComplexityBand,
    lower_prompt: &str,
) -> AgentRole {
    match task_category {
        TaskCategory::Research => AgentRole::Researcher,
        TaskCategory::Docs => AgentRole::Scribe,
        TaskCategory::Refactor => AgentRole::Refactorer,
        TaskCategory::Integration => AgentRole::IntegrationTester,
        TaskCategory::Verification => AgentRole::Auditor,
        _ if complexity == TaskComplexityBand::Complex
            || contains_any(lower_prompt, &["architecture", "design"]) =>
        {
            AgentRole::Architect
        }
        _ => AgentRole::Implementer,
    }
}

pub(crate) fn shadow_quality_score(
    prompt: &str,
    primary_result: &AgentResult,
    shadow_result: &AgentResult,
) -> f64 {
    if !shadow_result.success {
        return 0.0;
    }

    let Some(shadow_text) = result_text(shadow_result) else {
        return 0.0;
    };

    let prompt_requires_code = prompt_expects_code(prompt);
    let shadow_has_code = output_contains_code(shadow_text);

    let Some(primary_text) = result_text(primary_result) else {
        let structure_score = if shadow_text.split_whitespace().count() >= 8 {
            1.0_f64
        } else {
            0.5_f64
        };
        let code_score = if prompt_requires_code && !shadow_has_code {
            0.0_f64
        } else {
            1.0_f64
        };
        return structure_score.mul_add(0.3, code_score * 0.7);
    };

    let primary_words = primary_text.split_whitespace().count().max(1);
    let shadow_words = shadow_text.split_whitespace().count();
    let length_score = (shadow_words as f64 / primary_words as f64).min(1.0);

    let primary_has_code = output_contains_code(primary_text);
    let code_score = if prompt_requires_code || primary_has_code {
        if shadow_has_code { 1.0_f64 } else { 0.0_f64 }
    } else {
        1.0_f64
    };

    let primary_lines = primary_text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    let shadow_lines = shadow_text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    let structure_score = if primary_lines <= 1 {
        1.0_f64
    } else {
        (shadow_lines as f64 / primary_lines as f64).min(1.0)
    };

    length_score.mul_add(0.6, code_score.mul_add(0.25, structure_score * 0.15))
}

fn result_text(result: &AgentResult) -> Option<&str> {
    result
        .output
        .body
        .as_text()
        .ok()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn prompt_expects_code(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    contains_any(
        &lower,
        &[
            "code", "rust", "function", "impl", "struct", "test", "fix", "patch", "refactor",
        ],
    )
}

fn output_contains_code(text: &str) -> bool {
    text.contains("```")
        || text.contains("fn ")
        || text.contains("impl ")
        || text.contains("struct ")
        || text.contains("enum ")
        || text.contains("let ")
}

pub(crate) fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}
