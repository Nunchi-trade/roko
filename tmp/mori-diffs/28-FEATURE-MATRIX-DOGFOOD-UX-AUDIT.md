# 28 - Feature Matrix, Dogfood, and UX Runtime Audit

Date: 2026-04-26

Scope:

- `tmp/FEATURE-MATRIX.md`
- `tmp/dogfood/00-INDEX.md`
- `tmp/ux/ux-followup/00-INDEX.md` and referenced open UX catalog files
- Current code paths in `crates/roko-cli`, `crates/roko-orchestrator`, `crates/roko-gate`, `crates/roko-learn`, `crates/roko-neuro`, `crates/roko-compose`, `crates/roko-agent`, and `crates/roko-serve`

Bottom line:

- `tmp/FEATURE-MATRIX.md` is useful as an inventory, but it is not a reliable implementation-status source right now.
- The active `roko plan run` path is runner v2 for approval and non-approval.
- Runner v2 is not Mori-complete. It is still a narrow implementation runner with major orchestration gaps.
- The largest discovered correction is gate execution: runner v2 now advances through the configured rungs on the current compile/clippy/test path, but it still does not execute the advertised full 7-rung gate ladder end to end.
- `roko-serve` currently compiles as a library; the feature matrix's compile-broken claim is stale.

Verification commands run:

```bash
cargo check -p roko-serve --lib
rg -n "PlanRunner::from_plans_dir|efficiency_events|AdaptiveThresholds::|gate_thresholds|save_cascade_router|record_completed_run" crates/roko-cli/src crates/roko-learn/src crates/roko-gate/src -g '*.rs'
rg -n "120_000|Duration::from_secs\\(120\\)|timeout_ms.*120" crates/roko-compose crates/roko-gate crates/roko-cli/src/runner crates/roko-agent/src/provider crates/roko-agent/src/openai_compat_backend.rs crates/roko-agent/src/claude_agent.rs crates/roko-agent/src/codex_agent.rs
rg -n "route\\(.*knowledge|/knowledge|neuro/query|api/knowledge|knowledge" crates/roko-serve/src crates/roko-cli/src
rg -n "signals_jsonl|signals\\.jsonl|engrams\\.jsonl|FileSubstrate|Engram" crates/roko-cli/src crates/roko-fs/src crates/roko-core/src crates/roko-serve/src -g '*.rs'
rg -n "replan|Replan|GateCompletion|retry|format_gate_feedback" crates/roko-cli/src/runner crates/roko-cli/src/orchestrate.rs
rg -n "vcg_allocate|CognitiveWorkspace|ExtensionChain|run_pre_inference|run_on_gate|/Users/will|/tmp/|tmp/" crates -g '*.rs'
rg -n "max_gate_rung|spawn_gate\\(|run_rung\\(|RunGate|GatePassed|GateFailed" crates/roko-cli/src/runner crates/roko-orchestrator/src crates/roko-gate/src/rung_dispatch.rs -g '*.rs'
```

Compile proof:

- `cargo check -p roko-serve --lib` completed successfully without warnings.
- The stale unused import warnings in `crates/roko-serve/src/routes/status/metrics.rs` and `crates/roko-serve/src/routes/shared_runs.rs` were removed.
- Therefore the matrix line "roko-serve has compile error (missing DreamCycleReport fields)" is stale.
- `cargo check -p roko-cli --lib` completed successfully after the small-batch runtime fixes, with 76 remaining dead-code/style warnings in unfinished CLI modules.
- `cargo check -p roko-core --lib` completed successfully.
- `cargo test -p roko-core dashboard_snapshot::tests::agent_spawned_empty_model_records_validation_warning` passed.

Small implementation batch completed on 2026-04-26:

- [x] Removed the stale `roko-serve` metrics unused import.
- [x] Removed stale `roko-serve` shared-run route unused imports.
- [x] Made one-shot `roko run` emit a non-empty dashboard model label.
- [x] Made `/api/run` dashboard spawn events use the selected agent label as the model fallback instead of an empty model.
- [x] Made server event bridge `AgentSpawned`/`AgentStarted` dashboard conversions use a non-empty model fallback.
- [x] Added a dashboard reducer validation warning for any remaining empty `AgentSpawned.model` event.
- [x] Changed CascadeRouter startup candidates to come from effective model config.
- [x] Made `RunConfig.max_gate_rung` drive the current compile/clippy/test gate path.
- [x] Fixed runner-v2 Daimon policy routing so the loaded affect state is passed into CascadeRouter selection.
- [x] Removed the first batch of actionable `cargo check -p roko-cli --lib` unused warnings in runtime-feedback routing, inline primitives, merge, and projection code.

