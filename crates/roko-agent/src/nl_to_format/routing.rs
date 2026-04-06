//! Band-aware routing for the NL-to-Format pipeline.
//!
//! Decides whether a given task should use the two-pass NL-to-Format
//! pipeline or direct constrained decoding, based on the task's complexity
//! band and the target model tier.
//!
//! # Routing rules
//!
//! | Complexity | Tier    | Decision      |
//! |------------|---------|---------------|
//! | Complex    | Premium | `TwoPass`       |
//! | Complex    | Standard| `DirectFormat`  |
//! | Complex    | Fast    | `DirectFormat`  |
//! | Standard   | any     | `DirectFormat`  |
//! | Fast       | any     | `DirectFormat`  |
//!
//! Rationale: Premium models on complex tasks benefit from first "thinking
//! aloud" in natural language (chain-of-thought) before committing to a
//! structured format. Simpler tasks or cheaper models do better with
//! direct constrained decoding — the overhead of two passes isn't worth it.

use roko_core::agent::ModelTier;
use roko_core::task::TaskComplexityBand;

/// The routing decision for a given task/model combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingDecision {
    /// Use direct constrained decoding (single pass).
    DirectFormat,
    /// Use the two-pass NL-to-Format pipeline.
    TwoPass,
}

impl RoutingDecision {
    /// Whether this decision uses the two-pass pipeline.
    #[must_use]
    pub const fn is_two_pass(self) -> bool {
        matches!(self, Self::TwoPass)
    }
}

/// Determine whether to use the two-pass NL-to-Format pipeline.
///
/// Returns `true` only when **both** conditions hold:
/// - `complexity` is [`TaskComplexityBand::Complex`]
/// - `tier` is [`ModelTier::Premium`]
///
/// All other combinations return `false` (use direct format).
#[must_use]
pub const fn should_use_two_pass(complexity: TaskComplexityBand, tier: ModelTier) -> bool {
    matches!(
        (complexity, tier),
        (TaskComplexityBand::Complex, ModelTier::Premium)
    )
}

/// Full routing decision for a task/model combination.
///
/// Wraps [`should_use_two_pass`] into the [`RoutingDecision`] enum for
/// pattern-matching callers.
#[must_use]
pub const fn route(complexity: TaskComplexityBand, tier: ModelTier) -> RoutingDecision {
    if should_use_two_pass(complexity, tier) {
        RoutingDecision::TwoPass
    } else {
        RoutingDecision::DirectFormat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complex_premium_routes_to_two_pass() {
        assert!(should_use_two_pass(
            TaskComplexityBand::Complex,
            ModelTier::Premium
        ));
        assert_eq!(
            route(TaskComplexityBand::Complex, ModelTier::Premium),
            RoutingDecision::TwoPass
        );
    }

    #[test]
    fn complex_standard_routes_to_direct() {
        assert!(!should_use_two_pass(
            TaskComplexityBand::Complex,
            ModelTier::Standard
        ));
        assert_eq!(
            route(TaskComplexityBand::Complex, ModelTier::Standard),
            RoutingDecision::DirectFormat
        );
    }

    #[test]
    fn complex_fast_routes_to_direct() {
        assert!(!should_use_two_pass(
            TaskComplexityBand::Complex,
            ModelTier::Fast
        ));
    }

    #[test]
    fn standard_premium_routes_to_direct() {
        assert!(!should_use_two_pass(
            TaskComplexityBand::Standard,
            ModelTier::Premium
        ));
    }

    #[test]
    fn standard_standard_routes_to_direct() {
        assert!(!should_use_two_pass(
            TaskComplexityBand::Standard,
            ModelTier::Standard
        ));
    }

    #[test]
    fn standard_fast_routes_to_direct() {
        assert!(!should_use_two_pass(
            TaskComplexityBand::Standard,
            ModelTier::Fast
        ));
    }

    #[test]
    fn fast_any_tier_routes_to_direct() {
        for tier in [ModelTier::Fast, ModelTier::Standard, ModelTier::Premium] {
            assert!(!should_use_two_pass(TaskComplexityBand::Fast, tier));
            assert_eq!(
                route(TaskComplexityBand::Fast, tier),
                RoutingDecision::DirectFormat
            );
        }
    }

    #[test]
    fn routing_decision_is_two_pass_method() {
        assert!(RoutingDecision::TwoPass.is_two_pass());
        assert!(!RoutingDecision::DirectFormat.is_two_pass());
    }

    #[test]
    fn all_combinations_covered() {
        // Exhaustive: 3 bands x 3 tiers = 9 combos, only 1 is TwoPass
        let mut two_pass_count = 0;
        for band in [
            TaskComplexityBand::Fast,
            TaskComplexityBand::Standard,
            TaskComplexityBand::Complex,
        ] {
            for tier in [ModelTier::Fast, ModelTier::Standard, ModelTier::Premium] {
                if should_use_two_pass(band, tier) {
                    two_pass_count += 1;
                }
            }
        }
        assert_eq!(two_pass_count, 1);
    }
}
