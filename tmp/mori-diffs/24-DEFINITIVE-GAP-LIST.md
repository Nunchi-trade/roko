# Definitive Mori-Diffs Gap List

> This file is the exhaustive handoff for the remaining loose ends in `tmp/mori-diffs/`.
>
> Current note: [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) now supersedes this file as the canonical priority/impact ledger. Keep this file as historical gap-catalog evidence, but use `29` to decide what to implement next because several module-existence claims below are stale in the current working tree.
>
> Method: I enumerated every file in `tmp/mori-diffs/`, scanned all markdown files for unchecked boxes, `gap` markers, `stub` markers, and target-state language, then reconciled those notes against the current code paths in `crates/` and the proof notes already recorded in `19-SELF-REVIEW-AND-PROOF.md` and `23-HANDOFF-OPEN-ITEMS.md`.
>
> Conclusion: the Mori-diffs package is **not fully implemented**. Some slices are proven and archived. Many of the core runtime seams are still open.

### Architecture Runner Update (2026-04-28)
The 16-batch arch runner created all foundation modules referenced in Section 0's module targets. Gap clusters GAP-01 through GAP-08 now have architectural infrastructure (foundation traits, services, execution engine, adapters). Remaining gaps are primarily around proof runs, integration testing, and legacy retirement — not missing modules.

## What Is Already Proven

- `roko plan run` now uses the runner v2 path.
- Real provider probes pass for Anthropic, Moonshot, Z.ai, OpenAI, and Perplexity when configured with live keys.
- The runner has durable snapshots and a real gate path.
- Provider-neutral runtime events and projection scaffolding exist in parts of the codebase.

Those facts are real, but they do **not** mean the Mori-diffs package is complete.

## Search Result

The remaining unchecked items cluster into the same recurring gaps:

- dispatch is still not fully provider-neutral
- prompt assembly is still not fully centralized
- merge/retry/resume wiring is still partial
- persistence is still missing a full run-state contract
- learning/knowledge/dreams are still not all driven from one live feedback surface
- observability still lacks queryable event indexes and run-scoped APIs
- safety/extension hooks still are not wired through every active path
- `orchestrate.rs` still contains behavior that has not been fully retired or mirrored in runner v2

## Definitive Open Gaps

### 0. Missing Module Targets Still Called Out By The File Map

Source doc:

- [08-FILE-MAP.md](08-FILE-MAP.md)

Still-missing or still-unfinished module targets:

- `crates/roko-agent/src/runtime_events.rs`
- `crates/roko-cli/src/dispatch/mod.rs`
- `crates/roko-cli/src/dispatch/model_routing.rs`
- `crates/roko-cli/src/dispatch/prompt_builder.rs`
- `crates/roko-cli/src/dispatch/outcome.rs`
- `crates/roko-cli/src/dispatch/warm_pool.rs`
- `crates/roko-cli/src/runtime_feedback/mod.rs`
- `crates/roko-cli/src/runtime_feedback/episodes.rs`
- `crates/roko-cli/src/runtime_feedback/routing.rs`
- `crates/roko-cli/src/runtime_feedback/knowledge.rs`
- `crates/roko-cli/src/runtime_feedback/conductor.rs`
- `crates/roko-cli/src/runtime_feedback/dreams.rs`
- `crates/roko-cli/src/projection/mod.rs`
- `crates/roko-cli/src/projection/dashboard.rs`
- `crates/roko-cli/src/projection/cli_progress.rs`
- `crates/roko-cli/src/runner/task_dag.rs`
- `crates/roko-cli/src/runner/merge.rs`

Why this matters:

- the file map is still the clearest index of modules that need to exist or be formally reconciled with the newer `dispatch_v2` / runner-v2 structure

### 1. Dispatch And Agent Runtime

Source docs:

- [01-AGENT-DISPATCH.md](01-AGENT-DISPATCH.md)
- [07-MIGRATION.md](07-MIGRATION.md)
- [17-ARCHITECTURE-REALITY-CHECK.md](17-ARCHITECTURE-REALITY-CHECK.md)
- [18-MASTER-AUDIT.md](18-MASTER-AUDIT.md)
- [20-RUNTIME-RECONCILIATION.md](20-RUNTIME-RECONCILIATION.md)

