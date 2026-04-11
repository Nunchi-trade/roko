//! `CodexAgent` (§7.2) — `OpenAI` Codex backend.
//!
//! Codex's full wire protocol is JSON-RPC over stdio (local CLI) or over a
//! `WebSocket` app-server. See Mori's `AppServerConnection` at
//! `apps/mori/src/agent/connection.rs:1122-1791` for that flow.
//!
//! This file implements the **HTTPS fallback** path: Codex models are also
//! served through `OpenAI`'s `/v1/chat/completions` endpoint, so the wire
//! shape here is identical to [`crate::openai_agent::OpenAiAgent`]. A full
//! JSON-RPC app-server implementation lands in a later wave.
//!
//! # Design
//!
//! Like [`crate::claude_agent::ClaudeAgent`], this is a **library-layer**
//! agent. It never reads environment variables — callers (the CLI) inject
//! the API key explicitly. HTTP is routed through the shared
//! [`crate::http::HttpPoster`] trait so tests never touch the network.

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

/// Default `OpenAI` API base URL.
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com";

/// Default model slug used when the caller omits one.
pub const DEFAULT_MODEL: &str = "gpt-5-codex";

/// Maximum output tokens requested per call (default).
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

// ─── OpenAI Chat Completions wire types (minimal subset) ───────────────────

#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct ApiUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ChoiceMessage {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Choice {
    #[serde(default)]
    message: Option<ChoiceMessage>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ChatResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: ApiUsage,
}

#[derive(Debug, Serialize)]
struct RequestMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<RequestMessage<'a>>,
}

// ─── CodexAgent ────────────────────────────────────────────────────────────

/// An [`Agent`] that calls `OpenAI`'s Chat Completions API with Codex model
/// slugs (`gpt-5-codex`, `o1-codex`, …).
///
/// The agent is fully configurable; it never reads from the environment. The
/// API key is injected by the caller (typically the CLI layer). Use
/// [`CodexAgent::with_base_url`] to redirect requests to a test server.
///
/// # Example
///
/// ```ignore
/// use roko_agent::codex_agent::CodexAgent;
/// let agent = CodexAgent::new("sk-...", "gpt-5-codex");
/// ```
pub struct CodexAgent {
    api_key: String,
    model: String,
    name: String,
    base_url: String,
    timeout_ms: u64,
    max_tokens: u32,
    extra_headers: Vec<(String, String)>,
    poster: Arc<dyn HttpPoster>,
}

impl std::fmt::Debug for CodexAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexAgent")
            .field("model", &self.model)
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("timeout_ms", &self.timeout_ms)
            .field("max_tokens", &self.max_tokens)
            .finish_non_exhaustive()
    }
}

impl CodexAgent {
    /// Construct a new `CodexAgent` with the given API key and model slug.
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let model = model.into();
        let name = format!("codex:{model}");
        Self {
            api_key: api_key.into(),
            model,
            name,
            base_url: DEFAULT_BASE_URL.to_owned(),
            timeout_ms: 120_000,
            max_tokens: DEFAULT_MAX_TOKENS,
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
    /// `base_url` is prepended to `/v1/chat/completions`; trailing slashes
    /// are stripped.
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        let mut v = base_url.into();
        while v.ends_with('/') {
            v.pop();
        }
        self.base_url = v;
        self
    }

