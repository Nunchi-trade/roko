//! `PerplexityChatAgent` — Perplexity Sonar chat completions with search extensions.
//!
//! Calls `POST /chat/completions` with Perplexity-specific search parameters
//! and preserves citations, search results, and annotations in the output
//! signal's `"pplx_meta"` tag (JSON-serialised [`PerplexityMetadata`]).

use crate::agent::{Agent, AgentResult};
#[cfg(test)]
use crate::http::HttpPostError;
use crate::http::{HttpPoster, ReqwestPoster};
use crate::perplexity::types::{Annotation, PerplexityMetadata, SearchOptions, SearchResult};
use crate::translate::openai::parse_usage;
use crate::usage::Usage;
use async_trait::async_trait;
use roko_core::{Body, Context, Engram, Kind, Provenance};
use serde_json::{Value, json};
use std::time::Instant;

/// Perplexity Sonar chat agent with search-grounded completions.
///
/// Unlike the generic `OpenAiAgent`, this agent:
/// - Injects Perplexity search options (`search_domain_filter`,
///   `search_recency_filter`, `web_search_options`, etc.) into the request.
/// - Parses `citations`, `search_results`, and per-message `annotations`
///   from the response and stores them in the output signal's `"pplx_meta"` tag.
pub struct PerplexityChatAgent {
    api_key: String,
    base_url: String,
    model_slug: String,
    search_options: SearchOptions,
    system_prompt: Option<String>,
    timeout_ms: u64,
    name: String,
    poster: Box<dyn HttpPoster>,
}

impl PerplexityChatAgent {
    /// Construct with the production reqwest-backed HTTP poster.
    #[must_use]
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model_slug: impl Into<String>,
        name: impl Into<String>,
        timeout_ms: u64,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
            model_slug: model_slug.into(),
            search_options: SearchOptions::default(),
            system_prompt: None,
            timeout_ms,
            name: name.into(),
            poster: Box::new(ReqwestPoster::new()),
        }
    }

    /// Override Perplexity search options (domain filters, recency, mode, etc.).
    #[must_use]
    pub fn with_search_options(mut self, options: SearchOptions) -> Self {
        self.search_options = options;
        self
    }

    /// Set the system prompt injected before user messages.
    #[must_use]
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    #[cfg(test)]
    fn with_poster(mut self, poster: Box<dyn HttpPoster>) -> Self {
        self.poster = poster;
        self
    }

    fn endpoint(&self) -> String {
        let trimmed = self.base_url.trim_end_matches('/');
        format!("{trimmed}/chat/completions")
    }

    fn headers(&self) -> Vec<(String, String)> {
        vec![
            (
                "Authorization".to_string(),
                format!("Bearer {}", self.api_key),
            ),
            ("Content-Type".to_string(), "application/json".to_string()),
        ]
    }

    /// Build the request body with Perplexity-specific search parameters.
    fn build_request(&self, prompt_text: &str) -> Value {
        let mut messages: Vec<Value> = Vec::new();
        if let Some(ref sys) = self.system_prompt {
            messages.push(json!({ "role": "system", "content": sys }));
        }
        messages.push(json!({ "role": "user", "content": prompt_text }));

        let mut body = json!({
            "model": self.model_slug,
            "messages": messages,
        });

        let opts = &self.search_options;
        if let Some(ref filter) = opts.search_domain_filter {
            body["search_domain_filter"] = json!(filter);
        }
        if let Some(ref recency) = opts.search_recency_filter {
            body["search_recency_filter"] = json!(recency);
        }
        if let Some(ref mode) = opts.search_mode {
            body["search_mode"] = json!(mode);
        }
        if let Some(images) = opts.return_images {
            body["return_images"] = json!(images);
        }
        if let Some(related) = opts.return_related_questions {
            body["return_related_questions"] = json!(related);
        }
        if let Some(ref size) = opts.search_context_size {
            body["web_search_options"] = json!({ "search_context_size": size });
        }
        if let Some(ref after) = opts.search_after_date_filter {
            body["search_after_date_filter"] = json!(after);
        }
        if let Some(ref before) = opts.search_before_date_filter {
            body["search_before_date_filter"] = json!(before);
        }

        body
    }

    /// Parse Perplexity extensions: citations, search_results, annotations.
    fn parse_pplx_meta(&self, raw: &Value) -> PerplexityMetadata {
        let citations: Vec<String> = raw
            .get("citations")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let search_results: Vec<SearchResult> = raw
            .get("search_results")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let annotations: Vec<Annotation> = raw
            .pointer("/choices/0/message/annotations")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let related_questions: Vec<String> = raw
            .get("related_questions")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        PerplexityMetadata {
            citations,
            search_results,
            annotations,
            related_questions,
        }
    }

    fn failure(&self, input: &Engram, reason: String, started: &Instant) -> AgentResult {
        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let output = input
            .derive(Kind::AgentOutput, Body::text(reason))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("failed", "true")
            .build();
        AgentResult::fail(output).with_usage(Usage {
            wall_ms,
            ..Default::default()
        })
    }
}

