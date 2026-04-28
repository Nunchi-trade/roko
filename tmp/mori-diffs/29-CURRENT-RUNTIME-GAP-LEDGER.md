# 29 - Current Runtime Gap Ledger

Date: 2026-04-27

Purpose: this is the canonical aggregation layer for the current Roko runtime gap state. It reconciles the older design docs, active `mori-diffs` checklists, the feature matrix audit, and the current working-tree source shape into one prioritized implementation ledger.

If another agent only reads one file before choosing work, read this file first.

## How To Use This Ledger

Read order:

1. [README.md](README.md) for the package map.
2. This file for priority, impact, current truth, and stale-doc corrections.
3. [30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md](30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md) for architecture that is inelegant or side-effect-heavy even if it partially works.
4. [31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md](31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md) for codebase-wide counts, hot files, and ownership checklists.
5. [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md) for crate graph direction, dependency inversions, and layer gates.
6. [33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md](33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md) for config, credentials, provider policy, unsafe defaults, and provider proof classification.
7. [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md) for runtime events, projections, HTTP query, TUI state, and proof surfaces.
8. [35-TASK-PROCESS-LIFECYCLE-AUDIT.md](35-TASK-PROCESS-LIFECYCLE-AUDIT.md) for background tasks, child processes, cancellation, shutdown, and operation status.
9. [36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md](36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md) for one-shot project workflows, CLI/HTTP entrypoint convergence, workflow artifacts, and orchestration-engine ownership.
10. [37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md](37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md) for workspace layout, repositories, typed artifacts, storage migration, and query-proof storage ownership.
11. [38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md](38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md) for learning, knowledge, dreams, affect, conductor, routing, prompt diagnostics, and two-run cognitive proof.
12. [39-RUNNER-EXECUTION-POLICY-AUDIT.md](39-RUNNER-EXECUTION-POLICY-AUDIT.md) for runner state-machine, scheduling, gate, retry/replan, merge, and execution-decision convergence.
13. [40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md](40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md) for HTTP server, TUI, operation store, projection, repository, and adapter convergence.
14. [41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md) for inference gateway, model-call service, provider proof, cache/cost/batch, and direct-call-site convergence.
15. [23-HANDOFF-OPEN-ITEMS.md](23-HANDOFF-OPEN-ITEMS.md) for subsystem checklists.
16. The source doc linked from the gap card you are implementing.
17. [21-FEATURE-PARITY-MATRIX.md](21-FEATURE-PARITY-MATRIX.md) before claiming Mori parity.

Completion rule:

- Do not archive a doc because a module exists.
- Archive only when the active runner path invokes the behavior and a proof command or artifact is recorded.
- If a claim is true only in `orchestrate.rs`, mark it `legacy-only`, not complete.
- If a claim is true only in a crate-level API, mark it `built-unproven`, not complete.
- If a claim is true in the runner but lacks end-to-end proof, mark it `wired-unproven`.

Status labels:

- `proven`: active runner path has source evidence plus proof artifact or command.
- `wired-unproven`: active runner path calls it, but no reproducible proof yet.
- `built-unrouted`: module/API exists, but active runtime does not rely on it yet.
- `legacy-only`: exists in `orchestrate.rs` or old callers only.
- `open`: not designed/implemented enough to be used.
- `stale-doc`: older docs make a claim that no longer matches current source.

Priority labels:

- `P0`: blocks runtime convergence or Mori parity.
- `P1`: high-impact feature gap after the runtime spine is correct.
- `P2`: hardening, observability, or cleanup needed before stability claims.
- `P3`: documentation/status hygiene or non-blocking cleanup.

## Current Truth Snapshot

What is materially better than the original broken state:

- `roko plan run` is intended to use the runner-v2 path rather than old `PlanRunner`.
- `crates/roko-cli/src/dispatch/` exists and exports dispatcher, model routing, prompt assembly, outcomes, and warm-pool seams.
- `crates/roko-agent/src/runtime_events.rs` exists and defines provider-neutral `AgentRuntimeEvent`.
- Claude stream parsing has been moved under `roko-agent/src/provider/claude_cli/stream.rs`.
- `crates/roko-cli/src/runtime_feedback/` exists with feedback facade and episode, routing, knowledge, conductor, and dream sinks.
- `crates/roko-cli/src/projection/` and `crates/roko-cli/src/runner/projection.rs` exist and define a normalized projection vocabulary.
- `crates/roko-cli/src/runner/merge.rs` exists with `PlanMerger`, `GitMergeBackend`, `MergeBackend`, `RegressionGate`, and conflict evidence.
- `crates/roko-cli/src/runner/task_dag.rs` exists.
- `crates/roko-cli/src/runner/persist.rs` defines `.roko/state/run-state.json`, `RunStateSnapshot`, and JSONL recovery helpers.
- `crates/roko-cli/src/runner/resume.rs` defines strict resume validation and JSONL recovery.

### Architecture Runner Completion (2026-04-28)

The arch runner (`tmp/runners/arch/`) completed 16 batches (P0A-P4B) via Codex gpt-5.5, implementing Phases 0-3 of the MASTER-IMPLEMENTATION-PLAN on branch `codex/arch-run-20260428-012508`. All batches passed structural + compilation + anti-pattern verification.

**New modules (all cargo-check verified):**

