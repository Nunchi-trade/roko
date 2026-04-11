//! Three-stage cascade router for model selection (section 13.8-13.11).
//!
//! The cascade combines three routing strategies, automatically transitioning
//! as observation count grows:
//!
//! | Stage | Name | Observations | Strategy |
//! |-------|------|-------------|----------|
//! | 1 | Static | < 50 | Hardcoded role -> model table |
//! | 2 | Confidence | 50 - 200 | Empirical pass rates + confidence interval |
//! | 3 | UCB1 | > 200 | Full `LinUCB` contextual bandit |
//!
//! # [`CascadeModel`]
//!
//! The router returns a [`CascadeModel`] containing a primary model,
//! an optional fallback model, and a latency SLA in milliseconds.
//!
//! # Thread safety
//!
//! The cascade wraps a [`LinUCBRouter`] and an additional
//! [`parking_lot::Mutex`] for confidence-stage statistics.

use async_trait::async_trait;
use parking_lot::Mutex;
use roko_agent::{AgentResult, gemini::GeminiMetadata};
use roko_core::OperatingFrequency;
use roko_core::agent::{AgentRole, ModelSpec, ModelTier};
use roko_core::task::{TaskCategory, TaskComplexityBand};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use crate::cfactor::{AgentDispatchBias, CFactor};
use crate::costs_db::CostTable;
use crate::model_router::{
    COLD_START_THRESHOLD, CONTEXT_DIM, LinUCBRouter, RoutingContext, compute_routing_reward_v2,
};
use crate::pareto::{ModelObservation, compute_pareto_frontier};
use crate::provider_health::ProviderHealthRegistry;

/// Async runner used by free-tier Gemini shadow evaluation.
#[async_trait]
pub trait ShadowModelRunner: Send + Sync {
    /// Run `prompt` against `model_slug` and return the resulting agent output.
    async fn run_shadow(&self, prompt: &str, model_slug: &str) -> AgentResult;
}

// ─── CascadeStage ───────────────────────────────────────────────────────────

/// Which routing stage is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CascadeStage {
    /// Stage 1: hardcoded role -> model table (< 50 observations).
    Static,
    /// Stage 2: empirical pass rates with confidence interval (50-200 observations).
    Confidence,
    /// Stage 3: full `LinUCB` contextual bandit (> 200 observations).
    Ucb,
}

impl CascadeStage {
    /// Human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Confidence => "confidence",
            Self::Ucb => "ucb",
        }
    }
}

impl std::fmt::Display for CascadeStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

// ─── CascadeModel ───────────────────────────────────────────────────────────

/// Routing recommendation from the cascade.
#[derive(Debug, Clone)]
pub struct CascadeModel {
    /// Primary model to use.
    pub primary: ModelSpec,
    /// Fallback model if the primary fails or times out.
    pub fallback: Option<ModelSpec>,
    /// Latency SLA in milliseconds.
    pub latency_sla_ms: u64,
    /// Which cascade stage produced this recommendation.
    pub stage: CascadeStage,
}

/// Selection result for raw-context routing.
#[derive(Debug, Clone)]
pub struct CascadeSelection {
    /// Model chosen by the router.
    pub model: ModelSpec,
    /// Total observations accumulated by the router when this selection was made.
    pub observations: u64,
    /// Which cascade stage produced the recommendation.
    pub stage: CascadeStage,
}

/// Explainable routing output for one cascade decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CascadeRoutingExplanation {
    /// Which cascade stage produced the decision.
    pub stage: CascadeStage,
    /// Primary model selected by the router.
    pub selected_model: String,
    /// Fallback model, when one exists for the selected tier.
    pub fallback_model: Option<String>,
    /// Latency SLA associated with the selected tier.
    pub latency_sla_ms: u64,
    /// Candidate-level scoring details.
    pub candidates: Vec<CascadeRoutingCandidate>,
}

/// Score and status for one routing candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CascadeRoutingCandidate {
    /// Candidate model slug.
    pub model: String,
    /// Stage-specific numeric score.
    pub score: f64,
    /// Whether this candidate was selected.
    pub selected: bool,
    /// Whether cache affinity applies for this candidate.
    pub cache_affinity: bool,
    /// Whether the candidate is on the Pareto frontier, when known.
    pub pareto_optimal: Option<bool>,
}

// ─── Confidence-stage stats ─────────────────────────────────────────────────

/// Threshold for transitioning from Confidence to UCB stage.
const CONFIDENCE_TO_UCB_THRESHOLD: u64 = 200;
/// Affect confidence below which the router biases toward stronger models.
const LOW_AFFECT_CONFIDENCE_THRESHOLD: f64 = 0.3;
/// C-Factor above which the router biases toward cheaper models.
const HIGH_CFACTOR_THRESHOLD: f64 = 0.8;
/// C-Factor below which the router biases toward stronger models.
const LOW_CFACTOR_THRESHOLD: f64 = 0.4;
/// Cold-start bonus for reusing the previous model.
const CACHE_AFFINITY_BONUS: f64 = 0.15;
/// Recompute the Pareto frontier after every 50 observations.
const PARETO_RECOMPUTE_INTERVAL: u64 = 50;

/// Per-model observation record for the confidence stage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ModelStats {
    /// Number of trials (selections) for this model.
    trials: u64,
    /// Number of successes (gate passes).
    successes: u64,
    /// Total citations observed across Perplexity responses.
    total_citations: u64,
    /// Total Perplexity search latency observed in milliseconds.
    total_search_latency_ms: u64,
    /// Total observed cost in USD (token cost + per-request fee).
    total_cost_usd: f64,
    /// Number of Perplexity requests contributing metadata.
    perplexity_requests: u64,
    /// Total Gemini thinking tokens observed across responses.
    total_gemini_thinking_tokens: u64,
    /// Total Gemini cached tokens observed across responses.
    total_gemini_cached_tokens: u64,
    /// Total Gemini grounding queries executed across responses.
    total_gemini_grounding_queries: u64,
    /// Number of successful Gemini code-execution outcomes.
    gemini_code_execution_successes: u64,
    /// Number of failed Gemini code-execution outcomes.
    gemini_code_execution_failures: u64,
    /// Number of Gemini responses routed in the ≤200K context pricing tier.
    gemini_context_window_le_200k_requests: u64,
    /// Number of Gemini responses routed in the >200K context pricing tier.
    gemini_context_window_gt_200k_requests: u64,
    /// Number of Gemini requests contributing observation metadata.
    gemini_requests: u64,
}

impl ModelStats {
    /// Empirical pass rate.
    #[allow(clippy::cast_precision_loss)]
    fn pass_rate(&self) -> f64 {
        if self.trials == 0 {
            0.0
        } else {
            self.successes as f64 / self.trials as f64
        }
    }

    /// Width of the 95% Wilson confidence interval (approximate).
    ///
    /// Uses a normal approximation: `1.96 * sqrt(p * (1-p) / n)`.
    /// Returns `f64::INFINITY` for zero trials.
    #[allow(clippy::cast_precision_loss)]
    fn confidence_width(&self) -> f64 {
        if self.trials == 0 {
            return f64::INFINITY;
        }
        let p = self.pass_rate();
        let n = self.trials as f64;
        1.96 * (p * (1.0 - p) / n).sqrt()
    }

    /// Upper confidence bound on the pass rate.
    fn upper_bound(&self) -> f64 {
        (self.pass_rate() + self.confidence_width()).min(1.0)
    }

