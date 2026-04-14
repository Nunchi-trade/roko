# Spectral Liquidity Manifolds

> Riemannian geometry applied to DeFi execution costs. Liquidity pools form a curved manifold where geodesics are optimal execution paths and curvature indicates structural risk.


> **Implementation**: Specified

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [02-chain-oracles](./02-chain-oracles.md) for chain TA primitives, [06-hyperdimensional-ta](./06-hyperdimensional-ta.md) for pattern encoding
**Key sources**: `bardo-backup/prd/23-ta/02-spectral-liquidity-manifolds.md`

---

## Abstract

DeFi execution is not a simple price lookup. Every trade traverses a **liquidity landscape** where costs depend on pool depth, gas fees, timing, and opportunity costs. These costs vary non-linearly with trade size, time, and market conditions. The spectral liquidity manifold framework models this landscape using Riemannian geometry — the mathematical framework for curved spaces.

The core insight: a liquidity landscape is a curved space. The metric tensor encodes execution costs at each point. Geodesics (shortest paths on the manifold) are the optimal execution routes. Curvature measures structural stability — positive curvature (like a sphere) means the market self-corrects; negative curvature (like a saddle) means small perturbations amplify.

While this framework is natively chain-specific (DeFi liquidity is the domain), the mathematical structure generalizes. Any domain with spatially varying costs — CI/CD pipeline routing, research strategy selection, resource allocation — can be modeled as a manifold with a cost metric.

---

## The state manifold

The liquidity manifold is a smooth differentiable manifold M where each point represents a DeFi portfolio state:

```rust
/// A point on the liquidity manifold.
///
/// Coordinates represent the portfolio state:
/// (asset_0_balance, asset_1_balance, ..., asset_n_balance, liquidity_position_params)
///
/// The manifold dimension equals the number of independent state variables.
pub struct ManifoldPoint {
    /// Portfolio state coordinates.
    pub coordinates: Vec<f64>,

    /// Which protocol/pool this point belongs to.
    pub protocol: ProtocolId,

    /// Timestamp of the state observation.
    pub timestamp_ms: i64,
}

/// A tangent vector at a point on the manifold.
///
/// Tangent vectors represent infinitesimal trades — small changes
/// in portfolio state. The metric tensor measures the "cost" of
/// moving in each direction.
pub struct TangentVector {
    /// Components of the tangent vector in local coordinates.
    pub components: Vec<f64>,

    /// The point where this tangent vector is attached.
    pub base_point: ManifoldPoint,
}
```

### The metric tensor

The metric tensor g_ij defines the cost of moving from one state to another. It encodes four types of execution cost:

```rust
/// The metric tensor at a point on the liquidity manifold.
///
/// g_ij = slippage_ij + gas_ij + time_ij + opportunity_ij
///
/// Each component captures a different execution cost:
/// - slippage: price impact of the trade
/// - gas: transaction fee on the blockchain
/// - time: cost of waiting for confirmation
/// - opportunity: cost of capital locked during execution
pub struct MetricTensor {
    /// The n×n matrix of metric components.
    pub components: Vec<Vec<f64>>,

    /// Dimension of the manifold.
    pub dim: usize,
}

impl MetricTensor {
    /// Compute the metric tensor at a given point.
    ///
    /// This requires querying the current liquidity state of the
    /// underlying pools and computing the cost gradient in each direction.
    pub fn compute(point: &ManifoldPoint, pools: &[PoolState]) -> Self {
        let dim = point.coordinates.len();
        let mut g = vec![vec![0.0; dim]; dim];

        for i in 0..dim {
            for j in 0..dim {
                // Slippage component: d²(price_impact) / d(x_i)d(x_j)
                g[i][j] += slippage_metric(point, i, j, pools);

                // Gas component: constant per transaction, amortized
                g[i][j] += gas_metric(point, i, j);

                // Time component: confirmation time scaled by urgency
                g[i][j] += time_metric(point, i, j);

                // Opportunity component: capital lockup cost
                g[i][j] += opportunity_metric(point, i, j);
            }
        }

        MetricTensor { components: g, dim }
    }

    /// Inner product of two tangent vectors using this metric.
    /// This gives the "cost squared" of moving in direction v.
    pub fn inner_product(&self, v: &TangentVector, w: &TangentVector) -> f64 {
        let mut result = 0.0;
        for i in 0..self.dim {
            for j in 0..self.dim {
                result += self.components[i][j] * v.components[i] * w.components[j];
            }
        }
        result
    }

    /// Length of a tangent vector: the "cost" of an infinitesimal trade.
    pub fn norm(&self, v: &TangentVector) -> f64 {
        self.inner_product(v, v).sqrt()
    }
}
```

---

## Christoffel symbols — How the manifold curves

The Christoffel symbols Γ^k_ij describe how the coordinate system curves — they are the "gravitational field" of the liquidity manifold:

```rust
/// Christoffel symbols of the second kind: Γ^k_ij.
///
/// These describe how parallel transport along the manifold
/// rotates tangent vectors. In financial terms: they describe
/// how the cost of a trade changes as you move through the
/// liquidity landscape.
///
/// Γ^k_ij = (1/2) g^{kl} (∂_i g_{jl} + ∂_j g_{il} - ∂_l g_{ij})
pub struct ChristoffelSymbols {
    /// Γ^k_ij stored as [k][i][j].
    pub components: Vec<Vec<Vec<f64>>>,
    pub dim: usize,
}

impl ChristoffelSymbols {
    /// Compute Christoffel symbols from the metric tensor.
    ///
    /// Requires: metric tensor and its first derivatives at the point.
    /// Uses finite differences for derivatives when analytical forms
    /// are not available.
    pub fn compute(
        metric: &MetricTensor,
        metric_derivatives: &[MetricTensor],  // ∂_l g_{ij} for each l
    ) -> Self {
        let dim = metric.dim;
        let g_inv = metric.inverse();
        let mut gamma = vec![vec![vec![0.0; dim]; dim]; dim];

        for k in 0..dim {
            for i in 0..dim {
                for j in 0..dim {
                    for l in 0..dim {
                        gamma[k][i][j] += 0.5 * g_inv.components[k][l] * (
                            metric_derivatives[i].components[j][l] +
                            metric_derivatives[j].components[i][l] -
                            metric_derivatives[l].components[i][j]
                        );
                    }
                }
            }
        }

        ChristoffelSymbols { components: gamma, dim }
    }
}
```

