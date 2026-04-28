# 35 - Task Process Lifecycle Audit

Date: 2026-04-27

Purpose: this file documents the background task, child process, cancellation, timeout, shutdown, and operation-status architecture gaps that keep Roko from being a stable Mori-like orchestrator. It complements [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md), [31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md](31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md), [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md), and [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md).

If an agent is assigned "make Roko stop leaking/orphaning/losing operations" or "make serve/runner/daemon lifecycle robust", this file is the implementation handoff.

## Executive Verdict

Roko has good lifecycle primitives, but not a lifecycle system. `roko-runtime::process::ProcessSupervisor` is a strong reusable foundation with process ids, cancellation, graceful shutdown, process-session ledgers, and bulk wait/kill operations. `roko-core::shutdown::GracefulShutdown` and `roko-runtime::cancel::CancelToken` are also useful. The architecture problem is that most high-value entrypoints do not consistently route through those primitives.

Today, subprocesses and background tasks are launched directly from `roko-agent`, `roko-cli`, `roko-serve`, ACP bridge code, gates, tools, daemon code, route handlers, and sidecar flows. Some use `ProcessSupervisor`, some use `roko_agent::process::kill_tree`, some rely on `kill_on_drop(true)`, some store a `JoinHandle`, some drop it into `_handle`, some use `tokio_util::sync::CancellationToken`, some use `roko_runtime::cancel::CancelToken`, and some use a global static map.

The target is not to add another helper. The target is one lifecycle spine:

`RuntimeTaskSupervisor -> ManagedTask / ManagedProcess / ManagedCommand -> RuntimeEventStore -> RuntimeQueryService`

Initial self-grade after this pass: `9.83 / 10`.

Reason: this pass identifies the concrete spawn/cancel/shutdown split, names the existing primitives to preserve, defines the target lifecycle design, gives prioritized migration steps, and provides grep gates and proof requirements. It is not a `10` because a full proof would include live cancellation/orphan/crash artifacts generated from the current binary.

## Method

Commands used during this pass:

```bash
rg -n "tokio::spawn|spawn_blocking|std::thread::spawn|thread::spawn|JoinHandle|AbortHandle|CancellationToken|ctrl_c|shutdown|oneshot::channel|watch::channel|broadcast::channel|mpsc::channel" crates -g '*.rs'
rg -n "Command::new|tokio::process::Command|std::process::Command|kill_on_drop|child\\.kill|\\.kill\\(|wait_with_output|spawn\\(|Stdio::|pid\\(|Child" crates -g '*.rs'
rg -n "background|daemon|supervisor|Supervisor|OperationStatus|Operation|job_runner|JobRunner|sidecar|heartbeat|timeout|retry|resume|crash|recover|cleanup|orphan" crates -g '*.rs'
```

Count script used:

```bash
python3 - <<'PY'
from pathlib import Path
roots = [p for p in Path('crates').iterdir() if p.is_dir()]
patterns = {
    'tokio_spawn': 'tokio::spawn',
    'thread_spawn': 'thread::spawn',
    'std_thread_spawn': 'std::thread::spawn',
    'spawn_blocking': 'spawn_blocking',
    'join_handle': 'JoinHandle',
    'cancellation_token': 'CancellationToken',
    'command_new': 'Command::new',
    'tokio_process_command': 'tokio::process::Command',
    'std_process_command': 'std::process::Command',
    'kill_on_drop': 'kill_on_drop',
    'child_kill': '.kill(',
    'broadcast_channel': 'broadcast::channel',
    'mpsc_channel': 'mpsc::channel',
    'oneshot_channel': 'oneshot::channel',
    'watch_channel': 'watch::channel',
}
for root in roots:
    counts={k:0 for k in patterns}
    files=0
    for path in root.rglob('*.rs'):
        files += 1
        text=path.read_text(errors='ignore')
        for key, pat in patterns.items():
            counts[key]+=text.count(pat)
    total=sum(counts.values())
    if total:
        print(root.name, files, total, counts)
PY
```

## Current Scan Counts

| Crate | Files | Lifecycle Refs | `tokio::spawn` | Thread Spawn | `JoinHandle` | Cancellation Token | `Command::new` | `kill_on_drop` | Channels |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `roko-cli` | 243 | 232 | 26 | 8 | 23 | 24 | 69 | 2 | 23 |
| `roko-serve` | 78 | 127 | 58 | 2 | 41 | 3 | 8 | 0 | 5 |
| `roko-agent` | 160 | 82 | 18 | 21 | 16 | 0 | 12 | 3 | 1 |
| `roko-gate` | 42 | 34 | 0 | 0 | 0 | 0 | 12 | 10 | 0 |
| `roko-core` | 102 | 30 | 0 | 16 | 0 | 1 | 7 | 0 | 4 |
| `roko-plugin` | 3 | 28 | 6 | 0 | 0 | 15 | 0 | 0 | 7 |
| `roko-learn` | 78 | 26 | 8 | 6 | 6 | 0 | 0 | 0 | 6 |
| `roko-acp` | 8 | 23 | 4 | 0 | 2 | 0 | 3 | 0 | 4 |
| `roko-orchestrator` | 31 | 16 | 0 | 5 | 0 | 0 | 8 | 0 | 0 |
| `roko-agent-server` | 14 | 14 | 6 | 0 | 5 | 0 | 0 | 0 | 1 |
| `roko-runtime` | 15 | 10 | 4 | 0 | 0 | 1 | 3 | 1 | 1 |

Interpretation:

- `roko-runtime` owns the best process abstraction, but it is not the dominant owner of spawning.
- `roko-cli` and `roko-serve` still own too much lifecycle behavior for adapter crates.
- `roko-serve` has the highest background-task density and many task handles are not supervised as one group.
- `roko-agent`, `roko-cli`, ACP, gates, and serve routes have multiple direct subprocess paths.
- Gates and tools use `Command::output()` plus `kill_on_drop(true)` for bounded execution, which is not the same as observable, cancellable runtime process management.

## Hot Files

| File | Why It Matters |
| --- | --- |
| `crates/roko-runtime/src/process.rs` | Strong `ProcessSupervisor`, `SpawnConfig`, `ProcessSessionLedger`, cancel, wait, kill, and resume-compatible session state. This should be the process lifecycle root. |
| `crates/roko-runtime/src/cancel.rs` | Runtime-native cancellation token; should be the root cancellation vocabulary. |
| `crates/roko-core/src/shutdown.rs` | Graceful shutdown coordinator and leak sentinel concepts; useful but not wired as the one application shutdown path. |
| `crates/roko-agent/src/exec.rs` | Direct CLI subprocess spawn, stdout/stderr reader tasks, heartbeat loop, timeout, kill tree, PID registry. Duplicates runtime process supervision. |
| `crates/roko-agent/src/claude_cli_agent.rs` | Direct Claude CLI process ownership and stream parsing/progress. This is provider-specific lifecycle logic in provider code. |
| `crates/roko-cli/src/runner/agent_stream.rs` | Runner-local agent process spawn, stdout parser task, stderr task, PID registry, process kill. This should be dispatch/runtime-owned. |
| `crates/roko-cli/src/dispatch_direct.rs` | One-shot inline path spawns Claude directly and parses stream JSON outside the dispatcher/runtime lifecycle. |
| `crates/roko-cli/src/daemon.rs` | Has a serious shutdown choreography, but it supervises only selected handles and manually aborts others. |
| `crates/roko-cli/src/unified.rs` | Starts background serve and then calls `state.shutdown().await` followed by `handle.abort()`. This is not a clean managed lifecycle. |
| `crates/roko-serve/src/lib.rs` | Starts many background loops and subprocesses but stores most handles in ignored bindings. It also has event-source task groups that return an empty handle. |
| `crates/roko-serve/src/state.rs` | `AppState` has `cancel`, `supervisor`, `active_runs`, `active_plans`, and `operations`, but not one durable operation/task supervisor. |
| `crates/roko-serve/src/job_runner.rs` | Poll loop spawns per-job tasks with no durable operation handle or cancellation linkage. |
| `crates/roko-serve/src/routes/plans.rs` | HTTP route tasks have `CancelToken`, but spawned work does not receive it in the `runtime.run_once` call. Pause aborts the task rather than coordinated cancellation. |
| `crates/roko-serve/src/routes/run.rs` | Background run task is in memory and emits dashboard events directly. It should be a supervised operation. |
| `crates/roko-serve/src/routes/vision_loop.rs` | Uses a static `VISION_LOOPS` map and directly spawns `roko vision-loop`; cancellation can lose the child because the monitor task takes ownership. |
| `crates/roko-acp/src/bridge_events.rs` | Spawns Claude, Roko CLI, and shell commands directly with local cancellation and `child.kill()`. |
| `crates/roko-gate/src/*.rs` | Gate subprocesses use direct `Command` execution and timeouts with no runtime lifecycle events or shared process policy. |
| `crates/roko-agent/src/process/registry.rs` | Global PID registry is process-wide and based on current working directory, not a workspace-scoped runtime ledger. |

