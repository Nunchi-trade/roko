//! CodingOracle — code quality prediction from historical build/test patterns.
//!
//! Implements the Oracle trait for `OracleDomain::Coding`. Predicts whether
//! code changes will pass gates based on historical build times, test pass
//! rates, complexity drift, and dependency risk signals.

use async_trait::async_trait;
use roko_core::{
    CodingMetric, Context, Signal, Oracle, OracleDomain, OracleQuery, PredictedValue, Prediction,
    PredictionAccuracy, PredictionInterval, PredictionProvenance, QueryPayload,
};
use std::collections::VecDeque;

/// A historical build observation.
#[derive(Debug, Clone)]
pub struct BuildRecord {
    /// Build duration in seconds.
    pub duration_secs: f64,
    /// Whether the build succeeded.
    pub success: bool,
    /// Warning count.
    pub warnings: u32,
    /// Timestamp in milliseconds.
    pub ts_ms: i64,
}

/// A historical test run observation.
#[derive(Debug, Clone)]
pub struct TestRecord {
    /// Number of tests passed.
    pub passed: u32,
    /// Number of tests failed.
    pub failed: u32,
    /// Total test count.
    pub total: u32,
    /// Timestamp in milliseconds.
    pub ts_ms: i64,
}

/// CodingOracle predicts code quality outcomes from historical patterns.
pub struct CodingOracle {
    /// Rolling build history.
    build_history: parking_lot::RwLock<VecDeque<BuildRecord>>,
    /// Rolling test history.
    test_history: parking_lot::RwLock<VecDeque<TestRecord>>,
    /// Complexity observations (delta per change).
    complexity_history: parking_lot::RwLock<VecDeque<f64>>,
    /// Maximum history depth.
    max_history: usize,
}

impl CodingOracle {
    /// Create a new coding oracle with default history depth.
    #[must_use]
    pub fn new() -> Self {
        Self {
            build_history: parking_lot::RwLock::new(VecDeque::with_capacity(100)),
            test_history: parking_lot::RwLock::new(VecDeque::with_capacity(100)),
            complexity_history: parking_lot::RwLock::new(VecDeque::with_capacity(100)),
            max_history: 100,
        }
    }

    /// Record a build observation.
    pub fn observe_build(&self, record: BuildRecord) {
        let mut history = self.build_history.write();
        if history.len() >= self.max_history {
            history.pop_front();
        }
        history.push_back(record);
    }

    /// Record a test run observation.
    pub fn observe_test(&self, record: TestRecord) {
        let mut history = self.test_history.write();
        if history.len() >= self.max_history {
            history.pop_front();
        }
        history.push_back(record);
    }

    /// Record a complexity delta observation.
    pub fn observe_complexity(&self, delta: f64) {
        let mut history = self.complexity_history.write();
        if history.len() >= self.max_history {
            history.pop_front();
        }
        history.push_back(delta);
    }

    /// Predict build time based on trend analysis.
    fn predict_build_time(&self) -> (f64, f64) {
        let history = self.build_history.read();
        if history.is_empty() {
            return (30.0, 0.2); // Default 30s with low confidence.
        }

        let durations: Vec<f64> = history.iter().map(|r| r.duration_secs).collect();
        let mean = durations.iter().sum::<f64>() / durations.len() as f64;

        // Use EMA of last 10 builds for trend.
        let recent: Vec<f64> = durations.iter().rev().take(10).copied().collect();
        let recent_mean = recent.iter().sum::<f64>() / recent.len() as f64;

        // Trend-adjusted prediction: bias toward recent behavior.
        let predicted = mean * 0.3 + recent_mean * 0.7;
        let confidence = (history.len() as f64 / 20.0).min(0.85);

        (predicted, confidence)
    }

