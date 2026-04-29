# Advanced Geometry and Integration

> Depth for [07-spectral-liquidity-manifolds.md](../../docs/20-technical-analysis/07-spectral-liquidity-manifolds.md), [10-predictive-geometry-and-resonant-patterns.md](../../docs/20-technical-analysis/10-predictive-geometry-and-resonant-patterns.md), [12-somatic-ta-and-emergent-multiscale.md](../../docs/20-technical-analysis/12-somatic-ta-and-emergent-multiscale.md), [14-sheaf-tropical-geometry.md](../../docs/20-technical-analysis/14-sheaf-tropical-geometry.md). Expresses research-stage mathematical concepts as target-state Cell specializations with clear composition paths from existing primitives.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, HDC fingerprint, Score), [02-CELL](../../unified/02-CELL.md) (Cell, Route protocol, Verify protocol, Score protocol, Observe protocol), [03-GRAPH](../../unified/03-GRAPH.md) (Graph, Loop pattern), [04-hdc-pattern-encoding-and-metabolism.md](./04-hdc-pattern-encoding-and-metabolism.md) (HDC algebra), [03-oracle-as-score-cell.md](./03-oracle-as-score-cell.md) (CalibrationTracker)

---

## 1. Research-Stage vs Implementable: A Clear Boundary

This document describes four mathematical frameworks that are not implementable as specified today. They are expressed here as **target-state Cell specializations** to show:

1. How they compose with existing primitives (no new concepts needed).
2. What their naive initial implementation looks like (lookup tables, heuristics).
3. What the full mathematical realization requires (and why it is research-stage).
4. What concrete steps move each from research to runtime.

| Framework | Naive implementation (today) | Full realization (target) | Blocker |
|---|---|---|---|
| Spectral liquidity manifolds | Lookup tables + linear interpolation | Riemannian geodesic solver | Requires differentiable pool state models |
| Topological data analysis | Persistence diagram feature extraction | Sheaf neural networks (Bodnar 2022) | Computational cost of persistent homology |
| Sheaf consistency | Pairwise oracle correlation checks | Sheaf Laplacian spectral analysis | Dimensionality of restriction maps |
| Tropical geometry | Decision tree with learned thresholds | Tropical polynomial optimization | Theoretical: tropical VCG proof needed |

The design principle: **start naive, graduate to geometric**. Each framework has a lookup-table implementation that captures the intuition, and a mathematical implementation that captures the full theory. The Cell interface is the same for both -- only the internal logic changes.

---

## 2. Spectral Liquidity Manifolds: Route Cell with Geometric Metric

### The Insight

DeFi execution costs form a curved space. Every trade traverses a landscape where the cost depends on pool depth, gas, timing, and opportunity cost. These costs vary non-linearly with trade size. A Route Cell that models this landscape as a Riemannian manifold can find optimal execution paths (geodesics) that minimize total cost.

### As a Route Cell

