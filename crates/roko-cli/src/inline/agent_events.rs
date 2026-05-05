//! Reusable agent event stream adapter.
//!
//! Wraps [`crate::tui::ws_client::AgentStreamClient`] into a typed async
//! channel that any consumer can use — chat, run, plan execution, dashboard.
//!
//! This is the single integration point between the agent backend and the
//! inline rendering engine.

use serde_json::Value;
use tokio::sync::mpsc;

use crate::tui::ws_client::{AgentStreamClient, StreamChunk};

/// Typed event from an agent stream, suitable for driving any UI.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Connection established.
    Connected,
    /// Text content delta (append to streaming buffer).
    TextDelta(String),
    /// Reasoning / thinking content (may be rendered differently).
    ReasoningDelta(String),
    /// Tool call initiated.
    ToolCallStart {
        /// Tool name (e.g. "ReadFile", "Edit", "Bash").
        name: String,
        /// Tool input arguments.
        input: Value,
    },
    /// Tool call completed.
    ToolCallDone {
        /// Tool name.
        name: String,
        /// Tool result (output text or structured data).
        result: String,
        /// Duration in seconds.
        duration_s: f64,
    },
    /// Token usage update.
    Usage {
        /// Input tokens consumed.
        input_tokens: u64,
        /// Output tokens produced.
        output_tokens: u64,
        /// Cache creation tokens.
        cache_creation_tokens: u64,
        /// Cache read tokens.
        cache_read_tokens: u64,
    },
    /// Error from the stream.
    Error(String),
    /// Agent turn complete.
    Done {
        /// Session ID if provided.
        session: Option<String>,
    },
    /// Connection lost (will reconnect).
    Disconnected,
}

/// Parse a `StreamChunk` from the WebSocket client into typed `AgentEvent`s.
///
/// This is a pure function — no I/O, no state. Consumers can call it directly
/// or use [`AgentEventStream`] for the full async channel experience.
pub fn parse_chunk(chunk: StreamChunk) -> Vec<AgentEvent> {
    match chunk {
        StreamChunk::Connected => vec![AgentEvent::Connected],
        StreamChunk::Text(text) => vec![AgentEvent::TextDelta(text)],
        StreamChunk::Reasoning(text) => vec![AgentEvent::ReasoningDelta(text)],
        StreamChunk::ToolCall(value) => parse_tool_call(value),
        StreamChunk::Usage(value) => parse_usage(value),
        StreamChunk::Error(msg) => vec![AgentEvent::Error(msg)],
        StreamChunk::Done { session } => vec![AgentEvent::Done { session }],
        StreamChunk::Disconnected => vec![AgentEvent::Disconnected],
    }
}

fn parse_tool_call(value: Value) -> Vec<AgentEvent> {
    let name = value
        .get("name")
        .or_else(|| value.get("tool"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let input = value
        .get("input")
        .or_else(|| value.get("arguments"))
        .cloned()
        .unwrap_or(Value::Null);

    vec![AgentEvent::ToolCallStart { name, input }]
}

fn parse_usage(value: Value) -> Vec<AgentEvent> {
    let get_u64 = |key: &str| -> u64 { value.get(key).and_then(Value::as_u64).unwrap_or(0) };

    vec![AgentEvent::Usage {
        input_tokens: get_u64("input_tokens"),
        output_tokens: get_u64("output_tokens"),
        cache_creation_tokens: get_u64("cache_creation_input_tokens"),
        cache_read_tokens: get_u64("cache_read_input_tokens"),
    }]
}

/// Async channel of [`AgentEvent`]s backed by an [`AgentStreamClient`].
///
/// Spawns a background task that polls the WebSocket client and forwards
/// parsed events through a `tokio::sync::mpsc` channel.
pub struct AgentEventStream {
    rx: mpsc::Receiver<AgentEvent>,
    _task: tokio::task::JoinHandle<()>,
}

/// Spawn the polling loop that drains a `AgentStreamClient` and forwards
/// parsed events to the channel.
fn spawn_poll_loop(
    mut client: AgentStreamClient,
    tx: mpsc::Sender<AgentEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match client.try_recv() {
                Ok(chunk) => {
                    let is_terminal = matches!(chunk, StreamChunk::Done { .. });
                    for event in parse_chunk(chunk) {
                        if tx.send(event).await.is_err() {
                            return;
                        }
                    }
                    if is_terminal {
                        return;
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    return;
                }
            }
        }
    })
}

impl AgentEventStream {
    /// Connect to an agent via the roko-serve event bus.
    ///
    /// Returns `None` if no tokio runtime is available.
    pub fn connect(
        agent_id: impl Into<String>,
        serve_base_url: &str,
        auth_token: Option<String>,
    ) -> Option<Self> {
        let client = AgentStreamClient::connect(agent_id, serve_base_url, auth_token)?;
        let (tx, rx) = mpsc::channel(128);
        let task = spawn_poll_loop(client, tx);
        Some(Self { rx, _task: task })
    }

    /// Connect directly to a WebSocket endpoint.
    pub fn connect_direct(endpoint: impl Into<String>) -> Option<Self> {
        let client = AgentStreamClient::connect_direct(endpoint)?;
        let (tx, rx) = mpsc::channel(128);
        let task = spawn_poll_loop(client, tx);
        Some(Self { rx, _task: task })
    }

    /// Receive the next event (async).
    pub async fn recv(&mut self) -> Option<AgentEvent> {
        self.rx.recv().await
    }

    /// Try to receive without blocking.
    pub fn try_recv(&mut self) -> Result<AgentEvent, mpsc::error::TryRecvError> {
        self.rx.try_recv()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_text_delta() {
        let events = parse_chunk(StreamChunk::Text("hello".into()));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AgentEvent::TextDelta(t) if t == "hello"));
    }

    #[test]
    fn parse_tool_call_event() {
        let value = serde_json::json!({
            "name": "ReadFile",
            "input": { "path": "src/main.rs" }
        });
        let events = parse_chunk(StreamChunk::ToolCall(value));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AgentEvent::ToolCallStart { name, .. } if name == "ReadFile"));
    }

    #[test]
    fn parse_usage_event() {
        let value = serde_json::json!({
            "input_tokens": 1000,
            "output_tokens": 500,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 200
        });
        let events = parse_chunk(StreamChunk::Usage(value));
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            AgentEvent::Usage {
                input_tokens: 1000,
                output_tokens: 500,
                cache_read_tokens: 200,
                ..
            }
        ));
    }

    #[test]
    fn parse_done_event() {
        let events = parse_chunk(StreamChunk::Done {
            session: Some("sess-1".into()),
        });
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AgentEvent::Done { session: Some(s) } if s == "sess-1"));
    }
}
