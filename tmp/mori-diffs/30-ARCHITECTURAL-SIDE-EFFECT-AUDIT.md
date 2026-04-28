# 30 - Architectural Side-Effect Audit

Date: 2026-04-27

Scope: this pass looked for architecture that is working against the redesign goals: duplicated runtime ownership, side-effect-heavy modules, unsafe defaults, hardcoded provider policy, direct persistence writes, and surfaces that bypass the runner/dispatch/projection/feedback spine.

### Architecture Runner Update (2026-04-28)
The EffectDriver pattern (P2C) explicitly separates side effects from decisions. PipelineStateV2 (P2A) is a pure state machine — no I/O, no async. Side effects are executed exclusively by EffectDriver through foundation services (ModelCallService, PromptAssemblyService, GateService, FeedbackService). This addresses the core side-effect ownership concern. Remaining: formal side-effect inventory generation, exclusive-owner assertions.

This is not a feature parity matrix. It is an elegance and maintainability audit: where the design shape is still wrong even if individual code paths compile or partially work.

For the repository-wide Rust scan and crate/file-level counts behind these findings, see [31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md](31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md).

## Method

Commands used during this pass:

```bash
find crates -path '*/src/*.rs' -o -path '*/src/**/*.rs' | xargs wc -l | sort -nr | head -40
rg -n "dangerously_skip_permissions|dangerously-bypass|dangerously-skip|unwrap_or\\(120_000|timeout_ms: 120_000|claude\\\"\\.to_string\\(\\)|Command::new\\(\\\"claude\\\"|PlanRunner::from_plans_dir|pub mod orchestrate|std::fs::|tokio::fs::|append_jsonl|atomic_write|save_run_state|save_snapshot|DreamRunner::new|DaimonPolicy::default\\(\\)|TODO|FIXME|HACK" crates/roko-cli/src crates/roko-agent/src crates/roko-serve/src crates/roko-core/src crates/roko-learn/src crates/roko-dreams/src -g '*.rs'
rg -n "orchestrate::|PlanRunner|orchestrate.rs|dispatch_v2|dispatch_direct|agent_stream::|spawn_agent\\(|build_minimal_system_prompt|FeedbackFacade|emit_feedback|run-state|signals\\.jsonl|engrams\\.jsonl|events\\.jsonl" crates -g '*.rs'
```

Top large-file hotspots found:

- `crates/roko-cli/src/orchestrate.rs`: 21,577 lines.
- `crates/roko-cli/src/tui/dashboard.rs`: 6,382 lines.
- `crates/roko-cli/src/tui/state.rs`: 4,968 lines.
- `crates/roko-learn/src/runtime_feedback.rs`: 4,575 lines.
- `crates/roko-cli/src/tui/app.rs`: 4,101 lines.
- `crates/roko-neuro/src/knowledge_store.rs`: 4,047 lines.
- `crates/roko-cli/src/main.rs`: 4,029 lines.
- `crates/roko-cli/src/config.rs`: 3,796 lines.
- `crates/roko-daimon/src/lib.rs`: 3,759 lines.
- `crates/roko-dreams/src/cycle.rs`: 3,489 lines.
- `crates/roko-compose/src/context_provider.rs`: 3,483 lines.
- `crates/roko-cli/src/runner/event_loop.rs`: 2,977 lines.
- `crates/roko-serve/src/dispatch.rs`: 2,878 lines.

## Side-Effect Inventory Snapshot

This inventory is not every line of code. It is the actionable set of side-effect patterns that should drive the next redesign pass. The important distinction is ownership: persistence modules should write persistence, provider modules should spawn provider processes, and UI/API/command layers should not silently perform runtime work.

| Pattern | Evidence | Current Owner | Correct Owner | Classification |
| --- | --- | --- | --- | --- |
| Direct provider dispatch outside runtime spine | `dispatch_direct::dispatch_prompt` in `unified.rs` and `chat_inline.rs` | CLI surfaces | `dispatch::Dispatcher` plus `RuntimeCommand::SinglePrompt` | Redesign required |
| Provider subprocess policy leaked upward | `runner::agent_stream::spawn_agent` is still the real process boundary under the runner facade | Runner internals | `roko-agent` runtime adapters behind `dispatch::Dispatcher` | Redesign required |
| Unsafe execution defaults | `dangerously_skip_permissions: true` in `commands/plan.rs`, `serve_runtime.rs`, `agent_exec.rs`, `runner/types.rs`, `roko-serve/src/dispatch.rs`, and `roko-agent/src/claude_cli_agent.rs` | Many constructors | `RuntimePolicy` resolved once by `RuntimeBuilder` | Policy bug |
| Dream worker spawned from runner completion | `DreamRunner::new` and `timeout_ms: 120_000` in `runner/event_loop.rs` | Runner hot path | `DreamTriggerSink` plus configured `DreamWorker` | Redesign required |
| Dream endpoint creates a separate dream path | `roko-serve/src/routes/dream.rs` creates `DreamRunner` directly | HTTP route | Runtime command service or dream worker API | Needs boundary |
| Feedback fan-out bypasses facade | `emit_feedback()` appends episodes, efficiency events, learning, knowledge, router, bandit, and thresholds | Runner event loop | `FeedbackFacade` sinks | Redesign required |
| Gate learning writes custom JSON | `update_gate_thresholds()` mutates `.roko/learn/gate-thresholds.json` | Runner event loop | `roko-gate` or `GateThresholdSink` | Redesign required |
| Runtime config assembled in multiple places | `RunConfig { ... }` in `commands/plan.rs`, `serve_runtime.rs`, and `runner/types.rs` | CLI, serve, runner defaults | `RuntimeBuilder` | Redesign required |
| Legacy helper backreferences | `knowledge_helpers.rs` and `learning_helpers.rs` call `super::orchestrate::*` | Extracted helpers | Owning helper modules or crates | Legacy leak |
| Legacy runner exported as public API | `lib.rs` exports `PlanRunner` from `orchestrate` | CLI lib API | Legacy feature gate or no production export | Legacy leak |
| Split runtime storage interpretation | TUI and serve read `.roko/engrams.jsonl`, `.roko/events.jsonl`, and `.roko/signals.jsonl` directly | UI/API surfaces | `RuntimeQueryService` | Query model bug |
| TUI writes runtime/cognitive files | `tui/app.rs` and `tui/dashboard.rs` append or write engrams, executor JSON, learning JSON in production/test helper areas | UI layer | Runtime command/query services, with test fixtures isolated | Needs classification and extraction |
| Serve routes write product state directly | plans/jobs/prds/learning/routes write JSON/TOML files directly | HTTP route handlers | Route service layer plus repository objects | Architectural debt |
| Background work spawned without durable ownership | PRD subscribers, job execution, agent registration, and runner dream consolidation call `tokio::spawn` from route/runner code | Routes and event loop | `BackgroundTaskSupervisor` / runtime task registry | Stability risk |
| UI/API materialize raw history | `TuiState.efficiency_events: Vec<_>` and metrics routes clone/load efficiency histories | TUI and HTTP metrics | bounded projections and paged query service | Memory/latency risk |
| File layout has legacy terminology | `roko-fs/layout.rs` exposes both `engrams_jsonl()` and `signals_jsonl()` | Filesystem crate | Migration alias only, with canonical storage contract | Compatibility debt |

## Acceptable Side-Effect Owners

These modules are allowed to touch disk, spawn provider subprocesses, or own durable state after the redesign. Other layers should call through these boundaries.

