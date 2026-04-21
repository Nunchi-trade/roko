//! Tropical (max-plus) algebra for decision geometry (TA-14).
//!
//! The **tropical semiring** replaces standard arithmetic with:
//! - Addition: `a (+) b = max(a, b)`
//! - Multiplication: `a (*) b = a + b`
//! - Additive identity (zero): `-infinity`
//! - Multiplicative identity (one): `0`
//!
//! This turns piecewise-linear functions (ReLU networks, decision trees,
//! attention mechanisms) into polynomial operations in tropical algebra.
//!
//! # Core types
//!
//! - [`TropicalF64`]: newtype with max-plus operator overloading.
//! - [`TropicalPolynomial`]: `max_i(c_i + a_i . x)` — a max over affine functions.
//! - [`TropicalMatrix`]: matrix operations in the tropical semiring.
//! - [`tropical_attention`]: attention via `max_j(Q_i . K_j + V_j)`.
//!
//! # References
//!
//! - Maclagan, D. & Sturmfels, B. (2015). *Introduction to Tropical Geometry*.
//! - Zhang, M. et al. (2018). Tropical geometry of deep neural networks.
//! - Alfarra, M. et al. (2024). Decision boundaries in tropical geometry.

use std::fmt;
use std::ops::{Add, Mul};

// ---------------------------------------------------------------------------
// TropicalF64
// ---------------------------------------------------------------------------

/// A scalar in the tropical (max-plus) semiring.
///
/// - `a + b` computes `max(a, b)` (tropical addition).
/// - `a * b` computes `a + b` (tropical multiplication, i.e. standard addition).
/// - The additive identity (tropical zero) is `NEG_INFINITY`.
/// - The multiplicative identity (tropical one) is `0.0`.
#[derive(Clone, Copy, PartialEq, PartialOrd)]
pub struct TropicalF64(pub f64);

impl TropicalF64 {
    /// Tropical additive identity: `max(x, -inf) = x` for all x.
    pub const ZERO: Self = Self(f64::NEG_INFINITY);

    /// Tropical multiplicative identity: `x + 0 = x` for all x.
    pub const ONE: Self = Self(0.0);

    /// Create a new tropical scalar from a standard float.
    #[must_use]
    pub const fn new(value: f64) -> Self {
        Self(value)
    }

    /// Extract the inner f64 value.
    #[must_use]
    pub const fn value(self) -> f64 {
        self.0
    }

    /// Whether this value is the tropical zero (-infinity).
    #[must_use]
    pub fn is_zero(self) -> bool {
        self.0 == f64::NEG_INFINITY
    }

    /// Tropical exponentiation: `a^n = n * a` (standard multiplication).
    ///
    /// Since tropical multiplication is standard addition, repeated tropical
    /// multiplication n times equals standard multiplication by n.
    #[must_use]
    pub fn trop_pow(self, n: i32) -> Self {
        if self.is_zero() && n > 0 {
            return Self::ZERO;
        }
        Self(self.0 * f64::from(n))
    }
}

/// Tropical addition: `max(a, b)`.
impl Add for TropicalF64 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self(self.0.max(rhs.0))
    }
}

/// Tropical multiplication: `a + b` (standard addition).
impl Mul for TropicalF64 {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        // -inf + anything = -inf in standard arithmetic.
        if self.is_zero() || rhs.is_zero() {
            return Self::ZERO;
        }
        Self(self.0 + rhs.0)
    }
}

impl fmt::Debug for TropicalF64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            write!(f, "T(-inf)")
        } else {
            write!(f, "T({:.4})", self.0)
        }
    }
}

impl fmt::Display for TropicalF64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            write!(f, "-inf")
        } else {
            write!(f, "{:.4}", self.0)
        }
    }
}

// ---------------------------------------------------------------------------
// TropicalPolynomial
// ---------------------------------------------------------------------------

