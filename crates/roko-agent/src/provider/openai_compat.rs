//! OpenAI-compatible provider adapter.
//!
//! Kimi-K2.5 is an OpenAI-compatible backend with a few non-standard
//! constraints that the request builder and response history plumbing must
//! respect:
//!
//! - Images must be sent as base64 `data:image/...;base64,...` payloads.
//!   Plain image URLs are not accepted.
//! - When thinking is enabled, Kimi fixes `temperature = 1.0`, `top_p = 0.95`,
//!   and `n = 1`.
//! - With thinking enabled, `tool_choice` is limited to `"auto"` or `"none"`.
//! - The built-in `$web_search` tool is incompatible with thinking mode.
//! - Tool-call IDs are returned in `functions.<name>:<idx>` form and must be
//!   preserved exactly when dispatching tool results.
//! - A single request may include at most 128 tools.
//! - Requests have a 2-hour timeout budget.
//! - `reasoning_content` must be carried forward in conversation history when
//!   building the next turn.

use crate::Agent;
use crate::codex_agent::{CodexAgent, DEFAULT_MAX_TOKENS};
use crate::provider::{AgentCreationError, AgentOptions, ProviderAdapter, ProviderError};
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Adapter for OpenAI-compatible HTTP providers.
pub struct OpenAiCompatAdapter;

fn is_zai_provider(provider: &ProviderConfig, model: &ModelProfile) -> bool {
    model.provider.eq_ignore_ascii_case("zai")
        || provider
            .base_url
            .as_deref()
            .is_some_and(|base_url| base_url.contains("z.ai") || base_url.contains("bigmodel.cn"))
}

fn is_openrouter(base_url: &str) -> bool {
    base_url.contains("openrouter.ai")
}

fn inject_glm_params(
    body: &mut Map<String, Value>,
    provider: &ProviderConfig,
    model: &ModelProfile,
) {
    if !model.supports_thinking || !is_zai_provider(provider, model) {
        return;
    }

    body.insert(
        "thinking".to_string(),
        json!({
            "type": "enabled",
            "clear_thinking": true,
        }),
    );
    body.insert("tool_stream".to_string(), Value::Bool(true));
}

fn inject_provider_routing(
    body: &mut Map<String, Value>,
    provider: &ProviderConfig,
    model: &ModelProfile,
) {
    let Some(base_url) = provider.base_url.as_deref() else {
        return;
    };
    if !is_openrouter(base_url) {
        return;
    }

    let Some(routing) = model.provider_routing.as_ref() else {
        return;
    };

    if let Ok(value) = serde_json::to_value(routing) {
        body.insert("provider".to_string(), value);
    }
}

fn is_kimi_model(model: &ModelProfile) -> bool {
    model.slug.starts_with("kimi-")
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)]
pub(crate) struct ChatMessage {
    #[serde(default)]
    content: Vec<ContentBlock>,
}

