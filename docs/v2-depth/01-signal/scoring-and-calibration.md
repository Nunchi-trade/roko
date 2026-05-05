# Scoring and Calibration

> Depth for [01-SIGNAL.md](../../unified/01-SIGNAL.md) &sect;5. Solves the central problem of multi-axis scoring: how do you calibrate a quality measure when ground truth only comes from binary Verify verdicts? Derives the Score-Verify-Score feedback loop and its convergence properties.

---

## 1. The Calibration Problem

Every Signal carries a 5-axis Score:

```rust
pub struct Score {
    pub relevance:  f64,  // 0.0..=1.0
    pub quality:    f64,  // 0.0..=1.0
    pub confidence: f64,  // 0.0..=1.0
    pub novelty:    f64,  // 0.0..=1.0
    pub utility:    f64,  // 0.0..=1.0
}
```

See [01-SIGNAL.md](../../unified/01-SIGNAL.md) &sect;5 for the axis definitions. The problem is not *defining* axes -- it is *calibrating* them. A confidence of 0.8 from a compile Verify Cell (deterministic, binary) is not the same as 0.8 from an LLM judge Verify Cell (probabilistic, soft). If these scores are treated as commensurable, the system will over-trust the LLM judge and under-trust the compiler.

The only reliable ground truth in the system comes from **Verify protocol Cells** (gates). A Verify verdict is binary: pass or fail. The question is: how do you use binary verdicts to calibrate a 5-dimensional continuous score?

---

## 2. The Score-Verify-Score Loop

The calibration loop has three phases:

```
    Score Cells produce scores
         |
         v
    Compose uses scores to build context
         |
         v
    Agent acts on composed context
         |
         v
    Verify Cells produce binary verdicts
         |
         v
    Calibration Cells update Score Cell parameters
         |
         v
    Score Cells produce BETTER scores next time
         |
         v
    (repeat)
```

This is a **predict-observe-update** loop, which is the canonical structure of Bayesian inference. The Score is the prediction. The Verify verdict is the observation. The calibration update is the posterior.

```rust
/// The calibration loop as a Cell (implements the Score protocol).
///
/// This Cell wraps an inner Score Cell and adjusts its outputs
/// based on accumulated Verify verdicts.
pub struct CalibratedScorer {
    /// The inner scorer whose outputs are being calibrated.
    inner: Box<dyn ScoreCell>,

    /// Per-axis Beta-Binomial trackers.
    /// Each tracker maintains a posterior over the true quality
    /// of Signals that the inner scorer rates at a given level.
    calibrators: [AxisCalibrator; 5],

    /// Temperature parameter for post-hoc calibration.
    /// T > 1 reduces overconfidence; T < 1 sharpens.
    temperature: f64,

    /// Name for telemetry and debugging.
    name: String,
}

impl ScoreCell for CalibratedScorer {
    fn score(&self, signal: &Signal, ctx: &ScoreContext) -> Score {
        // 1. Get raw score from inner scorer
        let raw = self.inner.score(signal, ctx);

        // 2. Temperature-scale each axis
        let calibrated = Score {
            relevance:  self.calibrate_axis(0, raw.relevance),
            quality:    self.calibrate_axis(1, raw.quality),
            confidence: self.calibrate_axis(2, raw.confidence),
            novelty:    self.calibrate_axis(3, raw.novelty),
            utility:    self.calibrate_axis(4, raw.utility),
        };

        calibrated
    }
}
```

---

## 3. Temperature Scaling as a Score Protocol Cell

Temperature scaling (Guo et al. 2017) is the simplest calibration method: divide each score by a temperature parameter T > 0.

```rust
/// Temperature scaling for a single score axis.
///
/// Given a raw score s in [0, 1], the calibrated score is:
///   calibrated = sigmoid(logit(s) / T)
///
/// Where logit(s) = ln(s / (1 - s)) and sigmoid(x) = 1 / (1 + exp(-x)).
///
/// T > 1: reduces confidence (overconfident scorer)
/// T < 1: increases confidence (underconfident scorer)
/// T = 1: no change (perfectly calibrated)
pub fn temperature_scale(raw: f64, temperature: f64) -> f64 {
    if raw <= 0.0 || raw >= 1.0 {
        return raw.clamp(0.0, 1.0);
    }
    let logit = (raw / (1.0 - raw)).ln();
    let scaled_logit = logit / temperature;
    1.0 / (1.0 + (-scaled_logit).exp())
}

impl CalibratedScorer {
    fn calibrate_axis(&self, axis: usize, raw: f64) -> f64 {
        let t = self.calibrators[axis].temperature();
        temperature_scale(raw, t)
    }
}
```

