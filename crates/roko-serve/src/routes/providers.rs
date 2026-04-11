//! Provider inventory endpoints.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::state::AppState;
use roko_learn::provider_health::{HealthState, ProviderStatus};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/providers", get(list_providers))
        .route("/models", get(list_models))
}

/// `GET /api/providers` — list configured providers with health and model counts.
async fn list_providers(State(state): State<Arc<AppState>>) -> Json<ProvidersResponse> {
    let config = state.roko_config.read().await;
    let providers = config.effective_providers();
    let models = config.effective_models();
    drop(config);

    let health: HashMap<String, ProviderHealthInfo> = state
        .provider_health
        .snapshot()
        .into_iter()
        .map(|status| (status.provider.clone(), ProviderHealthInfo::from(status)))
        .collect();

    let mut providers: Vec<ProviderInfo> = providers
        .iter()
        .map(|(id, provider_config)| ProviderInfo {
            id: id.clone(),
            kind: provider_config.kind.label().to_string(),
            base_url: provider_config.base_url.clone(),
            has_api_key: provider_config.resolve_api_key().is_some(),
            health: health.get(id).cloned(),
            model_count: models
                .values()
                .filter(|model| &model.provider == id)
                .count(),
        })
        .collect();

    providers.sort_by(|a, b| a.id.cmp(&b.id));

    Json(ProvidersResponse { providers })
}

