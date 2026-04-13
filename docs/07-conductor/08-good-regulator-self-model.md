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

---

## Self-Model Accuracy Metrics

The self-model is only useful if it is accurate. A model that
misclassifies healthy agents as stuck, or predicts gate outcomes no
better than chance, degrades the system rather than regulating it.
This section defines formal accuracy measurements for each component
of the conductor's internal model.

### Prediction error metrics

Each metric measures the divergence between what the model predicted
and what actually happened:

```rust
/// Accuracy metrics for the conductor's self-model.
/// Each metric measures the divergence between predicted and observed behavior.
pub struct SelfModelAccuracy {
    /// Watcher threshold accuracy: fraction of interventions that improve outcomes.
    /// Computed as: (restarts where next attempt succeeds) / (total restarts).
    pub intervention_effectiveness: f64,

    /// Stuck detection precision: fraction of stuck detections that were genuine.
    /// Computed as: (stuck detections where agent was truly non-progressing) / (total detections).
    pub stuck_detection_precision: f64,

    /// Error classification accuracy: fraction of diagnoses with correct category.
    /// Computed as: (correct categories) / (total diagnoses).
    pub diagnosis_accuracy: f64,

    /// Prediction error on task completion time.
    /// RMSE between predicted and actual completion duration.
    pub completion_time_rmse_ms: f64,

    /// Prediction error on gate pass probability.
    /// Brier score: mean((predicted_pass_prob - actual_pass)^2).
    pub gate_pass_brier_score: f64,

    /// Overall model quality: harmonic mean of component accuracies.
    pub composite_accuracy: f64,
}
```

### Per-component accuracy tracking

Each model component makes a specific prediction that can be compared
against a concrete observation:

| Model Component | Prediction | Observation | Metric |
|----------------|-----------|------------|--------|
| Watcher thresholds | "This behavior is pathological" | Did restart improve outcome? | Intervention effectiveness |
| Stuck heuristics | "Agent is stuck" | Was the agent truly non-progressing? | Detection precision |
| Error categories | "This is an ImportError" | Was auto-fix successful? | Classification accuracy |
| Phase timeouts | "Task should finish in 300s" | Actual completion time | RMSE |
| Cost budgets | "Plan should cost < $10" | Actual plan cost | Mean absolute error |

The key constraint: every prediction needs a paired observation.
Predictions without observable outcomes cannot be calibrated. For
this reason, each metric above is defined in terms of an outcome
the system can actually measure after the fact.

### Brier score for calibration

The Brier score measures whether the model's confidence matches
reality. When the model says "80% chance of gate pass," do 80% of
those attempts actually pass?

Formally: `BS = (1/N) * sum((p_i - o_i)^2)` where p_i is the
predicted probability and o_i is the outcome (0 or 1). Perfect
calibration yields BS = 0. Random guessing yields BS = 0.25.

```rust
/// Brier score calculator for model calibration assessment.
/// Measures whether predicted probabilities match observed frequencies.
pub struct BrierScoreTracker {
    /// Running sum of squared errors.
    sum_squared_error: f64,
    /// Total predictions tracked.
    count: usize,
    /// Calibration bins: (predicted_prob_range, actual_pass_count, total_count).
    calibration_bins: Vec<CalibrationBin>,
}

pub struct CalibrationBin {
    pub range_low: f64,
    pub range_high: f64,
    pub actual_passes: usize,
    pub total: usize,
}

impl BrierScoreTracker {
    pub fn record(&mut self, predicted_prob: f64, actual_outcome: bool) {
        let outcome = if actual_outcome { 1.0 } else { 0.0 };
        self.sum_squared_error += (predicted_prob - outcome).powi(2);
        self.count += 1;
        // Update calibration bin
        for bin in &mut self.calibration_bins {
            if predicted_prob >= bin.range_low && predicted_prob < bin.range_high {
                bin.total += 1;
                if actual_outcome { bin.actual_passes += 1; }
                break;
            }
        }
    }

    pub fn brier_score(&self) -> f64 {
        if self.count == 0 { return 0.25; }
        self.sum_squared_error / self.count as f64
    }
}
```

