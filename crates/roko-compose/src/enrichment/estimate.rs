//! Cost estimation for enrichment pipeline runs.
//!
//! Produces estimates of token consumption, USD cost, and wall-clock duration
//! for a set of enrichment steps. Estimates are model-aware: different models
//! have different per-token pricing.
//!
//! These are rough estimates for planning and budgeting. Actual costs depend
//! on prompt content, model behavior, and retry counts.

use roko_core::TaskComplexityBand;

use super::step::EnrichStep;

// ── Per-step token estimates ────────────────────────────────────────────

/// Estimated input + output tokens for each enrichment step.
///
/// These are rough baselines from observed production runs.
const fn step_token_estimate(step: EnrichStep) -> (u32, u32) {
    // Returns (input_tokens, output_tokens).
    match step {
        EnrichStep::Prd => (500, 2_000),
        EnrichStep::Briefs => (1_000, 3_000),
        EnrichStep::Tasks | EnrichStep::Invariants | EnrichStep::Scribe => (1_500, 2_500),
        EnrichStep::Decompose => (2_000, 4_000),
        EnrichStep::Research => (2_000, 5_000),
        EnrichStep::Dependencies | EnrichStep::Fixtures => (1_000, 1_500),
        EnrichStep::Integration | EnrichStep::Verify | EnrichStep::Reviews => (2_000, 3_000),
        EnrichStep::Tests => (1_500, 3_500),
    }
}

// ── Model pricing ───────────────────────────────────────────────────────

/// Known model pricing tiers.
///
/// Prices are per 1M tokens in USD.
#[derive(Clone, Debug)]
struct ModelPricing {
    /// Cost per 1M input tokens.
    input_per_million: f64,
    /// Cost per 1M output tokens.
    output_per_million: f64,
}

/// Look up pricing for a model identifier.
///
/// Returns conservative estimates if the model is unknown.
fn model_pricing(model: &str) -> ModelPricing {
    // Normalize: check for key substrings.
    let m = model.to_ascii_lowercase();
    if m.contains("opus") {
        ModelPricing {
            input_per_million: 15.0,
            output_per_million: 75.0,
        }
    } else if m.contains("sonnet") {
        ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
        }
    } else if m.contains("haiku") {
        ModelPricing {
            input_per_million: 0.80,
            output_per_million: 4.0,
        }
    } else if m.contains("gpt-5.4-mini") || m.contains("gpt-4o-mini") {
        ModelPricing {
            input_per_million: 0.15,
            output_per_million: 0.60,
        }
    } else if m.contains("gpt-5") || m.contains("gpt-4o") {
        ModelPricing {
            input_per_million: 2.50,
            output_per_million: 10.0,
        }
    } else {
        // Unknown model: assume Sonnet-tier pricing as a safe default.
        ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
        }
    }
}

// ── Complexity multipliers ──────────────────────────────────────────────

/// Token multiplier based on complexity band.
///
/// More complex tasks tend to generate longer prompts and responses.
const fn complexity_multiplier(complexity: TaskComplexityBand) -> f64 {
    match complexity {
        TaskComplexityBand::Fast => 0.6,
        TaskComplexityBand::Standard => 1.0,
        // Complex + future-proof unknown bands.
        _ => 1.5,
    }
}

/// Estimated duration per step in seconds (before complexity scaling).
fn step_duration_estimate(step: EnrichStep) -> f64 {
    if step.needs_llm() {
        // LLM steps: ~10-30 seconds depending on output size.
        let (_, output_tokens) = step_token_estimate(step);
        // Rough: 50 tokens/second generation speed.
        let secs = f64::from(output_tokens) / 50.0;
        if secs < 5.0 { 5.0 } else { secs }
    } else {
        // Non-LLM steps: near-instant (file reading + formatting).
        0.5
    }
}

// ── Public types ────────────────────────────────────────────────────────

/// Estimated resource consumption for an enrichment run.
#[derive(Clone, Debug)]
pub struct EnrichmentEstimate {
    /// Total estimated tokens (input + output) across all steps.
    pub estimated_tokens: u32,
    /// Estimated input tokens.
    pub estimated_input_tokens: u32,
    /// Estimated output tokens.
    pub estimated_output_tokens: u32,
    /// Estimated cost in USD.
    pub estimated_cost_usd: f64,
    /// Estimated wall-clock duration in seconds (sequential execution).
    pub estimated_duration_secs: f64,
    /// Number of steps included in the estimate.
    pub step_count: usize,
    /// Number of steps that require LLM calls.
    pub llm_step_count: usize,
    /// Whether batch mode pricing was applied (50% discount).
    pub batch_mode: bool,
}

/// Information about the plan/task being enriched.
///
/// Used to adjust estimates based on plan characteristics.
pub struct PlanInfo {
    /// Approximate size of the plan document in characters.
    pub plan_size_chars: usize,
    /// Number of tasks in the plan (if known).
    pub task_count: Option<usize>,
}

