//! `OllamaAgent` — local LLM backend via the Ollama HTTP API.
//!
//! Posts to `{base_url}/api/chat` with a non-streaming request body and
//! parses the single response JSON into an [`AgentResult`].
//!
//! # Design
//!
//! This agent is deliberately synchronous-style (`stream: false`): the caller
//! gets the complete assistant message back as a single JSON document. This
//! keeps the implementation small and testable, and avoids the need for an
//! NDJSON parser in this specific entry-point module.
//!
//! HTTP I/O is hidden behind the shared [`crate::http::HttpPoster`] trait so
//! tests can inject a mock client without a real network dependency.

use crate::agent::{Agent, AgentResult};
#[cfg(test)]
use crate::http::HttpPostError;
use crate::http::{HttpPoster, ReqwestPoster};
use crate::usage::Usage;
use async_trait::async_trait;
use roko_core::{Body, Context, Kind, Provenance, Signal};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Default Ollama server URL (local installations listen here by default).
const DEFAULT_BASE_URL: &str = "http://localhost:11434";

/// Default per-turn timeout — local inference is slow, so we're generous.
const DEFAULT_TIMEOUT_MS: u64 = 180_000;

/// An agent that talks to a local Ollama server over HTTP.
///
/// # Example
///
/// ```ignore
/// let agent = OllamaAgent::new("llama3.1:8b");
/// let prompt = Signal::builder(Kind::Prompt).body(Body::text("Hi")).build();
/// let result = agent.run(&prompt, &Context::now()).await;
/// ```
pub struct OllamaAgent {
    base_url: String,
    model: String,
    timeout_ms: u64,
    name: String,
}

impl OllamaAgent {
    /// Construct an [`OllamaAgent`] for `model` against the default base URL
    /// (`http://localhost:11434`) and default timeout (180 s).
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        let model = model.into();
        let name = format!("ollama:{model}");
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            model,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            name,
        }
    }

    /// Override the Ollama server base URL (e.g. `http://192.168.1.10:11434`).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Override the per-turn timeout in milliseconds.
    #[must_use]
    pub const fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Model slug this agent targets (e.g. `llama3.1:8b`).
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Base URL this agent posts to.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Configured timeout in milliseconds.
    #[must_use]
    pub const fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    /// Execute one chat turn against the given poster. Shared by the real
    /// `Agent::run` and the unit tests.
    async fn run_with_poster(
        &self,
        poster: &dyn HttpPoster,
        input: &Signal,
    ) -> AgentResult {
        let started = Instant::now();

        let prompt_text = match input.body.as_text() {
            Ok(s) => s.to_string(),
            Err(_) => match serde_json::to_string(&input.body) {
                Ok(s) => s,
                Err(e) => {
                    return self.failure_signal(
                        input,
                        &format!("input body not readable as text or json: {e}"),
                        started,
                    );
                }
            },
        };

        let request = ChatRequest {
            model: &self.model,
            stream: false,
            messages: vec![ChatMessage {
                role: "user",
                content: &prompt_text,
            }],
        };

        let body = match serde_json::to_string(&request) {
            Ok(b) => b,
            Err(e) => {
                return self.failure_signal(
                    input,
                    &format!("failed to serialize request: {e}"),
                    started,
                );
            }
        };

        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));
        let raw = match poster
            .post_json(&url, &[], body.as_bytes(), self.timeout_ms)
            .await
        {
            Ok(raw) => raw,
            Err(e) => {
                return self.failure_signal(
                    input,
                    &format!("http error: {e}"),
                    started,
                );
            }
        };

        let resp: ChatResponse = match serde_json::from_str(&raw) {
            Ok(r) => r,
            Err(e) => {
                return self.failure_signal(
                    input,
                    &format!("failed to parse ollama response: {e}"),
                    started,
                );
            }
        };

        let message = resp.message.unwrap_or_default();
        let content = message.content;
        if content.is_empty() {
            return self.failure_signal(
                input,
                "ollama returned empty assistant content",
                started,
            );
        }

        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        let output = input
            .derive(Kind::AgentOutput, Body::text(content))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("model", &self.model)
            .build();

        let usage = Usage {
            input_tokens: resp.prompt_eval_count.unwrap_or(0),
            output_tokens: resp.eval_count.unwrap_or(0),
            wall_ms,
            ..Default::default()
        };

        AgentResult::ok(output).with_usage(usage)
    }

    fn failure_signal(&self, input: &Signal, reason: &str, started: Instant) -> AgentResult {
        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let output = input
            .derive(Kind::AgentOutput, Body::text(reason))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("model", &self.model)
            .tag("failed", "true")
            .build();
        AgentResult::fail(output).with_usage(Usage {
            wall_ms,
            ..Default::default()
        })
    }
}

