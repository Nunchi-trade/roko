//! Validation Registry for work proof attestation (CHAIN-12).
//!
//! Stores on-chain records of completed work: when an agent completes a job
//! and the result passes gate verification, the result hash and gate scores
//! are recorded. This provides a tamper-evident record that feeds the
//! reputation system.
//!
//! Part of the 3-registry pattern: Identity (who), Reputation (how well),
//! Validation (what was done and verified).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::phase2::{WorkProof, u256};

/// Configuration for the validation registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationRegistryConfig {
    /// Minimum gate pass rate to accept a proof.
    pub min_gate_pass_rate: f64,
    /// Maximum age (in blocks) for a proof to be considered recent.
    pub recent_proof_window_blocks: u64,
    /// Whether duplicate proofs for the same job are rejected.
    pub reject_duplicates: bool,
}

impl Default for ValidationRegistryConfig {
    fn default() -> Self {
        Self {
            min_gate_pass_rate: 0.5,
            recent_proof_window_blocks: 1000,
            reject_duplicates: true,
        }
    }
}

/// Decoded gate result from a work proof.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateScore {
    /// Verify kind identifier.
    pub gate_kind: String,
    /// Score (0.0 - 1.0).
    pub score: f64,
    /// Whether this gate passed.
    pub passed: bool,
}

/// Extended proof record maintained by the registry.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationRecord {
    /// The underlying work proof.
    pub proof: WorkProof,
    /// Decoded gate scores.
    pub gate_scores: Vec<GateScore>,
    /// Overall pass rate across all gates.
    pub overall_pass_rate: f64,
    /// Whether the proof was accepted by the registry.
    pub accepted: bool,
    /// Optional attester who independently verified the proof.
    pub attester_passport_id: Option<u256>,
}

/// The validation registry.
#[derive(Debug, Clone, Default)]
pub struct ValidationRegistry {
    /// Configuration.
    pub config: ValidationRegistryConfig,
    /// Records by job hash.
    records: HashMap<[u8; 32], Vec<ValidationRecord>>,
    /// Records by passport ID.
    records_by_passport: HashMap<u256, Vec<[u8; 32]>>,
    /// Total accepted proofs.
    accepted_count: usize,
    /// Total rejected proofs.
    rejected_count: usize,
}

impl ValidationRegistry {
    /// Create a new validation registry.
    #[must_use]
    pub fn new(config: ValidationRegistryConfig) -> Self {
        Self {
            config,
            ..Default::default()
        }
    }

    /// Submit a work proof with gate scores.
    ///
    /// Returns the validation record indicating acceptance or rejection.
    pub fn submit_proof(
        &mut self,
        proof: WorkProof,
        gate_scores: Vec<GateScore>,
        attester: Option<u256>,
    ) -> Result<ValidationRecord, ValidationError> {
        // Check for duplicates
        if self.config.reject_duplicates {
            if let Some(existing) = self.records.get(&proof.job_hash) {
                if existing
                    .iter()
                    .any(|r| r.proof.passport_id == proof.passport_id)
                {
                    return Err(ValidationError::DuplicateProof {
                        job_hash: proof.job_hash,
                        passport_id: proof.passport_id,
                    });
                }
            }
        }

        let pass_count = gate_scores.iter().filter(|g| g.passed).count();
        let total_gates = gate_scores.len().max(1);
        let overall_pass_rate = pass_count as f64 / total_gates as f64;
        let accepted = overall_pass_rate >= self.config.min_gate_pass_rate;

        if accepted {
            self.accepted_count += 1;
        } else {
            self.rejected_count += 1;
        }

        let record = ValidationRecord {
            proof: proof.clone(),
            gate_scores,
            overall_pass_rate,
            accepted,
            attester_passport_id: attester,
        };

        self.records
            .entry(proof.job_hash)
            .or_default()
            .push(record.clone());

        self.records_by_passport
            .entry(proof.passport_id)
            .or_default()
            .push(proof.job_hash);

        Ok(record)
    }

