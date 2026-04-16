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

use crate::SafetyLayer;
use crate::dispatcher::{HandlerResolver, ToolDispatcher};
use crate::gemini::GeminiAdapter;
use crate::mock::MockAgent;
use crate::{Agent, ExecAgent};
use roko_core::agent::{ProviderKind, resolve_model};
use roko_core::config::schema::RokoConfig;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use roko_core::tool::ToolRegistry;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
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

use crate::perplexity::{PerplexityAdapter, SearchOptions};

static ANTHROPIC_API_ADAPTER: AnthropicApiAdapter = AnthropicApiAdapter;
static CLAUDE_CLI_ADAPTER: ClaudeCliAdapter = ClaudeCliAdapter;
static CURSOR_ACP_ADAPTER: CursorAcpAdapter = CursorAcpAdapter;
static OPENAI_COMPAT_ADAPTER: OpenAiCompatAdapter = OpenAiCompatAdapter;
static PERPLEXITY_ADAPTER: PerplexityAdapter = PerplexityAdapter;
static GEMINI_ADAPTER: GeminiAdapter = GeminiAdapter;
const DEFAULT_PROVIDER_MAX_CONCURRENT: usize = 10;
pub const PERPLEXITY_SEARCH_OPTIONS_ARG_PREFIX: &str = "pplx.search_options=";

thread_local! {
    static ACTIVE_SAFETY_LAYER: RefCell<Option<SafetyLayer>> = const { RefCell::new(None) };
}

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
    if let Some(mock_agent) = mock_agent_from_env(&options)? {
        return Ok(mock_agent);
    }
    let safety_layer = current_safety_layer().or_else(|| Some(SafetyLayer::from_config(config)));
    let resolved = resolve_model(config, model_key);
    let profile = resolved
        .profile
        .or_else(|| config.effective_models().get(model_key).cloned());
    let provider_config = profile.as_ref().and_then(|profile| {
        resolved
            .provider_config
            .clone()
            .or_else(|| config.effective_providers().get(&profile.provider).cloned())
    });
    let legacy_command = options
        .command
        .as_deref()
        .or(config.agent.command.as_deref());

    // When the command is a known protocol CLI (claude, codex, etc.) but no
    // explicit provider/model config exists, synthesize sensible defaults so
    // `roko init` + `roko prd draft new` works out of the box.
    let (provider_config, profile) = match (provider_config, profile) {
        (Some(pc), Some(mp)) => (pc, mp),
        (pc, mp) if legacy_command.is_some_and(is_known_protocol_command) => {
            let cmd = legacy_command.unwrap(); // safe: is_some_and passed
            let kind =
                provider_kind_for_known_protocol_command(cmd).unwrap_or(resolved.provider_kind);
            let pc = pc.unwrap_or_else(|| ProviderConfig {
                kind,
                command: Some(cmd.to_string()),
                timeout_ms: options.timeout_ms.or(Some(120_000)),
                base_url: None,
                api_key_env: None,
                args: None,
                ttft_timeout_ms: None,
                connect_timeout_ms: None,
                extra_headers: None,
                max_concurrent: None,
            });
            let mp = mp.unwrap_or_else(|| ModelProfile {
                provider: format!("{kind}"),
                slug: resolved.slug.clone(),
                ..Default::default()
            });
            tracing::info!(
                model_key = model_key,
                slug = %resolved.slug,
                command = cmd,
                "no explicit provider config — using defaults for known CLI"
            );
            (pc, mp)
        }
        _ => {
            tracing::warn!(
                model_key = model_key,
                command = %legacy_command.unwrap_or("unknown"),
                "no provider found — falling back to ExecAgent (no tool support)"
            );

            let mut agent =
                ExecAgent::new(legacy_command.unwrap_or("cat"), options.extra_args.clone())
                    .with_safety_layer(safety_layer)
                    .with_timeout_ms(options.timeout_ms.unwrap_or(120_000));
            if !options.name.is_empty() {
                agent = agent.with_name(options.name.clone());
            }
            if !options.env.is_empty() {
                agent = agent.with_env(options.env.clone());
            }
            return Ok(Box::new(agent) as Box<dyn Agent>);
        }
    };

    tracing::info!(
        model_key = model_key,
        slug = %resolved.slug,
        provider = %provider_config.kind,
        base_url = ?provider_config.base_url,
        "creating agent via provider adapter"
    );

    if options.provider_semaphores.is_none() {
        let providers = config.effective_providers();
        options.provider_semaphores = Some(Arc::new(ProviderSemaphores::new(&providers)));
    }

    let adapter = adapter_for_kind(provider_config.kind);
    with_safety_layer(safety_layer, || {
        adapter.create_agent(&provider_config, &profile, &options)
    })
}

