# Extensibility and Agent Creation Sites

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). Agent extensibility via the Extension specialization (8 layers, 22 hooks). Agent creation sites as Graph instantiation patterns. Factory Cells that compose Agents from capability profiles, and the path to self-evolving agent configurations.

## Extension Architecture as a Functor Pattern

An **Extension** is a Cell that intercepts another Cell's Pipeline without changing the Graph's topology. In unified terms, this is the **Functor pattern**: an endofunctor F: Signal -> Signal that enriches/transforms data pre/post a Cell.

Roko's agent system has five extension points, each with a clear trait:

| Extension Point | Trait | Crate | Effort |
|---|---|---|---|
| New agent backend | `Agent` | roko-agent | Medium |
| New provider adapter | `ProviderAdapter` | roko-agent/provider | Low |
| New tool translator | `Translator` | roko-agent/translate | Medium |
| New LLM backend | `LlmBackend` | roko-agent/tool_loop | Low |
| New tool handler | `ToolHandler` | roko-core/tool | Low |

### The Four-Layer SDK

The SDK is layered so developers stop at the highest level that fits:

| Layer | User | Entry Point | What They Own |
|---|---|---|---|
| **One-liner** | Application author | `roko::run(...)` | Defaults, model selection, immediate success path |
| **Builder** | Agent author | `Agent::builder()` | Roles, tools, gates, prompts, configuration |
| **Trait impl** | Trait implementor | `ProviderAdapter`, `Translator`, `LlmBackend`, `ToolHandler` | Narrow, stable contracts |
| **Runtime impl** | Runtime implementor | Supervisor, transport wiring | Host process, cancellation, scheduling |

Each layer maps to a Cell abstraction level:
- One-liner: instantiates a pre-built Graph
- Builder: composes Cells into a custom Graph
- Trait impl: implements a new Cell type
- Runtime impl: implements a new Engine variant

## Adding a Provider: Config-Only Extension

The simplest extension. If the provider speaks OpenAI-compatible chat completions, no code is needed:

```toml
[providers.my-provider]
kind = "openai_compat"
base_url = "https://api.my-provider.com/v1"
api_key_env = "MY_PROVIDER_API_KEY"

[models.my-model-large]
provider = "my-provider"
slug = "my-model-large"
context_window = 128000
supports_tools = true
tool_format = "openai_json"
cost_input_per_m = 2.00
cost_output_per_m = 8.00
```

The `create_agent_for_model` factory resolves model -> provider -> adapter -> agent. No code changes.

### Adding a New Protocol Family (ProviderAdapter)

When a provider uses a non-standard protocol, implement `ProviderAdapter`:

```rust
/// A ProviderAdapter is a factory Cell: it takes config Signals
/// and produces an Agent Cell. Implements the Connect protocol
/// for the specific provider's wire format.
pub trait ProviderAdapter: Send + Sync {
    fn kind(&self) -> ProviderKind;

    fn create_agent(
        &self,
        provider: &ProviderConfig,
        model: &ModelProfile,
        options: &AgentOptions,
    ) -> Result<Box<dyn Agent>, AgentCreationError>;

    fn classify_error(&self, status: u16, body: &Value) -> ProviderError;
}
```

Registration in `adapter_for_kind` uses an exhaustive match, so the compiler catches unregistered variants:

```rust
pub fn adapter_for_kind(kind: ProviderKind) -> &'static dyn ProviderAdapter {
    match kind {
        ProviderKind::OpenAiCompat  => &OPENAI_COMPAT_ADAPTER,
        ProviderKind::ClaudeCli     => &CLAUDE_CLI_ADAPTER,
        ProviderKind::AnthropicApi  => &ANTHROPIC_API_ADAPTER,
        ProviderKind::CursorAcp     => &CURSOR_ACP_ADAPTER,
    }
}
```

### Adding an LlmBackend

If the provider supports tool calling and you want to use Roko's ToolLoop:

```rust
/// An LlmBackend is a Connect protocol Cell for the LLM call step.
/// It handles the HTTP request/response cycle for one provider.
#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn send_turn(
        &self,
        messages: &[Value],
        tools: &RenderedTools,
    ) -> Result<BackendResponse, LlmError>;
}
```

The existing `OllamaLlmBackend` at `crates/roko-agent/src/ollama_backend.rs` is a working reference. Wire it into the ToolLoop:

```rust
let backend = Arc::new(MyBackend { ... });
let translator = Arc::new(OpenAiTranslator);
let dispatcher = Arc::new(ToolDispatcher::new(registry, resolver));
let tool_loop = ToolLoop::new(translator, dispatcher, backend);
```

### Adding a Translator

For a model with a non-standard wire format, implement `Translator` (see [harness-and-format-engineering.md](harness-and-format-engineering.md) for the full format translation model).

## Eight Creation Sites: The Consolidation Problem

