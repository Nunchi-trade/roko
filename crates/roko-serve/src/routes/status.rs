//! Status, health, metrics, dashboard, episodes, signals, and operation endpoints.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

use crate::error::ApiError;
use crate::event_bus::Envelope;
use crate::state::AppState;
use roko_learn::cascade_router::CascadeStage;
use roko_learn::cfactor::{AgentDispatchBias, CFactor, CFactorComponents};
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::efficiency::{FleetCFactor, compute_fleet_cfactor};
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use roko_learn::model_router::COLD_START_THRESHOLD;
use roko_learn::prompt_experiment::{ExperimentStatus, ExperimentStore};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health))
        .route("/status", get(session_status))
        .route("/metrics", get(metrics))
        .route("/metrics/summary", get(metrics_summary))
        .route("/metrics/success_rate", get(success_rate))
        .route("/metrics/engagement", get(engagement))
        .route("/metrics/c_factor", get(c_factor_metrics))
        .route("/metrics/model_efficiency", get(model_efficiency))
        .route("/metrics/gate_rate", get(gate_rate))
        .route("/metrics/experiments", get(experiments_metric))
        .route("/metrics/feedback_latency", get(feedback_latency))
        .route("/metrics/velocity", get(velocity))
        .route("/metrics/coverage", get(coverage))
        .route("/dashboard", get(dashboard))
        .route("/gates/summary", get(gate_summary))
        .route("/gates/history", get(gates_history))
        .route("/gates/{gate_name}/history", get(gate_history))
        .route("/episodes", get(episodes))
        .route("/signals", get(signals))
        .route("/operations/{id}", get(operation_status))
}

/// `GET /api/health` — liveness check.
async fn health(State(state): State<Arc<AppState>>) -> (axum::http::StatusCode, Json<Value>) {
    let uptime_secs = state.started_at.elapsed().as_secs();
    let active_plans = state.active_plans.read().await.len();
    let active_agents = state.supervisor.count().await;

    (
        axum::http::StatusCode::OK,
        Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": uptime_secs,
        "active_plans": active_plans,
        "active_agents": active_agents,
        })),
    )
}

/// `GET /api/status` — session status overview.
async fn session_status(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let ss = state.runtime.session_status(state.workdir.clone());
    Ok(Json(json!({
        "session_id": ss.session_id,
        "workdir": ss.workdir,
        "daemon_running": ss.daemon_running,
        "signal_count": ss.signal_count,
        "episode_count": ss.episode_count,
        "last_episode_passed": ss.last_episode_passed,
    })))
}

/// `GET /api/metrics` — metric snapshots as JSON.
async fn metrics(State(state): State<Arc<AppState>>) -> Json<Value> {
    let snapshots = state.metrics.snapshot();
    Json(serde_json::to_value(snapshots).unwrap_or(json!([])))
}

#[derive(Debug, Deserialize)]
struct MetricsSummaryQuery {
    #[serde(default)]
    period: Option<String>,
}

/// `GET /api/metrics/summary` — aggregate recent execution and learning metrics.
async fn metrics_summary(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MetricsSummaryQuery>,
) -> Result<Json<Value>, ApiError> {
    let summary = build_metrics_summary(&state, query.period.as_deref()).await?;
    Ok(Json(serde_json::to_value(summary).map_err(|e| {
        ApiError::internal(format!("serialize metrics summary: {e}"))
    })?))
}

/// `GET /api/metrics/success_rate` — per template success rate, split by trigger kind.
async fn success_rate(State(state): State<Arc<AppState>>) -> Json<Value> {
    let runs = state.template_runs.read().await;
    Json(build_template_success_rate(&runs))
}

/// `GET /api/metrics/engagement` — feedback acknowledgement ratio per template.
async fn engagement(State(state): State<Arc<AppState>>) -> Json<Value> {
    let runs = state.template_runs.read().await;
    Json(build_template_engagement(&runs))
}

/// `GET /api/metrics/c_factor` — composite C-Factor, component metrics, per-agent, and per-fleet.
async fn c_factor_metrics(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let composite_path = state.workdir.join(".roko/learn/c-factor.jsonl");
    let efficiency_path = state.workdir.join(".roko/learn/efficiency.jsonl");

    let history = read_cfactor_history(&composite_path).await?;
    let events = read_efficiency_events(&efficiency_path).await?;
    let fleet = compute_fleet_cfactor(&events);

    Ok(Json(build_cfactor_metrics_response(
        &composite_path,
        &history,
        &efficiency_path,
        &events,
        fleet,
    )))
}

/// `GET /api/metrics/model_efficiency` — cost per successful episode for each routed model.
async fn model_efficiency(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko/learn/cascade-router.json");
    let snapshot = read_cascade_snapshot(&path).await?;
    let efficiency_path = state.workdir.join(".roko/learn/efficiency.jsonl");
    let events = read_efficiency_events(&efficiency_path).await?;
    Ok(Json(build_model_efficiency_response(
        &path, snapshot, &events,
    )))
}

/// `GET /api/metrics/gate_rate` — passed / total per gate with a trend delta.
async fn gate_rate(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko").join("signals.jsonl");
    let entries = read_jsonl_entries(&path).await?;
    Ok(Json(build_gate_rate_response(&entries)))
}

/// `GET /api/metrics/experiments` — best vs worst variant gap per experiment.
async fn experiments_metric(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko/learn/experiments.json");
    let store = read_experiment_store(&path).await?;
    Ok(Json(build_experiment_metrics_response(&path, &store)))
}

/// `GET /api/metrics/feedback_latency` — median hours from action to first feedback signal.
async fn feedback_latency(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko").join("signals.jsonl");
    let entries = read_jsonl_entries(&path).await?;
    Ok(Json(build_feedback_latency_response(&entries)))
}

/// `GET /api/metrics/velocity` — rate of change of success rate over time.
async fn velocity(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko/learn/efficiency.jsonl");
    let events = read_efficiency_events(&path).await?;
    Ok(Json(json!({
        "velocity": self_improvement_velocity(&events),
        "sample_count": events.len(),
    })))
}

/// `GET /api/metrics/coverage` — percentage of events that matched a known subscription.
async fn coverage(State(state): State<Arc<AppState>>) -> Json<Value> {
    let backlog = state.event_bus.replay_from(0);
    Json(build_coverage_response(&backlog))
}

/// `GET /api/dashboard` — dashboard scaffold as JSON.
async fn dashboard(State(state): State<Arc<AppState>>) -> Json<Value> {
    let info = state.runtime.dashboard_scaffold(&state.workdir);
    Json(json!({ "rendered": info.rendered }))
}

