//! Cognitive event to session/update streaming.
//!
//! Bridges Roko's provider system (via `roko-agent`) to ACP
//! `session/update` notifications.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

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

#[allow(unused_imports)]
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
    let mcp_servers = session.mcp_servers.clone();

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
        // TODO(arch): Wire run_with_workflow_engine as alternative to run_workflow_pipeline.
        // When workflow config selects v2 engine, call:
        //   run_with_workflow_engine(session_id, prompt, workdir, template, event_sender).await
        if let Some(template) = pipeline_template {
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
                    &mcp_servers,
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
                    &mcp_servers,
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
            session.push_assistant_turn(sr.assistant_text.clone());
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

/// Maximum number of tool-call → result rounds within a single prompt.
/// Bounds runaway loops if a model keeps re-issuing tool calls forever.
const MAX_TOOL_ITERATIONS: usize = 8;

/// One streamed completion's outcome: text + any pending tool calls.
struct CompletionOutcome {
    /// Accumulated assistant text (may be empty when only tool calls).
    content: String,
    /// Tool calls the model wants to invoke before continuing.
    tool_calls: Vec<PendingToolCall>,
    /// Token usage for this leg (best-effort).
    usage: Option<UsageInfo>,
    /// Provider-reported finish reason — currently unused (tool-call detection
    /// goes through the `tool_calls` vec) but kept for future routing.
    #[allow(dead_code)]
    finish_reason: Option<roko_agent::chat_types::FinishReason>,
}

#[derive(Debug, Clone, Default)]
struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// Streams a prompt through an OpenAI-compatible provider (zhipu/GLM,
/// moonshot/Kimi, OpenAI, Perplexity, Ollama, etc.) using the config
/// from roko.toml. Accepts a pre-built messages array (with system prompt + history).
///
/// If `mcp_servers` is non-empty, this spawns the MCP servers, exposes their
/// tools to the model, executes any tool calls the model emits, and loops
/// until the model produces a final text-only response (or `MAX_TOOL_ITERATIONS`
/// is hit).
#[allow(clippy::too_many_arguments)]
async fn run_openai_compat_cognitive_task(
    session_id: &str,
    messages: &[serde_json::Value],
    model_key: &str,
    roko_config: &RokoConfig,
    mcp_servers: &[crate::types::McpServerConfig],
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
    let slug = resolved.slug.clone();
    let max_tokens = resolved
        .profile
        .as_ref()
        .and_then(|profile| profile.max_output)
        .and_then(|value| u32::try_from(value).ok());

    info!(
        session_id,
        model_key,
        slug = %slug,
        base_url,
        has_api_key = !api_key.is_empty(),
        mcp_server_count = mcp_servers.len(),
        "dispatching prompt via OpenAI-compat provider"
    );

    if cancel_token.is_cancelled() {
        return Ok(());
    }

    // Spawn declared MCP servers and discover their tools. Each entry is
    // namespaced by `server.tool_name` so multiple servers can coexist
    // without name collisions.
    let mcp_state = setup_session_mcp(session_id, mcp_servers).await;
    let openai_tools = render_openai_tools(&mcp_state.tools);

    let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let extra_headers: Vec<(String, String)> = provider_config
        .and_then(|p| p.extra_headers.as_ref())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    let client = reqwest::Client::new();
    let mut working_messages: Vec<serde_json::Value> = messages.to_vec();
    let mut total_input = 0u64;
    let mut total_output = 0u64;

    for iteration in 0..MAX_TOOL_ITERATIONS {
        if cancel_token.is_cancelled() {
            return Ok(());
        }

        let outcome = match stream_one_completion(
            session_id,
            &endpoint,
            &api_key,
            &extra_headers,
            timeout_ms,
            &slug,
            &working_messages,
            &openai_tools,
            max_tokens,
            cancel_token.clone(),
            &event_sender,
            &client,
        )
        .await
        {
            Ok(o) => o,
            Err(e) => {
                error!(session_id, error = %e, "OpenAI-compat completion failed");
                let _ = event_sender
                    .send(CognitiveEvent::TokenChunk(format!("\nError: {e}")))
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

        if let Some(u) = &outcome.usage {
            total_input += u.input_tokens;
            total_output += u.output_tokens;
        }

        // No tool calls → final answer; we're done.
        if outcome.tool_calls.is_empty() {
            let usage = (total_input > 0 || total_output > 0).then(|| UsageInfo {
                total_tokens: total_input + total_output,
                input_tokens: total_input,
                output_tokens: total_output,
                thought_tokens: None,
                cached_read_tokens: None,
                cached_write_tokens: None,
            });
            let _ = event_sender
                .send(CognitiveEvent::Complete {
                    stop_reason: StopReason::EndTurn,
                    usage,
                })
                .await;
            return Ok(());
        }

        // Append the assistant message (with tool_calls) to history so the
        // model can see what it asked for in the next turn.
        let assistant_tool_calls: Vec<serde_json::Value> = outcome
            .tool_calls
            .iter()
            .map(|tc| {
                serde_json::json!({
                    "id": tc.id,
                    "type": "function",
                    "function": {
                        "name": tc.name,
                        "arguments": if tc.arguments.is_empty() { "{}".to_string() } else { tc.arguments.clone() },
                    }
                })
            })
            .collect();
        let mut assistant_msg = serde_json::Map::new();
        assistant_msg.insert("role".into(), serde_json::Value::String("assistant".into()));
        if outcome.content.is_empty() {
            assistant_msg.insert("content".into(), serde_json::Value::Null);
        } else {
            assistant_msg.insert(
                "content".into(),
                serde_json::Value::String(outcome.content.clone()),
            );
        }
        assistant_msg.insert(
            "tool_calls".into(),
            serde_json::Value::Array(assistant_tool_calls),
        );
        working_messages.push(serde_json::Value::Object(assistant_msg));

        // Execute each tool call and append its result message.
        for tc in &outcome.tool_calls {
            if cancel_token.is_cancelled() {
                return Ok(());
            }

            // Surface the call in the editor as a pending tool card.
            let _ = event_sender
                .send(CognitiveEvent::ToolCallStart {
                    tool_call_id: tc.id.clone(),
                    title: tc.name.clone(),
                    kind: ToolCallKind::Other,
                })
                .await;

            let parsed_args: serde_json::Value = if tc.arguments.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&tc.arguments)
                    .unwrap_or_else(|_| serde_json::Value::String(tc.arguments.clone()))
            };

            let (status, result_text) = match dispatch_session_mcp_tool(
                &mcp_state,
                &tc.name,
                parsed_args,
            )
            .await
            {
                Ok(text) => (ToolCallStatus::Completed, text),
                Err(text) => (ToolCallStatus::Failed, text),
            };

            let _ = event_sender
                .send(CognitiveEvent::ToolCallComplete {
                    tool_call_id: tc.id.clone(),
                    status,
                    content: vec![ContentBlock::Text {
                        text: result_text.clone(),
                    }],
                })
                .await;

            working_messages.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": result_text,
            }));
        }

        debug!(
            session_id,
            iteration,
            tool_call_count = outcome.tool_calls.len(),
            "completed tool iteration; looping"
        );
    }

    warn!(
        session_id,
        max_iterations = MAX_TOOL_ITERATIONS,
        "tool-calling loop hit iteration cap; emitting Complete"
    );
    let _ = event_sender
        .send(CognitiveEvent::TokenChunk(format!(
            "\n[stopped after {MAX_TOOL_ITERATIONS} tool rounds — model kept requesting more tools]"
        )))
        .await;
    let _ = event_sender
        .send(CognitiveEvent::Complete {
            stop_reason: StopReason::MaxTokens,
            usage: None,
        })
        .await;
    Ok(())
}

