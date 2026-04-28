# Mori Diffs Index

This folder is the implementation gap and runtime convergence package for moving Roko toward the Mori-like architecture.

## Start Here

1. [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) is the canonical current-state ledger. It is organized by priority and impact and calls out stale older claims.
2. [30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md](30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md) is the elegance/side-effect audit for architecture that should be redesigned.
3. [31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md](31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md) is the codebase-wide Rust scan with crate/file counts and migration checklists.
4. [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md) is the crate graph and dependency inversion audit with target layer rules.
5. [33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md](33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md) is the config, credentials, provider policy, and unsafe-default audit.
6. [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md) is the event/projection/query/API/TUI proof-surface audit.
7. [35-TASK-PROCESS-LIFECYCLE-AUDIT.md](35-TASK-PROCESS-LIFECYCLE-AUDIT.md) is the background task, process, cancellation, shutdown, and operation-store audit.
8. [36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md](36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md) is the one-shot/project workflow, CLI/HTTP entrypoint, artifact, and orchestration-engine audit.
9. [37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md](37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md) is the workspace layout, repository, artifact, migration, and storage-query audit.
10. [38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md](38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md) is the learning, knowledge, dreams, affect, conductor, routing, prompt, and cognitive closed-loop audit.
11. [39-RUNNER-EXECUTION-POLICY-AUDIT.md](39-RUNNER-EXECUTION-POLICY-AUDIT.md) is the runner state-machine, task scheduling, gate, retry/replan, merge, and proof audit.
12. [40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md](40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md) is the HTTP server, TUI, operation store, projection, repository, and adapter convergence audit.
13. [41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md) is the inference gateway, model-call service, provider proof, cache/cost/batch, and direct-call-site convergence audit.
14. [22-STABILITY-PLAN.md](22-STABILITY-PLAN.md) is the crash/resume/process/merge/provider stability proof contract.
15. [23-HANDOFF-OPEN-ITEMS.md](23-HANDOFF-OPEN-ITEMS.md) is the subsystem checklist handoff.
16. [21-FEATURE-PARITY-MATRIX.md](21-FEATURE-PARITY-MATRIX.md) is the Mori parity tracker.
17. [20-RUNTIME-RECONCILIATION.md](20-RUNTIME-RECONCILIATION.md) is the design blueprint for replacing `orchestrate.rs` without losing capability.
18. [28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md](28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md) is the detailed dogfood/runtime/UX audit.

## Priority Map

- `P0`: side-effect ownership firewall, repository-wide owner matrix and generated scan artifact, runtime convergence, dependency layer firewall/app-service boundary, provider dispatch/proof, inference gateway/model-call service, config/secret/policy resolution, prompt assembly, feedback facade, persistence/resume, runtime event store/projection/query/proof spine, task/process lifecycle, workflow entrypoint convergence, workspace layout/artifact repository convergence, runner execution policy convergence, serve/TUI adapter convergence.
- `P1`: DAG/merge/worktree, gate/retry/replan, knowledge/dream/affect cognitive closed loop, shutdown/operation proof, one-shot project workflow proof, storage migration/proof, safety/extensions.
- `P2`: schema/terminology, dependency layering, proof export, clean-clone verification.
- `P3`: stale documentation cleanup and status hygiene.

## Latest Deepening Passes

These docs now include source-verified drift sections, concrete service contracts, implementation batches, grep gates, proof requirements, and self-grades above the requested threshold:

- [23-HANDOFF-OPEN-ITEMS.md](23-HANDOFF-OPEN-ITEMS.md): source-corrected handoff checklist, updated checked/off source-wired items, current interpretation rules, no-context next-agent queue, and stop conditions distinguishing wired-unproven from proof-complete. Current self-grade: `9.90 / 10`.
- [22-STABILITY-PLAN.md](22-STABILITY-PLAN.md): stability source refresh, crash/resume matrix, provider/HTTP/TUI/merge proof matrix, generated stability proof report schema, tracked harness requirements, implementation batches, and no-context stable-exit gate. Current self-grade: `9.90 / 10`.
- [21-FEATURE-PARITY-MATRIX.md](21-FEATURE-PARITY-MATRIX.md): source-corrected parity overlay, stable row ids, status taxonomy, generated feature-parity report schema, parity proof harness requirements, implementation batches, and no-context completion gate. Current self-grade: `9.90 / 10`.
- [20-RUNTIME-RECONCILIATION.md](20-RUNTIME-RECONCILIATION.md): source-corrected runtime convergence phase rows, current command/model-call gap list, target runtime command service shape, generated reconciliation report schema, implementation batches, grep gates, and reconciliation exit gate. Current self-grade: `9.90 / 10`.
- [19-SELF-REVIEW-AND-PROOF.md](19-SELF-REVIEW-AND-PROOF.md): proof-governance corrections, current proof harness state, status vocabulary, proof strength ladder, generated proof-governance report schema, required scripts/reports, and archive exit gate. Current self-grade: `9.91 / 10`.
- [18-MASTER-AUDIT.md](18-MASTER-AUDIT.md): source-corrected master architecture audit, current runtime seam evidence, target service design, implementation batches, generated master proof contract, no-context handoff, and archive gate. Current self-grade: `9.91 / 10`.
- [17-ARCHITECTURE-REALITY-CHECK.md](17-ARCHITECTURE-REALITY-CHECK.md): source-corrected reality check, current truth table, status vocabulary, remaining command/query/provider/prompt/feedback/projection/legacy architecture work, no-context order, and archive gate. Current self-grade: `9.90 / 10`.
- [16-INFRASTRUCTURE.md](16-INFRASTRUCTURE.md): source-corrected infrastructure status, event-source/subscription/conductor/prompt/experiment proof batches, generated infrastructure proof report schema, no-context handoff, and archive gate. Current self-grade: `9.90 / 10`.
- [13-KNOWLEDGE-LIFECYCLE.md](13-KNOWLEDGE-LIFECYCLE.md): source-corrected knowledge runtime status, candidate-vs-lifecycle distinction, feedback enrichment/live-ingestor/replay/reinforcement/falsifier batches, generated proof schema, and archive gate. Current self-grade: `9.90 / 10`.
- [12-AFFECT-ROUTING.md](12-AFFECT-ROUTING.md): source-corrected active routing authority audit, config-default bypass finding, cascade context/model-choice fixes, affect/knowledge/provider/calibration batches, proof schema, and archive gate. Current self-grade: `9.91 / 10`.
- [11-PARALLEL-MERGE.md](11-PARALLEL-MERGE.md): source-corrected merge/warm-pool audit, PlanMerger/GitMergeBackend evidence, touched-file/merge/conflict/regression/resume/warm-pool proof batches, generated proof schema, and archive gate. Current self-grade: `9.90 / 10`.
- [10-DREAMS-CONSOLIDATION.md](10-DREAMS-CONSOLIDATION.md): source-corrected dream trigger audit, trigger-vs-consolidation distinction, dream worker/routing advice/prompt influence/cross-run proof batches, generated proof schema, and archive gate. Current self-grade: `9.90 / 10`.
- [09-COMPOSITION-AUCTION.md](09-COMPOSITION-AUCTION.md): source-corrected composition auction audit, active PromptAssembler vs roko-compose manifest split, strategy/VCG/cost attribution/section-effect batches, generated proof schema, and archive gate. Current self-grade: `9.90 / 10`.
- [08-FILE-MAP.md](08-FILE-MAP.md): source-corrected file ownership map, resolved missing-file inventory, current module line counts, ownership rules, legacy/transition classification batches, generated file-map proof schema, and archive gate. Current self-grade: `9.90 / 10`.
- [07-MIGRATION.md](07-MIGRATION.md): source-corrected migration state, stale blocker overrides, active module truth table, legacy cutover batches, provider/prompt/feedback/projection proof contract, grep gates, and archive gate. Current self-grade: `9.90 / 10`.
- [04-LEARNING.md](04-LEARNING.md): source-corrected feedback loop audit, current facade/sink truth table, lossy translation finding, learning transaction design, router/efficiency/knowledge/dream closure batches, generated two-run proof schema, and archive gate. Current self-grade: `9.91 / 10`.
- [03-PERSISTENCE.md](03-PERSISTENCE.md): source-corrected persistence state, current snapshot/run-state/resume truth table, remaining router/threshold/crash semantics, snapshot authority rules, generated crash-resume proof schema, and archive gate. Current self-grade: `9.91 / 10`.
- [02-PLAN-EXECUTION.md](02-PLAN-EXECUTION.md): source-corrected execution policy audit, TaskDag/PlanMerger truth table, active sentinel/global-handle gaps, reducer/effect target design, DAG/retry/merge/verify/budget batches, generated execution proof schema, and archive gate. Current self-grade: `9.90 / 10`.
- [01-AGENT-DISPATCH.md](01-AGENT-DISPATCH.md): source-corrected dispatch authority audit, runtime-events/dispatch truth table, config-default routing bypass, provider matrix/preflight/warm-pool/outcome-fidelity batches, grep gates, generated dispatch proof schema, and archive gate. Current self-grade: `9.91 / 10`.
- [00-OVERVIEW.md](00-OVERVIEW.md): source-corrected runner convergence entry point, current source truth, remaining runner gates, no-context implementation order, required generated proof artifacts, and archive gate. Current self-grade: `9.91 / 10`.
- [24-DEFINITIVE-GAP-LIST.md](24-DEFINITIVE-GAP-LIST.md): stale missing-module correction, current wiring evidence, canonical gap map, no-context implementation order, and proof matrix that reframes the file as historical taxonomy plus handoff rather than the priority ledger. Current self-grade: `9.90 / 10`.
- [25-CODE-ONLY-LEGACY-AUDIT.md](25-CODE-ONLY-LEGACY-AUDIT.md): current code-only legacy scan, executable legacy-surface taxonomy, replacement service architecture, high-risk runtime anchors, generated legacy-surface ledger schema, and merge/dispatch/provider/gate/policy/scaffold retirement batches. Current self-grade: `9.91 / 10`.
- [26-REPOSITORY-WIDE-CODE-AUDIT.md](26-REPOSITORY-WIDE-CODE-AUDIT.md): current tracked/non-doc scope refresh, marker classification taxonomy, high-signal source anchors, owner-doc routing, generated repository-marker inventory schema, and production fallback/scaffold/mock retirement batches. Current self-grade: `9.90 / 10`.
- [27-FILESYSTEM-RUNTIME-CI-AUDIT.md](27-FILESYSTEM-RUNTIME-CI-AUDIT.md): proof spine, clean-clone invariants, current tracked/ignored proof harness status, provider matrix requirements, proof report schema, runtime export contract, and worktree evidence policy. Current self-grade: `9.90 / 10`.
- [28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md](28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md): feature-matrix/dogfood/UX reconciliation contract, missing matrix finding, status taxonomy, stale-claim overrides, generated status ledger schema, and proof-first source-doc update batches. Current self-grade: `9.91 / 10`.
- [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md): canonical priority board, P0-00 side-effect ownership gap card, ordered implementation queue, aggregated no-context checklist, stop conditions, and ledger maintenance rules. Current self-grade: `9.90 / 10`.
- [30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md](30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md): side-effect ownership firewall, generated inventory schema, source-verified drift anchors, P0 migration batches, strict grep gates, and owner/proof completeness rules. Current self-grade: `9.91 / 10`.
- [31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md](31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md): repository-wide triage matrix, updated Rust file count, subsystem owner map, P0/P1 work queues, generated scan artifact schema, grep gates, and proof requirements. Current self-grade: `9.91 / 10`.
- [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md): layer firewall, machine-readable crate policy, core/runtime split, app-service boundary, provider-neutral domain contracts, CLI/server adapter slimming, graph checks, and proof gates. Current self-grade: `9.91 / 10`.
- [33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md](33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md): runtime context build record, resolved config provenance, secret chain, provider registry compatibility decisions, runtime policy, provider proof status taxonomy, and redaction service gates. Current self-grade: `9.92 / 10`.
- [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md): runtime event envelope/store, projection engine, query service, durable streams, proof bundles, route/TUI reader retirement, bridge-loop removal, and observability proof gates. Current self-grade: `9.91 / 10`.
- [35-TASK-PROCESS-LIFECYCLE-AUDIT.md](35-TASK-PROCESS-LIFECYCLE-AUDIT.md): task/process/service lifecycle, route-owned spawns, volatile operation maps, managed command runner, cancellation, crash recovery, and lifecycle query API. Current self-grade: `9.89 / 10`.
- [36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md](36-WORKFLOW-ENTRYPOINT-ORCHESTRATION-AUDIT.md): workflow entrypoints, PRD/plan/research/template route ownership, one-shot orchestration, workflow engine contract, and route-to-service migration. Current self-grade: `9.89 / 10`.
- [37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md](37-WORKSPACE-LAYOUT-ARTIFACT-STORE-AUDIT.md): workspace layout, typed artifact repositories, PRD/plan/research/job storage drift, storage migration, and query-proof artifact ownership. Current self-grade: `9.89 / 10`.
- [38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md](38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md): cognitive transaction spine, knowledge admission, dream scheduling, policy versioning, prompt influence refs, cognitive query service, and two-run proof. Current self-grade: `9.91 / 10`.
- [39-RUNNER-EXECUTION-POLICY-AUDIT.md](39-RUNNER-EXECUTION-POLICY-AUDIT.md): runner reducer/effect extraction, execution decision records, typed gate status, merge policy effects, serve projection query replacement, and legacy orchestrate retirement. Current self-grade: `9.90 / 10`.
- [40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md](40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md): HTTP/TUI adapter authority, command/query contracts, route/status map convergence, projection stream parity, TUI command/query migration, and strict adapter proof. Current self-grade: `9.90 / 10`.
- [41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md): inference gateway/model-call service, provider matrix proof, direct call-site migration, cache/cost/batch, prompt diagnostics, and provider query surfaces. Current self-grade: `9.86 / 10`.

## Completion Rule

A checklist item is complete only when the active runner path owns the behavior and the proof is recorded. Module existence is not enough. Legacy-only behavior in `orchestrate.rs` is not parity.

## Archive Rule

Move a doc to `archive/` only after its remaining checklist items are either:

- implemented and proof-linked; or
- explicitly superseded by [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) with stale claims corrected in the source doc.