/// A single monomial term: coefficient and exponent vector.
///
/// Represents `c (*) x_1^{a_1} (*) x_2^{a_2} (*) ...`
/// which in standard arithmetic is `c + a_1 * x_1 + a_2 * x_2 + ...`.
#[derive(Debug, Clone, PartialEq)]
pub struct TropicalTerm {
    /// Tropical coefficient (standard constant offset).
    pub coefficient: TropicalF64,
    /// Exponent vector (one entry per variable).
    pub exponents: Vec<i32>,
}

/// A tropical polynomial: `max_i(c_i + a_i . x)`.
///
/// Evaluation computes `max` over all terms, where each term is
/// `coefficient + sum(exponent_j * x_j)` in standard arithmetic.
/// This is exactly a max over affine functions — the same structure
/// as ReLU neural network layers and decision boundaries.
#[derive(Debug, Clone, PartialEq)]
pub struct TropicalPolynomial {
    /// The terms (monomials) of the polynomial.
    pub terms: Vec<TropicalTerm>,
}

impl TropicalPolynomial {
    /// Create a new tropical polynomial from a list of terms.
    #[must_use]
    pub fn new(terms: Vec<TropicalTerm>) -> Self {
        Self { terms }
    }

    /// Create from parallel arrays of coefficients and exponent vectors.
    ///
    /// Each `(coefficient, exponents)` pair becomes one term.
    #[must_use]
    pub fn from_affine(pairs: &[(f64, Vec<i32>)]) -> Self {
        Self {
            terms: pairs
                .iter()
                .map(|(c, e)| TropicalTerm {
                    coefficient: TropicalF64(*c),
                    exponents: e.clone(),
                })
                .collect(),
        }
    }

    /// Evaluate the polynomial at a point.
    ///
    /// Returns `max_i(c_i + sum_j(a_ij * x_j))`.
    /// The point must have the same dimensionality as the exponent vectors.
    #[must_use]
    pub fn evaluate(&self, point: &[TropicalF64]) -> TropicalF64 {
        let mut result = TropicalF64::ZERO;
        for term in &self.terms {
            let mut term_value = term.coefficient;
            for (exp, x) in term.exponents.iter().zip(point.iter()) {
                // Tropical x^exp = exp * x.value in standard arithmetic.
                term_value = term_value * x.trop_pow(*exp);
            }
            // Tropical addition = max.
            result = result + term_value;
        }
        result
    }

    /// Number of terms (monomials).
    #[must_use]
    pub fn num_terms(&self) -> usize {
        self.terms.len()
    }

    /// The index of the maximizing term at a given point.
    ///
    /// Returns `None` if the polynomial is empty. This identifies which
    /// "linear piece" of the piecewise-linear function is active.
    #[must_use]
    pub fn active_term(&self, point: &[TropicalF64]) -> Option<usize> {
        let mut best_idx = None;
        let mut best_val = f64::NEG_INFINITY;
        for (i, term) in self.terms.iter().enumerate() {
            let mut val = term.coefficient.0;
            for (exp, x) in term.exponents.iter().zip(point.iter()) {
                if x.is_zero() && *exp > 0 {
                    val = f64::NEG_INFINITY;
                    break;
                }
                val += (*exp as f64) * x.0;
            }
            if val > best_val {
                best_val = val;
                best_idx = Some(i);
            }
        }
        best_idx
    }
}

// ---------------------------------------------------------------------------
// TropicalMatrix
// ---------------------------------------------------------------------------

/// A matrix in the tropical semiring.
///
/// Elements are [`TropicalF64`] values. Matrix "multiplication" uses
/// tropical operations: `C_ij = max_k(A_ik + B_kj)`.
#[derive(Debug, Clone, PartialEq)]
pub struct TropicalMatrix {
    /// Number of rows.
    pub rows: usize,
    /// Number of columns.
    pub cols: usize,
    /// Row-major data.
    data: Vec<TropicalF64>,
}

