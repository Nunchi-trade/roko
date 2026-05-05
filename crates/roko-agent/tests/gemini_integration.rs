#![allow(missing_docs)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use roko_agent::gemini::{GeminiAdapter, GeminiMetadata, GeminiNativeAgent};
use roko_agent::provider::{AgentOptions, ProviderAdapter, ProviderError, create_agent_for_model};
use roko_agent::translate::{BackendResponse, GeminiTranslator, RenderedResults, Translator};
use roko_agent::{Agent, SafetyLayer};
use roko_core::agent::ProviderKind;
use roko_core::config::DEFAULT_TTFT_TIMEOUT_MS;
use roko_core::config::schema::{ModelProfile, ProviderConfig, RokoConfig};
use roko_core::tool::ToolResult;
use roko_core::{Body, Context, Engram, Kind};
use roko_learn::costs_db::CostTable;
use serde_json::{Value, json};

fn prompt(text: &str) -> Engram {
    Engram::builder(Kind::Prompt).body(Body::text(text)).build()
}

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

#[derive(Debug, Clone)]
struct ScriptedResponse {
    status: u16,
    body: String,
}

#[derive(Debug)]
struct TestServer {
    base_url: String,
    captured: Arc<Mutex<Vec<RecordedRequest>>>,
    handle: thread::JoinHandle<()>,
}

impl TestServer {
    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.captured.lock().expect("capture lock").clone()
    }

    fn join(self) {
        self.handle.join().expect("server thread");
    }
}

fn response(status: u16, body: Value) -> ScriptedResponse {
    ScriptedResponse {
        status,
        body: serde_json::to_string(&body).expect("serialize response body"),
    }
}

fn spawn_scripted_server(script: Vec<ScriptedResponse>) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_thread = Arc::clone(&captured);

    let handle = thread::spawn(move || {
        for exchange in script {
            let (mut stream, _) = listener.accept().expect("accept request");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set read timeout");

            let request = read_request(&mut stream);
            captured_thread.lock().expect("capture lock").push(request);
            write_response(&mut stream, exchange.status, &exchange.body);
        }
    });

    TestServer {
        base_url: format!("http://{addr}"),
        captured,
        handle,
    }
}

fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    let mut buf = Vec::new();
    let mut header_end = None;
    let mut content_length = 0usize;

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
            content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
        }

        if let Some(header_end) = header_end
            && buf.len() >= header_end + content_length
        {
            break;
        }
    }

    let header_end = header_end.unwrap_or(buf.len());
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let body_len = content_length.min(buf.len().saturating_sub(header_end));
    let body = String::from_utf8_lossy(&buf[header_end..header_end + body_len]).to_string();

    let mut lines = head.lines();
    let request_line = lines.next().expect("request line");
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default().to_string();
    let path = request_parts.next().unwrap_or_default().to_string();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect();

    RecordedRequest {
        method,
        path,
        headers,
        body,
    }
}

fn write_response(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let wire = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(wire.as_bytes()).expect("write response");
    stream.flush().expect("flush response");
}

fn header<'a>(request: &'a RecordedRequest, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(key, _)| key == &name.to_ascii_lowercase())
        .map(|(_, value)| value.as_str())
}

fn gemini_provider(base_url: impl Into<String>) -> ProviderConfig {
    ProviderConfig {
        kind: ProviderKind::GeminiApi,
        base_url: Some(base_url.into()),
        api_key_env: Some("PATH".to_string()),
        command: None,
        args: None,
        timeout_ms: Some(1_500),
        ttft_timeout_ms: Some(DEFAULT_TTFT_TIMEOUT_MS),
        connect_timeout_ms: Some(5_000),
        extra_headers: None,
        max_concurrent: None,
    }
}

