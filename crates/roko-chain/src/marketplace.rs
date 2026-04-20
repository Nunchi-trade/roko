//! Spore job marketplace with escrow and 3 hiring models (CHAIN-04).
//!
//! Implements the full job lifecycle state machine:
//! POSTED -> ASSIGNED -> IN_PROGRESS -> SUBMITTED -> SETTLED / DISPUTED / EXPIRED
//!
//! Three hiring models:
//! 1. RandomVRF -- power-of-two-choices (Sparrow, O(log log N) max load)
//! 2. BlindAuction -- commit-reveal Vickrey second-price with reputation adjustment
//! 3. DirectHire -- 1.5x premium, restricted to Protocol/Sovereign tier
//!
//! Escrow handles deposit/release/dispute/refund with 4-level dispute resolution.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::identity_economy_markets::{BountySpec, SparrowBid};
use crate::phase2::{
    DisputeLevel, DisputeOutcome, DisputeResolution, HiringModel, PassportTier, SporeJobPosting,
    u256,
};

// ---------------------------------------------------------------------------
// Job lifecycle
// ---------------------------------------------------------------------------

/// Job lifecycle states per the Spore spec (doc 10).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobState {
    /// Job posted and awaiting assignment.
    #[default]
    Posted,
    /// Agent assigned; work not yet started.
    Assigned,
    /// Agent is actively working.
    InProgress,
    /// Result submitted; awaiting settlement.
    Submitted,
    /// Settled successfully; escrow released.
    Settled,
    /// Under dispute; escrow locked.
    Disputed,
    /// Deadline passed without valid delivery.
    Expired,
}

/// A tracked job in the marketplace.
#[derive(Clone, Debug, PartialEq)]
pub struct MarketplaceJob {
    /// Job identifier.
    pub job_id: [u8; 32],
    /// Current lifecycle state.
    pub state: JobState,
    /// Poster passport ID.
    pub poster_passport_id: u256,
    /// Assigned agent passport ID (set on assignment).
    pub assigned_agent: Option<u256>,
    /// Budget deposited into escrow.
    pub budget: u256,
    /// Deadline block for delivery.
    pub deadline_block: u64,
    /// Hiring model used.
    pub hiring_model: HiringModel,
    /// Minimum reputation required for the job.
    pub min_reputation: f64,
    /// Minimum passport tier required.
    pub min_tier: PassportTier,
    /// Domain for the job.
    pub domain: String,
    /// Required capabilities bitmask.
    pub required_capabilities: u64,
    /// Result hash submitted by the agent.
    pub result_hash: Option<[u8; 32]>,
    /// Quality score from gate validation.
    pub quality_score: Option<f64>,
    /// Block at which job was posted.
    pub posted_at_block: u64,
    /// Payment determined by the hiring model.
    pub payment: Option<f64>,
}

// ---------------------------------------------------------------------------
// Escrow
// ---------------------------------------------------------------------------

/// Escrow entry tracking funds locked for a job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscrowEntry {
    /// Job identifier.
    pub job_id: [u8; 32],
    /// Depositor (poster) passport ID.
    pub depositor: u256,
    /// Amount held in escrow.
    pub amount: u256,
    /// Recipient (assigned agent) passport ID.
    pub recipient: Option<u256>,
    /// Whether the escrow has been released.
    pub released: bool,
    /// Whether the escrow is disputed.
    pub disputed: bool,
}

/// Configuration for the marketplace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketplaceConfig {
    /// Direct-hire premium multiplier (default 1.5).
    pub direct_hire_premium: f64,
    /// Platform fee fraction (default 0.02 = 2%).
    pub platform_fee_fraction: f64,
    /// Default challenge window in blocks for disputes.
    pub dispute_challenge_window: u64,
    /// Optimistic settlement window in blocks (72h at ~12s/block).
    pub optimistic_window_blocks: u64,
}

impl Default for MarketplaceConfig {
    fn default() -> Self {
        Self {
            direct_hire_premium: 1.5,
            platform_fee_fraction: 0.02,
            dispute_challenge_window: 100,
            optimistic_window_blocks: 21600, // ~72h at 12s blocks
        }
    }
}

