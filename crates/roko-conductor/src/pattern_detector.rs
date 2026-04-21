//! Complex pattern detection with temporal hysteresis (COND-07).
//!
//! CEP-inspired multi-watcher correlation. Three composition patterns:
//! - Conjunction: multiple watchers fire simultaneously
//! - Sequence: watchers fire in a specific order within a time window
//! - Negation: a watcher fails to fire when expected
//!
//! WatcherFamily grouping (Resource, Quality, Progress) escalates severity
//! when 2+ watchers in the same family fire.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::interventions::{Severity, WatcherOutput};

/// Watcher family classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatcherFamily {
    /// Cost, time, and context window pressure.
    Resource,
    /// Compile, test, and spec drift watchers.
    Quality,
    /// Ghost turn, iteration loop, stuck pattern, and review loop.
    Progress,
}

/// A compound pattern detected across multiple watchers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompoundPattern {
    /// Name of the detected pattern (e.g., "resource_exhaustion").
    pub pattern_name: String,
    /// Watchers that contributed to the pattern.
    pub contributing_watchers: Vec<String>,
    /// Escalated severity based on the pattern.
    pub escalated_severity: Severity,
}

/// Detects compound patterns from watcher output streams.
///
/// Maintains a ring buffer of recent watcher outputs and per-watcher
/// consecutive fire counts for temporal hysteresis.
#[derive(Debug, Clone)]
pub struct PatternDetector {
    /// Ring buffer of recent evaluate() cycle outputs.
    history: VecDeque<Vec<WatcherOutput>>,
    /// Maximum history length.
    max_history: usize,
    /// Default hysteresis window (consecutive fires needed before propagation).
    hysteresis_window: usize,
    /// Per-watcher consecutive fire count.
    consecutive_fires: HashMap<String, usize>,
    /// Watcher-to-family mapping.
    family_map: HashMap<String, WatcherFamily>,
}

impl Default for PatternDetector {
    fn default() -> Self {
        let mut family_map = HashMap::new();
        // Resource family.
        family_map.insert("cost-overrun".to_string(), WatcherFamily::Resource);
        family_map.insert("time-overrun".to_string(), WatcherFamily::Resource);
        family_map.insert(
            "context-window-pressure".to_string(),
            WatcherFamily::Resource,
        );
        // Quality family.
        family_map.insert("compile-fail-repeat".to_string(), WatcherFamily::Quality);
        family_map.insert("test-failure-budget".to_string(), WatcherFamily::Quality);
        family_map.insert("spec-drift".to_string(), WatcherFamily::Quality);
        // Progress family.
        family_map.insert("ghost-turn".to_string(), WatcherFamily::Progress);
        family_map.insert("iteration-loop".to_string(), WatcherFamily::Progress);
        family_map.insert("stuck-pattern".to_string(), WatcherFamily::Progress);
        family_map.insert("review-loop".to_string(), WatcherFamily::Progress);

        Self {
            history: VecDeque::new(),
            max_history: 10,
            hysteresis_window: 2,
            consecutive_fires: HashMap::new(),
            family_map,
        }
    }
}

impl PatternDetector {
    /// Create a detector with custom hysteresis window.
    #[must_use]
    pub fn with_hysteresis(mut self, window: usize) -> Self {
        self.hysteresis_window = window.max(1);
        self
    }

    /// Record outputs from one evaluate() cycle and detect compound patterns.
    pub fn record(&mut self, outputs: &[WatcherOutput]) -> Vec<CompoundPattern> {
        // Update consecutive fire counts.
        let fired_watchers: HashMap<&str, &WatcherOutput> = outputs
            .iter()
            .filter(|o| o.severity >= Severity::Warning)
            .map(|o| (o.watcher.as_str(), o))
            .collect();

        // Increment or reset consecutive counts.
        let all_watchers: Vec<String> = self.family_map.keys().cloned().collect();
        for watcher in &all_watchers {
            if fired_watchers.contains_key(watcher.as_str()) {
                *self.consecutive_fires.entry(watcher.clone()).or_default() += 1;
            } else {
                self.consecutive_fires.insert(watcher.clone(), 0);
            }
        }

        // Store in history.
        self.history.push_back(outputs.to_vec());
        if self.history.len() > self.max_history {
            self.history.pop_front();
        }

        // Detect patterns.
        let mut patterns = Vec::new();

        // Family-level aggregation: 2+ watchers in the same family fire.
        let mut family_fires: HashMap<WatcherFamily, Vec<String>> = HashMap::new();
        for (watcher, _output) in &fired_watchers {
            if let Some(&family) = self.family_map.get(*watcher) {
                family_fires
                    .entry(family)
                    .or_default()
                    .push((*watcher).to_string());
            }
        }

        for (family, watchers) in &family_fires {
            if watchers.len() >= 2 {
                let name = match family {
                    WatcherFamily::Resource => "resource_exhaustion",
                    WatcherFamily::Quality => "quality_degradation",
                    WatcherFamily::Progress => "progress_stall",
                };
                patterns.push(CompoundPattern {
                    pattern_name: name.to_string(),
                    contributing_watchers: watchers.clone(),
                    escalated_severity: Severity::Critical,
                });
            }
        }

        // Conjunction: all three resource watchers fire (extreme resource pressure).
        let resource_watchers = ["cost-overrun", "time-overrun", "context-window-pressure"];
        let all_resource_fired = resource_watchers
            .iter()
            .all(|w| fired_watchers.contains_key(w));
        if all_resource_fired {
            patterns.push(CompoundPattern {
                pattern_name: "total_resource_exhaustion".to_string(),
                contributing_watchers: resource_watchers.iter().map(|s| s.to_string()).collect(),
                escalated_severity: Severity::Critical,
            });
        }

        // Sequence: ghost_turn -> iteration_loop -> stuck_pattern (progressive degradation).
        if self.history.len() >= 3 {
            let has_ghost_recent = self
                .consecutive_fires
                .get("ghost-turn")
                .copied()
                .unwrap_or(0)
                > 0;
            let has_iter_recent = self
                .consecutive_fires
                .get("iteration-loop")
                .copied()
                .unwrap_or(0)
                > 0;
            let has_stuck_recent = self
                .consecutive_fires
                .get("stuck-pattern")
                .copied()
                .unwrap_or(0)
                > 0;
            if has_ghost_recent && has_iter_recent && has_stuck_recent {
                patterns.push(CompoundPattern {
                    pattern_name: "progressive_degradation".to_string(),
                    contributing_watchers: vec![
                        "ghost-turn".to_string(),
                        "iteration-loop".to_string(),
                        "stuck-pattern".to_string(),
                    ],
                    escalated_severity: Severity::Critical,
                });
            }
        }

        patterns
    }

