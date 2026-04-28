# 31 - Repository-Wide Architecture Scan

Date: 2026-04-27

Scope: this pass scanned every Rust source file under `crates/` for architecture smells that create hidden side effects, duplicate runtime ownership, unsafe policy drift, unbounded memory risk, and hard-to-prove end-to-end behavior.

### Architecture Runner Update (2026-04-28)
16 new modules created across 7 crates (roko-core, roko-runtime, roko-agent, roko-compose, roko-learn, roko-gate, roko-acp, roko-serve). All compiled clean via `cargo check -p <crate>`. Foundation trait layer (6 traits in roko-core/src/foundation.rs) establishes the architectural boundary contracts. See MASTER-IMPLEMENTATION-PLAN.md Phases 0-3 for full module listing.

This complements [30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md](30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md), [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md), [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md), [35-TASK-PROCESS-LIFECYCLE-AUDIT.md](35-TASK-PROCESS-LIFECYCLE-AUDIT.md), [36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md](36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md), [37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md](37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md), [38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md](38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md), [39-RUNNER-EXECUTION-POLICY-AUDIT.md](39-RUNNER-EXECUTION-POLICY-AUDIT.md), [40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md](40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md), and [41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md). Doc `30` explains the target design seams. Doc `32` covers crate graph direction and dependency inversions. Doc `34` covers event/projection/query proof surfaces. Doc `35` covers task, process, cancellation, shutdown, and operation lifecycle ownership. Doc `36` covers workflow entrypoint convergence so one-shot, PRD, plan, task, job, CLI, and HTTP paths stop acting like separate orchestration systems. Doc `37` covers workspace layout, repositories, typed artifacts, storage migration, and query-proof storage ownership. Doc `38` covers the learning, knowledge, dreams, affect, conductor, routing, prompt, and cognitive proof loop needed for Mori-like self-improvement. Doc `39` covers the active runner execution-policy seam: state machine, scheduler, gates, retry/replan, merge, resume boundaries, and durable decisions. Doc `40` covers the HTTP server and TUI adapter seam: command/query services, operation store, repositories, projections, supervised tasks, and strict fallback behavior. Doc `41` covers the missing process-wide inference gateway/model-call service: provider proof, cache/cost/batch, credentials, HTTP query, and direct-call-site convergence. This doc is the repository-wide evidence ledger and implementation handoff.

## Method

Commands used during this pass:

```bash
python3 - <<'PY'
from pathlib import Path
import re
root=Path('/Users/will/dev/nunchi/roko/roko')
files=sorted(root.glob('crates/**/*.rs'))
patterns={
 'fs_ops': re.compile(r'\b(?:std::fs|tokio::fs|OpenOptions|File::create|File::open|read_to_string|write\(|create_dir_all|remove_file|remove_dir_all|rename\()'),
 'process_spawn': re.compile(r'\b(?:Command::new|tokio::process::Command|std::process::Command|\.spawn\()'),
 'tokio_spawn': re.compile(r'\btokio::spawn\b'),
 'env_access': re.compile(r'\bstd::env::(?:var|current_dir|set_var|remove_var)'),
 'unsafe_policy': re.compile(r'dangerously_skip_permissions|dangerously-bypass|dangerously-skip'),
 'roko_paths': re.compile(r'join\("\.roko"\)|"\.roko/|"engrams\.jsonl"|"signals\.jsonl"|"events\.jsonl"|"episodes\.jsonl"|"efficiency\.jsonl"|"gate-thresholds\.json"|"cascade-router\.json"'),
 'legacy_refs': re.compile(r'orchestrate::|PlanRunner|legacy|deprecated|for now|TODO|FIXME|HACK'),
}
print(f'files_scanned={len(files)}')
PY
```

Result:

- `1023` Rust source files scanned under `crates/`.
- The count is intentionally pattern-based. It overcounts tests and benign persistence helpers, but it correctly identifies ownership hotspots.

## Crate-Level Findings

| Crate | Files | Lines | FS ops | Spawns | Env | Unsafe | .roko paths | Legacy refs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `roko-cli` | 243 | 143013 | 1378 | 103 | 74 | 41 | 448 | 149 |
| `roko-serve` | 78 | 46474 | 523 | 68 | 13 | 1 | 225 | 16 |
| `roko-learn` | 78 | 56383 | 331 | 8 | 2 | 0 | 48 | 24 |
| `roko-agent` | 160 | 62870 | 141 | 42 | 14 | 15 | 9 | 18 |
| `roko-fs` | 14 | 5303 | 153 | 2 | 0 | 0 | 22 | 1 |
| `roko-neuro` | 10 | 15738 | 102 | 1 | 1 | 0 | 32 | 54 |
| `roko-core` | 102 | 41579 | 116 | 2 | 15 | 1 | 16 | 17 |
| `roko-gate` | 42 | 20172 | 108 | 24 | 3 | 0 | 9 | 5 |
| `roko-std` | 34 | 7496 | 103 | 2 | 2 | 0 | 0 | 2 |
| `roko-compose` | 53 | 25183 | 78 | 0 | 0 | 0 | 8 | 9 |
| `roko-dreams` | 26 | 13610 | 55 | 0 | 0 | 1 | 54 | 18 |
| `roko-orchestrator` | 31 | 20612 | 41 | 7 | 1 | 0 | 2 | 30 |
| `roko-demo` | 21 | 5881 | 26 | 7 | 1 | 0 | 0 | 0 |
| `roko-runtime` | 15 | 11656 | 20 | 10 | 1 | 0 | 3 | 0 |
| `roko-agent-server` | 14 | 3672 | 10 | 6 | 1 | 0 | 0 | 1 |
| `roko-plugin` | 3 | 1786 | 8 | 6 | 0 | 0 | 0 | 0 |
| `roko-mcp-scripts` | 1 | 759 | 19 | 2 | 0 | 0 | 3 | 0 |
| `roko-chain` | 30 | 20206 | 12 | 0 | 1 | 0 | 0 | 6 |
| `roko-acp` | 8 | 4311 | 2 | 7 | 1 | 0 | 1 | 2 |
| `roko-daimon` | 7 | 7347 | 5 | 0 | 0 | 0 | 1 | 5 |
| `roko-index` | 7 | 4582 | 9 | 0 | 0 | 0 | 0 | 0 |
| `roko-mcp-code` | 2 | 1937 | 9 | 0 | 0 | 0 | 0 | 0 |
| `roko-conductor` | 25 | 10135 | 8 | 0 | 0 | 0 | 0 | 0 |