### 3.1 Learning the Temperature

The temperature T is learned from the Score-Verify loop by minimizing Expected Calibration Error (ECE):

```rust
/// Expected Calibration Error: measures how well scores predict
/// actual pass rates across bins.
///
/// ECE = sum_b (|B_b| / N) * |accuracy(B_b) - confidence(B_b)|
///
/// Where B_b is the set of predictions in bin b,
/// accuracy is the actual pass rate, and confidence is the
/// average predicted score.
pub fn compute_ece(
    predictions: &[(f64, bool)],  // (score, verdict_passed)
    n_bins: usize,
) -> f64 {
    let n = predictions.len() as f64;
    let bin_width = 1.0 / n_bins as f64;

    let mut ece = 0.0;
    for bin in 0..n_bins {
        let lo = bin as f64 * bin_width;
        let hi = lo + bin_width;

        let in_bin: Vec<&(f64, bool)> = predictions.iter()
            .filter(|(score, _)| *score >= lo && *score < hi)
            .collect();

        if in_bin.is_empty() {
            continue;
        }

        let bin_size = in_bin.len() as f64;
        let avg_confidence: f64 = in_bin.iter().map(|(s, _)| s).sum::<f64>() / bin_size;
        let accuracy: f64 = in_bin.iter().filter(|(_, p)| *p).count() as f64 / bin_size;

        ece += (bin_size / n) * (accuracy - avg_confidence).abs();
    }
    ece
}

/// Find the temperature that minimizes ECE via golden section search.
pub fn optimize_temperature(
    predictions: &[(f64, bool)],
    n_bins: usize,
) -> f64 {
    let mut lo = 0.1_f64;
    let mut hi = 10.0_f64;
    let golden = (5.0_f64.sqrt() - 1.0) / 2.0;

    for _ in 0..50 {
        let x1 = hi - golden * (hi - lo);
        let x2 = lo + golden * (hi - lo);

        let scaled_1: Vec<(f64, bool)> = predictions.iter()
            .map(|(s, p)| (temperature_scale(*s, x1), *p))
            .collect();
        let scaled_2: Vec<(f64, bool)> = predictions.iter()
            .map(|(s, p)| (temperature_scale(*s, x2), *p))
            .collect();

        if compute_ece(&scaled_1, n_bins) < compute_ece(&scaled_2, n_bins) {
            hi = x2;
        } else {
            lo = x1;
        }
    }
    (lo + hi) / 2.0
}
```

---

## 4. Beta-Binomial Updating for Confidence

The confidence axis has the cleanest calibration path because Verify verdicts provide direct binary evidence. The Beta-Binomial conjugate model gives exact Bayesian updates.

