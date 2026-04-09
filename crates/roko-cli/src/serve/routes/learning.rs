//! Learning data endpoints — efficiency, cascade router, experiments, gate thresholds.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use serde_json::Value;

use crate::serve::error::ApiError;
use crate::serve::state::AppState;
use roko_gate::adaptive_threshold::AdaptiveThresholds;
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_learn::cascade_router::CascadeStage;
use roko_learn::model_router::COLD_START_THRESHOLD;
use roko_learn::prompt_experiment::{ExperimentStatus, ExperimentStore, PromptExperiment};
use roko_learn::runtime_feedback::read_efficiency_events;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/learning/efficiency", get(efficiency))
        .route("/learn/efficiency", get(efficiency))
        .route("/learning/cascade-router", get(cascade_router))
        .route("/learn/cascade", get(cascade))
        .route("/learn/experiments", get(experiments))
        .route("/learning/experiments", get(experiments))
        .route("/learn/adaptive-thresholds", get(adaptive_thresholds))
        .route("/learning/adaptive-thresholds", get(adaptive_thresholds))
        .route("/learning/gate-thresholds", get(gate_thresholds))
}

/// `GET /api/learn/efficiency` — aggregate `.roko/learn/efficiency.jsonl`.
async fn efficiency(State(state): State<Arc<AppState>>) -> Result<Json<EfficiencyResponse>, ApiError> {
    let path = state.workdir.join(".roko/learn/efficiency.jsonl");
    let events = read_efficiency_events(&path)
        .await
        .map_err(|e| ApiError::internal(format!("read {}: {e}", path.display())))?;
    Ok(Json(build_efficiency_response(&events)))
}

/// `GET /api/learning/cascade-router` — read `.roko/learn/cascade-router.json`.
async fn cascade_router(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko/learn/cascade-router.json");
    read_json_file(&path).await
}

/// `GET /api/learn/cascade` — summarize `.roko/learn/cascade-router.json`.
async fn cascade(State(state): State<Arc<AppState>>) -> Result<Json<CascadeResponse>, ApiError> {
    let path = state.workdir.join(".roko/learn/cascade-router.json");
    let snapshot = read_cascade_snapshot(&path).await?;
    Ok(Json(build_cascade_response(&path, snapshot)))
}

/// `GET /api/learn/experiments` — summarize `.roko/learn/experiments.json`.
async fn experiments(State(state): State<Arc<AppState>>) -> Result<Json<ExperimentsResponse>, ApiError> {
    let path = state.workdir.join(".roko/learn/experiments.json");
    let store = read_experiment_store(&path).await?;
    Ok(Json(build_experiments_response(&path, &store)))
}

/// `GET /api/learning/gate-thresholds` — read `.roko/learn/gate-thresholds.json`.
async fn gate_thresholds(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko/learn/gate-thresholds.json");
    read_json_file(&path).await
}

/// `GET /api/learn/adaptive-thresholds` — summarize `.roko/learn/gate-thresholds.json`.
async fn adaptive_thresholds(
    State(state): State<Arc<AppState>>,
) -> Result<Json<AdaptiveThresholdsResponse>, ApiError> {
    let path = state.workdir.join(".roko/learn/gate-thresholds.json");
    let thresholds = AdaptiveThresholds::load_or_new(&path);
    Ok(Json(build_adaptive_thresholds_response(&path, &thresholds)))
}

// ── helpers ──────────────────────────────────────────────────────────

/// Read a JSON file and return its parsed value.
async fn read_json_file(path: &std::path::Path) -> Result<Json<Value>, ApiError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Json(Value::Null));
        }
        Err(e) => {
            return Err(ApiError::internal(format!("read {}: {e}", path.display())));
        }
    };
    let value: Value = serde_json::from_str(&content)
        .map_err(|e| ApiError::internal(format!("parse {}: {e}", path.display())))?;
    Ok(Json(value))
}