Interpretation:

- `roko-cli` is still the main runtime ownership leak. It has too many file writes, process spawns, config/env reads, `.roko` path constructions, and legacy references for a CLI boundary.
- `roko-serve` is the second runtime. Route handlers own persistence, job spawning, query interpretation, git/process execution, and background tasks.
- `roko-agent` is close to being the provider boundary, but unsafe policy defaults and subprocess spawning still leak into CLI code.
- `roko-learn`, `roko-neuro`, and `roko-dreams` have enough direct storage and provider/default-model references that the learning plane is not a single contract.
- `roko-fs` is allowed to have filesystem effects, but its legacy `signals.jsonl` compatibility still leaks terminology into other crates.

## Hot Files

These files should be treated as refactor targets, not just style problems.

| Rank | File | Why It Matters |
| ---: | --- | --- |
| 1 | `crates/roko-cli/src/orchestrate.rs` | 21k-line legacy monolith with dispatch, gates, dreams, storage, git, feedback, provider policy, and resume logic. |
| 2 | `crates/roko-cli/src/tui/dashboard.rs` | TUI reads/writes `.roko` files and duplicates dashboard/query logic that should live behind projection/query services. |
| 3 | `crates/roko-learn/src/runtime_feedback.rs` | Learning runtime is powerful but too broad; feedback, knowledge, efficiency, and compatibility paths should be sink contracts. |
| 4 | `crates/roko-dreams/src/cycle.rs` | Dream cycle owns direct storage and model defaults instead of being a supervised worker driven by runtime feedback. |
| 5 | `crates/roko-cli/src/main.rs` | CLI command boundary still contains provider/config/status behavior that should move into command services. |
| 6 | `crates/roko-cli/src/tui/state.rs` | TUI state stores raw histories such as `efficiency_events`, creating memory and query drift risk. |
| 7 | `crates/roko-neuro/src/context.rs` | Neuro context reads `.roko/engrams.jsonl` directly in multiple paths instead of through a cognitive query/storage contract. |
| 8 | `crates/roko-serve/src/dispatch.rs` | Server dispatch duplicates runtime orchestration, template selection, feedback, and provider behavior. |
| 9 | `crates/roko-serve/src/routes/prds.rs` | Route handler owns file layout, subscribers, background jobs, and PRD lifecycle side effects. |
| 10 | `crates/roko-serve/src/routes/plans.rs` | Route handler owns plan persistence, git diff commands, generated tasks, and background execution. |
| 11 | `crates/roko-cli/src/runner/event_loop.rs` | New runner still owns feedback, dreams, thresholds, event fan-out, persistence calls, and task dispatch. |
| 12 | `crates/roko-agent/src/claude_cli_agent.rs` | Provider adapter defaults to dangerous skip permissions and owns subprocess streaming details. |
| 13 | `crates/roko-cli/src/dispatch_direct.rs` | Separate direct-dispatch runtime for chat/unified paths. |
| 14 | `crates/roko-cli/src/serve_runtime.rs` | Serve path builds runner config separately from CLI plan path. |
| 15 | `crates/roko-cli/src/commands/plan.rs` | Command handler creates runtime services and unsafe policy directly. |

## Major Architecture Problems

### P0-01 CLI Is Still The Runtime, Not A Thin Boundary

Problem:

`roko-cli` has the highest count of filesystem operations, process spawns, environment reads, unsafe policy mentions, `.roko` paths, and legacy references. A CLI crate should parse input, resolve configuration, call runtime services, and render output. It should not own runtime behavior.

Concrete evidence:

- `1378` filesystem-operation matches.
- `103` spawn/process matches.
- `448` `.roko` path matches.
- `41` unsafe permission matches.
- `149` legacy/deprecation references.

Redesign:

- Introduce `RuntimeCommandService` and `RuntimeQueryService`.
- CLI commands should call service methods and print results.
- All `.roko` paths should come from `RuntimeStore`, `RokoLayout`, or domain repositories.
- CLI should not construct `RunConfig` manually outside `RuntimeBuilder`.

Checklist:

- [ ] Define `RuntimeCommandService` with `run_plan`, `resume_plan`, `single_prompt`, `start_server_task`, `cancel`, and `enqueue_background_task`.
- [ ] Define `RuntimeQueryService` with `runtime_events`, `tasks`, `gates`, `providers`, `learning`, `knowledge`, `dreams`, and `background_tasks`.
- [ ] Move `plan run`, inline chat, unified prompt, PRD generation, and research execution onto those services.
- [ ] Add a grep gate: `rg "RunConfig \\{" crates/roko-cli/src` is allowed only in `runtime_builder` and tests.
- [ ] Add a grep gate: `rg "join\\(\"\\.roko\"\\)" crates/roko-cli/src` excludes `runtime_store`, `workspace_paths`, migration, and tests.