```rust
/// Per-axis calibrator using Beta-Binomial conjugate updates.
///
/// The Beta distribution is the conjugate prior for the Binomial
/// likelihood. Given binary verdicts, the posterior is:
///   Beta(alpha + passes, beta + fails)
///
/// The posterior mean is the calibrated confidence:
///   E[theta] = alpha / (alpha + beta)
///
/// The posterior variance measures remaining uncertainty:
///   Var[theta] = (alpha * beta) / ((alpha + beta)^2 * (alpha + beta + 1))
pub struct AxisCalibrator {
    /// Pseudo-count for positive evidence (prior + observed passes).
    alpha: f64,
    /// Pseudo-count for negative evidence (prior + observed fails).
    beta: f64,
    /// Window size for recent-only calibration.
    window: usize,
    /// Rolling buffer of recent (score, verdict) pairs.
    recent: VecDeque<(f64, bool)>,
    /// Learned temperature (updated periodically from recent buffer).
    learned_temperature: f64,
}

impl AxisCalibrator {
    /// Default: weakly informative prior Beta(2, 2).
    /// Centered at 0.5, equivalent to 4 pseudo-observations.
    pub fn new(window: usize) -> Self {
        Self {
            alpha: 2.0,
            beta: 2.0,
            window,
            recent: VecDeque::with_capacity(window),
            learned_temperature: 1.0,
        }
    }

    /// Update after a Verify verdict.
    pub fn update(&mut self, raw_score: f64, passed: bool) {
        // Update Beta posterior
        if passed {
            self.alpha += 1.0;
        } else {
            self.beta += 1.0;
        }

        // Update rolling buffer
        self.recent.push_back((raw_score, passed));
        if self.recent.len() > self.window {
            self.recent.pop_front();
        }

        // Re-learn temperature periodically
        if self.recent.len() >= 20 && self.recent.len() % 10 == 0 {
            let pairs: Vec<(f64, bool)> = self.recent.iter().cloned().collect();
            self.learned_temperature = optimize_temperature(&pairs, 10);
        }
    }

    /// Posterior mean: calibrated probability.
    pub fn posterior_mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Posterior uncertainty (standard deviation).
    pub fn posterior_uncertainty(&self) -> f64 {
        let n = self.alpha + self.beta;
        ((self.alpha * self.beta) / (n * n * (n + 1.0))).sqrt()
    }

    /// Effective temperature for this axis.
    pub fn temperature(&self) -> f64 {
        self.learned_temperature
    }

    /// Credible interval at level 1-alpha.
    /// Uses the Beta quantile function.
    pub fn credible_interval(&self, level: f64) -> (f64, f64) {
        let tail = (1.0 - level) / 2.0;
        let lo = beta_quantile(self.alpha, self.beta, tail);
        let hi = beta_quantile(self.alpha, self.beta, 1.0 - tail);
        (lo, hi)
    }
}
```

---

## 5. Multi-Axis Updating

Each axis receives evidence from different sources. The calibration pipeline routes Verify verdicts to the appropriate axis based on what the verdict actually tests.

```rust
/// Route a Verify verdict to the appropriate axis calibrators.
///
/// A single verdict may update multiple axes. For example,
/// a compile gate passing updates both confidence (the code compiles)
/// and quality (the code is well-formed).
pub fn route_verdict(
    calibrators: &mut [AxisCalibrator; 5],
    verdict: &Verdict,
    original_score: &Score,
) {
    match verdict.evidence_kind {
        // Compile verdicts update confidence and quality
        EvidenceKind::Compile => {
            calibrators[CONFIDENCE].update(
                original_score.confidence,
                verdict.passed,
            );
            calibrators[QUALITY].update(
                original_score.quality,
                verdict.passed,
            );
        }

        // Test verdicts update confidence
        EvidenceKind::Test => {
            calibrators[CONFIDENCE].update(
                original_score.confidence,
                verdict.passed,
            );
        }

        // Relevance verdicts (e.g., from context evaluation)
        EvidenceKind::Relevance => {
            calibrators[RELEVANCE].update(
                original_score.relevance,
                verdict.passed,
            );
        }

        // Novelty verdicts (e.g., deduplication check)
        EvidenceKind::Novelty => {
            calibrators[NOVELTY].update(
                original_score.novelty,
                verdict.passed,  // "passed" means "is genuinely novel"
            );
        }

        // Utility verdicts (e.g., task completion check)
        EvidenceKind::Utility => {
            calibrators[UTILITY].update(
                original_score.utility,
                verdict.passed,
            );
        }

        _ => {
            // Unknown evidence kind: update confidence only
            calibrators[CONFIDENCE].update(
                original_score.confidence,
                verdict.passed,
            );
        }
    }
}

const RELEVANCE: usize = 0;
const QUALITY: usize = 1;
const CONFIDENCE: usize = 2;
const NOVELTY: usize = 3;
const UTILITY: usize = 4;
```

---

## 6. When Score Cells Disagree

Multiple Score Cells may produce different scores for the same Signal. A relevance scorer, a complexity scorer, and a reputation scorer each contribute a partial view. The question is: how do you aggregate them?

### 6.1 Element-Wise Aggregation

The simple approach: element-wise max (optimistic) or mean (consensus).

