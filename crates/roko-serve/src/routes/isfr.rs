//! ISFR REST API endpoints — keeper status, current rate, history, sources.
//!
//! Endpoints:
//!   GET /api/isfr/status   — keeper running flag, config params, counts
//!   GET /api/isfr/current  — most recent composite rate (or 204-style JSON hint)
//!   GET /api/isfr/history  — bounded ring of historical rates (?limit=N, max 256)
//!   GET /api/isfr/sources  — per-source health snapshots

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Query, State};
use axum::routing::get;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;

/// Register all ISFR routes. Called from `build_router()` via `.merge(isfr::routes())`.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/isfr/status", get(isfr_status))
        .route("/isfr/current", get(isfr_current_rate))
        .route("/isfr/history", get(isfr_rate_history))
        .route("/isfr/sources", get(isfr_sources))
}

// ─── GET /api/isfr/status ────────────────────────────────────────────────────

#[derive(Serialize)]
struct ISFRStatusResponse {
    /// Whether ISFR features are enabled in roko.toml.
    enabled: bool,
    /// Whether the keeper background task is currently running.
    keeper_running: bool,
    /// Number of source health entries tracked.
    sources_count: usize,
    /// Most recent composite rate in basis points (null when no rate yet).
    current_rate_bps: Option<u64>,
    /// Confidence as a 0.0–1.0 fraction (null when no rate yet).
    current_confidence: Option<f64>,
    /// Source poll interval from config (seconds).
    poll_interval_secs: u64,
    /// Epoch duration from config (seconds).
    epoch_duration_secs: u64,
}

async fn isfr_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ISFRStatusResponse>, ApiError> {
    // load_roko_config() is sync — returns Arc<RokoConfig>.
    let config = state.load_roko_config();

    let current = state.isfr.current_rate.read().await;
    let sources = state.isfr.sources.read().await;
    let running = state
        .isfr
        .keeper_running
        .load(std::sync::atomic::Ordering::Relaxed);

    Ok(Json(ISFRStatusResponse {
        enabled: config.isfr.enabled,
        keeper_running: running,
        sources_count: sources.len(),
        current_rate_bps: current.as_ref().map(|r| r.composite_bps),
        // confidence_bps is 0–10000 (basis points of confidence), convert to 0.0–1.0
        current_confidence: current.as_ref().map(|r| r.confidence_bps as f64 / 10_000.0),
        poll_interval_secs: config.isfr.poll_interval_secs,
        epoch_duration_secs: config.isfr.epoch_duration_secs,
    }))
}

// ─── GET /api/isfr/current ───────────────────────────────────────────────────

async fn isfr_current_rate(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let current = state.isfr.current_rate.read().await;
    match current.as_ref() {
        Some(rate) => Ok(Json(
            serde_json::to_value(rate).unwrap_or(serde_json::Value::Null),
        )),
        None => Ok(Json(serde_json::json!({
            "error": "no rate computed yet",
            "hint": "start the keeper with `roko isfr start`"
        }))),
    }
}

// ─── GET /api/isfr/history?limit=N ──────────────────────────────────────────

#[derive(Deserialize)]
struct HistoryQuery {
    /// Maximum number of history entries to return (default: 50, cap: 256).
    limit: Option<usize>,
}

async fn isfr_rate_history(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let history = state.isfr.rate_history.read().await;
    let limit = q.limit.unwrap_or(50).min(256);
    // Most recent first: iterate in reverse, collect up to `limit` entries.
    let rates: Vec<_> = history.iter().rev().take(limit).collect();
    // Return the array directly so the frontend can call history.map(...).
    Ok(Json(serde_json::to_value(&rates).unwrap_or(serde_json::Value::Array(vec![]))))
}

// ─── GET /api/isfr/sources ───────────────────────────────────────────────────

async fn isfr_sources(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let sources = state.isfr.sources.read().await;
    // Return the array directly so the frontend can call sources.map(...).
    Ok(Json(serde_json::to_value(&*sources).unwrap_or(serde_json::Value::Array(vec![]))))
}