| Batch | Module | Crate | Purpose |
|-------|--------|-------|---------|
| P0A | `runtime_event.rs` | roko-core | RuntimeEvent enum + WorkflowOutcome |
| P0B | `foundation.rs` | roko-core | 6 foundation traits (ModelCaller, PromptAssembler, FeedbackSink, GateRunner, EventConsumer, EffectExecutor) |
| P0C | `event_bus.rs` (ext) | roko-runtime | runtime_event_bus() singleton + emit_runtime_event() |
| P1A | `model_call_service.rs` | roko-agent | ModelCallService implementing ModelCaller |
| P1B | `prompt_assembly_service.rs` | roko-compose | PromptAssemblyService implementing PromptAssembler |
| P1C | `feedback_service.rs` | roko-learn | FeedbackService implementing FeedbackSink |
| P1D | `gate_service.rs` | roko-gate | GateService implementing GateRunner |
| P2A | `pipeline_state.rs` | roko-runtime | PipelineStateV2 config-driven state machine (express/standard/full) |
| P2B | `task_scheduler.rs` | roko-runtime | TaskScheduler pure DAG scheduler |
| P2C | `effect_driver.rs` | roko-runtime | EffectDriver (executes PipelineOutput via foundation services) |
| P2D | `workflow_engine.rs` | roko-runtime | WorkflowEngine facade (unified entry point) |
| P3A | `acp_adapter.rs` | roko-acp | AcpAdapter implementing EventConsumer |
| P3B | `adapters.rs` | roko-serve | SseAdapter implementing EventConsumer |
| P3C | `jsonl_logger.rs` + `projection.rs` | roko-runtime | JsonlLogger + RuntimeProjection |
| P4A | `run.rs` (ext) | roko-cli | run_with_workflow_engine() added |
| P4B | `runner.rs` + `bridge_events.rs` (ext) | roko-acp | WorkflowEngine integration, removed Command::new("claude") |

**Impact on gap ledger:**
- P0-01 (Runtime convergence): `wired-unproven` -- infrastructure complete, needs proof runs
- P0-02 (Provider dispatch + events): `wired-unproven` -- foundation services created
- P0-04 (Prompt assembly): `wired-unproven` -- PromptAssemblyService created
- P0-05 (Feedback closed loop): `wired-unproven` -- FeedbackService created
- P0-06 (Persistence + resume): `wired-unproven` -- PipelineStateV2 + WorkflowEngine checkpoint logic
- P1-02 (Gates): infrastructure created via GateService
- P1-04 (Observability): infrastructure created via RuntimeProjection + SseAdapter + JsonlLogger

**What remains for `proven` status:**
- End-to-end proof runs (P.1-P.10 in MASTER-IMPLEMENTATION-PLAN)
- Live provider credential testing
- Crash/resume proof
- `orchestrate.rs` retirement (Phase 6)

What is still not proven enough to claim Mori parity:

- Full provider matrix with live Anthropic, OpenAI, Moonshot, Z.AI, Perplexity, Claude CLI, and Codex CLI credentials.
- Multi-task dependency plan through the active runner.
- Retry versus replan versus blocked versus human-needed proof.
- Crash/resume proof without duplicate completion.
- Knowledge retrieval influencing a later prompt.
- Dream trigger producing reusable routing/prompt advice on a later run.
- HTTP/query/UI proof over the same runtime projection state.
- Inference gateway proof showing runner, HTTP, provider probes, research, dreams, neuro, vision, and tool calls all use one `ModelCallService`.
- Full deprecation of `orchestrate.rs` as a production-critical shadow runtime.

## Priority Board