```rust
/// Aggregate scores from multiple Score Cells.
///
/// Strategy: per-axis, take the weighted mean where weights
/// are the calibrated uncertainty of each scorer.
/// Low-uncertainty scorers dominate; high-uncertainty scorers
/// contribute less. This is precision-weighted averaging.
pub fn aggregate_scores(
    scores: &[(Score, &CalibratedScorer)],
) -> Score {
    let mut axes = [0.0_f64; 5];
    let mut weights = [0.0_f64; 5];

    for (score, scorer) in scores {
        let values = [
            score.relevance, score.quality, score.confidence,
            score.novelty, score.utility,
        ];
        for (i, &val) in values.iter().enumerate() {
            let precision = 1.0 / scorer.calibrators[i]
                .posterior_uncertainty()
                .max(1e-6)
                .powi(2);
            axes[i] += val * precision;
            weights[i] += precision;
        }
    }

    Score {
        relevance:  (axes[0] / weights[0].max(1e-6)).clamp(0.0, 1.0),
        quality:    (axes[1] / weights[1].max(1e-6)).clamp(0.0, 1.0),
        confidence: (axes[2] / weights[2].max(1e-6)).clamp(0.0, 1.0),
        novelty:    (axes[3] / weights[3].max(1e-6)).clamp(0.0, 1.0),
        utility:    (axes[4] / weights[4].max(1e-6)).clamp(0.0, 1.0),
    }
}
```

### 6.2 Pareto Front over Score Axes

When no single aggregation function is appropriate, the system can maintain the **Pareto front** -- the set of Signals where no other Signal dominates on all axes simultaneously.

```rust
/// Compute the Pareto front over a set of scored Signals.
///
/// A Signal s1 dominates s2 if s1 is >= s2 on all axes
/// and strictly > on at least one axis.
///
/// The Pareto front is the set of non-dominated Signals.
pub fn pareto_front(signals: &[Signal]) -> Vec<&Signal> {
    let mut front = Vec::new();

    for candidate in signals {
        let dominated = signals.iter().any(|other| {
            other as *const _ != candidate as *const _
                && dominates(&other.score, &candidate.score)
        });
        if !dominated {
            front.push(candidate);
        }
    }
    front
}

fn dominates(a: &Score, b: &Score) -> bool {
    let axes_a = [a.relevance, a.quality, a.confidence, a.novelty, a.utility];
    let axes_b = [b.relevance, b.quality, b.confidence, b.novelty, b.utility];

    let all_geq = axes_a.iter().zip(axes_b.iter()).all(|(a, b)| a >= b);
    let any_gt = axes_a.iter().zip(axes_b.iter()).any(|(a, b)| a > b);

    all_geq && any_gt
}
```

The Pareto front is useful for Route protocol Cells that want to maintain diversity: instead of selecting the single highest-scoring Signal, select from the Pareto front to preserve variety across score dimensions.

---

## 7. The Effective Score Formula

The 5 axes collapse into a single scalar when a total ordering is needed:

```rust
impl Score {
    /// Collapse 5 axes into a single scalar.
    ///
    /// The formula ensures:
    ///   - confidence = 0 -> effective = 0 (bad data is worthless)
    ///   - novelty acts as a bonus multiplier via (1 + novelty)
    ///   - utility acts as a bonus multiplier via (1 + utility)
    ///   - relevance and quality enter multiplicatively
    ///
    /// Novelty uses attenuation: novelty_eff = 1/(1+ln(1+freq))
    /// to prevent familiar Signals from being fully discounted.
    pub fn effective(&self) -> f64 {
        self.confidence
            * self.relevance.max(0.1)     // floor prevents relevance from killing score
            * self.quality.max(0.1)        // floor prevents quality from killing score
            * (1.0 + self.novelty)         // novelty bonus
            * (1.0 + self.utility)         // utility bonus
    }
}
```

### 7.1 Why Multiplicative

The multiplicative structure has two critical properties:

1. **Zero-confidence kills**: If confidence is 0, effective score is 0 regardless of other axes. This is a structural safety guarantee -- data known to be wrong cannot be prioritized by gaming other axes.

2. **Bonus stacking is superlinear**: A Signal that is novel AND useful gets `(1 + novelty) * (1 + utility)` -- a superlinear bonus. This rewards Signals that are good in multiple dimensions, creating an incentive for genuine quality rather than single-axis gaming.

### 7.2 The Floor on Relevance and Quality

The `max(0.1)` floor on relevance and quality prevents a score of 0 on either axis from zeroing the effective score entirely. This is different from confidence (where 0 *should* kill the score). The reasoning: a Signal with 0 relevance to the current context may be relevant to a future context. A Signal with 0 quality may still be useful as negative evidence. But a Signal with 0 confidence is known to be wrong and should never be prioritized.

