//! Roko light-client primitives + a Tempo-shaped implementation.
//!
//! This crate gives Roko agents a chain-agnostic [`LightClient`] trait and a
//! [`TempoLightClient`] backend so an agent can consume external chain state
//! (Tempo today, any commonware-based chain by extension) with cryptographic
//! verification — the Workstream A "agents-as-light-clients" demo from the
//! 2026-05-01 Jacob × Jae × JD Tempo+Oracle call.
//!
//! # Phase model
//!
//! | Phase | Backend | Network | Status |
//! |---|---|---|---|
//! | 0 | [`mock::MockLightClient`] | none | this PR — mock-only, deterministic |
//! | 1 | commonware-p2p follower-node anchor | follower socket | gated on [`commonware-backend`] feature, not yet wired |
//! | 2 | commonware-p2p LC peer protocol | LC peer | gated on Tempo's LC protocol shipping |
//!
//! The trait surface stays stable across phases; only the backing changes.
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
mod proof;
mod tempo;
mod traits;

/// Mock backends used in tests, examples, and offline development.
pub mod mock;

pub use error::LcError;
pub use proof::{AccountProof, AttestationSig, MerkleProof, VerifiedHeader};
pub use tempo::TempoLightClient;
pub use traits::{BlockNumber, LightClient};

/// Convenience result alias for light-client operations.
pub type LcResult<T> = Result<T, LcError>;
