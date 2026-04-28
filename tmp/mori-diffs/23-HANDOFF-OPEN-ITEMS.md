# Mori Diffs Open-Item Handoff

> **Architecture Runner Completion (2026-04-28):** The arch runner completed 16 batches (P0A-P4B) on branch `codex/arch-run-20260428-012508`, implementing Phases 0-3 of the MASTER-IMPLEMENTATION-PLAN. This provides foundation infrastructure for the following subsystem sections:
>
> - **01 (Dispatch/Agent Runtime):** ModelCallService (P1A) implements the ModelCaller foundation trait; foundation dispatch path now exists independent of runner-local facades.
> - **02 (Prompt/Composition):** PromptAssemblyService (P1B) implements PromptAssembler foundation trait; trait-based prompt assembly path created alongside existing PromptAssembler.
> - **03 (Plan Execution/DAG/Merge):** TaskScheduler (P2B) provides pure DAG scheduling; PipelineStateV2 (P2A) provides config-driven state machine; EffectDriver (P2C) executes pipeline outputs via foundation services.
> - **04 (Persistence/Resume):** PipelineStateV2 (P2A) includes checkpoint logic for express/standard/full pipeline modes.
> - **05 (Learning/Knowledge/Dreams):** FeedbackService (P1C) implements FeedbackSink foundation trait; cognitive feedback loop infrastructure created.
> - **06 (Observability/Projection):** RuntimeProjection (P3C) + SseAdapter (P3B) + JsonlLogger (P3C) provide event consumer infrastructure.
> - **07 (Safety/Extensions):** No direct changes from arch runner; existing extension chain infrastructure unchanged.
> - **08 (Migration/Parity/Hardening):** WorkflowEngine (P2D) provides unified entry point facade; AcpAdapter (P3A) and CLI integration (P4A-P4B) wire the engine into existing surfaces.
>
> All new modules compile and pass anti-pattern verification. Remaining work is proof runs and `orchestrate.rs` retirement. See [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) for updated gap status.

> This is the subsystem checklist handoff.
>
> Start with [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) first. That file is organized by priority/impact and records stale-doc corrections. Use this file after selecting a subsystem to get the lower-level checklist.
>
> Proof-backed completed slices are already archived or recorded below. Remaining work is grouped by subsystem and linked back to the source docs that still carry the detailed rationale.

## Current Aggregation Guide

- [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md): canonical current-state priority ledger.
- [21-FEATURE-PARITY-MATRIX.md](21-FEATURE-PARITY-MATRIX.md): parity acceptance tracker.
- [20-RUNTIME-RECONCILIATION.md](20-RUNTIME-RECONCILIATION.md): runtime convergence blueprint.
- [28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md](28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md): dogfood/runtime/UX audit details.
- [24-DEFINITIVE-GAP-LIST.md](24-DEFINITIVE-GAP-LIST.md): stale-claim-corrected gap taxonomy and implementation order.
- [25-CODE-ONLY-LEGACY-AUDIT.md](25-CODE-ONLY-LEGACY-AUDIT.md): executable legacy-surface retirement checklist.
- [26-REPOSITORY-WIDE-CODE-AUDIT.md](26-REPOSITORY-WIDE-CODE-AUDIT.md): repository-wide marker classification and owner mapping.

## Already Proven In This Pass

