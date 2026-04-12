# Predictive Geometry and Resonant Pattern Ecosystems

> Topological Data Analysis (TDA) extracts shape from time series. Persistence landscapes provide a Banach space for pattern comparison. Resonant patterns are living organisms with HDC genomes that compete for attention via VCG auction.


> **Implementation**: Specified

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

## Implementation details

### Rips filtration: point cloud sizing and memory

The Rips filtration builds a simplicial complex from a point cloud. The critical cost constraint is the O(n^2) distance matrix:

```rust
/// Rips filtration configuration.
pub struct RipsFiltrationConfig {
    /// Maximum number of points in the point cloud.
    /// Memory cost: O(n^2) for the distance matrix.
    ///   n = 1,000:  ~8 MB (f64 distances)
    ///   n = 5,000:  ~200 MB
    ///   n = 10,000: ~800 MB
    /// Default: 2,000 (keeps memory under 32 MB).
    pub max_points: usize,

    /// Distance metric for the point cloud.
    pub distance_metric: DistanceMetric,

    /// Maximum filtration scale (distances beyond this are ignored).
    /// Default: f64::MAX (no cutoff). Set lower to reduce computation.
    pub max_scale: f64,

    /// Maximum homological dimension to compute.
    /// 0 = connected components only. 1 = + loops. 2 = + voids.
    /// Default: 1 (components and loops). Higher dimensions are
    /// exponentially more expensive.
    pub max_dimension: usize,
}

pub enum DistanceMetric {
    /// Euclidean distance: sqrt(sum((x_i - y_i)^2)).
    /// Standard choice for delay-embedded time series.
    Euclidean,
    /// Maximum norm: max(|x_i - y_i|).
    /// Cheaper to compute, produces similar persistence diagrams.
    Chebyshev,
    /// Correlation distance: 1 - pearson_correlation(x, y).
    /// Use when magnitudes are uninformative (scale-invariant).
    Correlation,
}
```

For point clouds exceeding `max_points`, subsample uniformly at random. The persistence diagram is stable under subsampling: the bottleneck distance between the full and subsampled diagrams is bounded by 2 * the Hausdorff distance of the subsample (stability theorem, Cohen-Steiner et al., 2007).

### Persistence computation: algorithm selection

```rust
/// Persistence algorithm configuration.
pub struct PersistenceConfig {
    /// Algorithm choice.
    pub algorithm: PersistenceAlgorithm,
    /// Rust library for computation.
    /// Recommended: `ripser` crate (Rust port of Ripser).
    /// Fallback: `gudhi-rs` bindings if available.
    pub backend: PersistenceBackend,
}

pub enum PersistenceAlgorithm {
    /// Ripser (Bauer, 2021): optimized for Rips complexes.
    /// Uses implicit representations to avoid storing the full complex.
    /// Memory: O(n^2) for the distance matrix only.
    /// Speed: fastest known algorithm for Rips persistence.
    Ripser,
    /// Standard persistence via matrix reduction.
    /// Memory: O(m) where m = number of simplices (can be huge).
    /// Use only for non-Rips filtrations.
    MatrixReduction,
    /// Cohomology-based algorithm (de Silva et al., 2011).
    /// Faster than matrix reduction for high-dimensional features.
    /// Good choice when max_dimension >= 2.
    Cohomology,
}

pub enum PersistenceBackend {
    /// Pure Rust Ripser port.
    RipserRs,
    /// GUDHI bindings (requires C++ library).
    GudhiBindings,
}
```

**Recommendation**: Use `Ripser` + `RipserRs` for all standard use cases. Switch to `Cohomology` only when computing dimension >= 2 persistence on large point clouds.

### Delay embedding: dynamic parameter selection

Takens' embedding theorem requires choosing `embedding_dim` and `delay`. These are selected dynamically from the data:

```rust
/// Select delay embedding parameters from the time series.
///
/// delay: first minimum of the average mutual information (AMI).
///   AMI measures nonlinear dependence between x(t) and x(t+tau).
///   The first minimum gives the smallest tau where the lagged values
///   provide maximally independent information.
///
/// embedding_dim: smallest d where the false nearest neighbors (FNN)
///   fraction drops below 1%. FNN counts how many "close" points in
///   d dimensions are no longer close in d+1 dimensions.
pub struct DelayEmbeddingSelector {
    /// Maximum lag to test for AMI minimum.
    pub max_delay: usize,          // default: 50
    /// Maximum dimension to test for FNN.
    pub max_dim: usize,            // default: 10
    /// FNN threshold: stop when FNN fraction < this.
    pub fnn_threshold: f64,        // default: 0.01
}

impl DelayEmbeddingSelector {
    pub fn select(&self, series: &[f64]) -> (usize, usize) {
        let delay = self.first_ami_minimum(series);
        let dim = self.fnn_dimension(series, delay);
        (dim, delay)
    }

    fn first_ami_minimum(&self, series: &[f64]) -> usize {
        let mut prev_ami = f64::MAX;
        for tau in 1..=self.max_delay.min(series.len() / 4) {
            let ami = average_mutual_information(series, tau);
            if ami > prev_ami {
                return tau - 1; // previous tau was the minimum
            }
            prev_ami = ami;
        }
        self.max_delay // no minimum found, use max
    }

    fn fnn_dimension(&self, series: &[f64], delay: usize) -> usize {
        for d in 1..=self.max_dim {
            let fnn_frac = false_nearest_neighbors(series, d, delay);
            if fnn_frac < self.fnn_threshold {
                return d;
            }
        }
        self.max_dim // no clean embedding, use max
    }
}
```

**Detrending**: Before delay embedding, remove trends to prevent non-stationarity from dominating the topology. Apply first-order differencing: `x'(t) = x(t) - x(t-1)`. For strongly trending series, apply second-order differencing.

### Persistence landscape: discretization and construction

The persistence landscape is discretized on a uniform grid for practical computation:

```rust
/// Discretize a persistence landscape on a uniform grid.
///
/// The grid spans [t_min, t_max] with n_grid points.
/// At each grid point, evaluate the k-th layer function.
pub struct LandscapeDiscretization {
    /// Grid resolution (number of points).
    pub n_grid: usize,           // default: 500
    /// Number of landscape layers to compute.
    pub n_layers: usize,         // default: 5
    /// Grid range (auto-detected from diagram if not specified).
    pub t_min: Option<f64>,
    pub t_max: Option<f64>,
}

/// Construct discretized landscape from a persistence diagram.
///
/// For each (b, d) pair in the diagram, create a tent function:
///   f(t) = t - b        for b <= t <= (b+d)/2
///   f(t) = d - t        for (b+d)/2 <= t <= d
///   f(t) = 0            otherwise
///
/// Layer k at grid point t is the k-th largest tent function value at t.
pub fn discretize_landscape(
    diagram: &PersistenceDiagram,
    config: &LandscapeDiscretization,
) -> Vec<Vec<f64>> {
    let (t_min, t_max) = match (config.t_min, config.t_max) {
        (Some(a), Some(b)) => (a, b),
        _ => {
            let births: Vec<f64> = diagram.points.iter().map(|(b, _)| *b).collect();
            let deaths: Vec<f64> = diagram.points.iter().map(|(_, d)| *d).collect();
            (*births.iter().min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap_or(&0.0),
             *deaths.iter().max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap_or(&1.0))
        }
    };

    let step = (t_max - t_min) / config.n_grid as f64;
    let mut layers = vec![vec![0.0; config.n_grid]; config.n_layers];

    for grid_idx in 0..config.n_grid {
        let t = t_min + grid_idx as f64 * step;
        let mut values: Vec<f64> = diagram.points.iter()
            .map(|(b, d)| {
                let mid = (b + d) / 2.0;
                if t >= *b && t <= mid {
                    t - b
                } else if t > mid && t <= *d {
                    d - t
                } else {
                    0.0
                }
            })
            .collect();
        values.sort_by(|a, b| b.partial_cmp(a).unwrap());

        for k in 0..config.n_layers.min(values.len()) {
            layers[k][grid_idx] = values[k];
        }
    }

    layers
}
```

### Pattern niche: center and specificity

The niche center is computed as the weighted mean of recent activation states. Specificity measures how narrow the niche is:

```rust
/// Compute niche from activation history.
///
/// center = weighted mean of states where the pattern activated,
///          weighted by activation strength.
///
/// specificity = 1.0 / (1.0 + normalized_variance_of_activation_states).
///   specificity near 1.0: narrow specialist (activates in similar conditions).
///   specificity near 0.0: broad generalist (activates everywhere).
pub fn compute_niche(activation_history: &[(Vec<f64>, f64)]) -> PatternNiche {
    let total_weight: f64 = activation_history.iter().map(|(_, w)| w).sum();
    let dim = activation_history[0].0.len();

    let center: Vec<f64> = (0..dim).map(|d| {
        activation_history.iter()
            .map(|(state, w)| state[d] * w / total_weight)
            .sum()
    }).collect();

    let variance: f64 = activation_history.iter()
        .map(|(state, w)| {
            let dist_sq: f64 = state.iter().zip(&center)
                .map(|(s, c)| (s - c).powi(2))
                .sum();
            dist_sq * w / total_weight
        })
        .sum();

    let radius = variance.sqrt();
    let specificity = 1.0 / (1.0 + variance / dim as f64);

    PatternNiche { center, radius, specificity }
}
```

