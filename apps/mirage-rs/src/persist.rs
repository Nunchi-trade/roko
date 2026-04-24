//! Periodic atomic disk snapshots for mirage-rs state.
//!
//! Captures all persistable state (fork write layer, chain stores) into a single
//! JSON file, written atomically via write-then-rename. A background task runs on
//! a configurable interval and on graceful shutdown.
//!
//! # Design
//!
//! - No WAL, no SQLite, no new dependencies
//! - One JSON file, human-readable for Railway debugging
//! - Serialisation and I/O happen outside the writer gate (no RPC blocking)
//! - Reconstructible state (read cache, HDC/HNSW indices, event buses) is skipped

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use alloy_primitives::{Address, B256};
use serde::{Deserialize, Serialize};

use crate::fork::{DirtyStore, ForkState, LocalBlock, LocalReceipt, LocalTransaction, MirageFork};

/// Current schema version. Bump when the snapshot format changes.
pub const SNAPSHOT_VERSION: u32 = 1;

/// Default snapshot filename.
const SNAPSHOT_FILE: &str = "mirage-snapshot.json";

// ---------------------------------------------------------------------------
// Top-level snapshot
// ---------------------------------------------------------------------------

/// Complete serialisable snapshot of mirage-rs state.
#[derive(Serialize, Deserialize)]
pub struct MirageSnapshot {
    /// Schema version (reject unknown on load).
    pub version: u32,
    /// Unix seconds when the snapshot was captured.
    pub created_at: u64,
    /// Fork state (always present).
    pub fork: ForkSnapshot,
    /// Chain extension state (present when chain features were active).
    #[cfg(feature = "chain")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain: Option<ChainSnapshot>,
}

// ---------------------------------------------------------------------------
// Fork snapshot
// ---------------------------------------------------------------------------

/// Serialisable extract of [`ForkState`] (write layer + metadata).
///
/// Field names mirror [`ForkState`] — see that struct for per-field documentation.
#[derive(Clone, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct ForkSnapshot {
    pub local_block_number: u64,
    pub chain_id: u64,
    pub timestamp: u64,
    pub next_base_fee_per_gas: u128,
    pub coinbase: Address,
    pub prev_randao: B256,
    pub fork_block: u64,
    pub fork_url: Option<String>,
    pub strict_nonce: bool,
    pub strict_balance: bool,
    pub verify_signatures: bool,
    pub impersonated_accounts: HashSet<Address>,
    pub dirty: DirtyStore,
    pub receipts: HashMap<B256, LocalReceipt>,
    pub transactions: HashMap<B256, LocalTransaction>,
    pub blocks_by_number: BTreeMap<u64, LocalBlock>,
    pub blocks_by_hash: HashMap<B256, LocalBlock>,
}

// ---------------------------------------------------------------------------
// Chain snapshot
// ---------------------------------------------------------------------------

/// Serialisable extract of chain extension state.
///
/// Field names mirror [`crate::chain_rpc::ChainContext`] — see that struct for per-field docs.
#[cfg(feature = "chain")]
#[derive(Clone, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct ChainSnapshot {
    pub toggles: crate::chain_rpc::ChainToggles,
    pub knowledge: crate::chain::KnowledgeSnapshot,
    pub pheromones: crate::chain::PheromoneFieldSnapshot,
    pub agents: crate::chain::AgentRegistrySnapshot,
    pub tasks: crate::chain::TaskStoreSnapshot,
    pub predictions: crate::chain::PredictionStoreSnapshot,
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

/// Extracts a snapshot from a [`MirageFork`] handle.
///
/// The caller should hold the writer gate so that state is consistent.
pub fn capture_fork_snapshot(fork: &ForkState) -> ForkSnapshot {
    ForkSnapshot {
        local_block_number: fork.local_block_number,
        chain_id: fork.chain_id,
        timestamp: fork.timestamp,
        next_base_fee_per_gas: fork.next_base_fee_per_gas,
        coinbase: fork.coinbase,
        prev_randao: fork.prev_randao,
        fork_block: fork.fork_block,
        fork_url: fork.fork_url.clone(),
        strict_nonce: fork.strict_nonce,
        strict_balance: fork.strict_balance,
        verify_signatures: fork.verify_signatures,
        impersonated_accounts: fork.impersonated_accounts.clone(),
        dirty: fork.db.dirty.clone(),
        receipts: fork.receipts.clone(),
        transactions: fork.transactions.clone(),
        blocks_by_number: fork.blocks_by_number.clone(),
        blocks_by_hash: fork.blocks_by_hash.clone(),
    }
}

