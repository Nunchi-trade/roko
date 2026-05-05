#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use roko_agent::cursor_agent::CursorAgent;
use roko_agent::streaming::StreamChunk;
use roko_agent::tool_loop::LlmBackend;
use roko_agent::translate::{OpenAiTranslator, SessionState, Translator};
use roko_agent::{SafetyLayer, Usage};
use roko_core::tool::ToolCall;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum FixtureFrame {
    ReasoningDelta {
        turn: usize,
        text: String,
    },
    Draft {
        turn: usize,
        text: String,
    },
    ToolCall {
        turn: usize,
        index: usize,
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        arguments_delta: String,
    },
    Final {
        turn: usize,
        finish_reason: String,
        response_id: String,
        session_id: String,
        thread_id: String,
        usage: FixtureUsage,
    },
}

impl FixtureFrame {
    fn turn(&self) -> usize {
        match self {
            Self::ReasoningDelta { turn, .. }
            | Self::Draft { turn, .. }
            | Self::ToolCall { turn, .. }
            | Self::Final { turn, .. } => *turn,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FixtureUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    #[serde(default)]
    cached_tokens: u32,
}

impl FixtureUsage {
    fn to_wire(&self) -> Value {
        json!({
            "prompt_tokens": self.prompt_tokens,
            "completion_tokens": self.completion_tokens,
            "total_tokens": self.prompt_tokens + self.completion_tokens,
            "prompt_tokens_details": {
                "cached_tokens": self.cached_tokens,
            },
        })
    }

    fn as_usage(&self) -> Usage {
        Usage {
            input_tokens: self.prompt_tokens,
            output_tokens: self.completion_tokens,
            cache_read_tokens: self.cached_tokens,
            ..Usage::default()
        }
    }
}

#[derive(Debug, Clone)]
struct Fixture {
    reasoning: String,
    content: String,
    usage: Usage,
    response_id: String,
    session_id: String,
    thread_id: String,
    tool_calls: Vec<ExpectedToolCall>,
    response_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpectedToolCall {
    id: String,
    name: String,
    arguments_json: String,
}

#[derive(Debug, Clone, Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments_json: String,
}

impl Fixture {
    fn load(name: &str) -> Self {
        let path = fixture_path(name);
        let raw = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read fixture {}: {err}", path.display()));

        let mut grouped = BTreeMap::<usize, Vec<FixtureFrame>>::new();
        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            let frame: FixtureFrame = serde_json::from_str(line)
                .unwrap_or_else(|err| panic!("parse fixture line: {err}"));
            grouped.entry(frame.turn()).or_default().push(frame);
        }

        let mut reasoning = String::new();
        let mut content = String::new();
        let mut response_lines = Vec::new();
        let mut partial_calls = BTreeMap::<usize, PartialToolCall>::new();
        let mut final_frame = None;

        for (turn, frames) in grouped {
            for frame in frames {
                match frame {
                    FixtureFrame::ReasoningDelta { text, .. } => {
                        reasoning.push_str(&text);
                        response_lines.push(sse_line(json!({
                            "choices": [{
                                "delta": {
                                    "reasoning_content": text,
                                },
                            }],
                        })));
                    }
                    FixtureFrame::Draft { text, .. } => {
                        content.push_str(&text);
                        response_lines.push(sse_line(json!({
                            "choices": [{
                                "delta": {
                                    "content": text,
                                },
                            }],
                        })));
                    }
                    FixtureFrame::ToolCall {
                        index,
                        id,
                        name,
                        arguments_delta,
                        ..
                    } => {
                        let entry = partial_calls.entry(index).or_default();
                        if let Some(id) = id.as_ref() {
                            entry.id = id.clone();
                        }
                        if let Some(name) = name.as_ref() {
                            entry.name = name.clone();
                        }
                        entry.arguments_json.push_str(&arguments_delta);

                        let mut tool_call = serde_json::Map::new();
                        tool_call.insert("index".to_string(), json!(index));
                        if let Some(id) = id {
                            tool_call.insert("id".to_string(), Value::String(id));
                        }
                        let mut function = serde_json::Map::new();
                        if let Some(name) = name {
                            function.insert("name".to_string(), Value::String(name));
                        }
                        if !arguments_delta.is_empty() {
                            function
                                .insert("arguments".to_string(), Value::String(arguments_delta));
                        }
                        if !function.is_empty() {
                            tool_call.insert("function".to_string(), Value::Object(function));
                        }

                        response_lines.push(sse_line(json!({
                            "choices": [{
                                "delta": {
                                    "tool_calls": [Value::Object(tool_call)],
                                },
                            }],
                        })));
                    }
                    FixtureFrame::Final {
                        finish_reason,
                        response_id,
                        session_id,
                        thread_id,
                        usage,
                        ..
                    } => {
                        response_lines.push(sse_line(json!({
                            "id": response_id,
                            "session_id": session_id,
                            "thread_id": thread_id,
                            "choices": [{
                                "delta": {},
                                "finish_reason": finish_reason,
                            }],
                        })));
                        response_lines.push(sse_line(json!({
                            "id": response_id,
                            "session_id": session_id,
                            "thread_id": thread_id,
                            "usage": usage.to_wire(),
                        })));
                        final_frame = Some((response_id, session_id, thread_id, usage.as_usage()));
                    }
                }
            }

            assert_eq!(
                turn, 1,
                "cursor_streaming fixture should stay one-turn small"
            );
        }

        response_lines.push("data: [DONE]\n\n".to_string());

        let (response_id, session_id, thread_id, usage) =
            final_frame.unwrap_or_else(|| panic!("fixture {} missing final frame", path.display()));
        let tool_calls = partial_calls
            .into_values()
            .map(|call| ExpectedToolCall {
                id: call.id,
                name: call.name,
                arguments_json: call.arguments_json,
            })
            .collect();

        Self {
            reasoning,
            content,
            usage,
            response_id,
            session_id,
            thread_id,
            tool_calls,
            response_lines,
        }
    }
}

#[derive(Debug, Clone)]
struct RecordedRequest {
    path: String,
    body: Value,
}

struct MockHttp {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockHttp {
    fn spawn(response_lines: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("server addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured_requests = Arc::clone(&requests);

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set read timeout");

            let request = read_http_request(&mut stream);
            captured_requests
                .lock()
                .expect("requests lock")
                .push(request);

            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
            )
            .expect("write response headers");
            stream.flush().expect("flush response headers");

            for line in response_lines {
                if line.contains("\"tool_calls\"") {
                    let split_at = line.len() / 2;
                    stream
                        .write_all(line[..split_at].as_bytes())
                        .expect("write partial response line");
                    stream.flush().expect("flush partial response line");
                    stream
                        .write_all(line[split_at..].as_bytes())
                        .expect("write trailing response line");
                } else {
                    stream
                        .write_all(line.as_bytes())
                        .expect("write response line");
                }
                stream.flush().expect("flush response line");
                if line.contains("\"reasoning_content\"") {
                    stream
                        .write_all(b"data: {not valid json}\n\n")
                        .expect("write malformed frame");
                    stream.flush().expect("flush malformed frame");
                }
            }
        });

