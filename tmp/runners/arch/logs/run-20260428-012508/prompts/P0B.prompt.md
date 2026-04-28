# Architecture Batch P0B — Foundation traits (6 traits)

Run id: run-20260428-012508
Attempt: 1
Model: gpt-5.5
Reasoning: high

---

## Rules (mandatory)

## Mandatory Rules for All Batches

You are an unattended Codex batch agent. There is no prior chat history. This prompt is
entirely self-contained — everything you need is inlined below.

### Execution discipline

1. **Work ONLY within the listed write scope.** Do not modify files outside your scope.
2. **Run verify commands** before declaring success. If `cargo check` fails, fix the errors.
3. **If blocked**, implement the maximum possible and leave a `// TODO(arch): <reason>` comment.
4. **Do NOT create new crates.** All work goes into existing crate directories.
5. **Do NOT add Cargo.toml dependencies** unless the prompt explicitly lists them.
6. **Do NOT modify Cargo.toml** unless the prompt explicitly instructs you to.
7. **Do NOT spawn subagents or delegate.** Each batch is small enough for one agent.
8. **Do NOT create test files** in separate directories. Put `#[cfg(test)] mod tests` at the
   bottom of the implementation file.

### Code quality

9. **No `todo!()` or `unimplemented!()` in public API methods.** Use `Err(...)` or sensible
    defaults instead. Internal helper stubs are acceptable with `// TODO(arch)` markers.
10. **Use `async_trait`** for async trait methods. The crate is already available workspace-wide.
11. **Follow existing naming conventions.** Study the crate's `lib.rs` for style guidance.
12. **All public types need `pub` visibility** and should be re-exported from `lib.rs`.
13. **Prefer `anyhow::Result`** for fallible functions unless the crate uses a custom error type.

### Anti-patterns (condensed — see 03-ANTI-PATTERNS.md for details)

14. **NEVER `Command::new("claude")`** — use `InferenceProvider` / `ModelCallService`.
15. **NEVER `format!("You are the...")`** — use `PromptAssemblyService` / `SystemPromptBuilder`.
16. **NEVER put decision logic in the effect driver** — decisions live in the state machine.
17. **NEVER hardcode roles** — roles come from config or `AgentRole` enum.
18. **NEVER skip feedback recording** — `FeedbackService` must see every model call.
19. **NEVER copy code between entry points** — extract shared services.
20. **NEVER add execution logic to a specific entry point** — use shared services under
    `roko-runtime`, `roko-agent`, `roko-compose`, etc.

---

## Architecture Reference

## Architecture Reference

This is the target architecture. Your implementation must conform to these types and traits.

### RuntimeEvent enum (P0A creates this)

```rust
/// Every event the workflow engine can emit or consume.
/// Observers (ACP adapter, SSE adapter, JSONL logger, TUI) subscribe to these
/// via EventBus<RuntimeEvent>.
#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    // ── lifecycle ──
    WorkflowStarted {
        run_id: String,
        template: String,
        prompt: String,
    },
    PhaseTransition {
        run_id: String,
        from: String,
        to: String,
    },
    WorkflowCompleted {
        run_id: String,
        outcome: WorkflowOutcome,
    },

    // ── agent ──
    AgentSpawned {
        run_id: String,
        agent_id: String,
        role: String,
        model: String,
    },
    AgentOutput {
        run_id: String,
        agent_id: String,
        chunk: String,
    },
    AgentCompleted {
        run_id: String,
        agent_id: String,
        output: String,
        tokens_used: u64,
        cost_usd: f64,
    },
    AgentFailed {
        run_id: String,
        agent_id: String,
        error: String,
    },

    // ── gates ──
    GateStarted {
        run_id: String,
        gate_name: String,
        rung: u8,
    },
    GatePassed {
        run_id: String,
        gate_name: String,
        duration_ms: u64,
    },
    GateFailed {
        run_id: String,
        gate_name: String,
        output: String,
        duration_ms: u64,
    },

    // ── feedback ──
    FeedbackRecorded {
        run_id: String,
        kind: String,
        summary: String,
    },

    // ── persistence ──
    StateCheckpointed {
        run_id: String,
        path: String,
    },
}

#[derive(Debug, Clone)]
pub enum WorkflowOutcome {
    Success { commit_hash: Option<String> },
    Halted { reason: String },
    Cancelled,
}
```

### Foundation Traits (P0B creates these)

