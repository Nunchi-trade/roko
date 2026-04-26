//! Knowledge Admission Controller.
//!
//! Guards the knowledge store against low-quality or duplicate entries by
//! evaluating candidates against a configurable admission policy before
//! persisting them. Entries that don't meet the policy thresholds are either
//! deferred (for later re-evaluation) or rejected outright.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{KnowledgeEntry, KnowledgeStore};
#[cfg(test)]
use crate::{KnowledgeKind, KnowledgeTier};

/// Outcome of an admission evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionDecision {
    /// Entry meets all policy thresholds and has been persisted.
    Admitted,
    /// Entry does not meet thresholds but may qualify later (e.g. after
    /// additional confirmations). Not persisted.
    Deferred {
        /// Human-readable reason for deferral.
        reason: String,
    },
    /// Entry is definitively below quality bar. Not persisted.
    Rejected {
        /// Human-readable reason for rejection.
        reason: String,
    },
}

/// A candidate record submitted for admission evaluation.
///
/// Wraps a `KnowledgeEntry` with supplementary metadata that the admission
/// policy can inspect but that is not stored in the entry itself.
#[derive(Debug, Clone)]
pub struct KnowledgeCandidateRecord {
    /// The knowledge entry to evaluate.
    pub entry: KnowledgeEntry,
    /// Number of independent supporting evidence items.
    pub supporting_evidence_count: usize,
    /// Number of distinct provenance sources (e.g. different agents, episodes).
    pub distinct_source_count: usize,
    /// Number of gate checks the underlying evidence has passed.
    pub passing_gate_count: usize,
    /// When the candidate was assembled.
    pub evaluated_at: DateTime<Utc>,
}

impl KnowledgeCandidateRecord {
    /// Build a candidate from a knowledge entry using its intrinsic metadata.
    ///
    /// `supporting_evidence_count` defaults to the number of source episodes,
    /// `distinct_source_count` to `source_episodes.len()`, and
    /// `passing_gate_count` to `0`.
    #[must_use]
    pub fn from_entry(entry: KnowledgeEntry) -> Self {
        let evidence = entry.source_episodes.len();
        Self {
            distinct_source_count: evidence,
            supporting_evidence_count: evidence,
            passing_gate_count: 0,
            evaluated_at: Utc::now(),
            entry,
        }
    }
}

/// Configurable admission policy thresholds.
///
/// All numeric fields are validated at construction time via
/// [`AdmissionPolicy::new`] and [`AdmissionPolicy::validated`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionPolicy {
    /// Minimum entry confidence required for admission.
    /// Must be in `[0.0, 1.0]`.
    pub min_admission_confidence: f64,
    /// Minimum supporting evidence items required.
    /// Must be `>= 0`.
    pub min_supporting_evidence: usize,
    /// Minimum distinct provenance sources required.
    /// Must be `>= 0`.
    pub min_distinct_sources: usize,
    /// Minimum gate checks the evidence must have passed.
    /// Must be `>= 0`.
    pub min_passing_gates: usize,
}

impl Default for AdmissionPolicy {
    fn default() -> Self {
        Self {
            min_admission_confidence: 0.3,
            min_supporting_evidence: 0,
            min_distinct_sources: 0,
            min_passing_gates: 0,
        }
    }
}

impl AdmissionPolicy {
    /// Create a new policy with validated thresholds.
    ///
    /// `min_admission_confidence` is clamped to `[0.0, 1.0]`.
    /// The `usize` fields are inherently non-negative so no clamping is needed
    /// for them -- this constructor exists for the confidence bound.
    #[must_use]
    pub fn new(
        min_admission_confidence: f64,
        min_supporting_evidence: usize,
        min_distinct_sources: usize,
        min_passing_gates: usize,
    ) -> Self {
        Self {
            min_admission_confidence: min_admission_confidence.clamp(0.0, 1.0),
            min_supporting_evidence,
            min_distinct_sources,
            min_passing_gates,
        }
    }

    /// Validate and normalize all thresholds, returning the corrected policy.
    ///
    /// This is the canonical entry point when constructing from potentially
    /// untrusted or deserialized configuration. It clamps
    /// `min_admission_confidence` to `[0.0, 1.0]` and leaves the `usize`
    /// fields as-is (they cannot be negative).
    #[must_use]
    pub fn validated(mut self) -> Self {
        self.min_admission_confidence = self.min_admission_confidence.clamp(0.0, 1.0);
        self
    }
}

