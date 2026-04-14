//! Gemini explicit context cache lifecycle client.

use std::sync::Arc;

#[cfg(test)]
use crate::http::HttpPostError;
use crate::http::{HttpPoster, ReqwestPoster};

use super::types::Content;

const DEFAULT_TIMEOUT_MS: u64 = 120_000;

/// Error returned by the Gemini cache lifecycle client.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    /// HTTP transport or status error.
    #[error("http error: {0}")]
    Http(String),
    /// Failed to serialize a request body.
    #[error("serialize error: {0}")]
    Serialize(String),
    /// Failed to parse the API response.
    #[error("malformed response: {0}")]
    Parse(String),
    /// The API returned an invalid or incomplete payload.
    #[error("api error: {0}")]
    Api(String),
}

/// Client for Gemini's explicit context caching API.
pub struct GeminiCacheClient {
    api_key: String,
    base_url: String,
    timeout_ms: u64,
    poster: Arc<dyn HttpPoster>,
}

impl GeminiCacheClient {
    /// Construct a cache client using the production reqwest-backed HTTP poster.
    #[must_use]
    pub fn new(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            poster: Arc::new(ReqwestPoster::new()),
        }
    }

    /// Override the per-request timeout.
    #[must_use]
    pub const fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    #[cfg(test)]
    fn with_http_poster(mut self, poster: Arc<dyn HttpPoster>) -> Self {
        self.poster = poster;
        self
    }

    fn headers(&self) -> Vec<(String, String)> {
        vec![
            ("x-goog-api-key".to_string(), self.api_key.clone()),
            ("content-type".to_string(), "application/json".to_string()),
        ]
    }

    fn create_endpoint(&self) -> String {
        format!(
            "{}/v1beta/cachedContents",
            self.base_url.trim_end_matches('/')
        )
    }

    fn delete_endpoint(&self, cache_id: &str) -> String {
        let cache_name = cache_id.trim_start_matches('/');
        let cache_name = if cache_name.starts_with("cachedContents/") {
            cache_name.to_string()
        } else {
            format!("cachedContents/{cache_name}")
        };
        format!(
            "{}/v1beta/{cache_name}",
            self.base_url.trim_end_matches('/')
        )
    }

    /// Create a cache entry for reusable context (e.g., entire crate source).
    pub async fn create_cache(
        &self,
        model: &str,
        contents: &[Content],
        ttl_seconds: u64,
    ) -> Result<String, CacheError> {
        let body = serde_json::json!({
            "model": format!("models/{model}"),
            "contents": contents,
            "ttl": format!("{ttl_seconds}s"),
        });
        let body_bytes =
            serde_json::to_vec(&body).map_err(|error| CacheError::Serialize(error.to_string()))?;

        let response_text = self
            .poster
            .post_json(
                &self.create_endpoint(),
                &self.headers(),
                &body_bytes,
                self.timeout_ms,
            )
            .await
            .map_err(|error| CacheError::Http(error.to_string()))?;

        let response: CachedContentResponse = serde_json::from_str(&response_text)
            .map_err(|error| CacheError::Parse(format!("malformed response json: {error}")))?;

        response
            .name
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| CacheError::Api("response missing cache name".to_string()))
    }

    /// Delete a cache entry.
    pub async fn delete_cache(&self, cache_id: &str) -> Result<(), CacheError> {
        let response_text = self
            .poster
            .delete_json(
                &self.delete_endpoint(cache_id),
                &self.headers(),
                self.timeout_ms,
            )
            .await
            .map_err(|error| CacheError::Http(error.to_string()))?;

        serde_json::from_str::<serde_json::Value>(&response_text)
            .map_err(|error| CacheError::Parse(format!("malformed response json: {error}")))?;

        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
struct CachedContentResponse {
    name: Option<String>,
}

#[cfg(test)]
#[allow(clippy::disallowed_types)] // tests use std::sync::Mutex for simplicity
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Clone, Debug, Default)]
    struct Captured {
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        timeout_ms: u64,
    }

    #[derive(Debug)]
    struct MockPoster {
        captured: Arc<Mutex<Captured>>,
        post_response: Result<String, HttpPostError>,
        delete_response: Result<String, HttpPostError>,
    }

    impl MockPoster {
        fn with_responses(
            captured: Arc<Mutex<Captured>>,
            post_response: Result<serde_json::Value, HttpPostError>,
            delete_response: Result<serde_json::Value, HttpPostError>,
        ) -> Self {
            Self {
                captured,
                post_response: post_response.map(|value| value.to_string()),
                delete_response: delete_response.map(|value| value.to_string()),
            }
        }
    }

    #[async_trait::async_trait]
    impl HttpPoster for MockPoster {
        async fn post_json(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &[u8],
            timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            let mut captured = self.captured.lock().expect("capture lock");
            captured.method = "POST".to_string();
            captured.url = url.to_string();
            captured.headers = headers.to_vec();
            captured.body = body.to_vec();
            captured.timeout_ms = timeout_ms;
            self.post_response.clone()
        }

        async fn delete_json(
            &self,
            url: &str,
            headers: &[(String, String)],
            timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            let mut captured = self.captured.lock().expect("capture lock");
            captured.method = "DELETE".to_string();
            captured.url = url.to_string();
            captured.headers = headers.to_vec();
            captured.body.clear();
            captured.timeout_ms = timeout_ms;
            self.delete_response.clone()
        }
    }

    fn sample_contents() -> Vec<Content> {
        vec![Content {
            role: "user".to_string(),
            parts: vec![super::super::types::Part::Text {
                text: "pub fn cache_me() {}".to_string(),
            }],
        }]
    }

    #[tokio::test]
    async fn gemini_cache_create_cache_returns_cache_name() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let poster = Arc::new(MockPoster::with_responses(
            Arc::clone(&captured),
            Ok(serde_json::json!({
                "name": "cachedContents/cache-123",
                "model": "models/gemini-2.5-pro"
            })),
            Ok(serde_json::json!({})),
        ));
        let client =
            GeminiCacheClient::new("test-key", "https://generativelanguage.googleapis.com")
                .with_timeout_ms(4_321)
                .with_http_poster(poster);

        let cache_name = client
            .create_cache("gemini-2.5-pro", &sample_contents(), 300)
            .await
            .expect("create cache");

        assert_eq!(cache_name, "cachedContents/cache-123");

        let captured = captured.lock().expect("capture lock");
        assert_eq!(captured.method, "POST");
        assert_eq!(
            captured.url,
            "https://generativelanguage.googleapis.com/v1beta/cachedContents"
        );
        assert_eq!(captured.timeout_ms, 4_321);
        assert!(
            captured
                .headers
                .iter()
                .any(|(key, value)| key == "x-goog-api-key" && value == "test-key")
        );

        let body: serde_json::Value =
            serde_json::from_slice(&captured.body).expect("request body json");
        assert_eq!(body["model"], "models/gemini-2.5-pro");
        assert_eq!(body["ttl"], "300s");
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(
            body["contents"][0]["parts"][0]["text"],
            "pub fn cache_me() {}"
        );
    }

    #[tokio::test]
    async fn gemini_cache_delete_cache_accepts_full_or_short_ids() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let poster = Arc::new(MockPoster::with_responses(
            Arc::clone(&captured),
            Ok(serde_json::json!({
                "name": "cachedContents/cache-123"
            })),
            Ok(serde_json::json!({})),
        ));
        let client =
            GeminiCacheClient::new("test-key", "https://generativelanguage.googleapis.com")
                .with_http_poster(poster);

        client
            .delete_cache("cache-123")
            .await
            .expect("delete cache");

        let captured = captured.lock().expect("capture lock");
        assert_eq!(captured.method, "DELETE");
        assert_eq!(
            captured.url,
            "https://generativelanguage.googleapis.com/v1beta/cachedContents/cache-123"
        );
        assert!(captured.body.is_empty());
    }

    #[test]
    fn gemini_cache_generate_content_request_serializes_cached_content_reference() {
        let request = super::super::types::GenerateContentRequest {
            contents: sample_contents(),
            system_instruction: None,
            tools: None,
            tool_config: None,
            generation_config: None,
            safety_settings: None,
            cached_content: Some("cachedContents/cache-123".to_string()),
        };

        let body = serde_json::to_value(&request).expect("serialize request");
        assert_eq!(body["cachedContent"], "cachedContents/cache-123");
        assert_eq!(
            body["contents"][0]["parts"][0]["text"],
            "pub fn cache_me() {}"
        );
    }
}