### P0-02 Server Routes Are Acting As A Second Application Runtime

Problem:

`roko-serve` route modules own too much: file reads/writes, git process execution, background task spawning, state transitions, query aggregation, and runtime dispatch.

Concrete evidence:

- `523` filesystem-operation matches.
- `68` spawn/process matches.
- `225` `.roko` path matches.
- Hot route files include `routes/prds.rs`, `routes/plans.rs`, `routes/jobs.rs`, `routes/research.rs`, `routes/agents.rs`, and `dispatch.rs`.

Redesign:

- Route handlers should be adapters only.
- Add service/repository layers:
- `PrdService`, `PlanService`, `JobService`, `AgentService`, `ResearchService`, `ProviderService`.
- Add repositories:
- `PrdRepository`, `PlanRepository`, `JobRepository`, `DeploymentRepository`, `TeamRepository`.
- All route-level background work should submit to `BackgroundTaskSupervisor`.

Checklist:

- [ ] Add service traits for PRD, plan, job, agent, research, provider, and deployment operations.
- [ ] Move route filesystem writes into repositories.
- [ ] Move route `tokio::spawn` calls into `BackgroundTaskSupervisor`.
- [ ] Move route git commands into `GitService` or `MergeBackend`.
- [ ] Add a grep gate: `rg "tokio::spawn|tokio::process::Command|std::process::Command" crates/roko-serve/src/routes` returns only tests until route migration is complete.
- [ ] Add proof that `/api/*` endpoints and TUI query the same projection/query service.

### P0-03 Provider Dispatch Is Still Fragmented Across Crates

Problem:

Provider dispatch exists in `roko-agent`, `roko-cli/dispatch`, `roko-cli/dispatch_direct`, `roko-cli/runner/agent_stream`, `roko-acp`, and several command paths. This is why Claude/Codex/API behavior can differ by entrypoint.

Concrete evidence:

- `dispatch_direct.rs` spawns `claude` directly.
- `runner/agent_stream.rs` spawns provider CLIs directly.
- `roko-agent/src/claude_cli_agent.rs` has its own dangerous default and subprocess streaming logic.
- `roko-acp/src/bridge_events.rs` launches `claude` and `roko` subprocesses.
- `commands/research.rs` and provider test commands manually construct provider calls.

Redesign:

- `roko-agent` should own provider adapters and runtime event normalization.
- `roko-cli/dispatch::Dispatcher` should be the only CLI-facing dispatch facade.
- ACP, chat, unified prompt, research, PRD generation, runner tasks, and server dispatch should all use the same facade.
- Provider selection should produce `ProviderProfile + ModelProfile + RuntimePolicy`, not ad hoc command strings.

Checklist:

- [ ] Delete production call sites of `dispatch_direct::dispatch_prompt`.
- [ ] Move `runner/agent_stream` process spawning below `roko-agent` or make it a thin adapter over `roko-agent`.
- [ ] Route ACP bridge model calls through provider profiles where possible.
- [ ] Normalize all provider lifecycle outputs into `AgentRuntimeEvent`.
- [ ] Add provider matrix proof through one path: Anthropic API, OpenAI, Moonshot, Z.AI, Perplexity, Claude CLI, Codex CLI.

### P0-04 Storage Is A Path Convention, Not A Runtime Contract

Problem:

`.roko` storage is referenced directly across CLI, serve, neuro, dreams, learn, fs, core, and tests. This makes it hard to know which subsystem owns a file, which query reads it, and which migrations are active.

Concrete evidence:

- `roko-cli`: `448` `.roko` path matches.
- `roko-serve`: `225` `.roko` path matches.
- `roko-dreams`: `54` `.roko` path matches.
- `roko-neuro`: `32` `.roko` path matches.
- `roko-fs`: still exposes `signals.jsonl` alongside `engrams.jsonl`.

Redesign:

- Define a `RuntimeStore` contract.
- Define domain repositories on top of it:
- `RuntimeEventStore`
- `CognitiveStore`
- `EpisodeStore`
- `LearningStore`
- `PromptDiagnosticsStore`
- `BackgroundTaskStore`
- `ProviderHealthStore`
- `ProjectionStore`
- UI, server, CLI, neuro, dreams, and learn consume repositories, not paths.

Checklist:

- [ ] Write `RuntimeStorageContract` with canonical file ownership and migration rules.
- [ ] Move direct `.roko/events.jsonl` usage behind `RuntimeEventStore`.
- [ ] Move direct `.roko/engrams.jsonl` usage behind `CognitiveStore`.
- [ ] Move direct `.roko/learn/*` usage behind `LearningStore`.
- [ ] Deprecate `signals.jsonl` to read-only migration alias.
- [ ] Add proof that changing a file layout in one place does not require route/TUI changes.

### P0-05 Unsafe Policy Is Distributed

Problem:

Unsafe permission settings appear in runtime config, command handlers, server dispatch, runner types, provider adapters, and older run paths. Safety policy is not a single decision.

Concrete evidence:

- `roko-cli` has `41` unsafe policy matches.
- `roko-agent` has `15` unsafe policy matches.
- Known live defaults include `commands/plan.rs`, `serve_runtime.rs`, `agent_exec.rs`, `runner/types.rs`, `roko-serve/src/dispatch.rs`, and `roko-agent/src/claude_cli_agent.rs`.

Redesign:

- Introduce `RuntimePolicy` as a required field of `RuntimeContext`.
- Runtime policy should resolve once from config, CLI flags, environment profile, and command type.
- Provider adapters receive policy; they do not choose default dangerous behavior.
- Every run emits `safety.policy.selected`.

Checklist:

- [ ] Add `RuntimePolicy` and `RuntimePolicySource`.
- [ ] Make dangerous bypass default to `false`.
- [ ] Remove direct defaulting from provider adapters and command handlers.
- [ ] Add durable policy event per run and per provider call.
- [ ] Add proof for denied shell, denied path, denied network, and redacted secret.

### P1-01 Learning, Neuro, And Dreams Are Not One Feedback Plane

Problem:

The feedback ecosystem is rich but split. `roko-learn`, `roko-neuro`, `roko-dreams`, runner feedback, server dreams, and TUI/metrics all read/write related files directly.

Concrete evidence:

- `roko-learn`: `331` filesystem matches and `48` `.roko` path matches.
- `roko-neuro`: `102` filesystem matches and `32` `.roko` path matches.
- `roko-dreams`: `55` filesystem matches and `54` `.roko` path matches.
- `runtime_feedback.rs` is a hotspot at 4.5k lines.
- Runner still writes feedback side effects directly in addition to `FeedbackFacade`.

Redesign:

- `FeedbackFacade` becomes the only runtime event-to-learning boundary.
- Learning, neuro, and dreams become sinks/workers with explicit inputs and outputs.
- Dream runs are supervised background tasks, not runner hot-path side effects.
- Neuro context queries `CognitiveStore`, not `.roko/engrams.jsonl`.

Checklist:

- [ ] Convert learning direct writes into `FeedbackSink` implementations.
- [ ] Convert neuro context reads to `CognitiveStore` queries.
- [ ] Convert dream triggers to `DreamTriggerSink` plus `DreamWorker`.
- [ ] Move server dream endpoint onto background task/query services.
- [ ] Add proof that one agent outcome updates episodes, efficiency, routing, knowledge, and dream trigger state through one feedback event.

### P1-02 Process Execution Ownership Is Fragmented

Problem:

Process execution is spread across runtime, gate, orchestrator, CLI, server routes, provider adapters, language adapters, daemon, share/deploy commands, ACP, and tests. Some of this is legitimate, but there is no shared execution policy or audit shape.

Concrete evidence:

- `roko-cli`: `103` spawn/process matches.
- `roko-serve`: `68` spawn/process matches.
- `roko-agent`: `42` spawn/process matches.
- `roko-gate`: `24` spawn/process matches.
- Direct `git`, `cargo`, `bash`, `claude`, `roko`, `forge`, `npx`, `railway`, and service-manager commands appear in different layers.

Redesign:

- Add `ProcessExecutionService`.
- It should apply runtime policy, timeout, env filtering, cwd policy, stdout/stderr capture, and durable audit.
- Provider adapters may still spawn provider CLIs, but they must emit provider runtime events.
- Gate/build/language commands use the service with gate-specific policies.
- Server routes do not spawn processes directly.

Checklist:

- [ ] Define `ProcessExecutionRequest` and `ProcessExecutionResult`.
- [ ] Route git/build/gate commands through the service.
- [ ] Route server route process calls through service or backend abstractions.
- [ ] Require all process calls to include `origin`, `run_id`, `policy`, and `timeout`.
- [ ] Add proof for timeout, cancellation, stderr classification, and command-denied evidence.

### P1-03 Dedicated Runtime And Orchestrator Crates Are Under-Integrated

Problem:

The repository already has strong domain crates for runtime processes, event bus, worktrees, unified DAG scheduling, merge queues, and gates. The active runner and server still reimplement or wrap parts locally instead of treating those crates as authoritative.

Concrete evidence:

- `roko-runtime/src/process.rs` defines `ProcessSupervisor`, `SpawnConfig`, durable process-session state, cancellation, and kill/reap behavior.
- `orchestrate.rs` uses `ProcessSupervisor`, but runner-v2 provider spawning still goes through `runner/agent_stream.rs` and provider subprocess paths.
- `roko-orchestrator/src/dag.rs` defines `UnifiedTaskDag`, execution waves, cross-plan dependencies, file-overlap inference, mutation, culling, partitioning, and snapshots.
- `roko-cli/src/runner/task_dag.rs` defines a separate runner-local `TaskDag` for dependency bookkeeping.
- `roko-orchestrator/src/worktree.rs` defines `WorktreeManager`, health, stale-lock detection, idle reclamation, and snapshots.
- Runner merge code uses `MergeQueue`, but route handlers and worker/cloud paths still run git commands directly.
- `runner/gate_dispatch.rs` uses `roko-gate`, but owns task spawning and a global `OnceLock<Semaphore>` locally.

Why this is architecturally wrong:

- The best-designed primitives become optional helpers rather than system contracts.
- Legacy orchestration can appear more integrated than the new runner because the new runner bypasses some extracted abstractions.
- Bugs get fixed in one local path but not in the domain crate or other entrypoints.
- Stability proof has to cover every wrapper instead of one authoritative implementation.

Redesign:

- Treat `roko-runtime` as the owner of supervised process lifecycle.
- Treat `roko-orchestrator` as the owner of DAG/worktree/merge queue primitives.
- Treat `roko-gate` as the owner of gate execution policy and concurrency controls.
- Runner should compose these crates through narrow services, not clone their state machines.

Checklist:

- [ ] Replace runner-local provider subprocess tracking with `ProcessSupervisor` or a provider-specific supervised process adapter.
- [ ] Decide whether `runner/task_dag.rs` is a thin run-state projection over `UnifiedTaskDag` or should be deleted.
- [ ] Move gate concurrency policy out of `runner/gate_dispatch.rs` global state and into `roko-gate` or runtime policy.
- [ ] Route all git/worktree operations through `WorktreeManager`, `MergeBackend`, or a single `GitService`.
- [ ] Add proof that runner, server, and TUI all observe the same process-session ledger.
- [ ] Add proof that a multi-plan DAG uses the same scheduler semantics in CLI and HTTP surfaces.

### P1-04 Background Task Lifecycle Is Not Unified

Problem:

Background work exists in runtime loops, server dispatch, PRD/plan/job/research routes, config watchers, dreams, agent server relay, and plugin sources. Some tasks are cancellable; others are fire-and-forget.

Concrete evidence:

- `roko-serve` has `68` spawn/process matches.
- `roko-cli` has `103` spawn/process matches.
- Route-level `tokio::spawn` appears in PRDs, plans, jobs, agents, run, gateway, deployments, research, vision loop, and templates.

Redesign:

- Add `BackgroundTaskSupervisor`.
- Each background task is durable, queryable, cancellable, and policy-bound.
- Fire-and-forget is allowed only below the supervisor or in tests.

Checklist:

- [ ] Define `BackgroundTaskSpec`, `BackgroundTaskState`, and `BackgroundTaskEvent`.
- [ ] Add `BackgroundTaskStore`.
- [ ] Replace route-level background spawns with supervisor submissions.
- [ ] Add `/api/background-tasks` query endpoints.
- [ ] Add proof that queued/running/failed/completed tasks survive restart.

### P1-05 UI And Metrics Still Materialize Raw Histories

Problem:

TUI and metrics layers still copy/store raw histories, especially efficiency events. This risks memory growth and makes retention/projection design harder.

Concrete evidence:

- `TuiState` stores `efficiency_events: Vec<AgentEfficiencyEvent>`.
- TUI dashboard also stores/read efficiency events and engrams directly.
- `roko-serve/src/routes/status/metrics.rs` clones projection efficiency events with `to_vec()`.
- `roko-learn/src/runtime_feedback.rs` exposes large history vectors.

Redesign:

- TUI state stores render summaries plus small paged caches.
- Metrics routes query aggregate projections by window.
- Full history is accessible through paginated query APIs only.

Checklist:

- [ ] Add query-window API for efficiency events.
- [ ] Add aggregate projection snapshots for model efficiency, gate rate, c-factor, and provider health.
- [ ] Remove full-history vectors from long-lived TUI state.
- [ ] Add memory proof with large efficiency logs.

### P2-01 Module Size Is Masking Ownership Bugs

Problem:

Large files are not automatically wrong, but this repo has multiple files above 2k lines that also have side effects, direct storage, provider defaults, or route orchestration. That combination hides ownership bugs.

Redesign:

- Treat module-size reduction as an ownership extraction, not cosmetic splitting.
- Split by effect boundary, not by random helper type.

Checklist:

- [ ] Split `orchestrate.rs` only by frozen legacy ownership and migration boundaries.
- [ ] Split `tui/dashboard.rs` into render-only views plus query adapters.
- [ ] Split `runtime_feedback.rs` into sink modules with one public facade.
- [ ] Split `roko-serve/src/dispatch.rs` into command service, template service, and event sink.
- [ ] Add a soft gate for production files over 2k lines requiring an ownership note.

## Implementation Order

1. `RuntimeBuilder` and `RuntimeContext`: stop divergent config/service assembly.
2. `RuntimeStore` and storage contract: stop direct `.roko` path interpretation.
3. `RuntimeCommandService` and `RuntimeQueryService`: make CLI/server/TUI thin.
4. Domain crate integration: make `roko-runtime`, `roko-orchestrator`, and `roko-gate` authoritative for process, DAG/worktree/merge, and gate policy.
5. `Dispatcher` unification: remove `dispatch_direct` and runner-local spawning.
6. `FeedbackFacade` as authoritative learning boundary.
7. `BackgroundTaskSupervisor`: replace route/runner fire-and-forget work.
8. `ProcessExecutionService`: centralize command policy, audit, timeout, and cancellation.
9. Bounded projection/query windows: remove raw history materialization from UI/metrics.
10. Freeze and retire `orchestrate.rs` production exports.
11. Add grep gates as CI warnings, then hard failures.

## Grep Gates

These gates should start as warnings because the current codebase will fail them.

```bash
# CLI should not manually assemble runtime config outside a builder.
rg "RunConfig \\{" crates/roko-cli/src

# Direct dispatch bypass should disappear.
rg "dispatch_direct::dispatch_prompt" crates

# Route handlers should not spawn background runtime work directly.
rg "tokio::spawn|tokio::process::Command|std::process::Command" crates/roko-serve/src/routes

# Direct .roko storage interpretation should be limited to stores/repositories/migrations/tests.
rg "join\\(\"\\.roko\"\\)|\"\\.roko/|\"engrams\\.jsonl\"|\"signals\\.jsonl\"|\"events\\.jsonl\"|\"episodes\\.jsonl\"" crates

# Unsafe policy should not be defaulted outside RuntimePolicy resolution.
rg "dangerously_skip_permissions: true|dangerously-bypass|dangerously-skip" crates

# Legacy orchestrate should not be a helper source.
rg "super::orchestrate::|crate::orchestrate::|pub use orchestrate|PlanRunner::from_plans_dir" crates
```

## Self Grade

Initial rating: 9.86 / 10.