fn gemini_model(slug: &str) -> ModelProfile {
    ModelProfile {
        provider: "gemini".to_string(),
        slug: slug.to_string(),
        context_window: 1_048_576,
        max_output: Some(65_536),
        supports_tools: true,
        supports_thinking: true,
        supports_vision: false,
        supports_web_search: false,
        supports_mcp_tools: false,
        supports_partial: false,
        supports_grounding: false,
        supports_code_execution: false,
        supports_caching: false,
        provider_routing: None,
        tool_format: "gemini_native".to_string(),
        cost_input_per_m: None,
        cost_output_per_m: None,
        cost_input_per_m_high: None,
        cost_output_per_m_high: None,
        cost_cache_read_per_m: None,
        cost_cache_write_per_m: None,
        thinking_level: Some("high".to_string()),
        max_tools: None,
        max_tool_iterations: None,
        tokenizer_ratio: None,
        supports_search: false,
        supports_citations: false,
        supports_async: false,
        is_embedding_model: false,
        search_context_size: None,
        cost_per_request: None,
        use_max_completion_tokens: false,
        tier: None,
    }
}

fn gemini_config(base_url: impl Into<String>, model_key: &str, model: ModelProfile) -> RokoConfig {
    let mut config = RokoConfig::default();
    config
        .providers
        .insert("gemini".to_string(), gemini_provider(base_url));
    config.models.insert(model_key.to_string(), model);
    config
}

#[tokio::test]
async fn gemini_native_generate_content_with_function_calling() {
    let response_json = json!({
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [
                    { "text": "I need to inspect that file first." },
                    {
                        "functionCall": {
                            "name": "read_file",
                            "args": { "path": "src/lib.rs" },
                            "id": "gemini-call-7"
                        }
                    }
                ]
            },
            "finishReason": "STOP"
        }],
        "usageMetadata": {
            "promptTokenCount": 18,
            "candidatesTokenCount": 6,
            "totalTokenCount": 24
        }
    });
    let server = spawn_scripted_server(vec![response(200, response_json.clone())]);
    let agent = GeminiNativeAgent::new(
        "gemini-key".to_string(),
        server.base_url().to_string(),
        gemini_model("gemini-2.5-pro"),
        &AgentOptions::default(),
        SafetyLayer::with_defaults(),
    );

    let result = agent
        .run(&prompt("Inspect src/lib.rs"), &Context::now())
        .await;
    assert!(result.success);
    assert_eq!(
        result.output.body.as_text().expect("output text"),
        "I need to inspect that file first."
    );

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(
        requests[0].path,
        "/v1beta/models/gemini-2.5-pro:generateContent"
    );
    assert_eq!(header(&requests[0], "x-goog-api-key"), Some("gemini-key"));
    let request_body: Value = serde_json::from_str(&requests[0].body).expect("request body json");
    assert_eq!(request_body["contents"][0]["role"], "user");
    assert!(request_body.get("tools").is_none());

    let calls = GeminiTranslator
        .parse_calls(&BackendResponse::Json(response_json))
        .expect("parse function call");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "gemini-call-7");
    assert_eq!(calls[0].name, "read_file");
    assert_eq!(calls[0].arguments, json!({ "path": "src/lib.rs" }));

    server.join();
}

#[tokio::test]
async fn gemini_native_generate_content_with_google_search_grounding() {
    let response_json = json!({
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [{ "text": "Grounded answer." }]
            },
            "finishReason": "STOP",
            "groundingMetadata": {
                "webSearchQueries": ["rust edition 2024 let chains"],
                "groundingChunks": [{
                    "web": {
                        "uri": "https://doc.rust-lang.org/edition-guide/rust-2024/",
                        "title": "Rust Edition Guide"
                    }
                }],
                "groundingSupports": [{
                    "segment": {
                        "startIndex": 0,
                        "endIndex": 14,
                        "text": "Grounded answer"
                    },
                    "groundingChunkIndices": [0],
                    "confidenceScores": [0.98]
                }],
                "searchEntryPoint": {
                    "renderedContent": "<div>Search</div>"
                }
            }
        }],
        "usageMetadata": {
            "promptTokenCount": 11,
            "candidatesTokenCount": 5,
            "totalTokenCount": 16
        }
    });
    let server = spawn_scripted_server(vec![response(200, response_json)]);
    let agent = GeminiNativeAgent::new(
        "gemini-key".to_string(),
        server.base_url().to_string(),
        ModelProfile {
            slug: "gemini-3-flash-preview".to_string(),
            supports_grounding: true,
            supports_code_execution: false,
            ..gemini_model("gemini-3-flash-preview")
        },
        &AgentOptions::default(),
        SafetyLayer::with_defaults(),
    );

    let result = agent
        .run(&prompt("What changed in Rust 2024?"), &Context::now())
        .await;
    assert!(result.success);
    let metadata: GeminiMetadata =
        serde_json::from_str(result.output.tag("gemini_meta").expect("gemini_meta"))
            .expect("deserialize gemini metadata");
    let grounding = metadata
        .grounding_metadata
        .expect("grounding metadata should be preserved");
    assert_eq!(
        grounding.web_search_queries.expect("search queries"),
        vec!["rust edition 2024 let chains".to_string()]
    );
    assert_eq!(
        grounding.grounding_chunks.expect("grounding chunks")[0]
            .web
            .as_ref()
            .expect("web chunk")
            .title,
        "Rust Edition Guide"
    );
    assert_eq!(
        grounding.grounding_supports.expect("grounding supports")[0]
            .segment
            .text,
        "Grounded answer"
    );
    assert_eq!(
        grounding
            .search_entry_point
            .expect("search entry point")
            .rendered_content,
        "<div>Search</div>"
    );

    let requests = server.requests();
    let request_body: Value = serde_json::from_str(&requests[0].body).expect("request body json");
    let tools = request_body["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 1);
    assert!(tools[0].get("google_search").is_some());

    server.join();
}

