# Adaptive Supervision Loop

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). How self-model accuracy, threshold adaptation, and pressure management emerge as nested Loops with predict-publish-correct at every level.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal/Pulse duality, demurrage), [02-CELL](../../unified/02-CELL.md) (9 protocols, predict-publish-correct, Verify redesign, EFE routing), [05-AGENT](../../unified/05-AGENT.md) (Agent lifecycle, vitality, cognitive timescales), [07-LEARNING](../../unified/07-LEARNING.md) (L1-L4 loop taxonomy, predict-publish-correct), [16-diagnosis-and-stuck-detection.md](16-diagnosis-and-stuck-detection.md) (Diagnosis Route, Stuck Lens Cells, MetaCognition Loop)

**Source docs**: `docs/07-conductor/08-good-regulator-self-model.md`, `docs/07-conductor/12-yerkes-dodson-pressure.md`, `docs/07-conductor/15-conductor-learning-federation.md`

---

## 1. The Redesign Thesis

The original conductor uses static thresholds. `MAX_GHOST_TURNS = 3`. `MAX_COMPILE_FAIL_REPEAT = 3`. These constants were calibrated from production batch runs. They work for that workload. But workloads change, model versions change, codebase complexity changes. A threshold calibrated for Sonnet 3.5 may be too strict for Sonnet 4 or too lenient for Haiku.

The unified redesign makes five structural changes:

1. **Self-model accuracy is a Lens.** Brier score, intervention effectiveness, stuck detection precision -- all are read-only observations published as Pulses. The Lens does not take action. Downstream React Cells decide interventions based on what the Lens reports.

2. **Threshold adaptation is predict-publish-correct.** Each threshold predicts "behavior beyond this point is pathological." Reality publishes the outcome (did the intervention help?). A Beta distribution corrects the threshold. This is the same pattern that drives calibration throughout the system (see [07-LEARNING.md](../../unified/07-LEARNING.md)).

3. **Yerkes-Dodson pressure is a Score Cell.** It computes a scalar pressure index from multi-dimensional inputs. The Route Cell uses the pressure score to modulate watcher sensitivity. High pressure relaxes stuck detection to avoid collapse. Low pressure tightens it to avoid drift.

4. **Flow detection is a Lens.** It observes agent trajectory for flow indicators (consistent file changes, improving gate scores, diverse tools, moderate context usage). When flow is detected, it publishes a Pulse that downstream Cells use to raise thresholds.

5. **Triple-loop learning is three nested Loops.** Single-loop corrects errors. Double-loop changes thresholds. Triple-loop changes the learning rate. Each Loop operates at an increasing timescale, and each implements predict-publish-correct independently.

The consequence: supervision is not a separate subsystem bolted onto the agent runtime. It is a set of Cells in Graphs, subject to the same execution semantics, composing with the same primitives, and learning through the same predict-publish-correct pattern as everything else.

---

## 2. Self-Model Accuracy as a Lens

The self-model is the conductor's representation of what healthy execution looks like. Its accuracy determines whether interventions help or hurt. The SelfModelLens observes five accuracy metrics and publishes them as Pulses.

### 2.1 The Five Accuracy Metrics

```rust
/// Lens Cell: observes the accuracy of the conductor's self-model.
///
/// Conforms to: Observe protocol
/// Subscribes to: "conductor.intervention.*", "gate.verdict.*",
///                "agent.turn.*" Pulses on Bus
/// Publishes to: "telemetry.self_model.accuracy"
///
/// Read-only. Does not modify the self-model.
/// Downstream React Cells use these observations to trigger adaptation.
pub struct SelfModelLens {
    id: CellId,
    /// Brier score tracker for gate pass predictions.
    brier: BrierScoreTracker,
    /// Per-watcher intervention outcome tracking.
    intervention_outcomes: HashMap<String, (u64, u64)>,  // (successes, total)
    /// Stuck detection true/false positive tracking.
    stuck_outcomes: (u64, u64),  // (true_positives, total_detections)
    /// Diagnosis classification accuracy tracking.
    diagnosis_outcomes: (u64, u64),  // (correct, total)
}

/// The five accuracy metrics, published as a Pulse.
/// Each measures the divergence between what the model predicted
/// and what actually happened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfModelAccuracy {
    /// Fraction of interventions that improved outcomes.
    /// (restarts where next attempt succeeds) / (total restarts).
    pub intervention_effectiveness: f64,

    /// Fraction of stuck detections that were genuine.
    /// (true positives) / (total detections).
    pub stuck_detection_precision: f64,

    /// Fraction of diagnoses with correct category.
    /// (auto-fix succeeded for classified category) / (total auto-fixes).
    pub diagnosis_accuracy: f64,

    /// Brier score on gate pass probability predictions.
    /// BS = (1/N) * sum((predicted - actual)^2).
    /// Perfect = 0.0, random = 0.25.
    pub gate_pass_brier_score: f64,

    /// Harmonic mean of the four component accuracies.
    /// Harmonic mean penalizes low outliers -- one bad component
    /// drags the composite down more than arithmetic mean would.
    pub composite_accuracy: f64,
}
```

### 2.2 Brier Score: Is the Model Well-Calibrated?

The Brier score measures whether the model's confidence matches reality. When the model says "80% chance of gate pass," do 80% of those attempts actually pass?

```rust
/// Brier score calculator with calibration bins.
/// Measures whether predicted probabilities match observed frequencies.
pub struct BrierScoreTracker {
    /// Running sum of squared errors: sum((p_i - o_i)^2).
    sum_squared_error: f64,
    /// Total predictions tracked.
    count: usize,
    /// Calibration bins for diagnostic granularity.
    /// 10 bins covering [0.0, 0.1), [0.1, 0.2), ..., [0.9, 1.0].
    calibration_bins: [CalibrationBin; 10],
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
        let bin_idx = ((predicted_prob * 10.0) as usize).min(9);
        self.calibration_bins[bin_idx].total += 1;
        if actual_outcome {
            self.calibration_bins[bin_idx].actual_passes += 1;
        }
    }

    /// Brier score: 0.0 = perfect, 0.25 = random.
    pub fn brier_score(&self) -> f64 {
        if self.count == 0 { return 0.25; }
        self.sum_squared_error / self.count as f64
    }

    /// Is the model well-calibrated?
    /// Checks each bin: does the actual pass rate match the predicted range?
    pub fn calibration_error(&self) -> f64 {
        let mut total_error = 0.0;
        let mut total_weight = 0.0;
        for bin in &self.calibration_bins {
            if bin.total >= 5 {  // minimum sample size per bin
                let midpoint = (bin.range_low + bin.range_high) / 2.0;
                let actual_rate = bin.actual_passes as f64 / bin.total as f64;
                total_error += (midpoint - actual_rate).abs() * bin.total as f64;
                total_weight += bin.total as f64;
            }
        }
        if total_weight < f64::EPSILON { return 0.0; }
        total_error / total_weight
    }
}
```

