//! Episodes and signals endpoints.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

use crate::error::ApiError;
use crate::state::AppState;

use super::helpers::{read_jsonl_entries, MAX_JSONL_RESULTS};

/// `GET /api/episodes` — read episodes JSONL as a JSON array.
pub async fn episodes(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
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

#[derive(Deserialize)]
pub struct SignalQuery {
    pub limit: Option<usize>,
}

/// `GET /api/signals` — read signals JSONL as a JSON array, with optional `?limit=N`.
pub async fn signals(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SignalQuery>,
) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko").join("engrams.jsonl");
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
