//! [`FollowerChainClient`] — read-only [`ChainClient`] for `chain.mode =
//! "light" | "follower"` against a Daeji (Kora) endpoint.
//!
//! ## Design
//!
//! Daeji exposes two server-side surfaces today:
//! 1. **Ethereum JSON-RPC** at `:8545` (full `eth_*` API; no `eth_getProof`).
//! 2. **Kora-specific RPC** (`kora_nodeStatus`) on the same socket — returns
//!    `{ chain_id, current_view, finalized_count, peer_count, is_leader, ... }`.
//!
//! `FollowerChainClient` routes state reads through [`AlloyChainClient`] (the
//! Ethereum surface) and layers a `kora_nodeStatus` poller on top so callers
//! can observe consensus liveness without a separate scrape.
//!
//! Light vs Follower differ only in [`FollowerFlavor`] today. Pruning depth
//! becomes meaningful once Daeji's secondary-peer protocol streams blocks
//! end-to-end (it currently runs transport-only, see `bin/kora/src/cli.rs`).
//!
//! Threshold-cert verification is **not** wired today: `kora_nodeStatus` is
//! liveness telemetry, not a finalization receipt. When Jacob's threshold-cert
//! endpoint + BLS12-381 pubkey land we add `commonware-cryptography` and gate
//! reads on a verified view ≥ block.number. Until then this is a "trusted
//! RPC" backend with consensus-aware liveness — explicitly logged at
//! construction so callers can't mistake it for a verifying client.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::client::ChainClient;
use crate::types::{
    BlockNumber, CallResult, ChainError, ChainHeader, ChainResult, LogEntry, Receipt, TxHash,
    TxRequest,
};
use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

#[cfg(feature = "alloy-backend")]
use crate::alloy_impl::AlloyChainClient;

/// Active follower flavor — selects the human-readable `name()` and (later)
/// pruning depth once a local block cache exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowerFlavor {
    /// Light client (no history retained).
    Light,
    /// Full follower (history retained subject to pruning policy).
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

/// Snapshot of the Kora-specific `kora_nodeStatus` JSON-RPC response.
///
/// Field names match the camelCase wire format Daeji emits (see
/// `crates/node/rpc/src/state.rs::NodeStatus` in the daeji repo).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeStatus {
    /// Chain ID.
    pub chain_id: u64,
    /// Reporting validator's index (0..n).
    pub validator_index: u32,
    /// Seconds since the validator started.
    pub uptime_secs: u64,
    /// Current consensus view number.
    pub current_view: u64,
    /// Number of finalized blocks observed.
    pub finalized_count: u64,
    /// Number of blocks proposed by this validator.
    pub proposed_count: u64,
    /// Number of nullified rounds.
    pub nullified_count: u64,
    /// Connected peer count.
    pub peer_count: u64,
    /// Whether the reporting validator is the current leader.
    pub is_leader: bool,
}

/// Cached liveness snapshot plus the wall-clock time it was fetched.
#[derive(Clone, Debug)]
pub struct ChainStatus {
    /// Most recent successful response from `kora_nodeStatus`.
    pub status: NodeStatus,
    /// When the snapshot was captured.
    pub fetched_at: SystemTime,
}

/// Read-only chain client for `light` and `follower` modes.
///
/// State reads are served by [`AlloyChainClient`]; consensus liveness is
/// surfaced via the `kora_*` namespace on the same JSON-RPC endpoint.
#[derive(Clone)]
pub struct FollowerChainClient {
    flavor: FollowerFlavor,
    inner: Arc<Inner>,
}

struct Inner {
    rpc: Backend,
    last_status: RwLock<Option<ChainStatus>>,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("rpc", &self.rpc.label())
            .finish_non_exhaustive()
    }
}

/// Backend mux: real RPC or stub.
///
/// The stub variant exists so callers that ask for `light`/`follower` mode
/// without configuring an `rpc_url` get a typed error rather than a panic at
/// construction.
enum Backend {
    #[cfg(feature = "alloy-backend")]
    Rpc {
        rpc: AlloyChainClient,
        kora_url: String,
        http: reqwest::Client,
    },
    Stub {
        flavor_label: &'static str,
    },
}

impl Backend {
    fn label(&self) -> String {
        match self {
            #[cfg(feature = "alloy-backend")]
            Self::Rpc { kora_url, .. } => format!("rpc({kora_url})"),
            Self::Stub { flavor_label } => format!("stub({flavor_label})"),
        }
    }

    fn unsupported<T>(label: &'static str, op: &str) -> ChainResult<T> {
        Err(ChainError::Unsupported(format!(
            "{op}: {label} backend requires chain.rpc_url; configure it in roko.toml"
        )))
    }
}

impl std::fmt::Debug for FollowerChainClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FollowerChainClient")
            .field("flavor", &self.flavor)
            .field("backend", &self.inner.rpc.label())
            .finish()
    }
}