/// `GET /api/episodes` — read episodes JSONL as a JSON array.
async fn episodes(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.layout.episodes_path();
    let entries = read_jsonl_entries(&path).await?;
    let capped: Vec<Value> = entries
        .into_iter()
        .rev()
        .take(MAX_JSONL_RESULTS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Ok(Json(Value::Array(capped)))
}

/// `GET /api/gates/summary` — aggregate gate verdicts from `.roko/signals.jsonl`.
async fn gate_summary(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko").join("signals.jsonl");
    let entries = read_jsonl_entries(&path).await?;
    let mut summary = summarize_gate_entries(&entries);
    if let Some(obj) = summary.as_object_mut() {
        obj.insert("rungs".to_string(), json!(summarize_gate_rungs(&entries)));
    }
    Ok(Json(summary))
}

#[derive(Debug, Deserialize, Default)]
struct GateHistoryQuery {
    #[serde(default)]
    gate: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// `GET /api/gates/history` — recent gate verdicts across all gates.
async fn gates_history(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GateHistoryQuery>,
) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko").join("signals.jsonl");
    let entries = read_jsonl_entries(&path).await?;
    let gate_filter = query.gate.as_deref();
    let limit = query.limit.unwrap_or(100).min(MAX_JSONL_RESULTS);
    let mut history = build_recent_gate_history(&entries, gate_filter);
    let total = history.len();
    history.truncate(limit);

    Ok(Json(json!({
        "source": path.display().to_string(),
        "gate": gate_filter,
        "limit": limit,
        "total": total,
        "history": history,
    })))
}

/// `GET /api/gates/:gate_name/history` — time series of pass/fail results for one gate.
async fn gate_history(
    State(state): State<Arc<AppState>>,
    Path(gate_name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko").join("signals.jsonl");
    let entries = read_jsonl_entries(&path).await?;
    let mut history: Vec<Value> = entries
        .into_iter()
        .filter(|entry| extract_gate_name(entry).as_deref() == Some(gate_name.as_str()))
        .filter_map(|entry| {
            let passed = extract_gate_passed(&entry)?;
            Some(json!({
                "signal_id": entry.get("id").cloned().unwrap_or(Value::Null),
                "created_at_ms": entry.get("created_at_ms").cloned().unwrap_or(Value::Null),
                "gate": gate_name,
                "passed": passed,
                "duration_ms": extract_gate_duration_ms(&entry).unwrap_or(0),
                "plan_id": entry.pointer("/tags/plan_id").cloned().or_else(|| entry.pointer("/body/data/plan_id").cloned()).unwrap_or(Value::Null),
                "task_id": entry.pointer("/tags/task_id").cloned().or_else(|| entry.pointer("/body/data/task_id").cloned()).unwrap_or(Value::Null),
                "rung": entry.pointer("/tags/rung").cloned().or_else(|| entry.pointer("/body/data/rung").cloned()).unwrap_or(Value::Null),
            }))
        })
        .collect();

    if history.is_empty() {
        return Err(ApiError::not_found(format!("gate '{gate_name}' not found")));
    }

    history.sort_by(|a, b| {
        let a_ts = a
            .get("created_at_ms")
            .and_then(Value::as_i64)
            .unwrap_or(i64::MIN);
        let b_ts = b
            .get("created_at_ms")
            .and_then(Value::as_i64)
            .unwrap_or(i64::MIN);
        a_ts.cmp(&b_ts).then_with(|| {
            let a_id = a.get("signal_id").and_then(Value::as_str).unwrap_or("");
            let b_id = b.get("signal_id").and_then(Value::as_str).unwrap_or("");
            a_id.cmp(b_id)
        })
    });

    Ok(Json(json!({
        "gate": gate_name,
        "history": history,
    })))
}

const MAX_JSONL_RESULTS: usize = 10_000;

#[derive(Deserialize)]
struct SignalQuery {
    limit: Option<usize>,
}

/// `GET /api/signals` — read signals JSONL as a JSON array, with optional `?limit=N`.
async fn signals(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SignalQuery>,
) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko").join("signals.jsonl");
    let entries = read_jsonl_entries(&path).await?;
    let cap = q.limit.unwrap_or(MAX_JSONL_RESULTS).min(MAX_JSONL_RESULTS);
    let limited: Vec<Value> = entries
        .into_iter()
        .rev()
        .take(cap)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Ok(Json(Value::Array(limited)))
}

/// `GET /api/operations/:id` — look up a background operation by ID.
async fn operation_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let ops = state.operations.read().await;
    let handle = ops
        .get(&id)
        .ok_or_else(|| ApiError::not_found("operation not found"))?;
    let result = Json(json!({
        "id": id,
        "kind": handle.kind,
        "status": format!("{:?}", handle.status),
    }));
    drop(ops);
    Ok(result)
}

// ── helpers ──────────────────────────────────────────────────────────

/// Read a JSONL file and return each line as a parsed `serde_json::Value`.
async fn read_jsonl_entries(path: &std::path::Path) -> Result<Vec<Value>, ApiError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(ApiError::internal(format!("read {}: {e}", path.display()))),
    };
    let mut entries = Vec::new();
    for (line_no, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry = serde_json::from_str::<Value>(line).map_err(|e| {
            ApiError::internal(format!(
                "parse {} line {}: {e}",
                path.display(),
                line_no + 1
            ))
        })?;
        entries.push(entry);
    }
    Ok(entries)
}

