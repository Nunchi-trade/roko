//! Structs, enums, and trait definitions used by the cascade router.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use roko_agent::provider::ProviderError;
use roko_agent::AgentResult;
use roko_core::agent::ModelSpec;
use serde::{Deserialize, Serialize};

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

/// Recorded transition between cascade maturity stages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageTransition {
    /// Previous active stage.
    pub from: CascadeStage,
    /// Newly activated stage.
    pub to: CascadeStage,
    /// Observation count when the transition occurred.
    pub observations: u64,
    /// Timestamp when the transition was recorded.
    pub timestamp: DateTime<Utc>,
}

// ─── CascadeModel ───────────────────────────────────────────────────────────

/// Routing recommendation from the cascade.
#[derive(Debug, Clone)]
pub struct CascadeModel {
    /// Primary model to use.
    pub primary: ModelSpec,
    /// Ordered fallback models to try after general failures.
    pub fallback_chain: Vec<ModelSpec>,
    /// Larger-context model to try when the primary overflows context.
    pub context_overflow_fallback: Option<ModelSpec>,
    /// Latency SLA in milliseconds.
    pub latency_sla_ms: u64,
    /// Which cascade stage produced this recommendation.
    pub stage: CascadeStage,
}

impl CascadeModel {
    /// Return the model to use for the given attempt number.
    ///
    /// Attempt 0 is the primary model. Subsequent attempts walk the fallback
    /// chain in order until it is exhausted.
    #[must_use]
    pub fn model_for_attempt(&self, attempt: usize) -> Option<&ModelSpec> {
        match attempt {
            0 => Some(&self.primary),
            _ => self.fallback_chain.get(attempt - 1),
        }
    }

    /// Return the best fallback to use for a provider-specific failure.
    #[must_use]
    pub fn fallback_for_error(&self, error: &ProviderError) -> Option<&ModelSpec> {
        match error {
            ProviderError::ContextOverflow => self.context_overflow_fallback.as_ref(),
            ProviderError::RateLimit { .. } => self
                .fallback_chain
                .iter()
                .find(|model| model.backend != self.primary.backend)
                .or_else(|| self.fallback_chain.first()),
            _ => self.fallback_chain.first(),
        }
    }
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

/// Debug score for one model candidate within the cascade.
#[derive(Debug, Clone, PartialEq)]
pub struct CascadeCandidateScore {
    /// Model slug this score belongs to.
    pub slug: String,
    /// Stage-specific score used for comparison.
    pub score: f64,
    /// Whether this candidate was selected.
    pub selected: bool,
    /// Whether this candidate is on the current Pareto frontier.
    pub on_pareto_frontier: bool,
    /// LinUCB mean-reward estimate, when UCB routing is active.
    pub exploitation: Option<f64>,
    /// LinUCB exploration bonus, when UCB routing is active.
    pub exploration: Option<f64>,
}

/// Explainability snapshot for one routing decision.
#[derive(Debug, Clone, PartialEq)]
pub struct CascadeRouteExplanation {
    /// Which cascade stage handled this routing decision.
    pub stage: CascadeStage,
    /// Total observations recorded when the explanation was generated.
    pub observations: u64,
    /// Current LinUCB alpha, when the UCB stage is active.
    pub alpha: Option<f64>,
    /// Selected model slug.
    pub selected_slug: String,
    /// Candidate scores in descending order.
    pub candidates: Vec<CascadeCandidateScore>,
    /// Current Pareto frontier snapshot used by the cascade.
    pub pareto_frontier: Vec<String>,
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

/// Bias signal emitted by the conductor and applied at routing time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingBias {
    /// Model slugs to deprioritize.
    pub deprioritize: Vec<String>,
    /// Prefer cheaper tiers when live load or budget pressure is high.
    pub prefer_cheaper: bool,
    /// Human-readable explanation for debugging and logging.
    pub reason: String,
}

// ─── Confidence-stage stats ─────────────────────────────────────────────────

/// Threshold for transitioning from Confidence to UCB stage.
pub(crate) const CONFIDENCE_TO_UCB_THRESHOLD: u64 = 200;
/// Affect confidence below which the router biases toward stronger models.
pub(crate) const LOW_AFFECT_CONFIDENCE_THRESHOLD: f64 = 0.3;
/// C-Factor above which the router biases toward cheaper models.
pub(crate) const HIGH_CFACTOR_THRESHOLD: f64 = 0.8;
/// C-Factor below which the router biases toward stronger models.
pub(crate) const LOW_CFACTOR_THRESHOLD: f64 = 0.4;
/// Cold-start bonus for reusing the previous model.
pub(crate) const CACHE_AFFINITY_BONUS: f64 = 0.15;
/// Minimum score improvement required before switching away from the incumbent.
pub(crate) const HYSTERESIS_THRESHOLD: f64 = 0.10;
/// Recompute the Pareto frontier after every 50 observations.
pub(crate) const PARETO_RECOMPUTE_INTERVAL: u64 = 50;

/// Per-model observation record for the confidence stage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ModelStats {
    /// Number of trials (selections) for this model.
    pub(crate) trials: u64,
    /// Number of successes (gate passes).
    pub(crate) successes: u64,
    /// Total citations observed across Perplexity responses.
    pub(crate) total_citations: u64,
    /// Total Perplexity search latency observed in milliseconds.
    pub(crate) total_search_latency_ms: u64,
    /// Total observed cost in USD (token cost + per-request fee).
    pub(crate) total_cost_usd: f64,
    /// Number of Perplexity requests contributing metadata.
    pub(crate) perplexity_requests: u64,
    /// Total Gemini thinking tokens observed across responses.
    pub(crate) total_gemini_thinking_tokens: u64,
    /// Total Gemini cached tokens observed across responses.
    pub(crate) total_gemini_cached_tokens: u64,
    /// Total Gemini grounding queries executed across responses.
    pub(crate) total_gemini_grounding_queries: u64,
    /// Number of successful Gemini code-execution outcomes.
    pub(crate) gemini_code_execution_successes: u64,
    /// Number of failed Gemini code-execution outcomes.
    pub(crate) gemini_code_execution_failures: u64,
    /// Number of Gemini responses routed in the ≤200K context pricing tier.
    pub(crate) gemini_context_window_le_200k_requests: u64,
    /// Number of Gemini responses routed in the >200K context pricing tier.
    pub(crate) gemini_context_window_gt_200k_requests: u64,
    /// Number of Gemini requests contributing observation metadata.
    pub(crate) gemini_requests: u64,
}

