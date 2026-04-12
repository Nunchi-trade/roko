# Predictive Geometry and Resonant Pattern Ecosystems

> Topological Data Analysis (TDA) extracts shape from time series. Persistence landscapes provide a Banach space for pattern comparison. Resonant patterns are living organisms with HDC genomes that compete for attention via VCG auction.

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [06-hyperdimensional-ta](./06-hyperdimensional-ta.md) for HDC encoding, [08-adaptive-signal-metabolism](./08-adaptive-signal-metabolism.md) for evolutionary dynamics
**Key sources**: `bardo-backup/prd/23-ta/05-predictive-geometry.md`, `bardo-backup/prd/23-ta/06-resonant-pattern-ecosystem.md`

---

## Part I: Predictive Geometry via TDA

### Why topology for time series

Standard TA reduces time series to statistics (means, variances, correlations). These statistics capture numerical properties but miss **shape** — the topological structure of the data. A time series with two peaks and a valley has different topology than one with a gradual rise, even if both have the same mean and variance.

Topological Data Analysis (TDA) extracts shape features that are:

- **Coordinate-free**: invariant to scaling, translation, and monotone transformations
- **Multi-scale**: captures structure at every resolution simultaneously
- **Robust**: small perturbations in data produce small changes in topology
- **Composable**: topological features from different domains can be compared

### Persistence diagrams

A persistence diagram tracks the birth and death of topological features (connected components, loops, voids) across a filtration of the data:

```rust
/// A persistence diagram: the set of (birth, death) pairs for
/// topological features at a given dimension.
///
/// Each point (b, d) represents a feature that appears at scale b
/// and disappears at scale d. Long-lived features (d - b is large)
/// represent genuine structure. Short-lived features are noise.
pub struct PersistenceDiagram {
    /// The topological dimension (0 = components, 1 = loops, 2 = voids).
    pub dimension: usize,

    /// The (birth, death) pairs.
    pub points: Vec<(f64, f64)>,
}

impl PersistenceDiagram {
    /// Compute persistence from a time series using Rips filtration.
    pub fn from_time_series(series: &[f64], dimension: usize) -> Self {
        // Embed time series as point cloud using delay embedding
        // (Takens' theorem guarantees topological equivalence)
        let point_cloud = delay_embedding(series, embedding_dim: 3, delay: 1);

        // Build Rips complex filtration
        let filtration = rips_filtration(&point_cloud, max_scale: f64::MAX);

        // Compute persistent homology
        let diagram = compute_persistence(&filtration, dimension);

        diagram
    }

    /// Lifetime of a feature: death - birth.
    pub fn lifetimes(&self) -> Vec<f64> {
        self.points.iter().map(|(b, d)| d - b).collect()
    }

    /// The persistence of the longest-lived feature.
    pub fn max_persistence(&self) -> f64 {
        self.lifetimes().iter().cloned().fold(0.0, f64::max)
    }
}
```

### Persistence landscapes (Bubenik, 2015)

Persistence landscapes transform persistence diagrams into functions that live in a Banach space — enabling arithmetic operations (addition, subtraction, scaling) on topological features:

```rust
/// A persistence landscape: a sequence of piecewise-linear functions
/// derived from a persistence diagram.
///
/// Bubenik (2015): persistence landscapes form a Banach space,
/// enabling statistical operations (mean, variance, hypothesis testing)
/// on topological features.
///
/// Key property: the landscape is a FUNCTION, not a set of points.
/// Functions can be added, subtracted, scaled, and integrated —
/// operations that are not well-defined on persistence diagrams directly.
pub struct PersistenceLandscape {
    /// The landscape functions λ_k(t) for k = 1, 2, 3, ...
    /// λ_1 is the outermost envelope, λ_2 the next, etc.
    pub layers: Vec<PiecewiseLinearFunction>,

    /// The dimension of the underlying persistence diagram.
    pub dimension: usize,
}

pub struct PiecewiseLinearFunction {
    /// Breakpoints (t_i, f(t_i)).
    pub points: Vec<(f64, f64)>,
}

impl PersistenceLandscape {
    /// Convert a persistence diagram to a landscape.
    pub fn from_diagram(diagram: &PersistenceDiagram) -> Self {
        let mut tent_functions: Vec<PiecewiseLinearFunction> = diagram.points.iter()
            .map(|(b, d)| {
                let mid = (b + d) / 2.0;
                let height = (d - b) / 2.0;
                PiecewiseLinearFunction {
                    points: vec![(*b, 0.0), (mid, height), (*d, 0.0)],
                }
            })
            .collect();

        // Sort by peak height (descending) to get layers
        tent_functions.sort_by(|a, b| {
            b.max_value().partial_cmp(&a.max_value()).unwrap()
        });

        // Build layers by taking the k-th largest value at each t
        let layers = build_layers(&tent_functions);

        PersistenceLandscape { layers, dimension: diagram.dimension }
    }

    /// Add two landscapes (point-wise).
    pub fn add(&self, other: &PersistenceLandscape) -> PersistenceLandscape {
        // Point-wise addition of corresponding layers
        let layers = self.layers.iter()
            .zip_longest(other.layers.iter())
            .map(|pair| match pair {
                Both(a, b) => a.pointwise_add(b),
                Left(a) => a.clone(),
                Right(b) => b.clone(),
            })
            .collect();

        PersistenceLandscape { layers, dimension: self.dimension }
    }

    /// Scale a landscape by a constant.
    pub fn scale(&self, factor: f64) -> PersistenceLandscape {
        let layers = self.layers.iter()
            .map(|l| l.pointwise_scale(factor))
            .collect();

        PersistenceLandscape { layers, dimension: self.dimension }
    }

    /// L^p norm of the landscape (measure of total topological complexity).
    pub fn lp_norm(&self, p: f64) -> f64 {
        self.layers.iter()
            .map(|l| l.lp_integral(p))
            .sum::<f64>()
            .powf(1.0 / p)
    }
}
```

### Topology-to-trajectory mapping

The key application: use topological features to constrain trajectory predictions:

```rust
/// Map topological features to trajectory forecasts.
///
/// The persistence landscape provides topological constraints
/// on future price/metric trajectories. For example:
///
/// β_0 (component count):
///   - If the current time series has 2 connected components,
///     any predicted trajectory must eventually reduce to 1
///     (convergence) or increase to 3+ (divergence).
///   - This constrains the set of possible futures.
///
/// β_1 (loop count):
///   - If the time series has a persistent 1-cycle (loop),
///     the predicted trajectory should account for periodic behavior.
///
/// The mapping uses kernel regression: given a topological feature,
/// predict the trajectory parameters.
pub struct TopologyToTrajectory {
    /// Trained kernel regression model.
    kernel: KernelRegression,

    /// Historical (topology, trajectory) pairs for training.
    training_data: Vec<(PersistenceLandscape, Vec<f64>)>,
}

impl TopologyToTrajectory {
    /// Predict future trajectory from current topology.
    pub fn predict(&self, current_topology: &PersistenceLandscape) -> TrajectoryPrediction {
        let weights = self.kernel.compute_weights(current_topology);
        let predicted = self.training_data.iter()
            .zip(weights.iter())
            .map(|((_, traj), w)| traj.iter().map(|v| v * w).collect::<Vec<_>>())
            .fold(vec![0.0; self.training_data[0].1.len()], |acc, t| {
                acc.iter().zip(t.iter()).map(|(a, b)| a + b).collect()
            });

        TrajectoryPrediction {
            values: predicted,
            topological_constraints: self.extract_constraints(current_topology),
        }
    }
}

pub struct TrajectoryPrediction {
    /// Predicted future values.
    pub values: Vec<f64>,

    /// Topological constraints on the prediction.
    pub topological_constraints: Vec<TopologicalConstraint>,
}

pub enum TopologicalConstraint {
    /// β_0 constraint: the trajectory must have this many components.
    ComponentCount { expected: usize, tolerance: usize },

    /// β_1 constraint: the trajectory exhibits periodic behavior.
    PeriodicBehavior { period_estimate: f64, confidence: f64 },

    /// Persistence constraint: features with lifetime > threshold
    /// will likely persist in the future.
    FeaturePersistence { min_lifetime: f64, count: usize },
}
```

---

## Part II: Resonant Pattern Ecosystems

### Patterns as organisms

Resonant patterns extend the adaptive signal metabolism framework (see [08-adaptive-signal-metabolism.md](./08-adaptive-signal-metabolism.md)) by treating multi-signal patterns as organisms with HDC genomes:

```rust
/// A resonant pattern: a multi-signal pattern that acts as an organism
/// in the pattern ecosystem.
///
/// Each pattern has:
/// - An HDC "genome" (its encoded structure)
/// - A weight (fitness in the attention economy)
/// - A niche (the environmental conditions where it activates)
/// - A lineage (evolutionary history)
pub struct ResonantPattern {
    /// HDC genome: the pattern's identity vector.
    pub genome: HdcVector,

    /// Weight/fitness in the attention economy.
    pub weight: f64,

    /// The conditions under which this pattern activates.
    /// Encoded as a region in the state space.
    pub niche: PatternNiche,

    /// The signals that compose this pattern.
    pub signals: Vec<SignalId>,

    /// Evolutionary lineage (parent patterns).
    pub lineage: Vec<PatternId>,

    /// Generation counter.
    pub generation: u64,

    /// Historical accuracy when this pattern activated.
    pub accuracy_history: Vec<f64>,

    /// Topological fingerprint (persistence landscape summary).
    pub topo_fingerprint: Option<PersistenceLandscape>,
}

pub struct PatternNiche {
    /// Center of the niche in state space.
    pub center: Vec<f64>,

    /// Radius of activation.
    pub radius: f64,

    /// Niche specificity: narrow (specialist) vs. broad (generalist).
    pub specificity: f64,
}
```

### Reproductive algebra in HDC space

Patterns reproduce by combining parent genomes via HDC operations:

```rust
/// Pattern reproduction: combine two parent patterns into an offspring.
///
/// The reproductive algebra uses HDC operations:
/// - Bundle (majority vote): inherit traits from both parents
/// - Bind (XOR): create new associations
/// - Permute (rotate): shift temporal relationships
///
/// The offspring inherits structure from both parents but is
/// distinct — like biological sexual reproduction.
pub fn reproduce(
    parent_a: &ResonantPattern,
    parent_b: &ResonantPattern,
    mutation_rate: f64,
    rng: &mut impl Rng,
) -> ResonantPattern {
    // Crossover: bundle both genomes (majority vote preserves shared structure)
    let offspring_genome = parent_a.genome.bundle_with(&parent_b.genome);

    // Mutation: XOR with random noise
    let noise = HdcVector::random_with_density(mutation_rate, rng);
    let mutated = offspring_genome.xor(&noise);

    // Niche: interpolate between parent niches
    let niche = PatternNiche {
        center: parent_a.niche.center.iter()
            .zip(parent_b.niche.center.iter())
            .map(|(a, b)| (a + b) / 2.0)
            .collect(),
        radius: (parent_a.niche.radius + parent_b.niche.radius) / 2.0,
        specificity: (parent_a.niche.specificity + parent_b.niche.specificity) / 2.0,
    };

    ResonantPattern {
        genome: mutated,
        weight: 0.01,  // start with minimal weight
        niche,
        signals: merge_signal_sets(&parent_a.signals, &parent_b.signals),
        lineage: vec![parent_a.id(), parent_b.id()],
        generation: parent_a.generation.max(parent_b.generation) + 1,
        accuracy_history: vec![],
        topo_fingerprint: None,
    }
}
```

### VCG auction competition

Patterns compete for the limited attention budget through the VCG auction (Vickrey 1961, Clarke 1971, Groves 1973):

```rust
/// Pattern competition via VCG auction.
///
/// When multiple patterns activate simultaneously, they bid for
/// inclusion in the agent's cognitive context.
///
/// Bid = pattern_weight × niche_match × daimon_urgency
///
/// VCG truthfulness: each winner pays the second-highest bid,
/// preventing bid inflation.
pub fn pattern_auction(
    active_patterns: &[ResonantPattern],
    state: &EngineState,
    budget: usize,
) -> Vec<(PatternId, f64)> {
    let bids: Vec<(PatternId, f64)> = active_patterns.iter()
        .map(|p| {
            let niche_match = p.niche.match_score(state);
            let bid = p.weight * niche_match * state.daimon_urgency();
            (p.id(), bid)
        })
        .collect();

    // Sort by bid, take top `budget` patterns
    let mut sorted = bids.clone();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    // VCG payment: each winner pays the second-highest excluded bid
    let winners: Vec<_> = sorted.iter().take(budget).collect();
    winners.iter().map(|(id, bid)| {
        let payment = sorted.get(budget).map(|(_, b)| *b).unwrap_or(0.0);
        (*id, *bid - payment)  // surplus = bid - payment
    }).collect()
}
```

### Lotka-Volterra predator-prey dynamics

Patterns that deplete the same market opportunity (edge) interact via predator-prey dynamics:

```rust
/// Lotka-Volterra dynamics for patterns competing for the same edge.
///
/// When multiple patterns exploit the same alpha source,
/// the resource (edge) depletes. This creates predator-prey
/// oscillations: patterns grow → edge depletes → patterns shrink →
/// edge recovers → patterns grow again.
///
/// dx/dt = αx - βxy  (prey = edge opportunity)
/// dy/dt = δxy - γy  (predator = pattern exploiting the edge)
pub fn lotka_volterra_update(
    patterns: &mut [(ResonantPattern, f64)],  // (pattern, current_exploitation)
    edge_resources: &mut HashMap<String, f64>,
    dt: f64,
) {
    for (pattern, exploitation) in patterns.iter_mut() {
        for edge_id in pattern.exploited_edges() {
            if let Some(resource) = edge_resources.get_mut(edge_id) {
                let alpha = 0.1;  // resource growth rate
                let beta = 0.02;  // exploitation impact
                let delta = 0.01;  // benefit from exploitation
                let gamma = 0.05; // natural decay of exploitation

                // Prey (resource): grows naturally, depleted by exploitation
                let d_resource = alpha * *resource - beta * *resource * *exploitation;

                // Predator (exploitation): grows from resource, decays naturally
                let d_exploit = delta * *resource * *exploitation - gamma * *exploitation;

                *resource += d_resource * dt;
                *exploitation += d_exploit * dt;
            }
        }
    }
}
```

### Price equation — Partitioning evolutionary change

The Price equation (Price, 1970) partitions evolutionary change in the pattern ecosystem into selection and transmission components:

```rust
/// Price equation for pattern ecosystem analysis.
///
/// Δ(z̄) = Cov(w, z) / w̄ + E(w × Δz) / w̄
///
/// where:
///   z̄ = mean trait value (e.g., accuracy)
///   w = fitness (weight)
///   Cov(w, z) = selection component (fitter patterns have higher z)
///   E(w × Δz) = transmission component (mutation/drift)
///
/// This tells us: how much of the improvement in the pattern ensemble
/// is due to selection (bad patterns dying) vs. mutation (new patterns
/// being better than their parents)?
pub fn price_equation(
    patterns: &[ResonantPattern],
    trait_fn: impl Fn(&ResonantPattern) -> f64,
) -> PriceDecomposition {
    let fitnesses: Vec<f64> = patterns.iter().map(|p| p.weight).collect();
    let traits: Vec<f64> = patterns.iter().map(&trait_fn).collect();

    let mean_fitness: f64 = fitnesses.iter().sum::<f64>() / fitnesses.len() as f64;
    let mean_trait: f64 = traits.iter().sum::<f64>() / traits.len() as f64;

    // Covariance(fitness, trait) = selection pressure
    let covariance: f64 = fitnesses.iter().zip(traits.iter())
        .map(|(w, z)| (w - mean_fitness) * (z - mean_trait))
        .sum::<f64>() / fitnesses.len() as f64;

    let selection = covariance / mean_fitness;

    PriceDecomposition {
        total_change: 0.0,  // computed from generation-over-generation comparison
        selection_component: selection,
        transmission_component: 0.0,  // requires parent-offspring comparison
    }
}
```

---

## Academic foundations

- Bubenik, P. (2015). "Statistical Topological Data Analysis using Persistence Landscapes." *JMLR*, 16(3), 77-102. — Persistence landscapes as Banach space elements.
- Takens, F. (1981). "Detecting strange attractors in turbulence." *Lecture Notes in Mathematics*, 898, 366-381. — Delay embedding theorem for time series topology.
- Price, G. R. (1970). "Selection and Covariance." *Nature*, 227, 520-521. — The Price equation for partitioning evolutionary change.
- Vickrey, W. (1961). "Counterspeculation, Auctions, and Competitive Sealed Tenders." *Journal of Finance*, 16(1). — VCG auction for pattern competition.
- Lotka, A. J. (1925). *Elements of Physical Biology*. Williams & Wilkins. — Predator-prey dynamics.
- Volterra, V. (1926). "Fluctuations in the Abundance of a Species considered Mathematically." *Nature*, 118, 558-560. — Population dynamics equations.
- Carlsson, G. (2009). "Topology and Data." *Bulletin of the AMS*, 46(2), 255-308. — TDA foundations.

---

## Cross-references

- See [06-hyperdimensional-ta.md](./06-hyperdimensional-ta.md) for HDC genome encoding
- See [07-spectral-liquidity-manifolds.md](./07-spectral-liquidity-manifolds.md) for Riemannian geometry (complementary to TDA)
- See [08-adaptive-signal-metabolism.md](./08-adaptive-signal-metabolism.md) for signal-level evolution (patterns are composed of signals)
- See [12-somatic-ta-and-emergent-multiscale.md](./12-somatic-ta-and-emergent-multiscale.md) for emergent intelligence from pattern interactions