#[tokio::test]
async fn gemini_native_generate_content_with_code_execution() {
    let response_json = json!({
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [
                    {
                        "executableCode": {
                            "language": "PYTHON",
                            "code": "print(2 + 2)"
                        }
                    },
                    {
                        "codeExecutionResult": {
                            "outcome": "OUTCOME_OK",
                            "output": "4"
                        }
                    },
                    { "text": "Validated." }
                ]
            },
            "finishReason": "STOP"
        }],
        "usageMetadata": {
            "promptTokenCount": 14,
            "candidatesTokenCount": 4,
            "totalTokenCount": 18
        }
    });
    let server = spawn_scripted_server(vec![response(200, response_json)]);
    let agent = GeminiNativeAgent::new(
        "gemini-key".to_string(),
        server.base_url().to_string(),
        ModelProfile {
            supports_grounding: false,
            supports_code_execution: true,
            ..gemini_model("gemini-2.5-pro")
        },
        &AgentOptions::default(),
        SafetyLayer::with_defaults(),
    );

    let result = agent.run(&prompt("Verify 2 + 2"), &Context::now()).await;
    assert!(result.success);
    assert_eq!(
        result.output.body.as_text().expect("output text"),
        "Validated."
    );

    let metadata: GeminiMetadata =
        serde_json::from_str(result.output.tag("gemini_meta").expect("gemini_meta"))
            .expect("deserialize gemini metadata");
    assert_eq!(metadata.code_execution_results.len(), 1);
    assert_eq!(metadata.code_execution_results[0].outcome, "OUTCOME_OK");
    assert_eq!(metadata.code_execution_results[0].output, "4");

    let requests = server.requests();
    let request_body: Value = serde_json::from_str(&requests[0].body).expect("request body json");
    let tools = request_body["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 1);
    assert!(tools[0].get("code_execution").is_some());

    server.join();
}

#[tokio::test]
async fn gemini_native_generate_content_with_thinking() {
    let response_json = json!({
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [{ "text": "Reasoned answer." }]
            },
            "finishReason": "STOP"
        }],
        "usageMetadata": {
            "promptTokenCount": 20,
            "candidatesTokenCount": 7,
            "totalTokenCount": 27,
            "thinkingTokenCount": 19
        }
    });
    let server = spawn_scripted_server(vec![response(200, response_json)]);
    let agent = GeminiNativeAgent::new(
        "gemini-key".to_string(),
        server.base_url().to_string(),
        ModelProfile {
            slug: "gemini-3.1-pro-preview".to_string(),
            thinking_level: Some("dynamic".to_string()),
            ..gemini_model("gemini-3.1-pro-preview")
        },
        &AgentOptions {
            effort: Some("high".to_string()),
            ..Default::default()
        },
        SafetyLayer::with_defaults(),
    );

    let result = agent.run(&prompt("Think carefully"), &Context::now()).await;
    assert!(result.success);
    let metadata: GeminiMetadata =
        serde_json::from_str(result.output.tag("gemini_meta").expect("gemini_meta"))
            .expect("deserialize gemini metadata");
    assert_eq!(metadata.thinking_tokens, Some(19));

    let requests = server.requests();
    let request_body: Value = serde_json::from_str(&requests[0].body).expect("request body json");
    assert_eq!(
        request_body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
        "high"
    );

    server.join();
}

