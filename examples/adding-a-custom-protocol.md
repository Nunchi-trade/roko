# Adding a Custom Protocol Provider

Use this path when the provider does not fit the zero-code `openai_compat` flow in [adding-a-provider.md](./adding-a-provider.md).

This is the right guide when the provider needs one or more of these:

- a new wire protocol
- a custom auth/header scheme
- request shaping that does not fit the existing adapters
- response parsing that does not fit the existing agents
- a provider-native tool format

Examples in this repo:

- `crates/roko-agent/src/gemini/adapter.rs`
- `crates/roko-agent/src/provider/anthropic_api.rs`
- `crates/roko-agent/src/provider/cursor_acp.rs`
- `crates/roko-agent/src/perplexity/adapter.rs`

## The extension path

Roko's live provider extension path is:

1. Add a new `ProviderKind`
2. Implement a `ProviderAdapter`
3. Register that adapter in the provider factory
4. Add config, tests, and optional translator/tool-format wiring

The older "add a backend under `tool_loop/backends`" path is not the current architecture.

## Step 1: Add a `ProviderKind`

File: `crates/roko-core/src/agent.rs`

Add a new enum variant with a stable serialized name:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    AnthropicApi,
    ClaudeCli,
    OpenAiCompat,
    CursorAcp,
    PerplexityApi,
    GeminiApi,
    YourProviderApi,
}

impl ProviderKind {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::AnthropicApi => "anthropic_api",
            Self::ClaudeCli => "claude_cli",
            Self::OpenAiCompat => "openai_compat",
            Self::CursorAcp => "cursor_acp",
            Self::PerplexityApi => "perplexity_api",
            Self::GeminiApi => "gemini_api",
            Self::YourProviderApi => "your_provider_api",
        }
    }
}
```

Important:

- The `label()` value is the config value used in `[providers.<id>].kind`.
- Add or update the enum tests in this file so the new kind round-trips through serde.
- Do not add a new `AgentBackend` or slug heuristic by default. The new architecture is config-driven. Only touch `AgentBackend::from_model()` if you intentionally need legacy fallback inference.

## Step 2: Implement the adapter and concrete agent

Primary file: `crates/roko-agent/src/provider/your_provider.rs`

Optional new runtime files:

- `crates/roko-agent/src/your_provider/agent.rs`
- `crates/roko-agent/src/your_provider/types.rs`

The adapter is the provider-factory seam. It receives a resolved `ProviderConfig` and `ModelProfile`, then returns a concrete `Agent`.

Minimal adapter skeleton:

```rust
use crate::Agent;
use crate::provider::{AgentCreationError, AgentOptions, ProviderAdapter, ProviderError};
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use serde_json::Value;

pub struct YourProviderAdapter;

impl ProviderAdapter for YourProviderAdapter {
    fn kind(&self) -> ProviderKind {
        ProviderKind::YourProviderApi
    }

    fn create_agent(
        &self,
        provider: &ProviderConfig,
        model: &ModelProfile,
        options: &AgentOptions,
    ) -> Result<Box<dyn Agent>, AgentCreationError> {
        if provider.kind != self.kind() {
            return Err(AgentCreationError::InvalidKind(provider.kind));
        }

        let api_key = provider.resolve_api_key().ok_or_else(|| {
            AgentCreationError::MissingApiKey(
                provider
                    .api_key_env
                    .clone()
                    .unwrap_or_else(|| "YOUR_PROVIDER_API_KEY".into()),
            )
        })?;

        let base_url = provider
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.example.com".to_string());

        Ok(Box::new(YourProviderAgent::new(
            api_key,
            base_url,
            model.clone(),
            options,
        )))
    }

    fn classify_error(&self, status: u16, body: &Value) -> ProviderError {
        match status {
            429 => ProviderError::RateLimit { retry_after_ms: None },
            401 | 403 => ProviderError::AuthFailure,
            404 => ProviderError::ModelNotFound,
            408 | 504 => ProviderError::Timeout,
            500..=599 => ProviderError::ServerError(status),
            _ => ProviderError::Other(format!("HTTP {status}: {body}")),
        }
    }
}
```

Use the existing adapters as templates:

- `provider/anthropic_api.rs` for an HTTP API with a distinct request shape
- `provider/cursor_acp.rs` for a provider-specific HTTP protocol
- `gemini/adapter.rs` for an adapter that chooses between multiple concrete agent implementations
- `perplexity/adapter.rs` for capability-driven agent selection from `ModelProfile`

If the provider needs a brand-new protocol, put the protocol-specific request building and response parsing in the concrete agent implementation, not in call sites.

### If you need new config fields

Most providers fit the current `ProviderConfig` shape:

- `kind`
- `base_url`
- `api_key_env`
- `command`
- `args`
- `timeout_ms`
- `ttft_timeout_ms`
- `connect_timeout_ms`
- `extra_headers`
- `max_concurrent`

If that is not enough, add fields in both places:

- `crates/roko-core/src/config/schema.rs`
- `crates/roko-cli/src/config.rs`

The second file matters because CLI config uses `ProviderLayer` and `ModelProfileLayer` for layered merge. Do not add a field to only one schema.

## Step 3: Register the adapter

File: `crates/roko-agent/src/provider/mod.rs`

Wire the new adapter into the static factory:

```rust
pub mod your_provider;

