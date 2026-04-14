//! Multi-dimensional scoring for signals.
//!
//! A [`Score`] carries four primary axes plus three extended routing axes and
//! combines them into a single scalar via [`Score::effective`]. This is what
//! scorers produce and routers consume.
//!
//! The primary axes were chosen because every scoring mechanism in the Roko
//! design corpus (confidence, novelty, utility, reputation, fitness,
//! pfUtility, catalytic score, …) collapses into one of these:
//!
//! - **confidence**: how sure are we this signal is correct? \[0..1\]
//! - **novelty**: how new/surprising is this signal? \[0..1\]
//! - **utility**: how useful was this signal historically? \[0..∞)
//! - **reputation**: author's trustworthiness at time of emission \[0..∞)
//!
//! The extended axes provide extra shaping for downstream consumers without
//! forcing every scorer to populate them:
//!
//! - **precision**: how exact or narrowly applicable is this score? \[0..1\]
//! - **salience**: how much should this stand out during ranking? \[0..1\]
//! - **coherence**: how internally consistent is the supporting evidence? \[0..1\]
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
    /// \[0..1\] — how exact or narrowly applicable is this signal?
    #[serde(default)]
    pub precision: f32,
    /// \[0..1\] — how much extra ranking weight should this signal receive?
    #[serde(default)]
    pub salience: f32,
    /// \[0..1\] — how internally consistent is the evidence?
    #[serde(default)]
    pub coherence: f32,
}

impl Score {
    /// A zero score (all axes = 0). Equivalent to "no evidence".
    pub const ZERO: Self = Self {
        confidence: 0.0,
        novelty: 0.0,
        utility: 0.0,
        reputation: 0.0,
        precision: 0.0,
        salience: 0.0,
        coherence: 0.0,
    };

    /// A neutral score (confidence=0.5, others=0). Default when no scorer is applied.
    pub const NEUTRAL: Self = Self {
        confidence: 0.5,
        novelty: 0.0,
        utility: 0.0,
        reputation: 1.0,
        precision: 0.0,
        salience: 0.0,
        coherence: 0.0,
    };

    /// Construct a `Score`; values are clamped to their respective valid ranges.
    #[must_use]
    pub const fn new(confidence: f32, novelty: f32, utility: f32, reputation: f32) -> Self {
        Self {
            confidence: confidence.clamp(0.0, 1.0),
            novelty: novelty.clamp(0.0, 1.0),
            utility: if utility > 0.0 { utility } else { 0.0 },
            reputation: if reputation > 0.0 { reputation } else { 0.0 },
            precision: 0.0,
            salience: 0.0,
            coherence: 0.0,
        }
    }

    /// Construct a `Score` with the extended axes populated explicitly.
    #[must_use]
    pub const fn new_extended(
        confidence: f32,
        novelty: f32,
        utility: f32,
        reputation: f32,
        precision: f32,
        salience: f32,
        coherence: f32,
    ) -> Self {
        Self {
            confidence: confidence.clamp(0.0, 1.0),
            novelty: novelty.clamp(0.0, 1.0),
            utility: if utility > 0.0 { utility } else { 0.0 },
            reputation: if reputation > 0.0 { reputation } else { 0.0 },
            precision: precision.clamp(0.0, 1.0),
            salience: salience.clamp(0.0, 1.0),
            coherence: coherence.clamp(0.0, 1.0),
        }
    }

