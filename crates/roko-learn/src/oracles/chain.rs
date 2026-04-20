//! ChainOracle — on-chain TA indicators producing calibrated predictions.
//!
//! Implements the Oracle trait for `OracleDomain::Chain` with traditional
//! technical analysis indicators (MA, RSI, Bollinger) and DeFi-native signals.

use async_trait::async_trait;
use roko_core::{
    ChainMetric, ChainQueryPayload, Context, Engram, Oracle, OracleDomain,
    OracleQuery, PredictedValue, Prediction, PredictionAccuracy, PredictionInterval,
    PredictionProvenance, QueryPayload,
};
use std::collections::VecDeque;

/// A single price observation for time-series analysis.
#[derive(Debug, Clone, Copy)]
pub struct PricePoint {
    /// Observation timestamp in milliseconds.
    pub ts_ms: i64,
    /// Observed price.
    pub price: f64,
    /// High price for the period (for RSI/Bollinger).
    pub high: f64,
    /// Low price for the period.
    pub low: f64,
    /// Close price for the period.
    pub close: f64,
}

/// Traditional TA indicator output.
#[derive(Debug, Clone)]
pub struct IndicatorOutput {
    /// Indicator name.
    pub name: String,
    /// Current signal value.
    pub value: f64,
    /// Signal direction: positive = bullish, negative = bearish.
    pub signal: f64,
    /// Confidence in the signal.
    pub confidence: f64,
}

/// Compute simple moving average over the last `period` closes.
fn simple_ma(prices: &[f64], period: usize) -> Option<f64> {
    if prices.len() < period {
        return None;
    }
    let slice = &prices[prices.len() - period..];
    Some(slice.iter().sum::<f64>() / period as f64)
}

/// Compute exponential moving average.
fn exponential_ma(prices: &[f64], period: usize) -> Option<f64> {
    if prices.is_empty() || period == 0 {
        return None;
    }
    let alpha = 2.0 / (period as f64 + 1.0);
    let mut ema = prices[0];
    for &price in &prices[1..] {
        ema = alpha.mul_add(price, (1.0 - alpha) * ema);
    }
    Some(ema)
}

