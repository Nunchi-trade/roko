use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use roko_agent::http::{HttpPostError, HttpPoster};
use roko_agent::translate::openai::build_partial_continuation;
use serde_json::Value;

#[derive(Debug, Clone)]
struct RecordedRequest {
    url: String,
    headers: Vec<(String, String)>,
    body: Value,
    timeout_ms: u64,
}

#[derive(Debug)]
struct MockHttpPoster {
    responses: Mutex<VecDeque<String>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

impl MockHttpPoster {
    fn new(responses: Vec<String>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

#[async_trait]
impl HttpPoster for MockHttpPoster {
    async fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &[u8],
        timeout_ms: u64,
    ) -> Result<String, HttpPostError> {
        let body: Value = serde_json::from_slice(body).expect("request body must be json");
        self.requests
            .lock()
            .expect("requests lock")
            .push(RecordedRequest {
                url: url.to_string(),
                headers: headers.to_vec(),
                body,
                timeout_ms,
            });

        self.responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .ok_or_else(|| HttpPostError::transport("no mock response queued"))
    }
}

fn endpoint(base_url: &str) -> String {
    format!("{}/chat/completions", base_url.trim_end_matches('/'))
}

async fn post_turn(
    poster: &Arc<MockHttpPoster>,
    base_url: &str,
    model: &str,
    messages: &[Value],
) -> Result<Value, String> {
    let body = serde_json::json!({
        "model": model,
        "messages": messages,
    });
    let body_bytes =
        serde_json::to_vec(&body).map_err(|e| format!("serialize request failed: {e}"))?;

    let response_text = poster
        .post_json(
            &endpoint(base_url),
            &[
                ("authorization".to_string(), "Bearer test-key".to_string()),
                ("content-type".to_string(), "application/json".to_string()),
            ],
            &body_bytes,
            120_000,
        )
        .await
        .map_err(|e| e.to_string())?;

    serde_json::from_str(&response_text).map_err(|e| format!("malformed response json: {e}"))
}

fn assistant_content(response: &Value) -> &str {
    response["choices"][0]["message"]["content"]
        .as_str()
        .expect("assistant content")
}

#[tokio::test]
async fn kimi_partial_flow() {
    let prompt = "Write the answer in two parts.";
    let first_part = "Kimi hit the limit, so this is the first half ";
    let second_part = "and this is the rest.";
    let expected_output = format!("{first_part}{second_part}");

    let first_response = serde_json::json!({
        "id": "chatcmpl-kimi-1",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": first_part
            },
            "finish_reason": "length"
        }],
        "usage": {
            "prompt_tokens": 32,
            "completion_tokens": 16,
            "total_tokens": 48,
            "cached_tokens": 0
        }
    })
    .to_string();

    let second_response = serde_json::json!({
        "id": "chatcmpl-kimi-2",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": second_part
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 18,
            "completion_tokens": 8,
            "total_tokens": 26
        }
    })
    .to_string();

    let poster = MockHttpPoster::new(vec![first_response, second_response]);
    let base_url = "https://api.moonshot.ai/v1";
    let model = "kimi-k2.5";

    let mut messages = vec![serde_json::json!({
        "role": "user",
        "content": prompt
    })];

    let first = post_turn(&poster, base_url, model, &messages)
        .await
        .expect("first request should succeed");
    assert_eq!(first["choices"][0]["finish_reason"], "length");

    let truncated = assistant_content(&first).to_string();
    messages.push(build_partial_continuation(&truncated));

    let second = post_turn(&poster, base_url, model, &messages)
        .await
        .expect("second request should succeed");
    assert_eq!(second["choices"][0]["finish_reason"], "stop");

    let final_output = format!("{}{}", truncated, assistant_content(&second));
    assert_eq!(final_output, expected_output);

    let requests = poster.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].url,
        "https://api.moonshot.ai/v1/chat/completions"
    );
    assert_eq!(requests[0].timeout_ms, 120_000);
    assert_eq!(requests[0].body["model"], model);
    assert!(requests[0].headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("authorization") && value == "Bearer test-key"
    }));
    assert!(requests[0].headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type") && value == "application/json"
    }));
    assert_eq!(
        requests[0].body["messages"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(requests[0].body["messages"][0]["role"], "user");
    assert_eq!(requests[0].body["messages"][0]["content"], prompt);

    assert_eq!(
        requests[1].url,
        "https://api.moonshot.ai/v1/chat/completions"
    );
    assert_eq!(requests[1].body["model"], model);
    assert_eq!(
        requests[1].body["messages"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(requests[1].body["messages"][1]["role"], "assistant");
    assert_eq!(requests[1].body["messages"][1]["content"], truncated);
    assert_eq!(requests[1].body["messages"][1]["partial"], true);
}