Calibration bins enable a finer diagnostic than the scalar Brier score. Split predictions into 10 ranges and compare the actual pass rate within each bin against the bin midpoint. A well-calibrated model has actual rates that match midpoints. A model that is overconfident shows high actual failure rates in high-prediction bins. A model that is underconfident shows high actual pass rates in low-prediction bins.

### 2.3 The Lens Cell Implementation

```rust
impl Cell for SelfModelLens {
    fn id(&self) -> CellId { self.id }
    fn name(&self) -> &str { "self-model-lens" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Observe] }
    fn capabilities(&self) -> &Capabilities { Capabilities::read_only() }
    fn estimated_cost(&self) -> Cost { Cost::ZERO }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Process incoming outcome signals
        for signal in &input {
            match signal.kind() {
                Kind::InterventionOutcome => {
                    let watcher: String = signal.payload_str("watcher")
                        .unwrap_or_default().to_string();
                    let helped: bool = signal.payload_parse("improved")?;
                    let entry = self.intervention_outcomes
                        .entry(watcher).or_insert((0, 0));
                    entry.1 += 1;
                    if helped { entry.0 += 1; }
                }
                Kind::StuckDetectionOutcome => {
                    let true_positive: bool = signal.payload_parse("genuine")?;
                    self.stuck_outcomes.1 += 1;
                    if true_positive { self.stuck_outcomes.0 += 1; }
                }
                Kind::DiagnosisOutcome => {
                    let correct: bool = signal.payload_parse("correct")?;
                    self.diagnosis_outcomes.1 += 1;
                    if correct { self.diagnosis_outcomes.0 += 1; }
                }
                Kind::GateVerdict => {
                    let predicted: f64 = signal.payload_parse("predicted_pass_prob")
                        .unwrap_or(0.5);
                    let actual: bool = signal.payload_parse("passed")?;
                    self.brier.record(predicted, actual);
                }
                _ => {}
            }
        }

        // Compute accuracy metrics
        let ie = compute_ratio(self.intervention_outcomes.values()
            .map(|(s, t)| (*s, *t))
            .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1)));
        let sdp = compute_ratio(self.stuck_outcomes);
        let da = compute_ratio(self.diagnosis_outcomes);
        let brier = self.brier.brier_score();

        // Convert Brier to [0,1] accuracy (1.0 = perfect, 0.0 = random)
        let brier_accuracy = 1.0 - (brier / 0.25).min(1.0);

        let composite = harmonic_mean(&[ie, sdp, da, brier_accuracy]);

        let accuracy = SelfModelAccuracy {
            intervention_effectiveness: ie,
            stuck_detection_precision: sdp,
            diagnosis_accuracy: da,
            gate_pass_brier_score: brier,
            composite_accuracy: composite,
        };

        Ok(vec![Signal::pulse(
            Kind::Telemetry,
            topic!("telemetry.self_model.accuracy"),
            accuracy,
        )])
    }
}

fn compute_ratio((successes, total): (u64, u64)) -> f64 {
    if total == 0 { return 0.5; }  // no data = maximum uncertainty
    successes as f64 / total as f64
}

fn harmonic_mean(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    let inv_sum: f64 = values.iter()
        .map(|v| if *v < f64::EPSILON { 1.0 / f64::EPSILON } else { 1.0 / v })
        .sum();
    if inv_sum < f64::EPSILON { return 0.0; }
    n / inv_sum
}
```

The Lens publishes. It does not act. Downstream cells -- the ThresholdAdaptation Loop, the self-repair React Cell -- subscribe to `telemetry.self_model.accuracy` and decide what to do.

---

## 3. Threshold Adaptation as Predict-Publish-Correct

Each watcher threshold is a prediction: "behavior beyond this point is pathological." The prediction is tested every time the watcher fires. The outcome (did the intervention improve things?) updates the threshold via a Beta distribution.

This is predict-publish-correct (see [02-CELL.md](../../unified/02-CELL.md), [07-LEARNING.md](../../unified/07-LEARNING.md)) applied to supervision thresholds.

### 3.1 The ThresholdLearner Cell

```rust
/// Loop Cell: adapts watcher thresholds based on intervention outcomes.
///
/// Implements predict-publish-correct for each watcher threshold:
///   PREDICT: The threshold predicts intervention success.
///   PUBLISH: The intervention fires and its outcome is observed.
///   CORRECT: The Beta distribution updates, and the threshold adjusts.
///
/// Conforms to: React protocol (subscribes to outcomes, emits threshold changes)
/// Operates at: L2 timescale (double-loop learning)
pub struct ThresholdLearner {
    id: CellId,
    /// Per-watcher posterior tracking.
    posteriors: HashMap<String, ThresholdPosterior>,
}

/// Beta distribution posterior for a single watcher threshold.
/// Tracks whether interventions at this threshold improve outcomes.
pub struct ThresholdPosterior {
    /// Current threshold value.
    pub threshold: f64,
    /// Beta distribution: alpha = successful interventions.
    pub alpha: f64,
    /// Beta distribution: beta = unsuccessful interventions.
    pub beta: f64,
    /// Discount factor for non-stationarity.
    /// Old observations decay at rate discount^n, so the posterior
    /// tracks recent performance rather than all-time performance.
    /// Default: 0.995 (~200 observations for half-life).
    pub discount: f64,
    /// Minimum effective sample size before adapting.
    /// Prevents adaptation on too little data.
    /// Default: 10.
    pub min_samples: f64,
    /// Bounds for the threshold (never adapt beyond these).
    pub min_threshold: f64,
    pub max_threshold: f64,
}
```

### 3.2 The Bayesian Update

