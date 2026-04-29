//! Cognitive event to session/update streaming.
//!
//! Bridges Roko's provider system (via `roko-agent`) to ACP
//! `session/update` notifications.
//! All cognitive workflow dispatch now goes through
//! [`crate::runner::run_with_workflow_engine`], which uses `ModelCallService`
//! for provider-agnostic model calls.

use std::path::{Path, PathBuf};

use roko_agent::StreamChunk;
use roko_agent::streaming::parse_sse_line;
use roko_core::agent::{ProviderKind, resolve_model};
use roko_core::config::schema::RokoConfig;
use serde::Deserialize;
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite},
    sync::mpsc,
};
use tracing::{debug, error, info, warn};

use crate::runner::run_with_workflow_engine;
use crate::{
    session::{AcpSession, CancelToken},
    transport::{StdioTransport, TransportError, TransportResult},
    types::{
        ContentBlock, JsonRpcMessage, PlanEntry, SESSION_BUSY, SessionCancelParams,
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

/// Result of streaming events: the prompt result plus accumulated assistant text.
pub struct StreamResult {
    pub prompt_result: SessionPromptResult,
    /// Accumulated assistant text from TokenChunk events.
    pub assistant_text: String,
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
                    });
                };

                match event {
                    CognitiveEvent::Complete { stop_reason, .. } => {
                        return Ok(StreamResult {
                            prompt_result: SessionPromptResult { stop_reason },
                            assistant_text,
                        });
                    }
                    CognitiveEvent::MaxTokens => {
                        return Ok(StreamResult {
                            prompt_result: SessionPromptResult {
                                stop_reason: StopReason::MaxTokens,
                            },
                            assistant_text,
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

    debug!(
        session_id = %session.session_id,
        prompt_blocks = params.prompt.len(),
        prompt_chars = prompt_text.chars().count(),
        include_context = params.include_context,
        model_key = %model_key,
        workdir = %workdir.display(),
        "handling ACP session prompt"
    );

    // Build file context if include_context is set.
    let file_context = if params.include_context {
        let uris = extract_resource_uris(&params.prompt);
        if uris.is_empty() {
            String::new()
        } else {
            read_file_context(&uris, workdir)
        }
    } else {
        String::new()
    };

    // Get system prompt and history context (skip for slash commands).
    let system_prompt = session.system_prompt_for_mode().to_owned();
    let history_context = if is_slash_command {
        String::new()
    } else {
        session.build_history_context_for_cli()
    };
    let messages = if is_slash_command {
        Vec::new()
    } else {
        // Build combined system prompt with file context.
        let full_system = if file_context.is_empty() {
            system_prompt.clone()
        } else {
            format!("{system_prompt}\n\n{file_context}")
        };
        session.build_messages_array(&full_system, &prompt_text)
    };

    // Push user turn before dispatch (skip slash commands).
    if !is_slash_command {
        session.push_user_turn(prompt_text.clone());
    }

    let (event_sender, event_receiver) = mpsc::channel(64);
    let cancel_token = session.cancel_token.clone();
    let session_id = session.session_id.clone();
    let workdir = workdir.to_path_buf();
    let roko_config = roko_config.clone();

    // Capture workflow config for the pipeline.
    let workflow_config = session.config_state.workflow.clone();
    let clippy_enabled = session.config_state.clippy_enabled;
    let tests_enabled = session.config_state.tests_enabled;
    let max_iterations = session.config_state.max_iterations;
    let review_strictness = session.config_state.review_strictness.clone();

    let shared_run = session.shared_run.clone();

    let cognitive_task = tokio::spawn(async move {
        if is_slash_command {
            return run_slash_command(
                &session_id,
                prompt_text.trim(),
                &workdir,
                cancel_token,
                event_sender,
                shared_run,
            )
            .await;
        }

        // Check if a workflow pipeline should handle this prompt.
        let pipeline_template = if workflow_config == "auto" {
            Some(crate::pipeline::WorkflowTemplate::auto_select(&prompt_text))
        } else {
            crate::pipeline::WorkflowTemplate::from_config(&workflow_config)
        };
        if let Some(template) = pipeline_template {
            if std::env::var_os("ROKO_ACP_LEGACY").is_some() {
                return Ok(crate::runner::run_workflow_pipeline(
                    &session_id,
                    &prompt_text,
                    &workdir,
                    crate::runner::PipelineConfig {
                        template,
                        max_iterations,
                        clippy_enabled,
                        tests_enabled,
                        review_strictness,
                    },
                    cancel_token,
                    event_sender,
                    shared_run,
                )
                .await?);
            }

            run_with_workflow_engine(
                &session_id,
                &prompt_text,
                &workdir,
                workflow_template_name(&template),
                event_sender,
            )
            .await?;
            return Ok(());
        }

        // Default: single-agent dispatch (workflow = "none").
        // Resolve the model to determine which provider to use.
        let resolved = resolve_model(&roko_config, &model_key);
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

    let task_result = cognitive_task.await?;
    if let Err(e) = task_result {
        error!(error = %e, "cognitive task failed");
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

    Ok(())
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
            return Ok(());
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
        return Ok(());
    }

    // Stream SSE chunks.
    let mut response = response;
    let mut pending = Vec::new();
    let mut total_input = 0u64;
    let mut total_output = 0u64;

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

    Ok(())
}

// ── Slash command dispatch ───────────────────────────────────────────

/// Runs a roko CLI slash command and streams the output as ACP updates.
async fn run_slash_command(
    session_id: &str,
    raw_input: &str,
    workdir: &Path,
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
            if std::env::var_os("ROKO_ACP_LEGACY").is_some() {
                return Ok(crate::runner::run_workflow_pipeline(
                    session_id,
                    args,
                    workdir,
                    crate::runner::PipelineConfig {
                        template: crate::pipeline::WorkflowTemplate::Express,
                        max_iterations: 2,
                        clippy_enabled: true,
                        tests_enabled: true,
                        review_strictness: "standard".to_string(),
                    },
                    cancel_token,
                    event_sender,
                    shared_run,
                )
                .await?);
            }

            run_with_workflow_engine(session_id, args, workdir, "express", event_sender).await?;
            return Ok(());
        }
        "full" => {
            require_args!("full", "<prompt>");
            if std::env::var_os("ROKO_ACP_LEGACY").is_some() {
                return Ok(crate::runner::run_workflow_pipeline(
                    session_id,
                    args,
                    workdir,
                    crate::runner::PipelineConfig {
                        template: crate::pipeline::WorkflowTemplate::Full,
                        max_iterations: 2,
                        clippy_enabled: true,
                        tests_enabled: true,
                        review_strictness: "standard".to_string(),
                    },
                    cancel_token,
                    event_sender,
                    shared_run,
                )
                .await?);
            }

            run_with_workflow_engine(session_id, args, workdir, "full", event_sender).await?;
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
                let truncated = if contents.len() > 32_768 {
                    format!("{}... [truncated at 32KB]", &contents[..32_768])
                } else {
                    contents
                };
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
    use tokio::io::{AsyncBufReadExt, BufReader, duplex, empty};

    use super::*;
    use crate::{
        session::AcpSession,
        transport::StdioTransport,
        types::{JsonRpcNotification, SessionNewParams},
    };

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
}
