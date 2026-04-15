//! Health, capability, and stats routes.

use std::sync::Arc;

use axum::{Json, Router, extract::State, routing::get};

use crate::state::AgentState;

/// Public health and capability routes.
pub fn router() -> Router<Arc<AgentState>> {
    Router::new()
        .route("/health", get(health))
        .route("/capabilities", get(capabilities))
}

/// `GET /health`
pub async fn health(State(state): State<Arc<AgentState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "agent_id": state.agent_id(),
        "uptime_s": state.started_at().elapsed().as_secs(),
    }))
}

/// `GET /capabilities`
pub async fn capabilities(State(state): State<Arc<AgentState>>) -> Json<serde_json::Value> {
    Json(state.capabilities_manifest())
}

/// `GET /stats`
pub async fn stats(State(state): State<Arc<AgentState>>) -> Json<serde_json::Value> {
    Json(state.stats_payload())
}
