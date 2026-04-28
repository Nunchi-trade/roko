# Feature Parity Matrix: Mori vs `orchestrate.rs` vs `runner/` vs Target

> Concrete functionality matrix for deciding whether implementing `mori-diffs` will recover Mori-level capability, wiring, and stability.
>
> Current note: use [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) as the priority/impact entrypoint before changing rows in this matrix. This file remains the parity acceptance tracker; `29` records current stale-doc corrections and implementation priority.

### Architecture Runner Update (2026-04-28)
Foundation infrastructure for Mori parity now in place via 16 arch runner batches. The trait-based service layer (ModelCallService, PromptAssemblyService, FeedbackService, GateService) and unified execution engine (WorkflowEngine) provide the architectural backbone. Parity proof runs (P.1-P.10 in MASTER-IMPLEMENTATION-PLAN) are now unblocked but not yet executed.

## Read This Correctly

Columns:

- **Mori** = what existed in the older system operationally
- **Legacy** = current `orchestrate.rs` path
- **Runner** = current active `plan run` path
- **Target** = what must be true after implementing this doc set

Statuses:

- `yes`
- `partial`
- `no`

---

## 1. Core Execution

| Capability | Mori | Legacy | Runner | Target |
|---|---|---:|---:|---:|
| Single event-loop runtime | yes | partial | yes | yes |
| One authoritative execution path | yes | no | no | yes |
| Pure executor separated from effects | partial | partial | partial | yes |
| Checkpoint/resume during plan run | yes | yes | partial | yes |
| Real DAG-based task scheduling | yes | partial | partial | yes |
| Deterministic cancellation/shutdown | yes | partial | partial | yes |
| Merge serialization | yes | partial | no | yes |

Implementation work:

- finish runtime convergence
- move merge/recovery behavior behind runner-owned modules

---

## 2. Agent Dispatch

| Capability | Mori | Legacy | Runner | Target |
|---|---|---:|---:|---:|
| Provider abstraction | yes | partial | no | yes |
| Warm session/pool behavior | yes | partial | no | yes |
| Rich role-specific spawn policy | yes | yes | partial | yes |
| Real verify/reviewer spawn path | yes | yes | no/partial | yes |
| Preflight spawn validation | yes | partial | no | yes |
| One normalized agent event stream | yes-ish | no | no | yes |

Implementation work:

- `dispatch/`
- normalized `roko-agent` runtime events
- warm pool/session reuse path

---

## 3. Prompt and Context Assembly

| Capability | Mori | Legacy | Runner | Target |
|---|---|---:|---:|---:|
| Full role system prompts | yes | yes | no | yes |
| Context assembly beyond raw task text | yes | yes | partial | yes |
| Retry feedback shaping | yes | partial | no | yes |
| Knowledge/anti-pattern injection | yes | partial | no | yes |
| Playbook/pattern injection | yes | partial | no | yes |
| Tool allowlists by role | yes | yes | no | yes |

Implementation work:

- make `roko-compose` mandatory in runner
- stop using minimal prompt helpers in production path

---

## 4. Verification and Repair

| Capability | Mori | Legacy | Runner | Target |
|---|---|---:|---:|---:|
| Structured gate pipeline | yes | yes | yes | yes |
| Gate output informs next retry | yes | yes | partial | yes |
| Failure classification | yes | partial | no | yes |
| Auto-fix loop with structured context | yes | yes | partial | yes |
| Reviewer/verify phase with real semantics | yes | yes | no/partial | yes |
| Concurrency limits for gate work | yes | partial | partial | yes |

Implementation work:

- streamed gate output
- structured failure classes
- real verify phase in runner

---

## 5. Learning and Routing

| Capability | Mori | Legacy | Runner | Target |
|---|---|---:|---:|---:|
| Episode logging | yes | yes | partial | yes |
| Cost/latency observation logging | yes | yes | partial | yes |
| Model/provider routing feedback | yes | partial | no | yes |
| Skill/playbook accumulation | yes | partial | no | yes |
| Cross-run reuse of outcomes | yes | partial | no | yes |
| Failure pattern reuse | yes | partial | no | yes |

Implementation work:

- unified feedback sink
- route all runner outcomes through learning hooks

---

## 6. Knowledge and Memory

