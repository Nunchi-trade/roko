//! `OpenAiCompatLlmBackend` — HTTP adapter implementing [`LlmBackend`]
//! for OpenAI-compatible chat-completions endpoints.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::http::{HttpPoster, ReqwestPoster};
use crate::tool_loop::{LlmBackend, LlmError};
use crate::translate::{BackendResponse, RenderedTools};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_TIMEOUT_MS: u64 = 120_000;

/// HTTP adapter for OpenAI-compatible `/chat/completions` endpoints.
pub struct OpenAiCompatLlmBackend {
    api_key: String,
    model: String,
    base_url: String,
    timeout_ms: u64,
    extra_headers: Vec<(String, String)>,
    extra_body_params: Map<String, Value>,
    poster: Box<dyn HttpPoster>,
}

impl OpenAiCompatLlmBackend {
    /// Construct a backend for `model` with default URL and timeout.
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            extra_headers: Vec::new(),
            extra_body_params: Map::new(),
            poster: Box::new(ReqwestPoster::new()),
        }
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

    /// Inject additional HTTP headers on every request.
    #[must_use]
    pub fn with_extra_headers(mut self, extra_headers: HashMap<String, String>) -> Self {
        let mut extra_headers: Vec<(String, String)> = extra_headers.into_iter().collect();
        extra_headers.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        self.extra_headers = extra_headers;
        self
    }

    /// Inject additional JSON fields into the outbound request body.
    #[must_use]
    pub fn with_extra_body_params(mut self, extra_body_params: Map<String, Value>) -> Self {
        self.extra_body_params = extra_body_params;
        self
    }

    /// Inject a custom HTTP poster (for tests or alternate transports).
    #[must_use]
    pub fn with_poster(mut self, poster: Box<dyn HttpPoster>) -> Self {
        self.poster = poster;
        self
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    fn headers(&self) -> Vec<(String, String)> {
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        if !self.api_key.is_empty() {
            headers.push((
                "Authorization".to_string(),
                format!("Bearer {}", self.api_key),
            ));
        }
        headers.extend(self.extra_headers.iter().cloned());
        headers
    }
}

#[async_trait]
impl LlmBackend for OpenAiCompatLlmBackend {
    async fn send_turn(
        &self,
        messages: &[serde_json::Value],
        tools: &RenderedTools,
    ) -> Result<BackendResponse, LlmError> {
        let RenderedTools::JsonArray(tools) = tools else {
            return Err(LlmError::Backend("expected json tool array".into()));
        };

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "tools": tools,
        });

        if let Some(body_obj) = body.as_object_mut() {
            for (key, value) in &self.extra_body_params {
                body_obj.insert(key.clone(), value.clone());
            }
        }

        let body_bytes =
            serde_json::to_vec(&body).map_err(|e| LlmError::Backend(format!("serialize: {e}")))?;

        let raw = self
            .poster
            .post_json(
                &self.endpoint(),
                &self.headers(),
                &body_bytes,
                self.timeout_ms,
            )
            .await
            .map_err(|e| LlmError::Network(e.to_string()))?;

        let json: Value = serde_json::from_str(&raw)
            .map_err(|e| LlmError::Backend(format!("parse response: {e}")))?;

        Ok(BackendResponse::Json(json))
    }
}

impl std::fmt::Debug for OpenAiCompatLlmBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatLlmBackend")
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("timeout_ms", &self.timeout_ms)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use crate::dispatcher::{HandlerResolver, ToolDispatcher};
    use crate::tool_loop::{StopReason, ToolLoop};
    use crate::translate::{OpenAiTranslator, Translator};
    use roko_core::tool::{
        ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolHandler, ToolPermission,
        ToolResult, VecToolRegistry,
    };

    use crate::http::HttpPostError;

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
    async fn openai_compat_backend_requires_json_tools() {
        let (poster, _) = MockPoster::new(vec![]);
        let backend = OpenAiCompatLlmBackend::new("", "test-model").with_poster(Box::new(poster));

        let err = backend
            .send_turn(
                &[serde_json::json!({ "role": "user", "content": "hi" })],
                &RenderedTools::CliFlag("echo".to_string()),
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
}
