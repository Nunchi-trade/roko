//! `ClaudeAgent` — calls Anthropic's Messages API (HTTPS) to generate completions.
//!
//! This is a **library-layer** agent. It does **not** read environment
//! variables; callers (the CLI) inject the API key explicitly. This keeps
//! the crate free of I/O side-effects and makes tests deterministic.
//!
//! # Design: `HttpPoster` indirection
//!
//! So tests never hit the real network, HTTP is dispatched through the shared
//! [`crate::http::HttpPoster`] trait. Production uses
//! [`crate::http::ReqwestPoster`], which calls `reqwest`. Tests inject a
//! `MockPoster` that returns canned responses.

use crate::agent::{Agent, AgentResult};
#[cfg(test)]
use crate::http::HttpPostError;
use crate::http::{HttpPoster, ReqwestPoster};
use crate::translate::claude::{inject_cache_markers, inject_cache_markers_into_content};
use crate::usage::Usage;
use async_trait::async_trait;
use roko_core::{Body, Context, Kind, Provenance, Signal};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Instant;

/// Default Anthropic Messages API endpoint.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// Default Anthropic API version header.
pub const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";

/// Maximum output tokens requested per call (default).
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

// ─── Anthropic wire types (minimal subset) ─────────────────────────────────

/// One content block inside an assistant message.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    #[serde(other)]
    Other,
}

/// A native Anthropic tool definition passed to `tools`.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AnthropicTool {
    /// Tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema object for tool input.
    pub input_schema: serde_json::Value,
}

impl AnthropicTool {
    /// Construct a new Anthropic tool definition.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        }
    }
}

/// Anthropic `tool_choice` policy.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolChoice {
    /// Let the model decide whether to call tools.
    Auto,
    /// Force at least one tool call.
    Any,
    /// Force a specific named tool.
    Tool {
        /// Tool name to force.
        name: String,
    },
}

/// Usage block returned by Anthropic.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[allow(clippy::struct_field_names)]
struct ApiUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
}

/// Top-level Anthropic Messages API response.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct MessagesResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    usage: ApiUsage,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

// ─── ClaudeAgent ───────────────────────────────────────────────────────────

/// An [`Agent`] that calls Anthropic's Messages API.
///
/// The agent is fully configurable; it never reads from the environment. The
/// API key is injected by the caller (typically the CLI layer). Use
/// [`ClaudeAgent::with_base_url`] to redirect requests to a test server.
///
/// # Example
///
/// ```ignore
/// use roko_agent::claude_agent::ClaudeAgent;
/// let agent = ClaudeAgent::new("sk-ant-...", "claude-opus-4-6");
/// ```
pub struct ClaudeAgent {
    api_key: String,
    model: String,
    base_url: String,
    timeout_ms: u64,
    name: String,
    max_tokens: u32,
    anthropic_version: String,
    system_prompt: Option<String>,
    tools: Option<Vec<AnthropicTool>>,
    tool_choice: Option<ToolChoice>,
    poster: Arc<dyn HttpPoster>,
}

impl std::fmt::Debug for ClaudeAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeAgent")
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("timeout_ms", &self.timeout_ms)
            .field("name", &self.name)
            .field("max_tokens", &self.max_tokens)
            .field("anthropic_version", &self.anthropic_version)
            .finish_non_exhaustive()
    }
}

