//! Cognitive event to session/update streaming.
//!
//! Bridges Roko's provider system (via `roko-agent`) to ACP
//! `session/update` notifications.
//! All cognitive workflow dispatch now goes through
//! [`crate::runner::run_with_workflow_engine`], which uses `ModelCallService`
//! for provider-agnostic model calls.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};

use roko_agent::StreamChunk;
use roko_agent::safety::{SafetyLayer, ViolationSeverity};
use roko_agent::streaming::parse_sse_line;
use roko_core::ContentHash;
use roko_core::DaimonPolicy;
use roko_core::agent::{AgentRole, ProviderKind, resolve_model};
use roko_core::config::schema::{ModelProfile, RokoConfig};
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_dreams::{load_dream_routing_advice, relevant_pattern_summaries};
use roko_learn::{
    cascade_router::CascadeRouter,
    cost_table::CostTable,
    episode_logger::{Episode, EpisodeLogger, Usage as EpUsage},
    model_router::RoutingContext,
    playbook::Playbook,
};
use roko_neuro::{KnowledgeKind, KnowledgeQueryHit, KnowledgeTier};
use serde::Deserialize;
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite},
    sync::mpsc,
    task,
};
use tracing::{debug, error, info, warn};

use crate::knowledge::{DispatchKnowledge, append_context, query_dispatch_knowledge};
use crate::runner::run_with_workflow_engine;
use crate::{
    session::{AcpSession, CancelToken},
    transport::{StdioTransport, TransportError, TransportResult},
    types::{
        ContentBlock, CostInfo, JsonRpcMessage, PermissionAction, PermissionDecision,
        PermissionResponse, PlanEntry, RequestPermissionParams, SESSION_BUSY, SessionCancelParams,
        SessionPromptParams, SessionPromptResult, SessionUpdate, StopReason, ToolCallKind,
        ToolCallStatus, UsageInfo,
    },
};

// ── Claude CLI stream-json wire types (kept for claude_cli fallback) ──

/// Top-level stream event from `claude --output-format stream-json`.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeStreamEvent {
    System(ClaudeSystemEvent),
    Assistant(ClaudeAssistantEvent),
    Tool(ClaudeToolEvent),
    Result(ClaudeResultEvent),
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct ClaudeSystemEvent {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub model: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct ClaudeAssistantEvent {
    pub message: ClaudeMessage,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct ClaudeMessage {
    #[serde(default)]
    pub content: Vec<ClaudeContentBlock>,
    #[serde(default)]
    pub usage: Option<ClaudeUsage>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String },
    Thinking { thinking: String },
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct ClaudeToolEvent {
    #[serde(default, rename = "tool_name")]
    pub _tool_name: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub content: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct ClaudeResultEvent {
    #[serde(default)]
    pub total_cost_usd: Option<f64>,
    #[serde(default, rename = "is_error")]
    pub _is_error: bool,
    #[serde(default)]
    pub usage: Option<ClaudeUsage>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct ClaudeUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

// ── Error types ──────────────────────────────────────────────────────

/// Errors produced while bridging cognitive events to ACP session updates.
#[derive(Debug, Error)]
pub enum BridgeEventsError {
    /// The target session already has an active prompt in flight.
    #[error("session '{0}' already has an active prompt")]
    SessionBusy(String),
    /// JSON serialization for an outbound session update failed.
    #[error("failed to serialize ACP session update: {0}")]
    Serialize(#[from] serde_json::Error),
    /// Writing to the ACP stdio transport failed.
    #[error("failed to send ACP session update: {0}")]
    Transport(#[from] TransportError),
    /// The spawned cognitive task terminated unexpectedly.
    #[error("ACP cognitive task failed: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
    /// A pipeline runner error.
    #[error("ACP pipeline error: {0}")]
    Pipeline(#[from] anyhow::Error),
}

impl BridgeEventsError {
    /// Returns a JSON-RPC error tuple when the failure maps to a client-visible ACP error.
    #[must_use]
    pub fn rpc_error(&self) -> Option<(i32, String)> {
        match self {
            Self::SessionBusy(session_id) => Some((
                SESSION_BUSY,
                format!("session '{session_id}' already has an active prompt"),
            )),
            Self::Serialize(_) | Self::Transport(_) | Self::TaskJoin(_) | Self::Pipeline(_) => None,
        }
    }
}

/// Result alias for ACP event bridge operations.
pub type Result<T> = std::result::Result<T, BridgeEventsError>;

/// Maximum assistant response bytes stored in one history turn.
const MAX_HISTORY_ASSISTANT_BYTES: usize = 10_240;

static CASCADE_ROUTER_IO_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

// ── Cognitive events ─────────────────────────────────────────────────

/// Events emitted by the cognitive loop and mapped to ACP session updates.
#[derive(Debug, Clone)]
pub enum CognitiveEvent {
    /// A streamed agent-visible text chunk.
    TokenChunk(String),
    /// A streamed internal reasoning chunk.
    ThinkingChunk(String),
    /// A tool call has started running.
    ToolCallStart {
        tool_call_id: String,
        title: String,
        kind: ToolCallKind,
    },
    /// A tool call has finished with rendered content.
    ToolCallComplete {
        tool_call_id: String,
        status: ToolCallStatus,
        content: Vec<ContentBlock>,
    },
    /// A plan update with structured entries (shown as progress in editor).
    PlanUpdate { entries: Vec<PlanEntry> },
    /// Prompt execution completed normally.
    Complete {
        stop_reason: StopReason,
        usage: Option<UsageInfo>,
    },
    /// Prompt execution stopped because the token budget was exhausted.
    MaxTokens,
}

// ── Stream events → editor ───────────────────────────────────────────

/// Result of streaming events: the prompt result, accumulated assistant text,
/// and any provider-reported usage.
pub struct StreamResult {
    pub prompt_result: SessionPromptResult,
    /// Accumulated assistant text from TokenChunk events.
    pub assistant_text: String,
    /// Usage reported by the provider, if any.
    pub usage: Option<UsageInfo>,
}

fn pricing_table() -> &'static CostTable {
    static TABLE: OnceLock<CostTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        CostTable {
            models: std::collections::HashMap::new(),
        }
        .with_defaults()
    })
}

/// Calculate model cost from token counts.
///
/// Returns `None` when the model slug has no pricing row. Unknown pricing
/// stays unknown instead of collapsing to zero.
pub fn calculate_cost_for_model_slug(
    model_slug: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
) -> Option<f64> {
    let pricing = pricing_table().models.get(model_slug)?;
    Some(
        (input_tokens as f64 * pricing.input_per_m / 1_000_000.0)
            + (output_tokens as f64 * pricing.output_per_m / 1_000_000.0)
            + (cache_read_tokens as f64 * pricing.cache_read_per_m / 1_000_000.0),
    )
}

fn calculate_cost_without_cache_for_model_slug(
    model_slug: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
) -> Option<f64> {
    let pricing = pricing_table().models.get(model_slug)?;
    Some(
        (input_tokens as f64 * pricing.input_per_m / 1_000_000.0)
            + (output_tokens as f64 * pricing.output_per_m / 1_000_000.0)
            + (cache_read_tokens as f64 * pricing.input_per_m / 1_000_000.0),
    )
}

#[allow(clippy::too_many_arguments)]
async fn append_acp_episode(
    roko_config: &RokoConfig,
    workdir: &Path,
    session: &AcpSession,
    model_key: &str,
    prompt_text: &str,
    workflow_config: &str,
    is_pipeline_dispatch: bool,
    dispatch_started: Instant,
    stream_result: Option<&StreamResult>,
    task_error: Option<&str>,
    stream_error: Option<&str>,
    // When provided, overrides the pricing-table cost calculation with the
    // actual cost reported by the provider (e.g. from `WorkflowRunReport.cost`).
    cost_override: Option<f64>,
) {
    let resolved = resolve_model(roko_config, model_key);
    let elapsed = dispatch_started.elapsed();
    let input_hash = ContentHash::of(prompt_text.as_bytes()).to_hex();
    let output_source = stream_result
        .map(|sr| sr.assistant_text.as_str())
        .filter(|text| !text.is_empty())
        .or(task_error)
        .or(stream_error)
        .unwrap_or("");
    let output_hash = ContentHash::of(output_source.as_bytes()).to_hex();
    let mode = session.config_state.agent_mode.clone();
    let mut episode = Episode::new(mode.clone(), session.session_id.clone());

    episode.kind = if is_pipeline_dispatch {
        format!("acp-pipeline-{workflow_config}")
    } else {
        "acp-dispatch".to_string()
    };
    episode.agent_template = mode.clone();
    episode.model = resolved.slug.clone();
    episode.backend = resolved.provider_kind.label().to_string();
    episode.trigger_kind = if is_pipeline_dispatch {
        "acp_pipeline".to_string()
    } else {
        "acp_dispatch".to_string()
    };
    episode.trigger_signal_hash = input_hash.clone();
    episode.input_signal_hash = input_hash;
    episode.output_signal_hash = output_hash;
    episode.episode_id = episode.id.clone();
    episode.duration_secs = elapsed.as_secs_f64();
    let stream_usage = stream_result.and_then(|sr| sr.usage.as_ref());
    let mut usage = EpUsage {
        wall_ms: elapsed.as_millis() as u64,
        ..EpUsage::default()
    };
    if let Some(provider_usage) = stream_usage {
        let input_tokens = provider_usage.input_tokens;
        let output_tokens = provider_usage.output_tokens;
        let cached_read_tokens = provider_usage.cached_read_tokens.unwrap_or(0);
        usage.input_tokens = input_tokens;
        usage.output_tokens = output_tokens;
        usage.cache_read_tokens = cached_read_tokens;
        usage.cache_write_tokens = provider_usage.cached_write_tokens.unwrap_or(0);
        usage.cost_usd = cost_override
            .unwrap_or_else(|| {
                calculate_cost_for_model_slug(
                    &resolved.slug,
                    input_tokens,
                    output_tokens,
                    cached_read_tokens,
                )
                .unwrap_or(0.0)
            });
        usage.cost_usd_without_cache = cost_override
            .unwrap_or_else(|| {
                calculate_cost_without_cache_for_model_slug(
                    &resolved.slug,
                    input_tokens,
                    output_tokens,
                    cached_read_tokens,
                )
                .unwrap_or(usage.cost_usd)
            });
    }
    episode.usage = usage;
    episode.tokens_used = stream_usage.map(|usage| usage.total_tokens).unwrap_or(0);
    episode
        .extra
        .insert("entry_point".to_string(), serde_json::json!("acp"));
    episode
        .extra
        .insert("model".to_string(), serde_json::json!(resolved.slug));
    episode
        .extra
        .insert("mode".to_string(), serde_json::json!(mode));
    episode.extra.insert(
        "session_id".to_string(),
        serde_json::json!(session.session_id.clone()),
    );
    episode.extra.insert(
        "routing_mode".to_string(),
        serde_json::json!(session.config_state.routing_mode.clone()),
    );
    episode
        .extra
        .insert("workflow".to_string(), serde_json::json!(workflow_config));
    episode.extra.insert(
        "provider_kind".to_string(),
        serde_json::json!(resolved.provider_kind.label()),
    );

    let success = acp_dispatch_succeeded(stream_result, task_error, stream_error);
    episode.success = success;

    if !success {
        let failure_reason = task_error
            .or(stream_error)
            .map(str::to_string)
            .or_else(|| {
                stream_result.map(|sr| match sr.prompt_result.stop_reason {
                    StopReason::Cancelled => "cancelled".to_string(),
                    StopReason::MaxTokens => "max_tokens".to_string(),
                    StopReason::MaxTurnRequests => "max_turn_requests".to_string(),
                    StopReason::Refusal => "refusal".to_string(),
                    StopReason::EndTurn => "unknown failure".to_string(),
                })
            })
            .unwrap_or_else(|| "unknown failure".to_string());
        episode.failure_reason = Some(failure_reason);
    }

    let episodes_path = workdir.join(".roko").join("episodes.jsonl");
    if let Some(parent) = episodes_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let logger = EpisodeLogger::new(&episodes_path);
    if let Err(err) = logger.append(&episode).await {
        error!(
            session_id = %session.session_id,
            error = %err,
            "failed to append ACP episode"
        );
    }
}

fn acp_routing_context(mode: &str, prompt: &str) -> RoutingContext {
    let _prompt_len = prompt.len();
    let task_category = if mode == "research" {
        TaskCategory::Research
    } else {
        TaskCategory::Implementation
    };

    let role = match mode {
        "plan" => AgentRole::Strategist,
        "research" => AgentRole::Researcher,
        _ => AgentRole::Implementer,
    };

    RoutingContext {
        task_category,
        complexity: TaskComplexityBand::Standard,
        iteration: 0,
        role,
        crate_familiarity: 0.5,
        has_prior_failure: false,
        conductor_load: 0.0,
        active_agents: 0,
        ready_queue_depth: 0,
        max_queue_wait_hours: 0.0,
        daimon_policy: DaimonPolicy::default(),
        thinking_level: None,
        temperament: None,
        previous_model: None,
        plan_context_tokens: None,
        tier_thresholds: None,
    }
}

fn acp_dispatch_succeeded(
    stream_result: Option<&StreamResult>,
    task_error: Option<&str>,
    stream_error: Option<&str>,
) -> bool {
    task_error.is_none()
        && stream_error.is_none()
        && stream_result
            .map(|sr| matches!(sr.prompt_result.stop_reason, StopReason::EndTurn))
            .unwrap_or(false)
}

fn cascade_router_model_slugs(roko_config: &RokoConfig, resolved_slug: &str) -> Vec<String> {
    let mut model_slugs = roko_config.models.keys().cloned().collect::<Vec<_>>();
    if model_slugs.is_empty() {
        model_slugs.push(resolved_slug.to_owned());
    }
    model_slugs
}

fn compute_acp_reward(success: bool, wall_ms: u64, output_tokens: Option<u64>) -> f64 {
    if !success {
        return 0.0;
    }

    let latency_bonus = if wall_ms < 5_000 {
        0.15
    } else if wall_ms < 15_000 {
        0.05
    } else {
        0.0
    };
    let token_bonus = match output_tokens {
        Some(tokens) if tokens < 2_000 => 0.05,
        Some(tokens) if tokens < 5_000 => 0.02,
        _ => 0.0,
    };

    let score: f64 = 0.8 + latency_bonus + token_bonus;
    score.min(1.0)
}

fn record_cascade_observation(
    router_path: PathBuf,
    model_slug: String,
    routing_ctx: RoutingContext,
    success: bool,
    wall_ms: u64,
    output_tokens: Option<u64>,
    model_slugs: Vec<String>,
) {
    drop(task::spawn_blocking(move || {
        let _guard = CASCADE_ROUTER_IO_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        let router = CascadeRouter::load_or_new(&router_path, model_slugs);

        let Some(model_idx) = router.model_index_for_slug(&model_slug) else {
            debug!(
                model = %model_slug,
                "skipping cascade observation: model not in router arms"
            );
            return;
        };

        let context_vec = routing_ctx.to_features();
        let reward = compute_acp_reward(success, wall_ms, output_tokens);
        router.observe(context_vec, model_idx, reward);

        if let Err(error) = router.save(&router_path) {
            warn!(
                path = %router_path.display(),
                error = %error,
                "failed to persist cascade router after ACP observation"
            );
        }
    }));
}

fn truncate_assistant_history(text: &str) -> String {
    if text.len() <= MAX_HISTORY_ASSISTANT_BYTES {
        return text.to_owned();
    }

    let mut end = MAX_HISTORY_ASSISTANT_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }

    let mut truncated = String::with_capacity(end + "...[truncated]".len());
    truncated.push_str(&text[..end]);
    truncated.push_str("...[truncated]");
    truncated
}

/// Sends a `session/request_permission` request to the editor and waits for the decision.
///
/// Returns `PermissionDecision::Allow` if the action is already pre-granted.
/// Returns `PermissionDecision::Reject` on timeout or error, which is the safe default.
///
/// If the user chooses `AlwaysAllow`, the decision is remembered on the session and persisted
/// to `.roko/trust/permissions.json`.
pub async fn request_permission<R, W>(
    transport: &mut StdioTransport<R, W>,
    session: &mut AcpSession,
    workdir: &Path,
    action: PermissionAction,
    title: &str,
    detail: &str,
) -> PermissionDecision
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if session.is_pre_granted(&action) {
        debug!(
            session_id = %session.session_id,
            action = ?action,
            "permission pre-granted (always-allow)"
        );
        return PermissionDecision::Allow;
    }

    debug!(
        session_id = %session.session_id,
        action = ?action,
        title = %title,
        detail = %detail,
        "requesting permission from editor"
    );

    let params = serde_json::to_value(RequestPermissionParams {
        session_id: session.session_id.clone(),
        title: title.to_string(),
        detail: detail.to_string(),
        action: action.clone(),
    })
    .unwrap_or_else(|error| {
        warn!(
            session_id = %session.session_id,
            action = ?action,
            error = %error,
            "failed to serialize permission request; sending null payload"
        );
        serde_json::Value::Null
    });

    let mut request_transport = transport.clone();
    let request_future = request_transport.send_request("session/request_permission", params);
    tokio::pin!(request_future);
    let timeout = tokio::time::sleep(std::time::Duration::from_secs(300));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            biased;
            response = &mut request_future => {
                match response {
                    Ok(json_response) => {
                        if let Some(error) = json_response.error.as_ref() {
                            warn!(
                                session_id = %session.session_id,
                                action = ?action,
                                code = error.code,
                                message = %error.message,
                                "permission request returned an error; defaulting to Reject"
                            );
                            return PermissionDecision::Reject;
                        }

                        let decision = json_response
                            .result
                            .as_ref()
                            .and_then(|value| serde_json::from_value::<PermissionResponse>(value.clone()).ok())
                            .map(|response| response.decision)
                            .unwrap_or_else(|| {
                                warn!(
                                    session_id = %session.session_id,
                                    action = ?action,
                                    "permission response could not be parsed; defaulting to Reject"
                                );
                                PermissionDecision::Reject
                            });

                        if matches!(decision, PermissionDecision::AlwaysAllow) {
                            session.grant_always_allow(action.clone());
                            AcpSession::save_workspace_trust(workdir, &session.always_allowed);
                            info!(
                                session_id = %session.session_id,
                                action = ?action,
                                "permission permanently granted (always-allow persisted)"
                            );
                        }

                        return decision;
                    }
                    Err(error) => {
                        warn!(
                            session_id = %session.session_id,
                            action = ?action,
                            error = %error,
                            "permission request transport error; defaulting to Reject"
                        );
                        return PermissionDecision::Reject;
                    }
                }
            }
            inbound = transport.read_message() => {
                match inbound {
                    Ok(Some(JsonRpcMessage::Response(response))) => {
                        transport.handle_incoming_response(response);
                    }
                    Ok(Some(JsonRpcMessage::Notification(notification))) => {
                        if notification.method == "session/cancel" {
                            match serde_json::from_value::<SessionCancelParams>(
                                notification.params.unwrap_or(serde_json::Value::Null),
                            ) {
                                Ok(params) if params.session_id == session.session_id => {
                                    warn!(
                                        session_id = %session.session_id,
                                        "permission request cancelled by client; defaulting to Reject"
                                    );
                                    return PermissionDecision::Reject;
                                }
                                Ok(_) => {}
                                Err(error) => {
                                    warn!(
                                        session_id = %session.session_id,
                                        error = %error,
                                        "received malformed session/cancel while waiting for permission"
                                    );
                                }
                            }
                        } else {
                            debug!(
                                session_id = %session.session_id,
                                method = %notification.method,
                                "ignoring notification while waiting for permission"
                            );
                        }
                    }
                    Ok(Some(JsonRpcMessage::Request(request))) => {
                        warn!(
                            session_id = %session.session_id,
                            method = %request.method,
                            "ignoring inbound request while waiting for permission"
                        );
                    }
                    Ok(None) => {
                        warn!(
                            session_id = %session.session_id,
                            "ACP client disconnected while waiting for permission"
                        );
                        return PermissionDecision::Reject;
                    }
                    Err(error) => {
                        warn!(
                            session_id = %session.session_id,
                            error = %error,
                            "failed to read inbound message while waiting for permission; defaulting to Reject"
                        );
                        return PermissionDecision::Reject;
                    }
                }
            }
            _ = &mut timeout => {
                warn!(
                    session_id = %session.session_id,
                    action = ?action,
                    "permission request timed out after 5 minutes; defaulting to Reject"
                );
                return PermissionDecision::Reject;
            }
        }
    }
}