Source evidence index:

- `crates/roko-cli/src/commands/plan.rs:171` says runner v2 is used for all plan run modes.
- `crates/roko-cli/src/commands/plan.rs:305` calls `runner::event_loop::run(...)`.
- `crates/roko-cli/src/commands/plan.rs:207` to `crates/roko-cli/src/commands/plan.rs:215` initializes CascadeRouter from `.roko/learn/cascade-router.json`.
- `crates/roko-cli/src/commands/plan.rs` now initializes router candidates from `Config::effective_models()` and falls back to the default model only if no effective models are configured.
- `crates/roko-cli/src/commands/plan.rs:240` sets `dangerously_skip_permissions: true`.
- `crates/roko-cli/src/runner/event_loop.rs:935` to `crates/roko-cli/src/runner/event_loop.rs:986` selects the model through task hint, CascadeRouter, or config default.
- `crates/roko-cli/src/runner/event_loop.rs:966` and `crates/roko-cli/src/runner/event_loop.rs:1945` use `DaimonPolicy::default()`.
- `crates/roko-cli/src/runner/event_loop.rs:530` to `crates/roko-cli/src/runner/event_loop.rs:635` handles failed gates as retry or fatal, not replan.
- `crates/roko-cli/src/runner/event_loop.rs:1854` to `crates/roko-cli/src/runner/event_loop.rs:1913` writes episodes/efficiency, ingests knowledge, observes CascadeRouter, and records bandit feedback.
- `crates/roko-cli/src/runner/event_loop.rs:2132` to `crates/roko-cli/src/runner/event_loop.rs:2155` persists CascadeRouter on shutdown.
- `crates/roko-cli/src/runner/gate_dispatch.rs:55` calls `run_rung(...)` with default `RungExecutionInputs`.
- `crates/roko-gate/src/rung_dispatch.rs:145` to `crates/roko-gate/src/rung_dispatch.rs:223` returns stub-pass verdicts for several advanced rungs when inputs/oracles are missing.
- `crates/roko-orchestrator/src/executor/state_machine.rs:126` to `crates/roko-orchestrator/src/executor/state_machine.rs:128` transitions `Gating + GatePassed` directly to `Verifying`.
- `crates/roko-orchestrator/src/executor/state_machine.rs:241` to `crates/roko-orchestrator/src/executor/state_machine.rs:244` computes gate rung as current `gate_results.len()`.
- `crates/roko-cli/src/runner/types.rs:1160` defines `max_gate_rung`; runner v2 now uses it to continue the current gate path through configured rungs.
- `crates/roko-cli/src/runner/tui_bridge.rs:74` to `crates/roko-cli/src/runner/tui_bridge.rs:79` forwards model into `AgentSpawned`.
- `crates/roko-cli/src/run.rs` now publishes a non-empty resolved model/command for one-shot `AgentSpawned` events.
- `crates/roko-core/src/dashboard_snapshot.rs` records a `validation_warning` event when an `AgentSpawned` event has an empty model.
- `crates/roko-serve/src/lib.rs` now normalizes server-to-dashboard agent model labels with a non-empty fallback.
- `crates/roko-cli/src/runner/event_loop.rs` now loads `.roko/daimon/affect.json` and passes the resulting `DaimonPolicy` into CascadeRouter model selection.
- `crates/roko-cli/src/prd.rs:733` to `crates/roko-cli/src/prd.rs:736` and `crates/roko-cli/src/worker/cloud.rs:460` to `crates/roko-cli/src/worker/cloud.rs:464` still call legacy `PlanRunner::from_plans_dir`.
- `crates/roko-neuro/src/lifecycle.rs:277` to `crates/roko-neuro/src/lifecycle.rs:279` ingests runner episodes.
- `crates/roko-neuro/src/knowledge_store.rs:440` to `crates/roko-neuro/src/knowledge_store.rs:442` creates the knowledge directory before write.
- `crates/roko-serve/src/routes/neuro.rs:15` to `crates/roko-serve/src/routes/neuro.rs:19` registers `/neuro/query` and `/knowledge`.
- `crates/roko-compose/src/strategy.rs:46` to `crates/roko-compose/src/strategy.rs:57` keeps Auto composition on density-greedy until bidder observations are warm.
- `crates/roko-compose/src/prompt.rs:1214` calls `vcg_allocate(...)` when VCG is selected.