```rust
/// Spectral Liquidity Manifold: a Route Cell where the routing metric
/// is a Riemannian tensor over execution cost dimensions.
///
/// Dimensions of the metric tensor:
///   g_ij where i,j in {slippage, gas, time, opportunity}
///
/// A geodesic on this manifold = the execution path that minimizes
/// total cost. Different trade sizes create different metrics (the
/// manifold is size-dependent), so optimal routing varies with order magnitude.
///
/// NAIVE IMPLEMENTATION (today):
///   Lookup table of (pool, size, time) -> cost, with linear interpolation.
///   Routes by argmin over the table.
///
/// TARGET IMPLEMENTATION:
///   Metric tensor g_ij(x) learned from historical executions.
///   Geodesic solver (Runge-Kutta on the geodesic equation).
///   Ricci scalar R for stability assessment.
///
/// Location: `crates/roko-learn/src/geometry/manifold.rs`
pub struct ManifoldRouteCell {
    /// Current implementation mode.
    mode: ManifoldMode,
}

pub enum ManifoldMode {
    /// Lookup table with linear interpolation.
    /// Sufficient for discrete venue selection.
    Naive(CostLookupTable),
    /// Full Riemannian geodesic solver.
    /// Required for continuous execution path optimization.
    Geometric(RiemannianSolver),
}

/// The naive implementation: a cost lookup table.
pub struct CostLookupTable {
    /// (pool_id, trade_size_bucket, time_bucket) -> cost vector
    entries: HashMap<(PoolId, SizeBucket, TimeBucket), CostVector>,
}

pub struct CostVector {
    pub slippage: f64,      // price impact (basis points)
    pub gas: f64,           // gas cost (USD equivalent)
    pub time: f64,          // execution latency (seconds)
    pub opportunity: f64,   // opportunity cost of waiting (USD)
}

impl RouteProtocol for ManifoldRouteCell {
    async fn route(
        &self,
        candidates: &[RouteCandidate],
        ctx: &CellContext,
    ) -> RouteDecision {
        match &self.mode {
            ManifoldMode::Naive(table) => {
                // Naive: lookup cost for each candidate, pick minimum
                let costs: Vec<f64> = candidates.iter()
                    .map(|c| table.lookup(c.pool, c.size, ctx.current_time()).total_cost())
                    .collect();

                let best_idx = costs.iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
                    .map(|(i, _)| i)
                    .unwrap_or(0);

                RouteDecision {
                    selected: candidates[best_idx].clone(),
                    confidence: 0.7, // lookup tables have limited confidence
                    reasoning: "Minimum-cost route via lookup table".into(),
                }
            }
            ManifoldMode::Geometric(solver) => {
                // Geometric: solve geodesic equation for optimal path
                let start = ctx.current_state.as_manifold_point();
                let targets: Vec<ManifoldPoint> = candidates.iter()
                    .map(|c| c.as_manifold_point())
                    .collect();

                let geodesics = solver.compute_geodesics(start, &targets);
                let best = geodesics.iter()
                    .min_by(|a, b| a.path_length.partial_cmp(&b.path_length).unwrap())
                    .unwrap();

                RouteDecision {
                    selected: candidates[best.target_idx].clone(),
                    confidence: best.curvature_stability, // stability from Ricci scalar
                    reasoning: format!(
                        "Geodesic path (length={:.4}, curvature={:.4})",
                        best.path_length, best.ricci_scalar
                    ),
                }
            }
        }
    }
}
```

### The Metric Tensor

The full geometric implementation requires a metric tensor at each point of the manifold -- a symmetric positive-definite matrix that defines local distances:

```rust
/// The metric tensor at a point on the liquidity manifold.
///
/// g_ij(x) defines the cost of infinitesimal movement in direction (i,j)
/// at point x. The manifold is 4-dimensional:
///   dim 0: slippage (price impact)
///   dim 1: gas (execution cost)
///   dim 2: time (latency)
///   dim 3: opportunity (cost of waiting)
///
/// The tensor is LEARNED from historical execution data:
///   Each executed trade contributes a sample of (position, cost).
///   The tensor is fit via positive-definite matrix regression.
///
/// The Ricci scalar R at a point measures stability:
///   R > 0: market self-corrects (like a sphere -- geodesics converge)
///   R < 0: perturbations amplify (like a saddle -- geodesics diverge)
///   R = 0: flat space (linear cost model, geodesics are straight lines)
pub struct MetricTensor {
    /// 4x4 symmetric positive-definite matrix.
    pub components: [[f64; 4]; 4],
    /// Ricci scalar curvature at this point.
    pub ricci_scalar: f64,
}
```

### Graduation Path

| Stage | What | When |
|---|---|---|
| 0. Discrete | Pick cheapest venue from a static list | Now (already implemented) |
| 1. Lookup table | Interpolated cost table, updated hourly | Next (low engineering cost) |
| 2. Linear metric | Constant metric tensor (flat manifold) | After sufficient execution data |
| 3. Full Riemannian | Learned metric tensor + geodesic solver | Research-stage (needs differentiable pool models) |

---

## 3. Topological Data Analysis: Signal Metadata via Persistence

### The Insight

Time series have SHAPE -- topological features (loops, connected components, voids) that persist across scales. A price series with "two peaks and a valley" has different topology from "gradual rise," even if both have the same mean and variance. TDA extracts these shape features as persistence diagrams, which become Signal metadata.

### As Signal Metadata