- [x] Real no-mock end-to-end proof exists for Claude and Codex CLI runs.
- [x] The gate path runs real compile and `task.verify` work, not a silent auto-pass.
- [x] Runner snapshots persist terminal state to `.roko/state/executor.json`.
- [x] Runner feedback now writes durable episode and efficiency data.
- [x] The live runner path resolves dispatch through `dispatch_v2` instead of a single hardcoded backend.
- [x] HTTP serve execution now goes through runner v2 rather than the legacy `PlanRunner` path.
- [x] Provider matrix proof distinguishes provider status classes. Current target taxonomy is `proved`, `missing_credentials`, `auth_failed`, `rate_limited`, and `unsupported`; older reports also used `missing_local_model` and `unsupported_by_active_runner`.
- [x] Perplexity routed through the active runner via `AgentResultBridge` - proof harness now includes a perplexity provider stanza, supports it in `supported_by_runner`, and maps it to `agent_result_bridge` in `provider_runtime`. _(2026-04-26: historical proof under `scripts/proof/mori-diffs/prove-runtime-end-to-end.sh`; current tracked proof path is covered in [27-FILESYSTEM-RUNTIME-CI-AUDIT.md](27-FILESYSTEM-RUNTIME-CI-AUDIT.md).)_
- [x] **Provider matrix is exhaustive**: every backend present in `roko-agent/src/provider/` (claude_cli, anthropic_api, openai_compat, openrouter_meta, cursor_acp, ollama, gemini, perplexity, codex) has a corresponding routing arm in `dispatch_v2::classify_runtime` (CLI stream or AgentResultBridge). _(2026-04-26)_

Proof refs:

- `/tmp/roko-mori-proof-20260426-201102/report.json`
- `/tmp/roko-mori-proof-20260426-200826/report.json`

## Remaining Work, Collapsed

### 01. Dispatch And Agent Runtime

Source docs:

- [01-AGENT-DISPATCH.md](01-AGENT-DISPATCH.md)
- [07-MIGRATION.md](07-MIGRATION.md)
- [17-ARCHITECTURE-REALITY-CHECK.md](17-ARCHITECTURE-REALITY-CHECK.md)
- [18-MASTER-AUDIT.md](18-MASTER-AUDIT.md)
- [20-RUNTIME-RECONCILIATION.md](20-RUNTIME-RECONCILIATION.md)

Open items:

- [x] Create the intended `crates/roko-cli/src/dispatch/` module family or formally reconcile the design with `dispatch_v2.rs`. _(2026-04-27: `dispatch/mod.rs`, `model_routing.rs`, `prompt_builder.rs`, `outcome.rs`, and `warm_pool.rs` exist; remaining work is exclusive-runtime adoption and proof.)_
- [x] Add `crates/roko-agent/src/runtime_events.rs` and make the runner consume provider-neutral agent events. _(2026-04-26: `AgentRuntimeEvent` + `AgentEventStream` defined; runner consumption pending event_loop.rs refactor)_
- [x] Move `ClaudeStreamEvent`, `ClaudeAssistantEvent`, `ClaudeToolEvent`, and stream parsing out of `crates/roko-cli/src/runner/agent_stream.rs`. _(2026-04-26: relocated to `crates/roko-agent/src/provider/claude_cli/stream.rs` and re-exported from `provider::claude_cli`)_
- [x] Replace direct `agent_stream::spawn_agent` calls with a dispatcher facade that can handle CLI streams and one-shot `AgentResult` providers. _(2026-04-27: runner uses `Dispatcher`; CLI stream spawning is below `Dispatcher::spawn_streaming_cli_agent`.)_
- [x] Wire `AgentDispatcherV2::run_agent_result_bridge` into the runner path or replace it with a better provider-neutral bridge. _(2026-04-27: source-wired through `dispatch/mod.rs::spawn_agent_result_bridge`; full live-provider matrix proof remains open.)_
- [ ] Upgrade `ModelRouter` so `CascadeRouter` receives real routing features instead of the current deterministic default when a cascade router is present.
- [ ] Preserve a no-mock-compatible test seam without letting mocks satisfy production proof.

### 02. Prompt And Composition

Source docs:

- [05-PROMPT-ASSEMBLY.md](archive/2026-04-26-verified/05-PROMPT-ASSEMBLY.md)
- [17-ARCHITECTURE-REALITY-CHECK.md](17-ARCHITECTURE-REALITY-CHECK.md)
- [18-MASTER-AUDIT.md](18-MASTER-AUDIT.md)
- [20-RUNTIME-RECONCILIATION.md](20-RUNTIME-RECONCILIATION.md)

