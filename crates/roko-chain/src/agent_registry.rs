//! Agent Registry with soulbound (non-transferable) ERC-721 passports.
//!
//! CHAIN-02: Each agent gets a soulbound NFT passport with capabilities,
//! system prompt hash commitment, tier management, and 24h timelock for
//! prompt updates.
//!
//! Soulbound property: `transfer()` always fails.
//! Ventriloquist defense: system prompt hash committed at registration;
//! updates require 24h timelock; >3 changes in 30 days triggers reputation
//! penalty (-0.05).

use std::collections::HashMap;

use crate::phase2::{Address, AgentPassport, PassportTier, u256};

/// Seconds in 24 hours (timelock duration for prompt updates).
const PROMPT_UPDATE_TIMELOCK_SECS: u64 = 24 * 3600;

/// Seconds in 30 days (window for prompt change rate limiting).
const PROMPT_CHANGE_WINDOW_SECS: u64 = 30 * 24 * 3600;

/// Maximum prompt changes in the window before reputation penalty.
const MAX_PROMPT_CHANGES_IN_WINDOW: usize = 3;

/// KORAI stake thresholds for each passport tier.
const TIER_PROTOCOL_STAKE: u256 = 100_000;
const TIER_SOVEREIGN_STAKE: u256 = 25_000;
const TIER_WORKER_STAKE: u256 = 5_000;

/// 10 capability bits.
pub const CAP_INFERENCE: u64 = 1 << 0;
/// Data transformation capability.
pub const CAP_DATA_TRANSFORM: u64 = 1 << 1;
/// Fine-tuning capability.
pub const CAP_FINE_TUNE: u64 = 1 << 2;
/// Retrieval-augmented generation capability.
pub const CAP_RAG: u64 = 1 << 3;
/// Multi-agent orchestration capability.
pub const CAP_MULTI_AGENT: u64 = 1 << 4;
/// Trading / DeFi capability.
pub const CAP_TRADING: u64 = 1 << 5;
/// Security analysis capability.
pub const CAP_SECURITY: u64 = 1 << 6;
/// Analytics and metrics capability.
pub const CAP_ANALYTICS: u64 = 1 << 7;
/// Knowledge management capability.
pub const CAP_KNOWLEDGE: u64 = 1 << 8;
/// Strategic planning capability.
pub const CAP_STRATEGY: u64 = 1 << 9;

/// A pending system prompt hash update with timelock.
#[derive(Debug, Clone)]
pub struct PendingPromptUpdate {
    /// The new prompt hash being proposed.
    pub new_hash: [u8; 32],
    /// Unix timestamp (seconds) when the update was submitted.
    pub submitted_at: u64,
    /// Unix timestamp (seconds) when the update can be applied.
    pub executable_after: u64,
}

/// Record of a prompt hash change for rate limiting.
#[derive(Debug, Clone)]
struct PromptChangeRecord {
    /// Unix timestamp of the change.
    timestamp: u64,
}

/// Error type for agent registry operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// The passport ID does not exist.
    PassportNotFound(u256),
    /// Transfer attempted on a soulbound token.
    SoulboundTransferDenied,
    /// Caller is not the passport owner.
    NotOwner,
    /// The timelock has not elapsed yet.
    TimelockNotElapsed {
        /// Remaining seconds until the timelock expires.
        remaining_secs: u64,
    },
    /// No pending prompt update exists.
    NoPendingUpdate,
    /// Prompt change rate exceeded (>3 in 30 days).
    PromptChangeRateExceeded {
        /// Number of changes in the window.
        changes_in_window: usize,
    },
    /// Only registry admin can call this function.
    AdminOnly,
}

/// Configuration for the agent registry.
#[derive(Debug, Clone)]
pub struct AgentRegistryConfig {
    /// Address of the registry admin (can mint new passports).
    pub admin: String,
}

/// In-memory Agent Registry implementing soulbound ERC-721 passport management.
#[derive(Debug, Clone)]
pub struct AgentRegistry {
    config: AgentRegistryConfig,
    /// Passports keyed by passport_id.
    passports: HashMap<u256, AgentPassport>,
    /// Pending prompt hash updates keyed by passport_id.
    pending_updates: HashMap<u256, PendingPromptUpdate>,
    /// Prompt change history for rate limiting.
    prompt_changes: HashMap<u256, Vec<PromptChangeRecord>>,
    /// Next passport ID to mint.
    next_id: u256,
}