/// The Spore marketplace: manages jobs, escrow, and dispute resolution.
#[derive(Debug, Clone)]
pub struct Marketplace {
    /// Configuration.
    pub config: MarketplaceConfig,
    /// Active jobs by ID.
    jobs: HashMap<[u8; 32], MarketplaceJob>,
    /// Escrow entries by job ID.
    escrow: HashMap<[u8; 32], EscrowEntry>,
    /// Active disputes by job ID.
    disputes: HashMap<[u8; 32], DisputeResolution>,
    /// Current block number.
    current_block: u64,
    /// Collected bids per job (for auction model).
    bids: HashMap<[u8; 32], Vec<SparrowBid>>,
}

impl Default for Marketplace {
    fn default() -> Self {
        Self {
            config: MarketplaceConfig::default(),
            jobs: HashMap::new(),
            escrow: HashMap::new(),
            disputes: HashMap::new(),
            current_block: 0,
            bids: HashMap::new(),
        }
    }
}

impl Marketplace {
    /// Create a new marketplace.
    #[must_use]
    pub fn new(config: MarketplaceConfig) -> Self {
        Self {
            config,
            ..Default::default()
        }
    }

    /// Set the current block number.
    pub fn set_block(&mut self, block: u64) {
        self.current_block = block;
    }

    /// Current block number.
    #[must_use]
    pub fn current_block(&self) -> u64 {
        self.current_block
    }

    /// Number of tracked jobs.
    #[must_use]
    pub fn job_count(&self) -> usize {
        self.jobs.len()
    }

    /// Get a job by ID.
    #[must_use]
    pub fn get_job(&self, job_id: &[u8; 32]) -> Option<&MarketplaceJob> {
        self.jobs.get(job_id)
    }

    /// Get an escrow entry by job ID.
    #[must_use]
    pub fn get_escrow(&self, job_id: &[u8; 32]) -> Option<&EscrowEntry> {
        self.escrow.get(job_id)
    }

    /// Get a dispute by job ID.
    #[must_use]
    pub fn get_dispute(&self, job_id: &[u8; 32]) -> Option<&DisputeResolution> {
        self.disputes.get(job_id)
    }

    // -----------------------------------------------------------------------
    // Job creation
    // -----------------------------------------------------------------------

