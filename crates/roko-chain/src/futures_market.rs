//! Knowledge futures market (CHAIN-11).
//!
//! Allows agents to commit to producing specific knowledge by a deadline
//! with staked collateral. The lifecycle state machine:
//! OPEN -> COMMITTED -> SUBMITTED -> FULFILLED / EXPIRED

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::phase2::{
    DemandDeposit, DemandPool, FutureState, KnowledgeFuture, KnowledgeSpec, u256,
};

/// Configuration for the knowledge futures market.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FuturesMarketConfig {
    /// Minimum stake required to commit to a future.
    pub min_stake: u256,
    /// Early withdrawal penalty fraction (default 0.10 = 10%).
    pub early_withdrawal_penalty: f64,
    /// Minimum quality threshold for fulfillment.
    pub min_quality: f64,
    /// Maximum HDC similarity threshold for redundancy rejection.
    pub max_hdc_similarity: f64,
}

impl Default for FuturesMarketConfig {
    fn default() -> Self {
        Self {
            min_stake: 100,
            early_withdrawal_penalty: 0.10,
            min_quality: 0.5,
            max_hdc_similarity: 0.95,
        }
    }
}

/// A submission against a knowledge future.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FutureSubmission {
    /// Hash of the submitted knowledge entry.
    pub entry_hash: [u8; 32],
    /// Quality score assessed at submission time.
    pub quality_score: f64,
    /// HDC similarity to the target vector, if applicable.
    pub hdc_similarity: Option<f64>,
    /// Block number of submission.
    pub submitted_at_block: u64,
}

/// The knowledge futures market.
#[derive(Debug, Clone, Default)]
pub struct FuturesMarket {
    /// Market configuration.
    pub config: FuturesMarketConfig,
    /// Active futures by ID.
    futures: HashMap<[u8; 32], KnowledgeFuture>,
    /// Demand pools by specification domain+topic key.
    demand_pools: HashMap<String, DemandPool>,
    /// Pending submissions awaiting validation.
    submissions: HashMap<[u8; 32], FutureSubmission>,
    /// Current block number (set externally).
    current_block: u64,
}

