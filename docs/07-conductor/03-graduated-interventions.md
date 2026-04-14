# Graduated Interventions

> Three actions. No nudges. Continue, Restart, or Fail.
> The Conductor decides; it does not suggest.


> **Implementation**: Built

---

## The ConductorDecision Enum

Every evaluation cycle produces exactly one decision:

```rust
pub enum ConductorDecision {
    Continue,
    Restart { reason: String },
    Fail { reason: String },
}
```

**Continue**: All watchers report healthy. The plan proceeds without
intervention.

**Restart**: At least one watcher reported Warning severity. The current
agent is killed and restarted with different context. The key difference
from a retry: the restarted agent gets a FRESH start with ADDITIONAL
information about what went wrong. It is not the same agent continuing
from a confused state — it is a new agent with the benefit of hindsight.

**Fail**: At least one watcher reported Critical severity, or the
circuit breaker is tripped. The plan is marked as failed. The
orchestrator removes it from the merge queue, cancels in-flight tasks,
and dispatches work for other plans.

---

## Why No Nudge

Production experience (Issue #9, agent ghost turns; Issue #6, conductor
nudges without effect) demonstrated that nudging does not work:

```
Agent is stuck → Conductor sends nudge message →
Agent reads nudge → Agent attempts same approach →
Agent is still stuck → Conductor sends another nudge → ...
```

The problem: a confused agent remains confused after receiving a nudge.
The nudge says "you seem stuck, try a different approach" — but the
agent's confusion is WITHIN its context. Adding a nudge message to an
already-confused context does not reduce confusion. It may even increase
it by adding more text the agent needs to process.

The structural fix: the Conductor does not nudge. It either restarts
(kills the agent, gives a new agent the error analysis) or fails (marks
the plan for human attention). Both actions create a clean break from
the confused state.

This is Hard Guarantee 6 from the failure prevention catalog:
"The Conductor DECIDES, Never Nudges."

---

## The Severity System

### Three Levels

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info = 0,
    Warning = 1,
    Critical = 2,
}
```

The `PartialOrd` derivation enables severity comparison: `Critical >
Warning > Info`. The intervention policy uses this ordering to select
the maximum severity from all watcher outputs.

### Mapping to Decisions

| Severity | Decision | Orchestrator Action |
|----------|----------|-------------------|
| Info | Continue | No action. Log the observation. |
| Warning | Restart | Kill current agent. Spawn fresh agent with error context. |
| Critical | Fail | Mark plan as failed. Cancel in-flight work. Move to next plan. |

### Watcher Severity Defaults

| Watcher | Default Severity | Rationale |
|---------|-----------------|-----------|
| ghost-turn | Warning | Agent may recover; fresh start often helps |
| compile-fail-repeat | Warning | Different context may resolve the error |
| cost-overrun | Warning | May be worth one more attempt with budget awareness |
| iteration-loop | **Critical** | Three gate failures = fundamental mismatch |
| review-loop | Warning | Skip reviews and proceed to merge |
| spec-drift | Warning | Refocus the agent on declared scope |
| stuck-pattern | Warning | Fresh agent with different strategy |
| test-failure-budget | Warning | Agent is introducing regressions; needs restart |
| time-overrun | Warning | Early warning; may still finish in time |
| context-window-pressure | Warning | Compact context and retry |

Only `iteration-loop` defaults to Critical. Every other watcher
produces Warning, giving the plan one chance to recover through restart
before being failed.

---

## WatcherOutput

The intermediate representation between watcher signals and conductor
decisions:

```rust
pub struct WatcherOutput {
    pub watcher: String,       // which watcher fired
    pub severity: Severity,    // info / warning / critical
    pub description: String,   // human-readable explanation
    pub metric: Option<f64>,   // optional numeric value (ratio, count, etc.)
}
```

The Conductor collects `WatcherOutput`s from all watchers that fired
(returned non-empty signal vectors), then passes the collection to the
`InterventionPolicy` for resolution.

---

## The InterventionPolicy Trait

```rust
pub trait InterventionPolicy: Send + Sync {
    fn evaluate(
        &self,
        outputs: &[WatcherOutput],
        ctx: &Context,
    ) -> ConductorDecision;
}
```

The trait is deliberately simple: given a set of watcher outputs, produce
a decision. This allows different resolution strategies:

### WorstSeverityPolicy (Default)

The maximum severity among all outputs determines the decision:

```rust
pub struct WorstSeverityPolicy;

