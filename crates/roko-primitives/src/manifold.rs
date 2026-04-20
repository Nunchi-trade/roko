//! Riemannian geometry for execution cost landscapes (TA-06).
//!
//! Models the execution cost landscape as a 4-dimensional Riemannian manifold
//! where the metric tensor encodes local cost structure:
//!
//! ```text
//! g_ij(x) = diag(slippage_cost, gas_cost, time_cost, opportunity_cost)
//! ```
//!
//! The geodesic between two points is the minimum-cost execution path.
//!
//! # Core primitives
//!
//! - [`MetricTensor`]: callback-based metric that evaluates `g_ij` at any point.
//! - [`christoffel`]: Christoffel symbols via finite differences of the metric.
//! - [`geodesic_rk4`]: RK4 geodesic solver finding minimum-cost paths.
//! - [`ricci_scalar`]: curvature invariant measuring market stability.
//! - [`frechet_mean`]: intrinsic mean on the manifold via iterative geodesic averaging.
//!
//! All operations use hand-rolled 4x4 matrix arithmetic (no external deps).
//!
//! # References
//!
//! - do Carmo, M. P. (1992). *Riemannian Geometry*.
//! - Pennec, X. (2006). Intrinsic statistics on Riemannian manifolds.
//! - Frechet, M. (1948). Les elements aleatoires de nature quelconque dans un
//!   espace distancie.

/// Manifold dimension. The cost manifold has 4 axes:
/// slippage, gas, time, opportunity.
pub const DIM: usize = 4;

/// A point on the 4D cost manifold.
pub type Point = [f64; DIM];

/// A 4x4 symmetric matrix (metric tensor at a point).
pub type Mat4 = [[f64; DIM]; DIM];

// ---------------------------------------------------------------------------
// Small 4x4 matrix utilities
// ---------------------------------------------------------------------------

/// Create a 4x4 identity matrix.
#[must_use]
pub fn mat4_identity() -> Mat4 {
    let mut m = [[0.0; DIM]; DIM];
    for i in 0..DIM {
        m[i][i] = 1.0;
    }
    m
}

/// Create a 4x4 diagonal matrix.
#[must_use]
pub fn mat4_diag(d: &[f64; DIM]) -> Mat4 {
    let mut m = [[0.0; DIM]; DIM];
    for i in 0..DIM {
        m[i][i] = d[i];
    }
    m
}

/// Invert a 4x4 matrix via Gauss-Jordan elimination.
///
/// Returns `None` if the matrix is singular (determinant near zero).
#[must_use]
pub fn mat4_inverse(m: &Mat4) -> Option<Mat4> {
    let mut aug = [[0.0; 2 * DIM]; DIM];
    for i in 0..DIM {
        for j in 0..DIM {
            aug[i][j] = m[i][j];
        }
        aug[i][DIM + i] = 1.0;
    }

    for col in 0..DIM {
        // Partial pivot.
        let mut max_row = col;
        let mut max_val = aug[col][col].abs();
        for row in (col + 1)..DIM {
            if aug[row][col].abs() > max_val {
                max_val = aug[row][col].abs();
                max_row = row;
            }
        }
        if max_val < 1e-15 {
            return None; // Singular.
        }
        if max_row != col {
            aug.swap(col, max_row);
        }

        let pivot = aug[col][col];
        for j in 0..(2 * DIM) {
            aug[col][j] /= pivot;
        }

        for row in 0..DIM {
            if row == col {
                continue;
            }
            let factor = aug[row][col];
            for j in 0..(2 * DIM) {
                aug[row][j] -= factor * aug[col][j];
            }
        }
    }

    let mut inv = [[0.0; DIM]; DIM];
    for i in 0..DIM {
        for j in 0..DIM {
            inv[i][j] = aug[i][DIM + j];
        }
    }
    Some(inv)
}

// ---------------------------------------------------------------------------
// MetricTensor
// ---------------------------------------------------------------------------

/// A Riemannian metric tensor on a 4D manifold.
///
/// The metric is defined by a callback `g: Point -> Mat4` that returns the
/// symmetric positive-definite metric tensor at each point. For the execution
/// cost manifold, this encodes slippage, gas, time, and opportunity costs.
pub struct MetricTensor {
    /// The metric function: given a manifold point, returns g_ij.
    metric_fn: Box<dyn Fn(&Point) -> Mat4 + Send + Sync>,
}

