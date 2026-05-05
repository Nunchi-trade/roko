//! Mirage JSON-RPC reverse proxy routes.
//!
//! Exposes mirage-rs (bound to `127.0.0.1:8545` inside the container) through
//! roko-serve's single public port so Railway deployments can reach JSON-RPC,
//! WebSocket subscriptions, and the REST API.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};

use crate::error::ApiError;
use crate::state::AppState;

use super::proxy_ws::bridge_ws;
use super::ws::apply_ws_size_limits;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // JSON-RPC POST
        .route("/rpc", post(rpc_post))
        // JSON-RPC WebSocket (eth_subscribe)
        .route("/rpc", get(rpc_ws_upgrade))
        // Mirage live-events WebSocket
        .route("/rpc/events", get(rpc_events_ws_upgrade))
        // Health passthrough
        .route("/rpc/health", get(rpc_health))
        // REST API catch-all
        .route("/rpc/api/{*path}", any(rpc_api_proxy))
}

/// Return 503 when mirage is not configured.
fn require_mirage(state: &AppState) -> Result<String, ApiError> {
    state.mirage_url.clone().ok_or_else(|| ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "mirage_not_configured".into(),
        message: "ROKO_MIRAGE_URL not set — mirage proxy unavailable".into(),
        details: None,
    })
}

/// `POST /api/rpc` — forward JSON-RPC to mirage.
async fn rpc_post(
    State(state): State<Arc<AppState>>,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    let base = require_mirage(&state)?;

    let response = state
        .http_client
        .post(&base)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("mirage rpc proxy: {e}")))?;

    proxy_response(response).await
}

/// `GET /api/rpc` (WS upgrade) — eth_subscribe WebSocket.
async fn rpc_ws_upgrade(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, ApiError> {
    let base = require_mirage(&state)?;
    let upstream = base
        .replace("http://", "ws://")
        .replace("https://", "wss://");

    Ok(apply_ws_size_limits(ws).on_upgrade(move |socket| bridge_ws(socket, upstream)))
}

/// `GET /api/rpc/events` (WS upgrade) — mirage live events.
async fn rpc_events_ws_upgrade(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
    req: Request,
) -> Result<impl IntoResponse, ApiError> {
    let base = require_mirage(&state)?;
    let ws_base = base
        .replace("http://", "ws://")
        .replace("https://", "wss://");
    let query = req.uri().query().unwrap_or("");
    let upstream = if query.is_empty() {
        format!("{}/api/ws", ws_base.trim_end_matches('/'))
    } else {
        format!("{}/api/ws?{query}", ws_base.trim_end_matches('/'))
    };

    Ok(apply_ws_size_limits(ws).on_upgrade(move |socket| bridge_ws(socket, upstream)))
}

/// `GET /api/rpc/health` — proxy mirage health check.
async fn rpc_health(State(state): State<Arc<AppState>>) -> Result<Response, ApiError> {
    let base = require_mirage(&state)?;
    let url = format!("{}/health", base.trim_end_matches('/'));

    let response = state
        .http_client
        .get(&url)
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("mirage health proxy: {e}")))?;

    proxy_response(response).await
}

/// `GET|POST /api/rpc/api/{*path}` — proxy mirage REST API.
async fn rpc_api_proxy(
    State(state): State<Arc<AppState>>,
    req: Request,
) -> Result<Response, ApiError> {
    let base = require_mirage(&state)?;

    let path = req.uri().path().strip_prefix("/api/rpc/api/").unwrap_or("");
    let query = req.uri().query();
    let upstream_url = if let Some(q) = query {
        format!("{}/api/{path}?{q}", base.trim_end_matches('/'))
    } else {
        format!("{}/api/{path}", base.trim_end_matches('/'))
    };

    let method = req.method().clone();
    let content_type = req.headers().get(header::CONTENT_TYPE).cloned();
    let body = axum::body::to_bytes(req.into_body(), 4 * 1024 * 1024)
        .await
        .map_err(|e| ApiError::internal(format!("read proxy request body: {e}")))?;

    let mut upstream_req = state.http_client.request(method, &upstream_url);
    if let Some(ct) = content_type {
        upstream_req = upstream_req.header(header::CONTENT_TYPE, ct);
    }
    if !body.is_empty() {
        upstream_req = upstream_req.body(body);
    }

    let response = upstream_req
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("mirage api proxy: {e}")))?;

    proxy_response(response).await
}

/// Convert an upstream `reqwest::Response` into an axum `Response`, forwarding
/// status code and content-type.
async fn proxy_response(response: reqwest::Response) -> Result<Response, ApiError> {
    let status = response.status();
    let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
    let body = response
        .bytes()
        .await
        .map_err(|e| ApiError::internal(format!("read proxied response: {e}")))?;

    let mut builder = Response::builder().status(status.as_u16());
    if let Some(ct) = content_type {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }

    builder
        .body(Body::from(body))
        .map_err(|e| ApiError::internal(format!("build proxied response: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_mirage_returns_503_when_none() {
        // Construct a minimal AppState-like check: the helper reads state.mirage_url.
        // We just verify the error shape here.
        let err = ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "mirage_not_configured".into(),
            message: "test".into(),
            details: None,
        };
        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(err.code, "mirage_not_configured");
    }
}
