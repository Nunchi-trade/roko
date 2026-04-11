use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use roko_agent::Agent;
use roko_agent::codex_agent::CodexAgent;
use roko_agent::http::{HttpPostError, HttpPoster};
use roko_agent::translate::ChatResponse;
use roko_agent::translate::openai::parse_glm_metadata;
use roko_core::config::schema::ProviderRouting;
use roko_core::{Body, Context, Kind, Signal};
use serde_json::{Map, Value};

fn prompt(text: &str) -> Signal {
    Signal::builder(Kind::Prompt).body(Body::text(text)).build()
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
async fn openrouter_glm() {
    let response = serde_json::json!({
        "id": "chatcmpl-test",
        "model": "z-ai/glm-5.1",
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

    let mut extra_headers = HashMap::new();
    extra_headers.insert(
        "HTTP-Referer".to_string(),
        "https://github.com/nunchi/roko".to_string(),
    );
    extra_headers.insert("X-Title".to_string(), "roko-agent".to_string());

    let mut extra_body_params = Map::new();
    extra_body_params.insert(
        "provider".to_string(),
        serde_json::to_value(ProviderRouting {
            sort: Some("price".to_string()),
            order: Some(vec!["z-ai".to_string(), "moonshotai".to_string()]),
            allow_fallbacks: Some(true),
            max_price: None,
            require_parameters: None,
        })
        .expect("serialize provider routing"),
    );

    let agent = CodexAgent::new("test-key", "z-ai/glm-5.1")
        .with_base_url("https://openrouter.ai/api")
        .with_extra_headers(extra_headers)
        .with_extra_body_params(extra_body_params)
        .with_http_poster(poster)
        .with_name("openrouter-glm");

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
        parsed["provider"],
        serde_json::json!({
            "sort": "price",
            "order": ["z-ai", "moonshotai"],
            "allow_fallbacks": true
        })
    );
    assert!(
        request
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("HTTP-Referer")
                && value == "https://github.com/nunchi/roko")
    );
    assert!(
        request
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("X-Title")
                && value == "roko-agent")
    );
}

#[tokio::test]
async fn openrouter_fallback() {
    let response = serde_json::json!({
        "id": "chatcmpl-test",
        "model": "z-ai/glm-5",
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
    let (poster, captured) = MockPoster::new(response.clone());

    let mut extra_headers = HashMap::new();
    extra_headers.insert(
        "HTTP-Referer".to_string(),
        "https://github.com/nunchi/roko".to_string(),
    );
    extra_headers.insert("X-Title".to_string(), "roko-agent".to_string());

    let mut extra_body_params = Map::new();
    extra_body_params.insert(
        "provider".to_string(),
        serde_json::to_value(ProviderRouting {
            sort: Some("price".to_string()),
            order: Some(vec!["z-ai".to_string(), "moonshotai".to_string()]),
            allow_fallbacks: Some(true),
            max_price: None,
            require_parameters: None,
        })
        .expect("serialize provider routing"),
    );

    let agent = CodexAgent::new("test-key", "z-ai/glm-5.1")
        .with_base_url("https://openrouter.ai/api")
        .with_extra_headers(extra_headers)
        .with_extra_body_params(extra_body_params)
        .with_http_poster(poster)
        .with_name("openrouter-fallback");

    let result = agent
        .run(&prompt("Reply with the single word pong."), &Context::now())
        .await;
    assert!(
        result.success,
        "{}",
        result.output.body.as_text().unwrap_or("unknown")
    );
    assert_eq!(result.output.body.as_text().unwrap_or(""), "pong");

    let parsed_response: Value = serde_json::from_str(&response).expect("response json");
    let chat_response = ChatResponse {
        metadata: parse_glm_metadata(&parsed_response),
        ..Default::default()
    };
    assert_eq!(
        chat_response.metadata.model_used.as_deref(),
        Some("z-ai/glm-5")
    );

    let request = captured
        .lock()
        .expect("capture lock")
        .clone()
        .expect("captured request");
    assert_eq!(request.url, "https://openrouter.ai/api/v1/chat/completions");
    assert_eq!(request.timeout_ms, 120_000);
}
