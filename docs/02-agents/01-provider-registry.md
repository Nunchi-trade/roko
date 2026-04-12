# 01 — Provider Registry

> Sub-doc 01 of **02-agents** · Roko Documentation
>
> This document describes the config-driven provider registry that maps model
> names to providers and providers to protocol families. It covers the TOML
> schema, the `ProviderConfig` and `ModelProfile` structs, model resolution,
> and the effective-config merge logic.

---

## Overview

The provider registry is Roko's config-driven layer for binding model names to
concrete API endpoints. Before this layer existed, agent backends were inferred
from model slug heuristics — if the slug starts with `claude-`, spawn the Claude
CLI; if it starts with `ollama/`, use the Ollama HTTP API; otherwise fall back
to Codex. This heuristic-based approach (still present as `AgentBackend::from_model`
at `crates/roko-core/src/agent.rs:109`) cannot handle third-party providers like
ZhipuAI (GLM), Moonshot (Kimi), Perplexity, or Gemini, because their slugs don't
follow the convention of any built-in backend.

The provider registry solves this with two TOML tables:

- **`[providers.*]`** — Defines *where* to send requests (protocol, URL, auth)
- **`[models.*]`** — Defines *what* to send (model slug, capabilities, cost)

A model entry points at a provider entry via the `provider` field. At resolve
time, Roko looks up the model, finds the provider, determines the protocol
family (`ProviderKind`), and uses the appropriate adapter to construct a
configured `Agent` instance.

---

## TOML Schema

### Provider entries

```toml
[providers.anthropic]
kind = "anthropic_api"
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"
timeout_ms = 120000
max_concurrent = 5

[providers.zai]
kind = "openai_compat"
base_url = "https://open.bigmodel.cn/api/paas/v4"
api_key_env = "ZHIPUAI_API_KEY"
timeout_ms = 60000
extra_headers = { "X-Request-Source" = "roko" }

[providers.openrouter]
kind = "openai_compat"
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"
extra_headers = { "HTTP-Referer" = "https://roko.dev", "X-Title" = "Roko" }

[providers.local-claude]
kind = "claude_cli"
command = "claude"
timeout_ms = 300000

[providers.cursor]
kind = "cursor_acp"
command = "cursor-agent"
```

### Model entries

```toml
[models.glm-5-1]
provider = "zai"
slug = "glm-5.1"
context_window = 200000
max_output = 131072
supports_tools = true
supports_thinking = true
supports_web_search = true
tool_format = "openai_json"
cost_input_per_m = 1.40
cost_output_per_m = 4.40

[models.claude-opus]
provider = "anthropic"
slug = "claude-opus-4-6"
context_window = 200000
max_output = 32768
supports_tools = true
supports_thinking = true
supports_vision = true
tool_format = "anthropic_blocks"
cost_input_per_m = 15.00
cost_output_per_m = 75.00

[models.sonar-pro]
provider = "perplexity"
slug = "sonar-pro"
context_window = 200000
max_output = 8192
supports_tools = true
supports_search = true
supports_citations = true
tool_format = "openai_json"
cost_input_per_m = 3.00
cost_output_per_m = 15.00
cost_per_request = 0.005
search_context_size = "high"
```

---

## ProviderConfig Struct

Defined at `crates/roko-core/src/config/schema.rs:717`:

```rust
pub struct ProviderConfig {
    /// Protocol family used to talk to the provider.
    pub kind: ProviderKind,
    /// Base URL for HTTP providers.
    pub base_url: Option<String>,
    /// Environment variable name holding the API key.
    pub api_key_env: Option<String>,
    /// Command to spawn for CLI providers.
    pub command: Option<String>,
    /// Arguments passed to the CLI command.
    pub args: Option<Vec<String>>,
    /// Request timeout in milliseconds.
    pub timeout_ms: Option<u64>,
    /// Extra headers to inject on outbound requests.
    pub extra_headers: Option<HashMap<String, String>>,
    /// Maximum concurrent requests allowed for this provider.
    pub max_concurrent: Option<u32>,
}
```

### Field semantics

| Field | Required for | Purpose |
|---|---|---|
| `kind` | All providers | Selects the `ProviderAdapter` (see sub-doc 03) |
| `base_url` | HTTP providers | API endpoint root; the adapter appends the path |
| `api_key_env` | HTTP providers | Env var name; resolved at runtime via `resolve_api_key()` |
| `command` | CLI providers | Binary name (e.g., `"claude"`, `"cursor-agent"`) |
| `args` | CLI providers | Default arguments appended to every invocation |
| `timeout_ms` | All providers | Per-request timeout; overridable per-agent at spawn |
| `extra_headers` | HTTP providers | Injected into every outbound request |
| `max_concurrent` | All providers | Concurrency limiter for the provider's semaphore |

