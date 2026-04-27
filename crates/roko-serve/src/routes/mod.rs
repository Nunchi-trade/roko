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
use axum::response::Html;
use axum::routing::get;
use axum::{Json, Router};
use roko_core::config::ServeAuthConfig;
use serde_json::{Value, json};
use tower_http::services::ServeDir;
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

    // Serve static demo files from demo/demo-web/ if directory exists.
    let demo_dir = state.workdir.join("demo").join("demo-web");
    let static_service = if demo_dir.exists() {
        Some(ServeDir::new(demo_dir))
    } else {
        None
    };

    let mut router = Router::new()
        // Root index page — dashboard entry point.
        .route("/", get(root_index))
        // Top-level liveness probe — no auth, no /api prefix.
        .route("/health", get(top_level_health))
        .merge(webhooks::routes())
        // Shareable run pages — no auth, serves HTML at /runs/{id}
        .merge(shared_runs::routes())
        // PTY terminal sessions for web UI — no auth
        .merge(crate::terminal::routes())
        .nest("/api", api)
        .merge(ws);

    // Mount static demo files at /demo/
    if let Some(svc) = static_service {
        router = router.nest_service("/demo", svc);
    }

    router
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

/// `GET /` — root index page with links to all UI surfaces.
async fn root_index() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>roko serve</title>
<style>
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { background: #16121a; color: #a58e9e; font-family: 'Geist Mono','SF Mono','Menlo',monospace;
    display: flex; flex-direction: column; align-items: center; padding: 3rem; min-height: 100vh; }
  h1 { color: #b97894; font-size: 1.4rem; margin-bottom: 0.5rem; }
  .sub { color: #916e8a; font-size: 0.85rem; margin-bottom: 2rem; }
  .links { display: flex; flex-direction: column; gap: 0.75rem; width: 400px; }
  a { display: block; padding: 12px 20px; background: #0e0c10; border: 1px solid #372a37;
    border-radius: 6px; color: #d7c69e; text-decoration: none; transition: border-color 0.2s; }
  a:hover { border-color: #b97894; }
  a .label { font-weight: 600; }
  a .desc { color: #916e8a; font-size: 0.75rem; margin-top: 2px; }
  .section { color: #372a37; font-size: 0.7rem; text-transform: uppercase; letter-spacing: 0.1em;
    margin-top: 1.5rem; margin-bottom: 0.5rem; }
</style>
</head>
<body>
  <h1>◆ roko serve</h1>
  <p class="sub">agent runtime control plane</p>
  <div class="links">
    <div class="section">demo</div>
    <a href="/demo/demo.html"><span class="label">Unified Demo</span><div class="desc">Series A pitch demo — 7 scenarios, live terminals, ROSEDUST visual system</div></a>
    <a href="/demo/bench.html"><span class="label">Benchmark Lab</span><div class="desc">Configure and run SWE-bench evaluations, compare models, track self-learning</div></a>
    <a href="/demo/bench-live.html"><span class="label">Live Monitor</span><div class="desc">Real-time benchmark observation — task grid, cost chart, activity feed</div></a>
    <a href="/demo/builder.html"><span class="label">Builder</span><div class="desc">Type a request — roko builds it live in a temp repo</div></a>
    <a href="/demo/terminal.html"><span class="label">Terminal</span><div class="desc">Multi-pane browser terminal with real PTY sessions</div></a>
    <a href="/demo/index.html"><span class="label">Scripted Demo</span><div class="desc">Pre-recorded demo sequence (no backend needed)</div></a>
    <div class="section">api</div>
    <a href="/api/health"><span class="label">Health</span><div class="desc">Server health check</div></a>
    <a href="/api/status"><span class="label">Status</span><div class="desc">Workspace status and signal counts</div></a>
    <a href="/api/episodes"><span class="label">Episodes</span><div class="desc">Agent execution episodes</div></a>
    <a href="/api/terminal/sessions"><span class="label">Terminal Sessions</span><div class="desc">Active PTY session list</div></a>
    <div class="section">docs</div>
    <a href="/api/openapi.json"><span class="label">OpenAPI Spec</span><div class="desc">Full API schema (JSON)</div></a>
  </div>
</body>
</html>"#)
}

/// `GET /health` — bare liveness probe for load balancers and external tools.
///
/// Returns `{"status": "ok"}` unconditionally. For richer telemetry use
/// `GET /api/health`.
async fn top_level_health() -> Json<Value> {
    Json(json!({"status": "ok"}))
}
