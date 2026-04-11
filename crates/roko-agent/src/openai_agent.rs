//! `OpenAiAgent` — talks to any `OpenAI`-compatible `/chat/completions` endpoint.
//!
//! This is a minimal, synchronous (non-streaming) implementation of the
//! `OpenAI` Chat Completions API. It POSTs a JSON body containing the model
//! and a list of `{role, content}` messages and parses a single response.
//!
//! HTTP is pulled behind the shared [`crate::http::HttpPoster`] trait so tests
//! can inject a mock without making real network calls. Production code uses
//! [`crate::http::ReqwestPoster`].
//!
//! This module deliberately keeps the wire surface tight: it emits a single
//! [`Kind::AgentOutput`] signal and a [`Usage`] record built from the
//! `usage` object in the response. A full SSE-streaming implementation is
//! tracked separately — see the `agent-openai` spec.

#![allow(clippy::too_many_lines)]

use crate::agent::{Agent, AgentResult};
#[cfg(test)]
use crate::http::HttpPostError;
use crate::http::{HttpPoster, ReqwestPoster};
use crate::translate::openai::parse_usage;
use crate::usage::Usage;
use async_trait::async_trait;
use roko_core::{Body, Context, Kind, Provenance, Signal};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Instant;

/// Default `OpenAI` base URL.
const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Default per-request timeout in milliseconds.
const DEFAULT_TIMEOUT_MS: u64 = 120_000;

/// A non-streaming `OpenAI` chat-completions backend.
///
/// # Example
///
/// ```ignore
/// let agent = OpenAiAgent::new("sk-test-key", "gpt-4o-mini");
/// let prompt = Signal::builder(Kind::Prompt)
///     .body(Body::text("Say hi"))
///     .build();
/// let result = agent.run(&prompt, &Context::now()).await;
/// assert!(result.success);
/// ```
pub struct OpenAiAgent {
    api_key: String,
    model: String,
    base_url: String,
    timeout_ms: u64,
    name: String,
    extra_headers: Vec<(String, String)>,
    poster: Box<dyn HttpPoster>,
}

