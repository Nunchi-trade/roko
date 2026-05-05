//! ResearchOracle — research quality prediction from source analysis.
//!
//! Implements the Oracle trait for `OracleDomain::Research`. Estimates
//! research depth needed for a topic, source reliability, and
//! contradiction risk based on historical patterns.

use async_trait::async_trait;
use roko_core::{
    Context, Oracle, OracleDomain, OracleQuery, PredictedValue, Prediction, PredictionAccuracy,
    PredictionInterval, PredictionProvenance, QueryPayload, ResearchMetric, Signal,
};
use std::collections::HashMap;

/// A historical research outcome used for calibration.
#[derive(Debug, Clone)]
pub struct ResearchRecord {
    /// Topic of the research.
    pub topic: String,
    /// Number of sources consulted.
    pub source_count: u32,
    /// Overall quality score [0, 1].
    pub quality_score: f64,
    /// Whether contradictions were found.
    pub had_contradictions: bool,
    /// Timestamp in milliseconds.
    pub ts_ms: i64,
}

/// ResearchOracle predicts research quality from historical patterns.
pub struct ResearchOracle {
    /// Historical research outcomes by topic category.
    history: parking_lot::RwLock<Vec<ResearchRecord>>,
    /// Per-source reliability scores.
    source_reliability: parking_lot::RwLock<HashMap<String, Vec<f64>>>,
    /// Maximum history length.
    max_history: usize,
}

impl ResearchOracle {
    /// Create a new research oracle.
    #[must_use]
    pub fn new() -> Self {
        Self {
            history: parking_lot::RwLock::new(Vec::new()),
            source_reliability: parking_lot::RwLock::new(HashMap::new()),
            max_history: 200,
        }
    }

    /// Record a research outcome for calibration.
    pub fn observe_outcome(&self, record: ResearchRecord) {
        let mut history = self.history.write();
        if history.len() >= self.max_history {
            history.remove(0);
        }
        history.push(record);
    }

    /// Record a source reliability observation.
    pub fn observe_source_reliability(&self, source_id: &str, score: f64) {
        let mut reliability = self.source_reliability.write();
        let scores = reliability.entry(source_id.to_string()).or_default();
        if scores.len() >= 50 {
            scores.remove(0);
        }
        scores.push(score.clamp(0.0, 1.0));
    }

    /// Estimate source reliability from historical observations.
    fn estimate_reliability(&self, source_id: &str) -> f64 {
        let reliability = self.source_reliability.read();
        match reliability.get(source_id) {
            Some(scores) if !scores.is_empty() => scores.iter().sum::<f64>() / scores.len() as f64,
            _ => 0.6, // Default moderate reliability.
        }
    }

    /// Estimate research completeness for a topic based on similar prior research.
    fn estimate_completeness(&self) -> (f64, f64) {
        let history = self.history.read();
        if history.is_empty() {
            return (0.5, 0.2);
        }

        let qualities: Vec<f64> = history.iter().map(|r| r.quality_score).collect();
        let mean = qualities.iter().sum::<f64>() / qualities.len() as f64;
        let confidence = (history.len() as f64 / 20.0).min(0.8);

        (mean, confidence)
    }

    /// Estimate contradiction risk from historical patterns.
    fn estimate_contradiction_risk(&self) -> (f64, f64) {
        let history = self.history.read();
        if history.len() < 3 {
            return (0.2, 0.15);
        }

        let contradiction_rate =
            history.iter().filter(|r| r.had_contradictions).count() as f64 / history.len() as f64;
        let confidence = (history.len() as f64 / 15.0).min(0.75);

        (contradiction_rate, confidence)
    }
}

