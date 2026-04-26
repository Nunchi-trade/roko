//! Adaptive gate threshold tuning based on historical pass rates.
//!
//! Uses exponential moving averages (EMA) per gate rung to track pass rates
//! and suggest retry budgets and skip decisions.

use crate::hotelling::HotellingDetector;
use crate::spc::{SpcAlert, SpcDetector};
use roko_core::Temperament;
use roko_core::config::AgentThresholds;
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
    /// CUSUM high accumulator (detects upward shifts in pass rate).
    pub cusum_high: f64,
    /// CUSUM low accumulator (detects downward shifts in pass rate).
    pub cusum_low: f64,
    /// Whether CUSUM has detected a shift since last reset.
    pub cusum_shift_detected: bool,
}

impl Default for RungStats {
    fn default() -> Self {
        Self {
            ema_pass_rate: 0.5, // Start neutral.
            total_observations: 0,
            consecutive_passes: 0,
            cusum_high: 0.0,
            cusum_low: 0.0,
            cusum_shift_detected: false,
        }
    }
}

/// Default CUSUM sensitivity parameter (slack allowance).
///
/// Per spec (docs/04-verification/06-adaptive-thresholds.md): k=0.25.
/// This balances detection speed against false alarm rate for gate pipelines.
const DEFAULT_CUSUM_SENSITIVITY: f64 = 0.25;

/// Default CUSUM decision threshold.
/// CUSUM signals a shift when the accumulator exceeds this value.
const DEFAULT_CUSUM_THRESHOLD: f64 = 4.0;

/// Domain-specific threshold profile with role-specific priors (P1-14).
///
/// Different agent domains (coding, research, security, etc.) have different
/// baseline expectations for gate pass rates. A research agent may tolerate
/// higher failure rates on compile gates but demand stricter test coverage.
///
/// Profiles override the default neutral priors with domain-informed values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThresholdProfile {
    /// Profile name (e.g., "coding", "research", "security").
    pub name: String,
    /// Prior pass rate for each rung (overrides default 0.5 neutral).
    /// Key = rung number, Value = expected pass rate [0.0, 1.0].
    pub rung_priors: HashMap<u32, f64>,
    /// Floor multiplier: how much to tighten/relax the floor threshold.
    /// > 1.0 = stricter floors, < 1.0 = more lenient floors.
    pub floor_multiplier: f64,
    /// Maximum retries multiplier for this domain.
    pub retry_multiplier: f64,
    /// CUSUM sensitivity override (None = use default).
    pub cusum_sensitivity_override: Option<f64>,
}

impl ThresholdProfile {
    /// Coding domain: strict compile/clippy, moderate test tolerance.
    #[must_use]
    pub fn coding() -> Self {
        let mut priors = HashMap::new();
        priors.insert(0, 0.90); // compile: should almost always pass
        priors.insert(1, 0.80); // clippy: usually passes
        priors.insert(2, 0.65); // test: moderate expectation
        priors.insert(3, 0.50); // diff review: neutral
        Self {
            name: "coding".into(),
            rung_priors: priors,
            floor_multiplier: 1.0,
            retry_multiplier: 1.0,
            cusum_sensitivity_override: None,
        }
    }

    /// Research domain: lenient compile, strict correctness.
    #[must_use]
    pub fn research() -> Self {
        let mut priors = HashMap::new();
        priors.insert(0, 0.70); // compile: may experiment
        priors.insert(1, 0.60); // clippy: flexible
        priors.insert(2, 0.85); // test: must be correct
        priors.insert(3, 0.40); // diff: exploratory
        Self {
            name: "research".into(),
            rung_priors: priors,
            floor_multiplier: 0.8,
            retry_multiplier: 1.5, // more retries for research
            cusum_sensitivity_override: Some(0.30),
        }
    }

