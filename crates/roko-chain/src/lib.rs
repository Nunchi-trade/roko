#![deny(unsafe_code)]
#![warn(missing_docs)]

//! On-chain client abstractions for Roko.
//!
//! `ChainClient` reads chain state; `ChainWallet` signs and submits transactions.
//! These traits are backend-agnostic — this crate ships [`mock`] test doubles
//! and an optional Alloy-backed JSON-RPC backend. Other substrates, including
//! mirage-rs, can be adapted behind the same traits, but no dedicated mirage
//! backend ships here today.

#[cfg(feature = "alloy-backend")]
pub mod alloy_impl;
pub mod client;
pub mod gate;
pub mod identity_economy_identity;
pub mod identity_economy_markets;
pub mod mock;
pub mod phase2;
pub mod types;
pub mod wallet;
pub mod witness;

pub use client::ChainClient;
pub use gate::{
    MockTxSimulator, SimulationOutcome, TxSimGate, TxSimGateConfig, TxSimulator, WalletCheck,
    WalletGate, WalletGateConfig,
};
pub use mock::{MockChainClient, MockChainWallet, paired_mocks};
pub use phase2::*;
pub use types::{
    BlockNumber, CallResult, ChainError, ChainHeader, ChainResult, LogEntry, Receipt, TxHash,
    TxRequest,
};
pub use wallet::ChainWallet;
pub use witness::{ChainWitnessEngine, verify_on_chain, witness_on_chain};
