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

use crate::{Agent, ExecAgent};
use crate::gemini::GeminiAdapter;
use roko_core::agent::{ProviderKind, resolve_model};
use roko_core::config::schema::RokoConfig;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

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
const DEFAULT_PROVIDER_MAX_CONCURRENT: usize = 10;

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
    let mut options = options;
    let resolved = resolve_model(config, model_key);
    let profile = resolved
        .profile
        .or_else(|| config.effective_models().get(model_key).cloned());
    let provider_config = profile
        .as_ref()
        .and_then(|profile| {
            resolved.provider_config.clone().or_else(|| {
                config
                    .effective_providers()
                    .get(&profile.provider)
                    .cloned()
            })
        });
    let legacy_command = options.command.as_deref().or(config.agent.command.as_deref());

    let Some(provider_config) = provider_config else {
        if is_known_protocol_command(legacy_command) {
            return Err(AgentCreationError::MissingConfig("provider".into()));
        }

        tracing::warn!(
            model_key = model_key,
            command = %legacy_command.unwrap_or("unknown"),
            "no provider found — falling back to ExecAgent (no tool support)"
        );

        let mut agent = ExecAgent::new(
            legacy_command.unwrap_or("cat"),
            options.extra_args.clone(),
        )
        .with_timeout_ms(options.timeout_ms.unwrap_or(120_000));
        if !options.name.is_empty() {
            agent = agent.with_name(options.name.clone());
        }
        if !options.env.is_empty() {
            agent = agent.with_env(options.env.clone());
        }
        return Ok(Box::new(agent));
    };
    let profile = profile.ok_or_else(|| AgentCreationError::MissingConfig("model".into()))?;

    tracing::info!(
        model_key = model_key,
        slug = %resolved.slug,
        provider = %resolved.provider_kind,
        base_url = ?provider_config.base_url,
        "creating agent via provider adapter"
    );

    if options.provider_semaphores.is_none() {
        let providers = config.effective_providers();
        options.provider_semaphores = Some(Arc::new(ProviderSemaphores::new(&providers)));
    }

    let adapter = adapter_for_kind(resolved.provider_kind);
    adapter.create_agent(&provider_config, &profile, &options)
}

fn is_known_protocol_command(command: Option<&str>) -> bool {
    let Some(command) = command else {
        return false;
    };

    let executable = Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command);

    matches!(executable, "claude" | "codex" | "cursor-agent" | "cursor_agent")
}

/// Shared semaphores that cap in-flight requests per provider.
#[derive(Debug)]
pub struct ProviderSemaphores {
    semaphores: HashMap<String, Arc<Semaphore>>,
    default_permits: usize,
}

impl ProviderSemaphores {
    #[must_use]
    pub fn new(configs: &HashMap<String, ProviderConfig>) -> Self {
        let mut semaphores = HashMap::with_capacity(configs.len());
        for (id, config) in configs {
            let permits = config
                .max_concurrent
                .unwrap_or(DEFAULT_PROVIDER_MAX_CONCURRENT as u32)
                .max(1) as usize;
            semaphores.insert(id.clone(), Arc::new(Semaphore::new(permits)));
        }

        Self {
            semaphores,
            default_permits: DEFAULT_PROVIDER_MAX_CONCURRENT,
        }
    }