```rust
impl ThresholdPosterior {
    /// Record the outcome of an intervention triggered at this threshold.
    pub fn record_outcome(&mut self, intervention_helped: bool) {
        // Step 1: Discount old observations.
        // This makes the posterior non-stationary: recent evidence
        // matters more than historical evidence. Without discounting,
        // early observations dominate indefinitely.
        self.alpha = 1.0 + (self.alpha - 1.0) * self.discount;
        self.beta = 1.0 + (self.beta - 1.0) * self.discount;

        // Step 2: Update with new observation.
        if intervention_helped {
            self.alpha += 1.0;
        } else {
            self.beta += 1.0;
        }
    }

    /// Posterior mean: estimated intervention success rate.
    pub fn success_rate(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Effective sample size (accounting for discounting).
    pub fn effective_samples(&self) -> f64 {
        self.alpha + self.beta - 2.0
    }

    /// Should the threshold tighten, loosen, or hold?
    ///
    /// The logic encodes a simple heuristic:
    ///   >0.85 success rate: threshold is too lenient. It catches only
    ///     the obvious cases. Tighten to catch more.
    ///   <0.50 success rate: threshold is too strict. Too many false
    ///     positives. Loosen to reduce false alarms.
    ///   0.50-0.85: threshold is well-calibrated. Hold.
    pub fn threshold_direction(&self) -> ThresholdDirection {
        if self.effective_samples() < self.min_samples {
            return ThresholdDirection::Hold;
        }
        let rate = self.success_rate();
        if rate > 0.85 {
            ThresholdDirection::Tighten
        } else if rate < 0.50 {
            ThresholdDirection::Loosen
        } else {
            ThresholdDirection::Hold
        }
    }

    /// Apply threshold adjustment within bounds.
    pub fn apply_adjustment(&mut self, step_size: f64) {
        match self.threshold_direction() {
            ThresholdDirection::Tighten => {
                self.threshold = (self.threshold - step_size)
                    .max(self.min_threshold);
            }
            ThresholdDirection::Loosen => {
                self.threshold = (self.threshold + step_size)
                    .min(self.max_threshold);
            }
            ThresholdDirection::Hold => {}
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ThresholdDirection {
    Tighten,
    Loosen,
    Hold,
}
```

### 3.3 Why Beta Distributions

The Beta distribution is the conjugate prior for Bernoulli observations (binary outcomes: success/failure). This means:

- The update is closed-form: just increment alpha or beta. No iterative optimization.
- The posterior is always a Beta distribution. No distributional approximation needed.
- The mean is alpha / (alpha + beta). Trivially computable.
- The variance is alpha*beta / ((alpha+beta)^2 * (alpha+beta+1)). Measures confidence.

The discount factor (0.995) introduces non-stationarity. At discount 0.995, observations from ~138 steps ago carry half their original weight (0.995^138 ~ 0.5). This means the posterior tracks a sliding window of approximately 200-300 effective observations, adapting as the system evolves.

### 3.4 The Predict-Publish-Correct Cycle

Each threshold adaptation follows the same three-phase cycle that drives all learning in the system:

```
Phase 1: PREDICT
    Threshold = 3 for ghost_turn watcher.
    Prediction: "An agent with 3+ ghost turns is stuck."

Phase 2: PUBLISH
    Ghost turn watcher fires. Conductor restarts agent.
    Outcome observed: did the restarted agent succeed?
    Outcome published on Bus as InterventionOutcome Pulse.

Phase 3: CORRECT
    ThresholdLearner subscribes to InterventionOutcome.
    If restart succeeded: alpha += 1 (threshold was right).
    If restart also failed: beta += 1 (threshold was wrong).
    Success rate computed. Threshold adjusted if outside [0.50, 0.85].
```

This cycle repeats continuously. The threshold converges toward the value that maximizes intervention effectiveness for the current workload.

---

## 4. Yerkes-Dodson Pressure as a Score Cell

Pressure is multi-dimensional: iteration fraction, cost fraction, time fraction, stuck fraction. The PressureScore Cell collapses these dimensions into a single scalar that maps to the x-axis of the Yerkes-Dodson inverted-U curve.

### 4.1 The Pressure Score Cell

```rust
/// Score Cell: computes scalar pressure index from multi-dimensional inputs.
///
/// Conforms to: Score protocol
/// Input: Signal with execution state (iteration, cost, elapsed, stuck count)
/// Output: Signal with Kind::Scored carrying pressure index in [0.0, 1.0]
///
/// The pressure index maps to the Yerkes-Dodson inverted-U:
///   0.0-0.3: Zone 1 (under-arousal) -> drift, exploration, token waste
///   0.3-0.7: Zone 2 (optimal) -> focused execution, cooperation
///   0.7-1.0: Zone 3 (over-arousal) -> collapse in 5-12 turns
pub struct PressureScore {
    id: CellId,
    /// Weights for each pressure dimension. Sum to 1.0.
    weights: PressureWeights,
}

pub struct PressureWeights {
    pub iteration: f64,  // default: 0.30
    pub cost: f64,       // default: 0.25
    pub time: f64,       // default: 0.25
    pub stuck: f64,      // default: 0.20
}

impl Cell for PressureScore {
    fn id(&self) -> CellId { self.id }
    fn name(&self) -> &str { "pressure-score" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn capabilities(&self) -> &Capabilities { Capabilities::pure() }
    fn estimated_cost(&self) -> Cost { Cost::ZERO }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut results = Vec::new();

        for signal in &input {
            let iteration: u32 = signal.payload_parse("iteration")?;
            let max_iterations: u32 = signal.payload_parse("max_iterations")?;
            let cost_usd: f64 = signal.payload_parse("cost_usd")?;
            let cost_budget: f64 = signal.payload_parse("cost_budget_usd")?;
            let elapsed_ms: u64 = signal.payload_parse("elapsed_ms")?;
            let timeout_ms: u64 = signal.payload_parse("timeout_ms")?;
            let stuck_count: u32 = signal.payload_parse("stuck_count")?;
            let stuck_threshold: u32 = signal.payload_parse("stuck_threshold")?;

            let iter_pressure = iteration as f64 / max_iterations.max(1) as f64;
            let cost_pressure = cost_usd / cost_budget.max(f64::EPSILON);
            let time_pressure = elapsed_ms as f64 / timeout_ms.max(1) as f64;
            let stuck_pressure = stuck_count as f64 / stuck_threshold.max(1) as f64;

            let pressure_index =
                self.weights.iteration * iter_pressure
                + self.weights.cost * cost_pressure
                + self.weights.time * time_pressure
                + self.weights.stuck * stuck_pressure;

            // Clamp to [0, 1]
            let pressure_index = pressure_index.clamp(0.0, 1.0);

            // Determine pressure zone
            let zone = match pressure_index {
                p if p < 0.3 => PressureZone::UnderArousal,
                p if p < 0.7 => PressureZone::Optimal,
                _ => PressureZone::OverArousal,
            };

            results.push(Signal::builder(Kind::Scored)
                .payload(json!({
                    "pressure_index": pressure_index,
                    "zone": zone,
                    "components": {
                        "iteration": iter_pressure,
                        "cost": cost_pressure,
                        "time": time_pressure,
                        "stuck": stuck_pressure,
                    },
                }))
                .source(signal.id)
                .build());
        }

        Ok(results)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PressureZone {
    /// Below optimal. Agent may drift, explore tangentially, waste tokens.
    UnderArousal,
    /// Optimal. Agent is focused and productive.
    Optimal,
    /// Above optimal. Cooperative behavior collapses in 5-12 turns.
    OverArousal,
}
```

