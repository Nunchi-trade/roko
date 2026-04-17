//! Codex streaming replay conformance coverage.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use roko_agent::OpenAiCompatLlmBackend;
use roko_agent::Usage;
use roko_agent::streaming::StreamChunk;
use roko_agent::tool_loop::LlmBackend;
use roko_agent::translate::{
    BackendResponse, OpenAiTranslator, RenderedResults, SessionState, Translator,
};
use roko_core::tool::{ToolCall, ToolCategory, ToolDef, ToolPermission, ToolResult};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum FixtureFrame {
    Draft {
        turn: usize,
        text: String,
    },
    ReasoningDelta {
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
    ToolResult {
        turn: usize,
        tool_call_id: String,
        content: String,
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
            Self::Draft { turn, .. }
            | Self::ReasoningDelta { turn, .. }
            | Self::ToolCall { turn, .. }
            | Self::ToolResult { turn, .. }
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
    fn as_usage(&self) -> Usage {
        Usage {
            input_tokens: self.prompt_tokens,
            output_tokens: self.completion_tokens,
            cache_read_tokens: self.cached_tokens,
            ..Default::default()
        }
    }

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpectedToolCall {
    id: String,
    name: String,
    arguments_json: String,
}

#[derive(Debug, Clone)]
struct ToolResultFrame {
    tool_call_id: String,
    content: String,
}

#[derive(Debug, Clone)]
struct TurnFixture {
    turn: usize,
    finish_reason: String,
    response_id: String,
    session_id: String,
    thread_id: String,
    reasoning: String,
    content: String,
    usage: Usage,
    tool_calls: Vec<ExpectedToolCall>,
    tool_results: Vec<ToolResultFrame>,
    tool_call_frame_count: usize,
    response_lines: Vec<String>,
}

#[derive(Debug, Clone)]
struct Fixture {
    turns: Vec<TurnFixture>,
    expected_content: String,
    expected_reasoning: String,
    expected_tool_calls: Vec<ExpectedToolCall>,
    expected_session: SessionState,
    expected_usage: Usage,
}

impl Fixture {
    fn load(name: &str) -> Self {
        let path = fixture_path(name);
        let raw = fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!("read fixture {}: {err}", path.display());
        });
        let mut grouped = BTreeMap::<usize, Vec<FixtureFrame>>::new();
        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            let frame: FixtureFrame = serde_json::from_str(line)
                .unwrap_or_else(|err| panic!("parse fixture line: {err}"));
            grouped.entry(frame.turn()).or_default().push(frame);
        }

        let mut turns = Vec::with_capacity(grouped.len());
        let mut expected_reasoning = String::new();
        let mut expected_tool_calls = Vec::new();
        let mut expected_usage = Usage::default();
        let mut expected_content = String::new();
        let mut expected_session = SessionState::default();

        for (turn, frames) in grouped {
            let turn_fixture = TurnFixture::from_frames(turn, &frames);
            expected_reasoning.push_str(&turn_fixture.reasoning);
            expected_tool_calls.extend(turn_fixture.tool_calls.clone());
            add_usage(&mut expected_usage, turn_fixture.usage);
            if turn_fixture.finish_reason == "stop" {
                expected_content = turn_fixture.content.clone();
            }
            expected_session = SessionState {
                session_id: Some(turn_fixture.session_id.clone()),
                thread_id: Some(turn_fixture.thread_id.clone()),
                conversation_id: Some(turn_fixture.response_id.clone()),
            };
            turns.push(turn_fixture);
        }

        Self {
            turns,
            expected_content,
            expected_reasoning,
            expected_tool_calls,
            expected_session,
            expected_usage,
        }
    }
}