---

## Geodesics — Optimal execution paths

A geodesic on the liquidity manifold is the path that minimizes total execution cost. Finding the optimal route for a DeFi trade is equivalent to solving the geodesic equation:

```rust
/// Compute the geodesic (optimal execution path) between two portfolio states.
///
/// The geodesic equation:
///   d²x^k/dt² + Γ^k_ij (dx^i/dt)(dx^j/dt) = 0
///
/// Solved numerically using 4th-order Runge-Kutta integration.
///
/// The resulting path minimizes total execution cost (slippage + gas + time + opportunity).
pub fn compute_geodesic(
    start: &ManifoldPoint,
    end: &ManifoldPoint,
    manifold: &LiquidityManifold,
    n_steps: usize,
) -> Vec<ManifoldPoint> {
    let dt = 1.0 / n_steps as f64;
    let mut path = vec![start.clone()];
    let mut velocity = initial_velocity(start, end, manifold);

    for _ in 0..n_steps {
        let point = path.last().unwrap();
        let christoffel = manifold.christoffel_at(point);

        // Geodesic equation: acceleration = -Γ^k_ij v^i v^j
        let mut acceleration = vec![0.0; manifold.dim];
        for k in 0..manifold.dim {
            for i in 0..manifold.dim {
                for j in 0..manifold.dim {
                    acceleration[k] -= christoffel.components[k][i][j]
                        * velocity.components[i]
                        * velocity.components[j];
                }
            }
        }

        // RK4 integration step
        let (new_point, new_velocity) = rk4_step(point, &velocity, &acceleration, dt);
        velocity = new_velocity;
        path.push(new_point);
    }

    path
}
```

### Geodesic interpretation

| Geodesic property | Financial meaning |
|---|---|
| **Geodesic length** | Total execution cost of the optimal path |
| **Geodesic curvature** | How far the optimal path deviates from a "straight" trade |
| **Conjugate points** | Points where alternative optimal paths exist (arbitrage opportunities) |
| **Geodesic incompleteness** | Regions where no optimal path exists (illiquid, fragmented markets) |

---

## Curvature — Structural risk

The Riemann curvature tensor and its contractions reveal structural properties of the liquidity landscape:

### Riemann curvature tensor

```rust
/// Riemann curvature tensor: R^l_{ijk}.
///
/// Measures the failure of parallel transport around an infinitesimal loop.
/// In financial terms: how much does the cost structure change as you
/// move around in the liquidity landscape?
pub struct RiemannTensor {
    pub components: Vec<Vec<Vec<Vec<f64>>>>,  // R^l_{ijk}
    pub dim: usize,
}
```

### Ricci scalar — Market stability indicator

```rust
/// Ricci scalar: R = g^{ij} R_{ij} where R_{ij} = R^k_{ikj}.
///
/// A single number that summarizes the overall curvature at a point.
///
/// R > 0 (positive curvature, sphere-like):
///   Market self-corrects. Perturbations damp out.
///   Liquidity is resilient. Safe to execute.
///
/// R = 0 (flat):
///   Execution costs are uniform. No structural effects.
///
/// R < 0 (negative curvature, saddle-like):
///   Perturbations amplify. Small trades can have outsized impact.
///   Liquidity is fragile. Exercise caution.
pub fn ricci_scalar(
    riemann: &RiemannTensor,
    metric: &MetricTensor,
) -> f64 {
    let dim = riemann.dim;
    let g_inv = metric.inverse();
    let mut scalar = 0.0;

    // Contract R^k_{ikj} to get Ricci tensor R_{ij}
    // Then contract with g^{ij} to get scalar
    for i in 0..dim {
        for j in 0..dim {
            let mut ricci_ij = 0.0;
            for k in 0..dim {
                ricci_ij += riemann.components[k][i][k][j];
            }
            scalar += g_inv.components[i][j] * ricci_ij;
        }
    }

    scalar
}
```

The Ricci scalar acts as a chain oracle signal — when it turns negative, the chain oracle increases its prediction uncertainty and the Daimon raises arousal (urgency).

---

## Parallel transport — Cross-protocol pattern transfer

Parallel transport moves a tangent vector (a trading strategy) along a path on the manifold without "rotating" it. This is how TA patterns transfer between protocols:

```rust
/// Parallel transport a vector from one point to another along a geodesic.
///
/// Financial interpretation: take a trading strategy that works on
/// Protocol A and transport it to Protocol B, adjusting for the
/// different cost structure.
///
/// d(v^k)/dt + Γ^k_ij v^i (dx^j/dt) = 0
pub fn parallel_transport(
    vector: &TangentVector,
    along_path: &[ManifoldPoint],
    manifold: &LiquidityManifold,
) -> TangentVector {
    let mut transported = vector.clone();
    let n = along_path.len();

    for step in 0..n - 1 {
        let point = &along_path[step];
        let next = &along_path[step + 1];
        let christoffel = manifold.christoffel_at(point);

        let dx: Vec<f64> = next.coordinates.iter()
            .zip(point.coordinates.iter())
            .map(|(a, b)| a - b)
            .collect();

        // Update each component: dv^k = -Γ^k_ij v^i dx^j
        let mut new_components = transported.components.clone();
        for k in 0..manifold.dim {
            let mut delta = 0.0;
            for i in 0..manifold.dim {
                for j in 0..manifold.dim {
                    delta -= christoffel.components[k][i][j]
                        * transported.components[i]
                        * dx[j];
                }
            }
            new_components[k] += delta;
        }

        transported.components = new_components;
        transported.base_point = next.clone();
    }

    transported
}
```

---

## Exponential and logarithmic maps

These maps connect the manifold to its tangent spaces, enabling local linear approximation:

```rust
/// Exponential map: project from tangent space to manifold.
///
/// Given a point p and a tangent vector v, exp_p(v) follows the
/// geodesic starting at p in direction v for unit time.
///
/// Financial interpretation: "if I execute a trade of size v
/// starting from portfolio state p, where do I end up?"
pub fn exponential_map(
    point: &ManifoldPoint,
    vector: &TangentVector,
    manifold: &LiquidityManifold,
) -> ManifoldPoint {
    // Follow geodesic from point in direction vector for t=1
    let path = compute_geodesic_from_velocity(point, vector, manifold, 100);
    path.last().cloned().unwrap()
}

/// Logarithmic map: project from manifold to tangent space.
///
/// Given two points p and q, log_p(q) is the tangent vector at p
/// that points toward q along the geodesic.
///
/// Financial interpretation: "what trade gets me from portfolio p to portfolio q
/// via the optimal (geodesic) route?"
pub fn logarithmic_map(
    from: &ManifoldPoint,
    to: &ManifoldPoint,
    manifold: &LiquidityManifold,
) -> TangentVector {
    // Solve the boundary value problem: find v such that exp_from(v) = to
    // Uses shooting method with Newton iteration
    shooting_method(from, to, manifold, max_iter: 20)
}
```

### Fréchet mean — Consensus portfolio state

```rust
/// Fréchet mean: the point on the manifold that minimizes
/// the sum of squared geodesic distances to a set of points.
///
/// Financial interpretation: the "average" portfolio state
/// that is closest to all observed states. Used to compute
/// consensus positions across a collective of agents.
///
/// Computed iteratively via the Karcher mean algorithm.
pub fn frechet_mean(
    points: &[ManifoldPoint],
    manifold: &LiquidityManifold,
    max_iter: usize,
) -> ManifoldPoint {
    let mut mean = points[0].clone();

    for _ in 0..max_iter {
        // Compute mean tangent vector
        let tangent_sum: Vec<f64> = points.iter()
            .map(|p| logarithmic_map(&mean, p, manifold))
            .fold(vec![0.0; manifold.dim], |acc, v| {
                acc.iter().zip(v.components.iter())
                    .map(|(a, b)| a + b)
                    .collect()
            });

        let mean_tangent = TangentVector {
            components: tangent_sum.iter().map(|v| v / points.len() as f64).collect(),
            base_point: mean.clone(),
        };

        // Step toward mean tangent
        let step_size = 0.5;  // damping for convergence
        let scaled = TangentVector {
            components: mean_tangent.components.iter().map(|v| v * step_size).collect(),
            base_point: mean.clone(),
        };

        mean = exponential_map(&mean, &scaled, manifold);
    }

    mean
}
```

---

## Spectral decomposition — Eigenvalue analysis

The metric tensor's eigenvalues reveal the principal directions of cost and their magnitudes:

```rust
/// Spectral decomposition of the metric tensor.
///
/// Eigenvalues: the cost magnitude in each principal direction.
///   Large eigenvalue → expensive to move in that direction.
///   Small eigenvalue → cheap to move in that direction.
///
/// Eigenvectors: the principal directions.
///   The cheapest direction is the eigenvector with smallest eigenvalue.
///   The most expensive direction has the largest eigenvalue.
///
/// Condition number (λ_max / λ_min):
///   High condition number → highly anisotropic cost structure.
///   The market strongly favors some trades over others.
pub struct SpectralDecomposition {
    pub eigenvalues: Vec<f64>,
    pub eigenvectors: Vec<Vec<f64>>,
    pub condition_number: f64,
}

impl MetricTensor {
    pub fn spectral_decomposition(&self) -> SpectralDecomposition {
        let (eigenvalues, eigenvectors) = symmetric_eigendecomposition(&self.components);
        let condition_number = eigenvalues.last().unwrap() / eigenvalues.first().unwrap();

        SpectralDecomposition {
            eigenvalues,
            eigenvectors,
            condition_number,
        }
    }
}
```

---

## Implementation details

### Metric tensor computation: Hessian of price impact

The metric tensor `g_ij` at a point is the Hessian of the total execution cost function with respect to portfolio state variables. Since analytical Hessians are unavailable for arbitrary pool types, the implementation uses central finite differences:

```rust
/// Compute the metric tensor via numerical differentiation.
///
/// Uses central finite differences on the execution cost function:
///   g_ij = d²C / dx_i dx_j
///        ≈ [C(x+ε_i+ε_j) - C(x+ε_i-ε_j) - C(x-ε_i+ε_j) + C(x-ε_i-ε_j)] / (4ε²)
///
/// The step size ε is adaptive: ε = max(|x_i| * relative_eps, absolute_eps).
pub struct MetricTensorComputer {
    /// Relative step size for finite differences.
    pub relative_eps: f64,   // default: 1e-4
    /// Absolute step size floor (prevents division by near-zero).
    pub absolute_eps: f64,   // default: 1e-8
    /// The execution cost function C(x) for a given pool configuration.
    pub cost_fn: Box<dyn Fn(&[f64], &[PoolState]) -> f64 + Send + Sync>,
}

impl MetricTensorComputer {
    /// Compute g_ij at a point using central finite differences.
    pub fn compute(&self, point: &ManifoldPoint, pools: &[PoolState]) -> MetricTensor {
        let dim = point.coordinates.len();
        let mut g = vec![vec![0.0; dim]; dim];
        let x = &point.coordinates;

        for i in 0..dim {
            let eps_i = (x[i].abs() * self.relative_eps).max(self.absolute_eps);
            for j in i..dim {
                let eps_j = (x[j].abs() * self.relative_eps).max(self.absolute_eps);

                let mut x_pp = x.clone(); x_pp[i] += eps_i; x_pp[j] += eps_j;
                let mut x_pm = x.clone(); x_pm[i] += eps_i; x_pm[j] -= eps_j;
                let mut x_mp = x.clone(); x_mp[i] -= eps_i; x_mp[j] += eps_j;
                let mut x_mm = x.clone(); x_mm[i] -= eps_i; x_mm[j] -= eps_j;

                let c_pp = (self.cost_fn)(&x_pp, pools);
                let c_pm = (self.cost_fn)(&x_pm, pools);
                let c_mp = (self.cost_fn)(&x_mp, pools);
                let c_mm = (self.cost_fn)(&x_mm, pools);

                g[i][j] = (c_pp - c_pm - c_mp + c_mm) / (4.0 * eps_i * eps_j);
                g[j][i] = g[i][j]; // symmetric
            }
        }

        MetricTensor { components: g, dim }
    }
}
```

**Configuration parameters**:

| Parameter | Default | Range | Notes |
|---|---|---|---|
| `relative_eps` | 1e-4 | 1e-6 - 1e-2 | Smaller = more accurate but noisier. 1e-4 balances accuracy and numerical noise for f64. |
| `absolute_eps` | 1e-8 | 1e-12 - 1e-4 | Floor for coordinates near zero. |

