use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use roko_agent::Agent;
use roko_agent::codex_agent::CodexAgent;
use roko_agent::http::{HttpPostError, HttpPoster};
use roko_agent::provider::{AgentOptions, create_agent_for_model};
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig, RokoConfig};
use roko_core::{Body, Context, Kind, Signal};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

fn prompt(text: &str) -> Signal {
    Signal::builder(Kind::Prompt).body(Body::text(text)).build()
}

fn zai_provider_config(base_url: impl Into<String>) -> ProviderConfig {
    ProviderConfig {
        kind: ProviderKind::OpenAiCompat,
        base_url: Some(base_url.into()),
        api_key_env: Some("ZAI_API_KEY".to_string()),
        command: None,
        args: None,
        timeout_ms: Some(1_500),
        ttft_timeout_ms: Some(15_000),
        connect_timeout_ms: Some(5_000),
        extra_headers: None,
        max_concurrent: None,
    }
}

fn glm_5_1_model() -> ModelProfile {
    ModelProfile {
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

fn zai_config(base_url: impl Into<String>) -> RokoConfig {
    let mut config = RokoConfig::default();
    config
        .providers
        .insert("zai".to_string(), zai_provider_config(base_url));
    config.models.insert("glm-5-1".to_string(), glm_5_1_model());
    config
}

fn ollama_provider_config(base_url: impl Into<String>) -> ProviderConfig {
    ProviderConfig {
        kind: ProviderKind::OpenAiCompat,
        base_url: Some(base_url.into()),
        api_key_env: Some("PATH".to_string()),
        command: None,
        args: None,
        timeout_ms: Some(1_500),
        ttft_timeout_ms: Some(15_000),
        connect_timeout_ms: Some(5_000),
        extra_headers: None,
        max_concurrent: None,
    }
}

fn ollama_local_model() -> ModelProfile {
    ModelProfile {
        provider: "ollama".to_string(),
        slug: "llama3.1:8b".to_string(),
        context_window: 128_000,
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
        supports_search: false,
        supports_citations: false,
        supports_async: false,
        is_embedding_model: false,
        search_context_size: None,
        cost_per_request: None,
    }
}

fn ollama_local_config(base_url: impl Into<String>) -> RokoConfig {
    let mut config = RokoConfig::default();
    config
        .providers
        .insert("ollama".to_string(), ollama_provider_config(base_url));
    config
        .models
        .insert("ollama-local".to_string(), ollama_local_model());
    config
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

#[async_trait]
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
async fn glm_zai_direct() {
    let response = serde_json::json!({
        "id": "chatcmpl-test",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "pong"},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 11,
            "completion_tokens": 7,
            "total_tokens": 18
        }
    })
    .to_string();
    let (poster, captured) = MockPoster::new(response);

    let agent = CodexAgent::new("test-key", "glm-5.1")
        .with_base_url("https://api.z.ai/api/paas/v4")
        .with_http_poster(poster)
        .with_name("glm-zai-direct");

    let result = agent
        .run(&prompt("Reply with the single word pong."), &Context::now())
        .await;
    assert!(
        result.success,
        "{}",
        result.output.body.as_text().unwrap_or("unknown")
    );
    assert_eq!(result.output.body.as_text().unwrap_or(""), "pong");
    assert_eq!(result.usage.input_tokens, 11);
    assert_eq!(result.usage.output_tokens, 7);

    let request = captured
        .lock()
        .expect("capture lock")
        .clone()
        .expect("captured request");
    assert_eq!(
        request.url,
        "https://api.z.ai/api/paas/v4/v1/chat/completions"
    );
    assert_eq!(request.timeout_ms, 120_000);

    let parsed: Value = serde_json::from_str(&request.body).expect("request json");
    assert_eq!(parsed["model"], "glm-5.1");
    assert_eq!(
        parsed["messages"][0]["content"],
        "Reply with the single word pong."
    );
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
}

#[tokio::test]
async fn glm_openrouter() {
    let response = serde_json::json!({
        "id": "chatcmpl-test",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "pong"},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 15,
            "completion_tokens": 8,
            "total_tokens": 23
        }
    })
    .to_string();
    let (poster, captured) = MockPoster::new(response);

    let agent = CodexAgent::new("test-key", "z-ai/glm-5.1")
        .with_base_url("https://openrouter.ai/api")
        .with_http_poster(poster)
        .with_name("glm-openrouter");

    let result = agent
        .run(&prompt("Reply with the single word pong."), &Context::now())
        .await;
    assert!(
        result.success,
        "{}",
        result.output.body.as_text().unwrap_or("unknown")
    );
    assert_eq!(result.output.body.as_text().unwrap_or(""), "pong");
    assert_eq!(result.usage.input_tokens, 15);
    assert_eq!(result.usage.output_tokens, 8);

    let request = captured
        .lock()
        .expect("capture lock")
        .clone()
        .expect("captured request");
    assert_eq!(request.url, "https://openrouter.ai/api/v1/chat/completions");
    assert_eq!(request.timeout_ms, 120_000);

    let parsed: Value = serde_json::from_str(&request.body).expect("request json");
    assert_eq!(parsed["model"], "z-ai/glm-5.1");
    assert_eq!(
        parsed["messages"][0]["content"],
        "Reply with the single word pong."
    );
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
}