/// Maps cognitive events to ACP `session/update` notifications and streams them to the editor.
/// Returns both the prompt result and the accumulated assistant response text.
pub async fn stream_events_to_editor<R, W>(
    transport: &mut StdioTransport<R, W>,
    session_id: &str,
    mut events: mpsc::Receiver<CognitiveEvent>,
    cancel_token: &CancelToken,
) -> Result<StreamResult>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut assistant_text = String::new();

    loop {
        enum StreamAction {
            Cancelled,
            Event(Option<CognitiveEvent>),
            Inbound(TransportResult<Option<JsonRpcMessage>>),
        }

        let action = tokio::select! {
            biased;
            _ = cancel_token.cancelled() => StreamAction::Cancelled,
            maybe_event = events.recv() => StreamAction::Event(maybe_event),
            inbound = transport.read_message() => StreamAction::Inbound(inbound),
        };

        match action {
            StreamAction::Cancelled => {
                debug!(session_id, "ACP prompt cancelled while streaming events");
                return Ok(StreamResult {
                    prompt_result: SessionPromptResult {
                        stop_reason: StopReason::Cancelled,
                    },
                    assistant_text,
                    usage: None,
                });
            }
            StreamAction::Event(maybe_event) => {
                let Some(event) = maybe_event else {
                    warn!(
                        session_id,
                        "ACP event stream closed without an explicit completion event"
                    );
                    let stop_reason = if cancel_token.is_cancelled() {
                        StopReason::Cancelled
                    } else {
                        StopReason::EndTurn
                    };
                    return Ok(StreamResult {
                        prompt_result: SessionPromptResult { stop_reason },
                        assistant_text,
                        usage: None,
                    });
                };

                match event {
                    CognitiveEvent::Complete { stop_reason, usage } => {
                        return Ok(StreamResult {
                            prompt_result: SessionPromptResult { stop_reason },
                            assistant_text,
                            usage,
                        });
                    }
                    CognitiveEvent::MaxTokens => {
                        return Ok(StreamResult {
                            prompt_result: SessionPromptResult {
                                stop_reason: StopReason::MaxTokens,
                            },
                            assistant_text,
                            usage: None,
                        });
                    }
                    CognitiveEvent::TokenChunk(ref text) => {
                        assistant_text.push_str(text);
                        let update = map_event_to_update(event);
                        send_session_update(transport, session_id, update).await?;
                    }
                    other => {
                        let update = map_event_to_update(other);
                        send_session_update(transport, session_id, update).await?;
                    }
                }
            }
            StreamAction::Inbound(inbound) => match inbound? {
                Some(JsonRpcMessage::Notification(notification))
                    if notification.method == "session/cancel" =>
                {
                    match serde_json::from_value::<SessionCancelParams>(
                        notification.params.unwrap_or(serde_json::Value::Null),
                    ) {
                        Ok(params) if params.session_id == session_id => {
                            cancel_token.cancel();
                        }
                        Ok(_) => {}
                        Err(error) => {
                            warn!(
                                session_id,
                                error = %error,
                                "received malformed session/cancel while prompt was active"
                            );
                        }
                    }
                }
                Some(JsonRpcMessage::Notification(notification)) => {
                    warn!(
                        session_id,
                        method = %notification.method,
                        "ignoring unsupported notification while prompt was active"
                    );
                }
                Some(JsonRpcMessage::Response(response)) => {
                    transport.handle_incoming_response(response);
                }
                Some(JsonRpcMessage::Request(request)) => {
                    warn!(
                        session_id,
                        method = %request.method,
                        "ignoring inbound request while prompt was active"
                    );
                }
                None => {
                    warn!(
                        session_id,
                        "ACP client disconnected while prompt was active"
                    );
                    return Ok(StreamResult {
                        prompt_result: SessionPromptResult {
                            stop_reason: StopReason::Cancelled,
                        },
                        assistant_text,
                        usage: None,
                    });
                }
            },
        }
    }
}

// ── Session prompt entry point ───────────────────────────────────────

/// Handles a `session/prompt` request by running the cognitive task and streaming updates.
pub async fn handle_session_prompt<R, W>(
    transport: &mut StdioTransport<R, W>,
    session: &mut AcpSession,
    params: SessionPromptParams,
    workdir: &Path,
    roko_config: &RokoConfig,
) -> Result<SessionPromptResult>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if session.is_busy() {
        return Err(BridgeEventsError::SessionBusy(session.session_id.clone()));
    }

    session.begin_prompt();

    let outcome =
        handle_session_prompt_inner(transport, session, params, workdir, roko_config).await;
    session.finish_prompt();
    outcome
}

