//! Research-to-runtime pipeline: Paper -> Claim -> Heuristic -> Trial -> Ledger.
//!
//! Implements the LEARN-11 pipeline that ingests research papers, extracts
//! testable claims with falsifiers, tracks replication trials, and promotes
//! validated findings to runtime heuristics.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::heuristics::{
    Claim, ClaimId, Heuristic, Paper, PaperId, ReplicationLedger, ReplicationStatus,
};

/// Outcome of a single replication trial.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trial {
    /// Which claim was tested.
    pub claim_id: ClaimId,
    /// Whether the trial confirmed the claim.
    pub confirmed: bool,
    /// Observed effect size (0.0-1.0 normalized).
    pub observed_effect: f64,
    /// Number of observations in this trial.
    pub sample_size: u32,
    /// When the trial was run.
    pub timestamp: DateTime<Utc>,
    /// Optional context (task type, model, etc.).
    pub context: String,
}

/// The research-to-runtime pipeline.
///
/// Manages the lifecycle: Paper ingestion -> Claim extraction -> Trial
/// tracking -> Ledger updates -> Heuristic promotion.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResearchPipeline {
    /// Ingested papers keyed by paper ID.
    papers: HashMap<PaperId, Paper>,
    /// Extracted claims keyed by claim ID.
    claims: HashMap<ClaimId, Claim>,
    /// Replication trials per claim.
    trials: HashMap<ClaimId, Vec<Trial>>,
    /// Replication ledger per claim.
    ledgers: HashMap<ClaimId, ReplicationLedger>,
    /// Minimum trials before a claim can be promoted to a heuristic.
    min_trials_for_promotion: u32,
    /// Minimum replication rate (confirmations/trials) for promotion.
    min_replication_rate: f64,
}

impl ResearchPipeline {
    /// Create a new empty pipeline.
    #[must_use]
    pub fn new() -> Self {
        Self {
            papers: HashMap::new(),
            claims: HashMap::new(),
            trials: HashMap::new(),
            ledgers: HashMap::new(),
            min_trials_for_promotion: 5,
            min_replication_rate: 0.6,
        }
    }

    /// Configure the minimum trials needed before promotion.
    #[must_use]
    pub fn with_min_trials(mut self, n: u32) -> Self {
        self.min_trials_for_promotion = n.max(1);
        self
    }

    /// Configure the minimum replication rate for promotion.
    #[must_use]
    pub fn with_min_replication_rate(mut self, rate: f64) -> Self {
        self.min_replication_rate = rate.clamp(0.0, 1.0);
        self
    }

    /// Ingest a paper into the pipeline.
    pub fn ingest_paper(&mut self, paper: Paper) {
        self.papers.insert(paper.id.clone(), paper);
    }

    /// Extract and register a claim from a paper.
    ///
    /// The claim must reference an already-ingested paper. Returns the claim
    /// ID if successful.
    pub fn register_claim(&mut self, claim: Claim) -> Option<ClaimId> {
        if !self.papers.contains_key(&claim.paper) {
            return None;
        }

        let claim_id = claim.id.clone();

        // Add claim to its paper's claim list.
        if let Some(paper) = self.papers.get_mut(&claim.paper) {
            if !paper.claims.contains(&claim_id) {
                paper.claims.push(claim_id.clone());
            }
        }

        // Initialize ledger if not present.
        self.ledgers
            .entry(claim_id.clone())
            .or_insert_with(|| ReplicationLedger::new(&claim_id, 0.0, 0.0, 0));

        self.claims.insert(claim_id.clone(), claim);
        Some(claim_id)
    }