/// `GET /api/models` — list configured models with capabilities and pricing.
async fn list_models(State(state): State<Arc<AppState>>) -> Json<ModelsResponse> {
    let config = state.roko_config.read().await;
    let mut models: Vec<ModelInfo> = config
        .effective_models()
        .into_iter()
        .map(|(key, profile)| ModelInfo {
            key,
            slug: profile.slug,
            provider: profile.provider,
            context_window: profile.context_window,
            supports_tools: profile.supports_tools,
            supports_thinking: profile.supports_thinking,
            supports_vision: profile.supports_vision,
            cost_input_per_m: profile.cost_input_per_m,
            cost_output_per_m: profile.cost_output_per_m,
        })
        .collect();

    models.sort_by(|a, b| a.key.cmp(&b.key));

    Json(ModelsResponse { models })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProvidersResponse {
    providers: Vec<ProviderInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelsResponse {
    models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderInfo {
    id: String,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
    has_api_key: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    health: Option<ProviderHealthInfo>,
    model_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelInfo {
    key: String,
    slug: String,
    provider: String,
    context_window: u64,
    supports_tools: bool,
    supports_thinking: bool,
    supports_vision: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_input_per_m: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_output_per_m: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderHealthInfo {
    state: String,
    consecutive_failures: u32,
    total_attempts: u64,
    total_successes: u64,
}

impl From<ProviderStatus> for ProviderHealthInfo {
    fn from(status: ProviderStatus) -> Self {
        Self {
            state: match status.state {
                HealthState::Healthy => "healthy",
                HealthState::Unhealthy { .. } => "unhealthy",
                HealthState::Probing => "probing",
            }
            .to_string(),
            consecutive_failures: status.consecutive_failures,
            total_attempts: status.total_attempts,
            total_successes: status.total_successes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::deploy::create_backend;
    use crate::runtime::NoOpRuntime;

    #[tokio::test]
    async fn list_providers_returns_configured_providers_with_health() {
        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let mut config = roko_core::config::schema::RokoConfig::default();
        config.providers.insert(
            "zai".into(),
            roko_core::config::schema::ProviderConfig {
                kind: roko_core::agent::ProviderKind::OpenAiCompat,
                base_url: Some("https://api.z.ai/api/paas/v4".into()),
                api_key_env: Some("ZAI_API_KEY".into()),
                command: None,
                args: None,
                timeout_ms: Some(120_000),
                ttft_timeout_ms: Some(15_000),
                connect_timeout_ms: Some(5_000),
                extra_headers: None,
                max_concurrent: None,
            },
        );
        config.models.insert(
            "glm-5-1".into(),
            roko_core::config::schema::ModelProfile {
                provider: "zai".into(),
                slug: "glm-5.1".into(),
                context_window: 128_000,
                max_output: None,
                supports_tools: true,
                supports_thinking: false,
                supports_vision: false,
                supports_web_search: false,
                supports_mcp_tools: false,
                supports_partial: false,
                supports_grounding: false,
                supports_code_execution: false,
                supports_caching: false,
                provider_routing: None,
                tool_format: "openai_json".into(),
                cost_input_per_m: None,
                cost_output_per_m: None,
                cost_input_per_m_high: None,
                cost_output_per_m_high: None,
                cost_cache_read_per_m: None,
                cost_cache_write_per_m: None,
                thinking_level: None,
                max_tools: None,
                tokenizer_ratio: None,
                supports_search: false,
                supports_citations: false,
                supports_async: false,
                is_embedding_model: false,
                search_context_size: None,
                cost_per_request: None,
            },
        );

        let state = Arc::new(AppState::new(
            workdir,
            Arc::new(NoOpRuntime),
            config,
            deploy_backend,
        ));
        state.provider_health.record_failure("zai");

        let app = routes().with_state(Arc::clone(&state));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/providers")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let response: ProvidersResponse = serde_json::from_slice(&body).expect("parse response");

        assert_eq!(response.providers.len(), 1);
        assert_eq!(response.providers[0].id, "zai");
        assert_eq!(response.providers[0].kind, "openai_compat");
        assert_eq!(response.providers[0].model_count, 1);
        assert!(!response.providers[0].has_api_key);

        let health = response.providers[0]
            .health
            .as_ref()
            .expect("provider health should be present");
        assert_eq!(health.state, "healthy");
        assert_eq!(health.consecutive_failures, 1);
        assert_eq!(health.total_attempts, 1);
        assert_eq!(health.total_successes, 0);
    }

    #[tokio::test]
    async fn list_models_returns_configured_models_with_capabilities_and_costs() {
        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let mut config = roko_core::config::schema::RokoConfig::default();
        config.models.insert(
            "glm-5-1".into(),
            roko_core::config::schema::ModelProfile {
                provider: "zai".into(),
                slug: "glm-5.1".into(),
                context_window: 128_000,
                max_output: None,
                supports_tools: true,
                supports_thinking: true,
                supports_vision: true,
                supports_web_search: false,
                supports_mcp_tools: false,
                supports_partial: false,
                supports_grounding: false,
                supports_code_execution: false,
                supports_caching: false,
                provider_routing: None,
                tool_format: "openai_json".into(),
                cost_input_per_m: Some(1.40),
                cost_output_per_m: Some(4.40),
                cost_input_per_m_high: None,
                cost_output_per_m_high: None,
                cost_cache_read_per_m: None,
                cost_cache_write_per_m: None,
                thinking_level: None,
                max_tools: None,
                tokenizer_ratio: None,
                supports_search: false,
                supports_citations: false,
                supports_async: false,
                is_embedding_model: false,
                search_context_size: None,
                cost_per_request: None,
            },
        );

        let state = Arc::new(AppState::new(
            workdir,
            Arc::new(NoOpRuntime),
            config,
            deploy_backend,
        ));

        let app = routes().with_state(Arc::clone(&state));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/models")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let response: ModelsResponse = serde_json::from_slice(&body).expect("parse response");

        assert_eq!(response.models.len(), 1);
        let model = &response.models[0];
        assert_eq!(model.key, "glm-5-1");
        assert_eq!(model.slug, "glm-5.1");
        assert_eq!(model.provider, "zai");
        assert_eq!(model.context_window, 128_000);
        assert!(model.supports_tools);
        assert!(model.supports_thinking);
        assert!(model.supports_vision);
        assert_eq!(model.cost_input_per_m, Some(1.40));
        assert_eq!(model.cost_output_per_m, Some(4.40));
    }
}