## 1. User-Pasted Runtime Bugs

### #9 - Enrichment timeout hardcode

Status: partially open, not proven fixed.

Evidence:

- Runner v2 gate dispatch accepts a `timeout_secs` parameter and wraps gates with `tokio::time::timeout`, so the runner gate timeout is not the old fixed 120s path.
- Provider and gate defaults still contain multiple 120s defaults: `roko-agent/src/provider/mod.rs`, `roko-agent/src/provider/anthropic_api/tool_loop.rs`, `roko-agent/src/provider/openai_compat.rs`, `roko-agent/src/claude_agent.rs`, `roko-agent/src/codex_agent.rs`, `roko-gate/src/integration_gate.rs`.
- I did not find a direct runner-v2 gate-judge call hardcoding 120s. I did find `run_llm_judge_gate` using `RungExecutionConfig.llm_judge_min_score` but not a runner-wired judge payload.

Implementation checklist:

- [ ] Introduce a single runtime timeout policy object used by agent dispatch, enrichment, gate rungs, judge calls, and provider adapters.
- [ ] Replace provider-level `unwrap_or(120_000)` defaults with config-derived values or named constants owned by that policy.
- [ ] Add a proof script that sets a low timeout in `roko.toml`, runs a gate/judge path, and proves the configured value is used.
- [ ] Add a grep proof that no anonymous `120_000` timeout remains outside tests or named default constants.

### M1 - No streaming in non-approval path

Status: implemented for `roko plan run`, not universal for all commands.

Evidence:

- `PlanCmd::Run` always calls `runner::event_loop::run()` after dry-run handling.
- The approval branch only starts an approval TUI; it does not choose a different executor.
- Runner v2 uses the streaming agent path for CLI protocol dispatch.

Remaining work:

- [ ] Prove `roko plan run` non-approval with Claude, Codex, Anthropic API, OpenAI-compatible, Moonshot, Zai, and Perplexity all stream events into `.roko/events.jsonl` and `StateHub`.
- [ ] Move one-shot `roko run` onto the same runner/event stream contract or explicitly document it as a separate path.

### M2 - Model shows "-" in TUI

Status: fixed in code for runner v2 `plan run` and one-shot `roko run`; event-log proof still needed.

Evidence:

- Runner v2 forwards the selected model to `tui.model_selected()` and `tui.agent_spawned()`.
- Bridge dispatch forwards `provider_id:model`.
- One-shot `roko run` now uses `dashboard_agent_model(config)` and falls back to the configured command when no model is explicit.
- The shared dashboard snapshot reducer now records a validation warning if any remaining path emits an empty model.

Implementation checklist:

- [x] Change one-shot `roko run` to publish the resolved provider/model string, not `String::new()`.
- [ ] Add a snapshot or event-log proof that `AgentSpawned.model` is non-empty for both `roko run` and `roko plan run`.
- [x] Make empty model a validation warning in the TUI event reducer.

### F5 - Memory leak in `efficiency_events: Vec`

Status: fixed for runner v2 plan execution by avoiding the legacy in-memory accumulator, but not eliminated repository-wide.

Evidence:

- Legacy `PlanRunner` still has `efficiency_events: Vec<AgentEfficiencyEvent>`.
- Legacy `PlanRunner` still pushes into that vector in success and failure efficiency emission.
- Legacy `PlanRunner` is still reachable from PRD auto-plan execution and cloud worker execution.

Implementation checklist:

- [ ] Remove or bound `PlanRunner.efficiency_events`.
- [ ] Port PRD auto-plan execution from `PlanRunner::from_plans_dir` to runner v2.
- [ ] Port cloud worker execution from `PlanRunner::from_plans_dir` to runner v2.
- [ ] Add a long-run proof that RSS does not grow with appended efficiency events.

### #12 - Knowledge endpoint URL mismatch

Status: HTTP alias implemented.

Evidence:

- `roko-serve/src/routes/neuro.rs` registers `POST /api/neuro/query` and `GET /api/knowledge`.
- The knowledge alias queries the same `KnowledgeStore` and returns `{ results, total }`.

Remaining work:

- [ ] Add an HTTP proof script that seeds `.roko/neuro/knowledge.jsonl`, starts `roko serve`, and verifies both `/api/neuro/query` and `/api/knowledge?q=...`.
- [ ] Update CLI/TUI docs so users know whether `/api/knowledge`, `/api/knowledge/search`, or `/api/neuro/query` is canonical.

### #15 - Enrichment artifacts empty

Status: open.

Evidence:

- The matrix and dogfood docs treat this as moot under `skip_enrichment`, but that does not prove artifacts are populated when enrichment is enabled.
- I did not find an end-to-end proof tying enrichment execution to non-empty artifacts consumed by runner v2.

Implementation checklist:

- [ ] Define the enrichment artifact schema and canonical path.
- [ ] Make runner v2 write an artifact receipt for skipped, successful, and failed enrichment.
- [ ] Make prompt assembly consume the receipt or explicitly log why it was skipped.
- [ ] Add a proof script that runs a real enrichment path and asserts artifact count, non-empty content, and prompt consumption.

### S4 - `signals.jsonl` dead path

Status: open legacy/path compatibility issue.

Evidence:

- Canonical signal/engram storage is `.roko/engrams.jsonl` via `FileSubstrate`.
- Runner v2 also writes `.roko/events.jsonl`.
- `RokoLayout` still exposes `.roko/signals.jsonl`.
- Some cleanup/archive/status paths still mention or fallback to `signals.jsonl`.

Implementation checklist:

- [ ] Decide whether `signals.jsonl` is a deprecated alias or should be removed.
- [ ] If deprecated, add a migration shim from `.roko/signals.jsonl` to `.roko/engrams.jsonl`.
- [ ] Update serve parity metadata and CLI status docs to name `.roko/engrams.jsonl` and `.roko/events.jsonl`.
- [ ] Add a proof that conductor, webhooks, status routes, TUI, and retention all read the same canonical paths.

### S7 - `learn/` files stale in runner v2

Status: partially fixed.

Evidence:

- CascadeRouter is now selected, observed, and persisted in runner v2.
- `LearningRuntime::record_completed_run` also persists cascade-router updates immediately.
- Adaptive gate thresholds still have no runner-v2 update path.

Implementation checklist:

- [x] Remove stale docs that say runner v2 never updates `cascade-router.json`.
- [ ] Wire `AdaptiveThresholds` into runner v2 gate completion.
- [ ] Persist `.roko/learn/gate-thresholds.json` after every configured batch or every gate completion.
- [ ] Add a proof that both `cascade-router.json` and `gate-thresholds.json` change after a real plan run.

## 2. Runner V2 Completion

### Phase C - Make runner v2 default for all `plan run`

Status: implemented for `PlanCmd::Run`.

Evidence:

- `commands/plan.rs` handles dry-run, then enters runner v2 setup and calls `runner::event_loop::run()`.
- The `approval` flag only starts the approval TUI.

Remaining caveat:

- Legacy `PlanRunner` is still used outside `plan run`: PRD auto-plan execution and cloud worker execution.

Checklist:

- [ ] Port `prd.rs::run_generated_plans()` to runner v2.
- [ ] Port `worker/cloud.rs` execution to runner v2.
- [ ] Add a code-search CI guard blocking new `PlanRunner::from_plans_dir` call sites.

### Phase D - Deprecate `orchestrate.rs`

Status: not done.

Evidence:

- `orchestrate.rs` is still the large legacy runtime.
- `PlanRunner` is still exported and still used.
- Gate helpers and knowledge helpers still depend on legacy helper functions.

Checklist:

- [ ] Rename `orchestrate.rs` to `orchestrate_legacy.rs` only after active call sites are removed.
- [ ] Move reusable helpers into small modules owned by runner v2 or shared crates.
- [ ] Delete or quarantine legacy-only tests that keep the old runtime alive as the apparent source of truth.

### Phase E - Align with unified spec

Status: partial.

Evidence:

- Runner v2 emits structured `RunnerEvent`s and publishes `DashboardEvent`s through `StateHub`.
- Unified type renames and a single `Activity` recording contract are not enforced across CLI, serve, and TUI.
- There are still mixed terms: signal, engram, event, episode, activity.

Checklist:

- [ ] Define one canonical activity/event schema for runner, server, and TUI.
- [ ] Add conversion adapters for legacy `Engram`, `DashboardEvent`, `RunnerEvent`, and `Episode`.
- [ ] Make `.roko/events.jsonl` replay reconstruct the same state as live `StateHub`.

## 3. Critical Runner V2 Gaps Found While Auditing

### Gate ladder is not fully executed

Status: partially fixed and still high impact.

Evidence:

- `roko-gate::rung_dispatch::run_rung` supports rungs 0-6.
- Runner v2 dispatches `RunGate { rung }`, but `ParallelExecutor` computes rung from `plan_state.gate_results.len()`.
- State machine transitions `Gating + GatePassed -> Verifying`, so runner v2 now withholds `GatePassed` until the configured rung limit is reached.
- Runner v2 now records skipped clippy/test rungs when disabled and only leaves the task gate phase after `rung >= max_gate_rung`.
- `RunConfig.max_gate_rung` is now referenced by the runner event loop.
- Result: active runner v2 now handles the configured compile/clippy/test path better, but it still does not run compile -> clippy -> test -> generated/property/judge/integration as a complete real-input ladder.

Implementation checklist:

- [ ] Move gate-ladder state into runner v2, not implicit `gate_results.len()`.
- [x] Make `max_gate_rung` authoritative for the current compile/clippy/test gate path.
- [x] On gate pass, advance to the next rung until the configured max rung is reached for the current gate path.
- [ ] On gate failure, classify retry/replan/block/human and persist the failed rung.
- [ ] Treat plan-level verify as separate from per-task gate ladder.
- [ ] Wire richer `RungExecutionInputs` for rungs 3-6 so they do not return stub-pass verdicts.
- [ ] Add an end-to-end proof where rungs 0, 1, and 2 all execute and are visible in `.roko/events.jsonl`, `.roko/episodes.jsonl`, and the TUI projection.

### Adaptive gate thresholds are still not wired

Status: open.

Evidence:

- `AdaptiveThresholds` exists in `roko-gate`.
- `LearningPaths` reserves `.roko/learn/gate-thresholds.json`.
- Runner v2 `gate_dispatch.rs` does not load, update, or save `AdaptiveThresholds`.
- `runtime_feedback.rs` labels `gate_thresholds_every_n` as reserved for orchestrator-managed batching.

Checklist:

- [ ] Add an adaptive-threshold subsystem to `RunConfig`.
- [ ] Load `.roko/learn/gate-thresholds.json` at runner startup.
- [ ] On every `GateCompletion`, call the correct threshold update method for rung/verdict/outcome.
- [ ] Save threshold state on update and shutdown.
- [ ] Include threshold value and policy decision in `RunnerEvent::GateCompleted`.
- [ ] Add a proof that repeated pass/fail outcomes alter `threshold_for(rung)`.

### Replan-on-gate-failure is not wired in runner v2

Status: open.

Evidence:

- Legacy `orchestrate.rs` has replan ledger, strategy selection, and plan mutation code.
- Runner v2 failure handling only retries after backoff or marks a fatal terminal state.
- Runner v2 classifies `NeedsReplan` into a structural failure kind but does not generate or apply a revised plan.

Checklist:

- [ ] Port the replan ledger from legacy runtime into runner v2 or `roko-orchestrator`.
- [ ] Add a `RetryAction::Replan` path separate from `RetryAfterBackoff`.
- [ ] Generate a revised task or plan patch from gate failure context.
- [ ] Persist the replan request, model, prompt, result, and applied DAG mutation.
- [ ] Add dedupe and max-replans-per-plan caps.
- [ ] Add a proof that a structural gate failure creates `.roko/learn/replans.json`, mutates the DAG, and resumes from the revised task.