async fn handle_session_prompt_inner<R, W>(
    transport: &mut StdioTransport<R, W>,
    session: &mut AcpSession,
    params: SessionPromptParams,
    workdir: &Path,
    roko_config: &RokoConfig,
) -> Result<SessionPromptResult>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let prompt_text = extract_prompt_text(&params.prompt);
    let model_key = session.config_state.model.clone();
    let is_slash_command = prompt_text.trim_start().starts_with('/');
    let resolved = resolve_model(roko_config, &model_key);
    let resolved_for_logging = resolved.clone();

    debug!(
        session_id = %session.session_id,
        prompt_blocks = params.prompt.len(),
        prompt_chars = prompt_text.chars().count(),
        include_context = params.include_context,
        model_key = %model_key,
        workdir = %workdir.display(),
        "handling ACP session prompt"
    );

    // Capture workflow config before we decide whether context resolution is needed.
    let workflow_config = session.config_state.workflow.clone();

    // Check if a workflow pipeline should handle this prompt.
    let pipeline_template = if workflow_config == "auto" {
        Some(crate::pipeline::WorkflowTemplate::auto_select(&prompt_text))
    } else {
        crate::pipeline::WorkflowTemplate::from_config(&workflow_config)
    };

    if !is_slash_command {
        session.push_user_turn(prompt_text.clone());
    }

    let agent_mode = session.config_state.agent_mode.clone();
    if !is_slash_command && pipeline_template.is_none() && agent_mode == "code" {
        let permission_detail = format!(
            "The {} agent may read and modify files in {}.",
            agent_mode,
            workdir.display()
        );
        let decision = request_permission(
            transport,
            session,
            workdir,
            PermissionAction::FileEdit,
            "Allow code agent to edit files?",
            &permission_detail,
        )
        .await;

        if matches!(decision, PermissionDecision::Reject) {
            info!(
                session_id = %session.session_id,
                "user rejected permission for code agent dispatch"
            );
            let (reject_sender, reject_receiver) = mpsc::channel(4);
            let _ = reject_sender
                .send(CognitiveEvent::TokenChunk(
                    "Permission denied. The agent cannot proceed without file access.".to_string(),
                ))
                .await;
            let _ = reject_sender
                .send(CognitiveEvent::Complete {
                    stop_reason: StopReason::EndTurn,
                    usage: None,
                })
                .await;
            drop(reject_sender);
            return stream_events_to_editor(
                transport,
                &session.session_id,
                reject_receiver,
                &session.cancel_token,
            )
            .await
            .map(|sr| sr.prompt_result);
        }
    }

    let should_resolve_context = !is_slash_command && pipeline_template.is_none();

    let knowledge = if is_slash_command {
        DispatchKnowledge::default()
    } else {
        query_dispatch_knowledge(workdir, &prompt_text).await
    };
    let knowledge_context = knowledge.context_text();

    // Resolve context only for the single-agent path.
    // Resource blocks always resolve; @-mentions are only resolved when
    // prompt-time context is enabled.
    let file_context = if should_resolve_context {
        if params.include_context {
            resolve_context_items(&params.prompt, workdir).await
        } else {
            let uris = extract_resource_uris(&params.prompt);
            if uris.is_empty() {
                String::new()
            } else {
                read_file_context(&uris, workdir)
            }
        }
    } else {
        String::new()
    };

    // Get system prompt and history context for the single-agent path.
    let system_prompt = if should_resolve_context {
        session.build_system_prompt(workdir, &[], session.cached_conventions.as_deref())
    } else {
        String::new()
    };
    let history_context = if should_resolve_context {
        session.build_history_context_for_cli()
    } else {
        String::new()
    };
    let messages = if should_resolve_context {
        // Build combined system prompt with resolved context.
        let mut full_system = system_prompt.clone();
        full_system = append_context(&full_system, &file_context);
        full_system = append_context(&full_system, &knowledge_context);
        session.build_messages_array(&full_system, &prompt_text)
    } else {
        Vec::new()
    };

    let (event_sender, event_receiver) = mpsc::channel(64);
    if !is_slash_command {
        emit_knowledge_card(&knowledge, &event_sender).await;
    }
    let provenance = if is_slash_command {
        None
    } else {
        build_provenance(&knowledge.hits, &knowledge.playbooks, &prompt_text, workdir).await
    };
    let provenance_card = provenance.as_ref().map(render_provenance_card);
    if !is_slash_command
        && pipeline_template.is_none()
        && let Some(chain) = provenance.as_ref()
    {
        emit_provenance_card(chain, &event_sender).await;
    }
    let cancel_token = session.cancel_token.clone();
    let session_id = session.session_id.clone();
    let workdir = workdir.to_path_buf();
    let workdir_for_logging = workdir.clone();
    let roko_config = roko_config.clone();
    let roko_config_for_logging = roko_config.clone();
    let prompt_text_for_logging = prompt_text.clone();
    let model_key_for_logging = model_key.clone();
    let dispatch_started = Instant::now();
    let is_pipeline_dispatch = pipeline_template.is_some();

    let clippy_enabled = session.config_state.clippy_enabled;
    let tests_enabled = session.config_state.tests_enabled;
    let max_iterations = session.config_state.max_iterations;
    let review_strictness = session.config_state.review_strictness.clone();

    let shared_run = session.shared_run.clone();
    // SP-1: build a restrictive layer per dispatch; missing contracts fall closed.
    let pre_dispatch_violation = {
        let safety = SafetyLayer::with_defaults().with_role(&session.config_state.agent_mode);
        match safety.pre_dispatch_check(
            &session.session_id,
            "session-prompt",
            &session.config_state.agent_mode,
            &workdir,
        ) {
            Ok(()) => None,
            Err(violation) => {
                match violation.severity {
                    ViolationSeverity::Block => {
                        error!(
                            session_id = %session.session_id,
                            violation = ?violation.violation_type,
                            message = %violation.message,
                            "ACP pre-dispatch safety check BLOCKED dispatch"
                        );
                        Some(violation)
                    }
                    ViolationSeverity::Warn => {
                        warn!(
                            session_id = %session.session_id,
                            violation = ?violation.violation_type,
                            message = %violation.message,
                            "ACP pre-dispatch safety warning"
                        );
                        None
                    }
                }
            }
        }
    };

    // Shared channel for the workflow engine path: the cognitive task writes the
    // WorkflowRunReport's actual cost (which was aggregated from AgentCompleted events)
    // here so that append_acp_episode can use it instead of the pricing-table estimate.
    let workflow_cost_sink: Arc<Mutex<Option<f64>>> = Arc::new(Mutex::new(None));
    let workflow_cost_sink_task = Arc::clone(&workflow_cost_sink);

    let cognitive_task = tokio::spawn(async move {
        if let Some(violation) = pre_dispatch_violation {
            let message = violation.message;
            let _ = event_sender
                .send(CognitiveEvent::TokenChunk(format!(
                    "Safety check blocked this action: {}",
                    message
                )))
                .await;
            let _ = event_sender
                .send(CognitiveEvent::Complete {
                    stop_reason: StopReason::EndTurn,
                    usage: None,
                })
            .await;
            return Err(anyhow::anyhow!(
                "ACP pre-dispatch safety violation: {}",
                message
            )
            .into());
        }

        if is_slash_command {
            return run_slash_command(
                &session_id,
                prompt_text.trim(),
                &workdir,
                resolved.slug.clone(),
                cancel_token,
                event_sender,
                shared_run,
            )
            .await;
        }

        if let Some(template) = pipeline_template {
            if std::env::var_os("ROKO_ACP_LEGACY").is_some() {
                let legacy_run = shared_run.clone();
                let result = crate::runner::run_workflow_pipeline(
                    &session_id,
                    &prompt_text,
                    knowledge_context.clone(),
                    provenance_card.clone(),
                    &workdir,
                    crate::runner::PipelineConfig {
                        template,
                        max_iterations,
                        clippy_enabled,
                        tests_enabled,
                        review_strictness,
                        model_slug: resolved.slug.clone(),
                    },
                    cancel_token,
                    event_sender,
                    legacy_run.clone(),
                )
                .await;

                result?;

                let final_phase = legacy_run
                    .lock()
                    .await
                    .as_ref()
                    .map(|run| run.pipeline.phase.clone());

                return match final_phase {
                    Some(crate::pipeline::PipelinePhase::Complete) => Ok(()),
                    Some(crate::pipeline::PipelinePhase::Halted { reason }) => {
                        Err(anyhow::anyhow!("workflow pipeline halted: {reason}").into())
                    }
                    Some(crate::pipeline::PipelinePhase::Cancelled) => {
                        Err(anyhow::anyhow!("workflow pipeline cancelled").into())
                    }
                    Some(phase) => Err(anyhow::anyhow!(
                        "workflow pipeline ended in unexpected phase: {phase:?}"
                    )
                    .into()),
                    None => Err(anyhow::anyhow!(
                        "workflow pipeline completed without shared run state"
                    )
                    .into()),
                };
            }

            let report = run_with_workflow_engine(
                &session_id,
                &prompt_text,
                &workdir,
                workflow_template_name(&template),
                provenance_card,
                event_sender,
            )
            .await?;

            // Thread the actual cost from the report back to the main task so
            // append_acp_episode can record it instead of using the pricing-table estimate.
            if let Some(cost) = report.cost
                && let Ok(mut sink) = workflow_cost_sink_task.lock()
            {
                *sink = Some(cost);
            }

            if !report.success {
                return Err(anyhow::anyhow!(
                    "workflow engine reported unsuccessful run: {}",
                    report.output
                )
                .into());
            }

            return Ok(());
        }

        // Default: single-agent dispatch (workflow = "none").
        let provider_kind = resolved.provider_kind;

        info!(
            model_key = %model_key,
            slug = %resolved.slug,
            provider_kind = ?provider_kind,
            "resolved model for ACP prompt"
        );

        match provider_kind {
            ProviderKind::ClaudeCli => {
                // Build CLI prompt with history and file context prepended.
                let mut full_prompt = String::new();
                if !file_context.is_empty() {
                    full_prompt.push_str(&file_context);
                    full_prompt.push('\n');
                }
                if !history_context.is_empty() {
                    full_prompt.push_str(&history_context);
                }
                full_prompt.push_str(&prompt_text);

                run_claude_cognitive_task(
                    &session_id,
                    &full_prompt,
                    &workdir,
                    &resolved.slug,
                    "bypassPermissions",
                    &system_prompt,
                    cancel_token,
                    event_sender,
                )
                .await
            }
            ProviderKind::OpenAiCompat
            | ProviderKind::AnthropicApi
            | ProviderKind::GeminiApi
            | ProviderKind::PerplexityApi => {
                run_openai_compat_cognitive_task(
                    &session_id,
                    &messages,
                    &model_key,
                    &roko_config,
                    cancel_token,
                    event_sender,
                )
                .await
            }
            _ => {
                run_openai_compat_cognitive_task(
                    &session_id,
                    &messages,
                    &model_key,
                    &roko_config,
                    cancel_token,
                    event_sender,
                )
                .await
            }
        }
    });

    let stream_result = stream_events_to_editor(
        transport,
        &session.session_id,
        event_receiver,
        &session.cancel_token,
    )
    .await;

    if !is_slash_command {
        if let Ok(ref sr) = stream_result {
            if let Some(usage) = sr.usage.as_ref() {
                let size = resolved_for_logging
                    .profile
                    .as_ref()
                    .map(|profile| profile.context_window)
                    .unwrap_or_else(|| ModelProfile::default().context_window);
                let update = SessionUpdate::UsageUpdate {
                    used: usage.total_tokens,
                    size,
                    cost: calculate_cost_for_model_slug(
                        &resolved_for_logging.slug,
                        usage.input_tokens,
                        usage.output_tokens,
                        usage.cached_read_tokens.unwrap_or(0),
                    )
                    .map(|amount| CostInfo {
                        amount,
                        currency: "USD".to_string(),
                    }),
                };
                let _ = send_session_update(transport, &session.session_id, update).await;
            }
        }
    }

    let task_result = cognitive_task.await;
    let (task_error, task_join_error) = match task_result {
        Ok(Ok(())) => (None, None),
        Ok(Err(e)) => {
            let error_text = e.to_string();
            if error_text.starts_with("ACP pre-dispatch safety violation:") {
                warn!(error = %error_text, "cognitive task blocked before dispatch");
            } else {
                error!(error = %error_text, "cognitive task failed");
            }
            (Some(error_text), None)
        }
        Err(join_error) => {
            let error_text = join_error.to_string();
            error!(error = %error_text, "cognitive task failed to join");
            (
                Some(error_text),
                Some(BridgeEventsError::TaskJoin(join_error)),
            )
        }
    };
    let stream_error = stream_result.as_ref().err().map(|err| err.to_string());

    if let Ok(ref sr) = stream_result {
        if !sr.assistant_text.is_empty() {
            let safety = SafetyLayer::with_defaults().with_role(&session.config_state.agent_mode);
            let violations = safety.post_dispatch_check(
                &session.session_id,
                "session-prompt",
                &session.config_state.agent_mode,
                &sr.assistant_text,
                &[],
            );
            for v in &violations {
                // The response has already streamed; block-level findings are only logged here.
                match v.severity {
                    ViolationSeverity::Warn | ViolationSeverity::Block => {
                        warn!(
                            session_id = %session.session_id,
                            violation = ?v.violation_type,
                            message = %v.message,
                            "ACP post-dispatch safety violation"
                        );
                    }
                }
            }
        }
    }

    if !is_slash_command {
        // For the workflow engine path, the cognitive task wrote the actual provider cost
        // (from WorkflowRunReport) to workflow_cost_sink. Use it to override the
        // pricing-table estimate in append_acp_episode so the episode has accurate cost data.
        let cost_override = workflow_cost_sink.lock().ok().and_then(|g| *g);
        append_acp_episode(
            &roko_config_for_logging,
            &workdir_for_logging,
            session,
            &model_key_for_logging,
            &prompt_text_for_logging,
            &workflow_config,
            is_pipeline_dispatch,
            dispatch_started,
            stream_result.as_ref().ok(),
            task_error.as_deref(),
            stream_error.as_deref(),
            cost_override,
        )
        .await;

        let stream_result_ref = stream_result.as_ref().ok();
        let dispatch_succeeded = acp_dispatch_succeeded(
            stream_result_ref,
            task_error.as_deref(),
            stream_error.as_deref(),
        );
        let model_slugs =
            cascade_router_model_slugs(&roko_config_for_logging, &resolved_for_logging.slug);
        let routing_ctx =
            acp_routing_context(&session.config_state.agent_mode, &prompt_text_for_logging);
        let output_tokens =
            stream_result_ref.and_then(|sr| sr.usage.as_ref().map(|usage| usage.output_tokens));
        record_cascade_observation(
            workdir_for_logging
                .join(".roko")
                .join("learn")
                .join("cascade-router.json"),
            resolved_for_logging.slug,
            routing_ctx,
            dispatch_succeeded,
            dispatch_started.elapsed().as_millis() as u64,
            output_tokens,
            model_slugs,
        );
    }

    if let Some(join_error) = task_join_error {
        return Err(join_error);
    }

    // Push assistant turn after streaming completes (skip slash commands).
    match &stream_result {
        Ok(sr) if !is_slash_command && !sr.assistant_text.is_empty() => {
            session.push_assistant_turn(truncate_assistant_history(&sr.assistant_text));
        }
        _ => {}
    }

    stream_result.map(|sr| sr.prompt_result)
}

