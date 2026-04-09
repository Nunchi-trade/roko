//! Shared API auth middleware for `/api/*` routes.

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;
use roko_core::config::ServeAuthConfig;
use tower_http::cors::{Any, CorsLayer};

use crate::error::ApiError;

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

/// Build the CORS layer from configured origins.
pub fn cors_layer(cors_origins: &[String]) -> CorsLayer {
    if cors_origins.is_empty() {
        CorsLayer::permissive()
    } else {
        let allowed: Vec<axum::http::HeaderValue> =
            cors_origins.iter().filter_map(|o| o.parse().ok()).collect();
        CorsLayer::new()
            .allow_origin(allowed)
            .allow_methods(Any)
            .allow_headers(Any)
    }
}
