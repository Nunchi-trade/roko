//! Witness verification pipeline for Oracle predictions.
//!
//! TA-04: Before an oracle prediction is consumed by the orchestrator or
//! router, it passes through a witness verification step that checks
//! calibration, staleness, and internal consistency.

use roko_core::{CalibrationTracker, Prediction, ResidualCorrector};

/// Witness verdict: whether a prediction should be consumed as-is, adjusted,
/// or rejected.
#[derive(Debug, Clone, PartialEq)]
pub enum WitnessVerdict {
    /// Prediction passes verification and can be consumed directly.
    Accept,
    /// Prediction is acceptable after applying the given bias correction.
    AdjustAndAccept {
        /// Original predicted value.
        original: f64,
        /// Bias-corrected value.
        corrected: f64,
        /// Reason for the adjustment.
        reason: String,
    },
    /// Prediction is rejected and should not be consumed.
    Reject {
        /// Reason for rejection.
        reason: String,
    },
}

/// Configuration for the witness verifier.
#[derive(Debug, Clone)]
pub struct WitnessConfig {
    /// Minimum acceptable confidence threshold.
    pub min_confidence: f64,
    /// Maximum acceptable staleness in milliseconds.
    pub max_staleness_ms: i64,
    /// Maximum absolute bias before correction kicks in.
    pub bias_correction_threshold: f64,
    /// Maximum absolute bias before outright rejection.
    pub bias_rejection_threshold: f64,
}

impl Default for WitnessConfig {
    fn default() -> Self {
        Self {
            min_confidence: 0.15,
            max_staleness_ms: 3_600_000, // 1 hour
            bias_correction_threshold: 0.1,
            bias_rejection_threshold: 0.5,
        }
    }
}

/// Witness verifier that gates oracle predictions before consumption.
pub struct WitnessVerifier {
    config: WitnessConfig,
}

impl WitnessVerifier {
    /// Create a witness verifier with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: WitnessConfig::default(),
        }
    }

    /// Create a witness verifier with custom configuration.
    #[must_use]
    pub fn with_config(config: WitnessConfig) -> Self {
        Self { config }
    }

    /// Verify a prediction against calibration data and staleness checks.
    ///
    /// Returns a verdict indicating whether the prediction should be consumed,
    /// adjusted, or rejected.
    #[must_use]
    pub fn verify(
        &self,
        prediction: &Prediction,
        _calibration: &CalibrationTracker,
        corrector: &ResidualCorrector,
        now_ms: i64,
    ) -> WitnessVerdict {
        // Check 1: Confidence floor.
        if prediction.confidence < self.config.min_confidence {
            return WitnessVerdict::Reject {
                reason: format!(
                    "confidence {:.3} below minimum {:.3}",
                    prediction.confidence, self.config.min_confidence
                ),
            };
        }

        // Check 2: Staleness.
        let age_ms = now_ms - prediction.created_at_ms;
        if age_ms > self.config.max_staleness_ms {
            return WitnessVerdict::Reject {
                reason: format!(
                    "prediction age {}ms exceeds max staleness {}ms",
                    age_ms, self.config.max_staleness_ms
                ),
            };
        }

        // Check 3: Already resolved predictions should not be re-consumed.
        if prediction.outcome.is_some() {
            return WitnessVerdict::Reject {
                reason: "prediction already resolved".to_string(),
            };
        }

        // Check 4: Bias correction via the residual corrector.
        let domain_str = prediction
            .domain
            .as_ref()
            .map(|d| d.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let model_id = &prediction.provenance.model_id;
        let bias = corrector.bias(model_id, &domain_str);

        if bias.abs() > self.config.bias_rejection_threshold {
            return WitnessVerdict::Reject {
                reason: format!(
                    "systematic bias {:.3} exceeds rejection threshold {:.3}",
                    bias, self.config.bias_rejection_threshold
                ),
            };
        }

        if bias.abs() > self.config.bias_correction_threshold {
            if let Some(original) = prediction.value.as_f64() {
                let corrected = corrector.correct(model_id, &domain_str, original);
                return WitnessVerdict::AdjustAndAccept {
                    original,
                    corrected,
                    reason: format!(
                        "corrected bias {:.3} in {}/{}",
                        bias, model_id, domain_str
                    ),
                };
            }
        }

        WitnessVerdict::Accept
    }

    /// Batch-verify multiple predictions, returning only those that pass.
    pub fn filter_accepted<'a>(
        &self,
        predictions: &'a [Prediction],
        calibration: &CalibrationTracker,
        corrector: &ResidualCorrector,
        now_ms: i64,
    ) -> Vec<(&'a Prediction, WitnessVerdict)> {
        predictions
            .iter()
            .map(|p| (p, self.verify(p, calibration, corrector, now_ms)))
            .filter(|(_, verdict)| !matches!(verdict, WitnessVerdict::Reject { .. }))
            .collect()
    }
}

