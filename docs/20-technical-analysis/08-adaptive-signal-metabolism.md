# Adaptive Signal Metabolism

> Signals are living organisms. They compete for attention, reproduce when useful, die when obsolete, and evolve through mutation and selection. The TA subsystem is an ecological system governed by Hebbian learning and replicator dynamics.


> **Implementation**: Specified

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [06-hyperdimensional-ta](./06-hyperdimensional-ta.md) for HDC encoding, [05-witness-as-ta-generalized](./05-witness-as-ta-generalized.md) for the witness pipeline
**Key sources**: `bardo-backup/prd/23-ta/03-adaptive-signal-metabolism.md`

---

## Signals as organisms

In the adaptive signal metabolism framework, every TA signal is treated as a living organism that exists within an ecological system. Signals compete for the limited resource of agent attention. Useful signals grow stronger (reproduce). Obsolete signals weaken (die). Novel mutations emerge from HDC recombination. The system self-organizes into an optimal signal ensemble without manual curation.

This is not metaphor — it is a direct implementation of replicator dynamics (Taylor & Jonker, 1978) and evolutionary game theory applied to signal selection.

### The signal as a 5-tuple

```rust
/// A signal organism in the adaptive metabolism framework.
///
/// Each signal is a 5-tuple: (f, C, H, W, ctx)
/// - f: the signal function (computation)
/// - C: confidence (self-assessed reliability)
/// - H: HDC vector (pattern identity)
/// - W: weight (fitness in the attention economy)
/// - ctx: context (domain-specific parameters)
pub struct AdaptiveSignal {
    /// Unique identifier.
    pub id: SignalId,

    /// The computation that produces this signal's value.
    /// In Rust: a closure or function pointer.
    pub function: Box<dyn Fn(&EngineState) -> f64 + Send + Sync>,

    /// Self-assessed confidence [0.0, 1.0].
    /// Updated via Hebbian learning after each prediction cycle.
    pub confidence: f64,

    /// HDC vector encoding this signal's identity.
    /// Used for similarity search and cross-domain matching.
    pub hdc_vector: HdcVector,

    /// Weight (fitness) in the attention economy.
    /// Signals with higher weight get more attention budget.
    /// Updated via replicator dynamics.
    pub weight: f64,

    /// Domain-specific context for this signal.
    pub context: SignalContext,

    /// Lineage: parent signals this was derived from (for evolution tracking).
    pub lineage: Vec<SignalId>,

    /// Generation counter (how many mutation/selection cycles).
    pub generation: u64,

    /// Birth timestamp.
    pub created_at_ms: i64,

    /// Number of times this signal has been evaluated.
    pub evaluation_count: u64,

    /// Running accuracy statistics.
    pub accuracy: ExponentialMovingAverage,
}
```

---

## Hebbian learning — "Neurons that fire together wire together"

Signal confidence is updated via Oja's rule, a normalized variant of Hebbian learning that prevents runaway weight growth:

```rust
/// Hebbian update for signal confidence.
///
/// When a signal's prediction correlates with the actual outcome,
/// confidence increases. When it anti-correlates, confidence decreases.
///
/// Uses Oja's rule (Oja, 1982) for stability:
///   Δw = η × y × (x - y × w)
///
/// where:
///   w = current confidence
///   x = signal value (prediction)
///   y = outcome (actual value)
///   η = learning rate (typically 0.01-0.05)
///
/// The (- y × w) term prevents weights from growing without bound.
pub fn hebbian_update(
    signal: &mut AdaptiveSignal,
    prediction: f64,
    outcome: f64,
    learning_rate: f64,
) {
    let delta = learning_rate * outcome * (prediction - outcome * signal.confidence);
    signal.confidence = (signal.confidence + delta).clamp(0.0, 1.0);
}

/// Batch Hebbian update across all signals in the registry.
///
/// After each prediction resolution, all signals that contributed
/// to the prediction have their confidence updated.
pub fn batch_hebbian_update(
    registry: &mut SignalRegistry,
    predictions: &[(SignalId, f64)],
    outcome: f64,
    learning_rate: f64,
) {
    for (signal_id, prediction) in predictions {
        if let Some(signal) = registry.get_mut(signal_id) {
            hebbian_update(signal, *prediction, outcome, learning_rate);
            signal.evaluation_count += 1;
            signal.accuracy.update((prediction - outcome).abs());
        }
    }
}
```

