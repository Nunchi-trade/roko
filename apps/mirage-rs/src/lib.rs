//! `mirage-rs` — in-process fork state, JSON-RPC surface, and integration client.
//!
//! **Plan 03 regression anchors:** `bash plans/context/verify-chains/03-verify.sh` (each `inv-*` step
//! runs a filtered `cargo test -p mirage-rs --lib` for the named `#[test]`).

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(clippy, allow(missing_docs))]
// TODO(UX42-followup): remove `missing_panics_doc` once the remaining
// public Mirage APIs have accurate panic documentation.
#![allow(
    clippy::double_must_use,
    clippy::expect_used,
    clippy::format_push_string,
    clippy::cast_precision_loss,
    clippy::implicit_hasher,
    clippy::manual_clamp,
    clippy::match_same_arms,
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    clippy::cast_sign_loss,
    clippy::clone_on_ref_ptr,
    clippy::assigning_clones,
    clippy::missing_const_for_fn,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_ref_mut,
    clippy::needless_pass_by_value,
    clippy::needless_range_loop,
    clippy::redundant_clone,
    clippy::similar_names,
    clippy::unusual_byte_groupings,
    clippy::unnecessary_map_or,
    clippy::use_self,
    clippy::doc_markdown,
    clippy::needless_continue,
    clippy::map_unwrap_or,
    clippy::option_if_let_else,
    clippy::unnecessary_wraps,
    clippy::redundant_closure_for_method_calls,
    clippy::iter_without_into_iter,
    clippy::used_underscore_binding,
    clippy::suboptimal_flops,
    clippy::ignored_unit_patterns,
    clippy::manual_let_else,
    clippy::too_many_lines,
    clippy::significant_drop_tightening,
    clippy::ref_option,
    clippy::or_fun_call,
    clippy::unnested_or_patterns,
    clippy::type_complexity,
    clippy::iter_over_hash_type,
    clippy::explicit_iter_loop,
    clippy::cloned_instead_of_copied,
    clippy::nonminimal_bool
)]

use alloy_primitives::{Address, B256, Bytes, U256, keccak256};
use revm::database_interface::DBErrorMarker;
use serde::{Deserialize, Deserializer};

pub mod cow;
pub mod events;
pub mod fork;
pub mod integration;
pub mod provider;
pub mod rate_limit;
pub mod replay;
pub mod resources;
pub mod rpc;
pub mod scenario;

/// Chain extensions: HDC-indexed knowledge, stigmergy, and agent-coordination primitives.
///
/// Opt-in via the `chain` cargo feature. See [`chain`] for the module tree.
#[cfg(feature = "chain")]
pub mod chain;

/// Custom revm precompiles (HDC at `0xA0C`). Gated behind the `chain` feature.
pub mod precompiles;

/// JSON-RPC bindings for the chain extensions.
///
/// Registered on the mirage server when a [`chain_rpc::ChainContext`] is
/// provided at startup.
#[cfg(feature = "chain")]
pub mod chain_rpc;

/// HTTP REST API for dashboard consumption.
///
/// Covers the pheromone field, knowledge graph, and agent topology surfaces.
/// Opt-in via the `chain` feature alongside HDC/knowledge/stigmergy.
#[cfg(feature = "legacy-api")]
pub mod http_api;

/// Periodic atomic disk snapshots for state persistence across restarts.
pub mod persist;

/// Roko trait bridge: implements `roko_core::{Gate, Substrate}` over mirage.
#[cfg(feature = "roko")]
pub mod roko_bridge;

pub use cow::{BytecodeCache, CowState, MultiVersionStore, VersionEntry};
pub use fork::{
    Classification, ClassificationConfig, DiffClassifier, DirtyAccount, DirtyStore, ForkState,
    HybridDB, MirageFork, MirageStatus, ReadCache, WatchEntry, WatchSource,
};
pub use integration::{
    EventFilter, EventSource, MirageClient, MirageConfig, MirageEvent, MirageTestInstance,
    PositionRequest, PositionSnapshot, spawn_mirage_test_instance,
};
pub use provider::{BlockTag, UpstreamRpc};
pub use replay::{
    AccountDiff, FollowerConfig, LogEntry, SpeculativeExecutor, SpeculativeResult, StateDiff,
    TargetedFollower, TxReplay,
};
pub use resources::{MirageMode, Profile, ResourceModel, ResourceUsage};
pub use scenario::{
    JobStatus, RunMode, Scenario, ScenarioAssertions, ScenarioJob, ScenarioResult, ScenarioRunner,
    ScenarioSet, ScenarioSetStatus, ScenarioStatus, is_terminal_scenario_status,
    scenario_status_transition_valid,
};