/// Read and parse the persisted experiment store.
async fn read_experiment_store(path: &std::path::Path) -> Result<ExperimentStore, ApiError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApiError::not_found(format!(
                "{} not found",
                path.display()
            )));
        }
        Err(e) => {
            return Err(ApiError::internal(format!("read {}: {e}", path.display())));
        }
    };

    serde_json::from_str(&content)
        .map_err(|e| ApiError::internal(format!("parse {}: {e}", path.display())))
}

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

/// Aggregate efficiency events into task-level cost and timing metrics.
fn build_efficiency_response(events: &[AgentEfficiencyEvent]) -> EfficiencyResponse {
    let mut tasks: HashMap<TaskKey, TaskEfficiencyAggregate> = HashMap::new();

    for (index, event) in events.iter().enumerate() {
        let key = TaskKey {
            plan_id: event.plan_id.clone(),
            task_id: event.task_id.clone(),
        };
        let aggregate = tasks.entry(key).or_insert_with(TaskEfficiencyAggregate::default);
        aggregate.record(event, index);
    }

    let mut task_summaries: Vec<TaskEfficiencySummary> = tasks
        .into_iter()
        .map(|(key, aggregate)| TaskEfficiencySummary {
            plan_id: key.plan_id,
            task_id: key.task_id,
            timestamp: aggregate.timestamp,
            cost_usd: aggregate.cost_usd,
            tokens: aggregate.tokens,
            duration_ms: aggregate.duration_ms,
            sequence: aggregate.sequence,
        })
        .collect();

    task_summaries.sort_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then_with(|| a.sequence.cmp(&b.sequence))
            .then_with(|| a.plan_id.cmp(&b.plan_id))
            .then_with(|| a.task_id.cmp(&b.task_id))
    });

    let task_count = task_summaries.len() as f64;
    let total_cost: f64 = task_summaries.iter().map(|task| task.cost_usd).sum();
    let total_tokens: u64 = task_summaries.iter().map(|task| task.tokens).sum();
    let total_duration_ms: u64 = task_summaries.iter().map(|task| task.duration_ms).sum();

    let mut cumulative_cost = 0.0;
    let cost_trend = task_summaries
        .iter()
        .map(|task| {
            cumulative_cost += task.cost_usd;
            CostTrendPoint {
                timestamp: task.timestamp.clone(),
                cost_usd: task.cost_usd,
                cumulative_cost_usd: cumulative_cost,
            }
        })
        .collect();

    EfficiencyResponse {
        total_cost,
        cost_per_task: if task_count == 0.0 {
            0.0
        } else {
            total_cost / task_count
        },
        tokens_per_task: if task_count == 0.0 {
            0.0
        } else {
            total_tokens as f64 / task_count
        },
        avg_task_duration: if task_count == 0.0 {
            0.0
        } else {
            total_duration_ms as f64 / task_count
        },
        cost_trend,
    }
}

/// Build a structured response from the adaptive gate threshold store.
fn build_adaptive_thresholds_response(
    path: &std::path::Path,
    thresholds: &AdaptiveThresholds,
) -> AdaptiveThresholdsResponse {
    let mut rungs: Vec<RungThresholdSummary> = thresholds
        .all_rungs()
        .map(|(rung, stats)| RungThresholdSummary {
            rung: *rung,
            ema_pass_rate: stats.ema_pass_rate,
            total_observations: stats.total_observations,
            consecutive_passes: stats.consecutive_passes,
            suggested_max_retries: thresholds.suggested_max_retries(*rung),
            should_skip_rung: thresholds.should_skip_rung(*rung),
        })
        .collect();

    rungs.sort_by_key(|summary| summary.rung);

    AdaptiveThresholdsResponse {
        source: path.display().to_string(),
        tracked_rungs: rungs.len(),
        rungs,
    }
}