    #[allow(clippy::cast_precision_loss)]
    fn avg_citations_per_response(&self) -> f64 {
        if self.perplexity_requests == 0 {
            0.0
        } else {
            self.total_citations as f64 / self.perplexity_requests as f64
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn avg_search_latency_ms(&self) -> f64 {
        if self.perplexity_requests == 0 {
            0.0
        } else {
            self.total_search_latency_ms as f64 / self.perplexity_requests as f64
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn avg_cost_usd(&self) -> f64 {
        if self.perplexity_requests == 0 {
            0.0
        } else {
            self.total_cost_usd / self.perplexity_requests as f64
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn cost_per_success(&self) -> Option<f64> {
        if self.successes == 0 {
            None
        } else {
            Some(self.total_cost_usd / self.successes as f64)
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn avg_gemini_thinking_tokens_per_response(&self) -> f64 {
        if self.gemini_requests == 0 {
            0.0
        } else {
            self.total_gemini_thinking_tokens as f64 / self.gemini_requests as f64
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn avg_gemini_cached_tokens_per_response(&self) -> f64 {
        if self.gemini_requests == 0 {
            0.0
        } else {
            self.total_gemini_cached_tokens as f64 / self.gemini_requests as f64
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn avg_gemini_grounding_queries_per_response(&self) -> f64 {
        if self.gemini_requests == 0 {
            0.0
        } else {
            self.total_gemini_grounding_queries as f64 / self.gemini_requests as f64
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn gemini_code_execution_success_rate(&self) -> f64 {
        let attempts = self.gemini_code_execution_successes + self.gemini_code_execution_failures;
        if attempts == 0 {
            0.0
        } else {
            self.gemini_code_execution_successes as f64 / attempts as f64
        }
    }
}

/// Per-request Perplexity metadata captured by the cascade learning loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerplexityObservation {
    /// Number of citations returned with the response.
    pub citation_count: u64,
    /// Search-side latency in milliseconds.
    pub search_latency_ms: u64,
    /// Input tokens billed for the request.
    pub input_tokens: u64,
    /// Output tokens billed for the request.
    pub output_tokens: u64,
}

/// Gemini pricing tier used for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GeminiContextTier {
    /// Request stayed within Gemini's ≤200K pricing tier.
    UpTo200k,
    /// Request crossed into Gemini's >200K pricing tier.
    Over200k,
}

impl GeminiContextTier {
    /// Infer the pricing tier from the billed prompt tokens.
    #[must_use]
    pub const fn for_input_tokens(input_tokens: u64) -> Self {
        if input_tokens > 200_000 {
            Self::Over200k
        } else {
            Self::UpTo200k
        }
    }
}

/// Per-request Gemini metadata captured by the cascade learning loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeminiObservation {
    /// Prompt tokens billed for the request.
    pub input_tokens: u64,
    /// Output tokens billed for the request.
    pub output_tokens: u64,
    /// Thinking tokens consumed by the model, if reported.
    pub thinking_tokens: Option<u64>,
    /// Cached prompt tokens read by the request, if any.
    pub cached_tokens: Option<u64>,
    /// Number of grounding queries executed through Google Search.
    pub grounding_query_count: u64,
    /// Count of successful code-execution results returned by Gemini.
    pub code_execution_success_count: u64,
    /// Count of failed code-execution results returned by Gemini.
    pub code_execution_failure_count: u64,
    /// Gemini input-context pricing tier.
    pub context_tier: GeminiContextTier,
}

impl GeminiObservation {
    /// Build a router observation from Gemini adapter metadata.
    #[must_use]
    pub fn from_metadata(metadata: &GeminiMetadata, input_tokens: u64, output_tokens: u64) -> Self {
        let (code_execution_success_count, code_execution_failure_count) = metadata
            .code_execution_results
            .iter()
            .fold((0_u64, 0_u64), |(successes, failures), result| {
                if result.outcome.eq_ignore_ascii_case("OUTCOME_OK") {
                    (successes + 1, failures)
                } else {
                    (successes, failures + 1)
                }
            });

        Self {
            input_tokens,
            output_tokens,
            thinking_tokens: metadata.thinking_tokens,
            cached_tokens: metadata.cached_tokens,
            grounding_query_count: metadata
                .grounding_metadata
                .as_ref()
                .and_then(|grounding| grounding.web_search_queries.as_ref())
                .map_or(0, |queries| queries.len() as u64),
            code_execution_success_count,
            code_execution_failure_count,
            context_tier: GeminiContextTier::for_input_tokens(input_tokens),
        }
    }
}

/// Public snapshot of the richer per-model observation state.
#[derive(Debug, Clone, PartialEq)]
pub struct CascadeObservationStats {
    /// Number of trials recorded for the model.
    pub trials: u64,
    /// Number of successful trials.
    pub successes: u64,
    /// Total citations observed across Perplexity responses.
    pub total_citations: u64,
    /// Average citations per Perplexity response.
    pub avg_citations_per_response: f64,
    /// Total Perplexity search latency in milliseconds.
    pub total_search_latency_ms: u64,
    /// Average Perplexity search latency in milliseconds.
    pub avg_search_latency_ms: f64,
    /// Total observed cost in USD, including request fee.
    pub total_cost_usd: f64,
    /// Average observed cost in USD, including request fee.
    pub avg_cost_usd: f64,
    /// Number of Perplexity requests contributing observation metadata.
    pub perplexity_requests: u64,
    /// Total Gemini thinking tokens observed across responses.
    pub total_gemini_thinking_tokens: u64,
    /// Average Gemini thinking tokens per response.
    pub avg_gemini_thinking_tokens_per_response: f64,
    /// Total Gemini cached tokens observed across responses.
    pub total_gemini_cached_tokens: u64,
    /// Average Gemini cached tokens per response.
    pub avg_gemini_cached_tokens_per_response: f64,
    /// Total Gemini grounding queries executed across responses.
    pub total_gemini_grounding_queries: u64,
    /// Average Gemini grounding queries per response.
    pub avg_gemini_grounding_queries_per_response: f64,
    /// Number of successful Gemini code-execution outcomes.
    pub gemini_code_execution_successes: u64,
    /// Number of failed Gemini code-execution outcomes.
    pub gemini_code_execution_failures: u64,
    /// Success rate across Gemini code-execution outcomes.
    pub gemini_code_execution_success_rate: f64,
    /// Number of Gemini requests contributing observation metadata.
    pub gemini_requests: u64,
    /// Gemini requests routed in the ≤200K context tier.
    pub gemini_context_window_le_200k_requests: u64,
    /// Gemini requests routed in the >200K context tier.
    pub gemini_context_window_gt_200k_requests: u64,
}

#[derive(Debug, Clone, Copy)]
struct PerplexityObservationTotals {
    citation_count: u64,
    search_latency_ms: u64,
    total_cost_usd: f64,
}

#[derive(Debug, Clone, Copy)]
struct GeminiObservationTotals {
    thinking_tokens: u64,
    cached_tokens: u64,
    grounding_query_count: u64,
    code_execution_success_count: u64,
    code_execution_failure_count: u64,
    context_tier: GeminiContextTier,
}

// ─── Static role -> model table ─────────────────────────────────────────────

/// Build the default static role-to-model mapping.
///
/// Fast-tier roles prefer Gemini Flash-Lite, Standard-tier roles prefer
/// Gemini Flash, and Premium-tier roles prefer Opus with Gemini Pro Preview
/// as the premium fallback.
fn default_role_model_table(model_slugs: &[String]) -> HashMap<AgentRole, String> {
    let mut table = HashMap::new();

    // Research role → Perplexity Sonar when available, standard-tier fallback.
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
                pick_static_slug(model_slugs, &["gemini-2.5-flash-lite", "claude-haiku-3-5"])
            }
            ModelTier::Premium => pick_static_slug(
                model_slugs,
                &["claude-opus-4", "gemini-3.1-pro-preview", "gemini-2.5-pro"],
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

fn pick_static_slug(model_slugs: &[String], candidates: &[&str]) -> String {
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

fn pick_available_static_slug(model_slugs: &[String], candidates: &[&str]) -> String {
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
const fn default_latency_sla(tier: ModelTier) -> u64 {
    match tier {
        ModelTier::Fast => 10_000,
        ModelTier::Premium => 120_000,
        // Standard and forward-compat
        _ => 30_000,
    }
}

/// Map a model slug to an approximate tier for SLA purposes.
fn slug_to_tier(slug: &str) -> ModelTier {
    if slug.contains("gemini-2.5-flash-lite")
        || slug.contains("gemini-3.1-flash-lite-preview")
        || slug.contains("haiku")
    {
        ModelTier::Fast
    } else if slug.contains("gemini-3.1-pro-preview")
        || slug.contains("opus")
        || slug.contains("premium")
    {
        ModelTier::Premium
    } else {
        ModelTier::Standard
    }
}

/// Determine the fallback model slug for a given primary tier.
fn fallback_for_tier(tier: ModelTier) -> Option<String> {
    match tier {
        ModelTier::Fast => None, // no fallback below fast
        ModelTier::Standard => Some("claude-haiku-3-5".to_string()),
        // Premium and forward-compat: fall back to sonnet
        _ => Some("claude-sonnet-4-5".to_string()),
    }
}

fn low_confidence_tier_bonus(tier: ModelTier) -> f64 {
    match tier {
        ModelTier::Premium => 0.15,
        ModelTier::Standard => 0.05,
        ModelTier::Fast => 0.0,
        _ => 0.05,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThinkingPreference {
    Neutral,
    PreferThinking,
    PreferNonThinking,
}

fn thinking_preference(ctx: &RoutingContext) -> ThinkingPreference {
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

fn model_supports_thinking(slug: &str) -> bool {
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

fn thinking_filtered_candidates(candidates: &[String], ctx: &RoutingContext) -> Vec<String> {
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

fn pick_tier_extreme(candidates: &[String], prefer_strongest: bool) -> Option<String> {
    let mut iter = candidates.iter();
    let first = iter.next()?.clone();
    let mut best = first;
    let mut best_rank = model_tier_rank(slug_to_tier(&best));

    for slug in iter {
        let rank = model_tier_rank(slug_to_tier(slug));
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

fn apply_cache_affinity(scores: &mut [(String, f64)], previous_model: Option<&str>) {
    if let Some(prev) = previous_model {
        for (slug, score) in scores.iter_mut() {
            if slug == prev {
                *score += CACHE_AFFINITY_BONUS;
            }
        }
    }
}

fn model_tier_rank(tier: ModelTier) -> u8 {
    match tier {
        ModelTier::Premium => 2,
        ModelTier::Standard => 1,
        ModelTier::Fast => 0,
        _ => 1,
    }
}

fn slugs_match(lhs: &str, rhs: &str) -> bool {
    lhs == rhs || slug_family(lhs).is_some_and(|family| slug_family(rhs) == Some(family))
}

fn slug_family(slug: &str) -> Option<&'static str> {
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
    } else {
        None
    }
}

fn default_cost_table() -> &'static CostTable {
    static COST_TABLE: OnceLock<CostTable> = OnceLock::new();
    COST_TABLE.get_or_init(CostTable::default)
}

fn estimate_total_cost_usd(model_slug: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    default_cost_table()
        .lookup(model_slug)
        .map(|pricing| pricing.estimate_total(input_tokens, output_tokens))
        .unwrap_or(0.0)
}

// ─── CascadeRouter ──────────────────────────────────────────────────────────

/// Three-stage cascade router: Static -> Confidence -> UCB.
///
/// Thread-safe: wrap in `Arc` for shared access.
pub struct CascadeRouter {
    /// The `LinUCB` router used for stage 3 (and observations from all stages).
    linucb: LinUCBRouter,
    /// Per-model statistics for the confidence stage.
    confidence_stats: Mutex<HashMap<String, ModelStats>>,
    /// Cached Pareto frontier used to down-weight dominated models during UCB.
    pareto_frontier: Mutex<ParetoFrontierState>,
    /// Static role -> model table for stage 1.
    role_table: HashMap<AgentRole, String>,
    /// Ordered list of model slugs (arms available to the router).
    model_slugs: Vec<String>,
    /// Optional free-tier Gemini runner used for shadow evaluation.
    free_tier_shadow_runner: Option<Arc<dyn ShadowModelRunner>>,
}

/// Cached Pareto frontier state.
#[derive(Debug, Clone, Default)]
struct ParetoFrontierState {
    frontier: Vec<String>,
    bucket: u64,
}

impl CascadeRouter {
    /// Create a cascade router with the given model slugs.
    ///
    /// # Panics
    ///
    /// Panics if `model_slugs` is empty.
    pub fn new(model_slugs: Vec<String>) -> Self {
        assert!(
            !model_slugs.is_empty(),
            "CascadeRouter: need at least one model"
        );
        Self {
            linucb: LinUCBRouter::new(model_slugs.clone()),
            confidence_stats: Mutex::new(HashMap::new()),
            pareto_frontier: Mutex::new(ParetoFrontierState::default()),
            role_table: default_role_model_table(&model_slugs),
            model_slugs,
            free_tier_shadow_runner: None,
        }
    }

    /// Override the static role table (builder pattern).
    #[must_use]
    pub fn with_role_table(mut self, table: HashMap<AgentRole, String>) -> Self {
        self.role_table = table;
        self
    }

    /// Override the `LinUCB` router (builder pattern, for injecting pre-trained state).
    #[must_use]
    pub fn with_linucb(mut self, linucb: LinUCBRouter) -> Self {
        self.linucb = linucb;
        self
    }

    /// Enable free-tier Gemini shadow evaluation with the provided runner.
    #[must_use]
    pub fn with_free_tier_shadow_runner(mut self, runner: Arc<dyn ShadowModelRunner>) -> Self {
        self.free_tier_shadow_runner = Some(runner);
        self
    }

    /// Determine the current cascade stage based on total observations.
    #[must_use]
    pub fn current_stage(&self) -> CascadeStage {
        stage_for_observations(self.linucb.total_observations())
    }

    /// Total observations recorded across all stages.
    #[must_use]
    pub fn total_observations(&self) -> u64 {
        self.linucb.total_observations()
    }

    /// Select a model from a raw context vector.
    #[must_use]
    pub fn select(&self, context_vec: Vec<f64>) -> CascadeSelection {
        let observations = self.total_observations();
        let stage = stage_for_observations(observations);
        let model = self.linucb.select_features(&context_vec);
        CascadeSelection {
            model,
            observations,
            stage,
        }
    }

    /// Select a model for a given operating frequency.
    ///
    /// - `Gamma` returns `None` because reactive work is pure logic and should
    ///   not dispatch an LLM turn.
    /// - `Theta` uses the existing cascade router selection.
    /// - `Delta` always uses the strongest available model in the router.
    #[must_use]
    pub fn select_for_frequency(
        &self,
        frequency: OperatingFrequency,
        ctx: Option<&RoutingContext>,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> Option<ModelSpec> {
        match frequency {
            OperatingFrequency::Gamma => None,
            OperatingFrequency::Theta => {
                ctx.map(|ctx| self.route_with_cfactor(ctx, cfactor, agent_id).primary)
            }
            OperatingFrequency::Delta => {
                Some(self.bias_model_for_cfactor(self.strongest_model(), cfactor, agent_id))
            }
        }
    }

    /// Select a model for a given operating frequency from a candidate subset.
    ///
    /// When `candidates` is empty, the full router arm set is used.
    #[must_use]
    pub fn select_for_frequency_among(
        &self,
        frequency: OperatingFrequency,
        ctx: Option<&RoutingContext>,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
        candidates: &[String],
    ) -> Option<ModelSpec> {
        let candidates = if candidates.is_empty() {
            &self.model_slugs
        } else {
            candidates
        };

        match frequency {
            OperatingFrequency::Gamma => None,
            OperatingFrequency::Theta => ctx.map(|ctx| {
                self.route_with_cfactor_among(ctx, candidates, cfactor, agent_id)
                    .primary
            }),
            OperatingFrequency::Delta => Some(self.bias_model_for_cfactor_among(
                self.strongest_model_among(candidates),
                cfactor,
                agent_id,
                candidates,
            )),
        }
    }

    /// Return the strongest model currently available to the router.
    ///
    /// Preference order is premium > standard > fast. Within the same tier,
    /// the first slug wins so the choice stays stable.
    #[must_use]
    pub fn strongest_model(&self) -> ModelSpec {
        let mut best_slug = self
            .model_slugs
            .first()
            .cloned()
            .expect("CascadeRouter: need at least one model");
        let mut best_rank = model_tier_rank(slug_to_tier(&best_slug));

        for slug in self.model_slugs.iter().skip(1) {
            let rank = model_tier_rank(slug_to_tier(slug));
            if rank > best_rank {
                best_rank = rank;
                best_slug.clone_from(slug);
            }
        }

        ModelSpec::from_slug(best_slug)
    }

    /// Return the cheapest model currently available to the router.
    ///
    /// Preference order is fast < standard < premium. Within the same tier,
    /// the first slug wins so the choice stays stable.
    #[must_use]
    pub fn cheapest_model(&self) -> ModelSpec {
        let mut best_slug = self
            .model_slugs
            .first()
            .cloned()
            .expect("CascadeRouter: need at least one model");
        let mut best_rank = model_tier_rank(slug_to_tier(&best_slug));

        for slug in self.model_slugs.iter().skip(1) {
            let rank = model_tier_rank(slug_to_tier(slug));
            if rank < best_rank {
                best_rank = rank;
                best_slug.clone_from(slug);
            }
        }

        ModelSpec::from_slug(best_slug)
    }

    /// Return the strongest model from `candidates`.
    #[must_use]
    pub fn strongest_model_among(&self, candidates: &[String]) -> ModelSpec {
        let mut best_slug = candidates
            .first()
            .cloned()
            .unwrap_or_else(|| self.strongest_model().slug);
        let mut best_rank = model_tier_rank(slug_to_tier(&best_slug));

        for slug in candidates.iter().skip(1) {
            let rank = model_tier_rank(slug_to_tier(slug));
            if rank > best_rank {
                best_rank = rank;
                best_slug.clone_from(slug);
            }
        }

        ModelSpec::from_slug(best_slug)
    }

    /// Return the cheapest model from `candidates`.
    #[must_use]
    pub fn cheapest_model_among(&self, candidates: &[String]) -> ModelSpec {
        let mut best_slug = candidates
            .first()
            .cloned()
            .unwrap_or_else(|| self.cheapest_model().slug);
        let mut best_rank = model_tier_rank(slug_to_tier(&best_slug));

        for slug in candidates.iter().skip(1) {
            let rank = model_tier_rank(slug_to_tier(slug));
            if rank < best_rank {
                best_rank = rank;
                best_slug.clone_from(slug);
            }
        }

        ModelSpec::from_slug(best_slug)
    }

    /// Return the index of `slug` in the router's model list.
    #[must_use]
    pub fn model_index_for_slug(&self, slug: &str) -> Option<usize> {
        self.linucb.model_index(slug)
    }

    /// Route a context through the cascade, returning a recommendation.
    pub fn route(&self, ctx: &RoutingContext) -> CascadeModel {
        self.route_with_cfactor(ctx, None, None)
    }

    /// Route a context through the cascade, excluding models whose provider
    /// is currently unavailable.
    ///
    /// Unknown providers are treated as available so unannotated models keep
    /// participating in routing.
    pub fn route_with_health(
        &self,
        ctx: &RoutingContext,
        health: &ProviderHealthRegistry,
        model_providers: &HashMap<String, String>,
    ) -> CascadeModel {
        let available: Vec<String> = self
            .model_slugs
            .iter()
            .filter(|slug| {
                model_providers
                    .get(slug.as_str())
                    .map(|provider_id| health.is_available(provider_id))
                    .unwrap_or(true)
            })
            .cloned()
            .collect();

        if available.is_empty() {
            return self.route(ctx);
        }

        match self.current_stage() {
            CascadeStage::Static => self.route_static_filtered(ctx, &available),
            CascadeStage::Confidence => self.route_confidence_filtered(ctx, &available),
            CascadeStage::Ucb => self.route_ucb_filtered(ctx, &available),
        }
    }

    /// Route a context through the cascade, optionally biasing by C-Factor.
    pub fn route_with_cfactor(
        &self,
        ctx: &RoutingContext,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        match self.current_stage() {
            CascadeStage::Static => self.route_static(ctx, cfactor, agent_id),
            CascadeStage::Confidence => self.route_confidence(ctx, cfactor, agent_id),
            CascadeStage::Ucb => self.route_ucb(ctx, cfactor, agent_id),
        }
    }

    /// Route a context through the cascade over a candidate subset,
    /// optionally biasing by C-Factor.
    pub fn route_with_cfactor_among(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let candidates = if candidates.is_empty() {
            &self.model_slugs
        } else {
            candidates
        };

        match self.current_stage() {
            CascadeStage::Static => self.route_static_among(ctx, candidates, cfactor, agent_id),
            CascadeStage::Confidence => {
                self.route_confidence_among(ctx, candidates, cfactor, agent_id)
            }
            CascadeStage::Ucb => self.route_ucb_among(ctx, candidates, cfactor, agent_id),
        }
    }

    /// Explain a routing decision over the supplied candidate set.
    #[must_use]
    pub fn explain_routing(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
    ) -> CascadeRoutingExplanation {
        let candidates = if candidates.is_empty() {
            self.model_slugs.clone()
        } else {
            candidates.to_vec()
        };

        let route = match self.current_stage() {
            CascadeStage::Static => self.route_static_filtered(ctx, &candidates),
            CascadeStage::Confidence => self.route_confidence_filtered(ctx, &candidates),
            CascadeStage::Ucb => self.route_ucb_filtered(ctx, &candidates),
        };

        let frontier = self.current_pareto_frontier();
        let scores = self.stage_scores(ctx, &candidates, frontier.as_deref());
        let score_map: HashMap<_, _> = scores.into_iter().collect();

        let mut explained: Vec<_> = candidates
            .into_iter()
            .map(|slug| CascadeRoutingCandidate {
                cache_affinity: ctx.previous_model.as_deref() == Some(slug.as_str()),
                pareto_optimal: frontier
                    .as_ref()
                    .map(|frontier| frontier.iter().any(|frontier_slug| frontier_slug == &slug)),
                score: score_map.get(&slug).copied().unwrap_or(0.0),
                selected: slugs_match(&slug, &route.primary.slug),
                model: slug,
            })
            .collect();

        explained.sort_by(|a, b| {
            b.selected
                .cmp(&a.selected)
                .then_with(|| b.score.total_cmp(&a.score))
                .then_with(|| a.model.cmp(&b.model))
        });

        CascadeRoutingExplanation {
            stage: route.stage,
            selected_model: route.primary.slug,
            fallback_model: route.fallback.map(|model| model.slug),
            latency_sla_ms: route.latency_sla_ms,
            candidates: explained,
        }
    }

    /// Record an observation (updates both confidence stats and `LinUCB`).
    pub fn record_observation(
        &self,
        ctx: &RoutingContext,
        model_slug: &str,
        reward: f64,
        success: bool,
    ) {
        let Some(model_idx) = self.model_index_for_slug(model_slug) else {
            return;
        };
        self.observe_internal(&ctx.to_features(), model_idx, reward, success, None, None);
    }

    /// Record an observation enriched with Perplexity search metadata.
    ///
    /// The request cost is estimated from the existing [`CostTable`] using the
    /// model's token pricing plus any configured per-request fee.
    pub fn record_perplexity_observation(
        &self,
        ctx: &RoutingContext,
        model_slug: &str,
        reward: f64,
        success: bool,
        observation: PerplexityObservation,
    ) -> bool {
        let Some(model_idx) = self.model_index_for_slug(model_slug) else {
            return false;
        };

        let perplexity = PerplexityObservationTotals {
            citation_count: observation.citation_count,
            search_latency_ms: observation.search_latency_ms,
            total_cost_usd: estimate_total_cost_usd(
                model_slug,
                observation.input_tokens,
                observation.output_tokens,
            ),
        };
        self.observe_internal(
            &ctx.to_features(),
            model_idx,
            reward,
            success,
            Some(perplexity),
            None,
        );
        true
    }

    /// Record an observation enriched with Gemini-native metadata.
    pub fn record_gemini_observation(
        &self,
        ctx: &RoutingContext,
        model_slug: &str,
        reward: f64,
        success: bool,
        observation: GeminiObservation,
    ) -> bool {
        let Some(model_idx) = self.model_index_for_slug(model_slug) else {
            return false;
        };

        let gemini = GeminiObservationTotals {
            thinking_tokens: observation.thinking_tokens.unwrap_or(0),
            cached_tokens: observation.cached_tokens.unwrap_or(0),
            grounding_query_count: observation.grounding_query_count,
            code_execution_success_count: observation.code_execution_success_count,
            code_execution_failure_count: observation.code_execution_failure_count,
            context_tier: observation.context_tier,
        };
        self.observe_internal(
            &ctx.to_features(),
            model_idx,
            reward,
            success,
            None,
            Some(gemini),
        );
        true
    }

    /// Record a binary outcome for `model_slug` without a full routing context.
    ///
    /// This is used by event-driven feedback paths that only know which model
    /// produced the episode, not the original routing features.
    pub fn record_outcome(&self, model_slug: &str, success: bool) -> bool {
        let Some(model_idx) = self.model_index_for_slug(model_slug) else {
            return false;
        };

        let reward = if success { 1.0 } else { 0.0 };
        let context = [0.0; CONTEXT_DIM];
        self.observe_internal(&context, model_idx, reward, success, None, None);
        true
    }

    /// Run a shadow evaluation against a free-tier Gemini model.
    ///
    /// The shadow result never affects the primary task outcome. When the
    /// shadow response is judged good enough relative to the primary result,
    /// the router records a successful zero-cost observation for `free_model`.
    /// Callers are expected to schedule this work in parallel with the main
    /// request lifecycle.
    pub async fn shadow_evaluate(
        &mut self,
        prompt: &str,
        primary_result: &AgentResult,
        free_model: &str,
    ) {
        if !is_free_tier_gemini_model(free_model) {
            return;
        }

        let Some(model_idx) = self.model_index_for_slug(free_model) else {
            return;
        };
        let Some(runner) = self.free_tier_shadow_runner.clone() else {
            return;
        };

        let prompt = prompt.trim();
        if prompt.is_empty() {
            return;
        }

        let shadow_result = runner.run_shadow(prompt, free_model).await;
        let quality = shadow_quality_score(prompt, primary_result, &shadow_result);
        let passed = quality >= 0.65;
        let ctx = infer_shadow_routing_context(prompt, primary_result);
        let reward = if passed {
            compute_routing_reward_v2(
                quality,
                0.0,
                shadow_result.usage.wall_ms as f64,
                default_latency_sla(slug_to_tier(free_model)) as f64,
            )
        } else {
            0.0
        };

        self.observe_internal(
            &ctx.to_features_for_model(Some(free_model)),
            model_idx,
            reward,
            passed,
            None,
            None,
        );
    }

    /// Record a successful observation from a raw 18-dim context vector.
    ///
    /// This is the success-path entry point used by orchestration when the
    /// caller already has the model index in the router's arm list.
    pub fn observe(&self, context_vec: Vec<f64>, model_idx: usize, reward: f64) {
        self.observe_internal(&context_vec, model_idx, reward, true, None, None);
    }

    fn observe_internal(
        &self,
        context_vec: &[f64],
        model_idx: usize,
        reward: f64,
        success: bool,
        perplexity: Option<PerplexityObservationTotals>,
        gemini: Option<GeminiObservationTotals>,
    ) {
        let Some(slug) = self.model_slugs.get(model_idx) else {
            return;
        };

        // Update confidence stats.
        let mut stats = self.confidence_stats.lock();
        let entry = stats.entry(slug.clone()).or_default();
        entry.trials += 1;
        if success {
            entry.successes += 1;
        }
        if let Some(perplexity) = perplexity {
            entry.total_citations += perplexity.citation_count;
            entry.total_search_latency_ms += perplexity.search_latency_ms;
            entry.total_cost_usd += perplexity.total_cost_usd;
            entry.perplexity_requests += 1;
        }
        if let Some(gemini) = gemini {
            entry.total_gemini_thinking_tokens += gemini.thinking_tokens;
            entry.total_gemini_cached_tokens += gemini.cached_tokens;
            entry.total_gemini_grounding_queries += gemini.grounding_query_count;
            entry.gemini_code_execution_successes += gemini.code_execution_success_count;
            entry.gemini_code_execution_failures += gemini.code_execution_failure_count;
            entry.gemini_requests += 1;
            match gemini.context_tier {
                GeminiContextTier::UpTo200k => entry.gemini_context_window_le_200k_requests += 1,
                GeminiContextTier::Over200k => entry.gemini_context_window_gt_200k_requests += 1,
            }
        }
        drop(stats);

        // Update LinUCB (always, so it's ready when stage transitions).
        self.linucb.update_features(context_vec, model_idx, reward);
    }

    /// Access the underlying `LinUCB` router (for introspection / persistence).
    pub const fn linucb(&self) -> &LinUCBRouter {
        &self.linucb
    }

    /// Snapshot of confidence-stage statistics.
    pub fn confidence_snapshot(&self) -> HashMap<String, (u64, u64)> {
        self.confidence_stats
            .lock()
            .iter()
            .map(|(k, v)| (k.clone(), (v.trials, v.successes)))
            .collect()
    }

    /// Snapshot of richer per-model observations used by learning loops.
    pub fn observation_snapshot(&self) -> HashMap<String, CascadeObservationStats> {
        self.confidence_stats
            .lock()
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    CascadeObservationStats {
                        trials: v.trials,
                        successes: v.successes,
                        total_citations: v.total_citations,
                        avg_citations_per_response: v.avg_citations_per_response(),
                        total_search_latency_ms: v.total_search_latency_ms,
                        avg_search_latency_ms: v.avg_search_latency_ms(),
                        total_cost_usd: v.total_cost_usd,
                        avg_cost_usd: v.avg_cost_usd(),
                        perplexity_requests: v.perplexity_requests,
                        total_gemini_thinking_tokens: v.total_gemini_thinking_tokens,
                        avg_gemini_thinking_tokens_per_response: v
                            .avg_gemini_thinking_tokens_per_response(),
                        total_gemini_cached_tokens: v.total_gemini_cached_tokens,
                        avg_gemini_cached_tokens_per_response: v
                            .avg_gemini_cached_tokens_per_response(),
                        total_gemini_grounding_queries: v.total_gemini_grounding_queries,
                        avg_gemini_grounding_queries_per_response: v
                            .avg_gemini_grounding_queries_per_response(),
                        gemini_code_execution_successes: v.gemini_code_execution_successes,
                        gemini_code_execution_failures: v.gemini_code_execution_failures,
                        gemini_code_execution_success_rate: v.gemini_code_execution_success_rate(),
                        gemini_requests: v.gemini_requests,
                        gemini_context_window_le_200k_requests: v
                            .gemini_context_window_le_200k_requests,
                        gemini_context_window_gt_200k_requests: v
                            .gemini_context_window_gt_200k_requests,
                    },
                )
            })
            .collect()
    }

    /// Save confidence stats, model slugs, and total observation count to a JSON file.
    ///
    /// `LinUCB` arm weights are not persisted (they re-learn from new observations).
    /// Confidence stats represent the accumulated pass-rate history needed for
    /// stage-2 routing, and the total observation count determines which cascade
    /// stage is active after reload.
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let snapshot = CascadeSnapshot {
            model_slugs: self.model_slugs.clone(),
            confidence_stats: self
                .confidence_stats
                .lock()
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        PersistedModelStats {
                            trials: v.trials,
                            successes: v.successes,
                            total_citations: v.total_citations,
                            total_search_latency_ms: v.total_search_latency_ms,
                            total_cost_usd: v.total_cost_usd,
                            perplexity_requests: v.perplexity_requests,
                            total_gemini_thinking_tokens: v.total_gemini_thinking_tokens,
                            total_gemini_cached_tokens: v.total_gemini_cached_tokens,
                            total_gemini_grounding_queries: v.total_gemini_grounding_queries,
                            gemini_code_execution_successes: v.gemini_code_execution_successes,
                            gemini_code_execution_failures: v.gemini_code_execution_failures,
                            gemini_context_window_le_200k_requests: v
                                .gemini_context_window_le_200k_requests,
                            gemini_context_window_gt_200k_requests: v
                                .gemini_context_window_gt_200k_requests,
                            gemini_requests: v.gemini_requests,
                        },
                    )
                })
                .collect(),
            total_observations: self.linucb.total_observations(),
        };
        let json = serde_json::to_string_pretty(&snapshot)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Load a cascade router from a persisted JSON file, or create a new one.
    ///
    /// If the file exists and parses correctly, the confidence stats are restored
    /// and the model slugs from the file are merged with the provided `model_slugs`
    /// (the union is used). If the file doesn't exist or fails to parse, a fresh
    /// router is created with the given `model_slugs`.
    ///
    /// # Panics
    ///
    /// Panics if `model_slugs` is empty and no persisted state exists.
    pub fn load_or_new(path: &Path, model_slugs: Vec<String>) -> Self {
        let snapshot = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<CascadeSnapshot>(&s).ok());

        match snapshot {
            Some(snap) => {
                // Merge model sets: union of persisted + provided.
                let mut slugs: Vec<String> = snap.model_slugs;
                for s in &model_slugs {
                    if !slugs.contains(s) {
                        slugs.push(s.clone());
                    }
                }
                if slugs.is_empty() {
                    slugs = model_slugs;
                }
                assert!(!slugs.is_empty(), "CascadeRouter: need at least one model");
                let router = Self::new(slugs);
                // Restore confidence stats.
                let mut stats = router.confidence_stats.lock();
                for (model, persisted) in &snap.confidence_stats {
                    stats.insert(
                        model.clone(),
                        ModelStats {
                            trials: persisted.trials,
                            successes: persisted.successes,
                            total_citations: persisted.total_citations,
                            total_search_latency_ms: persisted.total_search_latency_ms,
                            total_cost_usd: persisted.total_cost_usd,
                            perplexity_requests: persisted.perplexity_requests,
                            total_gemini_thinking_tokens: persisted.total_gemini_thinking_tokens,
                            total_gemini_cached_tokens: persisted.total_gemini_cached_tokens,
                            total_gemini_grounding_queries: persisted
                                .total_gemini_grounding_queries,
                            gemini_code_execution_successes: persisted
                                .gemini_code_execution_successes,
                            gemini_code_execution_failures: persisted
                                .gemini_code_execution_failures,
                            gemini_context_window_le_200k_requests: persisted
                                .gemini_context_window_le_200k_requests,
                            gemini_context_window_gt_200k_requests: persisted
                                .gemini_context_window_gt_200k_requests,
                            gemini_requests: persisted.gemini_requests,
                        },
                    );
                }
                drop(stats);

                // Restore total observation count so the cascade stage is correct.
                // If the snapshot predates the `total_observations` field (default 0),
                // recompute from the sum of per-model trials.
                let total = if snap.total_observations > 0 {
                    snap.total_observations
                } else {
                    snap.confidence_stats.values().map(|s| s.trials).sum()
                };
                router.linucb.set_total_observations(total);

                router
            }
            None => Self::new(model_slugs),
        }
    }

    // ── Internal routing per stage ──────────────────────────────────────

    fn route_static(
        &self,
        ctx: &RoutingContext,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        if let Some(thinking_selected) = match thinking_preference(ctx) {
            ThinkingPreference::PreferThinking => {
                pick_tier_extreme(&thinking_filtered_candidates(&self.model_slugs, ctx), true)
            }
            ThinkingPreference::PreferNonThinking => {
                pick_tier_extreme(&thinking_filtered_candidates(&self.model_slugs, ctx), false)
            }
            ThinkingPreference::Neutral => None,
        } {
            let selected = self.bias_model_for_cfactor(
                ModelSpec::from_slug(thinking_selected),
                cfactor,
                agent_id,
            );
            let tier = slug_to_tier(&selected.slug);
            let fallback = fallback_for_tier(tier).map(ModelSpec::from_slug);

            return CascadeModel {
                primary: selected,
                fallback,
                latency_sla_ms: default_latency_sla(tier),
                stage: CascadeStage::Static,
            };
        }

        // For research tasks, prefer Perplexity Sonar when available.
        let slug = if ctx.task_category == TaskCategory::Research {
            self.model_slugs
                .iter()
                .find(|s| s.as_str() == "sonar-pro" || s.as_str() == "sonar")
                .cloned()
                .unwrap_or_else(|| {
                    self.role_table
                        .get(&ctx.role)
                        .cloned()
                        .unwrap_or_else(|| "claude-sonnet-4-5".to_string())
                })
        } else {
            self.role_table
                .get(&ctx.role)
                .cloned()
                .unwrap_or_else(|| "claude-sonnet-4-5".to_string())
        };

        let selected = self.bias_model_for_cfactor(ModelSpec::from_slug(&slug), cfactor, agent_id);
        let tier = slug_to_tier(&selected.slug);
        let fallback = fallback_for_tier(tier).map(ModelSpec::from_slug);

        CascadeModel {
            primary: selected,
            fallback,
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Static,
        }
    }

    fn route_static_filtered(&self, ctx: &RoutingContext, candidates: &[String]) -> CascadeModel {
        if let Some(thinking_selected) = match thinking_preference(ctx) {
            ThinkingPreference::PreferThinking => {
                pick_tier_extreme(&thinking_filtered_candidates(candidates, ctx), true)
            }
            ThinkingPreference::PreferNonThinking => {
                pick_tier_extreme(&thinking_filtered_candidates(candidates, ctx), false)
            }
            ThinkingPreference::Neutral => None,
        } {
            let selected = ModelSpec::from_slug(thinking_selected);
            let tier = slug_to_tier(&selected.slug);
            let fallback = fallback_for_tier(tier).map(ModelSpec::from_slug);

            return CascadeModel {
                primary: selected,
                fallback,
                latency_sla_ms: default_latency_sla(tier),
                stage: CascadeStage::Static,
            };
        }

        let slug = self
            .role_table
            .get(&ctx.role)
            .cloned()
            .unwrap_or_else(|| "claude-sonnet-4-5".to_string());

        let selected_slug = if candidates
            .iter()
            .any(|candidate| slugs_match(candidate, &slug))
        {
            slug
        } else {
            let tier_candidates: &[&str] = match ctx.role.model_tier() {
                ModelTier::Fast => &["gemini-2.5-flash-lite", "claude-haiku-3-5"],
                ModelTier::Premium => {
                    &["claude-opus-4", "gemini-3.1-pro-preview", "gemini-2.5-pro"]
                }
                _ => &[
                    "gemini-2.5-flash",
                    "gemini-2.5-pro",
                    "kimi-k2.5",
                    "kimi-k2-thinking",
                    "claude-sonnet-4-6",
                    "claude-sonnet-4-5",
                ],
            };
            pick_available_static_slug(candidates, tier_candidates)
        };
        let selected = ModelSpec::from_slug(selected_slug);
        let tier = slug_to_tier(&selected.slug);
        let fallback = fallback_for_tier(tier).map(ModelSpec::from_slug);

        CascadeModel {
            primary: selected,
            fallback,
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Static,
        }
    }

    fn route_static_among(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let mut route = self.route_static_filtered(ctx, candidates);
        route.primary =
            self.bias_model_for_cfactor_among(route.primary, cfactor, agent_id, candidates);
        route
    }

    fn route_confidence(
        &self,
        ctx: &RoutingContext,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let thinking_candidates = thinking_filtered_candidates(&self.model_slugs, ctx);
        let scores = self.confidence_scores(&thinking_candidates, ctx);
        let best_slug = choose_best_scored_slug(scores);

        let selected =
            self.bias_model_for_cfactor(ModelSpec::from_slug(&best_slug), cfactor, agent_id);
        let tier = slug_to_tier(&selected.slug);
        let fallback = fallback_for_tier(tier).map(ModelSpec::from_slug);

        CascadeModel {
            primary: selected,
            fallback,
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Confidence,
        }
    }

    fn route_confidence_filtered(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
    ) -> CascadeModel {
        let thinking_candidates = thinking_filtered_candidates(candidates, ctx);
        let scores = self.confidence_scores(&thinking_candidates, ctx);
        let best_slug = choose_best_scored_slug(scores);

        let selected = ModelSpec::from_slug(best_slug);
        let tier = slug_to_tier(&selected.slug);
        let fallback = fallback_for_tier(tier).map(ModelSpec::from_slug);

        CascadeModel {
            primary: selected,
            fallback,
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Confidence,
        }
    }

    fn route_confidence_among(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let mut route = self.route_confidence_filtered(ctx, candidates);
        route.primary =
            self.bias_model_for_cfactor_among(route.primary, cfactor, agent_id, candidates);
        route
    }

    fn route_ucb(
        &self,
        ctx: &RoutingContext,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let thinking_candidates = thinking_filtered_candidates(&self.model_slugs, ctx);
        let model = self.select_ucb_model(ctx, &thinking_candidates);
        let selected = self.bias_model_for_cfactor(model, cfactor, agent_id);
        let tier = slug_to_tier(&selected.slug);
        let fallback = fallback_for_tier(tier).map(ModelSpec::from_slug);

        CascadeModel {
            primary: selected,
            fallback,
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Ucb,
        }
    }

    fn route_ucb_filtered(&self, ctx: &RoutingContext, candidates: &[String]) -> CascadeModel {
        let thinking_candidates = thinking_filtered_candidates(candidates, ctx);
        let model = self.select_ucb_model(ctx, &thinking_candidates);
        let selected = model;
        let tier = slug_to_tier(&selected.slug);
        let fallback = fallback_for_tier(tier).map(ModelSpec::from_slug);

        CascadeModel {
            primary: selected,
            fallback,
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Ucb,
        }
    }

    fn route_ucb_among(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let mut route = self.route_ucb_filtered(ctx, candidates);
        route.primary =
            self.bias_model_for_cfactor_among(route.primary, cfactor, agent_id, candidates);
        route
    }

    /// Apply a C-Factor-based bias to a selected model.
    fn bias_model_for_cfactor(
        &self,
        model: ModelSpec,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> ModelSpec {
        let Some(cfactor) = cfactor else {
            return model;
        };

        if let Some(agent_id) = agent_id {
            match cfactor.dispatch_bias_for_agent(agent_id) {
                AgentDispatchBias::PreferStronger => return self.strongest_model(),
                AgentDispatchBias::PreferCheaper => return self.cheapest_model(),
                AgentDispatchBias::Neutral => {}
            }
        }

        if cfactor.overall > HIGH_CFACTOR_THRESHOLD {
            self.cheapest_model()
        } else if cfactor.overall < LOW_CFACTOR_THRESHOLD {
            self.strongest_model()
        } else {
            model
        }
    }

    fn bias_model_for_cfactor_among(
        &self,
        model: ModelSpec,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
        candidates: &[String],
    ) -> ModelSpec {
        let Some(cfactor) = cfactor else {
            return model;
        };

        if let Some(agent_id) = agent_id {
            match cfactor.dispatch_bias_for_agent(agent_id) {
                AgentDispatchBias::PreferStronger => return self.strongest_model_among(candidates),
                AgentDispatchBias::PreferCheaper => return self.cheapest_model_among(candidates),
                AgentDispatchBias::Neutral => {}
            }
        }

        if cfactor.overall > HIGH_CFACTOR_THRESHOLD {
            self.cheapest_model_among(candidates)
        } else if cfactor.overall < LOW_CFACTOR_THRESHOLD {
            self.strongest_model_among(candidates)
        } else {
            model
        }
    }

    fn confidence_scores(&self, candidates: &[String], ctx: &RoutingContext) -> Vec<(String, f64)> {
        let stats = self.confidence_stats.lock();
        let low_confidence = ctx.affect_confidence < LOW_AFFECT_CONFIDENCE_THRESHOLD;

        let mut scores: Vec<(String, f64)> = candidates
            .iter()
            .map(|slug| {
                let s = stats.get(slug).cloned().unwrap_or_default();
                let tier_bonus = if low_confidence {
                    low_confidence_tier_bonus(slug_to_tier(slug))
                } else {
                    0.0
                };
                (slug.clone(), s.upper_bound() + tier_bonus)
            })
            .collect();
        drop(stats);

        apply_cache_affinity(&mut scores, ctx.previous_model.as_deref());
        scores
    }

    fn select_ucb_model(&self, ctx: &RoutingContext, candidates: &[String]) -> ModelSpec {
        self.refresh_pareto_frontier_if_needed();

        let frontier = {
            let state = self.pareto_frontier.lock();
            if state.bucket == 0 || state.frontier.is_empty() {
                None
            } else {
                Some(state.frontier.clone())
            }
        };

        if let Some(frontier) = frontier {
            let base_alpha = self.linucb.current_alpha();
            self.linucb
                .select_features_from_candidates_with_alpha_adjuster(ctx, candidates, |slug| {
                    pareto_adjusted_alpha(base_alpha, slug, &frontier)
                })
        } else if candidates.len() == self.model_slugs.len() {
            self.linucb.select_model(ctx)
        } else {
            self.linucb.select_features_from_candidates(ctx, candidates)
        }
    }

    fn refresh_pareto_frontier_if_needed(&self) {
        let total = self.total_observations();
        if total < PARETO_RECOMPUTE_INTERVAL {
            return;
        }

        let bucket = total / PARETO_RECOMPUTE_INTERVAL;
        let needs_refresh = {
            let state = self.pareto_frontier.lock();
            state.bucket < bucket || state.frontier.is_empty()
        };

        if !needs_refresh {
            return;
        }

        let frontier = self.recompute_pareto_frontier();
        let mut state = self.pareto_frontier.lock();
        if state.bucket < bucket || state.frontier.is_empty() {
            state.frontier = frontier;
            state.bucket = bucket;
        }
    }

    fn current_pareto_frontier(&self) -> Option<Vec<String>> {
        self.refresh_pareto_frontier_if_needed();
        let state = self.pareto_frontier.lock();
        if state.bucket == 0 || state.frontier.is_empty() {
            None
        } else {
            Some(state.frontier.clone())
        }
    }

    fn stage_scores(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
        frontier: Option<&[String]>,
    ) -> Vec<(String, f64)> {
        match self.current_stage() {
            CascadeStage::Static => {
                let selected = self.route_static_filtered(ctx, candidates).primary.slug;
                candidates
                    .iter()
                    .map(|slug| {
                        (
                            slug.clone(),
                            if slugs_match(slug, &selected) {
                                1.0
                            } else {
                                0.0
                            },
                        )
                    })
                    .collect()
            }
            CascadeStage::Confidence => {
                let thinking_candidates = thinking_filtered_candidates(candidates, ctx);
                let score_map: HashMap<_, _> = self
                    .confidence_scores(&thinking_candidates, ctx)
                    .into_iter()
                    .collect();
                candidates
                    .iter()
                    .map(|slug| (slug.clone(), score_map.get(slug).copied().unwrap_or(0.0)))
                    .collect()
            }
            CascadeStage::Ucb => {
                let thinking_candidates = thinking_filtered_candidates(candidates, ctx);
                let base_alpha = self.linucb.current_alpha();
                let frontier = frontier.map(|frontier| frontier.to_vec());
                let score_map: HashMap<_, _> = self
                    .linucb
                    .score_features_from_candidates_with_alpha_adjuster(
                        ctx,
                        &thinking_candidates,
                        |slug| {
                            frontier.as_ref().map_or(base_alpha, |frontier| {
                                pareto_adjusted_alpha(base_alpha, slug, frontier)
                            })
                        },
                    )
                    .into_iter()
                    .collect();
                candidates
                    .iter()
                    .map(|slug| (slug.clone(), score_map.get(slug).copied().unwrap_or(0.0)))
                    .collect()
            }
        }
    }

    fn recompute_pareto_frontier(&self) -> Vec<String> {
        let stats = self.confidence_stats.lock();
        let mut observations = HashMap::new();
        let mut unobserved = Vec::new();

        for slug in &self.model_slugs {
            match stats.get(slug) {
                Some(model_stats) if model_stats.trials > 0 => {
                    observations.insert(
                        slug.clone(),
                        ModelObservation {
                            pass_rate: model_stats.pass_rate(),
                            cost_per_success: model_stats.cost_per_success().unwrap_or_else(|| {
                                pareto_cost_proxy(slug) / model_stats.pass_rate().max(0.01)
                            }),
                            avg_latency_ms: if model_stats.perplexity_requests > 0 {
                                model_stats.avg_search_latency_ms()
                            } else {
                                pareto_latency_proxy(slug)
                            },
                            observations: model_stats.trials,
                        },
                    );
                }
                _ => unobserved.push(slug.clone()),
            }
        }
        drop(stats);

        let mut frontier = if observations.is_empty() {
            Vec::new()
        } else {
            compute_pareto_frontier(&observations)
        };
        frontier.extend(unobserved);
        frontier.sort();
        frontier.dedup();
        frontier
    }
}

fn choose_best_scored_slug(scores: Vec<(String, f64)>) -> String {
    let mut iter = scores.into_iter();
    let Some((mut best_slug, mut best_score)) = iter.next() else {
        unreachable!("CascadeRouter: confidence scoring requires at least one candidate");
    };

    for (slug, score) in iter {
        if score > best_score {
            best_score = score;
            best_slug = slug;
        }
    }

    best_slug
}

fn pareto_adjusted_alpha(base_alpha: f64, slug: &str, frontier: &[String]) -> f64 {
    if frontier.iter().any(|frontier_slug| frontier_slug == slug) {
        base_alpha
    } else {
        base_alpha * 0.1
    }
}

fn pareto_cost_proxy(slug: &str) -> f64 {
    match slug_family(slug) {
        Some("gemini-3.1-flash-lite-preview") => 0.9,
        Some("gemini-3-flash-preview") => 1.5,
        Some("haiku") => 1.0,
        Some("sonnet") => 3.0,
        Some("opus") => 9.0,
        Some("kimi-k2") => 2.5,
        _ => match slug_to_tier(slug) {
            ModelTier::Fast => 1.0,
            ModelTier::Premium => 9.0,
            _ => 3.0,
        },
    }
}

fn pareto_latency_proxy(slug: &str) -> f64 {
    default_latency_sla(slug_to_tier(slug)) as f64
}

fn is_free_tier_gemini_model(slug: &str) -> bool {
    let slug = slug.to_ascii_lowercase();
    slug.contains("gemini-2.5-flash")
        || slug.contains("gemini-2.5-flash-lite")
        || slug.contains("gemini-3-flash-preview")
        || slug.contains("gemini-3.1-flash-lite-preview")
}

fn infer_shadow_routing_context(prompt: &str, primary_result: &AgentResult) -> RoutingContext {
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
        affect_confidence: if primary_result.success { 0.7 } else { 0.3 },
        thinking_level: None,
        previous_model: primary_result.output.tag("model").map(str::to_string),
        plan_context_tokens: Some((prompt.len() as u64).div_ceil(4)),
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

fn shadow_quality_score(
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

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

/// Determine the cascade stage from observation count.
const fn stage_for_observations(obs: u64) -> CascadeStage {
    if obs < COLD_START_THRESHOLD {
        CascadeStage::Static
    } else if obs < CONFIDENCE_TO_UCB_THRESHOLD {
        CascadeStage::Confidence
    } else {
        CascadeStage::Ucb
    }
}

// ─── Persistence ────────────────────────────────────────────────────────────

/// Persisted snapshot of cascade router state.
#[derive(Serialize, Deserialize)]
struct CascadeSnapshot {
    model_slugs: Vec<String>,
    confidence_stats: HashMap<String, PersistedModelStats>,
    /// Total observations across all models (used to restore cascade stage).
    ///
    /// Defaults to 0 for backward compatibility with snapshots written before
    /// this field was added; in that case `load_or_new` recomputes the total
    /// from the sum of per-model trials.
    #[serde(default)]
    total_observations: u64,
}

/// Serializable form of per-model confidence stats.
#[derive(Serialize, Deserialize)]
struct PersistedModelStats {
    trials: u64,
    successes: u64,
    #[serde(default)]
    total_citations: u64,
    #[serde(default)]
    total_search_latency_ms: u64,
    #[serde(default)]
    total_cost_usd: f64,
    #[serde(default)]
    perplexity_requests: u64,
    #[serde(default)]
    total_gemini_thinking_tokens: u64,
    #[serde(default)]
    total_gemini_cached_tokens: u64,
    #[serde(default)]
    total_gemini_grounding_queries: u64,
    #[serde(default)]
    gemini_code_execution_successes: u64,
    #[serde(default)]
    gemini_code_execution_failures: u64,
    #[serde(default)]
    gemini_context_window_le_200k_requests: u64,
    #[serde(default)]
    gemini_context_window_gt_200k_requests: u64,
    #[serde(default)]
    gemini_requests: u64,
}

// ─── Tests ────────────────────────────────────────��─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_health::{ErrorClass, ProviderHealthRegistry};
    use async_trait::async_trait;
    use roko_agent::gemini::{CodeExecutionResultPart, GroundingMetadata};
    use roko_core::task::{TaskCategory, TaskComplexityBand};
    use roko_core::{Body, Kind, Signal};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn test_slugs() -> Vec<String> {
        vec![
            "claude-haiku-3-5".to_string(),
            "claude-sonnet-4-5".to_string(),
            "claude-opus-4".to_string(),
        ]
    }

    fn default_ctx() -> RoutingContext {
        RoutingContext {
            task_category: TaskCategory::Implementation,
            complexity: TaskComplexityBand::Standard,
            iteration: 0,
            role: AgentRole::Implementer,
            crate_familiarity: 0.5,
            has_prior_failure: false,
            affect_confidence: 0.5,
            thinking_level: None,
            previous_model: None,
            plan_context_tokens: None,
        }
    }

    struct StubShadowRunner {
        result: AgentResult,
    }

    #[async_trait]
    impl ShadowModelRunner for StubShadowRunner {
        async fn run_shadow(&self, _prompt: &str, _model_slug: &str) -> AgentResult {
            self.result.clone()
        }
    }

    fn agent_result(text: &str, success: bool, model: &str, wall_ms: u64) -> AgentResult {
        let output = Signal::builder(Kind::AgentOutput)
            .body(Body::text(text))
            .tag("model", model)
            .build();

        let usage = roko_agent::Usage {
            wall_ms,
            ..Default::default()
        };

        if success {
            AgentResult::ok(output).with_usage(usage)
        } else {
            AgentResult::fail(output).with_usage(usage)
        }
    }

    // ── Test 1: starts in Static stage ──────────────────────────────────

    #[test]
    fn starts_in_static_stage() {
        let cascade = CascadeRouter::new(test_slugs());
        assert_eq!(cascade.current_stage(), CascadeStage::Static);
    }

    // ── Test 2: static stage uses role table ────────────────────────────

    #[test]
    fn static_stage_uses_role_table() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();
        let result = cascade.route(&ctx);

        // Implementer has Standard tier -> sonnet
        assert_eq!(result.stage, CascadeStage::Static);
        assert_eq!(result.primary.slug, "claude-sonnet-4-5");
    }

    // ── Test 3: static stage gives correct fallback ─────────────────────

    #[test]
    fn static_stage_fallback_for_standard() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();
        let result = cascade.route(&ctx);

        assert!(result.fallback.is_some());
        assert_eq!(result.fallback.as_ref().unwrap().slug, "claude-haiku-3-5");
    }

    // ── Test 4: fast tier has no fallback ────────────────────────────────

    #[test]
    fn fast_tier_no_fallback() {
        let cascade = CascadeRouter::new(test_slugs());
        let mut ctx = default_ctx();
        ctx.role = AgentRole::Conductor; // Fast tier

        let result = cascade.route(&ctx);
        assert!(result.fallback.is_none());
    }

    // ── Test 5: transitions to Confidence at 50 observations ────────────

    #[test]
    fn transitions_to_confidence_stage() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();

        // Feed 50 observations to cross the threshold.
        for _ in 0..50 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.8, true);
        }

        assert_eq!(cascade.current_stage(), CascadeStage::Confidence);
        let result = cascade.route(&ctx);
        assert_eq!(result.stage, CascadeStage::Confidence);
    }

    // ── Test 6: transitions to UCB at 200 observations ──────────────────

    #[test]
    fn transitions_to_ucb_stage() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();

        // Feed 200 observations.
        for _ in 0..200 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.8, true);
        }

        assert_eq!(cascade.current_stage(), CascadeStage::Ucb);
        let result = cascade.route(&ctx);
        assert_eq!(result.stage, CascadeStage::Ucb);
    }

    // ── Test 7: confidence stage prefers high-success model ─────────────

    #[test]
    fn confidence_stage_prefers_high_success_model() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();

        // Build up observations: sonnet mostly succeeds, haiku mostly fails.
        for i in 0..80 {
            if i < 25 {
                cascade.record_observation(&ctx, "claude-haiku-3-5", 0.2, false);
            } else if i < 50 {
                cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.9, true);
            } else if i < 65 {
                cascade.record_observation(&ctx, "claude-haiku-3-5", 0.2, false);
            } else {
                cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.9, true);
            }
        }

        assert_eq!(cascade.current_stage(), CascadeStage::Confidence);

        let result = cascade.route(&ctx);
        // Sonnet should have higher upper bound than haiku
        // (sonnet: 25/25 = 100%, haiku: 0/40 = 0%)
        assert_eq!(
            result.primary.slug, "claude-sonnet-4-5",
            "confidence stage should prefer the high-pass-rate model"
        );
    }

    // ── Test 7b: low affect confidence biases toward stronger model ──────

    #[test]
    fn low_affect_confidence_prefers_opus_over_sonnet() {
        let cascade = CascadeRouter::new(test_slugs());
        let mut ctx = default_ctx();

        for _ in 0..20 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.9, true);
        }
        for _ in 0..15 {
            cascade.record_observation(&ctx, "claude-opus-4", 0.9, true);
        }
        for _ in 0..10 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.1, false);
        }
        for _ in 0..5 {
            cascade.record_observation(&ctx, "claude-opus-4", 0.1, false);
        }

