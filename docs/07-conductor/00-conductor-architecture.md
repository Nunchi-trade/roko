# Conductor Architecture

> The Conductor is not a timeout manager. It is the agent's theory of mind
> about its own pipeline — the subsystem that watches execution unfold
> and asks: is this going where I predicted?

---

## Position in the Five-Layer Architecture

Roko's runtime stacks into five layers. Each layer has a distinct
responsibility boundary:

| Layer | Name | What It Owns | Key Traits |
|-------|------|-------------|------------|
| L0 | Runtime | Processes, I/O, OS-level lifecycle | `Substrate` |
| L1 | Framework | Tool definitions, agent capabilities | (tools API) |
| L2 | Scaffold | Prompt construction, context engineering | `Composer` |
| **L3** | **Harness** | **Output evaluation, meta-cognition** | **`Gate`, `Policy`** |
| L4 | Orchestration | Multi-agent scheduling, DAG execution | `Router`, `Scheduler` |

The Conductor sits at **Layer 3 — Harness**. It shares this layer with
the gate pipeline (compile, test, clippy, diff, coverage, spec, etc.)
but serves a fundamentally different function:

- **Gates** answer: did the output meet the acceptance criteria?
- **Conductor** answers: is the process itself healthy?

Gates evaluate artifacts. The Conductor evaluates trajectories.

This distinction matters because a plan can pass every individual gate
and still be pathological — looping through identical implement-gate
cycles, burning tokens on ghost turns, or drifting outside its declared
file scope without any single gate catching it.

---

## Synapse Architecture Placement

Roko's kernel defines one noun (`Signal`) and six verb traits:

```
Substrate — storage and I/O
Scorer    — numeric evaluation
Gate      — binary accept/reject
Router    — selection among alternatives
Composer  — prompt assembly
Policy    — reactive stream evaluation
```

The Conductor is a **composite `Policy`**. Every watcher implements the
`Policy` trait. The Conductor itself also implements `Policy`, delegating
to its inner watchers and aggregating their outputs through an
intervention policy.

```rust
// From crates/roko-conductor/src/conductor.rs
pub struct Conductor {
    watchers: Vec<Box<dyn Policy>>,
    policy: Box<dyn InterventionPolicy>,
    circuit_breaker: CircuitBreaker,
}

impl Policy for Conductor {
    fn decide(&self, stream: &[Signal], ctx: &Context) -> Vec<Signal> {
        // 1. Check circuit breaker — tripped plans get Failed immediately
        // 2. Run all watchers — collect WatcherOutputs
        // 3. Apply intervention policy — worst severity wins
        // 4. Record failures to circuit breaker
        // ...
    }
}
```

This composability means the Conductor can be used anywhere a `Policy`
is expected: inside the orchestrator's main loop, as a standalone
evaluation pass, or nested inside a larger policy composition.

---

## What the Conductor Is Not

Understanding the Conductor requires understanding what it deliberately
does not do:

**It is not a scheduler.** The Conductor does not decide which task runs
next, which agent gets spawned, or how resources are allocated. That is
L4 (Orchestration). The Conductor evaluates whether the current
execution trajectory is healthy and emits signals when it is not.

**It is not a gate.** Gates produce binary verdicts (pass/fail) on
artifacts. The Conductor produces graduated interventions (continue /
restart / fail) on processes. A gate looks at the code; the Conductor
looks at the agent producing the code.

**It is not a timeout manager.** Timeouts are one of ten watcher
categories. The Conductor's scope includes loop detection, cost
monitoring, context pressure tracking, spec drift measurement, test
regression detection, and review cycle analysis. Reducing it to
"timeouts" misses 90% of its function.

**It does not nudge.** A nudge is "please fix yourself" — which does not
work on confused agents. The Conductor has exactly three actions:
Continue (everything is fine), Restart (kill and restart with different
context), or Fail (mark the plan as failed). There is no "try harder."
This is a deliberate design decision derived from production experience:
agents that are stuck remain stuck after nudges (§6, Hard Guarantee 6
from the failure prevention catalog).

---

## Core Components

The Conductor comprises seven subsystems, each in its own module:

### 1. Watcher Ensemble (`watchers/`)

Ten watchers, each implementing `Policy`. Each watcher monitors a
specific failure mode by examining the signal stream:

