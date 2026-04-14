//! Shared affect primitives used across the cognitive stack.

use serde::{Deserialize, Serialize};

/// Normalized Pleasure-Arousal-Dominance vector.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct PadVector {
    /// Pleasure in `[-1.0, 1.0]`.
    pub pleasure: f64,
    /// Arousal in `[-1.0, 1.0]`.
    pub arousal: f64,
    /// Dominance in `[-1.0, 1.0]`.
    pub dominance: f64,
}

impl PadVector {
    /// Construct a normalized PAD vector.
    #[must_use]
    pub const fn new(pleasure: f64, arousal: f64, dominance: f64) -> Self {
        Self {
            pleasure,
            arousal,
            dominance,
        }
    }

    /// Neutral PAD vector.
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

/// Discrete behavioral state derived from PAD plus affect confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BehavioralState {
    /// Baseline active state.
    #[default]
    Engaged,
    /// Repeated failure / uncertainty; escalate or conserve.
    Struggling,
    /// Succeeding cheaply; can stay lean.
    Coasting,
    /// Learning / uncertain but not failing.
    Exploring,
    /// Confident exploitation of known-good patterns.
    Focused,
    /// Low-demand maintenance / consolidation mode.
    Resting,
}

impl BehavioralState {
    /// Classify a behavioral state from the current PAD vector and confidence.
    #[must_use]
    pub fn classify(pad: PadVector, confidence: f64) -> Self {
        let p = pad.pleasure;
        let a = pad.arousal;
        let d = pad.dominance;
        let c = confidence.clamp(0.0, 1.0);

        if pad == PadVector::neutral() {
            return Self::Engaged;
        }
        if c < 0.30 || d < -0.25 || (p < -0.30 && a > 0.30) {
            return Self::Struggling;
        }
        if p > 0.35 && c > 0.65 {
            return Self::Coasting;
        }
        if d > 0.30 && p > 0.25 {
            return Self::Focused;
        }
        if a < -0.20 {
            return Self::Resting;
        }
        if d < 0.10 && p > -0.20 {
            return Self::Exploring;
        }
        Self::Engaged
    }
}

/// First-class affect policy payload consumed by routing and other online decisions.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DaimonPolicy {
    /// Affect-derived confidence hint in `[0.0, 1.0]`.
    pub affect_confidence: f64,
    /// Current discrete behavioral state from the Daimon.
    pub behavioral_state: BehavioralState,
}

impl Default for DaimonPolicy {
    fn default() -> Self {
        Self::new(0.5, BehavioralState::Engaged)
    }
}

impl DaimonPolicy {
    /// Construct a normalized policy snapshot.
    #[must_use]
    pub fn new(affect_confidence: f64, behavioral_state: BehavioralState) -> Self {
        Self {
            affect_confidence: affect_confidence.clamp(0.0, 1.0),
            behavioral_state,
        }
    }
}

/// Optional emotional metadata attached to an Engram.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmotionalTag {
    /// Immediate PAD signal associated with the engram.
    pub pad: PadVector,
    /// Emotional intensity in `[0.0, 1.0]`.
    pub intensity: f32,
    /// Human-readable or machine-generated trigger label.
    pub trigger: String,
    /// Snapshot of the broader affective mood when the engram was created.
    pub mood_snapshot: PadVector,
}

impl EmotionalTag {
    /// Construct an emotional annotation.
    #[must_use]
    pub fn new(
        pad: PadVector,
        intensity: f32,
        trigger: impl Into<String>,
        mood_snapshot: PadVector,
    ) -> Self {
        Self {
            pad: pad.clamped(),
            intensity: intensity.clamp(0.0, 1.0),
            trigger: trigger.into(),
            mood_snapshot: mood_snapshot.clamped(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behavioral_state_classification_matches_thresholds() {
        assert_eq!(
            BehavioralState::classify(PadVector::neutral(), 0.5),
            BehavioralState::Engaged
        );
        assert_eq!(
            BehavioralState::classify(PadVector::new(0.0, 0.0, -0.4), 0.6),
            BehavioralState::Struggling
        );
        assert_eq!(
            BehavioralState::classify(PadVector::new(0.5, 0.0, 0.1), 0.8),
            BehavioralState::Coasting
        );
        assert_eq!(
            BehavioralState::classify(PadVector::new(0.3, 0.0, 0.4), 0.7),
            BehavioralState::Focused
        );
        assert_eq!(
            BehavioralState::classify(PadVector::new(0.0, -0.4, 0.0), 0.5),
            BehavioralState::Resting
        );
        assert_eq!(
            BehavioralState::classify(PadVector::new(0.0, 0.2, 0.0), 0.5),
            BehavioralState::Exploring
        );
    }

    #[test]
    fn pad_similarity_uses_neutral_fallback() {
        assert_eq!(
            PadVector::neutral().cosine_similarity(PadVector::new(1.0, 0.0, 0.0)),
            0.5
        );
    }

    #[test]
    fn emotional_tag_clamps_inputs() {
        let tag = EmotionalTag::new(
            PadVector::new(2.0, -2.0, 0.5),
            2.0,
            "gate_failure",
            PadVector::new(0.1, 0.2, 3.0),
        );
        assert_eq!(tag.pad, PadVector::new(1.0, -1.0, 0.5));
        assert_eq!(tag.intensity, 1.0);
        assert_eq!(tag.mood_snapshot, PadVector::new(0.1, 0.2, 1.0));
    }

    #[test]
    fn daimon_policy_clamps_confidence() {
        let policy = DaimonPolicy::new(1.5, BehavioralState::Focused);
        assert_eq!(policy.affect_confidence, 1.0);
        assert_eq!(policy.behavioral_state, BehavioralState::Focused);
    }
}
