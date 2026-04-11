//! Gemini OpenAI-compatible chat agent.

use crate::agent::{Agent, AgentResult};
use crate::codex_agent::{CodexAgent, DEFAULT_MAX_TOKENS};
use crate::provider::AgentOptions;
use async_trait::async_trait;
use roko_core::config::schema::ModelProfile;
use roko_core::{Context, Signal};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;

fn compat_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{trimmed}/v1beta/openai")
}

fn resolved_timeout_ms(options: &AgentOptions) -> u64 {
    options.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS)
}

fn resolved_max_tokens(model: &ModelProfile) -> u32 {
    model
        .max_output
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(DEFAULT_MAX_TOKENS)
}

fn resolved_name(options: &AgentOptions, default_name: String) -> String {
    if options.name.is_empty() {
        default_name
    } else {
        options.name.clone()
    }
}

/// Gemini agent backed by the OpenAI-compatible `/v1beta/openai` surface.
///
/// Used for models that do not require Gemini-native request/response handling.
pub struct GeminiCompatAgent {
    inner: CodexAgent,
}

impl GeminiCompatAgent {
    /// Construct a Gemini OpenAI-compatible agent.
    #[must_use]
    pub fn new(
        api_key: String,
        base_url: String,
        model: ModelProfile,
        options: &AgentOptions,
    ) -> Self {
        let name = resolved_name(options, format!("gemini-compat:{}", model.slug));
        let inner = CodexAgent::new(api_key, &model.slug)
            .with_base_url(compat_base_url(&base_url))
            .with_timeout_ms(resolved_timeout_ms(options))
            .with_max_tokens(resolved_max_tokens(&model))
            .with_name(name);
        Self { inner }
    }
}

#[async_trait]
impl Agent for GeminiCompatAgent {
    async fn run(&self, input: &Signal, ctx: &Context) -> AgentResult {
        self.inner.run(input, ctx).await
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn supports_streaming(&self) -> bool {
        self.inner.supports_streaming()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    fn base_model() -> ModelProfile {
        ModelProfile {
            provider: "gemini".to_string(),
            slug: "gemini-2.5-flash-lite".to_string(),
            context_window: 1_048_576,
            max_output: Some(65_536),
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
            supports_search: false,
            supports_citations: false,
            supports_async: false,
            is_embedding_model: false,
            search_context_size: None,
            cost_per_request: None,
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
    async fn gemini_compat_agent_uses_openai_compat_endpoint_for_flash_lite() {
        let response = serde_json::json!({
            "choices": [
                {
                    "message": {
                        "content": "compat ok"
                    }
                }
            ],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 4
            }
        })
        .to_string();

        let (base_url, captured, handle) = spawn_chat_server(response);
        let agent = GeminiCompatAgent::new(
            "test-key".to_string(),
            base_url,
            base_model(),
            &AgentOptions::default(),
        );
        let input = Signal::builder(Kind::Prompt)
            .body(Body::text("Say hi"))
            .build();

        let result = agent.run(&input, &Context::now()).await;
        handle.join().expect("server thread");

        assert!(result.success);
        assert_eq!(result.output.body.as_text().ok(), Some("compat ok"));

        let request = captured
            .lock()
            .expect("capture lock")
            .clone()
            .expect("captured request");
        assert!(request.starts_with("POST /v1beta/openai/v1/chat/completions HTTP/1.1"));
        assert!(request.contains("\"model\":\"gemini-2.5-flash-lite\""));
        assert!(request.contains("authorization: Bearer test-key"));
    }
}