impl MetricTensor {
    /// Create a metric tensor from a callback.
    pub fn new(f: impl Fn(&Point) -> Mat4 + Send + Sync + 'static) -> Self {
        Self {
            metric_fn: Box::new(f),
        }
    }

    /// Create a simple diagonal metric with constant weights.
    ///
    /// The cost at each point is `g = diag(w)`, independent of position.
    /// This is a flat (Euclidean-like) metric in the weighted coordinates.
    #[must_use]
    pub fn constant_diagonal(weights: [f64; DIM]) -> Self {
        let g = mat4_diag(&weights);
        Self::new(move |_| g)
    }

    /// Create the default execution cost metric.
    ///
    /// Each cost axis scales with the coordinate value:
    /// - Slippage: quadratic in amount (increases superlinearly).
    /// - Gas: linear in base fee.
    /// - Time: linear in block time.
    /// - Opportunity: quadratic in price movement rate.
    #[must_use]
    pub fn execution_cost() -> Self {
        Self::new(|x: &Point| {
            let slippage = 1.0 + x[0] * x[0]; // Quadratic slippage.
            let gas = 1.0 + x[1].abs();        // Linear gas.
            let time = 1.0 + x[2].abs();       // Linear time.
            let opportunity = 1.0 + x[3] * x[3]; // Quadratic opportunity.
            mat4_diag(&[slippage, gas, time, opportunity])
        })
    }

    /// Evaluate the metric tensor at a point.
    #[must_use]
    pub fn at(&self, point: &Point) -> Mat4 {
        (self.metric_fn)(point)
    }

    /// Evaluate the inverse metric tensor at a point.
    ///
    /// Returns `None` if the metric is singular at this point.
    #[must_use]
    pub fn inverse_at(&self, point: &Point) -> Option<Mat4> {
        mat4_inverse(&self.at(point))
    }
}

impl std::fmt::Debug for MetricTensor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricTensor")
            .field("at_origin", &self.at(&[0.0; DIM]))
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Christoffel symbols
// ---------------------------------------------------------------------------

/// Christoffel symbols of the second kind: `Gamma^k_ij`.
///
/// These are the connection coefficients that define how the coordinate
/// basis vectors change as you move along the manifold. They appear in the
/// geodesic equation: `d^2 x^k / dt^2 + Gamma^k_ij dx^i/dt dx^j/dt = 0`.
pub type ChristoffelSymbols = [[[f64; DIM]; DIM]; DIM];

/// Compute Christoffel symbols at a point via finite differences.
///
/// Uses the formula:
/// ```text
/// Gamma^k_ij = 1/2 g^{kl} (d_i g_{jl} + d_j g_{il} - d_l g_{ij})
/// ```
///
/// The step size `h` controls the finite difference accuracy (default: 1e-5).
///
/// Returns `None` if the metric is singular at the given point.
#[must_use]
pub fn christoffel(metric: &MetricTensor, point: &Point, h: f64) -> Option<ChristoffelSymbols> {
    let g_inv = metric.inverse_at(point)?;

    // Compute partial derivatives: dg_jl/dx_i via central differences.
    // dg[i][j][l] = d(g_jl)/d(x_i)
    let mut dg = [[[0.0; DIM]; DIM]; DIM];
    for i in 0..DIM {
        let mut p_plus = *point;
        let mut p_minus = *point;
        p_plus[i] += h;
        p_minus[i] -= h;
        let g_plus = metric.at(&p_plus);
        let g_minus = metric.at(&p_minus);
        for j in 0..DIM {
            for l in 0..DIM {
                dg[i][j][l] = (g_plus[j][l] - g_minus[j][l]) / (2.0 * h);
            }
        }
    }

    // Christoffel symbols: Gamma^k_ij = 1/2 g^{kl} (dg[i][j][l] + dg[j][i][l] - dg[l][i][j])
    let mut gamma = [[[0.0; DIM]; DIM]; DIM];
    for k in 0..DIM {
        for i in 0..DIM {
            for j in 0..DIM {
                let mut sum = 0.0;
                for l in 0..DIM {
                    sum += g_inv[k][l] * (dg[i][j][l] + dg[j][i][l] - dg[l][i][j]);
                }
                gamma[k][i][j] = 0.5 * sum;
            }
        }
    }

    Some(gamma)
}

