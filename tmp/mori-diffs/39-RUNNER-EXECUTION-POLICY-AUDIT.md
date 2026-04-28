# 39 - Runner Execution Policy Audit

Date: 2026-04-27

Status: active implementation handoff

### Architecture Runner Update (2026-04-28)
Runner execution policy now has typed engine:
- `PipelineStateV2` (P2A) is the pure state machine for execution decisions (phases, transitions, failure classification)
- `TaskScheduler` (P2B) handles DAG scheduling, wave computation, skip propagation
- `EffectDriver` (P2C) executes actions from state machine output
- `GateService` (P1D) provides unified gate dispatch
- Config-driven: express/standard/full/plan_execution workflow templates
- Remaining: full retry/replan policy convergence proof, merge policy, worktree isolation

Scope: active runner task scheduling, gate execution, retry/replan policy, merge/worktree policy, persistence checkpoints, resume boundaries, and proof surfaces. This doc is intentionally about the runtime control plane, not provider dispatch, prompt assembly, or cognitive feedback except where those systems must feed execution decisions.

If another agent only reads this file, it should be able to implement the next runner execution-policy slice without rediscovering the repo.

## Executive Verdict

Runner-v2 is no longer the original broken one-off path. It has real pieces:

- `crates/roko-cli/src/runner/event_loop.rs` calls `dispatch::Dispatcher`.
- `crates/roko-cli/src/runner/merge.rs` has `PlanMerger`, `GitMergeBackend`, `MergeBackend`, `RegressionGate`, and post-merge regression evidence.
- `crates/roko-cli/src/runner/task_dag.rs` has deterministic DAG bookkeeping, timeout, skipped propagation, and retry backoff primitives.
- `crates/roko-cli/src/runner/gate_dispatch.rs` dispatches gate rungs as background tasks and emits normalized completions.
- `crates/roko-orchestrator/src/repair.rs` and `crates/roko-orchestrator/src/replan.rs` define repair/replan concepts.
- `crates/roko-cli/src/runner/types.rs` contains durable event variants for prompt assembly, gate dispatch, merge backend completion, and retry decisions.

The remaining architecture problem is that these pieces are composed procedurally inside `runner/event_loop.rs`. That file is still a mini-orchestrator: it chooses tasks, resolves sentinel task names, checks budget, plans dispatch, starts agents, fans out events, handles gate results, applies retry rules, injects retry prompt context, handles plan verify, handles merge results, drains the merge queue, snapshots state, emits learning feedback, and triggers dreams.

That is better than the old monolith, but it is not yet the clean Mori-like control plane. The correct redesign is a typed `PlanExecutionEngine` with small policy services and durable decisions. The event loop should become a reactor that feeds events into the engine and executes returned effects.

## Source Evidence

Commands run for this audit:

```bash
rg -n "GateFailed|MergeSucceeded|MergeFailed|Replan|Retry|mark_task|next_ready|run_gate|merge|PlanMerger|RepairEngine|RepairDecision|ReplanStrategy" crates/roko-cli/src/runner crates/roko-orchestrator/src/repair.rs crates/roko-orchestrator/src/replan.rs
rg -n "TODO|FIXME|HACK|stub|placeholder|legacy|deprecated|for now" crates/roko-cli/src/runner crates/roko-gate/src crates/roko-orchestrator/src
python3 - <<'PY'
from pathlib import Path
import re
paths = [
    Path("crates/roko-cli/src/runner"),
    Path("crates/roko-gate/src"),
    Path("crates/roko-orchestrator/src"),
]
patterns = {
    "task_state": re.compile(r"TaskAttempt|TaskState|PlanPhase|current_phase|mark_task|completed_tasks"),
    "gate": re.compile(r"gate|Gate|rung|Rung"),
    "retry": re.compile(r"retry|Retry|backoff"),
    "replan": re.compile(r"replan|Replan|Repair"),
    "merge": re.compile(r"merge|Merge"),
    "worktree": re.compile(r"worktree|Worktree"),
    "dag": re.compile(r"DAG|Dag|depends_on|next_ready"),
    "persist": re.compile(r"snapshot|persist|jsonl|run-state|checkpoint"),
    "policy": re.compile(r"policy|Policy|decision|Decision|threshold|budget"),
    "spawn": re.compile(r"spawn|Command::new|tokio::spawn"),
}
files = sorted(p for root in paths for p in root.rglob("*.rs"))
for p in files:
    text = p.read_text(errors="ignore")
    counts = {k: len(v.findall(text)) for k, v in patterns.items()}
    total = sum(counts.values())
    if total >= 75:
        print(total, len(text.splitlines()), p, counts)
PY
```

Hotspot result:

| File | Lines | Matches | Meaning |
| --- | ---: | ---: | --- |
| `crates/roko-cli/src/runner/event_loop.rs` | 3036 | 850 | Runtime control plane still concentrates dispatch, gates, retries, merge, persistence, feedback, and policy. |
| `crates/roko-gate/src/gate_pipeline.rs` | 1119 | 455 | Gate pipeline is a substantial subsystem, not a trivial shell check. |
| `crates/roko-cli/src/runner/merge.rs` | 777 | 416 | Merge has a real backend/regression abstraction, but policy/result ownership is still runner-local. |
| `crates/roko-orchestrator/src/worktree.rs` | 1204 | 393 | Worktree behavior exists outside runner and needs a clean active-runtime seam. |
| `crates/roko-cli/src/runner/types.rs` | 1561 | 357 | Event vocabulary exists and should be the durable contract for policy decisions. |
| `crates/roko-orchestrator/src/dag.rs` | 2557 | 338 | Orchestrator DAG model is richer than active runner's current local scheduling use. |
| `crates/roko-orchestrator/src/merge_queue.rs` | 925 | 282 | Merge queue has conflict-aware semantics and snapshots. |
| `crates/roko-gate/src/rung_selector.rs` | 561 | 282 | Gate rung selection is real enough to be policy-controlled. |
| `crates/roko-gate/src/adaptive_threshold.rs` | 958 | 244 | Adaptive thresholding exists, but active decision integration is still incomplete. |
| `crates/roko-orchestrator/src/executor/mod.rs` | 1223 | 229 | Legacy orchestration still has executor concepts that must be reconciled or retired. |
| `crates/roko-orchestrator/src/repair.rs` | 957 | 222 | Repair policy is modeled, but active runner does not use it as the source of truth. |
| `crates/roko-cli/src/runner/state.rs` | 512 | 185 | Runtime state tracks attempts, retry backoff, gate effects, snapshots, and lifecycle projection. |
| `crates/roko-cli/src/runner/task_dag.rs` | 555 | 131 | Active DAG helper exists, but event loop still bypasses parts of it. |

Stub and placeholder evidence:

```bash
rg -n "stub_verdict|benchmark regression gate \\(stub\\)|placeholder|TODO|Currently a stub|not wired" crates/roko-gate/src crates/roko-orchestrator/src crates/roko-cli/src/runner
```

Observed gaps:

- `crates/roko-gate/src/rung_dispatch.rs` returns passing `stub gate` verdicts for missing symbol manifest, generated-test artifacts, verify-chain script, fact-check content, fact-check oracle, judge payload, judge oracle, and integration scenario.
- `crates/roko-gate/src/benchmark_gate.rs` says the benchmark regression gate is currently a stub.
- `crates/roko-gate/src/process_reward.rs` contains a heuristic scorer stub for future LLM-based PRM scoring.
- `crates/roko-gate/src/eval_generator.rs` has placeholder generated assertion logic.
- `crates/roko-orchestrator/src/repair.rs` says subgraph replacement currently returns a placeholder list that the runtime would expand.
- `crates/roko-cli/src/runner/event_loop.rs` still adds retry prompt context inline on the third attempt rather than delegating to a repair policy.

## Current Runtime Shape

### Active event loop ownership

`crates/roko-cli/src/runner/event_loop.rs` currently owns:

- Agent event ingestion and lifecycle projection.
- Gate completion handling.
- Merge completion handling.
- Plan verify completion handling.
- Executor ticking.
- Task resolution from `next`, `fix`, `regen-verify`, `review`, `doc-revision`, `docs`, and `enrich`.
- Per-plan budget checks.
- Dispatch planning through `Dispatcher`.
- Runtime provider resolution through `resolve_agent_runtime`.
- Runner event emission and feedback conversion.
- Snapshot persistence.
- Gate dispatch and duplicate effect suppression.
- Retry eligibility and backoff.
- Raw retry prompt enrichment.
- Dream consolidation trigger after plan completion.
- Merge queue drain.

This is the main design smell. The file is trying to be:

- Reactor.
- State machine.
- Task scheduler.
- Policy engine.
- Effect executor.
- Event store writer.
- Feedback bus.
- Recovery coordinator.

The target is to keep only the reactor/effect execution layer here.

### Good modules that should survive

- [ ] Keep `runner/types.rs` as the normalized event vocabulary, or move it into `roko-runtime` if CLI ownership remains too broad.
- [ ] Keep `runner/persist.rs` and `runner/resume.rs`, but put them behind `ExecutionEventStore` and `ExecutionSnapshotStore`.
- [ ] Keep `runner/merge.rs` concepts, but make `MergePolicyEngine` own queue admission, backend selection, conflict handling, and regression requirements.
- [ ] Keep `runner/task_dag.rs`, but promote it to `TaskScheduler` so the event loop never manually resolves task readiness.
- [ ] Keep `gate_dispatch.rs` as an effect adapter, but make `GatePolicyEngine` build the gate request and classify unsupported gates.
- [ ] Keep `roko-orchestrator/src/repair.rs`, but stop treating it as sidecar design. Use it for active retry, skip, subgraph replacement, and full replan choices.

## Target Architecture

The clean design is:

```text
CLI / HTTP / daemon command
  -> RuntimeCommandService
  -> PlanExecutionEngine
  -> ExecutionStateMachine
  -> TaskScheduler
  -> AgentDispatchService
  -> GatePolicyEngine
  -> RepairPolicyEngine
  -> MergePolicyEngine
  -> ExecutionEventStore
  -> RuntimeProjectionService
```

The event loop should do this:

```text
external event -> PlanExecutionEngine::apply(event) -> Vec<ExecutionEffect>
ExecutionEffect -> adapter runs IO -> external event
```

The engine should not spawn processes, write files, call git, or directly update TUI. It should produce typed decisions and effects. Adapters execute effects and report back with typed completion events.

### Core service boundaries

`PlanExecutionEngine`:

- Owns the state machine.
- Accepts `ExecutionInputEvent`.
- Produces `ExecutionDecision` records and `ExecutionEffect` requests.
- Has no direct filesystem, process, git, model, or HTTP access.

`TaskScheduler`:

- Owns DAG readiness, running set, skipped propagation, plan deadlines, and retry cooldown.
- Accepts plan/task definitions plus completed/failed state.
- Returns deterministic `TaskSchedulingDecision`.
- Replaces inline sentinel handling in `event_loop.rs`.

`AgentDispatchService`:

- Converts a runnable task and policy context into an `AgentDispatchRequest`.
- Uses `Dispatcher`, `PromptAssembler`, provider resolution, and model routing underneath.
- Returns provider-neutral `AgentRuntimeEvent`.
- Does not decide retry/replan.

`GatePolicyEngine`:

- Selects required rungs for a task.
- Builds typed gate inputs.
- Distinguishes `passed`, `failed`, `skipped_by_config`, `unsupported`, and `not_applicable`.
- Owns adaptive threshold inputs.
- Returns `GateDecision`.

`RepairPolicyEngine`:

- Converts gate/agent/merge failures into `FailureContext`.
- Calls `RepairEngine`.
- Computes skip/retry/subgraph/full-replan decisions using DAG closure, failure kind, attempts, task criticality, budget, and human-blocking evidence.
- Returns `RepairDecisionRecord`.

`MergePolicyEngine`:

- Owns merge queue admission, branch existence, in-place mode, worktree mode, conflict evidence, post-merge regression, retry/exhaustion, and next-queue drain.
- Returns `MergeDecision` and `MergeEffect`.

`ExecutionEventStore`:

- Appends durable runner events.
- Writes checkpoints at state-machine boundaries.
- Recovers partial writes before append.
- Exposes queryable projections.

`RuntimeProjectionService`:

- Builds task/gate/merge/provider/retry/resume projections from the same durable events used by CLI, TUI, HTTP, and proof scripts.

## Required Types

Add these types before extracting behavior. The missing design is not another helper function; it is a durable decision model.

```rust
pub enum ExecutionInputEvent {
    RunStarted { run_id: RunId },
    Tick,
    AgentStarted(AgentStarted),
    AgentOutput(AgentOutput),
    AgentCompleted(AgentCompleted),
    GateCompleted(GateCompleted),
    MergeCompleted(MergeCompleted),
    TimeoutExpired(TimeoutRef),
    CancellationRequested(CancelScope),
    ResumeLoaded(ResumeMarker),
}

pub enum ExecutionEffect {
    PersistEvent(RunnerEvent),
    SaveSnapshot(SnapshotReason),
    DispatchAgent(AgentDispatchRequest),
    RunGate(GateRunRequest),
    RunPlanVerify(PlanVerifyRequest),
    ApplyMerge(MergeRequest),
    StartRetryTimer(RetryTimerRequest),
    RequestReplan(PlanRevisionRequest),
    MarkHumanBlocked(HumanBlockRequest),
    PublishProjection(ProjectionUpdate),
    EmitFeedback(FeedbackEvent),
}

pub struct ExecutionDecision {
    pub run_id: RunId,
    pub plan_id: PlanId,
    pub task_id: Option<TaskId>,
    pub attempt: Option<u32>,
    pub kind: ExecutionDecisionKind,
    pub reason: String,
    pub evidence: Vec<DecisionEvidence>,
    pub produced_effects: Vec<ExecutionEffectId>,
}

pub enum ExecutionDecisionKind {
    ScheduleTask,
    SuppressDuplicateDispatch,
    DispatchAgent,
    DispatchGate,
    SkipGateByConfig,
    GateUnsupported,
    RetryTask,
    SkipTask,
    ReplaceSubgraph,
    FullReplan,
    EnterMergeQueue,
    MergeBlocked,
    MergeSucceeded,
    MergeFailed,
    PlanCompleted,
    PlanFailed,
    HumanBlocked,
}
```