/// Per-session MCP runtime: live clients keyed by server name + namespaced tool
/// catalog. Empty when no MCP servers were declared (or all failed to spawn).
struct SessionMcpState {
    /// `server_name` → live `McpClient` over stdio.
    clients: std::collections::HashMap<
        String,
        std::sync::Arc<roko_agent::mcp::McpClient<roko_agent::mcp::StdioTransport>>,
    >,
    /// Discovered tools, namespaced as `server.tool_name`. Each entry stores the
    /// tool's input schema for later OpenAI tool rendering.
    tools: Vec<NamespacedTool>,
}

#[derive(Clone)]
struct NamespacedTool {
    /// `server.tool_name` — the name the model emits in `tool_calls`.
    qualified_name: String,
    /// Owning server name (for routing the call back to the right client).
    server: String,
    /// Original (unprefixed) MCP tool name.
    bare_name: String,
    /// Tool description for the model.
    description: String,
    /// JSON-schema for the tool's input.
    schema: serde_json::Value,
}

/// Spawn each declared MCP server, run the `initialize` handshake, list its
/// tools, and namespace them as `server.tool_name`. Failures are logged but
/// don't fail the session — the model just doesn't see those tools.
async fn setup_session_mcp(
    session_id: &str,
    mcp_servers: &[crate::types::McpServerConfig],
) -> SessionMcpState {
    use roko_agent::mcp::{McpClient, StdioTransport};

    let mut state = SessionMcpState {
        clients: std::collections::HashMap::new(),
        tools: Vec::new(),
    };
    let mut used_tool_names = HashSet::new();

    for server in mcp_servers {
        let (command, args) = match &server.transport {
            crate::types::McpTransport::Stdio { command, args } => (command.clone(), args.clone()),
            crate::types::McpTransport::Http { url } => {
                warn!(
                    session_id,
                    server = %server.name,
                    url = %url,
                    "skipping HTTP MCP server — only stdio transport is supported via session/new today",
                );
                continue;
            }
        };

        let transport = match StdioTransport::spawn(&command, &args) {
            Ok(t) => t,
            Err(e) => {
                warn!(session_id, server = %server.name, error = %e, "failed to spawn MCP server");
                continue;
            }
        };

        let client = McpClient::new(transport);
        match tokio::time::timeout(std::time::Duration::from_secs(8), client.initialize()).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                warn!(session_id, server = %server.name, error = %e, "MCP initialize failed");
                continue;
            }
            Err(_) => {
                warn!(session_id, server = %server.name, "MCP initialize timed out after 8s");
                continue;
            }
        }

        let listed = match tokio::time::timeout(
            std::time::Duration::from_secs(8),
            client.list_tools(),
        )
        .await
        {
            Ok(Ok(t)) => t,
            Ok(Err(e)) => {
                warn!(session_id, server = %server.name, error = %e, "tools/list failed");
                continue;
            }
            Err(_) => {
                warn!(session_id, server = %server.name, "tools/list timed out");
                continue;
            }
        };

        info!(
            session_id,
            server = %server.name,
            tool_count = listed.len(),
            "discovered MCP tools"
        );
        for t in &listed {
            // OpenAI/Anthropic restrict tool names to `^[a-zA-Z0-9_-]{1,64}$` —
            // no dots — so we sanitize when building the model-visible name.
            // The original (`bare_name`) is preserved for the actual MCP call.
            let sanitized_server = sanitize_tool_segment(&server.name);
            let sanitized_tool = sanitize_tool_segment(&t.name);
            let base_qualified = format!("{sanitized_server}_{sanitized_tool}");
            let qualified = unique_tool_name(&base_qualified, &mut used_tool_names);
            if qualified != base_qualified {
                warn!(
                    session_id,
                    server = %server.name,
                    tool = %t.name,
                    base_name = %base_qualified,
                    qualified_name = %qualified,
                    "renamed MCP tool after sanitized name collision"
                );
            }
            state.tools.push(NamespacedTool {
                qualified_name: qualified,
                server: server.name.clone(),
                bare_name: t.name.clone(),
                description: t
                    .description
                    .clone()
                    .unwrap_or_else(|| format!("{} tool", t.name)),
                schema: t
                    .input_schema
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({"type": "object"})),
            });
        }
        state
            .clients
            .insert(server.name.clone(), std::sync::Arc::new(client));
    }

    state
}