---

## 8. Who Calibrates the Calibrators?

The calibration loop has a meta-problem: the calibrators themselves can be miscalibrated. Temperature scaling assumes a specific form of miscalibration (log-odds shift). If the actual miscalibration has a different form, temperature scaling will converge to a suboptimal temperature.

### 8.1 Calibration of Calibration

The meta-calibration approach: track the ECE of the calibrated scores (not the raw scores) and detect when it rises above a threshold.

```rust
/// Meta-calibration: detect when calibrated scores are themselves
/// miscalibrated, indicating that the calibration model (temperature
/// scaling) is insufficient.
pub struct MetaCalibrator {
    /// ECE of calibrated scores over a rolling window.
    calibrated_ece: f64,
    /// Threshold above which to flag miscalibration.
    ece_threshold: f64,  // default 0.05
    /// Rolling window of (calibrated_score, verdict) pairs.
    meta_window: VecDeque<(f64, bool)>,
    /// Window size.
    meta_window_size: usize,  // default 200
}

impl MetaCalibrator {
    pub fn update(&mut self, calibrated_score: f64, verdict: bool) {
        self.meta_window.push_back((calibrated_score, verdict));
        if self.meta_window.len() > self.meta_window_size {
            self.meta_window.pop_front();
        }

        if self.meta_window.len() >= 50 {
            let pairs: Vec<(f64, bool)> = self.meta_window.iter().cloned().collect();
            self.calibrated_ece = compute_ece(&pairs, 10);
        }
    }

    /// Returns true if the calibration model itself needs recalibration.
    pub fn is_miscalibrated(&self) -> bool {
        self.calibrated_ece > self.ece_threshold
    }

    /// When miscalibration is detected, what to do.
    pub fn remediation(&self) -> CalibrationRemediation {
        if self.calibrated_ece > self.ece_threshold * 3.0 {
            // Severe: reset to uninformative prior
            CalibrationRemediation::ResetPrior
        } else if self.calibrated_ece > self.ece_threshold * 2.0 {
            // Moderate: switch from temperature to isotonic regression
            CalibrationRemediation::SwitchToIsotonic
        } else {
            // Mild: increase window size for more data
            CalibrationRemediation::IncreaseWindow
        }
    }
}

pub enum CalibrationRemediation {
    ResetPrior,
    SwitchToIsotonic,
    IncreaseWindow,
}
```

### 8.2 Isotonic Regression Fallback

When temperature scaling is insufficient, isotonic regression provides a non-parametric alternative. It fits a monotone-increasing function from raw scores to calibrated probabilities, making no assumptions about the form of miscalibration.

```rust
/// Isotonic calibration: fit a monotone-increasing step function
/// from raw scores to observed pass rates.
///
/// Advantage: no parametric assumption (unlike temperature scaling).
/// Disadvantage: requires more data and does not extrapolate.
pub struct IsotonicCalibrator {
    /// Sorted breakpoints: (raw_score, calibrated_probability).
    breakpoints: Vec<(f64, f64)>,
}

impl IsotonicCalibrator {
    /// Fit from observed (score, verdict) pairs.
    /// Uses pool-adjacent-violators algorithm (PAVA).
    pub fn fit(pairs: &mut [(f64, bool)]) -> Self {
        pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

        // PAVA: merge adjacent blocks that violate monotonicity
        let mut blocks: Vec<(f64, f64, usize)> = pairs.iter()
            .map(|(s, p)| (*s, if *p { 1.0 } else { 0.0 }, 1))
            .collect();

        let mut i = 0;
        while i < blocks.len() - 1 {
            if blocks[i].1 > blocks[i + 1].1 {
                // Violation: merge blocks
                let (s1, v1, n1) = blocks[i];
                let (s2, v2, n2) = blocks[i + 1];
                let merged_val = (v1 * n1 as f64 + v2 * n2 as f64)
                    / (n1 + n2) as f64;
                blocks[i] = ((s1 + s2) / 2.0, merged_val, n1 + n2);
                blocks.remove(i + 1);
                if i > 0 { i -= 1; }  // check backward
            } else {
                i += 1;
            }
        }

        Self {
            breakpoints: blocks.into_iter()
                .map(|(s, v, _)| (s, v))
                .collect(),
        }
    }

    /// Calibrate a raw score using linear interpolation between breakpoints.
    pub fn calibrate(&self, raw: f64) -> f64 {
        if self.breakpoints.is_empty() {
            return raw;
        }
        // Binary search + interpolation
        let idx = self.breakpoints.partition_point(|(s, _)| *s < raw);
        if idx == 0 { return self.breakpoints[0].1; }
        if idx >= self.breakpoints.len() {
            return self.breakpoints.last().unwrap().1;
        }
        let (s0, v0) = self.breakpoints[idx - 1];
        let (s1, v1) = self.breakpoints[idx];
        let t = (raw - s0) / (s1 - s0).max(1e-10);
        v0 + t * (v1 - v0)
    }
}
```

