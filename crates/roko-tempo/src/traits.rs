//! The chain-agnostic [`LightClient`] trait.

use async_trait::async_trait;

use crate::error::LcError;
use crate::proof::{AccountProof, VerifiedHeader};

/// Block height alias — `u64` so the trait stays free of alloy / commonware
/// type dependencies. Backends convert as needed.
pub type BlockNumber = u64;

/// A trustless, read-only view onto a producer chain.
///
/// Implementations subscribe to the producer chain's verified-header stream
/// and serve account / storage reads at past heights with cryptographic
/// proofs against those headers' state roots. Roko agents wrap a
/// `LightClient` to consume external chain state without trusting an RPC
/// operator — the partnership thesis from the 2026-05-01 Tempo+Oracle call
/// (Jacob: *"when they consume data, that data is proven. There's no
/// potential for fraud."*).
///
/// The trait is **chain-agnostic**: Tempo and Daeji are both expected to be
/// served by the same trait. The backing differs (which p2p mesh, which
/// signing scheme), but the consumer surface is shared. See [`crate`] docs
/// for the phased rollout (mock → commonware-follower → LC peer).
#[async_trait]
pub trait LightClient: Send + Sync {
    /// Backend name (for logs, metrics, telemetry).
    fn name(&self) -> &str;

    /// The most recent header this backend has fully verified, or `None` if
    /// the backend has not observed one yet (cold start).
    fn latest_verified(&self) -> Option<VerifiedHeader>;

    /// Block on the next verified header. Phase 0 mocks tick deterministically;
    /// Phase 1+ backends will pull from the underlying gossip subscription.
    async fn await_next_header(&self) -> Result<VerifiedHeader, LcError>;

    /// Read the account record at `address` at `block`, returning a proof
    /// that commits the record into the block's state root. Callers must
    /// independently confirm the header naming `proof.against_state_root`
    /// is one they trust (typically: hold a [`VerifiedHeader`] whose
    /// `state_root` matches).
    async fn read_account_at(
        &self,
        address: &str,
        block: BlockNumber,
    ) -> Result<AccountProof, LcError>;

    /// Verify a proof's internal structure — that `merkle_proof` walks to
    /// `against_state_root` and the proof is consistent with the claimed
    /// account record.
    ///
    /// Verification of the **header** that named `against_state_root`
    /// (i.e. the BLS attestation check) happens during
    /// [`Self::await_next_header`], not here.
    fn verify_account(&self, proof: &AccountProof) -> Result<(), LcError>;
}
