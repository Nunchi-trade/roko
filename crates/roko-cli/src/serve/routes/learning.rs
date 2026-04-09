//! Learning data endpoints — efficiency, cascade router, experiments, gate thresholds.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value;

use crate::serve::error::ApiError;
use crate::serve::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/learning/efficiency", get(efficiency))
        .route("/learning/cascade-router", get(cascade_router))
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