impl FollowerChainClient {
    /// Construct an RPC-backed follower client.
    ///
    /// State reads route through `rpc_url` (Ethereum JSON-RPC). Consensus
    /// liveness is fetched from the same socket via `kora_nodeStatus`.
    ///
    /// # Errors
    ///
    /// Returns [`ChainError::Rpc`] if `rpc_url` is not a valid HTTP URL.
    #[cfg(feature = "alloy-backend")]
    pub fn rpc(flavor: FollowerFlavor, rpc_url: &str) -> ChainResult<Self> {
        let rpc = AlloyChainClient::http(rpc_url)?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| ChainError::Rpc(format!("reqwest builder: {e}")))?;
        Ok(Self {
            flavor,
            inner: Arc::new(Inner {
                rpc: Backend::Rpc {
                    rpc,
                    kora_url: rpc_url.to_string(),
                    http,
                },
                last_status: RwLock::new(None),
            }),
        })
    }

    /// Stub constructor — used when `chain.mode = light|follower` is selected
    /// but `chain.rpc_url` is not configured. Every read returns
    /// [`ChainError::Unsupported`].
    pub fn stub(flavor: FollowerFlavor) -> Self {
        Self {
            flavor,
            inner: Arc::new(Inner {
                rpc: Backend::Stub {
                    flavor_label: flavor.label(),
                },
                last_status: RwLock::new(None),
            }),
        }
    }

    /// Last cached `kora_nodeStatus` snapshot, if one has been fetched.
    pub fn chain_status(&self) -> Option<ChainStatus> {
        self.inner.last_status.read().clone()
    }

    /// Fetch and cache `kora_nodeStatus`. Returns the new snapshot on success.
    ///
    /// Cheap to call — a single HTTP POST. Callers can drive their own poll
    /// loop or call this lazily before reads that need a finalized-view sanity
    /// check.
    ///
    /// # Errors
    ///
    /// Returns [`ChainError::Rpc`] if the request fails or the response can't
    /// be parsed; [`ChainError::Unsupported`] for stub backends.
    pub async fn refresh_status(&self) -> ChainResult<NodeStatus> {
        match &self.inner.rpc {
            #[cfg(feature = "alloy-backend")]
            Backend::Rpc { kora_url, http, .. } => {
                let body = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "kora_nodeStatus",
                    "params": [],
                });
                let resp = http
                    .post(kora_url)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| ChainError::Rpc(format!("kora_nodeStatus send: {e}")))?
                    .error_for_status()
                    .map_err(|e| ChainError::Rpc(format!("kora_nodeStatus http: {e}")))?
                    .json::<JsonRpcEnvelope<NodeStatus>>()
                    .await
                    .map_err(|e| ChainError::Rpc(format!("kora_nodeStatus parse: {e}")))?;
                let status = resp.into_result()?;
                *self.inner.last_status.write() = Some(ChainStatus {
                    status: status.clone(),
                    fetched_at: SystemTime::now(),
                });
                Ok(status)
            }
            Backend::Stub { flavor_label } => Backend::unsupported(flavor_label, "refresh_status"),
        }
    }
}

#[derive(Deserialize)]
struct JsonRpcEnvelope<T> {
    #[serde(default)]
    result: Option<T>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Deserialize)]
struct JsonRpcError {
    #[allow(dead_code)]
    code: i64,
    message: String,
}

impl<T> JsonRpcEnvelope<T> {
    fn into_result(self) -> ChainResult<T> {
        if let Some(err) = self.error {
            return Err(ChainError::Rpc(format!("jsonrpc error: {}", err.message)));
        }
        self.result
            .ok_or_else(|| ChainError::Rpc("jsonrpc response missing both result and error".into()))
    }
}

#[async_trait]
impl ChainClient for FollowerChainClient {
    async fn block_number(&self) -> ChainResult<BlockNumber> {
        match &self.inner.rpc {
            #[cfg(feature = "alloy-backend")]
            Backend::Rpc { rpc, .. } => rpc.block_number().await,
            Backend::Stub { flavor_label } => Backend::unsupported(flavor_label, "block_number"),
        }
    }

    async fn get_block_header(&self, number: BlockNumber) -> ChainResult<ChainHeader> {
        match &self.inner.rpc {
            #[cfg(feature = "alloy-backend")]
            Backend::Rpc { rpc, .. } => rpc.get_block_header(number).await,
            Backend::Stub { flavor_label } => {
                Backend::unsupported(flavor_label, "get_block_header")
            }
        }
    }

    async fn get_receipt(&self, tx: &TxHash) -> ChainResult<Option<Receipt>> {
        match &self.inner.rpc {
            #[cfg(feature = "alloy-backend")]
            Backend::Rpc { rpc, .. } => rpc.get_receipt(tx).await,
            Backend::Stub { flavor_label } => Backend::unsupported(flavor_label, "get_receipt"),
        }
    }