#[allow(dead_code)]
impl ChatMessage {
    fn content_blocks(&self) -> Option<&[ContentBlock]> {
        (!self.content.is_empty()).then_some(self.content.as_slice())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
pub(crate) enum ContentBlock {
    Text { text: String },
    ImageUrl { image_url: ImageUrlBlock },
}

#[allow(dead_code)]
impl ContentBlock {
    fn is_image_url(&self) -> bool {
        matches!(self, Self::ImageUrl { .. })
    }

    fn is_base64(&self) -> bool {
        matches!(
            self,
            Self::ImageUrl { image_url }
                if image_url.url.starts_with("data:image/")
                    && image_url.url.contains(";base64,")
        )
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)]
pub(crate) struct ImageUrlBlock {
    url: String,
}

#[allow(dead_code)]
fn validate_vision_input(
    messages: &[ChatMessage],
    model: &ModelProfile,
) -> Result<(), AgentCreationError> {
    if !model.supports_vision {
        return Ok(());
    }

    for msg in messages {
        if let Some(content_blocks) = msg.content_blocks() {
            for block in content_blocks {
                if block.is_image_url() && !block.is_base64() {
                    return Err(AgentCreationError::MissingConfig(
                        "Kimi requires base64-encoded images, not URLs".into(),
                    ));
                }
            }
        }
    }

    Ok(())
}

fn inject_kimi_params(body: &mut Map<String, Value>, model: &ModelProfile) {
    if !model.supports_thinking || !is_kimi_model(model) {
        return;
    }

    body.insert(
        "thinking".to_string(),
        json!({
            "type": "enabled",
        }),
    );
}

impl ProviderAdapter for OpenAiCompatAdapter {
    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAiCompat
    }

    fn create_agent(
        &self,
        provider: &ProviderConfig,
        model: &ModelProfile,
        options: &AgentOptions,
    ) -> Result<Box<dyn Agent>, AgentCreationError> {
        let api_key = provider
            .resolve_api_key()
            .or_else(|| {
                if provider.base_url.as_deref() == Some("http://localhost:11434") {
                    Some(String::new())
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                AgentCreationError::MissingApiKey(provider.api_key_env.clone().unwrap_or_default())
            })?;

        let base_url = provider
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let base_url = base_url
            .strip_suffix("/v1")
            .unwrap_or(base_url.as_str())
            .to_string();

        let timeout = options
            .timeout_ms
            .or(provider.timeout_ms)
            .unwrap_or(120_000);
        let max_tokens = model
            .max_output
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(DEFAULT_MAX_TOKENS);
        let mut extra_body_params = Map::new();
        inject_glm_params(&mut extra_body_params, provider, model);
        inject_kimi_params(&mut extra_body_params, model);
        inject_provider_routing(&mut extra_body_params, provider, model);

        let agent = CodexAgent::new(api_key, model.slug.clone())
            .with_base_url(base_url)
            .with_timeout_ms(timeout)
            .with_max_tokens(max_tokens)
            .with_extra_body_params(extra_body_params)
            .with_name(options.name.clone());

        Ok(Box::new(agent))
    }

    fn classify_error(&self, status: u16, body: &Value) -> ProviderError {
        if let Some(code) = body.pointer("/error/code").and_then(Value::as_str) {
            return match code {
                "1302" => ProviderError::RateLimit {
                    retry_after_ms: Some(5_000),
                },
                "1303" | "1304" | "1305" => ProviderError::RateLimit {
                    retry_after_ms: Some(60_000),
                },
                "1301" => ProviderError::ContentPolicy,
                "1000" | "1001" | "1002" | "1003" | "1004" => ProviderError::AuthFailure,
                "1211" => ProviderError::ModelNotFound,
                "1261" => ProviderError::ContextOverflow,
                _ => ProviderError::Other(format!("Z.AI error {code}")),
            };
        }

        match status {
            429 => ProviderError::RateLimit {
                retry_after_ms: body
                    .pointer("/retry_after")
                    .and_then(|v| v.as_u64())
                    .map(|seconds| seconds * 1000),
            },
            401 | 403 => ProviderError::AuthFailure,
            404 => ProviderError::ModelNotFound,
            408 => ProviderError::Timeout,
            500..=599 => ProviderError::ServerError(status),
            _ => ProviderError::Other(format!("HTTP {}", status)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpPostError, HttpPoster};
    use roko_core::{Body, Context, Kind, Signal};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    fn prompt(text: &str) -> Signal {
        Signal::builder(Kind::Prompt).body(Body::text(text)).build()
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

    #[derive(Debug, Clone)]
    struct RecordedRequest {
        url: String,
        headers: Vec<(String, String)>,
        body: String,
        timeout_ms: u64,
    }

    #[derive(Debug)]
    struct MockPoster {
        response: String,
        captured: Arc<Mutex<Option<RecordedRequest>>>,
    }

    impl MockPoster {
        fn new(response: impl Into<String>) -> (Arc<Self>, Arc<Mutex<Option<RecordedRequest>>>) {
            let captured = Arc::new(Mutex::new(None));
            let poster = Arc::new(Self {
                response: response.into(),
                captured: Arc::clone(&captured),
            });
            (poster, captured)
        }
    }

    #[async_trait::async_trait]
    impl HttpPoster for MockPoster {
        async fn post_json(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &[u8],
            timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            let request = RecordedRequest {
                url: url.to_string(),
                headers: headers.to_vec(),
                body: String::from_utf8(body.to_vec()).expect("request body is utf8"),
                timeout_ms,
            };
            *self.captured.lock().expect("capture lock") = Some(request);
            Ok(self.response.clone())
        }
    }

    #[tokio::test]
    async fn glm_thinking_injection() {
        let response = serde_json::json!({
            "id": "chatcmpl-test",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "zai-ok"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 11,
                "completion_tokens": 7,
                "total_tokens": 18
            }
        })
        .to_string();
        let (base_url, captured, handle) = spawn_chat_server(response);

        let provider = ProviderConfig {
            kind: ProviderKind::OpenAiCompat,
            base_url: Some(format!("{base_url}/v4")),
            api_key_env: Some("PATH".to_string()),
            command: None,
            args: None,
            timeout_ms: Some(1_500),
            extra_headers: None,
            max_concurrent: None,
        };
        let model = ModelProfile {
            provider: "zai".to_string(),
            slug: "glm-5.1".to_string(),
            context_window: 200_000,
            max_output: Some(1_024),
            supports_tools: true,
            supports_thinking: true,
            supports_vision: false,
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: false,
            provider_routing: None,
            tool_format: "openai_json".to_string(),
            cost_input_per_m: None,
            cost_output_per_m: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            max_tools: None,
            tokenizer_ratio: None,
            supports_search: false,
            supports_citations: false,
            supports_async: false,
            is_embedding_model: false,
            search_context_size: None,
            cost_per_request: None,
        };
        let options = AgentOptions {
            timeout_ms: Some(2_500),
            name: "zai-agent".to_string(),
            ..Default::default()
        };

        let adapter = OpenAiCompatAdapter;
        assert_eq!(adapter.kind(), ProviderKind::OpenAiCompat);

        let agent = adapter
            .create_agent(&provider, &model, &options)
            .expect("create agent");
        assert_eq!(agent.name(), "zai-agent");

        let result = agent.run(&prompt("hello"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap_or(""), "zai-ok");
        assert_eq!(result.usage.input_tokens, 11);
        assert_eq!(result.usage.output_tokens, 7);

        let request = captured
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured request");
        assert!(request.starts_with("POST /v4/v1/chat/completions HTTP/1.1"));

        let body = request.split("\r\n\r\n").nth(1).expect("request body");
        let parsed: Value = serde_json::from_str(body).expect("json request body");
        assert_eq!(parsed["model"], "glm-5.1");
        assert_eq!(parsed["max_tokens"], 1024);
        assert_eq!(parsed["messages"][0]["content"], "hello");
        assert_eq!(
            parsed["thinking"],
            serde_json::json!({
                "type": "enabled",
                "clear_thinking": true
            })
        );
        assert_eq!(parsed["tool_stream"], Value::Bool(true));

        handle.join().expect("server thread");
    }

    #[tokio::test]
    async fn kimi_thinking_injection() {
        let response = serde_json::json!({
            "id": "chatcmpl-test",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "kimi-ok"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 13,
                "completion_tokens": 8,
                "total_tokens": 21
            }
        })
        .to_string();
        let (base_url, captured, handle) = spawn_chat_server(response);

        let provider = ProviderConfig {
            kind: ProviderKind::OpenAiCompat,
            base_url: Some(format!("{base_url}/v1")),
            api_key_env: Some("PATH".to_string()),
            command: None,
            args: None,
            timeout_ms: Some(1_500),
            extra_headers: None,
            max_concurrent: None,
        };
        let model = ModelProfile {
            provider: "moonshot".to_string(),
            slug: "kimi-k2.5".to_string(),
            context_window: 256_000,
            max_output: Some(65_535),
            supports_tools: true,
            supports_thinking: true,
            supports_vision: true,
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: true,
            provider_routing: None,
            tool_format: "openai_json".to_string(),
            cost_input_per_m: None,
            cost_output_per_m: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            max_tools: Some(128),
            tokenizer_ratio: None,
            supports_search: false,
            supports_citations: false,
            supports_async: false,
            is_embedding_model: false,
            search_context_size: None,
            cost_per_request: None,
        };
        let options = AgentOptions {
            timeout_ms: Some(2_500),
            name: "kimi-agent".to_string(),
            ..Default::default()
        };

        let adapter = OpenAiCompatAdapter;
        assert_eq!(adapter.kind(), ProviderKind::OpenAiCompat);

        let agent = adapter
            .create_agent(&provider, &model, &options)
            .expect("create agent");
        assert_eq!(agent.name(), "kimi-agent");

        let result = agent.run(&prompt("hello"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap_or(""), "kimi-ok");

        let request = captured
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured request");
        assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));

        let body = request.split("\r\n\r\n").nth(1).expect("request body");
        let parsed: Value = serde_json::from_str(body).expect("json request body");
        assert_eq!(parsed["model"], "kimi-k2.5");
        assert_eq!(parsed["max_tokens"], 65535);
        assert_eq!(parsed["messages"][0]["content"], "hello");
        assert_eq!(
            parsed["thinking"],
            serde_json::json!({
                "type": "enabled"
            })
        );

        handle.join().expect("server thread");
    }

    #[tokio::test]
    async fn openrouter_routing_injection() {
        let response = serde_json::json!({
            "id": "chatcmpl-test",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "openrouter-ok"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 9,
                "completion_tokens": 4,
                "total_tokens": 13
            }
        })
        .to_string();
        let (poster, captured) = MockPoster::new(response);

        let provider = ProviderConfig {
            kind: ProviderKind::OpenAiCompat,
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_key_env: Some("PATH".to_string()),
            command: None,
            args: None,
            timeout_ms: Some(1_500),
            extra_headers: None,
            max_concurrent: None,
        };
        let model = ModelProfile {
            provider: "openrouter".to_string(),
            slug: "z-ai/glm-5.1".to_string(),
            context_window: 200_000,
            max_output: Some(1_024),
            supports_tools: true,
            supports_thinking: true,
            supports_vision: false,
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: false,
            provider_routing: Some(roko_core::config::schema::ProviderRouting {
                sort: Some("price".to_string()),
                order: Some(vec!["z-ai".to_string(), "moonshotai".to_string()]),
                allow_fallbacks: Some(true),
                max_price: Some(0.0025),
                require_parameters: Some(vec!["temperature".to_string()]),
            }),
            tool_format: "openai_json".to_string(),
            cost_input_per_m: None,
            cost_output_per_m: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            max_tools: None,
            tokenizer_ratio: None,
            supports_search: false,
            supports_citations: false,
            supports_async: false,
            is_embedding_model: false,
            search_context_size: None,
            cost_per_request: None,
        };
        let options = AgentOptions {
            timeout_ms: Some(2_500),
            name: "openrouter-agent".to_string(),
            ..Default::default()
        };

        let mut extra_body_params = Map::new();
        inject_provider_routing(&mut extra_body_params, &provider, &model);

        let agent = CodexAgent::new("test-key", model.slug.clone())
            .with_base_url("https://openrouter.ai/api")
            .with_http_poster(poster)
            .with_extra_body_params(extra_body_params)
            .with_name(options.name.clone());

        let result = agent.run(&prompt("hello"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap_or(""), "openrouter-ok");

        let request = captured
            .lock()
            .expect("capture lock")
            .clone()
            .expect("captured request");
        assert_eq!(request.url, "https://openrouter.ai/api/v1/chat/completions");
        assert_eq!(request.timeout_ms, 120_000);
        assert!(
            request
                .headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("authorization")
                    && value == "Bearer test-key")
        );
        assert!(
            request
                .headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("content-type")
                    && value == "application/json")
        );
        let parsed: Value = serde_json::from_str(&request.body).expect("json request body");
        assert_eq!(
            parsed["provider"],
            serde_json::json!({
                "sort": "price",
                "order": ["z-ai", "moonshotai"],
                "allow_fallbacks": true,
                "max_price": 0.0025,
                "require_parameters": ["temperature"]
            })
        );
    }

    #[test]
    fn kimi_vision_base64_only() {
        let model = ModelProfile {
            provider: "moonshot".to_string(),
            slug: "kimi-k2.5".to_string(),
            context_window: 256_000,
            max_output: Some(65_535),
            supports_tools: true,
            supports_thinking: true,
            supports_vision: true,
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: true,
            provider_routing: None,
            tool_format: "openai_json".to_string(),
            cost_input_per_m: None,
            cost_output_per_m: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            max_tools: Some(128),
            tokenizer_ratio: None,
            supports_search: false,
            supports_citations: false,
            supports_async: false,
            is_embedding_model: false,
            search_context_size: None,
            cost_per_request: None,
        };

        let base64_message = ChatMessage {
            content: vec![ContentBlock::ImageUrl {
                image_url: ImageUrlBlock {
                    url: "data:image/png;base64,iVBORw0KGgo=".to_string(),
                },
            }],
        };
        assert!(validate_vision_input(&[base64_message], &model).is_ok());

        let url_message = ChatMessage {
            content: vec![ContentBlock::ImageUrl {
                image_url: ImageUrlBlock {
                    url: "https://example.com/image.png".to_string(),
                },
            }],
        };
        let err = validate_vision_input(&[url_message], &model).expect_err("expected error");
        assert!(matches!(
            err,
            AgentCreationError::MissingConfig(message)
                if message == "Kimi requires base64-encoded images, not URLs"
        ));

        let non_vision_model = ModelProfile {
            supports_vision: false,
            ..model
        };
        let accepted_for_other_provider = ChatMessage {
            content: vec![ContentBlock::ImageUrl {
                image_url: ImageUrlBlock {
                    url: "https://example.com/image.png".to_string(),
                },
            }],
        };
        assert!(validate_vision_input(&[accepted_for_other_provider], &non_vision_model).is_ok());
    }

    #[test]
    fn classify_error_maps_retry_after_and_auth() {
        let adapter = OpenAiCompatAdapter;
        let rate_limit = adapter.classify_error(429, &serde_json::json!({ "retry_after": 7 }));
        match rate_limit {
            ProviderError::RateLimit {
                retry_after_ms: Some(ms),
            } => assert_eq!(ms, 7_000),
            other => panic!("unexpected rate limit classification: {other:?}"),
        }
        assert!(matches!(
            adapter.classify_error(401, &Value::Null),
            ProviderError::AuthFailure
        ));
    }

    #[test]
    fn zai_error_classify_maps_business_codes() {
        let adapter = OpenAiCompatAdapter;

        match adapter.classify_error(429, &serde_json::json!({ "error": { "code": "1302" } })) {
            ProviderError::RateLimit {
                retry_after_ms: Some(ms),
            } => assert_eq!(ms, 5_000),
            other => panic!("unexpected Z.AI classification: {other:?}"),
        }

        assert!(matches!(
            adapter.classify_error(400, &serde_json::json!({ "error": { "code": "1261" } })),
            ProviderError::ContextOverflow
        ));
    }
}