```rust
/// Persistence diagram as Signal metadata.
///
/// Every time-series Signal can carry a persistence diagram that
/// describes its topological shape. This metadata is:
///   - Computed once when the Signal is created
///   - Stored alongside the Signal in Store
///   - Queryable: "find Signals with similar topology"
///   - Used by Score Cells for shape-based prediction
///
/// NAIVE IMPLEMENTATION (today):
///   Extract (birth, death) pairs for 0-dimensional features
///   (connected components) only. O(n log n) via union-find.
///
/// TARGET IMPLEMENTATION:
///   Full Rips filtration persistent homology up to dimension 2.
///   Persistence landscapes (Bubenik 2015) for statistical comparison.
///   Takens embedding for attractor reconstruction.
///
/// Location: `crates/roko-primitives/src/topology/persistence.rs`
pub struct PersistenceDiagram {
    /// Topological dimension (0 = components, 1 = loops, 2 = voids).
    pub dimension: usize,
    /// (birth, death) pairs for features at this dimension.
    pub features: Vec<PersistenceFeature>,
}

pub struct PersistenceFeature {
    /// Scale at which this feature appears.
    pub birth: f64,
    /// Scale at which this feature disappears.
    pub death: f64,
    /// Persistence = death - birth (lifetime of the feature).
    pub persistence: f64,
}

impl Signal {
    /// Attach persistence diagram as metadata.
    pub fn with_persistence(mut self, diagrams: Vec<PersistenceDiagram>) -> Self {
        self.metadata.persistence = Some(diagrams);
        self
    }

    /// Extract the most persistent feature (strongest topological structure).
    pub fn max_persistence(&self) -> Option<f64> {
        self.metadata.persistence.as_ref()
            .and_then(|diagrams| {
                diagrams.iter()
                    .flat_map(|d| d.features.iter())
                    .map(|f| f.persistence)
                    .max_by(|a, b| a.partial_cmp(b).unwrap())
            })
    }
}
```

### Takens Embedding: Attractor Reconstruction

Takens' theorem (1981) guarantees that a time series can be embedded in a higher-dimensional space where the topology of the underlying dynamical system's attractor is preserved. This is how TDA connects to prediction:

```rust
/// Takens embedding: reconstruct attractor from time series.
///
/// Given a scalar time series x(t), the delay embedding:
///   y(t) = [x(t), x(t-tau), x(t-2*tau), ..., x(t-(d-1)*tau)]
///
/// produces a d-dimensional point cloud that (for generic tau and
/// d >= 2*dim(attractor) + 1) is topologically equivalent to the
/// true attractor.
///
/// This means: topological features of the embedded point cloud
/// reveal the DYNAMICS of the underlying system, not just the statistics.
///
/// NAIVE IMPLEMENTATION:
///   Fixed delay tau=1, embedding dimension d=3.
///   Compute persistence of the embedded point cloud.
///
/// TARGET IMPLEMENTATION:
///   Adaptive tau (via mutual information minimization).
///   Adaptive d (via false nearest neighbors criterion).
///   Streaming persistence updates as new data arrives.
///
/// Location: `crates/roko-primitives/src/topology/takens.rs`
pub fn delay_embedding(
    series: &[f64],
    dimension: usize,   // embedding dimension d
    delay: usize,       // time delay tau
) -> Vec<Vec<f64>> {
    let n_points = series.len() - (dimension - 1) * delay;
    (0..n_points)
        .map(|i| {
            (0..dimension)
                .map(|d| series[i + d * delay])
                .collect()
        })
        .collect()
}
```

### Persistence Landscapes: Statistical Operations on Shape

Persistence landscapes (Bubenik 2015) convert persistence diagrams into functions in a Banach space, enabling arithmetic (addition, subtraction, scaling) and statistics (mean, variance, hypothesis testing) on topological features:

```rust
/// Persistence landscape: functional representation of persistence diagram.
///
/// Enables: mean landscape, landscape distance, landscape confidence bands.
/// These operations are NOT possible on raw persistence diagrams (which are
/// multisets, not vectors).
///
/// The landscape lambda_k(t) at level k is the k-th largest value of the
/// tent functions centered at each persistence point.
///
/// Location: `crates/roko-primitives/src/topology/landscape.rs`
pub struct PersistenceLandscape {
    /// Piecewise-linear functions, one per layer.
    pub layers: Vec<PiecewiseLinear>,
    /// Underlying topological dimension.
    pub dimension: usize,
}

impl PersistenceLandscape {
    /// Compute distance between two landscapes.
    /// L^p norm of the difference (p=2 for standard metric).
    pub fn distance(&self, other: &PersistenceLandscape, p: f64) -> f64 {
        self.layers.iter()
            .zip_longest(other.layers.iter())
            .map(|pair| {
                let diff = match pair {
                    Both(a, b) => a.subtract(b),
                    Left(a) => a.clone(),
                    Right(b) => b.negate(),
                };
                diff.lp_norm(p)
            })
            .sum::<f64>()
            .powf(1.0 / p)
    }
}
```

---

## 4. Sheaf Theory: Oracle Consistency Checking

### The Insight

Multiple Oracle subsystems produce predictions that should be mutually consistent. The chain oracle's price prediction should cohere with the liquidity manifold's cost estimate. The coding oracle's build time prediction should cohere with the dependency risk model. Sheaf theory provides the mathematical framework for checking this consistency.

### As a Verify Cell

```rust
/// Sheaf Consistency Verify Cell: checks that oracle predictions
/// form a globally consistent section.
///
/// The oracle subsystem graph G has:
///   Vertices: oracle subsystems (chain, coding, research, TDA, manifold, ...)
///   Edges: pairs that must be consistent (chain <-> manifold, etc.)
///
/// A cellular sheaf assigns:
///   F(vertex) = prediction space of that subsystem
///   F(edge) = comparison space for consistency
///   restriction maps: project subsystem predictions into comparison space
///
/// CONSISTENCY = coboundary delta(s) = 0
///   meaning: all pairs of adjacent oracles agree in their shared comparison space.
///
/// INCONSISTENCY = ||delta(s)||^2 > threshold
///   meaning: some oracle predictions contradict each other.
///   H^1 (first cohomology) measures STRUCTURAL contradictions that
///   cannot be resolved by adjusting individual predictions.
///
/// NAIVE IMPLEMENTATION (today):
///   Pairwise correlation checks between oracle outputs.
///   Flag when two oracles disagree beyond threshold.
///
/// TARGET IMPLEMENTATION:
///   Full sheaf Laplacian L_F = delta^T delta.
///   Spectral analysis: ker(L_F) = globally consistent predictions.
///   H^1 computation for structural inconsistency detection.
///
/// Location: `crates/roko-learn/src/consistency/sheaf.rs`
pub struct SheafConsistencyCell {
    mode: SheafMode,
}

pub enum SheafMode {
    /// Pairwise correlation checks.
    Naive(PairwiseChecker),
    /// Full sheaf Laplacian spectral analysis.
    Spectral(SheafLaplacianSolver),
}

/// Naive implementation: pairwise oracle agreement check.
pub struct PairwiseChecker {
    /// Pairs of oracles that should agree.
    pairs: Vec<(OracleId, OracleId)>,
    /// Maximum acceptable disagreement.
    threshold: f64,
}

impl VerifyProtocol for SheafConsistencyCell {
    async fn verify(&self, signal: &Signal, ctx: &CellContext) -> Verdict {
        match &self.mode {
            SheafMode::Naive(checker) => {
                // Collect predictions from all relevant oracles
                let predictions = ctx.collect_oracle_predictions().await;

                // Check pairwise consistency
                let mut max_disagreement = 0.0;
                let mut disagreeing_pair = None;

                for (oracle_a, oracle_b) in &checker.pairs {
                    let pred_a = predictions.get(oracle_a);
                    let pred_b = predictions.get(oracle_b);

                    if let (Some(a), Some(b)) = (pred_a, pred_b) {
                        let disagreement = a.normalized_distance(b);
                        if disagreement > max_disagreement {
                            max_disagreement = disagreement;
                            disagreeing_pair = Some((oracle_a, oracle_b));
                        }
                    }
                }

                let pass = max_disagreement < checker.threshold;
                Verdict {
                    pass,
                    reward: 1.0 - max_disagreement,
                    evidence: Evidence::ConsistencyCheck {
                        max_disagreement,
                        disagreeing_pair: disagreeing_pair.map(|(a, b)| (a.clone(), b.clone())),
                    },
                    message: if pass {
                        format!("Oracle predictions consistent (max disagreement {:.4})", max_disagreement)
                    } else {
                        format!("Oracle inconsistency detected: {:?} disagree by {:.4}", disagreeing_pair, max_disagreement)
                    },
                }
            }
            SheafMode::Spectral(solver) => {
                // Full sheaf Laplacian analysis (research-stage)
                let section = ctx.collect_oracle_section().await;
                let inconsistency = solver.compute_inconsistency(&section);
                let cohomology = solver.compute_h1(&section);

                Verdict {
                    pass: inconsistency < solver.threshold,
                    reward: 1.0 - inconsistency.min(1.0),
                    evidence: Evidence::SheafCohomology {
                        inconsistency,
                        h1_dimension: cohomology.dimension(),
                        structural_contradictions: cohomology.generators(),
                    },
                    message: format!(
                        "Sheaf analysis: inconsistency={:.4}, H^1 dim={}",
                        inconsistency, cohomology.dimension()
                    ),
                }
            }
        }
    }
}
```