### 4.2 The Inverted-U Curve in Agent Systems

Research on 770,000+ autonomous LLM agents demonstrates that cooperative behavior follows the Yerkes-Dodson inverted-U with environmental pressure (Yerkes & Dodson 1908):

- **Under-arousal** (Zone 1): agents drift, explore tangentially, produce verbose reasoning that burns tokens without advancing the task. Ghost turns are the extreme form.
- **Optimal** (Zone 2): agents build on each other's work, follow established patterns, produce complementary outputs. First-pass gate rates exceed 60%.
- **Over-arousal** (Zone 3): agents shift to minimal-effort strategies within 5-12 turns. They produce the simplest output that satisfies immediate constraints rather than contributing to the broader task. This is the agent equivalent of panic.

The collapse window of 5-12 turns is the critical constraint. The circuit breaker's `MAX_PLAN_FAILURES = 2` combined with `MAX_ITERATIONS = 3` creates a maximum of 6 attempts -- just at the edge of collapse. This is not coincidence; it was calibrated from the same production data.

### 4.3 Complexity-Pressure Interaction

The Yerkes-Dodson Law's most important implication: the optimal pressure level depends on task complexity. Simple tasks peak at higher pressure (more constraint helps straightforward work). Complex tasks peak at lower pressure (too much constraint degrades complex reasoning).

| Complexity | Optimal Zone Center | Phase Timeout | Iteration Budget | Rationale |
|---|---|---|---|---|
| Trivial | 0.55 | 120s | 1-2 | Tight constraints focus on the obvious solution |
| Simple | 0.50 | 180s | 2-3 | Moderate constraint with room for one retry |
| Standard | 0.45 | 300s | 3 | Default calibration; room to iterate |
| Complex | 0.35 | 600s | 3-5 | Lower pressure; room to backtrack and converge |

### 4.4 Pressure-Aware Watcher Modulation

The PressureScore output feeds into a Route Cell that modulates watcher sensitivity. When pressure is high (approaching Zone 3), watcher thresholds relax to prevent the system from pushing agents past the collapse point. When pressure is low (Zone 1), thresholds tighten to prevent drift.

```rust
/// Route Cell: modulates watcher thresholds based on pressure zone.
///
/// Conforms to: Route protocol
/// Input: PressureScore output + current watcher thresholds
/// Output: Adjusted threshold set
pub struct PressureModulationRoute {
    id: CellId,
    /// Multiplier when in Zone 3 (over-arousal). Relax thresholds.
    over_arousal_multiplier: f64,  // default: 1.5 (50% more lenient)
    /// Multiplier when in Zone 1 (under-arousal). Tighten thresholds.
    under_arousal_multiplier: f64, // default: 0.75 (25% more strict)
}

impl Cell for PressureModulationRoute {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Route] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut results = Vec::new();

        for signal in &input {
            let zone: PressureZone = signal.payload_parse("zone")?;
            let multiplier = match zone {
                PressureZone::UnderArousal => self.under_arousal_multiplier,
                PressureZone::Optimal => 1.0,
                PressureZone::OverArousal => self.over_arousal_multiplier,
            };

            results.push(Signal::builder(Kind::ThresholdAdjustment)
                .payload(json!({
                    "multiplier": multiplier,
                    "zone": zone,
                    "reason": match zone {
                        PressureZone::UnderArousal =>
                            "Pressure below optimal. Tightening to prevent drift.",
                        PressureZone::Optimal =>
                            "Pressure optimal. No adjustment.",
                        PressureZone::OverArousal =>
                            "Pressure above optimal. Relaxing to prevent collapse.",
                    },
                }))
                .source(signal.id)
                .build());
        }

        Ok(results)
    }
}
```

---

## 5. Flow Detection as a Lens

Csikszentmihalyi's flow (1975, 1990): a state of deep productive engagement where interruption is costly. When an agent is in flow, the supervision system should reduce intervention sensitivity to avoid disrupting the productive state.

### 5.1 The FlowDetector Lens

```rust
/// Lens Cell: detects sustained productive behavior (flow state).
///
/// Conforms to: Observe protocol
/// Subscribes to: "agent.turn.*" Pulses
/// Publishes to: "telemetry.flow.{agent_id}"
///
/// Flow indicators (all must be true for a turn to count as productive):
///   - Files changed > 0
///   - Context usage between 20% and 85%
///   - (Optionally) gate score trajectory improving
///   - (Optionally) tool utilization is diverse
///
/// Three consecutive productive turns = flow detected.
/// Flow threshold multiplier: 1.5 (watcher thresholds increase 50%).
///
/// CRITICAL: Flow does NOT override the circuit breaker.
/// If the circuit breaker fires (plan-level failure), flow state is
/// irrelevant. Flow preservation only affects per-turn watcher thresholds.
pub struct FlowDetectorLens {
    id: CellId,
    /// Minimum consecutive productive turns to declare flow.
    min_flow_turns: usize,  // default: 3
    /// Threshold multiplier when flow is detected.
    flow_threshold_multiplier: f64,  // default: 1.5
    /// Per-agent flow state.
    agent_flow: DashMap<AgentId, FlowState>,
}

pub struct FlowState {
    pub consecutive_productive_turns: usize,
    pub in_flow: bool,
    pub flow_started_at: Option<Instant>,
}

pub struct TurnMetrics {
    pub files_changed: usize,
    pub gate_score_improved: bool,
    pub tool_calls_diverse: bool,
    pub context_usage_ratio: f64,
}

impl TurnMetrics {
    /// A turn is productive if files changed and context usage is moderate.
    /// Conservative check: requires nonzero file changes AND moderate
    /// context usage. An agent that changes files but floods context
    /// (>85%) or barely uses it (<20%) is not in flow.
    pub fn is_productive(&self) -> bool {
        self.files_changed > 0
            && self.context_usage_ratio > 0.20
            && self.context_usage_ratio < 0.85
    }
}
```

