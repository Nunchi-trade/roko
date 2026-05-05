//! Route definitions for the roko HTTP API.
//!
//! Each submodule defines handlers for a related group of endpoints. The
//! [`build_router`] function assembles them into a single [`axum::Router`]
//! with CORS and tracing middleware.

mod agents;
mod aggregator;
mod auth;
mod bench;
mod chain;
pub(crate) mod config;
mod connectors;
mod deployments;
mod diagnosis;
mod dream;
mod event_ingest;
mod feeds;
mod gateway;
mod heartbeats;
mod integrations;
mod isfr;
mod jobs;
mod learning;
mod metrics;
mod middleware;
mod neuro;
mod plans;
pub(crate) mod prds;
mod projections;
mod providers;
mod research;
mod run;
mod runs;
mod secrets;
pub mod shared_runs;
pub(crate) mod sse;
mod status;
mod subscriptions;
mod swe_bench;
mod team;
mod templates;
mod vision_loop;
mod webhooks;
mod workflows;
mod workspaces;
mod ws;

mod proxy_ws;
mod relay_proxy;
mod rpc_proxy;

use std::convert::Infallible;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Arc;

use super::state::AppState;
use crate::adapters::SseAdapter;
use crate::error::ApiError;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::{Json, Router};
use futures::stream::{self, Stream};
use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use roko_core::config::ServeAuthConfig;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tower_http::trace::TraceLayer;

/// Global request-body cap. Axum's default is 2 MiB; we raise it to 4 MiB so
/// reasonably sized JSON payloads (PRDs, agent manifests, plan objects) still
/// fit while keeping the cap small enough to bound memory pressure from a
/// single hostile client. Webhook routes that accept opaque `Bytes` clamp
/// further to 1 MiB locally.
pub(crate) const DEFAULT_REQUEST_BODY_LIMIT_BYTES: usize = 4 * 1024 * 1024;

/// Default global rate limit applied to every route.
///
/// 100 requests per second is generous for legitimate traffic but bounds the
/// damage of a chatty / runaway client without per-endpoint configuration.
pub(crate) const DEFAULT_GLOBAL_RATE_PER_SEC: u32 = 100;

/// In-memory single-bucket rate limiter shared across all requests.
type GlobalRateLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>;

/// Build a non-keyed governor rate limiter with a fixed `req/s` budget.
pub(crate) fn build_global_rate_limiter(per_second: u32) -> Arc<GlobalRateLimiter> {
    let per_second =
        NonZeroU32::new(per_second.max(1)).expect("rate-limit must be non-zero (max(1) above)");
    Arc::new(RateLimiter::direct(Quota::per_second(per_second)))
}

/// Middleware: reject requests once the shared bucket has been exhausted.
///
/// Returns 429 with a stable `code = "rate_limited"` body so clients can
/// distinguish throttling from auth/validation errors.
pub(crate) async fn rate_limit_middleware(
    State(limiter): State<Arc<GlobalRateLimiter>>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    if limiter.check().is_err() {
        return Err(ApiError {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "rate_limited".into(),
            message: format!(
                "global rate limit exceeded ({DEFAULT_GLOBAL_RATE_PER_SEC} requests/sec)"
            ),
            details: None,
        });
    }
    Ok(next.run(req).await)
}

pub use self::config::reload_config_from_disk;
pub use self::deployments::load_persisted_deployments;
pub(crate) use self::middleware::cors_layer;
pub(crate) use self::prds::start_prd_publish_subscriber;
pub(crate) use self::ws::apply_ws_size_limits as ws_size_limits;