/// Replace any character that's not in OpenAI's tool-name alphabet (`a-z`,
/// `A-Z`, `0-9`, `_`, `-`) with `_`. Truncates at 28 chars per segment so the
/// final `server_tool` form stays under the 64-char tool name limit even
/// when both segments are at max length.
fn sanitize_tool_segment(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars().take(28) {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

fn unique_tool_name(base: &str, used: &mut HashSet<String>) -> String {
    if used.insert(base.to_owned()) {
        return base.to_owned();
    }

    for i in 2.. {
        let suffix = format!("_{i}");
        let max_base_len = 64usize.saturating_sub(suffix.len());
        let mut candidate_base: String = base.chars().take(max_base_len).collect();
        candidate_base.push_str(&suffix);
        if used.insert(candidate_base.clone()) {
            return candidate_base;
        }
    }

    unreachable!("unbounded suffix search must produce a unique tool name")
}

/// Render namespaced tools into OpenAI's `tools: [...]` request shape. Returns
/// `None` when there are no tools (the request omits the field entirely so the
/// provider doesn't reject it).
fn render_openai_tools(tools: &[NamespacedTool]) -> Option<serde_json::Value> {
    if tools.is_empty() {
        return None;
    }
    Some(serde_json::Value::Array(
        tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.qualified_name,
                        "description": t.description,
                        "parameters": t.schema,
                    }
                })
            })
            .collect(),
    ))
}

