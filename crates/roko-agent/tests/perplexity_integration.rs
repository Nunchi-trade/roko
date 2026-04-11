#![allow(missing_docs)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use roko_agent::Agent;
use roko_agent::perplexity::PerplexityChatAgent;
use roko_agent::perplexity::adapter::PerplexityAdapter;
use roko_agent::perplexity::deep_research::PerplexityDeepResearchAgent;
use roko_agent::perplexity::embed::PerplexityEmbedAgent;
use roko_agent::perplexity::search::{PerplexitySearchClient, SearchQuery};
use roko_agent::perplexity::types::{PerplexityMetadata, SearchOptions};
use roko_agent::provider::{ProviderAdapter, ProviderError};
use roko_core::{Body, Context, Kind, Signal};
use serde_json::{Value, json};

fn prompt(text: &str) -> Signal {
    Signal::builder(Kind::Prompt).body(Body::text(text)).build()
}

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    path: String,
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

fn response(status: u16, body: impl Into<String>) -> ScriptedResponse {
    ScriptedResponse {
        status,
        body: body.into(),
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

    RecordedRequest { method, path, body }
}

fn write_response(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        401 => "Unauthorized",
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

fn json_body(value: Value) -> String {
    serde_json::to_string(&value).expect("serialize json")
}

#[tokio::test]
async fn perplexity_chat_completions_preserve_citations_and_annotations() {
    let server = spawn_scripted_server(vec![response(
        200,
        json_body(json!({
            "id": "chatcmpl-pplx-001",
            "model": "sonar-pro",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Context engineering has become a core research topic.",
                    "annotations": [{
                        "start_index": 0,
                        "end_index": 21,
                        "title": "Context Engineering for LLMs",
                        "url": "https://arxiv.org/abs/2401.12345"
                    }]
                },
                "finish_reason": "stop"
            }],
            "citations": [
                "https://arxiv.org/abs/2401.12345",
                "https://example.com/research"
            ],
            "search_results": [{
                "url": "https://arxiv.org/abs/2401.12345",
                "title": "Context Engineering for LLMs",
                "content": "We study prompt and context design...",
                "date": "2024-01-15",
                "last_updated": "2024-03-01"
            }],
            "related_questions": ["What is context engineering?"],
            "usage": {
                "prompt_tokens": 18,
                "completion_tokens": 14,
                "total_tokens": 32
            }
        })),
    )]);

    let agent = PerplexityChatAgent::new(
        "pplx-key",
        server.base_url(),
        "sonar-pro",
        "perplexity:sonar-pro",
        60_000,
    );

    let result = agent
        .run(&prompt("Summarize context engineering."), &Context::now())
        .await;
    assert!(
        result.success,
        "{}",
        result.output.body.as_text().unwrap_or("unknown")
    );
    assert_eq!(
        result.output.body.as_text().expect("text body"),
        "Context engineering has become a core research topic."
    );

    let meta: PerplexityMetadata =
        serde_json::from_str(result.output.tag("pplx_meta").expect("pplx_meta tag"))
            .expect("valid pplx_meta json");
    assert_eq!(meta.citations.len(), 2);
    assert_eq!(meta.search_results.len(), 1);
    assert_eq!(meta.annotations.len(), 1);
    assert_eq!(meta.search_results[0].title, "Context Engineering for LLMs");
    assert_eq!(meta.annotations[0].url, "https://arxiv.org/abs/2401.12345");

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/chat/completions");
    let body: Value = serde_json::from_str(&requests[0].body).expect("request json");
    assert_eq!(body["model"], "sonar-pro");
    assert_eq!(body["messages"][0]["role"], "user");

    server.join();
}

#[tokio::test]
async fn perplexity_chat_academic_search_mode_is_injected() {
    let server = spawn_scripted_server(vec![response(
        200,
        json_body(json!({
            "id": "chatcmpl-pplx-002",
            "model": "sonar",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Academic search mode was applied."
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 4,
                "total_tokens": 14
            }
        })),
    )]);

    let opts = SearchOptions {
        search_mode: Some("academic".to_string()),
        ..Default::default()
    };
    let agent = PerplexityChatAgent::new(
        "pplx-key",
        server.base_url(),
        "sonar",
        "perplexity:sonar",
        60_000,
    )
    .with_search_options(opts);

    let result = agent
        .run(&prompt("Find recent papers."), &Context::now())
        .await;
    assert!(result.success);

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_str(&requests[0].body).expect("request json");
    assert_eq!(body["search_mode"], "academic");
    assert_eq!(body["model"], "sonar");

    server.join();
}