impl InterventionPolicy for WorstSeverityPolicy {
    fn evaluate(&self, outputs: &[WatcherOutput], _ctx: &Context) -> ConductorDecision {
        if outputs.is_empty() {
            return ConductorDecision::Continue;
        }

        let worst = outputs.iter()
            .map(|o| o.severity)
            .max()
            .unwrap_or(Severity::Info);

        match worst {
            Severity::Info => ConductorDecision::Continue,
            Severity::Warning => ConductorDecision::Restart {
                reason: format_watcher_reasons(outputs),
            },
            Severity::Critical => ConductorDecision::Fail {
                reason: format_watcher_reasons(outputs),
            },
        }
    }
}
```

This is a conservative policy: if ANY watcher reports a problem, the
Conductor acts on it. One watcher saying "critical" overrides nine
watchers saying "continue."

### Alternative Policies (Not Yet Implemented)

**MajoritySeverityPolicy**: Use the median severity instead of the
maximum. More tolerant — a single warning among nine healthy watchers
would be outvoted.

**WeightedSeverityPolicy**: Assign weights to watchers based on
their historical accuracy. Watchers with high false-positive rates
get lower weights. This requires the learning system to track
watcher accuracy.

**ContextualPolicy**: Different policies for different plan phases.
During Implementation, be aggressive (restart early). During Review,
be lenient (reviewers are inherently noisy). During Merge, be very
conservative (merge failures are expensive).

The `InterventionPolicy` trait supports all of these through
polymorphism. The Conductor stores `Box<dyn InterventionPolicy>` and
can switch policies at runtime.

---

## Decision Flow

The complete decision flow from signal stream to orchestrator action:

```
Signal Stream
    │
    ├── Watcher 1: ghost-turn      → [no fire]
    ├── Watcher 2: compile-fail    → Warning: "3 identical E0308 errors"
    ├── Watcher 3: cost-overrun    → [no fire]
    ├── Watcher 4: iteration-loop  → [no fire]
    ├── Watcher 5: review-loop     → [no fire]
    ├── Watcher 6: spec-drift      → Warning: "drift 32% exceeds 25%"
    ├── Watcher 7: stuck-pattern   → [no fire]
    ├── Watcher 8: test-budget     → [no fire]
    ├── Watcher 9: time-overrun    → [no fire]
    └── Watcher 10: ctx-pressure   → [no fire]
    │
    ▼
WatcherOutputs: [
    { watcher: "compile-fail-repeat", severity: Warning, ... },
    { watcher: "spec-drift", severity: Warning, ... },
]
    │
    ▼
WorstSeverityPolicy:
    max(Warning, Warning) = Warning
    │
    ▼
ConductorDecision::Restart {
    reason: "compile-fail-repeat: 3 identical E0308 errors; spec-drift: drift 32% exceeds 25%"
}
    │
    ▼
Orchestrator:
    1. Kill current agent
    2. Record failure in circuit breaker
    3. Spawn new agent with:
       - Error analysis from Diagnosis Engine
       - Updated context with compile error details
       - Refocused scope from spec drift data