    /// Predict test pass rate from historical runs.
    fn predict_test_pass_rate(&self) -> (f64, f64) {
        let history = self.test_history.read();
        if history.is_empty() {
            return (0.8, 0.2); // Optimistic default.
        }

        let rates: Vec<f64> = history
            .iter()
            .map(|r| {
                if r.total > 0 {
                    r.passed as f64 / r.total as f64
                } else {
                    1.0
                }
            })
            .collect();

        // Weighted average favoring recent runs.
        let n = rates.len();
        let mut weighted_sum = 0.0;
        let mut weight_sum = 0.0;
        for (i, rate) in rates.iter().enumerate() {
            let weight = (i + 1) as f64;
            weighted_sum += rate * weight;
            weight_sum += weight;
        }
        let predicted = weighted_sum / weight_sum.max(1.0);
        let confidence = (n as f64 / 10.0).min(0.9);

        (predicted, confidence)
    }

    /// Predict complexity drift direction.
    fn predict_complexity_delta(&self) -> (f64, f64) {
        let history = self.complexity_history.read();
        if history.len() < 3 {
            return (0.0, 0.15);
        }

        let values: Vec<f64> = history.iter().copied().collect();
        let mean = values.iter().sum::<f64>() / values.len() as f64;

        // Trend: are deltas getting larger?
        let n = values.len();
        let first_half_mean = values[..n / 2].iter().sum::<f64>() / (n / 2) as f64;
        let second_half_mean = values[n / 2..].iter().sum::<f64>() / (n - n / 2) as f64;
        let trend = second_half_mean - first_half_mean;

        let predicted = mean + trend * 0.5;
        let confidence = (n as f64 / 15.0).min(0.75);

        (predicted, confidence)
    }
}

impl Default for CodingOracle {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Oracle for CodingOracle {
    async fn predict(
        &self,
        query: &OracleQuery,
        _ctx: &Context,
    ) -> roko_core::error::Result<Prediction> {
        let payload = match &query.payload {
            QueryPayload::Coding(p) => p,
            _ => {
                return Err(roko_core::RokoError::Invalid(
                    "CodingOracle received non-coding query".into(),
                ));
            }
        };

        let resolve_by = query.created_at_ms + query.horizon.as_millis() as i64;

        let (value, confidence) = match payload.metric {
            CodingMetric::BuildTime => {
                let (predicted, conf) = self.predict_build_time();
                (PredictedValue::Numeric(predicted), conf)
            }
            CodingMetric::TestPassRate => {
                let (predicted, conf) = self.predict_test_pass_rate();
                (PredictedValue::Probability(predicted), conf)
            }
            CodingMetric::ComplexityDelta => {
                let (predicted, conf) = self.predict_complexity_delta();
                (PredictedValue::Numeric(predicted), conf)
            }
            CodingMetric::DependencyRisk => {
                // Simple heuristic: default low risk.
                (PredictedValue::Probability(0.15), 0.3)
            }
            CodingMetric::PerfRegression => {
                // Based on complexity trend.
                let (delta, conf) = self.predict_complexity_delta();
                let risk = (delta / 10.0).clamp(0.0, 0.8);
                (PredictedValue::Probability(risk), conf * 0.8)
            }
            CodingMetric::CoverageImpact => {
                // Estimate from test history trend.
                let (pass_rate, conf) = self.predict_test_pass_rate();
                let coverage_impact = (pass_rate - 0.9) * 10.0; // Scaled delta.
                (
                    PredictedValue::Numeric(coverage_impact.clamp(-5.0, 5.0)),
                    conf * 0.7,
                )
            }
            _ => {
                // Unknown metric — return a neutral, low-confidence prediction.
                (PredictedValue::Probability(0.5), 0.1)
            }
        };

        let mut prediction = Prediction::new(
            query.id,
            value,
            confidence,
            resolve_by,
            PredictionProvenance::new("coding_oracle", "coding_oracle_v1"),
        )
        .with_domain(OracleDomain::Coding);

        // Attach interval for numeric predictions.
        if let PredictedValue::Numeric(v) = prediction.value {
            let spread = v.abs() * 0.2 + 1.0;
            prediction =
                prediction.with_interval(PredictionInterval::new(v - spread, v + spread, 0.80));
        }

        Ok(prediction)
    }

