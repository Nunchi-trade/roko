//! Connector management routes.
//!
//! - `GET    /api/connectors`              — list all connectors
//! - `POST   /api/connectors`              — register a connector
//! - `DELETE /api/connectors/{name}`       — unregister a connector
//! - `GET    /api/connectors/{name}/health` — health status for a single connector

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use roko_core::connector::{ConnectorHealth, ConnectorInfo, ConnectorKind, ConnectorStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/connectors", get(list_connectors).post(create_connector))
        .route(
            "/connectors/{name}",
            axum::routing::delete(delete_connector),
        )
        .route("/connectors/{name}/health", get(connector_health))
}

// ── Request / Response types ──────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateConnectorRequest {
    name: String,
    kind: ConnectorKind,
    endpoint: String,
    #[serde(default)]
    metadata: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct ConnectorListResponse {
    connectors: Vec<ConnectorInfo>,
    total: usize,
    healthy: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct DeleteConnectorResponse {
    name: String,
    deleted: bool,
}

// ── Handlers ──────────────────────────────────────────────────────

/// `GET /api/connectors` — list all registered connectors.
async fn list_connectors(State(state): State<Arc<AppState>>) -> Json<ConnectorListResponse> {
    let reg = state.connectors.read().await;
    let connectors: Vec<ConnectorInfo> = reg.list().to_vec();
    let total = connectors.len();
    let healthy = reg.healthy_count();
    Json(ConnectorListResponse {
        connectors,
        total,
        healthy,
    })
}

/// `POST /api/connectors` — register a new connector.
async fn create_connector(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateConnectorRequest>,
) -> Result<(StatusCode, Json<ConnectorInfo>), ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::bad_request("connector name must not be empty"));
    }
    if req.endpoint.trim().is_empty() {
        return Err(ApiError::bad_request(
            "connector endpoint must not be empty",
        ));
    }

    let now = Utc::now();
    let info = ConnectorInfo {
        name: req.name,
        kind: req.kind,
        health: ConnectorHealth {
            status: ConnectorStatus::Connected,
            latency_ms: 0,
            last_check: now,
        },
        created_at: now,
        metadata: req.metadata,
    };

    let mut reg = state.connectors.write().await;
    reg.register(info.clone());

    Ok((StatusCode::CREATED, Json(info)))
}

/// `DELETE /api/connectors/{name}` — unregister a connector.
async fn delete_connector(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<DeleteConnectorResponse>, ApiError> {
    let mut reg = state.connectors.write().await;
    let deleted = reg.unregister(&name);
    if !deleted {
        return Err(ApiError::not_found(format!("connector '{name}' not found")));
    }
    Ok(Json(DeleteConnectorResponse { name, deleted }))
}

/// `GET /api/connectors/{name}/health` — health status for one connector.
async fn connector_health(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<ConnectorHealth>, ApiError> {
    let reg = state.connectors.read().await;
    let info = reg
        .get(&name)
        .ok_or_else(|| ApiError::not_found(format!("connector '{name}' not found")))?;
    Ok(Json(info.health.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::{Body, to_bytes};
    use axum::http::Request;
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
    async fn list_connectors_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());

        let response = routes()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/connectors")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: ConnectorListResponse = serde_json::from_slice(&body).expect("parse");
        assert!(payload.connectors.is_empty());
        assert_eq!(payload.total, 0);
        assert_eq!(payload.healthy, 0);
    }

    #[tokio::test]
    async fn create_then_list_connectors() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());

        let app = routes().with_state(Arc::clone(&state));

        // Create a connector.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/connectors")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "name": "my-api",
                            "kind": "api",
                            "endpoint": "https://example.com/api"
                        }))
                        .unwrap(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let created: ConnectorInfo = serde_json::from_slice(&body).expect("parse");
        assert_eq!(created.name, "my-api");

        // List should show 1 connector.
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/connectors")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: ConnectorListResponse = serde_json::from_slice(&body).expect("parse");
        assert_eq!(payload.total, 1);
        assert_eq!(payload.healthy, 1);
        assert_eq!(payload.connectors[0].name, "my-api");
    }

    #[tokio::test]
    async fn delete_connector_success() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());
        let app = routes().with_state(Arc::clone(&state));

        // Create first.
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/connectors")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"name":"rmv","kind":"mcp","endpoint":"stdio://test"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        // Delete.
        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/connectors/rmv")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: DeleteConnectorResponse = serde_json::from_slice(&body).expect("parse");
        assert!(payload.deleted);
    }

    #[tokio::test]
    async fn delete_connector_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());

        let response = routes()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/connectors/ghost")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn health_returns_status() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());
        let app = routes().with_state(Arc::clone(&state));

        // Register.
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/connectors")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"name":"hc","kind":"database","endpoint":"postgres://localhost"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        // Check health.
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/connectors/hc/health")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let health: ConnectorHealth = serde_json::from_slice(&body).expect("parse");
        assert_eq!(health.status, ConnectorStatus::Connected);
    }

    #[tokio::test]
    async fn health_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());

        let response = routes()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/connectors/nope/health")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
