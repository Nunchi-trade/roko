//! Agent HTTP endpoints for dashboard consumption.

use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::Deserialize;

use super::{ApiError, ApiState, MAX_LIMIT, PaginatedResponse, now_secs, with_cache_control};
use crate::chain::agent::{AgentStats, AgentTrace};

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
pub async fn list_agents(State(state): State<ApiState>) -> impl IntoResponse {
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
    let total = agents.len();
    with_cache_control(PaginatedResponse::new(agents, total, 0, total), 2)
}

/// `GET /api/agents/{id}/trace` — cognitive loop history for an agent.
pub async fn get_agent_trace(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(query): Query<TraceQuery>,
) -> Json<serde_json::Value> {
    let limit = query.limit.min(MAX_LIMIT);
    let chain = state.chain.read();
    match chain.agent_registry.get_traces(&id, limit, query.offset) {
        Some((traces, total)) => {
            let has_more = query.offset + traces.len() < total;
            Json(serde_json::json!({
                "agent_id": id,
                "items": traces,
                "total": total,
                "offset": query.offset,
                "limit": limit,
                "has_more": has_more,
            }))
        }
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
    let current_block = (state.current_block)();
    let chain = state.chain.read();
    let timeout_blocks: u64 = 200;
    match chain.agent_registry.get_agent(&id) {
        Some(agent) => {
            let alive = chain
                .agent_registry
                .is_alive(&id, current_block, timeout_blocks)
                .unwrap_or(false);
            Json(serde_json::json!({
                "agent_id": id,
                "alive": alive,
                "last_block": agent.last_heartbeat_block,
                "last_timestamp": agent.last_heartbeat_ts,
                "blocks_since": current_block.saturating_sub(agent.last_heartbeat_block),
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

// ---------------------------------------------------------------------------
// POST /api/agents — register a new agent
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RegisterAgentRequest {
    pub id: String,
    #[serde(default)]
    pub pubkey: String,
    pub role: String,
}

/// `POST /api/agents` — register a new agent.
pub async fn register_agent(
    State(state): State<ApiState>,
    Json(req): Json<RegisterAgentRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if req.id.is_empty() {
        return Err(ApiError {
            error: "agent id must not be empty".into(),
            code: 400,
        });
    }
    if req.role.is_empty() {
        return Err(ApiError {
            error: "agent role must not be empty".into(),
            code: 400,
        });
    }
    let address = req.pubkey.into_bytes();
    let now = now_secs();

    let mut chain = state.chain.write();
    let registered = chain
        .agent_registry
        .register(req.id.clone(), address, req.role.clone(), now);

    if registered {
        let _ = chain.agent_bus.send(crate::chain::AgentEvent::Registered {
            agent_id: req.id.clone(),
            role: req.role.clone(),
        });
    }

    if registered {
        Ok(Json(serde_json::json!({
            "registered": true,
            "agent_id": req.id,
            "role": req.role,
            "registered_at": now,
        })))
    } else {
        Err(ApiError {
            error: format!("agent '{}' already registered", req.id),
            code: 409,
        })
    }
}

// ---------------------------------------------------------------------------
// POST /api/agents/:id/heartbeat
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    #[serde(default)]
    pub tokens_used: u64,
    #[serde(default)]
    pub cost_usd: f64,
}

/// `POST /api/agents/:id/heartbeat` — send a heartbeat for an agent.
pub async fn agent_heartbeat(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let now = now_secs();
    let current_block = (state.current_block)();
    let mut chain = state.chain.write();

    let ok = chain.agent_registry.heartbeat(&id, current_block, now);
    if !ok {
        return Err(ApiError {
            error: format!("agent '{}' not found", id),
            code: 404,
        });
    }

    if req.tokens_used > 0 || req.cost_usd > 0.0 {
        let delta = AgentStats {
            total_tokens: req.tokens_used,
            total_cost_usd: req.cost_usd,
            ..AgentStats::default()
        };
        chain.agent_registry.add_stats_delta(&id, &delta);
    }

    let _ = chain.agent_bus.send(crate::chain::AgentEvent::Heartbeat {
        agent_id: id.clone(),
        block: current_block,
        timestamp: now,
    });

    Ok(Json(serde_json::json!({
        "ok": true,
        "agent_id": id,
        "block": current_block,
        "timestamp": now,
    })))
}

// ---------------------------------------------------------------------------
// POST /api/agents/:id/trace
// ---------------------------------------------------------------------------

/// `POST /api/agents/:id/trace` — record a cognitive trace for an agent.
pub async fn post_agent_trace(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(mut trace): Json<AgentTrace>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if trace.timestamp == 0 {
        trace.timestamp = now_secs();
    }
    let mut chain = state.chain.write();
    let ok = chain.agent_registry.add_trace(&id, trace);
    if !ok {
        return Err(ApiError {
            error: format!("agent '{}' not found", id),
            code: 404,
        });
    }

    let _ = chain.agent_bus.send(crate::chain::AgentEvent::Trace {
        agent_id: id.clone(),
        trace: chain
            .agent_registry
            .get_traces(&id, 1, 0)
            .and_then(|(t, _)| t.last().cloned())
            .unwrap_or_else(|| crate::chain::agent::AgentTrace {
                cycle: 0,
                phase: crate::chain::agent::CognitivePhase::Act,
                reads: vec![],
                reasoning: String::new(),
                action: String::new(),
                action_id: String::new(),
                timestamp: now_secs(),
            }),
    });

    Ok(Json(serde_json::json!({
        "ok": true,
        "agent_id": id,
    })))
}