#[tokio::test]
async fn gemini_openai_compat_path_for_flash_lite() {
    let server = spawn_scripted_server(vec![response(
        200,
        json!({
            "id": "chatcmpl-gemini-compat",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "compat ok"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 4
            }
        }),
    )]);
    let config = gemini_config(
        server.base_url(),
        "gemini-2-5-flash-lite",
        ModelProfile {
            slug: "gemini-2.5-flash-lite".to_string(),
            supports_thinking: false,
            supports_grounding: false,
            supports_code_execution: false,
            tool_format: "openai_json".to_string(),
            ..gemini_model("gemini-2.5-flash-lite")
        },
    );

    let agent = create_agent_for_model(&config, "gemini-2-5-flash-lite", AgentOptions::default())
        .expect("create flash-lite agent");
    assert_eq!(agent.name(), "gemini-compat:gemini-2.5-flash-lite");

    let result = agent.run(&prompt("Say hi"), &Context::now()).await;
    assert!(result.success);
    assert_eq!(
        result.output.body.as_text().expect("output text"),
        "compat ok"
    );

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/v1beta/openai/v1/chat/completions");
    assert!(
        header(&requests[0], "authorization").is_some_and(|value| value.starts_with("Bearer "))
    );
    let request_body: Value = serde_json::from_str(&requests[0].body).expect("request body json");
    assert_eq!(request_body["model"], "gemini-2.5-flash-lite");

    server.join();
}

#[test]
fn gemini_error_classification_handles_retry_context_overflow_and_auth() {
    let rate_limit = GeminiAdapter.classify_error(
        429,
        &json!({
            "error": {
                "details": [{ "retryDelay": "2.5s" }]
            }
        }),
    );
    match rate_limit {
        ProviderError::RateLimit {
            retry_after_ms: Some(ms),
        } => assert_eq!(ms, 2_500),
        other => panic!("unexpected rate-limit classification: {other:?}"),
    }

    let context_overflow = GeminiAdapter.classify_error(
        400,
        &json!({
            "error": {
                "message": "Request exceeds the maximum token limit."
            }
        }),
    );
    assert!(matches!(context_overflow, ProviderError::ContextOverflow));

    let auth = GeminiAdapter.classify_error(403, &Value::Null);
    assert!(matches!(auth, ProviderError::AuthFailure));
}

#[test]
fn gemini_tiered_pricing_calculation_switches_above_200k_context() {
    let table = CostTable::default();
    let gemini_pro = table
        .lookup("gemini-2.5-pro")
        .expect("gemini-2.5-pro pricing");

    let low_tier_total = gemini_pro.estimate_total(200_000, 50_000);
    let high_tier_total = gemini_pro.estimate_total(300_000, 50_000);

    assert!((low_tier_total - 0.75).abs() < 1e-9);
    assert!((high_tier_total - 1.50).abs() < 1e-9);
}

#[test]
fn gemini_function_call_id_round_trip() {
    let response = BackendResponse::Json(json!({
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [{
                    "functionCall": {
                        "name": "read_file",
                        "args": { "path": "src/lib.rs" },
                        "id": "gemini3-call-42"
                    }
                }]
            }
        }]
    }));

    let calls = GeminiTranslator
        .parse_calls(&response)
        .expect("parse Gemini 3 function call");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "gemini3-call-42");

    let RenderedResults::JsonMessages(messages) = GeminiTranslator.render_results(&[(
        calls[0].clone(),
        ToolResult::structured(r#"{"status":"ok"}"#),
    )]) else {
        panic!("expected JsonMessages");
    };
    assert_eq!(
        messages[0]["parts"][0]["functionResponse"]["id"],
        "gemini3-call-42"
    );
    assert_eq!(
        messages[0]["parts"][0]["functionResponse"]["response"],
        json!({ "status": "ok" })
    );
}