pub use your_provider::YourProviderAdapter;

static YOUR_PROVIDER_ADAPTER: YourProviderAdapter = YourProviderAdapter;

pub fn adapter_for_kind(kind: ProviderKind) -> &'static dyn ProviderAdapter {
    match kind {
        ProviderKind::OpenAiCompat => &OPENAI_COMPAT_ADAPTER,
        ProviderKind::ClaudeCli => &CLAUDE_CLI_ADAPTER,
        ProviderKind::AnthropicApi => &ANTHROPIC_API_ADAPTER,
        ProviderKind::CursorAcp => &CURSOR_ACP_ADAPTER,
        ProviderKind::PerplexityApi => &PERPLEXITY_ADAPTER,
        ProviderKind::GeminiApi => &GEMINI_ADAPTER,
        ProviderKind::YourProviderApi => &YOUR_PROVIDER_ADAPTER,
    }
}
```

This is the path used by `create_agent_for_model()`, which is the normal config-driven entrypoint:

1. `resolve_model()` picks the model/profile/provider
2. `create_agent_for_model()` calls `adapter_for_kind(resolved.provider_kind)`
3. the adapter constructs the concrete agent

## Step 4: Add model wiring, optional translator support, and tests

### Config entry

After the new kind exists, users configure it the same way as other providers:

```toml
[providers.your_provider]
kind = "your_provider_api"
base_url = "https://api.example.com"
api_key_env = "YOUR_PROVIDER_API_KEY"

[models.your-model]
provider = "your_provider"
slug = "your-model-v1"
context_window = 200000
supports_tools = true
tool_format = "your_provider_native"
```

The provider ID (`your_provider`) is just the config table key. The protocol family is the `kind`.

### Optional translator step

Only do this if the provider has a new tool-call wire format.

Relevant files:

- `crates/roko-agent/src/translate/mod.rs`
- `crates/roko-agent/src/translate/capability.rs`
- `crates/roko-agent/src/translate/your_provider.rs`

Gemini is the current example of a provider-native tool format:

- `gemini_native` is modeled as a custom `tool_format`
- `crates/roko-agent/src/translate/gemini.rs` implements the translator

Minimal shape:

```rust
#[derive(Debug, Default, Clone, Copy)]
pub struct YourProviderTranslator;

impl Translator for YourProviderTranslator {
    fn format(&self) -> ToolFormat {
        ToolFormat::Custom("your_provider_native".to_string())
    }

    // implement render_tools / parse_calls / render_results
}
```

Then either:

- select it centrally from `translate/capability.rs` based on `tool_format`, or
- use it directly inside your provider-specific agent if that keeps the code cleaner

If the provider uses an existing tool shape such as `openai_json` or `anthropic_blocks`, do not add a new translator.

### Tests

At minimum, add:

- a unit test beside the adapter module for `kind()` and error classification
- an agent-construction test that proves the adapter returns the right concrete agent
- an HTTP or protocol integration test under `crates/roko-agent/tests/` that captures a real request/response shape

Good examples:

- `crates/roko-agent/tests/gemini_integration.rs`
- `crates/roko-agent/tests/provider_integration.rs`
- tests beside each adapter module under `crates/roko-agent/src/provider/`

## Verification

After the code path is wired:

```bash
cargo check --workspace
cargo test --workspace --no-run
```

Then smoke-test with a real config entry:

```bash
export YOUR_PROVIDER_API_KEY="..."
roko run --model your-model "Reply with the single word: ok"
```

If you are running the HTTP server, inspect the loaded provider and model registry:

```bash
roko serve --port 9090
curl http://127.0.0.1:9090/api/providers
curl http://127.0.0.1:9090/api/models
```

## Common mistakes

- forcing a non-OpenAI provider into `openai_compat` instead of adding a real adapter
- adding a `ProviderKind` but forgetting `label()`
- adding new config fields only in `roko-core` and not mirroring them in `roko-cli`
- adding a slug heuristic when config-driven resolution is enough
- introducing a new translator even though the provider already matches an existing tool format
- skipping adapter registration in `crates/roko-agent/src/provider/mod.rs`
