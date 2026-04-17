//! ISFR proxy endpoints.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Query, State},
};
use serde_json::Value;

use super::{ApiError, ApiState};

const DEFAULT_ISFR_SERVICE_URL: &str = "http://localhost:8546";

/// `GET /api/isfr/current` — proxy latest ISFR data from the upstream service.
///
/// # Errors
///
/// Returns `502` if the upstream ISFR service is unavailable, returns a
/// non-success status, or returns malformed JSON.
pub async fn isfr_current(
    State(_state): State<ApiState>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    proxy_isfr("/v1/isfr/current", &query).await
}

/// `GET /api/isfr/history` — proxy ISFR history from the upstream service.
///
/// # Errors
///
/// Returns `502` if the upstream ISFR service is unavailable, returns a
/// non-success status, or returns malformed JSON.
pub async fn isfr_history(
    State(_state): State<ApiState>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    proxy_isfr("/v1/isfr/history", &query).await
}

async fn proxy_isfr(path: &str, query: &HashMap<String, String>) -> Result<Json<Value>, ApiError> {
    let base =
        std::env::var("ISFR_SERVICE_URL").unwrap_or_else(|_| DEFAULT_ISFR_SERVICE_URL.to_string());
    let url = format!("{base}{path}");
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .query(query)
        .send()
        .await
        .map_err(|error| ApiError {
            error: format!("isfr-service unavailable: {error}"),
            code: 502,
        })?;

    if !response.status().is_success() {
        return Err(ApiError {
            error: format!("isfr-service returned {}", response.status()),
            code: 502,
        });
    }

    let body = response.json::<Value>().await.map_err(|error| ApiError {
        error: format!("isfr-service bad response: {error}"),
        code: 502,
    })?;
    Ok(Json(body))
}