/// Compute 14-period RSI (Wilder 1978).
fn rsi(prices: &[f64], period: usize) -> Option<f64> {
    if prices.len() < period + 1 {
        return None;
    }

    let mut avg_gain = 0.0;
    let mut avg_loss = 0.0;

    // Initial average using first `period` changes.
    for i in 1..=period {
        let change = prices[i] - prices[i - 1];
        if change > 0.0 {
            avg_gain += change;
        } else {
            avg_loss += change.abs();
        }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;

    // Smooth over remaining data.
    let period_f = period as f64;
    for i in (period + 1)..prices.len() {
        let change = prices[i] - prices[i - 1];
        if change > 0.0 {
            avg_gain = (avg_gain * (period_f - 1.0) + change) / period_f;
            avg_loss = (avg_loss * (period_f - 1.0)) / period_f;
        } else {
            avg_gain = (avg_gain * (period_f - 1.0)) / period_f;
            avg_loss = (avg_loss * (period_f - 1.0) + change.abs()) / period_f;
        }
    }

    if avg_loss < f64::EPSILON {
        return Some(100.0);
    }
    let rs = avg_gain / avg_loss;
    Some(100.0 - 100.0 / (1.0 + rs))
}

/// Bollinger Bands output.
#[derive(Debug, Clone, Copy)]
pub struct BollingerBands {
    /// Middle band (SMA).
    pub middle: f64,
    /// Upper band (middle + 2 * stddev).
    pub upper: f64,
    /// Lower band (middle - 2 * stddev).
    pub lower: f64,
    /// Current price position within the bands in [0, 1].
    pub percent_b: f64,
}

/// Compute Bollinger Bands (20-period, 2-sigma).
fn bollinger_bands(prices: &[f64], period: usize) -> Option<BollingerBands> {
    if prices.len() < period {
        return None;
    }
    let slice = &prices[prices.len() - period..];
    let middle = slice.iter().sum::<f64>() / period as f64;
    let variance = slice.iter().map(|p| (p - middle).powi(2)).sum::<f64>() / period as f64;
    let stddev = variance.sqrt();
    let upper = middle + 2.0 * stddev;
    let lower = middle - 2.0 * stddev;
    let current = prices[prices.len() - 1];
    let bandwidth = upper - lower;
    let percent_b = if bandwidth > f64::EPSILON {
        (current - lower) / bandwidth
    } else {
        0.5
    };
    Some(BollingerBands {
        middle,
        upper,
        lower,
        percent_b,
    })
}

/// ChainOracle predicts on-chain metrics using traditional TA indicators.
///
/// This is a deterministic, zero-LLM oracle that operates on cached price
/// time series. For production use, feed it via an external price service;
/// for testing and self-hosting, it works on synthetic or historical data.
pub struct ChainOracle {
    /// Rolling price history per asset (keyed by target string).
    price_history: parking_lot::RwLock<std::collections::HashMap<String, VecDeque<f64>>>,
    /// Maximum history length.
    max_history: usize,
}

impl ChainOracle {
    /// Create a new chain oracle with default history depth.
    #[must_use]
    pub fn new() -> Self {
        Self {
            price_history: parking_lot::RwLock::new(std::collections::HashMap::new()),
            max_history: 200,
        }
    }

    /// Create a chain oracle with a custom history depth.
    #[must_use]
    pub fn with_max_history(max_history: usize) -> Self {
        Self {
            price_history: parking_lot::RwLock::new(std::collections::HashMap::new()),
            max_history,
        }
    }

    /// Feed a price observation for an asset.
    pub fn observe_price(&self, target: &str, price: f64) {
        let mut history = self.price_history.write();
        let series = history
            .entry(target.to_string())
            .or_insert_with(|| VecDeque::with_capacity(self.max_history));
        if series.len() >= self.max_history {
            series.pop_front();
        }
        series.push_back(price);
    }

    /// Get the current price series for an asset.
    fn prices(&self, target: &str) -> Vec<f64> {
        self.price_history
            .read()
            .get(target)
            .map(|deque| deque.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Run all indicators and produce a composite signal.
    fn compute_indicators(&self, target: &str) -> Vec<IndicatorOutput> {
        let prices = self.prices(target);
        let mut outputs = Vec::new();

        // SMA(20)
        if let Some(sma) = simple_ma(&prices, 20) {
            let current = prices.last().copied().unwrap_or(0.0);
            let signal = if current > sma { 1.0 } else { -1.0 };
            outputs.push(IndicatorOutput {
                name: "sma_20".to_string(),
                value: sma,
                signal,
                confidence: 0.6,
            });
        }

        // EMA(12)
        if let Some(ema) = exponential_ma(&prices, 12) {
            let current = prices.last().copied().unwrap_or(0.0);
            let signal = if current > ema { 1.0 } else { -1.0 };
            outputs.push(IndicatorOutput {
                name: "ema_12".to_string(),
                value: ema,
                signal,
                confidence: 0.65,
            });
        }

        // RSI(14)
        if let Some(rsi_value) = rsi(&prices, 14) {
            let signal = if rsi_value > 70.0 {
                -1.0 // Overbought
            } else if rsi_value < 30.0 {
                1.0 // Oversold
            } else {
                0.0 // Neutral
            };
            outputs.push(IndicatorOutput {
                name: "rsi_14".to_string(),
                value: rsi_value,
                signal,
                confidence: 0.7,
            });
        }

        // Bollinger Bands(20, 2)
        if let Some(bb) = bollinger_bands(&prices, 20) {
            let signal = if bb.percent_b > 1.0 {
                -0.8 // Above upper band
            } else if bb.percent_b < 0.0 {
                0.8 // Below lower band
            } else {
                (0.5 - bb.percent_b) * 2.0 // Mean-reversion signal
            };
            outputs.push(IndicatorOutput {
                name: "bollinger".to_string(),
                value: bb.percent_b,
                signal,
                confidence: 0.55,
            });
        }

        outputs
    }

    fn target_key(payload: &ChainQueryPayload) -> String {
        match &payload.target {
            roko_core::ChainTarget::Asset(s)
            | roko_core::ChainTarget::Protocol(s)
            | roko_core::ChainTarget::Pool(s)
            | roko_core::ChainTarget::Wallet(s) => s.clone(),
            _ => "unknown".to_string(),
        }
    }
}

impl Default for ChainOracle {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Oracle for ChainOracle {
    async fn predict(
        &self,
        query: &OracleQuery,
        _ctx: &Context,
    ) -> roko_core::error::Result<Prediction> {
        let payload = match &query.payload {
            QueryPayload::Chain(p) => p,
            _ => {
                return Err(roko_core::RokoError::Invalid(
                    "ChainOracle received non-chain query".into(),
                ));
            }
        };

        let target = Self::target_key(payload);
        let indicators = self.compute_indicators(&target);

        if indicators.is_empty() {
            // Insufficient data — return a low-confidence neutral prediction.
            let prediction = Prediction::new(
                query.id,
                PredictedValue::Probability(0.5),
                0.1,
                query.created_at_ms + query.horizon.as_millis() as i64,
                PredictionProvenance::new("chain_oracle", "chain_oracle_v1"),
            )
            .with_domain(OracleDomain::Chain);
            return Ok(prediction);
        }

        // Weighted consensus of indicator signals.
        let total_weight: f64 = indicators.iter().map(|i| i.confidence).sum();
        let weighted_signal: f64 = indicators
            .iter()
            .map(|i| i.signal * i.confidence)
            .sum::<f64>()
            / total_weight.max(f64::EPSILON);

        // Convert signal to a probability (bullish probability).
        let probability = (0.5 + weighted_signal * 0.3).clamp(0.05, 0.95);
        let confidence = (total_weight / indicators.len() as f64).clamp(0.1, 0.9);

        let resolve_by = query.created_at_ms + query.horizon.as_millis() as i64;

        let value = match payload.metric {
            ChainMetric::Price => {
                let current = self.prices(&target).last().copied().unwrap_or(0.0);
                let predicted = current * (1.0 + weighted_signal * 0.02);
                PredictedValue::Numeric(predicted)
            }
            ChainMetric::Gas | ChainMetric::Volatility | ChainMetric::LiquidityDepth
            | ChainMetric::MevOpportunity | ChainMetric::ProtocolHealth
            | ChainMetric::FundingRate => PredictedValue::Probability(probability),
            _ => PredictedValue::Probability(probability),
        };

        let mut prediction = Prediction::new(
            query.id,
            value,
            confidence,
            resolve_by,
            PredictionProvenance::new("chain_oracle", "chain_oracle_v1"),
        )
        .with_domain(OracleDomain::Chain);

        // Attach a 90% prediction interval for price predictions.
        if let PredictedValue::Numeric(predicted) = prediction.value {
            let spread = predicted * 0.05;
            prediction = prediction.with_interval(PredictionInterval::new(
                predicted - spread,
                predicted + spread,
                0.90,
            ));
        }

        Ok(prediction)
    }

    async fn evaluate(
        &self,
        prediction: &Prediction,
        outcome: &Engram,
    ) -> roko_core::error::Result<PredictionAccuracy> {
        let predicted = prediction.value.as_f64().unwrap_or(0.5);

        // Extract the actual value from the outcome engram body.
        let actual = outcome
            .body
            .as_text()
            .ok()
            .and_then(|text| text.parse::<f64>().ok())
            .unwrap_or(0.5);

        let residual = predicted - actual;
        let accuracy = 1.0 - residual.abs().min(1.0);

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
            OracleDomain::Chain,
            "chain",
        )
        .with_interval_hit(interval_hit)
        .with_resolution_lag_ms(resolution_lag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{ChainTarget, Context, OracleQuery, QueryPayload};
    use std::time::Duration;

    fn make_chain_query(metric: ChainMetric) -> OracleQuery {
        OracleQuery::new(
            OracleDomain::Chain,
            QueryPayload::Chain(ChainQueryPayload {
                target: ChainTarget::Asset("ETH".to_string()),
                metric,
                conditions: Vec::new(),
            }),
            Duration::from_secs(3600),
            0.5,
        )
    }

    #[tokio::test]
    async fn chain_oracle_predict_with_insufficient_data() {
        let oracle = ChainOracle::new();
        let query = make_chain_query(ChainMetric::Price);
        let ctx = Context::default();
        let prediction = oracle.predict(&query, &ctx).await.unwrap();

        assert_eq!(prediction.confidence, 0.1);
        assert_eq!(prediction.domain, Some(OracleDomain::Chain));
    }

    #[tokio::test]
    async fn chain_oracle_predict_with_price_data() {
        let oracle = ChainOracle::new();

        // Feed 30 price points.
        for i in 0..30 {
            oracle.observe_price("ETH", 2000.0 + (i as f64) * 10.0);
        }

        let query = make_chain_query(ChainMetric::Price);
        let ctx = Context::default();
        let prediction = oracle.predict(&query, &ctx).await.unwrap();

        assert!(prediction.confidence > 0.1);
        assert!(prediction.interval.is_some());
        if let PredictedValue::Numeric(value) = &prediction.value {
            assert!(*value > 0.0);
        } else {
            panic!("Expected numeric prediction for price metric");
        }
    }

    #[test]
    fn rsi_computation() {
        // Construct a series where prices go up then down.
        let mut prices: Vec<f64> = (0..20).map(|i| 100.0 + i as f64).collect();
        let result = rsi(&prices, 14).unwrap();
        assert!(result > 50.0, "Uptrend RSI should be > 50, got {result}");

        // Add a downtrend.
        for i in 0..10 {
            prices.push(119.0 - i as f64 * 2.0);
        }
        let result = rsi(&prices, 14).unwrap();
        assert!(result < 60.0, "Mixed trend RSI should be moderate, got {result}");
    }

    #[test]
    fn bollinger_bands_computation() {
        let prices: Vec<f64> = (0..25).map(|i| 100.0 + (i as f64).sin() * 5.0).collect();
        let bb = bollinger_bands(&prices, 20).unwrap();
        assert!(bb.upper > bb.middle);
        assert!(bb.lower < bb.middle);
        assert!((0.0..=1.5).contains(&bb.percent_b));
    }

    #[test]
    fn simple_ma_computation() {
        let prices = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        assert_eq!(simple_ma(&prices, 5), Some(30.0));
        assert_eq!(simple_ma(&prices, 3), Some(40.0));
        assert_eq!(simple_ma(&prices, 6), None);
    }
}
