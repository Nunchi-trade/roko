//! Task queue routes.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};

use crate::state::{AgentState, TaskCompletionRequest};

/// Task routes.
pub fn router() -> Router<Arc<AgentState>> {
    Router::new()
        .route("/tasks", get(list_tasks))
        .route("/tasks/{id}/accept", post(accept_task))
        .route("/tasks/{id}/complete", post(complete_task))
}

async fn list_tasks(State(state): State<Arc<AgentState>>) -> Json<Vec<crate::state::TaskEntry>> {
    Json(state.list_tasks().await)
}

async fn accept_task(
    State(state): State<Arc<AgentState>>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match state.accept_task(id).await {
        Some(task) => (StatusCode::OK, Json(task)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn complete_task(
    State(state): State<Arc<AgentState>>,
    Path(id): Path<u64>,
    Json(request): Json<TaskCompletionRequest>,
) -> impl IntoResponse {
    match state.complete_task(id, request).await {
        Some(task) => (StatusCode::OK, Json(task)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
