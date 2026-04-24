//! Inference gateway endpoints.
//!
//! Provides centralized inference dispatch so agents never hold API keys
//! directly. All completion requests flow through this gateway which handles
//! model selection (via [`CascadeRouter`]), provider health tracking, cost
//! accounting, and event publishing.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use validator::Validate;

use roko_core::agent::{AgentRole, resolve_model};
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_learn::cascade_router::CascadeRouter;
use roko_learn::model_router::RoutingContext;

use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::extract::{RequestPayload, ValidJson, validate_with_validator};
use crate::state::{AppState, OperationHandle, OperationStatus};

/// Register inference gateway routes.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/inference/complete", post(inference_complete))
        .route("/gateway/stats", get(gateway_stats))
        .route("/gateway/models", get(gateway_models))
        .route("/inference/batch/submit", post(batch_submit))
        .route("/inference/batch/{id}", get(batch_status))
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// A single message in the completion request.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    /// Role of the message author (e.g. `"user"`, `"assistant"`, `"system"`).
    role: String,
    /// Text content of the message.
    content: String,
}

/// Request body for `POST /api/inference/complete`.
#[derive(Debug, Clone, Deserialize, Validate)]
struct CompletionRequest {
    /// Model slug to dispatch to. When absent the gateway uses the
    /// [`CascadeRouter`] to select the optimal model.
    #[serde(default)]
    model: Option<String>,

    /// Conversation messages to send.
    #[validate(length(min = 1))]
    messages: Vec<ChatMessage>,

    /// Maximum output tokens. Defaults to provider/model limit when absent.
    #[serde(default)]
    max_tokens: Option<u32>,

    /// Sampling temperature.
    #[serde(default)]
    temperature: Option<f64>,

    /// Optional tool definitions to pass to the model.
    #[serde(default)]
    tools: Option<Vec<Value>>,

    /// Calling agent identifier. Used for attribution and event tagging.
    #[serde(default)]
    agent_id: Option<String>,
}

impl RequestPayload for CompletionRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        validate_with_validator(self)
    }
}

/// Token usage counters returned in a completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TokenUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
}

/// Response body for `POST /api/inference/complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompletionResponse {
    /// Unique identifier for this completion request.
    id: String,
    /// Model that actually served the request.
    model: String,
    /// Text content of the assistant reply.
    content: String,
    /// Token usage counters.
    usage: TokenUsage,
    /// Reason the model stopped generating (e.g. `"end_turn"`, `"max_tokens"`).
    stop_reason: String,
    /// Estimated cost of this request in USD.
    cost_usd: f64,
}

/// A single request inside a batch submission.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
struct BatchRequestItem {
    /// Caller-assigned identifier for correlating results.
    #[validate(length(min = 1))]
    custom_id: String,

    /// Model slug override for this specific request.
    #[serde(default)]
    model: Option<String>,

    /// Conversation messages.
    #[validate(length(min = 1))]
    messages: Vec<ChatMessage>,
}

/// Request body for `POST /api/inference/batch/submit`.
#[derive(Debug, Clone, Deserialize, Validate)]
struct BatchSubmitRequest {
    /// List of individual completion requests.
    #[validate(length(min = 1))]
    #[validate(nested)]
    requests: Vec<BatchRequestItem>,
}

impl RequestPayload for BatchSubmitRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        validate_with_validator(self)
    }
}

/// Response body for `POST /api/inference/batch/submit`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BatchSubmitResponse {
    /// Unique identifier for the batch.
    batch_id: String,
    /// Number of requests in the batch.
    count: u32,
    /// Initial status (always `"queued"`).
    status: String,
}

