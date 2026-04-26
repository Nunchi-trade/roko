//! Collective-intelligence policy hooks driven by C-Factor summaries.
//!
//! The C-Factor implements Woolley et al.'s five process variables for measuring
//! collective intelligence, extended with cohort metrics and learnable weights.
//!
//! Five core process variables:
//! 1. Turn-taking equality   (Pulse authorship entropy on Bus)
//! 2. Social perceptiveness  (peer.prediction vs peer.outcome residuals)
//! 3. Trust calibration      (citation reciprocity + gate survival in Store)
//! 4. Channel openness       (Bus delivery confirmation + subscriber reach)
//! 5. Cognitive diversity    (HDC distance across cohort Engrams)

use crate::{Body, Context, Engram, Kind, React, Provenance, Score};
use std::sync::Arc;

/// The five Woolley process variables measured per cohort.
///
/// This is the spec-aligned struct from `docs/00-architecture/14-c-factor-collective-intelligence.md`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CohortMetrics {
    /// Authorship entropy across Bus messages (0=monopoly, 1=perfectly equal).
    pub turn_taking_entropy: f64,
    /// Mean accuracy of peer predictions vs actual outcomes.
    pub peer_prediction_accuracy: f64,
    /// Ratio of reciprocated citations in shared knowledge.
    pub citation_reciprocity: f64,
    /// Fraction of Bus messages successfully delivered to all subscribers.
    pub delivery_rate: f64,
    /// Average HDC cosine distance between cohort members' Engram vectors.
    pub hdc_diversity: f64,
}

impl CohortMetrics {
    /// Compute the composite C-Factor score given weights.
    pub fn composite(&self, weights: &CohortWeights) -> f64 {
        let raw = weights.turn_taking * self.turn_taking_entropy
            + weights.social_perceptiveness * self.peer_prediction_accuracy
            + weights.trust_calibration * self.citation_reciprocity
            + weights.channel_openness * self.delivery_rate
            + weights.cognitive_diversity * self.hdc_diversity
            + weights.bias;
        raw.clamp(0.0, 1.0)
    }
}

/// Learnable per-variable weights for the C-Factor composite score.
///
/// These can be fit online via gradient descent on cohort performance outcomes.
#[derive(Clone, Debug, PartialEq)]
pub struct CohortWeights {
    /// Weight for turn-taking equality.
    pub turn_taking: f64,
    /// Weight for social perceptiveness.
    pub social_perceptiveness: f64,
    /// Weight for trust calibration (citation reciprocity).
    pub trust_calibration: f64,
    /// Weight for channel openness (delivery rate).
    pub channel_openness: f64,
    /// Weight for cognitive diversity (HDC distance).
    pub cognitive_diversity: f64,
    /// Bias term.
    pub bias: f64,
}

impl Default for CohortWeights {
    /// Equal weights summing to ~1.0 with zero bias (uniform prior).
    fn default() -> Self {
        Self {
            turn_taking: 0.2,
            social_perceptiveness: 0.2,
            trust_calibration: 0.2,
            channel_openness: 0.2,
            cognitive_diversity: 0.2,
            bias: 0.0,
        }
    }
}

impl CohortWeights {
    /// Update weights via simple gradient step given observed outcome.
    ///
    /// `metrics`: the input features (cohort metrics).
    /// `actual_performance`: the observed collective performance (target in [0,1]).
    /// `learning_rate`: step size for gradient update.
    pub fn update(&mut self, metrics: &CohortMetrics, actual_performance: f64, learning_rate: f64) {
        let predicted = metrics.composite(self);
        let error = actual_performance - predicted;

        self.turn_taking += learning_rate * error * metrics.turn_taking_entropy;
        self.social_perceptiveness += learning_rate * error * metrics.peer_prediction_accuracy;
        self.trust_calibration += learning_rate * error * metrics.citation_reciprocity;
        self.channel_openness += learning_rate * error * metrics.delivery_rate;
        self.cognitive_diversity += learning_rate * error * metrics.hdc_diversity;
        self.bias += learning_rate * error;
    }
}

/// Compact collective-intelligence summary for policy evaluation.
///
/// Extends the Woolley variables with operational metrics for richer policy decisions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CFactorSummary {
    /// Overall composite score in `[0, 1]`.
    pub overall: f64,
    /// Signed trend signal. Negative means degradation.
    pub trend: f64,
    /// Fractional regression against the trailing window.
    pub regression_drop: f64,
    /// Verify-pass component from the latest snapshot.
    pub gate_pass_rate: f64,

    // --- Woolley five process variables (spec-aligned names) ---
    /// Turn-taking equality: authorship entropy across Bus.
    pub turn_taking_equality: f64,
    /// Social perceptiveness: peer prediction accuracy.
    pub social_perceptiveness: f64,
    /// Citation reciprocity: trust calibration signal.
    pub citation_reciprocity: f64,
    /// Delivery rate: channel openness signal.
    pub delivery_rate: f64,
    /// HDC diversity: cognitive diversity across cohort.
    pub hdc_diversity: f64,

    /// Number of episodes behind the snapshot.
    pub episode_count: usize,
    /// Most positive contributors in the current window.
    pub top_positive_contributors: Vec<String>,
    /// Most negative contributors in the current window.
    pub top_negative_contributors: Vec<String>,
}

