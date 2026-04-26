//! Health, relay, parity, retention, and state-hub snapshot endpoints.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde_json::{Value, json};

use crate::state::AppState;

/// `GET /api/health` — liveness check with live telemetry.
pub async fn health(State(state): State<Arc<AppState>>) -> (axum::http::StatusCode, Json<Value>) {
    let uptime_secs = state.started_at.elapsed().as_secs();
    let active_plans = state.active_plans.read().await.len();
    // Use discovered agents count (includes both local and remote agents).
    let supervised = state.supervisor.count().await;
    let discovered = state.discovered_agents.read().await.len();
    let active_agents = supervised.max(discovered);
    let active_runs = state.active_runs.read().await.len();

    // Build a compact provider health summary from the tracker.
    let provider_snapshot = state.provider_health.snapshot();
    let providers_total = provider_snapshot.len();
    let providers_healthy = provider_snapshot
        .iter()
        .filter(|ps| ps.consecutive_failures == 0)
        .count();
    let providers_unhealthy = providers_total.saturating_sub(providers_healthy);
    let provider_summary = json!({
        "total": providers_total,
        "healthy": providers_healthy,
        "unhealthy": providers_unhealthy,
    });

    // Determine status: "ok" / "degraded" / "down"
    let status = if providers_total > 0 && providers_healthy == 0 {
        "down"
    } else if providers_unhealthy > 0 {
        "degraded"
    } else {
        "ok"
    };

    (
        axum::http::StatusCode::OK,
        Json(json!({
            "status": status,
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_secs": uptime_secs,
            "active_plans": active_plans,
            "active_agents": active_agents,
            "active_runs": active_runs,
            "providers": provider_summary,
        })),
    )
}

/// `GET /api/relay/health` — return relay connection diagnostics.
pub async fn relay_health(State(state): State<Arc<AppState>>) -> Json<Value> {
    let health = state.relay_health.read().clone();
    Json(serde_json::to_value(&health).unwrap_or_default())
}

/// `GET /api/parity` — return cross-surface parity matrix.
pub async fn parity_handler() -> Json<Value> {
    let matrix = crate::parity::build_parity_matrix();
    Json(serde_json::to_value(&matrix).unwrap_or_default())
}

/// `GET /api/retention` — return retention policies and any current violations.
pub async fn retention_handler(State(state): State<Arc<AppState>>) -> Json<Value> {
    let policies = crate::retention::default_retention_policies();
    let violations = crate::retention::check_retention(&state.workdir);
    let status = crate::retention::RetentionStatus {
        policies,
        violations,
    };
    Json(serde_json::to_value(&status).unwrap_or_default())
}

/// `GET /api/statehub/snapshot` — return the current state-hub dashboard snapshot.
pub async fn statehub_snapshot(State(state): State<Arc<AppState>>) -> Json<Value> {
    let snapshot = state.state_hub.current_snapshot();
    Json(serde_json::to_value(&snapshot).unwrap_or_default())
}
