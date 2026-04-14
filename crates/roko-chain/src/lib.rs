#![deny(unsafe_code)]
#![warn(missing_docs)]

//! On-chain client abstractions for Roko.
//!
//! `ChainClient` reads chain state; `ChainWallet` signs and submits transactions.
//! These traits are backend-agnostic — see [`mock`] for test doubles, and the
//! future `MirageChainClient` / `AlloyChainClient` impls for real backends.

#[cfg(feature = "alloy-backend")]
pub mod alloy_impl;
pub mod client;
pub mod gate;
pub mod mock;
pub mod types;
pub mod wallet;
pub mod witness;

pub use client::ChainClient;
pub use gate::{
    MockTxSimulator, SimulationOutcome, TxSimGate, TxSimGateConfig, TxSimulator, WalletCheck,
    WalletGate, WalletGateConfig,
};
pub use mock::{MockChainClient, MockChainWallet, paired_mocks};
pub use types::{
    BlockNumber, CallResult, ChainError, ChainHeader, ChainResult, LogEntry, Receipt, TxHash,
    TxRequest,
};
pub use wallet::ChainWallet;
pub use witness::ChainWitnessEngine;
