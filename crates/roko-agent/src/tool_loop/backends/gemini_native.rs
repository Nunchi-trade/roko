//! Gemini native `generateContent` backend for the shared tool loop.

use async_trait::async_trait;
use roko_core::config::schema::ModelProfile;
use roko_core::defaults::DEFAULT_REQUEST_TIMEOUT_MS;
use serde_json::Value;

use crate::gemini::native::{
    build_generate_content_request, build_generation_config, system_instruction_from_segments,
};
use crate::gemini::types::{
    Content, GeminiTool, GenerateContentRequest, GenerateContentResponse, GenerationConfig, Part,
};
use crate::gemini::wire::{
    generate_content_endpoint, generate_content_headers, send_generate_content_request,
    serialize_generate_content_request,
};
#[cfg(test)]
use crate::http::HttpPostError;
use crate::http::{HttpPoster, ReqwestPoster};
use crate::provider::AgentOptions;
use crate::tool_loop::{LlmBackend, LlmError};
use crate::translate::{BackendResponse, RenderedTools, SessionState};

const DEFAULT_TIMEOUT_MS: u64 = DEFAULT_REQUEST_TIMEOUT_MS;

/// HTTP backend for Gemini-native `generateContent` models.
pub struct GeminiNativeBackend {
    api_key: String,
    base_url: String,
    model: ModelProfile,
    thinking_level: Option<String>,
    cached_content: Option<String>,
    timeout_ms: u64,
    poster: Box<dyn HttpPoster>,
}

impl GeminiNativeBackend {
    /// Construct a Gemini-native backend from provider/model options.
    #[must_use]
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model: ModelProfile,
        options: &AgentOptions,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
            thinking_level: options
                .effort
                .clone()
                .or_else(|| model.thinking_level.clone()),
            cached_content: options.cached_content.clone(),
            timeout_ms: options.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
            model,
            poster: Box::new(ReqwestPoster::new()),
        }
    }

    #[cfg(test)]
    #[must_use]
    fn with_poster(mut self, poster: Box<dyn HttpPoster>) -> Self {
        self.poster = poster;
        self
    }

    fn translate_message(message: &Value) -> Option<Content> {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");

        if role == "system" {
            return None;
        }

        if message.get("parts").is_some() {
            return serde_json::from_value(message.clone()).ok();
        }

        let text = message.get("content").and_then(Value::as_str)?;
        if text.trim().is_empty() {
            return None;
        }

        let role = match role {
            "assistant" => "model",
            "model" => "model",
            _ => "user",
        };

        Some(Content {
            role: role.to_string(),
            parts: vec![Part::Text {
                text: text.to_string(),
            }],
        })
    }

    fn system_instruction(messages: &[Value]) -> Option<Content> {
        let mut segments = Vec::new();

        for message in messages {
            if message.get("role").and_then(Value::as_str) != Some("system") {
                continue;
            }

            if let Some(text) = message.get("content").and_then(Value::as_str) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    segments.push(trimmed.to_string());
                }
            }
        }

        system_instruction_from_segments(segments)
    }

    fn translate_messages(messages: &[Value]) -> Vec<Content> {
        messages
            .iter()
            .filter_map(Self::translate_message)
            .collect()
    }

    fn translate_tools(tools: &RenderedTools) -> Result<Option<Vec<GeminiTool>>, LlmError> {
        let RenderedTools::JsonArray(tools) = tools else {
            return Err(LlmError::Backend("expected json tool array".into()));
        };

        let tools: Vec<GeminiTool> = serde_json::from_value(tools.clone())
            .map_err(|err| LlmError::Backend(format!("parse tool declarations: {err}")))?;
        Ok((!tools.is_empty()).then_some(tools))
    }

    fn generation_config(&self) -> Option<GenerationConfig> {
        build_generation_config(&self.model, self.thinking_level.as_deref())
    }

    fn build_request(
        &self,
        messages: &[Value],
        tools: &RenderedTools,
    ) -> Result<GenerateContentRequest, LlmError> {
        Ok(build_generate_content_request(
            Self::translate_messages(messages),
            Self::system_instruction(messages),
            Self::translate_tools(tools)?,
            self.generation_config(),
            None,
            self.cached_content.clone(),
        ))
    }
}

#[async_trait]
impl LlmBackend for GeminiNativeBackend {
    async fn send_turn(
        &self,
        messages: &[Value],
        tools: &RenderedTools,
        _session: &SessionState,
    ) -> Result<BackendResponse, LlmError> {
        let request = self.build_request(messages, tools)?;
        let body = serialize_generate_content_request(&request)
            .map_err(|err| LlmError::Backend(format!("serialize request: {err}")))?;
        let raw = send_generate_content_request(
            &*self.poster,
            &generate_content_endpoint(&self.base_url, &self.model.slug),
            &generate_content_headers(&self.api_key),
            &body,
            self.timeout_ms,
        )
        .await
        .map_err(|err| LlmError::Network(err.to_string()))?;

        let json: Value = serde_json::from_str(&raw)
            .map_err(|err| LlmError::Backend(format!("parse response: {err}")))?;

        serde_json::from_value::<GenerateContentResponse>(json.clone())
            .map_err(|err| LlmError::Backend(format!("validate response: {err}")))?;

        Ok(BackendResponse::Json(json))
    }