    /// Override the `max_tokens` limit sent on each request.
    #[must_use]
    pub const fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
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
        format!("{}/v1/chat/completions", self.base_url)
    }

    fn headers(&self) -> Vec<(String, String)> {
        let mut headers = vec![
            (
                "authorization".to_owned(),
                format!("Bearer {}", self.api_key),
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
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
impl Agent for CodexAgent {
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

        let req = ChatRequest {
            model: &self.model,
            max_tokens: self.max_tokens,
            messages: vec![RequestMessage {
                role: "user",
                content: &prompt_text,
            }],
        };
        let body = match serde_json::to_vec(&req) {
            Ok(v) => v,
            Err(e) => {
                return self.fail(input, &format!("serialize request failed: {e}"), started);
            }
        };

        let url = self.endpoint();
        let headers = self.headers();

        let response_text = match self
            .poster
            .post_json(&url, &headers, &body, self.timeout_ms)
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

        let parsed: ChatResponse = match serde_json::from_str(&response_text) {
            Ok(p) => p,
            Err(e) => {
                return self.fail(input, &format!("malformed response JSON: {e}"), started);
            }
        };

        let Some(first) = parsed.choices.first() else {
            return self.fail(input, "response contained no content", started);
        };
        let content = first
            .message
            .as_ref()
            .and_then(|m| m.content.as_ref())
            .map_or("", String::as_str);
        if content.is_empty() {
            return self.fail(input, "response contained no content", started);
        }

        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let usage = Usage {
            input_tokens: parsed.usage.prompt_tokens,
            output_tokens: parsed.usage.completion_tokens,
            cache_read_tokens: 0,
            cache_create_tokens: 0,
            cost_usd: 0.0,
            wall_ms,
        };

        let mut builder = input
            .derive(Kind::AgentOutput, Body::text(content))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("model", &self.model);
        if let Some(stop) = &first.finish_reason {
            builder = builder.tag("stop_reason", stop);
        }
        if let Some(id) = &parsed.id {
            builder = builder.tag("response_id", id);
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

    fn agent_with(poster: Arc<dyn HttpPoster>) -> CodexAgent {
        CodexAgent::new("test-key", "gpt-5-codex")
            .with_base_url("https://example.test")
            .with_http_poster(poster)
    }

    fn canned_ok(content: &str, prompt_tokens: u32, completion_tokens: u32) -> String {
        serde_json::json!({
            "id": "chatcmpl-abc",
            "model": "gpt-5-codex",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": content},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": prompt_tokens + completion_tokens
            }
        })
        .to_string()
    }

    #[tokio::test]
    async fn successful_response_populates_output_and_usage() {
        let poster = MockPoster::ok(canned_ok("hello world", 12, 34));
        let agent = agent_with(poster.clone());
        let result = agent.run(&prompt("hi"), &Context::now()).await;
        assert!(result.success);
        let text = result.output.body.as_text().unwrap_or("");
        assert_eq!(text, "hello world");
        assert_eq!(result.usage.input_tokens, 12);
        assert_eq!(result.usage.output_tokens, 34);
        assert_eq!(result.usage.cache_read_tokens, 0);
        assert_eq!(result.usage.cache_create_tokens, 0);
        assert_eq!(result.output.tag("model"), Some("gpt-5-codex"));
        assert_eq!(result.output.tag("stop_reason"), Some("stop"));
        assert_eq!(result.output.tag("response_id"), Some("chatcmpl-abc"));
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
    async fn missing_content_returns_failure() {
        // choices[0].message.content is absent.
        let body = serde_json::json!({
            "id": "chatcmpl-1",
            "choices": [{"index": 0, "message": {"role": "assistant"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 0, "total_tokens": 1}
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
                .contains("no content")
        );
    }

    #[tokio::test]
    async fn no_choices_returns_failure() {
        // Separate path: empty choices array.
        let body = serde_json::json!({
            "id": "chatcmpl-2",
            "choices": [],
            "usage": {"prompt_tokens": 1, "completion_tokens": 0, "total_tokens": 1}
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
                .contains("no content")
        );
    }

    #[tokio::test]
    async fn multiple_choices_uses_first() {
        let body = serde_json::json!({
            "id": "chatcmpl-multi",
            "choices": [
                {"index": 0, "message": {"role": "assistant", "content": "first"}, "finish_reason": "stop"},
                {"index": 1, "message": {"role": "assistant", "content": "second"}, "finish_reason": "stop"}
            ],
            "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
        })
        .to_string();
        let poster = MockPoster::ok(body);
        let agent = agent_with(poster);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap_or(""), "first");
    }

    #[tokio::test]
    async fn custom_base_url_is_used() {
        let poster = MockPoster::ok(canned_ok("ok", 1, 1));
        let agent = CodexAgent::new("k", "m")
            .with_base_url("https://custom.test/api/")
            .with_http_poster(poster.clone());
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let call = poster.last_call().expect("should have a recorded call");
        assert_eq!(call.url, "https://custom.test/api/v1/chat/completions");
    }

    #[tokio::test]
    async fn with_base_url_strips_trailing_slashes() {
        let agent = CodexAgent::new("k", "m").with_base_url("https://x.test///");
        assert_eq!(agent.base_url(), "https://x.test");
    }

    #[tokio::test]
    async fn headers_include_bearer_auth_token() {
        let poster = MockPoster::ok(canned_ok("ok", 1, 1));
        let agent =
            CodexAgent::new("sk-secret-xyz", "gpt-5-codex").with_http_poster(poster.clone());
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let call = poster.last_call().expect("call recorded");
        let auth = call
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .map(|(_, v)| v.clone())
            .expect("authorization header");
        assert_eq!(auth, "Bearer sk-secret-xyz");
        let ctype = call
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.clone())
            .expect("content-type header");
        assert_eq!(ctype, "application/json");
    }

    #[tokio::test]
    async fn timeout_ms_is_forwarded_to_poster() {
        let poster = MockPoster::ok(canned_ok("ok", 1, 1));
        let agent = CodexAgent::new("k", "m")
            .with_http_poster(poster.clone())
            .with_timeout_ms(42_000);
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let call = poster.last_call().expect("call recorded");
        assert_eq!(call.timeout_ms, 42_000);
    }

    #[tokio::test]
    async fn extra_headers_are_included_in_request() {
        let poster = MockPoster::ok(canned_ok("ok", 1, 1));
        let mut extra_headers = HashMap::new();
        extra_headers.insert("HTTP-Referer".to_string(), "roko-agent".to_string());
        extra_headers.insert("X-Title".to_string(), "roko".to_string());

        let agent = CodexAgent::new("k", "m")
            .with_http_poster(poster.clone())
            .with_extra_headers(extra_headers);
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let call = poster.last_call().expect("call recorded");
        let header_map: std::collections::HashMap<String, String> = call.headers.into_iter().collect();
        assert_eq!(header_map.get("HTTP-Referer"), Some(&"roko-agent".to_string()));
        assert_eq!(header_map.get("X-Title"), Some(&"roko".to_string()));
        assert_eq!(
            header_map.get("authorization"),
            Some(&"Bearer k".to_string())
        );
    }

    #[tokio::test]
    async fn request_body_contains_model_and_max_tokens() {
        let poster = MockPoster::ok(canned_ok("ok", 1, 1));
        let agent = CodexAgent::new("k", "my-codex-model")
            .with_http_poster(poster.clone())
            .with_max_tokens(256);
        let _ = agent.run(&prompt("hello there"), &Context::now()).await;
        let call = poster.last_call().expect("call recorded");
        let v: serde_json::Value =
            serde_json::from_slice(&call.body).expect("request body is valid JSON");
        assert_eq!(v["model"], "my-codex-model");
        assert_eq!(v["max_tokens"], 256);
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["messages"][0]["content"], "hello there");
    }

    #[tokio::test]
    async fn output_lineage_tracks_input() {
        let poster = MockPoster::ok(canned_ok("ok", 1, 1));
        let agent = agent_with(poster);
        let input = prompt("track me");
        let input_id = input.id;
        let result = agent.run(&input, &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.lineage, vec![input_id]);
    }

    #[tokio::test]
    async fn name_defaults_include_model_prefix() {
        let agent = CodexAgent::new("k", "gpt-5-codex");
        assert_eq!(agent.name(), "codex:gpt-5-codex");
    }

    #[tokio::test]
    async fn supports_streaming_returns_true() {
        let agent = CodexAgent::new("k", "gpt-5-codex");
        assert!(agent.supports_streaming());
    }

    #[tokio::test]
    async fn with_name_overrides_default() {
        let agent = CodexAgent::new("k", "m").with_name("my-codex");
        assert_eq!(agent.name(), "my-codex");
    }
}
