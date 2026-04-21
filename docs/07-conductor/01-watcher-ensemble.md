# Watcher Ensemble

> Ten independent detectors, each focused on one failure mode,
> each implementing the `Policy` trait, each testable in isolation.


> **Implementation**: Built

---

## The Policy Trait

Every watcher implements the same trait:

```rust
pub trait Policy: Send + Sync {
    fn decide(&self, stream: &[Engram], ctx: &Context) -> Vec<Engram>;
    fn name(&self) -> &str;
}
```

`decide()` receives the full signal stream and returns intervention
signals. An empty return means "healthy — nothing to report." A
non-empty return means the watcher has detected an anomaly and is
emitting one or more intervention signals with severity tags.

The Context provides the current tick position. Watchers may use it
for time-relative calculations but most ignore it, operating purely on
the signal stream content.

---

## Watcher Catalog

### 1. Ghost Turn Detector

**Module**: `watchers/ghost_turn.rs`
**Constant**: `MAX_GHOST_TURNS = 3`
**Watcher name**: `ghost-turn`

**What it detects**: Agent turns that produce zero meaningful output.
A ghost turn is a turn where the model returned immediately (often
under 5 seconds) with no tool calls, no file changes, and no
substantive content. This is a known failure mode with API-based agents
— the model may return an empty response, a brief acknowledgment
without action, or repeat its own instructions back.

**How it works**: Scans the signal stream for `AgentOutput` signals.
Tracks the most recent agent's output sequence. If the output body
matches the ghost pattern (below minimum meaningful length), increments
a counter. After `MAX_GHOST_TURNS` consecutive ghost turns from the
same agent, fires a warning.

**Detection logic**: The watcher checks `AgentOutput` signals for body
content. It considers a turn "ghost" when the output body is empty or
below a minimum threshold. Three consecutive ghost turns trigger the
intervention.

**Severity**: Warning (triggers restart with fresh context)

**Why three, not one**: A single empty response can be a transient API
issue. Two might be a flaky connection. Three consecutive ghost turns
indicate the agent has entered a degenerate state and will not recover
without intervention. The threshold balances false positives (killing a
slow-to-start agent) against false negatives (letting a broken agent
burn tokens).

**Production context**: Ghost turns were Issue #9 in the production
failure catalog. In early batch runs, ghost agents would appear active
(process running, consuming API quota) but produce nothing useful —
repeating themselves, asking clarifying questions to nobody, or
describing intended actions without executing them. These could burn
significant token budget before manual detection. The ghost turn watcher
automates what was previously a manual grep-the-logs operation.

---

### 2. Compile Fail Repeat Detector

**Module**: `watchers/compile_fail_repeat.rs`
**Constant**: `MAX_COMPILE_FAIL_REPEAT = 3`
**Watcher name**: `compile-fail-repeat`

**What it detects**: The same compile error appearing across consecutive
gate verdicts without the agent making progress toward fixing it.

**How it works**: Examines `GateVerdict` signals for compile-related
gate results. Extracts error fingerprints from the gate verdict body.
When the same fingerprint appears `MAX_COMPILE_FAIL_REPEAT` times
consecutively for the same plan, fires an intervention.

**Detection logic**: The watcher looks for `GateVerdict` signals,
extracts the error content from the body, and tracks whether the
same errors recur. Three identical compile errors in sequence indicate
the agent is attempting the same fix repeatedly.

**Severity**: Warning (triggers restart with error analysis context)

**Why this matters**: An agent stuck on a compile error is the most
common form of agent loop. The agent reads the error, attempts a fix,
recompiles, gets the same error, reads it again, attempts the same fix.
Without intervention, this cycle continues until the iteration limit.
The compile fail repeat watcher detects this after 3 cycles instead of
letting it run to exhaustion.

**Connection to Diagnosis Engine**: When this watcher fires, the
Diagnosis Engine can classify the specific error and suggest an
intervention. E.g., if the repeated error is E0432 (unresolved import),
the suggested intervention is `AutoFix` — a cheap Haiku-tier fix.
If it is E0277 (trait not implemented), the suggestion is
`RestartAgent` with additional type context.

---

### 3. Cost Overrun Detector

**Module**: `watchers/cost_overrun.rs`
**Constant**: `DEFAULT_BUDGET_USD = 10.0`
**Watcher name**: `cost-overrun`

**What it detects**: Plan-level cost exceeding the allocated budget.

**How it works**: Scans for `Metric` signals tagged with
`name=plan_cost` to track accumulated cost. Compares against the
plan's budget (from tags or default). When accumulated cost exceeds
budget, fires an intervention.

