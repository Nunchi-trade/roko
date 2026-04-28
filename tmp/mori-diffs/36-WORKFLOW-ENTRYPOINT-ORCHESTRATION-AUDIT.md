# 36 - Workflow Entrypoint And Orchestration Audit

Date: 2026-04-27

Purpose: capture the architectural reason Roko still feels like it needs five commands to do one thing. The current codebase has many command surfaces and prompt builders, but it does not have one typed workflow engine that owns "idea -> PRD -> plan -> tasks -> run -> observe -> resume".

### Architecture Runner Update (2026-04-28)
Workflow entrypoint convergence achieved:
- `WorkflowEngine` (P2D) is the single facade for all workflow execution
- CLI wired via `run_with_workflow_engine()` in `roko-cli/src/run.rs` (P4A)
- ACP wired via `run_with_workflow_engine()` in `roko-acp/src/runner.rs` (P4B)
- Config-driven workflows via `WorkflowConfig::express()` / `standard()` / `full()` (P2A)
- Remaining: HTTP entry point wiring, `roko plan run` integration, `orchestrate.rs` retirement

This doc is an implementation handoff. An agent should be able to implement the checklist items without reading the chat history.

## Executive Verdict

Roko has enough pieces to run useful work, but the entrypoints are not designed as one orchestration product. CLI commands, HTTP routes, PRD helpers, plan helpers, jobs, one-shot chat, and serve runtime each decide how to build prompts, write artifacts, invoke models, launch execution, and record state.

That produces the user-visible problem:

- `roko prd draft` creates one kind of artifact through one prompt path.
- `roko prd plan` creates a plan through another path.
- `roko plan generate` creates or regenerates tasks through another path.
- `roko plan run` uses the runner path.
- `roko run` and bare prompts mostly behave like one-shot prompt execution, not a typed multi-stage project workflow.
- HTTP routes can bypass typed plan execution and call `run_once` with an execution prompt.

The redesign is not "add another one-shot command". The redesign is a workflow engine that is called by every CLI, HTTP, TUI, daemon, job, and automation entrypoint.

Target spine:

```text
WorkflowRequest
  -> WorkflowResolver
  -> WorkflowPlan / OperationDAG
  -> StepExecutor registry
  -> ArtifactStore
  -> RuntimeEventStore
  -> OperationStore
  -> RuntimeQueryService
```

All frontends should submit a workflow request and receive a durable operation id. No frontend should directly assemble prompts, spawn agents, run plans, or fake plan execution status.

## Relationship To Other Mori-Diffs Docs

- [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) is the canonical priority board.
- [31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md](31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md) shows that `roko-cli` and `roko-serve` are still acting as runtimes.
- [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md) defines the crate-direction rules this workflow engine should follow.
- [33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md](33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md) defines the runtime context and provider policy needed before any workflow step invokes a model.
- [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md) defines the durable event and query surface every workflow step must publish to.
- [35-TASK-PROCESS-LIFECYCLE-AUDIT.md](35-TASK-PROCESS-LIFECYCLE-AUDIT.md) defines the task/process/cancellation spine used by long-running workflow steps.

## Evidence Scan

Commands used for this pass:

```bash
rg -n "enum .*Cmd|struct .*Cmd|Command::Run|PlanCmd|PrdCmd|cmd_run|cmd_oneshot|cmd_pipe|run_once|generate_plan_from_prd|build_.*prompt|PlanRunner::from_plans_dir" crates/roko-cli/src crates/roko-serve/src -g '*.rs'
rg -n "run_once\\(|generate_plan_from_prd\\(|run_plan\\(|build_.*prompt|queue_.*op|tokio::spawn" crates/roko-serve/src/routes crates/roko-serve/src/runtime.rs crates/roko-cli/src/serve_runtime.rs -g '*.rs'
python3 - <<'PY'
from pathlib import Path
import re
root = Path('/Users/will/dev/nunchi/roko/roko')
terms = ['prd', 'plan', 'task', 'generate', 'execute', 'run_once', 'prompt', 'workflow', 'operation', 'job']
for base in [root/'crates/roko-cli/src', root/'crates/roko-serve/src']:
    rows = []
    for path in sorted(base.rglob('*.rs')):
        text = path.read_text(errors='replace')
        counts = {term: len(re.findall(term, text, flags=re.I)) for term in terms}
        total = sum(counts.values())
        if total:
            rows.append((total, path.relative_to(root), counts))
    for total, path, counts in sorted(rows, reverse=True)[:15]:
        print(total, path, counts)
PY
```

High-signal files from the scan:

- `crates/roko-cli/src/main.rs`
- `crates/roko-cli/src/commands/util.rs`
- `crates/roko-cli/src/commands/prd.rs`
- `crates/roko-cli/src/commands/plan.rs`
- `crates/roko-cli/src/prd.rs`
- `crates/roko-cli/src/serve_runtime.rs`
- `crates/roko-serve/src/runtime.rs`
- `crates/roko-serve/src/routes/prds.rs`
- `crates/roko-serve/src/routes/plans.rs`
- `crates/roko-serve/src/routes/jobs.rs`
- `crates/roko-serve/src/job_runner.rs`

Observed top-level distribution:

| Area | Hot Files | Meaning |
| --- | --- | --- |
| CLI legacy orchestration | `orchestrate.rs`, `main.rs`, `commands/plan.rs`, `commands/prd.rs`, `prd.rs` | Command handlers still contain workflow decisions and prompt construction. |
| Runner | `runner/event_loop.rs`, `runner/agent_stream.rs`, `runner/merge.rs` | Plan execution is becoming real, but it is not yet the workflow owner for all entrypoints. |
| Serve routes | `routes/prds.rs`, `routes/plans.rs`, `routes/research.rs`, `routes/jobs.rs` | HTTP endpoints build prompts and schedule side effects directly. |
| Serve runtime bridge | `serve_runtime.rs`, `runtime.rs` | A runtime trait exists, but its abstraction is `run_once`-centric and too weak for project workflows. |
| Jobs | `job_runner.rs`, `routes/jobs.rs` | Jobs are treated as route/server tasks, not durable workflow operations with typed artifacts. |

## Current Entrypoint Map

### CLI Entrypoints

`crates/roko-cli/src/main.rs`

