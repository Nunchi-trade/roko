//! `OpenAiCompatLlmBackend` — HTTP adapter implementing [`LlmBackend`]
//! for OpenAI-compatible chat-completions endpoints.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value};
use tokio::sync::mpsc;

use crate::http::{HttpPoster, ReqwestPoster};
use crate::rate_limit::ProviderRateLimiter;
use crate::streaming::{StreamAccumulator, StreamChunk, parse_sse_line};
use crate::tool_loop::{LlmBackend, LlmError};
use crate::translate::FinishReason;
use crate::translate::{BackendResponse, RenderedTools, SessionState};
use crate::usage::Usage;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_PROVIDER_RPM: u32 = 60;

#[derive(Debug, Clone, Default)]
struct StreamResponseMetadata {
    response_id: Option<String>,
    session_id: Option<String>,
    thread_id: Option<String>,
}

fn shared_rate_limiter() -> Arc<ProviderRateLimiter> {
    static SHARED_RATE_LIMITER: OnceLock<Arc<ProviderRateLimiter>> = OnceLock::new();
    Arc::clone(
        SHARED_RATE_LIMITER
            .get_or_init(|| Arc::new(ProviderRateLimiter::new(DEFAULT_PROVIDER_RPM))),
    )
}

fn compute_headers(api_key: &str, extra_headers: &[(String, String)]) -> Vec<(String, String)> {
    let mut headers = Vec::with_capacity(2 + extra_headers.len());
    headers.push(("Content-Type".to_string(), "application/json".to_string()));
    if !api_key.is_empty() {
        headers.push(("Authorization".to_string(), format!("Bearer {api_key}")));
    }
    headers.extend(extra_headers.iter().cloned());
    headers
}

/// HTTP adapter for OpenAI-compatible `/chat/completions` endpoints.
pub struct OpenAiCompatLlmBackend {
    api_key: String,
    model: String,
    provider_id: String,
    base_url: String,
    timeout_ms: u64,
    max_tokens: Option<u32>,
    extra_headers: Vec<(String, String)>,
    extra_body_params: Map<String, Value>,
    rate_limiter: Arc<ProviderRateLimiter>,
    poster: Box<dyn HttpPoster>,
    /// Pre-computed HTTP headers (Content-Type + Auth + extras).
    computed_headers: Vec<(String, String)>,
    /// When true, omit `session_id`, `thread_id`, and `conversation_id` from
    /// request bodies. Strict OpenAI-compatible providers (e.g. Cerebras)
    /// reject unknown top-level fields.
    skip_session_fields: bool,
    /// When true, include `"parallel_tool_calls": false` in request bodies.
    /// Small models (e.g. Llama 3.1 8B via Cerebras) cannot reliably handle
    /// multiple tool calls in a single turn.
    disable_parallel_tool_calls: bool,
    /// When true, normalize `content: ""` to `content: null` on assistant
    /// messages that carry `tool_calls`. Strict providers (e.g. Cerebras)
    /// reject empty-string content alongside tool calls.
    normalize_tool_call_content: bool,
}