### Christoffel symbol finite difference parameters

The Christoffel symbols require first derivatives of the metric tensor. These are also computed via central finite differences:

```rust
/// Compute ∂_l g_{ij} via central finite differences on the metric.
///
///   ∂_l g_{ij} ≈ [g_{ij}(x + ε_l) - g_{ij}(x - ε_l)] / (2ε_l)
///
/// This requires 2*dim metric tensor evaluations (each itself O(dim²) cost evaluations).
/// Total cost: O(dim³) cost function evaluations per Christoffel computation.
pub fn metric_derivatives(
    computer: &MetricTensorComputer,
    point: &ManifoldPoint,
    pools: &[PoolState],
) -> Vec<MetricTensor> {
    let dim = point.coordinates.len();
    let mut derivs = Vec::with_capacity(dim);

    for l in 0..dim {
        let eps_l = (point.coordinates[l].abs() * computer.relative_eps)
            .max(computer.absolute_eps);

        let mut x_plus = point.clone();
        x_plus.coordinates[l] += eps_l;
        let g_plus = computer.compute(&x_plus, pools);

        let mut x_minus = point.clone();
        x_minus.coordinates[l] -= eps_l;
        let g_minus = computer.compute(&x_minus, pools);

        let mut dg = vec![vec![0.0; dim]; dim];
        for i in 0..dim {
            for j in 0..dim {
                dg[i][j] = (g_plus.components[i][j] - g_minus.components[i][j])
                    / (2.0 * eps_l);
            }
        }

        derivs.push(MetricTensor { components: dg, dim });
    }

    derivs
}
```

The step size for Christoffel computation should match the metric tensor step size. Using a different scale introduces inconsistency between the metric and its derivatives.

### Geodesic solver: dynamic step count and error tolerance

The geodesic solver uses adaptive 4th-order Runge-Kutta (RK4) with dynamic step count:

```rust
/// Adaptive geodesic solver with error-controlled step sizing.
///
/// Starts with `n_steps` uniform steps. After initial solve,
/// estimates local truncation error by comparing RK4 with RK2.
/// Doubles step count in regions where error exceeds tolerance.
pub struct GeodesicSolverConfig {
    /// Initial step count.
    pub initial_n_steps: usize,     // default: 100
    /// Maximum step count (prevents runaway refinement).
    pub max_n_steps: usize,         // default: 10_000
    /// Local truncation error tolerance per step.
    pub error_tolerance: f64,       // default: 1e-6
    /// Maximum geodesic parameter length (prevents infinite geodesics).
    pub max_parameter: f64,         // default: 10.0
    /// Singular point detection threshold (eigenvalue ratio).
    pub singularity_threshold: f64, // default: 1e-10
}

impl GeodesicSolverConfig {
    /// Detect singular points where the metric degenerates.
    ///
    /// A point is singular if the metric tensor's condition number
    /// exceeds 1/singularity_threshold, or if any eigenvalue is
    /// negative (the metric is no longer positive-definite).
    pub fn is_singular(&self, metric: &MetricTensor) -> bool {
        let spectral = metric.spectral_decomposition();
        let min_eigenvalue = spectral.eigenvalues.first().copied().unwrap_or(0.0);
        min_eigenvalue < self.singularity_threshold
            || spectral.condition_number > 1.0 / self.singularity_threshold
    }
}
```

**Singular point handling**: When the solver encounters a singular point (degenerate metric), it:

1. Halves the step size and retries.
2. If still singular after 3 retries, records a `GeodesicIncomplete` result with the last valid point.
3. Logs the singular location for manifold diagnostics.

### Exponential and logarithmic map parameters

```rust
/// Exponential map configuration.
pub struct ExpMapConfig {
    /// Number of geodesic integration steps.
    pub n_steps: usize,       // default: 100
    /// Error tolerance for integration.
    pub tolerance: f64,        // default: 1e-6
}

/// Logarithmic map configuration (shooting method).
///
/// The shooting method solves: find v such that exp_p(v) = q.
/// It iterates by adjusting v based on the error exp_p(v) - q.
pub struct LogMapConfig {
    /// Maximum Newton iterations for the shooting method.
    pub max_iterations: usize,     // default: 20
    /// Convergence tolerance: ||exp_p(v) - q|| < tolerance.
    pub convergence_tolerance: f64, // default: 1e-6
    /// Line search parameters (backtracking Armijo).
    pub armijo_c: f64,              // default: 1e-4
    pub armijo_tau: f64,            // default: 0.5
    /// Initial step size for Newton line search.
    pub initial_step: f64,          // default: 1.0
    /// Minimum step size before declaring failure.
    pub min_step: f64,              // default: 1e-10
}
```

The Newton line search in the logarithmic map uses backtracking Armijo conditions: accept a step if `f(x + alpha*d) <= f(x) + c*alpha*grad_f . d`, where `c = 1e-4` (sufficient decrease) and `alpha` is halved each backtrack attempt (`tau = 0.5`). Maximum backtracks: `ceil(log2(initial_step / min_step))`.

### Ricci scalar thresholds for market fragility

| Ricci scalar range | Interpretation | Agent response |
|---|---|---|
| R > 1.0 | Strongly self-correcting. Trades have predictable costs. | Execute normally. |
| 0.0 < R <= 1.0 | Mildly stable. Some cost variation. | Execute with wider slippage tolerance. |
| -0.5 <= R <= 0.0 | Neutral to mildly fragile. | Reduce position sizes by 50%. |
| -2.0 <= R < -0.5 | Fragile. Small trades amplify. | Reduce position sizes by 80%. Alert Daimon (raise Arousal). |
| R < -2.0 | Critically fragile. Market structure unstable. | Suppress all execution. Escalate to T2. |

These thresholds are configurable per protocol. Concentrated liquidity AMMs (Uniswap V3) tend toward higher curvature magnitude than constant-product AMMs (Uniswap V2), so adjust accordingly.

### Failure modes

1. **Degenerate manifold**: The metric tensor has zero or negative eigenvalues. Cause: a liquidity pool is empty or nearly so. Mitigation: skip the degenerate dimension (project out the null eigenspace) or mark the pool as unavailable.

2. **Disconnected components**: The manifold splits into disconnected regions (e.g., two isolated liquidity pools with no bridge). Geodesics between disconnected components do not exist. The solver returns `GeodesicIncomplete` with the reason `DisconnectedComponents`.