impl Default for WitnessVerifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{
        ContentHash, OracleDomain, PredictedValue, PredictionAccuracy, PredictionOutcome,
        PredictionProvenance,
    };

    fn make_prediction(confidence: f64, created_at_ms: i64) -> Prediction {
        let mut p = Prediction::new(
            ContentHash::of(b"test-query"),
            PredictedValue::Probability(0.7),
            confidence,
            created_at_ms + 60_000,
            PredictionProvenance::new("test_model", "test_oracle"),
        )
        .with_domain(OracleDomain::Coding);
        // Override the auto-generated timestamp so staleness checks work.
        p.created_at_ms = created_at_ms;
        p
    }

    #[test]
    fn accept_valid_prediction() {
        let verifier = WitnessVerifier::new();
        let now = chrono::Utc::now().timestamp_millis();
        let prediction = make_prediction(0.8, now - 1000);
        let calibration = CalibrationTracker::default();
        let corrector = ResidualCorrector::default();

        let verdict = verifier.verify(&prediction, &calibration, &corrector, now);
        assert_eq!(verdict, WitnessVerdict::Accept);
    }

    #[test]
    fn reject_low_confidence() {
        let verifier = WitnessVerifier::new();
        let now = chrono::Utc::now().timestamp_millis();
        let prediction = make_prediction(0.05, now - 1000);
        let calibration = CalibrationTracker::default();
        let corrector = ResidualCorrector::default();

        let verdict = verifier.verify(&prediction, &calibration, &corrector, now);
        assert!(matches!(verdict, WitnessVerdict::Reject { .. }));
    }

    #[test]
    fn reject_stale_prediction() {
        let verifier = WitnessVerifier::new();
        let now = chrono::Utc::now().timestamp_millis();
        // 2 hours old.
        let prediction = make_prediction(0.8, now - 7_200_000);
        let calibration = CalibrationTracker::default();
        let corrector = ResidualCorrector::default();

        let verdict = verifier.verify(&prediction, &calibration, &corrector, now);
        assert!(matches!(verdict, WitnessVerdict::Reject { .. }));
    }

    #[test]
    fn adjust_biased_prediction() {
        let verifier = WitnessVerifier::new();
        let now = chrono::Utc::now().timestamp_millis();
        let prediction = make_prediction(0.8, now - 1000);
        let calibration = CalibrationTracker::default();
        let corrector = ResidualCorrector::new(1.0); // Instant convergence.

        // Feed a consistent bias.
        for _ in 0..10 {
            corrector.update("test_model", "coding", 0.2);
        }

        let verdict = verifier.verify(&prediction, &calibration, &corrector, now);
        match verdict {
            WitnessVerdict::AdjustAndAccept {
                original,
                corrected,
                ..
            } => {
                assert!((original - 0.7).abs() < 0.01);
                assert!(corrected < original, "corrected should be less than original");
            }
            _ => panic!("Expected AdjustAndAccept, got {verdict:?}"),
        }
    }

    #[test]
    fn reject_resolved_prediction() {
        let verifier = WitnessVerifier::new();
        let now = chrono::Utc::now().timestamp_millis();
        let mut prediction = make_prediction(0.8, now - 1000);
        prediction.outcome = Some(PredictionOutcome {
            actual: PredictedValue::Probability(0.9),
            evidence_id: ContentHash::of(b"evidence"),
            resolved_at_ms: now,
            accuracy: PredictionAccuracy::new(
                prediction.id,
                ContentHash::of(b"evidence"),
                0.8,
                -0.2,
                OracleDomain::Coding,
                "coding",
            ),
        });

        let calibration = CalibrationTracker::default();
        let corrector = ResidualCorrector::default();

        let verdict = verifier.verify(&prediction, &calibration, &corrector, now);
        assert!(matches!(verdict, WitnessVerdict::Reject { .. }));
    }

    #[test]
    fn filter_accepted_removes_rejects() {
        let verifier = WitnessVerifier::new();
        let now = chrono::Utc::now().timestamp_millis();
        let predictions = vec![
            make_prediction(0.8, now - 1000),  // Accept.
            make_prediction(0.05, now - 1000), // Reject: low confidence.
            make_prediction(0.6, now - 1000),  // Accept.
        ];
        let calibration = CalibrationTracker::default();
        let corrector = ResidualCorrector::default();

        let accepted = verifier.filter_accepted(&predictions, &calibration, &corrector, now);
        assert_eq!(accepted.len(), 2);
    }
}
