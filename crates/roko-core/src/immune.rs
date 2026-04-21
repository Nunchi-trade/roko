//! Cognitive immune system -- detecting and quarantining compromised knowledge.
//!
//! This module provides the core quarantine mechanism for the Roko immune system.
//! Engrams that fail validation (anomalous outputs, hallucinated data, tainted
//! sources) are quarantined instead of being immediately rejected or accepted.
//!
//! # Architecture
//!
//! ```text
//! Engram ──check()──► AnomalyDetector ──score()──► QuarantineDecision
//!                                                       │
//!                          ┌──── Accept (score < threshold)
//!                          ├──── Quarantine (score >= threshold)
//!                          └──── Reject (auto_reject && score >= threshold)
//!
//! QuarantineVault ───── stores quarantined engrams for review
//! IncidentLink ──────── connects related taint incidents
//! ImmuneResponse ────── recovery action after quarantine review
//! ```

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{ContentHash, Taint};

/// An anomaly score computed for an engram during immune screening.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnomalyScore {
    /// Overall anomaly score in [0, 1]. Higher = more anomalous.
    pub score: f64,
    /// Per-dimension scores that contributed to the overall score.
    pub dimensions: HashMap<String, f64>,
    /// Which taint classification was detected, if any.
    pub detected_taint: Option<Taint>,
}

impl AnomalyScore {
    /// Create a clean (non-anomalous) score.
    #[must_use]
    pub fn clean() -> Self {
        Self {
            score: 0.0,
            dimensions: HashMap::new(),
            detected_taint: None,
        }
    }

    /// Create an anomaly score with a single dimension.
    #[must_use]
    pub fn from_score(score: f64) -> Self {
        Self {
            score: score.clamp(0.0, 1.0),
            dimensions: HashMap::new(),
            detected_taint: None,
        }
    }

    /// Add a dimension score.
    pub fn with_dimension(mut self, name: impl Into<String>, score: f64) -> Self {
        self.dimensions.insert(name.into(), score.clamp(0.0, 1.0));
        self
    }

    /// Attach detected taint.
    pub fn with_taint(mut self, taint: Taint) -> Self {
        self.detected_taint = Some(taint);
        self
    }

    /// Whether the score exceeds the given threshold.
    #[must_use]
    pub fn exceeds_threshold(&self, threshold: f64) -> bool {
        self.score >= threshold
    }
}

/// Decision made by the immune system about an engram.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuarantineDecision {
    /// Engram is clean, allow it through.
    Accept,
    /// Engram is suspicious, quarantine for review.
    Quarantine,
    /// Engram is flagged and auto-reject is enabled.
    Reject,
}

/// A quarantined engram entry in the vault.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuarantineEntry {
    /// Content hash of the quarantined engram.
    pub hash: ContentHash,
    /// Anomaly score that triggered quarantine.
    pub anomaly_score: AnomalyScore,
    /// When the engram was quarantined.
    pub quarantined_at: DateTime<Utc>,
    /// Current review status.
    pub status: QuarantineStatus,
    /// Optional reviewer notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer_notes: Option<String>,
    /// Incident links to related quarantine events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub incident_links: Vec<IncidentLink>,
}

/// Status of a quarantined entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuarantineStatus {
    /// Awaiting review.
    Pending,
    /// Reviewed and approved (will be released).
    Approved,
    /// Reviewed and rejected (will be purged).
    Rejected,
    /// Escalated for higher-level review.
    Escalated,
}

/// A link between related taint incidents.
///
/// When multiple engrams are quarantined due to related causes (e.g., the same
/// tainted source), incident links connect them for batch review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncidentLink {
    /// The related quarantine entry hash.
    pub related_hash: ContentHash,
    /// Relationship description.
    pub relation: IncidentRelation,
    /// When the link was established.
    pub linked_at: DateTime<Utc>,
}

/// Kind of relationship between incident entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentRelation {
    /// Both engrams came from the same tainted source.
    SameSource,
    /// One engram propagated taint to the other.
    Propagated,
    /// Both engrams contradict each other.
    Contradiction,
    /// Both engrams were produced in the same agent session.
    SameSession,
}