/// Summarize one active experiment, including variant performance and significance.
fn summarize_experiment(experiment: &PromptExperiment) -> ActiveExperimentSummary {
    let mut variants: Vec<VariantPerformance> = experiment
        .variants
        .iter()
        .map(|variant| {
            let stats = experiment.stats.get(&variant.id).cloned().unwrap_or_default();
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
    let active: Vec<&VariantPerformance> = variants.iter().filter(|variant| variant.active).collect();
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
    let p_value = two_proportion_p_value(best.successes, best.trials, runner_up.successes, runner_up.trials);
    let z_score = two_proportion_z_score(best.successes, best.trials, runner_up.successes, runner_up.trials);
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
        * (0.319_381_5
            + t * (-0.356_563_8 + t * (1.781_478 + t * (-1.821_256 + t * 1.330_274))));
    if x >= 0.0 { 1.0 - prob } else { prob }
}

/// Read and parse a cascade snapshot, or return `None` if the file is missing.
async fn read_cascade_snapshot(path: &std::path::Path) -> Result<Option<CascadeSnapshotData>, ApiError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(e) => {
            return Err(ApiError::internal(format!("read {}: {e}", path.display())));
        }
    };

    let snapshot = serde_json::from_str::<CascadeSnapshotData>(&content)
        .map_err(|e| ApiError::internal(format!("parse {}: {e}", path.display())))?;
    Ok(Some(snapshot))
}

/// Build a structured cascade response from persisted snapshot data.
fn build_cascade_response(
    path: &std::path::Path,
    snapshot: Option<CascadeSnapshotData>,
) -> CascadeResponse {
    let snapshot = snapshot.unwrap_or_default();
    let total_observations = snapshot
        .confidence_stats
        .values()
        .map(|stats| stats.trials)
        .sum::<u64>();
    let stage = stage_for_observations(total_observations);
    let weights = compute_model_weights(&snapshot);
    let routing_stats = CascadeRoutingStats {
        current_stage: stage.label().to_string(),
        total_observations,
        registered_models: snapshot.model_slugs.len(),
        observed_models: snapshot.confidence_stats.len(),
        best_model: weights.first().map(|weight| weight.model.clone()),
    };

    let recommendations = task_recommendations(&weights);

    CascadeResponse {
        source: path.display().to_string(),
        current_stage: stage.label().to_string(),
        model_weights: weights,
        routing_stats,
        recommended_models: recommendations,
    }
}

/// Infer the cascade stage from the number of recorded observations.
fn stage_for_observations(observations: u64) -> CascadeStage {
    if observations < COLD_START_THRESHOLD {
        CascadeStage::Static
    } else if observations < 200 {
        CascadeStage::Confidence
    } else {
        CascadeStage::Ucb
    }
}

/// Compute normalized model weights from the confidence-stage snapshot.
fn compute_model_weights(snapshot: &CascadeSnapshotData) -> Vec<CascadeModelWeight> {
    let mut weights: Vec<CascadeModelWeight> = snapshot
        .model_slugs
        .iter()
        .chain(snapshot.confidence_stats.keys())
        .fold(Vec::<String>::new(), |mut acc, slug| {
            if !acc.iter().any(|seen| seen == slug) {
                acc.push(slug.clone());
            }
            acc
        })
        .into_iter()
        .map(|model| {
            let stats = snapshot.confidence_stats.get(&model);
            let trials = stats.map(|s| s.trials).unwrap_or(0);
            let successes = stats.map(|s| s.successes).unwrap_or(0);
            let upper_confidence_bound = confidence_upper_bound(trials, successes);
            CascadeModelWeight {
                model,
                trials,
                successes,
                pass_rate: pass_rate(trials, successes),
                upper_confidence_bound,
                normalized_weight: 0.0,
            }
        })
        .collect();

    let total_weight = weights
        .iter()
        .map(|weight| weight.upper_confidence_bound)
        .sum::<f64>()
        .max(f64::EPSILON);

    for weight in &mut weights {
        weight.normalized_weight = weight.upper_confidence_bound / total_weight;
    }

    weights.sort_by(|a, b| {
        b.normalized_weight
            .partial_cmp(&a.normalized_weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.model.cmp(&b.model))
    });

    weights
}

