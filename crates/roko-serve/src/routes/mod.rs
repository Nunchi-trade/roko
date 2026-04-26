//! Route definitions for the roko HTTP API.
//!
//! Each submodule defines handlers for a related group of endpoints. The
//! [`build_router`] function assembles them into a single [`axum::Router`]
//! with CORS and tracing middleware.

mod agents;
mod aggregator;
mod auth;
mod chain;
pub(crate) mod config;
mod connectors;
mod deployments;
mod diagnosis;
mod dream;
mod feeds;
mod gateway;
mod heartbeats;
mod integrations;
mod jobs;
mod learning;
mod middleware;
mod neuro;
mod plans;
pub(crate) mod prds;
mod projections;
mod providers;
mod research;
mod run;
mod secrets;
pub mod shared_runs;
mod sse;
mod status;
mod subscriptions;
mod team;
mod templates;
mod vision_loop;
mod webhooks;
mod ws;

use std::sync::Arc;

use super::state::AppState;
use axum::routing::get;
use axum::{Json, Router};
use roko_core::config::ServeAuthConfig;
use serde_json::{Value, json};
use tower_http::trace::TraceLayer;

pub use self::config::reload_config_from_disk;
pub use self::deployments::load_persisted_deployments;
pub(crate) use self::prds::start_prd_publish_subscriber;

/// Build the complete API router with all route groups and middleware.
pub fn build_router(
    state: Arc<AppState>,
    cors_origins: &[String],
    api_auth: ServeAuthConfig,
) -> Router {
    let cors = middleware::cors_layer(cors_origins);

    let api = Router::new()
        .merge(crate::openapi::routes())
        .merge(status::routes())
        .merge(jobs::routes())
        .merge(heartbeats::routes())
        .merge(plans::routes())
        .merge(prds::routes())
        .merge(run::routes())
        .merge(research::routes())
        .merge(subscriptions::routes())
        .merge(templates::routes())
        .merge(aggregator::routes())
        .merge(agents::routes())
        .merge(learning::routes())
        .merge(config::routes())
        .merge(deployments::routes())
        .merge(diagnosis::routes())
        .merge(integrations::routes())
        .merge(projections::routes())
        .merge(neuro::routes())
        .merge(dream::routes())
        .merge(gateway::routes())
        .merge(chain::routes())
        .merge(connectors::routes())
        .merge(feeds::routes())
        .merge(auth::routes())
        .merge(secrets::routes())
        .merge(vision_loop::routes())
        .merge(team::routes())
        .nest("/providers", providers::router())
        .nest("/models", providers::models_router())
        .nest("/routing", providers::routing_router())
        .merge(sse::routes());

    let api = if api_auth.enabled {
        api.layer(axum::middleware::from_fn(middleware::require_scope))
            .layer(axum::middleware::from_fn_with_state(
                Arc::clone(&state),
                middleware::require_api_key,
            ))
    } else {
        api
    };

    // Secret-scrubbing layer: redacts API keys / tokens from JSON responses.
    let scrubber = Arc::clone(&state.scrubber);
    let api = api.layer(axum::middleware::from_fn_with_state(
        scrubber,
        middleware::scrub_secrets,
    ));

    let ws = if api_auth.enabled {
        ws::routes().layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            middleware::require_api_key,
        ))
    } else {
        ws::routes()
    };

    Router::new()
        // Top-level liveness probe — no auth, no /api prefix.
        .route("/health", get(top_level_health))
        .merge(webhooks::routes())
        // Shareable run pages — no auth, serves HTML at /runs/{id}
        .merge(shared_runs::routes())
        .nest("/api", api)
        .merge(ws)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

/// `GET /health` — bare liveness probe for load balancers and external tools.
///
/// Returns `{"status": "ok"}` unconditionally. For richer telemetry use
/// `GET /api/health`.
async fn top_level_health() -> Json<Value> {
    Json(json!({"status": "ok"}))
}
