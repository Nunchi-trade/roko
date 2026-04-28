# 40 - Serve And TUI Runtime Adapter Audit

Date: 2026-04-27

Status: active implementation handoff

### Architecture Runner Update (2026-04-28)
Serve/TUI adapter convergence infrastructure:
- `SseAdapter` (P3B) implements EventConsumer for HTTP SSE streaming
- `RuntimeProjection` (P3C) provides materialized view for REST endpoint queries
- `EventConsumer` trait (P0B) enables any new adapter with one implementation
- Remaining: wiring REST endpoints to RuntimeProjection, TuiAdapter implementation, operation store convergence

Scope: `roko-serve`, HTTP routes, server background jobs, server projections, websocket/SSE surfaces, and TUI command/query behavior. This doc focuses on whether serve/TUI are thin adapters over the same runtime services or whether they still own runtime behavior directly.

If another agent only reads this file, it should be able to implement the next serve/TUI convergence slice without rediscovering the repo.

## Executive Verdict

`roko-serve` and the TUI have some good seams:

- `crates/roko-serve/src/runtime.rs` defines `CliRuntime`, so `roko-serve` does not directly depend on `roko-cli`.
- `crates/roko-cli/src/serve_runtime.rs` implements that trait with real CLI internals.
- `crates/roko-serve/src/projection_contract.rs` defines `RuntimeProjectionSet`, projection envelopes, projection names, and query filters.
- `AppState` owns `RokoLayout`, `SharedStateHub`, `ProcessSupervisor`, config snapshotting, provider health, and cancellation.
- `/api/projections/*`, several status routes, and some learning routes already read via `RuntimeProjectionSet`.

The remaining architecture problem is that serve and TUI are still not just adapters. They still:

- Spawn background tasks directly from route handlers.
- Track operation/run/plan status in serve-local `HashMap`s.
- Build ad hoc prompts for plans, PRDs, plan chat, research, gateway batches, dreams, templates, and jobs.
- Write `.roko` files directly from route modules and TUI UI actions.
- Run `git` directly from plan review routes and TUI views.
- Synthesize fallback plans and synthetic chain inputs in the job runner.
- Publish dashboard events manually that can diverge from durable runner events.
- Query some state through projections while other state is read from files, route-local maps, or live StateHub snapshots.

This means the HTTP API and TUI can still become a second runtime beside the CLI runner. The correct redesign is not more route helper functions. The correct redesign is a shared runtime API:

```text
HTTP routes / TUI actions / WebSocket / SSE
  -> RuntimeCommandService
  -> RuntimeQueryService
  -> RuntimeOperationStore
  -> RuntimeArtifactRepository
  -> BackgroundTaskSupervisor
  -> RuntimeProjectionService
```

Routes should validate input and call services. TUI should render query results and submit commands. Neither should write `.roko` artifacts, start long-running runtime work, or infer execution status from private maps.

## Source Evidence

Commands run for this audit:

```bash
find crates/roko-serve/src -maxdepth 3 -type f -name '*.rs' | sort
rg -n "tokio::spawn|Command::new|std::process::Command|tokio::process::Command|std::fs|tokio::fs|OpenOptions|create_dir_all|read_to_string|write\\(|join\\(\"\\.roko\"\\)|events\\.jsonl|episodes\\.jsonl|engrams\\.jsonl|PlanRunner|orchestrate|RunConfig|PlanExecution|Dispatcher|PromptAssembler|AgentRuntimeEvent|Job|Operation|broadcast|subscribe|watch|projection|Projection" crates/roko-serve/src crates/roko-cli/src/serve_runtime.rs crates/roko-cli/src/tui -g '*.rs'
python3 - <<'PY'
from pathlib import Path
import re
roots=[Path("crates/roko-serve/src"),Path("crates/roko-cli/src/tui")]
patterns={
 "fs": re.compile(r"\\b(?:std::fs|tokio::fs|OpenOptions|File::create|File::open|read_to_string|write\\(|create_dir_all|remove_file|remove_dir_all|rename\\()"),
 "spawn": re.compile(r"\\b(?:tokio::spawn|Command::new|tokio::process::Command|std::process::Command|\\.spawn\\()"),
 "paths": re.compile(r"join\\(\"\\.roko\"\\)|\"\\.roko/|events\\.jsonl|episodes\\.jsonl|engrams\\.jsonl|jobs\\.jsonl|plans/|prd/"),
 "runtime": re.compile(r"PlanRunner|RunConfig|Dispatcher|PromptAssembler|AgentRuntimeEvent|orchestrate|runtime|Runtime|runner"),
 "state": re.compile(r"Arc<Mutex|RwLock|broadcast|watch::|mpsc|Subscriber|subscribe|Job|TaskStatus|Projection|projection"),
 "git": re.compile(r"git|Git|diff|merge|branch|worktree"),
 "http": re.compile(r"Router|route\\(|Json<|State<|WebSocket|Sse|Event"),
 "legacy": re.compile(r"legacy|TODO|FIXME|HACK|for now|stub|deprecated"),
}
files=sorted(p for root in roots for p in root.rglob("*.rs"))
print("files_scanned",len(files))
for p in files:
    text=p.read_text(errors="ignore")
    counts={k:len(v.findall(text)) for k,v in patterns.items()}
    total=sum(counts.values())
    if total >= 35:
        print(total, len(text.splitlines()), p, counts)
PY
```

Result:

- `152` serve/TUI Rust files scanned.
- The count is pattern-based and intentionally broad. It includes benign tests, but it correctly identifies runtime ownership hotspots.

Top hotspots:

| File | Lines | Matches | Meaning |
| --- | ---: | ---: | --- |
| `crates/roko-cli/src/tui/app.rs` | 4101 | 303 | TUI owns command submission, direct `.roko` writes, git refresh orchestration, and UI-local runtime actions. |
| `crates/roko-serve/src/routes/plans.rs` | 1653 | 300 | Plan routes own CRUD, execution, pause/resume, review, git diff/merge, plan chat, estimates, and generation. |
| `crates/roko-cli/src/tui/dashboard.rs` | 6382 | 269 | TUI dashboard directly reads many `.roko` files and runs git instead of only consuming projections. |
| `crates/roko-serve/src/projection_contract.rs` | 2815 | 266 | Projection surface is rich, but it still directly loads file inputs and mixes live/recovered state. |
| `crates/roko-serve/src/routes/jobs.rs` | 1747 | 213 | Job routes own file-backed job state, publication, state hub updates, and HTTP behavior. |
| `crates/roko-serve/src/routes/prds.rs` | 1309 | 204 | PRD routes own lifecycle, auto-plan orchestration, audit episodes, background subscribers, and prompt construction. |
| `crates/roko-serve/src/lib.rs` | 1321 | 200 | Server startup owns many background loops and dropped join handles. |
| `crates/roko-cli/src/tui/state.rs` | 4968 | 196 | TUI state mirrors many projections and file-backed facts locally. |
| `crates/roko-serve/src/dispatch.rs` | 2878 | 158 | Serve has a template dispatch runtime separate from plan runner dispatch. |
| `crates/roko-serve/src/job_runner.rs` | 1079 | 139 | Job runner owns job lifecycle, polling, fallback planning, plan execution, and synthetic chain execution. |
| `crates/roko-serve/src/routes/research.rs` | 793 | 106 | Research routes own artifact layout, prompts, and operation lifecycle. |
| `crates/roko-serve/src/state.rs` | 973 | 89 | AppState owns live operation maps and only persists part of server state. |

## Good Seams To Preserve

### `CliRuntime`

`crates/roko-serve/src/runtime.rs` is a useful dependency inversion seam. It prevents a circular crate dependency.

Keep:

- [ ] Trait-based serve-to-runtime boundary.
- [ ] Structured result types such as `RunResult`, `PlanGenerationResult`, and `PlanExecutionResult`.
- [ ] Repo-aware methods like `resolve_repo_workdir`, `repo_roko_config`, and `list_repos`.

Change:

