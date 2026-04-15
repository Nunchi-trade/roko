//! Research route.

use std::sync::Arc;

use axum::{Json, Router, extract::State, routing::post};

use crate::state::{AgentState, ResearchRequest, ResearchResponse};

/// Research routes.
pub fn router() -> Router<Arc<AgentState>> {
    Router::new().route("/research", post(research))
}

async fn research(
    State(state): State<Arc<AgentState>>,
    Json(request): Json<ResearchRequest>,
) -> Json<ResearchResponse> {
    Json(state.research(request).await)
}
