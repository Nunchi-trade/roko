//! Tool-call translators (§36.c) — the bridge between canonical Roko
//! tools and each LLM backend's wire format.
//!
//! Every backend that speaks tools implements [`Translator`]. One instance
//! per backend (Claude / Ollama / `OpenAI` / `ReAct`). The future
//! `ToolLoop` calls [`Translator::render_tools`] once
//! per turn to generate the backend-specific tool spec, then
//! [`Translator::parse_calls`] to extract what the backend emitted, and
//! [`Translator::render_results`] to feed results back on the next turn.
//!
//! # Translator design
//!
//! Translators are **sync, pure functions** of their inputs. No I/O, no
//! side effects — they simply reshape data from canonical form into the
//! backend's expected shape and back. The async bits (HTTP, subprocess)
//! belong to backend adapters that sit *above* the translator.
//!
//! # Submodules
//!
//! - [`claude`] — Claude CLI (`--tools=...` flag + stream-json `tool_use` blocks)
//! - [`gemini`] — Gemini native `functionDeclarations` / `functionCall` / `functionResponse`
//! - [`ollama`] — OpenAI-compatible JSON over `/api/chat`
//! - [`openai`] — `/v1/chat/completions` (mostly same wire as Ollama)
//! - [`react`] — prompt-level `ReAct` fallback for models without native tools
//! - [`capability`] — detect a model's capabilities + pick a translator
//!
//! # Research note
//!
//! Tool-call format preference is model-specific. Research shows 5–30
//! accuracy points on the table when using the wrong format (Meta-Harness,
//! `WildToolBench`, `Qwen3-coder` documented format switch above 5 tools).
//! This module is the enforcement point: each backend gets exactly the
//! format its profile says it prefers.

#![allow(clippy::module_name_repetitions)]

pub use crate::chat_types::{ChatResponse, FinishReason, ResponseMetadata, SessionState};
use crate::usage::Usage;
use roko_core::tool::{ToolCall, ToolDef, ToolFormat, ToolResult};

pub mod capability;
pub mod claude;
pub mod gemini;
pub mod ollama;
pub mod openai;
pub mod react;

pub use capability::{
    ModelCapabilities, capabilities_for, capabilities_from_profile, translator_for,
    translator_for_capabilities, translator_for_profile, translator_name_for,
    translator_name_for_capabilities, translator_name_for_profile,
};
pub use claude::ClaudeTranslator;
pub use gemini::GeminiTranslator;
pub use ollama::OllamaTranslator;
pub use openai::{OpenAiTranslator, StrictOpenAiTranslator};
pub use react::ReActTranslator;

/// Normalize provider-specific finish reasons into canonical [`FinishReason`] values.
#[must_use]
pub fn normalize_finish_reason(raw: &str) -> FinishReason {
    match raw {
        "stop" | "end_turn" => FinishReason::Stop,
        "length" | "max_tokens" => FinishReason::Length,
        "tool_calls" | "tool_use" => FinishReason::ToolCalls,
        "content_filter" | "sensitive" => FinishReason::ContentFilter,
        "network_error" => FinishReason::Error("network_error".into()),
        "model_context_window_exceeded" => FinishReason::Error("context_overflow".into()),
        other => FinishReason::Error(other.to_string()),
    }
}

/// Bidirectional bridge between canonical tools and a backend's wire format.
///
/// Implementors are sync and **pure**: given identical inputs they must
/// produce identical outputs, and they perform no I/O.
pub trait Translator: Send + Sync {
    /// Which wire format this translator emits/parses.
    fn format(&self) -> ToolFormat;

    /// Serialize the tool catalog into the backend's expected shape.
    ///
    /// The caller passes the output through to the HTTP body (JSON tools
    /// array), a CLI flag (`--tools=Read,Edit,Bash`), or the system
    /// prompt (`ReAct` embeds schemas directly).
    fn render_tools(&self, tools: &[ToolDef]) -> RenderedTools;

    /// Parse the backend's response into a list of canonical tool calls.
    ///
    /// Returns an empty `Vec` if the response has no tool calls (the LLM
    /// answered directly). Returns `Err(TranslatorError::Malformed)` if
    /// the response *claims* tool calls but can't be parsed.
    fn parse_calls(&self, response: &BackendResponse) -> Result<Vec<ToolCall>, TranslatorError>;

