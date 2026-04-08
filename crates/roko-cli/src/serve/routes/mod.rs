//! Route definitions for the roko HTTP API.
//!
//! Each submodule defines handlers for a related group of endpoints. The
//! [`build_router`] function assembles them into a single [`axum::Router`]
//! with CORS and tracing middleware.

mod agents;
mod config;
mod deployments;
mod learning;
mod plans;
mod prds;
mod research;
mod run;
mod status;
mod templates;
mod ws;

use std::sync::Arc;

use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use super::state::AppState;

/// Build the complete API router with all route groups and middleware.
pub fn build_router(state: Arc<AppState>, cors_origins: &[String]) -> Router {
    let cors = if cors_origins.is_empty() {
        CorsLayer::permissive()
    } else {
        let allowed: Vec<axum::http::HeaderValue> = cors_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(allowed)
            .allow_methods(Any)
            .allow_headers(Any)
    };

    let api = Router::new()
        .merge(status::routes())
        .merge(plans::routes())
        .merge(prds::routes())
        .merge(run::routes())
        .merge(research::routes())
        .merge(templates::routes())
        .merge(agents::routes())
        .merge(learning::routes())
        .merge(config::routes())
        .merge(deployments::routes());

    Router::new()
        .nest("/api", api)
        .merge(ws::routes())
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}