**Detection logic**: The watcher finds the most recent cost metric
signal for each plan and compares the cumulative cost against the
budget. The budget is read from a tag on the signal or falls back
to `DEFAULT_BUDGET_USD`.

**Severity**: Warning at threshold, escalates based on overage

**Why cost matters**: In production batch runs, a single runaway plan
can consume more budget than all other plans combined. The most
expensive failure mode is an agent that produces plausible-looking but
incorrect code, passes compilation, fails tests, gets retried with more
context (more tokens), fails again with slightly different errors, and
repeats. Each iteration costs more than the last because the context
grows. Without cost monitoring, this can run the total batch cost to
10x the expected budget.

**Budget allocation strategy**: The default $10 budget per plan is
conservative for Opus-tier tasks and generous for Haiku-tier tasks.
In practice, the budget should be set based on plan complexity:

| Complexity | Typical Cost | Suggested Budget |
|-----------|-------------|-----------------|
| Trivial | $0.10–0.50 | $2.00 |
| Simple | $0.50–2.00 | $5.00 |
| Standard | $1.00–5.00 | $10.00 |
| Complex | $3.00–15.00 | $25.00 |

The adaptive gate threshold system (in `roko-gate`) can eventually
feed cost data back to budget allocation, creating a learning loop.

---

### 4. Iteration Loop Detector

**Module**: `watchers/iteration_loop.rs`
**Constant**: `MAX_ITERATION_LOOP = 3`
**Watcher name**: `iteration-loop`

**What it detects**: Plans cycling through the gate-fail-retry loop
without making progress toward passing.

**How it works**: Tracks `GateVerdict` signals per plan. When a plan
accumulates `MAX_ITERATION_LOOP` consecutive gate failures without
an intervening gate pass, fires an intervention.

**Detection logic**: The watcher scans for `GateVerdict` signals,
tracking consecutive failures per plan. It resets the counter when a
gate pass is observed. Three consecutive failures trigger the
intervention.

**Severity**: Critical (triggers plan failure)

**Why critical**: This is the only watcher that defaults to Critical
severity. The rationale: three consecutive gate failures indicate a
fundamental mismatch between the agent's approach and the requirements.
More iterations of the same approach will not converge. The plan needs
either a different strategy (different model, more context, alternative
decomposition) or human attention.

**Hard Guarantee connection**: This implements Hard Guarantee 3 from
the failure prevention catalog — "Hard Iteration Cap (Not Soft, Not
Heuristic)." The iteration limit is enforced by the state machine, not
by heuristic detection. The conductor's role changes from "detect loops
and decide whether to intervene" to "the plan failed; decide whether
it is worth retrying with a different approach."

**The compound problem**: Each retry iteration is more expensive than
the last. The agent's context grows (previous errors, reflections,
gate output), increasing token cost. The probability of convergence
decreases with each failed attempt (if the first three attempts
failed, the fourth is unlikely to succeed without a fundamentally
different approach). Cutting off at three prevents the exponential
cost growth of diminishing-probability retries.

---

### 5. Review Loop Detector

**Module**: `watchers/review_loop.rs`
**Constant**: `MAX_REVIEW_CYCLES = 3`
**Watcher name**: `review-loop`

**What it detects**: Plans receiving repeated review rejects without
advancing to a later phase.

**How it works**: Scans `PlanPhase` signals for the most recent plan ID.
Counts `ReviewRejected` events for that plan. Resets the counter on
`ReviewApproved`, `DocRevisionDone`, or `MergeSucceeded`. Fires when
the count reaches `MAX_REVIEW_CYCLES`.

```rust
// From watchers/review_loop.rs — decision logic
match plan_event(s).as_deref() {
    Some("ReviewRejected") => {
        review_rejects += 1;
        if review_rejects >= self.max_cycles {
            // Fire intervention
        }
    }
    Some("ReviewApproved") | Some("DocRevisionDone") | Some("MergeSucceeded") => {
        review_rejects = 0;  // Reset — progress was made
    }
    _ => {}
}
```

**Severity**: Warning (triggers review skip or strategy change)

**The bikeshedding problem**: In production batch runs, reviewer agents
can enter a cycle where code passes compilation and tests but the
reviewer repeatedly requests stylistic changes. Each review reject
triggers a re-implementation cycle. The implementer makes the requested
changes, the reviewer finds new stylistic concerns, and the cycle
repeats. Three consecutive rejects without progress indicates
bikeshedding — the code works, the reviewers are not converging, and
further iterations waste tokens.

