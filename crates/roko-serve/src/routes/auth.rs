//! API key management routes.
//!
//! - `POST   /api/api-keys`        — create a new named API key
//! - `GET    /api/api-keys`        — list all keys (metadata only, never the key itself)
//! - `DELETE /api/api-keys/:name`  — revoke a key by name
//!
//! Keys are stored as SHA-256 hashes in `.roko/api-keys.json`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use chrono::Utc;
use roko_core::config::ApiKeyEntry;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::middleware::hash_api_key;
use crate::error::ApiError;
use crate::state::AppState;

/// Request payload for creating a new API key.
#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    /// Human-readable name for the key (must be unique).
    pub name: String,
    /// Permission scope: "admin", "agent:write", "read", etc.
    #[serde(default = "default_scope")]
    pub scope: String,
    /// Optional ISO 8601 expiry timestamp.
    #[serde(default)]
    pub expires_at: Option<String>,
}

fn default_scope() -> String {
    "admin".into()
}

/// Response returned when a new key is created.
/// The plaintext key is returned **once** and never stored.
#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub name: String,
    pub key: String,
    pub scope: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// Summary of a stored key (no secret material).
#[derive(Debug, Serialize)]
pub struct ApiKeySummary {
    pub name: String,
    pub scope: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api-keys", get(list_api_keys).post(create_api_key))
        .route("/api-keys/{name}", delete(revoke_api_key))
}

fn api_keys_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("api-keys.json")
}

fn load_api_keys(workdir: &Path) -> Vec<ApiKeyEntry> {
    let path = api_keys_path(workdir);
    match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save_api_keys(workdir: &Path, keys: &[ApiKeyEntry]) -> Result<(), ApiError> {
    let path = api_keys_path(workdir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ApiError::internal(format!("failed to create api-keys directory: {e}")))?;
    }
    let data = serde_json::to_string_pretty(keys)
        .map_err(|e| ApiError::internal(format!("failed to serialize api-keys: {e}")))?;
    std::fs::write(&path, data)
        .map_err(|e| ApiError::internal(format!("failed to write api-keys.json: {e}")))?;
    Ok(())
}

/// `POST /api/api-keys` — generate a new API key, store its SHA-256 hash,
/// and return the plaintext key exactly once.
async fn create_api_key(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<CreateApiKeyResponse>), ApiError> {
    if req.name.is_empty() {
        return Err(ApiError::bad_request("name must not be empty"));
    }
    crate::error::validate_path_segment(&req.name, "key name")?;

    let mut keys = load_api_keys(&state.workdir);

    if keys.iter().any(|k| k.name == req.name) {
        return Err(ApiError::conflict(format!(
            "API key with name '{}' already exists",
            req.name
        )));
    }

    // Generate a random plaintext key: `roko_` prefix + UUID (no hyphens).
    let plaintext = format!("roko_{}", Uuid::new_v4().as_simple());
    let key_hash = hash_api_key(&plaintext);
    let created_at = Utc::now().to_rfc3339();

    let entry = ApiKeyEntry {
        name: req.name.clone(),
        key_hash,
        scope: req.scope.clone(),
        created_at: created_at.clone(),
        expires_at: req.expires_at.clone(),
    };

    keys.push(entry);
    save_api_keys(&state.workdir, &keys)?;

    // Also push the entry into the live ServeAuthConfig so the middleware
    // picks it up immediately without a server restart.
    {
        let mut cfg = state.load_roko_config().as_ref().clone();
        cfg.serve.auth.api_keys = keys;
        state.store_roko_config(cfg);
    }

    Ok((
        StatusCode::CREATED,
        Json(CreateApiKeyResponse {
            name: req.name,
            key: plaintext,
            scope: req.scope,
            created_at,
            expires_at: req.expires_at,
        }),
    ))
}

/// `GET /api/api-keys` — list all stored API keys (metadata only).
async fn list_api_keys(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let keys = load_api_keys(&state.workdir);
    let summaries: Vec<ApiKeySummary> = keys
        .into_iter()
        .map(|k| ApiKeySummary {
            name: k.name,
            scope: k.scope,
            created_at: k.created_at,
            expires_at: k.expires_at,
        })
        .collect();
    Json(json!({ "keys": summaries }))
}

/// `DELETE /api/api-keys/:name` — revoke an API key by name.
async fn revoke_api_key(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let mut keys = load_api_keys(&state.workdir);
    let before = keys.len();
    keys.retain(|k| k.name != name);
    if keys.len() == before {
        return Err(ApiError::not_found(format!(
            "API key with name '{name}' not found"
        )));
    }
    save_api_keys(&state.workdir, &keys)?;

    // Update live config.
    {
        let mut cfg = state.load_roko_config().as_ref().clone();
        cfg.serve.auth.api_keys = keys;
        state.store_roko_config(cfg);
    }

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_api_key_is_deterministic() {
        let hash1 = hash_api_key("test-key-123");
        let hash2 = hash_api_key("test-key-123");
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64); // SHA-256 hex = 64 chars
    }

    #[test]
    fn hash_api_key_differs_for_different_inputs() {
        let hash1 = hash_api_key("key-a");
        let hash2 = hash_api_key("key-b");
        assert_ne!(hash1, hash2);
    }
}