/// Read a JSONL file and return the entries as a `Json<Value::Array>`.
async fn read_jsonl_array(path: &std::path::Path) -> Result<Json<Value>, ApiError> {
    let entries = read_jsonl_entries(path).await?;
    Ok(Json(Value::Array(entries)))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct MetricsSummaryResponse {
    period: String,
    agents_run: u64,
    success_rate: f64,
    feedback_engagement_rate: f64,
    avg_cost_per_episode_cents: u64,
    experiments_active: usize,
    best_experiment_lift: Option<ExperimentLiftSummary>,
    gate_pass_rate: f64,
    self_improvement_velocity: f64,
    c_factor: f64,
    active_plans: usize,
    top_templates: Vec<TemplateSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct ExperimentLiftSummary {
    name: String,
    lift: f64,
    winning: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct TemplateSummary {
    name: String,
    runs: u64,
    success_rate: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct CFactorMetricsResponse {
    source: CFactorMetricsSource,
    composite: CFactorCompositeSummary,
    sub_metrics: CFactorComponents,
    per_agent: Vec<CFactorAgentSummary>,
    per_fleet: FleetCFactor,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct CFactorMetricsSource {
    composite_history_path: String,
    efficiency_events_path: String,
    composite_history_count: usize,
    efficiency_event_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct CFactorCompositeSummary {
    overall: f64,
    computed_at: chrono::DateTime<chrono::Utc>,
    episode_count: usize,
    history_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct CFactorAgentSummary {
    agent_id: String,
    episode_count: usize,
    without_agent_overall: f64,
    contribution_score: f64,
    dispatch_bias: String,
}

#[derive(Debug, Default)]
struct RunAggregate {
    runs: u64,
    successes: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CascadeSnapshotData {
    #[serde(default)]
    model_slugs: Vec<String>,
    #[serde(default)]
    confidence_stats: HashMap<String, PersistedModelStatsData>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PersistedModelStatsData {
    #[serde(default)]
    trials: u64,
}

#[derive(Debug, Default)]
struct ModelEfficiencyAggregate {
    total_cost_usd: f64,
    total_episodes: u64,
    successful_episodes: u64,
}

#[derive(Debug, Default)]
struct GateRateAggregate {
    passed_gates: u64,
    total_gates: u64,
    samples: Vec<(i64, bool)>,
}

#[derive(Debug, Clone, Default)]
struct SignalNode {
    kind: String,
    created_at_ms: i64,
    lineage: Vec<String>,
}

async fn build_metrics_summary(
    state: &AppState,
    period: Option<&str>,
) -> Result<MetricsSummaryResponse, ApiError> {
    let (period_label, window_days) = parse_period(period);
    let window_start = Utc::now() - Duration::days(i64::try_from(window_days).unwrap_or(7));

    let efficiency_path = state.workdir.join(".roko/learn/efficiency.jsonl");
    let efficiency_events = read_efficiency_events(&efficiency_path).await?;
    let efficiency_events: Vec<AgentEfficiencyEvent> = efficiency_events
        .into_iter()
        .filter(|event| event_time(event).is_some_and(|ts| ts >= window_start))
        .collect();

    let episodes_path = state.layout.episodes_path();
    let episodes = EpisodeLogger::read_all_lossy(&episodes_path)
        .await
        .map_err(|e| ApiError::internal(format!("read {}: {e}", episodes_path.display())))?;
    let episodes: Vec<Episode> = episodes
        .into_iter()
        .filter(|episode| episode.timestamp >= window_start)
        .collect();

    let experiment_path = state.workdir.join(".roko/learn/experiments.json");
    let experiments = read_experiment_store(&experiment_path).await?;
    let c_factor_path = state.workdir.join(".roko/learn/c-factor.jsonl");
    let c_factor_history = read_cfactor_history(&c_factor_path).await?;
    let c_factor = c_factor_history
        .last()
        .map(|snapshot| snapshot.overall)
        .unwrap_or(0.0);
    let active_plans = state.active_plans.read().await.len();

    let agents_run = efficiency_events.len() as u64;
    let success_count = efficiency_events
        .iter()
        .filter(|event| event.gate_passed)
        .count() as u64;
    let success_rate = ratio(success_count, agents_run);
    let avg_cost_per_episode_cents = if agents_run == 0 {
        0
    } else {
        let total_cost_usd: f64 = efficiency_events.iter().map(|event| event.cost_usd).sum();
        ((total_cost_usd / agents_run as f64) * 100.0)
            .round()
            .max(0.0) as u64
    };

    let feedback_engagement_rate = feedback_engagement_rate(&episodes);
    let gate_pass_rate = gate_pass_rate(&episodes, &efficiency_events);
    let self_improvement_velocity = self_improvement_velocity(&efficiency_events);
    let experiments_active = experiments.running_count();
    let best_experiment_lift = best_experiment_lift(&experiments);
    let top_templates = top_templates(state, window_start).await;

    Ok(MetricsSummaryResponse {
        period: period_label,
        agents_run,
        success_rate,
        feedback_engagement_rate,
        avg_cost_per_episode_cents,
        experiments_active,
        best_experiment_lift,
        gate_pass_rate,
        self_improvement_velocity,
        c_factor,
        active_plans,
        top_templates,
    })
}

fn build_template_success_rate(
    runs: &HashMap<String, Vec<crate::state::TemplateRunRecord>>,
) -> Value {
    let mut templates = Vec::new();

    for (template_name, records) in runs {
        let mut by_trigger: BTreeMap<String, RunAggregate> = BTreeMap::new();
        for record in records {
            let trigger = if record.trigger_kind.trim().is_empty() {
                "unknown".to_string()
            } else {
                record.trigger_kind.clone()
            };
            let aggregate = by_trigger.entry(trigger).or_default();
            aggregate.runs += 1;
            if record.success {
                aggregate.successes += 1;
            }
        }

        let triggers = by_trigger
            .into_iter()
            .map(|(trigger_kind, aggregate)| {
                json!({
                    "trigger_kind": trigger_kind,
                    "successful_episodes": aggregate.successes,
                    "total_episodes": aggregate.runs,
                    "success_rate": ratio(aggregate.successes, aggregate.runs),
                })
            })
            .collect::<Vec<_>>();

        templates.push(json!({
            "template": template_name,
            "triggers": triggers,
        }));
    }

    templates.sort_by(|a, b| {
        a.get("template")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("template").and_then(Value::as_str).unwrap_or(""))
    });

    json!({ "templates": templates })
}

fn build_template_engagement(
    runs: &HashMap<String, Vec<crate::state::TemplateRunRecord>>,
) -> Value {
    let mut templates = Vec::new();

    for (template_name, records) in runs {
        let total_actions = records.len() as u64;
        let acknowledged_actions = records.iter().filter(|record| record.success).count() as u64;
        templates.push(json!({
            "template": template_name,
            "acknowledged_actions": acknowledged_actions,
            "total_actions": total_actions,
            "engagement_rate": ratio(acknowledged_actions, total_actions),
        }));
    }

    templates.sort_by(|a, b| {
        a.get("template")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("template").and_then(Value::as_str).unwrap_or(""))
    });

    json!({ "templates": templates })
}

fn build_model_efficiency_response(
    path: &std::path::Path,
    snapshot: Option<CascadeSnapshotData>,
    events: &[AgentEfficiencyEvent],
) -> Value {
    let total_observations = snapshot
        .as_ref()
        .map(|snap| {
            snap.confidence_stats
                .values()
                .map(|stats| stats.trials)
                .sum::<u64>()
        })
        .unwrap_or(0);
    let current_stage = cascade_stage_for_observations(total_observations).label();

    let mut models: BTreeMap<String, ModelEfficiencyAggregate> = BTreeMap::new();
    if let Some(snapshot) = &snapshot {
        for slug in &snapshot.model_slugs {
            models.entry(slug.clone()).or_default();
        }
        for slug in snapshot.confidence_stats.keys() {
            models.entry(slug.clone()).or_default();
        }
    }

    for event in events {
        let aggregate = models.entry(event.model.clone()).or_default();
        aggregate.total_cost_usd += event.cost_usd;
        aggregate.total_episodes += 1;
        if event.gate_passed {
            aggregate.successful_episodes += 1;
        }
    }

    let mut rows: Vec<Value> = models
        .into_iter()
        .map(|(model, aggregate)| {
            let cost_per_success = if aggregate.successful_episodes == 0 {
                0.0
            } else {
                aggregate.total_cost_usd / aggregate.successful_episodes as f64
            };
            json!({
                "model": model,
                "total_episodes": aggregate.total_episodes,
                "successful_episodes": aggregate.successful_episodes,
                "total_cost_usd": aggregate.total_cost_usd,
                "cost_per_successful_episode_usd": cost_per_success,
                "success_rate": ratio(aggregate.successful_episodes, aggregate.total_episodes),
            })
        })
        .collect();

    rows.sort_by(|a, b| {
        a.get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("model").and_then(Value::as_str).unwrap_or(""))
    });

    json!({
        "source": path.display().to_string(),
        "current_stage": current_stage,
        "total_observations": total_observations,
        "models": rows,
    })
}

fn build_gate_rate_response(entries: &[Value]) -> Value {
    let mut by_gate: BTreeMap<String, GateRateAggregate> = BTreeMap::new();

    for entry in entries {
        let Some(kind) = entry.get("kind").and_then(Value::as_str) else {
            continue;
        };
        if !is_gate_result_kind(kind) {
            continue;
        }

        let Some(gate_name) = extract_gate_name(entry) else {
            continue;
        };
        let Some(passed) = extract_gate_passed(entry) else {
            continue;
        };
        let timestamp = entry_timestamp_ms(entry).unwrap_or_default();

        let aggregate = by_gate.entry(gate_name).or_default();
        aggregate.total_gates += 1;
        if passed {
            aggregate.passed_gates += 1;
        }
        aggregate.samples.push((timestamp, passed));
    }

    let mut gates = Vec::new();
    for (gate, aggregate) in by_gate {
        let (trend_delta, trend_direction, baseline_rate, recent_rate) =
            gate_trend(&aggregate.samples);
        gates.push(json!({
            "gate": gate,
            "passed_gates": aggregate.passed_gates,
            "total_gates": aggregate.total_gates,
            "gate_rate": ratio(aggregate.passed_gates, aggregate.total_gates),
            "trend": {
                "delta": trend_delta,
                "direction": trend_direction,
                "baseline_rate": baseline_rate,
                "recent_rate": recent_rate,
            },
        }));
    }

    gates.sort_by(|a, b| {
        a.get("gate")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("gate").and_then(Value::as_str).unwrap_or(""))
    });

    json!({ "gates": gates })
}

fn build_experiment_metrics_response(path: &std::path::Path, store: &ExperimentStore) -> Value {
    let mut experiments = Vec::new();

    for experiment in store.iter() {
        let mut variants: Vec<_> = experiment
            .variants
            .iter()
            .filter(|variant| variant.active)
            .collect();
        if variants.len() < 2 {
            variants = experiment.variants.iter().collect();
        }

        let mut ranked: Vec<Value> = variants
            .iter()
            .map(|variant| {
                let stats = experiment
                    .stats
                    .get(&variant.id)
                    .cloned()
                    .unwrap_or_default();
                json!({
                    "id": variant.id,
                    "name": variant.name,
                    "section_name": variant.section_name,
                    "active": variant.active,
                    "trials": stats.trials,
                    "successes": stats.successes,
                    "success_rate": stats.success_rate(),
                })
            })
            .collect();

        ranked.sort_by(|a, b| {
            b.get("success_rate")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .partial_cmp(&a.get("success_rate").and_then(Value::as_f64).unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    b.get("trials")
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                        .cmp(&a.get("trials").and_then(Value::as_u64).unwrap_or(0))
                })
                .then_with(|| {
                    a.get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .cmp(b.get("id").and_then(Value::as_str).unwrap_or(""))
                })
        });

        let best = ranked.first().cloned();
        let worst = ranked.last().cloned();
        let difference = match (
            best.as_ref()
                .and_then(|v| v.get("success_rate"))
                .and_then(Value::as_f64),
            worst
                .as_ref()
                .and_then(|v| v.get("success_rate"))
                .and_then(Value::as_f64),
        ) {
            (Some(best_rate), Some(worst_rate)) if ranked.len() >= 2 => {
                Some(best_rate - worst_rate)
            }
            _ => None,
        };

        experiments.push(json!({
            "experiment_id": experiment.experiment_id,
            "section_name": experiment.section_name,
            "status": experiment.status,
            "best_variant": best,
            "worst_variant": worst,
            "metric_difference": difference,
        }));
    }

    experiments.sort_by(|a, b| {
        a.get("experiment_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("experiment_id").and_then(Value::as_str).unwrap_or(""))
    });

    json!({
        "source": path.display().to_string(),
        "experiments": experiments,
    })
}

fn build_feedback_latency_response(entries: &[Value]) -> Value {
    let index = build_signal_index(entries);
    let mut latencies_hours = Vec::new();

    for entry in entries {
        if signal_kind(entry).as_deref() != Some("gate_verdict") {
            continue;
        }
        let Some(gate_ts) = entry_timestamp_ms(entry) else {
            continue;
        };
        let Some(signal_id) = signal_id(entry) else {
            continue;
        };
        let Some(action_ts) =
            ancestor_timestamp(&index, &signal_id, &["agent_output", "agent_message"])
        else {
            continue;
        };
        if gate_ts < action_ts {
            continue;
        }
        latencies_hours.push((gate_ts - action_ts) as f64 / 3_600_000.0);
    }

    latencies_hours.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_hours = median(&latencies_hours);

    json!({
        "sample_count": latencies_hours.len(),
        "median_hours": median_hours,
    })
}

fn build_coverage_response(backlog: &[Envelope<crate::events::ServerEvent>]) -> Value {
    let subscription_terms = [
        "plan",
        "task",
        "gate",
        "execution",
        "episode",
        "efficiency",
        "run",
        "operation",
        "deployment",
        "error",
        "server_shutdown",
        "agent",
    ];

    let mut matched = 0u64;
    let mut unhandled = 0u64;

    for envelope in backlog {
        let Ok(value) = serde_json::to_value(&envelope.payload) else {
            unhandled += 1;
            continue;
        };

        let mut event_types = Vec::new();
        if let Some(event_type) = value.get("type").and_then(Value::as_str) {
            event_types.push(event_type);
        }
        if event_types.contains(&"execution") {
            if let Some(exec_type) = value
                .get("event")
                .and_then(|event| event.get("type"))
                .and_then(Value::as_str)
            {
                event_types.push(exec_type);
            }
        }

        if event_types.iter().any(|event_type| {
            subscription_terms
                .iter()
                .any(|term| event_type.contains(term))
        }) {
            matched += 1;
        } else {
            unhandled += 1;
        }
    }

    let total = matched + unhandled;
    json!({
        "matched_events": matched,
        "unhandled_events": unhandled,
        "coverage": ratio(matched, total),
        "subscription_terms": subscription_terms,
    })
}

fn build_signal_index(entries: &[Value]) -> HashMap<String, SignalNode> {
    let mut index = HashMap::new();

    for entry in entries {
        let Some(id) = signal_id(entry) else {
            continue;
        };
        let kind = signal_kind(entry).unwrap_or_else(|| "unknown".to_string());
        let created_at_ms = entry_timestamp_ms(entry).unwrap_or_default();
        let lineage = entry
            .get("lineage")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        index.insert(
            id,
            SignalNode {
                kind,
                created_at_ms,
                lineage,
            },
        );
    }

    index
}

fn ancestor_timestamp(
    index: &HashMap<String, SignalNode>,
    signal_id: &str,
    desired_kinds: &[&str],
) -> Option<i64> {
    let mut visited = std::collections::HashSet::new();
    ancestor_timestamp_inner(index, signal_id, desired_kinds, &mut visited)
}

fn ancestor_timestamp_inner(
    index: &HashMap<String, SignalNode>,
    signal_id: &str,
    desired_kinds: &[&str],
    visited: &mut std::collections::HashSet<String>,
) -> Option<i64> {
    let node = index.get(signal_id)?;
    if desired_kinds.iter().any(|kind| *kind == node.kind) {
        return Some(node.created_at_ms);
    }

    for parent in &node.lineage {
        if !visited.insert(parent.clone()) {
            continue;
        }
        if let Some(ts) = ancestor_timestamp_inner(index, parent, desired_kinds, visited) {
            return Some(ts);
        }
    }

    None
}

fn signal_kind(entry: &Value) -> Option<String> {
    entry
        .get("kind")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn signal_id(entry: &Value) -> Option<String> {
    entry
        .get("id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn entry_timestamp_ms(entry: &Value) -> Option<i64> {
    entry
        .get("created_at_ms")
        .and_then(Value::as_i64)
        .or_else(|| {
            entry
                .get("created_at_ms")
                .and_then(Value::as_u64)
                .map(|ts| ts as i64)
        })
}

fn gate_trend(samples: &[(i64, bool)]) -> (f64, String, f64, f64) {
    if samples.len() < 2 {
        return (0.0, "flat".to_string(), 0.0, 0.0);
    }

    let mut ordered = samples.to_vec();
    ordered.sort_by_key(|(ts, _)| *ts);

    let split = ordered.len() / 2;
    if split == 0 || split == ordered.len() {
        return (0.0, "flat".to_string(), 0.0, 0.0);
    }

    let baseline = &ordered[..split];
    let recent = &ordered[split..];
    let baseline_rate = ratio(
        baseline.iter().filter(|(_, passed)| *passed).count() as u64,
        baseline.len() as u64,
    );
    let recent_rate = ratio(
        recent.iter().filter(|(_, passed)| *passed).count() as u64,
        recent.len() as u64,
    );
    let delta = recent_rate - baseline_rate;
    let direction = if delta > 0.01 {
        "improving"
    } else if delta < -0.01 {
        "declining"
    } else {
        "flat"
    };

    (delta, direction.to_string(), baseline_rate, recent_rate)
}

fn cascade_stage_for_observations(observations: u64) -> CascadeStage {
    if observations < COLD_START_THRESHOLD {
        CascadeStage::Static
    } else if observations < 200 {
        CascadeStage::Confidence
    } else {
        CascadeStage::Ucb
    }
}

fn median(values: &[f64]) -> Option<f64> {
    match values.len() {
        0 => None,
        len if len % 2 == 1 => values.get(len / 2).copied(),
        len => {
            let upper = values.get(len / 2).copied()?;
            let lower = values.get((len / 2).saturating_sub(1)).copied()?;
            Some((lower + upper) / 2.0)
        }
    }
}

async fn read_cfactor_history(path: &std::path::Path) -> Result<Vec<CFactor>, ApiError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(ApiError::internal(format!("read {}: {e}", path.display()))),
    };

    let mut history = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(snapshot) = serde_json::from_str::<CFactor>(trimmed) {
            history.push(snapshot);
        }
    }
    Ok(history)
}

async fn read_efficiency_events(
    path: &std::path::Path,
) -> Result<Vec<AgentEfficiencyEvent>, ApiError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(ApiError::internal(format!("read {}: {e}", path.display()))),
    };

    let mut events = Vec::new();
    for (line_no, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str::<AgentEfficiencyEvent>(line).map_err(|e| {
            ApiError::internal(format!(
                "parse {} line {}: {e}",
                path.display(),
                line_no + 1
            ))
        })?;
        events.push(event);
    }
    Ok(events)
}

async fn read_experiment_store(path: &std::path::Path) -> Result<ExperimentStore, ApiError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ExperimentStore::new()),
        Err(e) => return Err(ApiError::internal(format!("read {}: {e}", path.display()))),
    };

    serde_json::from_str(&content)
        .map_err(|e| ApiError::internal(format!("parse {}: {e}", path.display())))
}

