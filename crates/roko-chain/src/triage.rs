//! Triage pipeline for scoring and routing observed chain events (CHAIN-07).
//!
//! A 4-stage rule-based pipeline (no LLM) that processes observed events:
//! 1. Rule-based filter -- match against known patterns
//! 2. MIDAS-R anomaly detection -- streaming anomaly scorer
//! 3. Contextual enrichment -- attach metadata
//! 4. HDC/Bayesian curiosity scoring -- information gain scoring

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::observer::ObservedEvent;

/// Configuration for the triage pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriageConfig {
    /// Minimum anomaly score to flag an event (0.0 - 1.0).
    pub anomaly_threshold: f64,
    /// Minimum curiosity score to route an event to knowledge ingestion.
    pub curiosity_threshold: f64,
    /// Known contract addresses mapped to their labels.
    pub known_contracts: HashMap<String, String>,
    /// Topic hashes that indicate known event types.
    pub known_topics: HashMap<String, String>,
}

impl Default for TriageConfig {
    fn default() -> Self {
        Self {
            anomaly_threshold: 0.7,
            curiosity_threshold: 0.5,
            known_contracts: HashMap::new(),
            known_topics: HashMap::new(),
        }
    }
}

/// Result of the triage pipeline for one event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriageResult {
    /// The original observed event.
    pub event: ObservedEvent,
    /// Whether the event matched known rules.
    pub rule_matched: bool,
    /// Rule label if matched.
    pub rule_label: Option<String>,
    /// MIDAS-R anomaly score (0.0 - 1.0).
    pub anomaly_score: f64,
    /// Whether the event was flagged as anomalous.
    pub is_anomalous: bool,
    /// Enrichment metadata attached during stage 3.
    pub enrichment: EventEnrichment,
    /// Curiosity score from stage 4 (0.0 - 1.0).
    pub curiosity_score: f64,
    /// Recommended routing action.
    pub action: TriageAction,
}

/// Enrichment metadata attached to a triaged event.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EventEnrichment {
    /// Contract label (from known_contracts mapping).
    pub contract_label: Option<String>,
    /// Event type label (from known_topics mapping).
    pub event_type_label: Option<String>,
    /// Domain context tags.
    pub domain_tags: Vec<String>,
}

/// Routing action determined by the triage pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriageAction {
    /// Route to knowledge ingestion (high curiosity).
    IngestKnowledge,
    /// Route to conductor alert (anomaly detected).
    AlertConductor,
    /// Route to marketplace handler (job-related event).
    MarketplaceHandler,
    /// Drop the event (below all thresholds).
    Drop,
}

/// Streaming anomaly scorer inspired by MIDAS-R (Bhatia et al. 2020).
///
/// Uses a simplified count-based model: tracks event frequency per
/// address and flags sudden spikes as anomalous.
#[derive(Debug, Clone, Default)]
pub struct MidasRScorer {
    /// Event counts per address in the current window.
    address_counts: HashMap<String, u64>,
    /// Historical mean event rate per address.
    address_means: HashMap<String, f64>,
    /// Number of observation windows completed.
    window_count: u64,
    /// Smoothing factor for updating means.
    alpha: f64,
}

impl MidasRScorer {
    /// Create a new MIDAS-R scorer with the given smoothing factor.
    #[must_use]
    pub fn new(alpha: f64) -> Self {
        Self {
            alpha: alpha.clamp(0.01, 1.0),
            ..Default::default()
        }
    }

    /// Record an event from the given address.
    pub fn observe(&mut self, address: &str) {
        *self
            .address_counts
            .entry(address.to_lowercase())
            .or_default() += 1;
    }

    /// Score an address based on deviation from its historical mean.
    ///
    /// Returns a value in [0.0, 1.0] where higher means more anomalous.
    #[must_use]
    pub fn score(&self, address: &str) -> f64 {
        let key = address.to_lowercase();
        let count = self.address_counts.get(&key).copied().unwrap_or(0) as f64;
        let mean = self.address_means.get(&key).copied().unwrap_or(1.0).max(0.1);
        let z = (count - mean) / mean;
        // Sigmoid-like compression to [0, 1]
        1.0 - 1.0 / (1.0 + z.max(0.0))
    }

    /// Advance the observation window: update historical means and reset counts.
    pub fn advance_window(&mut self) {
        self.window_count += 1;
        for (addr, &count) in &self.address_counts {
            let current_mean = self.address_means.get(addr).copied().unwrap_or(count as f64);
            let new_mean = self.alpha * count as f64 + (1.0 - self.alpha) * current_mean;
            self.address_means.insert(addr.clone(), new_mean);
        }
        self.address_counts.clear();
    }

    /// Number of observation windows completed.
    #[must_use]
    pub fn window_count(&self) -> u64 {
        self.window_count
    }
}

/// The 4-stage triage pipeline.
#[derive(Debug, Clone)]
pub struct TriagePipeline {
    /// Pipeline configuration.
    pub config: TriageConfig,
    /// Streaming anomaly scorer.
    pub anomaly_scorer: MidasRScorer,
}

