//! Cascade router state, cost-tier, and c-factor trend endpoints.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use chrono::Duration;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ApiError;
use crate::state::AppState;
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_learn::aggregate::{CFactorBucket, cfactor_trend as aggregate_cfactor_trend};
use roko_learn::cascade_router::CascadeStage;
use roko_learn::model_router::COLD_START_THRESHOLD;

/// `GET /api/c-factor/trend` — read `.roko/learn/c-factor.jsonl` and return a trend series.
pub async fn cfactor_trend(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CFactorTrendQuery>,
) -> Result<Json<Vec<CFactorBucket>>, ApiError> {
    let path = state.workdir.join(".roko/learn/c-factor.jsonl");
    if !path.exists() {
        return Ok(Json(Vec::new()));
    }

    let (bucket, n_buckets) = parse_cfactor_trend_window(query.window.as_deref());
    let buckets = aggregate_cfactor_trend(&path, bucket, n_buckets)
        .map_err(|err| ApiError::internal(format!("read {}: {err}", path.display())))?;
    Ok(Json(buckets))
}

/// `GET /api/learning/cascade-router` — read `.roko/learn/cascade-router.json`.
pub async fn cascade_router(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko/learn/cascade-router.json");
    super::helpers::read_json_file(&path).await
}

/// `GET /api/learn/cascade` — summarize `.roko/learn/cascade-router.json`.
pub async fn cascade(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CascadeResponse>, ApiError> {
    let path = state.workdir.join(".roko/learn/cascade-router.json");
    let snapshot = read_cascade_snapshot(&path).await?;
    Ok(Json(build_cascade_response(&path, snapshot)))
}

/// `GET /api/learning/cost-tiers` — summarize T0/T1/T2 routing distribution.
pub async fn cost_tiers(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CostTierResponse>, ApiError> {
    let path = state.workdir.join(".roko/learn/cascade-router.json");
    let snapshot = read_cascade_snapshot(&path).await?;
    Ok(Json(build_cost_tier_response(snapshot)))
}

// ── helpers ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CFactorTrendQuery {
    #[serde(default)]
    window: Option<String>,
}

pub(crate) fn parse_cfactor_trend_window(raw: Option<&str>) -> (Duration, usize) {
    match raw {
        Some("7d") => (Duration::hours(1), 7 * 24),
        _ => (Duration::hours(1), 24),
    }
}

/// Read and parse a cascade snapshot, or return `None` if the file is missing.
pub(crate) async fn read_cascade_snapshot(
    path: &std::path::Path,
) -> Result<Option<CascadeSnapshotData>, ApiError> {
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
                weight: selected
                    .map(|weight| weight.normalized_weight)
                    .unwrap_or(0.0),
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

fn cost_tier_for_model(model: &str) -> &'static str {
    if model.contains("rust") || model.contains("fsm") || model.contains("haiku") {
        "T0"
    } else if model.contains("opus") || model.contains("premium") {
        "T2"
    } else {
        "T1"
    }
}

fn build_cost_tier_response(snapshot: Option<CascadeSnapshotData>) -> CostTierResponse {
    let snapshot = snapshot.unwrap_or_default();
    let mut t0 = 0_u64;
    let mut t1 = 0_u64;
    let mut t2 = 0_u64;

    for (model, stats) in snapshot.confidence_stats {
        match cost_tier_for_model(&model) {
            "T0" => t0 += stats.trials,
            "T1" => t1 += stats.trials,
            _ => t2 += stats.trials,
        }
    }

    let total = t0 + t1 + t2;
    let denom = (total as f64).max(f64::EPSILON);

    CostTierResponse {
        t0,
        t1,
        t2,
        total,
        sample_count: total,
        t0_pct: if total == 0 {
            0.0
        } else {
            (t0 as f64 / denom) * 100.0
        },
        t1_pct: if total == 0 {
            0.0
        } else {
            (t1 as f64 / denom) * 100.0
        },
        t2_pct: if total == 0 {
            0.0
        } else {
            (t2 as f64 / denom) * 100.0
        },
    }
}

// ── types ────────────────────────────────────────────────────────────

/// Parsed cascade router snapshot matching the persisted JSON format.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct CascadeSnapshotData {
    #[serde(default)]
    pub model_slugs: Vec<String>,
    #[serde(default)]
    pub confidence_stats: HashMap<String, PersistedModelStatsData>,
}

/// Per-model confidence stats from the cascade router JSON.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct PersistedModelStatsData {
    #[serde(default)]
    pub trials: u64,
    #[serde(default)]
    pub successes: u64,
}

/// Structured API response for `GET /api/learn/cascade`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CascadeResponse {
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

/// Structured API response for `GET /api/learning/cost-tiers`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CostTierResponse {
    #[serde(rename = "T0")]
    t0: u64,
    #[serde(rename = "T1")]
    t1: u64,
    #[serde(rename = "T2")]
    t2: u64,
    total: u64,
    sample_count: u64,
    t0_pct: f64,
    t1_pct: f64,
    t2_pct: f64,
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
