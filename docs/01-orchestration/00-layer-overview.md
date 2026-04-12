# L4 Orchestration — Layer Overview

> **Layer**: L4 Orchestration
> **Crate**: `roko-orchestrator` (`crates/roko-orchestrator/`)
> **Runtime harness**: `roko-cli/src/orchestrate.rs`
> **Status**: Wired end-to-end. Plan-execute-gate-persist loop operational.


> **Implementation**: Shipping

---

## Purpose

L4 Orchestration is the topmost layer of the Roko five-layer architecture. It is
the control plane that coordinates multiple agents working on multiple plans
simultaneously. Everything below L4 — the L3 Harness (agent pools, MCP,
safety), L2 Scaffold (prompt assembly, gate pipeline), L1 Framework (Synapse
traits, Engram types), and L0 Runtime (file substrate, cancellation, event
bus) — provides building blocks. L4 composes them into a self-hosting
development loop.

The orchestrator's job is to answer one question: **given N plans, M agents,
and a finite budget, what should happen next?** It answers this by maintaining
a pure state machine that emits action requests — spawn an agent, run a gate,
merge a branch — while the runtime harness (`PlanRunner` in `orchestrate.rs`)
dispatches those actions to real subsystems and feeds results back as events.

This separation between the pure state machine and the effectful runtime is the
central architectural decision of L4. It makes the orchestrator testable,
snapshot-serializable, and crash-recoverable without mocking I/O.

---

## Position in the Five-Layer Architecture

The five-layer model (described in `refactoring-prd/02-five-layers.md`) organizes
Roko's architecture by dependency direction — each layer depends only on layers
below it:

| Layer | Name | Responsibility | Key Crate |
|-------|------|----------------|-----------|
| **L4** | **Orchestration** | Plan DAGs, parallel execution, merge serialization, crash recovery, replan | `roko-orchestrator` + `roko-cli` |
| L3 | Harness | Agent pools, MCP integration, safety layer, tool dispatch | `roko-agent` + `bardo-runtime` |
| L2 | Scaffold | Prompt assembly, gate pipeline, adaptive thresholds, system prompt builder | `roko-compose` + `roko-gate` |
| L1 | Framework | Synapse traits (Substrate, Scorer, Gate, Router, Composer, Policy), Engram types, config schema | `roko-core` |
| L0 | Runtime | File substrate (JSONL), cancellation tokens, event bus, observability sinks | `roko-fs` + `bardo-runtime` |

L4 is the only layer that knows about plans, tasks, DAGs, worktrees, and the
overall execution lifecycle. It is also the only layer that coordinates across
plans — resolving file conflicts, serializing merges, and managing the execution
queue.

### Dependency direction

L4 depends on every layer below it. It imports:

- **L3**: `roko_agent::ClaudeCliAgent`, `roko_agent::ExecAgent`,
  `roko_agent::mcp::McpConfig`, `bardo_runtime::ProcessSupervisor`,
  `bardo_runtime::cancel::CancelToken`
- **L2**: `roko_compose::PromptComposer`, `roko_compose::RoleSystemPromptSpec`,
  `roko_gate::compile::CompileGate`, `roko_gate::test_gate::TestGate`,
  `roko_gate::clippy_gate::ClippyGate`,
  `roko_gate::adaptive_threshold::AdaptiveThresholds`
- **L1**: `roko_core::Signal`, `roko_core::AgentRole`, `roko_core::PlanPhase`,
  `roko_core::Verdict`, `roko_core::Budget`, `roko_core::Substrate`
- **L0**: `roko_fs::FileSubstrate`, `roko_fs::RokoLayout`

No layer below L4 imports `roko-orchestrator`. This ensures the orchestrator
can be replaced or extended without affecting the framework.

### Cross-cuts

Three cognitive cross-cuts span all five layers:

1. **Neuro** — knowledge store, Engram decay, tier management
2. **Daimon** — PAD affect vector, behavioral state modulation
3. **Dreams** — idle-time consolidation, NREM replay, REM imagination

L4 interacts with all three:

- **Neuro**: `PlanRunner` maintains a `KnowledgeStore` and queries it per-task
  for scoped context. Successful task patterns are distilled into knowledge
  entries.
- **Daimon**: `PlanRunner` holds a `DaimonState` and uses it to modulate
  dispatch parameters — arousal influences task prioritization, confidence
  affects model selection.
- **Dreams**: `DreamRunner` integration is available for Delta-frequency
  consolidation during idle periods.

---

## The Separation: Pure State Machine vs. Effectful Harness

The orchestrator is divided into two halves:

### Pure state machine (`roko-orchestrator`)

Located in `crates/roko-orchestrator/`, this crate contains:

- **`ParallelExecutor`** — the top-level state machine. Holds per-plan
  `PlanState` entries, an execution queue, and an `ExecutorConfig`. Never does
  I/O. Its `tick()` method returns `Vec<ExecutorAction>` — requests for the
  runtime to fulfill. Its `apply_event()` method accepts `ExecutorEvent` values
  — results from the runtime — and transitions plan phases accordingly.

- **`PlanStateMachine`** — the phase transition logic. Given a `PlanState` and
  an `ExecutorEvent`, it computes the next `PlanPhase` or rejects the
  transition. It also suggests the next `ExecutorAction` for any given phase.

- **`UnifiedTaskDag`** — cross-plan task scheduling with file-conflict
  detection, topological sort, wave computation, and critical path estimation.

- **`MergeQueue`** — file-conflict-aware merge serialization with priority
  ordering and retry-with-backoff.

- **`WorktreeManager`** — per-plan git worktree lifecycle (create, remove,
  health check, idle reclamation).

- **`EventLog`** — append-only, hash-chained event log for tamper-evident
  audit trail and crash recovery.

- **`RecoveryEngine`** — crash recovery from executor snapshots and event-log
  replay.

- **`PostMergeRunner`** — post-merge regression detection and follow-up.

All of these types are `Serialize + Deserialize` where needed. None of them
perform I/O. They are testable with simple in-memory construction.

### Effectful runtime harness (`roko-cli/src/orchestrate.rs`)

Located in `crates/roko-cli/src/orchestrate.rs`, the `PlanRunner` struct is
the runtime harness that connects the pure state machine to real side effects:

```
PlanRunner {
    executor: ParallelExecutor,       // pure state machine
    event_log: EventLog,              // hash-chained audit log
    worktrees: WorktreeManager,       // git worktree lifecycle
    post_merge: PostMergeRunner,      // regression detection
    learning: LearningRuntime,        // episode logger, model router, experiments
    daimon: DaimonState,              // affect modulation
    skill_library: SkillLibrary,      // reusable task patterns
    knowledge_store: KnowledgeStore,  // durable knowledge
    supervisor: ProcessSupervisor,    // agent process lifecycle
    conductor: Arc<Conductor>,        // anomaly detection
    adaptive_thresholds: AdaptiveThresholds,  // gate retry budgets
    metrics: Arc<MetricRegistry>,     // prometheus-style metrics
    // ... 30+ fields total
}
```

`PlanRunner` implements the dispatch loop:

1. Call `executor.tick()` to get `Vec<ExecutorAction>`
2. For each action:
   - `SpawnAgent` → build `AgentRunConfig`, launch `ClaudeCliAgent` or
     `ExecAgent` in a `JoinSet`
   - `RunGate` → invoke `CompileGate`, `TestGate`, `ClippyGate` in sequence
   - `MergeBranch` → git merge from plan worktree into batch branch
   - `DispatchPlan` → create worktree, initialize task tracker
   - `PausePlan` / `ResumePlan` → toggle `paused` flag
3. Feed results back as `ExecutorEvent` values
4. Auto-save executor snapshot every `AUTOSAVE_INTERVAL` (5) actions
5. Log events to hash-chained `EventLog`

This division means you can unit-test the entire orchestration logic — phase
transitions, queue ordering, conflict detection — without spawning processes or
touching the filesystem.

---

## Key Concepts

### Plan

A plan is a unit of work defined by a directory containing `plan.md`
(with optional YAML frontmatter) and `tasks.toml`. Plans are discovered by
scanning a `plans/` directory. Each plan gets:

- A `PlanState` in the executor
- A git worktree for isolated work
- A `TaskTracker` for per-task progress
- An entry in the execution queue

### Phase lifecycle

Each plan progresses through a defined sequence of phases:

```
Queued → Enriching → Implementing → Gating → Verifying → Reviewing
       → DocRevision → Merging → Complete
```

With retry loops:

- `Gating → AutoFixing → Gating` (up to 5 iterations)
- `Verifying → RegeneratingVerify → Verifying`
- `Reviewing → Implementing` (on rejection)

And terminal states: `Complete`, `Failed`, `Skipped`.

### ExecutorAction

The vocabulary of side-effects the executor can request:

| Action | Effect |
|--------|--------|
| `DispatchPlan` | Begin executing a queued plan |
| `SpawnAgent` | Launch an agent process for a task (with role: Implementer, Strategist, Auditor, AutoFixer, Scribe) |
| `RunGate` | Execute a gate rung (compile, test, clippy) |
| `RunVerify` | Run task-level verification commands |
| `MergeBranch` | Merge plan worktree into batch branch |
| `FailPlan` | Mark plan as terminally failed |
| `CompletePlan` | Mark plan as complete |
| `PausePlan` | Pause a running plan |
| `ResumePlan` | Resume a paused plan |
| `Reorder` | Move a plan in the queue |

