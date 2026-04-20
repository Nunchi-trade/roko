//! `roko-primitives` — pure compute primitives with zero internal workspace dependencies.
//!
//! This crate holds shared data structures that multiple top-level crates need
//! without pulling in Roko-platform-specific code:
//!
//! - [`HdcVector`]: 10,240-bit hyperdimensional computing vector (XOR bind, majority bundle,
//!   Hamming similarity, serialization, deterministic seeding)
//! - [`InferenceTier`]: three-tier inference gate (T0/T1/T2) with u8 conversion
//! - [`TierRouter`]: pure model-selection function mapping tier + vitality -> model name
//!
//! Any crate that needs HDC or tier routing should depend on this crate directly rather
//! than pulling in higher-level workspace crates.

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// HDC codebook: deterministic symbol allocation, role-filler binding,
/// pattern store, and cross-domain resonance detection (TA-05).
pub mod codebook;
pub mod hdc;
/// Riemannian geometry: metric tensors, Christoffel symbols, geodesics,
/// Ricci curvature, and Frechet means for execution cost manifolds (TA-06).
pub mod manifold;
pub mod pad;
/// Robust statistics: trimmed mean, MAD, Hodges-Lehmann estimator (TA-10).
pub mod robust_stats;
/// Cellular sheaves for oracle consistency checking: coboundary operators,
/// sheaf Laplacian, inconsistency scores, and outlier identification (TA-13).
pub mod sheaf;
/// Topological Data Analysis: persistence diagrams, Takens embedding,
/// persistence landscapes (TA-09).
pub mod tda;
pub mod tier;
/// Tropical (max-plus) algebra: `TropicalF64`, polynomials, tropical attention,
/// and adversarial distance computation (TA-14).
pub mod tropical;

pub use codebook::{
    Codebook, CodingCodebook, PatternStore, RESONANCE_THRESHOLD, ResonanceResult, StoredPattern,
    detect_cross_domain_resonance, role_bind, unbind,
};
pub use hdc::{
    BundleAccumulator, DecayingBundleAccumulator, HDC_BITS, HDC_BYTES, HdcVector, ItemMemory,
};
pub use pad::PadVector;
pub use tier::{InferenceTier, T2_VITALITY_THRESHOLD, TierError, TierRouter};
