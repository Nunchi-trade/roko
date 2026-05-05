use crate::Agent;
use crate::cursor_agent::{CursorAgent, DEFAULT_BASE_URL};
use crate::provider::{
    AgentCreationError, AgentOptions, ProviderAdapter, ProviderError, current_safety_layer,
};
use crate::safety::SafetyLayer;
use roko_core::agent::ProviderKind;
#[cfg(test)]
use roko_core::config::DEFAULT_TTFT_TIMEOUT_MS;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use roko_core::defaults::DEFAULT_REQUEST_TIMEOUT_MS;
use serde_json::Value;

/// Adapter for the Cursor ACP HTTP fallback.
pub struct CursorAcpAdapter;

impl CursorAcpAdapter {
    fn base_url(provider: &ProviderConfig) -> String {
        let base_url = provider
            .base_url
            .as_deref()
            .unwrap_or(DEFAULT_BASE_URL)
            .trim_end_matches('/');

        base_url.strip_suffix("/v1").unwrap_or(base_url).to_string()
    }
}

impl ProviderAdapter for CursorAcpAdapter {
    fn kind(&self) -> ProviderKind {
        ProviderKind::CursorAcp
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
            .unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS);

        let mut agent = CursorAgent::new(
            api_key,
            model.slug.clone(),
            current_safety_layer().unwrap_or_else(SafetyLayer::with_defaults),
        )
        .with_base_url(Self::base_url(provider))
        .with_timeout_ms(timeout_ms);

        if let Some(headers) = &provider.extra_headers {
            agent = agent.with_extra_headers(headers.clone());
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
    use roko_core::{Body, Context, Signal, Kind};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    fn prompt(text: &str) -> Signal {
        Signal::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    fn spawn_prompt_server(
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

    fn cursor_model() -> ModelProfile {
        ModelProfile {
            provider: "cursor".to_string(),
            slug: "cursor-composer".to_string(),
            context_window: 200_000,
            max_output: Some(2_048),
            supports_tools: true,
            supports_thinking: false,
            supports_vision: false,
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: false,
            supports_grounding: false,
            supports_code_execution: false,
            supports_caching: false,
            provider_routing: None,
            tool_format: "openai_json".to_string(),
            cost_input_per_m: None,
            cost_output_per_m: None,
            cost_input_per_m_high: None,
            cost_output_per_m_high: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            thinking_level: None,
            max_tools: None,
            tokenizer_ratio: None,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn cursor_acp_adapter_creates_agent_and_posts_acp_request() {
        let response = serde_json::json!({
            "session_id": "sess_123",
            "model": "cursor-composer",
            "messages": [
                {"role": "assistant", "content": "cursor-ok"}
            ],
            "usage": {
                "input_tokens": 14,
                "output_tokens": 29
            },
            "stop_reason": "end_turn"
        })
        .to_string();
        let (base_url, captured, handle) = spawn_prompt_server(response);

        let provider = ProviderConfig {
            kind: ProviderKind::CursorAcp,
            base_url: Some(format!("{base_url}/v1/")),
            api_key_env: Some("PATH".to_string()),
            command: None,
            args: None,
            timeout_ms: Some(1_500),
            ttft_timeout_ms: Some(DEFAULT_TTFT_TIMEOUT_MS),
            connect_timeout_ms: Some(5_000),
            extra_headers: None,
            max_concurrent: None,
        };
        let options = AgentOptions {
            timeout_ms: Some(2_500),
            name: "cursor-adapter".to_string(),
            ..Default::default()
        };
        let model = cursor_model();

        let adapter = CursorAcpAdapter;
        assert_eq!(adapter.kind(), ProviderKind::CursorAcp);

        let agent = adapter
            .create_agent(&provider, &model, &options)
            .expect("create agent");
        assert_eq!(agent.name(), "cursor-adapter");
        assert!(agent.supports_streaming());

        let result = agent.run(&prompt("hello"), &Context::now()).await;
        assert!(
            result.success,
            "{}",
            result.output.body.as_text().unwrap_or("unknown")
        );
        assert_eq!(result.output.body.as_text().unwrap_or(""), "cursor-ok");
        assert_eq!(result.usage.input_tokens, 14);
        assert_eq!(result.usage.output_tokens, 29);

        let request = captured
            .lock()
            .expect("capture lock")
            .clone()
            .expect("request captured");
        assert!(request.starts_with("POST /v1/prompt HTTP/1.1"));
        assert!(request.to_lowercase().contains("x-cursor-protocol: acp/1"));

        let body = request.split("\r\n\r\n").nth(1).expect("request body");
        let body: serde_json::Value = serde_json::from_str(body).expect("valid request body");
        assert_eq!(body["protocol"], "acp/1");
        assert_eq!(body["model"], "cursor-composer");
        assert_eq!(body["prompt"]["role"], "user");
        assert_eq!(body["prompt"]["content"], "hello");

        handle.join().expect("server thread");
    }
}