// ── Legacy Claude CLI dispatch ───────────────────────────────────────

/// Handles legacy Claude CLI model selections without spawning a subprocess.
///
/// TODO(arch): Replace this compatibility shim with provider-backed
/// `ModelCallService` dispatch for single-agent ACP prompts. WorkflowEngine
/// already uses the shared provider abstraction through `run_with_workflow_engine`.
#[allow(clippy::too_many_arguments)]
async fn run_claude_cognitive_task(
    _session_id: &str,
    _prompt_text: &str,
    _workdir: &Path,
    _model: &str,
    _permission_mode: &str,
    _system_prompt: &str,
    _cancel_token: CancelToken,
    event_sender: mpsc::Sender<CognitiveEvent>,
) -> Result<()> {
    let _ = event_sender
        .send(CognitiveEvent::TokenChunk(
            "Claude CLI dispatch is disabled in this ACP path. Configure a provider-backed model or enable the WorkflowEngine path.".to_string(),
        ))
        .await;
    let _ = event_sender
        .send(CognitiveEvent::Complete {
            stop_reason: StopReason::EndTurn,
            usage: None,
        })
        .await;

    Err(anyhow::anyhow!("Claude CLI dispatch is disabled in this ACP path").into())
}

// ── OpenAI-compatible provider dispatch ──────────────────────────────

/// Streams a prompt through an OpenAI-compatible provider (zhipu/GLM,
/// moonshot/Kimi, OpenAI, Perplexity, Ollama, etc.) using the config
/// from roko.toml. Accepts a pre-built messages array (with system prompt + history).
async fn run_openai_compat_cognitive_task(
    session_id: &str,
    messages: &[serde_json::Value],
    model_key: &str,
    roko_config: &RokoConfig,
    cancel_token: CancelToken,
    event_sender: mpsc::Sender<CognitiveEvent>,
) -> Result<()> {
    let resolved = resolve_model(roko_config, model_key);
    let provider_config = resolved.provider_config.as_ref();

    let base_url = provider_config
        .and_then(|p| p.base_url.as_deref())
        .unwrap_or("https://api.openai.com/v1");

    let api_key = provider_config
        .and_then(|p| p.resolve_api_key())
        .unwrap_or_default();

    let timeout_ms = provider_config
        .and_then(|p| p.timeout_ms)
        .unwrap_or(120_000);

    let slug = &resolved.slug;

    info!(
        session_id,
        model_key,
        slug,
        base_url,
        has_api_key = !api_key.is_empty(),
        "dispatching prompt via OpenAI-compat provider"
    );

    if cancel_token.is_cancelled() {
        return Ok(());
    }

    // Build the request body with pre-built messages array.
    let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": slug,
        "messages": messages,
        "stream": true
    });

    let client = reqwest::Client::new();
    let mut request = client
        .post(&endpoint)
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .header("Content-Type", "application/json");

    if !api_key.is_empty() {
        request = request.header("Authorization", format!("Bearer {api_key}"));
    }

    // Inject any extra headers from the provider config.
    if let Some(extra) = provider_config.and_then(|p| p.extra_headers.as_ref()) {
        for (k, v) in extra {
            request = request.header(k.as_str(), v.as_str());
        }
    }

    let response = match request.json(&body).send().await {
        Ok(r) => r,
        Err(e) => {
            error!(session_id, error = %e, "HTTP request to provider failed");
            let _ = event_sender
                .send(CognitiveEvent::TokenChunk(format!(
                    "Error: failed to connect to {base_url}: {e}"
                )))
                .await;
            let _ = event_sender
                .send(CognitiveEvent::Complete {
                    stop_reason: StopReason::EndTurn,
                    usage: None,
                })
                .await;
            return Err(anyhow::anyhow!("failed to connect to {base_url}: {e}").into());
        }
    };

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        error!(session_id, %status, "provider returned error: {error_text}");
        let _ = event_sender
            .send(CognitiveEvent::TokenChunk(format!(
                "Error ({status}): {error_text}"
            )))
            .await;
        let _ = event_sender
            .send(CognitiveEvent::Complete {
                stop_reason: StopReason::EndTurn,
                usage: None,
            })
            .await;
        return Err(anyhow::anyhow!("provider returned {status}: {error_text}").into());
    }

    // Stream SSE chunks.
    let mut response = response;
    let mut pending = Vec::new();
    let mut total_input = 0u64;
    let mut total_output = 0u64;
    let mut stream_error: Option<String> = None;

    loop {
        if cancel_token.is_cancelled() {
            return Ok(());
        }

        let chunk = tokio::select! {
            biased;
            _ = cancel_token.cancelled() => return Ok(()),
            result = response.chunk() => result,
        };

        let chunk = match chunk {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                warn!(session_id, error = %e, "error reading SSE chunk");
                stream_error = Some(e.to_string());
                break;
            }
        };

        pending.extend_from_slice(&chunk);

        // Process complete lines.
        while let Some(newline_idx) = pending.iter().position(|b| *b == b'\n') {
            let line_bytes: Vec<u8> = pending.drain(..=newline_idx).collect();
            let line = String::from_utf8_lossy(&line_bytes);
            let line = line.trim_end_matches(['\r', '\n']);

            if let Some(stream_chunk) = parse_sse_line(line) {
                match stream_chunk {
                    StreamChunk::ContentDelta(text) => {
                        if event_sender
                            .send(CognitiveEvent::TokenChunk(text))
                            .await
                            .is_err()
                        {
                            return Ok(());
                        }
                    }
                    StreamChunk::ReasoningDelta(text) => {
                        if event_sender
                            .send(CognitiveEvent::ThinkingChunk(text))
                            .await
                            .is_err()
                        {
                            return Ok(());
                        }
                    }
                    StreamChunk::Usage(usage) => {
                        total_input = u64::from(usage.input_tokens);
                        total_output = u64::from(usage.output_tokens);
                    }
                    StreamChunk::Done(_) => {}
                    StreamChunk::Error(e) => {
                        warn!(session_id, error = %e, "stream error from provider");
                        stream_error = Some(e.to_string());
                    }
                    StreamChunk::ToolCallDelta { .. } => {
                        // Tool calls not yet surfaced via ACP for openai-compat.
                    }
                }
            }
        }
    }

    // Process remaining bytes.
    if !pending.is_empty() {
        let line = String::from_utf8_lossy(&pending);
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(StreamChunk::ContentDelta(text)) = parse_sse_line(line) {
            let _ = event_sender.send(CognitiveEvent::TokenChunk(text)).await;
        }
    }

    let usage = if total_input > 0 || total_output > 0 {
        Some(UsageInfo {
            total_tokens: total_input + total_output,
            input_tokens: total_input,
            output_tokens: total_output,
            thought_tokens: None,
            cached_read_tokens: None,
            cached_write_tokens: None,
        })
    } else {
        None
    };

    let _ = event_sender
        .send(CognitiveEvent::Complete {
            stop_reason: StopReason::EndTurn,
            usage,
        })
        .await;

    if let Some(error) = stream_error {
        Err(anyhow::anyhow!("provider stream error: {error}").into())
    } else {
        Ok(())
    }
}

async fn emit_knowledge_card(
    knowledge: &DispatchKnowledge,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) {
    let Some(card) = knowledge.card() else {
        return;
    };

    let tool_call_id = "knowledge-query".to_string();
    let _ = event_sender
        .send(CognitiveEvent::ToolCallStart {
            tool_call_id: tool_call_id.clone(),
            title: card.title,
            kind: ToolCallKind::Other,
        })
        .await;
    let _ = event_sender
        .send(CognitiveEvent::ToolCallComplete {
            tool_call_id,
            status: ToolCallStatus::Completed,
            content: vec![ContentBlock::Text { text: card.body }],
        })
        .await;
}

/// A chain tracing why Roko chose a particular approach.
#[derive(Debug, Clone, PartialEq)]
struct ProvenanceChain {
    sources: Vec<ProvenanceSource>,
    confidence: f64,
}

/// One source in a decision provenance chain.
#[derive(Debug, Clone, PartialEq)]
enum ProvenanceSource {
    Playbook {
        id: String,
        goal: String,
        total_outcomes: u64,
        success_rate: f64,
    },
    Episode {
        task_id: String,
        success: bool,
        gate_summary: String,
    },
    Knowledge {
        kind: KnowledgeKind,
        tier: KnowledgeTier,
        score: f64,
        summary: String,
    },
    DreamPattern {
        description: String,
        guidance: String,
        confidence: f64,
    },
}

/// Build provenance from already-queried knowledge/playbook results and
/// best-effort episode/dream lookups.
async fn build_provenance(
    knowledge_hits: &[KnowledgeQueryHit],
    playbooks: &[Playbook],
    prompt: &str,
    workdir: &Path,
) -> Option<ProvenanceChain> {
    let mut sources = Vec::new();
    let mut has_playbook_source = false;

    for playbook in playbooks {
        let Some(success_rate) = playbook.success_rate() else {
            continue;
        };

        has_playbook_source = true;
        sources.push(ProvenanceSource::Playbook {
            id: playbook.id.clone(),
            goal: truncate_with_limit(playbook.goal.trim(), 80, "..."),
            total_outcomes: playbook.total_outcomes(),
            success_rate,
        });
    }

    let episodes_path = workdir.join(".roko").join("episodes.jsonl");
    let prompt_keywords = prompt_keywords(prompt);
    let episodes_future = EpisodeLogger::read_all_lossy(&episodes_path);
    let dreams_future = async {
        if prompt_keywords.is_empty() {
            return Vec::new();
        }

        match task::spawn_blocking({
            let workdir = workdir.to_path_buf();
            move || load_dream_routing_advice(&workdir)
        })
        .await
        {
            Ok(Ok(advice)) => {
                let mut seen_signatures = HashSet::new();
                let mut dream_sources = Vec::new();
                for keyword in prompt_keywords {
                    for pattern in relevant_pattern_summaries(&advice, &keyword, 0.5, 2) {
                        if !seen_signatures.insert(pattern.signature) {
                            continue;
                        }

                        dream_sources.push(ProvenanceSource::DreamPattern {
                            description: truncate_with_limit(&pattern.description, 80, "..."),
                            guidance: truncate_with_limit(&pattern.guidance, 80, "..."),
                            confidence: pattern.confidence,
                        });

                        if dream_sources.len() == 2 {
                            return dream_sources;
                        }
                    }
                }

                dream_sources
            }
            Ok(Err(err)) => {
                warn!(
                    workdir = %workdir.display(),
                    error = %err,
                    "dream routing advice load failed"
                );
                Vec::new()
            }
            Err(err) => {
                warn!(
                    workdir = %workdir.display(),
                    error = %err,
                    "dream routing advice task failed"
                );
                Vec::new()
            }
        }
    };

    let (episodes_result, dream_sources) = tokio::join!(episodes_future, dreams_future);

    match episodes_result {
        Ok(episodes) => {
            let matched_ids: HashSet<&str> = playbooks.iter().map(|pb| pb.id.as_str()).collect();
            let mut episode_count = 0usize;
            for episode in episodes.iter().rev().take(100) {
                if !matched_ids.contains(episode.task_id.as_str()) {
                    continue;
                }

                let gate_summary = if episode.gate_verdicts.is_empty() {
                    String::from("no gate verdicts")
                } else {
                    episode
                        .gate_verdicts
                        .iter()
                        .map(|verdict| {
                            format!(
                                "{}:{}",
                                verdict.gate,
                                if verdict.passed { "pass" } else { "fail" }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                };

                sources.push(ProvenanceSource::Episode {
                    task_id: episode.task_id.clone(),
                    success: episode.success,
                    gate_summary,
                });

                episode_count += 1;
                if episode_count == 3 {
                    break;
                }
            }
        }
        Err(err) => {
            warn!(
                workdir = %workdir.display(),
                error = %err,
                "episode log read failed"
            );
        }
    }

    for hit in knowledge_hits.iter().take(3) {
        sources.push(ProvenanceSource::Knowledge {
            kind: hit.entry.kind,
            tier: hit.entry.tier,
            score: hit.total_score,
            summary: truncate_with_limit(hit.entry.content.trim(), 80, "..."),
        });
    }

    for source in dream_sources {
        sources.push(source);
    }

    if sources.is_empty() || (!has_playbook_source && sources.len() < 2) {
        return None;
    }

    let scores = sources
        .iter()
        .map(|source| match source {
            ProvenanceSource::Playbook { success_rate, .. } => *success_rate,
            ProvenanceSource::Episode { success, .. } => {
                if *success {
                    1.0
                } else {
                    0.0
                }
            }
            ProvenanceSource::Knowledge { score, .. } => score_to_confidence(*score),
            ProvenanceSource::DreamPattern { confidence, .. } => *confidence,
        })
        .collect::<Vec<_>>();

    let confidence = if scores.is_empty() {
        0.0
    } else {
        scores.iter().sum::<f64>() / scores.len() as f64
    };

    Some(ProvenanceChain {
        sources,
        confidence,
    })
}

/// Emit a provenance card into ACP updates.
async fn emit_provenance_card(
    chain: &ProvenanceChain,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) {
    let tool_call_id = format!("decision-provenance-{}", uuid::Uuid::new_v4());
    let _ = event_sender
        .send(CognitiveEvent::ToolCallStart {
            tool_call_id: tool_call_id.clone(),
            title: "Decision provenance".to_string(),
            kind: ToolCallKind::Other,
        })
        .await;
    let _ = event_sender
        .send(CognitiveEvent::ToolCallComplete {
            tool_call_id,
            status: ToolCallStatus::Completed,
            content: vec![ContentBlock::Text {
                text: render_provenance_card(chain),
            }],
        })
        .await;
}

fn render_provenance_card(chain: &ProvenanceChain) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "{} source{}, {:.0}% confidence",
        chain.sources.len(),
        if chain.sources.len() == 1 { "" } else { "s" },
        chain.confidence * 100.0
    ));
    lines.push(String::new());

    for source in &chain.sources {
        match source {
            ProvenanceSource::Playbook {
                id,
                goal,
                total_outcomes,
                success_rate,
            } => {
                lines.push(format!(
                    "- Playbook `{id}` ({} runs, {:.0}% success)",
                    total_outcomes,
                    success_rate * 100.0
                ));
                lines.push(format!("  Goal: {}", goal));
            }
            ProvenanceSource::Episode {
                task_id,
                success,
                gate_summary,
            } => {
                lines.push(format!(
                    "- Episode `{task_id}` [{}]",
                    if *success { "pass" } else { "fail" }
                ));
                lines.push(format!(
                    "  Gates: {}",
                    truncate_with_limit(gate_summary, 80, "...")
                ));
            }
            ProvenanceSource::Knowledge {
                kind,
                tier,
                score,
                summary,
            } => {
                lines.push(format!(
                    "- Knowledge [{}/{}] ({:.2})",
                    kind.as_str(),
                    knowledge_tier_label(*tier),
                    score
                ));
                lines.push(format!("  {}", summary));
            }
            ProvenanceSource::DreamPattern {
                description,
                guidance,
                confidence,
            } => {
                lines.push(format!("- Dream pattern ({:.0}%)", confidence * 100.0));
                lines.push(format!("  Description: {}", description));
                lines.push(format!("  Guidance: {}", guidance));
            }
        }
        lines.push(String::new());
    }

    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }

    lines.join("\n")
}

