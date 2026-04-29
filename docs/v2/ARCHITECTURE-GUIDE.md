# Roko v2 Architecture Guide

This is the definitive architecture reference for the Roko v2 runtime. It covers
the conceptual model, crate layer diagram, all protocol traits, the workflow
engine internals, event system, learning architecture, safety model, knowledge
store, and extension system. Read it before touching any cross-cutting subsystem.

---

## Table of Contents

1. [Philosophy](#1-philosophy)
2. [Crate Architecture](#2-crate-architecture)
3. [Protocol Traits](#3-protocol-traits)
4. [Foundation Service Traits](#4-foundation-service-traits)
5. [WorkflowEngine Architecture](#5-workflowengine-architecture)
6. [RuntimeEvent System](#6-runtimeevent-system)
7. [Learning Architecture](#7-learning-architecture)
8. [Safety Architecture](#8-safety-architecture)
9. [Knowledge Architecture](#9-knowledge-architecture)
10. [Extension System](#10-extension-system)

---

## 1. Philosophy

### 1 Noun + 6 Verb Traits

The entire Roko design corpus reduces to one data type and six operations:

```
Signal (Engram)     — the universal datum
  Store             — persist and retrieve
  Score             — rate along dimensions
  Verify (Gate)     — check against ground truth
  Route             — select among candidates
  Compose           — combine under budget
  React (Policy)    — watch stream, emit
```

Every capability in the system — agent spawning, gate verification, prompt
assembly, model routing, memory retrieval, pheromone reaction, chain
participation — is an implementation of one of those six traits. When you add
a feature, ask which verb it belongs to. If it does not map cleanly to one of
the six, that is a sign to reconsider the design.

The three additional protocol traits (Observe, Connect, Trigger) extend the six
for external-world integration but follow the same pattern: each is a `Cell`
supertrait with a single primary operation.

### Everything is a Graph of Cells

Every computation unit in Roko is a `Cell`. A Cell has:
- A stable `cell_id` (string)
- A `cell_name` for display
- A semantic `cell_version` (major, minor, patch)
- A `protocols()` list declaring which traits it conforms to
- Optional `estimated_cost()` and `estimated_duration()` for scheduling

Cells form graphs. The orchestrator resolves the DAG, the scheduler decides
execution order, and the effect driver invokes each Cell via its protocol. The
graph is addressable because every signal (engram) flowing through it is
content-hashed. The graph is observable because every transition emits a
`RuntimeEvent`.

### Universal Loop

Every workflow — from `roko run "fix bug"` to a full multi-agent plan execution
— maps to the same loop:

```
query → score → route → compose → act → verify → write → react
  │       │        │       │        │       │        │       │
Store  Score    Route  Compose  Effect  Verify   Store   React
```

1. **Query** — retrieve relevant engrams from the substrate
2. **Score** — rank them along relevance, recency, reputation, catalysis
3. **Route** — select model, backend, or gate via bandit/cascade router
4. **Compose** — assemble system prompt under token budget
5. **Act** — dispatch to LLM or tool, produce output
6. **Verify** — run gate pipeline (compile, test, clippy, diff, oracle)
7. **Write** — persist the outcome as a new engram
8. **React** — conductors and policies observe stream, intervene if needed

This loop is not metaphorical — it is the literal call sequence in
`WorkflowEngine::run_with_cancel` and `orchestrate.rs`.

---

## 2. Crate Architecture

### Layer Diagram

```
┌──────────────────────────────────────────────────────────────────────┐
│  CLI / Serve / ACP / Agent-Server                                     │
│  roko-cli  roko-serve  roko-acp  roko-agent-server                    │
├──────────────────────────────────────────────────────────────────────┤
│  Service Layer                                                         │
│  roko-agent  roko-compose  roko-gate  roko-learn  roko-neuro          │
│  roko-conductor  roko-dreams  roko-daimon  roko-orchestrator          │
├──────────────────────────────────────────────────────────────────────┤
│  Runtime / Core                                                        │
│  roko-runtime  roko-core                                              │
├──────────────────────────────────────────────────────────────────────┤
│  Primitives / Support                                                  │
│  roko-primitives  roko-std  roko-fs  roko-plugin                      │
├──────────────────────────────────────────────────────────────────────┤
│  Language / Index / MCP / Chain                                        │
│  roko-lang-*  roko-index  roko-mcp-*  roko-chain                      │
│  apps/mirage-rs  apps/agent-relay  apps/roko-chain-watcher            │
└──────────────────────────────────────────────────────────────────────┘
```

**Dependency rule**: lower layers MUST NOT import upper layers. Violations cause
compilation cycles. The CI layer-check enforces this. The one current known
tension is that `roko-core` and `roko-runtime` have a circular dependency in
progress of being resolved — the comment `TODO(arch)` marks the affected call
sites.

### Crate Reference Table

| Crate | Layer | Purpose | Key Exports |
|---|---|---|---|
| `roko-primitives` | Primitives | HDC vectors, tier routing, inference tiers | `HdcVector`, `InferenceTier` |
| `roko-core` | Core/Kernel | Signal (Engram), 6+3 protocol traits, Cell, foundation service traits, RuntimeEvent, extensions, cognitive workspace, policy manifest | `Engram`, `Store`, `Score`, `Verify`, `Route`, `Compose`, `React`, `Cell`, `RuntimeEvent`, `ModelCaller`, `GateRunner`, `PromptAssembler`, `FeedbackSink`, `Extension` |
| `roko-std` | Primitives | Default impls: `MemorySubstrate`, `NoOpScorer`, simple routers, 19 builtin tools, mock dispatcher | `MemorySubstrate`, `MockDispatcher` |
| `roko-fs` | Primitives | Filesystem substrate: append-only JSONL, GC, `.roko/` layout | `FileSubstrate`, `ArchiveColdSubstrate` |
| `roko-plugin` | Primitives | Plugin SDK: `EventSource`, `FeedbackCollector` | Plugin trait |
| `roko-runtime` | Runtime | `WorkflowEngine`, `PipelineStateV2`, `EffectDriver`, `TaskScheduler`, `EventBus`, `CancelToken`, process supervisor, heartbeat | `WorkflowEngine`, `EventBus`, `PipelineStateV2`, `RokoEvent` |
| `roko-gate` | Service | 11 gate implementations, 7-rung pipeline, adaptive thresholds, oracle rungs 4-6 | `CompileGate`, `TestGate`, `ClippyGate`, `DiffGate`, `ShellGate` |
| `roko-compose` | Service | Prompt assembly, `SystemPromptBuilder` (9-layer), 9 role templates, context packing, VCG attention auction | `SystemPromptBuilder`, `PromptComposer`, `SectionScorer` |
| `roko-agent` | Service | 5+ LLM backends (Claude CLI, Claude API, OpenAI-compat, Ollama, Gemini, Perplexity), connection pools, MCP passthrough, tool loop, safety layer | `ExecAgent`, `ClaudeApiAgent`, `ToolDispatcher`, `AgentContract` |
| `roko-learn` | Service | Episode logger, playbook store, UCB bandits, cascade router, prompt experiments A/B, efficiency events, adaptive gate thresholds, error pattern store | `CascadeRouter`, `EpisodeLogger`, `PlaybookStore`, `ExperimentStore`, `AdaptiveGateThresholds` |
| `roko-neuro` | Service | Durable knowledge store, distillation pipeline, tier progression, Ebbinghaus decay, confirmation/conflict tracking, custody chain, A-MAC admission | `NeuroStore`, `DistillationPipeline`, `KnowledgeEntry` |
| `roko-conductor` | Service | 10 reactive watchers, circuit breaker, interventions, diagnosis | `Conductor`, `CircuitBreaker` |
| `roko-dreams` | Service | Offline consolidation: hypnagogia, imagination, cycle phases | `DreamCycle`, `DreamJournal` |
| `roko-daimon` | Service | Affect engine: PAD vectors, somatic markers, `DaimonPolicy`, behavioral state, dispatch modulation | `DaimonPolicy`, `DaimonState`, `BehavioralState` |
| `roko-orchestrator` | Service | Plan DAG, parallel executor, merge queue, per-task dispatch, worktree manager | `PlanRunner`, `PlanExecutor` |
| `roko-agent-server` | CLI/Serve | Per-agent HTTP sidecar: 13 routes including `/message` (real LLM dispatch), `/stream` WS, `/predictions`, `/research`, `/tasks` | Axum router |
| `roko-serve` | CLI/Serve | HTTP control plane: ~85 REST routes + SSE + WebSocket on :6677 | Axum router, `StateHub`, SSE adapter |
| `roko-acp` | CLI/Serve | ACP server surface for editor integrations (VS Code, Zed) | ACP adapter |
| `roko-cli` | CLI | Binary entry point: all subcommands, ratatui TUI (F1-F7 tabs), `orchestrate.rs` plan runner | `main`, TUI, `PlanRunner` |
| `roko-index` | Index | Code intelligence: parser, symbol graph, PageRank, HDC fingerprints | `CodeIndex`, `SymbolGraph` |
| `roko-mcp-code` | MCP | Code-intelligence MCP server (used by agents via `--mcp-config`) | MCP server |
| `roko-mcp-github` | MCP | GitHub MCP integration | MCP server |
| `roko-mcp-slack` | MCP | Slack MCP integration | MCP server |
| `roko-mcp-scripts` | MCP | Script runner MCP integration | MCP server |
| `roko-mcp-stdio` | MCP | Stdio MCP bridge | MCP server |
| `roko-lang-rust` | Language | Rust build system support, parse, lint commands | `RustGate`, `RustLang` |
| `roko-lang-typescript` | Language | TypeScript / Node build system support | `TsLang` |
| `roko-lang-go` | Language | Go build system support | `GoLang` |
| `roko-chain` | Chain | Chain client, wallet trait abstractions, reads + signed writes | `ChainClient` |
| `roko-demo` | App | Demo environment orchestrator: contract deploy, fixture seed, scenario manifests | `DemoOrchestrator` |
| `apps/mirage-rs` | App | In-process EVM fork simulator. Optional `chain` feature adds HDC/pheromone subsystems. Optional `roko` feature bridges to `roko-core` traits | `MirageServer` |
| `apps/agent-relay` | App | Agent relay server | Relay |
| `apps/roko-chain-watcher` | App | Long-running agent that observes a mirage chain and posts insights via HTTP JSON-RPC | Chain watcher |

---

## 3. Protocol Traits

All protocol traits live in `crates/roko-core/src/traits.rs`. Every trait
(except `Bus`) requires `Cell` as a supertrait, giving the execution engine
identity, cost estimation, and protocol introspection.

Source: `/Users/will/dev/nunchi/roko/roko/crates/roko-core/src/traits.rs`

### Store

Stores and retrieves `Engram`s. All storage backends are API-identical.

```rust
pub trait Store: Send + Sync {
    async fn put(&self, engram: Engram) -> Result<ContentHash>;
    async fn get(&self, id: &ContentHash) -> Result<Option<Engram>>;
    async fn query(&self, q: &Query, ctx: &Context) -> Result<Vec<Engram>>;
    async fn query_similar(
        &self, fp: &HdcVector, radius: f32, limit: usize, ctx: &Context
    ) -> Result<Vec<(ContentHash, f32)>>;
    async fn prune(&self, threshold: f32, ctx: &Context) -> Result<usize>;
}
```

**Idempotence**: `put` is idempotent for signals with identical content hashes.
Re-putting the same signal is a no-op.

**Implementations**:
- `MemorySubstrate` (`roko-std`) — in-memory, for tests
- `FileSubstrate` (`roko-fs`) — append-only JSONL at `.roko/signals.jsonl`
- `HdcSubstrate` — semantic similarity search via HDC vector index
- `ChainSubstrate` — shared on-chain state (Phase 2+)

### ColdStore

Archival store for aged-out engrams.

```rust
pub trait ColdStore: Send + Sync {
    async fn archive(&self, engram: Engram) -> Result<ContentHash>;
    async fn archive_batch(&self, engrams: Vec<Engram>) -> Result<usize>;
    async fn thaw(&self, id: &ContentHash) -> Result<Option<Engram>>;
    async fn purge_before(&self, epoch_ms: i64) -> Result<usize>;
}
```

Migration flow: `Store (hot) --age_out()--> ColdStore (cold/archive) <--thaw()--`.

**Implementation**: `ArchiveColdSubstrate` (`roko-fs`) — compressed JSONL archive.

### Score

Rates an engram along multi-dimensional axes. Pure function of `(engram, context)`.

```rust
pub trait Score: Send + Sync {
    fn score(&self, engram: &Engram, ctx: &Context) -> ScoreValue;
    fn score_engram(&self, engram: &Engram, ctx: &Context) -> ScoreValue;
    fn score_pulse(&self, p: &Pulse, ctx: &Context) -> ScoreValue;
    fn score_datum(&self, datum: Datum<'_>, ctx: &Context) -> ScoreValue;
}
```

Scorers compose freely via `CompositeScorer` using + and × operations.

**Implementations**: `RelevanceScorer`, `RecencyScorer`, `ReputationScorer`,
`CatalyticScorer` (how many downstream engrams does this enable).

### Verify

Verifies an engram against ground truth. Gates are the bridge to external reality.

```rust
#[async_trait]
pub trait Verify: Send + Sync {
    async fn verify(&self, engram: &Engram, ctx: &Context) -> Verdict;
    async fn verify_stream(&self, pulses: &[Pulse], ctx: &Context) -> Verdict;
    fn name(&self) -> &str;
}
```

A `Verdict` with `passed = true` is a claim that the engram is correct in some
domain (compiled, tests pass, schema valid, balance sufficient).

**Implementations** (in `roko-gate`): `CompileGate`, `TestGate`, `ClippyGate`,
`DiffGate`, `ShellGate`, `OracleGate` (rungs 4-6).

### Route

Selects one engram from many candidates. Routers learn via `feedback`.

```rust
pub trait Route: Send + Sync {
    fn select(&self, candidates: &[Engram], ctx: &Context) -> Option<Selection>;
    fn select_engram(&self, candidates: &[Engram], ctx: &Context) -> Option<Selection>;
    fn select_pulse(&self, candidates: &[Pulse], ctx: &Context) -> Option<Selection>;
    fn feedback(&self, outcome: &Outcome);
    fn name(&self) -> &str;
}
```

**Implementations**:
- `StaticRouter` — deterministic, config-driven
- `LinUCBRouter` — contextual bandit with feature vectors
- `CascadeRouter` — multi-stage: confidence threshold first, UCB if uncertain
- `WeightedRouter` — softmax over scorer outputs

### Compose

Combines multiple engrams into one new engram under a `Budget`.

```rust
pub trait Compose: Send + Sync {
    fn compose(
        &self, engrams: &[Engram], budget: &Budget,
        scorer: &dyn Score, ctx: &Context,
    ) -> Result<Engram>;
    fn compose_datums(
        &self, datums: &[Datum<'_>], budget: &Budget,
        scorer: &dyn Score, ctx: &Context,
    ) -> Result<Engram>;
    fn name(&self) -> &str;
}
```

The `Budget` constrains by token count, byte count, engram count, or wall time.
`compose_datums` accepts a polymorphic mix of persisted engrams and ephemeral
pulses — the default implementation converts pulses to synthetic engrams and
delegates to `compose`.

**Implementations**: `PromptComposer` (`roko-compose`), `ContextPacker`.

### React

Watches a stream of engrams and emits new engrams in response. This is the
reactive/behavioral layer.

```rust
pub trait React: Send + Sync {
    fn decide(&self, stream: &[Engram], ctx: &Context) -> Vec<Engram>;
    fn decide_with_pulses(
        &self, engrams: &[Engram], pulses: &[Pulse], ctx: &Context,
    ) -> PolicyOutputs;
    fn name(&self) -> &str;
}
```

`PolicyOutputs` can contain both engrams (to persist) and pulses (to publish
on the Bus). Policies run continuously over the engram stream.

**Implementations**: conductor watchers, circuit breaker, episode logger,
pheromone reactor, heartbeat emitter, sentinel detector.

### Bus

Publish/subscribe transport for ephemeral `Pulse`s.

```rust
pub trait Bus: Send + Sync {
    type Receiver: Send;
    fn publish(&self, pulse: Pulse) -> Result<u64>;
    fn subscribe(&self, filter: TopicFilter) -> Result<Self::Receiver>;
}
```

The Bus complements the durable `Store`. Pulses flow through Bus for immediate
downstream reactions. Only pulses worth persisting get promoted to `Engram`s
and written to a `Store`.

### Observe

Passive data collection from external sources.

```rust
pub trait Observe: Cell {
    fn observe(&self) -> Vec<Engram>;
}
```

### Connect

Manages connections to external systems.

```rust
pub trait Connect: Cell {
    fn connect(&self) -> Result<()>;
    fn health(&self) -> bool;
    fn disconnect(&self) -> Result<()>;
}
```

### Trigger

Armed conditions that fire when criteria are met.

```rust
pub trait Trigger: Cell {
    fn arm(&self) -> Result<()>;
    fn disarm(&self) -> Result<()>;
}
```

### Cell (Supertrait)

Every protocol trait (Store, Score, Verify, Route, Compose, React, Observe,
Connect, Trigger) uses `Cell` as a supertrait.

Source: `/Users/will/dev/nunchi/roko/roko/crates/roko-core/src/cell.rs`

```rust
pub trait Cell: Send + Sync + 'static {
    fn cell_id(&self) -> &str;
    fn cell_name(&self) -> &str;
    fn cell_version(&self) -> CellVersion { (0, 1, 0) }
    fn protocols(&self) -> &[&str] { &[] }
    fn estimated_cost(&self) -> Option<f64> { None }
    fn estimated_duration(&self) -> Option<Duration> { None }
}
```

The `protocols()` method returns a list of protocol names the cell conforms to,
enabling runtime introspection. The scheduler uses `estimated_cost` and
`estimated_duration` for budget-aware task ordering.

---

## 4. Foundation Service Traits

Foundation service traits live in `crates/roko-core/src/foundation.rs`. They
define the contracts between the `WorkflowEngine` and its backing services.
Each trait has exactly one concrete implementation in its designated crate.

Source: `/Users/will/dev/nunchi/roko/roko/crates/roko-core/src/foundation.rs`

### ModelCaller → ModelCallService (roko-agent)

```rust
#[async_trait]
pub trait ModelCaller: Send + Sync {
    async fn call(&self, req: ModelCallRequest) -> Result<ModelCallResponse>;
}
```

`ModelCallRequest` carries the full call specification: model ID, system prompt,
conversation history, token budget, routing hints, cache policy, caller surface,
and run ID. `ModelCallResponse` returns content, token usage, cost, and request
ID.

**Cache policy variants**: `Default` (standard L1 lookup), `Bypass` (skip
lookup, still store), `ForceRefresh` (skip + evict prior cached result).

**Caller surface constants** (`foundation::caller`):
- `CLI` — direct `roko run` invocation
- `SERVE` — HTTP control plane call
- `RESEARCH` — research agent call
- `DREAMS` — offline consolidation call

**Error types** (`GatewayError`):
- `ProviderError` — LLM provider returned an error
- `BudgetExceeded` — token or cost limit hit
- `RateLimited` — provider throttled; optional `retry_after_ms`
- `CacheError` — L1 cache operation failed
- `Cancelled` — cooperative cancellation
- `ConvergenceDetected` — identical output repeated N consecutive times

### PromptAssembler → PromptAssemblyService (roko-compose)

```rust
#[async_trait]
pub trait PromptAssembler: Send + Sync {
    async fn assemble(&self, spec: PromptSpec) -> Result<String>;
    fn last_prompt_section_ids(&self) -> Vec<String>;
    fn last_knowledge_ids(&self) -> Vec<String>;
}
```

`PromptSpec` carries the agent role, task description, working directory,
gate feedback from prior iterations, and anti-patterns. After assembly, the
assembler records which section IDs and knowledge entry IDs were included —
these are forwarded to `ModelCallRequest` for learning attribution.

### FeedbackSink → FeedbackService (roko-learn)

```rust
#[async_trait]
pub trait FeedbackSink: Send + Sync {
    async fn record(&self, event: FeedbackEvent) -> Result<()>;
    async fn flush(&self) -> Result<()>;
}
```

**FeedbackEvent variants**:
- `ModelCall` — records per-call cost, latency, token usage, role, prompt
  sections, knowledge IDs, model, provider, and success flag
- `GateResult` — records gate name, pass/fail, run ID, duration
- `WorkflowComplete` — records total cost, tokens, duration, outcome

The feedback sink is the central learning signal recorder. Every model call,
gate run, and workflow completion flows through here. The learning subsystem
(`roko-learn`) processes these events asynchronously to update the cascade
router, adaptive gate thresholds, and efficiency JSONL.

### GateRunner → GateService (roko-gate)

```rust
#[async_trait]
pub trait GateRunner: Send + Sync {
    async fn run_gates(&self, config: GateConfig) -> Result<GateReport>;
}
```

`GateConfig` specifies: working directory, enabled gate names, shell gate
commands, and maximum rung (0-6). `GateReport` aggregates `GateVerdict` items
— each verdict carries gate name, pass/skip status, output text, and duration.

Helper methods on `GateReport`:
- `all_passed()` — true when every gate passed and none were skipped
- `first_failure()` — first failing `GateVerdict`, if any
- `failure_summary()` — formatted string of all failures for agent feedback

### EventConsumer — JsonlLogger, SseAdapter, AcpAdapter

```rust
pub trait EventConsumer: Send + Sync {
    fn consume(&self, event: &RuntimeEvent);
}
```

Consumers are non-blocking. If async work is needed, they buffer internally.

**Implementations**:
- `JsonlLogger` — appends events to `.roko/episodes.jsonl`
- `SseAdapter` — streams events to connected SSE clients (`roko-serve`)
- `AcpAdapter` — sends events to ACP-connected editors (`roko-acp`)
- `StateHub` — updates in-memory dashboard state for TUI and REST polling

### EffectExecutor → EffectDriver (roko-runtime)

```rust
#[async_trait]
pub trait EffectExecutor: Send + Sync {
    async fn execute(&self, effect: Effect) -> Result<EffectOutcome>;
}
```

**Effect variants**: `SpawnAgent`, `RunGates`, `Commit`, `Checkpoint`.

**EffectOutcome variants**: `AgentDone` (agent ID, output, tokens, cost, files
changed), `GatesDone`, `CommitDone`, `CheckpointDone`, `Failed`.

The critical design principle: the state machine (`PipelineStateV2`) decides
WHAT to do by returning `PipelineOutput`; the `EffectDriver` decides HOW.
This separation makes the state machine pure, serializable, and testable.

### AffectPolicy → DaimonPolicy (roko-daimon)

```rust
#[async_trait]
pub trait AffectPolicy: Send + Sync {
    fn pre_dispatch(&self, task_id: &str, role: &str) -> AffectContext;
    fn on_task_outcome(&mut self, task_id: &str, succeeded: bool,
                       tokens_used: u64, cost_usd: f64);
    fn on_gate_result(&mut self, gate_name: &str, passed: bool,
                      rung: u8, confidence: f64);
    fn modulate_dispatch(&self, role: &str, params: &mut DispatchModulation);
    fn behavioral_state(&self) -> BehavioralState;
    async fn persist(&self) -> Result<()>;
}
```

`AffectContext` carries the current `BehavioralState`, PAD vector (Pleasure,
Arousal, Dominance), and optional emotional tag. `DispatchModulation` carries:
- `tier_bias` — -1.0 (prefer cheapest) to +1.0 (prefer most capable)
- `turn_limit_factor` — multiplier on default turn limit
- `exploration_rate` — higher = more exploratory routing

**Behavioral states**: `Engaged`, `Stressed`, `Fatigued`, `Curious`,
`Frustrated`, and others defined in `roko-core/src/affect.rs`.

When affect is disabled, `NoOpAffectPolicy` provides neutral defaults — all
modulation parameters are identity, behavioral state is `Engaged`.

---

## 5. WorkflowEngine Architecture

The `WorkflowEngine` is the shared entry point for CLI (`roko run`), ACP, and
the HTTP control plane. It ties together `PipelineStateV2` (decisions) and
`EffectDriver` (side effects) into a run loop.

Source: `/Users/will/dev/nunchi/roko/roko/crates/roko-runtime/src/workflow_engine.rs`
Source: `/Users/will/dev/nunchi/roko/roko/crates/roko-runtime/src/pipeline_state.rs`

### PipelineStateV2: Pure State Machine

```rust
pub struct PipelineStateV2 {
    pub phase: Phase,
    pub config: WorkflowConfig,
    pub iteration: u32,
    pub autofix_attempts: u32,
    pub original_prompt: String,
    pub strategist_brief: Option<String>,
    pub review_findings: Vec<String>,
    pub last_gate_failure: Option<String>,
    pub files_changed: u32,
    pub commit_hash: Option<String>,
}
```

**Phases**:
```
Pending → Strategizing → Implementing → Gating → AutoFixing → Reviewing → Committing → Complete
                            ↑_______________↑ (retry loop)
                            ↑_______________________________↑ (review revision)
```

Terminal phases: `Complete`, `Halted { reason }`, `Cancelled`.

**Transitions** (driven by `step(PipelineInput) -> PipelineOutput`):

| From | Input | To | Output |
|---|---|---|---|
| `Pending` | `Start` | `Strategizing` | `SpawnStrategist` |
| `Pending` | `Start` (no strategy) | `Implementing` | `SpawnImplementer` |
| `Strategizing` | `StrategyComplete` | `Implementing` | `SpawnImplementer { context: brief }` |
| `Implementing` | `AgentCompleted` | `Gating` | `RunGates` |
| `Implementing` | `AgentFailed` | `Halted` | `Halt` |
| `Gating` | `GatesPassed` (no review) | `Committing` | `Commit` |
| `Gating` | `GatesPassed` (with review) | `Reviewing` | `SpawnReviewer` |
| `Gating` | `GateFailed` (autofix budget) | `AutoFixing` | `SpawnAutoFixer` |
| `Gating` | `GateFailed` (iteration budget) | `Implementing` | `SpawnImplementer { context: error }` |
| `AutoFixing` | `AgentCompleted` | `Gating` | `RunGates` |
| `Reviewing` | `ReviewApproved` | `Committing` | `Commit` |
| `Reviewing` | `ReviewRevise` | `Implementing` | `SpawnImplementer { context: findings }` |
| `Committing` | `CommitDone` | `Complete` | `Done { Success }` |
| `*` | `UserCancel` | `Cancelled` | `Done { Cancelled }` |
| `*` | `ResourceExhausted` | `Halted` | `Halt` |

**Three workflow templates**:

| Template | has_strategy | has_review | max_iterations | max_autofix_attempts |
|---|---|---|---|---|
| `Express` | false | false | 1 | 1 |
| `Standard` | false | true | 2 | 2 |
| `Full` | true | true | 3 | 2 |

Templates are loaded from `roko.toml` via `WorkflowConfig::from_toml_str`.
Any key can override the template preset.

### Checkpoint and Resume

`PipelineStateV2` is fully serializable via `serde`. Checkpoint and restore:

```rust
// Serialize
let json = pipeline.checkpoint()?;

// Restore
let pipeline = PipelineStateV2::from_checkpoint(&json)?;
```

The serialized JSON is written by `Effect::Checkpoint` to
`.roko/state/executor.json`. The `--resume` flag on `roko plan run` loads this
file and continues from the exact saved phase, preserving iteration count,
review findings, and gate failure context.

### EffectDriver

The `EffectDriver` bridges `PipelineOutput` actions to actual async operations:

```rust
pub struct EffectServices {
    pub default_model: String,
    pub model_caller: Arc<dyn ModelCaller>,
    pub prompt_assembler: Arc<dyn PromptAssembler>,
    pub feedback_sink: Arc<dyn FeedbackSink>,
    pub gate_runner: Arc<dyn GateRunner>,
    pub affect_policy: Option<Arc<Mutex<dyn AffectPolicy>>>,
}
```

The driver applies affect modulation before each dispatch:
1. `affect_policy.pre_dispatch(task_id, role)` → `AffectContext`
2. Compute `DispatchModulation` from context
3. Adjust temperature, tier bias, cache policy, turn limit
4. After completion: `affect_policy.on_task_outcome(...)` and `on_gate_result(...)`

Constants governing the modulation:
- `BASE_TEMPERATURE = 0.2`
- `EXPLORATION_TEMPERATURE_RANGE = 0.6` (added when exploration rate is high)
- `TIER_TEMPERATURE_RANGE = 0.1` (adjusted by tier bias)
- Cache bypass is applied when `exploration_rate > 0.5`

### TaskScheduler

`TaskScheduler` is a pure DAG dependency resolver used by `WorkflowEngine` for
multi-task plan execution.

Source: `/Users/will/dev/nunchi/roko/roko/crates/roko-runtime/src/task_scheduler.rs`

```rust
pub struct TaskScheduler {
    tasks: HashMap<String, SchedulableTask>,
    status: HashMap<String, TaskStatus>,
    max_parallel: usize,
}
```

Each `SchedulableTask` has an ID, a list of dependency IDs, and a list of files
it will modify. The scheduler:
1. Computes which tasks are `Ready` (all deps `Completed`)
2. Respects `max_parallel` when returning startable tasks
3. Applies file exclusion: two tasks that modify the same file cannot run
   simultaneously
4. Marks dependents as `Skipped` when a dependency `Failed`

Task status flow: `Blocked → Ready → Running → Completed | Failed | Skipped`.

### WorkflowEngine Run Loop

The `WorkflowEngine::run_with_cancel` loop:

```
1. Create PipelineStateV2 from config
2. Create EffectDriver from EffectServices
3. Emit RuntimeEvent::WorkflowStarted
4. Loop:
   a. Check CancelToken — if cancelled, step(UserCancel), break
   b. step(last_input) → PipelineOutput
   c. Emit RuntimeEvent::PhaseTransition
   d. Match PipelineOutput:
      - SpawnAgent → call model_caller, emit AgentSpawned/AgentOutput/AgentCompleted
      - RunGates → call gate_runner, emit GateStarted/GatePassed/GateFailed
      - Commit → run git commit via shell
      - Checkpoint → serialize + write to disk, emit StateCheckpointed
      - Done → break
      - Halt → break
   e. Record feedback via feedback_sink
   f. Map effect outcome to PipelineInput for next iteration
5. Emit RuntimeEvent::WorkflowCompleted
6. Return WorkflowRunReport
```

---

## 6. RuntimeEvent System

Source: `/Users/will/dev/nunchi/roko/roko/crates/roko-core/src/runtime_event.rs`
Source: `/Users/will/dev/nunchi/roko/roko/crates/roko-runtime/src/event_bus.rs`

### RuntimeEvent: 12 Variants

```rust
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum RuntimeEvent {
    // Lifecycle (3)
    WorkflowStarted { run_id, template, prompt },
    PhaseTransition { run_id, from, to },
    WorkflowCompleted { run_id, outcome: WorkflowOutcome },

    // Agent (4)
    AgentSpawned { run_id, agent_id, role, model },
    AgentOutput { run_id, agent_id, chunk },
    AgentCompleted { run_id, agent_id, output, tokens_used, cost_usd },
    AgentFailed { run_id, agent_id, error },

    // Gates (3)
    GateStarted { run_id, gate_name, rung },
    GatePassed { run_id, gate_name, duration_ms },
    GateFailed { run_id, gate_name, output, duration_ms },

    // Feedback (1)
    FeedbackRecorded { run_id, kind, summary },

    // Persistence (1)
    StateCheckpointed { run_id, path },
}
```

`WorkflowOutcome` variants: `Success { commit_hash }`, `Halted { reason }`,
`Cancelled`.

### RuntimeEventEnvelope

Every emitted event is wrapped in an envelope for ordered delivery and replay:

```rust
pub struct RuntimeEventEnvelope {
    pub run_id: String,
    pub seq: u64,
    pub ts: DateTime<Utc>,
    pub schema_version: u8,  // currently 1
    pub source: String,
    pub payload: RuntimeEvent,
}
```

### EventBus: Typed Broadcast with Replay Ring

```
Producer --emit()--> EventBus --broadcast--> Subscriber₁
                         │                   Subscriber₂
                         │                   Subscriber₃
                         ▼
                    ReplayRing (bounded VecDeque)
```

`EventBus<E>` is generic over any `E: Clone + Send + Sync + 'static`. It
combines:
- A `tokio::sync::broadcast` channel for live fan-out (never blocks producers)
- A bounded `VecDeque` replay ring for late subscriber catch-up
- Monotonic sequence numbering (`AtomicU64`) for gap detection

Key API:
```rust
let bus: EventBus<RuntimeEvent> = EventBus::new(2048);
let mut rx: broadcast::Receiver<_> = bus.subscribe();
bus.emit(event);
let replayed: Vec<_> = bus.replay_from(seq);
let sender: BusSender<_> = bus.sender(); // shareable, emit-only handle
```

If the ring is full, the oldest event is evicted. Live subscribers that fall
behind will miss events on the broadcast channel but can always catch up via
`replay_from`.

### RokoEvent: Cross-Runtime Bus

In addition to `RuntimeEvent`, a second typed bus carries `RokoEvent` — the
cross-subsystem event type used for inter-component coordination:

```rust
pub enum RokoEvent {
    PlanRevision { request_id, plan_id, task_id, reason, failing_verdicts, ... },
    PrdPublished { slug, path, published_at, origin },
    HeartbeatTick(HeartbeatTick),
    HeartbeatWakeup { condition, issued_at },
    CognitiveSignal { signal, issued_at },
    AgentLifecycleTransition(LifecycleTransition),
    TickBroadcast { tick_id, agent_id, tier, passed, cost_usd, ... },
    ReactDecision { tick_id, decision, signals, decided_at },
}
```

The global `RokoEvent` bus is a process-singleton: `global_event_bus()`.

**Consumers of RuntimeEvent**:
- `JsonlLogger` — appends to `.roko/episodes.jsonl` for audit trail
- `SseAdapter` — pushes to `/events` SSE stream (roko-serve)
- `AcpAdapter` — pushes to ACP-connected editor surface (roko-acp)
- `StateHub` — updates in-memory dashboard snapshot for TUI and REST

---

## 7. Learning Architecture

All learning state lives in `.roko/learn/`. Every component is append-only or
EMA-smoothed — no destructive updates.

### CascadeRouter: Adaptive Model Selection

Persists to `.roko/learn/cascade-router.json`.

The cascade router operates in two stages:

1. **Confidence stage** — if the current context matches a known-good prior
   selection with confidence above threshold, use it directly
2. **UCB bandit stage** — if confidence is insufficient, apply Upper Confidence
   Bound selection over model candidates

Per-model UCB state tracks:
- `n_trials` — total selection count
- `n_successes` — successful outcomes
- `total_cost_usd` — cumulative cost
- `avg_latency_ms` — running average latency

Model routing is configurable via `roko.toml` under `[models]`. Currently, the
neuro store is not yet consulted for model selection (open gap: knowledge-
informed routing).

### EpisodeLogger: Agent Turn Recording

Persists to `.roko/episodes.jsonl`. Every agent turn emits an episode entry
capturing: run ID, agent ID, role, model, prompt section IDs, knowledge IDs,
HDC fingerprint, token usage, cost, gate verdicts, and outcome.

The HDC fingerprint is computed from the concatenation of input tokens and
stored alongside the episode for semantic deduplication and playbook extraction.

### PlaybookStore: Extracted Tool-Call Sequences

Persists to `.roko/learn/playbooks.json`. The playbook store extracts
successful tool-call sequences from completed episodes and indexes them by
task type and role. At dispatch time, the orchestrator queries the playbook
store and injects matching plays into the system prompt.

### ErrorPatternStore: Cross-Agent Error Sharing

Captures structured failure patterns (gate name, error classification, pattern
ID) across all agent runs. Patterns are de-duplicated and shared across agents
so a pattern learned by one agent benefits all future dispatches.

When a gate fails, the failure pattern ID is attached to the `PlanRevision`
`RokoEvent`. The planner agent receives the blocking findings and adjusts the
task plan accordingly.

### AdaptiveGateThresholds: EMA-Smoothed Per-Rung

Persists to `.roko/learn/gate-thresholds.json`. For each gate rung (0-6), an
exponential moving average smooths the observed pass rate over recent runs.
When the EMA falls below a configurable threshold, the orchestrator adjusts the
gate configuration (e.g., loosening thresholds for flaky tests, tightening for
critical compile gates).

EMA update: `ema_new = α × observed + (1 - α) × ema_old` where α is the
smoothing factor (default 0.1).

### ExperimentStore: Prompt A/B Experiments

Persists to `.roko/learn/experiments.json`. Supports multi-armed bandit
experiments over prompt variant selection. Each experiment tracks:
- Variant IDs and their prompt section overrides
- Per-variant outcome counts (success, failure, cost)
- Thompson sampling weights for variant selection

### FeedbackService: Centralized Learning Signal Recording

The `FeedbackSink` trait implementation (`roko-learn`) aggregates all learning
signals and fans them out to the appropriate sub-stores:
- Model call → update cascade router stats, efficiency log
- Gate result → update adaptive gate thresholds
- Workflow complete → update experiment outcomes

### Knowledge Admission: A-MAC

The neuro store admits new knowledge entries through the A-MAC filter:

| Dimension | Meaning |
|---|---|
| **A**ccuracy | Does the claim match verifiable facts? (gate-verified) |
| **M**odel | Which model produced the claim, and what is its track record? |
| **A**udit | Is the lineage traceable? (provenance chain intact) |
| **C**onfidence | What is the claim's uncertainty score? |

Entries below the A-MAC threshold for any dimension are rejected. Entries at
the borderline are admitted as `Provisional` tier and must accumulate
confirmations before promotion.

---

## 8. Safety Architecture

Safety in Roko is defense-in-depth across four layers.

### Layer 1: AgentContract (Role-Scoped YAML)

Every agent role has a YAML contract at `.roko/contracts/<role>.yaml`. The
contract declares:
- Allowed tool categories (read-only, write, shell, network)
- Forbidden file path patterns
- Maximum cost per invocation
- Maximum turn count
- Allowed external domains

The `ToolDispatcher` in `roko-agent` checks every tool call against the
contract before execution.

### Layer 2: ContractLoadMode

When a contract YAML is missing, the system chooses between:

- `Strict` — fail closed. Dispatch is rejected with a clear error.
- `RestrictedFallback` — fall back to a minimally permissive default contract.
  Logs a warning. Never grants permissions beyond the fallback set.

The current deployment uses `RestrictedFallback` where YAML is missing (partial
wiring). Production deployments should use `Strict` for all roles.

### Layer 3: Capability Intersection Enforcement

The `ToolDispatcher` computes the intersection of:
1. Tools declared in the agent manifest
2. Tools permitted by the agent contract
3. Tools available in the current MCP config

Only tools present in all three sets are offered to the LLM. This prevents
prompt injection attacks from expanding the tool surface beyond what the role
and contract authorize.

### Layer 4: Pre/Post Checks

Before and after every tool invocation, the `ToolDispatcher` runs:
- **Pre-check**: path traversal detection, domain allowlist validation, cost
  budget check, turn count check
- **Post-check**: output sanitization, sensitive data scrubbing, audit log entry

The `Extension` system's Action layer (layer 4) also participates: extensions
can return `ActionDecision::Block(reason)` to veto a tool call before it
executes, or `ActionDecision::Rewrite(modified)` to redirect it.

### Safety Principle: Fail-Closed

Every safety check defaults to deny-on-error. If the contract cannot be loaded,
the tool table cannot be computed, or the capability intersection produces an
empty set, the dispatch is rejected — not silently permitted.

---

## 9. Knowledge Architecture

### Neuro Store: Append-Only JSONL with HDC Fingerprints

The neuro store (`roko-neuro`) is the durable knowledge layer. It stores
`KnowledgeEntry` records in append-only JSONL at `.roko/neuro/`. Each entry
carries:
- `id` — BLAKE3 content hash
- `hdc_fingerprint` — HDC vector for semantic similarity queries
- `knowledge_type` — one of 6 types (Fact, Heuristic, Pattern, Anti-pattern,
  Insight, Playbook)
- `tier` — one of 4 validation tiers (Provisional, Confirmed, Trusted, Canonical)
- `confirmations` — list of run IDs that confirmed this entry
- `conflicts` — list of run IDs that contradicted this entry
- `decay` — Ebbinghaus decay parameters
- `provenance` — lineage back to source episodes

### Six Knowledge Types

| Type | Definition |
|---|---|
| `Fact` | Verifiable claim about the codebase or environment |
| `Heuristic` | Rule of thumb derived from repeated outcomes |
| `Pattern` | Successful code or workflow pattern |
| `AntiPattern` | Pattern that consistently leads to failure |
| `Insight` | Cross-domain synthesis from multiple facts/heuristics |
| `Playbook` | Sequenced tool-call procedure for a class of tasks |

### Four Validation Tiers

| Tier | Confirmation Count | Half-Life |
|---|---|---|
| `Provisional` | 0–1 | 7 days |
| `Confirmed` | 2–4 | 30 days |
| `Trusted` | 5–9 | 90 days |
| `Canonical` | 10+ | 365 days |

Higher tiers decay more slowly (Ebbinghaus-inspired). Entries decay toward
zero weight and are pruned when weight falls below the configured threshold.

### Three-Stage Distillation

```
Episodes (.roko/episodes.jsonl)
    ↓  [extract patterns from tool-call sequences]
Playbooks (.roko/learn/playbooks.json)
    ↓  [synthesize across plays]
Insights → Heuristics → Canonical knowledge entries
```

The `DistillationPipeline` in `roko-neuro` runs offline (triggered by
`roko knowledge dream run` or the dreams subsystem). It:
1. Reads recent episodes from the episode log
2. Extracts successful tool-call sequences as candidate playbook entries
3. Synthesizes across multiple plays to produce heuristics
4. Promotes high-quality heuristics to `KnowledgeEntry` records in the neuro store
5. Cross-references against existing entries: increments `confirmations` on
   match, logs to `conflicts` on contradiction

### Confirmation and Conflict Tracking

Entries gain confirmation when a new episode produces the same claim. Conflicts
are logged but do not immediately demote an entry. After 3 unresolved conflicts
without matching confirmations, the entry is demoted one tier.

### Ebbinghaus Decay with Tier-Specific Half-Lives

Weight at time `t` from creation:
```
w(t) = w₀ × e^(−λt)
```
where `λ = ln(2) / half_life`. Each tier has its own `half_life`. The `prune`
method on `Store` removes entries where `w(t) < threshold` at the current
timestamp.

### Custody Chain

Every knowledge entry carries an audit trail of all agents that produced,
confirmed, or contradicted it. The custody chain is append-only and is
inspectable via `roko knowledge custody list/show/verify`.

---

## 10. Extension System

Source: `/Users/will/dev/nunchi/roko/roko/crates/roko-core/src/extension.rs`

Extensions hook into the agent tick pipeline via an 8-layer hook system.
Every hook has a default no-op implementation; extensions only override what
they need.

### 8 Layers (Execution Order)

| Layer | # | Hooks | Purpose |
|---|---|---|---|
| Foundation | 0 | `on_init`, `on_shutdown` | Lifecycle setup and teardown |
| Perception | 1 | `on_observe`, `on_filter` | Raw input processing |
| Memory | 2 | `on_retrieve`, `on_store` | Knowledge access |
| Cognition | 3 | `pre_inference`, `post_inference`, `on_gate` | LLM interaction |
| Action | 4 | `pre_action`, `post_action`, `on_tool_call` | Tool execution |
| Social | 5 | `on_message_send`, `on_message_receive` | Inter-agent messaging |
| Meta | 6 | `on_reflect`, `on_cost_update` | Self-monitoring |
| Recovery | 7 | `on_error` | Fault handling |

```rust
pub trait Extension: Send + Sync {
    fn name(&self) -> &str;
    fn layer(&self) -> ExtensionLayer;
    fn meta(&self) -> ExtensionMeta;

    // Foundation (0)
    async fn on_init(&mut self) -> Result<()> { Ok(()) }
    async fn on_shutdown(&mut self) -> Result<()> { Ok(()) }

    // Perception (1)
    // on_observe, on_filter...

    // Memory (2)
    // on_retrieve, on_store...

    // Cognition (3)
    async fn pre_inference(&mut self, req: &mut InferenceRequest) -> Result<()> { Ok(()) }
    async fn post_inference(&mut self, resp: &mut InferenceResponse) -> Result<()> { Ok(()) }
    async fn on_gate(&self, event: &GateEvent) -> Result<()> { Ok(()) }

    // Action (4)
    async fn pre_action(&self, ...) -> Result<ActionDecision> { Ok(ActionDecision::Proceed) }
    async fn post_action(&self, ...) -> Result<()> { Ok(()) }
    async fn on_tool_call(&self, event: &ToolCallEvent) -> Result<ToolDecision> {
        Ok(ToolDecision::Allow)
    }

    // Social (5)
    // on_message_send, on_message_receive...

    // Meta (6)
    async fn on_reflect(&self, state: &ReflectionState) -> Result<Vec<Adjustment>> { Ok(vec![]) }
    async fn on_cost_update(&self, update: &CostUpdate) -> Result<()> { Ok(()) }

    // Recovery (7)
    async fn on_error(&self, event: &ErrorEvent) -> Result<RecoveryAction> {
        Ok(RecoveryAction::Propagate)
    }
}
```

### Decision Types

**ActionDecision** (pre-action hook):
- `Proceed` — allow the action to execute
- `Block(reason)` — veto the action with a reason logged to audit trail
- `Rewrite(modified)` — redirect to a modified version of the action

**ToolDecision** (tool-call hook):
- `Allow` — permit the tool call
- `Deny(reason)` — reject with reason
- `Rewrite(args)` — allow with modified arguments

**RecoveryAction** (error hook):
- `Propagate` — surface the error to the caller
- `Retry` — retry the failed operation
- `Skip` — skip the failed step and continue
- `Fallback(value)` — substitute a fallback value

### Extension Metadata

```rust
pub struct ExtensionMeta {
    pub name: String,
    pub layer: ExtensionLayer,
    pub optional: bool,     // if true, errors are logged and ignored
    pub depends_on: Vec<String>,  // must load before this extension
    pub version: String,
}
```

Extensions with `optional = false` (default) propagate errors to the caller.
Extensions with `optional = true` log errors and continue.

### Loading from Manifests

Extensions are declared in `roko.toml` under:
- `[agent.extensions]` — global extensions applied to all roles
- `[agent.roles.<role>.extensions]` — role-specific overrides

Extensions are loaded in dependency order (topological sort of `depends_on`),
then by layer order (Foundation first, Recovery last). Within a layer, they run
in the order listed in the configuration.

### Typed Hook Parameters

All hook parameters are typed structs (`InferenceRequest`, `InferenceResponse`,
`GateEvent`, `ErrorEvent`, `Observation`, `ToolCallEvent`, `CostUpdate`,
`AgentMessage`, `RetrievalResult`, `StoreEntry`, `ReflectionState`). Extensions
receive these by mutable reference for hooks that can modify them
(`pre_inference` can modify `InferenceRequest`, `post_inference` can annotate
`InferenceResponse`), or by shared reference for hooks that only observe.

---

## Appendix: Key File Locations

| What | Path |
|---|---|
| Workspace root | `/Users/will/dev/nunchi/roko/roko/` |
| All crates | `/Users/will/dev/nunchi/roko/roko/crates/` |
| Protocol traits | `crates/roko-core/src/traits.rs` |
| Foundation service traits | `crates/roko-core/src/foundation.rs` |
| Cell supertrait | `crates/roko-core/src/cell.rs` |
| RuntimeEvent | `crates/roko-core/src/runtime_event.rs` |
| Extension system | `crates/roko-core/src/extension.rs` |
| Cognitive workspace | `crates/roko-core/src/cognitive_workspace.rs` |
| Policy manifest | `crates/roko-core/src/policy_manifest.rs` |
| WorkflowEngine | `crates/roko-runtime/src/workflow_engine.rs` |
| PipelineStateV2 | `crates/roko-runtime/src/pipeline_state.rs` |
| EffectDriver | `crates/roko-runtime/src/effect_driver.rs` |
| TaskScheduler | `crates/roko-runtime/src/task_scheduler.rs` |
| EventBus | `crates/roko-runtime/src/event_bus.rs` |
| Orchestrator | `crates/roko-cli/src/orchestrate.rs` |
| Agent dispatcher | `crates/roko-agent/src/dispatcher/mod.rs` |
| Safety layer | `crates/roko-agent/src/safety/` |
| System prompt builder | `crates/roko-compose/src/system_prompt_builder.rs` |
| Role templates | `crates/roko-compose/src/templates/` |
| Builtin roles manifest | `crates/roko-core/src/builtin_roles/core_roles.toml` |
| Roko data directory | `.roko/` |
| Signal log | `.roko/signals.jsonl` |
| Episode log | `.roko/episodes.jsonl` |
| Executor snapshots | `.roko/state/` |
| Learning state | `.roko/learn/` |
| PRD storage | `.roko/prd/` |
| Research artifacts | `.roko/research/` |
| Gap tracker | `.roko/GAPS.md` |

## Appendix: Workspace Members by Layer

```toml
# Primitives
crates/roko-primitives
crates/roko-runtime
crates/roko-core
crates/roko-std
crates/roko-gate
crates/roko-fs
crates/roko-compose
crates/roko-plugin

# Services
crates/roko-agent
crates/roko-orchestrator
crates/roko-conductor
crates/roko-learn
crates/roko-neuro
crates/roko-dreams
crates/roko-daimon

# CLI / Serve / ACP
crates/roko-cli
crates/roko-serve
crates/roko-agent-server
crates/roko-acp

# MCP servers
crates/roko-mcp-stdio
crates/roko-mcp-github
crates/roko-mcp-slack
crates/roko-mcp-scripts
crates/roko-mcp-code

# Language + Index
crates/roko-lang-rust
crates/roko-lang-typescript
crates/roko-lang-go
crates/roko-index

# Chain
crates/roko-chain

# Demo + Apps
crates/roko-demo
apps/mirage-rs
apps/agent-relay
apps/roko-chain-watcher
```