**Reset semantics**: The counter resets on any positive progress event.
This means a plan that receives one reject, then an approval, then
two more rejects has only counted two consecutive rejects — the
approval reset the counter. Only sustained, uninterrupted rejection
sequences trigger the intervention.

---

### 6. Spec Drift Detector

**Module**: `watchers/spec_drift.rs`
**Constant**: `MAX_SPEC_DRIFT_RATIO = 0.25`
**Watcher name**: `spec-drift`

**What it detects**: Agent file edits drifting outside the declared
scope of the task.

**How it works**: Examines `Metric` signals tagged
`name=spec_drift`. The signal body contains a `SpecDriftEvent` with:
- `write_files`: files the task declared it would modify
- `changed_files`: files the agent actually modified
- `unexpected_files`: changed files not in the declared set
- `drift_ratio`: fraction of changes that were unexpected

When `drift_ratio > MAX_SPEC_DRIFT_RATIO` (25%), fires an intervention.

**Detection logic**: The watcher supports two signal formats — a
structured JSON body with full file lists, or a simple tag-based
format with just the drift ratio number. This dual-format support
allows both detailed and lightweight drift reporting.

```rust
// Drift computation from SpecDriftEvent
fn drift_ratio(&self) -> f64 {
    let changed = self.changed_files.len();
    if changed == 0 { return 0.0; }
    self.unexpected_files().len() as f64 / changed as f64
}
```

**Severity**: Warning

**Why 25%**: Some drift is normal and healthy. An agent implementing
a new function may need to update a `mod.rs` file or add an import
to a sibling module. A 10% drift ratio is typical for well-scoped
tasks. At 25%, the agent is making substantial changes outside its
declared scope — potentially stepping on another concurrent agent's
territory or introducing unplanned coupling.

**Path matching**: The `path_is_allowed()` function supports both
exact matches and prefix matches. If the declared write file is
`src/auth/`, any file under that directory is considered in-scope.
This prevents false positives when the task declares a directory
but the agent creates new files within it.

---

### 7. Stuck Pattern Detector

**Module**: `watchers/stuck_pattern.rs`
**Constant**: `MAX_STUCK_REPEATS = 4`
**Watcher name**: `stuck-pattern`

**What it detects**: Agent producing identical actions across
consecutive turns.

**How it works**: Tracks recent agent actions (tool calls, file edits)
and computes similarity between consecutive turns. When four consecutive
turns produce identical or near-identical actions, fires an intervention.

**Severity**: Warning

**Relationship to other watchers**: The stuck pattern detector overlaps
with compile fail repeat (which specifically catches identical compile
errors) and ghost turn (which catches zero output). The stuck pattern
detector is the general-purpose version — it catches any form of
repetitive behavior, not just compile loops or empty responses.

---

### 8. Test Failure Budget Detector

**Module**: `watchers/test_failure_budget.rs`
**Constant**: `MIN_FAILURE_INCREASE = 1`
**Watcher name**: `test-failure-budget`

**What it detects**: Test failure count increasing beyond the baseline
observed earlier in the signal stream.

**How it works**: Scans `GateVerdict` signals for structured test counts.
For each plan, records the first observed failure count as the baseline.
When the latest failure count exceeds the baseline by
`MIN_FAILURE_INCREASE`, fires an intervention.

```rust
// Per-plan baseline tracking
baselines.entry(plan_id.clone()).or_insert(failed);  // First seen = baseline
latest.insert(plan_id, failed);                       // Always update latest

// Fire when latest exceeds baseline
if current_failed.saturating_sub(baseline_failed) >= self.min_failure_increase {
    // Emit intervention
}
```

**Severity**: Warning

**The regression signal**: This watcher detects a specific problem —
the agent is making things worse, not better. If a plan starts with
1 failing test and ends with 3 failing tests, the agent introduced
2 new test failures. This is a stronger signal than "tests are failing"
(which might be the expected state at start) — it means the agent's
changes are actively harmful.

**Plan independence**: Each plan has its own baseline. Plan A starting
with 5 failures and Plan B starting with 0 failures are tracked
independently. A failure increase on Plan A does not affect Plan B's
baseline.

**Custom thresholds**: The constructor accepts a custom
`min_failure_increase`. For codebases with flaky tests, setting this
to 3 (rather than 1) avoids false positives from non-deterministic
test outcomes.

---

### 9. Time Overrun Detector

**Module**: `watchers/time_overrun.rs`
**Constant**: `ALERT_THRESHOLD = 0.80`
**Watcher name**: `time-overrun`

**What it detects**: Tasks approaching their timeout threshold.