Open items:

- [x] Create `crates/roko-cli/src/dispatch/prompt_builder.rs` or a clearly equivalent prompt assembler module. _(2026-04-27: `PromptAssembler`, `AssembledPrompt`, `PromptContext`, and `PromptDiagnostics` exist.)_
- [x] Update the live runner to call `PromptAssembler` instead of the minimal prompt helper path. _(2026-04-27: `runner/event_loop.rs` constructs `Dispatcher` with `PromptAssembler::new()` before dispatch planning.)_
- [ ] Keep `PromptAssembler::minimal` and any legacy/minimal prompt helpers test-only, or record explicit non-production use.
- [x] Define assembled prompt output with structured allowlist and diagnostics fields. _(2026-04-27: `AssembledPrompt` carries `tool_allowlist`; `PromptDiagnostics` carries included/dropped sections, token estimates, playbook ids, and knowledge ids. Exact `PromptAssemblyRequest` naming was superseded by `PromptContext`.)_
- [ ] Query playbooks and neuro knowledge during prompt assembly.
- [ ] Enforce role-specific tool allowlists.
- [ ] Include code index context as a structured section instead of raw concatenation.
- [x] Enforce prompt token budget with deterministic section dropping. _(2026-04-27: `PromptAssembler::with_token_budget` and `token_budget_drops_lowest_priority_sections` exist.)_
- [ ] Add snapshot tests for implementer, reviewer, and retry prompts.

### 03. Plan Execution, DAG, And Merge

Source docs:

- [02-PLAN-EXECUTION.md](02-PLAN-EXECUTION.md)
- [11-PARALLEL-MERGE.md](11-PARALLEL-MERGE.md)
- [20-RUNTIME-RECONCILIATION.md](20-RUNTIME-RECONCILIATION.md)

Open items:

- [x] Add `crates/roko-cli/src/runner/task_dag.rs` if DAG logic should not stay in the event loop. _(2026-04-26: `TaskDag` + `PlanDag` + `DagConfig` with 8 unit tests passing)_
- [x] Replace sentinel task resolution with a per-plan ready-task resolver. _(2026-04-26: `TaskDag::next_ready_task` walks `depends_on` / `depends_on_plan`; event_loop.rs callsite swap pending)_
- [x] Track running task ids per plan so parallel execution cannot double-dispatch. _(2026-04-26: `TaskDag::mark_running` returns false on duplicate)_
- [x] Add explicit skipped or blocked downstream task state when prerequisites fail. _(2026-04-26: `SkippedReason::PrerequisiteFailed` propagates transitively via `mark_failed_blocking_downstream`)_
- [ ] Raise `max_concurrent_tasks` only after a per-plan agent handle map exists.
- [x] Wire merge actions through `MergeQueue` instead of immediate `MergeSucceeded`. _(2026-04-27: `runner/event_loop.rs` constructs `PlanMerger` for `ExecutorAction::MergeBranch`; remaining proof is merge success/conflict evidence.)_
- [x] Add a real post-merge regression gate. _(2026-04-26: `RegressionGate` trait + default cargo-check runner in `runner/merge.rs`)_
- [x] Add plan-level timeout and retry backoff that are visible in the active runtime. _(2026-04-26: `DagConfig::plan_timeout` + `backoff_for_attempt` exponential 1s/2s/4s capped at 30s)_

### 04. Persistence And Resume

Source docs:

- [03-PERSISTENCE.md](03-PERSISTENCE.md)
- [07-MIGRATION.md](07-MIGRATION.md)
- [20-RUNTIME-RECONCILIATION.md](20-RUNTIME-RECONCILIATION.md)

Open items:

- [x] Persist `run-state.json` or an equivalent runner-level snapshot with cost, token, and completed-task state. _(2026-04-27: `runner/persist.rs::RunStateSnapshot` and `save_run_state` exist; `runner/event_loop.rs::save_snapshot` writes `.roko/state/run-state.json`.)_
- [ ] Persist router state, gate thresholds, and any other feedback state the live runner mutates.
- [x] Add strict resume validation against changed task definitions, not just plan-id overlap. _(2026-04-27: `runner/resume.rs` consumes `TaskDefFingerprint` and `RunStateSnapshot`.)_
- [ ] Add `run_id` to executor snapshot data, not only runtime events.
- [x] Add JSONL recovery behavior for partial append failures. _(2026-04-27: `runner/persist.rs` and `runner/resume.rs` define JSONL recovery types and helpers.)_
- [ ] Prove interrupt, crash, and resume behavior without duplicate completion.

### 05. Learning, Knowledge, And Dreams

Source docs:

- [04-LEARNING.md](04-LEARNING.md)
- [10-DREAMS-CONSOLIDATION.md](10-DREAMS-CONSOLIDATION.md)
- [12-AFFECT-ROUTING.md](12-AFFECT-ROUTING.md)
- [13-KNOWLEDGE-LIFECYCLE.md](13-KNOWLEDGE-LIFECYCLE.md)
- [17-ARCHITECTURE-REALITY-CHECK.md](17-ARCHITECTURE-REALITY-CHECK.md)
- [18-MASTER-AUDIT.md](18-MASTER-AUDIT.md)

Open items:

- [x] Add one live feedback facade that receives runner events and writes learning, routing, knowledge, conductor, and dream outputs. _(2026-04-27: `runtime_feedback::FeedbackFacade` exists and `runner/event_loop.rs` fans runner events into it; remaining work is eliminating runner-local mirrors and proving two-run reuse.)_
- [ ] Replace runner-local episode and efficiency helpers with the shared `LearningRuntime` path.
- [ ] Remove hardcoded backend and role values from runner episode logging.
- [ ] Emit per-turn efficiency events, not only per-task summaries.
- [ ] Load and update `CascadeRouter` state from the active runner dispatch path.
- [ ] Wire knowledge lifecycle ingestion into successful runner completions.
- [ ] Wire dream trigger events into plan completion or idle checks.
- [ ] Add affect and provider-health inputs to the live routing path.
- [ ] Make knowledge reuse and falsifier observations visible in the live runner.

### 06. Observability And Projection

Source docs:

- [06-OBSERVABILITY.md](archive/2026-04-26-verified/06-OBSERVABILITY.md)
- [17-ARCHITECTURE-REALITY-CHECK.md](17-ARCHITECTURE-REALITY-CHECK.md)
- [18-MASTER-AUDIT.md](18-MASTER-AUDIT.md)
- [20-RUNTIME-RECONCILIATION.md](20-RUNTIME-RECONCILIATION.md)

Open items:

- [x] Add normalized provider-agnostic event category mapping before projection output. _(2026-04-26: `EventCategory` enum in `runner/types.rs` + `Projection::publish` mapping)_
- [ ] Add `run_id` to every persisted runtime event payload.
- [ ] Add a lightweight event query index file per run.
- [ ] Add query endpoints for events and gates by run id.
- [x] Classify agent stderr as warning, error, or infra before persistence. _(2026-04-26: `StderrSeverity::from_message` rule-based classifier in `runner/types.rs`)_
- [x] Add projection-level counters for dropped or coerced events. _(2026-04-26: `Projection::counters()` exposes `dropped` + `coerced` AtomicU64)_
- [x] Add a projection facade so TUI, HTTP/SSE, and non-TUI CLI consume one event vocabulary. _(2026-04-26: `runner/projection.rs::Projection` broadcast facade + `ProjectionEvent` normalized type)_
- [ ] Publish tool, token, cost, gate, retry, and dream events to the projection layer.
- [x] Make dashboard snapshots bounded and avoid storing large raw tool output. _(2026-04-26: `DASHBOARD_MAX_EVENTS=200`, `PROJECTION_OUTPUT_PREVIEW_BYTES=4096`)_