/// Dispatch a model-emitted `tool_calls[].function.name` to the matching MCP
/// server and return its rendered text result. Returns `Err(text)` if the tool
/// cannot be resolved or the underlying call fails — that error text is fed
/// back to the model so it can recover.
async fn dispatch_session_mcp_tool(
    state: &SessionMcpState,
    qualified_name: &str,
    arguments: serde_json::Value,
) -> std::result::Result<String, String> {
    let tool = state
        .tools
        .iter()
        .find(|t| t.qualified_name == qualified_name)
        .ok_or_else(|| format!("unknown tool: {qualified_name}"))?;
    let client = state
        .clients
        .get(&tool.server)
        .ok_or_else(|| format!("MCP server '{}' is not connected", tool.server))?;

    let result = match tokio::time::timeout(
        std::time::Duration::from_secs(60),
        client.call_tool(&tool.bare_name, arguments),
    )
    .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return Err(format!("tool call failed: {e}")),
        Err(_) => return Err(format!("tool call timed out after 60s: {qualified_name}")),
    };

    let mut text = String::new();
    for block in &result.content {
        if let Some(t) = &block.text {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(t);
        }
    }
    if result.is_error {
        Err(if text.is_empty() {
            "tool reported an error".to_string()
        } else {
            text
        })
    } else if text.is_empty() {
        Ok("(empty result)".to_string())
    } else {
        Ok(text)
    }
}

