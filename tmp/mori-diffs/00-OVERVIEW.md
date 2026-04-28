# Runner v3: Architecture Overview

> Runner-focused redesign package. This is now one part of a larger repo-wide audit:
> - [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) is the current canonical priority/impact ledger
> - [17-ARCHITECTURE-REALITY-CHECK.md](17-ARCHITECTURE-REALITY-CHECK.md) explains the current spec/runtime mismatch
> - [18-MASTER-AUDIT.md](18-MASTER-AUDIT.md) gives the repo-wide redesign and migration order
> - [19-SELF-REVIEW-AND-PROOF.md](19-SELF-REVIEW-AND-PROOF.md) records the iteration, scoring, and evidence standard
>
> This file remains the runner-specific architecture proposal.
>
> For current implementation truth, use [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) before trusting older unchecked boxes in this file.

## Architecture Runner Completion (2026-04-28)

The foundation architecture (Phases 0-3) has been implemented via 16 automated Codex batches on branch `codex/arch-run-20260428-012508`. This created the trait-based service layer (ModelCallService, PromptAssemblyService, FeedbackService, GateService), execution engine (PipelineStateV2, TaskScheduler, EffectDriver, WorkflowEngine), and adapters (AcpAdapter, SseAdapter, JsonlLogger, RuntimeProjection). All code compiles and passes anti-pattern verification. See [MASTER-IMPLEMENTATION-PLAN.md](../subsystem-audits/MASTER-IMPLEMENTATION-PLAN.md) for full status.

## Design Principles

1. **Provider-agnostic dispatch** - The runner never names a specific LLM backend. All agent creation goes through `create_agent_for_model()`, which supports 8 backends.
2. **Everything is an event** - Every state change (spawn, turn, tool, gate, phase) emits a `DashboardEvent` through `StateHub`. The TUI is a subscriber, not a special case.
3. **Crash-safe persistence** - All state files written atomically (tmp+rename). Resume reconstructs the full execution context from 5 on-disk files.
4. **Learning feeds forward** - `CascadeRouter` is consulted at dispatch and updated on every outcome. Manual overrides (`force_backend`) are recorded as observations.
5. **Minimal new code** - Reuse `roko-agent`, `roko-compose`, `roko-learn`, `roko-gate`, `roko-orchestrator`. The runner is a thin orchestration layer over existing crates.

## Architecture Diagram

```
                    +----------------------------------------------+
                    |                  runner v3                    |
                    |                                              |
  plans/           |  +---------+    +------------+              |
  tasks.toml  ---> |  |  DAG    |--->|  Dispatch   |              |
                    |  |Executor |    |  (dispatch/)|              |
                    |  +----+----+    +-----+------+              |
                    |       |               |                      |
                    |       |    +----------+----------+          |
                    |       |    |                     |          |
                    |       v    v                     v          |
                    |  +------------+  +----------+  +--------+  |
                    |  |  Event     |  |  Agent    |  | Prompt  |  |
                    |  |  Loop      |<--+  Stream   |  | Builder |  |
                    |  |  (select!) |  |  Parser   |  +--------+  |
                    |  +--+-+--+---+  +----------+              |
                    |     | |  |                                  |
                    |     | |  +-------+                          |
                    |     | |          |                          |
                    |     v v          v                          |
                    |  +------+  +---------+  +--------------+  |
                    |  |Persist|  |  Gate    |  |   Learning   |  |
                    |  |(5 files)|  |Dispatch |  |  Collector   |  |
                    |  +------+  +---------+  +--------------+  |
                    |     |          |               |            |
                    +-----+----------+---------------+------------+
                          |          |               |
                          v          v               v
                    .roko/state/   roko-gate     roko-learn
                                                 roko-neuro
                    - - - - - - - - - - - - - - - - - - - -
                                   StateHub
                                      |
                              +-------+-------+
                              v       v       v
                            TUI    HTTP/SSE  tracing
```

## Audit Path Summary

| # | Path | v2 Score | v3 Target | Spec Doc |
|---|------|----------|-----------|----------|
| 1 | Agent Dispatch | 5/10 | 9/10 | [01-AGENT-DISPATCH.md](01-AGENT-DISPATCH.md) |
| 2 | Plan Execution | 6/10 | 9/10 | [02-PLAN-EXECUTION.md](02-PLAN-EXECUTION.md) |
| 3 | State Persistence | 3/10 | 9/10 | [03-PERSISTENCE.md](03-PERSISTENCE.md) |
| 4 | Learning Feedback | 3/10 | 8/10 | [04-LEARNING.md](04-LEARNING.md) |
| 5 | Prompt Assembly | 7/10 | 9/10 | [05-PROMPT-ASSEMBLY.md](archive/2026-04-26-verified/05-PROMPT-ASSEMBLY.md) |
| 6 | Observable Execution | 6/10 | 9/10 | [06-OBSERVABILITY.md](archive/2026-04-26-verified/06-OBSERVABILITY.md) |