### Sheaf Laplacian and Diffusion

The sheaf Laplacian L_F = delta^T delta generalizes the graph Laplacian to vector-valued data. Its kernel is the space of globally consistent predictions. Diffusion with the sheaf Laplacian drives oracle predictions toward consistency:

```rust
/// Sheaf Laplacian diffusion: drive predictions toward consistency.
///
/// The diffusion equation:
///   ds/dt = -L_F * s
///
/// drives the section s toward ker(L_F) -- the space of globally
/// consistent predictions. After sufficient diffusion time,
/// all oracles will agree in their shared comparison spaces.
///
/// This is an alternative to "pick one oracle and trust it":
/// instead, blend all oracles toward mutual consistency.
///
/// The smallest nonzero eigenvalue lambda_1 of L_F measures
/// how quickly diffusion converges (the "consistency gap").
pub fn sheaf_diffusion_step(
    section: &mut SheafSection,
    laplacian: &SheafLaplacian,
    step_size: f64,
) {
    let gradient = laplacian.apply(section);
    for (i, value) in section.vertex_values.iter_mut().enumerate() {
        for (j, v) in value.iter_mut().enumerate() {
            *v -= step_size * gradient[i][j];
        }
    }
}
```

---

## 5. Tropical Geometry: Piecewise-Linear Decision Boundaries

### The Insight

Tropical algebra replaces (addition, multiplication) with (min/max, addition). In tropical arithmetic, polynomials become piecewise-linear functions. This means decision boundaries computed tropically are piecewise-linear -- fast to evaluate, interpretable, and exact (no approximation from discretization).

### As a Route Cell with Algebraic Boundaries