- [ ] Do not leave `run_once(prompt)` as the dominant command primitive.
- [ ] Do not let default `run_plan` implement plan execution by writing a prompt that asks an agent to execute a plan.
- [ ] Rename or replace `CliRuntime` with `RuntimeCommandService` once the service is shared by CLI, HTTP, and TUI.

### `RuntimeProjectionSet`

`crates/roko-serve/src/projection_contract.rs` is a good start for query convergence.

Keep:

- [ ] Stable projection catalog.
- [ ] Projection envelope with name, canonical name, version, cursor, computed time, recovered flag, evidence, and state.
- [ ] Aliases like `events -> event_log`, `providers -> provider_state`, `trace -> execution_trace`.
- [ ] SSE projection stream filtering.

Change:

- [ ] Move file loading behind `RuntimeProjectionService` and repositories.
- [ ] Add operation, job, PRD, plan artifact, merge, and decision projections.
- [ ] Make HTTP/TUI consume this service instead of direct `.roko` readers.

### `AppState`

`crates/roko-serve/src/state.rs` has useful central state:

- `RokoLayout`.
- `SharedStateHub`.
- `ProcessSupervisor`.
- cancellation token.
- config snapshot.
- provider health.
- metrics.
- event bus.

Keep these, but reduce direct ownership:

- [ ] `AppState` should hold service handles, not domain-specific mutable maps for every feature.
- [ ] `active_runs`, `active_plans`, and `operations` should move behind `RuntimeOperationStore`.
- [ ] `event_bus` and `state_hub` should be adapters fed by durable runtime events, not independent sources of truth.

## Current Broken Shape

### HTTP plan execution is still prompt-based in several paths

Evidence:

- `routes/plans.rs::execute_plan` builds a plan execution prompt and calls `runtime.run_once`.
- `routes/plans.rs::resume_plan` builds a resume prompt and calls `runtime.run_once`.
- `routes/plans.rs::generate_plan` builds a plan generation prompt and calls `runtime.run_once`.
- `routes/plans.rs::plan_chat` builds an LLM mutation prompt and writes the response to `.roko/state/{plan_id}.chat-response.json`.
- `routes/prds.rs::queue_plan_generation_op` builds a plan generation prompt and calls `runtime.run_once`.
- `CliRuntime::run_plan` has a default implementation that calls `run_once` with a plan-execution prompt.

The real implementation in `roko-cli/src/serve_runtime.rs` overrides `run_plan`, but route paths often still call `run_once` directly instead of command-specific service methods. This is why serve can appear to support a workflow while bypassing runner-v2 features.

### Serve operations are mostly volatile

Evidence:

- `AppState` has `active_runs`, `active_plans`, and `operations` as `RwLock<HashMap<...>>`.
- `RunHandle`, `PlanHandle`, and `OperationHandle` contain `JoinHandle<()>`.
- `save_snapshot` persists discovered agents and template runs, not active runs, plans, operation history, operation attempts, cancellation state, or terminal failure evidence.
- `/api/run/{id}/status` reads `active_runs`.
- `/api/plans/{id}/status` reads `active_plans`.
- `/api/operations/{id}` reads `operations`.

This means a server restart loses operation truth unless the underlying runner also wrote durable events and the endpoint knows how to recover them. Status is not one canonical projection.

### Routes are repositories and workers

Evidence:

- `routes/plans.rs` reads and writes `.roko/plans`, `.roko/state`, `.roko/learn`, `.roko/prd`, and review JSONL.
- `routes/prds.rs` reads/writes `.roko/prd`, `.roko/plans`, `.roko/episodes.jsonl`, and starts subscribers.
- `routes/jobs.rs` is backed by `.roko/jobs/*.json`.
- `routes/research.rs` reads/writes `.roko/research` and constructs research prompts.
- `routes/deployments.rs`, `routes/templates.rs`, `routes/team.rs`, `routes/auth.rs`, `routes/secrets.rs`, and `routes/feeds.rs` each own their own storage paths.

Route modules should validate HTTP input and call services. They should not know storage layout or long-running job mechanics.

### TUI still writes command files directly

Evidence:

- `tui/app.rs::SubmitInject` appends an inject directive to `.roko/engrams.jsonl`.
- `tui/app.rs::ConfirmYes` appends a confirm directive to `.roko/engrams.jsonl`.
- `tui/app.rs::submit_marketplace_job` writes a JSON file to `.roko/jobs`.
- `tui/dashboard.rs` directly reads `.roko/engrams.jsonl`, `.roko/episodes.jsonl`, `.roko/learn/*`, `.roko/state/*`, `.roko/jobs/*`, `.roko/prd/*`, `.roko/neuro/*`, `.roko/task-outputs/*`, and git diff output.

This makes TUI a second command writer and a second query engine. It should submit commands to runtime services and consume projection snapshots.

### Server review/merge owns git directly

Evidence:

- `routes/plans.rs::find_agent_branch` runs `git branch --list`.
- `routes/plans.rs::diff_summary` runs `git diff --stat`.
- `routes/plans.rs::parse_git_diff` runs `git diff --numstat` and `git diff`.
- `routes/plans.rs::merge_branch` runs `git merge --no-ff`.
- Review approval does not use `runner/merge.rs`, `MergePolicyEngine`, typed conflict evidence, post-merge regression, or queue semantics.

This is exactly the kind of one-off merge path that breaks Mori parity.

### Job runner has synthetic and fallback behavior that can masquerade as real execution

Evidence:

- `job_runner.rs::execute_chain_monitor_job` creates synthetic mock chain events.
- `job_runner.rs::execute_chain_analysis_job` creates synthetic analysis events.
- `job_runner.rs::prepare_coding_plan` falls back to `synthesize_coding_plan` when runtime planning is unavailable.
- `execute_coding_job` marks gate results as `"runtime": true` when structured gates are absent.

These can be useful dev fallbacks, but they must never be counted as full end-to-end proof. In strict or Mori-parity mode, unsupported real integrations must be explicit failures or `unsupported`, not synthetic success.

## Target Architecture

The target is:

```text
HTTP route / TUI action / WebSocket message / SSE subscription
  -> adapter validation
  -> RuntimeCommandService or RuntimeQueryService
  -> domain service
  -> repository / event store / projection service
  -> durable operation event
  -> projection update
```

### Runtime command service

Define a shared command service, not a CLI-specific bridge:

```rust
pub trait RuntimeCommandService: Send + Sync {
    async fn start_run(&self, command: StartRunCommand) -> Result<OperationRef>;
    async fn start_workflow(&self, command: StartWorkflowCommand) -> Result<OperationRef>;
    async fn generate_prd(&self, command: GeneratePrdCommand) -> Result<OperationRef>;
    async fn promote_prd(&self, command: PromotePrdCommand) -> Result<ArtifactRef>;
    async fn generate_plan(&self, command: GeneratePlanCommand) -> Result<OperationRef>;
    async fn execute_plan(&self, command: ExecutePlanCommand) -> Result<OperationRef>;
    async fn pause_operation(&self, id: OperationId) -> Result<OperationRef>;
    async fn resume_operation(&self, id: OperationId) -> Result<OperationRef>;
    async fn submit_review(&self, command: ReviewDecisionCommand) -> Result<OperationRef>;
    async fn create_job(&self, command: CreateJobCommand) -> Result<JobRef>;
    async fn execute_job(&self, command: ExecuteJobCommand) -> Result<OperationRef>;
    async fn submit_tui_directive(&self, command: TuiDirectiveCommand) -> Result<CommandAck>;
}
```

Rules:

- [ ] No command method should accept raw prompt text as the only structured input unless the command is truly `StartRunCommand`.
- [ ] Every command returns an `OperationRef` or `ArtifactRef` with durable ids.
- [ ] Every command emits operation events before and after work starts.
- [ ] Long-running commands are scheduled through `BackgroundTaskSupervisor`.

### Runtime query service

Define one query service for HTTP and TUI:

```rust
pub trait RuntimeQueryService: Send + Sync {
    async fn projection(&self, name: ProjectionName, query: ProjectionQuery) -> Result<ProjectionFrame>;
    async fn operation(&self, id: OperationId) -> Result<OperationProjection>;
    async fn operations(&self, query: OperationQuery) -> Result<Vec<OperationProjection>>;
    async fn artifact(&self, id: ArtifactId) -> Result<ArtifactProjection>;
    async fn job(&self, id: JobId) -> Result<JobProjection>;
    async fn prd(&self, slug: PrdSlug) -> Result<PrdProjection>;
    async fn plan(&self, id: PlanId) -> Result<PlanProjection>;
    async fn git(&self, query: GitQuery) -> Result<GitProjection>;
}
```

Rules:

- [ ] HTTP routes and TUI views use this service.
- [ ] TUI dashboard direct readers become compatibility fallbacks only.
- [ ] Query service returns source evidence and freshness metadata.
- [ ] Query service can recover from durable events after server restart.

### Runtime operation store

Replace serve-local maps with durable operations:

```rust
pub enum RuntimeOperationKind {
    RunOnce,
    Workflow,
    PrdDraft,
    PrdPromote,
    PlanGenerate,
    PlanExecute,
    PlanReview,
    JobExecute,
    Research,
    Dream,
    GatewayBatch,
    TemplateDispatch,
}

pub enum RuntimeOperationState {
    Queued,
    Running,
    Cancelling,
    Cancelled,
    Completed,
    Failed,
    Blocked,
}

pub struct RuntimeOperationRecord {
    pub id: OperationId,
    pub kind: RuntimeOperationKind,
    pub state: RuntimeOperationState,
    pub command_ref: Option<CommandRef>,
    pub run_id: Option<String>,
    pub plan_id: Option<String>,
    pub job_id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub evidence: Vec<ArtifactRef>,
}
```

Rules:

- [ ] A server restart must not turn a known operation into `404`.
- [ ] `JoinHandle` is not the source of truth.
- [ ] In-memory handles are execution details; durable records are query truth.

### Artifact repositories

Move direct `.roko` access behind repositories:

```text
PrdRepository
PlanRepository
JobRepository
ResearchRepository
TemplateRepository
DeploymentRepository
ReviewRepository
RuntimeEventStore
ProjectionStore
SecretStore
TeamRepository
```

Rules:

- [ ] Repositories own path validation, atomic writes, schema migration, and evidence metadata.
- [ ] Routes never call `workdir.join(".roko")`.
- [ ] TUI never writes command files directly.

### Background task supervisor

Move route-level `tokio::spawn` into one supervisor:

```rust
pub trait BackgroundTaskSupervisor {
    fn spawn_operation(&self, op: RuntimeOperationRecord, task: RuntimeTask) -> OperationRef;
    async fn cancel(&self, id: OperationId) -> Result<CancelOutcome>;
    async fn shutdown(&self) -> Result<ShutdownReport>;
}
```

Rules:

- [ ] Every spawned task has an operation id.
- [ ] Every task has a cancellation token.
- [ ] Every task emits started/completed/failed/cancelled events.
- [ ] Every task writes durable operation state.
- [ ] Dropped join handles are allowed only for tasks supervised elsewhere.

## P0 Findings And Checklists

### P0-01 Replace Prompt-Based HTTP Workflow Commands

Problem:

HTTP plan/PRD commands still create prompts and call `runtime.run_once`. This bypasses runner-v2 and typed artifacts.

Evidence:

- `routes/plans.rs::execute_plan` calls `runtime.run_once`.
- `routes/plans.rs::resume_plan` calls `runtime.run_once`.
- `routes/plans.rs::generate_plan` calls `runtime.run_once`.
- `routes/prds.rs::queue_plan_generation_op` calls `runtime.run_once`.
- `CliRuntime::run_plan` default calls `run_once` with a plan-execution prompt.

Target:

- `POST /api/prds/{slug}/draft` calls `RuntimeCommandService::generate_prd`.
- `POST /api/prds/{slug}/plan` calls `RuntimeCommandService::generate_plan`.
- `POST /api/plans/{id}/execute` calls `RuntimeCommandService::execute_plan`.
- `POST /api/plans/{id}/resume` calls `RuntimeCommandService::resume_operation` or `execute_plan` with a resume token.
- `POST /api/plans/{id}/chat` calls a plan mutation service, not arbitrary `run_once`.

Checklist:

- [ ] Add `RuntimeCommandService` trait beside or above `CliRuntime`.
- [ ] Implement it in `roko-cli/src/serve_runtime.rs` using real PRD, planner, and runner APIs.
- [ ] Replace plan route `runtime.run_once` calls with typed command calls.
- [ ] Replace PRD route `runtime.run_once` calls with typed command calls.
- [ ] Delete or deprecate `CliRuntime::run_plan` default prompt fallback.
- [ ] Add strict mode where unsupported command methods return `unsupported`, not prompt fallback.
- [ ] Add proof that `/api/plans/{id}/execute` invokes runner-v2 and writes runner events.
- [ ] Add proof that `/api/prds/{slug}/plan` creates typed plan artifacts and returns operation/artifact refs.

### P0-02 Make Operations Durable And Shared

Problem:

Serve-local active maps are volatile and not the same as runner state.

Evidence:

- `AppState::active_runs`, `active_plans`, and `operations` are `RwLock<HashMap<...>>`.
- `RunHandle`, `PlanHandle`, and `OperationHandle` contain `JoinHandle`.
- `save_snapshot` does not persist active operations.
- Status routes read these maps directly.

Target:

- `RuntimeOperationStore` is the source of truth.
- The operation store is queryable by HTTP, TUI, SSE, and proof scripts.
- Active `JoinHandle`s are attached to operation ids but are not the durable truth.

Checklist:

- [ ] Define `RuntimeOperationRecord` and `RuntimeOperationEvent`.
- [ ] Add durable append-only log: `.roko/operations/events.jsonl` or shared runtime events with operation variants.
- [ ] Add materialized operation snapshot.
- [ ] Move `active_runs`, `active_plans`, and `operations` behind `RuntimeOperationStore`.
- [ ] Update `/api/run/{id}/status`, `/api/plans/{id}/status`, and `/api/operations/{id}` to query the store.
- [ ] Add projection `operations`.
- [ ] Add SSE stream `projection:operations`.
- [ ] Add crash/restart proof: start long operation, kill server, restart, operation is not `404`.
- [ ] Add cancellation proof: cancel updates durable state and child task/process stops.

### P0-03 Move Route Filesystem Writes Into Repositories

Problem:

Route handlers own storage layout and write files directly.

Evidence:

- `routes/plans.rs` writes plan JSON, pause snapshots, chat responses, reviews, and reads efficiency history.
- `routes/prds.rs` renames PRD files, writes audit episodes, scans PRD directories, and counts plans.
- `job_runner.rs` writes research reports, job artifacts, PRDs, synthesized plans, and job status files.
- TUI writes jobs and directives directly.

Target:

- Route modules call repositories.
- Repositories own `.roko` layout, atomic writes, schema validation, and migration.

Checklist:

- [ ] Add `PrdRepository`.
- [ ] Add `PlanRepository`.
- [ ] Add `JobRepository`.
- [ ] Add `ReviewRepository`.
- [ ] Add `ResearchRepository`.
- [ ] Add `RuntimeArtifactRepository`.
- [ ] Replace route-level `workdir.join(".roko")` calls with repository calls.
- [ ] Add grep gate: `rg -n "join\\(\"\\.roko\"\\)|\"\\.roko/" crates/roko-serve/src/routes crates/roko-cli/src/tui` should return only tests and compatibility adapters.
- [ ] Add proof that each repository writes evidence metadata consumed by projections.

### P0-04 Route Long-Running Work Through A Supervisor

Problem:

Many route handlers call `tokio::spawn` directly and insert a handle in a map.

Evidence:

- `routes/run.rs::spawn_background_run` spawns a task.
- `routes/plans.rs` spawns tasks for execute, resume, chat, generate, and estimate-like commands.
- `routes/prds.rs` spawns tasks for draft, plan, consolidate, and subscribers.
- `routes/research.rs`, `routes/dream.rs`, `routes/templates.rs`, `routes/gateway.rs`, `routes/deployments.rs`, and `routes/agents.rs` spawn tasks.
- `lib.rs::start_background` starts many background loops and stores join handles in `_` bindings.

Target:

- `BackgroundTaskSupervisor` owns all long-running work.
- Each task has operation id, cancel token, lifecycle events, error evidence, and shutdown behavior.

Checklist:

- [ ] Implement `BackgroundTaskSupervisor` backed by `roko_runtime::process::ProcessSupervisor` and task handles.
- [ ] Add `spawn_operation` API.
- [ ] Move `/api/run` spawning into the supervisor.
- [ ] Move plan/PRD/research/dream/template/gateway/deployment spawning into the supervisor.
- [ ] Register startup loops as supervised services with names: dispatch loop, config watcher, PRD subscriber, feedback loop, state hub bridge, state saver, job runner, cold archival, relay registration.
- [ ] Add endpoint `/api/operations` listing current and historical operations.
- [ ] Add shutdown proof that all supervised tasks receive cancellation and report final state.

### P0-05 TUI Must Become A Command/Projection Client

Problem:

TUI is still both a command writer and query engine.

Evidence:

- `tui/app.rs` appends inject and confirm directives to `.roko/engrams.jsonl`.
- `tui/app.rs` writes jobs to `.roko/jobs`.
- `tui/dashboard.rs` directly reads many `.roko` files.
- `tui/dashboard.rs` and `tui/views/git_view.rs` run git commands.

Target:

- TUI uses `RuntimeCommandClient` for commands.
- TUI uses `RuntimeQueryClient` for projections.
- File watchers become fallback/recovery adapters, not primary query logic.

Checklist:

- [ ] Define `RuntimeCommandClient` with local in-process and HTTP implementations.
- [ ] Route TUI inject/confirm through `submit_tui_directive`.
- [ ] Route TUI job creation through `create_job`.
- [ ] Route TUI plan actions through typed plan commands.
- [ ] Replace dashboard direct readers with projection calls.
- [ ] Keep `.roko` file tailers only as offline compatibility mode.
- [ ] Add TUI proof that job creation appears through the same `marketplace_jobs` projection as HTTP.
- [ ] Add TUI proof that inject/confirm produces durable command events, not raw untyped engrams only.

### P0-06 Move Plan Review Git Operations Into Git/Merge Services

Problem:

Plan review routes implement their own git diff and merge path.

Evidence:

- `find_agent_branch`, `diff_summary`, `parse_git_diff`, and `merge_branch` live in `routes/plans.rs`.
- Review approval calls `git merge --no-ff` directly.
- It does not use runner merge queue, typed conflict evidence, post-merge regression, or workspace policy.

Target:

- Plan review routes call `GitService` for read-only diff views.
- Merge approval calls `MergePolicyEngine` or a shared `MergeService`.

Checklist:

- [ ] Add `GitQueryService` with branch search, diff summary, structured diff, status, worktree list.
- [ ] Add `MergeCommandService` or reuse `MergePolicyEngine`.
- [ ] Replace direct route git commands.
- [ ] Return typed conflict evidence on merge failure.
- [ ] Run post-merge regression before marking review approved.
- [ ] Add merge queue conflict proof through HTTP review approval.
- [ ] Add grep gate: `rg -n "tokio::process::Command::new\\(\"git\"\\)|std::process::Command::new\\(\"git\"\\)" crates/roko-serve/src/routes crates/roko-cli/src/tui` should return zero outside `GitService`.

### P0-07 Remove Synthetic Success From Job Runner Strict Paths

Problem:

Job runner fallback behavior can make unsupported real features look successful.

Evidence:

- Chain monitor and chain analysis use synthetic events.
- Coding jobs synthesize fallback plans when runtime PRD planning is unavailable.
- Missing structured gate results become `"runtime": true`.

Target:

- In normal/dev mode, synthetic fallback can exist but must be labeled.
- In strict/Mori-parity mode, unsupported real execution returns `unsupported` or fails proof.

Checklist:

- [ ] Add `ExecutionMode::{DevFallback, Normal, Strict, MoriParity}`.
- [ ] Tag fallback outputs as `fallback_synthetic`, `fallback_plan`, or `unstructured_gate`.
- [ ] In `Strict` and `MoriParity`, disable synthetic chain events.
- [ ] In `Strict` and `MoriParity`, disable synthesized coding plans unless explicitly requested.
- [ ] Replace `"runtime": true` default gate with `gate_status: unstructured` or `unsupported`.
- [ ] Add proof that strict job execution fails when real chain/provider/planner inputs are missing.
- [ ] Add proof that dev fallback results are visible as fallback in projections.

### P0-08 Projection Service Is Rich But Not Yet The Only Query Path

Problem:

Projection endpoints exist, but many routes and TUI views still read their own sources.

Evidence:

- `/api/projections/{name}` uses `RuntimeProjectionSet`.
- Some status/learning endpoints use projections.
- Other routes read `.roko` files directly.
- TUI direct readers duplicate many projection calculations.
- `RuntimeProjectionSet::load` directly opens files rather than calling repositories.

Target:

- `RuntimeProjectionService` is the only query aggregation path.
- Routes and TUI use it.
- File loading is behind repositories.

Checklist:

- [ ] Move `RuntimeProjectionSet` into a shared runtime/query crate or service module.
- [ ] Add repository-backed loading for events, episodes, efficiency, costs, provider outcomes, knowledge, jobs, PRDs, plans, operations, reviews, and git.
- [ ] Add projections for `operations`, `jobs`, `reviews`, `prd_lifecycle`, `plan_artifacts`, `merge_review`, and `tui_commands`.
- [ ] Update status routes to use `RuntimeQueryService`.
- [ ] Update TUI dashboard to use `RuntimeQueryService`.
- [ ] Add proof that HTTP `/api/projections/*`, status routes, and TUI render the same source ids and cursor.

## P1 Findings And Checklists

### P1-01 Server Event Bus And StateHub Need One Durable Event Source

Current situation:

- `EventBus<ServerEvent>` is live-only plus replay ring.
- `SharedStateHub` persists dashboard events to `.roko/events.jsonl`.
- Runner events also use `.roko/events.jsonl`.
- Routes manually publish `DashboardEvent`s.

Checklist:

- [ ] Define canonical `RuntimeEventEnvelope` for serve, runner, job, PRD, plan, and TUI command events.
- [ ] Bridge `ServerEvent` to runtime events or retire it as an internal transport.
- [ ] Make `StateHub` a projection sink, not a parallel event source.
- [ ] Add event-source field to every projection row.
- [ ] Prove that a run event appears once with stable id across HTTP, SSE, TUI, and JSONL.

### P1-02 Server Startup Needs A Supervised Service Registry

Current situation:

- `ServerBuilder::start_background` starts dispatch loop, builtin sources, config watcher, PRD subscriber, feedback loop, StateHub bridge, state saver, job runner, cold archival, JWKS prime, relay registration, and chain watcher.
- Many handles are assigned to `_` variables and rely on cancellation side effects.

Checklist:

- [ ] Add `ServiceRegistry` with service name, kind, health, last tick, cancellation, and join handle.
- [ ] Register every startup loop.
- [ ] Expose `/api/services`.
- [ ] Emit service lifecycle events.
- [ ] Add restart/shutdown proof.

### P1-03 Operation Auth/Policy Should Be Resolved Before Command Execution

Current situation:

- Route middleware handles HTTP auth.
- Runtime execution policy, unsafe mode, provider policy, and route permissions are not one command authorization record.

Checklist:

- [ ] Add `CommandPolicyContext` with caller identity, scopes, workdir, repo, unsafe policy, provider policy, budget, and operation kind.
- [ ] Attach it to every `RuntimeCommand`.
- [ ] Persist redacted policy context id with operation events.
- [ ] Reject unsafe operations before background spawn.

### P1-04 Route-Level Retention And Snapshot Policies Need Repositories

Current situation:

- Retention, deployments, team, secrets, templates, feeds, connectors, and auth each own path conventions.

Checklist:

- [ ] Add repository contracts for each domain.
- [ ] Add migration metadata and schema versions.
- [ ] Make retention operate on repository catalogs, not hardcoded path lists.
- [ ] Add proof that retention does not delete active operation evidence.

## Implementation Plan

### Phase 0 - Freeze Route-Level Runtime Ownership

- [ ] Add a developer note in `routes/plans.rs`, `routes/prds.rs`, `routes/run.rs`, and `job_runner.rs`: new runtime behavior belongs in command services.
- [ ] Add grep audit to CI for route-level `runtime.run_once`, route-level `tokio::spawn`, and route-level `.roko` writes.
- [ ] Create tracking issues from this doc.

### Phase 1 - Add Shared Runtime API

- [ ] Add `RuntimeCommandService`.
- [ ] Add `RuntimeQueryService`.
- [ ] Add command types for run, workflow, PRD draft, PRD promote, plan generation, plan execution, plan review, job creation, job execution, research, dream, and TUI directives.
- [ ] Add `OperationRef`, `ArtifactRef`, `CommandAck`, and error statuses.
- [ ] Implement adapters using existing CLI/runtime code without changing behavior.

### Phase 2 - Durable Operation Store

- [ ] Add operation event log and snapshot.
- [ ] Emit operation events from existing route code.
- [ ] Update status endpoints to read operation projections.
- [ ] Add operation projection and SSE stream.
- [ ] Add restart proof.

### Phase 3 - Route Migration

- [ ] Migrate `/api/run`.
- [ ] Migrate `/api/prds/*`.
- [ ] Migrate `/api/plans/*`.
- [ ] Migrate `/api/research`.
- [ ] Migrate `/api/jobs`.
- [ ] Migrate `/api/dream`.
- [ ] Migrate gateway batches and template actions.

### Phase 4 - Repository Migration

- [ ] Add repositories for PRD, plan, job, review, research, deployment, template, team, auth, secrets, feeds, and connectors.
- [ ] Move direct `.roko` access behind repositories.
- [ ] Add repository schema version and source evidence.
- [ ] Add migration proof for existing workspace layout.

### Phase 5 - TUI Migration

- [ ] Add local runtime command/query client.
- [ ] Replace TUI direct command writes.
- [ ] Replace dashboard direct readers with projections.
- [ ] Keep file tailers as offline fallback.
- [ ] Add TUI/HTTP projection parity proof.

### Phase 6 - Git/Merge Service

- [ ] Add Git query service.
- [ ] Add merge command service using runner merge policy.
- [ ] Replace direct route/TUI git subprocesses except read-only GitService internals.
- [ ] Add review merge proof.

### Phase 7 - Strict Proof Mode

- [ ] Add strict/Mori-parity mode to server/job runner.
- [ ] Disable synthetic fallback success.
- [ ] Classify unsupported real integrations.
- [ ] Add proof bundle for HTTP, TUI, jobs, PRDs, plans, projections, operations, and merge review.

## Proof Matrix Required Before Claiming Completion

| Proof | Required Evidence |
| --- | --- |
| HTTP run | `/api/run` returns operation id, durable operation event, runner event, projection row, status survives restart. |
| HTTP PRD draft | `/api/prds/{slug}/draft` creates typed artifact, operation events, projection update, no route prompt fallback. |
| HTTP PRD promote and auto-plan | Promote writes PRD through repository, emits PRD event, queues plan generation operation, no duplicate subscriber loop. |
| HTTP plan execute | `/api/plans/{id}/execute` invokes runner-v2 through command service, not `run_once` prompt fallback. |
| HTTP plan resume | Resume uses runner resume token/state, not a natural-language resume prompt. |
| HTTP plan review merge | Review approval uses merge service, returns typed conflict/regression evidence. |
| Job create/execute | Job repository write, operation event, runner/provider proof, no synthetic success in strict mode. |
| TUI job creation | TUI command goes through command service and appears in HTTP `marketplace_jobs` projection. |
| TUI inject/confirm | TUI command produces typed command event and runtime effect, not only raw `engrams.jsonl`. |
| Projection parity | HTTP projection, TUI dashboard, and JSONL evidence share source ids/cursors. |
| Server restart | Running/completed/failed operations remain queryable after restart. |
| Shutdown | All supervised services and operation tasks receive cancellation and final state. |
| Strict fallback | Synthetic chain/fallback plan/unstructured gates are rejected or marked unsupported in strict mode. |

Suggested proof commands:

```bash
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case http-run-operation
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case http-prd-to-plan
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case http-plan-execute-runner
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case http-plan-review-merge
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case tui-job-command
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case operation-restart
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case serve-shutdown
tests/proof/mori-diffs/prove-runtime-end-to-end.sh --case strict-no-synthetic-success
```

## Grep Gates For Implementation Agents

Use these while implementing. They are not proof alone, but they catch regressions.

```bash
rg -n "runtime\\.run_once\\(" crates/roko-serve/src/routes crates/roko-serve/src/job_runner.rs
rg -n "tokio::spawn" crates/roko-serve/src/routes crates/roko-serve/src/job_runner.rs crates/roko-serve/src/lib.rs
rg -n "join\\(\"\\.roko\"\\)|\"\\.roko/" crates/roko-serve/src/routes crates/roko-serve/src/job_runner.rs crates/roko-cli/src/tui
rg -n "tokio::process::Command::new\\(\"git\"\\)|std::process::Command::new\\(\"git\"\\)" crates/roko-serve/src/routes crates/roko-cli/src/tui
rg -n "synthesize_coding_plan|MockChainClient|runtime.*true|unstructured gate|plan-execution prompt" crates/roko-serve/src/job_runner.rs crates/roko-serve/src/runtime.rs
rg -n "RuntimeCommandService|RuntimeQueryService|RuntimeOperationStore|BackgroundTaskSupervisor|GitQueryService|MergeCommandService" crates/roko-serve/src crates/roko-cli/src crates/roko-runtime/src
```

Target after implementation:

- [ ] First grep returns zero route/job-runner command paths except `StartRunCommand`.
- [ ] Second grep returns only supervisor internals and tests.
- [ ] Third grep returns only repository implementations, layout modules, migration, and tests.
- [ ] Fourth grep returns only GitService internals and tests.
- [ ] Fifth grep returns no strict-mode success fallbacks.
- [ ] Sixth grep shows shared services active in HTTP and TUI.

## Agent Handoff Checklist

### Small Batch A - Runtime Command Service

- [ ] Add service trait and command types.
- [ ] Implement CLI-backed service using current internals.
- [ ] Wire `/api/run` through it.
- [ ] Add operation event output for `/api/run`.

### Small Batch B - Operation Store

- [ ] Add durable operation event log.
- [ ] Add materialized operation projection.
- [ ] Migrate `/api/operations/{id}`.
- [ ] Add restart proof.

### Small Batch C - PRD And Plan Commands

- [ ] Add typed PRD/plan service methods.
- [ ] Replace PRD route prompt fallbacks.
- [ ] Replace plan route prompt fallbacks.
- [ ] Add HTTP PRD-to-plan and plan-execute proof.

### Small Batch D - Repositories

- [ ] Add PRD/plan/job/review repositories.
- [ ] Replace route `.roko` writes in migrated paths.
- [ ] Add repository evidence metadata.
- [ ] Add migration proof.

### Small Batch E - TUI Command/Query Client

- [ ] Add local runtime client.
- [ ] Migrate job create and inject/confirm.
- [ ] Migrate dashboard high-value projections.
- [ ] Add TUI/HTTP parity proof.

### Small Batch F - Git/Merge Service

