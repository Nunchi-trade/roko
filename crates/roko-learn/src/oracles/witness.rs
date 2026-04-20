//! Witness verification pipeline for Oracle predictions.
//!
//! TA-04: Generalized witness pipeline — domain-agnostic data ingestion trait
//! plus prediction verification. Before an oracle prediction is consumed by the
//! orchestrator or router, it passes through a witness verification step that
//! checks calibration, staleness, and internal consistency.
//!
//! The `Witness` trait generalizes observation across domains: each domain
//! oracle uses a witness to observe its environment and triage signals. The
//! triage pipeline uses threshold-based anomaly detection to determine whether
//! a signal is anomalous and should trigger tier escalation.

use roko_core::{CalibrationTracker, OracleDomain, Prediction, ResidualCorrector};
use std::collections::HashMap;

// ─── Generalized Witness trait (TA-04) ──────────────────────────────────

/// A single observation from the environment.
#[derive(Debug, Clone)]
pub struct Observation {
    /// Observation domain.
    pub domain: OracleDomain,
    /// Human-readable signal name (e.g., "build_time", "gas_price").
    pub name: String,
    /// Observed value.
    pub value: f64,
    /// Unix milliseconds when observed.
    pub observed_at_ms: i64,
    /// Optional metadata tags.
    pub tags: HashMap<String, String>,
}

impl Observation {
    /// Create a new observation.
    pub fn new(domain: OracleDomain, name: impl Into<String>, value: f64) -> Self {
        Self {
            domain,
            name: name.into(),
            value,
            observed_at_ms: chrono::Utc::now().timestamp_millis(),
            tags: HashMap::new(),
        }
    }

    /// Add a tag to this observation.
    #[must_use]
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }
}

/// Classification of the observation anomaly level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnomalyLevel {
    /// Within normal range — no escalation needed.
    Normal,
    /// Mildly anomalous — monitor but do not escalate.
    Elevated,
    /// Highly anomalous — trigger tier escalation.
    Anomalous,
}

/// Per-observation triage outcome.
#[derive(Debug, Clone)]
pub struct TriagedObservation {
    /// The original observation.
    pub observation: Observation,
    /// Whether the observation is anomalous.
    pub anomaly_level: AnomalyLevel,
    /// Z-score relative to the running window (if computed).
    pub z_score: f64,
}

/// Aggregate triage result from a witness observation cycle.
#[derive(Debug, Clone)]
pub struct TriageResult {
    /// Triaged observations.
    pub observations: Vec<TriagedObservation>,
    /// Overall anomaly score in `[0.0, 1.0]` — fraction of observations that
    /// are anomalous.
    pub anomaly_fraction: f64,
    /// Whether any observation triggered a tier escalation.
    pub escalation_triggered: bool,
}

impl TriageResult {
    /// Create an empty triage result.
    pub fn empty() -> Self {
        Self {
            observations: Vec::new(),
            anomaly_fraction: 0.0,
            escalation_triggered: false,
        }
    }
}

/// Domain-agnostic data ingestion trait (TA-04).
///
/// Each domain oracle uses a `Witness` to observe its environment and triage
/// signals. Witnesses operate at T0 speed (reflex, <1ms) for threshold-based
/// checks and can trigger tier escalation for anomalous signals.
pub trait Witness: Send + Sync {
    /// Observe the environment and return raw signal data.
    ///
    /// Implementations should be lightweight and deterministic — no LLM calls.
    fn observe(&self) -> Vec<Observation>;

    /// Triage observations into anomaly classifications.
    ///
    /// Uses threshold-based anomaly detection to determine whether each
    /// observation is anomalous and should trigger tier escalation.
    fn triage(&self, observations: &[Observation]) -> TriageResult;
}

// ─── CodingWitness (TA-04) ─────────────────────────────────────────────

/// Running statistics for a named signal used by threshold-based triage.
#[derive(Debug, Clone)]
struct SignalStats {
    /// Running mean.
    mean: f64,
    /// Running variance (online Welford algorithm).
    m2: f64,
    /// Observation count.
    count: u64,
    /// Threshold z-score for anomaly classification.
    z_threshold: f64,
}

impl SignalStats {
    fn new(z_threshold: f64) -> Self {
        Self {
            mean: 0.0,
            m2: 0.0,
            count: 0,
            z_threshold,
        }
    }

