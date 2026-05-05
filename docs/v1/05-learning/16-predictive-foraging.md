# Predictive Foraging

> **PRD source:** `refactoring-prd/09-innovations.md` §VII
> **Implementation plan:** `modelrouting/12-advanced-patterns.md` (tasks 2J.04–2J.06)
> **Theoretical basis:** Optimal Foraging Theory (MacArthur & Pianka 1966), Calibration (Gneiting & Raftery 2007)
> **Cross-references:** [04-cascade-router](04-cascade-router.md), [15-collective-calibration-31x](15-collective-calibration-31x.md), [14-stability-mechanisms](14-stability-mechanisms.md), [18-self-learning-cybernetic-loops](18-self-learning-cybernetic-loops.md)


> **Implementation**: Shipping

---

## Purpose

Predictive Foraging turns every orchestrator decision into a falsifiable prediction. Before each task, the system predicts: duration, complexity, gate outcome, and merge conflict probability. After execution, predictions are compared against actual outcomes. The gap between prediction and reality — the calibration error — becomes a learning signal that feeds back into the prediction models.

The name "foraging" comes from optimal foraging theory: an agent foraging for resources (information, successful outcomes) must decide where to invest its attention. A well-calibrated predictor directs foraging effort toward the highest-value opportunities, avoiding areas that look promising but consistently disappoint.

This is the task-level slice of the Bus-backed predict-publish-correct loop described in [18-self-learning-cybernetic-loops](18-self-learning-cybernetic-loops.md); see [Naming and Glossary](../00-architecture/01-naming-and-glossary.md) for the two-fabric vocabulary. See `../../tmp/refinements/10-self-learning-cybernetic-loops.md` for the full proposal. In that wider loop, `prediction.error.*` is a first-class signal family rather than just a local calibration metric.

---

## Predictions

The system makes four types of predictions at task dispatch time:

### 1. Duration Prediction

```
Prediction: "Task T3 will take approximately 45 seconds of wall time."
Actual: 78 seconds
Error: +73% (underprediction)
```

Duration predictions are computed from baseline statistics for the `(role, complexity_band)` slice (see [06-task-metrics-and-baselines](06-task-metrics-and-baselines.md)). The prediction model starts with the slice average and adjusts based on:
- Crate familiarity (familiar crates → shorter duration)
- Iteration count (retries take longer)
- Playbook rule matches (rules suggesting complexity → longer)

### 2. Complexity Prediction

```
Prediction: "Task T3 is Standard complexity."
Actual: Required 4 iterations, touched 7 files → effectively Complex
Error: Underestimated complexity
```

Complexity predictions come from the plan generator's static analysis of task specs. The calibration tracker compares the predicted complexity band against the actual execution characteristics (iterations needed, files touched, gate failures encountered).

### 3. Gate Outcome Prediction

```
Prediction: "Task T3 has 72% probability of first-attempt gate pass."
Actual: Gate failed on first attempt (compile error)
Error: Overconfident by ~22%
```

Gate predictions use the cascade router's per-model pass rate statistics adjusted by task features. A well-calibrated predictor should have its 70% predictions succeed approximately 70% of the time.

### 4. Merge Conflict Prediction

```
Prediction: "Tasks T3 and T5 have 30% probability of merge conflict (both modify roko-core/src/config.rs)."
Actual: No conflict
Error: False positive
```

Merge conflict predictions use file overlap analysis between concurrent tasks. When two tasks modify the same file, the probability of conflict increases with the number of overlapping lines.

---

## CalibrationTracker

The `CalibrationTracker` records predictions and outcomes, then computes calibration metrics:

```
CalibrationTracker {
    predictions: Vec<PredictionRecord>,
    // Each record: { prediction, actual, timestamp, context }
}
```

### Calibration Metric

For probabilistic predictions (gate outcome, conflict probability), calibration is measured as the Brier score:

```
Brier score = (1/N) × Σ (predicted_probability − actual_outcome)²
```

A perfectly calibrated predictor has Brier score 0. A predictor that always predicts 50% has Brier score 0.25 on binary outcomes. Lower is better.

### Reliability Diagram

Calibration is visualized as a reliability diagram: predictions are binned by predicted probability (0-10%, 10-20%, ..., 90-100%), and for each bin, the actual success rate is plotted. A well-calibrated predictor falls on the diagonal (predicted 70% → actual ~70%).

```
Actual %  ↑
  100% │                                     ●
       │                                 ●
   80% │                            ●
       │                        ●
   60% │                   ●        ← perfectly calibrated (diagonal)
       │              ●
   40% │         ●
       │     ●
   20% │ ●
       │
    0% └──────────────────────────────────► Predicted %
       0%   20%   40%   60%   80%  100%
```

### Arithmetic Corrector

When calibration error is detected, the system applies a simple arithmetic correction:

```
corrected_prediction = raw_prediction × correction_factor
```

The correction factor is computed from historical calibration data:

```
correction_factor = actual_mean / predicted_mean
```

For example, if the system consistently predicts 70% pass rate but observes 55% actual pass rate, the correction factor is 55/70 ≈ 0.786. Future raw predictions of 70% become corrected predictions of 55%.

This arithmetic correction runs in approximately **50 nanoseconds** — negligible overhead per decision. Despite its simplicity, it captures the dominant source of miscalibration (systematic bias) without requiring complex recalibration models.

---

## Prediction as Learning Signal