#[tokio::test]
async fn kimi_moonshot_direct() {
    let response = serde_json::json!({
        "id": "chatcmpl-test",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "pong"},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 13,
            "completion_tokens": 9,
            "total_tokens": 22
        }
    })
    .to_string();
    let (poster, captured) = MockPoster::new(response);

    let agent = CodexAgent::new("test-key", "kimi-k2.5")
        .with_base_url("https://api.moonshot.ai")
        .with_http_poster(poster)
        .with_name("kimi-moonshot-direct");

    let result = agent
        .run(&prompt("Reply with the single word pong."), &Context::now())
        .await;
    assert!(
        result.success,
        "{}",
        result.output.body.as_text().unwrap_or("unknown")
    );
    assert_eq!(result.output.body.as_text().unwrap_or(""), "pong");
    assert_eq!(result.usage.input_tokens, 13);
    assert_eq!(result.usage.output_tokens, 9);

    let request = captured
        .lock()
        .expect("capture lock")
        .clone()
        .expect("captured request");
    assert_eq!(request.url, "https://api.moonshot.ai/v1/chat/completions");
    assert_eq!(request.timeout_ms, 120_000);

    let parsed: Value = serde_json::from_str(&request.body).expect("request json");
    assert_eq!(parsed["model"], "kimi-k2.5");
    assert_eq!(
        parsed["messages"][0]["content"],
        "Reply with the single word pong."
    );
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
}

#[tokio::test]
async fn ollama_local_via_factory_uses_mock_server() {
    let response = serde_json::json!({
        "id": "chatcmpl-test",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "ollama-ok"},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 8,
            "completion_tokens": 4,
            "total_tokens": 12
        }
    })
    .to_string();
    let (base_url, captured, handle) = spawn_chat_server(response);
    let config = ollama_local_config(base_url);
    let options = AgentOptions {
        timeout_ms: Some(2_500),
        name: "ollama-local-agent".to_string(),
        ..Default::default()
    };

    let agent = create_agent_for_model(&config, "ollama-local", options)
        .expect("create ollama-local agent");
    assert_eq!(agent.name(), "ollama-local-agent");

    let result = agent
        .run(&prompt("hello from ollama"), &Context::now())
        .await;
    assert!(
        result.success,
        "{}",
        result.output.body.as_text().unwrap_or("unknown")
    );
    assert_eq!(result.output.body.as_text().unwrap_or(""), "ollama-ok");
    assert_eq!(result.usage.input_tokens, 8);
    assert_eq!(result.usage.output_tokens, 4);

    let request = captured
        .lock()
        .expect("capture lock")
        .take()
        .expect("captured request");
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));

    let body = request.split("\r\n\r\n").nth(1).expect("request body");
    let parsed: serde_json::Value = serde_json::from_str(body).expect("json request body");
    assert_eq!(parsed["model"], "llama3.1:8b");
    assert_eq!(parsed["messages"][0]["content"], "hello from ollama");

    handle.join().expect("server thread");
}

#[cfg(feature = "integration")]
mod live {
    use super::*;

    #[tokio::test]
    async fn glm_zai_direct_live_http() {
        if std::env::var_os("ZAI_API_KEY").is_none() {
            eprintln!("skipping Z.AI live integration test: ZAI_API_KEY is not set");
            return;
        }

        let config = zai_config("https://api.z.ai/api/paas/v4");
        let options = AgentOptions {
            timeout_ms: Some(120_000),
            name: "glm-zai-live".to_string(),
            ..Default::default()
        };

        let agent = create_agent_for_model(&config, "glm-5-1", options).expect("create agent");
        assert_eq!(agent.name(), "glm-zai-live");

        let result = agent
            .run(&prompt("Reply with the single word pong."), &Context::now())
            .await;
        assert!(
            result.success,
            "{}",
            result.output.body.as_text().unwrap_or("unknown")
        );

        let output = result
            .output
            .body
            .as_text()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        assert!(!output.is_empty(), "expected non-empty response from Z.AI");
        assert!(
            output.contains("pong"),
            "expected the live response to mention pong, got: {output}"
        );
    }
}