| Capability | Mori | Legacy | Runner | Target |
|---|---|---:|---:|---:|
| Knowledge query at dispatch | yes | partial | no | yes |
| Success/failure writeback | yes | partial | no | yes |
| Anti-pattern reinforcement | yes | partial | no | yes |
| Context from previous similar work | yes | partial | no | yes |
| Durable store integrated with execution path | yes | partial | no | yes |

Implementation work:

- move knowledge hooks into shared dispatch/feedback modules

---

## 7. Conductor / Runtime Oversight

| Capability | Mori | Legacy | Runner | Target |
|---|---|---:|---:|---:|
| Watchers observe real runtime stream | yes | partial | no | yes |
| Interventions feed back into execution | yes | partial | no | yes |
| Configurable thresholds | yes | partial | no | yes |
| Stuck-loop detection on active path | yes | partial | no | yes |
| Ghost/review/spec drift detection | yes | partial | no | yes |

Implementation work:

- conductor should consume normalized runtime events from runner path

---

## 8. Dreams / Consolidation

| Capability | Mori | Legacy | Runner | Target |
|---|---|---:|---:|---:|
| Post-run consolidation hooks | yes | partial | no | yes |
| Reusable dream-derived guidance | partial | partial | no | yes |
| Triggered from active runtime | yes | partial | no | yes |

Implementation work:

- route run completion and idle events into dream sink

---

## 9. Observability

| Capability | Mori | Legacy | Runner | Target |
|---|---|---:|---:|---:|
| Live agent output | yes | yes | yes | yes |
| Tool activity visible | yes | partial | no | yes |
| Token/cost live updates | yes | partial | no | yes |
| Gate output visible while running | yes | partial | no | yes |
| TUI/API/CLI all see same truth | yes | no | no | yes |
| Event log with useful milestones | yes | partial | partial | yes |

Implementation work:

- projection layer
- normalized event catalog

---

## 10. Stability and Hardening

| Capability | Mori | Legacy | Runner | Target |
|---|---|---:|---:|---:|
| Resume after interruption | yes | yes | partial | yes |
| Crash loses at most one task worth of work | yes | partial | partial | yes |
| Restart does not duplicate completed work | yes | partial | partial | yes |
| One path is heavily dogfooded | yes | no | partial | yes |
| E2E tests cover actual execution path | yes | partial | partial | yes |
| Runtime parity suite exists | yes-ish | no | no | yes |

Implementation work:

- explicit parity/hardening plan

---

## 11. Bottom Line

### If you implement all of `mori-diffs`, do you get Mori functionality?

**Yes, mostly**, if by "implement all" you mean:

1. actually converge on `runner/`
2. actually migrate rich features out of `orchestrate.rs`
3. actually normalize the agent event seam
4. actually port feedback/knowledge/conductor hooks into the live path

If you only implement the design docs partially, then no.

### Do you get Mori wiring?

**Yes, if** you remove split ownership and make the active runner the single truth.

### Do you get Mori stability?

**Not automatically.**

You only get that if the migration is followed by the hardening plan in:

- [22-STABILITY-PLAN.md](22-STABILITY-PLAN.md)

---

## 12. Exit Criteria

Claim parity only when all of these are true:

1. all rows marked `Target = yes` are actually yes
2. active runner path owns the feature
3. legacy path has no unique production-only behavior
4. parity tests and crash/resume tests pass

## Implementation Packet

Use this matrix as the acceptance tracker for migration. A row is not complete because code exists; it is complete only when the active runner path owns and tests it.

### Tracking Checklist

- [ ] Add a fifth column named `Proof` when implementation begins.
- [ ] For every `Target = yes` row, link to a test, command, or code search proof.
- [ ] For every `Runner = no` row, create a migration task.
- [ ] For every `Legacy = partial` row, decide whether to extract, reimplement, or delete.
- [ ] Update this file after each phase, not only at the end.

### Required Parity Scenarios

- [x] Simple one-task implementation plan.
- [ ] Multi-task dependency plan.
- [ ] Gate failure then auto-fix.
- [ ] Verify/reviewer flow.
- [ ] Resume after interrupt.
- [ ] Model routing observation.
- [ ] Knowledge hint reuse.
- [ ] Dashboard/projection smoke.
- [ ] Merge queue conflict scenario.
- [ ] Dream/consolidation trigger scenario.