/// Recovery action taken after quarantine review.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImmuneResponse {
    /// Which quarantine entry this response applies to.
    pub entry_hash: ContentHash,
    /// The action taken.
    pub action: ResponseAction,
    /// When the response was issued.
    pub responded_at: DateTime<Utc>,
    /// Optional description of the recovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Action taken in response to a quarantine review.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseAction {
    /// Release the engram back into the hot substrate.
    Release,
    /// Purge the engram permanently.
    Purge,
    /// Retag the engram with corrected taint classification.
    Retag,
    /// Move to cold storage for archival.
    Archive,
}

/// In-memory quarantine vault for holding suspect engrams.
pub struct QuarantineVault {
    /// Entries indexed by content hash.
    entries: HashMap<ContentHash, QuarantineEntry>,
    /// Anomaly threshold for quarantine decisions.
    threshold: f64,
    /// Maximum number of entries before escalation.
    max_entries: usize,
    /// Whether to auto-reject above-threshold engrams.
    auto_reject: bool,
}

impl QuarantineVault {
    /// Create a new vault with the given configuration.
    #[must_use]
    pub fn new(threshold: f64, max_entries: usize, auto_reject: bool) -> Self {
        Self {
            entries: HashMap::new(),
            threshold: threshold.clamp(0.0, 1.0),
            max_entries,
            auto_reject,
        }
    }

    /// Create a vault with default settings (threshold=0.8, max=50, no auto-reject).
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(0.8, 50, false)
    }

    /// Screen an engram and decide whether to quarantine it.
    #[must_use]
    pub fn screen(&self, anomaly: &AnomalyScore) -> QuarantineDecision {
        if anomaly.exceeds_threshold(self.threshold) {
            if self.auto_reject {
                QuarantineDecision::Reject
            } else {
                QuarantineDecision::Quarantine
            }
        } else {
            QuarantineDecision::Accept
        }
    }

    /// Add an engram to quarantine. Returns `true` if added, `false` if vault is full.
    pub fn quarantine(&mut self, hash: ContentHash, anomaly: AnomalyScore) -> bool {
        if self.entries.len() >= self.max_entries {
            return false;
        }

        let entry = QuarantineEntry {
            hash,
            anomaly_score: anomaly,
            quarantined_at: Utc::now(),
            status: QuarantineStatus::Pending,
            reviewer_notes: None,
            incident_links: Vec::new(),
        };
        self.entries.insert(hash, entry);
        true
    }

    /// Review and update the status of a quarantined entry.
    pub fn review(
        &mut self,
        hash: &ContentHash,
        status: QuarantineStatus,
        notes: Option<String>,
    ) -> bool {
        if let Some(entry) = self.entries.get_mut(hash) {
            entry.status = status;
            entry.reviewer_notes = notes;
            true
        } else {
            false
        }
    }

    /// Link two quarantine entries as related incidents.
    pub fn link_incidents(
        &mut self,
        a: ContentHash,
        b: ContentHash,
        relation: IncidentRelation,
    ) -> bool {
        if !self.entries.contains_key(&a) || !self.entries.contains_key(&b) {
            return false;
        }

        let now = Utc::now();

        if let Some(entry_a) = self.entries.get_mut(&a) {
            entry_a.incident_links.push(IncidentLink {
                related_hash: b,
                relation,
                linked_at: now,
            });
        }
        if let Some(entry_b) = self.entries.get_mut(&b) {
            entry_b.incident_links.push(IncidentLink {
                related_hash: a,
                relation,
                linked_at: now,
            });
        }

        true
    }

    /// Get a quarantine entry by hash.
    #[must_use]
    pub fn get(&self, hash: &ContentHash) -> Option<&QuarantineEntry> {
        self.entries.get(hash)
    }

    /// All pending entries.
    #[must_use]
    pub fn pending(&self) -> Vec<&QuarantineEntry> {
        self.entries
            .values()
            .filter(|e| e.status == QuarantineStatus::Pending)
            .collect()
    }

    /// Remove approved and rejected entries, returning the released hashes.
    pub fn drain_resolved(&mut self) -> (Vec<ContentHash>, Vec<ContentHash>) {
        let mut released = Vec::new();
        let mut purged = Vec::new();

        self.entries.retain(|hash, entry| match entry.status {
            QuarantineStatus::Approved => {
                released.push(*hash);
                false
            }
            QuarantineStatus::Rejected => {
                purged.push(*hash);
                false
            }
            _ => true,
        });

        (released, purged)
    }

    /// Current number of quarantined entries.
    #[must_use]
    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// Whether the vault has reached maximum capacity.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.entries.len() >= self.max_entries
    }

    /// Whether escalation is needed (vault is >= 80% full).
    #[must_use]
    pub fn needs_escalation(&self) -> bool {
        self.entries.len() as f64 >= self.max_entries as f64 * 0.8
    }
}

