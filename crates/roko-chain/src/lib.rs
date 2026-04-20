#![deny(unsafe_code)]
#![warn(missing_docs)]

//! On-chain client abstractions for Roko.
//!
//! `ChainClient` reads chain state; `ChainWallet` signs and submits transactions.
//! These traits are backend-agnostic — this crate ships [`mock`] test doubles
//! and an optional Alloy-backed JSON-RPC backend. Other substrates, including
//! mirage-rs, can be adapted behind the same traits, but no dedicated mirage
//! backend ships here today.

/// Agent Registry with soulbound ERC-721 passports (CHAIN-02).
pub mod agent_registry;
#[cfg(feature = "alloy-backend")]
pub mod alloy_impl;
pub mod client;
pub mod futures_market;
pub mod gate;
/// KORAI token with lazy demurrage (CHAIN-01).
pub mod korai_token;
pub mod heartbeat_ext;
pub mod identity_economy_identity;
pub mod identity_economy_markets;
pub mod isfr;
/// Spore job marketplace with escrow and 3 hiring models (CHAIN-04).
pub mod marketplace;
pub mod mock;
pub mod observer;
pub mod phase2;
/// Reputation Registry with 7-domain EMA scoring (CHAIN-03).
pub mod reputation_registry;
/// TraceRank: PageRank-style reputation propagation over payment edges (P1-02).
pub mod trace_rank;
/// Chain domain DeFi tool definitions (10 core tools).
pub mod tools;
pub mod triage;
pub mod types;
pub mod validation_registry;
pub mod wallet;
pub mod witness;

pub use agent_registry::AgentRegistry;
pub use client::ChainClient;
pub use gate::{
    MevAlert, MevAnalysisInput, MevDetector, MevDetectorConfig, MevGate, MevPattern, MevSeverity,
    MempoolTx, MockTxSimulator, SandwichBundle, SimulationOutcome, TxSimGate, TxSimGateConfig,
    TxSimulator, WalletCheck, WalletGate, WalletGateConfig,
};
pub use futures_market::{FuturesMarket, FuturesMarketConfig};
pub use heartbeat_ext::{
    ChainHeartbeatExtension, ChainPreActResult, PolicyCageConfig, PolicyCageState,
    PolicyViolation, SimulateResult, SleepwalkerConfig, ValidateResult, ViolationSeverity,
};
pub use isfr::{IsfrConfig, IsfrRegistry};
pub use korai_token::{KoraiToken, KoraiTokenConfig};
pub use marketplace::{
    Marketplace, MarketplaceConfig, MarketplaceError, MarketplaceJob, JobState,
    EscrowEntry, AssignmentResult, SettlementResult,
};
pub use mock::{MockChainClient, MockChainWallet, paired_mocks};
pub use observer::{AddressFilter, BlockObserver, BlockObserverConfig, BlockTracker, ObservedEvent};
pub use phase2::*;
pub use reputation_registry::ReputationRegistry;
pub use triage::{
    EventEnrichment, MidasRScorer, TriageAction, TriageConfig, TriagePipeline, TriageResult,
};
pub use types::{
    BlockNumber, CallResult, ChainError, ChainHeader, ChainResult, LogEntry, Receipt, TxHash,
    TxRequest,
};
pub use validation_registry::{
    GateScore, ValidationError, ValidationRecord, ValidationRegistry, ValidationRegistryConfig,
    VerificationResult,
};
pub use wallet::ChainWallet;
pub use witness::{ChainWitnessEngine, verify_on_chain, witness_on_chain};
/// HTTP 402 micropayment protocol with state channels (CHAIN-08).
pub mod x402;
pub use x402::{
    X402Config, X402Error, X402Manager, PaymentAuthorization, PaymentRequest,
    StateChannel, ChannelLifecycle, BalanceProof, ChannelSettlement, VerificationStatus,
};
