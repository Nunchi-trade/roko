use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use roko_agent::dispatcher::ToolDispatcher;
use roko_agent::http::{HttpPostError, HttpPoster};
use roko_agent::tool_loop::{LlmBackend, LlmError, StopReason, ToolLoop};
use roko_agent::translate::{BackendResponse, OpenAiTranslator, RenderedTools, Translator};
use roko_core::tool::{ToolContext, ToolDef};
use roko_std::tool::builtin::{edit_file, read_file};
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
        self.requests.lock().expect("requests lock").push(RecordedRequest {
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
struct GlmHttpBackend {
    poster: Arc<MockHttpPoster>,
    base_url: String,
    model: String,
    reasoning: Arc<Mutex<Vec<String>>>,
}

impl GlmHttpBackend {
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
        format!(
            "{}/chat/completions",
            self.base_url.trim_end_matches('/')
        )
    }
}

#[async_trait]
impl LlmBackend for GlmHttpBackend {
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

fn edit_tools() -> Vec<ToolDef> {
    vec![read_file::tool_def(), edit_file::tool_def()]
}

fn tool_context(worktree: &std::path::Path) -> ToolContext {
    ToolContext::testing(worktree)
}

#[tokio::test]
async fn glm_full_tool_loop() {
    let tempdir = tempdir().expect("tempdir");
    let file_path = tempdir.path().join("note.txt");
    tokio::fs::write(&file_path, "hello world")
        .await
        .expect("seed file");

    let first_response = serde_json::json!({
        "id": "chatcmpl-glm-1",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "",
                "reasoning_content": "I should replace the greeting in the file before answering.",
                "tool_calls": [{
                    "id": "call-edit-1",
                    "type": "function",
                    "function": {
                        "name": "edit_file",
                        "arguments": serde_json::json!({
                            "path": "note.txt",
                            "old_string": "hello",
                            "new_string": "goodbye"
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
            "prompt_tokens_details": {
                "cached_tokens": 4
            }
        }
    })
    .to_string();

    let second_response = serde_json::json!({
        "id": "chatcmpl-glm-2",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Updated the file."
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
        GlmHttpBackend::new(poster.clone(), "https://api.z.ai/api/paas/v4", "glm-5.1");

    let registry = Arc::new(StaticToolRegistry::new());
    let resolver = Arc::new(|name: &str| handler_for(name));
    let dispatcher = Arc::new(ToolDispatcher::new(registry, resolver));
    let translator: Arc<dyn Translator> = Arc::new(OpenAiTranslator);
    let loop_runner = ToolLoop::new(translator, dispatcher, backend);

    let result = loop_runner
        .run(
            "You are a careful file-editing assistant.",
            "Update note.txt using the available tools.",
            &edit_tools(),
            &tool_context(tempdir.path()),
        )
        .await;

    assert_eq!(result.stop_reason, StopReason::Stop);
    assert_eq!(result.iterations, 1);
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].name, "edit_file");
    assert_eq!(result.tool_calls[0].id, "call-edit-1");
    assert_eq!(result.final_text, "Updated the file.");

    let captured_reasoning = reasoning.lock().expect("reasoning lock").clone();
    assert_eq!(captured_reasoning.len(), 1);
    assert_eq!(
        captured_reasoning[0],
        "I should replace the greeting in the file before answering."
    );

    let requests = poster.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].url,
        "https://api.z.ai/api/paas/v4/chat/completions"
    );
    assert_eq!(requests[0].timeout_ms, 120_000);
    assert!(
        requests[0].headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("authorization") && value == "Bearer test-key"
        }),
        "expected auth header on first request"
    );
    assert!(
        requests[0].headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("content-type") && value == "application/json"
        }),
        "expected content-type header on first request"
    );
    assert!(
        requests[0]
            .body
            .get("tools")
            .and_then(Value::as_array)
            .is_some(),
        "expected tools array in first request"
    );
    assert_eq!(requests[0].body["tools"][0]["function"]["name"], "read_file");
    assert_eq!(requests[0].body["tools"][1]["function"]["name"], "edit_file");

    let first_turn_messages = requests[0]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .expect("first request messages");
    assert_eq!(first_turn_messages[0]["role"], "system");
    assert_eq!(first_turn_messages[1]["role"], "user");

    let second_turn_messages = requests[1]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .expect("second request messages");
    let tool_message = second_turn_messages
        .iter()
        .find(|msg| msg.get("tool_call_id").is_some())
        .expect("tool result message");
    assert_eq!(tool_message["tool_call_id"], "call-edit-1");
    assert!(
        tool_message["content"]
            .as_str()
            .expect("tool content")
            .contains("edited"),
        "expected tool result to be rendered back to the model"
    );

    let updated = tokio::fs::read_to_string(&file_path)
        .await
        .expect("read edited file");
    assert_eq!(updated, "goodbye world");
}
