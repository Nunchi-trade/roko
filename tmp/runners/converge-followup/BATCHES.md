# Converge Followup Batches

Runner: `tmp/runners/converge-followup/run.sh`
Source: `tmp/subsystem-audits/converge-runner/DEEP-AUDIT.md` (Section 6)

## Wave A — Contract Repair

| Batch | Title | Scope | Deps | Verify |
|-------|-------|-------|------|--------|
| A01 | RuntimeEvent serde envelope | runtime_event.rs | — | quick |
| A02 | Delete runtime-local AffectPolicy | effect_driver.rs, run.rs | — | quick |
| A03 | Extend ModelCallRequest/FeedbackEvent metadata | foundation.rs | A01 | quick |
| A04 | Replace debug-string JSONL with typed JSON | jsonl_logger.rs, projection.rs | A01 | quick |
| A05 | Contract guard tests | runtime/lib.rs | A01-A04 | quick |

## Wave B — Engine Semantics

| Batch | Title | Scope | Deps | Verify |
|-------|-------|-------|------|--------|
| B01 | Agent event ownership — real lifecycle events | workflow_engine.rs, effect_driver.rs | A01,A04 | quick |
| B02 | Apply affect modulation to model request | effect_driver.rs | A02 | quick |
| B03 | Typed gate config — fix shell/custom names | gate_service.rs, util.rs | — | quick |
| B04 | Resume/checkpoint correctness | workflow_engine.rs | A01,B01 | quick |
| B05 | Workflow completion feedback | workflow_engine.rs | A01 | quick |
| B06 | Typed review and commit outcomes | workflow_engine.rs, effect_driver.rs | B01 | quick |

## Wave C — Service Wiring

| Batch | Title | Scope | Deps | Verify |
|-------|-------|-------|------|--------|
| C01 | Shared service factory | run.rs, state.rs | A02,A03 | quick |
| C02 | ModelCallService completeness | model_call_service.rs | C01 | quick |
| C03 | Gateway durability | gateway.rs, gateway_events.rs | C01,C02 | quick |
| C04 | PromptAssemblyService live context | prompt_assembly_service.rs | C01 | quick |
| C05 | Feedback/knowledge provenance | feedback_service.rs | C02,C04 | quick |
| C06 | Cache key order, budget correctness | model_call_service.rs | C02 | quick |

## Wave D — Entry Point Convergence

| Batch | Title | Scope | Deps | Verify |
|-------|-------|-------|------|--------|
| D01 | WorkflowRunReport type | workflow_engine.rs, run.rs | B05,B06 | quick |
| D02 | `--share` parity on v2 | run.rs, util.rs | D01 | quick |
| D03 | Plan execution on v2 engine | orchestrate.rs | D01 | quick |
| D04 | Server workflow event consumers | routes/mod.rs, adapters.rs | D01 | quick |
| D05 | ACP session convergence | shared_runs.rs | D01,C01 | quick |

## Wave E — Legacy Retirement

| Batch | Title | Scope | Deps | Verify |
|-------|-------|-------|------|--------|
| E01 | Feature-gate legacy-orchestrate | cli/lib.rs, Cargo.toml | D03 | quick |
| E02 | Feature-gate direct dispatch | run.rs, dispatch_direct.rs | D02,E01 | quick |
| E03 | Legacy-disabled compile check | Cargo.toml | E01,E02 | full |
| E04 | Legacy-enabled compatibility | Cargo.toml | E03 | full |
| E05 | Delete old prompt builders/parsers | run.rs | E01,D01 | quick |

## Wave F — Proof and Enforcement

| Batch | Title | Scope | Deps | Verify |
|-------|-------|-------|------|--------|
| F01 | Default v2 run integration test | workflow_engine.rs | D01,C01 | full |
| F02 | Share transcript integration test | run.rs | D02 | full |
| F03 | Resume integration test | workflow_engine.rs | B04,D01 | full |
| F04 | Gateway durability test | gateway.rs | C03 | full |
| F05 | Knowledge loop test | feedback_service.rs | C04,C05 | full |
| F06 | Architecture negative CI checks | layer_check.rs | A05 | full |

## Dependency Graph

```
Wave A (contracts):  A01 ──┬── A03 ──┐
                     A02 ──┤         ├── A05
                           └── A04 ──┘

Wave B (engine):     A01,A04 → B01 → B04, B06
                     A02 → B02
                     A01 → B05
                     B03 (no deps)

Wave C (services):   A02,A03 → C01 → C02 → C03, C06
                              C01 → C04
                              C02,C04 → C05

Wave D (entry):      B05,B06 → D01 → D02, D03, D04
                     D01,C01 → D05

Wave E (retire):     D03 → E01 → E02 → E03 → E04
                     E01,D01 → E05

Wave F (proof):      D01,C01 → F01    D02 → F02
                     B04,D01 → F03    C03 → F04
                     C04,C05 → F05    A05 → F06
```