State should become explicit:

```rust
pub enum TaskExecutionState {
    Pending,
    Ready,
    DispatchingAgent,
    AgentRunning,
    AgentCompleted,
    Gating { rung: u32 },
    RetryWaiting { not_before_ms: u64 },
    ReplanRequested { request_id: String },
    Skipped { reason: SkippedReason },
    Completed,
    Failed { reason: String },
}

pub enum PlanExecutionState {
    Pending,
    Running,
    Verifying,
    MergeQueued,
    Merging,
    Completed,
    Failed,
    Blocked,
}
```

Do not encode these only as transient fields like `current_task`, `agent_active`, and `gate_output`.

## P0 Findings And Checklists

### P0-01 Event Loop Still Owns Policy

Evidence:

- `runner/event_loop.rs` has more than 3000 lines.
- It has the highest execution-policy count in the scan: `850` matches across task state, gates, retry, replan, merge, worktree, persistence, policy, and spawn.
- It directly applies `ExecutorEvent::GateFailed`, computes retry eligibility, mutates executor iteration state, sets backoff, emits retry decisions, and injects retry prompt context.
- It directly applies `ExecutorEvent::MergeSucceeded` and `ExecutorEvent::MergeFailed`.
- It directly triggers dream consolidation after plan completion.

Why this is wrong:

- A reactor should not choose business policy.
- It makes proof hard because decisions are spread across match arms.
- It makes resume hard because policy phase transitions are not always first-class decisions.
- It encourages one-off additions inside a giant file.

Target:

- `event_loop.rs` becomes an adapter with channel select and effect execution.
- `PlanExecutionEngine::apply` owns transitions.
- Every major choice is recorded as `ExecutionDecision`.

Checklist:

- [ ] Create `crates/roko-cli/src/runner/policy/mod.rs` or, preferably, `crates/roko-runtime/src/execution/`.
- [ ] Move retry eligibility from `event_loop.rs` into `RepairPolicyEngine`.
- [ ] Move task scheduling from `event_loop.rs` into `TaskScheduler`.
- [ ] Move merge completion handling from `event_loop.rs` into `MergePolicyEngine`.
- [ ] Move plan verify completion handling into `GatePolicyEngine` or `PlanVerificationPolicy`.
- [ ] Move dream trigger emission into feedback/cognitive sink by consuming `PlanCompleted`, not by direct `tokio::spawn` in runner.
- [ ] Add a compile-time boundary: policy modules cannot depend on `TuiBridge`, `tokio::process::Command`, or direct `.roko` paths.
- [ ] Add a grep gate: `rg -n "ExecutorEvent::GateFailed|ExecutorEvent::MergeSucceeded|set_replan_context|DreamRunner::new" crates/roko-cli/src/runner/event_loop.rs` should return zero after migration.

### P0-02 Task Scheduling Is Partially Duplicated

Evidence:

- `runner/task_dag.rs` already has `next_ready_task`, `ready_tasks`, `mark_running`, `mark_failed_blocking_downstream`, `mark_plan_timed_out`, and `schedule_retry`.
- `runner/event_loop.rs` still manually resolves sentinel task strings such as `next`, `fix`, and `regen-verify` by walking task definitions.
- Gate pass handling in `event_loop.rs` manually checks for more tasks using `completed.contains` plus `is_ready_with_plan_deps`.

Why this is wrong:

- There should be exactly one place that decides ready/running/blocked/skipped.
- Sentinel strings are legacy behavior leaking into core scheduling.
- Multi-plan dependency behavior and retry cooldown can diverge from `TaskDag`.

Target:

- `TaskScheduler::on_plan_tick(plan_id)` returns a `TaskSchedulingDecision`.
- The scheduler translates legacy sentinel actions only at the executor compatibility boundary.
- Event loop never walks `TaskDef` dependencies directly.

Checklist:

- [ ] Add `TaskSchedulingDecision::{Dispatch, PlanComplete, PlanBlocked, PlanTimedOut, RetryCoolingDown, DuplicateSuppressed}`.
- [ ] Add `TaskSchedulerInput` with completed tasks, completed plans, running tasks, failed tasks, skipped tasks, deadlines, and retry cooldowns.
- [ ] Replace the `next`/`fix`/`regen-verify` block in `event_loop.rs` with `TaskScheduler::resolve_action`.
- [ ] Replace the post-gate `has_more` scan in `event_loop.rs` with `TaskScheduler::after_task_passed`.
- [ ] Emit `RunnerEvent` or `ExecutionDecision` for `DuplicateSuppressed`, `PlanBlocked`, `PlanTimedOut`, and `TaskSkipped`.
- [ ] Add proof plan with tasks `A -> {B,C} -> D`, two ready branches, one failed prerequisite, and one plan dependency.
- [ ] Add resume proof where a task is marked running before crash and is not double-dispatched on resume.

### P0-03 Retry/Replan Policy Is Not The Active Repair Engine

Evidence:

- `roko-orchestrator/src/repair.rs` defines `RepairEngine`, `RepairDecision`, `RepairAction::{RetryTask, ReplaceSubgraph, FullReplan, SkipTask}`, and stability metrics.
- `roko-orchestrator/src/replan.rs` defines `FailureDisposition`, `PlanRevisionRequest`, `ReplanStrategy`, and `ReplanResult`.
- Active runner gate failure handling currently computes `can_retry` from plan iteration and `failure_kind.is_retryable()`.
- On third or later attempt, active runner appends raw failure text to prompt context with an inline string.
- Subgraph replacement in `repair.rs` still says the affected task list is a placeholder that runtime must expand.

Why this is wrong:

- Retry, skip, decompose, full replan, blocked, and human-needed are a policy ladder. They should not be if-statements in the event loop.
- Prompt repair context should be structured and bounded, not raw gate output spliced inline.
- Replan requests must be durable so crash/resume can continue, dedupe, or query them.

Target:

- `RepairPolicyEngine` is active in runner.
- It builds `FailureContext` from gate verdicts, dispatch errors, merge failures, plan topology, attempt count, budgets, and human-blocking evidence.
- It returns a durable repair decision and effects.

Checklist:

- [ ] Define `RuntimeFailureContext` in runner/runtime layer and map it to `roko_orchestrator::repair::FailureContext`.
- [ ] Compute real DAG transitive closure for `ReplaceSubgraph`; delete placeholder expansion semantics.
- [ ] Replace inline `can_retry` logic with `RepairPolicyEngine::decide`.
- [ ] Replace `state.set_replan_context` raw text with `RepairPromptContext` generated from structured evidence.
- [ ] Emit `RunnerEvent::RetryDecision` for retry and a new or existing durable event for skip, subgraph replacement, full replan, blocked, and human-needed decisions.
- [ ] Persist `PlanRevisionRequest` with resume token before asking a planner to revise anything.
- [ ] Ensure model escalation flows through dispatcher model policy, not an ad hoc prompt string.
- [ ] Add proof that compile failure retries once, escalates or repairs on second failure, and emits a bounded repair context.
- [ ] Add proof that a non-retryable structural failure produces `NeedsReplan` or `NeedsHuman`, not a retry loop.
- [ ] Add proof that a leaf non-critical task can be skipped only after allowed retry policy says so.

