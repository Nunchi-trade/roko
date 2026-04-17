//! Adaptive gate threshold tuning based on historical pass rates.
//!
//! Uses exponential moving averages (EMA) per gate rung to track pass rates
//! and suggest retry budgets and skip decisions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::Path;

/// EMA decay factor. 0.1 means recent observations weigh more heavily.
const EMA_ALPHA: f64 = 0.1;

/// Floor for suggested retries — never go below this.
const MIN_RETRIES: u32 = 1;
/// Ceiling for suggested retries — never exceed this.
const MAX_RETRIES: u32 = 5;

/// Number of consecutive passes required before suggesting a rung skip.
const SKIP_STREAK_THRESHOLD: u32 = 20;

/// Per-rung statistics tracked by the adaptive threshold system.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RungStats {
    /// Exponential moving average of the pass rate (0.0 to 1.0).
    pub ema_pass_rate: f64,
    /// Total observations for this rung.
    pub total_observations: u64,
    /// Consecutive passes (reset on any failure).
    pub consecutive_passes: u32,
}

impl Default for RungStats {
    fn default() -> Self {
        Self {
            ema_pass_rate: 0.5, // Start neutral.
            total_observations: 0,
            consecutive_passes: 0,
        }
    }
}

/// Adaptive gate thresholds: per-rung EMA of pass rates with floor/ceiling bounds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveThresholds {
    /// Per-rung statistics, keyed by rung number.
    #[serde(default)]
    rungs: HashMap<u32, RungStats>,
}

impl AdaptiveThresholds {
    /// Create a new empty set of adaptive thresholds.
    pub fn new() -> Self {
        Self {
            rungs: HashMap::new(),
        }
    }

    /// Load from a JSON file.
    ///
    /// Returns `NotFound` if the file does not exist and `InvalidData` if the
    /// file exists but does not contain valid adaptive-threshold JSON.
    pub fn load(path: &Path) -> Result<Self, io::Error> {
        let file = std::fs::File::open(path)?;
        serde_json::from_reader(file).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }

    /// Load from a JSON file, or create new if missing/corrupt.
    pub fn load_or_new(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    /// Save to a JSON file (atomic write).
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let mut tmp_file = std::fs::File::create(&tmp)?;
        tmp_file.write_all(json.as_bytes())?;
        tmp_file.sync_all()?;
        drop(tmp_file);
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Return the current threshold for a rung.
    ///
    /// Unknown rungs default to the neutral threshold of `0.5`.
    pub fn threshold_for(&self, rung: u32) -> f64 {
        self.rungs
            .get(&rung)
            .map_or(0.5, |stats| stats.ema_pass_rate)
    }

    /// Update statistics for a rung after a gate run.
    pub fn observe(&mut self, rung: u32, passed: bool) {
        let stats = self.rungs.entry(rung).or_default();
        let value = if passed { 1.0 } else { 0.0 };

        if stats.total_observations == 0 {
            stats.ema_pass_rate = value;
        } else {
            stats.ema_pass_rate = EMA_ALPHA.mul_add(value, (1.0 - EMA_ALPHA) * stats.ema_pass_rate);
        }

        stats.total_observations += 1;

        if passed {
            stats.consecutive_passes += 1;
        } else {
            stats.consecutive_passes = 0;
        }
    }

    /// Backwards-compatible alias for `observe`.
    pub fn update(&mut self, rung: u32, passed: bool) {
        self.observe(rung, passed);
    }

    /// Suggest a maximum retry count for a rung based on its historical pass rate.
    ///
    /// High pass rate → fewer retries needed (the gate usually passes).
    /// Low pass rate → more retries allowed (the gate often fails, give it more chances).
    pub fn suggested_max_retries(&self, rung: u32) -> u32 {
        let Some(stats) = self.rungs.get(&rung) else {
            return 3; // Default for unknown rungs.
        };

        if stats.total_observations < 5 {
            return 3; // Not enough data yet.
        }

        // Map pass rate to retry count: high pass → low retries, low pass → high retries.
        // pass_rate 1.0 → 1 retry, pass_rate 0.0 → 5 retries.
        let max_f = f64::from(MAX_RETRIES);
        let range_f = f64::from(MAX_RETRIES - MIN_RETRIES);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let retries = stats.ema_pass_rate.mul_add(-range_f, max_f).round() as u32;

        retries.clamp(MIN_RETRIES, MAX_RETRIES)
    }

    /// Advisory: should this rung be skipped?
    ///
    /// Returns `true` if the rung has passed consecutively at least
    /// `SKIP_STREAK_THRESHOLD` times, suggesting it's always passing and
    /// could be skipped to save time. The caller should treat this as
    /// advisory and still run the rung periodically.
    pub fn should_skip_rung(&self, rung: u32) -> bool {
        self.rungs
            .get(&rung)
            .is_some_and(|s| s.consecutive_passes >= SKIP_STREAK_THRESHOLD)
    }

    /// Get stats for a specific rung (for reporting).
    pub fn rung_stats(&self, rung: u32) -> Option<&RungStats> {
        self.rungs.get(&rung)
    }

    /// Iterate over all tracked rungs.
    pub fn all_rungs(&self) -> impl Iterator<Item = (&u32, &RungStats)> {
        self.rungs.iter()
    }
}

impl Default for AdaptiveThresholds {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rung_starts_neutral() {
        let at = AdaptiveThresholds::new();
        assert_eq!(at.threshold_for(0), 0.5);
        assert_eq!(at.suggested_max_retries(0), 3); // Default for unknown.
        assert!(!at.should_skip_rung(0));
    }

    #[test]
    fn high_pass_rate_reduces_retries() {
        let mut at = AdaptiveThresholds::new();
        for _ in 0..20 {
            at.update(1, true);
        }
        // With ~100% pass rate, should suggest 1 retry.
        assert_eq!(at.suggested_max_retries(1), MIN_RETRIES);
    }

    #[test]
    fn low_pass_rate_increases_retries() {
        let mut at = AdaptiveThresholds::new();
        for _ in 0..20 {
            at.update(2, false);
        }
        // With ~0% pass rate, should suggest max retries.
        assert_eq!(at.suggested_max_retries(2), MAX_RETRIES);
    }

    #[test]
    fn consecutive_passes_trigger_skip() {
        let mut at = AdaptiveThresholds::new();
        for _ in 0..SKIP_STREAK_THRESHOLD {
            at.update(3, true);
        }
        assert!(at.should_skip_rung(3));
    }

    #[test]
    fn failure_resets_skip_streak() {
        let mut at = AdaptiveThresholds::new();
        for _ in 0..19 {
            at.update(4, true);
        }
        at.update(4, false); // Reset streak.
        assert!(!at.should_skip_rung(4));
    }

    #[test]
    fn round_trip_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gate-thresholds.json");

        let mut at = AdaptiveThresholds::new();
        for _ in 0..10 {
            at.update(1, true);
        }
        at.save(&path).unwrap();

        let loaded = AdaptiveThresholds::load_or_new(&path);
        assert_eq!(loaded.rung_stats(1).unwrap().total_observations, 10);
    }
}
