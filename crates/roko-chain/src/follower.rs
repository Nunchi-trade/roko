//! [`FollowerChainClient`] — read-only [`ChainClient`] backed by a Commonware
//! `alto-follower` subprocess.
//!
//! The same backend serves both `chain.mode = "light"` and `chain.mode =
//! "follower"`. The only difference between the two modes is the follower's
//! `pruning_depth` config — `0` keeps no history (light client), `None` keeps
//! everything (full follower). The Roko trait surface is identical either way,
//! so callers don't need to know which mode is active.
//!
//! ## Phase A status
//!
//! This crate currently ships the **skeleton**: the type implements
//! [`ChainClient`] but every method returns
//! [`ChainError::Unsupported`](crate::ChainError::Unsupported). Phase B (when
//! Jacob's indexer URL + threshold pubkey land for Daeji) wires the actual
//! subprocess + state-read logic. Until then `roko run --chain-mode light` is
//! safe to invoke — it just won't satisfy `chain.*` tool calls.
//!
//! See `~/.claude/plans/greedy-moseying-cerf.md` for the full plan.

use crate::client::ChainClient;
use crate::types::{
    BlockNumber, CallResult, ChainError, ChainHeader, ChainResult, LogEntry, Receipt, TxHash,
    TxRequest,
};
use async_trait::async_trait;

/// Active follower flavor — selects the human-readable `name()` and (in Phase
/// B) the `pruning_depth` passed to the subprocess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowerFlavor {
    /// Light client (no history; `pruning_depth = 0`).
    Light,
    /// Full follower (history retained per `pruning_depth`).
    Follower,
}

impl FollowerFlavor {
    fn label(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Follower => "follower",
        }
    }
}

/// Read-only chain client backed by an `alto-follower` subprocess.
///
/// Cheap to clone — actual state lives behind the subprocess handle (Phase B).
#[derive(Clone, Debug)]
pub struct FollowerChainClient {
    flavor: FollowerFlavor,
}

impl FollowerChainClient {
    /// Construct a stub client. Phase A only — Phase B replaces with a
    /// constructor that takes a `FollowerSupervisor::Handle`.
    pub fn stub(flavor: FollowerFlavor) -> Self {
        Self { flavor }
    }

    fn unsupported<T>(&self, op: &str) -> ChainResult<T> {
        Err(ChainError::Unsupported(format!(
            "{op}: {} backend not yet wired (Phase B)",
            self.flavor.label()
        )))
    }
}

#[async_trait]
impl ChainClient for FollowerChainClient {
    async fn block_number(&self) -> ChainResult<BlockNumber> {
        self.unsupported("block_number")
    }

    async fn get_block_header(&self, _number: BlockNumber) -> ChainResult<ChainHeader> {
        self.unsupported("get_block_header")
    }

    async fn get_receipt(&self, _tx: &TxHash) -> ChainResult<Option<Receipt>> {
        self.unsupported("get_receipt")
    }

    async fn get_logs(
        &self,
        _from: BlockNumber,
        _to: BlockNumber,
        _addresses: &[String],
        _topics: &[String],
    ) -> ChainResult<Vec<LogEntry>> {
        self.unsupported("get_logs")
    }

    async fn get_storage_at(
        &self,
        _address: &str,
        _slot: &str,
        _block: Option<BlockNumber>,
    ) -> ChainResult<Vec<u8>> {
        self.unsupported("get_storage_at")
    }

    async fn eth_call(
        &self,
        _request: &TxRequest,
        _block: Option<BlockNumber>,
    ) -> ChainResult<CallResult> {
        self.unsupported("eth_call")
    }

    async fn get_balance(&self, _address: &str, _block: Option<BlockNumber>) -> ChainResult<u128> {
        self.unsupported("get_balance")
    }

    async fn chain_id(&self) -> ChainResult<u64> {
        self.unsupported("chain_id")
    }

    fn name(&self) -> &str {
        match self.flavor {
            FollowerFlavor::Light => "light",
            FollowerFlavor::Follower => "follower",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stub_returns_unsupported_for_every_method() {
        let client = FollowerChainClient::stub(FollowerFlavor::Light);
        assert!(matches!(
            client.block_number().await,
            Err(ChainError::Unsupported(_))
        ));
        assert!(matches!(
            client.chain_id().await,
            Err(ChainError::Unsupported(_))
        ));
        assert!(matches!(
            client.get_balance("0xabc", None).await,
            Err(ChainError::Unsupported(_))
        ));
    }

    #[test]
    fn name_reflects_flavor() {
        assert_eq!(
            FollowerChainClient::stub(FollowerFlavor::Light).name(),
            "light"
        );
        assert_eq!(
            FollowerChainClient::stub(FollowerFlavor::Follower).name(),
            "follower"
        );
    }

    #[tokio::test]
    async fn unsupported_error_carries_flavor_label() {
        let client = FollowerChainClient::stub(FollowerFlavor::Follower);
        let Err(ChainError::Unsupported(msg)) = client.block_number().await else {
            panic!("expected unsupported");
        };
        assert!(msg.contains("follower"));
        assert!(msg.contains("Phase B"));
    }
}