#[tokio::test]
async fn perplexity_deep_research_submit_and_poll_cycle() {
    let server = spawn_scripted_server(vec![
        response(200, json_body(json!({ "request_id": "req-123" }))),
        response(200, json_body(json!({ "status": "pending" }))),
        response(
            200,
            json_body(json!({
                "id": "resp-deep-001",
                "model": "sonar-deep-research",
                "status": "completed",
                "output": [{
                    "role": "assistant",
                    "content": [{
                        "type": "text",
                        "text": "Deep research completed."
                    }]
                }],
                "usage": {
                    "input_tokens": 240,
                    "output_tokens": 960,
                    "total_tokens": 1200
                },
                "citations": [
                    "https://example.com/paper-a",
                    "https://example.com/paper-b"
                ],
                "search_results": [{
                    "url": "https://example.com/paper-a",
                    "title": "Paper A",
                    "content": "Evidence supporting the conclusion...",
                    "date": "2024-08-01",
                    "last_updated": null
                }]
            })),
        ),
    ]);

    let agent = PerplexityDeepResearchAgent::new(
        "pplx-key",
        server.base_url(),
        "sonar-deep-research",
        "perplexity:sonar-deep-research",
    )
    .with_poll_interval_ms(0)
    .with_max_poll_attempts(2);

    let result = agent
        .run(
            &prompt("Investigate search-grounded research."),
            &Context::now(),
        )
        .await;
    assert!(
        result.success,
        "{}",
        result.output.body.as_text().unwrap_or("unknown")
    );
    assert_eq!(
        result.output.body.as_text().expect("text body"),
        "Deep research completed."
    );
    assert_eq!(result.output.tag("deep_research_id"), Some("resp-deep-001"));

    let meta: PerplexityMetadata =
        serde_json::from_str(result.output.tag("pplx_meta").expect("pplx_meta tag"))
            .expect("valid pplx_meta json");
    assert_eq!(meta.citations.len(), 2);
    assert_eq!(meta.search_results.len(), 1);

    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/v1/async/sonar");
    assert_eq!(requests[1].method, "GET");
    assert_eq!(requests[1].path, "/v1/async/sonar/req-123");
    assert_eq!(requests[2].method, "GET");
    assert_eq!(requests[2].path, "/v1/async/sonar/req-123");
    let submit_body: Value = serde_json::from_str(&requests[0].body).expect("submit json");
    assert_eq!(submit_body["model"], "sonar-deep-research");
    assert_eq!(
        submit_body["messages"][0]["content"],
        "Investigate search-grounded research."
    );

    server.join();
}

#[tokio::test]
async fn perplexity_search_single_query_uses_search_api() {
    let server = spawn_scripted_server(vec![response(
        200,
        json_body(json!({
            "results": [{
                "query": "rust async patterns",
                "results": [{
                    "url": "https://example.com/rust-async",
                    "title": "Rust Async Patterns",
                    "content": "A guide to async Rust...",
                    "date": "2025-01-01",
                    "last_updated": null
                }]
            }]
        })),
    )]);

    let client = PerplexitySearchClient::new("pplx-key").with_base_url(server.base_url());
    let response = client
        .search("rust async patterns")
        .await
        .expect("search ok");
    assert_eq!(response.query, "rust async patterns");
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].title, "Rust Async Patterns");

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/search");
    let body: Value = serde_json::from_str(&requests[0].body).expect("request json");
    assert_eq!(body["queries"].as_array().expect("queries").len(), 1);
    assert_eq!(body["queries"][0]["query"], "rust async patterns");

    server.join();
}

