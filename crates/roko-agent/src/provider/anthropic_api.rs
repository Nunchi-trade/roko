use crate::Agent;
use crate::claude_agent::{ClaudeAgent, DEFAULT_BASE_URL, DEFAULT_MAX_TOKENS};
use crate::provider::{AgentCreationError, AgentOptions, ProviderAdapter, ProviderError};
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use serde_json::Value;

/// Adapter for the Anthropic Messages API.
pub struct AnthropicApiAdapter;

impl AnthropicApiAdapter {
    fn base_url(provider: &ProviderConfig) -> String {
        let base_url = provider
            .base_url
            .as_deref()
            .unwrap_or(DEFAULT_BASE_URL)
            .trim_end_matches('/');

        base_url.strip_suffix("/v1").unwrap_or(base_url).to_string()
    }
}

impl ProviderAdapter for AnthropicApiAdapter {
    fn kind(&self) -> ProviderKind {
        ProviderKind::AnthropicApi
    }

    fn create_agent(
        &self,
        provider: &ProviderConfig,
        model: &ModelProfile,
        options: &AgentOptions,
    ) -> Result<Box<dyn Agent>, AgentCreationError> {
        if provider.kind != self.kind() {
            return Err(AgentCreationError::InvalidKind(provider.kind));
        }

        let api_key = provider.resolve_api_key().ok_or_else(|| {
            AgentCreationError::MissingApiKey(provider.api_key_env.clone().unwrap_or_default())
        })?;
        let timeout_ms = options
            .timeout_ms
            .or(provider.timeout_ms)
            .unwrap_or(120_000);
        let max_tokens = model
            .max_output
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(DEFAULT_MAX_TOKENS);

        let mut agent = ClaudeAgent::new(api_key, model.slug.clone())
            .with_base_url(Self::base_url(provider))
            .with_timeout_ms(timeout_ms)
            .with_max_tokens(max_tokens);

        if let Some(prompt) = &options.system_prompt {
            agent = agent.with_system_prompt(prompt.clone());
        }
        if !options.name.is_empty() {
            agent = agent.with_name(options.name.clone());
        }

        Ok(Box::new(agent))
    }

    fn classify_error(&self, status: u16, body: &Value) -> ProviderError {
        match status {
            429 => ProviderError::RateLimit {
                retry_after_ms: body
                    .pointer("/retry_after")
                    .and_then(|value| value.as_u64())
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

    fn spawn_messages_server(
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

    fn anthropic_model() -> ModelProfile {
        ModelProfile {
            provider: "anthropic".to_string(),
            slug: "claude-sonnet-4-6".to_string(),
            context_window: 200_000,
            max_output: Some(1_024),
            supports_tools: true,
            supports_thinking: false,
            supports_vision: false,
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: false,
            provider_routing: None,
            tool_format: "anthropic_blocks".to_string(),
            cost_input_per_m: None,
            cost_output_per_m: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            max_tools: None,
            tokenizer_ratio: None,
        }
    }

    #[tokio::test]
    async fn anthropic_api_adapter_creates_agent_for_messages_endpoint() {
        let response = serde_json::json!({
            "id": "msg_test",
            "model": "claude-sonnet-4-6",
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "anthropic-ok"}],
            "usage": {
                "input_tokens": 12,
                "output_tokens": 34,
                "cache_read_input_tokens": 5,
                "cache_creation_input_tokens": 7
            }
        })
        .to_string();
        let (base_url, captured, handle) = spawn_messages_server(response);

        let provider = ProviderConfig {
            kind: ProviderKind::AnthropicApi,
            base_url: Some(format!("{base_url}/v1")),
            api_key_env: Some("PATH".to_string()),
            command: None,
            args: None,
            timeout_ms: Some(1_500),
            extra_headers: None,
            max_concurrent: None,
        };
        let options = AgentOptions {
            timeout_ms: Some(2_500),
            system_prompt: Some("system guidance".to_string()),
            name: "anthropic-agent".to_string(),
            ..Default::default()
        };
        let model = anthropic_model();

        let adapter = AnthropicApiAdapter;
        assert_eq!(adapter.kind(), ProviderKind::AnthropicApi);

        let agent = adapter
            .create_agent(&provider, &model, &options)
            .expect("create agent");
        assert_eq!(agent.name(), "anthropic-agent");

        let result = agent.run(&prompt("hello"), &Context::now()).await;
        assert!(
            result.success,
            "{}",
            result.output.body.as_text().unwrap_or("unknown")
        );
        assert_eq!(result.output.body.as_text().unwrap_or(""), "anthropic-ok");
        assert_eq!(result.usage.input_tokens, 12);
        assert_eq!(result.usage.output_tokens, 34);

        let request = captured
            .lock()
            .expect("capture lock")
            .clone()
            .expect("request captured");
        let request_lower = request.to_lowercase();
        assert!(request.starts_with("POST /v1/messages HTTP/1.1"));
        assert!(request_lower.contains("x-api-key:"));
        assert!(request_lower.contains("anthropic-version:"));

        let body = request.split("\r\n\r\n").nth(1).expect("request body");
        let body: serde_json::Value = serde_json::from_str(body).expect("valid request body");
        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(body["max_tokens"], 1_024);
        assert_eq!(body["system"], "system guidance");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "hello");

        handle.join().expect("server thread");
    }
}