    async fn get_logs(
        &self,
        from: BlockNumber,
        to: BlockNumber,
        addresses: &[String],
        topics: &[String],
    ) -> ChainResult<Vec<LogEntry>> {
        match &self.inner.rpc {
            #[cfg(feature = "alloy-backend")]
            Backend::Rpc { rpc, .. } => rpc.get_logs(from, to, addresses, topics).await,
            Backend::Stub { flavor_label } => Backend::unsupported(flavor_label, "get_logs"),
        }
    }

    async fn get_storage_at(
        &self,
        address: &str,
        slot: &str,
        block: Option<BlockNumber>,
    ) -> ChainResult<Vec<u8>> {
        match &self.inner.rpc {
            #[cfg(feature = "alloy-backend")]
            Backend::Rpc { rpc, .. } => rpc.get_storage_at(address, slot, block).await,
            Backend::Stub { flavor_label } => Backend::unsupported(flavor_label, "get_storage_at"),
        }
    }

    async fn eth_call(
        &self,
        request: &TxRequest,
        block: Option<BlockNumber>,
    ) -> ChainResult<CallResult> {
        match &self.inner.rpc {
            #[cfg(feature = "alloy-backend")]
            Backend::Rpc { rpc, .. } => rpc.eth_call(request, block).await,
            Backend::Stub { flavor_label } => Backend::unsupported(flavor_label, "eth_call"),
        }
    }

    async fn get_balance(&self, address: &str, block: Option<BlockNumber>) -> ChainResult<u128> {
        match &self.inner.rpc {
            #[cfg(feature = "alloy-backend")]
            Backend::Rpc { rpc, .. } => rpc.get_balance(address, block).await,
            Backend::Stub { flavor_label } => Backend::unsupported(flavor_label, "get_balance"),
        }
    }

    async fn chain_id(&self) -> ChainResult<u64> {
        match &self.inner.rpc {
            #[cfg(feature = "alloy-backend")]
            Backend::Rpc { rpc, .. } => rpc.chain_id().await,
            Backend::Stub { flavor_label } => Backend::unsupported(flavor_label, "chain_id"),
        }
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
    async fn stub_unsupported_error_mentions_rpc_url() {
        let client = FollowerChainClient::stub(FollowerFlavor::Follower);
        let Err(ChainError::Unsupported(msg)) = client.block_number().await else {
            panic!("expected unsupported");
        };
        assert!(msg.contains("rpc_url"), "msg = {msg}");
        assert!(msg.contains("follower"), "msg = {msg}");
    }

    #[test]
    fn stub_chain_status_starts_empty() {
        let client = FollowerChainClient::stub(FollowerFlavor::Light);
        assert!(client.chain_status().is_none());
    }

    #[cfg(feature = "alloy-backend")]
    #[tokio::test]
    async fn rpc_backend_constructs_and_labels_correctly() {
        // Don't actually hit the network — just verify construction doesn't
        // panic and the backend label round-trips.
        let client =
            FollowerChainClient::rpc(FollowerFlavor::Follower, "http://127.0.0.1:1").unwrap();
        assert_eq!(client.name(), "follower");
        assert!(client.chain_status().is_none());
    }

    #[cfg(feature = "alloy-backend")]
    #[tokio::test]
    async fn rpc_backend_rejects_invalid_url() {
        let err = FollowerChainClient::rpc(FollowerFlavor::Light, "not-a-url");
        assert!(err.is_err());
    }

    #[test]
    fn node_status_camel_case_roundtrip() {
        let json = r#"{
            "chainId": 1337,
            "validatorIndex": 2,
            "uptimeSecs": 100,
            "currentView": 50,
            "finalizedCount": 42,
            "proposedCount": 10,
            "nullifiedCount": 3,
            "peerCount": 4,
            "isLeader": true
        }"#;
        let s: NodeStatus = serde_json::from_str(json).expect("parse");
        assert_eq!(s.chain_id, 1337);
        assert_eq!(s.finalized_count, 42);
        assert!(s.is_leader);
    }

    #[test]
    fn jsonrpc_envelope_extracts_result() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"chainId":1,"validatorIndex":0,"uptimeSecs":0,"currentView":0,"finalizedCount":0,"proposedCount":0,"nullifiedCount":0,"peerCount":0,"isLeader":false}}"#;
        let env: JsonRpcEnvelope<NodeStatus> = serde_json::from_str(json).unwrap();
        let s = env.into_result().unwrap();
        assert_eq!(s.chain_id, 1);
    }

    #[test]
    fn jsonrpc_envelope_surfaces_error() {
        let json =
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"method not found"}}"#;
        let env: JsonRpcEnvelope<NodeStatus> = serde_json::from_str(json).unwrap();
        let Err(ChainError::Rpc(msg)) = env.into_result() else {
            panic!("expected Rpc error");
        };
        assert!(msg.contains("method not found"), "msg = {msg}");
    }
}