- [ ] Add Git query service.
- [ ] Route review diff through Git service.
- [ ] Route review approval through merge service.
- [ ] Add conflict/regression proof.

### Small Batch G - Strict Job Runner

- [ ] Add execution mode policy.
- [ ] Mark synthetic fallback outputs.
- [ ] Reject fallback success in strict/Mori-parity mode.
- [ ] Add strict proof.

## Acceptance Criteria

The work is complete only when all are true:

- [ ] HTTP routes do not own runtime execution policy.
- [ ] HTTP routes do not spawn long-running work directly.
- [ ] HTTP routes do not write `.roko` artifacts directly outside repository adapters.
- [ ] TUI submits commands through runtime command client.
- [ ] TUI reads runtime state through projection/query client by default.
- [ ] Operation status survives server restart.
- [ ] PRD, plan, job, research, review, and run workflows have typed operation events.
- [ ] Plan execution through HTTP uses runner-v2, not prompt fallback.
- [ ] Plan review merge uses shared merge policy and typed conflict/regression evidence.
- [ ] Strict/Mori-parity mode cannot pass by using synthetic chain data, synthesized fallback plans, or unstructured gate defaults.
- [ ] HTTP, SSE, TUI, and JSONL proof agree on operation ids, cursors, source evidence, and terminal status.

## Initial Self-Grade And Iteration Proof

Initial self-grade before adding implementation batches and strict-mode fallback analysis: `9.36/10`.

Reason it was not high enough:

- It identified that routes were too fat but did not distinguish good seams from bad ones.
- It did not explicitly cover TUI as a command writer and query engine.
- It did not call out synthetic job-runner success as a proof blocker.
- It did not give enough service contracts for agents to implement without broader context.

Iteration performed:

- Added scan counts and hotspot table.
- Added source-specific evidence for `CliRuntime`, `RuntimeProjectionSet`, `AppState`, plan routes, PRD routes, job runner, and TUI.
- Added target command/query/operation/repository/supervisor contracts.
- Added P0/P1 implementation checklists.
- Added proof matrix and grep gates.
- Added small batch handoff sections.

Final self-grade: `9.83/10`.

Why not `10/10`:

- This is an audit and handoff, not the implementation.
- Exact service crate placement should be decided alongside the existing `roko-runtime` and runner extraction work.
- Some route-specific details, especially deployments/auth/secrets/team, need their own repository migration passes after the shared service layer exists.

## 2026-04-27 Deepening Pass - Adapter Authority And Query Contract

The previous pass correctly identified that serve and TUI are still too fat. This pass tightens the implementation boundary: serve and TUI should not merely be "thinner"; they should be forbidden from owning command execution, lifecycle state, artifact storage, git/process execution, or projection reconstruction. They should be adapters over the runtime services defined in [35-TASK-PROCESS-LIFECYCLE-AUDIT.md](35-TASK-PROCESS-LIFECYCLE-AUDIT.md), [36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md](36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md), [37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md](37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md), and [41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md).

The clean design is a shared command/query contract:

```text
CLI command / HTTP route / TUI action / websocket request
  -> RuntimeCommandService::submit(CommandEnvelope)
  -> RuntimeTaskSupervisor / WorkflowEngine / InferenceGateway / ArtifactStore
  -> durable lifecycle + artifact + projection events
  -> RuntimeQueryService::query(QueryEnvelope)
  -> HTTP JSON / SSE / websocket / TUI view / proof bundle
```

### Adapter Drift A1 - TUI Still Writes Runtime Commands Directly

Evidence:

```text
crates/roko-cli/src/tui/app.rs:1228 TuiAction::SubmitInject
crates/roko-cli/src/tui/app.rs:1322 TuiAction::ConfirmYes
crates/roko-cli/src/tui/app.rs:1675 submit_marketplace_job
crates/roko-cli/src/tui/app.rs:2486 std::fs::write marketplace job
crates/roko-cli/src/tui/views/dashboard_view.rs:2211 std::fs::write
```

Problem:

- [ ] TUI actions can mutate `.roko` files without runtime command validation.
- [ ] Inject/confirm/job-create actions can bypass idempotency, authorization, policy checks, event correlation, and operation lifecycle.
- [ ] TUI success can mean "file write succeeded", not "runtime accepted and processed the command".
- [ ] Command errors are UI-local and cannot be queried through HTTP proof endpoints.

Implementation checklist:

- [ ] Add `RuntimeCommandClient` for TUI with local and remote implementations.
- [ ] Replace TUI inject writes with `RuntimeCommand::AppendDirective { directive_kind, payload, source: Tui }`.
- [ ] Replace TUI confirm writes with `RuntimeCommand::ConfirmOperation { operation_id, decision, source: Tui }`.
- [ ] Replace marketplace job JSON writes with `RuntimeCommand::CreateJob`.
- [ ] Return `CommandAck { command_id, operation_id, accepted_at, validation, projection_cursor }`.
- [ ] Render command acceptance and later terminal outcome separately in TUI.
- [ ] Record all TUI commands in the operation/event projection with `surface = "tui"`.

Acceptance proof:

- [ ] Trigger inject, confirm, and marketplace job actions from TUI.
- [ ] Query each command by `command_id` over HTTP.
- [ ] Prove no direct TUI write was needed for runtime command acceptance.

### Adapter Drift A2 - TUI Is Still A Query Engine

Evidence:

```text
crates/roko-cli/src/tui/app.rs:618 watch_roko_dir_with_fallback
crates/roko-cli/src/tui/app.rs:624 watch_git_repo_with_fallback
crates/roko-cli/src/tui/app.rs:2327 refresh_snapshot
crates/roko-cli/src/tui/state.rs:1503 std::fs::read_to_string
crates/roko-cli/src/tui/state.rs:1849 fallback_route_metrics_for_agent
crates/roko-cli/src/tui/dashboard.rs:577 spawn_blocking refresh
crates/roko-cli/src/tui/dashboard.rs:804 std::fs::read_to_string
crates/roko-cli/src/tui/dashboard.rs:944 load_dashboard_git_diff
crates/roko-cli/src/tui/dashboard.rs:960 Command::new("git")
crates/roko-cli/src/tui/views/git_view.rs:548 Command::new("git")
```

Problem:

- [ ] TUI reconstructs state by watching `.roko`, reading many files, and running git.
- [ ] TUI can disagree with HTTP projections and durable runner events.
- [ ] UI fallback metrics can mask missing live runtime data.
- [ ] Rendering and refresh code becomes an accidental second projection engine.
- [ ] Remote TUI mode cannot work cleanly if local filesystem reads are authoritative.

Implementation checklist:

- [ ] Add `RuntimeQueryClient` with local projection-backed and HTTP-backed implementations.
- [ ] Replace dashboard file readers with `RuntimeQuery::DashboardSnapshot`.
- [ ] Replace git view direct `git` calls with `RuntimeQuery::GitStatus` and `RuntimeQuery::GitDiff`.
- [ ] Replace fallback route metrics with explicit `ProjectionCompleteness { complete, missing_sources, stale_sources }`.
- [ ] Keep filesystem watchers only as invalidation hints for local mode, never as the source of truth.
- [ ] Add a TUI setting for `query_mode = local | http`, with both paths hitting the same projection schema.
- [ ] Make every TUI panel declare the projection names it requires.

Acceptance proof:

- [ ] TUI dashboard, git view, plans view, agents view, marketplace view, and logs view render from projection DTOs.
- [ ] Disconnect local `.roko` direct reads in a proof mode and show TUI can still render from HTTP projections.
- [ ] Compare TUI snapshot JSON with HTTP projection JSON and prove matching cursors.

### Adapter Drift A3 - Serve Status Routes Still Read Private State

Evidence:

