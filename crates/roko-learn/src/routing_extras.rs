//! Routing extras for lookahead and calibration support.
//!
//! These types capture the documented lookahead router and calibration
//! scaffolding that sit around the core cascade router.

#![allow(dead_code)]

use crate::cascade_router::{CascadeModel, CascadeRouter};
use crate::model_router::RoutingContext;
use roko_core::agent::ModelSpec;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Placeholder task dependency graph used by the lookahead router.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDag {}

impl TaskDag {
    /// Create an empty task DAG shell.
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }

    /// Whether the shell currently contains any task data.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        true
    }
}

/// Cache reuse statistics keyed by `(model, role)`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheReuseModel {
    /// Per-(model, role) estimated cache hit rate when reusing the same model.
    cache_hit_rates: HashMap<(String, String), f64>,
    /// Average input tokens saved per cache hit.
    avg_tokens_saved_per_hit: u64,
    /// Cost per 1M tokens for cache reads vs fresh input.
    cache_read_discount: f64,
}

impl CacheReuseModel {
    /// Create a cache reuse model with conservative defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache_hit_rates: HashMap::new(),
            avg_tokens_saved_per_hit: 0,
            cache_read_discount: 1.0,
        }
    }

    /// Override the cache-read discount.
    #[must_use]
    pub fn with_cache_read_discount(mut self, discount: f64) -> Self {
        self.cache_read_discount = if discount.is_finite() && discount >= 0.0 {
            discount
        } else {
            1.0
        };
        self
    }

    /// Set the average input-token savings per cache hit.
    #[must_use]
    pub fn with_avg_tokens_saved_per_hit(mut self, tokens: u64) -> Self {
        self.avg_tokens_saved_per_hit = tokens;
        self
    }

    /// Record the estimated cache-hit rate for a `(model, role)` pair.
    pub fn record_hit_rate(
        &mut self,
        model: impl Into<String>,
        role: impl Into<String>,
        hit_rate: f64,
    ) {
        self.cache_hit_rates
            .insert((model.into(), role.into()), hit_rate.clamp(0.0, 1.0));
    }

    /// Read the estimated cache-hit rate for a `(model, role)` pair.
    #[must_use]
    pub fn cache_hit_rate(&self, model: &str, role: &str) -> f64 {
        self.cache_hit_rates
            .get(&(model.to_string(), role.to_string()))
            .copied()
            .unwrap_or(0.0)
    }

    /// Estimate tokens saved for a candidate model and role.
    #[must_use]
    pub fn estimated_tokens_saved(&self, model: &str, role: &str) -> f64 {
        self.cache_hit_rate(model, role) * self.avg_tokens_saved_per_hit as f64
    }

    /// Estimate cost saved for a candidate model and role.
    #[must_use]
    pub fn estimated_cost_saved(&self, model: &str, role: &str) -> f64 {
        self.estimated_tokens_saved(model, role) * self.cache_read_discount / 1_000_000.0
    }
}

/// Lookahead wrapper around the core cascade router.
pub struct LookaheadRouter {
    /// Base cascade router for individual decisions.
    inner: CascadeRouter,
    /// Task dependency graph for lookahead.
    task_graph: TaskDag,
    /// Lookahead horizon (default: 3 tasks ahead).
    pub horizon: usize,
    /// Discount factor for future savings (default: 0.9).
    pub gamma: f64,
    /// KV cache reuse probability model.
    cache_model: CacheReuseModel,
}

impl LookaheadRouter {
    /// Create a lookahead router wrapper.
    #[must_use]
    pub fn new(inner: CascadeRouter, task_graph: TaskDag) -> Self {
        Self {
            inner,
            task_graph,
            horizon: 3,
            gamma: 0.9,
            cache_model: CacheReuseModel::new(),
        }
    }

    /// Override the lookahead horizon.
    #[must_use]
    pub fn with_horizon(mut self, horizon: usize) -> Self {
        self.horizon = horizon.max(1);
        self
    }

    /// Override the future-savings discount factor.
    #[must_use]
    pub fn with_gamma(mut self, gamma: f64) -> Self {
        self.gamma = if gamma.is_finite() {
            gamma.clamp(0.0, 1.0)
        } else {
            0.9
        };
        self
    }

    /// Access the wrapped cascade router.
    #[must_use]
    pub const fn inner(&self) -> &CascadeRouter {
        &self.inner
    }