The key insight of predictive foraging is that **prediction errors are more informative than raw outcomes**. A task that fails is one data point. A task that was predicted to succeed with 90% confidence but failed is a strong signal that the prediction model is miscalibrated for this type of task.

This creates a higher-order learning loop:

```
Level 0: Task outcome (pass/fail)
    │
    ▼
Level 1: Was the prediction correct? (calibration error)
    │
    ▼
Level 2: Is the prediction model systematically biased? (calibration drift)
    │
    ▼
Level 3: Are the features used for prediction informative? (feature importance)
```

Each level produces a distinct learning signal:
- Level 0 updates the bandit arm (standard reward).
- Level 1 updates the calibration correction factor.
- Level 2 triggers prediction model retraining (or feature engineering).
- Level 3 informs the next round of system design improvements.

---

## Integration with Routing

Calibrated predictions improve routing decisions:

```
Task T3: predicted gate pass probability = 0.55 (after calibration)
    │
    ▼
CascadeRouter: Low confidence → prefer stronger model
    │
    ▼
Routes to claude-opus-4 instead of claude-sonnet-4
```

Without calibration, the raw predicted probability might be 0.72, leading the router to use a weaker (cheaper) model. The calibrated prediction of 0.55 correctly identifies this as a risky task that benefits from a stronger model.

---

## Foraging Strategy

Optimal foraging theory suggests allocating effort proportional to expected return. In the agent context:

| Predicted Outcome | Foraging Strategy |
|-------------------|-------------------|
| High pass probability, low cost | Quick execution — use cheapest model |
| High pass probability, high cost | Standard execution — optimize for cost |
| Low pass probability, low cost | Speculative execution — try cheap model first |
| Low pass probability, high cost | Careful execution — invest in thorough prompting |

The cascade router implements this strategy through its C-Factor-driven bias: high-confidence tasks get cheaper models, low-confidence tasks get stronger models.

---

## Surface Predictions in TUI

Predictions are surfaced in the dashboard (see [16-heartbeat](../16-heartbeat/INDEX.md)):

```
Plan X: predicted completion in 4 minutes (based on similar plans)
Plan Y: HIGH risk of gate failure (low affordance code, no tests in target files)
Plans A and B: 30% chance of merge conflict (both modify roko-core/src/event.rs)
```

This gives the operator forward-looking diagnostics, enabling proactive intervention instead of reactive debugging.

---

## Performance

The predictive foraging pipeline adds minimal overhead:

| Operation | Cost | When |
|-----------|------|------|
| Generate predictions | ~1μs | Before each task dispatch |
| Calibration correction | ~50ns | Per prediction |
| Record prediction + outcome | ~10μs (JSONL append) | After each task |
| Recalibrate correction factor | ~100μs | Every 50 episodes |

Total per-task overhead: < 15μs. This is negligible compared to the agent execution time (typically 10-120 seconds per task).

---

## Practical Example

### Before Calibration

The system predicts gate pass probabilities based on raw per-model statistics:

```
Task T7: modify roko-core/src/config/schema.rs
    Model: claude-sonnet-4
    Raw prediction: 72% gate pass probability
    → Router: 72% is above threshold → use sonnet (cheaper)
    Actual: gate FAILED (compile error in config)
```

Over 50 tasks, the raw predictor shows systematic overconfidence:

```
Predicted 70-80% range: 45 tasks
    Actual pass rate: 55%
    Expected pass rate: ~75%
    Bias: +20% overconfident
```

### After Calibration

The arithmetic corrector adjusts:

```
correction_factor = 55% / 75% = 0.733
```

Now for Task T107:

```
Task T107: modify roko-core/src/config/schema.rs
    Model: claude-sonnet-4
    Raw prediction: 72%
    Corrected prediction: 72% × 0.733 = 52.8%
    → Router: 52.8% is below threshold → use opus (stronger)
    Actual: gate PASSED (opus handles config changes correctly)
```

The calibrated prediction correctly identifies this as a risky task, causing the router to invest in a stronger model. The cost of using opus ($1.38) is lower than the cost of a failed sonnet attempt plus retry ($0.78 + $1.38 = $2.16).

### Calibration Improves Over Time

As the corrector accumulates more data, its bias estimate becomes more precise. After 200 tasks, per-category correction factors emerge:

```
Category: config_modification
    correction_factor: 0.733 (overconfident on config tasks)

Category: test_scaffolding
    correction_factor: 1.05 (slightly underconfident on test tasks)

Category: cross_crate_refactor
    correction_factor: 0.62 (very overconfident on cross-crate tasks)
```

Per-category correction captures the observation that prediction accuracy varies by task type: the system is well-calibrated for test scaffolding but systematically overconfident for cross-crate refactoring.

---

## Relationship to Other Documents

- **[04-cascade-router](04-cascade-router.md)** — Calibrated predictions inform routing bias.
- **[06-task-metrics-and-baselines](06-task-metrics-and-baselines.md)** — Baselines provide the raw data for duration and complexity predictions.
- **[14-stability-mechanisms](14-stability-mechanisms.md)** — Calibration correction acts as a damping mechanism for prediction-based routing.
- **[15-collective-calibration-31x](15-collective-calibration-31x.md)** — Collective calibration is the aggregate-level version of individual prediction calibration.
- **[17-adas-and-autocatalytic](17-adas-and-autocatalytic.md)** — Predictive foraging is one of the 14 frontier innovations in the Roko innovation roadmap.
