//! LLM provider abstraction for demo scenarios.
//!
//! The scripted spine owns chain interaction; providers only return structured
//! JSON fragments for scenario slots. `StubLlm` remains the deterministic
//! default so tests and CI stay reproducible.

use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use async_trait::async_trait;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

/// Request to the LLM. `slot` names the field the scenario wants filled.
#[derive(Clone, Debug)]
pub struct LlmRequest {
    /// The slot being asked about (e.g. "bid_amount", "approve", "insight_content").
    pub slot: String,
    /// Free-form context the scenario wants the LLM to reason over.
    pub context: serde_json::Value,
}

/// A structured response from the LLM.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Produce a JSON value for the requested slot.
    async fn fill(&self, req: LlmRequest) -> anyhow::Result<serde_json::Value>;

    /// Human-readable backend label for logs/events.
    fn label(&self) -> &str {
        "unknown"
    }
}

/// Deterministic stub: returns canned values per slot based on a seed counter.
pub struct StubLlm {
    counter: AtomicU64,
}

impl StubLlm {
    /// Fresh stub.
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
        }
    }

    fn next(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }
}

impl Default for StubLlm {
    fn default() -> Self {
        Self::new()
    }
}

/// Shape: `{ bid: bool, submission_content: Option<String> }`
#[derive(Serialize, Deserialize)]
pub struct BidDecision {
    /// Worker decision.
    pub bid: bool,
    /// Free-text submission content.
    pub submission_content: Option<String>,
}

/// Shape: `{ approve: bool, reason: String }`
#[derive(Serialize, Deserialize)]
pub struct VoteDecision {
    /// Approve flag.
    pub approve: bool,
    /// Human-readable reason.
    pub reason: String,
}

/// Anthropic-backed provider.
pub struct ClaudeApiProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl ClaudeApiProvider {
    /// Construct from environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error if `ANTHROPIC_API_KEY` is missing.
    pub fn from_env() -> anyhow::Result<Self> {
        let api_key = env::var("ANTHROPIC_API_KEY")
            .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY is required for claude backend"))?;
        let model =
            env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| "claude-sonnet-4-20250514".into());
        Ok(Self::new(api_key, model))
    }

    /// Construct directly.
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model,
        }
    }
}

/// Ollama-backed provider.
pub struct OllamaProvider {
    client: reqwest::Client,
    model: String,
    base_url: String,
}

impl OllamaProvider {
    /// Construct from environment variables.
    pub fn from_env() -> Self {
        Self {
            client: reqwest::Client::new(),
            model: env::var("OLLAMA_MODEL").unwrap_or_else(|_| "gemma3:7b".into()),
            base_url: env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into()),
        }
    }
}

/// Round-robin provider over multiple backends.
pub struct MultiProvider {
    providers: Vec<Arc<dyn LlmProvider>>,
    counter: AtomicUsize,
}

impl MultiProvider {
    /// Create a new round-robin provider set.
    ///
    /// # Errors
    ///
    /// Returns an error if `providers` is empty.
    pub fn new(providers: Vec<Arc<dyn LlmProvider>>) -> anyhow::Result<Self> {
        if providers.is_empty() {
            return Err(anyhow::anyhow!(
                "multi backend requires at least one provider"
            ));
        }
        Ok(Self {
            providers,
            counter: AtomicUsize::new(0),
        })
    }
}

/// Factory for configured providers.
///
/// # Errors
///
/// Returns an error if the requested backend is unknown or any configured
/// backend cannot be constructed.
pub fn create_provider(backend: &str) -> anyhow::Result<Arc<dyn LlmProvider>> {
    match backend {
        "stub" => Ok(Arc::new(StubLlm::new())),
        "claude" => Ok(Arc::new(ClaudeApiProvider::from_env()?)),
        "ollama" => Ok(Arc::new(OllamaProvider::from_env())),
        "multi" => {
            let mut providers: Vec<Arc<dyn LlmProvider>> = Vec::new();
            if env::var("ANTHROPIC_API_KEY").is_ok() {
                providers.push(Arc::new(ClaudeApiProvider::from_env()?));
            }
            providers.push(Arc::new(OllamaProvider::from_env()));
            Ok(Arc::new(MultiProvider::new(providers)?))
        }
        other => Err(anyhow::anyhow!("unknown llm backend: {other}")),
    }
}

