//! Chain witness helpers for attestation publication and verification.
//!
//! The witness path anchors an attestation fingerprint on-chain by submitting
//! a small transaction and then checking that the mined receipt contains the
//! expected witness payload. The mock backend mirrors this by emitting a log
//! entry that carries the fingerprint bytes.

use crate::{ChainClient, ChainError, ChainResult, ChainWallet, TxHash, TxRequest};
use roko_core::{Attestation, ChainAttestation, ContentHash};

pub(crate) const WITNESS_MARKER: &[u8] = b"roko.attestation.witness:";
pub(crate) const WITNESS_TOPIC: &str = "roko.attestation.witness";
pub(crate) const WITNESS_TO: &str = "0x00000000000000000000000000000000000000c0";
const DEFAULT_RECEIPT_TIMEOUT_MS: u64 = 30_000;

/// Helper for anchoring and checking attestation witnesses on-chain.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ChainWitnessEngine;

impl ChainWitnessEngine {
    /// Human-readable subsystem label.
    pub const LABEL: &'static str = "Chain Witness";
    /// Static marker string for logs and diagnostics.
    pub const MARKER: &'static str = "roko-chain subsystem: witness";

    /// Construct a witness engine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Returns a static marker describing witness behavior.
    #[must_use]
    pub const fn observe(self) -> &'static str {
        Self::MARKER
    }

    /// Submit a witness transaction for `attestation` and attach chain
    /// metadata if the receipt is available.
    pub async fn witness_on_chain(
        self,
        attestation: &mut Attestation,
        wallet: &dyn ChainWallet,
        client: &dyn ChainClient,
    ) -> ChainResult<TxHash> {
        let tx = witness_tx_request(attestation);
        let tx_hash = wallet.sign_and_submit(tx).await?;

        match wallet
            .wait_for_receipt(&tx_hash, DEFAULT_RECEIPT_TIMEOUT_MS)
            .await
        {
            Ok(receipt) => {
                let chain_id = client.chain_id().await?;
                attestation.chain_attestation = Some(ChainAttestation {
                    chain_id,
                    tx_hash: tx_hash_to_bytes(&tx_hash)?,
                    block_number: receipt.block_number,
                });
            }
            Err(ChainError::Unsupported(_) | ChainError::Timeout(_)) => {}
            Err(err) => return Err(err),
        }

        Ok(tx_hash)
    }

    /// Check that `attestation` was witnessed on-chain and the mined receipt
    /// contains the expected witness payload.
    pub async fn verify_on_chain(
        self,
        attestation: &Attestation,
        client: &dyn ChainClient,
    ) -> ChainResult<bool> {
        let Some(chain_attestation) = attestation.chain_attestation.as_ref() else {
            return Ok(false);
        };

        let tx_hash = tx_hash_from_bytes(&chain_attestation.tx_hash);
        let Some(receipt) = client.get_receipt(&tx_hash).await? else {
            return Ok(false);
        };
        if !receipt.status {
            return Ok(false);
        }
        Ok(receipt.logs.iter().any(|log| {
            log.topics.iter().any(|topic| topic == WITNESS_TOPIC)
                && log.data == attestation.witness_hash().0
        }))
    }
}

/// Anchor an attestation on-chain using the default witness engine.
pub async fn witness_on_chain(
    attestation: &mut Attestation,
    wallet: &dyn ChainWallet,
    client: &dyn ChainClient,
) -> ChainResult<TxHash> {
    ChainWitnessEngine::new()
        .witness_on_chain(attestation, wallet, client)
        .await
}

/// Verify an on-chain witness using the default witness engine.
pub async fn verify_on_chain(
    attestation: &Attestation,
    client: &dyn ChainClient,
) -> ChainResult<bool> {
    ChainWitnessEngine::new()
        .verify_on_chain(attestation, client)
        .await
}

fn witness_tx_request(attestation: &Attestation) -> TxRequest {
    let mut data = Vec::with_capacity(WITNESS_MARKER.len() + 32);
    data.extend_from_slice(WITNESS_MARKER);
    data.extend_from_slice(&attestation.witness_hash().0);
    TxRequest {
        to: Some(WITNESS_TO.to_string()),
        from: None,
        value: 0,
        data,
        gas_limit: Some(50_000),
        max_fee_per_gas: None,
        max_priority_fee_per_gas: None,
        nonce: None,
    }
}

fn tx_hash_to_bytes(tx_hash: &TxHash) -> ChainResult<[u8; 32]> {
    tx_hash_from_hex(tx_hash.as_str())
}

fn tx_hash_from_bytes(bytes: &[u8; 32]) -> TxHash {
    TxHash::new(format!("0x{}", hex(bytes)))
}

fn tx_hash_from_hex(value: &str) -> ChainResult<[u8; 32]> {
    let Some(hash) = ContentHash::from_hex(value.trim_start_matches("0x")) else {
        return Err(ChainError::Rpc(format!("invalid tx hash: {value}")));
    };
    Ok(hash.0)
}

fn hex(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::paired_mocks;
    use roko_core::{
        Body, Engram, Kind, Provenance,
        attestation::{SigningKey, sign},
    };

    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    #[tokio::test(flavor = "current_thread")]
    async fn witness_roundtrip_records_chain_attestation() {
        let (client, wallet) = paired_mocks(1_000);
        let engram = Engram::builder(Kind::Task)
            .body(Body::text("anchor this"))
            .provenance(Provenance::trusted("roko"))
            .created_at_ms(0)
            .build();
        let mut attestation = sign(&engram, &signing_key(9));

        let tx_hash = witness_on_chain(&mut attestation, &wallet, &client)
            .await
            .unwrap();
        let chain_attestation = attestation
            .chain_attestation
            .as_ref()
            .expect("chain attestation");
        assert_eq!(tx_hash_from_bytes(&chain_attestation.tx_hash), tx_hash);
        assert!(verify_on_chain(&attestation, &client).await.unwrap());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verify_on_chain_rejects_missing_witness() {
        let (client, _wallet) = paired_mocks(1_000);
        let engram = Engram::builder(Kind::Task).created_at_ms(0).build();
        let attestation = sign(&engram, &signing_key(1));
        assert!(!verify_on_chain(&attestation, &client).await.unwrap());
    }
}
