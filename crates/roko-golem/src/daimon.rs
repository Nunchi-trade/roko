//! Daimon subsystem scaffold.
//!
//! Placeholder API only; affect or motivation modeling is not yet implemented.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{GolemSubsystemId, GolemSubsystemSummary, ScaffoldEngine};

/// Normalized PAD affect state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AffectState {
    /// Pleasure dimension in `[-1.0, 1.0]`.
    /// Success pushes this positive; failure pushes it negative.
    pub pleasure: f64,
    /// Arousal dimension in `[-1.0, 1.0]`.
    /// Time pressure and urgency push this positive; idle pushes it negative.
    pub arousal: f64,
    /// Dominance dimension in `[-1.0, 1.0]`.
    /// Agency and control push this positive; blocked or stuck pushes it negative.
    pub dominance: f64,
    /// Last time this affect state was updated.
    pub updated_at: DateTime<Utc>,
}

/// Named PAD octant for logging and dashboard display.
///
/// The octants correspond to the sign of the PAD vector:
/// - `+P+A+D` => `Excited`
/// - `+P+A-D` => `Surprised`
/// - `+P-A+D` => `Confident`
/// - `+P-A-D` => `Relaxed`
/// - `-P+A+D` => `Angry`
/// - `-P+A-D` => `Anxious`
/// - `-P-A+D` => `Bored`
/// - `-P-A-D` => `Depressed`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AffectOctant {
    /// Succeeding under pressure.
    Excited,
    /// Unexpected success.
    Surprised,
    /// Calm, in control, succeeding.
    Confident,
    /// Nothing urgent, things are fine.
    Relaxed,
    /// Frustrated but still trying.
    Angry,
    /// Failing, pressured, no control.
    Anxious,
    /// Nothing happening, agent idle.
    Bored,
    /// Repeated failures, no agency.
    Depressed,
}

impl AffectOctant {
    /// Human-readable label for this octant.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Excited => "Excited",
            Self::Surprised => "Surprised",
            Self::Confident => "Confident",
            Self::Relaxed => "Relaxed",
            Self::Angry => "Angry",
            Self::Anxious => "Anxious",
            Self::Bored => "Bored",
            Self::Depressed => "Depressed",
        }
    }

    /// Resolve a PAD vector to its octant.
    ///
    /// Exact zero vectors are treated as `Relaxed` so the neutral dashboard
    /// state stays readable instead of collapsing to an arbitrary octant.
    #[must_use]
    pub const fn from_pad(pleasure: f64, arousal: f64, dominance: f64) -> Self {
        if pleasure == 0.0 && arousal == 0.0 && dominance == 0.0 {
            return Self::Relaxed;
        }

        let positive_pleasure = !pleasure.is_sign_negative();
        let positive_arousal = !arousal.is_sign_negative();
        let positive_dominance = !dominance.is_sign_negative();

        match (positive_pleasure, positive_arousal, positive_dominance) {
            (true, true, true) => Self::Excited,
            (true, true, false) => Self::Surprised,
            (true, false, true) => Self::Confident,
            (true, false, false) => Self::Relaxed,
            (false, true, true) => Self::Angry,
            (false, true, false) => Self::Anxious,
            (false, false, true) => Self::Bored,
            (false, false, false) => Self::Depressed,
        }
    }
}

impl fmt::Display for AffectOctant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl AffectState {
    /// Resolve this PAD vector to its named octant.
    #[must_use]
    pub fn octant(&self) -> AffectOctant {
        AffectOctant::from_pad(self.pleasure, self.arousal, self.dominance)
    }

    /// Human-readable label for logging and dashboards.
    #[must_use]
    pub fn octant_label(&self) -> &'static str {
        self.octant().label()
    }
}

/// Placeholder daimon engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DaimonEngine;

impl DaimonEngine {
    /// Stable subsystem id.
    pub const ID: GolemSubsystemId = GolemSubsystemId::Daimon;
    /// Human-readable subsystem label.
    pub const LABEL: &'static str = "Daimon";
    /// Static scaffold marker string.
    pub const MARKER: &'static str = "roko-golem scaffold: daimon";

    /// Construct a placeholder daimon engine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Summary metadata for this scaffold subsystem.
    #[must_use]
    pub const fn summary(self) -> GolemSubsystemSummary {
        GolemSubsystemSummary::new(Self::ID, Self::LABEL, Self::MARKER)
    }

    /// Returns a static marker describing scaffold behavior.
    #[must_use]
    pub const fn evaluate(self) -> &'static str {
        Self::MARKER
    }
}

impl ScaffoldEngine for DaimonEngine {
    fn summary(self) -> GolemSubsystemSummary {
        self.summary()
    }
}

#[cfg(test)]
mod tests {
    use super::{AffectOctant, AffectState};
    use chrono::Utc;

    #[test]
    fn maps_all_pad_octants_to_named_states() {
        let now = Utc::now();

        let cases = [
            (1.0, 1.0, 1.0, AffectOctant::Excited),
            (1.0, 1.0, -1.0, AffectOctant::Surprised),
            (1.0, -1.0, 1.0, AffectOctant::Confident),
            (1.0, -1.0, -1.0, AffectOctant::Relaxed),
            (-1.0, 1.0, 1.0, AffectOctant::Angry),
            (-1.0, 1.0, -1.0, AffectOctant::Anxious),
            (-1.0, -1.0, 1.0, AffectOctant::Bored),
            (-1.0, -1.0, -1.0, AffectOctant::Depressed),
        ];

        for (pleasure, arousal, dominance, expected) in cases {
            let state = AffectState {
                pleasure,
                arousal,
                dominance,
                updated_at: now,
            };

            assert_eq!(state.octant(), expected);
            assert_eq!(state.octant_label(), expected.label());
            assert_eq!(expected.to_string(), expected.label());
        }
    }

    #[test]
    fn neutral_vector_defaults_to_relaxed() {
        let state = AffectState {
            pleasure: 0.0,
            arousal: 0.0,
            dominance: 0.0,
            updated_at: Utc::now(),
        };

        assert_eq!(state.octant(), AffectOctant::Relaxed);
    }
}