---

## Replicator dynamics — Fitness-proportionate selection

Signal weights evolve according to replicator dynamics (Taylor & Jonker, 1978): signals with above-average fitness gain weight; below-average signals lose weight. This creates a self-organizing ensemble without manual threshold tuning:

```rust
/// Replicator dynamics update for signal weights.
///
/// The replicator equation:
///   dw_i/dt = w_i × (f_i - f̄)
///
/// where:
///   w_i = weight of signal i
///   f_i = fitness of signal i
///   f̄ = average fitness across all signals
///
/// Fitness is the accuracy of the signal's predictions.
/// Signals that predict better than average grow.
/// Signals that predict worse than average shrink.
pub fn replicator_update(registry: &mut SignalRegistry, dt: f64) {
    let signals: Vec<(SignalId, f64, f64)> = registry.iter()
        .map(|s| (s.id, s.weight, s.fitness()))
        .collect();

    let total_weight: f64 = signals.iter().map(|(_, w, _)| w).sum();
    let avg_fitness: f64 = signals.iter()
        .map(|(_, w, f)| w * f / total_weight)
        .sum();

    for (id, weight, fitness) in &signals {
        let delta = weight * (fitness - avg_fitness) * dt;
        if let Some(signal) = registry.get_mut(id) {
            signal.weight = (signal.weight + delta).max(0.001);  // floor to prevent extinction
        }
    }

    // Normalize weights to sum to 1.0
    registry.normalize_weights();
}
```

### Fisher's fundamental theorem

Fisher's fundamental theorem of natural selection (Fisher, 1930) applies: the rate of increase in mean fitness equals the genetic variance in fitness. In signal terms: **the rate at which the signal ensemble improves equals the diversity of signal quality**. This has a practical implication — if all signals have similar accuracy, improvement stalls. The system must maintain diversity to keep improving.

```rust
/// Compute Fisher's variance (rate of improvement potential).
///
/// V(fitness) = Σ w_i × (f_i - f̄)²
///
/// When V is high: the ensemble is rapidly improving.
/// When V approaches 0: the ensemble has converged (may need mutation injection).
pub fn fisher_variance(registry: &SignalRegistry) -> f64 {
    let avg_fitness = registry.mean_fitness();
    registry.iter()
        .map(|s| s.weight * (s.fitness() - avg_fitness).powi(2))
        .sum()
}
```

---

## Speciation — Signal evolution

New signals emerge through mutation of successful parent signals:

```rust
/// Signal speciation: create a new signal by mutating a parent.
///
/// The mutation operator perturbs the parent's HDC vector
/// (XOR with a random noise vector of controlled density)
/// and adjusts the signal function parameters.
///
/// New signals start with low weight (0.01) and must prove
/// themselves through replicator dynamics before gaining
/// significant attention budget.
pub fn speciate(
    parent: &AdaptiveSignal,
    mutation_rate: f64,
    rng: &mut impl Rng,
) -> AdaptiveSignal {
    // Mutate HDC vector: flip bits with probability = mutation_rate
    let noise = HdcVector::random_with_density(mutation_rate, rng);
    let mutated_hv = parent.hdc_vector.xor(&noise);

    // Mutate signal function parameters
    let mutated_function = parent.function.mutate(mutation_rate, rng);

    AdaptiveSignal {
        id: SignalId::new(),
        function: mutated_function,
        confidence: 0.5,  // start neutral
        hdc_vector: mutated_hv,
        weight: 0.01,  // start with minimal weight
        context: parent.context.clone(),
        lineage: {
            let mut l = parent.lineage.clone();
            l.push(parent.id);
            l
        },
        generation: parent.generation + 1,
        created_at_ms: now_ms(),
        evaluation_count: 0,
        accuracy: ExponentialMovingAverage::new(0.1),
    }
}
```

### Fitness landscape (Sewall Wright, 1932)

The ensemble of signals exists on a fitness landscape — a surface where each point is a signal configuration and the height is its fitness. The replicator dynamics push signals uphill (toward higher fitness), but speciation (mutation) allows escape from local optima:

```rust
/// The fitness landscape for a signal ensemble.
///
/// Each signal's position is its HDC vector (in 10,240-bit space).
/// The height at each position is the signal's fitness (accuracy).
///
/// Properties:
/// - Rugged: many local optima (similar signals with different accuracy)
/// - Dynamic: the landscape shifts as market/code/research conditions change
/// - High-dimensional: 10,240 dimensions, many escape routes from local optima
///
/// Navigation via: replicator dynamics (hill climbing) + speciation (exploration)
pub struct FitnessLandscape {
    /// Current signal positions and heights.
    pub signals: Vec<(HdcVector, f64)>,

    /// Landscape roughness (variance of fitness across neighbors).
    pub roughness: f64,

    /// Landscape shift rate (how fast the landscape changes).
    pub shift_rate: f64,
}
```

### Red Queen dynamic

Following Van Valen's Red Queen hypothesis (1973): in adversarial environments, signals must continuously evolve just to maintain their fitness, because the environment (adversary) co-evolves. In the chain domain, MEV searchers adapt to agent strategies. In the coding domain, codebase structure evolves. Signals that stop evolving become obsolete:

```rust
/// Red Queen pressure: signals must evolve to maintain fitness.
///
/// Implemented as a constant downward pressure on all signal weights:
///   w_i(t+1) = w_i(t) × (1 - decay_rate) + replicator_delta
///
/// Without improvement, signals decay toward zero.
/// Only signals that continuously outperform survive.
pub fn apply_red_queen_pressure(registry: &mut SignalRegistry, decay_rate: f64) {
    for signal in registry.iter_mut() {
        signal.weight *= 1.0 - decay_rate;
    }
}
```

---

## SignalRegistry — The ecosystem container

```rust
/// The signal registry: manages the full signal ecosystem.
///
/// Contains all living signals, tracks their fitness over time,
/// manages speciation and extinction events.
pub struct SignalRegistry {
    /// All active signals.
    signals: HashMap<SignalId, AdaptiveSignal>,

    /// Maximum population (attention budget constraint).
    max_population: usize,

    /// Speciation rate (probability of mutation per generation).
    speciation_rate: f64,

    /// Extinction threshold (minimum weight before removal).
    extinction_threshold: f64,

    /// Generation counter.
    generation: u64,
}

impl SignalRegistry {
    /// Run one evolutionary step.
    ///
    /// 1. Evaluate all signals against recent data
    /// 2. Hebbian update of confidence
    /// 3. Replicator dynamics update of weights
    /// 4. Speciate: create mutations of top performers
    /// 5. Extinction: remove signals below threshold
    /// 6. Red Queen: apply constant decay pressure
    pub fn evolve_step(&mut self, data: &[Engram], outcomes: &[Engram]) {
        // 1. Evaluate
        let predictions = self.evaluate_all(data);

        // 2. Hebbian update
        for (pred, outcome) in predictions.iter().zip(outcomes) {
            batch_hebbian_update(self, &pred.signal_contributions, outcome.numeric_value(), 0.02);
        }

        // 3. Replicator dynamics
        replicator_update(self, dt: 1.0);

        // 4. Speciation
        let top_signals: Vec<_> = self.top_k(5);
        for parent in &top_signals {
            if rand::random::<f64>() < self.speciation_rate {
                let child = speciate(parent, mutation_rate: 0.05, &mut rng);
                self.insert(child);
            }
        }

        // 5. Extinction
        self.remove_below_threshold(self.extinction_threshold);

        // 6. Red Queen
        apply_red_queen_pressure(self, decay_rate: 0.001);

        // 7. Enforce population cap
        while self.signals.len() > self.max_population {
            self.remove_weakest();
        }

        self.generation += 1;
    }
}
```

---

## Heartbeat integration

The signal metabolism operates at all three cognitive speeds:

| Speed | Signal metabolism activity |
|---|---|
| **Gamma** (~5-15s) | Signals evaluate against current data. No learning. Cost: microseconds. |
| **Theta** (~75s) | Hebbian update + replicator dynamics step. Predictions resolve. |
| **Delta** (hours) | Full evolutionary step: speciation, extinction, Red Queen. Landscape analysis. |

At Gamma frequency, the signal registry is read-only — probes read signal values but don't update weights. This ensures the T0 probe system (80% of ticks costing nothing) is not disrupted by evolutionary computation.

