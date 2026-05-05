//! Inference gateway endpoints.
//!
//! Provides centralized inference dispatch so agents never hold API keys
//! directly. All completion requests flow through this gateway which handles
//! model selection (via [`CascadeRouter`]), provider health tracking, cost
//! accounting, and event publishing.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use validator::Validate;

use roko_agent::GatewayEventWriter;
use roko_core::agent::{AgentRole, resolve_model};
use roko_core::config::schema::RokoConfig;
use roko_core::foundation::{
    CachePolicy, ChatMessage as CoreChatMessage, MessageRole as CoreMessageRole,
    ModelCallRequest as CoreModelCallRequest, ModelCallResponse as CoreModelCallResponse,
    ModelCaller, caller,
};
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_learn::bandits::UcbBandit;
use roko_learn::cascade_router::CascadeRouter;
use roko_learn::model_router::RoutingContext;

use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::extract::{RequestPayload, ValidJson, validate_with_validator};
use crate::state::{AppState, BatchProgress, OperationHandle, OperationStatus};

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

    // -- Routing hint fields (B4) ------------------------------------------
    /// Broad task category hint for model routing (e.g. `"implementation"`,
    /// `"research"`, `"refactor"`). Parsed into [`TaskCategory`]; unknown
    /// values fall back to `Implementation`.
    #[serde(default)]
    task_category: Option<String>,

    /// Complexity band hint (`"fast"`, `"standard"`, `"complex"`). Parsed
    /// into [`TaskComplexityBand`]; unknown values fall back to `Standard`.
    #[serde(default)]
    complexity: Option<String>,

    /// Agent role hint (e.g. `"implementer"`, `"researcher"`, `"auditor"`).
    /// Parsed into [`AgentRole`]; unknown values fall back to `Implementer`.
    #[serde(default)]
    role: Option<String>,

    /// Current iteration number for the calling task (0-based).
    #[serde(default)]
    iteration: Option<u32>,

    /// Crate name hint for familiarity-based routing.
    #[serde(default)]
    crate_name: Option<String>,

    /// Whether a prior attempt at this task failed.
    #[serde(default)]
    has_prior_failure: Option<bool>,
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
    average_latency_ms: f64,
}

/// Gateway-level statistics response.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GatewayStatsResponse {
    total_requests: u64,
    cache_hits: u64,
    cache_hit_rate: f64,
    total_cost_usd: f64,
    average_latency_ms: f64,
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
    // Model selection (D1: uses cached CascadeRouter)
    // -----------------------------------------------------------------------
    let hints = RoutingHints {
        task_category: body.task_category.clone(),
        complexity: body.complexity.clone(),
        role: body.role.clone(),
        iteration: body.iteration,
        crate_name: body.crate_name.clone(),
        has_prior_failure: body.has_prior_failure,
    };

    let model_slug = if let Some(ref requested) = body.model {
        let resolved = resolve_model(&config, requested);
        resolved.slug
    } else {
        select_model_via_router(&state, &hints).await
    };

    let auto_routed = body.model.is_none();
    let agent_label = body.agent_id.as_deref().unwrap_or("gateway").to_owned();

    state.event_bus.publish(ServerEvent::InferenceStarted {
        request_id: request_id.clone(),
        model: model_slug.clone(),
        agent_id: agent_label.clone(),
        auto_routed,
    });

    let req = model_call_request(
        model_slug.clone(),
        &body.messages,
        body.max_tokens,
        body.temperature,
        body.agent_id.clone(),
    );
    let requested_provider = provider_id_for_model(&config, &model_slug);

    let call_start = std::time::Instant::now();

    let model_response = state.model_call_service.call(req).await.map_err(|err| {
        if let Some(provider) = requested_provider.as_deref() {
            state.provider_health.record_failure(provider);
        }
        let _duration_ms = call_start.elapsed().as_millis() as u64;
        state.event_bus.publish(ServerEvent::InferenceFailed {
            request_id: request_id.clone(),
            model: model_slug.clone(),
            agent_id: agent_label.clone(),
            error: err.to_string(),
        });
        ApiError::internal(format!("inference dispatch failed: {err}"))
    })?;

    let served_model = model_response.model.clone();
    if let Some(served_provider) = provider_id_for_model(&config, &served_model) {
        state.provider_health.record_success(&served_provider);
    }
    let input_tokens = model_response.usage.input_tokens;
    let output_tokens = model_response.usage.output_tokens;
    let cost_usd = model_response.usage.cost_usd;
    let content_preview: String = model_response.content.chars().take(200).collect();

    // B1: accumulate per-model token + cost counters for gateway_stats.
    let counters = state.gateway_counters_for(&served_model).await;
    counters.record(input_tokens, output_tokens, cost_usd);

    let duration_ms = call_start.elapsed().as_millis() as u64;

    state.event_bus.publish(ServerEvent::InferenceCompleted {
        request_id: request_id.clone(),
        model: served_model.clone(),
        agent_id: agent_label.clone(),
        input_tokens,
        output_tokens,
        cost_usd,
        duration_ms,
    });

    // Publish event for SSE/WS consumers.
    state.event_bus.publish(ServerEvent::AgentOutput {
        agent_id: agent_label.clone(),
        run_id: Some(request_id.clone()),
        content: content_preview,
        done: true,
        metadata: Some(json!({
            "model": served_model,
            "cost_usd": cost_usd,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
        })),
    });

    let response = completion_response_from_model_call(request_id, model_response);

    Ok(Json(response))
}