| Priority | Area | Impact | Current State | Primary Docs |
|---|---|---|---|---|
| P0 | Side-effect ownership firewall / generated inventory | Highest: without exclusive effect owners, every runtime feature can be wired twice or bypass proof | open | [30](30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md), [31](31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md), [32](32-DEPENDENCY-LAYERING-AUDIT.md), [34](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md), [35](35-TASK-PROCESS-LIFECYCLE-AUDIT.md), [40](40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md) |
| P0 | Runtime convergence / legacy retirement | Highest: prevents two runtimes from diverging | wired-unproven (arch runner infra complete, needs proof runs) | [20](20-RUNTIME-RECONCILIATION.md), [21](21-FEATURE-PARITY-MATRIX.md), [28](28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md) |
| P0 | Provider dispatch + runtime events | Highest: all models/providers depend on it | wired-unproven (ModelCallService + foundation traits created) | [01](01-AGENT-DISPATCH.md), [20](20-RUNTIME-RECONCILIATION.md) |
| P0 | Inference gateway / model-call service | Highest: every model call, provider proof, cache, cost, batch, and credential boundary depends on one service | wired-unproven (ModelCallService created by arch runner P1A) | [41](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md), [33](33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md), [34](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md), [40](40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md) |
| P0 | Config, secrets, provider policy | Highest: provider proof and safety depend on one resolved runtime context | open | [33](33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md), [32](32-DEPENDENCY-LAYERING-AUDIT.md) |
| P0 | Task/process lifecycle and cancellation spine | Highest: provider runs, serve tasks, daemon mode, cancellation, and crash recovery depend on one owner | partial (TaskScheduler + PipelineStateV2 created by arch runner P2A-P2B) | [35](35-TASK-PROCESS-LIFECYCLE-AUDIT.md), [34](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md), [32](32-DEPENDENCY-LAYERING-AUDIT.md) |
| P0 | Workflow entrypoint convergence | Highest: users need one durable idea-to-done path, not separate PRD/plan/task/run commands with different runtimes | partial (WorkflowEngine facade created by arch runner P2D) | [36](36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md), [20](20-RUNTIME-RECONCILIATION.md), [34](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md), [35](35-TASK-PROCESS-LIFECYCLE-AUDIT.md) |
| P0 | Workspace layout and artifact repository convergence | Highest: resume, workflow artifacts, HTTP/TUI query, proof, retention, and migration depend on one storage contract | open | [37](37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md), [34](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md), [36](36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md) |
| P0 | Runner execution policy convergence | Highest: task scheduling, gates, retries, replans, merge, and resume decisions still need one typed policy engine | partial (TaskScheduler + EffectDriver + GateService created by arch runner P2B-P2C, P1D) | [39](39-RUNNER-EXECUTION-POLICY-AUDIT.md), [02](02-PLAN-EXECUTION.md), [11](11-PARALLEL-MERGE.md), [14](archive/2026-04-26-verified/14-FAILURE-RETRY.md), [20](20-RUNTIME-RECONCILIATION.md) |
| P0 | Serve/TUI adapter convergence | Highest: HTTP and TUI still own command, query, operation, storage, git, and fallback runtime behavior | partial (SseAdapter + RuntimeProjection created by arch runner P3B-P3C) | [40](40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md), [34](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md), [35](35-TASK-PROCESS-LIFECYCLE-AUDIT.md), [36](36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md), [37](37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md), [39](39-RUNNER-EXECUTION-POLICY-AUDIT.md) |
| P0 | Full provider proof matrix | Highest: proves real end-to-end behavior, not mocks | open proof | [27](27-FILESYSTEM-RUNTIME-CI-AUDIT.md), [28](28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md) |
| P0 | Prompt assembly as one path | High: determines agent quality and safety | wired-unproven (PromptAssemblyService created by arch runner P1B) | [05](archive/2026-04-26-verified/05-PROMPT-ASSEMBLY.md), [09](09-COMPOSITION-AUCTION.md) |
| P0 | Feedback facade closed loop | High: makes runs improve over time | wired-unproven (FeedbackService created by arch runner P1C) | [04](04-LEARNING.md), [10](10-DREAMS-CONSOLIDATION.md), [13](13-KNOWLEDGE-LIFECYCLE.md) |
| P0 | Persistence + resume contract | High: required for reliability | wired-unproven (PipelineStateV2 checkpoint logic created by arch runner P2A) | [03](03-PERSISTENCE.md), [22](22-STABILITY-PLAN.md) |
| P1 | DAG, merge, worktree isolation | High: required for real plan execution | wired-unproven | [39](39-RUNNER-EXECUTION-POLICY-AUDIT.md), [02](02-PLAN-EXECUTION.md), [11](11-PARALLEL-MERGE.md) |
| P1 | Gate ladder, retry, replan | High: required for self-correction | partial (GateService created by arch runner P1D) | [39](39-RUNNER-EXECUTION-POLICY-AUDIT.md), [02](02-PLAN-EXECUTION.md), [14](archive/2026-04-26-verified/14-FAILURE-RETRY.md), [28](28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md) |
| P1 | Knowledge, playbooks, dreams, affect cognitive closed loop | High: required for Mori-like learning and self-improvement | partial | [38](38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md), [04](04-LEARNING.md), [10](10-DREAMS-CONSOLIDATION.md), [12](12-AFFECT-ROUTING.md), [13](13-KNOWLEDGE-LIFECYCLE.md) |
| P1 | HTTP/TUI/API observability and projection proof | High: required to inspect and prove runtime state | partial (RuntimeProjection + SseAdapter + JsonlLogger created by arch runner P3B-P3C) | [40](40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md), [06](archive/2026-04-26-verified/06-OBSERVABILITY.md), [27](27-FILESYSTEM-RUNTIME-CI-AUDIT.md), [28](28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md), [34](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md) |
| P1 | Shutdown, operation store, and process proof | High: required for cancellation/resume/orphan stability | open | [35](35-TASK-PROCESS-LIFECYCLE-AUDIT.md), [03](03-PERSISTENCE.md), [22](22-STABILITY-PLAN.md) |
| P1 | One-shot project workflow proof | High: proves `roko run` and HTTP workflow endpoints can move from idea to durable artifacts and execution | open | [36](36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md), [27](27-FILESYSTEM-RUNTIME-CI-AUDIT.md) |
| P1 | Storage migration and query proof | High: proves old workspaces, new workspaces, CLI, HTTP, and TUI agree on durable facts | open | [37](37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md), [27](27-FILESYSTEM-RUNTIME-CI-AUDIT.md) |
| P1 | Safety and extensions | High: required before trusting automated execution | partial | [15](archive/2026-04-26-verified/15-SAFETY-EXTENSIONS.md), [28](28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md) |
| P2 | Runtime schema / terminology | Medium: prevents observer drift | open | [16](16-INFRASTRUCTURE.md), [28](28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md) |
| P2 | Proof bundle/export tooling | Medium: makes claims reproducible | open | [27](27-FILESYSTEM-RUNTIME-CI-AUDIT.md) |
| P2 | Residual side-effect and dependency cleanup | Medium: remaining allowlist shrink after the P0 ownership firewall exists | open | [30](30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md), [31](31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md), [32](32-DEPENDENCY-LAYERING-AUDIT.md) |
| P3 | Stale status/doc cleanup | Medium: stops agents from redoing old work | open | [00](00-OVERVIEW.md), [23](23-HANDOFF-OPEN-ITEMS.md), [24](24-DEFINITIVE-GAP-LIST.md), [28](28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md) |

## P0 Gap Cards

### P0-00 Side-Effect Ownership Firewall And Generated Inventory

Impact:

- This is now the first runtime-convergence task, not cleanup.
- Without an ownership firewall, command handlers, route handlers, TUI state, runner hot paths, provider adapters, feedback sinks, and storage repositories can all keep writing private state directly.
- That makes proof ambiguous: a feature may "work" in one surface while bypassing the actual runtime spine.

Current state:

- [30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md](30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md) now defines the ownership firewall, effect classes, generated inventory schema, source anchors, migration batches, and strict grep gates.
- Fresh scan evidence in doc 30 found `1028` Rust files under `crates`, with `305` files containing at least one side-effect or ownership-smell pattern.
- Broad filesystem / append / atomic-write patterns produced `1428` matches across `228` files.
- `tokio::spawn` appears in `75` files.
- `RunConfig {`, dangerous permission defaults, direct dispatch, dream runner construction, orchestrate backreferences, raw efficiency vectors, and legacy `signals.jsonl` references all still need classification or migration.

Evidence to check:

```bash
find crates -name '*.rs' -print | wc -l
rg -l "std::fs::|tokio::fs::|append_jsonl|atomic_write|Command::new\\(|tokio::spawn|RunConfig \\{|dangerously_skip_permissions|dispatch_direct::dispatch_prompt|DreamRunner::new|PlanRunner::from_plans_dir" crates -g '*.rs' | wc -l
rg -n "dispatch_direct::dispatch_prompt|RunConfig \\{|dangerously_skip_permissions: true|DreamRunner::new|update_gate_thresholds|super::orchestrate::|PlanRunner::from_plans_dir|efficiency_events: Vec|signals\\.jsonl" crates -g '*.rs'
```