**How it works**: Examines `Custom("conductor.agent_output")` signals
for `TaskTimingEvent` payloads containing `duration_ms` and
`timeout_secs`. When the ratio exceeds 80% of the timeout, fires
an early warning.

```rust
// Threshold check using integer arithmetic to avoid floating-point edge cases
fn exceeds_threshold(duration_ms: u64, timeout_secs: u64) -> bool {
    if timeout_secs == 0 { return false; }
    let timeout_ms = timeout_secs.saturating_mul(1000);
    duration_ms.saturating_mul(5) > timeout_ms.saturating_mul(4)
    // Equivalent to: duration_ms / timeout_ms > 4/5 = 0.80
}
```

**Severity**: Warning

**Why 80%, not 100%**: The 80% threshold provides a 20% buffer for the
system to react. At 100%, the task has already timed out — there is
nothing to do except fail it. At 80%, the Conductor can signal the
orchestrator to prepare for a potential timeout: start warming a
replacement agent, checkpoint the current state, or adjust the
remaining task's priority.

**Integer arithmetic**: The threshold check uses `saturating_mul`
instead of floating-point division to avoid edge cases with zero
denominators and floating-point precision. The comparison
`duration * 5 > timeout * 4` is algebraically equivalent to
`duration / timeout > 0.80` but avoids division.

---

### 10. Context Window Pressure Detector

**Module**: `watchers/context_window_pressure.rs`
**Constant**: `MAX_CONTEXT_USAGE_RATIO = 0.80`
**Watcher name**: `context-window-pressure`

**What it detects**: Agent context window filling beyond safe limits.

**How it works**: Examines `TokenUsage` signals for token consumption.
Supports two signal formats:

1. **AgentEfficiencyEvent body**: Deserializes the structured event
   to extract `total_prompt_tokens` and looks up the model's context
   window size from a built-in table.

2. **Tag-based format**: Reads `tokens_used` and `tokens_total` (or
   `model`) from signal tags.

When `used / total > MAX_CONTEXT_USAGE_RATIO`, fires an intervention.

**Model context windows**:

| Model Pattern | Context Window |
|--------------|---------------|
| `*opus*` | 1,000,000 tokens |
| `*haiku*`, `*sonnet*` | 200,000 tokens |
| Unknown | No fire (cannot compute ratio) |

**Severity**: Warning (triggers context compaction)

**Why 80%**: From production monitoring research and the Semantic Kernel
framework: trigger compaction at 80% utilization, not 100%. At 100% the
next request fails with a context overflow error. At 80% there is still
space to compact gracefully — truncating old tool results, summarizing
earlier conversation turns, or dropping low-relevance context sections.

**The compaction cascade**: When this watcher fires, the orchestrator
should trigger the tool result compaction strategy (from the production
hardening plan): truncate old tool results to 200 characters, preserve
recent results intact, maintain tool_call_id integrity. This recovers
20-40% of context space without losing critical recent context.

**AgentEfficiencyEvent integration**: The context window pressure
watcher reads from the same `AgentEfficiencyEvent` signals that feed
the learning system's efficiency tracking. This means every agent turn
that records efficiency data also gets context pressure monitoring for
free — no additional instrumentation needed.

---

## Watcher Independence

Each watcher is independent:

- **No shared state**: Watchers do not read each other's output or
  maintain shared counters.
- **No ordering dependency**: Watchers can execute in any order.
  The Conductor iterates them sequentially for simplicity, but
  parallel execution would produce identical results.
- **No cross-watcher interaction**: The ghost turn watcher does not
  know about the stuck pattern watcher. If both fire simultaneously,
  the intervention policy resolves the conflict (worst severity wins).

This independence is what makes the ensemble testable. Each watcher
has its own `#[cfg(test)] mod tests` with focused test cases that
construct specific signal sequences and verify the watcher's response.
No test needs to set up the full Conductor or mock other watchers.

---

## Adding a New Watcher

To add an eleventh watcher:

1. Create a new file in `watchers/` implementing `Policy`
2. Add it to `watchers/mod.rs`
3. Add it to `Conductor::new()` in `conductor.rs`
4. Update the `watcher_count()` test (currently asserts 10)
5. Write focused tests for the new watcher's detection logic

The Conductor's `evaluate()` method automatically picks up any watcher
in the `watchers` vector. No other code needs to change.

---

## File Reference