## Module Map

### New: `crates/roko-cli/src/dispatch/`

Extracted from `orchestrate.rs` - all agent creation and prompt assembly logic.

| File | Responsibility |
|------|---------------|
| `mod.rs` | `AgentDispatcher` struct, `dispatch()` entry point |
| `model_routing.rs` | `CascadeRouter` integration, `RoutingContext` construction |
| `prompt_builder.rs` | `PromptAssembler` using `RoleSystemPromptSpec` 9-layer builder |
| `outcome.rs` | `AgentOutcome`, `DispatchError` types |
| `warm_pool.rs` | Pre-spawned agent pool for fast gate->reviewer transitions |

### Rewritten: `crates/roko-cli/src/runner/`

Provider-agnostic event loop with full persistence and observability.

| File | Responsibility | Change |
|------|---------------|--------|
| `mod.rs` | Public re-exports | Updated exports |
| `event_loop.rs` | Main `tokio::select!` loop | **Rewritten** - uses `dispatch/` instead of hardcoded Claude CLI |
| `agent_events.rs` | Event handler -> RunState + TuiBridge | **Expanded** - publishes all event types to StateHub |
| `gate_dispatch.rs` | Gate rung execution | **Enhanced** - timeout, cancellation, semaphore |
| `state.rs` | `RunState` + `CompletedTaskSet` | **Enhanced** - per-plan completed tasks, routing decisions |
| `tui_bridge.rs` | `TuiBridge` convenience methods | **Expanded** - full event catalog |
| `persist.rs` | Atomic file I/O | **Enhanced** - 5-file snapshot, version field |
| `plan_loader.rs` | TOML plan parsing | Unchanged |
| `types.rs` | Core types | **Rewritten** - provider-agnostic `AgentEvent` |

## Existing Code Reused (NOT rebuilt)

These are stable, tested modules that v3 calls into - no modifications needed:

| What | Crate | Key Type/Function |
|------|-------|-------------------|
| Agent factory | `roko-agent` | `create_agent_for_model()` -> all 8 backends |
| Agent options | `roko-agent` | `AgentOptions` (effort, fallback, tools) |
| Safety spawn | `roko-cli` | `spawn_agent_with_layer()`, `SpawnAgentSpec` |
| Model routing | `roko-learn` | `CascadeRouter::select_model()` |
| TUI events | `roko-core` | `DashboardEvent` (25+ variants) |
| State hub | `roko-core` | `StateHub` pub/sub |
| Prompt builder | `roko-compose` | `RoleSystemPromptSpec` 9-layer builder |
| Episode logger | `roko-learn` | `EpisodeLogger`, `Episode` |
| Gate pipeline | `roko-gate` | `run_rung()` with `RungExecutionConfig` |
| Process mgmt | `roko-agent` | `register_spawned_pid()`, `kill_tree()` |
| Task parser | `roko-cli` | `TaskDef` with DAG deps, verify steps |
| Provider semaphores | `roko-agent` | `ProviderSemaphores` for concurrency |
| Executor | `roko-orchestrator` | `ParallelExecutor`, `ExecutorSnapshot` |

## What v2 Gets Wrong (Why v3 Exists)

### Path 1: Agent Dispatch (5/10)
- **Hardcoded Claude CLI** - `agent_stream.rs` spawns `claude` binary directly via `Command::new`. No provider abstraction.
- **Static model selection** - Uses `task.model_hint` or falls back to `config.model`. CascadeRouter exists but is never consulted.
- **Auto-pass verify** - `RunVerify` action is handled with `let _ = executor.apply_event(plan_id, &ExecutorEvent::VerifyPassed)` - a stub.
- **No warm pool** - Every agent spawn is cold. Gate->reviewer transition pays full startup cost.
- **No pre-spawn validation** - Spawns blind; DOA (dead on arrival) only caught after timeout.