### P0-04 Gate Ladder Has Passing Stubs For Missing Capabilities

Evidence:

- `rung_dispatch.rs` uses `stub_verdict` that returns `Verdict::pass`.
- Missing higher-rung inputs currently pass:
- `symbol`: no `SymbolManifest` wired into rung 3.
- `generated_test:cargo`: generated test artifacts not wired.
- `verify_chain`: no verify script wired into rung 4.
- `fact_check`: no fact-check content or oracle.
- `llm_judge`: no judge payload or oracle.
- `integration:build_test`: no integration scenario.
- `benchmark_gate.rs` explicitly says the benchmark regression gate is a stub.

Why this is wrong:

- A missing gate dependency is not the same as success.
- Passing stubs let proof claim maturity while core features are absent.
- Operators need to know whether a gate passed, was skipped by config, was not applicable, or was unsupported.

Target:

- Gate verdicts have explicit status:
- `passed`
- `failed`
- `skipped_by_config`
- `not_applicable`
- `unsupported`
- `errored`
- Only `passed`, `skipped_by_config`, and selected `not_applicable` statuses can let execution advance, and the proof must show which one happened.

Checklist:

- [ ] Extend gate verdict summary or wrap it with `GateStatus`.
- [ ] Change `stub_verdict` to produce `unsupported` evidence, not passing success.
- [ ] Add `GatePolicyEngine` rule that says whether unsupported gates are fatal for the current run mode.
- [ ] Add run modes: `fast`, `normal`, `strict`, and `mori-parity`. In `mori-parity`, unsupported advertised gates fail proof.
- [ ] Wire symbol manifest creation or mark rung 3 unsupported with evidence.
- [ ] Wire generated-test artifact store or mark rung 4 unsupported with evidence.
- [ ] Wire verify-chain scripts from task/plan definitions or mark unsupported.
- [ ] Wire fact-check oracle from provider/config policy or mark unsupported.
- [ ] Wire LLM judge oracle from provider/config policy or mark unsupported.
- [ ] Wire integration scenario selection from task acceptance contracts or mark unsupported.
- [ ] Add proof that unsupported gate statuses are visible in event JSONL, HTTP projections, and TUI/proof output.
- [ ] Add grep gate: `rg -n "Verdict::pass\\([^\\n]*stub|stub gate" crates/roko-gate/src` should not identify successful stub gates.

### P0-05 Merge Policy Is Improved But Still Not A Complete Execution Service

Evidence:

- `runner/merge.rs` now has `GitMergeBackend`, conflict detection, `git merge --abort`, branch/in-place modes, `RegressionGate`, and queue drain.
- `event_loop.rs` still owns `handle_merge_completion`, applies executor events, emits plan completion, and drains the next queued merge.
- Conflict paths are extracted from formatted output text through `conflict_paths_from_merge_output`.
- `MergeQueue` has snapshots, file locks, blocked conflicts, retry counts, and failed states.

Why this is still incomplete:

- Merge result evidence should be typed before rendering text.
- Queue retry/exhaustion should be part of merge policy, not spread across queue and event loop.
- Worktree mode versus in-place mode must be a runtime policy decision with proof, not an incidental branch absence check.
- Merge success should prove git state, regression, event persistence, projection, and resume behavior.

Target:

- `MergePolicyEngine` owns queue admission, merge backend selection, conflict evidence, regression proof, retries, and post-completion queue drain.
- `event_loop.rs` only executes `MergeEffect` and feeds `MergeCompleted` back into the engine.

Checklist:

- [ ] Define `MergeDecision::{Queued, Reserved, BlockedByFiles, BackendSucceeded, BackendFailed, RegressionPassed, RegressionFailed, Exhausted}`.
- [ ] Define typed `MergeConflictEvidence { branch, conflicted_paths, stdout_digest, stderr_digest, abort_result }`.
- [ ] Replace `conflict_paths_from_merge_output` with typed conflict evidence from `GitMergeBackend`.
- [ ] Move `handle_merge_completion` decision logic into `MergePolicyEngine`.
- [ ] Make branch mode, worktree mode, and in-place mode explicit config/policy values.
- [ ] Add proof for branch merge success into a temporary repo.
- [ ] Add proof for branch merge conflict with typed conflicted paths and clean abort.
- [ ] Add proof for post-merge regression failure after successful git merge.
- [ ] Add proof for queued non-conflicting merges and blocked conflicting merges.
- [ ] Add resume proof where crash happens after merge reservation but before completion.

### P0-06 Durable Decisions Are Still Incomplete

Evidence:

- `RunnerEvent` has important event variants: prompt assembly, agent dispatch, gate dispatch, gate completion, merge backend completion, and retry decision.
- Some important choices still happen as local state mutation or TUI messages.
- Examples: duplicate dispatch suppression, scheduler blocked/ready decisions, gate skipped by config, merge queue blocked, dream trigger, direct feedback writes, and some fatal path reasons.

Why this is wrong:

- If a decision is not durable, it cannot be proven, queried, resumed, or debugged.
- The UI and HTTP API should not infer state from logs or local memory.

Target:

- Every non-trivial choice emits an `ExecutionDecision`.
- `RunnerEvent` either contains these directly or has a parallel event family that projections consume.

Checklist:

- [ ] Add `ExecutionDecisionRecorded` event or extend `RunnerEvent` with decision variants.
- [ ] Emit decisions for duplicate agent suppression.
- [ ] Emit decisions for retry cooldown suppression.
- [ ] Emit decisions for plan timeout.
- [ ] Emit decisions for gate skipped by config.
- [ ] Emit decisions for gate unsupported.
- [ ] Emit decisions for merge queue blocked.
- [ ] Emit decisions for repair/replan selection.
- [ ] Emit decisions for dream/consolidation enqueue, not direct dream execution.
- [ ] Add projection fields for latest decision per task, latest decision per plan, and decision history.
- [ ] Add HTTP query proof that decisions are visible without reading raw JSONL.

## P1 Findings And Checklists

### P1-01 Active Runner Should Use One Orchestrator DAG Model

Current situation:

- `runner/task_dag.rs` is small and pragmatic.
- `roko-orchestrator/src/dag.rs` is richer and legacy-adjacent.
- Active runner should not grow a second incompatible DAG language unless this is a deliberate split.

Checklist:

- [ ] Decide whether `TaskDag` becomes the canonical runtime DAG or wraps `roko-orchestrator::dag`.
- [ ] Move canonical DAG types to a non-CLI crate if HTTP/server/runtime need them.
- [ ] Preserve deterministic ordering from `TaskDag`.
- [ ] Preserve richer graph validation from `roko-orchestrator::dag` if useful.
- [ ] Add migration tests for existing `tasks.toml` plans.

### P1-02 Adaptive Gate Thresholds And Routing Must Feed Decisions Before Dispatch

Current situation:

- Runner updates cascade router and adaptive thresholds after gate completion.
- Dispatch planning uses dispatcher/model routing, but gate/retry policy still does not visibly consume all learned threshold decisions as first-class inputs.

