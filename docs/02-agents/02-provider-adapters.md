# 02 — Provider Adapters

> Sub-doc 02 of **02-agents** · Roko Documentation
>
> This document describes the `ProviderAdapter` trait, the four concrete
> adapter implementations, the unified factory function `create_agent_for_model`,
> and the error classification system. It traces the design from the
> implementation plan through to the working code.


> **Implementation**: Shipping

---

## The ProviderAdapter Trait

The `ProviderAdapter` trait lives at `crates/roko-agent/src/provider/mod.rs:113`
and defines the contract for creating configured `Agent` instances from provider
config and model profiles:

```rust
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
```

Three methods, three responsibilities:

1. **`kind()`** — Identity. Returns which `ProviderKind` this adapter serves.
   Used by tests and diagnostics, not by dispatch (dispatch goes through
   `adapter_for_kind` by `ProviderKind` match).

2. **`create_agent()`** — Factory. Takes the provider configuration (URL,
   auth, timeout), the model profile (slug, capabilities, costs), and
   runtime options (system prompt, tools, MCP config), and returns a fully
   configured `Box<dyn Agent>`. This is where protocol-specific construction
   happens: the `AnthropicApiAdapter` creates an `AnthropicApiAgent` with
   content-block serialization, while the `OpenAiCompatAdapter` creates an
   `OpenAiAgent` with chat-completions format.

3. **`classify_error()`** — Error normalization. Takes an HTTP status code
   and response body and maps them to a canonical `ProviderError` variant.
   This drives the retry policy: rate limits trigger backoff, auth failures
   are terminal, server errors trigger fallback.

---

## The Four Adapters

Each adapter is a unit struct instantiated as a static constant. No per-request
state, no allocations on the hot path:

```rust
static ANTHROPIC_API_ADAPTER: AnthropicApiAdapter = AnthropicApiAdapter;
static CLAUDE_CLI_ADAPTER: ClaudeCliAdapter = ClaudeCliAdapter;
static CURSOR_ACP_ADAPTER: CursorAcpAdapter = CursorAcpAdapter;
static OPENAI_COMPAT_ADAPTER: OpenAiCompatAdapter = OpenAiCompatAdapter;
```

### 1. OpenAiCompatAdapter (`provider/openai_compat.rs`)

Handles the `OpenAiCompat` protocol family — the most widely used adapter
because most LLM providers expose an OpenAI-compatible chat completions API.

**Providers served:** ZhipuAI (GLM-5.1, GLM-4-Flash), Moonshot (Kimi),
DeepSeek, OpenRouter (200+ models), Perplexity (Sonar), Gemini (via
`/v1beta/openai/` compat endpoint), any `/v1/chat/completions`-compatible
API.

**Construction flow:**
1. Read `base_url` from `ProviderConfig`
2. Resolve API key from the environment variable named in `api_key_env`
3. Build an `OpenAiAgent` with the model slug from `ModelProfile`
4. Set timeout from `options.timeout_ms` or `provider.timeout_ms`
5. Inject `extra_headers` from the provider config
6. Set `max_tokens` from `profile.max_output`

**Error classification:** Parses the response body for OpenAI-style error
codes (`rate_limit_exceeded`, `model_not_found`, `context_length_exceeded`)
and maps them to canonical `ProviderError` variants.

### 2. AnthropicApiAdapter (`provider/anthropic_api.rs`)

Handles the `AnthropicApi` protocol family — Anthropic's native Messages
API, which uses content blocks rather than plain strings and supports
unique features like extended thinking and prompt caching.

**Construction flow:**
1. Read `base_url` (defaults to `https://api.anthropic.com`)
2. Resolve API key from `ANTHROPIC_API_KEY` env var
3. Build a `ClaudeAgent` (the HTTP-based Claude agent, not the CLI one)
4. Configure thinking support based on `profile.supports_thinking`
5. Set the `anthropic-version` header

**Distinction from `ClaudeCliAdapter`:** The `AnthropicApiAdapter` creates
an HTTP-based agent that Roko's ToolLoop drives. The `ClaudeCliAdapter`
creates a subprocess-based agent that drives its own internal tool loop.