### Safety is not universal

Status: open.

Evidence:

- `commands/plan.rs` sets `dangerously_skip_permissions: true` in `RunConfig`.
- CLI dispatch passes the flag into Claude CLI as `--dangerously-skip-permissions`.
- Codex dispatch maps it to `--dangerously-bypass-approvals-and-sandbox`.
- `roko-agent` has a `SafetyLayer`, role tools, hook chain, and provider scoped safety, but runner v2's default plan path explicitly bypasses CLI-level approval/sandbox protections.

Checklist:

- [ ] Make dangerous permission bypass opt-in from CLI/config, not runner default.
- [ ] Create a provider-neutral safety contract passed into every dispatch path.
- [ ] Enforce role-local tool allowlists for Claude CLI, Codex CLI, API tool loops, and ExecAgent.
- [ ] Emit safety audit records into `.roko/events.jsonl`.
- [ ] Add proof scripts for denied file path, denied shell command, denied network call, secret-scrubbed output, and rate limit.

## 4. `tmp/FEATURE-MATRIX.md` Corrections

### Build status

Feature matrix says: `roko-serve` compile error.

Current audit says: stale. `cargo check -p roko-serve --lib` passed without warnings after removing stale unused imports.

Checklist:

- [x] Replace the build-status line with the current command result.
- [x] Add date, command, and warning details.

### Core execution loop

Feature matrix says: DAG execution, agent dispatch, gates, and resume are wired.

Current audit says: partially true.

Details:

- Plan DAG execution exists.
- Agent dispatch exists.
- Resume exists.
- Gate execution is materially overclaimed because the full 7-rung gate ladder is not implemented end to end in runner v2.

Checklist:

- [ ] Change "Gate Pipeline: WIRED" to "PARTIAL".
- [x] Document that runner v2 currently executes a narrow gate path and now uses `max_gate_rung` for that path.
- [ ] Add a proof command that shows all intended rungs execute before marking the gate pipeline wired.

### CascadeRouter

Feature matrix says: closed loop.

Current audit says: mostly wired but not provider-proven.

Details:

- Runner v2 selects through `CascadeRouter`.
- Runner v2 observes gate outcome, cost, and latency.
- Runner v2 saves `.roko/learn/cascade-router.json` on shutdown.
- The startup candidate list now comes from effective provider/model config, but there is no end-to-end provider proof across all configured providers.

Checklist:

- [x] Build router candidates from resolved provider/model config, not two hardcoded slugs.
- [ ] Record provider id, model slug, role, task category, complexity, and failure history.
- [ ] Add proof that Anthropic, Claude CLI, Codex, OpenAI, Moonshot, Zai, and Perplexity candidates can all be routed when configured.

### Episode logging and efficiency events

Feature matrix says: wired.

Current audit says: wired but duplicated/split across paths.

Details:

- Runner v2 appends `.roko/episodes.jsonl`.
- `LearningRuntime` also appends an episode under `.roko/learn/episodes.jsonl`.
- Runner v2 appends `.roko/learn/efficiency.jsonl`.

Checklist:

- [ ] Decide whether root episodes and learn episodes are both canonical.
- [ ] If both stay, document their different consumers and schemas.
- [ ] If not, collapse to one episode log plus derived learning indexes.

### Section effectiveness and prompt learning

Feature matrix says: closed loop.

Current audit says: closed loop exists in legacy `PlanRunner`; runner v2 needs a direct proof.

Evidence:

- Legacy orchestrate builds prompts with `section_effectiveness_snapshot()` and records prompt sections into efficiency events.
- `LearningRuntime::append_efficiency_event` updates a `SectionEffectivenessRegistry`.
- Runner v2's prompt assembly path needs an explicit proof that it records prompt section metadata and consumes learned section effectiveness on subsequent tasks.

Checklist:

- [ ] Prove runner v2 emits non-empty `prompt_sections` in efficiency events.
- [ ] Prove runner v2 reads section effectiveness before composing the next prompt.
- [ ] Add before/after prompt-manifest evidence where a learned section changes priority/tokens.

### Knowledge/Neuro store