```

---

## Escalation Semantics

### What Happens on Restart

When the Conductor decides Restart:

1. **The current agent is terminated.** Not paused, not given a
   final chance — terminated. Its process is killed and its context
   is discarded.

2. **The error context is preserved.** Gate results, compiler errors,
   watcher observations, and the reason for restart are collected into
   an error brief.

3. **A new agent is spawned.** Fresh context. No memory of the
   confused state. But it receives the error brief — it knows what
   the previous agent tried and why it failed.

4. **The iteration counter increments.** This restart counts toward
   the plan's iteration limit. After MAX_ITERATION_LOOP restarts,
   the next failure will be Critical.

The restart is not a continuation. It is a fresh start with the
benefit of hindsight.

### What Happens on Fail

When the Conductor decides Fail:

1. **All in-flight tasks for the plan are cancelled.** Agents are
   killed. Worktree state is preserved for post-mortem.

2. **The plan phase transitions to Failed(reason).** The reason
   includes which watcher fired and why.

3. **The circuit breaker records the failure.** If this is the
   second failure, the plan is tripped and will never be automatically
   retried.

4. **The orchestrator moves on.** Other plans that do not depend on
   the failed plan continue without interruption.

5. **The failure is surfaced.** The plan appears as "Failed" in the
   dashboard with the full reason. The deferred-failures log captures
   structured records with error snippets and failure context.

---

## Cooldown Periods

Each watcher intervention has a built-in cooldown to prevent the
conductor from firing the same intervention on consecutive evaluation
cycles.

The production experience that motivated cooldowns: the conductor
would detect a stuck agent, emit a restart signal, and then on the
next tick (before the restart had taken effect), detect the same stuck
signal again and emit another restart. This double-fire would sometimes
kill the replacement agent that was still starting up.

The cooldown ensures that after a watcher fires, it does not fire
again for the same plan until enough time has passed for the
intervention to take effect. The production default is 120 seconds
per plan per watcher.

---

## Intervention Signals

When the Conductor makes a non-Continue decision, it emits a signal
to the stream:

```rust
Signal::builder(Kind::Custom("conductor.intervention".into()))
    .body(Body::text(format!("{watcher_name}: {description}")))
    .tag("watcher", watcher_name)
    .tag("severity", severity_str)
    .tag("plan_id", plan_id)
    // ... watcher-specific tags
    .build()
```

These signals serve two purposes:

1. **Observability**: The dashboard, event log, and signal replay
   system can show exactly when and why the conductor intervened.

2. **Learning**: The efficiency tracking system records interventions
   as negative signals. Plans that trigger conductor interventions
   produce data for the cascade router's reward function, penalizing
   model/task combinations that produce intervention-worthy behavior.

---

## Relationship to Yerkes-Dodson Dynamics

Research on 770,000+ autonomous agents (§2.7 of the orchestration
reference) shows that cooperative behavior follows an inverted-U
curve with environmental pressure:

- **Too little pressure** (no iteration limits, generous timeouts):
  agents waste tokens exploring irrelevant approaches
- **Moderate pressure** (bounded iterations, reasonable timeouts):
  agents focus on the task and cooperate effectively
- **Too much pressure** (aggressive limits, tight timeouts):
  agents collapse into minimal-effort responses, skip steps, and
  produce incomplete work

The Conductor's intervention thresholds are Yerkes-Dodson parameters.
They sit somewhere on this curve:

- `MAX_GHOST_TURNS = 3` — how much silence before intervention
- `MAX_COMPILE_FAIL_REPEAT = 3` — how many identical errors before restart
- `MAX_ITERATION_LOOP = 3` — how many gate failures before fail
- `MAX_REVIEW_CYCLES = 3` — how many review rejects before skip
- `ALERT_THRESHOLD = 0.80` — how full the context can get
- `MAX_SPEC_DRIFT_RATIO = 0.25` — how much scope drift is tolerated

Each threshold represents a pressure setting. Too aggressive and agents
collapse. Too lenient and agents waste. The learning system
(efficiency events, cascade router observations) provides data for
tuning these thresholds over time — moving along the Yerkes-Dodson
curve toward the peak of the inverted-U.

Reference: Yerkes & Dodson (1908). "The relation of strength of
stimulus to rapidity of habit-formation."

---

## File Reference

| File | What |
|------|------|
| `crates/roko-conductor/src/interventions.rs` | Severity, WatcherOutput, InterventionPolicy, WorstSeverityPolicy |
| `crates/roko-conductor/src/conductor.rs` | evaluate() — decision flow |
| `crates/roko-core/src/agent.rs` | ConductorDecision enum |
