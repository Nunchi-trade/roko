//! Crate-level error types for roko-agent.
//!
//! Public API functions return these typed errors. Internal helpers
//! may use `anyhow` or local error types freely.

pub use crate::provider::{AgentCreationError, ProviderError};
pub use crate::tool_loop::LlmError;

/// Top-level error for any agent dispatch operation visible to callers
/// outside this crate.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// Agent could not be constructed from the given config.
    #[error("agent creation failed: {0}")]
    Creation(#[from] AgentCreationError),

    /// The LLM backend returned an error during a turn.
    #[error("llm backend error: {0}")]
    Backend(#[from] LlmError),

    /// A provider-level error (rate limit, auth failure, etc.)
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),

    /// Tool dispatch failed (bad args, permission denied, tool panicked).
    #[error("tool dispatch error: {0}")]
    ToolDispatch(String),

    /// Safety contract rejected the operation.
    #[error("safety contract violation: {0}")]
    SafetyViolation(String),

    /// Generic catch-all for errors that don't fit above categories.
    /// New callers should prefer adding a typed variant rather than using this.
    #[error("{0}")]
    Other(String),
}