Done criteria:

- [ ] A side-effect inventory generator scans every `crates/**/*.rs` file.
- [ ] The generator emits effect records with path, line, symbol, pattern, effect class, current layer, correct owner, status, production/test classification, linked doc, and proof gate.
- [ ] Every production side effect has exactly one owner class.
- [ ] Unknown production owner count is zero.
- [ ] Owner manifest exists and distinguishes permanent owners from temporary migration violations.
- [ ] Command, route, TUI, and runner reducer code call command/query/effect services rather than owning runtime side effects directly.
- [ ] Strict grep gates from doc 30 pass or produce only allowlisted temporary violations.
- [ ] Query/proof output shows runtime policy, provider calls, prompt diagnostics, feedback transactions, gate decisions, merge outcomes, background tasks, and projection updates sharing stable ids.

### P0-01 Runtime Convergence And `orchestrate.rs` Retirement

Impact:

- This is the top architecture problem. As long as `orchestrate.rs` remains a production-critical shadow runtime, Roko has two sources of truth.
- New work can accidentally land in the old monolith, recreating the same tangled design Mori had.

Current state:

- `runner/` is the active target for plan execution.
- `crates/roko-cli/src/orchestrate.rs` is still exported from `lib.rs` and still contains large amounts of unique behavior.
- Helper modules still depend on `orchestrate.rs` for some functions.
- Some non-plan-run flows and docs still talk as if orchestration lives in `orchestrate.rs`.

Evidence to check:

```bash
rg -n "pub mod orchestrate|pub use orchestrate|PlanRunner::from_plans_dir|orchestrate::" crates/roko-cli/src crates/roko-serve/src -g '*.rs'
rg -n "auto-dream|replan|knowledge routing|format_bandit|worktree|MergeBranch|RunVerify" crates/roko-cli/src/orchestrate.rs
```

Done criteria:

- [ ] `roko plan run` and all one-shot/auto-plan/cloud run paths use the runner runtime or a shared runtime facade.
- [ ] No production-critical feature exists only in `orchestrate.rs`.
- [ ] `orchestrate.rs` has a freeze banner and shrinks or moves to `orchestrate_legacy.rs`.
- [ ] A code-search proof shows no new feature code depends on `PlanRunner`.
- [ ] [21-FEATURE-PARITY-MATRIX.md](21-FEATURE-PARITY-MATRIX.md) has proof links for every row before deletion/quarantine.

### P0-02 Provider Dispatch And Runtime Events

Impact:

- All providers and models depend on this seam.
- If the runner understands provider-specific wire formats, every provider creates another special case.

Current state:

- `crates/roko-cli/src/dispatch/` exists.
- `crates/roko-agent/src/runtime_events.rs` exists.
- `crates/roko-agent/src/provider/claude_cli/stream.rs` owns Claude stream parsing.
- Runner code imports `Dispatcher`, `PromptAssembler`, `ResolvedAgentRuntime`, `WarmPool`, and `resolve_agent_runtime`.
- Provider-neutral event usage exists, but full live-provider proof is still missing.

Evidence to check:

```bash
rg -n "AgentRuntimeEvent|Dispatcher|ResolvedAgentRuntime|resolve_agent_runtime|ClaudeStreamEvent" crates/roko-cli/src/runner crates/roko-agent/src
tests/proof/mori-diffs/prove-runtime-end-to-end.sh
```

Done criteria:

- [ ] Runner consumes provider-neutral events for CLI and API/result providers.
- [ ] Claude/Codex CLI and API providers share the same dispatch facade.
- [ ] Provider statuses are exactly `proved`, `missing_credentials`, `auth_failed`, `rate_limited`, or `unsupported`.
- [ ] Live proof covers Anthropic, OpenAI, Moonshot, Z.AI, Perplexity, Claude CLI, and Codex CLI.
- [ ] Unsupported means the provider genuinely cannot run through the active runtime, not that the proof script lacks a branch.

### P0-02A Inference Gateway And Model-Call Service

Impact:

- This is the missing runtime boundary above provider adapters.
- Without it, HTTP inference, provider tests, runner tasks, research, dreams, neuro distillation, vision, web search, and legacy orchestration can all "work" through different model-call paths.
- Provider proof, cache proof, cost proof, batch proof, credential isolation, and HTTP query proof cannot be trusted until they all pass through one `ModelCallService`.

Current state:

- `crates/roko-cli/src/dispatch/` is a useful runner-local facade, but it is not a process-wide inference gateway.
- `crates/roko-serve/src/routes/gateway.rs` exposes gateway-looking endpoints but calls `state.runtime.run_once`, stores volatile counters, hardcodes cache metrics to zero, and implements batch as local concurrent fan-out.
- `docker/gateway.Dockerfile` explicitly says the `roko-gateway` crate does not exist and ships the `roko` CLI binary as a placeholder.
- Research, dreams, vision, neuro distillation, web search, and provider-test routes still have direct provider/agent/secret paths.

Evidence to check:

```bash
rg -n "run_once\\(|cache_hits: u64 = 0|cache_read_tokens: u64 = 0|BATCH_CONCURRENCY|tokio::spawn" crates/roko-serve/src/routes/gateway.rs
rg -n "std::env::var\\(\\\"(ANTHROPIC|PERPLEXITY)_API_KEY\\\"\\)|create_agent_for_model|PerplexitySearchClient::new" crates/roko-neuro/src crates/roko-std/src crates/roko-cli/src/commands/research.rs crates/roko-dreams/src crates/roko-cli/src/vision_loop crates/roko-serve/src/routes/providers.rs
rg -n "placeholder|does not yet exist|--bin roko|roko-gateway crate" docker/gateway.Dockerfile docker/docker-compose.yml
```

Done criteria:

- [ ] A shared `ModelCallService` / `InferenceGateway` exists outside route handlers and CLI command handlers.
- [ ] Runner, HTTP inference, provider probes, research, dreams, neuro distillation, vision, and web search all use that service for model/provider calls.
- [ ] Every model call emits durable gateway events and is visible through gateway query endpoints.
- [ ] Provider matrix proof runs through `ModelCallService::probe_provider`.
- [ ] Cache, output budget, thinking cap, provider fallback, cost, and batch behavior are implemented or explicitly disabled with durable events.
- [ ] Docker/compose no longer advertise a placeholder gateway as a working service.
- [ ] [41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md) grep gates pass or each remaining hit is allowlisted.

### P0-03 Full Provider Proof Matrix

Impact:

- This is the difference between architecture that looks good and architecture that actually runs.
- Without this proof, provider support is only a claim.

Current state:

- Tracked proof harness exists at [tests/proof/mori-diffs/prove-runtime-end-to-end.sh](../../tests/proof/mori-diffs/prove-runtime-end-to-end.sh).
- Prior smoke proof classified API providers as `missing_credentials` when keys were absent.
- That is useful classification, not proof that those providers work with live credentials.

Command:

```bash
ROKO_PROOF_PROVIDERS=anthropic,openai,moonshot,zai,perplexity,claude,codex \
ROKO_PROOF_ARTIFACT_ROOT=/tmp/roko-provider-proof \
tests/proof/mori-diffs/prove-runtime-end-to-end.sh
```

Done criteria:

- [ ] Each configured provider either produces a real run artifact or a precise non-success status.
- [ ] Each successful provider writes events, episodes, efficiency data, prompt diagnostics, gate evidence, and provider/model labels.
- [ ] Provider failures include raw stderr/stdout evidence in the artifact directory without leaking secrets.
- [ ] Report JSON can be checked from a clean clone.

### P0-04 Prompt Assembly As One Runtime Path

Impact:

- Prompt quality controls task success, safety, and learning reuse.
- Scattered prompt construction makes knowledge/playbook/dream/affect injection unreliable.

Current state:

- `crates/roko-cli/src/dispatch/prompt_builder.rs` exists and defines `PromptAssembler`.
- It can collect neuro knowledge and episode knowledge.
- Runner calls `PromptAssembler::new()` through dispatch planning.
- Older docs that say only `build_minimal_system_prompt` exists are stale.
- End-to-end proof that prompt diagnostics are persisted and consumed by later learning is still open.

Evidence to check:

```bash
rg -n "PromptAssembler|PromptDiagnostics|knowledge_ids|playbook_ids|prompt.assembled|build_minimal_system_prompt" crates/roko-cli/src/dispatch crates/roko-cli/src/runner
```

Done criteria:

- [ ] Every production dispatch path uses `PromptAssembler` or a single equivalent prompt facade.
- [ ] Prompt diagnostics include included/dropped sections, knowledge ids, playbook ids, token budget, and allowlist decisions.
- [ ] Retry/replan prompts receive structured failure context, not raw gate text only.
- [ ] A two-run proof shows first-run learning changes second-run prompt assembly.

### P0-05 Feedback Facade Closed Loop

Impact:

- This is what makes Roko improve across runs.
- Without one feedback surface, learning, knowledge, dreams, conductor, and routing drift independently.

Current state:

- `crates/roko-cli/src/runtime_feedback/` exists.
- `FeedbackFacade`, `FeedbackEvent`, and sinks for episodes, routing, knowledge, conductor, and dreams exist.
- Runner has `feedback_facade` fields and translates some `RunnerEvent`s into `FeedbackEvent`s.
- Runner still also emits legacy/local feedback paths directly in `emit_feedback`, so the architecture is not fully collapsed into the facade.
- The current `runner_event_to_feedback` path fills some task-completed provider/model/token values with defaults when the event lacks enough detail.

Evidence to check:

```bash
rg -n "FeedbackFacade|FeedbackEvent|runner_event_to_feedback|emit_feedback|append_jsonl|record_runner_event|RuntimeKnowledgeLifecycle|observe_cascade_router" crates/roko-cli/src/runner crates/roko-cli/src/runtime_feedback
```

Done criteria:

- [ ] The active runner emits one authoritative feedback event stream.
- [ ] Episodes, routing observations, knowledge candidates, conductor observations, dream triggers, and efficiency events consume that stream.
- [ ] Provider/model/tokens/cost/latency are populated from actual dispatch data.
- [ ] A second run demonstrably consumes a routing/knowledge/prompt observation from the first run.
- [ ] Direct runner-local feedback writes are removed or reduced to compatibility mirrors.

### P0-06 Persistence And Resume Contract

Impact:

- Without durable run state and strict resume validation, long-running automated work cannot be trusted.

Current state:

- `PersistPaths` includes `run-state.json`, `cascade-router.json`, `gate-thresholds.json`, and event/episode logs.
- `RunStateSnapshot` exists.
- `resume.rs` provides strict validation and JSONL recovery helpers.
- Source evidence for definitions exists; end-to-end proof that the active runner writes and resumes from `run-state.json` without duplicate completion is still required.

Evidence to check:

```bash
rg -n "RunStateSnapshot|save_run_state|load_run_state|prepare_resume|recover_jsonl|run-state.json" crates/roko-cli/src/runner
```

Done criteria:

- [ ] Active runner writes `run-state.json` at safe checkpoints.
- [ ] Resume loads it and refuses stale task definitions.
- [ ] JSONL recovery runs before appending after crash.
- [ ] Crash proof covers active agent output, post-agent/pre-gate, in-gate, post-gate/pre-snapshot, stale pid files, stale plan ids, and corrupt optional learning state.

## P1 Gap Cards

### P1-01 DAG, Parallelism, Merge, And Worktree Isolation

Impact:

- Required for real multi-task plan execution.
- Required to prevent silent conflicts and fake merge success.

Current state:

- `runner/task_dag.rs` exists.
- `runner/merge.rs` exists with a real `GitMergeBackend` and regression gate abstraction.
- Runner handles `ExecutorAction::MergeBranch` through `PlanMerger`.
- `max_concurrent_tasks` is still effectively constrained until there is a per-plan/per-task agent-handle map and proof.
- Worktree-per-task behavior and touched-file discovery still need end-to-end runner proof.

Evidence to check:

```bash
rg -n "max_concurrent_tasks|agent_handle|TaskDag|PlanMerger|GitMergeBackend|MergeBranch|files_changed|worktree" crates/roko-cli/src/runner crates/roko-orchestrator/src
```

Done criteria:

- [ ] Multi-task dependency proof runs A -> B -> C in order.
- [ ] Independent tasks do not double-dispatch.
- [ ] Non-conflicting merges complete.
- [ ] Conflicting merges produce conflict evidence in events/projection/HTTP state.
- [ ] Post-merge regression failure does not become `MergeSucceeded`.

### P1-02 Gate Ladder, Retry, Replan, And Adaptive Thresholds

Impact:

- This controls self-repair and automated stabilization.

Current state:

- Current compile/clippy/test gate path is improved and uses `max_gate_rung`.
- Advanced gate rungs still need real inputs/proof.
- Runner has retry/backoff behavior and replan context, but not a fully proven replan ledger/DAG mutation path.
- `update_gate_thresholds` currently writes an EMA-style JSON structure directly; this is useful but not the same as proving the intended `AdaptiveThresholds` model is wired end to end.

Evidence to check:

```bash
rg -n "max_gate_rung|RunGate|GateFailed|RetryDecision|replan|gate-thresholds|AdaptiveThresholds|update_gate_thresholds" crates/roko-cli/src/runner crates/roko-gate/src
```

Done criteria:

- [ ] Proof shows rungs 0, 1, and 2 executing and appearing in events, episodes, and projection.
- [ ] Rungs 3-6 either run with real inputs or are explicitly disabled with visible reasons.
- [ ] Structural failure creates a replan record and mutates/resumes the DAG.
- [ ] Adaptive thresholds are loaded, updated, saved, and reflected in gate decisions.

### P1-03 Knowledge, Playbooks, Dreams, And Affect

Impact:

- This is the Mori-like "memory and consolidation" layer.
- It determines whether future runs get smarter.

Current state:

- Knowledge ingestion exists through runner feedback and neuro lifecycle paths.
- `PromptAssembler` can read neuro and episode knowledge.
- Dream trigger sink exists and runner has direct dream consolidation code, but the correct long-term path should be non-blocking and feedback-driven.
- Some dream code still hardcodes Claude timeout/model defaults.
- Affect is used in routing in parts of the runner, but prompt metadata and affect-delta persistence still need proof.

Evidence to check:

```bash
rg -n "RuntimeKnowledgeLifecycle|collect_neuro_knowledge|collect_episode_knowledge|DreamTriggerSink|DreamRunner|DaimonPolicy|affect|dream-routing-advice" crates/roko-cli/src crates/roko-dreams/src crates/roko-daimon/src
```

Done criteria:

- [ ] Successful runner completion writes durable knowledge that can be queried.
- [ ] A later prompt includes relevant knowledge/playbook/dream-derived guidance.
- [ ] Dream lifecycle emits start/skip/complete/fail events.
- [ ] Dream failures are non-fatal to the plan runner.
- [ ] Affect state is included in routing, prompt diagnostics, and episode metadata.

### P1-04 HTTP, TUI, And Queryable Observability

Impact:

- Users need to inspect runs without opening `.roko` files manually.
- Proof and UI should query the same source of truth.

Current state:

- Projection facade exists.
- Dashboard and CLI progress projections exist.
- Event JSONL exists.
- Queryable run-scoped HTTP proof is still incomplete.

Evidence to check:

```bash
rg -n "ProjectionEvent|ProjectionSubscriber|dashboard_snapshot|events.jsonl|/api/knowledge|/api/neuro/query|runtime|projection" crates/roko-cli/src crates/roko-serve/src
```

Done criteria:

- [ ] A proof script starts `roko serve`, runs a tiny plan, and queries runtime/gate/knowledge/learning/projection endpoints.
- [ ] HTTP projection data matches `.roko/events.jsonl` and TUI/StateHub data.
- [ ] Events are queryable by run id, plan id, task id, category, and gate rung.
- [ ] Provider runtime lifecycle, prompt diagnostics, merge backend result, retry decision, conflict evidence, and resume marker are all queryable.

### P1-05 Safety And Extensions

Impact:

- Automated agent execution needs role/tool/sandbox constraints to be trustworthy.
- Extensions are the intended way to avoid future hardcoded feature branches.

Current state:

- Extension chain loading exists in `RunConfig::from_roko_config`.
- Runner initializes extensions and fires pre/post/gate/error hooks.
- Older docs saying the chain is always empty are stale for configured extensions.
- Plan execution still defaults `dangerously_skip_permissions: true`, which must be treated as a blocker before safety claims.

Evidence to check:

```bash
rg -n "ExtensionChain|load_extensions|fire_pre_inference|fire_post_inference|fire_on_gate|fire_on_error|dangerously_skip_permissions|allowed_tools|SafetyLayer" crates/roko-cli/src crates/roko-agent/src
```

Done criteria:

- [ ] Dangerous permission bypass is opt-in, not default.
- [ ] Role-local tool allowlists apply across Claude CLI, Codex CLI, API tool loops, and ExecAgent.
- [ ] Extension hooks are proven in dispatch, gate, error, and shutdown paths.
- [ ] Safety denials emit durable audit events and are queryable through HTTP/TUI.

## P2/P3 Gap Cards

### P2-01 Runtime Schema And Terminology

Current issue:

- The code and docs still mix events, engrams, signals, activities, episodes, dashboard events, and projection events.

Done criteria:

- [ ] One canonical activity/event schema is documented for runner, server, and TUI.
- [ ] Legacy `Engram`, `DashboardEvent`, `RunnerEvent`, `Episode`, and `ProjectionEvent` conversions are explicit.
- [ ] `.roko/signals.jsonl`, `.roko/engrams.jsonl`, and `.roko/events.jsonl` have documented ownership and migration rules.