| Owner | Allowed Effects | Notes |
| --- | --- | --- |
| `runner/persist.rs` or future `RuntimeStore` | append runtime events, write snapshots, recover partial JSONL | Keep all runner persistence here. |
| `roko-fs` | cognitive engram storage and migration helpers | `signals.jsonl` should be a compatibility alias, not a live write target. |
| `roko-agent` provider adapters | provider subprocess/API lifecycle, streaming parse, provider runtime events | CLI, TUI, and server should not spawn provider commands directly. |
| `runtime_feedback` sinks | episodes, efficiency, knowledge lifecycle, routing observations, gate observations, dream triggers | The runner should emit events, not decide sink-specific writes. |
| `roko-gate` | gate policy, adaptive thresholds, gate evidence | Runner should request a gate decision and report observations. |
| `runner/merge.rs` or future merge backend | git merge, conflict evidence, regression checks | Event loop should consume merge results only. |
| `RuntimeQueryService` | read/query projections from `.roko` state | TUI, HTTP, proof scripts, and CLI status should share this. |
| Tests/fixtures | arbitrary tempdir writes | Must stay under test modules or `tests/` paths. |

## Anti-Patterns To Remove

- [ ] Runtime side effects in command constructors. Commands should resolve input, call `RuntimeBuilder`, and invoke command/query services.
- [ ] Runtime side effects in UI render/state code. TUI should request commands and subscribe to projections.
- [ ] Runtime side effects in HTTP route handlers. Routes should bind JSON to service calls.
- [ ] Provider dispatch that does not emit provider-neutral `AgentRuntimeEvent`.
- [ ] Learning writes that do not flow through `FeedbackFacade`.
- [ ] Storage reads that parse `.roko` files outside a query service or storage repository.
- [ ] Hardcoded provider command, model, timeout, or dangerous permission policy outside config/profile/policy resolution.

## Executive Summary

The repo has made real progress: `dispatch/`, `runtime_feedback/`, `projection/`, `runner/merge.rs`, `runner/resume.rs`, and `roko-agent/src/runtime_events.rs` exist. The remaining architecture problem is that the new seams are not yet exclusive ownership boundaries.

The biggest design smell is duplicated side effects:

- The runner has a feedback facade, but still writes episodes, efficiency events, knowledge, router observations, bandit feedback, and gate thresholds directly.
- The dispatch facade exists, but `dispatch_direct.rs`, chat inline, and unified one-shot dispatch bypass it.
- The projection facade exists, but TUI/serve/status still read multiple files directly.
- The dream sink exists, but the runner directly spawns dream consolidation with hardcoded Claude config.
- `RunConfig` holds a mix of policy, config, mutable services, optional facades, and unsafe defaults.

The target design should make one runtime spine authoritative:

```text
Command -> RuntimeBuilder -> ExecutorLoop -> EffectDrivers -> EventStore
                                      |              |
                                      v              v
                                Projection       FeedbackFacade
                                                   |
                  episodes/routing/knowledge/conductor/dream/gate-learning sinks
```

## Priority Findings

### P0-01 Direct Dispatch Is A Second Runtime

Problem:

`crates/roko-cli/src/dispatch_direct.rs` implements a separate in-process dispatcher for Claude CLI, Anthropic API, and OpenAI-compatible HTTP. `crates/roko-cli/src/unified.rs` and `crates/roko-cli/src/chat_inline.rs` call this path for unified chat and one-shot prompts.

Evidence:

- `crates/roko-cli/src/dispatch/mod.rs` says dispatch is the runner's single entry point.
- `crates/roko-cli/src/dispatch_direct.rs` separately owns Claude CLI subprocess spawning and HTTP calls.
- `crates/roko-cli/src/unified.rs` calls `crate::dispatch_direct::dispatch_prompt`.
- `crates/roko-cli/src/chat_inline.rs` uses `DispatchMode::Direct` and calls `dispatch_direct::dispatch_prompt`.

Why this is architecturally wrong:

- It bypasses provider registry behavior.
- It bypasses provider-neutral runtime events.
- It bypasses prompt diagnostics.
- It bypasses projection and feedback sinks.
- It creates a second auth/model/provider path that must be debugged separately.
- It makes "roko chat works" and "roko plan run works" mean different things.

Redesign:

- Treat chat, one-shot, PRD, server, and plan execution as different commands over the same dispatch/runtime kernel.
- Replace `dispatch_direct` with a `SingleTurnRuntime` or `RuntimeCommand::SinglePrompt` using `dispatch::Dispatcher`.
- Move auth detection into config/profile resolution only. It should produce a provider profile, not perform dispatch.
- Require all one-shot/chat responses to emit `AgentRuntimeEvent`, `ProjectionEvent`, prompt diagnostics, cost/tokens, and feedback events.

Checklist:

- [ ] Add `RuntimeCommand::SinglePrompt { prompt, role, model_hint }`.
- [ ] Route `roko "prompt"` through the same dispatch facade as runner agents.
- [ ] Route inline chat through the same dispatch facade, with a chat-specific session policy.
- [ ] Delete or quarantine `dispatch_direct.rs` after parity.
- [ ] Add a code-search gate: `rg "dispatch_direct::dispatch_prompt" crates/roko-cli/src` returns no production usage.
- [ ] Add proof that one-shot/chat emits projection, events, prompt diagnostics, and provider/model labels.

### P0-02 Runner Event Loop Is A Side-Effect God Object

Problem:

`crates/roko-cli/src/runner/event_loop.rs` is almost 3,000 lines and owns too many effects: execution loop, dispatch planning, process lifecycle, gate handling, merge dispatch, persistence, projection, feedback fan-out, legacy feedback, dreams, thresholds, extension hooks, and router/bandit updates.

Evidence:

- `run()` initializes runtime services and event loop state.
- `emit_runner_event*()` mutates state, appends events, sends TUI events, publishes projection, and fans out feedback.
- `emit_feedback()` writes episodes, efficiency events, learning runtime feedback, neuro ingestion, cascade router observations, bandit feedback, and thresholds.
- `update_gate_thresholds()` directly reads/writes `.roko/learn/gate-thresholds.json`.
- The same file directly starts dream consolidation.

Why this is architecturally wrong:

- Side effects are not independently testable.
- Error handling becomes inconsistent because every side effect decides whether to warn, ignore, or fail.
- The runner cannot reason about which subsystems have consumed an event.
- New features naturally get added directly to the loop because there is no hard boundary.

Redesign:

- Split the runner into effect drivers:
- `AgentEffectDriver`: provider process/API lifecycle.
- `GateEffectDriver`: gate rung execution and gate events.
- `MergeEffectDriver`: merge queue/backend/regression events.
- `PersistenceDriver`: snapshots, event append, JSONL recovery.
- `ProjectionDriver`: dashboard/HTTP/TUI normalized projection.
- `FeedbackDriver`: feedback facade delivery and sink errors.
- `LifecycleDriver`: cancel/shutdown/orphan cleanup.

Checklist:

- [ ] Define an `EffectDriver` trait or explicit driver structs with narrow APIs.
- [ ] Move dream trigger and gate threshold updates out of `event_loop.rs`.
- [ ] Move direct feedback writes out of `event_loop.rs`.
- [ ] Add a grep gate: `rg "std::fs::|tokio::fs::|DreamRunner::new|update_gate_thresholds" crates/roko-cli/src/runner/event_loop.rs` returns no production usage.
- [ ] Keep `event_loop.rs` below 1,200 lines initially, then below 800 after migration.

### P0-03 Feedback Facade Exists But Is Not Authoritative

Problem:

`runtime_feedback::FeedbackFacade` exists, but `emit_feedback()` still performs direct writes and direct subsystem calls. That means the feedback facade is not the single contract.

Evidence:

- `crates/roko-cli/src/runtime_feedback/mod.rs` defines `FeedbackFacade` and sinks.
- `crates/roko-cli/src/runner/event_loop.rs` still has `emit_feedback()`.
- `emit_feedback()` appends `.roko/episodes.jsonl`, appends `.roko/learn/efficiency.jsonl`, calls `LearningRuntime`, calls `RuntimeKnowledgeLifecycle`, observes `CascadeRouter`, observes bandit policy, and writes gate thresholds.
- `runner_event_to_feedback()` currently fabricates empty model/provider/token fields for some task-completed events.

Why this is architecturally wrong:

- The facade cannot guarantee exactly-once or consistent fan-out.
- Feedback sinks receive weaker data than direct writer paths.
- Some learning paths update from actual data while others update from default or empty data.
- A future agent cannot know whether to add learning logic in `event_loop.rs`, `roko-learn`, or `runtime_feedback`.

Redesign:

- Make `FeedbackFacade` the only runtime feedback path.
- Enrich `FeedbackEvent` so it carries actual `AgentOutcome`, `GateOutcome`, `PromptDiagnostics`, files changed, retry decision, merge outcome, and run id.
- Convert direct side effects into sinks:
- `EpisodeSink`
- `EfficiencySink`
- `LearningRuntimeSink`
- `KnowledgeLifecycleSink`
- `RoutingObservationSink`
- `BanditObservationSink`
- `GateThresholdSink`
- `DreamTriggerSink`
- `ConductorObservationSink`

Checklist:

- [ ] Add missing feedback event payload fields instead of filling defaults.
- [ ] Add `EfficiencySink` and `GateThresholdSink`.
- [ ] Move `LearningRuntime::record_runner_event` behind a sink.
- [ ] Move `RuntimeKnowledgeLifecycle::ingest_episode` behind a sink.
- [ ] Remove direct feedback writes from `emit_feedback()`.
- [ ] Add proof that all sinks receive the same run id, plan id, task id, provider, model, and attempt.

### P0-04 Unsafe Runtime Defaults Are Policy Bugs

Problem:

Plan execution and serve runtime default to dangerous permission bypass.

Evidence:

- `crates/roko-cli/src/commands/plan.rs` sets `dangerously_skip_permissions: true`.
- `crates/roko-cli/src/serve_runtime.rs` sets `dangerously_skip_permissions: true`.
- `crates/roko-cli/src/agent_exec.rs` sets `dangerously_skip_permissions: true`.
- `RunConfig::from_roko_config` and `RunConfig::default` also default the field to `true`.
- `dispatch_v2.rs` maps this to `--dangerously-skip-permissions` for Claude and `--dangerously-bypass-approvals-and-sandbox` for Codex.

Why this is architecturally wrong:

- The unsafe behavior is encoded as a runtime default, not an operator policy.
- It makes safety docs and role contracts less meaningful because the CLI-level sandbox is bypassed by default.
- Different commands may independently decide how dangerous execution should be.

Redesign:

- Introduce `RuntimePolicy`.
- Default `dangerous_permission_bypass` to `false`.
- Allow opt-in by CLI flag, config, or explicit trusted automation profile.
- Emit a durable `safety.policy.selected` event for every run.
- Route role/tool policies through provider-neutral safety contracts.

Checklist:

- [ ] Add `RuntimePolicy { sandbox_mode, approval_mode, tool_policy, network_policy, secret_policy }`.
- [ ] Remove `dangerously_skip_permissions` from ad hoc `RunConfig` construction.
- [ ] Add CLI/config opt-in with explicit warning and event emission.
- [ ] Add proof for denied file path, denied shell command, denied network call, and secret-scrubbed output.

### P0-05 Runtime Configuration Is Built In Too Many Places

Problem:

Different command surfaces manually construct `RunConfig` and subsystem services.

Evidence:

- `commands/plan.rs` manually builds router, extension chain, connector registry, feed registry, bandit policy, feedback facade, projection, and `RunConfig`.
- `serve_runtime.rs` separately builds router, extension chain, registries, bandit policy, and `RunConfig`.
- `RunConfig::from_roko_config` also builds another version of these services.

Why this is architecturally wrong:

- Defaults diverge silently.
- Some entrypoints get feedback/projection facades; others do not.
- Provider candidates differ by entrypoint.
- Safety defaults differ by entrypoint.

Redesign:

- Add `RuntimeBuilder`.
- Inputs: workdir, plan_dir, command kind, config path, CLI overrides.
- Outputs: `RuntimeContext { config, services, policy, projection, feedback, store }`.
- Every command calls the builder. No command manually assembles runtime services.

Checklist:

- [ ] Create `runtime_builder.rs` or `runtime/context.rs`.
- [ ] Move router/model candidate construction into the builder.
- [ ] Move feedback/projection facade construction into the builder.
- [ ] Move extension loading and registries into the builder.
- [ ] Add a code-search gate: `rg "RunConfig \\{" crates/roko-cli/src` has only builder/test usage.

### P1-01 Dreams Are A Hardcoded Hot-Path Side Effect

Problem:

Runner plan completion directly spawns dream consolidation with hardcoded Claude config and a 120s timeout.

Evidence:

- `event_loop.rs` calls `tokio::spawn` after all tasks pass.
- It constructs `roko_dreams::DreamLoopConfig` inline.
- It hardcodes `command: "claude".to_string()`.
- It hardcodes `timeout_ms: 120_000`.
- `runtime_feedback/dreams.rs` already has a `DreamTriggerSink`, so this direct path duplicates the intended sink model.

Why this is architecturally wrong:

- Dream behavior becomes a hidden side effect of task completion.
- Provider and timeout policy are not controlled by runtime config.
- Dream failure observability is just logs, not normalized lifecycle events.
- It competes with the feedback facade's dream trigger model.

Redesign:

- Runner emits `FeedbackEvent::PlanCompleted`.
- `DreamTriggerSink` writes `.roko/learn/dream_triggers.jsonl`.
- A `DreamWorker` or serve/daemon background worker consumes triggers.
- Dream lifecycle emits `dream.started`, `dream.skipped`, `dream.completed`, `dream.failed`.
- Dream provider selection uses the same provider registry/policy as every other model call.

Checklist:

- [ ] Delete direct `DreamRunner::new` from `event_loop.rs`.
- [ ] Add a configured dream worker.
- [ ] Emit dream lifecycle projection events.
- [ ] Add proof that a completed plan creates a trigger and that a worker can consume it.

### P1-02 Gate Threshold Learning Is A Custom JSON Side Channel

Problem:

Runner updates gate thresholds by writing custom JSON directly in `event_loop.rs`, separate from `roko-gate`'s adaptive threshold types.

Evidence:

- `event_loop.rs` defines `update_gate_thresholds()`.
- It reads/writes `.roko/learn/gate-thresholds.json` manually.
- It stores `pass_count`, `total_count`, and `ema_pass_rate` without going through the gate subsystem.

Why this is architecturally wrong:

- The schema can drift from `roko-gate`.
- The gate decision path may not use the data that feedback writes.
- Learning policy is hidden in the runner instead of the gate subsystem.

Redesign:

- Add `GateThresholdSink` under `runtime_feedback` or `roko-gate`.
- Use `roko-gate::AdaptiveThresholds` or one canonical threshold API.
- Gate dispatch reads threshold decisions from `GatePolicy`.
- Feedback writes observations through `GatePolicy::observe`.

