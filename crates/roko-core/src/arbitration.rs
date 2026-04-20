//! Cross-cut arbitration protocol (INT-21).
//!
//! When the three cross-cut subsystems (Daimon, Neuro, Dreams) produce
//! conflicting guidance, the arbitrator resolves the conflict using a
//! weighted priority scheme.
//!
//! Each subsystem emits [`SubsystemGuidance`] values, which the
//! [`Arbitrator`] collects, detects conflicts, and resolves into a single
//! [`ArbitrationOutcome`].
//!
//! # Priority model
//!
//! Default priority weights:
//! - **Neuro** (knowledge store): 0.50 — factual knowledge is highest priority
//! - **Daimon** (affect/motivation): 0.30 — emotional state modulates behavior
//! - **Dreams** (consolidation): 0.20 — offline insights are advisory
//!
//! These can be overridden via [`ArbitrationConfig`].

use serde::{Deserialize, Serialize};

/// Which cross-cut subsystem produced the guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subsystem {
    /// Daimon: affect engine and somatic markers.
    Daimon,
    /// Neuro: durable knowledge store and distillation.
    Neuro,
    /// Dreams: offline consolidation and hypothesis generation.
    Dreams,
}

impl Subsystem {
    /// Human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Daimon => "daimon",
            Self::Neuro => "neuro",
            Self::Dreams => "dreams",
        }
    }
}

/// A single piece of guidance from a cross-cut subsystem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubsystemGuidance {
    /// Which subsystem produced this guidance.
    pub source: Subsystem,
    /// Domain or topic this guidance applies to (e.g., "model_selection", "retry_strategy").
    pub domain: String,
    /// The recommended action or value.
    pub recommendation: String,
    /// Confidence in this recommendation, in `[0.0, 1.0]`.
    pub confidence: f64,
    /// Optional numeric signal (e.g., urgency score, risk level).
    pub signal_value: Option<f64>,
}

impl SubsystemGuidance {
    /// Construct a new guidance entry.
    #[must_use]
    pub fn new(
        source: Subsystem,
        domain: impl Into<String>,
        recommendation: impl Into<String>,
        confidence: f64,
    ) -> Self {
        Self {
            source,
            domain: domain.into(),
            recommendation: recommendation.into(),
            confidence: confidence.clamp(0.0, 1.0),
            signal_value: None,
        }
    }

    /// Attach a numeric signal value.
    #[must_use]
    pub fn with_signal(mut self, value: f64) -> Self {
        self.signal_value = Some(value);
        self
    }

    /// Effective weight = base priority * confidence.
    #[must_use]
    pub fn effective_weight(&self, config: &ArbitrationConfig) -> f64 {
        let base = config.priority_weight(self.source);
        base * self.confidence
    }
}

/// Configuration for the arbitration protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArbitrationConfig {
    /// Priority weight for the Neuro subsystem (default 0.50).
    #[serde(default = "ArbitrationConfig::default_neuro_weight")]
    pub neuro_weight: f64,
    /// Priority weight for the Daimon subsystem (default 0.30).
    #[serde(default = "ArbitrationConfig::default_daimon_weight")]
    pub daimon_weight: f64,
    /// Priority weight for the Dreams subsystem (default 0.20).
    #[serde(default = "ArbitrationConfig::default_dreams_weight")]
    pub dreams_weight: f64,
    /// Minimum confidence threshold to participate in arbitration (default 0.1).
    #[serde(default = "ArbitrationConfig::default_min_confidence")]
    pub min_confidence: f64,
}

impl Default for ArbitrationConfig {
    fn default() -> Self {
        Self {
            neuro_weight: Self::default_neuro_weight(),
            daimon_weight: Self::default_daimon_weight(),
            dreams_weight: Self::default_dreams_weight(),
            min_confidence: Self::default_min_confidence(),
        }
    }
}

impl ArbitrationConfig {
    fn default_neuro_weight() -> f64 {
        0.50
    }
    fn default_daimon_weight() -> f64 {
        0.30
    }
    fn default_dreams_weight() -> f64 {
        0.20
    }
    fn default_min_confidence() -> f64 {
        0.1
    }

    /// Get the priority weight for a subsystem.
    #[must_use]
    pub fn priority_weight(&self, subsystem: Subsystem) -> f64 {
        match subsystem {
            Subsystem::Daimon => self.daimon_weight,
            Subsystem::Neuro => self.neuro_weight,
            Subsystem::Dreams => self.dreams_weight,
        }
    }
}

