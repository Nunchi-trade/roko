//! Rolling latency statistics for model/provider pairs.
//!
//! This module tracks a small recent window of observed response latencies
//! together with exponential moving averages for time-to-first-token,
//! total latency, and output throughput. A thread-safe registry persists the
//! stats per `(model, provider)` pair.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::Path;

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
    pub recent_latencies: VecDeque<f64>,
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
        self.recent_latencies.push_back(total_ms);
        if self.recent_latencies.len() > 100 {
            self.recent_latencies.pop_front();
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

        let mut latencies: Vec<f64> = self.recent_latencies.iter().copied().collect();
        latencies.sort_by(|a, b| a.total_cmp(b));

        let clamped = quantile.clamp(0.0, 1.0);
        let idx = ((latencies.len() as f64) * clamped).floor() as usize;
        let idx = idx.min(latencies.len().saturating_sub(1));
        latencies[idx]
    }
}

/// Persisted snapshot for [`LatencyRegistry`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LatencyRegistrySnapshot {
    /// All tracked latency records.
    entries: Vec<LatencyRegistryEntry>,
}

/// Persisted entry for one `(model, provider)` pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LatencyRegistryEntry {
    /// Model slug this entry belongs to.
    model: String,
    /// Provider identifier this entry belongs to.
    provider: String,
    /// Stats for this model/provider pair.
    stats: LatencyStats,
}

/// Centralized latency registry keyed by `(model, provider)`.
pub struct LatencyRegistry {
    stats: Mutex<HashMap<(String, String), LatencyStats>>,
}

impl LatencyRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            stats: Mutex::new(HashMap::new()),
        }
    }

    /// Record a latency observation for `model` routed through `provider`.
    pub fn record(&self, model: &str, provider: &str, ttft_ms: f64, total_ms: f64, tokens: u64) {
        let mut stats = self.stats.lock();
        let key = (model.to_owned(), provider.to_owned());
        let entry = stats.entry(key).or_insert_with(|| LatencyStats {
            model_slug: model.to_owned(),
            provider_id: provider.to_owned(),
            ..Default::default()
        });
        entry.record(ttft_ms, total_ms, tokens);
    }

    /// Return the recorded stats for a `(model, provider)` pair.
    #[must_use]
    pub fn get(&self, model: &str, provider: &str) -> Option<LatencyStats> {
        self.stats
            .lock()
            .get(&(model.to_owned(), provider.to_owned()))
            .cloned()
    }

    /// Persist the registry to `path` as JSON.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut entries: Vec<_> = self
            .stats
            .lock()
            .iter()
            .map(|((model, provider), stats)| LatencyRegistryEntry {
                model: model.clone(),
                provider: provider.clone(),
                stats: stats.clone(),
            })
            .collect();
        entries.sort_by(|a, b| a.model.cmp(&b.model).then(a.provider.cmp(&b.provider)));

        let snapshot = LatencyRegistrySnapshot { entries };
        let json = serde_json::to_string_pretty(&snapshot)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let tmp_path = path.with_extension("json.tmp");
        std::fs::write(&tmp_path, json)?;
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    }

    /// Load the registry from `path`, or return an empty registry.
    #[must_use]
    pub fn load_or_new(path: &Path) -> Self {
        let snapshot = std::fs::read_to_string(path)
            .ok()
            .and_then(|contents| serde_json::from_str::<LatencyRegistrySnapshot>(&contents).ok());

        match snapshot {
            Some(snapshot) => {
                let mut stats = HashMap::new();
                for entry in snapshot.entries {
                    stats.insert((entry.model, entry.provider), entry.stats);
                }
                Self {
                    stats: Mutex::new(stats),
                }
            }
            None => Self::new(),
        }
    }
}

impl Default for LatencyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{LatencyRegistry, LatencyStats};
    use std::collections::VecDeque;
    use tempfile::tempdir;

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
        assert_eq!(stats.recent_latencies, VecDeque::from(vec![200.0, 100.0]));
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

    #[test]
    fn latency_registry_tracks_pairs_independently() {
        let registry = LatencyRegistry::new();

        registry.record("glm-5.1", "zai", 100.0, 200.0, 50);
        registry.record("glm-5.1", "openrouter", 300.0, 400.0, 25);
        registry.record("glm-5.1", "zai", 200.0, 300.0, 75);

        let zai = registry.get("glm-5.1", "zai").expect("zai stats");
        let openrouter = registry
            .get("glm-5.1", "openrouter")
            .expect("openrouter stats");

        assert_eq!(zai.model_slug, "glm-5.1");
        assert_eq!(zai.provider_id, "zai");
        assert_eq!(zai.observations, 2);
        assert_eq!(zai.recent_latencies, VecDeque::from(vec![200.0, 300.0]));
        assert_eq!(openrouter.model_slug, "glm-5.1");
        assert_eq!(openrouter.provider_id, "openrouter");
        assert_eq!(openrouter.observations, 1);
        assert_eq!(openrouter.recent_latencies, VecDeque::from(vec![400.0]));
    }

    #[test]
    fn latency_registry_persists_round_trip() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join(".roko/learn/latency-stats.json");

        let registry = LatencyRegistry::new();
        registry.record("glm-5.1", "zai", 120.0, 240.0, 60);
        registry.record("claude-sonnet-4-6", "anthropic", 80.0, 160.0, 40);
        registry.save(&path).expect("save");

        let loaded = LatencyRegistry::load_or_new(&path);
        let zai = loaded.get("glm-5.1", "zai").expect("zai stats");
        let anthropic = loaded
            .get("claude-sonnet-4-6", "anthropic")
            .expect("anthropic stats");

        assert_eq!(zai.model_slug, "glm-5.1");
        assert_eq!(zai.provider_id, "zai");
        assert_eq!(zai.observations, 1);
        assert_eq!(zai.recent_latencies, VecDeque::from(vec![240.0]));
        assert_eq!(anthropic.model_slug, "claude-sonnet-4-6");
        assert_eq!(anthropic.provider_id, "anthropic");
        assert_eq!(anthropic.observations, 1);
        assert_eq!(anthropic.recent_latencies, VecDeque::from(vec![160.0]));
    }
}