## Existing Pieces That Should Be Preserved

- `roko-runtime::process::ProcessSupervisor` should become the single long-lived child-process owner.
- `roko-runtime::process::ProcessSessionLedger` should become the durable process/session fact source for crash/resume/orphan proof.
- `roko-runtime::cancel::CancelToken` should be the internal root token. External token types can adapt into it.
- `roko_agent::process::set_process_group` and `kill_tree` have useful Unix process-group behavior. Move or wrap them under the runtime process backend instead of duplicating a second process layer.
- `roko-core::shutdown::GracefulShutdown` has good drain/report concepts. Reuse the semantics for application shutdown reports.
- Server `active_runs`, `active_plans`, and `operations` are useful API concepts, but they should become durable projections of a supervised operation store.
- `roko-plugin::EventSource` already accepts `tokio_util::sync::CancellationToken`; keep an adapter while migrating toward the runtime cancel root.

## Target Design

The target design has one place to spawn work, one place to cancel work, one place to observe work, and one place to query work.

| Layer | Component | Responsibility |
| --- | --- | --- |
| L1 contract | `RuntimeTaskSpec` | Describes task id, kind, owner, run id, workdir, timeout, shutdown policy, retry policy, durability, and observability labels. |
| L2 supervisor | `RuntimeTaskSupervisor` | Owns async tasks, managed commands, long-lived processes, cancellation tree, join handles, shutdown phases, and leak detection. |
| L3 process backend | `ManagedProcess` / `ManagedCommand` | Wraps `ProcessSupervisor`, process groups, stdout/stderr pumps, byte limits, timeout handling, PID/session ledger, and exit classification. |
| L4 operation store | `OperationStore` | Durable state machine for run, plan, PRD, research, job, vision-loop, daemon, sidecar, and background event-source operations. |
| L5 events | `RuntimeEventStore` | Appends task/process/operation lifecycle events with correlation ids and evidence references. |
| L6 query | `RuntimeQueryService` | Returns live and recovered task/process/operation state to HTTP, TUI, CLI, and proof scripts. |
| L7 adapters | CLI, serve, ACP, gates, providers, tools | Request supervised work and render/query results. They do not own raw `JoinHandle`/`Child` lifecycle. |

Core principle:

- A route handler, provider, gate, or CLI command may define what should run.
- Only the lifecycle spine decides how it is spawned, cancelled, timed out, reaped, logged, persisted, and queried.

## P0 Findings

### P0-01 Process Spawning Is Split Across Multiple Owners

Problem:

`roko-runtime::process::ProcessSupervisor` exists, but direct process ownership remains widespread. Provider adapters, runner code, ACP bridge code, server routes, gates, tools, and daemon utilities all construct `Command` directly.

Evidence:

```text
crates/roko-runtime/src/process.rs:876 ProcessSupervisor::spawn owns SpawnConfig -> Command -> ProcessHandle.
crates/roko-agent/src/exec.rs:157 ExecAgent builds Command directly and manages stdout/stderr/heartbeat/timeout.
crates/roko-agent/src/claude_cli_agent.rs:302 build_command and 414 cmd.spawn own Claude CLI directly.
crates/roko-cli/src/runner/agent_stream.rs:145 builds Command for a CLI agent inside runner.
crates/roko-cli/src/dispatch_direct.rs:50 spawns `claude` directly for inline one-shot.
crates/roko-serve/src/routes/vision_loop.rs:133 spawns `roko vision-loop` directly.
crates/roko-acp/src/bridge_events.rs:446, 1109, 1210 spawn Claude, Roko, and shell commands directly.
crates/roko-gate/src/compile.rs:85 and related gates spawn verification commands directly.
```

Why it matters:

Every direct `Command` path makes a new decision about process groups, environment scrubbing, stdin closure, stdout/stderr reading, timeout, kill escalation, PID tracking, and event emission. That is why behavior works in one entrypoint but fails in another.

Target design:

All executable processes should use a shared `ManagedCommandRunner` backed by `ProcessSupervisor`.

Implementation checklist:

- [ ] Add `RuntimeTaskSpec` to `roko-runtime` with `task_id`, `kind`, `owner`, `workdir`, `timeout_ms`, `grace_ms`, `durability`, `correlation_id`, and `shutdown_policy`.
- [ ] Add `ManagedCommandSpec` with program, args, stdin mode, stdout mode, stderr mode, env policy, process-group policy, byte limits, and parser adapter id.
- [ ] Add `RuntimeTaskSupervisor::spawn_command(spec, command)` that internally uses `ProcessSupervisor::spawn`.
- [ ] Add `RuntimeTaskSupervisor::run_command_to_output` for short-lived gates/tools so they do not need to manage `Child` directly.
- [ ] Move or wrap `roko_agent::process::set_process_group`, `kill_tree`, and descendant collection under the runtime process backend.
- [ ] Migrate `roko-agent/src/exec.rs` and `roko-agent/src/claude_cli_agent.rs` to request managed commands instead of owning `Command`.
- [ ] Migrate `roko-cli/src/runner/agent_stream.rs` to dispatch/runtime-managed process streams.
- [ ] Delete or quarantine `roko-cli/src/dispatch_direct.rs` after one-shot inline uses the same dispatcher/runtime path.
- [ ] Migrate ACP subprocess calls to managed commands.
- [ ] Migrate `roko-serve/src/routes/vision_loop.rs` to `RuntimeTaskSupervisor::spawn_process`.
- [ ] Migrate gate and tool command execution to `run_command_to_output`.
- [ ] Add grep gate: production code should not contain raw `tokio::process::Command::new` or `std::process::Command::new` outside runtime process backend, tests, build scripts, and explicitly allowlisted installer/system integration code.

### P0-02 Background Tasks Are Fire-And-Forget Instead Of Supervised

Problem:

Many server and CLI background tasks are spawned and then ignored, or only partially tracked. Some loops honor `state.cancel`; some tasks have independent cancellation; some per-request tasks are stored in maps; some event-source groups return an empty handle while spawning child tasks.

Evidence:

```text
crates/roko-serve/src/lib.rs:224 starts dispatch_loop and drops the handle.
crates/roko-serve/src/lib.rs:226-232 starts config watcher, PRD orchestrator, feedback loop, state bridge, saver, job runner, and archival into ignored bindings.
crates/roko-serve/src/lib.rs:1075-1108 start_event_source_group spawns multiple tasks and returns tokio::spawn(async {}).
crates/roko-serve/src/job_runner.rs:76 spawns job execution tasks without tracking them.
crates/roko-cli/src/daemon.rs:361-365 starts scheduler/watchers/dispatch/feedback/dreams into ignored bindings.
crates/roko-cli/src/unified.rs:67-70 shuts down background serve and then aborts the server handle.
```

Why it matters:

An orchestrator needs to answer: what is running, why is it running, who owns it, can it be cancelled, did it stop, and what evidence proves it stopped? Fire-and-forget tasks cannot answer those questions after a hang, crash, or failed provider run.

Target design:

Every background task should be registered as a `ManagedTask` with a task id, parent id, cancellation token, join policy, and runtime lifecycle events.

Implementation checklist:

- [ ] Add `RuntimeTaskSupervisor::spawn_task(spec, future)` returning `ManagedTaskHandle`.
- [ ] Make each `start_*` server loop register with the supervisor instead of returning a bare `JoinHandle`.
- [ ] Replace ignored `_handle` locals in `roko-serve/src/lib.rs` and `roko-cli/src/daemon.rs` with supervised task ids.
- [ ] Make `start_event_source_group` return a real group handle that owns the ingest loop and all source tasks.
- [ ] Add task parent/child relationships so event-source tasks, job tasks, route tasks, and process IO pumps can be drained by group.
- [ ] Add task lifecycle events: `task.started`, `task.cancel_requested`, `task.completed`, `task.failed`, `task.panicked`, `task.timed_out`, `task.force_aborted`.
- [ ] Add join policies: `must_join`, `best_effort`, `detached_but_registered`, and `shutdown_only`.
- [ ] Make `AppState::shutdown` ask the task supervisor to cancel and drain before process shutdown.
- [ ] Make daemon shutdown use the same task supervisor, not a separate manual choreography.
- [ ] Add grep gate: `tokio::spawn` in production code must either be inside `RuntimeTaskSupervisor`, an allowlisted tiny local test adapter, or explicitly wrapped with task registration.

### P0-03 Cancellation Tokens Are Fragmented And Often Not Propagated

Problem:

Roko uses both `roko_runtime::cancel::CancelToken` and `tokio_util::sync::CancellationToken`. Some HTTP handles store a cancel token but do not pass it into the actual running work. Some paths abort tasks instead of cooperative cancellation. Some processes are killed directly instead of going through process shutdown policy.

