# Conductor Integration

> **Crate**: `roko-conductor`
> **Integration point**: `PlanRunner.conductor` in `roko-cli/src/orchestrate.rs`
> **Background checker**: `WatcherRunner` (tails signals, runs conductor every
> 30 seconds)

---

## Overview

The Conductor is the meta-cognitive controller that monitors the orchestration
pipeline and detects anomalies. While the `ParallelExecutor` drives plans
through phases and the `PlanRunner` dispatches actions, the Conductor watches
the system's behavior over time and intervenes when things go wrong.

The Conductor operates at a different timescale than the executor. The executor
is reactive (process this event, emit this action). The Conductor is reflective
(over the last 30 seconds, are these signals healthy?).

---

## Architecture

```
                 ┌──────────────┐
                 │   Conductor  │
                 │              │
  Vec<Signal> ──►│  10 Watchers │──► ConductorDecision
                 │  DiagnosisEng│
                 │  Circuit Brkr│
                 └──────────────┘
```

The Conductor contains:

- **10 watchers**: Pattern detectors that scan recent signals for anomalies
- **DiagnosisEngine**: Correlates watcher findings into diagnoses
- **Circuit breaker**: Prevents cascading failures by halting problematic plans

---

## Watchers

Each watcher checks for a specific anomaly pattern:

| # | Watcher | Detects | Signal |
|---|---------|---------|--------|
| 1 | Silence detector | Agent hasn't produced output for > threshold | `conductor:alert:silence` |
| 2 | Ghost turn detector | Agent is looping without making progress | `conductor:alert:ghost_turn` |
| 3 | Compile failure escalation | Repeated compilation failures on same files | `conductor:alert:compile_loop` |
| 4 | Review loop detector | Implementation-review-rejection cycle repeated | `conductor:alert:review_loop` |
| 5 | Cost overrun detector | Cumulative cost exceeds budget threshold | `conductor:alert:cost_overrun` |
| 6 | Context window pressure | Token usage approaching model's context limit | `conductor:alert:context_pressure` |
| 7 | Gate failure rate | Gate failure rate exceeds threshold per plan | `conductor:alert:gate_failure_rate` |
| 8 | Deadlock detector | Multiple plans waiting on each other | `conductor:alert:deadlock` |
| 9 | Resource pressure | Too many concurrent processes / disk usage | `conductor:alert:resource_pressure` |
| 10 | Progress stall | No phase transitions for extended period | `conductor:alert:progress_stall` |

### Signal-based operation

Watchers consume `Signal` values from the signal log (`.roko/signals.jsonl`).
The `WatcherRunner` periodically reads the most recent signals and passes
them to the conductor:

```rust
let findings = self.conductor.check_all(&signals);
```

Alert signals are written back to the signal log, where they become visible
to the orchestrator on the next evaluation cycle.

---

## Background Watcher Runner

The `WatcherRunner` runs as a background Tokio task:

```rust
struct WatcherRunner {
    conductor: Arc<Conductor>,
    signals_path: PathBuf,
    efficiency_path: PathBuf,
    budget_usd: Option<f64>,
    cancel: TokioCancellationToken,
}
```

### Operation cycle

Every `WATCHER_INTERVAL_SECS` (30 seconds):

1. Read the most recent `WATCHER_SIGNAL_TAIL` (200) signals from
   `.roko/signals.jsonl`
2. Load efficiency events and build cost metric signals
3. Build context window pressure signals from efficiency data
4. Run `conductor.check_all(&signals)`
5. Filter for alert-type signals
6. Persist alert signals back to the signal log

The watcher respects the cancellation token for graceful shutdown.

---

## Cost Monitoring

Cost monitoring is tightly integrated with the conductor:

### Budget tracking

```rust
fn build_cost_overrun_signals(text: &str, budget_usd: f64) -> Vec<Signal>
```

Sums the cost from recent efficiency events and emits metric signals:

```json
{ "kind": "Metric", "name": "plan_cost", "value": "1.234567" }
{ "kind": "Metric", "name": "plan_budget", "value": "5.000000" }
```

The conductor's cost overrun watcher compares plan cost against budget and
fires `conductor:alert:cost_overrun` when the threshold is breached.

### Context window pressure

```rust
fn build_context_window_pressure_signal(text: &str) -> Option<Signal>
```

Reads the latest efficiency event to extract token usage:

```json
{
  "kind": "TokenUsage",
  "plan_id": "01-workspace",
  "model": "claude-sonnet-4-20250514",
  "tokens_used": "180000"
}
```

High token usage indicates the agent is approaching the model's context window
limit, which degrades performance. The conductor can intervene by:

- Escalating to a model with a larger context window
- Splitting the task into smaller subtasks
- Pruning context to reduce token usage

---

## Yerkes-Dodson Dynamics

The Conductor implements Yerkes-Dodson pressure dynamics as described in
`refactoring-prd/05-agent-types.md`:

> The conductor applies Yerkes-Dodson dynamics: moderate pressure maximizes
> multi-agent cooperation; extreme pressure collapses it.
>
> - Low pressure: agents over-explore (waste budget on low-value tasks)
> - Moderate pressure: optimal — agents focus on high-value work
> - High pressure: agents under-explore (miss opportunities, make errors)

The Daimon's arousal dimension maps to this pressure model:

| Arousal Level | Behavior | Conductor Action |
|---------------|----------|------------------|
| Low (< 0.3) | Agents are under-stimulated, exploring too much | Increase urgency signals |
| Moderate (0.3–0.7) | Optimal zone | No intervention |
| High (> 0.7) | Agents are over-stressed, making errors | Reduce load, pause low-priority plans |

The conductor adjusts pressure by:

1. **Pausing plans**: If too many plans are active and agents are struggling,
   pause lower-priority plans to reduce load
2. **Model escalation**: If an agent is repeatedly failing, escalate to a more
   capable model
3. **Task decomposition**: If a task is too complex, suggest splitting it
4. **Budget reallocation**: If one plan is consuming too much budget, constrain
   it and redistribute

---

## Diagnosis Engine

The `DiagnosisEngine` correlates multiple watcher findings into actionable
diagnoses:

```rust
let findings = conductor.check_all(&signals);
let diagnosis = diagnosis_engine.diagnose(&findings);
```

Example diagnosis:

```
Watcher: compile_loop (3 consecutive failures on roko-core/src/lib.rs)
Watcher: cost_overrun (plan cost $2.34 exceeds 80% of $3.00 budget)
Diagnosis: Plan 01-workspace is stuck in a compile-fix loop and burning budget.
Action: Escalate model from claude-sonnet to claude-opus for the fix task.
```

---

## Conductor Decisions

The conductor produces `ConductorDecision` values:

| Decision | Effect |
|----------|--------|
| `Continue` | No intervention needed |
| `PausePlan(plan_id)` | Pause a plan to reduce load |
| `EscalateModel(plan_id, model)` | Use a more capable model |
| `ReplanTask(plan_id, task_id)` | Regenerate the task plan |
| `FailPlan(plan_id, reason)` | Mark a plan as failed |
| `Alert(message)` | Emit an alert for operator attention |

The `PlanRunner` processes these decisions:

```rust
match decision {
    ConductorDecision::PausePlan(plan_id) => {
        executor.pause_plan(&plan_id)?;
    }
    ConductorDecision::EscalateModel(plan_id, model) => {
        task_tracker.set_task_model_hint(task_id, Some(model))?;
    }
    // ...
}
```

---

## Connection to Mori Resilience Patterns

The conductor system in Roko corresponds to the conductor interventions
described in the Mori resilience documentation
(`bardo-backup/prd/25-mori/mori-resilience.md`):

| Mori Intervention | Roko Equivalent |
|-------------------|-----------------|
| Silence detection | Watcher #1 (silence detector) |
| Ghost turn detection | Watcher #2 (ghost turn detector) |
| Compile failure escalation | Watcher #3 (compile failure escalation) |
| Review loop detection | Watcher #4 (review loop detector) |
| Error classification (E0432/E0433/E0063/E0308/E0277) | Handled by `roko-gate` error parsing |
| Three-tier memory (Episodes→Patterns→Playbook) | `LearningRuntime` in the runtime harness |

The Roko conductor adds cost monitoring, context window pressure, and
Yerkes-Dodson dynamics that were not present in the Mori system.

---

## References

- Yerkes, R. M. & Dodson, J. D. (1908). The relation of strength of stimulus to
  rapidity of habit-formation. *Journal of Comparative Neurology and
  Psychology*, 18(5), 459–482.
- Nygard, M. T. (2007). *Release It! Design and Deploy Production-Ready
  Software*. Pragmatic Bookshelf. (Circuit breaker pattern)
- Beer, S. (1972). *Brain of the Firm: The Managerial Cybernetics of
  Organization*. Allen Lane. (Viable System Model — the conductor is
  System 3/3* in Beer's taxonomy, monitoring operations and intervening
  when homeostasis is threatened)
- Conant, R. C. & Ashby, W. R. (1970). Every good regulator of a system must
  be a model of that system. *International Journal of Systems Science*, 1(2),
  89–97. (The conductor maintains a model of the orchestration system —
  signal patterns, cost trends, progress rates — to regulate it effectively)