    /// Serialize tool results back into the shape the backend consumes
    /// on the next turn (typically role=`"tool"`, `tool_call_id`, content).
    fn render_results(&self, results: &[(ToolCall, ToolResult)]) -> RenderedResults;

    /// Extract the assistant message from a backend response for injection
    /// into conversation history. Returns `None` by default.
    fn render_assistant_message(&self, _response: &BackendResponse) -> Option<serde_json::Value> {
        None
    }
}

// ─── Payload enums ────────────────────────────────────────────────────────

/// Backend-specific tool-spec payload emitted by [`Translator::render_tools`].
#[derive(Debug, Clone)]
pub enum RenderedTools {
    /// JSON array for the HTTP body (`tools: [...]`). Used by Ollama,
    /// `OpenAI`, and any `OpenAI`-compatible gateway.
    JsonArray(serde_json::Value),
    /// CLI flag payload (e.g. `"Read,Edit,Bash"`). Used by Claude CLI.
    CliFlag(String),
    /// Text block to inline into the system prompt. Used by the `ReAct`
    /// fallback for models without native function calling.
    SystemPromptBlock(String),
}

/// Backend-specific tool-result payload emitted by
/// [`Translator::render_results`] for the next turn.
#[derive(Debug, Clone)]
pub enum RenderedResults {
    /// Array of tool-result messages (`OpenAI`, Ollama, compatible gateways).
    JsonMessages(serde_json::Value),
    /// Text to splice into the prompt (`ReAct`).
    TextBlock(String),
    /// No-op — the backend owns its own tool-call loop (Claude CLI,
    /// Codex MCP). Roko does not feed results back in these cases.
    HandledByBackend,
}

/// Raw backend response passed into [`Translator::parse_calls`].
///
/// Opaque to translators except via their own [`parse_calls`](Translator::parse_calls)
/// impl — one variant per wire shape the ecosystem currently uses.
#[derive(Debug, Clone)]
pub enum BackendResponse {
    /// Single JSON object (Ollama `/api/chat`, `OpenAI`
    /// `/v1/chat/completions`, Anthropic API non-streaming).
    Json(serde_json::Value),
    /// Sequence of stream-json events (Claude CLI).
    StreamJson(Vec<serde_json::Value>),
    /// Plain-text completion (`ReAct` models).
    Text(String),
}

