//! Predictive-foraging primitives: calibration-aware scoring and policy hooks.

use crate::{Budget, Context, Engram, Kind, Policy, Provenance, Score, Scorer};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Calibration summary for one `(model, task_category)` pair.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PredictionCalibrationSummary {
    /// Recent empirical accuracy in `[0, 1]`.
    pub recent_accuracy: f64,
    /// Coverage / interval hit rate in `[0, 1]`.
    pub coverage: f64,
    /// Signed mean residual (`predicted - actual`).
    pub mean_bias: f64,
    /// Short-horizon accuracy trend. Negative means degradation.
    pub accuracy_trend: f64,
    /// Number of observations behind the estimate.
    pub sample_count: usize,
    /// Confidence in the estimate, typically `min(sample_count / 200, 1)`.
    pub confidence: f64,
}

impl PredictionCalibrationSummary {
    /// Conservative default when no calibration history exists yet.
    #[must_use]
    pub fn cold_start() -> Self {
        Self {
            recent_accuracy: 0.5,
            coverage: 0.5,
            mean_bias: 0.0,
            accuracy_trend: 0.0,
            sample_count: 0,
            confidence: 0.0,
        }
    }

    #[must_use]
    fn coherence(self) -> f32 {
        (1.0 - self.mean_bias.abs().min(1.0)) as f32
    }
}

/// Read-only calibration source used by predictive scoring / policies.
pub trait PredictionCalibrationSource: Send + Sync {
    /// Return the current calibration summary for `model` in `task_category`.
    fn summary(&self, model: &str, task_category: &str) -> PredictionCalibrationSummary;
}

/// Calibration-aware scorer approximating expected free energy for Engrams.
pub struct PredictiveScorer {
    calibration: Arc<dyn PredictionCalibrationSource>,
    pragmatic_weight: f32,
    token_cost_per_1k: f32,
}

impl PredictiveScorer {
    /// Construct a predictive scorer with the default PRD constants.
    #[must_use]
    pub fn new(calibration: Arc<dyn PredictionCalibrationSource>) -> Self {
        Self {
            calibration,
            pragmatic_weight: 1.0,
            token_cost_per_1k: 0.01,
        }
    }

    /// Override the pragmatic-value weight.
    #[must_use]
    pub const fn with_pragmatic_weight(mut self, pragmatic_weight: f32) -> Self {
        self.pragmatic_weight = pragmatic_weight;
        self
    }

    /// Override the per-1k-token cost coefficient.
    #[must_use]
    pub const fn with_token_cost_per_1k(mut self, token_cost_per_1k: f32) -> Self {
        self.token_cost_per_1k = token_cost_per_1k;
        self
    }

    #[must_use]
    fn pragmatic_value(&self, signal: &Engram, ctx: &Context) -> f32 {
        let base = signal.score.utility.max(0.0);
        let goal_overlap = ctx
            .goal
            .as_deref()
            .map(|goal| overlap_ratio(goal, &body_text(signal)))
            .unwrap_or(0.0);
        let task_overlap = ctx
            .attr("roko.task_text")
            .map(|task| overlap_ratio(task, &body_text(signal)))
            .unwrap_or(0.0);
        (base + goal_overlap.max(task_overlap)).max(0.0)
    }

    #[must_use]
    fn epistemic_value(
        &self,
        signal: &Engram,
        summary: PredictionCalibrationSummary,
        body: &str,
    ) -> f32 {
        let uncertainty = (1.0 - summary.confidence).clamp(0.0, 1.0) as f32;
        let low_accuracy = (1.0 - summary.recent_accuracy).clamp(0.0, 1.0) as f32;
        let warningish = keyword_weight(
            body,
            &[
                "warning",
                "risk",
                "uncertain",
                "verify",
                "counterexample",
                "prediction",
                "error",
                "failure",
                "fallback",
            ],
        );
        (signal.score.novelty.max(0.0)
            + warningish * (0.45 + 0.35 * uncertainty)
            + low_accuracy * 0.20)
            .clamp(0.0, 1.0)
    }

    #[must_use]
    fn token_cost_penalty(&self, signal: &Engram) -> f32 {
        let tokens = Budget::estimate_tokens(signal.body.byte_size()) as f32;
        (tokens / 1000.0) * self.token_cost_per_1k
    }
}