### 5.2 Flow vs. Collapse: Observable Differences

| Signal | Flow State | Collapse State |
|---|---|---|
| Files changed per turn | Consistent, moderate (1-5) | Zero or extreme (0 or 20+) |
| Gate score trajectory | Improving or stable | Flat or declining |
| Tool utilization | Diverse, purposeful | Repetitive or absent |
| Context usage | 40-70% of window | >85% or <20% |
| Cost per meaningful change | Low, stable | High, increasing |

The key distinction: flow produces *consistent moderate output*. Collapse produces either nothing or frantic bursts. The consistency signal is what the FlowDetector Lens measures.

### 5.3 Flow-Aware Threshold Adjustment

When flow is detected, the Lens publishes a Pulse that downstream Cells use to raise watcher thresholds by the `flow_threshold_multiplier` (1.5x default):

```rust
impl Cell for FlowDetectorLens {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Observe] }
    fn capabilities(&self) -> &Capabilities { Capabilities::read_only() }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut results = Vec::new();

        for signal in &input {
            let agent_id: AgentId = signal.payload_parse("agent_id")?;
            let metrics: TurnMetrics = signal.payload_parse("turn_metrics")?;

            let mut state = self.agent_flow
                .entry(agent_id.clone())
                .or_insert(FlowState {
                    consecutive_productive_turns: 0,
                    in_flow: false,
                    flow_started_at: None,
                });

            if metrics.is_productive() {
                state.consecutive_productive_turns += 1;
                if state.consecutive_productive_turns >= self.min_flow_turns
                    && !state.in_flow
                {
                    state.in_flow = true;
                    state.flow_started_at = Some(Instant::now());

                    // Publish flow-detected Pulse
                    results.push(Signal::pulse(
                        Kind::Telemetry,
                        topic!("telemetry.flow.detected"),
                        json!({
                            "agent_id": agent_id,
                            "threshold_multiplier": self.flow_threshold_multiplier,
                            "consecutive_productive_turns":
                                state.consecutive_productive_turns,
                        }),
                    ));
                }
            } else {
                if state.in_flow {
                    // Publish flow-ended Pulse
                    results.push(Signal::pulse(
                        Kind::Telemetry,
                        topic!("telemetry.flow.ended"),
                        json!({
                            "agent_id": agent_id,
                            "duration_ms": state.flow_started_at
                                .map(|t| t.elapsed().as_millis())
                                .unwrap_or(0),
                        }),
                    ));
                }
                state.consecutive_productive_turns = 0;
                state.in_flow = false;
                state.flow_started_at = None;
            }
        }

        Ok(results)
    }
}
```

---

## 6. The ConductorBandit as a Route Cell

The ConductorBandit selects interventions from a learned policy rather than a static lookup table. It is a Route Cell with Thompson sampling internals.

### 6.1 State, Actions, and the Bandit

```rust
/// Route Cell: selects conductor interventions via contextual bandit.
///
/// Conforms to: Route protocol
/// Input: Signal with 19D state vector (extracted from watcher outputs)
/// Output: Signal with Kind::Intervention carrying the selected action
///
/// Algorithm: 65% Thompson Sampling + 35% linear context model.
/// Warmup: 50 observations before the bandit overrides static policy.
/// Fallback: static WorstSeverityPolicy when confidence < 0.6.
pub struct ConductorBanditRoute {
    id: CellId,
    /// Per-action Beta posteriors for Thompson sampling.
    arms: Vec<BanditArm>,
    /// Linear context weights for exploitation.
    context_weights: Vec<Vec<f64>>,  // [action][feature]
    /// Thompson vs. context blend ratio.
    thompson_weight: f64,  // default: 0.65
    /// Minimum observations before learning activates.
    warmup: usize,  // default: 50
    /// Minimum confidence to override static policy.
    min_confidence: f64,  // default: 0.6
    /// Total observations so far.
    total_observations: usize,
}

/// The 5 actions the bandit can select.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BanditAction {
    /// Let the agent continue without intervention.
    Continue,
    /// Inject a hint into the agent's next prompt.
    InjectHint,
    /// Switch to a different model (cheaper or more capable).
    SwitchModel,
    /// Kill and restart the agent with fresh context.
    Restart,
    /// Mark the plan as failed. Stop attempting.
    Abort,
}

pub struct BanditArm {
    pub action: BanditAction,
    pub alpha: f64,  // successes
    pub beta: f64,   // failures
    pub discount: f64,  // default: 0.995
}
```

### 6.2 The 19-Dimensional State Vector

The state vector captures execution context that drives intervention decisions:

| Feature | Dim | Range | What it captures |
|---|---|---|---|
| Iteration / max_iterations | 1 | [0, 1] | How far into the retry budget |
| Failure count | 2 | [0, ~10] | Total failures this plan attempt |
| Elapsed / timeout | 3 | [0, 1] | Time pressure |
| Cost / budget | 4 | [0, 1] | Budget pressure |
| Model tier | 5 | {0, 1, 2} | Haiku=0, Sonnet=1, Opus=2 |
| Task complexity | 6 | {0, 1, 2, 3} | Trivial/Simple/Standard/Complex |
| Error pattern hash bits | 7-12 | {0, 1} | Which error categories have appeared |
| Pressure index | 13 | [0, 1] | From PressureScore Cell |
| Flow detected | 14 | {0, 1} | From FlowDetector Lens |
| Interaction: iter x failures | 15 | [0, ~10] | Rising cost of continued retry |
| Interaction: cost x complexity | 16 | [0, ~3] | Expensive tasks need more patience |
| Interaction: elapsed x model_tier | 17 | [0, 2] | Stronger models tolerate more time |
| Stuck kind count | 18 | [0, 6] | How many stuck detectors fired |
| Self-model accuracy | 19 | [0, 1] | Composite from SelfModelLens |