impl CFactorSummary {
    /// Convert to the pure Woolley `CohortMetrics` struct.
    pub fn to_cohort_metrics(&self) -> CohortMetrics {
        CohortMetrics {
            turn_taking_entropy: self.turn_taking_equality,
            peer_prediction_accuracy: self.social_perceptiveness,
            citation_reciprocity: self.citation_reciprocity,
            delivery_rate: self.delivery_rate,
            hdc_diversity: self.hdc_diversity,
        }
    }
}

/// Read-only source for the latest collective-intelligence summary.
pub trait CFactorSource: Send + Sync {
    /// Return the current collective-intelligence summary.
    fn summary(&self) -> Option<CFactorSummary>;
}

/// React that emits coordination warnings and strengths from C-Factor signals.
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

impl React for CFactorPolicy {
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
            || summary.social_perceptiveness <= self.coordination_threshold
        {
            outputs.push(
                Engram::builder(Kind::Insight)
                    .body(Body::text(format!(
                        "Collective coordination is weak: overall {:.2}, turn-taking {:.2}, social perceptiveness {:.2}, HDC diversity {:.2}, citation reciprocity {:.2}, delivery rate {:.2}.",
                        summary.overall,
                        summary.turn_taking_equality,
                        summary.social_perceptiveness,
                        summary.hdc_diversity,
                        summary.citation_reciprocity,
                        summary.delivery_rate,
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
            social_perceptiveness: 0.28,
            citation_reciprocity: 0.45,
            delivery_rate: 0.60,
            hdc_diversity: 0.42,
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
            social_perceptiveness: 0.71,
            citation_reciprocity: 0.68,
            delivery_rate: 0.85,
            hdc_diversity: 0.63,
            episode_count: 24,
            top_positive_contributors: vec!["implementer=+0.120".into()],
            top_negative_contributors: Vec::new(),
        }))));

        let outputs = policy.decide(&[], &Context::now());
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].tag("alert_kind"), Some("strength"));
    }

    #[test]
    fn cohort_metrics_composite_with_equal_weights() {
        let metrics = CohortMetrics {
            turn_taking_entropy: 0.8,
            peer_prediction_accuracy: 0.7,
            citation_reciprocity: 0.6,
            delivery_rate: 0.9,
            hdc_diversity: 0.5,
        };
        let weights = CohortWeights::default(); // equal 0.2 each, bias=0
        let composite = metrics.composite(&weights);

        // Expected: 0.2*(0.8+0.7+0.6+0.9+0.5) = 0.2*3.5 = 0.70
        assert!((composite - 0.70).abs() < 0.001);
    }

    #[test]
    fn cohort_weights_update_reduces_error() {
        let metrics = CohortMetrics {
            turn_taking_entropy: 0.8,
            peer_prediction_accuracy: 0.7,
            citation_reciprocity: 0.6,
            delivery_rate: 0.9,
            hdc_diversity: 0.5,
        };
        let mut weights = CohortWeights::default();
        let target = 0.85;

        let initial_error = (metrics.composite(&weights) - target).abs();

        // Run several updates.
        for _ in 0..20 {
            weights.update(&metrics, target, 0.1);
        }

        let final_error = (metrics.composite(&weights) - target).abs();
        assert!(
            final_error < initial_error,
            "weight update should reduce error: {initial_error} -> {final_error}"
        );
    }

    #[test]
    fn summary_to_cohort_metrics() {
        let summary = CFactorSummary {
            turn_taking_equality: 0.7,
            social_perceptiveness: 0.6,
            citation_reciprocity: 0.5,
            delivery_rate: 0.8,
            hdc_diversity: 0.4,
            ..Default::default()
        };
        let metrics = summary.to_cohort_metrics();
        assert!((metrics.turn_taking_entropy - 0.7).abs() < f64::EPSILON);
        assert!((metrics.peer_prediction_accuracy - 0.6).abs() < f64::EPSILON);
        assert!((metrics.citation_reciprocity - 0.5).abs() < f64::EPSILON);
        assert!((metrics.delivery_rate - 0.8).abs() < f64::EPSILON);
        assert!((metrics.hdc_diversity - 0.4).abs() < f64::EPSILON);
    }
}