impl TriagePipeline {
    /// Create a new triage pipeline from configuration.
    #[must_use]
    pub fn new(config: TriageConfig) -> Self {
        Self {
            config,
            anomaly_scorer: MidasRScorer::new(0.1),
        }
    }

    /// Run the full 4-stage pipeline on an observed event.
    pub fn triage(&mut self, event: ObservedEvent) -> TriageResult {
        // Stage 1: Rule-based filter
        let (rule_matched, rule_label) = self.stage_rule_filter(&event);

        // Stage 2: MIDAS-R anomaly detection
        self.anomaly_scorer.observe(&event.log.address);
        let anomaly_score = self.anomaly_scorer.score(&event.log.address);
        let is_anomalous = anomaly_score >= self.config.anomaly_threshold;

        // Stage 3: Contextual enrichment
        let enrichment = self.stage_enrich(&event);

        // Stage 4: Curiosity scoring
        let curiosity_score = self.stage_curiosity(&event, rule_matched, anomaly_score);

        // Routing decision
        let action = self.route(rule_matched, is_anomalous, curiosity_score, &enrichment);

        TriageResult {
            event,
            rule_matched,
            rule_label,
            anomaly_score,
            is_anomalous,
            enrichment,
            curiosity_score,
            action,
        }
    }

    /// Batch triage: process multiple events and advance the anomaly window.
    pub fn triage_batch(&mut self, events: Vec<ObservedEvent>) -> Vec<TriageResult> {
        let results: Vec<_> = events.into_iter().map(|e| self.triage(e)).collect();
        self.anomaly_scorer.advance_window();
        results
    }

    /// Stage 1: Match events against known contract addresses and topic hashes.
    fn stage_rule_filter(&self, event: &ObservedEvent) -> (bool, Option<String>) {
        let addr_key = event.log.address.to_lowercase();
        if let Some(label) = self.config.known_contracts.get(&addr_key) {
            return (true, Some(label.clone()));
        }
        for topic in &event.log.topics {
            let topic_key = topic.to_lowercase();
            if let Some(label) = self.config.known_topics.get(&topic_key) {
                return (true, Some(label.clone()));
            }
        }
        (false, None)
    }

    /// Stage 3: Attach contextual metadata.
    fn stage_enrich(&self, event: &ObservedEvent) -> EventEnrichment {
        let contract_label = self
            .config
            .known_contracts
            .get(&event.log.address.to_lowercase())
            .cloned();
        let event_type_label = event
            .log
            .topics
            .iter()
            .find_map(|t| self.config.known_topics.get(&t.to_lowercase()).cloned());
        let mut domain_tags = Vec::new();
        if contract_label.is_some() {
            domain_tags.push("known_contract".to_string());
        }
        if event_type_label.is_some() {
            domain_tags.push("known_event".to_string());
        }

        EventEnrichment {
            contract_label,
            event_type_label,
            domain_tags,
        }
    }

    /// Stage 4: Compute curiosity score based on novelty and relevance.
    fn stage_curiosity(
        &self,
        event: &ObservedEvent,
        rule_matched: bool,
        anomaly_score: f64,
    ) -> f64 {
        let mut score = 0.0;

        // Rule-matched events have baseline curiosity
        if rule_matched {
            score += 0.3;
        }

        // Anomaly contributes to curiosity
        score += anomaly_score * 0.4;

        // Data richness: more log data = more interesting
        let data_richness = (event.log.data.len() as f64 / 256.0).min(1.0);
        score += data_richness * 0.2;

        // Topic count: more topics = more structured event
        let topic_bonus = (event.log.topics.len() as f64 / 4.0).min(1.0);
        score += topic_bonus * 0.1;

        score.clamp(0.0, 1.0)
    }