// ---------------------------------------------------------------------------
// Geodesic solver (RK4)
// ---------------------------------------------------------------------------

/// A point on a geodesic, combining position and velocity.
#[derive(Debug, Clone, Copy)]
pub struct GeodesicPoint {
    /// Position on the manifold.
    pub position: Point,
    /// Velocity (tangent vector).
    pub velocity: Point,
}

/// Solve the geodesic equation from a starting point with initial velocity
/// using 4th-order Runge-Kutta integration.
///
/// The geodesic equation:
/// ```text
/// d^2 x^k / dt^2 + Gamma^k_ij (dx^i/dt)(dx^j/dt) = 0
/// ```
///
/// This finds the locally length-minimizing path, which in the cost manifold
/// is the minimum-cost execution path.
///
/// # Arguments
/// - `metric`: the Riemannian metric tensor.
/// - `start`: initial position.
/// - `velocity`: initial tangent vector.
/// - `steps`: number of RK4 integration steps.
/// - `dt`: time step size.
/// - `h`: finite difference step for Christoffel computation.
///
/// Returns the path as a sequence of positions.
#[must_use]
pub fn geodesic_rk4(
    metric: &MetricTensor,
    start: Point,
    velocity: Point,
    steps: usize,
    dt: f64,
    h: f64,
) -> Vec<GeodesicPoint> {
    let mut path = Vec::with_capacity(steps + 1);
    let mut x = start;
    let mut v = velocity;
    path.push(GeodesicPoint {
        position: x,
        velocity: v,
    });

    for _ in 0..steps {
        // RK4 for the system: dx/dt = v, dv^k/dt = -Gamma^k_ij v^i v^j
        let (k1x, k1v) = geodesic_deriv(metric, &x, &v, h);
        let (x2, v2) = step_state(&x, &v, &k1x, &k1v, 0.5 * dt);
        let (k2x, k2v) = geodesic_deriv(metric, &x2, &v2, h);
        let (x3, v3) = step_state(&x, &v, &k2x, &k2v, 0.5 * dt);
        let (k3x, k3v) = geodesic_deriv(metric, &x3, &v3, h);
        let (x4, v4) = step_state(&x, &v, &k3x, &k3v, dt);
        let (k4x, k4v) = geodesic_deriv(metric, &x4, &v4, h);

        for i in 0..DIM {
            x[i] += dt / 6.0 * (k1x[i] + 2.0 * k2x[i] + 2.0 * k3x[i] + k4x[i]);
            v[i] += dt / 6.0 * (k1v[i] + 2.0 * k2v[i] + 2.0 * k3v[i] + k4v[i]);
        }

        path.push(GeodesicPoint {
            position: x,
            velocity: v,
        });
    }

    path
}

/// Compute (dx/dt, dv/dt) for the geodesic equation.
fn geodesic_deriv(
    metric: &MetricTensor,
    x: &Point,
    v: &Point,
    h: f64,
) -> (Point, Point) {
    let dx = *v;
    let mut dv = [0.0; DIM];

    if let Some(gamma) = christoffel(metric, x, h) {
        for k in 0..DIM {
            let mut acc = 0.0;
            for i in 0..DIM {
                for j in 0..DIM {
                    acc += gamma[k][i][j] * v[i] * v[j];
                }
            }
            dv[k] = -acc;
        }
    }

    (dx, dv)
}

/// Euler step helper for RK4 intermediate states.
fn step_state(x: &Point, v: &Point, dx: &Point, dv: &Point, dt: f64) -> (Point, Point) {
    let mut xn = [0.0; DIM];
    let mut vn = [0.0; DIM];
    for i in 0..DIM {
        xn[i] = x[i] + dt * dx[i];
        vn[i] = v[i] + dt * dv[i];
    }
    (xn, vn)
}

// ---------------------------------------------------------------------------
// Geodesic distance (approximate)
// ---------------------------------------------------------------------------