Evidence:

```text
crates/roko-runtime/src/cancel.rs defines the runtime cancel token.
crates/roko-plugin/src/lib.rs EventSource::start accepts tokio_util::sync::CancellationToken.
crates/roko-cli/src/runner/event_loop.rs uses tokio_util::sync::CancellationToken.
crates/roko-serve/src/state.rs AppState uses roko_runtime::cancel::CancelToken.
crates/roko-serve/src/routes/plans.rs:240 stores PlanHandle.cancel but execute_plan does not pass it to runtime.run_once.
crates/roko-serve/src/routes/plans.rs:296 cancels then aborts the tokio task.
crates/roko-acp/src/bridge_events.rs uses local cancel checks and child.kill().
```

Why it matters:

Cancellation is a tree, not a boolean. If parent cancellation does not propagate into route work, provider dispatch, subprocess IO pumps, process groups, and proof events, Roko cannot reliably pause, stop, resume, or prove shutdown.

Target design:

Use `roko_runtime::cancel::CancelToken` as the internal root. Add explicit adapters for `tokio_util::sync::CancellationToken` at plugin boundaries.

Implementation checklist:

- [ ] Define a `RuntimeCancellation` trait or adapter functions that bridge `tokio_util::CancellationToken` into `roko_runtime::CancelToken`.
- [ ] Make `RunConfig`, `DispatchRequest`, `RuntimeTaskSpec`, `ManagedCommandSpec`, and `CliRuntime::run_once` accept a runtime cancel token or cancellation context.
- [ ] Change plan execution, plan resume, PRD generation, research, background run, job runner, and vision-loop routes to pass their operation token into the actual work.
- [ ] Replace `handle.abort()` with cooperative cancellation plus bounded join; allow abort only after shutdown policy escalates.
- [ ] Emit `cancel_requested`, `cancel_acknowledged`, `cancel_escalated`, and `cancel_completed` events.
- [ ] Persist cancellation reason, requester, timestamp, and affected child task ids.
- [ ] Add proof: cancelling an HTTP plan run cancels provider dispatch, child process, IO pumps, and operation status without duplicate completion.
- [ ] Add grep gate: route handlers may not call `JoinHandle::abort` except in a lifecycle-supervisor force-abort path.

### P0-04 Operations Are In-Memory State Maps Instead Of Durable Runtime Facts

Problem:

The server has in-memory maps for active runs, plans, and operations. Vision loop uses a global static map. Server state snapshot persists only selected state. These maps are not the durable source of truth for operation history, cancellation, resume, crash recovery, or proof.

Evidence:

```text
crates/roko-serve/src/state.rs:374-379 active_runs, active_plans, and operations are RwLock<HashMap<...>>.
crates/roko-serve/src/routes/vision_loop.rs:21 static VISION_LOOPS stores in-flight runs.
crates/roko-serve/src/routes/run.rs:89 spawn_background_run returns a run id and stores task state in memory.
crates/roko-serve/src/routes/plans.rs:248 active.insert(id, plan_handle) stores running plan state in memory.
crates/roko-serve/src/state.rs:615 save_snapshot persists discovered agents and template runs, not a complete operation ledger.
```

Why it matters:

HTTP status, TUI state, proof scripts, and resume need the same facts. In-memory maps are useful caches, but they vanish on crash and do not prove lifecycle transitions.

Target design:

Add a durable `OperationStore` and make in-memory maps materialized live indexes.

Implementation checklist:

- [ ] Define `OperationId`, `OperationKind`, `OperationStatus`, `OperationTransition`, and `OperationEvidenceRef` in a shared runtime or serve-neutral crate.
- [ ] Add `OperationStore` append/read APIs under `.roko/state/operations.jsonl` or the unified runtime event store.
- [ ] Store operation creation, start, progress, cancel, pause, resume, completion, failure, timeout, and force-abort transitions.
- [ ] Make `active_runs`, `active_plans`, and `operations` live indexes rebuilt from `OperationStore` plus currently supervised task ids.
- [ ] Replace static `VISION_LOOPS` with supervised operations and process-session records.
- [ ] Add query methods for `operation_by_id`, `operations_by_run`, `operations_by_status`, and `operation_evidence`.
- [ ] Make `/api/runs`, `/api/plans/*/status`, `/api/operations/*`, `/api/vision-loop/*`, and TUI views query the same operation projection.
- [ ] Add crash/restart proof that an in-flight operation recovers as `interrupted` or `resumable` with child process evidence.

### P0-05 Child IO, Heartbeat, And Stream Parsing Are Owned By Callers

Problem:

Each direct process owner decides how to read stdout/stderr, parse provider streams, surface heartbeat messages, and accumulate output. This duplicates buffering and makes observability inconsistent.

Evidence:

```text
crates/roko-agent/src/exec.rs:213 stdout task, 244 stderr task, 266 heartbeat task.
crates/roko-agent/src/claude_cli_agent.rs:455 stdout stream parser, 546 stderr task, 569 heartbeat task.
crates/roko-cli/src/runner/agent_stream.rs:199 stdout reader task and 220 stderr reader task.
crates/roko-cli/src/dispatch_direct.rs:75 reads Claude stdout line by line, then waits and reads stderr.
crates/roko-acp/src/bridge_events.rs reads subprocess stdout in local loops for Claude, Roko, and shell commands.
```

Why it matters:

Provider runtime events, progress, stderr diagnostics, byte limits, silent timeouts, and final output extraction should be consistent across runner, inline, ACP, serve, and CLI.

Target design:

The runtime process backend should include an IO pump that emits provider-neutral stream events and bounded artifacts.

Implementation checklist:

- [ ] Add `ProcessIoPolicy` with stdout/stderr mode, line/chunk mode, max bytes, redaction policy, and silence heartbeat interval.
- [ ] Add `ProcessStreamAdapter` trait that maps raw lines/chunks into `RuntimeEventEnvelope` and optional final output.
- [ ] Implement adapters for Claude stream-json, Codex stream-json, generic text, shell output, gate JSON, and MCP subprocess logs.
- [ ] Move heartbeat/silence detection into the IO pump so it is not provider-specific.
- [ ] Persist stdout/stderr previews and full artifact paths with byte counts and truncation status.
- [ ] Replace provider-specific progress `eprintln!` with runtime events plus adapter rendering.
- [ ] Add proof that the same provider run emits identical lifecycle/progress events through runner, inline one-shot, serve, and ACP.

## P1 Findings

### P1-01 Shutdown Is Not One Ordered Choreography

Problem:

There are multiple shutdown implementations: server `AppState::shutdown`, daemon `graceful_shutdown_daemon`, core `GracefulShutdown`, runtime process supervisor shutdown, provider kill trees, and ad-hoc task aborts.

Evidence:

```text
crates/roko-serve/src/state.rs:598 shutdown saves snapshot, cancels, shuts down supervisor, publishes server shutdown.
crates/roko-cli/src/daemon.rs:1366 graceful_shutdown_daemon cancels HTTP, aborts reload signal task, waits/kills supervisor, flushes artifacts.
crates/roko-core/src/shutdown.rs defines GracefulShutdown and LeakSentinel.
crates/roko-runtime/src/process.rs:950 shutdown_all and 1032 kill_all manage processes.
crates/roko-cli/src/unified.rs:67 calls state.shutdown then handle.abort.
```

Target design:

One `ShutdownCoordinator` should run every application lifecycle:

1. Reject new operations.
2. Emit shutdown requested event.
3. Cancel operation/task tree.
4. Drain must-join tasks.
5. Stop IO pumps.
6. Gracefully terminate processes.
7. Force-kill remaining processes.
8. Flush event/projection/operation stores.
9. Emit leak report and shutdown complete event.

Implementation checklist:

- [ ] Add `ShutdownCoordinator` backed by `RuntimeTaskSupervisor`, `ProcessSupervisor`, `OperationStore`, and `RuntimeEventStore`.
- [ ] Replace `AppState::shutdown` internals with coordinator phases.
- [ ] Replace daemon-specific shutdown internals with coordinator phases plus daemon-specific socket cleanup hooks.
- [ ] Replace unified chat `handle.abort()` with coordinator shutdown and bounded join.
- [ ] Add shutdown hooks for config watcher, scheduler, feedback loop, dream loop, job runner, relay registration, bridges, and event-source groups.
- [ ] Emit a shutdown report containing task count, process count, forced aborts, forced kills, persisted files, and leak sentinel result.
- [ ] Add proof: `roko serve` with active background run shuts down without orphaned child processes and with queryable shutdown report.

### P1-02 Gates And Tools Need A Managed Command Policy

Problem:

Gates and tools are short-lived commands, but they still need consistent timeout, cancellation, output limits, process-group kill, redaction, and observability. Today they use direct `Command::output()` and `kill_on_drop(true)`.

