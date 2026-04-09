//! Learning data endpoints — efficiency, cascade router, experiments, gate thresholds.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value;

use crate::serve::error::ApiError;
use crate::serve::state::AppState;
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_learn::cascade_router::CascadeStage;
use roko_learn::model_router::COLD_START_THRESHOLD;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/learning/efficiency", get(efficiency))
        .route("/learning/cascade-router", get(cascade_router))
        .route("/learn/cascade", get(cascade))
        .route("/learning/experiments", get(experiments))
        .route("/learning/gate-thresholds", get(gate_thresholds))
}

/// `GET /api/learning/efficiency` — read `.roko/learn/efficiency.jsonl`.
async fn efficiency(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko/learn/efficiency.jsonl");
    read_jsonl(&path).await
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

/// `GET /api/learning/experiments` — read `.roko/learn/experiments.json`.
async fn experiments(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko/learn/experiments.json");
    read_json_file(&path).await
}

/// `GET /api/learning/gate-thresholds` — read `.roko/learn/gate-thresholds.json`.
async fn gate_thresholds(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko/learn/gate-thresholds.json");
    read_json_file(&path).await
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

/// Read a JSONL file and return entries as a JSON array.
async fn read_jsonl(path: &std::path::Path) -> Result<Json<Value>, ApiError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Json(Value::Array(Vec::new())));
        }
        Err(e) => {
            return Err(ApiError::internal(format!("read {}: {e}", path.display())));
        }
    };
    let entries: Vec<Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    Ok(Json(Value::Array(entries)))
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