    /// Verify that a proof exists and was accepted for a given job.
    #[must_use]
    pub fn verify_proof(&self, job_hash: &[u8; 32], passport_id: u256) -> VerificationResult {
        let Some(records) = self.records.get(job_hash) else {
            return VerificationResult::NotFound;
        };

        let Some(record) = records.iter().find(|r| r.proof.passport_id == passport_id) else {
            return VerificationResult::NotFound;
        };

        if record.accepted {
            VerificationResult::Verified {
                pass_rate: record.overall_pass_rate,
                block_number: record.proof.block_number,
                attested: record.attester_passport_id.is_some(),
            }
        } else {
            VerificationResult::Rejected {
                pass_rate: record.overall_pass_rate,
                threshold: self.config.min_gate_pass_rate,
            }
        }
    }

    /// Get all validation records for a passport.
    #[must_use]
    pub fn records_for_passport(&self, passport_id: u256) -> Vec<&ValidationRecord> {
        let Some(job_hashes) = self.records_by_passport.get(&passport_id) else {
            return Vec::new();
        };
        job_hashes
            .iter()
            .flat_map(|h| {
                self.records.get(h).into_iter().flat_map(|records| {
                    records
                        .iter()
                        .filter(|r| r.proof.passport_id == passport_id)
                })
            })
            .collect()
    }

    /// Count of accepted proofs in the registry.
    #[must_use]
    pub fn accepted_count(&self) -> usize {
        self.accepted_count
    }

    /// Count of rejected proofs in the registry.
    #[must_use]
    pub fn rejected_count(&self) -> usize {
        self.rejected_count
    }

    /// Total number of unique jobs with validation records.
    #[must_use]
    pub fn job_count(&self) -> usize {
        self.records.len()
    }

    /// Get recent accepted proofs within the window.
    #[must_use]
    pub fn recent_accepted(&self, current_block: u64) -> Vec<&ValidationRecord> {
        let cutoff = current_block.saturating_sub(self.config.recent_proof_window_blocks);
        self.records
            .values()
            .flat_map(|records| {
                records
                    .iter()
                    .filter(|r| r.accepted && r.proof.block_number >= cutoff)
            })
            .collect()
    }
}

/// Result of a proof verification lookup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VerificationResult {
    /// Proof was found and accepted.
    Verified {
        /// Overall gate pass rate.
        pass_rate: f64,
        /// Block number of the proof.
        block_number: u64,
        /// Whether the proof had an independent attester.
        attested: bool,
    },
    /// Proof was found but rejected.
    Rejected {
        /// Overall gate pass rate.
        pass_rate: f64,
        /// Threshold that was not met.
        threshold: f64,
    },
    /// No proof found for this job/passport combination.
    NotFound,
}

