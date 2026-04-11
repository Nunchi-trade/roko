//! Rolling latency statistics for model/provider pairs.
//!
//! This module tracks a small recent window of observed response latencies
//! together with exponential moving averages for time-to-first-token,
//! total latency, and output throughput.

use serde::{Deserialize, Serialize};

/// Rolling latency statistics for one model routed through one provider.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LatencyStats {
    /// Model slug this history belongs to.
    pub model_slug: String,
    /// Provider identifier this history belongs to.
    pub provider_id: String,
    /// Exponential moving average of time to first token, in milliseconds.
    pub ttft_ema_ms: f64,
    /// Exponential moving average of total response latency, in milliseconds.
    pub total_latency_ema_ms: f64,
    /// Exponential moving average of output throughput, in tokens per second.
    pub tokens_per_second_ema: f64,
    /// Number of observations recorded for this model/provider pair.
    pub observations: u64,
    /// Last 100 total latencies, used for percentile calculations.
    pub recent_latencies: Vec<f64>,
}

impl LatencyStats {
    /// Record a new latency observation.
    pub fn record(&mut self, ttft_ms: f64, total_ms: f64, output_tokens: u64) {
        let alpha = 0.1;
        self.ttft_ema_ms = alpha * ttft_ms + (1.0 - alpha) * self.ttft_ema_ms;
        self.total_latency_ema_ms = alpha * total_ms + (1.0 - alpha) * self.total_latency_ema_ms;
        if total_ms > 0.0 && output_tokens > 0 {
            let tps = output_tokens as f64 / (total_ms / 1000.0);
            self.tokens_per_second_ema = alpha * tps + (1.0 - alpha) * self.tokens_per_second_ema;
        }
        self.observations += 1;
        self.recent_latencies.push(total_ms);
        if self.recent_latencies.len() > 100 {
            self.recent_latencies.remove(0);
        }
    }

    /// Return the p50 latency in milliseconds.
    pub fn p50_ms(&self) -> f64 {
        self.percentile(0.50)
    }

    /// Return the p95 latency in milliseconds.
    pub fn p95_ms(&self) -> f64 {
        self.percentile(0.95)
    }

    /// Return the p99 latency in milliseconds.
    pub fn p99_ms(&self) -> f64 {
        self.percentile(0.99)
    }

    fn percentile(&self, quantile: f64) -> f64 {
        if self.recent_latencies.is_empty() {
            return 0.0;
        }

        let mut latencies = self.recent_latencies.clone();
        latencies.sort_by(|a, b| a.total_cmp(b));

        let clamped = quantile.clamp(0.0, 1.0);
        let idx = ((latencies.len() as f64) * clamped).floor() as usize;
        let idx = idx.min(latencies.len().saturating_sub(1));
        latencies[idx]
    }
}

#[cfg(test)]
mod tests {
    use super::LatencyStats;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn latency_stats_record_updates_ema_and_percentiles() {
        let mut stats = LatencyStats {
            model_slug: "glm-5.1".to_string(),
            provider_id: "zai".to_string(),
            ..Default::default()
        };

        stats.record(100.0, 200.0, 50);
        stats.record(200.0, 100.0, 100);

        assert_close(stats.ttft_ema_ms, 29.0);
        assert_close(stats.total_latency_ema_ms, 28.0);
        assert_close(stats.tokens_per_second_ema, 122.5);
        assert_eq!(stats.observations, 2);
        assert_eq!(stats.recent_latencies, vec![200.0, 100.0]);
        assert_close(stats.p50_ms(), 200.0);
        assert_close(stats.p95_ms(), 200.0);
        assert_close(stats.p99_ms(), 200.0);
    }

    #[test]
    fn latency_stats_keeps_last_hundred_samples() {
        let mut stats = LatencyStats::default();

        for i in 0..105 {
            stats.record(i as f64, i as f64, 1);
        }

        assert_eq!(stats.observations, 105);
        assert_eq!(stats.recent_latencies.len(), 100);
        assert_close(stats.recent_latencies[0], 5.0);
        assert_close(stats.recent_latencies[99], 104.0);
    }
}
