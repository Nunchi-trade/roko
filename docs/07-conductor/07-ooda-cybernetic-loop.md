# OODA and the Cybernetic Loop

> Observe the signal stream. Orient through watcher analysis.
> Decide via intervention policy. Act through orchestrator commands.
> Every evaluation cycle is one iteration of this loop.

---

## The OODA Framework

Boyd's OODA loop (Observe-Orient-Decide-Act) provides the conceptual
framework for the Conductor's evaluation cycle. Each conductor tick
maps directly to one OODA iteration:

### Observe

The signal stream is the observation input. Every agent turn, gate
result, phase transition, cost event, and timing measurement produces
a Signal that enters the stream. The Conductor reads the stream
without modifying it.

Signals consumed:
- `TokenUsage` — token counts per turn
- `GateVerdict` — gate pass/fail with structured results
- `AgentOutput` — agent turn content
- `PlanPhase` — phase transition events
- `Metric` — numeric measurements (cost, drift, coverage)
- `Custom("conductor.agent_output")` — timing data

The observation phase is pure reading. No state is modified. No
decisions are made.

### Orient

Orientation is where raw observations become assessments. Each watcher
transforms raw signals into structured evaluations:

- Ghost turn watcher: "Agent 7 produced zero output for 3 consecutive turns"
- Cost overrun watcher: "Plan 12 has spent $8.40 of its $10.00 budget"
- Spec drift watcher: "Plan 3 has 32% file changes outside declared scope"

Orientation also includes the stuck detector's meta-cognition
assessment and the diagnosis engine's error classification. Raw
data becomes typed assessments.

The orient phase corresponds to the `check_all()` method — running
all watchers against the signal stream and collecting their outputs.

### Decide

The intervention policy resolves multiple watcher assessments into a
single decision. `WorstSeverityPolicy` selects the maximum severity:

```
Input:  [Warning(compile-fail), Warning(spec-drift)]
Output: ConductorDecision::Restart { reason: "..." }
```

The circuit breaker also participates in the decide phase: a tripped
plan produces `Fail` regardless of watcher outputs.

### Act

The Conductor does not act directly. It returns a `ConductorDecision`
to the orchestrator, which translates it into concrete actions:

| Decision | Orchestrator Action |
|----------|-------------------|
| Continue | Do nothing — proceed with current execution |
| Restart | Kill agent process, prepare error context, spawn fresh agent |
| Fail | Cancel in-flight tasks, mark plan as Failed, move to next plan |

This separation of decision from action is deliberate. The Conductor
has no direct access to processes, files, or agents. It operates
purely on the signal stream and produces pure decisions. The
orchestrator translates decisions into effects.

---

## Cybernetic Structure

The Conductor's evaluation cycle implements a cybernetic feedback loop
in the classical sense (Wiener, 1948):

```
┌──────────────┐     Signals      ┌──────────────┐
│              │ ───────────────→  │              │
│  Execution   │                  │  Conductor   │
│  (Agents,    │                  │  (Watchers,  │
│   Gates,     │  ←───────────── │   Policy,    │
│   Merges)    │   Decision       │   Breaker)   │
│              │                  │              │
└──────────────┘                  └──────────────┘
        │                                 │
        │         Environment             │
        └─────────────────────────────────┘
```

**Sensor**: Signal stream (observes execution state)
**Comparator**: Watchers (compare observed state to thresholds)
**Controller**: Intervention policy (decides corrective action)
**Actuator**: Orchestrator (executes the decision)
**Environment**: Agents + codebase + gates (the system being regulated)

### Negative Feedback

The Conductor implements negative feedback — it acts to reduce
deviation from the desired state. When spec drift exceeds 25%, the
intervention signal pushes the system back toward in-scope work. When
cost exceeds budget, the signal pushes toward termination or restart.
When compile errors repeat, the signal pushes toward a fresh approach.

This is classical homeostatic regulation: the system has a set point
(healthy execution) and corrects deviations.

### Positive Feedback (Absent by Design)

The Conductor does not implement positive feedback — it does not
amplify trends. It does not say "the agent is doing great, give it
more resources." Positive feedback in the conductor domain would risk
runaway behavior: a successful agent getting more context, producing
more output, consuming more tokens, triggering cost overrun.

Positive feedback lives in the learning system instead: successful
model-task combinations get higher reward in the cascade router,
successful patterns promote to playbook rules. The Conductor's job
is stability, not optimization.

---

## Feedback Loop Frequency

The Conductor evaluates at a frequency determined by the orchestrator's
event loop:

**Per-event evaluation**: The orchestrator calls `conductor.evaluate()`
after significant events — agent turn completion, gate result, phase
transition. This is event-driven, not time-driven.

