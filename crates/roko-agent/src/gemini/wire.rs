//! Shared HTTP plumbing for Gemini native `generateContent` calls.

use crate::http::{HttpPostError, HttpPoster};

use super::types::GenerateContentRequest;

#[must_use]
pub(crate) fn generate_content_endpoint(base_url: &str, model_slug: &str) -> String {
    format!(
        "{}/v1beta/models/{}:generateContent",
        base_url.trim_end_matches('/'),
        model_slug
    )
}

#[must_use]
pub(crate) fn generate_content_headers(api_key: &str) -> Vec<(String, String)> {
    vec![
        ("x-goog-api-key".to_string(), api_key.to_string()),
        ("content-type".to_string(), "application/json".to_string()),
    ]
}

pub(crate) fn serialize_generate_content_request(
    request: &GenerateContentRequest,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(request)
}

pub(crate) async fn send_generate_content_request(
    poster: &dyn HttpPoster,
    endpoint: &str,
    headers: &[(String, String)],
    body: &[u8],
    timeout_ms: u64,
) -> Result<String, HttpPostError> {
    poster.post_json(endpoint, headers, body, timeout_ms).await
}