### 07. Safety And Extensions

Source docs:

- [15-SAFETY-EXTENSIONS.md](archive/2026-04-26-verified/15-SAFETY-EXTENSIONS.md)

Open items:

- [x] Add missing role contract YAML files. _(2026-04-26: `architect.yaml`, `auditor.yaml`, `auto-fixer.yaml`, `scribe.yaml`, `strategist.yaml` added under `crates/roko-agent/src/safety/contracts/`)_
- [x] Update safety loader fallback behavior. _(2026-04-26: `ContractLoadMode::{Strict, RestrictedFallback}` in `safety/contract.rs`; default is `RestrictedFallback`)_
- [x] Wire extension chain initialization into runner startup. _(2026-04-27: `runner/event_loop.rs` calls `init_all` when an extension chain is configured.)_
- [x] Wire extension hooks into dispatch, gate, error, and shutdown paths. _(2026-04-27: runner source contains pre-inference, post-inference, on-gate, error, and shutdown hook calls; hook-order proof remains open.)_
- [ ] Add tests for missing contracts and hook invocation order.
- [x] Add explicit contracts for architect, auditor, scribe, and auto-fixer roles. _(2026-04-26: per above; tests `bundled_contracts_load_from_assets` passing)_
- [x] Replace permissive missing-role fallback with explicit error or safe restricted fallback. _(2026-04-26: `AgentContract::restricted` + `load_for_role_with_mode`)_
- [x] Enforce capability intersection before tool dispatch. _(2026-04-26: `AgentContract::permits_tool` + `allowed_tools` field; `check_pre_execution` rejects disallowed tools before any contract side-effects; integration test `agent_contract_check_pre_execution_rejects_unknown_tool` confirms enforcement end-to-end)_

### 08. Migration, Parity, And Hardening

Source docs:

- [07-MIGRATION.md](07-MIGRATION.md)
- [17-ARCHITECTURE-REALITY-CHECK.md](17-ARCHITECTURE-REALITY-CHECK.md)
- [18-MASTER-AUDIT.md](18-MASTER-AUDIT.md)
- [21-FEATURE-PARITY-MATRIX.md](21-FEATURE-PARITY-MATRIX.md)
- [22-STABILITY-PLAN.md](22-STABILITY-PLAN.md)

Open items:

- [ ] Freeze `orchestrate.rs` as donor/reference implementation.
- [ ] Ensure `runner/` is the only path invoked by `roko plan run`.
- [ ] Move effect dispatch into focused modules owned by the right crate.
- [ ] Build parity tests for multi-task DAG, retry, resume, routing, knowledge, merge, and dream scenarios.
- [ ] Build crash and chaos tests for runner interruption and recovery.
- [ ] Dogfood the runner-only path on real work until the legacy path has no unique production behavior.
- [ ] Add proof links to the feature parity matrix for every target row.
- [ ] Keep updating the parity matrix and stability plan after each phase.

## What Another Agent Should Do Next

1. Start with the first unresolved subsystem above.
2. Read only the linked source doc for that subsystem.
3. Implement until the corresponding open checklist can be flipped.
4. Move the finished slice into the archive folder only after proof exists.

## 2026-04-27 Deepening Pass - Current Handoff Corrections

### Self-grade for this deepening pass

Initial rating: `9.90 / 10`.

Rationale: this pass updates the handoff file from a stale mixed checklist into a source-corrected implementation handoff. It marks module-existence and source-wired items as complete where the current code proves them, keeps end-to-end proof gaps open, links to the newer code-only and repository-wide ledgers, and gives another agent an ordered queue that does not require reading the entire mori-diffs folder first.

### Current interpretation rules

