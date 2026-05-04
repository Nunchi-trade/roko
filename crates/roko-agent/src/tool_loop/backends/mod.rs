//! Tool-loop backend adapters and OpenAI-compatible factory helpers.

use std::sync::Arc;

use async_trait::async_trait;
use roko_core::agent::ProviderKind;
#[cfg(test)]
use roko_core::config::DEFAULT_TTFT_TIMEOUT_MS;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use roko_core::defaults::DEFAULT_REQUEST_TIMEOUT_MS;

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

/// Create the OpenAI-compatible tool-loop backend for a resolved provider + model pair.
pub fn create_openai_compat_backend(
    provider: &ProviderConfig,
    model: &ModelProfile,
    poster: Arc<dyn HttpPoster>,
) -> Result<Arc<dyn LlmBackend>, AgentCreationError> {
    match provider.kind {
        ProviderKind::OpenAiCompat => {
            let api_key = resolve_api_key(provider)?;
            let backend = OpenAiCompatBackend::new(api_key, model.slug.clone())
                .with_provider_id(model.provider.clone())
                .with_base_url(base_url_for_tool_loop(provider))
                .with_timeout_ms(provider.timeout_ms.unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS))
                .with_max_tokens(max_tokens_for_model(model))
                .with_extra_headers(provider.extra_headers.clone().unwrap_or_default())
                .with_extra_body_params(build_extra_body_params(provider, model))
                .with_skip_session_fields(true)
                .with_use_max_completion_tokens(model.use_max_completion_tokens)
                .with_ttft_timeout_ms(provider.ttft_timeout_ms)
                .with_poster(Box::new(SharedHttpPoster { inner: poster }));
            Ok(Arc::new(backend))
        }
        ProviderKind::AnthropicApi => {
            crate::provider::anthropic_api::tool_loop::create_tool_loop_backend(
                provider,
                model,
                &crate::provider::AgentOptions::default(),
                Box::new(SharedHttpPoster { inner: poster }),
            )
        }
        ProviderKind::ClaudeCli | ProviderKind::CursorAcp => {
            Err(AgentCreationError::MissingConfig(
                "CLI/ACP backends don't use LlmBackend — they own the tool loop".into(),
            ))
        }
        ProviderKind::PerplexityApi => {
            // Perplexity's chat completions API is OpenAI-compatible.
            let api_key = resolve_api_key(provider)?;
            let base_url = provider
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.perplexity.ai".to_string());
            Ok(Arc::new(
                OpenAiCompatBackend::new(api_key, model.slug.clone())
                    .with_provider_id(model.provider.clone())
                    .with_base_url(base_url)
                    .with_timeout_ms(provider.timeout_ms.unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS))
                    .with_max_tokens(max_tokens_for_model(model))
                    .with_extra_headers(provider.extra_headers.clone().unwrap_or_default())
                    .with_skip_session_fields(true)
                    .with_use_max_completion_tokens(model.use_max_completion_tokens)
                    .with_ttft_timeout_ms(provider.ttft_timeout_ms)
                    .with_poster(Box::new(SharedHttpPoster { inner: poster })),
            ))
        }
        ProviderKind::CerebrasApi => {
            // Cerebras exposes an OpenAI-compatible chat completions surface.
            // Small models need: temperature 0 for determinism, no parallel
            // tool calls, and content normalization (empty string → null).
            let api_key = resolve_api_key(provider)?;
            let base_url = provider
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.cerebras.ai/v1".to_string());
            let mut extra = build_extra_body_params(provider, model);
            extra
                .entry("temperature")
                .or_insert(serde_json::Value::from(0));
            Ok(Arc::new(
                OpenAiCompatBackend::new(api_key, model.slug.clone())
                    .with_provider_id(model.provider.clone())
                    .with_base_url(base_url)
                    .with_timeout_ms(provider.timeout_ms.unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS))
                    .with_max_tokens(max_tokens_for_model(model))
                    .with_extra_headers(provider.extra_headers.clone().unwrap_or_default())
                    .with_extra_body_params(extra)
                    .with_skip_session_fields(true)
                    .with_disable_parallel_tool_calls(true)
                    .with_normalize_tool_call_content(true)
                    .with_use_max_completion_tokens(model.use_max_completion_tokens)
                    .with_ttft_timeout_ms(provider.ttft_timeout_ms)
                    .with_poster(Box::new(SharedHttpPoster { inner: poster })),
            ))
        }
        ProviderKind::GeminiApi => Err(AgentCreationError::MissingConfig(
            "Gemini tool-loop backend is not implemented yet".into(),
        )),
    }
}

