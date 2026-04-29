//! Provider inventory endpoints.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;
use roko_agent::provider::{AgentOptions, create_agent_for_model};
use roko_core::agent::{AgentRole, ModelTier, resolve_model};
use roko_core::config::schema::{ModelProfile, RokoConfig};
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_core::{Body as SignalBody, Context, Engram, Kind};
use roko_learn::cascade_router::CascadeRouter;
use roko_learn::model_router::RoutingContext;
use roko_learn::provider_health::{HealthState, ProviderStatus};

const PROVIDER_TEST_PROMPT: &str = "Say hello.";

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_providers))
        .route("/{id}/health", get(provider_health))
        .route("/{id}/test", post(test_provider))
}

pub fn models_router() -> Router<Arc<AppState>> {
    Router::new().route("/", get(list_models))
}

pub fn routing_router() -> Router<Arc<AppState>> {
    Router::new().route("/explain", get(explain_routing))
}

#[cfg(test)]
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .nest("/providers", router())
        .nest("/models", models_router())
        .nest("/routing", routing_router())
}

/// `GET /api/providers` — list configured providers with health and model counts.
async fn list_providers(State(state): State<Arc<AppState>>) -> Json<ProvidersResponse> {
    let config = state.load_roko_config();
    let providers = config.effective_providers();
    let models = config.effective_models();

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
    let config = state.load_roko_config();
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

/// `GET /api/routing/explain` — explain the current routing decision.
async fn explain_routing(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RoutingExplainParams>,
) -> Result<Json<RoutingExplanation>, ApiError> {
    let model = params
        .model
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("missing query parameter: model"))?;
    let role = params
        .role
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("missing query parameter: role"))?;

    let role = parse_agent_role(role)
        .ok_or_else(|| ApiError::bad_request(format!("invalid role: {role}")))?;

    let config = state.load_roko_config().as_ref().clone();
    let resolved = resolve_model(&config, model);
    let complexity = params
        .complexity
        .as_deref()
        .map(parse_complexity_band)
        .transpose()?
        .unwrap_or_else(|| default_complexity_for_role(role));

    let effective_models = config.effective_models();
    let model_catalog = build_model_catalog(&effective_models);
    let mut model_slugs: Vec<String> = model_catalog.keys().cloned().collect();
    if model_slugs.is_empty() {
        model_slugs.push(resolved.slug.clone());
    }
    model_slugs.sort();

    let routing_ctx = RoutingContext {
        task_category: TaskCategory::Implementation,
        complexity,
        iteration: 1,
        role,
        crate_familiarity: 0.5,
        has_prior_failure: false,
        conductor_load: 0.0,
        active_agents: 0,
        ready_queue_depth: 0,
        max_queue_wait_hours: 0.0,
        daimon_policy: roko_core::DaimonPolicy::default(),
        thinking_level: None,
        temperament: None,
        previous_model: Some(resolved.slug.clone()),
        plan_context_tokens: None,
        tier_thresholds: None,
    };

    let cascade_path = state.workdir.join(".roko/learn/cascade-router.json");
    let router = CascadeRouter::load_or_new(&cascade_path, model_slugs.clone());
    let all_explanation = router.explain_routing(&routing_ctx, &model_slugs);

    let mut health_by_provider: HashMap<String, ProviderStatus> = HashMap::new();
    for provider in model_catalog.values().map(|entry| entry.provider.as_str()) {
        health_by_provider
            .entry(provider.to_owned())
            .or_insert_with(|| state.provider_health.get(provider));
    }

    let available_candidates: Vec<String> = model_slugs
        .iter()
        .filter(|slug| {
            model_catalog
                .get(slug.as_str())
                .map(|entry| provider_status_available(&health_by_provider[&entry.provider]))
                .unwrap_or(true)
        })
        .cloned()
        .collect();

    let eligible_explanation = (!available_candidates.is_empty())
        .then(|| router.explain_routing(&routing_ctx, &available_candidates));
    let selected = eligible_explanation.as_ref().map_or_else(
        || all_explanation.selected_model.clone(),
        |explanation| explanation.selected_model.clone(),
    );
    let selected_model = model_slugs
        .iter()
        .find(|slug| routing_slugs_match(slug, &selected))
        .cloned()
        .unwrap_or(selected);

    let score_by_model: HashMap<_, _> = all_explanation
        .candidates
        .into_iter()
        .map(|candidate| (candidate.model.clone(), candidate))
        .collect();

    let fallback_model = eligible_explanation.as_ref().map_or_else(
        || all_explanation.fallback_model.clone(),
        |explanation| explanation.fallback_model.clone(),
    );

    let mut candidates: Vec<_> = model_slugs
        .iter()
        .map(|slug| {
            let detail = score_by_model.get(slug).cloned();
            let model_info = model_catalog.get(slug.as_str());
            let provider = model_info
                .map(|entry| entry.provider.clone())
                .unwrap_or_else(|| {
                    resolved.profile.as_ref().map_or_else(
                        || resolved.provider_kind.label().to_string(),
                        |profile| profile.provider.clone(),
                    )
                });
            let health = health_by_provider
                .get(&provider)
                .map(|status| provider_state_label(status.state).to_string())
                .unwrap_or_else(|| "healthy".to_string());

            RoutingCandidate {
                model_key: model_info.and_then(|entry| entry.model_key.clone()),
                model_slug: slug.clone(),
                provider,
                score: detail.as_ref().map_or(0.0, |candidate| candidate.score),
                selected: routing_slugs_match(slug, &selected_model),
                eligible: available_candidates
                    .iter()
                    .any(|candidate| candidate == slug),
                health,
                cache_affinity: detail
                    .as_ref()
                    .is_some_and(|candidate| candidate.cache_affinity),
                pareto_optimal: detail.and_then(|candidate| candidate.pareto_optimal),
            }
        })
        .collect();

    candidates.sort_by(|a, b| {
        b.selected
            .cmp(&a.selected)
            .then_with(|| b.score.total_cmp(&a.score))
            .then_with(|| a.model_slug.cmp(&b.model_slug))
    });

    Ok(Json(RoutingExplanation {
        requested_model: model.to_string(),
        resolved_model: resolved.slug,
        role: role.label().to_string(),
        complexity: complexity.label().to_string(),
        stage: all_explanation.stage.label().to_string(),
        selected_model,
        fallback_model,
        latency_sla_ms: all_explanation.latency_sla_ms,
        candidates,
    }))
}