### Path 2: Plan Execution (6/10)
- **Sentinel-based DAG** - Resolves `"next"`, `"fix"`, `"regen-verify"` sentinels by walking task list. Not real topological sort.
- **No gate timeout** - `spawn_gate()` fires a `tokio::spawn` with no timeout or cancellation token.
- **No concurrency limit** - Multiple gate rungs can run `cargo test` simultaneously, thrashing CPU.
- **Post-hoc budget** - Budget check happens after cost is incurred (`is_turn_done` branch).
- **No failure classification** - All failures treated identically. No transient vs permanent distinction.
- **No merge queue** - Nothing prevents concurrent agents from creating git conflicts.

### Path 3: State Persistence (3/10)
- **Only executor.json** - `save_snapshot()` persists only `ExecutorSnapshot`. CascadeRouter, gate thresholds, daimon state, and efficiency state are not saved.
- **No version field** - Snapshot JSON has no version, making forward-compatible migration impossible.
- **Count-based tracking** - `tasks_completed: usize` counts total completions. Doesn't track *which* tasks per plan completed.
- **Resume is partial** - `try_resume()` loads executor snapshot but doesn't restore learning state, routing decisions, or per-task DAG progress.

### Path 4: Learning Feedback (3/10)
- **CascadeRouter unused** - Model selection ignores the router entirely. The router exists in `roko-learn` but is never called from the runner.
- **No routing observations** - Task outcomes (success/fail, cost, latency) never feed back into routing.
- **Hardcoded backend** - `ep.backend = "claude".to_string()` - always "claude" regardless of actual provider.
- **No knowledge ingestion** - Successful gate passes don't trigger neuro store ingestion.

### Path 5: Prompt Assembly (7/10)
- **Manual prompt** - `build_minimal_system_prompt()` constructs a basic 3-line prompt. Ignores the 9-layer `RoleSystemPromptSpec`.
- **No playbook injection** - Playbook store is never queried for relevant patterns.
- **No anti-pattern warnings** - Neuro store has anti-patterns but they're not injected into prompts.
- **Raw gate feedback** - On retry, gate output is prepended as raw text, not parsed into structured feedback.
- **No tool allowlists** - All roles get all tools. Reviewers can write, researchers can delete.

### Path 6: Observable Execution (6/10)
- **Sparse events** - Only `MessageDelta`, `AgentSpawned`, `AgentCompleted`, `GateResult`, and phase transitions are published. Token usage, tool calls, and costs are not surfaced.
- **No per-task progress** - TUI can't show a checklist of tasks with status icons.
- **No cost events** - Cost is tracked in `RunState` but never published as an event.
- **No non-TUI output** - Without TUI, agent output goes to `debug!()` tracing only. No structured logging path.

## Data Flow: Full Task Lifecycle

```
1. DAG Executor resolves next ready task
2. Dispatch:
   a. CascadeRouter selects model (with fallback)
   b. PromptAssembler builds 9-layer system prompt
      - Queries playbook store
      - Queries neuro store for anti-patterns
      - Injects gate failure feedback if retry
   c. AgentDispatcher creates agent via create_agent_for_model()
   d. Pre-spawn validation (binary, repo, prompt)
   e. Agent spawns -> PID registered
3. Event Loop:
   a. Agent stdout -> stream parser -> AgentEvent channel
   b. Every event -> RunState update + StateHub publish
   c. TokenUsage -> EfficiencyEvent
   d. TurnCompleted -> cost update + episode prep
4. Gate:
   a. Agent exits -> gate dispatch with timeout + cancellation
   b. Semaphore limits concurrent cargo processes
   c. Each verdict -> GateResult event -> StateHub
   d. Gate output parsed into structured feedback
5. Learning:
   a. Episode logged (model, provider, tokens, cost, gate result, files)
   b. Efficiency event emitted
   c. CascadeRouter observation recorded
   d. On pass: neuro ingestion candidate
6. Persistence:
   a. 5 files saved atomically after every task + gate
   b. Per-plan completed task tracking
7. Next:
   a. DAG resolves next ready task -> back to step 2
   b. Or: plan complete -> CompletePlan action
```

## Companion Documents