Calibration bins enable a finer diagnostic: split predictions into
ranges (0.0-0.1, 0.1-0.2, ... 0.9-1.0) and compare the actual pass
rate within each bin against the predicted range. A well-calibrated
model has actual rates that match the bin midpoints.

---

## Self-Model Learning — Online Adaptation

A static self-model drifts as the system evolves. New model versions
produce different error patterns. Codebase changes alter what "normal"
spec drift looks like. Task complexity shifts change typical iteration
counts. The conductor needs to adapt its model online, without
operator intervention.

### Bayesian threshold adaptation

Each watcher threshold encodes a belief: "interventions at this
threshold improve outcomes." The conductor can track this belief as
a Beta distribution and update it from observed intervention results.

The update rule:
- Intervention fires and the restarted agent succeeds: alpha += 1 (evidence that the threshold is useful)
- Intervention fires and the restarted agent also fails: beta += 1 (evidence that the threshold is too aggressive)
- The threshold is well-calibrated when alpha / (alpha + beta) approximates the target precision (e.g., 0.8)

```rust
/// Bayesian threshold adaptation for conductor watchers.
/// Tracks whether interventions triggered at each threshold improve outcomes.
pub struct ThresholdLearner {
    /// Per-watcher threshold performance tracking.
    watchers: HashMap<String, ThresholdPosterior>,
}

pub struct ThresholdPosterior {
    /// Current threshold value.
    pub threshold: f64,
    /// Beta distribution parameters for intervention success rate.
    pub alpha: f64,  // successful interventions (restart led to success)
    pub beta: f64,   // unsuccessful interventions (restart led to failure)
    /// Discount factor for non-stationarity (default: 0.995).
    pub discount: f64,
    /// Minimum effective sample size before adapting (default: 10).
    pub min_samples: f64,
}

impl ThresholdPosterior {
    pub fn record_outcome(&mut self, intervention_helped: bool) {
        // Discount old observations for non-stationarity
        self.alpha = 1.0 + (self.alpha - 1.0) * self.discount;
        self.beta = 1.0 + (self.beta - 1.0) * self.discount;
        // Update with new observation
        if intervention_helped {
            self.alpha += 1.0;
        } else {
            self.beta += 1.0;
        }
    }

    /// Estimated intervention success rate (posterior mean).
    pub fn success_rate(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Effective sample size (accounting for discounting).
    pub fn effective_samples(&self) -> f64 {
        self.alpha + self.beta - 2.0
    }

    /// Should the threshold be tightened (lower) or loosened (higher)?
    pub fn threshold_adjustment(&self) -> ThresholdDirection {
        if self.effective_samples() < self.min_samples {
            return ThresholdDirection::Hold;
        }
        let rate = self.success_rate();
        if rate > 0.85 {
            // Interventions are too successful — threshold may be too lenient
            // (catching only obvious cases). Consider tightening.
            ThresholdDirection::Tighten
        } else if rate < 0.5 {
            // Interventions are mostly unsuccessful — threshold may be too strict
            // (false positives). Consider loosening.
            ThresholdDirection::Loosen
        } else {
            ThresholdDirection::Hold
        }
    }
}

pub enum ThresholdDirection { Tighten, Loosen, Hold }
```

The discount factor (0.995) causes old observations to decay
gradually, so the posterior tracks non-stationary behavior. Without
discounting, early observations would dominate indefinitely, and the
model would resist adaptation as the system evolves.

### Kalman filter for state estimation

System parameters drift over time: baseline error rates shift as the
codebase grows, typical costs change as model pricing evolves, and
completion times vary as task complexity changes. A scalar Kalman
filter provides online estimation that balances the existing model
against new observations.

