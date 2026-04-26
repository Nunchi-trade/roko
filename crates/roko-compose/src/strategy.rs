//! Prompt composition strategy selection.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::AttentionBidder;

/// Minimum bidder observations before `Auto` enables VCG allocation.
pub const DEFAULT_VCG_WARMUP_OBSERVATIONS: u32 = 10;

/// Strategy for allocating prompt token budget across candidate sections.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompositionStrategy {
    /// Select `Vcg` once learned bidder observations are warm; otherwise use
    /// the deterministic density-greedy path.
    #[default]
    Auto,
    /// Deterministic greedy allocation by score density.
    DensityGreedy,
    /// Backward-compatible alias for density-greedy allocation.
    WeightedSum,
    /// VCG-style allocation with payments and displacement diagnostics.
    Vcg,
}

impl CompositionStrategy {
    /// Resolve an explicit strategy from `self`.
    ///
    /// `Auto` uses the minimum observation count across active bidders. Cold
    /// starts stay deterministic; warm bidders activate VCG diagnostics.
    #[must_use]
    pub fn resolve(
        self,
        bidder_observations: &HashMap<AttentionBidder, u32>,
        warmup_observations: u32,
    ) -> Self {
        match self {
            Self::Auto => Self::auto_select(bidder_observations, warmup_observations),
            Self::WeightedSum => Self::DensityGreedy,
            other => other,
        }
    }

    /// Select a strategy from learned bidder observation counts.
    #[must_use]
    pub fn auto_select(
        bidder_observations: &HashMap<AttentionBidder, u32>,
        warmup_observations: u32,
    ) -> Self {
        let min_obs = bidder_observations.values().copied().min().unwrap_or(0);
        if min_obs >= warmup_observations {
            Self::Vcg
        } else {
            Self::DensityGreedy
        }
    }

    /// Whether this strategy resolves to the density-greedy path.
    #[must_use]
    pub const fn is_density_greedy(self) -> bool {
        matches!(self, Self::DensityGreedy | Self::WeightedSum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_selects_density_greedy_when_cold() {
        let observations = HashMap::from([
            (AttentionBidder::TaskContext, 12),
            (AttentionBidder::CodeIntelligence, 3),
        ]);

        assert_eq!(
            CompositionStrategy::auto_select(&observations, DEFAULT_VCG_WARMUP_OBSERVATIONS),
            CompositionStrategy::DensityGreedy
        );
    }

    #[test]
    fn auto_selects_vcg_when_all_active_bidders_are_warm() {
        let observations = HashMap::from([
            (AttentionBidder::TaskContext, 12),
            (AttentionBidder::CodeIntelligence, 10),
        ]);

        assert_eq!(
            CompositionStrategy::auto_select(&observations, DEFAULT_VCG_WARMUP_OBSERVATIONS),
            CompositionStrategy::Vcg
        );
    }
}
