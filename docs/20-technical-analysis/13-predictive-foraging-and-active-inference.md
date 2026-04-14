# Predictive Foraging and Active Inference

> Every knowledge retrieval is a falsifiable prediction. The CalibrationTracker corrects biases at ~50ns per correction. Active inference (factorized discrete POMDP with 90 states) drives context selection via Expected Free Energy. This is the complete prediction-resolution-calibration loop.


> **Implementation**: Specified

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [01-oracle-trait](./01-oracle-trait.md) for Oracle trait, [00-vision-ta-generalized](./00-vision-ta-generalized.md) for the universal prediction vision
**Key sources**: `refactoring-prd/09-innovations.md` §VII, §XIX.A-C, `bardo-backup/tmp/agent-chain/10-predictive-foraging.md`, `tmp/implementation-plans/modelrouting/12-advanced-patterns.md`

---

## Predictive foraging — The core loop

Predictive foraging transforms every agent action into a learning opportunity. Before acting, the agent makes a falsifiable prediction about the outcome. After acting, the prediction is compared to reality. The difference (residual) feeds an arithmetic corrector that improves future predictions. This loop costs ~50 nanoseconds per correction — pure arithmetic, no LLM.

```
1. PREDICT    → Oracle.predict(query, ctx) → Prediction
2. ACT        → Agent.execute(action) → output
3. VERIFY     → Gate.verify(output) → Engram (ground truth)
4. RESOLVE    → Oracle.evaluate(prediction, outcome) → PredictionAccuracy
5. CORRECT    → ResidualCorrector.update(model, category, residual) → adjusted bias
6. CALIBRATE  → CalibrationTracker.update(model, category, accuracy) → updated stats
7. FEEDBACK   → Router.feedback(model, accuracy) → updated bandit arms
8. LEARN      → Neuro.store(pattern) → knowledge entry
```

Steps 1-4 are the prediction lifecycle (see [01-oracle-trait.md](./01-oracle-trait.md)). Steps 5-8 are the learning loop that makes predictions improve over time.

### PredictionClaim — The falsifiable commitment

```rust
/// A PredictionClaim is an Engram that commits the agent to a
/// specific prediction about a specific outcome.
///
/// The claim is stored BEFORE action execution. This makes it
/// impossible for the agent to "retrodict" — to claim after the
/// fact that it predicted the right outcome.
///
/// The claim structure:
///   "I predict that [metric] will be [value] with [confidence]
///    in [horizon], and this prediction is based on [lineage]."
pub struct PredictionClaim {
    /// The Engram that stores this claim.
    pub engram: Engram,

    /// The prediction (from Oracle.predict()).
    pub prediction: Prediction,

    /// When the claim was registered (before action execution).
    pub registered_at_ms: i64,

    /// Status: Pending | Resolved(accuracy) | Expired.
    pub status: ClaimStatus,
}

pub enum ClaimStatus {
    Pending,
    Resolved(PredictionAccuracy),
    Expired,
}
```

### ResidualCorrector — ~50ns per correction

The ResidualCorrector is the workhorse of predictive foraging. It maintains per-(model, task_category) bias estimates and corrects raw predictions:

```rust
/// ResidualCorrector: fast bias elimination.
///
/// For each (model, category) pair, tracks the exponential moving
/// average of prediction residuals (predicted - actual).
///
/// Correction: adjusted = raw - mean_bias(model, category)
///
/// Cost: ~50 nanoseconds per correction.
///   - HashMap lookup: ~20ns
///   - EMA update: ~10ns
///   - Subtraction: ~1ns
///   - Cache overhead: ~19ns
///
/// At 1,000 predictions/day/agent: 50µs total daily cost for corrections.
pub struct ResidualCorrector {
    biases: DashMap<(String, String), ExponentialMovingAverage>,
    alpha: f64,  // EMA smoothing factor (typically 0.1)
}

impl ResidualCorrector {
    pub fn new(alpha: f64) -> Self {
        Self {
            biases: DashMap::new(),
            alpha,
        }
    }

    /// Correct a raw prediction by subtracting estimated bias.
    pub fn correct(&self, model: &str, category: &str, raw_value: f64) -> f64 {
        let key = (model.to_string(), category.to_string());
        match self.biases.get(&key) {
            Some(ema) => raw_value - ema.current(),
            None => raw_value,  // no correction data yet
        }
    }

    /// Update bias estimate with a new residual observation.
    pub fn update(&self, model: &str, category: &str, residual: f64) {
        let key = (model.to_string(), category.to_string());
        self.biases
            .entry(key)
            .or_insert_with(|| ExponentialMovingAverage::new(self.alpha))
            .value_mut()
            .update(residual);
    }

    /// Get the current bias estimate for a (model, category) pair.
    pub fn bias(&self, model: &str, category: &str) -> f64 {
        let key = (model.to_string(), category.to_string());
        self.biases.get(&key)
            .map(|ema| ema.current())
            .unwrap_or(0.0)
    }
}
```