impl BackendResponse {
    /// Extract the final assistant text (no tool calls) from this response.
    ///
    /// Used by the multi-turn loop to obtain the final answer when the
    /// backend stops calling tools. Returns the empty string if the
    /// response has no obvious text field.
    #[must_use]
    pub fn extract_text(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Json(v) => v
                .pointer("/message/content")
                .and_then(|x| x.as_str())
                .or_else(|| {
                    v.pointer("/choices/0/message/content")
                        .and_then(|x| x.as_str())
                })
                .or_else(|| extract_gemini_text(v))
                .unwrap_or("")
                .to_string(),
            Self::StreamJson(events) => {
                let mut buf = String::new();
                for ev in events {
                    let event_type = ev.get("type").and_then(|t| t.as_str());
                    match event_type {
                        // Tool events: include tool output in the response text
                        // so the caller can see what tools actually did.
                        Some("tool") => {
                            let content = ev
                                .get("content")
                                .and_then(|c| c.as_str())
                                .or_else(|| ev.get("output").and_then(|o| o.as_str()));
                            if let Some(content) = content.filter(|s| !s.is_empty()) {
                                let tool_name =
                                    ev.get("tool").and_then(|t| t.as_str()).unwrap_or("tool");
                                buf.push_str(&format!("\n[{tool_name}]\n"));
                                // Truncate very large outputs
                                if content.len()
                                    > roko_core::defaults::DEFAULT_TOOL_OUTPUT_TRUNCATE_AT
                                {
                                    let mut end =
                                        roko_core::defaults::DEFAULT_TOOL_OUTPUT_TRUNCATE_AT;
                                    while !content.is_char_boundary(end) {
                                        end -= 1;
                                    }
                                    buf.push_str(&content[..end]);
                                    buf.push_str("...[truncated]\n");
                                } else {
                                    buf.push_str(content);
                                    buf.push('\n');
                                }
                            }
                        }
                        // Assistant events: extract text content as before
                        _ => {
                            if let Some(delta) = ev.pointer("/delta/text").and_then(|x| x.as_str())
                            {
                                buf.push_str(delta);
                            } else if let Some(text) =
                                ev.pointer("/content_block/text").and_then(|x| x.as_str())
                            {
                                buf.push_str(text);
                            } else if let Some(blocks) =
                                ev.pointer("/message/content").and_then(|x| x.as_array())
                            {
                                for block in blocks {
                                    if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                                        if let Some(text) =
                                            block.get("text").and_then(|t| t.as_str())
                                        {
                                            buf.push_str(text);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                buf
            }
        }
    }

    /// Extract tool execution outputs from stream-json events.
    ///
    /// Returns a list of `(tool_name, content)` pairs. Only meaningful for
    /// `StreamJson` responses from Claude CLI.
    #[must_use]
    pub fn extract_tool_outputs(&self) -> Vec<(Option<String>, String)> {
        let Self::StreamJson(events) = self else {
            return Vec::new();
        };
        let mut outputs = Vec::new();
        for ev in events {
            if ev.get("type").and_then(|t| t.as_str()) != Some("tool") {
                continue;
            }
            let content = ev
                .get("content")
                .and_then(|c| c.as_str())
                .or_else(|| ev.get("output").and_then(|o| o.as_str()));
            if let Some(content) = content.filter(|s| !s.is_empty()) {
                let tool_name = ev.get("tool").and_then(|t| t.as_str()).map(String::from);
                let truncated =
                    if content.len() > roko_core::defaults::DEFAULT_TOOL_OUTPUT_TRUNCATE_AT {
                        let mut end = roko_core::defaults::DEFAULT_TOOL_OUTPUT_TRUNCATE_AT;
                        while !content.is_char_boundary(end) {
                            end -= 1;
                        }
                        format!("{}...[truncated]", &content[..end])
                    } else {
                        content.to_string()
                    };
                outputs.push((tool_name, truncated));
            }
        }
        outputs
    }

    /// Extract session ID from a Claude CLI Result event.
    #[must_use]
    pub fn extract_session_id(&self) -> Option<String> {
        let Self::StreamJson(events) = self else {
            return None;
        };
        events.iter().rev().find_map(|ev| {
            if ev.get("type").and_then(|t| t.as_str()) == Some("result") {
                ev.get("session_id")
                    .and_then(|s| s.as_str())
                    .map(String::from)
            } else {
                None
            }
        })
    }

    /// Extract reasoning/thinking content from the response.
    #[must_use]
    pub fn extract_reasoning(&self) -> Option<String> {
        match self {
            Self::Json(v) => v
                .pointer("/choices/0/message")
                .and_then(extract_reasoning_from_value)
                .or_else(|| v.pointer("/message").and_then(extract_reasoning_from_value))
                .or_else(|| extract_reasoning_from_value(v)),
            Self::StreamJson(events) => {
                let mut buf = String::new();
                for ev in events {
                    if let Some(reasoning) = extract_reasoning_from_stream_event(ev) {
                        buf.push_str(&reasoning);
                    }
                }
                if buf.is_empty() { None } else { Some(buf) }
            }
            Self::Text(_) => None,
        }
    }

    /// Extract token usage metadata when the backend reports it.
    ///
    /// For `StreamJson` (Claude CLI), prefers the final `result` event's
    /// cumulative usage; falls back to the last `assistant` event's usage
    /// for partial/interrupted streams.
    #[must_use]
    pub fn extract_usage(&self) -> Usage {
        match self {
            Self::Json(v) => openai::parse_usage(v),
            Self::StreamJson(events) => {
                // Prefer the final `result` event — it carries cumulative session usage.
                for ev in events.iter().rev() {
                    if ev.get("type").and_then(|t| t.as_str()) == Some("result") {
                        if let Some(usage) = ev.get("usage") {
                            return Usage {
                                input_tokens: usage
                                    .get("input_tokens")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0)
                                    .min(u64::from(u32::MAX))
                                    as u32,
                                output_tokens: usage
                                    .get("output_tokens")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0)
                                    .min(u64::from(u32::MAX))
                                    as u32,
                                cache_read_tokens: usage
                                    .get("cache_read_input_tokens")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0)
                                    .min(u64::from(u32::MAX))
                                    as u32,
                                cache_create_tokens: usage
                                    .get("cache_creation_input_tokens")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0)
                                    .min(u64::from(u32::MAX))
                                    as u32,
                                ..Default::default()
                            };
                        }
                        break; // result event present but no usage block — stop here
                    }
                }
                // Fall back to the last assistant event's usage (partial stream).
                for ev in events.iter().rev() {
                    if ev.get("type").and_then(|t| t.as_str()) == Some("assistant") {
                        if let Some(usage) = ev
                            .get("message")
                            .and_then(|msg| msg.get("usage"))
                        {
                            return Usage {
                                input_tokens: usage
                                    .get("input_tokens")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0)
                                    .min(u64::from(u32::MAX))
                                    as u32,
                                output_tokens: usage
                                    .get("output_tokens")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0)
                                    .min(u64::from(u32::MAX))
                                    as u32,
                                cache_read_tokens: usage
                                    .get("cache_read_input_tokens")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0)
                                    .min(u64::from(u32::MAX))
                                    as u32,
                                cache_create_tokens: usage
                                    .get("cache_creation_input_tokens")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0)
                                    .min(u64::from(u32::MAX))
                                    as u32,
                                ..Default::default()
                            };
                        }
                    }
                }
                Usage::default()
            }
            Self::Text(_) => Usage::default(),
        }
    }

    /// Extract the raw finish reason string from this response.
    ///
    /// For `StreamJson` (Claude CLI), scans the events in reverse for the
    /// `result` event and derives a finish reason from `is_error` and the
    /// presence of tool-use blocks.
    #[must_use]
    pub fn extract_finish_reason_raw(&self) -> Option<String> {
        match self {
            Self::Json(v) => {
                // OpenAI / Ollama style
                v.pointer("/choices/0/finish_reason")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            }
            Self::StreamJson(events) => {
                for ev in events.iter().rev() {
                    if ev.get("type").and_then(|t| t.as_str()) == Some("result") {
                        // is_error: true maps to a terminal error condition.
                        if ev.get("is_error").and_then(serde_json::Value::as_bool)
                            == Some(true)
                        {
                            return Some("error".to_string());
                        }
                        // Explicit stop_reason field (Claude CLI >= 1.0.16)
                        // takes priority when present.
                        if let Some(reason) = ev
                            .get("stop_reason")
                            .and_then(serde_json::Value::as_str)
                            .filter(|s| !s.is_empty())
                        {
                            return Some(reason.to_string());
                        }
                        // Detect tool use from content_block_start events or
                        // assistant message content blocks.
                        let has_tool = events.iter().any(|e| {
                            // content_block_start with type=tool_use
                            if e.get("type").and_then(|t| t.as_str())
                                == Some("content_block_start")
                            {
                                if let Some(block) = e.get("content_block") {
                                    return block.get("type").and_then(|t| t.as_str())
                                        == Some("tool_use");
                                }
                            }
                            // assistant event with tool_use content blocks
                            if e.get("type").and_then(|t| t.as_str()) == Some("assistant") {
                                if let Some(content) = e
                                    .pointer("/message/content")
                                    .and_then(serde_json::Value::as_array)
                                {
                                    return content.iter().any(|block| {
                                        block.get("type").and_then(|t| t.as_str())
                                            == Some("tool_use")
                                    });
                                }
                            }
                            false
                        });
                        return Some(
                            if has_tool { "tool_use" } else { "end_turn" }.to_string(),
                        );
                    }
                }
                None
            }
            Self::Text(_) => None,
        }
    }
}