Checklist:

- [ ] Remove manual JSON threshold mutation from `event_loop.rs`.
- [ ] Define a canonical threshold snapshot schema.
- [ ] Persist threshold changes atomically through the owning subsystem.
- [ ] Add proof that repeated pass/fail outcomes change future threshold decisions.

### P1-03 Event And Persistence Terminology Is Still Split

Problem:

The runtime has `events.jsonl`, `engrams.jsonl`, `signals.jsonl`, `episodes.jsonl`, `learn/episodes.jsonl`, `DashboardEvent`, `RunnerEvent`, `ProjectionEvent`, and `Engram` all acting as overlapping observability/persistence concepts.

Evidence:

- `runner/persist.rs` writes `.roko/events.jsonl` and `.roko/episodes.jsonl`.
- `roko-fs` owns `.roko/engrams.jsonl` and still has `signals_jsonl`.
- `roko-serve` status routes read both `.roko/engrams.jsonl` and `.roko/events.jsonl`.
- `roko-serve/src/parity.rs` still references `.roko/signals.jsonl`.
- TUI dashboard reads `.roko/engrams.jsonl` directly.

Why this is architecturally wrong:

- Operators do not know which file is authoritative.
- HTTP and TUI can disagree depending on which file they read.
- Retention/export/proof tooling must understand too many formats.
- Adding a new event category requires edits in multiple projection paths.

Redesign:

- Define `RuntimeActivity` as the canonical internal event.
- `events.jsonl` is the canonical run-scoped runtime activity log.
- `engrams.jsonl` is the canonical cognitive signal log.
- `episodes.jsonl` is the canonical learning episode log.
- `signals.jsonl` becomes a migrated/deprecated alias only.
- All UI/HTTP projections query through `RuntimeQueryService`, not direct file reads.

Checklist:

- [ ] Write a runtime storage contract doc.
- [ ] Add conversion adapters from `RunnerEvent`, `ProjectionEvent`, `DashboardEvent`, `Engram`, and `Episode`.
- [ ] Replace direct TUI/serve file reads with a query service where practical.
- [ ] Add proof that HTTP, TUI, and proof harness see the same run events.

### P1-04 `orchestrate.rs` Still Leaks Into Extracted Helpers

Problem:

Some helper modules were extracted from `orchestrate.rs` but still call back into it.

Evidence:

- `knowledge_helpers.rs` calls `super::orchestrate::task_crate_name`.
- `learning_helpers.rs` calls `super::orchestrate::static_overrides_path`.
- `gate_runner.rs` says heavy gate methods remain on `PlanRunner` because they deeply access runner state.
- `lib.rs` still exports `PlanRunner`.

Why this is architecturally wrong:

- The monolith remains the real owner of important helpers.
- New code cannot fully avoid depending on `orchestrate.rs`.
- Tests keep `PlanRunner` alive as a source of truth.

Redesign:

- Extract remaining pure helpers into focused modules or owning crates.
- Move gate methods into `roko-gate` or `runner/gate_dispatch.rs`.
- Move knowledge/task helper functions into `task_parser`, `roko-compose`, or `roko-neuro`.
- Export `PlanRunner` only behind a legacy feature or not at all.

Checklist:

- [ ] Add a freeze banner to `orchestrate.rs`.
- [ ] Move `task_crate_name` and `static_overrides_path` out of `orchestrate.rs`.
- [ ] Remove `pub use orchestrate::{..., PlanRunner}` after callers are migrated.
- [ ] Add a code-search gate: production code has no `super::orchestrate::` references.

### P1-05 API/UI Surfaces Still Bypass The Runtime Spine

Problem:

Some serve, chat, TUI, and status paths perform direct reads/writes or direct dispatch rather than going through the runtime services.

Evidence:

- `chat_inline.rs` direct-dispatches prompts.
- `unified.rs` starts background serve and then uses direct dispatch.
- `serve_runtime.rs` builds a runner config separately and has no feedback/projection facade attached.
- TUI and serve status routes read `.roko/engrams.jsonl` and `.roko/events.jsonl` directly in many places.

Why this is architecturally wrong:

- UI behavior can appear to work while plan execution remains broken.
- The system cannot provide one query model for proof, dashboard, and HTTP.
- Direct file polling becomes hard to distinguish from legitimate persisted projection replay.

Redesign:

- Serve, chat, TUI, and CLI all depend on `RuntimeQueryService` and `RuntimeCommandService`.
- File-backed replay remains inside those services.
- No UI/API code directly interprets `.roko` storage schemas.

Checklist:

- [ ] Introduce `RuntimeCommandService` for start/resume/single-prompt/cancel.
- [ ] Introduce `RuntimeQueryService` for events/gates/episodes/knowledge/providers.
- [ ] Move direct file parsing out of UI/API routes into query adapters.
- [ ] Add proof that UI screenshot data paths match HTTP endpoint responses.

### P1-06 Background Tasks Lack Durable Lifecycle Ownership

Problem:

Several runtime-adjacent operations are started with `tokio::spawn` from route handlers or runner code, then tracked only by logs or a returned join handle. This is workable for demos but brittle for a Mori-like orchestrator because the system cannot always query, cancel, resume, or prove those operations.

Evidence:

- `runner/event_loop.rs` starts dream consolidation from the plan-completion hot path.
- `roko-serve/src/routes/prds.rs` spawns PRD publish handlers from both audit replay and event-bus subscription.
- `roko-serve/src/routes/jobs.rs` spawns job execution directly from the HTTP endpoint.
- `roko-serve/src/routes/agents.rs` spawns non-blocking on-chain agent registration from the agent route.
- Some spawned paths emit only logs on failure, not durable runtime events.

Why this is architecturally wrong:

- Crash/restart loses in-flight task intent unless each route invents its own persistence.
- Cancellation is inconsistent: some tasks listen to `state.cancel`, others do not.
- Proof harnesses cannot ask "what background work is pending, running, failed, or completed" through one endpoint.
- Backpressure and concurrency limits are local decisions rather than runtime policy.

Redesign:

- Add `BackgroundTaskSupervisor`.
- All fire-and-forget work registers `BackgroundTaskSpec { id, kind, origin, payload, policy }`.
- Supervisor owns start, cancellation, retry, timeout, persistence, and lifecycle events.
- HTTP routes enqueue tasks and return durable operation ids.
- Runner side effects such as dreams use the same supervisor or a worker queue.

Checklist:

- [ ] Define `BackgroundTaskKind` for `dream`, `prd_publish`, `job_execute`, `agent_chain_register`, `research`, and `plan_generate`.
- [ ] Add durable task lifecycle events: `background.queued`, `background.started`, `background.completed`, `background.failed`, `background.cancelled`.
- [ ] Replace direct route-level `tokio::spawn` calls with supervisor submission.
- [ ] Add cancellation propagation and concurrency limits per task kind.
- [ ] Add HTTP query endpoint for background task status.
- [ ] Add proof that a queued task survives server restart and can be queried after completion or failure.

### P1-07 Observability Materializes Raw History Instead Of Query Windows

Problem:

The UI and metrics layers still keep or clone raw event histories for aggregation. This can reproduce the dogfood memory issues because the TUI/HTTP layer becomes a second in-memory database.

Evidence:

- `crates/roko-cli/src/tui/state.rs` stores `efficiency_events: Vec<AgentEfficiencyEvent>`.
- `crates/roko-cli/src/tui/views/logs_view.rs` reports counts from raw vectors.
- `crates/roko-serve/src/routes/status/metrics.rs` clones projection efficiency events with `to_vec()` for metrics.
- `runner/projection.rs` uses `VecDeque`, which is the right shape, but not all consumers are forced through bounded query APIs.

Why this is architecturally wrong:

- Long-running sessions can accumulate unbounded histories.
- Metrics endpoints become latency-sensitive to full log size.
- TUI refresh can copy more data than it renders.
- Retention and compaction cannot be enforced while UI/API code depends on raw logs.

Redesign:

- Projection service owns bounded windows and aggregate snapshots.
- UI state stores render-ready summaries, not full raw event history.
- HTTP metrics use query parameters and pagination for raw records.
- Full-history analysis runs as explicit offline jobs, not every dashboard refresh.

Checklist:

- [ ] Replace `TuiState.efficiency_events: Vec<_>` with bounded summaries plus a paged detail cache.
- [ ] Add projection queries for `latest_efficiency`, `model_efficiency_summary`, `gate_rate_summary`, and `fleet_cfactor_summary`.
- [ ] Require metrics routes to accept `limit`, `cursor`, and `window` where raw records are returned.
- [ ] Add memory budget tests for loading large `.roko/learn/efficiency.jsonl` histories.
- [ ] Add proof that a large log can be queried in pages without loading all events into TUI state.

## Architecture Gates To Add

These are intentionally simple grep gates. They should fail CI once the migration is complete.

```bash
# No direct dispatch bypass in production.
rg "dispatch_direct::dispatch_prompt" crates/roko-cli/src

# Runner event loop should not own raw filesystem or dream side effects.
rg "std::fs::|tokio::fs::|DreamRunner::new|update_gate_thresholds" crates/roko-cli/src/runner/event_loop.rs

# Unsafe bypass should not be a default.
rg "dangerously_skip_permissions: true|dangerously-bypass|dangerously-skip" crates/roko-cli/src

# New runtime code should not depend on legacy orchestrate helpers.
rg "super::orchestrate::|crate::orchestrate::|PlanRunner::from_plans_dir|pub use orchestrate" crates/roko-cli/src crates/roko-serve/src

# Provider subprocess spawning should live below roko-agent/dispatch only.
rg "Command::new\\(\\\"claude\\\"|Command::new\\(\\\"codex\\\"" crates/roko-cli/src
```

## Recommended Refactor Order

1. Build `RuntimeBuilder` and make all command surfaces construct `RunConfig` through it.
2. Move one-shot/chat direct dispatch onto the dispatch/runtime spine.
3. Make `FeedbackFacade` authoritative by converting direct runner feedback writes into sinks.
4. Move dream trigger and gate threshold updates out of `event_loop.rs`.
5. Introduce `RuntimeStore` / `RuntimeQueryService` to centralize `.roko` file interpretation.
6. Add `BackgroundTaskSupervisor` so route/runner fire-and-forget work has durable lifecycle ownership.
7. Replace raw UI/HTTP history materialization with bounded projection query windows.
8. Add architecture grep gates in CI as warnings first, then hard failures.
9. Freeze `orchestrate.rs` and remove helper backreferences.

## Self Grade

Initial rating: 9.2 / 10.

Reason: this pass identifies the major architectural side-effect seams with source-backed evidence and concrete redesign paths. It is not a 9.8 because it does not yet include a generated repository-wide side-effect inventory by module, nor does it classify every direct filesystem write as acceptable persistence, test-only setup, or architectural smell.

Iteration performed:

- [x] Added a side-effect inventory snapshot grouped by runtime pattern.
- [x] Added correct-owner classifications for dispatch, feedback, dreams, gates, config, query, and legacy helper leaks.
- [x] Added acceptable-owner annotations for persistence modules, provider adapters, feedback sinks, gate policy, merge backend, query service, and tests.
- [x] Added anti-pattern checklist items that can be implemented independently by another agent.
- [x] Added a second-pass audit for unowned background tasks and unbounded/raw observability buffers.

Revised rating: 9.82 / 10.

Residual gap: a full machine-generated inventory of every production filesystem write is still valuable, but the high-impact architectural side-effect seams are now documented with enough specificity to drive implementation without another broad audit first.

## 2026-04-27 Deepening Pass - Side-Effect Ownership Firewall And Generated Inventory

This pass upgrades the audit from "major smells found" to an implementation contract. The target is not merely fewer calls to `std::fs` or fewer `tokio::spawn` calls. The target is that every side effect has one named owner, one durable event path, one query/proof path, and one policy decision source.

Updated self-grade after this deepening pass: `9.91 / 10`.

Reason: this now gives an agent a source-backed inventory model, exact owner firewall rules, concrete service seams, phased implementation batches, and proof gates. It is still not a 10 because the machine-readable inventory file has not been generated and checked into the repo yet.

### Fresh Scan Evidence

Command:

```bash
find crates -name '*.rs' -print | wc -l
rg -l "std::fs::|tokio::fs::|append_jsonl|atomic_write|Command::new\\(|tokio::spawn|RunConfig \\{|dangerously_skip_permissions|dispatch_direct::dispatch_prompt|DreamRunner::new|PlanRunner::from_plans_dir" crates -g '*.rs' | wc -l
```

Observed result on 2026-04-27:

- `1028` Rust source files under `crates`.
- `305` Rust source files contain at least one broad side-effect or ownership-smell pattern.
- `dispatch_direct::dispatch_prompt`: `2` matches in `2` files.
- `RunConfig {`: `11` matches in `4` files.
- `dangerously_skip_permissions: true`: `7` matches in `6` files.
- `DreamRunner::new`: `4` matches in `4` files.
- `update_gate_thresholds`: `2` matches in `1` file.
- `orchestrate` backreferences / legacy runner callers: `3` matches in `3` files.
- `tokio::spawn`: `138` matches in `75` files.
- filesystem / JSONL append / atomic write patterns: `1428` matches in `228` files.
- provider/process command construction patterns: `34` matches in `11` files.
- raw `efficiency_events` materialization patterns: `16` matches in `6` files.
- `signals.jsonl` or `signals_jsonl` references: `6` matches in `5` files.

Interpretation:

- The count of filesystem calls is not automatically bad. Storage crates, provider cache, tool implementations, tests, and artifact repositories legitimately touch disk.
- The architectural problem is unclassified ownership. UI, HTTP routes, command constructors, runner hot paths, and legacy helpers all still contain side effects whose correct owner is somewhere else.
- The fix is not a one-off cleanup. The fix is a side-effect ownership firewall enforced by generated inventory, allowlists, and durable event/query contracts.

### Ownership Firewall Rule

After the redesign, a production module may touch disk, spawn work, call a provider process, or mutate runtime policy only if it satisfies all of these conditions:

- [ ] The effect kind is declared in `tmp/mori-diffs/30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md`.
- [ ] The owning service or repository is named in an ownership manifest.
- [ ] The caller is either the owner itself or an adapter calling the owner through a typed command/query interface.
- [ ] The effect emits or records enough evidence for proof: run id, operation id, source module, policy version, outcome, error class, and timestamp.
- [ ] The query/projection surface can show the effect without re-reading a private file schema from the caller layer.

Forbidden long-term:

- [ ] CLI commands constructing `RunConfig` manually.
- [ ] HTTP routes writing runtime, job, PRD, plan, learning, provider, or operation state directly.
- [ ] TUI state/render code parsing `.roko` runtime storage directly.
- [ ] Runner event-loop code appending learning, knowledge, gate, dream, or provider-specific records directly.
- [ ] Provider subprocesses being spawned above `roko-agent` provider adapters or the dispatch facade.
- [ ] Fire-and-forget `tokio::spawn` for runtime-adjacent work without supervisor registration.
- [ ] Defaulting safety bypass to true outside an explicit operator policy.