impl Default for QuarantineVault {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_hash(n: u8) -> ContentHash {
        let mut bytes = [0u8; 32];
        bytes[0] = n;
        ContentHash(bytes)
    }

    #[test]
    fn anomaly_score_clean() {
        let score = AnomalyScore::clean();
        assert_eq!(score.score, 0.0);
        assert!(!score.exceeds_threshold(0.5));
    }

    #[test]
    fn anomaly_score_from_value() {
        let score = AnomalyScore::from_score(0.9);
        assert!(score.exceeds_threshold(0.8));
        assert!(!score.exceeds_threshold(0.95));
    }

    #[test]
    fn anomaly_score_with_dimensions() {
        let score = AnomalyScore::from_score(0.7)
            .with_dimension("coherence", 0.3)
            .with_dimension("novelty", 0.9);
        assert_eq!(score.dimensions.len(), 2);
        assert_eq!(score.dimensions["coherence"], 0.3);
    }

    #[test]
    fn anomaly_score_clamping() {
        let score = AnomalyScore::from_score(2.0);
        assert_eq!(score.score, 1.0);

        let score = AnomalyScore::from_score(-1.0);
        assert_eq!(score.score, 0.0);
    }

    #[test]
    fn screen_accept_below_threshold() {
        let vault = QuarantineVault::new(0.8, 50, false);
        let score = AnomalyScore::from_score(0.5);
        assert_eq!(vault.screen(&score), QuarantineDecision::Accept);
    }

    #[test]
    fn screen_quarantine_above_threshold() {
        let vault = QuarantineVault::new(0.8, 50, false);
        let score = AnomalyScore::from_score(0.9);
        assert_eq!(vault.screen(&score), QuarantineDecision::Quarantine);
    }

    #[test]
    fn screen_reject_with_auto_reject() {
        let vault = QuarantineVault::new(0.8, 50, true);
        let score = AnomalyScore::from_score(0.9);
        assert_eq!(vault.screen(&score), QuarantineDecision::Reject);
    }

    #[test]
    fn quarantine_and_retrieve() {
        let mut vault = QuarantineVault::with_defaults();
        let hash = dummy_hash(1);
        let score = AnomalyScore::from_score(0.85);

        assert!(vault.quarantine(hash, score.clone()));
        assert_eq!(vault.count(), 1);

        let entry = vault.get(&hash).unwrap();
        assert_eq!(entry.status, QuarantineStatus::Pending);
        assert_eq!(entry.anomaly_score.score, 0.85);
    }

    #[test]
    fn quarantine_respects_capacity() {
        let mut vault = QuarantineVault::new(0.5, 2, false);

        assert!(vault.quarantine(dummy_hash(1), AnomalyScore::from_score(0.6)));
        assert!(vault.quarantine(dummy_hash(2), AnomalyScore::from_score(0.7)));
        assert!(!vault.quarantine(dummy_hash(3), AnomalyScore::from_score(0.8)));
        assert!(vault.is_full());
    }

    #[test]
    fn review_updates_status() {
        let mut vault = QuarantineVault::with_defaults();
        let hash = dummy_hash(1);
        vault.quarantine(hash, AnomalyScore::from_score(0.9));

        assert!(vault.review(&hash, QuarantineStatus::Approved, Some("looks fine".into())));
        let entry = vault.get(&hash).unwrap();
        assert_eq!(entry.status, QuarantineStatus::Approved);
        assert_eq!(entry.reviewer_notes.as_deref(), Some("looks fine"));
    }

    #[test]
    fn review_nonexistent_returns_false() {
        let mut vault = QuarantineVault::with_defaults();
        assert!(!vault.review(&dummy_hash(99), QuarantineStatus::Rejected, None));
    }

