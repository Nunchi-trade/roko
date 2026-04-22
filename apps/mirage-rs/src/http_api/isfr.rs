//! ISFR proxy endpoints.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Query, State},
};
use serde_json::Value;

use super::{ApiError, ApiState, now_secs};

/// `GET /api/isfr/current` — proxy latest ISFR data from the upstream service.
///
/// # Errors
///
/// Returns `502` only when `ISFR_STRICT_PROXY` is enabled. Otherwise an
/// unavailable upstream degrades to a local no-data payload so dashboards do
/// not spam gateway errors when no ISFR sidecar is deployed.
pub async fn isfr_current(
    State(state): State<ApiState>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    match proxy_isfr("/v1/isfr/current", &query).await {
        Ok(body) => Ok(body),
        Err(error) if strict_proxy_enabled() => Err(error),
        Err(error) => Ok(Json(local_current_fallback(&state, &error.error))),
    }
}

/// `GET /api/isfr/history` — proxy ISFR history from the upstream service.
///
/// # Errors
///
/// Returns `502` only when `ISFR_STRICT_PROXY` is enabled. Otherwise an
/// unavailable upstream degrades to an empty local history payload.
pub async fn isfr_history(
    State(state): State<ApiState>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    match proxy_isfr("/v1/isfr/history", &query).await {
        Ok(body) => Ok(body),
        Err(error) if strict_proxy_enabled() => Err(error),
        Err(error) => Ok(Json(local_history_fallback(&state, &query, &error.error))),
    }
}

async fn proxy_isfr(path: &str, query: &HashMap<String, String>) -> Result<Json<Value>, ApiError> {
    let base = std::env::var("ISFR_SERVICE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ApiError {
            error: "ISFR_SERVICE_URL not configured".to_owned(),
            code: 503,
        })?;
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

fn strict_proxy_enabled() -> bool {
    std::env::var("ISFR_STRICT_PROXY")
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

fn local_current_fallback(state: &ApiState, reason: &str) -> Value {
    let chain = state.chain.read();
    serde_json::json!({
        "status": "unavailable",
        "source": "mirage-local-fallback",
        "reason": reason,
        "state": "no_data",
        "value_bps": null,
        "value": null,
        "confidence": 0.0,
        "block": (state.current_block)(),
        "timestamp": now_secs(),
        "counts": {
            "insights": chain.knowledge.len(),
            "pheromones": chain.pheromones.len(),
            "agents": chain.agent_registry.list_agents().len(),
            "tasks": chain.task_store.len(),
            "prediction_sessions": chain.prediction_store.session_count(),
            "prediction_claims": chain.prediction_store.claim_count(),
        },
    })
}

fn local_history_fallback(
    state: &ApiState,
    query: &HashMap<String, String>,
    reason: &str,
) -> Value {
    serde_json::json!({
        "status": "unavailable",
        "source": "mirage-local-fallback",
        "reason": reason,
        "state": "no_data",
        "items": [],
        "points": [],
        "history": [],
        "query": query,
        "block": (state.current_block)(),
        "timestamp": now_secs(),
    })
}