### 3. ClaudeCliAdapter (`provider/claude_cli.rs`)

Handles the `ClaudeCli` protocol family — spawns the `claude` CLI binary
as a subprocess and communicates via stream-JSON over pipes.

**Construction flow:**
1. Read `command` from `ProviderConfig` (defaults to `"claude"`)
2. Build a `ClaudeCliAgent` with the model slug
3. Configure MCP passthrough via `--mcp-config` if `options.mcp_config` is set
4. Set bare mode, effort level, system prompt, tools, skip-permissions
5. Attach extra args from options

**Key property:** Claude CLI drives its own tool loop internally. Roko
does not use `ToolLoop` for this adapter — it sends a single prompt and
the CLI handles tool calling, multi-turn reasoning, and file edits. Roko
receives the final output plus intermediate signals via the stream-JSON
protocol. This is the primary adapter used by `orchestrate.rs` today.

### 4. CursorAcpAdapter (`provider/cursor_acp.rs`)

Handles the `CursorAcp` protocol family — Cursor's Agent Client Protocol,
a JSON-RPC protocol for communicating with Cursor's agent runtime.

**Construction flow:**
1. Read `command` from `ProviderConfig` (defaults to `"cursor-agent"`)
2. Build a `CursorAgent` with the model slug
3. Configure based on agent options

---

## The Unified Factory: `create_agent_for_model`

The factory function at `crates/roko-agent/src/provider/mod.rs:82` is the
single entry point for config-driven agent construction:

```rust
pub fn create_agent_for_model(
    config: &RokoConfig,
    model_key: &str,
    options: AgentOptions,
) -> Result<Box<dyn Agent>, AgentCreationError> {
    let resolved = resolve_model(config, model_key);

    let profile = resolved.profile
        .or_else(|| config.effective_models().get(model_key).cloned())
        .ok_or_else(|| AgentCreationError::MissingConfig("model".into()))?;

    let provider_config = resolved.provider_config
        .or_else(|| config.effective_providers().get(&profile.provider).cloned())
        .ok_or_else(|| AgentCreationError::MissingConfig("provider".into()))?;

    tracing::info!(
        model_key, slug = %resolved.slug,
        provider = %resolved.provider_kind,
        base_url = ?provider_config.base_url,
        "creating agent via provider adapter"
    );

    let adapter = adapter_for_kind(resolved.provider_kind);
    adapter.create_agent(&provider_config, &profile, &options)
}
```

### Resolution chain

1. `resolve_model(config, model_key)` — Look up the model in config, falling
   back to slug heuristics (see sub-doc 01).
2. `resolved.profile.or_else(|| config.effective_models()...)` — If resolution
   didn't find a profile, try the effective (merged) model registry.
3. `resolved.provider_config.or_else(|| config.effective_providers()...)` —
   Same fallback chain for the provider config.
4. `adapter_for_kind(resolved.provider_kind)` — Get the static adapter
   instance.
5. `adapter.create_agent(...)` — Construct the configured agent.

### Static dispatch via `adapter_for_kind`

```rust
pub fn adapter_for_kind(kind: ProviderKind) -> &'static dyn ProviderAdapter {
    match kind {
        ProviderKind::OpenAiCompat => &OPENAI_COMPAT_ADAPTER,
        ProviderKind::ClaudeCli    => &CLAUDE_CLI_ADAPTER,
        ProviderKind::AnthropicApi => &ANTHROPIC_API_ADAPTER,
        ProviderKind::CursorAcp    => &CURSOR_ACP_ADAPTER,
    }
}
```

This is a static dispatch table, not a dynamic registry. Adding a new
protocol family requires adding a variant to `ProviderKind`, implementing
`ProviderAdapter`, and adding a match arm. This is intentional — protocol
families change rarely, and the exhaustive match ensures no variant is
forgotten.

---

## AgentOptions

The `AgentOptions` struct at `crates/roko-agent/src/provider/mod.rs:132`
carries runtime parameters that aren't part of the config registry:

```rust
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
```