| File | Lines | What |
|------|-------|------|
| `watchers/mod.rs` | ~20 | Module declarations, re-exports |
| `watchers/ghost_turn.rs` | ~150 | Ghost turn detection |
| `watchers/compile_fail_repeat.rs` | ~180 | Compile error repetition |
| `watchers/cost_overrun.rs` | ~180 | Cost budget monitoring |
| `watchers/iteration_loop.rs` | ~170 | Gate-fail cycle detection |
| `watchers/review_loop.rs` | ~230 | Review reject cycles |
| `watchers/spec_drift.rs` | ~264 | File scope drift |
| `watchers/stuck_pattern.rs` | ~170 | Repeated action detection |
| `watchers/test_failure_budget.rs` | ~202 | Test regression detection |
| `watchers/time_overrun.rs` | ~182 | Timeout approach warning |
| `watchers/context_window_pressure.rs` | ~233 | Token usage monitoring |

---

## Watcher Composition — Complex Pattern Detection

Individual watchers answer narrow questions: "Did the agent ghost?"
"Did compile errors repeat?" Real failures are rarely that tidy.
A context window filling up while compile errors repeat while cost
climbs — that compound pattern signals something no single watcher
can catch alone.

This section describes how watchers compose to detect those
multi-signal patterns.

### CEP-inspired pattern matching

Complex Event Processing engines — Apache Flink's FlinkCEP, Esper
EPL, Siddhi CQL — solve a structurally identical problem: detect
temporal patterns in high-volume event streams. The core technique
is NFA-based pattern matching over ordered signal sequences.

Two categories of temporal patterns matter for watcher composition:

**Sequence detection** — "Signal A followed by Signal B within time T."
Example: "Agent retried 3+ times within 60 seconds."

```
PATTERN [every A{3,} within 60 sec]
```

**Absence detection** — "Signal A occurred, but Signal B did NOT
follow within time T." Example: "Tool call without response within
30 seconds."

```
PATTERN [A -> (not B where timer:within(30 sec))]
```

**Monotonic progression** — "Each successive signal exceeds the
previous on some metric." Example: "Escalating latency: each turn
slower than the last."

```
MATCH_RECOGNIZE(PATTERN (A B+ C) DEFINE B AS B.latency > A.latency)
```

These patterns translate directly to agent monitoring. The signal
stream is the event source. Watchers produce the events. A composite
pattern layer matches sequences across watcher outputs.

The struct below captures this:

```rust
/// Composite pattern that matches sequences across multiple watchers.
/// Inspired by NFA-based CEP engines (FlinkCEP, Esper EPL).
pub struct CompositePattern {
    /// Pattern stages — each stage matches one or more signal conditions.
    stages: Vec<PatternStage>,
    /// Maximum wall-clock duration for the entire pattern to complete.
    within: Duration,
    /// Contiguity: Strict (no gaps), Relaxed (skip non-matching), or NonDeterministic.
    contiguity: Contiguity,
}

pub struct PatternStage {
    /// Signal kind(s) this stage matches.
    match_kinds: Vec<Kind>,
    /// Predicate evaluated against the signal's tags and body.
    predicate: Box<dyn Fn(&Engram) -> bool + Send + Sync>,
    /// Quantifier: Exactly(n), AtLeast(n), Between(min, max).
    quantifier: Quantifier,
    /// Negation: if true, this stage matches the ABSENCE of the pattern.
    negated: bool,
}

pub enum Contiguity { Strict, Relaxed, NonDeterministic }
pub enum Quantifier { Exactly(usize), AtLeast(usize), Between(usize, usize) }
```

**Contiguity modes** control how strictly the pattern engine matches
consecutive events:

- **Strict**: Every signal between stages must match (no irrelevant
  signals allowed in the gap).
- **Relaxed**: Non-matching signals between stages are skipped.
- **NonDeterministic**: Multiple partial matches can coexist,
  branching on ambiguous signals.

For agent monitoring, Relaxed contiguity is the right default. Agent
signal streams contain many signal types; requiring strict adjacency
between pattern stages would miss most real patterns.

### Multi-watcher correlation

When multiple watchers fire at the same time, the question is: one
root cause, or multiple independent failures? The answer depends on
which watchers fired.

Watchers group into three families based on what they measure:

**Resource family**: cost-overrun, context-window-pressure, time-overrun.
These track consumption of finite budgets (money, tokens, wall-clock
time). When two resource watchers fire together, the root cause is
usually a single runaway process burning all three budgets
simultaneously.

**Behavioral family**: ghost-turn, stuck-pattern, compile-fail-repeat,
iteration-loop. These detect degenerate agent behavior. Two behavioral
watchers firing together confirms the agent is stuck — the specific
failure mode is being caught from multiple angles.