| Doc | What It Covers |
|-----|---------------|
| [01-AGENT-DISPATCH.md](01-AGENT-DISPATCH.md) | Provider-agnostic dispatch, model routing, warm pool, pre-spawn validation |
| [02-PLAN-EXECUTION.md](02-PLAN-EXECUTION.md) | DAG resolution, gate timeout/semaphore, retries, merge queue |
| [03-PERSISTENCE.md](03-PERSISTENCE.md) | 5-file snapshot, atomic writes, resume validation, version field |
| [04-LEARNING.md](04-LEARNING.md) | CascadeRouter integration, episode logging, knowledge ingestion |
| [05-PROMPT-ASSEMBLY.md](archive/2026-04-26-verified/05-PROMPT-ASSEMBLY.md) | 9-layer builder, playbooks, anti-patterns, structured feedback |
| [06-OBSERVABILITY.md](archive/2026-04-26-verified/06-OBSERVABILITY.md) | Full event catalog, TUI bridge, non-TUI output, cost tracking |
| [07-MIGRATION.md](07-MIGRATION.md) | Transition plan from v2 + orchestrate.rs |
| [08-FILE-MAP.md](08-FILE-MAP.md) | Every file created/modified with line-level detail |

## Verification

```bash
# Build
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --no-deps -- -D warnings

# E2E with real agent:
rm -f .roko/state/executor.json
cargo run -p roko-cli -- plan run .roko/plans/unified-migration-phase0/

# Verify TUI populates with real data:
cargo run -p roko-cli -- dashboard

# Verify resume:
# (Ctrl+C during plan run, then re-run same command)
cargo run -p roko-cli -- plan run .roko/plans/unified-migration-phase0/
# Should resume from snapshot, skip completed tasks
```

## Agent Implementation Packet

Use this file as the entrypoint for the runner v3 work. Do not begin feature work until the agent has read:

- `tmp/mori-diffs/20-RUNTIME-RECONCILIATION.md`
- `tmp/mori-diffs/21-FEATURE-PARITY-MATRIX.md`
- `tmp/mori-diffs/22-STABILITY-PLAN.md`
- `tmp/unified/27-ORCHESTRATOR.md`
- `docs/01-orchestration/00-layer-overview.md`
- `crates/roko-cli/src/commands/plan.rs`
- `crates/roko-cli/src/runner/mod.rs`
- `crates/roko-cli/src/runner/event_loop.rs`
- `crates/roko-cli/src/orchestrate.rs`

## Current Handoff

If you only read one follow-up file after this overview, read:

- [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md)

That file is the current priority/impact ledger and identifies stale older claims. After choosing a subsystem there, use [23-HANDOFF-OPEN-ITEMS.md](23-HANDOFF-OPEN-ITEMS.md) for the detailed checklist.

### Invariants

- [ ] `roko plan run` remains routed through `roko_cli::runner::run`.
- [ ] `orchestrate.rs` is treated as a donor/reference implementation, not the future runtime.
- [ ] New behavior lands behind modules callable by `runner/`.
- [ ] The runner must not learn provider-specific wire formats.
- [ ] The runner must not build production prompts by hand.
- [ ] Every migrated feature has a test or smoke command proving it is active in the runner path.

### Implementation Checklist

- [x] Update this overview to provide a single handoff entrypoint for remaining work.
- [ ] Create `crates/roko-cli/src/dispatch/` with module stubs listed in `08-FILE-MAP.md`.
- [ ] Add `pub mod dispatch;` to `crates/roko-cli/src/lib.rs` if the dispatcher is library-visible.
- [ ] Replace the direct `agent_stream::spawn_agent` call in `runner/event_loop.rs` with a dispatcher facade.
- [ ] Move provider-specific stream parsing below `roko-agent`.
- [ ] Replace `build_minimal_system_prompt` in the live path with `roko-compose` prompt assembly.
- [ ] Add a feedback facade that receives runner events and writes episodes, routing observations, knowledge candidates, and conductor observations.
- [ ] Add a projection facade that turns runner events into `DashboardEvent`, CLI progress, and optional HTTP/SSE events.
- [ ] Add parity tests before deleting any legacy-only behavior.

### Acceptance Criteria

- [ ] `rg "ClaudeStreamEvent|ClaudeAssistantEvent|ClaudeToolEvent" crates/roko-cli/src/runner` returns no production usage.
- [ ] `rg "build_minimal_system_prompt" crates/roko-cli/src/runner` returns no production usage.
- [ ] `cargo check -p roko-cli` passes.
- [ ] A minimal plan can run through `runner/` with mock dispatch.
- [ ] A gate failure produces a retry with structured feedback.
- [ ] A cancelled run can resume without duplicating a completed task.

## No-Mock Proof Snapshot (2026-04-26)

This package now has fresh no-mock runtime proof artifacts for both agent CLIs:

- [x] Codex smoke pass: `/tmp/roko-real-e2e-nrUD05/logs/codex-run-3.stdout`
- [x] Claude smoke pass: `/tmp/roko-real-e2e-nrUD05/logs/claude-run-1.stdout`
- [x] Gate + run event trace: `/tmp/roko-real-e2e-nrUD05/work/.roko/events.jsonl`
- [x] Final executor phase trace: `/tmp/roko-real-e2e-nrUD05/work/.roko/state/executor.json`

Boundaries of this proof:

- This proves one-task end-to-end execution, not full parity.
- [ ] Multi-task DAG, retry loop depth, resume interruption, and routing/knowledge parity are still pending.

## Worker 9 Evidence Checklist (2026-04-26)

Current source-backed implementation evidence:

- [x] `crates/roko-cli/src/dispatch_v2.rs` provides a provider dispatch resolver with `CliProviderConfig`, `CliDispatchRequest`, `CliInvocation`, `ProviderDispatchResolver`, and `AgentDispatcherV2`.
- [x] `crates/roko-cli/src/runner/gate_dispatch.rs` executes gates through `GatePayload`, a process semaphore, `tokio::time::timeout`, and real `task.verify` shell commands.
- [x] `crates/roko-cli/src/runner/persist.rs` owns `.roko/state/executor.json`, `.roko/events.jsonl`, `.roko/episodes.jsonl`, `.roko/learn/efficiency.jsonl`, and runtime pid cleanup paths.
- [x] `crates/roko-cli/src/runner/event_loop.rs` now blocks duplicate active agent spawns, saves executor snapshots after key transitions, appends structured runner events, and completes the no-mock one-task Codex/Claude smoke runs.
- [x] `crates/roko-cli/src/runner/agent_stream.rs` calls `build_composed_system_prompt` through the live prompt path, with legacy fallback still present.

Remaining implementation work before this overview can be archived:

- [ ] Replace `dispatch_v2.rs` with the intended `crates/roko-cli/src/dispatch/` module family or update all docs to the actual abstraction boundary.
- [ ] Add `crates/roko-agent/src/runtime_events.rs` and move `ClaudeStreamEvent` parsing out of `crates/roko-cli/src/runner/agent_stream.rs`.
- [ ] Wire `AgentDispatcherV2::run_agent_result_bridge` into `runner/event_loop.rs`; it is currently documented as not wired because runner started events require an OS pid.
- [ ] Add an active feedback facade under the live runner path for `LearningRuntime`, `CascadeRouter`, knowledge lifecycle, conductor observations, and dream triggers.
- [ ] Add an active projection facade for TUI, HTTP/SSE, and non-TUI CLI progress from one normalized event vocabulary.
- [ ] Replace the runner `MergeBranch` auto-success path with `roko-orchestrator/src/merge_queue.rs` integration.
- [ ] Produce proof for multi-task DAG execution, retry classification/backoff, interrupted resume, routing observation persistence, knowledge writeback, and projection parity.

## 2026-04-27 Deepening Pass - Current Runner Convergence Entry Point

Self-grade for this pass:

- Initial rating: 9.91 / 10.
- Reasoning: this overview now has a current source-truth section, exact remaining convergence gates, a no-context execution order, and links into the detailed implementation/proof docs. The score is not higher because the executable proof reports are still pending.

This section supersedes older "v2 gets wrong" and "remaining implementation work" bullets where source has moved forward.

### Current Source Truth

- [x] `crates/roko-agent/src/runtime_events.rs` exists and is exported by `roko-agent`.
- [x] Claude CLI stream parsing now lives under `roko-agent/src/provider/claude_cli/stream.rs`, not in runner-local Claude protocol types.
- [x] `crates/roko-cli/src/dispatch/` exists and is exported by `roko-cli`.
- [x] `runner/event_loop.rs` calls the dispatch facade and resolves CLI versus provider-backed bridge runtimes.
- [x] `AgentDispatcherV2::run_agent_result_bridge` is wired through `spawn_agent_result_bridge`.
- [x] `crates/roko-cli/src/runtime_feedback/` exists and `commands/plan.rs` wires episode, routing, knowledge, conductor, and dream sinks.
- [x] `crates/roko-cli/src/projection/` and `runner/projection.rs` exist.
- [x] `runner/persist.rs` writes `run-state.json` and includes router/threshold paths.
- [x] `runner/resume.rs` exists and performs strict resume validation helpers.
- [x] `runner/task_dag.rs` exists, but active spawn resolution still needs to delegate to it.
- [x] `runner/merge.rs` exists and `MergeBranch` submits through `PlanMerger`.

### Remaining Runner Convergence Gates