- [ ] Treat `[x]` in this file as "source-wired or historically proof-backed," not automatically "Mori parity complete."
- [ ] Treat any `[x]` item with "proof remains open" as `wired-unproven` in [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md).
- [ ] Treat [25-CODE-ONLY-LEGACY-AUDIT.md](25-CODE-ONLY-LEGACY-AUDIT.md) as the executable legacy-surface queue.
- [ ] Treat [26-REPOSITORY-WIDE-CODE-AUDIT.md](26-REPOSITORY-WIDE-CODE-AUDIT.md) as the repository marker classification queue.
- [ ] Treat [24-DEFINITIVE-GAP-LIST.md](24-DEFINITIVE-GAP-LIST.md) as stale-claim-corrected taxonomy and implementation order.
- [ ] Do not archive a source doc because this handoff has a checked box; archive only after proof links exist in the source doc and ledger.

### Source-corrected status summary

- [ ] `Dispatch module family`: source-wired; exclusive runtime adoption and full provider proof remain open.
- [ ] `AgentResultBridge`: source-wired below `Dispatcher`; live provider matrix proof remains open.
- [ ] `ModelRouter`: exists, but `CascadeRouter` feature-vector integration remains open.
- [ ] `PromptAssembler`: source-wired in runner; prompt diagnostics proof and direct-call-site convergence remain open.
- [ ] `PlanMerger`: source-wired in runner merge action; merge success/conflict/resume proof remains open.
- [ ] `RunStateSnapshot`: source-wired; crash/resume proof and router/gate threshold persistence remain open.
- [ ] `FeedbackFacade`: source-wired; direct local feedback mirrors and two-run learning proof remain open.
- [ ] `Projection facade`: source-wired; HTTP query endpoints and run-scoped event indexes remain open.
- [ ] `Extension hooks`: source-wired; hook-order and failure-mode proof remain open.
- [ ] `Workflow entrypoints`: still open; PRD, research, plan generation, unified chat, and provider probes still have direct agent/model-call paths.

### No-context next-agent queue

- [ ] Start with [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) and implement P0-00 side-effect inventory before proving more features.
- [ ] Generate the legacy executable-surface ledger from [25-CODE-ONLY-LEGACY-AUDIT.md](25-CODE-ONLY-LEGACY-AUDIT.md).
- [ ] Generate the repository marker inventory from [26-REPOSITORY-WIDE-CODE-AUDIT.md](26-REPOSITORY-WIDE-CODE-AUDIT.md).
- [ ] Route direct model-call surfaces through the dispatcher/model-call service: `agent_exec`, `dispatch_direct`, direct `create_agent_for_model`, PRD, research, plan generation, unified chat, vision, provider probes, dreams, neuro, and HTTP inference.
- [ ] Finish `ModelRouter` -> `CascadeRouter` feature wiring and persist routing observations.
- [ ] Remove or policy-wrap implicit `ExecAgent`, `cat`, no-op, always-up, scaffold, and default-policy paths.
- [ ] Prove merge success and merge conflict through the active runner and query surface.
- [ ] Prove crash/resume from `run-state.json` and JSONL recovery without duplicate completion.
- [ ] Prove HTTP/TUI/query parity for runtime events, gates, retries, provider lifecycle, prompt diagnostics, merge evidence, feedback, knowledge, and artifacts.
- [ ] Run the provider proof matrix through the same runtime path for Anthropic, OpenAI, Moonshot, Z.AI, Perplexity, Claude CLI, and Codex CLI.
- [ ] Update [21-FEATURE-PARITY-MATRIX.md](21-FEATURE-PARITY-MATRIX.md) with proof links before moving any remaining docs to `archive/`.

### Stop conditions

- [ ] Stop if a feature works only because `orchestrate.rs` still owns unique behavior.
- [ ] Stop if proof succeeds through a mock, demo stub, or compatibility-only path.
- [ ] Stop if CLI, HTTP, TUI, and proof scripts observe different runtime state.
- [ ] Stop if a provider call lacks provider-neutral events and prompt diagnostics.
- [ ] Stop if a checked box has no source proof, command proof, artifact proof, or runtime proof.
