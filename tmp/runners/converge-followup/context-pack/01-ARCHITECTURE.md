# Architecture

## Crate Layers

```
Layer 0 (kernel):  roko-core          — Signal, 6 verb traits, foundation types, RuntimeEvent
Layer 1 (support): roko-fs, roko-primitives, roko-neuro, roko-dreams, roko-daimon, roko-index
Layer 2 (domain):  roko-agent, roko-gate, roko-compose, roko-learn, roko-orchestrator, roko-conductor
Layer 3 (runtime): roko-runtime       — WorkflowEngine, PipelineState, EffectDriver, JsonlLogger
Layer 4 (surface): roko-cli, roko-serve, roko-agent-server, roko-mcp-*
```

Dependencies flow downward only. Layer 3 uses Layer 0 types for events and contracts.
Layer 4 constructs services from Layer 2 and runs them through Layer 3.

## Key Types

| Type | Crate | Role |
|------|-------|------|
| `RuntimeEvent` | roko-core | Durable event envelope for workflow lifecycle |
| `ModelCallRequest` | roko-core | Full inference request contract |
| `ModelCallResult` | roko-core | Inference response with metadata |
| `AffectPolicy` | roko-core | Dispatch modulation from affect engine |
| `DispatchModulation` | roko-core | Concrete modulation values |
| `FeedbackEvent` | roko-core | Learning event for episodes/metrics |
| `FeedbackSink` | roko-core | Trait for recording feedback |
| `WorkflowEngine` | roko-runtime | Runs workflow pipelines |
| `EffectDriver` | roko-runtime | Executes typed effects (spawn, gate, commit) |
| `PipelineState` | roko-runtime | Workflow phase state machine |
| `ModelCallService` | roko-agent | Inference gateway (routing, cache, budget) |
| `PromptAssemblyService` | roko-compose | Dynamic prompt builder |
| `FeedbackService` | roko-learn | Episode recording, efficiency metrics |
| `GateService` | roko-gate | Gate pipeline execution |

## Intended Flow (v2)

```
Entry point (CLI/server/ACP)
  → ServiceFactory constructs services
  → WorkflowEngine drives PipelineState
  → EffectDriver executes effects:
      → PromptAssemblyService assembles prompt
      → ModelCallService dispatches to provider
      → GateService runs gates
      → FeedbackService records outcomes
  → WorkflowRunReport returned to entry point
  → Entry point uses report for CLI output, sharing, StateHub
```

## Current Reality

- Entry points construct services independently (CLI in `run.rs:480-547`, server in `state.rs:479-482`)
- `EffectDriver` has local policy traits instead of using core foundation
- `WorkflowEngine` emits empty/duplicate lifecycle events
- `JsonlLogger` writes debug strings, `RuntimeProjection` parses them back
- Legacy `run_once`, `dispatch_agent`, and `orchestrate.rs` remain reachable
- `--share` doesn't work on the default v2 path
- Plan execution uses an env-gated adapter inside legacy orchestration