Interaction terms matter because the right intervention depends on feature combinations. High iteration count alone might mean "keep trying." High iteration count combined with rising cost means "abort." The linear context model captures these interactions explicitly.

### 6.3 Thompson Sampling with Linear Context

```rust
impl Cell for ConductorBanditRoute {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Route] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut results = Vec::new();

        for signal in &input {
            let features: Vec<f64> = signal.payload_parse("features")?;
            assert_eq!(features.len(), 19, "Expected 19D state vector");

            // Warmup: defer to static policy
            if self.total_observations < self.warmup {
                results.push(self.static_fallback(signal)?);
                continue;
            }

            // Thompson sampling: sample from each arm's Beta posterior
            let mut thompson_scores: Vec<(BanditAction, f64)> = self.arms.iter()
                .map(|arm| {
                    let sample = beta_sample(arm.alpha, arm.beta);
                    (arm.action, sample)
                })
                .collect();

            // Linear context: dot product of features with per-action weights
            let context_scores: Vec<(BanditAction, f64)> = self.arms.iter()
                .enumerate()
                .map(|(i, arm)| {
                    let score: f64 = features.iter()
                        .zip(self.context_weights[i].iter())
                        .map(|(f, w)| f * w)
                        .sum();
                    (arm.action, sigmoid(score))
                })
                .collect();

            // Blend: 65% Thompson + 35% linear context
            let blended: Vec<(BanditAction, f64)> = thompson_scores.iter()
                .zip(context_scores.iter())
                .map(|((action, t), (_, c))| {
                    (*action, self.thompson_weight * t
                        + (1.0 - self.thompson_weight) * c)
                })
                .collect();

            // Select action with highest blended score
            let (action, confidence) = blended.iter()
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .copied()
                .unwrap_or((BanditAction::Continue, 0.0));

            // Low confidence: fall back to static policy
            if confidence < self.min_confidence {
                results.push(self.static_fallback(signal)?);
                continue;
            }

            results.push(Signal::builder(Kind::Intervention)
                .payload(json!({
                    "action": action,
                    "confidence": confidence,
                    "source": "conductor_bandit",
                }))
                .source(signal.id)
                .build());
        }

        Ok(results)
    }
}

fn beta_sample(alpha: f64, beta: f64) -> f64 {
    // In production: use rand_distr::Beta.
    // Pseudocode: sample from Beta(alpha, beta).
    let x: f64 = rand::random();
    // Approximate: use the posterior mean + noise scaled by variance.
    let mean = alpha / (alpha + beta);
    let var = (alpha * beta) / ((alpha + beta).powi(2) * (alpha + beta + 1.0));
    (mean + x.sqrt() * var.sqrt() * 2.0 - var.sqrt()).clamp(0.0, 1.0)
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}
```

### 6.4 Reward Shaping

The bandit learns from intervention outcomes. The reward signal must capture nuance: a well-timed Abort is a good outcome (saves tokens), not just "failure."

| Action | Outcome | Reward | Rationale |
|---|---|---|---|
| Continue | Next gate passes | 0.9 | Best case: no intervention needed, agent succeeds |
| Continue | Next gate fails | 0.1 | Passive when intervention was needed |
| InjectHint | Next gate passes | 0.85 | Lightweight intervention that worked |
| InjectHint | Next gate fails | 0.3 | At least tried to help without disrupting |
| SwitchModel | Succeeds on new model | 0.8 | Correct model routing |
| SwitchModel | Also fails | 0.2 | Model was not the issue |
| Restart | Restarted agent succeeds | 0.8 | Correct restart decision |
| Restart | Restarted agent fails | 0.2 | Problem is deeper than restart can fix |
| Abort | Plan later retried and failed again | 0.7 | Correct fail-fast saved resources |
| Abort | Plan later retried and succeeded | 0.1 | Premature abort wasted the first attempt |

Continue-pass gets 0.9, not 1.0: reserving 1.0 prevents reward saturation. Abort-correct gets 0.7: correct failure prediction is valuable but not as valuable as success. The system should prefer actions that lead to success over actions that correctly predict failure.

---

## 7. Triple-Loop Learning: Three Nested Loops

Triple-loop learning (Argyris & Schon 1978) instantiated as three nested Loop Graphs at increasing timescales.

### 7.1 The Three Loops

```
Loop 1 — Single-loop (per-task, gamma/theta timescale)
========================================================
    Agent fails -> Conductor detects -> Intervention -> Agent retries
    WHAT CHANGES: The immediate error is corrected.
    WHAT STAYS: Thresholds, learning rate, meta-parameters.
    TIMESCALE: Seconds to minutes.

Loop 2 — Double-loop (per-plan, theta/delta timescale)
========================================================
    Thresholds produce false positives or false negatives
    -> ThresholdLearner records outcomes
    -> Beta posteriors update
    -> Thresholds adjust
    WHAT CHANGES: Watcher thresholds, intervention mapping.
    WHAT STAYS: Learning rate, discount factor, min_samples.
    TIMESCALE: Minutes to hours.

Loop 3 — Triple-loop (per-session, delta timescale)
========================================================
    Double-loop oscillates (threshold goes strict -> lenient -> strict)
    -> SelfModelLens detects oscillation
    -> Learning rate adjusted (too high -> reduce, too low -> increase)
    WHAT CHANGES: Learning rate, discount factor, min_samples.
    WHAT STAYS: The architecture itself.
    TIMESCALE: Hours to days.
```

### 7.2 Single-Loop: Correct Errors

The single loop is what the conductor does today. An agent fails. The conductor detects the failure pattern. It restarts, hints, switches models, or aborts. The immediate problem is addressed. No learning occurs -- the same threshold, the same watcher, the same intervention fires next time.

This is adequate when the system and its workload are static. It is inadequate when either changes.

### 7.3 Double-Loop: Change Thresholds

The double loop is the ThresholdLearner (section 3). It tracks intervention effectiveness per watcher and adjusts thresholds. This changes the conductor's behavior for future similar situations.

```rust
/// Double-loop: adjust thresholds based on accumulated evidence.
/// Runs at delta frequency (after plan completion or batch end).
impl ThresholdLearner {
    pub fn double_loop_update(&mut self) {
        for (watcher_name, posterior) in &mut self.posteriors {
            let step = self.learning_rate * match posterior.threshold_direction() {
                ThresholdDirection::Tighten => -1.0,
                ThresholdDirection::Loosen => 1.0,
                ThresholdDirection::Hold => 0.0,
            };
            posterior.apply_adjustment(step.abs());
        }
    }
}
```