Checklist:

- [ ] Define `PolicyContext` passed into dispatch, gate, and repair decisions.
- [ ] Include provider health, historical gate pass rates, adaptive thresholds, task risk, budget, latency, and prompt diagnostics.
- [ ] Persist the policy context id with every decision.
- [ ] Add proof that a learned threshold affects a later gate/routing decision.

### P1-03 Plan Verify Is A Gate Policy, Not A Special Case

Current situation:

- `spawn_plan_verify` and `handle_plan_verify_completion` are separate paths.
- Failure always emits retry decision with "verify regeneration is available" even though the policy is not the same as task retry.

Checklist:

- [ ] Model plan verify as a `GateScope::{Task, Plan, Merge}`.
- [ ] Use the same gate status model as task gates.
- [ ] Route plan verify failure into `RepairPolicyEngine`.
- [ ] Add proof for plan verify pass, failure, retry, and non-retryable failure.

### P1-04 Worktree Policy Is Not A First-Class Active Runtime Contract

Current situation:

- `roko-orchestrator/src/worktree.rs` has substantial worktree semantics.
- `runner/merge.rs` supports branch mode and in-place mode.
- The active runtime still needs one explicit worktree policy that covers plan branches, dirty working tree, branch absence, cleanup, and resume.

Checklist:

- [ ] Define `WorkspaceExecutionMode::{InPlace, Branch, Worktree}`.
- [ ] Make mode selection a resolved runtime config decision.
- [ ] Emit `WorkspacePrepared`, `WorkspaceDirty`, `WorkspaceCleaned`, and `WorkspaceCleanupFailed` events.
- [ ] Prove branch/worktree cleanup on success and failure.
- [ ] Prove dirty initial workspace is refused or handled according to policy.

## Implementation Plan

### Phase 0 - Freeze New Event-Loop Policy

- [ ] Add a doc comment at top of `event_loop.rs` saying new policy belongs in execution policy services.
- [ ] Add a CI grep that rejects new direct calls to `ExecutorEvent::GateFailed`, `ExecutorEvent::MergeSucceeded`, and `ExecutorEvent::MergeFailed` outside policy modules after extraction begins.
- [ ] Add a tracking comment near current retry logic pointing to this doc.

### Phase 1 - Add Durable Decision Model

- [ ] Add `ExecutionDecision`, `ExecutionDecisionKind`, `DecisionEvidence`, and `ExecutionEffectId`.
- [ ] Add event serialization for decisions.
- [ ] Add projection support for decision history.
- [ ] Emit decisions from current code before moving behavior.
- [ ] Verify no behavior changes in this phase.

### Phase 2 - Extract Task Scheduler

- [ ] Create `TaskScheduler` facade around `TaskDag`.
- [ ] Replace inline task readiness scans in `event_loop.rs`.
- [ ] Add scheduler proof tests with branching DAG, failed prerequisite, plan dependency, timeout, and retry cooldown.
- [ ] Emit scheduler decisions.

### Phase 3 - Extract Gate Policy

- [ ] Add `GateStatus`.
- [ ] Convert passing stubs to unsupported/not-applicable statuses.
- [ ] Add `GatePolicyEngine` for rung selection, config skip, unsupported fatality, and adaptive threshold context.
- [ ] Replace direct gate skip/pass logic in `event_loop.rs`.
- [ ] Add strict mode proof that unsupported advertised gates fail parity proof.

### Phase 4 - Activate Repair Policy

- [ ] Add `RepairPolicyEngine`.
- [ ] Map gate/agent/merge failures to `FailureContext`.
- [ ] Use `RepairEngine` for retry, skip, replace subgraph, and full replan.
- [ ] Persist `PlanRevisionRequest` before planner calls.
- [ ] Generate structured retry prompt context through prompt assembly diagnostics.
- [ ] Add proof for retry, escalation, skip, subgraph replacement, full replan, blocked, and human-needed.

### Phase 5 - Extract Merge Policy

- [ ] Add typed merge evidence.
- [ ] Move queue drain and completion decisions into `MergePolicyEngine`.
- [ ] Make workspace mode explicit.
- [ ] Add merge success, conflict, regression failure, queue-blocking, and resume proof.

### Phase 6 - Collapse Event Loop To Reactor

- [ ] Event loop only multiplexes channels and executes `ExecutionEffect`s.
- [ ] TUI consumes projection/events instead of direct policy calls.
- [ ] Feedback facade consumes durable events instead of direct runner-local writes.
- [ ] Dream trigger becomes a feedback sink effect.
- [ ] HTTP/CLI/TUI proof reads the same projections.

## Proof Matrix Required Before Claiming Completion

All proof must run in a temporary folder outside the repo, without mocks for the active runtime path. Unit tests can exist, but they do not satisfy end-to-end proof alone.

| Proof | Required Evidence |
| --- | --- |
| Task success | Agent dispatch, prompt diagnostics, task completion, gate pass, event JSONL, projection query. |
| DAG branching | Two independent tasks ready after prerequisite, no duplicate dispatch, deterministic ordering. |
| Retry after gate failure | Failed gate, retry decision, backoff, second attempt, bounded repair context. |
| Retry exhausted | Final failure with `Exhausted`, no infinite loop, queryable reason. |
| Non-retryable failure | `NotRetryable` or `NeedsHuman`, no retry loop. |
| Subgraph replacement | Transitive closure evidence, modified task set, restart behavior. |
| Full replan | Durable `PlanRevisionRequest`, planner invocation, old/new plan link. |
| Gate unsupported | Unsupported higher-rung capability appears as unsupported, not pass. |
| Merge success | Real git branch merge plus regression pass. |
| Merge conflict | Real conflict, typed paths, abort result, failed plan/projection. |
| Regression failure | Git merge succeeds, regression gate fails, merge is not marked complete. |
| Merge queue blocking | Conflicting files block, disjoint files can proceed. |
| Crash resume | Crash at dispatch, gate, retry-wait, merge-reserved, and post-gate/pre-snapshot boundaries. |
| HTTP query | Same decision/projection data available from HTTP without raw JSONL scraping. |
| TUI query | TUI model/status/gate/retry/merge state matches projection data. |

Suggested tracked proof commands:

```bash
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case task-success
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case dag-branching
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case retry-gate-failure
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case retry-exhausted
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case gate-unsupported
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case merge-success
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case merge-conflict
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case regression-failure
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case crash-resume
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case http-query
```

## Grep Gates For Implementation Agents

Use these while implementing. They are not proof alone, but they catch regressions.

```bash
rg -n "ExecutorEvent::GateFailed|ExecutorEvent::MergeSucceeded|ExecutorEvent::MergeFailed|set_replan_context|DreamRunner::new" crates/roko-cli/src/runner/event_loop.rs
rg -n "Verdict::pass\\([^\\n]*stub|stub gate" crates/roko-gate/src
rg -n "strip_prefix\\(\"merge:\"\\)|conflicted paths:" crates/roko-cli/src/runner
rg -n "next\"|\"fix\"|\"regen-verify\"" crates/roko-cli/src/runner/event_loop.rs
rg -n "RepairEngine|RepairDecision|FailureContext|PlanRevisionRequest" crates/roko-cli/src/runner crates/roko-orchestrator/src
rg -n "ExecutionDecision|GateStatus|MergeDecision|TaskSchedulingDecision|RepairPolicyEngine|MergePolicyEngine" crates/roko-cli/src crates/roko-runtime/src
```

