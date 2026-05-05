# Agent Cell and Provider System

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). How an Agent is instantiated as a Cell specialization, how the provider registry acts as a Route Cell selecting among 8+ LLM backends, and how the unified factory function bridges config-driven model resolution to concrete Agent construction.

---

## 1. The Agent as a Cell Specialization

The spec defines an Agent as `Space + Extensions + Memory + adaptive clock + vitality`. At the implementation level, instantiating an Agent means constructing a Cell that conforms to a specific subset of the 9 protocols and wiring it into the runtime.

The `Agent` trait in code occupies the **act** step of the universal loop (query -> score -> route -> compose -> act -> verify -> write -> react). It is deliberately kept separate from the six protocol traits (Store, Score, Verify, Route, Compose, React) because it violates their four core properties:

| Protocol trait property | Agent behavior |
|---|---|
| **Synchronous** | Agents are `async` (subprocess spawn, HTTP calls, network waits) |
| **Deterministic** | LLMs are stochastic; same prompt yields different outputs |
| **Side-effect-free** | Agents edit files, run commands, mutate the filesystem |
| **Single Signal** | A single agent run produces a stream of intermediate Signals |

This separation is a key architectural decision grounded in the CoALA cognitive architecture (Sumers et al., 2023, arXiv:2309.02427): perception and reasoning (the protocol traits) are separated from action execution (the Agent).

### Agent trait as Cell interface

```rust
/// The Agent trait maps to Cell semantics:
///   Cell::id()           -> agent.name() + model + role
///   Cell::input_schema() -> Signal<Kind::Prompt> | Signal<Kind::Task>
///   Cell::output_schema()-> AgentResult { output: Signal, trace: Vec<Signal>, usage: Usage }
///   Cell::protocols()    -> Connect (external LLM), Store (episode logging)
///   Cell::cost_estimate()-> from ModelProfile cost rates * estimated tokens
#[async_trait]
pub trait Agent: Send + Sync {
    /// Run the agent against the input Signal.
    async fn run(&self, input: &Signal, ctx: &Context) -> AgentResult;

    /// Human-readable name for logs/metrics.
    fn name(&self) -> &str;

    /// Whether this agent emits a streaming trace (many Signals) or single output.
    fn supports_streaming(&self) -> bool { false }
}
```

The `AgentResult` captures everything a Cell execution produces:

```rust
pub struct AgentResult {
    /// Primary output Signal (Kind::AgentOutput).
    pub output: Signal,
    /// Intermediate Signals emitted during the run (tool calls, diffs, status).
    /// Ordered chronologically -- this is the agent's trace.
    pub trace: Vec<Signal>,
    /// Token usage + cost metrics.
    pub usage: Usage,
    /// Whether the run succeeded (non-zero exit / connection error = false).
    /// Even failed runs produce useful diagnostic output.
    pub success: bool,
}
```

Key design choice: `AgentResult` wraps success/failure as a boolean flag rather than using `Result<T, E>`, because failed agent runs still produce output needed for logging, episode recording, retry decisions, and gate feedback.

### Usage tracking as economic observability

Every agent execution produces a `Usage` record that feeds into three downstream systems:

```rust
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,   // Anthropic prompt caching
    pub cache_write_tokens: u64,
    pub cost_usd: f64,            // Computed from ModelProfile cost rates
    pub duration_ms: u64,
    pub model: String,            // Which model was used (for cost attribution)
}
```

| Consumer | What it reads | Purpose |
|---|---|---|
| **EpisodeLogger** | All fields | Episode recording to `.roko/episodes.jsonl` |
| **Efficiency tracker** | `cost_usd`, `duration_ms`, `tokens` | Efficiency events to `.roko/learn/efficiency.jsonl` |
| **CascadeRouter** | `cost_usd`, success flag | Learning signal for adaptive model routing |

This is the **Observe protocol** applied to agent execution: every run produces observation Signals without mutating the agent's state.

