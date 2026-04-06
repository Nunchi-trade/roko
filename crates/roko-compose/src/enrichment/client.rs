//! LLM client trait for the enrichment pipeline.
//!
//! The pipeline calls models through this trait. Implementations live in the
//! app layer (not in this crate) — no HTTP logic, no subprocess spawning here.
//! This follows anti-pattern #8: I/O at boundary only.

/// Trait for calling an LLM backend.
///
/// Implementations live outside this crate (e.g. in the CLI or gateway client).
/// The enrichment pipeline only depends on this trait, never on concrete HTTP
/// or subprocess code.
#[async_trait::async_trait]
pub trait LlmClient: Send + Sync {
    /// Send a prompt to the LLM and return the generated text.
    ///
    /// # Arguments
    /// - `model` — model identifier (e.g. `"claude-sonnet-4-6"`).
    /// - `system` — system prompt.
    /// - `user` — user message.
    /// - `max_tokens` — token budget for the response.
    ///
    /// # Errors
    /// Returns an error if the LLM call fails (network, rate limit, etc.).
    async fn call(
        &self,
        model: &str,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
}