#[async_trait]
impl Agent for PerplexityChatAgent {
    async fn run(&self, input: &Engram, _ctx: &Context) -> AgentResult {
        let started = Instant::now();

        let prompt_text = match input.body.as_text() {
            Ok(s) => s.to_string(),
            Err(_) => match serde_json::to_string(&input.body) {
                Ok(s) => s,
                Err(e) => {
                    return self.failure(
                        input,
                        format!("input body not readable as text or json: {e}"),
                        &started,
                    );
                }
            },
        };

        let body = self.build_request(&prompt_text);
        let body_bytes = match serde_json::to_vec(&body) {
            Ok(v) => v,
            Err(e) => {
                return self.failure(input, format!("request serialize failed: {e}"), &started);
            }
        };

        let url = self.endpoint();
        let headers = self.headers();

        let response_text = match self
            .poster
            .post_json(&url, &headers, &body_bytes, self.timeout_ms)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                return self.failure(input, format!("http error: {e}"), &started);
            }
        };

        let parsed: Value = match serde_json::from_str(&response_text) {
            Ok(v) => v,
            Err(e) => {
                return self.failure(input, format!("malformed response json: {e}"), &started);
            }
        };

        if let Some(err) = parsed.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown api error");
            return self.failure(input, format!("api error: {msg}"), &started);
        }

        let content = parsed
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str);
        let content = match content {
            Some(c) => c.to_string(),
            None => {
                return self.failure(
                    input,
                    "response missing choices[0].message.content".to_string(),
                    &started,
                );
            }
        };

        let pplx_meta = self.parse_pplx_meta(&parsed);
        let meta_json = serde_json::to_string(&pplx_meta).unwrap_or_default();

        let usage = parse_usage(&parsed);
        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        let out_signal = input
            .derive(Kind::AgentOutput, Body::text(&content))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("model", &self.model_slug)
            .tag("pplx_meta", &meta_json)
            .build();

        AgentResult::ok(out_signal).with_usage(Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            wall_ms,
            ..Default::default()
        })
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn supports_streaming(&self) -> bool {
        false
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_types)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug, Default)]
    struct Captured {
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    struct MockPoster {
        captured: Arc<Mutex<Option<Captured>>>,
        response: Result<String, HttpPostError>,
    }

    impl MockPoster {
        fn ok(body: impl Into<String>) -> (Self, Arc<Mutex<Option<Captured>>>) {
            let captured = Arc::new(Mutex::new(None));
            (
                Self {
                    captured: captured.clone(),
                    response: Ok(body.into()),
                },
                captured,
            )
        }

        fn err(msg: impl Into<String>) -> (Self, Arc<Mutex<Option<Captured>>>) {
            let captured = Arc::new(Mutex::new(None));
            let m: String = msg.into();
            let err = m
                .strip_prefix("http ")
                .and_then(|rest| {
                    let (code, tail) = rest.split_once(':')?;
                    let code: u16 = code.trim().parse().ok()?;
                    Some(HttpPostError::http(code, tail.trim_start()))
                })
                .unwrap_or_else(|| HttpPostError::transport(m));
            (
                Self {
                    captured: captured.clone(),
                    response: Err(err),
                },
                captured,
            )
        }
    }

    #[async_trait]
    impl HttpPoster for MockPoster {
        async fn post_json(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &[u8],
            _timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            *self.captured.lock().expect("lock mock captured") = Some(Captured {
                url: url.to_string(),
                headers: headers.to_vec(),
                body: body.to_vec(),
            });
            self.response.clone()
        }
    }

    fn prompt(text: &str) -> Engram {
        Engram::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    fn agent_with(poster: Box<dyn HttpPoster>) -> PerplexityChatAgent {
        PerplexityChatAgent::new(
            "pplx-key",
            "https://api.perplexity.ai",
            "sonar",
            "perplexity:sonar",
            120_000,
        )
        .with_poster(poster)
    }

    fn canned_ok(content: &str) -> String {
        json!({
            "id": "resp-test",
            "model": "sonar",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": content },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
        })
        .to_string()
    }

    fn canned_with_citations(content: &str) -> String {
        json!({
            "id": "resp-cite",
            "model": "sonar",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content,
                    "annotations": [
                        {
                            "start_index": 0,
                            "end_index": 10,
                            "title": "Paper Title",
                            "url": "https://example.com/paper"
                        }
                    ]
                },
                "finish_reason": "stop"
            }],
            "citations": [
                "https://example.com/paper",
                "https://example.com/other"
            ],
            "search_results": [
                {
                    "url": "https://example.com/paper",
                    "title": "Paper Title",
                    "content": "Abstract text here",
                    "date": "2024-01-01",
                    "last_updated": null
                }
            ],
            "related_questions": ["What is this about?"],
            "usage": { "prompt_tokens": 20, "completion_tokens": 10, "total_tokens": 30 }
        })
        .to_string()
    }

    // ── Core behaviour ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn perplexity_chat_agent_successful_response_produces_agent_output() {
        let (mock, _) = MockPoster::ok(canned_ok("hello from perplexity"));
        let agent = agent_with(Box::new(mock));
        let result = agent.run(&prompt("hi"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.kind, Kind::AgentOutput);
        assert_eq!(
            result.output.body.as_text().expect("text body"),
            "hello from perplexity"
        );
        assert_eq!(result.output.tag("model"), Some("sonar"));
    }

    #[tokio::test]
    async fn perplexity_chat_agent_citations_preserved_in_pplx_meta_tag() {
        let (mock, _) = MockPoster::ok(canned_with_citations("answer with citations"));
        let agent = agent_with(Box::new(mock));
        let result = agent
            .run(&prompt("research question"), &Context::now())
            .await;
        assert!(result.success);

        let meta_json = result
            .output
            .tag("pplx_meta")
            .expect("pplx_meta tag must be present");

        let meta: PerplexityMetadata =
            serde_json::from_str(meta_json).expect("pplx_meta must be valid JSON");

        assert_eq!(meta.citations.len(), 2);
        assert_eq!(meta.citations[0], "https://example.com/paper");
        assert_eq!(meta.citations[1], "https://example.com/other");
    }

    #[tokio::test]
    async fn perplexity_chat_agent_search_results_preserved_in_pplx_meta_tag() {
        let (mock, _) = MockPoster::ok(canned_with_citations("answer"));
        let agent = agent_with(Box::new(mock));
        let result = agent.run(&prompt("q"), &Context::now()).await;
        assert!(result.success);

        let meta: PerplexityMetadata =
            serde_json::from_str(result.output.tag("pplx_meta").expect("pplx_meta tag"))
                .expect("valid JSON");

        assert_eq!(meta.search_results.len(), 1);
        assert_eq!(meta.search_results[0].title, "Paper Title");
        assert_eq!(meta.search_results[0].url, "https://example.com/paper");
        assert_eq!(meta.search_results[0].date, Some("2024-01-01".to_string()));
        assert!(meta.search_results[0].last_updated.is_none());
    }

    #[tokio::test]
    async fn perplexity_chat_agent_annotations_preserved_in_pplx_meta_tag() {
        let (mock, _) = MockPoster::ok(canned_with_citations("answer"));
        let agent = agent_with(Box::new(mock));
        let result = agent.run(&prompt("q"), &Context::now()).await;
        assert!(result.success);

        let meta: PerplexityMetadata =
            serde_json::from_str(result.output.tag("pplx_meta").expect("pplx_meta tag"))
                .expect("valid JSON");

        assert_eq!(meta.annotations.len(), 1);
        assert_eq!(meta.annotations[0].start_index, 0);
        assert_eq!(meta.annotations[0].end_index, 10);
        assert_eq!(meta.annotations[0].title, "Paper Title");
    }

    #[tokio::test]
    async fn perplexity_chat_agent_empty_pplx_meta_when_no_citations() {
        let (mock, _) = MockPoster::ok(canned_ok("plain answer"));
        let agent = agent_with(Box::new(mock));
        let result = agent.run(&prompt("q"), &Context::now()).await;
        assert!(result.success);

        let meta: PerplexityMetadata =
            serde_json::from_str(result.output.tag("pplx_meta").expect("pplx_meta tag"))
                .expect("valid JSON");

        assert!(meta.citations.is_empty());
        assert!(meta.search_results.is_empty());
        assert!(meta.annotations.is_empty());
    }

    #[tokio::test]
    async fn perplexity_chat_agent_usage_is_parsed() {
        let (mock, _) = MockPoster::ok(canned_ok("ok"));
        let agent = agent_with(Box::new(mock));
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.usage.input_tokens, 10);
        assert_eq!(result.usage.output_tokens, 5);
    }

    // ── Search option injection ────────────────────────────────────────────────

    #[tokio::test]
    async fn perplexity_chat_agent_search_options_injected_into_request_body() {
        let (mock, captured) = MockPoster::ok(canned_ok("ok"));
        let opts = SearchOptions {
            search_recency_filter: Some("week".to_string()),
            search_mode: Some("academic".to_string()),
            search_context_size: Some("high".to_string()),
            return_images: Some(false),
            ..Default::default()
        };
        let agent = PerplexityChatAgent::new(
            "key",
            "https://api.perplexity.ai",
            "sonar-pro",
            "perplexity:sonar-pro",
            60_000,
        )
        .with_search_options(opts)
        .with_poster(Box::new(mock));

        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let c = captured.lock().expect("lock").clone().expect("captured");
        let body: Value = serde_json::from_slice(&c.body).expect("body is json");

        assert_eq!(body["model"], "sonar-pro");
        assert_eq!(body["search_recency_filter"], "week");
        assert_eq!(body["search_mode"], "academic");
        assert_eq!(body["web_search_options"]["search_context_size"], "high");
        assert_eq!(body["return_images"], false);
    }

    #[tokio::test]
    async fn perplexity_chat_agent_domain_filter_injected() {
        let (mock, captured) = MockPoster::ok(canned_ok("ok"));
        let opts = SearchOptions {
            search_domain_filter: Some(vec!["arxiv.org".to_string(), "nature.com".to_string()]),
            ..Default::default()
        };
        let agent = PerplexityChatAgent::new(
            "key",
            "https://api.perplexity.ai",
            "sonar",
            "perplexity:sonar",
            60_000,
        )
        .with_search_options(opts)
        .with_poster(Box::new(mock));

        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let c = captured.lock().expect("lock").clone().expect("captured");
        let body: Value = serde_json::from_slice(&c.body).expect("body is json");
        let filter = body["search_domain_filter"].as_array().expect("array");
        assert_eq!(filter.len(), 2);
        assert_eq!(filter[0], "arxiv.org");
        assert_eq!(filter[1], "nature.com");
    }

    #[tokio::test]
    async fn perplexity_chat_agent_system_prompt_injected_as_first_message() {
        let (mock, captured) = MockPoster::ok(canned_ok("ok"));
        let agent = PerplexityChatAgent::new(
            "key",
            "https://api.perplexity.ai",
            "sonar",
            "perplexity:sonar",
            60_000,
        )
        .with_system_prompt("You are a research assistant.")
        .with_poster(Box::new(mock));

        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let c = captured.lock().expect("lock").clone().expect("captured");
        let body: Value = serde_json::from_slice(&c.body).expect("body is json");
        let msgs = body["messages"].as_array().expect("messages");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "You are a research assistant.");
        assert_eq!(msgs[1]["role"], "user");
    }

    #[tokio::test]
    async fn perplexity_chat_agent_no_system_prompt_sends_single_user_message() {
        let (mock, captured) = MockPoster::ok(canned_ok("ok"));
        let agent = agent_with(Box::new(mock));
        let _ = agent.run(&prompt("hello"), &Context::now()).await;
        let c = captured.lock().expect("lock").clone().expect("captured");
        let body: Value = serde_json::from_slice(&c.body).expect("body is json");
        let msgs = body["messages"].as_array().expect("messages");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], "hello");
    }

    // ── Error handling ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn perplexity_chat_agent_http_401_is_failure() {
        let (mock, _) = MockPoster::err("http 401: unauthorized");
        let agent = agent_with(Box::new(mock));
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert_eq!(result.output.tag("failed"), Some("true"));
        assert!(result.output.body.as_text().expect("text").contains("401"));
    }

    #[tokio::test]
    async fn perplexity_chat_agent_api_error_object_is_failure() {
        let body = json!({
            "error": { "message": "model not found", "type": "not_found" }
        })
        .to_string();
        let (mock, _) = MockPoster::ok(body);
        let agent = agent_with(Box::new(mock));
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .expect("text")
                .contains("model not found")
        );
    }

    #[tokio::test]
    async fn perplexity_chat_agent_malformed_json_is_failure() {
        let (mock, _) = MockPoster::ok("not { valid json");
        let agent = agent_with(Box::new(mock));
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .expect("text")
                .contains("malformed")
        );
    }

    #[tokio::test]
    async fn perplexity_chat_agent_missing_choices_is_failure() {
        let body = json!({ "id": "x", "model": "sonar" }).to_string();
        let (mock, _) = MockPoster::ok(body);
        let agent = agent_with(Box::new(mock));
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .expect("text")
                .contains("missing choices")
        );
    }

    // ── Wire details ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn perplexity_chat_agent_bearer_header_is_set() {
        let (mock, captured) = MockPoster::ok(canned_ok("ok"));
        let agent = agent_with(Box::new(mock));
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let c = captured.lock().expect("lock").clone().expect("captured");
        let auth = c
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("Authorization"))
            .map(|(_, v)| v.clone())
            .expect("Authorization header");
        assert_eq!(auth, "Bearer pplx-key");
    }

    #[tokio::test]
    async fn perplexity_chat_agent_endpoint_uses_base_url() {
        let (mock, captured) = MockPoster::ok(canned_ok("ok"));
        let agent = PerplexityChatAgent::new(
            "key",
            "https://api.perplexity.ai",
            "sonar",
            "perplexity:sonar",
            60_000,
        )
        .with_poster(Box::new(mock));
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let c = captured.lock().expect("lock").clone().expect("captured");
        assert_eq!(c.url, "https://api.perplexity.ai/chat/completions");
    }

    #[tokio::test]
    async fn perplexity_chat_agent_trailing_slash_base_url_normalized() {
        let (mock, captured) = MockPoster::ok(canned_ok("ok"));
        let agent = PerplexityChatAgent::new(
            "key",
            "https://api.perplexity.ai/",
            "sonar",
            "perplexity:sonar",
            60_000,
        )
        .with_poster(Box::new(mock));
        let _ = agent.run(&prompt("x"), &Context::now()).await;
        let c = captured.lock().expect("lock").clone().expect("captured");
        assert_eq!(c.url, "https://api.perplexity.ai/chat/completions");
    }

    #[test]
    fn perplexity_chat_agent_name_is_returned() {
        let agent = PerplexityChatAgent::new(
            "k",
            "https://api.perplexity.ai",
            "sonar",
            "my-pplx-agent",
            60_000,
        );
        assert_eq!(agent.name(), "my-pplx-agent");
        assert!(!agent.supports_streaming());
    }
}
