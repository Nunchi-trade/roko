//! Status, health, metrics, dashboard, episodes, signals, and operation endpoints.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use serde::Deserialize;
use std::collections::BTreeMap;
use serde_json::{Value, json};

use crate::serve::error::ApiError;
use crate::serve::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health))
        .route("/status", get(session_status))
        .route("/metrics", get(metrics))
        .route("/dashboard", get(dashboard))
        .route("/gates/summary", get(gate_summary))
        .route("/episodes", get(episodes))
        .route("/signals", get(signals))
        .route("/operations/{id}", get(operation_status))
}

/// `GET /api/health` — liveness check.
async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
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
    let entries: Vec<Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    Ok(entries)
}

/// Read a JSONL file and return the entries as a `Json<Value::Array>`.
async fn read_jsonl_array(path: &std::path::Path) -> Result<Json<Value>, ApiError> {
    let entries = read_jsonl_entries(path).await?;
    Ok(Json(Value::Array(entries)))
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
}