These fields mirror the parameters that `orchestrate.rs` currently threads
through `AgentRunConfig` (line 431). The goal is for `AgentOptions` to
replace `AgentRunConfig` entirely when the migration to `create_agent_for_model`
is complete.

---

## Error Classification and Retry Policy

### ProviderError enum

```rust
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
```

Each adapter's `classify_error` method maps provider-specific error shapes
to these canonical variants. This normalization is critical for the retry
policy — the same `ProviderError::RateLimit` variant drives the same
backoff behavior regardless of whether it came from Anthropic's
`overloaded_error`, OpenAI's `rate_limit_exceeded`, or ZhipuAI's
`1301` error code.

### RetryAction enum and `should_retry`

```rust
pub enum RetryAction {
    WaitAndRetry { delay_ms: u64 },
    TryFallback,
    TryWithSmallerContext,
    Skip,
}

pub fn should_retry(error: &ProviderError) -> RetryAction {
    match error {
        ProviderError::RateLimit { retry_after_ms } =>
            RetryAction::WaitAndRetry { delay_ms: retry_after_ms.unwrap_or(5_000) },
        ProviderError::AuthFailure    => RetryAction::Skip,
        ProviderError::Timeout        => RetryAction::TryFallback,
        ProviderError::ServerError(_) => RetryAction::TryFallback,
        ProviderError::ContentPolicy  => RetryAction::Skip,
        ProviderError::ContextOverflow => RetryAction::TryWithSmallerContext,
        _                             => RetryAction::TryFallback,
    }
}
```

The retry policy is deterministic and provider-agnostic:

- **Rate limit** → Wait the specified delay (or 5s default), then retry the
  same provider. The delay comes from the provider's `retry-after` header
  when available.
- **Auth failure** → Skip. No amount of retrying will fix a bad API key.
- **Timeout / Server error** → Try a different provider. The current provider
  may be temporarily overloaded.
- **Content policy** → Skip. The prompt triggered a content filter; retrying
  won't help.
- **Context overflow** → Try with smaller context. The prompt exceeded the
  model's window; the caller should prune history and retry.
- **Model not found / Other** → Try fallback. The model may not be available
  on this provider.

---

## AgentCreationError

```rust
pub enum AgentCreationError {
    MissingApiKey(String),
    MissingConfig(String),
    InvalidKind(ProviderKind),
}
```

These are construction-time errors, not runtime errors. They indicate that
the configuration is incomplete or invalid, not that a request failed.

---

## Test Coverage

The provider module includes integration tests that exercise the full factory
path with a mock HTTP server:

```rust
#[tokio::test]
async fn create_agent_for_model_returns_configured_agent() {
    let (base_url, captured, handle) = spawn_chat_server(response);
    let config = test_config(format!("{base_url}/v4"));
    let options = AgentOptions {
        timeout_ms: Some(2_500),
        name: "factory-agent".to_string(),
        ..Default::default()
    };
    let agent = create_agent_for_model(&config, "glm-5-1", options)
        .expect("create agent for model");
    assert_eq!(agent.name(), "factory-agent");

    let result = agent.run(&prompt("hello"), &Context::now()).await;
    assert!(result.success);
    assert_eq!(result.output.body.as_text().unwrap_or(""), "factory-ok");
}
```

This test verifies the complete chain: config resolution → adapter selection →
agent construction → HTTP request → response parsing → `AgentResult` extraction.
The captured request is inspected to verify the correct model slug, max_tokens,
and message format were sent.

---

## Citations

1. Implementation plan `modelrouting/03-provider-adapters.md` — ProviderAdapter
   trait design, 4 implementations, factory function. 19 tasks.
2. Implementation plan `modelrouting/01-architecture.md` — Three-layer provider
   system, why static dispatch.
3. `crates/roko-agent/src/provider/mod.rs` — Full 407-line source.
4. `crates/roko-agent/src/provider/openai_compat.rs` — OpenAiCompatAdapter.
5. `crates/roko-agent/src/provider/anthropic_api.rs` — AnthropicApiAdapter.
6. `crates/roko-agent/src/provider/claude_cli.rs` — ClaudeCliAdapter.
7. `crates/roko-agent/src/provider/cursor_acp.rs` — CursorAcpAdapter.