    /// Record a replication trial for a claim.
    ///
    /// Updates the claim's calibration and the replication ledger.
    pub fn record_trial(&mut self, trial: Trial) {
        let claim_id = trial.claim_id.clone();

        // Update claim calibration.
        if let Some(claim) = self.claims.get_mut(&claim_id) {
            claim.calibration.trials += 1;
            if trial.confirmed {
                claim.calibration.confirmations += 1;
            } else {
                claim.calibration.violations += 1;
            }
            claim.calibration.last_trial_at = trial.timestamp;

            // Update Brier score.
            let predicted = if claim.calibration.trials > 1 {
                claim.calibration.confirmations as f64 / (claim.calibration.trials - 1) as f64
            } else {
                0.5
            };
            let actual = if trial.confirmed { 1.0 } else { 0.0 };
            let error_sq = (predicted - actual).powi(2);
            let n = claim.calibration.trials as f64;
            claim.calibration.brier_score =
                claim.calibration.brier_score * ((n - 1.0) / n) + error_sq / n;
        }

        // Update replication ledger.
        if let Some(ledger) = self.ledgers.get_mut(&claim_id) {
            ledger.our_n += trial.sample_size;
            // Running average of observed effect.
            let total_n = ledger.our_n as f64;
            let trial_n = trial.sample_size as f64;
            ledger.our_effect = ledger.our_effect * ((total_n - trial_n) / total_n)
                + trial.observed_effect * (trial_n / total_n);

            // Update status based on accumulated evidence.
            if let Some(claim) = self.claims.get(&claim_id) {
                let rate =
                    claim.calibration.confirmations as f64 / claim.calibration.trials.max(1) as f64;
                ledger.status = if rate >= 0.7 {
                    ReplicationStatus::Replicated
                } else if rate <= 0.3 {
                    ReplicationStatus::Diverged
                } else {
                    ReplicationStatus::Mixed
                };
            }
        }

        // Store the trial.
        self.trials.entry(claim_id).or_default().push(trial);
    }

    /// Check if a claim is eligible for promotion to a heuristic.
    #[must_use]
    pub fn is_promotable(&self, claim_id: &str) -> bool {
        let Some(claim) = self.claims.get(claim_id) else {
            return false;
        };
        if claim.calibration.trials < self.min_trials_for_promotion {
            return false;
        }
        let rate = claim.calibration.confirmations as f64 / claim.calibration.trials as f64;
        rate >= self.min_replication_rate
    }

    /// Promote a validated claim to a runtime heuristic.
    ///
    /// Returns `None` if the claim doesn't meet promotion criteria.
    #[must_use]
    pub fn promote_to_heuristic(&self, claim_id: &str) -> Option<Heuristic> {
        if !self.is_promotable(claim_id) {
            return None;
        }

        let claim = self.claims.get(claim_id)?;
        let paper = self.papers.get(&claim.paper)?;

        let heuristic_id = format!("research:{}", claim.id);
        let description = format!("[{}] {}: {}", paper.title, claim.id, claim.hypothesis);

        let mut heuristic = Heuristic::new(
            &heuristic_id,
            description,
            claim.context.clone(),
            claim.falsifier.clone(),
        );
        heuristic.calibration = claim.calibration.clone();

        Some(heuristic)
    }

    /// Return all claims that are currently promotable.
    #[must_use]
    pub fn promotable_claims(&self) -> Vec<&ClaimId> {
        self.claims
            .keys()
            .filter(|id| self.is_promotable(id))
            .collect()
    }

    /// Return the replication ledger for a claim.
    #[must_use]
    pub fn ledger(&self, claim_id: &str) -> Option<&ReplicationLedger> {
        self.ledgers.get(claim_id)
    }

    /// Return the trials for a claim.
    #[must_use]
    pub fn trials_for(&self, claim_id: &str) -> &[Trial] {
        self.trials
            .get(claim_id)
            .map(|v| v.as_slice())
            .unwrap_or_default()
    }

    /// Return a reference to all ingested papers.
    #[must_use]
    pub fn papers(&self) -> &HashMap<PaperId, Paper> {
        &self.papers
    }

    /// Return a reference to all claims.
    #[must_use]
    pub fn claims(&self) -> &HashMap<ClaimId, Claim> {
        &self.claims
    }
}

