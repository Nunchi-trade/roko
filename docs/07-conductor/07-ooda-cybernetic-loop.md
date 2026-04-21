# OODA and the Cybernetic Loop

> Observe the signal stream. Orient through watcher analysis.
> Decide via intervention policy. Act through orchestrator commands.
> Every evaluation cycle is one iteration of this loop.


> **Implementation**: Built

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

---

## OODA Loop Speed Optimization

### Boyd's key insight: tempo, not speed

Boyd's central argument was not "cycle faster." It was "get inside the
adversary's decision loop." For the Conductor, the adversary is drift —
the gap between what agents are doing and what they should be doing. The
goal is to update the world model faster than agent behavior can diverge
from the plan.

"Operating inside the loop" means the Conductor detects and corrects a
deviation before the agent's next action compounds it. If an agent
produces a ghost turn and the Conductor restarts it before the next turn
fires, the loop stays tight. If three ghost turns accumulate before
detection, the Conductor is operating outside the loop — reacting to
damage rather than preventing it.

Tempo is relational. A Conductor that evaluates every 500 ms is
overbuilt if agents produce turns every 30 seconds. A Conductor that
evaluates every 10 seconds is underbuilt if agents produce turns every
2 seconds. The right tempo is one evaluation per event, with periodic
health checks to cover silent failures.

### What determines conductor cycle time

The Conductor's per-evaluation latency breaks down as follows:

| Component | Typical Latency | Optimization |
|-----------|----------------|--------------|
| Signal stream scan (10 watchers) | < 1 ms | Already optimal — pure in-memory scan |
| Circuit breaker lookup (DashMap) | < 1 us | Already optimal — lock-free concurrent map |
| Intervention policy resolution | < 1 us | Already optimal — max comparison |
| Anomaly detector check | < 0.1 ms | EWMA update is O(1) |
| Health monitor snapshot | ~10 ms | Periodic, not on critical path |
| MetaCognition assessment | ~1 ms | Theta frequency, not every turn |
| **Total per-evaluation** | **< 2 ms** | **Negligible vs agent turn times** |

At < 2 ms per evaluation, the Conductor adds negligible overhead to any
agent turn. The evaluation itself is never the bottleneck. The bottleneck
is upstream: when do observations arrive?

### The real bottleneck: observation latency

The Conductor evaluates fast, but it can only evaluate what it can see.
Observations arrive when events are produced — and an agent stuck in a
long reasoning chain produces no events. Between the start of an agent
turn and the completion of that turn, the Conductor is blind.

The per-10s health monitor is the only mechanism that detects this gap.
It checks infrastructure status independent of the event stream. But
10 seconds is a coarse interval. An agent that hangs for 9 seconds
gets no detection until the next health tick.

A dedicated liveness monitor would close this gap:

```rust
/// Heartbeat-based liveness detection for agents between events.
/// Detects stuck agents that produce no observable signals.
pub struct LivenessMonitor {
    /// Expected heartbeat interval per agent (default: 30s).
    expected_interval: Duration,
    /// Last heartbeat timestamp per agent.
    last_heartbeat: DashMap<String, Instant>,
    /// Warning threshold: fire warning at this multiple of expected_interval.
    warning_multiplier: f64,  // default: 2.0 (60s for 30s interval)
    /// Critical threshold: fire critical at this multiple.
    critical_multiplier: f64, // default: 5.0 (150s for 30s interval)
}
```

The liveness monitor runs on its own timer. If an agent's last heartbeat
exceeds `expected_interval * warning_multiplier`, it emits a warning
signal into the stream. If it exceeds `critical_multiplier`, it emits
a critical signal. This gives the Conductor visibility into the gap
between events without requiring agents to change their behavior —
heartbeats are emitted by the process supervisor, not by agents
themselves.

### Implicit Guidance and Control (IG&C)

Boyd described a shortcut in the OODA loop: when the Orient phase
recognizes a well-known pattern, it can bypass the full Decide phase
and jump straight to Act. He called this Implicit Guidance and Control.
Klein's Recognition-Primed Decision model (1998) documents the same
phenomenon in human experts — experienced firefighters do not deliberate
over options; they recognize the situation and act.

For the Conductor, IG&C means pre-compiled rules for common failure
patterns:

```rust
/// Pre-compiled action rules for known patterns (Boyd's IG&C shortcut).
/// Bypasses full watcher evaluation for well-understood failure modes.
pub struct ImplicitGuidance {
    /// Map from recognized pattern fingerprint to pre-computed action.
    rules: Vec<ImplicitRule>,
}

pub struct ImplicitRule {
    /// Pattern name for logging/observability.
    pub name: &'static str,
    /// Fast check: does this pattern match the current signal stream?
    pub matcher: Box<dyn Fn(&[Engram]) -> bool + Send + Sync>,
    /// Pre-computed action to take when the pattern matches.
    pub action: ConductorDecision,
    /// Minimum confidence from bandit training before this rule activates.
    pub min_confidence: f64,
}
```

When `ImplicitGuidance` matches the current signal stream, the Conductor
skips full watcher evaluation and returns the pre-computed decision.
This is faster (sub-microsecond), but more importantly it encodes
institutional knowledge: patterns the system has seen before and knows
how to handle.

IG&C rules should not be hand-written. They should be extracted from the
ConductorBandit's converged actions. When a bandit arm converges to >95%
selection rate for a given failure pattern, that pattern graduates to an
IG&C rule. The bandit continues to explore; the IG&C rule handles the
common case.

---

## Nested OODA loops — multi-timescale control

A single OODA loop is insufficient for a system that operates across
multiple timescales. Agent turns happen in seconds, tasks take minutes,
plans run for hours. A single loop tuned for seconds would generate
excessive churn at the plan level. A single loop tuned for hours would
miss per-turn anomalies.

The solution is nested loops, each operating at its own frequency.

### Three-level nesting

Roko's cognitive frequencies map to three nested OODA loops:

```
┌─────────────────────────────────────────────────────┐
│  Delta Loop (Strategic)                              │
│  Period: per-batch (hours)                           │
│  Orient: cross-plan patterns, model effectiveness    │
│  Decide: cascade router updates, threshold tuning    │
│  Act:    policy changes for next batch               │
│                                                      │
│  ┌─────────────────────────────────────────────┐    │
│  │  Theta Loop (Operational)                    │    │
│  │  Period: per-task (minutes)                   │    │
│  │  Orient: MetaCognitionHook assessment         │    │
│  │  Decide: strategy adjustment, escalation      │    │
│  │  Act:    restart agent, switch model           │    │
│  │                                                │    │
│  │  ┌───────────────────────────────────────┐   │    │
│  │  │  Gamma Loop (Tactical)                 │   │    │
│  │  │  Period: per-turn (seconds)             │   │    │
│  │  │  Orient: all 10 watchers                │   │    │
│  │  │  Decide: intervention policy            │   │    │
│  │  │  Act:    continue/restart/fail           │   │    │
│  │  └───────────────────────────────────────┘   │    │
│  └─────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────┘
```

The innermost loop (Gamma) handles per-turn tactical decisions: is this
agent turn healthy? The middle loop (Theta) handles per-task operational
decisions: is this task making progress? The outer loop (Delta) handles
per-batch strategic decisions: is the system learning and improving?

### Separation of concerns

Each loop has its own orientation model — different data sources,
different scopes, different update frequencies:

| Loop | Orientation Source | Model Scope | Update Frequency |
|------|-------------------|-------------|-----------------|
| Gamma | Watcher thresholds (static constants) | Single agent turn | Every turn |
| Theta | MetaCognition assessment + stuck heuristics | Task trajectory | Every 3-5 turns |
| Delta | Efficiency events + cascade router observations | Cross-plan patterns | Per batch |

The Gamma loop does not know about cross-plan patterns. The Delta loop
does not know about individual agent turns. This is not a limitation —
it is the design. Each loop sees only what it needs at its timescale.

### Parameter cascade

Slower loops set the parameters for faster loops. This is the
foundational principle from hierarchical control theory (Mesarovic
et al. 1970): the slower controller sets the frame within which the
faster controller operates.

In Roko:
- **Delta sets Theta parameters**: adaptive gate thresholds, default
  model tier, cost budgets per task
- **Theta sets Gamma parameters**: adjusted watcher thresholds based
  on meta-cognition assessment, intervention cooldown periods

```rust
/// Hierarchical parameter cascade: slower loops configure faster loops.
pub struct ParameterCascade {
    /// Delta-level parameters (updated per batch).
    pub delta: DeltaParameters,
    /// Theta-level parameters (updated per task).
    pub theta: ThetaParameters,
    /// Gamma-level parameters (used per turn, set by Theta).
    pub gamma: GammaParameters,
}

pub struct DeltaParameters {
    pub default_model_tier: ModelTier,
    pub base_cost_budget_usd: f64,
    pub gate_threshold_adjustments: HashMap<String, f64>,
}

pub struct ThetaParameters {
    pub adjusted_stuck_threshold: usize,
    pub adjusted_ghost_turn_max: usize,
    pub current_pressure_level: f64,  // 0.0 to 1.0
}

pub struct GammaParameters {
    pub watcher_thresholds: WatcherThresholds,
    pub intervention_cooldown: Duration,
}
```

