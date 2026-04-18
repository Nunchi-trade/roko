//! [`Gate`](roko_core::traits::Gate) impls that validate on-chain preconditions
//! before an agent signs a tx.
//!
//! These gates close the loop between planning and execution: an agent produces
//! a `Engram` describing a planned transaction, and the gate answers "is this
//! safe to sign?". Two complementary checks live here:
//!
//! * [`WalletGate`] — balance / nonce checks against a
//!   [`ChainWallet`](crate::ChainWallet) bound to a [`ChainClient`](crate::ChainClient).
//!   Fails fast if the wallet cannot afford the tx or its nonce is out of sync.
//!   If callers request allowance enforcement today, the gate reports that as
//!   unsupported instead of silently pretending the check happened.
//! * [`TxSimGate`] — simulates the planned tx through a pluggable
//!   [`TxSimulator`] (for example an alloy `eth_call` wrapper, a caller-owned
//!   mirage adapter, or a mock) and returns a verdict on revert / gas overrun.
//!
//! # Engram contract
//!
//! Both gates read the signal body as a JSON-encoded [`TxRequest`](crate::TxRequest).
//! Accepted body shapes:
//!
//! * [`Body::Json`](roko_core::Body::Json) whose value deserializes to `TxRequest`.
//! * [`Body::Text`](roko_core::Body::Text) containing valid `TxRequest` JSON.
//!
//! Empty / raw-bytes bodies produce a failing verdict without side effects.
//! The gates never mutate the input signal.

pub mod tx_sim_gate;
pub mod wallet_gate;

pub use tx_sim_gate::{
    MockTxSimulator, SimulationOutcome, TxSimGate, TxSimGateConfig, TxSimulator,
};
pub use wallet_gate::{WalletCheck, WalletGate, WalletGateConfig};