/// Captures a complete snapshot from the mirage handle and optional chain context.
#[cfg(feature = "chain")]
pub fn capture_snapshot(
    mirage: &MirageFork,
    chain_ctx: Option<&crate::chain_rpc::ChainContext>,
) -> MirageSnapshot {
    let state = mirage.inner.read();
    let fork = capture_fork_snapshot(&state.fork);
    let chain = chain_ctx.map(|ctx| ChainSnapshot {
        toggles: ctx.toggles,
        knowledge: ctx.knowledge.snapshot(),
        pheromones: ctx.pheromones.snapshot(),
        agents: ctx.agent_registry.snapshot(),
        tasks: ctx.task_store.snapshot(),
        predictions: ctx.prediction_store.snapshot(),
    });
    MirageSnapshot {
        version: SNAPSHOT_VERSION,
        created_at: now_secs(),
        fork,
        chain,
    }
}

/// Captures a fork-only snapshot (no chain extensions).
#[cfg(not(feature = "chain"))]
pub fn capture_snapshot(mirage: &MirageFork) -> MirageSnapshot {
    let state = mirage.inner.read();
    let fork = capture_fork_snapshot(&state.fork);
    MirageSnapshot {
        version: SNAPSHOT_VERSION,
        created_at: now_secs(),
        fork,
    }
}

// ---------------------------------------------------------------------------
// Apply (restore)
// ---------------------------------------------------------------------------

/// Mutates a fresh [`ForkState`] with data from a snapshot.
///
/// After restoring, prunes excess blocks (and their txs/receipts) so that
/// snapshots written before the pruning fix don't immediately OOM.
pub fn apply_fork_snapshot(fork: &mut ForkState, snap: ForkSnapshot) {
    fork.local_block_number = snap.local_block_number;
    fork.chain_id = snap.chain_id;
    fork.timestamp = snap.timestamp;
    fork.next_base_fee_per_gas = snap.next_base_fee_per_gas;
    fork.coinbase = snap.coinbase;
    fork.prev_randao = snap.prev_randao;
    fork.fork_block = snap.fork_block;
    fork.fork_url = snap.fork_url;
    fork.strict_nonce = snap.strict_nonce;
    fork.strict_balance = snap.strict_balance;
    fork.verify_signatures = snap.verify_signatures;
    fork.impersonated_accounts = snap.impersonated_accounts;
    fork.db.dirty = snap.dirty;
    fork.receipts = snap.receipts;
    fork.transactions = snap.transactions;
    fork.blocks_by_number = snap.blocks_by_number;
    fork.blocks_by_hash = snap.blocks_by_hash;
    // Prune to bounded size — catches pre-fix snapshots with unbounded state.
    fork.prune_old_blocks();
}

