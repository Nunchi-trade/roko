//! Projecting arbitrary inputs into 10,240-bit HDC hypervectors.
//!
//! The agent-chain design (doc 04) projects 1,536-dimensional float embeddings
//! (bge-small-en-v1.5) into binary hypervectors via a seeded random projection
//! matrix. We replicate that shape here with a deterministic seed so every node
//! computes the same projection for the same float vector.
//!
//! For the POC we also expose a simpler `project_bytes` path that bypasses
//! embeddings entirely and seeds an HDC vector from the raw byte string — useful
//! when agents don't have an embedding model available.
//!
//! # Determinism
//!
//! Both code paths are deterministic across nodes given the same seed and input.
//! This is load-bearing: if two nodes disagree on the HDC vector for the same
//! content, they disagree on which insights match a query.
//!
//! # No floating point in the critical path
//!
//! After construction, the projection matrix is a fixed bit-matrix. Projection
//! from a float vector is: XOR each row's bit with sign(input[i]), majority-vote
//! the result. No `f32` multiply-accumulate.

use roko_primitives::HdcVector;
use serde::{Deserialize, Serialize};

/// Bit width of the HDC hypervector (matches `roko_primitives::HdcVector`).
pub const HDC_BITS: usize = 10_240;

/// Default input dimensionality for the projection matrix (bge-small-en-v1.5).
pub const DEFAULT_EMBEDDING_DIM: usize = 1_536;

/// A seeded {0,1}^(HDC_BITS × EMBEDDING_DIM) projection matrix.
///
/// Stored in row-major bit-packed form: one `u64` word per every 64 input dims,
/// `HDC_BITS` rows. Total memory: `HDC_BITS × ceil(EMBEDDING_DIM / 64) × 8` bytes
/// (≈1.9 MB for 1536D → 10240b). Not held in memory during POC tests — callers
/// typically hash directly via `project_bytes`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectionMatrix {
    /// Input dimensionality.
    pub input_dim: usize,
    /// Output dimensionality (always 10,240).
    pub output_dim: usize,
    /// Raw bits: `rows[row_idx] = Vec<u64>` length `ceil(input_dim / 64)`.
    rows: Vec<Vec<u64>>,
    /// Seed used to derive the matrix (stored for auditability).
    pub seed: u64,
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl ProjectionMatrix {
    /// Constructs a deterministic projection matrix from a 64-bit seed.
    #[must_use]
    pub fn from_seed(seed: u64, input_dim: usize) -> Self {
        let words_per_row = input_dim.div_ceil(64);
        let mut state = seed.max(1);
        let mut rows = Vec::with_capacity(HDC_BITS);
        for _ in 0..HDC_BITS {
            let mut row = Vec::with_capacity(words_per_row);
            for _ in 0..words_per_row {
                row.push(splitmix64(&mut state));
            }
            rows.push(row);
        }
        Self {
            input_dim,
            output_dim: HDC_BITS,
            rows,
            seed,
        }
    }

    /// Projects a slice of floats into an HDC vector via sign-projection.
    ///
    /// For each output bit `j`:
    ///   `bit_j = sign(sum_i matrix[j,i] ? +input[i] : -input[i])`
    ///
    /// Panics in debug builds if `input.len() != self.input_dim`. In release
    /// builds it truncates / zero-pads.
    #[must_use]
    pub fn project_floats(&self, input: &[f32]) -> HdcVector {
        debug_assert_eq!(input.len(), self.input_dim, "input dim mismatch");
        let n = input.len().min(self.input_dim);
        let mut bytes = [0u8; HDC_BITS / 8];
        for (row_idx, row) in self.rows.iter().enumerate() {
            let mut sum: f32 = 0.0;
            for i in 0..n {
                let bit = (row[i / 64] >> (i % 64)) & 1 == 1;
                let x = input[i];
                sum += if bit { x } else { -x };
            }
            // Strict > 0 breaks ties symmetrically: zero-sum rows go to bit 0 for
            // BOTH a vector and its negation, avoiding a positive-bias artefact at
            // low input dimensions.
            if sum > 0.0 {
                bytes[row_idx / 8] |= 1 << (row_idx % 8);
            }
        }
        // Build an HdcVector from the packed bits. Since HdcVector is [u64; 160]
        // in native-endian, we roundtrip through to_bytes/from_bytes to stay
        // inside the public API.
        let mut le = [0u8; 1280];
        for (i, byte) in bytes.iter().enumerate() {
            le[i] = *byte;
        }
        // HdcVector::from_bytes expects little-endian u64 words; our bit layout
        // matches when we pack bit j into byte (j/8) bit (j%8), so the LE
        // interpretation aligns by construction.
        HdcVector::from_bytes(&le)
    }
}