**Periodic health check**: The health monitor runs on a fixed interval
(every 10 seconds), independent of events. This catches infrastructure
problems that do not produce events (e.g., an agent that has silently
died).

**Theta-frequency meta-cognition**: The `MetaCognitionHook` runs at
Theta frequency — less often than per-event, more often than per-phase.
This provides medium-granularity self-assessment without the overhead
of running all stuck detection heuristics on every event.

The three frequencies provide layered coverage:

| Frequency | What Runs | Catches |
|-----------|----------|---------|
| Per-event | All 10 watchers | Task-level anomalies |
| Every 10s | Health monitor | Infrastructure failures |
| Theta | MetaCognitionHook | Stuck agents between events |

---

## Closed-Loop Properties

### Stability

The Conductor's feedback loop is stable because:

1. **Bounded responses**: Every decision is one of three options
   (Continue/Restart/Fail). There is no unbounded escalation.
2. **Cooldown periods**: After firing, each watcher has a cooldown
   before it can fire again for the same plan. This prevents
   oscillation (fire → restart → fire → restart).
3. **Circuit breaker**: After two failures, the plan is permanently
   failed. This prevents infinite retry loops.
4. **Monotonic progress**: Failed plans do not re-enter the pipeline
   automatically. Each restart is a fresh attempt with additional
   information, not a continuation of the failed state.

### Observability

Every decision produces a signal that enters the stream. This means
the Conductor's own behavior is observable:

- Dashboard shows when the conductor intervened and why
- Signal replay can reconstruct every decision
- Learning system records interventions as negative signals
- The conductor's own watchers could theoretically monitor the
  conductor's behavior (second-order meta-cognition)

### Latency

The conductor evaluation cycle adds latency to the orchestrator's
event loop. Measured latency:

| Component | Typical Latency |
|-----------|----------------|
| Circuit breaker check | < 1 μs (DashMap lookup) |
| All 10 watchers | < 1 ms (stream scan, no I/O) |
| Intervention policy | < 1 μs (max comparison) |
| Signal emission | < 10 μs (signal construction) |
| **Total** | **< 2 ms** |

This latency is negligible compared to agent turn times (seconds to
minutes) and gate execution times (seconds to minutes). The conductor
evaluation is never the bottleneck.

---

## Comparison to Other Cybernetic Architectures

### Beer's Viable System Model

The Conductor implements multiple VSM systems:

| VSM System | Function | Roko Component |
|-----------|----------|----------------|
| System 1 | Operations | Individual agents |
| System 2 | Coordination | Shared conventions, templates |
| **System 3** | **Control** | **Conductor (internal regulation)** |
| **System 3*** | **Audit** | **Health monitor (independent check)** |
| System 4 | Intelligence | Learning system (adaptation) |
| System 5 | Policy | Configuration, design principles |

The Conductor is primarily System 3 — it monitors and controls the
internal operations of the agent ensemble. The health monitor adds
System 3* — an independent audit channel that checks the orchestrator's
model against reality.

Reference: Beer, S. (1972). *Brain of the Firm*.

### Conant-Ashby Good Regulator Theorem

"Every good regulator of a system must be a model of that system."
(Conant & Ashby, 1970)

The Conductor models the pipeline through:
- **Watcher thresholds**: model of what "normal" looks like
- **Stuck heuristics**: model of pathological behavior patterns
- **Error categories**: model of failure modes
- **Health checks**: model of infrastructure requirements

This model is currently static (thresholds are constants). The learning
system provides a path to an adaptive model — thresholds that update
based on observed system behavior, becoming a more accurate model
over time.

### Ashby's Law of Requisite Variety

"Only variety can absorb variety." (Ashby, 1956)

The Conductor's regulatory variety is:
- 10 watcher types × configurable thresholds
- 6 stuck heuristics × configurable thresholds
- 20 error categories × 9 intervention types
- 3 severity levels × 3 decision types
- 4 health checks × 3 health statuses

This regulatory variety must match or exceed the variety of the
system being regulated. If the agent ensemble can fail in more
distinct ways than the Conductor can detect, some failures will go
unregulated.

The modular architecture supports variety expansion: adding a new
watcher adds a new detection dimension. Adding a new error pattern
adds a new classification. The system's regulatory variety grows as
new failure modes are cataloged.

---

## File Reference

| File | What |
|------|------|
| `crates/roko-conductor/src/conductor.rs` | The OODA loop implementation (evaluate()) |
| `crates/roko-conductor/src/stuck_detection.rs` | MetaCognitionHook (Theta-frequency assessment) |
| `crates/roko-conductor/src/health.rs` | Health monitor (periodic infrastructure check) |
| `crates/roko-conductor/src/interventions.rs` | Decision resolution (Orient → Decide) |
