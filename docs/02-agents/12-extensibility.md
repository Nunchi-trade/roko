# 12 — Extensibility and SDK

> Sub-doc 12 of **02-agents** · Roko Documentation
>
> This document describes how to add new agent backends, new provider
> adapters, new tool translators, and new LlmBackend implementations.
> It covers the 8-step domain plugin process and the extensibility
> architecture.


> **Implementation**: Shipping

---

## Extensibility Points

Roko's agent system has five extensibility points, each with a clear trait
or registration mechanism:

| Extension point | Trait/Interface | Location | Effort |
|---|---|---|---|
| New agent backend | `Agent` | `roko-agent/src/agent.rs` | Medium |
| New provider adapter | `ProviderAdapter` | `roko-agent/src/provider/` | Low |
| New tool translator | `Translator` | `roko-agent/src/translate/` | Medium |
| New LLM backend | `LlmBackend` | `roko-agent/src/tool_loop/` | Low |
| New tool handler | `ToolHandler` | `roko-core/src/tool/` | Low |

---

## Adding a New Provider

The simplest extension. If the provider speaks an existing protocol (most
likely OpenAI-compatible chat completions), no code is needed — just config:

### Step 1: Add provider entry in `roko.toml`

```toml
[providers.my-provider]
kind = "openai_compat"
base_url = "https://api.my-provider.com/v1"
api_key_env = "MY_PROVIDER_API_KEY"
timeout_ms = 60000
```

### Step 2: Add model entries

```toml
[models.my-model-large]
provider = "my-provider"
slug = "my-model-large"
context_window = 128000
max_output = 4096
supports_tools = true
tool_format = "openai_json"
cost_input_per_m = 2.00
cost_output_per_m = 8.00

[models.my-model-small]
provider = "my-provider"
slug = "my-model-small"
context_window = 32000
supports_tools = true
tool_format = "openai_json"
cost_input_per_m = 0.50
cost_output_per_m = 2.00
```

### Step 3: Use it

```bash
cargo run -p roko-cli -- run "Hello" --model my-model-large
```

The `create_agent_for_model` factory resolves the model, finds the provider,
sees `kind = "openai_compat"`, and uses the `OpenAiCompatAdapter` to construct
an `OpenAiAgent`. No code changes needed.

---

## Adding a New Protocol Family (ProviderAdapter)

If the provider uses a protocol that doesn't fit any existing adapter, you
need a new `ProviderAdapter` implementation:

### Step 1: Add a ProviderKind variant

In `crates/roko-core/src/agent.rs`:

```rust
pub enum ProviderKind {
    AnthropicApi,
    ClaudeCli,
    OpenAiCompat,
    CursorAcp,
    MyProtocol,  // NEW
}
```

### Step 2: Implement ProviderAdapter

In `crates/roko-agent/src/provider/my_protocol.rs`:

```rust
pub struct MyProtocolAdapter;

impl ProviderAdapter for MyProtocolAdapter {
    fn kind(&self) -> ProviderKind {
        ProviderKind::MyProtocol
    }

    fn create_agent(
        &self,
        provider: &ProviderConfig,
        model: &ModelProfile,
        options: &AgentOptions,
    ) -> Result<Box<dyn Agent>, AgentCreationError> {
        // Construct your agent from the config
        let base_url = provider.base_url.as_deref()
            .ok_or_else(|| AgentCreationError::MissingConfig("base_url".into()))?;
        let api_key = provider.resolve_api_key()
            .ok_or_else(|| AgentCreationError::MissingApiKey(
                provider.api_key_env.clone().unwrap_or_default()
            ))?;

        Ok(Box::new(MyProtocolAgent::new(base_url, &api_key, &model.slug)))
    }

    fn classify_error(&self, status: u16, body: &Value) -> ProviderError {
        // Map provider-specific errors to canonical variants
        match status {
            429 => ProviderError::RateLimit { retry_after_ms: None },
            401 | 403 => ProviderError::AuthFailure,
            500..=599 => ProviderError::ServerError(status),
            _ => ProviderError::Other(format!("status {status}")),
        }
    }
}
```

### Step 3: Register in adapter_for_kind

In `crates/roko-agent/src/provider/mod.rs`:

```rust
static MY_PROTOCOL_ADAPTER: MyProtocolAdapter = MyProtocolAdapter;

pub fn adapter_for_kind(kind: ProviderKind) -> &'static dyn ProviderAdapter {
    match kind {
        ProviderKind::OpenAiCompat => &OPENAI_COMPAT_ADAPTER,
        ProviderKind::ClaudeCli    => &CLAUDE_CLI_ADAPTER,
        ProviderKind::AnthropicApi => &ANTHROPIC_API_ADAPTER,
        ProviderKind::CursorAcp    => &CURSOR_ACP_ADAPTER,
        ProviderKind::MyProtocol   => &MY_PROTOCOL_ADAPTER,
    }
}
```

The exhaustive `match` ensures the compiler catches any unregistered variant.

---

## Adding a New LlmBackend