    #[test]
    fn drain_resolved_separates_approved_and_rejected() {
        let mut vault = QuarantineVault::with_defaults();
        let h1 = dummy_hash(1);
        let h2 = dummy_hash(2);
        let h3 = dummy_hash(3);

        vault.quarantine(h1, AnomalyScore::from_score(0.9));
        vault.quarantine(h2, AnomalyScore::from_score(0.85));
        vault.quarantine(h3, AnomalyScore::from_score(0.95));

        vault.review(&h1, QuarantineStatus::Approved, None);
        vault.review(&h2, QuarantineStatus::Rejected, None);
        // h3 stays pending

        let (released, purged) = vault.drain_resolved();
        assert_eq!(released.len(), 1);
        assert!(released.contains(&h1));
        assert_eq!(purged.len(), 1);
        assert!(purged.contains(&h2));
        assert_eq!(vault.count(), 1); // only h3 remains
    }

    #[test]
    fn pending_returns_only_pending() {
        let mut vault = QuarantineVault::with_defaults();
        let h1 = dummy_hash(1);
        let h2 = dummy_hash(2);

        vault.quarantine(h1, AnomalyScore::from_score(0.9));
        vault.quarantine(h2, AnomalyScore::from_score(0.85));
        vault.review(&h1, QuarantineStatus::Approved, None);

        let pending = vault.pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].hash, h2);
    }

    #[test]
    fn link_incidents_bidirectional() {
        let mut vault = QuarantineVault::with_defaults();
        let h1 = dummy_hash(1);
        let h2 = dummy_hash(2);

        vault.quarantine(h1, AnomalyScore::from_score(0.9));
        vault.quarantine(h2, AnomalyScore::from_score(0.85));

        assert!(vault.link_incidents(h1, h2, IncidentRelation::SameSource));

        let e1 = vault.get(&h1).unwrap();
        assert_eq!(e1.incident_links.len(), 1);
        assert_eq!(e1.incident_links[0].related_hash, h2);

        let e2 = vault.get(&h2).unwrap();
        assert_eq!(e2.incident_links.len(), 1);
        assert_eq!(e2.incident_links[0].related_hash, h1);
    }

    #[test]
    fn link_fails_for_missing_entries() {
        let mut vault = QuarantineVault::with_defaults();
        assert!(!vault.link_incidents(dummy_hash(1), dummy_hash(2), IncidentRelation::SameSource));
    }

    #[test]
    fn needs_escalation_at_80_percent() {
        let mut vault = QuarantineVault::new(0.5, 10, false);
        for i in 0..8 {
            vault.quarantine(dummy_hash(i), AnomalyScore::from_score(0.6));
        }
        assert!(vault.needs_escalation());
    }

    #[test]
    fn no_escalation_when_below_threshold() {
        let mut vault = QuarantineVault::new(0.5, 10, false);
        for i in 0..5 {
            vault.quarantine(dummy_hash(i), AnomalyScore::from_score(0.6));
        }
        assert!(!vault.needs_escalation());
    }

    #[test]
    fn serde_roundtrip_quarantine_entry() {
        let entry = QuarantineEntry {
            hash: dummy_hash(1),
            anomaly_score: AnomalyScore::from_score(0.85)
                .with_dimension("coherence", 0.3)
                .with_taint(Taint::LlmHallucination {
                    detail: "made up fact".into(),
                }),
            quarantined_at: Utc::now(),
            status: QuarantineStatus::Pending,
            reviewer_notes: Some("checking".into()),
            incident_links: vec![IncidentLink {
                related_hash: dummy_hash(2),
                relation: IncidentRelation::SameSource,
                linked_at: Utc::now(),
            }],
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: QuarantineEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.hash, entry.hash);
        assert_eq!(back.status, QuarantineStatus::Pending);
        assert_eq!(back.incident_links.len(), 1);
    }

    #[test]
    fn serde_roundtrip_immune_response() {
        let resp = ImmuneResponse {
            entry_hash: dummy_hash(1),
            action: ResponseAction::Release,
            responded_at: Utc::now(),
            description: Some("confirmed valid".into()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: ImmuneResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.action, ResponseAction::Release);
    }
}