---

## 2. Concrete Agent Implementations

Roko ships 7 agent implementations, each targeting a different backend protocol:

| Implementation | Backend | Protocol | Tool loop |
|---|---|---|---|
| `ClaudeCliAgent` | `claude` CLI binary | Stream-JSON over subprocess pipes | **Internal** (CLI drives its own tool loop) |
| `ClaudeAgent` | Anthropic Messages API | HTTP JSON (content blocks) | **External** (Roko's ToolLoop drives it) |
| `OpenAiAgent` | OpenAI Chat Completions | HTTP JSON | **External** |
| `OllamaAgent` | Ollama `/api/chat` | HTTP JSON | **External** |
| `CursorAgent` | Cursor Agent Client Protocol | JSON-RPC | **Internal** |
| `ExecAgent` | Any CLI binary | stdin/stdout | **External** |
| `MockAgent` | In-memory | Deterministic | N/A (test double) |

The **Internal vs External** tool loop distinction is critical: `ClaudeCliAgent` sends a single prompt and the CLI handles multi-turn tool calling internally. HTTP-based agents go through Roko's `ToolLoop` (see [tool-loop-and-mcp.md](tool-loop-and-mcp.md)), which drives the iterative perceive-think-act-verify cycle.

---

## 3. The Provider Registry as a Route Cell

The provider system is a three-layer Route Cell that resolves a model name to a concrete Agent instance. In unified terms, it implements the Route protocol: given a task requirement (input Signal), select the best Cell (provider + model) to handle it.

### Layer 1: Provider entries (where to send requests)

Providers are configured in `roko.toml` as `[providers.*]` entries:

```toml
[providers.anthropic]
kind = "anthropic_api"
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"
timeout_ms = 120000
max_concurrent = 5

[providers.openrouter]
kind = "openai_compat"
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"
extra_headers = { "HTTP-Referer" = "https://roko.dev" }

[providers.local-claude]
kind = "claude_cli"
command = "claude"
timeout_ms = 300000
```

The `ProviderConfig` struct:

```rust
pub struct ProviderConfig {
    pub kind: ProviderKind,                          // Protocol family (dispatch key)
    pub base_url: Option<String>,                    // API endpoint root (HTTP providers)
    pub api_key_env: Option<String>,                 // Env var name (never stored in config)
    pub command: Option<String>,                     // Binary name (CLI providers)
    pub args: Option<Vec<String>>,                   // Default CLI arguments
    pub timeout_ms: Option<u64>,                     // Per-request timeout
    pub extra_headers: Option<HashMap<String, String>>, // Injected into HTTP requests
    pub max_concurrent: Option<u32>,                 // Concurrency limiter
}
```

### Layer 2: Model entries (what to send)

Models are configured as `[models.*]` entries pointing at providers:

```toml
[models.claude-opus]
provider = "anthropic"
slug = "claude-opus-4-6"
context_window = 200000
max_output = 32768
supports_tools = true
supports_thinking = true
tool_format = "anthropic_blocks"
cost_input_per_m = 15.00
cost_output_per_m = 75.00
```

The `ModelProfile` struct carries 20+ capability flags that drive adapter behavior:

```rust
pub struct ModelProfile {
    pub provider: String,            // Key into [providers.*]
    pub slug: String,                // Model ID sent to the API
    pub context_window: u64,         // Token window size
    pub max_output: Option<u64>,     // Output-token cap
    pub supports_tools: bool,        // Tool calling
    pub supports_thinking: bool,     // Reasoning/thinking output
    pub supports_vision: bool,       // Image inputs
    pub supports_web_search: bool,   // Built-in web search
    pub supports_mcp_tools: bool,    // MCP tool protocol
    pub tool_format: String,         // Wire format: "openai_json" | "anthropic_blocks"
    pub cost_input_per_m: Option<f64>,   // $/M input tokens
    pub cost_output_per_m: Option<f64>,  // $/M output tokens
    // ... (full list in 02-agents/01-provider-registry.md)
}
```

### Layer 3: ProviderKind dispatch (how to talk)

The `ProviderKind` enum is the primary dispatch key:

```rust
pub enum ProviderKind {
    AnthropicApi,   // Anthropic Messages API (content blocks, thinking, caching)
    ClaudeCli,      // `claude` CLI subprocess (stream-JSON protocol)
    OpenAiCompat,   // OpenAI chat completions (de facto standard for 6+ providers)
    CursorAcp,      // Cursor Agent Client Protocol (JSON-RPC)
}
```

`OpenAiCompat` handles the majority of providers because the OpenAI `/v1/chat/completions` format has become the universal LLM wire protocol. ZhipuAI, Moonshot, DeepSeek, OpenRouter, Perplexity, and Gemini all expose this protocol. Provider-specific behavior (Perplexity citations, Gemini grounding) is captured in `ModelProfile` flags and `ResponseMetadata` extension fields.

### Model resolution (the Route decision)

The `resolve_model()` function implements a two-phase resolution that bridges the old heuristic world and the new config-driven world:

```rust
pub fn resolve_model(config: &RokoConfig, model_key: &str) -> ResolvedModel {
    // Phase 1: Try config registry (explicit [models.*] entries)
    if let Some(profile) = config.models.get(model_key) {
        let provider_config = config.providers.get(&profile.provider);
        return ResolvedModel { slug, provider_kind, profile, provider_config };
    }

    // Phase 2: Fall back to slug heuristic
    //   "claude-*" -> AnthropicApi/ClaudeCli
    //   "gpt-*" / "o3-*" -> OpenAiCompat
    //   "ollama/*" -> OpenAiCompat (Ollama endpoint)
    let backend = AgentBackend::from_model(model_key);
    ResolvedModel { slug: model_key, provider_kind: inferred, ... }
}
```

This means users who pass bare model slugs (e.g., `"claude-opus-4-6"`) continue working via heuristics, while users with `[providers.*]` and `[models.*]` entries get full control.

---

## 4. The Unified Factory: `create_agent_for_model`

The factory function is the single entry point for config-driven Agent construction. In Cell terms, it is a **Connect protocol** operation: resolve the external system, validate the connection, and return a configured Cell.

```rust
pub fn create_agent_for_model(
    config: &RokoConfig,
    model_key: &str,
    options: AgentOptions,
) -> Result<Box<dyn Agent>, AgentCreationError> {
    // 1. Resolve model -> (slug, provider_kind, profile, provider_config)
    let resolved = resolve_model(config, model_key);

    // 2. Look up the static adapter for this protocol family
    let adapter = adapter_for_kind(resolved.provider_kind);

    // 3. Construct the agent via the adapter
    adapter.create_agent(&provider_config, &profile, &options)
}
```

### ProviderAdapter trait (the Connect protocol Cell)

Each protocol family has a static adapter that knows how to construct agents:

```rust
pub trait ProviderAdapter: Send + Sync {
    /// Which protocol family this adapter handles.
    fn kind(&self) -> ProviderKind;

    /// Construct a configured Agent from provider config + model profile + options.
    fn create_agent(
        &self,
        provider: &ProviderConfig,
        model: &ModelProfile,
        options: &AgentOptions,
    ) -> Result<Box<dyn Agent>, AgentCreationError>;

    /// Classify an error response into a canonical error type.
    fn classify_error(&self, status: u16, body: &Value) -> ProviderError;
}
```

The adapters are static dispatch (no dynamic registry, no allocations):

```rust
static ANTHROPIC_API_ADAPTER: AnthropicApiAdapter = AnthropicApiAdapter;
static CLAUDE_CLI_ADAPTER: ClaudeCliAdapter = ClaudeCliAdapter;
static CURSOR_ACP_ADAPTER: CursorAcpAdapter = CursorAcpAdapter;
static OPENAI_COMPAT_ADAPTER: OpenAiCompatAdapter = OpenAiCompatAdapter;

pub fn adapter_for_kind(kind: ProviderKind) -> &'static dyn ProviderAdapter {
    match kind {
        ProviderKind::OpenAiCompat => &OPENAI_COMPAT_ADAPTER,
        ProviderKind::ClaudeCli    => &CLAUDE_CLI_ADAPTER,
        ProviderKind::AnthropicApi => &ANTHROPIC_API_ADAPTER,
        ProviderKind::CursorAcp    => &CURSOR_ACP_ADAPTER,
    }
}
```

Adding a new protocol family requires: (1) add `ProviderKind` variant, (2) implement `ProviderAdapter`, (3) add match arm. The exhaustive match ensures no variant is forgotten.

### AgentOptions (runtime parameters)

```rust
pub struct AgentOptions {
    pub timeout_ms: Option<u64>,
    pub system_prompt: Option<String>,
    pub tools: Option<String>,         // CSV tool allowlist
    pub mcp_config: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub extra_args: Vec<String>,
    pub effort: Option<String>,        // Thinking effort level
    pub bare_mode: bool,               // Skip interactive features
    pub dangerously_skip_permissions: bool,
    pub name: String,
}
```

---

## 5. Error Classification and Retry

Each adapter normalizes provider-specific errors to canonical variants:

```rust
pub enum ProviderError {
    RateLimit { retry_after_ms: Option<u64> },  // 429 from any provider
    AuthFailure,                                 // Bad API key
    Timeout,                                     // Request timed out
    ServerError(u16),                            // 5xx from provider
    ContentPolicy,                               // Content filter triggered
    ContextOverflow,                             // Prompt exceeds context window
    ModelNotFound,                               // Model slug not recognized
    Other(String),
}
```

The retry policy is deterministic and provider-agnostic:

| Error | Action | Rationale |
|---|---|---|
| `RateLimit` | Wait and retry same provider | Delay from `retry-after` header or 5s default |
| `AuthFailure` | Skip (terminal) | Retrying won't fix a bad key |
| `Timeout` / `ServerError` | Try fallback provider | Provider may be temporarily overloaded |
| `ContentPolicy` | Skip (terminal) | Prompt triggered content filter |
| `ContextOverflow` | Try with smaller context | Prune history and retry |
| `ModelNotFound` | Try fallback | Model may not be available on this provider |

### Agent construction errors

```rust
pub enum AgentCreationError {
    MissingApiKey(String),    // Env var not set
    MissingConfig(String),    // No [providers.*] or [models.*] entry
    InvalidKind(ProviderKind), // Adapter mismatch
}
```

These are configuration errors caught at construction time, not runtime failures.

---

## 6. Provider Capability Matrix

Each protocol family supports a different feature subset. This matrix drives adapter behavior:

| Capability | Anthropic API | Claude CLI | OpenAI Compat | Cursor ACP |
|---|---|---|---|---|
| Streaming | SSE | Stream-JSON | SSE | JSON-RPC |
| Tool calling | Content blocks | `--tools` flag | Function calling | JSON-RPC |
| Extended thinking | `thinking` param (1K-128K budget) | `--effort` flag | o3/o4-mini reasoning | N/A |
| Structured output | Tool use schemas | N/A | `json_schema` constrained decoding | N/A |
| Prompt caching | Server-side, 90% savings, 5min-1hr TTL | Built-in | Auto-caching, 50% discount | N/A |
| Vision | Content blocks with `image` type | `--input` flag | `image_url` in messages | N/A |
| MCP | Native (creator of MCP) | `--mcp-config` passthrough | Not native | N/A |
| Max context | 200K | 200K | 1M (GPT-4.1) | Model-dependent |
| Max output | 128K (with thinking) | Model-dependent | 100K (o3) | Model-dependent |

---

## 7. Effective Config Merge

The config system implements a priority merge:

1. **User-specified** `[providers.*]` / `[models.*]` (highest priority)
2. **Built-in model profiles** from `profile_for_model()` in roko-core
3. **Slug-heuristic fallback** (lowest priority)

This means Roko ships with baseline providers and models that work out of the box while allowing full user override.

---

## 8. Mori-Diffs Reality

The mori-diff at [01-AGENT-DISPATCH.md](../../mori-diffs/01-AGENT-DISPATCH.md) identifies the central gap: **the runner v2 hardcodes Claude CLI as the only agent backend**. The runner's `agent_stream.rs` spawns `claude` directly via `tokio::process::Command`. The runner's `RunConfig` embeds `claude_program: PathBuf`. The stream parser only understands Claude's `stream-json` protocol.

Meanwhile, `create_agent_for_model()` and the full provider adapter system exist but are only reachable from `orchestrate.rs`, not from the runner event loop. The planned `AgentDispatcher` module (`crates/roko-cli/src/dispatch/`) would bridge this gap, replacing the hardcoded spawn with provider-agnostic dispatch through the adapter layer.

---

## What This Enables

- **Single factory function** for all 8+ backends -- callers never import provider-specific types
- **Config-driven model binding** -- add new providers via TOML without code changes
- **Automatic capability matching** -- Route protocol selects best model for task requirements
- **Normalized error handling** -- same retry logic regardless of which provider failed
- **Cost observability** -- every agent execution reports `Usage` for learning and budgeting

## Feedback Loops

1. **CascadeRouter -> Provider selection**: Agent execution outcomes feed `report_outcome()` which updates the CascadeRouter's learned model preferences. Over time, the Route decision improves.
2. **Usage -> Budget enforcement**: `TurnBudget` reads `Usage.cost_usd` to enforce per-role spending limits. Budget pressure flows back through vitality (behavioral phase modulation).
3. **Error classification -> Retry policy**: `classify_error()` normalizes provider failures into retry actions, creating a feedback loop between runtime errors and dispatch strategy.
4. **Effective config merge -> Capability evolution**: As new providers are added to `[providers.*]`, the capability matrix expands, giving the Route Cell more candidates.

## Open Questions

1. **Agent trait vs Cell trait unification**: The `Agent` trait and `Cell` trait exist as separate abstractions. Should `Agent` explicitly implement `Cell` with protocol conformance declared? This would allow the Engine to schedule agent executions through the same Graph machinery as other Cells.

2. **Provider registry dynamism**: The adapter dispatch is static (exhaustive match on `ProviderKind`). Should new protocol families be registerable at runtime via a plugin mechanism, or is the compile-time guarantee worth the constraint?

3. **Runner v2 migration timeline**: The runner still hardcodes Claude CLI. The `AgentDispatcher` design exists in the mori-diff but is not implemented. Until it ships, provider-agnostic dispatch only works through `orchestrate.rs`, not through the runner event loop.

4. **API key lifecycle**: `resolve_api_key()` reads env vars at construction time. Should there be a refresh mechanism for rotated keys, or is process restart sufficient?

---

## Citations

1. Sumers, T. R. et al. (2023). "Cognitive Architectures for Language Agents." arXiv:2309.02427.
2. `crates/roko-agent/src/agent.rs` -- Agent trait and AgentResult.
3. `crates/roko-agent/src/provider/mod.rs` -- ProviderAdapter, create_agent_for_model, adapter_for_kind.
4. `crates/roko-core/src/config/schema.rs` -- ProviderConfig, ModelProfile.
5. `crates/roko-core/src/agent.rs` -- ProviderKind, AgentBackend, resolve_model.
6. `tmp/mori-diffs/01-AGENT-DISPATCH.md` -- Runner dispatch gap analysis.
