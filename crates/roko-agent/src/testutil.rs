//! Shared parity-test harness helpers for backend integration coverage.

use crate::Agent;
use crate::cursor_agent::CursorAgent;
use crate::dispatcher::{HandlerResolver, ToolDispatcher};
use crate::exec::ExecAgent;
use crate::http::{HttpPostError, HttpPoster};
use crate::openai_compat_backend::OpenAiCompatLlmBackend;
use crate::safety::SafetyLayer;
use crate::streaming::StreamChunk;
use crate::streaming::parse_sse_line;
use crate::tool_loop::{LlmBackend, StopReason, ToolLoop};
use crate::translate::{
    BackendResponse, FinishReason, OpenAiTranslator, RenderedTools, SessionState, Translator,
};
use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolHandler, ToolPermission,
    ToolRegistry, ToolResult, VecToolRegistry,
};
use roko_core::{Body, Context, Engram, Kind};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

/// Backends covered by the UX32 parity harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParityBackend {
    /// Codex parity is exercised through the OpenAI-compatible backend surface.
    Codex,
    /// Cursor parity uses `CursorAgent`'s `LlmBackend` path.
    Cursor,
    /// OpenAI parity uses the OpenAI-compatible backend surface.
    OpenAi,
    /// Exec parity uses the subprocess-backed `Agent` surface.
    Exec,
}

impl ParityBackend {
    fn fixture_dir(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Cursor => "cursor",
            Self::OpenAi => "openai",
            Self::Exec => "exec",
        }
    }

    fn model(self) -> &'static str {
        match self {
            Self::Codex => "gpt-5-codex",
            Self::Cursor => "cursor-composer",
            Self::OpenAi => "gpt-4o-mini",
            Self::Exec => "exec",
        }
    }
}

/// Metadata derived from one parity fixture directory.
#[derive(Debug, Clone)]
pub struct BackendScenario {
    /// Root directory containing the fixture files for one scenario.
    pub fixture_path: PathBuf,
    /// Expected final assistant-visible text.
    pub expected_content: String,
    /// Expected number of parsed tool calls.
    pub expected_tool_calls: usize,
    /// Expected provider-issued session identifier, when the backend supports it.
    pub expected_session: Option<String>,
}

/// Run the shared happy-path assertion for `backend`.
pub async fn run_happy_path(backend: ParityBackend) -> Result<(), String> {
    match backend {
        ParityBackend::Exec => run_exec_happy_path().await,
        _ => run_llm_happy_path(backend).await,
    }
}

/// Run the shared streaming assertion for `backend`.
pub async fn run_streaming(backend: ParityBackend) -> Result<(), String> {
    match backend {
        ParityBackend::Codex | ParityBackend::Cursor | ParityBackend::OpenAi => {
            run_llm_streaming(backend).await
        }
        ParityBackend::Exec => Err("ExecAgent does not implement streaming".to_string()),
    }
}

/// Run the shared tool-call assertion for `backend`.
pub async fn run_tool_call(backend: ParityBackend) -> Result<(), String> {
    match backend {
        ParityBackend::Codex | ParityBackend::Cursor | ParityBackend::OpenAi => {
            run_llm_tool_call(backend).await
        }
        ParityBackend::Exec => Err("ExecAgent does not implement a tool-calling loop".to_string()),
    }
}

/// Run the shared error-path assertion for `backend`.
pub async fn run_error_path(backend: ParityBackend) -> Result<(), String> {
    match backend {
        ParityBackend::Exec => run_exec_error_path().await,
        _ => run_llm_error_path(backend).await,
    }
}

/// Run the shared session-continuation assertion for `backend`.
pub async fn run_session_continuation(backend: ParityBackend) -> Result<(), String> {
    match backend {
        ParityBackend::Codex | ParityBackend::Cursor | ParityBackend::OpenAi => {
            run_llm_session_continuation(backend).await
        }
        ParityBackend::Exec => Err("ExecAgent has no session or conversation state".to_string()),
    }
}

#[derive(Debug, Clone)]
struct RecordedRequest {
    body: Value,
}

#[derive(Debug, Clone)]
struct RecordedPoster {
    state: Arc<PosterState>,
}

#[derive(Debug)]
struct PosterState {
    responses: Mutex<VecDeque<Result<String, HttpPostError>>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

impl RecordedPoster {
    fn new(responses: Vec<Result<String, HttpPostError>>) -> Self {
        Self {
            state: Arc::new(PosterState {
                responses: Mutex::new(responses.into_iter().collect()),
                requests: Mutex::new(Vec::new()),
            }),
        }
    }