Open items:

- [ ] Create the intended `crates/roko-cli/src/dispatch/` module family or formally reconcile the design with `dispatch_v2.rs`.
- [ ] Replace direct `agent_stream::spawn_agent` calls with a dispatcher facade that can handle CLI streams and one-shot `AgentResult` providers.
- [ ] Wire `AgentDispatcherV2::run_agent_result_bridge` into the runner path or replace it with a better provider-neutral bridge.
- [ ] Move model choice out of `task_def.model_hint.or(config.model)` and into a routing module that can consult `CascadeRouter`.
- [ ] Preserve a no-mock-compatible test seam without relying on production mocks.

Why this is still a gap:

- the live runner still has to finish the migration from backend-shaped stream handling to provider-neutral runtime events
- the active path still needs a single dispatch facade that owns both CLI-stream agents and result-only providers

### 2. Prompt And Composition

Source docs:

- [05-PROMPT-ASSEMBLY.md](archive/2026-04-26-verified/05-PROMPT-ASSEMBLY.md)
- [17-ARCHITECTURE-REALITY-CHECK.md](17-ARCHITECTURE-REALITY-CHECK.md)
- [18-MASTER-AUDIT.md](18-MASTER-AUDIT.md)
- [20-RUNTIME-RECONCILIATION.md](20-RUNTIME-RECONCILIATION.md)

Open items:

- [ ] Create `crates/roko-cli/src/dispatch/prompt_builder.rs` or a clearly equivalent prompt assembler module.
- [ ] Update the live runner to call `PromptAssembler` instead of the minimal prompt helper path.
- [ ] Keep `build_minimal_system_prompt` only for tests or delete it from production flow.
- [ ] Define `PromptAssemblyRequest` and `AssembledPrompt` with structured retry, allowlist, and diagnostics fields.
- [ ] Query playbooks and neuro knowledge during prompt assembly.
- [ ] Enforce role-specific tool allowlists.
- [ ] Include code index context as a structured section instead of raw concatenation.
- [ ] Enforce prompt token budget with deterministic section dropping.
- [ ] Add snapshot tests for implementer, reviewer, and retry prompts.

Why this is still a gap:

- prompt construction is still split between the older helper path and the target-state composition design
- structured retry feedback and knowledge-backed prompt shaping are not yet fully centralized

### 3. Plan Execution, DAG, And Merge

Source docs:

- [02-PLAN-EXECUTION.md](02-PLAN-EXECUTION.md)
- [11-PARALLEL-MERGE.md](11-PARALLEL-MERGE.md)
- [20-RUNTIME-RECONCILIATION.md](20-RUNTIME-RECONCILIATION.md)

Open items:

- [ ] Raise `max_concurrent_tasks` only after a per-plan agent handle map exists.
- [ ] Replace any remaining ad hoc merge completion with `MergeQueue`-backed dispatch.
- [ ] Add a real post-merge regression gate.
- [ ] Keep plan-level timeout and retry backoff visible in the active runtime.
- [ ] Ensure the active runner uses the real DAG resolver path everywhere, not just in isolated helper seams.

Why this is still a gap:

- multi-task execution is better than before, but the merge/retry/resume story is not fully closed
- the current runner still needs proof that concurrency, conflict serialization, and post-merge validation are all wired through the same path

### 4. Persistence And Resume

Source docs:

- [03-PERSISTENCE.md](03-PERSISTENCE.md)
- [07-MIGRATION.md](07-MIGRATION.md)
- [20-RUNTIME-RECONCILIATION.md](20-RUNTIME-RECONCILIATION.md)

Open items:

- [ ] Persist `run-state.json` or an equivalent runner-level snapshot with cost, token, and completed-task state.
- [ ] Persist router state, gate thresholds, and any other feedback state the live runner mutates.
- [ ] Add strict resume validation against changed task definitions, not just plan-id overlap.
- [ ] Add `run_id` to executor snapshot data, not only runtime events.
- [ ] Add JSONL recovery behavior for partial append failures.
- [ ] Prove interrupt, crash, and resume behavior without duplicate completion.

