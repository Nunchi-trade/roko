# roko-dreams

Offline consolidation -- replay, imagination, and dream-cycle scheduling.

## What it does

Runs background consolidation cycles that replay past episodes, generate counterfactual
scenarios, rehearse threat responses, and distill insights during idle periods. Implements
a biologically-inspired dream cycle with scheduling policies, budget controls, and a
staging buffer for confidence-gated knowledge promotion.

## Key types and modules

- `DreamRunner` / `DreamEngine` -- top-level dream loop with schedule and budget controls
- `DreamCycle` / `DreamCycleReport` -- single cycle execution and reporting
- `DreamConfig` / `DreamBudget` -- configuration: triggers, intervals, token limits
- `DreamSchedulePolicy` / `DreamHeartbeatPolicy` -- when to trigger dream cycles
- `replay` -- Mattar-Daw prioritized replay: utility-weighted episode selection
- `imagination` -- counterfactual generation: `imagine`, `counterfactual_episode`, `synthesize_hypotheses`
- `hypnagogia` -- liminal state engine: `HypnagogiaEngine`, `ThalamicGate`, `DaliInterrupt`
- `rehearsal` -- threat rehearsal: `rehearse_threats` -> `RehearsalOutcome`
- `staging` -- confidence staging buffer for knowledge promotion
- `threat` -- threat scenario enumeration and warning generation
- `cycle` -- full cycle orchestration with `AgentDispatcher` integration
- `runner` -- runtime controls, heartbeat, and bus-pulse trigger configuration

## Usage

```rust
use roko_dreams::{DreamRunner, DreamConfig, DreamBudget};

let config = DreamConfig {
    budget: DreamBudget { max_tokens: 10_000, max_episodes: 50 },
    ..Default::default()
};
let mut runner = DreamRunner::new(config);
let report = runner.run_cycle(&episodes, &dispatcher).await?;
```

## Architecture

Runs as a background process during agent idle time. Consumes episodes from `roko-learn`,
generates distilled knowledge that feeds into `roko-neuro`, and produces rehearsal outcomes
that update somatic markers in `roko-daimon`. The staging buffer ensures only
high-confidence insights graduate to the durable knowledge store. Phase 2+ feature --
not yet wired into the main orchestration loop.