fn extract_reasoning_from_value(value: &serde_json::Value) -> Option<String> {
    if let Some(reasoning) = value
        .get("reasoning_content")
        .and_then(serde_json::Value::as_str)
    {
        return Some(reasoning.to_string());
    }

    value
        .get("content")
        .and_then(serde_json::Value::as_array)
        .and_then(|blocks| extract_reasoning_from_blocks(blocks.as_slice()))
}

fn extract_gemini_text(value: &serde_json::Value) -> Option<&str> {
    value
        .pointer("/candidates/0/content/parts")
        .and_then(serde_json::Value::as_array)
        .and_then(|parts| {
            parts.iter().find_map(|part| {
                part.get("text")
                    .and_then(serde_json::Value::as_str)
                    .filter(|text| !text.is_empty())
            })
        })
}

fn extract_reasoning_from_blocks(blocks: &[serde_json::Value]) -> Option<String> {
    let mut buf = String::new();

    for block in blocks {
        if block.get("type").and_then(serde_json::Value::as_str) != Some("thinking") {
            continue;
        }

        if let Some(reasoning) = block
            .get("thinking")
            .and_then(serde_json::Value::as_str)
            .or_else(|| block.get("text").and_then(serde_json::Value::as_str))
        {
            buf.push_str(reasoning);
        }
    }

    if buf.is_empty() { None } else { Some(buf) }
}