#[async_trait]
impl LlmProvider for StubLlm {
    async fn fill(&self, req: LlmRequest) -> anyhow::Result<serde_json::Value> {
        let n = self.next();
        let out = match req.slot.as_str() {
            "bounty_amount" => {
                let v = 10 + (n % 10) * 10;
                serde_json::json!(v)
            }
            "job_spec" => serde_json::json!(format!("compute feature#{n}")),
            "bid_amount" | "bid" => serde_json::to_value(BidDecision {
                bid: true,
                submission_content: Some(format!("result#{n}")),
            })?,
            "submission_content" => serde_json::json!(format!("submission#{n}")),
            "approve" | "approve_decision" => serde_json::to_value(VoteDecision {
                approve: n % 5 != 4,
                reason: "looks plausible".into(),
            })?,
            "confirm_decision" => serde_json::json!(n % 3 != 2),
            "insight_content" => {
                serde_json::json!(format!("heuristic#{n}: prefer lower slippage pools"))
            }
            "route_proposal" => {
                let route = if n.is_multiple_of(2) {
                    vec![
                        serde_json::json!({
                            "pool": "morpho-usdc-eth",
                            "amount_usdc": 60_000,
                            "reason": "reward under-utilized liquidity"
                        }),
                        serde_json::json!({
                            "pool": "aave-v3-usdc-eth",
                            "amount_usdc": 40_000,
                            "reason": "hedge execution risk"
                        }),
                    ]
                } else {
                    vec![
                        serde_json::json!({
                            "pool": "compound-v3-usdc",
                            "amount_usdc": 55_000,
                            "reason": "stable borrow curve"
                        }),
                        serde_json::json!({
                            "pool": "aave-v3-usdc-eth",
                            "amount_usdc": 45_000,
                            "reason": "deep base liquidity"
                        }),
                    ]
                };
                serde_json::json!({
                    "route": route,
                    "expected_output_eth": 50.0 + (n as f64 * 0.75),
                    "confidence": 0.72 + ((n % 5) as f64 * 0.03),
                    "reasoning": format!("stub strategy #{n}"),
                })
            }
            _ => serde_json::Value::Null,
        };
        Ok(out)
    }

    fn label(&self) -> &str {
        "stub"
    }
}

#[async_trait]
impl LlmProvider for ClaudeApiProvider {
    async fn fill(&self, req: LlmRequest) -> anyhow::Result<serde_json::Value> {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_str(&self.api_key)?);
        headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .headers(headers)
            .json(&serde_json::json!({
                "model": self.model,
                "max_tokens": 512,
                "temperature": 0,
                "messages": [{
                    "role": "user",
                    "content": provider_prompt(&req),
                }],
            }))
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;

        let text = response
            .get("content")
            .and_then(|value| value.as_array())
            .and_then(|items| {
                items.iter().find_map(|item| {
                    (item.get("type").and_then(|v| v.as_str()) == Some("text"))
                        .then(|| item.get("text").and_then(|v| v.as_str()))
                        .flatten()
                })
            })
            .ok_or_else(|| anyhow::anyhow!("claude response missing text content"))?;
        decode_slot_value(text)
    }

    fn label(&self) -> &str {
        &self.model
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    async fn fill(&self, req: LlmRequest) -> anyhow::Result<serde_json::Value> {
        let response = self
            .client
            .post(format!(
                "{}/api/generate",
                self.base_url.trim_end_matches('/')
            ))
            .json(&serde_json::json!({
                "model": self.model,
                "prompt": provider_prompt(&req),
                "stream": false,
                "format": "json",
                "options": {
                    "temperature": 0
                }
            }))
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;
        let text = response
            .get("response")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("ollama response missing `response` field"))?;
        decode_slot_value(text)
    }

    fn label(&self) -> &str {
        &self.model
    }
}

#[async_trait]
impl LlmProvider for MultiProvider {
    async fn fill(&self, req: LlmRequest) -> anyhow::Result<serde_json::Value> {
        let index = self.counter.fetch_add(1, Ordering::Relaxed) % self.providers.len();
        self.providers[index].fill(req).await
    }

    fn label(&self) -> &str {
        "multi"
    }
}

fn provider_prompt(req: &LlmRequest) -> String {
    format!(
        "You are a structured output engine.\n\
         Return JSON only.\n\
         Put the requested value under a top-level key named `value`.\n\
         Slot: {}\n\
         Context JSON:\n{}\n",
        req.slot,
        serde_json::to_string_pretty(&req.context).unwrap_or_else(|_| "{}".into())
    )
}

fn decode_slot_value(text: &str) -> anyhow::Result<serde_json::Value> {
    let json = parse_json_payload(text)?;
    Ok(json.get("value").cloned().unwrap_or(json))
}

fn parse_json_payload(text: &str) -> anyhow::Result<serde_json::Value> {
    serde_json::from_str(text).or_else(|_| {
        let start = text
            .find('{')
            .ok_or_else(|| anyhow::anyhow!("provider response did not contain JSON"))?;
        let end = text
            .rfind('}')
            .ok_or_else(|| anyhow::anyhow!("provider response did not contain JSON"))?;
        serde_json::from_str(&text[start..=end]).map_err(Into::into)
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{LlmProvider, StubLlm, parse_json_payload};

    #[test]
    fn parses_embedded_json() {
        let parsed = parse_json_payload("text before {\"value\": 42} text after").unwrap();
        assert_eq!(parsed["value"], 42);
    }

    #[test]
    fn stub_label_is_stable() {
        assert_eq!(StubLlm::new().label(), "stub");
    }
}