3. **Numerical instability in Christoffel symbols**: When the metric changes rapidly (high curvature), finite differences amplify truncation error. Mitigation: reduce `eps` by 10x in high-curvature regions (detected when eigenvalue ratio > 100).

4. **Ill-conditioned metric inverse**: Required for Christoffel computation. When condition number > 1e8, use pseudoinverse (SVD with eigenvalue floor at 1e-10).

5. **Geodesic divergence**: RK4 integration can diverge near singular points. The adaptive solver detects divergence when `||velocity|| > 1e6` and terminates early.

### Integration wiring

The spectral liquidity manifold integrates into the chain oracle prediction pipeline:

```
ChainOracle::predict()
  -> query on-chain pool states (via alloy provider)
  -> construct ManifoldPoint from portfolio state
  -> MetricTensorComputer::compute() at current point
  -> SpectralDecomposition for eigenvalue analysis
  -> ricci_scalar() for fragility assessment
  -> if R > threshold: compute_geodesic() for optimal execution path
  -> if R < threshold: suppress execution, raise Daimon arousal
  -> encode manifold features as HDC vector (via DeFiCodebook)
  -> emit as Engram to the witness pipeline
```

### Test criteria

- **Metric symmetry**: `g[i][j] == g[j][i]` for all i, j (within f64 epsilon).
- **Metric positive-definiteness**: All eigenvalues of a well-formed metric are positive.
- **Geodesic consistency**: `exp_p(log_p(q)) == q` within convergence tolerance.
- **Christoffel symmetry**: `Gamma[k][i][j] == Gamma[k][j][i]` (lower indices are symmetric).
- **Ricci scalar sign**: For a known constant-product AMM with deep liquidity, R > 0. For a pool at 99% depletion, R < 0.
- **Adaptive step refinement**: Halving error tolerance halves the integration error (4th-order convergence).
- **Singular point detection**: A pool with zero liquidity triggers `is_singular() == true`.

---

## Information Geometry and Statistical Manifold Extensions

The spectral liquidity manifold described above uses Riemannian geometry to model execution costs. This section extends the framework into **information geometry** -- the Riemannian geometry of probability distributions. When oracles produce probabilistic predictions, the space of those prediction distributions is itself a manifold with rich geometric structure. Information geometry provides coordinate-free tools for measuring distances between distributions, optimizing oracle parameters, and detecting distribution drift.

The key bridge: the liquidity manifold's metric tensor measures execution cost; the Fisher-Rao metric measures **statistical distinguishability**. Combining both gives a product manifold where geodesics jointly optimize execution cost and prediction accuracy.

### Fisher-Rao Metric on Oracle Distributions

```rust
/// Information-geometric extension of the liquidity manifold.
///
/// The Fisher-Rao metric is the UNIQUE Riemannian metric invariant
/// under sufficient statistics (Cencov's theorem, 1982).
///
/// When oracle predictions form a parametric family p(y|theta),
/// the Fisher information matrix defines a natural Riemannian metric:
///   G_ij(theta) = E[d(log p(y|theta))/d(theta_i) * d(log p(y|theta))/d(theta_j)]
///
/// This metric captures the "distinguishability" of nearby parameter values.
/// Two parameter values theta and theta + d(theta) are "far apart" if the
/// distributions p(y|theta) and p(y|theta + d(theta)) are easy to distinguish
/// from samples. The Fisher-Rao distance is the geodesic distance under
/// this metric.
///
/// For the oracle prediction pipeline, this means:
/// - Parameters that strongly affect predictions are "far" from each other.
/// - Parameters with redundant effects are "close" (the manifold collapses
///   in those directions, reflected by small eigenvalues of G).
/// - The volume element sqrt(det(G)) gives the Jeffreys prior --
///   the uninformative prior that is invariant under reparameterization.
///
/// (Amari & Nagaoka, 2000, "Methods of Information Geometry")
pub struct FisherRaoManifold {
    /// Dimension of the parameter space.
    pub dim: usize,
    /// Fisher information matrix at a point.
    /// Given parameter vector theta, returns the dim x dim Fisher matrix G(theta).
    pub fisher_matrix: Box<dyn Fn(&[f64]) -> Vec<Vec<f64>> + Send + Sync>,
    /// Alpha-connection parameter (alpha in [-1, 1]).
    /// alpha = 0: Levi-Civita connection (Riemannian, metric-compatible)
    /// alpha = 1: e-connection (exponential family natural parameters)
    /// alpha = -1: m-connection (mixture family natural parameters)
    /// Other values interpolate between these extremes.
    pub alpha: f64,
}

impl FisherRaoManifold {
    /// Compute the Fisher-Rao distance between two parameter values.
    ///
    /// This is the geodesic distance under the Fisher metric.
    /// For univariate Gaussians N(mu, sigma^2), the Fisher-Rao distance
    /// has a known closed form; for general families, we integrate
    /// numerically along the geodesic.
    pub fn distance(&self, theta_1: &[f64], theta_2: &[f64]) -> f64 {
        // Construct geodesic between theta_1 and theta_2 on the
        // Fisher-Rao manifold and integrate sqrt(g_ij dx^i dx^j).
        let geodesic = self.compute_geodesic(theta_1, theta_2, 200);
        let mut length = 0.0;
        for step in 0..geodesic.len() - 1 {
            let p = &geodesic[step];
            let q = &geodesic[step + 1];
            let g = (self.fisher_matrix)(p);
            let dp: Vec<f64> = q.iter().zip(p.iter()).map(|(a, b)| a - b).collect();
            let mut local_sq = 0.0;
            for i in 0..self.dim {
                for j in 0..self.dim {
                    local_sq += g[i][j] * dp[i] * dp[j];
                }
            }
            length += local_sq.max(0.0).sqrt();
        }
        length
    }

    /// Spectral decomposition of the Fisher matrix at a point.
    ///
    /// Eigenvalues reveal which parameter directions are most "informative":
    /// - Large eigenvalue: small changes in that direction produce distinguishable distributions.
    /// - Small eigenvalue: parameter is poorly identified (sloppy direction).
    ///
    /// This directly informs oracle parameter pruning:
    /// parameters along sloppy directions can be fixed without loss of predictive power.
    pub fn fisher_spectrum(&self, theta: &[f64]) -> (Vec<f64>, Vec<Vec<f64>>) {
        let g = (self.fisher_matrix)(theta);
        symmetric_eigendecomposition(&g)
    }
}
```