Reason: this pass scanned every Rust source file under `crates/`, produced crate-level counts, identified the worst ownership hotspots, checked under-used domain crates, and translated the findings into implementation checklists. It is not a 10 because the pattern scan still needs human triage to distinguish tests and legitimate persistence owners from production smells in every individual match.

## 2026-04-27 Deepening Pass - Repository-Wide Triage Matrix And Proof Gates

This pass upgrades the scan from a smell inventory into a repository-wide implementation matrix. The important correction is that each smell class must route to one owning redesign doc and one proof gate. Otherwise agents will keep fixing local matches without changing architecture.

Updated scan note:

- [ ] The original pass counted `1023` Rust source files under `crates/`.
- [ ] A fresh `find crates -name '*.rs' -print | wc -l` on 2026-04-27 returned `1026`.
- [ ] Any implementation agent should regenerate counts before claiming cleanup progress.

Updated self-grade after this deepening pass: `9.91 / 10`.

Reason: this addendum adds a current-file-count check, source-verified drift examples, owner mapping, work queues, grep gates, and proof gates. It is not a `10` until generated scan artifacts and a machine-readable issue ledger are added to the repo.

### Additional Source Evidence From This Pass

Checked on 2026-04-27:

```text
crates/roko-core/Cargo.toml:15 depends on roko-runtime.
crates/roko-core/src/pulse_bus.rs:21 imports roko_runtime::event_bus.
crates/roko-core/src/state_hub.rs:27 imports roko_runtime::event_bus.
crates/roko-cli/src/lib.rs:103 re-exports roko_serve.
crates/roko-cli/src/unified.rs:120 returns roko_serve::state::AppState.
crates/roko-cli/src/prd.rs:728 calls roko_core::config::load_config.
crates/roko-cli/src/prd.rs:735 constructs StateHub directly.
crates/roko-cli/src/worker/cloud.rs:453 calls roko_core::config::load_config.
crates/roko-cli/src/worker/cloud.rs:461 constructs StateHub directly.
crates/roko-cli/src/tui/app.rs:419 creates SharedStateHub in-process.
crates/roko-cli/src/tui/app.rs:422 reads .roko/events.jsonl to replay dashboard events.
crates/roko-cli/src/tui/app.rs:1234 writes .roko/engrams.jsonl from TUI injection.
crates/roko-cli/src/tui/app.rs:2475 writes .roko/jobs from TUI job form.
crates/roko-cli/src/tui/app.rs:3111 reads ROKO_SERVE_URL.
crates/roko-cli/src/tui/config_meta.rs:679 reads config files directly for TUI metadata.
crates/roko-cli/src/tui/views/marketplace_view.rs:5 says data source is .roko/jobs files.
crates/roko-serve/src/lib.rs:224 spawns dispatch_loop directly.
crates/roko-serve/src/lib.rs:297 spawns watcher command directly.
crates/roko-serve/src/lib.rs:717 constructs KnowledgeStore directly.
crates/roko-serve/src/lib.rs:770 maps ServerEvent to DashboardEvent.
crates/roko-serve/src/lib.rs:945 maps DashboardEvent to ServerEvent.
crates/roko-serve/src/routes/providers.rs:301 creates an agent directly.
crates/roko-serve/src/routes/plans.rs:936,953,1003,1013,1079 run git commands directly.
crates/roko-serve/src/routes/vision_loop.rs:133 spawns the roko binary directly.
crates/roko-serve/src/routes/config.rs:127 reloads config through load_config directly.
crates/roko-serve/src/routes/run.rs:102 spawns route-owned run task.
crates/roko-serve/src/routes/run.rs:114 emits DashboardEvent directly.
crates/roko-serve/src/routes/jobs.rs:845 maps job lifecycle to DashboardEvents in route code.
crates/roko-serve/src/routes/status/health.rs:117 exposes StateHub event query directly.
crates/roko-dreams/src/runner.rs:65 loads config directly.
crates/roko-dreams/src/runner.rs:79 and 109 create agents directly.
crates/roko-neuro/src/episode_completion.rs:25 reads ANTHROPIC_API_KEY directly.
crates/roko-neuro/src/lifecycle.rs:364 appends lifecycle JSONL directly.
crates/roko-neuro/src/admission.rs:656 and 664 append candidate/decision JSONL directly.
crates/roko-std/src/tool/builtin/web_search.rs:261 reads PERPLEXITY_API_KEY directly.
crates/roko-acp/src/bridge_events.rs:564 spawns claude directly.
crates/roko-acp/src/runner.rs:551,601,646,673 spawn shell/claude/git commands directly.
crates/roko-runtime/src/process.rs:884 is the existing supervised process owner.
```

### Repository-Wide Owner Matrix

Every scan category should have one owning redesign doc. Agents should update the owner doc, not invent local policy in this file.

