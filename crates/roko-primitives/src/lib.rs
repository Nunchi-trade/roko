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
pub mod pad;
/// Robust statistics: trimmed mean, MAD, Hodges-Lehmann estimator (TA-10).
pub mod robust_stats;
/// Topological Data Analysis: persistence diagrams, Takens embedding,
/// persistence landscapes (TA-09).
pub mod tda;
pub mod tier;

pub use hdc::{
    BundleAccumulator, DecayingBundleAccumulator, HDC_BITS, HDC_BYTES, HdcVector, ItemMemory,
};
pub use codebook::{
    Codebook, CodingCodebook, PatternStore, ResonanceResult, StoredPattern,
    RESONANCE_THRESHOLD, detect_cross_domain_resonance, role_bind, unbind,
};
pub use pad::PadVector;
pub use tier::{InferenceTier, T2_VITALITY_THRESHOLD, TierError, TierRouter};
