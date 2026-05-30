//! Error types for the light-client trait surface.

use thiserror::Error;

/// Errors a [`crate::LightClient`] can surface to callers.
#[derive(Error, Debug)]
pub enum LcError {
    /// The backend does not yet have a verified header (e.g. cold start).
    #[error("no verified header yet")]
    NoVerifiedHeader,

    /// The requested block is unknown to this backend.
    #[error("unknown block height {0}")]
    UnknownBlock(u64),

    /// The requested account is unknown at the requested block.
    #[error("unknown account {address} at block {block}")]
    UnknownAccount {
        /// Hex-encoded address that was looked up.
        address: String,
        /// Block height the lookup targeted.
        block: u64,
    },

    /// A Merkle proof did not verify against the expected state root.
    #[error("merkle proof did not verify against state root {state_root}")]
    InvalidProof {
        /// State root the proof was checked against.
        state_root: String,
    },

    /// A header attestation signature did not verify against the validator
    /// quorum at the expected epoch.
    #[error("attestation signature did not verify (quorum {quorum_id})")]
    InvalidAttestation {
        /// Validator quorum identifier the attestation claimed.
        quorum_id: String,
    },

    /// Catch-all for backend-specific failures.
    #[error("backend error: {0}")]
    Backend(String),
}