/// Constructs a [`ChainContext`] from a chain snapshot.
#[cfg(feature = "chain")]
pub fn chain_context_from_snapshot(
    snap: ChainSnapshot,
    hnsw_threshold: usize,
) -> crate::chain_rpc::ChainContext {
    use crate::chain::{KnowledgeStore, PheromoneField, PredictionStore, TaskStore};

    let ChainSnapshot {
        toggles,
        knowledge: knowledge_snapshot,
        pheromones: pheromone_snapshot,
        agents: legacy_agent_snapshot,
        tasks: task_snapshot,
        predictions: prediction_snapshot,
    } = snap;

    // `from_snapshot` rebuilds the brute-force HDC index. HNSW is not persisted
    // — it auto-activates when entry count exceeds `hnsw_threshold` on the next
    // `post()`. This is acceptable: HNSW is a query-time optimisation, not state.
    let _ = hnsw_threshold; // reserved for future HNSW rebuild
    let knowledge = KnowledgeStore::from_snapshot(knowledge_snapshot);
    let pheromones = PheromoneField::from_snapshot(pheromone_snapshot, 0.01);
    let legacy_agent_count = legacy_agent_snapshot.agents.len();
    let legacy_trace_count = legacy_agent_snapshot.traces.len();
    if legacy_agent_count > 0 || legacy_trace_count > 0 {
        tracing::info!(
            legacy_agent_count,
            legacy_trace_count,
            "ignoring persisted legacy AgentRegistry snapshot; ERC-8004 contracts remain the durable identity source"
        );
    }

    crate::chain_rpc::ChainContext {
        knowledge,
        pheromones,
        toggles,
        #[cfg(feature = "roko")]
        pheromone_bus: None,
        #[cfg(feature = "roko")]
        insight_bus: None,
        agent_registry: crate::chain::AgentRegistry::new(),
        agent_bus: tokio::sync::broadcast::channel(1_024).0,
        task_store: TaskStore::from_snapshot(task_snapshot),
        task_bus: tokio::sync::broadcast::channel(1_024).0,
        prediction_store: PredictionStore::from_snapshot(prediction_snapshot),
        prediction_bus: tokio::sync::broadcast::channel(1_024).0,
    }
}

// ---------------------------------------------------------------------------
// Disk I/O
// ---------------------------------------------------------------------------

/// Reads a snapshot from disk. Returns `Ok(None)` if the file does not exist.
pub fn load_snapshot(state_dir: &Path) -> anyhow::Result<Option<MirageSnapshot>> {
    let path = state_dir.join(SNAPSHOT_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read(&path)
        .map_err(|e| anyhow::anyhow!("failed to read snapshot at {}: {e}", path.display()))?;
    let snap: MirageSnapshot = serde_json::from_slice(&data)
        .map_err(|e| anyhow::anyhow!("failed to parse snapshot at {}: {e}", path.display()))?;
    if snap.version != SNAPSHOT_VERSION {
        anyhow::bail!(
            "unsupported snapshot version {} (expected {})",
            snap.version,
            SNAPSHOT_VERSION,
        );
    }
    Ok(Some(snap))
}

/// Writes a snapshot to disk atomically (write to `.tmp`, then rename).
pub fn write_snapshot(snap: &MirageSnapshot, state_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(state_dir)?;
    let final_path = state_dir.join(SNAPSHOT_FILE);
    let tmp_path = state_dir.join(format!("{SNAPSHOT_FILE}.tmp"));
    let data = serde_json::to_vec(snap)
        .map_err(|e| anyhow::anyhow!("failed to serialise snapshot: {e}"))?;
    std::fs::write(&tmp_path, &data)
        .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, &final_path)
        .map_err(|e| anyhow::anyhow!("failed to rename snapshot: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Background persistence loop
// ---------------------------------------------------------------------------

/// Spawns a background task that periodically snapshots state to disk.
///
/// The loop runs until the shutdown receiver fires. On each tick it captures
/// a snapshot (under the internal read lock — no writer gate needed for reads),
/// then serialises and writes outside any lock.
#[cfg(feature = "chain")]
pub fn spawn_persistence_loop(
    mirage: MirageFork,
    chain_ctx: Option<std::sync::Arc<parking_lot::RwLock<crate::chain_rpc::ChainContext>>>,
    state_dir: PathBuf,
    interval: Duration,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = shutdown_rx.recv() => {
                    tracing::info!("persistence loop: shutdown signal received");
                    break;
                }
            }
            let snap = match chain_ctx.as_ref() {
                Some(ctx) => {
                    let chain = ctx.read();
                    capture_snapshot(&mirage, Some(&*chain))
                }
                None => capture_snapshot(&mirage, None),
            };
            if let Err(e) = write_snapshot(&snap, &state_dir) {
                tracing::warn!("periodic snapshot failed: {e}");
            } else {
                tracing::debug!("periodic snapshot written to {}", state_dir.display());
            }
        }
    });
}