impl AgentRegistry {
    /// Create a new agent registry with the given admin address.
    pub fn new(admin: impl Into<String>) -> Self {
        Self {
            config: AgentRegistryConfig {
                admin: admin.into(),
            },
            passports: HashMap::new(),
            pending_updates: HashMap::new(),
            prompt_changes: HashMap::new(),
            next_id: 1,
        }
    }

    /// Mint a new soulbound passport. Only callable by the admin.
    ///
    /// Returns the new passport ID on success.
    pub fn mint(
        &mut self,
        caller: &str,
        owner: Address,
        capabilities: u64,
        system_prompt_hash: [u8; 32],
        initial_stake: u256,
    ) -> Result<u256, RegistryError> {
        if caller != self.config.admin {
            return Err(RegistryError::AdminOnly);
        }

        let passport_id = self.next_id;
        self.next_id += 1;

        let tier = tier_from_stake(initial_stake);

        let passport = AgentPassport {
            passport_id,
            owner,
            capability_list: capabilities,
            system_prompt_hash,
            tier,
            ..AgentPassport::default()
        };

        self.passports.insert(passport_id, passport);
        Ok(passport_id)
    }

    /// Attempt to transfer a passport. Always fails (soulbound).
    pub fn transfer(
        &self,
        _passport_id: u256,
        _from: &Address,
        _to: &Address,
    ) -> Result<(), RegistryError> {
        Err(RegistryError::SoulboundTransferDenied)
    }

    /// Look up a passport by ID.
    pub fn get_passport(&self, passport_id: u256) -> Option<&AgentPassport> {
        self.passports.get(&passport_id)
    }

    /// Submit a system prompt hash update (starts the 24h timelock).
    ///
    /// The ventriloquist defense: prompt changes require a timelock so that
    /// malicious prompt injection cannot immediately take effect.
    pub fn submit_prompt_update(
        &mut self,
        passport_id: u256,
        caller: &Address,
        new_hash: [u8; 32],
        now: u64,
    ) -> Result<(), RegistryError> {
        let passport = self
            .passports
            .get(&passport_id)
            .ok_or(RegistryError::PassportNotFound(passport_id))?;

        if &passport.owner != caller {
            return Err(RegistryError::NotOwner);
        }

        self.pending_updates.insert(
            passport_id,
            PendingPromptUpdate {
                new_hash,
                submitted_at: now,
                executable_after: now + PROMPT_UPDATE_TIMELOCK_SECS,
            },
        );

        Ok(())
    }

    /// Execute a pending prompt update after the timelock has elapsed.
    ///
    /// Returns the reputation penalty if rate-limited (>3 changes in 30 days).
    pub fn execute_prompt_update(
        &mut self,
        passport_id: u256,
        caller: &Address,
        now: u64,
    ) -> Result<f64, RegistryError> {
        let passport = self
            .passports
            .get(&passport_id)
            .ok_or(RegistryError::PassportNotFound(passport_id))?;

        if &passport.owner != caller {
            return Err(RegistryError::NotOwner);
        }

        let pending = self
            .pending_updates
            .get(&passport_id)
            .ok_or(RegistryError::NoPendingUpdate)?;

        if now < pending.executable_after {
            return Err(RegistryError::TimelockNotElapsed {
                remaining_secs: pending.executable_after - now,
            });
        }

        let new_hash = pending.new_hash;

        // Check prompt change rate.
        let changes = self.prompt_changes.entry(passport_id).or_default();
        // Remove changes outside the 30-day window.
        changes.retain(|c| now.saturating_sub(c.timestamp) < PROMPT_CHANGE_WINDOW_SECS);
        changes.push(PromptChangeRecord { timestamp: now });

        let penalty = if changes.len() > MAX_PROMPT_CHANGES_IN_WINDOW {
            -0.05 // Reputation penalty per spec
        } else {
            0.0
        };

        // Apply the update.
        if let Some(passport) = self.passports.get_mut(&passport_id) {
            passport.system_prompt_hash = new_hash;
        }

        self.pending_updates.remove(&passport_id);

        Ok(penalty)
    }

