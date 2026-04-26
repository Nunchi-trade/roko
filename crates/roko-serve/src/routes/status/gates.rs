//! Verify summary and history endpoints.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::projection_contract::{ProjectionQuery, RuntimeProjectionSet};
use crate::state::AppState;

use super::helpers::{
    MAX_JSONL_RESULTS, extract_gate_duration_ms, extract_gate_name, extract_gate_passed,
    extract_gate_rung, is_gate_result_kind, read_jsonl_entries,
};
use super::metrics::ratio;

/// `GET /api/gates/summary` — aggregate gate verdicts from canonical projections.
pub async fn gate_summary(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    Ok(Json(projections.gate_summary(&ProjectionQuery::default())))
}

#[derive(Debug, Deserialize, Default)]
pub struct GateHistoryQuery {
    #[serde(default)]
    gate: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// `GET /api/gates/history` — recent gate verdicts across all gates.
pub async fn gates_history(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GateHistoryQuery>,
) -> Result<Json<Value>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    let query = ProjectionQuery {
        gate: query.gate,
        limit: query.limit.map(|limit| limit.min(MAX_JSONL_RESULTS)),
        ..ProjectionQuery::default()
    };
    Ok(Json(projections.gate_history(&query)))
}

/// `GET /api/gates/:gate_name/history` — time series of pass/fail results for one gate.
pub async fn gate_history(
    State(state): State<Arc<AppState>>,
    Path(gate_name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    let query = ProjectionQuery {
        gate: Some(gate_name.clone()),
        limit: Some(MAX_JSONL_RESULTS),
        ..ProjectionQuery::default()
    };
    let history = projections.gate_history(&query);
    let total = history.get("total").and_then(Value::as_u64).unwrap_or(0);
    if total == 0 {
        return Err(ApiError::not_found(format!("gate '{gate_name}' not found")));
    }

    Ok(Json(history))
}

// ── private helpers ──────────────────────────────────────────────────

async fn read_gate_entries(state: &AppState) -> Result<Vec<Value>, ApiError> {
    let mut entries =
        read_jsonl_entries(&state.workdir.join(".roko").join("engrams.jsonl")).await?;
    let runner_events =
        read_jsonl_entries(&state.workdir.join(".roko").join("events.jsonl")).await?;
    entries.extend(runner_events.iter().flat_map(runner_gate_entries));
    Ok(entries)
}

fn gate_sources(state: &AppState) -> Vec<String> {
    [".roko/engrams.jsonl", ".roko/events.jsonl"]
        .into_iter()
        .map(|path| state.workdir.join(path).display().to_string())
        .collect()
}

fn runner_gate_entries(event: &Value) -> Vec<Value> {
    if event.get("type").and_then(Value::as_str) != Some("gate.completed") {
        return Vec::new();
    }

    let plan_id = event.get("plan_id").cloned().unwrap_or(Value::Null);
    let task_id = event.get("task_id").cloned().unwrap_or(Value::Null);
    let rung = event.get("rung").cloned().unwrap_or(Value::Null);
    let duration_ms = event.get("duration_ms").cloned().unwrap_or(Value::Null);
    let timestamp = event.get("timestamp").cloned().unwrap_or(Value::Null);

    event
        .get("verdicts")
        .and_then(Value::as_array)
        .map(|verdicts| {
            verdicts
                .iter()
                .map(|verdict| {
                    let gate = verdict.get("gate").cloned().unwrap_or(Value::Null);
                    let passed = verdict.get("passed").cloned().unwrap_or(Value::Null);
                    json!({
                        "id": Value::Null,
                        "created_at_ms": Value::Null,
                        "timestamp": timestamp,
                        "kind": "gate_verdict",
                        "tags": {
                            "gate": gate,
                            "passed": passed.as_bool().map(|value| value.to_string()).unwrap_or_default(),
                            "plan_id": plan_id,
                            "task_id": task_id,
                            "rung": rung,
                            "duration_ms": duration_ms,
                        },
                        "body": {
                            "data": {
                                "gate": gate,
                                "passed": passed,
                                "plan_id": plan_id,
                                "task_id": task_id,
                                "rung": rung,
                                "duration_ms": duration_ms,
                                "summary": verdict.get("summary").cloned().unwrap_or(Value::Null),
                            }
                        }
                    })
                })
                .collect()
        })
        .unwrap_or_default()
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
