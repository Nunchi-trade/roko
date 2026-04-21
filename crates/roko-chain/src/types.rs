//! Simplified chain primitives used by [`ChainClient`](crate::ChainClient) and
//! [`ChainWallet`](crate::ChainWallet).
//!
//! These types are deliberately narrow. They capture only what the trait
//! surface needs today — backend-agnostic, no Alloy / revm / k256 coupling.
//! Richer types (e.g. full block, access lists, EIP-4844 blobs) can be added
//! when a concrete backend actually needs them.

use serde::{Deserialize, Serialize};

/// Block number (u64) alias.
pub type BlockNumber = u64;

/// Transaction hash (hex, `0x`-prefixed, 66 chars).
///
/// Stored as a `String` rather than `[u8; 32]` so the mock backends and
/// downstream serialized formats can round-trip unmodified. Validation of
/// length/prefix is the producer's responsibility — real backends will
/// construct these from typed hash values.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TxHash(pub String);

impl TxHash {
    /// Construct a `TxHash` from a displayable value.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Borrow the underlying hex string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TxHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Simplified block header for light-client needs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainHeader {
    /// Block number.
    pub number: BlockNumber,
    /// Block hash (hex, `0x`-prefixed).
    pub hash: String,
    /// Parent block hash.
    pub parent: String,
    /// Unix timestamp (seconds).
    pub timestamp: u64,
}

/// Subset of an `eth_call` result: the returned bytes plus gas used.
#[derive(Clone, Debug)]
pub struct CallResult {
    /// Raw output bytes returned by the call.
    pub output: Vec<u8>,
    /// Gas reported as consumed by the call.
    pub gas_used: u64,
}

/// Simplified transaction request used by [`ChainWallet`](crate::ChainWallet).
///
/// All fields are optional where the wallet can fill them in (nonce, gas
/// limit, fees). `to == None` represents a contract-creation transaction.
#[derive(Clone, Debug, Default)]
pub struct TxRequest {
    /// Destination address (hex, `0x`-prefixed). `None` = contract creation.
    pub to: Option<String>,
    /// Explicit sender (wallet may override).
    pub from: Option<String>,
    /// Value transferred, in wei.
    pub value: u128,
    /// Calldata bytes.
    pub data: Vec<u8>,
    /// Gas limit; wallet estimates if `None`.
    pub gas_limit: Option<u64>,
    /// EIP-1559 max fee per gas.
    pub max_fee_per_gas: Option<u128>,
    /// EIP-1559 max priority fee per gas.
    pub max_priority_fee_per_gas: Option<u128>,
    /// Nonce; wallet assigns from its nonce manager if `None`.
    pub nonce: Option<u64>,
}

/// Simplified receipt returned by a chain backend after a tx is mined.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    /// Hash of the transaction this receipt belongs to.
    pub tx_hash: TxHash,
    /// `true` if the tx succeeded (EVM status = 1).
    pub status: bool,
    /// Block number the tx was included in.
    pub block_number: BlockNumber,
    /// Gas used by the transaction.
    pub gas_used: u64,
    /// Logs emitted during execution.
    pub logs: Vec<LogEntry>,
}

/// A single event log entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Address that emitted the log.
    pub address: String,
    /// Indexed topics (`0x`-prefixed hex).
    pub topics: Vec<String>,
    /// Non-indexed data bytes.
    pub data: Vec<u8>,
}

/// Errors returned by chain backends.
///
/// Intentionally distinct from
/// [`RokoError`](roko_core::error::RokoError): chain operations surface a
/// richer, domain-specific error space (nonce gaps, offline RPCs,
/// insufficient funds) that does not collapse cleanly into the kernel
/// error. Callers that need a `RokoError` can map via
/// `RokoError::substrate(err)` or similar at the crate boundary.
#[derive(Clone, Debug, thiserror::Error)]
pub enum ChainError {
    /// Generic RPC-layer failure (HTTP error, JSON-RPC error, …).
    #[error("rpc error: {0}")]
    Rpc(String),
    /// An operation exceeded its timeout.
    #[error("timeout: {0}")]
    Timeout(String),
    /// No RPC endpoint was reachable.
    #[error("offline; no reachable RPC")]
    Offline,
    /// Wallet does not have enough balance for the requested transaction.
    #[error("insufficient funds (have {have}, need {need})")]
    InsufficientFunds {
        /// Balance available to the wallet.
        have: u128,
        /// Amount required by the transaction.
        need: u128,
    },
    /// Submitted nonce did not match the expected next nonce.
    #[error("nonce gap: expected {expected}, got {got}")]
    NonceGap {
        /// Nonce the backend expected next.
        expected: u64,
        /// Nonce actually provided.
        got: u64,
    },
    /// Input address was malformed.
    #[error("invalid address: {0}")]
    InvalidAddress(String),
    /// Operation is not supported by this backend.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

/// Convenience alias for a `Result` with a [`ChainError`].
pub type ChainResult<T> = Result<T, ChainError>;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn txhash_serde_roundtrip() {
        let h = TxHash::new("0xabc");
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(json, "\"0xabc\"");
        let back: TxHash = serde_json::from_str(&json).unwrap();
        assert_eq!(back, h);
    }

    #[test]
    fn txhash_display_and_as_str() {
        let h = TxHash::new("0xdeadbeef");
        assert_eq!(h.as_str(), "0xdeadbeef");
        assert_eq!(format!("{h}"), "0xdeadbeef");
    }

    #[test]
    fn chain_error_display_messages() {
        let e = ChainError::Rpc("boom".into());
        assert!(format!("{e}").contains("boom"));

        let e = ChainError::InsufficientFunds { have: 1, need: 2 };
        let s = format!("{e}");
        assert!(s.contains("have 1"));
        assert!(s.contains("need 2"));

        let e = ChainError::NonceGap {
            expected: 5,
            got: 7,
        };
        let s = format!("{e}");
        assert!(s.contains("expected 5"));
        assert!(s.contains("got 7"));

        let e = ChainError::Offline;
        assert!(format!("{e}").contains("offline"));
    }

    #[test]
    fn tx_request_default_is_empty() {
        let r = TxRequest::default();
        assert!(r.to.is_none());
        assert!(r.from.is_none());
        assert_eq!(r.value, 0);
        assert!(r.data.is_empty());
        assert!(r.nonce.is_none());
    }

    #[test]
    fn receipt_serde_roundtrip() {
        let r = Receipt {
            tx_hash: TxHash::new("0x01"),
            status: true,
            block_number: 42,
            gas_used: 21_000,
            logs: vec![LogEntry {
                address: "0xcafe".into(),
                topics: vec!["0xaa".into()],
                data: vec![1, 2, 3],
            }],
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: Receipt = serde_json::from_str(&j).unwrap();
        assert_eq!(back.tx_hash, r.tx_hash);
        assert_eq!(back.block_number, 42);
        assert_eq!(back.logs.len(), 1);
        assert_eq!(back.logs[0].data, vec![1, 2, 3]);
    }
}