### P2-02 Proof Bundle / Clean Clone Verification

Current issue:

- Proof artifacts are currently scattered under `/tmp` unless the caller chooses an artifact root.

Done criteria:

- [ ] `roko export proof --run-id <id> --out <dir>` exports a secret-safe evidence bundle.
- [ ] `roko proof verify <dir>` validates the bundle without the original workspace.
- [ ] Proof scripts emit a machine-readable report and a human-readable summary.

### P3-01 Stale Documentation Cleanup

Current issue:

- Several docs correctly describe historic gaps but are stale for current module existence.

Known stale or partially stale claims:

- `dispatch/` missing: stale; the directory exists.
- `runtime_feedback/` missing: stale; the directory exists.
- `projection/` missing: stale; the directory exists.
- `runner/merge.rs` missing: stale; the file exists.
- `runner/task_dag.rs` missing: stale; the file exists.
- `roko-agent/src/runtime_events.rs` missing: stale; the file exists.
- `ExtensionChain` always empty: stale for configured extensions; still unproven for all hooks and safety.
- `roko-serve` compile broken due `DreamCycleReport`: stale per [28](28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md).

Still true despite stale module-existence claims:

- Full end-to-end provider proof is not complete.
- Full Mori parity is not complete.
- `orchestrate.rs` is not retired.
- Several systems are wired but not proven through the active runner path.

Done criteria:

- [ ] Update each source doc after implementing its gap, not only this ledger.
- [ ] Add proof links to [21-FEATURE-PARITY-MATRIX.md](21-FEATURE-PARITY-MATRIX.md).
- [ ] Move stale unchecked items to a "stale claim corrected" note instead of leaving them as active work.

## Source Doc Index By Task Type

Use this table to avoid rereading the entire folder.

| Task Type | Start Here | Then Read |
|---|---|---|
| Choose next runtime work | This file | [23](23-HANDOFF-OPEN-ITEMS.md), [24](24-DEFINITIVE-GAP-LIST.md) |
| Audit inelegant architecture / side effects | [30](30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md) | [31](31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md), this file, [20](20-RUNTIME-RECONCILIATION.md) |
| Provider/model proof | P0-02, P0-03 | [01](01-AGENT-DISPATCH.md), [27](27-FILESYSTEM-RUNTIME-CI-AUDIT.md), [28](28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md) |
| Prompt/context work | P0-04 | [05](archive/2026-04-26-verified/05-PROMPT-ASSEMBLY.md), [09](09-COMPOSITION-AUCTION.md), [16](16-INFRASTRUCTURE.md) |
| Feedback/learning work | P0-05 | [04](04-LEARNING.md), [10](10-DREAMS-CONSOLIDATION.md), [13](13-KNOWLEDGE-LIFECYCLE.md) |
| Resume/persistence work | P0-06 | [03](03-PERSISTENCE.md), [22](22-STABILITY-PLAN.md) |
| DAG/merge/concurrency work | P1-01 | [02](02-PLAN-EXECUTION.md), [11](11-PARALLEL-MERGE.md) |
| Gate/retry/replan work | P1-02 | [02](02-PLAN-EXECUTION.md), [14](archive/2026-04-26-verified/14-FAILURE-RETRY.md) |
| Knowledge/dream/affect work | P1-03 | [10](10-DREAMS-CONSOLIDATION.md), [12](12-AFFECT-ROUTING.md), [13](13-KNOWLEDGE-LIFECYCLE.md) |
| HTTP/TUI/projection work | P1-04 | [06](archive/2026-04-26-verified/06-OBSERVABILITY.md), [27](27-FILESYSTEM-RUNTIME-CI-AUDIT.md) |
| Safety/extensions work | P1-05 | [15](archive/2026-04-26-verified/15-SAFETY-EXTENSIONS.md) |
| Parity claim | [21](21-FEATURE-PARITY-MATRIX.md) | [22](22-STABILITY-PLAN.md), this file |

## Minimum Proof Standard

For any checklist item flipped to `[x]`, record at least one of:

- source proof: exact file path and symbol or line reference;
- command proof: command and result;
- artifact proof: path to report/log/event file;
- runtime proof: clean temporary workspace run with no mocks.

Required proof for parity:

- [ ] one-task implementation plan;
- [ ] multi-task dependency plan;
- [ ] gate failure then auto-fix;
- [ ] verify/reviewer flow;
- [ ] crash/resume;
- [ ] routing observation reuse;
- [ ] knowledge hint reuse;
- [ ] dream/consolidation trigger;
- [ ] merge success and merge conflict;
- [ ] HTTP/TUI/projection query proof;
- [ ] full provider matrix with live credentials or explicit non-success classifications.

## Current Recommended Work Order

1. Generate the side-effect inventory and owner manifest, then classify every production violation.
2. Build `RuntimeContext` / `RuntimeBuilder` so config, secrets, provider registry, policy, feedback, projection, store, command service, and query service are resolved once.
3. Move direct dispatch, chat, one-shot, PRD, research, dreams, neuro, vision, and provider probes onto the shared runtime/model-call path.
4. Collapse direct runner-local feedback writes behind `FeedbackFacade` or explicitly mark compatibility mirrors.
5. Prove active runner persistence/resume with `run-state.json` and JSONL recovery.
6. Prove multi-task DAG plus merge success/conflict through the active runner.
7. Prove prompt/knowledge/learning reuse across two runs.
8. Prove HTTP/TUI/projection querying against the same run.
9. Run the full provider proof matrix with real credentials and record results.
10. Freeze and shrink `orchestrate.rs`.
11. Reconcile stale source docs and update parity rows.

## 2026-04-27 Deepening Pass - Canonical Implementation Queue

This pass makes the ledger a true aggregation layer for the new architecture docs. The earlier version correctly listed gaps, but it still let agents choose proof work before ownership work. That order is unsafe: proof is not trustworthy until side effects, config, dispatch, query, and feedback have exclusive owners.