fn prompt_keywords(prompt: &str) -> Vec<String> {
    let mut keywords = Vec::new();

    for raw in prompt.split(|ch: char| !ch.is_alphanumeric()) {
        let keyword = raw.trim().to_ascii_lowercase();
        if keyword.len() <= 4 || keywords.iter().any(|existing| existing == &keyword) {
            continue;
        }

        keywords.push(keyword);
        if keywords.len() == 5 {
            break;
        }
    }

    keywords
}

fn knowledge_tier_label(tier: KnowledgeTier) -> &'static str {
    match tier {
        KnowledgeTier::Transient => "transient",
        KnowledgeTier::Working => "working",
        KnowledgeTier::Consolidated => "consolidated",
        KnowledgeTier::Persistent => "persistent",
    }
}

fn score_to_confidence(score: f64) -> f64 {
    let score = score.max(0.0);
    score / (1.0 + score)
}

// ── Slash command dispatch ───────────────────────────────────────────

/// Runs a roko CLI slash command and streams the output as ACP updates.
async fn run_slash_command(
    session_id: &str,
    raw_input: &str,
    workdir: &Path,
    model_slug: String,
    cancel_token: CancelToken,
    event_sender: mpsc::Sender<CognitiveEvent>,
    shared_run: crate::session::SharedWorkflowRun,
) -> Result<()> {
    let input = raw_input.trim_start_matches('/');
    let (command, args) = match input.split_once(char::is_whitespace) {
        Some((cmd, rest)) => (cmd.trim(), rest.trim()),
        None => (input.trim(), ""),
    };

    // Helper to send a usage hint and return early.
    macro_rules! require_args {
        ($cmd:expr, $hint:expr) => {
            if args.is_empty() {
                let _ = event_sender
                    .send(CognitiveEvent::TokenChunk(format!(
                        "Usage: /{} {}",
                        $cmd, $hint
                    )))
                    .await;
                let _ = event_sender
                    .send(CognitiveEvent::Complete {
                        stop_reason: StopReason::EndTurn,
                        usage: None,
                    })
                    .await;
                return Ok(());
            }
        };
    }

    // Map slash command names to roko CLI args.
    let cli_args: Vec<String> = match command {
        // ── Status & Diagnostics ──
        "status" => vec!["status".into()],
        "doctor" => vec!["doctor".into()],
        "config" => vec!["config".into(), "show".into()],
        "learn" => vec!["learn".into(), "all".into()],

        // ── Research (foraging phase) ──
        "research" => {
            require_args!("research", "<topic>");
            vec!["research".into(), "topic".into(), args.into()]
        }
        "search" => {
            require_args!("search", "<query>");
            vec!["research".into(), "search".into(), args.into()]
        }
        "enhance-prd" => {
            require_args!("enhance-prd", "<slug>");
            vec!["research".into(), "enhance-prd".into(), args.into()]
        }

        // ── Specification (PRD lifecycle) ──
        "prd-idea" => {
            require_args!("prd-idea", "<idea text>");
            vec!["prd".into(), "idea".into(), args.into()]
        }
        "prd-draft" => {
            require_args!("prd-draft", "<slug>");
            vec!["prd".into(), "draft".into(), "new".into(), args.into()]
        }
        "prd-list" => vec!["prd".into(), "list".into()],
        "prd-status" => vec!["prd".into(), "status".into()],
        "prd-plan" => {
            require_args!("prd-plan", "<slug>");
            vec!["prd".into(), "plan".into(), args.into()]
        }
        "prd-consolidate" => vec!["prd".into(), "consolidate".into()],

        // ── Planning ──
        "plan-list" => vec!["plan".into(), "list".into()],
        "plan-generate" => {
            require_args!("plan-generate", "<description>");
            vec!["plan".into(), "generate".into(), args.into()]
        }
        "plan-validate" => {
            let dir = if args.is_empty() { "plans/" } else { args };
            vec!["plan".into(), "validate".into(), dir.into()]
        }
        "plan-run" => {
            let dir = if args.is_empty() { "plans/" } else { args };
            vec!["plan".into(), "run".into(), dir.into()]
        }

        // ── Implementation & Execution ──
        "run" => {
            require_args!("run", "<prompt>");
            vec!["run".into(), args.into()]
        }
        "agents" => vec!["agent".into(), "list".into()],
        "agent-chat" => {
            require_args!("agent-chat", "<agent name>");
            vec!["agent".into(), "chat".into(), "--agent".into(), args.into()]
        }

        // ── Verification & Gates ──
        "build" => {
            return run_shell_command(
                session_id,
                "cargo build --workspace",
                workdir,
                cancel_token,
                event_sender,
            )
            .await;
        }
        "test" => {
            return run_shell_command(
                session_id,
                "cargo test --workspace",
                workdir,
                cancel_token,
                event_sender,
            )
            .await;
        }
        "clippy" => {
            return run_shell_command(
                session_id,
                "cargo clippy --workspace --no-deps -- -D warnings",
                workdir,
                cancel_token,
                event_sender,
            )
            .await;
        }
        "fmt" => {
            return run_shell_command(
                session_id,
                "cargo +nightly fmt --all --check",
                workdir,
                cancel_token,
                event_sender,
            )
            .await;
        }
        "gate" => {
            // Run the full gate pipeline sequentially.
            return run_shell_command(
                session_id,
                "cargo +nightly fmt --all --check && cargo clippy --workspace --no-deps -- -D warnings && cargo test --workspace",
                workdir,
                cancel_token, event_sender,
            ).await;
        }

        // ── Knowledge & Dreams ──
        "knowledge" => {
            require_args!("knowledge", "<topic>");
            vec!["knowledge".into(), "query".into(), args.into()]
        }
        "knowledge-stats" => vec!["knowledge".into(), "stats".into()],
        "dream" => vec!["knowledge".into(), "dream".into(), "run".into()],

        // ── Code Intelligence ──
        "index" => {
            let sub = if args.is_empty() { "stats" } else { args };
            let parts: Vec<&str> = sub.splitn(2, char::is_whitespace).collect();
            let mut v = vec!["index".into(), parts[0].into()];
            if parts.len() > 1 {
                v.push(parts[1].into());
            }
            v
        }
        "explain" => {
            require_args!("explain", "<topic>");
            vec!["explain".into(), args.into()]
        }
        "replay" => {
            require_args!("replay", "<hash>");
            vec!["replay".into(), args.into()]
        }

        // ── Feedback & Learning ──
        "learn-router" => vec!["learn".into(), "router".into()],
        "learn-episodes" => vec!["learn".into(), "episodes".into()],
        "learn-tune" => {
            let target = if args.is_empty() { "gates" } else { args };
            vec!["learn".into(), "tune".into(), target.into()]
        }

        // ── New commands (plan-show, plan-resume, analyze, review, agent-start/stop, knowledge-gc/backup, audit) ──
        "plan-show" => {
            require_args!("plan-show", "<name>");
            vec!["plan".into(), "show".into(), args.into()]
        }
        "plan-resume" => {
            let path = if args.is_empty() {
                ".roko/state/executor.json"
            } else {
                args
            };
            vec![
                "plan".into(),
                "run".into(),
                "plans/".into(),
                "--resume".into(),
                path.into(),
            ]
        }
        "analyze" => vec!["research".into(), "analyze".into()],
        "review" => {
            let target = if args.is_empty() { "HEAD~1" } else { args };
            return run_shell_command(
                session_id,
                &format!("git diff {target}"),
                workdir,
                cancel_token,
                event_sender,
            )
            .await;
        }
        "agent-start" => {
            require_args!("agent-start", "<name>");
            vec!["agent".into(), "start".into(), "--name".into(), args.into()]
        }
        "agent-stop" => {
            require_args!("agent-stop", "<name>");
            vec!["agent".into(), "stop".into(), "--name".into(), args.into()]
        }
        "knowledge-gc" => vec!["knowledge".into(), "gc".into()],
        "knowledge-backup" => vec!["knowledge".into(), "backup".into()],
        "audit" => vec!["config".into(), "plugins".into(), "audit".into()],

        // ── Workflow ──
        "workflow" => {
            let sub = if args.is_empty() { "list" } else { args };
            match sub {
                "list" | "status" | "cancel" | "resume" => {
                    let msg = match sub {
                        "list" => "\
Workflow pipelines:
  none     — Single agent, no pipeline (current default)
  express  — Implement → gate → commit (fastest)
  standard — Implement → gate → review → commit
  full     — Strategy → implement → gate → multi-review → commit
  auto     — Select pipeline based on task complexity

Use the Workflow dropdown in the status bar to select, or:
  /express <prompt>      Run express pipeline
  /full <prompt>         Run full pipeline
  /review-this           Review current changes
  /pipeline <name>       Run a named pipeline"
                            .to_string(),
                        "status" => {
                            let guard = shared_run.lock().await;
                            match guard.as_ref() {
                                Some(run) => run.status_summary(),
                                None => "No active workflow run. Start one with /express, /full, or select a workflow in the config dropdown.".to_string(),
                            }
                        }
                        "cancel" => "No active workflow to cancel.".to_string(),
                        "resume" => "No halted workflow to resume.".to_string(),
                        _ => "Unknown workflow subcommand. Use: list, status, cancel, resume"
                            .to_string(),
                    };
                    let _ = event_sender.send(CognitiveEvent::TokenChunk(msg)).await;
                    let _ = event_sender
                        .send(CognitiveEvent::Complete {
                            stop_reason: StopReason::EndTurn,
                            usage: None,
                        })
                        .await;
                    return Ok(());
                }
                _ => {
                    let _ = event_sender
                        .send(CognitiveEvent::TokenChunk(format!(
                            "Unknown workflow subcommand: {sub}\n\nUse: /workflow list | status | cancel | resume"
                        )))
                        .await;
                    let _ = event_sender
                        .send(CognitiveEvent::Complete {
                            stop_reason: StopReason::EndTurn,
                            usage: None,
                        })
                        .await;
                    return Ok(());
                }
            }
        }
        "express" => {
            require_args!("express", "<prompt>");
            let knowledge = query_dispatch_knowledge(workdir, args).await;
            emit_knowledge_card(&knowledge, &event_sender).await;
            let provenance_card =
                build_provenance(&knowledge.hits, &knowledge.playbooks, args, workdir)
                    .await
                    .as_ref()
                    .map(render_provenance_card);
            let knowledge_context = knowledge.context_text();
            if std::env::var_os("ROKO_ACP_LEGACY").is_some() {
                return Ok(crate::runner::run_workflow_pipeline(
                    session_id,
                    args,
                    knowledge_context,
                    provenance_card,
                    workdir,
                    crate::runner::PipelineConfig {
                        template: crate::pipeline::WorkflowTemplate::Express,
                        max_iterations: 2,
                        clippy_enabled: true,
                        tests_enabled: true,
                        review_strictness: "standard".to_string(),
                        model_slug: model_slug.clone(),
                    },
                    cancel_token,
                    event_sender,
                    shared_run,
                )
                .await?);
            }

            run_with_workflow_engine(
                session_id,
                args,
                workdir,
                "express",
                provenance_card,
                event_sender,
            )
            .await?;
            return Ok(());
        }
        "full" => {
            require_args!("full", "<prompt>");
            let knowledge = query_dispatch_knowledge(workdir, args).await;
            emit_knowledge_card(&knowledge, &event_sender).await;
            let provenance_card =
                build_provenance(&knowledge.hits, &knowledge.playbooks, args, workdir)
                    .await
                    .as_ref()
                    .map(render_provenance_card);
            let knowledge_context = knowledge.context_text();
            if std::env::var_os("ROKO_ACP_LEGACY").is_some() {
                return Ok(crate::runner::run_workflow_pipeline(
                    session_id,
                    args,
                    knowledge_context,
                    provenance_card,
                    workdir,
                    crate::runner::PipelineConfig {
                        template: crate::pipeline::WorkflowTemplate::Full,
                        max_iterations: 2,
                        clippy_enabled: true,
                        tests_enabled: true,
                        review_strictness: "standard".to_string(),
                        model_slug: model_slug.clone(),
                    },
                    cancel_token,
                    event_sender,
                    shared_run,
                )
                .await?);
            }

            run_with_workflow_engine(
                session_id,
                args,
                workdir,
                "full",
                provenance_card,
                event_sender,
            )
            .await?;
            return Ok(());
        }
        "review-this" => {
            return run_shell_command(session_id, "git diff", workdir, cancel_token, event_sender)
                .await;
        }
        "pipeline" => {
            require_args!("pipeline", "<name>");
            let _ = event_sender
                .send(CognitiveEvent::TokenChunk(format!(
                    "[Pipeline: {args}] Not yet implemented. Available: express, standard, full\n\nUse /workflow list to see all pipelines."
                )))
                .await;
            let _ = event_sender
                .send(CognitiveEvent::Complete {
                    stop_reason: StopReason::EndTurn,
                    usage: None,
                })
                .await;
            return Ok(());
        }

        // ── Help ──
        "help" => {
            let help_text = "\
Available commands (organized by Will's core loop):

  Status & Diagnostics
    /status            Workspace status, signals, agents, runs
    /doctor            Diagnose workspace bootstrap state
    /config            Show roko.toml configuration
    /learn             Learning state overview

  Research (foraging)
    /research <topic>  Deep research with citations (Perplexity)
    /search <query>    Quick web search
    /enhance-prd <slug> Enrich a PRD with web research

  Specification (PRD lifecycle)
    /prd-idea <text>   Capture a work item idea
    /prd-draft <slug>  Draft a new PRD
    /prd-list          List all PRDs
    /prd-status        PRD pipeline coverage report
    /prd-plan <slug>   Generate plan from published PRD
    /prd-consolidate   Scan PRDs for gaps and duplicates

  Planning
    /plan-list         List all plans
    /plan-show <name>  Show a specific plan
    /plan-generate     Generate plan from a prompt
    /plan-validate     Lint tasks.toml without executing
    /plan-run [dir]    Execute a plan (orchestrate→gate→persist)
    /plan-resume [path] Resume an interrupted plan run

  Implementation & Execution
    /run <prompt>      Single prompt → universal loop
    /agents            List agents and their status
    /agent-chat <name> Interactive chat with a specific agent
    /agent-start <name> Start a named agent
    /agent-stop <name>  Stop a running agent

  Verification & Gates
    /build             cargo build --workspace
    /test              cargo test --workspace
    /clippy            cargo clippy --workspace
    /fmt               cargo +nightly fmt --all --check
    /gate              Full pipeline: fmt + clippy + test
    /review [target]   git diff of target (default: HEAD~1)

  Research & Analysis
    /research <topic>  Deep research with citations (Perplexity)
    /search <query>    Quick web search
    /enhance-prd <slug> Enrich a PRD with web research
    /analyze           Analyze execution data

  Knowledge & Dreams
    /knowledge <topic> Query durable knowledge store
    /knowledge-stats   Knowledge store statistics
    /knowledge-gc      Garbage collect knowledge store
    /knowledge-backup  Backup knowledge store
    /dream             Dream consolidation (NREM→REM→integration)

  Code Intelligence
    /index [cmd]       Build/search/stats code index
    /explain <topic>   Explain a concept at 3 depth levels
    /replay <hash>     Walk signal DAG by hash

  Feedback & Learning
    /learn-router      Cascade router state and model routing
    /learn-episodes    Recent episode log
    /learn-tune [what] Tune adaptive thresholds

  Workflow Pipelines
    /workflow [sub]    list/status/cancel/resume workflows
    /express <prompt>  Express: implement → gate → commit
    /full <prompt>     Full: strategy → implement → gate → review → commit
    /review-this       Review current uncommitted changes
    /pipeline <name>   Run a named workflow pipeline

  System
    /audit             Plugin security audit

  /help               This message";
            let _ = event_sender
                .send(CognitiveEvent::TokenChunk(help_text.into()))
                .await;
            let _ = event_sender
                .send(CognitiveEvent::Complete {
                    stop_reason: StopReason::EndTurn,
                    usage: None,
                })
                .await;
            return Ok(());
        }

        _ => {
            let _ = event_sender
                .send(CognitiveEvent::TokenChunk(format!(
                    "Unknown command: /{command}\n\nType /help for available commands."
                )))
                .await;
            let _ = event_sender
                .send(CognitiveEvent::Complete {
                    stop_reason: StopReason::EndTurn,
                    usage: None,
                })
                .await;
            return Ok(());
        }
    };

    info!(session_id, command, ?cli_args, "executing slash command");

    // Find the roko binary.
    let roko_bin = std::env::current_exe().unwrap_or_else(|_| "roko".into());

    let mut child = match tokio::process::Command::new(&roko_bin)
        .args(&cli_args)
        .current_dir(workdir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = event_sender
                .send(CognitiveEvent::TokenChunk(format!(
                    "Failed to run `roko {}`:\n{e}",
                    cli_args.join(" ")
                )))
                .await;
            let _ = event_sender
                .send(CognitiveEvent::Complete {
                    stop_reason: StopReason::EndTurn,
                    usage: None,
                })
                .await;
            return Ok(());
        }
    };

    // Stream stdout line-by-line.
    let stdout = child.stdout.take().expect("stdout was piped");
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut line = String::new();
    let mut output = String::new();

    loop {
        if cancel_token.is_cancelled() {
            let _ = child.kill().await;
            return Ok(());
        }
        line.clear();
        let read = tokio::select! {
            biased;
            _ = cancel_token.cancelled() => {
                let _ = child.kill().await;
                return Ok(());
            }
            r = reader.read_line(&mut line) => r,
        };
        match read {
            Ok(0) => break,
            Ok(_) => output.push_str(&line),
            Err(e) => {
                warn!(session_id, error = %e, "error reading slash command output");
                break;
            }
        }
    }

    // Also capture stderr.
    if let Some(stderr) = child.stderr.take() {
        let mut stderr_buf = String::new();
        let mut stderr_reader = tokio::io::BufReader::new(stderr);
        while let Ok(n) = stderr_reader.read_line(&mut stderr_buf).await {
            if n == 0 {
                break;
            }
        }
        let stderr_trimmed = stderr_buf.trim();
        if !stderr_trimmed.is_empty() {
            output.push_str("\n--- stderr ---\n");
            output.push_str(stderr_trimmed);
        }
    }

    let _ = child.wait().await;

    if output.is_empty() {
        output = format!("/{command} completed (no output)");
    }

    let _ = event_sender.send(CognitiveEvent::TokenChunk(output)).await;
    let _ = event_sender
        .send(CognitiveEvent::Complete {
            stop_reason: StopReason::EndTurn,
            usage: None,
        })
        .await;

    Ok(())
}

