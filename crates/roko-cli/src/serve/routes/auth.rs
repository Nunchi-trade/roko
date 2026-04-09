//! Shared API auth middleware for `/api/*` routes.

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;
use roko_core::config::ServeAuthConfig;

use crate::serve::error::ApiError;

/// Require a matching `X-Api-Key` header for the request to continue.
pub async fn require_api_key(
    State(auth): State<ServeAuthConfig>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let supplied = req
        .headers()
        .get("X-Api-Key")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");

    if supplied != auth.api_key {
        return Err(ApiError::unauthorized(
            "invalid or missing X-Api-Key header",
        ));
    }

    Ok(next.run(req).await)
}