    /// Update the passport tier based on current stake (called by staking contract).
    pub fn update_tier(
        &mut self,
        passport_id: u256,
        stake: u256,
    ) -> Result<PassportTier, RegistryError> {
        let passport = self
            .passports
            .get_mut(&passport_id)
            .ok_or(RegistryError::PassportNotFound(passport_id))?;

        let new_tier = tier_from_stake(stake);
        passport.tier = new_tier;
        Ok(new_tier)
    }

    /// Number of registered passports.
    pub fn passport_count(&self) -> usize {
        self.passports.len()
    }

    /// Check if a passport has a specific capability.
    pub fn has_capability(&self, passport_id: u256, capability_bit: u64) -> bool {
        self.passports
            .get(&passport_id)
            .is_some_and(|p| p.capability_list & capability_bit != 0)
    }
}

/// Determine passport tier from KORAI stake amount only (legacy path).
///
/// For full tier evaluation including job count and reputation, use
/// [`TierProgressionRules::evaluate`].
fn tier_from_stake(stake: u256) -> PassportTier {
    if stake >= TIER_PROTOCOL_STAKE {
        PassportTier::Protocol
    } else if stake >= TIER_SOVEREIGN_STAKE {
        PassportTier::Sovereign
    } else if stake >= TIER_WORKER_STAKE {
        PassportTier::Worker
    } else {
        PassportTier::Edge
    }
}

// ─── Tier Progression Rules ─────────────────────────────────────────

/// Tier progression requirements per spec.
///
/// From `docs/08-chain/04-korai-passport-erc-721-soulbound.md` lines 107-118:
/// - Edge -> Worker: Stake 5,000 KORAI + 10 jobs with avg reputation > 0.5
/// - Worker -> Sovereign: Stake 25,000 KORAI + 100 jobs with avg reputation > 0.7
/// - Sovereign -> Protocol: Governance vote (cannot self-promote)
/// - Demotion: stake drops below threshold OR reputation below minimum for 30 consecutive days
#[derive(Debug, Clone, PartialEq)]
pub struct TierProgressionRules {
    /// Minimum jobs for Edge -> Worker.
    pub edge_to_worker_jobs: u64,
    /// Minimum avg reputation for Edge -> Worker.
    pub edge_to_worker_min_rep: f64,
    /// Minimum jobs for Worker -> Sovereign.
    pub worker_to_sovereign_jobs: u64,
    /// Minimum avg reputation for Worker -> Sovereign.
    pub worker_to_sovereign_min_rep: f64,
    /// Days below minimum reputation before demotion triggers.
    pub demotion_grace_days: u64,
}

impl Default for TierProgressionRules {
    fn default() -> Self {
        Self {
            edge_to_worker_jobs: 10,
            edge_to_worker_min_rep: 0.5,
            worker_to_sovereign_jobs: 100,
            worker_to_sovereign_min_rep: 0.7,
            demotion_grace_days: 30,
        }
    }
}

/// Agent progression state tracked for demotion logic.
#[derive(Debug, Clone, Default)]
pub struct AgentProgressionState {
    /// Total completed jobs across all domains.
    pub total_jobs: u64,
    /// Average reputation across all domains (decay-adjusted).
    pub avg_reputation: f64,
    /// Current KORAI stake.
    pub stake: u256,
    /// Consecutive days reputation has been below tier minimum (for demotion).
    pub days_below_min: u64,
    /// Whether Protocol tier was granted via governance (not self-promoted).
    pub governance_approved: bool,
}

/// Result of a tier evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TierEvaluation {
    /// Agent stays at current tier.
    Maintain(PassportTier),
    /// Agent qualifies for promotion.
    Promote(PassportTier),
    /// Agent should be demoted.
    Demote(PassportTier),
    /// Promotion requires governance vote (cannot self-promote to Protocol).
    RequiresGovernance,
}

impl TierProgressionRules {
    /// Evaluate what tier an agent qualifies for based on their full state.
    ///
    /// Returns the appropriate action (maintain, promote, or demote).
    pub fn evaluate(
        &self,
        current_tier: PassportTier,
        state: &AgentProgressionState,
    ) -> TierEvaluation {
        // First check demotion conditions.
        if let Some(demoted) = self.check_demotion(current_tier, state) {
            return TierEvaluation::Demote(demoted);
        }

        // Then check promotion conditions.
        if let Some(promoted) = self.check_promotion(current_tier, state) {
            return TierEvaluation::Promote(promoted);
        }

        // Special case: Sovereign -> Protocol requires governance vote.
        // If they meet stake requirements but lack governance approval, signal it.
        if current_tier == PassportTier::Sovereign
            && state.stake >= TIER_PROTOCOL_STAKE
            && !state.governance_approved
        {
            return TierEvaluation::RequiresGovernance;
        }

        TierEvaluation::Maintain(current_tier)
    }