### 7.4 Triple-Loop: Change the Learning Rate

The triple loop detects when double-loop learning oscillates or fails to converge, and adjusts the learning parameters.

```rust
/// Triple-loop: adjust the learning parameters of the double-loop.
/// Runs at the end of each batch or session.
pub struct TripleLoopController {
    /// History of composite accuracy at each double-loop step.
    accuracy_history: VecDeque<f64>,
    /// Window size for oscillation detection.
    window: usize,  // default: 10
    /// Current learning rate for the double-loop.
    learning_rate: f64,
    /// Learning rate bounds.
    lr_min: f64,  // default: 0.001
    lr_max: f64,  // default: 0.1
}

impl TripleLoopController {
    /// Detect whether the double-loop is oscillating.
    /// Oscillation: accuracy alternates up-down-up-down for N steps.
    fn is_oscillating(&self) -> bool {
        if self.accuracy_history.len() < self.window {
            return false;
        }
        let recent: Vec<f64> = self.accuracy_history.iter()
            .rev()
            .take(self.window)
            .copied()
            .collect();

        // Count direction changes
        let mut changes = 0;
        for i in 1..recent.len() {
            let prev_dir = recent[i - 1] > recent.get(i.wrapping_sub(2))
                .copied().unwrap_or(recent[i - 1]);
            let curr_dir = recent[i] > recent[i - 1];
            if prev_dir != curr_dir {
                changes += 1;
            }
        }

        // If more than 60% of steps are direction changes, it is oscillating
        changes as f64 / (recent.len() - 1) as f64 > 0.6
    }

    /// Detect whether the double-loop is converging too slowly.
    /// Slow convergence: accuracy improves by less than epsilon
    /// over the last N steps.
    fn is_converging_slowly(&self) -> bool {
        if self.accuracy_history.len() < self.window {
            return false;
        }
        let first = self.accuracy_history[self.accuracy_history.len() - self.window];
        let last = *self.accuracy_history.back().unwrap_or(&0.5);
        let improvement = last - first;
        improvement < 0.01  // less than 1% improvement over the window
    }

    /// Triple-loop update: adjust the double-loop's learning rate.
    pub fn triple_loop_update(&mut self, current_accuracy: f64) {
        self.accuracy_history.push_back(current_accuracy);
        if self.accuracy_history.len() > self.window * 3 {
            self.accuracy_history.pop_front();
        }

        if self.is_oscillating() {
            // Learning rate too high -> reduce by 50%
            self.learning_rate = (self.learning_rate * 0.5).max(self.lr_min);
        } else if self.is_converging_slowly() {
            // Learning rate too low -> increase by 50%
            self.learning_rate = (self.learning_rate * 1.5).min(self.lr_max);
        }
        // Otherwise: learning rate is well-calibrated. Hold.
    }
}
```

### 7.5 The Three Loops as a Graph

```
Triple-Loop Supervision Graph
===============================

    +--------------------------------------------------+
    | Loop 3 (delta timescale)                         |
    | TripleLoopController                             |
    |   observes: L2 accuracy trend                    |
    |   adjusts: learning_rate, discount, min_samples  |
    |                                                  |
    |  +--------------------------------------------+  |
    |  | Loop 2 (theta/delta timescale)             |  |
    |  | ThresholdLearner                           |  |
    |  |   observes: intervention outcomes          |  |
    |  |   adjusts: watcher thresholds              |  |
    |  |                                            |  |
    |  |  +--------------------------------------+  |  |
    |  |  | Loop 1 (gamma/theta timescale)       |  |  |
    |  |  | Conductor (watchers + circuit breaker)|  |  |
    |  |  |   observes: agent behavior           |  |  |
    |  |  |   acts: Continue/Restart/Abort       |  |  |
    |  |  +--------------------------------------+  |  |
    |  +--------------------------------------------+  |
    +--------------------------------------------------+
```

Each loop implements predict-publish-correct independently:

| Loop | Predicts | Publishes | Corrects |
|---|---|---|---|
| L1 | "This behavior is pathological" | Intervention outcome | None (static rules today) |
| L2 | "This threshold separates healthy/stuck" | Threshold adjustment | Beta posterior on the threshold |
| L3 | "This learning rate will converge" | Learning rate adjustment | Oscillation/convergence detector |

---

## 8. Model-Specific Pressure Profiles

Different models have different Yerkes-Dodson curves. An Opus-class model tolerates more pressure before collapse. A Haiku-class model collapses sooner and steeper.

```rust
/// Per-model Yerkes-Dodson curve parameters.
/// Learned from execution history, not declared by fiat.
pub struct ModelPressureProfile {
    /// Model identifier (e.g., "claude-opus-4-6").
    pub model: String,
    /// Estimated optimal pressure level (0.0 to 1.0).
    pub optimal_pressure: f64,
    /// Estimated collapse threshold.
    pub collapse_threshold: f64,
    /// Confidence in the estimate (number of observations).
    pub observations: usize,
    /// Binned performance estimator.
    estimator: YerkesDodsonEstimator,
}

/// Online estimator for Yerkes-Dodson curve shape.
/// Maintains binned (pressure, performance) observations.
pub struct YerkesDodsonEstimator {
    /// 10 bins covering [0.0, 1.0] pressure range.
    bins: [PressureBin; 10],
    /// Total observations across all bins.
    total: usize,
    /// Minimum observations per bin before trusting the estimate.
    min_bin_count: usize,  // default: 5
}

pub struct PressureBin {
    pub center: f64,
    pub sum_performance: f64,
    pub count: usize,
}

impl YerkesDodsonEstimator {
    pub fn record(&mut self, pressure: f64, performance: f64) {
        let bin_idx = ((pressure * 10.0) as usize).min(9);
        self.bins[bin_idx].sum_performance += performance;
        self.bins[bin_idx].count += 1;
        self.total += 1;
    }

    /// Estimate the optimal pressure level.
    /// Returns the center of the bin with the highest average performance.
    pub fn estimated_optimum(&self) -> f64 {
        if self.total < 20 {
            return 0.5;  // insufficient data -> moderate default
        }
        self.bins.iter()
            .filter(|b| b.count >= self.min_bin_count)
            .max_by(|a, b| {
                let avg_a = a.sum_performance / a.count as f64;
                let avg_b = b.sum_performance / b.count as f64;
                avg_a.partial_cmp(&avg_b).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|b| b.center)
            .unwrap_or(0.5)
    }
}
```