impl ClaudeAgent {
    /// Construct a new `ClaudeAgent` with the given API key and model slug.
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let model = model.into();
        let name = format!("claude:{model}");
        Self {
            api_key: api_key.into(),
            model,
            base_url: DEFAULT_BASE_URL.to_owned(),
            timeout_ms: 120_000,
            name,
            max_tokens: DEFAULT_MAX_TOKENS,
            anthropic_version: DEFAULT_ANTHROPIC_VERSION.to_owned(),
            system_prompt: None,
            tools: None,
            tool_choice: None,
            poster: Arc::new(ReqwestPoster::new()),
        }
    }

    /// Override the per-request timeout in milliseconds (default 120 s).
    #[must_use]
    pub const fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Override the API base URL. Primarily for tests.
    ///
    /// `base_url` is prepended to `/v1/messages`; do not include a trailing slash.
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        let mut v = base_url.into();
        while v.ends_with('/') {
            v.pop();
        }
        self.base_url = v;
        self
    }

    /// Override the agent's display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Override the `max_tokens` limit sent on each request.
    #[must_use]
    pub const fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Attach a system prompt to the request payload.
    #[must_use]
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Attach native Anthropic tool definitions.
    #[must_use]
    pub fn with_tools(mut self, tools: Vec<AnthropicTool>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Attach Anthropic `tool_choice` policy.
    #[must_use]
    pub fn with_tool_choice(mut self, tool_choice: ToolChoice) -> Self {
        self.tool_choice = Some(tool_choice);
        self
    }

    /// Override the `anthropic-version` header sent on each request.
    #[must_use]
    pub fn with_anthropic_version(mut self, version: impl Into<String>) -> Self {
        self.anthropic_version = version.into();
        self
    }

    /// Inject a custom [`HttpPoster`] (mostly for tests).
    #[must_use]
    pub fn with_http_poster(mut self, poster: Arc<dyn HttpPoster>) -> Self {
        self.poster = poster;
        self
    }

    /// Configured model slug.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Configured base URL.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/messages", self.base_url)
    }

    fn headers(&self) -> Vec<(String, String)> {
        vec![
            ("x-api-key".to_owned(), self.api_key.clone()),
            (
                "anthropic-version".to_owned(),
                self.anthropic_version.clone(),
            ),
        ]
    }

    fn request_body(&self, prompt_text: &str) -> Result<Value, serde_json::Error> {
        let mut messages = vec![json!({
            "role": "user",
            "content": prompt_text,
        })];
        inject_cache_markers(&mut messages);

        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": messages,
        });

        if let Some(system_prompt) = &self.system_prompt {
            let mut system = Value::String(system_prompt.clone());
            let _ = inject_cache_markers_into_content(&mut system);
            body["system"] = system;
        }
        if let Some(tools) = &self.tools {
            body["tools"] = serde_json::to_value(tools)?;
        }
        if let Some(tool_choice) = &self.tool_choice {
            body["tool_choice"] = serde_json::to_value(tool_choice)?;
        }

        Ok(body)
    }

    fn fail(&self, input: &Signal, reason: &str, started: Instant) -> AgentResult {
        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let output = input
            .derive(Kind::AgentOutput, Body::text(reason))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("failed", "true")
            .build();
        AgentResult::fail(output).with_usage(Usage {
            wall_ms,
            ..Default::default()
        })
    }
}