/// Read the persisted cascade router snapshot if it exists.
async fn read_cascade_snapshot(
    path: &std::path::Path,
) -> Result<Option<CascadeSnapshotData>, ApiError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(ApiError::internal(format!("read {}: {e}", path.display()))),
    };

    let snapshot = serde_json::from_str::<CascadeSnapshotData>(&content)
        .map_err(|e| ApiError::internal(format!("parse {}: {e}", path.display())))?;
    Ok(Some(snapshot))
}

fn parse_period(period: Option<&str>) -> (String, u64) {
    let raw = period.unwrap_or("last_7_days").trim();
    match raw {
        "last_7_days" => ("last_7_days".to_string(), 7),
        "last_30_days" => ("last_30_days".to_string(), 30),
        "last_90_days" => ("last_90_days".to_string(), 90),
        other => {
            if let Some(days) = other
                .strip_prefix("last_")
                .and_then(|rest| rest.strip_suffix("_days"))
                .and_then(|n| n.parse::<u64>().ok())
            {
                (format!("last_{days}_days"), days)
            } else {
                ("last_7_days".to_string(), 7)
            }
        }
    }
}

fn event_time(event: &AgentEfficiencyEvent) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&event.timestamp)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn ratio(numer: u64, denom: u64) -> f64 {
    if denom == 0 {
        0.0
    } else {
        numer as f64 / denom as f64
    }
}