/// Runs a raw shell command (for /build, /test, /clippy) and streams output.
async fn run_shell_command(
    session_id: &str,
    shell_cmd: &str,
    workdir: &Path,
    cancel_token: CancelToken,
    event_sender: mpsc::Sender<CognitiveEvent>,
) -> Result<()> {
    info!(session_id, shell_cmd, "executing shell command");

    let mut child = match tokio::process::Command::new("sh")
        .args(["-c", shell_cmd])
        .current_dir(workdir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = event_sender
                .send(CognitiveEvent::TokenChunk(format!(
                    "Failed to run `{shell_cmd}`: {e}"
                )))
                .await;
            let _ = event_sender
                .send(CognitiveEvent::Complete {
                    stop_reason: StopReason::EndTurn,
                    usage: None,
                })
                .await;
            return Ok(());
        }
    };

    let stdout = child.stdout.take().expect("stdout was piped");
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut line = String::new();
    let mut output = String::new();

    loop {
        if cancel_token.is_cancelled() {
            let _ = child.kill().await;
            return Ok(());
        }
        line.clear();
        let read = tokio::select! {
            biased;
            _ = cancel_token.cancelled() => {
                let _ = child.kill().await;
                return Ok(());
            }
            r = reader.read_line(&mut line) => r,
        };
        match read {
            Ok(0) => break,
            Ok(_) => output.push_str(&line),
            Err(e) => {
                warn!(session_id, error = %e, "error reading shell command output");
                break;
            }
        }
    }

    if let Some(stderr) = child.stderr.take() {
        let mut stderr_buf = String::new();
        let mut stderr_reader = tokio::io::BufReader::new(stderr);
        while let Ok(n) = stderr_reader.read_line(&mut stderr_buf).await {
            if n == 0 {
                break;
            }
        }
        let stderr_trimmed = stderr_buf.trim();
        if !stderr_trimmed.is_empty() {
            output.push_str("\n--- stderr ---\n");
            output.push_str(stderr_trimmed);
        }
    }

    let exit_status = child.wait().await;
    let code = exit_status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
    if code != 0 {
        output.push_str(&format!("\n\nProcess exited with code {code}"));
    }

    if output.is_empty() {
        output = format!("`{shell_cmd}` completed (no output)");
    }

    let _ = event_sender.send(CognitiveEvent::TokenChunk(output)).await;
    let _ = event_sender
        .send(CognitiveEvent::Complete {
            stop_reason: StopReason::EndTurn,
            usage: None,
        })
        .await;

    Ok(())
}

/// Maps a Claude tool name to an ACP tool call kind.
#[allow(dead_code)]
fn tool_name_to_kind(name: &str) -> ToolCallKind {
    match name {
        "Edit" | "MultiEdit" => ToolCallKind::Edit,
        "Write" => ToolCallKind::Create,
        "Bash" | "Terminal" => ToolCallKind::Terminal,
        _ => ToolCallKind::Other,
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

fn map_event_to_update(event: CognitiveEvent) -> SessionUpdate {
    match event {
        CognitiveEvent::TokenChunk(text) => SessionUpdate::AgentMessageChunk {
            content: text_block(text),
            _meta: None,
        },
        CognitiveEvent::ThinkingChunk(text) => SessionUpdate::AgentThoughtChunk {
            content: text_block(text),
        },
        CognitiveEvent::ToolCallStart {
            tool_call_id,
            title,
            kind,
        } => SessionUpdate::ToolCall {
            tool_call_id,
            title,
            kind,
            status: ToolCallStatus::InProgress,
            content: Vec::new(),
        },
        CognitiveEvent::ToolCallComplete {
            tool_call_id,
            status,
            content,
        } => SessionUpdate::ToolCallUpdate {
            tool_call_id,
            status,
            content,
        },
        CognitiveEvent::PlanUpdate { entries } => SessionUpdate::Plan { entries },
        CognitiveEvent::Complete { .. } | CognitiveEvent::MaxTokens => {
            unreachable!("terminal cognitive events are handled before update mapping")
        }
    }
}

async fn send_session_update<R, W>(
    transport: &mut StdioTransport<R, W>,
    session_id: &str,
    update: SessionUpdate,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let update_value = serde_json::to_value(update)?;
    let params = serde_json::json!({
        "sessionId": session_id,
        "update": update_value,
    });
    transport
        .send_notification("session/update", params)
        .await
        .map_err(BridgeEventsError::from)
}

fn extract_prompt_text(prompt: &[ContentBlock]) -> String {
    prompt
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } => text.clone(),
            ContentBlock::Resource { .. } => String::new(),
            ContentBlock::Diff { path, diff } => format!("diff {path}:\n{diff}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extracts `file://` URIs from Resource blocks in the prompt.
fn extract_resource_uris(prompt: &[ContentBlock]) -> Vec<String> {
    use crate::types::ResourceRef;
    prompt
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Resource {
                resource: ResourceRef::File { uri },
            } => Some(uri.clone()),
            _ => None,
        })
        .collect()
}

/// Reads file contents for the given URIs, returning XML-tagged file context.
/// Validates that paths stay within the workdir for security.
fn read_file_context(uris: &[String], workdir: &Path) -> String {
    let mut context = String::new();
    let workdir_canonical = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());

    for uri in uris {
        let path_str = uri.strip_prefix("file://").unwrap_or(uri);
        let path = PathBuf::from(path_str);

        // Security: ensure path is within workdir.
        let canonical = match path.canonicalize() {
            Ok(p) => p,
            Err(_) => continue,
        };
        if !canonical.starts_with(&workdir_canonical) {
            warn!(path = %path.display(), "skipping file outside workdir");
            continue;
        }

        match std::fs::read_to_string(&canonical) {
            Ok(contents) => {
                // Cap individual file at 32KB to avoid blowing up context.
                let truncated = truncate_with_limit(&contents, 32_768, "... [truncated at 32KB]");
                let rel_path = canonical
                    .strip_prefix(&workdir_canonical)
                    .unwrap_or(&canonical);
                context.push_str(&format!(
                    "<file path=\"{}\">\n{}\n</file>\n",
                    rel_path.display(),
                    truncated
                ));
            }
            Err(e) => {
                warn!(path = %canonical.display(), error = %e, "failed to read file for context");
            }
        }
    }

    context
}