#[async_trait]
impl Agent for OllamaAgent {
    async fn run(&self, input: &Signal, _ctx: &Context) -> AgentResult {
        let poster = ReqwestPoster::new();
        self.run_with_poster(&poster, input).await
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn supports_streaming(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    stream: bool,
    messages: Vec<ChatMessage<'a>>,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    message: Option<ChatResponseMessage>,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Deserialize, Default)]
struct ChatResponseMessage {
    #[serde(default)]
    #[allow(dead_code)]
    role: String,
    #[serde(default)]
    content: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Records every request and replays a canned response (or error).
    struct MockPoster {
        response: Result<String, HttpPostError>,
        last_url: Arc<Mutex<Option<String>>>,
        last_body: Arc<Mutex<Option<Vec<u8>>>>,
    }

    impl MockPoster {
        fn ok(body: impl Into<String>) -> Self {
            Self {
                response: Ok(body.into()),
                last_url: Arc::new(Mutex::new(None)),
                last_body: Arc::new(Mutex::new(None)),
            }
        }

        fn err(msg: impl Into<String>) -> Self {
            Self {
                response: Err(HttpPostError::transport(msg)),
                last_url: Arc::new(Mutex::new(None)),
                last_body: Arc::new(Mutex::new(None)),
            }
        }

        fn last_url(&self) -> Option<String> {
            self.last_url.lock().unwrap().clone()
        }

        fn last_body(&self) -> Option<String> {
            self.last_body
                .lock()
                .unwrap()
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).into_owned())
        }
    }

    #[async_trait]
    impl HttpPoster for MockPoster {
        async fn post_json(
            &self,
            url: &str,
            _headers: &[(String, String)],
            body: &[u8],
            _timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            *self.last_url.lock().unwrap() = Some(url.to_string());
            *self.last_body.lock().unwrap() = Some(body.to_vec());
            self.response.clone()
        }
    }

    fn prompt(text: &str) -> Signal {
        Signal::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    fn canned_ok_response() -> String {
        r#"{
            "model": "llama3.1:8b",
            "message": {"role": "assistant", "content": "Hello from Ollama."},
            "done": true,
            "prompt_eval_count": 12,
            "eval_count": 34
        }"#
        .to_string()
    }

    #[tokio::test]
    async fn successful_response_produces_output_signal() {
        let agent = OllamaAgent::new("llama3.1:8b");
        let poster = MockPoster::ok(canned_ok_response());
        let result = agent.run_with_poster(&poster, &prompt("hi")).await;
        assert!(result.success);
        assert_eq!(
            result.output.body.as_text().unwrap(),
            "Hello from Ollama."
        );
        assert_eq!(result.output.kind, Kind::AgentOutput);
        assert_eq!(result.output.tag("model"), Some("llama3.1:8b"));
    }

    #[tokio::test]
    async fn http_error_becomes_failed_result() {
        let agent = OllamaAgent::new("llama3.1:8b");
        let poster = MockPoster::err("connection refused");
        let result = agent.run_with_poster(&poster, &prompt("hi")).await;
        assert!(!result.success);
        let body = result.output.body.as_text().unwrap();
        assert!(body.contains("http error"), "got: {body}");
        assert!(body.contains("connection refused"), "got: {body}");
        // And a transport-error tag in the human message.
        assert!(body.contains("transport error"), "got: {body}");
        assert_eq!(result.output.tag("failed"), Some("true"));
    }

    #[tokio::test]
    async fn malformed_json_fails_gracefully() {
        let agent = OllamaAgent::new("llama3.1:8b");
        let poster = MockPoster::ok("this is not json {{{");
        let result = agent.run_with_poster(&poster, &prompt("hi")).await;
        assert!(!result.success);
        assert!(result
            .output
            .body
            .as_text()
            .unwrap()
            .contains("failed to parse ollama response"));
    }

    #[tokio::test]
    async fn empty_content_marks_result_failed() {
        let agent = OllamaAgent::new("llama3.1:8b");
        let poster = MockPoster::ok(
            r#"{"message":{"role":"assistant","content":""},"done":true}"#,
        );
        let result = agent.run_with_poster(&poster, &prompt("hi")).await;
        assert!(!result.success);
        assert!(result
            .output
            .body
            .as_text()
            .unwrap()
            .contains("empty assistant content"));
    }

    #[tokio::test]
    async fn usage_fields_come_from_eval_counts() {
        let agent = OllamaAgent::new("llama3.1:8b");
        let poster = MockPoster::ok(canned_ok_response());
        let result = agent.run_with_poster(&poster, &prompt("hi")).await;
        assert!(result.success);
        assert_eq!(result.usage.input_tokens, 12);
        assert_eq!(result.usage.output_tokens, 34);
    }

    #[tokio::test]
    async fn missing_counts_default_to_zero() {
        let agent = OllamaAgent::new("llama3.1:8b");
        // No prompt_eval_count / eval_count fields.
        let poster = MockPoster::ok(
            r#"{"message":{"role":"assistant","content":"ok"},"done":true}"#,
        );
        let result = agent.run_with_poster(&poster, &prompt("hi")).await;
        assert!(result.success);
        assert_eq!(result.usage.input_tokens, 0);
        assert_eq!(result.usage.output_tokens, 0);
    }

    #[tokio::test]
    async fn custom_base_url_is_used() {
        let agent = OllamaAgent::new("qwen2.5-coder:7b")
            .with_base_url("http://ollama.internal:11434");
        let poster = MockPoster::ok(canned_ok_response());
        let _ = agent.run_with_poster(&poster, &prompt("hi")).await;
        assert_eq!(
            poster.last_url().as_deref(),
            Some("http://ollama.internal:11434/api/chat")
        );
    }

    #[tokio::test]
    async fn trailing_slash_in_base_url_is_normalized() {
        let agent =
            OllamaAgent::new("llama3.1:8b").with_base_url("http://host.local:11434/");
        let poster = MockPoster::ok(canned_ok_response());
        let _ = agent.run_with_poster(&poster, &prompt("hi")).await;
        assert_eq!(
            poster.last_url().as_deref(),
            Some("http://host.local:11434/api/chat")
        );
    }

    #[tokio::test]
    async fn request_body_enforces_stream_false() {
        let agent = OllamaAgent::new("llama3.1:8b");
        let poster = MockPoster::ok(canned_ok_response());
        let _ = agent.run_with_poster(&poster, &prompt("hi")).await;
        let body = poster.last_body().expect("body captured");
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["stream"], serde_json::Value::Bool(false));
        assert_eq!(parsed["model"], "llama3.1:8b");
        assert_eq!(parsed["messages"][0]["role"], "user");
        assert_eq!(parsed["messages"][0]["content"], "hi");
    }

