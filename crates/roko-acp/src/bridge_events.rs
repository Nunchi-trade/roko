//! Cognitive event to session/update streaming.

use std::time::Duration;

use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc,
};
use tracing::{debug, warn};

use crate::{
    session::{AcpSession, CancelToken},
    transport::{StdioTransport, TransportError, TransportResult},
    types::{
        ContentBlock, JsonRpcMessage, PlanEntry, PlanStatus, Priority, SESSION_BUSY,
        SessionCancelParams, SessionPromptParams, SessionPromptResult, SessionUpdate, StopReason,
        ToolCallKind, ToolCallStatus, UsageInfo,
    },
};

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

/// Events emitted by the cognitive loop and mapped to ACP session updates.
#[derive(Debug, Clone)]
pub enum CognitiveEvent {
    /// A streamed agent-visible text chunk.
    TokenChunk(String),
    /// A streamed internal reasoning chunk.
    ThinkingChunk(String),
    /// A tool call has started running.
    ToolCallStart {
        /// Stable tool call identifier.
        tool_call_id: String,
        /// User-facing tool title.
        title: String,
        /// ACP tool call category.
        kind: ToolCallKind,
    },
    /// A tool call has finished with rendered content.
    ToolCallComplete {
        /// Stable tool call identifier.
        tool_call_id: String,
        /// Final tool status.
        status: ToolCallStatus,
        /// Rendered tool output blocks.
        content: Vec<ContentBlock>,
    },
    /// A gate has started.
    GateStarted {
        /// Gate display name.
        gate_name: String,
        /// Tool card identifier used for the gate.
        tool_call_id: String,
    },
    /// A gate has completed and produced a summary.
    GateCompleted {
        /// Gate display name.
        gate_name: String,
        /// Tool card identifier used for the gate.
        tool_call_id: String,
        /// Whether the gate passed.
        passed: bool,
        /// Markdown/plaintext summary for the UI.
        summary: String,
        /// Gate runtime in milliseconds.
        duration_ms: u64,
    },
    /// The plan execution phase has changed.
    PhaseTransition {
        /// New phase identifier.
        phase: String,
        /// Plan entries for the updated phase.
        entries: Vec<PlanEntry>,
    },
    /// A conductor watcher fired an action.
    WatcherTriggered {
        /// Watcher display name.
        watcher_name: String,
        /// Action taken by the watcher.
        action: String,
    },
    /// Prompt execution completed normally.
    Complete {
        /// Final stop reason for the prompt.
        stop_reason: StopReason,
        /// Optional token usage payload.
        usage: Option<UsageInfo>,
    },
    /// Prompt execution stopped because the token budget was exhausted.
    MaxTokens,
}

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

/// Handles a `session/prompt` request by running the cognitive task and streaming updates.
pub async fn handle_session_prompt<R, W>(
    transport: &mut StdioTransport<R, W>,
    session: &mut AcpSession,
    params: SessionPromptParams,
) -> Result<SessionPromptResult>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if session.is_busy() {
        return Err(BridgeEventsError::SessionBusy(session.session_id.clone()));
    }

    session.begin_prompt();

    let outcome = handle_session_prompt_inner(transport, session, params).await;
    session.finish_prompt();
    outcome
}

async fn handle_session_prompt_inner<R, W>(
    transport: &mut StdioTransport<R, W>,
    session: &mut AcpSession,
    params: SessionPromptParams,
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
        "handling ACP session prompt"
    );

    let (event_sender, event_receiver) = mpsc::channel(16);
    let cancel_token = session.cancel_token.clone();
    let session_id = session.session_id.clone();

    let cognitive_task = tokio::spawn(async move {
        run_placeholder_cognitive_task(&session_id, &prompt_text, cancel_token, event_sender).await
    });

    let stream_result = stream_events_to_editor(
        transport,
        &session.session_id,
        event_receiver,
        &session.cancel_token,
    )
    .await;
    let task_result = cognitive_task.await?;
    task_result?;

    stream_result
}