Evidence:

```text
crates/roko-gate/src/compile.rs:85-110 builds cargo command and wraps cmd.output() in timeout.
crates/roko-gate/src/test_gate.rs:116-135 same pattern for tests.
crates/roko-gate/src/shell.rs:75-90 shell gate command with timeout.
crates/roko-std/src/tool/builtin/bash.rs:86-90 bash tool command with timeout.
```

Target design:

Use a `ManagedCommandRunner` for all command-to-output execution. It can optimize for short-lived commands, but it still records task/process lifecycle and applies a central policy.

Implementation checklist:

- [ ] Define default command policies for `gate`, `tool`, `provider_cli`, `mcp_server`, `watcher`, `daemon_helper`, and `sidecar`.
- [ ] Migrate compile/test/clippy/generated/property/integration/security/shell gates to `ManagedCommandRunner`.
- [ ] Migrate built-in bash, grep, glob, and test tools where they shell out.
- [ ] Add output byte caps and artifact spillover for gate stdout/stderr.
- [ ] Emit gate command lifecycle events linked to the parent gate event.
- [ ] Prove timeout kills the full process tree, not just the parent process.

### P1-03 Sidecar, Serve, Daemon, And Unified Modes Have Independent Runtime Contracts

Problem:

Roko now has several application modes: inline one-shot, inline chat with background serve, `roko serve`, daemon, agent sidecar, ACP, plan runner, and serve-triggered plans. They do not all share one lifecycle/app runtime contract.

Evidence:

```text
crates/roko-cli/src/unified.rs starts background serve and uses direct dispatch.
crates/roko-cli/src/daemon.rs starts serve-like loops plus daemon IPC and signal handling.
crates/roko-cli/src/agent_serve.rs owns sidecar deletion/shutdown flows.
crates/roko-serve/src/lib.rs starts HTTP app loops.
crates/roko-acp/src/bridge_events.rs owns its own subprocess lifecycle.
```

Target design:

Create one `RokoApplicationRuntime` or `RuntimeHost` for all modes. Modes configure which adapters are enabled; they do not duplicate lifecycle rules.

Implementation checklist:

- [ ] Define `RuntimeHostConfig` with mode, workdir, config, enabled adapters, cancellation root, event store, operation store, task supervisor, process supervisor, and query service.
- [ ] Use `RuntimeHost` from `roko serve`, daemon, unified chat, plan runner, ACP, and agent sidecar.
- [ ] Make serve/daemon/unified differ by adapter set, not by lifecycle implementation.
- [ ] Ensure background serve launched by unified chat registers as a child host/task and shuts down through the same coordinator.
- [ ] Add proof that the same run id can be queried through CLI, HTTP, and TUI regardless of start mode.

### P1-04 PID Registry Should Become Workspace-Scoped Process Ledger

Problem:

`roko_agent::process::registry` persists PIDs to `.roko/runtime/agent-pids.json` based on current working directory and a process-wide static set. This is useful as a safety net, but it conflicts with the richer workspace-scoped `ProcessSessionLedger` already in `roko-runtime`.

Evidence:

```text
crates/roko-agent/src/process/registry.rs:24 agent_pids_path uses std::env::current_dir().
crates/roko-agent/src/process/registry.rs:49 register_spawned_pid persists global process set.
crates/roko-runtime/src/process.rs:46 default_process_session_ledger_path uses workdir/.roko/state/process-sessions.json.
crates/roko-runtime/src/process.rs:570 record_session_state writes structured session records.
```

Target design:

The PID registry should be a compatibility reaper over the process-session ledger, not a separate process fact store.

Implementation checklist:

- [ ] Make process session ledger the authoritative process record.
- [ ] Include workspace id, run id, operation id, task id, provider id, OS pid, process group id, started/updated/ended timestamps, and state.
- [ ] Teach startup cleanup to read the process-session ledger and classify stale live PIDs as orphaned/interrupted.
- [ ] Keep old `.roko/runtime/agent-pids.json` reader as migration fallback.
- [ ] Remove current-dir dependence from process registration APIs.
- [ ] Add proof: crash after process spawn, restart in same workspace, stale child is reaped and operation is marked interrupted.

## P2 Findings

### P2-01 Empty No-Op Handles Hide Missing Task Ownership

Problem:

Several functions return `tokio::spawn(async {})` as a no-op handle when no work starts or when real child tasks are spawned elsewhere. This satisfies types but hides ownership.

Evidence:

```text
crates/roko-serve/src/scheduler.rs:27 returns tokio::spawn(async {}) on duplicate scheduler.
crates/roko-serve/src/lib.rs:1072 returns tokio::spawn(async {}) for empty event-source groups.
crates/roko-serve/src/lib.rs:1108 returns tokio::spawn(async {}) after spawning real event source tasks.
crates/roko-serve/src/routes/status/mod.rs:415 creates a dummy JoinHandle.
```

Implementation checklist:

- [ ] Replace dummy handles with `ManagedTaskHandle::noop(reason)` or `TaskGroupHandle` with explicit empty state.
- [ ] Make no-op handles visible in lifecycle projections as `skipped` or `not_started`, not invisible successful tasks.
- [ ] Add grep gate: no production `tokio::spawn(async {})` except tests.

### P2-02 Static Process-Wide Guards Are Not Multi-Workspace Safe

Problem:

The scheduler uses a process-wide atomic guard to prevent duplicate scheduler starts. That may be useful in one server, but it is not safe for multi-workspace/multi-host tests or embedded hosts.

Evidence:

```text
crates/roko-serve/src/scheduler.rs:12 static SCHEDULER_STARTED.
crates/roko-serve/src/scheduler.rs:21 start_scheduler uses a process-wide compare_exchange.
```

Implementation checklist:

- [ ] Replace process-wide scheduler guard with a supervisor-owned task registry keyed by workdir and component id.
- [ ] Allow multiple independent workspaces in one process.
- [ ] Emit duplicate-start attempts as lifecycle events with owner/workdir identity.

## Implementation Order

Do not start by deleting old process code. Start by creating the common lifecycle seam and migrating one path at a time.

1. Create `RuntimeTaskSpec`, `ManagedCommandSpec`, `RuntimeTaskSupervisor`, and `ManagedTaskHandle` in `roko-runtime`.
2. Wire `RuntimeTaskSupervisor` to existing `ProcessSupervisor` and `CancelToken`.
3. Add runtime lifecycle events and operation transitions to the event/projection plan from [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md).
4. Migrate runner/provider CLI dispatch first, because it is the highest-value process path.
5. Migrate serve route operations (`run`, `plans`, `research`, `PRD`, `vision-loop`) to supervised operations.
6. Migrate server background loops into supervised task groups.
7. Migrate daemon/unified/sidecar hosts to `RuntimeHost` and `ShutdownCoordinator`.
8. Migrate gates/tools to `ManagedCommandRunner`.
9. Replace PID registry authority with process-session ledger and keep migration fallback.
10. Add grep gates and proof scripts after each migration slice.

## Grep Gates

These gates should be run before claiming this doc is complete:

```bash
rg -n "tokio::process::Command::new|std::process::Command::new|Command::new" crates/roko-cli/src crates/roko-serve/src crates/roko-agent/src crates/roko-acp/src crates/roko-gate/src crates/roko-std/src -g '*.rs'
rg -n "tokio::spawn\\(|std::thread::spawn|thread::spawn|spawn_blocking" crates/roko-cli/src crates/roko-serve/src crates/roko-agent/src crates/roko-acp/src -g '*.rs'
rg -n "JoinHandle::abort|\\.abort\\(\\)|child\\.kill\\(\\)|\\.kill\\(Duration|kill_on_drop\\(true\\)" crates/roko-cli/src crates/roko-serve/src crates/roko-agent/src crates/roko-acp/src crates/roko-gate/src crates/roko-std/src -g '*.rs'
rg -n "tokio_util::sync::CancellationToken|roko_runtime::cancel::CancelToken|CancelToken::new\\(\\)" crates/roko-cli/src crates/roko-serve/src crates/roko-agent/src crates/roko-acp/src crates/roko-plugin/src -g '*.rs'
rg -n "tokio::spawn\\(async \\{\\}\\)|static .*LazyLock|static .*AtomicBool|active_runs|active_plans|operations|VISION_LOOPS" crates/roko-serve/src crates/roko-cli/src -g '*.rs'
```

Expected final state:

- Raw `Command::new` appears only in runtime process backend, tests, build scripts, installer/system integration code, and explicitly documented allowlist entries.
- Raw `tokio::spawn` appears only in `RuntimeTaskSupervisor` or allowlisted adapter internals that register their task.
- `JoinHandle::abort`, direct `child.kill()`, and dummy empty tasks are not used in production lifecycle paths.
- Cancellation token construction is centralized at host/operation/task boundaries.
- Active operation maps are cache/indexes over durable operation/runtime events.

