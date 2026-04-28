# Architecture Batch P2A — PipelineState v2 (config-driven)

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

## Batch P2A: PipelineState v2 (Config-Driven)

### Write Scope
- **CREATE**: `crates/roko-runtime/src/pipeline_state.rs`
- **MODIFY**: `crates/roko-runtime/src/lib.rs` (add `pub mod pipeline_state;` and re-export)

### Dependencies
- P0A (RuntimeEvent types — for WorkflowOutcome)

### DO NOT
- Modify any other files
- Modify the existing PipelineState in `roko-acp/src/pipeline.rs` (that's ACP-specific)
- Add Cargo.toml dependencies
- Put side-effects in the state machine (this is a PURE state machine)

### Context

There is already a `PipelineState` in `roko-acp/src/pipeline.rs`. That one is ACP-specific.
This new `PipelineStateV2` is the shared, entry-point-agnostic state machine that all
consumers (CLI, ACP, HTTP) will use. It lives in `roko-runtime` because it's infrastructure.

The key difference: the ACP pipeline has 9 phases with ACP-specific concerns baked in.
PipelineStateV2 is config-driven — which phases to include (strategy, review, etc.) comes
from a `WorkflowConfig` struct, not hardcoded logic.

### Task

Create a pure state machine `PipelineStateV2` with config-driven phase selection.

#### File: `crates/roko-runtime/src/pipeline_state.rs`

```rust
//! PipelineStateV2 — config-driven workflow state machine.
//!
//! This is a PURE state machine with no side effects. It takes events and
//! returns actions. The EffectDriver (P2C) executes the actions.
//!
//! Config determines which phases are active:
//! - Express:  implement → gate → commit
//! - Standard: implement → gate → review → commit
//! - Full:     strategy → implement → gate → review → commit

use roko_core::runtime_event::WorkflowOutcome;

/// Configuration for the pipeline. Determines which phases are active.
#[derive(Debug, Clone)]
pub struct WorkflowConfig {
    /// Include a strategist phase before implementation
    pub has_strategy: bool,
    /// Include a review phase after gates pass
    pub has_review: bool,
    /// Maximum implement → gate → review iterations
    pub max_iterations: u32,
    /// Maximum autofix attempts per gate failure
    pub max_autofix_attempts: u32,
}

impl WorkflowConfig {
    /// Express: implement → gate → commit
    pub fn express() -> Self {
        Self {
            has_strategy: false,
            has_review: false,
            max_iterations: 1,
            max_autofix_attempts: 1,
        }
    }

    /// Standard: implement → gate → review → commit
    pub fn standard() -> Self {
        Self {
            has_strategy: false,
            has_review: true,
            max_iterations: 2,
            max_autofix_attempts: 2,
        }
    }

    /// Full: strategy → implement → gate → review → commit
    pub fn full() -> Self {
        Self {
            has_strategy: true,
            has_review: true,
            max_iterations: 3,
            max_autofix_attempts: 2,
        }
    }
}

/// Current phase of the pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Pending,
    Strategizing,
    Implementing,
    Gating,
    AutoFixing,
    Reviewing,
    Committing,
    Complete,
    Halted { reason: String },
    Cancelled,
}

impl Phase {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Phase::Complete | Phase::Halted { .. } | Phase::Cancelled)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Phase::Pending => "pending",
            Phase::Strategizing => "strategizing",
            Phase::Implementing => "implementing",
            Phase::Gating => "gating",
            Phase::AutoFixing => "auto_fixing",
            Phase::Reviewing => "reviewing",
            Phase::Committing => "committing",
            Phase::Complete => "complete",
            Phase::Halted { .. } => "halted",
            Phase::Cancelled => "cancelled",
        }
    }
}

/// Events fed into the state machine.
#[derive(Debug, Clone)]
pub enum PipelineInput {
    /// Start the pipeline
    Start,
    /// Strategist completed with a brief
    StrategyComplete { brief: String },
    /// Strategy phase was skipped
    StrategySkipped,
    /// Agent completed with output
    AgentCompleted { output: String, files_changed: u32 },
    /// Agent failed
    AgentFailed { error: String },
    /// All gates passed
    GatesPassed,
    /// A gate failed
    GateFailed { gate: String, output: String },
    /// Review approved
    ReviewApproved { summary: String },
    /// Review requests revisions
    ReviewRevise { findings: Vec<String> },
    /// Commit done
    CommitDone { hash: String },
    /// User cancelled
    UserCancel,
    /// Timeout or budget exceeded
    ResourceExhausted { reason: String },
}

/// Actions the state machine asks the effect driver to execute.
#[derive(Debug, Clone)]
pub enum PipelineOutput {
    /// Spawn a strategist agent
    SpawnStrategist { prompt: String },
    /// Spawn an implementer agent
    SpawnImplementer { prompt: String, context: Option<String> },
    /// Spawn an autofix agent
    SpawnAutoFixer { error_output: String },
    /// Run verification gates
    RunGates,
    /// Spawn a reviewer agent
    SpawnReviewer { diff_context: Option<String> },
    /// Create a commit
    Commit,
    /// Pipeline is done
    Done { outcome: WorkflowOutcome },
    /// Pipeline is halted
    Halt { reason: String },
}

/// Pure state machine for workflow pipelines.
#[derive(Debug, Clone)]
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

impl PipelineStateV2 {
    pub fn new(config: WorkflowConfig, prompt: String) -> Self {
        Self {
            phase: Phase::Pending,
            config,
            iteration: 0,
            autofix_attempts: 0,
            original_prompt: prompt,
            strategist_brief: None,
            review_findings: Vec::new(),
            last_gate_failure: None,
            files_changed: 0,
            commit_hash: None,
        }
    }

    /// Feed an event into the state machine, get an action back.
    /// This is the ONLY way to drive the state machine. No side-effects here.
    pub fn step(&mut self, input: PipelineInput) -> PipelineOutput {
        match (&self.phase, input) {
            // ── Start ──
            (Phase::Pending, PipelineInput::Start) => {
                if self.config.has_strategy {
                    self.phase = Phase::Strategizing;
                    PipelineOutput::SpawnStrategist {
                        prompt: self.original_prompt.clone(),
                    }
                } else {
                    self.phase = Phase::Implementing;
                    self.iteration = 1;
                    PipelineOutput::SpawnImplementer {
                        prompt: self.original_prompt.clone(),
                        context: None,
                    }
                }
            }

            // ── Strategy ──
            (Phase::Strategizing, PipelineInput::StrategyComplete { brief }) => {
                self.strategist_brief = Some(brief.clone());
                self.phase = Phase::Implementing;
                self.iteration = 1;
                PipelineOutput::SpawnImplementer {
                    prompt: self.original_prompt.clone(),
                    context: Some(brief),
                }
            }
            (Phase::Strategizing, PipelineInput::StrategySkipped) => {
                self.phase = Phase::Implementing;
                self.iteration = 1;
                PipelineOutput::SpawnImplementer {
                    prompt: self.original_prompt.clone(),
                    context: None,
                }
            }

            // ── Implementation ──
            (Phase::Implementing, PipelineInput::AgentCompleted { output: _, files_changed }) => {
                self.files_changed = files_changed;
                self.phase = Phase::Gating;
                self.autofix_attempts = 0;
                PipelineOutput::RunGates
            }
            (Phase::Implementing, PipelineInput::AgentFailed { error }) => {
                self.phase = Phase::Halted { reason: error.clone() };
                PipelineOutput::Halt { reason: error }
            }

            // ── Gating ──
            (Phase::Gating, PipelineInput::GatesPassed) => {
                if self.config.has_review {
                    self.phase = Phase::Reviewing;
                    PipelineOutput::SpawnReviewer { diff_context: None }
                } else {
                    self.phase = Phase::Committing;
                    PipelineOutput::Commit
                }
            }
            (Phase::Gating, PipelineInput::GateFailed { gate, output }) => {
                self.last_gate_failure = Some(output.clone());
                if self.autofix_attempts < self.config.max_autofix_attempts {
                    self.autofix_attempts += 1;
                    self.phase = Phase::AutoFixing;
                    PipelineOutput::SpawnAutoFixer { error_output: output }
                } else if self.iteration < self.config.max_iterations {
                    // Retry full implementation with gate feedback
                    self.iteration += 1;
                    self.autofix_attempts = 0;
                    self.phase = Phase::Implementing;
                    PipelineOutput::SpawnImplementer {
                        prompt: self.original_prompt.clone(),
                        context: Some(format!(
                            "Previous attempt failed gate '{}'. Error:\n{}",
                            gate, output
                        )),
                    }
                } else {
                    let reason = format!("Gate '{}' failed after {} iterations", gate, self.iteration);
                    self.phase = Phase::Halted { reason: reason.clone() };
                    PipelineOutput::Halt { reason }
                }
            }

            // ── AutoFix ──
            (Phase::AutoFixing, PipelineInput::AgentCompleted { .. }) => {
                self.phase = Phase::Gating;
                PipelineOutput::RunGates
            }
            (Phase::AutoFixing, PipelineInput::AgentFailed { error }) => {
                // Autofix failed — try full re-implementation if iterations remain
                if self.iteration < self.config.max_iterations {
                    self.iteration += 1;
                    self.autofix_attempts = 0;
                    self.phase = Phase::Implementing;
                    PipelineOutput::SpawnImplementer {
                        prompt: self.original_prompt.clone(),
                        context: self.last_gate_failure.clone(),
                    }
                } else {
                    let reason = format!("Autofix failed: {}", error);
                    self.phase = Phase::Halted { reason: reason.clone() };
                    PipelineOutput::Halt { reason }
                }
            }

            // ── Review ──
            (Phase::Reviewing, PipelineInput::ReviewApproved { .. }) => {
                self.phase = Phase::Committing;
                PipelineOutput::Commit
            }
            (Phase::Reviewing, PipelineInput::ReviewRevise { findings }) => {
                self.review_findings.extend(findings);
                if self.iteration < self.config.max_iterations {
                    self.iteration += 1;
                    self.autofix_attempts = 0;
                    self.phase = Phase::Implementing;
                    let feedback = self.review_findings.join("\n- ");
                    PipelineOutput::SpawnImplementer {
                        prompt: self.original_prompt.clone(),
                        context: Some(format!("Review findings:\n- {}", feedback)),
                    }
                } else {
                    // Max iterations reached, commit anyway
                    self.phase = Phase::Committing;
                    PipelineOutput::Commit
                }
            }

            // ── Commit ──
            (Phase::Committing, PipelineInput::CommitDone { hash }) => {
                self.commit_hash = Some(hash.clone());
                self.phase = Phase::Complete;
                PipelineOutput::Done {
                    outcome: WorkflowOutcome::Success {
                        commit_hash: Some(hash),
                    },
                }
            }

            // ── Universal transitions ──
            (_, PipelineInput::UserCancel) => {
                self.phase = Phase::Cancelled;
                PipelineOutput::Done {
                    outcome: WorkflowOutcome::Cancelled,
                }
            }
            (_, PipelineInput::ResourceExhausted { reason }) => {
                self.phase = Phase::Halted { reason: reason.clone() };
                PipelineOutput::Halt { reason }
            }

            // ── Invalid transition ──
            (phase, input) => {
                let reason = format!(
                    "Invalid transition: {:?} in phase {:?}",
                    std::mem::discriminant(&input),
                    phase
                );
                self.phase = Phase::Halted { reason: reason.clone() };
                PipelineOutput::Halt { reason }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn express_happy_path() {
        let mut sm = PipelineStateV2::new(
            WorkflowConfig::express(),
            "fix bug".into(),
        );

        let out = sm.step(PipelineInput::Start);
        assert!(matches!(out, PipelineOutput::SpawnImplementer { .. }));
        assert_eq!(sm.phase, Phase::Implementing);

        let out = sm.step(PipelineInput::AgentCompleted { output: "done".into(), files_changed: 2 });
        assert!(matches!(out, PipelineOutput::RunGates));

        let out = sm.step(PipelineInput::GatesPassed);
        assert!(matches!(out, PipelineOutput::Commit));

        let out = sm.step(PipelineInput::CommitDone { hash: "abc".into() });
        assert!(matches!(out, PipelineOutput::Done { .. }));
        assert_eq!(sm.phase, Phase::Complete);
    }

    #[test]
    fn standard_with_review() {
        let mut sm = PipelineStateV2::new(
            WorkflowConfig::standard(),
            "add feature".into(),
        );

        sm.step(PipelineInput::Start);
        sm.step(PipelineInput::AgentCompleted { output: "done".into(), files_changed: 3 });
        sm.step(PipelineInput::GatesPassed);

        assert_eq!(sm.phase, Phase::Reviewing);

        let out = sm.step(PipelineInput::ReviewApproved { summary: "lgtm".into() });
        assert!(matches!(out, PipelineOutput::Commit));
    }

    #[test]
    fn full_with_strategy() {
        let mut sm = PipelineStateV2::new(
            WorkflowConfig::full(),
            "complex task".into(),
        );

        let out = sm.step(PipelineInput::Start);
        assert!(matches!(out, PipelineOutput::SpawnStrategist { .. }));
        assert_eq!(sm.phase, Phase::Strategizing);

        let out = sm.step(PipelineInput::StrategyComplete { brief: "plan".into() });
        assert!(matches!(out, PipelineOutput::SpawnImplementer { context: Some(_), .. }));
    }

    #[test]
    fn gate_failure_triggers_autofix() {
        let mut sm = PipelineStateV2::new(
            WorkflowConfig::standard(),
            "fix".into(),
        );

        sm.step(PipelineInput::Start);
        sm.step(PipelineInput::AgentCompleted { output: "done".into(), files_changed: 1 });

        let out = sm.step(PipelineInput::GateFailed {
            gate: "compile".into(),
            output: "error[E0308]".into(),
        });
        assert!(matches!(out, PipelineOutput::SpawnAutoFixer { .. }));
        assert_eq!(sm.phase, Phase::AutoFixing);
    }

    #[test]
    fn cancel_from_any_phase() {
        let mut sm = PipelineStateV2::new(
            WorkflowConfig::express(),
            "task".into(),
        );
        sm.step(PipelineInput::Start);

        let out = sm.step(PipelineInput::UserCancel);
        assert!(matches!(out, PipelineOutput::Done { outcome: WorkflowOutcome::Cancelled }));
        assert_eq!(sm.phase, Phase::Cancelled);
    }

    #[test]
    fn review_revise_triggers_reimplementation() {
        let mut sm = PipelineStateV2::new(
            WorkflowConfig::standard(),
            "feature".into(),
        );

        sm.step(PipelineInput::Start);
        sm.step(PipelineInput::AgentCompleted { output: "v1".into(), files_changed: 2 });
        sm.step(PipelineInput::GatesPassed);

        let out = sm.step(PipelineInput::ReviewRevise {
            findings: vec!["needs error handling".into()],
        });
        assert!(matches!(out, PipelineOutput::SpawnImplementer { .. }));
        assert_eq!(sm.iteration, 2);
    }
}
```

#### Modification: `crates/roko-runtime/src/lib.rs`

Add:
```rust
pub mod pipeline_state;
pub use pipeline_state::{PipelineStateV2, WorkflowConfig, Phase, PipelineInput, PipelineOutput};
```

### Done Criteria
```bash
grep -q 'pub struct PipelineStateV2' crates/roko-runtime/src/pipeline_state.rs
grep -q 'pub fn step' crates/roko-runtime/src/pipeline_state.rs
grep -q 'pub mod pipeline_state' crates/roko-runtime/src/lib.rs
cargo check -p roko-runtime
cargo test -p roko-runtime --lib -- pipeline_state
```