    #[tokio::test]
    async fn default_url_is_localhost_11434() {
        let agent = OllamaAgent::new("llama3.1:8b");
        assert_eq!(agent.base_url(), "http://localhost:11434");
        let poster = MockPoster::ok(canned_ok_response());
        let _ = agent.run_with_poster(&poster, &prompt("hi")).await;
        assert_eq!(
            poster.last_url().as_deref(),
            Some("http://localhost:11434/api/chat")
        );
    }

    #[tokio::test]
    async fn builder_defaults_and_overrides() {
        let agent = OllamaAgent::new("mistral:7b");
        assert_eq!(agent.model(), "mistral:7b");
        assert_eq!(agent.base_url(), "http://localhost:11434");
        assert_eq!(agent.timeout_ms(), 180_000);
        assert_eq!(agent.name(), "ollama:mistral:7b");

        let agent = agent
            .with_base_url("http://remote:11434")
            .with_timeout_ms(5_000);
        assert_eq!(agent.base_url(), "http://remote:11434");
        assert_eq!(agent.timeout_ms(), 5_000);
    }

    #[tokio::test]
    async fn does_not_advertise_streaming() {
        let agent = OllamaAgent::new("llama3.1:8b");
        assert!(!agent.supports_streaming());
    }

    #[tokio::test]
    async fn output_lineage_tracks_input() {
        let agent = OllamaAgent::new("llama3.1:8b");
        let input = prompt("lineage test");
        let input_id = input.id;
        let poster = MockPoster::ok(canned_ok_response());
        let result = agent.run_with_poster(&poster, &input).await;
        assert!(result.success);
        assert_eq!(result.output.lineage, vec![input_id]);
    }

    #[tokio::test]
    async fn failure_signal_is_tagged_and_carries_model() {
        let agent = OllamaAgent::new("llama3.1:8b");
        let poster = MockPoster::err("boom");
        let result = agent.run_with_poster(&poster, &prompt("x")).await;
        assert!(!result.success);
        assert_eq!(result.output.tag("failed"), Some("true"));
        assert_eq!(result.output.tag("model"), Some("llama3.1:8b"));
        assert_eq!(result.output.tag("agent"), Some("ollama:llama3.1:8b"));
    }
}