#[async_trait]
impl Agent for ClaudeAgent {
    #[allow(clippy::too_many_lines)]
    async fn run(&self, input: &Signal, _ctx: &Context) -> AgentResult {
        let started = Instant::now();

        let prompt_text = match input.body.as_text() {
            Ok(s) => s.to_owned(),
            Err(_) => match serde_json::to_string(&input.body) {
                Ok(s) => s,
                Err(e) => {
                    return self.fail(
                        input,
                        &format!("input body not readable as text or json: {e}"),
                        started,
                    );
                }
            },
        };

        let request_body = match self.request_body(&prompt_text) {
            Ok(body) => body,
            Err(e) => {
                return self.fail(input, &format!("serialize request failed: {e}"), started);
            }
        };
        let body = match serde_json::to_string(&request_body) {
            Ok(s) => s,
            Err(e) => {
                return self.fail(input, &format!("serialize request failed: {e}"), started);
            }
        };

        let url = self.endpoint();
        let headers = self.headers();

        let response_text = match self
            .poster
            .post_json(&url, &headers, body.as_bytes(), self.timeout_ms)
            .await
        {
            Ok(text) => text,
            Err(e) => {
                let reason = match e.status {
                    Some(code) => format!("http {code}: {}", e.message),
                    None => format!("transport error: {}", e.message),
                };
                return self.fail(input, &reason, started);
            }
        };

        if response_text.trim().is_empty() {
            return self.fail(input, "empty response body", started);
        }

        let parsed: MessagesResponse = match serde_json::from_str(&response_text) {
            Ok(p) => p,
            Err(e) => {
                return self.fail(input, &format!("malformed response JSON: {e}"), started);
            }
        };

        let mut combined = String::new();
        let mut trace = Vec::new();
        for block in &parsed.content {
            match block {
                ContentBlock::Text { text } => {
                    if !combined.is_empty() {
                        combined.push('\n');
                    }
                    combined.push_str(text);
                }
                ContentBlock::ToolUse { id, name, input } => {
                    let input_json =
                        serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string());
                    trace.push(
                        Signal::builder(Kind::AgentMessage)
                            .body(Body::text(format!(
                                "tool_use id={id} name={name} input={input_json}"
                            )))
                            .provenance(Provenance::agent(&self.name))
                            .tag("stream", "tool_use")
                            .tag("tool_id", id)
                            .tag("tool_name", name)
                            .build(),
                    );
                }
                ContentBlock::Other => {}
            }
        }

        if combined.is_empty() {
            if trace.is_empty() {
                return self.fail(input, "response contained no text content blocks", started);
            }
            combined = "assistant requested tool use".to_string();
        }

        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let usage = Usage {
            input_tokens: parsed.usage.input_tokens,
            output_tokens: parsed.usage.output_tokens,
            cache_read_tokens: parsed.usage.cache_read_input_tokens,
            cache_create_tokens: parsed.usage.cache_creation_input_tokens,
            cost_usd: 0.0,
            wall_ms,
        };

        let mut builder = input
            .derive(Kind::AgentOutput, Body::text(combined))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("model", &self.model);
        if let Some(stop) = &parsed.stop_reason {
            builder = builder.tag("stop_reason", stop);
        }
        if let Some(id) = &parsed.id {
            builder = builder.tag("response_id", id);
        }
        let output = builder.build();

