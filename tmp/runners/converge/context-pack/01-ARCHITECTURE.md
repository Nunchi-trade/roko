## Architecture Reference

This is the current architecture after the arch runner (Phases 0-3). Your implementation
must conform to and extend these types and traits.

### RuntimeEvent enum (in roko-core/src/runtime_event.rs)

```rust
#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    WorkflowStarted  { run_id: String, template: String, prompt: String },
    PhaseTransition  { run_id: String, from: String, to: String },
    WorkflowCompleted{ run_id: String, outcome: WorkflowOutcome },
    AgentSpawned     { run_id: String, agent_id: String, role: String, model: String },
    AgentOutput      { run_id: String, agent_id: String, chunk: String },
    AgentCompleted   { run_id: String, agent_id: String, output: String, tokens_used: u64, cost_usd: f64 },
    AgentFailed      { run_id: String, agent_id: String, error: String },
    GateStarted      { run_id: String, gate_name: String, rung: u8 },
    GatePassed       { run_id: String, gate_name: String, duration_ms: u64 },
    GateFailed       { run_id: String, gate_name: String, output: String, duration_ms: u64 },
    FeedbackRecorded { run_id: String, kind: String, summary: String },
    StateCheckpointed{ run_id: String, path: String },
}

#[derive(Debug, Clone)]
pub enum WorkflowOutcome {
    Success { commit_hash: Option<String> },
    Halted { reason: String },
    Cancelled,
}

impl RuntimeEvent {
    pub fn run_id(&self) -> &str { ... }
    pub fn kind(&self) -> &'static str { ... }
}
```

### Foundation Traits (in roko-core/src/foundation.rs)

```rust
// --- Request/Response types ---
pub struct ModelCallRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub role: Option<String>,
}

pub struct ChatMessage { pub role: MessageRole, pub content: String }
pub enum MessageRole { System, User, Assistant }

pub struct ModelCallResponse {
    pub content: String,
    pub model: String,
    pub usage: TokenUsage,
    pub stop_reason: Option<String>,
}

pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

pub struct PromptSpec {
    pub role: Option<String>,
    pub task: Option<String>,
    pub workdir: Option<PathBuf>,
    pub gate_feedback: Vec<String>,
    pub anti_patterns: Vec<String>,
}

pub enum FeedbackEvent {
    ModelCall { run_id: String, model: String, role: Option<String>,
               input_tokens: u64, output_tokens: u64, cost_usd: f64,
               latency_ms: u64, success: bool },
    GateResult { run_id: String, gate_name: String, passed: bool, duration_ms: u64 },
    WorkflowComplete { run_id: String, outcome: String, total_cost_usd: f64,
                       total_tokens: u64, duration_ms: u64 },
}

pub struct GateConfig {
    pub workdir: PathBuf,
    pub enabled_gates: Vec<String>,
    pub max_rung: Option<u8>,
}

pub struct GateVerdict { pub gate_name: String, pub passed: bool, pub output: String, pub duration_ms: u64 }
pub struct GateReport { pub verdicts: Vec<GateVerdict> }

pub enum Effect {
    SpawnAgent { run_id: String, role: String, model: String, system_prompt: String, user_prompt: String, workdir: PathBuf },
    RunGates   { run_id: String, config: GateConfig },
    Commit     { run_id: String, workdir: PathBuf, message: String },
    Checkpoint { run_id: String, state_json: String, path: PathBuf },
}

pub enum EffectOutcome {
    AgentDone     { agent_id: String, output: String, tokens_used: u64, cost_usd: f64, files_changed: Vec<String> },
    GatesDone     { report: GateReport },
    CommitDone    { hash: String, message: String },
    CheckpointDone{ path: String },
    Failed        { error: String },
}

// --- Traits ---
#[async_trait] pub trait ModelCaller: Send + Sync {
    async fn call(&self, req: ModelCallRequest) -> Result<ModelCallResponse>;
}
#[async_trait] pub trait PromptAssembler: Send + Sync {
    async fn assemble(&self, spec: PromptSpec) -> Result<String>;
}
#[async_trait] pub trait FeedbackSink: Send + Sync {
    async fn record(&self, event: FeedbackEvent) -> Result<()>;
}
#[async_trait] pub trait GateRunner: Send + Sync {
    async fn run_gates(&self, config: GateConfig) -> Result<GateReport>;
}
pub trait EventConsumer: Send + Sync {
    fn consume(&self, event: &RuntimeEvent);
}
#[async_trait] pub trait EffectExecutor: Send + Sync {
    async fn execute(&self, effect: Effect) -> Result<EffectOutcome>;
}

impl GateReport {
    pub fn all_passed(&self) -> bool { ... }
    pub fn first_failure(&self) -> Option<&GateVerdict> { ... }
    pub fn failure_summary(&self) -> String { ... }
}
```

### Existing Concrete Services