**Coordination family**: review-loop, spec-drift, test-failure-budget.
These detect multi-agent coordination breakdowns. Two coordination
watchers firing together indicates a systemic scoping or communication
problem, not a single agent failure.

```rust
/// Watcher family grouping for correlated signal analysis.
pub struct WatcherFamily {
    pub name: &'static str,
    pub members: Vec<&'static str>,
}

pub const WATCHER_FAMILIES: &[WatcherFamily] = &[
    WatcherFamily { name: "resource", members: &["cost-overrun", "context-window-pressure", "time-overrun"] },
    WatcherFamily { name: "behavioral", members: &["ghost-turn", "stuck-pattern", "compile-fail-repeat", "iteration-loop"] },
    WatcherFamily { name: "coordination", members: &["review-loop", "spec-drift", "test-failure-budget"] },
];
```

**Within-family correlation**: If 2+ watchers in the same family fire
simultaneously, treat them as a single correlated event. The underlying
cause is likely one issue manifesting in multiple metrics. De-duplicate
before escalating — report the highest-severity watcher's signal,
annotated with which other family members also fired.

**Cross-family correlation**: If watchers from different families fire
simultaneously, the situation is more severe. A behavioral failure
(stuck-pattern) combined with a resource failure (cost-overrun) means
the agent is both stuck AND burning budget doing it. Cross-family
correlation should escalate severity by one level: a Warning from
each family becomes a single Critical intervention.

---

## Watcher Priority and Conflict Resolution

When multiple watchers fire in the same evaluation cycle, the
Conductor must produce a single coherent intervention. The current
approach is conservative. The alternatives below offer more nuance
at the cost of more complexity.

### Current approach: WorstSeverityPolicy

`WorstSeverityPolicy` takes the maximum severity across all fired
watchers. If ghost-turn fires at Warning and iteration-loop fires at
Critical, the result is Critical.

This is simple, conservative, and effective. It never under-reacts.
The tradeoff: it can over-react when a low-confidence Critical
watcher fires alongside high-confidence Warning watchers. Every
alternative below addresses this tradeoff.

### Bayesian fusion

Instead of taking the max, combine watcher outputs probabilistically.
Each watcher provides a likelihood ratio — how much more probable
is this signal under "anomaly" than under "normal operation"?

The combined posterior follows from log-odds addition:

```
log P(anomaly | signals) = log P(anomaly) + sum_i log LR_i(s_i)
```

A watcher that fires with a high likelihood ratio shifts the posterior
strongly toward anomaly. A watcher that stays silent shifts it toward
normal (by its silent likelihood ratio). The net effect: watchers
that are historically accurate carry more weight than watchers that
produce frequent false positives.

```rust
/// Bayesian fusion of watcher outputs for conflict resolution.
pub struct BayesianFusionPolicy {
    /// Prior probability of anomaly (before any watchers fire).
    prior_log_odds: f64,
    /// Per-watcher calibration: (log_likelihood_ratio_when_fired, log_likelihood_ratio_when_silent).
    watcher_calibrations: HashMap<String, WatcherCalibration>,
}

pub struct WatcherCalibration {
    /// log(P(fire | anomaly) / P(fire | normal)) — how informative is this watcher when it fires?
    pub log_lr_fired: f64,
    /// log(P(silent | anomaly) / P(silent | normal)) — how informative is silence?
    pub log_lr_silent: f64,
    /// Historical precision (true positives / all positives).
    pub precision: f64,
    /// Historical recall (true positives / all anomalies).
    pub recall: f64,
}
```

This requires calibration data: each watcher's true positive and
false positive rates from historical runs. The learning system
(roko-learn) already tracks per-gate precision via the adaptive
threshold mechanism. Extending that tracking to watchers gives the
calibration data Bayesian fusion needs.

### Dempster-Shafer theory

Bayesian fusion assumes watchers are well-calibrated probabilistic
classifiers. In practice, watchers often express something weaker:
"I think something is wrong, but I'm not sure what." Dempster-Shafer
theory handles this uncertainty directly.

Each watcher provides a mass function over three states:
{anomaly}, {normal}, {anomaly, normal} (ignorance).

A watcher that fires with high confidence assigns most mass to
{anomaly}. A watcher that is uncertain assigns mass to
{anomaly, normal} — expressing ignorance, NOT confidence in
normality. This distinction matters: "I don't know" is different
from "everything is fine."

Combination follows Dempster's rule:

```
m_12(A) = sum_{B intersect C = A} m_1(B) * m_2(C) / (1 - K)
```