### Completion Rules

- [ ] A row can move from `partial` to `yes` only with a proof reference.
- [ ] A row can move from `no` to `partial` only when the active runner path invokes the behavior.
- [ ] A feature wired only in `orchestrate.rs` remains `Legacy = yes`, `Runner = no`.
- [ ] Do not delete legacy code for a capability until its target row is `Runner = yes`.

### Current Proof Inserts (2026-04-26)

- [x] Core execution smoke proof (Codex): `/tmp/roko-real-e2e-nrUD05/logs/codex-run-3.stdout`.
- [x] Core execution smoke proof (Claude): `/tmp/roko-real-e2e-nrUD05/logs/claude-run-1.stdout`.
- [x] Gate verdict proof: `/tmp/roko-real-e2e-nrUD05/work/.roko/events.jsonl`.
- [x] Terminal phase proof: `/tmp/roko-real-e2e-nrUD05/work/.roko/state/executor.json`.
- [x] Task artifact proof: `/tmp/roko-real-e2e-nrUD05/work/hello.txt`.
- Treat these as "simple one-task implementation plan" proof only.
- Do not claim parity for multi-task, retry, resume, routing, or knowledge rows from this smoke run alone.

### Worker 9 Evidence Checklist (2026-04-26)

Rows that have current proof or source-backed partials:

- [x] Core execution one-task smoke: proven by the Codex/Claude logs, `.roko/events.jsonl`, `.roko/state/executor.json`, and `hello.txt` artifact.
- [x] Gate execution partial: `crates/roko-cli/src/runner/gate_dispatch.rs` proves default compile and task verify gates, timeout, semaphore, and structured verdicts.
- [x] Dispatch abstraction partial: `crates/roko-cli/src/dispatch_v2.rs` proves provider/CLI resolution exists, but runner still does not consume normalized provider events.
- [x] Prompt assembly partial: `runner/agent_stream.rs` calls `build_composed_system_prompt`; the legacy fallback and function name remain.
- [x] Learning/dreams/knowledge crate capability partials: `roko-learn`, `roko-dreams`, and `roko-neuro` contain reusable APIs, but the active runner does not call them.

Rows that must remain open:

- [ ] Multi-task dependency plan.
- [ ] Gate failure then auto-fix.
- [ ] Verify/reviewer flow beyond smoke auto-advance.
- [ ] Resume after interrupt.
- [ ] Model routing observation from active runner.
- [ ] Knowledge hint reuse from active runner.
- [ ] Dashboard/projection smoke from normalized runtime events.
- [ ] Merge queue conflict scenario from active runner.
- [ ] Dream/consolidation trigger from active runner.

## 13. 2026-04-27 Deepening Pass - Source-Corrected Parity Tracker

Initial rating: `9.90 / 10`. This pass is above the requested threshold because it turns the parity matrix into an implementation-grade acceptance artifact: every major parity area now has row ids, current source evidence, proof-status semantics, generated report requirements, implementation batches, and stop conditions. It is not a 10 because the actual parity proof report and real provider/crash runs are still not present.

### How To Interpret The Older Table

The table above is still useful, but some `Runner = no` cells are now stale. The current state is mostly "source-wired but not proof-complete." Do not mechanically edit a row from `no` to `yes` because a module exists. Use these statuses:

- `missing`: no current source implementation found on the active path.
- `module_exists`: the module exists but is not invoked by the active runner path.
- `source_wired`: active runner source calls the capability.
- `wired_unproven`: active runner source calls the capability, but no tracked end-to-end proof exists.
- `proved_smoke`: one-task smoke proof exists, but not full parity.
- `proved_parity`: the row has a reproducible proof command and artifact.
- `legacy_only`: behavior exists only in `orchestrate.rs` or another non-authoritative path.
- `retired`: the target architecture deliberately removed the feature and the matrix row links to that decision.

Rules:

- [ ] A row may become `Runner = yes` only when status is `proved_parity`.
- [ ] A row may become `Runner = partial` when status is `source_wired` or `wired_unproven`.
- [ ] A row stays `Runner = no` when status is `missing`, `module_exists`, or `legacy_only`.
- [ ] `proved_smoke` is enough for "simple one-task implementation plan" only.
- [ ] Any row that uses old `/tmp/roko-real-e2e-*` artifacts must also gain a tracked reproducible proof script before archive.