    /// Determine the routing action based on triage scores.
    fn route(
        &self,
        rule_matched: bool,
        is_anomalous: bool,
        curiosity_score: f64,
        enrichment: &EventEnrichment,
    ) -> TriageAction {
        if is_anomalous {
            return TriageAction::AlertConductor;
        }
        if enrichment
            .domain_tags
            .iter()
            .any(|t| t.contains("marketplace") || t.contains("job"))
        {
            return TriageAction::MarketplaceHandler;
        }
        if curiosity_score >= self.config.curiosity_threshold {
            return TriageAction::IngestKnowledge;
        }
        if rule_matched {
            return TriageAction::IngestKnowledge;
        }
        TriageAction::Drop
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::types::LogEntry;

    fn test_event(address: &str, topic: &str) -> ObservedEvent {
        ObservedEvent {
            block_number: 100,
            block_hash: "0xblock100".to_string(),
            block_timestamp: 1_700_000_100,
            log: LogEntry {
                address: address.to_string(),
                topics: vec![topic.to_string()],
                data: vec![1, 2, 3, 4],
            },
        }
    }

    #[test]
    fn triage_drops_unknown_event() {
        let config = TriageConfig::default();
        let mut pipeline = TriagePipeline::new(config);

        let result = pipeline.triage(test_event("0xunknown", "0xunknown"));

        assert!(!result.rule_matched);
        assert!(!result.is_anomalous);
        assert_eq!(result.action, TriageAction::Drop);
    }

    #[test]
    fn triage_matches_known_contract() {
        let mut config = TriageConfig::default();
        config
            .known_contracts
            .insert("0xcafe".to_string(), "KoraiToken".to_string());
        let mut pipeline = TriagePipeline::new(config);

        let result = pipeline.triage(test_event("0xcafe", "0xtopic"));

        assert!(result.rule_matched);
        assert_eq!(result.rule_label, Some("KoraiToken".to_string()));
        assert_eq!(result.enrichment.contract_label, Some("KoraiToken".to_string()));
    }

    #[test]
    fn triage_matches_known_topic() {
        let mut config = TriageConfig::default();
        config
            .known_topics
            .insert("0xabcd".to_string(), "Transfer".to_string());
        let mut pipeline = TriagePipeline::new(config);

        let result = pipeline.triage(test_event("0xany", "0xabcd"));

        assert!(result.rule_matched);
        assert_eq!(result.rule_label, Some("Transfer".to_string()));
    }

    #[test]
    fn midas_r_scorer_detects_spike() {
        let mut scorer = MidasRScorer::new(0.1);

        // Establish baseline: 2 events per window
        for _ in 0..2 {
            scorer.observe("0xcafe");
        }
        scorer.advance_window();

        // Spike: 20 events in one window
        for _ in 0..20 {
            scorer.observe("0xcafe");
        }

        let score = scorer.score("0xcafe");
        assert!(
            score > 0.5,
            "expected spike to produce high anomaly score, got {score}"
        );
    }

    #[test]
    fn midas_r_scorer_returns_zero_for_normal() {
        let mut scorer = MidasRScorer::new(0.1);

        // Establish baseline
        for _ in 0..5 {
            scorer.observe("0xcafe");
        }
        scorer.advance_window();

        // Same rate
        for _ in 0..5 {
            scorer.observe("0xcafe");
        }

        let score = scorer.score("0xcafe");
        assert!(
            score < 0.5,
            "expected normal rate to produce low anomaly score, got {score}"
        );
    }

    #[test]
    fn triage_alerts_conductor_on_anomaly() {
        let config = TriageConfig {
            anomaly_threshold: 0.3,
            ..Default::default()
        };
        let mut pipeline = TriagePipeline::new(config);

        // Create a spike: many events from the same address
        for _ in 0..2 {
            pipeline.anomaly_scorer.observe("0xspiker");
        }
        pipeline.anomaly_scorer.advance_window();
        for _ in 0..50 {
            pipeline.anomaly_scorer.observe("0xspiker");
        }

        let result = pipeline.triage(test_event("0xspiker", "0xtopic"));

        assert!(result.is_anomalous);
        assert_eq!(result.action, TriageAction::AlertConductor);
    }

    #[test]
    fn triage_batch_advances_window() {
        let config = TriageConfig::default();
        let mut pipeline = TriagePipeline::new(config);

        let events = vec![
            test_event("0xa", "0xt1"),
            test_event("0xb", "0xt2"),
        ];

        let results = pipeline.triage_batch(events);
        assert_eq!(results.len(), 2);
        assert_eq!(pipeline.anomaly_scorer.window_count(), 1);
    }

    #[test]
    fn curiosity_score_increases_with_rule_match() {
        let mut config = TriageConfig::default();
        config
            .known_contracts
            .insert("0xcafe".to_string(), "Token".to_string());
        let mut pipeline = TriagePipeline::new(config);

        let matched = pipeline.triage(test_event("0xcafe", "0xt"));
        let unmatched = pipeline.triage(test_event("0xunknown", "0xt"));

        assert!(
            matched.curiosity_score > unmatched.curiosity_score,
            "rule-matched event should have higher curiosity"
        );
    }

    #[test]
    fn enrichment_tags_known_items() {
        let mut config = TriageConfig::default();
        config
            .known_contracts
            .insert("0xcafe".to_string(), "Token".to_string());
        config
            .known_topics
            .insert("0xabcd".to_string(), "Transfer".to_string());
        let mut pipeline = TriagePipeline::new(config);

        let result = pipeline.triage(test_event("0xcafe", "0xabcd"));

        assert!(result.enrichment.domain_tags.contains(&"known_contract".to_string()));
        assert!(result.enrichment.domain_tags.contains(&"known_event".to_string()));
    }

    #[test]
    fn triage_result_serialization_roundtrip() {
        let config = TriageConfig::default();
        let mut pipeline = TriagePipeline::new(config);

        let result = pipeline.triage(test_event("0xa", "0xb"));
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: TriageResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result.event, deserialized.event);
        assert_eq!(result.action, deserialized.action);
    }
}