Updated self-grade after this deepening pass: `9.90 / 10`.

Reason: the ledger now puts side-effect ownership at P0, adds a P0-00 gap card, and defines the canonical queue below. It is not 10 because individual source docs still need proof links as implementation lands.

### Canonical Queue

Agents should work in this order unless the user explicitly asks for a narrower fix.

| Order | Workstream | Why It Comes Here | Primary Checklist |
|---|---|---|---|
| 0 | Side-effect inventory and owner manifest | Prevents more ad hoc wiring while implementing the rest | [30](30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md) SE-01 |
| 1 | Runtime context / config / policy | Every command needs the same provider, secret, policy, feedback, projection, and storage context | [33](33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md), [32](32-DEPENDENCY-LAYERING-AUDIT.md) |
| 2 | Inference gateway / model-call service | All model calls must share provider proof, events, prompts, cache, cost, and credentials | [41](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md) |
| 3 | Command/query service boundary | CLI, HTTP, TUI, and proof need one command path and one query path | [40](40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md), [34](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md) |
| 4 | Runner reducer and effect drivers | The event loop must stop owning persistence, feedback, gate, dream, merge, and projection effects | [39](39-RUNNER-EXECUTION-POLICY-AUDIT.md), [30](30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md) |
| 5 | Feedback transaction spine | Episodes, efficiency, knowledge, routing, conductor, gates, and dreams need one cognitive transaction | [38](38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md) |
| 6 | Background supervisor and process lifecycle | Fire-and-forget route/runner work must become durable, cancellable, and queryable | [35](35-TASK-PROCESS-LIFECYCLE-AUDIT.md) |
| 7 | Artifact repository and workspace layout | PRDs, plans, tasks, jobs, research, logs, and proof bundles need typed repositories | [37](37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md) |
| 8 | Workflow engine / one-shot path | Idea to PRD to plan to tasks to run should be one durable workflow, not five unrelated commands | [36](36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md) |
| 9 | Proof matrix and parity | Only after common ownership exists can provider/projection/resume/merge proof be trusted | [21](21-FEATURE-PARITY-MATRIX.md), [27](27-FILESYSTEM-RUNTIME-CI-AUDIT.md), [28](28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md) |

### Aggregated P0 Checklist For A No-Context Agent

If another agent gets only this file, it should implement these unchecked items in order:

- [ ] Read this file and [30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md](30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md).
- [ ] Add the side-effect inventory generator and owner manifest.
- [ ] Run the generator and record total Rust files, matched files, production violations, unknown owners, and allowlisted temporary violations.
- [ ] Add `RuntimeContextBuild` and `ResolvedRuntimeConfig` records that capture config source, secret source labels, provider registry, runtime policy, feedback facade, projection/query services, artifact repositories, and event store.
- [ ] Replace production `RunConfig {` literals with builder usage.
- [ ] Make dangerous permission bypass opt-in, provenance-tracked, and queryable.
- [ ] Create or finish `ModelCallService` / `InferenceGateway` so runner, HTTP, provider probes, research, dreams, neuro, vision, and web search share one model-call path.
- [ ] Route `dispatch_direct` callers through `RuntimeCommand::SinglePrompt` or equivalent command service.
- [ ] Ensure every model call emits provider-neutral runtime events, prompt diagnostics, provider/model labels, cost/tokens when available, and redacted error evidence.
- [ ] Define `RuntimeCommandService` for start, resume, cancel, single prompt, workflow run, background task submit, and proof export.
- [ ] Define `RuntimeQueryService` for run state, events, streams, providers, prompt diagnostics, gates, retries, merges, feedback, knowledge, background tasks, artifacts, and proof bundles.
- [ ] Move route/TUI/status direct storage reads behind query/repository services.
- [ ] Split runner event-loop side effects into effect drivers or equivalent service calls.
- [ ] Move direct feedback writes into facade sinks and require a shared feedback transaction id.
- [ ] Move dream trigger, gate threshold, router, bandit, conductor, and knowledge observations behind sinks.
- [ ] Move route-level PRD/job/research/dream/deployment/agent-registration spawns behind `BackgroundTaskSupervisor`.
- [ ] Add typed artifact repositories for PRD, plan, task, job, research, template, proof, and runtime logs.
- [ ] Implement one-shot workflow orchestration that can generate PRD, plan, tasks, and execute through one durable workflow id.
- [ ] Run proof for one-task plan, multi-task DAG, gate failure retry, replan, crash/resume, merge success, merge conflict, HTTP/TUI query, knowledge reuse, dream trigger, and provider matrix.
- [ ] Update [21-FEATURE-PARITY-MATRIX.md](21-FEATURE-PARITY-MATRIX.md) with proof links before claiming Mori parity.
- [ ] Archive a mori-diffs doc only after its remaining checklist items are implemented or explicitly superseded with proof links.

### Stop Conditions

Do not mark work complete if any of these are true:

- [ ] A feature works only through `orchestrate.rs`.
- [ ] A feature works only through a route-specific, TUI-specific, chat-specific, or proof-script-specific code path.
- [ ] A model call happens without provider-neutral events and prompt diagnostics.
- [ ] A background operation starts without a durable operation id.
- [ ] A proof script reads private storage files that HTTP/TUI cannot query through the same service.
- [ ] A checklist item is checked because a module exists but the active runtime path does not invoke it.
- [ ] Provider proof uses mocks where the user requested live provider behavior.

### Ledger Maintenance Rule

When implementation changes the truth:

- [ ] Update the specific source doc.
- [ ] Update this ledger's status label.
- [ ] Add source proof and command/artifact proof.
- [ ] Update [README.md](README.md) if priority or read order changes.
- [ ] Update [21-FEATURE-PARITY-MATRIX.md](21-FEATURE-PARITY-MATRIX.md) if the change affects Mori parity.
- [ ] Move completed docs to `archive/` only after proof-linked completion, not after code existence.