### VCG auction: auctioneer, payment, zero-bid prevention

The auctioneer is the heartbeat's Theta-frequency tick. At each Theta tick, active patterns bid for inclusion in the cognitive context (limited to `budget` slots).

**Payment mechanism**: Each winning pattern pays the externality it imposes -- the decrease in total welfare that others experience because this pattern occupies a slot. In practice, this equals the bid of the highest-ranked excluded pattern:

```rust
/// VCG payment computation.
///
/// For winner i with bid b_i, payment = optimal welfare without i minus
/// welfare of others when i wins.
///
/// With single-item-per-slot allocation, this simplifies to:
/// payment_i = bid of the (budget+1)-th ranked pattern.
///
/// Zero-bid prevention: patterns with weight < min_bid are excluded
/// from the auction entirely.
pub struct AuctionConfig {
    /// Maximum patterns in the cognitive context.
    pub budget: usize,           // default: 10
    /// Minimum bid to participate.
    pub min_bid: f64,            // default: 0.001
}
```

If fewer than `budget` patterns have bids above `min_bid`, all qualifying patterns win and pay zero (no competition).

### Lotka-Volterra: sensitivity analysis and domain calibration

The four Lotka-Volterra parameters have domain-specific interpretations:

| Parameter | Symbol | Chain domain | Coding domain | Default |
|---|---|---|---|---|
| Resource growth rate | alpha | How fast arbitrage opportunity regenerates | How fast new code surfaces bugs | 0.1 |
| Exploitation impact | beta | How much trading depletes the opportunity | How much testing reveals bugs | 0.02 |
| Benefit from exploitation | delta | Profit per unit of opportunity exploited | Information gain per bug found | 0.01 |
| Natural decay | gamma | Strategy obsolescence rate (MEV competition) | Bug fix rate (resolves the opportunity) | 0.05 |

**Sensitivity analysis**: The system has a stable equilibrium at:
- `resource* = gamma / delta`
- `exploitation* = alpha / beta`

Small perturbations around equilibrium oscillate with period `T = 2*pi / sqrt(alpha * gamma)`. With defaults: `T = 2*pi / sqrt(0.005) ~= 89` time units.

If `alpha * gamma` is too small, oscillations are slow and the system appears static. If `beta * delta` is too large relative to `alpha * gamma`, the system collapses (exploitation exceeds recovery).

**Calibration procedure**: Observe real resource recovery rates and exploitation impact over 20+ Theta cycles. Fit alpha, beta, delta, gamma via least-squares on the observed trajectories.

### Error handling

- **Empty persistence diagram**: If the point cloud produces no persistent features, return an empty PersistenceLandscape with zero layers.
- **Auction with zero patterns**: Return empty winners list.
- **Lotka-Volterra negative values**: Clamp resource and exploitation to `[0.0, max_resource]`. Log a warning if clamping occurs.
- **Delay embedding on short series**: If `series.len() < embedding_dim * delay`, fall back to `embedding_dim = 2, delay = 1`.
- **NaN in niche computation**: If all activation weights are zero, return a niche with center at the origin and radius = infinity (universal generalist).

### Integration wiring

```
Oracle::predict()
  -> collect recent time series (last 2000 points)
  -> DelayEmbeddingSelector::select() for dynamic dim/delay
  -> delay_embedding() to produce point cloud
  -> RipsFiltration with Ripser backend
  -> PersistenceDiagram -> PersistenceLandscape
  -> TopologyToTrajectory::predict() for topological constraints
  -> ResonantPattern ecosystem:
       -> pattern_auction() at Theta frequency
       -> lotka_volterra_update() for resource dynamics
       -> reproduce() for top patterns at Delta frequency
  -> encode landscape features as HDC vector
  -> emit as Engram
```

### Test criteria

- **Takens embedding**: Delay-embedding a sine wave with period P recovers a circle-like point cloud. The H1 persistence diagram has one dominant point with lifetime proportional to the amplitude.
- **Persistence stability**: Adding Gaussian noise with stddev sigma shifts the bottleneck distance by at most O(sigma).
- **Landscape linearity**: `landscape(A + B) == landscape(A).add(landscape(B))` for diagrams A, B.
- **VCG truthfulness**: No pattern benefits from bidding other than its true value.
- **Lotka-Volterra equilibrium**: Starting from equilibrium with default parameters, the system stays within 1% of equilibrium for 1000 steps with dt=0.1.
- **Niche convergence**: After 100 activations in similar states, the niche center is within 5% of the true activation centroid.
- **Memory budget**: 2000-point Rips filtration completes within 32 MB memory.

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
