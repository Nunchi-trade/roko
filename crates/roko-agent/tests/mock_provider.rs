use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

pub async fn mock_openai_compat() -> (MockServer, String) {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(default_openai_response())
        .mount(&server)
        .await;

    let base_url = server.uri();
    (server, base_url)
}

pub async fn mock_openai_with_tool_calls() -> (MockServer, String) {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(OpenAiToolCallSequence::default())
        .mount(&server)
        .await;

    let base_url = server.uri();
    (server, base_url)
}

fn default_openai_response() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "id": "mock-1",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Mock response"
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }
    }))
}

#[derive(Default)]
struct OpenAiToolCallSequence {
    request_count: AtomicUsize,
}

impl Respond for OpenAiToolCallSequence {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        match self.request_count.fetch_add(1, Ordering::SeqCst) {
            0 => ResponseTemplate::new(200).set_body_json(json!({
                "id": "mock-tool-1",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "id": "call-mock-1",
                            "type": "function",
                            "function": {
                                "name": "read_file",
                                "arguments": "{\"path\":\"note.txt\"}"
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": 6,
                    "total_tokens": 18
                }
            })),
            _ => ResponseTemplate::new(200).set_body_json(json!({
                "id": "mock-tool-2",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Mock tool loop complete"
                    },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 14,
                    "completion_tokens": 4,
                    "total_tokens": 18
                }
            })),
        }
    }
}