```rust
/// Tropical Route Cell: piecewise-linear decision boundaries
/// computed algebraically from tropical polynomials.
///
/// In tropical geometry:
///   a ⊕ b = min(a, b)    (tropical addition = min)
///   a ⊙ b = a + b        (tropical multiplication = addition)
///
/// A tropical polynomial p(x) = min_i(a_i + b_i^T x) defines a
/// piecewise-linear function. The "tropical hypersurface"
/// (where the minimum is achieved by multiple terms simultaneously)
/// is the decision boundary.
///
/// For routing: each term corresponds to a candidate route,
/// with a_i = fixed cost and b_i = variable cost vector.
/// The tropical hypersurface divides the input space into regions
/// where each route is optimal.
///
/// NAIVE IMPLEMENTATION (today):
///   Decision tree with learned thresholds (functionally equivalent).
///   Each leaf is a routing decision. Boundaries are axis-aligned.
///
/// TARGET IMPLEMENTATION:
///   Tropical polynomials with non-axis-aligned boundaries.
///   Tropical VCG: Vickrey auction with tropical polynomial valuations.
///   Exact gradient-free optimization via tropical geometry.
///
/// Location: `crates/roko-learn/src/geometry/tropical.rs`
pub struct TropicalRouteCell {
    mode: TropicalMode,
}

pub enum TropicalMode {
    /// Decision tree (piecewise-linear with axis-aligned boundaries).
    Naive(DecisionTree),
    /// Tropical polynomial (general piecewise-linear boundaries).
    Polynomial(TropicalPolynomial),
}

/// A tropical polynomial: p(x) = trop_sum_i (a_i trop_prod_j x_j^{n_ij})
///                              = min_i (a_i + sum_j n_ij * x_j)
pub struct TropicalPolynomial {
    /// Terms: each term is (constant, coefficient_vector).
    /// p(x) = min over terms of (constant + coefficients . x)
    pub terms: Vec<TropicalTerm>,
}

pub struct TropicalTerm {
    /// Constant term a_i (fixed cost of this route).
    pub constant: f64,
    /// Coefficient vector b_i (variable cost per input dimension).
    pub coefficients: Vec<f64>,
    /// Which route/action this term corresponds to.
    pub route_id: RouteId,
}

impl TropicalPolynomial {
    /// Evaluate: find which term achieves the minimum (the optimal route).
    pub fn evaluate(&self, x: &[f64]) -> (RouteId, f64) {
        self.terms.iter()
            .map(|term| {
                let value = term.constant
                    + term.coefficients.iter().zip(x).map(|(c, xi)| c * xi).sum::<f64>();
                (term.route_id.clone(), value)
            })
            .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .unwrap()
    }

    /// Find the tropical hypersurface (decision boundary).
    /// Points where two or more terms achieve the minimum simultaneously.
    pub fn decision_boundaries(&self) -> Vec<HyperplaneSegment> {
        let mut boundaries = Vec::new();
        for i in 0..self.terms.len() {
            for j in (i+1)..self.terms.len() {
                // Boundary between terms i and j:
                // a_i + b_i . x = a_j + b_j . x
                // (b_i - b_j) . x = a_j - a_i
                let normal: Vec<f64> = self.terms[i].coefficients.iter()
                    .zip(&self.terms[j].coefficients)
                    .map(|(bi, bj)| bi - bj)
                    .collect();
                let offset = self.terms[j].constant - self.terms[i].constant;

                boundaries.push(HyperplaneSegment { normal, offset, routes: (i, j) });
            }
        }
        boundaries
    }
}
```

### Tropical VCG (Target State)

The VCG auction (currently used for context assembly in Compose protocol) can be reformulated tropically. In tropical VCG, bidder valuations are tropical polynomials, and the VCG allocation + payment is computed via tropical polynomial algebra:

```rust
/// Tropical VCG: auction with piecewise-linear valuations.
///
/// Standard VCG: allocate to maximize social welfare, charge externality price.
/// Tropical VCG: valuations are tropical polynomials -> allocation is a
/// tropical optimization problem -> exact solution via linear programming.
///
/// Advantage over standard VCG:
///   - Closed-form solution (no iterative optimization)
///   - Piecewise-linear structure is interpretable
///   - Composable: tropical sum of tropical polynomials is a tropical polynomial
///
/// This is RESEARCH-STAGE. The theoretical connection is clear but
/// the implementation requires tropical linear programming infrastructure
/// that does not exist in the current codebase.
///
/// Location: future `crates/roko-learn/src/geometry/tropical_vcg.rs`
pub struct TropicalVcg {
    /// Bidder valuations as tropical polynomials.
    pub valuations: Vec<TropicalPolynomial>,
    /// Budget constraint (maximum total allocation).
    pub budget: f64,
}
```

---

## 6. Somatic Integration: IIT Phi as an Observe Cell

### The Insight

Integrated Information Theory (Tononi 2004) measures how much a system is "more than the sum of its parts." The Phi measure (information integration) applied to the agent's subsystems detects when subsystem combinations produce emergent capability -- capability that none of the subsystems has alone.

### As an Observe Cell (Lens)