/// Errors from the validation registry.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ValidationError {
    /// Duplicate proof submitted.
    #[error("duplicate proof for job {job_hash:?} by passport {passport_id}")]
    DuplicateProof {
        /// Job hash.
        job_hash: [u8; 32],
        /// Passport that already submitted.
        passport_id: u256,
    },
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_proof(passport_id: u256, job_hash: [u8; 32]) -> WorkProof {
        WorkProof {
            passport_id,
            job_hash,
            deliverable_merkle_root: [0u8; 32],
            gate_results: vec![1, 1, 0], // encoded pass/fail
            clearing_cert: Vec::new(),
            block_number: 100,
            timestamp: 1_700_000_000,
        }
    }

    fn test_gates(passed: &[bool]) -> Vec<GateScore> {
        passed
            .iter()
            .enumerate()
            .map(|(i, &p)| GateScore {
                gate_kind: format!("gate_{i}"),
                score: if p { 0.9 } else { 0.2 },
                passed: p,
            })
            .collect()
    }

    #[test]
    fn submit_and_verify_accepted_proof() {
        let mut registry = ValidationRegistry::new(ValidationRegistryConfig::default());
        let job_hash = [1u8; 32];

        let record = registry
            .submit_proof(
                test_proof(42, job_hash),
                test_gates(&[true, true, true]),
                None,
            )
            .unwrap();

        assert!(record.accepted);
        assert!((record.overall_pass_rate - 1.0).abs() < 1e-10);
        assert_eq!(registry.accepted_count(), 1);

        match registry.verify_proof(&job_hash, 42) {
            VerificationResult::Verified {
                pass_rate,
                block_number,
                attested,
            } => {
                assert!((pass_rate - 1.0).abs() < 1e-10);
                assert_eq!(block_number, 100);
                assert!(!attested);
            }
            other => panic!("expected Verified, got {:?}", other),
        }
    }

    #[test]
    fn submit_rejected_proof() {
        let mut registry = ValidationRegistry::new(ValidationRegistryConfig {
            min_gate_pass_rate: 0.8,
            ..Default::default()
        });

        let record = registry
            .submit_proof(
                test_proof(42, [1u8; 32]),
                test_gates(&[true, false, false]),
                None,
            )
            .unwrap();

        assert!(!record.accepted);
        assert!((record.overall_pass_rate - 1.0 / 3.0).abs() < 0.01);
        assert_eq!(registry.rejected_count(), 1);
    }

    #[test]
    fn verify_nonexistent_returns_not_found() {
        let registry = ValidationRegistry::new(ValidationRegistryConfig::default());
        assert_eq!(
            registry.verify_proof(&[0u8; 32], 99),
            VerificationResult::NotFound
        );
    }

    #[test]
    fn duplicate_proof_rejected() {
        let mut registry = ValidationRegistry::new(ValidationRegistryConfig::default());
        let job_hash = [1u8; 32];

        registry
            .submit_proof(test_proof(42, job_hash), test_gates(&[true]), None)
            .unwrap();

        let err = registry
            .submit_proof(test_proof(42, job_hash), test_gates(&[true]), None)
            .unwrap_err();

        assert!(matches!(err, ValidationError::DuplicateProof { .. }));
    }

    #[test]
    fn different_passports_can_submit_for_same_job() {
        let mut registry = ValidationRegistry::new(ValidationRegistryConfig::default());
        let job_hash = [1u8; 32];

        registry
            .submit_proof(test_proof(42, job_hash), test_gates(&[true]), None)
            .unwrap();
        registry
            .submit_proof(test_proof(43, job_hash), test_gates(&[true]), None)
            .unwrap();

        assert_eq!(registry.accepted_count(), 2);
    }

    #[test]
    fn records_for_passport() {
        let mut registry = ValidationRegistry::new(ValidationRegistryConfig::default());

        registry
            .submit_proof(test_proof(42, [1u8; 32]), test_gates(&[true]), None)
            .unwrap();
        registry
            .submit_proof(test_proof(42, [2u8; 32]), test_gates(&[true]), None)
            .unwrap();
        registry
            .submit_proof(test_proof(99, [3u8; 32]), test_gates(&[true]), None)
            .unwrap();

        let records = registry.records_for_passport(42);
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn attested_proof_marked_correctly() {
        let mut registry = ValidationRegistry::new(ValidationRegistryConfig::default());
        let job_hash = [1u8; 32];

        registry
            .submit_proof(
                test_proof(42, job_hash),
                test_gates(&[true]),
                Some(99), // attester
            )
            .unwrap();

        match registry.verify_proof(&job_hash, 42) {
            VerificationResult::Verified { attested, .. } => {
                assert!(attested, "proof should be marked as attested");
            }
            other => panic!("expected Verified, got {:?}", other),
        }
    }

    #[test]
    fn recent_accepted_respects_window() {
        let mut registry = ValidationRegistry::new(ValidationRegistryConfig {
            recent_proof_window_blocks: 50,
            ..Default::default()
        });

        let mut old_proof = test_proof(42, [1u8; 32]);
        old_proof.block_number = 10;
        registry
            .submit_proof(old_proof, test_gates(&[true]), None)
            .unwrap();

        let mut new_proof = test_proof(43, [2u8; 32]);
        new_proof.block_number = 90;
        registry
            .submit_proof(new_proof, test_gates(&[true]), None)
            .unwrap();

        let recent = registry.recent_accepted(100);
        assert_eq!(recent.len(), 1); // Only block 90 is within window [50, 100]
        assert_eq!(recent[0].proof.passport_id, 43);
    }

    #[test]
    fn job_count_tracks_unique_jobs() {
        let mut registry = ValidationRegistry::new(ValidationRegistryConfig::default());

        registry
            .submit_proof(test_proof(1, [1u8; 32]), test_gates(&[true]), None)
            .unwrap();
        registry
            .submit_proof(test_proof(2, [1u8; 32]), test_gates(&[true]), None)
            .unwrap();
        registry
            .submit_proof(test_proof(3, [2u8; 32]), test_gates(&[true]), None)
            .unwrap();

        assert_eq!(registry.job_count(), 2);
    }
}
