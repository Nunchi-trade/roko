//! `PerplexitySearchClient` — Perplexity pure-search API (no generation).
//!
//! Calls `POST /search` to retrieve structured ranked results without
//! running a generation model. Useful for finding sources, verifying URLs,
//! and enriching context at a flat $5/1K requests cost.
//!
//! Multi-query bundling: up to 5 queries per call.

#[cfg(test)]
use crate::http::HttpPostError;
use crate::http::{HttpPoster, ReqwestPoster};
use crate::perplexity::types::SearchResult;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DEFAULT_BASE_URL: &str = "https://api.perplexity.ai";
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_BATCH_SIZE: usize = 5;

/// Error type for search API calls.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// HTTP transport or status error.
    #[error("http error: {0}")]
    Http(String),
    /// Failed to serialize the request body.
    #[error("serialize error: {0}")]
    Serialize(String),
    /// Failed to parse the API response.
    #[error("malformed response: {0}")]
    Parse(String),
    /// The API returned an error object.
    #[error("api error: {0}")]
    Api(String),
    /// More than 5 queries submitted in one batch.
    #[error("too many queries: max {MAX_BATCH_SIZE} per batch, got {0}")]
    TooManyQueries(usize),
}

/// A single query to submit to the Perplexity Search API.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchQuery {
    pub query: String,
    /// Restrict results to these domains (e.g. `["arxiv.org", "github.com"]`).
    pub domain_filter: Option<Vec<String>>,
    /// Optional date range as `(after, before)` date strings (e.g. `"2024-01-01"`).
    ///
    /// Skipped during serde serialization; converted to wire fields in
    /// [`PerplexitySearchClient::query_to_wire`].
    #[serde(skip)]
    pub date_range: Option<(String, String)>,
    /// ISO 3166-1 alpha-2 country code for regional filtering (e.g. `"US"`, `"DE"`).
    pub region: Option<String>,
}

/// Structured search results for one query.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<SearchResult>,
}

/// Client for Perplexity's pure-search API (no LLM generation).
pub struct PerplexitySearchClient {
    api_key: String,
    base_url: String,
    timeout_ms: u64,
    poster: Box<dyn HttpPoster>,
}

