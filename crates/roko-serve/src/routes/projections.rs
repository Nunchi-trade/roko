//! StateHub-backed projection routes for remote read and watch flows.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::{Json, Router};
use futures::stream::{self, Stream, StreamExt};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tracing::warn;

use crate::error::ApiError;
use crate::projection_contract::{
    ProjectionQuery, RuntimeProjectionSet, projection_accepts_event, projection_delta_frame,
};
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/projections/catalog", get(projections_catalog))
        .route("/projections/{name}", get(get_projection))
        .route("/projections/{name}/stream", get(stream_projection))
}

/// `GET /api/projections/catalog` — return projection names, versions, and invalidation policies.
async fn projections_catalog() -> Json<Value> {
    let entries = crate::projection_contract::projection_policies();
    Json(json!({ "projections": entries }))
}

async fn get_projection(
    Path(name): Path<String>,
    Query(query): Query<ProjectionQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    let projection = projections.project(&name, &query)?;
    Ok(Json(projections.state_frame(&name, projection)))
}

async fn stream_projection(
    Path(name): Path<String>,
    Query(query): Query<ProjectionQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    let initial_state = projections.project(&name, &query)?;
    let initial = Event::default()
        .event("state")
        .id(projections.cursor.to_string())
        .data(projections.state_frame(&name, initial_state).to_string());

    let name_for_stream = name.clone();
    let query_for_stream = query.clone();
    let delta_stream = stream::unfold(state.state_hub.subscribe_events(), move |mut rx| {
        let name = name_for_stream.clone();
        let query = query_for_stream.clone();
        async move {
            loop {
                match rx.recv().await {
                    Ok(envelope) => {
                        if !projection_accepts_event(&name, &query, &envelope.payload) {
                            continue;
                        }
                        let event = Event::default()
                            .event("delta")
                            .id(envelope.seq.to_string())
                            .data(
                                projection_delta_frame(&name, envelope.seq, &envelope.payload)
                                    .to_string(),
                            );
                        return Some((Ok(event), rx));
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(projection = %name, skipped, "projection stream lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        }
    });

    Ok(
        Sse::new(stream::once(async move { Ok(initial) }).chain(delta_stream))
            .keep_alive(KeepAlive::default()),
    )
}