    fn requests(&self) -> Result<Vec<RecordedRequest>, String> {
        self.state
            .requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| "recorded poster request lock poisoned".to_string())
    }
}

#[async_trait]
impl HttpPoster for RecordedPoster {
    async fn post_json(
        &self,
        _url: &str,
        _headers: &[(String, String)],
        body: &[u8],
        _timeout_ms: u64,
    ) -> Result<String, HttpPostError> {
        let body = serde_json::from_slice(body).map_err(|err| {
            HttpPostError::transport(format!("request body decode failed: {err}"))
        })?;
        self.state
            .requests
            .lock()
            .map_err(|_| HttpPostError::transport("recorded poster request lock poisoned"))?
            .push(RecordedRequest { body });

        self.state
            .responses
            .lock()
            .map_err(|_| HttpPostError::transport("recorded poster response lock poisoned"))?
            .pop_front()
            .ok_or_else(|| HttpPostError::transport("no mock response queued"))
            .and_then(std::convert::identity)
    }
}

#[derive(Debug)]
struct StreamServer {
    addr: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    handle: Option<thread::JoinHandle<Result<(), String>>>,
}

impl StreamServer {
    fn spawn(lines: Vec<String>) -> Result<Self, String> {
        let listener =
            TcpListener::bind("127.0.0.1:0").map_err(|err| format!("bind stream server: {err}"))?;
        let addr = listener
            .local_addr()
            .map_err(|err| format!("stream server local addr: {err}"))?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured_requests = Arc::clone(&requests);

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .map_err(|err| format!("accept request: {err}"))?;
            let request = read_http_request(&mut stream)?;
            captured_requests
                .lock()
                .map_err(|_| "stream server request lock poisoned".to_string())?
                .push(request);

            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
            )
            .map_err(|err| format!("write stream response headers: {err}"))?;
            stream
                .flush()
                .map_err(|err| format!("flush stream response headers: {err}"))?;

            for line in lines {
                stream
                    .write_all(line.as_bytes())
                    .map_err(|err| format!("write stream response line: {err}"))?;
                stream
                    .flush()
                    .map_err(|err| format!("flush stream response line: {err}"))?;
            }

