# 12 — Extensibility and SDK

> Sub-doc 12 of **02-agents** · Roko Documentation
>
> This document describes how to add new agent backends, new provider
> adapters, new tool translators, and new LlmBackend implementations.
> It covers the 8-step domain plugin process, the four-layer Rust SDK
> surface, and the extensibility architecture.
>
> See also: `../../tmp/refinements/22-developer-ux-rust.md` and
> `../00-architecture/01-naming-and-glossary.md`.


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

## Four-Layer Rust SDK

The SDK is intentionally layered so Rust developers can stop at the
highest level that fits their task.

| Layer | Primary user | Typical entry point | What they own | Where failure should surface |
|---|---|---|---|---|
| One-liner | Application author | `roko::run(...)` | Defaults, model selection, memory path, immediate success path | At the call site, with typed errors |
| Builder | Agent author | `Agent::builder()` | Roles, tools, gates, prompts, memory, configuration | At `.build()`, not first `.send()` |
| Trait impl | Trait implementor | `ProviderAdapter`, `Translator`, `LlmBackend`, `ToolHandler` | Narrow, stable contracts with no runtime leakage | Compile-time contract errors and typed runtime errors |
| Runtime impl | Runtime implementor | Runtime / supervisor / transport wiring | Host process, cancellation, transport, scheduling, platform-specific execution | In runtime bootstrap and lifecycle code |

Practical guidance:

- Application authors should be able to paste a one-liner and get a
  working agent in under a minute.
- Agent authors should stay on the builder surface unless they are
  replacing a kernel contract.
- Trait implementors should keep dependencies narrow and implement the
  smallest stable interface that solves the problem.
- Runtime implementors should wire execution hosts directly, not add
  application-facing configuration detours.
- Every layer should have a matching example and README entry so the
  first working path is obvious and the advanced paths stay discoverable.

The four layers are the frame for the rest of this chapter: the
extensibility points below are the trait-implementor and runtime-implementor
layers in practice, while the builder surface is how most agent authors
compose the system.

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

## Self-Evolving Agent Architecture

Beyond static extensibility, Roko's architecture supports **self-evolution** —
the system improving its own agent configurations over time.

### Darwin Gödel Machine Pattern

The Darwin Gödel Machine (Sakana AI, arXiv:2505.22954, 2025) iteratively
modifies its own code and empirically validates each change using benchmarks.
It grows an archive of generated coding agents, samples from the archive,
and agents self-modify to create new versions.

**Results:** SWE-bench improved from 20.0% to 50.0% (2.5× improvement).
Self-discovered improvements included: patch validation steps, better file
viewing, enhanced editing tools, ranking multiple solutions, adding history
of failed attempts.

**Mapping to Roko:** Roko already has the infrastructure for self-modification
(PRD → plan → execute → gate → persist). The DGM pattern adds an
**evolutionary archive** — maintaining a population of agent configurations
and selecting for fitness:

```rust
/// Evolutionary archive for agent configurations.
/// Each entry is a configuration that produced good results,
/// along with its fitness score on recent tasks.
pub struct AgentArchive {
    /// Archive of agent configurations with fitness scores.
    entries: Vec<ArchiveEntry>,
    /// Maximum archive size (default: 50).
    max_entries: usize,
    /// Minimum fitness to remain in archive (default: 0.5).
    min_fitness: f64,
}

pub struct ArchiveEntry {
    /// The agent configuration (role, model, system prompt, tools, parameters).
    pub config: AgentConfiguration,
    /// Fitness score: weighted combination of gate pass rate, cost efficiency,
    /// and token efficiency (0.0–1.0).
    pub fitness: f64,
    /// Task types this configuration excels at.
    pub specializations: Vec<String>,
    /// Generation number (how many mutation steps from the seed config).
    pub generation: u32,
    /// Lineage: parent configuration IDs.
    pub parents: Vec<String>,
}

pub struct AgentConfiguration {
    pub role: AgentRole,
    pub model_key: String,
    pub system_prompt_overrides: HashMap<String, String>,
    pub tool_allowlist: Option<Vec<String>>,
    pub temperament: Temperament,
    pub reasoning_strategy: ReasoningStrategy,
    pub max_iterations: usize,
}

impl AgentArchive {
    /// Select a configuration for a new task, with tournament selection.
    pub fn select(&self, task_type: &str) -> &ArchiveEntry {
        // 1. Filter entries specialized for this task type
        // 2. Tournament selection: pick k random, return highest fitness
        // 3. With probability ε, return a random entry (exploration)
        todo!()
    }

    /// Mutate a configuration to create a variant for testing.
    pub fn mutate(&self, parent: &AgentConfiguration) -> AgentConfiguration {
        // Possible mutations:
        // - Change model_key (try a different model)
        // - Adjust system_prompt_overrides (add/remove instructions)
        // - Modify tool_allowlist (add/remove tools)
        // - Change reasoning_strategy (ReAct → Reflexion)
        // - Adjust max_iterations
        todo!()
    }

    /// After a task completes, update the archive.
    pub fn update(&mut self, config: &AgentConfiguration, result: &AgentResult) {
        // 1. Compute fitness from gate results, cost, tokens
        // 2. If fitness > min_fitness, add to archive
        // 3. If archive full, evict lowest-fitness entry
        // 4. Record specializations based on task type
    }
}
```

