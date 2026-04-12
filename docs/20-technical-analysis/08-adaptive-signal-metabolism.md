# Adaptive Signal Metabolism

> Signals are living organisms. They compete for attention, reproduce when useful, die when obsolete, and evolve through mutation and selection. The TA subsystem is an ecological system governed by Hebbian learning and replicator dynamics.

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