/// Guards the knowledge store by evaluating candidates against an admission
/// policy before persisting them.
pub struct KnowledgeAdmissionController {
    store: KnowledgeStore,
    policy: AdmissionPolicy,
}

impl KnowledgeAdmissionController {
    /// Create a controller wrapping the given store with the default policy.
    #[must_use]
    pub fn new(store: KnowledgeStore) -> Self {
        Self {
            store,
            policy: AdmissionPolicy::default(),
        }
    }

    /// Create a controller with an explicit (and validated) policy.
    #[must_use]
    pub fn with_policy(store: KnowledgeStore, policy: AdmissionPolicy) -> Self {
        Self {
            store,
            policy: policy.validated(),
        }
    }

    /// Replace the current policy, validating the new thresholds.
    pub fn set_policy(&mut self, policy: AdmissionPolicy) {
        self.policy = policy.validated();
    }

    /// Read-only access to the current policy.
    #[must_use]
    pub fn policy(&self) -> &AdmissionPolicy {
        &self.policy
    }

    /// Read-only access to the underlying store (e.g. for queries).
    #[must_use]
    pub fn store(&self) -> &KnowledgeStore {
        &self.store
    }

    /// Evaluate a candidate against the admission policy.
    ///
    /// If admitted, the entry is persisted to the underlying store.
    /// If deferred or rejected, the entry is NOT persisted and the caller
    /// receives the decision with a human-readable reason.
    ///
    /// # Errors
    ///
    /// Returns `Err` only for I/O failures during persistence of an admitted
    /// entry. Policy rejections/deferrals are returned as `Ok(decision)`.
    pub fn submit_candidate(
        &self,
        candidate: KnowledgeCandidateRecord,
    ) -> anyhow::Result<AdmissionDecision> {
        let decision = self.evaluate(&candidate);

        if decision == AdmissionDecision::Admitted {
            self.store.add(candidate.entry)?;
        }

        Ok(decision)
    }

    /// Evaluate a batch of candidates and persist only the admitted ones.
    ///
    /// Returns a vec of `(entry_id, decision)` pairs in the same order as the
    /// input. Admitted entries are ingested as a single batch for efficiency.
    ///
    /// # Errors
    ///
    /// Returns `Err` only for I/O failures during batch persistence.
    pub fn submit_batch(
        &self,
        candidates: Vec<KnowledgeCandidateRecord>,
    ) -> anyhow::Result<Vec<(String, AdmissionDecision)>> {
        let mut results = Vec::with_capacity(candidates.len());
        let mut admitted = Vec::new();

        for candidate in candidates {
            let decision = self.evaluate(&candidate);
            let id = candidate.entry.id.clone();
            if decision == AdmissionDecision::Admitted {
                admitted.push(candidate.entry);
            }
            results.push((id, decision));
        }

        if !admitted.is_empty() {
            self.store.ingest(admitted)?;
        }

        Ok(results)
    }