### CalibrationTracker — Per-(model, category) accuracy

```rust
/// CalibrationTracker aggregates prediction accuracy statistics.
///
/// Tracks per-(model, task_category):
/// - Mean residual (bias)
/// - Mean absolute error (accuracy)
/// - Interval calibration (fraction of outcomes within prediction intervals)
/// - Accuracy trend (improving or degrading)
///
/// On-chain (Korai): shared across all agents in the collective.
/// A new agent importing the collective calibration starts with
/// pre-learned biases — this is the mechanism behind the 31.6x
/// faster calibration heuristic.
pub struct CalibrationTracker {
    stats: DashMap<(String, String), CalibrationStats>,
}

pub struct CalibrationStats {
    pub mean_residual: ExponentialMovingAverage,
    pub mean_absolute_error: ExponentialMovingAverage,
    pub interval_coverage: ExponentialMovingAverage,
    pub count: u64,
    pub trend: TrendEstimator,
}

impl CalibrationTracker {
    /// Update calibration with a resolved prediction.
    pub fn update(&self, accuracy: &PredictionAccuracy) {
        let key = (
            accuracy.domain.to_string(),
            accuracy.category.clone(),
        );

        self.stats.entry(key)
            .or_insert_with(|| CalibrationStats::new(alpha: 0.1))
            .value_mut()
            .update(accuracy);
    }

    /// Get calibrated confidence for a (model, category) pair.
    /// This is used to adjust raw Oracle confidence values.
    pub fn calibrated_confidence(&self, model: &str, category: &str) -> f64 {
        let key = (model.to_string(), category.to_string());
        self.stats.get(&key)
            .map(|s| 1.0 - s.mean_absolute_error.current())
            .unwrap_or(0.5)  // prior: 50% confidence when no data
    }

    /// Get the accuracy trend (positive = improving).
    pub fn accuracy_trend(&self, model: &str, category: &str) -> f64 {
        let key = (model.to_string(), category.to_string());
        self.stats.get(&key)
            .map(|s| s.trend.slope())
            .unwrap_or(0.0)
    }

    /// Export calibration data for on-chain publishing (Korai).
    pub fn export(&self) -> CollectiveCalibration {
        let entries: Vec<_> = self.stats.iter()
            .map(|entry| {
                let ((model, category), stats) = entry.pair();
                CalibrationEntry {
                    model: model.clone(),
                    category: category.clone(),
                    mean_bias: stats.mean_residual.current(),
                    mean_absolute_error: stats.mean_absolute_error.current(),
                    interval_coverage: stats.interval_coverage.current(),
                    count: stats.count,
                }
            })
            .collect();

        CollectiveCalibration { entries }
    }

    /// Import collective calibration from Korai.
    pub fn import(&self, collective: &CollectiveCalibration) {
        for entry in &collective.entries {
            let key = (entry.model.clone(), entry.category.clone());
            self.stats.entry(key)
                .or_insert_with(|| CalibrationStats::from_collective(entry));
        }
    }
}
```

---

## Active inference — Expected Free Energy

Active inference (Friston, 2010, *Nature Reviews Neuroscience*) provides the theoretical foundation for how agents select actions (including what context to retrieve). The agent maintains an internal generative model of the world and selects actions that minimize Expected Free Energy (EFE).

### Factorized discrete POMDP — The state space

The agent's state space is a factorized discrete Partially Observable Markov Decision Process with 6 × 5 × 3 = 90 states:

```rust
/// Factorized discrete POMDP for active inference.
///
/// The state space factors into three independent dimensions:
///   - Task complexity (6 levels): trivial, simple, moderate, complex, expert, research
///   - Information state (5 levels): blind, partial, adequate, comprehensive, complete
///   - Confidence state (3 levels): low, medium, high
///
/// Total: 6 × 5 × 3 = 90 discrete states.
///
/// This factorization reduces the state space exponentially compared
/// to a flat representation (which would need to enumerate all
/// possible combinations of continuous variables).
pub struct ActiveInferenceState {
    /// Current beliefs about the state (probability distribution over 90 states).
    pub beliefs: Array3<f64>,  // shape: [6, 5, 3]

    /// Generative model matrices (see below).
    pub model: GenerativeModel,
}

/// The four matrices of the generative model.
pub struct GenerativeModel {
    /// A matrix: observation likelihood P(o | s).
    /// "Given state s, what observations would I expect?"
    /// Shape: [n_observations, 6, 5, 3]
    pub a: Array4<f64>,

    /// B matrix: transition dynamics P(s' | s, a).
    /// "Given state s and action a, what state will I be in next?"
    /// One matrix per action.
    /// Shape: [n_actions][6, 5, 3, 6, 5, 3]
    pub b: Vec<Array6<f64>>,

    /// C matrix: preferred observations (goal).
    /// "What observations do I want to see?"
    /// Encodes: high task success, high confidence, complete information.
    /// Shape: [n_observations]
    pub c: Array1<f64>,

    /// D matrix: initial state prior.
    /// "What state do I believe I start in?"
    /// Shape: [6, 5, 3]
    pub d: Array3<f64>,
}
```

### EFE decomposition for context selection

Expected Free Energy decomposes into three terms that drive context selection in the VCG attention auction:

```rust
/// Expected Free Energy (EFE) for action evaluation.
///
/// G(π) = pragmatic_value + epistemic_value - ambiguity
///
/// where:
///   pragmatic_value = E[ln P(o_desired | s_π)] — expected goal achievement
///   epistemic_value = E[H(s | o_π) - H(s | o_π, θ)] — expected information gain
///   ambiguity = H(o | s_π) — expected observation noise
///
/// Lower EFE → better action.
/// The agent selects actions (including context retrieval actions)
/// that minimize EFE.
pub fn expected_free_energy(
    beliefs: &Array3<f64>,
    action: usize,
    model: &GenerativeModel,
) -> EfeDecomposition {
    // Predicted state after action
    let predicted_state = apply_transition(beliefs, action, &model.b[action]);

    // Pragmatic value: how much does this action achieve the goal?
    let pragmatic = compute_pragmatic_value(&predicted_state, &model.c, &model.a);

    // Epistemic value: how much information does this action provide?
    let epistemic = compute_epistemic_value(&predicted_state, beliefs, &model.a);

    // Ambiguity: how noisy are the expected observations?
    let ambiguity = compute_ambiguity(&predicted_state, &model.a);

    EfeDecomposition {
        total: pragmatic + epistemic - ambiguity,
        pragmatic,
        epistemic,
        ambiguity,
    }
}

pub struct EfeDecomposition {
    /// Total EFE (lower = better).
    pub total: f64,
    /// Goal achievement (higher = more goal-directed).
    pub pragmatic: f64,
    /// Information gain (higher = more exploratory).
    pub epistemic: f64,
    /// Observation noise (lower = less ambiguous).
    pub ambiguity: f64,
}
```

### Context foraging stopping rule — Charnov's MVT

The agent forages for context (retrieves Engrams to fill the context window) and must decide when to stop. Charnov's marginal value theorem (Charnov, 1976, *Theoretical Population Biology*) provides the optimal stopping rule:

```rust
/// Context foraging stopping rule based on Charnov's MVT.
///
/// Stop retrieving context when the marginal information gain
/// of the next retrieval drops below the average gain rate
/// across all context patches (domains/topics).
///
/// gain_rate = total_information_gained / total_tokens_spent
///
/// Retrieve next item if:
///   marginal_gain(next_item) > gain_rate × marginal_cost(next_item)
///
/// This naturally balances breadth (exploring many topics) vs.
/// depth (going deep on one topic) based on the current
/// information landscape.
pub struct ContextForager {
    /// Current information gain rate (running average).
    gain_rate: ExponentialMovingAverage,

    /// Per-domain context patches.
    patches: HashMap<String, ContextPatch>,

    /// Token budget remaining.
    budget_remaining: usize,
}

pub struct ContextPatch {
    /// Domain/topic identifier.
    pub id: String,
    /// Items available in this patch.
    pub items: Vec<Engram>,
    /// Estimated information gain per item (decreasing as more items are retrieved).
    pub marginal_gain: f64,
    /// Cost per item (tokens).
    pub item_cost: usize,
    /// Items already retrieved from this patch.
    pub retrieved: usize,
}

impl ContextForager {
    /// Decide whether to continue foraging or stop.
    pub fn should_continue(&self) -> bool {
        // Find the best next item across all patches
        let best_patch = self.patches.values()
            .max_by(|a, b| {
                let ratio_a = a.marginal_gain / a.item_cost as f64;
                let ratio_b = b.marginal_gain / b.item_cost as f64;
                ratio_a.partial_cmp(&ratio_b).unwrap()
            });

        match best_patch {
            Some(patch) => {
                let marginal_ratio = patch.marginal_gain / patch.item_cost as f64;
                marginal_ratio > self.gain_rate.current()
                    && self.budget_remaining > patch.item_cost
            }
            None => false,
        }
    }

    /// Select the next item to retrieve.
    pub fn select_next(&mut self) -> Option<(String, Engram)> {
        if !self.should_continue() {
            return None;
        }

        // Select from the patch with highest marginal gain/cost ratio
        let best_id = self.patches.values()
            .max_by(|a, b| {
                let ratio_a = a.marginal_gain / a.item_cost as f64;
                let ratio_b = b.marginal_gain / b.item_cost as f64;
                ratio_a.partial_cmp(&ratio_b).unwrap()
            })
            .map(|p| p.id.clone())?;

        let patch = self.patches.get_mut(&best_id)?;
        let item = patch.items.get(patch.retrieved)?.clone();

        // Update state
        patch.retrieved += 1;
        patch.marginal_gain *= 0.8;  // diminishing returns
        self.budget_remaining -= patch.item_cost;
        self.gain_rate.update(patch.marginal_gain);

        Some((best_id, item))
    }
}
```

### EFE as VCG bid

The EFE decomposition feeds directly into the VCG attention auction:

```rust
/// Convert EFE score into a VCG auction bid.
///
/// Higher epistemic value → higher bid (the agent WANTS to know this)
/// Higher pragmatic value → higher bid (this helps achieve the goal)
/// Higher ambiguity → lower bid (noisy information is less valuable)
///
/// Modulated by Daimon PAD state:
/// - High arousal → urgency multiplier on pragmatic bids
/// - Low dominance → boost epistemic bids (need more information)
/// - Low pleasure → boost iteration memory bids (learn from failures)
pub fn efe_to_bid(
    efe: &EfeDecomposition,
    pad: &PadState,
    section_type: &str,
) -> f64 {
    let urgency = pad.arousal.max(0.1);
    let exploration = 1.0 - pad.dominance;
    let failure_boost = (1.0 - pad.pleasure).max(0.0);

    let base_bid = match section_type {
        "prediction_context" => efe.epistemic * (1.0 + exploration),
        "task_context" => efe.pragmatic * urgency,
        "failure_memory" => efe.pragmatic * failure_boost,
        "knowledge" => efe.epistemic * exploration,
        _ => efe.total,
    };

    base_bid.max(0.0)
}
```

---

## Thompson Sampling for oracle selection

When multiple oracle implementations are available for the same query, Thompson Sampling (Thompson, 1933) selects which oracle to use:

```rust
/// Thompson Sampling for oracle selection.
///
/// Each oracle maintains a Beta distribution modeling its accuracy:
///   Beta(α_success, β_failure)
///
/// To select an oracle:
///   1. Sample from each oracle's Beta distribution
///   2. Select the oracle with the highest sample
///
/// This naturally balances exploration (trying less-used oracles)
/// with exploitation (preferring proven oracles).
pub struct ThompsonOracleSelector {
    arms: HashMap<String, ThompsonArm>,
}

pub struct ThompsonArm {
    /// Oracle identifier.
    pub oracle_id: String,

    /// Success count (predictions with accuracy > threshold).
    pub alpha: f64,

    /// Failure count (predictions with accuracy <= threshold).
    pub beta: f64,
}

impl ThompsonArm {
    pub fn new(oracle_id: String) -> Self {
        Self {
            oracle_id,
            alpha: 1.0,  // prior: Beta(1,1) = uniform
            beta: 1.0,
        }
    }

    /// Sample from the Beta distribution.
    pub fn sample(&self, rng: &mut impl Rng) -> f64 {
        Beta::new(self.alpha, self.beta)
            .unwrap()
            .sample(rng)
    }

    /// Update after observing a prediction outcome.
    pub fn update(&mut self, accuracy: f64, threshold: f64) {
        if accuracy > threshold {
            self.alpha += 1.0;
        } else {
            self.beta += 1.0;
        }
    }
}

impl ThompsonOracleSelector {
    /// Select the best oracle for a given query.
    pub fn select(&self, rng: &mut impl Rng) -> &str {
        self.arms.values()
            .max_by(|a, b| {
                a.sample(rng).partial_cmp(&b.sample(rng)).unwrap()
            })
            .map(|arm| arm.oracle_id.as_str())
            .unwrap()
    }
}
```

For non-stationary environments (where oracle quality changes over time), the f-dsw (fixed-share with discounting) variant of Thompson Sampling (Raj & Kalyani, 2017) is used. This adds a discount factor that gradually forgets old observations, allowing the selector to track changing oracle quality.

---

## Collective calibration on Korai

The full predictive foraging loop extends to the collective via Korai:

```
Individual agent:
  Predict → Act → Verify → Correct → Calibrate

Collective:
  Agent A publishes calibration → Korai ISFR
  Agent B imports calibration → starts with pre-learned biases
  Agent B's corrections refine the collective calibration
  → Published back to Korai
  → Next agent starts even better
```

The collective calibration heuristic (1/sqrt(N×t), see `refactoring-prd/09-innovations.md` §VI) projects that with N=1,000 agents, a new agent reaches ~82% accuracy in 3 days instead of 3 months. This is a theoretical upper bound under the independence assumption — actual speedup depends on agent correlation and domain shift.

---

## Three cognitive speeds in the prediction loop

| Speed | Prediction activity |
|---|---|
| **Gamma** (~5-15s) | T0 probes evaluate prediction error scalar. No prediction resolution. Cost: µs. |
| **Theta** (~75s) | Pending predictions resolved. Residuals computed. CalibrationTracker updated. EMA thresholds adjusted. |
| **Delta** (hours) | Cross-model calibration analysis. Thompson Sampling arms updated. Collective calibration published to Korai. Predictive strategy fragments consolidated in Dreams. |

---

## Academic foundations

- Friston, K. (2010). "The free-energy principle: a unified brain theory?" *Nature Reviews Neuroscience*, 11(2), 127-138. — Active inference framework.
- Charnov, E. L. (1976). "Optimal foraging: the marginal value theorem." *Theoretical Population Biology*, 9, 129-136. — Context foraging stopping rule.
- Pirolli, P., & Card, S. (1999). "Information foraging." *Psychological Review*, 106(4), 643-675. — Information foraging theory.
- Thompson, W. R. (1933). "On the Likelihood that One Unknown Probability Exceeds Another." *Biometrika*, 25(3-4), 285-294. — Thompson Sampling.
- Raj, V., & Kalyani, S. (2017). "Taming Non-stationary Bandits: A Bayesian Approach." arXiv:1707.09727. — f-dsw Thompson Sampling for non-stationary environments.
- Conant, R. C., & Ashby, W. R. (1970). "Every good regulator of a system must be a model of that system." *International Journal of Systems Science*, 1(2), 89-97. — Good Regulator Theorem.
- Chen, L., et al. (2023). "FrugalGPT." arXiv:2305.05176. — Cascade routing for cost-effective prediction.
- Lee, S., et al. (2026). "Meta-Harness." arXiv:2603.28052. — Harness optimization.

---

## Cross-References

- See [01-oracle-trait.md](./01-oracle-trait.md) for the Oracle trait that predictions use
- See [00-vision-ta-generalized.md](./00-vision-ta-generalized.md) for the vision of universal oracle primitives
- See [02-chain-oracles.md](./02-chain-oracles.md) and [03-coding-oracles.md](./03-coding-oracles.md) for domain-specific prediction examples
- See topic [05-learning](../05-learning/INDEX.md) for CascadeRouter and bandit integration
- See topic [06-neuro](../06-neuro/INDEX.md) for knowledge tier progression from prediction outcomes