```rust
/// Simplified scalar Kalman filter for online parameter estimation.
/// Used to track slowly-drifting system parameters (baseline error rate, typical cost).
pub struct ScalarKalman {
    /// Current state estimate.
    pub estimate: f64,
    /// Estimation uncertainty (variance).
    pub uncertainty: f64,
    /// Process noise: how much the true value can drift per step.
    pub process_noise: f64,
    /// Measurement noise: how noisy observations are.
    pub measurement_noise: f64,
}

impl ScalarKalman {
    pub fn update(&mut self, observation: f64) {
        // Predict step: uncertainty grows by process noise
        self.uncertainty += self.process_noise;
        // Update step: incorporate observation
        let kalman_gain = self.uncertainty / (self.uncertainty + self.measurement_noise);
        self.estimate += kalman_gain * (observation - self.estimate);
        self.uncertainty *= 1.0 - kalman_gain;
    }

    /// Prediction error (how surprising was the last observation?).
    pub fn prediction_error(&self, observation: f64) -> f64 {
        (observation - self.estimate).abs()
    }
}
```

The Kalman gain is the key: when uncertainty is high relative to
measurement noise, the filter trusts new observations more. When
uncertainty is low, it trusts the existing estimate. This provides
the same "precision weighting" behavior described in the
precision-weighted prediction errors section, but through a
classical filtering framework rather than an active inference one.

### Active inference integration

The precision-weighted prediction error framework described earlier
in this document provides the mechanism; the active inference
integration provides the weighting. Errors from reliable sources
drive large model updates. Errors from noisy sources drive small ones.

Precision is derived from the cascade router's observation count per
context. A context with 200 observations has high precision (the model
knows what to expect). A context with 3 observations has low precision
(the model is guessing).

```rust
/// Precision-weighted model update inspired by active inference.
/// Errors from reliable sources drive large updates; noisy sources drive small ones.
pub struct PrecisionWeightedUpdater {
    /// Per-context precision estimates (inverse variance of prediction errors).
    context_precision: HashMap<String, f64>,
    /// Minimum precision (prevents zero-weight on novel contexts).
    min_precision: f64,  // default: 0.1
    /// Maximum precision (prevents over-confidence on familiar contexts).
    max_precision: f64,  // default: 10.0
}

impl PrecisionWeightedUpdater {
    /// Update the model with a precision-weighted prediction error.
    pub fn weighted_update(
        &self,
        context: &str,
        prediction_error: f64,
        base_learning_rate: f64,
    ) -> f64 {
        let precision = self.context_precision
            .get(context)
            .copied()
            .unwrap_or(self.min_precision)
            .clamp(self.min_precision, self.max_precision);

        // Learning rate scales with precision: precise contexts drive larger updates
        base_learning_rate * precision * prediction_error
    }
}
```

The min/max precision bounds prevent two failure modes. Without a
minimum, novel contexts would produce zero-weight updates and the
model would never learn about new task types. Without a maximum,
familiar contexts would dominate all updates and the model would
over-fit to historical patterns even when the underlying system has
changed.

---

## The Internal Model Principle and Forward Prediction

### Francis-Wonham (1976) — The Internal Model Principle

The Internal Model Principle (IMP) strengthens Conant-Ashby. Where
the Good Regulator theorem says "a good regulator must contain a
model," the IMP says the controller must contain a copy of the
dynamics generating the signals it must track. Not just any model —
the same dynamical structure.

For the conductor, this has a concrete implication: the conductor's
model of gate outcomes must mirror the actual gate pipeline's logic.
If the gate pipeline runs compile, test, clippy, and diff checks in
sequence, the conductor's forward model must predict the outcome of
each check in that same sequence. If a new gate is added (say, a
security audit gate), the conductor's model must incorporate it or
regulation degrades — the conductor cannot anticipate failures from
a gate it does not model.

This is testable: when the gate pipeline changes and the conductor's
model does not update, intervention effectiveness should drop
measurably. The Brier score on gate pass prediction should increase
(worsen). The Bayesian threshold posteriors should shift toward
higher beta (more unsuccessful interventions). These signals indicate
that the internal model has diverged from the system it regulates.

### Forward prediction