```rust
/// Call an LLM model. Wraps provider selection, streaming, cost tracking.
#[async_trait]
pub trait ModelCaller: Send + Sync {
    async fn call(&self, req: ModelCallRequest) -> Result<ModelCallResponse>;
    async fn stream(&self, req: ModelCallRequest) -> Result<ModelCallStream>;
}

/// Assemble a system prompt for a given role and context.
#[async_trait]
pub trait PromptAssembler: Send + Sync {
    async fn assemble(&self, spec: PromptSpec) -> Result<String>;
}

/// Record feedback from model calls, gate results, and workflow outcomes.
#[async_trait]
pub trait FeedbackSink: Send + Sync {
    async fn record(&self, event: FeedbackEvent) -> Result<()>;
}

/// Run a set of verification gates against a working directory.
#[async_trait]
pub trait GateRunner: Send + Sync {
    async fn run_gates(&self, config: GateConfig) -> Result<GateReport>;
}

/// Consume RuntimeEvents for side-effects (logging, UI updates, etc).
pub trait EventConsumer: Send + Sync {
    fn consume(&self, event: &RuntimeEvent);
}

/// Execute a side-effect (spawn agent, run gate, commit, etc).
/// The effect driver calls these; the state machine decides WHAT to do,
/// the EffectExecutor decides HOW.
#[async_trait]
pub trait EffectExecutor: Send + Sync {
    async fn execute(&self, effect: Effect) -> Result<EffectOutcome>;
}
```

### Crate Dependency Map

```
roko-core          (no internal deps — kernel types + traits)
    ↑
roko-runtime       (depends on: roko-core)
    ↑
roko-agent         (depends on: roko-core)
roko-compose       (depends on: roko-core)
roko-learn         (depends on: roko-core)
roko-gate          (depends on: roko-core)
    ↑
roko-acp           (depends on: roko-core, roko-runtime, roko-agent, roko-gate, roko-compose)
roko-serve         (depends on: roko-core, roko-runtime, roko-agent, roko-gate, roko-serve)
roko-cli           (depends on: everything)
```

### Key Existing Types

These types already exist in `roko-core` and should be used, not recreated:

- `Engram` — universal signal type (hash, content, metadata, lineage)
- `Context` — execution context (working directory, config, cancel token)
- `AgentRole` — enum of agent roles (Implementer, Strategist, Reviewer, etc.)
- `Verdict` — gate verdict (pass/fail/skip with details)
- `ModelTier` — model capability tier (Fast, Standard, Premium, Research)
- `ProviderKind` — LLM provider (Claude, OpenAi, Ollama, Gemini, Perplexity)
- `Temperament` — agent behavior dial (Conservative, Balanced, Aggressive, Exploratory)
- `CancelToken` — cooperative cancellation (in roko-runtime)

### Key Existing Infrastructure

- `EventBus<E>` in `roko-runtime::event_bus` — typed broadcast with replay ring
- `SystemPromptBuilder` in `roko-compose` — 9-layer prompt assembly
- `InferenceProvider` trait in `roko-agent` — LLM backend abstraction
- `Verify` trait in `roko-gate` — gate execution trait
- `AdaptiveThresholds` in `roko-gate` — per-rung adaptive gate skipping
- `EpisodeLogger` in `roko-learn` — append-only JSONL episode recording
- `CascadeRouter` in `roko-learn` — model routing with learning
- `PipelineState` in `roko-acp::pipeline` — existing 9-state machine (ACP-specific)
- `WorkflowRun` in `roko-acp::workflow` — existing run tracking struct

---

## Anti-Patterns (DO NOT violate)

## Anti-Patterns — DO NOT Do These

These are concrete examples of code that has been written in this codebase before and caused
problems. Each one has a "BAD" example and a "GOOD" replacement.

### AP-1: Never shell out to claude CLI

**BAD** (actual code found in codebase):
```rust
let output = Command::new("claude")
    .arg("--print")
    .arg("--dangerously-skip-permissions")
    .arg(&prompt)
    .current_dir(&workdir)
    .output()
    .await?;
```

**GOOD** — use the provider abstraction:
```rust
let response = model_caller.call(ModelCallRequest {
    model: model_spec,
    messages: vec![...],
    ..Default::default()
}).await?;
```

**Why**: Shelling out bypasses cost tracking, feedback recording, provider health monitoring,
rate limiting, and model routing. It also makes the code untestable without a real claude binary.

---

### AP-2: Never inline prompt strings

**BAD**:
```rust
let prompt = format!(
    "You are the {} agent. Your task is to {}. \
     Follow these conventions: {}",
    role, task, conventions
);
```

**GOOD** — use PromptAssemblyService:
```rust
let prompt = prompt_assembler.assemble(PromptSpec {
    role: AgentRole::Implementer,
    task_context: task.clone(),
    ..Default::default()
}).await?;
```