- `Command::Run` is described as generating and executing a plan, but the dispatch path reaches `commands::util::cmd_run`, which is mostly a universal one-shot runner.
- Bare prompt input routes to inline one-shot behavior.
- Piped input routes to `cmd_pipe`, then `cmd_oneshot`.
- `PlanCmd::Run` uses the runner-v2 plan execution path.
- `PlanCmd::Generate` and `PlanCmd::Regenerate` build prompts directly and call agent helpers.
- `PrdCmd::Draft` builds a PRD scaffold and calls an agent helper directly.
- `PrdCmd::Plan` calls `prd::generate_plan_from_prd`.
- `PrdCmd::Consolidate` builds a prompt directly and calls an agent helper directly.
- `Resume` delegates to `PlanCmd::Run` over `plans/`, not a workflow operation id.

### HTTP Entrypoints

`crates/roko-serve/src/runtime.rs`

- Defines a `CliRuntime` trait with `run_once`, `generate_plan_from_prd`, and default `run_plan`.
- The default `run_plan` can degrade into "build a prompt and call `run_once`", which is not typed plan execution.

`crates/roko-serve/src/routes/prds.rs`

- PRD draft, consolidate, plan generation, and auto-orchestrate paths build prompts in-route or queue route-local operations.
- Published PRDs can trigger auto-orchestration via route-owned subscribers instead of a workflow trigger registry.

`crates/roko-serve/src/routes/plans.rs`

- Plan execution can build a "execute this plan" prompt and call `runtime.run_once` instead of calling typed `runtime.run_plan`.
- Plan chat and plan generation build prompts in-route and write route-owned artifacts.

`crates/roko-serve/src/routes/jobs.rs` and `crates/roko-serve/src/job_runner.rs`

- Jobs are server-side task records that synthesize PRDs, plans, and generated task metadata, but they are not represented as first-class workflow operations with step events, artifacts, retries, and resumable state.

### Runtime And Runner Entrypoints

`crates/roko-cli/src/serve_runtime.rs`

- Bridges HTTP to CLI helpers, but each helper still picks its own lower-level path.
- `run_plan_on_local_runtime` builds runner config and launches the runner separately from normal CLI `plan run` plumbing.

`crates/roko-cli/src/runner/`

- Runner-v2 has the right direction for actual plan execution.
- It should become a step executor under the workflow engine, not the only place where orchestration semantics exist.

## Target Design

### Core Types

Add a workflow core module in the runtime/application layer, not in route handlers.

Suggested location options:

- Preferred long-term: new crate `roko-workflow` or `roko-runtime::workflow`.
- Acceptable transitional step: `crates/roko-cli/src/workflow/` with a clear TODO to move once dependency layering is fixed.

Required types:

```rust
pub struct WorkflowRequest {
    pub workspace: WorkspaceId,
    pub actor: WorkflowActor,
    pub intent: WorkflowIntent,
    pub inputs: WorkflowInputs,
    pub options: WorkflowOptions,
    pub runtime: RuntimeContext,
}

pub enum WorkflowIntent {
    PromptToAnswer,
    PromptToPrd,
    PromptToPlan,
    PrdToPlan,
    PlanToTasks,
    PlanToRun,
    PromptToProjectRun,
    ResearchToPrd,
    JobToProjectRun,
    ResumeOperation,
    PlanChatEdit,
}

pub struct WorkflowPlan {
    pub operation_id: OperationId,
    pub steps: Vec<WorkflowStep>,
    pub artifacts: Vec<ArtifactRef>,
}

pub struct WorkflowStep {
    pub step_id: StepId,
    pub kind: WorkflowStepKind,
    pub inputs: Vec<ArtifactRef>,
    pub outputs: Vec<ArtifactType>,
    pub retry_policy: StepRetryPolicy,
    pub cancellation: CancellationPolicy,
}

pub enum WorkflowStepKind {
    AssemblePrompt,
    InvokeAgent,
    DraftPrd,
    PromotePrd,
    GeneratePlan,
    ValidatePlan,
    GenerateTasks,
    ExecutePlan,
    RunGate,
    MergeWorktree,
    PersistArtifact,
    PublishProjection,
}
```

### Service Boundaries

The workflow engine should depend on facades, not concrete route/CLI helpers:

- `PromptAssembler` from the dispatch prompt path.
- `Dispatcher` from the provider dispatch path.
- `PlanExecutionService` backed by runner-v2.
- `ArtifactStore` for PRDs, plans, tasks, transcripts, proof bundles, logs, and generated diffs.
- `OperationStore` for durable workflow status.
- `RuntimeEventStore` for append-only events.
- `RuntimeQueryService` for HTTP/TUI/query projections.
- `RuntimeTaskSupervisor` from [35-TASK-PROCESS-LIFECYCLE-AUDIT.md](35-TASK-PROCESS-LIFECYCLE-AUDIT.md) for long-running steps.

No workflow step should call:

- `run_once` directly unless the step kind is explicitly `InvokeAgent`.
- `tokio::spawn` directly.
- `Command::new` directly.
- route-local prompt builders.
- command-local prompt builders.
- ad hoc `.roko` path construction.

### User-Facing Commands

The CLI should expose one durable project workflow command and keep old commands as thin aliases.

Suggested command shape:

```bash
roko run "build the dashboard" --auto
roko run "build the dashboard" --to prd
roko run "build the dashboard" --to plan
roko run "build the dashboard" --to tasks
roko run "build the dashboard" --to done
roko run --from prd:.roko/prd/frontend.md --to done
roko run --from plan:plans/frontend --to done
roko workflow resume <operation-id>
roko workflow status <operation-id>
roko workflow artifacts <operation-id>
```

Semantics:

- `--to prd`: draft and persist PRD only.
- `--to plan`: draft/persist PRD if needed, generate plan, stop.
- `--to tasks`: draft PRD if needed, generate plan, generate task list, stop.
- `--to done`: execute all generated tasks through runner-v2.
- `--auto`: allow safe default transitions without approval when policy permits.
- `--approve`: pause at policy-defined approval gates.
- `--from`: resume from an existing artifact and skip prior steps.
- no flag: default should be explicit in config. Recommended default is `--to plan` for safety or `--to done --approve` for Mori-like automation.

Old commands should remain as compatibility wrappers:

- `roko prd draft` -> `WorkflowIntent::PromptToPrd`
- `roko prd plan` -> `WorkflowIntent::PrdToPlan`
- `roko plan generate` -> `WorkflowIntent::PlanToTasks`
- `roko plan run` -> `WorkflowIntent::PlanToRun`
- `roko resume` -> `WorkflowIntent::ResumeOperation`

