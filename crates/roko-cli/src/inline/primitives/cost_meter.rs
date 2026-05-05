//! Primitive 5: `CostMeter` — real-time cost tracking across a session.
//!
//! Tracks cumulative cost, token counts, and model usage across multiple
//! runs/turns. Displays as a status bar and can produce a waterfall summary.

use std::collections::HashMap;

/// Tracks cumulative cost and token usage across a session.
#[derive(Debug, Clone)]
pub struct CostMeter {
    /// Total cost in USD.
    pub total_cost: f64,
    /// Total input tokens.
    pub input_tokens: u64,
    /// Total output tokens.
    pub output_tokens: u64,
    /// Total cache hits.
    pub cache_hits: u64,
    /// Total cache misses.
    pub cache_misses: u64,
    /// Per-model token counts.
    pub model_tokens: HashMap<String, u64>,
    /// Number of completed runs.
    pub run_count: u32,
    /// Naive baseline cost (what it would have cost at full Opus rates).
    pub naive_baseline: f64,
}

impl CostMeter {
    /// Create a new empty meter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            total_cost: 0.0,
            input_tokens: 0,
            output_tokens: 0,
            cache_hits: 0,
            cache_misses: 0,
            model_tokens: HashMap::new(),
            run_count: 0,
            naive_baseline: 0.0,
        }
    }

    /// Record a completed run.
    pub fn record_run(&mut self, cost: f64, input: u64, output: u64, model: &str, naive_cost: f64) {
        self.total_cost += cost;
        self.input_tokens += input;
        self.output_tokens += output;
        self.naive_baseline += naive_cost;
        self.run_count += 1;
        *self.model_tokens.entry(model.to_string()).or_default() += input + output;
    }

    /// Record a cache event.
    pub fn record_cache(&mut self, hit: bool) {
        if hit {
            self.cache_hits += 1;
        } else {
            self.cache_misses += 1;
        }
    }

    /// Cache hit rate as a percentage (0-100).
    #[must_use]
    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            return 0.0;
        }
        (self.cache_hits as f64 / total as f64) * 100.0
    }

    /// Savings ratio (naive_baseline / total_cost). Returns 1.0 if no cost.
    #[must_use]
    pub fn savings_ratio(&self) -> f64 {
        if self.total_cost <= 0.0 {
            return 1.0;
        }
        self.naive_baseline / self.total_cost
    }

    /// The primary model used (by token count).
    #[must_use]
    pub fn primary_model(&self) -> Option<&str> {
        self.model_tokens
            .iter()
            .max_by_key(|(_, count)| **count)
            .map(|(name, _)| name.as_str())
    }

    /// Total token count.
    #[must_use]
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

impl Default for CostMeter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_meter_new() {
        let meter = CostMeter::new();
        assert_eq!(meter.total_cost, 0.0);
        assert_eq!(meter.run_count, 0);
        assert_eq!(meter.savings_ratio(), 1.0);
    }

    #[test]
    fn cost_meter_record() {
        let mut meter = CostMeter::new();
        meter.record_run(0.031, 4821, 1203, "haiku", 0.91);
        assert_eq!(meter.run_count, 1);
        assert!((meter.total_cost - 0.031).abs() < f64::EPSILON);
        assert_eq!(meter.input_tokens, 4821);
        assert_eq!(meter.output_tokens, 1203);
        assert_eq!(meter.primary_model(), Some("haiku"));
    }

    #[test]
    fn cost_meter_savings() {
        let mut meter = CostMeter::new();
        meter.record_run(0.031, 4821, 1203, "haiku", 0.93);
        let ratio = meter.savings_ratio();
        assert!(ratio > 25.0, "expected >25x savings, got {ratio}x");
    }

    #[test]
    fn cost_meter_cache_rate() {
        let mut meter = CostMeter::new();
        for _ in 0..87 {
            meter.record_cache(true);
        }
        for _ in 0..13 {
            meter.record_cache(false);
        }
        let rate = meter.cache_hit_rate();
        assert!((rate - 87.0).abs() < 0.1);
    }
}
