# Architecture Runner — Batch Manifest

16 batches across 5 phases. Each batch produces one or two new files plus a `mod` declaration.

## Phase 0: Core Types & Traits

| Batch | Title | Write Scope | Verify | Deps |
|-------|-------|-------------|--------|------|
| P0A | RuntimeEvent types | `roko-core/src/runtime_event.rs` (new), `roko-core/src/lib.rs` (mod decl) | `cargo check -p roko-core` | — |
| P0B | Foundation traits (6 traits) | `roko-core/src/foundation.rs` (new), `roko-core/src/lib.rs` | `cargo check -p roko-core` | P0A |
| P0C | EventBus RuntimeEvent support | `roko-runtime/src/event_bus.rs` (extend), `roko-runtime/src/lib.rs` | `cargo check -p roko-runtime` | P0A |

## Phase 1: Foundation Services

| Batch | Title | Write Scope | Verify | Deps |
|-------|-------|-------------|--------|------|
| P1A | ModelCallService | `roko-agent/src/model_call_service.rs` (new), `roko-agent/src/lib.rs` | `cargo check -p roko-agent` | P0A, P0B |
| P1B | PromptAssemblyService | `roko-compose/src/prompt_assembly_service.rs` (new), `roko-compose/src/lib.rs` | `cargo check -p roko-compose` | P0A, P0B |
| P1C | FeedbackService | `roko-learn/src/feedback_service.rs` (new), `roko-learn/src/lib.rs` | `cargo check -p roko-learn` | P0A, P0B |
| P1D | GateService | `roko-gate/src/gate_service.rs` (new), `roko-gate/src/lib.rs` | `cargo check -p roko-gate` | P0A, P0B |

## Phase 2: Execution Engine

| Batch | Title | Write Scope | Verify | Deps |
|-------|-------|-------------|--------|------|
| P2A | PipelineState v2 | `roko-runtime/src/pipeline_state.rs` (new), `roko-runtime/src/lib.rs` | `cargo check + test` | P0A |
| P2B | TaskScheduler | `roko-runtime/src/task_scheduler.rs` (new), `roko-runtime/src/lib.rs` | `cargo check + test` | — |
| P2C | EffectDriver | `roko-runtime/src/effect_driver.rs` (new), `roko-runtime/src/lib.rs` | `cargo check` | P1A-D, P2A, P2B |
| P2D | WorkflowEngine facade | `roko-runtime/src/workflow_engine.rs` (new), `roko-runtime/src/lib.rs` | `cargo check` | P2C |

## Phase 3: Adapters

| Batch | Title | Write Scope | Verify | Deps |
|-------|-------|-------------|--------|------|
| P3A | AcpAdapter | `roko-acp/src/acp_adapter.rs` (new), `roko-acp/src/lib.rs` | `cargo check -p roko-acp` | P0B, P0C |
| P3B | SseAdapter + REST | `roko-serve/src/adapters.rs` (new), `roko-serve/src/routes/` | `cargo check -p roko-serve` | P0B, P0C |
| P3C | JsonlLogger + Projection | `roko-runtime/src/jsonl_logger.rs` (new), `roko-runtime/src/projection.rs` (new) | `cargo check -p roko-runtime` | P0B, P0C |

## Phase 4: Wiring

| Batch | Title | Write Scope | Verify | Deps |
|-------|-------|-------------|--------|------|
| P4A | Wire CLI entry points | `roko-cli/src/run.rs`, `roko-cli/src/plan_cmd.rs` | `cargo check -p roko-cli` | P2D |
| P4B | Wire ACP entry points | `roko-acp/src/bridge_events.rs`, `roko-acp/src/runner.rs` | `cargo check -p roko-acp` | P2D, P3A |

## Dependency DAG

```
P0A ──→ P0B ──→ P1A ──→ P2C ──→ P2D ──→ P4A
  │       │       │       ↑       ↑       │
  │       │     P1B ──────┤       │       └→ P4B
  │       │     P1C ──────┤       │           ↑
  │       │     P1D ──────┤       │           │
  │       │               │       │           │
  ├──→ P0C ──→ P3A ──────│───────│───────────┘
  │       │     P3B       │       │
  │       │     P3C       │       │
  │       │               │       │
  └──→ P2A ───────────────┘       │
                                  │
P2B ──────────────────────────────┘
```

## Verification Levels

Each batch is verified at 3 levels:

1. **Structural** — `grep` checks that expected structs/traits/modules exist
2. **Compilation** — `cargo check -p <crate>` (and `cargo test` where noted)
3. **Anti-pattern** — `grep` checks that banned patterns are NOT present