The self-model enables prediction, and prediction enables anticipatory
intervention. Instead of waiting for a watcher to trigger (reactive),
the conductor can predict future state and intervene before a failure
materializes (proactive).

Given current execution state — iteration count, accumulated cost,
error count, time elapsed — the forward predictor estimates the
probability that the next gate attempt will pass. If that probability
drops below a threshold (e.g., 0.3), the conductor can preemptively
trigger a strategy change: switch to a stronger model, enrich the
context, or restructure the approach.

```rust
/// Forward prediction using the conductor's self-model.
/// Predicts future state to enable anticipatory intervention.
pub struct ForwardPredictor {
    /// Learned mapping: (current_state) -> (predicted_next_state).
    /// Implemented as linear regression on state features.
    weights: Vec<f64>,
    /// Bias term.
    bias: f64,
    /// Feature extractor: signal stream -> state features.
    feature_dim: usize,
}

impl ForwardPredictor {
    /// Predict the probability of gate pass given current execution state.
    pub fn predict_pass_probability(&self, features: &[f64]) -> f64 {
        assert_eq!(features.len(), self.feature_dim);
        let logit: f64 = self.bias + features.iter()
            .zip(self.weights.iter())
            .map(|(f, w)| f * w)
            .sum::<f64>();
        // Sigmoid to get probability
        1.0 / (1.0 + (-logit).exp())
    }

    /// Online update via stochastic gradient descent.
    pub fn update(&mut self, features: &[f64], actual_pass: bool, lr: f64) {
        let predicted = self.predict_pass_probability(features);
        let target = if actual_pass { 1.0 } else { 0.0 };
        let error = predicted - target;
        // SGD update
        self.bias -= lr * error;
        for (w, f) in self.weights.iter_mut().zip(features.iter()) {
            *w -= lr * error * f;
        }
    }
}
```

The feature vector for prediction includes: current iteration number,
cumulative cost so far, error count in this attempt, time elapsed as
a fraction of timeout, context window usage ratio, and the cascade
router's model confidence for this task type. These features are
available at every point during execution, so prediction is continuous
rather than point-in-time.

### World model vs. self-model

Two distinct models operate inside the conductor, and conflating them
leads to calibration errors:

- **World model**: What will the environment do? This predicts gate
  outcomes, compile results, test results — things external to the
  conductor. The world model answers: "Will this code pass the test
  suite?"

- **Self-model**: What will this system do? This predicts watcher
  behavior, intervention effectiveness, threshold accuracy — things
  internal to the conductor. The self-model answers: "Will my
  intervention improve the outcome?"

Both are needed. The world model predicts what agents will face.
The self-model predicts how the conductor will respond. The IMP
constrains the world model: it must contain the dynamics of the gate
pipeline. The Good Regulator theorem constrains the self-model: it
must contain the dynamics of the watcher ensemble.

A conductor that has a good world model but a poor self-model will
accurately predict failures but respond to them badly (wrong
intervention, wrong timing). A conductor that has a good self-model
but a poor world model will respond well to predicted failures but
miss the ones it did not predict. Regulation quality depends on both.

### References

- Conant, R.C. & Ashby, W.R. (1970). "Every good regulator of a system must be a model of that system." *International Journal of Systems Science*, 1(2), 89-97.
- Francis, B.A. & Wonham, W.M. (1976). "The Internal Model Principle of Control Theory." *Automatica*, 12(5), 457-465.
- Friston, K. (2010). "The free-energy principle: a unified brain theory?" *Nature Reviews Neuroscience*, 11(2), 127-138.
- Chen, B. et al. (2022). "Full-Body Visual Self-Modeling of Robot Morphologies." *Science Robotics*, 7(68).
- Kalman, R.E. (1960). "A New Approach to Linear Filtering and Prediction Problems." *Journal of Basic Engineering*, 82(1), 35-45.
- Song, Y. et al. (2025). "The Good, the Bad, and the Greedy: Evaluation of LLMs Should Not Ignore Non-Determinism." *ICLR 2025*.
