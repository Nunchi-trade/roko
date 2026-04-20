//! KORAI token implementation with lazy demurrage.
//!
//! CHAIN-01: KORAI is the native token of the Korai chain with 1% annual
//! demurrage (holding cost that decays balances over time, preventing
//! hoarding). The demurrage is applied on-read (lazy calculation) rather
//! than periodic transactions to avoid gas overhead.
//!
//! Demurrage formula:
//!   `effective_balance = stored_balance * (1 - annual_rate) ^ (elapsed_seconds / seconds_per_year)`
//!
//! Five earning pathways: task completion, knowledge contribution, validation
//! participation, reputation staking, marketplace fees.
//!
//! Five spending mechanisms: compute purchase, knowledge access, job posting,
//! escrow deposits, governance participation.

use std::collections::HashMap;

use crate::phase2::u256;

/// Seconds in one year (365.25 days).
const SECONDS_PER_YEAR: f64 = 365.25 * 24.0 * 3600.0;

/// Default annual demurrage rate (1%).
const DEFAULT_DEMURRAGE_RATE: f64 = 0.01;

/// An individual balance record with demurrage tracking.
#[derive(Debug, Clone)]
pub struct BalanceRecord {
    /// Stored (raw) balance before demurrage application.
    pub stored_balance: u256,
    /// Unix timestamp (seconds) of the last balance update.
    pub last_update: u64,
}

impl BalanceRecord {
    /// Create a new balance record at the given timestamp.
    pub fn new(balance: u256, timestamp: u64) -> Self {
        Self {
            stored_balance: balance,
            last_update: timestamp,
        }
    }

    /// Compute the effective balance with demurrage applied.
    ///
    /// Uses the formula: `stored * (1 - rate) ^ (elapsed / year)`.
    /// This is the lazy on-read calculation from the spec.
    pub fn effective_balance(&self, now: u64, annual_rate: f64) -> u256 {
        if now <= self.last_update || self.stored_balance == 0 {
            return self.stored_balance;
        }
        let elapsed = (now - self.last_update) as f64;
        let decay_factor = (1.0 - annual_rate).powf(elapsed / SECONDS_PER_YEAR);
        (self.stored_balance as f64 * decay_factor) as u256
    }

    /// Apply demurrage and update the stored balance (materialise the decay).
    pub fn materialise_demurrage(&mut self, now: u64, annual_rate: f64) {
        let effective = self.effective_balance(now, annual_rate);
        self.stored_balance = effective;
        self.last_update = now;
    }
}

/// Earning pathway for KORAI tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EarningPathway {
    /// Tokens earned by completing tasks.
    TaskCompletion,
    /// Tokens earned by contributing knowledge entries.
    KnowledgeContribution,
    /// Tokens earned by participating in validation.
    ValidationParticipation,
    /// Tokens earned by staking on reputation.
    ReputationStaking,
    /// Tokens earned from marketplace fees.
    MarketplaceFees,
}

/// Spending mechanism for KORAI tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpendingMechanism {
    /// Spend tokens to purchase compute resources.
    ComputePurchase,
    /// Spend tokens to access knowledge.
    KnowledgeAccess,
    /// Spend tokens to post a job.
    JobPosting,
    /// Deposit tokens into escrow.
    EscrowDeposit,
    /// Spend tokens for governance participation.
    GovernanceParticipation,
}

/// A transfer record in the ledger.
#[derive(Debug, Clone)]
pub struct Transfer {
    /// Source address.
    pub from: String,
    /// Destination address.
    pub to: String,
    /// Amount transferred.
    pub amount: u256,
    /// Earning pathway (for mints) or spending mechanism (for burns).
    pub reason: TransferReason,
    /// Unix timestamp (seconds).
    pub timestamp: u64,
}

/// Reason for a token transfer.
#[derive(Debug, Clone)]
pub enum TransferReason {
    /// Tokens minted via an earning pathway.
    Earn(EarningPathway),
    /// Tokens spent via a spending mechanism.
    Spend(SpendingMechanism),
    /// Plain transfer between accounts.
    Transfer,
}

/// Configuration for the KORAI token contract.
#[derive(Debug, Clone)]
pub struct KoraiTokenConfig {
    /// Annual demurrage rate (default 0.01 = 1%).
    pub demurrage_rate: f64,
    /// Token name.
    pub name: String,
    /// Token symbol.
    pub symbol: String,
    /// Whether this is the testnet variant (DAEJI).
    pub is_testnet: bool,
}

