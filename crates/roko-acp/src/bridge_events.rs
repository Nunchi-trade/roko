//! Cognitive event to session/update streaming.
//!
//! This module bridges the `claude` CLI subprocess (running with
//! `--output-format stream-json`) to ACP `session/update` notifications.

use std::path::Path;

use serde::Deserialize;
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _},
    sync::mpsc,
};
use tracing::{debug, error, warn};

use crate::{
    session::{AcpSession, CancelToken},
    transport::{StdioTransport, TransportError, TransportResult},
    types::{
        ContentBlock, JsonRpcMessage, SESSION_BUSY,
        SessionCancelParams, SessionPromptParams, SessionPromptResult, SessionUpdate, StopReason,
        ToolCallKind, ToolCallStatus, UsageInfo,
    },
};

// ── Claude CLI stream-json wire types (inlined from roko-agent) ──────

/// Top-level stream event from `claude --output-format stream-json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeStreamEvent {
    System(ClaudeSystemEvent),
    Assistant(ClaudeAssistantEvent),
    Tool(ClaudeToolEvent),
    Result(ClaudeResultEvent),
}

#[derive(Debug, Clone, Deserialize)]
struct ClaudeSystemEvent {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ClaudeAssistantEvent {
    pub message: ClaudeMessage,
}

#[derive(Debug, Clone, Deserialize)]
struct ClaudeMessage {
    #[serde(default)]
    pub content: Vec<ClaudeContentBlock>,
    #[serde(default)]
    pub usage: Option<ClaudeUsage>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String },
    Thinking { thinking: String },
}

#[derive(Debug, Clone, Deserialize)]
struct ClaudeToolEvent {
    #[serde(default, rename = "tool_name")]
    pub _tool_name: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ClaudeResultEvent {
    #[serde(default)]
    pub total_cost_usd: Option<f64>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub usage: Option<ClaudeUsage>,
}

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
            Self::Serialize(_) | Self::Transport(_) | Self::TaskJoin(_) => None,
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
    /// Prompt execution completed normally.
    Complete {
        stop_reason: StopReason,
        usage: Option<UsageInfo>,
    },
    /// Prompt execution stopped because the token budget was exhausted.
    MaxTokens,
}

// ── Stream events → editor ───────────────────────────────────────────

