//! Operating frequency bands for 3-speed cognition.
//!
//! These frequencies name the intended cadence of agent behavior:
//! - `Gamma`: reactive, ~10s
//! - `Theta`: strategic, ~2-5min
//! - `Delta`: consolidation, ~30min+

use bardo_primitives::tier::InferenceTier;
use serde::{Deserialize, Serialize};

/// Cognitive operating frequency for agent work.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatingFrequency {
    /// Reactive mode: perceive, retrieve, act.
    ///
    /// Tool calls, cache lookups, and signal routing.
    Gamma,
    /// Strategic mode: re-plan, update goals, evaluate progress.
    ///
    /// Periodic step-back / course-correction passes.
    Theta,
    /// Consolidation mode: replay, distill, meta-cognate.
    ///
    /// Slow learning and knowledge consolidation.
    Delta,
}

impl OperatingFrequency {
    /// Map to the existing inference tier model.
    #[must_use]
    pub const fn inference_tier(self) -> InferenceTier {
        match self {
            Self::Gamma => InferenceTier::T0,
            Self::Theta => InferenceTier::T1,
            Self::Delta => InferenceTier::T2,
        }
    }
}

impl From<OperatingFrequency> for InferenceTier {
    fn from(value: OperatingFrequency) -> Self {
        value.inference_tier()
    }
}

impl From<InferenceTier> for OperatingFrequency {
    fn from(value: InferenceTier) -> Self {
        match value {
            InferenceTier::T0 => Self::Gamma,
            InferenceTier::T1 => Self::Theta,
            InferenceTier::T2 => Self::Delta,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OperatingFrequency;
    use bardo_primitives::tier::InferenceTier;

    #[test]
    fn maps_to_inference_tiers() {
        assert_eq!(OperatingFrequency::Gamma.inference_tier(), InferenceTier::T0);
        assert_eq!(OperatingFrequency::Theta.inference_tier(), InferenceTier::T1);
        assert_eq!(OperatingFrequency::Delta.inference_tier(), InferenceTier::T2);
    }

    #[test]
    fn round_trips_from_inference_tiers() {
        assert_eq!(OperatingFrequency::from(InferenceTier::T0), OperatingFrequency::Gamma);
        assert_eq!(OperatingFrequency::from(InferenceTier::T1), OperatingFrequency::Theta);
        assert_eq!(OperatingFrequency::from(InferenceTier::T2), OperatingFrequency::Delta);
    }
}