The cascade flows one direction: slow to fast. The Gamma loop never
modifies Delta parameters. If the Gamma loop detects something that
requires a strategic response, it emits a signal into the stream. The
Delta loop picks it up on its next evaluation — at its own pace.

### Singular perturbation principle

When timescales are well-separated, each loop can be analyzed
independently. This is the singular perturbation result from control
theory: if the fast loop reaches steady state before the slow loop
takes its next step, the two loops decouple mathematically.

In Roko:
- Gamma runs every ~5 seconds (per agent turn)
- Theta runs every ~75 seconds (per task, roughly every 15 Gamma cycles)
- Delta runs every ~hours (per batch, roughly 50-100 Theta cycles)

The ~15x separation between adjacent levels is sufficient for
quasi-static decoupling. The Gamma loop treats Theta parameters as
constants — they change slowly relative to per-turn evaluation. The
Theta loop assumes the Gamma loop has reached its steady-state
decision for the current watcher outputs.

This separation is what makes the hierarchical architecture tractable.
Without it, every parameter change at every level would interact with
every other level, producing a combinatorial analysis problem. With it,
each level can be understood, tuned, and debugged in isolation.

---

## Algedonic signals — priority interrupts

### Definition

Algedonic signals are pain/pleasure signals that bypass the normal
management hierarchy, going directly from operations to policy. The
term comes from Greek: algos (pain) + hedone (pleasure). Beer
introduced the concept in the Viable System Model as the mechanism
by which System 1 (operations) alerts System 5 (policy) without
waiting for the signal to propagate through Systems 2, 3, and 4.

In the Conductor, algedonic signals represent conditions severe enough
that the normal evaluation pipeline is too slow. The Gamma loop should
not deliberate over whether a safety violation warrants intervention.

### When algedonic signals fire

Four conditions trigger algedonic escalation:

1. **Runaway cost**: total session cost exceeds 2x budget before 50%
   of wall time has elapsed. The cost trajectory predicts catastrophic
   overrun, not a gradual approach to the budget ceiling.

2. **Safety violation**: an agent attempts to modify files outside the
   declared workspace scope, execute disallowed commands, or access
   restricted resources. Any safety violation is an immediate interrupt
   regardless of severity assessment.

3. **Total infrastructure failure**: all agents are down simultaneously.
   Not one agent failing (which the Gamma loop handles), but every
   agent in the current execution losing connectivity or crashing at
   once.

4. **Operator interrupt**: explicit Ctrl+C or shutdown command. The
   human operator is the ultimate algedonic channel — their interrupt
   overrides everything.

### Escalation with time windows

Each layer in the hierarchy gets a bounded window to respond before the
signal escalates upward:

```
Agent detects anomaly → Conductor has 5s to respond
    | (no response within 5s)
Orchestrator has 30s to respond
    | (no response within 30s)
Policy layer triggers emergency shutdown
```

The time windows enforce liveness. A Conductor that hangs (perhaps
because the anomaly also affects its evaluation path) cannot silently
block the escalation. If the Conductor does not respond within 5
seconds, the orchestrator takes over. If the orchestrator does not
respond within 30 seconds, the policy layer performs an emergency
shutdown — kill all agents, persist state, exit with a non-zero status.

Algedonic signals are rare by design. If they fire frequently, the
normal feedback loops are miscalibrated. A well-tuned system routes
almost everything through the Gamma/Theta/Delta hierarchy and reserves
algedonic escalation for genuine emergencies.

---

## References

- Boyd, J. (1995). "The Essence of Winning and Losing" (OODA loop diagram).
- Boyd, J. (1976). "Destruction and Creation" (theoretical underpinning of OODA).
- Beer, S. (1972). *Brain of the Firm* (Viable System Model, algedonic signals).
- Mesarovic, M., Macko, D., and Takahara, Y. (1970). *Theory of Hierarchical, Multilevel Systems* (hierarchical control, parameter cascade).
- Klein, G. (1998). *Sources of Power: How People Make Decisions* (Recognition-Primed Decision model, parallel to IG&C).
