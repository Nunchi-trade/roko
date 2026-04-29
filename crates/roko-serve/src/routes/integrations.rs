//! Integration catalog endpoints.
//!
//! `GET /api/integrations` — list all registered service integrations.
//! `GET /api/integrations/:name` — get details for a specific integration.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, State};
use axum::routing::get;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::integrations::{IntegrationRegistry, ServiceIntegration};
use crate::state::AppState;

/// Build the integrations router.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/integrations", get(list_integrations))
        .route("/integrations/{name}", get(get_integration))
}

#[derive(Serialize, Deserialize)]
struct IntegrationListResponse {
    integrations: Vec<ServiceIntegration>,
    total: usize,
}

/// `GET /api/integrations` — list all registered service integrations
/// across all three layers.
async fn list_integrations(
    State(_state): State<Arc<AppState>>,
) -> axum::Json<IntegrationListResponse> {
    let registry = IntegrationRegistry::with_builtins();
    let integrations: Vec<ServiceIntegration> = registry.list().into_iter().cloned().collect();
    let total = integrations.len();
    axum::Json(IntegrationListResponse {
        integrations,
        total,
    })
}

/// `GET /api/integrations/:name` — get details for a specific integration.
async fn get_integration(
    State(_state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<axum::Json<ServiceIntegration>, ApiError> {
    let registry = IntegrationRegistry::with_builtins();
    registry
        .get(&name)
        .cloned()
        .map(axum::Json)
        .ok_or_else(|| ApiError::not_found(format!("integration `{name}` not found")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::deploy::create_backend;
    use crate::runtime::NoOpRuntime;
    use roko_core::config::schema::RokoConfig;

    fn test_state() -> Arc<AppState> {
        let dir = tempfile::tempdir().expect("tempdir");
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        Arc::new(
            AppState::new(
                dir.path().to_path_buf(),
                Arc::new(NoOpRuntime),
                RokoConfig::default(),
                deploy_backend,
            )
            .expect("AppState::new"),
        )
    }

    #[tokio::test]
    async fn list_integrations_returns_builtins() {
        let state = test_state();
        let response = routes()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .uri("/integrations")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: IntegrationListResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.total >= 6);
        assert!(resp.integrations.iter().any(|i| i.name == "github"));
    }

    #[tokio::test]
    async fn get_integration_returns_github() {
        let state = test_state();
        let response = routes()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .uri("/integrations/github")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_unknown_integration_returns_404() {
        let state = test_state();
        let response = routes()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .uri("/integrations/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