    /// Access the task graph used for lookahead planning.
    #[must_use]
    pub const fn task_graph(&self) -> &TaskDag {
        &self.task_graph
    }

    /// Access the cache reuse model.
    #[must_use]
    pub const fn cache_model(&self) -> &CacheReuseModel {
        &self.cache_model
    }

    /// Mutably access the cache reuse model.
    #[must_use]
    pub fn cache_model_mut(&mut self) -> &mut CacheReuseModel {
        &mut self.cache_model
    }

    /// Route with tier-downgrade lookahead.
    ///
    /// Before committing to the cascade-selected model, checks whether a
    /// cheaper tier has sufficiently high estimated success probability. If
    /// `P(success | cheaper_tier) > threshold`, the cheaper model is returned
    /// instead, saving cost without meaningful quality loss.
    #[must_use]
    pub fn route_with_lookahead(
        &self,
        ctx: &RoutingContext,
        calibration: &RouterCalibration,
        threshold: f64,
    ) -> CascadeModel {
        let baseline = self.inner.route(ctx);
        let baseline_tier = tier_rank_for_slug(&baseline.primary.slug);

        // Only attempt downgrade if we're at Standard or Premium.
        if baseline_tier == 0 {
            return baseline;
        }

        // Check each cheaper tier from cheapest up.
        let candidates = self.inner.model_slugs();
        for candidate_slug in candidates {
            let candidate_tier = tier_rank_for_slug(candidate_slug);
            if candidate_tier >= baseline_tier {
                continue;
            }

            // Look up calibration data for this candidate.
            if let Some(cal) = calibration.calibration(candidate_slug) {
                // Estimate success probability from the bin data.
                let success_prob = estimate_success_probability(cal);
                if success_prob > threshold {
                    // Cheaper model is likely to succeed — use it.
                    let mut downgraded = baseline.clone();
                    downgraded.primary = ModelSpec::from_slug(candidate_slug);
                    return downgraded;
                }
            }
        }

        baseline
    }
}

/// Estimate success probability from calibration data.
///
/// Uses the weighted average of actual success rates across non-empty bins.
fn estimate_success_probability(cal: &ModelCalibration) -> f64 {
    let mut total_count = 0u32;
    let mut weighted_rate = 0.0;
    for bin in &cal.bins {
        if bin.count > 0 {
            total_count += bin.count;
            weighted_rate += bin.actual_rate * bin.count as f64;
        }
    }
    if total_count == 0 {
        return 0.5; // No data — neutral prior.
    }
    weighted_rate / total_count as f64
}

/// Map a model slug to a tier rank (0 = fast/cheap, 1 = standard, 2 = premium).
fn tier_rank_for_slug(slug: &str) -> u8 {
    if slug.contains("gemini-2.5-flash-lite")
        || slug.contains("gemini-3.1-flash-lite-preview")
        || slug.contains("haiku")
    {
        0
    } else if slug.contains("opus")
        || slug.contains("premium")
        || slug.contains("gemini-3.1-pro-preview")
    {
        2
    } else {
        1
    }
}

/// Router calibration state for model confidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterCalibration {
    /// Per-model calibration data.
    calibrations: HashMap<String, ModelCalibration>,
    /// Overall calibration score (lower is better, 0 = perfect).
    pub brier_score: f64,
    /// Recalibration interval (default: every 100 routing decisions).
    pub recalibrate_interval: u32,
}

impl Default for RouterCalibration {
    fn default() -> Self {
        Self {
            calibrations: HashMap::new(),
            brier_score: 0.0,
            recalibrate_interval: 100,
        }
    }
}

impl RouterCalibration {
    /// Create a router calibration tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the recalibration interval.
    #[must_use]
    pub fn with_recalibrate_interval(mut self, recalibrate_interval: u32) -> Self {
        self.recalibrate_interval = recalibrate_interval.max(1);
        self
    }