    /// Post a new job to the marketplace. Deposits budget into escrow.
    ///
    /// # Errors
    ///
    /// Returns an error if a job with the same ID already exists.
    pub fn create_job(&mut self, posting: &SporeJobPosting) -> Result<(), MarketplaceError> {
        if self.jobs.contains_key(&posting.job_id) {
            return Err(MarketplaceError::DuplicateJob);
        }

        let job = MarketplaceJob {
            job_id: posting.job_id,
            state: JobState::Posted,
            poster_passport_id: posting.poster_passport_id,
            assigned_agent: None,
            budget: posting.budget,
            deadline_block: posting.deadline_block,
            hiring_model: posting.hiring_model.clone(),
            min_reputation: posting.min_reputation,
            min_tier: posting.min_tier,
            domain: posting.domain.clone(),
            required_capabilities: posting.required_capabilities,
            result_hash: None,
            quality_score: None,
            posted_at_block: self.current_block,
            payment: None,
        };

        // Deposit into escrow.
        let escrow = EscrowEntry {
            job_id: posting.job_id,
            depositor: posting.poster_passport_id,
            amount: posting.budget,
            recipient: None,
            released: false,
            disputed: false,
        };

        self.jobs.insert(posting.job_id, job);
        self.escrow.insert(posting.job_id, escrow);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Bid submission (for auction model)
    // -----------------------------------------------------------------------

    /// Submit a bid for a posted job (used by BlindAuction hiring model).
    ///
    /// # Errors
    ///
    /// Returns an error if the job doesn't exist or isn't in Posted state.
    pub fn submit_bid(
        &mut self,
        job_id: &[u8; 32],
        bid: SparrowBid,
    ) -> Result<(), MarketplaceError> {
        let job = self.jobs.get(job_id).ok_or(MarketplaceError::NotFound)?;

        if job.state != JobState::Posted {
            return Err(MarketplaceError::InvalidState {
                expected: "Posted".to_string(),
                actual: format!("{:?}", job.state),
            });
        }

        self.bids.entry(*job_id).or_default().push(bid);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Assignment (3 hiring models)
    // -----------------------------------------------------------------------

    /// Assign a job using the RandomVRF model (power-of-two-choices).
    ///
    /// Picks two candidates from the pool and selects the one with higher
    /// reputation. The payment equals the full budget.
    ///
    /// # Errors
    ///
    /// Returns an error if the job doesn't exist, isn't Posted, or the
    /// pool is empty.
    pub fn assign_random_vrf(
        &mut self,
        job_id: &[u8; 32],
        pool: &[SparrowBid],
    ) -> Result<AssignmentResult, MarketplaceError> {
        let job = self.jobs.get(job_id).ok_or(MarketplaceError::NotFound)?;
        if job.state != JobState::Posted {
            return Err(MarketplaceError::InvalidState {
                expected: "Posted".to_string(),
                actual: format!("{:?}", job.state),
            });
        }
        if pool.is_empty() {
            return Err(MarketplaceError::EmptyPool);
        }

        let bounty = self.job_to_bounty(job);
        let decision = crate::identity_economy_markets::dispatch_random_vrf(pool, &bounty)
            .ok_or(MarketplaceError::EmptyPool)?;

        self.finalize_assignment(job_id, decision.winner, decision.payment)
    }

    /// Assign a job using the BlindAuction model (Vickrey second-price).
    ///
    /// Uses previously submitted bids. The winner pays the second-best
    /// reputation-adjusted score (truthful bidding).
    ///
    /// # Errors
    ///
    /// Returns an error if the job doesn't exist, isn't Posted, or there
    /// are no bids.
    pub fn assign_blind_auction(
        &mut self,
        job_id: &[u8; 32],
    ) -> Result<AssignmentResult, MarketplaceError> {
        let job = self.jobs.get(job_id).ok_or(MarketplaceError::NotFound)?;
        if job.state != JobState::Posted {
            return Err(MarketplaceError::InvalidState {
                expected: "Posted".to_string(),
                actual: format!("{:?}", job.state),
            });
        }

        let bids = self.bids.get(job_id).ok_or(MarketplaceError::EmptyPool)?;
        if bids.is_empty() {
            return Err(MarketplaceError::EmptyPool);
        }

        let bounty = self.job_to_bounty(job);
        let decision = crate::identity_economy_markets::dispatch_blind_auction(bids, &bounty)
            .ok_or(MarketplaceError::EmptyPool)?;

        self.finalize_assignment(job_id, decision.winner, decision.payment)
    }

    /// Assign a job using DirectHire model.
    ///
    /// The poster specifies the target agent directly. A 1.5x premium
    /// is applied. Restricted to Protocol/Sovereign tier agents.
    ///
    /// # Errors
    ///
    /// Returns an error if the job doesn't exist, isn't Posted, or the
    /// agent tier is below Sovereign.
    pub fn assign_direct_hire(
        &mut self,
        job_id: &[u8; 32],
        target_passport_id: u256,
        agent_tier: PassportTier,
        repeat_count: u32,
    ) -> Result<AssignmentResult, MarketplaceError> {
        let job = self.jobs.get(job_id).ok_or(MarketplaceError::NotFound)?;
        if job.state != JobState::Posted {
            return Err(MarketplaceError::InvalidState {
                expected: "Posted".to_string(),
                actual: format!("{:?}", job.state),
            });
        }

        // Direct hire restricted to Protocol/Sovereign tier
        if !agent_tier.has_privilege(PassportTier::Sovereign) {
            return Err(MarketplaceError::InsufficientTier {
                required: PassportTier::Sovereign,
                actual: agent_tier,
            });
        }

        let base_fee = job.budget as f64 * self.config.direct_hire_premium;
        let decision = crate::identity_economy_markets::dispatch_direct_hire(
            target_passport_id,
            base_fee,
            repeat_count,
        );

        self.finalize_assignment(job_id, decision.winner, decision.payment)
    }

    // -----------------------------------------------------------------------
    // Job progression
    // -----------------------------------------------------------------------

    /// Mark a job as in-progress (agent starts work).
    ///
    /// # Errors
    ///
    /// Returns an error if the job doesn't exist or isn't Assigned.
    pub fn start_work(&mut self, job_id: &[u8; 32]) -> Result<(), MarketplaceError> {
        let job = self.jobs.get_mut(job_id).ok_or(MarketplaceError::NotFound)?;
        if job.state != JobState::Assigned {
            return Err(MarketplaceError::InvalidState {
                expected: "Assigned".to_string(),
                actual: format!("{:?}", job.state),
            });
        }
        job.state = JobState::InProgress;
        Ok(())
    }

    /// Submit a result for a job.
    ///
    /// # Errors
    ///
    /// Returns an error if the job isn't InProgress.
    pub fn submit_result(
        &mut self,
        job_id: &[u8; 32],
        result_hash: [u8; 32],
        quality_score: f64,
    ) -> Result<(), MarketplaceError> {
        let job = self.jobs.get_mut(job_id).ok_or(MarketplaceError::NotFound)?;
        if job.state != JobState::InProgress {
            return Err(MarketplaceError::InvalidState {
                expected: "InProgress".to_string(),
                actual: format!("{:?}", job.state),
            });
        }
        job.result_hash = Some(result_hash);
        job.quality_score = Some(quality_score);
        job.state = JobState::Submitted;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Settlement
    // -----------------------------------------------------------------------

    /// Settle a job: release escrow to the assigned agent.
    ///
    /// # Errors
    ///
    /// Returns an error if the job isn't Submitted.
    pub fn settle_job(&mut self, job_id: &[u8; 32]) -> Result<SettlementResult, MarketplaceError> {
        let job = self.jobs.get(job_id).ok_or(MarketplaceError::NotFound)?;
        if job.state != JobState::Submitted {
            return Err(MarketplaceError::InvalidState {
                expected: "Submitted".to_string(),
                actual: format!("{:?}", job.state),
            });
        }

        let agent = job
            .assigned_agent
            .ok_or(MarketplaceError::NoAssignedAgent)?;
        let payment = job.payment.unwrap_or(job.budget as f64);
        let platform_fee = (payment * self.config.platform_fee_fraction) as u256;
        let agent_payment = (payment as u256).saturating_sub(platform_fee);

        // Release escrow
        if let Some(escrow) = self.escrow.get_mut(job_id) {
            escrow.released = true;
        }

        let job = self.jobs.get_mut(job_id).unwrap();
        job.state = JobState::Settled;

        Ok(SettlementResult {
            agent_payment,
            platform_fee,
            agent_passport_id: agent,
            quality_score: job.quality_score.unwrap_or(0.0),
        })
    }

    // -----------------------------------------------------------------------
    // Expiration
    // -----------------------------------------------------------------------

    /// Expire a job past its deadline. Refunds escrow to the poster.
    ///
    /// # Errors
    ///
    /// Returns an error if the job doesn't exist or deadline hasn't passed.
    pub fn expire_job(&mut self, job_id: &[u8; 32]) -> Result<ExpirationResult, MarketplaceError> {
        let job = self.jobs.get(job_id).ok_or(MarketplaceError::NotFound)?;

        if self.current_block < job.deadline_block {
            return Err(MarketplaceError::DeadlineNotReached {
                current: self.current_block,
                deadline: job.deadline_block,
            });
        }

        if job.state == JobState::Settled || job.state == JobState::Expired {
            return Err(MarketplaceError::InvalidState {
                expected: "not Settled/Expired".to_string(),
                actual: format!("{:?}", job.state),
            });
        }

        let refund = job.budget;
        let poster = job.poster_passport_id;

        // Refund escrow
        if let Some(escrow) = self.escrow.get_mut(job_id) {
            escrow.released = true;
        }

        let job = self.jobs.get_mut(job_id).unwrap();
        job.state = JobState::Expired;

        Ok(ExpirationResult {
            refund,
            poster_passport_id: poster,
        })
    }

    // -----------------------------------------------------------------------
    // Disputes (4-level escalation)
    // -----------------------------------------------------------------------

    /// Open a dispute on a submitted job.
    ///
    /// Starts at BondEscalation round 1. The escrow is locked until
    /// the dispute resolves.
    ///
    /// # Errors
    ///
    /// Returns an error if the job isn't Submitted.
    pub fn open_dispute(
        &mut self,
        job_id: &[u8; 32],
        challenger: u256,
        challenger_bond: u256,
    ) -> Result<(), MarketplaceError> {
        let job = self.jobs.get_mut(job_id).ok_or(MarketplaceError::NotFound)?;
        if job.state != JobState::Submitted {
            return Err(MarketplaceError::InvalidState {
                expected: "Submitted".to_string(),
                actual: format!("{:?}", job.state),
            });
        }

        let defender = job
            .assigned_agent
            .ok_or(MarketplaceError::NoAssignedAgent)?;

        let dispute = DisputeResolution {
            entry_hash: job.result_hash.unwrap_or([0; 32]),
            challenger,
            defender,
            current_level: DisputeLevel::BondEscalation { round: 1 },
            challenger_bond,
            defender_bond: 0,
            jury: None,
            deadline_block: self.current_block + self.config.dispute_challenge_window,
        };

        if let Some(escrow) = self.escrow.get_mut(job_id) {
            escrow.disputed = true;
        }

        job.state = JobState::Disputed;
        self.disputes.insert(*job_id, dispute);

        Ok(())
    }

    /// Escalate a dispute to the next level.
    ///
    /// # Errors
    ///
    /// Returns an error if no active dispute exists.
    pub fn escalate_dispute(
        &mut self,
        job_id: &[u8; 32],
    ) -> Result<&DisputeLevel, MarketplaceError> {
        let dispute = self
            .disputes
            .get_mut(job_id)
            .ok_or(MarketplaceError::NoDispute)?;

        dispute.current_level = match &dispute.current_level {
            DisputeLevel::BondEscalation { round } => {
                if *round >= 3 {
                    DisputeLevel::PeerJury {
                        votes_for: 0,
                        votes_against: 0,
                    }
                } else {
                    DisputeLevel::BondEscalation { round: round + 1 }
                }
            }
            DisputeLevel::PeerJury { .. } => DisputeLevel::GovernanceVote {
                proposal_id: *job_id,
            },
            level => {
                return Err(MarketplaceError::InvalidState {
                    expected: "escalatable dispute level".to_string(),
                    actual: format!("{:?}", level),
                });
            }
        };

        dispute.deadline_block = self.current_block + self.config.dispute_challenge_window;
        Ok(&dispute.current_level)
    }

    /// Resolve a dispute with a final outcome.
    ///
    /// # Errors
    ///
    /// Returns an error if no active dispute exists.
    pub fn resolve_dispute(
        &mut self,
        job_id: &[u8; 32],
        winner: u256,
        outcome: DisputeOutcome,
    ) -> Result<DisputeSettlement, MarketplaceError> {
        let dispute = self
            .disputes
            .get_mut(job_id)
            .ok_or(MarketplaceError::NoDispute)?;

        dispute.current_level = DisputeLevel::Resolved {
            winner,
            outcome: outcome.clone(),
        };

        let job = self.jobs.get(job_id).ok_or(MarketplaceError::NotFound)?;

        let (escrow_to, refund_amount) = match &outcome {
            DisputeOutcome::EntryUpheld => {
                // Agent wins: release to agent.
                (job.assigned_agent.unwrap_or(0), job.budget)
            }
            DisputeOutcome::EntryRemoved | DisputeOutcome::EntryAmended { .. } => {
                // Challenger wins: refund to poster.
                (job.poster_passport_id, job.budget)
            }
        };

        if let Some(escrow) = self.escrow.get_mut(job_id) {
            escrow.released = true;
            escrow.disputed = false;
        }

        Ok(DisputeSettlement {
            winner,
            outcome,
            escrow_recipient: escrow_to,
            amount: refund_amount,
            challenger_bond_returned: dispute.challenger_bond,
            defender_bond_returned: dispute.defender_bond,
        })
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Convert a job to a BountySpec for dispatch functions.
    fn job_to_bounty(&self, job: &MarketplaceJob) -> BountySpec {
        BountySpec {
            job_id: job.job_id,
            title: String::new(),
            description: String::new(),
            required_capabilities: job.required_capabilities,
            required_domain: job.domain.clone(),
            min_reputation: job.min_reputation,
            max_budget_usdc: job.budget as u64,
            deadline: job.deadline_block,
            hiring_model: job.hiring_model.clone(),
            evaluation_criteria: Vec::new(),
            quality_threshold: 0.0,
            poster_passport_id: job.poster_passport_id,
        }
    }

    /// Finalize an assignment: transition job state and update escrow.
    fn finalize_assignment(
        &mut self,
        job_id: &[u8; 32],
        winner: u256,
        payment: f64,
    ) -> Result<AssignmentResult, MarketplaceError> {
        let job = self.jobs.get_mut(job_id).ok_or(MarketplaceError::NotFound)?;
        job.state = JobState::Assigned;
        job.assigned_agent = Some(winner);
        job.payment = Some(payment);

        if let Some(escrow) = self.escrow.get_mut(job_id) {
            escrow.recipient = Some(winner);
        }

        Ok(AssignmentResult {
            agent_passport_id: winner,
            payment,
            hiring_model: job.hiring_model.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Result of assigning an agent to a job.
#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentResult {
    /// Assigned agent passport ID.
    pub agent_passport_id: u256,
    /// Determined payment.
    pub payment: f64,
    /// Hiring model used.
    pub hiring_model: HiringModel,
}

/// Result of settling a job.
#[derive(Debug, Clone, PartialEq)]
pub struct SettlementResult {
    /// Payment to the agent (after platform fee).
    pub agent_payment: u256,
    /// Platform fee collected.
    pub platform_fee: u256,
    /// Agent who received the payment.
    pub agent_passport_id: u256,
    /// Quality score of the submission.
    pub quality_score: f64,
}

/// Result of expiring a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpirationResult {
    /// Refunded amount.
    pub refund: u256,
    /// Poster who receives the refund.
    pub poster_passport_id: u256,
}

/// Result of resolving a dispute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisputeSettlement {
    /// Winner of the dispute.
    pub winner: u256,
    /// Outcome category.
    pub outcome: DisputeOutcome,
    /// Recipient of the escrow.
    pub escrow_recipient: u256,
    /// Amount transferred.
    pub amount: u256,
    /// Challenger bond returned.
    pub challenger_bond_returned: u256,
    /// Defender bond returned.
    pub defender_bond_returned: u256,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from the marketplace.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MarketplaceError {
    /// Job not found.
    #[error("job not found")]
    NotFound,
    /// Duplicate job ID.
    #[error("job with this ID already exists")]
    DuplicateJob,
    /// Invalid state transition.
    #[error("invalid state: expected {expected}, got {actual}")]
    InvalidState {
        /// Expected state.
        expected: String,
        /// Actual state.
        actual: String,
    },
    /// Empty candidate pool.
    #[error("no candidates in the pool")]
    EmptyPool,
    /// No agent assigned to this job.
    #[error("no agent assigned to this job")]
    NoAssignedAgent,
    /// Deadline not reached.
    #[error("deadline not reached: current block {current}, deadline {deadline}")]
    DeadlineNotReached {
        /// Current block.
        current: u64,
        /// Deadline block.
        deadline: u64,
    },
    /// Insufficient passport tier for direct hire.
    #[error("insufficient tier: required {required:?}, got {actual:?}")]
    InsufficientTier {
        /// Required tier.
        required: PassportTier,
        /// Actual tier.
        actual: PassportTier,
    },
    /// No active dispute.
    #[error("no active dispute for this job")]
    NoDispute,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::identity_economy_identity::Signature;
    use crate::identity_economy_markets::SparrowBid;

    fn test_posting() -> SporeJobPosting {
        SporeJobPosting {
            job_id: [1; 32],
            domain: "coding".to_string(),
            required_capabilities: 0b11,
            budget: 10_000,
            deadline_block: 500,
            hiring_model: HiringModel::RandomVRF,
            min_reputation: 0.5,
            min_tier: PassportTier::Worker,
            description_cid: "QmTest".to_string(),
            poster_passport_id: 100,
            direct_hire_target: None,
            max_agents: 1,
        }
    }

    fn test_bid(passport: u256, price: u64, rep: f64) -> SparrowBid {
        SparrowBid {
            bidder_passport_id: passport,
            bounty_id: [1; 32],
            price_usdc: price,
            estimated_time: 3600,
            capability_proof: 0b11,
            reputation_snapshot: rep,
            signature: Signature::default(),
        }
    }

    #[test]
    fn create_job_deposits_escrow() {
        let mut market = Marketplace::new(MarketplaceConfig::default());
        let posting = test_posting();

        market.create_job(&posting).unwrap();

        let job = market.get_job(&posting.job_id).unwrap();
        assert_eq!(job.state, JobState::Posted);
        assert_eq!(job.budget, 10_000);

        let escrow = market.get_escrow(&posting.job_id).unwrap();
        assert_eq!(escrow.amount, 10_000);
        assert!(!escrow.released);
    }

    #[test]
    fn duplicate_job_rejected() {
        let mut market = Marketplace::new(MarketplaceConfig::default());
        let posting = test_posting();

        market.create_job(&posting).unwrap();
        let err = market.create_job(&posting).unwrap_err();
        assert!(matches!(err, MarketplaceError::DuplicateJob));
    }

    #[test]
    fn random_vrf_assignment() {
        let mut market = Marketplace::new(MarketplaceConfig::default());
        let posting = test_posting();
        market.create_job(&posting).unwrap();

        let pool = vec![
            test_bid(10, 100, 0.9),
            test_bid(20, 100, 0.7),
            test_bid(30, 100, 0.8),
        ];

        let result = market.assign_random_vrf(&posting.job_id, &pool).unwrap();
        assert!(pool.iter().any(|b| b.bidder_passport_id == result.agent_passport_id));
        assert!(matches!(result.hiring_model, HiringModel::RandomVRF));

        let job = market.get_job(&posting.job_id).unwrap();
        assert_eq!(job.state, JobState::Assigned);
        assert!(job.assigned_agent.is_some());
    }

    #[test]
    fn blind_auction_assignment() {
        let mut market = Marketplace::new(MarketplaceConfig::default());
        let posting = test_posting();
        market.create_job(&posting).unwrap();

        market
            .submit_bid(&posting.job_id, test_bid(10, 100, 0.9))
            .unwrap();
        market
            .submit_bid(&posting.job_id, test_bid(20, 120, 0.8))
            .unwrap();

        let result = market.assign_blind_auction(&posting.job_id).unwrap();
        assert_eq!(result.agent_passport_id, 10); // Higher rep wins
        assert!(result.payment > 0.0);
    }

    #[test]
    fn direct_hire_restricted_to_high_tier() {
        let mut market = Marketplace::new(MarketplaceConfig::default());
        let posting = test_posting();
        market.create_job(&posting).unwrap();

        // Worker tier: rejected
        let err = market
            .assign_direct_hire(&posting.job_id, 42, PassportTier::Worker, 0)
            .unwrap_err();
        assert!(matches!(err, MarketplaceError::InsufficientTier { .. }));

        // Sovereign tier: accepted
        let result = market
            .assign_direct_hire(&posting.job_id, 42, PassportTier::Sovereign, 0)
            .unwrap();
        assert_eq!(result.agent_passport_id, 42);
        assert!(result.payment > 10_000.0); // 1.5x premium
    }

    #[test]
    fn full_lifecycle_post_to_settled() {
        let mut market = Marketplace::new(MarketplaceConfig::default());
        let posting = test_posting();
        market.create_job(&posting).unwrap();

        // Assign
        let pool = vec![test_bid(42, 100, 0.9)];
        market.assign_random_vrf(&posting.job_id, &pool).unwrap();

        // Start work
        market.start_work(&posting.job_id).unwrap();
        assert_eq!(
            market.get_job(&posting.job_id).unwrap().state,
            JobState::InProgress
        );

        // Submit result
        market
            .submit_result(&posting.job_id, [2; 32], 0.85)
            .unwrap();
        assert_eq!(
            market.get_job(&posting.job_id).unwrap().state,
            JobState::Submitted
        );

        // Settle
        let settlement = market.settle_job(&posting.job_id).unwrap();
        assert_eq!(settlement.agent_passport_id, 42);
        assert!(settlement.agent_payment > 0);
        assert_eq!(
            market.get_job(&posting.job_id).unwrap().state,
            JobState::Settled
        );
        assert!(market.get_escrow(&posting.job_id).unwrap().released);
    }

    #[test]
    fn expire_job_refunds_escrow() {
        let mut market = Marketplace::new(MarketplaceConfig::default());
        let posting = test_posting();
        market.create_job(&posting).unwrap();

        market.set_block(600); // Past deadline of 500
        let result = market.expire_job(&posting.job_id).unwrap();
        assert_eq!(result.refund, 10_000);
        assert_eq!(result.poster_passport_id, 100);
        assert_eq!(
            market.get_job(&posting.job_id).unwrap().state,
            JobState::Expired
        );
    }

    #[test]
    fn expire_before_deadline_fails() {
        let mut market = Marketplace::new(MarketplaceConfig::default());
        let posting = test_posting();
        market.create_job(&posting).unwrap();

        market.set_block(100);
        let err = market.expire_job(&posting.job_id).unwrap_err();
        assert!(matches!(err, MarketplaceError::DeadlineNotReached { .. }));
    }

    #[test]
    fn dispute_escalation_through_4_levels() {
        let mut market = Marketplace::new(MarketplaceConfig::default());
        let posting = test_posting();
        market.create_job(&posting).unwrap();

        let pool = vec![test_bid(42, 100, 0.9)];
        market.assign_random_vrf(&posting.job_id, &pool).unwrap();
        market.start_work(&posting.job_id).unwrap();
        market
            .submit_result(&posting.job_id, [2; 32], 0.85)
            .unwrap();

        // Open dispute at BondEscalation round 1
        market
            .open_dispute(&posting.job_id, 99, 500)
            .unwrap();
        let dispute = market.get_dispute(&posting.job_id).unwrap();
        assert!(matches!(
            dispute.current_level,
            DisputeLevel::BondEscalation { round: 1 }
        ));

        // Escalate to round 2
        let level = market.escalate_dispute(&posting.job_id).unwrap();
        assert!(matches!(level, DisputeLevel::BondEscalation { round: 2 }));

        // Escalate to round 3
        market.escalate_dispute(&posting.job_id).unwrap();

        // Escalate to PeerJury
        let level = market.escalate_dispute(&posting.job_id).unwrap();
        assert!(matches!(level, DisputeLevel::PeerJury { .. }));

        // Escalate to GovernanceVote
        let level = market.escalate_dispute(&posting.job_id).unwrap();
        assert!(matches!(level, DisputeLevel::GovernanceVote { .. }));
    }

    #[test]
    fn dispute_resolution_upholds_entry() {
        let mut market = Marketplace::new(MarketplaceConfig::default());
        let posting = test_posting();
        market.create_job(&posting).unwrap();

        let pool = vec![test_bid(42, 100, 0.9)];
        market.assign_random_vrf(&posting.job_id, &pool).unwrap();
        market.start_work(&posting.job_id).unwrap();
        market
            .submit_result(&posting.job_id, [2; 32], 0.85)
            .unwrap();
        market
            .open_dispute(&posting.job_id, 99, 500)
            .unwrap();

        // Resolve in agent's favor
        let settlement = market
            .resolve_dispute(&posting.job_id, 42, DisputeOutcome::EntryUpheld)
            .unwrap();
        assert_eq!(settlement.winner, 42);
        assert_eq!(settlement.escrow_recipient, 42);
        assert_eq!(settlement.amount, 10_000);
    }

    #[test]
    fn dispute_resolution_removes_entry() {
        let mut market = Marketplace::new(MarketplaceConfig::default());
        let posting = test_posting();
        market.create_job(&posting).unwrap();

        let pool = vec![test_bid(42, 100, 0.9)];
        market.assign_random_vrf(&posting.job_id, &pool).unwrap();
        market.start_work(&posting.job_id).unwrap();
        market
            .submit_result(&posting.job_id, [2; 32], 0.85)
            .unwrap();
        market
            .open_dispute(&posting.job_id, 99, 500)
            .unwrap();

        // Resolve in challenger's favor
        let settlement = market
            .resolve_dispute(&posting.job_id, 99, DisputeOutcome::EntryRemoved)
            .unwrap();
        assert_eq!(settlement.winner, 99);
        assert_eq!(settlement.escrow_recipient, 100); // Poster gets refund
    }
}
