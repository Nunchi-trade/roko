//! ISFR API endpoints — keeper status, current rates, history, sources, and
//! a topic-filtered SSE stream.
//!
//! # REST endpoints (F1)
//!
//! | Method | Path                  | Description                              |
//! |--------|-----------------------|------------------------------------------|
//! | GET    | `/isfr/status`        | Keeper status and config summary         |
//! | GET    | `/isfr/current`       | Most recent composite rate               |
//! | GET    | `/isfr/history`       | Rate history ring (newest-first, max 256)|
//! | GET    | `/isfr/sources`       | Per-source health snapshots              |
//!
//! # SSE stream (F2)
//!
//! | Method | Path                  | Description                              |
//! |--------|-----------------------|------------------------------------------|
//! | GET    | `/isfr/stream`        | Topic-filtered SSE (ISFR events only)    |
//!
//! The existing `/api/events` and `/ws` endpoints already broadcast all
//! `ServerEvent` variants (including the three ISFR ones) to subscribers.
//! This file adds a convenience filtered SSE endpoint for clients that only
//! want ISFR events.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Json;
use axum::Router;
use futures::stream::{self, Stream};
use serde::Serialize;
use tokio::sync::broadcast;
use tracing::warn;

use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::routes::sse::sse_response_headers;
use crate::state::AppState;

/// Register all ISFR API routes under `/isfr/…`.
///
/// Called from [`crate::routes::build_router`] and merged into the `/api` sub-router.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/isfr/status", get(isfr_status))
        .route("/isfr/current", get(isfr_current_rate))
        .route("/isfr/history", get(isfr_rate_history))
        .route("/isfr/sources", get(isfr_sources))
        .route("/isfr/stream", get(isfr_stream))
}

// ─── GET /api/isfr/status ─────────────────────────────────────────────────

#[derive(Serialize)]
struct ISFRStatusResponse {
    enabled: bool,
    keeper_running: bool,
    sources_count: usize,
    current_rate_bps: Option<u64>,
    poll_interval_secs: u64,
    epoch_duration_secs: u64,
}

/// `GET /api/isfr/status` — keeper enabled/running state + config summary.
async fn isfr_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ISFRStatusResponse>, ApiError> {
    // load_roko_config() is SYNC — it returns Arc<RokoConfig> immediately.
    let config = state.load_roko_config();

    let current = state.isfr.current_rate.read().await;
    let sources = state.isfr.sources.read().await;
    let running = state
        .isfr
        .keeper_running
        .load(std::sync::atomic::Ordering::Relaxed);

    // Extract composite_bps from the JSON value if present.
    let current_rate_bps = current
        .as_ref()
        .and_then(|v| v.get("composite_bps"))
        .and_then(serde_json::Value::as_u64);

    Ok(Json(ISFRStatusResponse {
        enabled: config.isfr.enabled,
        keeper_running: running,
        sources_count: sources.len(),
        current_rate_bps,
        poll_interval_secs: config.isfr.poll_interval_secs,
        epoch_duration_secs: config.isfr.epoch_duration_secs,
    }))
}

// ─── GET /api/isfr/current ────────────────────────────────────────────────

/// `GET /api/isfr/current` — most recently computed composite rate.
async fn isfr_current_rate(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let current = state.isfr.current_rate.read().await;
    match current.as_ref() {
        Some(rate) => Ok(Json(rate.clone())),
        None => Ok(Json(serde_json::json!({
            "error": "no rate computed yet",
            "hint": "start the keeper with `roko isfr start`"
        }))),
    }
}

// ─── GET /api/isfr/history?limit=N ───────────────────────────────────────

#[derive(serde::Deserialize)]
struct HistoryQuery {
    limit: Option<usize>,
}

/// `GET /api/isfr/history` — rate history ring, newest-first, up to `limit` (max 256).
async fn isfr_rate_history(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<HistoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let history = state.isfr.rate_history.read().await;
    let limit = q.limit.unwrap_or(50).min(256);
    let rates: Vec<_> = history.iter().rev().take(limit).collect();
    Ok(Json(serde_json::json!({
        "rates": rates,
        "total": history.len(),
    })))
}

// ─── GET /api/isfr/sources ────────────────────────────────────────────────

/// `GET /api/isfr/sources` — per-source health snapshots.
async fn isfr_sources(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let sources = state.isfr.sources.read().await;
    Ok(Json(serde_json::json!({ "sources": *sources })))
}