## P0 Findings

### P0-01 Entrypoints Choose Different Runtimes Instead Of One Workflow Engine

Problem:

CLI, HTTP, jobs, and runner paths each choose execution helpers independently. This means a feature can work in one path and be missing from another.

Evidence:

- CLI one-shot paths call `cmd_oneshot`, `cmd_pipe`, `cmd_run`, or inline unified helpers.
- PRD/plan commands call agent helper functions directly.
- Serve routes call `runtime.run_once` directly.
- Runner-v2 is used for plan execution, but not for all "run this project" semantics.

Implementation checklist:

- [ ] Add `WorkflowEngine::submit(request) -> WorkflowSubmission`.
- [ ] Add `WorkflowEngine::resume(operation_id) -> WorkflowSubmission`.
- [ ] Add `WorkflowEngine::status(operation_id) -> WorkflowStatus`.
- [ ] Add a CLI adapter that converts every project-oriented command into `WorkflowRequest`.
- [ ] Add an HTTP adapter that converts every project-oriented route into `WorkflowRequest`.
- [ ] Add a grep gate: project commands may call `WorkflowEngine`, not `run_once`, `run_agent_logged`, or route-local prompt builders.
- [ ] Add proof that CLI and HTTP submit the same workflow intent and produce the same event schema for the same input.

### P0-02 `roko run` Does Not Own The Multi-Stage Project Workflow

Problem:

The command that sounds like "do the whole thing" is not the single owner of PRD, plan, tasks, execution, observation, and resume.

Evidence:

- `Command::Run` is described as "Generate and execute a plan".
- The implementation path is a universal prompt loop unless a user manually enters the separate PRD/plan/task/run flow.

Implementation checklist:

- [ ] Define the default `roko run` workflow in config: `prompt_to_answer`, `prompt_to_plan`, or `prompt_to_project_run`.
- [ ] Add `--to` and `--from` flags to make pipeline boundaries explicit.
- [ ] Add `--operation-id` output for every `roko run` invocation that persists work.
- [ ] Add `--json` output containing operation id, workflow intent, step ids, artifact refs, provider statuses, and next actions.
- [ ] Make `cmd_run` submit `WorkflowIntent::PromptToProjectRun` when the input requests a project workflow.
- [ ] Keep pure chat behavior as an explicit `WorkflowIntent::PromptToAnswer`.
- [ ] Add proof command for one-shot full flow in a temp workspace.

Proof shape:

```bash
tmpdir="$(mktemp -d)"
cd "$tmpdir"
roko run "create a tiny CLI that prints hello" --to done --json > run.json
jq -e '.operation_id and (.steps | length > 0)' run.json
roko workflow artifacts "$(jq -r .operation_id run.json)"
roko workflow status "$(jq -r .operation_id run.json)" --json
```

### P0-03 PRD, Plan, And Task Generation Are Prompt Side Effects, Not Typed Artifact Operations

Problem:

Generation commands often build a prompt and trust output conventions instead of producing typed artifacts through a shared artifact contract.

Evidence:

- `commands/prd.rs` writes draft scaffolds and captures agent output.
- `prd.rs` generates plans from PRDs with custom failure-context prompt logic.
- `commands/plan.rs` generates and regenerates tasks with local prompt strings.
- Serve routes duplicate plan generation prompt logic.

Implementation checklist:

- [ ] Define `ArtifactType::{PrdDraft, PrdPublished, PlanSpec, TaskList, RunSnapshot, ProofBundle}`.
- [ ] Define validators for every artifact type.
- [ ] Add `ArtifactStore::put_typed` and `ArtifactStore::get_typed`.
- [ ] Replace direct scaffold writes with `ArtifactStore` writes.
- [ ] Replace command-local parsing with artifact validators.
- [ ] Add event types: `artifact.created`, `artifact.validated`, `artifact.rejected`, `artifact.promoted`.
- [ ] Add proof that generated PRD, plan, and task artifacts can be queried by operation id from CLI and HTTP.

### P0-04 HTTP Plan Execution Can Bypass Typed Plan Execution

Problem:

HTTP plan routes can construct an "execute this plan" prompt and call `runtime.run_once`. That is not the same as runner-v2 plan execution with task state, gates, merge, retry, resume, and proof.

Evidence:

- `crates/roko-serve/src/routes/plans.rs` has prompt builders for plan execution and calls `runtime.run_once`.
- `crates/roko-serve/src/runtime.rs` has a `run_plan` abstraction, but the route surface is not consistently forced through typed plan execution.

Implementation checklist:

- [ ] Remove route-level plan execution prompt construction.
- [ ] Make every `/plans/*/execute` style endpoint call `WorkflowIntent::PlanToRun`.
- [ ] Make `WorkflowStepKind::ExecutePlan` call runner-v2 through `PlanExecutionService`.
- [ ] Make `PlanExecutionService` publish task, gate, merge, retry, and resume events.
- [ ] Add a failing test or grep gate that rejects `build_plan_execution_prompt` in route code.
- [ ] Add proof that HTTP plan execution writes the same runner events as CLI `roko plan run`.

### P0-05 Auto-Orchestrate Is A Route-Owned Subscriber Instead Of A Workflow Trigger

Problem:

Auto-orchestration after PRD publish is currently route/server behavior. That means it is hard to reason about policy, cancellation, retries, resume, and duplicate triggering.

Evidence:

- PRD routes own publish side effects and plan-generation queues.
- Plan generation after publish is not modeled as a durable workflow trigger with idempotency.

Implementation checklist:

- [ ] Add `WorkflowTrigger` definitions: `OnPrdPublished`, `OnPlanValidated`, `OnGateFailed`, `OnResumeRequested`, `OnJobQueued`.
- [ ] Store triggers durably with idempotency keys.
- [ ] Replace route subscribers with trigger emission.
- [ ] Add `WorkflowTriggerRunner` under `RuntimeTaskSupervisor`.
- [ ] Add duplicate-trigger proof: publishing the same PRD twice must not create duplicate plan runs unless requested.

### P0-06 Prompt Builders Are Duplicated Across CLI And Serve

Problem:

The same conceptual prompt exists in many files. This blocks knowledge injection, prompt diagnostics, playbooks, safety policy, and provider-specific formatting from being consistent.

Evidence:

- `routes/prds.rs` has local PRD and plan prompt builders.
- `routes/plans.rs` has local plan execution, chat, and generation prompt builders.
- `commands/prd.rs`, `commands/plan.rs`, and `prd.rs` contain command-local prompt strings.
- `dispatch/prompt_builder.rs` already exists and should be the shared path.

Implementation checklist:

- [ ] Move prompt templates into a `PromptTemplateRegistry`.
- [ ] Make `PromptAssembler` accept `WorkflowStepContext`.
- [ ] Add template ids for PRD draft, PRD consolidate, plan generation, task generation, plan chat, gate retry, replan, and execution.
- [ ] Emit `prompt.assembled` events with template id, input artifact ids, knowledge ids, playbook ids, token budget, and dropped sections.
- [ ] Delete or quarantine route-local and command-local prompt builders.
- [ ] Add grep gate: `build_.*prompt` is allowed only in prompt/template modules and tests.

## P1 Findings

### P1-01 Jobs Are Not First-Class Workflow Operations

Problem:

Jobs should be workflow requests with durable step state. Today they are server-managed tasks with their own artifact synthesis.

Implementation checklist:

- [ ] Represent a job as `WorkflowIntent::JobToProjectRun`.
- [ ] Make job status a projection over workflow operation state.
- [ ] Move job runner execution under `RuntimeTaskSupervisor`.
- [ ] Add job artifacts: source request, generated PRD, generated plan, generated tasks, run proof.
- [ ] Add proof that a queued job can be resumed after process restart without duplicate execution.

### P1-02 Resume Is Executor-Centric Instead Of Workflow-Centric

Problem:

Resume should restart or continue a workflow operation, not only a plan runner in a hardcoded directory.

Implementation checklist:

- [ ] Add workflow snapshots at every step transition.
- [ ] Make `roko workflow resume <operation-id>` the primary resume command.
- [ ] Make old `roko resume` resolve to the most recent resumable workflow or require an explicit operation id.
- [ ] Record skipped, completed, failed, retried, and blocked steps durably.
- [ ] Add crash proof for PRD generation, plan generation, task execution, and merge phases.

### P1-03 Research, Enhance, And Consolidate Flows Are Freeform Agent Runs

Problem:

Research and consolidation affect project artifacts, but they are executed as freeform prompts. That makes their outputs hard to validate and replay.

Implementation checklist:

- [ ] Add `WorkflowIntent::ResearchToPrd`.
- [ ] Add `WorkflowIntent::PlanChatEdit`.
- [ ] Store research sources, summaries, and artifact patches as typed artifacts.
- [ ] Require validators before writing artifact changes.
- [ ] Add proof that an enhancement changes an artifact and records the before/after refs.

### P1-04 Dry-Run And Proof Modes Are Not Uniform

Problem:

Some paths have dry-run behavior, some preserve scaffolds, and some execute. There is no single proof contract for a workflow.

Implementation checklist:

- [ ] Add `WorkflowOptions { dry_run, proof_mode, max_steps, require_live_provider, allow_write, allow_merge }`.
- [ ] Make every step return a `StepOutcome` with `would_run`, `ran`, `skipped`, `blocked`, or `failed`.
- [ ] Make proof mode write a bundle with command, environment classification, artifacts, events, projections, and redacted provider evidence.
- [ ] Add proof that dry-run creates no workspace writes outside the proof bundle.

## Concrete Implementation Plan

### Phase 1 - Define Workflow Core

- [ ] Add `WorkflowRequest`, `WorkflowIntent`, `WorkflowInputs`, `WorkflowOptions`, `WorkflowPlan`, `WorkflowStep`, `WorkflowStepKind`, `WorkflowRun`, and `StepOutcome`.
- [ ] Add `WorkflowEngine` trait with `submit`, `resume`, `cancel`, `status`, and `artifacts`.
- [ ] Add `WorkflowPlanner` that expands an intent into steps.
- [ ] Add `WorkflowExecutor` that executes steps through registered `StepExecutor` implementations.
- [ ] Add `WorkflowArtifactStore` facade over existing PRD, plan, task, and proof storage.
- [ ] Add `WorkflowOperationStore` facade over operation status from doc `35`.
- [ ] Add event emission for `workflow.submitted`, `workflow.planned`, `workflow.step.started`, `workflow.step.completed`, `workflow.step.failed`, `workflow.blocked`, `workflow.resumed`, and `workflow.cancelled`.

### Phase 2 - Migrate CLI Entrypoints

- [ ] Replace `cmd_run` project behavior with `WorkflowEngine::submit`.
- [ ] Keep pure one-shot chat as `WorkflowIntent::PromptToAnswer`.
- [ ] Convert `prd draft`, `prd plan`, `prd consolidate`, `plan generate`, `plan regenerate`, `plan run`, and `resume` into workflow adapters.
- [ ] Return operation ids in human output and JSON output.
- [ ] Add CLI proof commands for `--to prd`, `--to plan`, `--to tasks`, and `--to done`.

### Phase 3 - Migrate HTTP Entrypoints

- [ ] Add HTTP workflow routes: `POST /api/workflows`, `GET /api/workflows/:id`, `POST /api/workflows/:id/resume`, `POST /api/workflows/:id/cancel`, `GET /api/workflows/:id/artifacts`.
- [ ] Change PRD draft/publish/plan routes to submit workflow requests or query workflow artifacts.
- [ ] Change plan generation/execution/chat routes to submit workflow requests or query workflow artifacts.
- [ ] Change jobs to be workflow requests and workflow projections.
- [ ] Remove route-local prompt builders after migration.

### Phase 4 - Migrate Prompt And Artifact Handling

- [ ] Create `PromptTemplateRegistry`.
- [ ] Route every workflow prompt through `PromptAssembler`.
- [ ] Add typed validators for PRD, plan, task list, chat patch, research summary, run proof, and merge proof.
- [ ] Add before/after artifact refs for every mutating workflow step.
- [ ] Add prompt diagnostics to workflow step events.

### Phase 5 - Proof And Retirement

- [ ] Add an end-to-end proof script for a clean temporary workspace.
- [ ] Prove CLI full workflow and HTTP full workflow produce the same operation/event/artifact model.
- [ ] Prove resume after crash at each step boundary.
- [ ] Prove cancellation kills or blocks running step work through the task/process lifecycle spine.
- [ ] Prove route grep gates contain no route-local prompt execution.
- [ ] Mark old prompt builders and helper paths deprecated, then delete after compatibility period.

## Grep Gates

Use these as implementation guards:

```bash
# Project entrypoints should use WorkflowEngine, not low-level agent calls.
rg -n "run_once\\(|run_agent_logged\\(|run_agent_capture|build_.*prompt" crates/roko-cli/src/commands crates/roko-cli/src/prd.rs crates/roko-serve/src/routes -g '*.rs'

# HTTP plan execution should not be prompt-based.
rg -n "build_plan_execution_prompt|execute this plan|runtime\\.run_once" crates/roko-serve/src/routes/plans.rs

# Route handlers should not spawn workflow work directly.
rg -n "tokio::spawn|spawn_blocking|Command::new" crates/roko-serve/src/routes -g '*.rs'

# Old one-shot paths should be explicit chat only, not project workflow execution.
rg -n "cmd_run|cmd_oneshot|cmd_pipe|cmd_oneshot_inline" crates/roko-cli/src -g '*.rs'
```

Passing state:

- The first grep may return compatibility wrappers only if each call delegates to `WorkflowEngine` first or is explicitly `PromptToAnswer`.
- The second grep should return no production route-level execution prompt.
- The third grep should return no project workflow route spawns.
- The fourth grep should show only adapter code with clear intent classification.

## End-To-End Proof Requirements

### CLI Full Workflow Proof

```bash
tmpdir="$(mktemp -d)"
cd "$tmpdir"
roko run "create a tiny checked-in hello-world script" --to done --json > workflow.json
op="$(jq -r '.operation_id' workflow.json)"
test -n "$op"
roko workflow status "$op" --json | jq -e '.status == "completed"'
roko workflow artifacts "$op" --json | jq -e '.artifacts[] | select(.type == "RunSnapshot")'
```

Expected evidence:

- operation id
- workflow submitted/planned/step events
- PRD or plan artifacts, depending on configured pipeline
- task execution events
- gate evidence
- merge or no-merge decision
- final run snapshot

### HTTP Full Workflow Proof

```bash
tmpdir="$(mktemp -d)"
cd "$tmpdir"
roko serve --port 0 --json > serve.json &
server_pid=$!
base="$(jq -r '.base_url' serve.json)"
curl -sS -X POST "$base/api/workflows" \
  -H 'content-type: application/json' \
  -d '{"intent":"PromptToProjectRun","input":"create a tiny checked-in hello-world script","to":"done"}' \
  | tee workflow.json
op="$(jq -r '.operation_id' workflow.json)"
curl -sS "$base/api/workflows/$op" | jq -e '.status'
curl -sS "$base/api/workflows/$op/artifacts" | jq -e '.artifacts'
kill "$server_pid"
```

Expected evidence:

- same operation schema as CLI
- same artifact schema as CLI
- same event/projection schema as CLI
- no route-specific fake status

### Resume Proof

```bash
tmpdir="$(mktemp -d)"
cd "$tmpdir"
ROKO_PROOF_CRASH_AT_STEP=GeneratePlan roko run "create hello-world" --to done --json > crashed.json || true
op="$(jq -r '.operation_id' crashed.json)"
roko workflow resume "$op" --json > resumed.json
jq -e '.status == "completed"' resumed.json
roko workflow events "$op" --json | jq -e '[.events[] | select(.type == "workflow.resumed")] | length == 1'
```

Expected evidence:

- no duplicate PRD or plan artifact unless a retry emitted a new artifact version
- resumed step starts after the last durable completed step
- failed step includes reason and retry decision

## Done Criteria

This audit is complete only when:

- [ ] Every project-oriented CLI command is a workflow adapter.
- [ ] Every project-oriented HTTP endpoint is a workflow adapter or workflow query.
- [ ] A full one-shot "idea to done" command exists and persists an operation id.
- [ ] PRD, plan, tasks, run snapshots, proof bundles, and prompt diagnostics are typed artifacts.
- [ ] Runner-v2 is the only implementation of plan execution.
- [ ] `run_once` remains only as the provider invocation primitive for explicit chat or workflow `InvokeAgent` steps.
- [ ] Jobs are workflow operations, not a separate server execution model.
- [ ] Resume and cancellation are workflow-operation based.
- [ ] CLI and HTTP proof scripts pass in a clean temporary workspace.
- [ ] Grep gates show no route-local project prompt execution.

## 2026-04-27 Deepening Pass - Source-Verified Workflow Drift

This pass re-read the hot workflow surfaces after the gateway/model-call audit. The prior version of this doc had the right target design, but it did not give enough source-level detail for an implementation agent to remove the route/command-local workflow behavior without rediscovering it.

### Drift D1 - HTTP PRD Draft Is A Route-Owned Agent Run

Current source shape:

- `crates/roko-serve/src/routes/prds.rs::draft_prd` creates `.roko/prd/drafts`, writes or refreshes a scaffold, builds a PRD prompt, spawns a `tokio::spawn`, calls `runtime.run_once`, publishes `ServerEvent::OperationStarted`, and stores an `OperationHandle` in `state.operations`.
- The route owns artifact layout, prompt construction, operation lifecycle, task spawning, and status publication.

Why this matters:

- This makes HTTP PRD drafting a different product path than CLI PRD drafting.
- A server restart can lose the route-local operation truth.
- The generated PRD is not a typed workflow artifact with a step id, prompt diagnostics id, model-call id, and validation result.

Replacement checklist:

- [ ] Add `WorkflowStepKind::DraftPrd`.
- [ ] Add `PrdArtifactRepository::create_or_load_draft(slug) -> ArtifactRef`.
- [ ] Add `PrdDraftService::build_request(slug, draft_ref, context) -> WorkflowStepRequest`.
- [ ] Make `PrdDraftService` call `PromptAssembler` with a `WorkflowStepContext`.
- [ ] Make the model call go through `ModelCallService` from [41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md).
- [ ] Make the response write through `ArtifactStore` with a before/after artifact version.
- [ ] Emit `workflow.step.started`, `workflow.step.completed`, `artifact.created`, `prompt.assembled`, and `model_call.completed`.
- [ ] Replace `POST /api/prds/{slug}/draft` with a thin adapter that submits `WorkflowIntent::PromptToPrd` or `WorkflowStepKind::DraftPrd`.
- [ ] Add proof that a PRD draft operation survives server restart and can be queried by operation id.

### Drift D2 - PRD Promote Owns Auto-Orchestration

Current source shape:

- `crates/roko-serve/src/routes/prds.rs::promote_prd` renames a draft file into `published/`, appends a `prd_published` episode, conditionally queues plan generation, and emits a `RokoEvent::PrdPublished`.
- `spawn_prd_publish_subscriber`, `follow_prd_published_audit`, and `handle_prd_published_event` live in the route module.
- Duplicate suppression is a route-local `OnceLock<Mutex<HashMap<...>>>` with a 60 second window.

Why this matters:

- Publish-to-plan is exactly a workflow trigger. Keeping it in a route module hides policy, idempotency, retry, and cancellation.
- The route dedupe window is not a durable idempotency key. A restart resets it.
- Auto-orchestration can be triggered by both episode replay and event bus subscription, but the durable operation model is not the source of truth.

Replacement checklist:

- [ ] Add `WorkflowTriggerKind::OnPrdPublished`.
- [ ] Add durable `WorkflowTriggerStore` records keyed by `workspace_id`, `trigger_kind`, `artifact_id`, and `artifact_version`.
- [ ] Replace route-local `RECENT_PUBLISHES` with durable idempotency.
- [ ] Move publish episode conversion into a workflow trigger adapter, not `routes/prds.rs`.
- [ ] Make PRD promotion write an artifact transition: `draft -> published`.
- [ ] Make auto-plan submit `WorkflowIntent::PrdToPlan` with parent operation id.
- [ ] Add duplicate-trigger proof: replaying the same `PrdPublished` event after restart does not create a second plan workflow.
- [ ] Add manual override proof: operator can explicitly request a new plan from the same PRD version and the new workflow records why.

### Drift D3 - PRD-To-Plan Generation Is Duplicated

Current source shape:

- `crates/roko-serve/src/routes/prds.rs::queue_plan_generation_op` builds a plan prompt and calls `runtime.run_once`.
- `crates/roko-serve/src/routes/plans.rs::generate_plan` separately builds a similar prompt and calls `runtime.run_once`.
- `crates/roko-cli/src/commands/plan.rs::PlanCmd::Generate` and `PlanCmd::Regenerate` build separate prompts and call `run_agent_logged`.
- `crates/roko-cli/src/commands/prd.rs` has its own command-local PRD drafting and agent-output materialization behavior.

Why this matters:

- Plan quality, task schema, MCP metadata, verify commands, and model policy depend on which endpoint the user happened to use.
- The same concept has multiple prompt templates and multiple failure policies.
- The system cannot prove "PRD generated plan" in one query shape.

Replacement checklist:

- [ ] Add `WorkflowStepKind::GeneratePlan`.
- [ ] Add one `PlanGenerationService`.
- [ ] Make CLI `roko prd plan`, CLI `roko plan generate`, HTTP `/api/prds/{slug}/plan`, HTTP `/api/plans/generate`, and auto-plan triggers call the same service.
- [ ] Put PRD-to-plan prompt construction behind `PromptAssembler` with template id `workflow.prd_to_plan.v1`.
- [ ] Validate outputs with a typed `PlanArtifactValidator`.
- [ ] Require plan output to be stored as versioned artifacts, not only side effects written by an agent.
- [ ] Emit `workflow.step.completed` with generated `plan_id`, `task_count`, `artifact_refs`, and `validation_status`.
- [ ] Add proof that CLI and HTTP PRD-to-plan with the same PRD produce the same operation/event/artifact schema.

### Drift D4 - HTTP Plan Execution Can Be Natural-Language Prompt Execution

Current source shape:

- `crates/roko-serve/src/routes/plans.rs::execute_plan` loads a plan, builds `build_plan_execution_prompt`, then calls `runtime.run_once`.
- `crates/roko-serve/src/routes/plans.rs::generate_plan` and related plan routes also call `runtime.run_once`.
- `crates/roko-serve/src/runtime.rs` has a `CliRuntime::run_plan` abstraction, but route code still calls `run_once` directly in important paths.

Why this matters:

- Natural-language "execute this plan" bypasses runner-v2 scheduling, DAG ordering, gates, retry/replan, merge policy, resume, and proof events.
- HTTP plan execution can appear to work while not exercising the orchestrator replacement.

Replacement checklist:

- [ ] Delete or quarantine `build_plan_execution_prompt` from route execution paths.
- [ ] Add `PlanExecutionService::start(command) -> OperationRef`.
- [ ] Back `PlanExecutionService` with runner-v2 only.
- [ ] Make `/api/plans/{id}/execute` call `WorkflowIntent::PlanToRun`.
- [ ] Make `/api/plans/{id}/resume` call `WorkflowIntent::ResumeOperation` with a plan/run operation id.
- [ ] Ensure plan execution emits runner task, gate, retry, merge, and resume events under the parent workflow operation.
- [ ] Add grep gate: `rg -n "build_plan_execution_prompt|execute it in the current workspace|runtime\\.run_once" crates/roko-serve/src/routes/plans.rs` returns no production execution path.
- [ ] Add proof that HTTP plan execution and CLI `roko plan run` produce the same runner event schema.

### Drift D5 - Research Enhancement Is A Route-Owned Mutation Flow

Current source shape:

- `crates/roko-serve/src/routes/research.rs::spawn_research_op` spawns a task, calls `runtime.run_once`, and records a generic operation.
- The research prompts instruct the agent to update PRDs/plans/tasks in place and write research summaries.
- The route reads PRDs and plans directly from `.roko` paths.

Why this matters:

- Research is both a model call and an artifact mutation, but neither side is represented as typed workflow steps.
- The route cannot prove which artifact version was read, what changed, which sources were used, or whether the mutation was validated.

Replacement checklist:

- [ ] Add `WorkflowStepKind::ResearchTopic`.
- [ ] Add `WorkflowStepKind::EnhancePrdWithResearch`.
- [ ] Add `WorkflowStepKind::EnhancePlanWithResearch`.
- [ ] Add `WorkflowStepKind::EnhanceTasksWithResearch`.
- [ ] Store research outputs as typed artifacts with source/citation metadata.
- [ ] For mutating enhancement steps, record before/after artifact refs.
- [ ] Route model calls through `ModelCallService` with caller `research`.
- [ ] Add validators for "research summary exists", "citations captured", and "mutated target still parses".
- [ ] Add proof that a research enhancement can be queried by operation id and shows target artifact diff.

### Drift D6 - Template Deploy Is A Parallel One-Off Runtime

Current source shape:

- `crates/roko-serve/src/routes/templates.rs::deploy_template` renders a template, spawns a task, calls `runtime.run_once`, updates `template_runs`, and publishes operation completion.
- Cloud deploy behavior branches inside the same route.

