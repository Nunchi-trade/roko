//! Claude CLI `stream-json` parser.
//!
//! The Claude CLI emits JSON-Lines on stdout when invoked with
//! `--output-format stream-json`. This module owns the protocol-specific
//! deserialization types and a [`parse_stream_line`] adapter that translates
//! each line into one or more provider-neutral [`AgentRuntimeEvent`]s.
//!
//! Keeping the wire types here means runner-side code never has to know about
//! the Claude protocol — it consumes [`AgentRuntimeEvent`] from any provider
//! adapter that produces them.

use serde::Deserialize;
use tracing::debug;

use crate::runtime_events::AgentRuntimeEvent;

/// Top-level stream event from `claude --output-format stream-json`.
///
/// Re-exported from this submodule path
/// (`roko_agent::provider::claude_cli::ClaudeStreamEvent`) for callers that
/// want to inspect the raw protocol shape directly.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClaudeStreamEvent {
    System(ClaudeSystemEvent),
    Assistant(ClaudeAssistantEvent),
    Tool(ClaudeToolEvent),
    Result(ClaudeResultEvent),
}

/// The `system` init event (handshake announcing session id and model).
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeSystemEvent {
    #[serde(default)]
    pub subtype: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub tools: Vec<serde_json::Value>,
}

/// An assistant message event carrying one or more content blocks plus
/// optional usage accounting.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeAssistantEvent {
    #[serde(default)]
    pub subtype: String,
    pub message: ClaudeMessage,
}

/// The message body inside an assistant event.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeMessage {
    #[serde(default)]
    pub content: Vec<ClaudeContentBlock>,
    #[serde(default)]
    pub usage: Option<ClaudeUsage>,
}

/// One assistant content block — either visible text or a tool invocation.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClaudeContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
}

/// A `tool` event carrying a tool result payload.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeToolEvent {
    #[serde(default)]
    pub subtype: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub content: String,
}

/// The terminal `result` event, emitted after the run finishes.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeResultEvent {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub total_cost_usd: Option<f64>,
    #[serde(default)]
    pub num_turns: Option<u32>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub duration_ms: Option<f64>,
    #[serde(default)]
    pub duration_api_ms: Option<f64>,
    /// Final cumulative usage for the session.
    #[serde(default)]
    pub usage: Option<ClaudeUsage>,
}

/// Token usage block as reported by the Claude CLI protocol.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

/// Limit on tool output bytes propagated through the runner; longer payloads
/// are truncated with an ellipsis marker so downstream buffers stay bounded.
const TOOL_OUTPUT_TRUNCATE_AT: usize = 4096;

/// Parse a single line of `--output-format stream-json` into zero or more
/// provider-neutral [`AgentRuntimeEvent`]s.
///
/// Returns an empty vec for empty lines or unparseable content. May return
/// multiple events when one wire-level message carries both content and usage
/// (e.g. an assistant message with both text and a usage block).
#[must_use]
pub fn parse_stream_line(line: &str) -> Vec<AgentRuntimeEvent> {
    let line = line.trim();
    if line.is_empty() {
        return Vec::new();
    }

    let event: ClaudeStreamEvent = match serde_json::from_str(line) {
        Ok(e) => e,
        Err(e) => {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                return parse_generic_json_line(&value);
            }
            debug!(line_len = line.len(), err = %e, "ignoring unparseable stream line");
            return Vec::new();
        }
    };

    match event {
        ClaudeStreamEvent::System(sys) => vec![AgentRuntimeEvent::SystemInit {
            session_id: sys.session_id,
            model: sys.model,
        }],

        ClaudeStreamEvent::Assistant(asst) => {
            // An assistant event can carry content blocks AND usage in the
            // same message — emit content first, usage second.
            let mut events = Vec::new();

            for block in &asst.message.content {
                match block {
                    ClaudeContentBlock::Text { text } => {
                        events.push(AgentRuntimeEvent::MessageDelta { text: text.clone() });
                    }
                    ClaudeContentBlock::ToolUse { id, name, .. } => {
                        events.push(AgentRuntimeEvent::ToolCall {
                            id: id.clone(),
                            name: name.clone(),
                        });
                    }
                }
            }

            if let Some(usage) = &asst.message.usage {
                events.push(AgentRuntimeEvent::TokenUsage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cache_read_tokens: usage.cache_read_input_tokens,
                    cache_write_tokens: usage.cache_creation_input_tokens,
                });
            }

            events
        }

        ClaudeStreamEvent::Tool(tool) => {
            let output = if tool.content.len() > TOOL_OUTPUT_TRUNCATE_AT {
                format!(
                    "{}\u{2026} [truncated]",
                    &tool.content[..TOOL_OUTPUT_TRUNCATE_AT]
                )
            } else {
                tool.content
            };
            vec![AgentRuntimeEvent::ToolOutput {
                id: tool.tool_use_id,
                output,
            }]
        }

        ClaudeStreamEvent::Result(res) => {
            let mut events = vec![AgentRuntimeEvent::TurnCompleted {
                session_id: Some(res.session_id).filter(|s| !s.is_empty()),
                total_cost_usd: res.total_cost_usd,
                num_turns: res.num_turns,
                is_error: res.is_error,
            }];
            if let Some(usage) = &res.usage {
                events.push(AgentRuntimeEvent::TokenUsage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cache_read_tokens: usage.cache_read_input_tokens,
                    cache_write_tokens: usage.cache_creation_input_tokens,
                });
            }
            events
        }
    }
}