```rust
/// IIT Phi measurement: Observe Cell (Lens) over the entire system.
///
/// Phi = information generated by the whole that is not generated
/// by any partition into independent parts.
///
/// For the agent's 9 primary subsystems:
///   Full enumeration: 2^9 - 1 = 511 bipartitions
///   For each bipartition: compute mutual information between parts
///   Phi = min over all bipartitions of the information lost by partitioning
///
/// High Phi = subsystems are deeply integrated (the whole > sum of parts)
/// Low Phi = subsystems are operating independently (no emergent capability)
///
/// NAIVE IMPLEMENTATION (today):
///   Approximate Phi via 4 carefully chosen bipartitions (not all 511).
///   Use mutual information between recent Pulses as proxy for integration.
///
/// TARGET IMPLEMENTATION:
///   Full bipartition enumeration with cached mutual information.
///   PID (Partial Information Decomposition) to separate synergy from redundancy.
///   Real-time Phi tracking as a Lens observable.
///
/// Location: `crates/roko-learn/src/integration/phi.rs`
pub struct PhiObserveCell {
    mode: PhiMode,
    /// The subsystems to measure integration across.
    subsystem_ids: Vec<SubsystemId>,
}

pub enum PhiMode {
    /// Approximate: 4 canonical bipartitions.
    Approximate(ApproximatePhi),
    /// Full: all bipartitions with caching.
    Full(FullPhi),
}

impl ObserveProtocol for PhiObserveCell {
    /// Read-only observation of system integration.
    async fn observe(&self, ctx: &CellContext) -> ObservationSignal {
        match &self.mode {
            PhiMode::Approximate(approx) => {
                // Collect recent Pulse activity per subsystem
                let activity: Vec<Vec<f64>> = self.subsystem_ids.iter()
                    .map(|id| ctx.bus.recent_activity(id, 100))
                    .collect();

                // 4 canonical bipartitions:
                //   1. Each subsystem alone vs rest
                //   2. First half vs second half
                //   3. Even-indexed vs odd-indexed
                //   4. High-activity vs low-activity
                let partitions = approx.canonical_partitions(&activity);
                let phi = partitions.iter()
                    .map(|(part_a, part_b)| {
                        let mi_whole = mutual_information_whole(&activity);
                        let mi_parts = mutual_information(&activity, part_a)
                            + mutual_information(&activity, part_b);
                        mi_whole - mi_parts
                    })
                    .min_by(|a, b| a.partial_cmp(b).unwrap())
                    .unwrap_or(0.0);

                ObservationSignal {
                    metric: "phi".into(),
                    value: phi,
                    interpretation: if phi > 0.5 {
                        "High integration: subsystems producing emergent capability"
                    } else {
                        "Low integration: subsystems operating independently"
                    }.into(),
                }
            }
            _ => todo!("Full Phi implementation")
        }
    }
}
```

### PID Synergy Detection

Partial Information Decomposition separates the information that subsystem combinations provide into synergy (information available ONLY from the combination, not from parts individually) and redundancy (information available from any part alone):

```rust
/// PID synergy detection: which subsystem COMBINATIONS produce
/// emergent capability?
///
/// For subsystems A, B predicting target T:
///   I(A,B;T) = Redundancy(A,B;T) + Unique(A;T) + Unique(B;T) + Synergy(A,B;T)
///
/// Synergy > 0 means A and B together predict T better than either alone.
/// This identifies which subsystem pairings are "more than sum of parts."
///
/// Example: chain oracle + coding oracle together predict deployment
/// success better than either alone (synergy), because deployment
/// depends on both market conditions (gas, congestion) and code quality.
pub struct PidSynergyDetector {
    /// Minimum synergy to report.
    threshold: f64,
}

impl PidSynergyDetector {
    /// Find subsystem pairs with significant synergy.
    pub fn detect_synergies(
        &self,
        activity: &[Vec<f64>],
        target: &[f64],
    ) -> Vec<SynergyResult> {
        let n = activity.len();
        let mut results = Vec::new();

        for i in 0..n {
            for j in (i+1)..n {
                let joint_mi = mutual_information_2d(&activity[i], &activity[j], target);
                let mi_i = mutual_information_1d(&activity[i], target);
                let mi_j = mutual_information_1d(&activity[j], target);
                let redundancy = mi_i.min(mi_j); // Minimum specific information
                let synergy = joint_mi - mi_i - mi_j + redundancy;

                if synergy > self.threshold {
                    results.push(SynergyResult {
                        subsystem_a: i,
                        subsystem_b: j,
                        synergy,
                        joint_mi,
                    });
                }
            }
        }

        results.sort_by(|a, b| b.synergy.partial_cmp(&a.synergy).unwrap());
        results
    }
}
```

