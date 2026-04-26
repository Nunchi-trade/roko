//! Verify summary and history endpoints.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::state::AppState;

use super::helpers::{
    MAX_JSONL_RESULTS, extract_gate_duration_ms, extract_gate_name,
    extract_gate_passed, extract_gate_rung, is_gate_result_kind, read_jsonl_entries,
};
use super::metrics::ratio;

/// `GET /api/gates/summary` — aggregate gate verdicts from `.roko/engrams.jsonl`.
pub async fn gate_summary(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko").join("engrams.jsonl");
    let entries = read_jsonl_entries(&path).await?;
    let mut summary = summarize_gate_entries(&entries);
    if let Some(obj) = summary.as_object_mut() {
        obj.insert("rungs".to_string(), json!(summarize_gate_rungs(&entries)));
    }
    Ok(Json(summary))
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
    let path = state.workdir.join(".roko").join("engrams.jsonl");
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
pub async fn gate_history(
    State(state): State<Arc<AppState>>,
    Path(gate_name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko").join("engrams.jsonl");
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

// ── private helpers ──────────────────────────────────────────────────

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