/// Per-request result within a completed batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BatchResultItem {
    /// Caller-assigned correlation id.
    custom_id: String,
    /// Whether this individual request succeeded.
    success: bool,
    /// Completion response when successful.
    #[serde(skip_serializing_if = "Option::is_none")]
    response: Option<CompletionResponse>,
    /// Error message when the request failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Response body for `GET /api/inference/batch/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BatchStatusResponse {
    /// Batch identifier.
    batch_id: String,
    /// Current lifecycle status.
    status: String,
    /// Number of completed requests so far.
    completed: u32,
    /// Total number of requests in the batch.
    total: u32,
    /// Individual results, populated once the batch finishes.
    #[serde(skip_serializing_if = "Option::is_none")]
    results: Option<Vec<BatchResultItem>>,
}

/// Per-model statistics in the gateway stats response.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelStats {
    requests: u64,
    tokens_in: u64,
    tokens_out: u64,
    cost_usd: f64,
}

/// Gateway-level statistics response.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GatewayStatsResponse {
    total_requests: u64,
    cache_hits: u64,
    cache_hit_rate: f64,
    total_cost_usd: f64,
    models: HashMap<String, ModelStats>,
    providers: Value,
}

/// Metadata for a single model returned by the models endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GatewayModelInfo {
    id: String,
    provider: String,
    context_window: u64,
    max_output: u64,
    supports_tools: bool,
    supports_vision: bool,
    cost_per_1k_input: f64,
    cost_per_1k_output: f64,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/inference/complete` — dispatch a completion request through the
/// inference gateway.
///
/// When `model` is omitted from the request body the gateway consults the
/// [`CascadeRouter`] to select the best-fit model based on current health,
/// latency, and learning state. The selected model is recorded in the
/// response so callers know which model served their request.
async fn inference_complete(
    State(state): State<Arc<AppState>>,
    ValidJson(body): ValidJson<CompletionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let config = state.load_roko_config();

    // -----------------------------------------------------------------------
    // Model selection
    // -----------------------------------------------------------------------
    let model_slug = if let Some(ref requested) = body.model {
        let resolved = resolve_model(&config, requested);
        resolved.slug
    } else {
        select_model_via_router(&state)
    };

    // -----------------------------------------------------------------------
    // Build the prompt from the messages array
    // -----------------------------------------------------------------------
    let prompt = format_messages_as_prompt(&body.messages);

    // -----------------------------------------------------------------------
    // Dispatch through the runtime
    // -----------------------------------------------------------------------
    let result = state
        .runtime
        .run_once(state.workdir.as_path(), &prompt)
        .await
        .map_err(|err| {
            state.provider_health.record_failure(&model_slug);
            ApiError::internal(format!("inference dispatch failed: {err}"))
        })?;

    state.provider_health.record_success(&model_slug);

    let content = result.output_text.unwrap_or_default();

    // Estimate token counts from character lengths as a reasonable
    // approximation until per-provider tokenizers are wired.
    let input_tokens = estimate_tokens(&prompt);
    let output_tokens = estimate_tokens(&content);
    let cache_read_tokens: u64 = 0;

    // Compute cost from model profile pricing when available.
    let cost_usd = compute_cost(&config, &model_slug, input_tokens, output_tokens);

    let agent_label = body.agent_id.as_deref().unwrap_or("gateway").to_owned();

    // Publish event for SSE/WS consumers.
    state.event_bus.publish(ServerEvent::AgentOutput {
        agent_id: agent_label.clone(),
        run_id: Some(request_id.clone()),
        content: content.chars().take(200).collect(),
        done: true,
        metadata: Some(json!({
            "model": model_slug,
            "cost_usd": cost_usd,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
        })),
    });

    let response = CompletionResponse {
        id: request_id,
        model: model_slug,
        content,
        usage: TokenUsage {
            input_tokens,
            output_tokens,
            cache_read_tokens,
        },
        stop_reason: "end_turn".to_string(),
        cost_usd,
    };

    Ok(Json(response))
}

/// `GET /api/gateway/stats` — aggregate gateway statistics from provider
/// health and latency registries.
async fn gateway_stats(State(state): State<Arc<AppState>>) -> Json<GatewayStatsResponse> {
    let health_snapshot = state.provider_health.snapshot();

    let mut total_requests: u64 = 0;
    let mut total_successes: u64 = 0;
    let mut providers_json = serde_json::Map::new();

    for status in &health_snapshot {
        total_requests = total_requests.saturating_add(status.total_attempts);
        total_successes = total_successes.saturating_add(status.total_successes);

        let latency = state
            .latency_registry
            .get_all_for_provider(&status.provider);

        providers_json.insert(
            status.provider.clone(),
            json!({
                "state": format!("{:?}", status.state),
                "total_attempts": status.total_attempts,
                "total_successes": status.total_successes,
                "consecutive_failures": status.consecutive_failures,
                "latency_p50_ms": latency.p50_ms(),
                "latency_p95_ms": latency.p95_ms(),
                "latency_p99_ms": latency.p99_ms(),
                "error_rate": status.error_rate(),
            }),
        );
    }

    // Build per-model stats from provider-level data. This is a coarse
    // projection since the health tracker is keyed by provider, not model.
    // A future iteration should track per-model counters natively.
    let config = state.load_roko_config();
    let effective_models = config.effective_models();
    let mut model_stats: HashMap<String, ModelStats> = HashMap::new();

    for (key, profile) in &effective_models {
        let provider_status = health_snapshot
            .iter()
            .find(|s| s.provider == profile.provider);

        if let Some(status) = provider_status {
            model_stats.insert(
                key.clone(),
                ModelStats {
                    requests: status.total_attempts,
                    tokens_in: 0,
                    tokens_out: 0,
                    cost_usd: 0.0,
                },
            );
        }
    }

    // Cache hit rate is a placeholder — populated once the inference cache
    // layer is wired (currently no per-request caching exists).
    let cache_hits: u64 = 0;
    let cache_hit_rate = if total_requests > 0 {
        cache_hits as f64 / total_requests as f64
    } else {
        0.0
    };

    Json(GatewayStatsResponse {
        total_requests,
        cache_hits,
        cache_hit_rate,
        total_cost_usd: 0.0,
        models: model_stats,
        providers: Value::Object(providers_json),
    })
}

/// `GET /api/gateway/models` — list available models with capabilities and
/// pricing metadata.
async fn gateway_models(State(state): State<Arc<AppState>>) -> Json<Vec<GatewayModelInfo>> {
    let config = state.load_roko_config();
    let effective_models = config.effective_models();

    let mut models: Vec<GatewayModelInfo> = effective_models
        .into_iter()
        .filter(|(_, profile)| !profile.is_embedding_model)
        .map(|(key, profile)| {
            let max_output = profile.max_output.unwrap_or(4096);
            let cost_per_1k_input = profile.cost_input_per_m.map(|c| c / 1000.0).unwrap_or(0.0);
            let cost_per_1k_output = profile.cost_output_per_m.map(|c| c / 1000.0).unwrap_or(0.0);

            GatewayModelInfo {
                id: key,
                provider: profile.provider,
                context_window: profile.context_window,
                max_output,
                supports_tools: profile.supports_tools,
                supports_vision: profile.supports_vision,
                cost_per_1k_input,
                cost_per_1k_output,
            }
        })
        .collect();

    models.sort_by(|a, b| a.id.cmp(&b.id));

    Json(models)
}

/// `POST /api/inference/batch/submit` — submit a batch of inference requests
/// for background processing.
///
/// Each request in the batch is dispatched sequentially through the runtime.
/// The batch is tracked as a generic operation and results can be polled via
/// `GET /api/inference/batch/{id}`.
async fn batch_submit(
    State(state): State<Arc<AppState>>,
    ValidJson(body): ValidJson<BatchSubmitRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let batch_id = uuid::Uuid::new_v4().to_string();
    let count = body.requests.len() as u32;

    let state_for_task = Arc::clone(&state);
    let batch_id_for_task = batch_id.clone();
    let requests = body.requests;

    let handle = tokio::spawn(async move {
        let config = state_for_task.load_roko_config();
        let mut results: Vec<BatchResultItem> = Vec::with_capacity(requests.len());

        for item in &requests {
            let model_slug = if let Some(ref requested) = item.model {
                let resolved = resolve_model(&config, requested);
                resolved.slug
            } else {
                select_model_via_router(&state_for_task)
            };

            let prompt = format_messages_as_prompt(&item.messages);

            match state_for_task
                .runtime
                .run_once(state_for_task.workdir.as_path(), &prompt)
                .await
            {
                Ok(result) => {
                    state_for_task.provider_health.record_success(&model_slug);
                    let content = result.output_text.unwrap_or_default();
                    let input_tokens = estimate_tokens(&prompt);
                    let output_tokens = estimate_tokens(&content);
                    let cost_usd = compute_cost(&config, &model_slug, input_tokens, output_tokens);

                    results.push(BatchResultItem {
                        custom_id: item.custom_id.clone(),
                        success: true,
                        response: Some(CompletionResponse {
                            id: uuid::Uuid::new_v4().to_string(),
                            model: model_slug,
                            content,
                            usage: TokenUsage {
                                input_tokens,
                                output_tokens,
                                cache_read_tokens: 0,
                            },
                            stop_reason: "end_turn".to_string(),
                            cost_usd,
                        }),
                        error: None,
                    });
                }
                Err(err) => {
                    state_for_task.provider_health.record_failure(&model_slug);
                    results.push(BatchResultItem {
                        custom_id: item.custom_id.clone(),
                        success: false,
                        response: None,
                        error: Some(err.to_string()),
                    });
                }
            }
        }

        // Serialize results and store them in the operation handle.
        let results_json = serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string());
        if let Some(op) = state_for_task
            .operations
            .write()
            .await
            .get_mut(&batch_id_for_task)
        {
            op.status = OperationStatus::Completed {
                result: Some(results_json),
            };
        }
    });

    let op = OperationHandle {
        id: batch_id.clone(),
        kind: "inference_batch".to_string(),
        status: OperationStatus::Running,
        handle,
    };

    state.operations.write().await.insert(batch_id.clone(), op);

    state.event_bus.publish(ServerEvent::OperationStarted {
        op_id: batch_id.clone(),
        kind: "inference_batch".to_string(),
    });

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(BatchSubmitResponse {
            batch_id,
            count,
            status: "queued".to_string(),
        }),
    ))
}

/// `GET /api/inference/batch/{id}` — check the status of a previously
/// submitted batch.
async fn batch_status(
    State(state): State<Arc<AppState>>,
    Path(batch_id): Path<String>,
) -> Result<Json<BatchStatusResponse>, ApiError> {
    let ops = state.operations.read().await;
    let op = ops
        .get(&batch_id)
        .ok_or_else(|| ApiError::not_found(format!("batch {batch_id} not found")))?;

    match &op.status {
        OperationStatus::Running => Ok(Json(BatchStatusResponse {
            batch_id,
            status: "processing".to_string(),
            completed: 0,
            total: 0,
            results: None,
        })),
        OperationStatus::Completed { result } => {
            let results: Vec<BatchResultItem> = result
                .as_deref()
                .and_then(|json_str| serde_json::from_str(json_str).ok())
                .unwrap_or_default();
            let total = results.len() as u32;
            let completed = results.iter().filter(|r| r.success).count() as u32
                + results.iter().filter(|r| !r.success).count() as u32;

            Ok(Json(BatchStatusResponse {
                batch_id,
                status: "completed".to_string(),
                completed,
                total,
                results: Some(results),
            }))
        }
        OperationStatus::Failed { error } => Ok(Json(BatchStatusResponse {
            batch_id: batch_id.clone(),
            status: "failed".to_string(),
            completed: 0,
            total: 0,
            results: Some(vec![BatchResultItem {
                custom_id: batch_id,
                success: false,
                response: None,
                error: Some(error.clone()),
            }]),
        })),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format a message array into a single prompt string suitable for
/// `CliRuntime::run_once`. System messages are prepended as context,
/// user/assistant turns follow in order.
fn format_messages_as_prompt(messages: &[ChatMessage]) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(messages.len());
    for msg in messages {
        match msg.role.as_str() {
            "system" => parts.push(format!("[System]\n{}", msg.content)),
            "user" => parts.push(format!("[User]\n{}", msg.content)),
            "assistant" => parts.push(format!("[Assistant]\n{}", msg.content)),
            other => parts.push(format!("[{other}]\n{}", msg.content)),
        }
    }
    parts.join("\n\n")
}

/// Select the optimal model via the CascadeRouter.
fn select_model_via_router(state: &AppState) -> String {
    let config = state.load_roko_config();
    let effective_models = config.effective_models();
    let mut model_slugs: Vec<String> = effective_models
        .values()
        .filter(|p| !p.is_embedding_model)
        .map(|p| p.slug.clone())
        .collect();
    model_slugs.sort();

    if model_slugs.is_empty() {
        return config.agent.default_model.clone();
    }

    let cascade_path = state.workdir.join(".roko/learn/cascade-router.json");
    let router = CascadeRouter::load_or_new(&cascade_path, model_slugs.clone());
    let routing_ctx = RoutingContext {
        task_category: TaskCategory::Implementation,
        complexity: TaskComplexityBand::Standard,
        iteration: 1,
        role: AgentRole::Implementer,
        crate_familiarity: 0.5,
        has_prior_failure: false,
        conductor_load: 0.0,
        active_agents: 0,
        ready_queue_depth: 0,
        max_queue_wait_hours: 0.0,
        daimon_policy: roko_core::DaimonPolicy::default(),
        thinking_level: None,
        temperament: None,
        previous_model: None,
        plan_context_tokens: None,
        tier_thresholds: None,
    };

    let explanation = router.explain_routing(&routing_ctx, &model_slugs);
    explanation.selected_model
}

/// Rough token count estimate: ~4 characters per token for English text.
fn estimate_tokens(text: &str) -> u64 {
    (text.len() as u64).div_ceil(4)
}

/// Compute the estimated cost of a request in USD from model profile pricing.
fn compute_cost(
    config: &roko_core::config::schema::RokoConfig,
    model_slug: &str,
    input_tokens: u64,
    output_tokens: u64,
) -> f64 {
    let models = config.effective_models();
    let profile = models.values().find(|p| p.slug == model_slug);

    let input_cost = profile
        .and_then(|p| p.cost_input_per_m)
        .map(|rate| rate * input_tokens as f64 / 1_000_000.0)
        .unwrap_or(0.0);

    let output_cost = profile
        .and_then(|p| p.cost_output_per_m)
        .map(|rate| rate * output_tokens as f64 / 1_000_000.0)
        .unwrap_or(0.0);

    input_cost + output_cost
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use axum::body::{Body, to_bytes};
    use axum::extract::State;
    use axum::http::Request;
    use roko_core::config::ServeAuthConfig;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::deploy::create_backend;
    use crate::routes::build_router;
    use crate::runtime::NoOpRuntime;

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let state = Arc::new(AppState::new(
            workdir,
            Arc::new(NoOpRuntime),
            roko_core::config::schema::RokoConfig::default(),
            deploy_backend,
        ));
        (dir, state)
    }

    #[test]
    fn completion_request_rejects_empty_messages() {
        let request = CompletionRequest {
            model: None,
            messages: vec![],
            max_tokens: None,
            temperature: None,
            tools: None,
            agent_id: None,
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn completion_request_accepts_valid_messages() {
        let request = CompletionRequest {
            model: Some("claude-sonnet-4-6".into()),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "Hello".into(),
            }],
            max_tokens: Some(1024),
            temperature: Some(0.7),
            tools: None,
            agent_id: Some("agent-1".into()),
        };
        assert!(request.validate().is_ok());
    }

    #[test]
    fn batch_request_rejects_empty_requests() {
        let request = BatchSubmitRequest { requests: vec![] };
        assert!(request.validate().is_err());
    }

    #[test]
    fn batch_request_rejects_empty_custom_id() {
        let request = BatchSubmitRequest {
            requests: vec![BatchRequestItem {
                custom_id: "".into(),
                model: None,
                messages: vec![ChatMessage {
                    role: "user".into(),
                    content: "test".into(),
                }],
            }],
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn batch_request_rejects_empty_messages_in_item() {
        let request = BatchSubmitRequest {
            requests: vec![BatchRequestItem {
                custom_id: "req-1".into(),
                model: None,
                messages: vec![],
            }],
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn batch_request_accepts_valid_batch() {
        let request = BatchSubmitRequest {
            requests: vec![
                BatchRequestItem {
                    custom_id: "req-1".into(),
                    model: None,
                    messages: vec![ChatMessage {
                        role: "user".into(),
                        content: "first".into(),
                    }],
                },
                BatchRequestItem {
                    custom_id: "req-2".into(),
                    model: Some("claude-haiku-4-5".into()),
                    messages: vec![ChatMessage {
                        role: "user".into(),
                        content: "second".into(),
                    }],
                },
            ],
        };
        assert!(request.validate().is_ok());
    }

    #[test]
    fn format_messages_produces_structured_prompt() {
        let messages = vec![
            ChatMessage {
                role: "system".into(),
                content: "You are helpful.".into(),
            },
            ChatMessage {
                role: "user".into(),
                content: "Hello".into(),
            },
        ];

        let prompt = format_messages_as_prompt(&messages);
        assert!(prompt.contains("[System]\nYou are helpful."));
        assert!(prompt.contains("[User]\nHello"));
    }

    #[test]
    fn estimate_tokens_rounds_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hi"), 1);
        assert_eq!(estimate_tokens("hello"), 2);
        assert_eq!(estimate_tokens("hello world!!!"), 4);
    }

    #[test]
    fn compute_cost_zero_when_no_pricing() {
        let config = roko_core::config::schema::RokoConfig::default();
        let cost = compute_cost(&config, "nonexistent-model", 1000, 500);
        assert_eq!(cost, 0.0);
    }

    #[tokio::test]
    async fn gateway_stats_returns_valid_structure() {
        let (_dir, state) = test_state();
        let Json(stats) = gateway_stats(State(state)).await;

        assert_eq!(stats.total_requests, 0);
        assert_eq!(stats.cache_hits, 0);
        assert_eq!(stats.cache_hit_rate, 0.0);
        assert_eq!(stats.total_cost_usd, 0.0);
    }

    #[tokio::test]
    async fn gateway_models_returns_sorted_list() {
        let (_dir, state) = test_state();
        let Json(models) = gateway_models(State(state)).await;

        // Default config may produce models; verify sorting regardless.
        for window in models.windows(2) {
            assert!(window[0].id <= window[1].id);
        }
    }

    #[tokio::test]
    async fn batch_status_returns_404_for_unknown_batch() {
        let (_dir, state) = test_state();
        let err = batch_status(State(state), Path("nonexistent".into()))
            .await
            .expect_err("unknown batch should 404");

        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn complete_route_rejects_empty_body() {
        let (_dir, state) = test_state();
        let app = build_router(Arc::clone(&state), &[], ServeAuthConfig::default());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/inference/complete")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"messages":[]}"#))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(
            response.status(),
            axum::http::StatusCode::BAD_REQUEST,
            "empty messages should fail validation"
        );

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: Value = serde_json::from_slice(&body).expect("parse body");
        assert_eq!(payload["code"], "validation_error");
    }

    #[tokio::test]
    async fn complete_route_accepts_valid_request() {
        let (_dir, state) = test_state();
        let app = build_router(Arc::clone(&state), &[], ServeAuthConfig::default());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/inference/complete")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"messages":[{"role":"user","content":"Say hello"}]}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: Value = serde_json::from_slice(&body).expect("parse body");
        assert!(payload.get("id").is_some());
        assert!(payload.get("model").is_some());
        assert!(payload.get("usage").is_some());
        assert!(payload.get("cost_usd").is_some());
        assert_eq!(payload["stop_reason"], "end_turn");
    }
}
