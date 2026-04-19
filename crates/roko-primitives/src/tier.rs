//! Inference tier model: T0/T1/T2 with vitality-aware model selection.

use serde::{Deserialize, Serialize};

/// Error returned when an invalid tier value is parsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TierError(u8);

impl std::fmt::Display for TierError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid inference tier: {}", self.0)
    }
}

impl std::error::Error for TierError {}

/// Three-tier gate for inference spend and latency.
///
/// - `T0`: suppress — heuristics only, no LLM call
/// - `T1`: analyze — light LLM (Haiku-class)
/// - `T2`: deliberate — full LLM (Opus/Sonnet based on vitality)
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum InferenceTier {
    /// Suppress inference entirely. Returns `None` from `TierRouter`.
    T0 = 0,
    /// Light inference. Always routes to Haiku-class model.
    T1 = 1,
    /// Full inference. Routes to Opus above vitality threshold, Sonnet below.
    T2 = 2,
}

impl std::convert::TryFrom<u8> for InferenceTier {
    type Error = TierError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::T0),
            1 => Ok(Self::T1),
            2 => Ok(Self::T2),
            v => Err(TierError(v)),
        }
    }
}

impl From<InferenceTier> for u8 {
    fn from(value: InferenceTier) -> Self {
        value as Self
    }
}

/// Vitality threshold: below this, T2 degrades from Opus to Sonnet (sharp boundary, not fuzzy).
pub const T2_VITALITY_THRESHOLD: f32 = 0.3;

/// Maps an `InferenceTier` + vitality score to a concrete model identifier.
///
/// This is a pure stateless function. All model selection logic lives here.
pub struct TierRouter;

impl TierRouter {
    /// Select a model based on tier and vitality.
    ///
    /// - `T0` → `None` (suppress inference)
    /// - `T1` → `"claude-haiku-4-5"` (regardless of vitality)
    /// - `T2` → `"claude-opus-4-6"` if vitality ≥ 0.3, `"claude-sonnet-4"` if below
    #[must_use]
    pub fn select_model(tier: InferenceTier, vitality: f32) -> Option<&'static str> {
        match tier {
            InferenceTier::T0 => None,
            InferenceTier::T1 => Some("claude-haiku-4-5"),
            InferenceTier::T2 => {
                if vitality >= T2_VITALITY_THRESHOLD {
                    Some("claude-opus-4-6")
                } else {
                    Some("claude-sonnet-4")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use super::{InferenceTier, TierRouter};

    #[test]
    fn t0_suppresses() {
        assert_eq!(TierRouter::select_model(InferenceTier::T0, 1.0), None);
        assert_eq!(TierRouter::select_model(InferenceTier::T0, 0.0), None);
    }

    #[test]
    fn t1_always_haiku() {
        assert_eq!(
            TierRouter::select_model(InferenceTier::T1, 1.0),
            Some("claude-haiku-4-5")
        );
        assert_eq!(
            TierRouter::select_model(InferenceTier::T1, 0.0),
            Some("claude-haiku-4-5")
        );
    }

    #[test]
    fn t2_opus_above_threshold() {
        assert_eq!(
            TierRouter::select_model(InferenceTier::T2, 0.5),
            Some("claude-opus-4-6")
        );
    }

    #[test]
    fn t2_sonnet_below_threshold() {
        assert_eq!(
            TierRouter::select_model(InferenceTier::T2, 0.1),
            Some("claude-sonnet-4")
        );
    }

    #[test]
    fn t2_sharp_threshold_at_0_3() {
        assert_eq!(
            TierRouter::select_model(InferenceTier::T2, 0.3),
            Some("claude-opus-4-6")
        );
        assert_eq!(
            TierRouter::select_model(InferenceTier::T2, 0.299_99),
            Some("claude-sonnet-4")
        );
    }

    #[test]
    fn tier_try_from_valid() {
        assert_eq!(InferenceTier::try_from(0).unwrap(), InferenceTier::T0);
        assert_eq!(InferenceTier::try_from(1).unwrap(), InferenceTier::T1);
        assert_eq!(InferenceTier::try_from(2).unwrap(), InferenceTier::T2);
    }

    #[test]
    fn tier_try_from_invalid() {
        assert!(InferenceTier::try_from(3).is_err());
    }

    #[test]
    fn tier_into_u8() {
        assert_eq!(u8::from(InferenceTier::T0), 0);
        assert_eq!(u8::from(InferenceTier::T1), 1);
        assert_eq!(u8::from(InferenceTier::T2), 2);
    }
}
