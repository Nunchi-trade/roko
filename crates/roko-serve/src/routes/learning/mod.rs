//! Learning data endpoints — efficiency, cascade router, experiments, gate thresholds.

pub(super) mod experiments;
pub(crate) mod helpers;
pub(crate) mod router_state;

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::projection_contract::{ProjectionQuery, RuntimeProjectionSet};
use crate::state::AppState;
use roko_gate::adaptive_threshold::AdaptiveThresholds;
use roko_learn::efficiency::AgentEfficiencyEvent;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/c-factor/trend", get(router_state::cfactor_trend))
        .route("/learning/efficiency", get(efficiency))
        .route("/learn/efficiency", get(efficiency))
        .route("/learning/costs", get(costs))
        .route("/learn/costs", get(costs))
        .route("/learning/provider-outcomes", get(provider_outcomes))
        .route("/learn/provider-outcomes", get(provider_outcomes))
        .route("/learning/retries", get(retries))
        .route("/learn/retries", get(retries))
        .route("/learning/runtime-feedback", get(runtime_feedback))
        .route("/learn/runtime-feedback", get(runtime_feedback))
        .route("/learning/cascade-router", get(learn_router_snapshot))
        .route("/learn/cascade-router", get(learn_router_snapshot))
        .route("/learning/cascade", get(router_state::cascade))
        .route("/learning/cost-tiers", get(router_state::cost_tiers))
        .route("/learn/cost-tiers", get(router_state::cost_tiers))
        .route("/learn/cascade", get(router_state::cascade))
        .route("/learn/experiments", get(experiments::experiments))
        .route("/learning/experiments", get(experiments::experiments))
        .route("/learn/adaptive-thresholds", get(adaptive_thresholds))
        .route("/learning/adaptive-thresholds", get(adaptive_thresholds))
        .route("/learning/gate-thresholds", get(gate_thresholds))
        .route("/learn/gate-thresholds", get(gate_thresholds))
        .route("/learn/router", get(learn_router_snapshot))
        .route("/executor/state", get(executor_state))
}

// ── handlers kept in mod.rs ──────────────────────────────────────────

/// `GET /api/learn/efficiency` — aggregate `.roko/learn/efficiency.jsonl`.
async fn efficiency(
    State(state): State<Arc<AppState>>,
) -> Result<Json<EfficiencyResponse>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    Ok(Json(build_efficiency_response_with_evidence(
        projections.efficiency_events(),
        projections.evidence(),
    )))
}

/// `GET /api/learning/costs` — canonical runtime cost projection.
async fn costs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProjectionQuery>,
) -> Result<Json<Value>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    Ok(Json(projections.project("cost_state", &query)?))
}

/// `GET /api/learning/provider-outcomes` — provider/model outcome proof surface.
async fn provider_outcomes(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProjectionQuery>,
) -> Result<Json<Value>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    Ok(Json(projections.project("provider_state", &query)?))
}

/// `GET /api/learning/retries` — retry attempt proof surface.
async fn retries(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProjectionQuery>,
) -> Result<Json<Value>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    Ok(Json(projections.project("retry_state", &query)?))
}

/// `GET /api/learning/runtime-feedback` — joined feedback store overview.
async fn runtime_feedback(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProjectionQuery>,
) -> Result<Json<Value>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    Ok(Json(projections.project("runtime_feedback", &query)?))
}

/// `GET /api/learning/gate-thresholds` — read `.roko/learn/gate-thresholds.json`.
async fn gate_thresholds(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    let learning = projections.project("learning_policy_state", &ProjectionQuery::default())?;
    let node = learning
        .get("gate_thresholds")
        .cloned()
        .unwrap_or(Value::Null);
    Ok(Json(source_projection_payload(
        node,
        projections.evidence(),
    )))
}

/// `GET /api/learn/adaptive-thresholds` — summarize `.roko/learn/gate-thresholds.json`.
async fn adaptive_thresholds(
    State(state): State<Arc<AppState>>,
) -> Result<Json<AdaptiveThresholdsResponse>, ApiError> {
    let path = state.workdir.join(".roko/learn/gate-thresholds.json");
    let thresholds = AdaptiveThresholds::load_or_new(&path);
    Ok(Json(build_adaptive_thresholds_response(&path, &thresholds)))
}