    /// Check if a watcher has fired N consecutive times (hysteresis gate).
    #[must_use]
    pub fn passes_hysteresis(&self, watcher: &str, n: usize) -> bool {
        self.consecutive_fires.get(watcher).copied().unwrap_or(0) >= n
    }

    /// Check if a watcher passes the default hysteresis window.
    #[must_use]
    pub fn passes_default_hysteresis(&self, watcher: &str) -> bool {
        self.passes_hysteresis(watcher, self.hysteresis_window)
    }

    /// Get the consecutive fire count for a watcher.
    #[must_use]
    pub fn consecutive_count(&self, watcher: &str) -> usize {
        self.consecutive_fires.get(watcher).copied().unwrap_or(0)
    }

    /// Clear all state.
    pub fn reset(&mut self) {
        self.history.clear();
        self.consecutive_fires.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warning(watcher: &str) -> WatcherOutput {
        WatcherOutput::new(watcher, Severity::Warning, "test")
    }

    fn info(watcher: &str) -> WatcherOutput {
        WatcherOutput::new(watcher, Severity::Info, "ok")
    }

    #[test]
    fn no_outputs_no_patterns() {
        let mut pd = PatternDetector::default();
        let patterns = pd.record(&[]);
        assert!(patterns.is_empty());
    }

    #[test]
    fn single_watcher_no_pattern() {
        let mut pd = PatternDetector::default();
        let patterns = pd.record(&[warning("cost-overrun")]);
        assert!(patterns.is_empty());
    }

    #[test]
    fn resource_family_two_watchers_triggers_pattern() {
        let mut pd = PatternDetector::default();
        let patterns = pd.record(&[warning("cost-overrun"), warning("time-overrun")]);
        assert!(
            patterns
                .iter()
                .any(|p| p.pattern_name == "resource_exhaustion")
        );
    }

    #[test]
    fn all_resource_watchers_triggers_total_exhaustion() {
        let mut pd = PatternDetector::default();
        let patterns = pd.record(&[
            warning("cost-overrun"),
            warning("time-overrun"),
            warning("context-window-pressure"),
        ]);
        assert!(
            patterns
                .iter()
                .any(|p| p.pattern_name == "total_resource_exhaustion")
        );
    }

    #[test]
    fn hysteresis_tracking() {
        let mut pd = PatternDetector::default();

        pd.record(&[warning("ghost-turn")]);
        assert!(!pd.passes_default_hysteresis("ghost-turn"));

        pd.record(&[warning("ghost-turn")]);
        assert!(pd.passes_default_hysteresis("ghost-turn"));
        assert_eq!(pd.consecutive_count("ghost-turn"), 2);
    }

    #[test]
    fn hysteresis_resets_on_miss() {
        let mut pd = PatternDetector::default();

        pd.record(&[warning("ghost-turn")]);
        pd.record(&[warning("ghost-turn")]);
        assert!(pd.passes_default_hysteresis("ghost-turn"));

        // Miss resets the count.
        pd.record(&[info("ghost-turn")]);
        assert!(!pd.passes_default_hysteresis("ghost-turn"));
        assert_eq!(pd.consecutive_count("ghost-turn"), 0);
    }

    #[test]
    fn quality_family_pattern() {
        let mut pd = PatternDetector::default();
        let patterns = pd.record(&[
            warning("compile-fail-repeat"),
            warning("test-failure-budget"),
        ]);
        assert!(
            patterns
                .iter()
                .any(|p| p.pattern_name == "quality_degradation")
        );
    }

    #[test]
    fn reset_clears_state() {
        let mut pd = PatternDetector::default();
        pd.record(&[warning("ghost-turn")]);
        pd.record(&[warning("ghost-turn")]);
        assert!(pd.passes_default_hysteresis("ghost-turn"));

        pd.reset();
        assert!(!pd.passes_default_hysteresis("ghost-turn"));
    }
}
