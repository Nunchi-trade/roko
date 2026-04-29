# Event Log and Conductor

> Depth for [03-GRAPH.md](../../unified/03-GRAPH.md). How execution Pulses flow through Bus, are observed by Conductor Lens Cells, and trigger graduated interventions.

---

## Problem

A Graph executing multiple Flows has no inherent self-awareness. Plans can stall, loop, exhaust budgets, or deadlock without any individual Cell noticing. The system needs:

1. A **Bus** for execution Pulses (event log) -- an append-only, tamper-evident stream of everything that happens
2. A set of **Lens Cells** (watchers) -- Observe-protocol Cells that read the Bus without mutating state
3. A **React Cell** (conductor) -- a meta-cognitive controller that consumes Lens observations and produces interventions

This is the cybernetic pattern: observe-diagnose-intervene. The conductor is the system's homeostatic regulator.

---

## Event Log as Bus

The event log is the Bus implementation for orchestration Pulses. See [snapshot-and-recovery.md](snapshot-and-recovery.md) for the hash chain mechanics. This section focuses on the event log's role as the input stream for the conductor.

### Bus Properties

| Property | How the Event Log Implements It |
|---|---|
| **Ephemeral** | Pulses are sequence-numbered, ring-bufferable. The WatcherRunner only reads the most recent 200. |
| **Ordered** | Monotonically increasing sequence numbers. Concurrent appends are serialized by mutex. |
| **Typed** | 11 EventKinds with structured JSON payloads. |
| **Observable** | Any Lens Cell can read without mutation. The conductor reads every 30 seconds. |
| **Durable subset** | The hash chain provides tamper-evident persistence. Events can graduate to Signals via the episode log. |

The event log straddles Bus and Store: it is ephemeral in that recent events dominate decision-making, but durable in that the full chain is persisted for recovery and audit.

---

## Conductor Architecture

The Conductor is a composite Cell containing three subsystems:

```
Vec<Signal> -----> [ 10 Watchers ] -----> [ DiagnosisEngine ] -----> ConductorDecision
                   (Lens Cells)           (correlator)
                        |
                   [ Circuit Breaker ]
                   (halts cascading failures)
```

### Signal-Based Operation

The conductor does not watch the event log directly. Instead, the `WatcherRunner` periodically (every 30 seconds) reads the most recent signals from `.roko/signals.jsonl` and passes them to the conductor:

```rust
let findings = conductor.check_all(&signals);
```

Alert signals are written back to the signal log, where they become visible to the orchestrator on the next evaluation cycle. This is a publish-subscribe pattern through Store: the conductor reads from Store, writes alerts to Store, and the orchestrator reads alerts from Store.

---

## 10 Watchers (Lens Cells)

Each watcher is a Lens Cell implementing the Observe protocol. It reads recent Signals without mutation, looking for specific anomaly patterns.

| # | Watcher | What It Detects | Emitted Signal |
|---|---|---|---|
| 1 | **Silence detector** | Agent has not produced output for > threshold | `conductor:alert:silence` |
| 2 | **Ghost turn detector** | Agent is looping without progress (output but no state change) | `conductor:alert:ghost_turn` |
| 3 | **Compile failure escalation** | Repeated compilation failures on the same files | `conductor:alert:compile_loop` |
| 4 | **Review loop detector** | Implementation-review-rejection cycle repeated N times | `conductor:alert:review_loop` |
| 5 | **Cost overrun detector** | Cumulative cost exceeds budget threshold | `conductor:alert:cost_overrun` |
| 6 | **Context window pressure** | Token usage approaching model's context limit | `conductor:alert:context_pressure` |
| 7 | **Gate failure rate** | Gate failure rate exceeds threshold per Flow | `conductor:alert:gate_failure_rate` |
| 8 | **Deadlock detector** | Multiple Flows waiting on each other | `conductor:alert:deadlock` |
| 9 | **Resource pressure** | Too many concurrent processes / disk usage | `conductor:alert:resource_pressure` |
| 10 | **Progress stall** | No phase transitions for extended period | `conductor:alert:progress_stall` |

### Cost Monitoring Integration

The `WatcherRunner` builds cost metric Signals from efficiency events:

```rust
fn build_cost_overrun_signals(efficiency_text: &str, budget_usd: f64) -> Vec<Signal> {
    // Sum costs from recent efficiency events
    // Emit: { kind: "Metric", name: "plan_cost", value: "1.234567" }
    // Emit: { kind: "Metric", name: "plan_budget", value: "5.000000" }
}
```

The cost overrun watcher compares plan cost against budget and fires when the threshold is breached.

### Context Window Pressure

Reads the latest efficiency event to extract token usage:

```json
{
    "kind": "TokenUsage",
    "plan_id": "01-workspace",
    "model": "claude-sonnet-4-20250514",
    "tokens_used": "180000"
}
```

High token usage indicates the agent is approaching its context limit, degrading performance. The conductor can intervene by escalating to a larger-context model, splitting the task, or pruning context.

---

## Diagnosis Engine

The `DiagnosisEngine` correlates multiple watcher findings into actionable diagnoses. A single watcher detects a symptom; the diagnosis engine identifies the disease.

Example correlation:

```
Watcher 3: compile_loop (3 consecutive failures on roko-core/src/lib.rs)
Watcher 5: cost_overrun (plan cost $2.34 exceeds 80% of $3.00 budget)

Diagnosis: Plan 01-workspace is stuck in a compile-fix loop and burning budget.
Action: Escalate model from claude-sonnet to claude-opus for the fix task.
```

This is a Compose operation: multiple Lens observations are composed into a single diagnostic Signal that the conductor can act on.

---

## Conductor Decisions

The conductor produces typed decisions:

| Decision | Effect |
|---|---|
| `Continue` | No intervention needed |
| `PausePlan(plan_id)` | Suspend a Flow to reduce load |
| `EscalateModel(plan_id, model)` | Route to a more capable model |
| `ReplanTask(plan_id, task_id)` | Regenerate the task plan |
| `FailPlan(plan_id, reason)` | Mark a Flow as terminally failed |
| `Alert(message)` | Emit an alert for operator attention |

The `PlanRunner` processes decisions by mapping them to executor operations:

```rust
match decision {
    ConductorDecision::PausePlan(id) => executor.pause_plan(&id),
    ConductorDecision::EscalateModel(id, model) => {
        task_tracker.set_task_model_hint(task_id, Some(model))
    }
    ConductorDecision::FailPlan(id, reason) => {
        executor.apply_event(&id, &ExecutorEvent::PlanFailed)
    }
    // ...
}
```

---

## Yerkes-Dodson Dynamics

The Conductor implements Yerkes-Dodson pressure dynamics: moderate pressure maximizes multi-agent cooperation; extreme pressure collapses it.

```
Performance
    ^
    |     *****
    |   **     ***
    |  *          **
    | *              *
    |*                 *
    +--------------------> Arousal
   Low     Optimal    High
```

The Daimon's arousal dimension maps to conductor behavior:

| Arousal | Agent Behavior | Conductor Response |
|---|---|---|
| Low (< 0.3) | Under-stimulated, over-exploring | Increase urgency signals |
| Moderate (0.3 - 0.7) | Optimal zone | No intervention |
| High (> 0.7) | Over-stressed, making errors | Reduce load, pause low-priority Flows |

Pressure adjustment mechanisms:

1. **Pause Flows** -- Reduce concurrent work when agents are struggling
2. **Model escalation** -- Use more capable models for repeatedly failing tasks
3. **Task decomposition** -- Split complex tasks into smaller pieces
4. **Budget reallocation** -- Constrain over-spending Flows, redistribute to productive ones

---

## Circuit Breaker

The circuit breaker prevents cascading failures. When a Flow fails repeatedly, the breaker trips and prevents further dispatch until conditions improve.

This follows the Nygard (2007) circuit breaker pattern:

```
CLOSED (normal) ---[failure threshold exceeded]---> OPEN (tripped)
                                                       |
                                                  [timeout]
                                                       |
                                                       v
                                                 HALF-OPEN (probe)
                                                       |
                                              [probe succeeds]---> CLOSED
                                              [probe fails]------> OPEN
```

In the conductor context:
- **CLOSED**: Normal operation, Flow executes tasks
- **OPEN**: Flow suspended after N consecutive failures, no new dispatches
- **HALF-OPEN**: After a timeout, one probe task is dispatched. Success resets; failure re-trips.

---

## WatcherRunner Background Task

The `WatcherRunner` is the execution harness for the conductor, running as a background Tokio task:

```rust
struct WatcherRunner {
    conductor: Arc<Conductor>,
    signals_path: PathBuf,       // .roko/signals.jsonl
    efficiency_path: PathBuf,    // .roko/learn/efficiency.jsonl
    budget_usd: Option<f64>,
    cancel: TokioCancellationToken,
}
```

Every `WATCHER_INTERVAL_SECS` (30 seconds):

1. Read the most recent `WATCHER_SIGNAL_TAIL` (200) Signals from Store
2. Load efficiency events and build cost metric Signals
3. Build context window pressure Signals from efficiency data
4. Run `conductor.check_all(&signals)`
5. Filter for alert-type Signals
6. Persist alert Signals back to Store

The runner respects the cancellation token for graceful shutdown.

---

## Connection to Viable System Model

The conductor maps to Beer's Viable System Model (1972):

| VSM Level | Roko Component |
|---|---|
| System 1 (Operations) | Individual agents executing tasks |
| System 2 (Coordination) | DAG scheduler, merge queue |
| System 3 (Control) | **Conductor** -- monitors operations, intervenes |
| System 3* (Audit) | Event log hash chain, verification |
| System 4 (Intelligence) | CascadeRouter, learning loops |
| System 5 (Policy) | Gate pipeline, safety contracts |

The conductor is System 3/3*: it maintains a model of the orchestration system (signal patterns, cost trends, progress rates) and intervenes when homeostasis is threatened. Per Conant-Ashby (1970): "every good regulator of a system must be a model of that system."

---

## What This Enables

1. **Self-monitoring execution** -- The system detects its own pathological states (loops, stalls, budget exhaustion) without operator intervention.
2. **Graduated intervention** -- From Continue (no action) through PausePlan to FailPlan, the conductor escalates proportionally to the severity of the anomaly.
3. **Tamper-evident audit** -- The BLAKE3 hash chain makes the execution history forensically verifiable. Any modification to any event breaks the chain.
4. **Pressure regulation** -- Yerkes-Dodson dynamics keep the multi-agent system in its optimal operating zone, preventing both under-utilization and over-stress.

---

## Feedback Loops

1. **Watcher -> Conductor -> Executor -> Watcher**: The conductor observes execution via watchers, produces decisions, the executor applies them (pause plan, escalate model), the resulting behavior changes the signals, and the watchers observe the new state. This is a closed Loop pattern with 30-second tick period.

2. **Cost feedback**: Efficiency events feed cost watchers. Cost overrun triggers model downgrade or plan pause. Reduced spend shows up in subsequent efficiency events. Self-regulating budget control.

3. **Context pressure feedback**: High token usage triggers model escalation (to larger context) or task splitting. The next agent run has more headroom. If the escalation was unnecessary, the cascade router learns to route back down.

4. **Gate failure -> conductor alert -> model escalation -> gate pass**: A compile loop watcher triggers model escalation. The more capable model fixes the compilation error. The success feeds back to the cascade router, recording that this task complexity requires the higher model tier.

---

## Open Questions

1. **Watcher thresholds**: The 10 watchers use hardcoded thresholds (silence timeout, compile failure count, cost percentage). These should be adaptive -- the gate threshold EMA mechanism (`roko-learn`) could be extended to conductor thresholds.

2. **Watcher priority and conflict**: When multiple watchers fire simultaneously with contradictory recommendations (e.g., watcher 5 says "reduce cost" while watcher 10 says "escalate model"), the diagnosis engine must resolve the conflict. The current resolution strategy is not specified.

3. **Conductor latency**: The 30-second tick interval means the conductor can be up to 30 seconds late in detecting anomalies. For fast-failing scenarios (agent crashes immediately), this delay is significant. A hybrid approach -- 30-second periodic check plus immediate notification on critical events -- would reduce latency.

4. **Deadlock detection scope**: Watcher 8 (deadlock detector) checks for Flows waiting on each other. But deadlocks can also occur between the merge queue and the executor (a merge blocks on a file held by a running task that depends on the merge result). Cross-subsystem deadlock detection is not implemented.

5. **Mori parity gap -- error classification**: Mori classified Rust compiler errors (E0432, E0433, E0063, E0308, E0277) into categories with specific remediation strategies. In Roko, this is handled by `roko-gate` error parsing rather than the conductor. The boundary between gate-level error handling and conductor-level pattern detection is not cleanly defined.