    /// Update with a new value and return the z-score.
    fn update(&mut self, value: f64) -> f64 {
        self.count += 1;
        let delta = value - self.mean;
        self.mean += delta / self.count as f64;
        let delta2 = value - self.mean;
        self.m2 += delta * delta2;

        if self.count < 3 {
            return 0.0;
        }
        let stddev = (self.m2 / (self.count - 1) as f64).sqrt();
        if stddev < f64::EPSILON {
            return 0.0;
        }
        (value - self.mean) / stddev
    }

    /// Classify based on z-score.
    fn classify(&self, z_score: f64) -> AnomalyLevel {
        let abs_z = z_score.abs();
        if abs_z > self.z_threshold * 1.5 {
            AnomalyLevel::Anomalous
        } else if abs_z > self.z_threshold {
            AnomalyLevel::Elevated
        } else {
            AnomalyLevel::Normal
        }
    }
}

/// Filesystem and CI/CD coding metrics used as witness input.
#[derive(Debug, Clone, Default)]
pub struct CodingMetrics {
    /// Last build duration in seconds.
    pub build_time_secs: f64,
    /// Test pass rate in `[0.0, 1.0]`.
    pub test_pass_rate: f64,
    /// McCabe complexity delta since last observation.
    pub complexity_delta: f64,
    /// Number of outdated dependencies divided by total.
    pub dependency_freshness: f64,
    /// Lines changed per commit (code churn).
    pub churn_rate: f64,
    /// Number of co-changed file pairs.
    pub file_coupling_count: f64,
}

/// CodingWitness observes filesystem and CI/CD state for the `CodingOracle`.
///
/// Implements the six T0 coding probes from the spec: build_time_trend,
/// test_pass_rate, complexity_delta, dependency_freshness, churn_rate,
/// and file_coupling.
pub struct CodingWitness {
    /// Current metrics snapshot (updated externally).
    metrics: parking_lot::RwLock<CodingMetrics>,
    /// Per-signal running statistics for anomaly detection.
    stats: parking_lot::RwLock<HashMap<String, SignalStats>>,
    /// Z-score threshold for anomaly classification.
    z_threshold: f64,
}

impl CodingWitness {
    /// Create a new coding witness with default z-threshold (2.0).
    pub fn new() -> Self {
        Self {
            metrics: parking_lot::RwLock::new(CodingMetrics::default()),
            stats: parking_lot::RwLock::new(HashMap::new()),
            z_threshold: 2.0,
        }
    }

    /// Create a coding witness with a custom z-threshold.
    pub fn with_z_threshold(z_threshold: f64) -> Self {
        Self {
            metrics: parking_lot::RwLock::new(CodingMetrics::default()),
            stats: parking_lot::RwLock::new(HashMap::new()),
            z_threshold,
        }
    }

    /// Update the current metrics snapshot.
    pub fn update_metrics(&self, metrics: CodingMetrics) {
        *self.metrics.write() = metrics;
    }

    /// Get or insert running stats for a signal name.
    fn get_or_insert_stats<'a>(
        stats: &'a mut HashMap<String, SignalStats>,
        name: &'a str,
        z_threshold: f64,
    ) -> &'a mut SignalStats {
        stats
            .entry(name.to_string())
            .or_insert_with(|| SignalStats::new(z_threshold))
    }
}

impl Default for CodingWitness {
    fn default() -> Self {
        Self::new()
    }
}

impl Witness for CodingWitness {
    fn observe(&self) -> Vec<Observation> {
        let metrics = self.metrics.read();
        vec![
            Observation::new(
                OracleDomain::Coding,
                "build_time_trend",
                metrics.build_time_secs,
            ),
            Observation::new(
                OracleDomain::Coding,
                "test_pass_rate",
                metrics.test_pass_rate,
            ),
            Observation::new(
                OracleDomain::Coding,
                "complexity_delta",
                metrics.complexity_delta,
            ),
            Observation::new(
                OracleDomain::Coding,
                "dependency_freshness",
                metrics.dependency_freshness,
            ),
            Observation::new(OracleDomain::Coding, "churn_rate", metrics.churn_rate),
            Observation::new(
                OracleDomain::Coding,
                "file_coupling",
                metrics.file_coupling_count,
            ),
        ]
    }