impl PerplexitySearchClient {
    /// Construct with the production reqwest-backed HTTP poster.
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            poster: Box::new(ReqwestPoster::new()),
        }
    }

    /// Override the base URL (e.g. for testing or proxies).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Override the per-request timeout (default 30 s).
    #[must_use]
    pub const fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    #[cfg(test)]
    fn with_poster(mut self, poster: Box<dyn HttpPoster>) -> Self {
        self.poster = poster;
        self
    }

    fn endpoint(&self) -> String {
        let trimmed = self.base_url.trim_end_matches('/');
        format!("{trimmed}/search")
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

    /// Convert a public [`SearchQuery`] into the Perplexity wire format.
    fn query_to_wire(q: &SearchQuery) -> Value {
        let mut obj = json!({ "query": q.query });
        if let Some(ref domains) = q.domain_filter {
            obj["search_domain_filter"] = json!(domains);
        }
        if let Some((ref after, ref before)) = q.date_range {
            obj["search_after_date_filter"] = json!(after);
            obj["search_before_date_filter"] = json!(before);
        }
        if let Some(ref region) = q.region {
            obj["country"] = json!(region);
        }
        obj
    }

    /// Execute up to 5 search queries in a single request.
    ///
    /// Returns one [`SearchResponse`] per query in the same order as `queries`.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError`] on validation, HTTP, serialization, or API errors.
    pub async fn search_batch(
        &self,
        queries: &[SearchQuery],
    ) -> Result<Vec<SearchResponse>, SearchError> {
        if queries.len() > MAX_BATCH_SIZE {
            return Err(SearchError::TooManyQueries(queries.len()));
        }

        let wire_queries: Vec<Value> = queries.iter().map(Self::query_to_wire).collect();
        let body = json!({ "queries": wire_queries });
        let body_bytes =
            serde_json::to_vec(&body).map_err(|e| SearchError::Serialize(e.to_string()))?;

        let response_text = self
            .poster
            .post_json(
                &self.endpoint(),
                &self.headers(),
                &body_bytes,
                self.timeout_ms,
            )
            .await
            .map_err(|e| SearchError::Http(e.to_string()))?;

        let parsed: Value = serde_json::from_str(&response_text)
            .map_err(|e| SearchError::Parse(format!("malformed response json: {e}")))?;

        if let Some(err) = parsed.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown api error");
            return Err(SearchError::Api(msg.to_string()));
        }

        let results_array = parsed
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| SearchError::Parse("response missing 'results' array".to_string()))?;

        let mut out = Vec::with_capacity(results_array.len());
        for item in results_array {
            let resp: SearchResponse = serde_json::from_value(item.clone())
                .map_err(|e| SearchError::Parse(format!("failed to parse result item: {e}")))?;
            out.push(resp);
        }
        Ok(out)
    }

    /// Single query convenience method.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError`] on HTTP, serialization, or API errors.
    pub async fn search(&self, query: &str) -> Result<SearchResponse, SearchError> {
        self.search_batch(&[SearchQuery {
            query: query.to_string(),
            ..Default::default()
        }])
        .await
        .map(|mut r| r.pop().unwrap_or_default())
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
            (
                Self {
                    captured: captured.clone(),
                    response: Err(HttpPostError::transport(msg)),
                },
                captured,
            )
        }
    }

    #[async_trait::async_trait]
    impl HttpPoster for MockPoster {
        async fn post_json(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &[u8],
            _timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            *self.captured.lock().expect("lock") = Some(Captured {
                url: url.to_string(),
                headers: headers.to_vec(),
                body: body.to_vec(),
            });
            self.response.clone()
        }
    }

    fn client_with(poster: Box<dyn HttpPoster>) -> PerplexitySearchClient {
        PerplexitySearchClient::new("pplx-key").with_poster(poster)
    }

    fn canned_results(query: &str, results: &[(&str, &str, &str)]) -> String {
        let result_items: Vec<Value> = results
            .iter()
            .map(|(url, title, content)| {
                json!({
                    "url": url,
                    "title": title,
                    "content": content,
                    "date": null,
                    "last_updated": null,
                })
            })
            .collect();
        json!({
            "results": [
                {
                    "query": query,
                    "results": result_items,
                }
            ]
        })
        .to_string()
    }

    fn canned_batch(entries: &[(&str, &[(&str, &str, &str)])]) -> String {
        let result_groups: Vec<Value> = entries
            .iter()
            .map(|(q, results)| {
                let items: Vec<Value> = results
                    .iter()
                    .map(|(url, title, content)| {
                        json!({
                            "url": url,
                            "title": title,
                            "content": content,
                            "date": null,
                            "last_updated": null,
                        })
                    })
                    .collect();
                json!({ "query": q, "results": items })
            })
            .collect();
        json!({ "results": result_groups }).to_string()
    }

    // ── single search ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn perplexity_search_single_returns_results() {
        let body = canned_results(
            "rust async patterns",
            &[
                (
                    "https://example.com/1",
                    "Async Rust Patterns",
                    "Async in Rust...",
                ),
                (
                    "https://example.com/2",
                    "Tokio Guide",
                    "Tokio is a runtime...",
                ),
            ],
        );
        let (mock, _) = MockPoster::ok(body);
        let client = client_with(Box::new(mock));
        let resp = client.search("rust async patterns").await.expect("ok");
        assert_eq!(resp.query, "rust async patterns");
        assert_eq!(resp.results.len(), 2);
        assert_eq!(resp.results[0].url, "https://example.com/1");
        assert_eq!(resp.results[0].title, "Async Rust Patterns");
        assert_eq!(resp.results[1].url, "https://example.com/2");
    }

    #[tokio::test]
    async fn perplexity_search_sends_correct_endpoint_and_body() {
        let body = canned_results("test", &[]);
        let (mock, captured) = MockPoster::ok(body);
        let client = client_with(Box::new(mock));
        let _ = client.search("test").await.expect("ok");
        let c = captured.lock().expect("lock").clone().expect("captured");
        assert_eq!(c.url, "https://api.perplexity.ai/search");
        let parsed: Value = serde_json::from_slice(&c.body).expect("body is json");
        let queries = parsed["queries"].as_array().expect("queries array");
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0]["query"], "test");
    }

    #[tokio::test]
    async fn perplexity_search_sets_bearer_header() {
        let body = canned_results("x", &[]);
        let (mock, captured) = MockPoster::ok(body);
        let client = PerplexitySearchClient::new("pplx-secret").with_poster(Box::new(mock));
        let _ = client.search("x").await.expect("ok");
        let c = captured.lock().expect("lock").clone().expect("captured");
        let auth = c
            .headers
            .iter()
            .find(|(k, _)| k == "Authorization")
            .map(|(_, v)| v.clone())
            .expect("auth header");
        assert_eq!(auth, "Bearer pplx-secret");
    }

    // ── batch search ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn perplexity_search_batch_returns_one_response_per_query() {
        let body = canned_batch(&[
            (
                "rust traits",
                &[("https://a.com", "Traits", "Trait info...")],
            ),
            (
                "rust lifetimes",
                &[("https://b.com", "Lifetimes", "Lifetime info...")],
            ),
        ]);
        let (mock, _) = MockPoster::ok(body);
        let client = client_with(Box::new(mock));
        let queries = vec![
            SearchQuery {
                query: "rust traits".to_string(),
                ..Default::default()
            },
            SearchQuery {
                query: "rust lifetimes".to_string(),
                ..Default::default()
            },
        ];
        let responses = client.search_batch(&queries).await.expect("ok");
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0].query, "rust traits");
        assert_eq!(responses[0].results.len(), 1);
        assert_eq!(responses[1].query, "rust lifetimes");
    }

    #[tokio::test]
    async fn perplexity_search_batch_sends_all_queries() {
        let body = canned_batch(&[("a", &[]), ("b", &[]), ("c", &[])]);
        let (mock, captured) = MockPoster::ok(body);
        let client = client_with(Box::new(mock));
        let queries: Vec<SearchQuery> = ["a", "b", "c"]
            .iter()
            .map(|q| SearchQuery {
                query: q.to_string(),
                ..Default::default()
            })
            .collect();
        let _ = client.search_batch(&queries).await.expect("ok");
        let c = captured.lock().expect("lock").clone().expect("captured");
        let parsed: Value = serde_json::from_slice(&c.body).expect("json");
        let wire = parsed["queries"].as_array().expect("queries");
        assert_eq!(wire.len(), 3);
        assert_eq!(wire[0]["query"], "a");
        assert_eq!(wire[1]["query"], "b");
        assert_eq!(wire[2]["query"], "c");
    }

    #[tokio::test]
    async fn perplexity_search_batch_rejects_more_than_5_queries() {
        let (mock, _) = MockPoster::ok("{}");
        let client = client_with(Box::new(mock));
        let queries: Vec<SearchQuery> = (0..6)
            .map(|i| SearchQuery {
                query: format!("query {i}"),
                ..Default::default()
            })
            .collect();
        let err = client
            .search_batch(&queries)
            .await
            .expect_err("should fail");
        assert!(matches!(err, SearchError::TooManyQueries(6)));
        assert!(err.to_string().contains("6"));
    }

    // ── filters ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn perplexity_search_domain_filter_is_sent() {
        let body = canned_results("rust", &[]);
        let (mock, captured) = MockPoster::ok(body);
        let client = client_with(Box::new(mock));
        let query = SearchQuery {
            query: "rust".to_string(),
            domain_filter: Some(vec!["docs.rs".to_string(), "crates.io".to_string()]),
            ..Default::default()
        };
        let _ = client.search_batch(&[query]).await.expect("ok");
        let c = captured.lock().expect("lock").clone().expect("captured");
        let parsed: Value = serde_json::from_slice(&c.body).expect("json");
        let filter = &parsed["queries"][0]["search_domain_filter"];
        assert_eq!(filter[0], "docs.rs");
        assert_eq!(filter[1], "crates.io");
    }

    #[tokio::test]
    async fn perplexity_search_date_range_is_sent() {
        let body = canned_results("news", &[]);
        let (mock, captured) = MockPoster::ok(body);
        let client = client_with(Box::new(mock));
        let query = SearchQuery {
            query: "news".to_string(),
            date_range: Some(("2024-01-01".to_string(), "2025-01-01".to_string())),
            ..Default::default()
        };
        let _ = client.search_batch(&[query]).await.expect("ok");
        let c = captured.lock().expect("lock").clone().expect("captured");
        let parsed: Value = serde_json::from_slice(&c.body).expect("json");
        let q = &parsed["queries"][0];
        assert_eq!(q["search_after_date_filter"], "2024-01-01");
        assert_eq!(q["search_before_date_filter"], "2025-01-01");
    }

    #[tokio::test]
    async fn perplexity_search_region_is_sent() {
        let body = canned_results("local news", &[]);
        let (mock, captured) = MockPoster::ok(body);
        let client = client_with(Box::new(mock));
        let query = SearchQuery {
            query: "local news".to_string(),
            region: Some("DE".to_string()),
            ..Default::default()
        };
        let _ = client.search_batch(&[query]).await.expect("ok");
        let c = captured.lock().expect("lock").clone().expect("captured");
        let parsed: Value = serde_json::from_slice(&c.body).expect("json");
        assert_eq!(parsed["queries"][0]["country"], "DE");
    }

    // ── error handling ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn perplexity_search_http_error_returns_search_error() {
        let (mock, _) = MockPoster::err("network failure");
        let client = client_with(Box::new(mock));
        let err = client.search("x").await.expect_err("should fail");
        assert!(matches!(err, SearchError::Http(_)));
    }

    #[tokio::test]
    async fn perplexity_search_api_error_object_returns_search_error() {
        let body = json!({
            "error": { "message": "invalid api key", "type": "auth" }
        })
        .to_string();
        let (mock, _) = MockPoster::ok(body);
        let client = client_with(Box::new(mock));
        let err = client.search("x").await.expect_err("should fail");
        assert!(matches!(err, SearchError::Api(_)));
        assert!(err.to_string().contains("invalid api key"));
    }

    #[tokio::test]
    async fn perplexity_search_malformed_json_returns_parse_error() {
        let (mock, _) = MockPoster::ok("not { valid json");
        let client = client_with(Box::new(mock));
        let err = client.search("x").await.expect_err("should fail");
        assert!(matches!(err, SearchError::Parse(_)));
    }

    #[tokio::test]
    async fn perplexity_search_missing_results_field_returns_parse_error() {
        let (mock, _) = MockPoster::ok(json!({ "id": "xyz" }).to_string());
        let client = client_with(Box::new(mock));
        let err = client.search("x").await.expect_err("should fail");
        assert!(matches!(err, SearchError::Parse(_)));
        assert!(err.to_string().contains("'results'"));
    }

    // ── result fields ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn perplexity_search_result_fields_are_populated() {
        let body = json!({
            "results": [{
                "query": "attention mechanism",
                "results": [{
                    "url": "https://arxiv.org/abs/1706.03762",
                    "title": "Attention Is All You Need",
                    "content": "We propose the Transformer...",
                    "date": "2017-06-12",
                    "last_updated": null
                }]
            }]
        })
        .to_string();
        let (mock, _) = MockPoster::ok(body);
        let client = client_with(Box::new(mock));
        let resp = client.search("attention mechanism").await.expect("ok");
        assert_eq!(resp.results.len(), 1);
        let r = &resp.results[0];
        assert_eq!(r.url, "https://arxiv.org/abs/1706.03762");
        assert_eq!(r.title, "Attention Is All You Need");
        assert_eq!(r.content, "We propose the Transformer...");
        assert_eq!(r.date.as_deref(), Some("2017-06-12"));
        assert!(r.last_updated.is_none());
    }

    // ── edge cases ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn perplexity_search_empty_results_returns_empty_vec() {
        let body = canned_results("obscure topic", &[]);
        let (mock, _) = MockPoster::ok(body);
        let client = client_with(Box::new(mock));
        let resp = client.search("obscure topic").await.expect("ok");
        assert_eq!(resp.query, "obscure topic");
        assert!(resp.results.is_empty());
    }

    #[tokio::test]
    async fn perplexity_search_custom_base_url_is_used() {
        let body = canned_results("test", &[]);
        let (mock, captured) = MockPoster::ok(body);
        let client = PerplexitySearchClient::new("k")
            .with_base_url("https://proxy.example.com/v1")
            .with_poster(Box::new(mock));
        let _ = client.search("test").await.expect("ok");
        let c = captured.lock().expect("lock").clone().expect("captured");
        assert_eq!(c.url, "https://proxy.example.com/v1/search");
    }

    #[tokio::test]
    async fn perplexity_search_trailing_slash_base_url_is_normalized() {
        let body = canned_results("test", &[]);
        let (mock, captured) = MockPoster::ok(body);
        let client = PerplexitySearchClient::new("k")
            .with_base_url("https://api.perplexity.ai/")
            .with_poster(Box::new(mock));
        let _ = client.search("test").await.expect("ok");
        let c = captured.lock().expect("lock").clone().expect("captured");
        assert_eq!(c.url, "https://api.perplexity.ai/search");
    }

    #[tokio::test]
    async fn perplexity_search_max_batch_5_succeeds() {
        let entries: Vec<(&str, &[(&str, &str, &str)])> = vec![
            ("q1", &[]),
            ("q2", &[]),
            ("q3", &[]),
            ("q4", &[]),
            ("q5", &[]),
        ];
        let body = canned_batch(&entries);
        let (mock, _) = MockPoster::ok(body);
        let client = client_with(Box::new(mock));
        let queries: Vec<SearchQuery> = (1..=5)
            .map(|i| SearchQuery {
                query: format!("q{i}"),
                ..Default::default()
            })
            .collect();
        let responses = client.search_batch(&queries).await.expect("max 5 ok");
        assert_eq!(responses.len(), 5);
    }
}