where K is the conflict mass — the sum of products where the
intersection is empty. When K > 0.5, the watchers genuinely
disagree about what is happening. This is itself a signal: high
conflict between watchers means the system should escalate for human
review rather than auto-resolve.

**When to prefer Dempster-Shafer over Bayesian**: When watchers
have poor calibration data (early in a project's lifecycle), when
watchers express qualitative judgments rather than probabilistic
scores, or when the "I don't know" state carries important
information.

### Weighted voting with online learning

A lighter-weight alternative to full probabilistic fusion: weight
each watcher's vote by its historical precision.

Track per-watcher precision via an online confusion matrix. Each
time a watcher fires and the outcome is later confirmed (task
succeeded or failed), update the matrix. Weight each watcher's
severity vote by its precision score.

Use Thompson sampling — already available in roko-learn's bandit
infrastructure — to adapt weights over time. Each watcher is an
arm. The reward is correct prediction (watcher fired and the task
genuinely failed, or watcher stayed silent and the task succeeded).
Watchers with high false-positive rates get downweighted
dynamically. Watchers with consistently accurate predictions gain
influence.

This approach requires less calibration data than Bayesian fusion
and handles non-stationary watcher accuracy (a watcher that was
accurate last month but is noisy this month gets downweighted
automatically).

### Temporal hysteresis

Orthogonal to the fusion method: prevent single-spike false
positives by requiring sustained firing before a watcher's output
counts.

A watcher must fire for N consecutive evaluations before its signal
propagates to the intervention policy. Default N=1 (current
behavior — no hysteresis). For noisy watchers, set N=3: the watcher
must fire three times in a row before its output reaches the fusion
layer.

This prevents oscillation patterns: fire, restart, fire, restart.
With hysteresis, the first two firings are absorbed. The third
confirms the pattern is real and propagates the intervention.

Hysteresis is per-watcher configurable. Stable watchers
(iteration-loop, cost-overrun) should keep N=1 — they fire based
on accumulated counts and are already resistant to transient noise.
Noisy watchers (spec-drift, context-window-pressure) benefit from
N=2 or N=3.

---

## Streaming Anomaly Detection Integration

The threshold-based watchers above catch known failure modes:
compile loops, ghost turns, cost overruns. They do not catch
novel failures — patterns that nobody anticipated when writing
the watcher catalog. Streaming anomaly detection fills that gap.

### Online Isolation Forest

The original Isolation Forest (Liu et al., 2008) builds random
binary trees over a batch dataset. Points that isolate quickly
(short average path length) are anomalies. The online variant
(ICML 2024) adapts this to streaming data by maintaining a sliding
window and splitting leaf nodes incrementally.

Each node tracks a count and bounding box. When a leaf accumulates
enough points, it splits on a random dimension at a random value
within the bounding box. Old points outside the sliding window
are decremented from leaf counts.

Anomaly score:

```
s = 2^(-E(depth) / c(window_size))
```

where `E(depth)` is the expected depth of the point across all trees
and `c(n)` is the average path length of an unsuccessful search in a
binary search tree of size n (the normalization factor).

For agent monitoring, each agent turn becomes a multivariate point:
latency (ms), tokens consumed, tool calls made, error rate. A turn
that scores above the threshold on the forest is flagged as
anomalous — even if no individual watcher fires.

```rust
/// Online Isolation Forest for streaming anomaly detection.
/// Reference: ICML 2024, "Online Isolation Forest"
pub struct OnlineIsolationForest {
    trees: Vec<IsolationTree>,
    window_size: usize,       // omega: sliding window (default: 1000)
    max_leaf_samples: usize,   // eta: split threshold (default: 8)
    num_trees: usize,          // tau: ensemble size (default: 50)
    score_threshold: f64,      // anomaly if score > threshold (default: 0.7)
}

pub struct IsolationTree {
    root: IsolationNode,
}

pub enum IsolationNode {
    Internal {
        split_dim: usize,
        split_val: f64,
        count: usize,
        bbox: BoundingBox,
        left: Box<IsolationNode>,
        right: Box<IsolationNode>,
    },
    Leaf {
        count: usize,
        bbox: BoundingBox,
        samples: Vec<Vec<f64>>,  // retained for splitting
    },
}
```

**Default parameters**: tau=50 trees, omega=1000 window size,
eta=8 max leaf samples, score threshold=0.7. These are standard
values from the literature. For agent monitoring workloads with
lower volume (hundreds of turns per plan, not millions), reduce
omega to 100-200 to build the model faster.

### CUSUM for per-watcher change detection

EWMA z-scores (used by the existing watchers) detect spikes —
single-turn anomalies that deviate sharply from the mean.
CUSUM (Cumulative Sum, Page 1954) detects something different:
sustained shifts. A metric that drifts upward by 0.5 sigma per
turn won't trigger a z-score alarm for many turns, but CUSUM
catches it quickly.

The upper CUSUM statistic:

```
C_t = max(0, C_{t-1} + (x_t - mu_0 - k))
```

where `mu_0` is the baseline mean and `k` is the allowance
parameter (typically delta/2, where delta is the shift magnitude
to detect). When `C_t` exceeds the decision threshold `h`, CUSUM
raises an alarm.

The lower CUSUM statistic tracks negative shifts symmetrically.
Together they detect both upward and downward sustained changes.

```rust
/// CUSUM change-point detector for sustained anomaly detection.
/// Detects persistent shifts from a baseline, complementing EWMA spike detection.
pub struct CusumDetector {
    /// Target mean (baseline behavior).
    mu_0: f64,
    /// Allowance parameter (typically delta/2 where delta is the shift to detect).
    k: f64,
    /// Decision threshold (alarm when cumulative sum exceeds h).
    h: f64,
    /// Upper CUSUM statistic.
    upper: f64,
    /// Lower CUSUM statistic.
    lower: f64,
}

impl CusumDetector {
    pub fn update(&mut self, value: f64) -> Option<CusumAlarm> {
        self.upper = (self.upper + (value - self.mu_0 - self.k)).max(0.0);
        self.lower = (self.lower - (value - self.mu_0 + self.k)).max(0.0);
        if self.upper > self.h {
            Some(CusumAlarm::UpperShift { cumsum: self.upper })
        } else if self.lower > self.h {
            Some(CusumAlarm::LowerShift { cumsum: self.lower })
        } else {
            None
        }
    }
}
```

**Operating characteristics**: With k = delta/2 and h = 4-5 sigma,
the average run length before a false alarm (ARL_0) is roughly
500 samples. The average run length to detect a 1-sigma shift
(ARL_1) is roughly 26 samples. CUSUM detects sustained degradation
10-50x faster than EWMA z-scores for the same false alarm rate.

**Application**: Attach a CUSUM detector to each watcher's numeric
output (cost per turn, latency per turn, drift ratio). The watcher's
own threshold catches spikes; CUSUM catches gradual worsening that
stays below the spike threshold but accumulates over time.

### TraceAegis-style behavioral rules

TraceAegis (arXiv 2510.11203, October 2024) monitors tool
invocation traces at the gateway level. Instead of statistical
anomaly detection, it defines explicit behavioral rules: expected
call ordering, parameter constraints, intent-aligned state
transitions.

This approach detects a class of failures that statistical methods
miss: an agent that calls tools in a valid but unauthorized order,
modifies parameters within normal ranges but in a goal-misaligned
way, or makes state transitions that are individually legal but
collectively drift from the task intent.

Behavioral rules for agent monitoring:

- **Call ordering**: Tool A must precede Tool B (e.g., read a file
  before editing it). Violations indicate the agent is operating
  on stale or missing context.
- **Parameter constraints**: Tool parameters must fall within
  task-declared bounds (e.g., file edits restricted to declared
  write paths). Violations overlap with spec-drift detection but
  operate at the tool-call level rather than the file-change level.
- **State transitions**: After a gate failure, the next action
  should be a diagnostic step (read error output, inspect failing
  test), not a blind retry. Violations indicate the agent is not
  learning from feedback.
- **API boundary enforcement**: Agents should not call tools
  outside their declared capability set. An implementation agent
  calling a deployment tool is a boundary violation.

Validated F1 scores for TraceAegis-style rule checking range
from 0.93 to 0.96 in their evaluated scenarios, making this a
high-precision complement to the statistical methods above.

### References

- Liu, F. T., Ting, K. M., & Zhou, Z.-H. (2008). "Isolation Forest." ICDM.
- Online Isolation Forest. ICML 2024.
- Page, E. S. (1954). "Continuous Inspection Schemes." Biometrika.
- TraceAegis. arXiv 2510.11203, October 2024.
- Dempster, A. P. (1967). "Upper and Lower Probabilities Induced by a Multivalued Mapping." Annals of Mathematical Statistics.
- Shafer, G. (1976). A Mathematical Theory of Evidence. Princeton University Press.
- Apache Flink CEP documentation: https://nightlies.apache.org/flink/flink-docs-stable/docs/libs/cep/
- Esper EPL documentation: https://www.espertech.com/esper/