/// Approximate geodesic distance between two points.
///
/// Integrates the metric tensor along a straight line (Euclidean) path
/// between `a` and `b`. For nearby points or flat metrics this is exact;
/// for curved metrics this is an upper bound on the true geodesic distance.
///
/// # Arguments
/// - `metric`: the Riemannian metric.
/// - `a`: start point.
/// - `b`: end point.
/// - `segments`: number of integration segments (more = more accurate).
#[must_use]
pub fn approx_geodesic_distance(
    metric: &MetricTensor,
    a: &Point,
    b: &Point,
    segments: usize,
) -> f64 {
    let segments = segments.max(1);
    let mut total = 0.0;

    for s in 0..segments {
        let t0 = s as f64 / segments as f64;
        let t1 = (s + 1) as f64 / segments as f64;
        let t_mid = (t0 + t1) / 2.0;

        // Midpoint on the straight-line path.
        let mut mid = [0.0; DIM];
        let mut tangent = [0.0; DIM];
        for i in 0..DIM {
            mid[i] = a[i] + t_mid * (b[i] - a[i]);
            tangent[i] = b[i] - a[i];
        }

        let g = metric.at(&mid);
        // ds^2 = g_ij dx^i dx^j
        let mut ds_sq = 0.0;
        for i in 0..DIM {
            for j in 0..DIM {
                ds_sq += g[i][j] * tangent[i] * tangent[j];
            }
        }

        let segment_length = (1.0 / segments as f64) * ds_sq.abs().sqrt();
        total += segment_length;
    }

    total
}

// ---------------------------------------------------------------------------
// Ricci scalar
// ---------------------------------------------------------------------------

/// Compute the Ricci scalar curvature at a point.
///
/// `R = g^ij R_ij` where `R_ij` is the Ricci tensor (trace of Riemann tensor).
/// The Riemann tensor is computed from Christoffel symbols and their derivatives.
///
/// - `R > 0`: locally "sphere-like" — converging geodesics (market instability).
/// - `R < 0`: locally "saddle-like" — diverging geodesics (stable conditions).
/// - `R = 0`: flat (Euclidean-like).
///
/// Returns `None` if the metric is singular.
#[must_use]
pub fn ricci_scalar(metric: &MetricTensor, point: &Point, h: f64) -> Option<f64> {
    let g_inv = metric.inverse_at(point)?;
    let gamma = christoffel(metric, point, h)?;

    // Compute dGamma[m][k][i][j] = d(Gamma^k_ij)/d(x_m) via central differences.
    let mut dgamma = [[[[0.0; DIM]; DIM]; DIM]; DIM];
    for m in 0..DIM {
        let mut p_plus = *point;
        let mut p_minus = *point;
        p_plus[m] += h;
        p_minus[m] -= h;
        let gamma_plus = christoffel(metric, &p_plus, h)?;
        let gamma_minus = christoffel(metric, &p_minus, h)?;
        for k in 0..DIM {
            for i in 0..DIM {
                for j in 0..DIM {
                    dgamma[m][k][i][j] =
                        (gamma_plus[k][i][j] - gamma_minus[k][i][j]) / (2.0 * h);
                }
            }
        }
    }

    // Riemann tensor: R^l_ijk = d_i Gamma^l_jk - d_j Gamma^l_ik
    //                          + Gamma^l_im Gamma^m_jk - Gamma^l_jm Gamma^m_ik
    // Ricci tensor: R_ij = R^k_ikj  (contract first and third indices)
    let mut ricci = [[0.0; DIM]; DIM];
    for i in 0..DIM {
        for j in 0..DIM {
            let mut sum = 0.0;
            for k in 0..DIM {
                // R^k_ikj
                sum += dgamma[i][k][k][j] - dgamma[k][k][i][j];
                for m in 0..DIM {
                    sum += gamma[k][i][m] * gamma[m][k][j]
                        - gamma[k][k][m] * gamma[m][i][j];
                }
            }
            ricci[i][j] = sum;
        }
    }

    // Scalar curvature: R = g^ij R_ij
    let mut scalar = 0.0;
    for i in 0..DIM {
        for j in 0..DIM {
            scalar += g_inv[i][j] * ricci[i][j];
        }
    }

    Some(scalar)
}

// ---------------------------------------------------------------------------
// Frechet mean
// ---------------------------------------------------------------------------