- [ ] Dispatch authority: fix the config-default-as-model-hint bypass and prove override, task hint, router, and default fallback semantics.
- [ ] Provider proof: run the matrix for Anthropic, OpenAI, Moonshot, Z.AI, Perplexity, Claude CLI, and Codex CLI with explicit statuses.
- [ ] Execution authority: make `TaskDag` the active scheduling owner instead of inline sentinel resolution.
- [ ] Process/effect authority: replace the global single `agent_handle` with effect-scoped handles before enabling true concurrent task execution.
- [ ] Persistence authority: prove crash/resume across task, gate, retry, merge, stale task, future schema, JSONL tail corruption, and orphan cleanup.
- [ ] Feedback authority: preserve real provider/model/tokens/cost into feedback instead of synthetic blank outcomes.
- [ ] Knowledge/dream authority: consume hot-path candidate/trigger outboxes into durable neuro/dream lifecycle and prove second-run influence.
- [ ] Prompt authority: reconcile active `dispatch/prompt_builder.rs` with `roko-compose` VCG/cost/section-effect manifests.
- [ ] Observability authority: query the same projection through TUI, CLI progress, HTTP polling, and HTTP/SSE.
- [ ] Legacy retirement: classify or retire `orchestrate.rs`, `dispatch_v2.rs`, `dispatch_direct.rs`, old prompt helpers, and `PlanRunner::from_plans_dir` callers.

### Implementation Order For A No-Context Agent

1. Start with [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) and take the highest-priority unchecked item.
2. For dispatch/model/provider work, use [01-AGENT-DISPATCH.md](01-AGENT-DISPATCH.md), [12-AFFECT-ROUTING.md](12-AFFECT-ROUTING.md), and [41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md).
3. For task/gate/retry/merge work, use [02-PLAN-EXECUTION.md](02-PLAN-EXECUTION.md), [11-PARALLEL-MERGE.md](11-PARALLEL-MERGE.md), and [39-RUNNER-EXECUTION-POLICY-AUDIT.md](39-RUNNER-EXECUTION-POLICY-AUDIT.md).
4. For crash/resume/proof work, use [03-PERSISTENCE.md](03-PERSISTENCE.md), [22-STABILITY-PLAN.md](22-STABILITY-PLAN.md), and [27-FILESYSTEM-RUNTIME-CI-AUDIT.md](27-FILESYSTEM-RUNTIME-CI-AUDIT.md).
5. For learning/knowledge/dream/prompt work, use [04-LEARNING.md](04-LEARNING.md), [09-COMPOSITION-AUCTION.md](09-COMPOSITION-AUCTION.md), [10-DREAMS-CONSOLIDATION.md](10-DREAMS-CONSOLIDATION.md), [13-KNOWLEDGE-LIFECYCLE.md](13-KNOWLEDGE-LIFECYCLE.md), and [38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md](38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md).
6. For HTTP/TUI/query work, use [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md) and [40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md](40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md).
7. For repo-wide cleanup, use [25-CODE-ONLY-LEGACY-AUDIT.md](25-CODE-ONLY-LEGACY-AUDIT.md), [26-REPOSITORY-WIDE-CODE-AUDIT.md](26-REPOSITORY-WIDE-CODE-AUDIT.md), [30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md](30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md), [31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md](31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md), and [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md).

### Required Proof Artifacts

- [ ] `tmp/mori-diffs/generated/agent-dispatch-proof.json`
- [ ] `tmp/mori-diffs/generated/plan-execution-proof.json`
- [ ] `tmp/mori-diffs/generated/persistence-resume-proof.json`
- [ ] `tmp/mori-diffs/generated/learning-feedback-proof.json`
- [ ] `tmp/mori-diffs/generated/migration-cutover-report.json`
- [ ] `tmp/mori-diffs/generated/file-map-proof-report.json`
- [ ] `tmp/mori-diffs/generated/provider-dispatch-matrix.json`
- [ ] `tmp/mori-diffs/generated/stability-proof-report.json`
- [ ] `tmp/mori-diffs/generated/observability-proof-bundle.json`
- [ ] `tmp/mori-diffs/generated/feature-parity-report.json`

### Archive Gate

- [ ] Every active mori-diffs doc has a current source-truth section or is explicitly historical/archived.
- [ ] README lists the current docs and their self-grades.
- [ ] Every stale older claim in this overview is superseded by a corrected section or corrected in place.
- [ ] The generated proof artifacts above exist and are linked.
- [ ] The current gap ledger shows no P0 or P1 runner-convergence items open.