fn feedback_engagement_rate(episodes: &[Episode]) -> f64 {
    if episodes.is_empty() {
        return 0.0;
    }

    let engaged = episodes
        .iter()
        .filter(|episode| !episode.gate_verdicts.is_empty())
        .count() as u64;
    ratio(engaged, episodes.len() as u64)
}

fn gate_pass_rate(episodes: &[Episode], efficiency_events: &[AgentEfficiencyEvent]) -> f64 {
    let mut passed = 0u64;
    let mut total = 0u64;

    for episode in episodes {
        for verdict in &episode.gate_verdicts {
            total += 1;
            if verdict.passed {
                passed += 1;
            }
        }
    }

    if total > 0 {
        return ratio(passed, total);
    }

    let passed_events = efficiency_events
        .iter()
        .filter(|event| event.gate_passed)
        .count() as u64;
    ratio(passed_events, efficiency_events.len() as u64)
}

fn self_improvement_velocity(events: &[AgentEfficiencyEvent]) -> f64 {
    if events.len() < 2 {
        return 0.0;
    }

    let mut ordered: Vec<&AgentEfficiencyEvent> = events.iter().collect();
    ordered.sort_by_key(|event| event_time(event));

    let first_ts = match ordered.first().and_then(|event| event_time(event)) {
        Some(ts) => ts,
        None => return 0.0,
    };
    let last_ts = match ordered.last().and_then(|event| event_time(event)) {
        Some(ts) => ts,
        None => return 0.0,
    };

    if first_ts == last_ts {
        return 0.0;
    }

    let split = ordered.len() / 2;
    if split == 0 || split == ordered.len() {
        return 0.0;
    }

    let early = &ordered[..split];
    let late = &ordered[split..];
    let early_rate = ratio(
        early.iter().filter(|event| event.gate_passed).count() as u64,
        early.len() as u64,
    );
    let late_rate = ratio(
        late.iter().filter(|event| event.gate_passed).count() as u64,
        late.len() as u64,
    );

    let span_days = (last_ts - first_ts).num_seconds().max(1) as f64 / 86_400.0;
    (late_rate - early_rate) / span_days
}

fn best_experiment_lift(store: &ExperimentStore) -> Option<ExperimentLiftSummary> {
    let mut best: Option<ExperimentLiftSummary> = None;

    for experiment in store
        .iter()
        .filter(|experiment| experiment.status == ExperimentStatus::Running)
    {
        let mut active_variants: Vec<_> = experiment
            .variants
            .iter()
            .filter(|variant| variant.active)
            .map(|variant| {
                let stats = experiment
                    .stats
                    .get(&variant.id)
                    .cloned()
                    .unwrap_or_default();
                (variant.name.clone(), stats.trials, stats.success_rate())
            })
            .collect();

        active_variants.sort_by(|a, b| {
            b.2.partial_cmp(&a.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.0.cmp(&b.0))
        });

        let Some((winning_name, _, winning_rate)) = active_variants.first().cloned() else {
            continue;
        };
        let Some((_, _, runner_up_rate)) = active_variants.get(1).cloned() else {
            continue;
        };

        let lift = winning_rate - runner_up_rate;
        let candidate = ExperimentLiftSummary {
            name: if experiment.section_name.trim().is_empty() {
                experiment.experiment_id.clone()
            } else {
                experiment.section_name.clone()
            },
            lift,
            winning: winning_name,
        };

        if best
            .as_ref()
            .map(|current| candidate.lift > current.lift)
            .unwrap_or(true)
        {
            best = Some(candidate);
        }
    }

    best
}