// ─── GET /api/isfr/stream (SSE) ───────────────────────────────────────────

/// `GET /api/isfr/stream` — topic-filtered SSE stream of ISFR events only.
///
/// The general `/api/events` endpoint already broadcasts every `ServerEvent`
/// variant; this endpoint is a convenience filter for clients that only care
/// about ISFR rate updates (e.g. the ISFR dashboard tile).
///
/// Forwards only the three ISFR event kinds:
/// - `isfr_rate_computed`
/// - `isfr_source_health_changed`
/// - `isfr_keeper_state_changed`
async fn isfr_stream(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let rx = state.event_bus.subscribe();
    let sse = isfr_sse_from_bus(rx);
    (sse_response_headers(), sse)
}

/// Build an SSE stream from an event-bus receiver, filtering to ISFR events.
fn isfr_sse_from_bus(
    rx: broadcast::Receiver<crate::event_bus::Envelope<ServerEvent>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(envelope) => {
                    let is_isfr = matches!(
                        &envelope.payload,
                        ServerEvent::IsfrRateComputed { .. }
                            | ServerEvent::IsfrSourceHealthChanged { .. }
                            | ServerEvent::IsfrKeeperStateChanged { .. }
                    );
                    if is_isfr {
                        let data = serde_json::to_string(&envelope.payload).unwrap_or_default();
                        return Some((Ok(Event::default().data(data)), rx));
                    }
                    // Non-ISFR event — skip and poll again.
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(n, "ISFR SSE client lagged, skipped events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keepalive"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::events::ServerEvent;

    #[test]
    fn isfr_rate_computed_is_matched_by_filter() {
        let event = ServerEvent::IsfrRateComputed {
            composite_bps: 580,
            lending_bps: 620,
            structured_bps: 540,
            funding_bps: 310,
            staking_bps: 390,
            confidence_bps: 8_500,
            source_count: 3,
            timestamp_ms: 1_700_000_000_000,
        };
        let is_isfr = matches!(
            &event,
            ServerEvent::IsfrRateComputed { .. }
                | ServerEvent::IsfrSourceHealthChanged { .. }
                | ServerEvent::IsfrKeeperStateChanged { .. }
        );
        assert!(is_isfr);
    }

    #[test]
    fn non_isfr_event_is_not_matched() {
        let event = ServerEvent::PlanStarted {
            plan_id: "p1".into(),
        };
        let is_isfr = matches!(
            &event,
            ServerEvent::IsfrRateComputed { .. }
                | ServerEvent::IsfrSourceHealthChanged { .. }
                | ServerEvent::IsfrKeeperStateChanged { .. }
        );
        assert!(!is_isfr);
    }

    #[test]
    fn isfr_source_health_changed_is_matched() {
        let event = ServerEvent::IsfrSourceHealthChanged {
            source_id: "mock-aave".into(),
            health: "stale".into(),
            last_rate_bps: Some(610),
        };
        let is_isfr = matches!(
            &event,
            ServerEvent::IsfrRateComputed { .. }
                | ServerEvent::IsfrSourceHealthChanged { .. }
                | ServerEvent::IsfrKeeperStateChanged { .. }
        );
        assert!(is_isfr);
    }

    #[test]
    fn isfr_keeper_state_changed_is_matched() {
        let event = ServerEvent::IsfrKeeperStateChanged { running: true };
        let is_isfr = matches!(
            &event,
            ServerEvent::IsfrRateComputed { .. }
                | ServerEvent::IsfrSourceHealthChanged { .. }
                | ServerEvent::IsfrKeeperStateChanged { .. }
        );
        assert!(is_isfr);
    }

    #[test]
    fn isfr_rate_computed_serializes_correctly() {
        let event = ServerEvent::IsfrRateComputed {
            composite_bps: 580,
            lending_bps: 620,
            structured_bps: 540,
            funding_bps: 310,
            staking_bps: 390,
            confidence_bps: 8_500,
            source_count: 3,
            timestamp_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_value(&event).expect("serialize");
        assert_eq!(json["type"], "isfr_rate_computed");
        assert_eq!(json["composite_bps"], 580);
        assert_eq!(json["lending_bps"], 620);
        assert_eq!(json["confidence_bps"], 8_500);
        assert_eq!(json["source_count"], 3);
    }
}