Feature matrix says: not wired because `.roko/neuro/` is never created.

Current audit says: stale/partially false.

Evidence:

- Runner v2 calls `RuntimeKnowledgeLifecycle::for_workdir(workdir).ingest_episode(&episode)`.
- `KnowledgeStore::ingest` creates the parent directory before writing.
- The remaining gap is not initialization; it is query/injection and proof.

Checklist:

- [ ] Update matrix to say "write path wired, query/injection path partial".
- [ ] Prove `.roko/neuro/knowledge.jsonl` receives entries after a successful runner v2 episode.
- [ ] Wire knowledge retrieval into runner v2 prompt assembly.
- [ ] Add HTTP and CLI proof for querying the same knowledge entries.

### Daimon/Affect

Feature matrix says: partial/default.

Current audit says: partially improved, still not a closed loop.

Evidence:

- Runner v2 loads `.roko/daimon/affect.json` through `DaimonState::load_or_new`.
- Runner v2 passes the loaded `DaimonPolicy` into CascadeRouter routing context for model selection.
- Runner v2 still uses `DaimonPolicy::default()` in the post-run router observation path.
- Prompt assembly, episode metadata, and affect-delta persistence are still not wired.

Checklist:

- [x] Load affect state at runner startup.
- [x] Include affect state in model-routing context.
- [ ] Include affect state in prompt assembly and episode metadata.
- [ ] Persist affect deltas after task/gate outcomes.
- [ ] Add proof that changing affect state changes route or prompt metadata.

### Dreams/consolidation

Feature matrix says: manual works, automatic trigger not wired.

Current audit agrees.

Checklist:

- [ ] Define trigger policy: after plan complete, after N episodes, idle timer, or explicit CLI.
- [ ] Run dream consolidation after a successful plan when policy allows.
- [ ] Emit dream lifecycle events into `.roko/events.jsonl`.
- [ ] Add proof that episodes become knowledge/playbook/routing recommendations after dream run.

### HTTP control plane

Feature matrix says: broken compile.

Current audit says: stale. Library compile passes.

Remaining checklist:

- [ ] Run targeted route tests for providers, knowledge, projections, learning, and run endpoints.
- [ ] Add a single proof script that starts `roko serve`, runs a tiny plan, and queries the HTTP projections.
- [ ] Verify the UI screenshot data paths against real endpoint responses.

## 5. UX Follow-up Catalog Status

The UX catalog says 112 entries total, 72 done, 40 open. That count is plausible as a catalog count, but several individual statuses are stale.

High-value items:

- CognitiveWorkspace VCG auction: partial. `vcg_allocate` is called by prompt composition, but `CompositionStrategy::Auto` falls back to density-greedy until all active bidders have enough observations. This is not necessarily wrong, but it means VCG is not the default cold-start behavior.
- ExtensionChain: partial. Runner v2 constructs an empty `ExtensionChain` and calls pre/post/gate/error/shutdown hooks, but no real extensions are registered by default. Legacy orchestrate also constructs an empty chain.
- Hardcoded paths: open. There are live examples under source and tests, including absolute `/Users/will/...` prompt references and many `/tmp/...` test paths. Some are harmless tests, but the current catalog item should not be closed without classification.
- TUI event parity: partial. The TUI subscribes to `StateHub` and uses notify-based `.roko` watching, but many dashboard fields are still loaded from files. This is acceptable for persisted projections, but not equivalent to a fully live Mori UI.

UX implementation checklist:

- [ ] Re-audit each of the 40 open UX entries against current code.
- [ ] Split "file-backed persisted projection" from "bad polling fallback" so file reads are not treated as bugs by default.
- [ ] For each open catalog item, add one proof command and one source-of-truth path.
- [ ] Move stale DONE claims that are false back to unchecked items.
- [ ] Add a generated index that cannot be hand-edited out of sync with the underlying checklist files.

## 6. Provider and Model End-to-End Status

Status: not proven.

Evidence:

- Provider adapters exist for multiple runtimes.
- Runner v2 resolves provider/model dispatch through `ProviderDispatchResolver`.
- The active router candidate list is now built from effective configured models, but all-provider execution has not been proven.
- No audit proof was found that all configured user providers/models run through plan execution, stream events, pass safety, write episodes, update learning, and expose HTTP/TUI state.

