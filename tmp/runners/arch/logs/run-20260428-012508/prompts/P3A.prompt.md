# Architecture Batch P3A — AcpAdapter (EventConsumer → ACP)

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

## Batch P3A: AcpAdapter (EventConsumer → ACP)

### Write Scope
- **CREATE**: `crates/roko-acp/src/acp_adapter.rs`
- **MODIFY**: `crates/roko-acp/src/lib.rs` (add `pub mod acp_adapter;`)

### Dependencies
- P0B (EventConsumer trait)
- P0C (RuntimeEvent bus)

### DO NOT
- Modify any other files (especially not `bridge_events.rs` or `runner.rs`)
- Add Cargo.toml dependencies
- Replace existing ACP event handling — this is an ADDITION
- Create a new crate

### Existing Code Context

The ACP module already has `CognitiveEvent` for its internal event protocol:
```rust
pub enum CognitiveEvent {
    TokenChunk { session_id: String, content: String },
    ToolCallStart { session_id: String, call_id: String, title: String },
    ToolCallComplete { session_id: String, call_id: String, output: String, success: bool },
    PlanUpdate { session_id: String, entries: Vec<PlanEntry> },
    Complete { session_id: String, stop_reason: String },
}
```

### Task

Create `AcpAdapter` — implements `EventConsumer` to bridge `RuntimeEvent`s into the existing
ACP event protocol (`CognitiveEvent`). When the WorkflowEngine emits runtime events, this
adapter maps them to ACP session updates.

#### File: `crates/roko-acp/src/acp_adapter.rs`