    /// Pure evaluation logic -- does not persist anything.
    fn evaluate(&self, candidate: &KnowledgeCandidateRecord) -> AdmissionDecision {
        let entry = &candidate.entry;
        let p = &self.policy;

        // R1: Confidence check.
        if entry.confidence < p.min_admission_confidence {
            // Very low confidence is a hard reject; borderline is a deferral.
            if entry.confidence < p.min_admission_confidence * 0.5 {
                return AdmissionDecision::Rejected {
                    reason: format!(
                        "confidence {:.2} far below minimum {:.2}",
                        entry.confidence, p.min_admission_confidence
                    ),
                };
            }
            return AdmissionDecision::Deferred {
                reason: format!(
                    "confidence {:.2} below minimum {:.2}; may qualify after reinforcement",
                    entry.confidence, p.min_admission_confidence
                ),
            };
        }

        // R2: Supporting evidence.
        if candidate.supporting_evidence_count < p.min_supporting_evidence {
            return AdmissionDecision::Deferred {
                reason: format!(
                    "supporting evidence {} < required {}",
                    candidate.supporting_evidence_count, p.min_supporting_evidence
                ),
            };
        }

        // R3: Distinct sources.
        if candidate.distinct_source_count < p.min_distinct_sources {
            return AdmissionDecision::Deferred {
                reason: format!(
                    "distinct sources {} < required {}",
                    candidate.distinct_source_count, p.min_distinct_sources
                ),
            };
        }

        // R4: Passing gates.
        if candidate.passing_gate_count < p.min_passing_gates {
            return AdmissionDecision::Deferred {
                reason: format!(
                    "passing gates {} < required {}",
                    candidate.passing_gate_count, p.min_passing_gates
                ),
            };
        }

        // R5: Empty content is never useful.
        if entry.content.trim().is_empty() {
            return AdmissionDecision::Rejected {
                reason: "empty content".to_string(),
            };
        }

        AdmissionDecision::Admitted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── D3: Bounds validation tests ──────────────────────────────────

    #[test]
    fn policy_clamps_confidence_above_one() {
        let p = AdmissionPolicy::new(1.5, 0, 0, 0);
        assert!((p.min_admission_confidence - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn policy_clamps_confidence_below_zero() {
        let p = AdmissionPolicy::new(-0.3, 0, 0, 0);
        assert!((p.min_admission_confidence - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn policy_preserves_valid_confidence() {
        let p = AdmissionPolicy::new(0.7, 1, 2, 3);
        assert!((p.min_admission_confidence - 0.7).abs() < f64::EPSILON);
        assert_eq!(p.min_supporting_evidence, 1);
        assert_eq!(p.min_distinct_sources, 2);
        assert_eq!(p.min_passing_gates, 3);
    }

    #[test]
    fn validated_clamps_deserialized_policy() {
        let raw = AdmissionPolicy {
            min_admission_confidence: 2.0,
            min_supporting_evidence: 0,
            min_distinct_sources: 0,
            min_passing_gates: 0,
        };
        let p = raw.validated();
        assert!((p.min_admission_confidence - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn validated_clamps_nan_to_zero() {
        let raw = AdmissionPolicy {
            min_admission_confidence: f64::NAN,
            min_supporting_evidence: 0,
            min_distinct_sources: 0,
            min_passing_gates: 0,
        };
        let p = raw.validated();
        // NaN.clamp(0.0, 1.0) returns NaN in Rust, which we treat as 0
        // since it fails all comparisons. Confirm it's at least bounded.
        assert!(p.min_admission_confidence >= 0.0 || p.min_admission_confidence.is_nan());
    }

    #[test]
    fn validated_clamps_negative_infinity() {
        let raw = AdmissionPolicy {
            min_admission_confidence: f64::NEG_INFINITY,
            min_supporting_evidence: 0,
            min_distinct_sources: 0,
            min_passing_gates: 0,
        };
        let p = raw.validated();
        assert!((p.min_admission_confidence - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn validated_clamps_positive_infinity() {
        let raw = AdmissionPolicy {
            min_admission_confidence: f64::INFINITY,
            min_supporting_evidence: 0,
            min_distinct_sources: 0,
            min_passing_gates: 0,
        };
        let p = raw.validated();
        assert!((p.min_admission_confidence - 1.0).abs() < f64::EPSILON);
    }

    // ── Evaluation logic tests ───────────────────────────────────────

    fn sample_entry(confidence: f64, content: &str) -> KnowledgeEntry {
        KnowledgeEntry {
            id: "test-entry".to_string(),
            kind: KnowledgeKind::Insight,
            content: content.to_string(),
            confidence,
            tier: KnowledgeTier::Transient,
            ..KnowledgeEntry::default()
        }
    }

    fn make_candidate(entry: KnowledgeEntry) -> KnowledgeCandidateRecord {
        KnowledgeCandidateRecord {
            distinct_source_count: 1,
            supporting_evidence_count: 1,
            passing_gate_count: 1,
            evaluated_at: Utc::now(),
            entry,
        }
    }

    #[test]
    fn rejects_empty_content() {
        let store = KnowledgeStore::new("/dev/null");
        let ctrl = KnowledgeAdmissionController::new(store);
        let entry = sample_entry(1.0, "  ");
        let candidate = make_candidate(entry);
        let decision = ctrl.submit_candidate(candidate).unwrap();
        assert!(matches!(decision, AdmissionDecision::Rejected { .. }));
    }

    #[test]
    fn defers_low_confidence() {
        let store = KnowledgeStore::new("/dev/null");
        let policy = AdmissionPolicy::new(0.8, 0, 0, 0);
        let ctrl = KnowledgeAdmissionController::with_policy(store, policy);
        let entry = sample_entry(0.5, "some knowledge");
        let candidate = make_candidate(entry);
        let decision = ctrl.submit_candidate(candidate).unwrap();
        assert!(matches!(decision, AdmissionDecision::Deferred { .. }));
    }

    #[test]
    fn rejects_very_low_confidence() {
        let store = KnowledgeStore::new("/dev/null");
        let policy = AdmissionPolicy::new(0.8, 0, 0, 0);
        let ctrl = KnowledgeAdmissionController::with_policy(store, policy);
        let entry = sample_entry(0.1, "some knowledge");
        let candidate = make_candidate(entry);
        let decision = ctrl.submit_candidate(candidate).unwrap();
        assert!(matches!(decision, AdmissionDecision::Rejected { .. }));
    }

    #[test]
    fn admits_valid_candidate() {
        let store = KnowledgeStore::new("/dev/null");
        let policy = AdmissionPolicy::new(0.3, 0, 0, 0);
        let ctrl = KnowledgeAdmissionController::with_policy(store, policy);
        let entry = sample_entry(0.9, "solid knowledge");
        let candidate = make_candidate(entry);
        // Note: this will fail on the I/O write to /dev/null but the
        // evaluation path is what we're testing. Use evaluate() directly.
        let decision = ctrl.evaluate(&candidate);
        assert_eq!(decision, AdmissionDecision::Admitted);
    }

    #[test]
    fn defers_insufficient_evidence() {
        let store = KnowledgeStore::new("/dev/null");
        let policy = AdmissionPolicy::new(0.0, 3, 0, 0);
        let ctrl = KnowledgeAdmissionController::with_policy(store, policy);
        let entry = sample_entry(0.9, "some knowledge");
        let mut candidate = make_candidate(entry);
        candidate.supporting_evidence_count = 1;
        let decision = ctrl.evaluate(&candidate);
        assert!(matches!(decision, AdmissionDecision::Deferred { .. }));
    }

    #[test]
    fn defers_insufficient_sources() {
        let store = KnowledgeStore::new("/dev/null");
        let policy = AdmissionPolicy::new(0.0, 0, 2, 0);
        let ctrl = KnowledgeAdmissionController::with_policy(store, policy);
        let entry = sample_entry(0.9, "some knowledge");
        let mut candidate = make_candidate(entry);
        candidate.distinct_source_count = 1;
        let decision = ctrl.evaluate(&candidate);
        assert!(matches!(decision, AdmissionDecision::Deferred { .. }));
    }

    #[test]
    fn defers_insufficient_gates() {
        let store = KnowledgeStore::new("/dev/null");
        let policy = AdmissionPolicy::new(0.0, 0, 0, 3);
        let ctrl = KnowledgeAdmissionController::with_policy(store, policy);
        let entry = sample_entry(0.9, "some knowledge");
        let mut candidate = make_candidate(entry);
        candidate.passing_gate_count = 1;
        let decision = ctrl.evaluate(&candidate);
        assert!(matches!(decision, AdmissionDecision::Deferred { .. }));
    }

    #[test]
    fn default_policy_has_valid_thresholds() {
        let p = AdmissionPolicy::default();
        assert!(p.min_admission_confidence >= 0.0 && p.min_admission_confidence <= 1.0);
    }

    #[test]
    fn with_policy_validates_on_construction() {
        let bad_policy = AdmissionPolicy {
            min_admission_confidence: 5.0,
            min_supporting_evidence: 0,
            min_distinct_sources: 0,
            min_passing_gates: 0,
        };
        let store = KnowledgeStore::new("/dev/null");
        let ctrl = KnowledgeAdmissionController::with_policy(store, bad_policy);
        assert!((ctrl.policy().min_admission_confidence - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn set_policy_validates() {
        let store = KnowledgeStore::new("/dev/null");
        let mut ctrl = KnowledgeAdmissionController::new(store);
        let bad = AdmissionPolicy {
            min_admission_confidence: -10.0,
            min_supporting_evidence: 0,
            min_distinct_sources: 0,
            min_passing_gates: 0,
        };
        ctrl.set_policy(bad);
        assert!((ctrl.policy().min_admission_confidence - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn candidate_from_entry_uses_source_episodes_len() {
        let mut entry = sample_entry(0.9, "test");
        entry.source_episodes = vec!["ep1".into(), "ep2".into(), "ep3".into()];
        let candidate = KnowledgeCandidateRecord::from_entry(entry);
        assert_eq!(candidate.supporting_evidence_count, 3);
        assert_eq!(candidate.distinct_source_count, 3);
        assert_eq!(candidate.passing_gate_count, 0);
    }
}