        Self {
            base_url: format!("http://{addr}"),
            requests,
            handle: Some(handle),
        }
    }

    fn base_url(&self) -> String {
        self.base_url.clone()
    }

    fn finish(mut self) -> Vec<RecordedRequest> {
        if let Some(handle) = self.handle.take() {
            handle.join().expect("join mock http thread");
        }
        self.requests.lock().expect("requests lock").clone()
    }
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("cursor")
        .join(name)
}

fn sse_line(value: Value) -> String {
    format!("data: {value}\n\n")
}

fn read_http_request(stream: &mut TcpStream) -> RecordedRequest {
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
    let headers = String::from_utf8_lossy(&buf[..header_end]);
    let path = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("request path")
        .to_string();

    RecordedRequest {
        path,
        body: serde_json::from_slice(body).expect("request body json"),
    }
}

async fn collect_chunks(mut rx: mpsc::Receiver<StreamChunk>) -> Vec<StreamChunk> {
    let mut chunks = Vec::new();
    while let Some(chunk) = rx.recv().await {
        chunks.push(chunk);
    }
    chunks
}

fn collect_reasoning(chunks: &[StreamChunk]) -> String {
    chunks
        .iter()
        .filter_map(|chunk| match chunk {
            StreamChunk::ReasoningDelta(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn collect_content(chunks: &[StreamChunk]) -> String {
    chunks
        .iter()
        .filter_map(|chunk| match chunk {
            StreamChunk::ContentDelta(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn cursor_streaming_basic_replays_cleanly() {
    let fixture = Fixture::load("streaming-basic.jsonl");
    let mock_http = MockHttp::spawn(fixture.response_lines.clone());
    let backend = CursorAgent::new("test-key", "cursor-composer", SafetyLayer::with_defaults())
        .with_base_url(mock_http.base_url())
        .with_timeout_ms(5_000);
    let translator = OpenAiTranslator;
    let rendered_tools = translator.render_tools(&[roko_core::tool::ToolDef::new(
        "read_file",
        "Read a file",
        roko_core::tool::ToolCategory::Read,
        roko_core::tool::ToolPermission::read_only(),
    )]);
    let messages = vec![
        json!({"role": "system", "content": "You are Cursor."}),
        json!({"role": "user", "content": "stream the basic answer"}),
    ];
    let session = SessionState::default();
    let (event_tx, event_rx) = mpsc::channel(roko_core::defaults::DEFAULT_CHANNEL_BUFFER);

    let response = backend
        .send_turn_streaming(&messages, &rendered_tools, &session, event_tx)
        .await
        .expect("cursor streaming replay");
    let chunks = collect_chunks(event_rx).await;
    let requests = mock_http.finish();

    assert_eq!(collect_reasoning(&chunks), fixture.reasoning);
    assert_eq!(collect_content(&chunks), fixture.content);
    assert!(
        chunks
            .iter()
            .any(|chunk| matches!(chunk, StreamChunk::ToolCallDelta { .. }))
    );
    assert!(chunks.iter().any(|chunk| matches!(
        chunk,
        StreamChunk::Done(roko_agent::translate::FinishReason::ToolCalls)
    )));

    assert_eq!(response.extract_text(), fixture.content);
    assert_eq!(
        response.extract_reasoning().as_deref(),
        Some(fixture.reasoning.as_str())
    );
    assert_eq!(
        response.extract_usage().input_tokens,
        fixture.usage.input_tokens
    );
    assert_eq!(
        response.extract_usage().output_tokens,
        fixture.usage.output_tokens
    );
    assert_eq!(
        response.extract_usage().cache_read_tokens,
        fixture.usage.cache_read_tokens
    );

    let turn_session = backend.extract_session(&response);
    assert_eq!(
        turn_session.conversation_id.as_deref(),
        Some(fixture.response_id.as_str())
    );
    assert_eq!(
        turn_session.session_id.as_deref(),
        Some(fixture.session_id.as_str())
    );
    assert_eq!(
        turn_session.thread_id.as_deref(),
        Some(fixture.thread_id.as_str())
    );

    let mut tool_calls = translator
        .parse_calls(&response)
        .expect("parse streamed cursor tool calls");
    tool_calls.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(tool_calls, expected_tool_calls(&fixture.tool_calls));

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/v1/chat/completions");
    assert_eq!(requests[0].body["model"], "cursor-composer");
    assert_eq!(requests[0].body["stream"], json!(true));
    assert_eq!(
        requests[0].body["messages"][1]["content"],
        "stream the basic answer"
    );
    assert_eq!(
        requests[0].body["tools"][0]["function"]["name"],
        "read_file"
    );
}

fn expected_tool_calls(expected: &[ExpectedToolCall]) -> Vec<ToolCall> {
    expected
        .iter()
        .map(|tool_call| {
            ToolCall::new(
                tool_call.id.clone(),
                tool_call.name.clone(),
                serde_json::from_str(&tool_call.arguments_json)
                    .expect("expected tool arguments should parse"),
            )
        })
        .collect()
}