| Model Tier | Peak Location | Collapse Threshold | Curve Shape |
|---|---|---|---|
| Opus (Premium) | Higher (~0.55) | Later (~0.80) | Wide peak, gradual collapse |
| Sonnet (Standard) | Moderate (~0.45) | Moderate (~0.70) | Standard width |
| Haiku (Fast) | Lower (~0.35) | Earlier (~0.55) | Narrow peak, steep collapse |

The estimator defaults to moderate pressure (0.5) until it has at least 20 observations. Below that threshold, binned averages are too noisy to trust.

---

## 9. The Complete Supervision Graph

All the Cells compose into a supervision Graph that runs concurrently with the agent's cognitive pipeline:

```
Supervision Graph (theta frequency)
=====================================

  Agent Turn   Watcher        Pressure
  Metrics      Outputs        State
     |            |              |
     v            v              v
  +-------+  +-----------+  +-----------+
  | Flow   |  | SelfModel |  | Pressure  |
  | Detect |  | Lens      |  | Score     |
  | (Lens) |  | (Lens)    |  | (Score)   |
  +-------+  +-----------+  +-----------+
     |            |              |
     +------+-----+-----+-------+
            |           |
            v           v
     +-----------+ +------------------+
     | Threshold | | PressureModulate |
     | Learner   | | Route            |
     | (React)   | | (Route)          |
     +-----------+ +------------------+
            |           |
            v           v
     +---------------------------+
     | Adjusted Watcher          |
     | Thresholds                |
     +---------------------------+
            |
            v
     +---------------------------+
     | ConductorBandit Route     |
     | (intervention selection)  |
     +---------------------------+
            |
            v
     Intervention Signal
```

This Graph is a Hot Graph: it stays resident and re-fires every theta tick. The Engine executes it with the same semantics as task plans -- snapshot, resume, budget accounting, telemetry.

---

## What This Enables

1. **Self-improving supervision.** Thresholds adapt to the current workload via predict-publish-correct. A system running Opus agents on complex tasks learns different thresholds than one running Haiku agents on simple tasks, without operator configuration.

2. **Pressure-aware intervention.** The Yerkes-Dodson Score Cell prevents the supervision system from pushing agents past the collapse point. When pressure is high, thresholds relax. When pressure is low, they tighten. The system seeks the optimal zone automatically.

3. **Flow preservation.** The FlowDetector Lens prevents productive agents from being interrupted by false positive detections. This preserves the 3+ turns of context-rebuilding cost that a restart would impose.

4. **Converging learning.** Triple-loop learning detects oscillation in double-loop threshold adaptation and adjusts the learning rate. This prevents the common failure mode where adaptive systems oscillate between too-strict and too-lenient configurations.

5. **Model-aware calibration.** Per-model pressure profiles learn the Yerkes-Dodson curve for each model tier. The system automatically adjusts pressure when the operator switches between Opus and Haiku, or when a new model version ships with different collapse characteristics.

6. **Transparent decision-making.** Every supervision decision is a Signal with provenance. The intervention Signal carries the confidence, the zone, the feature vector, and the source (bandit vs. static fallback). Operators can query the Store to understand why any particular intervention fired.

---

## Feedback Loops

- **SelfModelLens -> L2 ThresholdLearner**: Accuracy metrics drive threshold adaptation. When intervention effectiveness drops below 50%, thresholds are loosened. When it exceeds 85%, thresholds tighten.
- **PressureScore -> L1 Watchers**: Pressure zone modulates watcher sensitivity in real time. Zone 3 relaxes thresholds by 1.5x. Zone 1 tightens by 0.75x.
- **FlowDetector -> L1 Watchers**: Flow detection raises thresholds by 1.5x for the duration of the flow state. Prevents productive agents from being interrupted.
- **ConductorBandit -> L2 ThresholdLearner**: Bandit reward signals feed back to threshold posteriors. Actions that produce good outcomes increase alpha; actions that fail increase beta.
- **TripleLoopController -> L2 ThresholdLearner**: When double-loop oscillates, the triple loop reduces the learning rate. When it converges too slowly, it increases the learning rate.
- **ModelPressureProfile -> PressureScore**: Learned per-model optimal pressure shifts the zone boundaries. An Opus agent's Zone 2 runs from 0.35-0.65; a Haiku agent's from 0.20-0.50.
- **Gate verdicts -> SelfModelLens**: Every gate verdict updates the Brier score tracker, providing continuous calibration feedback.
- **Diagnosis outcomes -> SelfModelLens**: Every auto-fix success/failure updates diagnosis accuracy, tracking whether error classification is correct.

---

## Open Questions

1. **Discount factor selection.** The default discount of 0.995 gives a half-life of ~138 observations. Should this be adaptive? A system with rapidly changing workloads needs a shorter half-life (lower discount). A stable system needs a longer half-life (higher discount). The triple-loop could adapt the discount factor alongside the learning rate.

2. **Pressure weight learning.** The PressureScore weights (0.30 iteration, 0.25 cost, 0.25 time, 0.20 stuck) are static. Should they be learned from the Yerkes-Dodson estimator? The risk: if the weights drift to emphasize one dimension, the pressure index loses its multi-dimensional coverage.

3. **Cross-agent flow detection.** The current FlowDetector operates per-agent. When multiple agents in a plan are all in flow simultaneously, should the plan-level conductor (L3 in the federation architecture) further relax thresholds? The risk is that synchronized flow masks synchronized collapse.

4. **Bandit warmup cold start.** The 50-observation warmup means the first 50 interventions use the static policy. For short-lived deployments (less than 50 tasks), the bandit never activates. Should the warmup be reduced for small workloads, or should prior knowledge from previous sessions bootstrap the bandit?

5. **Flow and collapse are not binary.** The current FlowDetector uses a binary classification (productive/not-productive per turn). Real agent behavior exists on a spectrum. Should the Lens produce a continuous flow score instead? The tradeoff: a continuous score requires a threshold to convert to a multiplier, introducing another parameter to adapt.

6. **Causal direction of pressure-performance relationship.** Does moderate pressure cause good performance, or do easy tasks produce both moderate pressure and good performance? The binned estimator cannot distinguish correlation from causation. Controlled experiments (deliberately varying pressure on matched tasks) would be needed to establish causality.