| Service | Crate | Traits Implemented | Status |
|---------|-------|--------------------|--------|
| `ModelCallService` | `roko-agent` | `ModelCaller` (roko-core) | Stub — needs real provider dispatch |
| `PromptAssemblyService` | `roko-compose` | `PromptAssembler` (roko-core) | Basic — needs full 9-layer builder |
| `FeedbackService` | `roko-learn` | `FeedbackSink` (roko-core) | Works — writes efficiency.jsonl |
| `GateService` | `roko-gate` | `GateRunner` (roko-core) | Partial — rungs 0-2 only |
| `JsonlLogger` | `roko-runtime` | `EventConsumer` (LOCAL copy) | Works — needs roko-core trait |
| `RuntimeProjection` | `roko-runtime` | (stateless reader) | Partial — 5 of 12 variants |

### Execution Engine (in roko-runtime)

```rust
// PipelineStateV2 — pure state machine, no I/O
pub struct PipelineStateV2 { pub phase: Phase, ... }
impl PipelineStateV2 {
    pub fn new(config: WorkflowConfig, prompt: String) -> Self;
    pub fn step(&mut self, input: PipelineInput) -> PipelineOutput;
}

// EffectDriver — executes side effects
pub struct EffectDriver { services: EffectServices, ... }
impl EffectDriver {
    pub fn new(services: EffectServices, run_id: String, workdir: PathBuf) -> Self;
    pub async fn spawn_agent(&self, role: &str, user_prompt: &str, context: Option<&str>) -> PipelineInput;
    pub async fn run_gates(&self, enabled_gates: &[String]) -> PipelineInput;
    pub async fn commit(&self, message: &str) -> PipelineInput;
}

// EffectServices — dependency injection bundle
pub struct EffectServices {
    pub model_caller: Arc<dyn ModelCaller>,       // Currently: LOCAL trait
    pub prompt_assembler: Arc<dyn PromptAssembler>, // Currently: LOCAL trait
    pub feedback_sink: Arc<dyn FeedbackSink>,     // Currently: LOCAL trait
    pub gate_runner: Arc<dyn GateRunner>,         // Currently: LOCAL trait
}
// ^^^ Track F fixes these to use roko-core traits

// WorkflowEngine — top-level facade
pub struct WorkflowEngine { services: EffectServices, consumers: Vec<Arc<dyn WorkflowEventConsumer>> }
impl WorkflowEngine {
    pub fn new(services: EffectServices) -> Self;
    pub fn add_consumer(&mut self, consumer: Arc<dyn WorkflowEventConsumer>);
    pub async fn run(&self, config: WorkflowRunConfig) -> Result<WorkflowResult>;
}
```

### Crate Dependency Map (CURRENT — Track F01 fixes the cycle)

```
roko-core          (CURRENTLY depends on roko-runtime — WRONG, F01 removes this)
    ↑
roko-runtime       (depends on: roko-core after F01)
    ↑
roko-agent         (depends on: roko-core)
roko-compose       (depends on: roko-core)
roko-learn         (depends on: roko-core)
roko-gate          (depends on: roko-core)
    ↑
roko-acp           (depends on: roko-core, roko-runtime, roko-agent, roko-gate, roko-compose)
roko-serve         (depends on: roko-core, roko-runtime)
roko-cli           (depends on: everything)
```

### Key Existing Infrastructure (do NOT recreate)

- `EventBus<E>` in `roko-runtime::event_bus` — typed broadcast with replay ring
- `SystemPromptBuilder` in `roko-compose` — 9-layer prompt assembly
- `InferenceProvider` trait in `roko-agent` — LLM backend abstraction
- `adapter_for_kind()` in `roko-agent` — creates backend for a ProviderKind
- `create_agent_for_model()` in `roko-agent` — model string → provider
- `Verify` trait in `roko-gate` — gate execution trait
- `CompileGate`, `TestGate`, `ClippyGate` in `roko-gate` — existing gate impls
- `AdaptiveThresholds` in `roko-gate` — per-rung adaptive gate skipping
- `EpisodeLogger` in `roko-learn` — append-only JSONL episode recording
- `CascadeRouter` in `roko-learn` — model routing with LinUCB bandit learning
- `PlaybookStore` in `roko-learn` — proven action sequences with success scores
- `ExperimentStore` in `roko-learn` — prompt A/B testing
- `ConductorBandit` in `roko-learn` — retry strategy learning
- `KnowledgeStore` in `roko-neuro` — durable knowledge with tier progression
- `StateHub` in `roko-runtime` — push-based dashboard event hub
- `ProcessSupervisor` in `roko-runtime` — agent lifecycle management
- `CancelToken` in `roko-runtime` — cooperative cancellation

### Key CLI Entry Points

```rust
// roko-cli/src/run.rs
pub async fn run_once(workdir, config, prompt, hub) -> Result<RunReport>;  // CURRENT active path
pub async fn run_with_workflow_engine(prompt, workdir, template, gates) -> Result<()>; // NEW path (not yet wired)

// roko-cli/src/orchestrate.rs (21,577 lines — will be feature-gated)
pub struct PlanRunner { ... } // ~70 fields
impl PlanRunner {
    pub async fn run_all(&mut self, cancel) -> Result<OrchestrationReport>; // main tick loop
    pub async fn from_plans_dir(dir, workdir, config, ...) -> Result<Self>;
}

// roko-cli/src/dispatch_direct.rs (fast path for unified chat)
pub async fn dispatch_prompt(auth, prompt) -> Result<DispatchResult>;
```