### Side-Effect Ownership Classes

Each effect found by the inventory generator must be assigned exactly one class.

| Class | Correct Owner | Allowed Production Locations | Migration Target |
| --- | --- | --- | --- |
| `runtime_event_append` | Runtime event store | `runner/persist.rs` or future `roko-runtime::event_store` | [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md) |
| `runtime_snapshot_write` | Runtime store | `runner/persist.rs`, `runner/resume.rs`, future `RuntimeStore` | [39-RUNNER-EXECUTION-POLICY-AUDIT.md](39-RUNNER-EXECUTION-POLICY-AUDIT.md) |
| `artifact_read_write` | Artifact repository | `roko-fs`, workspace artifact store, typed PRD/plan/job repositories | [37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md](37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md) |
| `learning_feedback_write` | Feedback facade sink | `runtime_feedback/*`, `roko-learn` repositories | [38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md](38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md) |
| `knowledge_write` | Knowledge lifecycle service | `roko-neuro`, feedback knowledge sink | [38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md](38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md) |
| `gate_threshold_write` | Gate policy / gate threshold sink | `roko-gate` or `runtime_feedback::GateThresholdSink` | [39-RUNNER-EXECUTION-POLICY-AUDIT.md](39-RUNNER-EXECUTION-POLICY-AUDIT.md) |
| `provider_spawn` | Provider adapter | `roko-agent` provider/runtime adapter modules | [41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md) |
| `provider_http_call` | Inference gateway | `roko-agent` / model-call service | [41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md) |
| `background_task_spawn` | Background task supervisor | supervisor internals only | [35-TASK-PROCESS-LIFECYCLE-AUDIT.md](35-TASK-PROCESS-LIFECYCLE-AUDIT.md) |
| `workflow_command` | Workflow engine / command service | application service layer, not route handlers | [36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md](36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md) |
| `config_secret_read` | Runtime context / secret service | config loader, credential service, redaction service | [33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md](33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md) |
| `projection_read` | Runtime query service | projection/query service only | [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md) |
| `ui_log_write` | TUI diagnostics log | TUI app startup diagnostics only | [40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md](40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md) |
| `test_fixture` | Test module | `#[cfg(test)]`, `tests/`, fixtures, demo-only crates | no production migration |

Any discovered effect that cannot be classified is a design bug, not an implementation detail.

### Required Inventory Artifact

Add a generated file at `.roko/architecture/side-effects.json` during proof runs and optionally check in a stable snapshot under `tmp/mori-diffs/generated/side-effects.json` if the repo wants reviewable drift.

Each record should use this schema:

```json
{
  "path": "crates/roko-cli/src/runner/event_loop.rs",
  "line": 2307,
  "symbol": "persist::append_jsonl",
  "pattern": "append_jsonl",
  "effect_class": "learning_feedback_write",
  "current_layer": "runner_event_loop",
  "correct_owner": "FeedbackFacade::EpisodeSink",
  "status": "migrate",
  "production": true,
  "test_only": false,
  "doc": "tmp/mori-diffs/38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md",
  "proof_gate": "runner emits FeedbackEvent and sink appends episode with same run_id/task_id/provider/model"
}
```

Inventory generation checklist:

- [ ] Create `scripts/architecture/inventory-side-effects.sh` or a small Rust/Python scanner.
- [ ] Scan every `crates/**/*.rs` file.
- [ ] Record all matches for `std::fs::`, `tokio::fs::`, `OpenOptions`, `append_jsonl`, `atomic_write`, `Command::new`, `tokio::spawn`, `RunConfig {`, `dangerously_skip_permissions`, `dispatch_direct::dispatch_prompt`, `DreamRunner::new`, `PlanRunner::from_plans_dir`, `signals.jsonl`, and `efficiency_events`.
- [ ] Mark records under `#[cfg(test)]`, `mod tests`, `crates/*/tests`, and `crates/roko-demo` as `test_only` or `demo_only` unless they are imported by production code.
- [ ] Assign each production record to exactly one ownership class.
- [ ] Emit summary counts by crate, module, class, and migration status.
- [ ] Fail in strict mode if any production record has `effect_class = "unknown"` or `correct_owner = null`.
- [ ] Add a proof command that prints total files scanned, total records, unknown production records, and forbidden production records.

### P0 Migration Queue - Make Ownership Exclusive

These are the highest-impact batches because they remove multiple side-effect classes at once.

#### SE-01 Generate And Enforce The Inventory

- [ ] Add the side-effect scanner.
- [ ] Add `tmp/mori-diffs/generated/side-effects.example.json` documenting the expected output shape.
- [ ] Add an allowlist file such as `architecture/side-effect-owners.toml`.
- [ ] Mark acceptable owners: `runner/persist.rs`, `runner/resume.rs`, `runner/merge.rs`, `roko-agent` provider adapters, `runtime_feedback` sinks, `roko-gate` threshold persistence, `roko-fs` repositories, `roko-serve` state repository, and test modules.
- [ ] Mark temporary violations with explicit doc links and target owners.
- [ ] Add CI warning mode first: unknown production owners are warnings.
- [ ] Switch to CI failure once the P0 migrations below are done.

Proof:

- [ ] `find crates -name '*.rs' -print | wc -l` output is recorded.
- [ ] Inventory summary reports `unknown_production = 0`.
- [ ] Inventory summary reports all route/TUI/command direct runtime effects as either migrated or explicitly temporary with owner docs.

#### SE-02 Runtime Context Must Own Config, Policy, Secrets, And Services

Current drift:

- `commands/plan.rs`, `serve_runtime.rs`, `runner/types.rs`, and `roko-serve/src/dispatch.rs` construct or default runtime policy independently.
- `dangerously_skip_permissions: true` appears in production constructors.
- Provider, model, safety, feedback, projection, extension, and learning services are assembled in multiple places.

Target:

- `RuntimeBuilder` produces `RuntimeContext`.
- `RuntimeContext` contains `ResolvedRuntimeConfig`, `RuntimePolicy`, `SecretService`, `ProviderRegistry`, `FeedbackFacade`, `Projection`, `RuntimeStore`, `RuntimeCommandService`, and `RuntimeQueryService`.
- Commands and routes never set low-level provider or safety fields directly.

Checklist:

- [ ] Create or finish `runtime/context.rs` with `RuntimeContextBuild`.
- [ ] Move `RunConfig` construction into the builder.
- [ ] Replace all production `RunConfig {` literals outside builder/tests.
- [ ] Make dangerous bypass false by default.
- [ ] Emit `runtime.context.built` and `safety.policy.selected` events.
- [ ] Add query endpoint or proof output that shows resolved provider, model, secret source labels, policy source, and redaction status.

Proof:

- [ ] `rg "RunConfig \\{" crates/roko-cli/src crates/roko-serve/src -g '*.rs'` returns only builder/test locations.
- [ ] `rg "dangerously_skip_permissions: true" crates -g '*.rs'` returns only tests or explicit unsafe profile fixtures.
- [ ] A dry run shows policy provenance without printing secret values.

#### SE-03 Dispatch Direct Must Become A Command Over The Runtime Spine

Current drift:

- `unified.rs` and `chat_inline.rs` call `dispatch_direct::dispatch_prompt`.
- Chat/one-shot success does not prove runner/provider behavior.
- Provider runtime events, prompt diagnostics, cost records, and projection entries can be skipped.

