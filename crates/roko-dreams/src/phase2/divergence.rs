//! Phase 2 divergence-analysis stubs.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Divergence quantification across a fleet of dream-enabled agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DivergenceMetrics {
    /// Jensen-Shannon divergence between agent knowledge distributions.
    pub knowledge_jsd: f64,
    /// Mean pairwise HDC similarity for the fleet's top knowledge entries.
    pub knowledge_overlap: f64,
    /// Strategy entropy across playbook heuristics.
    pub strategy_entropy: f64,
    /// Mean HDC distance of dream insights from the collective centroid.
    pub insight_novelty: f64,
    /// Fraction of each agent's knowledge that is unique to that agent.
    pub mean_uniqueness_fraction: f64,
}

impl DivergenceMetrics {
    /// Construct a divergence snapshot.
    #[must_use]
    pub const fn new(
        knowledge_jsd: f64,
        knowledge_overlap: f64,
        strategy_entropy: f64,
        insight_novelty: f64,
        mean_uniqueness_fraction: f64,
    ) -> Self {
        Self {
            knowledge_jsd,
            knowledge_overlap,
            strategy_entropy,
            insight_novelty,
            mean_uniqueness_fraction,
        }
    }
}

/// Target divergence band used to keep a fleet out of monoculture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DivergenceTargets {
    /// Target Jensen-Shannon divergence range.
    pub target_jsd_range: (f64, f64),
    /// Target knowledge-overlap range.
    pub target_overlap_range: (f64, f64),
    /// Minimum strategy entropy across agents.
    pub min_strategy_entropy: f64,
    /// Minimum insight novelty required for healthy divergence.
    pub min_insight_novelty: f64,
}

impl Default for DivergenceTargets {
    fn default() -> Self {
        Self {
            target_jsd_range: (0.20, 0.60),
            target_overlap_range: (0.35, 0.65),
            min_strategy_entropy: 2.0,
            min_insight_novelty: 0.25,
        }
    }
}

impl DivergenceTargets {
    /// Construct the documented default target band.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            target_jsd_range: (0.20, 0.60),
            target_overlap_range: (0.35, 0.65),
            min_strategy_entropy: 2.0,
            min_insight_novelty: 0.25,
        }
    }

    /// Check whether the supplied metrics fall inside the target band.
    #[must_use]
    pub fn is_within_targets(&self, metrics: &DivergenceMetrics) -> bool {
        metrics.knowledge_jsd >= self.target_jsd_range.0
            && metrics.knowledge_jsd <= self.target_jsd_range.1
            && metrics.knowledge_overlap >= self.target_overlap_range.0
            && metrics.knowledge_overlap <= self.target_overlap_range.1
            && metrics.strategy_entropy >= self.min_strategy_entropy
            && metrics.insight_novelty >= self.min_insight_novelty
    }
}
