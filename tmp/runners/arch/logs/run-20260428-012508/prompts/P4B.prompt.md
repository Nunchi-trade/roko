# Architecture Batch P4B — Wire ACP entry points

Run id: run-20260428-012508
Attempt: 2
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

## Previous attempt failure context

Anti-pattern violation detected.

Recent log tail:
    Checking toml_datetime v0.6.11
    Checking roko-primitives v0.1.0 (/Users/will/dev/nunchi/roko/roko/.roko/worktrees/arch-run-20260428-012508/crates/roko-primitives)
    Checking chrono v0.4.44
    Checking serde_urlencoded v0.7.1
    Checking tracing-serde v0.2.0
    Checking h2 v0.4.13
    Checking tracing-subscriber v0.3.23
    Checking yoke v0.8.2
    Checking toml_edit v0.22.27
    Checking zerovec v0.11.6
    Checking zerotrie v0.2.4
    Checking roko-runtime v0.1.0 (/Users/will/dev/nunchi/roko/roko/.roko/worktrees/arch-run-20260428-012508/crates/roko-runtime)
    Checking tracing-appender v0.2.5
    Checking tinystr v0.8.3
    Checking potential_utf v0.1.5
    Checking icu_collections v2.2.0
    Checking toml v0.8.23
    Checking icu_locale_core v2.2.0
    Checking hyper v1.9.0
    Checking icu_provider v2.2.0
    Checking icu_properties v2.2.0
    Checking icu_normalizer v2.2.0
    Checking hyper-util v0.1.20
    Checking idna_adapter v1.2.1
    Checking idna v1.1.0
    Checking url v2.5.8
    Checking hyper-tls v0.6.0
    Checking hyper-rustls v0.27.8
    Checking reqwest v0.12.28
    Checking roko-core v0.1.0 (/Users/will/dev/nunchi/roko/roko/.roko/worktrees/arch-run-20260428-012508/crates/roko-core)
    Checking roko-chain v0.1.0 (/Users/will/dev/nunchi/roko/roko/.roko/worktrees/arch-run-20260428-012508/crates/roko-chain)
    Checking roko-fs v0.1.0 (/Users/will/dev/nunchi/roko/roko/.roko/worktrees/arch-run-20260428-012508/crates/roko-fs)
    Checking roko-std v0.1.0 (/Users/will/dev/nunchi/roko/roko/.roko/worktrees/arch-run-20260428-012508/crates/roko-std)
    Checking roko-agent v0.1.0 (/Users/will/dev/nunchi/roko/roko/.roko/worktrees/arch-run-20260428-012508/crates/roko-agent)
    Checking roko-gate v0.1.0 (/Users/will/dev/nunchi/roko/roko/.roko/worktrees/arch-run-20260428-012508/crates/roko-gate)
    Checking roko-acp v0.1.0 (/Users/will/dev/nunchi/roko/roko/.roko/worktrees/arch-run-20260428-012508/crates/roko-acp)
    Finished `dev` profile [optimized + debuginfo] target(s) in 1m 14s
[antipattern] ! grep -rn 'Command::new.*claude' crates/roko-acp/src/bridge_events.rs
crates/roko-acp/src/bridge_events.rs:579:    let mut cmd = tokio::process::Command::new("claude");
[antipattern] ! grep -rn 'Command::new.*claude' crates/roko-acp/src/runner.rs

Use the above failure context to avoid repeating the same mistake.

---

## Batch P4B: Wire ACP Entry Points

### Write Scope
- **MODIFY**: `crates/roko-acp/src/runner.rs` (add WorkflowEngine path)
- **MODIFY**: `crates/roko-acp/src/bridge_events.rs` (add RuntimeEvent consumer registration)

### Dependencies
- P2D (WorkflowEngine)
- P3A (AcpAdapter)

### DO NOT
- Remove existing code — ADD alongside existing code
- Modify any other files
- Add Cargo.toml dependencies unless roko-runtime is not already a dependency
- Shell out to `claude` CLI for the new path

### Existing Code Context