Why this is still a gap:

- snapshotting exists, but it is not yet the whole runtime contract
- resume must be validated against task drift, not merely against the presence of a prior snapshot

### 5. Learning, Knowledge, And Dreams

Source docs:

- [04-LEARNING.md](04-LEARNING.md)
- [10-DREAMS-CONSOLIDATION.md](10-DREAMS-CONSOLIDATION.md)
- [12-AFFECT-ROUTING.md](12-AFFECT-ROUTING.md)
- [13-KNOWLEDGE-LIFECYCLE.md](13-KNOWLEDGE-LIFECYCLE.md)
- [17-ARCHITECTURE-REALITY-CHECK.md](17-ARCHITECTURE-REALITY-CHECK.md)
- [18-MASTER-AUDIT.md](18-MASTER-AUDIT.md)

Open items:

- [ ] Add one live feedback facade that receives runner events and writes learning, routing, knowledge, conductor, and dream outputs.
- [ ] Replace runner-local episode and efficiency helpers with the shared `LearningRuntime` path.
- [ ] Remove hardcoded backend and role values from runner episode logging.
- [ ] Emit per-turn efficiency events, not only per-task summaries.
- [ ] Load and update `CascadeRouter` state from the active runner dispatch path.
- [ ] Wire knowledge lifecycle ingestion into successful runner completions.
- [ ] Wire dream trigger events into plan completion or idle checks.
- [ ] Add affect and provider-health inputs to the live routing path.
- [ ] Make knowledge reuse and falsifier observations visible in the live runner.

Why this is still a gap:

- the learning stack is real, but it is still not all driven from one authoritative runtime feedback surface

### 6. Observability And Projection

Source docs:

- [06-OBSERVABILITY.md](archive/2026-04-26-verified/06-OBSERVABILITY.md)
- [17-ARCHITECTURE-REALITY-CHECK.md](17-ARCHITECTURE-REALITY-CHECK.md)
- [18-MASTER-AUDIT.md](18-MASTER-AUDIT.md)
- [20-RUNTIME-RECONCILIATION.md](20-RUNTIME-RECONCILIATION.md)

Open items:

- [ ] Add `run_id` to every persisted runtime event payload.
- [ ] Add a lightweight event query index file per run.
- [ ] Add query endpoints for events and gates by run id.
- [ ] Publish tool, token, cost, gate, retry, and dream events to the projection layer.
- [ ] Keep dashboard snapshots bounded and avoid storing large raw tool output.

Why this is still a gap:

- the projection layer exists, but the queryable, run-scoped observability story is not complete

### 7. Safety And Extensions

Source docs:

- [15-SAFETY-EXTENSIONS.md](archive/2026-04-26-verified/15-SAFETY-EXTENSIONS.md)

Open items:

- [ ] Wire extension chain initialization into runner startup.
- [ ] Wire extension hooks into dispatch, gate, error, and shutdown paths.
- [ ] Add tests for missing contracts and hook invocation order.

Why this is still a gap:

- the contracts exist, but the live runner still needs the full hook chain wired through every active path

### 8. Migration, Parity, And Hardening

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

Why this is still a gap:

- the repo still has a transition period where both old and new runtime stories exist
- the remaining work is not design-only; it is proof, parity, and retirement work

## Why `roko prd draft new` Failed In The Log You Showed

That failure does **not** prove the Mori-diffs package is complete.

It proves the opposite:

- `roko prd draft new` still routes through the direct agent-exec path, not the full runner architecture
- in that run, the workspace config did not provide a provider registry, so the path fell back to the default `claude` command
- the underlying `claude` subprocess exited `1` after a few seconds and produced no useful completion
- the scaffold was preserved, which is correct, but the flow still depends on the CLI subprocess being healthy

So the failure was a real runtime failure, but it was not the same thing as the runner-vs-orchestrate architectural gap. Both issues still exist.

## Bottom Line

No, the Mori-diffs folder does **not** mean everything is fully implemented.

What it means is:

- some slices are proven
- some slices are archived as verified design/proof records
- the core runtime still has open seams in dispatch, composition, feedback, observability, persistence, safety, and parity

If you want the next agent to work effectively, this file plus `23-HANDOFF-OPEN-ITEMS.md` are the right starting points.

## 2026-04-27 Deepening Pass - Stale-Claim Correction And Canonical Handoff

### Self-grade for this deepening pass

Initial rating: `9.90 / 10`.

Rationale: this pass keeps the value of the original definitive gap list but prevents it from misleading the next agent. The old file correctly identified the architectural clusters, but several "missing module" items are now stale because the files exist. This update records that correction, maps each remaining gap to the current owner docs, and turns the document into a no-context handoff that points agents toward wiring/proof work instead of recreating modules.

### Current source correction

The first section above says module targets were still missing or unfinished. On 2026-04-27, the missing-file part is stale. This command now reports all listed targets as present:

```bash
for p in \
  crates/roko-agent/src/runtime_events.rs \
  crates/roko-cli/src/dispatch/mod.rs \
  crates/roko-cli/src/dispatch/model_routing.rs \
  crates/roko-cli/src/dispatch/prompt_builder.rs \
  crates/roko-cli/src/dispatch/outcome.rs \
  crates/roko-cli/src/dispatch/warm_pool.rs \
  crates/roko-cli/src/runtime_feedback/mod.rs \
  crates/roko-cli/src/runtime_feedback/episodes.rs \
  crates/roko-cli/src/runtime_feedback/routing.rs \
  crates/roko-cli/src/runtime_feedback/knowledge.rs \
  crates/roko-cli/src/runtime_feedback/conductor.rs \
  crates/roko-cli/src/runtime_feedback/dreams.rs \
  crates/roko-cli/src/projection/mod.rs \
  crates/roko-cli/src/projection/dashboard.rs \
  crates/roko-cli/src/projection/cli_progress.rs \
  crates/roko-cli/src/runner/task_dag.rs \
  crates/roko-cli/src/runner/merge.rs; do
    test -e "$p" && echo "exists $p" || echo "missing $p"
done
```

Observed result:

- [ ] All `17` module targets listed in section `0` exist in the current checkout.
- [ ] The remaining gap is not "create these files"; it is "make every active CLI/HTTP/TUI/proof path use these modules as the only implementation path."
- [ ] Keep section `0` above as historical evidence only.
- [ ] Use [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md), [25-CODE-ONLY-LEGACY-AUDIT.md](25-CODE-ONLY-LEGACY-AUDIT.md), and [26-REPOSITORY-WIDE-CODE-AUDIT.md](26-REPOSITORY-WIDE-CODE-AUDIT.md) for current implementation ordering.

### Current wiring evidence

The target seams exist, but source search still shows mixed routing:

- [ ] `crates/roko-cli/src/dispatch/mod.rs:197` documents the production bridge around `AgentDispatcherV2::run_agent_result_bridge`.
- [ ] `crates/roko-cli/src/dispatch/mod.rs:271` calls `AgentDispatcherV2::run_agent_result_bridge`.
- [ ] `crates/roko-cli/src/dispatch/prompt_builder.rs:247` defines `PromptAssembler`.
- [ ] `crates/roko-cli/src/runner/event_loop.rs:1832` constructs `PromptAssembler::new()` inside the runner.
- [ ] `crates/roko-cli/src/runner/event_loop.rs:1113` and `:2256` construct `PlanMerger`.
- [ ] `crates/roko-cli/src/runner/persist.rs:81` defines `RunStateSnapshot`.
- [ ] `crates/roko-cli/src/runner/persist.rs:195` defines `save_run_state`.
- [ ] `crates/roko-cli/src/runtime_feedback/mod.rs:153` defines `FeedbackFacade`.
- [ ] `crates/roko-cli/src/runner/event_loop.rs:1267` still converts runner events through `runner_event_to_feedback`, and `:1285` defines that conversion locally.
- [ ] `crates/roko-cli/src/commands/prd.rs:8`, `crates/roko-cli/src/commands/research.rs:8`, and `crates/roko-cli/src/commands/plan.rs:403` / `:470` still import direct `agent_exec` helpers.
- [ ] `crates/roko-cli/src/agent_exec.rs:1` explicitly says it is for direct CLI flows such as PRD, research, and plan generation.
- [ ] `crates/roko-cli/src/unified.rs:95` and `crates/roko-cli/src/chat_inline.rs:1475` still call `dispatch_direct::dispatch_prompt`.
- [ ] `crates/roko-serve/src/routes/providers.rs:301`, `crates/roko-cli/src/vision_loop/evaluator.rs:79`, and `crates/roko-serve/src/dispatch.rs:1807` still call `create_agent_for_model` directly.

