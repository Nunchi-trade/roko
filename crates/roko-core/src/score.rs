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

fn finite_unit_interval(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn finite_non_negative(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

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
    pub fn new(confidence: f32, novelty: f32, utility: f32, reputation: f32) -> Self {
        Self {
            confidence: finite_unit_interval(confidence),
            novelty: finite_unit_interval(novelty),
            utility: finite_non_negative(utility),
            reputation: finite_non_negative(reputation),
            precision: 0.0,
            salience: 0.0,
            coherence: 0.0,
        }
    }

    /// Construct a `Score` with the extended axes populated explicitly.
    #[must_use]
    pub fn new_extended(
        confidence: f32,
        novelty: f32,
        utility: f32,
        reputation: f32,
        precision: f32,
        salience: f32,
        coherence: f32,
    ) -> Self {
        Self {
            confidence: finite_unit_interval(confidence),
            novelty: finite_unit_interval(novelty),
            utility: finite_non_negative(utility),
            reputation: finite_non_negative(reputation),
            precision: finite_unit_interval(precision),
            salience: finite_unit_interval(salience),
            coherence: finite_unit_interval(coherence),
        }
    }

    /// Returns `true` when every axis is finite.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.confidence.is_finite()
            && self.novelty.is_finite()
            && self.utility.is_finite()
            && self.reputation.is_finite()
            && self.precision.is_finite()
            && self.salience.is_finite()
            && self.coherence.is_finite()
    }

    /// Scalar effective score combining the primary axes plus salience and coherence.
    ///
    /// # Formula (6-factor)
    ///
    /// ```text
    /// effective = confidence
    ///           × (1 + novelty)
    ///           × (1 + utility)
    ///           × reputation
    ///           × salience_factor
    ///           × coherence_factor
    /// ```
    ///
    /// where `salience_factor` and `coherence_factor` are each `0.5 + 0.5 × axis`
    /// when the axis is non-zero, or `1.0` when zero (opt-in soft damping).
    ///
    /// # Design rationale
    ///
    /// The original doc spec (doc 03-score-7-axis-appraisal.md) described a
    /// 4-factor formula: `confidence × (1 + novelty) × (1 + utility) × reputation`.
    /// This implementation **extends** the spec with two additional soft factors
    /// (salience, coherence) because a 7-axis score type that ignores 3 of its
    /// axes in the scalar reduction wastes information. The extension is
    /// backward-compatible: when salience and coherence are zero (the default
    /// for `Score::new()`), the 6-factor formula reduces exactly to the
    /// 4-factor spec. Precision remains excluded by design -- it describes
    /// applicability narrowness, not quality, and is consumed separately by
    /// routers that need specificity ranking.
    ///
    /// # Properties
    ///
    /// - zero confidence → zero effective score (false positives are worthless)
    /// - novelty and utility act as multipliers (additive bonuses to 1.0)
    /// - reputation directly scales the result
    /// - salience and coherence softly damp or boost the final ranking
    /// - precision is tracked separately and does not affect the scalar score
    #[must_use]
    pub fn effective(&self) -> f32 {
        if !self.is_finite() {
            return 0.0;
        }
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
        finite_non_negative(
            self.confidence
                * (1.0 + self.novelty)
                * (1.0 + self.utility)
                * self.reputation
                * salience_factor
                * coherence_factor,
        )
    }

    /// Is this score above the given threshold on the effective axis?
    #[must_use]
    pub fn exceeds(&self, threshold: f32) -> bool {
        threshold.is_finite() && self.effective() > threshold
    }

    /// Compute a score from a confidence value alone, using neutral reputation.
    #[must_use]
    pub fn from_confidence(confidence: f32) -> Self {
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
            confidence: finite_unit_interval(self.confidence * other.confidence),
            novelty: finite_unit_interval(self.novelty * other.novelty),
            utility: finite_non_negative(self.utility * other.utility),
            reputation: finite_non_negative(self.reputation * other.reputation),
            precision: finite_unit_interval(self.precision * other.precision),
            salience: finite_unit_interval(self.salience * other.salience),
            coherence: finite_unit_interval(self.coherence * other.coherence),
        }
    }
}

/// Element-wise addition — aggregates evidence from multiple scorers.
/// Confidence and novelty are clamped to 1.0; utility and reputation accumulate.
impl Add for Score {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self {
            confidence: finite_unit_interval(self.confidence + other.confidence),
            novelty: finite_unit_interval(self.novelty + other.novelty),
            utility: finite_non_negative(self.utility + other.utility),
            reputation: finite_non_negative(self.reputation + other.reputation),
            precision: finite_unit_interval(self.precision + other.precision),
            salience: finite_unit_interval(self.salience + other.salience),
            coherence: finite_unit_interval(self.coherence + other.coherence),
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

    #[test]
    fn constructors_scrub_non_finite_axes() {
        let score = Score::new_extended(
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            f32::INFINITY,
            f32::NAN,
            f32::NEG_INFINITY,
        );
        assert_eq!(score, Score::ZERO);
        assert!(score.is_finite());
    }

    #[test]
    fn effective_returns_zero_for_non_finite_scores() {
        let score = Score {
            confidence: 0.8,
            novelty: 0.2,
            utility: f32::INFINITY,
            reputation: 1.0,
            precision: 0.0,
            salience: 0.0,
            coherence: 0.0,
        };
        assert_eq!(score.effective(), 0.0);
        assert!(!score.is_finite());
    }

    #[test]
    fn exceeds_rejects_non_finite_thresholds() {
        let score = Score::new(0.9, 0.0, 0.0, 1.0);
        assert!(!score.exceeds(f32::NAN));
        assert!(!score.exceeds(f32::INFINITY));
    }
}