The `resolve_api_key()` method reads the named environment variable at
runtime, so API keys never appear in the TOML file:

```rust
impl ProviderConfig {
    pub fn resolve_api_key(&self) -> Option<String> {
        self.api_key_env
            .as_ref()
            .and_then(|env_name| std::env::var(env_name).ok())
    }
}
```

---

## ProviderKind Enum

Defined at `crates/roko-core/src/agent.rs:34`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Anthropic Messages API over HTTP.
    AnthropicApi,
    /// `claude` CLI subprocess protocol.
    ClaudeCli,
    /// OpenAI chat completions-compatible HTTP APIs.
    OpenAiCompat,
    /// Cursor Agent Client Protocol.
    CursorAcp,
}
```

This enum is the **primary dispatch key** for the entire provider system.
When the factory function `create_agent_for_model` needs to construct an
agent, it passes the `ProviderKind` to `adapter_for_kind()`, which returns
the static adapter instance for that protocol family. See sub-doc 03
(Provider Adapters) for the dispatch table.

The four variants cover the protocol families currently in production:

- **`AnthropicApi`** — Anthropic's native Messages API with `content` blocks,
  thinking output, and prompt caching.
- **`ClaudeCli`** — Anthropic's `claude` CLI binary, which drives its own
  tool loop internally (stream-JSON protocol over subprocess pipes).
- **`OpenAiCompat`** — The OpenAI chat completions API, which is the de facto
  standard for third-party providers. ZhipuAI (GLM), Moonshot (Kimi),
  DeepSeek, OpenRouter, Perplexity, and Gemini all expose this protocol.
- **`CursorAcp`** — Cursor's Agent Client Protocol, a JSON-RPC protocol
  for communicating with Cursor's agent runtime.

### Why `OpenAiCompat` handles most providers

The OpenAI chat completions format (`/v1/chat/completions`) has become the
universal LLM wire protocol. Most providers implement it directly or provide
a compatibility layer. This means a single `OpenAiCompatAdapter` can serve
ZhipuAI, Moonshot, DeepSeek, OpenRouter, Perplexity (4 API surfaces but
chat completions for the primary one), and Gemini (native + OpenAI-compat
endpoint). Provider-specific behavior (like Perplexity's `citations` field
or Gemini's `grounding` metadata) is captured in `ModelProfile` capability
flags and `ResponseMetadata` extension fields rather than requiring separate
adapters.

---

## ModelProfile Struct

Defined at `crates/roko-core/src/config/schema.rs:819`:

```rust
pub struct ModelProfile {
    pub provider: String,              // Key into [providers.*]
    pub slug: String,                  // Model ID sent to the API
    pub context_window: u64,           // Token window size (default: 128_000)
    pub max_output: Option<u64>,       // Output-token cap
    pub supports_tools: bool,          // Tool calling (default: true)
    pub supports_thinking: bool,       // Reasoning/thinking output
    pub supports_vision: bool,         // Image inputs
    pub supports_web_search: bool,     // Built-in web search
    pub supports_mcp_tools: bool,      // MCP tool protocol
    pub supports_partial: bool,        // Partial continuation
    pub provider_routing: Option<ProviderRouting>,  // OpenRouter overrides
    pub tool_format: String,           // Wire format for tools (default: "openai_json")
    pub cost_input_per_m: Option<f64>,         // $/M input tokens
    pub cost_output_per_m: Option<f64>,        // $/M output tokens
    pub cost_cache_read_per_m: Option<f64>,    // $/M cache reads
    pub cost_cache_write_per_m: Option<f64>,   // $/M cache writes
    pub max_tools: Option<u32>,                // Degradation threshold
    pub tokenizer_ratio: Option<f64>,          // vs o200k_base
    pub supports_search: bool,         // Grounded search (Perplexity)
    pub supports_citations: bool,      // Response citations
    pub supports_async: bool,          // Async job API (deep research)
    pub is_embedding_model: bool,      // Embedding vs chat
    pub search_context_size: Option<String>,   // "low"/"medium"/"high"
    pub cost_per_request: Option<f64>,         // Per-request fee
}
```

### Capability flags

The `supports_*` flags drive adapter behavior at multiple levels:

- **`supports_tools`** — Whether the adapter includes a `tools` array in the
  request body. If false, the adapter omits tools entirely (useful for
  embedding models or models with degraded tool support).
- **`supports_thinking`** — Whether to parse `reasoning_content` or
  `thinking` blocks from the response. See sub-doc 10 (Format Translation)
  for reasoning extraction.
- **`tool_format`** — Selects the `Translator` implementation: `"openai_json"`,
  `"anthropic_blocks"`, `"ollama_json"`, or `"react_text"`. This is the
  enforcement point for the Meta-Harness principle that tool-call format
  preference is model-specific (see sub-doc 09 for full discussion).
- **`max_tools`** — When set, the adapter truncates the tool array to this
  size. Research shows that some models (notably Qwen3-coder) degrade above
  5 tools when using certain formats.

### Cost metadata

The cost fields (`cost_input_per_m`, `cost_output_per_m`, etc.) feed into:

1. **`Usage` computation** — After each agent run, the `Usage` struct
   multiplies token counts by cost rates to produce `cost_usd`.
2. **Budget enforcement** — The per-role `TurnBudget` checks accumulated
   cost against the ceiling before allowing further turns.
3. **Model routing** — The `CascadeRouter` and `LinUCB` bandit in
   `roko-learn` use cost as one dimension of the Pareto frontier when
   selecting models.

---

## Model Resolution

The `resolve_model` function at `crates/roko-core/src/agent.rs:239` bridges
the old heuristic world and the new config-driven world:

```rust
pub fn resolve_model(config: &RokoConfig, model_key: &str) -> ResolvedModel {
    // 1. Try the config registry first
    if let Some(profile) = config.models.get(model_key) {
        let provider_config = config.providers.get(&profile.provider).cloned();
        let backend = AgentBackend::from_model(&profile.slug);
        let provider_kind = provider_config
            .as_ref()
            .map(|p| p.kind)
            .unwrap_or_else(|| provider_kind_from_backend(backend));
        return ResolvedModel { ... };
    }

    // 2. Fall back to slug heuristic
    let backend = AgentBackend::from_model(model_key);
    ResolvedModel {
        slug: model_key.trim().to_owned(),
        provider_kind: provider_kind_from_backend(backend),
        ...
    }
}
```

The returned `ResolvedModel` carries:

- `model_key` — The original lookup key
- `slug` — The API-wire model ID
- `provider_kind` — Which adapter to use
- `provider_config` — Full provider config (if found)
- `profile` — Full model profile (if found)
- `backend` — Legacy backend inference (for backwards compatibility)

This two-phase resolution means existing users who rely on bare model slugs
(e.g., `"claude-opus-4-6"`) continue to work via the heuristic path, while
users who configure `[providers.*]` and `[models.*]` get full control.

---

## Effective Config Merge

The `RokoConfig` struct provides `effective_providers()` and
`effective_models()` methods that merge built-in defaults with user-provided
config. This means Roko ships with a baseline set of known providers and
models that work out of the box, while users can override any field.

The merge priority is:
1. User-specified `[providers.*]` / `[models.*]` (highest)
2. Built-in model profiles from `profile_for_model()` in `roko-core`
3. Slug-heuristic fallback (lowest)

---

## ProviderRouting (OpenRouter)

The `ProviderRouting` struct enables OpenRouter-specific request shaping:

```rust
pub struct ProviderRouting {
    pub sort: Option<String>,           // "price", "throughput", "latency"
    pub order: Option<Vec<String>>,     // Explicit provider preference
    pub allow_fallbacks: Option<bool>,  // Auto-failover
    pub max_price: Option<f64>,         // Cost ceiling per token
    pub require_parameters: Option<Vec<String>>, // Required provider features
}
```

When a model's `provider_routing` field is set, the `OpenAiCompatAdapter`
injects these as OpenRouter-specific headers or body extensions. See
sub-doc 15 (Provider Integrations) for OpenRouter details.

---

## Citations

1. Implementation plan `modelrouting/02-provider-registry.md` — Full TOML
   schema design, ProviderKind enum, ProviderConfig struct, ModelProfile struct.
2. Implementation plan `modelrouting/01-architecture.md` — Three-layer provider
   system design, why config-driven binding.
3. `crates/roko-core/src/config/schema.rs:717` — ProviderConfig source.
4. `crates/roko-core/src/config/schema.rs:819` — ModelProfile source.
5. `crates/roko-core/src/agent.rs:34` — ProviderKind enum source.
6. `crates/roko-core/src/agent.rs:239` — resolve_model function source.