    async fn evaluate(
        &self,
        prediction: &Prediction,
        outcome: &Signal,
    ) -> roko_core::error::Result<PredictionAccuracy> {
        let predicted = prediction.value.as_f64().unwrap_or(0.5);
        let actual = outcome
            .body
            .as_text()
            .ok()
            .and_then(|text| text.parse::<f64>().ok())
            .unwrap_or(0.5);

        let residual = predicted - actual;
        let accuracy = match &prediction.value {
            PredictedValue::Probability(_) => 1.0 - residual.abs(),
            PredictedValue::Numeric(_) => {
                let scale = predicted.abs().max(1.0);
                1.0 - (residual.abs() / scale).min(1.0)
            }
            _ => 1.0 - residual.abs().min(1.0),
        };

        let interval_hit = prediction
            .interval
            .as_ref()
            .map(|interval| interval.contains(actual));

        let resolution_lag = chrono::Utc::now().timestamp_millis() - prediction.created_at_ms;

        Ok(PredictionAccuracy::new(
            prediction.id,
            outcome.id,
            accuracy.clamp(0.0, 1.0),
            residual,
            OracleDomain::Coding,
            "coding",
        )
        .with_interval_hit(interval_hit)
        .with_resolution_lag_ms(resolution_lag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{CodingQueryPayload, CodingScope, Context, OracleQuery, QueryPayload};
    use std::time::Duration;

    fn make_coding_query(metric: CodingMetric) -> OracleQuery {
        OracleQuery::new(
            OracleDomain::Coding,
            QueryPayload::Coding(CodingQueryPayload {
                scope: CodingScope::Workspace,
                metric,
                change_context: None,
            }),
            Duration::from_secs(60),
            0.5,
        )
    }

    #[tokio::test]
    async fn coding_oracle_build_time_no_history() {
        let oracle = CodingOracle::new();
        let query = make_coding_query(CodingMetric::BuildTime);
        let ctx = Context::default();
        let prediction = oracle.predict(&query, &ctx).await.unwrap();

        assert_eq!(prediction.confidence, 0.2);
        if let PredictedValue::Numeric(v) = &prediction.value {
            assert!((v - 30.0).abs() < f64::EPSILON);
        }
    }

    #[tokio::test]
    async fn coding_oracle_build_time_with_history() {
        let oracle = CodingOracle::new();
        let now = chrono::Utc::now().timestamp_millis();

        for i in 0..15 {
            oracle.observe_build(BuildRecord {
                duration_secs: 25.0 + i as f64,
                success: true,
                warnings: 0,
                ts_ms: now + i * 1000,
            });
        }

        let query = make_coding_query(CodingMetric::BuildTime);
        let ctx = Context::default();
        let prediction = oracle.predict(&query, &ctx).await.unwrap();

        assert!(prediction.confidence > 0.5);
        if let PredictedValue::Numeric(v) = &prediction.value {
            assert!(*v > 20.0 && *v < 50.0);
        }
    }

    #[tokio::test]
    async fn coding_oracle_test_pass_rate() {
        let oracle = CodingOracle::new();
        let now = chrono::Utc::now().timestamp_millis();

        for i in 0..10 {
            oracle.observe_test(TestRecord {
                passed: 95,
                failed: 5,
                total: 100,
                ts_ms: now + i * 1000,
            });
        }

        let query = make_coding_query(CodingMetric::TestPassRate);
        let ctx = Context::default();
        let prediction = oracle.predict(&query, &ctx).await.unwrap();

        if let PredictedValue::Probability(p) = &prediction.value {
            assert!((*p - 0.95).abs() < 0.05);
        }
    }
}
