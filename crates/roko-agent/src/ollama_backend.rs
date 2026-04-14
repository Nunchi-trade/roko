//! `OllamaLlmBackend` — HTTP adapter implementing [`LlmBackend`] for Ollama.
//!
//! Always sets `stream: false` (M21: Ollama silently drops tool calls in streaming mode).

use async_trait::async_trait;
use std::sync::Arc;

use crate::cache::{ResponseCache, request_hash, shared_response_cache};
use crate::http::{HttpPoster, ReqwestPoster};
use crate::tool_loop::{LlmBackend, LlmError};
use crate::translate::{BackendResponse, RenderedTools, SessionState};

const DEFAULT_BASE_URL: &str = "http://localhost:11434";
const DEFAULT_TIMEOUT_MS: u64 = 180_000;

/// HTTP adapter for Ollama's `/api/chat` endpoint, implementing [`LlmBackend`].
pub struct OllamaLlmBackend {
    model: String,
    base_url: String,
    timeout_ms: u64,
    poster: Box<dyn HttpPoster>,
    response_cache: Option<Arc<ResponseCache>>,
}

impl OllamaLlmBackend {
    /// Construct a backend for `model` with default URL and timeout.
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            poster: Box::new(ReqwestPoster::new()),
            response_cache: Some(shared_response_cache()),
        }
    }

    /// Override the Ollama server base URL.
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

    /// Inject a custom HTTP poster (for tests).
    #[must_use]
    pub fn with_poster(mut self, poster: Box<dyn HttpPoster>) -> Self {
        self.poster = poster;
        self
    }

    /// Override the response cache used for identical request payloads.
    #[must_use]
    pub fn with_response_cache(mut self, response_cache: Arc<ResponseCache>) -> Self {
        self.response_cache = Some(response_cache);
        self
    }

    /// Disable content-addressed response caching for this backend instance.
    #[must_use]
    pub fn without_response_cache(mut self) -> Self {
        self.response_cache = None;
        self
    }

    async fn execute_request(
        &self,
        url: &str,
        body_bytes: &[u8],
    ) -> Result<BackendResponse, LlmError> {
        let raw = self
            .poster
            .post_json(url, &[], body_bytes, self.timeout_ms)
            .await
            .map_err(|e| LlmError::Network(e.to_string()))?;

        let json: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| LlmError::Backend(format!("parse response: {e}")))?;

        Ok(BackendResponse::Json(json))
    }
}

#[async_trait]
impl LlmBackend for OllamaLlmBackend {
    async fn send_turn(
        &self,
        messages: &[serde_json::Value],
        tools: &RenderedTools,
        _session: &SessionState,
    ) -> Result<BackendResponse, LlmError> {
        let tools_value = match tools {
            RenderedTools::JsonArray(arr) => arr.clone(),
            _ => serde_json::json!([]),
        };

        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "tools": tools_value,
            "stream": false,
        });

        let body_bytes =
            serde_json::to_vec(&body).map_err(|e| LlmError::Backend(format!("serialize: {e}")))?;

        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));
        if let Some(response_cache) = &self.response_cache {
            let prompt_hash = request_hash("ollama", &url, &body_bytes);
            response_cache
                .get_or_compute(prompt_hash, || async {
                    self.execute_request(&url, &body_bytes).await
                })
                .await
        } else {
            self.execute_request(&url, &body_bytes).await
        }
    }
}

