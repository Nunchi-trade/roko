//! [`TempoLightClient`] — the Tempo-shaped façade over a [`LightClient`]
//! backend.
//!
//! The default constructor [`TempoLightClient::testnet`] connects to the
//! Tempo "Moderato" testnet via JSON-RPC and verifies state proofs locally.
//! [`TempoLightClient::with_rpc`] pins a custom endpoint (devnet, alternate
//! provider, future mainnet). [`TempoLightClient::with_backend`] accepts any
//! [`LightClient`] impl — used by tests with the optional `mock` feature.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::LcError;
use crate::proof::{AccountProof, VerifiedHeader};
use crate::rpc::TempoRpcBackend;
use crate::traits::{BlockNumber, LightClient};
use crate::{MODERATO_CHAIN_ID, MODERATO_RPC_URL};

/// A [`LightClient`] specialised to Tempo's chain.
#[derive(Clone)]
pub struct TempoLightClient {
    inner: Arc<dyn LightClient>,
}

impl TempoLightClient {
    /// Connect to the Tempo "Moderato" public testnet (chain id 42431,
    /// `https://rpc.moderato.tempo.xyz`).
    ///
    /// # Errors
    ///
    /// Returns [`LcError::Backend`] if the testnet is unreachable or returns
    /// an unexpected chain id.
    pub async fn testnet() -> Result<Self, LcError> {
        Self::with_rpc(MODERATO_RPC_URL, MODERATO_CHAIN_ID).await
    }

    /// Connect to a Tempo-compatible RPC endpoint with an explicit expected
    /// chain id. Use this for dev / staging / mainnet (once published).
    ///
    /// # Errors
    ///
    /// Returns [`LcError::Backend`] if the URL is malformed, the endpoint is
    /// unreachable, or its chain id differs from `expected_chain_id`.
    pub async fn with_rpc(rpc_url: &str, expected_chain_id: u64) -> Result<Self, LcError> {
        let backend = TempoRpcBackend::connect(rpc_url, expected_chain_id).await?;
        Ok(Self::with_backend(Arc::new(backend)))
    }

    /// Construct over an explicit inner backend. Useful for tests with the
    /// `mock` feature, or future commonware-p2p backends.
    pub fn with_backend(inner: Arc<dyn LightClient>) -> Self {
        Self { inner }
    }

    /// Build a Tempo light-client over the in-memory mock chain. Available
    /// only when the `mock` cargo feature is enabled — exposed for tests
    /// and offline demos that must run with no network.
    #[cfg(feature = "mock")]
    pub fn mock() -> Self {
        Self::with_backend(Arc::new(crate::mock::MockLightClient::tempo_demo()))
    }
}

#[async_trait]
impl LightClient for TempoLightClient {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn latest_verified(&self) -> Option<VerifiedHeader> {
        self.inner.latest_verified()
    }

    async fn await_next_header(&self) -> Result<VerifiedHeader, LcError> {
        self.inner.await_next_header().await
    }

    async fn read_account_at(
        &self,
        address: &str,
        block: BlockNumber,
    ) -> Result<AccountProof, LcError> {
        self.inner.read_account_at(address, block).await
    }

    fn verify_account(&self, proof: &AccountProof) -> Result<(), LcError> {
        self.inner.verify_account(proof)
    }
}

#[cfg(all(test, feature = "mock"))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_round_trip() {
        let lc = TempoLightClient::mock();
        let h = lc.await_next_header().await.unwrap();
        let proof = lc
            .read_account_at("0x000000000000000000000000000000000000A55E", h.height)
            .await
            .unwrap();
        lc.verify_account(&proof).unwrap();
    }
}
