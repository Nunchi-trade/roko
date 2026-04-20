# roko-conductor

Reactive intelligence layer -- anomaly detection, intervention, and self-healing.

## What it does

Watches signal streams from running plans and decides when to intervene: restart an agent,
change model, escalate to a human, or abort a plan. Composed of 10 specialized watcher
policies (pure functions over engrams), a circuit breaker for failure budgets, a diagnosis
engine for root-cause classification, and adaptive threshold learning.

## Key types and modules

- `Conductor` -- composite policy that runs all watchers and produces a `ConductorDecision`
- `watchers` -- 10 specialized anomaly detectors, each implementing `Policy`
- `circuit_breaker` -- per-plan failure budget with Holt forecasting for proactive trips
- `interventions` -- severity classification (`Severity`) and escalation via `InterventionPolicy`
- `state_machine` -- plan phase tracking with timeouts and transition records
- `diagnosis` -- `DiagnosisEngine`: error pattern matching and `SuggestedIntervention`
- `stuck_detection` -- `StuckDetector` with meta-cognition hook for self-assessment
- `threshold_learner` -- `AdaptiveThreshold` that learns from intervention outcomes
- `health` -- `HealthMonitor` producing `SystemSnapshot` and `HealthStatus`
- `self_healing` -- recovery strategies: `SelfHealingPolicy` with `HealingAction`
- `federation` -- hierarchical conductor: L1 turn, L2 task, L3 plan, L4 fleet (COND-05)
- `pattern_detector` -- complex event pattern detection with temporal hysteresis (COND-07)
- `yerkes_dodson` -- pressure-performance framework for optimal arousal targeting (COND-04)

## Re-exports from roko-core

- `ConductorDecision` -- the final action: `Continue`, `Retry`, `Escalate`, `Abort`
- `PlanPhase` / `PhaseKind` -- plan execution phase tracking
- `CognitiveSignal` -- signal type for watcher input

## Usage

```rust
use roko_conductor::{Conductor, CircuitBreaker, RoutingBias};

let mut conductor = Conductor::default();
let decision = conductor.evaluate(&engrams, &plan_state);
match decision {
    ConductorDecision::Continue => { /* proceed */ }
    ConductorDecision::Retry { .. } => { /* retry with adjustments */ }
    ConductorDecision::Abort { reason } => { /* stop plan */ }
    _ => {}
}
```

## Architecture

Sits between the orchestrator and agents. The orchestrator feeds engrams (signal snapshots)
to the conductor after each agent turn. Watchers are pure -- no side effects. The conductor
collects their outputs, applies the intervention policy, and returns a single decision that
the orchestrator acts on. The threshold learner closes the feedback loop by adjusting
sensitivity based on whether past interventions helped or hurt.