### ExecutorEvent

Events fed back from the runtime to drive state transitions:

| Event | Meaning |
|-------|---------|
| `Start` | Plan has been dispatched |
| `EnrichmentDone` | Enrichment phase completed |
| `ImplementationDone` | All tasks in current iteration done |
| `GatePassed` / `GateFailed` | Gate verdict |
| `AutoFixDone` | Auto-fix agent completed |
| `VerifyPassed` / `VerifyFailed` | Verification result |
| `ReviewApproved` / `ReviewRejected` | Auditor verdict |
| `DocRevisionDone` | Documentation revision completed |
| `MergeSucceeded` / `MergeFailed` | Merge outcome |
| `Skip` | Operator skip |
| `Fatal(reason)` | Unrecoverable failure |

---

## What L4 Orchestration Enables

With all components wired, L4 enables the Roko self-hosting loop:

```bash
# Capture → Draft → Research → Plan → Execute → Resume
roko prd idea "Wire SystemPromptBuilder into orchestrate.rs"
roko prd draft new "system-prompt-wiring"
roko research enhance-prd system-prompt-wiring
roko prd plan system-prompt-wiring
roko plan run plans/
roko plan run plans/ --resume .roko/state/executor.json
```

Each step of this loop is a CLI command backed by L4 orchestration:

1. **Plan discovery** scans `plans/`, parses frontmatter, ranks by priority
2. **DAG construction** builds a cross-plan task graph with file-conflict edges
3. **Parallel execution** dispatches agents to worktrees, up to configured limits
4. **Gate validation** runs compile/test/clippy per plan, with auto-fix retries
5. **Merge serialization** queues merges with conflict detection and retry
6. **Crash recovery** restores state from snapshots and event-log replay
7. **Learning feedback** records episodes, updates model routing, adapts thresholds

The orchestrator does not merely run tasks. It coordinates a multi-agent system
where agents modify a shared codebase through isolated worktrees, gates ensure
correctness, and merges are serialized to prevent conflicts. This is stigmergic
coordination via git — agents communicate indirectly through the codebase they
modify (Grassé 1959; Parunak 2002).

---

## Sub-document Map

This document set covers L4 Orchestration in depth across 14 sub-documents:

| # | Document | Topic |
|---|----------|-------|
| 00 | This document | Layer overview, architecture position, key concepts |
| 01 | `01-plan-discovery.md` | Plan scanning, frontmatter parsing, ranking |
| 02 | `02-unified-task-dag.md` | Cross-plan DAG, file conflicts, topological sort, waves |
| 03 | `03-parallel-executor.md` | Pure state machine, tick/event loop, config |
| 04 | `04-plan-phases.md` | Phase lifecycle, state transitions, retry loops |
| 05 | `05-executor-actions.md` | Action vocabulary, dispatch semantics |
| 06 | `06-runtime-harness.md` | PlanRunner, agent dispatch, gate invocation |
| 07 | `07-worktree-isolation.md` | Per-plan worktrees, branch naming, health, reclamation |
| 08 | `08-merge-queue.md` | File-conflict-aware merge serialization |
| 09 | `09-snapshot-recovery.md` | Crash recovery, event-log replay, validation |
| 10 | `10-event-log.md` | Hash-chained event sourcing, tamper detection |
| 11 | `11-conductor-integration.md` | Anomaly detection, Yerkes-Dodson dynamics |
| 12 | `12-stigmergy-niche.md` | Stigmergic coordination via git, niche construction |
| 13 | `13-cross-domain-orchestration.md` | Multi-domain DAGs (code + chain + research) |

---

## References

- Grassé, P.-P. (1959). La reconstruction du nid et les coordinations interindividuelles chez Bellicositermes natalensis et Cubitermes sp. *Insectes Sociaux*, 6(1), 41–80.
- Parunak, H. V. D. (2002). Digital pheromones for coordination of unmanned vehicles. *AAMAS 2002*.
- Dorigo, M. & Gambardella, L. M. (1997). Ant colony system: A cooperative learning approach to the traveling salesman problem. *IEEE Trans. Evolutionary Computation*, 1(1), 53–66.
- Woolley, A. W. et al. (2010). Evidence for a collective intelligence factor in the performance of human groups. *Science*, 330(6004), 686–688.
- Tomasello, M. (2014). *A Natural History of Human Thinking*. Harvard University Press.
- Odling-Smee, F. J., Laland, K. N. & Feldman, M. W. (2003). *Niche Construction: The Neglected Process in Evolution*. Princeton University Press.
- Yerkes, R. M. & Dodson, J. D. (1908). The relation of strength of stimulus to rapidity of habit-formation. *Journal of Comparative Neurology and Psychology*, 18(5), 459–482.