The Fisher-Rao manifold connects to the liquidity manifold through a product structure: the full state space is M_liq x M_stat, where M_liq carries the execution cost metric and M_stat carries the Fisher-Rao metric. Geodesics on the product manifold optimize both execution cost and parameter estimation simultaneously.

### Natural Gradient for Oracle Parameter Updates

```rust
/// Natural gradient descent on the oracle parameter manifold.
///
/// Standard gradient descent ignores the geometry of parameter space.
/// In Euclidean gradient descent, the update direction depends on how
/// the parameters are chosen (e.g., mu vs. mu^2). This is a fundamental
/// flaw: the optimizer's behavior changes under reparameterization.
///
/// Natural gradient (Amari, 1998) corrects for curvature:
///   theta_{t+1} = theta_t - eta * G(theta_t)^{-1} * grad(L(theta_t))
///
/// where G(theta) is the Fisher information matrix.
///
/// Key properties:
/// - Equivalent to steepest descent in the KL-divergence metric.
/// - Convergence: O(1/t) regardless of parameterization (coordinate-free).
/// - For exponential families: natural gradient in natural parameters
///   reduces to standard gradient in expectation parameters (and vice versa).
/// - Achieves the Cramer-Rao bound asymptotically: no other first-order
///   method converges faster in the information-geometric sense.
///
/// (Amari, 1998, "Natural Gradient Works Efficiently in Learning")
pub struct NaturalGradientOptimizer {
    pub learning_rate: f64,
    /// Tikhonov damping to regularize Fisher matrix inversion.
    /// G_reg = G + lambda * I (prevents singularity for flat directions).
    /// Interpretation: adds a small isotropic component to the metric,
    /// ensuring all directions have at least lambda curvature.
    /// Larger lambda -> more regularization -> closer to standard gradient.
    pub damping: f64,  // default: 1e-4
    /// Fisher matrix estimation method.
    pub fisher_method: FisherEstimation,
}

impl NaturalGradientOptimizer {
    /// Compute one natural gradient step.
    ///
    /// Returns the parameter update: delta_theta = -eta * G_reg^{-1} * grad_L.
    pub fn step(
        &self,
        theta: &[f64],
        gradient: &[f64],
        fisher_fn: &dyn Fn(&[f64]) -> Vec<Vec<f64>>,
    ) -> Vec<f64> {
        let dim = theta.len();
        let mut g = (fisher_fn)(theta);

        // Apply Tikhonov damping: G_reg = G + lambda * I
        for i in 0..dim {
            g[i][i] += self.damping;
        }

        // Solve G_reg * delta = gradient via Cholesky decomposition.
        // G_reg is symmetric positive-definite (given damping > 0),
        // so Cholesky is numerically stable and O(d^3 / 3).
        let natural_grad = cholesky_solve(&g, gradient);

        // Scale by learning rate
        natural_grad.iter().map(|&ng| -self.learning_rate * ng).collect()
    }
}

pub enum FisherEstimation {
    /// Exact: compute full Fisher matrix (O(d^2) per sample).
    /// Requires access to the log-likelihood's analytical Hessian.
    Exact,
    /// Empirical: Monte Carlo estimate from samples.
    /// G_hat = (1/N) sum_{n=1}^{N} grad(log p(y_n|theta)) * grad(log p(y_n|theta))^T
    /// Unbiased but high variance for small N.
    Empirical { n_samples: usize },
    /// Kronecker-factored (K-FAC, Martens & Grosse, 2015).
    /// Approximates G as Kronecker product of two smaller matrices: G ~= A (x) B.
    /// For neural network layers with d_in inputs and d_out outputs:
    ///   Full Fisher: d_in*d_out x d_in*d_out (huge)
    ///   K-FAC: d_in x d_in and d_out x d_out (manageable)
    /// Reduces inversion cost from O((d_in*d_out)^3) to O(d_in^3 + d_out^3).
    KroneckerFactored,
    /// Diagonal approximation (cheapest, O(d) per sample).
    /// Keeps only the diagonal of the Fisher matrix.
    /// Equivalent to adaptive learning rates (like Adam without momentum).
    /// Loses off-diagonal structure (parameter correlations).
    Diagonal,
}
```

**Configuration parameters**:

| Parameter | Default | Range | Notes |
|---|---|---|---|
| `learning_rate` | 0.01 | 1e-4 - 1.0 | Standard learning rate. Natural gradient is less sensitive to this than standard gradient. |
| `damping` | 1e-4 | 1e-8 - 1.0 | Too small: G may be singular. Too large: reverts to standard gradient. |
| `n_samples` (empirical) | 100 | 10 - 10000 | Variance of Fisher estimate scales as O(1/N). |

### Alpha-Connections and Dually Flat Structure