fn extract_reasoning_from_stream_event(event: &serde_json::Value) -> Option<String> {
    if let Some(reasoning) = event
        .pointer("/delta/reasoning_content")
        .and_then(serde_json::Value::as_str)
    {
        return Some(reasoning.to_string());
    }

    if let Some(reasoning) = event
        .pointer("/delta/thinking")
        .and_then(serde_json::Value::as_str)
    {
        return Some(reasoning.to_string());
    }

    if let Some(reasoning) = event
        .pointer("/content_block/reasoning_content")
        .and_then(serde_json::Value::as_str)
    {
        return Some(reasoning.to_string());
    }

    if let Some(block) = event.get("content_block")
        && block.get("type").and_then(serde_json::Value::as_str) == Some("thinking")
        && let Some(reasoning) = block
            .get("thinking")
            .and_then(serde_json::Value::as_str)
            .or_else(|| block.get("text").and_then(serde_json::Value::as_str))
    {
        return Some(reasoning.to_string());
    }

    if let Some(delta) = event.get("delta")
        && delta.get("type").and_then(serde_json::Value::as_str) == Some("thinking_delta")
        && let Some(reasoning) = delta
            .get("thinking")
            .and_then(serde_json::Value::as_str)
            .or_else(|| delta.get("text").and_then(serde_json::Value::as_str))
    {
        return Some(reasoning.to_string());
    }

    None
}