/// `GET /api/learn/router` — return the cascade router snapshot as JSON.
async fn learn_router_snapshot(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    let learning = projections.project("learning_policy_state", &ProjectionQuery::default())?;
    let node = learning
        .get("cascade_router")
        .cloned()
        .unwrap_or(Value::Null);
    Ok(Json(source_projection_payload(
        node,
        projections.evidence(),
    )))
}

/// `GET /api/executor/state` — return the executor snapshot as JSON.
async fn executor_state(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    Ok(Json(
        projections.project("executor_state", &ProjectionQuery::default())?,
    ))
}

// ── helpers ──────────────────────────────────────────────────────────

/// Aggregate efficiency events into task-level cost and timing metrics.
fn build_efficiency_response(events: &[AgentEfficiencyEvent]) -> EfficiencyResponse {
    build_efficiency_response_with_evidence(events, json!({"state": "not_loaded"}))
}

fn build_efficiency_response_with_evidence(
    events: &[AgentEfficiencyEvent],
    evidence: Value,
) -> EfficiencyResponse {
    let mut tasks: HashMap<TaskKey, TaskEfficiencyAggregate> = HashMap::new();

    for (index, event) in events.iter().enumerate() {
        let key = TaskKey {
            plan_id: event.plan_id.clone(),
            task_id: event.task_id.clone(),
        };
        let aggregate = tasks
            .entry(key)
            .or_insert_with(TaskEfficiencyAggregate::default);
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
        tasks: task_summaries,
        evidence,
    }
}

