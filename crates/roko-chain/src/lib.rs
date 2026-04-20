#![deny(unsafe_code)]
#![warn(missing_docs)]
// Chain crate: numerics-heavy with many trait impls; suppress pedantic lints
// that fire frequently without improving correctness.
#![allow(
    clippy::approx_constant,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::derivable_impls,
    clippy::derive_partial_eq_without_eq,
    clippy::doc_markdown,
    clippy::format_collect,
    clippy::imprecise_flops,
    clippy::items_after_statements,
    clippy::manual_let_else,
    clippy::manual_midpoint,
    clippy::manual_slice_fill,
    clippy::manual_strip,
    clippy::map_unwrap_or,
    clippy::match_same_arms,
    clippy::missing_const_for_fn,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::single_match_else,
    clippy::struct_excessive_bools,
    clippy::suboptimal_flops,
    clippy::too_many_arguments,
    clippy::uninlined_format_args,
    clippy::unwrap_used,
    clippy::unused_self,
    unused_imports
)]

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
pub mod heartbeat_ext;
pub mod identity_economy_identity;
pub mod identity_economy_markets;
pub mod isfr;
/// KORAI token with lazy demurrage (CHAIN-01).
pub mod korai_token;
/// Spore job marketplace with escrow and 3 hiring models (CHAIN-04).
pub mod marketplace;
pub mod mock;
pub mod observer;
pub mod phase2;
/// Reputation Registry with 7-domain EMA scoring (CHAIN-03).
pub mod reputation_registry;
/// Chain domain DeFi tool definitions (10 core tools).
pub mod tools;
/// TraceRank: PageRank-style reputation propagation over payment edges (P1-02).
pub mod trace_rank;
pub mod triage;
pub mod types;
pub mod validation_registry;
pub mod wallet;
pub mod witness;

pub use agent_registry::AgentRegistry;
pub use client::ChainClient;
pub use futures_market::{FuturesMarket, FuturesMarketConfig};
pub use gate::{
    MempoolTx, MevAlert, MevAnalysisInput, MevDetector, MevDetectorConfig, MevGate, MevPattern,
    MevSeverity, MockTxSimulator, SandwichBundle, SimulationOutcome, TxSimGate, TxSimGateConfig,
    TxSimulator, WalletCheck, WalletGate, WalletGateConfig,
};
pub use heartbeat_ext::{
    ChainHeartbeatExtension, ChainPreActResult, PolicyCageConfig, PolicyCageState, PolicyViolation,
    SimulateResult, SleepwalkerConfig, ValidateResult, ViolationSeverity,
};
pub use isfr::{IsfrConfig, IsfrRegistry};
pub use korai_token::{KoraiToken, KoraiTokenConfig};
pub use marketplace::{
    AssignmentResult, EscrowEntry, JobState, Marketplace, MarketplaceConfig, MarketplaceError,
    MarketplaceJob, SettlementResult,
};
pub use mock::{MockChainClient, MockChainWallet, paired_mocks};
pub use observer::{
    AddressFilter, BlockObserver, BlockObserverConfig, BlockTracker, ObservedEvent,
};
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
    BalanceProof, ChannelLifecycle, ChannelSettlement, PaymentAuthorization, PaymentRequest,
    StateChannel, VerificationStatus, X402Config, X402Error, X402Manager,
};