impl TropicalMatrix {
    /// Create a new tropical matrix initialized to tropical zero (-inf).
    #[must_use]
    pub fn zeros(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            data: vec![TropicalF64::ZERO; rows * cols],
        }
    }

    /// Create the tropical identity matrix (0 on diagonal, -inf elsewhere).
    #[must_use]
    pub fn identity(n: usize) -> Self {
        let mut m = Self::zeros(n, n);
        for i in 0..n {
            m.set(i, i, TropicalF64::ONE);
        }
        m
    }

    /// Create from a row-major f64 slice.
    #[must_use]
    pub fn from_f64(rows: usize, cols: usize, data: &[f64]) -> Self {
        assert_eq!(data.len(), rows * cols, "data length mismatch");
        Self {
            rows,
            cols,
            data: data.iter().copied().map(TropicalF64).collect(),
        }
    }

    /// Get element at (row, col).
    #[must_use]
    pub fn get(&self, row: usize, col: usize) -> TropicalF64 {
        self.data[row * self.cols + col]
    }

    /// Set element at (row, col).
    pub fn set(&mut self, row: usize, col: usize, value: TropicalF64) {
        self.data[row * self.cols + col] = value;
    }

    /// Tropical matrix multiplication: `C_ij = max_k(A_ik + B_kj)`.
    ///
    /// This is the max-plus analogue of standard matrix multiply.
    /// Each element of the result is the tropical inner product of a row
    /// of self with a column of rhs.
    #[must_use]
    pub fn mul(&self, rhs: &TropicalMatrix) -> TropicalMatrix {
        assert_eq!(
            self.cols, rhs.rows,
            "dimension mismatch for tropical matmul"
        );
        let mut result = TropicalMatrix::zeros(self.rows, rhs.cols);
        for i in 0..self.rows {
            for j in 0..rhs.cols {
                let mut val = TropicalF64::ZERO;
                for k in 0..self.cols {
                    // tropical: max(val, A_ik + B_kj)
                    val = val + (self.get(i, k) * rhs.get(k, j));
                }
                result.set(i, j, val);
            }
        }
        result
    }

    /// Tropical matrix-vector product: `y_i = max_j(M_ij + x_j)`.
    #[must_use]
    pub fn mul_vec(&self, x: &[TropicalF64]) -> Vec<TropicalF64> {
        assert_eq!(x.len(), self.cols, "vector length mismatch");
        let mut result = vec![TropicalF64::ZERO; self.rows];
        for i in 0..self.rows {
            for j in 0..self.cols {
                result[i] = result[i] + (self.get(i, j) * x[j]);
            }
        }
        result
    }
}

// ---------------------------------------------------------------------------
// Tropical attention
// ---------------------------------------------------------------------------

/// Compute tropical attention: `max_j(q . k_j + v_j)`.
///
/// This is the tropical limit of softmax attention:
/// `softmax(QK^T/sqrt(d))V` becomes `max_j(Q_i . K_j + V_j)` when the
/// softmax temperature goes to zero. The result is piecewise-linear and
/// exactly interpretable.
///
/// # Arguments
/// - `q`: query vector (d-dimensional).
/// - `keys`: sequence of key vectors, each d-dimensional.
/// - `values`: value scalar for each key position.
///
/// Returns `(max_value, attending_to_index)`.
#[must_use]
pub fn tropical_attention(q: &[f64], keys: &[Vec<f64>], values: &[f64]) -> (f64, usize) {
    assert_eq!(keys.len(), values.len(), "keys and values length mismatch");
    assert!(!keys.is_empty(), "need at least one key");

    let mut best_val = f64::NEG_INFINITY;
    let mut best_idx = 0;

    for (j, key) in keys.iter().enumerate() {
        // Dot product q . k_j (standard arithmetic).
        let dot: f64 = q.iter().zip(key.iter()).map(|(a, b)| a * b).sum();
        // Tropical attention score: dot + value_j.
        let score = dot + values[j];
        if score > best_val {
            best_val = score;
            best_idx = j;
        }
    }

    (best_val, best_idx)
}

/// Compute tropical attention over a full batch of queries.
///
/// For each query, finds `max_j(q_i . k_j + v_j)`.
///
/// Returns a vector of `(max_value, attending_to_index)` per query.
#[must_use]
pub fn tropical_attention_batch(
    queries: &[Vec<f64>],
    keys: &[Vec<f64>],
    values: &[f64],
) -> Vec<(f64, usize)> {
    queries
        .iter()
        .map(|q| tropical_attention(q, keys, values))
        .collect()
}

