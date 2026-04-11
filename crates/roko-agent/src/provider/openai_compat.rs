use crate::Agent;
use crate::codex_agent::{CodexAgent, DEFAULT_MAX_TOKENS};
use crate::provider::{AgentCreationError, AgentOptions, ProviderAdapter, ProviderError};
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use serde_json::{Map, Value, json};

/// Adapter for OpenAI-compatible HTTP providers.
pub struct OpenAiCompatAdapter;

fn is_zai_provider(provider: &ProviderConfig, model: &ModelProfile) -> bool {
    model.provider.eq_ignore_ascii_case("zai")
        || provider.base_url.as_deref().is_some_and(|base_url| {
            base_url.contains("z.ai") || base_url.contains("bigmodel.cn")
        })
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

        let agent = CodexAgent::new(api_key, model.slug.clone())
            .with_base_url(base_url)
            .with_timeout_ms(timeout)
            .with_max_tokens(max_tokens)
            .with_extra_body_params(extra_body_params)
            .with_name(options.name.clone());

        Ok(Box::new(agent))
    }

    fn classify_error(&self, status: u16, body: &Value) -> ProviderError {
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
            tool_format: "openai_json".to_string(),
            cost_input_per_m: None,
            cost_output_per_m: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            max_tools: None,
            tokenizer_ratio: None,
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
}