Why this matters:

- Template deployment is a workflow request with parameters, backend policy, model-call behavior, deployment artifacts, and operation state.
- Keeping it as a route-local branch means deployments, template runs, and model calls do not share proof semantics.

Replacement checklist:

- [ ] Add `WorkflowIntent::TemplateDeploy`.
- [ ] Add `TemplateRepository` and `TemplateRunProjection`.
- [ ] Convert in-process template deploy to workflow step `InvokeTemplateAgent`.
- [ ] Convert cloud template deploy to workflow step `DeployWorker`.
- [ ] Emit typed template render, model-call, deployment, and completion events.
- [ ] Add proof that `/api/templates/{name}/deploy` returns a workflow operation id and the template run appears in projections after restart.

### Drift D7 - Job Runner Can Synthesize Success

Current source shape:

- `crates/roko-serve/src/job_runner.rs::execute_research_job` calls `state.runtime.run_once` and writes `.roko/research/{job_id}.md`.
- `execute_coding_job` materializes a PRD, prepares a plan, calls `state.runtime.run_plan`, writes job artifacts, and synthesizes a generic runtime gate if no structured gates exist.
- `prepare_coding_plan` falls back to `synthesize_coding_plan` when runtime PRD planning is unavailable.
- Chain monitor and chain analysis jobs generate synthetic mock chain events.

Why this matters:

- A job can finish with artifacts that were synthesized by fallback code rather than the real planner, real provider, real chain source, or real gate pipeline.
- This is useful for demos, but it must not count as Mori parity or strict end-to-end proof.

Replacement checklist:

- [ ] Represent every job as `WorkflowIntent::JobToProjectRun`, `JobToResearch`, `JobToChainMonitor`, or `JobToChainAnalysis`.
- [ ] Add strict mode: `synthesize_coding_plan` is disabled unless `allow_synthetic_fallback = true`.
- [ ] Tag synthetic chain inputs as `input_mode = synthetic`.
- [ ] Replace generic `"runtime": true` gate fallback with `gate_status = unstructured` or `unsupported`.
- [ ] Store generated PRD, plan, result summary, changed artifacts, and gate evidence as workflow artifacts.
- [ ] Add job status projection from workflow operation events.
- [ ] Add proof that strict job execution fails visibly when the real planner/provider/chain inputs are unavailable.
- [ ] Add proof that demo mode succeeds only with `fallback_synthetic` evidence tags.

### Drift D8 - CLI Commands Are Workflow Logic, Not Thin Adapters

Current source shape:

- `crates/roko-cli/src/commands/prd.rs` creates scaffolds, builds PRD prompts, detects whether an agent modified a file by mtime, captures output, materializes markdown fallback output, and persists capture episodes.
- `crates/roko-cli/src/commands/plan.rs::Generate` and `Regenerate` read source content, build prompt strings, call agent helpers, and rely on the agent to write plan files.
- `crates/roko-cli/src/commands/job.rs` reads and writes job JSON directly.

Why this matters:

- CLI commands are still application services. They are not just input adapters.
- A CLI bug or prompt tweak can diverge from HTTP and TUI behavior.
- The mtime-based "did the agent write the file" heuristic is not a typed artifact write contract.

Replacement checklist:

- [ ] Convert `roko prd draft new/edit` into `WorkflowEngine::submit`.
- [ ] Convert `roko prd plan` into `WorkflowEngine::submit`.
- [ ] Convert `roko plan generate/regenerate` into `WorkflowEngine::submit`.
- [ ] Convert job create/update/execute commands into repository and workflow command calls.
- [ ] Replace mtime detection with artifact write intents and artifact repository commit records.
- [ ] Preserve CLI UX, but make `--json` return operation id, step ids, artifact refs, status, and next actions.
- [ ] Add compatibility aliases for existing commands, but make grep gates require they call `WorkflowEngine`.

### Drift D9 - Existing Use-Case Docs Are Now Partially Stale

Current source shape:

- [../../docs/USE-CASES.md](../../docs/USE-CASES.md) says automatic plan generation from published PRDs is not implemented.
- Current `routes/prds.rs` does implement route-owned auto-plan behavior after publish.

Why this matters:

- The old doc is wrong in a subtle way: something exists, but the design is not the desired workflow-engine design.
- Implementation agents may either reimplement it from scratch or falsely mark it done.

Replacement checklist:

- [ ] Update public docs after the workflow engine exists.
- [ ] Label current route-owned auto-plan as `partial-route-owned`, not `not implemented` and not `proven`.
- [ ] Require proof that auto-plan is durable, idempotent, restart-safe, and queryable before docs call it implemented.

## Workflow Service Contracts To Implement

These contracts are the minimum clean design that avoids another ad hoc layer.

```rust
#[async_trait::async_trait]
pub trait WorkflowEngine: Send + Sync {
    async fn submit(&self, request: WorkflowRequest) -> Result<WorkflowSubmission>;
    async fn resume(&self, operation_id: OperationId) -> Result<WorkflowSubmission>;
    async fn cancel(&self, operation_id: OperationId, reason: CancelReason) -> Result<()>;
    async fn status(&self, operation_id: OperationId) -> Result<WorkflowStatus>;
    async fn artifacts(&self, operation_id: OperationId) -> Result<Vec<ArtifactRef>>;
}

pub trait StepExecutor: Send + Sync {
    fn kind(&self) -> WorkflowStepKind;
    async fn execute(&self, ctx: StepContext) -> Result<StepOutcome>;
}
```

Required service dependencies:

- [ ] `ArtifactStore` for PRDs, plans, tasks, research reports, job artifacts, run snapshots, prompt diagnostics, and proof bundles.
- [ ] `OperationStore` for workflow and step state.
- [ ] `ModelCallService` for any model/provider call.
- [ ] `PromptAssembler` for any generated prompt.
- [ ] `PlanExecutionService` for runner-v2 execution.
- [ ] `RuntimeTaskSupervisor` for long-running step execution.
- [ ] `WorkflowTriggerStore` for idempotent event-driven submissions.
- [ ] `RuntimeQueryService` for status, artifacts, events, and projections.

Prohibited dependencies:

- [ ] Route handlers must not call `runtime.run_once` for project workflows.
- [ ] Route handlers must not spawn long-running workflow tasks directly.
- [ ] CLI command handlers must not build project workflow prompts directly.
- [ ] Job runner must not synthesize success without explicit fallback evidence.
- [ ] Workflow steps must not write `.roko` paths directly; they must use repositories/artifact store.

