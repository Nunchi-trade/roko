//! Provider routing and agent construction.
//!
//! This module is the migration layer between the older "pick a concrete
//! [`Agent`](crate::Agent) and construct it directly" style and the newer
//! provider-aware factory flow.
//!
//! ## Relationship to `Agent`
//!
//! The concrete backends in this crate, such as `ClaudeCliAgent`,
//! `CodexAgent`, `CursorAgent`, and `OllamaAgent`, still implement
//! [`Agent`](crate::Agent). The provider layer does not replace that trait.
//! Instead, it chooses and configures one of those implementations from
//! `RokoConfig` plus a model key, then returns `Box<dyn Agent>` to the
//! existing runtime.
//!
//! ## When to use `create_agent_for_model`
//!
//! Use [`create_agent_for_model`] when you want config-driven resolution:
//! the caller has a `RokoConfig`, a model key, and wants Roko to resolve the
//! provider, model slug, timeout, and other provider settings in one place.
//! This is the right entry point for CLI/runtime code that should follow the
//! configured routing rules.
//!
//! Construct a concrete agent directly when you already know the exact
//! backend you want, such as in a test, a focused integration, or a
//! backend-specific utility that intentionally bypasses model resolution.
//!
//! ## Adding a new provider
//!
//! There are two supported paths:
//!
//! - If the provider needs new runtime behavior, implement
//!   [`ProviderAdapter`] for it and register the adapter in
//!   [`adapter_for_kind`].
//! - If the provider is already covered by an existing adapter, add a
//!   matching `providers.*` entry and point one or more `models.*` entries at
//!   it in `roko.toml`.
//!
//! In both cases, the goal is the same: keep provider-specific wiring out of
//! the call sites and centralize it in this module.

use crate::Agent;
use crate::gemini::GeminiAdapter;
use roko_core::agent::{ProviderKind, resolve_model};
use roko_core::config::schema::RokoConfig;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use serde_json::Value;
use std::fmt;
use std::path::PathBuf;

pub mod anthropic_api;
pub mod claude_cli;
pub mod cursor_acp;
pub mod openai_compat;
pub mod openrouter_meta;

pub use anthropic_api::AnthropicApiAdapter;
pub use claude_cli::ClaudeCliAdapter;
pub use cursor_acp::CursorAcpAdapter;
pub use openai_compat::OpenAiCompatAdapter;
pub use openrouter_meta::fetch_model_metadata;

use crate::perplexity::PerplexityAdapter;

static ANTHROPIC_API_ADAPTER: AnthropicApiAdapter = AnthropicApiAdapter;
static CLAUDE_CLI_ADAPTER: ClaudeCliAdapter = ClaudeCliAdapter;
static CURSOR_ACP_ADAPTER: CursorAcpAdapter = CursorAcpAdapter;
static OPENAI_COMPAT_ADAPTER: OpenAiCompatAdapter = OpenAiCompatAdapter;
static PERPLEXITY_ADAPTER: PerplexityAdapter = PerplexityAdapter;
static GEMINI_ADAPTER: GeminiAdapter = GeminiAdapter;

/// Return the static adapter for a provider kind.
#[must_use]
pub fn adapter_for_kind(kind: ProviderKind) -> &'static dyn ProviderAdapter {
    match kind {
        ProviderKind::OpenAiCompat => &OPENAI_COMPAT_ADAPTER,
        ProviderKind::ClaudeCli => &CLAUDE_CLI_ADAPTER,
        ProviderKind::AnthropicApi => &ANTHROPIC_API_ADAPTER,
        ProviderKind::CursorAcp => &CURSOR_ACP_ADAPTER,
        ProviderKind::PerplexityApi => &PERPLEXITY_ADAPTER,
        ProviderKind::GeminiApi => &GEMINI_ADAPTER,
    }
}

