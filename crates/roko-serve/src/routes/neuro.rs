//! Neuro knowledge store query endpoint.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::extract::{RequestPayload, ValidJson};
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/neuro/query", post(neuro_query))
        .route("/knowledge", get(knowledge_query))
}

#[derive(Debug, Deserialize)]
struct NeuroQueryRequest {
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    min_tier: Option<String>,
}

fn default_limit() -> usize {
    10
}

impl RequestPayload for NeuroQueryRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        if self.query.trim().is_empty() {
            return Err(ApiError::bad_request("query must not be blank"));
        }
        Ok(())
    }
}

/// `POST /api/neuro/query` — query the knowledge store.
async fn neuro_query(
    State(state): State<Arc<AppState>>,
    ValidJson(body): ValidJson<NeuroQueryRequest>,
) -> Result<Json<Value>, ApiError> {
    let layout = &state.layout;
    let store = roko_neuro::knowledge_store::KnowledgeStore::for_layout(layout);

    let results = store
        .query(&body.query, body.limit)
        .map_err(|e| ApiError::internal(format!("neuro query failed: {e}")))?;

    let total = results.len();
    let entries: Vec<Value> = results
        .into_iter()
        .map(|entry| {
            json!({
                "id": entry.id,
                "content": entry.content,
                "kind": format!("{:?}", entry.kind),
                "tier": format!("{:?}", entry.tier),
                "relevance": entry.confidence,
                "created_at": entry.created_at.to_rfc3339(),
            })
        })
        .collect();

    Ok(Json(json!({
        "results": entries,
        "total": total,
    })))
}

/// Query params for the GET knowledge alias.
#[derive(Debug, Deserialize)]
struct KnowledgeQueryParams {
    #[serde(default)]
    q: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

/// `GET /api/knowledge?q=<topic>&limit=N` — alias for neuro query.
async fn knowledge_query(
    State(state): State<Arc<AppState>>,
    Query(params): Query<KnowledgeQueryParams>,
) -> Result<Json<Value>, ApiError> {
    if params.q.trim().is_empty() {
        return Ok(Json(json!({ "results": [], "total": 0 })));
    }

    let layout = &state.layout;
    let store = roko_neuro::knowledge_store::KnowledgeStore::for_layout(layout);

    let results = store
        .query(&params.q, params.limit)
        .map_err(|e| ApiError::internal(format!("knowledge query failed: {e}")))?;

    let total = results.len();
    let entries: Vec<Value> = results
        .into_iter()
        .map(|entry| {
            json!({
                "id": entry.id,
                "content": entry.content,
                "kind": format!("{:?}", entry.kind),
                "tier": format!("{:?}", entry.tier),
                "relevance": entry.confidence,
                "created_at": entry.created_at.to_rfc3339(),
            })
        })
        .collect();

    Ok(Json(json!({
        "results": entries,
        "total": total,
    })))
}