impl PlanInfo {
    /// Create plan info with just the plan size.
    pub const fn new(plan_size_chars: usize) -> Self {
        Self {
            plan_size_chars,
            task_count: None,
        }
    }

    /// Set the task count.
    #[must_use]
    pub const fn with_task_count(mut self, count: usize) -> Self {
        self.task_count = Some(count);
        self
    }

    /// Multiplier based on plan size. Larger plans produce bigger prompts.
    fn size_multiplier(&self) -> f64 {
        // Baseline: 5000 chars. Scale linearly up to 2x for very large plans.
        #[allow(clippy::cast_precision_loss)]
        let ratio = self.plan_size_chars as f64 / 5000.0;
        ratio.clamp(0.5, 2.0)
    }
}

/// Estimate the cost and duration of running a set of enrichment steps.
///
/// # Arguments
///
/// - `plan_info` — characteristics of the plan being enriched.
/// - `complexity` — the task's complexity band.
/// - `steps` — the enrichment steps that will be executed.
/// - `model` — the model identifier for pricing lookup.
/// - `batch_mode` — whether batch API pricing applies (50% discount).
pub fn estimate_enrichment(
    plan_info: &PlanInfo,
    complexity: TaskComplexityBand,
    steps: &[EnrichStep],
    model: &str,
    batch_mode: bool,
) -> EnrichmentEstimate {
    let pricing = model_pricing(model);
    let cmul = complexity_multiplier(complexity);
    let smul = plan_info.size_multiplier();
    let combined_mul = cmul * smul;

    let mut total_input: f64 = 0.0;
    let mut total_output: f64 = 0.0;
    let mut total_duration: f64 = 0.0;
    let mut llm_step_count: usize = 0;

    for &step in steps {
        let (input_est, output_est) = step_token_estimate(step);
        total_input += f64::from(input_est) * combined_mul;
        total_output += f64::from(output_est) * combined_mul;
        total_duration += step_duration_estimate(step) * cmul;

        if step.needs_llm() {
            llm_step_count += 1;
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let estimated_input_tokens = total_input.round() as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let estimated_output_tokens = total_output.round() as u32;
    let estimated_tokens = estimated_input_tokens + estimated_output_tokens;

    // Cost calculation: per-million pricing.
    let input_cost = total_input / 1_000_000.0 * pricing.input_per_million;
    let output_cost = total_output / 1_000_000.0 * pricing.output_per_million;
    let mut estimated_cost_usd = input_cost + output_cost;

    // Batch mode: 50% discount on all costs.
    if batch_mode {
        estimated_cost_usd *= 0.5;
    }

    EnrichmentEstimate {
        estimated_tokens,
        estimated_input_tokens,
        estimated_output_tokens,
        estimated_cost_usd,
        estimated_duration_secs: total_duration,
        step_count: steps.len(),
        llm_step_count,
        batch_mode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enrichment::step::ALL_ORDERED;

    fn default_plan_info() -> PlanInfo {
        PlanInfo::new(5000)
    }

    #[test]
    fn estimate_all_steps_standard_produces_nonzero() {
        let est = estimate_enrichment(
            &default_plan_info(),
            TaskComplexityBand::Standard,
            ALL_ORDERED,
            "claude-sonnet-4-6",
            false,
        );

        assert!(est.estimated_tokens > 0, "tokens should be positive");
        assert!(est.estimated_cost_usd > 0.0, "cost should be positive");
        assert!(
            est.estimated_duration_secs > 0.0,
            "duration should be positive"
        );
        assert_eq!(est.step_count, 13);
        assert_eq!(est.llm_step_count, 6); // 6 LLM steps in ALL_ORDERED
        assert!(!est.batch_mode);
    }

    #[test]
    fn batch_mode_halves_cost() {
        let standard = estimate_enrichment(
            &default_plan_info(),
            TaskComplexityBand::Standard,
            ALL_ORDERED,
            "claude-sonnet-4-6",
            false,
        );
        let batch = estimate_enrichment(
            &default_plan_info(),
            TaskComplexityBand::Standard,
            ALL_ORDERED,
            "claude-sonnet-4-6",
            true,
        );

        let ratio = batch.estimated_cost_usd / standard.estimated_cost_usd;
        assert!(
            (ratio - 0.5).abs() < 0.001,
            "batch should be ~50% of standard cost, got ratio {ratio}"
        );
        assert!(batch.batch_mode);
        assert!(!standard.batch_mode);
    }

    #[test]
    fn fast_complexity_cheaper_than_standard() {
        let fast = estimate_enrichment(
            &default_plan_info(),
            TaskComplexityBand::Fast,
            ALL_ORDERED,
            "claude-sonnet-4-6",
            false,
        );
        let standard = estimate_enrichment(
            &default_plan_info(),
            TaskComplexityBand::Standard,
            ALL_ORDERED,
            "claude-sonnet-4-6",
            false,
        );

        assert!(
            fast.estimated_cost_usd < standard.estimated_cost_usd,
            "fast ({}) should cost less than standard ({})",
            fast.estimated_cost_usd,
            standard.estimated_cost_usd
        );
        assert!(fast.estimated_tokens < standard.estimated_tokens);
    }

    #[test]
    fn complex_more_expensive_than_standard() {
        let standard = estimate_enrichment(
            &default_plan_info(),
            TaskComplexityBand::Standard,
            ALL_ORDERED,
            "claude-sonnet-4-6",
            false,
        );
        let complex = estimate_enrichment(
            &default_plan_info(),
            TaskComplexityBand::Complex,
            ALL_ORDERED,
            "claude-sonnet-4-6",
            false,
        );

        assert!(
            complex.estimated_cost_usd > standard.estimated_cost_usd,
            "complex ({}) should cost more than standard ({})",
            complex.estimated_cost_usd,
            standard.estimated_cost_usd
        );
    }

    #[test]
    fn haiku_cheaper_than_sonnet() {
        let haiku = estimate_enrichment(
            &default_plan_info(),
            TaskComplexityBand::Standard,
            ALL_ORDERED,
            "claude-haiku-4-5-20251001",
            false,
        );
        let sonnet = estimate_enrichment(
            &default_plan_info(),
            TaskComplexityBand::Standard,
            ALL_ORDERED,
            "claude-sonnet-4-6",
            false,
        );

        assert!(
            haiku.estimated_cost_usd < sonnet.estimated_cost_usd,
            "haiku ({}) should be cheaper than sonnet ({})",
            haiku.estimated_cost_usd,
            sonnet.estimated_cost_usd
        );
    }

    #[test]
    fn opus_most_expensive() {
        let sonnet = estimate_enrichment(
            &default_plan_info(),
            TaskComplexityBand::Standard,
            ALL_ORDERED,
            "claude-sonnet-4-6",
            false,
        );
        let opus = estimate_enrichment(
            &default_plan_info(),
            TaskComplexityBand::Standard,
            ALL_ORDERED,
            "claude-opus-4-6",
            false,
        );

        assert!(
            opus.estimated_cost_usd > sonnet.estimated_cost_usd,
            "opus ({}) should be more expensive than sonnet ({})",
            opus.estimated_cost_usd,
            sonnet.estimated_cost_usd
        );
    }

    #[test]
    fn single_step_estimate() {
        let est = estimate_enrichment(
            &default_plan_info(),
            TaskComplexityBand::Standard,
            &[EnrichStep::Prd],
            "claude-sonnet-4-6",
            false,
        );

        assert_eq!(est.step_count, 1);
        assert_eq!(est.llm_step_count, 0); // Prd is non-LLM
        assert!(est.estimated_tokens > 0);
        assert!(est.estimated_cost_usd > 0.0);
    }

    #[test]
    fn empty_steps_returns_zero_estimate() {
        let est = estimate_enrichment(
            &default_plan_info(),
            TaskComplexityBand::Standard,
            &[],
            "claude-sonnet-4-6",
            false,
        );

        assert_eq!(est.estimated_tokens, 0);
        assert_eq!(est.estimated_cost_usd, 0.0);
        assert_eq!(est.estimated_duration_secs, 0.0);
        assert_eq!(est.step_count, 0);
        assert_eq!(est.llm_step_count, 0);
    }

    #[test]
    fn large_plan_increases_estimate() {
        let small_plan = PlanInfo::new(1000);
        let large_plan = PlanInfo::new(20_000);

        let small = estimate_enrichment(
            &small_plan,
            TaskComplexityBand::Standard,
            ALL_ORDERED,
            "claude-sonnet-4-6",
            false,
        );
        let large = estimate_enrichment(
            &large_plan,
            TaskComplexityBand::Standard,
            ALL_ORDERED,
            "claude-sonnet-4-6",
            false,
        );

        assert!(
            large.estimated_tokens > small.estimated_tokens,
            "large plan ({}) should have more tokens than small ({})",
            large.estimated_tokens,
            small.estimated_tokens,
        );
    }

    #[test]
    fn plan_info_with_task_count() {
        let info = PlanInfo::new(5000).with_task_count(10);
        assert_eq!(info.task_count, Some(10));
        assert_eq!(info.plan_size_chars, 5000);
    }

    #[test]
    fn unknown_model_uses_sonnet_pricing() {
        let known = estimate_enrichment(
            &default_plan_info(),
            TaskComplexityBand::Standard,
            ALL_ORDERED,
            "claude-sonnet-4-6",
            false,
        );
        let unknown = estimate_enrichment(
            &default_plan_info(),
            TaskComplexityBand::Standard,
            ALL_ORDERED,
            "some-unknown-model-v3",
            false,
        );

        // Unknown model defaults to Sonnet pricing, so costs should match.
        assert!(
            (known.estimated_cost_usd - unknown.estimated_cost_usd).abs() < 0.0001,
            "unknown model should default to sonnet pricing"
        );
    }
}