            Ok(())
        });

        Ok(Self {
            addr: addr.to_string(),
            requests,
            handle: Some(handle),
        })
    }

    fn base_url(&self, backend: ParityBackend) -> String {
        match backend {
            ParityBackend::Codex | ParityBackend::OpenAi => format!("http://{}/v1", self.addr),
            ParityBackend::Cursor => format!("http://{}", self.addr),
            ParityBackend::Exec => format!("http://{}", self.addr),
        }
    }

    fn finish(mut self) -> Result<Vec<RecordedRequest>, String> {
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .map_err(|_| "join stream server thread".to_string())??;
        }
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| "stream server request lock poisoned".to_string())
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ErrorFixture {
    status: u16,
    body: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct ExecScenario {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

#[derive(Debug, Clone, Deserialize)]
struct ExecErrorScenario {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StreamFrame {
    ReasoningDelta {
        #[serde(default)]
        turn: usize,
        text: String,
    },
    Draft {
        #[serde(default)]
        turn: usize,
        text: String,
    },
    ToolCall {
        #[serde(default)]
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
        #[serde(default)]
        turn: usize,
        finish_reason: String,
        response_id: String,
        session_id: String,
        thread_id: String,
        usage: FixtureUsage,
    },
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FixtureUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    #[serde(default)]
    cached_tokens: u32,
}

impl FixtureUsage {
    fn as_usage(&self) -> crate::Usage {
        crate::Usage {
            input_tokens: self.prompt_tokens,
            output_tokens: self.completion_tokens,
            cache_read_tokens: self.cached_tokens,
            ..crate::Usage::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExpectedChunk {
    Reasoning(String),
    Content(String),
    ToolCall {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments: String,
    },
    Usage {
        input_tokens: u32,
        output_tokens: u32,
        cached_tokens: u32,
    },
}

#[derive(Debug, Clone)]
struct StreamScenario {
    scenario: BackendScenario,
    expected_chunks: Vec<ExpectedChunk>,
    response_lines: Vec<String>,
    expected_thread: String,
    expected_conversation: String,
}

struct EchoHandler;

#[async_trait]
impl ToolHandler for EchoHandler {
    fn name(&self) -> &str {
        "echo"
    }

    async fn execute(&self, call: ToolCall, _ctx: &ToolContext) -> ToolResult {
        ToolResult::text(call.arguments.to_string())
    }
}

async fn run_llm_happy_path(backend: ParityBackend) -> Result<(), String> {
    let response = load_json_response(backend, "happy/response.json")?;
    let scenario = BackendScenario {
        fixture_path: scenario_path(backend, "happy"),
        expected_content: BackendResponse::Json(response.clone()).extract_text(),
        expected_tool_calls: 0,
        expected_session: extract_session_id(&response),
    };
    let poster = RecordedPoster::new(vec![Ok(response.to_string())]);
    let messages = prompt_messages("happy path parity");
    let rendered_tools = empty_tools();
    let session = SessionState::default();

    let response = match backend {
        ParityBackend::Codex | ParityBackend::OpenAi => {
            let backend = openai_compat_backend(backend, poster.clone());
            backend
                .send_turn(&messages, &rendered_tools, &session)
                .await
        }
        ParityBackend::Cursor => {
            let backend = cursor_backend(poster.clone());
            backend
                .send_turn(&messages, &rendered_tools, &session)
                .await
        }
        ParityBackend::Exec => unreachable!("handled by exec happy path"),
    }
    .map_err(|err| format!("happy path request failed: {err}"))?;

    if response.extract_text() != scenario.expected_content {
        return Err(format!(
            "happy path content mismatch for {}",
            scenario.fixture_path.display()
        ));
    }

    let requests = poster.requests()?;
    let first_request = requests
        .first()
        .ok_or_else(|| "happy path request was not recorded".to_string())?;
    assert_request_model(first_request, backend.model())?;
    assert_request_prompt(first_request, "happy path parity")?;

    Ok(())
}

async fn run_exec_happy_path() -> Result<(), String> {
    let scenario: ExecScenario =
        load_json_file(scenario_path(ParityBackend::Exec, "happy/scenario.json"))?;
    let input = prompt_signal("");
    let result = ExecAgent::new(
        "sh",
        vec![
            "-c".to_string(),
            "printf '%s' \"$ROKO_EXEC_STDOUT\"".to_string(),
        ],
        SafetyLayer::with_defaults(),
    )
    .with_env_var("ROKO_EXEC_STDOUT", scenario.stdout.clone())
    .run(&input, &Context::now())
    .await;
    if !result.success {
        return Err("exec happy path unexpectedly failed".to_string());
    }
    let actual = result.output.body.as_text().unwrap_or_default();
    if actual != scenario.stdout {
        return Err(format!(
            "exec happy path content mismatch: expected {:?}, got {:?}",
            scenario.stdout, actual
        ));
    }
    if scenario.exit_code != 0 {
        return Err("exec happy fixture must use exit_code 0".to_string());
    }
    if !scenario.stderr.is_empty() {
        return Err("exec happy fixture should not expect stderr".to_string());
    }
    Ok(())
}

async fn run_llm_streaming(backend: ParityBackend) -> Result<(), String> {
    let scenario = load_stream_scenario(backend)?;
    let server = StreamServer::spawn(scenario.response_lines.clone())?;
    let messages = prompt_messages("streaming parity");
    let rendered_tools = empty_tools();
    let session = SessionState::default();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();

    let response = match backend {
        ParityBackend::Codex | ParityBackend::OpenAi => {
            let backend = openai_compat_backend_with_base_url(backend, server.base_url(backend));
            backend
                .send_turn_streaming(&messages, &rendered_tools, &session, event_tx)
                .await
        }
        ParityBackend::Cursor => {
            let backend = cursor_backend_with_base_url(server.base_url(backend));
            backend
                .send_turn_streaming(&messages, &rendered_tools, &session, event_tx)
                .await
        }
        ParityBackend::Exec => unreachable!("handled by exec ignore"),
    }
    .map_err(|err| format!("streaming request failed: {err}"))?;

    let mut chunks = Vec::new();
    while let Some(chunk) = event_rx.recv().await {
        chunks.push(chunk);
    }

    let observed = normalize_chunks(&chunks);
    if observed != scenario.expected_chunks {
        return Err(format!(
            "stream chunk mismatch for {}",
            scenario.scenario.fixture_path.display()
        ));
    }

    match chunks.last() {
        Some(StreamChunk::Done(FinishReason::Stop)) => {}
        other => {
            return Err(format!(
                "stream did not end with a clean stop chunk: {other:?}"
            ));
        }
    }

    if response.extract_text() != scenario.scenario.expected_content {
        return Err("streamed final content mismatch".to_string());
    }

    let usage = response.extract_usage();
    let expected_usage = usage_from_chunks(&scenario.expected_chunks);
    if usage.input_tokens != expected_usage.input_tokens
        || usage.output_tokens != expected_usage.output_tokens
        || usage.cache_read_tokens != expected_usage.cache_read_tokens
    {
        return Err("streamed usage mismatch".to_string());
    }

    let session = extract_backend_session(&backend, &response);
    if session.session_id.as_deref() != scenario.scenario.expected_session.as_deref()
        || session.thread_id.as_deref() != Some(scenario.expected_thread.as_str())
        || session.conversation_id.as_deref() != Some(scenario.expected_conversation.as_str())
    {
        return Err("streamed session metadata mismatch".to_string());
    }

    let requests = server.finish()?;
    let first_request = requests
        .first()
        .ok_or_else(|| "streaming request was not recorded".to_string())?;
    assert_request_model(first_request, backend.model())?;
    assert_stream_flag(first_request)?;

    Ok(())
}

async fn run_llm_tool_call(backend: ParityBackend) -> Result<(), String> {
    let first = load_json_response(backend, "tool-call/turn-01.response.json")?;
    let second = load_json_response(backend, "tool-call/turn-02.response.json")?;
    let first_response = BackendResponse::Json(first.clone());
    let translator = OpenAiTranslator;
    let expected_calls = translator
        .parse_calls(&first_response)
        .map_err(|err| format!("parse tool-call fixture: {err}"))?;
    let scenario = BackendScenario {
        fixture_path: scenario_path(backend, "tool-call"),
        expected_content: BackendResponse::Json(second.clone()).extract_text(),
        expected_tool_calls: expected_calls.len(),
        expected_session: extract_session_id(&second).or_else(|| extract_session_id(&first)),
    };
    let poster = RecordedPoster::new(vec![Ok(first.to_string()), Ok(second.to_string())]);

    let output = match backend {
        ParityBackend::Codex | ParityBackend::OpenAi => {
            make_tool_loop(openai_compat_backend(backend, poster.clone()))
        }
        ParityBackend::Cursor => make_tool_loop(cursor_backend(poster.clone())),
        ParityBackend::Exec => unreachable!("handled by exec ignore"),
    }
    .run(
        "You may use the echo tool.",
        "Use the tool before answering.",
        &[parity_tool()],
        &ToolContext::testing("/tmp"),
    )
    .await;

    if output.stop_reason != StopReason::Stop {
        return Err(format!(
            "tool loop did not stop cleanly: {:?}",
            output.stop_reason
        ));
    }
    if output.final_text != scenario.expected_content {
        return Err(format!(
            "tool-call final text mismatch for {}",
            scenario.fixture_path.display()
        ));
    }
    if output.tool_calls.len() != scenario.expected_tool_calls {
        return Err(format!(
            "tool-call count mismatch: expected {}, got {}",
            scenario.expected_tool_calls,
            output.tool_calls.len()
        ));
    }

    let requests = poster.requests()?;
    let second_request = requests
        .get(1)
        .ok_or_else(|| "tool-call continuation request was not recorded".to_string())?;
    assert_request_has_assistant_tool_call(second_request, &expected_calls)?;
    assert_request_has_tool_results(second_request, &expected_calls)?;

    Ok(())
}

async fn run_llm_error_path(backend: ParityBackend) -> Result<(), String> {
    let fixture: ErrorFixture = load_json_file(scenario_path(backend, "error/response.json"))?;
    let poster = RecordedPoster::new(vec![Err(HttpPostError::http(
        fixture.status,
        render_error_body(&fixture.body),
    ))]);
    let messages = prompt_messages("error parity");
    let rendered_tools = empty_tools();
    let session = SessionState::default();

    let error = match backend {
        ParityBackend::Codex | ParityBackend::OpenAi => {
            let backend = openai_compat_backend(backend, poster);
            backend
                .send_turn(&messages, &rendered_tools, &session)
                .await
        }
        ParityBackend::Cursor => {
            let backend = cursor_backend(poster);
            backend
                .send_turn(&messages, &rendered_tools, &session)
                .await
        }
        ParityBackend::Exec => unreachable!("handled by exec error path"),
    }
    .err()
    .ok_or_else(|| "error path unexpectedly succeeded".to_string())?;

    let rendered = error.to_string();
    if !rendered.contains(&fixture.status.to_string()) {
        return Err(format!(
            "error path did not preserve HTTP status: {rendered}"
        ));
    }

    Ok(())
}

async fn run_exec_error_path() -> Result<(), String> {
    let scenario: ExecErrorScenario =
        load_json_file(scenario_path(ParityBackend::Exec, "error/scenario.json"))?;
    let input = prompt_signal("");
    let result = ExecAgent::new(
        "sh",
        vec![
            "-c".to_string(),
            format!(
                "printf '%s' \"$ROKO_EXEC_STDERR\" 1>&2; exit {}",
                scenario.exit_code
            ),
        ],
        SafetyLayer::with_defaults(),
    )
    .with_env_var("ROKO_EXEC_STDERR", scenario.stderr.clone())
    .run(&input, &Context::now())
    .await;
    if result.success {
        return Err("exec error path unexpectedly succeeded".to_string());
    }
    let actual = result.output.body.as_text().unwrap_or_default();
    if !actual.contains(scenario.stderr.trim()) {
        return Err(format!(
            "exec error output missing expected stderr {:?}: {:?}",
            scenario.stderr, actual
        ));
    }
    if !scenario.stdout.is_empty() {
        return Err("exec error fixture should not expect stdout".to_string());
    }
    Ok(())
}

async fn run_llm_session_continuation(backend: ParityBackend) -> Result<(), String> {
    let first = load_json_response(backend, "session-1/response.json")?;
    let second = load_json_response(backend, "session-2/response.json")?;
    let poster = RecordedPoster::new(vec![Ok(first.to_string()), Ok(second.to_string())]);
    let rendered_tools = empty_tools();
    let mut session = SessionState::default();
    let first_messages = prompt_messages("session one");

    let first_response = match backend {
        ParityBackend::Codex | ParityBackend::OpenAi => {
            let backend = openai_compat_backend(backend, poster.clone());
            backend
                .send_turn(&first_messages, &rendered_tools, &session)
                .await
        }
        ParityBackend::Cursor => {
            let backend = cursor_backend(poster.clone());
            backend
                .send_turn(&first_messages, &rendered_tools, &session)
                .await
        }
        ParityBackend::Exec => unreachable!("handled by exec ignore"),
    }
    .map_err(|err| format!("first session turn failed: {err}"))?;

    merge_session_state(
        &mut session,
        extract_backend_session(&backend, &first_response),
    );

    let expected_first_session = session
        .session_id
        .clone()
        .ok_or_else(|| "first session turn did not yield a session_id".to_string())?;
    let expected_first_thread = session
        .thread_id
        .clone()
        .ok_or_else(|| "first session turn did not yield a thread_id".to_string())?;
    let expected_first_conversation = session
        .conversation_id
        .clone()
        .ok_or_else(|| "first session turn did not yield a conversation_id".to_string())?;
    let second_messages = prompt_messages("session two");

    let second_response = match backend {
        ParityBackend::Codex | ParityBackend::OpenAi => {
            let backend = openai_compat_backend(backend, poster.clone());
            backend
                .send_turn(&second_messages, &rendered_tools, &session)
                .await
        }
        ParityBackend::Cursor => {
            let backend = cursor_backend(poster.clone());
            backend
                .send_turn(&second_messages, &rendered_tools, &session)
                .await
        }
        ParityBackend::Exec => unreachable!("handled by exec ignore"),
    }
    .map_err(|err| format!("second session turn failed: {err}"))?;

    let expected_second = BackendResponse::Json(second.clone()).extract_text();
    if second_response.extract_text() != expected_second {
        return Err("second session turn content mismatch".to_string());
    }

    let requests = poster.requests()?;
    let second_request = requests
        .get(1)
        .ok_or_else(|| "second session request was not recorded".to_string())?;
    if second_request.body["session_id"] != expected_first_session {
        return Err("second request did not forward session_id".to_string());
    }
    if second_request.body["thread_id"] != expected_first_thread {
        return Err("second request did not forward thread_id".to_string());
    }
    if second_request.body["conversation_id"] != expected_first_conversation {
        return Err("second request did not forward conversation_id".to_string());
    }

    Ok(())
}

fn load_stream_scenario(backend: ParityBackend) -> Result<StreamScenario, String> {
    let path = scenario_path(backend, "streaming/frames.jsonl");
    let raw = fs::read_to_string(&path)
        .map_err(|err| format!("read stream fixture {}: {err}", path.display()))?;
    let mut expected_chunks = Vec::new();
    let mut response_lines = Vec::new();
    let mut content = String::new();
    let mut session_id = None;
    let mut thread_id = None;
    let mut conversation_id = None;

    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        if let Ok(frame) = serde_json::from_str::<StreamFrame>(line) {
            match frame {
                StreamFrame::ReasoningDelta { text, .. } => {
                    append_stream_value(
                        &json!({
                            "choices": [{
                                "delta": {
                                    "reasoning_content": text,
                                },
                            }],
                        }),
                        &mut expected_chunks,
                        &mut response_lines,
                        &mut content,
                        &mut conversation_id,
                        &mut session_id,
                        &mut thread_id,
                    );
                }
                StreamFrame::Draft { text, .. } => {
                    append_stream_value(
                        &json!({
                            "choices": [{
                                "delta": {
                                    "content": text,
                                },
                            }],
                        }),
                        &mut expected_chunks,
                        &mut response_lines,
                        &mut content,
                        &mut conversation_id,
                        &mut session_id,
                        &mut thread_id,
                    );
                }
                StreamFrame::ToolCall {
                    index,
                    id,
                    name,
                    arguments_delta,
                    ..
                } => {
                    let mut tool_call = serde_json::Map::new();
                    tool_call.insert("index".to_string(), Value::from(index));
                    if let Some(id) = id {
                        tool_call.insert("id".to_string(), Value::String(id));
                    }
                    let mut function = serde_json::Map::new();
                    if let Some(name) = name {
                        function.insert("name".to_string(), Value::String(name));
                    }
                    if !arguments_delta.is_empty() {
                        function.insert("arguments".to_string(), Value::String(arguments_delta));
                    }
                    if !function.is_empty() {
                        tool_call.insert("function".to_string(), Value::Object(function));
                    }
                    append_stream_value(
                        &json!({
                            "choices": [{
                                "delta": {
                                    "tool_calls": [Value::Object(tool_call)],
                                },
                            }],
                        }),
                        &mut expected_chunks,
                        &mut response_lines,
                        &mut content,
                        &mut conversation_id,
                        &mut session_id,
                        &mut thread_id,
                    );
                }
                StreamFrame::Final {
                    finish_reason,
                    response_id,
                    session_id: sid,
                    thread_id: tid,
                    usage,
                    ..
                } => {
                    append_stream_value(
                        &json!({
                            "id": response_id,
                            "session_id": sid,
                            "thread_id": tid,
                            "choices": [{
                                "delta": {},
                                "finish_reason": finish_reason,
                            }],
                        }),
                        &mut expected_chunks,
                        &mut response_lines,
                        &mut content,
                        &mut conversation_id,
                        &mut session_id,
                        &mut thread_id,
                    );
                    append_stream_value(
                        &json!({
                            "id": conversation_id.as_deref().unwrap_or_default(),
                            "session_id": session_id.as_deref().unwrap_or_default(),
                            "thread_id": thread_id.as_deref().unwrap_or_default(),
                            "usage": {
                                "prompt_tokens": usage.prompt_tokens,
                                "completion_tokens": usage.completion_tokens,
                                "total_tokens": usage.prompt_tokens + usage.completion_tokens,
                                "prompt_tokens_details": {
                                    "cached_tokens": usage.cached_tokens,
                                },
                            },
                        }),
                        &mut expected_chunks,
                        &mut response_lines,
                        &mut content,
                        &mut conversation_id,
                        &mut session_id,
                        &mut thread_id,
                    );
                }
            }
            continue;
        }

        let value: Value =
            serde_json::from_str(line).map_err(|err| format!("parse stream frame: {err}"))?;
        append_stream_value(
            &value,
            &mut expected_chunks,
            &mut response_lines,
            &mut content,
            &mut conversation_id,
            &mut session_id,
            &mut thread_id,
        );
    }

    response_lines.push("data: [DONE]\n\n".to_string());

    let scenario = BackendScenario {
        fixture_path: scenario_path(backend, "streaming"),
        expected_content: content,
        expected_tool_calls: expected_chunks
            .iter()
            .filter(|chunk| matches!(chunk, ExpectedChunk::ToolCall { .. }))
            .count(),
        expected_session: session_id.clone(),
    };

    Ok(StreamScenario {
        scenario,
        expected_chunks,
        response_lines,
        expected_thread: thread_id.ok_or_else(|| "stream fixture missing thread_id".to_string())?,
        expected_conversation: conversation_id
            .ok_or_else(|| "stream fixture missing response_id".to_string())?,
    })
}

fn append_stream_value(
    value: &Value,
    expected_chunks: &mut Vec<ExpectedChunk>,
    response_lines: &mut Vec<String>,
    content: &mut String,
    conversation_id: &mut Option<String>,
    session_id: &mut Option<String>,
    thread_id: &mut Option<String>,
) {
    if let Some(id) = value.get("id").and_then(Value::as_str) {
        *conversation_id = Some(id.to_string());
    }
    if let Some(id) = value.get("session_id").and_then(Value::as_str) {
        *session_id = Some(id.to_string());
    }
    if let Some(id) = value.get("thread_id").and_then(Value::as_str) {
        *thread_id = Some(id.to_string());
    }

    let line = sse_line(value.clone());
    if let Some(chunk) = parse_sse_line(line.trim_end()) {
        match chunk {
            StreamChunk::ReasoningDelta(text) => {
                expected_chunks.push(ExpectedChunk::Reasoning(text));
            }
            StreamChunk::ContentDelta(text) => {
                content.push_str(&text);
                expected_chunks.push(ExpectedChunk::Content(text));
            }
            StreamChunk::ToolCallDelta {
                index,
                id_delta,
                name_delta,
                arguments_delta,
            } => {
                expected_chunks.push(ExpectedChunk::ToolCall {
                    index,
                    id: id_delta,
                    name: name_delta,
                    arguments: arguments_delta,
                });
            }
            StreamChunk::Usage(usage) => {
                expected_chunks.push(ExpectedChunk::Usage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cached_tokens: usage.cache_read_tokens,
                });
            }
            StreamChunk::Done(_) | StreamChunk::Error(_) => {}
        }
    }

    response_lines.push(line);
}

fn normalize_chunks(chunks: &[StreamChunk]) -> Vec<ExpectedChunk> {
    let mut out = Vec::new();
    for chunk in chunks {
        match chunk {
            StreamChunk::ReasoningDelta(text) => out.push(ExpectedChunk::Reasoning(text.clone())),
            StreamChunk::ContentDelta(text) => out.push(ExpectedChunk::Content(text.clone())),
            StreamChunk::ToolCallDelta {
                index,
                id_delta,
                name_delta,
                arguments_delta,
            } => out.push(ExpectedChunk::ToolCall {
                index: *index,
                id: id_delta.clone(),
                name: name_delta.clone(),
                arguments: arguments_delta.clone(),
            }),
            StreamChunk::Usage(usage) => out.push(ExpectedChunk::Usage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_tokens: usage.cache_read_tokens,
            }),
            StreamChunk::Done(_) | StreamChunk::Error(_) => {}
        }
    }
    out
}

fn usage_from_chunks(chunks: &[ExpectedChunk]) -> crate::Usage {
    let mut usage = crate::Usage::default();
    for chunk in chunks {
        if let ExpectedChunk::Usage {
            input_tokens,
            output_tokens,
            cached_tokens,
        } = chunk
        {
            usage = crate::Usage {
                input_tokens: *input_tokens,
                output_tokens: *output_tokens,
                cache_read_tokens: *cached_tokens,
                ..crate::Usage::default()
            };
        }
    }
    usage
}

fn make_tool_loop<B>(backend: B) -> ToolLoop
where
    B: LlmBackend + 'static,
{
    let registry: Arc<dyn ToolRegistry> =
        Arc::new(VecToolRegistry::from_tools(vec![parity_tool()]));
    let resolver: Arc<dyn HandlerResolver> = Arc::new(|name: &str| {
        (name == "echo").then(|| Arc::new(EchoHandler) as Arc<dyn ToolHandler>)
    });
    let dispatcher = Arc::new(ToolDispatcher::new(registry, resolver));
    let translator: Arc<dyn Translator> = Arc::new(OpenAiTranslator);
    ToolLoop::new(translator, dispatcher, Arc::new(backend))
}

fn parity_tool() -> ToolDef {
    ToolDef::new(
        "echo",
        "Echo structured arguments back to the model.",
        ToolCategory::Meta,
        ToolPermission::read_only(),
    )
    .with_concurrency(ToolConcurrency::Parallel)
}

fn openai_compat_backend(backend: ParityBackend, poster: RecordedPoster) -> OpenAiCompatLlmBackend {
    OpenAiCompatLlmBackend::new("test-key", backend.model()).with_poster(Box::new(poster))
}

fn openai_compat_backend_with_base_url(
    backend: ParityBackend,
    base_url: String,
) -> OpenAiCompatLlmBackend {
    OpenAiCompatLlmBackend::new("test-key", backend.model())
        .with_base_url(base_url)
        .with_timeout_ms(5_000)
}

fn cursor_backend(poster: RecordedPoster) -> CursorAgent {
    CursorAgent::new(
        "test-key",
        ParityBackend::Cursor.model(),
        SafetyLayer::with_defaults(),
    )
    .with_http_poster(Arc::new(poster))
}

fn cursor_backend_with_base_url(base_url: String) -> CursorAgent {
    CursorAgent::new(
        "test-key",
        ParityBackend::Cursor.model(),
        SafetyLayer::with_defaults(),
    )
    .with_base_url(base_url)
    .with_timeout_ms(5_000)
}

fn extract_backend_session(backend: &ParityBackend, response: &BackendResponse) -> SessionState {
    match backend {
        ParityBackend::Codex | ParityBackend::OpenAi => {
            OpenAiCompatLlmBackend::new("test-key", backend.model()).extract_session(response)
        }
        ParityBackend::Cursor => {
            CursorAgent::new("test-key", backend.model(), SafetyLayer::with_defaults())
                .extract_session(response)
        }
        ParityBackend::Exec => SessionState::default(),
    }
}

fn extract_session_id(response: &Value) -> Option<String> {
    response
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn prompt_messages(text: &str) -> Vec<Value> {
    vec![json!({ "role": "user", "content": text })]
}

fn prompt_signal(text: &str) -> Engram {
    Engram::builder(Kind::Prompt).body(Body::text(text)).build()
}

fn empty_tools() -> RenderedTools {
    RenderedTools::JsonArray(Value::Array(Vec::new()))
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

fn assert_request_model(request: &RecordedRequest, expected_model: &str) -> Result<(), String> {
    if request.body["model"] != expected_model {
        return Err(format!(
            "request model mismatch: expected {expected_model:?}, got {:?}",
            request.body["model"]
        ));
    }
    Ok(())
}

fn assert_request_prompt(request: &RecordedRequest, expected_prompt: &str) -> Result<(), String> {
    let prompt = request
        .body
        .pointer("/messages/0/content")
        .and_then(Value::as_str)
        .ok_or_else(|| "request prompt content missing".to_string())?;
    if prompt != expected_prompt {
        return Err(format!(
            "request prompt mismatch: expected {expected_prompt:?}, got {prompt:?}"
        ));
    }
    Ok(())
}

fn assert_stream_flag(request: &RecordedRequest) -> Result<(), String> {
    if request.body["stream"] != Value::Bool(true) {
        return Err("streaming request did not set stream=true".to_string());
    }
    Ok(())
}

fn assert_request_has_assistant_tool_call(
    request: &RecordedRequest,
    expected_calls: &[ToolCall],
) -> Result<(), String> {
    let messages = request
        .body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| "tool-call request missing messages array".to_string())?;
    for expected in expected_calls {
        let found = messages.iter().any(|message| {
            message["role"] == "assistant"
                && message["tool_calls"].as_array().is_some_and(|calls| {
                    calls.iter().any(|call| {
                        let actual_arguments = call["function"]["arguments"]
                            .as_str()
                            .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok());
                        call["id"] == expected.id
                            && call["function"]["name"] == expected.name
                            && actual_arguments == Some(expected.arguments.clone())
                    })
                })
        });
        if !found {
            return Err(format!(
                "assistant continuation missing tool call {}",
                expected.id
            ));
        }
    }
    Ok(())
}

fn assert_request_has_tool_results(
    request: &RecordedRequest,
    expected_calls: &[ToolCall],
) -> Result<(), String> {
    let messages = request
        .body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| "tool-call request missing messages array".to_string())?;
    for expected in expected_calls {
        let expected_content = expected.arguments.to_string();
        let found = messages.iter().any(|message| {
            message["role"] == "tool"
                && message["tool_call_id"] == expected.id
                && message["content"] == expected_content
        });
        if !found {
            return Err(format!(
                "continuation missing tool result for {}",
                expected.id
            ));
        }
    }
    Ok(())
}

fn scenario_path(backend: ParityBackend, relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(backend.fixture_dir())
        .join(relative)
}

fn load_json_response(backend: ParityBackend, relative: &str) -> Result<Value, String> {
    load_json_file(scenario_path(backend, relative))
}

fn load_json_file<T>(path: PathBuf) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let raw = fs::read_to_string(&path)
        .map_err(|err| format!("read fixture {}: {err}", path.display()))?;
    serde_json::from_str(&raw).map_err(|err| format!("parse fixture {}: {err}", path.display()))
}

fn render_error_body(body: &Value) -> String {
    body.as_str()
        .map(str::to_string)
        .unwrap_or_else(|| body.to_string())
}

fn sse_line(value: Value) -> String {
    format!("data: {value}\n\n")
}

fn read_http_request(stream: &mut TcpStream) -> Result<RecordedRequest, String> {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|err| format!("read request bytes: {err}"))?;
        if read == 0 {
            return Err("request closed before headers completed".to_string());
        }
        buf.extend_from_slice(&chunk[..read]);

        if let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
    };

    let headers = String::from_utf8_lossy(&buf[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);

    while buf.len() < header_end + content_length {
        let read = stream
            .read(&mut chunk)
            .map_err(|err| format!("read request body: {err}"))?;
        if read == 0 {
            return Err("request closed before body completed".to_string());
        }
        buf.extend_from_slice(&chunk[..read]);
    }

    let body = serde_json::from_slice(&buf[header_end..header_end + content_length])
        .map_err(|err| format!("decode request body json: {err}"))?;
    Ok(RecordedRequest { body })
}