    /// Security domain: strict everything, few retries.
    #[must_use]
    pub fn security() -> Self {
        let mut priors = HashMap::new();
        priors.insert(0, 0.95); // compile: must pass
        priors.insert(1, 0.90); // clippy: strict
        priors.insert(2, 0.90); // test: strict
        priors.insert(3, 0.80); // diff: careful review
        Self {
            name: "security".into(),
            rung_priors: priors,
            floor_multiplier: 1.3,                  // stricter floors
            retry_multiplier: 0.7,                  // fewer retries
            cusum_sensitivity_override: Some(0.15), // more sensitive to shifts
        }
    }

    /// Get the prior for a specific rung, falling back to 0.5 neutral.
    #[must_use]
    pub fn prior_for_rung(&self, rung: u32) -> f64 {
        self.rung_priors.get(&rung).copied().unwrap_or(0.5)
    }

    /// Look up a profile by name.
    #[must_use]
    pub fn by_name(name: &str) -> Self {
        match name {
            "coding" => Self::coding(),
            "research" => Self::research(),
            "security" => Self::security(),
            _ => Self {
                name: name.into(),
                rung_priors: HashMap::new(),
                floor_multiplier: 1.0,
                retry_multiplier: 1.0,
                cusum_sensitivity_override: None,
            },
        }
    }
}

/// Adaptive gate thresholds: per-rung EMA of pass rates with floor/ceiling bounds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveThresholds {
    /// Per-rung statistics, keyed by rung number.
    #[serde(default)]
    rungs: HashMap<u32, RungStats>,
    /// CUSUM sensitivity parameter (configurable, default 0.05).
    #[serde(default = "default_cusum_sensitivity")]
    cusum_sensitivity: f64,
    /// CUSUM decision threshold (configurable, default 4.0).
    #[serde(default = "default_cusum_threshold")]
    cusum_threshold: f64,
    /// Per-rung SPC detectors (CUSUM + EWMA Control Chart + BOCPD).
    /// Wired in GATE-01: each `observe()` call feeds through all three
    /// detectors and collects any alerts.
    #[serde(default)]
    spc_detectors: HashMap<u32, SpcDetector>,
    /// Multi-gate joint anomaly detector (GATE-08).
    /// Tracks the full pass-rate vector across rungs and detects systemic
    /// shifts via Hotelling's T-squared statistic.
    #[serde(skip)]
    hotelling: Option<HotellingDetector>,
    /// SPC alerts accumulated since last drain.
    #[serde(skip)]
    pending_spc_alerts: Vec<(u32, SpcAlert)>,
    /// Whether the last full-pipeline observation triggered a joint anomaly.
    #[serde(skip)]
    joint_anomaly_detected: bool,
}

fn default_cusum_sensitivity() -> f64 {
    DEFAULT_CUSUM_SENSITIVITY
}

fn default_cusum_threshold() -> f64 {
    DEFAULT_CUSUM_THRESHOLD
}

impl AdaptiveThresholds {
    /// Create a new empty set of adaptive thresholds.
    pub fn new() -> Self {
        Self {
            rungs: HashMap::new(),
            cusum_sensitivity: DEFAULT_CUSUM_SENSITIVITY,
            cusum_threshold: DEFAULT_CUSUM_THRESHOLD,
            spc_detectors: HashMap::new(),
            hotelling: None,
            pending_spc_alerts: Vec::new(),
            joint_anomaly_detected: false,
        }
    }

    /// Override the CUSUM sensitivity parameter.
    ///
    /// Smaller values detect smaller shifts sooner but may produce more false
    /// alarms. Typical range: 0.01 to 0.1.
    #[must_use]
    pub fn with_cusum_sensitivity(mut self, sensitivity: f64) -> Self {
        self.cusum_sensitivity = if sensitivity.is_finite() && sensitivity > 0.0 {
            sensitivity
        } else {
            DEFAULT_CUSUM_SENSITIVITY
        };
        self
    }

    /// Override the CUSUM decision threshold.
    ///
    /// Larger values require more evidence before signaling a shift. Typical
    /// range: 2.0 to 8.0.
    #[must_use]
    pub fn with_cusum_threshold(mut self, threshold: f64) -> Self {
        self.cusum_threshold = if threshold.is_finite() && threshold > 0.0 {
            threshold
        } else {
            DEFAULT_CUSUM_THRESHOLD
        };
        self
    }