impl OpenAiCompatLlmBackend {
    /// Construct a backend for `model` with default URL and timeout.
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let model = model.into();
        let api_key = api_key.into();
        let computed_headers = compute_headers(&api_key, &[]);
        Self {
            api_key,
            provider_id: model.clone(),
            model,
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_tokens: None,
            extra_headers: Vec::new(),
            extra_body_params: Map::new(),
            rate_limiter: shared_rate_limiter(),
            poster: Box::new(ReqwestPoster::new()),
            computed_headers,
            skip_session_fields: false,
            disable_parallel_tool_calls: false,
            normalize_tool_call_content: false,
        }
    }

    /// Override the provider identifier used for request throttling.
    #[must_use]
    pub fn with_provider_id(mut self, provider_id: impl Into<String>) -> Self {
        self.provider_id = provider_id.into();
        self
    }

    /// Override the provider base URL.
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

    /// Override the max output tokens sent on every request.
    #[must_use]
    pub const fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Inject additional HTTP headers on every request.
    #[must_use]
    pub fn with_extra_headers(mut self, extra_headers: HashMap<String, String>) -> Self {
        let mut extra_headers: Vec<(String, String)> = extra_headers.into_iter().collect();
        extra_headers.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        self.extra_headers = extra_headers;
        self.computed_headers = compute_headers(&self.api_key, &self.extra_headers);
        self
    }

    /// Inject additional JSON fields into the outbound request body.
    #[must_use]
    pub fn with_extra_body_params(mut self, extra_body_params: Map<String, Value>) -> Self {
        self.extra_body_params = extra_body_params;
        self
    }

    /// Override the shared provider rate limiter.
    #[must_use]
    pub fn with_rate_limiter(mut self, rate_limiter: Arc<ProviderRateLimiter>) -> Self {
        self.rate_limiter = rate_limiter;
        self
    }

    /// Inject a custom HTTP poster (for tests or alternate transports).
    #[must_use]
    pub fn with_poster(mut self, poster: Box<dyn HttpPoster>) -> Self {
        self.poster = poster;
        self
    }

    /// Skip `session_id`, `thread_id`, and `conversation_id` in request bodies.
    ///
    /// Strict OpenAI-compatible providers (e.g. Cerebras) reject unknown
    /// top-level fields. Enable this for providers that only accept the
    /// standard OpenAI Chat Completions schema.
    #[must_use]
    pub const fn with_skip_session_fields(mut self, skip: bool) -> Self {
        self.skip_session_fields = skip;
        self
    }

    /// Include `"parallel_tool_calls": false` in request bodies.
    ///
    /// Small models that cannot reliably handle multiple tool calls in a
    /// single turn should have this enabled to constrain the backend to
    /// emitting at most one tool call per response.
    #[must_use]
    pub const fn with_disable_parallel_tool_calls(mut self, disable: bool) -> Self {
        self.disable_parallel_tool_calls = disable;
        self
    }

    /// Normalize `content: ""` to `content: null` on assistant messages
    /// that carry `tool_calls`. Strict providers (e.g. Cerebras) reject
    /// empty-string content alongside tool calls.
    #[must_use]
    pub const fn with_normalize_tool_call_content(mut self, normalize: bool) -> Self {
        self.normalize_tool_call_content = normalize;
        self
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    /// Return a clone of the cached request headers.
    fn headers(&self) -> Vec<(String, String)> {
        self.computed_headers.clone()
    }

    fn build_body(
        &self,
        messages: &[serde_json::Value],
        tools: &RenderedTools,
        session: &SessionState,
        stream: bool,
    ) -> Result<Vec<u8>, LlmError> {
        let RenderedTools::JsonArray(tools) = tools else {
            return Err(LlmError::Backend("expected json tool array".into()));
        };
        // Optionally normalize assistant messages for strict providers.
        let messages = if self.normalize_tool_call_content {
            let mut msgs = messages.to_vec();
            for msg in &mut msgs {
                if msg.get("role").and_then(Value::as_str) == Some("assistant")
                    && msg.get("tool_calls").is_some_and(Value::is_array)
                {
                    if let Some(content) = msg.get("content") {
                        if content.as_str().is_some_and(str::is_empty) || content.is_null() {
                            if let Some(obj) = msg.as_object_mut() {
                                obj.insert("content".to_string(), Value::Null);
                            }
                        }
                    }
                }
            }
            std::borrow::Cow::Owned(msgs)
        } else {
            std::borrow::Cow::Borrowed(messages)
        };

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": *messages,
            "tools": tools,
        });

        if let Some(body_obj) = body.as_object_mut() {
            if let Some(max_tokens) = self.max_tokens {
                body_obj.insert("max_tokens".to_string(), Value::from(max_tokens));
            }
            if !self.skip_session_fields {
                if let Some(session_id) = &session.session_id {
                    body_obj.insert("session_id".to_string(), Value::String(session_id.clone()));
                }
                if let Some(thread_id) = &session.thread_id {
                    body_obj.insert("thread_id".to_string(), Value::String(thread_id.clone()));
                }
                if let Some(conversation_id) = &session.conversation_id {
                    body_obj.insert(
                        "conversation_id".to_string(),
                        Value::String(conversation_id.clone()),
                    );
                }
            }
            if self.disable_parallel_tool_calls {
                body_obj.insert("parallel_tool_calls".to_string(), Value::Bool(false));
            }
            if stream {
                body_obj.insert("stream".to_string(), Value::Bool(true));
            }
            for (key, value) in &self.extra_body_params {
                body_obj.insert(key.clone(), value.clone());
            }
        }

        serde_json::to_vec(&body).map_err(|e| LlmError::Backend(format!("serialize: {e}")))
    }

    fn push_stream_line(
        line: &[u8],
        accumulator: &mut StreamAccumulator,
        event_tx: &mpsc::UnboundedSender<StreamChunk>,
    ) {
        let line = String::from_utf8_lossy(line);
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(chunk) = parse_sse_line(line) {
            accumulator.push(chunk.clone());
            let _ = event_tx.send(chunk);
        }
    }

    fn capture_stream_metadata(line: &[u8], metadata: &mut StreamResponseMetadata) {
        let line = String::from_utf8_lossy(line);
        let line = line.trim_end_matches(['\r', '\n']);
        let Some(line) = line.strip_prefix("data:").map(str::trim_start) else {
            return;
        };
        if line == "[DONE]" {
            return;
        }

        let Ok(json) = serde_json::from_str::<Value>(line) else {
            return;
        };

        if let Some(response_id) = json.get("id").and_then(Value::as_str) {
            metadata.response_id = Some(response_id.to_string());
        }
        if let Some(session_id) = json.get("session_id").and_then(Value::as_str) {
            metadata.session_id = Some(session_id.to_string());
        }
        if let Some(thread_id) = json.get("thread_id").and_then(Value::as_str) {
            metadata.thread_id = Some(thread_id.to_string());
        }
    }

    fn stream_response_to_json(
        response: crate::chat_types::ChatResponse,
        metadata: StreamResponseMetadata,
    ) -> Result<Value, LlmError> {
        let message = response
            .raw_assistant_message
            .clone()
            .unwrap_or_else(|| response.as_assistant_message());
        let message = serde_json::to_value(message)
            .map_err(|e| LlmError::Backend(format!("serialize streamed response: {e}")))?;

        let mut json = serde_json::json!({
            "choices": [{
                "index": 0,
                "message": message,
                "finish_reason": finish_reason_to_wire(&response.finish_reason),
            }],
            "usage": usage_to_wire(&response.usage),
        });
        if let Some(body) = json.as_object_mut() {
            if let Some(response_id) = metadata.response_id {
                body.insert("id".to_string(), Value::String(response_id));
            }
            if let Some(session_id) = metadata.session_id {
                body.insert("session_id".to_string(), Value::String(session_id));
            }
            if let Some(thread_id) = metadata.thread_id {
                body.insert("thread_id".to_string(), Value::String(thread_id));
            }
        }

        Ok(json)
    }
}