/// Build one recommendation per task category.
fn task_recommendations(weights: &[CascadeModelWeight]) -> Vec<TaskRecommendation> {
    const TASK_CATEGORIES: [TaskCategory; 8] = [
        TaskCategory::Scaffolding,
        TaskCategory::Implementation,
        TaskCategory::Integration,
        TaskCategory::Verification,
        TaskCategory::Research,
        TaskCategory::Refactor,
        TaskCategory::Infra,
        TaskCategory::Docs,
    ];

    TASK_CATEGORIES
        .into_iter()
        .map(|category| {
            let complexity = complexity_for_category(category);
            let preferred_tier = tier_for_complexity(complexity);
            let selected = select_model_for_tier(weights, preferred_tier);
            TaskRecommendation {
                task_category: category.label().to_string(),
                complexity_band: complexity.label().to_string(),
                recommended_model: selected
                    .map(|weight| weight.model.clone())
                    .unwrap_or_else(|| "unknown".to_string()),
                weight: selected.map(|weight| weight.normalized_weight).unwrap_or(0.0),
            }
        })
        .collect()
}

/// Map a task category to the complexity band used for routing.
fn complexity_for_category(category: TaskCategory) -> TaskComplexityBand {
    match category {
        TaskCategory::Scaffolding | TaskCategory::Docs => TaskComplexityBand::Fast,
        TaskCategory::Research | TaskCategory::Refactor => TaskComplexityBand::Complex,
        TaskCategory::Implementation
        | TaskCategory::Integration
        | TaskCategory::Verification
        | TaskCategory::Infra
        | _ => TaskComplexityBand::Standard,
    }
}

/// Convert complexity to the corresponding model tier label.
fn tier_for_complexity(complexity: TaskComplexityBand) -> &'static str {
    match complexity {
        TaskComplexityBand::Fast => "fast",
        TaskComplexityBand::Complex => "premium",
        TaskComplexityBand::Standard | _ => "standard",
    }
}

/// Select the highest-weight model compatible with the requested tier.
fn select_model_for_tier<'a>(
    weights: &'a [CascadeModelWeight],
    tier: &str,
) -> Option<&'a CascadeModelWeight> {
    weights
        .iter()
        .filter(|weight| tier_for_model(&weight.model) == tier)
        .max_by(|a, b| {
            a.normalized_weight
                .partial_cmp(&b.normalized_weight)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.model.cmp(&b.model))
        })
        .or_else(|| weights.first())
}

/// Infer a coarse tier label from a model slug.
fn tier_for_model(model: &str) -> &'static str {
    if model.contains("haiku") {
        "fast"
    } else if model.contains("opus") || model.contains("premium") {
        "premium"
    } else {
        "standard"
    }
}

/// Empirical pass rate for a model.
fn pass_rate(trials: u64, successes: u64) -> f64 {
    if trials == 0 {
        0.0
    } else {
        successes as f64 / trials as f64
    }
}

/// Approximate UCB-style confidence bound used by the cascade.
fn confidence_upper_bound(trials: u64, successes: u64) -> f64 {
    if trials == 0 {
        return 1.0;
    }

    let p = pass_rate(trials, successes);
    let width = 1.96 * (p * (1.0 - p) / trials as f64).sqrt();
    (p + width).min(1.0)
}

/// Parsed cascade router snapshot matching the persisted JSON format.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct CascadeSnapshotData {
    #[serde(default)]
    model_slugs: Vec<String>,
    #[serde(default)]
    confidence_stats: HashMap<String, PersistedModelStatsData>,
}

/// Per-model confidence stats from the cascade router JSON.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct PersistedModelStatsData {
    #[serde(default)]
    trials: u64,
    #[serde(default)]
    successes: u64,
}