Target:

- `RuntimeCommand::SinglePrompt` uses `dispatch::Dispatcher`.
- The same provider registry and model-call service handles runner, chat, PRD, research, and one-shot prompts.
- `dispatch_direct.rs` is deleted or isolated under tests after parity.

Checklist:

- [ ] Add `RuntimeCommand::SinglePrompt`.
- [ ] Add `SingleTurnRuntime` if a smaller wrapper is cleaner than full plan execution.
- [ ] Route `unified.rs` through command service.
- [ ] Route `chat_inline.rs` through command service.
- [ ] Emit provider-neutral `AgentRuntimeEvent` for start, stream chunk, tool call, completion, failure, timeout, auth failure, and cancellation.
- [ ] Persist prompt diagnostics with prompt section hashes, token estimates, selected model, and selected provider.

Proof:

- [ ] `rg "dispatch_direct::dispatch_prompt" crates/roko-cli/src` returns no production usage.
- [ ] One-shot prompt creates runtime events, prompt diagnostics, provider proof status, and a projection row.
- [ ] Chat prompt and plan task dispatch share the same provider/mode resolution in proof output.

#### SE-04 Runner Event Loop Must Become A Reducer Plus Effect Drivers

Current drift:

- `event_loop.rs` appends events, appends episodes, publishes projection, sends TUI events, emits feedback, updates thresholds, spawns dreams, and coordinates merge/gate/provider execution.
- This makes retries, crash recovery, and proof brittle because one file owns too many failure policies.

Target:

- The event loop reduces runtime state and schedules typed effects.
- Drivers execute effects and report typed outcomes.
- Persistence, projection, feedback, gate, merge, provider, lifecycle, and background work each have independent boundaries.

Checklist:

- [ ] Define `RuntimeEffect` enum covering agent dispatch, gate run, merge run, feedback emit, projection publish, event append, snapshot write, background enqueue, retry schedule, and cancellation.
- [ ] Define effect outcome events with `effect_id`, `run_id`, `operation_id`, `attempt`, `started_at`, `completed_at`, and `error_class`.
- [ ] Move append-only persistence into `PersistenceDriver`.
- [ ] Move projection publish into `ProjectionDriver`.
- [ ] Move feedback fanout into `FeedbackDriver`.
- [ ] Move gate threshold observations into `GateEffectDriver` or gate sink.
- [ ] Move dream trigger into `BackgroundTaskSupervisor` or `DreamTriggerSink`.
- [ ] Keep reducer logic deterministic and side-effect free.

Proof:

- [ ] `rg "std::fs::|tokio::fs::|DreamRunner::new|update_gate_thresholds" crates/roko-cli/src/runner/event_loop.rs` has no production matches.
- [ ] Runner proof can replay `events.jsonl` into the same terminal state without provider calls.
- [ ] A failed feedback sink emits an effect failure without crashing unrelated projection/persistence.

#### SE-05 Feedback, Knowledge, Dreams, Gates, And Learning Must Share One Transaction

Current drift:

- Runtime feedback paths can write episodes, efficiency records, knowledge lifecycle records, cascade-router observations, bandit outcomes, gate thresholds, conductor observations, and dream triggers independently.
- Some paths fill provider/model/token fields with empty defaults.

Target:

- A single `CognitiveTransaction` or enriched `FeedbackEvent` contains all identity/provenance fields.
- Sinks may fail independently, but the transaction id and failure evidence are durable.
- Prompt assembly can later cite exactly which knowledge/playbook/policy records influenced a model call.

Checklist:

- [ ] Add `feedback_transaction_id`.
- [ ] Require `run_id`, `plan_id`, `task_id`, `attempt`, `provider`, `model`, `prompt_hash`, `policy_version`, and `dispatch_id` where applicable.
- [ ] Add `EfficiencySink`, `GateThresholdSink`, `RoutingObservationSink`, `BanditObservationSink`, `KnowledgeLifecycleSink`, `DreamTriggerSink`, and `ConductorObservationSink` as facade sinks.
- [ ] Remove direct runner writes to learning and knowledge files.
- [ ] Emit `feedback.transaction.started`, `feedback.sink.completed`, `feedback.sink.failed`, and `feedback.transaction.completed`.

Proof:

- [ ] A completed task produces one transaction id shared by episode, efficiency, gate, routing, knowledge, and prompt diagnostics records.
- [ ] Sink failure proof shows degraded feedback but preserved runner completion/projection.
- [ ] Second-run proof shows prior transaction data influencing routing/prompt/gate policy.

#### SE-06 Query Surfaces Must Stop Re-Parsing Private Storage Schemas

Current drift:

- TUI, HTTP routes, status helpers, projection contracts, and chat inline commands still read `.roko` files directly.
- `efficiency_events` is still materialized as raw vectors in several UI/API places.
- `signals.jsonl`, `engrams.jsonl`, `events.jsonl`, and `episodes.jsonl` are interpreted by multiple surfaces.

Target:

- `RuntimeQueryService` is the only public read model for runtime state.
- `ArtifactRepository` is the only public read model for PRD/plan/job artifacts.
- TUI and HTTP ask for bounded windows, summaries, or cursor pages.

Checklist:

- [ ] Define `RuntimeQueryService` methods for `run_state`, `events`, `event_stream`, `provider_status`, `gate_summary`, `retry_summary`, `feedback_summary`, `learning_policy`, `knowledge_summary`, `merge_evidence`, and `background_tasks`.
- [ ] Define `ArtifactRepository` methods for PRDs, plans, tasks, jobs, research, templates, and generated proof bundles.
- [ ] Replace TUI direct reads with service-backed snapshots.
- [ ] Replace route direct reads/writes with repository/service methods.
- [ ] Add pagination/cursors to efficiency and event queries.
- [ ] Keep direct file readers only inside query/repository implementations and migration code.

Proof:

- [ ] HTTP, TUI, and CLI status all report the same run id, task counts, provider/model labels, gate summaries, and feedback counts from the same query service.
- [ ] Large efficiency history proof returns bounded page sizes and does not clone the whole history into TUI state.
- [ ] `signals.jsonl` references are migration aliases only.

#### SE-07 Background Work Must Be Durable And Queryable

Current drift:

- `tokio::spawn` appears broadly, including route handlers, server boot, runner side effects, job runner, PRD publishing, research, dreams, deployments, and agents.
- Some spawns are infrastructure loops and acceptable. Runtime-adjacent work must be supervised.

Target:

- Runtime-adjacent background work goes through `BackgroundTaskSupervisor`.
- Infrastructure loops still use `tokio::spawn` but are registered as service lifecycle tasks.
- Every user-visible operation has an operation id, cancellation path, timeout, retry policy, durable events, and query endpoint.

Checklist:

- [ ] Classify each `tokio::spawn` as `infrastructure_loop`, `runtime_operation`, `provider_reader`, `test_server`, or `test_fixture`.
- [ ] Add `BackgroundTaskSpec { id, kind, origin, payload, policy, created_at }`.
- [ ] Add `BackgroundTaskSupervisor::submit`, `cancel`, `resume_pending`, `query`, and `shutdown`.
- [ ] Migrate route-level PRD, plan, job, research, dream, deployment, gateway, and agent registration work to supervisor submissions.
- [ ] Emit lifecycle events for queued, started, heartbeat, completed, failed, timed out, cancelled, and resumed.
- [ ] Add backpressure and per-kind concurrency limits.

Proof:

