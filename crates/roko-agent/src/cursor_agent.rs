//! `CursorAgent` (§7.3) — Cursor ACP (Agent Client Protocol) backend.
//!
//! Cursor's full ACP is WebSocket-framed JSON with a session
//! lifecycle plus streaming tool-call events (see Mori's
//! `CursorAcpConnection` in `apps/mori/src/agent/connection.rs` around
//! lines 1792 to 2338 for the native wire shape). A full WS transport
//! is **out of scope for this wave**. The path shipped here is the
//! **HTTPS `/v1/prompt` fallback**: a single POST carrying an
//! ACP-envelope prompt, response parsed into an [`AgentResult`].
//!
//! # Design
//!
//! Like [`crate::claude_agent::ClaudeAgent`], this is a
//! **library-layer** agent. It does **not** read environment variables;
//! callers (the CLI) inject the API key explicitly. HTTP is dispatched
//! through the shared [`crate::http::HttpPoster`] trait so tests never
//! hit the real network.
//!
//! # Wire format
//!
//! Request body (ACP-over-HTTP minimal envelope):
//!
//! ```json
//! {
//!   "protocol": "acp/1",
//!   "model": "cursor-composer",
//!   "prompt": {"role": "user", "content": "…"}
//! }
//! ```
//!
//! Response:
//!
//! ```json
//! {
//!   "session_id": "…",
//!   "model": "…",
//!   "messages": [{"role": "assistant", "content": "…"}],
//!   "usage": {"input_tokens": 12, "output_tokens": 34},
//!   "stop_reason": "end_turn"
//! }
//! ```

use crate::agent::{Agent, AgentResult};
#[cfg(test)]
use crate::http::HttpPostError;
use crate::http::{HttpPoster, ReqwestPoster};
use crate::usage::Usage;
use async_trait::async_trait;
use roko_core::{Body, Context, Kind, Provenance, Signal};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// Default Cursor ACP endpoint host.
pub const DEFAULT_BASE_URL: &str = "https://api.cursor.sh";

/// Default model slug used when the caller omits one.
pub const DEFAULT_MODEL: &str = "cursor-composer";

/// Default ACP protocol version tag.
pub const DEFAULT_PROTOCOL_VERSION: &str = "acp/1";

// ─── ACP-over-HTTP wire types (minimal subset) ─────────────────────────────

/// One message in the ACP prompt/response exchange.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct AcpMessage {
    #[serde(default)]
    role: String,
    #[serde(default)]
    content: String,
}

/// ACP usage block (token counts only — no cache tokens, no cost).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct ApiUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

/// Top-level Cursor ACP response.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct PromptResponse {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    messages: Vec<AcpMessage>,
    #[serde(default)]
    usage: ApiUsage,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct RequestPrompt<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct PromptRequest<'a> {
    protocol: &'a str,
    model: &'a str,
    prompt: RequestPrompt<'a>,
}

// ─── CursorAgent ───────────────────────────────────────────────────────────

/// An [`Agent`] that calls Cursor's ACP-over-HTTP `/v1/prompt` endpoint.
///
/// The agent is fully configurable; it never reads from the environment.
/// The API key is injected by the caller (typically the CLI layer). Use
/// [`CursorAgent::with_base_url`] to redirect requests to a test server.
///
/// # Example
///
/// ```ignore
/// use roko_agent::cursor_agent::CursorAgent;
/// let agent = CursorAgent::new("sk-cursor-...", "cursor-composer");
/// ```
pub struct CursorAgent {
    api_key: String,
    model: String,
    name: String,
    base_url: String,
    timeout_ms: u64,
    protocol_version: String,
    extra_headers: Vec<(String, String)>,
    poster: Arc<dyn HttpPoster>,
}

impl std::fmt::Debug for CursorAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CursorAgent")
            .field("model", &self.model)
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("timeout_ms", &self.timeout_ms)
            .field("protocol_version", &self.protocol_version)
            .finish_non_exhaustive()
    }
}

impl CursorAgent {
    /// Construct a new `CursorAgent` with the given API key and model slug.
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let model = model.into();
        let name = format!("cursor:{model}");
        Self {
            api_key: api_key.into(),
            model,
            name,
            base_url: DEFAULT_BASE_URL.to_owned(),
            timeout_ms: 120_000,
            protocol_version: DEFAULT_PROTOCOL_VERSION.to_owned(),
            extra_headers: Vec::new(),
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
    /// `base_url` is prepended to `/v1/prompt`; trailing slashes are
    /// stripped.
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        let mut v = url.into();
        while v.ends_with('/') {
            v.pop();
        }
        self.base_url = v;
        self
    }