/// Build the complete API router with all route groups and middleware.
pub fn build_router(
    state: Arc<AppState>,
    cors_origins: &[String],
    api_auth: ServeAuthConfig,
) -> Router {
    state
        .sse_adapter
        .set_state_hub_consumer(crate::dashboard_event_bridge(&state));
    state.sse_adapter.start_runtime_event_subscription();

    let roko_config = state.load_roko_config();
    let cors = middleware::cors_layer(cors_origins, roko_config.server.unsafe_public_cors);
    let terminal_enabled = roko_config.serve.terminal_enabled;
    let terminal_requires_auth = terminal_enabled
        && !bind_is_loopback(&roko_config.server.bind)
        && !roko_config.serve.acknowledge_public_risk;

    let api = Router::new()
        .merge(crate::openapi::routes())
        .merge(status::routes())
        .merge(jobs::routes())
        .merge(heartbeats::routes())
        .merge(plans::routes())
        .merge(prds::routes())
        .merge(run::routes())
        .merge(runs::routes())
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
        .merge(event_ingest::routes())
        .merge(gateway::routes())
        .merge(chain::routes())
        .merge(connectors::routes())
        .merge(feeds::routes())
        .merge(isfr::routes())
        .merge(auth::routes())
        .merge(secrets::routes())
        .merge(vision_loop::routes())
        .merge(team::routes())
        .merge(bench::routes())
        .merge(swe_bench::routes())
        .merge(workflows::routes())
        .merge(workspaces::routes())
        .merge(shared_runs::auth_routes())
        .merge(webhooks::authenticated_routes())
        .nest("/providers", providers::router())
        .nest("/models", providers::models_router())
        .nest("/routing", providers::routing_router())
        .merge(sse::routes())
        .merge(rpc_proxy::routes())
        .route("/workflow/events", get(workflow_sse_handler));

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

    let terminal = if terminal_enabled {
        let terminal = crate::terminal::routes();
        if terminal_requires_auth {
            terminal.layer(axum::middleware::from_fn_with_state(
                Arc::clone(&state),
                middleware::require_api_key,
            ))
        } else {
            terminal
        }
    } else {
        crate::terminal::disabled_routes()
    };

    let ws = if api_auth.enabled {
        ws::routes().layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            middleware::require_api_key,
        ))
    } else {
        ws::routes()
    };

    let router = Router::new()
        // Top-level liveness probe — no auth, no /api prefix.
        .route("/health", get(top_level_health))
        // Top-level readiness probe — no auth, no /api prefix.
        .route("/ready", get(top_level_ready))
        // Standard Prometheus scrape endpoint — no auth, no /api prefix.
        .route("/metrics", get(metrics::metrics_handler))
        .merge(webhooks::public_routes())
        // Public share-receipt reader: no auth required so recipients can
        // open share links without a roko API key.
        .merge(shared_runs::public_routes())
        // PTY terminal sessions for web UI — gated by config and bind policy.
        .merge(terminal)
        .nest("/api", api)
        .merge(ws)
        .merge(relay_proxy::routes())
        // SPA fallback — serves embedded React app for all unmatched routes
        .fallback(crate::embedded::serve_embedded);

    let rate_limiter = build_global_rate_limiter(DEFAULT_GLOBAL_RATE_PER_SEC);

    router
        .layer(DefaultBodyLimit::max(DEFAULT_REQUEST_BODY_LIMIT_BYTES))
        .layer(axum::middleware::from_fn_with_state(
            rate_limiter,
            rate_limit_middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

/// `GET /health` — bare liveness probe for load balancers and external tools.
///
/// Returns 200 while the process is alive. For richer telemetry use
/// `GET /api/health`.
async fn top_level_health(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": state.started_at.elapsed().as_secs(),
    }))
}

/// `GET /ready` — readiness probe for platforms that drain shutting-down
/// instances before stopping them.
async fn top_level_ready(State(state): State<Arc<AppState>>) -> (StatusCode, Json<Value>) {
    if state.cancel.is_cancelled() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "shutting_down",
                "version": env!("CARGO_PKG_VERSION"),
                "uptime_secs": state.started_at.elapsed().as_secs(),
            })),
        );
    }

    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_secs": state.started_at.elapsed().as_secs(),
        })),
    )
}

/// `GET /api/workflow/events` — RuntimeEvent-typed SSE stream for WorkflowEngine.
async fn workflow_sse_handler(
    State(state): State<Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    let adapter: &Arc<SseAdapter> = &state.sse_adapter;
    let rx = adapter.subscribe();
    let sse = workflow_sse_from_adapter(rx);
    (sse::sse_response_headers(), sse)
}

fn workflow_sse_from_adapter(
    rx: broadcast::Receiver<crate::adapters::SseEvent>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(sse_event) => {
                    let data = serde_json::to_string(&sse_event).unwrap_or_default();
                    let event = Event::default().event(sse_event.kind.clone()).data(data);
                    return Some((Ok(event), rx));
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(n, "workflow SSE client lagged");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(8))
            .text("keepalive"),
    )
}