**Why**: Inline prompts skip the 9-layer system prompt builder, miss anti-patterns, miss
conventions, miss gate feedback from prior iterations, and can't be A/B tested.

---

### AP-3: Never add execution logic to a specific entry point

**BAD** — putting gate execution in the CLI:
```rust
// in roko-cli/src/run.rs
async fn run_gates(workdir: &Path) -> Result<()> {
    let compile = CompileGate;
    let test = TestGate;
    compile.verify(&signal, &ctx).await?;
    test.verify(&signal, &ctx).await?;
}
```

**GOOD** — use the shared GateService:
```rust
// in roko-cli/src/run.rs
let report = gate_runner.run_gates(GateConfig {
    workdir: workdir.to_path_buf(),
    enabled_gates: vec!["compile", "test"],
    ..Default::default()
}).await?;
```

**Why**: If gate logic lives in the CLI, the ACP server and HTTP API can't use it. Extract
to a shared service so all entry points (CLI, ACP, HTTP) get the same behavior.

---

### AP-4: Never put decisions in the effect driver

**BAD**:
```rust
impl EffectDriver {
    async fn handle_gate_result(&self, result: &GateReport) {
        if result.all_passed() {
            self.run_reviewer().await;  // Decision!
        } else if self.iteration < self.max_iterations {
            self.run_autofix().await;   // Decision!
        } else {
            self.halt("too many failures"); // Decision!
        }
    }
}
```

**GOOD** — decisions in the state machine, effects in the driver:
```rust
// State machine decides:
let action = pipeline_state.step(PipelineEvent::GatesPassed);
// Effect driver executes:
match action {
    PipelineAction::SpawnReviewer { .. } => effect_driver.spawn_agent(...).await,
    PipelineAction::SpawnAutoFixer { .. } => effect_driver.spawn_agent(...).await,
    PipelineAction::Halt { reason } => effect_driver.halt(reason).await,
}
```

**Why**: When decisions are in the effect driver, the state machine becomes meaningless and
the workflow can't be tested without real side-effects. Pure state machines are testable.

---

### AP-5: Never hardcode roles

**BAD**:
```rust
if role == "implementer" {
    model = "claude-sonnet-4-20250514";
} else if role == "reviewer" {
    model = "claude-opus-4-20250514";
}
```

**GOOD** — roles come from config:
```rust
let model = cascade_router.select(&TaskRequirements {
    role: spec.role.clone(),
    complexity: spec.complexity,
    ..Default::default()
});
```

**Why**: Hardcoded roles break when users configure different models or add custom roles.

---

### AP-6: Never skip feedback recording

**BAD**:
```rust
let response = provider.call(request).await?;
// Just use the response directly, no recording
process_response(response);
```

**GOOD**:
```rust
let response = model_caller.call(request).await?;
// ModelCallService internally records to FeedbackSink
// Or explicitly:
feedback_sink.record(FeedbackEvent::ModelCall {
    model: request.model.clone(),
    tokens: response.usage.total_tokens,
    cost_usd: response.usage.cost_usd,
    latency_ms: elapsed.as_millis() as u64,
}).await?;
```

**Why**: Without feedback, the cascade router can't learn, efficiency metrics are wrong,
and cost tracking is blind.

---

### AP-7: Never copy code between entry points

**BAD** — duplicating gate logic across CLI and ACP:
```rust
// roko-cli/src/run.rs
async fn cli_run_gates() { /* 50 lines of gate logic */ }

// roko-acp/src/runner.rs
async fn acp_run_gates() { /* same 50 lines, slightly different */ }
```

**GOOD** — shared service:
```rust
// roko-gate/src/gate_service.rs
pub struct GateService { /* ... */ }
impl GateRunner for GateService { /* ... */ }

// Both CLI and ACP use:
let report = gate_service.run_gates(config).await?;
```

**Why**: Duplicated code drifts. When one copy gets a fix, the other doesn't.

---

### AP-8: Never modify files outside your write scope