    /// Override the ACP protocol version tag sent on each request
    /// (default `"acp/1"`).
    #[must_use]
    pub fn with_protocol_version(mut self, v: impl Into<String>) -> Self {
        self.protocol_version = v.into();
        self
    }

    /// Override the agent's display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Inject a custom [`HttpPoster`] (mostly for tests).
    #[must_use]
    pub fn with_http_poster(mut self, poster: Arc<dyn HttpPoster>) -> Self {
        self.poster = poster;
        self
    }

    /// Inject additional HTTP headers on every request.
    #[must_use]
    pub fn with_extra_headers(mut self, extra_headers: HashMap<String, String>) -> Self {
        let mut extra_headers: Vec<(String, String)> = extra_headers.into_iter().collect();
        extra_headers.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        self.extra_headers = extra_headers;
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
        format!("{}/v1/prompt", self.base_url)
    }

    fn headers(&self) -> Vec<(String, String)> {
        let mut headers = vec![
            (
                "authorization".to_owned(),
                format!("Bearer {}", self.api_key),
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
            (
                "x-cursor-protocol".to_owned(),
                self.protocol_version.clone(),
            ),
        ];
        headers.extend(self.extra_headers.iter().cloned());
        headers
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
impl Agent for CursorAgent {
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

        let req = PromptRequest {
            protocol: &self.protocol_version,
            model: &self.model,
            prompt: RequestPrompt {
                role: "user",
                content: &prompt_text,
            },
        };
        let body = match serde_json::to_string(&req) {
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

        let parsed: PromptResponse = match serde_json::from_str(&response_text) {
            Ok(p) => p,
            Err(e) => {
                return self.fail(input, &format!("malformed response JSON: {e}"), started);
            }
        };

        let assistant_text = parsed
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant")
            .map(|m| m.content.clone());

        let Some(assistant_text) = assistant_text else {
            return self.fail(input, "response contained no assistant message", started);
        };

        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let usage = Usage {
            input_tokens: parsed.usage.input_tokens,
            output_tokens: parsed.usage.output_tokens,
            cache_read_tokens: 0,
            cache_create_tokens: 0,
            cost_usd: 0.0,
            wall_ms,
        };

        let mut builder = input
            .derive(Kind::AgentOutput, Body::text(assistant_text))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("model", &self.model);
        if let Some(sid) = &parsed.session_id {
            builder = builder.tag("session_id", sid);
        }
        if let Some(stop) = &parsed.stop_reason {
            builder = builder.tag("stop_reason", stop);
        }
        let output = builder.build();

        AgentResult::ok(output).with_usage(usage)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn supports_streaming(&self) -> bool {
        true
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

    fn agent_with(poster: Arc<dyn HttpPoster>) -> CursorAgent {
        CursorAgent::new("test-key", "cursor-composer")
            .with_base_url("https://example.test")
            .with_http_poster(poster)
    }

    #[tokio::test]
    async fn successful_response_populates_output_and_usage() {
        let body = serde_json::json!({
            "session_id": "sess_abc",
            "model": "cursor-composer",
            "stop_reason": "end_turn",
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello from cursor"}
            ],
            "usage": {"input_tokens": 12, "output_tokens": 34}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = agent_with(poster.clone());
        let result = agent.run(&prompt("hi"), &Context::now()).await;
        assert!(result.success);
        let text = result.output.body.as_text().unwrap_or("");
        assert_eq!(text, "hello from cursor");
        assert_eq!(result.usage.input_tokens, 12);
        assert_eq!(result.usage.output_tokens, 34);
        assert_eq!(result.usage.cache_read_tokens, 0);
        assert_eq!(result.usage.cache_create_tokens, 0);
        assert_eq!(result.output.tag("model"), Some("cursor-composer"));
        assert_eq!(result.output.tag("session_id"), Some("sess_abc"));
        assert_eq!(result.output.tag("stop_reason"), Some("end_turn"));
        assert_eq!(poster.call_count(), 1);
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
    async fn missing_assistant_message_returns_failure() {
        // messages[] has only a user message — no assistant reply.
        let body = serde_json::json!({
            "session_id": "s1",
            "messages": [{"role": "user", "content": "hi"}],
            "usage": {"input_tokens": 1, "output_tokens": 0}
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
                .contains("no assistant message")
        );
    }

    #[tokio::test]
    async fn empty_messages_array_returns_failure() {
        let body = serde_json::json!({
            "session_id": "s1",
            "messages": [],
            "usage": {"input_tokens": 1, "output_tokens": 0}
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
                .contains("no assistant message")
        );
    }

    #[tokio::test]
    async fn multiple_messages_uses_last_assistant() {
        let body = serde_json::json!({
            "messages": [
                {"role": "assistant", "content": "first draft"},
                {"role": "user", "content": "more please"},
                {"role": "assistant", "content": "final answer"}
            ],
            "usage": {"input_tokens": 2, "output_tokens": 3}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = agent_with(poster);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap_or(""), "final answer");
    }

    #[tokio::test]
    async fn custom_base_url_strips_trailing_slashes() {
        let body = serde_json::json!({
            "messages": [{"role": "assistant", "content": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = CursorAgent::new("k", "m")
            .with_base_url("https://custom.test/api///")
            .with_http_poster(poster.clone());
        assert_eq!(agent.base_url(), "https://custom.test/api");
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let call = poster.last_call().expect("should have a recorded call");
        assert_eq!(call.url, "https://custom.test/api/v1/prompt");
    }

    #[tokio::test]
    async fn headers_include_bearer_and_acp_protocol() {
        let body = serde_json::json!({
            "messages": [{"role": "assistant", "content": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = CursorAgent::new("secret-key", "cursor-x").with_http_poster(poster.clone());
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let call = poster.last_call().expect("call recorded");
        let header_map: std::collections::HashMap<String, String> =
            call.headers.into_iter().collect();
        assert_eq!(
            header_map.get("authorization"),
            Some(&"Bearer secret-key".to_owned())
        );
        assert_eq!(
            header_map.get("content-type"),
            Some(&"application/json".to_owned())
        );
        assert_eq!(
            header_map.get("x-cursor-protocol"),
            Some(&"acp/1".to_owned())
        );
    }

    #[tokio::test]
    async fn timeout_ms_is_forwarded_to_poster() {
        let body = serde_json::json!({
            "messages": [{"role": "assistant", "content": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = CursorAgent::new("k", "m")
            .with_http_poster(poster.clone())
            .with_timeout_ms(42_000);
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let call = poster.last_call().expect("call recorded");
        assert_eq!(call.timeout_ms, 42_000);
    }

    #[tokio::test]
    async fn request_body_contains_protocol_and_model() {
        let body = serde_json::json!({
            "messages": [{"role": "assistant", "content": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = CursorAgent::new("k", "my-model").with_http_poster(poster.clone());
        let _ = agent.run(&prompt("hello there"), &Context::now()).await;
        let call = poster.last_call().expect("call recorded");
        let v: serde_json::Value =
            serde_json::from_slice(&call.body).expect("request body is valid JSON");
        assert_eq!(v["protocol"], "acp/1");
        assert_eq!(v["model"], "my-model");
        assert_eq!(v["prompt"]["role"], "user");
        assert_eq!(v["prompt"]["content"], "hello there");
    }

    #[tokio::test]
    async fn with_protocol_version_overrides_default() {
        let body = serde_json::json!({
            "messages": [{"role": "assistant", "content": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = CursorAgent::new("k", "m")
            .with_http_poster(poster.clone())
            .with_protocol_version("acp/2");
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let call = poster.last_call().expect("call recorded");
        let header_map: std::collections::HashMap<String, String> =
            call.headers.into_iter().collect();
        assert_eq!(
            header_map.get("x-cursor-protocol"),
            Some(&"acp/2".to_owned())
        );
        let v: serde_json::Value =
            serde_json::from_slice(&call.body).expect("request body is valid JSON");
        assert_eq!(v["protocol"], "acp/2");
    }

    #[tokio::test]
    async fn output_lineage_tracks_input() {
        let body = serde_json::json!({
            "messages": [{"role": "assistant", "content": "ok"}],
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
    async fn name_defaults_include_model_prefix() {
        let agent = CursorAgent::new("k", "cursor-composer");
        assert_eq!(agent.name(), "cursor:cursor-composer");
    }

    #[tokio::test]
    async fn supports_streaming_returns_true() {
        let agent = CursorAgent::new("k", "m");
        assert!(agent.supports_streaming());
    }

    #[tokio::test]
    async fn with_name_overrides_default() {
        let agent = CursorAgent::new("k", "m").with_name("my-cursor-agent");
        assert_eq!(agent.name(), "my-cursor-agent");
    }
}