If your provider supports tool calling and you want to use Roko's ToolLoop
(rather than the provider's internal loop), implement `LlmBackend`:

```rust
pub struct MyBackend {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

#[async_trait]
impl LlmBackend for MyBackend {
    async fn send_turn(
        &self,
        messages: &[serde_json::Value],
        tools: &RenderedTools,
    ) -> Result<BackendResponse, LlmError> {
        let body = build_request_body(&self.model, messages, tools);
        let response = self.client
            .post(&format!("{}/chat", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send().await
            .map_err(|e| LlmError::Network(e.to_string()))?;

        let json: Value = response.json().await
            .map_err(|e| LlmError::Backend(e.to_string()))?;

        Ok(BackendResponse::Json(json))
    }
}
```

Then wire it into the ToolLoop:

```rust
let backend = Arc::new(MyBackend { ... });
let translator = Arc::new(OpenAiTranslator);  // If OpenAI-compatible
let dispatcher = Arc::new(ToolDispatcher::new(registry, resolver));
let tool_loop = ToolLoop::new(translator, dispatcher, backend);

let output = tool_loop.run(system_prompt, user_prompt, &tools, &ctx).await;
```

The existing `OllamaLlmBackend` at `crates/roko-agent/src/ollama_backend.rs`
is a working reference implementation.

---

## Adding a New Translator

If a model uses a wire format not covered by the four existing translators:

```rust
pub struct MyFormatTranslator;

impl Translator for MyFormatTranslator {
    fn format(&self) -> ToolFormat {
        ToolFormat::MyFormat  // Add to the ToolFormat enum first
    }

    fn render_tools(&self, tools: &[ToolDef]) -> RenderedTools {
        // Convert canonical ToolDefs to your format
        let json = tools.iter().map(|t| {
            json!({
                "tool_name": t.name,
                "tool_desc": t.description,
                "params": t.schema,
            })
        }).collect::<Vec<_>>();
        RenderedTools::JsonArray(json!(json))
    }

    fn parse_calls(&self, response: &BackendResponse) -> Result<Vec<ToolCall>, TranslatorError> {
        // Extract tool calls from your format
        let BackendResponse::Json(ref v) = *response else {
            return Ok(vec![]);
        };
        // ... parse your format ...
        Ok(calls)
    }

    fn render_results(&self, results: &[(ToolCall, ToolResult)]) -> RenderedResults {
        // Format results for the next turn
        RenderedResults::JsonMessages(json!([...]))
    }
}
```

Then register it in `translator_for` in `translate/capability.rs`.

---

## 8-Step Domain Plugin Process

The refactoring PRD §05-agent-types defines an 8-step process for adding
a new domain-specific agent type:

1. **Define the role** — Add a variant to `AgentRole` with default tier,
   budget, and permissions.
2. **Create the role template** — Write a system prompt template in
   `roko-compose/src/templates/`.
3. **Register tools** — Define domain-specific `ToolDef` entries and
   `ToolHandler` implementations.
4. **Configure the model** — Add `[models.*]` entries for models suited
   to the domain.
5. **Wire the provider** — Ensure the provider config exists for the
   model's backend.
6. **Set gate criteria** — Define domain-specific gate checks (e.g., for
   a Solidity agent: compile with `forge build`, test with `forge test`).
7. **Add to the router** — Register the role's default tier in the
   CascadeRouter so model routing works from the first run.
8. **Test end-to-end** — Run `roko run "<domain prompt>"` and verify the
   full pipeline: prompt assembly → agent execution → gate validation →
   persistence.

---

## Adding a New LlmBackend: Full Example

The refactoring PRD §05-agent-types documents the process for adding a new
`LlmBackend` implementation:

1. Add a struct implementing `LlmBackend::send_turn()`.
2. Add a module under `roko-agent/src/` (e.g., `my_backend.rs`).
3. Re-export from `lib.rs`.
4. Wire into the provider adapter's `create_agent()` method.
5. Add an integration test with a mock HTTP server (see `provider/mod.rs`
   tests for the pattern).
6. Add a `[models.*]` entry in `roko.toml` pointing at a `[providers.*]`
   entry with the correct `kind`.

---

## Event System: EventSource and FeedbackCollector

The refactoring PRD §10-developer-guide describes two additional plugin
interfaces for agent integration:

### EventSource

Agents can emit domain-specific events that the learning subsystem captures:

```rust
pub trait EventSource: Send + Sync {
    fn events(&self) -> Vec<DomainEvent>;
}
```

These events feed into the efficiency tracking pipeline and the episode
logger, providing domain-specific signal for the CascadeRouter's
learning loop.

### FeedbackCollector

Agents can collect feedback from their execution for future improvement:

```rust
pub trait FeedbackCollector: Send + Sync {
    fn collect(&self, result: &AgentResult) -> Vec<FeedbackSignal>;
}
```

Feedback signals are persisted alongside episodes and used by the adaptive
gate thresholds to adjust pass criteria.

---

## Citations

1. Refactoring PRD §05-agent-types — 8-step domain plugin process,
   LlmBackend addition process.
2. Refactoring PRD §10-developer-guide — EventSource, FeedbackCollector,
   plugin system.
3. `crates/roko-agent/src/provider/mod.rs` — ProviderAdapter trait,
   adapter_for_kind dispatch.
4. `crates/roko-agent/src/tool_loop/mod.rs` — LlmBackend trait.
5. `crates/roko-agent/src/translate/mod.rs` — Translator trait.
6. `crates/roko-agent/src/ollama_backend.rs` — Reference LlmBackend impl.