impl FuturesMarket {
    /// Create a new futures market.
    #[must_use]
    pub fn new(config: FuturesMarketConfig) -> Self {
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

    /// Create a new knowledge future.
    ///
    /// Returns the future ID (derived from spec).
    pub fn create_future(
        &mut self,
        spec: KnowledgeSpec,
        reward: u256,
        deadline_block: u64,
    ) -> [u8; 32] {
        let future_id = self.derive_future_id(&spec, deadline_block);

        let future = KnowledgeFuture {
            future_id,
            specification: spec,
            producer_passport_id: 0,
            stake: 0,
            deadline_block,
            reward,
            state: FutureState::Open,
        };

        self.futures.insert(future_id, future);
        future_id
    }

    /// Commit to producing a future by staking collateral.
    ///
    /// # Errors
    ///
    /// Returns an error if the future doesn't exist, is not open,
    /// or the stake is below minimum.
    pub fn commit(
        &mut self,
        future_id: &[u8; 32],
        producer_passport_id: u256,
        stake: u256,
    ) -> Result<(), FuturesError> {
        let future = self
            .futures
            .get_mut(future_id)
            .ok_or(FuturesError::NotFound)?;

        if future.state != FutureState::Open {
            return Err(FuturesError::InvalidState {
                expected: "Open".to_string(),
                actual: format!("{:?}", future.state),
            });
        }

        if stake < self.config.min_stake {
            return Err(FuturesError::InsufficientStake {
                provided: stake,
                required: self.config.min_stake,
            });
        }

        future.producer_passport_id = producer_passport_id;
        future.stake = stake;
        future.state = FutureState::Submitted; // Use Submitted as "Committed" equivalent

        Ok(())
    }

    /// Submit a result for a committed future.
    ///
    /// # Errors
    ///
    /// Returns an error if the future doesn't exist or is not in the
    /// committed (Submitted) state.
    pub fn submit(
        &mut self,
        future_id: &[u8; 32],
        entry_hash: [u8; 32],
        quality_score: f64,
        hdc_similarity: Option<f64>,
    ) -> Result<(), FuturesError> {
        let future = self
            .futures
            .get(future_id)
            .ok_or(FuturesError::NotFound)?;

        if future.state != FutureState::Submitted {
            return Err(FuturesError::InvalidState {
                expected: "Submitted (committed)".to_string(),
                actual: format!("{:?}", future.state),
            });
        }

        let submission = FutureSubmission {
            entry_hash,
            quality_score,
            hdc_similarity,
            submitted_at_block: self.current_block,
        };

        self.submissions.insert(*future_id, submission);
        Ok(())
    }

    /// Attempt to fulfill a future by validating its submission.
    ///
    /// Returns the reward and stake if successful.
    ///
    /// # Errors
    ///
    /// Returns an error if no submission exists, quality is too low,
    /// or HDC similarity doesn't match the target.
    pub fn fulfill(&mut self, future_id: &[u8; 32]) -> Result<FulfillmentResult, FuturesError> {
        let submission = self
            .submissions
            .get(future_id)
            .ok_or(FuturesError::NoSubmission)?
            .clone();

        let future = self
            .futures
            .get(future_id)
            .ok_or(FuturesError::NotFound)?;

        // Quality check
        let min_quality = if future.specification.min_quality > 0.0 {
            future.specification.min_quality
        } else {
            self.config.min_quality
        };

        if submission.quality_score < min_quality {
            return Err(FuturesError::QualityBelowThreshold {
                quality: submission.quality_score,
                threshold: min_quality,
            });
        }

        // HDC similarity check if target is specified
        if future.specification.target_hdc.is_some() {
            if let Some(sim) = submission.hdc_similarity {
                if sim < min_quality {
                    return Err(FuturesError::QualityBelowThreshold {
                        quality: sim,
                        threshold: min_quality,
                    });
                }
            }
        }

        let future = self.futures.get_mut(future_id).unwrap();
        let stake = future.stake;
        let reward = future.reward;
        future.state = FutureState::Fulfilled;

        Ok(FulfillmentResult {
            stake_returned: stake,
            reward_released: reward,
            quality_score: submission.quality_score,
        })
    }

    /// Expire a future past its deadline. Slashes the stake and returns
    /// the reward to the demand pool.
    ///
    /// # Errors
    ///
    /// Returns an error if the future doesn't exist or deadline hasn't passed.
    pub fn expire(&mut self, future_id: &[u8; 32]) -> Result<ExpirationResult, FuturesError> {
        let future = self
            .futures
            .get(future_id)
            .ok_or(FuturesError::NotFound)?;

        if self.current_block < future.deadline_block {
            return Err(FuturesError::DeadlineNotReached {
                current: self.current_block,
                deadline: future.deadline_block,
            });
        }

        if future.state == FutureState::Fulfilled {
            return Err(FuturesError::InvalidState {
                expected: "not Fulfilled".to_string(),
                actual: "Fulfilled".to_string(),
            });
        }

        let future = self.futures.get_mut(future_id).unwrap();
        let stake_slashed = future.stake;
        let reward_returned = future.reward;
        future.state = FutureState::Expired;

        Ok(ExpirationResult {
            stake_slashed,
            reward_returned,
        })
    }

    /// Deposit demand into a pool for a knowledge specification.
    pub fn deposit_demand(
        &mut self,
        spec: KnowledgeSpec,
        depositor: u256,
        amount: u256,
    ) -> String {
        let pool_key = format!("{}:{}", spec.domain, spec.topic);
        let pool = self
            .demand_pools
            .entry(pool_key.clone())
            .or_insert_with(|| DemandPool {
                spec,
                total_demand: 0,
                deposits: Vec::new(),
                committed_producer: None,
                created_at_block: self.current_block,
            });

        pool.total_demand += amount;
        pool.deposits.push(DemandDeposit {
            depositor_passport_id: depositor,
            amount,
            deposited_at_block: self.current_block,
        });

        pool_key
    }

    /// Withdraw from a demand pool with early withdrawal penalty.
    ///
    /// # Errors
    ///
    /// Returns an error if the pool or deposit doesn't exist.
    pub fn withdraw_demand(
        &mut self,
        pool_key: &str,
        depositor: u256,
    ) -> Result<WithdrawalResult, FuturesError> {
        let pool = self
            .demand_pools
            .get_mut(pool_key)
            .ok_or(FuturesError::NotFound)?;

        let deposit_idx = pool
            .deposits
            .iter()
            .position(|d| d.depositor_passport_id == depositor)
            .ok_or(FuturesError::NoDeposit)?;

        let deposit = pool.deposits.remove(deposit_idx);
        let penalty = (deposit.amount as f64 * self.config.early_withdrawal_penalty) as u256;
        let refund = deposit.amount.saturating_sub(penalty);
        pool.total_demand = pool.total_demand.saturating_sub(deposit.amount);

        Ok(WithdrawalResult { refund, penalty })
    }

    /// Get the total demand for a pool.
    #[must_use]
    pub fn pool_demand(&self, pool_key: &str) -> u256 {
        self.demand_pools
            .get(pool_key)
            .map_or(0, |p| p.total_demand)
    }

    /// Get a future by ID.
    #[must_use]
    pub fn get_future(&self, future_id: &[u8; 32]) -> Option<&KnowledgeFuture> {
        self.futures.get(future_id)
    }

    /// Number of active futures.
    #[must_use]
    pub fn future_count(&self) -> usize {
        self.futures.len()
    }

    /// Derive a deterministic future ID from spec and deadline.
    fn derive_future_id(&self, spec: &KnowledgeSpec, deadline: u64) -> [u8; 32] {
        let mut id = [0u8; 32];
        let key = format!("{}:{}:{}", spec.domain, spec.topic, deadline);
        let bytes = key.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            id[i % 32] ^= b;
        }
        id
    }
}