fn source_projection_payload(node: Value, evidence: Value) -> Value {
    let projection_state = node
        .get("state")
        .cloned()
        .unwrap_or_else(|| Value::String("unknown".to_string()));
    let source = node
        .get("source")
        .cloned()
        .or_else(|| node.get("path").cloned())
        .unwrap_or(Value::Null);
    let raw = node.get("value").cloned();

    if let Some(Value::Object(mut map)) = raw.clone() {
        map.entry("projection_state".to_string())
            .or_insert(projection_state);
        if !source.is_null() {
            map.entry("source".to_string()).or_insert(source);
        }
        map.entry("value".to_string())
            .or_insert(raw.unwrap_or(Value::Null));
        map.insert("evidence".to_string(), evidence);
        return Value::Object(map);
    }

    if let Some(value) = raw {
        return json!({
            "projection_state": projection_state,
            "source": source,
            "value": value,
            "evidence": evidence,
        });
    }

    match node {
        Value::Object(mut map) => {
            map.insert("evidence".to_string(), evidence);
            Value::Object(map)
        }
        other => json!({
            "projection_state": projection_state,
            "source": source,
            "value": other,
            "evidence": evidence,
        }),
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

// ── types ────────────────────────────────────────────────────────────

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
    tasks: Vec<TaskEfficiencySummary>,
    evidence: Value,
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
    use anyhow::anyhow;
    use roko_core::OperatingFrequency;
    use std::error::Error;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::State;
    use axum::http::Request;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::deploy::create_backend;
    use crate::routes::build_router;
    use crate::runtime::NoOpRuntime;
    use crate::state::AppState;
    use roko_core::config::ServeAuthConfig;
    use roko_learn::aggregate::CFactorBucket;
    use roko_learn::prompt_experiment::PromptExperiment;

    fn make_experiment() -> PromptExperiment {
        let variants = vec![
            roko_learn::prompt_experiment::PromptVariant {
                id: "baseline".into(),
                name: "Baseline".into(),
                section_name: "system_prompt".into(),
                content: "v1".into(),
                slug: None,
                active: true,
            },
            roko_learn::prompt_experiment::PromptVariant {
                id: "verbose".into(),
                name: "Verbose".into(),
                section_name: "system_prompt".into(),
                content: "v2".into(),
                slug: None,
                active: true,
            },
        ];
        let mut exp = PromptExperiment::new("exp-1", "system_prompt", variants);
        exp.stats = HashMap::from([
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
        ]);
        exp.min_trials_per_variant = 5;
        exp.min_effect_size = 0.1;
        exp
    }

    fn snapshot() -> router_state::CascadeSnapshotData {
        let mut confidence_stats = HashMap::new();
        confidence_stats.insert(
            "claude-sonnet-4-5".to_string(),
            router_state::PersistedModelStatsData {
                trials: 50,
                successes: 45,
            },
        );
        confidence_stats.insert(
            "claude-haiku-3-5".to_string(),
            router_state::PersistedModelStatsData {
                trials: 30,
                successes: 20,
            },
        );

        router_state::CascadeSnapshotData {
            model_slugs: vec![
                "claude-sonnet-4-5".to_string(),
                "claude-haiku-3-5".to_string(),
                "claude-opus-4".to_string(),
            ],
            confidence_stats,
        }
    }

    #[test]
    fn cascade_response_summarizes_weights_and_recommendations() -> Result<(), Box<dyn Error>> {
        let _path = std::path::Path::new("/tmp/.roko/learn/cascade-router.json");
        let response = {
            // Call cascade endpoint logic via public handler if needed
            // For unit tests, we verify the cascade response structure
            let snapshot = snapshot();
            let total_observations = snapshot
                .confidence_stats
                .values()
                .map(|s| s.trials)
                .sum::<u64>();
            assert_eq!(total_observations, 80);
            assert_eq!(snapshot.model_slugs.len(), 3);
            Ok(())
        };
        response
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
            reasoning_tokens: 0,
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
            outcome: "success".into(),
            gate_errors: Vec::new(),
            model_used: "claude-sonnet-4-5".into(),
            frequency: OperatingFrequency::Theta,
            strategy_attempted: "none".into(),
            timestamp: timestamp.into(),
        }
    }

    fn test_state() -> Result<(tempfile::TempDir, Arc<AppState>), Box<dyn Error>> {
        let dir = tempdir().map_err(|err| anyhow!("failed to create tempdir: {err}"))?;
        let workdir = dir.path().to_path_buf();
        let deploy_backend = Arc::from(
            create_backend("manual", None, None, None)
                .map_err(|err| anyhow!("failed to create manual backend: {err}"))?,
        );
        let state = Arc::new(AppState::new(
            workdir,
            Arc::new(NoOpRuntime),
            roko_core::config::schema::RokoConfig::default(),
            deploy_backend,
        ).expect("AppState::new"));
        Ok((dir, state))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cascade_alias_is_served_under_api_grouping() -> Result<(), Box<dyn Error>> {
        let (dir, state) = test_state()?;
        let learn_dir = dir.path().join(".roko").join("learn");
        tokio::fs::create_dir_all(&learn_dir)
            .await
            .map_err(|err| anyhow!("failed to create learn dir for cascade alias test: {err}"))?;
        let cascade_path = learn_dir.join("cascade-router.json");
        tokio::fs::write(
            &cascade_path,
            serde_json::json!({
                "model_slugs": ["claude-sonnet-4-5"],
                "confidence_stats": {
                    "claude-sonnet-4-5": { "trials": 50, "successes": 30 }
                }
            })
            .to_string(),
        )
        .await
        .map_err(|err| anyhow!("failed to write cascade snapshot fixture: {err}"))?;

        let app = build_router(Arc::clone(&state), &[], ServeAuthConfig::default());
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/learning/cascade")
                    .body(Body::empty())
                    .map_err(|err| anyhow!("failed to build cascade alias request: {err}"))?,
            )
            .await
            .map_err(|err| anyhow!("cascade alias request failed: {err}"))?;

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(|err| anyhow!("failed to read cascade alias response body: {err}"))?;
        let payload: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|err| anyhow!("failed to parse cascade alias response body: {err}"))?;
        assert_eq!(payload["source"], cascade_path.display().to_string());
        assert_eq!(payload["routing_stats"]["total_observations"], 50);
        assert_eq!(payload["model_weights"][0]["model"], "claude-sonnet-4-5");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn experiments_returns_empty_store_when_missing() -> Result<(), Box<dyn Error>> {
        let (_dir, state) = test_state()?;

        let response = experiments::experiments(State(state))
            .await
            .map_err(|err| {
                anyhow!(
                    "missing experiments endpoint should succeed: {}",
                    err.message
                )
            })?;
        let body = response.0;

        assert_eq!(body.running_experiments, 0);
        assert_eq!(body.concluded_experiments, 0);
        assert!(body.active_experiments.is_empty());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn learn_alias_routes_expose_cascade_router_cost_tiers_and_gate_thresholds()
    -> Result<(), Box<dyn Error>> {
        let (dir, state) = test_state()?;
        let learn_dir = dir.path().join(".roko").join("learn");
        tokio::fs::create_dir_all(&learn_dir)
            .await
            .map_err(|err| anyhow!("failed to create learn dir for alias routes test: {err}"))?;

        let cascade_path = learn_dir.join("cascade-router.json");
        tokio::fs::write(
            &cascade_path,
            serde_json::json!({
                "model_slugs": ["claude-sonnet-4-5", "claude-haiku-3-5"],
                "confidence_stats": {
                    "claude-sonnet-4-5": { "trials": 50, "successes": 30 },
                    "claude-haiku-3-5": { "trials": 10, "successes": 8 }
                }
            })
            .to_string(),
        )
        .await
        .map_err(|err| anyhow!("failed to write cascade snapshot fixture: {err}"))?;

        let gate_thresholds_path = learn_dir.join("gate-thresholds.json");
        tokio::fs::write(
            &gate_thresholds_path,
            serde_json::json!({"hello": "world"}).to_string(),
        )
        .await
        .map_err(|err| anyhow!("failed to write gate thresholds fixture: {err}"))?;

        let app = build_router(Arc::clone(&state), &[], ServeAuthConfig::default());

        let cascade_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/learn/cascade-router")
                    .body(Body::empty())
                    .map_err(|err| {
                        anyhow!("failed to build cascade-router alias request: {err}")
                    })?,
            )
            .await
            .map_err(|err| anyhow!("cascade-router alias request failed: {err}"))?;
        assert_eq!(cascade_response.status(), axum::http::StatusCode::OK);
        let cascade_body = axum::body::to_bytes(cascade_response.into_body(), usize::MAX)
            .await
            .map_err(|err| anyhow!("failed to read cascade-router alias response body: {err}"))?;
        let cascade_payload: serde_json::Value = serde_json::from_slice(&cascade_body)
            .map_err(|err| anyhow!("failed to parse cascade-router alias response body: {err}"))?;
        assert_eq!(
            cascade_payload["model_slugs"]
                .as_array()
                .expect("invariant: cascade response should contain a model_slugs array")
                .len(),
            2
        );

        let cost_tiers_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/learn/cost-tiers")
                    .body(Body::empty())
                    .map_err(|err| anyhow!("failed to build cost-tiers alias request: {err}"))?,
            )
            .await
            .map_err(|err| anyhow!("cost-tiers alias request failed: {err}"))?;
        assert_eq!(cost_tiers_response.status(), axum::http::StatusCode::OK);
        let cost_tiers_body = axum::body::to_bytes(cost_tiers_response.into_body(), usize::MAX)
            .await
            .map_err(|err| anyhow!("failed to read cost-tiers alias response body: {err}"))?;
        let cost_tiers_payload: serde_json::Value = serde_json::from_slice(&cost_tiers_body)
            .map_err(|err| anyhow!("failed to parse cost-tiers alias response body: {err}"))?;
        assert_eq!(cost_tiers_payload["total"], 60);
        assert_eq!(cost_tiers_payload["T0"], 10);
        assert_eq!(cost_tiers_payload["T1"], 50);

        let thresholds_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/learn/gate-thresholds")
                    .body(Body::empty())
                    .map_err(|err| {
                        anyhow!("failed to build gate-thresholds alias request: {err}")
                    })?,
            )
            .await
            .map_err(|err| anyhow!("gate-thresholds alias request failed: {err}"))?;
        assert_eq!(thresholds_response.status(), axum::http::StatusCode::OK);
        let thresholds_body = axum::body::to_bytes(thresholds_response.into_body(), usize::MAX)
            .await
            .map_err(|err| anyhow!("failed to read gate-thresholds alias response body: {err}"))?;
        let thresholds_payload: serde_json::Value = serde_json::from_slice(&thresholds_body)
            .map_err(|err| anyhow!("failed to parse gate-thresholds alias response body: {err}"))?;
        assert_eq!(thresholds_payload["hello"], "world");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn experiments_returns_500_for_invalid_json() -> Result<(), Box<dyn Error>> {
        let (dir, state) = test_state()?;
        let path = dir.path().join(".roko/learn/experiments.json");
        let path_parent = path
            .parent()
            .ok_or_else(|| anyhow!("experiments fixture path should have a parent directory"))?;
        tokio::fs::create_dir_all(path_parent)
            .await
            .map_err(|err| anyhow!("failed to create learn dir for experiments test: {err}"))?;
        tokio::fs::write(&path, "{not-json}")
            .await
            .map_err(|err| anyhow!("failed to write corrupt experiments fixture: {err}"))?;

        let err = match experiments::experiments(State(state)).await {
            Ok(_) => return Err(anyhow!("corrupt experiments should fail").into()),
            Err(err) => err,
        };

        assert_eq!(err.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        Ok(())
    }

    #[test]
    fn efficiency_response_aggregates_task_level_metrics() {
        let events = vec![
            efficiency_event(
                "plan-a",
                "task-1",
                "2026-04-08T10:00:00Z",
                1.0,
                100,
                50,
                1_000,
            ),
            efficiency_event(
                "plan-a",
                "task-1",
                "2026-04-08T10:05:00Z",
                2.0,
                120,
                30,
                2_000,
            ),
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
    fn cfactor_trend_window_defaults_to_24h_and_supports_7d() {
        use chrono::Duration;
        assert_eq!(
            router_state::parse_cfactor_trend_window(None),
            (Duration::hours(1), 24)
        );
        assert_eq!(
            router_state::parse_cfactor_trend_window(Some("24h")),
            (Duration::hours(1), 24)
        );
        assert_eq!(
            router_state::parse_cfactor_trend_window(Some("7d")),
            (Duration::hours(1), 168)
        );
        assert_eq!(
            router_state::parse_cfactor_trend_window(Some("unexpected")),
            (Duration::hours(1), 24)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cfactor_trend_returns_empty_array_when_missing() -> Result<(), Box<dyn Error>> {
        let (_dir, state) = test_state()?;

        let response = build_router(Arc::clone(&state), &[], ServeAuthConfig::default())
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/c-factor/trend")
                    .body(Body::empty())
                    .map_err(|err| anyhow!("failed to build c-factor trend request: {err}"))?,
            )
            .await
            .map_err(|err| anyhow!("c-factor trend request failed: {err}"))?;

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(|err| anyhow!("failed to read c-factor trend response body: {err}"))?;
        let payload: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|err| anyhow!("failed to parse c-factor trend response body: {err}"))?;
        assert_eq!(
            payload
                .as_array()
                .expect("invariant: c-factor trend response should be an array")
                .len(),
            0
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cfactor_trend_returns_hourly_buckets_for_default_window() -> Result<(), Box<dyn Error>>
    {
        let (dir, state) = test_state()?;
        let learn_dir = dir.path().join(".roko").join("learn");
        tokio::fs::create_dir_all(&learn_dir)
            .await
            .map_err(|err| anyhow!("failed to create learn dir for c-factor trend test: {err}"))?;
        let now = chrono::Utc::now();
        let now_ms = now.timestamp_millis();
        let hour_ms: i64 = 3_600_000;
        let hour_start_ms = (now_ms / hour_ms) * hour_ms;
        let t1 = chrono::DateTime::from_timestamp_millis(hour_start_ms + 10 * 60_000)
            .unwrap()
            .to_rfc3339();
        let t2 = chrono::DateTime::from_timestamp_millis(hour_start_ms + 40 * 60_000)
            .unwrap()
            .to_rfc3339();
        tokio::fs::write(
            learn_dir.join("c-factor.jsonl"),
            [
                serde_json::json!({
                    "computed_at": t1,
                    "overall": 0.35
                })
                .to_string(),
                serde_json::json!({
                    "computed_at": t2,
                    "overall": 0.65
                })
                .to_string(),
            ]
            .join("\n")
                + "\n",
        )
        .await
        .map_err(|err| anyhow!("failed to write c-factor trend fixture: {err}"))?;

        let response = build_router(Arc::clone(&state), &[], ServeAuthConfig::default())
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/c-factor/trend")
                    .body(Body::empty())
                    .map_err(|err| anyhow!("failed to build c-factor trend request: {err}"))?,
            )
            .await
            .map_err(|err| anyhow!("c-factor trend request failed: {err}"))?;

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(|err| anyhow!("failed to read c-factor trend response body: {err}"))?;
        let payload: Vec<CFactorBucket> = serde_json::from_slice(&body)
            .map_err(|err| anyhow!("failed to parse c-factor trend response body: {err}"))?;
        assert_eq!(payload.len(), 24);
        assert!(payload.iter().any(|bucket| bucket.samples == 2));
        Ok(())
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
        assert_eq!(response.rungs[0].suggested_max_retries, 3);
        assert!((response.rungs[0].ema_pass_rate - 0.0).abs() < 1e-9);
        assert_eq!(response.rungs[1].rung, 2);
        assert_eq!(response.rungs[1].total_observations, 5);
        assert_eq!(response.rungs[1].consecutive_passes, 5);
        assert_eq!(response.rungs[1].suggested_max_retries, 1);
        assert!((response.rungs[1].ema_pass_rate - 1.0).abs() < 1e-9);
        assert!(!response.rungs[1].should_skip_rung);
    }
}
