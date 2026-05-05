//! Header, account, and proof types carried across the light-client trait.
//!
//! These types are intentionally narrow — they hold only what an agent needs
//! to (a) prove a chain advanced past a known height, and (b) prove an
//! account's state at a height. Rich on-chain data (transactions, logs,
//! storage tries) can be added when a real backend needs it.

use serde::{Deserialize, Serialize};

use crate::traits::BlockNumber;

/// A header that has been cryptographically verified (BLS attestation
/// signature checked against the producer chain's validator quorum).
///
/// On Tempo, the attestation is a commonware-style threshold signature over
/// the canonical header bytes; on Daeji it is the same shape (both chains
/// use commonware-cryptography for threshold signing). The trait is agnostic
/// to which chain produced it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifiedHeader {
    /// Block height (monotone non-decreasing across consecutive verified headers).
    pub height: BlockNumber,
    /// Hex-encoded `0x`-prefixed block hash.
    pub block_hash: String,
    /// Hex-encoded `0x`-prefixed parent hash.
    pub parent_hash: String,
    /// Hex-encoded `0x`-prefixed state root the header commits to.
    pub state_root: String,
    /// Wall-clock timestamp the producer chain stamped on the header.
    pub timestamp_ms: u64,
    /// The validator-quorum signature that authenticates this header.
    pub attestation: AttestationSig,
}

/// A producer-chain validator-quorum signature over a header.
///
/// The trait does not prescribe a signing scheme — Tempo and Daeji both
/// happen to use commonware-cryptography BLS threshold signing today, but a
/// `PoW` chain or single-validator chain would substitute its own primitive
/// here.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttestationSig {
    /// Raw signature bytes (BLS aggregate, ECDSA, schnorr, etc.).
    pub bytes: Vec<u8>,
    /// Validator-quorum identifier — opaque string the backend interprets
    /// (e.g. `"epoch=42,threshold=67/100"`). Recorded so a verifier can
    /// look up the right validator set.
    pub quorum_id: String,
}

/// An account's state at a specific block, with a Merkle proof against the
/// header's `state_root`.
///
/// Holding a `VerifiedHeader` whose `state_root` matches `against_state_root`
/// is what gives the proof its trust. The
/// [`crate::LightClient::verify_account`] method checks the proof structure;
/// callers must additionally confirm the matching header is one they trust.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountProof {
    /// Hex-encoded `0x`-prefixed address the proof is about.
    pub address: String,
    /// Block this state was read at.
    pub block: BlockNumber,
    /// Native-token balance in wei (or chain-native smallest unit).
    pub balance_wei: u128,
    /// Account nonce.
    pub nonce: u64,
    /// Hex-encoded `0x`-prefixed code hash (zero for EOAs).
    pub code_hash: String,
    /// Merkle proof committing the account record into the state trie.
    pub merkle_proof: MerkleProof,
    /// State root the proof must verify against.
    pub against_state_root: String,
}

/// A flat list of trie nodes that, when walked, prove an account or storage
/// slot belongs to a state root.
///
/// The encoding is left to the backend — Phase 0 ships a deterministic mock
/// proof; Phase 1 will use commonware-storage's trie node encoding.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MerkleProof {
    /// Trie nodes from leaf to root. Phase 0 mock simply records a hash of
    /// the canonical account record committed by the seeded state root.
    pub nodes: Vec<Vec<u8>>,
}