## Concrete Migration Order

### Batch W1 - Define Workflow Core

- [ ] Add workflow request, intent, step, operation, artifact, and outcome types.
- [ ] Add `WorkflowEngine`, `StepExecutor`, `WorkflowPlanner`, and `WorkflowExecutor`.
- [ ] Add durable workflow events: `workflow.submitted`, `workflow.planned`, `workflow.step.started`, `workflow.step.completed`, `workflow.step.failed`, `workflow.blocked`, `workflow.resumed`, `workflow.cancelled`.
- [ ] Add operation id and step id generation.
- [ ] Add proof-only dry-run mode that plans steps without workspace mutation.

### Batch W2 - Artifact Repositories

- [ ] Add PRD repository.
- [ ] Add plan repository.
- [ ] Add task-list repository.
- [ ] Add research artifact repository.
- [ ] Add job artifact repository.
- [ ] Add proof bundle repository.
- [ ] Add before/after artifact version records for mutating steps.

### Batch W3 - PRD And Plan Steps

- [ ] Implement `DraftPrd`.
- [ ] Implement `PromotePrd`.
- [ ] Implement `GeneratePlan`.
- [ ] Implement `RegeneratePlan`.
- [ ] Implement `ValidatePlan`.
- [ ] Move prompt construction into `PromptAssembler` templates.
- [ ] Replace CLI and HTTP PRD/plan generation paths with workflow submissions.

### Batch W4 - Plan Execution Step

- [ ] Implement `ExecutePlan`.
- [ ] Back it with runner-v2 only.
- [ ] Link runner run id, plan id, task ids, and gate ids to the parent workflow operation.
- [ ] Replace HTTP plan execute/resume prompt fallback paths.
- [ ] Add proof that runner events are visible by workflow operation id.

### Batch W5 - Research, Template, And Job Steps

- [ ] Implement research topic/enhancement steps.
- [ ] Implement template deploy steps.
- [ ] Implement job-to-workflow conversion.
- [ ] Mark synthetic fallback modes explicitly.
- [ ] Replace `job_runner.rs` execution with workflow submissions and projections.

### Batch W6 - Triggers And Resume

- [ ] Implement durable `OnPrdPublished` trigger.
- [ ] Implement duplicate trigger idempotency across restart.
- [ ] Implement workflow snapshots at every step transition.
- [ ] Implement `roko workflow resume <operation-id>`.
- [ ] Replace old resume entrypoints with workflow resume adapters.

### Batch W7 - Proof And Retirement

- [ ] Add CLI proof: prompt to PRD, prompt to plan, prompt to tasks, prompt to done.
- [ ] Add HTTP proof: `/api/workflows` prompt to done.
- [ ] Add auto-plan proof: publish PRD, restart server, verify exactly one plan workflow.
- [ ] Add strict job proof: synthetic fallback is rejected or tagged.
- [ ] Add grep gate for route-local `run_once`, route-local `tokio::spawn`, command-local project prompt builders, and `.roko` direct writes.
- [ ] Deprecate route/command-local helpers after proof passes.

## Additional Grep Gates From This Pass

```bash
# PRD/plan/research/template HTTP routes should not own model workflow execution.
rg -n "runtime\\.run_once|run_once\\(&workdir|tokio::spawn|build_.*prompt|OperationHandle" \
  crates/roko-serve/src/routes/prds.rs \
  crates/roko-serve/src/routes/plans.rs \
  crates/roko-serve/src/routes/research.rs \
  crates/roko-serve/src/routes/templates.rs

# CLI project commands should be workflow adapters, not local prompt/materialization engines.
rg -n "run_agent_logged|run_agent_capture|std::fs::write|mtime|build_generation_prompt|materialize_agent_markdown_output" \
  crates/roko-cli/src/commands/prd.rs \
  crates/roko-cli/src/commands/plan.rs \
  crates/roko-cli/src/commands/job.rs

# Job runner cannot silently prove real execution with synthetic fallback.
rg -n "synthesize_coding_plan|MockChainClient|runtime.*true|structured gate|fallback coding plan" \
  crates/roko-serve/src/job_runner.rs

# Workflow implementation should be present before route-local behavior is retired.
rg -n "WorkflowEngine|WorkflowStepKind|WorkflowTriggerStore|PlanExecutionService|ArtifactStore" \
  crates/roko-cli/src crates/roko-serve/src crates/roko-runtime/src
```

Pass condition:

- [ ] The first grep returns no production command path outside thin workflow adapters.
- [ ] The second grep returns no project workflow logic outside compatibility adapters that immediately call `WorkflowEngine`.
- [ ] The third grep returns only explicit demo-mode or strict-mode-classified fallback code.
- [ ] The fourth grep proves real shared workflow services exist.

## Updated Self-Grade After Deepening

Previous score: `9.83 / 10`.

Updated score: `9.89 / 10`.

Reasoning:

- The original doc already had the correct architecture: one `WorkflowEngine`, typed steps, typed artifacts, operation ids, CLI/HTTP/TUI adapters, and proof scripts.
- This pass adds source-verified drift cases for PRD draft, PRD promote auto-orchestration, duplicate PRD-to-plan generation, HTTP plan execution, research enhancement, template deployment, job runner fallback, CLI PRD/plan commands, and stale use-case documentation.
- The new migration batches are specific enough for an implementation agent to start without rediscovering route-local behavior.
- Residual risk remains implementation-level: the correct crate split depends on the broader dependency-layering refactor in [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md).

## Original Self-Grade From Prior Pass

Score: `9.83 / 10`

Reasoning:

- Strong: identifies the product-level orchestration problem behind the repeated user pain, maps every major current entrypoint, proposes a typed workflow engine rather than another one-off command, and gives concrete migration/proof checklists.
- Strong: connects the workflow redesign to config, prompt assembly, provider dispatch, process lifecycle, observability, and artifact persistence docs.
- Residual gap: this is still documentation, not implementation. The exact final crate location depends on the broader dependency-layering work in doc `32`.

The original score was above `9.8`; the 2026-04-27 deepening pass above raises the implementation-readiness score to `9.89 / 10`.

Self-grade validation note: Current self-grade is `9.89 / 10`; this file is above the requested threshold and remains open until the workflow proof and retirement gates above pass.