At Theta frequency, learning happens — confidence updates and weight adjustments based on resolved predictions.

At Delta frequency, the full evolutionary cycle runs — new signals are born, old ones die, and the fitness landscape is analyzed for stagnation.

---

## Domain-specific signal contexts

```rust
/// Domain-specific context for signal metabolism.
pub enum SignalContext {
    /// Chain signals: DeFi-specific parameters.
    Chain(ChainSignalContext),

    /// Coding signals: software engineering parameters.
    Coding(CodingSignalContext),

    /// Research signals: information analysis parameters.
    Research(ResearchSignalContext),

    /// Custom domain.
    Custom(serde_json::Value),
}

pub struct ChainSignalContext {
    /// Which protocols this signal monitors.
    pub protocols: Vec<ProtocolId>,
    /// Which assets this signal tracks.
    pub assets: Vec<AssetId>,
    /// Time granularity (block-level, minute, hourly).
    pub granularity: Duration,
}

pub struct CodingSignalContext {
    /// Which crates/modules this signal monitors.
    pub scope: CodingScope,
    /// Which metrics this signal tracks.
    pub metrics: Vec<CodingMetric>,
    /// Event granularity (commit-level, CI run, daily).
    pub granularity: Duration,
}

pub struct ResearchSignalContext {
    /// Which topics this signal covers.
    pub topics: Vec<String>,
    /// Which source types this signal evaluates.
    pub source_types: Vec<SourceType>,
    /// Evaluation cadence.
    pub granularity: Duration,
}
```

---

## Implementation details

### Replicator dynamics: dt semantics and numerical stability

The `dt` parameter in `replicator_update()` represents the elapsed time in arbitrary units since the last update. In practice:

- At **Theta frequency** (~75s): `dt = 1.0` (one Theta tick = one evolutionary time unit).
- At **Delta frequency** (hours): `dt` accumulates missed Theta ticks if replicator was not called at Theta, but this is not recommended. Run replicator at every Theta tick.

**Numerical stability**: The replicator equation `dw_i/dt = w_i * (f_i - f_bar)` is solved via forward Euler. This is adequate when `dt * max(|f_i - f_bar|) < 1.0`. If this condition fails, weights can go negative.

```rust
/// Safe replicator update with stability check.
///
/// Forward Euler is stable when dt * max_fitness_deviation < 1.0.
/// If this condition fails, subdivide the step.
pub fn replicator_update_safe(registry: &mut SignalRegistry, dt: f64) {
    let signals: Vec<(SignalId, f64, f64)> = registry.iter()
        .map(|s| (s.id, s.weight, s.fitness()))
        .collect();

    let total_weight: f64 = signals.iter().map(|(_, w, _)| w).sum();
    let avg_fitness: f64 = signals.iter()
        .map(|(_, w, f)| w * f / total_weight)
        .sum();

    let max_deviation = signals.iter()
        .map(|(_, _, f)| (f - avg_fitness).abs())
        .fold(0.0f64, f64::max);

    // Subdivide if Euler would be unstable
    let n_substeps = ((dt * max_deviation).ceil() as usize).max(1);
    let sub_dt = dt / n_substeps as f64;

    for _ in 0..n_substeps {
        for (id, weight, fitness) in &signals {
            let delta = weight * (fitness - avg_fitness) * sub_dt;
            if let Some(signal) = registry.get_mut(id) {
                signal.weight = (signal.weight + delta).max(0.001);
            }
        }
    }

    registry.normalize_weights();
}
```

RK4 is an alternative but provides negligible benefit here because the replicator dynamics are evaluated at coarse (Theta) intervals and the forward Euler error is dominated by the discretization of the fitness landscape, not the integrator.

### Speciation: adaptive mutation rate

The mutation rate adapts based on Red Queen pressure and Fisher's variance:

```rust
/// Compute adaptive mutation rate.
///
/// When Fisher's variance is low (ensemble converged), increase mutation
/// to inject diversity. When variance is high (actively evolving),
/// reduce mutation to let selection operate.
///
/// The formula incorporates Red Queen pressure: in adversarial domains
/// (chain), mutation rate has a higher floor.
///
///   mutation_rate = base_rate * (1.0 + rq_pressure) / (1.0 + fisher_v / fisher_scale)
///
/// where:
///   base_rate:    0.05 (default)
///   rq_pressure:  0.0 (coding) to 1.0 (chain, adversarial)
///   fisher_v:     current Fisher's variance
///   fisher_scale: 0.1 (normalizing constant)
pub fn adaptive_mutation_rate(
    base_rate: f64,
    red_queen_pressure: f64,
    fisher_variance: f64,
) -> f64 {
    let fisher_scale = 0.1;
    let rate = base_rate * (1.0 + red_queen_pressure) / (1.0 + fisher_variance / fisher_scale);
    rate.clamp(0.01, 0.3) // never below 1% or above 30%
}
```

### HdcVector::random_with_density() distribution

`random_with_density(density, rng)` generates a 10,240-bit vector where each bit is independently set to 1 with probability `density`:

- `density = 0.5`: standard dense random vector (used for codebook generation).
- `density = 0.05`: sparse noise vector (used for mutation). On average, 512 bits are flipped.
- `density = 0.001`: very sparse noise (used for fine-tuning). On average, ~10 bits are flipped.

The distribution is Bernoulli per bit. Implementation uses `rng.gen::<f64>() < density` per bit (slow) or batch generation via geometric distribution of inter-bit gaps (fast, O(density * dim) expected operations).

### Fitness computation

`s.fitness()` returns a composite score combining prediction accuracy and information value:

```rust
impl AdaptiveSignal {
    /// Compute fitness for replicator dynamics.
    ///
    /// fitness = accuracy_ema * (1.0 + information_ratio)
    ///
    /// accuracy_ema:      exponential moving average of |prediction - outcome|,
    ///                    inverted so higher accuracy = higher fitness.
    ///                    Specifically: 1.0 - accuracy.value() where accuracy
    ///                    tracks mean absolute error.
    ///
    /// information_ratio: how much unique information this signal provides
    ///                    beyond what other signals already cover.
    ///                    Computed as 1.0 - max_correlation_with_other_signals.
    ///                    Range: [0.0, 1.0]. Higher = more unique.
    pub fn fitness(&self) -> f64 {
        let accuracy_score = 1.0 - self.accuracy.value().min(1.0);
        let info_ratio = self.information_ratio.unwrap_or(0.5);
        accuracy_score * (1.0 + info_ratio)
    }
}
```

**Range**: `[0.0, 2.0]`. A signal with perfect accuracy and completely unique information scores 2.0. A signal with zero accuracy scores 0.0 regardless of uniqueness.

### Heartbeat integration state machine

Signal metabolism integrates with the heartbeat via a three-state machine:

```
State: GAMMA (read-only)
  Entry: heartbeat tick at Gamma frequency (~5-15s)
  Action: evaluate all signals, collect predictions. No weight updates.
  Transition: on Theta tick -> THETA

State: THETA (learning)
  Entry: heartbeat tick at Theta frequency (~75s)
  Action:
    1. Resolve predictions from last Theta cycle.
    2. Hebbian update of signal confidence.
    3. Replicator dynamics update of signal weights.
    4. Update Fisher's variance.
  Transition: on Delta tick -> DELTA
               on Gamma tick -> GAMMA

State: DELTA (evolution)
  Entry: heartbeat tick at Delta frequency (hours)
  Action:
    1. Run full replicator update with accumulated dt.
    2. Speciate: mutate top-k signals (k = 5, mutation_rate from adaptive formula).
    3. Extinction: remove signals with weight < extinction_threshold (default: 0.001).
    4. Red Queen pressure: decay all weights by decay_rate (default: 0.001).
    5. Enforce population cap (default: 500).
    6. Analyze fitness landscape for stagnation.
    7. If Fisher's variance < 0.001: inject 10 random signals to restore diversity.
  Transition: on Gamma tick -> GAMMA
```

### Oja's rule learning rate calibration

The learning rate `eta` for Oja's rule should be calibrated per domain:

| Domain | Recommended eta | Rationale |
|---|---|---|
| Chain (DeFi) | 0.01 | High noise, slow learning prevents overfit to flash events. |
| Coding | 0.05 | Lower noise, faster adaptation to codebase changes. |
| Research | 0.02 | Moderate noise, medium adaptation speed. |

