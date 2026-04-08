//! HTTP REST API for dashboard consumption (Issue #1).
//!
//! Provides JSON endpoints that expose pheromone field, knowledge graph, and
//! agent topology data for the kauri dashboard. These complement the existing
//! JSON-RPC surface with REST semantics optimized for UI consumption:
//!
//! - Pagination, filtering, and sorting on all list endpoints
//! - Decay projections and time-bucketed heatmap data for animated visualizations
//! - Force-directed graph data (nodes + edges) for d3.js/force-graph layouts
//! - Agent interaction topology derived from knowledge store confirmations
//! - WebSocket streaming of real-time pheromone and insight events (roko feature)
//!
//! # Endpoint summary
//!
//! | Method | Path                    | Description                                |
//! |--------|-------------------------|--------------------------------------------|
//! | GET    | `/api/pheromones`       | List active pheromones (filter/sort/page)  |
//! | GET    | `/api/pheromones/summary` | Aggregate stats per kind                 |
//! | POST   | `/api/pheromones/query` | Top-K by HDC similarity × intensity        |
//! | GET    | `/api/pheromones/heatmap` | Time-bucketed deposit activity           |
//! | GET    | `/api/knowledge/entries`| List insight entries (filter/sort/page)    |
//! | GET    | `/api/knowledge/edges`  | Dependency + HDC similarity edges          |
//! | GET    | `/api/knowledge/search` | Semantic search over knowledge store       |
//! | GET    | `/api/knowledge/kinds`  | Enumerate knowledge + pheromone kinds      |
//! | GET    | `/api/agents/topology`  | Agent interaction graph (nodes + edges)    |
//! | GET    | `/api/stats`            | Combined dashboard statistics              |
//! | WS     | `/api/ws`               | Live event stream (roko feature)           |

use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use parking_lot::RwLock;
use serde::Serialize;
use std::sync::Arc;

use crate::chain_rpc::ChainContext;

pub mod agent;
pub mod knowledge;
pub mod pheromone;
pub mod topology;
#[cfg(feature = "roko")]
pub mod ws;

/// Shared state for HTTP API handlers.
#[derive(Clone)]
pub struct ApiState {
    /// Chain context holding the knowledge store and pheromone field.
    pub chain: Arc<RwLock<ChainContext>>,
    /// Subscription manager for WebSocket streaming (roko feature only).
    #[cfg(feature = "roko")]
    pub subs: Option<crate::chain_rpc::SubscriptionManager>,
}

/// Builds the `/api` router with all dashboard endpoints.
#[must_use]
pub fn build_router(state: ApiState) -> Router {
    let router = Router::new()
        // Pheromone field
        .route("/pheromones", get(pheromone::list_pheromones))
        .route("/pheromones/summary", get(pheromone::pheromone_summary))
        .route("/pheromones/query", post(pheromone::query_pheromones))
        .route("/pheromones/heatmap", get(pheromone::pheromone_heatmap))
        // Knowledge graph
        .route("/knowledge/entries", get(knowledge::list_entries))
        .route("/knowledge/edges", get(knowledge::list_edges))
        .route("/knowledge/search", get(knowledge::search_knowledge))
        .route("/knowledge/kinds", get(knowledge::list_kinds))
        // Agent topology
        .route("/agents/topology", get(topology::agent_topology))
        // Agent registry
        .route("/agents", get(agent::list_agents))
        .route("/agents/{id}/trace", get(agent::get_agent_trace))
        .route("/agents/{id}/heartbeat", get(agent::get_agent_heartbeat))
        .route("/agents/{id}/stats", get(agent::get_agent_stats))
        // Combined stats
        .route("/stats", get(combined_stats));

    #[cfg(feature = "roko")]
    let router = router.route("/ws", get(ws::ws_handler));

    router.with_state(state)
}

/// Current timestamp in seconds since UNIX epoch.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// JSON error response returned by API endpoints.
#[derive(Serialize)]
pub struct ApiError {
    /// Human-readable error message.
    pub error: String,
    /// HTTP status code.
    pub code: u16,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(self)).into_response()
    }
}

fn default_limit() -> usize {
    100
}

/// Combined dashboard statistics.
async fn combined_stats(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let now = now_secs();
    let chain = state.chain.read();

    let insight_count = chain.knowledge.len();
    let pheromone_count = chain.pheromones.len();

    let mut threat_count = 0usize;
    let mut opportunity_count = 0usize;
    let mut wisdom_count = 0usize;
    let mut total_intensity = 0.0f64;
    for p in chain.pheromones.iter() {
        match p.kind {
            crate::chain::PheromoneKind::Threat => threat_count += 1,
            crate::chain::PheromoneKind::Opportunity => opportunity_count += 1,
            crate::chain::PheromoneKind::Wisdom => wisdom_count += 1,
        }
        total_intensity += p.current_intensity(now) as f64;
    }

    // Knowledge state breakdown
    let mut active = 0usize;
    let mut confirmed = 0usize;
    let mut challenged = 0usize;
    let mut decaying = 0usize;
    for entry in chain.knowledge.entries() {
        match entry.state {
            crate::chain::KnowledgeState::Active => active += 1,
            crate::chain::KnowledgeState::Confirmed => confirmed += 1,
            crate::chain::KnowledgeState::Challenged => challenged += 1,
            crate::chain::KnowledgeState::Decaying => decaying += 1,
            _ => {}
        }
    }

    Json(serde_json::json!({
        "insights": {
            "total": insight_count,
            "active": active,
            "confirmed": confirmed,
            "challenged": challenged,
            "decaying": decaying,
        },
        "pheromones": {
            "total": pheromone_count,
            "threat": threat_count,
            "opportunity": opportunity_count,
            "wisdom": wisdom_count,
            "total_intensity": total_intensity,
        },
        "toggles": {
            "hdc": chain.toggles.hdc,
            "knowledge": chain.toggles.knowledge,
            "stigmergy": chain.toggles.stigmergy,
        },
        "timestamp": now,
    }))
}