impl ModelStats {
    /// Empirical pass rate.
    pub(crate) fn pass_rate(&self) -> f64 {
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
    pub(crate) fn confidence_width(&self) -> f64 {
        if self.trials == 0 {
            return f64::INFINITY;
        }
        let p = self.pass_rate();
        let n = self.trials as f64;
        1.96 * (p * (1.0 - p) / n).sqrt()
    }

    /// Upper confidence bound on the pass rate.
    pub(crate) fn upper_bound(&self) -> f64 {
        (self.pass_rate() + self.confidence_width()).min(1.0)
    }

    pub(crate) fn avg_citations_per_response(&self) -> f64 {
        if self.perplexity_requests == 0 {
            0.0
        } else {
            self.total_citations as f64 / self.perplexity_requests as f64
        }
    }

    pub(crate) fn avg_search_latency_ms(&self) -> f64 {
        if self.perplexity_requests == 0 {
            0.0
        } else {
            self.total_search_latency_ms as f64 / self.perplexity_requests as f64
        }
    }

    pub(crate) fn avg_cost_usd(&self) -> f64 {
        if self.perplexity_requests == 0 {
            0.0
        } else {
            self.total_cost_usd / self.perplexity_requests as f64
        }
    }

    pub(crate) fn cost_per_success(&self) -> Option<f64> {
        if self.successes == 0 {
            None
        } else {
            Some(self.total_cost_usd / self.successes as f64)
        }
    }

    pub(crate) fn avg_gemini_thinking_tokens_per_response(&self) -> f64 {
        if self.gemini_requests == 0 {
            0.0
        } else {
            self.total_gemini_thinking_tokens as f64 / self.gemini_requests as f64
        }
    }

    pub(crate) fn avg_gemini_cached_tokens_per_response(&self) -> f64 {
        if self.gemini_requests == 0 {
            0.0
        } else {
            self.total_gemini_cached_tokens as f64 / self.gemini_requests as f64
        }
    }

    pub(crate) fn avg_gemini_grounding_queries_per_response(&self) -> f64 {
        if self.gemini_requests == 0 {
            0.0
        } else {
            self.total_gemini_grounding_queries as f64 / self.gemini_requests as f64
        }
    }

    pub(crate) fn gemini_code_execution_success_rate(&self) -> f64 {
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
    pub fn from_metadata(
        metadata: &roko_agent::gemini::GeminiMetadata,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Self {
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
pub(crate) struct PerplexityObservationTotals {
    pub(crate) citation_count: u64,
    pub(crate) search_latency_ms: u64,
    pub(crate) total_cost_usd: f64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GeminiObservationTotals {
    pub(crate) thinking_tokens: u64,
    pub(crate) cached_tokens: u64,
    pub(crate) grounding_query_count: u64,
    pub(crate) code_execution_success_count: u64,
    pub(crate) code_execution_failure_count: u64,
    pub(crate) context_tier: GeminiContextTier,
}

/// Cached Pareto frontier state.
#[derive(Debug, Clone, Default)]
pub(crate) struct ParetoFrontierState {
    pub(crate) frontier: Vec<String>,
    pub(crate) bucket: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct StageTracking {
    pub(crate) current: CascadeStage,
    pub(crate) transitions: Vec<StageTransition>,
}