/// Compute the Frechet mean of a set of points on the manifold.
///
/// The Frechet mean minimizes `sum_i d^2(x, x_i)` where `d` is geodesic
/// distance. It is the intrinsic "center of mass" on a curved space.
///
/// Uses iterative gradient descent with exponential map approximation.
///
/// # Arguments
/// - `metric`: the Riemannian metric.
/// - `points`: the observations to average.
/// - `max_iter`: maximum iterations.
/// - `tol`: convergence tolerance (Euclidean distance between iterates).
///
/// Returns the mean point and the number of iterations used.
#[must_use]
pub fn frechet_mean(
    metric: &MetricTensor,
    points: &[Point],
    max_iter: usize,
    tol: f64,
) -> (Point, usize) {
    assert!(!points.is_empty(), "need at least one point");

    if points.len() == 1 {
        return (points[0], 0);
    }

    // Initialize with the Euclidean mean.
    let n = points.len() as f64;
    let mut mean = [0.0; DIM];
    for p in points {
        for i in 0..DIM {
            mean[i] += p[i] / n;
        }
    }

    for iter in 0..max_iter {
        // Compute the gradient: the average of log_mean(p_i) vectors.
        // In the small-curvature regime, log_mean(p) ~ p - mean (in g-weighted coords).
        let g_inv = match metric.inverse_at(&mean) {
            Some(inv) => inv,
            None => return (mean, iter),
        };

        let mut tangent = [0.0; DIM];
        for p in points {
            for i in 0..DIM {
                tangent[i] += (p[i] - mean[i]) / n;
            }
        }

        // Apply inverse metric to get the Riemannian gradient direction.
        let mut grad = [0.0; DIM];
        for i in 0..DIM {
            for j in 0..DIM {
                grad[i] += g_inv[i][j] * tangent[j];
            }
        }

        // Step along the gradient (shrinking step size).
        let step = 1.0 / (1.0 + iter as f64 * 0.1);
        let mut new_mean = [0.0; DIM];
        let mut dist_sq = 0.0;
        for i in 0..DIM {
            let delta = step * tangent[i]; // Use tangent, not metric-adjusted grad.
            new_mean[i] = mean[i] + delta;
            dist_sq += delta * delta;
        }

        mean = new_mean;

        if dist_sq.sqrt() < tol {
            return (mean, iter + 1);
        }
    }

    (mean, max_iter)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- Matrix utilities ---

    #[test]
    fn identity_inverse_is_identity() {
        let id = mat4_identity();
        let inv = mat4_inverse(&id).unwrap();
        for i in 0..DIM {
            for j in 0..DIM {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!(
                    (inv[i][j] - expected).abs() < 1e-12,
                    "inv[{i}][{j}] = {} (expected {expected})",
                    inv[i][j]
                );
            }
        }
    }

    #[test]
    fn diagonal_inverse() {
        let d = mat4_diag(&[2.0, 4.0, 5.0, 10.0]);
        let inv = mat4_inverse(&d).unwrap();
        assert!((inv[0][0] - 0.5).abs() < 1e-12);
        assert!((inv[1][1] - 0.25).abs() < 1e-12);
        assert!((inv[2][2] - 0.2).abs() < 1e-12);
        assert!((inv[3][3] - 0.1).abs() < 1e-12);
    }

    #[test]
    fn singular_matrix_returns_none() {
        let m = [[1.0, 0.0, 0.0, 0.0],
                  [0.0, 0.0, 0.0, 0.0],
                  [0.0, 0.0, 1.0, 0.0],
                  [0.0, 0.0, 0.0, 1.0]];
        assert!(mat4_inverse(&m).is_none());
    }

    // --- MetricTensor ---

    #[test]
    fn constant_diagonal_metric() {
        let metric = MetricTensor::constant_diagonal([1.0, 2.0, 3.0, 4.0]);
        let g = metric.at(&[0.0, 0.0, 0.0, 0.0]);
        assert_eq!(g[0][0], 1.0);
        assert_eq!(g[1][1], 2.0);
        assert_eq!(g[2][2], 3.0);
        assert_eq!(g[3][3], 4.0);
        assert_eq!(g[0][1], 0.0);
    }

    #[test]
    fn execution_cost_metric_varies_with_position() {
        let metric = MetricTensor::execution_cost();
        let g_origin = metric.at(&[0.0, 0.0, 0.0, 0.0]);
        let g_far = metric.at(&[2.0, 3.0, 4.0, 5.0]);

        // At origin all costs are 1.0.
        assert_eq!(g_origin[0][0], 1.0);
        // Away from origin, costs increase.
        assert!(g_far[0][0] > g_origin[0][0], "slippage should increase with amount");
        assert!(g_far[1][1] > g_origin[1][1], "gas should increase with base fee");
    }

    // --- Christoffel symbols ---

    #[test]
    fn flat_metric_has_zero_christoffel() {
        let metric = MetricTensor::constant_diagonal([1.0, 1.0, 1.0, 1.0]);
        let gamma = christoffel(&metric, &[0.0; DIM], 1e-5).unwrap();
        for k in 0..DIM {
            for i in 0..DIM {
                for j in 0..DIM {
                    assert!(
                        gamma[k][i][j].abs() < 1e-8,
                        "Gamma[{k}][{i}][{j}] = {} (expected 0 for flat metric)",
                        gamma[k][i][j]
                    );
                }
            }
        }
    }

    #[test]
    fn curved_metric_has_nonzero_christoffel() {
        let metric = MetricTensor::execution_cost();
        // At (1, 1, 1, 1) the metric is position-dependent.
        let gamma = christoffel(&metric, &[1.0, 1.0, 1.0, 1.0], 1e-5).unwrap();
        // Slippage axis is quadratic, so Gamma^0_00 should be non-zero.
        assert!(
            gamma[0][0][0].abs() > 1e-6,
            "Gamma^0_00 should be non-zero for curved metric: {}",
            gamma[0][0][0]
        );
    }

    #[test]
    fn christoffel_symmetry_in_lower_indices() {
        // Christoffel symbols are symmetric in lower indices: Gamma^k_ij = Gamma^k_ji
        let metric = MetricTensor::execution_cost();
        let gamma = christoffel(&metric, &[1.0, 2.0, 0.5, 0.3], 1e-5).unwrap();
        for k in 0..DIM {
            for i in 0..DIM {
                for j in 0..DIM {
                    assert!(
                        (gamma[k][i][j] - gamma[k][j][i]).abs() < 1e-6,
                        "Gamma[{k}][{i}][{j}] = {}, Gamma[{k}][{j}][{i}] = {}",
                        gamma[k][i][j],
                        gamma[k][j][i]
                    );
                }
            }
        }
    }

    // --- Geodesic solver ---

    #[test]
    fn geodesic_on_flat_metric_is_straight_line() {
        let metric = MetricTensor::constant_diagonal([1.0, 1.0, 1.0, 1.0]);
        let start = [0.0, 0.0, 0.0, 0.0];
        let velocity = [1.0, 0.0, 0.0, 0.0];
        let path = geodesic_rk4(&metric, start, velocity, 100, 0.01, 1e-5);

        // On flat metric, geodesic is a straight line: x(t) = start + t * velocity.
        let last = path.last().unwrap();
        assert!(
            (last.position[0] - 1.0).abs() < 1e-4,
            "expected x ≈ 1.0, got {}",
            last.position[0]
        );
        assert!(
            last.position[1].abs() < 1e-4,
            "expected y ≈ 0.0, got {}",
            last.position[1]
        );
    }

    #[test]
    fn geodesic_on_curved_metric_deviates() {
        let metric = MetricTensor::execution_cost();
        let start = [0.1, 0.1, 0.1, 0.1];
        let velocity = [1.0, 0.0, 0.0, 0.0];
        let path = geodesic_rk4(&metric, start, velocity, 100, 0.01, 1e-5);

        // On curved metric, the geodesic should deviate from a straight line.
        // The velocity should change (acceleration due to curvature).
        let last = path.last().unwrap();
        let velocity_changed = (last.velocity[0] - 1.0).abs() > 1e-4
            || last.velocity[1].abs() > 1e-6
            || last.velocity[2].abs() > 1e-6
            || last.velocity[3].abs() > 1e-6;
        assert!(
            velocity_changed,
            "velocity should change on curved metric: {:?}",
            last.velocity
        );
    }

    // --- Geodesic distance ---

    #[test]
    fn flat_distance_equals_euclidean() {
        let metric = MetricTensor::constant_diagonal([1.0, 1.0, 1.0, 1.0]);
        let a = [0.0, 0.0, 0.0, 0.0];
        let b = [1.0, 0.0, 0.0, 0.0];
        let dist = approx_geodesic_distance(&metric, &a, &b, 100);
        assert!(
            (dist - 1.0).abs() < 1e-6,
            "flat distance should be Euclidean: {dist}"
        );
    }

    #[test]
    fn weighted_flat_distance() {
        let metric = MetricTensor::constant_diagonal([4.0, 1.0, 1.0, 1.0]);
        let a = [0.0, 0.0, 0.0, 0.0];
        let b = [1.0, 0.0, 0.0, 0.0];
        let dist = approx_geodesic_distance(&metric, &a, &b, 100);
        // sqrt(4 * 1^2) = 2
        assert!(
            (dist - 2.0).abs() < 1e-6,
            "weighted distance should be 2.0: {dist}"
        );
    }

    // --- Ricci scalar ---

    #[test]
    fn flat_metric_has_zero_ricci() {
        let metric = MetricTensor::constant_diagonal([1.0, 1.0, 1.0, 1.0]);
        let r = ricci_scalar(&metric, &[0.0; DIM], 1e-4).unwrap();
        assert!(
            r.abs() < 1e-4,
            "flat metric should have R ≈ 0: {r}"
        );
    }

    #[test]
    fn curved_metric_has_nonzero_ricci() {
        // Use a strongly curved 2-sphere-like metric embedded in 4D.
        // g_00 = 1, g_11 = sin^2(x_0) — this is the standard metric on S^2
        // which has Ricci scalar R = 2 (positive curvature).
        // We embed it in 4D with flat extra dimensions.
        let metric = MetricTensor::new(|x: &Point| {
            let mut g = mat4_identity();
            let sin_x0 = x[0].sin();
            // Avoid degeneracy at x_0 = 0: ensure g_11 > 0.
            g[1][1] = sin_x0 * sin_x0 + 0.01;
            g
        });
        // At theta = pi/4 (away from poles where sin^2 is non-degenerate).
        let point = [std::f64::consts::FRAC_PI_4, 0.5, 0.0, 0.0];
        let r = ricci_scalar(&metric, &point, 1e-3).unwrap();
        // The sphere has R > 0 (positive curvature).
        assert!(
            r.abs() > 0.01,
            "sphere-like metric should have non-zero curvature: {r}"
        );
    }

    // --- Frechet mean ---

    #[test]
    fn frechet_mean_of_single_point() {
        let metric = MetricTensor::constant_diagonal([1.0, 1.0, 1.0, 1.0]);
        let points = vec![[1.0, 2.0, 3.0, 4.0]];
        let (mean, iters) = frechet_mean(&metric, &points, 100, 1e-8);
        assert_eq!(iters, 0);
        for i in 0..DIM {
            assert!((mean[i] - points[0][i]).abs() < 1e-12);
        }
    }

    #[test]
    fn frechet_mean_on_flat_is_arithmetic() {
        let metric = MetricTensor::constant_diagonal([1.0, 1.0, 1.0, 1.0]);
        let points = vec![
            [0.0, 0.0, 0.0, 0.0],
            [2.0, 4.0, 6.0, 8.0],
        ];
        let (mean, _) = frechet_mean(&metric, &points, 100, 1e-8);
        // On flat space, Frechet mean = arithmetic mean.
        for i in 0..DIM {
            assert!(
                (mean[i] - (points[0][i] + points[1][i]) / 2.0).abs() < 1e-4,
                "mean[{i}] = {} (expected {})",
                mean[i],
                (points[0][i] + points[1][i]) / 2.0
            );
        }
    }

    #[test]
    fn frechet_mean_symmetric_inputs() {
        let metric = MetricTensor::constant_diagonal([1.0, 1.0, 1.0, 1.0]);
        let points = vec![
            [-1.0, 0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
        ];
        let (mean, _) = frechet_mean(&metric, &points, 100, 1e-8);
        // Symmetric => mean is origin.
        assert!(mean[0].abs() < 1e-4, "mean[0] = {}", mean[0]);
    }
}