---

## 9. What This Enables

1. **Truthful scoring**: Calibrated scores mean what they say. A confidence of 0.8 means an 80% chance of passing verification, not "the scorer felt 80% confident." This makes score-based routing and composition reliable.

2. **Cross-scorer comparability**: Temperature scaling normalizes scores across different Score Cells. A relevance score from an HDC similarity search and a relevance score from an LLM judge become comparable after calibration.

3. **Uncertainty-aware aggregation**: Precision-weighted averaging means that well-calibrated scorers dominate the aggregate, while poorly-calibrated scorers contribute less. The system self-corrects.

4. **Meta-calibration detection**: The system knows when its own calibration is failing and can escalate to more sophisticated methods (isotonic regression) or reset to uninformative priors.

5. **Pareto-aware routing**: The Pareto front preserves diversity when a total ordering is too lossy. Route Cells can select from the Pareto front to balance exploration vs. exploitation across score dimensions.

---

## 10. Feedback Loops

1. **Score -> Compose -> Act -> Verify -> Calibrate -> Score**: The core loop. Scores inform composition, composition feeds agents, agents produce outputs, Verify Cells judge outputs, judgments calibrate future scores. Convergence depends on the Verify Cells being informative (not random).

2. **Temperature -> ECE -> Temperature**: Temperature scaling optimizes against ECE, but ECE itself depends on the score distribution, which changes as calibration improves. This is a fixed-point iteration that converges when the temperature stabilizes.

3. **Scorer count -> Precision weighting -> Effective scorer count**: As more Score Cells are added, precision weighting determines which ones matter. Poorly-calibrated cells get down-weighted, effectively reducing the number of contributing scorers. This prevents the "too many cooks" problem.

4. **Verify verdict rate -> Calibration data -> Calibration quality -> Score quality -> Verify verdict rate**: More verdicts mean better calibration, which means better scores, which means better-composed context, which means better agent outputs, which means more Verify passes. A virtuous cycle -- as long as the Verify Cells are themselves reliable.

---

## 11. Open Questions

1. **Axis independence assumption**: The calibration model treats each axis independently (5 separate Beta-Binomial trackers). But axes may be correlated -- a Signal with high relevance is likely to also have high quality. Should the calibrator use a multivariate model? A 5-dimensional Dirichlet? The answer depends on whether the correlation structure is stable or context-dependent.

2. **Cold-start calibration**: A new Score Cell starts with an uninformative prior (Beta(2, 2)). How many Verify verdicts does it need before its temperature estimate is reliable? Empirically, temperature scaling needs ~50-100 samples. Until then, should the system use a conservative default temperature (T = 1.5, reducing all scores)?

3. **Adversarial scorers**: If a Score Cell is deliberately miscalibrated (adversarial or buggy), precision-weighted aggregation will down-weight it, but only after enough data accumulates to detect the miscalibration. Is there a faster detection mechanism? Anomaly detection on the per-cell ECE trajectory could flag outliers.

4. **Verify Cell reliability**: The entire calibration loop assumes Verify Cells provide reliable ground truth. But some Verify Cells (LLM judges, heuristic checks) are themselves uncertain. Should Verify verdicts carry a confidence weight that modulates the calibration update? This leads to a second-order calibration problem.

5. **Score drift**: If the underlying distribution of Signal quality changes over time (e.g., the agent improves and produces better outputs), the calibrators must adapt. The rolling window addresses this, but the window size is a tradeoff: too short and calibration is noisy; too long and it lags behind drift. Adaptive window sizing (e.g., ADWIN algorithm) could help.