---

## What This Enables

1. **Graduated complexity**: Each framework starts with a naive implementation (lookup table, pairwise checks, decision tree) and can graduate to the full mathematical realization without changing the Cell interface. The system improves its own routing/prediction quality as computational resources allow.

2. **Geometric routing intuition**: The manifold framework gives a principled answer to "how should I route this trade/request?" -- follow the geodesic. Even the naive implementation (cost lookup) benefits from this conceptual clarity.

3. **Shape-aware prediction**: TDA extracts topological features that statistical methods miss. "This price series has the same shape as the one before the last crash" is a prediction that moving averages cannot make.

4. **Consistency-enforced predictions**: Sheaf theory ensures that when multiple oracles produce predictions, they do not contradict each other. Contradictions are detected algebraically, not by expensive cross-validation.

5. **Integration awareness**: IIT Phi measurement tells the agent when its subsystems are genuinely collaborating vs operating in isolation. Low Phi suggests subsystem coupling needs improvement.

---

## Feedback Loops

| Loop | Participants | Signal | Timescale |
|---|---|---|---|
| **Manifold learning** | Execution outcomes -> MetricTensor update | Cost observations | Per-execution |
| **Topology tracking** | New time-series data -> persistence diagram update | Shape features | Per-Signal |
| **Consistency enforcement** | Oracle predictions -> sheaf coboundary -> prediction adjustment | Inconsistency measure | Per-prediction-batch |
| **Integration monitoring** | All subsystem activity -> Phi measurement -> coupling adjustment | Phi value | Theta (periodic) |
| **Synergy discovery** | PID analysis -> configuration changes to increase synergy | Synergy scores | Delta (offline) |

---

## Open Questions

1. **Computational feasibility**: Full persistent homology is O(n^3) in the number of simplices. For real-time time series with 1000+ points, this may exceed Gamma frequency budgets. Is the naive approximation (0-dimensional features only) sufficient for practical prediction?

2. **Metric tensor learning**: The Riemannian metric tensor requires sufficient execution data to learn accurately. In sparse markets (thin liquidity, few trades), the tensor estimate may be unreliable. How should uncertainty about the metric itself be handled?

3. **Sheaf dimension choices**: The restriction maps require choosing which dimensions of each oracle's prediction space are "comparable." These choices encode domain knowledge about what SHOULD be consistent. How should they be discovered vs. specified?

4. **Phi computational cost**: Even approximate Phi over 9 subsystems with 4 bipartitions requires significant mutual information estimation. Is this observation worth its computational cost? Can it be computed lazily (only when integration problems are suspected)?

5. **Tropical VCG incentive compatibility**: Standard VCG is incentive-compatible (truthful bidding is dominant strategy). Does this property hold when valuations are tropical polynomials? The theoretical proof is open.

---

## Implementation Tasks

- [ ] Implement `CostLookupTable` as naive Route Cell for liquidity routing in `crates/roko-learn/src/geometry/`
- [ ] Add persistence diagram computation (0-dimensional only, union-find algorithm) in `crates/roko-primitives/src/topology/`
- [ ] Implement `PersistenceLandscape` with distance metric for topological Signal comparison
- [ ] Add Takens delay embedding with fixed parameters (d=3, tau=1) for time series Signals
- [ ] Implement `PairwiseChecker` as naive sheaf consistency Verify Cell in `crates/roko-learn/src/consistency/`
- [ ] Implement approximate Phi Observe Cell with 4 canonical bipartitions
- [ ] Add `TropicalPolynomial` evaluation (piecewise-linear routing) in `crates/roko-learn/src/geometry/tropical.rs`
- [ ] Create graduation-path configuration: mode selection (Naive vs Geometric) via `roko.toml`
- [ ] Add PID synergy detector as an offline analysis tool in `roko learn` CLI subcommand
- [ ] Wire persistence diagram as optional Signal metadata field in `crates/roko-core/src/signal.rs`