/// Errors a [`Translator`] may produce.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TranslatorError {
    /// The backend response claimed tool calls but couldn't be parsed.
    #[error("malformed response: {0}")]
    Malformed(String),
    /// The translator was handed a response in a format it doesn't accept.
    #[error("unsupported format: {0:?}")]
    UnsupportedFormat(ToolFormat),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_response_extract_text_from_text() {
        let r = BackendResponse::Text("hello".into());
        assert_eq!(r.extract_text(), "hello");
    }

    #[test]
    fn backend_response_extract_text_from_ollama_json() {
        let r = BackendResponse::Json(serde_json::json!({
            "message": { "content": "answer" }
        }));
        assert_eq!(r.extract_text(), "answer");
    }

    #[test]
    fn backend_response_extract_text_from_openai_json() {
        let r = BackendResponse::Json(serde_json::json!({
            "choices": [{"message": {"content": "done"}}]
        }));
        assert_eq!(r.extract_text(), "done");
    }

    #[test]
    fn backend_response_extract_text_from_gemini_json() {
        let r = BackendResponse::Json(serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "functionCall": { "name": "read_file", "args": { "path": "x" } } },
                        { "text": "done" }
                    ]
                }
            }]
        }));
        assert_eq!(r.extract_text(), "done");
    }

    #[test]
    fn backend_response_extract_text_from_stream_json() {
        let r = BackendResponse::StreamJson(vec![
            serde_json::json!({"delta": {"text": "one "}}),
            serde_json::json!({"delta": {"text": "two"}}),
        ]);
        assert_eq!(r.extract_text(), "one two");
    }

    #[test]
    fn backend_response_extract_text_empty_when_absent() {
        let r = BackendResponse::Json(serde_json::json!({}));
        assert_eq!(r.extract_text(), "");
    }

    #[test]
    fn backend_response_extract_reasoning_from_openai_json() {
        let r = BackendResponse::Json(serde_json::json!({
            "choices": [{
                "message": {
                    "content": "answer",
                    "reasoning_content": "thinking"
                }
            }]
        }));
        assert_eq!(r.extract_reasoning(), Some("thinking".to_string()));
    }

    #[test]
    fn backend_response_extract_reasoning_from_claude_json_blocks() {
        let r = BackendResponse::Json(serde_json::json!({
            "content": [
                { "type": "text", "text": "answer" },
                { "type": "thinking", "thinking": "hmm" }
            ]
        }));
        assert_eq!(r.extract_reasoning(), Some("hmm".to_string()));
    }

    #[test]
    fn backend_response_extract_reasoning_from_stream_json() {
        let r = BackendResponse::StreamJson(vec![
            serde_json::json!({
                "type": "content_block_start",
                "content_block": { "type": "thinking", "thinking": "step 1" }
            }),
            serde_json::json!({
                "type": "content_block_delta",
                "delta": { "type": "thinking_delta", "thinking": " step 2" }
            }),
        ]);
        assert_eq!(r.extract_reasoning(), Some("step 1 step 2".to_string()));
    }

    #[test]
    fn backend_response_extract_usage_from_openai_json() {
        let r = BackendResponse::Json(serde_json::json!({
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 5,
                "prompt_tokens_details": {
                    "cached_tokens": 3
                }
            }
        }));
        assert_eq!(
            r.extract_usage(),
            Usage {
                input_tokens: 12,
                output_tokens: 5,
                cache_read_tokens: 3,
                ..Default::default()
            }
        );
    }

    #[test]
    fn chat_response_defaults_and_variants() {
        let response = ChatResponse::default();
        assert_eq!(response.content, "");
        assert_eq!(response.reasoning, None);
        assert!(response.tool_calls.is_empty());
        assert_eq!(response.usage, Usage::default());
        assert!(matches!(response.finish_reason, FinishReason::Stop));
        assert_eq!(response.metadata.response_id, None);
        assert_eq!(response.metadata.model_used, None);
        assert_eq!(response.metadata.cached_tokens, None);
        assert_eq!(response.metadata.content_filter, None);
        assert_eq!(response.metadata.extra, None);
        assert_eq!(response.metadata.provider_latency_ms, None);
        assert_eq!(response.metadata.raw_finish_reason, None);

        assert_eq!(FinishReason::Length, FinishReason::Length);
        assert_eq!(FinishReason::ToolCalls, FinishReason::ToolCalls);
        assert_eq!(FinishReason::ContentFilter, FinishReason::ContentFilter);
        assert_eq!(
            FinishReason::Error("boom".into()),
            FinishReason::Error("boom".into())
        );
    }

    #[test]
    fn glm_finish_reasons() {
        assert_eq!(normalize_finish_reason("stop"), FinishReason::Stop);
        assert_eq!(normalize_finish_reason("end_turn"), FinishReason::Stop);
        assert_eq!(normalize_finish_reason("length"), FinishReason::Length);
        assert_eq!(normalize_finish_reason("max_tokens"), FinishReason::Length);
        assert_eq!(
            normalize_finish_reason("tool_calls"),
            FinishReason::ToolCalls
        );
        assert_eq!(normalize_finish_reason("tool_use"), FinishReason::ToolCalls);
        assert_eq!(
            normalize_finish_reason("content_filter"),
            FinishReason::ContentFilter
        );
        assert_eq!(
            normalize_finish_reason("sensitive"),
            FinishReason::ContentFilter
        );
        assert_eq!(
            normalize_finish_reason("network_error"),
            FinishReason::Error("network_error".into())
        );
        assert_eq!(
            normalize_finish_reason("model_context_window_exceeded"),
            FinishReason::Error("context_overflow".into())
        );
        assert_eq!(
            normalize_finish_reason("something_else"),
            FinishReason::Error("something_else".into())
        );
    }

    #[test]
    fn translator_error_display_shows_variants() {
        let e = TranslatorError::Malformed("bad json".into());
        assert!(e.to_string().contains("bad json"));
        let e = TranslatorError::UnsupportedFormat(ToolFormat::ReActText);
        assert!(e.to_string().contains("ReActText"));
    }

    // ── Enum round-trip tests (SLOT A0 requirement) ──────────────────

    #[test]
    fn rendered_tools_json_array_round_trip() {
        let inner = serde_json::json!([{"type": "function", "function": {"name": "read_file"}}]);
        let rt = RenderedTools::JsonArray(inner.clone());
        match rt {
            RenderedTools::JsonArray(v) => assert_eq!(v, inner),
            other => panic!("expected JsonArray, got {other:?}"),
        }
    }

    #[test]
    fn rendered_tools_cli_flag_round_trip() {
        let csv = "Read,Edit,Bash".to_string();
        let rt = RenderedTools::CliFlag(csv.clone());
        match rt {
            RenderedTools::CliFlag(s) => assert_eq!(s, csv),
            other => panic!("expected CliFlag, got {other:?}"),
        }
    }

    #[test]
    fn rendered_tools_system_prompt_block_round_trip() {
        let block = "You have access to:\n### read_file\n".to_string();
        let rt = RenderedTools::SystemPromptBlock(block.clone());
        match rt {
            RenderedTools::SystemPromptBlock(s) => assert_eq!(s, block),
            other => panic!("expected SystemPromptBlock, got {other:?}"),
        }
    }

    #[test]
    fn rendered_results_json_messages_round_trip() {
        let msgs = serde_json::json!([{"role": "tool", "content": "ok"}]);
        let rr = RenderedResults::JsonMessages(msgs.clone());
        match rr {
            RenderedResults::JsonMessages(v) => assert_eq!(v, msgs),
            other => panic!("expected JsonMessages, got {other:?}"),
        }
    }

    #[test]
    fn rendered_results_text_block_round_trip() {
        let obs = "Observation: file contents here\n\n".to_string();
        let rr = RenderedResults::TextBlock(obs.clone());
        match rr {
            RenderedResults::TextBlock(s) => assert_eq!(s, obs),
            other => panic!("expected TextBlock, got {other:?}"),
        }
    }

    #[test]
    fn rendered_results_handled_by_backend_round_trip() {
        let rr = RenderedResults::HandledByBackend;
        assert!(
            matches!(rr, RenderedResults::HandledByBackend),
            "expected HandledByBackend"
        );
    }

    #[test]
    fn backend_response_clone_preserves_variant() {
        let original = BackendResponse::Json(serde_json::json!({"test": true}));
        let cloned = original.clone();
        match (&original, &cloned) {
            (BackendResponse::Json(a), BackendResponse::Json(b)) => assert_eq!(a, b),
            _ => panic!("clone changed variant"),
        }
    }

    #[test]
    fn translator_error_eq_same_variants() {
        let a = TranslatorError::Malformed("x".into());
        let b = TranslatorError::Malformed("x".into());
        assert_eq!(a, b);
        let c = TranslatorError::Malformed("y".into());
        assert_ne!(a, c);
    }

    // ── Tool output extraction tests ──────────────────────────────────

    #[test]
    fn stream_json_extract_text_includes_tool_output() {
        let r = BackendResponse::StreamJson(vec![
            serde_json::json!({"type": "assistant", "delta": {"text": "Let me read that."}}),
            serde_json::json!({"type": "tool", "tool": "Read", "content": "fn main() {}"}),
            serde_json::json!({"type": "assistant", "delta": {"text": " Done."}}),
        ]);
        let text = r.extract_text();
        assert!(text.contains("Let me read that."));
        assert!(text.contains("[Read]"));
        assert!(text.contains("fn main() {}"));
        assert!(text.contains(" Done."));
    }

    #[test]
    fn stream_json_extract_tool_outputs_separate() {
        let r = BackendResponse::StreamJson(vec![
            serde_json::json!({"type": "tool", "tool": "Bash", "content": "hello world"}),
            serde_json::json!({"type": "tool", "content": "orphan output"}),
            serde_json::json!({"type": "assistant", "delta": {"text": "answer"}}),
        ]);
        let outputs = r.extract_tool_outputs();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].0.as_deref(), Some("Bash"));
        assert_eq!(outputs[0].1, "hello world");
        assert_eq!(outputs[1].0, None);
        assert_eq!(outputs[1].1, "orphan output");
    }

    #[test]
    fn stream_json_extract_tool_outputs_with_output_field() {
        let r = BackendResponse::StreamJson(vec![
            serde_json::json!({"type": "tool", "tool": "Edit", "output": "file modified"}),
        ]);
        let outputs = r.extract_tool_outputs();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].1, "file modified");
    }

    #[test]
    fn stream_json_extract_tool_outputs_skips_empty() {
        let r = BackendResponse::StreamJson(vec![
            serde_json::json!({"type": "tool", "tool": "Bash", "content": ""}),
            serde_json::json!({"type": "tool", "tool": "Read"}),
        ]);
        assert!(r.extract_tool_outputs().is_empty());
    }

    #[test]
    fn stream_json_extract_session_id() {
        let r = BackendResponse::StreamJson(vec![
            serde_json::json!({"type": "assistant", "delta": {"text": "hi"}}),
            serde_json::json!({"type": "result", "session_id": "abc-123", "is_error": false}),
        ]);
        assert_eq!(r.extract_session_id().as_deref(), Some("abc-123"));
    }

    #[test]
    fn extract_session_id_none_for_non_stream() {
        let r = BackendResponse::Text("hello".into());
        assert_eq!(r.extract_session_id(), None);
        let r2 = BackendResponse::Json(serde_json::json!({}));
        assert_eq!(r2.extract_session_id(), None);
    }

    #[test]
    fn stream_json_tool_output_truncates_large() {
        let large = "x".repeat(5000);
        let r = BackendResponse::StreamJson(vec![
            serde_json::json!({"type": "tool", "tool": "Bash", "content": large}),
        ]);
        let outputs = r.extract_tool_outputs();
        assert_eq!(outputs.len(), 1);
        assert!(outputs[0].1.len() < 4200);
        assert!(outputs[0].1.ends_with("...[truncated]"));
    }

    // ── StreamJson usage extraction tests ───────────────────────────────

    #[test]
    fn stream_json_extract_usage_from_result_event() {
        let r = BackendResponse::StreamJson(vec![
            serde_json::json!({"type": "content_block_delta", "delta": {"text": "hi"}}),
            serde_json::json!({
                "type": "result",
                "usage": {
                    "input_tokens": 1500,
                    "output_tokens": 350,
                    "cache_read_input_tokens": 200,
                    "cache_creation_input_tokens": 50
                }
            }),
        ]);
        let usage = r.extract_usage();
        assert_eq!(usage.input_tokens, 1500);
        assert_eq!(usage.output_tokens, 350);
        assert_eq!(usage.cache_read_tokens, 200);
        assert_eq!(usage.cache_create_tokens, 50);
    }

    #[test]
    fn stream_json_extract_usage_from_result_event_missing_usage() {
        let r = BackendResponse::StreamJson(vec![
            serde_json::json!({"type": "content_block_delta", "delta": {"text": "hi"}}),
            serde_json::json!({"type": "result", "is_error": false}),
        ]);
        assert_eq!(r.extract_usage(), Usage::default());
    }

    #[test]
    fn stream_json_extract_usage_no_result_falls_back_to_assistant() {
        let r = BackendResponse::StreamJson(vec![
            serde_json::json!({
                "type": "assistant",
                "message": {
                    "content": [],
                    "usage": {
                        "input_tokens": 800,
                        "output_tokens": 120,
                        "cache_read_input_tokens": 30,
                        "cache_creation_input_tokens": 10
                    }
                }
            }),
            serde_json::json!({"type": "content_block_delta", "delta": {"text": "text"}}),
        ]);
        let usage = r.extract_usage();
        assert_eq!(usage.input_tokens, 800);
        assert_eq!(usage.output_tokens, 120);
        assert_eq!(usage.cache_read_tokens, 30);
        assert_eq!(usage.cache_create_tokens, 10);
    }

    #[test]
    fn stream_json_extract_usage_returns_default_when_no_events() {
        let r = BackendResponse::StreamJson(vec![]);
        assert_eq!(r.extract_usage(), Usage::default());
    }

    // ── StreamJson finish reason extraction tests ───────────────────────

    #[test]
    fn stream_json_extract_finish_reason_end_turn() {
        let r = BackendResponse::StreamJson(vec![
            serde_json::json!({"type": "content_block_delta", "delta": {"text": "done"}}),
            serde_json::json!({"type": "result", "is_error": false}),
        ]);
        assert_eq!(r.extract_finish_reason_raw(), Some("end_turn".to_string()));
    }

    #[test]
    fn stream_json_extract_finish_reason_tool_use() {
        let r = BackendResponse::StreamJson(vec![
            serde_json::json!({
                "type": "content_block_start",
                "content_block": {"type": "tool_use", "name": "read_file", "id": "123"}
            }),
            serde_json::json!({"type": "content_block_delta", "delta": {"text": ""}}),
            serde_json::json!({"type": "result", "is_error": false}),
        ]);
        assert_eq!(
            r.extract_finish_reason_raw(),
            Some("tool_use".to_string())
        );
    }

    #[test]
    fn stream_json_extract_finish_reason_error() {
        let r = BackendResponse::StreamJson(vec![
            serde_json::json!({"type": "content_block_delta", "delta": {"text": "partial"}}),
            serde_json::json!({"type": "result", "is_error": true}),
        ]);
        assert_eq!(r.extract_finish_reason_raw(), Some("error".to_string()));
    }

    #[test]
    fn stream_json_extract_finish_reason_none_when_no_result() {
        let r = BackendResponse::StreamJson(vec![
            serde_json::json!({
                "type": "assistant",
                "message": {"content": [], "stop_reason": "end_turn"}
            }),
            serde_json::json!({"type": "content_block_delta", "delta": {"text": "hi"}}),
        ]);
        assert_eq!(r.extract_finish_reason_raw(), None);
    }
}
