//! `PerplexityDeepResearchAgent` — async polling agent for `sonar-deep-research`.
//!
//! Deep research jobs are long-running (1–10 minutes). This agent submits an
//! async job via `POST /v1/async/sonar`, then polls
//! `GET /v1/async/sonar/{request_id}` until the job completes, fails, or
//! the poll limit is exhausted.

use crate::agent::{Agent, AgentResult};
#[cfg(test)]
use crate::http::HttpPostError;
use crate::http::{HttpPoster, ReqwestPoster};
use crate::perplexity::types::{AgentResponse, PerplexityMetadata};
use crate::usage::Usage;
use async_trait::async_trait;
use roko_core::{Body, Context, Kind, Provenance, Signal};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// Default polling interval between status checks.
const DEFAULT_POLL_INTERVAL_MS: u64 = 5_000;
/// Default maximum number of polling attempts (120 × 5 s = 10 minutes).
const DEFAULT_MAX_POLL_ATTEMPTS: u32 = 120;

/// Async polling agent for Perplexity's `sonar-deep-research` model.
///
/// Submits a research job via `POST /v1/async/sonar` and polls
/// `GET /v1/async/sonar/{request_id}` until the job status transitions to
/// `"completed"` or `"failed"`, or the poll limit is reached.
pub struct PerplexityDeepResearchAgent {
    api_key: String,
    base_url: String,
    model_slug: String,
    poll_interval_ms: u64,
    max_poll_attempts: u32,
    name: String,
    poster: Box<dyn HttpPoster>,
}