impl TurnFixture {
    fn from_frames(turn: usize, frames: &[FixtureFrame]) -> Self {
        let mut reasoning = String::new();
        let mut content = String::new();
        let mut tool_results = Vec::new();
        let mut partial_calls = BTreeMap::<usize, PartialToolCall>::new();
        let mut tool_call_frame_count = 0;
        let mut response_lines = Vec::new();
        let mut final_frame = None;

        for frame in frames {
            match frame {
                FixtureFrame::Draft { text, .. } => {
                    content.push_str(text);
                    response_lines.push(sse_line(json!({
                        "choices": [{
                            "delta": {
                                "content": text,
                            },
                        }],
                    })));
                }
                FixtureFrame::ReasoningDelta { text, .. } => {
                    reasoning.push_str(text);
                    response_lines.push(sse_line(json!({
                        "choices": [{
                            "delta": {
                                "reasoning_content": text,
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
                    tool_call_frame_count += 1;
                    let entry = partial_calls.entry(*index).or_default();
                    if let Some(id) = id {
                        entry.id = id.clone();
                    }
                    if let Some(name) = name {
                        entry.name = name.clone();
                    }
                    entry.arguments_json.push_str(arguments_delta);

                    let mut tool_call = Map::new();
                    tool_call.insert("index".to_string(), json!(index));
                    if let Some(id) = id {
                        tool_call.insert("id".to_string(), Value::String(id.clone()));
                    }
                    let mut function = Map::new();
                    if let Some(name) = name {
                        function.insert("name".to_string(), Value::String(name.clone()));
                    }
                    if !arguments_delta.is_empty() {
                        function.insert(
                            "arguments".to_string(),
                            Value::String(arguments_delta.clone()),
                        );
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
                FixtureFrame::ToolResult {
                    tool_call_id,
                    content,
                    ..
                } => tool_results.push(ToolResultFrame {
                    tool_call_id: tool_call_id.clone(),
                    content: content.clone(),
                }),
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
                    final_frame = Some((
                        finish_reason.clone(),
                        response_id.clone(),
                        session_id.clone(),
                        thread_id.clone(),
                        usage.as_usage(),
                    ));
                }
            }
        }

        response_lines.push("data: [DONE]\n\n".to_string());

        let (finish_reason, response_id, session_id, thread_id, usage) =
            final_frame.unwrap_or_else(|| panic!("turn {turn} missing final frame"));
        let mut tool_calls: Vec<ExpectedToolCall> = partial_calls
            .into_values()
            .map(|call| ExpectedToolCall {
                id: call.id,
                name: call.name,
                arguments_json: call.arguments_json,
            })
            .collect();
        tool_calls.sort_by(|a, b| a.id.cmp(&b.id));

        Self {
            turn,
            finish_reason,
            response_id,
            session_id,
            thread_id,
            reasoning,
            content,
            usage,
            tool_calls,
            tool_results,
            tool_call_frame_count,
            response_lines,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments_json: String,
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
    fn spawn(turns: Vec<TurnFixture>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("server addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured_requests = Arc::clone(&requests);

        let handle = thread::spawn(move || {
            for turn in turns {
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

                for line in &turn.response_lines {
                    stream
                        .write_all(line.as_bytes())
                        .expect("write response line");
                    stream.flush().expect("flush response line");
                }
            }
        });

        Self {
            base_url: format!("http://{addr}/v1"),
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

#[tokio::test]
async fn codex_ten_turn_basic_replays_cleanly() {
    let fixture = Fixture::load("ten-turn-basic.jsonl");
    let mock_http = MockHttp::spawn(fixture.turns.clone());
    let backend = OpenAiCompatLlmBackend::new("test-key", "gpt-5-codex")
        .with_base_url(mock_http.base_url())
        .with_timeout_ms(5_000);
    let translator = OpenAiTranslator;
    let rendered_tools = translator.render_tools(&[echo_tool()]);
    let mut messages = vec![
        json!({"role": "system", "content": "You are Codex."}),
        json!({"role": "user", "content": "please help"}),
    ];
    let mut session = SessionState::default();
    let mut expected_tool_call_iter = fixture.expected_tool_calls.iter();
    let mut observed_reasoning = String::new();
    let mut observed_usage = Usage::default();
    let mut observed_tool_calls = Vec::new();
    let mut final_text = String::new();

    for turn in &fixture.turns {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let response = backend
            .send_turn_streaming(&messages, &rendered_tools, &session, event_tx)
            .await
            .unwrap_or_else(|err| panic!("turn {} replay failed: {err}", turn.turn));
        let chunks = collect_chunks(&mut event_rx).await;

        let chunk_reasoning = collect_reasoning(&chunks);
        let chunk_content = collect_content(&chunks);
        let chunk_tool_delta_count = chunks
            .iter()
            .filter(|chunk| matches!(chunk, StreamChunk::ToolCallDelta { .. }))
            .count();
        assert_eq!(
            chunk_reasoning, turn.reasoning,
            "turn {} reasoning",
            turn.turn
        );
        assert_eq!(chunk_content, turn.content, "turn {} content", turn.turn);
        assert_eq!(
            chunk_tool_delta_count, turn.tool_call_frame_count,
            "turn {} tool-call frame count",
            turn.turn
        );

        observed_reasoning.push_str(&chunk_reasoning);
        add_usage(&mut observed_usage, response.extract_usage());
        final_text = response.extract_text();

        let turn_session = backend.extract_session(&response);
        assert_eq!(
            turn_session.conversation_id.as_deref(),
            Some(turn.response_id.as_str()),
            "turn {} conversation id",
            turn.turn
        );
        assert_eq!(
            turn_session.session_id.as_deref(),
            Some(turn.session_id.as_str()),
            "turn {} session id",
            turn.turn
        );
        assert_eq!(
            turn_session.thread_id.as_deref(),
            Some(turn.thread_id.as_str()),
            "turn {} thread id",
            turn.turn
        );
        merge_session_state(&mut session, turn_session);

        assert_eq!(
            finish_reason(&response),
            Some(turn.finish_reason.as_str()),
            "turn {} finish reason",
            turn.turn
        );

        let mut turn_tool_calls = translator
            .parse_calls(&response)
            .unwrap_or_else(|err| panic!("turn {} tool-call parse failed: {err}", turn.turn));
        turn_tool_calls.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(
            turn_tool_calls.len(),
            turn.tool_calls.len(),
            "turn {} tool-call count",
            turn.turn
        );
        for expected in &turn.tool_calls {
            let actual = turn_tool_calls
                .iter()
                .find(|call| call.id == expected.id)
                .unwrap_or_else(|| panic!("turn {} missing tool call {}", turn.turn, expected.id));
            assert_tool_call_eq(actual, expected);
            let next_expected = expected_tool_call_iter
                .next()
                .unwrap_or_else(|| panic!("missing global expected tool call {}", expected.id));
            assert_eq!(next_expected, expected);
        }
        observed_tool_calls.extend(turn_tool_calls.clone());

        let assistant_message = translator
            .render_assistant_message(&response)
            .unwrap_or_else(|| panic!("turn {} missing assistant message", turn.turn));
        messages.push(assistant_message);

        if !turn.tool_results.is_empty() {
            let rendered_results =
                render_tool_results(&translator, &turn_tool_calls, &turn.tool_results);
            messages.extend(rendered_results);
        }
    }

    assert!(
        expected_tool_call_iter.next().is_none(),
        "unused expected tool calls remained"
    );

    let requests = mock_http.finish();

    assert_eq!(final_text, fixture.expected_content);
    assert_eq!(observed_reasoning, fixture.expected_reasoning);
    assert_eq!(
        observed_usage.input_tokens,
        fixture.expected_usage.input_tokens
    );
    assert_eq!(
        observed_usage.output_tokens,
        fixture.expected_usage.output_tokens
    );
    assert_eq!(
        observed_usage.cache_read_tokens,
        fixture.expected_usage.cache_read_tokens
    );
    assert_eq!(observed_tool_calls.len(), fixture.expected_tool_calls.len());
    assert_eq!(
        session.conversation_id.as_deref(),
        fixture.expected_session.conversation_id.as_deref()
    );
    assert_eq!(
        session.session_id.as_deref(),
        fixture.expected_session.session_id.as_deref()
    );
    assert_eq!(
        session.thread_id.as_deref(),
        fixture.expected_session.thread_id.as_deref()
    );

    assert_request_replay(&requests, &fixture);
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("codex")
        .join("sessions")
        .join(name)
}

fn sse_line(value: Value) -> String {
    format!("data: {}\n\n", value)
}

fn echo_tool() -> ToolDef {
    ToolDef::new(
        "echo",
        "Echo structured state",
        ToolCategory::Read,
        ToolPermission::read_only(),
    )
}

async fn collect_chunks(event_rx: &mut mpsc::UnboundedReceiver<StreamChunk>) -> Vec<StreamChunk> {
    let mut chunks = Vec::new();
    while let Some(chunk) = event_rx.recv().await {
        chunks.push(chunk);
    }
    chunks
}

fn collect_reasoning(chunks: &[StreamChunk]) -> String {
    let mut reasoning = String::new();
    for chunk in chunks {
        if let StreamChunk::ReasoningDelta(delta) = chunk {
            reasoning.push_str(delta);
        }
    }
    reasoning
}

fn collect_content(chunks: &[StreamChunk]) -> String {
    let mut content = String::new();
    for chunk in chunks {
        if let StreamChunk::ContentDelta(delta) = chunk {
            content.push_str(delta);
        }
    }
    content
}

fn finish_reason(response: &BackendResponse) -> Option<&str> {
    match response {
        BackendResponse::Json(json) => json
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str),
        BackendResponse::StreamJson(_) | BackendResponse::Text(_) => None,
    }
}

fn assert_tool_call_eq(actual: &ToolCall, expected: &ExpectedToolCall) {
    assert_eq!(actual.id, expected.id);
    assert_eq!(actual.name, expected.name);
    let expected_arguments: Value = serde_json::from_str(&expected.arguments_json)
        .unwrap_or_else(|err| panic!("parse expected tool arguments: {err}"));
    assert_eq!(actual.arguments, expected_arguments);
}

fn render_tool_results(
    translator: &OpenAiTranslator,
    tool_calls: &[ToolCall],
    tool_results: &[ToolResultFrame],
) -> Vec<Value> {
    let rendered = translator.render_results(
        &tool_results
            .iter()
            .map(|frame| {
                let call = tool_calls
                    .iter()
                    .find(|call| call.id == frame.tool_call_id)
                    .unwrap_or_else(|| panic!("missing tool call {}", frame.tool_call_id))
                    .clone();
                (call, ToolResult::text(frame.content.clone()))
            })
            .collect::<Vec<_>>(),
    );
    let RenderedResults::JsonMessages(messages) = rendered else {
        panic!("expected JsonMessages");
    };
    messages
        .as_array()
        .unwrap_or_else(|| panic!("tool result messages should be an array"))
        .to_vec()
}

fn merge_session_state(current: &mut SessionState, next: SessionState) {
    if next.session_id.is_some() {
        current.session_id = next.session_id;
    }
    if next.thread_id.is_some() {
        current.thread_id = next.thread_id;
    }
    if next.conversation_id.is_some() {
        current.conversation_id = next.conversation_id;
    }
}

fn add_usage(total: &mut Usage, usage: Usage) {
    total.input_tokens = total.input_tokens.saturating_add(usage.input_tokens);
    total.output_tokens = total.output_tokens.saturating_add(usage.output_tokens);
    total.cache_read_tokens = total
        .cache_read_tokens
        .saturating_add(usage.cache_read_tokens);
    total.cache_create_tokens = total
        .cache_create_tokens
        .saturating_add(usage.cache_create_tokens);
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
    let headers = String::from_utf8_lossy(&buf[..header_end]);
    let path = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("request path")
        .to_string();
    let body = serde_json::from_slice(&buf[header_end..header_end + content_length])
        .expect("request body json");

    RecordedRequest { path, body }
}

fn assert_request_replay(requests: &[RecordedRequest], fixture: &Fixture) {
    assert_eq!(requests.len(), fixture.turns.len(), "request count");

    for (index, request) in requests.iter().enumerate() {
        assert_eq!(request.path, "/v1/chat/completions");
        assert_eq!(request.body["model"], "gpt-5-codex");
        assert_eq!(request.body["stream"], true);
        assert_eq!(request.body["tools"][0]["function"]["name"], "echo");

        if index == 0 {
            continue;
        }

        let previous_turn = &fixture.turns[index - 1];
        let messages = request.body["messages"]
            .as_array()
            .unwrap_or_else(|| panic!("request {index} messages should be an array"));

        assert!(messages.iter().any(|message| {
            message["role"] == "assistant"
                && message["reasoning_content"] == previous_turn.reasoning
        }));

        for expected_tool_call in &previous_turn.tool_calls {
            assert!(messages.iter().any(|message| {
                message["role"] == "assistant"
                    && message["tool_calls"].as_array().is_some_and(|calls| {
                        calls.iter().any(|call| {
                            let actual_arguments =
                                call["function"]["arguments"]
                                    .as_str()
                                    .and_then(|arguments| {
                                        serde_json::from_str::<Value>(arguments).ok()
                                    });
                            let expected_arguments =
                                serde_json::from_str::<Value>(&expected_tool_call.arguments_json)
                                    .ok();
                            call["id"] == expected_tool_call.id
                                && call["function"]["name"] == expected_tool_call.name
                                && actual_arguments == expected_arguments
                        })
                    })
            }));
        }

        for tool_result in &previous_turn.tool_results {
            assert!(messages.iter().any(|message| {
                message["role"] == "tool"
                    && message["tool_call_id"] == tool_result.tool_call_id
                    && message["content"] == tool_result.content
            }));
        }
    }
}
