//! Agent HTTP endpoints for dashboard consumption.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;

use super::ApiState;

/// Trace pagination query parameters.
#[derive(Deserialize)]
pub struct TraceQuery {
    /// Maximum number of traces to return (default 10).
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Offset into the trace list (default 0).
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    10
}

/// `GET /api/agents` — list all registered agents with summary stats.
pub async fn list_agents(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let chain = state.chain.read();
    let agents: Vec<_> = chain
        .agent_registry
        .list_agents()
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "role": a.role,
                "registered_at": a.registered_at,
                "last_heartbeat_block": a.last_heartbeat_block,
                "last_heartbeat_ts": a.last_heartbeat_ts,
                "stats": a.stats,
            })
        })
        .collect();
    Json(serde_json::json!({ "agents": agents }))
}

/// `GET /api/agents/{id}/trace` — cognitive loop history for an agent.
pub async fn get_agent_trace(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(query): Query<TraceQuery>,
) -> Json<serde_json::Value> {
    let chain = state.chain.read();
    match chain.agent_registry.get_traces(&id, query.limit, query.offset) {
        Some((traces, total)) => Json(serde_json::json!({
            "agent_id": id,
            "traces": traces,
            "total": total,
        })),
        None => Json(serde_json::json!({
            "error": "agent not found",
            "agent_id": id,
        })),
    }
}

/// `GET /api/agents/{id}/heartbeat` — liveness status for an agent.
pub async fn get_agent_heartbeat(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let chain = state.chain.read();
    let timeout_blocks: u64 = 200;
    match chain.agent_registry.get_agent(&id) {
        Some(agent) => {
            let alive = chain
                .agent_registry
                .is_alive(&id, 0, timeout_blocks)
                .unwrap_or(false);
            Json(serde_json::json!({
                "agent_id": id,
                "alive": alive,
                "last_block": agent.last_heartbeat_block,
                "last_timestamp": agent.last_heartbeat_ts,
                "blocks_since": 0u64.saturating_sub(agent.last_heartbeat_block),
                "timeout_blocks": timeout_blocks,
            }))
        }
        None => Json(serde_json::json!({
            "error": "agent not found",
            "agent_id": id,
        })),
    }
}

/// `GET /api/agents/{id}/stats` — aggregated stats for an agent.
pub async fn get_agent_stats(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let chain = state.chain.read();
    match chain.agent_registry.get_agent(&id) {
        Some(agent) => Json(serde_json::json!({
            "agent_id": id,
            "confirmations_given": agent.stats.confirmations_given,
            "challenges_given": agent.stats.challenges_given,
            "warnings_posted": agent.stats.warnings_posted,
            "insights_posted": agent.stats.insights_posted,
            "delta_cycles": agent.stats.delta_cycles,
            "total_cost_usd": agent.stats.total_cost_usd,
            "total_tokens": agent.stats.total_tokens,
            "registered_at": agent.registered_at,
        })),
        None => Json(serde_json::json!({
            "error": "agent not found",
            "agent_id": id,
        })),
    }
}
