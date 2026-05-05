//! [`TempoLightClient`] — the Tempo-shaped façade over a [`LightClient`]
//! backend.
//!
//! Phase 0 (this PR) wraps a [`crate::mock::MockLightClient`] so the trait
//! surface, the agent integration, and the demo flow can be exercised
//! end-to-end without requiring a Tempo network connection. Phase 1 swaps
//! the inner backend for a real commonware-p2p subscription to a Tempo
//! follower-node behind the `commonware-backend` cargo feature.
//!
//! The struct itself does not change between phases — only the inner
//! `Box<dyn LightClient>` does. That stability is the point: agent code
//! written today against `TempoLightClient` will not need a rewrite when
//! the real network backing lands.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::LcError;
use crate::mock::MockLightClient;
use crate::proof::{AccountProof, VerifiedHeader};
use crate::traits::{BlockNumber, LightClient};

/// A [`LightClient`] specialised to Tempo's chain. Wraps an inner backend
/// (mock today; commonware-p2p in Phase 1) and tags it with a Tempo-specific
/// name for telemetry.
#[derive(Clone)]
pub struct TempoLightClient {
    inner: Arc<dyn LightClient>,
}

impl TempoLightClient {
    /// Construct over an explicit inner backend. Tests use this with a
    /// `MockLightClient`; production code (Phase 1+) will use it with a
    /// commonware-p2p backend.
    pub fn with_backend(inner: Arc<dyn LightClient>) -> Self {
        Self { inner }
    }

    /// Construct a Tempo light-client over the canned demo chain shipped in
    /// [`MockLightClient::tempo_demo`]. Suitable for `examples/tempo_tail.rs`
    /// and integration tests that should run with no network.
    pub fn demo() -> Self {
        Self::with_backend(Arc::new(MockLightClient::tempo_demo()))
    }
}

#[async_trait]
impl LightClient for TempoLightClient {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "tempo"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn demo_round_trip() {
        let lc = TempoLightClient::demo();
        assert_eq!(lc.name(), "tempo");
        // Mock ships with a 200ms tick by default; for the test we don't
        // care about timing — just that the seeded chain advances.
        let h = lc.await_next_header().await.unwrap();
        let proof = lc
            .read_account_at("0x000000000000000000000000000000000000A55E", h.height)
            .await
            .unwrap();
        lc.verify_account(&proof).unwrap();
    }
}