        AgentResult::ok(output).with_trace(trace).with_usage(usage)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn supports_streaming(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // A mock HttpPoster that returns canned responses and records calls.
    struct MockPoster {
        response: Mutex<Result<String, HttpPostError>>,
        calls: Mutex<Vec<MockCall>>,
    }

    #[derive(Clone, Debug)]
    struct MockCall {
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        timeout_ms: u64,
    }

    impl MockPoster {
        fn ok(body: impl Into<String>) -> Arc<Self> {
            Arc::new(Self {
                response: Mutex::new(Ok(body.into())),
                calls: Mutex::new(Vec::new()),
            })
        }

        fn err(status: Option<u16>, msg: impl Into<String>) -> Arc<Self> {
            let err = match status {
                Some(s) => HttpPostError::http(s, msg),
                None => HttpPostError::transport(msg),
            };
            Arc::new(Self {
                response: Mutex::new(Err(err)),
                calls: Mutex::new(Vec::new()),
            })
        }

        fn call_count(&self) -> usize {
            self.calls.lock().map(|v| v.len()).unwrap_or(0)
        }

        fn last_call(&self) -> Option<MockCall> {
            self.calls.lock().ok().and_then(|v| v.last().cloned())
        }
    }

    #[async_trait]
    impl HttpPoster for MockPoster {
        async fn post_json(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &[u8],
            timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            if let Ok(mut c) = self.calls.lock() {
                c.push(MockCall {
                    url: url.to_owned(),
                    headers: headers.to_vec(),
                    body: body.to_vec(),
                    timeout_ms,
                });
            }
            // Clone the stored result without consuming it.
            let guard = self
                .response
                .lock()
                .map_err(|_| HttpPostError::transport("lock poisoned"))?;
            match &*guard {
                Ok(s) => Ok(s.clone()),
                Err(e) => Err(e.clone()),
            }
        }
    }

    fn prompt(text: &str) -> Signal {
        Signal::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    fn agent_with(poster: Arc<dyn HttpPoster>) -> ClaudeAgent {
        ClaudeAgent::new("test-key", "claude-test-model")
            .with_base_url("https://example.test")
            .with_http_poster(poster)
    }

    #[tokio::test]
    async fn successful_response_populates_output_and_usage() {
        let body = serde_json::json!({
            "id": "msg_abc",
            "model": "claude-test-model",
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "hello world"}],
            "usage": {
                "input_tokens": 12,
                "output_tokens": 34,
                "cache_read_input_tokens": 5,
                "cache_creation_input_tokens": 7
            }
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = agent_with(poster.clone());
        let result = agent.run(&prompt("hi"), &Context::now()).await;
        assert!(result.success);
        let text = result.output.body.as_text().unwrap_or("");
        assert_eq!(text, "hello world");
        assert_eq!(result.usage.input_tokens, 12);
        assert_eq!(result.usage.output_tokens, 34);
        assert_eq!(result.usage.cache_read_tokens, 5);
        assert_eq!(result.usage.cache_create_tokens, 7);
        assert_eq!(result.output.tag("model"), Some("claude-test-model"));
        assert_eq!(result.output.tag("stop_reason"), Some("end_turn"));
        assert_eq!(result.output.tag("response_id"), Some("msg_abc"));
        assert_eq!(poster.call_count(), 1);
    }

    #[tokio::test]
    async fn serialized_request_includes_system_prompt() {
        let poster = MockPoster::ok(
            serde_json::json!({
                "content": [{"type": "text", "text": "ok"}],
                "usage": {}
            })
            .to_string(),
        );
        let agent = agent_with(poster.clone()).with_system_prompt("system guidance");
        let _ = agent.run(&prompt("hi"), &Context::now()).await;

        let call = poster.last_call().expect("expected one HTTP call");
        let body: serde_json::Value = serde_json::from_slice(&call.body).expect("valid json");
        assert_eq!(
            body.get("system").and_then(serde_json::Value::as_str),
            Some("system guidance")
        );
        assert_eq!(
            body.get("model").and_then(serde_json::Value::as_str),
            Some("claude-test-model")
        );
    }

    #[tokio::test]
    async fn serialized_request_includes_tools_and_tool_choice() {
        let poster = MockPoster::ok(
            serde_json::json!({
                "content": [{"type": "text", "text": "ok"}],
                "usage": {}
            })
            .to_string(),
        );
        let tools = vec![AnthropicTool::new(
            "read_file",
            "Read a file",
            serde_json::json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }),
        )];
        let agent = agent_with(poster.clone())
            .with_tools(tools)
            .with_tool_choice(ToolChoice::Tool {
                name: "read_file".to_string(),
            });
        let _ = agent.run(&prompt("hi"), &Context::now()).await;

        let call = poster.last_call().expect("expected one HTTP call");
        let body: serde_json::Value = serde_json::from_slice(&call.body).expect("valid json");
        assert_eq!(body["tools"][0]["name"], "read_file");
        assert_eq!(body["tool_choice"]["type"], "tool");
        assert_eq!(body["tool_choice"]["name"], "read_file");
    }

    #[tokio::test]
    async fn http_4xx_returns_failure() {
        let poster = MockPoster::err(Some(401), r#"{"error": "unauthorized"}"#);
        let agent = agent_with(poster);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        let reason = result.output.body.as_text().unwrap_or("");
        assert!(reason.contains("http 401"), "reason was: {reason}");
        assert_eq!(result.output.tag("failed"), Some("true"));
    }

    #[tokio::test]
    async fn http_5xx_returns_failure() {
        let poster = MockPoster::err(Some(503), "service unavailable");
        let agent = agent_with(poster);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .unwrap_or("")
                .contains("http 503")
        );
    }

