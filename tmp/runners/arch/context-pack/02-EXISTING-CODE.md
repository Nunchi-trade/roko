## Existing Code Reference

This file contains excerpts of existing code that batches must extend (not replace).
Each batch prompt inlines the specific excerpts it needs. This file is the master index.

### roko-core/src/lib.rs — Module Structure

The kernel crate exports types via `pub mod` declarations. New modules (like `runtime_event`
and `foundation`) must be added the same way: declare `pub mod X;` and re-export key types.

Pattern:
```rust
pub mod runtime_event;  // P0A adds this
pub mod foundation;     // P0B adds this

// Re-exports at crate root:
pub use runtime_event::{RuntimeEvent, WorkflowOutcome};
pub use foundation::{ModelCaller, PromptAssembler, FeedbackSink, GateRunner, EventConsumer, EffectExecutor};
```

### roko-runtime/src/event_bus.rs — EventBus<E>

Existing generic event bus. P0C adds `RuntimeEvent` support:

```rust
pub struct EventBus<E: Clone + Send + 'static> {
    tx: broadcast::Sender<Envelope<E>>,
    seq: Arc<AtomicU64>,
    ring: Arc<Mutex<VecDeque<Envelope<E>>>>,
    capacity: usize,
}

impl<E: Clone + Send + 'static> EventBus<E> {
    pub fn new(capacity: usize) -> Self;
    pub fn emit(&self, event: E) -> u64;
    pub fn subscribe(&self) -> broadcast::Receiver<Envelope<E>>;
    pub fn replay_from(&self, since_seq: u64) -> Vec<Envelope<E>>;
    pub fn total_emitted(&self) -> u64;
}

// Existing singleton:
pub fn global_event_bus() -> &'static EventBus<RokoEvent>
```

P0C should add a parallel singleton for RuntimeEvent:
```rust
pub fn runtime_event_bus() -> &'static EventBus<RuntimeEvent>
```

### roko-agent — Provider Dispatch

Existing provider abstraction that ModelCallService wraps:

```rust
// roko-agent/src/lib.rs re-exports:
pub use provider::{ProviderAdapter, adapter_for_kind, create_agent_for_model};

// The adapter_for_kind function creates a provider adapter for a given ProviderKind.
// ModelCallService should use this internally, NOT shell out to claude.
```

### roko-compose — SystemPromptBuilder

Existing 9-layer prompt builder that PromptAssemblyService wraps:

```rust
pub struct SystemPromptBuilder {
    role_identity: String,
    conventions: Option<String>,
    domain: Option<String>,
    context: Option<String>,
    task: Option<String>,
    gate_feedback: Vec<String>,
    tools: Option<String>,
    anti_patterns: Vec<String>,
    // ... more layers
}

impl SystemPromptBuilder {
    pub fn new(role_identity: &str) -> Self;
    pub fn with_conventions(self, text: &str) -> Self;
    pub fn with_domain(self, text: &str) -> Self;
    pub fn with_task(self, text: &str) -> Self;
    pub fn build(self) -> String;
}
```

### roko-gate — Verify Trait

Existing gate trait that GateService wraps:

```rust
#[async_trait]
pub trait Verify: Send + Sync {
    async fn verify(&self, signal: &Engram, ctx: &Context) -> Result<Verdict>;
    fn name(&self) -> &str;
}

// Concrete gates:
pub struct CompileGate;  // rung 0
pub struct ClippyGate;   // rung 1
pub struct TestGate;     // rung 2
```

### roko-learn — Episode + Efficiency

Existing feedback infrastructure that FeedbackService wraps:

```rust
// EpisodeLogger: append-only JSONL
pub struct EpisodeLogger { /* ... */ }
impl EpisodeLogger {
    pub fn append(&self, episode: &Episode) -> Result<()>;
}

// CascadeRouter: model routing
pub struct CascadeRouter { /* ... */ }
impl CascadeRouter {
    pub fn select(&self, requirements: &TaskRequirements) -> ModelSpec;
    pub fn record_outcome(&mut self, spec: &ModelSpec, outcome: &TaskOutcome) -> Result<()>;
}
```

### roko-acp — Pipeline + Runner (existing ACP-specific code)

P3A (AcpAdapter) and P4B consume RuntimeEvents and map them to ACP session updates.
The existing code in `bridge_events.rs` already has a `CognitiveEvent` enum — the adapter
should bridge between `RuntimeEvent` and the existing ACP protocol.

```rust
// Existing in bridge_events.rs:
pub enum CognitiveEvent {
    TokenChunk { session_id: String, content: String },
    ToolCallStart { session_id: String, call_id: String, title: String },
    ToolCallComplete { session_id: String, call_id: String, output: String, success: bool },
    PlanUpdate { session_id: String, entries: Vec<PlanEntry> },
    Complete { session_id: String, stop_reason: String },
}
```

### roko-cli/src/run.rs — CLI Entry Point

The `run` subcommand currently uses `orchestrate.rs` directly. P4A wires it to use
`WorkflowEngine` as an alternative path (behind a flag or config option).