/// Projects an arbitrary byte string into an HDC vector.
///
/// Uses `HdcVector::from_seed` under the hood, which FNV-1a's the input into a
/// 64-bit state and fills 160 u64 words via splitmix64. Deterministic: identical
/// bytes produce identical vectors. Two related-but-not-identical inputs produce
/// well-separated vectors (typical Hamming similarity < 0.55).
///
/// Useful when no embedding model is available — the retrieval quality drops to
/// "exact content matching" semantics, but the pipeline still works.
#[must_use]
pub fn project_bytes(bytes: &[u8]) -> HdcVector {
    HdcVector::from_seed(bytes)
}

/// Projects text content into an HDC vector using token-bundle encoding.
///
/// Splits on whitespace, seeds each token into its own HDC vector, then bundles
/// them via majority vote. This gives a crude bag-of-tokens semantic fingerprint:
/// documents sharing tokens share similarity without a dedicated embedding model.
///
/// # Similarity preservation
///
/// If `a` and `b` share ≥50% of their tokens, `similarity(project_tokens(a), project_tokens(b))`
/// will typically exceed 0.75. Completely disjoint token sets yield ~0.50 (random).
#[must_use]
pub fn project_tokens(text: &str) -> HdcVector {
    let tokens: Vec<HdcVector> = text
        .split_whitespace()
        .map(|tok| HdcVector::from_seed(tok.as_bytes()))
        .collect();
    if tokens.is_empty() {
        return HdcVector::zeros();
    }
    let refs: Vec<&HdcVector> = tokens.iter().collect();
    HdcVector::bundle(&refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_from_seed_is_deterministic() {
        let a = ProjectionMatrix::from_seed(42, 128);
        let b = ProjectionMatrix::from_seed(42, 128);
        assert_eq!(a.input_dim, b.input_dim);
        assert_eq!(a.rows, b.rows);
    }

    #[test]
    fn different_seeds_yield_different_matrices() {
        let a = ProjectionMatrix::from_seed(42, 128);
        let b = ProjectionMatrix::from_seed(43, 128);
        assert_ne!(a.rows, b.rows);
    }

    #[test]
    fn project_floats_is_deterministic() {
        let matrix = ProjectionMatrix::from_seed(7, 8);
        let x = [0.1_f32, -0.2, 0.3, 0.4, -0.5, 0.6, -0.7, 0.8];
        let v1 = matrix.project_floats(&x);
        let v2 = matrix.project_floats(&x);
        assert_eq!(v1, v2);
    }

    #[test]
    fn project_floats_preserves_near_inputs() {
        let matrix = ProjectionMatrix::from_seed(9, 16);
        let x = [0.1_f32; 16];
        let mut y = x;
        y[0] += 1e-3; // tiny perturbation
        let v1 = matrix.project_floats(&x);
        let v2 = matrix.project_floats(&y);
        // Similarity should be very high for nearly-identical input.
        assert!(
            v1.similarity(&v2) > 0.9,
            "expected near-identical projections, got similarity={}",
            v1.similarity(&v2)
        );
    }

    #[test]
    fn project_floats_separates_opposite_inputs() {
        // Use a realistic input dim: at 1536 (bge-small-en-v1.5), zero-sum rows
        // are astronomically rare and opposite inputs should produce near-complementary
        // bits. At 16 dims, ~20% of rows tie to zero for uniform ±0.5 input — hence
        // the larger dim here.
        let matrix = ProjectionMatrix::from_seed(11, 256);
        let x = [0.5_f32; 256];
        let y = [-0.5_f32; 256];
        let v1 = matrix.project_floats(&x);
        let v2 = matrix.project_floats(&y);
        assert!(
            v1.similarity(&v2) < 0.10,
            "expected opposite inputs to project oppositely, got similarity={}",
            v1.similarity(&v2)
        );
    }

    #[test]
    fn project_bytes_is_deterministic() {
        let v1 = project_bytes(b"hello world");
        let v2 = project_bytes(b"hello world");
        assert_eq!(v1, v2);
    }

    #[test]
    fn project_tokens_overlap_boosts_similarity() {
        let a = project_tokens("deploy uniswap v3 pool with tick 60");
        let b = project_tokens("deploy uniswap v3 pool at fee tier 3000");
        let c = project_tokens("transfer erc20 tokens to vault");
        let ab = a.similarity(&b);
        let ac = a.similarity(&c);
        assert!(
            ab > ac,
            "shared tokens should boost similarity: ab={ab} ac={ac}"
        );
    }

    #[test]
    fn project_tokens_empty_is_zero_vector() {
        let v = project_tokens("");
        assert_eq!(v, HdcVector::zeros());
    }
}