### Voyager-Style Skill Library

Voyager (Wang et al., 2023, arXiv:2305.16291) demonstrated that an
ever-growing library of executable skills enables lifelong learning with
three components: automatic curriculum, skill library, and iterative
prompting. Skills compound the agent's abilities and transfer to new tasks.

**Mapping to Roko:** The EpisodeLogger + playbook system in `roko-learn`
already captures execution traces. The missing piece is **skill extraction**:
identifying reusable patterns from successful episodes and storing them as
composable skills with semantic descriptions for retrieval.

### Agent Memory Sharing

How do agents in a multi-agent team transfer learned strategies?

```rust
/// Shared memory for multi-agent teams.
/// Agents can read from and contribute to a shared knowledge base
/// that persists across plan executions.
pub struct SharedAgentMemory {
    /// Successful strategies indexed by task type.
    strategies: HashMap<String, Vec<LearnedStrategy>>,
    /// Tool usage patterns (what works, what fails).
    tool_patterns: ToolTransitionGraph,
    /// Model routing preferences learned from team experience.
    routing_preferences: HashMap<String, ModelPreference>,
}

pub struct LearnedStrategy {
    pub description: String,
    /// The approach that worked (compressed as prompt fragment).
    pub approach: String,
    /// Task types this strategy applies to.
    pub applicable_to: Vec<String>,
    /// Confidence in this strategy (EMA of success rate).
    pub confidence: f64,
    /// Which agent discovered this strategy.
    pub discovered_by: AgentRole,
    /// Number of times this strategy has been successfully applied.
    pub success_count: u32,
}
```

### Intrinsic vs. Extrinsic Metacognition

Liu & van der Schaar (2025, ICML 2025 Position Paper, arXiv:2506.05109)
argue that existing "self-improving" agents rely on **extrinsic
metacognitive mechanisms** — fixed, human-designed loops (like ReAct or
reflection prompts). True self-improvement requires **intrinsic
metacognitive learning**: the agent's ability to evaluate, reflect on, and
adapt its own learning processes.

Three required components:
1. **Metacognitive knowledge** — Self-assessment of capabilities, tasks, and
   learning strategies.
2. **Metacognitive planning** — Deciding what and how to learn.
3. **Metacognitive evaluation** — Reflecting on learning experiences to
   improve future learning.

Roko's learning layer (efficiency events, cascade router, experiments,
adaptive thresholds) is extrinsic metacognition. The path to intrinsic
metacognition would require roko to modify its own learning mechanisms —
e.g., adjusting the EMA smoothing factor based on observed convergence rate,
or switching from LinUCB to Thompson Sampling when the arm set changes.

---

## Citations

1. Refactoring PRD §05-agent-types — 8-step domain plugin process,
   LlmBackend addition process.
2. Refactoring PRD §10-developer-guide — EventSource, FeedbackCollector,
   plugin system.
3. Sakana AI et al. (2025). "Darwin Gödel Machine: Open-Ended Evolution of
   Self-Improving Agents." arXiv:2505.22954. — SWE-bench 20% → 50%.
4. Wang, G. et al. (2023). "Voyager: An Open-Ended Embodied Agent with LLMs."
   arXiv:2305.16291. — Lifelong skill learning.
5. Liu, T. & van der Schaar, M. (2025). "Truly Self-Improving Agents Require
   Intrinsic Metacognitive Learning." ICML 2025. arXiv:2506.05109. —
   Extrinsic vs. intrinsic metacognition.
6. `crates/roko-agent/src/provider/mod.rs` — ProviderAdapter trait.
7. `crates/roko-agent/src/tool_loop/mod.rs` — LlmBackend trait.
8. `crates/roko-agent/src/translate/mod.rs` — Translator trait.
9. `crates/roko-agent/src/ollama_backend.rs` — Reference LlmBackend impl.
