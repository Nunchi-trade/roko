# Architecture Batch P2D — WorkflowEngine facade

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

## Batch P2D: WorkflowEngine Facade

### Write Scope
- **CREATE**: `crates/roko-runtime/src/workflow_engine.rs`
- **MODIFY**: `crates/roko-runtime/src/lib.rs` (add `pub mod workflow_engine;` and re-export)

### Dependencies
- P2C (EffectDriver, EffectServices)

### DO NOT
- Modify any other files
- Add Cargo.toml dependencies
- Shell out to any CLI
- Put decision logic here — delegate to PipelineStateV2

### Task

Create `WorkflowEngine` — the top-level facade that ties together PipelineStateV2 (state machine)
and EffectDriver (side-effects) into a simple async run loop.

This is the single entry point that CLI, ACP, and HTTP all use to execute a workflow.

#### File: `crates/roko-runtime/src/workflow_engine.rs`

```rust
//! WorkflowEngine — top-level workflow execution facade.
//!
//! Ties together PipelineStateV2 (decisions) and EffectDriver (effects)
//! into a run loop. This is the shared entry point for CLI, ACP, and HTTP.

use anyhow::Result;
use roko_core::foundation::{EventConsumer, FeedbackEvent};
use roko_core::runtime_event::{RuntimeEvent, WorkflowOutcome};
use std::path::PathBuf;
use std::sync::Arc;

use crate::effect_driver::{EffectDriver, EffectServices};
use crate::event_bus::emit_runtime_event;
use crate::pipeline_state::{PipelineStateV2, PipelineInput, PipelineOutput, WorkflowConfig};

/// Configuration for a workflow run.
#[derive(Debug, Clone)]
pub struct WorkflowRunConfig {
    /// User prompt
    pub prompt: String,
    /// Working directory
    pub workdir: PathBuf,
    /// Workflow configuration (express/standard/full)
    pub workflow: WorkflowConfig,
    /// Which gates to run
    pub enabled_gates: Vec<String>,
    /// Commit message prefix
    pub commit_prefix: Option<String>,
}

/// Result of a workflow run.
#[derive(Debug, Clone)]
pub struct WorkflowResult {
    pub run_id: String,
    pub outcome: WorkflowOutcome,
    pub iterations: u32,
}

/// The top-level workflow execution engine.
///
/// Usage:
/// ```ignore
/// let engine = WorkflowEngine::new(services);
/// let result = engine.run(config).await?;
/// ```
pub struct WorkflowEngine {
    services: EffectServices,
    /// Optional event consumers to notify
    consumers: Vec<Arc<dyn EventConsumer>>,
}

impl WorkflowEngine {
    /// Create a new WorkflowEngine with the given services.
    pub fn new(services: EffectServices) -> Self {
        Self {
            services,
            consumers: Vec::new(),
        }
    }

    /// Add an event consumer that will be notified of all RuntimeEvents.
    pub fn add_consumer(&mut self, consumer: Arc<dyn EventConsumer>) {
        self.consumers.push(consumer);
    }

