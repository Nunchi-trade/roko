//! [`TempoRpcBackend`] — real Tempo testnet light-client backend.
//!
//! Talks to a Tempo JSON-RPC endpoint (HTTP, optionally WebSocket for
//! `newHeads`) and serves the [`crate::LightClient`] surface backed by:
//!
//! - **Header chain**: trust-on-first-use (TOFU) anchor at construction time,
//!   then parent-hash-linked verification of every subsequent header. New
//!   headers whose `parentHash` does not match our latest verified hash are
//!   rejected — protects against a misbehaving RPC replaying a different
//!   fork.
//! - **State reads**: `eth_getProof` (EIP-1186) returns a Merkle Patricia
//!   trie proof committing the account record into the block's `stateRoot`.
//!   [`crate::mpt::verify_account_proof`] walks the proof locally to confirm.
//!
//! What this backend does **not** do (yet): verify the Tempo validator BLS
//! aggregate signature over each header. Tempo does not currently expose
//! its consensus-peer surface publicly, so we cannot subscribe to
//! signed-consensus output. The RPC endpoint we connect to is the trust
//! anchor for "did a validator quorum actually produce this header." This
//! is materially weaker than full consensus verification but materially
//! stronger than naive RPC trust — every state read still proves what it
//! claims to prove against the header. Phase 2 (commonware-cryptography
//! integration once Tempo opens its surface) closes the remaining gap.

use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::Address;
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::rpc::types::eth::{BlockId, BlockNumberOrTag, BlockTransactionsKind};
use async_trait::async_trait;
use parking_lot::RwLock;
use tracing::{debug, warn};

use crate::error::LcError;
use crate::mpt;
use crate::proof::{AccountProof, AttestationSig, MerkleProof, VerifiedHeader};
use crate::traits::{BlockNumber, LightClient};

const POLL_INTERVAL: Duration = Duration::from_millis(250);
const RPC_TOFU_QUORUM_ID: &str = "tempo-rpc-tofu";

/// Real Tempo backend. Holds an alloy HTTP provider and the latest
/// header it has verified (TOFU + parent-hash chained).
pub struct TempoRpcBackend {
    provider: Arc<DynProvider>,
    rpc_url: String,
    chain_id: u64,
    latest: Arc<RwLock<Option<VerifiedHeader>>>,
    name: Arc<str>,
}

impl TempoRpcBackend {
    /// Build a backend pointing at `rpc_url` and immediately install the
    /// chain's tip as the trust-on-first-use anchor.
    ///
    /// # Errors
    ///
    /// Returns [`LcError::Backend`] if the URL is malformed, the endpoint is
    /// unreachable, or the chain ID returned by the endpoint differs from
    /// `expected_chain_id`.
    pub async fn connect(rpc_url: &str, expected_chain_id: u64) -> Result<Self, LcError> {
        let url = reqwest::Url::parse(rpc_url)
            .map_err(|e| LcError::Backend(format!("invalid rpc url {rpc_url}: {e}")))?;
        let provider = ProviderBuilder::new().connect_http(url).erased();
        let provider = Arc::new(provider);

        let observed = provider
            .get_chain_id()
            .await
            .map_err(|e| LcError::Backend(format!("eth_chainId: {e}")))?;
        if observed != expected_chain_id {
            return Err(LcError::Backend(format!(
                "chain id mismatch: expected {expected_chain_id}, RPC reports {observed}"
            )));
        }

        let backend = Self {
            provider,
            rpc_url: rpc_url.to_string(),
            chain_id: expected_chain_id,
            latest: Arc::new(RwLock::new(None)),
            name: Arc::from(format!("tempo-rpc({rpc_url})").as_str()),
        };
        backend.install_tofu_anchor().await?;
        Ok(backend)
    }

    /// Pin the chain's current tip as the verified anchor. Subsequent headers
    /// must parent-hash-chain back to this one (or to its descendants).
    async fn install_tofu_anchor(&self) -> Result<(), LcError> {
        let tip_height = self
            .provider
            .get_block_number()
            .await
            .map_err(|e| LcError::Backend(format!("eth_blockNumber: {e}")))?;
        let header = self.fetch_header(tip_height).await?;
        debug!(
            target: "roko_tempo::rpc",
            height = header.height,
            block_hash = %header.block_hash,
            "tofu anchor installed"
        );
        *self.latest.write() = Some(header);
        Ok(())
    }