    /// Load from a JSON file.
    ///
    /// Returns `NotFound` if the file does not exist and `InvalidData` if the
    /// file exists but does not contain valid adaptive-threshold JSON.
    ///
    /// # Errors
    ///
    /// Returns any filesystem error from opening `path`, or
    /// [`io::ErrorKind::InvalidData`] if the file contents are not valid
    /// adaptive-threshold JSON.
    pub fn load(path: &Path) -> Result<Self, io::Error> {
        let file = std::fs::File::open(path)?;
        serde_json::from_reader(file).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }

    /// Load from a JSON file, or create new if missing/corrupt.
    pub fn load_or_new(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    /// Save to a JSON file (atomic write).
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot cannot be serialized, the parent
    /// directory cannot be created, or the temporary/output files cannot be
    /// written and renamed atomically.
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

    /// Apply a role-local threshold floor over the adaptive EMA baseline.
    #[must_use]
    pub fn override_for_role(
        &self,
        _role: &str,
        thresholds: Option<&AgentThresholds>,
        rung: u32,
    ) -> f64 {
        let nominal = self.threshold_for(rung);
        let Some(floor) = thresholds
            .and_then(|thresholds| thresholds.gate_pass_rate_floor)
            .filter(|floor| floor.is_finite())
        else {
            return nominal;
        };
        nominal.max(floor.clamp(0.0, 1.0))
    }

    /// Update statistics for a rung after a gate run.
    ///
    /// Updates EMA pass rate, consecutive pass streak, CUSUM accumulators,
    /// and feeds the observation to the per-rung SPC detector ensemble
    /// (CUSUM + EWMA Control Chart + BOCPD). When any detector fires, the
    /// alert is collected and can be drained via [`Self::drain_spc_alerts`].
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

        // CUSUM change detection.
        // Deviation from the current EMA baseline.
        let deviation = value - stats.ema_pass_rate;

        // Accumulate upward shifts: detect improvement in pass rate.
        stats.cusum_high = (stats.cusum_high + deviation - self.cusum_sensitivity).max(0.0);
        // Accumulate downward shifts: detect degradation in pass rate.
        stats.cusum_low = (stats.cusum_low - deviation - self.cusum_sensitivity).max(0.0);

        // Check if either accumulator exceeds the decision threshold.
        if stats.cusum_high > self.cusum_threshold || stats.cusum_low > self.cusum_threshold {
            stats.cusum_shift_detected = true;
            // Reset EMA to the current observation to adapt quickly.
            stats.ema_pass_rate = value;
            // Reset CUSUM accumulators after detection.
            stats.cusum_high = 0.0;
            stats.cusum_low = 0.0;
        } else {
            stats.cusum_shift_detected = false;
        }

        // ── SPC detector ensemble (GATE-01 wiring) ──────────────────────
        // Feed the pass/fail observation to the per-rung SPC detector.
        // Lazily initialize with the current EMA as the target.
        let target = stats.ema_pass_rate;
        let spc = self
            .spc_detectors
            .entry(rung)
            .or_insert_with(|| SpcDetector::new(target, 0.1));
        let alerts = spc.update(value);
        for alert in alerts {
            self.pending_spc_alerts.push((rung, alert));
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

    /// Whether CUSUM detected a distributional shift on the last observation
    /// for the given rung.
    pub fn cusum_shift_detected(&self, rung: u32) -> bool {
        self.rungs
            .get(&rung)
            .is_some_and(|s| s.cusum_shift_detected)
    }

    /// Return the current CUSUM accumulator values for a rung.
    ///
    /// Returns `(cusum_high, cusum_low)`, or `(0.0, 0.0)` for unknown rungs.
    pub fn cusum_values(&self, rung: u32) -> (f64, f64) {
        self.rungs
            .get(&rung)
            .map(|s| (s.cusum_high, s.cusum_low))
            .unwrap_or((0.0, 0.0))
    }

    /// Drain all pending SPC alerts accumulated since the last drain.
    ///
    /// Each alert is a `(rung, SpcAlert)` pair. The caller (typically the
    /// conductor or orchestrator) should log or react to these alerts.
    pub fn drain_spc_alerts(&mut self) -> Vec<(u32, SpcAlert)> {
        std::mem::take(&mut self.pending_spc_alerts)
    }

    /// Whether any SPC alerts are pending.
    #[must_use]
    pub fn has_spc_alerts(&self) -> bool {
        !self.pending_spc_alerts.is_empty()
    }

    /// Return a reference to the per-rung SPC detector, if initialized.
    #[must_use]
    pub fn spc_detector(&self, rung: u32) -> Option<&SpcDetector> {
        self.spc_detectors.get(&rung)
    }

    /// Record a complete pipeline run for joint anomaly detection (GATE-08).
    ///
    /// `pass_rates` is a vector of pass/fail values (0.0 or 1.0) for each
    /// gate in the pipeline, in a stable order. The Hotelling detector is
    /// lazily initialized on the first call. When the T-squared statistic
    /// exceeds the chi-squared threshold, `joint_anomaly_detected` is set.
    pub fn observe_pipeline(&mut self, pass_rates: &[f64]) {
        if pass_rates.is_empty() {
            return;
        }

        let det = self
            .hotelling
            .get_or_insert_with(|| HotellingDetector::new(pass_rates.len(), 0.05));

        // Dimension mismatch: skip if the pipeline size changed.
        if pass_rates.len() != det.mean().len() {
            return;
        }

        det.update(pass_rates);
        self.joint_anomaly_detected = det.is_anomalous(pass_rates);
    }

    /// Whether the last `observe_pipeline()` call detected a joint anomaly.
    #[must_use]
    pub fn joint_anomaly_detected(&self) -> bool {
        self.joint_anomaly_detected
    }

    /// Return the Hotelling detector, if initialized.
    #[must_use]
    pub fn hotelling_detector(&self) -> Option<&HotellingDetector> {
        self.hotelling.as_ref()
    }

    /// Update the adaptive threshold for a rung based on a prediction
    /// residual (TA-15: gate threshold EMA from prediction residuals).
    ///
    /// When oracles systematically overestimate task success, the absolute
    /// residual tightens the gate threshold for the corresponding rung.
    /// This creates automatic gate tightening without manual tuning.
    pub fn observe_residual(&mut self, rung: u32, residual: f64) {
        let stats = self.rungs.entry(rung).or_default();
        let abs_residual = residual.abs().clamp(0.0, 1.0);

        // Nudge the EMA toward caution when the residual indicates
        // over-prediction (positive residual = predicted success but
        // actually failed). We use a softer alpha (0.05) to avoid
        // over-reacting to single observations.
        const RESIDUAL_ALPHA: f64 = 0.05;
        if stats.total_observations > 0 {
            // Tighten: reduce the pass-rate expectation proportionally
            // to the absolute residual magnitude.
            let adjustment = RESIDUAL_ALPHA * abs_residual;
            stats.ema_pass_rate = (stats.ema_pass_rate - adjustment).clamp(0.0, 1.0);
        }
    }

    /// Incorporate neuro-derived knowledge hints into threshold tuning
    /// (INT-15: Neuro -> Verify Thresholds).
    ///
    /// `known_failure_rungs` lists rung indices where neuro's knowledge store
    /// has recorded persistent failure patterns.  For those rungs, the
    /// thresholds are nudged toward caution: the CUSUM sensitivity is tightened
    /// so that quality regressions are detected sooner, and the EMA is biased
    /// slightly downward when few observations exist (less than 10). This
    /// prevents the adaptive system from being overly optimistic about rungs
    /// that neuro already knows are problematic.
    ///
    /// `known_stable_rungs` lists rungs where neuro knowledge confirms
    /// consistent passing.  For those, the CUSUM sensitivity is relaxed
    /// slightly to avoid false alarms.
    pub fn apply_neuro_hints(&mut self, known_failure_rungs: &[u32], known_stable_rungs: &[u32]) {
        for &rung in known_failure_rungs {
            let stats = self.rungs.entry(rung).or_default();
            // For rungs with known failure patterns, bias the EMA toward caution
            // when we have limited empirical data.
            if stats.total_observations < 10 {
                stats.ema_pass_rate = (stats.ema_pass_rate * 0.7).min(0.5);
            }
        }
        // Tighten CUSUM sensitivity for failure-prone rungs by lowering the
        // global sensitivity parameter (detects smaller shifts sooner).
        if !known_failure_rungs.is_empty() {
            let tighter = (self.cusum_sensitivity * 0.7).max(0.01);
            self.cusum_sensitivity = tighter;
        }
        // Relax CUSUM sensitivity for stable rungs (avoids false alarms).
        if !known_stable_rungs.is_empty() && known_failure_rungs.is_empty() {
            let relaxed = (self.cusum_sensitivity * 1.3).min(0.15);
            self.cusum_sensitivity = relaxed;
        }
    }

    // ─── Temperament-aware adjustments (AGT-06) ────────────────────

    /// Adjust the suggested retry count for a rung based on temperament.
    ///
    /// - **Conservative**: fewer retries (floor at 1, ceiling at 3)
    /// - **Balanced**: no adjustment (default behavior)
    /// - **Aggressive**: more retries allowed (floor at 2, ceiling at 5)
    /// - **Exploratory**: same as Aggressive
    pub fn suggested_max_retries_for_temperament(
        &self,
        rung: u32,
        temperament: Temperament,
    ) -> u32 {
        let base = self.suggested_max_retries(rung);
        match temperament {
            Temperament::Conservative => base.min(3).max(1),
            Temperament::Balanced => base,
            Temperament::Aggressive | Temperament::Exploratory => base.min(5).max(2),
        }
    }

    /// Return the effective pass-rate threshold for a rung under a given
    /// temperament.
    ///
    /// - **Conservative**: raises the threshold by 10% (stricter gates)
    /// - **Balanced**: returns the raw EMA threshold
    /// - **Aggressive**: lowers the threshold by 15% (more permissive)
    /// - **Exploratory**: lowers the threshold by 10%
    pub fn threshold_for_temperament(&self, rung: u32, temperament: Temperament) -> f64 {
        let base = self.threshold_for(rung);
        let adjusted = match temperament {
            Temperament::Conservative => base * 1.10,
            Temperament::Balanced => base,
            Temperament::Aggressive => base * 0.85,
            Temperament::Exploratory => base * 0.90,
        };
        adjusted.clamp(0.0, 1.0)
    }

    /// Advisory: should this rung be skipped under a given temperament?
    ///
    /// - **Conservative**: never skip (always run all rungs)
    /// - **Balanced**: use the default streak-based skip logic
    /// - **Aggressive**: skip more aggressively (lower streak threshold)
    /// - **Exploratory**: use the default streak-based skip logic
    pub fn should_skip_rung_for_temperament(&self, rung: u32, temperament: Temperament) -> bool {
        match temperament {
            Temperament::Conservative => false,
            Temperament::Balanced | Temperament::Exploratory => self.should_skip_rung(rung),
            Temperament::Aggressive => self
                .rungs
                .get(&rung)
                .is_some_and(|s| s.consecutive_passes >= SKIP_STREAK_THRESHOLD / 2),
        }
    }
}

impl Default for AdaptiveThresholds {
    fn default() -> Self {
        Self {
            rungs: HashMap::new(),
            cusum_sensitivity: DEFAULT_CUSUM_SENSITIVITY,
            cusum_threshold: DEFAULT_CUSUM_THRESHOLD,
            spc_detectors: HashMap::new(),
            hotelling: None,
            pending_spc_alerts: Vec::new(),
            joint_anomaly_detected: false,
        }
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
        let dir = tempfile::tempdir()
            .expect("invariant: adaptive-threshold test should create a temp directory");
        let path = dir.path().join("gate-thresholds.json");

        let mut at = AdaptiveThresholds::new();
        for _ in 0..10 {
            at.update(1, true);
        }
        at.save(&path)
            .expect("invariant: adaptive thresholds should save to the temp file");

        let loaded = AdaptiveThresholds::load_or_new(&path);
        assert_eq!(
            loaded
                .rung_stats(1)
                .expect("invariant: persisted rung stats should exist after reload")
                .total_observations,
            10
        );
    }

    #[test]
    fn cusum_detects_sudden_degradation() {
        // Start with many passes to establish a high baseline.
        let mut at = AdaptiveThresholds::new().with_cusum_threshold(2.0);
        for _ in 0..30 {
            at.update(5, true);
        }
        assert!(!at.cusum_shift_detected(5));
        let stats_before = at.rung_stats(5).unwrap().ema_pass_rate;
        assert!(stats_before > 0.9);

        // Now inject a run of failures — CUSUM should detect the shift.
        let mut detected = false;
        for _ in 0..20 {
            at.update(5, false);
            if at.cusum_shift_detected(5) {
                detected = true;
                break;
            }
        }
        assert!(detected, "CUSUM should detect degradation");
    }

    #[test]
    fn cusum_accumulators_stay_non_negative() {
        let mut at = AdaptiveThresholds::new();
        for _ in 0..10 {
            at.update(6, true);
        }
        let (high, low) = at.cusum_values(6);
        assert!(high >= 0.0);
        assert!(low >= 0.0);
    }

    #[test]
    fn cusum_configurable_sensitivity() {
        // Very high sensitivity (low slack) should detect sooner.
        let mut at = AdaptiveThresholds::new()
            .with_cusum_sensitivity(0.01)
            .with_cusum_threshold(1.0);
        for _ in 0..20 {
            at.update(7, true);
        }
        let mut detected_early = false;
        for i in 0..10 {
            at.update(7, false);
            if at.cusum_shift_detected(7) {
                detected_early = true;
                assert!(i < 8, "should detect quickly with high sensitivity");
                break;
            }
        }
        assert!(detected_early);
    }

    #[test]
    fn role_override_raises_floor_without_lowering_nominal_threshold() {
        let mut at = AdaptiveThresholds::new();
        for _ in 0..10 {
            at.update(1, false);
        }

        let strict = AgentThresholds {
            gate_pass_rate_floor: Some(0.75),
        };
        assert_eq!(at.override_for_role("implementer", Some(&strict), 1), 0.75);

        let lenient = AgentThresholds {
            gate_pass_rate_floor: Some(0.10),
        };
        assert!(at.override_for_role("implementer", Some(&lenient), 1) >= at.threshold_for(1));
    }

    // ─── Temperament tests (AGT-06) ────────────────────────────────

    #[test]
    fn conservative_temperament_fewer_retries() {
        let mut at = AdaptiveThresholds::new();
        for _ in 0..10 {
            at.update(1, false); // Low pass rate = more retries
        }
        let base = at.suggested_max_retries(1);
        let conservative = at.suggested_max_retries_for_temperament(1, Temperament::Conservative);
        assert!(
            conservative <= 3,
            "conservative should cap at 3, got {conservative}"
        );
        assert!(
            conservative <= base,
            "conservative should be <= base {base}"
        );
    }

    #[test]
    fn aggressive_temperament_more_retries() {
        let mut at = AdaptiveThresholds::new();
        for _ in 0..10 {
            at.update(1, true); // High pass rate = fewer retries
        }
        let base = at.suggested_max_retries(1);
        let aggressive = at.suggested_max_retries_for_temperament(1, Temperament::Aggressive);
        assert!(
            aggressive >= 2,
            "aggressive should floor at 2, got {aggressive}"
        );
        assert!(aggressive >= base, "aggressive should be >= base {base}");
    }

    #[test]
    fn conservative_temperament_stricter_threshold() {
        let mut at = AdaptiveThresholds::new();
        for _ in 0..10 {
            at.update(1, true);
        }
        let base = at.threshold_for(1);
        let conservative = at.threshold_for_temperament(1, Temperament::Conservative);
        assert!(
            conservative >= base,
            "conservative threshold {conservative} should be >= base {base}"
        );
    }

    #[test]
    fn aggressive_temperament_relaxed_threshold() {
        let mut at = AdaptiveThresholds::new();
        for _ in 0..10 {
            at.update(1, true);
        }
        let base = at.threshold_for(1);
        let aggressive = at.threshold_for_temperament(1, Temperament::Aggressive);
        assert!(
            aggressive <= base,
            "aggressive threshold {aggressive} should be <= base {base}"
        );
    }

    #[test]
    fn conservative_never_skips_rung() {
        let mut at = AdaptiveThresholds::new();
        for _ in 0..100 {
            at.update(1, true);
        }
        assert!(
            at.should_skip_rung(1),
            "base should suggest skipping after 100 passes"
        );
        assert!(
            !at.should_skip_rung_for_temperament(1, Temperament::Conservative),
            "conservative should never skip"
        );
    }

    // ─── SPC wiring tests (GATE-01) ──────��─────────────────────────

    #[test]
    fn spc_detector_initialized_on_first_observe() {
        let mut at = AdaptiveThresholds::new();
        assert!(at.spc_detector(0).is_none());
        at.observe(0, true);
        assert!(at.spc_detector(0).is_some());
    }

    #[test]
    fn spc_alerts_accumulate_on_major_shift() {
        let mut at = AdaptiveThresholds::new();
        // Establish a stable baseline.
        for _ in 0..30 {
            at.observe(0, true);
        }
        assert!(!at.has_spc_alerts());

        // Inject a major shift — should trigger at least one SPC alert.
        for _ in 0..50 {
            at.observe(0, false);
        }

        // At least one of the three SPC detectors should have fired.
        let alerts = at.drain_spc_alerts();
        assert!(
            !alerts.is_empty(),
            "SPC should detect major shift after 50 failures"
        );
        assert!(alerts.iter().all(|(rung, _)| *rung == 0));
    }

    #[test]
    fn spc_alerts_drain_empties_pending() {
        let mut at = AdaptiveThresholds::new();
        for _ in 0..30 {
            at.observe(0, true);
        }
        for _ in 0..30 {
            at.observe(0, false);
        }
        let _ = at.drain_spc_alerts();
        assert!(!at.has_spc_alerts());
    }

    // ─── Hotelling / pipeline tests (GATE-08 wiring) ──────────────

    #[test]
    fn hotelling_initialized_on_pipeline_observe() {
        let mut at = AdaptiveThresholds::new();
        assert!(at.hotelling_detector().is_none());
        at.observe_pipeline(&[1.0, 1.0, 1.0]);
        assert!(at.hotelling_detector().is_some());
    }

    #[test]
    fn hotelling_detects_joint_anomaly() {
        let mut at = AdaptiveThresholds::new();
        // Establish a wider baseline with more variance to avoid
        // false anomalies on near-mean points.
        for i in 0..200 {
            let v = 0.8 + (i as f64 % 20.0) * 0.01;
            at.observe_pipeline(&[v, v]);
        }

        // A massive joint shift should be detected.
        at.observe_pipeline(&[0.0, 0.0]);
        assert!(at.joint_anomaly_detected());
    }

    // ─── Residual-based threshold update (TA-15) ──────────────────

    #[test]
    fn residual_tightens_threshold() {
        let mut at = AdaptiveThresholds::new();
        // Establish some baseline.
        for _ in 0..20 {
            at.observe(0, true);
        }
        let before = at.threshold_for(0);

        // Large residual should tighten (reduce) the threshold.
        at.observe_residual(0, 0.5);
        let after = at.threshold_for(0);

        assert!(
            after < before,
            "residual should tighten threshold: {after} < {before}"
        );
    }

    #[test]
    fn zero_residual_does_not_change_threshold() {
        let mut at = AdaptiveThresholds::new();
        for _ in 0..20 {
            at.observe(0, true);
        }
        let before = at.threshold_for(0);
        at.observe_residual(0, 0.0);
        let after = at.threshold_for(0);
        assert!((after - before).abs() < 1e-10);
    }
}