/// Structured API response for `GET /api/learn/cascade`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
struct CascadeResponse {
    source: String,
    current_stage: String,
    model_weights: Vec<CascadeModelWeight>,
    routing_stats: CascadeRoutingStats,
    recommended_models: Vec<TaskRecommendation>,
}

/// Normalized per-model weight details.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
struct CascadeModelWeight {
    model: String,
    trials: u64,
    successes: u64,
    pass_rate: f64,
    upper_confidence_bound: f64,
    normalized_weight: f64,
}

/// Aggregate routing stats derived from the snapshot.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
struct CascadeRoutingStats {
    current_stage: String,
    total_observations: u64,
    registered_models: usize,
    observed_models: usize,
    best_model: Option<String>,
}

/// Recommended model for one task category.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
struct TaskRecommendation {
    task_category: String,
    complexity_band: String,
    recommended_model: String,
    weight: f64,
}

/// Structured API response for `GET /api/learn/experiments`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct ExperimentsResponse {
    source: String,
    running_experiments: usize,
    concluded_experiments: usize,
    active_experiments: Vec<ActiveExperimentSummary>,
}

/// Summary for one active experiment.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct ActiveExperimentSummary {
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

#[derive(Debug, Clone, Default, Eq, Hash, PartialEq)]
struct TaskKey {
    plan_id: String,
    task_id: String,
}

#[derive(Debug, Clone, Default)]
struct TaskEfficiencyAggregate {
    cost_usd: f64,
    tokens: u64,
    duration_ms: u64,
    timestamp: String,
    sequence: usize,
}

impl TaskEfficiencyAggregate {
    fn record(&mut self, event: &AgentEfficiencyEvent, sequence: usize) {
        self.cost_usd += event.cost_usd;
        self.tokens += event.total_tokens();
        self.duration_ms += event.duration_ms.max(event.wall_time_ms);

        if self.timestamp.is_empty() || event.timestamp >= self.timestamp {
            self.timestamp = event.timestamp.clone();
            self.sequence = sequence;
        }
    }
}

/// Cost trend point derived from the aggregated efficiency events.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct CostTrendPoint {
    timestamp: String,
    cost_usd: f64,
    cumulative_cost_usd: f64,
}

/// Task-level efficiency summary derived from the JSONL event stream.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct TaskEfficiencySummary {
    plan_id: String,
    task_id: String,
    timestamp: String,
    cost_usd: f64,
    tokens: u64,
    duration_ms: u64,
    sequence: usize,
}

/// Structured API response for `GET /api/learn/efficiency`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct EfficiencyResponse {
    total_cost: f64,
    cost_per_task: f64,
    tokens_per_task: f64,
    avg_task_duration: f64,
    cost_trend: Vec<CostTrendPoint>,
}

/// Structured API response for `GET /api/learn/adaptive-thresholds`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct AdaptiveThresholdsResponse {
    source: String,
    tracked_rungs: usize,
    rungs: Vec<RungThresholdSummary>,
}

