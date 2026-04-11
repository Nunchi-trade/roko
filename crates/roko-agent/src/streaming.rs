//! Typed streaming events for provider adapters and tool loops.

use crate::chat_types::{ChatResponse, FinishReason};
use crate::translate::{normalize_finish_reason, openai::parse_usage};
use crate::usage::Usage;
use roko_core::tool::ToolCall;
use serde_json::Value;

/// Incremental stream events normalized across GLM and Kimi responses.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Incremental reasoning or thinking text.
    ReasoningDelta(String),
    /// Incremental assistant-visible content text.
    ContentDelta(String),
    /// Incremental function-call data for one tool call slot.
    ToolCallDelta {
        /// Zero-based tool call index within the current assistant turn.
        index: usize,
        /// Incremental tool call identifier fragment, if the provider streamed one.
        id_delta: Option<String>,
        /// Incremental function name fragment, if the provider streamed one.
        name_delta: Option<String>,
        /// Incremental JSON argument text for the tool call.
        arguments_delta: String,
    },
    /// Token accounting emitted during or after the stream.
    Usage(Usage),
    /// Terminal stream marker with the canonical finish reason.
    Done(FinishReason),
    /// Terminal provider or transport error surfaced as a stream event.
    Error(String),
}

/// Incrementally reconstruct a canonical [`ChatResponse`] from stream chunks.
#[derive(Debug, Clone, Default)]
pub struct StreamAccumulator {
    reasoning: String,
    content: String,
    tool_calls: Vec<PartialToolCall>,
    usage: Usage,
    finish_reason: FinishReason,
}

#[derive(Debug, Clone, Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl StreamAccumulator {
    /// Create an empty accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Incorporate one streamed chunk into the in-progress response.
    pub fn push(&mut self, chunk: StreamChunk) {
        match chunk {
            StreamChunk::ReasoningDelta(delta) => self.reasoning.push_str(&delta),
            StreamChunk::ContentDelta(delta) => self.content.push_str(&delta),
            StreamChunk::ToolCallDelta {
                index,
                id_delta,
                name_delta,
                arguments_delta,
            } => {
                while self.tool_calls.len() <= index {
                    self.tool_calls.push(PartialToolCall::default());
                }

                let tool_call = &mut self.tool_calls[index];
                if let Some(id) = id_delta {
                    tool_call.id = id;
                }
                if let Some(name) = name_delta {
                    tool_call.name = name;
                }
                tool_call.arguments.push_str(&arguments_delta);
            }
            StreamChunk::Usage(usage) => self.usage = usage,
            StreamChunk::Done(finish_reason) => self.finish_reason = finish_reason,
            StreamChunk::Error(_) => {}
        }
    }

    /// Convert the accumulated stream state into a canonical response.
    #[must_use]
    pub fn finalize(self) -> ChatResponse {
        let tool_calls = self
            .tool_calls
            .into_iter()
            .filter(|tool_call| {
                !(tool_call.id.is_empty()
                    && tool_call.name.is_empty()
                    && tool_call.arguments.trim().is_empty())
            })
            .map(|tool_call| {
                let arguments = if tool_call.arguments.trim().is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str(&tool_call.arguments)
                        .unwrap_or_else(|_| Value::String(tool_call.arguments))
                };

                ToolCall::new(tool_call.id, tool_call.name, arguments)
            })
            .collect();

        let mut response = ChatResponse {
            content: self.content,
            reasoning: (!self.reasoning.is_empty()).then_some(self.reasoning),
            tool_calls,
            usage: self.usage,
            finish_reason: self.finish_reason,
            ..Default::default()
        };
        response.raw_assistant_message = Some(response.as_assistant_message());
        response
    }
}

/// Parse a single OpenAI-compatible SSE line into a canonical stream chunk.
#[must_use]
pub fn parse_sse_line(line: &str) -> Option<StreamChunk> {
    let line = line.strip_prefix("data:")?.trim_start();
    if line == "[DONE]" {
        return Some(StreamChunk::Done(FinishReason::Stop));
    }

    let json: Value = serde_json::from_str(line).ok()?;
    let delta = json.pointer("/choices/0/delta").unwrap_or(&Value::Null);

    // GLM streams reasoning before content, so surface that first.
    if let Some(reasoning) = delta.get("reasoning_content").and_then(Value::as_str) {
        return Some(StreamChunk::ReasoningDelta(reasoning.to_string()));
    }
    if let Some(content) = delta.get("content").and_then(Value::as_str) {
        return Some(StreamChunk::ContentDelta(content.to_string()));
    }
    if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
        for tc in tool_calls {
            let index = tc
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(0);
            return Some(StreamChunk::ToolCallDelta {
                index,
                id_delta: tc.get("id").and_then(Value::as_str).map(str::to_string),
                name_delta: tc
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                arguments_delta: tc
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    if json.get("usage").is_some() {
        return Some(StreamChunk::Usage(parse_usage(&json)));
    }
    if let Some(reason) = json
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
    {
        return Some(StreamChunk::Done(normalize_finish_reason(reason)));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{StreamChunk, parse_sse_line};
    use crate::chat_types::FinishReason;

    #[test]
    fn sse_parser_reads_reasoning_delta() {
        let chunk = parse_sse_line(
            r#"data: {"choices":[{"delta":{"reasoning_content":"Need to inspect the file."}}]}"#,
        );

        assert!(matches!(
            chunk,
            Some(StreamChunk::ReasoningDelta(reasoning)) if reasoning == "Need to inspect the file."
        ));
    }

    #[test]
    fn sse_parser_reads_content_delta() {
        let chunk =
            parse_sse_line(r#"data: {"choices":[{"delta":{"content":"I can answer now."}}]}"#);

        assert!(matches!(
            chunk,
            Some(StreamChunk::ContentDelta(content)) if content == "I can answer now."
        ));
    }

    #[test]
    fn sse_parser_reads_tool_call_delta() {
        let chunk = parse_sse_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_glm_","function":{"name":"edit_file","arguments":"{\"path\":\"note.txt\"}"}}]}}]}"#,
        );

        assert!(matches!(
            chunk,
            Some(StreamChunk::ToolCallDelta {
                index: 1,
                id_delta: Some(id),
                name_delta: Some(name),
                arguments_delta,
            }) if id == "call_glm_"
                && name == "edit_file"
                && arguments_delta == "{\"path\":\"note.txt\"}"
        ));
    }

    #[test]
    fn sse_parser_reads_usage_chunk() {
        let chunk = parse_sse_line(
            r#"data: {"choices":[],"usage":{"prompt_tokens":21,"completion_tokens":9,"prompt_tokens_details":{"cached_tokens":4}}}"#,
        );

        assert!(matches!(
            chunk,
            Some(StreamChunk::Usage(usage))
                if usage.input_tokens == 21
                    && usage.output_tokens == 9
                    && usage.cache_read_tokens == 4
        ));
    }

    #[test]
    fn sse_parser_reads_finish_reason_chunk() {
        let chunk =
            parse_sse_line(r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#);

        assert!(matches!(
            chunk,
            Some(StreamChunk::Done(FinishReason::ToolCalls))
        ));
    }

    #[test]
    fn sse_parser_reads_done_marker() {
        let chunk = parse_sse_line("data: [DONE]");

        assert!(matches!(chunk, Some(StreamChunk::Done(FinishReason::Stop))));
    }

    #[test]
    fn sse_parser_ignores_non_data_lines() {
        assert!(parse_sse_line("event: message").is_none());
    }
}