    pub async fn acquire(&self, provider_id: &str) -> OwnedSemaphorePermit {
        let semaphore = self
            .semaphores
            .get(provider_id)
            .cloned()
            .unwrap_or_else(|| Arc::new(Semaphore::new(self.default_permits)));

        semaphore.acquire_owned().await.expect("semaphore closed")
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
    pub command: Option<String>,
    pub timeout_ms: Option<u64>,
    pub system_prompt: Option<String>,
    pub cached_content: Option<String>,
    pub tools: Option<String>,
    pub mcp_config: Option<PathBuf>,
    pub provider_semaphores: Option<Arc<ProviderSemaphores>>,
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

/// Retry decision for a classified provider error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryAction {
    /// Wait for the specified delay, then retry the same provider.
    WaitAndRetry { delay_ms: u64 },
    /// Try a different provider or backend.
    TryFallback,
    /// Retry with a smaller or shorter context.
    TryWithSmallerContext,
    /// Do not retry this error.
    Skip,
}

/// Map a provider error class to a retry action.
#[must_use]
pub fn should_retry(error: &ProviderError) -> RetryAction {
    match error {
        ProviderError::RateLimit { retry_after_ms } => RetryAction::WaitAndRetry {
            delay_ms: retry_after_ms.unwrap_or(5_000),
        },
        ProviderError::AuthFailure => RetryAction::Skip,
        ProviderError::Timeout => RetryAction::TryFallback,
        ProviderError::ServerError(_) => RetryAction::TryFallback,
        ProviderError::ContentPolicy => RetryAction::Skip,
        ProviderError::ContextOverflow => RetryAction::TryWithSmallerContext,
        _ => RetryAction::TryFallback,
    }
}

impl ProviderError {
    #[must_use]
    pub const fn retry_after_ms(&self) -> Option<u64> {
        match self {
            Self::RateLimit { retry_after_ms } => *retry_after_ms,
            _ => None,
        }
    }
}

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
    use tokio::time::timeout;

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
                ..Default::default()
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
        assert!(
            request.starts_with("POST /v4/chat/completions HTTP/1.1"),
            "unexpected request line: {}",
            request.lines().next().unwrap_or("")
        );
        let body = request.split("\r\n\r\n").nth(1).expect("request body");
        let parsed: serde_json::Value = serde_json::from_str(body).expect("json request body");
        assert_eq!(parsed["model"], "glm-5.1");
        assert_eq!(parsed["max_tokens"], 1024);
        assert_eq!(parsed["messages"][1]["content"], "hello");

        handle.join().expect("server thread");
    }

    #[test]
    fn retry_policy_maps_error_classes() {
        assert_eq!(
            should_retry(&ProviderError::RateLimit {
                retry_after_ms: Some(1_250),
            }),
            RetryAction::WaitAndRetry { delay_ms: 1_250 }
        );
        assert_eq!(
            should_retry(&ProviderError::RateLimit {
                retry_after_ms: None,
            }),
            RetryAction::WaitAndRetry { delay_ms: 5_000 }
        );
        assert_eq!(should_retry(&ProviderError::AuthFailure), RetryAction::Skip);
        assert_eq!(
            should_retry(&ProviderError::Timeout),
            RetryAction::TryFallback
        );
        assert_eq!(
            should_retry(&ProviderError::ServerError(503)),
            RetryAction::TryFallback
        );
        assert_eq!(
            should_retry(&ProviderError::ContentPolicy),
            RetryAction::Skip
        );
        assert_eq!(
            should_retry(&ProviderError::ContextOverflow),
            RetryAction::TryWithSmallerContext
        );
        assert_eq!(
            should_retry(&ProviderError::Other("x".to_string())),
            RetryAction::TryFallback
        );
    }

    #[tokio::test]
    async fn exec_agent_fallback_for_unknown_model_key() {
        let mut config = RokoConfig::default();
        config.agent.command = Some("cat".to_string());

        let agent = create_agent_for_model(
            &config,
            "unknown-model",
            AgentOptions {
                timeout_ms: Some(250),
                name: "fallback-agent".to_string(),
                ..Default::default()
            },
        )
        .expect("fallback exec agent");

        assert_eq!(agent.name(), "fallback-agent");

        let result = agent.run(&prompt("fallback-ok"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap_or(""), "fallback-ok");
    }

    #[tokio::test]
    async fn provider_semaphore_blocks_fourth_request_when_limit_is_three() {
        let mut configs = HashMap::new();
        configs.insert(
            "zai".to_string(),
            ProviderConfig {
                kind: ProviderKind::OpenAiCompat,
                base_url: Some("https://api.z.ai/api/paas/v4".to_string()),
                api_key_env: Some("ZAI_API_KEY".to_string()),
                command: None,
                args: None,
                timeout_ms: Some(1_500),
                ttft_timeout_ms: Some(15_000),
                connect_timeout_ms: Some(5_000),
                extra_headers: None,
                max_concurrent: Some(3),
            },
        );

        let semaphores = ProviderSemaphores::new(&configs);
        let permit_one = semaphores.acquire("zai").await;
        let permit_two = semaphores.acquire("zai").await;
        let permit_three = semaphores.acquire("zai").await;

        assert!(
            timeout(Duration::from_millis(50), semaphores.acquire("zai"))
                .await
                .is_err(),
            "fourth request should block while all permits are held"
        );

        drop(permit_one);

        let permit_four = timeout(Duration::from_millis(50), semaphores.acquire("zai"))
            .await
            .expect("fourth request should acquire after a permit is released");

        drop(permit_two);
        drop(permit_three);
        drop(permit_four);
    }
}