/// Result alias for mirage operations.
pub type Result<T> = std::result::Result<T, MirageError>;

/// Transaction request accepted by the RPC server and client.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionRequest {
    /// Sender address.
    pub from: Option<Address>,
    /// Destination address.
    pub to: Option<Address>,
    /// Gas limit.
    pub gas: Option<u64>,
    /// Transferred value.
    pub value: Option<U256>,
    /// Input data.
    pub data: Option<Bytes>,
    /// Legacy gas price.
    pub gas_price: Option<u128>,
    /// Nonce.
    pub nonce: Option<u64>,
    /// Chain ID.
    pub chain_id: Option<u64>,
}

// Custom deserializer: some clients (alloy, viem) send BOTH `data` and `input`
// with the same payload, which trips serde's rename_alias. We accept either or
// both; if both are present, they must match.
impl<'de> serde::Deserialize<'de> for TransactionRequest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;

        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            from: Option<Address>,
            to: Option<Address>,
            #[serde(default, deserialize_with = "deserialize_optional_u64")]
            gas: Option<u64>,
            value: Option<U256>,
            #[serde(default)]
            data: Option<Bytes>,
            #[serde(default)]
            input: Option<Bytes>,
            #[serde(default, deserialize_with = "deserialize_optional_u128")]
            gas_price: Option<u128>,
            #[serde(default, deserialize_with = "deserialize_optional_u64")]
            nonce: Option<u64>,
            #[serde(default, deserialize_with = "deserialize_optional_u64")]
            chain_id: Option<u64>,
        }

        let Raw {
            from,
            to,
            gas,
            value,
            data,
            input,
            gas_price,
            nonce,
            chain_id,
        } = Raw::deserialize(deserializer)?;

        let data = match (data, input) {
            (Some(d), Some(i)) if d != i => {
                return Err(D::Error::custom(
                    "transaction request contains both `data` and `input` with different values",
                ));
            }
            (Some(d), _) | (None, Some(d)) => Some(d),
            (None, None) => None,
        };

        Ok(Self {
            from,
            to,
            gas,
            value,
            data,
            gas_price,
            nonce,
            chain_id,
        })
    }
}

fn deserialize_optional_u64<'de, D>(deserializer: D) -> std::result::Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_optional_numeric(deserializer, |text| {
        let value = parse_numeric_quantity(text)?;
        u64::try_from(value).map_err(|_| "numeric value out of range".to_owned())
    })
}

fn deserialize_optional_u128<'de, D>(deserializer: D) -> std::result::Result<Option<u128>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_optional_numeric(deserializer, parse_numeric_quantity)
}

fn deserialize_optional_numeric<'de, D, T, F>(
    deserializer: D,
    parse_text: F,
) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    F: FnOnce(&str) -> std::result::Result<T, String> + Copy,
    T: TryFrom<u64>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumericValue {
        Number(u64),
        Text(String),
        Null,
    }

    match Option::<NumericValue>::deserialize(deserializer)? {
        None | Some(NumericValue::Null) => Ok(None),
        Some(NumericValue::Number(value)) => T::try_from(value)
            .map(Some)
            .map_err(|_| serde::de::Error::custom("numeric value out of range")),
        Some(NumericValue::Text(text)) => parse_text(&text)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

fn parse_numeric_quantity(text: &str) -> std::result::Result<u128, String> {
    if let Some(hex) = text.strip_prefix("0x") {
        u128::from_str_radix(hex, 16).map_err(|error| error.to_string())
    } else {
        text.parse::<u128>().map_err(|error| error.to_string())
    }
}

/// Simplified bytecode wrapper.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Bytecode(Bytes);

impl Bytecode {
    /// Creates bytecode from raw bytes.
    #[must_use]
    pub fn new_raw(bytes: Bytes) -> Self {
        Self(bytes)
    }

    /// Returns the bytecode hash.
    #[must_use]
    pub fn hash_slow(&self) -> B256 {
        keccak256(&self.0)
    }

    /// Returns the underlying bytes.
    #[must_use]
    pub fn bytecode(&self) -> &Bytes {
        &self.0
    }
}

/// Simplified account information used by the lazy fork.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    /// Account balance.
    pub balance: U256,
    /// Account nonce.
    pub nonce: u64,
    /// Code hash.
    pub code_hash: B256,
    /// Contract bytecode.
    pub code: Option<Bytecode>,
}

