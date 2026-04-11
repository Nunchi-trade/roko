use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use roko_agent::OpenAiCompatLlmBackend;
use roko_agent::Usage;
use roko_agent::dispatcher::ToolDispatcher;
use roko_agent::tool_loop::{StopReason, ToolLoop};
use roko_agent::translate::{OpenAiTranslator, Translator};
use roko_core::tool::{ToolContext, ToolDef};
use roko_std::tool::builtin::read_file;
use roko_std::tool::handlers::handler_for;
use roko_std::tool::registry::StaticToolRegistry;
use serde_json::Value;
use tempfile::tempdir;

fn spawn_chat_server(
    responses: Vec<String>,
) -> (String, Arc<Mutex<Vec<Value>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_requests = Arc::clone(&captured);

    let handle = thread::spawn(move || {
        for response in responses {
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
            let body = &buf[header_end..header_end + content_length];
            let request: Value = serde_json::from_slice(body).expect("request body json");
            captured_requests
                .lock()
                .expect("capture lock")
                .push(request);

            let response_bytes = response.as_bytes();
            let wire = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_bytes.len(),
                response
            );
            stream.write_all(wire.as_bytes()).expect("write response");
            stream.flush().expect("flush response");
        }
    });

    (format!("http://{addr}/v1"), captured, handle)
}

fn read_tools() -> Vec<ToolDef> {
    vec![read_file::tool_def()]
}

fn tool_context(worktree: &std::path::Path) -> ToolContext {
    ToolContext::testing(worktree)
}

#[tokio::test]
async fn tool_loop_glm_e2e() {
    let tempdir = tempdir().expect("tempdir");
    let src_dir = tempdir.path().join("src");
    tokio::fs::create_dir_all(&src_dir)
        .await
        .expect("create src dir");
    tokio::fs::write(
        src_dir.join("lib.rs"),
        "pub fn answer() -> u32 {\n    42\n}\n",
    )
    .await
    .expect("seed src/lib.rs");

    let first_response = serde_json::json!({
        "id": "chatcmpl-glm-1",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "",
                "reasoning_content": "I need to read the file before I can answer.",
                "tool_calls": [{
                    "id": "call-read-1",
                    "type": "function",
                    "function": {
                        "name": "read_file",
                        "arguments": serde_json::json!({
                            "path": "src/lib.rs"
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
                "content": "The file contains...",
                "reasoning_content": "Based on the file, I can answer now."
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 17,
            "completion_tokens": 4,
            "total_tokens": 21,
            "prompt_tokens_details": {
                "cached_tokens": 2
            }
        }
    })
    .to_string();

    let (base_url, captured_requests, handle) =
        spawn_chat_server(vec![first_response, second_response]);

    let backend = OpenAiCompatLlmBackend::new("test-key", "glm-5.1")
        .with_base_url(base_url)
        .with_timeout_ms(120_000);

    let registry = Arc::new(StaticToolRegistry::new());
    let resolver = Arc::new(|name: &str| handler_for(name));
    let dispatcher = Arc::new(ToolDispatcher::new(registry, resolver));
    let translator: Arc<dyn Translator> = Arc::new(OpenAiTranslator);
    let loop_runner = ToolLoop::new(translator, dispatcher, Arc::new(backend));

    let result = loop_runner
        .run(
            "You are a careful file-reading assistant.",
            "Read the file src/lib.rs",
            &read_tools(),
            &tool_context(tempdir.path()),
        )
        .await;

    handle.join().expect("server thread");

    assert_eq!(result.stop_reason, StopReason::Stop);
    assert_eq!(result.final_text, "The file contains...");
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].id, "call-read-1");
    assert_eq!(result.tool_calls[0].name, "read_file");
    assert_eq!(
        result.total_usage,
        Usage {
            input_tokens: 38,
            output_tokens: 13,
            cache_read_tokens: 6,
            ..Default::default()
        }
    );

    let requests = captured_requests.lock().expect("capture lock");
    assert_eq!(requests.len(), 2);

    let second_turn_messages = requests[1]["messages"]
        .as_array()
        .expect("second turn messages");
    let assistant_message = second_turn_messages
        .iter()
        .find(|msg| msg["role"] == "assistant")
        .expect("assistant tool call message");
    assert_eq!(
        assistant_message["reasoning_content"],
        "I need to read the file before I can answer."
    );

    let tool_message = second_turn_messages
        .iter()
        .find(|msg| msg.get("tool_call_id").is_some())
        .expect("tool result message");
    assert_eq!(tool_message["tool_call_id"], "call-read-1");
    assert_eq!(
        tool_message["content"],
        "pub fn answer() -> u32 {\n    42\n}\n"
    );
}
