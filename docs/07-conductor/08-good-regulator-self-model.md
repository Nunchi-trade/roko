# Good Regulator and the Self-Model

> "Every good regulator of a system must be a model of that system."
> — Conant & Ashby (1970)
>
> The Conductor is Roko's self-model. It represents the system's
> understanding of what healthy execution looks like.


> **Implementation**: Built

---

## The Theorem

The Good Regulator Theorem (Conant & Ashby, 1970) states that any
system that successfully regulates another system must contain a model
of that system. This is not a design recommendation — it is a
mathematical proof. A regulator that does not model the system it
controls cannot be an optimal regulator.

For the Conductor: to regulate agent execution, the Conductor must
model what healthy agent execution looks like. Every threshold, every
heuristic, every error pattern is a component of this model.

---

## Components of the Self-Model

### 1. Behavioral Norms (Watcher Thresholds)

Each watcher threshold encodes an expectation about normal behavior:

| Threshold | Expectation |
|-----------|------------|
| `MAX_GHOST_TURNS = 3` | A healthy agent produces meaningful output on every turn |
| `MAX_COMPILE_FAIL_REPEAT = 3` | A healthy agent does not repeat the same compile error |
| `MAX_ITERATION_LOOP = 3` | A healthy plan converges within 3 gate-fail cycles |
| `MAX_REVIEW_CYCLES = 3` | A healthy plan passes review within 3 cycles |
| `MAX_SPEC_DRIFT_RATIO = 0.25` | A healthy agent modifies at most 25% unexpected files |
| `MAX_STUCK_REPEATS = 4` | A healthy agent does not repeat identical actions |
| `MIN_FAILURE_INCREASE = 1` | A healthy agent does not increase test failures |
| `ALERT_THRESHOLD = 0.80` | A healthy task completes within 80% of its timeout |
| `MAX_CONTEXT_USAGE_RATIO = 0.80` | A healthy agent uses at most 80% of its context window |
| `MAX_PLAN_FAILURES = 2` | A recoverable plan succeeds within 2 attempts |

These thresholds define the "normal region" of execution space. When
execution leaves this region, the Conductor intervenes to push it back.

### 2. Failure Taxonomy (Error Categories)

The 20 error categories in the diagnosis engine model the system's
failure modes:

```
CompileError, TestFailure, TypeMismatch, BorrowCheckerError,
LifetimeError, ImportError, MissingFile, PermissionDenied,
NetworkError, TimeoutError, OomError, DiskFull,
LlmRateLimit, LlmContextOverflow, LlmRefusal,
ProcessCrash, LoopDetected, ClippyWarning,
GitConflict, DependencyError
```

Each category represents the system's understanding of a distinct
way things can go wrong. The intervention mapping (which action to
take for each category) represents the system's understanding of
how to recover from each failure mode.

### 3. Process Patterns (Stuck Heuristics)

The six stuck kinds model pathological execution patterns:

```
OutputLoop    — doing the same thing repeatedly
NoProgress    — doing things that produce no results
GateLoop      — oscillating between two broken states
CompileLoop   — toggling between incompatible fixes
EmptyOutput   — producing text without action
ExcessiveRetries — retrying without changing approach
```

Each pattern is a mode of execution that LOOKS like progress (the
agent is active, producing output, calling tools) but IS NOT progress.
The stuck detector models the difference between activity and progress.

### 4. Infrastructure Expectations (Health Checks)

The health monitor models infrastructure requirements:

- Agents should be running (agent status)
- Agents should be responsive (terminal liveness)
- Specifications should be current (spec drift)
- Quality should be maintained (coverage trend)

These expectations define what "the system is ready to do work" means.

---

## Model Accuracy

The self-model's accuracy determines the Conductor's effectiveness.
An inaccurate model produces:

### False Positives (Model Too Strict)

The model considers healthy behavior to be pathological. Examples:
- `MAX_GHOST_TURNS = 1` would kill agents that take one turn to
  read context before producing output
- `MAX_SPEC_DRIFT_RATIO = 0.05` would flag agents that update a
  mod.rs file alongside their primary target

False positives waste resources — healthy agents are killed and
restarted unnecessarily.

### False Negatives (Model Too Lenient)

The model considers pathological behavior to be healthy. Examples:
- `MAX_GHOST_TURNS = 10` would let a stuck agent burn tokens for
  10 turns before intervention
- `MAX_ITERATION_LOOP = 10` would let a non-converging plan retry
  10 times before failing

False negatives waste resources — pathological agents run unchecked.

### The Tuning Challenge

The model must be calibrated against real execution data. The current
thresholds are derived from production experience during batch runs
in March-April 2026. They represent the best-known calibration for
that period's codebase, model versions, and task complexity.

As these factors change, the model drifts. New model versions may
have different failure patterns. Codebase evolution changes what
"normal" spec drift looks like. Task complexity shifts change what
"normal" iteration count means.

---

## Static vs. Adaptive Models

### Current: Static Model

All thresholds are compile-time constants or constructor parameters.
The model does not update based on observed behavior:

```rust
pub const MAX_CONTEXT_USAGE_RATIO: f64 = 0.80;
pub const MAX_GHOST_TURNS: usize = 3;
pub const MAX_COMPILE_FAIL_REPEAT: usize = 3;
```