    fn backend_id(&self) -> &'static str {
        "gemini"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translate::RenderedTools;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[derive(Debug)]
    struct CapturedRequest {
        url: String,
        headers: Vec<(String, String)>,
        body: Value,
        timeout_ms: u64,
    }

    struct MockPoster {
        response: String,
        requests: Arc<Mutex<Vec<CapturedRequest>>>,
    }

    impl MockPoster {
        fn new(response: String, requests: Arc<Mutex<Vec<CapturedRequest>>>) -> Self {
            Self { response, requests }
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
            let body = serde_json::from_slice(body)
                .map_err(|err| HttpPostError::transport(format!("parse request body: {err}")))?;
            self.requests
                .lock()
                .expect("requests lock")
                .push(CapturedRequest {
                    url: url.to_string(),
                    headers: headers.to_vec(),
                    body,
                    timeout_ms,
                });
            Ok(self.response.clone())
        }
    }

    fn tool_model() -> ModelProfile {
        ModelProfile {
            provider: "gemini".to_string(),
            slug: "gemini-3.1-pro-preview".to_string(),
            context_window: 1_048_576,
            max_output: Some(8_192),
            supports_tools: true,
            supports_thinking: true,
            supports_vision: false,
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: false,
            supports_grounding: false,
            supports_code_execution: false,
            supports_caching: false,
            provider_routing: None,
            tool_format: "gemini_native".to_string(),
            cost_input_per_m: None,
            cost_output_per_m: None,
            cost_input_per_m_high: None,
            cost_output_per_m_high: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            thinking_level: Some("dynamic".to_string()),
            max_tools: None,
            tokenizer_ratio: None,
            supports_search: false,
            supports_citations: false,
            supports_async: false,
            is_embedding_model: false,
            search_context_size: None,
            cost_per_request: None,
            use_max_completion_tokens: false,
            tier: None,
        }
    }

    #[tokio::test]
    async fn gemini_native_backend_builds_generate_content_request() {
        let response = json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [
                        { "text": "done" }
                    ]
                }
            }],
            "usage_metadata": {
                "prompt_token_count": 12,
                "candidates_token_count": 4,
                "total_token_count": 16
            }
        })
        .to_string();

        let requests = Arc::new(Mutex::new(Vec::new()));
        let mock = MockPoster::new(response, Arc::clone(&requests));
        let backend = GeminiNativeBackend::new(
            "test-key".to_string(),
            "https://generativelanguage.googleapis.com".to_string(),
            tool_model(),
            &AgentOptions {
                timeout_ms: Some(42),
                cached_content: Some("cache-123".to_string()),
                effort: Some("high".to_string()),
                ..Default::default()
            },
        )
        .with_poster(Box::new(mock));

        let messages = vec![
            json!({ "role": "system", "content": "system prompt" }),
            json!({ "role": "user", "content": "hello" }),
        ];
        let tools = RenderedTools::JsonArray(json!([{
            "functionDeclarations": [{
                "name": "echo",
                "description": "echo args",
                "parameters": { "type": "object", "properties": {} }
            }]
        }]));

        let response = backend
            .send_turn(&messages, &tools, &SessionState::default())
            .await
            .expect("send turn");

        assert!(matches!(response, BackendResponse::Json(_)));

        let request = requests.lock().expect("requests lock");
        assert_eq!(request.len(), 1);
        assert_eq!(
            request[0].url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.1-pro-preview:generateContent"
        );
        assert_eq!(request[0].timeout_ms, 42);
        assert!(
            request[0]
                .headers
                .iter()
                .any(|(name, value)| name == "x-goog-api-key" && value == "test-key")
        );
        assert!(
            request[0]
                .headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("content-type")
                    && value == "application/json")
        );
        assert_eq!(
            request[0].body["systemInstruction"]["parts"][0]["text"],
            "system prompt"
        );
        assert_eq!(request[0].body["contents"][0]["role"], "user");
        assert_eq!(request[0].body["contents"][0]["parts"][0]["text"], "hello");
        assert_eq!(request[0].body["cachedContent"], "cache-123");
        assert_eq!(
            request[0].body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "high"
        );
        assert_eq!(
            request[0].body["tools"][0]["functionDeclarations"][0]["name"],
            "echo"
        );
    }
}