```rust
//! AcpAdapter — bridges RuntimeEvent → CognitiveEvent for ACP sessions.
//!
//! Implements EventConsumer to receive workflow events and maps them
//! to the ACP session update protocol.

use roko_core::foundation::EventConsumer;
use roko_core::runtime_event::RuntimeEvent;
use tokio::sync::mpsc;

use crate::bridge_events::CognitiveEvent;
use crate::types::{PlanEntry, PlanStatus, Priority};

/// Adapter that translates RuntimeEvents into ACP CognitiveEvents.
///
/// Created per-session and registered as an EventConsumer on the
/// WorkflowEngine. When the engine emits events, this adapter filters
/// for the relevant run_id and forwards mapped events to the ACP session.
pub struct AcpAdapter {
    /// Session ID this adapter is associated with
    session_id: String,
    /// Run ID to filter events for
    run_id: String,
    /// Channel to send mapped events to the ACP session handler
    sender: mpsc::Sender<CognitiveEvent>,
}

impl AcpAdapter {
    /// Create a new AcpAdapter for the given session and run.
    pub fn new(
        session_id: String,
        run_id: String,
        sender: mpsc::Sender<CognitiveEvent>,
    ) -> Self {
        Self {
            session_id,
            run_id,
            sender,
        }
    }

    /// Map a RuntimeEvent to an optional CognitiveEvent.
    fn map_event(&self, event: &RuntimeEvent) -> Option<CognitiveEvent> {
        // Only process events for our run
        if event.run_id() != self.run_id {
            return None;
        }

        match event {
            RuntimeEvent::AgentOutput { chunk, .. } => {
                Some(CognitiveEvent::TokenChunk {
                    session_id: self.session_id.clone(),
                    content: chunk.clone(),
                })
            }

            RuntimeEvent::AgentSpawned { agent_id, role, .. } => {
                Some(CognitiveEvent::ToolCallStart {
                    session_id: self.session_id.clone(),
                    call_id: agent_id.clone(),
                    title: format!("Agent: {}", role),
                })
            }

            RuntimeEvent::AgentCompleted { agent_id, output, .. } => {
                Some(CognitiveEvent::ToolCallComplete {
                    session_id: self.session_id.clone(),
                    call_id: agent_id.clone(),
                    output: output.clone(),
                    success: true,
                })
            }

            RuntimeEvent::AgentFailed { agent_id, error, .. } => {
                Some(CognitiveEvent::ToolCallComplete {
                    session_id: self.session_id.clone(),
                    call_id: agent_id.clone(),
                    output: error.clone(),
                    success: false,
                })
            }

            RuntimeEvent::GateStarted { gate_name, .. } => {
                Some(CognitiveEvent::ToolCallStart {
                    session_id: self.session_id.clone(),
                    call_id: format!("gate-{}", gate_name),
                    title: format!("Gate: {}", gate_name),
                })
            }

            RuntimeEvent::GatePassed { gate_name, .. } => {
                Some(CognitiveEvent::ToolCallComplete {
                    session_id: self.session_id.clone(),
                    call_id: format!("gate-{}", gate_name),
                    output: format!("✓ {} passed", gate_name),
                    success: true,
                })
            }

            RuntimeEvent::GateFailed { gate_name, output, .. } => {
                Some(CognitiveEvent::ToolCallComplete {
                    session_id: self.session_id.clone(),
                    call_id: format!("gate-{}", gate_name),
                    output: output.clone(),
                    success: false,
                })
            }

            RuntimeEvent::PhaseTransition { from, to, .. } => {
                Some(CognitiveEvent::TokenChunk {
                    session_id: self.session_id.clone(),
                    content: format!("[Phase: {} → {}]\n", from, to),
                })
            }

            RuntimeEvent::WorkflowCompleted { outcome, .. } => {
                Some(CognitiveEvent::Complete {
                    session_id: self.session_id.clone(),
                    stop_reason: outcome.to_string(),
                })
            }

            // Events that don't need ACP mapping
            RuntimeEvent::WorkflowStarted { .. }
            | RuntimeEvent::FeedbackRecorded { .. }
            | RuntimeEvent::StateCheckpointed { .. } => None,
        }
    }
}

impl EventConsumer for AcpAdapter {
    fn consume(&self, event: &RuntimeEvent) {
        if let Some(cognitive_event) = self.map_event(event) {
            // Non-blocking send — if the channel is full, drop the event
            let _ = self.sender.try_send(cognitive_event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_by_run_id() {
        let (tx, _rx) = mpsc::channel(16);
        let adapter = AcpAdapter::new("sess1".into(), "run1".into(), tx);

        // Event for our run — should produce a mapping
        let event = RuntimeEvent::AgentOutput {
            run_id: "run1".into(),
            agent_id: "a1".into(),
            chunk: "hello".into(),
        };
        assert!(adapter.map_event(&event).is_some());

        // Event for different run — should be filtered out
        let event = RuntimeEvent::AgentOutput {
            run_id: "run2".into(),
            agent_id: "a1".into(),
            chunk: "hello".into(),
        };
        assert!(adapter.map_event(&event).is_none());
    }

    #[test]
    fn maps_gate_events() {
        let (tx, _rx) = mpsc::channel(16);
        let adapter = AcpAdapter::new("sess1".into(), "run1".into(), tx);

        let event = RuntimeEvent::GatePassed {
            run_id: "run1".into(),
            gate_name: "compile".into(),
            duration_ms: 1000,
        };

        let mapped = adapter.map_event(&event).unwrap();
        match mapped {
            CognitiveEvent::ToolCallComplete { success, .. } => assert!(success),
            _ => panic!("Expected ToolCallComplete"),
        }
    }
}
```

#### Modification: `crates/roko-acp/src/lib.rs`

Add:
```rust
pub mod acp_adapter;
```

### Done Criteria
```bash
grep -q 'pub struct AcpAdapter' crates/roko-acp/src/acp_adapter.rs
grep -q 'impl EventConsumer for AcpAdapter' crates/roko-acp/src/acp_adapter.rs
grep -q 'pub mod acp_adapter' crates/roko-acp/src/lib.rs
cargo check -p roko-acp
```