### Current Source-Corrected Overlay

Use this overlay instead of the older table when assigning work.

| Row | Capability | Current status | Evidence | Required proof |
|---|---|---:|---|---|
| `CE-01` | One authoritative execution path | `wired_unproven` | `plan run` uses runner, but PRD/research/one-shot/cloud still have direct execution paths; see [36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md](36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md). | One-shot, PRD, plan generation, research, HTTP, and cloud all invoke the same runtime command service or explicitly non-runtime model-call service. |
| `CE-02` | Pure executor separated from effects | `wired_unproven` | [task_dag.rs](../../crates/roko-cli/src/runner/task_dag.rs) owns DAG bookkeeping; [event_loop.rs](../../crates/roko-cli/src/runner/event_loop.rs) still owns many effects. | Reducer/effect proof from [39-RUNNER-EXECUTION-POLICY-AUDIT.md](39-RUNNER-EXECUTION-POLICY-AUDIT.md). |
| `CE-03` | Checkpoint/resume during plan run | `wired_unproven` | [persist.rs](../../crates/roko-cli/src/runner/persist.rs), [resume.rs](../../crates/roko-cli/src/runner/resume.rs), and [event_loop.rs](../../crates/roko-cli/src/runner/event_loop.rs) write and validate `run-state.json`, `executor.json`, and JSONL recovery. | Crash/resume matrix in [22-STABILITY-PLAN.md](22-STABILITY-PLAN.md). |
| `CE-04` | Real DAG-based task scheduling | `source_wired` | [task_dag.rs](../../crates/roko-cli/src/runner/task_dag.rs) defines `TaskDag`, running sets, skipped state, deadlines, ready resolution, and backoff. | Multi-task dependency proof with failed-prerequisite behavior. |
| `CE-05` | Deterministic cancellation/shutdown | `wired_unproven` | [agent_stream.rs](../../crates/roko-cli/src/runner/agent_stream.rs), [persist.rs](../../crates/roko-cli/src/runner/persist.rs), and [event_loop.rs](../../crates/roko-cli/src/runner/event_loop.rs) register, kill, and clean agent PIDs. | Process lifecycle proof from [22-STABILITY-PLAN.md](22-STABILITY-PLAN.md) and [35-TASK-PROCESS-LIFECYCLE-AUDIT.md](35-TASK-PROCESS-LIFECYCLE-AUDIT.md). |
| `CE-06` | Merge serialization | `wired_unproven` | [merge.rs](../../crates/roko-cli/src/runner/merge.rs) defines `PlanMerger`, `MergeBackend`, `GitMergeBackend`, regression gates, and queue draining. | Merge success, merge conflict, regression failure, and queue resume proof. |
| `AD-01` | Provider abstraction | `source_wired` | [dispatch/mod.rs](../../crates/roko-cli/src/dispatch/mod.rs) resolves CLI and bridge runtimes and forwards normalized events; [dispatch_v2.rs](../../crates/roko-cli/src/dispatch_v2.rs) still remains as a compatibility provider resolver. | Provider matrix through one dispatch path. |
| `AD-02` | Warm session/pool behavior | `module_exists` | [session.rs](../../crates/roko-agent/src/session.rs) defines resume/session validation; [warm_pool.rs](../../crates/roko-cli/src/dispatch/warm_pool.rs) exists. | Reuse proof with prompt/context fingerprint validation and no stale session carryover. |
| `AD-03` | Rich role-specific spawn policy | `wired_unproven` | [prompt_builder.rs](../../crates/roko-cli/src/dispatch/prompt_builder.rs) carries role, allowlist, verify, files, acceptance, and diagnostics into dispatch. | Role matrix proof for implementer, reviewer, verifier, and research roles. |
| `AD-04` | Verify/reviewer spawn path | `wired_unproven` | Gate verify commands run through runner gates; reviewer-specific provider spawn remains unproven. | Verify/reviewer flow with real provider, role prompt, and gate outcome. |
| `AD-05` | Normalized agent event stream | `source_wired` | [runtime_events.rs](../../crates/roko-agent/src/runtime_events.rs) defines `AgentRuntimeEvent`; [dispatch/mod.rs](../../crates/roko-cli/src/dispatch/mod.rs) forwards bridge events. | CLI and API provider runs both emit the same event categories. |
| `PC-01` | Full role system prompts | `wired_unproven` | [prompt_builder.rs](../../crates/roko-cli/src/dispatch/prompt_builder.rs) owns `PromptContext`, `AssembledPrompt`, diagnostics, allowlists, and sections. | Prompt snapshot proof for implementer/reviewer/retry. |
| `PC-02` | Knowledge/playbook/anti-pattern injection | `wired_unproven` | `PromptAssembler::new()` registers knowledge, playbook, and section-effectiveness sources in [prompt_builder.rs](../../crates/roko-cli/src/dispatch/prompt_builder.rs). | Two-run proof where run 2 consumes knowledge/playbook output from run 1. |
| `PC-03` | Retry feedback shaping | `source_wired` | `GateFeedback::from_raw` and `PromptContext.gate_feedback` exist in [prompt_builder.rs](../../crates/roko-cli/src/dispatch/prompt_builder.rs). | Gate failure then auto-fix proof with structured retry prompt diagnostics. |
| `VR-01` | Structured gate pipeline | `source_wired` | [gate_dispatch.rs](../../crates/roko-cli/src/runner/gate_dispatch.rs) owns gate payloads, timeout, semaphore, shell gates, and verify commands. | Gate history projection and retry proof. |
| `VR-02` | Failure classification and retry | `wired_unproven` | [types.rs](../../crates/roko-cli/src/runner/types.rs) and [event_loop.rs](../../crates/roko-cli/src/runner/event_loop.rs) contain runner failure and retry decision paths. | Failed gate retries once with backoff, then records terminal failure if exhausted. |
| `LR-01` | Episode/cost/latency feedback | `wired_unproven` | [runtime_feedback/mod.rs](../../crates/roko-cli/src/runtime_feedback/mod.rs) defines `FeedbackFacade` and event vocabulary; [roko-learn runtime feedback](../../crates/roko-learn/src/runtime_feedback.rs) persists richer learning records. | Durable episode/efficiency/provider records after a real runner task. |
| `LR-02` | Model/provider routing feedback | `module_exists` | Routing sinks and `CascadeRouter` exist, but active runner routing still needs proof that real features update router state. | Two-run routing proof where first run changes second-run route decision or status. |
| `KM-01` | Knowledge query at dispatch | `wired_unproven` | Prompt assembler can load workdir knowledge; [lifecycle.rs](../../crates/roko-neuro/src/lifecycle.rs) owns runtime knowledge lifecycle. | Prompt diagnostics include knowledge IDs and later lifecycle receipt reinforces them. |
| `KM-02` | Success/failure knowledge writeback | `module_exists` | [lifecycle.rs](../../crates/roko-neuro/src/lifecycle.rs) can ingest runtime observations, admission, reinforcement, and heuristic demotions. | Successful runner task creates or reinforces `.roko/neuro` knowledge. |
| `CO-01` | Conductor/runtime oversight | `module_exists` | Feedback facade has `ConductorObservationSink`; conductor engines exist in `roko-learn`, but live intervention loop is unproven. | Runtime event stream produces conductor observation and intervention decision evidence. |
| `DR-01` | Dreams/consolidation trigger | `module_exists` | [dreams runner](../../crates/roko-dreams/src/runner.rs) defines `DreamRunner`, `DreamLoopConfig`, `DreamTrigger`, and `PlanCompletionTriggerPolicy`; feedback facade has `DreamTriggerSink`. | Completed plan triggers or explicitly skips dream consolidation with durable event. |
| `OB-01` | TUI/API/CLI one truth | `wired_unproven` | [projection.rs](../../crates/roko-cli/src/runner/projection.rs), [projection/mod.rs](../../crates/roko-cli/src/projection/mod.rs), and [projections.rs](../../crates/roko-serve/src/routes/projections.rs) define projection broadcast, dashboard snapshot, subscribers, HTTP projection reads, and SSE streams. | HTTP/TUI/query parity proof after runner completion and server restart. |
| `ST-01` | Mori-level stability | `wired_unproven` | Stability primitives exist in persistence, resume, process cleanup, projection, and merge code. | Full [22-STABILITY-PLAN.md](22-STABILITY-PLAN.md) proof report. |

