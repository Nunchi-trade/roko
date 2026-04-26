//! Metrics endpoints — success rate, engagement, c-factor, model efficiency,
//! gate rate, experiments, feedback latency, velocity, coverage, and prometheus.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::event_bus::Envelope;
use crate::projection_contract::{ProjectionQuery, RuntimeProjectionSet};
use crate::state::AppState;
use roko_learn::cascade_router::CascadeStage;
use roko_learn::cfactor::{AgentDispatchBias, CFactor, CFactorComponents};
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::efficiency::{FleetCFactor, compute_fleet_cfactor};
use roko_learn::episode_logger::Episode;
use roko_learn::model_router::COLD_START_THRESHOLD;
use roko_learn::prompt_experiment::{ExperimentStatus, ExperimentStore};

use super::helpers::{read_cfactor_history, read_jsonl_entries};
use crate::routes::learning::helpers::read_experiment_store;
use crate::routes::learning::router_state::{CascadeSnapshotData, read_cascade_snapshot};

// ── handler functions ────────────────────────────────────────────────

/// `GET /api/metrics` — metric snapshots as JSON.
pub async fn metrics(State(state): State<Arc<AppState>>) -> Json<Value> {
    let snapshots = state.metrics.snapshot();
    Json(serde_json::to_value(snapshots).unwrap_or(json!([])))
}

#[derive(Debug, Deserialize)]
pub struct MetricsSummaryQuery {
    #[serde(default)]
    period: Option<String>,
}

/// `GET /api/metrics/summary` — aggregate recent execution and learning metrics.
pub async fn metrics_summary(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MetricsSummaryQuery>,
) -> Result<Json<Value>, ApiError> {
    let summary = build_metrics_summary(&state, query.period.as_deref()).await?;
    Ok(Json(serde_json::to_value(summary).map_err(|e| {
        ApiError::internal(format!("serialize metrics summary: {e}"))
    })?))
}

/// `GET /api/metrics/success_rate` — per template success rate, split by trigger kind.
pub async fn success_rate(State(state): State<Arc<AppState>>) -> Json<Value> {
    let runs = state.template_runs.read().await;
    Json(build_template_success_rate(&runs))
}

/// `GET /api/metrics/engagement` — feedback acknowledgement ratio per template.
pub async fn engagement(State(state): State<Arc<AppState>>) -> Json<Value> {
    let runs = state.template_runs.read().await;
    Json(build_template_engagement(&runs))
}