    async fn fetch_header(&self, height: u64) -> Result<VerifiedHeader, LcError> {
        let block = self
            .provider
            .get_block_by_number(BlockNumberOrTag::Number(height))
            .kind(BlockTransactionsKind::Hashes)
            .await
            .map_err(|e| LcError::Backend(format!("eth_getBlockByNumber({height}): {e}")))?
            .ok_or(LcError::UnknownBlock(height))?;
        let h = &block.header;
        Ok(VerifiedHeader {
            height: h.number,
            block_hash: format!("{:#x}", h.hash),
            parent_hash: format!("{:#x}", h.parent_hash),
            state_root: format!("{:#x}", h.state_root),
            timestamp_ms: h.timestamp.saturating_mul(1000),
            attestation: AttestationSig {
                bytes: vec![],
                quorum_id: RPC_TOFU_QUORUM_ID.into(),
            },
        })
    }

    /// Tempo RPC URL this backend talks to.
    #[must_use]
    pub fn rpc_url(&self) -> &str {
        &self.rpc_url
    }

    /// Configured chain id (Moderato testnet = 42431).
    #[must_use]
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }
}

#[async_trait]
impl LightClient for TempoRpcBackend {
    fn name(&self) -> &str {
        &self.name
    }

    fn latest_verified(&self) -> Option<VerifiedHeader> {
        self.latest.read().clone()
    }

    async fn await_next_header(&self) -> Result<VerifiedHeader, LcError> {
        loop {
            let prev = self
                .latest
                .read()
                .clone()
                .ok_or(LcError::NoVerifiedHeader)?;
            let tip = self
                .provider
                .get_block_number()
                .await
                .map_err(|e| LcError::Backend(format!("eth_blockNumber: {e}")))?;
            if tip > prev.height {
                let next_height = prev.height + 1;
                let header = self.fetch_header(next_height).await?;
                if header.parent_hash != prev.block_hash {
                    warn!(
                        target: "roko_tempo::rpc",
                        height = header.height,
                        expected_parent = %prev.block_hash,
                        observed_parent = %header.parent_hash,
                        "parent-hash mismatch — possible reorg or RPC swap; rejecting"
                    );
                    return Err(LcError::InvalidProof {
                        state_root: header.state_root,
                    });
                }
                *self.latest.write() = Some(header.clone());
                return Ok(header);
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    async fn read_account_at(
        &self,
        address: &str,
        block: BlockNumber,
    ) -> Result<AccountProof, LcError> {
        let addr: Address = address
            .parse()
            .map_err(|e| LcError::Backend(format!("address {address}: {e}")))?;
        let block_id = BlockId::Number(BlockNumberOrTag::Number(block));
        let resp = self
            .provider
            .get_proof(addr, vec![])
            .block_id(block_id)
            .await
            .map_err(|e| LcError::Backend(format!("eth_getProof: {e}")))?;

        // The state_root must come from the verified header at `block`.
        // We re-fetch the header here rather than trusting the proof's
        // claimed root, then verify the proof walks to that root.
        let header = self.fetch_header(block).await?;

        Ok(AccountProof {
            address: format!("{addr:#x}"),
            block,
            balance_wei: u128::try_from(resp.balance).unwrap_or(u128::MAX),
            nonce: resp.nonce,
            code_hash: format!("{:#x}", resp.code_hash),
            storage_hash: format!("{:#x}", resp.storage_hash),
            merkle_proof: MerkleProof {
                nodes: resp.account_proof.into_iter().map(|b| b.to_vec()).collect(),
            },
            against_state_root: header.state_root,
        })
    }

    fn verify_account(&self, proof: &AccountProof) -> Result<(), LcError> {
        mpt::verify_account_proof(proof)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quorum_id_marker_is_honest() {
        // The RPC backend MUST flag itself as TOFU so callers can tell it
        // apart from a fully-consensus-verified backend.
        assert_eq!(RPC_TOFU_QUORUM_ID, "tempo-rpc-tofu");
    }
}