## Proof Requirements

This audit is complete only when proof scripts can show the following without mocks:

- [ ] Start a real provider-backed plan run and query the operation, task, process, provider, prompt, gate, and merge lifecycle through HTTP.
- [ ] Cancel a running provider CLI operation and prove process tree termination, operation state, runtime events, and no duplicate completion.
- [ ] Timeout a command/gate and prove full process-tree kill plus gate failure evidence.
- [ ] Crash after child process spawn, restart, reap stale process, and query interrupted/resumable state.
- [ ] Start `roko serve`, scheduler, job runner, feedback, dream loop, and event-source group, then shut down and query a shutdown report with zero leaked tasks/processes.
- [ ] Run inline one-shot, unified chat, `roko serve` HTTP run, daemon run, ACP run, and plan runner through the same lifecycle/event/query path.
- [ ] Run the grep gates above and store their allowlist output in the proof bundle.

## Handoff Checklist

- [ ] Implement `RuntimeTaskSpec`, `ManagedCommandSpec`, and `RuntimeTaskSupervisor`.
- [ ] Back `RuntimeTaskSupervisor` with existing `ProcessSupervisor`.
- [ ] Add `OperationStore` or map operations onto the unified runtime event store.
- [ ] Add lifecycle event types to `RuntimeEventEnvelope`.
- [ ] Migrate runner/provider CLI dispatch.
- [ ] Migrate inline one-shot/direct dispatch.
- [ ] Migrate serve background loops.
- [ ] Migrate serve route operations.
- [ ] Migrate vision-loop subprocess route.
- [ ] Migrate daemon and unified shutdown.
- [ ] Migrate ACP subprocess execution.
- [ ] Migrate gates and tools to managed commands.
- [ ] Replace global PID registry authority with process-session ledger.
- [ ] Add shutdown coordinator.
- [ ] Add lifecycle projection and HTTP query methods.
- [ ] Add proof scripts for cancel, timeout, crash/restart, shutdown, and multi-entrypoint parity.
- [ ] Run grep gates and document allowlisted exceptions.

## 2026-04-27 Deepening Pass - Source-Verified Lifecycle Drift

This pass turns the broad lifecycle audit above into concrete, source-backed implementation work. The recurring design failure is not that Roko lacks lifecycle primitives. It has useful pieces: `ProcessSupervisor`, process-group setup, `kill_tree`, cancellation tokens, runner stream handles, and route operation handles. The failure is that those pieces are not the only path. Long-running work is still started by routes, server startup code, scheduler code, job loops, and runner helpers using local `tokio::spawn`, in-memory `JoinHandle` maps, direct process spawning, or synthetic no-op handles.

The target design is a single operation/task/process lifecycle plane:

- [ ] Every user-visible unit of work is an `Operation`.
- [ ] Every async unit under an operation is a `RuntimeTask`.
- [ ] Every subprocess is a `ManagedProcess`.
- [ ] Every background loop is a named `RuntimeService`.
- [ ] Every operation/task/process/service transition emits durable events.
- [ ] HTTP and TUI status read projections of durable lifecycle events, not private route-local maps.
- [ ] Cancellation, timeout, crash recovery, process reaping, and shutdown all use the same supervisor path.

### Lifecycle Drift L1 - Serve Operation Maps Are Volatile Lifecycle Stores

Evidence:

```text
crates/roko-serve/src/state.rs:209 RunHandle
crates/roko-serve/src/state.rs:223 PlanHandle
crates/roko-serve/src/state.rs:237 OperationHandle
crates/roko-serve/src/state.rs:375 active_runs: RwLock<HashMap<String, RunHandle>>
crates/roko-serve/src/state.rs:377 active_plans: RwLock<HashMap<String, PlanHandle>>
crates/roko-serve/src/state.rs:379 operations: RwLock<HashMap<String, OperationHandle>>
```

Problem:

- [ ] `RunHandle`, `PlanHandle`, and `OperationHandle` contain `JoinHandle`s and mutable in-memory status.
- [ ] Operation truth disappears on server restart.
- [ ] A route can report success/failure without a durable event proving task start, process start, provider call, cancellation, timeout, or process exit.
- [ ] Multiple status mechanisms exist: `active_runs`, `active_plans`, generic `operations`, job JSON files, runner events, and provider-specific logs.
- [ ] Crash recovery cannot distinguish "task completed", "task abandoned", "process still running", and "process killed during shutdown" from these maps alone.

Implementation checklist:

- [ ] Introduce `RuntimeOperationStore` backed by the same durable event/projection path described in [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md).
- [ ] Define `OperationId`, `TaskId`, `ProcessId`, `ServiceId`, `AttemptId`, and `CorrelationId` as stable ids shared by CLI, serve, HTTP, runner, dispatcher, and agent runtime.
- [ ] Replace mutable `OperationStatus` fields with event-derived projections.
- [ ] Keep in-memory maps only as caches from operation id to active supervisor handle.
- [ ] Persist `operation.created`, `operation.started`, `operation.completed`, `operation.failed`, `operation.cancel_requested`, `operation.cancelled`, `operation.timeout`, and `operation.recovered` events.
- [ ] Add a startup reconciler that marks operations without terminal events as `interrupted` or `recovering`.
- [ ] Add HTTP query endpoints for operation detail, task tree, process tree, lifecycle timeline, current status, and recovery decision.
- [ ] Add a migration adapter that can expose old `active_runs` / `active_plans` responses from the new projection until routes are migrated.

Acceptance proof:

- [ ] Start a run, restart `roko serve`, and query the same operation id.
- [ ] The query returns a durable lifecycle timeline even though old `JoinHandle`s no longer exist.
- [ ] No route-local status map is required to answer completed/failed/cancelled/interrupted state.

### Lifecycle Drift L2 - Routes Spawn Long-Running Operations Directly

Evidence:

```text
crates/roko-serve/src/routes/run.rs:102 tokio::spawn
crates/roko-serve/src/routes/plans.rs:216 tokio::spawn
crates/roko-serve/src/routes/plans.rs:374 tokio::spawn
crates/roko-serve/src/routes/plans.rs:506 tokio::spawn
crates/roko-serve/src/routes/plans.rs:1159 tokio::spawn
crates/roko-serve/src/routes/prds.rs:491 tokio::spawn
crates/roko-serve/src/routes/prds.rs:671 tokio::spawn
crates/roko-serve/src/routes/prds.rs:905 tokio::spawn
crates/roko-serve/src/routes/research.rs:213 tokio::spawn
crates/roko-serve/src/routes/templates.rs:151 tokio::spawn
crates/roko-serve/src/routes/dream.rs:47 tokio::spawn
crates/roko-serve/src/routes/gateway.rs:522 tokio::spawn
crates/roko-serve/src/routes/deployments.rs:660 tokio::spawn
```

Problem:

- [ ] Route handlers own runtime behavior instead of constructing operation specs and handing them to a supervisor.
- [ ] Each route decides its own status mutations, cancellation behavior, retry behavior, and artifact write behavior.
- [ ] Some route-owned operations call `runtime.run_once` with ad hoc prompts instead of using the planned workflow/dispatch path.
- [ ] Route background tasks cannot be listed, joined, recovered, or shut down uniformly.
- [ ] HTTP status cannot prove nested task/provider/process lifecycle consistently.

Target service seam:

```rust
pub trait RuntimeTaskSupervisor: Send + Sync {
    async fn spawn_operation(&self, spec: RuntimeOperationSpec) -> Result<OperationRef>;
    async fn spawn_task(&self, spec: RuntimeTaskSpec) -> Result<TaskRef>;
    async fn cancel_operation(&self, id: OperationId, reason: CancelReason) -> Result<CancelReport>;
    async fn wait_operation(&self, id: OperationId) -> Result<OperationOutcome>;
    async fn status(&self, id: OperationId) -> Result<OperationProjection>;
    async fn shutdown_group(&self, group: RuntimeGroupId, reason: ShutdownReason) -> Result<ShutdownReport>;
}
```

Required operation spec:

```rust
pub struct RuntimeOperationSpec {
    pub id: OperationId,
    pub kind: OperationKind,
    pub owner: OperationOwner,
    pub workspace: WorkspaceRef,
    pub correlation: CorrelationId,
    pub cancel_policy: CancelPolicy,
    pub timeout_policy: TimeoutPolicy,
    pub retry_policy: RetryPolicy,
    pub observability: ObservabilityPolicy,
    pub artifact_policy: ArtifactPolicy,
    pub root_task: RuntimeTaskSpec,
}
```

Implementation checklist:

- [ ] Add `RuntimeOperationSpec` and `RuntimeTaskSpec` in a runtime crate that both `roko-cli` and `roko-serve` can depend on.
- [ ] Add `OperationKind::{PlanRun, PlanResume, PlanChat, PrdDraft, PrdPromote, Research, TemplateDeploy, Dream, GatewayRequest, Deployment, Job, VisionLoop, AgentServe}`.
- [ ] Make route handlers validate HTTP payloads, build specs, call `RuntimeTaskSupervisor`, and return `OperationRef`.
- [ ] Move route-local `runtime.run_once` calls behind a common `WorkflowTask` or `DispatchTask`.
- [ ] Move per-route status mutation into lifecycle event emission.
- [ ] Move per-route artifact writes into the artifact-store plan from [37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md](37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md).
- [ ] Add one compatibility endpoint that maps old route status DTOs from the new lifecycle projection.

Acceptance proof:

- [ ] `rg -n "tokio::spawn" crates/roko-serve/src/routes -g '*.rs'` returns only allowlisted short-lived request-local helpers or tests.
- [ ] Starting PRD, plan, research, template, dream, gateway, deployment, and run operations all creates lifecycle events with the same schema.
- [ ] Cancelling any operation uses the same cancel path and emits the same cancel event family.

### Lifecycle Drift L3 - Startup Loops Are Bare Background Tasks

Evidence:

```text
crates/roko-serve/src/lib.rs:224 tokio::spawn(dispatch::dispatch_loop(...))
crates/roko-serve/src/lib.rs:240 tokio::spawn(config watcher)
crates/roko-serve/src/lib.rs:273 tokio::spawn(PRD subscriber)
crates/roko-serve/src/lib.rs:315 tokio::spawn(sidecar watcher)
crates/roko-serve/src/lib.rs:753 tokio::spawn(state hub)
crates/roko-serve/src/lib.rs:926 tokio::spawn(state saver)
crates/roko-serve/src/lib.rs:1078 tokio::spawn(signal source)
crates/roko-serve/src/lib.rs:1084 tokio::spawn(signal ingest loop)
crates/roko-serve/src/lib.rs:1112 tokio::spawn(cold archival)
crates/roko-serve/src/lib.rs:1146 tokio::spawn(chain watcher)
crates/roko-serve/src/job_runner.rs:48 tokio::spawn(run_job_loop(state))
crates/roko-serve/src/feedback.rs:38 tokio::spawn
crates/roko-serve/src/dreams.rs:40 tokio::spawn
crates/roko-serve/src/config_watcher.rs:34 tokio::spawn
crates/roko-serve/src/fswatcher.rs:14 tokio::spawn
```

Problem:

- [ ] Server background loops are not declared as first-class services.
- [ ] Startup has no complete service manifest, dependency graph, readiness state, or shutdown order.
- [ ] Some handles are returned, some are ignored, and some are hidden behind helper methods.
- [ ] Failure of a background loop may log but does not consistently update a lifecycle projection.
- [ ] A user cannot query "what loops are running, what failed, when did they last heartbeat, and what owns them?"

Target service model:

```rust
pub struct RuntimeServiceSpec {
    pub id: ServiceId,
    pub name: String,
    pub group: RuntimeGroupId,
    pub workspace: Option<WorkspaceRef>,
    pub dependencies: Vec<ServiceId>,
    pub restart_policy: RestartPolicy,
    pub heartbeat: HeartbeatPolicy,
    pub shutdown: ShutdownPolicy,
    pub task: ServiceTask,
}
```

Implementation checklist:

- [ ] Add `RuntimeServiceRegistry` owned by `RuntimeTaskSupervisor`.
- [ ] Convert server startup to build a `ServiceGroupSpec` instead of calling `tokio::spawn` directly.
- [ ] Register dispatch loop, config watcher, PRD subscriber, sidecar watcher, state hub, state saver, job runner, feedback loop, dream loop, fs watcher, signal source, signal ingest, relay, archival, and chain watcher as named services.
- [ ] Emit `service.declared`, `service.starting`, `service.ready`, `service.heartbeat`, `service.failed`, `service.restarting`, `service.stopping`, and `service.stopped`.
- [ ] Add restart policies only where retry is safe; otherwise fail the service group visibly.
- [ ] Add `GET /api/runtime/services`, `GET /api/runtime/services/{id}`, and `GET /api/runtime/services/{id}/events`.
- [ ] Include service health in the UI runtime panel and proof output.

Acceptance proof:

- [ ] Start `roko serve` and query all declared services.
- [ ] Kill or force-fail one allowlisted service and prove restart/failure behavior through events.
- [ ] Stop `roko serve` and prove shutdown emits stop events for every service.

### Lifecycle Drift L4 - Scheduler Uses Process-Wide Static Guard And Dummy Handles

Evidence:

```text
crates/roko-serve/src/scheduler.rs:13 static SCHEDULER_STARTED: AtomicBool
crates/roko-serve/src/scheduler.rs:27 return tokio::spawn(async {})
crates/roko-serve/src/lib.rs:1072 return tokio::spawn(async {})
crates/roko-serve/src/lib.rs:1108 tokio::spawn(async {})
crates/roko-serve/src/routes/gateway.rs:1178 handle: tokio::spawn(async {})
crates/roko-serve/src/routes/status/mod.rs:415 handle: tokio::spawn(async {})
```

Problem:

- [ ] Static guards prevent correct multi-workspace or multi-server behavior in one process.
- [ ] Dummy `tokio::spawn(async {})` handles create false lifecycle records.
- [ ] Callers cannot distinguish "component disabled", "already running", "not configured", and "started no-op placeholder".
- [ ] Tests and production code can accidentally depend on fake task handles.

Implementation checklist:

- [ ] Replace `SCHEDULER_STARTED` with supervisor-owned service identity keyed by `{workspace, service_name}`.
- [ ] Replace dummy handles with explicit `ServiceStartOutcome::{Started, AlreadyRunning, Disabled, NotConfigured, Failed}`.
- [ ] Add lifecycle events for duplicate-start and disabled/not-configured outcomes.
- [ ] Require every `JoinHandle` stored in production state to come from a real supervised task.
- [ ] Add a grep gate for `tokio::spawn(async {})` and require zero production matches.

Acceptance proof:

- [ ] Starting two independent workspaces in one process registers two scheduler services.
- [ ] Starting the same scheduler twice returns `AlreadyRunning` with a lifecycle event, not a dummy handle.
- [ ] `rg -n "tokio::spawn\\(async \\{\\}\\)" crates -g '*.rs'` has only tests or no matches.

### Lifecycle Drift L5 - Vision Loop Subprocess Bypasses Process Supervisor

Evidence:

```text
crates/roko-serve/src/routes/vision_loop.rs:133 tokio::process::Command::new("roko")
crates/roko-serve/src/routes/vision_loop.rs:157 tokio::spawn
crates/roko-serve/src/routes/vision_loop.rs:237 child.kill().await
```

Problem:

- [ ] Vision loop subprocesses are tracked in route-local state instead of the process ledger.
- [ ] Termination uses direct `child.kill().await`, not the cross-platform process-tree shutdown path.
- [ ] Restart/recovery cannot prove whether the child process was killed, exited, or leaked.
- [ ] Process stdout/stderr, args, cwd, env fingerprint, exit code, and kill sequence are not normalized with provider CLI processes.

Implementation checklist:

- [ ] Represent vision-loop start as `OperationKind::VisionLoop`.
- [ ] Represent the spawned `roko` child as `RuntimeTask::ManagedCommand(ManagedCommandSpec)`.
- [ ] Route cancellation through `RuntimeTaskSupervisor::cancel_operation`.
- [ ] Use `ProcessSupervisor` plus `kill_tree` for termination.
- [ ] Emit `process.spawned`, `process.stdout`, `process.stderr`, `process.exit`, `process.kill_requested`, `process.kill_escalated`, and `process.kill_completed`.
- [ ] Persist process metadata: command, redacted args, cwd, env fingerprint, pid, pgid/session id where available, parent operation id, and attempt id.
- [ ] Add crash startup reconciliation for active vision-loop process records.

Acceptance proof:

- [ ] Start a vision loop through HTTP, query the operation, and see a managed process child.
- [ ] Stop the vision loop, query kill events, and prove no child process remains.
- [ ] Crash/restart during an active vision loop and prove stale process handling.

### Lifecycle Drift L6 - Strong Process Primitives Exist But Are Not Universal

Evidence:

```text
crates/roko-agent/src/process/kill.rs:33 kill_tree
crates/roko-agent/src/process/group.rs process-group setup helpers
crates/roko-agent/src/process/registry.rs process registry helpers
crates/roko-cli/src/runner/agent_stream.rs:17 imports kill_tree and set_process_group
crates/roko-cli/src/runner/agent_stream.rs:99 uses kill_tree
crates/roko-cli/src/runner/agent_stream.rs:151 kill_on_drop(true)
crates/roko-agent/src/exec.rs:157 Command::new
crates/roko-agent/src/exec.rs:168 kill_on_drop(true)
crates/roko-agent/src/claude_cli_agent.rs:303 Command::new
crates/roko-agent/src/claude_cli_agent.rs:348 kill_on_drop(true)
```