async fn top_templates(state: &AppState, window_start: DateTime<Utc>) -> Vec<TemplateSummary> {
    let runs = state.template_runs.read().await;
    let mut summary: BTreeMap<String, RunAggregate> = BTreeMap::new();

    for (template_name, records) in runs.iter() {
        let aggregate = summary.entry(template_name.clone()).or_default();
        for record in records
            .iter()
            .filter(|record| record.timestamp >= window_start)
        {
            aggregate.runs += 1;
            if record.success {
                aggregate.successes += 1;
            }
        }
    }

    let mut templates: Vec<TemplateSummary> = summary
        .into_iter()
        .filter(|(_, aggregate)| aggregate.runs > 0)
        .map(|(name, aggregate)| TemplateSummary {
            name,
            runs: aggregate.runs,
            success_rate: ratio(aggregate.successes, aggregate.runs),
        })
        .collect();

    templates.sort_by(|a, b| {
        b.runs
            .cmp(&a.runs)
            .then_with(|| {
                b.success_rate
                    .partial_cmp(&a.success_rate)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.name.cmp(&b.name))
    });

    templates.truncate(5);
    templates
}

#[derive(Debug, Default)]
struct GateSummaryAcc {
    total_runs: u64,
    passed_runs: u64,
    total_duration_ms: f64,
    last_run: Option<Value>,
}

#[derive(Debug, Serialize)]
struct GateSummary {
    total_runs: u64,
    pass_rate: f64,
    avg_duration_ms: f64,
    last_run: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct GateRungSummary {
    rung: u32,
    passed_runs: u64,
    failed_runs: u64,
    total_runs: u64,
    pass_rate: f64,
}

fn summarize_gate_entries(entries: &[Value]) -> Value {
    let mut by_gate: BTreeMap<String, GateSummaryAcc> = BTreeMap::new();

    for entry in entries {
        let Some(kind) = entry.get("kind").and_then(Value::as_str) else {
            continue;
        };
        if !is_gate_result_kind(kind) {
            continue;
        }

        let Some(gate_name) = extract_gate_name(entry) else {
            continue;
        };
        let Some(passed) = extract_gate_passed(entry) else {
            continue;
        };
        let duration_ms = extract_gate_duration_ms(entry).unwrap_or(0);

        let acc = by_gate.entry(gate_name).or_default();
        acc.total_runs += 1;
        if passed {
            acc.passed_runs += 1;
        }
        acc.total_duration_ms += duration_ms as f64;
        acc.last_run = Some(entry.clone());
    }

    let summary: BTreeMap<String, GateSummary> = by_gate
        .into_iter()
        .filter_map(|(gate, acc)| {
            let last_run = acc.last_run?;
            let total_runs = acc.total_runs;
            let pass_rate = if total_runs == 0 {
                0.0
            } else {
                acc.passed_runs as f64 / total_runs as f64
            };
            let avg_duration_ms = if total_runs == 0 {
                0.0
            } else {
                acc.total_duration_ms / total_runs as f64
            };
            Some((
                gate,
                GateSummary {
                    total_runs,
                    pass_rate,
                    avg_duration_ms,
                    last_run,
                },
            ))
        })
        .collect();

    serde_json::to_value(summary).unwrap_or_else(|_| json!({}))
}

fn summarize_gate_rungs(entries: &[Value]) -> Vec<GateRungSummary> {
    let mut by_rung: BTreeMap<u32, (u64, u64)> = BTreeMap::new();

    for entry in entries {
        let Some(kind) = entry.get("kind").and_then(Value::as_str) else {
            continue;
        };
        if !is_gate_result_kind(kind) {
            continue;
        }

        let Some(rung) = extract_gate_rung(entry) else {
            continue;
        };
        let Some(passed) = extract_gate_passed(entry) else {
            continue;
        };

        let counts = by_rung.entry(rung).or_insert((0, 0));
        counts.1 += 1;
        if passed {
            counts.0 += 1;
        }
    }

    let mut rungs = by_rung
        .into_iter()
        .map(|(rung, (passed_runs, total_runs))| GateRungSummary {
            rung,
            passed_runs,
            failed_runs: total_runs.saturating_sub(passed_runs),
            total_runs,
            pass_rate: ratio(passed_runs, total_runs),
        })
        .collect::<Vec<_>>();

    rungs.sort_by_key(|summary| summary.rung);
    rungs
}

fn build_recent_gate_history(entries: &[Value], gate_filter: Option<&str>) -> Vec<Value> {
    let mut history: Vec<Value> = entries
        .iter()
        .filter(|entry| {
            let Some(kind) = entry.get("kind").and_then(Value::as_str) else {
                return false;
            };
            if !is_gate_result_kind(kind) {
                return false;
            }
            match gate_filter {
                Some(gate) => extract_gate_name(entry).as_deref() == Some(gate),
                None => true,
            }
        })
        .filter_map(|entry| {
            let gate = extract_gate_name(entry)?;
            let passed = extract_gate_passed(entry)?;
            Some(json!({
                "signal_id": entry.get("id").cloned().unwrap_or(Value::Null),
                "created_at_ms": entry.get("created_at_ms").cloned().unwrap_or(Value::Null),
                "gate": gate,
                "passed": passed,
                "duration_ms": extract_gate_duration_ms(entry).unwrap_or(0),
                "plan_id": entry.pointer("/tags/plan_id")
                    .cloned()
                    .or_else(|| entry.pointer("/body/data/plan_id").cloned())
                    .unwrap_or(Value::Null),
                "task_id": entry.pointer("/tags/task_id")
                    .cloned()
                    .or_else(|| entry.pointer("/body/data/task_id").cloned())
                    .unwrap_or(Value::Null),
                "rung": entry.pointer("/tags/rung")
                    .cloned()
                    .or_else(|| entry.pointer("/body/data/rung").cloned())
                    .unwrap_or(Value::Null),
                "kind": entry.get("kind").cloned().unwrap_or(Value::Null),
            }))
        })
        .collect();

    history.sort_by(|a, b| {
        let a_ts = a
            .get("created_at_ms")
            .and_then(Value::as_i64)
            .unwrap_or(i64::MIN);
        let b_ts = b
            .get("created_at_ms")
            .and_then(Value::as_i64)
            .unwrap_or(i64::MIN);
        b_ts.cmp(&a_ts).then_with(|| {
            let a_id = a.get("signal_id").and_then(Value::as_str).unwrap_or("");
            let b_id = b.get("signal_id").and_then(Value::as_str).unwrap_or("");
            b_id.cmp(a_id)
        })
    });

    history
}

fn is_gate_result_kind(kind: &str) -> bool {
    kind == "gate_verdict" || kind.starts_with("gate:") || kind.starts_with("gate_")
}

fn extract_gate_name(entry: &Value) -> Option<String> {
    entry
        .pointer("/tags/gate")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            entry
                .pointer("/body/data/gate")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            entry
                .pointer("/body/gate")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            entry
                .get("kind")
                .and_then(Value::as_str)
                .and_then(|kind| kind.strip_prefix("gate:").or(kind.strip_prefix("gate_")))
                .map(ToOwned::to_owned)
        })
}