`runner.rs` has `run_workflow_pipeline()` which drives the existing ACP-specific pipeline.
We want to add an alternative path that uses `WorkflowEngine` from `roko-runtime`.

`bridge_events.rs` has the dispatch logic that routes user prompts to execution. It
already handles `CognitiveEvent`s.

### Task

#### Modifications to `crates/roko-acp/src/runner.rs`

Add a new function that creates a WorkflowEngine with an AcpAdapter consumer:

```rust
use roko_runtime::workflow_engine::{WorkflowEngine, WorkflowRunConfig, WorkflowResult};
use roko_runtime::pipeline_state::WorkflowConfig;
use roko_runtime::effect_driver::EffectServices;
use roko_core::foundation::{ModelCaller, PromptAssembler, FeedbackSink, GateRunner};
use crate::acp_adapter::AcpAdapter;
use crate::bridge_events::CognitiveEvent;
use std::sync::Arc;

/// Execute a prompt via WorkflowEngine, bridging events to ACP protocol.
///
/// This is an alternative to run_workflow_pipeline() that uses the shared
/// WorkflowEngine architecture. Events are bridged to the ACP session via
/// AcpAdapter (EventConsumer → CognitiveEvent → session updates).
pub async fn run_with_workflow_engine(
    session_id: &str,
    prompt: &str,
    workdir: &std::path::Path,
    template: &str,
    event_sender: tokio::sync::mpsc::Sender<CognitiveEvent>,
) -> anyhow::Result<WorkflowResult> {
    use roko_agent::model_call_service::ModelCallService;
    use roko_compose::prompt_assembly_service::PromptAssemblyService;
    use roko_learn::feedback_service::FeedbackService;
    use roko_gate::gate_service::GateService;

    // Build foundation services
    let services = EffectServices {
        model_caller: Arc::new(ModelCallService::new("claude-sonnet-4-20250514".to_string())),
        prompt_assembler: Arc::new(PromptAssemblyService::new()),
        feedback_sink: Arc::new(FeedbackService::from_roko_dir(&workdir.join(".roko"))),
        gate_runner: Arc::new(GateService::new()),
    };

    // Create workflow config
    let workflow = match template {
        "express" => WorkflowConfig::express(),
        "full" => WorkflowConfig::full(),
        _ => WorkflowConfig::standard(),
    };

    let config = WorkflowRunConfig {
        prompt: prompt.to_string(),
        workdir: workdir.to_path_buf(),
        workflow,
        enabled_gates: vec!["compile".into(), "test".into()],
        commit_prefix: Some("feat".to_string()),
    };

    // Create engine with ACP adapter
    let mut engine = WorkflowEngine::new(services);

    // Generate a run_id for the adapter to filter on
    // The engine generates its own run_id internally, but we need one
    // for the adapter. Use a deterministic one based on session.
    let run_id = format!("acp_{}", session_id);
    let adapter = AcpAdapter::new(
        session_id.to_string(),
        run_id,
        event_sender,
    );
    engine.add_consumer(Arc::new(adapter));

    // Run the workflow
    engine.run(config).await
}
```

#### Modifications to `crates/roko-acp/src/bridge_events.rs`

Add a comment and an import showing where the new path connects. Do NOT modify the existing
dispatch logic — just ensure the new function is importable and add a comment:

```rust
// Near the top of the file, add:
// TODO(arch): Wire run_with_workflow_engine as alternative to run_workflow_pipeline
// When workflow config selects v2 engine, call:
//   runner::run_with_workflow_engine(session_id, prompt, workdir, template, event_sender).await
```

Read the actual `bridge_events.rs` to find the right place for this comment. It should be
near the dispatch point where prompts are sent to execution.

### Done Criteria
```bash
grep -q 'run_with_workflow_engine' crates/roko-acp/src/runner.rs
grep -q 'WorkflowEngine\|workflow_engine' crates/roko-acp/src/runner.rs
grep -q 'AcpAdapter\|acp_adapter' crates/roko-acp/src/runner.rs
! grep -rn 'Command::new.*claude' crates/roko-acp/src/runner.rs
cargo check -p roko-acp
```