/// Resolves context annotations in prompt blocks into a single context string.
///
/// Explicit file attachments are resolved as XML-tagged file content. Text
/// blocks are scanned for `@` mentions and each supported mention is resolved
/// to either git context or file content.
pub(crate) async fn resolve_context_items(prompt: &[ContentBlock], workdir: &Path) -> String {
    use crate::types::ResourceRef;

    let mut parts = Vec::new();

    for block in prompt {
        match block {
            ContentBlock::Resource {
                resource: ResourceRef::File { uri },
            } => match resolve_file_uri(uri, workdir).await {
                Ok(content) => parts.push(content),
                Err(error) => {
                    warn!(uri = %uri, error = %error, "failed to resolve file resource URI");
                }
            },
            ContentBlock::Text { text } => {
                for label in extract_at_mentions(text) {
                    match resolve_at_mention(&label, workdir).await {
                        Ok(content) => parts.push(content),
                        Err(error) => {
                            warn!(label = %label, error = %error, "failed to resolve @-mention");
                        }
                    }
                }
            }
            ContentBlock::Diff { .. } => {}
        }
    }

    parts.join("\n\n")
}

async fn resolve_file_uri(uri: &str, workdir: &Path) -> anyhow::Result<String> {
    let path_str = uri.strip_prefix("file://").unwrap_or(uri);
    let (rel_path, contents) = resolve_local_file_contents(Path::new(path_str), workdir).await?;
    Ok(format!(
        "<file path=\"{}\">\n{}\n</file>",
        rel_path.display(),
        contents
    ))
}

async fn resolve_at_mention(label: &str, workdir: &Path) -> anyhow::Result<String> {
    match label {
        "branch-diff" | "diff" => {
            let output = tokio::process::Command::new("git")
                .args(["diff"])
                .current_dir(workdir)
                .output()
                .await?;
            ensure_git_output_success(&output, "git diff")?;
            let diff = String::from_utf8_lossy(&output.stdout);
            let truncated = truncate_with_limit(&diff, 10_240, "...\n[truncated]");
            Ok(format!("--- branch diff ---\n{truncated}"))
        }
        "recent-commits" | "git-log" | "log" => {
            let output = tokio::process::Command::new("git")
                .args(["log", "--oneline", "-20"])
                .current_dir(workdir)
                .output()
                .await?;
            ensure_git_output_success(&output, "git log")?;
            let log = String::from_utf8_lossy(&output.stdout);
            let truncated = truncate_with_limit(&log, 10_240, "...\n[truncated]");
            Ok(format!("--- recent commits ---\n{truncated}"))
        }
        "status" | "git-status" => {
            let output = tokio::process::Command::new("git")
                .args(["status", "--short"])
                .current_dir(workdir)
                .output()
                .await?;
            ensure_git_output_success(&output, "git status")?;
            let status = String::from_utf8_lossy(&output.stdout);
            let truncated = truncate_with_limit(&status, 10_240, "...\n[truncated]");
            Ok(format!("--- git status ---\n{truncated}"))
        }
        _ => {
            let (rel_path, contents) =
                resolve_local_file_contents(Path::new(label), workdir).await?;
            Ok(format!("--- {} ---\n{contents}", rel_path.display()))
        }
    }
}

async fn resolve_local_file_contents(
    path: &Path,
    workdir: &Path,
) -> anyhow::Result<(PathBuf, String)> {
    let full_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workdir.join(path)
    };

    let workdir_canonical = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());
    let canonical = full_path
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("cannot canonicalize {}: {error}", full_path.display()))?;
    if !canonical.starts_with(&workdir_canonical) {
        return Err(anyhow::anyhow!(
            "path {} is outside workdir",
            canonical.display()
        ));
    }

    let contents = tokio::fs::read_to_string(&canonical).await?;
    let rel_path = canonical
        .strip_prefix(&workdir_canonical)
        .unwrap_or(&canonical)
        .to_path_buf();
    let truncated = truncate_with_limit(&contents, 32_768, "... [truncated at 32KB]");

    Ok((rel_path, truncated))
}

fn extract_at_mentions(text: &str) -> Vec<String> {
    let mut mentions = Vec::new();
    let mut search_start = 0;

    while let Some(relative_at) = text[search_start..].find('@') {
        let at_index = search_start + relative_at;
        let prev = text[..at_index].chars().next_back();
        if matches!(prev, Some(c) if c.is_alphanumeric() || c == '_' || c == '-' || c == '.') {
            search_start = at_index + 1;
            continue;
        }

        let mut end = at_index + 1;
        while end < text.len() {
            let ch = text[end..].chars().next().expect("valid char boundary");
            if ch.is_whitespace()
                || ch == '@'
                || matches!(
                    ch,
                    ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '<' | '>'
                )
            {
                break;
            }
            end += ch.len_utf8();
        }

        let label = text[at_index + 1..end].trim_matches(|ch: char| {
            matches!(
                ch,
                ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '<' | '>' | '\'' | '"'
            )
        });
        if !label.is_empty() && !label.starts_with('@') {
            mentions.push(label.to_owned());
        }

        search_start = end;
    }

    mentions
}

fn truncate_with_limit(text: &str, limit: usize, suffix: &str) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }

    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }

    let mut truncated = String::with_capacity(end + suffix.len());
    truncated.push_str(&text[..end]);
    truncated.push_str(suffix);
    truncated
}

fn ensure_git_output_success(output: &std::process::Output, command: &str) -> anyhow::Result<()> {
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        Err(anyhow::anyhow!("{command} failed"))
    } else {
        Err(anyhow::anyhow!("{command} failed: {stderr}"))
    }
}

fn workflow_template_name(template: &crate::pipeline::WorkflowTemplate) -> &'static str {
    match template {
        crate::pipeline::WorkflowTemplate::Express => "express",
        crate::pipeline::WorkflowTemplate::Standard => "standard",
        crate::pipeline::WorkflowTemplate::Full => "full",
    }
}