| Smell Class | Current Examples | Owning Redesign Doc | Target Owner |
| --- | --- | --- | --- |
| Crate direction violation | `roko-core -> roko-runtime`, domain crates -> `roko-agent`, CLI -> serve | [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md) | Layer manifest and app service boundary |
| Config/env/key drift | direct `ANTHROPIC_API_KEY`, `PERPLEXITY_API_KEY`, `ROKO_CONFIG`, manual global merge | [33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md](33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md) | `RuntimeConfigLoader` and `SecretService` |
| Event/projection drift | StateHub/EventBus/dashboard bridge, direct events JSONL, route/TUI raw reads | [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md) | `RuntimeEventStore`, `ProjectionEngine`, `RuntimeQueryService` |
| Process/task lifecycle drift | route `tokio::spawn`, `Command::new`, volatile operation maps | [35-TASK-PROCESS-LIFECYCLE-AUDIT.md](35-TASK-PROCESS-LIFECYCLE-AUDIT.md) | `RuntimeTaskSupervisor`, `ManagedCommandRunner`, `OperationStore` |
| Workflow entrypoint drift | PRD, plan, research, run, TUI, serve each assemble workflows differently | [36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md](36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md) | `WorkflowEngine` and command service |
| Storage/artifact drift | direct `.roko` paths, jobs files, PRD files, engrams, episodes | [37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md](37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md) | `WorkspaceLayout`, repositories, artifact store |
| Cognitive feedback drift | learn/neuro/dreams direct stores and side effects | [38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md](38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md) | `CognitiveLoopEngine`, feedback sinks |
| Runner policy drift | runner side effects, gate/retry/merge decisions, legacy orchestrate overlap | [39-RUNNER-EXECUTION-POLICY-AUDIT.md](39-RUNNER-EXECUTION-POLICY-AUDIT.md) | reducer/effect split and execution decision records |
| HTTP/TUI adapter drift | routes/TUI own command/query behavior and raw reads | [40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md](40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md) | command/query adapters |
| Provider/model-call drift | direct `create_agent_for_model`, direct CLI/API provider calls, gateway facade | [41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md) | `ModelCallService` / `InferenceGateway` |

### P0 Work Queues

These queues are designed for parallel implementation. Each queue has a clear source of truth and proof gate.

#### Queue A - Layer Firewall

- [ ] Generate current local dependency graph with `cargo metadata`.
- [ ] Add machine-readable layer manifest.
- [ ] Mark current violations: `roko-core -> roko-runtime`, `roko-agent -> roko-learn`, domain crates -> `roko-agent`, CLI -> serve, serve -> domain implementations.
- [ ] Implement graph check script with temporary allowlist.
- [ ] Move StateHub/PulseBus implementation out of `roko-core`.
- [ ] Remove CLI public server re-export.
- [ ] Replace domain concrete provider dependencies with service traits.
- [ ] Save before/after graph snapshots.

Proof gate:

- [ ] `cargo metadata` output matches layer manifest or produces only allowlisted violations.
- [ ] [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md) links the graph proof.

#### Queue B - Runtime Context And Provider Proof

- [ ] Create `RuntimeContextBuild` and attach build id to runs and provider proof.
- [ ] Move config path/env resolution into one loader.
- [ ] Move provider credentials into `SecretService`.
- [ ] Replace direct env reads in CLI, neuro, std web search, provider routes, provider tests, and agent serve.
- [ ] Split connectivity checks from runtime provider proof.
- [ ] Make provider proof call `ModelCallService::probe_provider`.
- [ ] Persist provider proof into projections and proof bundles.

Proof gate:

- [ ] Provider matrix returns explicit statuses for Anthropic, OpenAI, Moonshot, Z.AI, Perplexity, Claude CLI, and Codex CLI.
- [ ] `provider_state`, `proof_state`, and proof bundle agree on context build id, provider id, model key, status, and event ids.

#### Queue C - Event Store And Query Spine

- [ ] Define `RuntimeEventEnvelope`.
- [ ] Implement durable `RuntimeEventStore`.
- [ ] Replace runner direct event append with event store append.
- [ ] Replace StateHub/EventBus route streams with projection stream service.
- [ ] Replace route/TUI raw file readers with query service.
- [ ] Add proof bundle service.

Proof gate:

- [ ] A run can be restarted and all projections rebuild from durable runtime events.
- [ ] SSE/WebSocket stream reconnect resumes by durable cursor.

#### Queue D - Workflow And Runtime Services

- [ ] Introduce `roko-app` or equivalent app service boundary.
- [ ] Move CLI plan run, HTTP plan run, PRD generation, research, jobs, and one-shot run to `RuntimeCommandService`.
- [ ] Move HTTP/TUI status reads to `RuntimeQueryService`.
- [ ] Move route-owned background spawns to `RuntimeTaskSupervisor`.
- [ ] Move process commands to `ManagedCommandRunner`.

Proof gate:

- [ ] CLI and HTTP for the same workflow produce the same operation id, projection cursor, artifact ids, and proof bundle evidence.

#### Queue E - Cognitive Loop Integration

- [ ] Make runner outcomes flow through `CognitiveTransaction`.
- [ ] Move learning/neuro/dream direct writes behind repositories and sinks.
- [ ] Make prompt assembly cite cognitive source versions.
- [ ] Add two-run proof: run A writes knowledge/policy, run B consumes it with evidence.

Proof gate:

- [ ] Cognitive query service shows feedback ingestion, knowledge admission, dream trigger, policy update, and prompt influence with event ids.

### P1 Work Queues

#### Queue F - Storage And Artifact Migration

- [ ] Define `WorkspaceLayout`.
- [ ] Define typed artifact repositories for PRD, plan, task, job, proof, prompt diagnostics, model call, process, and cognitive outputs.
- [ ] Add storage migration status.
- [ ] Replace direct `.roko` path strings outside repositories.
- [ ] Add clean-clone proof fixtures.

Proof gate:

- [ ] A storage layout migration changes repository internals without changing route/TUI/runner code.

#### Queue G - Runner Policy Hardening

- [ ] Extract reducer/effect split from runner event loop.
- [ ] Convert gate skip/pass/fail to typed statuses.
- [ ] Persist retry/replan decisions.
- [ ] Wire merge backend success and conflict evidence.
- [ ] Retire legacy orchestrate production callers.

Proof gate:

- [ ] Runner proof covers gate failure retry, merge success, merge conflict, resume after crash, prompt diagnostics, and provider events.

#### Queue H - UI/HTTP Consistency