/// Create a provider-selected backend for the shared tool-loop runtime.
///
/// This centralizes the backend choice for the runtime paths that need to
/// select between OpenAI-compatible and Gemini-native HTTP surfaces.
pub fn create_tool_loop_backend(
    provider: &ProviderConfig,
    model: &ModelProfile,
    options: &crate::provider::AgentOptions,
    poster: Arc<dyn HttpPoster>,
) -> Result<Arc<dyn LlmBackend>, AgentCreationError> {
    match provider.kind {
        ProviderKind::OpenAiCompat => create_openai_compat_backend(provider, model, poster),
        ProviderKind::GeminiApi if model.supports_tools && model.tool_format == "gemini_native" => {
            Ok(Arc::new(GeminiNativeBackend::new(
                resolve_api_key(provider)?,
                provider
                    .base_url
                    .as_deref()
                    .unwrap_or("https://generativelanguage.googleapis.com"),
                model.clone(),
                options,
            )))
        }
        ProviderKind::GeminiApi => Err(AgentCreationError::MissingConfig(
            "Gemini native tool-loop backend requires gemini_native tool_format".into(),
        )),
        ProviderKind::AnthropicApi => {
            crate::provider::anthropic_api::tool_loop::create_tool_loop_backend(
                provider,
                model,
                options,
                Box::new(SharedHttpPoster { inner: poster }),
            )
        }
        ProviderKind::ClaudeCli | ProviderKind::CursorAcp => {
            Err(AgentCreationError::MissingConfig(
                "CLI/ACP backends don't use LlmBackend — they own the tool loop".into(),
            ))
        }
        ProviderKind::PerplexityApi | ProviderKind::CerebrasApi => {
            create_openai_compat_backend(provider, model, poster)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translate::{BackendResponse, RenderedTools, SessionState};
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::thread;
    use std::time::Duration;

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
            ttft_timeout_ms: Some(DEFAULT_TTFT_TIMEOUT_MS),
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

    fn spawn_chat_server(
        response: String,
    ) -> (String, Arc<Mutex<Option<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("server addr");
        let captured = Arc::new(Mutex::new(None));
        let captured_request = Arc::clone(&captured);

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set read timeout");

            let mut buf = Vec::new();
            let mut header_end = None;
            let mut content_length = None;

            loop {
                let mut chunk = [0_u8; 1024];
                let n = stream.read(&mut chunk).expect("read request");
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);

                if header_end.is_none()
                    && let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    header_end = Some(pos + 4);
                    let headers = String::from_utf8_lossy(&buf[..pos + 4]);
                    content_length = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    });
                }

                if let (Some(header_end), Some(content_length)) = (header_end, content_length)
                    && buf.len() >= header_end + content_length
                {
                    break;
                }
            }

            let header_end = header_end.expect("request headers");
            let content_length = content_length.expect("content length");
            let request = String::from_utf8_lossy(&buf[..header_end + content_length]).to_string();
            *captured_request.lock().expect("capture lock") = Some(request);

            let response_bytes = response.as_bytes();
            let wire = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_bytes.len(),
                response
            );
            stream.write_all(wire.as_bytes()).expect("write response");
            stream.flush().expect("flush response");
        });

        (format!("http://{}", addr), captured, handle)
    }

    #[tokio::test]
    async fn create_openai_compat_backend_builds_openai_compat_backend_for_zai() {
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

        let backend = create_openai_compat_backend(&provider, &model, poster.clone())
            .expect("create backend");
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

    #[tokio::test]
    async fn create_tool_loop_backend_routes_gemini_native_models_to_generate_content() {
        let response = json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{ "text": "native ok" }]
                },
                "finishReason": "STOP"
            }]
        })
        .to_string();
        let (base_url, captured, handle) = spawn_chat_server(response);
        let expected_base_url = base_url.clone();
        let provider = ProviderConfig {
            kind: ProviderKind::GeminiApi,
            base_url: Some(base_url),
            api_key_env: None,
            command: None,
            args: None,
            timeout_ms: Some(90_000),
            ttft_timeout_ms: Some(DEFAULT_TTFT_TIMEOUT_MS),
            connect_timeout_ms: Some(5_000),
            extra_headers: None,
            max_concurrent: None,
        };
        let model = ModelProfile {
            provider: "gemini".to_string(),
            slug: "gemini-2.5-pro".to_string(),
            context_window: 1_048_576,
            max_output: Some(65_536),
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
            thinking_level: Some("low".to_string()),
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
        };
        let backend = create_tool_loop_backend(
            &provider,
            &model,
            &crate::provider::AgentOptions::default(),
            Arc::new(MockPoster::new("{}".to_string())),
        )
        .expect("create gemini tool-loop backend");

        let response = backend
            .send_turn(
                &[json!({ "role": "user", "content": "hi" })],
                &RenderedTools::JsonArray(json!([])),
                &SessionState::default(),
            )
            .await
            .expect("send turn");

        assert!(matches!(response, BackendResponse::Json(_)));

        let request = captured
            .lock()
            .expect("capture lock")
            .clone()
            .expect("captured request");
        assert!(
            request.starts_with("POST /v1beta/models/gemini-2.5-pro:generateContent HTTP/1.1"),
            "unexpected request line: {request}"
        );
        assert!(
            request.contains(&expected_base_url.replace("http://", "host: ")),
            "expected host header for {expected_base_url}: {request}"
        );
        assert!(
            request.contains("\"text\":\"hi\""),
            "expected prompt payload in request: {request}"
        );

        handle.join().expect("server thread");
    }
}