#[tokio::test]
async fn perplexity_search_batch_five_queries_are_sent_together() {
    let server = spawn_scripted_server(vec![response(
        200,
        json_body(json!({
            "results": [
                {
                    "query": "q1",
                    "results": []
                },
                {
                    "query": "q2",
                    "results": []
                },
                {
                    "query": "q3",
                    "results": []
                },
                {
                    "query": "q4",
                    "results": []
                },
                {
                    "query": "q5",
                    "results": []
                }
            ]
        })),
    )]);

    let client = PerplexitySearchClient::new("pplx-key").with_base_url(server.base_url());
    let queries = vec![
        SearchQuery {
            query: "q1".to_string(),
            ..Default::default()
        },
        SearchQuery {
            query: "q2".to_string(),
            ..Default::default()
        },
        SearchQuery {
            query: "q3".to_string(),
            ..Default::default()
        },
        SearchQuery {
            query: "q4".to_string(),
            ..Default::default()
        },
        SearchQuery {
            query: "q5".to_string(),
            ..Default::default()
        },
    ];

    let responses = client
        .search_batch(&queries)
        .await
        .expect("batch search ok");
    assert_eq!(responses.len(), 5);
    assert_eq!(responses[4].query, "q5");

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_str(&requests[0].body).expect("request json");
    let queries_json = body["queries"].as_array().expect("queries array");
    assert_eq!(queries_json.len(), 5);
    assert_eq!(queries_json[0]["query"], "q1");
    assert_eq!(queries_json[4]["query"], "q5");

    server.join();
}

#[tokio::test]
async fn perplexity_embedding_single_text_uses_embeddings_api() {
    let server = spawn_scripted_server(vec![response(
        200,
        json_body(json!({
            "object": "list",
            "model": "pplx-embed-v1-4b",
            "data": [{
                "object": "embedding",
                "index": 0,
                "embedding": [0.125, 0.25, 0.5]
            }],
            "usage": {
                "prompt_tokens": 2,
                "total_tokens": 2
            }
        })),
    )]);

    let agent = PerplexityEmbedAgent::new(
        "pplx-key",
        format!("{}/v1", server.base_url()),
        "pplx-embed-v1-4b",
    );
    let vectors = agent.embed(&["hello world"]).await.expect("embed ok");
    assert_eq!(vectors.len(), 1);
    assert_eq!(vectors[0].len(), 3);
    assert!((vectors[0][0] - 0.125).abs() < 1e-6);
    assert!((vectors[0][2] - 0.5).abs() < 1e-6);

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/v1/embeddings");
    let body: Value = serde_json::from_str(&requests[0].body).expect("request json");
    assert_eq!(body["model"], "pplx-embed-v1-4b");
    assert_eq!(body["input"][0], "hello world");

    server.join();
}

#[test]
fn perplexity_error_classification_maps_429_401_and_404() {
    let rate_limit = PerplexityAdapter.classify_error(429, &json!({ "retry_after": 15 }));
    assert!(matches!(
        rate_limit,
        ProviderError::RateLimit {
            retry_after_ms: Some(15_000)
        }
    ));

    let auth = PerplexityAdapter.classify_error(401, &Value::Null);
    assert!(matches!(auth, ProviderError::AuthFailure));

    let not_found = PerplexityAdapter.classify_error(404, &Value::Null);
    assert!(matches!(not_found, ProviderError::ModelNotFound));
}

#[tokio::test]
async fn perplexity_domain_and_recency_filters_are_injected() {
    let server = spawn_scripted_server(vec![response(
        200,
        json_body(json!({
            "id": "chatcmpl-pplx-003",
            "model": "sonar",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Filtered search completed."
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 9,
                "completion_tokens": 5,
                "total_tokens": 14
            }
        })),
    )]);

    let opts = SearchOptions {
        search_domain_filter: Some(vec!["arxiv.org".to_string(), "nature.com".to_string()]),
        search_recency_filter: Some("week".to_string()),
        ..Default::default()
    };
    let agent = PerplexityChatAgent::new(
        "pplx-key",
        server.base_url(),
        "sonar",
        "perplexity:sonar",
        60_000,
    )
    .with_search_options(opts);

    let result = agent
        .run(&prompt("Find relevant research."), &Context::now())
        .await;
    assert!(result.success);

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_str(&requests[0].body).expect("request json");
    assert_eq!(
        body["search_domain_filter"],
        json!(["arxiv.org", "nature.com"])
    );
    assert_eq!(body["search_recency_filter"], "week");

    server.join();
}
