//! Dashboard, session status, operation status, and truth map endpoints.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::state::AppState;

/// `GET /api/dashboard` — dashboard scaffold as JSON.
pub async fn dashboard(State(state): State<Arc<AppState>>) -> Json<Value> {
    let info = state.runtime.dashboard_scaffold(&state.workdir);
    Json(json!({ "rendered": info.rendered }))
}

/// `GET /api/status` — session status overview.
pub async fn session_status(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
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

/// `GET /api/operations/:id` — look up a background operation by ID.
pub async fn operation_status(
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

/// `GET /api/truth_map` — return the entity truth-source registry.
pub async fn truth_map_handler() -> Json<Value> {
    Json(serde_json::to_value(crate::truth_map::truth_map()).unwrap_or_default())
}