/// `GET /api/providers/{id}/health` — detailed health for a specific provider.
async fn provider_health(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
) -> Json<ProviderHealthResponse> {
    let health = state.provider_health.get(&provider_id);
    let latency = state.latency_registry.get_all_for_provider(&provider_id);

    Json(ProviderHealthResponse {
        provider_id,
        state: provider_state_label(health.state).to_string(),
        consecutive_failures: health.consecutive_failures,
        lifetime_attempts: health.total_attempts,
        lifetime_successes: health.total_successes,
        last_success_at: health.last_success_at,
        last_failure_at: health.last_failure_at,
        latency_p50_ms: latency.p50_ms(),
        latency_p95_ms: latency.p95_ms(),
        latency_p99_ms: latency.p99_ms(),
        error_rate: health.error_rate(),
    })
}

/// `POST /api/providers/{id}/test` — send a minimal live request through the provider.
async fn test_provider(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
) -> Result<Json<ProviderTestResponse>, ApiError> {
    let config = state.load_roko_config().as_ref().clone();
    let providers = config.effective_providers();
    let Some(provider) = providers.get(&provider_id) else {
        return Err(ApiError::not_found(format!(
            "provider {provider_id} is not configured"
        )));
    };

    let Some((model_key, model)) = select_test_model(&config, &provider_id) else {
        return Err(ApiError::bad_request(format!(
            "provider {provider_id} has no configured non-embedding models to test"
        )));
    };

    let agent = create_agent_for_model(
        &config,
        &model_key,
        AgentOptions {
            timeout_ms: provider.timeout_ms,
            name: format!("provider-test-{provider_id}"),
            ..Default::default()
        },
    )
    .map_err(|err| {
        ApiError::bad_request(format!(
            "failed to create test agent for provider {provider_id}: {err}"
        ))
    })?;

    let started = Instant::now();
    let result = agent.run(&provider_test_prompt(), &Context::now()).await;
    let fallback_latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let latency_ms = result.usage.wall_ms.max(fallback_latency_ms);
    let output = result
        .output
        .body
        .as_text()
        .ok()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);

    if result.success {
        state.provider_health.record_success(&provider_id);
    } else {
        state.provider_health.record_failure(&provider_id);
    }

    // The probe is a single-shot request, so use the same end-to-end latency
    // for both TTFT and total latency until streaming timings are available.
    state.latency_registry.record(
        &model.slug,
        &provider_id,
        latency_ms as f64,
        latency_ms as f64,
        u64::from(result.usage.output_tokens),
    );

    let response_text = result.success.then(|| output.clone()).flatten();
    let error_text =
        (!result.success).then(|| output.unwrap_or_else(|| "provider test failed".to_string()));

    Ok(Json(ProviderTestResponse {
        provider_id,
        model_key,
        model_slug: model.slug,
        success: result.success,
        latency_ms,
        input_tokens: result.usage.input_tokens,
        output_tokens: result.usage.output_tokens,
        total_tokens: result.usage.total_tokens(),
        response: response_text,
        error: error_text,
    }))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProvidersResponse {
    providers: Vec<ProviderInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelsResponse {
    models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RoutingExplainParams {
    model: Option<String>,
    role: Option<String>,
    complexity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RoutingExplanation {
    requested_model: String,
    resolved_model: String,
    role: String,
    complexity: String,
    stage: String,
    selected_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback_model: Option<String>,
    latency_sla_ms: u64,
    candidates: Vec<RoutingCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RoutingCandidate {
    #[serde(skip_serializing_if = "Option::is_none")]
    model_key: Option<String>,
    model_slug: String,
    provider: String,
    score: f64,
    selected: bool,
    eligible: bool,
    health: String,
    cache_affinity: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pareto_optimal: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderHealthResponse {
    provider_id: String,
    state: String,
    consecutive_failures: u32,
    lifetime_attempts: u64,
    lifetime_successes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_success_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_failure_at: Option<chrono::DateTime<chrono::Utc>>,
    latency_p50_ms: f64,
    latency_p95_ms: f64,
    latency_p99_ms: f64,
    error_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderTestResponse {
    provider_id: String,
    model_key: String,
    model_slug: String,
    success: bool,
    latency_ms: u64,
    input_tokens: u32,
    output_tokens: u32,
    total_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    response: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
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

#[derive(Debug, Clone)]
struct ModelCatalogEntry {
    model_key: Option<String>,
    provider: String,
}

impl From<ProviderStatus> for ProviderHealthInfo {
    fn from(status: ProviderStatus) -> Self {
        Self {
            state: provider_state_label(status.state).to_string(),
            consecutive_failures: status.consecutive_failures,
            total_attempts: status.total_attempts,
            total_successes: status.total_successes,
        }
    }
}

fn provider_state_label(state: HealthState) -> &'static str {
    match state {
        HealthState::Healthy => "healthy",
        HealthState::Unhealthy { .. } => "unhealthy",
        HealthState::Probing => "probing",
    }
}

fn provider_status_available(status: &ProviderStatus) -> bool {
    match status.state {
        HealthState::Healthy => true,
        HealthState::Unhealthy { recovery_at } => Instant::now() >= recovery_at,
        HealthState::Probing => false,
    }
}

fn parse_agent_role(raw: &str) -> Option<AgentRole> {
    [
        AgentRole::Conductor,
        AgentRole::Strategist,
        AgentRole::Implementer,
        AgentRole::Architect,
        AgentRole::Researcher,
        AgentRole::Auditor,
        AgentRole::QuickReviewer,
        AgentRole::Scribe,
        AgentRole::Critic,
        AgentRole::AutoFixer,
        AgentRole::Refactorer,
        AgentRole::PrePlanner,
        AgentRole::DocVerifier,
        AgentRole::IntegrationTester,
        AgentRole::MergeResolver,
        AgentRole::TerminalValidator,
        AgentRole::GolemLifecycleTester,
        AgentRole::SpecDriftDetector,
        AgentRole::RegressionDetector,
        AgentRole::PerformanceSentinel,
        AgentRole::CoverageTracker,
        AgentRole::PlanLifecycleManager,
        AgentRole::CrossSystemTester,
        AgentRole::ErrorDiagnoser,
        AgentRole::DependencyValidator,
        AgentRole::PatternExtractor,
        AgentRole::SnapshotComparator,
        AgentRole::FullLoopValidator,
    ]
    .into_iter()
    .find(|role| role.label() == raw)
}

fn parse_complexity_band(raw: &str) -> Result<TaskComplexityBand, ApiError> {
    match raw {
        "fast" | "mechanical" => Ok(TaskComplexityBand::Fast),
        "complex" | "premium" | "architectural" => Ok(TaskComplexityBand::Complex),
        "standard" | "focused" => Ok(TaskComplexityBand::Standard),
        _ => Err(ApiError::bad_request(format!("invalid complexity: {raw}"))),
    }
}

fn default_complexity_for_role(role: AgentRole) -> TaskComplexityBand {
    match role.model_tier() {
        ModelTier::Fast => TaskComplexityBand::Fast,
        ModelTier::Premium => TaskComplexityBand::Complex,
        ModelTier::Standard | _ => TaskComplexityBand::Standard,
    }
}

fn build_model_catalog(
    models: &HashMap<String, ModelProfile>,
) -> HashMap<String, ModelCatalogEntry> {
    let mut catalog = HashMap::new();
    for (model_key, profile) in models {
        catalog
            .entry(profile.slug.clone())
            .or_insert_with(|| ModelCatalogEntry {
                model_key: Some(model_key.clone()),
                provider: profile.provider.clone(),
            });
    }
    catalog
}

fn routing_slugs_match(lhs: &str, rhs: &str) -> bool {
    lhs == rhs
        || routing_slug_family(lhs).is_some_and(|family| routing_slug_family(rhs) == Some(family))
}

fn routing_slug_family(slug: &str) -> Option<&'static str> {
    if slug.contains("haiku") {
        Some("haiku")
    } else if slug.contains("sonnet") {
        Some("sonnet")
    } else if slug.contains("opus") {
        Some("opus")
    } else if slug.contains("glm") {
        Some("glm")
    } else {
        None
    }
}

fn select_test_model(config: &RokoConfig, provider_id: &str) -> Option<(String, ModelProfile)> {
    let mut models: Vec<_> = config
        .effective_models()
        .into_iter()
        .filter(|(_, model)| model.provider == provider_id && !model.is_embedding_model)
        .collect();
    models.sort_by(|(left, _), (right, _)| left.cmp(right));
    models.into_iter().next()
}

fn provider_test_prompt() -> Engram {
    Engram::builder(Kind::Prompt)
        .body(SignalBody::text(PROVIDER_TEST_PROMPT))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::deploy::create_backend;
    use crate::runtime::NoOpRuntime;

    fn spawn_http_server(
        status_line: &str,
        response: String,
    ) -> (
        String,
        Arc<std::sync::Mutex<Option<String>>>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("server addr");
        let captured = Arc::new(std::sync::Mutex::new(None));
        let captured_request = Arc::clone(&captured);
        let status_line = status_line.to_string();

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set read timeout");

            let mut buf = Vec::new();
            let mut header_end = None;
            let mut content_length = None;

            loop {
                let mut chunk = [0_u8; 1024];
                let n = stream.read(&mut chunk).expect("read request");
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);

                if header_end.is_none()
                    && let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    header_end = Some(pos + 4);
                    let headers = String::from_utf8_lossy(&buf[..pos + 4]);
                    content_length = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    });
                }

                if let (Some(header_end), Some(content_length)) = (header_end, content_length)
                    && buf.len() >= header_end + content_length
                {
                    break;
                }
            }

            let header_end = header_end.expect("request headers");
            let content_length = content_length.expect("content length");
            let request = String::from_utf8_lossy(&buf[..header_end + content_length]).to_string();
            *captured_request.lock().expect("capture lock") = Some(request);

            let response_bytes = response.as_bytes();
            let wire = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_bytes.len(),
                response
            );
            stream.write_all(wire.as_bytes()).expect("write response");
            stream.flush().expect("flush response");
        });

        (format!("http://{}", addr), captured, handle)
    }

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
        ).expect("AppState::new"));
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
        ).expect("AppState::new"));

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

        let model = response
            .models
            .iter()
            .find(|model| model.key == "glm-5-1")
            .expect("glm-5-1 model should be present in the configured model list");
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

    #[tokio::test]
    async fn explain_routing_reports_scores_and_provider_health() {
        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        std::fs::create_dir_all(workdir.join(".roko/learn")).expect("create learn dir");
        std::fs::write(
            workdir.join(".roko/learn/cascade-router.json"),
            serde_json::json!({
                "model_slugs": ["glm-5.1", "claude-sonnet-4-6"],
                "confidence_stats": {
                    "glm-5.1": { "trials": 80, "successes": 72 },
                    "claude-sonnet-4-6": { "trials": 80, "successes": 60 }
                },
                "total_observations": 160
            })
            .to_string(),
        )
        .expect("write cascade snapshot");

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
        config.providers.insert(
            "anthropic".into(),
            roko_core::config::schema::ProviderConfig {
                kind: roko_core::agent::ProviderKind::AnthropicApi,
                base_url: Some("https://api.anthropic.com".into()),
                api_key_env: Some("ANTHROPIC_API_KEY".into()),
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
                context_window: 200_000,
                max_output: None,
                supports_tools: true,
                supports_thinking: true,
                supports_vision: false,
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
        config.models.insert(
            "claude-sonnet-4-6".into(),
            roko_core::config::schema::ModelProfile {
                provider: "anthropic".into(),
                slug: "claude-sonnet-4-6".into(),
                context_window: 200_000,
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
                tool_format: "anthropic_blocks".into(),
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
        ).expect("AppState::new"));
        state.provider_health.record_failure("zai");
        state.provider_health.record_failure("zai");
        state.provider_health.record_failure("zai");

        let app = routes().with_state(Arc::clone(&state));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/routing/explain?model=glm-5-1&role=implementer")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let response: RoutingExplanation =
            serde_json::from_slice(&body).expect("parse routing explanation");

        assert_eq!(response.resolved_model, "glm-5.1");
        assert_eq!(response.role, "implementer");
        assert_eq!(response.complexity, "standard");
        assert_eq!(response.stage, "confidence");
        assert_eq!(response.selected_model, "claude-sonnet-4-6");
        assert_eq!(response.candidates.len(), 2);

        let glm = response
            .candidates
            .iter()
            .find(|candidate| candidate.model_slug == "glm-5.1")
            .expect("glm candidate");
        assert_eq!(glm.provider, "zai");
        assert!(!glm.selected);
        assert!(!glm.eligible);
        assert_eq!(glm.health, "unhealthy");
        assert!(glm.cache_affinity);

        let claude = response
            .candidates
            .iter()
            .find(|candidate| candidate.model_slug == "claude-sonnet-4-6")
            .expect("claude candidate");
        assert_eq!(claude.provider, "anthropic");
        assert!(claude.selected);
        assert!(claude.eligible);
        assert_eq!(claude.health, "healthy");
        assert!(!claude.cache_affinity);
    }

    #[tokio::test]
    async fn provider_health_returns_circuit_state_and_latency_percentiles() {
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

        let state = Arc::new(AppState::new(
            workdir,
            Arc::new(NoOpRuntime),
            config,
            deploy_backend,
        ).expect("AppState::new"));
        state.provider_health.record_success("zai");
        state.provider_health.record_failure("zai");
        state.provider_health.record_failure("zai");
        state.provider_health.record_failure("zai");
        state
            .latency_registry
            .record("glm-5.1", "zai", 10.0, 100.0, 1);
        state
            .latency_registry
            .record("glm-5.1", "zai", 20.0, 200.0, 1);
        state
            .latency_registry
            .record("glm-4.6", "zai", 30.0, 300.0, 1);

        let app = routes().with_state(Arc::clone(&state));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/providers/zai/health")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let response: ProviderHealthResponse =
            serde_json::from_slice(&body).expect("parse response");

        assert_eq!(response.provider_id, "zai");
        assert_eq!(response.state, "unhealthy");
        assert_eq!(response.consecutive_failures, 3);
        assert_eq!(response.lifetime_attempts, 4);
        assert_eq!(response.lifetime_successes, 1);
        assert!(response.last_success_at.is_some());
        assert!(response.last_failure_at.is_some());
        assert_eq!(response.latency_p50_ms, 200.0);
        assert_eq!(response.latency_p95_ms, 300.0);
        assert_eq!(response.latency_p99_ms, 300.0);
        assert_eq!(response.error_rate, 0.75);
    }

    #[tokio::test]
    async fn test_provider_sends_live_request_and_records_metrics() {
        let response = serde_json::json!({
            "id": "chatcmpl-test",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hello"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 9,
                "completion_tokens": 4,
                "total_tokens": 13
            }
        })
        .to_string();
        let (base_url, captured, handle) = spawn_http_server("HTTP/1.1 200 OK", response);

        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let mut config = roko_core::config::schema::RokoConfig::default();
        config.providers.insert(
            "zai".into(),
            roko_core::config::schema::ProviderConfig {
                kind: roko_core::agent::ProviderKind::OpenAiCompat,
                base_url: Some(base_url),
                api_key_env: Some("PATH".into()),
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
        ).expect("AppState::new"));

        let app = routes().with_state(Arc::clone(&state));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/providers/zai/test")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let response: ProviderTestResponse = serde_json::from_slice(&body).expect("parse response");

        assert_eq!(response.provider_id, "zai");
        assert_eq!(response.model_key, "glm-5-1");
        assert_eq!(response.model_slug, "glm-5.1");
        assert!(response.success);
        assert_eq!(response.input_tokens, 9);
        assert_eq!(response.output_tokens, 4);
        assert_eq!(response.total_tokens, 13);
        assert_eq!(response.response.as_deref(), Some("hello"));

        let health = state.provider_health.get("zai");
        assert_eq!(health.total_attempts, 1);
        assert_eq!(health.total_successes, 1);
        assert_eq!(health.consecutive_failures, 0);

        let latency = state
            .latency_registry
            .get("glm-5.1", "zai")
            .expect("latency stats");
        assert_eq!(latency.observations, 1);
        assert_eq!(latency.recent_latencies.len(), 1);
        assert_eq!(
            latency.recent_latencies[0], response.latency_ms as f64,
            "provider test latency should match the value recorded in the latency registry"
        );

        let request = captured
            .lock()
            .expect("capture lock")
            .clone()
            .expect("captured request");
        assert!(
            request.starts_with("POST ")
                && request.contains("/chat/completions")
                && request.contains("HTTP/1.1"),
            "provider probe should send a POST request to the chat completions endpoint: {request}"
        );
        assert!(request.contains("\"content\":\"Say hello.\""));

        handle.join().expect("server thread");
    }

    #[tokio::test]
    async fn test_provider_requires_a_routable_model() {
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

        let state = Arc::new(AppState::new(
            workdir,
            Arc::new(NoOpRuntime),
            config,
            deploy_backend,
        ).expect("AppState::new"));

        let app = routes().with_state(Arc::clone(&state));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/providers/zai/test")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("parse response");
        assert_eq!(payload["code"], "bad_request");
        assert_eq!(
            payload["message"],
            "provider zai has no configured non-embedding models to test"
        );
    }
}