```rust
/// Alpha-connection family on the statistical manifold.
///
/// The alpha-connection nabla^(alpha) interpolates between the exponential (alpha=1)
/// and mixture (alpha=-1) connections:
///   nabla^(alpha) = (1+alpha)/2 * nabla^(e) + (1-alpha)/2 * nabla^(m)
///
/// When alpha = 0, this recovers the Levi-Civita (metric-compatible) connection.
///
/// For exponential families, the (1,-1)-connections make the manifold
/// DUALLY FLAT: both natural parameters theta and expectation parameters eta
/// yield flat coordinate systems. The canonical divergence is the
/// Bregman divergence (= KL divergence for exponential families).
///
/// Why this matters for oracle predictions:
/// - The natural parameter theta represents the "raw" oracle model parameters.
/// - The expectation parameter eta = E[T(y)] represents the sufficient statistics.
/// - The Legendre transform F(theta) <-> F*(eta) converts between them.
/// - The Bregman divergence D_F(p, q) = KL(p || q) measures prediction error
///   in a way that respects the manifold geometry.
///
/// (Amari, 2016, "Information Geometry and Its Applications", Springer)
pub struct DuallyFlatManifold {
    /// Natural (theta) coordinate system.
    /// For a Gaussian: theta = (mu/sigma^2, -1/(2*sigma^2)).
    pub theta_coords: Vec<f64>,
    /// Expectation (eta) coordinate system.
    /// For a Gaussian: eta = (mu, mu^2 + sigma^2).
    pub eta_coords: Vec<f64>,
    /// Legendre transform: F(theta) -> F*(eta) where eta = grad(F(theta)).
    /// F(theta) = log-partition function (log normalizer) for exponential families.
    pub potential: Box<dyn Fn(&[f64]) -> f64 + Send + Sync>,
    /// Conjugate potential: F*(eta) = theta . eta - F(theta).
    /// F*(eta) = negative entropy for exponential families.
    pub conjugate_potential: Box<dyn Fn(&[f64]) -> f64 + Send + Sync>,
}

impl DuallyFlatManifold {
    /// Bregman divergence (= canonical divergence on dually flat manifold).
    ///
    /// D_F(p, q) = F(theta_p) - F(theta_q) - grad(F(theta_q)) . (theta_p - theta_q)
    ///
    /// Properties:
    /// - D_F(p, q) >= 0 with equality iff p = q.
    /// - D_F(p, q) != D_F(q, p) in general (asymmetric).
    /// - For exponential families: D_F(p, q) = KL(q || p).
    ///   Note the argument swap: Bregman in theta coords = reverse KL.
    /// - The symmetrized Bregman (D_F(p,q) + D_F(q,p))/2 = (theta_p - theta_q) . (eta_p - eta_q).
    pub fn bregman_divergence(&self, p: &[f64], q: &[f64]) -> f64 {
        let f_p = (self.potential)(p);
        let f_q = (self.potential)(q);
        let grad_q = numerical_gradient(self.potential.as_ref(), q);
        f_p - f_q - dot(&grad_q, &subtract(p, q))
    }

    /// Convert natural parameters to expectation parameters.
    /// eta = grad(F(theta))
    pub fn theta_to_eta(&self, theta: &[f64]) -> Vec<f64> {
        numerical_gradient(self.potential.as_ref(), theta)
    }

    /// Convert expectation parameters to natural parameters.
    /// theta = grad(F*(eta))
    pub fn eta_to_theta(&self, eta: &[f64]) -> Vec<f64> {
        numerical_gradient(self.conjugate_potential.as_ref(), eta)
    }

    /// m-geodesic: straight line in eta (expectation) coordinates.
    ///
    /// The mixture geodesic is flat in eta space:
    ///   eta(t) = (1 - t) * eta_p + t * eta_q
    ///
    /// This corresponds to mixing distributions:
    ///   p_t = (1-t) * p + t * q  (in the mixture sense)
    pub fn m_geodesic(
        &self,
        eta_p: &[f64],
        eta_q: &[f64],
        n_steps: usize,
    ) -> Vec<Vec<f64>> {
        (0..=n_steps)
            .map(|step| {
                let t = step as f64 / n_steps as f64;
                eta_p.iter()
                    .zip(eta_q.iter())
                    .map(|(&a, &b)| (1.0 - t) * a + t * b)
                    .collect()
            })
            .collect()
    }

    /// e-geodesic: straight line in theta (natural) coordinates.
    ///
    /// The exponential geodesic is flat in theta space:
    ///   theta(t) = (1 - t) * theta_p + t * theta_q
    ///
    /// This corresponds to exponential tilting of distributions.
    pub fn e_geodesic(
        &self,
        theta_p: &[f64],
        theta_q: &[f64],
        n_steps: usize,
    ) -> Vec<Vec<f64>> {
        (0..=n_steps)
            .map(|step| {
                let t = step as f64 / n_steps as f64;
                theta_p.iter()
                    .zip(theta_q.iter())
                    .map(|(&a, &b)| (1.0 - t) * a + t * b)
                    .collect()
            })
            .collect()
    }
}
```

The dually flat structure provides two complementary views of the same oracle prediction manifold. The e-geodesic interpolates in log-likelihood space (exponential tilting), while the m-geodesic interpolates in moment space (mixture averaging). The Pythagorean theorem holds: for any triple (p, q, r) where the e-geodesic from p to r is orthogonal to the m-geodesic from q to r, we have D_F(p, q) = D_F(p, r) + D_F(r, q). This generalizes the Euclidean Pythagorean theorem to information geometry.

### Wasserstein Geometry for Distribution Transport

