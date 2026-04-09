//! Agent process management endpoints.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};

use bardo_runtime::process::ProcessId;

use crate::serve::error::ApiError;
use crate::serve::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/agents", get(list_agents))
        .route("/agents/{id}", get(get_agent))
        .route("/agents/{id}/stop", post(stop_agent))
        .route("/agents/{id}/episodes", get(agent_episodes))
}

/// `GET /api/agents` — list all managed agent processes.
async fn list_agents(State(state): State<Arc<AppState>>) -> Json<Value> {
    let entries = state.supervisor.list().await;
    let items: Vec<Value> = entries
        .into_iter()
        .map(|(id, label)| {
            json!({
                "id": id.0,
                "label": label,
            })
        })
        .collect();
    Json(Value::Array(items))
}

/// `GET /api/agents/:id` — get info about a specific agent process.
async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    let entries = state.supervisor.list().await;
    let found = entries.into_iter().find(|(pid, _)| pid.0 == id);

    match found {
        Some((pid, label)) => Ok(Json(json!({
            "id": pid.0,
            "label": label,
        }))),
        None => Err(ApiError::not_found(format!("agent {id} not found"))),
    }
}

/// `POST /api/agents/:id/stop` — shut down a specific agent process.
async fn stop_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    state.supervisor.shutdown(ProcessId(id)).await.map_or_else(
        || Err(ApiError::not_found(format!("agent {id} not found"))),
        |o| {
            Ok(Json(json!({
                "id": id,
                "outcome": format!("{o:?}"),
            })))
        },
    )
}

/// `GET /api/agents/:id/episodes` — filter episodes for a specific agent.
async fn agent_episodes(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    let path = state.layout.episodes_path();
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Json(json!([]))),
        Err(e) => {
            return Err(ApiError::internal(format!("read episodes: {e}")));
        }
    };

    let agent_id_str = id.to_string();
    let mut filtered: Vec<Value> = Vec::new();
    for (line_no, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str::<Value>(line)
            .map_err(|e| ApiError::internal(format!("parse episodes line {}: {e}", line_no + 1)))?;
        if value
            .get("agent_id")
            .is_some_and(|a| a.as_str() == Some(&agent_id_str) || a.as_u64() == Some(id))
        {
            filtered.push(value);
        }
    }

    Ok(Json(Value::Array(filtered)))
}