/// Result of arbitrating conflicting guidance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArbitrationOutcome {
    /// The domain that was arbitrated.
    pub domain: String,
    /// The winning recommendation.
    pub winner: SubsystemGuidance,
    /// Whether there was an actual conflict (multiple subsystems disagreed).
    pub had_conflict: bool,
    /// All guidance entries considered, sorted by effective weight descending.
    pub all_guidance: Vec<SubsystemGuidance>,
    /// Total effective weight of the winning guidance.
    pub winning_weight: f64,
}

/// The cross-cut arbitrator.
///
/// Collects guidance from Daimon, Neuro, and Dreams, detects conflicts
/// (multiple subsystems providing guidance on the same domain with different
/// recommendations), and resolves them using weighted priority.
#[derive(Debug, Clone)]
pub struct Arbitrator {
    config: ArbitrationConfig,
    pending: Vec<SubsystemGuidance>,
}

impl Default for Arbitrator {
    fn default() -> Self {
        Self::new(ArbitrationConfig::default())
    }
}

impl Arbitrator {
    /// Construct an arbitrator with the given config.
    #[must_use]
    pub fn new(config: ArbitrationConfig) -> Self {
        Self {
            config,
            pending: Vec::new(),
        }
    }

    /// Submit a guidance entry for arbitration.
    pub fn submit(&mut self, guidance: SubsystemGuidance) {
        if guidance.confidence >= self.config.min_confidence {
            self.pending.push(guidance);
        }
    }

    /// Submit multiple guidance entries at once.
    pub fn submit_all(&mut self, entries: impl IntoIterator<Item = SubsystemGuidance>) {
        for entry in entries {
            self.submit(entry);
        }
    }

    /// Resolve all pending guidance into arbitration outcomes.
    ///
    /// Groups guidance by domain, detects conflicts, and picks the winner
    /// for each domain based on effective weight (priority * confidence).
    /// Drains the pending buffer.
    pub fn resolve(&mut self) -> Vec<ArbitrationOutcome> {
        let entries = std::mem::take(&mut self.pending);

        // Group by domain.
        let mut by_domain: std::collections::HashMap<String, Vec<SubsystemGuidance>> =
            std::collections::HashMap::new();
        for entry in entries {
            by_domain
                .entry(entry.domain.clone())
                .or_default()
                .push(entry);
        }

        let mut outcomes = Vec::new();
        for (domain, mut entries) in by_domain {
            // Sort by effective weight descending.
            entries.sort_by(|a, b| {
                let wa = a.effective_weight(&self.config);
                let wb = b.effective_weight(&self.config);
                wb.partial_cmp(&wa).unwrap_or(std::cmp::Ordering::Equal)
            });

            // Detect conflict: 2+ entries from different subsystems with
            // different recommendations.
            let has_conflict = {
                let unique_recs: std::collections::HashSet<&str> =
                    entries.iter().map(|e| e.recommendation.as_str()).collect();
                let unique_sources: std::collections::HashSet<Subsystem> =
                    entries.iter().map(|e| e.source).collect();
                unique_recs.len() > 1 && unique_sources.len() > 1
            };

            let winner = entries[0].clone();
            let winning_weight = winner.effective_weight(&self.config);

            outcomes.push(ArbitrationOutcome {
                domain,
                winner,
                had_conflict: has_conflict,
                all_guidance: entries,
                winning_weight,
            });
        }

        outcomes
    }