    #[tokio::test]
    async fn transport_error_returns_failure() {
        let poster = MockPoster::err(None, "dns lookup failed");
        let agent = agent_with(poster);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .unwrap_or("")
                .contains("transport error")
        );
    }

    #[tokio::test]
    async fn malformed_json_returns_failure() {
        let poster = MockPoster::ok("{not really json");
        let agent = agent_with(poster);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .unwrap_or("")
                .contains("malformed response JSON")
        );
    }

    #[tokio::test]
    async fn empty_body_returns_failure() {
        let poster = MockPoster::ok("   \n  ");
        let agent = agent_with(poster);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .unwrap_or("")
                .contains("empty response body")
        );
    }

    #[tokio::test]
    async fn missing_content_returns_failure() {
        let body = serde_json::json!({
            "id": "msg_1",
            "usage": {"input_tokens": 1, "output_tokens": 2},
            "content": []
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = agent_with(poster);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .unwrap_or("")
                .contains("no text content blocks")
        );
    }

    #[tokio::test]
    async fn non_text_content_blocks_are_skipped() {
        let body = serde_json::json!({
            "content": [
                {"type": "tool_use", "id": "t1", "name": "calc", "input": {}},
                {"type": "text", "text": "only text survives"}
            ],
            "usage": {"input_tokens": 1, "output_tokens": 2}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = agent_with(poster);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(
            result.output.body.as_text().unwrap_or(""),
            "only text survives"
        );
        assert_eq!(result.trace.len(), 1);
        assert_eq!(result.trace[0].tag("stream"), Some("tool_use"));
    }

    #[tokio::test]
    async fn tool_use_only_response_is_not_failed() {
        let body = serde_json::json!({
            "content": [
                {"type": "tool_use", "id": "t1", "name": "calc", "input": {"x": 1}}
            ],
            "usage": {"input_tokens": 1, "output_tokens": 2}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = agent_with(poster);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(
            result.output.body.as_text().unwrap_or(""),
            "assistant requested tool use"
        );
        assert_eq!(result.trace.len(), 1);
        assert_eq!(result.trace[0].tag("tool_name"), Some("calc"));
    }

    #[tokio::test]
    async fn multiple_text_blocks_are_joined() {
        let body = serde_json::json!({
            "content": [
                {"type": "text", "text": "part1"},
                {"type": "text", "text": "part2"}
            ],
            "usage": {"input_tokens": 1, "output_tokens": 2}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = agent_with(poster);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap_or(""), "part1\npart2");
    }

    #[tokio::test]
    async fn custom_base_url_is_used() {
        let body = serde_json::json!({
            "content": [{"type": "text", "text": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = ClaudeAgent::new("k", "m")
            .with_base_url("https://custom.test/api/")
            .with_http_poster(poster.clone());
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let call = poster.last_call().expect("should have a recorded call");
        assert_eq!(call.url, "https://custom.test/api/v1/messages");
    }

    #[tokio::test]
    async fn headers_include_api_key_and_version() {
        let body = serde_json::json!({
            "content": [{"type": "text", "text": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = ClaudeAgent::new("secret-key", "claude-x")
            .with_http_poster(poster.clone())
            .with_anthropic_version("2024-01-01");
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let call = poster.last_call().expect("call recorded");
        let header_map: std::collections::HashMap<String, String> =
            call.headers.into_iter().collect();
        assert_eq!(header_map.get("x-api-key"), Some(&"secret-key".to_owned()));
        assert_eq!(
            header_map.get("anthropic-version"),
            Some(&"2024-01-01".to_owned())
        );
    }

    #[tokio::test]
    async fn timeout_ms_is_forwarded_to_poster() {
        let body = serde_json::json!({
            "content": [{"type": "text", "text": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = ClaudeAgent::new("k", "m")
            .with_http_poster(poster.clone())
            .with_timeout_ms(42_000);
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let call = poster.last_call().expect("call recorded");
        assert_eq!(call.timeout_ms, 42_000);
    }

    #[tokio::test]
    async fn request_body_contains_model_and_prompt() {
        let body = serde_json::json!({
            "content": [{"type": "text", "text": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = ClaudeAgent::new("k", "my-model")
            .with_http_poster(poster.clone())
            .with_max_tokens(256);
        let _ = agent.run(&prompt("hello there"), &Context::now()).await;
        let call = poster.last_call().expect("call recorded");
        let v: serde_json::Value =
            serde_json::from_slice(&call.body).expect("request body is valid JSON");
        assert_eq!(v["model"], "my-model");
        assert_eq!(v["max_tokens"], 256);
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["messages"][0]["content"], "hello there");
    }

    #[tokio::test]
    async fn anthropic_cache_markers_convert_system_prompt_boundaries() {
        let body = serde_json::json!({
            "content": [{"type": "text", "text": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = ClaudeAgent::new("k", "my-model")
            .with_http_poster(poster.clone())
            .with_system_prompt(
                "Role instructions\n\n<!-- cache:system -->\n\nWorkspace context\n\n<!-- cache:session -->\n\nTurn-local instructions",
            );

        let _ = agent.run(&prompt("hello there"), &Context::now()).await;

        let call = poster.last_call().expect("call recorded");
        let v: serde_json::Value =
            serde_json::from_slice(&call.body).expect("request body is valid JSON");
        let system = v["system"].as_array().expect("system blocks");
        assert_eq!(system.len(), 3);
        assert_eq!(system[0]["cache_control"]["type"], "ephemeral");
        assert_eq!(system[1]["cache_control"]["type"], "ephemeral");
        assert!(system[2].get("cache_control").is_none());
    }

    #[tokio::test]
    async fn anthropic_cache_markers_convert_message_boundaries() {
        let body = serde_json::json!({
            "content": [{"type": "text", "text": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = ClaudeAgent::new("k", "my-model").with_http_poster(poster.clone());
        let _ = agent
            .run(
                &prompt("Shared prefix\n\n<!-- cache:system -->\n\nFresh tail"),
                &Context::now(),
            )
            .await;

        let call = poster.last_call().expect("call recorded");
        let v: serde_json::Value =
            serde_json::from_slice(&call.body).expect("request body is valid JSON");
        let message_blocks = v["messages"][0]["content"]
            .as_array()
            .expect("message blocks");
        assert_eq!(message_blocks.len(), 2);
        assert_eq!(message_blocks[0]["cache_control"]["type"], "ephemeral");
        assert!(message_blocks[1].get("cache_control").is_none());
    }

    #[tokio::test]
    async fn output_lineage_tracks_input() {
        let body = serde_json::json!({
            "content": [{"type": "text", "text": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = agent_with(poster);
        let input = prompt("track me");
        let input_id = input.id;
        let result = agent.run(&input, &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.lineage, vec![input_id]);
    }

    #[tokio::test]
    async fn with_base_url_strips_trailing_slashes() {
        let agent = ClaudeAgent::new("k", "m").with_base_url("https://x.test///");
        assert_eq!(agent.base_url(), "https://x.test");
    }

    #[tokio::test]
    async fn name_defaults_include_model() {
        let agent = ClaudeAgent::new("k", "claude-3-5-sonnet");
        assert_eq!(agent.name(), "claude:claude-3-5-sonnet");
        assert!(!agent.supports_streaming());
    }

    #[tokio::test]
    async fn with_name_overrides_default_name() {
        let agent = ClaudeAgent::new("k", "m").with_name("my-agent");
        assert_eq!(agent.name(), "my-agent");
    }
}