Conclusion:

- [ ] The architecture has moved from "missing modules" to "modules exist but are not yet the exclusive runtime path."
- [ ] Completion requires route/call-site migration plus proof, not another design-only rewrite.

### Canonical current gap map

Use this as the corrected version of the old eight-cluster list:

- [ ] `GAP-01 Runtime context`: build one resolved context for config, credentials, provider registry, policy, event store, artifact repositories, feedback, command service, and query service. Owner docs: [33](33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md), [32](32-DEPENDENCY-LAYERING-AUDIT.md), [30](30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md).
- [ ] `GAP-02 Dispatch/model calls`: route runner, chat, unified, PRD, plan generation, research, provider probes, dreams, neuro, vision, and HTTP inference through one dispatcher/model-call service. Owner docs: [41](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md), [25](25-CODE-ONLY-LEGACY-AUDIT.md), [01](01-AGENT-DISPATCH.md).
- [ ] `GAP-03 Prompt assembly`: make `PromptAssembler` the only production prompt path and record prompt diagnostics. Owner docs: [25](25-CODE-ONLY-LEGACY-AUDIT.md), [29](29-CURRENT-RUNTIME-GAP-LEDGER.md), [09](09-COMPOSITION-AUCTION.md).
- [ ] `GAP-04 Runner policy`: make runner reducer/effect drivers own scheduling, gates, retry/replan, merge, snapshots, and resume. Owner docs: [39](39-RUNNER-EXECUTION-POLICY-AUDIT.md), [02](02-PLAN-EXECUTION.md), [11](11-PARALLEL-MERGE.md).
- [ ] `GAP-05 Merge proof`: require real merge backend evidence before `MergeSucceeded`. Owner docs: [25](25-CODE-ONLY-LEGACY-AUDIT.md), [39](39-RUNNER-EXECUTION-POLICY-AUDIT.md), [27](27-FILESYSTEM-RUNTIME-CI-AUDIT.md).
- [ ] `GAP-06 Persistence/resume`: prove active-runner `run-state.json`, JSONL recovery, strict resume validation, crash recovery, and no duplicate completion. Owner docs: [29](29-CURRENT-RUNTIME-GAP-LEDGER.md), [37](37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md), [03](03-PERSISTENCE.md).
- [ ] `GAP-07 Feedback/cognition`: collapse runner-local feedback, learning, knowledge, routing, dreams, and affect into one feedback transaction spine. Owner docs: [38](38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md), [04](04-LEARNING.md), [10](10-DREAMS-CONSOLIDATION.md), [12](12-AFFECT-ROUTING.md), [13](13-KNOWLEDGE-LIFECYCLE.md).
- [ ] `GAP-08 Observability/query`: make HTTP/TUI/CLI/proof read the same projections and query services. Owner docs: [34](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md), [40](40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md), [27](27-FILESYSTEM-RUNTIME-CI-AUDIT.md).
- [ ] `GAP-09 Workflow entrypoints`: replace separate PRD/plan/research/job/run direct-agent paths with one workflow command path. Owner docs: [36](36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md), [35](35-TASK-PROCESS-LIFECYCLE-AUDIT.md), [37](37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md).
- [ ] `GAP-10 Compatibility retirement`: classify remaining legacy behavior through generated ledgers and exclude compatibility-only paths from parity claims. Owner docs: [25](25-CODE-ONLY-LEGACY-AUDIT.md), [26](26-REPOSITORY-WIDE-CODE-AUDIT.md), [28](28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md).
- [ ] `GAP-11 Provider proof`: run the real provider matrix through the active runtime with precise statuses. Owner docs: [27](27-FILESYSTEM-RUNTIME-CI-AUDIT.md), [28](28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md), [41](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md).
- [ ] `GAP-12 Archive/proof hygiene`: archive only after implementation and proof links exist, not after module existence. Owner docs: [29](29-CURRENT-RUNTIME-GAP-LEDGER.md), [21](21-FEATURE-PARITY-MATRIX.md), [23](23-HANDOFF-OPEN-ITEMS.md).

