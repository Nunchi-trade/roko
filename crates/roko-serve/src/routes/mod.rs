//! Route definitions for the roko HTTP API.
//!
//! Each submodule defines handlers for a related group of endpoints. The
//! [`build_router`] function assembles them into a single [`axum::Router`]
//! with CORS and tracing middleware.

mod agents;
mod aggregator;
mod config;
mod deployments;
mod diagnosis;
mod learning;
mod middleware;
mod plans;
pub(crate) mod prds;
mod providers;
mod research;
mod run;
mod sse;
mod status;
mod subscriptions;
mod templates;
mod webhooks;
mod ws;

use std::sync::Arc;

use super::state::AppState;
use axum::Router;
use roko_core::config::ServeAuthConfig;
use tower_http::trace::TraceLayer;

pub use self::config::reload_config_from_disk;
pub(crate) use self::prds::start_prd_publish_subscriber;

/// Build the complete API router with all route groups and middleware.
pub fn build_router(
    state: Arc<AppState>,
    cors_origins: &[String],
    api_auth: ServeAuthConfig,
) -> Router {
    let cors = middleware::cors_layer(cors_origins);

    let api = Router::new()
        .merge(status::routes())
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
        .nest("/providers", providers::router())
        .nest("/models", providers::models_router())
        .nest("/routing", providers::routing_router())
        .merge(sse::routes());

    let api = if api_auth.enabled {
        api.layer(axum::middleware::from_fn_with_state(
            api_auth,
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

    Router::new()
        .merge(webhooks::routes())
        .nest("/api", api)
        .merge(ws::routes())
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}