/// Compute the adversarial distance for a tropical polynomial decision boundary.
///
/// Given a polynomial and a point, this computes the minimum perturbation
/// needed to change which term is active (i.e., cross a decision boundary).
/// This equals half the gap between the two highest-scoring terms.
///
/// Returns `None` if the polynomial has fewer than 2 terms.
#[must_use]
pub fn adversarial_distance(poly: &TropicalPolynomial, point: &[TropicalF64]) -> Option<f64> {
    if poly.terms.len() < 2 {
        return None;
    }

    // Evaluate each term at the point.
    let mut scores: Vec<f64> = poly
        .terms
        .iter()
        .map(|term| {
            let mut val = term.coefficient.0;
            for (exp, x) in term.exponents.iter().zip(point.iter()) {
                if x.is_zero() && *exp > 0 {
                    return f64::NEG_INFINITY;
                }
                val += (*exp as f64) * x.0;
            }
            val
        })
        .collect();

    scores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    // Gap between top two scores divided by 2 (L-inf perturbation to flip).
    if scores.len() >= 2 && scores[0].is_finite() && scores[1].is_finite() {
        Some((scores[0] - scores[1]) / 2.0)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- TropicalF64 basics ---

    #[test]
    fn tropical_addition_is_max() {
        let a = TropicalF64(3.0);
        let b = TropicalF64(5.0);
        assert_eq!((a + b).0, 5.0);

        let c = TropicalF64(-2.0);
        assert_eq!((a + c).0, 3.0);
    }

    #[test]
    fn tropical_multiplication_is_standard_addition() {
        let a = TropicalF64(3.0);
        let b = TropicalF64(5.0);
        assert_eq!((a * b).0, 8.0);

        let c = TropicalF64(-2.0);
        assert_eq!((a * c).0, 1.0);
    }

    #[test]
    fn tropical_zero_identity() {
        let a = TropicalF64(42.0);
        // max(42, -inf) = 42
        assert_eq!((a + TropicalF64::ZERO).0, 42.0);
        // -inf is absorbing for multiplication: -inf + 42 = -inf
        assert_eq!((a * TropicalF64::ZERO).0, f64::NEG_INFINITY);
    }

    #[test]
    fn tropical_one_identity() {
        let a = TropicalF64(42.0);
        // 42 + 0 = 42
        assert_eq!((a * TropicalF64::ONE).0, 42.0);
    }

    #[test]
    fn tropical_pow() {
        let a = TropicalF64(3.0);
        // a^3 in tropical = 3 * 3 = 9 in standard
        assert_eq!(a.trop_pow(3).0, 9.0);
        // a^0 = tropical one = 0
        assert_eq!(a.trop_pow(0).0, 0.0);
        // a^(-1) = -3
        assert_eq!(a.trop_pow(-1).0, -3.0);
    }

    #[test]
    fn tropical_semiring_associativity() {
        let a = TropicalF64(1.0);
        let b = TropicalF64(2.0);
        let c = TropicalF64(3.0);

        // Addition is associative: max(max(a,b),c) == max(a,max(b,c))
        assert_eq!(((a + b) + c).0, (a + (b + c)).0);

        // Multiplication is associative: (a+b)+c == a+(b+c) in std
        assert_eq!(((a * b) * c).0, (a * (b * c)).0);
    }

    #[test]
    fn tropical_distributive_law() {
        // a * (b + c) == (a * b) + (a * c)
        // i.e. a + max(b, c) == max(a + b, a + c)
        let a = TropicalF64(2.0);
        let b = TropicalF64(3.0);
        let c = TropicalF64(5.0);

        let lhs = a * (b + c);
        let rhs = (a * b) + (a * c);
        assert_eq!(lhs.0, rhs.0);
    }

    // --- TropicalPolynomial ---

    #[test]
    fn polynomial_evaluation_max_over_affine() {
        // p(x) = max(2 + 1*x, 5 + (-1)*x)
        // Two affine functions: f1(x) = 2 + x, f2(x) = 5 - x
        // At x = 0: max(2, 5) = 5
        // At x = 4: max(6, 1) = 6
        // At x = 1.5: max(3.5, 3.5) = 3.5 (intersection)
        let poly = TropicalPolynomial::from_affine(&[(2.0, vec![1]), (5.0, vec![-1])]);

        let at_0 = poly.evaluate(&[TropicalF64(0.0)]);
        assert_eq!(at_0.0, 5.0);

        let at_4 = poly.evaluate(&[TropicalF64(4.0)]);
        assert_eq!(at_4.0, 6.0);

        let at_1_5 = poly.evaluate(&[TropicalF64(1.5)]);
        assert!((at_1_5.0 - 3.5).abs() < 1e-10);
    }

    #[test]
    fn polynomial_2d_evaluation() {
        // p(x, y) = max(1 + 2x + 0y, 3 + 0x + 1y)
        let poly = TropicalPolynomial::from_affine(&[(1.0, vec![2, 0]), (3.0, vec![0, 1])]);

        // At (1, 1): max(1 + 2*1, 3 + 1*1) = max(3, 4) = 4
        let result = poly.evaluate(&[TropicalF64(1.0), TropicalF64(1.0)]);
        assert_eq!(result.0, 4.0);

        // At (2, 0): max(1 + 4, 3 + 0) = max(5, 3) = 5
        let result = poly.evaluate(&[TropicalF64(2.0), TropicalF64(0.0)]);
        assert_eq!(result.0, 5.0);
    }

    #[test]
    fn polynomial_active_term() {
        let poly = TropicalPolynomial::from_affine(&[
            (2.0, vec![1]),  // 2 + x
            (5.0, vec![-1]), // 5 - x
        ]);

        // At x = 0: 2 vs 5 -> term 1
        assert_eq!(poly.active_term(&[TropicalF64(0.0)]), Some(1));
        // At x = 4: 6 vs 1 -> term 0
        assert_eq!(poly.active_term(&[TropicalF64(4.0)]), Some(0));
    }

    // --- TropicalMatrix ---

    #[test]
    fn tropical_matrix_identity() {
        let id = TropicalMatrix::identity(3);
        // Diagonal = 0 (tropical one), off-diagonal = -inf (tropical zero).
        assert_eq!(id.get(0, 0).0, 0.0);
        assert_eq!(id.get(1, 1).0, 0.0);
        assert_eq!(id.get(0, 1).0, f64::NEG_INFINITY);
    }

    #[test]
    fn tropical_matrix_mul_identity() {
        let a = TropicalMatrix::from_f64(2, 2, &[1.0, 2.0, 3.0, 4.0]);
        let id = TropicalMatrix::identity(2);
        let result = a.mul(&id);

        // A * I = A in tropical.
        assert_eq!(result.get(0, 0).0, 1.0);
        assert_eq!(result.get(0, 1).0, 2.0);
        assert_eq!(result.get(1, 0).0, 3.0);
        assert_eq!(result.get(1, 1).0, 4.0);
    }

    #[test]
    fn tropical_matrix_mul_example() {
        // A = [[1, 3], [2, 4]]
        // B = [[5, 6], [7, 8]]
        // C_ij = max_k(A_ik + B_kj)
        // C_00 = max(1+5, 3+7) = max(6, 10) = 10
        // C_01 = max(1+6, 3+8) = max(7, 11) = 11
        // C_10 = max(2+5, 4+7) = max(7, 11) = 11
        // C_11 = max(2+6, 4+8) = max(8, 12) = 12
        let a = TropicalMatrix::from_f64(2, 2, &[1.0, 3.0, 2.0, 4.0]);
        let b = TropicalMatrix::from_f64(2, 2, &[5.0, 6.0, 7.0, 8.0]);
        let c = a.mul(&b);

        assert_eq!(c.get(0, 0).0, 10.0);
        assert_eq!(c.get(0, 1).0, 11.0);
        assert_eq!(c.get(1, 0).0, 11.0);
        assert_eq!(c.get(1, 1).0, 12.0);
    }

    #[test]
    fn tropical_matrix_vec_product() {
        // M = [[1, 2], [3, 4]]
        // x = [5, 6]
        // y_0 = max(1+5, 2+6) = max(6, 8) = 8
        // y_1 = max(3+5, 4+6) = max(8, 10) = 10
        let m = TropicalMatrix::from_f64(2, 2, &[1.0, 2.0, 3.0, 4.0]);
        let x = vec![TropicalF64(5.0), TropicalF64(6.0)];
        let y = m.mul_vec(&x);

        assert_eq!(y[0].0, 8.0);
        assert_eq!(y[1].0, 10.0);
    }

    // --- Tropical attention ---

    #[test]
    fn tropical_attention_selects_best_key() {
        // q = [1, 0], keys = [[1, 0], [0, 1]], values = [0, 0]
        // scores: q.k0 + v0 = 1 + 0 = 1, q.k1 + v1 = 0 + 0 = 0
        let q = vec![1.0, 0.0];
        let keys = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let values = vec![0.0, 0.0];

        let (val, idx) = tropical_attention(&q, &keys, &values);
        assert_eq!(val, 1.0);
        assert_eq!(idx, 0);
    }

    #[test]
    fn tropical_attention_values_add_bias() {
        // q = [1, 0], keys = [[1, 0], [0, 1]], values = [0, 10]
        // scores: 1+0=1, 0+10=10 -> second key wins due to high value
        let q = vec![1.0, 0.0];
        let keys = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let values = vec![0.0, 10.0];

        let (val, idx) = tropical_attention(&q, &keys, &values);
        assert_eq!(val, 10.0);
        assert_eq!(idx, 1);
    }

    #[test]
    fn tropical_attention_batch_works() {
        let queries = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let keys = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let values = vec![0.0, 0.0];

        let results = tropical_attention_batch(&queries, &keys, &values);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1, 0); // first query matches first key
        assert_eq!(results[1].1, 1); // second query matches second key
    }

    // --- Adversarial distance ---

    #[test]
    fn adversarial_distance_measures_gap() {
        // p(x) = max(2 + x, 5 - x)
        // At x = 0: scores are 2 and 5, gap = 3, distance = 1.5
        let poly = TropicalPolynomial::from_affine(&[(2.0, vec![1]), (5.0, vec![-1])]);

        let dist = adversarial_distance(&poly, &[TropicalF64(0.0)]);
        assert_eq!(dist, Some(1.5));
    }

    #[test]
    fn adversarial_distance_at_boundary() {
        // p(x) = max(2 + x, 5 - x), intersection at x = 1.5
        let poly = TropicalPolynomial::from_affine(&[(2.0, vec![1]), (5.0, vec![-1])]);

        let dist = adversarial_distance(&poly, &[TropicalF64(1.5)]);
        assert!(
            dist.unwrap().abs() < 1e-10,
            "distance at boundary should be 0"
        );
    }

    #[test]
    fn adversarial_distance_single_term() {
        let poly = TropicalPolynomial::from_affine(&[(1.0, vec![1])]);
        assert_eq!(adversarial_distance(&poly, &[TropicalF64(0.0)]), None);
    }

    // --- Semiring properties (property checks) ---

    #[test]
    fn tropical_commutativity() {
        let a = TropicalF64(7.0);
        let b = TropicalF64(3.0);

        // Additive commutativity: max(a,b) == max(b,a)
        assert_eq!((a + b).0, (b + a).0);

        // Multiplicative commutativity: a+b == b+a in standard
        assert_eq!((a * b).0, (b * a).0);
    }

    #[test]
    fn tropical_zero_absorbs_multiplication() {
        let a = TropicalF64(100.0);
        let zero = TropicalF64::ZERO;
        // -inf + 100 = -inf
        assert!((a * zero).is_zero());
        assert!((zero * a).is_zero());
    }
}