impl PerplexityDeepResearchAgent {
    /// Construct with the production reqwest-backed HTTP poster.
    #[must_use]
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model_slug: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
            model_slug: model_slug.into(),
            poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
            max_poll_attempts: DEFAULT_MAX_POLL_ATTEMPTS,
            name: name.into(),
            poster: Box::new(ReqwestPoster::new()),
        }
    }

    /// Override the poll interval (default 5000 ms).
    #[must_use]
    pub const fn with_poll_interval_ms(mut self, ms: u64) -> Self {
        self.poll_interval_ms = ms;
        self
    }

    /// Override the maximum number of poll attempts (default 120).
    #[must_use]
    pub const fn with_max_poll_attempts(mut self, attempts: u32) -> Self {
        self.max_poll_attempts = attempts;
        self
    }

    #[cfg(test)]
    fn with_poster(mut self, poster: Box<dyn HttpPoster>) -> Self {
        self.poster = poster;
        self
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

    fn submit_url(&self) -> String {
        let trimmed = self.base_url.trim_end_matches('/');
        format!("{trimmed}/v1/async/sonar")
    }

    fn poll_url(&self, request_id: &str) -> String {
        let trimmed = self.base_url.trim_end_matches('/');
        format!("{trimmed}/v1/async/sonar/{request_id}")
    }

    /// Submit an async deep research job. Returns the `request_id`.
    async fn submit(&self, prompt: &str) -> Result<String, String> {
        let body = json!({
            "model": self.model_slug,
            "messages": [{ "role": "user", "content": prompt }]
        });
        let body_bytes = serde_json::to_vec(&body).map_err(|e| format!("serialize failed: {e}"))?;

        let response_text = self
            .poster
            .post_json(&self.submit_url(), &self.headers(), &body_bytes, 30_000)
            .await
            .map_err(|e| format!("submit http error: {e}"))?;

        let parsed: Value = serde_json::from_str(&response_text)
            .map_err(|e| format!("submit malformed json: {e}"))?;

        if let Some(err) = parsed.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown api error");
            return Err(format!("api error: {msg}"));
        }

        parsed
            .get("request_id")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .ok_or_else(|| "submit response missing request_id".to_string())
    }

    /// Poll for the result of a submitted job.
    ///
    /// Returns `None` if still pending/processing, `Some(AgentResponse)` on
    /// completion, or an error if the job failed.
    async fn poll(&self, request_id: &str) -> Result<Option<AgentResponse>, String> {
        let response_text = self
            .poster
            .get_json(&self.poll_url(request_id), &self.headers(), 30_000)
            .await
            .map_err(|e| format!("poll http error: {e}"))?;

        let parsed: Value = serde_json::from_str(&response_text)
            .map_err(|e| format!("poll malformed json: {e}"))?;

        if let Some(err) = parsed.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown api error");
            return Err(format!("api error: {msg}"));
        }

        let status = parsed
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        match status {
            "completed" => {
                let response: AgentResponse = serde_json::from_value(parsed)
                    .map_err(|e| format!("parse completed response failed: {e}"))?;
                Ok(Some(response))
            }
            "failed" => {
                let msg = parsed
                    .get("error_message")
                    .and_then(Value::as_str)
                    .unwrap_or("deep research job failed");
                Err(format!("deep research failed: {msg}"))
            }
            "pending" | "processing" => Ok(None),
            other => Err(format!("unexpected poll status: {other}")),
        }
    }

    /// Submit a deep research job and poll until completion or timeout.
    async fn run_deep_research(&self, prompt: &str) -> Result<AgentResponse, String> {
        let request_id = self.submit(prompt).await?;

        for _ in 0..self.max_poll_attempts {
            tokio::time::sleep(Duration::from_millis(self.poll_interval_ms)).await;
            if let Some(response) = self.poll(&request_id).await? {
                return Ok(response);
            }
        }

        Err(format!(
            "deep research timed out after {} attempts",
            self.max_poll_attempts
        ))
    }

    fn failure(&self, input: &Signal, reason: String, started: &Instant) -> AgentResult {
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
impl Agent for PerplexityDeepResearchAgent {
    async fn run(&self, input: &Signal, _ctx: &Context) -> AgentResult {
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

        let agent_response = match self.run_deep_research(&prompt_text).await {
            Ok(r) => r,
            Err(e) => return self.failure(input, e, &started),
        };

        // Extract text content from the first assistant output item.
        let content = agent_response
            .output
            .iter()
            .find(|item| item.role == "assistant")
            .and_then(|item| item.content.iter().find(|b| b.content_type == "text"))
            .and_then(|b| b.text.as_deref())
            .unwrap_or("")
            .to_string();

        // Preserve search metadata in the output signal tag.
        let pplx_meta = PerplexityMetadata {
            citations: agent_response.citations,
            search_results: agent_response.search_results,
            annotations: vec![],
            related_questions: vec![],
        };
        let meta_json = serde_json::to_string(&pplx_meta).unwrap_or_default();

        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        let out_signal = input
            .derive(Kind::AgentOutput, Body::text(&content))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("model", &self.model_slug)
            .tag("pplx_meta", &meta_json)
            .tag("deep_research_id", &agent_response.id)
            .build();

        AgentResult::ok(out_signal).with_usage(Usage {
            input_tokens: agent_response.usage.input_tokens as u32,
            output_tokens: agent_response.usage.output_tokens as u32,
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
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    /// A mock that returns canned POST responses from a queue, and canned GET
    /// responses from a separate queue — allowing stateful polling simulation.
    struct SequentialMock {
        post_responses: Arc<Mutex<VecDeque<Result<String, HttpPostError>>>>,
        get_responses: Arc<Mutex<VecDeque<Result<String, HttpPostError>>>>,
    }

    impl SequentialMock {
        fn new(
            post_responses: Vec<Result<String, HttpPostError>>,
            get_responses: Vec<Result<String, HttpPostError>>,
        ) -> Self {
            Self {
                post_responses: Arc::new(Mutex::new(post_responses.into_iter().collect())),
                get_responses: Arc::new(Mutex::new(get_responses.into_iter().collect())),
            }
        }
    }

    #[async_trait]
    impl HttpPoster for SequentialMock {
        async fn post_json(
            &self,
            _url: &str,
            _headers: &[(String, String)],
            _body: &[u8],
            _timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            self.post_responses
                .lock()
                .expect("lock")
                .pop_front()
                .unwrap_or_else(|| Err(HttpPostError::transport("post queue exhausted")))
        }

        async fn get_json(
            &self,
            _url: &str,
            _headers: &[(String, String)],
            _timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            self.get_responses
                .lock()
                .expect("lock")
                .pop_front()
                .unwrap_or_else(|| Err(HttpPostError::transport("get queue exhausted")))
        }
    }

    fn submit_ok(request_id: &str) -> String {
        serde_json::json!({ "request_id": request_id }).to_string()
    }

    fn poll_pending() -> String {
        serde_json::json!({ "status": "pending" }).to_string()
    }

    fn poll_processing() -> String {
        serde_json::json!({ "status": "processing" }).to_string()
    }

    fn poll_completed(content: &str) -> String {
        serde_json::json!({
            "id": "resp-deep-001",
            "model": "sonar-deep-research",
            "status": "completed",
            "output": [{
                "role": "assistant",
                "content": [{ "type": "text", "text": content }]
            }],
            "usage": { "input_tokens": 500, "output_tokens": 1200, "total_tokens": 1700 },
            "citations": ["https://example.com/paper1", "https://example.com/paper2"],
            "search_results": []
        })
        .to_string()
    }

    fn poll_failed() -> String {
        serde_json::json!({
            "status": "failed",
            "error_message": "research job timed out internally"
        })
        .to_string()
    }

    fn prompt(text: &str) -> Signal {
        Signal::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    fn agent_with(mock: SequentialMock) -> PerplexityDeepResearchAgent {
        PerplexityDeepResearchAgent::new(
            "pplx-key",
            "https://api.perplexity.ai",
            "sonar-deep-research",
            "perplexity:sonar-deep-research",
        )
        .with_poll_interval_ms(0) // no real delay in tests
        .with_poster(Box::new(mock))
    }

    // ── Polling loop behaviour ─────────────────────────────────────────────────

    #[tokio::test]
    async fn deep_research_polling_completes_immediately() {
        let mock = SequentialMock::new(
            vec![Ok(submit_ok("req-001"))],
            vec![Ok(poll_completed("deep research result"))],
        );
        let result = agent_with(mock)
            .run(&prompt("research question"), &Context::now())
            .await;
        assert!(result.success);
        assert_eq!(
            result.output.body.as_text().expect("text body"),
            "deep research result"
        );
        assert_eq!(result.output.tag("deep_research_id"), Some("resp-deep-001"));
        assert_eq!(result.output.tag("model"), Some("sonar-deep-research"));
    }

    #[tokio::test]
    async fn deep_research_polling_pending_then_completed() {
        let mock = SequentialMock::new(
            vec![Ok(submit_ok("req-002"))],
            vec![
                Ok(poll_pending()),
                Ok(poll_processing()),
                Ok(poll_completed("answer after waiting")),
            ],
        );
        let result = agent_with(mock)
            .run(&prompt("complex question"), &Context::now())
            .await;
        assert!(result.success);
        assert_eq!(
            result.output.body.as_text().expect("text body"),
            "answer after waiting"
        );
    }

    #[tokio::test]
    async fn deep_research_polling_failed_status_is_failure() {
        let mock = SequentialMock::new(
            vec![Ok(submit_ok("req-003"))],
            vec![Ok(poll_pending()), Ok(poll_failed())],
        );
        let result = agent_with(mock).run(&prompt("q"), &Context::now()).await;
        assert!(!result.success);
        assert_eq!(result.output.tag("failed"), Some("true"));
        let body = result.output.body.as_text().expect("text body");
        assert!(
            body.contains("research job timed out internally"),
            "got: {body}"
        );
    }

    #[tokio::test]
    async fn deep_research_polling_timeout_when_always_pending() {
        let get_responses: Vec<Result<String, HttpPostError>> =
            (0..3).map(|_| Ok(poll_pending())).collect();
        let mock = SequentialMock::new(vec![Ok(submit_ok("req-004"))], get_responses);
        let result = PerplexityDeepResearchAgent::new(
            "pplx-key",
            "https://api.perplexity.ai",
            "sonar-deep-research",
            "perplexity:sonar-deep-research",
        )
        .with_poll_interval_ms(0)
        .with_max_poll_attempts(3) // only 3 attempts, all return pending
        .with_poster(Box::new(mock))
        .run(&prompt("forever pending"), &Context::now())
        .await;
        assert!(!result.success);
        let body = result.output.body.as_text().expect("text body");
        assert!(body.contains("timed out"), "got: {body}");
        assert!(body.contains("3"), "got: {body}");
    }

    #[tokio::test]
    async fn deep_research_polling_submit_http_error_is_failure() {
        let mock = SequentialMock::new(vec![Err(HttpPostError::http(401, "unauthorized"))], vec![]);
        let result = agent_with(mock).run(&prompt("q"), &Context::now()).await;
        assert!(!result.success);
        let body = result.output.body.as_text().expect("text body");
        assert!(body.contains("401"), "got: {body}");
    }

    #[tokio::test]
    async fn deep_research_polling_submit_missing_request_id_is_failure() {
        let mock = SequentialMock::new(vec![Ok(r#"{"status": "accepted"}"#.to_string())], vec![]);
        let result = agent_with(mock).run(&prompt("q"), &Context::now()).await;
        assert!(!result.success);
        let body = result.output.body.as_text().expect("text body");
        assert!(body.contains("request_id"), "got: {body}");
    }

    #[tokio::test]
    async fn deep_research_polling_poll_http_error_is_failure() {
        let mock = SequentialMock::new(
            vec![Ok(submit_ok("req-005"))],
            vec![Err(HttpPostError::http(500, "server error"))],
        );
        let result = agent_with(mock).run(&prompt("q"), &Context::now()).await;
        assert!(!result.success);
        let body = result.output.body.as_text().expect("text body");
        assert!(body.contains("500"), "got: {body}");
    }

    #[tokio::test]
    async fn deep_research_polling_usage_is_mapped() {
        let mock = SequentialMock::new(
            vec![Ok(submit_ok("req-006"))],
            vec![Ok(poll_completed("result"))],
        );
        let result = agent_with(mock).run(&prompt("q"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.usage.input_tokens, 500);
        assert_eq!(result.usage.output_tokens, 1200);
    }

    #[tokio::test]
    async fn deep_research_polling_citations_in_pplx_meta() {
        let mock = SequentialMock::new(
            vec![Ok(submit_ok("req-007"))],
            vec![Ok(poll_completed("cited answer"))],
        );
        let result = agent_with(mock).run(&prompt("q"), &Context::now()).await;
        assert!(result.success);

        let meta: PerplexityMetadata =
            serde_json::from_str(result.output.tag("pplx_meta").expect("pplx_meta tag"))
                .expect("valid JSON");
        assert_eq!(meta.citations.len(), 2);
        assert_eq!(meta.citations[0], "https://example.com/paper1");
    }

    // ── Wire details ───────────────────────────────────────────────────────────

    #[test]
    fn deep_research_polling_submit_url_uses_v1_async_path() {
        let agent = PerplexityDeepResearchAgent::new(
            "k",
            "https://api.perplexity.ai",
            "sonar-deep-research",
            "test",
        );
        assert_eq!(
            agent.submit_url(),
            "https://api.perplexity.ai/v1/async/sonar"
        );
    }

    #[test]
    fn deep_research_polling_poll_url_includes_request_id() {
        let agent = PerplexityDeepResearchAgent::new(
            "k",
            "https://api.perplexity.ai",
            "sonar-deep-research",
            "test",
        );
        assert_eq!(
            agent.poll_url("abc-123"),
            "https://api.perplexity.ai/v1/async/sonar/abc-123"
        );
    }

    #[test]
    fn deep_research_polling_trailing_slash_normalized() {
        let agent = PerplexityDeepResearchAgent::new(
            "k",
            "https://api.perplexity.ai/",
            "sonar-deep-research",
            "test",
        );
        assert_eq!(
            agent.submit_url(),
            "https://api.perplexity.ai/v1/async/sonar"
        );
    }

    #[test]
    fn deep_research_polling_name_and_streaming() {
        let agent = PerplexityDeepResearchAgent::new(
            "k",
            "https://api.perplexity.ai",
            "sonar-deep-research",
            "my-deep-agent",
        );
        assert_eq!(agent.name(), "my-deep-agent");
        assert!(!agent.supports_streaming());
    }
}