fn extract_session(response: &Value) -> SessionState {
    SessionState {
        session_id: response
            .pointer("/session_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        thread_id: response
            .pointer("/thread_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        conversation_id: response
            .pointer("/id")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

#[async_trait]
impl LlmBackend for OpenAiCompatLlmBackend {
    async fn send_turn(
        &self,
        messages: &[serde_json::Value],
        tools: &RenderedTools,
        session: &SessionState,
    ) -> Result<BackendResponse, LlmError> {
        let body_bytes = self.build_body(messages, tools, session, false)?;
        self.rate_limiter.acquire(&self.provider_id).await;

        let raw = self
            .poster
            .post_json(
                &self.endpoint(),
                &self.computed_headers,
                &body_bytes,
                self.timeout_ms,
            )
            .await
            .map_err(|e| LlmError::Network(e.to_string()))?;

        let json: Value = serde_json::from_str(&raw)
            .map_err(|e| LlmError::Backend(format!("parse response: {e}")))?;

        Ok(BackendResponse::Json(json))
    }

    async fn send_turn_streaming(
        &self,
        messages: &[serde_json::Value],
        tools: &RenderedTools,
        session: &SessionState,
        event_tx: mpsc::UnboundedSender<StreamChunk>,
    ) -> Result<BackendResponse, LlmError> {
        let body_bytes = self.build_body(messages, tools, session, true)?;
        self.rate_limiter.acquire(&self.provider_id).await;

        let mut req = crate::provider::shared_http_client()
            .post(self.endpoint())
            .timeout(Duration::from_millis(self.timeout_ms));
        for (key, value) in &self.computed_headers {
            req = req.header(key.as_str(), value.as_str());
        }

        let response = req.body(body_bytes).send().await.map_err(|e| {
            let message = format!("request failed: {e}");
            let _ = event_tx.send(StreamChunk::Error(message.clone()));
            LlmError::Network(message)
        })?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.map_err(|e| {
                let message = format!("read body failed: {e}");
                let _ = event_tx.send(StreamChunk::Error(message.clone()));
                LlmError::Network(message)
            })?;
            let message = crate::http::HttpPostError::http(status.as_u16(), text).to_string();
            let _ = event_tx.send(StreamChunk::Error(message.clone()));
            return Err(LlmError::Network(message));
        }

        let mut response = response;
        let mut pending = Vec::new();
        let mut accumulator = StreamAccumulator::new();
        let mut metadata = StreamResponseMetadata::default();

        loop {
            let chunk = response.chunk().await.map_err(|e| {
                let message = format!("read chunk failed: {e}");
                let _ = event_tx.send(StreamChunk::Error(message.clone()));
                LlmError::Network(message)
            })?;
            let Some(chunk) = chunk else {
                break;
            };

            pending.extend_from_slice(&chunk);
            while let Some(newline_idx) = pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = pending.drain(..=newline_idx).collect();
                Self::capture_stream_metadata(&line, &mut metadata);
                Self::push_stream_line(&line, &mut accumulator, &event_tx);
            }
        }

        if !pending.is_empty() {
            Self::capture_stream_metadata(&pending, &mut metadata);
            Self::push_stream_line(&pending, &mut accumulator, &event_tx);
        }

        let json = Self::stream_response_to_json(accumulator.finalize(), metadata)?;
        Ok(BackendResponse::Json(json))
    }

    fn extract_session(&self, response: &BackendResponse) -> SessionState {
        match response {
            BackendResponse::Json(json) => extract_session(json),
            BackendResponse::StreamJson(_) | BackendResponse::Text(_) => SessionState::default(),
        }
    }

    fn backend_id(&self) -> &'static str {
        "openai_compat"
    }
}

fn finish_reason_to_wire(finish_reason: &FinishReason) -> String {
    match finish_reason {
        FinishReason::Stop => "stop".to_string(),
        FinishReason::Length => "length".to_string(),
        FinishReason::ToolCalls => "tool_calls".to_string(),
        FinishReason::ContentFilter => "content_filter".to_string(),
        FinishReason::Error(reason) => reason.clone(),
    }
}

fn usage_to_wire(usage: &Usage) -> Value {
    serde_json::json!({
        "prompt_tokens": usage.input_tokens,
        "completion_tokens": usage.output_tokens,
        "total_tokens": usage.input_tokens + usage.output_tokens,
        "prompt_tokens_details": {
            "cached_tokens": usage.cache_read_tokens,
        },
    })
}

impl std::fmt::Debug for OpenAiCompatLlmBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatLlmBackend")
            .field("model", &self.model)
            .field("provider_id", &self.provider_id)
            .field("base_url", &self.base_url)
            .field("timeout_ms", &self.timeout_ms)
            .field("max_tokens", &self.max_tokens)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration as StdDuration;
    use std::time::Instant;

    use crate::dispatcher::{HandlerResolver, ToolDispatcher};
    use crate::tool_loop::{StopReason, ToolLoop};
    use crate::translate::{OpenAiTranslator, Translator};
    use roko_core::tool::{
        ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolHandler, ToolPermission,
        ToolResult, VecToolRegistry,
    };
    use tokio::time::{Duration, timeout};

    use crate::http::HttpPostError;

    fn test_timeout(ms: u64) -> Duration {
        let scaled = if std::env::var("CI").map(|v| v == "true").unwrap_or(false) {
            ms * 10
        } else {
            ms
        };
        Duration::from_millis(scaled)
    }

    #[derive(Clone, Debug)]
    struct RecordedRequest {
        url: String,
        headers: Vec<(String, String)>,
        body: Value,
        timeout_ms: u64,
    }

    struct MockPoster {
        responses: Mutex<VecDeque<Result<String, HttpPostError>>>,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
    }

    impl MockPoster {
        fn new(
            responses: Vec<Result<String, HttpPostError>>,
        ) -> (Self, Arc<Mutex<Vec<RecordedRequest>>>) {
            let requests = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    responses: Mutex::new(responses.into_iter().collect()),
                    requests: requests.clone(),
                },
                requests,
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
            timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            let body: Value = serde_json::from_slice(body).expect("request body must be json");
            self.requests
                .lock()
                .expect("requests lock")
                .push(RecordedRequest {
                    url: url.to_string(),
                    headers: headers.to_vec(),
                    body,
                    timeout_ms,
                });

            self.responses
                .lock()
                .expect("responses lock")
                .pop_front()
                .expect("mock response queued")
        }
    }

    struct EchoHandler;

    #[async_trait]
    impl ToolHandler for EchoHandler {
        fn name(&self) -> &str {
            "echo"
        }

        async fn execute(&self, call: ToolCall, _ctx: &ToolContext) -> ToolResult {
            ToolResult::text(call.arguments.to_string())
        }
    }

    fn test_tools() -> Vec<ToolDef> {
        vec![
            ToolDef::new(
                "echo",
                "echo args",
                ToolCategory::Meta,
                ToolPermission::read_only(),
            )
            .with_concurrency(ToolConcurrency::Parallel),
        ]
    }

    fn make_tool_loop(backend: OpenAiCompatLlmBackend) -> ToolLoop {
        let registry: Arc<dyn roko_core::tool::ToolRegistry> =
            Arc::new(VecToolRegistry::from_tools(test_tools()));
        let resolver: Arc<dyn HandlerResolver> =
            Arc::new(|name: &str| -> Option<Arc<dyn ToolHandler>> {
                if name == "echo" {
                    Some(Arc::new(EchoHandler) as Arc<dyn ToolHandler>)
                } else {
                    None
                }
            });
        let dispatcher = Arc::new(ToolDispatcher::new(registry, resolver));
        let translator: Arc<dyn Translator> = Arc::new(OpenAiTranslator);
        ToolLoop::new(translator, dispatcher, Arc::new(backend))
    }

    fn read_http_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(StdDuration::from_secs(2)))
            .expect("set read timeout");

        let mut buf = Vec::new();
        let mut chunk = [0_u8; 1024];
        let header_end = loop {
            let read = stream.read(&mut chunk).expect("read request bytes");
            assert!(read > 0, "request closed before headers completed");
            buf.extend_from_slice(&chunk[..read]);

            if let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
                break pos + 4;
            }
        };

        let headers = String::from_utf8_lossy(&buf[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);

        while buf.len() < header_end + content_length {
            let read = stream.read(&mut chunk).expect("read request body");
            assert!(read > 0, "request closed before body completed");
            buf.extend_from_slice(&chunk[..read]);
        }

        String::from_utf8(buf).expect("request should be valid utf8")
    }

    fn sse_json_line(value: Value) -> String {
        format!("data: {value}\n\n")
    }

    #[tokio::test]
    async fn openai_compat_backend_posts_expected_request() {
        let (poster, requests) = MockPoster::new(vec![Ok(serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "done"
                }
            }]
        })
        .to_string())]);
        let mut extra_body_params = Map::new();
        extra_body_params.insert(
            "thinking".to_string(),
            serde_json::json!({ "type": "enabled" }),
        );
        let backend = OpenAiCompatLlmBackend::new("test-key", "glm-5.1")
            .with_base_url("https://api.z.ai/api/paas/v4/")
            .with_timeout_ms(90_000)
            .with_extra_headers(HashMap::from([(
                "X-Test-Header".to_string(),
                "present".to_string(),
            )]))
            .with_extra_body_params(extra_body_params)
            .with_poster(Box::new(poster));

        let response = backend
            .send_turn(
                &[serde_json::json!({ "role": "user", "content": "hi" })],
                &RenderedTools::JsonArray(serde_json::json!([{
                    "type": "function",
                    "function": {
                        "name": "echo",
                        "description": "echo args",
                        "parameters": {
                            "type": "object",
                            "properties": {}
                        }
                    }
                }])),
                &SessionState::default(),
            )
            .await
            .expect("send turn");
        assert!(matches!(response, BackendResponse::Json(_)));

        let requests = requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].url,
            "https://api.z.ai/api/paas/v4/chat/completions"
        );
        assert_eq!(requests[0].timeout_ms, 90_000);
        assert!(
            requests[0].headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("authorization") && value == "Bearer test-key"
            }),
            "expected authorization header"
        );
        assert!(
            requests[0].headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("x-test-header") && value == "present"
            }),
            "expected extra header"
        );
        assert_eq!(requests[0].body["model"], "glm-5.1");
        assert_eq!(requests[0].body["thinking"]["type"], "enabled");
        assert_eq!(requests[0].body["tools"][0]["function"]["name"], "echo");
    }

    #[test]
    fn headers_are_cached_and_recomputed_for_extra_headers() {
        let backend = OpenAiCompatLlmBackend::new("test-key", "glm-5.1");

        let first = backend.headers();
        let second = backend.headers();

        assert_eq!(
            first,
            vec![
                ("Content-Type".to_string(), "application/json".to_string()),
                ("Authorization".to_string(), "Bearer test-key".to_string()),
            ]
        );
        assert_eq!(first, second);

        let updated = backend
            .with_extra_headers(HashMap::from([
                ("X-B".to_string(), "2".to_string()),
                ("X-A".to_string(), "1".to_string()),
            ]))
            .headers();

        assert_eq!(
            updated,
            vec![
                ("Content-Type".to_string(), "application/json".to_string()),
                ("Authorization".to_string(), "Bearer test-key".to_string()),
                ("X-A".to_string(), "1".to_string()),
                ("X-B".to_string(), "2".to_string()),
            ]
        );
    }

    struct HeaderPointerPoster {
        responses: Mutex<VecDeque<Result<String, HttpPostError>>>,
        auth_ptrs: Arc<Mutex<Vec<usize>>>,
    }

    impl HeaderPointerPoster {
        fn new(
            responses: Vec<Result<String, HttpPostError>>,
        ) -> (Self, Arc<Mutex<Vec<usize>>>) {
            let auth_ptrs = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    responses: Mutex::new(responses.into_iter().collect()),
                    auth_ptrs: auth_ptrs.clone(),
                },
                auth_ptrs,
            )
        }
    }

    #[async_trait]
    impl HttpPoster for HeaderPointerPoster {
        async fn post_json(
            &self,
            _url: &str,
            headers: &[(String, String)],
            _body: &[u8],
            _timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            let auth_ptr = headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .map(|(_, value)| value.as_ptr() as usize)
                .expect("authorization header");
            self.auth_ptrs
                .lock()
                .expect("auth ptr lock")
                .push(auth_ptr);
            self.responses
                .lock()
                .expect("responses lock")
                .pop_front()
                .expect("mock response queued")
        }
    }

    #[tokio::test]
    async fn request_path_reuses_cached_authorization_header() {
        let (poster, auth_ptrs) = HeaderPointerPoster::new(vec![
            Ok(serde_json::json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "done"
                    }
                }]
            })
            .to_string()),
            Ok(serde_json::json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "done"
                    }
                }]
            })
            .to_string()),
        ]);
        let backend = OpenAiCompatLlmBackend::new("test-key", "glm-5.1")
            .with_poster(Box::new(poster));
        let messages = [serde_json::json!({ "role": "user", "content": "hi" })];
        let tools = RenderedTools::JsonArray(serde_json::json!([]));
        let session = SessionState::default();

        backend
            .send_turn(&messages, &tools, &session)
            .await
            .expect("first send turn");
        backend
            .send_turn(&messages, &tools, &session)
            .await
            .expect("second send turn");

        let auth_ptrs = auth_ptrs.lock().expect("auth ptr lock");
        assert_eq!(auth_ptrs.len(), 2);
        assert_eq!(auth_ptrs[0], auth_ptrs[1]);
    }

    #[tokio::test]
    async fn rate_limited_backend_spreads_same_provider_requests() {
        let limiter = Arc::new(ProviderRateLimiter::new_per_second(1));
        let (poster_a, requests_a) = MockPoster::new(vec![Ok(serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "done"
                }
            }]
        })
        .to_string())]);
        let (poster_b, requests_b) = MockPoster::new(vec![Ok(serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "done"
                }
            }]
        })
        .to_string())]);

        let backend_a = OpenAiCompatLlmBackend::new("test-key", "glm-5.1")
            .with_provider_id("zai")
            .with_rate_limiter(limiter.clone())
            .with_poster(Box::new(poster_a));
        let backend_b = OpenAiCompatLlmBackend::new("test-key", "glm-5.1")
            .with_provider_id("zai")
            .with_rate_limiter(limiter)
            .with_poster(Box::new(poster_b));
        let messages = [serde_json::json!({ "role": "user", "content": "hi" })];
        let tools = RenderedTools::JsonArray(serde_json::json!([]));
        let session = SessionState::default();

        let start = Instant::now();
        let (response_a, response_b) = tokio::join!(
            backend_a.send_turn(&messages, &tools, &session),
            backend_b.send_turn(&messages, &tools, &session),
        );
        let elapsed = start.elapsed();

        assert!(matches!(response_a, Ok(BackendResponse::Json(_))));
        assert!(matches!(response_b, Ok(BackendResponse::Json(_))));
        assert!(
            elapsed >= StdDuration::from_millis(900),
            "same-provider requests should be throttled, got {elapsed:?}"
        );
        assert!(
            elapsed < StdDuration::from_secs(3),
            "test limiter should finish promptly, got {elapsed:?}"
        );
        assert_eq!(requests_a.lock().expect("requests lock").len(), 1);
        assert_eq!(requests_b.lock().expect("requests lock").len(), 1);
    }

    #[test]
    fn session_extraction_reads_openai_compat_response_ids() {
        let backend = OpenAiCompatLlmBackend::new("test-key", "glm-5.1");
        let response = BackendResponse::Json(serde_json::json!({
            "id": "chatcmpl-glm",
            "session_id": "sess-glm",
            "thread_id": "thread-glm",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "done"
                }
            }]
        }));

        let session = backend.extract_session(&response);

        assert_eq!(session.conversation_id.as_deref(), Some("chatcmpl-glm"));
        assert_eq!(session.session_id.as_deref(), Some("sess-glm"));
        assert_eq!(session.thread_id.as_deref(), Some("thread-glm"));
    }

    #[test]
    fn session_extraction_defaults_when_ids_are_absent() {
        let backend = OpenAiCompatLlmBackend::new("test-key", "kimi-k2.5");
        let response = BackendResponse::Json(serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "done"
                }
            }]
        }));

        let session = backend.extract_session(&response);

        assert!(session.session_id.is_none());
        assert!(session.thread_id.is_none());
        assert!(session.conversation_id.is_none());
    }

    #[tokio::test]
    async fn tool_loop_basic() {
        let first_response = serde_json::json!({
            "id": "chatcmpl-1",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {
                            "name": "echo",
                            "arguments": serde_json::json!({ "value": 1 }).to_string()
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        })
        .to_string();
        let second_response = serde_json::json!({
            "id": "chatcmpl-2",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "final answer"
                },
                "finish_reason": "stop"
            }]
        })
        .to_string();
        let (poster, requests) = MockPoster::new(vec![Ok(first_response), Ok(second_response)]);
        let backend = OpenAiCompatLlmBackend::new("test-key", "glm-5.1")
            .with_base_url("https://api.z.ai/api/paas/v4")
            .with_poster(Box::new(poster));
        let tool_loop = make_tool_loop(backend);
        let ctx = ToolContext::testing("/tmp");

        let result = tool_loop
            .run(
                "system prompt",
                "call the tool, then finish",
                &test_tools(),
                &ctx,
            )
            .await;

        assert_eq!(result.stop_reason, StopReason::Stop);
        assert_eq!(result.iterations, 1);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id, "call-1");
        assert_eq!(result.tool_calls[0].name, "echo");
        assert_eq!(result.final_text, "final answer");

        let requests = requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 2, "expected two HTTP turns");
        let second_turn_messages = requests[1]
            .body
            .get("messages")
            .and_then(Value::as_array)
            .expect("second turn messages");
        let assistant_message = second_turn_messages
            .iter()
            .find(|msg| msg["role"] == "assistant")
            .expect("assistant tool-call message");
        assert_eq!(assistant_message["tool_calls"][0]["id"], "call-1");
        let tool_message = second_turn_messages
            .iter()
            .find(|msg| msg.get("tool_call_id").is_some())
            .expect("tool result message");
        assert_eq!(tool_message["tool_call_id"], "call-1");
        assert_eq!(tool_message["content"], "{\"value\":1}");
    }

    #[tokio::test]
    async fn streaming_tool_loop_emits_chunks_and_matches_final_result() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("listener addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_server = requests.clone();

        let server = thread::spawn(move || {
            let responses = [
                vec![
                    sse_json_line(serde_json::json!({
                        "choices": [{
                            "delta": {
                                "reasoning_content": "Need to inspect the args. "
                            }
                        }]
                    })),
                    sse_json_line(serde_json::json!({
                        "choices": [{
                            "delta": {
                                "tool_calls": [{
                                    "index": 0,
                                    "id": "call-1",
                                    "function": {
                                        "name": "echo",
                                        "arguments": "{\"value\":"
                                    }
                                }]
                            }
                        }]
                    })),
                    sse_json_line(serde_json::json!({
                        "choices": [{
                            "delta": {
                                "tool_calls": [{
                                    "index": 0,
                                    "function": {
                                        "arguments": "1}"
                                    }
                                }]
                            }
                        }]
                    })),
                    sse_json_line(serde_json::json!({
                        "choices": [{
                            "delta": {},
                            "finish_reason": "tool_calls"
                        }]
                    })),
                    sse_json_line(serde_json::json!({
                        "usage": {
                            "prompt_tokens": 11,
                            "completion_tokens": 7,
                            "prompt_tokens_details": {
                                "cached_tokens": 3
                            }
                        }
                    })),
                    "data: [DONE]\n\n".to_string(),
                ],
                vec![
                    sse_json_line(serde_json::json!({
                        "choices": [{
                            "delta": {
                                "content": "final "
                            }
                        }]
                    })),
                    sse_json_line(serde_json::json!({
                        "choices": [{
                            "delta": {
                                "content": "answer"
                            }
                        }]
                    })),
                    sse_json_line(serde_json::json!({
                        "choices": [{
                            "delta": {},
                            "finish_reason": "stop"
                        }]
                    })),
                    sse_json_line(serde_json::json!({
                        "usage": {
                            "prompt_tokens": 9,
                            "completion_tokens": 4
                        }
                    })),
                    "data: [DONE]\n\n".to_string(),
                ],
            ];

            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept request");
                let request = read_http_request(&mut stream);
                requests_for_server
                    .lock()
                    .expect("requests lock")
                    .push(request);

                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
                )
                .expect("write response headers");
                stream.flush().expect("flush response headers");

                for chunk in response {
                    stream
                        .write_all(chunk.as_bytes())
                        .expect("write response chunk");
                    stream.flush().expect("flush response chunk");
                    thread::sleep(StdDuration::from_millis(40));
                }
            }
        });

        let backend = OpenAiCompatLlmBackend::new("test-key", "glm-5.1")
            .with_base_url(format!("http://{addr}"))
            .with_timeout_ms(5_000);
        let tool_loop = make_tool_loop(backend);
        let tools = test_tools();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let run = tokio::spawn(async move {
            let ctx = ToolContext::testing("/tmp");
            tool_loop
                .run_streaming(
                    "system prompt",
                    "call the tool, then finish",
                    &tools,
                    &ctx,
                    event_tx,
                )
                .await
        });

        let first_chunk = timeout(test_timeout(500), event_rx.recv())
            .await
            .expect("stream should emit before completion")
            .expect("stream channel open");
        assert!(matches!(first_chunk, StreamChunk::ReasoningDelta(_)));
        assert!(
            !run.is_finished(),
            "streaming chunks should arrive before the tool loop finishes"
        );

        let result = run.await.expect("tool loop task");
        let mut chunks = vec![first_chunk];
        while let Some(chunk) = event_rx.recv().await {
            chunks.push(chunk);
        }

        assert_eq!(result.stop_reason, StopReason::Stop);
        assert_eq!(result.iterations, 1);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id, "call-1");
        assert_eq!(result.tool_calls[0].name, "echo");
        assert_eq!(result.final_text, "final answer");
        assert_eq!(result.total_usage.input_tokens, 20);
        assert_eq!(result.total_usage.output_tokens, 11);
        assert_eq!(result.total_usage.cache_read_tokens, 3);

        assert!(chunks.iter().any(|chunk| matches!(
            chunk,
            StreamChunk::ToolCallDelta {
                index: 0,
                id_delta: Some(id),
                name_delta: Some(name),
                arguments_delta,
            } if id == "call-1" && name == "echo" && arguments_delta == "{\"value\":"
        )));
        assert!(chunks.iter().any(|chunk| {
            matches!(chunk, StreamChunk::ContentDelta(content) if content == "final ")
        }));
        assert!(
            chunks
                .iter()
                .any(|chunk| { matches!(chunk, StreamChunk::Done(FinishReason::ToolCalls)) })
        );

        let requests = requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 2, "expected two streamed HTTP turns");
        assert!(
            requests
                .iter()
                .all(|request| request.contains("\"stream\":true"))
        );
        assert!(requests[1].contains("\"tool_call_id\":\"call-1\""));
        assert!(requests[1].contains("\"arguments\":\"{\\\"value\\\":1}\""));

        server.join().expect("server thread");
    }

    #[tokio::test]
    async fn openai_compat_backend_requires_json_tools() {
        let (poster, _) = MockPoster::new(vec![]);
        let backend = OpenAiCompatLlmBackend::new("", "test-model").with_poster(Box::new(poster));

        let err = backend
            .send_turn(
                &[serde_json::json!({ "role": "user", "content": "hi" })],
                &RenderedTools::CliFlag("echo".to_string()),
                &SessionState::default(),
            )
            .await
            .expect_err("non-json tools should fail");

        assert!(matches!(err, LlmError::Backend(_)));
    }

    #[test]
    fn debug_impl_mentions_model() {
        let backend = OpenAiCompatLlmBackend::new("test-key", "glm-5.1");
        let s = format!("{backend:?}");
        assert!(s.contains("OpenAiCompatLlmBackend"));
        assert!(s.contains("glm-5.1"));
    }

    /// Live integration test — sends a real request to OpenAI to verify the
    /// request body is well-formed and the model parameter is accepted.
    ///
    /// Requires `OPENAI_API_KEY` in the environment. Skipped otherwise.
    #[tokio::test]
    async fn live_openai_gpt4o_accepts_request() {
        let Ok(api_key) = std::env::var("OPENAI_API_KEY") else {
            eprintln!("skipping: OPENAI_API_KEY not set");
            return;
        };

        let backend = OpenAiCompatLlmBackend::new(api_key, "gpt-4o")
            .with_max_tokens(10);
        let result = backend
            .send_turn(
                &[serde_json::json!({ "role": "user", "content": "say hi" })],
                &RenderedTools::JsonArray(serde_json::json!([])),
                &SessionState::default(),
            )
            .await;

        match &result {
            Ok(BackendResponse::Json(json)) => {
                assert!(
                    json.pointer("/choices/0/message/content").is_some(),
                    "expected choices[0].message.content in response: {json}"
                );
            }
            Ok(other) => panic!("unexpected response variant: {other:?}"),
            Err(e) => panic!("request failed: {e:?}"),
        }
    }
}