/// One rung entry from the adaptive threshold store.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct RungThresholdSummary {
    rung: u32,
    ema_pass_rate: f64,
    total_observations: u64,
    consecutive_passes: u32,
    suggested_max_retries: u32,
    should_skip_rung: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use axum::extract::State;
    use tempfile::tempdir;

    use crate::config::Config;
    use crate::serve::deploy::create_backend;
    use crate::serve::state::AppState;

    fn make_experiment() -> PromptExperiment {
        PromptExperiment {
            experiment_id: "exp-1".into(),
            section_name: "system_prompt".into(),
            variants: vec![
                roko_learn::prompt_experiment::PromptVariant {
                    id: "baseline".into(),
                    name: "Baseline".into(),
                    section_name: "system_prompt".into(),
                    content: "v1".into(),
                    active: true,
                },
                roko_learn::prompt_experiment::PromptVariant {
                    id: "verbose".into(),
                    name: "Verbose".into(),
                    section_name: "system_prompt".into(),
                    content: "v2".into(),
                    active: true,
                },
            ],
            stats: HashMap::from([
                (
                    "baseline".into(),
                    roko_learn::prompt_experiment::VariantStats {
                        trials: 10,
                        successes: 8,
                    },
                ),
                (
                    "verbose".into(),
                    roko_learn::prompt_experiment::VariantStats {
                        trials: 10,
                        successes: 5,
                    },
                ),
            ]),
            status: ExperimentStatus::Running,
            winner_id: None,
            min_trials_per_variant: 5,
            min_effect_size: 0.1,
        }
    }

    fn snapshot() -> CascadeSnapshotData {
        let mut confidence_stats = HashMap::new();
        confidence_stats.insert(
            "claude-sonnet-4-5".to_string(),
            PersistedModelStatsData {
                trials: 50,
                successes: 45,
            },
        );
        confidence_stats.insert(
            "claude-haiku-3-5".to_string(),
            PersistedModelStatsData {
                trials: 30,
                successes: 20,
            },
        );

        CascadeSnapshotData {
            model_slugs: vec![
                "claude-sonnet-4-5".to_string(),
                "claude-haiku-3-5".to_string(),
                "claude-opus-4".to_string(),
            ],
            confidence_stats,
        }
    }

    #[test]
    fn cascade_response_summarizes_weights_and_recommendations() {
        let response = build_cascade_response(std::path::Path::new("/tmp/.roko/learn/cascade-router.json"), Some(snapshot()));

        assert_eq!(response.current_stage, "confidence");
        assert_eq!(response.routing_stats.total_observations, 80);
        assert_eq!(response.routing_stats.registered_models, 3);
        assert_eq!(response.model_weights.len(), 3);
        assert!((response.model_weights.iter().map(|w| w.normalized_weight).sum::<f64>() - 1.0).abs() < 1e-9);

        let docs = response
            .recommended_models
            .iter()
            .find(|rec| rec.task_category == "docs")
            .expect("docs recommendation");
        assert_eq!(docs.complexity_band, "fast");
        assert_eq!(docs.recommended_model, "claude-haiku-3-5");

        let implementation = response
            .recommended_models
            .iter()
            .find(|rec| rec.task_category == "implementation")
            .expect("implementation recommendation");
        assert_eq!(implementation.complexity_band, "standard");
        assert_eq!(implementation.recommended_model, "claude-sonnet-4-5");

        let research = response
            .recommended_models
            .iter()
            .find(|rec| rec.task_category == "research")
            .expect("research recommendation");
        assert_eq!(research.complexity_band, "complex");
        assert_eq!(research.recommended_model, "claude-opus-4");
    }

    #[test]
    fn experiments_response_summarizes_active_experiments() {
        let mut store = ExperimentStore::new();
        store.register(make_experiment());

        let response =
            build_experiments_response(std::path::Path::new("/tmp/.roko/learn/experiments.json"), &store);

        assert_eq!(response.running_experiments, 1);
        assert_eq!(response.concluded_experiments, 0);
        assert_eq!(response.active_experiments.len(), 1);

        let exp = &response.active_experiments[0];
        assert_eq!(exp.experiment_id, "exp-1");
        assert_eq!(exp.variants.len(), 2);
        assert_eq!(exp.variants[0].id, "baseline");
        assert!(exp.significance.best_variant_id.is_some());
        assert!(exp.significance.p_value.is_some());
    }

    fn efficiency_event(
        plan_id: &str,
        task_id: &str,
        timestamp: &str,
        cost_usd: f64,
        input_tokens: u64,
        output_tokens: u64,
        wall_time_ms: u64,
    ) -> AgentEfficiencyEvent {
        AgentEfficiencyEvent {
            agent_id: "agent-1".into(),
            role: "Implementer".into(),
            backend: "claude".into(),
            model: "claude-sonnet-4-5".into(),
            plan_id: plan_id.into(),
            task_id: task_id.into(),
            input_tokens,
            output_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd,
            cost_usd_without_cache: cost_usd,
            prompt_sections: Vec::new(),
            total_prompt_tokens: input_tokens,
            system_prompt_tokens: 0,
            tools_available: 0,
            tools_used: 0,
            tool_calls: Vec::new(),
            wall_time_ms,
            duration_ms: wall_time_ms,
            time_to_first_token_ms: 0,
            was_warm_start: false,
            iteration: 1,
            gate_passed: true,
            timestamp: timestamp.into(),
        }
    }

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let deploy_backend = Arc::from(
            create_backend("manual", None, None, None).expect("manual backend"),
        );
        let state = Arc::new(AppState::new(
            workdir,
            Config::default(),
            roko_core::config::schema::RokoConfig::default(),
            deploy_backend,
        ));
        (dir, state)
    }

    #[tokio::test]
    async fn experiments_returns_404_when_missing() {
        let (_dir, state) = test_state();

        let err = experiments(State(state))
            .await
            .expect_err("missing experiments should fail");

        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn experiments_returns_500_for_invalid_json() {
        let (dir, state) = test_state();
        let path = dir.path().join(".roko/learn/experiments.json");
        tokio::fs::create_dir_all(path.parent().expect("experiments parent"))
            .await
            .expect("create learn dir");
        tokio::fs::write(&path, "{not-json}")
            .await
            .expect("write corrupt experiments");

        let err = experiments(State(state))
            .await
            .expect_err("corrupt experiments should fail");

        assert_eq!(err.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn efficiency_response_aggregates_task_level_metrics() {
        let events = vec![
            efficiency_event("plan-a", "task-1", "2026-04-08T10:00:00Z", 1.0, 100, 50, 1_000),
            efficiency_event("plan-a", "task-1", "2026-04-08T10:05:00Z", 2.0, 120, 30, 2_000),
            efficiency_event("plan-b", "task-2", "2026-04-08T11:00:00Z", 3.0, 80, 20, 500),
        ];

        let response = build_efficiency_response(&events);

        assert!((response.total_cost - 6.0).abs() < 1e-9);
        assert!((response.cost_per_task - 3.0).abs() < 1e-9);
        assert!((response.tokens_per_task - 200.0).abs() < 1e-9);
        assert!((response.avg_task_duration - 1750.0).abs() < 1e-9);
        assert_eq!(response.cost_trend.len(), 2);
        assert!((response.cost_trend[0].cumulative_cost_usd - 3.0).abs() < 1e-9);
        assert!((response.cost_trend[1].cumulative_cost_usd - 6.0).abs() < 1e-9);
    }

    #[test]
    fn adaptive_thresholds_response_exposes_per_rung_ema_values() {
        let mut thresholds = AdaptiveThresholds::new();
        for _ in 0..5 {
            thresholds.update(2, true);
        }
        for _ in 0..3 {
            thresholds.update(1, false);
        }

        let response = build_adaptive_thresholds_response(
            std::path::Path::new("/tmp/.roko/learn/gate-thresholds.json"),
            &thresholds,
        );

        assert_eq!(response.source, "/tmp/.roko/learn/gate-thresholds.json");
        assert_eq!(response.tracked_rungs, 2);
        assert_eq!(response.rungs.len(), 2);
        assert_eq!(response.rungs[0].rung, 1);
        assert_eq!(response.rungs[0].total_observations, 3);
        assert_eq!(response.rungs[0].consecutive_passes, 0);
        assert_eq!(response.rungs[0].suggested_max_retries, 5);
        assert!((response.rungs[0].ema_pass_rate - 0.0).abs() < 1e-9);
        assert_eq!(response.rungs[1].rung, 2);
        assert_eq!(response.rungs[1].total_observations, 5);
        assert_eq!(response.rungs[1].consecutive_passes, 5);
        assert_eq!(response.rungs[1].suggested_max_retries, 1);
        assert!((response.rungs[1].ema_pass_rate - 1.0).abs() < 1e-9);
        assert!(response.rungs[1].should_skip_rung);
    }
}