/// Spawns a background persistence loop (fork-only, no chain extensions).
#[cfg(not(feature = "chain"))]
pub fn spawn_persistence_loop(
    mirage: MirageFork,
    state_dir: PathBuf,
    interval: Duration,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = shutdown_rx.recv() => {
                    tracing::info!("persistence loop: shutdown signal received");
                    break;
                }
            }
            let snap = capture_snapshot(&mirage);
            if let Err(e) = write_snapshot(&snap, &state_dir) {
                tracing::warn!("periodic snapshot failed: {e}");
            } else {
                tracing::debug!("periodic snapshot written to {}", state_dir.display());
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_fork_snapshot() {
        use crate::fork::{ForkState, HybridDB};
        use crate::provider::UpstreamRpc;
        use std::sync::Arc;

        let upstream = Arc::new(UpstreamRpc::new_with_limits(None, None, 1, 1, 1));
        let mut fork = ForkState::new(
            HybridDB::new(
                upstream,
                100,
                Duration::from_secs(12),
                1.try_into().unwrap(),
                1,
            ),
            42,
            1,
        );
        fork.strict_nonce = true;
        fork.coinbase = Address::repeat_byte(0xAB);

        let snap = capture_fork_snapshot(&fork);
        let json = serde_json::to_vec(&snap).expect("serialise");
        let restored: ForkSnapshot = serde_json::from_slice(&json).expect("deserialise");

        assert_eq!(restored.local_block_number, 42);
        assert_eq!(restored.chain_id, 1);
        assert!(restored.strict_nonce);
        assert_eq!(restored.coinbase, Address::repeat_byte(0xAB));
    }

    #[test]
    fn atomic_write_cleans_up_tmp() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let snap = MirageSnapshot {
            version: SNAPSHOT_VERSION,
            created_at: 1234,
            fork: ForkSnapshot {
                local_block_number: 1,
                chain_id: 1,
                timestamp: 0,
                next_base_fee_per_gas: 0,
                coinbase: Address::ZERO,
                prev_randao: B256::ZERO,
                fork_block: 0,
                fork_url: None,
                strict_nonce: false,
                strict_balance: false,
                verify_signatures: false,
                impersonated_accounts: HashSet::new(),
                dirty: DirtyStore::default(),
                receipts: HashMap::new(),
                transactions: HashMap::new(),
                blocks_by_number: BTreeMap::new(),
                blocks_by_hash: HashMap::new(),
            },
            #[cfg(feature = "chain")]
            chain: None,
        };

        write_snapshot(&snap, dir.path()).expect("write");

        let tmp_path = dir.path().join(format!("{SNAPSHOT_FILE}.tmp"));
        assert!(
            !tmp_path.exists(),
            ".tmp file should be cleaned up by rename"
        );

        let final_path = dir.path().join(SNAPSHOT_FILE);
        assert!(final_path.exists());

        let loaded = load_snapshot(dir.path()).expect("load").expect("some");
        assert_eq!(loaded.version, SNAPSHOT_VERSION);
        assert_eq!(loaded.created_at, 1234);
        assert_eq!(loaded.fork.local_block_number, 1);
    }

    #[test]
    fn load_missing_returns_none() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let result = load_snapshot(dir.path()).expect("no error");
        assert!(result.is_none());
    }

    #[test]
    fn load_bad_version_returns_error() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let path = dir.path().join(SNAPSHOT_FILE);
        std::fs::write(&path, r#"{"version":999,"created_at":0,"fork":{}}"#).unwrap();
        let result = load_snapshot(dir.path());
        assert!(result.is_err());
    }

    #[cfg(feature = "chain")]
    #[test]
    fn chain_context_restore_discards_legacy_agent_registry_snapshot() {
        let mut agent_registry = crate::chain::AgentRegistry::new();
        assert!(agent_registry.register(
            "agent-1".to_owned(),
            vec![0xAA],
            "worker".to_owned(),
            "owner-1".to_owned(),
            123
        ));

        let snap = ChainSnapshot {
            toggles: crate::chain_rpc::ChainToggles::all(),
            knowledge: crate::chain::KnowledgeStore::new().snapshot(),
            pheromones: crate::chain::PheromoneField::default().snapshot(),
            agents: agent_registry.snapshot(),
            tasks: crate::chain::TaskStore::new().snapshot(),
            predictions: crate::chain::PredictionStore::new().snapshot(),
        };

        let restored = chain_context_from_snapshot(snap, 1_000);

        assert!(restored.agent_registry.list_agents().is_empty());
    }
}