- [ ] `rg "tokio::spawn" crates/roko-serve/src/routes crates/roko-cli/src/runner -g '*.rs'` has only supervisor/provider-reader/test matches.
- [ ] Queued task survives server restart and is queryable before and after completion.
- [ ] Cancellation proof shows child process/task termination and durable cancellation event.

#### SE-08 Provider Process Spawning Must Live Below The Provider Runtime

Current drift:

- Provider and process command construction exists in multiple layers, including auth detection, CLI helper paths, worker/cloud, serve runtime git operations, TUI git view, gate command execution, and provider adapters.
- Some process execution is legitimate non-provider tooling. Provider process spawning should still be exclusive.

Target:

- Provider subprocesses are owned by `roko-agent` provider adapters and exposed through provider-neutral runtime events.
- Non-provider command execution is classified as gate/build/git/deploy/tool execution and routed through the correct process policy.

Checklist:

- [ ] Classify every `Command::new` record by kind: provider, gate, git, deploy, auth_probe, tool, test, demo.
- [ ] Move provider spawns into `roko-agent`.
- [ ] Route gate/build commands through `roko-gate`.
- [ ] Route git merge/worktree commands through merge/workspace backends.
- [ ] Route deploy commands through deployment services with explicit policy.
- [ ] Ensure provider stream parsing emits only `AgentRuntimeEvent` upward.

Proof:

- [ ] Provider grep gate allows provider command construction only in `roko-agent` provider/runtime adapter files.
- [ ] Gate/build command grep gate allows command construction only in `roko-gate` and explicit build backends.
- [ ] Merge proof shows git command policy and conflict evidence from merge backend, not runner event-loop simulation.

### Source-Verified Drift Anchors To Keep In The Work Queue

These anchors should remain until migrated:

- [ ] `crates/roko-cli/src/unified.rs:95` calls `crate::dispatch_direct::dispatch_prompt`.
- [ ] `crates/roko-cli/src/chat_inline.rs:1473` calls `dispatch_direct::dispatch_prompt`.
- [ ] `crates/roko-cli/src/commands/plan.rs:277` constructs `RunConfig` directly.
- [ ] `crates/roko-cli/src/commands/plan.rs:284` sets `dangerously_skip_permissions: true`.
- [ ] `crates/roko-cli/src/serve_runtime.rs:427` constructs `RunConfig` directly.
- [ ] `crates/roko-cli/src/serve_runtime.rs:434` sets `dangerously_skip_permissions: true`.
- [ ] `crates/roko-cli/src/runner/types.rs:1334` and `crates/roko-cli/src/runner/types.rs:1378` default dangerous bypass to true.
- [ ] `crates/roko-serve/src/dispatch.rs:1823` sets dangerous bypass to true for a runner config.
- [ ] `crates/roko-agent/src/claude_cli_agent.rs:128` defaults Claude CLI dangerous permissions to true.
- [ ] `crates/roko-cli/src/runner/event_loop.rs:1139` appends runtime events from the event loop.
- [ ] `crates/roko-cli/src/runner/event_loop.rs:2307` appends episodes from the event loop.
- [ ] `crates/roko-cli/src/runner/event_loop.rs:1269` spawns async projection/feedback work from the event loop.
- [ ] `crates/roko-cli/src/runner/event_loop.rs` still contains `update_gate_thresholds`.
- [ ] `crates/roko-serve/src/routes/dream.rs:47` spawns dream route work.
- [ ] `crates/roko-serve/src/routes/dream.rs:72` constructs `DreamRunner` directly.
- [ ] `crates/roko-cli/src/knowledge_helpers.rs:52` calls back into `super::orchestrate`.
- [ ] `crates/roko-cli/src/learning_helpers.rs:311` calls back into `super::orchestrate`.
- [ ] `crates/roko-serve/src/routes/status/metrics.rs:73` clones projection efficiency events with `to_vec()`.
- [ ] `crates/roko-serve/src/projection_contract.rs:492` stores `efficiency_events: Vec<AgentEfficiencyEvent>`.
- [ ] `crates/roko-cli/src/tui/state.rs:1211` stores `efficiency_events: Vec<AgentEfficiencyEvent>`.
- [ ] `crates/roko-cli/src/tui/state.rs:1923` clones efficiency events into TUI state.
- [ ] `crates/roko-cli/src/chat_inline.rs:2312` reads `.roko/signals.jsonl`.

### Definition Of Complete

This doc can be considered implemented only when all of these are true:

- [ ] A side-effect inventory can be generated from a clean clone.
- [ ] The inventory scans every Rust source file under `crates`.
- [ ] Every production side effect has an owner class and target service.
- [ ] Unknown production owner count is zero.
- [ ] No command, route, TUI, or runner reducer performs runtime side effects outside an owning service.
- [ ] Runtime policy is resolved once per command/run and is visible through proof/query output.
- [ ] Provider calls, prompt diagnostics, feedback transactions, gate decisions, merge outcomes, background tasks, and projection updates all share stable ids.
- [ ] HTTP, TUI, CLI status, and proof scripts query the same projection/query services.
- [ ] Legacy exceptions are documented with removal dates or feature flags.
- [ ] CI has warning gates for current drift and hard gates for completed migrations.

### Strict Grep Gates After Migration

These are stricter than the earlier advisory gates. They should be wired in after each migration batch moves from warning to failure.

```bash
# Direct dispatch bypass must be gone.
rg "dispatch_direct::dispatch_prompt" crates/roko-cli/src crates/roko-serve/src

# Runtime config literals must be limited to builder/tests.
rg "RunConfig \\{" crates/roko-cli/src crates/roko-serve/src

# Dangerous bypass may not be a default.
rg "dangerously_skip_permissions: true|dangerously-bypass|dangerously-skip" crates/roko-cli/src crates/roko-serve/src crates/roko-agent/src

# Runner reducer/event loop must not own filesystem, dream, or threshold side effects.
rg "std::fs::|tokio::fs::|append_jsonl|atomic_write|DreamRunner::new|update_gate_thresholds" crates/roko-cli/src/runner/event_loop.rs

# Runtime-adjacent route work must go through the supervisor.
rg "tokio::spawn" crates/roko-serve/src/routes crates/roko-cli/src/runner -g '*.rs'

# Legacy orchestrate backreferences must be gone.
rg "super::orchestrate::|crate::orchestrate::|PlanRunner::from_plans_dir|pub use orchestrate" crates/roko-cli/src crates/roko-serve/src

# Raw storage terminology must not leak into UI/API surfaces.
rg "signals\\.jsonl|signals_jsonl|engrams_jsonl|events_jsonl|episodes_jsonl" crates/roko-cli/src/tui crates/roko-serve/src/routes crates/roko-serve/src/lib.rs

# Raw efficiency history must not be cloned into UI/API state.
rg "efficiency_events: Vec|efficiency_events\\(\\)\\.to_vec\\(\\)|\\.efficiency_events\\.clone\\(\\)" crates/roko-cli/src/tui crates/roko-serve/src
```

### Handoff Prompt For Another Agent

Use this as the implementation handoff if another agent has no prior context:

```text
You are implementing the side-effect ownership redesign from tmp/mori-diffs/30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md. Start with the 2026-04-27 deepening pass. Do not patch individual call sites ad hoc. First add the side-effect inventory generator and owner manifest. Then migrate P0 batches SE-02 through SE-06 in order. Every production side effect must have exactly one owner class and a proof gate. Do not mark a checklist complete unless grep gates and query/proof outputs show the effect flows through the owning service.
```
