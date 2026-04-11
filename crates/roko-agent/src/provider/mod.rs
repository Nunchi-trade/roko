use crate::Agent;
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use serde_json::Value;
use std::fmt;
use std::path::PathBuf;

pub mod anthropic_api;
pub mod claude_cli;
pub mod cursor_acp;
pub mod openai_compat;

pub use anthropic_api::AnthropicApiAdapter;
pub use claude_cli::ClaudeCliAdapter;
pub use cursor_acp::CursorAcpAdapter;
pub use openai_compat::OpenAiCompatAdapter;

static ANTHROPIC_API_ADAPTER: AnthropicApiAdapter = AnthropicApiAdapter;
static CLAUDE_CLI_ADAPTER: ClaudeCliAdapter = ClaudeCliAdapter;
static CURSOR_ACP_ADAPTER: CursorAcpAdapter = CursorAcpAdapter;
static OPENAI_COMPAT_ADAPTER: OpenAiCompatAdapter = OpenAiCompatAdapter;

/// Return the static adapter for a provider kind.
#[must_use]
pub fn adapter_for_kind(kind: ProviderKind) -> &'static dyn ProviderAdapter {
    match kind {
        ProviderKind::OpenAiCompat => &OPENAI_COMPAT_ADAPTER,
        ProviderKind::ClaudeCli => &CLAUDE_CLI_ADAPTER,
        ProviderKind::AnthropicApi => &ANTHROPIC_API_ADAPTER,
        ProviderKind::CursorAcp => &CURSOR_ACP_ADAPTER,
    }
}

/// Adapter for a protocol family. Creates Agent instances configured for a
/// specific provider and model.
pub trait ProviderAdapter: Send + Sync {
    /// Which protocol family this adapter handles.
    fn kind(&self) -> ProviderKind;

    /// Create an Agent instance from provider config and model profile.
    fn create_agent(
        &self,
        provider: &ProviderConfig,
        model: &ModelProfile,
        options: &AgentOptions,
    ) -> Result<Box<dyn Agent>, AgentCreationError>;

    /// Classify an error response into a canonical error type.
    /// Used by health tracking to decide retry vs cooldown vs skip.
    fn classify_error(&self, status: u16, body: &Value) -> ProviderError;
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default)]
pub struct AgentOptions {
    pub timeout_ms: Option<u64>,
    pub system_prompt: Option<String>,
    pub tools: Option<String>,
    pub mcp_config: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub extra_args: Vec<String>,
    pub effort: Option<String>,
    pub bare_mode: bool,
    pub dangerously_skip_permissions: bool,
    pub name: String,
}

#[derive(Debug, Clone)]
pub enum ProviderError {
    RateLimit { retry_after_ms: Option<u64> },
    AuthFailure,
    Timeout,
    ServerError(u16),
    ContentPolicy,
    ContextOverflow,
    ModelNotFound,
    Other(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RateLimit { retry_after_ms } => match retry_after_ms {
                Some(ms) => write!(f, "rate limited; retry after {ms} ms"),
                None => f.write_str("rate limited"),
            },
            Self::AuthFailure => f.write_str("authentication failed"),
            Self::Timeout => f.write_str("request timed out"),
            Self::ServerError(status) => write!(f, "server error {status}"),
            Self::ContentPolicy => f.write_str("content policy violation"),
            Self::ContextOverflow => f.write_str("context overflow"),
            Self::ModelNotFound => f.write_str("model not found"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ProviderError {}

#[derive(Debug, thiserror::Error)]
pub enum AgentCreationError {
    #[error("Missing API key: env var {0} not set")]
    MissingApiKey(String),
    #[error("Missing required config field: {0}")]
    MissingConfig(String),
    #[error("Invalid provider kind: {0:?}")]
    InvalidKind(ProviderKind),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_for_kind_returns_expected_adapter() {
        assert_eq!(
            adapter_for_kind(ProviderKind::OpenAiCompat).kind(),
            ProviderKind::OpenAiCompat
        );
        assert_eq!(
            adapter_for_kind(ProviderKind::ClaudeCli).kind(),
            ProviderKind::ClaudeCli
        );
        assert_eq!(
            adapter_for_kind(ProviderKind::AnthropicApi).kind(),
            ProviderKind::AnthropicApi
        );
        assert_eq!(
            adapter_for_kind(ProviderKind::CursorAcp).kind(),
            ProviderKind::CursorAcp
        );
    }
}