| Watcher | Module | What It Detects |
|---------|--------|----------------|
| Ghost Turn | `ghost_turn.rs` | Agent turns with zero meaningful output |
| Compile Fail Repeat | `compile_fail_repeat.rs` | Identical compile errors repeating |
| Cost Overrun | `cost_overrun.rs` | Plan cost exceeding budget |
| Iteration Loop | `iteration_loop.rs` | Repeated gate-fail retry cycles |
| Review Loop | `review_loop.rs` | Repeated review rejects without progress |
| Spec Drift | `spec_drift.rs` | File edits outside declared scope |
| Stuck Pattern | `stuck_pattern.rs` | Repeated identical agent actions |
| Test Failure Budget | `test_failure_budget.rs` | Test failure count increasing |
| Time Overrun | `time_overrun.rs` | Task approaching timeout threshold |
| Context Window Pressure | `context_window_pressure.rs` | Token usage exceeding context limits |

Each watcher operates independently. They share no state with each
other. They read the signal stream, apply their detection logic, and
either return empty (healthy) or return intervention signals.

### 2. Circuit Breaker (`circuit_breaker.rs`)

Per-plan failure budget tracking. Uses `DashMap` for thread-safe
concurrent access. A plan that accumulates `MAX_PLAN_FAILURES` (default
2) failures is permanently tripped — no further retries.

```rust
pub struct CircuitBreaker {
    failures: DashMap<String, FailureRecord>,
}
```

This prevents the pathological case where a fundamentally broken plan
cycles through retry after retry, burning tokens on every attempt. Two
failures is the budget. After that, the plan requires human attention.

### 3. Intervention Policy (`interventions.rs`)

Maps watcher outputs to conductor decisions through a severity system:

```
Info     → ConductorDecision::Continue
Warning  → ConductorDecision::Restart
Critical → ConductorDecision::Fail
```

The default policy is `WorstSeverityPolicy`: the highest severity among
all watcher outputs determines the decision. If nine watchers say
"continue" and one says "critical," the decision is Fail.

### 4. Diagnosis Engine (`diagnosis.rs`)

Thirty-four built-in error patterns covering twenty error categories.
Given raw error output (compiler messages, test output, agent logs), the
diagnosis engine classifies the error, assigns a confidence score, and
suggests an intervention:

```rust
pub enum SuggestedIntervention {
    RetryWithContext,
    AutoFix,
    RestartAgent,
    AbortPlan,
    BackoffRetry,
    MergeResolution,
    ReduceContext,
    SwitchModel,
    WarnAndContinue,
}
```

This structured classification replaces ad-hoc error parsing. Instead of
grepping for "error[E0308]" in raw output, the diagnosis engine returns
a typed `Diagnosis` with category, confidence, affected files, and
suggested action.

### 5. Stuck Detection (`stuck_detection.rs`)

Six heuristics for detecting stuck agents:

- **OutputLoop**: Agent producing identical output across turns
- **NoProgress**: No file changes within a time window
- **GateLoop**: Gate failures repeating without change
- **CompileLoop**: Same compile errors repeating
- **EmptyOutput**: Turns with no meaningful content
- **ExcessiveRetries**: Too many retry attempts

The `StuckDetector` operates at configurable thresholds. The
`MetaCognitionHook` wraps it for periodic self-assessment at Theta
frequency: "Am I stuck? Am I thrashing? Should I escalate?"

### 6. Health Monitor (`health.rs`)

Four system-level health checks producing a `HealthStatus` (Healthy /
Degraded / Critical):

- **terminal_liveness**: Is the agent process still responsive?
- **agent_status**: Are expected agents running?
- **spec_drift**: Has the implementation diverged from specification?
- **coverage_trend**: Is test coverage trending down?

The health monitor operates on `SystemSnapshot` — a point-in-time view
of system state including active agent count, heartbeat recency, spec
hash comparison, and coverage history.

### 7. State Machine (`state_machine.rs`)

Phase timeout configuration by plan complexity:

| Phase | Complex | Standard | Fast |
|-------|---------|----------|------|
| Implementing | 600s | 300s | 120s |
| Gating | 300s | 300s | 300s |
| Reviewing | 300s | 300s | 300s |
| Merging | 60s | 60s | 60s |

`PhaseTransition` records capture the plan ID, source phase, target
phase, timestamp, and reason — providing a complete audit trail of every
plan's progression through the pipeline.

---

## Evaluation Flow

When the orchestrator calls `conductor.evaluate()`, the following
sequence executes:

```
1. Circuit breaker check
   └─ If plan is tripped → return Fail immediately

2. Run all 10 watchers against the signal stream
   └─ Each watcher returns Vec<Signal> (empty = healthy)
   └─ Collect all non-empty results as WatcherOutputs

3. Apply intervention policy
   └─ WorstSeverityPolicy: max(all severities) → decision
   └─ Info → Continue, Warning → Restart, Critical → Fail

4. If decision is Restart or Fail:
   └─ Record failure in circuit breaker
   └─ Emit intervention signal to stream

5. Return ConductorDecision
```

The entire evaluation is stateless from the Conductor's perspective —
it reads the signal stream and produces a decision. State tracking
(failure counts, circuit breaker trips) lives in the `CircuitBreaker`,
which uses thread-safe `DashMap` for concurrent access.

---

## Signal Flow

The Conductor communicates exclusively through signals. It reads
`Signal` instances from the stream and writes `Signal` instances back:

**Input signals consumed:**

| Kind | What the Conductor Reads |
|------|------------------------|
| `TokenUsage` | Token counts for context pressure |
| `GateVerdict` | Test results for failure budget |
| `AgentOutput` | Output content for ghost turn / stuck detection |
| `PlanPhase` | Phase events for review loop tracking |
| `Metric` (name=spec_drift) | Drift ratios for spec drift |
| `Custom("conductor.agent_output")` | Timing data for time overrun |

**Output signals emitted:**

| Kind | When |
|------|------|
| `Custom("conductor.intervention")` | Any watcher fires |

Intervention signals carry tags: `watcher` (which watcher fired),
`severity` (info/warning/critical), and watcher-specific metadata
(ratio, count, plan_id, task_id, etc.).

---

## Design Decisions

### Why Watchers Are Policies, Not Gates

Gates produce binary verdicts. Policies produce signals with graduated
severity. The Conductor needs graduation because not every anomaly
warrants the same response:

- Context window at 82% → warning (restart with compacted context)
- Spec drift at 30% → warning (the agent is exploring nearby files)
- Three identical compile errors → critical (the agent is stuck)

A gate would reduce all of these to "fail," losing the information
needed for appropriate response.

### Why Ten Watchers Instead of One Smart Monitor

Each watcher is a focused detector for one failure mode. This
decomposition provides:

1. **Testability** — each watcher has isolated unit tests
2. **Configurability** — thresholds are per-watcher
3. **Composability** — add or remove watchers without touching others
4. **Diagnosability** — the intervention signal says which watcher fired

A monolithic monitor would conflate detection with diagnosis. By keeping
watchers separate, the system can tell you not just "something is wrong"
but "the agent has produced three identical compile errors" — a
much more actionable signal.

### Why the Circuit Breaker is Per-Plan

Plans are the unit of retry. A failing plan should not poison other
plans. The circuit breaker tracks failures per plan ID, so plan A
hitting its failure budget does not affect plan B.

The `DashMap` provides thread-safe concurrent access because the
orchestrator may evaluate multiple plans in parallel.

---

## References

- Conant & Ashby (1970) — "Every good regulator of a system must be a
  model of that system." The Conductor models the pipeline's health.
- Beer (1972) — Viable System Model, System 3 (internal oversight) +
  System 3* (audit). The Conductor fills both roles.
- Boyd — OODA loop (Observe-Orient-Decide-Act). Each conductor
  evaluation cycle is one OODA iteration.

---

## File Reference

| File | What |
|------|------|
| `crates/roko-conductor/src/lib.rs` | Module structure, re-exports |
| `crates/roko-conductor/src/conductor.rs` | Conductor struct, evaluate(), Policy impl |
| `crates/roko-conductor/src/circuit_breaker.rs` | Per-plan failure tracking |
| `crates/roko-conductor/src/interventions.rs` | Severity, WatcherOutput, InterventionPolicy |
| `crates/roko-conductor/src/diagnosis.rs` | 34 error patterns, 20 categories |
| `crates/roko-conductor/src/health.rs` | SystemSnapshot, 4 health checks |
| `crates/roko-conductor/src/state_machine.rs` | Phase timeouts, PhaseTransition records |
| `crates/roko-conductor/src/stuck_detection.rs` | 6 stuck heuristics, MetaCognitionHook |
| `crates/roko-conductor/src/watchers/` | 10 watcher modules |