/// Stream one chat completion request, emitting content/thinking deltas as
/// `CognitiveEvent`s and accumulating any `tool_calls` the model produces.
/// Does not emit `Complete` — the caller decides when the loop terminates.
#[allow(clippy::too_many_arguments)]
async fn stream_one_completion(
    session_id: &str,
    endpoint: &str,
    api_key: &str,
    extra_headers: &[(String, String)],
    timeout_ms: u64,
    slug: &str,
    messages: &[serde_json::Value],
    openai_tools: &Option<serde_json::Value>,
    max_tokens: Option<u32>,
    cancel_token: CancelToken,
    event_sender: &mpsc::Sender<CognitiveEvent>,
    client: &reqwest::Client,
) -> std::result::Result<CompletionOutcome, String> {
    let mut body = serde_json::json!({
        "model": slug,
        "messages": messages,
        "stream": true,
    });
    if let Some(body_obj) = body.as_object_mut() {
        if let Some(max_tokens) = max_tokens {
            body_obj.insert("max_tokens".into(), serde_json::Value::from(max_tokens));
        }
        if let Some(tools) = openai_tools {
            body_obj.insert("tools".into(), tools.clone());
            body_obj.insert(
                "tool_choice".into(),
                serde_json::Value::String("auto".into()),
            );
        }
    }

    let mut request = client
        .post(endpoint)
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .header("Content-Type", "application/json");
    if !api_key.is_empty() {
        request = request.header("Authorization", format!("Bearer {api_key}"));
    }
    for (k, v) in extra_headers {
        request = request.header(k.as_str(), v.as_str());
    }

    let response = request
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("connection failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("provider returned {status}: {error_text}"));
    }

    let mut response = response;
    let mut pending = Vec::new();
    let mut content = String::new();
    let mut tool_call_slots: Vec<PendingToolCall> = Vec::new();
    let mut usage: Option<UsageInfo> = None;
    let mut finish: Option<roko_agent::chat_types::FinishReason> = None;

    loop {
        if cancel_token.is_cancelled() {
            return Ok(CompletionOutcome {
                content,
                tool_calls: tool_call_slots,
                usage,
                finish_reason: finish,
            });
        }

        let chunk = tokio::select! {
            biased;
            _ = cancel_token.cancelled() => return Ok(CompletionOutcome {
                content,
                tool_calls: tool_call_slots,
                usage,
                finish_reason: finish,
            }),
            r = response.chunk() => r,
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

        while let Some(newline_idx) = pending.iter().position(|b| *b == b'\n') {
            let line_bytes: Vec<u8> = pending.drain(..=newline_idx).collect();
            let line = String::from_utf8_lossy(&line_bytes);
            let line = line.trim_end_matches(['\r', '\n']);
            if let Some(stream_chunk) = parse_sse_line(line) {
                apply_stream_chunk(
                    stream_chunk,
                    &mut content,
                    &mut tool_call_slots,
                    &mut usage,
                    &mut finish,
                    event_sender,
                )
                .await;
            }
        }
    }

    if !pending.is_empty() {
        let line = String::from_utf8_lossy(&pending);
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(stream_chunk) = parse_sse_line(line) {
            apply_stream_chunk(
                stream_chunk,
                &mut content,
                &mut tool_call_slots,
                &mut usage,
                &mut finish,
                event_sender,
            )
            .await;
        }
    }

    Ok(CompletionOutcome {
        content,
        tool_calls: tool_call_slots
            .into_iter()
            .filter(|tc| !tc.id.is_empty() || !tc.name.is_empty() || !tc.arguments.is_empty())
            .collect(),
        usage,
        finish_reason: finish,
    })
}

/// Apply one decoded `StreamChunk`: stream content/thinking to the editor
/// immediately, accumulate tool-call deltas in slot order, and capture
/// usage / finish_reason for the caller to inspect.
async fn apply_stream_chunk(
    chunk: StreamChunk,
    content: &mut String,
    tool_call_slots: &mut Vec<PendingToolCall>,
    usage: &mut Option<UsageInfo>,
    finish: &mut Option<roko_agent::chat_types::FinishReason>,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) {
    match chunk {
        StreamChunk::ContentDelta(text) => {
            content.push_str(&text);
            let _ = event_sender.send(CognitiveEvent::TokenChunk(text)).await;
        }
        StreamChunk::ReasoningDelta(text) => {
            let _ = event_sender.send(CognitiveEvent::ThinkingChunk(text)).await;
        }
        StreamChunk::Usage(u) => {
            *usage = Some(UsageInfo {
                total_tokens: u64::from(u.input_tokens) + u64::from(u.output_tokens),
                input_tokens: u64::from(u.input_tokens),
                output_tokens: u64::from(u.output_tokens),
                thought_tokens: None,
                cached_read_tokens: None,
                cached_write_tokens: None,
            });
        }
        StreamChunk::Done(reason) => {
            *finish = Some(reason);
        }
        StreamChunk::Error(e) => {
            warn!(error = %e, "stream error from provider");
        }
        StreamChunk::ToolCallDelta {
            index,
            id_delta,
            name_delta,
            arguments_delta,
        } => {
            while tool_call_slots.len() <= index {
                tool_call_slots.push(PendingToolCall::default());
            }
            let slot = &mut tool_call_slots[index];
            if let Some(id) = id_delta {
                if !id.is_empty() {
                    slot.id = id;
                }
            }
            if let Some(name) = name_delta {
                if !name.is_empty() {
                    slot.name = name;
                }
            }
            slot.arguments.push_str(&arguments_delta);
        }
    }
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
        "full" => {
            require_args!("full", "<prompt>");
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
    fn tool_name_mapping() {
        assert_eq!(tool_name_to_kind("Edit"), ToolCallKind::Edit);
        assert_eq!(tool_name_to_kind("Write"), ToolCallKind::Create);
        assert_eq!(tool_name_to_kind("Bash"), ToolCallKind::Terminal);
        assert_eq!(tool_name_to_kind("Read"), ToolCallKind::Other);
    }

    #[test]
    fn mcp_tool_names_are_unique_after_sanitizing() {
        let mut used = HashSet::new();

        let first = unique_tool_name("nunchi_desktop_tiles_create", &mut used);
        let second = unique_tool_name("nunchi_desktop_tiles_create", &mut used);

        assert_eq!(first, "nunchi_desktop_tiles_create");
        assert_eq!(second, "nunchi_desktop_tiles_create_2");
    }

    #[test]
    fn mcp_tool_name_suffix_preserves_openai_length_limit() {
        let mut used = HashSet::new();
        let base = "a".repeat(64);

        let first = unique_tool_name(&base, &mut used);
        let second = unique_tool_name(&base, &mut used);

        assert_eq!(first.len(), 64);
        assert_eq!(second.len(), 64);
        assert!(second.ends_with("_2"));
    }
}
