//! Tool-loop backend adapters and provider-aware factory helpers.

use std::sync::Arc;

use async_trait::async_trait;
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig};

use crate::http::{HttpPostError, HttpPoster};
use crate::provider::AgentCreationError;
use crate::provider::openai_compat::{
    base_url_for_tool_loop, build_extra_body_params, max_tokens_for_model, resolve_api_key,
};
use crate::tool_loop::LlmBackend;

/// Tail-latency hedging for latency-sensitive requests.
pub mod gemini_native;
pub mod hedged;
pub mod openai_compat;

pub use gemini_native::GeminiNativeBackend;
pub use hedged::HedgedBackend;
pub use openai_compat::OpenAiCompatBackend;

struct SharedHttpPoster {
    inner: Arc<dyn HttpPoster>,
}

#[async_trait]
impl HttpPoster for SharedHttpPoster {
    async fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &[u8],
        timeout_ms: u64,
    ) -> Result<String, HttpPostError> {
        self.inner.post_json(url, headers, body, timeout_ms).await
    }
}

/// Create the tool-loop backend for a resolved provider + model pair.
pub fn create_backend(
    provider: &ProviderConfig,
    model: &ModelProfile,
    poster: Arc<dyn HttpPoster>,
) -> Result<Arc<dyn LlmBackend>, AgentCreationError> {
    match provider.kind {
        ProviderKind::OpenAiCompat => Ok(Arc::new(
            OpenAiCompatBackend::new(resolve_api_key(provider)?, model.slug.clone())
                .with_provider_id(model.provider.clone())
                .with_base_url(base_url_for_tool_loop(provider))
                .with_timeout_ms(provider.timeout_ms.unwrap_or(120_000))
                .with_max_tokens(max_tokens_for_model(model))
                .with_extra_headers(provider.extra_headers.clone().unwrap_or_default())
                .with_extra_body_params(build_extra_body_params(provider, model))
                .with_poster(Box::new(SharedHttpPoster { inner: poster })),
        )),
        ProviderKind::AnthropicApi => Err(AgentCreationError::MissingConfig(
            "Anthropic HTTP tool-loop backend is not implemented yet".into(),
        )),
        ProviderKind::ClaudeCli | ProviderKind::CursorAcp => {
            Err(AgentCreationError::MissingConfig(
                "CLI/ACP backends don't use LlmBackend — they own the tool loop".into(),
            ))
        }
        ProviderKind::PerplexityApi => Err(AgentCreationError::MissingConfig(
            "Perplexity tool-loop backend is not implemented yet".into(),
        )),
        ProviderKind::GeminiApi => Err(AgentCreationError::MissingConfig(
            "Gemini tool-loop backend is not implemented yet".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translate::{BackendResponse, RenderedTools, SessionState};
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Debug)]
    struct CapturedRequest {
        url: String,
        headers: Vec<(String, String)>,
        body: Value,
        timeout_ms: u64,
    }

    struct MockPoster {
        response: String,
        requests: Mutex<Vec<CapturedRequest>>,
    }

    impl MockPoster {
        fn new(response: String) -> Self {
            Self {
                response,
                requests: Mutex::new(Vec::new()),
            }
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

    fn zai_provider() -> ProviderConfig {
        ProviderConfig {
            kind: ProviderKind::OpenAiCompat,
            base_url: Some("https://api.z.ai/api/paas/v4".to_string()),
            api_key_env: Some("PATH".to_string()),
            command: None,
            args: None,
            timeout_ms: Some(90_000),
            ttft_timeout_ms: Some(15_000),
            connect_timeout_ms: Some(5_000),
            extra_headers: Some(HashMap::from([(
                "X-Test-Header".to_string(),
                "present".to_string(),
            )])),
            max_concurrent: None,
        }
    }

    fn glm_5_1_profile() -> ModelProfile {
        ModelProfile {
            provider: "zai".to_string(),
            slug: "glm-5.1".to_string(),
            context_window: 200_000,
            max_output: Some(131_072),
            supports_tools: true,
            supports_thinking: true,
            tool_format: "openai_json".to_string(),
            cost_input_per_m: Some(1.40),
            cost_output_per_m: Some(4.40),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn create_backend_factory_builds_openai_compat_backend_for_zai() {
        let poster = Arc::new(MockPoster::new(
            json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "done"
                    }
                }]
            })
            .to_string(),
        ));
        let provider = zai_provider();
        let model = glm_5_1_profile();

        let backend = create_backend(&provider, &model, poster.clone()).expect("create backend");
        let response = backend
            .send_turn(
                &[json!({ "role": "user", "content": "hi" })],
                &RenderedTools::JsonArray(json!([{
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

        let requests = poster.requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].url,
            "https://api.z.ai/api/paas/v4/chat/completions"
        );
        assert_eq!(requests[0].timeout_ms, 90_000);
        assert!(
            requests[0].headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("authorization") && value.starts_with("Bearer ")
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
        assert_eq!(requests[0].body["max_tokens"], 131_072);
        assert_eq!(requests[0].body["thinking"]["type"], "enabled");
        assert_eq!(requests[0].body["thinking"]["clear_thinking"], true);
        assert_eq!(requests[0].body["tool_stream"], true);
        assert_eq!(requests[0].body["tools"][0]["function"]["name"], "echo");
    }
}