    fn triage(&self, observations: &[Observation]) -> TriageResult {
        let mut stats = self.stats.write();
        let mut triaged = Vec::with_capacity(observations.len());
        let mut anomalous_count = 0;
        let mut escalation = false;

        for obs in observations {
            let signal_stats = Self::get_or_insert_stats(&mut stats, &obs.name, self.z_threshold);
            let z_score = signal_stats.update(obs.value);
            let anomaly_level = signal_stats.classify(z_score);

            if anomaly_level == AnomalyLevel::Anomalous {
                anomalous_count += 1;
                escalation = true;
            }

            triaged.push(TriagedObservation {
                observation: obs.clone(),
                anomaly_level,
                z_score,
            });
        }

        let anomaly_fraction = if observations.is_empty() {
            0.0
        } else {
            anomalous_count as f64 / observations.len() as f64
        };

        TriageResult {
            observations: triaged,
            anomaly_fraction,
            escalation_triggered: escalation,
        }
    }
}

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
                    reason: format!("corrected bias {:.3} in {}/{}", bias, model_id, domain_str),
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
                assert!(
                    corrected < original,
                    "corrected should be less than original"
                );
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

    // ─── Generalized Witness trait tests (TA-04) ───────────────────────

    #[test]
    fn coding_witness_observe_returns_six_probes() {
        let witness = CodingWitness::new();
        witness.update_metrics(CodingMetrics {
            build_time_secs: 25.0,
            test_pass_rate: 0.95,
            complexity_delta: 2.0,
            dependency_freshness: 0.1,
            churn_rate: 150.0,
            file_coupling_count: 3.0,
        });
        let observations = witness.observe();
        assert_eq!(
            observations.len(),
            6,
            "CodingWitness should produce 6 observations"
        );
        assert_eq!(observations[0].name, "build_time_trend");
        assert_eq!(observations[1].name, "test_pass_rate");
    }

    #[test]
    fn coding_witness_triage_normal_observations() {
        let witness = CodingWitness::new();

        // Feed several observations with natural variation to build a
        // realistic baseline with non-zero variance.
        for i in 0..30 {
            witness.update_metrics(CodingMetrics {
                build_time_secs: 25.0 + (i as f64 * 0.3).sin() * 2.0,
                test_pass_rate: 0.95 + (i as f64 * 0.2).cos() * 0.02,
                complexity_delta: 1.0 + (i as f64 * 0.5).sin() * 0.3,
                ..CodingMetrics::default()
            });
            let observations = witness.observe();
            let _triage = witness.triage(&observations);
        }

        // One more observation within the natural range should not
        // trigger escalation.
        witness.update_metrics(CodingMetrics {
            build_time_secs: 26.0,
            test_pass_rate: 0.94,
            complexity_delta: 1.2,
            ..CodingMetrics::default()
        });
        let observations = witness.observe();
        let result = witness.triage(&observations);
        assert!(!result.escalation_triggered);
    }

    #[test]
    fn coding_witness_triage_detects_anomaly() {
        let witness = CodingWitness::new();

        // Build a stable baseline.
        for _ in 0..20 {
            witness.update_metrics(CodingMetrics {
                build_time_secs: 25.0,
                test_pass_rate: 0.95,
                complexity_delta: 1.0,
                ..CodingMetrics::default()
            });
            let observations = witness.observe();
            let _triage = witness.triage(&observations);
        }

        // Sudden spike — build time 10x normal.
        witness.update_metrics(CodingMetrics {
            build_time_secs: 250.0,
            test_pass_rate: 0.95,
            complexity_delta: 1.0,
            ..CodingMetrics::default()
        });
        let observations = witness.observe();
        let result = witness.triage(&observations);
        assert!(
            result.escalation_triggered,
            "10x build time should trigger escalation"
        );
        assert!(result.anomaly_fraction > 0.0);
    }

    #[test]
    fn triage_result_empty() {
        let result = TriageResult::empty();
        assert!(result.observations.is_empty());
        assert!(!result.escalation_triggered);
    }

    #[test]
    fn observation_with_tag() {
        let obs =
            Observation::new(OracleDomain::Coding, "build_time", 25.0).with_tag("source", "ci");
        assert_eq!(obs.tags.get("source").unwrap(), "ci");
    }
}
