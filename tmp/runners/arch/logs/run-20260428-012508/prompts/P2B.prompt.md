# Architecture Batch P2B — TaskScheduler (pure DAG)

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

## Batch P2B: TaskScheduler (Pure DAG)

### Write Scope
- **CREATE**: `crates/roko-runtime/src/task_scheduler.rs`
- **MODIFY**: `crates/roko-runtime/src/lib.rs` (add `pub mod task_scheduler;` and re-export)

### Dependencies
- None (this is a standalone pure-logic module)

### DO NOT
- Modify any other files
- Add Cargo.toml dependencies
- Put side-effects (I/O, spawning, etc.) in the scheduler — it's pure logic
- Create a new crate

### Task

Create `TaskScheduler` — a pure DAG scheduler that determines which tasks are ready to run
based on dependency resolution. No execution logic — just "given these tasks and their
dependencies, which are ready?"

This is used by the WorkflowEngine (P2D) for multi-task plan execution.

#### File: `crates/roko-runtime/src/task_scheduler.rs`

```rust
//! TaskScheduler — pure DAG dependency resolver.
//!
//! Given a set of tasks with dependencies, determines which tasks are ready
//! to run. No execution logic — just scheduling decisions.
//!
//! Used by WorkflowEngine for multi-task plan execution.

use std::collections::{HashMap, HashSet, VecDeque};

/// A task in the DAG.
#[derive(Debug, Clone)]
pub struct SchedulableTask {
    /// Unique task identifier
    pub id: String,
    /// Task IDs this task depends on (must complete first)
    pub depends_on: Vec<String>,
    /// Files this task will modify (for exclusion checking)
    pub files: Vec<String>,
}

/// Current status of a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    /// Waiting for dependencies
    Blocked,
    /// Dependencies satisfied, ready to run
    Ready,
    /// Currently executing
    Running,
    /// Completed successfully
    Completed,
    /// Failed
    Failed { error: String },
    /// Skipped (dependency failed)
    Skipped,
}

/// Pure DAG scheduler. No side-effects.
#[derive(Debug)]
pub struct TaskScheduler {
    tasks: HashMap<String, SchedulableTask>,
    status: HashMap<String, TaskStatus>,
    /// Maximum number of tasks that can run in parallel
    max_parallel: usize,
}

impl TaskScheduler {
    /// Create a new scheduler with the given tasks and parallelism limit.
    pub fn new(tasks: Vec<SchedulableTask>, max_parallel: usize) -> Self {
        let mut status = HashMap::new();
        let task_map: HashMap<String, SchedulableTask> = tasks
            .into_iter()
            .map(|t| {
                status.insert(t.id.clone(), TaskStatus::Blocked);
                (t.id.clone(), t)
            })
            .collect();

        let mut scheduler = Self {
            tasks: task_map,
            status,
            max_parallel,
        };
        scheduler.update_ready();
        scheduler
    }

    /// Get all tasks that are ready to run right now.
    pub fn ready_tasks(&self) -> Vec<&str> {
        self.status
            .iter()
            .filter(|(_, s)| **s == TaskStatus::Ready)
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// Get tasks that can be started now, respecting max_parallel and file exclusion.
    pub fn next_batch(&self) -> Vec<&str> {
        let running_count = self.status.values().filter(|s| **s == TaskStatus::Running).count();
        let available_slots = self.max_parallel.saturating_sub(running_count);

        if available_slots == 0 {
            return Vec::new();
        }

        // Collect files currently being modified by running tasks
        let running_files: HashSet<&str> = self
            .status
            .iter()
            .filter(|(_, s)| **s == TaskStatus::Running)
            .flat_map(|(id, _)| {
                self.tasks
                    .get(id)
                    .map(|t| t.files.iter().map(|f| f.as_str()).collect::<Vec<_>>())
                    .unwrap_or_default()
            })
            .collect();

        let mut batch = Vec::new();
        let mut batch_files: HashSet<&str> = HashSet::new();

        for (id, status) in &self.status {
            if *status != TaskStatus::Ready {
                continue;
            }
            if batch.len() >= available_slots {
                break;
            }

            // Check file exclusion
            let task = match self.tasks.get(id) {
                Some(t) => t,
                None => continue,
            };

            let has_conflict = task.files.iter().any(|f| {
                running_files.contains(f.as_str()) || batch_files.contains(f.as_str())
            });

            if !has_conflict {
                for f in &task.files {
                    batch_files.insert(f.as_str());
                }
                batch.push(id.as_str());
            }
        }

        batch
    }

    /// Mark a task as running.
    pub fn mark_running(&mut self, task_id: &str) {
        if let Some(status) = self.status.get_mut(task_id) {
            *status = TaskStatus::Running;
        }
    }

    /// Mark a task as completed. Updates downstream dependencies.
    pub fn mark_completed(&mut self, task_id: &str) {
        if let Some(status) = self.status.get_mut(task_id) {
            *status = TaskStatus::Completed;
        }
        self.update_ready();
    }

    /// Mark a task as failed. Skips downstream dependents.
    pub fn mark_failed(&mut self, task_id: &str, error: String) {
        if let Some(status) = self.status.get_mut(task_id) {
            *status = TaskStatus::Failed { error };
        }
        self.skip_dependents(task_id);
        self.update_ready();
    }

    /// Check if all tasks are in a terminal state.
    pub fn is_done(&self) -> bool {
        self.status.values().all(|s| matches!(s, TaskStatus::Completed | TaskStatus::Failed { .. } | TaskStatus::Skipped))
    }

    /// Get the status of a specific task.
    pub fn task_status(&self, task_id: &str) -> Option<&TaskStatus> {
        self.status.get(task_id)
    }

    /// Get a summary: (completed, failed, skipped, running, blocked, ready)
    pub fn summary(&self) -> (usize, usize, usize, usize, usize, usize) {
        let mut completed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        let mut running = 0;
        let mut blocked = 0;
        let mut ready = 0;

        for status in self.status.values() {
            match status {
                TaskStatus::Completed => completed += 1,
                TaskStatus::Failed { .. } => failed += 1,
                TaskStatus::Skipped => skipped += 1,
                TaskStatus::Running => running += 1,
                TaskStatus::Blocked => blocked += 1,
                TaskStatus::Ready => ready += 1,
            }
        }

        (completed, failed, skipped, running, blocked, ready)
    }

    // ── internal ──

    fn update_ready(&mut self) {
        let blocked_ids: Vec<String> = self
            .status
            .iter()
            .filter(|(_, s)| **s == TaskStatus::Blocked)
            .map(|(id, _)| id.clone())
            .collect();

        for id in blocked_ids {
            if let Some(task) = self.tasks.get(&id) {
                let all_deps_done = task.depends_on.iter().all(|dep| {
                    matches!(self.status.get(dep), Some(TaskStatus::Completed))
                });
                if all_deps_done {
                    self.status.insert(id, TaskStatus::Ready);
                }
            }
        }
    }

    fn skip_dependents(&mut self, failed_id: &str) {
        let mut to_skip: VecDeque<String> = VecDeque::new();

        // Find all tasks that depend (directly or transitively) on the failed task
        for (id, task) in &self.tasks {
            if task.depends_on.iter().any(|d| d == failed_id) {
                to_skip.push_back(id.clone());
            }
        }

        while let Some(skip_id) = to_skip.pop_front() {
            if let Some(status) = self.status.get_mut(&skip_id) {
                if !matches!(status, TaskStatus::Completed | TaskStatus::Failed { .. } | TaskStatus::Skipped) {
                    *status = TaskStatus::Skipped;
                    // Also skip transitive dependents
                    for (id, task) in &self.tasks {
                        if task.depends_on.iter().any(|d| d == &skip_id) {
                            to_skip.push_back(id.clone());
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tasks() -> Vec<SchedulableTask> {
        vec![
            SchedulableTask { id: "T1".into(), depends_on: vec![], files: vec!["a.rs".into()] },
            SchedulableTask { id: "T2".into(), depends_on: vec![], files: vec!["b.rs".into()] },
            SchedulableTask { id: "T3".into(), depends_on: vec!["T1".into(), "T2".into()], files: vec!["c.rs".into()] },
            SchedulableTask { id: "T4".into(), depends_on: vec!["T3".into()], files: vec!["d.rs".into()] },
        ]
    }

    #[test]
    fn initial_ready_tasks() {
        let sched = TaskScheduler::new(make_tasks(), 4);
        let mut ready = sched.ready_tasks();
        ready.sort();
        assert_eq!(ready, vec!["T1", "T2"]);
    }

    #[test]
    fn completing_unblocks_dependents() {
        let mut sched = TaskScheduler::new(make_tasks(), 4);
        sched.mark_running("T1");
        sched.mark_completed("T1");
        sched.mark_running("T2");
        sched.mark_completed("T2");

        let ready = sched.ready_tasks();
        assert_eq!(ready, vec!["T3"]);
    }

    #[test]
    fn failure_skips_dependents() {
        let mut sched = TaskScheduler::new(make_tasks(), 4);
        sched.mark_running("T1");
        sched.mark_failed("T1", "compile error".into());

        assert!(matches!(sched.task_status("T3"), Some(TaskStatus::Skipped)));
        assert!(matches!(sched.task_status("T4"), Some(TaskStatus::Skipped)));
    }

    #[test]
    fn file_exclusion() {
        let tasks = vec![
            SchedulableTask { id: "A".into(), depends_on: vec![], files: vec!["shared.rs".into()] },
            SchedulableTask { id: "B".into(), depends_on: vec![], files: vec!["shared.rs".into()] },
            SchedulableTask { id: "C".into(), depends_on: vec![], files: vec!["other.rs".into()] },
        ];
        let sched = TaskScheduler::new(tasks, 4);
        let batch = sched.next_batch();
        // A and B conflict on shared.rs, so only one of them + C should be in the batch
        assert!(batch.len() <= 3);
        assert!(!(batch.contains(&"A") && batch.contains(&"B")));
    }

    #[test]
    fn respects_max_parallel() {
        let tasks = vec![
            SchedulableTask { id: "A".into(), depends_on: vec![], files: vec![] },
            SchedulableTask { id: "B".into(), depends_on: vec![], files: vec![] },
            SchedulableTask { id: "C".into(), depends_on: vec![], files: vec![] },
        ];
        let sched = TaskScheduler::new(tasks, 2);
        let batch = sched.next_batch();
        assert!(batch.len() <= 2);
    }

    #[test]
    fn is_done_when_all_terminal() {
        let mut sched = TaskScheduler::new(make_tasks(), 4);
        sched.mark_running("T1");
        sched.mark_failed("T1", "err".into());
        sched.mark_running("T2");
        sched.mark_completed("T2");
        // T3, T4 should be skipped due to T1 failure
        assert!(sched.is_done());
    }

    #[test]
    fn summary_counts() {
        let mut sched = TaskScheduler::new(make_tasks(), 4);
        let (c, f, s, r, b, rdy) = sched.summary();
        assert_eq!(c, 0);
        assert_eq!(rdy, 2); // T1, T2
        assert_eq!(b, 2);   // T3, T4
    }
}
```

#### Modification: `crates/roko-runtime/src/lib.rs`

Add:
```rust
pub mod task_scheduler;
pub use task_scheduler::{TaskScheduler, SchedulableTask, TaskStatus};
```

### Done Criteria
```bash
grep -q 'pub struct TaskScheduler' crates/roko-runtime/src/task_scheduler.rs
grep -q 'pub fn next_batch' crates/roko-runtime/src/task_scheduler.rs
grep -q 'pub mod task_scheduler' crates/roko-runtime/src/lib.rs
cargo check -p roko-runtime
cargo test -p roko-runtime --lib -- task_scheduler
```