/// Simplified execution result used by the fallback executor.
#[derive(Debug, Clone, Default)]
pub struct ExecutionResult {
    /// Whether the execution succeeded.
    pub success: bool,
    /// Gas used.
    pub gas_used: u64,
    /// Output bytes.
    pub output: Bytes,
}

/// Shared error surface for `mirage-rs` library code.
#[derive(Debug, thiserror::Error)]
pub enum MirageError {
    /// Invalid JSON-RPC parameters or malformed local API input.
    #[error("invalid params: {0}")]
    InvalidParams(String),
    /// Unsupported operation for the current simplified fork state.
    #[error("unsupported operation: {0}")]
    Unsupported(String),
    /// Referenced account cannot be used as the transaction sender.
    #[error("invalid from address: {0}")]
    InvalidFrom(Address),
    /// Requested snapshot ID does not exist or was already consumed.
    #[error("snapshot not found: {0}")]
    SnapshotNotFound(u64),
    /// Requested scenario set does not exist.
    #[error("scenario set not found: {0}")]
    SetNotFound(String),
    /// Scenario set is already executing or otherwise cannot be started again yet.
    #[error("scenario set already running: {0}")]
    SetAlreadyRunning(String),
    /// Scenario set has no scenarios defined.
    #[error("scenario set has no scenarios: {0}")]
    SetHasNoScenarios(String),
    /// Requested scenario job does not exist.
    #[error("scenario job not found: {0}")]
    JobNotFound(String),
    /// Requested scenario job has not completed yet.
    #[error("scenario job not complete: {0}")]
    JobNotComplete(String),
    /// The requested protocol type is not supported by the position helper.
    #[error("unknown protocol type: {0}")]
    UnknownProtocolType(String),
    /// ERC-20 slot detection failed for the requested token/account pair.
    #[error("ERC-20 balance slot detection failed for token {0}")]
    SlotDetectionFailed(Address),
    /// Target address is already tracked and the watch list is at capacity.
    #[error("watch list full")]
    WatchListFull,
    /// Upstream RPC request failed.
    #[error("upstream RPC error: {0}")]
    Upstream(String),
    /// Local bind failed.
    #[error("failed to bind mirage on port {0}")]
    BindFailed(u16),
    /// A time-bound operation exceeded its timeout.
    #[error("operation timed out: {0}")]
    Timeout(String),
    /// I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// HTTP client failure.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    /// Background blocking task failure.
    #[error("background task failed: {0}")]
    BackgroundTask(String),
    /// JSON serialization or parsing failure.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// TOML parsing failure.
    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),
}

impl DBErrorMarker for MirageError {}