### No-context implementation order

Give another agent this order if it has no other context:

- [ ] Implement `GAP-01` first so every later command receives the same resolved runtime context.
- [ ] Implement `GAP-02` second so every model/provider call uses one dispatch/model-call path.
- [ ] Implement `GAP-03` third so every dispatch call has prompt diagnostics and typed context.
- [ ] Implement `GAP-04` and `GAP-05` together so runner state changes and merge state changes are evidence-backed.
- [ ] Implement `GAP-06` before crash/resume proof.
- [ ] Implement `GAP-08` before UI/API proof so proof queries the same state users see.
- [ ] Implement `GAP-07` after the event spine is stable so learning/dreams/knowledge consume authoritative runtime events.
- [ ] Implement `GAP-09` after command/query services exist so PRD/plan/research/job paths do not recreate direct-agent execution.
- [ ] Implement `GAP-10` continuously using the generated ledgers from [25](25-CODE-ONLY-LEGACY-AUDIT.md) and [26](26-REPOSITORY-WIDE-CODE-AUDIT.md).
- [ ] Implement `GAP-11` only after dispatch and observability are unified; otherwise provider proof can pass through the wrong path.
- [ ] Implement `GAP-12` after every gap has command proof, source proof, artifact proof, and runtime proof.

### Definitive proof matrix for this file

- [ ] `module_presence`: all target files from section `0` exist, but this is not enough for closure.
- [ ] `exclusive_dispatch`: source search shows no production direct `agent_exec`, `dispatch_direct`, or `create_agent_for_model` call outside the model-call service.
- [ ] `exclusive_prompt`: source search shows no production prompt builder outside `PromptAssembler`.
- [ ] `exclusive_merge`: source search shows `MergeSucceeded` can be emitted only after `MergeService` or `PlanMerger` evidence.
- [ ] `exclusive_feedback`: runner emits one authoritative feedback event stream; direct feedback writes are compatibility mirrors only.
- [ ] `exclusive_events`: production paths write/query through the runtime event store and projection/query services.
- [ ] `exclusive_workflow`: PRD, plan, research, job, and run entrypoints share the same workflow engine or command service.
- [ ] `real_provider_matrix`: Anthropic, OpenAI, Moonshot, Z.AI, Perplexity, Claude CLI, and Codex CLI are each `proved`, `missing_credentials`, `auth_failed`, `rate_limited`, or `unsupported` through the same runtime path.
- [ ] `resume_crash`: crash/resume proof shows no duplicate completion and no lost merge/gate/provider state.
- [ ] `archive_ready`: [21-FEATURE-PARITY-MATRIX.md](21-FEATURE-PARITY-MATRIX.md), [23-HANDOFF-OPEN-ITEMS.md](23-HANDOFF-OPEN-ITEMS.md), and [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) contain proof links for every closed item.

### Updated bottom line

- [ ] Do not use this file alone as the current implementation queue.
- [ ] Use this file as the historical gap taxonomy plus stale-claim correction.
- [ ] Use [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) as the priority ledger.
- [ ] Use [25-CODE-ONLY-LEGACY-AUDIT.md](25-CODE-ONLY-LEGACY-AUDIT.md) and [26-REPOSITORY-WIDE-CODE-AUDIT.md](26-REPOSITORY-WIDE-CODE-AUDIT.md) as executable-source cleanup ledgers.
- [ ] Treat any "file exists" claim as `wired-unproven` until an end-to-end proof command and artifact are recorded.