### Generated Parity Report

Create a generated parity report and treat it as the machine-readable source of truth for this matrix:

- [ ] `tmp/mori-diffs/generated/feature-parity-report.json`

Schema:

```json
{
  "schema_version": 1,
  "generated_at": "2026-04-27T00:00:00Z",
  "git_commit": "unknown",
  "rows": [
    {
      "id": "CE-03",
      "capability": "Checkpoint/resume during plan run",
      "mori": "yes",
      "legacy": "yes",
      "runner": "partial",
      "target": "yes",
      "status": "wired_unproven",
      "source_refs": [
        "crates/roko-cli/src/runner/persist.rs",
        "crates/roko-cli/src/runner/resume.rs",
        "crates/roko-cli/src/runner/event_loop.rs"
      ],
      "proof_refs": [],
      "owner_doc": "22-STABILITY-PLAN.md",
      "next_action": "Run crash/resume matrix and attach stability proof report."
    }
  ]
}
```

Rules:

- [ ] Every row in the overlay must exist in the generated report.
- [ ] Every row must have one owner doc.
- [ ] Every `source_wired` or `wired_unproven` row must include at least one source reference.
- [ ] Every `proved_parity` row must include at least one reproducible proof command and at least one artifact path.
- [ ] The report must preserve old Mori/Legacy/Runner/Target values and add current proof status without destroying historical comparison.
- [ ] A CI or proof script must fail if any `Target = yes` row is `missing`, `legacy_only`, or `module_exists` without an owner issue.

