//! RuntimeEvent ingest endpoints for out-of-process roko commands.

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use roko_core::RuntimeEvent;
use roko_core::foundation::EventConsumer;

use crate::error::ApiError;
use crate::extract::ApiJson;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/events/ingest", post(ingest_event))
        .route("/events/ingest/batch", post(ingest_event_batch))
}

async fn ingest_event(
    State(state): State<Arc<AppState>>,
    ApiJson(event): ApiJson<RuntimeEvent>,
) -> Result<StatusCode, ApiError> {
    ensure_ingest_allowed(&state)?;
    consume_runtime_event(&state, &event);
    Ok(StatusCode::ACCEPTED)
}

async fn ingest_event_batch(
    State(state): State<Arc<AppState>>,
    ApiJson(events): ApiJson<Vec<RuntimeEvent>>,
) -> Result<StatusCode, ApiError> {
    ensure_ingest_allowed(&state)?;
    for event in events {
        consume_runtime_event(&state, &event);
    }
    Ok(StatusCode::ACCEPTED)
}

fn ensure_ingest_allowed(state: &AppState) -> Result<(), ApiError> {
    let config = state.load_roko_config();
    if super::bind_is_loopback(&config.server.bind) || config.serve.auth.enabled {
        return Ok(());
    }

    Err(ApiError::forbidden(
        "event ingest requires a loopback bind or enabled serve auth",
    ))
}

fn consume_runtime_event(state: &AppState, event: &RuntimeEvent) {
    state.sse_adapter.consume(event);
}
