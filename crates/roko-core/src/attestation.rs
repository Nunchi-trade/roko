//! Cryptographic attestation metadata for [`crate::Engram`].
//!
//! Attestations are optional proofs of origin layered on top of an Engram's
//! content identity. They are intentionally excluded from the content hash so
//! the same Engram can be attested after creation without changing its ID.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A 64-byte Ed25519 signature over an Engram's [`crate::ContentHash`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ed25519Signature(pub [u8; 64]);

impl Serialize for Ed25519Signature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Ed25519Signature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        let len = bytes.len();
        let inner: [u8; 64] = bytes
            .try_into()
            .map_err(|_| serde::de::Error::invalid_length(len, &"64 bytes"))?;
        Ok(Self(inner))
    }
}

/// A 32-byte public key for the signer of an attested Engram.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PublicKey(pub [u8; 32]);

/// On-chain witness that an Engram hash existed on a particular chain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainAttestation {
    /// Chain identifier (for example, Korai mainnet).
    pub chain_id: u64,
    /// Transaction hash containing the anchored content hash.
    pub tx_hash: [u8; 32],
    /// Block number at which the attestation was recorded.
    pub block_number: u64,
}

/// Cryptographic proof that a specific signer produced an Engram.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attestation {
    /// Ed25519 signature over the Engram's content hash.
    pub signature: Ed25519Signature,
    /// Public key of the signer or attesting runtime.
    pub public_key: PublicKey,
    /// Optional chain witness for timestamped publication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_attestation: Option<ChainAttestation>,
}