fn mock_agent_from_env(
    options: &AgentOptions,
) -> Result<Option<Box<dyn Agent>>, AgentCreationError> {
    let Ok(dispatcher) = env::var("ROKO_DISPATCHER") else {
        return Ok(None);
    };
    let fixture = match dispatcher.strip_prefix("mock-") {
        Some(fixture) if !fixture.trim().is_empty() => fixture,
        _ => return Ok(None),
    };

    let mut agent = MockAgent::scripted_from_fixture(fixture)
        .map_err(|err| AgentCreationError::FixtureLoad(err.to_string()))?;
    if let Some(working_dir) = options.working_dir.clone() {
        agent = agent.with_working_dir(working_dir);
    }
    if let Ok(state_path) = env::var("ROKO_MOCK_STATE_PATH") {
        let state_path = state_path.trim();
        if !state_path.is_empty() {
            agent = agent.with_state_path(state_path);
        }
    }
    if !options.name.is_empty() {
        agent = agent.with_name(options.name.clone());
    }
    Ok(Some(Box::new(agent)))
}

/// Run `f` with an optional safety layer attached to provider-backed agent construction.
///
/// This is intentionally scoped to synchronous construction so the thread-local
/// state cannot leak across async suspension points.
#[must_use]
pub fn with_safety_layer<R>(layer: Option<SafetyLayer>, f: impl FnOnce() -> R) -> R {
    let scope = set_active_safety_layer(layer);
    let result = f();
    drop(scope);
    result
}

/// Run `f` with the current safety layer, or default to [`SafetyLayer::with_defaults()`].
///
/// This is the common case for direct agent construction paths that want the same
/// safety scope behavior as orchestrated runs without having to duplicate the fallback.
#[must_use]
pub fn with_scoped_safety_layer<R>(f: impl FnOnce() -> R) -> R {
    let layer = current_safety_layer().or_else(|| Some(SafetyLayer::with_defaults()));
    with_safety_layer(layer, f)
}

/// Build a `ToolDispatcher` and attach the active safety layer if one is present.
#[must_use]
pub fn build_tool_dispatcher(
    registry: Arc<dyn ToolRegistry>,
    resolver: Arc<dyn HandlerResolver>,
) -> Arc<ToolDispatcher> {
    let dispatcher = ToolDispatcher::new(registry, resolver);
    match current_safety_layer() {
        Some(layer) => Arc::new(dispatcher.with_safety(layer)),
        None => Arc::new(dispatcher),
    }
}

/// Return the safety layer currently scoped to provider-backed construction, if any.
#[must_use]
pub fn current_safety_layer() -> Option<SafetyLayer> {
    ACTIVE_SAFETY_LAYER.with(|slot| slot.borrow().clone())
}

struct SafetyLayerScope {
    previous: Option<SafetyLayer>,
}

impl Drop for SafetyLayerScope {
    fn drop(&mut self) {
        ACTIVE_SAFETY_LAYER.with(|slot| {
            slot.replace(self.previous.take());
        });
    }
}

fn set_active_safety_layer(layer: Option<SafetyLayer>) -> SafetyLayerScope {
    let previous = ACTIVE_SAFETY_LAYER.with(|slot| slot.replace(layer));
    SafetyLayerScope { previous }
}

#[must_use]
pub fn is_known_protocol_command(command: &str) -> bool {
    provider_kind_for_known_protocol_command(command).is_some()
}

#[must_use]
fn provider_kind_for_known_protocol_command(command: &str) -> Option<ProviderKind> {
    let executable = Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command);

    match executable {
        "claude" => Some(ProviderKind::ClaudeCli),
        "codex" => Some(ProviderKind::OpenAiCompat),
        "cursor-agent" | "cursor_agent" => Some(ProviderKind::CursorAcp),
        _ => None,
    }
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
    pub working_dir: Option<PathBuf>,
    pub provider_semaphores: Option<Arc<ProviderSemaphores>>,
    pub env: Vec<(String, String)>,
    pub extra_args: Vec<String>,
    pub effort: Option<String>,
    pub bare_mode: bool,
    pub dangerously_skip_permissions: bool,
    pub name: String,
}

