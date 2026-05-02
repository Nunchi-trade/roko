//! `PerplexityEmbedAgent` — Perplexity embedding API.
//!
//! Calls `POST /v1/embeddings` to generate float-vector embeddings for a batch
//! of input texts, and `POST /v1/contextualizedembeddings` for context-aware
//! embeddings.

#[cfg(test)]
use crate::http::HttpPostError;
use crate::http::{HttpPoster, ReqwestPoster};
use serde_json::{Value, json};

/// Default embedding model.
const DEFAULT_MODEL: &str = "pplx-embed-v1-4b";
/// Default per-request timeout in milliseconds (30 s is sufficient for embeddings).
const DEFAULT_TIMEOUT_MS: u64 = roko_core::defaults::DEFAULT_EMBED_TIMEOUT_MS;

/// Error type for embedding calls.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
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
}

/// Agent wrapper for Perplexity's embedding API.
pub struct PerplexityEmbedAgent {
    api_key: String,
    base_url: String,
    model_slug: String,
    timeout_ms: u64,
    poster: Box<dyn HttpPoster>,
}

impl PerplexityEmbedAgent {
    /// Construct with the production reqwest-backed HTTP poster.
    #[must_use]
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model_slug: impl Into<String>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
            model_slug: model_slug.into(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            poster: Box::new(ReqwestPoster::new()),
        }
    }

    /// Override the per-request timeout.
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

    fn headers(&self) -> Vec<(String, String)> {
        vec![
            (
                "Authorization".to_string(),
                format!("Bearer {}", self.api_key),
            ),
            ("Content-Type".to_string(), "application/json".to_string()),
        ]
    }

    fn embeddings_endpoint(&self) -> String {
        let trimmed = self.base_url.trim_end_matches('/');
        format!("{trimmed}/embeddings")
    }

    fn contextualized_endpoint(&self) -> String {
        let trimmed = self.base_url.trim_end_matches('/');
        format!("{trimmed}/contextualizedembeddings")
    }

    async fn post_and_parse(&self, url: &str, body: Value) -> Result<Vec<Vec<f32>>, EmbedError> {
        let body_bytes =
            serde_json::to_vec(&body).map_err(|e| EmbedError::Serialize(e.to_string()))?;

        let response_text = self
            .poster
            .post_json(url, &self.headers(), &body_bytes, self.timeout_ms)
            .await
            .map_err(|e| EmbedError::Http(e.to_string()))?;

        let parsed: Value = serde_json::from_str(&response_text)
            .map_err(|e| EmbedError::Parse(format!("malformed response json: {e}")))?;

        if let Some(err) = parsed.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown api error");
            return Err(EmbedError::Api(msg.to_string()));
        }

        let data = parsed
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| EmbedError::Parse("response missing 'data' array".to_string()))?;

        let mut result = Vec::with_capacity(data.len());
        for item in data {
            let embedding = item
                .get("embedding")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    EmbedError::Parse("data item missing 'embedding' array".to_string())
                })?;
            let floats: Vec<f32> = embedding
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            result.push(floats);
        }
        Ok(result)
    }

    /// Generate embeddings for a batch of texts.
    ///
    /// # Errors
    ///
    /// Returns [`EmbedError`] on HTTP, serialization, or API errors.
    pub async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let body = json!({
            "model": self.model_slug,
            "input": texts,
        });
        self.post_and_parse(&self.embeddings_endpoint(), body).await
    }

    /// Generate contextualized embeddings.
    ///
    /// # Errors
    ///
    /// Returns [`EmbedError`] on HTTP, serialization, or API errors.
    pub async fn embed_contextualized(
        &self,
        texts: &[&str],
        context: &str,
    ) -> Result<Vec<Vec<f32>>, EmbedError> {
        let body = json!({
            "model": self.model_slug,
            "input": texts,
            "context": context,
        });
        self.post_and_parse(&self.contextualized_endpoint(), body)
            .await
    }
}

