//! Cryptographic attestation metadata for [`crate::Engram`].
//!
//! Attestations are optional proofs of origin layered on top of an Engram's
//! content identity. They are intentionally excluded from the content hash so
//! the same Engram can be attested after creation without changing its ID.

use crate::{ContentHash, Engram};
pub use ed25519_dalek::SigningKey;
use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey};
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

impl Attestation {
    /// Hash used when anchoring this attestation to chain logs.
    ///
    /// The witness hash intentionally excludes `chain_attestation` so the
    /// anchored proof stays stable before and after publication.
    #[must_use]
    pub fn witness_hash(&self) -> ContentHash {
        let mut bytes = Vec::with_capacity(64 + 32);
        bytes.extend_from_slice(&self.signature.0);
        bytes.extend_from_slice(&self.public_key.0);
        ContentHash::of(&bytes)
    }

    /// Attach a chain witness to this attestation.
    #[must_use]
    pub fn with_chain_attestation(mut self, chain_attestation: ChainAttestation) -> Self {
        self.chain_attestation = Some(chain_attestation);
        self
    }
}

/// Sign an engram's content hash with Ed25519.
#[must_use]
pub fn sign(engram: &Engram, key: &SigningKey) -> Attestation {
    let hash = engram.content_hash();
    let signature = key.sign(&hash.0);
    Attestation {
        signature: Ed25519Signature(signature.to_bytes()),
        public_key: PublicKey(key.verifying_key().to_bytes()),
        chain_attestation: None,
    }
}

/// Verify that an attestation matches an engram's content hash.
#[must_use]
pub fn verify(engram: &Engram, attestation: &Attestation) -> bool {
    let Ok(public_key) = VerifyingKey::from_bytes(&attestation.public_key.0) else {
        return false;
    };
    let signature = Signature::from_bytes(&attestation.signature.0);
    public_key
        .verify(&engram.content_hash().0, &signature)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Body, Engram, Kind, Provenance};

    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let engram = Engram::builder(Kind::Task)
            .body(Body::text("implement attestation"))
            .provenance(Provenance::trusted("roko"))
            .created_at_ms(0)
            .build();
        let key = signing_key(7);

        let attestation = sign(&engram, &key);
        assert_eq!(attestation.public_key.0, key.verifying_key().to_bytes());
        assert!(verify(&engram, &attestation));
    }

    #[test]
    fn verify_rejects_tampered_content() {
        let base = Engram::builder(Kind::Task)
            .body(Body::text("original"))
            .created_at_ms(0)
            .build();
        let tampered = Engram::builder(Kind::Task)
            .body(Body::text("tampered"))
            .created_at_ms(0)
            .build();
        let key = signing_key(11);

        let attestation = sign(&base, &key);
        assert!(!verify(&tampered, &attestation));
    }

    #[test]
    fn witness_hash_is_stable_across_chain_attachment() {
        let engram = Engram::builder(Kind::Task).created_at_ms(0).build();
        let mut attestation = sign(&engram, &signing_key(3));
        let witness = attestation.witness_hash();
        attestation = attestation.with_chain_attestation(ChainAttestation {
            chain_id: 99,
            tx_hash: [1; 32],
            block_number: 12,
        });
        assert_eq!(witness, attestation.witness_hash());
    }
}
