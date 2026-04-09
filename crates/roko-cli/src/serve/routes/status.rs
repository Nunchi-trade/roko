//! Status, health, metrics, dashboard, episodes, signals, and operation endpoints.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::serve::error::ApiError;
use crate::serve::state::AppState;
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use roko_learn::prompt_experiment::{ExperimentStatus, ExperimentStore};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health))
        .route("/status", get(session_status))
        .route("/metrics", get(metrics))
        .route("/metrics/summary", get(metrics_summary))
        .route("/dashboard", get(dashboard))
        .route("/gates/summary", get(gate_summary))
        .route("/gates/{gate_name}/history", get(gate_history))
        .route("/episodes", get(episodes))
        .route("/signals", get(signals))
        .route("/operations/{id}", get(operation_status))
}

/// `GET /api/health` — liveness check.
async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    let uptime_secs = state.started_at.elapsed().as_secs();
    let active_plans = state.active_plans.read().await.len();
    let active_agents = state.supervisor.count().await;

    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": uptime_secs,
        "active_plans": active_plans,
        "active_agents": active_agents,
    }))
}

/// `GET /api/status` — session status overview.
async fn session_status(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let ss = crate::status::SessionStatus::offline(state.workdir.clone());
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
    Ok(Json(
        serde_json::to_value(summary)
            .map_err(|e| ApiError::internal(format!("serialize metrics summary: {e}")))?,
    ))
}

/// `GET /api/dashboard` — dashboard scaffold as JSON.
async fn dashboard(State(state): State<Arc<AppState>>) -> Json<Value> {
    let scaffold = crate::tui::DashboardScaffold::new_in(&state.workdir);
    let text = format!("{scaffold:?}");
    Json(json!({ "rendered": text }))
}

/// `GET /api/episodes` — read episodes JSONL as a JSON array.
async fn episodes(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.layout.episodes_path();
    read_jsonl_array(&path).await
}

/// `GET /api/gates/summary` — aggregate gate verdicts from `.roko/signals.jsonl`.
async fn gate_summary(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko").join("signals.jsonl");
    let entries = read_jsonl_entries(&path).await?;
    Ok(Json(summarize_gate_entries(&entries)))
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
    let limited = match q.limit {
        Some(n) => entries
            .into_iter()
            .rev()
            .take(n)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
        None => entries,
    };
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
            ApiError::internal(format!("parse {} line {}: {e}", path.display(), line_no + 1))
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

#[derive(Debug, Default)]
struct RunAggregate {
    runs: u64,
    successes: u64,
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

    let agents_run = efficiency_events.len() as u64;
    let success_count = efficiency_events.iter().filter(|event| event.gate_passed).count() as u64;
    let success_rate = ratio(success_count, agents_run);
    let avg_cost_per_episode_cents = if agents_run == 0 {
        0
    } else {
        let total_cost_usd: f64 = efficiency_events.iter().map(|event| event.cost_usd).sum();
        ((total_cost_usd / agents_run as f64) * 100.0).round().max(0.0) as u64
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
        top_templates,
    })
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
            ApiError::internal(format!("parse {} line {}: {e}", path.display(), line_no + 1))
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

    for experiment in store.iter().filter(|experiment| experiment.status == ExperimentStatus::Running) {
        let mut active_variants: Vec<_> = experiment
            .variants
            .iter()
            .filter(|variant| variant.active)
            .map(|variant| {
                let stats = experiment.stats.get(&variant.id).cloned().unwrap_or_default();
                (
                    variant.name.clone(),
                    stats.trials,
                    stats.success_rate(),
                )
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
        for record in records.iter().filter(|record| record.timestamp >= window_start) {
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
            .then_with(|| b.success_rate.partial_cmp(&a.success_rate).unwrap_or(std::cmp::Ordering::Equal))
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
        .or_else(|| {
            entry
                .pointer("/body/data/passed")
                .and_then(Value::as_bool)
        })
        .or_else(|| entry.pointer("/body/passed").and_then(Value::as_bool))
}

fn extract_gate_duration_ms(entry: &Value) -> Option<u64> {
    entry
        .pointer("/body/data/duration_ms")
        .and_then(Value::as_u64)
        .or_else(|| entry.pointer("/body/duration_ms").and_then(Value::as_u64))
        .or_else(|| entry.pointer("/tags/duration_ms").and_then(Value::as_u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use axum::extract::{Path, Query, State};
    use tempfile::tempdir;

    use crate::config::Config;
    use crate::serve::deploy::create_backend;
    use crate::serve::state::{AppState, OperationStatus, PlanHandle};
    use roko_core::{Body, Kind, Provenance, Signal, Verdict};

    fn gate_signal(gate: &str, passed: bool, duration_ms: u64) -> Value {
        let mut verdict = if passed {
            Verdict::pass(gate)
        } else {
            Verdict::fail(gate, "boom")
        };
        verdict.duration_ms = duration_ms;

        let signal = Signal::builder(Kind::GateVerdict)
            .body(Body::from_json(&verdict).unwrap())
            .provenance(Provenance::trusted("test"))
            .tag("gate", gate)
            .tag("passed", passed.to_string())
            .build();
        serde_json::to_value(signal).unwrap()
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
        compile_late.created_at_ms = 20;
        let mut compile_early = gate_signal("compile", true, 100);
        compile_early.created_at_ms = 10;
        let mut test = gate_signal("test", true, 200);
        test.created_at_ms = 15;

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
    async fn health_reports_runtime_counts() {
        let (_dir, state) = test_state();

        let plan_handle = PlanHandle {
            id: "plan-1".into(),
            plan_dir: std::path::PathBuf::from("/tmp/plan-1"),
            status: OperationStatus::Running,
            handle: tokio::spawn(async {}),
        };
        state
            .active_plans
            .write()
            .await
            .insert(plan_handle.id.clone(), plan_handle);

        let _agent_id = state
            .supervisor
            .spawn(bardo_runtime::process::SpawnConfig {
                program: "sleep".into(),
                args: vec!["60".into()],
                label: "health-test-agent".into(),
                ..Default::default()
            })
            .await
            .expect("spawn test agent");

        let response = health(State(state.clone())).await;
        let body = response.0;

        assert_eq!(body["status"], "ok");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert!(body["uptime_secs"].as_u64().is_some());
        assert_eq!(body["active_plans"].as_u64(), Some(1));
        assert_eq!(body["active_agents"].as_u64(), Some(1));

        state.supervisor.shutdown_all().await;
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
        let signals = dir.path().join(".roko").join("signals.jsonl");
        tokio::fs::create_dir_all(signals.parent().expect("signals parent"))
            .await
            .expect("create signals dir");
        tokio::fs::write(&signals, "{not-json}\n")
            .await
            .expect("write corrupt signals");

        let err = signals(State(state), Query(SignalQuery { limit: Some(1) }))
            .await
            .expect_err("corrupt signals should fail");

        assert_eq!(err.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }
}