async fn run_placeholder_cognitive_task(
    session_id: &str,
    prompt_text: &str,
    cancel_token: CancelToken,
    event_sender: mpsc::Sender<CognitiveEvent>,
) -> Result<()> {
    debug!(
        session_id,
        prompt_chars = prompt_text.chars().count(),
        "running placeholder ACP cognitive task"
    );

    if cancel_token.is_cancelled() {
        return Ok(());
    }

    if event_sender
        .send(CognitiveEvent::TokenChunk("Processing...".to_owned()))
        .await
        .is_err()
    {
        return Ok(());
    }

    if cancel_token.is_cancelled() {
        return Ok(());
    }

    tokio::select! {
        _ = cancel_token.cancelled() => return Ok(()),
        _ = tokio::time::sleep(Duration::from_millis(50)) => {}
    }

    if cancel_token.is_cancelled() {
        return Ok(());
    }

    let _ = event_sender
        .send(CognitiveEvent::Complete {
            stop_reason: StopReason::EndTurn,
            usage: Some(UsageInfo {
                total_tokens: 16,
                input_tokens: 6,
                output_tokens: 10,
                thought_tokens: None,
                cached_read_tokens: None,
                cached_write_tokens: None,
            }),
        })
        .await;

    Ok(())
}

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
        CognitiveEvent::GateStarted {
            gate_name,
            tool_call_id,
        } => SessionUpdate::ToolCall {
            tool_call_id,
            title: format!("Gate: {gate_name}"),
            kind: ToolCallKind::Other,
            status: ToolCallStatus::InProgress,
            content: vec![text_block(format!("Running gate `{gate_name}`."))],
        },
        CognitiveEvent::GateCompleted {
            gate_name,
            tool_call_id,
            passed,
            summary,
            duration_ms,
        } => SessionUpdate::ToolCallUpdate {
            tool_call_id,
            status: ToolCallStatus::Completed,
            content: vec![text_block(format_gate_summary(
                &gate_name,
                passed,
                &summary,
                duration_ms,
            ))],
        },
        CognitiveEvent::PhaseTransition { phase, entries } => SessionUpdate::Plan {
            entries: if entries.is_empty() {
                vec![PlanEntry {
                    content: format!("Entered phase `{phase}`"),
                    priority: Priority::Medium,
                    status: PlanStatus::InProgress,
                }]
            } else {
                entries
            },
        },
        CognitiveEvent::WatcherTriggered {
            watcher_name,
            action,
        } => SessionUpdate::ToolCall {
            tool_call_id: watcher_tool_call_id(&watcher_name, &action),
            title: format!("Watcher: {watcher_name}"),
            kind: ToolCallKind::Other,
            status: ToolCallStatus::Completed,
            content: vec![text_block(format!("Watcher action: {action}"))],
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

fn format_gate_summary(
    gate_name: &str,
    passed: bool,
    summary: &str,
    duration_ms: u64,
) -> String {
    let verdict = if passed { "passed" } else { "failed" };
    format!("Gate `{gate_name}` {verdict} in {duration_ms} ms.\n\n{summary}")
}

fn watcher_tool_call_id(watcher_name: &str, action: &str) -> String {
    format!(
        "watcher_{}_{}",
        slugify(watcher_name),
        slugify(action),
    )
}

fn slugify(value: &str) -> String {
    let slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();

    let trimmed = slug.trim_matches('_');
    if trimmed.is_empty() {
        "event".to_owned()
    } else {
        trimmed.to_owned()
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

        assert_eq!(result.session_id, "sess_test");
        assert_eq!(result.stop_reason, StopReason::EndTurn);
        assert_eq!(
            result.usage,
            Some(UsageInfo {
                total_tokens: 12,
                input_tokens: 5,
                output_tokens: 7,
                thought_tokens: None,
                cached_read_tokens: None,
                cached_write_tokens: None,
            })
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
}
