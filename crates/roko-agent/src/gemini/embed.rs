//! `GeminiEmbedAgent` — Gemini embedding API.
//!
//! Uses Gemini's OpenAI-compatible embeddings surface to generate float-vector
//! embeddings for batches of input texts.

use std::time::Instant;

#[cfg(test)]
use crate::http::HttpPostError;
use crate::http::{HttpPoster, ReqwestPoster};
use crate::{Agent, AgentResult, Usage};
use async_trait::async_trait;
use roko_core::{Body, Context, Engram, Kind, Provenance};
use serde_json::{Value, json};

/// Default Gemini embedding model.
const DEFAULT_MODEL: &str = "gemini-embedding-2-preview";
/// Default per-request timeout in milliseconds.
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

fn compat_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{trimmed}/v1beta/openai")
}

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

/// Agent wrapper for Gemini's embeddings API.
pub struct GeminiEmbedAgent {
    api_key: String,
    base_url: String,
    model_slug: String,
    timeout_ms: u64,
    name: String,
    poster: Box<dyn HttpPoster>,
}

impl GeminiEmbedAgent {
    /// Construct with the production reqwest-backed HTTP poster.
    #[must_use]
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model_slug: impl Into<String>,
    ) -> Self {
        let model_slug = model_slug.into();
        Self {
            api_key: api_key.into(),
            base_url: compat_base_url(&base_url.into()),
            name: format!("gemini-embed:{model_slug}"),
            model_slug,
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

    /// Override the display name used for logs and tests.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
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
        format!("{}/v1/embeddings", self.base_url.trim_end_matches('/'))
    }

    async fn post_and_parse(&self, body: Value) -> Result<Vec<Vec<f32>>, EmbedError> {
        let body_bytes =
            serde_json::to_vec(&body).map_err(|e| EmbedError::Serialize(e.to_string()))?;

        let response_text = self
            .poster
            .post_json(
                &self.embeddings_endpoint(),
                &self.headers(),
                &body_bytes,
                self.timeout_ms,
            )
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
            let floats = embedding
                .iter()
                .map(|value| {
                    value.as_f64().map(|float| float as f32).ok_or_else(|| {
                        EmbedError::Parse("embedding array contains non-numeric value".to_string())
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
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
        self.post_and_parse(body).await
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

impl Default for GeminiEmbedAgent {
    fn default() -> Self {
        Self::new(
            "",
            "https://generativelanguage.googleapis.com",
            DEFAULT_MODEL,
        )
    }
}

#[async_trait]
impl Agent for GeminiEmbedAgent {
    async fn run(&self, input: &Engram, _ctx: &Context) -> AgentResult {
        let started = Instant::now();
        let prompt_text = match input.body.as_text() {
            Ok(text) => text.to_string(),
            Err(_) => match serde_json::to_string(&input.body) {
                Ok(text) => text,
                Err(error) => {
                    return self.failure(
                        input,
                        format!("input body not readable as text or json: {error}"),
                        &started,
                    );
                }
            },
        };

        let embeddings = match self.embed(&[prompt_text.as_str()]).await {
            Ok(embeddings) => embeddings,
            Err(error) => {
                return self.failure(input, format!("embed failed: {error}"), &started);
            }
        };

        let Some(first) = embeddings.first() else {
            return self.failure(
                input,
                "embedding response contained no vectors".to_string(),
                &started,
            );
        };

        let content = match serde_json::to_string(first) {
            Ok(content) => content,
            Err(error) => {
                return self.failure(
                    input,
                    format!("embedding serialize failed: {error}"),
                    &started,
                );
            }
        };

        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let output = input
            .derive(Kind::AgentOutput, Body::text(&content))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("model", &self.model_slug)
            .build();

        AgentResult::ok(output).with_usage(Usage {
            wall_ms,
            ..Default::default()
        })
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn backend_id(&self) -> &'static str {
        "gemini"
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
        timeout_ms: u64,
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

    #[async_trait]
    impl HttpPoster for MockPoster {
        async fn post_json(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &[u8],
            timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            *self.captured.lock().expect("lock") = Some(Captured {
                url: url.to_string(),
                headers: headers.to_vec(),
                body: body.to_vec(),
                timeout_ms,
            });
            self.response.clone()
        }
    }

    fn agent_with(poster: Box<dyn HttpPoster>) -> GeminiEmbedAgent {
        GeminiEmbedAgent::new(
            "gemini-key",
            "https://generativelanguage.googleapis.com",
            "gemini-embedding-2-preview",
        )
        .with_poster(poster)
    }

    fn canned_embeddings(vecs: &[Vec<f32>]) -> String {
        let data: Vec<serde_json::Value> = vecs
            .iter()
            .enumerate()
            .map(|(index, embedding)| {
                json!({
                    "object": "embedding",
                    "index": index,
                    "embedding": embedding,
                })
            })
            .collect();
        json!({
            "object": "list",
            "data": data,
            "model": "gemini-embedding-2-preview",
            "usage": {
                "prompt_tokens": 4,
                "total_tokens": 4
            }
        })
        .to_string()
    }

    #[tokio::test]
    async fn gemini_embed_returns_float_vectors() {
        let expected = vec![vec![0.125, 0.25, 0.5], vec![1.0, 2.0, 3.0]];
        let (mock, _) = MockPoster::ok(canned_embeddings(&expected));
        let agent = agent_with(Box::new(mock));

        let result = agent.embed(&["hello", "world"]).await.expect("embed ok");

        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn gemini_embed_posts_to_openai_compat_embeddings_endpoint() {
        let (mock, captured) = MockPoster::ok(canned_embeddings(&[vec![0.0]]));
        let agent = agent_with(Box::new(mock)).with_timeout_ms(12_345);

        let _ = agent.embed(&["test text"]).await.expect("embed ok");

        let captured = captured.lock().expect("lock").clone().expect("captured");
        assert_eq!(
            captured.url,
            "https://generativelanguage.googleapis.com/v1beta/openai/v1/embeddings"
        );
        assert_eq!(captured.timeout_ms, 12_345);
        assert!(
            captured
                .headers
                .contains(&("Authorization".to_string(), "Bearer gemini-key".to_string()))
        );
        let body: Value = serde_json::from_slice(&captured.body).expect("request body json");
        assert_eq!(body["model"], "gemini-embedding-2-preview");
        assert_eq!(body["input"][0], "test text");
    }

    #[tokio::test]
    async fn gemini_embed_surfaces_transport_errors() {
        let (mock, _) = MockPoster::err("timeout");
        let agent = agent_with(Box::new(mock));

        let err = agent.embed(&["x"]).await.expect_err("should fail");

        assert!(matches!(err, EmbedError::Http(message) if message.contains("timeout")));
    }

    #[tokio::test]
    async fn gemini_embed_agent_run_serializes_first_vector() {
        let (mock, _) = MockPoster::ok(canned_embeddings(&[vec![0.1, 0.2, 0.3]]));
        let agent = agent_with(Box::new(mock));
        let input = Engram::builder(Kind::Prompt)
            .body(Body::text("index this"))
            .build();

        let result = agent.run(&input, &Context::now()).await;

        assert!(result.success);
        assert_eq!(result.output.body.as_text().ok(), Some("[0.1,0.2,0.3]"));
    }
}