/// Construct a `PerplexityEmbedAgent` using the default model and Perplexity base URL.
impl Default for PerplexityEmbedAgent {
    fn default() -> Self {
        Self::new("", "https://api.perplexity.ai/v1", DEFAULT_MODEL)
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
            _headers: &[(String, String)],
            body: &[u8],
            _timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            *self.captured.lock().expect("lock") = Some(Captured {
                url: url.to_string(),
                body: body.to_vec(),
            });
            self.response.clone()
        }
    }

    fn agent_with(poster: Box<dyn HttpPoster>) -> PerplexityEmbedAgent {
        PerplexityEmbedAgent::new(
            "pplx-key",
            "https://api.perplexity.ai/v1",
            "pplx-embed-v1-4b",
        )
        .with_poster(poster)
    }

    fn canned_embeddings(vecs: &[Vec<f32>]) -> String {
        let data: Vec<serde_json::Value> = vecs
            .iter()
            .enumerate()
            .map(|(i, v)| {
                json!({
                    "object": "embedding",
                    "index": i,
                    "embedding": v,
                })
            })
            .collect();
        json!({
            "object": "list",
            "data": data,
            "model": "pplx-embed-v1-4b",
            "usage": { "prompt_tokens": 5, "total_tokens": 5 }
        })
        .to_string()
    }

    // ── embed ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn perplexity_embed_returns_float_vectors() {
        let expected = vec![vec![0.1_f32, 0.2, 0.3], vec![0.4_f32, 0.5, 0.6]];
        let (mock, _) = MockPoster::ok(canned_embeddings(&expected));
        let agent = agent_with(Box::new(mock));
        let result = agent.embed(&["hello", "world"]).await.expect("embed ok");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].len(), 3);
        assert!((result[0][0] - 0.1_f32).abs() < 1e-5);
        assert!((result[1][2] - 0.6_f32).abs() < 1e-5);
    }

    #[tokio::test]
    async fn perplexity_embed_sends_correct_endpoint_and_body() {
        let (mock, captured) = MockPoster::ok(canned_embeddings(&[vec![0.0]]));
        let agent = agent_with(Box::new(mock));
        let _ = agent.embed(&["test text"]).await.expect("ok");
        let c = captured.lock().expect("lock").clone().expect("captured");
        assert_eq!(c.url, "https://api.perplexity.ai/v1/embeddings");
        let body: Value = serde_json::from_slice(&c.body).expect("body is json");
        assert_eq!(body["model"], "pplx-embed-v1-4b");
        assert_eq!(body["input"][0], "test text");
    }

    #[tokio::test]
    async fn perplexity_embed_single_text_returns_one_vector() {
        let (mock, _) = MockPoster::ok(canned_embeddings(&[vec![1.0, 2.0]]));
        let agent = agent_with(Box::new(mock));
        let result = agent.embed(&["single"]).await.expect("ok");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 2);
    }

    #[tokio::test]
    async fn perplexity_embed_http_error_returns_embed_error() {
        let (mock, _) = MockPoster::err("transport failure");
        let agent = agent_with(Box::new(mock));
        let err = agent.embed(&["x"]).await.expect_err("should fail");
        assert!(matches!(err, EmbedError::Http(_)));
    }

    #[tokio::test]
    async fn perplexity_embed_api_error_object_returns_embed_error() {
        let body = json!({
            "error": { "message": "invalid model", "type": "not_found" }
        })
        .to_string();
        let (mock, _) = MockPoster::ok(body);
        let agent = agent_with(Box::new(mock));
        let err = agent.embed(&["x"]).await.expect_err("should fail");
        assert!(matches!(err, EmbedError::Api(_)));
        assert!(err.to_string().contains("invalid model"));
    }

    #[tokio::test]
    async fn perplexity_embed_malformed_json_returns_parse_error() {
        let (mock, _) = MockPoster::ok("not { valid json");
        let agent = agent_with(Box::new(mock));
        let err = agent.embed(&["x"]).await.expect_err("should fail");
        assert!(matches!(err, EmbedError::Parse(_)));
    }

    // ── embed_contextualized ──────────────────────────────────────────────────

    #[tokio::test]
    async fn perplexity_embed_contextualized_returns_float_vectors() {
        let expected = vec![vec![0.9_f32, 0.8, 0.7]];
        let (mock, _) = MockPoster::ok(canned_embeddings(&expected));
        let agent = agent_with(Box::new(mock));
        let result = agent
            .embed_contextualized(&["query"], "background context")
            .await
            .expect("ok");
        assert_eq!(result.len(), 1);
        assert!((result[0][0] - 0.9_f32).abs() < 1e-5);
    }

    #[tokio::test]
    async fn perplexity_embed_contextualized_sends_correct_endpoint_and_body() {
        let (mock, captured) = MockPoster::ok(canned_embeddings(&[vec![0.0]]));
        let agent = agent_with(Box::new(mock));
        let _ = agent
            .embed_contextualized(&["query text"], "some context")
            .await
            .expect("ok");
        let c = captured.lock().expect("lock").clone().expect("captured");
        assert_eq!(
            c.url,
            "https://api.perplexity.ai/v1/contextualizedembeddings"
        );
        let body: Value = serde_json::from_slice(&c.body).expect("body is json");
        assert_eq!(body["model"], "pplx-embed-v1-4b");
        assert_eq!(body["input"][0], "query text");
        assert_eq!(body["context"], "some context");
    }
}
