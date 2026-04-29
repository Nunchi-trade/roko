# Roko Integration Guide

A comprehensive end-to-end guide for integrating Roko and for understanding how
roko develops itself. Covers architecture, the self-hosting workflow, service
wiring, gate configuration, learning, and deployment.

---

## Table of Contents

1. [Architecture Overview](#1-architecture-overview)
2. [Self-Hosting Workflow (End-to-End)](#2-self-hosting-workflow-end-to-end)
3. [WorkflowEngine Integration](#3-workflowengine-integration)
4. [Model Call Pipeline](#4-model-call-pipeline)
5. [Prompt Assembly Pipeline](#5-prompt-assembly-pipeline)
6. [Gate Pipeline](#6-gate-pipeline)
7. [Learning and Feedback](#7-learning-and-feedback)
8. [Demo Dashboard](#8-demo-dashboard)
9. [Configuration](#9-configuration)
10. [Deployment](#10-deployment)

---

## 1. Architecture Overview

### The v2 Layer Stack

Roko is structured as a strict service hierarchy. Every entry point — CLI,
HTTP server, and ACP — shares the same runtime core.

```
Entry points
  ┌──────────┐  ┌──────────┐  ┌──────────┐
  │  CLI     │  │  Serve   │  │  ACP     │
  │  roko-cli│  │ :6677    │  │  adapter │
  └────┬─────┘  └────┬─────┘  └────┬─────┘
       │              │              │
       └──────────────▼──────────────┘
              WorkflowEngine
         (roko-runtime/workflow_engine.rs)
                    │
         ┌──────────▼──────────┐
         │  PipelineStateV2    │  ← pure state machine, no I/O
         │  EffectDriver       │  ← executes side-effects
         └──────────┬──────────┘
                    │  calls
       ┌────────────┼────────────────────┐
       ▼            ▼                    ▼
 ModelCaller  PromptAssembler      GateRunner   FeedbackSink
 (roko-agent) (roko-compose)      (roko-gate)   (roko-learn)
```

Foundation traits live in `roko-core/src/foundation.rs`:

| Trait | Implementor | Purpose |
|---|---|---|
| `ModelCaller` | `ModelCallService` (roko-agent) | LLM dispatch with routing |
| `PromptAssembler` | `PromptAssemblyService` (roko-compose) | 9-layer system prompt |
| `FeedbackSink` | `FeedbackService` (roko-learn) | Episode and feedback recording |
| `GateRunner` | `GateService` (roko-gate) | 7-rung gate pipeline |
| `EventConsumer` | `JsonlLogger`, `SseAdapter`, ACP | Runtime event observation |
| `AffectPolicy` | `DaimonPolicy` (roko-daimon) | Behavioral dispatch modulation |
| `EffectExecutor` | `EffectDriver` (roko-runtime) | Translates state-machine actions to I/O |

### State Machine + Effect Driver Pattern

The core design keeps decisions separate from side-effects:

```
PipelineStateV2::step(PipelineInput) -> PipelineOutput
                                              │
                              EffectDriver executes the output
                                              │
                              returns PipelineInput back to loop
```

`PipelineStateV2` is a pure Rust struct with no `async` methods and no I/O. It
can be serialized to JSON for checkpoint/resume without any special handling.
`EffectDriver` owns the `Arc<dyn ModelCaller>`, `Arc<dyn GateRunner>`, etc. and
translates `PipelineOutput` variants into real work.

### Workflow Templates

Three templates are available, each extending the previous:

| Template | Phases | Use case |
|---|---|---|
| `express` | Implement → Gate → Commit | Fast single-pass tasks |
| `standard` | Implement → Gate → Review → Commit | Default; adds LLM review |
| `full` | Strategy → Implement → Gate → Review → Commit | Complex tasks with upfront planning |

### Event-Driven Observability

Every significant action emits a `RuntimeEvent` to a process-global broadcast
bus (`EventBus<RuntimeEvent>`). Consumers register at startup and receive a
non-blocking fire-and-forget notification for each event.

```
WorkflowEngine / EffectDriver
     │
     │  emit_runtime_event(RuntimeEvent::AgentSpawned { ... })
     ▼
EventBus<RuntimeEvent>
     ├── JsonlLogger   → .roko/runtime-events.jsonl
     ├── SseAdapter    → HTTP GET /v1/events stream
     └── AcpAdapter    → per-run ACP protocol messages
```

The `RuntimeEvent` enum (`roko-core/src/runtime_event.rs`) covers:

- Lifecycle: `WorkflowStarted`, `PhaseTransition`, `WorkflowCompleted`
- Agent: `AgentSpawned`, `AgentOutput`, `AgentCompleted`, `AgentFailed`
- Gates: `GateStarted`, `GatePassed`, `GateFailed`
- Feedback: `FeedbackRecorded`
- Persistence: `StateCheckpointed`

Every event carries a `run_id` so consumers can correlate across concurrent runs.

---

## 2. Self-Hosting Workflow (End-to-End)

This section walks through the complete loop that roko uses to develop itself.
Every command exists in the CLI today.

### Prerequisites

```bash
# Rust 1.91+ is required (alloy deps)
rustup update stable
rustup default stable

# Build the binary
cd /path/to/roko
cargo build -p roko-cli --release

# Or use cargo run for development
alias roko='cargo run -p roko-cli --'
```

### Step 1: Initialize the workspace

```bash
roko init
```

Creates `.roko/` with the standard directory layout:

```
.roko/
  prd/
    ideas/
    drafts/
    published/
  state/       ← executor checkpoints
  learn/
    efficiency.jsonl
    episodes.jsonl
    cascade-router.json
    gate-thresholds.json
    experiments.json
  runtime-events.jsonl
  signals.jsonl
  GAPS.md
```

Also creates a starter `roko.toml` in the workspace root. Edit it to configure
providers before proceeding (see [Section 9: Configuration](#9-configuration)).

### Step 2: Capture a work item

```bash
roko prd idea "Wire knowledge store into CascadeRouter for model selection"
```

Creates a dated idea file in `.roko/prd/ideas/`. Ideas are lightweight — just
a title and timestamp. Use `roko prd list` to see all ideas.

### Step 3: Draft a PRD

```bash
roko prd draft new "knowledge-informed-routing"
```

Launches a Claude agent that reads the idea, reads the current codebase via
the code-intelligence MCP, and writes a structured PRD into
`.roko/prd/drafts/knowledge-informed-routing.md`.

The PRD includes a mandatory **Repository Grounding** section that lists the
specific files and types the implementation will touch. This grounding is
injected as context when generating the plan in step 5.

### Step 4: Enrich with research

```bash
roko research enhance-prd knowledge-informed-routing
```

Launches a Perplexity research agent that queries for relevant prior art,
papers, and API documentation, then appends a **Research** section to the draft
PRD. This step is optional but significantly improves plan quality for novel
subsystems.

### Step 5: Review and promote the PRD

```bash
# Inspect the draft
roko prd draft list

# Edit if needed
roko prd draft edit knowledge-informed-routing

# Promote to published
roko prd draft promote knowledge-informed-routing
```

Promoting moves the file to `.roko/prd/published/`. If `prd.auto_plan = true`
in `roko.toml`, the plan generation step below triggers automatically via the
`prd_publish_subscriber` background task in `roko-serve`.

### Step 6: Generate an implementation plan

```bash
roko prd plan knowledge-informed-routing
```

A Claude agent reads the published PRD (including the Repository Grounding
section), reads the relevant source files, and produces a
`plans/knowledge-informed-routing/tasks.toml` with a DAG of implementation tasks.

Each task in the TOML has:
- `id` — stable identifier for resumption
- `title` — human-readable description
- `depends_on` — list of task ids that must complete first
- `prompt` — the implementation instruction given to the agent

Validate the plan without executing:

```bash
roko plan validate plans/knowledge-informed-routing/
```

### Step 7: Execute the plan

```bash
roko plan run plans/knowledge-informed-routing/
```

This starts the main orchestration loop (`orchestrate.rs`). For each task:

1. Build the 9-layer system prompt via `PromptAssemblyService`
2. Route to a model via `CascadeRouter`
3. Dispatch the agent via `ModelCallService`
4. Run the 7-rung gate pipeline via `GateService`
5. On gate failure: attempt autofix or replan
6. On success: record feedback, update learning state, checkpoint
7. Commit changes and advance to the next task

Progress is visible in real time on the TUI.

### Step 8: Resume if interrupted

```bash
roko plan run plans/knowledge-informed-routing/ \
  --resume .roko/state/executor.json
```

The `PipelineStateV2` checkpoint is written atomically after each phase
transition. Resumption restores the exact pipeline state — phase, iteration
count, accumulated review findings — so work is never duplicated.

### Step 9: Watch progress

```bash
roko dashboard
```

Opens the interactive ratatui TUI. F1-F7 cycle through tabs:

| Tab | Content |
|---|---|
| F1 | Active tasks and phase indicators |
| F2 | Agent output stream |
| F3 | Gate results per task |
| F4 | Episode history |
| F5 | Learning state (router, experiments) |
| F6 | Cost and token usage |
| F7 | System health |

The TUI uses a file watcher (`notify::RecommendedWatcher`) to pick up changes
to `.roko/` without polling. Updates appear in under 250 ms.

### Step 10: Inspect learning state

```bash
roko learn all          # full dump
roko learn router       # cascade router bandit state
roko learn experiments  # A/B prompt experiments
roko learn efficiency   # per-turn efficiency events
roko learn episodes     # episode history
```

After a few successful runs the cascade router accumulates enough observations
to graduate from the static routing table (stage 1) into the confidence-based
stage (stage 2, 50+ observations) and eventually the LinUCB bandit stage (stage
3, 200+ observations).

---

## 3. WorkflowEngine Integration

This section shows how to wire up a `WorkflowEngine` programmatically, which is
what the CLI, HTTP server, and ACP adapter all do.

### 3.1 Instantiate the services

```rust
use std::sync::Arc;
use std::path::PathBuf;

use roko_agent::model_call_service::ModelCallService;
use roko_compose::prompt_assembly_service::PromptAssemblyService;
use roko_learn::feedback_service::FeedbackService;
use roko_gate::gate_service::GateService;
use roko_runtime::effect_driver::EffectServices;
use roko_runtime::workflow_engine::{WorkflowEngine, WorkflowRunConfig};
use roko_runtime::pipeline_state::WorkflowConfig;

// 1. ModelCallService — wraps provider dispatch, routing, cost tracking
let model_caller = Arc::new(
    ModelCallService::new("claude-sonnet-4-6".to_string())
        .with_config(roko_config.clone())           // provider routing table
        .with_feedback_sink(Arc::clone(&feedback))  // record model calls
        .with_model_router(router_fn)               // CascadeRouter closure
);

// 2. PromptAssemblyService — 9-layer system prompt builder
let prompt_assembler = Arc::new(
    PromptAssemblyService::new()
        .with_domain_context("Rust systems programming".to_string())
        .with_episodes_path(PathBuf::from(".roko/learn/episodes.jsonl"))
        .with_playbook_store(Arc::clone(&playbook_store))
        .with_knowledge_store(Arc::clone(&neuro_store))
        .with_token_budget(8192)
);

// 3. FeedbackService — records events to .roko/learn/
let feedback = Arc::new(
    FeedbackService::new(PathBuf::from(".roko/learn"))
        .with_episode_logger(episode_logger)
        .with_cascade_router(Arc::clone(&cascade_router))
);

// 4. GateService — runs the 7-rung gate pipeline
let gate_runner = Arc::new(
    GateService::new()
        .with_adaptive_thresholds(adaptive_thresholds)
);
```

### 3.2 Build EffectServices and WorkflowEngine

```rust
let services = EffectServices {
    default_model: "claude-sonnet-4-6".to_string(),
    model_caller: Arc::clone(&model_caller),
    prompt_assembler: Arc::clone(&prompt_assembler),
    feedback_sink: Arc::clone(&feedback),
    gate_runner: Arc::clone(&gate_runner),
    affect_policy: None,  // or Some(Arc::new(Mutex::new(daimon_policy)))
};

let mut engine = WorkflowEngine::new(services);
```

### 3.3 Attach event consumers

```rust
use roko_runtime::jsonl_logger::JsonlLogger;
use std::sync::Arc;

// Write runtime events to disk
let logger = Arc::new(JsonlLogger::from_roko_dir(
    &PathBuf::from(".roko")
));
engine.add_consumer(logger);

// SSE adapter (used by HTTP serve)
// engine.add_consumer(Arc::new(sse_adapter));
```

### 3.4 Run a workflow

```rust
use roko_runtime::workflow_engine::WorkflowRunConfig;
use roko_runtime::pipeline_state::WorkflowConfig;
use roko_core::foundation::ShellGateCommand;

let config = WorkflowRunConfig {
    prompt: "Add error handling to the database connection module".to_string(),
    workdir: PathBuf::from("/path/to/workspace"),
    workflow: WorkflowConfig::standard(),   // or express() / full()
    enabled_gates: vec![
        "compile".to_string(),
        "clippy".to_string(),
        "test".to_string(),
    ],
    shell_gates: vec![],   // custom shell gate commands
    commit_prefix: Some("fix:".to_string()),
};

let report = engine.run(config).await?;

println!("run_id: {}", report.run_id);
println!("success: {}", report.success);
println!("model: {}", report.model);
println!("tokens: {}", report.token_usage);
println!("cost: ${:.4}", report.cost.unwrap_or(0.0));
println!("duration: {:.1}s", report.duration_secs);
```

`WorkflowRunReport` also contains:

- `report.gates` — `Vec<GateOutcome>` with pass/fail per gate
- `report.events` — `Vec<RuntimeEventEnvelope>` emitted during the run
- `report.checkpoint_path` — path to the last checkpoint written

### 3.5 Cooperative cancellation

```rust
use roko_runtime::cancel::CancelToken;

let token = CancelToken::new();
let token_clone = token.clone();

// Cancel from another task
tokio::spawn(async move {
    tokio::time::sleep(Duration::from_secs(30)).await;
    token_clone.cancel();
});

let report = engine.run_with_cancel(config, token).await?;
// report.outcome will be WorkflowOutcome::Cancelled
```

### 3.6 Checkpoint and resume

The `PipelineStateV2` serializes to JSON after every phase transition:

```rust
// Checkpointing happens automatically inside EffectDriver::save_checkpoint.
// To manually checkpoint:
let json = pipeline_state.checkpoint()?;
std::fs::write(".roko/state/executor.json", &json)?;

// To restore:
let restored = PipelineStateV2::from_checkpoint(&json)?;
// Feed back an input appropriate for the restored phase
let output = restored.step(PipelineInput::Start);
```

The WorkflowEngine writes checkpoints automatically via `EffectDriver` and
emits `RuntimeEvent::StateCheckpointed` for each write.

---

## 4. Model Call Pipeline

### 4.1 Provider resolution

`ModelCallService` selects a provider in this priority order:

1. `request.model` if explicitly set and non-empty
2. The `model_router` closure (wraps `CascadeRouter`)
3. `default_model` from `EffectServices`

Provider dispatch goes through `create_agent_for_model` in
`crates/roko-agent/src/provider/`. Supported provider kinds:

| Kind | Config key | Notes |
|---|---|---|
| `claude_cli` | `providers.anthropic` | Shells out to `claude` CLI binary |
| `anthropic_api` | — | Direct Anthropic API |
| `openai_compat` | `providers.openai`, `providers.cerebras`, etc. | Any OpenAI-compatible endpoint |
| `cerebras_api` | `providers.cerebras` | Cerebras inference API |
| `perplexity_api` | `providers.perplexity` | Perplexity research API |
| `gemini_native` | `providers.gemini` | Google Gemini API |
| `ollama` | `providers.ollama` | Local Ollama server |

### 4.2 CascadeRouter: adaptive model selection

The `CascadeRouter` in `crates/roko-learn/src/cascade_router.rs` progresses
through three stages automatically as it accumulates per-role observations:

| Stage | Observation count | Strategy |
|---|---|---|
| 1 Static | < 50 | Hardcoded role → model table |
| 2 Confidence | 50-200 | Empirical pass-rate + confidence interval |
| 3 UCB/LinUCB | > 200 | Full contextual bandit |

```toml
# roko.toml
[routing]
mode = "auto_override"
algorithm = "linucb"
discount_factor = 0.99
fast_task_model = "claude-haiku-4-5"
standard_task_model = "claude-sonnet-4-6"
complex_task_model = "claude-opus-4-6"
context_strategy = "mcp_first"

[routing.weights]
quality = 0.5
cost = 0.3
latency = 0.2
```

The router state persists to `.roko/learn/cascade-router.json` so learning
survives restarts. Inspect it with:

```bash
roko learn router
```

### 4.3 Cost tracking and budget enforcement

`ModelCallService` enforces budgets at two levels:

- **Per-call budget**: `ModelCallRequest.budget.max_cost_usd`
- **Service-lifetime budget**: configured via `with_budget_usd`

```toml
# roko.toml
[budget]
max_plan_usd = 25.0      # maximum spend per plan run
max_turn_usd = 3.0       # maximum spend per agent turn
prompt_token_budget = 10000
```

When a budget is exceeded the service returns
`GatewayError::BudgetExceeded { detail }` which propagates as
`PipelineInput::ResourceExhausted` to halt the workflow cleanly.

### 4.4 MCP config passthrough

MCP configuration is passed directly to provider construction:

```toml
# roko.toml
[agent]
mcp_config = "/path/to/mcp-config.json"
```

When set, `ModelCallService` passes the config file path into every
`AgentOptions` struct so that all providers that support MCP (the Claude CLI
backend and the Anthropic API backend) inherit it automatically.

### 4.5 Affect-based dispatch modulation

If an `AffectPolicy` is attached to `EffectServices`, each agent dispatch goes
through behavioral modulation before the model call:

```rust
// From EffectDriver::spawn_agent:
let mut modulation = DispatchModulation::default();
if let Some(ref affect) = self.services.affect_policy {
    let policy = affect.lock().await;
    policy.modulate_dispatch(role, &mut modulation);
}
// modulation.tier_bias       → shifts model tier selection
// modulation.turn_limit_factor → scales max_tokens
// modulation.exploration_rate  → adjusts temperature + cache policy
```

The DaimonPolicy (in `roko-daimon`) implements PAD-space affect tracking and
updates on every task outcome and gate result.

### 4.6 L1 response cache

`ModelCallService` maintains an exact-match LRU cache keyed on
`(model, system_prompt_hash, user_content_hash)`. This avoids redundant API
calls for identical prompts.

Cache behavior is controlled per-request via `ModelCallRequest.cache_policy`:

| Policy | Behavior |
|---|---|
| `Default` | Normal cache lookup |
| `Bypass` | Skip lookup, still store result |
| `ForceRefresh` | Skip lookup AND discard prior cached result |

High exploration rates (from `AffectPolicy`) automatically set `Bypass` to
avoid stale responses during exploratory phases.

---

## 5. Prompt Assembly Pipeline

### 5.1 9-layer SystemPromptBuilder

`PromptAssemblyService` builds system prompts in nine ordered layers:

| Layer | Content | Source |
|---|---|---|
| 0 | Role identity | `role_prompts::role_identity_for(role)` |
| 1 | Task specification | `PromptSpec.task` |
| 2 | Project conventions | Detected from `workdir` |
| 3 | Domain context | Static text, knowledge store entries |
| 4 | Episode history | Recent successful episodes from `.roko/learn/episodes.jsonl` |
| 5 | Tool instructions | Tool usage guidance |
| 6 | Playbook techniques | Relevant playbook entries from `PlaybookStore` |
| 7 | Gate feedback | Prior gate failure output |
| 8 | Anti-patterns | Extracted failure anti-patterns |

Each layer has an effectiveness score. Sections with consistently low scores
are trimmed first when the prompt approaches the token budget.

### 5.2 Context sources

**Neuro/knowledge store** (layer 3):
```rust
// Queries the neuro store for entries related to the current task
let entries = neuro_store.query(task_description, 5)?;
// High-scoring entries are injected as domain context
```

**Episode history** (layer 4):
```rust
// Reads recent successful episodes for similar tasks
// Episodes include: role, model, duration, token cost, gate verdicts
let episodes = EpisodeLogger::load_recent(&path, limit)?;
```

**Playbook store** (layer 6):
```rust
// Queries learned techniques relevant to the task
let context = QueryContext { role, task_snippet, workdir };
let techniques = playbook_store.query(&context, 3)?;
```

### 5.3 Convention detection

When `workdir` is set, the service auto-detects:

- Build system (Cargo, npm, Go modules, etc.)
- Language stack
- Test framework
- Linting configuration
- Common patterns from source file sampling

Detected conventions are injected in layer 2 and take precedence over
`default_conventions`.

### 5.4 Section effectiveness scoring

After each workflow run, `FeedbackService` correlates prompt section IDs with
run outcomes:

```rust
// FeedbackEvent::ModelCall carries prompt_section_ids
// FeedbackEvent::WorkflowComplete carries success/failure
// FeedbackService joins them and updates SectionEffectivenessRegistry
```

Sections with `score < threshold` are skipped in future assemblies, reducing
prompt bloat. Inspect the effectiveness state:

```bash
# Scores persist at .roko/learn/section-effects.json
cat .roko/learn/section-effects.json
```

### 5.5 Gate feedback injection

When a task iteration fails gates, the failure output is injected into layer 7
for the next iteration:

```rust
// In WorkflowEngine.run loop, after GateFailed:
let spec = PromptSpec {
    gate_feedback: vec![gate_failure_output],
    // ...
};
let system_prompt = prompt_assembler.assemble(spec).await?;
```

The implementer agent sees the exact compiler or test failure and can fix it
without a full context re-read.

---

## 6. Gate Pipeline

### 6.1 7-rung gate pipeline

Gates run in rung order, stopping at the first failure:

| Rung | Name | Gate | What it checks |
|---|---|---|---|
| 0 | compile | `CompileGate` | `cargo build --workspace` |
| 1 | clippy | `ClippyGate` | `cargo clippy --workspace --no-deps -- -D warnings` |
| 2 | test | `TestGate` | `cargo test --workspace` |
| 3 | diff | `diff:git` | `git diff --stat` (sanity check) |
| 4 | fmt | `FormatCheckGate` | `cargo fmt --check` |
| 5 | shell | `ShellGate` | Custom shell command |
| 6 | judge | `StubJudgeGate` | LLM judge (stub, not wired to live LLM) |

Rung 0 (compile) is never skipped regardless of adaptive threshold state.
All other rungs can be skipped when the adaptive threshold system determines
the gate has been passing consistently.

### 6.2 Enabling specific gates

```toml
# roko.toml
[gates]
clippy_enabled = true
skip_tests = false
max_iterations = 3
```

In `WorkflowRunConfig`:
```rust
WorkflowRunConfig {
    enabled_gates: vec![
        "compile".to_string(),
        "clippy".to_string(),
        "test".to_string(),
        "fmt".to_string(),
        "shell".to_string(),
    ],
    shell_gates: vec![
        ShellGateCommand {
            program: "cargo".to_string(),
            args: vec!["audit".to_string()],
            timeout_ms: 60_000,
        },
    ],
    // ...
}
```

### 6.3 Adaptive thresholds

`AdaptiveThresholds` tracks pass-streak counts per rung using an EMA. When a
rung has passed consistently for long enough, it is skipped until a failure
is observed.

State persists to `.roko/learn/gate-thresholds.json`. Tune with:

```bash
roko learn tune gates
```

### 6.4 Gate failure replanning

When gate failures exhaust the autofix budget and the iteration limit, the
`learning_config.replan_on_gate_failure` flag triggers a plan revision:

```toml
# roko.toml
[learning]
replan_on_gate_failure = true
replan_max_per_plan = 2
replan_gate_attempts = 3
```

A replan emits `RokoEvent::PlanRevision` on the global event bus, which is
consumed by the `prd_publish_subscriber` to trigger a new planning agent pass
with the gate failure context injected.

### 6.5 Shell gate configuration

Custom gates can wrap any shell command:

```toml
# roko.toml — example custom audit gate
[[gates.shell]]
name = "audit"
program = "cargo"
args = ["audit", "--deny", "warnings"]
timeout_ms = 120000
```

Or in `WorkflowRunConfig.shell_gates` for per-run overrides.

---

## 7. Learning and Feedback

The learning subsystem records everything and uses past outcomes to improve
future decisions. All learning state lives in `.roko/learn/`.

### 7.1 Episode logging

Every agent turn produces an `Episode` record:

```jsonl
// .roko/learn/episodes.jsonl
{
  "id": "ep-20260429-001",
  "role": "implementer",
  "model": "claude-sonnet-4-6",
  "prompt_hash": "a1b2c3",
  "gate_verdict": "pass",
  "tokens": 4200,
  "cost_usd": 0.0126,
  "duration_ms": 8400,
  "hdc_fingerprint": "deadbeef..."
}
```

HDC fingerprints identify semantically similar episodes for replay and
retrieval. The `FeedbackService` computes them from the combined task + output
content.

Inspect episodes:

```bash
roko learn episodes --limit 20
```

### 7.2 Playbook extraction

`PlaybookStore` accumulates techniques from successful runs. Entries include
the role, the effective approach used, and gate outcomes:

```bash
# View extracted playbooks
roko learn all
```

Playbooks are queried at dispatch time and injected into layer 6 of the system
prompt. A technique that worked for a similar task (same role, same codebase
conventions) is surfaced automatically.

### 7.3 Anti-pattern extraction

Failed gate outputs are fingerprinted and stored as anti-patterns. On future
passes the `PromptAssemblyService` injects the relevant anti-patterns into
layer 8, telling the agent what not to do.

### 7.4 CascadeRouter bandit observations

After each `FeedbackEvent::WorkflowComplete` the `FeedbackService` calls
`CascadeRouter::observe_outcome(role, model, success, cost, latency)`. The
router updates its LinUCB arm weights so better-performing models get higher
selection probability.

```bash
# Inspect bandit state per role
roko learn router
```

### 7.5 Provider model pass-rate telemetry

`FeedbackService` maintains `ProviderModelOutcome` records:

```bash
# Per provider, per model: pass rate and average cost
roko learn efficiency
```

This data feeds into the `CascadeRouter` confidence stage and into
`ProviderHealthRegistry` for circuit-breaking unhealthy providers.

### 7.6 Knowledge admission (A-MAC)

Knowledge entries earn scores based on how often they correlate with successful
outcomes. A new entry starts neutral; each time it contributes to a passing run
its score increments; each time to a failing run it decrements. Entries below a
minimum score threshold are excluded from future prompts.

Score state persists to `.roko/learn/knowledge-scores.json`.

### 7.7 Prompt experiment tracking

A/B prompt experiments live in `.roko/learn/experiments.json`:

```bash
roko config experiments         # list active experiments
roko learn tune routing         # tune routing weights
```

The `ModelExperimentStore` assigns variants to runs and records outcomes,
enabling statistically grounded comparisons between prompt strategies.

---

## 8. Demo Dashboard

### 8.1 Starting the HTTP control plane

```bash
roko serve
# Listening on http://127.0.0.1:6677
```

Configuration:

```toml
# roko.toml
[server]
bind = "127.0.0.1"
port = 6677
cors_origins = []

[serve.auth]
enabled = false
api_key = ""

[serve]
auto_orchestrate = true
```

The server exposes approximately 85 REST routes plus SSE streaming. Key
endpoint groups:

| Prefix | What |
|---|---|
| `GET /v1/status` | System health, agent fleet status |
| `GET /v1/plans` | Plan list and status |
| `POST /v1/plans` | Create and run a plan |
| `GET /v1/agents` | Agent list |
| `POST /v1/agents/:id/message` | Send message to a running agent |
| `GET /v1/events` | SSE stream of `RuntimeEvent` |
| `GET /v1/learn/*` | Learning state inspection |
| `GET /v1/prd/*` | PRD management |
| `GET /v1/gates/*` | Gate pipeline status |
| `GET /v1/bench` | Benchmark endpoint |
| `GET /v1/dream` | Dream consolidation status |

### 8.2 Demo seeding

```bash
roko init --demo
```

Seeds the `.roko/` directory with synthetic data for dashboards:
- 10 example plans at various completion stages
- 30 synthetic episodes
- Pre-populated learning state
- Example gate verdicts

This mode uses `agent.command = "echo"` (configured in
`demo/demo-resources/roko.toml`) so no real LLM calls are made.

### 8.3 SSE event streaming

Connect to the event stream for real-time updates:

```bash
curl -N http://127.0.0.1:6677/v1/events
```

Each line is a JSON `RuntimeEventEnvelope`:

```json
{
  "run_id": "run-abc123",
  "seq": 42,
  "ts": "2026-04-29T03:00:00Z",
  "schema_version": 1,
  "source": "workflow_engine",
  "payload": {
    "kind": "gate_passed",
    "data": {
      "run_id": "run-abc123",
      "gate_name": "compile",
      "duration_ms": 1840
    }
  }
}
```

The browser-side `useSSE` hook in the demo app (`demo/demo-app/src/hooks/useSSE.ts`)
subscribes to this stream and dispatches events into React state.

### 8.4 Per-agent sidecar

Each running agent exposes its own HTTP sidecar on a dynamically assigned port:

```bash
roko agent serve --name my-agent --port 7001
```

The sidecar (`roko-agent-server`) exposes 13 routes:

| Endpoint | What |
|---|---|
| `POST /message` | Send message, get real LLM response |
| `GET /stream` | WebSocket streaming |
| `GET /predictions` | Agent belief state |
| `GET /research` | Trigger agent research |
| `GET /tasks` | Agent task queue |

---

## 9. Configuration

### 9.1 roko.toml structure

The full schema is defined in
`crates/roko-core/src/config/schema.rs` (struct `RokoConfig`).
All sections have sane defaults; only providers need explicit configuration.

Minimum working configuration:

```toml
config_version = 1
schema_version = 2

[project]
name = "my-project"
root = "."

[agent]
default_model = "claude-sonnet-4-6"
default_backend = "anthropic"

[providers.anthropic]
kind = "claude_cli"
command = "claude"
default_model = "claude-sonnet-4-6"
models = ["claude-haiku-4-5", "claude-sonnet-4-6", "claude-opus-4-6"]
```

### 9.2 Provider configuration examples

**Anthropic Claude CLI (recommended for local development)**:
```toml
[providers.anthropic]
kind = "claude_cli"
command = "claude"
default_model = "claude-sonnet-4-6"
models = ["claude-haiku-4-5", "claude-sonnet-4-6", "claude-opus-4-6"]
```

**OpenAI or OpenAI-compatible endpoint**:
```toml
[providers.openai]
kind = "openai_compat"
api_key_env = "OPENAI_API_KEY"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1"
models = ["gpt-4.1", "gpt-4.1-mini", "o4-mini"]
```

**Cerebras (fast inference)**:
```toml
[providers.cerebras]
kind = "cerebras_api"
api_key_env = "CEREBRAS_API_KEY"
base_url = "https://api.cerebras.ai/v1"
default_model = "llama-3.3-70b"
models = ["llama-3.3-70b", "llama-3.1-8b"]
```

**Gemini**:
```toml
[providers.gemini]
kind = "openai_compat"
api_key_env = "GEMINI_API_KEY"
base_url = "https://generativelanguage.googleapis.com/v1beta/openai"
default_model = "gemini-2.5-flash"
models = ["gemini-2.5-flash", "gemini-2.5-pro"]
```

**Local Ollama**:
```toml
[providers.ollama]
kind = "openai_compat"
api_key_env = ""
base_url = "http://localhost:11434/v1"
default_model = "llama3.1"
models = ["llama3.1", "codellama"]
```

**Perplexity (research only)**:
```toml
[providers.perplexity]
kind = "perplexity_api"
api_key_env = "PERPLEXITY_API_KEY"
default_model = "sonar-pro"
models = ["sonar", "sonar-pro", "sonar-reasoning-pro"]
```

### 9.3 Model alias configuration

Model aliases decouple role names from provider-specific slugs:

```toml
[models.haiku]
provider = "anthropic"
slug = "claude-haiku-4-5"

[models.sonnet]
provider = "anthropic"
slug = "claude-sonnet-4-6"

[models.fast]
provider = "cerebras"
slug = "llama-3.3-70b"
```

The CascadeRouter refers to models by alias. Changing the underlying provider
slug for an alias requires no change to routing configuration.

### 9.4 Gate configuration

```toml
[gates]
clippy_enabled = true
skip_tests = false
max_iterations = 3

# Custom shell gate
[[gates.shell]]
name = "audit"
program = "cargo"
args = ["audit", "--deny", "warnings"]
timeout_ms = 120000
```

### 9.5 Learning configuration

```toml
[learning]
auto_playbook_refresh = true
knowledge_file_intel = true
knowledge_warnings = true
knowledge_wave_context = true
knowledge_error_patterns = true
learning_min_occurrences = 2
file_intel_max_entries = 15
warning_max_entries = 5
replan_on_gate_failure = true
replan_max_per_plan = 2
replan_gate_attempts = 3
use_lookahead_router = false
lookahead_threshold = 0.7
```

### 9.6 Budget configuration

```toml
[budget]
max_plan_usd = 25.0      # halt plan if total spend exceeds this
max_turn_usd = 3.0       # halt single turn if cost exceeds this
prompt_token_budget = 10000
```

### 9.7 Routing configuration

```toml
[routing]
mode = "auto_override"
algorithm = "linucb"        # or "greedy", "epsilon_greedy"
discount_factor = 0.99
fast_task_model = "claude-haiku-4-5"
standard_task_model = "claude-sonnet-4-6"
complex_task_model = "claude-opus-4-6"
context_strategy = "mcp_first"

[routing.weights]
quality = 0.5
cost = 0.3
latency = 0.2
```

### 9.8 Pipeline templates

```toml
[pipeline.mechanical]
strategist = false
reviewers = false
reviewer_mode = "quick"
max_iterations = 1

[pipeline.focused]
strategist = false
reviewers = false
reviewer_mode = "quick"
max_iterations = 2

[pipeline.integrative]
strategist = true
reviewers = true
reviewer_mode = "quick"
max_iterations = 2

[pipeline.architectural]
strategist = true
reviewers = true
reviewer_mode = "full"
max_iterations = 3
```

Or equivalently with `WorkflowConfig`:

```rust
WorkflowConfig::express()   // mechanical
WorkflowConfig::standard()  // focused with review
WorkflowConfig::full()      // architectural
```

Or with TOML inline:

```toml
# workflow.toml (loaded by WorkflowConfig::from_toml)
template = "full"
max_iterations = 4          # override preset

[[workflow.steps]]
name = "strategy"
role = "strategist"

[[workflow.steps]]
name = "implement"
role = "implementer"

[[workflow.steps]]
name = "review"
role = "reviewer"
```

### 9.9 Secrets management

API keys should never be committed. Use environment variable references:

```toml
[providers.openai]
api_key_env = "OPENAI_API_KEY"   # reads $OPENAI_API_KEY at runtime
```

Manage secrets through the CLI:

```bash
roko config set-secret OPENAI_API_KEY sk-...
roko config check-secrets                    # verify all keys are present
roko config secrets list                     # list configured secret names
roko config secrets rotate OPENAI_API_KEY    # rotate a key
```

---

## 10. Deployment

### 10.1 Railway deployment

```bash
roko deploy railway
```

This:
1. Reads `[serve.deploy]` from `roko.toml`
2. Creates or updates a Railway service using the worker image
3. Sets the required environment variables listed in `[serve.deploy].environment`
4. Configures the start command as `roko serve`

Configuration:

```toml
[serve.deploy]
provider = "railway"
environment = [
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "PERPLEXITY_API_KEY",
    "GITHUB_TOKEN",
]
```

### 10.2 System service (daemon)

Install roko as a launchd (macOS) or systemd (Linux) service:

```bash
roko daemon install

# Lifecycle management
roko daemon start
roko daemon stop
roko daemon status
roko daemon logs
```

The daemon runs `roko serve` on startup and restarts it on failure.

### 10.3 Docker

```bash
roko deploy docker
```

Builds and tags a Docker image from the Dockerfile in the workspace root.
The image:
- Uses a multi-stage build (builder + runtime)
- Copies only the `roko-cli` binary
- Exposes port 6677
- Defaults to `roko serve` as the entrypoint

Run directly:

```bash
docker run -p 6677:6677 \
  -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY \
  -v /path/to/workspace:/workspace \
  roko-worker:latest serve
```

### 10.4 Worker mode

For multi-node deployments, `roko worker` runs as a stateless task worker that
pulls work from a queue:

```bash
# On the control node
roko serve                          # exposes :6677

# On worker nodes
roko worker --control http://control:6677
```

```toml
# roko.toml on worker node
[deploy]
backend = "manual"
worker_image = "ghcr.io/nunchi-trade/roko-worker:latest"
```

### 10.5 Environment variables

All configuration values in `roko.toml` can be overridden with environment
variables using the pattern `ROKO_<SECTION>_<KEY>`:

```bash
ROKO_SERVER_PORT=8080 roko serve
ROKO_BUDGET_MAX_PLAN_USD=50.0 roko plan run plans/
ROKO_AGENT_DEFAULT_MODEL=claude-opus-4-6 roko plan run plans/
```

API key environment variables are referenced by name in the config:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
export OPENAI_API_KEY="sk-..."
export PERPLEXITY_API_KEY="pplx-..."
export GEMINI_API_KEY="..."
export CEREBRAS_API_KEY="..."
```

### 10.6 Pre-commit checks

Always run before committing:

```bash
cargo +nightly fmt --all                               # format (nightly matches CI)
cargo clippy --workspace --no-deps -- -D warnings      # must pass clean
cargo test --workspace                                 # must pass
```

The CI enforces all three. Skip at your peril.

---

## Key file locations

| What | Path |
|---|---|
| Workspace config | `roko.toml` |
| Data directory | `.roko/` |
| Executor checkpoints | `.roko/state/` |
| PRD storage | `.roko/prd/` |
| Research artifacts | `.roko/research/` |
| Learning state | `.roko/learn/` |
| Runtime events | `.roko/runtime-events.jsonl` |
| Episodes | `.roko/learn/episodes.jsonl` |
| Cascade router state | `.roko/learn/cascade-router.json` |
| Gate thresholds | `.roko/learn/gate-thresholds.json` |
| Prompt experiments | `.roko/learn/experiments.json` |
| Knowledge scores | `.roko/learn/knowledge-scores.json` |
| Gap tracker | `.roko/GAPS.md` |
| Foundation traits | `crates/roko-core/src/foundation.rs` |
| RuntimeEvent types | `crates/roko-core/src/runtime_event.rs` |
| WorkflowEngine | `crates/roko-runtime/src/workflow_engine.rs` |
| PipelineStateV2 | `crates/roko-runtime/src/pipeline_state.rs` |
| EffectDriver | `crates/roko-runtime/src/effect_driver.rs` |
| ModelCallService | `crates/roko-agent/src/model_call_service.rs` |
| PromptAssemblyService | `crates/roko-compose/src/prompt_assembly_service.rs` |
| FeedbackService | `crates/roko-learn/src/feedback_service.rs` |
| GateService | `crates/roko-gate/src/gate_service.rs` |
| CascadeRouter | `crates/roko-learn/src/cascade_router.rs` |
| Orchestrator | `crates/roko-cli/src/orchestrate.rs` |
| HTTP routes | `crates/roko-serve/src/routes/` |
| TUI | `crates/roko-cli/src/tui/` |

## Known gaps

The following items are built but not fully wired at runtime:

| Gap | Status |
|---|---|
| Knowledge-informed model routing | Neuro store not yet consulted in CascadeRouter |
| Cold substrate archival | Built in `roko-dreams` but no cron trigger |
| `force_backend` override learning | CascadeRouter does not learn from manual overrides |
| Chain runtime integration | Phase 2+ — requires blockchain backend |
| VCG auction in composition | Built but greedy path dominates at runtime |
| Safety contracts | `AgentContract` falls back to permissive default when YAML missing |
| LLM judge gate | Stub implementation; not wired to a live model |

Track current gaps in `.roko/GAPS.md`.