Agent construction was historically split across 8 places in the codebase. In unified terms, this means 8 separate Graph instantiation sites, each potentially producing differently-configured Agent Cells.

### The Sites

| Site | Location | Status |
|---|---|---|
| 1. `orchestrate.rs::run_prepared_agent` | Primary plan execution | Migrated (routed + no-routing paths) |
| 2. `orchestrate.rs` model selection | AgentRunConfig construction | Partially migrated |
| 3. `run.rs` | Single-prompt execution (`roko run`) | Migrated |
| 4. `prd.rs` | PRD draft/plan generation | Not migrated |
| 5. `research.rs` | Research agents | Partially migrated |
| 6. `agent_exec.rs` | Background task helper | Migrated |
| 7. Test code | Mock and integration tests | N/A (direct construction correct for tests) |
| 8. Examples/benchmarks | Example code | Acceptable |

### The Problem

Multiple creation sites mean:
1. **Inconsistent behavior** -- Direct fallback paths miss shared defaults, options, or safety settings.
2. **No single point for routing** -- The CascadeRouter can only intercept model selection on the factory path.
3. **Hard to add providers** -- Each manual path requires backend-aware handling.

### The Target: Single Factory Cell

The target is a single factory Cell that all production sites use:

```
Call site (any of 6 production sites)
    |
    v
create_agent_for_model(config, model_key, options)
    |
    +-- resolve_model(config, model_key) -> ResolvedModel
    |       +-- Config registry lookup
    |       +-- Fallback to slug heuristic
    |
    +-- CascadeRouter may override model_key -> different tier
    |
    +-- adapter_for_kind(provider_kind) -> &dyn ProviderAdapter
    |
    +-- adapter.create_agent(provider, profile, options) -> Box<dyn Agent>
```

In unified terms, `create_agent_for_model` is a **factory Cell** implementing the Route protocol: given a model key + options, it selects and instantiates the appropriate Agent Cell from the provider registry. The CascadeRouter sits in the factory path as a Route Cell that can override the model key.

### Migration Strategy

Each site migrates independently by replacing direct agent construction with `create_agent_for_model` + `AgentOptions`:

```rust
let options = AgentOptions {
    timeout_ms: Some(cfg.timeout_ms),
    system_prompt: Some(cfg.system_prompt),
    tools: Some(cfg.allowed_tools_csv),
    mcp_config: cfg.mcp_config,
    effort: Some(cfg.effort),
    bare_mode: cfg.bare_mode,
    // ... all config in one struct
};
let agent = create_agent_for_model(config, &cfg.model, options)?;
```

## The 8-Step Domain Plugin Process

Adding a new domain-specific agent type follows 8 steps that correspond to Cell creation and Graph wiring:

| Step | What | Unified Concept |
|---|---|---|
| 1. Define role | Add `AgentRole` variant with tier, budget, permissions | Cell identity + capabilities |
| 2. Create template | System prompt template in roko-compose | Compose protocol content |
| 3. Register tools | Domain-specific ToolDef + ToolHandler | New Cells with Connect protocol |
| 4. Configure model | `[models.*]` entries for domain-suited models | Route protocol candidates |
| 5. Wire provider | Provider config for the model's backend | Connect protocol wiring |
| 6. Set gates | Domain-specific Verify Cells (e.g., `forge build` for Solidity) | Verify protocol instances |
| 7. Add to router | Default tier in CascadeRouter | Route protocol initialization |
| 8. Test end-to-end | `roko run "<prompt>"` through full pipeline | Graph execution verification |

## Event System: Plugin Integration Cells

Two additional Cell types for domain integration:

```rust
/// EventSource Cell: emits domain-specific events for the learning subsystem.
/// Implements Observe protocol (read-only observation).
pub trait EventSource: Send + Sync {
    fn events(&self) -> Vec<DomainEvent>;
}

/// FeedbackCollector Cell: captures execution results for future improvement.
/// Implements React protocol (watches outcomes, emits feedback Signals).
pub trait FeedbackCollector: Send + Sync {
    fn collect(&self, result: &AgentResult) -> Vec<FeedbackSignal>;
}
```

These feed into the efficiency tracking pipeline and adaptive gate thresholds, providing domain-specific signal for the CascadeRouter's learning Loop.

## Self-Evolving Agent Architecture

Beyond static extensibility, the architecture supports **self-evolution** -- the system improving its own agent configurations over time.

### Darwin Godel Machine Pattern

The Darwin Godel Machine (Sakana AI, 2025, arXiv:2505.22954) iteratively modifies its own code and validates changes via benchmarks. Results: SWE-bench improved from 20.0% to 50.0% (2.5x). Self-discovered improvements included patch validation, better file viewing, ranking multiple solutions, and history of failed attempts.

In unified terms, this is a **Loop** where:
- The agent configuration is a Signal (content-addressed, versioned)
- Mutation is a Compose Cell that produces variants
- Evaluation is a Verify Cell (gate pipeline)
- Selection is a Route Cell (tournament + fitness)