impl Scorer for PredictiveScorer {
    fn score(&self, signal: &Engram, ctx: &Context) -> Score {
        let model = signal
            .tag("model_slug")
            .or_else(|| signal.tag("model"))
            .or_else(|| ctx.attr("roko.model_slug"))
            .unwrap_or(signal.provenance.author.as_str());
        let task_category = signal
            .tag("task_category")
            .or_else(|| ctx.attr("roko.task_category"))
            .unwrap_or("unknown");
        let summary = self.calibration.summary(model, task_category);
        let body = body_text(signal);
        let pragmatic = self.pragmatic_value(signal, ctx);
        let epistemic = self.epistemic_value(signal, summary, &body);
        let salience = (pragmatic * self.pragmatic_weight + epistemic
            - self.token_cost_penalty(signal))
        .clamp(0.0, 1.0);
        let calibration_confidence = if summary.sample_count == 0 {
            1.0
        } else {
            summary.recent_accuracy.clamp(0.0, 1.0) as f32
        };

        Score::new_extended(
            (signal.score.confidence * calibration_confidence).clamp(0.0, 1.0),
            signal.score.novelty.max(epistemic).clamp(0.0, 1.0),
            (signal.score.utility + pragmatic).max(0.0),
            signal.score.reputation,
            summary.coverage.clamp(0.0, 1.0) as f32,
            salience,
            signal.score.coherence.max(summary.coherence()),
        )
    }

    fn name(&self) -> &'static str {
        "predictive_scorer"
    }
}

/// Policy that emits calibration warnings / regime-shift insights.
pub struct PredictionPolicy {
    calibration: Arc<dyn PredictionCalibrationSource>,
    min_samples: usize,
    bias_threshold: f64,
    degradation_threshold: f64,
}

impl PredictionPolicy {
    /// Construct a prediction policy with conservative defaults.
    #[must_use]
    pub fn new(calibration: Arc<dyn PredictionCalibrationSource>) -> Self {
        Self {
            calibration,
            min_samples: 8,
            bias_threshold: 0.15,
            degradation_threshold: 0.05,
        }
    }

    /// Require at least this many samples before emitting interventions.
    #[must_use]
    pub const fn with_min_samples(mut self, min_samples: usize) -> Self {
        self.min_samples = min_samples;
        self
    }

    /// Override the systematic-bias alert threshold.
    #[must_use]
    pub const fn with_bias_threshold(mut self, bias_threshold: f64) -> Self {
        self.bias_threshold = bias_threshold;
        self
    }

    /// Override the degradation alert threshold.
    #[must_use]
    pub const fn with_degradation_threshold(mut self, degradation_threshold: f64) -> Self {
        self.degradation_threshold = degradation_threshold;
        self
    }
}

impl Policy for PredictionPolicy {
    fn decide(&self, stream: &[Engram], ctx: &Context) -> Vec<Engram> {
        let mut seen = BTreeSet::new();
        let mut outputs = Vec::new();

        for signal in stream {
            let model = signal
                .tag("model_slug")
                .or_else(|| signal.tag("model"))
                .or_else(|| ctx.attr("roko.model_slug"))
                .unwrap_or(signal.provenance.author.as_str());
            let category = signal
                .tag("task_category")
                .or_else(|| ctx.attr("roko.task_category"))
                .unwrap_or("unknown");
            if !seen.insert((model.to_string(), category.to_string())) {
                continue;
            }

            let summary = self.calibration.summary(model, category);
            if summary.sample_count < self.min_samples {
                continue;
            }

            if summary.mean_bias.abs() >= self.bias_threshold {
                outputs.push(
                    Engram::builder(Kind::Insight)
                        .body(crate::Body::text(format!(
                            "Prediction calibration drift for {model}/{category}: mean bias {:+.2} over {} runs",
                            summary.mean_bias, summary.sample_count
                        )))
                        .provenance(Provenance::trusted("prediction_policy"))
                        .score(Score::new_extended(
                            0.8,
                            0.3,
                            0.4,
                            1.0,
                            summary.coverage.clamp(0.0, 1.0) as f32,
                            0.75,
                            summary.coherence(),
                        ))
                        .tag("model_slug", model)
                        .tag("task_category", category)
                        .tag("policy", "prediction")
                        .tag("alert_kind", "systematic_bias")
                        .build(),
                );
            }

            if summary.accuracy_trend <= -self.degradation_threshold {
                outputs.push(
                    Engram::builder(Kind::Prediction)
                        .body(crate::Body::text(format!(
                            "Prediction accuracy is degrading for {model}/{category}: trend {:+.2}",
                            summary.accuracy_trend
                        )))
                        .provenance(Provenance::trusted("prediction_policy"))
                        .score(Score::new_extended(
                            0.7,
                            0.5,
                            0.3,
                            1.0,
                            summary.coverage.clamp(0.0, 1.0) as f32,
                            0.82,
                            summary.coherence(),
                        ))
                        .tag("model_slug", model)
                        .tag("task_category", category)
                        .tag("policy", "prediction")
                        .tag("alert_kind", "degrading_accuracy")
                        .build(),
                );
            }
        }

        outputs
    }

