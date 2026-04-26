//! Episodes and signals endpoints.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::Value;

use crate::error::ApiError;
use crate::projection_contract::{ProjectionQuery, RuntimeProjectionSet};
use crate::state::AppState;

use super::helpers::{MAX_JSONL_RESULTS, read_jsonl_entries};

/// `GET /api/episodes` — normalized episode proof rows from canonical projections.
pub async fn episodes(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProjectionQuery>,
) -> Result<Json<Value>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    Ok(Json(Value::Array(projections.episode_items(&query))))
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