```text
crates/roko-serve/src/routes/status/dashboard.rs:56 GET /api/operations/:id reads operations
crates/roko-serve/src/routes/status/health.rs:17 active_plans
crates/roko-serve/src/routes/status/health.rs:22 active_runs
crates/roko-serve/src/routes/status/metrics.rs:152 active_plans
crates/roko-serve/src/routes/status/metrics.rs:388 active_plans
crates/roko-serve/src/routes/agents.rs:1491 active_runs
crates/roko-serve/src/routes/gateway.rs:840 operations count as active_agents
```

Problem:

- [ ] HTTP status answers still depend on serve-local maps, not durable operation projections.
- [ ] Different endpoints count "active" differently.
- [ ] Provider/gateway active-agent counts are derived from operation map size instead of task/process/provider lifecycle.
- [ ] A restart changes answers even if durable events exist elsewhere.

Implementation checklist:

- [ ] Replace every status route read of `active_runs`, `active_plans`, or `operations` with `RuntimeQueryService`.
- [ ] Define `RuntimeStatusSummary` with active operations, active tasks, active processes, active provider calls, queue depth, service health, and projection lag.
- [ ] Define `OperationLookup` for `/api/operations/{id}` backed by the operation projection from doc 35.
- [ ] Define `ProviderActivitySummary` backed by inference gateway events from doc 41.
- [ ] Keep live map counts only as debug fields under `debug.live_handles`, clearly marked non-authoritative.
- [ ] Add restart tests proving status survives process restart.

Acceptance proof:

- [ ] Start operations, query status routes, restart server, query again.
- [ ] Counts match durable projections and no endpoint requires live `JoinHandle` maps.
- [ ] Gateway active-provider/agent counts match provider lifecycle events, not generic operation map length.

### Adapter Drift A4 - Event Bus, StateHub, Projections, And OpenAPI Are Not One Contract

Evidence:

```text
crates/roko-serve/src/routes/projections.rs:39 RuntimeProjectionSet::load
crates/roko-serve/src/routes/projections.rs:49 RuntimeProjectionSet::load
crates/roko-serve/src/projection_contract.rs:467 RuntimeProjectionSet
crates/roko-serve/src/projection_contract.rs:527 RuntimeProjectionSet::load
crates/roko-serve/src/routes/sse.rs:46 state_hub.subscribe_events
crates/roko-serve/src/routes/ws.rs:78 event_bus.replay_from
crates/roko-serve/src/routes/ws.rs:89 event_bus.subscribe
crates/roko-serve/src/openapi.rs:459 PlanCreateRequest
crates/roko-serve/src/openapi.rs:478 RunRequest
```

Problem:

- [ ] Projection endpoints, SSE, websocket, StateHub, EventBus, and OpenAPI DTOs are adjacent but not the same command/query contract.
- [ ] A feature can be queryable through one surface but invisible or differently shaped through another.
- [ ] Event streams can expose broadcast events that are not durable lifecycle/projection events.
- [ ] OpenAPI describes route request DTOs, but not the canonical runtime command/query envelopes.

Implementation checklist:

- [ ] Define `CommandEnvelope` and `QueryEnvelope` as canonical API contracts.
- [ ] Generate route DTOs, OpenAPI schemas, TUI client DTOs, and proof schemas from those contract types.
- [ ] Make SSE/websocket stream projection deltas, not raw divergent event bus variants.
- [ ] Make `RuntimeProjectionSet::load` delegate to `RuntimeQueryService`, not directly assemble from mixed state.
- [ ] Add projection names for operations, tasks, processes, services, artifacts, workflow runs, model calls, provider health, merge evidence, and command acknowledgements.
- [ ] Add cursor semantics common to HTTP polling, SSE, websocket, and TUI refresh.
- [ ] Mark EventBus/StateHub as delivery/cache mechanisms, not canonical sources.

Acceptance proof:

- [ ] The same operation id can be queried through HTTP JSON, SSE delta stream, websocket replay, and TUI snapshot.
- [ ] All surfaces report the same projection cursor and terminal status.
- [ ] OpenAPI includes command/query envelopes and proof endpoints.

### Adapter Drift A5 - Serve Route Runtime Methods Still Permit Prompt Fallbacks

Evidence:

```text
crates/roko-serve/src/runtime.rs:142 CliRuntime trait
crates/roko-serve/src/routes/plans.rs:222 runtime.run_once
crates/roko-serve/src/routes/plans.rs:380 runtime.run_once
crates/roko-serve/src/routes/plans.rs:507 runtime.run_once
crates/roko-serve/src/routes/plans.rs:1162 runtime.run_once
crates/roko-serve/src/routes/prds.rs:500 runtime.run_once
crates/roko-serve/src/routes/prds.rs:685 runtime.run_once
crates/roko-serve/src/routes/prds.rs:913 runtime.run_once
crates/roko-serve/src/routes/research.rs:222 runtime.run_once
crates/roko-serve/src/routes/templates.rs:156 runtime.run_once
crates/roko-serve/src/job_runner.rs:281 runtime.run_once
```

Problem:

- [ ] `run_once` is too powerful and too vague for HTTP route code.
- [ ] It lets a route implement a workflow by asking an agent to do work in text, bypassing typed workflow state, artifacts, gates, retries, merge, resume, and proof.
- [ ] Feature completeness becomes unknowable because "agent got a prompt" looks like a wired feature.

Implementation checklist:

- [ ] Deprecate `CliRuntime::run_once` for serve route use.
- [ ] Replace route calls with typed commands: `StartRun`, `ExecutePlan`, `ResumePlan`, `GeneratePlan`, `ChatPlan`, `DraftPrd`, `PromotePrd`, `ResearchTopic`, `DeployTemplate`, `RunJob`.
- [ ] Allow freeform `RunOnce` only as an explicit interactive command kind with strict proof labels, not as internal implementation glue.
- [ ] Make strict/Mori-parity mode reject route-owned prompt fallback for typed workflows.
- [ ] Add a compile-time or lintable allowlist for `runtime.run_once(` call sites.

Acceptance proof:

- [ ] `rg -n "runtime\\.run_once\\(" crates/roko-serve/src/routes crates/roko-serve/src/job_runner.rs` returns zero unallowlisted matches.
- [ ] HTTP plan execution produces runner-v2 events, not a generic run-once transcript.
- [ ] HTTP PRD-to-plan produces typed workflow artifacts and operation events.

### Adapter Drift A6 - Job Runner Synthetic Success Is An Adapter Proof Blocker

Evidence:

```text
crates/roko-serve/src/job_runner.rs:183 Generic fallback: use description as prompt
crates/roko-serve/src/job_runner.rs:413 MockChainClient::local
crates/roko-serve/src/job_runner.rs:626 synthesizing fallback plan
crates/roko-serve/src/job_runner.rs:647 runtime PRD planning unavailable; synthesizing fallback coding plan
crates/roko-serve/src/job_runner.rs:654 synthesize_coding_plan
```

Problem:

- [ ] The job runner can provide useful demo behavior, but it cannot be considered Mori parity proof if fallback/synthetic paths can complete as success.
- [ ] Adapter surfaces may display job completion without proving real workflow execution, real provider execution, real chain execution, or real artifact production.
- [ ] The system needs explicit execution modes so demo/local/dev behavior cannot masquerade as end-to-end functionality.

Implementation checklist:

- [ ] Add `ExecutionMode::{Demo, LocalDev, Strict, MoriParity}` to runtime command context.
- [ ] Mark every fallback/synthetic job path with `synthetic = true`, `proof_eligible = false`, and source evidence.
- [ ] In `Strict` and `MoriParity`, reject synthetic coding plans, `MockChainClient`, generic prompt fallback, and missing runtime PRD planning.
- [ ] Surface synthetic status in HTTP, SSE, websocket, TUI, and proof bundles.
- [ ] Add proof gates that fail if synthetic paths appear in strict runs.

Acceptance proof:

- [ ] Demo mode can still run synthetic examples but labels them non-proof.
- [ ] Strict mode fails clearly when real runtime dependencies are missing.
- [ ] Mori parity proof artifacts contain no synthetic job completions.

## Canonical Adapter Contracts

Command contract:

```rust
pub trait RuntimeCommandService: Send + Sync {
    async fn submit(&self, command: CommandEnvelope) -> Result<CommandAck>;
    async fn cancel(&self, command: CancelCommand) -> Result<CommandAck>;
    async fn validate(&self, command: CommandEnvelope) -> Result<CommandValidation>;
}
```

Query contract:

```rust
pub trait RuntimeQueryService: Send + Sync {
    async fn query(&self, query: QueryEnvelope) -> Result<QueryResponse>;
    async fn stream(&self, query: QueryEnvelope) -> Result<ProjectionStream>;
    async fn proof_bundle(&self, id: ProofSubjectId) -> Result<ProofBundle>;
}
```

TUI adapter contract:

```rust
pub trait TuiRuntimeClient: Send + Sync {
    async fn submit_action(&self, action: TuiCommandAction) -> Result<CommandAck>;
    async fn load_panel(&self, panel: TuiPanelQuery) -> Result<TuiPanelSnapshot>;
    async fn subscribe(&self, query: TuiSubscription) -> Result<TuiEventStream>;
}
```

HTTP adapter rule:

- [ ] Route handlers parse requests into `CommandEnvelope` or `QueryEnvelope`.
- [ ] Route handlers call the service.
- [ ] Route handlers serialize the service result.
- [ ] Route handlers do not own runtime policy, storage layout, process execution, lifecycle state, or projection assembly.

TUI adapter rule:

- [ ] TUI actions submit commands.
- [ ] TUI panels render query snapshots.
- [ ] TUI watches are invalidation hints only.
- [ ] TUI never writes canonical `.roko` runtime artifacts directly.
- [ ] TUI never runs git/process/model/provider commands directly.

## Adapter Migration Batches

### Batch U1 - Command Envelope And Service

- [ ] Define `CommandEnvelope`, `CommandKind`, `CommandSource`, `CommandAck`, and `CommandValidation`.
- [ ] Implement command submission for `StartRun`.
- [ ] Implement command submission for plan execute/resume/generate/chat.
- [ ] Implement command submission for PRD draft/promote/plan-from-prd.
- [ ] Implement command submission for research/template/deployment/job.
- [ ] Add command ids and correlation ids to lifecycle events.
- [ ] Add OpenAPI schemas for command submission.

### Batch U2 - Query Envelope And Projection Service

- [ ] Define `QueryEnvelope`, `ProjectionCursor`, `ProjectionCompleteness`, and `QueryResponse`.
- [ ] Wrap existing `RuntimeProjectionSet` behind `RuntimeQueryService`.
- [ ] Add operation/task/process/service/artifact/model-call query subjects.
- [ ] Add proof bundle query subject.
- [ ] Make SSE and websocket stream `QueryResponse` deltas.
- [ ] Add OpenAPI schemas for query/proof endpoints.

### Batch U3 - HTTP Route Migration

- [ ] Migrate `/api/run`.
- [ ] Migrate `/api/plans` execute/resume/generate/chat/status/review.
- [ ] Migrate `/api/prds` draft/promote/generate/status.
- [ ] Migrate `/api/research`.
- [ ] Migrate `/api/templates`.
- [ ] Migrate `/api/jobs`.
- [ ] Migrate `/api/gateway` to inference gateway query/command surfaces.
- [ ] Migrate status/health/metrics to query projections.
- [ ] Remove route-owned long-running spawns and direct status maps from migrated paths.

### Batch U4 - TUI Command Migration

- [ ] Implement local `TuiRuntimeClient` over `RuntimeCommandService`.
- [ ] Implement remote `TuiRuntimeClient` over HTTP command/query endpoints.
- [ ] Migrate inject directive action.
- [ ] Migrate confirm action.
- [ ] Migrate marketplace job create/update actions.
- [ ] Migrate plan run/resume/cancel actions.
- [ ] Migrate config save actions to config command service where runtime-affecting.
- [ ] Add command ack rendering and terminal outcome rendering.

### Batch U5 - TUI Query Migration

- [ ] Migrate dashboard summary to projections.
- [ ] Migrate plan/agent/task panels to projections.
- [ ] Migrate git view to Git query service.
- [ ] Migrate marketplace/jobs view to job projections.
- [ ] Migrate logs/events view to projection stream.
- [ ] Migrate config/provider view to config/provider projections.
- [ ] Remove direct `.roko` reads from default panel refresh path.
- [ ] Keep direct local fallback only as explicit offline diagnostic mode.

### Batch U6 - Strict Adapter Proof

- [ ] Proof: HTTP command returns command ack and operation id.
- [ ] Proof: TUI action creates the same command/operation event as HTTP.
- [ ] Proof: HTTP JSON, SSE, websocket, and TUI show the same projection cursor.
- [ ] Proof: restart preserves operation/status answers.
- [ ] Proof: strict mode fails on synthetic job fallbacks.
- [ ] Proof: no route/TUI direct `.roko` writes in migrated command paths.
- [ ] Proof: no route/TUI direct git/process calls in migrated query paths.

## Additional Adapter Grep Gates

```bash
rg -n "std::fs::write|tokio::fs::write|OpenOptions|append_jsonl|write_jsonl" crates/roko-cli/src/tui crates/roko-serve/src/routes crates/roko-serve/src/job_runner.rs -g '*.rs'
rg -n "std::fs::read_to_string|tokio::fs::read_to_string|read_dir|watch_roko_dir_with_fallback|watch_git_repo_with_fallback" crates/roko-cli/src/tui crates/roko-serve/src/routes -g '*.rs'
rg -n "Command::new\\(\"git\"\\)|tokio::process::Command::new\\(\"git\"\\)|std::process::Command::new\\(\"git\"\\)" crates/roko-cli/src/tui crates/roko-serve/src/routes -g '*.rs'
rg -n "runtime\\.run_once\\(" crates/roko-serve/src/routes crates/roko-serve/src/job_runner.rs -g '*.rs'
rg -n "active_runs|active_plans|state\\.operations|OperationHandle|PlanHandle|RunHandle" crates/roko-serve/src/routes crates/roko-serve/src/state.rs -g '*.rs'
rg -n "synthesize_coding_plan|MockChainClient|Generic fallback|synthetic|fallback plan" crates/roko-serve/src/job_runner.rs -g '*.rs'
rg -n "RuntimeCommandService|RuntimeQueryService|CommandEnvelope|QueryEnvelope|TuiRuntimeClient|ProjectionCompleteness" crates/roko-cli/src crates/roko-serve/src crates/roko-runtime/src -g '*.rs'
```

Completion targets:

- [ ] Write grep returns only repository/adapters, tests, or explicit offline diagnostic mode.
- [ ] Read grep returns only query-service/repository implementations, tests, or explicit offline diagnostic mode.
- [ ] Git/process grep returns only Git service internals, managed command backend, tests, or allowlisted diagnostics.
- [ ] `run_once` grep returns no typed workflow implementation paths.
- [ ] Live-map grep returns no authoritative route/status answers.
- [ ] Synthetic grep returns no strict/Mori-parity success paths.
- [ ] Contract grep finds active command/query adapter implementation.

## Updated Self-Grade After Adapter Deepening

Score before this pass: **9.83 / 10**.

Current score after this pass: **9.90 / 10**.

What improved:

- [ ] The audit now defines hard adapter authority rules instead of only recommending thinner routes.
- [ ] It separates command submission, query projection, lifecycle, artifact storage, and proof responsibilities.
- [ ] It pins TUI command writes, TUI query reconstruction, serve status maps, mixed event surfaces, prompt fallback routes, and synthetic job success to source evidence.
- [ ] It gives concrete `RuntimeCommandService`, `RuntimeQueryService`, and `TuiRuntimeClient` contracts.
- [ ] It gives migration batches and grep gates that can be assigned without additional repository context.

Remaining risk:

- [ ] Some low-priority surfaces such as auth, secrets, team, feeds, subscriptions, and aggregator routes may need follow-up repository-specific migrations after the shared command/query contract exists.
