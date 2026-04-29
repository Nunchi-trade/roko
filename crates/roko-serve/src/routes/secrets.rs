//! Secret management endpoints.
//!
//! Provides CRUD routes for secrets stored in `.roko/secrets.toml` via
//! [`roko_core::secrets::FileStore`]. Secret **values** are never returned
//! in responses; the list endpoint only reports which namespaces have keys
//! configured.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use roko_core::secrets::namespace::Namespace;
use roko_core::secrets::{FileStore, SecretStore};

use crate::error::ApiError;
use crate::extract::ApiJson;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/secrets", get(list_secrets))
        .route("/secrets/{namespace}/{key}", post(set_secret))
        .route("/secrets/{namespace}/{key}", delete(delete_secret))
        .route("/secrets/{namespace}/{key}/test", post(test_secret))
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct SecretEntry {
    namespace: String,
    source: String,
    configured: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct ListSecretsResponse {
    secrets: Vec<SecretEntry>,
}

#[derive(Debug, Deserialize)]
struct SetSecretBody {
    value: String,
}

#[derive(Debug, Serialize)]
struct SetSecretResponse {
    namespace: String,
    key: String,
    status: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DeleteSecretResponse {
    namespace: String,
    key: String,
    deleted: bool,
}

#[derive(Debug, Serialize)]
struct TestSecretResponse {
    status: String,
    message: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn open_file_store(state: &AppState) -> Result<FileStore, ApiError> {
    let secrets_path = state.layout.root().join("secrets.toml");
    FileStore::open(&secrets_path)
        .map_err(|e| ApiError::internal(format!("open secrets store: {e}")))
}

fn parse_namespace(namespace: &str, key: &str) -> Result<Namespace, ApiError> {
    let combined = format!("{namespace}.{key}");
    Namespace::parse(&combined)
        .map_err(|e| ApiError::bad_request(format!("invalid namespace: {e}")))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/secrets` -- list all configured secret namespaces (no values).
async fn list_secrets(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ListSecretsResponse>, ApiError> {
    let store = open_file_store(&state)?;
    let entries: Vec<SecretEntry> = store
        .list_entries()
        .into_iter()
        .map(|(category, provider)| SecretEntry {
            namespace: format!("{category}.{provider}"),
            source: store.name().to_string(),
            configured: true,
        })
        .collect();
    Ok(Json(ListSecretsResponse { secrets: entries }))
}

/// `POST /api/secrets/:namespace/:key` -- set a secret value.
async fn set_secret(
    State(state): State<Arc<AppState>>,
    Path((namespace, key)): Path<(String, String)>,
    ApiJson(body): ApiJson<SetSecretBody>,
) -> Result<Json<SetSecretResponse>, ApiError> {
    let ns = parse_namespace(&namespace, &key)?;
    if body.value.is_empty() {
        return Err(ApiError::bad_request("secret value must not be empty"));
    }
    let store = open_file_store(&state)?;
    store
        .set(&ns, body.value)
        .map_err(|e| ApiError::internal(format!("write secret: {e}")))?;
    Ok(Json(SetSecretResponse {
        namespace,
        key,
        status: "created".into(),
    }))
}

/// `DELETE /api/secrets/:namespace/:key` -- remove a secret.
async fn delete_secret(
    State(state): State<Arc<AppState>>,
    Path((namespace, key)): Path<(String, String)>,
) -> Result<Json<DeleteSecretResponse>, ApiError> {
    let ns = parse_namespace(&namespace, &key)?;
    let store = open_file_store(&state)?;
    let deleted = store
        .delete(&ns)
        .map_err(|e| ApiError::internal(format!("delete secret: {e}")))?;
    Ok(Json(DeleteSecretResponse {
        namespace,
        key,
        deleted,
    }))
}

/// `POST /api/secrets/:namespace/:key/test` -- test if a secret is valid.
///
/// For LLM providers, makes a lightweight API call (e.g. list-models) with the
/// stored key. Returns `"valid"`, `"invalid"`, or `"error"`.
async fn test_secret(
    State(state): State<Arc<AppState>>,
    Path((namespace, key)): Path<(String, String)>,
) -> Result<Json<TestSecretResponse>, ApiError> {
    let ns = parse_namespace(&namespace, &key)?;
    let store = open_file_store(&state)?;
    let secret = store
        .get(&ns)
        .map_err(|e| ApiError::internal(format!("read secret: {e}")))?
        .ok_or_else(|| {
            ApiError::not_found(format!("no secret configured for {}.{}", namespace, key))
        })?;

    let (status, message) = test_provider_key(&namespace, &key, &secret).await;
    Ok(Json(TestSecretResponse { status, message }))
}

/// Make a lightweight health-check call for known providers.
async fn test_provider_key(namespace: &str, key: &str, secret: &str) -> (String, String) {
    match (namespace, key) {
        ("llm", "anthropic") => test_anthropic(secret).await,
        ("llm", "openai") => test_openai(secret).await,
        ("llm", "gemini") => test_gemini(secret).await,
        ("llm", "perplexity") => test_perplexity(secret).await,
        _ => (
            "error".into(),
            format!("no test implemented for {namespace}.{key}"),
        ),
    }
}

async fn test_anthropic(key: &str) -> (String, String) {
    let client = reqwest::Client::new();
    match client
        .get("https://api.anthropic.com/v1/models")
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            ("valid".into(), "Anthropic API key is valid".into())
        }
        Ok(resp) if resp.status().as_u16() == 401 => (
            "invalid".into(),
            "Anthropic API key is invalid (401)".into(),
        ),
        Ok(resp) => (
            "error".into(),
            format!("Anthropic API returned status {}", resp.status()),
        ),
        Err(e) => ("error".into(), format!("request failed: {e}")),
    }
}

async fn test_openai(key: &str) -> (String, String) {
    let client = reqwest::Client::new();
    match client
        .get("https://api.openai.com/v1/models")
        .bearer_auth(key)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            ("valid".into(), "OpenAI API key is valid".into())
        }
        Ok(resp) if resp.status().as_u16() == 401 => {
            ("invalid".into(), "OpenAI API key is invalid (401)".into())
        }
        Ok(resp) => (
            "error".into(),
            format!("OpenAI API returned status {}", resp.status()),
        ),
        Err(e) => ("error".into(), format!("request failed: {e}")),
    }
}

async fn test_gemini(key: &str) -> (String, String) {
    let client = reqwest::Client::new();
    let url = format!("https://generativelanguage.googleapis.com/v1beta/models?key={key}");
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            ("valid".into(), "Gemini API key is valid".into())
        }
        Ok(resp) if resp.status().as_u16() == 400 || resp.status().as_u16() == 403 => (
            "invalid".into(),
            format!("Gemini API key is invalid ({})", resp.status()),
        ),
        Ok(resp) => (
            "error".into(),
            format!("Gemini API returned status {}", resp.status()),
        ),
        Err(e) => ("error".into(), format!("request failed: {e}")),
    }
}

async fn test_perplexity(key: &str) -> (String, String) {
    let client = reqwest::Client::new();
    match client
        .get("https://api.perplexity.ai/models")
        .bearer_auth(key)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            ("valid".into(), "Perplexity API key is valid".into())
        }
        Ok(resp) if resp.status().as_u16() == 401 => (
            "invalid".into(),
            "Perplexity API key is invalid (401)".into(),
        ),
        Ok(resp) => (
            "error".into(),
            format!("Perplexity API returned status {}", resp.status()),
        ),
        Err(e) => ("error".into(), format!("request failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use roko_core::config::schema::RokoConfig;
    use tower::ServiceExt;

    use crate::deploy::create_backend;
    use crate::runtime::NoOpRuntime;

    fn test_state(workdir: std::path::PathBuf) -> Arc<AppState> {
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        Arc::new(AppState::new(
            workdir,
            Arc::new(NoOpRuntime),
            RokoConfig::default(),
            deploy_backend,
        ).expect("AppState::new"))
    }

    #[tokio::test]
    async fn list_secrets_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let roko_dir = dir.path().join(".roko");
        std::fs::create_dir_all(&roko_dir).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());

        let response = routes()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/secrets")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: ListSecretsResponse = serde_json::from_slice(&body).expect("parse");
        assert!(payload.secrets.is_empty());
    }

    #[tokio::test]
    async fn set_then_list_secrets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let roko_dir = dir.path().join(".roko");
        std::fs::create_dir_all(&roko_dir).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());

        let app = routes().with_state(Arc::clone(&state));

        // Set a secret.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/secrets/llm/anthropic")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"value":"sk-test"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);

        // List should show the entry.
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/secrets")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: ListSecretsResponse = serde_json::from_slice(&body).expect("parse");
        assert_eq!(payload.secrets.len(), 1);
        assert_eq!(payload.secrets[0].namespace, "llm.anthropic");
        assert!(payload.secrets[0].configured);
    }

    #[tokio::test]
    async fn delete_nonexistent_secret() {
        let dir = tempfile::tempdir().expect("tempdir");
        let roko_dir = dir.path().join(".roko");
        std::fs::create_dir_all(&roko_dir).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());

        let response = routes()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/secrets/llm/anthropic")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: DeleteSecretResponse = serde_json::from_slice(&body).expect("parse");
        assert!(!payload.deleted);
    }

    #[tokio::test]
    async fn set_empty_value_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let roko_dir = dir.path().join(".roko");
        std::fs::create_dir_all(&roko_dir).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());

        let response = routes()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/secrets/llm/anthropic")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"value":""}"#))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
