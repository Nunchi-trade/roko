use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use roko_agent::codex_agent::CodexAgent;
use roko_agent::http::{HttpPostError, HttpPoster};
use roko_agent::provider::{AgentOptions, create_agent_for_model};
use roko_agent::Agent;
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig, RokoConfig};
use roko_core::{Body, Context, Kind, Signal};
use serde_json::Value;

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
        tool_format: "openai_json".to_string(),
        cost_input_per_m: None,
        cost_output_per_m: None,
        cost_cache_read_per_m: None,
        cost_cache_write_per_m: None,
        max_tools: None,
        tokenizer_ratio: None,
    }
}

fn zai_config(base_url: impl Into<String>) -> RokoConfig {
    let mut config = RokoConfig::default();
    config.providers.insert("zai".to_string(), zai_provider_config(base_url));
    config.models.insert("glm-5-1".to_string(), glm_5_1_model());
    config
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

    let result = agent.run(&prompt("Reply with the single word pong."), &Context::now()).await;
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
    assert_eq!(request.url, "https://api.z.ai/api/paas/v4/v1/chat/completions");
    assert_eq!(request.timeout_ms, 120_000);

    let parsed: Value = serde_json::from_str(&request.body).expect("request json");
    assert_eq!(parsed["model"], "glm-5.1");
    assert_eq!(parsed["messages"][0]["content"], "Reply with the single word pong.");
    assert!(request
        .headers
        .iter()
        .any(|(name, value)| name.eq_ignore_ascii_case("authorization") && value == "Bearer test-key"));
    assert!(request
        .headers
        .iter()
        .any(|(name, value)| name.eq_ignore_ascii_case("content-type") && value == "application/json"));
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
    assert_eq!(parsed["messages"][0]["content"], "Reply with the single word pong.");
    assert!(request
        .headers
        .iter()
        .any(|(name, value)| name.eq_ignore_ascii_case("authorization") && value == "Bearer test-key"));
    assert!(request
        .headers
        .iter()
        .any(|(name, value)| name.eq_ignore_ascii_case("content-type") && value == "application/json"));
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

        let output = result.output.body.as_text().unwrap_or("").trim().to_lowercase();
        assert!(
            !output.is_empty(),
            "expected non-empty response from Z.AI"
        );
        assert!(
            output.contains("pong"),
            "expected the live response to mention pong, got: {output}"
        );
    }
}