    /// Clear all pending guidance without resolving.
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// Number of pending guidance entries.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_guidance_no_conflict() {
        let mut arb = Arbitrator::default();
        arb.submit(SubsystemGuidance::new(
            Subsystem::Neuro,
            "model_selection",
            "use_opus",
            0.9,
        ));
        let outcomes = arb.resolve();
        assert_eq!(outcomes.len(), 1);
        assert!(!outcomes[0].had_conflict);
        assert_eq!(outcomes[0].winner.source, Subsystem::Neuro);
    }

    #[test]
    fn two_subsystems_same_recommendation_no_conflict() {
        let mut arb = Arbitrator::default();
        arb.submit(SubsystemGuidance::new(
            Subsystem::Neuro,
            "retry_strategy",
            "conservative",
            0.8,
        ));
        arb.submit(SubsystemGuidance::new(
            Subsystem::Daimon,
            "retry_strategy",
            "conservative",
            0.7,
        ));
        let outcomes = arb.resolve();
        assert_eq!(outcomes.len(), 1);
        assert!(!outcomes[0].had_conflict);
    }

    #[test]
    fn conflict_resolved_by_weight() {
        let mut arb = Arbitrator::default();
        // Neuro says "conservative" with high confidence.
        arb.submit(SubsystemGuidance::new(
            Subsystem::Neuro,
            "retry_strategy",
            "conservative",
            0.9,
        ));
        // Daimon says "aggressive" with high confidence.
        arb.submit(SubsystemGuidance::new(
            Subsystem::Daimon,
            "retry_strategy",
            "aggressive",
            0.9,
        ));
        // Dreams says "exploratory" with lower confidence.
        arb.submit(SubsystemGuidance::new(
            Subsystem::Dreams,
            "retry_strategy",
            "exploratory",
            0.5,
        ));

        let outcomes = arb.resolve();
        assert_eq!(outcomes.len(), 1);
        let outcome = &outcomes[0];
        assert!(outcome.had_conflict);
        // Neuro wins: 0.50 * 0.9 = 0.45 > Daimon 0.30 * 0.9 = 0.27.
        assert_eq!(outcome.winner.source, Subsystem::Neuro);
        assert_eq!(outcome.winner.recommendation, "conservative");
    }

    #[test]
    fn daimon_wins_when_confidence_much_higher() {
        let mut arb = Arbitrator::default();
        // Neuro has low confidence.
        arb.submit(SubsystemGuidance::new(
            Subsystem::Neuro,
            "model_selection",
            "use_haiku",
            0.2,
        ));
        // Daimon has very high confidence.
        arb.submit(SubsystemGuidance::new(
            Subsystem::Daimon,
            "model_selection",
            "use_opus",
            1.0,
        ));

        let outcomes = arb.resolve();
        let outcome = &outcomes[0];
        assert!(outcome.had_conflict);
        // Daimon: 0.30 * 1.0 = 0.30 > Neuro: 0.50 * 0.2 = 0.10.
        assert_eq!(outcome.winner.source, Subsystem::Daimon);
    }

    #[test]
    fn low_confidence_filtered_out() {
        let mut arb = Arbitrator::default();
        arb.submit(SubsystemGuidance::new(
            Subsystem::Dreams,
            "retry_strategy",
            "exploratory",
            0.05, // Below min_confidence (0.1).
        ));
        assert_eq!(arb.pending_count(), 0);
    }

    #[test]
    fn multiple_domains_resolved_independently() {
        let mut arb = Arbitrator::default();
        arb.submit(SubsystemGuidance::new(
            Subsystem::Neuro,
            "model_selection",
            "use_opus",
            0.8,
        ));
        arb.submit(SubsystemGuidance::new(
            Subsystem::Daimon,
            "retry_strategy",
            "aggressive",
            0.9,
        ));

        let outcomes = arb.resolve();
        assert_eq!(outcomes.len(), 2);
        // Both should have no conflict since they are different domains.
        for outcome in &outcomes {
            assert!(!outcome.had_conflict);
        }
    }

    #[test]
    fn signal_value_preserved() {
        let mut arb = Arbitrator::default();
        arb.submit(
            SubsystemGuidance::new(Subsystem::Daimon, "urgency", "high", 0.8)
                .with_signal(0.95),
        );
        let outcomes = arb.resolve();
        assert_eq!(outcomes[0].winner.signal_value, Some(0.95));
    }

    #[test]
    fn custom_config_changes_winner() {
        // Give dreams the highest weight.
        let config = ArbitrationConfig {
            neuro_weight: 0.1,
            daimon_weight: 0.1,
            dreams_weight: 0.8,
            min_confidence: 0.1,
        };
        let mut arb = Arbitrator::new(config);
        arb.submit(SubsystemGuidance::new(
            Subsystem::Neuro,
            "strategy",
            "conservative",
            0.9,
        ));
        arb.submit(SubsystemGuidance::new(
            Subsystem::Dreams,
            "strategy",
            "exploratory",
            0.9,
        ));

        let outcomes = arb.resolve();
        assert_eq!(outcomes[0].winner.source, Subsystem::Dreams);
    }
}
