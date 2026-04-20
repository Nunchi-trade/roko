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

use crate::phase2::{u256, AgentPassport, PassportTier, Hash, Address};

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
        passport.tier = new_tier.clone();
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

/// Determine passport tier from KORAI stake amount.
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
            .mint(ADMIN, owner(), CAP_INFERENCE | CAP_RAG, sample_prompt_hash(), 30_000)
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
        let err = registry.execute_prompt_update(id, &owner(), now).unwrap_err();
        assert!(matches!(err, RegistryError::TimelockNotElapsed { .. }));

        // Execute after 24h.
        let after_24h = now + PROMPT_UPDATE_TIMELOCK_SECS;
        let penalty = registry.execute_prompt_update(id, &owner(), after_24h).unwrap();
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
            registry.submit_prompt_update(id, &owner(), hash, now).unwrap();
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
}