    /// Check if agent qualifies for promotion.
    fn check_promotion(
        &self,
        current_tier: PassportTier,
        state: &AgentProgressionState,
    ) -> Option<PassportTier> {
        match current_tier {
            PassportTier::Edge => {
                if state.stake >= TIER_WORKER_STAKE
                    && state.total_jobs >= self.edge_to_worker_jobs
                    && state.avg_reputation >= self.edge_to_worker_min_rep
                {
                    Some(PassportTier::Worker)
                } else {
                    None
                }
            }
            PassportTier::Worker => {
                if state.stake >= TIER_SOVEREIGN_STAKE
                    && state.total_jobs >= self.worker_to_sovereign_jobs
                    && state.avg_reputation >= self.worker_to_sovereign_min_rep
                {
                    Some(PassportTier::Sovereign)
                } else {
                    None
                }
            }
            PassportTier::Sovereign => {
                if state.stake >= TIER_PROTOCOL_STAKE && state.governance_approved {
                    Some(PassportTier::Protocol)
                } else {
                    None
                }
            }
            PassportTier::Protocol => None, // Already highest
        }
    }

    /// Check if agent should be demoted.
    fn check_demotion(
        &self,
        current_tier: PassportTier,
        state: &AgentProgressionState,
    ) -> Option<PassportTier> {
        // Protocol tier is only demoted by governance, not automatically.
        if current_tier == PassportTier::Protocol {
            return None;
        }

        let (min_stake, min_rep) = match current_tier {
            PassportTier::Sovereign => (TIER_SOVEREIGN_STAKE, self.worker_to_sovereign_min_rep),
            PassportTier::Worker => (TIER_WORKER_STAKE, self.edge_to_worker_min_rep),
            PassportTier::Edge | PassportTier::Protocol => return None,
        };

        // Immediate demotion if stake drops below threshold.
        if state.stake < min_stake {
            return Some(match current_tier {
                PassportTier::Sovereign => PassportTier::Worker,
                PassportTier::Worker => PassportTier::Edge,
                _ => return None,
            });
        }

        // Demotion after grace period if reputation stays below minimum.
        if state.avg_reputation < min_rep && state.days_below_min >= self.demotion_grace_days {
            return Some(match current_tier {
                PassportTier::Sovereign => PassportTier::Worker,
                PassportTier::Worker => PassportTier::Edge,
                _ => return None,
            });
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADMIN: &str = "admin";

    fn owner() -> Address {
        "0x0001".to_string()
    }

    fn other() -> Address {
        "0x0002".to_string()
    }

    fn sample_prompt_hash() -> [u8; 32] {
        [0xAB; 32]
    }

    #[test]
    fn mint_creates_passport() {
        let mut registry = AgentRegistry::new(ADMIN);
        let id = registry
            .mint(
                ADMIN,
                owner(),
                CAP_INFERENCE | CAP_RAG,
                sample_prompt_hash(),
                30_000,
            )
            .unwrap();

        let passport = registry.get_passport(id).unwrap();
        assert_eq!(passport.owner, owner());
        assert_eq!(passport.capability_list, CAP_INFERENCE | CAP_RAG);
        assert_eq!(passport.system_prompt_hash, sample_prompt_hash());
        assert_eq!(passport.tier, PassportTier::Sovereign); // 30k stake
    }

    #[test]
    fn mint_only_by_admin() {
        let mut registry = AgentRegistry::new(ADMIN);
        let err = registry
            .mint("not_admin", owner(), 0, [0; 32], 0)
            .unwrap_err();
        assert_eq!(err, RegistryError::AdminOnly);
    }

    #[test]
    fn transfer_always_fails_soulbound() {
        let mut registry = AgentRegistry::new(ADMIN);
        let id = registry.mint(ADMIN, owner(), 0, [0; 32], 0).unwrap();
        let err = registry.transfer(id, &owner(), &other()).unwrap_err();
        assert_eq!(err, RegistryError::SoulboundTransferDenied);
    }

    #[test]
    fn prompt_update_requires_timelock() {
        let mut registry = AgentRegistry::new(ADMIN);
        let id = registry.mint(ADMIN, owner(), 0, [0; 32], 0).unwrap();
        let now = 1_000_000;

        // Submit update.
        registry
            .submit_prompt_update(id, &owner(), [0xFF; 32], now)
            .unwrap();

        // Try to execute immediately -- should fail.
        let err = registry
            .execute_prompt_update(id, &owner(), now)
            .unwrap_err();
        assert!(matches!(err, RegistryError::TimelockNotElapsed { .. }));

        // Execute after 24h.
        let after_24h = now + PROMPT_UPDATE_TIMELOCK_SECS;
        let penalty = registry
            .execute_prompt_update(id, &owner(), after_24h)
            .unwrap();
        assert_eq!(penalty, 0.0);

        // Verify the hash was updated.
        let passport = registry.get_passport(id).unwrap();
        assert_eq!(passport.system_prompt_hash, [0xFF; 32]);
    }

    #[test]
    fn prompt_update_rate_limit_penalty() {
        let mut registry = AgentRegistry::new(ADMIN);
        let id = registry.mint(ADMIN, owner(), 0, [0; 32], 0).unwrap();
        let mut now = 1_000_000u64;

        // Make 4 changes (1 under limit, then 3 more to exceed).
        for i in 0..4u8 {
            let hash = [i; 32];
            registry
                .submit_prompt_update(id, &owner(), hash, now)
                .unwrap();
            now += PROMPT_UPDATE_TIMELOCK_SECS;
            let penalty = registry.execute_prompt_update(id, &owner(), now).unwrap();

            if i < 3 {
                assert_eq!(penalty, 0.0, "no penalty for change #{i}");
            } else {
                assert!(
                    (penalty - (-0.05)).abs() < f64::EPSILON,
                    "should have -0.05 penalty for 4th change, got {penalty}"
                );
            }
            now += 1; // Small gap between changes.
        }
    }

    #[test]
    fn tier_from_stake_thresholds() {
        assert_eq!(tier_from_stake(0), PassportTier::Edge);
        assert_eq!(tier_from_stake(4_999), PassportTier::Edge);
        assert_eq!(tier_from_stake(5_000), PassportTier::Worker);
        assert_eq!(tier_from_stake(24_999), PassportTier::Worker);
        assert_eq!(tier_from_stake(25_000), PassportTier::Sovereign);
        assert_eq!(tier_from_stake(99_999), PassportTier::Sovereign);
        assert_eq!(tier_from_stake(100_000), PassportTier::Protocol);
    }

    #[test]
    fn update_tier_changes_passport() {
        let mut registry = AgentRegistry::new(ADMIN);
        let id = registry.mint(ADMIN, owner(), 0, [0; 32], 0).unwrap();

        assert_eq!(registry.get_passport(id).unwrap().tier, PassportTier::Edge);

        let new_tier = registry.update_tier(id, 50_000).unwrap();
        assert_eq!(new_tier, PassportTier::Sovereign);
        assert_eq!(
            registry.get_passport(id).unwrap().tier,
            PassportTier::Sovereign
        );
    }

    #[test]
    fn capability_check() {
        let mut registry = AgentRegistry::new(ADMIN);
        let id = registry
            .mint(ADMIN, owner(), CAP_TRADING | CAP_SECURITY, [0; 32], 0)
            .unwrap();

        assert!(registry.has_capability(id, CAP_TRADING));
        assert!(registry.has_capability(id, CAP_SECURITY));
        assert!(!registry.has_capability(id, CAP_INFERENCE));
    }

    #[test]
    fn prompt_update_requires_owner() {
        let mut registry = AgentRegistry::new(ADMIN);
        let id = registry.mint(ADMIN, owner(), 0, [0; 32], 0).unwrap();

        let err = registry
            .submit_prompt_update(id, &other(), [0xFF; 32], 100)
            .unwrap_err();
        assert_eq!(err, RegistryError::NotOwner);
    }

    // ─── Tier progression tests ─────────────────────────────────────

    #[test]
    fn edge_promotes_to_worker_with_stake_jobs_and_rep() {
        let rules = TierProgressionRules::default();
        let state = AgentProgressionState {
            total_jobs: 12,
            avg_reputation: 0.6,
            stake: 5_000,
            ..Default::default()
        };

        let result = rules.evaluate(PassportTier::Edge, &state);
        assert_eq!(result, TierEvaluation::Promote(PassportTier::Worker));
    }

    #[test]
    fn edge_stays_without_enough_jobs() {
        let rules = TierProgressionRules::default();
        let state = AgentProgressionState {
            total_jobs: 5, // need 10
            avg_reputation: 0.8,
            stake: 10_000,
            ..Default::default()
        };

        let result = rules.evaluate(PassportTier::Edge, &state);
        assert_eq!(result, TierEvaluation::Maintain(PassportTier::Edge));
    }

    #[test]
    fn edge_stays_without_enough_reputation() {
        let rules = TierProgressionRules::default();
        let state = AgentProgressionState {
            total_jobs: 20,
            avg_reputation: 0.4, // need 0.5
            stake: 10_000,
            ..Default::default()
        };

        let result = rules.evaluate(PassportTier::Edge, &state);
        assert_eq!(result, TierEvaluation::Maintain(PassportTier::Edge));
    }

    #[test]
    fn worker_promotes_to_sovereign() {
        let rules = TierProgressionRules::default();
        let state = AgentProgressionState {
            total_jobs: 150,
            avg_reputation: 0.75,
            stake: 25_000,
            ..Default::default()
        };

        let result = rules.evaluate(PassportTier::Worker, &state);
        assert_eq!(result, TierEvaluation::Promote(PassportTier::Sovereign));
    }

    #[test]
    fn sovereign_requires_governance_for_protocol() {
        let rules = TierProgressionRules::default();
        let state = AgentProgressionState {
            total_jobs: 500,
            avg_reputation: 0.95,
            stake: 100_000,
            governance_approved: false,
            ..Default::default()
        };

        let result = rules.evaluate(PassportTier::Sovereign, &state);
        assert_eq!(result, TierEvaluation::RequiresGovernance);
    }

    #[test]
    fn sovereign_promotes_with_governance_approval() {
        let rules = TierProgressionRules::default();
        let state = AgentProgressionState {
            total_jobs: 500,
            avg_reputation: 0.95,
            stake: 100_000,
            governance_approved: true,
            ..Default::default()
        };

        let result = rules.evaluate(PassportTier::Sovereign, &state);
        assert_eq!(result, TierEvaluation::Promote(PassportTier::Protocol));
    }

    #[test]
    fn demotion_on_stake_drop() {
        let rules = TierProgressionRules::default();
        let state = AgentProgressionState {
            total_jobs: 200,
            avg_reputation: 0.8,
            stake: 4_000, // below Worker threshold (5000)
            ..Default::default()
        };

        let result = rules.evaluate(PassportTier::Worker, &state);
        assert_eq!(result, TierEvaluation::Demote(PassportTier::Edge));
    }

    #[test]
    fn demotion_after_grace_period() {
        let rules = TierProgressionRules::default();
        let state = AgentProgressionState {
            total_jobs: 200,
            avg_reputation: 0.3, // below Worker min (0.5)
            stake: 10_000,       // stake is fine
            days_below_min: 30,  // grace period expired
            ..Default::default()
        };

        let result = rules.evaluate(PassportTier::Worker, &state);
        assert_eq!(result, TierEvaluation::Demote(PassportTier::Edge));
    }

    #[test]
    fn no_demotion_within_grace_period() {
        let rules = TierProgressionRules::default();
        let state = AgentProgressionState {
            total_jobs: 200,
            avg_reputation: 0.3, // below Worker min
            stake: 10_000,
            days_below_min: 15, // within grace period
            ..Default::default()
        };

        let result = rules.evaluate(PassportTier::Worker, &state);
        assert_eq!(result, TierEvaluation::Maintain(PassportTier::Worker));
    }

    #[test]
    fn protocol_never_auto_demoted() {
        let rules = TierProgressionRules::default();
        let state = AgentProgressionState {
            total_jobs: 0,
            avg_reputation: 0.0,
            stake: 0, // even with zero everything
            days_below_min: 999,
            governance_approved: true,
            ..Default::default()
        };

        let result = rules.evaluate(PassportTier::Protocol, &state);
        assert_eq!(result, TierEvaluation::Maintain(PassportTier::Protocol));
    }
}
