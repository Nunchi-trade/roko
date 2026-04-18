//! Time-based decay for signals.
//!
//! Every signal decays. Pheromones fade over hours; episodes become less
//! relevant over weeks; playbook rules age out of playbooks. The [`Decay`]
//! type unifies all of these: it's a function that takes an age and returns
//! a weight multiplier in \[0..1\].

use serde::{Deserialize, Serialize};

fn finite_weight(weight: f32) -> f32 {
    if weight.is_finite() {
        weight.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// How a signal's weight diminishes over time.
///
/// `Decay::apply(age_ms)` returns a multiplier in `[0.0, 1.0]` that scales
/// the signal's score. A fresh signal has multiplier `1.0`; a fully-decayed
/// signal has multiplier `0.0`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Decay {
    /// No decay — signal weight is permanent (use for config, schemas, identity).
    None,

    /// Exponential half-life: `weight = 0.5 ^ (age / half_life_ms)`.
    ///
    /// Matches pheromone decay in the agent-chain design:
    /// - THREAT: `half_life_ms = 2 * 3600 * 1000` (2 hours)
    /// - OPPORTUNITY: `half_life_ms = 4 * 3600 * 1000` (4 hours)
    /// - WISDOM: `half_life_ms = 24 * 3600 * 1000` (24 hours)
    HalfLife {
        /// Milliseconds for weight to halve.
        half_life_ms: u64,
    },

    /// Hard cutoff: full weight until `ttl_ms`, then zero.
    /// Useful for signals with strict time windows (offers, bounties).
    Ttl {
        /// Milliseconds of full validity before weight drops to zero.
        ttl_ms: u64,
    },

    /// Ebbinghaus forgetting curve: `weight = exp(-age / (strength * scale_ms))`.
    ///
    /// Matches psychological memory decay; higher `strength` = longer retention.
    /// `strength` is in `[0..∞)`; `scale_ms` controls the overall time scale.
    Ebbinghaus {
        /// Retention multiplier. Higher = signal persists longer.
        strength: f32,
        /// Base time unit in milliseconds.
        scale_ms: u64,
    },
}

impl Decay {
    /// Returns `true` when this decay function's parameters are finite.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        match self {
            Self::None | Self::HalfLife { .. } | Self::Ttl { .. } => true,
            Self::Ebbinghaus { strength, .. } => strength.is_finite(),
        }
    }

    /// Apply decay to get a weight multiplier at the given age (milliseconds since emission).
    ///
    /// Clamped to `[0.0, 1.0]`. Negative ages (clock skew) return `1.0`.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn apply(&self, age_ms: i64) -> f32 {
        if age_ms <= 0 {
            return 1.0;
        }
        let age_ms = age_ms as f32;
        match self {
            Self::None => 1.0,
            Self::HalfLife { half_life_ms } => {
                if *half_life_ms == 0 {
                    return 0.0;
                }
                let hl = *half_life_ms as f32;
                finite_weight((0.5_f32).powf(age_ms / hl))
            }
            Self::Ttl { ttl_ms } => {
                if age_ms >= *ttl_ms as f32 {
                    0.0
                } else {
                    1.0
                }
            }
            Self::Ebbinghaus { strength, scale_ms } => {
                if *scale_ms == 0 || !strength.is_finite() || *strength <= 0.0 {
                    return 0.0;
                }
                let scale = (*strength) * (*scale_ms as f32);
                finite_weight((-age_ms / scale).exp())
            }
        }
    }

    /// Is this signal still meaningfully alive (weight > threshold)?
    #[must_use]
    pub fn is_alive(&self, age_ms: i64, threshold: f32) -> bool {
        threshold.is_finite() && self.apply(age_ms) > threshold
    }

    // ─── Convenience constructors matching agent-chain pheromone half-lives ────

    /// THREAT pheromone half-life (2 hours).
    pub const THREAT: Self = Self::HalfLife {
        half_life_ms: 7_200_000,
    };
    /// OPPORTUNITY pheromone half-life (4 hours).
    pub const OPPORTUNITY: Self = Self::HalfLife {
        half_life_ms: 14_400_000,
    };
    /// WISDOM pheromone half-life (24 hours).
    pub const WISDOM: Self = Self::HalfLife {
        half_life_ms: 86_400_000,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_never_decays() {
        assert_eq!(Decay::None.apply(0), 1.0);
        assert_eq!(Decay::None.apply(i64::MAX), 1.0);
    }

    #[test]
    fn half_life_halves_at_hl() {
        let d = Decay::HalfLife { half_life_ms: 1000 };
        assert!((d.apply(0) - 1.0).abs() < 1e-6);
        assert!((d.apply(1000) - 0.5).abs() < 1e-6);
        assert!((d.apply(2000) - 0.25).abs() < 1e-6);
        assert!((d.apply(3000) - 0.125).abs() < 1e-6);
    }

    #[test]
    fn half_life_approaches_zero() {
        let d = Decay::HalfLife { half_life_ms: 100 };
        // After 20 half-lives, should be essentially zero
        assert!(d.apply(2000) < 1e-5);
    }

    #[test]
    fn ttl_is_step_function() {
        let d = Decay::Ttl { ttl_ms: 5000 };
        assert_eq!(d.apply(0), 1.0);
        assert_eq!(d.apply(4999), 1.0);
        assert_eq!(d.apply(5000), 0.0);
        assert_eq!(d.apply(10_000), 0.0);
    }

    #[test]
    fn ebbinghaus_decays_exponentially() {
        let d = Decay::Ebbinghaus {
            strength: 1.0,
            scale_ms: 1000,
        };
        assert!((d.apply(0) - 1.0).abs() < 1e-6);
        // at age=scale, weight = 1/e ≈ 0.368
        assert!((d.apply(1000) - (-1.0_f32).exp()).abs() < 1e-6);
    }

    #[test]
    fn negative_age_returns_one() {
        let d = Decay::HalfLife { half_life_ms: 100 };
        assert_eq!(d.apply(-500), 1.0);
    }

    #[test]
    fn is_alive_threshold() {
        let d = Decay::HalfLife { half_life_ms: 1000 };
        assert!(d.is_alive(500, 0.5)); // weight ~0.707 > 0.5
        assert!(!d.is_alive(2500, 0.5)); // weight ~0.177 < 0.5
    }

    #[test]
    fn pheromone_constants() {
        // THREAT (2h) at 1h should be ~0.707
        let one_hour_ms = 3_600_000_i64;
        let w = Decay::THREAT.apply(one_hour_ms);
        assert!((w - 0.70710677).abs() < 1e-5);
    }

    #[test]
    fn zero_half_life_decays_immediately() {
        let d = Decay::HalfLife { half_life_ms: 0 };
        assert_eq!(d.apply(1), 0.0);
    }

    #[test]
    fn non_finite_ebbinghaus_strength_decays_to_zero() {
        let d = Decay::Ebbinghaus {
            strength: f32::NAN,
            scale_ms: 1000,
        };
        assert!(!d.is_finite());
        assert_eq!(d.apply(1000), 0.0);
    }

    #[test]
    fn is_alive_rejects_non_finite_thresholds() {
        let d = Decay::HalfLife { half_life_ms: 1000 };
        assert!(!d.is_alive(100, f32::NAN));
        assert!(!d.is_alive(100, f32::INFINITY));
    }
}
