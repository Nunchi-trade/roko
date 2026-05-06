//! Agent-relay reverse proxy routes.
//!
//! Exposes the agent-relay service (bound to `127.0.0.1:9011` inside the
//! container) through roko-serve's single public port. WebSocket routes are
//! registered before the catch-all so they match first.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};

use crate::error::ApiError;
use crate::state::AppState;

use super::proxy_ws::bridge_ws;
use super::ws::apply_ws_size_limits;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // WebSocket routes — registered before catch-all
        .route("/relay/agents/ws", get(relay_agents_ws))
        .route("/relay/events/ws", get(relay_events_ws))
        // Catch-all for relay HTTP endpoints (health, agents CRUD, etc.)
        .route("/relay/{*path}", any(relay_proxy))
        .route("/relay", any(relay_root_proxy))
}

/// Return the relay HTTP base URL (scheme + authority only), or 503 when not configured.
fn require_relay(state: &AppState) -> Result<String, ApiError> {
    let raw = state.agent_relay_url.as_deref().ok_or_else(|| ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "agent_relay_not_configured".into(),
        message: "ROKO_AGENT_RELAY_URL not set — relay proxy unavailable".into(),
        details: None,
    })?;
    // Normalise: strip any path, convert ws(s) to http(s).
    Ok(normalize_relay_base(raw))
}

/// Extract `http(s)://host:port` from any relay URL variant.
fn normalize_relay_base(raw: &str) -> String {
    let s = raw
        .replace("ws://", "http://")
        .replace("wss://", "https://");
    // Keep only scheme + authority (strip path).
    if let Some(idx) = s.find("://") {
        let after_scheme = &s[idx + 3..];
        if let Some(slash) = after_scheme.find('/') {
            return s[..idx + 3 + slash].to_string();
        }
    }
    s
}

/// `GET /relay/agents/ws` — proxy agent WebSocket.
async fn relay_agents_ws(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, ApiError> {
    let base = require_relay(&state)?;
    let upstream = format!(
        "{}/relay/agents/ws",
        base.trim_end_matches('/')
            .replace("http://", "ws://")
            .replace("https://", "wss://")
    );

    Ok(apply_ws_size_limits(ws).on_upgrade(move |socket| bridge_ws(socket, upstream)))
}

/// `GET /relay/events/ws` — proxy relay events WebSocket.
async fn relay_events_ws(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, ApiError> {
    let base = require_relay(&state)?;
    let upstream = format!(
        "{}/relay/events/ws",
        base.trim_end_matches('/')
            .replace("http://", "ws://")
            .replace("https://", "wss://")
    );

    Ok(apply_ws_size_limits(ws).on_upgrade(move |socket| bridge_ws(socket, upstream)))
}

/// `GET|POST|DELETE /relay/{*path}` — proxy relay HTTP endpoints.
async fn relay_proxy(
    State(state): State<Arc<AppState>>,
    req: Request,
) -> Result<Response, ApiError> {
    let base = require_relay(&state)?;

    let path = req.uri().path().strip_prefix("/relay/").unwrap_or("");
    let query = req.uri().query();
    let upstream_url = if let Some(q) = query {
        format!("{}/relay/{path}?{q}", base.trim_end_matches('/'))
    } else {
        format!("{}/relay/{path}", base.trim_end_matches('/'))
    };

    forward_http(&state, req, &upstream_url).await
}

/// `GET|POST /relay` — proxy relay root.
async fn relay_root_proxy(
    State(state): State<Arc<AppState>>,
    req: Request,
) -> Result<Response, ApiError> {
    let base = require_relay(&state)?;
    let upstream_url = format!("{}/relay", base.trim_end_matches('/'));
    forward_http(&state, req, &upstream_url).await
}

/// Forward an HTTP request to an upstream URL, returning the upstream response.
async fn forward_http(
    state: &AppState,
    req: Request,
    upstream_url: &str,
) -> Result<Response, ApiError> {
    let method = req.method().clone();
    let content_type = req.headers().get(header::CONTENT_TYPE).cloned();
    let body = axum::body::to_bytes(req.into_body(), 4 * 1024 * 1024)
        .await
        .map_err(|e| ApiError::internal(format!("read relay proxy request body: {e}")))?;

    let mut upstream_req = state.http_client.request(method, upstream_url);
    if let Some(ct) = content_type {
        upstream_req = upstream_req.header(header::CONTENT_TYPE, ct);
    }
    if !body.is_empty() {
        upstream_req = upstream_req.body(body);
    }

    let response = upstream_req
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("relay proxy: {e}")))?;

    let status = response.status();
    let resp_ct = response.headers().get(header::CONTENT_TYPE).cloned();
    let resp_body = response
        .bytes()
        .await
        .map_err(|e| ApiError::internal(format!("read relay proxy response: {e}")))?;

    let mut builder = Response::builder().status(status.as_u16());
    if let Some(ct) = resp_ct {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }

    builder
        .body(Body::from(resp_body))
        .map_err(|e| ApiError::internal(format!("build relay proxy response: {e}")))
}