    fn name(&self) -> &'static str {
        "prediction_policy"
    }
}

fn body_text(signal: &Engram) -> String {
    match &signal.body {
        crate::Body::Empty => String::new(),
        crate::Body::Text(text) => text.clone(),
        crate::Body::Json(value) => value.to_string(),
        crate::Body::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn overlap_ratio(left: &str, right: &str) -> f32 {
    let left_terms = tokenize(left);
    let right_terms = tokenize(right);
    if left_terms.is_empty() || right_terms.is_empty() {
        return 0.0;
    }
    let overlap = left_terms.intersection(&right_terms).count() as f32;
    overlap / left_terms.len().max(right_terms.len()) as f32
}

fn tokenize(text: &str) -> BTreeSet<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(|token| {
            let normalized = token.trim().to_ascii_lowercase();
            (!normalized.is_empty()).then_some(normalized)
        })
        .collect()
}

fn keyword_weight(text: &str, keywords: &[&str]) -> f32 {
    let lower = text.to_ascii_lowercase();
    keywords
        .iter()
        .any(|keyword| lower.contains(keyword))
        .then_some(1.0)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeCalibration {
        summaries: HashMap<(String, String), PredictionCalibrationSummary>,
    }

    impl FakeCalibration {
        fn with_summary(
            mut self,
            model: &str,
            category: &str,
            summary: PredictionCalibrationSummary,
        ) -> Self {
            self.summaries
                .insert((model.to_string(), category.to_string()), summary);
            self
        }
    }

    impl PredictionCalibrationSource for FakeCalibration {
        fn summary(&self, model: &str, task_category: &str) -> PredictionCalibrationSummary {
            self.summaries
                .get(&(model.to_string(), task_category.to_string()))
                .copied()
                .unwrap_or_else(PredictionCalibrationSummary::cold_start)
        }
    }

    #[test]
    fn predictive_scorer_boosts_warning_sections_when_calibration_is_uncertain() {
        let calibration = Arc::new(FakeCalibration::default().with_summary(
            "claude-sonnet-4-5",
            "implementation",
            PredictionCalibrationSummary {
                recent_accuracy: 0.45,
                coverage: 0.60,
                mean_bias: 0.10,
                accuracy_trend: -0.08,
                sample_count: 32,
                confidence: 0.15,
            },
        ));
        let scorer = PredictiveScorer::new(calibration);
        let signal = Engram::builder(Kind::PromptSection)
            .body(crate::Body::text("Warning: verify assumptions and check likely failure modes"))
            .score(Score::new(0.8, 0.2, 0.2, 1.0))
            .tag("model_slug", "claude-sonnet-4-5")
            .tag("task_category", "implementation")
            .build();
        let ctx = Context::at(0).with_goal("fix compiler failure safely");

        let score = scorer.score(&signal, &ctx);

        assert!(score.salience > 0.5);
        assert!(score.novelty >= 0.2);
        assert!(score.precision > 0.5);
    }

    #[test]
    fn prediction_policy_emits_bias_and_degradation_alerts() {
        let calibration = Arc::new(FakeCalibration::default().with_summary(
            "gpt-5",
            "implementation",
            PredictionCalibrationSummary {
                recent_accuracy: 0.55,
                coverage: 0.70,
                mean_bias: 0.22,
                accuracy_trend: -0.07,
                sample_count: 18,
                confidence: 0.8,
            },
        ));
        let policy = PredictionPolicy::new(calibration);
        let stream = vec![
            Engram::builder(Kind::Prediction)
                .body(crate::Body::text("route coding task"))
                .tag("model_slug", "gpt-5")
                .tag("task_category", "implementation")
                .build(),
        ];

        let outputs = policy.decide(&stream, &Context::at(0));

        assert_eq!(outputs.len(), 2);
        assert!(outputs.iter().any(|engram| engram.tag("alert_kind") == Some("systematic_bias")));
        assert!(outputs
            .iter()
            .any(|engram| engram.tag("alert_kind") == Some("degrading_accuracy")));
    }
}