impl Default for KoraiTokenConfig {
    fn default() -> Self {
        Self {
            demurrage_rate: DEFAULT_DEMURRAGE_RATE,
            name: "KORAI".to_string(),
            symbol: "KORAI".to_string(),
            is_testnet: false,
        }
    }
}

impl KoraiTokenConfig {
    /// Create the testnet variant (DAEJI).
    pub fn testnet() -> Self {
        Self {
            demurrage_rate: DEFAULT_DEMURRAGE_RATE,
            name: "DAEJI".to_string(),
            symbol: "DAEJI".to_string(),
            is_testnet: true,
        }
    }
}

/// In-memory KORAI ERC-20 token with lazy demurrage.
///
/// Balances are stored raw and demurrage is applied on every read via
/// `balance_of()`. This avoids per-block transactions while still
/// enforcing the 1% annual holding cost.
#[derive(Debug, Clone)]
pub struct KoraiToken {
    config: KoraiTokenConfig,
    /// Balances keyed by address.
    balances: HashMap<String, BalanceRecord>,
    /// Total supply (pre-demurrage, for accounting).
    total_supply: u256,
    /// Transfer history.
    transfers: Vec<Transfer>,
}

impl KoraiToken {
    /// Create a new KORAI token with the given configuration.
    pub fn new(config: KoraiTokenConfig) -> Self {
        Self {
            config,
            balances: HashMap::new(),
            total_supply: 0,
            transfers: Vec::new(),
        }
    }

    /// Create with default mainnet configuration.
    pub fn mainnet() -> Self {
        Self::new(KoraiTokenConfig::default())
    }

    /// Create with testnet (DAEJI) configuration.
    pub fn testnet() -> Self {
        Self::new(KoraiTokenConfig::testnet())
    }

    /// Token name.
    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Token symbol.
    pub fn symbol(&self) -> &str {
        &self.config.symbol
    }

    /// Read the effective (demurrage-adjusted) balance for an address.
    ///
    /// This is the lazy demurrage calculation: `stored * (1-r)^(t/year)`.
    pub fn balance_of(&self, address: &str, now: u64) -> u256 {
        self.balances
            .get(address)
            .map(|record| record.effective_balance(now, self.config.demurrage_rate))
            .unwrap_or(0)
    }

    /// Mint tokens to an address (earning pathway).
    pub fn mint(&mut self, to: &str, amount: u256, pathway: EarningPathway, now: u64) {
        let entry = self
            .balances
            .entry(to.to_string())
            .or_insert_with(|| BalanceRecord::new(0, now));

        // Materialise existing demurrage before adding new tokens.
        entry.materialise_demurrage(now, self.config.demurrage_rate);
        entry.stored_balance = entry.stored_balance.saturating_add(amount);
        entry.last_update = now;

        self.total_supply = self.total_supply.saturating_add(amount);

        self.transfers.push(Transfer {
            from: "0x0".to_string(),
            to: to.to_string(),
            amount,
            reason: TransferReason::Earn(pathway),
            timestamp: now,
        });
    }

    /// Transfer tokens between addresses.
    ///
    /// Returns `true` on success, `false` if insufficient balance.
    pub fn transfer(&mut self, from: &str, to: &str, amount: u256, now: u64) -> bool {
        // Materialise sender demurrage.
        let effective = self.balance_of(from, now);
        if effective < amount {
            return false;
        }

        // Deduct from sender.
        if let Some(sender) = self.balances.get_mut(from) {
            sender.materialise_demurrage(now, self.config.demurrage_rate);
            sender.stored_balance = sender.stored_balance.saturating_sub(amount);
            sender.last_update = now;
        }

        // Credit receiver.
        let receiver = self
            .balances
            .entry(to.to_string())
            .or_insert_with(|| BalanceRecord::new(0, now));
        receiver.materialise_demurrage(now, self.config.demurrage_rate);
        receiver.stored_balance = receiver.stored_balance.saturating_add(amount);
        receiver.last_update = now;

        self.transfers.push(Transfer {
            from: from.to_string(),
            to: to.to_string(),
            amount,
            reason: TransferReason::Transfer,
            timestamp: now,
        });

        true
    }