```rust
/// Wasserstein-2 geometry on the space of oracle prediction distributions.
///
/// The W_2 metric measures the cost of optimally transporting one distribution
/// to another. Unlike KL divergence, W_2 is a true metric (symmetric,
/// satisfies triangle inequality) and is defined even when distributions
/// have non-overlapping support.
///
/// For Gaussian predictions N(mu_1, Sigma_1) and N(mu_2, Sigma_2):
///
///   W_2^2 = ||mu_1 - mu_2||^2 + tr(Sigma_1 + Sigma_2 - 2*(Sigma_1^{1/2} Sigma_2 Sigma_1^{1/2})^{1/2})
///
/// The Bures metric (infinitesimal W_2 on Gaussians) defines a Riemannian
/// metric on the space of Gaussian distributions. This is NOT the Fisher-Rao
/// metric -- it captures different geometric structure:
/// - Fisher-Rao: statistical distinguishability (hypothesis testing).
/// - Wasserstein: physical transport cost (mass movement).
///
/// The W_2 metric captures distribution shift -- when oracle prediction
/// distributions change over time, the Wasserstein distance quantifies
/// how much the predictions have drifted. Unlike KL divergence, this
/// drift measure is bounded and interpretable as a physical distance.
///
/// (Villani, 2009, "Optimal Transport: Old and New", Springer)
pub struct WassersteinDistanceComputer {
    /// Distribution representation.
    pub representation: DistributionRepr,
}

pub enum DistributionRepr {
    /// Gaussian: track mean and covariance.
    /// W_2 has closed form for Gaussians (Bures distance).
    Gaussian { mean: Vec<f64>, cov: Vec<Vec<f64>> },
    /// Empirical: track samples.
    /// W_2 computed via linear programming (exact) or Sinkhorn (approximate).
    Empirical { samples: Vec<f64> },
    /// Histogram: discretized distribution.
    /// W_2 computed via the transportation simplex or Sinkhorn regularization.
    Histogram { bins: Vec<f64>, counts: Vec<f64> },
}

impl WassersteinDistanceComputer {
    /// Compute W_2 distance between two Gaussian distributions.
    ///
    /// Uses the closed-form Bures distance formula.
    /// Requires matrix square root computation (via Schur decomposition).
    pub fn gaussian_w2(
        mean_1: &[f64],
        cov_1: &[Vec<f64>],
        mean_2: &[f64],
        cov_2: &[Vec<f64>],
    ) -> f64 {
        let mean_diff_sq: f64 = mean_1.iter()
            .zip(mean_2.iter())
            .map(|(a, b)| (a - b).powi(2))
            .sum();

        // tr(Sigma_1 + Sigma_2 - 2 * (Sigma_1^{1/2} Sigma_2 Sigma_1^{1/2})^{1/2})
        let sqrt_1 = matrix_sqrt(cov_1);
        let product = matrix_multiply(&matrix_multiply(&sqrt_1, cov_2), &sqrt_1);
        let sqrt_product = matrix_sqrt(&product);
        let trace_term = trace(cov_1) + trace(cov_2) - 2.0 * trace(&sqrt_product);

        (mean_diff_sq + trace_term).max(0.0).sqrt()
    }

    /// Compute W_2 distance between empirical distributions via Sinkhorn.
    ///
    /// Sinkhorn divergence is an entropy-regularized approximation to W_2:
    ///   W_2^epsilon = min_P <P, C> + epsilon * KL(P || a (x) b)
    ///
    /// where C is the cost matrix, P is the transport plan, and epsilon
    /// controls regularization (epsilon -> 0 recovers exact W_2).
    ///
    /// Complexity: O(n^2 / epsilon^2) vs O(n^3 log n) for exact W_2.
    pub fn sinkhorn_w2(
        samples_1: &[f64],
        samples_2: &[f64],
        epsilon: f64,    // regularization, default: 0.01
        max_iter: usize, // default: 100
    ) -> f64 {
        let n = samples_1.len();
        let m = samples_2.len();

        // Cost matrix: C_ij = |x_i - y_j|^2
        let cost: Vec<Vec<f64>> = (0..n)
            .map(|i| (0..m).map(|j| (samples_1[i] - samples_2[j]).powi(2)).collect())
            .collect();

        // Gibbs kernel: K_ij = exp(-C_ij / epsilon)
        let kernel: Vec<Vec<f64>> = cost.iter()
            .map(|row| row.iter().map(|&c| (-c / epsilon).exp()).collect())
            .collect();

        // Sinkhorn iterations: alternating row/column normalization
        let mut u = vec![1.0 / n as f64; n];
        let mut v = vec![1.0 / m as f64; m];

        for _ in 0..max_iter {
            // u = a ./ (K * v)
            for i in 0..n {
                let kv: f64 = (0..m).map(|j| kernel[i][j] * v[j]).sum();
                u[i] = (1.0 / n as f64) / kv.max(1e-30);
            }
            // v = b ./ (K^T * u)
            for j in 0..m {
                let ku: f64 = (0..n).map(|i| kernel[i][j] * u[i]).sum();
                v[j] = (1.0 / m as f64) / ku.max(1e-30);
            }
        }

        // Transport cost: sum_ij u_i K_ij v_j C_ij
        let mut total = 0.0;
        for i in 0..n {
            for j in 0..m {
                total += u[i] * kernel[i][j] * v[j] * cost[i][j];
            }
        }
        total.max(0.0).sqrt()
    }
}
```

The Wasserstein distance connects to the liquidity manifold through **distribution drift detection**. When the chain oracle's prediction distribution shifts (measured by W_2), the manifold metric must be recomputed. The rate of Wasserstein drift serves as a signal for metric staleness: if W_2(p_t, p_{t-1}) exceeds a threshold, the cached metric tensor and Christoffel symbols are invalidated and recomputed from fresh pool states.

### Test criteria

- **Fisher-Rao positive definiteness**: For any exponential family, the Fisher matrix has all positive eigenvalues. Verify by constructing the Fisher matrix for a Gaussian family at multiple parameter values and checking that all eigenvalues exceed zero.
- **Natural gradient coordinate invariance**: The natural gradient produces the same update (in distribution space) regardless of parameterization. Verify by computing the natural gradient update in both natural and mean parameterizations of a Gaussian and confirming that the resulting distributions match within numerical tolerance.
- **Bregman divergence non-negativity**: D_F(p, q) >= 0 with equality iff p = q. Verify for the Gaussian log-partition potential with random parameter pairs.
- **Wasserstein triangle inequality**: W_2(p, r) <= W_2(p, q) + W_2(q, r). Verify for triples of Gaussian distributions using the closed-form Bures distance.

### Information geometry references

- Amari, S. (1998). "Natural Gradient Works Efficiently in Learning." *Neural Computation*, 10(2), 251-276.
- Amari, S. (2016). *Information Geometry and Its Applications*. Springer.
- Cencov, N. N. (1982). *Statistical Decision Rules and Optimal Inference*. AMS.
- Villani, C. (2009). *Optimal Transport: Old and New*. Springer.
- Martens, J., & Grosse, R. (2015). "Optimizing Neural Networks with Kronecker-Factored Approximate Curvature." *ICML 2015*.

---

## Academic foundations

- Amari, S., & Nagaoka, H. (2000). *Methods of Information Geometry*. AMS/Oxford. — Riemannian geometry for statistical manifolds.
- do Carmo, M. P. (1992). *Riemannian Geometry*. Birkhäuser. — Standard reference for geodesics, curvature, parallel transport.
- Pennec, X. (2006). "Intrinsic Statistics on Riemannian Manifolds." *Journal of Mathematical Imaging and Vision*, 25(1), 127-154. — Fréchet mean and Karcher iteration.
- Adams, R. P., & Stegle, O. (2012). "Gaussian Process Product Models." *ICML 2012*. — GP-based metric tensor estimation.
- Bronstein, M. M., et al. (2017). "Geometric Deep Learning." *IEEE Signal Processing Magazine*, 34(6), 18-42. — Geometric methods for learning on manifolds.

---

## Cross-References

- See [02-chain-oracles.md](./02-chain-oracles.md) for chain oracle integration
- See [06-hyperdimensional-ta.md](./06-hyperdimensional-ta.md) for HDC encoding of manifold features
- See [09-causal-microstructure-discovery.md](./09-causal-microstructure-discovery.md) for causal analysis of manifold dynamics
- See [10-predictive-geometry-and-resonant-patterns.md](./10-predictive-geometry-and-resonant-patterns.md) for TDA on manifold topology