/// Maps cognitive events to ACP `session/update` notifications and streams them to the editor.
pub async fn stream_events_to_editor<R, W>(
    transport: &mut StdioTransport<R, W>,
    session_id: &str,
    mut events: mpsc::Receiver<CognitiveEvent>,
    cancel_token: &CancelToken,
) -> Result<SessionPromptResult>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
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
                return Ok(SessionPromptResult {
                    session_id: session_id.to_owned(),
                    stop_reason: StopReason::Cancelled,
                    usage: None,
                });
            }
            StreamAction::Event(maybe_event) => {
                let Some(event) = maybe_event else {
                    warn!(session_id, "ACP event stream closed without an explicit completion event");
                    let stop_reason = if cancel_token.is_cancelled() {
                        StopReason::Cancelled
                    } else {
                        StopReason::Error
                    };
                    return Ok(SessionPromptResult {
                        session_id: session_id.to_owned(),
                        stop_reason,
                        usage: None,
                    });
                };

                match event {
                    CognitiveEvent::Complete { stop_reason, usage } => {
                        return Ok(SessionPromptResult {
                            session_id: session_id.to_owned(),
                            stop_reason,
                            usage,
                        });
                    }
                    CognitiveEvent::MaxTokens => {
                        return Ok(SessionPromptResult {
                            session_id: session_id.to_owned(),
                            stop_reason: StopReason::MaxTokens,
                            usage: None,
                        });
                    }
                    other => {
                        let update = map_event_to_update(other);
                        send_session_update(transport, update).await?;
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
                    warn!(session_id, "ACP client disconnected while prompt was active");
                    return Ok(SessionPromptResult {
                        session_id: session_id.to_owned(),
                        stop_reason: StopReason::Cancelled,
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
) -> Result<SessionPromptResult>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if session.is_busy() {
        return Err(BridgeEventsError::SessionBusy(session.session_id.clone()));
    }

    session.begin_prompt();

    let outcome = handle_session_prompt_inner(transport, session, params, workdir).await;
    session.finish_prompt();
    outcome
}

async fn handle_session_prompt_inner<R, W>(
    transport: &mut StdioTransport<R, W>,
    session: &mut AcpSession,
    params: SessionPromptParams,
    workdir: &Path,
) -> Result<SessionPromptResult>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let prompt_text = extract_prompt_text(&params.prompt);
    debug!(
        session_id = %session.session_id,
        prompt_blocks = params.prompt.len(),
        prompt_chars = prompt_text.chars().count(),
        include_context = params.include_context,
        workdir = %workdir.display(),
        "handling ACP session prompt"
    );

    let (event_sender, event_receiver) = mpsc::channel(64);
    let cancel_token = session.cancel_token.clone();
    let session_id = session.session_id.clone();
    let workdir = workdir.to_path_buf();

    let cognitive_task = tokio::spawn(async move {
        run_claude_cognitive_task(&session_id, &prompt_text, &workdir, cancel_token, event_sender)
            .await
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
        error!(error = %e, "Claude cognitive task failed");
    }

    stream_result
}

// ── Real Claude CLI dispatch ─────────────────────────────────────────

/// Spawns `claude --print --output-format stream-json --verbose` as a
/// subprocess, pipes the prompt to stdin, and streams parsed events back
/// through the `event_sender` channel.
async fn run_claude_cognitive_task(
    session_id: &str,
    prompt_text: &str,
    workdir: &Path,
    cancel_token: CancelToken,
    event_sender: mpsc::Sender<CognitiveEvent>,
) -> Result<()> {
    debug!(
        session_id,
        prompt_chars = prompt_text.chars().count(),
        workdir = %workdir.display(),
        "spawning claude CLI for ACP cognitive task"
    );

    if cancel_token.is_cancelled() {
        return Ok(());
    }

    let mut child = match tokio::process::Command::new("claude")
        .arg("--print")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .current_dir(workdir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            error!(error = %e, "failed to spawn claude CLI");
            let _ = event_sender
                .send(CognitiveEvent::TokenChunk(format!(
                    "Error: failed to spawn `claude` CLI: {e}"
                )))
                .await;
            let _ = event_sender
                .send(CognitiveEvent::Complete {
                    stop_reason: StopReason::Error,
                    usage: None,
                })
                .await;
            return Ok(());
        }
    };

    // Write prompt to stdin and close it.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt_text.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }

    // Accumulate usage across the stream.
    let mut total_input = 0u64;
    let mut total_output = 0u64;
    let mut total_cache_read = 0u64;
    let mut total_cache_write = 0u64;
    let mut final_cost: Option<f64> = None;
    let mut is_error = false;

    // Read stdout line-by-line.
    let stdout = child.stdout.take().expect("stdout was piped");
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut line = String::new();

    loop {
        if cancel_token.is_cancelled() {
            let _ = child.kill().await;
            return Ok(());
        }

        line.clear();

        let read_result = tokio::select! {
            biased;
            _ = cancel_token.cancelled() => {
                let _ = child.kill().await;
                return Ok(());
            }
            result = reader.read_line(&mut line) => result,
        };

        match read_result {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => {
                warn!(session_id, error = %e, "error reading claude stdout");
                break;
            }
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let event: ClaudeStreamEvent = match serde_json::from_str(trimmed) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let cognitive_events = match event {
            ClaudeStreamEvent::System(sys) => {
                debug!(
                    session_id,
                    claude_session = %sys.session_id,
                    model = %sys.model,
                    "claude CLI system init"
                );
                Vec::new()
            }
            ClaudeStreamEvent::Assistant(asst) => {
                let mut events = Vec::new();
                for block in &asst.message.content {
                    match block {
                        ClaudeContentBlock::Text { text } => {
                            events.push(CognitiveEvent::TokenChunk(text.clone()));
                        }
                        ClaudeContentBlock::ToolUse { id, name } => {
                            events.push(CognitiveEvent::ToolCallStart {
                                tool_call_id: id.clone(),
                                title: name.clone(),
                                kind: tool_name_to_kind(name),
                            });
                        }
                        ClaudeContentBlock::Thinking { thinking } => {
                            events.push(CognitiveEvent::ThinkingChunk(thinking.clone()));
                        }
                    }
                }
                if let Some(usage) = &asst.message.usage {
                    total_input = total_input.max(usage.input_tokens);
                    total_output = total_output.max(usage.output_tokens);
                    total_cache_read = total_cache_read.max(usage.cache_read_input_tokens);
                    total_cache_write = total_cache_write.max(usage.cache_creation_input_tokens);
                }
                events
            }
            ClaudeStreamEvent::Tool(tool) => {
                let truncated = if tool.content.len() > 4096 {
                    format!("{}... [truncated]", &tool.content[..4096])
                } else {
                    tool.content
                };
                vec![CognitiveEvent::ToolCallComplete {
                    tool_call_id: tool.tool_use_id,
                    status: ToolCallStatus::Completed,
                    content: vec![ContentBlock::Text { text: truncated }],
                }]
            }
            ClaudeStreamEvent::Result(res) => {
                if let Some(usage) = &res.usage {
                    total_input = total_input.max(usage.input_tokens);
                    total_output = total_output.max(usage.output_tokens);
                    total_cache_read = total_cache_read.max(usage.cache_read_input_tokens);
                    total_cache_write = total_cache_write.max(usage.cache_creation_input_tokens);
                }
                final_cost = res.total_cost_usd;
                is_error = res.is_error;
                Vec::new() // we'll emit Complete after the loop
            }
        };

        for ce in cognitive_events {
            if event_sender.send(ce).await.is_err() {
                let _ = child.kill().await;
                return Ok(());
            }
        }
    }

    // Wait for process to exit.
    let status = child.wait().await;
    debug!(session_id, ?status, cost = ?final_cost, "claude CLI process exited");

    let stop_reason = if is_error {
        StopReason::Error
    } else {
        StopReason::EndTurn
    };

    let usage = if total_input > 0 || total_output > 0 {
        Some(UsageInfo {
            total_tokens: total_input + total_output,
            input_tokens: total_input,
            output_tokens: total_output,
            thought_tokens: None,
            cached_read_tokens: if total_cache_read > 0 {
                Some(total_cache_read)
            } else {
                None
            },
            cached_write_tokens: if total_cache_write > 0 {
                Some(total_cache_write)
            } else {
                None
            },
        })
    } else {
        None
    };

    let _ = event_sender
        .send(CognitiveEvent::Complete { stop_reason, usage })
        .await;

    Ok(())
}

/// Maps a Claude tool name to an ACP tool call kind.
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
        CognitiveEvent::ThinkingChunk(text) => SessionUpdate::ThoughtMessageChunk {
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
        CognitiveEvent::Complete { .. } | CognitiveEvent::MaxTokens => {
            unreachable!("terminal cognitive events are handled before update mapping")
        }
    }
}

async fn send_session_update<R, W>(
    transport: &mut StdioTransport<R, W>,
    update: SessionUpdate,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let params = serde_json::to_value(update)?;
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
            ContentBlock::Resource { resource } => format!("resource: {resource:?}"),
            ContentBlock::Diff { path, diff } => format!("diff {path}:\n{diff}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
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

        assert_eq!(result.session_id, "sess_test");
        assert_eq!(result.stop_reason, StopReason::EndTurn);

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
                "sessionUpdate": "agent_message_chunk",
                "content": {
                    "type": "text",
                    "text": "hello"
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

        assert_eq!(result.session_id, "sess_cancel");
        assert_eq!(result.stop_reason, StopReason::Cancelled);
        assert_eq!(result.usage, None);
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
}
