//! Roko light-client primitives + a real Tempo testnet backend.
//!
//! Workstream A "agents-as-light-clients" surface from the 2026-05-01 Jacob ×
//! Jae × JD Tempo+Oracle call.
//!
//! This crate gives Roko agents a chain-agnostic [`LightClient`] trait and a
//! [`TempoLightClient`] that connects to the Tempo "Moderato" testnet, fetches
//! headers + EIP-1186 state proofs, and verifies them locally.
//!
//! # Backends
//!
//! | Backend | Network | Default? | Cargo feature |
//! |---|---|---|---|
//! | [`rpc::TempoRpcBackend`] | Tempo testnet (Moderato) over JSON-RPC | yes | always-on |
//! | [`mock::MockLightClient`] | none — deterministic in-memory chain | no — tests/demos only | `mock` |
//! | commonware-p2p follower socket | Tempo consensus peer | no — Phase 2, gated on Tempo opening its surface | `commonware-backend` |
//!
//! The trait surface is stable across backends; only the inner implementation
//! changes.
//!
//! # Trust model (current default — RPC + TOFU)
//!
//! - **State reads**: cryptographically verified. `eth_getProof` returns a
//!   Merkle Patricia trie proof. We walk it locally against the header's
//!   `stateRoot`. The RPC operator cannot lie about an account's balance,
//!   nonce, storage root, or code hash without producing an invalid proof.
//! - **Header chain**: trust-on-first-use. The first header observed at
//!   construction is pinned as the anchor; every subsequent header must
//!   parent-hash-chain back to it. The RPC operator can still feed us a
//!   counterfeit anchor at boot time. Fixing this requires either a
//!   hardcoded checkpoint or full BLS aggregate-signature verification —
//!   the latter lands in Phase 2 once Tempo opens its consensus-peer
//!   surface (commonware-cryptography integration).
//!
//! # Cross-references
//!
//! - Nunchi-trade/collaboration PR #144 — Tempo Integration Workstreams (this
//!   crate is the Workstream A artifact, agent-side).
//! - Nunchi-trade/collaboration PR #143 §0 + PR #145 §0 — the source-of-truth
//!   architecture this trait mirrors: native chain state on the producer
//!   chain, verified consumption on the agent side.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod mpt;
mod proof;
mod tempo;
mod traits;

/// Real Tempo testnet (Moderato) JSON-RPC backend.
pub mod rpc;

/// Deterministic in-memory backend.
///
/// Used by tests, examples, and offline development. Off by default — enable
/// with `--features mock`.
#[cfg(feature = "mock")]
pub mod mock;

pub use error::LcError;
pub use proof::{AccountProof, AttestationSig, MerkleProof, VerifiedHeader};
pub use rpc::TempoRpcBackend;
pub use tempo::TempoLightClient;
pub use traits::{BlockNumber, LightClient};

/// Convenience result alias for light-client operations.
pub type LcResult<T> = Result<T, LcError>;

/// Tempo testnet "Moderato" canonical RPC URL.
pub const MODERATO_RPC_URL: &str = "https://rpc.moderato.tempo.xyz";

/// Tempo testnet "Moderato" chain id.
pub const MODERATO_CHAIN_ID: u64 = 42431;