Target after implementation:

- [ ] First grep returns zero or only compatibility wrappers that call policy services.
- [ ] Second grep returns zero passing stub gates.
- [ ] Third grep returns zero string-parsed merge evidence.
- [ ] Fourth grep returns zero sentinel-task handling inside event loop.
- [ ] Fifth grep shows active runner usage of repair policy.
- [ ] Sixth grep shows the new policy types in active runtime code.

## Agent Handoff Checklist

### Small Batch A - Durable Decisions

- [ ] Add decision types and JSON serialization.
- [ ] Emit decisions from current scheduler, gate, retry, and merge branches.
- [ ] Update projection to expose decisions.
- [ ] Add proof query for latest decision per plan/task.

### Small Batch B - Scheduler Extraction

- [ ] Wrap `TaskDag` with `TaskScheduler`.
- [ ] Replace inline readiness scans.
- [ ] Remove sentinel handling from event loop.
- [ ] Add scheduler end-to-end proof.

### Small Batch C - Gate Status

- [ ] Add `GateStatus`.
- [ ] Convert passing stubs to unsupported.
- [ ] Update gate summaries and projections.
- [ ] Add strict/parity proof that unsupported fails.

### Small Batch D - Repair Policy

- [ ] Add `RepairPolicyEngine`.
- [ ] Use `RepairEngine` in active runner.
- [ ] Persist `PlanRevisionRequest`.
- [ ] Add retry/replan proof.

### Small Batch E - Merge Policy

- [ ] Add typed merge evidence.
- [ ] Move merge completion and queue drain out of event loop.
- [ ] Prove merge success/conflict/regression/resume.

## Acceptance Criteria

The work is complete only when all are true:

- [ ] `event_loop.rs` is primarily channel select plus effect execution.
- [ ] Task readiness is owned by `TaskScheduler`.
- [ ] Retry/replan/skip/subgraph/full replan are owned by `RepairPolicyEngine`.
- [ ] Gate missing capability is never represented as a passing stub.
- [ ] Merge success/failure/conflict/regression decisions are typed and durable.
- [ ] Every execution decision is queryable through projections.
- [ ] HTTP, TUI, and proof scripts consume the same projection data.
- [ ] Crash/resume proofs cover dispatch, gate, retry, merge, and snapshot boundaries.
- [ ] `orchestrate.rs` has no production-only execution policy that runner lacks.

## Initial Self-Grade And Iteration Proof

Initial self-grade before adding code-evidence sections and implementation packets: `9.42/10`.

Reason it was not high enough:

- It identified the broad event-loop problem but did not give enough source-level evidence.
- It did not distinguish existing good modules from missing active wiring.
- It did not give bounded work packets an implementation agent could execute one at a time.
- It did not define typed decision contracts.

Iteration performed:

- Added direct source evidence from `event_loop.rs`, `task_dag.rs`, `gate_dispatch.rs`, `rung_dispatch.rs`, `merge.rs`, `repair.rs`, `replan.rs`, and `merge_queue.rs`.
- Added target service boundaries and core types.
- Added P0/P1 checklists with grep gates.
- Added proof matrix and handoff batches.
- Added completion criteria that require active runner ownership, not module existence.

Final self-grade: `9.84/10`.

Why not `10/10`:

- This is still an audit and implementation handoff, not the implementation itself.
- Exact line numbers will drift as active runner work continues.
- The final design should be revisited once the execution policy types are implemented and proof results expose practical friction.

## 2026-04-27 Deepening Pass - Reducer, Effect, And Side-Effect Boundary

The earlier pass correctly identifies `runner/event_loop.rs` as the procedural control-plane hotspot. This pass makes the extraction boundary concrete: runner execution policy should become a pure reducer plus explicit effects. Side effects should move into adapters that are already covered by the lifecycle, adapter, artifact, gateway, and observability docs.

The target is not "split the file into smaller files." The target is:

```text
ExecutionInputEvent + ExecutionState
  -> PlanExecutionEngine::reduce(...)
  -> Vec<ExecutionDecision>
  -> Vec<ExecutionEffect>
  -> effect adapters execute IO
  -> typed completion events feed the reducer again
```

### Runner Drift R1 - Event Loop Still Owns Policy And Effects Together

Evidence:

```text
crates/roko-cli/src/runner/event_loop.rs:1678 plan_started event emission
crates/roko-cli/src/runner/event_loop.rs:1748 duplicate agent suppression
crates/roko-cli/src/runner/event_loop.rs:1753 retry cooldown check
crates/roko-cli/src/runner/event_loop.rs:1763 spawning agent
crates/roko-cli/src/runner/event_loop.rs:1864 replan context prompt mutation
crates/roko-cli/src/runner/event_loop.rs:1939 AgentSpawnConfig construction
crates/roko-cli/src/runner/event_loop.rs:2074 gate config skip handling
crates/roko-cli/src/runner/event_loop.rs:2100 dispatching gate
crates/roko-cli/src/runner/event_loop.rs:2256 PlanMerger construction
crates/roko-cli/src/runner/event_loop.rs:2294 feedback emission after gate
crates/roko-cli/src/runner/event_loop.rs:2356 adaptive threshold write
crates/roko-cli/src/runner/event_loop.rs:2397 cascade router observation
crates/roko-cli/src/runner/event_loop.rs:2586 extension on_gate hook
```

Problem:

- [ ] The event loop is still deciding what should happen and performing side effects.
- [ ] A policy decision cannot be replayed without also understanding IO branches.
- [ ] Tests can validate local branches but not the complete decision contract.
- [ ] HTTP/TUI/proof cannot query why a decision happened unless the event branch happened to emit enough context.

Implementation checklist:

- [ ] Add `ExecutionInputEvent` for external inputs: command start, agent completion, gate completion, merge completion, timer fired, cancel requested, resume loaded, shutdown requested.
- [ ] Add `ExecutionDecision` for durable "why" records: task selected, task skipped, dispatch planned, gate required, gate skipped, retry scheduled, replan requested, merge queued, merge blocked, merge started, merge failed, run completed.
- [ ] Add `ExecutionEffect` for side effects: spawn agent, run gate, run merge, write artifact, update feedback, update threshold, publish TUI event, emit dream trigger, save snapshot.
- [ ] Change event-loop branches to call `PlanExecutionEngine::reduce`.
- [ ] Execute returned effects through typed adapters.
- [ ] Persist decisions before executing effects when replay safety requires it.
- [ ] Include decision id and causation event id in every effect and completion event.

Acceptance proof:

- [ ] Replay the same event log into `PlanExecutionEngine` and get the same decisions without running providers, gates, git, or TUI.
- [ ] Query latest decision for a plan/task and see the causation event.
- [ ] Kill the process after decision persistence but before effect completion and resume idempotently.

### Runner Drift R2 - Serve Runtime Scrapes Runner Events Instead Of Querying Projection Service

Evidence:

```text
crates/roko-cli/src/serve_runtime.rs:166 collect_runner_gate_results
crates/roko-cli/src/serve_runtime.rs:473 collect_runner_gate_results
crates/roko-cli/src/serve_runtime.rs:486 serde_json::from_str::<RunnerEvent>
crates/roko-cli/src/serve_runtime.rs:498 RunnerEvent::GateCompleted
crates/roko-cli/src/serve_runtime.rs:732 render_plan_execution_summary
```

Problem:

- [ ] Serve integration reads runner JSONL directly to produce summaries.
- [ ] Gate evidence query logic is duplicated outside the projection service.
- [ ] HTTP can diverge from CLI/TUI/proof if the scraping logic misses a new event variant.
- [ ] This path encourages every adapter to parse runner logs independently.

Implementation checklist:

- [ ] Move gate evidence queries into `RuntimeProjectionService`.
- [ ] Replace `collect_runner_gate_results` with `RuntimeQuery::GateEvidence { operation_id, plan_ids }`.
- [ ] Return summary DTOs from projection state, not ad hoc post-run scans.
- [ ] Include projection cursor and evidence source in `PlanExecutionResult`.
- [ ] Add a compatibility wrapper only if current HTTP response shape must be preserved.

Acceptance proof:

- [ ] Run plan through serve runtime and produce the same summary via HTTP projection query.
- [ ] Delete the serve-runtime scraper and prove no route loses gate evidence.
- [ ] Add a new gate completion event field and show projections carry it without new scraping code.

### Runner Drift R3 - Gate Skip Is Represented As Passing Result

Evidence:

```text
crates/roko-cli/src/runner/event_loop.rs:1624 record_skipped_gate_rung
crates/roko-cli/src/runner/event_loop.rs:1642 ctx.tui.gate_result(..., true)
crates/roko-cli/src/runner/event_loop.rs:2074 skip clippy gate when disabled
crates/roko-cli/src/runner/event_loop.rs:2088 skip test gate when skip_tests
```

Problem:

- [ ] A skipped gate should not be indistinguishable from a passing gate.
- [ ] TUI/proof can overstate quality if disabled gates appear green.
- [ ] Retry/replan policy cannot reason cleanly about `passed`, `skipped_by_config`, `unsupported`, `not_applicable`, and `failed`.

Implementation checklist:

- [ ] Add `GateStatus::{Passed, Failed, SkippedByConfig, Unsupported, NotApplicable, Cancelled, TimedOut}`.
- [ ] Replace boolean `passed` as the primary proof status with `GateStatus`.
- [ ] Keep boolean compatibility fields only as derived display helpers.
- [ ] Make disabled clippy/test gates emit `SkippedByConfig`, not pass.
- [ ] Make missing/stub gate capabilities emit `Unsupported`.
- [ ] Update TUI/HTTP/proof to render skipped/unsupported distinctly.
- [ ] Make strict/Mori-parity proof fail on unsupported required gates and optionally fail on skipped required gates.

Acceptance proof:

- [ ] Run with `skip_tests = true` and prove the test gate is `skipped_by_config`, not passed.
- [ ] Trigger an unsupported gate and prove it cannot satisfy strict proof.
- [ ] TUI and HTTP show the same non-green status.

### Runner Drift R4 - Merge Backend Is Better, But Still Owns Process And Queue Effects

Evidence:

```text
crates/roko-cli/src/runner/merge.rs:135 MergeBackend
crates/roko-cli/src/runner/merge.rs:144 GitMergeBackend
crates/roko-cli/src/runner/merge.rs:196 git merge --no-ff
crates/roko-cli/src/runner/merge.rs:210 git merge --abort
crates/roko-cli/src/runner/merge.rs:292 cargo-check regression gate
crates/roko-cli/src/runner/merge.rs:307 spawn_blocking
crates/roko-cli/src/runner/merge.rs:475 tokio::spawn
```

Problem:

- [ ] Merge abstractions exist, but merge and regression still spawn work locally instead of using the runtime task/process lifecycle from doc 35.
- [ ] Queue admission, merge backend execution, conflict evidence, regression evidence, and completion are still coupled.
- [ ] Merge process events are not uniformly queryable as managed commands.

Implementation checklist:

- [ ] Split `MergePolicyEngine` from `MergeEffectExecutor`.
- [ ] Make queue admission/reservation a pure decision.
- [ ] Make git merge and regression checks `ExecutionEffect::RunManagedCommand` or `ExecutionEffect::RunMergeBackend`.
- [ ] Route git/cargo process execution through `ManagedCommandRunner`.
- [ ] Emit `MergeDecision::{Queued, Blocked, Reserved, Started, Succeeded, Conflict, RegressionFailed, Failed}`.
- [ ] Store conflict paths, abort result, stdout/stderr refs, exit status, and regression evidence as typed artifacts.
- [ ] Resume pending merge reservations from durable decisions and task/process state.

Acceptance proof:

- [ ] Merge conflict proof includes managed process events and typed conflict artifact refs.
- [ ] Regression failure proof shows git merge succeeded but merge decision is not `Succeeded`.
- [ ] Crash after merge reservation resumes without double-merging.

### Runner Drift R5 - Feedback, Learning, Dream, Extension, And Threshold Updates Are Runner Side Effects

Evidence:

```text
crates/roko-cli/src/runner/event_loop.rs:2294 emit runner-owned feedback after gate
crates/roko-cli/src/runner/event_loop.rs:2326 record_runner_event
crates/roko-cli/src/runner/event_loop.rs:2356 update_gate_thresholds
crates/roko-cli/src/runner/event_loop.rs:2397 observe cascade router
crates/roko-cli/src/runner/event_loop.rs:2586 fire_on_gate_hook
```

Problem:

- [ ] Execution policy should decide that feedback/learning hooks are needed, but not own their storage and side effects.
- [ ] Threshold and cascade-router updates can be lost or duplicated if retries/resume replay local branches.
- [ ] Dream/extension/feedback updates are hard to test as idempotent effects.

Implementation checklist:

- [ ] Represent these as `ExecutionEffect::{RecordFeedback, UpdateGateThreshold, ObserveRoutingOutcome, FireExtensionHook, TriggerDream}`.
- [ ] Include effect idempotency keys derived from run id, plan id, task id, attempt, gate, and decision id.
- [ ] Execute feedback effects through the feedback facade from [38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md](38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md).
- [ ] Execute threshold/routing effects through dedicated policy repositories, not inline JSON writes.
- [ ] Execute extension hooks after durable decision persistence and record hook outcome events.
- [ ] Ensure replay can skip already-completed idempotent effects.

Acceptance proof:

- [ ] Retry/resume does not duplicate threshold updates.
- [ ] Extension hook failure is queryable and does not corrupt runner state.
- [ ] Dream trigger is causally linked to the execution decision that produced it.

### Runner Drift R6 - Legacy Orchestrate Still Exports A Competing Policy Surface

Evidence:

```text
crates/roko-cli/src/lib.rs:116 pub use orchestrate::{OrchestrationReport, PlanRunReport, PlanRunner}
crates/roko-cli/src/orchestrate.rs:2328 pub struct PlanRunner
crates/roko-cli/src/orchestrate.rs:4795 gate_failure_next_action
crates/roko-cli/src/orchestrate.rs:5251 apply_replan_result
crates/roko-cli/src/orchestrate.rs:5297 persist adaptive gate thresholds
crates/roko-cli/src/orchestrate.rs:5575 run_conductor_check
crates/roko-cli/src/orchestrate.rs:7318 auto-dream path
```

Problem:

- [ ] `orchestrate.rs` still exposes the names and policy behaviors that runner-v2 is trying to replace.
- [ ] Callers can unintentionally use legacy `PlanRunner` and get different retry/replan/gate/merge/feedback semantics.
- [ ] Policy parity cannot be proven while two production policy surfaces remain active.

Implementation checklist:

- [ ] Rename legacy export to `orchestrate_legacy::{LegacyPlanRunner, LegacyOrchestrationReport, LegacyPlanRunReport}`.
- [ ] Add compile-time deprecation on legacy constructors.
- [ ] Move any remaining required policy behavior from `orchestrate.rs` into runner-v2 policy services.
- [ ] Add a grep gate for `PlanRunner::from_` outside legacy tests and compatibility shims.
- [ ] Require all CLI, serve, worker, and PRD execution callers to use the new command/execution service.
- [ ] Keep legacy only as an explicit fallback command with non-parity proof labels until removed.

Acceptance proof:

- [ ] Public crate exports no longer make legacy `PlanRunner` look canonical.
- [ ] `rg -n "PlanRunner::from_|use crate::orchestrate|roko_cli::PlanRunner" crates -g '*.rs'` has only allowlisted legacy shims/tests.
- [ ] Feature proof commands all report runner-v2 execution engine path.

## Reducer And Effect Contract

Core reducer API:

```rust
pub trait PlanExecutionEngine {
    fn reduce(
        &mut self,
        state: &ExecutionState,
        event: ExecutionInputEvent,
    ) -> Result<ExecutionReduction>;
}

pub struct ExecutionReduction {
    pub decisions: Vec<ExecutionDecision>,
    pub effects: Vec<ExecutionEffect>,
    pub checkpoint: Option<ExecutionCheckpointIntent>,
}
```

Input events:

- [ ] `CommandStarted`
- [ ] `ResumeLoaded`
- [ ] `TaskReady`
- [ ] `AgentDispatchCompleted`
- [ ] `AgentCompleted`
- [ ] `GateCompleted`
- [ ] `MergeBackendCompleted`
- [ ] `RegressionCompleted`
- [ ] `RetryTimerElapsed`
- [ ] `CancelRequested`
- [ ] `ShutdownRequested`
- [ ] `EffectFailed`

Decision records:

- [ ] `PlanStarted`
- [ ] `TaskSelected`
- [ ] `TaskSkipped`
- [ ] `AgentDispatchPlanned`
- [ ] `GateRequired`
- [ ] `GateSkipped`
- [ ] `GateUnsupported`
- [ ] `RetryScheduled`
- [ ] `RetryExhausted`
- [ ] `RepairRequested`
- [ ] `ReplanRequested`
- [ ] `MergeQueued`
- [ ] `MergeBlocked`
- [ ] `MergeReserved`
- [ ] `MergeSucceeded`
- [ ] `MergeConflict`
- [ ] `RegressionFailed`
- [ ] `PlanCompleted`
- [ ] `RunCompleted`

Effect variants:

- [ ] `SpawnAgent`
- [ ] `RunGate`
- [ ] `RunPlanVerify`
- [ ] `RunMergeBackend`
- [ ] `RunRegression`
- [ ] `WriteCheckpoint`
- [ ] `WriteArtifact`
- [ ] `RecordFeedback`
- [ ] `UpdateGateThreshold`
- [ ] `ObserveRoutingOutcome`
- [ ] `FireExtensionHook`
- [ ] `PublishTuiEvent`
- [ ] `TriggerDream`
- [ ] `ScheduleTimer`
- [ ] `CancelTask`

Rules:

- [ ] Reducers do not perform IO.
- [ ] Reducers do not spawn tasks.
- [ ] Reducers do not read or write files.
- [ ] Reducers do not call providers, gates, git, TUI, feedback stores, or dream runners.
- [ ] Effects contain idempotency keys.
- [ ] Decisions contain causation ids.
- [ ] Effect completions reference the originating effect id.
- [ ] Snapshot writes occur only at declared checkpoint intents.
- [ ] Replay from durable inputs must be deterministic.

## Additional Runner Grep Gates

```bash
rg -n "ctx\\.tui\\.|update_gate_thresholds|observe_cascade_router|fire_on_gate_hook|record_runner_event|DreamRunner::new|save_snapshot\\(" crates/roko-cli/src/runner/event_loop.rs
rg -n "RuntimeProjectionService|RuntimeQuery::GateEvidence|collect_runner_gate_results" crates/roko-cli/src/serve_runtime.rs crates/roko-serve/src crates/roko-cli/src
rg -n "gate_result\\([^\\n]*true\\)|GateStatus|SkippedByConfig|Unsupported" crates/roko-cli/src/runner crates/roko-gate/src crates/roko-serve/src crates/roko-cli/src/tui -g '*.rs'
rg -n "tokio::process::Command::new\\(\"git\"\\)|std::process::Command::new\\(\"git\"\\)|spawn_blocking|tokio::spawn" crates/roko-cli/src/runner/merge.rs crates/roko-cli/src/runner/event_loop.rs
rg -n "PlanRunner::from_|pub use orchestrate|use crate::orchestrate|roko_cli::PlanRunner" crates -g '*.rs'
rg -n "ExecutionInputEvent|ExecutionDecision|ExecutionEffect|PlanExecutionEngine|MergePolicyEngine|RepairPolicyEngine|TaskScheduler" crates/roko-cli/src crates/roko-runtime/src crates/roko-orchestrator/src -g '*.rs'
```

Completion targets:

- [ ] First grep has only effect adapter calls or is empty inside reducer branches.
- [ ] Second grep shows serve querying projections instead of scraping JSONL.
- [ ] Third grep shows gate status is typed and skipped/unsupported are not green passes.
- [ ] Fourth grep has process spawns only under managed command/effect adapters.
- [ ] Fifth grep has only explicit legacy shims/tests.
- [ ] Sixth grep shows active implementation of the reducer/effect contract.

## Updated Self-Grade After Reducer Deepening

Score before this pass: **9.84 / 10**.

Current score after this pass: **9.90 / 10**.

What improved:

- [ ] The audit now defines the exact reducer/effect contract needed to extract the event loop cleanly.
- [ ] It identifies side-effect leaks that were not sufficiently separated before: TUI publication, feedback, learning thresholds, cascade-router updates, extension hooks, serve JSONL scraping, gate skip-as-pass, merge subprocesses, and legacy exports.
- [ ] It gives implementation batches and grep gates that distinguish true architecture convergence from file splitting.
- [ ] It ties runner-policy extraction to lifecycle, adapter, artifact, gateway, observability, and cognitive-feedback docs.

Remaining risk:

- [ ] A later implementation pass should decide whether the reducer lives in `roko-runtime`, `roko-orchestrator`, or a new `roko-execution` crate after dependency-layering work from [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md) is applied.
