//! Cerebras Inference provider adapter.
//!
//! Cerebras provides ultra-fast LLM inference via an OpenAI-compatible API at
//! `api.cerebras.ai/v1`. This adapter delegates to the OpenAI-compat HTTP
//! machinery but provides Cerebras-specific error classification and identity.

use crate::Agent;
use crate::provider::{
    AgentCreationError, AgentOptions, ProviderAdapter, ProviderError,
    openai_compat::OpenAiCompatAdapter,
};
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use serde_json::Value;

/// Adapter for the Cerebras Inference API.
///
/// Cerebras is fully OpenAI-compatible, so agent creation delegates to
/// [`OpenAiCompatAdapter`]. The dedicated adapter exists so that:
/// - Health tracking distinguishes Cerebras from other OpenAI-compat providers
/// - Error classification handles Cerebras-specific status codes
/// - Future Cerebras features (speculative decoding hints, batch API) have a home
pub struct CerebrasAdapter;

impl ProviderAdapter for CerebrasAdapter {
    fn kind(&self) -> ProviderKind {
        ProviderKind::CerebrasApi
    }

    fn create_agent(
        &self,
        provider: &ProviderConfig,
        model: &ModelProfile,
        options: &AgentOptions,
    ) -> Result<Box<dyn Agent>, AgentCreationError> {
        // Delegate fully to OpenAI-compat — Cerebras uses the same protocol.
        OpenAiCompatAdapter.create_agent(provider, model, options)
    }

    fn classify_error(&self, status: u16, body: &Value) -> ProviderError {
        match status {
            401 | 403 => ProviderError::AuthFailure,
            429 => {
                let retry_after_ms = body
                    .get("error")
                    .and_then(|e| e.get("retry_after"))
                    .and_then(|v| v.as_f64())
                    .map(|secs| (secs * 1000.0) as u64);
                ProviderError::RateLimit { retry_after_ms }
            }
            404 => ProviderError::ModelNotFound,
            408 | 504 => ProviderError::Timeout,
            400 => {
                let msg = body
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("");
                if msg.contains("context") || msg.contains("token") {
                    ProviderError::ContextOverflow
                } else {
                    ProviderError::Other(msg.to_string())
                }
            }
            500..=599 => ProviderError::ServerError(status),
            _ => ProviderError::Other(format!("HTTP {status}")),
        }
    }
}