/// Resolve a model key and create a configured agent for it.
///
/// This is the unified entrypoint for provider-aware agent construction.
#[must_use]
pub fn create_agent_for_model(
    config: &RokoConfig,
    model_key: &str,
    options: AgentOptions,
) -> Result<Box<dyn Agent>, AgentCreationError> {
    let resolved = resolve_model(config, model_key);

    let profile = resolved
        .profile
        .or_else(|| config.effective_models().get(model_key).cloned())
        .ok_or_else(|| AgentCreationError::MissingConfig("model".into()))?;

    let provider_config = resolved
        .provider_config
        .or_else(|| config.effective_providers().get(&profile.provider).cloned())
        .ok_or_else(|| AgentCreationError::MissingConfig("provider".into()))?;

    tracing::info!(
        model_key = model_key,
        slug = %resolved.slug,
        provider = %resolved.provider_kind,
        base_url = ?provider_config.base_url,
        "creating agent via provider adapter"
    );

    let adapter = adapter_for_kind(resolved.provider_kind);
    adapter.create_agent(&provider_config, &profile, &options)
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
    pub cached_content: Option<String>,
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
    use roko_core::config::schema::{ModelProfile, ProviderConfig, RokoConfig};
    use roko_core::{Body, Context, Kind, Signal};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    fn prompt(text: &str) -> Signal {
        Signal::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    fn spawn_chat_server(
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

                if let (Some(header_end), Some(content_length)) = (header_end, content_length)
                    && buf.len() >= header_end + content_length
                {
                    break;
                }
            }

            let header_end = header_end.expect("request headers");
            let content_length = content_length.expect("content length");
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

    fn test_config(base_url: String) -> RokoConfig {
        let mut config = RokoConfig::default();
        config.providers.insert(
            "zai".to_string(),
            ProviderConfig {
                kind: ProviderKind::OpenAiCompat,
                base_url: Some(base_url),
                api_key_env: Some("PATH".to_string()),
                command: None,
                args: None,
                timeout_ms: Some(1_500),
                ttft_timeout_ms: Some(15_000),
                connect_timeout_ms: Some(5_000),
                extra_headers: None,
                max_concurrent: None,
            },
        );
        config.models.insert(
            "glm-5-1".to_string(),
            ModelProfile {
                provider: "zai".to_string(),
                slug: "glm-5.1".to_string(),
                context_window: 200_000,
                max_output: Some(1_024),
                supports_tools: true,
                supports_thinking: true,
                supports_vision: false,
                supports_web_search: false,
                supports_mcp_tools: false,
                supports_partial: false,
                supports_grounding: false,
                supports_code_execution: false,
                supports_caching: false,
                provider_routing: None,
                tool_format: "openai_json".to_string(),
                cost_input_per_m: None,
                cost_output_per_m: None,
                cost_input_per_m_high: None,
                cost_output_per_m_high: None,
                cost_cache_read_per_m: None,
                cost_cache_write_per_m: None,
                thinking_level: None,
                max_tools: None,
                tokenizer_ratio: None,
                supports_search: false,
                supports_citations: false,
                supports_async: false,
                is_embedding_model: false,
                search_context_size: None,
                cost_per_request: None,
            },
        );
        config
    }

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
        assert_eq!(
            adapter_for_kind(ProviderKind::PerplexityApi).kind(),
            ProviderKind::PerplexityApi
        );
        assert_eq!(
            adapter_for_kind(ProviderKind::GeminiApi).kind(),
            ProviderKind::GeminiApi
        );
    }

    #[tokio::test]
    async fn create_agent_for_model_returns_configured_agent() {
        let response = serde_json::json!({
            "id": "chatcmpl-test",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "factory-ok"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 11,
                "completion_tokens": 7,
                "total_tokens": 18
            }
        })
        .to_string();
        let (base_url, captured, handle) = spawn_chat_server(response);
        let config = test_config(format!("{base_url}/v4"));
        let options = AgentOptions {
            timeout_ms: Some(2_500),
            name: "factory-agent".to_string(),
            ..Default::default()
        };

        let agent =
            create_agent_for_model(&config, "glm-5-1", options).expect("create agent for model");
        assert_eq!(agent.name(), "factory-agent");

        let result = agent.run(&prompt("hello"), &Context::now()).await;
        assert!(
            result.success,
            "{}",
            result.output.body.as_text().unwrap_or("unknown")
        );
        assert_eq!(result.output.body.as_text().unwrap_or(""), "factory-ok");

        let request = captured
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured request");
        assert!(request.starts_with("POST /v4/v1/chat/completions HTTP/1.1"));
        let body = request.split("\r\n\r\n").nth(1).expect("request body");
        let parsed: serde_json::Value = serde_json::from_str(body).expect("json request body");
        assert_eq!(parsed["model"], "glm-5.1");
        assert_eq!(parsed["max_tokens"], 1024);
        assert_eq!(parsed["messages"][0]["content"], "hello");

        handle.join().expect("server thread");
    }
}