        assert_eq!(cascade.current_stage(), CascadeStage::Confidence);

        ctx.affect_confidence = 0.2;
        let low_confidence = cascade.route(&ctx);
        assert_eq!(
            low_confidence.primary.slug, "claude-opus-4",
            "low affect confidence should bias toward the stronger premium model"
        );

        ctx.affect_confidence = 0.9;
        let high_confidence = cascade.route(&ctx);
        // High confidence allows routing to cheaper models
        assert!(
            ["claude-haiku-3-5", "claude-sonnet-4-5"]
                .contains(&high_confidence.primary.slug.as_str()),
            "high confidence should allow cheaper model, got: {}",
            high_confidence.primary.slug
        );
    }

    #[test]
    fn cache_affinity_bonus() {
        let cascade = CascadeRouter::new(vec![
            "claude-sonnet-4-5".to_string(),
            "claude-sonnet-4-6".to_string(),
        ]);
        let mut ctx = default_ctx();

        for _ in 0..80 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.8, true);
        }
        for _ in 0..10 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.2, false);
        }
        for _ in 0..82 {
            cascade.record_observation(&ctx, "claude-sonnet-4-6", 0.8, true);
        }
        for _ in 0..8 {
            cascade.record_observation(&ctx, "claude-sonnet-4-6", 0.2, false);
        }

        assert_eq!(cascade.current_stage(), CascadeStage::Confidence);

        let no_affinity = cascade.route(&ctx);
        assert_eq!(no_affinity.primary.slug, "claude-sonnet-4-6");

        ctx.previous_model = Some("claude-sonnet-4-5".to_string());
        let with_affinity = cascade.route(&ctx);
        assert_eq!(with_affinity.primary.slug, "claude-sonnet-4-5");
    }

    // ── Test 7c: health-aware routing skips unhealthy providers ─────────

    #[test]
    fn cascade_health_aware_excludes_unhealthy_provider_models() {
        let cascade = CascadeRouter::new(vec![
            "claude-sonnet-4-5".to_string(),
            "claude-opus-4".to_string(),
        ]);
        let ctx = default_ctx();

        // Push the router into UCB so the candidate-aware LinUCB path is exercised.
        for _ in 0..200 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 1.0, true);
        }
        assert_eq!(cascade.current_stage(), CascadeStage::Ucb);
        assert_eq!(cascade.route(&ctx).primary.slug, "claude-sonnet-4-5");

        let health = ProviderHealthRegistry::new();
        for _ in 0..3 {
            health.record_failure("anthropic", ErrorClass::ServerError);
        }

        let mut model_providers = HashMap::new();
        model_providers.insert("claude-sonnet-4-5".to_string(), "anthropic".to_string());
        model_providers.insert("claude-opus-4".to_string(), "openai".to_string());

        let routed = cascade.route_with_health(&ctx, &health, &model_providers);
        assert_eq!(
            routed.primary.slug, "claude-opus-4",
            "unhealthy providers should be excluded from cascade selection"
        );
    }

    // ── Test 8: stage labels are correct ────────────────────────────────

    #[test]
    fn stage_labels() {
        assert_eq!(CascadeStage::Static.label(), "static");
        assert_eq!(CascadeStage::Confidence.label(), "confidence");
        assert_eq!(CascadeStage::Ucb.label(), "ucb");
    }

    // ── Test 9: frequency routing follows the frequency policy ─────────

    #[test]
    fn frequency_routing_uses_expected_policy() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();

        assert_eq!(
            cascade.select_for_frequency(OperatingFrequency::Gamma, Some(&ctx), None, None),
            None
        );

        let theta = cascade
            .select_for_frequency(OperatingFrequency::Theta, Some(&ctx), None, None)
            .expect("theta should route");
        assert_eq!(theta.slug, "claude-sonnet-4-5");

        let delta = cascade
            .select_for_frequency(OperatingFrequency::Delta, Some(&ctx), None, None)
            .expect("delta should route");
        assert_eq!(delta.slug, "claude-opus-4");
    }

    #[test]
    fn high_cfactor_prefers_cheapest_model() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();
        let cfactor = CFactor {
            overall: 0.9,
            ..CFactor::default()
        };

        let selected = cascade
            .select_for_frequency(
                OperatingFrequency::Theta,
                Some(&ctx),
                Some(&cfactor),
                Some("Implementer"),
            )
            .expect("theta should route");

        assert_eq!(selected.slug, "claude-haiku-3-5");
    }

    #[test]
    fn low_cfactor_prefers_strongest_model() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();
        let cfactor = CFactor {
            overall: 0.2,
            ..CFactor::default()
        };

        let selected = cascade
            .select_for_frequency(
                OperatingFrequency::Theta,
                Some(&ctx),
                Some(&cfactor),
                Some("Implementer"),
            )
            .expect("theta should route");

        assert_eq!(selected.slug, "claude-opus-4");
    }

    #[test]
    fn strongest_model_falls_back_to_best_available_slug() {
        let cascade = CascadeRouter::new(vec![
            "claude-haiku-3-5".to_string(),
            "claude-sonnet-4-5".to_string(),
        ]);

        assert_eq!(cascade.strongest_model().slug, "claude-sonnet-4-5");
    }

    // ── Test 11: observation count is consistent ────────────────────────

    #[test]
    fn observation_count_tracks_correctly() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();

        assert_eq!(cascade.total_observations(), 0);

        cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.8, true);
        cascade.record_observation(&ctx, "claude-haiku-3-5", 0.3, false);
        cascade.record_observation(&ctx, "claude-opus-4", 0.9, true);

        assert_eq!(cascade.total_observations(), 3);
    }

    // ── Test 12: confidence snapshot tracks trials ──────────────────────

    #[test]
    fn confidence_snapshot_accurate() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();

        cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.8, true);
        cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.5, false);
        cascade.record_observation(&ctx, "claude-haiku-3-5", 0.9, true);

        let snap = cascade.confidence_snapshot();
        assert_eq!(snap.get("claude-sonnet-4-5"), Some(&(2, 1)));
        assert_eq!(snap.get("claude-haiku-3-5"), Some(&(1, 1)));
    }

    // ── Test 11: latency SLA varies by tier ─────────────────────────────

    #[test]
    fn latency_sla_by_tier() {
        let cascade = CascadeRouter::new(test_slugs());

        let mut ctx = default_ctx();
        ctx.role = AgentRole::Conductor; // Fast
        let fast = cascade.route(&ctx);

        ctx.role = AgentRole::Implementer; // Standard
        let standard = cascade.route(&ctx);

        ctx.role = AgentRole::Architect; // Premium
        let premium = cascade.route(&ctx);

        assert!(fast.latency_sla_ms < standard.latency_sla_ms);
        assert!(standard.latency_sla_ms < premium.latency_sla_ms);
    }

    // ── Test 12: stage_for_observations boundaries ──────────────────────

    #[test]
    fn stage_boundaries() {
        assert_eq!(stage_for_observations(0), CascadeStage::Static);
        assert_eq!(stage_for_observations(49), CascadeStage::Static);
        assert_eq!(stage_for_observations(50), CascadeStage::Confidence);
        assert_eq!(stage_for_observations(199), CascadeStage::Confidence);
        assert_eq!(stage_for_observations(200), CascadeStage::Ucb);
        assert_eq!(stage_for_observations(1000), CascadeStage::Ucb);
    }

    // ── Test 13: model_stats pass_rate computation ──────────────────────

    #[test]
    fn model_stats_pass_rate() {
        let mut s = ModelStats::default();
        assert!((s.pass_rate() - 0.0).abs() < f64::EPSILON);

        s.trials = 10;
        s.successes = 7;
        assert!((s.pass_rate() - 0.7).abs() < f64::EPSILON);
    }

    // ── Test 14: confidence width shrinks with more data ────────────────

    #[test]
    fn confidence_width_shrinks() {
        let s10 = ModelStats {
            trials: 10,
            successes: 7,
            ..ModelStats::default()
        };
        let s100 = ModelStats {
            trials: 100,
            successes: 70,
            ..ModelStats::default()
        };
        let s1000 = ModelStats {
            trials: 1000,
            successes: 700,
            ..ModelStats::default()
        };

        assert!(s10.confidence_width() > s100.confidence_width());
        assert!(s100.confidence_width() > s1000.confidence_width());
    }

    // ── Test 15: premium role uses opus in static stage ─────────────────

    #[test]
    fn premium_role_gets_opus() {
        let cascade = CascadeRouter::new(test_slugs());
        let mut ctx = default_ctx();
        ctx.role = AgentRole::Architect; // Premium tier

        let result = cascade.route(&ctx);
        assert_eq!(result.primary.slug, "claude-opus-4");
        // Premium fallback is sonnet
        assert_eq!(result.fallback.as_ref().unwrap().slug, "claude-sonnet-4-5");
    }

    // ── Test 16: display impl for CascadeStage ──────────────────────────

    #[test]
    fn cascade_stage_display() {
        assert_eq!(format!("{}", CascadeStage::Static), "static");
        assert_eq!(format!("{}", CascadeStage::Ucb), "ucb");
    }

    // ── Test 17: custom role table ──────────────────────────────────────

    #[test]
    fn custom_role_table() {
        let mut table = HashMap::new();
        table.insert(AgentRole::Implementer, "gpt-5".to_string());

        let cascade = CascadeRouter::new(test_slugs()).with_role_table(table);
        let ctx = default_ctx();
        let result = cascade.route(&ctx);

        assert_eq!(result.primary.slug, "gpt-5");
    }

    #[test]
    fn cascade_router_kimi_selects_kimi_in_static_stage() {
        let cascade = CascadeRouter::new(vec!["kimi-k2.5".to_string()]);
        let ctx = default_ctx();

        let result = cascade.route(&ctx);
        assert_eq!(result.stage, CascadeStage::Static);
        assert_eq!(result.primary.slug, "kimi-k2.5");
    }

    #[test]
    fn cascade_gemini_routes_configured_fast_standard_and_premium_models() {
        let cascade = CascadeRouter::new(vec![
            "gemini-2.5-flash-lite".to_string(),
            "gemini-2.5-flash".to_string(),
            "gemini-2.5-pro".to_string(),
            "gemini-3.1-pro-preview".to_string(),
        ]);
        let mut ctx = default_ctx();

        ctx.role = AgentRole::Conductor;
        let fast = cascade.route(&ctx);
        assert_eq!(fast.primary.slug, "gemini-2.5-flash-lite");
        assert!(fast.fallback.is_none());

        ctx.role = AgentRole::Implementer;
        let standard = cascade.route(&ctx);
        assert_eq!(standard.primary.slug, "gemini-2.5-flash");
        assert_eq!(
            standard.fallback.as_ref().expect("standard fallback").slug,
            "claude-haiku-3-5"
        );

        ctx.role = AgentRole::Architect;
        let premium = cascade.route(&ctx);
        assert_eq!(premium.primary.slug, "gemini-3.1-pro-preview");
        assert_eq!(
            premium.fallback.as_ref().expect("premium fallback").slug,
            "claude-sonnet-4-5"
        );
    }

    #[test]
    fn cascade_gemini_prefers_opus_for_premium_when_available() {
        let cascade = CascadeRouter::new(vec![
            "gemini-2.5-flash-lite".to_string(),
            "gemini-2.5-flash".to_string(),
            "gemini-2.5-pro".to_string(),
            "gemini-3.1-pro-preview".to_string(),
            "claude-opus-4".to_string(),
        ]);
        let mut ctx = default_ctx();
        ctx.role = AgentRole::Architect;

        let result = cascade.route(&ctx);
        assert_eq!(result.primary.slug, "claude-opus-4");
    }

    #[test]
    fn cascade_gemini_matches_openrouter_slug_families() {
        let cascade = CascadeRouter::new(vec![
            "google/gemini-2.5-flash-lite".to_string(),
            "google/gemini-2.5-flash".to_string(),
            "google/gemini-3.1-pro-preview".to_string(),
        ]);
        let mut ctx = default_ctx();

        ctx.role = AgentRole::Conductor;
        assert_eq!(
            cascade.route(&ctx).primary.slug,
            "google/gemini-2.5-flash-lite"
        );

        ctx.role = AgentRole::Implementer;
        assert_eq!(cascade.route(&ctx).primary.slug, "google/gemini-2.5-flash");

        ctx.role = AgentRole::Architect;
        assert_eq!(
            cascade.route(&ctx).primary.slug,
            "google/gemini-3.1-pro-preview"
        );
    }

    #[test]
    fn routing_context_thinking_high_prefers_thinking_models() {
        let cascade = CascadeRouter::new(vec![
            "gemini-2.5-flash-lite".to_string(),
            "gemini-2.5-flash".to_string(),
            "gemini-2.5-pro".to_string(),
        ]);
        let mut ctx = default_ctx();
        ctx.complexity = TaskComplexityBand::Complex;
        ctx.thinking_level = Some("high".to_string());

        let result = cascade.route(&ctx);
        assert_ne!(result.primary.slug, "gemini-2.5-flash-lite");
        assert!(model_supports_thinking(&result.primary.slug));
    }

    #[test]
    fn routing_context_thinking_minimal_prefers_non_thinking_models() {
        let cascade = CascadeRouter::new(vec![
            "gemini-2.5-flash-lite".to_string(),
            "gemini-2.5-flash".to_string(),
            "gemini-2.5-pro".to_string(),
        ]);
        let mut ctx = default_ctx();
        ctx.thinking_level = Some("minimal".to_string());

        let result = cascade.route(&ctx);
        assert_eq!(result.primary.slug, "gemini-2.5-flash-lite");
    }

    #[tokio::test]
    async fn shadow_evaluate_records_observation_for_passing_free_model() {
        let primary = agent_result(
            "```rust\nfn answer() -> u32 { 42 }\n```",
            true,
            "gemini-2.5-pro",
            900,
        );
        let shadow = agent_result(
            "```rust\nfn answer() -> u32 { 42 }\n```",
            true,
            "gemini-2.5-flash-lite",
            120,
        );
        let mut cascade = CascadeRouter::new(vec![
            "gemini-2.5-pro".to_string(),
            "gemini-2.5-flash-lite".to_string(),
        ])
        .with_free_tier_shadow_runner(Arc::new(StubShadowRunner { result: shadow }));

        cascade
            .shadow_evaluate(
                "Implement a Rust function that returns 42 and include code.",
                &primary,
                "gemini-2.5-flash-lite",
            )
            .await;

        let stats = cascade.observation_snapshot();
        let flash_lite = stats
            .get("gemini-2.5-flash-lite")
            .expect("flash-lite stats");

        assert_eq!(flash_lite.trials, 1);
        assert_eq!(flash_lite.successes, 1);
        assert_eq!(cascade.total_observations(), 1);
    }

    #[tokio::test]
    async fn shadow_evaluate_records_failed_observation_when_shadow_output_is_weaker() {
        let primary = agent_result(
            "```rust\nfn answer() -> u32 { 42 }\n```\nAdd a unit test.",
            true,
            "gemini-2.5-pro",
            900,
        );
        let weak_shadow = agent_result("done", true, "gemini-2.5-flash-lite", 120);
        let mut cascade = CascadeRouter::new(vec![
            "gemini-2.5-pro".to_string(),
            "gemini-2.5-flash-lite".to_string(),
        ])
        .with_free_tier_shadow_runner(Arc::new(StubShadowRunner {
            result: weak_shadow,
        }));

        cascade
            .shadow_evaluate(
                "Implement a Rust function and add tests for it.",
                &primary,
                "gemini-2.5-flash-lite",
            )
            .await;

        let stats = cascade.observation_snapshot();
        let flash_lite = stats
            .get("gemini-2.5-flash-lite")
            .expect("flash-lite stats");

        assert_eq!(flash_lite.trials, 1);
        assert_eq!(flash_lite.successes, 0);
    }

    #[tokio::test]
    async fn shadow_evaluate_shifts_router_toward_free_model() {
        let prompt = "Implement a Rust function that parses a config string into a struct.";
        let primary = agent_result(
            "```rust\nstruct Config { enabled: bool }\nfn parse_config(input: &str) -> Config { Config { enabled: input == \"on\" } }\n```",
            true,
            "gemini-2.5-pro",
            900,
        );
        let shadow = agent_result(
            "```rust\nstruct Config { enabled: bool }\nfn parse_config(input: &str) -> Config { Config { enabled: input.trim() == \"on\" } }\n```",
            true,
            "gemini-2.5-flash-lite",
            110,
        );
        let ctx = infer_shadow_routing_context(prompt, &primary);
        let mut route_ctx = ctx.clone();
        route_ctx.previous_model = None;
        let mut cascade = CascadeRouter::new(vec![
            "gemini-2.5-pro".to_string(),
            "gemini-2.5-flash-lite".to_string(),
        ])
        .with_free_tier_shadow_runner(Arc::new(StubShadowRunner { result: shadow }));

        for _ in 0..34 {
            cascade.record_observation(&ctx, "gemini-2.5-pro", 0.9, true);
        }
        for _ in 0..6 {
            cascade.record_observation(&ctx, "gemini-2.5-pro", 0.0, false);
        }
        for _ in 0..5 {
            cascade.record_observation(&ctx, "gemini-2.5-flash-lite", 0.8, true);
        }
        for _ in 0..5 {
            cascade.record_observation(&ctx, "gemini-2.5-flash-lite", 0.0, false);
        }

        assert_eq!(cascade.current_stage(), CascadeStage::Confidence);
        assert_eq!(cascade.route(&route_ctx).primary.slug, "gemini-2.5-pro");

        for _ in 0..40 {
            cascade
                .shadow_evaluate(prompt, &primary, "gemini-2.5-flash-lite")
                .await;
        }

        let stats = cascade.observation_snapshot();
        let flash_lite = stats
            .get("gemini-2.5-flash-lite")
            .expect("flash-lite stats");
        assert_eq!(flash_lite.trials, 50);
        assert_eq!(flash_lite.successes, 45);
        assert_eq!(
            cascade.route(&route_ctx).primary.slug,
            "gemini-2.5-flash-lite"
        );
    }

    #[test]
    fn gemini_observations_include_quality_and_cost_signals() {
        let cascade = CascadeRouter::new(vec![
            "gemini-2.5-pro".to_string(),
            "claude-sonnet-4-5".to_string(),
        ]);
        let ctx = default_ctx();

        assert!(cascade.record_gemini_observation(
            &ctx,
            "gemini-2.5-pro",
            0.92,
            true,
            GeminiObservation {
                input_tokens: 250_000,
                output_tokens: 1_024,
                thinking_tokens: Some(64),
                cached_tokens: Some(512),
                grounding_query_count: 3,
                code_execution_success_count: 2,
                code_execution_failure_count: 1,
                context_tier: GeminiContextTier::Over200k,
            },
        ));

        let stats = cascade.observation_snapshot();
        let gemini = stats.get("gemini-2.5-pro").expect("gemini stats");

        assert_eq!(gemini.trials, 1);
        assert_eq!(gemini.successes, 1);
        assert_eq!(gemini.gemini_requests, 1);
        assert_eq!(gemini.total_gemini_thinking_tokens, 64);
        assert!((gemini.avg_gemini_thinking_tokens_per_response - 64.0).abs() < 1e-9);
        assert_eq!(gemini.total_gemini_cached_tokens, 512);
        assert!((gemini.avg_gemini_cached_tokens_per_response - 512.0).abs() < 1e-9);
        assert_eq!(gemini.total_gemini_grounding_queries, 3);
        assert!((gemini.avg_gemini_grounding_queries_per_response - 3.0).abs() < 1e-9);
        assert_eq!(gemini.gemini_code_execution_successes, 2);
        assert_eq!(gemini.gemini_code_execution_failures, 1);
        assert!((gemini.gemini_code_execution_success_rate - (2.0 / 3.0)).abs() < 1e-9);
        assert_eq!(gemini.gemini_context_window_le_200k_requests, 0);
        assert_eq!(gemini.gemini_context_window_gt_200k_requests, 1);
    }

    #[test]
    fn gemini_observations_from_metadata_extract_router_signals() {
        let metadata = GeminiMetadata {
            grounding_metadata: Some(GroundingMetadata {
                web_search_queries: Some(vec![
                    "Rust cargo metadata".to_string(),
                    "Rust cargo workspace".to_string(),
                ]),
                grounding_chunks: None,
                grounding_supports: None,
                search_entry_point: None,
            }),
            code_execution_results: vec![
                CodeExecutionResultPart {
                    outcome: "OUTCOME_OK".to_string(),
                    output: "passed".to_string(),
                },
                CodeExecutionResultPart {
                    outcome: "OUTCOME_ERROR".to_string(),
                    output: "failed".to_string(),
                },
            ],
            thinking_tokens: Some(11),
            cached_tokens: Some(80),
            safety_ratings: Vec::new(),
        };

        let observation = GeminiObservation::from_metadata(&metadata, 240_000, 512);

        assert_eq!(observation.thinking_tokens, Some(11));
        assert_eq!(observation.cached_tokens, Some(80));
        assert_eq!(observation.grounding_query_count, 2);
        assert_eq!(observation.code_execution_success_count, 1);
        assert_eq!(observation.code_execution_failure_count, 1);
        assert_eq!(observation.context_tier, GeminiContextTier::Over200k);
    }

    #[test]
    fn gemini_observations_persist_across_save_and_load() {
        let cascade = CascadeRouter::new(vec![
            "gemini-2.5-flash".to_string(),
            "claude-sonnet-4-5".to_string(),
        ]);
        let ctx = default_ctx();

        assert!(cascade.record_gemini_observation(
            &ctx,
            "gemini-2.5-flash",
            0.8,
            true,
            GeminiObservation {
                input_tokens: 120_000,
                output_tokens: 600,
                thinking_tokens: Some(21),
                cached_tokens: Some(144),
                grounding_query_count: 1,
                code_execution_success_count: 1,
                code_execution_failure_count: 0,
                context_tier: GeminiContextTier::UpTo200k,
            },
        ));
        assert!(cascade.record_gemini_observation(
            &ctx,
            "gemini-2.5-flash",
            0.0,
            false,
            GeminiObservation {
                input_tokens: 260_000,
                output_tokens: 700,
                thinking_tokens: Some(34),
                cached_tokens: Some(32),
                grounding_query_count: 4,
                code_execution_success_count: 0,
                code_execution_failure_count: 2,
                context_tier: GeminiContextTier::Over200k,
            },
        ));

        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("cascade-router.json");
        cascade.save(&path).expect("save cascade router");

        let reloaded = CascadeRouter::load_or_new(
            &path,
            vec![
                "gemini-2.5-flash".to_string(),
                "claude-sonnet-4-5".to_string(),
            ],
        );
        let stats = reloaded.observation_snapshot();
        let gemini = stats.get("gemini-2.5-flash").expect("gemini stats");

        assert_eq!(gemini.gemini_requests, 2);
        assert_eq!(gemini.total_gemini_thinking_tokens, 55);
        assert_eq!(gemini.total_gemini_cached_tokens, 176);
        assert_eq!(gemini.total_gemini_grounding_queries, 5);
        assert_eq!(gemini.gemini_code_execution_successes, 1);
        assert_eq!(gemini.gemini_code_execution_failures, 2);
        assert!((gemini.gemini_code_execution_success_rate - (1.0 / 3.0)).abs() < 1e-9);
        assert_eq!(gemini.gemini_context_window_le_200k_requests, 1);
        assert_eq!(gemini.gemini_context_window_gt_200k_requests, 1);
    }

    // ── Test 18: UCB stage uses linucb selection ────────────────────────

    #[test]
    fn ucb_stage_uses_trained_linucb() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();

        // Train haiku as the best arm with many observations.
        for _ in 0..250 {
            cascade.record_observation(&ctx, "claude-haiku-3-5", 1.0, true);
            // Give some data to other arms too so LinUCB has seen them.
            if cascade.total_observations() % 10 == 0 {
                cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.1, false);
                cascade.record_observation(&ctx, "claude-opus-4", 0.1, false);
            }
        }

        assert_eq!(cascade.current_stage(), CascadeStage::Ucb);
        let result = cascade.route(&ctx);
        // LinUCB should prefer the highly-rewarded arm
        assert_eq!(result.primary.slug, "claude-haiku-3-5");
    }

    #[test]
    fn record_outcome_updates_model_statistics() {
        let cascade = CascadeRouter::new(test_slugs());

        assert!(cascade.record_outcome("claude-sonnet-4-5", true));
        assert_eq!(cascade.total_observations(), 1);

        let stats = cascade.confidence_snapshot();
        assert_eq!(stats.get("claude-sonnet-4-5"), Some(&(1, 1)));
    }

    #[test]
    fn perplexity_observations_include_citations_latency_and_total_cost() {
        let cascade = CascadeRouter::new(vec![
            "sonar-pro".to_string(),
            "claude-sonnet-4-5".to_string(),
        ]);
        let mut ctx = default_ctx();
        ctx.role = AgentRole::Researcher;
        ctx.task_category = TaskCategory::Research;

        assert!(cascade.record_perplexity_observation(
            &ctx,
            "sonar-pro",
            0.95,
            true,
            PerplexityObservation {
                citation_count: 6,
                search_latency_ms: 1_200,
                input_tokens: 1_000,
                output_tokens: 500,
            },
        ));

        let stats = cascade.observation_snapshot();
        let sonar = stats.get("sonar-pro").expect("sonar-pro stats");

        assert_eq!(sonar.trials, 1);
        assert_eq!(sonar.successes, 1);
        assert_eq!(sonar.total_citations, 6);
        assert!((sonar.avg_citations_per_response - 6.0).abs() < 1e-9);
        assert_eq!(sonar.total_search_latency_ms, 1_200);
        assert!((sonar.avg_search_latency_ms - 1_200.0).abs() < 1e-9);
        assert_eq!(sonar.perplexity_requests, 1);
        assert!((sonar.total_cost_usd - 0.0245).abs() < 1e-9);
        assert!((sonar.avg_cost_usd - 0.0245).abs() < 1e-9);
    }

    #[test]
    fn perplexity_observations_persist_across_save_and_load() {
        let cascade =
            CascadeRouter::new(vec!["sonar".to_string(), "claude-sonnet-4-5".to_string()]);
        let mut ctx = default_ctx();
        ctx.role = AgentRole::Researcher;
        ctx.task_category = TaskCategory::Research;

        assert!(cascade.record_perplexity_observation(
            &ctx,
            "sonar",
            0.9,
            true,
            PerplexityObservation {
                citation_count: 3,
                search_latency_ms: 900,
                input_tokens: 2_000,
                output_tokens: 1_000,
            },
        ));

        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("cascade-router.json");
        cascade.save(&path).expect("save cascade router");

        let reloaded = CascadeRouter::load_or_new(
            &path,
            vec!["sonar".to_string(), "claude-sonnet-4-5".to_string()],
        );
        let stats = reloaded.observation_snapshot();
        let sonar = stats.get("sonar").expect("sonar stats");

        assert_eq!(sonar.total_citations, 3);
        assert_eq!(sonar.total_search_latency_ms, 900);
        assert_eq!(sonar.perplexity_requests, 1);
        assert!((sonar.total_cost_usd - 0.008).abs() < 1e-9);
    }

    // ── cascade_perplexity: Researcher routes to sonar-pro ───────────────

    #[test]
    fn cascade_perplexity_researcher_routes_to_sonar_pro() {
        let slugs = vec![
            "sonar-pro".to_string(),
            "sonar".to_string(),
            "claude-haiku-3-5".to_string(),
            "claude-sonnet-4-5".to_string(),
        ];
        let cascade = CascadeRouter::new(slugs);
        let mut ctx = default_ctx();
        ctx.role = AgentRole::Researcher;
        ctx.task_category = TaskCategory::Research;

        let result = cascade.route(&ctx);
        assert_eq!(result.stage, CascadeStage::Static);
        assert_eq!(result.primary.slug, "sonar-pro");
    }

    #[test]
    fn cascade_perplexity_research_category_biases_any_role() {
        let slugs = vec![
            "sonar-pro".to_string(),
            "claude-haiku-3-5".to_string(),
            "claude-sonnet-4-5".to_string(),
        ];
        let cascade = CascadeRouter::new(slugs);
        let mut ctx = default_ctx();
        ctx.role = AgentRole::Implementer;
        ctx.task_category = TaskCategory::Research;

        let result = cascade.route(&ctx);
        assert_eq!(result.primary.slug, "sonar-pro");
    }

    #[test]
    fn cascade_perplexity_falls_back_to_standard_when_no_sonar() {
        let cascade = CascadeRouter::new(test_slugs()); // no sonar in test_slugs
        let mut ctx = default_ctx();
        ctx.role = AgentRole::Researcher;
        ctx.task_category = TaskCategory::Research;

        let result = cascade.route(&ctx);
        // No sonar available → standard tier fallback
        assert_ne!(result.primary.slug, "sonar-pro");
        assert_ne!(result.primary.slug, "sonar");
    }

    #[test]
    fn pareto_pruning_reduces_alpha_for_dominated_models() {
        let frontier = vec!["claude-sonnet-4-5".to_string()];
        let base_alpha = 0.8;

        assert!(
            (pareto_adjusted_alpha(base_alpha, "claude-sonnet-4-5", &frontier) - base_alpha).abs()
                < f64::EPSILON
        );
        assert!(
            (pareto_adjusted_alpha(base_alpha, "claude-haiku-3-5", &frontier) - base_alpha * 0.1)
                .abs()
                < f64::EPSILON
        );
    }

    #[test]
    fn pareto_frontier_refreshes_every_50_observations() {
        let cascade = CascadeRouter::new(vec![
            "claude-haiku-3-5".to_string(),
            "claude-sonnet-4-5".to_string(),
        ]);
        let ctx = default_ctx();

        for _ in 0..50 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 1.0, true);
        }

        assert_eq!(cascade.pareto_frontier.lock().bucket, 1);
        let frontier = cascade.pareto_frontier.lock().frontier.clone();
        assert!(frontier.contains(&"claude-haiku-3-5".to_string()));
        assert!(frontier.contains(&"claude-sonnet-4-5".to_string()));

        for _ in 0..50 {
            cascade.record_observation(&ctx, "claude-haiku-3-5", 0.0, false);
        }

        assert_eq!(cascade.pareto_frontier.lock().bucket, 2);
        let frontier = cascade.pareto_frontier.lock().frontier.clone();
        assert!(frontier.contains(&"claude-sonnet-4-5".to_string()));
        assert!(
            !frontier.contains(&"claude-haiku-3-5".to_string()),
            "dominated models should be pruned from the frontier after refresh"
        );
    }
}