    /// Burn tokens from an address (spending mechanism).
    ///
    /// Returns `true` on success, `false` if insufficient balance.
    pub fn burn(
        &mut self,
        from: &str,
        amount: u256,
        mechanism: SpendingMechanism,
        now: u64,
    ) -> bool {
        let effective = self.balance_of(from, now);
        if effective < amount {
            return false;
        }

        if let Some(record) = self.balances.get_mut(from) {
            record.materialise_demurrage(now, self.config.demurrage_rate);
            record.stored_balance = record.stored_balance.saturating_sub(amount);
            record.last_update = now;
        }

        self.total_supply = self.total_supply.saturating_sub(amount);

        self.transfers.push(Transfer {
            from: from.to_string(),
            to: "0x0".to_string(),
            amount,
            reason: TransferReason::Spend(mechanism),
            timestamp: now,
        });

        true
    }

    /// Total supply (raw, not demurrage-adjusted).
    pub fn total_supply(&self) -> u256 {
        self.total_supply
    }

    /// Transfer history.
    pub fn transfers(&self) -> &[Transfer] {
        &self.transfers
    }

    /// Demurrage rate.
    pub fn demurrage_rate(&self) -> f64 {
        self.config.demurrage_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE_YEAR: u64 = (365.25 * 24.0 * 3600.0) as u64;

    #[test]
    fn balance_decays_over_one_year() {
        let mut token = KoraiToken::mainnet();
        let now = 1_000_000;

        token.mint("alice", 10_000, EarningPathway::TaskCompletion, now);
        assert_eq!(token.balance_of("alice", now), 10_000);

        // After 1 year at 1% demurrage, balance should be ~9900.
        let after_1y = now + ONE_YEAR;
        let balance = token.balance_of("alice", after_1y);
        assert!(
            (9800..=9950).contains(&balance),
            "expected ~9900 after 1 year demurrage, got {balance}"
        );
    }

    #[test]
    fn balance_decays_over_ten_years() {
        let mut token = KoraiToken::mainnet();
        let now = 1_000_000;

        token.mint("bob", 100_000, EarningPathway::KnowledgeContribution, now);

        // After 10 years: 100000 * (0.99)^10 ~ 90438.
        let after_10y = now + 10 * ONE_YEAR;
        let balance = token.balance_of("bob", after_10y);
        assert!(
            (89_000..=91_000).contains(&balance),
            "expected ~90438 after 10 years, got {balance}"
        );
    }

    #[test]
    fn transfer_materialises_demurrage() {
        let mut token = KoraiToken::mainnet();
        let now = 1_000_000;

        token.mint("alice", 10_000, EarningPathway::TaskCompletion, now);

        // Wait 1 year, then transfer half.
        let after_1y = now + ONE_YEAR;
        let balance_before = token.balance_of("alice", after_1y);
        let half = balance_before / 2;
        assert!(token.transfer("alice", "bob", half, after_1y));

        // Alice should have roughly half of her demurraged balance.
        let alice_balance = token.balance_of("alice", after_1y);
        assert!(alice_balance > 0);
        assert!(alice_balance <= balance_before - half + 1); // Allow rounding.

        // Bob should have the transferred amount.
        assert_eq!(token.balance_of("bob", after_1y), half);
    }

    #[test]
    fn transfer_insufficient_funds_fails() {
        let mut token = KoraiToken::mainnet();
        let now = 1_000_000;

        token.mint("alice", 100, EarningPathway::MarketplaceFees, now);
        assert!(!token.transfer("alice", "bob", 200, now));
    }

    #[test]
    fn burn_reduces_supply() {
        let mut token = KoraiToken::mainnet();
        let now = 1_000_000;

        token.mint("alice", 5_000, EarningPathway::ValidationParticipation, now);
        assert_eq!(token.total_supply(), 5_000);

        assert!(token.burn("alice", 2_000, SpendingMechanism::ComputePurchase, now));
        assert_eq!(token.balance_of("alice", now), 3_000);
        assert_eq!(token.total_supply(), 3_000);
    }

    #[test]
    fn no_demurrage_at_same_timestamp() {
        let record = BalanceRecord::new(10_000, 100);
        assert_eq!(record.effective_balance(100, 0.01), 10_000);
    }

    #[test]
    fn testnet_config() {
        let token = KoraiToken::testnet();
        assert_eq!(token.name(), "DAEJI");
        assert_eq!(token.symbol(), "DAEJI");
    }

    #[test]
    fn earning_and_spending_pathways_recorded() {
        let mut token = KoraiToken::mainnet();
        let now = 1_000_000;

        token.mint("alice", 1_000, EarningPathway::ReputationStaking, now);
        assert!(token.burn("alice", 500, SpendingMechanism::JobPosting, now));

        assert_eq!(token.transfers().len(), 2);
    }
}