impl Default for ResearchOracle {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Oracle for ResearchOracle {
    async fn predict(
        &self,
        query: &OracleQuery,
        _ctx: &Context,
    ) -> roko_core::error::Result<Prediction> {
        let payload = match &query.payload {
            QueryPayload::Research(p) => p,
            _ => {
                return Err(roko_core::RokoError::Invalid(
                    "ResearchOracle received non-research query".into(),
                ));
            }
        };

        let resolve_by = query.created_at_ms + query.horizon.as_millis() as i64;

        let (value, confidence) = match payload.metric {
            ResearchMetric::Reliability => {
                let source_id = payload
                    .source
                    .url
                    .as_deref()
                    .or(payload.source.doi.as_deref())
                    .unwrap_or(&payload.source.title);
                let reliability = self.estimate_reliability(source_id);
                let conf = {
                    let scores = self.source_reliability.read();
                    scores
                        .get(source_id)
                        .map(|s| (s.len() as f64 / 10.0).min(0.85))
                        .unwrap_or(0.2)
                };
                (PredictedValue::Probability(reliability), conf)
            }
            ResearchMetric::Completeness => {
                let (completeness, conf) = self.estimate_completeness();
                (PredictedValue::Probability(completeness), conf)
            }
            ResearchMetric::ContradictionRisk => {
                let (risk, conf) = self.estimate_contradiction_risk();
                (PredictedValue::Probability(risk), conf)
            }
            ResearchMetric::ReplicationProbability => {
                // Based on source reliability and overall quality.
                let (quality, _) = self.estimate_completeness();
                let source_rel = {
                    let source_id = payload
                        .source
                        .url
                        .as_deref()
                        .unwrap_or(&payload.source.title);
                    self.estimate_reliability(source_id)
                };
                let replication = (quality * 0.4 + source_rel * 0.6).clamp(0.1, 0.95);
                (PredictedValue::Probability(replication), 0.4)
            }
            ResearchMetric::CitationMomentum => {
                // Placeholder: no citation data yet.
                (PredictedValue::Numeric(0.0), 0.1)
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
            PredictionProvenance::new("research_oracle", "research_oracle_v1"),
        )
        .with_domain(OracleDomain::Research);

        if let PredictedValue::Probability(p) = prediction.value {
            prediction = prediction.with_interval(PredictionInterval::new(
                (p - 0.15).max(0.0),
                (p + 0.15).min(1.0),
                0.80,
            ));
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
        let accuracy = (1.0 - residual.abs()).clamp(0.0, 1.0);

        let interval_hit = prediction
            .interval
            .as_ref()
            .map(|interval| interval.contains(actual));

        let resolution_lag = chrono::Utc::now().timestamp_millis() - prediction.created_at_ms;

        Ok(PredictionAccuracy::new(
            prediction.id,
            outcome.id,
            accuracy,
            residual,
            OracleDomain::Research,
            "research",
        )
        .with_interval_hit(interval_hit)
        .with_resolution_lag_ms(resolution_lag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Context, OracleQuery, QueryPayload, ResearchQueryPayload, SourceReference};
    use std::time::Duration;

    fn make_research_query(metric: ResearchMetric) -> OracleQuery {
        OracleQuery::new(
            OracleDomain::Research,
            QueryPayload::Research(ResearchQueryPayload {
                source: SourceReference {
                    title: "Test source".to_string(),
                    url: Some("https://example.com/paper".to_string()),
                    doi: None,
                    authors: vec!["Author".to_string()],
                },
                metric,
                claim_context: Some("Test claim".to_string()),
            }),
            Duration::from_secs(3600),
            0.5,
        )
    }

    #[tokio::test]
    async fn research_oracle_reliability_no_history() {
        let oracle = ResearchOracle::new();
        let query = make_research_query(ResearchMetric::Reliability);
        let ctx = Context::default();
        let prediction = oracle.predict(&query, &ctx).await.unwrap();

        // Default reliability with low confidence.
        assert!(prediction.confidence <= 0.3);
        if let PredictedValue::Probability(p) = &prediction.value {
            assert!((*p - 0.6).abs() < 0.01);
        }
    }

    #[tokio::test]
    async fn research_oracle_reliability_with_history() {
        let oracle = ResearchOracle::new();

        // Feed reliability observations.
        for _ in 0..10 {
            oracle.observe_source_reliability("https://example.com/paper", 0.85);
        }

        let query = make_research_query(ResearchMetric::Reliability);
        let ctx = Context::default();
        let prediction = oracle.predict(&query, &ctx).await.unwrap();

        assert!(prediction.confidence > 0.5);
        if let PredictedValue::Probability(p) = &prediction.value {
            assert!((*p - 0.85).abs() < 0.05);
        }
    }

    #[tokio::test]
    async fn research_oracle_contradiction_risk() {
        let oracle = ResearchOracle::new();
        let now = chrono::Utc::now().timestamp_millis();

        // Half of past research had contradictions.
        for i in 0..10 {
            oracle.observe_outcome(ResearchRecord {
                topic: "test".to_string(),
                source_count: 5,
                quality_score: 0.7,
                had_contradictions: i % 2 == 0,
                ts_ms: now + i * 1000,
            });
        }

        let query = make_research_query(ResearchMetric::ContradictionRisk);
        let ctx = Context::default();
        let prediction = oracle.predict(&query, &ctx).await.unwrap();

        if let PredictedValue::Probability(p) = &prediction.value {
            assert!((*p - 0.5).abs() < 0.1);
        }
    }
}