/// `GET /api/metrics/c_factor` — composite C-Factor, component metrics, per-agent, and per-fleet.
pub async fn c_factor_metrics(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let composite_path = state.workdir.join(".roko/learn/c-factor.jsonl");
    let projections = RuntimeProjectionSet::load(&state).await?;
    let efficiency_path = projections.feedback.efficiency_path.clone();

    let history = read_cfactor_history(&composite_path).await?;
    let events = projections.efficiency_events().to_vec();
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
pub async fn model_efficiency(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko/learn/cascade-router.json");
    let snapshot = read_cascade_snapshot(&path).await?;
    let projections = RuntimeProjectionSet::load(&state).await?;
    Ok(Json(build_model_efficiency_response(
        &path,
        snapshot,
        projections.efficiency_events(),
    )))
}

/// `GET /api/metrics/gate_rate` — passed / total per gate with a trend delta.
pub async fn gate_rate(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    let query = ProjectionQuery::default();
    Ok(Json(json!({
        "summary": projections.gate_summary(&query),
        "history": projections.gate_history(&query),
        "evidence": projections.evidence(),
    })))
}

/// `GET /api/metrics/experiments` — best vs worst variant gap per experiment.
pub async fn experiments_metric(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko/learn/experiments.json");
    let store = read_experiment_store(&path).await?;
    Ok(Json(build_experiment_metrics_response(&path, &store)))
}

/// `GET /api/metrics/feedback_latency` — median hours from action to first feedback signal.
pub async fn feedback_latency(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko").join("engrams.jsonl");
    let entries = read_jsonl_entries(&path).await?;
    Ok(Json(build_feedback_latency_response(&entries)))
}

/// `GET /api/metrics/velocity` — rate of change of success rate over time.
pub async fn velocity(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    Ok(Json(json!({
        "velocity": self_improvement_velocity(projections.efficiency_events()),
        "sample_count": projections.efficiency_events().len(),
        "evidence": projections.evidence(),
    })))
}

/// `GET /api/metrics/coverage` — percentage of events that matched a known subscription.
pub async fn coverage(State(state): State<Arc<AppState>>) -> Json<Value> {
    let backlog = state.event_bus.replay_from(0);
    Json(build_coverage_response(&backlog))
}

/// `GET /api/metrics/prometheus` — Prometheus text exposition format.
pub async fn prometheus_metrics(
    State(state): State<Arc<AppState>>,
) -> (
    axum::http::StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    let snapshot = state.state_hub.current_snapshot();
    let s = &snapshot.stats;
    let uptime = state.started_at.elapsed().as_secs();
    let active_agents = state.supervisor.count().await;
    let active_plans = state.active_plans.read().await.len();

    // Episode count from JSONL file (best-effort).
    let episodes_path = state.layout.episodes_path();
    let episode_count = tokio::fs::read_to_string(&episodes_path)
        .await
        .map(|content| content.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);

    let mut out = String::with_capacity(2048);

    // Helper macro for Prometheus lines.
    macro_rules! prom {
        (counter, $name:expr, $help:expr, $val:expr) => {{
            out.push_str(&format!("# HELP {} {}\n", $name, $help));
            out.push_str(&format!("# TYPE {} counter\n", $name));
            out.push_str(&format!("{} {}\n", $name, $val));
        }};
        (gauge, $name:expr, $help:expr, $val:expr) => {{
            out.push_str(&format!("# HELP {} {}\n", $name, $help));
            out.push_str(&format!("# TYPE {} gauge\n", $name));
            out.push_str(&format!("{} {}\n", $name, $val));
        }};
    }

    prom!(
        gauge,
        "roko_uptime_seconds",
        "Seconds since roko-serve started",
        uptime
    );
    prom!(
        gauge,
        "roko_agents_active",
        "Number of currently active agents",
        active_agents
    );
    prom!(
        gauge,
        "roko_plans_active",
        "Number of currently executing plans",
        active_plans
    );
    prom!(
        counter,
        "roko_plans_completed_total",
        "Total plans completed successfully",
        s.plans_completed
    );
    prom!(
        counter,
        "roko_plans_failed_total",
        "Total plans that failed",
        s.plans_failed
    );
    prom!(
        counter,
        "roko_tasks_completed_total",
        "Total tasks completed",
        s.tasks_completed
    );
    prom!(
        counter,
        "roko_tasks_failed_total",
        "Total tasks that failed",
        s.tasks_failed
    );
    prom!(
        gauge,
        "roko_tasks_active",
        "Number of currently executing tasks",
        s.tasks_active
    );
    prom!(
        counter,
        "roko_gate_pass_total",
        "Total gate checks that passed",
        s.gates_passed
    );
    prom!(
        counter,
        "roko_gate_fail_total",
        "Total gate checks that failed",
        s.gates_failed
    );
    prom!(
        counter,
        "roko_errors_total",
        "Total error events recorded",
        s.errors_total
    );
    prom!(
        counter,
        "roko_episodes_total",
        "Total episodes recorded",
        episode_count
    );

    (
        axum::http::StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        out,
    )
}

// ── private helpers ──────────────────────────────────────────────────

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

    let projections = RuntimeProjectionSet::load(state).await?;
    let efficiency_events = projections.efficiency_events().to_vec();
    let efficiency_events: Vec<AgentEfficiencyEvent> = efficiency_events
        .into_iter()
        .filter(|event| event_time(event).is_some_and(|ts| ts >= window_start))
        .collect();

    let episodes = projections.episodes().to_vec();
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
        if !super::helpers::is_gate_result_kind(kind) {
            continue;
        }

        let Some(gate_name) = super::helpers::extract_gate_name(entry) else {
            continue;
        };
        let Some(passed) = super::helpers::extract_gate_passed(entry) else {
            continue;
        };
        let timestamp = super::helpers::entry_timestamp_ms(entry).unwrap_or_default();

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
        if super::helpers::signal_kind(entry).as_deref() != Some("gate_verdict") {
            continue;
        }
        let Some(gate_ts) = super::helpers::entry_timestamp_ms(entry) else {
            continue;
        };
        let Some(signal_id) = super::helpers::signal_id(entry) else {
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
        let Some(id) = super::helpers::signal_id(entry) else {
            continue;
        };
        let kind = super::helpers::signal_kind(entry).unwrap_or_else(|| "unknown".to_string());
        let created_at_ms = super::helpers::entry_timestamp_ms(entry).unwrap_or_default();
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

pub(super) fn ratio(numer: u64, denom: u64) -> f64 {
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