impl OpenAiAgent {
    /// Construct an agent with the production reqwest-backed HTTP poster.
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let model = model.into();
        let name = format!("openai:{model}");
        Self {
            api_key: api_key.into(),
            model,
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            name,
            extra_headers: Vec::new(),
            poster: Box::new(ReqwestPoster::new()),
        }
    }

    /// Override the base URL (e.g. `http://localhost:8000/v1` for vLLM).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Override the per-request timeout (default 120 s).
    #[must_use]
    pub const fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
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

    /// Internal constructor used by tests to inject a mock poster.
    #[cfg(test)]
    fn with_poster(mut self, poster: Box<dyn HttpPoster>) -> Self {
        self.poster = poster;
        self
    }

    fn endpoint(&self) -> String {
        let trimmed = self.base_url.trim_end_matches('/');
        format!("{trimmed}/chat/completions")
    }

    fn headers(&self) -> Vec<(String, String)> {
        let mut headers = vec![
            (
                "Authorization".to_string(),
                format!("Bearer {}", self.api_key),
            ),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];
        headers.extend(self.extra_headers.iter().cloned());
        headers
    }

    fn failure(&self, input: &Signal, reason: String, started: &Instant) -> AgentResult {
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
impl Agent for OpenAiAgent {
    async fn run(&self, input: &Signal, _ctx: &Context) -> AgentResult {
        let started = Instant::now();

        // Extract prompt text (JSON fallback for non-text bodies).
        let prompt_text = match input.body.as_text() {
            Ok(s) => s.to_string(),
            Err(_) => match serde_json::to_string(&input.body) {
                Ok(s) => s,
                Err(e) => {
                    return self.failure(
                        input,
                        format!("input body not readable as text or json: {e}"),
                        &started,
                    );
                }
            },
        };

        // Build the request body.
        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "user", "content": prompt_text }
            ]
        });
        let body_bytes = match serde_json::to_vec(&body) {
            Ok(v) => v,
            Err(e) => {
                return self.failure(input, format!("request serialize failed: {e}"), &started);
            }
        };

        let url = self.endpoint();
        let headers = self.headers();

        // POST via the injected http poster; the poster enforces the timeout.
        let response_text = match self
            .poster
            .post_json(&url, &headers, &body_bytes, self.timeout_ms)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                return self.failure(input, format!("http error: {e}"), &started);
            }
        };

        // Parse JSON.
        let parsed: Value = match serde_json::from_str(&response_text) {
            Ok(v) => v,
            Err(e) => {
                return self.failure(input, format!("malformed response json: {e}"), &started);
            }
        };

        // Upstream errors: `{"error": {...}}`.
        if let Some(err) = parsed.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown api error");
            return self.failure(input, format!("api error: {msg}"), &started);
        }

        // Extract the first choice's message content.
        let content = parsed
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str);
        let content = match content {
            Some(c) => c.to_string(),
            None => {
                return self.failure(
                    input,
                    "response missing choices[0].message.content".to_string(),
                    &started,
                );
            }
        };

        // Pull usage if present.
        let usage = parse_usage(&parsed);

        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        let out_signal = input
            .derive(Kind::AgentOutput, Body::text(&content))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("model", &self.model)
            .build();

        AgentResult::ok(out_signal).with_usage(Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            wall_ms,
            ..Default::default()
        })
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn supports_streaming(&self) -> bool {
        false
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_types)] // tests use std::sync::Mutex for simplicity
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Captured state for a single call to the mock poster.
    #[derive(Clone, Debug, Default)]
    struct Captured {
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    /// Test-only poster that returns a canned response and captures the
    /// request so assertions can inspect it.
    struct MockPoster {
        captured: Arc<Mutex<Option<Captured>>>,
        response: Result<String, HttpPostError>,
    }

    impl MockPoster {
        fn ok(body: impl Into<String>) -> (Self, Arc<Mutex<Option<Captured>>>) {
            let captured = Arc::new(Mutex::new(None));
            (
                Self {
                    captured: captured.clone(),
                    response: Ok(body.into()),
                },
                captured,
            )
        }

        fn err(msg: impl Into<String>) -> (Self, Arc<Mutex<Option<Captured>>>) {
            let captured = Arc::new(Mutex::new(None));
            // Parse leading "http NNN:" if present so the mock carries
            // the same structured status as the real poster would.
            let m: String = msg.into();
            let err = m
                .strip_prefix("http ")
                .and_then(|rest| {
                    let (code, tail) = rest.split_once(':')?;
                    let code: u16 = code.trim().parse().ok()?;
                    Some(HttpPostError::http(code, tail.trim_start()))
                })
                .unwrap_or_else(|| HttpPostError::transport(m));
            (
                Self {
                    captured: captured.clone(),
                    response: Err(err),
                },
                captured,
            )
        }
    }

    #[async_trait]
    impl HttpPoster for MockPoster {
        async fn post_json(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &[u8],
            _timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            *self.captured.lock().expect("lock mock captured") = Some(Captured {
                url: url.to_string(),
                headers: headers.to_vec(),
                body: body.to_vec(),
            });
            self.response.clone()
        }
    }

    fn prompt(text: &str) -> Signal {
        Signal::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    fn agent_with(api_key: &str, model: &str, poster: Box<dyn HttpPoster>) -> OpenAiAgent {
        OpenAiAgent::new(api_key, model).with_poster(poster)
    }

    fn canned_ok(content: &str, prompt_tokens: u32, completion_tokens: u32) -> String {
        serde_json::json!({
            "id": "chatcmpl-test",
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
    async fn successful_response_produces_agent_output() {
        let (mock, _) = MockPoster::ok(canned_ok("hello world", 10, 5));
        let agent = agent_with("sk-test", "gpt-4o-mini", Box::new(mock));
        let result = agent.run(&prompt("hi"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.kind, Kind::AgentOutput);
        assert_eq!(
            result.output.body.as_text().expect("text body"),
            "hello world"
        );
        assert_eq!(result.output.tag("model"), Some("gpt-4o-mini"));
    }

    #[tokio::test]
    async fn usage_fields_are_parsed_from_response() {
        let (mock, _) = MockPoster::ok(canned_ok("ok", 42, 17));
        let agent = agent_with("sk-test", "gpt-4o", Box::new(mock));
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.usage.input_tokens, 42);
        assert_eq!(result.usage.output_tokens, 17);
    }

    #[tokio::test]
    async fn http_401_is_reported_as_failure() {
        let (mock, _) = MockPoster::err("http 401: {\"error\":{\"message\":\"bad key\"}}");
        let agent = agent_with("sk-bad", "gpt-4o-mini", Box::new(mock));
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert_eq!(result.output.tag("failed"), Some("true"));
        assert!(
            result
                .output
                .body
                .as_text()
                .expect("text body")
                .contains("401"),
            "expected 401 in failure message"
        );
    }

    #[tokio::test]
    async fn http_429_rate_limit_is_reported_as_failure() {
        let (mock, _) = MockPoster::err("http 429: rate limited");
        let agent = agent_with("sk-ok", "gpt-4o-mini", Box::new(mock));
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .expect("text body")
                .contains("429")
        );
    }

    #[tokio::test]
    async fn malformed_json_fails_gracefully() {
        let (mock, _) = MockPoster::ok("not { valid json");
        let agent = agent_with("sk-test", "gpt-4o-mini", Box::new(mock));
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .expect("text body")
                .contains("malformed")
        );
    }

    #[tokio::test]
    async fn missing_choices_is_a_failure() {
        let body =
            serde_json::json!({ "id": "x", "usage": {"prompt_tokens":0,"completion_tokens":0}})
                .to_string();
        let (mock, _) = MockPoster::ok(body);
        let agent = agent_with("sk-test", "gpt-4o-mini", Box::new(mock));
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .expect("text body")
                .contains("missing choices")
        );
    }

    #[tokio::test]
    async fn empty_content_still_succeeds() {
        let (mock, _) = MockPoster::ok(canned_ok("", 3, 0));
        let agent = agent_with("sk-test", "gpt-4o-mini", Box::new(mock));
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().expect("text body"), "");
        assert_eq!(result.usage.output_tokens, 0);
        assert_eq!(result.usage.input_tokens, 3);
    }

    #[tokio::test]
    async fn custom_base_url_is_respected() {
        let (mock, captured) = MockPoster::ok(canned_ok("ok", 1, 1));
        let agent = OpenAiAgent::new("sk-x", "local-model")
            .with_base_url("http://localhost:8000/v1")
            .with_poster(Box::new(mock));
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let c = captured.lock().expect("lock").clone().expect("captured");
        assert_eq!(c.url, "http://localhost:8000/v1/chat/completions");
    }

    #[tokio::test]
    async fn trailing_slash_base_url_is_normalized() {
        let (mock, captured) = MockPoster::ok(canned_ok("ok", 1, 1));
        let agent = OpenAiAgent::new("sk-x", "m")
            .with_base_url("http://host/v1/")
            .with_poster(Box::new(mock));
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let c = captured.lock().expect("lock").clone().expect("captured");
        assert_eq!(c.url, "http://host/v1/chat/completions");
    }

    #[tokio::test]
    async fn bearer_header_is_set() {
        let (mock, captured) = MockPoster::ok(canned_ok("ok", 0, 0));
        let agent = agent_with("sk-secret-xyz", "gpt-4o-mini", Box::new(mock));
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let c = captured.lock().expect("lock").clone().expect("captured");
        let auth = c
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("Authorization"))
            .map(|(_, v)| v.clone())
            .expect("authorization header");
        assert_eq!(auth, "Bearer sk-secret-xyz");
    }

    #[tokio::test]
    async fn request_body_contains_model_and_messages() {
        let (mock, captured) = MockPoster::ok(canned_ok("ok", 0, 0));
        let agent = agent_with("sk-x", "gpt-test", Box::new(mock));
        let _ = agent.run(&prompt("please explain"), &Context::now()).await;
        let c = captured.lock().expect("lock").clone().expect("captured");
        let body: Value = serde_json::from_slice(&c.body).expect("body is json");
        assert_eq!(body["model"], "gpt-test");
        let msgs = body["messages"].as_array().expect("messages array");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], "please explain");
    }

    #[tokio::test]
    async fn api_error_object_becomes_failure() {
        let body = serde_json::json!({
            "error": {"message": "invalid api key", "type": "auth"}
        })
        .to_string();
        let (mock, _) = MockPoster::ok(body);
        let agent = agent_with("sk-bad", "gpt-4o-mini", Box::new(mock));
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .expect("text body")
                .contains("invalid api key")
        );
    }

    #[tokio::test]
    async fn timeout_triggers_failure() {
        // Poster simulates the shared ReqwestPoster's behaviour: wait up to
        // `timeout_ms` and then surface a transport error if we overran.
        struct SlowPoster;
        #[async_trait]
        impl HttpPoster for SlowPoster {
            async fn post_json(
                &self,
                _url: &str,
                _headers: &[(String, String)],
                _body: &[u8],
                timeout_ms: u64,
            ) -> Result<String, HttpPostError> {
                tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)).await;
                Err(HttpPostError::transport(format!(
                    "timed out after {timeout_ms} ms"
                )))
            }
        }
        let agent = OpenAiAgent::new("sk-x", "m")
            .with_timeout_ms(10)
            .with_poster(Box::new(SlowPoster));
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .expect("text body")
                .contains("timed out")
        );
    }

    #[tokio::test]
    async fn extra_headers_are_included_in_request() {
        let (mock, captured) = MockPoster::ok(canned_ok("ok", 1, 1));
        let mut extra_headers = HashMap::new();
        extra_headers.insert("HTTP-Referer".to_string(), "roko-agent".to_string());
        extra_headers.insert("X-Title".to_string(), "roko".to_string());

        let agent = OpenAiAgent::new("sk-x", "m")
            .with_extra_headers(extra_headers)
            .with_poster(Box::new(mock));
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let c = captured.lock().expect("lock").clone().expect("captured");
        let header_map: std::collections::HashMap<String, String> = c.headers.into_iter().collect();
        assert_eq!(
            header_map.get("HTTP-Referer"),
            Some(&"roko-agent".to_string())
        );
        assert_eq!(header_map.get("X-Title"), Some(&"roko".to_string()));
        assert_eq!(
            header_map.get("Authorization"),
            Some(&"Bearer sk-x".to_string())
        );
    }

    #[tokio::test]
    async fn output_signal_derives_lineage_from_input() {
        let (mock, _) = MockPoster::ok(canned_ok("reply", 1, 1));
        let agent = agent_with("sk-x", "m", Box::new(mock));
        let input = prompt("trace me");
        let input_id = input.id;
        let result = agent.run(&input, &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.lineage, vec![input_id]);
    }

    #[test]
    fn endpoint_uses_default_base_url() {
        let agent = OpenAiAgent::new("k", "m");
        assert_eq!(
            agent.endpoint(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn name_includes_model() {
        let agent = OpenAiAgent::new("k", "gpt-foo");
        assert_eq!(agent.name(), "openai:gpt-foo");
        assert!(!agent.supports_streaming());
    }
}