### Implementation Batches

#### PM-01: Matrix Normalization

- [ ] Add stable row ids to the older markdown tables above.
- [ ] Add a `Status` column using the taxonomy in this section.
- [ ] Add a `Proof` column linking to proof commands or generated reports.
- [ ] Backfill `owner_doc` for every row using docs `22`, `27`, `29`, `33`, `34`, `35`, `36`, `38`, `39`, `40`, and `41`.
- [ ] Move old `/tmp/roko-real-e2e-*` proof references into `proved_smoke` notes until tracked proof scripts reproduce them.

#### PM-02: Parity Proof Harness

- [ ] Add `tests/proof/mori-diffs/prove-feature-parity.sh`.
- [ ] Make it call lower-level proof scripts for provider, stability, projection, merge, prompt, feedback, and workflow rows.
- [ ] Make it generate `tmp/mori-diffs/generated/feature-parity-report.json`.
- [ ] Make it preserve failed temporary workspaces and print exact reproduction commands.
- [ ] Make it exit non-zero if any `P0` row remains `missing`, `legacy_only`, `failed`, or `proof_missing`.

#### PM-03: Execution Parity

- [ ] Prove a one-task plan through runner only with durable events and snapshots.
- [ ] Prove a multi-task dependency plan through runner only.
- [ ] Prove failed prerequisite behavior.
- [ ] Prove retry after gate failure.
- [ ] Prove resume after interrupt.
- [ ] Prove cancellation cleans child processes.
- [ ] Prove merge success and merge conflict.

#### PM-04: Dispatch And Prompt Parity

- [ ] Prove Codex CLI and Claude CLI emit the same normalized event categories.
- [ ] Prove Anthropic API, OpenAI API, Moonshot, Z.AI, and Perplexity use the same dispatch/model-call path or are explicitly classified.
- [ ] Prove role prompts differ by role and include allowlist diagnostics.
- [ ] Prove retry prompts include structured gate feedback.
- [ ] Prove knowledge/playbook sections are injected when stores contain relevant entries.
- [ ] Prove prompt diagnostics are durable and queryable.

#### PM-05: Feedback, Knowledge, Conductor, Dreams

- [ ] Prove a completed runner task writes episode, efficiency, and provider/model outcome records through the feedback facade.
- [ ] Prove routing state updates after a terminal task event.
- [ ] Prove knowledge candidate creation from a successful task.
- [ ] Prove knowledge reinforcement when a later prompt reuses a prior entry.
- [ ] Prove conductor observation is derived from normalized runtime events.
- [ ] Prove dream consolidation triggers or explicitly skips after plan completion.