impl AgentOptions {
    /// Root the agent subprocess in the given working directory.
    #[must_use]
    pub fn with_working_dir(mut self, working_dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(working_dir.into());
        self
    }

    /// Append Perplexity search options as a structured `extra_args` payload.
    #[must_use]
    pub fn with_perplexity_search_options(mut self, search_options: SearchOptions) -> Self {
        let encoded = serde_json::to_string(&search_options)
            .expect("Perplexity search options must serialize");
        self.extra_args
            .push(format!("{PERPLEXITY_SEARCH_OPTIONS_ARG_PREFIX}{encoded}"));
        self
    }
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
    #[error("Failed to load mock fixture: {0}")]
    FixtureLoad(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::config::schema::{ModelProfile, ProviderConfig, RokoConfig};
    use roko_core::{Body, Context, Engram, Kind};
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::time::timeout;

    fn prompt(text: &str) -> Engram {
        Engram::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    fn write_script(path: &std::path::Path, body: &str) {
        fs::write(path, body).expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(path).expect("script metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).expect("chmod script");
        }
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

    fn perplexity_config(
        base_url: Option<String>,
        model_slug: &str,
        supports_async: bool,
    ) -> RokoConfig {
        let mut config = RokoConfig::default();
        config.providers.insert(
            "perplexity".to_string(),
            ProviderConfig {
                kind: ProviderKind::PerplexityApi,
                base_url,
                api_key_env: Some("PATH".to_string()),
                command: None,
                args: None,
                timeout_ms: Some(300_000),
                ttft_timeout_ms: Some(15_000),
                connect_timeout_ms: Some(5_000),
                extra_headers: None,
                max_concurrent: None,
            },
        );
        config.models.insert(
            model_slug.to_string(),
            ModelProfile {
                provider: "perplexity".to_string(),
                slug: model_slug.to_string(),
                context_window: 127_072,
                max_output: Some(8_192),
                supports_tools: false,
                supports_thinking: false,
                supports_vision: false,
                supports_web_search: true,
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
                supports_search: true,
                supports_citations: true,
                supports_async,
                is_embedding_model: false,
                search_context_size: Some("medium".to_string()),
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

    #[test]
    fn build_tool_dispatcher_attaches_scoped_safety_layer() {
        fn no_handler(_: &str) -> Option<Arc<dyn roko_core::tool::ToolHandler>> {
            None
        }

        let registry: Arc<dyn roko_core::tool::ToolRegistry> =
            Arc::new(roko_core::tool::VecToolRegistry::from_tools(Vec::new()));
        let resolver: Arc<dyn HandlerResolver> = Arc::new(no_handler);

        let dispatcher = with_safety_layer(Some(SafetyLayer::with_defaults()), || {
            build_tool_dispatcher(registry, resolver)
        });

        assert!(dispatcher.safety().is_some());
    }

    #[test]
    fn with_scoped_safety_layer_defaults_when_unscoped() {
        let layer = with_scoped_safety_layer(current_safety_layer);
        assert!(layer.is_some());
    }

    #[test]
    fn with_scoped_safety_layer_preserves_existing_scope() {
        let observed = with_safety_layer(Some(SafetyLayer::with_defaults()), || {
            with_scoped_safety_layer(current_safety_layer)
        });
        assert!(observed.is_some());
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

    #[tokio::test]
    async fn create_agent_for_model_routes_perplexity_search_grounded_chat() {
        let response = serde_json::json!({
            "id": "chatcmpl-pplx",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "research-ok"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 21,
                "completion_tokens": 9,
                "total_tokens": 30
            }
        })
        .to_string();
        let (base_url, captured, handle) = spawn_chat_server(response);
        let config = perplexity_config(Some(base_url.clone()), "sonar-pro", false);
        let options = AgentOptions {
            timeout_ms: Some(45_000),
            name: "research-agent".to_string(),
            ..Default::default()
        }
        .with_perplexity_search_options(SearchOptions {
            search_domain_filter: Some(vec!["arxiv.org".to_string(), "nature.com".to_string()]),
            search_recency_filter: Some("week".to_string()),
            search_context_size: Some("high".to_string()),
            search_mode: Some("academic".to_string()),
            return_images: Some(false),
            ..Default::default()
        });

        let agent = create_agent_for_model(&config, "sonar-pro", options)
            .expect("create perplexity chat agent");
        assert_eq!(agent.name(), "research-agent");

        let result = agent.run(&prompt("research"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap_or(""), "research-ok");

        let request = captured
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured request");
        assert!(request.starts_with("POST /chat/completions HTTP/1.1"));
        let body = request.split("\r\n\r\n").nth(1).expect("request body");
        let parsed: serde_json::Value = serde_json::from_str(body).expect("json request body");
        assert_eq!(parsed["search_domain_filter"][0], "arxiv.org");
        assert_eq!(parsed["search_recency_filter"], "week");
        assert_eq!(parsed["search_mode"], "academic");
        assert_eq!(parsed["web_search_options"]["search_context_size"], "high");
        assert_eq!(parsed["return_images"], false);

        handle.join().expect("server thread");
    }

    #[test]
    fn create_agent_for_model_routes_perplexity_async_models_to_deep_research() {
        let config = perplexity_config(None, "sonar-deep-research", true);
        let agent = create_agent_for_model(
            &config,
            "sonar-deep-research",
            AgentOptions {
                name: "deep-research-agent".to_string(),
                ..Default::default()
            },
        )
        .expect("create deep research agent");
        assert_eq!(agent.name(), "deep-research-agent");
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

    #[test]
    fn exec_agent_fallback_defaults_safety_layer_when_unscoped() {
        let mut config = RokoConfig::default();
        config.agent.command = Some("sh".to_string());

        let agent = create_agent_for_model(
            &config,
            "unknown-model",
            AgentOptions {
                timeout_ms: Some(250),
                name: "fallback-agent".to_string(),
                extra_args: vec!["-c".to_string(), "rm -rf /".to_string()],
                ..Default::default()
            },
        )
        .expect("fallback exec agent");
        assert_eq!(agent.name(), "fallback-agent");

        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let result = runtime.block_on(async { agent.run(&prompt(""), &Context::now()).await });
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .unwrap_or("")
                .contains("blocked by safety layer")
        );
    }

    #[test]
    fn exec_agent_fallback_uses_scoped_safety_layer_when_active() {
        let mut config = RokoConfig::default();
        config.agent.command = Some("sh".to_string());

        let agent = with_safety_layer(Some(SafetyLayer::with_defaults()), || {
            create_agent_for_model(
                &config,
                "unknown-model",
                AgentOptions {
                    timeout_ms: Some(250),
                    name: "fallback-agent".to_string(),
                    extra_args: vec!["-c".to_string(), "rm -rf /".to_string()],
                    ..Default::default()
                },
            )
        })
        .expect("fallback exec agent");
        assert_eq!(agent.name(), "fallback-agent");

        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let result = runtime.block_on(async { agent.run(&prompt(""), &Context::now()).await });
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .unwrap_or("")
                .contains("blocked by safety layer")
        );
    }

    #[test]
    fn known_protocol_command_detection_handles_paths() {
        assert!(is_known_protocol_command("claude"));
        assert!(is_known_protocol_command("/tmp/cursor-agent"));
        assert!(is_known_protocol_command("bin/cursor_agent"));
        assert!(is_known_protocol_command("/usr/local/bin/codex"));
        assert!(!is_known_protocol_command("cat"));
    }

    #[tokio::test]
    async fn create_agent_for_model_uses_command_kind_for_ambiguous_claude_model_key() {
        let tmp = tempdir().expect("tempdir");
        let script = tmp.path().join("claude");
        let prompt_file = tmp.path().join("prompt.txt");
        let response = r#"{"type":"content_block_delta","delta":{"text":"factory-claude-ok"}}"#;
        let script_body = format!(
            "#!/bin/sh\nset -eu\ncat > \"{}\"\nprintf '%s\\n' '{}'\n",
            prompt_file.display(),
            response,
        );
        write_script(&script, &script_body);

        let agent = create_agent_for_model(
            &RokoConfig::default(),
            "claude",
            AgentOptions {
                command: Some(script.display().to_string()),
                timeout_ms: Some(5_000),
                name: "factory-claude".to_string(),
                ..Default::default()
            },
        )
        .expect("create synthesized claude agent");

        assert_eq!(agent.name(), "factory-claude");

        let result = agent.run(&prompt("hello"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(
            result.output.body.as_text().unwrap_or(""),
            "factory-claude-ok"
        );
        assert_eq!(
            fs::read_to_string(prompt_file).expect("read prompt file"),
            "hello"
        );
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
