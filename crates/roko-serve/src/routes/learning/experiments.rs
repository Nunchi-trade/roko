//! A/B experiment endpoints and statistical helpers.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::error::ApiError;
use crate::state::AppState;
use roko_learn::prompt_experiment::{ExperimentStatus, ExperimentStore, PromptExperiment};

/// `GET /api/learn/experiments` — summarize `.roko/learn/experiments.json`.
pub async fn experiments(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ExperimentsResponse>, ApiError> {
    let path = state.workdir.join(".roko/learn/experiments.json");
    let store = super::helpers::read_experiment_store(&path).await?;
    Ok(Json(build_experiments_response(&path, &store)))
}

// ── helpers ──────────────────────────────────────────────────────────

/// Build the structured experiments response from the persisted store.
fn build_experiments_response(
    path: &std::path::Path,
    store: &ExperimentStore,
) -> ExperimentsResponse {
    let active_experiments: Vec<ActiveExperimentSummary> = store
        .iter()
        .filter(|experiment| experiment.status == ExperimentStatus::Running)
        .map(summarize_experiment)
        .collect();

    ExperimentsResponse {
        source: path.display().to_string(),
        running_experiments: store.running_count(),
        concluded_experiments: store.concluded_count(),
        active_experiments,
    }
}

/// Summarize one active experiment, including variant performance and significance.
fn summarize_experiment(experiment: &PromptExperiment) -> ActiveExperimentSummary {
    let mut variants: Vec<VariantPerformance> = experiment
        .variants
        .iter()
        .map(|variant| {
            let stats = experiment
                .stats
                .get(&variant.id)
                .cloned()
                .unwrap_or_default();
            VariantPerformance {
                id: variant.id.clone(),
                name: variant.name.clone(),
                section_name: variant.section_name.clone(),
                active: variant.active,
                trials: stats.trials,
                successes: stats.successes,
                success_rate: stats.success_rate(),
            }
        })
        .collect();

    variants.sort_by(|a, b| {
        b.success_rate
            .partial_cmp(&a.success_rate)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.trials.cmp(&a.trials))
            .then_with(|| a.id.cmp(&b.id))
    });

    let significance = experiment_significance(experiment, &variants);
    let total_trials = variants.iter().map(|variant| variant.trials).sum();

    ActiveExperimentSummary {
        experiment_id: experiment.experiment_id.clone(),
        section_name: experiment.section_name.clone(),
        status: experiment.status,
        winner_id: experiment.winner_id.clone(),
        min_trials_per_variant: experiment.min_trials_per_variant,
        min_effect_size: experiment.min_effect_size,
        total_trials,
        variants,
        significance,
    }
}

/// Compute a simple significance summary from the top two active variants.
fn experiment_significance(
    experiment: &PromptExperiment,
    variants: &[VariantPerformance],
) -> ExperimentSignificance {
    let active: Vec<&VariantPerformance> =
        variants.iter().filter(|variant| variant.active).collect();
    if active.len() < 2 {
        return ExperimentSignificance {
            best_variant_id: active.first().map(|variant| variant.id.clone()),
            runner_up_variant_id: None,
            best_success_rate: active.first().map(|variant| variant.success_rate),
            runner_up_success_rate: None,
            success_rate_gap: None,
            z_score: None,
            p_value: None,
            alpha: 0.05,
            meets_effect_size_threshold: false,
            statistically_significant: false,
            note: Some("need at least two active variants".into()),
        };
    }

    let best = active[0];
    let runner_up = active[1];
    let gap = best.success_rate - runner_up.success_rate;
    let p_value = two_proportion_p_value(
        best.successes,
        best.trials,
        runner_up.successes,
        runner_up.trials,
    );
    let z_score = two_proportion_z_score(
        best.successes,
        best.trials,
        runner_up.successes,
        runner_up.trials,
    );
    let meets_effect_size_threshold = gap >= experiment.min_effect_size;
    let statistically_significant = p_value
        .map(|p| p < 0.05 && meets_effect_size_threshold)
        .unwrap_or(false);

    ExperimentSignificance {
        best_variant_id: Some(best.id.clone()),
        runner_up_variant_id: Some(runner_up.id.clone()),
        best_success_rate: Some(best.success_rate),
        runner_up_success_rate: Some(runner_up.success_rate),
        success_rate_gap: Some(gap),
        z_score,
        p_value,
        alpha: 0.05,
        meets_effect_size_threshold,
        statistically_significant,
        note: None,
    }
}

/// Approximate two-sided p-value for a two-proportion z-test.
fn two_proportion_p_value(
    successes_a: u64,
    trials_a: u64,
    successes_b: u64,
    trials_b: u64,
) -> Option<f64> {
    let z = two_proportion_z_score(successes_a, trials_a, successes_b, trials_b)?;
    Some(2.0 * (1.0 - standard_normal_cdf(z.abs())))
}

/// Compute the z-score for a two-proportion z-test.
fn two_proportion_z_score(
    successes_a: u64,
    trials_a: u64,
    successes_b: u64,
    trials_b: u64,
) -> Option<f64> {
    if trials_a == 0 || trials_b == 0 {
        return None;
    }

    let p1 = successes_a as f64 / trials_a as f64;
    let p2 = successes_b as f64 / trials_b as f64;
    let pooled = (successes_a + successes_b) as f64 / (trials_a + trials_b) as f64;
    let standard_error =
        (pooled * (1.0 - pooled) * (1.0 / trials_a as f64 + 1.0 / trials_b as f64)).sqrt();
    if standard_error == 0.0 {
        return None;
    }

    Some((p1 - p2) / standard_error)
}

/// Approximate the CDF of the standard normal distribution.
fn standard_normal_cdf(x: f64) -> f64 {
    // Abramowitz and Stegun 7.1.26 approximation.
    let t = 1.0 / (1.0 + 0.231_641_9 * x.abs());
    let d = 0.398_942_3 * (-0.5 * x * x).exp();
    let prob = d
        * t
        * (0.319_381_5 + t * (-0.356_563_8 + t * (1.781_478 + t * (-1.821_256 + t * 1.330_274))));
    if x >= 0.0 { 1.0 - prob } else { prob }
}

// ── types ────────────────────────────────────────────────────────────

/// Structured API response for `GET /api/learn/experiments`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExperimentsResponse {
    pub source: String,
    pub running_experiments: usize,
    pub concluded_experiments: usize,
    pub active_experiments: Vec<ActiveExperimentSummary>,
}

/// Summary for one active experiment.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ActiveExperimentSummary {
    experiment_id: String,
    section_name: String,
    status: ExperimentStatus,
    winner_id: Option<String>,
    min_trials_per_variant: u64,
    min_effect_size: f64,
    total_trials: u64,
    variants: Vec<VariantPerformance>,
    significance: ExperimentSignificance,
}

/// Per-variant performance summary.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct VariantPerformance {
    id: String,
    name: String,
    section_name: String,
    active: bool,
    trials: u64,
    successes: u64,
    success_rate: f64,
}

/// Statistical significance summary for the top two active variants.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct ExperimentSignificance {
    best_variant_id: Option<String>,
    runner_up_variant_id: Option<String>,
    best_success_rate: Option<f64>,
    runner_up_success_rate: Option<f64>,
    success_rate_gap: Option<f64>,
    z_score: Option<f64>,
    p_value: Option<f64>,
    alpha: f64,
    meets_effect_size_threshold: bool,
    statistically_significant: bool,
    note: Option<String>,
}