    /// Whether any calibration data has been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.calibrations.is_empty()
    }

    /// Read the calibration state for a model, if present.
    #[must_use]
    pub fn calibration(&self, model: &str) -> Option<&ModelCalibration> {
        self.calibrations.get(model)
    }

    /// Read or create the calibration state for a model.
    pub fn calibration_mut(&mut self, model: impl Into<String>) -> &mut ModelCalibration {
        let model = model.into();
        self.calibrations
            .entry(model.clone())
            .or_insert_with(|| ModelCalibration::new(model))
    }

    /// Record a prediction/outcome pair for a model.
    pub fn record_prediction(
        &mut self,
        model: impl Into<String>,
        predicted_probability: f64,
        actual_success: bool,
    ) {
        let model = model.into();
        let calibration = self
            .calibrations
            .entry(model.clone())
            .or_insert_with(|| ModelCalibration::new(model));
        calibration.record_prediction(predicted_probability, actual_success);
        self.brier_score = self
            .calibrations
            .values()
            .flat_map(|calibration| calibration.predictions.iter())
            .map(|(predicted_probability, actual_success)| {
                let actual_probability = if *actual_success { 1.0 } else { 0.0 };
                let delta = predicted_probability - actual_probability;
                delta * delta
            })
            .sum::<f64>()
            / (self
                .calibrations
                .values()
                .map(|calibration| calibration.predictions.len())
                .sum::<usize>()
                .max(1) as f64);
    }
}

/// Per-model calibration data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCalibration {
    /// Model slug.
    pub model: String,
    /// Predicted pass probabilities and actual outcomes.
    predictions: Vec<(f64, bool)>,
    /// Calibration bins (10 bins, 0-10%, 10-20%, ..., 90-100%).
    pub bins: [CalibrationBin; 10],
    /// Platt scaling parameters: a, b for sigmoid(a × raw_score + b).
    pub platt_a: f64,
    /// Platt scaling offset.
    pub platt_b: f64,
    /// Isotonic regression mapping (non-parametric calibration).
    pub isotonic_map: Vec<(f64, f64)>,
}

impl ModelCalibration {
    /// Create empty calibration state for a model.
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            predictions: Vec::new(),
            bins: std::array::from_fn(|idx| {
                CalibrationBin::new(idx as f64 / 10.0, (idx + 1) as f64 / 10.0)
            }),
            platt_a: 1.0,
            platt_b: 0.0,
            isotonic_map: Vec::new(),
        }
    }

    /// Record a prediction/outcome pair.
    pub fn record_prediction(&mut self, predicted_probability: f64, actual_success: bool) {
        let predicted_probability = predicted_probability.clamp(0.0, 1.0);
        self.predictions
            .push((predicted_probability, actual_success));

        let bin_index = ((predicted_probability * 10.0).floor() as usize).min(9);
        let bin = &mut self.bins[bin_index];
        bin.count = bin.count.saturating_add(1);
        let actual_rate = if actual_success { 1.0 } else { 0.0 };
        let prior = (bin.count.saturating_sub(1)) as f64;
        bin.actual_rate = if bin.count == 0 {
            actual_rate
        } else {
            (bin.actual_rate * prior + actual_rate) / bin.count as f64
        };
        bin.ece_contribution = (bin.actual_rate - predicted_probability).abs();
    }

    /// Number of recorded prediction samples.
    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.predictions.len()
    }

    /// Read the calibration bin for a probability.
    #[must_use]
    pub fn bin_for_probability(&self, probability: f64) -> &CalibrationBin {
        let index = ((probability.clamp(0.0, 1.0) * 10.0).floor() as usize).min(9);
        &self.bins[index]
    }
}

/// One calibration bin within a model calibration table.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CalibrationBin {
    /// Bin range lower bound.
    pub lower: f64,
    /// Bin range upper bound.
    pub upper: f64,
    /// Number of predictions in this bin.
    pub count: u32,
    /// Actual success rate within this bin.
    pub actual_rate: f64,
    /// Expected Calibration Error for this bin.
    pub ece_contribution: f64,
}

impl CalibrationBin {
    /// Create a bin with explicit bounds.
    #[must_use]
    pub const fn new(lower: f64, upper: f64) -> Self {
        Self {
            lower,
            upper,
            count: 0,
            actual_rate: 0.0,
            ece_contribution: 0.0,
        }
    }

    /// Whether the supplied probability falls within the bin.
    #[must_use]
    pub const fn contains(&self, probability: f64) -> bool {
        probability >= self.lower && probability <= self.upper
    }

    /// Width of the bin.
    #[must_use]
    pub const fn width(&self) -> f64 {
        self.upper - self.lower
    }
}