Your prompt specifies an exact write scope (e.g., "create `model_call_service.rs`, add mod
decl to `lib.rs`"). Do not modify other files, even if they would benefit from it.

---

### AP-9: Never create new crates

All 18 crates already exist. Your code goes into existing crate directories. If you think
you need a new crate, you are wrong — find the right existing crate.

---

### AP-10: Never use `todo!()` in public APIs

```rust
// BAD
pub async fn stream(&self, req: ModelCallRequest) -> Result<ModelCallStream> {
    todo!()
}

// GOOD — return an error or basic implementation
pub async fn stream(&self, req: ModelCallRequest) -> Result<ModelCallStream> {
    // Streaming not yet implemented; fall back to single call
    let response = self.call(req).await?;
    Ok(ModelCallStream::from_complete(response))
}
```

---

## Batch P0B: Foundation Traits

### Write Scope
- **CREATE**: `crates/roko-core/src/foundation.rs`
- **MODIFY**: `crates/roko-core/src/lib.rs` (add `pub mod foundation;` and re-exports)

### Dependencies
- P0A must be complete (RuntimeEvent types exist in `roko-core/src/runtime_event.rs`)

### DO NOT
- Modify any other files
- Add Cargo.toml dependencies
- Create new crates
- Duplicate types that already exist in `roko-core` (Engram, Verdict, AgentRole, etc.)

### Task

Create 6 foundation traits in `crates/roko-core/src/foundation.rs`. These traits define
the contracts between the workflow engine and its services. Each service crate (roko-agent,
roko-compose, roko-learn, roko-gate) will implement one of these traits.

#### File: `crates/roko-core/src/foundation.rs`

```rust
//! Foundation traits for the workflow engine.
//!
//! These define the contracts between the engine and its services:
//! - `ModelCaller` — call LLMs (implemented by roko-agent)
//! - `PromptAssembler` — build system prompts (implemented by roko-compose)
//! - `FeedbackSink` — record feedback (implemented by roko-learn)
//! - `GateRunner` — run verification gates (implemented by roko-gate)
//! - `EventConsumer` — observe runtime events (implemented by adapters)
//! - `EffectExecutor` — execute side-effects (implemented by roko-runtime)

use crate::runtime_event::RuntimeEvent;
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;

// ── ModelCaller ──

/// Request to call an LLM model.
#[derive(Debug, Clone)]
pub struct ModelCallRequest {
    /// Model identifier (e.g., "claude-sonnet-4-20250514")
    pub model: String,
    /// System prompt
    pub system: Option<String>,
    /// User messages
    pub messages: Vec<ChatMessage>,
    /// Maximum tokens to generate
    pub max_tokens: Option<u32>,
    /// Temperature (0.0–1.0)
    pub temperature: Option<f32>,
    /// Role for model routing
    pub role: Option<String>,
}

/// A single chat message.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

/// Message role in a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

/// Response from a model call.
#[derive(Debug, Clone)]
pub struct ModelCallResponse {
    pub content: String,
    pub model: String,
    pub usage: TokenUsage,
    pub stop_reason: Option<String>,
}

/// Token usage and cost from a model call.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

/// Call an LLM model. Wraps provider selection, streaming, cost tracking.
#[async_trait]
pub trait ModelCaller: Send + Sync {
    /// Single-shot model call, returns complete response.
    async fn call(&self, req: ModelCallRequest) -> Result<ModelCallResponse>;
}

// ── PromptAssembler ──

/// Specification for assembling a system prompt.
#[derive(Debug, Clone, Default)]
pub struct PromptSpec {
    /// Agent role (determines identity layer)
    pub role: Option<String>,
    /// Task description
    pub task: Option<String>,
    /// Working directory for convention detection
    pub workdir: Option<PathBuf>,
    /// Gate feedback from prior iterations
    pub gate_feedback: Vec<String>,
    /// Anti-patterns to include
    pub anti_patterns: Vec<String>,
}

/// Assemble a system prompt for a given role and context.
#[async_trait]
pub trait PromptAssembler: Send + Sync {
    /// Build a complete system prompt from the spec.
    async fn assemble(&self, spec: PromptSpec) -> Result<String>;
}

// ── FeedbackSink ──

/// A feedback event to record.
#[derive(Debug, Clone)]
pub enum FeedbackEvent {
    /// Feedback from a model call.
    ModelCall {
        run_id: String,
        model: String,
        role: String,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        latency_ms: u64,
        success: bool,
    },
    /// Feedback from a gate execution.
    GateResult {
        run_id: String,
        gate_name: String,
        passed: bool,
        duration_ms: u64,
    },
    /// Feedback from a workflow completion.
    WorkflowComplete {
        run_id: String,
        outcome: String,
        total_cost_usd: f64,
        total_tokens: u64,
        duration_ms: u64,
    },
}

/// Record feedback from model calls, gate results, and workflow outcomes.
#[async_trait]
pub trait FeedbackSink: Send + Sync {
    /// Record a feedback event.
    async fn record(&self, event: FeedbackEvent) -> Result<()>;
}

// ── GateRunner ──

/// Configuration for a gate run.
#[derive(Debug, Clone)]
pub struct GateConfig {
    /// Working directory to verify
    pub workdir: PathBuf,
    /// Which gates to run (e.g., ["compile", "test", "clippy"])
    pub enabled_gates: Vec<String>,
    /// Maximum rung to run (0–6)
    pub max_rung: Option<u8>,
}

/// Result from a single gate.
#[derive(Debug, Clone)]
pub struct GateVerdict {
    pub gate_name: String,
    pub passed: bool,
    pub output: String,
    pub duration_ms: u64,
}

/// Report from running a set of gates.
#[derive(Debug, Clone)]
pub struct GateReport {
    pub verdicts: Vec<GateVerdict>,
}

impl GateReport {
    /// Returns true if all gates passed.
    pub fn all_passed(&self) -> bool {
        self.verdicts.iter().all(|v| v.passed)
    }

    /// Returns the first failing gate, if any.
    pub fn first_failure(&self) -> Option<&GateVerdict> {
        self.verdicts.iter().find(|v| !v.passed)
    }

    /// Collects all failure outputs for agent feedback.
    pub fn failure_summary(&self) -> String {
        self.verdicts
            .iter()
            .filter(|v| !v.passed)
            .map(|v| format!("{}: {}", v.gate_name, v.output))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Run a set of verification gates against a working directory.
#[async_trait]
pub trait GateRunner: Send + Sync {
    /// Execute gates per the config, returning a report.
    async fn run_gates(&self, config: GateConfig) -> Result<GateReport>;
}

// ── EventConsumer ──

/// Consume RuntimeEvents for side-effects (logging, UI updates, etc).
///
/// Consumers must be non-blocking. If they need async work, they should
/// buffer internally and process asynchronously.
pub trait EventConsumer: Send + Sync {
    /// Called for each event emitted by the workflow engine.
    fn consume(&self, event: &RuntimeEvent);
}

// ── EffectExecutor ──

/// A side-effect the workflow engine needs to execute.
#[derive(Debug, Clone)]
pub enum Effect {
    /// Spawn an agent with the given role and prompt.
    SpawnAgent {
        run_id: String,
        role: String,
        model: String,
        system_prompt: String,
        user_prompt: String,
        workdir: PathBuf,
    },
    /// Run verification gates.
    RunGates {
        run_id: String,
        config: GateConfig,
    },
    /// Create a git commit.
    Commit {
        run_id: String,
        workdir: PathBuf,
        message: String,
    },
    /// Persist a state checkpoint.
    Checkpoint {
        run_id: String,
        state_json: String,
        path: PathBuf,
    },
}

/// Outcome from executing an effect.
#[derive(Debug, Clone)]
pub enum EffectOutcome {
    /// Agent completed with output.
    AgentDone {
        agent_id: String,
        output: String,
        tokens_used: u64,
        cost_usd: f64,
        files_changed: Vec<String>,
    },
    /// Gates completed.
    GatesDone {
        report: GateReport,
    },
    /// Commit created.
    CommitDone {
        hash: String,
        message: String,
    },
    /// Checkpoint saved.
    CheckpointDone {
        path: String,
    },
    /// Effect failed.
    Failed {
        error: String,
    },
}

/// Execute a side-effect (spawn agent, run gates, commit, checkpoint).
///
/// The state machine decides WHAT to do; the EffectExecutor decides HOW.
#[async_trait]
pub trait EffectExecutor: Send + Sync {
    /// Execute the given effect, returning the outcome.
    async fn execute(&self, effect: Effect) -> Result<EffectOutcome>;
}
```

#### Modification: `crates/roko-core/src/lib.rs`

Add near the other `pub mod` declarations:
```rust
pub mod foundation;
```

Add to the re-export block (key types only — keep it manageable):
```rust
pub use foundation::{
    ModelCaller, ModelCallRequest, ModelCallResponse, TokenUsage,
    ChatMessage, MessageRole,
    PromptAssembler, PromptSpec,
    FeedbackSink, FeedbackEvent,
    GateRunner, GateConfig, GateReport, GateVerdict,
    EventConsumer,
    EffectExecutor, Effect, EffectOutcome,
};
```

### Done Criteria
```bash
grep -q 'pub trait ModelCaller' crates/roko-core/src/foundation.rs
grep -q 'pub trait PromptAssembler' crates/roko-core/src/foundation.rs
grep -q 'pub trait FeedbackSink' crates/roko-core/src/foundation.rs
grep -q 'pub trait GateRunner' crates/roko-core/src/foundation.rs
grep -q 'pub trait EventConsumer' crates/roko-core/src/foundation.rs
grep -q 'pub trait EffectExecutor' crates/roko-core/src/foundation.rs
grep -q 'pub mod foundation' crates/roko-core/src/lib.rs
cargo check -p roko-core
```
