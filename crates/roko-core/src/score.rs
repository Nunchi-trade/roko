//! Multi-dimensional scoring for signals.
//!
//! A [`Score`] is four orthogonal axes combined into a single scalar via
//! [`Score::effective`]. This is what scorers produce and routers consume.
//!
//! The four axes were chosen because every scoring mechanism in the Roko
//! design corpus (confidence, novelty, utility, reputation, fitness,
//! pfUtility, catalytic score, …) collapses into one of these:
//!
//! - **confidence**: how sure are we this signal is correct? \[0..1\]
//! - **novelty**: how new/surprising is this signal? \[0..1\]
//! - **utility**: how useful was this signal historically? \[0..∞)
//! - **reputation**: author's trustworthiness at time of emission \[0..∞)
//!
//! Scorers compose via arithmetic: `score_a * score_b` scales each axis
//! independently. `score_a + score_b` aggregates evidence.

use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul};

/// A multi-dimensional quality score for a signal.
///
/// Scores are typically computed at query time by a [`Scorer`](crate::Scorer)
/// rather than stored on the signal itself, so different contexts can rank
/// the same signal differently.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Score {
    /// \[0..1\] — how confident are we this signal is correct/valid?
    pub confidence: f32,
    /// \[0..1\] — how novel is this signal compared to prior signals?
    pub novelty: f32,
    /// \[0..∞) — how useful has this signal proven historically?
    pub utility: f32,
    /// \[0..∞) — reputation of the signal's author at emission time.
    pub reputation: f32,
}

impl Score {
    /// A zero score (all axes = 0). Equivalent to "no evidence".
    pub const ZERO: Self = Self {
        confidence: 0.0,
        novelty: 0.0,
        utility: 0.0,
        reputation: 0.0,
    };

    /// A neutral score (confidence=0.5, others=0). Default when no scorer is applied.
    pub const NEUTRAL: Self = Self {
        confidence: 0.5,
        novelty: 0.0,
        utility: 0.0,
        reputation: 1.0,
    };

    /// Construct a `Score`; values are clamped to their respective valid ranges.
    #[must_use]
    pub const fn new(confidence: f32, novelty: f32, utility: f32, reputation: f32) -> Self {
        Self {
            confidence: confidence.clamp(0.0, 1.0),
            novelty: novelty.clamp(0.0, 1.0),
            utility: if utility > 0.0 { utility } else { 0.0 },
            reputation: if reputation > 0.0 { reputation } else { 0.0 },
        }
    }

    /// Scalar effective score combining all four axes.
    ///
    /// The formula `confidence × (1 + novelty) × (1 + utility) × reputation`
    /// was chosen so that:
    /// - zero confidence → zero effective score (false positives are worthless)
    /// - novelty and utility act as multipliers (additive bonuses to 1.0)
    /// - reputation directly scales the result
    #[must_use]
    pub fn effective(&self) -> f32 {
        self.confidence * (1.0 + self.novelty) * (1.0 + self.utility) * self.reputation
    }

    /// Is this score above the given threshold on the effective axis?
    #[must_use]
    pub fn exceeds(&self, threshold: f32) -> bool {
        self.effective() > threshold
    }

    /// Compute a score from a confidence value alone, using neutral reputation.
    #[must_use]
    pub const fn from_confidence(confidence: f32) -> Self {
        Self::new(confidence, 0.0, 0.0, 1.0)
    }
}

impl Default for Score {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

/// Element-wise multiplication — scales each axis independently.
/// Useful for combining a base score with a per-axis modifier.
impl Mul for Score {
    type Output = Self;
    fn mul(self, other: Self) -> Self {
        Self {
            confidence: (self.confidence * other.confidence).clamp(0.0, 1.0),
            novelty: (self.novelty * other.novelty).clamp(0.0, 1.0),
            utility: self.utility * other.utility,
            reputation: self.reputation * other.reputation,
        }
    }
}

/// Element-wise addition — aggregates evidence from multiple scorers.
/// Confidence and novelty are clamped to 1.0; utility and reputation accumulate.
impl Add for Score {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self {
            confidence: (self.confidence + other.confidence).clamp(0.0, 1.0),
            novelty: (self.novelty + other.novelty).clamp(0.0, 1.0),
            utility: self.utility + other.utility,
            reputation: self.reputation + other.reputation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_clamps_values() {
        let s = Score::new(2.0, -1.0, -5.0, -1.0);
        assert_eq!(s.confidence, 1.0);
        assert_eq!(s.novelty, 0.0);
        assert_eq!(s.utility, 0.0);
        assert_eq!(s.reputation, 0.0);
    }

    #[test]
    fn zero_effective_when_no_confidence() {
        let s = Score::new(0.0, 1.0, 10.0, 5.0);
        assert_eq!(s.effective(), 0.0);
    }

    #[test]
    fn effective_scales_with_all_axes() {
        // confidence=1, novelty=0, utility=0, reputation=1 → 1 × 1 × 1 × 1 = 1
        let s = Score::new(1.0, 0.0, 0.0, 1.0);
        assert!((s.effective() - 1.0).abs() < 1e-6);

        // confidence=0.5, novelty=1, utility=1, reputation=2 → 0.5×2×2×2 = 4
        let s = Score::new(0.5, 1.0, 1.0, 2.0);
        assert!((s.effective() - 4.0).abs() < 1e-6);
    }

    #[test]
    fn multiplication_scales_independently() {
        let a = Score::new(0.8, 0.5, 2.0, 1.5);
        let modifier = Score::new(1.0, 1.0, 0.5, 2.0);
        let product = a * modifier;
        assert!((product.confidence - 0.8).abs() < 1e-6);
        assert!((product.utility - 1.0).abs() < 1e-6);
        assert!((product.reputation - 3.0).abs() < 1e-6);
    }

    #[test]
    fn addition_aggregates_evidence() {
        let a = Score::new(0.3, 0.4, 1.0, 1.0);
        let b = Score::new(0.4, 0.4, 2.0, 0.5);
        let sum = a + b;
        assert!((sum.confidence - 0.7).abs() < 1e-6);
        assert!((sum.novelty - 0.8).abs() < 1e-6);
        assert!((sum.utility - 3.0).abs() < 1e-6);
        assert!((sum.reputation - 1.5).abs() < 1e-6);
    }

    #[test]
    fn addition_clamps_confidence_and_novelty() {
        let a = Score::new(0.8, 0.9, 0.0, 0.0);
        let b = Score::new(0.5, 0.5, 0.0, 0.0);
        let sum = a + b;
        assert_eq!(sum.confidence, 1.0);
        assert_eq!(sum.novelty, 1.0);
    }

    #[test]
    fn exceeds_uses_effective() {
        let low = Score::new(0.1, 0.0, 0.0, 1.0); // effective = 0.1
        let high = Score::new(0.9, 0.0, 0.0, 1.0); // effective = 0.9
        assert!(!low.exceeds(0.5));
        assert!(high.exceeds(0.5));
    }
}
