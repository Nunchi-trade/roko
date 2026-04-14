//! Collective-intelligence policy hooks driven by C-Factor summaries.

use crate::{Body, Context, Engram, Kind, Policy, Provenance, Score};
use std::sync::Arc;

/// Compact collective-intelligence summary for policy evaluation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CFactorSummary {
    /// Overall composite score in `[0, 1]`.
    pub overall: f64,
    /// Signed trend signal. Negative means degradation.
    pub trend: f64,
    /// Fractional regression against the trailing window.
    pub regression_drop: f64,
    /// Gate-pass component from the latest snapshot.
    pub gate_pass_rate: f64,
    /// Turn-taking-equality component from the latest snapshot.
    pub turn_taking_equality: f64,
    /// Social-sensitivity component from the latest snapshot.
    pub social_sensitivity: f64,
    /// Diversity / specialization coverage component.
    pub task_diversity_coverage: f64,
    /// Number of episodes behind the snapshot.
    pub episode_count: usize,
    /// Most positive contributors in the current window.
    pub top_positive_contributors: Vec<String>,
    /// Most negative contributors in the current window.
    pub top_negative_contributors: Vec<String>,
}

/// Read-only source for the latest collective-intelligence summary.
pub trait CFactorSource: Send + Sync {
    /// Return the current collective-intelligence summary.
    fn summary(&self) -> Option<CFactorSummary>;
}

/// Policy that emits coordination warnings and strengths from C-Factor signals.
pub struct CFactorPolicy {
    source: Arc<dyn CFactorSource>,
    min_episode_count: usize,
    regression_threshold: f64,
    low_overall_threshold: f64,
    coordination_threshold: f64,
}

impl CFactorPolicy {
    /// Construct a C-Factor policy with conservative defaults.
    #[must_use]
    pub fn new(source: Arc<dyn CFactorSource>) -> Self {
        Self {
            source,
            min_episode_count: 8,
            regression_threshold: 0.08,
            low_overall_threshold: 0.45,
            coordination_threshold: 0.4,
        }
    }

    /// Require at least this many episodes before emitting interventions.
    #[must_use]
    pub const fn with_min_episode_count(mut self, min_episode_count: usize) -> Self {
        self.min_episode_count = min_episode_count;
        self
    }
}

impl Policy for CFactorPolicy {
    fn decide(&self, _stream: &[Engram], _ctx: &Context) -> Vec<Engram> {
        let Some(summary) = self.source.summary() else {
            return Vec::new();
        };
        if summary.episode_count < self.min_episode_count {
            return Vec::new();
        }

        let mut outputs = Vec::new();
        if summary.regression_drop >= self.regression_threshold || summary.trend < -0.05 {
            outputs.push(
                Engram::builder(Kind::Insight)
                    .body(Body::text(format!(
                        "Collective calibration is regressing: C-Factor {:.2}, regression drop {:.1}% across {} episodes. Tighten coordination and bias toward stronger collective scaffolds.",
                        summary.overall,
                        summary.regression_drop * 100.0,
                        summary.episode_count
                    )))
                    .provenance(Provenance::trusted("cfactor_policy"))
                    .score(Score::new_extended(0.85, 0.25, 0.45, 1.0, 0.8, 0.8, 0.85))
                    .tag("policy", "cfactor")
                    .tag("alert_kind", "regression")
                    .build(),
            );
        }

        if summary.overall <= self.low_overall_threshold
            || summary.turn_taking_equality <= self.coordination_threshold
            || summary.social_sensitivity <= self.coordination_threshold
        {
            outputs.push(
                Engram::builder(Kind::Insight)
                    .body(Body::text(format!(
                        "Collective coordination is weak: overall {:.2}, turn-taking {:.2}, social sensitivity {:.2}, diversity {:.2}. Prefer clearer handoffs, narrower roles, and explicit reuse of prior outputs.",
                        summary.overall,
                        summary.turn_taking_equality,
                        summary.social_sensitivity,
                        summary.task_diversity_coverage
                    )))
                    .provenance(Provenance::trusted("cfactor_policy"))
                    .score(Score::new_extended(0.8, 0.2, 0.4, 1.0, 0.75, 0.7, 0.8))
                    .tag("policy", "cfactor")
                    .tag("alert_kind", "coordination")
                    .build(),
            );
        }

        if summary.overall >= 0.7 && !summary.top_positive_contributors.is_empty() {
            outputs.push(
                Engram::builder(Kind::Insight)
                    .body(Body::text(format!(
                        "Collective intelligence is compounding: C-Factor {:.2}. Preserve the current high-yield collaboration pattern around {}.",
                        summary.overall,
                        summary.top_positive_contributors.join(", ")
                    )))
                    .provenance(Provenance::trusted("cfactor_policy"))
                    .score(Score::new_extended(0.75, 0.15, 0.5, 1.0, 0.7, 0.65, 0.8))
                    .tag("policy", "cfactor")
                    .tag("alert_kind", "strength")
                    .build(),
            );
        }

        outputs
    }

    fn name(&self) -> &str {
        "cfactor_policy"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct FixedSource(Option<CFactorSummary>);

    impl CFactorSource for FixedSource {
        fn summary(&self) -> Option<CFactorSummary> {
            self.0.clone()
        }
    }

    #[test]
    fn cfactor_policy_emits_regression_and_coordination_alerts() {
        let policy = CFactorPolicy::new(Arc::new(FixedSource(Some(CFactorSummary {
            overall: 0.38,
            trend: -0.1,
            regression_drop: 0.14,
            gate_pass_rate: 0.5,
            turn_taking_equality: 0.31,
            social_sensitivity: 0.28,
            task_diversity_coverage: 0.42,
            episode_count: 18,
            top_positive_contributors: Vec::new(),
            top_negative_contributors: vec!["reviewer".into()],
        }))));

        let outputs = policy.decide(&[], &Context::now());
        assert_eq!(outputs.len(), 2);
        assert!(
            outputs
                .iter()
                .any(|engram| engram.tag("alert_kind") == Some("regression"))
        );
        assert!(
            outputs
                .iter()
                .any(|engram| engram.tag("alert_kind") == Some("coordination"))
        );
    }

    #[test]
    fn cfactor_policy_emits_strength_signal_for_healthy_collective() {
        let policy = CFactorPolicy::new(Arc::new(FixedSource(Some(CFactorSummary {
            overall: 0.76,
            trend: 0.08,
            regression_drop: 0.0,
            gate_pass_rate: 0.8,
            turn_taking_equality: 0.74,
            social_sensitivity: 0.71,
            task_diversity_coverage: 0.63,
            episode_count: 24,
            top_positive_contributors: vec!["implementer=+0.120".into()],
            top_negative_contributors: Vec::new(),
        }))));

        let outputs = policy.decide(&[], &Context::now());
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].tag("alert_kind"), Some("strength"));
    }
}