fn extract_gate_passed(entry: &Value) -> Option<bool> {
    entry
        .pointer("/tags/passed")
        .and_then(Value::as_str)
        .and_then(|s| match s {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
        .or_else(|| entry.pointer("/body/data/passed").and_then(Value::as_bool))
        .or_else(|| entry.pointer("/body/passed").and_then(Value::as_bool))
}

fn extract_gate_duration_ms(entry: &Value) -> Option<u64> {
    entry
        .pointer("/body/data/duration_ms")
        .and_then(Value::as_u64)
        .or_else(|| entry.pointer("/body/duration_ms").and_then(Value::as_u64))
        .or_else(|| entry.pointer("/tags/duration_ms").and_then(Value::as_u64))
}

fn extract_gate_rung(entry: &Value) -> Option<u32> {
    let raw = entry
        .pointer("/tags/rung")
        .or_else(|| entry.pointer("/body/data/rung"))
        .or_else(|| entry.pointer("/body/rung"))?;

    raw.as_u64()
        .map(|rung| rung as u32)
        .or_else(|| raw.as_str().and_then(|text| text.parse::<u32>().ok()))
}

fn build_cfactor_metrics_response(
    composite_path: &std::path::Path,
    history: &[CFactor],
    efficiency_path: &std::path::Path,
    events: &[AgentEfficiencyEvent],
    fleet: FleetCFactor,
) -> Value {
    let composite = history.last().cloned().unwrap_or_default();
    let per_agent = composite
        .agent_contributions
        .iter()
        .map(|contribution| CFactorAgentSummary {
            agent_id: contribution.agent_id.clone(),
            episode_count: contribution.episode_count,
            without_agent_overall: contribution.without_agent_overall,
            contribution_score: contribution.contribution_score,
            dispatch_bias: dispatch_bias_label(
                composite.dispatch_bias_for_agent(contribution.agent_id.as_str()),
            ),
        })
        .collect::<Vec<_>>();

    let response = CFactorMetricsResponse {
        source: CFactorMetricsSource {
            composite_history_path: composite_path.display().to_string(),
            efficiency_events_path: efficiency_path.display().to_string(),
            composite_history_count: history.len(),
            efficiency_event_count: events.len(),
        },
        composite: CFactorCompositeSummary {
            overall: composite.overall,
            computed_at: composite.computed_at,
            episode_count: composite.episode_count,
            history_count: history.len(),
        },
        sub_metrics: composite.components,
        per_agent,
        per_fleet: fleet,
    };

    serde_json::to_value(response).unwrap_or_else(|e| {
        json!({
            "error": format!("serialize c-factor metrics: {e}"),
        })
    })
}

fn dispatch_bias_label(bias: AgentDispatchBias) -> String {
    match bias {
        AgentDispatchBias::PreferStronger => "prefer_stronger".to_string(),
        AgentDispatchBias::PreferCheaper => "prefer_cheaper".to_string(),
        AgentDispatchBias::Neutral => "neutral".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use axum::body::Body as AxumBody;
    use axum::extract::{Path, Query, State};
    use axum::http::Request;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::deploy::create_backend;
    use crate::routes::build_router;
    use crate::runtime::NoOpRuntime;
    use crate::state::{AppState, OperationStatus, PlanHandle};
    use roko_core::config::ServeAuthConfig;
    use roko_core::{Body, Engram, Kind, Provenance, Verdict};

    fn gate_signal(gate: &str, passed: bool, duration_ms: u64) -> Value {
        let mut verdict = if passed {
            Verdict::pass(gate)
        } else {
            Verdict::fail(gate, "boom")
        };
        verdict.duration_ms = duration_ms;

        let signal = Engram::builder(Kind::GateVerdict)
            .body(
                Body::from_json(&verdict)
                    .expect("invariant: verdict helper should serialize test payloads"),
            )
            .provenance(Provenance::trusted("test"))
            .tag("gate", gate)
            .tag("passed", passed.to_string())
            .build();
        let mut signal = serde_json::to_value(signal)
            .expect("invariant: verdict helper should serialize signal envelopes");
        signal
            .as_object_mut()
            .expect("gate signal should be an object")
            .entry("tags")
            .or_insert_with(|| serde_json::json!({}));
        signal
    }

    fn gate_signal_with_rung(gate: &str, rung: u32, passed: bool, duration_ms: u64) -> Value {
        let mut signal = gate_signal(gate, passed, duration_ms);
        signal
            .as_object_mut()
            .expect("gate signal should be an object")
            .get_mut("tags")
            .and_then(Value::as_object_mut)
            .expect("tags should be an object")
            .insert("rung".into(), Value::from(rung));
        signal
    }

    #[test]
    fn summarize_gate_entries_aggregates_by_gate_name() {
        let entries = vec![
            gate_signal("compile", true, 100),
            gate_signal("compile", false, 300),
            gate_signal("test", true, 200),
        ];

        let summary = summarize_gate_entries(&entries);

        assert_eq!(summary["compile"]["total_runs"], 2);
        assert_eq!(summary["compile"]["pass_rate"], 0.5);
        assert_eq!(summary["compile"]["avg_duration_ms"], 200.0);
        assert_eq!(summary["compile"]["last_run"]["tags"]["passed"], "false");
        assert_eq!(summary["test"]["total_runs"], 1);
        assert_eq!(summary["test"]["pass_rate"], 1.0);
        assert_eq!(summary["test"]["avg_duration_ms"], 200.0);
    }

    #[test]
    fn gate_history_filters_and_orders_by_timestamp() {
        let mut compile_late = gate_signal("compile", false, 300);
        compile_late
            .as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(20));
        let mut compile_early = gate_signal("compile", true, 100);
        compile_early
            .as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(10));
        let mut test = gate_signal("test", true, 200);
        test.as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(15));

        let entries = vec![compile_late, compile_early, test];
        let mut history: Vec<Value> = entries
            .into_iter()
            .filter(|entry| extract_gate_name(entry).as_deref() == Some("compile"))
            .filter_map(|entry| {
                let passed = extract_gate_passed(&entry)?;
                Some(json!({
                    "signal_id": entry.get("id").cloned().unwrap_or(Value::Null),
                    "created_at_ms": entry.get("created_at_ms").cloned().unwrap_or(Value::Null),
                    "gate": "compile",
                    "passed": passed,
                }))
            })
            .collect();

        history.sort_by(|a, b| {
            let a_ts = a
                .get("created_at_ms")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MIN);
            let b_ts = b
                .get("created_at_ms")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MIN);
            a_ts.cmp(&b_ts)
        });

        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["passed"], true);
        assert_eq!(history[0]["created_at_ms"], 10);
        assert_eq!(history[1]["passed"], false);
        assert_eq!(history[1]["created_at_ms"], 20);
    }

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let state = Arc::new(AppState::new(
            workdir,
            Arc::new(NoOpRuntime),
            roko_core::config::schema::RokoConfig::default(),
            deploy_backend,
        ));
        (dir, state)
    }

    #[tokio::test]
    async fn gates_history_collection_is_mounted_under_api_grouping() {
        let (dir, state) = test_state();
        let signals = dir.path().join(".roko").join("signals.jsonl");
        tokio::fs::create_dir_all(signals.parent().expect("signals parent"))
            .await
            .expect("create signals dir");
        let mut compile_early = gate_signal("compile", true, 120);
        compile_early
            .as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(10));
        let mut compile_late = gate_signal("compile", false, 300);
        compile_late
            .as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(20));
        let mut test = gate_signal("test", true, 200);
        test.as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(30));
        tokio::fs::write(
            &signals,
            [compile_early, compile_late, test]
                .into_iter()
                .map(|entry| entry.to_string())
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .await
        .expect("write gate history");

        let app = build_router(Arc::clone(&state), &[], ServeAuthConfig::default());
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/gates/history?limit=2")
                    .body(AxumBody::empty())
                    .expect("request"),
            )
            .await
            .expect("gate history response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let payload: Value = serde_json::from_slice(&body).expect("parse gate history response");
        assert_eq!(payload["source"], signals.display().to_string());
        assert_eq!(payload["total"], 3);
        assert_eq!(payload["limit"], 2);
        assert_eq!(
            payload["history"]
                .as_array()
                .expect("invariant: gate history payload should contain a history array")
                .len(),
            2
        );
        assert_eq!(payload["history"][0]["gate"], "test");
        assert_eq!(payload["history"][1]["gate"], "compile");
    }

    #[tokio::test]
    async fn gate_summary_includes_rung_breakdown_under_api_grouping() {
        let (dir, state) = test_state();
        let signals = dir.path().join(".roko").join("signals.jsonl");
        tokio::fs::create_dir_all(signals.parent().expect("signals parent"))
            .await
            .expect("create signals dir");
        let mut compile_pass = gate_signal_with_rung("compile", 1, true, 120);
        compile_pass
            .as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(10));
        let mut compile_fail = gate_signal_with_rung("compile", 1, false, 300);
        compile_fail
            .as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(20));
        let mut test_pass = gate_signal_with_rung("test", 2, true, 200);
        test_pass
            .as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(30));
        tokio::fs::write(
            &signals,
            [compile_pass, compile_fail, test_pass]
                .into_iter()
                .map(|entry| entry.to_string())
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .await
        .expect("write gate summary");

        let app = build_router(Arc::clone(&state), &[], ServeAuthConfig::default());
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/gates/summary")
                    .body(AxumBody::empty())
                    .expect("request"),
            )
            .await
            .expect("gate summary response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let payload: Value = serde_json::from_slice(&body).expect("parse gate summary response");
        assert_eq!(payload["compile"]["total_runs"], 2);
        assert_eq!(payload["compile"]["pass_rate"], 0.5);
        assert_eq!(
            payload["rungs"]
                .as_array()
                .expect("invariant: gate summary payload should contain a rung array")
                .len(),
            2
        );
        assert_eq!(payload["rungs"][0]["rung"], 1);
        assert_eq!(payload["rungs"][0]["passed_runs"], 1);
        assert_eq!(payload["rungs"][0]["failed_runs"], 1);
        assert_eq!(payload["rungs"][1]["rung"], 2);
        assert_eq!(payload["rungs"][1]["passed_runs"], 1);
        assert_eq!(payload["rungs"][1]["failed_runs"], 0);
    }

    #[tokio::test]
    async fn health_reports_status_version_uptime_and_counts() {
        let (_dir, state) = test_state();

        let response = health(State(state.clone())).await;
        let body = response.1.0;

        assert_eq!(body["status"], "ok");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert!(body["uptime_secs"].as_u64().is_some());
        assert_eq!(body["active_plans"], 0);
        assert_eq!(body["active_agents"], 0);
    }

    #[tokio::test]
    async fn metrics_summary_includes_active_plans_and_c_factor() {
        let (dir, state) = test_state();
        let plan_handle = PlanHandle {
            id: "plan-1".into(),
            plan_dir: dir.path().join(".roko/plans/plan-1"),
            status: OperationStatus::Running,
            handle: tokio::spawn(async {}),
        };
        state
            .active_plans
            .write()
            .await
            .insert("plan-1".into(), plan_handle);

        let app = build_router(Arc::clone(&state), &[], ServeAuthConfig::default());
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/metrics/summary")
                    .body(AxumBody::empty())
                    .expect("request"),
            )
            .await
            .expect("metrics summary response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let payload: Value = serde_json::from_slice(&body).expect("parse metrics summary response");
        assert_eq!(payload["period"], "last_7_days");
        assert_eq!(payload["active_plans"], 1);
        assert_eq!(payload["c_factor"], 0.0);
        assert_eq!(payload["experiments_active"], 0);
    }

    #[tokio::test]
    async fn c_factor_metrics_combines_composite_and_fleet_snapshots() {
        let (dir, state) = test_state();
        let learn_dir = dir.path().join(".roko").join("learn");
        tokio::fs::create_dir_all(&learn_dir)
            .await
            .expect("create learn dir");

        let c_factor_path = learn_dir.join("c-factor.jsonl");
        let efficiency_path = learn_dir.join("efficiency.jsonl");

        let earlier = serde_json::json!({
            "overall": 0.25,
            "components": {
                "gate_pass_rate": 0.20,
                "cost_efficiency": 0.20,
                "speed": 0.20,
                "information_flow_rate": 0.20,
                "first_try_rate": 0.20,
                "knowledge_growth": 0.20,
                "knowledge_integration_rate": 0.20,
                "task_diversity_coverage": 0.20,
                "convergence_velocity": 0.20,
                "turn_taking_equality": 0.20,
                "social_sensitivity": 0.20
            },
            "agent_contributions": [
                {
                    "agent_id": "agent-a",
                    "episode_count": 1,
                    "without_agent_overall": 0.10,
                    "contribution_score": 0.15
                }
            ],
            "computed_at": "2026-04-04T12:00:00Z",
            "episode_count": 1
        });
        let recent = serde_json::json!({
            "overall": 0.71,
            "components": {
                "gate_pass_rate": 0.80,
                "cost_efficiency": 0.60,
                "speed": 0.55,
                "information_flow_rate": 0.40,
                "first_try_rate": 0.75,
                "knowledge_growth": 0.30,
                "knowledge_integration_rate": 0.25,
                "task_diversity_coverage": 0.35,
                "convergence_velocity": 0.45,
                "turn_taking_equality": 0.50,
                "social_sensitivity": 0.65
            },
            "agent_contributions": [
                {
                    "agent_id": "agent-a",
                    "episode_count": 3,
                    "without_agent_overall": 0.58,
                    "contribution_score": 0.13
                },
                {
                    "agent_id": "agent-b",
                    "episode_count": 2,
                    "without_agent_overall": 0.79,
                    "contribution_score": -0.08
                }
            ],
            "computed_at": "2026-04-07T12:00:00Z",
            "episode_count": 5
        });

        tokio::fs::write(
            &c_factor_path,
            [earlier.to_string(), recent.to_string()].join("\n") + "\n",
        )
        .await
        .expect("write c-factor history");

        let events = vec![
            serde_json::json!({
                "agent_id": "agent-a",
                "role": "Implementer",
                "backend": "claude",
                "model": "claude-sonnet-4-6",
                "plan_id": "plan-a",
                "task_id": "task-a1",
                "input_tokens": 1000,
                "output_tokens": 200,
                "cache_read_tokens": 100,
                "cache_write_tokens": 10,
                "cost_usd": 0.40,
                "cost_usd_without_cache": 0.50,
                "prompt_sections": [],
                "total_prompt_tokens": 1200,
                "system_prompt_tokens": 200,
                "tools_available": 8,
                "tools_used": 4,
                "tool_calls": [],
                "wall_time_ms": 4000,
                "duration_ms": 4000,
                "time_to_first_token_ms": 500,
                "was_warm_start": false,
                "iteration": 1,
                "gate_passed": true,
                "outcome": "success",
                "gate_errors": [],
                "model_used": "claude-sonnet-4-6",
                "frequency": "theta",
                "strategy_attempted": "none",
                "timestamp": "2026-04-07T12:00:00Z"
            }),
            serde_json::json!({
                "agent_id": "agent-b",
                "role": "Reviewer",
                "backend": "claude",
                "model": "claude-sonnet-4-6",
                "plan_id": "plan-a",
                "task_id": "task-a2",
                "input_tokens": 900,
                "output_tokens": 150,
                "cache_read_tokens": 80,
                "cache_write_tokens": 10,
                "cost_usd": 0.30,
                "cost_usd_without_cache": 0.40,
                "prompt_sections": [],
                "total_prompt_tokens": 1050,
                "system_prompt_tokens": 200,
                "tools_available": 8,
                "tools_used": 3,
                "tool_calls": [],
                "wall_time_ms": 3000,
                "duration_ms": 3000,
                "time_to_first_token_ms": 450,
                "was_warm_start": true,
                "iteration": 1,
                "gate_passed": true,
                "outcome": "success",
                "gate_errors": [],
                "model_used": "claude-sonnet-4-6",
                "frequency": "theta",
                "strategy_attempted": "none",
                "timestamp": "2026-04-07T12:05:00Z"
            }),
            serde_json::json!({
                "agent_id": "agent-c",
                "role": "Implementer",
                "backend": "claude",
                "model": "claude-haiku-4-5",
                "plan_id": "plan-b",
                "task_id": "task-b1",
                "input_tokens": 700,
                "output_tokens": 100,
                "cache_read_tokens": 50,
                "cache_write_tokens": 5,
                "cost_usd": 0.10,
                "cost_usd_without_cache": 0.15,
                "prompt_sections": [],
                "total_prompt_tokens": 800,
                "system_prompt_tokens": 180,
                "tools_available": 6,
                "tools_used": 2,
                "tool_calls": [],
                "wall_time_ms": 2000,
                "duration_ms": 2000,
                "time_to_first_token_ms": 350,
                "was_warm_start": false,
                "iteration": 1,
                "gate_passed": false,
                "outcome": "failure",
                "gate_errors": ["test failed"],
                "model_used": "claude-haiku-4-5",
                "frequency": "theta",
                "strategy_attempted": "retry_same",
                "timestamp": "2026-04-07T12:10:00Z"
            }),
        ];
        tokio::fs::write(
            &efficiency_path,
            events
                .into_iter()
                .map(|event| event.to_string())
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .await
        .expect("write efficiency events");

        let response = c_factor_metrics(State(state))
            .await
            .expect("c-factor metrics");
        let body = response.0;

        assert_eq!(body["source"]["composite_history_count"], 2);
        assert_eq!(body["source"]["efficiency_event_count"], 3);
        assert_eq!(body["composite"]["overall"], 0.71);
        assert_eq!(body["composite"]["episode_count"], 5);
        assert_eq!(body["sub_metrics"]["gate_pass_rate"], 0.80);
        assert_eq!(body["per_agent"][0]["agent_id"], "agent-a");
        assert_eq!(body["per_agent"][0]["dispatch_bias"], "prefer_cheaper");
        assert_eq!(body["per_agent"][1]["dispatch_bias"], "prefer_stronger");
        assert_eq!(body["per_fleet"]["plan_count"], 2);
        assert_eq!(body["per_fleet"]["agent_count"], 3);
        assert_eq!(body["per_fleet"]["observation_count"], 3);
    }

    #[tokio::test]
    async fn gate_history_returns_404_for_missing_gate() {
        let (_dir, state) = test_state();

        let err = gate_history(State(state), Path("compile".into()))
            .await
            .expect_err("missing gate should fail");

        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn gate_history_returns_500_for_invalid_jsonl() {
        let (dir, state) = test_state();
        let signals = dir.path().join(".roko").join("signals.jsonl");
        tokio::fs::create_dir_all(signals.parent().expect("signals parent"))
            .await
            .expect("create signals dir");
        tokio::fs::write(&signals, "{not-json}\n")
            .await
            .expect("write corrupt signals");

        let err = gate_history(State(state), Path("compile".into()))
            .await
            .expect_err("corrupt signals should fail");

        assert_eq!(err.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn signals_returns_500_for_invalid_jsonl() {
        let (dir, state) = test_state();
        let signals_path = dir.path().join(".roko").join("signals.jsonl");
        tokio::fs::create_dir_all(signals_path.parent().expect("signals parent"))
            .await
            .expect("create signals dir");
        tokio::fs::write(&signals_path, "{not-json}\n")
            .await
            .expect("write corrupt signals");

        let err = signals(State(state), Query(SignalQuery { limit: Some(1) }))
            .await
            .expect_err("corrupt signals should fail");

        assert_eq!(err.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }
}