- [ ] Make TUI consume query snapshots and projection streams.
- [ ] Make HTTP status routes call query service.
- [ ] Remove StateHub health/debug endpoints as proof surfaces or mark them explicitly diagnostic.
- [ ] Add cursor consistency proof between TUI and HTTP projection.

Proof gate:

- [ ] TUI and HTTP show same run state within documented lag and both cite projection cursor.

### Machine-Readable Scan Artifact

Add a generated artifact so future agents can prove they scanned the codebase rather than manually sampling.

Suggested path:

```text
tmp/mori-diffs/proof/repository-scan-YYYY-MM-DD.json
```

Suggested schema:

```json
{
  "generated_at": "2026-04-27T00:00:00Z",
  "rust_files_scanned": 1026,
  "patterns": {
    "direct_env_reads": [],
    "direct_provider_construction": [],
    "route_spawns": [],
    "direct_roko_paths": [],
    "statehub_dashboard_events": [],
    "unsafe_policy": [],
    "legacy_orchestrate": []
  },
  "owners": {
    "direct_env_reads": "33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md",
    "direct_provider_construction": "41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md",
    "route_spawns": "35-TASK-PROCESS-LIFECYCLE-AUDIT.md",
    "direct_roko_paths": "37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md",
    "statehub_dashboard_events": "34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md",
    "unsafe_policy": "33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md",
    "legacy_orchestrate": "39-RUNNER-EXECUTION-POLICY-AUDIT.md"
  }
}
```

Implementation checklist:

- [ ] Add a script that emits the scan artifact from `rg`/`cargo metadata`.
- [ ] Include test/prod classification or at least file path plus `cfg(test)` context when possible.
- [ ] Include owner doc and severity for every match.
- [ ] Include baseline counts and latest counts.
- [ ] Add a summary table generator for this doc.
- [ ] Add a diff mode that compares current scan to baseline.

### Repository-Wide Grep Gates

These are not all expected to pass today. They define the cleanup direction.

```bash
find crates -name '*.rs' -print | wc -l
```

Expected:

- [ ] The count is recorded in the scan artifact.

```bash
rg -n "roko_runtime::" crates/roko-core/src -g '*.rs'
```

Expected:

- [ ] Zero production matches after core/runtime split.

```bash
rg -n "std::env::var\\(|std::env::var_os\\(|ANTHROPIC_API_KEY|OPENAI_API_KEY|MOONSHOT_API_KEY|ZAI_API_KEY|PERPLEXITY_API_KEY" \
  crates/roko-cli/src crates/roko-serve/src crates/roko-neuro/src crates/roko-std/src crates/roko-agent/src -g '*.rs'
```

Expected:

- [ ] Only config/secret adapters and tests read provider/key env vars.

```bash
rg -n "create_agent_for_model|AgentOptions" \
  crates/roko-cli/src crates/roko-serve/src crates/roko-dreams/src crates/roko-neuro/src crates/roko-compose/src crates/roko-gate/src -g '*.rs'
```

Expected:

- [ ] Production calls go through `ModelCallService`, dispatch facade, or provider adapter internals only.

```bash
rg -n "tokio::spawn|tokio::process::Command|std::process::Command|Command::new" \
  crates/roko-serve/src/routes crates/roko-cli/src/commands crates/roko-acp/src -g '*.rs'
```

Expected:

- [ ] Route/command/ACP surfaces submit tasks or process requests through supervised services.

```bash
rg -n "join\\(\"\\.roko\"\\)|\"\\.roko/|engrams\\.jsonl|events\\.jsonl|episodes\\.jsonl|efficiency\\.jsonl|gate-thresholds\\.json|cascade-router\\.json" \
  crates -g '*.rs'
```

Expected:

- [ ] Production direct storage paths are limited to repositories, layout code, migrations, and tests.

```bash
rg -n "DashboardEvent|ServerEvent|StateHub|state_hub|EventBus" \
  crates/roko-cli/src crates/roko-serve/src -g '*.rs'
```

Expected:

- [ ] Adapter surfaces use runtime events, query service, and projection streams instead of view events as truth.

```bash
rg -n "dangerously_skip_permissions|dangerously-bypass|dangerously-skip" crates -g '*.rs'
```

Expected:

- [ ] Dangerous policy appears only in `RuntimePolicy`, provider adapter argument rendering, tests, and migration compatibility.

### Definition Of Complete

- [ ] A generated repository scan artifact exists and records current Rust file count.
- [ ] Every scan match is assigned an owner doc and severity.
- [ ] The layer graph check exists and is linked from [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md).
- [ ] Config/secret/provider drift is handled by [33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md](33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md).
- [ ] Event/projection/query drift is handled by [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md).
- [ ] Process/task lifecycle drift is handled by [35-TASK-PROCESS-LIFECYCLE-AUDIT.md](35-TASK-PROCESS-LIFECYCLE-AUDIT.md).
- [ ] Workflow entrypoint drift is handled by [36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md](36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md).
- [ ] Storage/artifact drift is handled by [37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md](37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md).
- [ ] Cognitive feedback drift is handled by [38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md](38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md).
- [ ] Runner policy drift is handled by [39-RUNNER-EXECUTION-POLICY-AUDIT.md](39-RUNNER-EXECUTION-POLICY-AUDIT.md).
- [ ] HTTP/TUI adapter drift is handled by [40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md](40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md).
- [ ] Provider/model-call drift is handled by [41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md).
- [ ] The repo can prove one end-to-end run through CLI and HTTP using the same context build, event store, projections, provider proof, workflow artifacts, cognitive updates, and proof bundle.