impl MirageError {
    /// Returns the JSON-RPC error code for this failure.
    #[must_use]
    pub const fn rpc_code(&self) -> i32 {
        match self {
            Self::InvalidParams(_) => -32602,
            Self::Unsupported(_) => -32603,
            Self::InvalidFrom(_) => -32010,
            Self::SnapshotNotFound(_) => -32001,
            Self::SetNotFound(_) => -32050,
            Self::SetAlreadyRunning(_) => -32051,
            Self::SetHasNoScenarios(_) => -32052,
            Self::JobNotFound(_) => -32054,
            Self::JobNotComplete(_) => -32055,
            Self::UnknownProtocolType(_) => -32040,
            Self::SlotDetectionFailed(_) => -32020,
            Self::WatchListFull => -32030,
            Self::Upstream(_) => -32099,
            Self::BindFailed(_)
            | Self::Timeout(_)
            | Self::BackgroundTask(_)
            | Self::Io(_)
            | Self::Http(_)
            | Self::Json(_)
            | Self::Toml(_) => -32603,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn test_mirage_error_rpc_codes() {
        let addr = Address::ZERO;

        assert_eq!(MirageError::SnapshotNotFound(1).rpc_code(), -32001);
        assert_eq!(MirageError::InvalidFrom(addr).rpc_code(), -32010);
        assert_eq!(MirageError::SlotDetectionFailed(addr).rpc_code(), -32020);
        assert_eq!(MirageError::WatchListFull.rpc_code(), -32030);
        assert_eq!(
            MirageError::UnknownProtocolType("x".into()).rpc_code(),
            -32040
        );
        assert_eq!(MirageError::SetNotFound("s".into()).rpc_code(), -32050);
        assert_eq!(
            MirageError::SetAlreadyRunning("s".into()).rpc_code(),
            -32051
        );
        assert_eq!(
            MirageError::SetHasNoScenarios("s".into()).rpc_code(),
            -32052
        );
        assert_eq!(MirageError::JobNotFound("j".into()).rpc_code(), -32054);
        assert_eq!(MirageError::JobNotComplete("j".into()).rpc_code(), -32055);
        assert_eq!(MirageError::Upstream("err".into()).rpc_code(), -32099);
    }

    /// Plan V2: `-32601` is reserved for JSON-RPC "method not found" from the transport;
    /// domain failures use Mirage-specific bands or `-32602`/`-32603`.
    #[test]
    fn test_mirage_error_rpc_code_never_reserved_method_not_found() {
        let addr = Address::ZERO;
        let http_err = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(200))
            .build()
            .expect("client")
            .get("http://127.0.0.1:1/")
            .send()
            .expect_err("connection to closed port should fail");

        let mut cases = vec![
            MirageError::InvalidParams("p".into()),
            MirageError::Unsupported("u".into()),
            MirageError::InvalidFrom(addr),
            MirageError::SnapshotNotFound(1),
            MirageError::SetNotFound("s".into()),
            MirageError::SetAlreadyRunning("s".into()),
            MirageError::SetHasNoScenarios("s".into()),
            MirageError::JobNotFound("j".into()),
            MirageError::JobNotComplete("j".into()),
            MirageError::UnknownProtocolType("x".into()),
            MirageError::SlotDetectionFailed(addr),
            MirageError::WatchListFull,
            MirageError::Upstream("e".into()),
            MirageError::BindFailed(8545),
            MirageError::Timeout("t".into()),
            MirageError::BackgroundTask("b".into()),
            MirageError::Io(std::io::Error::other("io")),
            MirageError::Json(serde_json::from_str::<serde_json::Value>("").unwrap_err()),
            MirageError::Toml(toml::from_str::<toml::Value>("[").unwrap_err()),
        ];
        cases.push(MirageError::Http(http_err));
        for err in cases {
            assert_ne!(
                err.rpc_code(),
                -32601,
                "MirageError must not use -32601 (reserved): {err:?}"
            );
        }
    }

    /// Mirage telemetry emits `ResourceWarning` via roko-runtime's `EventBus`
    /// (non-blocking emit path; see `apply_resource_pressure` in `rpc.rs`).
    #[test]
    fn telemetry_bus_emits_resource_warning() {
        use crate::events::MirageTelemetryEvent;
        use roko_runtime::event_bus::EventBus;

        let bus = EventBus::<MirageTelemetryEvent>::new(4);
        bus.sender().emit(MirageTelemetryEvent::ResourceWarning {
            resource: "memory".to_owned(),
            utilization: 0.62,
        });
        let replay = bus.replay_from(0);
        assert_eq!(replay.len(), 1);
        assert!(matches!(
            &replay[0].payload,
            MirageTelemetryEvent::ResourceWarning {
                resource,
                utilization
            } if resource == "memory" && (*utilization - 0.62f64).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn transaction_request_deserializes_hex_quantity_fields() {
        let request: TransactionRequest = serde_json::from_value(serde_json::json!({
            "from": "0x1000000000000000000000000000000000000001",
            "to": "0x2000000000000000000000000000000000000002",
            "gas": "0x5208",
            "gasPrice": "0x3b9aca00",
            "nonce": "0x7",
            "chainId": "0x1",
            "value": "0xa",
            "input": "0xdeadbeef"
        }))
        .expect("hex quantity request parses");

        assert_eq!(request.gas, Some(21_000));
        assert_eq!(request.gas_price, Some(1_000_000_000));
        assert_eq!(request.nonce, Some(7));
        assert_eq!(request.chain_id, Some(1));
        assert_eq!(request.value, Some(U256::from(10_u64)));
        assert_eq!(
            request.data,
            Some(Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]))
        );
    }
}