impl std::fmt::Debug for OllamaLlmBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OllamaLlmBackend")
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("timeout_ms", &self.timeout_ms)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpPostError;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct MockPoster {
        response: Result<String, HttpPostError>,
        last_url: Arc<Mutex<Option<String>>>,
        last_body: Arc<Mutex<Option<Vec<u8>>>>,
        call_count: Arc<AtomicUsize>,
    }

    impl MockPoster {
        fn ok(body: impl Into<String>) -> Self {
            Self {
                response: Ok(body.into()),
                last_url: Arc::new(Mutex::new(None)),
                last_body: Arc::new(Mutex::new(None)),
                call_count: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn err(msg: impl Into<String>) -> Self {
            Self {
                response: Err(HttpPostError::transport(msg)),
                last_url: Arc::new(Mutex::new(None)),
                last_body: Arc::new(Mutex::new(None)),
                call_count: Arc::new(AtomicUsize::new(0)),
            }
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
            self.call_count.fetch_add(1, Ordering::SeqCst);
            *self.last_url.lock().unwrap() = Some(url.to_string());
            *self.last_body.lock().unwrap() = Some(body.to_vec());
            self.response.clone()
        }
    }

    fn canned_response() -> String {
        r#"{"message":{"role":"assistant","content":"Hello!"},"done":true}"#.to_string()
    }

    #[tokio::test]
    async fn send_turn_posts_correct_url() {
        let poster = MockPoster::ok(canned_response());
        let url_ref = poster.last_url.clone();
        let backend = OllamaLlmBackend::new("gemma4:26b")
            .with_base_url("http://myhost:11434")
            .without_response_cache()
            .with_poster(Box::new(poster));

        let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let tools = RenderedTools::JsonArray(serde_json::json!([]));
        let _ = backend
            .send_turn(&msgs, &tools, &SessionState::default())
            .await;

        assert_eq!(
            url_ref.lock().unwrap().as_deref(),
            Some("http://myhost:11434/api/chat")
        );
    }

    #[tokio::test]
    async fn send_turn_enforces_stream_false() {
        let poster = MockPoster::ok(canned_response());
        let body_ref = poster.last_body.clone();
        let backend = OllamaLlmBackend::new("gemma4:26b")
            .without_response_cache()
            .with_poster(Box::new(poster));

        let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let tools = RenderedTools::JsonArray(serde_json::json!([]));
        let _ = backend
            .send_turn(&msgs, &tools, &SessionState::default())
            .await;

        let body: serde_json::Value =
            serde_json::from_slice(body_ref.lock().unwrap().as_ref().unwrap()).unwrap();
        assert_eq!(body["stream"], false);
        assert_eq!(body["model"], "gemma4:26b");
    }

    #[tokio::test]
    async fn send_turn_returns_json_response() {
        let poster = MockPoster::ok(
            r#"{"message":{"role":"assistant","content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"read_file","arguments":{"path":"x"}}}]}}"#,
        );
        let backend = OllamaLlmBackend::new("m")
            .without_response_cache()
            .with_poster(Box::new(poster));

        let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let tools = RenderedTools::JsonArray(serde_json::json!([]));
        let result = backend
            .send_turn(&msgs, &tools, &SessionState::default())
            .await
            .unwrap();
        match result {
            BackendResponse::Json(v) => assert!(v["message"]["tool_calls"].is_array()),
            other => panic!("expected Json, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_turn_network_error() {
        let poster = MockPoster::err("connection refused");
        let backend = OllamaLlmBackend::new("m")
            .without_response_cache()
            .with_poster(Box::new(poster));

        let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let tools = RenderedTools::JsonArray(serde_json::json!([]));
        let err = backend
            .send_turn(&msgs, &tools, &SessionState::default())
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::Network(_)));
    }

    #[tokio::test]
    async fn send_turn_malformed_json() {
        let poster = MockPoster::ok("not json {{{");
        let backend = OllamaLlmBackend::new("m")
            .without_response_cache()
            .with_poster(Box::new(poster));

        let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let tools = RenderedTools::JsonArray(serde_json::json!([]));
        let err = backend
            .send_turn(&msgs, &tools, &SessionState::default())
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::Backend(_)));
    }

    #[tokio::test]
    async fn trailing_slash_normalized() {
        let poster = MockPoster::ok(canned_response());
        let url_ref = poster.last_url.clone();
        let backend = OllamaLlmBackend::new("m")
            .with_base_url("http://h:11434/")
            .without_response_cache()
            .with_poster(Box::new(poster));

        let msgs = vec![serde_json::json!({"role": "user", "content": "x"})];
        let tools = RenderedTools::JsonArray(serde_json::json!([]));
        let _ = backend
            .send_turn(&msgs, &tools, &SessionState::default())
            .await;

        assert_eq!(
            url_ref.lock().unwrap().as_deref(),
            Some("http://h:11434/api/chat")
        );
    }

    #[tokio::test]
    async fn debug_impl() {
        let backend = OllamaLlmBackend::new("test-model");
        let s = format!("{backend:?}");
        assert!(s.contains("OllamaLlmBackend"));
        assert!(s.contains("test-model"));
    }

    #[tokio::test]
    async fn response_cache_avoids_second_http_call() {
        let poster = MockPoster::ok(canned_response());
        let call_count = Arc::clone(&poster.call_count);
        let backend = OllamaLlmBackend::new("cached-model")
            .with_base_url("http://cache-test:11434")
            .with_response_cache(Arc::new(ResponseCache::new(30_000)))
            .with_poster(Box::new(poster));

        let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let tools = RenderedTools::JsonArray(serde_json::json!([]));

        let first = backend
            .send_turn(&msgs, &tools, &SessionState::default())
            .await
            .unwrap();
        let second = backend
            .send_turn(&msgs, &tools, &SessionState::default())
            .await
            .unwrap();

        assert!(matches!(first, BackendResponse::Json(_)));
        assert!(matches!(second, BackendResponse::Json(_)));
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }
}