**Advantage**: Predictable, easy to reason about, no drift.
**Disadvantage**: Cannot adapt to changing conditions.

### Future: Adaptive Model

The learning system provides the infrastructure for an adaptive
self-model. The components exist:

- **Adaptive gate thresholds** (`roko-gate/src/adaptive_threshold.rs`):
  EMA-based threshold adjustment per gate rung. Already wired.
- **Efficiency events** (`roko-learn/src/efficiency.rs`): Per-turn
  metrics including iteration count, cost, success rate. Already
  collected.
- **Cascade router observations**: Model-task combination outcomes.
  Already recorded.

An adaptive Conductor model would:

1. Record the threshold that triggered each intervention
2. Track whether the intervention improved the outcome (did the
   restarted agent succeed? did the failed plan succeed on retry?)
3. Adjust thresholds toward values that maximize intervention
   effectiveness

For example, if interventions triggered at `MAX_GHOST_TURNS = 3`
successfully recover 80% of stuck agents, but interventions at
`MAX_GHOST_TURNS = 2` recover 90%, the adaptive model would lower
the threshold to 2.

This is the cascade router pattern applied to conductor thresholds:
the system learns which thresholds produce the best outcomes.

---

## Precision-Weighted Prediction Errors

The Good Regulator framework connects to precision-weighted prediction
errors from active inference theory:

**Prediction**: The model predicts what healthy execution looks like
(thresholds define the prediction).

**Prediction error**: The difference between predicted (healthy) and
observed (actual) behavior. Each watcher computes a prediction error:
"I predicted the agent would produce output; it produced none."

**Precision weighting**: Not all prediction errors are equally
informative. Prediction errors on familiar tasks (tasks with many
historical episodes) should be weighted more heavily — the model
is confident in its prediction, so a deviation is surprising and
informative. Prediction errors on novel tasks (no similar episodes)
should be weighted less — the model is uncertain, so a deviation is
expected.

**Familiar task failure = high-precision error**: The model has seen
many similar tasks succeed. When this task fails, the failure is
surprising and should trigger strong learning (update the model
significantly).

**Novel task failure = low-precision error**: The model has no
experience with this type of task. Failure is not surprising and
should trigger weak learning (update the model cautiously).

This precision weighting prevents the model from over-reacting to
novel task failures (which might be one-off anomalies) while ensuring
it reacts strongly to familiar task failures (which indicate a real
change in the system's behavior).

**Implementation path**: The cascade router's observation count per
context provides the precision signal. Contexts with many observations
have high precision. Contexts with few observations have low precision.
The conductor could use this same signal to weight its threshold
adjustments.

Reference: This framework draws on Song et al. (ICLR 2025) on
self-improvement convergence: systems improve when the verifier's
precision exceeds the generator's. The conductor's precision (accuracy
of its self-model) must exceed the agent's variety (range of failure
modes) for the feedback loop to converge toward healthy execution.

---

## The Model Gap

The self-model is always incomplete. The six stuck kinds do not
cover all possible stuck modes. The 20 error categories do not cover
all possible errors. The 34 patterns do not match all possible error
messages.

This incompleteness is inherent — a complete model would be as complex
as the system itself (a consequence of Ashby's Law). The practical
response is:

1. **Default handling**: Unknown errors fall through to generic
   categories (CompileError → RetryWithContext). The system has a
   response even when the model does not have a specific classification.

2. **Error logging**: Every error that does not match a specific
   pattern is logged with full context. These unmatched errors are
   candidates for new patterns.

3. **Model expansion**: New patterns and categories are added as new
   error types are encountered in production. The model grows toward
   completeness over time.

4. **Learning integration**: The efficiency tracking system records
   all errors, including unclassified ones. Over time, clustering of
   unclassified errors reveals new categories that the model should
   include.

---

## Recursive Self-Modeling

The meta-cognition hook introduces a recursive element: the system
models its own modeling process.

```
Level 0: Agent executes task
Level 1: Watchers model agent execution
Level 2: MetaCognitionHook models watcher effectiveness
```

The meta-cognition hook asks: "Am I stuck?" This is a second-order
question — it is the system asking about the effectiveness of its
own first-order monitoring.

In principle, this recursion could continue (Level 3: "Is my
meta-cognition effective?"), but in practice two levels suffice.
The law of diminishing returns applies: each level of meta-cognition
adds complexity but decreasing diagnostic value.

---

## File Reference

| File | What |
|------|------|
| `crates/roko-conductor/src/conductor.rs` | The self-model instantiation (Conductor::new() creates 10 watchers) |
| `crates/roko-conductor/src/stuck_detection.rs` | Process pattern model (6 stuck heuristics) |
| `crates/roko-conductor/src/diagnosis.rs` | Failure taxonomy (20 categories, 34 patterns) |
| `crates/roko-conductor/src/health.rs` | Infrastructure expectation model (4 checks) |
| `crates/roko-learn/src/efficiency.rs` | Data source for model calibration |
| `crates/roko-gate/src/adaptive_threshold.rs` | Adaptive model precedent (EMA thresholds) |
