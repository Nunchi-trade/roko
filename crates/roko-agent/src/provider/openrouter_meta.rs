//! OpenRouter model metadata helper.
//!
//! This fetches the OpenRouter model catalog and converts a single model entry
//! into Roko's `ModelProfile` shape. The helper is intentionally optional and
//! read-only: callers can use it to seed config defaults, but it does not modify
//! the config on its own.

use roko_core::config::schema::ModelProfile;
use roko_core::error::{Result, RokoError};
use serde::Deserialize;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
const HTTP_REFERER: &str = "https://github.com/nunchi/roko";
const X_TITLE: &str = "roko-agent";

/// Fetch a model profile from OpenRouter's catalog.
///
/// The helper queries the OpenRouter models API, finds `model_id` in the
/// returned catalog, and maps the published metadata into a `ModelProfile`.
pub async fn fetch_model_metadata(api_key: &str, model_id: &str) -> Result<ModelProfile> {
    fetch_model_metadata_from(DEFAULT_BASE_URL, api_key, model_id).await
}

async fn fetch_model_metadata_from(
    base_url: &str,
    api_key: &str,
    model_id: &str,
) -> Result<ModelProfile> {
    if api_key.trim().is_empty() {
        return Err(RokoError::Invalid(
            "OpenRouter API key is required to fetch model metadata".to_string(),
        ));
    }

    let endpoint = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|err| RokoError::Invalid(format!("failed to build HTTP client: {err}")))?;
    let response = client
        .get(endpoint)
        .bearer_auth(api_key)
        .header("HTTP-Referer", HTTP_REFERER)
        .header("X-Title", X_TITLE)
        .send()
        .await
        .map_err(|err| {
            RokoError::Invalid(format!("failed to fetch OpenRouter metadata: {err}"))
        })?;

    let status = response.status();
    let body = response.text().await.map_err(|err| {
        RokoError::Invalid(format!("failed to read OpenRouter metadata response: {err}"))
    })?;
    if !status.is_success() {
        return Err(RokoError::Invalid(format!(
            "OpenRouter metadata request failed with {status}: {body}"
        )));
    }

    let payload: OpenRouterModelsResponse = serde_json::from_str(&body)?;
    let model = payload
        .into_models()
        .into_iter()
        .find(|candidate| candidate.matches(model_id))
        .ok_or_else(|| {
            RokoError::Invalid(format!(
                "OpenRouter model metadata not found for {model_id}"
            ))
        })?;

    Ok(model.into_profile())
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenRouterModelsResponse {
    Wrapped { data: Vec<OpenRouterModel> },
    Direct(OpenRouterModel),
}