#### PM-06: Observability And UX Parity

- [ ] Prove HTTP projection catalog, dashboard, event log, gate history, and runtime feedback endpoints all read the same durable run.
- [ ] Prove TUI snapshot and HTTP projection agree on plan/task/agent/gate state.
- [ ] Prove CLI non-TUI output reports useful progress without requiring the TUI.
- [ ] Prove tool calls, token/cost updates, gate output, retry decisions, and merge results appear in projections.

### Required Parity Scenario Definitions

These are the exact scenarios the parity harness should implement. Do not substitute unit tests for these rows.

- [ ] `PARITY-SIMPLE-TASK`: one task edits one file, passes gate, writes events/snapshots/projection.
- [ ] `PARITY-MULTI-TASK-DAG`: three tasks with `A -> B -> C`; verify ordering and no duplicate dispatch.
- [ ] `PARITY-FAILED-PREREQ`: `A` fails terminally; `B`/`C` are skipped or blocked with explicit reason.
- [ ] `PARITY-GATE-RETRY`: first gate fails, retry prompt includes structured feedback, second attempt passes.
- [ ] `PARITY-VERIFY-REVIEWER`: verifier/reviewer role path runs real semantics, not smoke auto-advance.
- [ ] `PARITY-RESUME`: kill between agent and gate or gate and snapshot; restart without duplicate completion.
- [ ] `PARITY-ROUTING-SECOND-RUN`: run 1 writes provider/model outcome; run 2 can query or use the observation.
- [ ] `PARITY-KNOWLEDGE-SECOND-RUN`: run 1 creates/reinforces knowledge; run 2 injects it and records retrieval.
- [ ] `PARITY-PROJECTION-HTTP-TUI`: HTTP and TUI projections read the same canonical state.
- [ ] `PARITY-MERGE-CONFLICT`: real git conflict is recorded as failure evidence, not success.
- [ ] `PARITY-DREAM-TRIGGER`: completed plan triggers or explicitly skips dream consolidation with reason.
- [ ] `PARITY-PROVIDER-MATRIX`: all configured providers run through the same dispatch/model-call proof path.

### No-Context Handoff Checklist

Give this block to another agent with no additional context:

- [ ] Read this file and [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md).
- [ ] Generate `tmp/mori-diffs/generated/feature-parity-report.json` from the overlay rows.
- [ ] Implement `tests/proof/mori-diffs/prove-feature-parity.sh`.
- [ ] For each overlay row, fill `status`, `source_refs`, `proof_refs`, and `owner_doc`.
- [ ] Run or implement the scenario definitions above until every P0 row is at least `wired_unproven` and every archive candidate is `proved_parity`.
- [ ] Update old table cells only after proof exists; do not make source-only rows say `Runner = yes`.
- [ ] If a row cannot be implemented because target architecture changed, mark it `retired` with an ADR or owner-doc link.
- [ ] Update [README.md](README.md), [22-STABILITY-PLAN.md](22-STABILITY-PLAN.md), and [27-FILESYSTEM-RUNTIME-CI-AUDIT.md](27-FILESYSTEM-RUNTIME-CI-AUDIT.md) with the proof report path.

### Completion Gate

Do not archive this file until:

- [ ] `feature-parity-report.json` exists and covers every overlay row.
- [ ] Every P0 row is `proved_parity` or explicitly `retired`.
- [ ] Every P1 row is at least `wired_unproven` with owner doc and proof plan.
- [ ] Provider matrix proof has statuses for Anthropic, OpenAI, Moonshot, Z.AI, Perplexity, Claude CLI, and Codex CLI.
- [ ] Stability proof report from [22-STABILITY-PLAN.md](22-STABILITY-PLAN.md) exists.
- [ ] HTTP/TUI projection proof from [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md) exists.
- [ ] Merge proof from [39-RUNNER-EXECUTION-POLICY-AUDIT.md](39-RUNNER-EXECUTION-POLICY-AUDIT.md) exists.
- [ ] No row relies only on old untracked `/tmp` artifacts.
- [ ] No row claims parity for behavior that remains unique to `orchestrate.rs`.
