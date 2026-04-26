//! Catalyst scoring for downstream-impact-aware ranking.

use crate::{Context, Engram, Score};
use crate::traits::Score as ScoreTrait;
use std::sync::Arc;

/// Observed downstream impact for one signal.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CatalystImpactSummary {
    /// Normalized downstream impact in `[0, 1]`.
    pub downstream_impact: f32,
    /// Count of observed downstream reuses / derivatives.
    pub reuse_count: usize,
    /// Count of confirmations or integrations attributed to this signal.
    pub confirmation_count: usize,
    /// Confidence in the impact estimate.
    pub confidence: f32,
}

/// Read-only downstream-impact source used by [`CatalystScorer`].
pub trait CatalystSignalSource: Send + Sync {
    /// Return the current impact estimate for `signal` in `ctx`.
    fn impact(&self, signal: &Engram, ctx: &Context) -> CatalystImpactSummary;
}

/// Score signals by how strongly they catalyze useful downstream work.
pub struct CatalystScorer {
    source: Arc<dyn CatalystSignalSource>,
    lineage_weight: f32,
    confirmation_weight: f32,
}

impl CatalystScorer {
    /// Construct a catalyst scorer with conservative defaults.
    #[must_use]
    pub fn new(source: Arc<dyn CatalystSignalSource>) -> Self {
        Self {
            source,
            lineage_weight: 0.15,
            confirmation_weight: 0.2,
        }
    }

    /// Override how much lineage breadth contributes to utility.
    #[must_use]
    pub const fn with_lineage_weight(mut self, lineage_weight: f32) -> Self {
        self.lineage_weight = lineage_weight;
        self
    }

    /// Override how much confirmations contribute to utility.
    #[must_use]
    pub const fn with_confirmation_weight(mut self, confirmation_weight: f32) -> Self {
        self.confirmation_weight = confirmation_weight;
        self
    }
}

impl ScoreTrait for CatalystScorer {
    fn score(&self, signal: &Engram, ctx: &Context) -> Score {
        let summary = self.source.impact(signal, ctx);
        let lineage_signal = (signal.lineage.len() as f32 / 8.0).min(1.0);
        let reuse_signal = (summary.reuse_count as f32 / 8.0).min(1.0);
        let confirmation_signal = (summary.confirmation_count as f32 / 8.0).min(1.0);
        let confidence = summary.confidence.clamp(0.0, 1.0);
        let impact = summary.downstream_impact.clamp(0.0, 1.0);
        let catalytic_utility = (impact * (0.7 + 0.3 * confidence)
            + lineage_signal * self.lineage_weight
            + confirmation_signal * self.confirmation_weight
            + reuse_signal * 0.15)
            .clamp(0.0, 1.0);

        Score::new_extended(
            0.0,
            0.0,
            catalytic_utility,
            0.0,
            confidence,
            impact.max(reuse_signal).max(confirmation_signal),
            confidence.max((0.4 + 0.6 * confirmation_signal).clamp(0.0, 1.0)),
        )
    }

    fn name(&self) -> &'static str {
        "catalyst_scorer"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Body, Kind};

    #[derive(Clone, Copy)]
    struct FixedSource(CatalystImpactSummary);

    impl CatalystSignalSource for FixedSource {
        fn impact(&self, _signal: &Engram, _ctx: &Context) -> CatalystImpactSummary {
            self.0
        }
    }

    fn signal() -> Engram {
        Engram::builder(Kind::PromptSection)
            .body(Body::text("keep the verification context"))
            .build()
    }

    #[test]
    fn catalyst_scorer_returns_utility_and_salience_from_impact() {
        let scorer = CatalystScorer::new(Arc::new(FixedSource(CatalystImpactSummary {
            downstream_impact: 0.8,
            reuse_count: 6,
            confirmation_count: 4,
            confidence: 0.75,
        })));

        let score = scorer.score(&signal(), &Context::now());
        assert!(score.utility > 0.7, "utility={}", score.utility);
        assert!(score.salience >= 0.8, "salience={}", score.salience);
        assert!(score.precision >= 0.75, "precision={}", score.precision);
    }

    #[test]
    fn catalyst_scorer_stays_quiet_without_observed_impact() {
        let scorer = CatalystScorer::new(Arc::new(FixedSource(CatalystImpactSummary::default())));

        let score = scorer.score(&signal(), &Context::now());
        assert_eq!(score.utility, 0.0);
        assert_eq!(score.salience, 0.0);
        assert_eq!(score.precision, 0.0);
    }
}