    /// Scalar effective score combining the primary axes plus salience and coherence.
    ///
    /// The formula
    /// `confidence × (1 + novelty) × (1 + utility) × reputation × (0.5 + 0.5 × salience) × (0.5 + 0.5 × coherence)`
    /// was chosen so that:
    /// - zero confidence → zero effective score (false positives are worthless)
    /// - novelty and utility act as multipliers (additive bonuses to 1.0)
    /// - reputation directly scales the result
    /// - salience and coherence softly damp or boost the final ranking
    /// - precision is tracked separately and does not affect the scalar score
    #[must_use]
    pub fn effective(&self) -> f32 {
        let salience_factor = if self.salience == 0.0 {
            1.0
        } else {
            0.5 + 0.5 * self.salience
        };
        let coherence_factor = if self.coherence == 0.0 {
            1.0
        } else {
            0.5 + 0.5 * self.coherence
        };
        self.confidence
            * (1.0 + self.novelty)
            * (1.0 + self.utility)
            * self.reputation
            * salience_factor
            * coherence_factor
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
            precision: (self.precision * other.precision).clamp(0.0, 1.0),
            salience: (self.salience * other.salience).clamp(0.0, 1.0),
            coherence: (self.coherence * other.coherence).clamp(0.0, 1.0),
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
            precision: (self.precision + other.precision).clamp(0.0, 1.0),
            salience: (self.salience + other.salience).clamp(0.0, 1.0),
            coherence: (self.coherence + other.coherence).clamp(0.0, 1.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_clamps_values() {
        let s = Score::new_extended(2.0, -1.0, -5.0, -1.0, 2.0, -1.0, 3.0);
        assert_eq!(s.confidence, 1.0);
        assert_eq!(s.novelty, 0.0);
        assert_eq!(s.utility, 0.0);
        assert_eq!(s.reputation, 0.0);
        assert_eq!(s.precision, 1.0);
        assert_eq!(s.salience, 0.0);
        assert_eq!(s.coherence, 1.0);
    }

    #[test]
    fn zero_effective_when_no_confidence() {
        let s = Score::new(0.0, 1.0, 10.0, 5.0);
        assert_eq!(s.effective(), 0.0);
    }

    #[test]
    fn effective_scales_with_all_axes() {
        // confidence=1, novelty=0, utility=0, reputation=1, salience=0, coherence=0
        // → 1 × 1 × 1 × 1 × 1 × 1 = 1
        let s = Score::new(1.0, 0.0, 0.0, 1.0);
        assert!((s.effective() - 1.0).abs() < 1e-6);

        // confidence=0.5, novelty=1, utility=1, reputation=2, salience=1, coherence=1
        // → 0.5 × 2 × 2 × 2 × 1 × 1 = 4
        let s = Score::new_extended(0.5, 1.0, 1.0, 2.0, 0.0, 1.0, 1.0);
        assert!((s.effective() - 4.0).abs() < 1e-6);
    }

    #[test]
    fn zero_extended_axes_preserve_legacy_effective_score() {
        let s = Score::new_extended(0.5, 1.0, 1.0, 2.0, 0.0, 0.0, 0.0);
        assert!((s.effective() - 4.0).abs() < 1e-6);
    }

    #[test]
    fn serde_defaults_missing_extended_axes_to_zero() {
        let json = r#"{
            "confidence": 0.25,
            "novelty": 0.5,
            "utility": 1.0,
            "reputation": 2.0
        }"#;
        let score: Score = serde_json::from_str(json).unwrap();
        assert_eq!(score.precision, 0.0);
        assert_eq!(score.salience, 0.0);
        assert_eq!(score.coherence, 0.0);
    }

    #[test]
    fn precision_does_not_change_effective_score() {
        let low_precision = Score::new_extended(0.75, 0.25, 0.5, 1.5, 0.0, 0.5, 0.5);
        let high_precision = Score::new_extended(0.75, 0.25, 0.5, 1.5, 1.0, 0.5, 0.5);
        assert!((low_precision.effective() - high_precision.effective()).abs() < 1e-6);
    }

    #[test]
    fn multiplication_scales_independently() {
        let a = Score::new_extended(0.8, 0.5, 2.0, 1.5, 0.8, 0.6, 0.5);
        let modifier = Score::new_extended(1.0, 1.0, 0.5, 2.0, 0.5, 0.5, 0.25);
        let product = a * modifier;
        assert!((product.confidence - 0.8).abs() < 1e-6);
        assert!((product.utility - 1.0).abs() < 1e-6);
        assert!((product.reputation - 3.0).abs() < 1e-6);
        assert!((product.precision - 0.4).abs() < 1e-6);
        assert!((product.salience - 0.3).abs() < 1e-6);
        assert!((product.coherence - 0.125).abs() < 1e-6);
    }

    #[test]
    fn addition_aggregates_evidence() {
        let a = Score::new_extended(0.3, 0.4, 1.0, 1.0, 0.2, 0.3, 0.4);
        let b = Score::new_extended(0.4, 0.4, 2.0, 0.5, 0.5, 0.6, 0.7);
        let sum = a + b;
        assert!((sum.confidence - 0.7).abs() < 1e-6);
        assert!((sum.novelty - 0.8).abs() < 1e-6);
        assert!((sum.utility - 3.0).abs() < 1e-6);
        assert!((sum.reputation - 1.5).abs() < 1e-6);
        assert!((sum.precision - 0.7).abs() < 1e-6);
        assert!((sum.salience - 0.9).abs() < 1e-6);
        assert_eq!(sum.coherence, 1.0);
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
        let high = Score::new_extended(0.9, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0); // effective = 0.9
        assert!(!low.exceeds(0.5));
        assert!(high.exceeds(0.5));
    }
}
