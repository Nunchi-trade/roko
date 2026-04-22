//! Heartbeat protocol endpoints.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use roko_core::{HEARTBEAT_RING_CAPACITY, HeartbeatPayload};

use crate::events::ServerEvent;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/heartbeats", post(receive_heartbeat).get(list_heartbeats))
        .route("/network/stats", get(network_stats))
}

async fn receive_heartbeat(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<HeartbeatPayload>,
) -> axum::http::StatusCode {
    state.event_bus.publish(ServerEvent::HeartbeatReceived {
        sender_id: payload.sender_id.clone(),
        active_tasks: payload.active_tasks,
        active_agents: payload.active_agents,
    });

    let mut ring = state.heartbeats.write().await;
    if ring.len() >= HEARTBEAT_RING_CAPACITY {
        ring.pop_front();
    }
    ring.push_back(payload);

    axum::http::StatusCode::ACCEPTED
}

#[derive(Debug, Default, Deserialize)]
struct ListQuery {
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    50
}

async fn list_heartbeats(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListQuery>,
) -> Json<Vec<HeartbeatPayload>> {
    let ring = state.heartbeats.read().await;
    let limit = if query.limit == 0 { 50 } else { query.limit };
    let items: Vec<HeartbeatPayload> = ring.iter().rev().take(limit).cloned().collect();
    Json(items)
}

async fn network_stats(State(state): State<Arc<AppState>>) -> Json<Vec<roko_core::NetworkStats>> {
    let ring = state.heartbeats.read().await;
    let mut per_sender: std::collections::HashMap<String, (usize, String, f64)> =
        std::collections::HashMap::new();
    for hb in ring.iter() {
        let entry = per_sender
            .entry(hb.sender_id.clone())
            .or_insert((0, String::new(), 0.0));
        entry.0 += 1;
        if hb.timestamp > entry.1 {
            entry.1 = hb.timestamp.clone();
        }
        entry.2 += hb.active_tasks as f64;
    }
    let stats: Vec<roko_core::NetworkStats> = per_sender
        .into_iter()
        .map(
            |(sender_id, (count, last_seen, total_tasks))| roko_core::NetworkStats {
                sender_id,
                heartbeat_count: count,
                last_seen,
                avg_active_tasks: if count > 0 {
                    total_tasks / count as f64
                } else {
                    0.0
                },
            },
        )
        .collect();
    Json(stats)
}