```rust
/// Evolutionary archive: a Store of agent configuration Signals
/// with fitness scores. Each entry decays via demurrage unless
/// reinforced by successful task completion.
pub struct AgentArchive {
    entries: Vec<ArchiveEntry>,
    max_entries: usize,         // 50
    min_fitness: f64,           // 0.5
}

pub struct ArchiveEntry {
    pub config: AgentConfiguration,
    pub fitness: f64,           // Gate pass rate * cost efficiency * token efficiency
    pub specializations: Vec<String>,  // Task types this config excels at
    pub generation: u32,        // Mutation depth from seed
    pub parents: Vec<String>,   // Lineage
}
```

Roko already has the infrastructure: PRD -> plan -> execute -> gate -> persist. The DGM pattern adds the evolutionary archive and selection loop.

### Voyager-Style Skill Library

Voyager (Wang et al., 2023, arXiv:2305.16291) showed that an ever-growing library of executable skills enables lifelong learning. In Roko's terms, the EpisodeLogger + playbook system already captures execution traces. The missing piece is **skill extraction**: identifying reusable patterns from successful episodes and storing them as composable Heuristic Signals.

### Intrinsic vs. Extrinsic Metacognition

Liu & van der Schaar (2025, ICML, arXiv:2506.05109) distinguish:

- **Extrinsic metacognition**: fixed human-designed loops (ReAct, reflection). This is what Roko's learning layer does today (efficiency events, cascade router, experiments, adaptive thresholds).
- **Intrinsic metacognition**: the agent evaluates and adapts its own learning processes. This would require Roko to modify its own learning mechanisms -- e.g., adjusting EMA smoothing factor based on convergence rate, or switching from LinUCB to Thompson Sampling when the arm set changes.

The path from extrinsic to intrinsic is the L4 evolution surface in the roadmap: the spec itself becomes an agent in the evolutionary archive, bounded by Variance Inequality + c-factor gate.

---

## What This Enables

1. **Zero-code provider addition** -- Config-only extensions for OpenAI-compatible providers mean new models are available in minutes, not hours.
2. **Single safety enforcement point** -- The factory Cell (`create_agent_for_model`) ensures every production agent flows through the same safety Pipeline, regardless of which call site instantiated it.
3. **Self-improving configurations** -- The evolutionary archive pattern allows the system to discover better agent configurations automatically, compounding performance over time.

## Feedback Loops

1. **Creation site consolidation -> routing quality -> agent performance**: Every site that migrates to the factory Cell gives the CascadeRouter more routing decisions to learn from, improving future routing.
2. **Plugin registration -> tool diversity -> harness quality**: New ToolHandler registrations expand the harness's capability. The Meta-Harness thesis says harness quality dominates performance, so each new tool compounds system capability.
3. **Evolutionary archive -> configuration mutation -> gate verification -> archive update**: The DGM Loop uses the same gate pipeline that validates normal task output, so improvements in gate quality directly improve evolutionary selection.

## Open Questions

1. **Factory Cell for Claude CLI**: Claude CLI drives its own tool loop, which means it bypasses the ToolDispatcher/SafetyLayer Pipeline. Should the factory Cell apply safety at the orchestrator level (pre-prompt) for Claude CLI paths?
2. **Configuration versioning**: If agent configurations evolve via the DGM pattern, how should configuration Signals be versioned? Content-addressed (hash of config) or sequential (generation number)?
3. **Cross-profile composition**: When two domain profiles claim the same role name or tool ID, what is the resolution policy? Union with explicit collision detection, or namespacing?
4. **Skill extraction**: What is the minimum viable skill extraction from episodes? Pattern: successful episode -> template with placeholders -> stored as Heuristic Signal with `when` predicate and `then` action.

---

## Citations

1. Sakana AI et al. (2025). "Darwin Godel Machine." arXiv:2505.22954. -- SWE-bench 20% -> 50%.
2. Wang, G. et al. (2023). "Voyager: An Open-Ended Embodied Agent with LLMs." arXiv:2305.16291.
3. Liu, T. & van der Schaar, M. (2025). "Truly Self-Improving Agents Require Intrinsic Metacognitive Learning." ICML. arXiv:2506.05109.
4. `crates/roko-agent/src/provider/mod.rs` -- ProviderAdapter, create_agent_for_model.
5. `crates/roko-agent/src/tool_loop/mod.rs` -- LlmBackend, ToolLoop.
6. `crates/roko-agent/src/translate/mod.rs` -- Translator trait.
7. `crates/roko-agent/src/translate/capability.rs` -- ModelCapabilities, translator_for.
8. See [02-CELL.md](../../unified/02-CELL.md) for Cell trait, protocol definitions.
9. See [12-EXTENSIONS.md](../../unified/12-EXTENSIONS.md) for the 8-layer, 22-hook Extension system.