The learning rate can be made adaptive: `eta = base_eta / (1.0 + 0.01 * evaluation_count)`. This annealing schedule reduces learning rate as the signal accumulates more observations, following the Robbins-Monro conditions for stochastic approximation convergence.

### normalize_weights() semantics

`normalize_weights()` rescales all signal weights so they sum to 1.0:

```rust
impl SignalRegistry {
    /// Normalize weights so they sum to 1.0.
    ///
    /// Preserves relative proportions. Does NOT preserve absolute magnitudes.
    /// After normalization, each weight represents the signal's share of the
    /// total attention budget.
    ///
    /// If total weight is zero (all signals extinct), distributes weight
    /// uniformly: each signal gets 1.0 / n.
    pub fn normalize_weights(&mut self) {
        let total: f64 = self.signals.values().map(|s| s.weight).sum();
        if total < 1e-12 {
            // All weights near zero: reset to uniform
            let uniform = 1.0 / self.signals.len() as f64;
            for s in self.signals.values_mut() {
                s.weight = uniform;
            }
        } else {
            for s in self.signals.values_mut() {
                s.weight /= total;
            }
        }
    }
}
```

The sum-to-1.0 convention means weights are interpretable as probability distributions over signals. This is consistent with the replicator dynamics formulation where weights are population shares.

### Error handling

- **Division by zero in replicator**: If `total_weight == 0`, skip the replicator step and log a warning. This can only happen if all signals were externally removed.
- **NaN in Hebbian update**: If prediction or outcome is NaN, skip the update for that signal.
- **Population collapse**: If signal count drops below `min_population` (default: 10) after extinction, inject `min_population - current` random signals.
- **Infinite fitness**: Clamp fitness to `[0.0, 10.0]` to prevent a single signal from dominating.

### Test criteria

- **Replicator conservation**: After `replicator_update()`, total weight is unchanged (before normalization).
- **Replicator stability**: With `dt = 1.0` and fitness deviations < 1.0, no weight goes negative.
- **Hebbian convergence**: A signal that always predicts correctly converges to confidence ~1.0 within 100 updates.
- **Speciation diversity**: After speciation, `hamming_similarity(parent, child)` is between 0.9 and 0.99 for mutation_rate = 0.05.
- **Extinction threshold**: After `evolve_step()`, no signal has weight < `extinction_threshold` (they are removed).
- **Fisher's variance monotonicity**: When all signals have identical fitness, Fisher's variance is 0.0.
- **Normalize idempotence**: Calling `normalize_weights()` twice produces the same result.

---

## Academic foundations

- Taylor, P. D., & Jonker, L. B. (1978). "Evolutionary Stable Strategies and Game Dynamics." *Mathematical Biosciences*, 40(1-2), 145-156. — Replicator dynamics.
- Fisher, R. A. (1930). *The Genetical Theory of Natural Selection*. Clarendon Press. — Fisher's fundamental theorem.
- Wright, S. (1932). "The Roles of Mutation, Inbreeding, Crossbreeding, and Selection in Evolution." *Proceedings of the Sixth International Congress of Genetics*, 1, 356-366. — Fitness landscapes.
- Van Valen, L. (1973). "A New Evolutionary Law." *Evolutionary Theory*, 1, 1-30. — Red Queen hypothesis.
- Oja, E. (1982). "Simplified neuron model as a principal component analyzer." *Journal of Mathematical Biology*, 15(3), 267-273. — Oja's learning rule.
- Hebb, D. O. (1949). *The Organization of Behavior*. Wiley. — Hebbian learning.
- Kleyko, D., et al. (2022). "A Survey on Hyperdimensional Computing." *ACM Computing Surveys*, 54(6). — HDC operations for signal encoding.

---

## Cross-references

- See [06-hyperdimensional-ta.md](./06-hyperdimensional-ta.md) for HDC encoding fundamentals
- See [05-witness-as-ta-generalized.md](./05-witness-as-ta-generalized.md) for the data pipeline that feeds signals
- See [10-predictive-geometry-and-resonant-patterns.md](./10-predictive-geometry-and-resonant-patterns.md) for resonant pattern ecosystems
- See [12-somatic-ta-and-emergent-multiscale.md](./12-somatic-ta-and-emergent-multiscale.md) for emergent intelligence from signal interactions