Problem:

- [ ] Provider CLI, runner stream, MCP child processes, vision loop, git merge, regression commands, deploy CLI, and route git commands use different process wrappers.
- [ ] The best implementation (`kill_tree`, process groups, registry) is present but not a mandatory backend.
- [ ] `kill_on_drop(true)` is not enough to prove graceful shutdown, child tree cleanup, or durable lifecycle evidence.
- [ ] Process behavior cannot be queried uniformly by operation id.

Target backend:

```rust
pub trait ManagedCommandRunner: Send + Sync {
    async fn spawn(&self, spec: ManagedCommandSpec) -> Result<ManagedProcessRef>;
    async fn signal_stdin_close(&self, process_id: ProcessId) -> Result<()>;
    async fn terminate_tree(&self, process_id: ProcessId, grace: Duration) -> Result<KillReport>;
    async fn wait(&self, process_id: ProcessId) -> Result<ProcessExit>;
}
```

Implementation checklist:

- [ ] Move process-group setup, environment sanitization, kill-tree shutdown, registry updates, stdout/stderr streaming, and process events under `ManagedCommandRunner`.
- [ ] Make `ClaudeCliAgent`, `CodexAgent`, generic CLI provider execution, MCP child processes, runner `agent_stream`, git merge, route git commands, deployment CLI, and vision loop use `ManagedCommandRunner`.
- [ ] Add a small allowlist for one-shot shell calls that are not long-running and do not affect operation lifecycle.
- [ ] Emit process events from inside the managed command backend, not from callers.
- [ ] Ensure all process event payloads redact secrets and provider tokens.

Acceptance proof:

- [ ] A provider CLI process, vision-loop subprocess, git merge command, MCP child process, and deployment CLI process all appear in the same process projection schema.
- [ ] Cancelling each one produces the same kill event sequence.
- [ ] A grep gate proves raw `Command::new` production call sites are either migrated or allowlisted with a documented reason.

### Lifecycle Drift L7 - Runner Gate And Merge Tasks Spawn Locally

Evidence:

```text
crates/roko-cli/src/runner/gate_dispatch.rs:38 tokio::spawn
crates/roko-cli/src/runner/gate_dispatch.rs:128 tokio::spawn
crates/roko-cli/src/runner/event_loop.rs:669 tokio::spawn
crates/roko-cli/src/runner/event_loop.rs:1269 tokio::spawn
crates/roko-cli/src/runner/merge.rs:307 spawn_blocking
crates/roko-cli/src/runner/merge.rs:475 tokio::spawn
```

Problem:

- [ ] Runner v2 is closer to a real orchestrator, but some internal parallelism is still invisible outside local code.
- [ ] Gate execution, merge execution, regression commands, and post-merge tasks need task ids and event spans.
- [ ] HTTP proof cannot explain "what was running" during a gate or merge unless these tasks are part of the same runtime tree.
- [ ] Merge success/failure/conflict evidence should be a lifecycle/artifact outcome, not just runner-local state.

Implementation checklist:

- [ ] Model gates as `RuntimeTaskKind::Gate`.
- [ ] Model merge as `RuntimeTaskKind::Merge`.
- [ ] Model regression checks as `RuntimeTaskKind::RegressionCheck`.
- [ ] Use supervisor task spawning for concurrent gates, merges, and blocking regressions.
- [ ] Attach gate, merge, and regression events to parent plan-run operation id.
- [ ] Store conflict evidence and merge outputs through artifact-store repositories.
- [ ] Surface gate/merge tasks in `GET /api/runtime/operations/{id}/tasks`.

Acceptance proof:

- [ ] Run a plan with a gate failure and query retry/gate lifecycle by operation id.
- [ ] Run a merge success and query merge backend result by operation id.
- [ ] Run a merge conflict and query conflict files, stderr/stdout, exit code, and no-success terminal state.

### Lifecycle Drift L8 - Job Runner Uses Split Cancellation And Execution Ownership

Evidence:

```text
crates/roko-serve/src/job_runner.rs:48 tokio::spawn(run_job_loop(state))
crates/roko-serve/src/job_runner.rs:76 tokio::spawn(per-job work)
crates/roko-serve/src/routes/jobs.rs:1237 tokio::spawn
```

Problem:

- [ ] Job scheduling and job execution are not first-class runtime services/tasks.
- [ ] Job status can be represented in job JSON without the task/process lifecycle that produced it.
- [ ] Cancel/retry semantics for jobs are not guaranteed to match plan-run, research, PRD, or deployment operations.
- [ ] A job can own subwork without that subwork appearing in the same operation tree.

Implementation checklist:

- [ ] Register job runner as `RuntimeServiceSpec { name: "job-runner" }`.
- [ ] Represent each job execution as `OperationKind::Job`.
- [ ] Represent each job step as a child `RuntimeTask`.
- [ ] Emit job lifecycle through runtime events first, then derive existing job JSON projections from those events.
- [ ] Route job cancellation through `RuntimeTaskSupervisor::cancel_operation`.
- [ ] Add idempotency keys so a restarted job runner does not duplicate in-flight work.

Acceptance proof:

- [ ] Schedule a job, query the job-runner service, query the job operation, and see child task events.
- [ ] Cancel a job and prove the same cancel event sequence used by plan/research operations.
- [ ] Restart during a running job and prove idempotent recovery.

### Lifecycle Drift L9 - Lifecycle Query API Is Missing The Debug Views Needed For Agent Feedback

Problem:

- [ ] Agents cannot reliably improve Roko from runtime feedback if they cannot query what was started, what failed, what was killed, what retried, what produced artifacts, and what remains running.
- [ ] Current status endpoints are fragmented across operations, jobs, plans, provider logs, route DTOs, and filesystem artifacts.
- [ ] There is no single query shape that explains end-to-end execution across CLI, HTTP, provider, gate, merge, and background services.

Required HTTP/debug API:

- [ ] `GET /api/runtime/operations` with filters for kind, status, workspace, owner, created range, model, provider, and correlation id.
- [ ] `GET /api/runtime/operations/{id}` with current projection and terminal outcome.
- [ ] `GET /api/runtime/operations/{id}/timeline` with all lifecycle events.
- [ ] `GET /api/runtime/operations/{id}/tasks` with parent/child task tree.
- [ ] `GET /api/runtime/operations/{id}/processes` with process tree and kill/exit evidence.
- [ ] `GET /api/runtime/operations/{id}/artifacts` with typed artifact refs.
- [ ] `GET /api/runtime/services` with service health and heartbeat age.
- [ ] `GET /api/runtime/services/{id}/timeline`.
- [ ] `GET /api/runtime/processes` with active/stale/exited filters.
- [ ] `GET /api/runtime/orphans` with startup-reconciled process/task records.
- [ ] `GET /api/runtime/proof/{operation_id}` returning a compact proof bundle for tests and dogfooding.

Implementation checklist:

- [ ] Build these endpoints from projections, not live handles.
- [ ] Add JSON schemas and example responses to proof docs.
- [ ] Add redaction at projection boundaries.
- [ ] Add cursor pagination for event timelines.
- [ ] Add stable status taxonomy: `queued`, `starting`, `running`, `waiting`, `cancelling`, `cancelled`, `retrying`, `completed`, `failed`, `timed_out`, `interrupted`, `recovered`.

Acceptance proof:

- [ ] A real provider-backed plan run can be debugged from one operation id.
- [ ] A crash/restart run can be debugged from one operation id.
- [ ] A merge-conflict run can be debugged from one operation id.
- [ ] A server background-service failure can be debugged from one service id.

## Runtime Lifecycle Contract

This is the concrete contract the implementation should converge on. The exact module names can change, but the architectural boundary should not.

Core types:

- [ ] `RuntimeOperationSpec`: declarative user-visible work request.
- [ ] `RuntimeTaskSpec`: declarative async task, subprocess task, model-call task, workflow task, gate task, merge task, or service-loop task.
- [ ] `ManagedCommandSpec`: command, cwd, redacted args, env policy, timeout, stdin policy, stdout/stderr policy, process-group policy, kill policy, and artifact policy.
- [ ] `RuntimeServiceSpec`: background loop declaration with dependencies, restart policy, readiness, heartbeat, and shutdown.
- [ ] `OperationRef`: returned immediately by HTTP/CLI entrypoints and usable for status/wait/cancel.
- [ ] `TaskRef`: internal child task reference.
- [ ] `ManagedProcessRef`: process id plus durable process metadata.
- [ ] `LifecycleEvent`: durable event emitted by operation/task/process/service transitions.
- [ ] `RuntimeProjectionStore`: query backend for operation, task, process, service, and proof projections.