/// Best-effort fallback for lines that decode as generic JSON but do not
/// match the `stream-json` envelope. Used to surface error or message-shaped
/// events emitted by older or non-Claude CLI tools.
fn parse_generic_json_line(value: &serde_json::Value) -> Vec<AgentRuntimeEvent> {
    let event_type = value
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if event_type.contains("error") {
        let message = value
            .get("message")
            .or_else(|| value.get("error"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("agent emitted an error event");
        return vec![AgentRuntimeEvent::Error {
            message: message.to_string(),
        }];
    }

    if event_type.contains("message") || event_type.contains("output") {
        for key in ["text", "message", "content", "delta"] {
            if let Some(text) = value.get(key).and_then(serde_json::Value::as_str) {
                return vec![AgentRuntimeEvent::MessageDelta {
                    text: text.to_string(),
                }];
            }
        }
    }

    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_system_event() {
        let line = r#"{"type":"system","subtype":"init","session_id":"abc-123","model":"claude-sonnet-4-6","tools":[]}"#;
        let event = parse_stream_line(line).into_iter().next().unwrap();
        match event {
            AgentRuntimeEvent::SystemInit { session_id, model } => {
                assert_eq!(session_id, "abc-123");
                assert_eq!(model, "claude-sonnet-4-6");
            }
            _ => panic!("expected SystemInit"),
        }
    }

    #[test]
    fn parse_assistant_text() {
        let line = r#"{"type":"assistant","subtype":"message","message":{"content":[{"type":"text","text":"hello world"}],"usage":null}}"#;
        let event = parse_stream_line(line).into_iter().next().unwrap();
        match event {
            AgentRuntimeEvent::MessageDelta { text } => {
                assert_eq!(text, "hello world");
            }
            _ => panic!("expected MessageDelta"),
        }
    }

    #[test]
    fn parse_assistant_tool_use() {
        let line = r#"{"type":"assistant","subtype":"message","message":{"content":[{"type":"tool_use","id":"tu_1","name":"Read","input":{"path":"foo"}}],"usage":null}}"#;
        let event = parse_stream_line(line).into_iter().next().unwrap();
        match event {
            AgentRuntimeEvent::ToolCall { id, name } => {
                assert_eq!(id, "tu_1");
                assert_eq!(name, "Read");
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn parse_tool_event() {
        let line = r#"{"type":"tool","subtype":"result","tool_name":"Bash","tool_use_id":"tu_2","content":"output here"}"#;
        let event = parse_stream_line(line).into_iter().next().unwrap();
        match event {
            AgentRuntimeEvent::ToolOutput { id, output } => {
                assert_eq!(id, "tu_2");
                assert_eq!(output, "output here");
            }
            _ => panic!("expected ToolOutput"),
        }
    }

    #[test]
    fn parse_result_event() {
        let line = r#"{"type":"result","session_id":"sess-1","total_cost_usd":0.05,"num_turns":3,"is_error":false}"#;
        let event = parse_stream_line(line).into_iter().next().unwrap();
        match event {
            AgentRuntimeEvent::TurnCompleted {
                session_id,
                total_cost_usd,
                num_turns,
                is_error,
            } => {
                assert_eq!(session_id.unwrap(), "sess-1");
                assert!((total_cost_usd.unwrap() - 0.05).abs() < f64::EPSILON);
                assert_eq!(num_turns.unwrap(), 3);
                assert!(!is_error);
            }
            _ => panic!("expected TurnCompleted"),
        }
    }

    #[test]
    fn parse_empty_line() {
        assert!(parse_stream_line("").is_empty());
        assert!(parse_stream_line("   ").is_empty());
    }

    #[test]
    fn parse_malformed_json() {
        assert!(parse_stream_line("{not json}").is_empty());
    }

    #[test]
    fn tool_output_truncation() {
        let long_content = "x".repeat(5000);
        let line = format!(
            r#"{{"type":"tool","subtype":"result","tool_name":"Bash","tool_use_id":"tu_3","content":"{long_content}"}}"#
        );
        let event = parse_stream_line(&line).into_iter().next().unwrap();
        match event {
            AgentRuntimeEvent::ToolOutput { output, .. } => {
                assert!(output.len() < 5000);
                assert!(output.ends_with("\u{2026} [truncated]"));
            }
            _ => panic!("expected ToolOutput"),
        }
    }

    #[test]
    fn assistant_message_emits_text_then_usage() {
        let line = r#"{"type":"assistant","subtype":"message","message":{"content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":3,"output_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}"#;
        let events = parse_stream_line(line);
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0],
            AgentRuntimeEvent::MessageDelta { ref text } if text == "hi"
        ));
        assert!(matches!(
            events[1],
            AgentRuntimeEvent::TokenUsage {
                input_tokens: 3,
                output_tokens: 1,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            }
        ));
    }

    #[test]
    fn generic_error_line_maps_to_error_event() {
        let line = r#"{"type":"error","message":"rate limited"}"#;
        let events = parse_stream_line(line);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentRuntimeEvent::Error { message } => assert_eq!(message, "rate limited"),
            _ => panic!("expected Error"),
        }
    }
}
