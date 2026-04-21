//! Canonical Pleasure-Arousal-Dominance (PAD) vector for affect modeling.
//!
//! [`PadVector`] is the single canonical representation used across the entire
//! Roko workspace.  All dimensions are `f64` in `[-1.0, 1.0]`.
//!
//! # Why f64?
//!
//! f64 avoids precision loss in chained arithmetic (decay, EMA, cosine
//! similarity) that would accumulate across cognitive ticks when using f32.
//! Modules that need compact atomic storage (e.g. `CorticalState` in
//! roko-runtime) should convert at the boundary via `as f32` / `as f64`.

use serde::{Deserialize, Serialize};

/// Normalized Pleasure-Arousal-Dominance vector.
///
/// The canonical affect primitive shared across roko-core, roko-runtime,
/// roko-daimon, roko-neuro, and roko-dreams.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct PadVector {
    /// Pleasure axis in `[-1.0, 1.0]`.
    pub pleasure: f64,
    /// Arousal axis in `[-1.0, 1.0]`.
    pub arousal: f64,
    /// Dominance axis in `[-1.0, 1.0]`.
    pub dominance: f64,
}

impl PadVector {
    /// Construct a PAD vector (unclamped — use [`clamped`](Self::clamped) if needed).
    #[must_use]
    pub const fn new(pleasure: f64, arousal: f64, dominance: f64) -> Self {
        Self {
            pleasure,
            arousal,
            dominance,
        }
    }

    /// Neutral PAD vector (all zeros).
    #[must_use]
    pub const fn neutral() -> Self {
        Self::new(0.0, 0.0, 0.0)
    }

    /// Clamp all dimensions to the legal `[-1.0, 1.0]` range.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            pleasure: self.pleasure.clamp(-1.0, 1.0),
            arousal: self.arousal.clamp(-1.0, 1.0),
            dominance: self.dominance.clamp(-1.0, 1.0),
        }
    }

    /// Add a delta in-place, keeping the vector normalized.
    pub fn apply_delta(&mut self, pleasure: f64, arousal: f64, dominance: f64) {
        *self = Self::new(
            self.pleasure + pleasure,
            self.arousal + arousal,
            self.dominance + dominance,
        )
        .clamped();
    }

    /// Apply an exponential decay factor in-place.
    pub fn decay_by_factor(&mut self, factor: f64) {
        *self = Self::new(
            self.pleasure * factor,
            self.arousal * factor,
            self.dominance * factor,
        )
        .clamped();
    }

    /// Euclidean magnitude of the PAD vector.
    #[must_use]
    pub fn magnitude(self) -> f64 {
        (self.pleasure.powi(2) + self.arousal.powi(2) + self.dominance.powi(2)).sqrt()
    }

    /// PAD cosine similarity mapped to `[0.0, 1.0]`.
    #[must_use]
    pub fn cosine_similarity(self, other: Self) -> f64 {
        let dot = self.pleasure * other.pleasure
            + self.arousal * other.arousal
            + self.dominance * other.dominance;
        let mag_self = self.magnitude();
        let mag_other = other.magnitude();
        if mag_self == 0.0 || mag_other == 0.0 {
            return 0.5;
        }
        (dot / (mag_self * mag_other) + 1.0) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_is_zero() {
        let v = PadVector::neutral();
        assert_eq!(v.pleasure, 0.0);
        assert_eq!(v.arousal, 0.0);
        assert_eq!(v.dominance, 0.0);
    }

    #[test]
    fn clamped_caps_values() {
        let v = PadVector::new(2.0, -2.0, 0.5).clamped();
        assert_eq!(v.pleasure, 1.0);
        assert_eq!(v.arousal, -1.0);
        assert_eq!(v.dominance, 0.5);
    }

    #[test]
    fn cosine_similarity_neutral_fallback() {
        assert_eq!(
            PadVector::neutral().cosine_similarity(PadVector::new(1.0, 0.0, 0.0)),
            0.5
        );
    }

    #[test]
    fn magnitude_unit_vector() {
        let v = PadVector::new(1.0, 0.0, 0.0);
        assert!((v.magnitude() - 1.0).abs() < 1e-10);
    }
}