/// Result of a successful fulfillment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FulfillmentResult {
    /// Stake returned to the producer.
    pub stake_returned: u256,
    /// Reward released from the demand pool.
    pub reward_released: u256,
    /// Quality score of the submission.
    pub quality_score: f64,
}

/// Result of a future expiration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpirationResult {
    /// Stake slashed from the producer.
    pub stake_slashed: u256,
    /// Reward returned to the demand pool.
    pub reward_returned: u256,
}

/// Result of a demand withdrawal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WithdrawalResult {
    /// Amount refunded after penalty.
    pub refund: u256,
    /// Penalty retained.
    pub penalty: u256,
}

/// Errors from the futures market.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FuturesError {
    /// Future not found.
    #[error("future not found")]
    NotFound,
    /// Invalid state transition.
    #[error("invalid state: expected {expected}, got {actual}")]
    InvalidState {
        /// Expected state.
        expected: String,
        /// Actual state.
        actual: String,
    },
    /// Insufficient stake.
    #[error("insufficient stake: provided {provided}, required {required}")]
    InsufficientStake {
        /// Stake provided.
        provided: u256,
        /// Stake required.
        required: u256,
    },
    /// No submission exists.
    #[error("no submission found for this future")]
    NoSubmission,
    /// Quality below threshold.
    #[error("quality {quality:.3} below threshold {threshold:.3}")]
    QualityBelowThreshold {
        /// Quality score.
        quality: f64,
        /// Required threshold.
        threshold: f64,
    },
    /// Deadline not yet reached.
    #[error("deadline not reached: current block {current}, deadline {deadline}")]
    DeadlineNotReached {
        /// Current block.
        current: u64,
        /// Required deadline.
        deadline: u64,
    },
    /// No demand deposit found.
    #[error("no deposit found for this depositor")]
    NoDeposit,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_spec() -> KnowledgeSpec {
        KnowledgeSpec {
            domain: "rust".to_string(),
            topic: "async-patterns".to_string(),
            min_quality: 0.6,
            target_hdc: None,
            acceptance_criteria: Vec::new(),
        }
    }

    #[test]
    fn create_future_and_retrieve() {
        let mut market = FuturesMarket::new(FuturesMarketConfig::default());
        let id = market.create_future(test_spec(), 1000, 500);

        let future = market.get_future(&id).unwrap();
        assert_eq!(future.state, FutureState::Open);
        assert_eq!(future.reward, 1000);
        assert_eq!(future.deadline_block, 500);
    }

    #[test]
    fn full_lifecycle_open_to_fulfilled() {
        let mut market = FuturesMarket::new(FuturesMarketConfig::default());
        let id = market.create_future(test_spec(), 1000, 500);

        // Commit
        market.commit(&id, 42, 200).unwrap();
        let future = market.get_future(&id).unwrap();
        assert_eq!(future.producer_passport_id, 42);
        assert_eq!(future.stake, 200);

        // Submit
        market.submit(&id, [1u8; 32], 0.85, None).unwrap();

        // Fulfill
        let result = market.fulfill(&id).unwrap();
        assert_eq!(result.stake_returned, 200);
        assert_eq!(result.reward_released, 1000);
        assert!((result.quality_score - 0.85).abs() < 1e-10);

        let future = market.get_future(&id).unwrap();
        assert_eq!(future.state, FutureState::Fulfilled);
    }

    #[test]
    fn expire_slashes_stake() {
        let mut market = FuturesMarket::new(FuturesMarketConfig::default());
        let id = market.create_future(test_spec(), 1000, 100);

        market.commit(&id, 42, 200).unwrap();
        market.set_block(200); // Past deadline

        let result = market.expire(&id).unwrap();
        assert_eq!(result.stake_slashed, 200);
        assert_eq!(result.reward_returned, 1000);

        let future = market.get_future(&id).unwrap();
        assert_eq!(future.state, FutureState::Expired);
    }

    #[test]
    fn expire_before_deadline_fails() {
        let mut market = FuturesMarket::new(FuturesMarketConfig::default());
        let id = market.create_future(test_spec(), 1000, 500);
        market.commit(&id, 42, 200).unwrap();
        market.set_block(100);

        let err = market.expire(&id).unwrap_err();
        assert!(matches!(err, FuturesError::DeadlineNotReached { .. }));
    }

    #[test]
    fn commit_with_insufficient_stake_fails() {
        let mut market = FuturesMarket::new(FuturesMarketConfig {
            min_stake: 500,
            ..Default::default()
        });
        let id = market.create_future(test_spec(), 1000, 500);

        let err = market.commit(&id, 42, 100).unwrap_err();
        assert!(matches!(err, FuturesError::InsufficientStake { .. }));
    }

    #[test]
    fn fulfill_rejects_low_quality() {
        let mut market = FuturesMarket::new(FuturesMarketConfig::default());
        let id = market.create_future(test_spec(), 1000, 500);

        market.commit(&id, 42, 200).unwrap();
        market.submit(&id, [1u8; 32], 0.2, None).unwrap(); // Below 0.6 threshold

        let err = market.fulfill(&id).unwrap_err();
        assert!(matches!(err, FuturesError::QualityBelowThreshold { .. }));
    }

    #[test]
    fn demand_pool_deposit_and_withdraw() {
        let mut market = FuturesMarket::new(FuturesMarketConfig::default());

        let pool_key = market.deposit_demand(test_spec(), 1, 1000);
        market.deposit_demand(test_spec(), 2, 500);

        assert_eq!(market.pool_demand(&pool_key), 1500);

        // Withdraw with 10% penalty
        let result = market.withdraw_demand(&pool_key, 1).unwrap();
        assert_eq!(result.penalty, 100);
        assert_eq!(result.refund, 900);
        assert_eq!(market.pool_demand(&pool_key), 500);
    }

    #[test]
    fn withdraw_nonexistent_depositor_fails() {
        let mut market = FuturesMarket::new(FuturesMarketConfig::default());
        let pool_key = market.deposit_demand(test_spec(), 1, 1000);

        let err = market.withdraw_demand(&pool_key, 99).unwrap_err();
        assert!(matches!(err, FuturesError::NoDeposit));
    }

    #[test]
    fn cannot_commit_to_nonexistent_future() {
        let mut market = FuturesMarket::new(FuturesMarketConfig::default());
        let err = market.commit(&[0u8; 32], 42, 200).unwrap_err();
        assert!(matches!(err, FuturesError::NotFound));
    }

    #[test]
    fn cannot_expire_fulfilled_future() {
        let mut market = FuturesMarket::new(FuturesMarketConfig::default());
        let id = market.create_future(test_spec(), 1000, 100);
        market.commit(&id, 42, 200).unwrap();
        market.submit(&id, [1u8; 32], 0.9, None).unwrap();
        market.fulfill(&id).unwrap();

        market.set_block(200);
        let err = market.expire(&id).unwrap_err();
        assert!(matches!(err, FuturesError::InvalidState { .. }));
    }

    #[test]
    fn future_count_tracks_active_futures() {
        let mut market = FuturesMarket::new(FuturesMarketConfig::default());
        assert_eq!(market.future_count(), 0);

        market.create_future(test_spec(), 1000, 500);
        assert_eq!(market.future_count(), 1);

        let mut spec2 = test_spec();
        spec2.topic = "other-topic".to_string();
        market.create_future(spec2, 2000, 600);
        assert_eq!(market.future_count(), 2);
    }
}