Provider proof checklist:

- [ ] Add a temporary workspace proof script that accepts env vars for Anthropic, OpenAI, Moonshot, Zai, and Perplexity.
- [ ] For each provider/model, run one minimal plan task through `roko plan run`.
- [ ] Assert non-empty streamed output.
- [ ] Assert `.roko/events.jsonl` contains agent spawn, output, gate started, gate completed, and task completed events.
- [ ] Assert `.roko/episodes.jsonl` includes the resolved provider/model.
- [ ] Assert `.roko/learn/efficiency.jsonl` includes tokens/cost/model where supported.
- [ ] Assert `/api/projections/runtime`, `/api/knowledge`, `/api/learning/*`, and provider routes reflect the run.
- [ ] Mark unsupported models as explicit unsupported with errors, not silent fallback.

## 7. Definitive Open Items To Pass To Another Agent

Runtime architecture:

- [ ] Port PRD auto-plan execution from legacy `PlanRunner` to runner v2.
- [ ] Port cloud worker execution from legacy `PlanRunner` to runner v2.
- [ ] Remove or bound legacy `PlanRunner.efficiency_events`.
- [ ] Deprecate `orchestrate.rs` only after no active call sites remain.

Gate system:

- [ ] Extend runner v2 from the current configurable compile/clippy/test path to the full real-input gate ladder.
- [x] Make `RunConfig.max_gate_rung` used.
- [ ] Wire rungs 3-6 with real inputs or disable them explicitly instead of stub-passing.
- [ ] Wire adaptive gate thresholds into runner v2.
- [ ] Add per-gate pass/fail timeline data.

Replan/retry:

- [ ] Port replan-on-gate-failure from legacy runtime to runner v2.
- [ ] Persist replan ledger and plan mutations.
- [ ] Add proof for retry vs replan vs blocked vs human-needed paths.

Safety:

- [ ] Stop defaulting plan execution to dangerous permission bypass.
- [ ] Enforce role/tool safety across CLI and API providers.
- [ ] Emit safety audit events and query them from HTTP/TUI.

Learning:

- [x] Build CascadeRouter candidates from provider config.
- [ ] Prove runner v2 prompt-section effectiveness loop.
- [ ] Decide root episode vs learn episode canonical path.
- [ ] Wire knowledge query/injection into runner v2 prompts.
- [ ] Add automatic dreams/consolidation trigger.
- [ ] Persist Daimon affect deltas and propagate affect state beyond model routing.

Observability:

- [ ] Normalize signals/engrams/events/activity terminology.
- [ ] Retire or migrate `signals.jsonl`.
- [ ] Add endpoint-level proofs for runtime, learning, knowledge, gate, provider, and projection data.
- [ ] Add UI proof scripts that compare HTTP projection data with TUI/StateHub data.

Docs:

- [x] Add a top-level audit warning to `tmp/FEATURE-MATRIX.md` so future agents do not treat stale claims as authoritative.
- [ ] Fully reconcile `tmp/FEATURE-MATRIX.md` section by section so the body matches the 2026-04-26 source audit.
- [ ] Recompute `tmp/ux/ux-followup/00-INDEX.md` from source checklist files.
- [ ] Replace hand-maintained DONE counts with a generated index.

## 8. Current Self-Assessment

Initial rating: 9.1 / 10.

Why not 9.8:

- I verified the highest-risk claims against source code and a compile check, but I did not execute a full temporary-workspace plan run across all real providers in this pass.
- I did not individually re-open and re-grade all 40 UX catalog entries, only the high-value categories and source code patterns behind them.
- I patched a small implementation batch, but the larger architecture items still need full end-to-end proof across real providers and HTTP/TUI surfaces.

What would make this 9.8:

- [ ] Add a generated proof script suite under `scripts/proof/feature-matrix/`.
- [ ] Run a temp-workspace `roko plan run` proof.
- [ ] Run all configured provider proofs with real keys.
- [ ] Generate a machine-readable status JSON from source searches and proof results.
- [ ] Patch `tmp/FEATURE-MATRIX.md` and `tmp/ux/ux-followup/00-INDEX.md` with the verified current status.