fn bind_is_loopback(bind: &str) -> bool {
    let host = bind
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(bind);

    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    host.parse::<IpAddr>().is_ok_and(|addr| addr.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt as _;
    use roko_core::config::{RokoConfig, ServeAuthConfig};
    use serde_json::Value;
    use tempfile::tempdir;
    use tower::ServiceExt as _;

    use crate::deploy::create_backend;
    use crate::runtime::NoOpRuntime;

    fn build_test_state_and_router(
        config: RokoConfig,
    ) -> (tempfile::TempDir, Arc<AppState>, axum::Router) {
        let dir = tempdir().expect("tempdir");
        let deploy = Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let state = Arc::new(
            AppState::new(
                dir.path().to_path_buf(),
                Arc::new(NoOpRuntime),
                config.clone(),
                deploy,
            )
            .expect("AppState::new"),
        );
        let router = build_router(Arc::clone(&state), &[], config.serve.auth.clone());
        (dir, state, router)
    }

    fn build_test_router(config: RokoConfig) -> (tempfile::TempDir, axum::Router) {
        let (dir, _state, router) = build_test_state_and_router(config);
        (dir, router)
    }

    async fn get_json(router: &axum::Router, uri: &str) -> (StatusCode, Value) {
        let req = Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("build request");
        let resp = router.clone().oneshot(req).await.expect("oneshot");
        let status = resp.status();
        let body = resp
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn terminal_routes_are_disabled_by_default() {
        let (_dir, app) = build_test_router(RokoConfig::default());
        let (status, body) = get_json(&app, "/api/terminal/sessions").await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "Terminal disabled");
        assert_eq!(
            body["hint"],
            "Set serve.terminal_enabled=true or use --enable-terminal"
        );
    }

    #[tokio::test]
    async fn top_level_health_and_ready_are_available_without_auth() {
        let mut config = RokoConfig::default();
        config.serve.auth = ServeAuthConfig {
            enabled: true,
            api_key: "health-secret".into(),
            api_keys: Vec::new(),
            privy_app_id: None,
        };

        let (_dir, app) = build_test_router(config);

        let (status, body) = get_json(&app, "/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert!(body["uptime_secs"].as_u64().is_some());

        let (status, body) = get_json(&app, "/ready").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert!(body["uptime_secs"].as_u64().is_some());
    }

    #[tokio::test]
    async fn top_level_ready_reports_shutting_down_after_cancellation() {
        let (_dir, state, app) = build_test_state_and_router(RokoConfig::default());
        state.cancel.cancel();

        let (status, body) = get_json(&app, "/ready").await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["status"], "shutting_down");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert!(body["uptime_secs"].as_u64().is_some());
    }

    #[tokio::test]
    async fn terminal_routes_allow_loopback_without_auth() {
        let mut config = RokoConfig::default();
        config.serve.terminal_enabled = true;

        let (_dir, app) = build_test_router(config);
        let (status, body) = get_json(&app, "/api/terminal/sessions").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!({ "sessions": [] }));
    }

    #[tokio::test]
    async fn terminal_routes_require_auth_on_public_bind() {
        let mut config = RokoConfig::default();
        config.server.bind = "0.0.0.0".into();
        config.serve.terminal_enabled = true;
        config.serve.auth = ServeAuthConfig {
            enabled: true,
            api_key: "terminal-secret".into(),
            api_keys: Vec::new(),
            privy_app_id: None,
        };

        let (_dir, app) = build_test_router(config);
        let (status, body) = get_json(&app, "/api/terminal/sessions").await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["code"], "unauthorized");
    }

    /// Verify that `DefaultBodyLimit::max(4 MiB)` rejects a 4 MiB + 1 byte
    /// body via axum's standard 413 path. We isolate the layer with a tiny
    /// `Bytes`-extracting handler so the assertion holds independent of
    /// route-specific parsing (which would otherwise mask the body cap).
    #[tokio::test]
    async fn body_size_limit_returns_413_for_oversized_payload() {
        async fn echo_body(_: axum::body::Bytes) -> StatusCode {
            StatusCode::OK
        }

        let app = axum::Router::new()
            .route("/echo", axum::routing::post(echo_body))
            .layer(DefaultBodyLimit::max(DEFAULT_REQUEST_BODY_LIMIT_BYTES));

        let oversized = vec![b'a'; DEFAULT_REQUEST_BODY_LIMIT_BYTES + 1];
        let req = Request::builder()
            .method("POST")
            .uri("/echo")
            .body(Body::from(oversized))
            .expect("build request");
        let resp = app.clone().oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);

        // Sanity-check that an in-budget body still goes through.
        let in_budget = vec![b'a'; 1024];
        let req = Request::builder()
            .method("POST")
            .uri("/echo")
            .body(Body::from(in_budget))
            .expect("build request");
        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Drive the rate-limit middleware directly with a tiny budget. We use
    /// `axum::middleware::from_fn_with_state` rather than `build_router` so
    /// we don't have to defeat the production limiter (100 req/s is too high
    /// to exhaust deterministically in a unit test).
    #[tokio::test]
    async fn rate_limit_middleware_returns_429_when_exceeded() {
        let limiter = build_global_rate_limiter(2);
        let app = axum::Router::new()
            .route("/ping", axum::routing::get(|| async { "pong" }))
            .layer(axum::middleware::from_fn_with_state(
                limiter,
                rate_limit_middleware,
            ));

        // First two requests fit inside the per-second budget.
        for _ in 0..2 {
            let req = Request::builder()
                .uri("/ping")
                .body(Body::empty())
                .expect("build request");
            let resp = app.clone().oneshot(req).await.expect("oneshot");
            assert_eq!(resp.status(), StatusCode::OK);
        }

        // The third immediate request must be throttled because the bucket
        // has been drained (governor refills at 2 tokens/sec, so a burst of
        // 3 within the same instant is guaranteed to overflow).
        let req = Request::builder()
            .uri("/ping")
            .body(Body::empty())
            .expect("build request");
        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        let body = resp
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        let json: Value = serde_json::from_slice(&body).expect("json body");
        assert_eq!(json["code"], "rate_limited");
    }
}
