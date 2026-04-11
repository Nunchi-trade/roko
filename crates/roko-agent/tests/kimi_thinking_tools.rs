use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use roko_agent::dispatcher::ToolDispatcher;
use roko_agent::http::{HttpPostError, HttpPoster};
use roko_agent::tool_loop::{LlmBackend, LlmError, StopReason, ToolLoop};
use roko_agent::translate::{BackendResponse, OpenAiTranslator, RenderedTools, Translator};
use roko_core::tool::{ToolContext, ToolDef};
use roko_std::tool::builtin::read_file;
use roko_std::tool::handlers::handler_for;
use roko_std::tool::registry::StaticToolRegistry;
use serde_json::Value;
use tempfile::tempdir;

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

#[derive(Debug)]
struct KimiHttpBackend {
    poster: Arc<MockHttpPoster>,
    base_url: String,
    model: String,
    reasoning: Arc<Mutex<Vec<String>>>,
}

impl KimiHttpBackend {
    fn new(
        poster: Arc<MockHttpPoster>,
        base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> (Arc<Self>, Arc<Mutex<Vec<String>>>) {
        let reasoning = Arc::new(Mutex::new(Vec::new()));
        let backend = Arc::new(Self {
            poster,
            base_url: base_url.into(),
            model: model.into(),
            reasoning: reasoning.clone(),
        });
        (backend, reasoning)
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }
}

#[async_trait]
impl LlmBackend for KimiHttpBackend {
    async fn send_turn(
        &self,
        messages: &[Value],
        tools: &RenderedTools,
    ) -> Result<BackendResponse, LlmError> {
        let RenderedTools::JsonArray(tools) = tools else {
            return Err(LlmError::Backend("expected json tool array".into()));
        };

        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "tools": tools,
        });
        let body_bytes = serde_json::to_vec(&body)
            .map_err(|e| LlmError::Backend(format!("serialize request failed: {e}")))?;

        let response_text = self
            .poster
            .post_json(
                &self.endpoint(),
                &[
                    ("authorization".to_string(), "Bearer test-key".to_string()),
                    ("content-type".to_string(), "application/json".to_string()),
                ],
                &body_bytes,
                120_000,
            )
            .await
            .map_err(|e| LlmError::Network(e.to_string()))?;

        let response_json: Value = serde_json::from_str(&response_text)
            .map_err(|e| LlmError::Backend(format!("malformed response json: {e}")))?;
        let response = BackendResponse::Json(response_json);
        if let Some(reasoning) = response.extract_reasoning() {
            self.reasoning
                .lock()
                .expect("reasoning lock")
                .push(reasoning);
        }
        Ok(response)
    }
}

fn tool_definitions() -> Vec<ToolDef> {
    vec![read_file::tool_def()]
}

fn tool_context(worktree: &std::path::Path) -> ToolContext {
    ToolContext::testing(worktree)
}

#[tokio::test]
async fn kimi_thinking_with_tools() {
    let tempdir = tempdir().expect("tempdir");
    let file_path = tempdir.path().join("note.txt");
    tokio::fs::write(&file_path, "kimi needs a quick read")
        .await
        .expect("seed file");

    let first_response = serde_json::json!({
        "id": "chatcmpl-kimi-1",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "",
                "reasoning_content": "I should inspect the file before answering.",
                "tool_calls": [{
                    "id": "functions.Read:0",
                    "type": "function",
                    "function": {
                        "name": "read_file",
                        "arguments": serde_json::json!({
                            "path": "note.txt"
                        }).to_string()
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 21,
            "completion_tokens": 9,
            "total_tokens": 30,
            "cached_tokens": 4
        }
    })
    .to_string();

    let second_response = serde_json::json!({
        "id": "chatcmpl-kimi-2",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "I read the file and can answer now."
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 17,
            "completion_tokens": 4,
            "total_tokens": 21
        }
    })
    .to_string();

    let poster = MockHttpPoster::new(vec![first_response, second_response]);
    let (backend, reasoning) =
        KimiHttpBackend::new(poster.clone(), "https://api.moonshot.ai/v1", "kimi-k2.5");

    let registry = Arc::new(StaticToolRegistry::new());
    let resolver = Arc::new(|name: &str| handler_for(name));
    let dispatcher = Arc::new(ToolDispatcher::new(registry, resolver));
    let translator: Arc<dyn Translator> = Arc::new(OpenAiTranslator);
    let loop_runner = ToolLoop::new(translator, dispatcher, backend);

    let result = loop_runner
        .run(
            "You are a careful file assistant.",
            "Read the note and respond.",
            &tool_definitions(),
            &tool_context(tempdir.path()),
        )
        .await;

    assert_eq!(result.stop_reason, StopReason::Stop);
    assert_eq!(result.iterations, 1);
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].id, "functions.Read:0");
    assert_eq!(result.tool_calls[0].name, "read_file");
    assert_eq!(result.final_text, "I read the file and can answer now.");

    let captured_reasoning = reasoning.lock().expect("reasoning lock").clone();
    assert_eq!(
        captured_reasoning,
        vec!["I should inspect the file before answering."]
    );

    let requests = poster.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].url,
        "https://api.moonshot.ai/v1/chat/completions"
    );
    assert_eq!(requests[0].timeout_ms, 120_000);
    assert!(
        requests[0].headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("authorization") && value == "Bearer test-key"
        }),
        "expected auth header on first request"
    );
    assert_eq!(
        requests[0].body["messages"].as_array().map(Vec::len),
        Some(2)
    );

    assert_eq!(
        requests[1].url,
        "https://api.moonshot.ai/v1/chat/completions"
    );
    let messages = requests[1].body["messages"]
        .as_array()
        .expect("messages array");
    assert_eq!(messages.len(), 4);
    let assistant = messages
        .iter()
        .find(|message| message["role"] == "assistant")
        .expect("assistant message in history");
    assert_eq!(
        assistant["reasoning_content"],
        "I should inspect the file before answering."
    );
    assert_eq!(assistant["content"], "");
    assert_eq!(assistant["tool_calls"][0]["id"], "functions.Read:0");
    assert_eq!(assistant["tool_calls"][0]["function"]["name"], "read_file");
}