/// Macro for inline claim definitions.
///
/// Usage:
/// ```ignore
/// let claim = claim!(
///     id: "claim-001",
///     paper: "paper-001",
///     hypothesis: "Chain-of-thought improves reasoning by 15%",
///     falsifier: Predicate::language_is(Language::Rust),
/// );
/// ```
#[macro_export]
macro_rules! claim {
    (
        id: $id:expr,
        paper: $paper:expr,
        hypothesis: $hyp:expr,
        falsifier: $falsifier:expr $(,)?
    ) => {
        $crate::heuristics::Claim::new($id, $paper, $hyp, $falsifier)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heuristics::{PaperProvenance, Predicate};
    use roko_core::project::Language;

    fn sample_paper() -> Paper {
        Paper::new(
            "paper-001",
            "Chain-of-Thought Prompting",
            vec!["Wei et al.".to_string()],
            2022,
            PaperProvenance::ExternalCitation("NeurIPS 2022".to_string()),
        )
    }

    fn sample_claim() -> Claim {
        Claim::new(
            "claim-001",
            "paper-001",
            "CoT improves reasoning by 15%",
            Predicate::language_is(Language::Rust),
        )
    }

    #[test]
    fn ingest_paper_and_register_claim() {
        let mut pipeline = ResearchPipeline::new();
        pipeline.ingest_paper(sample_paper());

        let claim_id = pipeline.register_claim(sample_claim());
        assert_eq!(claim_id, Some("claim-001".to_string()));
        assert!(pipeline.claims.contains_key("claim-001"));
        assert!(
            pipeline
                .papers
                .get("paper-001")
                .unwrap()
                .claims
                .contains(&"claim-001".to_string())
        );
    }

    #[test]
    fn register_claim_without_paper_fails() {
        let mut pipeline = ResearchPipeline::new();
        let claim_id = pipeline.register_claim(sample_claim());
        assert!(claim_id.is_none());
    }

    #[test]
    fn record_trial_updates_calibration() {
        let mut pipeline = ResearchPipeline::new();
        pipeline.ingest_paper(sample_paper());
        pipeline.register_claim(sample_claim());

        pipeline.record_trial(Trial {
            claim_id: "claim-001".to_string(),
            confirmed: true,
            observed_effect: 0.15,
            sample_size: 10,
            timestamp: Utc::now(),
            context: "test".to_string(),
        });

        let claim = pipeline.claims.get("claim-001").unwrap();
        assert_eq!(claim.calibration.trials, 1);
        assert_eq!(claim.calibration.confirmations, 1);
    }

    #[test]
    fn promotion_requires_sufficient_trials() {
        let mut pipeline = ResearchPipeline::new().with_min_trials(3);
        pipeline.ingest_paper(sample_paper());
        pipeline.register_claim(sample_claim());

        // Not enough trials.
        assert!(!pipeline.is_promotable("claim-001"));

        // Add enough confirming trials.
        for _ in 0..3 {
            pipeline.record_trial(Trial {
                claim_id: "claim-001".to_string(),
                confirmed: true,
                observed_effect: 0.14,
                sample_size: 5,
                timestamp: Utc::now(),
                context: "test".to_string(),
            });
        }

        assert!(pipeline.is_promotable("claim-001"));
    }

    #[test]
    fn promote_to_heuristic_works() {
        let mut pipeline = ResearchPipeline::new().with_min_trials(2);
        pipeline.ingest_paper(sample_paper());
        pipeline.register_claim(sample_claim());

        for _ in 0..3 {
            pipeline.record_trial(Trial {
                claim_id: "claim-001".to_string(),
                confirmed: true,
                observed_effect: 0.15,
                sample_size: 5,
                timestamp: Utc::now(),
                context: "test".to_string(),
            });
        }

        let heuristic = pipeline.promote_to_heuristic("claim-001");
        assert!(heuristic.is_some());
        let h = heuristic.unwrap();
        assert!(h.id.starts_with("research:"));
        assert_eq!(h.calibration.trials, 3);
    }

    #[test]
    fn failed_trials_prevent_promotion() {
        let mut pipeline = ResearchPipeline::new()
            .with_min_trials(3)
            .with_min_replication_rate(0.6);
        pipeline.ingest_paper(sample_paper());
        pipeline.register_claim(sample_claim());

        // All trials fail.
        for _ in 0..5 {
            pipeline.record_trial(Trial {
                claim_id: "claim-001".to_string(),
                confirmed: false,
                observed_effect: 0.02,
                sample_size: 5,
                timestamp: Utc::now(),
                context: "test".to_string(),
            });
        }

        assert!(!pipeline.is_promotable("claim-001"));
    }

    #[test]
    fn ledger_status_updates_with_evidence() {
        let mut pipeline = ResearchPipeline::new();
        pipeline.ingest_paper(sample_paper());
        pipeline.register_claim(sample_claim());

        // Mostly confirming -> Replicated.
        for _ in 0..8 {
            pipeline.record_trial(Trial {
                claim_id: "claim-001".to_string(),
                confirmed: true,
                observed_effect: 0.14,
                sample_size: 5,
                timestamp: Utc::now(),
                context: "test".to_string(),
            });
        }

        let ledger = pipeline.ledger("claim-001").unwrap();
        assert_eq!(ledger.status, ReplicationStatus::Replicated);
    }

    #[test]
    fn claim_macro_works() {
        let _claim = claim!(
            id: "test-claim",
            paper: "test-paper",
            hypothesis: "Test hypothesis",
            falsifier: Predicate::language_is(Language::Rust),
        );
    }
}