    /// Execute a workflow run.
    ///
    /// This is the main entry point. It:
    /// 1. Creates a PipelineStateV2 from the config
    /// 2. Creates an EffectDriver from the services
    /// 3. Runs the state machine loop: step → execute → feed back → repeat
    /// 4. Returns the outcome
    pub async fn run(&self, config: WorkflowRunConfig) -> Result<WorkflowResult> {
        let run_id = generate_run_id();

        // Initialize state machine
        let mut pipeline = PipelineStateV2::new(
            config.workflow.clone(),
            config.prompt.clone(),
        );

        // Initialize effect driver
        let driver = EffectDriver::new(
            EffectServices {
                model_caller: Arc::clone(&self.services.model_caller),
                prompt_assembler: Arc::clone(&self.services.prompt_assembler),
                feedback_sink: Arc::clone(&self.services.feedback_sink),
                gate_runner: Arc::clone(&self.services.gate_runner),
            },
            run_id.clone(),
            config.workdir.clone(),
        );

        // Emit workflow started
        let template_name = if config.workflow.has_strategy {
            "full"
        } else if config.workflow.has_review {
            "standard"
        } else {
            "express"
        };

        self.emit(RuntimeEvent::WorkflowStarted {
            run_id: run_id.clone(),
            template: template_name.to_string(),
            prompt: config.prompt.clone(),
        });

        // Start the state machine
        let mut output = pipeline.step(PipelineInput::Start);

        // Run loop: execute action → get result → feed back into state machine
        loop {
            let old_phase = pipeline.phase.label();

            let input = match &output {
                PipelineOutput::SpawnStrategist { prompt } => {
                    driver.spawn_agent("strategist", prompt, None).await
                }
                PipelineOutput::SpawnImplementer { prompt, context } => {
                    driver.spawn_agent("implementer", prompt, context.as_deref()).await
                }
                PipelineOutput::SpawnAutoFixer { error_output } => {
                    driver.spawn_agent("autofix", "Fix the following errors", Some(error_output)).await
                }
                PipelineOutput::SpawnReviewer { diff_context } => {
                    driver.spawn_agent("reviewer", "Review the changes", diff_context.as_deref()).await
                }
                PipelineOutput::RunGates => {
                    driver.run_gates(&config.enabled_gates).await
                }
                PipelineOutput::Commit => {
                    let message = config.commit_prefix
                        .as_deref()
                        .map(|p| format!("{}: {}", p, truncate(&config.prompt, 60)))
                        .unwrap_or_else(|| format!("feat: {}", truncate(&config.prompt, 60)));
                    driver.commit(&message).await
                }
                PipelineOutput::Done { outcome } => {
                    self.emit(RuntimeEvent::WorkflowCompleted {
                        run_id: run_id.clone(),
                        outcome: outcome.clone(),
                    });

                    // Record final feedback
                    let _ = self.services.feedback_sink.record(FeedbackEvent::WorkflowComplete {
                        run_id: run_id.clone(),
                        outcome: outcome.to_string(),
                        total_cost_usd: 0.0, // TODO(arch): accumulate from agent calls
                        total_tokens: 0,
                        duration_ms: 0,
                    }).await;

                    return Ok(WorkflowResult {
                        run_id,
                        outcome: outcome.clone(),
                        iterations: pipeline.iteration,
                    });
                }
                PipelineOutput::Halt { reason } => {
                    let outcome = WorkflowOutcome::Halted { reason: reason.clone() };
                    self.emit(RuntimeEvent::WorkflowCompleted {
                        run_id: run_id.clone(),
                        outcome: outcome.clone(),
                    });

                    return Ok(WorkflowResult {
                        run_id,
                        outcome,
                        iterations: pipeline.iteration,
                    });
                }
            };

            // Feed result back into the state machine
            output = pipeline.step(input);
            let new_phase = pipeline.phase.label();

            // Emit phase transition if phase changed
            if old_phase != new_phase {
                self.emit(RuntimeEvent::PhaseTransition {
                    run_id: run_id.clone(),
                    from: old_phase.to_string(),
                    to: new_phase.to_string(),
                });
            }
        }
    }

    fn emit(&self, event: RuntimeEvent) {
        // Notify local consumers
        for consumer in &self.consumers {
            consumer.consume(&event);
        }
        // Emit to global bus
        emit_runtime_event(event);
    }
}

fn generate_run_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("run_{:x}", ts)
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..s.floor_char_boundary(max)]
    }
}
```

#### Modification: `crates/roko-runtime/src/lib.rs`

Add:
```rust
pub mod workflow_engine;
pub use workflow_engine::{WorkflowEngine, WorkflowRunConfig, WorkflowResult};
```

### Done Criteria
```bash
grep -q 'pub struct WorkflowEngine' crates/roko-runtime/src/workflow_engine.rs
grep -q 'pub async fn run' crates/roko-runtime/src/workflow_engine.rs
grep -q 'pub mod workflow_engine' crates/roko-runtime/src/lib.rs
! grep -rn 'Command::new' crates/roko-runtime/src/workflow_engine.rs
cargo check -p roko-runtime
```