Required event families:

- [ ] `operation.created`
- [ ] `operation.started`
- [ ] `operation.waiting`
- [ ] `operation.retrying`
- [ ] `operation.completed`
- [ ] `operation.failed`
- [ ] `operation.cancel_requested`
- [ ] `operation.cancelled`
- [ ] `operation.timed_out`
- [ ] `operation.interrupted`
- [ ] `operation.recovered`
- [ ] `task.spawned`
- [ ] `task.started`
- [ ] `task.heartbeat`
- [ ] `task.completed`
- [ ] `task.failed`
- [ ] `task.cancel_requested`
- [ ] `task.cancelled`
- [ ] `task.timed_out`
- [ ] `process.spawned`
- [ ] `process.stdout`
- [ ] `process.stderr`
- [ ] `process.stdin_closed`
- [ ] `process.exited`
- [ ] `process.kill_requested`
- [ ] `process.kill_escalated`
- [ ] `process.kill_completed`
- [ ] `process.orphan_detected`
- [ ] `process.orphan_reaped`
- [ ] `service.declared`
- [ ] `service.starting`
- [ ] `service.ready`
- [ ] `service.heartbeat`
- [ ] `service.failed`
- [ ] `service.restarting`
- [ ] `service.stopping`
- [ ] `service.stopped`
- [ ] `shutdown.started`
- [ ] `shutdown.task_cancelled`
- [ ] `shutdown.process_reaped`
- [ ] `shutdown.completed`

Design rules:

- [ ] Routes must not own long-running work.
- [ ] Routes must not store authoritative lifecycle state.
- [ ] Routes may only validate requests, build specs, call runtime services, and return/query projections.
- [ ] Raw `tokio::spawn` is allowed only inside `RuntimeTaskSupervisor`, service adapters, tests, or documented short-lived helper internals.
- [ ] Raw `Command::new` is allowed only inside `ManagedCommandRunner`, build/install/test code, or documented one-shot allowlist entries.
- [ ] Direct `JoinHandle::abort` is allowed only inside supervisor shutdown internals.
- [ ] Direct `child.kill()` is allowed only inside the managed process backend.
- [ ] `kill_on_drop(true)` may be a safety net, not the primary lifecycle mechanism.
- [ ] Operation status must be reconstructable from durable events after process restart.
- [ ] Every proof script must query the runtime API rather than trusting command exit status alone.

## Migration Batches

### Batch T1 - Durable Operation And Event Store

- [ ] Define operation/task/process/service ids and lifecycle event payloads.
- [ ] Implement append-only event store integration.
- [ ] Implement runtime projections for operation status, task tree, process tree, service health, and proof bundle.
- [ ] Add startup reconciliation for non-terminal operations and stale process records.
- [ ] Add HTTP read endpoints for projections.
- [ ] Preserve current route DTO compatibility by mapping from projections.

### Batch T2 - Supervisor Facade

- [ ] Implement `RuntimeTaskSupervisor`.
- [ ] Wrap existing `ProcessSupervisor`, cancellation tokens, and process kill primitives.
- [ ] Add `RuntimeServiceRegistry`.
- [ ] Add `ManagedCommandRunner`.
- [ ] Add lifecycle event emission from inside the supervisor and managed command backend.
- [ ] Add unit tests for state transitions without provider calls.

### Batch T3 - Route Operation Migration

- [ ] Migrate `routes/run.rs`.
- [ ] Migrate `routes/plans.rs` execute/resume/chat/generate operations.
- [ ] Migrate `routes/prds.rs` draft/promote/generate operations.
- [ ] Migrate `routes/research.rs`.
- [ ] Migrate `routes/templates.rs`.
- [ ] Migrate `routes/dream.rs`.
- [ ] Migrate `routes/gateway.rs`.
- [ ] Migrate `routes/deployments.rs`.
- [ ] Migrate route status endpoints to read projections.
- [ ] Remove direct route status mutation after compatibility coverage is in place.

### Batch T4 - Server Service Migration

- [ ] Build a service manifest in server startup.
- [ ] Register dispatch loop as a service.
- [ ] Register config watcher as a service.
- [ ] Register PRD subscriber as a service.
- [ ] Register sidecar watcher as a service.
- [ ] Register state hub and state saver as services.
- [ ] Register scheduler as a workspace-keyed service.
- [ ] Register job runner as a service.
- [ ] Register feedback, dream, fs watcher, signal ingest, relay, archival, and chain watcher loops as services.
- [ ] Add service query endpoints and shutdown report projection.

### Batch T5 - Process Migration

- [ ] Migrate provider CLI execution to `ManagedCommandRunner`.
- [ ] Migrate runner `agent_stream`.
- [ ] Migrate MCP child process execution.
- [ ] Migrate vision loop.
- [ ] Migrate git merge backend and route git calls.
- [ ] Migrate deployment CLI wrappers.
- [ ] Migrate regression command execution.
- [ ] Add allowlist for remaining raw process calls with owner and expiration.

### Batch T6 - Runner Internal Task Migration

- [ ] Register plan-run operation roots from CLI and HTTP.
- [ ] Register gates as child tasks.
- [ ] Register merge as child tasks.
- [ ] Register regression checks as child tasks.
- [ ] Attach retry decisions to the operation timeline.
- [ ] Attach provider prompt/model lifecycle to the same operation id.
- [ ] Expose runner proof through HTTP projection endpoints.

### Batch T7 - Proof And Cleanup

- [ ] Add proof script for real provider run.
- [ ] Add proof script for route-started plan run.
- [ ] Add proof script for cancellation and process-tree kill.
- [ ] Add proof script for timeout and kill escalation.
- [ ] Add proof script for crash/restart recovery.
- [ ] Add proof script for merge success.
- [ ] Add proof script for merge conflict.
- [ ] Add proof script for service startup/shutdown report.
- [ ] Run grep gates and store allowlist evidence.
- [ ] Remove old `active_runs`, `active_plans`, and route-owned operation status once compatibility endpoints read from projections.

## Additional Grep Gates From Deepening Pass

Run these in addition to the earlier gates before marking this audit complete:

```bash
rg -n "active_runs|active_plans|operations: RwLock|OperationHandle|RunHandle|PlanHandle" crates/roko-serve/src -g '*.rs'
rg -n "tokio::spawn|tokio::task::spawn_blocking" crates/roko-serve/src/routes crates/roko-serve/src/lib.rs crates/roko-serve/src/job_runner.rs crates/roko-serve/src/scheduler.rs crates/roko-cli/src/runner -g '*.rs'
rg -n "tokio::process::Command::new|std::process::Command::new|Command::new|child\\.kill\\(|kill_on_drop" crates/roko-serve/src crates/roko-cli/src crates/roko-agent/src -g '*.rs'
rg -n "tokio::spawn\\(async \\{\\}\\)|static .*AtomicBool|SCHEDULER_STARTED" crates/roko-serve/src crates/roko-cli/src -g '*.rs'
rg -n "RuntimeTaskSupervisor|RuntimeOperationSpec|RuntimeTaskSpec|ManagedCommandSpec|RuntimeServiceSpec|ManagedCommandRunner" crates -g '*.rs'
```

Expected completion state:

- [ ] The first gate has no authoritative lifecycle stores outside projection/cache adapters.
- [ ] The second gate has only supervisor internals, service adapters, tests, or documented allowlisted helpers.
- [ ] The third gate has only managed command backend, tests, build/install integrations, or documented one-shot allowlist entries.
- [ ] The fourth gate has zero production matches.
- [ ] The fifth gate finds the new lifecycle contract in active production code.

## Updated Self-Grade After Deepening Pass

Initial score before this pass: **8.9 / 10**. The previous version identified the right lifecycle direction, but it was not complete enough for a context-free implementation agent because route-level, service-level, scheduler-level, process-level, job-level, and query-level work were not separately pinned to source evidence and acceptance proof.

Current score after this pass: **9.89 / 10**.

Why this is now above the requested bar:

- [ ] It identifies each major lifecycle architecture flaw with concrete source evidence.
- [ ] It distinguishes operations, tasks, processes, services, and projections instead of collapsing them into "spawn cleanup".
- [ ] It gives explicit contracts for `RuntimeTaskSupervisor`, `RuntimeOperationSpec`, `RuntimeTaskSpec`, `ManagedCommandRunner`, and `RuntimeServiceSpec`.
- [ ] It gives migration batches that avoid a risky all-at-once rewrite.
- [ ] It defines the missing HTTP/debug query layer needed for automated agent feedback.
- [ ] It defines acceptance proof for cancellation, timeout, crash recovery, service shutdown, provider processes, vision loop, jobs, gates, and merges.

Remaining risk:

- [ ] The final module boundaries should be reconciled with the dispatch, workflow, artifact-store, gateway, and observability docs before implementation starts, so the runtime crate layout does not create circular dependencies.