fn text_block(text: String) -> ContentBlock {
    ContentBlock::Text { text }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, duplex, empty};

    use super::*;
    use crate::{
        session::AcpSession,
        transport::StdioTransport,
        types::{JsonRpcNotification, PermissionDecision, SessionNewParams},
    };

    fn test_session(model: &str, workflow: &str) -> AcpSession {
        let mut session = AcpSession::new(SessionNewParams {
            session_name: None,
            client_capabilities: None,
            mcp_servers: Vec::new(),
        });
        session.config_state.model = model.to_string();
        session.config_state.workflow = workflow.to_string();
        session
    }

    async fn reply_to_permission_request<C>(client: C, result: serde_json::Value)
    where
        C: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .expect("read permission request");
        let request: serde_json::Value = serde_json::from_str(&line).expect("parse request");
        let request_id = request["id"].clone();
        let mut client = reader.into_inner();
        let response = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": result,
        });
        let payload = serde_json::to_vec(&response).expect("serialize response");
        client
            .write_all(&payload)
            .await
            .expect("write response bytes");
        client.write_all(b"\n").await.expect("write newline");
        client.flush().await.expect("flush response");
    }

    #[tokio::test]
    async fn stream_events_to_editor_emits_notifications_and_returns_completion() {
        let (client, server) = duplex(4096);
        let mut transport = StdioTransport::from_io(empty(), server);
        let mut reader = BufReader::new(client);
        let cancel_token = CancelToken::new();
        let (sender, receiver) = mpsc::channel(8);

        sender
            .send(CognitiveEvent::TokenChunk("hello".to_owned()))
            .await
            .expect("send token chunk");
        sender
            .send(CognitiveEvent::Complete {
                stop_reason: StopReason::EndTurn,
                usage: Some(UsageInfo {
                    total_tokens: 12,
                    input_tokens: 5,
                    output_tokens: 7,
                    thought_tokens: None,
                    cached_read_tokens: None,
                    cached_write_tokens: None,
                }),
            })
            .await
            .expect("send completion");
        drop(sender);

        let result =
            stream_events_to_editor(&mut transport, "sess_test", receiver, &cancel_token).await;
        let result = result.expect("stream should succeed");

        assert_eq!(result.prompt_result.stop_reason, StopReason::EndTurn);
        assert_eq!(
            result.usage.as_ref().map(|usage| usage.total_tokens),
            Some(12)
        );
        assert_eq!(
            result.usage.as_ref().map(|usage| usage.input_tokens),
            Some(5)
        );
        assert_eq!(
            result.usage.as_ref().map(|usage| usage.output_tokens),
            Some(7)
        );

        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .expect("read notification line");
        let notification: JsonRpcNotification =
            serde_json::from_str(&line).expect("deserialize notification");
        assert_eq!(notification.method, "session/update");
        assert_eq!(
            notification.params,
            Some(json!({
                "sessionId": "sess_test",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {
                        "type": "text",
                        "text": "hello"
                    }
                }
            }))
        );
    }

    #[tokio::test]
    async fn stream_events_to_editor_returns_cancelled_when_token_is_cancelled() {
        let (_client, server) = duplex(1024);
        let mut transport = StdioTransport::from_io(empty(), server);
        let cancel_token = CancelToken::new();
        let (_sender, receiver) = mpsc::channel(1);

        cancel_token.cancel();

        let result =
            stream_events_to_editor(&mut transport, "sess_cancel", receiver, &cancel_token)
                .await
                .expect("cancelled prompt should still return a result");

        assert_eq!(result.prompt_result.stop_reason, StopReason::Cancelled);
    }

    #[tokio::test]
    async fn handle_session_prompt_rejects_busy_sessions() {
        let (_client, server) = duplex(1024);
        let mut transport = StdioTransport::from_io(empty(), server);
        let mut session = AcpSession::new(SessionNewParams {
            session_name: None,
            client_capabilities: None,
            mcp_servers: Vec::new(),
        });
        let session_id = session.session_id.clone();
        session.begin_prompt();

        let roko_config = RokoConfig::default();
        let error = handle_session_prompt(
            &mut transport,
            &mut session,
            SessionPromptParams {
                session_id: session_id.clone(),
                prompt: vec![ContentBlock::Text {
                    text: "busy".to_owned(),
                }],
                include_context: false,
            },
            Path::new("."),
            &roko_config,
        )
        .await
        .expect_err("busy session should be rejected");

        assert_eq!(
            error.rpc_error(),
            Some((
                SESSION_BUSY,
                format!("session '{session_id}' already has an active prompt")
            ))
        );
    }

    #[tokio::test]
    async fn request_permission_returns_allow_for_pregranted_action() {
        let mut transport = StdioTransport::from_io(empty(), tokio::io::sink());
        let mut session = AcpSession::new(SessionNewParams {
            session_name: Some("perm-test".to_string()),
            client_capabilities: None,
            mcp_servers: Vec::new(),
        });
        let action = crate::types::PermissionAction::FileEdit;
        session.grant_always_allow(action.clone());

        let decision = request_permission(
            &mut transport,
            &mut session,
            Path::new("."),
            action,
            "Allow code agent to edit files?",
            "The code agent may read and modify files.",
        )
        .await;

        assert_eq!(decision, PermissionDecision::Allow);
    }

    #[tokio::test]
    async fn request_permission_persists_always_allow_decision() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        let workdir = tmp.path().to_path_buf();
        let mut session = AcpSession::new(SessionNewParams {
            session_name: Some("perm-test".to_string()),
            client_capabilities: None,
            mcp_servers: Vec::new(),
        });
        let action = crate::types::PermissionAction::FileEdit;

        let (client, server) = duplex(4096);
        let (server_reader, server_writer) = tokio::io::split(server);
        let mut transport = StdioTransport::from_io(server_reader, server_writer);
        let ((), decision) = tokio::join!(
            reply_to_permission_request(client, json!({ "decision": "always_allow" })),
            request_permission(
                &mut transport,
                &mut session,
                &workdir,
                action.clone(),
                "Allow code agent to edit files?",
                "The code agent may read and modify files.",
            ),
        );

        assert_eq!(decision, PermissionDecision::AlwaysAllow);
        assert!(session.always_allowed.contains(&action));
        assert!(AcpSession::load_workspace_trust(&workdir).contains(&action));
    }

    #[tokio::test]
    async fn request_permission_defaults_to_reject_on_malformed_response() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        let workdir = tmp.path().to_path_buf();
        let mut session = AcpSession::new(SessionNewParams {
            session_name: Some("perm-test".to_string()),
            client_capabilities: None,
            mcp_servers: Vec::new(),
        });
        let action = crate::types::PermissionAction::FileEdit;

        let (client, server) = duplex(4096);
        let (server_reader, server_writer) = tokio::io::split(server);
        let mut transport = StdioTransport::from_io(server_reader, server_writer);
        let ((), decision) = tokio::join!(
            reply_to_permission_request(client, json!({ "decision": "maybe" })),
            request_permission(
                &mut transport,
                &mut session,
                &workdir,
                action.clone(),
                "Allow code agent to edit files?",
                "The code agent may read and modify files.",
            ),
        );

        assert_eq!(decision, PermissionDecision::Reject);
        assert!(!session.always_allowed.contains(&action));
        assert!(AcpSession::load_workspace_trust(&workdir).is_empty());
    }

    #[tokio::test]
    async fn append_acp_episode_records_single_dispatch_episode() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        let workdir = tmp.path();
        let session = test_session("claude-sonnet-4-6", "none");
        let roko_config = RokoConfig::default();
        let stream_result = StreamResult {
            prompt_result: SessionPromptResult {
                stop_reason: StopReason::EndTurn,
            },
            assistant_text: "hello from acp".to_string(),
            usage: Some(UsageInfo {
                total_tokens: 12,
                input_tokens: 5,
                output_tokens: 7,
                thought_tokens: None,
                cached_read_tokens: Some(2),
                cached_write_tokens: Some(1),
            }),
        };
        let dispatch_started = Instant::now();

        tokio::time::sleep(std::time::Duration::from_millis(1)).await;

        append_acp_episode(
            &roko_config,
            workdir,
            &session,
            &session.config_state.model,
            "trim a file",
            &session.config_state.workflow,
            false,
            dispatch_started,
            Some(&stream_result),
            None,
            None,
            None,
        )
        .await;

        let episodes_path = workdir.join(".roko").join("episodes.jsonl");
        let episodes = EpisodeLogger::read_all(&episodes_path)
            .await
            .expect("read episodes");

        assert_eq!(episodes.len(), 1);
        let episode = &episodes[0];
        assert_eq!(episode.kind, "acp-dispatch");
        assert_eq!(episode.agent_template, "code");
        assert_eq!(episode.task_id, session.session_id);
        assert_eq!(episode.extra.get("entry_point"), Some(&json!("acp")));
        assert_eq!(
            episode.extra.get("session_id"),
            Some(&json!(episode.task_id.clone()))
        );
        assert_eq!(
            episode.extra.get("routing_mode"),
            Some(&json!("auto_override"))
        );
        assert!(episode.usage.wall_ms > 0);
        assert_eq!(episode.tokens_used, 12);
        assert_eq!(episode.usage.input_tokens, 5);
        assert_eq!(episode.usage.output_tokens, 7);
        assert_eq!(episode.usage.cache_read_tokens, 2);
        assert_eq!(episode.usage.cache_write_tokens, 1);
        assert!(episode.usage.cost_usd > 0.0);
        assert!(episode.usage.cost_usd_without_cache >= episode.usage.cost_usd);
        assert!(episode.success);
        assert_eq!(episode.failure_reason, None);
    }

    #[tokio::test]
    async fn append_acp_episode_records_pipeline_kind() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        let workdir = tmp.path();
        let session = test_session("claude-sonnet-4-6", "express");
        let roko_config = RokoConfig::default();
        let stream_result = StreamResult {
            prompt_result: SessionPromptResult {
                stop_reason: StopReason::EndTurn,
            },
            assistant_text: "pipeline complete".to_string(),
            usage: None,
        };
        let dispatch_started = Instant::now();

        tokio::time::sleep(std::time::Duration::from_millis(1)).await;

        append_acp_episode(
            &roko_config,
            workdir,
            &session,
            &session.config_state.model,
            "wire ACP logging",
            &session.config_state.workflow,
            true,
            dispatch_started,
            Some(&stream_result),
            None,
            None,
            None,
        )
        .await;

        let episodes_path = workdir.join(".roko").join("episodes.jsonl");
        let episodes = EpisodeLogger::read_all(&episodes_path)
            .await
            .expect("read episodes");

        assert_eq!(episodes.len(), 1);
        let episode = &episodes[0];
        assert_eq!(episode.kind, "acp-pipeline-express");
        assert_eq!(episode.extra.get("workflow"), Some(&json!("express")));
        assert!(episode.success);
    }

    #[test]
    fn acp_routing_context_maps_modes_to_roles() {
        let plan = acp_routing_context("plan", "wire router feedback");
        assert_eq!(plan.task_category, TaskCategory::Implementation);
        assert_eq!(plan.role, AgentRole::Strategist);

        let research = acp_routing_context("research", "find the source of truth");
        assert_eq!(research.task_category, TaskCategory::Research);
        assert_eq!(research.role, AgentRole::Researcher);

        let code = acp_routing_context("code", "edit file");
        assert_eq!(code.task_category, TaskCategory::Implementation);
        assert_eq!(code.role, AgentRole::Implementer);
    }

    #[test]
    fn acp_dispatch_reward_distinguishes_success_and_failure() {
        assert_eq!(compute_acp_reward(false, 200, Some(120)), 0.0);
        assert!(compute_acp_reward(true, 1_000, Some(1_000)) > 0.9);
        assert!(compute_acp_reward(true, 20_000, None) >= 0.8);
    }

    #[test]
    fn cascade_router_model_slugs_falls_back_when_config_is_empty() {
        let config = RokoConfig::default();
        let slugs = cascade_router_model_slugs(&config, "fallback-slug");
        assert_eq!(slugs, vec!["fallback-slug".to_string()]);
    }

    #[test]
    fn calculate_cost_for_model_slug_handles_known_and_unknown_models() {
        let known = calculate_cost_for_model_slug("claude-sonnet-4-6", 1_000, 500, 250)
            .expect("known pricing should exist");
        assert!(known > 0.0);

        assert_eq!(
            calculate_cost_for_model_slug("definitely-not-a-real-model", 1_000, 500, 250),
            None
        );
    }

    #[test]
    fn assistant_history_truncation_caps_bytes_and_preserves_boundaries() {
        let text = "é".repeat(6_000);
        let truncated = truncate_assistant_history(&text);
        let suffix = "...[truncated]";
        let prefix_len = truncated.len() - suffix.len();

        assert!(truncated.ends_with(suffix));
        assert!(truncated.len() <= MAX_HISTORY_ASSISTANT_BYTES + suffix.len());
        assert!(truncated.len() < text.len());
        assert!(truncated[..prefix_len].chars().all(|c| c == 'é'));
    }

    #[test]
    fn tool_name_mapping() {
        assert_eq!(tool_name_to_kind("Edit"), ToolCallKind::Edit);
        assert_eq!(tool_name_to_kind("Write"), ToolCallKind::Create);
        assert_eq!(tool_name_to_kind("Bash"), ToolCallKind::Terminal);
        assert_eq!(tool_name_to_kind("Read"), ToolCallKind::Other);
    }

    #[test]
    fn extract_at_mentions_supports_embedded_mentions() {
        let mentions = extract_at_mentions("fix @src/main.rs and @branch-diff, not foo@bar.com");
        assert_eq!(mentions, vec!["src/main.rs", "branch-diff"]);
    }

    #[tokio::test]
    async fn resolve_context_items_resolves_resource_and_path_mentions() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        let workdir = tmp.path();
        let file_path = workdir.join("src/main.rs");
        std::fs::create_dir_all(file_path.parent().expect("parent directory"))
            .expect("create dirs");
        std::fs::write(&file_path, "fn main() {}\n").expect("write file");

        let prompt = vec![
            ContentBlock::Resource {
                resource: crate::types::ResourceRef::File {
                    uri: format!("file://{}", file_path.display()),
                },
            },
            ContentBlock::Text {
                text: "check @src/main.rs".to_owned(),
            },
        ];

        let context = resolve_context_items(&prompt, workdir).await;
        assert!(context.contains("<file path=\"src/main.rs\">"));
        assert!(context.contains("--- src/main.rs ---"));
        assert!(context.contains("fn main() {}"));
    }

    #[test]
    fn truncate_with_limit_is_char_safe() {
        let text = "é".repeat(20_000);
        let truncated = truncate_with_limit(&text, 32_768, "... [truncated]");
        let prefix_len = truncated.len() - "... [truncated]".len();

        assert!(truncated.ends_with("... [truncated]"));
        assert!(truncated.len() < text.len());
        assert!(truncated[..prefix_len].chars().all(|c| c == 'é'));
    }

    #[tokio::test]
    async fn build_provenance_includes_all_source_types() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        let workdir = tmp.path();
        std::fs::create_dir_all(workdir.join(".roko").join("learn")).expect("create learn dir");

        let playbook = Playbook {
            id: "dispatch-chain".into(),
            name: "dispatch-chain".into(),
            goal: "Reuse the proven dispatch path for similar tasks".into(),
            steps: Vec::new(),
            success_count: 3,
            failure_count: 1,
            created_at_ms: 0,
            last_used_ms: Some(0),
        };

        let mut episode = Episode::new("agent-1", playbook.id.as_str()).succeeded();
        episode.kind = "agent_turn".into();
        episode.gate_verdicts = vec![
            roko_learn::episode_logger::GateVerdict::new("compile", true),
            roko_learn::episode_logger::GateVerdict::new("test", true),
        ];
        let logger = EpisodeLogger::new(workdir.join(".roko").join("episodes.jsonl"));
        logger.append(&episode).await.expect("append episode");

        let advice = roko_dreams::DreamRoutingAdvice {
            generated_at: chrono::Utc::now(),
            source_dream_report: "dream-report".into(),
            recommendations: Vec::new(),
            pattern_summaries: vec![roko_dreams::PatternSummary {
                description: "dispatch decisions should show the evidence chain".into(),
                applies_to: vec!["dispatch".into()],
                guidance: "surface the chain before strategist work starts".into(),
                confidence: 0.91,
                signature: 42,
            }],
        };
        std::fs::write(
            workdir
                .join(".roko")
                .join("learn")
                .join("dream-routing-advice.json"),
            serde_json::to_string(&advice).expect("serialize dream advice"),
        )
        .expect("write dream advice");

        let knowledge_hits = vec![KnowledgeQueryHit {
            entry: roko_neuro::KnowledgeEntry {
                id: "knowledge-1".into(),
                kind: KnowledgeKind::StrategyFragment,
                content: "Prefer the proven dispatcher path".into(),
                confidence: 0.9,
                tier: KnowledgeTier::Persistent,
                source_episodes: vec![playbook.id.clone()],
                tags: vec!["dispatch".into()],
                ..Default::default()
            },
            total_score: 0.85,
            breakdown: roko_neuro::KnowledgeQueryBreakdown {
                keyword_score: 1.0,
                effective_confidence: 0.9,
                recency_factor: 1.0,
                emotional_boost: 1.0,
                hdc_similarity: None,
            },
        }];

        let chain = build_provenance(
            &knowledge_hits,
            &[playbook],
            "dispatch the request",
            workdir,
        )
        .await
        .expect("meaningful provenance");

        assert_eq!(chain.sources.len(), 4);
        assert!(matches!(
            chain.sources[0],
            ProvenanceSource::Playbook { .. }
        ));
        assert!(matches!(chain.sources[1], ProvenanceSource::Episode { .. }));
        assert!(matches!(
            chain.sources[2],
            ProvenanceSource::Knowledge { .. }
        ));
        assert!(matches!(
            chain.sources[3],
            ProvenanceSource::DreamPattern { .. }
        ));
        assert!(chain.confidence > 0.0);

        let card = render_provenance_card(&chain);
        assert!(card.contains("4 sources"));
        assert!(card.contains("Playbook `dispatch-chain`"));
        assert!(card.contains("Episode `dispatch-chain`"));
        assert!(card.contains("Knowledge [strategy_fragment/persistent]"));
        assert!(card.contains("Dream pattern"));
    }

    #[tokio::test]
    async fn build_provenance_suppresses_trivial_knowledge_only_chains() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        let workdir = tmp.path();

        let knowledge_hits = vec![KnowledgeQueryHit {
            entry: roko_neuro::KnowledgeEntry {
                id: "knowledge-2".into(),
                kind: KnowledgeKind::Insight,
                content: "A lone idea without supporting history".into(),
                confidence: 0.5,
                tier: KnowledgeTier::Working,
                ..Default::default()
            },
            total_score: 0.4,
            breakdown: roko_neuro::KnowledgeQueryBreakdown {
                keyword_score: 1.0,
                effective_confidence: 0.5,
                recency_factor: 1.0,
                emotional_boost: 1.0,
                hdc_similarity: None,
            },
        }];

        let chain = build_provenance(&knowledge_hits, &[], "small", workdir).await;
        assert!(chain.is_none());
    }
}