impl OpenRouterModelsResponse {
    fn into_models(self) -> Vec<OpenRouterModel> {
        match self {
            Self::Wrapped { data } => data,
            Self::Direct(model) => vec![model],
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OpenRouterModel {
    #[serde(default)]
    id: String,
    #[serde(default)]
    canonical_slug: Option<String>,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    architecture: Option<Architecture>,
    #[serde(default)]
    pricing: Option<Pricing>,
    #[serde(default)]
    top_provider: Option<TopProvider>,
    #[serde(default)]
    supported_parameters: Vec<String>,
}

impl OpenRouterModel {
    fn matches(&self, model_id: &str) -> bool {
        self.id == model_id || self.canonical_slug.as_deref() == Some(model_id)
    }

    fn into_profile(self) -> ModelProfile {
        let context_window = self
            .context_length
            .or_else(|| self.top_provider.as_ref().and_then(|provider| provider.context_length))
            .unwrap_or(128_000);
        let max_output = self
            .top_provider
            .as_ref()
            .and_then(|provider| provider.max_completion_tokens);
        let supported_parameters = self.supported_parameters;
        let architecture = self.architecture;
        let pricing = self.pricing;

        ModelProfile {
            provider: "openrouter".to_string(),
            slug: self.canonical_slug.unwrap_or(self.id),
            context_window,
            max_output,
            supports_tools: supports_parameters(&supported_parameters, &["tools", "tool_choice"]),
            supports_thinking: supports_parameters(
                &supported_parameters,
                &["reasoning", "include_reasoning", "thinking"],
            ),
            supports_vision: architecture
                .as_ref()
                .is_some_and(|arch| arch.input_modalities.iter().any(|item| item == "image")),
            supports_web_search: supports_parameters(&supported_parameters, &["web_search"]),
            supports_mcp_tools: supports_parameters(&supported_parameters, &["mcp", "mcp_tools"]),
            supports_partial: supports_parameters(
                &supported_parameters,
                &["partial", "continuation"],
            ),
            provider_routing: None,
            tool_format: "openai_json".to_string(),
            cost_input_per_m: pricing
                .as_ref()
                .and_then(|value| value.prompt.as_ref())
                .and_then(|value| value.per_million()),
            cost_output_per_m: pricing
                .as_ref()
                .and_then(|value| value.completion.as_ref())
                .and_then(|value| value.per_million()),
            cost_cache_read_per_m: pricing
                .as_ref()
                .and_then(|value| value.input_cache_read.as_ref())
                .and_then(|value| value.per_million()),
            cost_cache_write_per_m: pricing
                .as_ref()
                .and_then(|value| value.input_cache_write.as_ref())
                .and_then(|value| value.per_million()),
            max_tools: None,
            tokenizer_ratio: None,
        }
    }
}

fn supports_parameters(parameters: &[String], wanted: &[&str]) -> bool {
    parameters
        .iter()
        .any(|parameter| wanted.iter().any(|wanted| parameter == wanted))
}

#[derive(Debug, Clone, Deserialize)]
struct Architecture {
    #[serde(default)]
    input_modalities: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Pricing {
    #[serde(default)]
    prompt: Option<Decimal>,
    #[serde(default)]
    completion: Option<Decimal>,
    #[serde(default)]
    input_cache_read: Option<Decimal>,
    #[serde(default)]
    input_cache_write: Option<Decimal>,
}

#[derive(Debug, Clone, Deserialize)]
struct TopProvider {
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    max_completion_tokens: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Decimal {
    Number(f64),
    String(String),
}

impl Decimal {
    fn per_million(&self) -> Option<f64> {
        let base = match self {
            Self::Number(value) => *value,
            Self::String(value) => value.parse().ok()?,
        };
        Some(base * 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    fn spawn_models_server(
        response: String,
    ) -> (String, Arc<Mutex<Option<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("server addr");
        let captured = Arc::new(Mutex::new(None));
        let captured_request = Arc::clone(&captured);

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set read timeout");

            let mut buf = Vec::new();
            let mut header_end = None;
            let mut content_length = None;

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
                    content_length = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    });
                }

                if let Some(header_end) = header_end {
                    match content_length {
                        Some(content_length) if buf.len() >= header_end + content_length => {
                            break;
                        }
                        None => break,
                        _ => {}
                    }
                }
            }

            let header_end = header_end.expect("request headers");
            let content_length = content_length.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..header_end + content_length]).to_string();
            *captured_request.lock().expect("capture lock") = Some(request);

            let response_bytes = response.as_bytes();
            let wire = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_bytes.len(),
                response
            );
            stream.write_all(wire.as_bytes()).expect("write response");
            stream.flush().expect("flush response");
        });

        (format!("http://{}", addr), captured, handle)
    }

    #[tokio::test]
    async fn openrouter_meta_fetch() {
        let response = serde_json::json!({
            "data": [{
                "id": "z-ai/glm-5.1",
                "canonical_slug": "z-ai/glm-5.1",
                "context_length": 200000,
                "architecture": {
                    "input_modalities": ["text"],
                    "output_modalities": ["text"],
                    "tokenizer": "o200k_base",
                    "instruct_type": "openai"
                },
                "pricing": {
                    "prompt": "0.00000126",
                    "completion": "0.00000396",
                    "request": "0",
                    "web_search": "0",
                    "internal_reasoning": "0",
                    "input_cache_read": "0",
                    "input_cache_write": "0"
                },
                "top_provider": {
                    "context_length": 200000,
                    "max_completion_tokens": 131072,
                    "is_moderated": false
                },
                "supported_parameters": [
                    "tools",
                    "tool_choice",
                    "max_tokens",
                    "temperature",
                    "top_p",
                    "reasoning",
                    "include_reasoning",
                    "response_format"
                ]
            }]
        })
        .to_string();
        let (base_url, captured, handle) = spawn_models_server(response);

        let profile = fetch_model_metadata_from(&format!("{base_url}/api/v1"), "test-key", "z-ai/glm-5.1")
            .await
            .expect("fetch model metadata");

        assert_eq!(profile.provider, "openrouter");
        assert_eq!(profile.slug, "z-ai/glm-5.1");
        assert_eq!(profile.context_window, 200_000);
        assert_eq!(profile.max_output, Some(131_072));
        assert!(profile.supports_tools);
        assert!(profile.supports_thinking);
        assert!(!profile.supports_vision);
        assert_eq!(profile.tool_format, "openai_json");
        assert!(
            profile
                .cost_input_per_m
                .is_some_and(|value| (value - 1.26).abs() < 1e-9)
        );
        assert!(
            profile
                .cost_output_per_m
                .is_some_and(|value| (value - 3.96).abs() < 1e-9)
        );

        let request = captured
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured request");
        assert!(request.starts_with("GET /api/v1/models HTTP/1.1"));
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("authorization: bearer test-key"));
        assert!(lower.contains("http-referer: https://github.com/nunchi/roko"));
        assert!(lower.contains("x-title: roko-agent"));

        handle.join().expect("server thread");
    }
}