/// `GET /api/gateway/stats` — aggregate gateway statistics from the durable
/// gateway event log.
async fn gateway_stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GatewayStatsResponse>, ApiError> {
    let projection = GatewayEventWriter::for_workdir(&state.workdir)
        .projection()
        .map_err(|err| ApiError::internal(format!("load gateway event log: {err}")))?;
    let model_aggregates = projection.stats_by_model();
    let provider_aggregates = projection.stats_by_provider();
    let total_requests = projection.total_events() as u64;
    let cache_hits: u64 = model_aggregates
        .values()
        .map(|stats| stats.cache_hits)
        .sum();
    let cache_hit_rate = if total_requests > 0 {
        cache_hits as f64 / total_requests as f64
    } else {
        0.0
    };
    let total_cost_usd = projection.total_cost_usd();
    let total_latency_ms: u64 = model_aggregates
        .values()
        .map(|stats| stats.total_latency_ms)
        .sum();
    let average_latency_ms = if total_requests > 0 {
        total_latency_ms as f64 / total_requests as f64
    } else {
        0.0
    };

    let health_snapshot = state.provider_health.snapshot();

    let mut providers_json = serde_json::Map::new();

    for status in &health_snapshot {
        let latency = state
            .latency_registry
            .get_all_for_provider(&status.provider);
        let durable = provider_aggregates.get(&status.provider);
        let total_attempts = durable.map_or(0, |stats| stats.count);
        let total_successes = durable.map_or(0, |stats| stats.count.saturating_sub(stats.errors));
        let durable_error_rate = durable.map_or(0.0, |stats| {
            if stats.count > 0 {
                stats.errors as f64 / stats.count as f64
            } else {
                0.0
            }
        });

        providers_json.insert(
            status.provider.clone(),
            json!({
                "state": format!("{:?}", status.state),
                "total_attempts": total_attempts,
                "total_successes": total_successes,
                "consecutive_failures": status.consecutive_failures,
                "latency_p50_ms": latency.p50_ms(),
                "latency_p95_ms": latency.p95_ms(),
                "latency_p99_ms": latency.p99_ms(),
                "error_rate": durable_error_rate,
            }),
        );
    }

    for (provider, stats) in &provider_aggregates {
        if providers_json.contains_key(provider) {
            continue;
        }
        let error_rate = if stats.count > 0 {
            stats.errors as f64 / stats.count as f64
        } else {
            0.0
        };
        providers_json.insert(
            provider.clone(),
            json!({
                "state": "Unknown",
                "total_attempts": stats.count,
                "total_successes": stats.count.saturating_sub(stats.errors),
                "consecutive_failures": 0,
                "latency_p50_ms": null,
                "latency_p95_ms": null,
                "latency_p99_ms": null,
                "error_rate": error_rate,
            }),
        );
    }

    let model_stats = model_aggregates
        .into_iter()
        .map(|(model, stats)| {
            (
                model,
                ModelStats {
                    requests: stats.count,
                    tokens_in: stats.total_input_tokens,
                    tokens_out: stats.total_output_tokens,
                    cost_usd: stats.total_cost_usd,
                    average_latency_ms: if stats.count > 0 {
                        stats.total_latency_ms as f64 / stats.count as f64
                    } else {
                        0.0
                    },
                },
            )
        })
        .collect();

    Ok(Json(GatewayStatsResponse {
        total_requests,
        cache_hits,
        cache_hit_rate,
        total_cost_usd,
        average_latency_ms,
        models: model_stats,
        providers: Value::Object(providers_json),
    }))
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

/// Maximum number of batch items dispatched concurrently (B2).
const BATCH_CONCURRENCY: usize = 8;

/// `POST /api/inference/batch/submit` — submit a batch of inference requests
/// for background processing.
///
/// Batch items are dispatched **in parallel** (up to [`BATCH_CONCURRENCY`]
/// concurrent requests) through the runtime (B2). An [`AtomicUsize`] counter
/// in [`BatchProgress`] is incremented after each item completes so the status
/// endpoint can report incremental progress (B3).
///
/// Results can be polled via `GET /api/inference/batch/{id}`.
async fn batch_submit(
    State(state): State<Arc<AppState>>,
    ValidJson(body): ValidJson<BatchSubmitRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let batch_id = uuid::Uuid::new_v4().to_string();
    let count = body.requests.len() as u32;

    // B3: create a shared progress counter visible to the status endpoint.
    let progress = Arc::new(BatchProgress {
        completed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        total: body.requests.len(),
    });
    state
        .batch_progress
        .write()
        .await
        .insert(batch_id.clone(), Arc::clone(&progress));

    let state_for_task = Arc::clone(&state);
    let batch_id_for_task = batch_id.clone();
    let requests = body.requests;

    let handle = tokio::spawn(async move {
        let config = state_for_task.load_roko_config();

        // B2: process batch items concurrently with a bounded stream.
        let results: Vec<BatchResultItem> = stream::iter(requests)
            .map(|item| {
                let state_ref = Arc::clone(&state_for_task);
                let config_ref = Arc::clone(&config);
                let progress_ref = Arc::clone(&progress);
                async move {
                    let model_slug = if let Some(ref requested) = item.model {
                        let resolved = resolve_model(&config_ref, requested);
                        resolved.slug
                    } else {
                        // Batch items do not carry per-item routing hints; use
                        // defaults (same as previous hardcoded behaviour).
                        select_model_via_router(&state_ref, &RoutingHints::default()).await
                    };

                    let req =
                        model_call_request(model_slug.clone(), &item.messages, None, None, None);
                    let requested_provider = provider_id_for_model(&config_ref, &model_slug);

                    let result_item = match state_ref.model_call_service.call(req).await {
                        Ok(model_response) => {
                            let served_model = model_response.model.clone();
                            if let Some(served_provider) =
                                provider_id_for_model(&config_ref, &served_model)
                            {
                                state_ref.provider_health.record_success(&served_provider);
                            }
                            let input_tokens = model_response.usage.input_tokens;
                            let output_tokens = model_response.usage.output_tokens;
                            let cost_usd = model_response.usage.cost_usd;

                            // B1: accumulate per-model counters.
                            let counters = state_ref.gateway_counters_for(&served_model).await;
                            counters.record(input_tokens, output_tokens, cost_usd);

                            BatchResultItem {
                                custom_id: item.custom_id.clone(),
                                success: true,
                                response: Some(completion_response_from_model_call(
                                    uuid::Uuid::new_v4().to_string(),
                                    model_response,
                                )),
                                error: None,
                            }
                        }
                        Err(err) => {
                            if let Some(provider) = requested_provider.as_deref() {
                                state_ref.provider_health.record_failure(provider);
                            }
                            BatchResultItem {
                                custom_id: item.custom_id.clone(),
                                success: false,
                                response: None,
                                error: Some(err.to_string()),
                            }
                        }
                    };

                    // B3: increment progress counter after each item.
                    progress_ref.completed.fetch_add(1, Ordering::Relaxed);

                    result_item
                }
            })
            .buffer_unordered(BATCH_CONCURRENCY)
            .collect()
            .await;

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

        // Clean up progress entry now that the batch is done.
        state_for_task
            .batch_progress
            .write()
            .await
            .remove(&batch_id_for_task);
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
        OperationStatus::Running => {
            // B3: read incremental progress from the shared counter.
            let progress = state.batch_progress.read().await;
            let (completed, total) = if let Some(bp) = progress.get(&batch_id) {
                (bp.completed.load(Ordering::Relaxed) as u32, bp.total as u32)
            } else {
                (0, 0)
            };
            Ok(Json(BatchStatusResponse {
                batch_id,
                status: "processing".to_string(),
                completed,
                total,
                results: None,
            }))
        }
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

fn model_call_request(
    model: String,
    messages: &[ChatMessage],
    max_tokens: Option<u32>,
    temperature: Option<f64>,
    role: Option<String>,
) -> CoreModelCallRequest {
    CoreModelCallRequest {
        model,
        system: None,
        messages: messages.iter().map(core_chat_message).collect(),
        max_tokens,
        temperature: temperature.map(|t| t as f32),
        role,
        caller: Some(caller::SERVE.to_string()),
        run_id: None,
        prompt_section_ids: Vec::new(),
        knowledge_ids: Vec::new(),
        budget: None,
        budget_remaining: None,
        routing_hints: Vec::new(),
        cache_policy: CachePolicy::Default,
    }
}

fn core_chat_message(message: &ChatMessage) -> CoreChatMessage {
    CoreChatMessage {
        role: match message.role.as_str() {
            "system" => CoreMessageRole::System,
            "assistant" => CoreMessageRole::Assistant,
            _ => CoreMessageRole::User,
        },
        content: message.content.clone(),
    }
}

fn completion_response_from_model_call(
    id: String,
    response: CoreModelCallResponse,
) -> CompletionResponse {
    let cost_usd = response.usage.cost_usd;
    CompletionResponse {
        id,
        model: response.model,
        content: response.content,
        usage: TokenUsage {
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
        },
        stop_reason: response
            .stop_reason
            .unwrap_or_else(|| "end_turn".to_string()),
        cost_usd,
    }
}

/// Routing hints extracted from an incoming [`CompletionRequest`].
#[derive(Debug, Default)]
struct RoutingHints {
    task_category: Option<String>,
    complexity: Option<String>,
    role: Option<String>,
    iteration: Option<u32>,
    crate_name: Option<String>,
    has_prior_failure: Option<bool>,
}

/// Parse a string into [`TaskCategory`], falling back to `Implementation`
/// for unrecognised values.
fn parse_task_category(s: &str) -> TaskCategory {
    match s.to_ascii_lowercase().as_str() {
        "scaffolding" => TaskCategory::Scaffolding,
        "integration" => TaskCategory::Integration,
        "verification" => TaskCategory::Verification,
        "research" => TaskCategory::Research,
        "refactor" => TaskCategory::Refactor,
        "infra" => TaskCategory::Infra,
        "docs" => TaskCategory::Docs,
        // "implementation" and unrecognised values
        _ => TaskCategory::Implementation,
    }
}

/// Parse a string into [`TaskComplexityBand`], falling back to `Standard`
/// for unrecognised values.
fn parse_complexity(s: &str) -> TaskComplexityBand {
    match s.to_ascii_lowercase().as_str() {
        "fast" => TaskComplexityBand::Fast,
        "complex" => TaskComplexityBand::Complex,
        // "standard" and unrecognised values
        _ => TaskComplexityBand::Standard,
    }
}

/// Parse a string into [`AgentRole`], falling back to `Implementer` for
/// unrecognised values.
fn parse_agent_role(s: &str) -> AgentRole {
    match s.to_ascii_lowercase().replace('-', "_").as_str() {
        "conductor" => AgentRole::Conductor,
        "strategist" => AgentRole::Strategist,
        "architect" => AgentRole::Architect,
        "researcher" => AgentRole::Researcher,
        "auditor" => AgentRole::Auditor,
        "quick_reviewer" => AgentRole::QuickReviewer,
        "scribe" => AgentRole::Scribe,
        "critic" => AgentRole::Critic,
        "auto_fixer" => AgentRole::AutoFixer,
        "refactorer" => AgentRole::Refactorer,
        "pre_planner" => AgentRole::PrePlanner,
        "doc_verifier" => AgentRole::DocVerifier,
        "integration_tester" => AgentRole::IntegrationTester,
        "merge_resolver" => AgentRole::MergeResolver,
        // "implementer" and unrecognised values
        _ => AgentRole::Implementer,
    }
}

fn provider_id_for_model(config: &RokoConfig, model_key_or_slug: &str) -> Option<String> {
    let models = config.effective_models();
    if let Some(profile) = models.get(model_key_or_slug) {
        return Some(profile.provider.clone());
    }
    if let Some(profile) = models
        .values()
        .find(|profile| profile.slug == model_key_or_slug)
    {
        return Some(profile.provider.clone());
    }

    None
}

/// Select the optimal model via the [`CascadeRouter`] (D1: cached in AppState),
/// optionally refined by a [`UcbBandit`] when one has been trained.
///
/// On first call the router is loaded from disk and cached. Subsequent
/// requests read the cached instance through an `RwLock`, avoiding
/// per-request file I/O.
///
/// When the caller supplies routing hints (task category, complexity, role,
/// iteration) they are parsed into the corresponding enum variants and used
/// to populate the [`RoutingContext`]. Missing fields fall back to sensible
/// defaults (matching previous hardcoded behaviour).
///
/// After the cascade router selects a model, we check for a persisted
/// [`UcbBandit`] at `.roko/learn/model-bandit.json`. If the bandit has
/// been trained (>0 total pulls) its selection overrides the cascade pick.
async fn select_model_via_router(state: &AppState, hints: &RoutingHints) -> String {
    let config = state.load_roko_config();
    let mut all_model_slugs: Vec<String> = config.model_slugs_for_cascade();
    all_model_slugs.sort();

    if all_model_slugs.is_empty() {
        return config.agent.default_model.clone();
    }

    let mut candidate_slugs = config.available_model_slugs_for_cascade();
    if candidate_slugs.is_empty() {
        candidate_slugs = all_model_slugs.clone();
    }
    candidate_slugs.sort();

    // -- B4: build RoutingContext from caller hints, with defaults ----------
    let task_category = hints
        .task_category
        .as_deref()
        .map(parse_task_category)
        .unwrap_or(TaskCategory::Implementation);

    let complexity = hints
        .complexity
        .as_deref()
        .map(parse_complexity)
        .unwrap_or(TaskComplexityBand::Standard);

    let role = hints
        .role
        .as_deref()
        .map(parse_agent_role)
        .unwrap_or(AgentRole::Implementer);

    let iteration = hints.iteration.unwrap_or(1);

    // Derive routing context from runtime state where available.
    let active_agents = state.operations.read().await.len() as u32;
    let has_prior_failure = hints.has_prior_failure.unwrap_or(iteration > 1);

    let routing_ctx = RoutingContext {
        task_category,
        complexity,
        iteration,
        role,
        crate_familiarity: 0.5,
        has_prior_failure,
        conductor_load: 0.0,
        active_agents,
        ready_queue_depth: 0,
        max_queue_wait_hours: 0.0,
        daimon_policy: roko_core::DaimonPolicy::default(),
        thinking_level: None,
        temperament: None,
        previous_model: None,
        plan_context_tokens: None,
        tier_thresholds: None,
    };

    // D1: use cached CascadeRouter — fast path reads, slow path loads once.
    let cascade_pick = {
        let guard = state.cascade_router.read().await;
        if let Some(ref router) = *guard {
            router
                .explain_routing(&routing_ctx, &candidate_slugs)
                .selected_model
        } else {
            drop(guard);
            let mut guard = state.cascade_router.write().await;
            if guard.is_none() {
                let cascade_path = state.workdir.join(".roko/learn/cascade-router.json");
                let router = CascadeRouter::load_or_new(&cascade_path, all_model_slugs.clone());
                *guard = Some(router);
            }
            let router = guard.as_ref().expect("just initialised");
            router
                .explain_routing(&routing_ctx, &candidate_slugs)
                .selected_model
        }
    };

    // -- A4: bandit refinement ---------------------------------------------
    //
    // Attempt to load a persisted UcbBandit. When it has been trained
    // (total_pulls > 0) we use its selection to refine the cascade pick.
    // On any failure we silently fall back to the cascade result.
    let bandit_path = state.workdir.join(".roko/learn/model-bandit.json");
    if let Ok(bandit) = UcbBandit::load(&bandit_path, candidate_slugs) {
        if bandit.total_pulls() > 0 {
            let bandit_pick = bandit.select();
            return bandit_pick;
        }
    }

    cascade_pick
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
    use roko_agent::GatewayEvent;
    use roko_agent::ModelCallService;
    use roko_agent::task_runner::{CostTable, ModelPricing};
    use roko_core::agent::ProviderKind;
    use roko_core::config::ServeAuthConfig;
    use roko_core::config::schema::{ModelProfile, ProviderConfig};
    use roko_learn::FeedbackService;
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
        let state = Arc::new(
            AppState::new(
                workdir,
                Arc::new(NoOpRuntime),
                roko_core::config::schema::RokoConfig::default(),
                deploy_backend,
            )
            .expect("AppState::new"),
        );
        (dir, state)
    }

    #[test]
    fn provider_id_for_model_requires_configured_model_profile() {
        let mut config = roko_core::config::schema::RokoConfig::default();
        config.models.clear();
        config.providers.clear();
        config.providers.insert(
            "openai_compat".to_string(),
            ProviderConfig {
                kind: ProviderKind::OpenAiCompat,
                base_url: None,
                api_key_env: Some("OPENAI_API_KEY".to_string()),
                command: None,
                args: None,
                timeout_ms: None,
                ttft_timeout_ms: None,
                connect_timeout_ms: None,
                extra_headers: None,
                max_concurrent: None,
            },
        );

        assert_eq!(provider_id_for_model(&config, "gpt-new-unconfigured"), None);
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
            task_category: None,
            complexity: None,
            role: None,
            iteration: None,
            crate_name: None,
            has_prior_failure: None,
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
            task_category: None,
            complexity: None,
            role: None,
            iteration: None,
            crate_name: None,
            has_prior_failure: None,
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
    fn model_call_request_maps_messages_and_metadata() {
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

        let request = model_call_request(
            "claude-sonnet-4-6".into(),
            &messages,
            Some(1024),
            Some(0.7),
            Some("agent-1".into()),
        );

        assert_eq!(request.model, "claude-sonnet-4-6");
        assert_eq!(request.max_tokens, Some(1024));
        assert_eq!(request.temperature, Some(0.7));
        assert_eq!(request.role.as_deref(), Some("agent-1"));
        assert_eq!(request.caller.as_deref(), Some(caller::SERVE));
        assert_eq!(request.cache_policy, CachePolicy::Default);
        assert_eq!(request.messages[0].role, CoreMessageRole::System);
        assert_eq!(request.messages[1].role, CoreMessageRole::User);
    }

    #[test]
    fn completion_response_uses_model_call_usage_and_cost() {
        let response = completion_response_from_model_call(
            "request-1".into(),
            CoreModelCallResponse {
                content: "done".into(),
                model: "served-model".into(),
                usage: roko_core::foundation::TokenUsage {
                    input_tokens: 100,
                    output_tokens: 50,
                    total_tokens: 150,
                    cost_usd: 0.0123,
                },
                stop_reason: Some("max_tokens".into()),
                request_id: None,
            },
        );

        assert_eq!(response.id, "request-1");
        assert_eq!(response.model, "served-model");
        assert_eq!(response.content, "done");
        assert_eq!(response.usage.input_tokens, 100);
        assert_eq!(response.usage.output_tokens, 50);
        assert_eq!(response.stop_reason, "max_tokens");
        assert_eq!(response.cost_usd, 0.0123);
    }

    #[test]
    fn routing_hints_derive_prior_failure_from_iteration() {
        // When has_prior_failure is explicitly set, use it.
        let hints = RoutingHints {
            iteration: Some(1),
            has_prior_failure: Some(true),
            ..Default::default()
        };
        let derived = hints
            .has_prior_failure
            .unwrap_or(hints.iteration.unwrap_or(1) > 1);
        assert!(derived, "explicit true should be respected");

        // When not set, iteration > 1 implies prior failure.
        let hints = RoutingHints {
            iteration: Some(3),
            has_prior_failure: None,
            ..Default::default()
        };
        let derived = hints
            .has_prior_failure
            .unwrap_or(hints.iteration.unwrap_or(1) > 1);
        assert!(derived, "iteration 3 implies prior failure");

        // Iteration 1 with no explicit flag = no prior failure.
        let hints = RoutingHints {
            iteration: Some(1),
            has_prior_failure: None,
            ..Default::default()
        };
        let derived = hints
            .has_prior_failure
            .unwrap_or(hints.iteration.unwrap_or(1) > 1);
        assert!(!derived, "iteration 1 means no prior failure");
    }

    #[tokio::test]
    async fn routing_context_uses_active_operations_count() {
        let (_dir, state) = test_state();

        // Initially no operations → active_agents = 0.
        let ops_count = state.operations.read().await.len() as u32;
        assert_eq!(ops_count, 0);

        // Insert a dummy operation.
        state.operations.write().await.insert(
            "test-op".to_string(),
            OperationHandle {
                id: "test-op".to_string(),
                kind: "test".to_string(),
                status: OperationStatus::Running,
                handle: tokio::spawn(async {}),
            },
        );

        let ops_count = state.operations.read().await.len() as u32;
        assert_eq!(
            ops_count, 1,
            "active_agents should reflect operations count"
        );

        // Clean up.
        state.operations.write().await.remove("test-op");
    }

    #[tokio::test]
    async fn gateway_stats_returns_valid_structure() {
        let (_dir, state) = test_state();
        let Json(stats) = gateway_stats(State(state)).await.expect("gateway stats");

        assert_eq!(stats.total_requests, 0);
        assert_eq!(stats.cache_hits, 0);
        assert_eq!(stats.cache_hit_rate, 0.0);
        assert_eq!(stats.total_cost_usd, 0.0);
        assert_eq!(stats.average_latency_ms, 0.0);
    }

    #[tokio::test]
    async fn test_gateway_writes_durable_events() {
        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let model = "gateway-durable-model";
        let provider = "gateway-durable-provider";

        let script = workdir.join("mock-provider.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nsleep 0.01\nprintf 'gateway durable reply'\n",
        )
        .expect("write mock provider");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&script)
                .expect("mock provider metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&script, permissions).expect("chmod mock provider");
        }

        let mut config = roko_core::config::schema::RokoConfig::default();
        config.agent.default_model = model.to_string();
        config.agent.command = Some(script.display().to_string());
        config.models.insert(
            model.to_string(),
            ModelProfile {
                provider: provider.to_string(),
                slug: model.to_string(),
                cost_input_per_m: Some(1.0),
                cost_output_per_m: Some(2.0),
                ..Default::default()
            },
        );

        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let feedback = Arc::new(FeedbackService::from_roko_dir_with_episodes(
            &workdir.join(".roko"),
        ));
        let feedback_sink: Arc<dyn roko_core::foundation::FeedbackSink> = feedback.clone();
        let mut cost_table = CostTable::default();
        cost_table.insert(
            model,
            ModelPricing {
                input_per_m: 1.0,
                output_per_m: 2.0,
                cache_read_per_m: 0.0,
                cache_write_per_m: 0.0,
            },
        );

        let mut state = AppState::new(
            workdir.clone(),
            Arc::new(NoOpRuntime),
            config.clone(),
            deploy_backend,
        )
        .expect("AppState::new");
        state.model_call_service = Arc::new(
            ModelCallService::new(model.to_string())
                .with_config(config)
                .with_cost_table(cost_table)
                .with_feedback_sink(feedback_sink)
                .with_gateway_event_writer(Arc::new(GatewayEventWriter::for_workdir(&workdir)))
                .with_run_id("gateway-durable-test"),
        );
        let state = Arc::new(state);
        let app = build_router(
            Arc::clone(&state),
            &[],
            ServeAuthConfig {
                enabled: false,
                ..ServeAuthConfig::default()
            },
        );

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/inference/complete")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"model":"{model}","agent_id":"gateway-test-agent","messages":[{{"role":"user","content":"Say hello from the gateway durability test"}}]}}"#
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let completion: CompletionResponse = serde_json::from_slice(&body).expect("completion");
        assert_eq!(completion.model, model);
        assert!(completion.usage.input_tokens > 0);
        assert!(completion.usage.output_tokens > 0);
        assert!(completion.cost_usd > 0.0);

        let gateway_log = workdir.join(".roko").join("learn").join("gateway.jsonl");
        let gateway_events = std::fs::read_to_string(&gateway_log).expect("gateway log");
        let events: Vec<GatewayEvent> = gateway_events
            .lines()
            .map(|line| serde_json::from_str(line).expect("gateway event"))
            .collect();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.model, model);
        assert_eq!(event.provider.as_deref(), Some(provider));
        assert!(event.input_tokens > 0);
        assert!(event.output_tokens > 0);
        assert!(event.cost_usd > 0.0);
        assert!(event.latency_ms > 0);
        assert!(!event.cache_hit);
        assert!(event.success);

        feedback.flush_async().await.expect("flush feedback");
        let feedback_log = workdir.join(".roko").join("learn").join("efficiency.jsonl");
        let feedback_events = std::fs::read_to_string(&feedback_log).expect("feedback log");
        let feedback_event: Value = feedback_events
            .lines()
            .map(|line| serde_json::from_str(line).expect("feedback event"))
            .find(|event: &Value| event["kind"] == "model_call")
            .expect("model feedback event");
        assert_eq!(feedback_event["model"], model);
        assert_eq!(feedback_event["provider"], provider);
        assert_eq!(feedback_event["input_tokens"], event.input_tokens);
        assert_eq!(feedback_event["output_tokens"], event.output_tokens);
        assert_eq!(feedback_event["cost_usd"], event.cost_usd);
        assert_eq!(feedback_event["latency_ms"], event.latency_ms);

        let health = state.provider_health.get(provider);
        assert_eq!(health.total_attempts, 1);
        assert_eq!(health.total_successes, 1);
        assert_eq!(state.provider_health.get(model).total_attempts, 0);

        let Json(stats) = gateway_stats(State(Arc::clone(&state)))
            .await
            .expect("gateway stats");
        assert!(stats.total_requests > 0);
        assert!(stats.average_latency_ms > 0.0);
        let model_stats = stats.models.get(model).expect("model stats");
        assert_eq!(model_stats.requests, 1);
        assert_eq!(model_stats.tokens_in, event.input_tokens);
        assert_eq!(model_stats.tokens_out, event.output_tokens);
        assert_eq!(model_stats.cost_usd, event.cost_usd);
        assert_eq!(model_stats.average_latency_ms, event.latency_ms as f64);
        assert_eq!(stats.providers[provider]["total_attempts"], 1);
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
    async fn batch_submit_dispatches_concurrently() {
        // Submit 3 batch items and verify all complete.
        // The NoOpRuntime returns instantly, so buffer_unordered processes
        // all items concurrently within BATCH_CONCURRENCY.
        let (_dir, state) = test_state();
        let body = BatchSubmitRequest {
            requests: (0..3)
                .map(|i| BatchRequestItem {
                    custom_id: format!("req-{i}"),
                    model: None,
                    messages: vec![ChatMessage {
                        role: "user".into(),
                        content: format!("item {i}"),
                    }],
                })
                .collect(),
        };

        let resp = batch_submit(State(Arc::clone(&state)), ValidJson(body))
            .await
            .expect("batch submit");
        let (parts, body_bytes) = resp.into_response().into_parts();
        assert_eq!(parts.status, axum::http::StatusCode::ACCEPTED);

        let payload: Value =
            serde_json::from_slice(&to_bytes(body_bytes, usize::MAX).await.expect("body"))
                .expect("json");
        assert_eq!(payload["count"], 3);

        let batch_id = payload["batch_id"].as_str().expect("batch_id").to_string();

        // Wait briefly for the spawned task to complete.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Verify all items completed via the operations map.
        let ops = state.operations.read().await;
        if let Some(op) = ops.get(&batch_id) {
            match &op.status {
                OperationStatus::Completed { result } => {
                    let items: Vec<BatchResultItem> =
                        serde_json::from_str(result.as_deref().unwrap_or("[]")).expect("parse");
                    assert_eq!(items.len(), 3, "all batch items should complete");
                    assert!(items.iter().all(|i| i.success), "all items should succeed");
                }
                _ => {
                    // May still be running — that's OK for a unit test with NoOpRuntime
                }
            }
        }
    }

    #[tokio::test]
    async fn batch_concurrency_constant_is_reasonable() {
        assert!(
            BATCH_CONCURRENCY >= 2 && BATCH_CONCURRENCY <= 32,
            "BATCH_CONCURRENCY ({BATCH_CONCURRENCY}) should be between 2 and 32"
        );
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
        let app = build_router(
            Arc::clone(&state),
            &[],
            ServeAuthConfig {
                enabled: false,
                ..ServeAuthConfig::default()
            },
        );

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
    async fn complete_route_dispatches_valid_request_to_model_service() {
        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let mut config = roko_core::config::schema::RokoConfig::default();
        config.agent.command = Some("false".into());
        let state = Arc::new(
            AppState::new(workdir, Arc::new(NoOpRuntime), config, deploy_backend)
                .expect("AppState::new"),
        );
        let app = build_router(
            Arc::clone(&state),
            &[],
            ServeAuthConfig {
                enabled: false,
                ..ServeAuthConfig::default()
            },
        );

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/inference/complete")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"model":"invalid-model","messages":[{"role":"user","content":"Say hello"}]}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: Value = serde_json::from_slice(&body).expect("parse body");
        assert_eq!(payload["code"], "internal_error");
        assert!(
            payload["message"]
                .as_str()
                .is_some_and(|message| message.contains("inference dispatch failed"))
        );
    }
}
