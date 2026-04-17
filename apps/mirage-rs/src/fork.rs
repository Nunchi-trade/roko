//! Core fork state and lazy database layers.

#![allow(
    clippy::needless_pass_by_value,
    clippy::too_many_arguments,
    clippy::unnecessary_wraps
)]

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    num::NonZeroUsize,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use alloy_primitives::{Address, B256, Bytes, U256, address, keccak256};
use lru::LruCache;
use parking_lot::{Mutex, RwLock};
use revm::primitives::{Log as RevmLog, TxKind, hardfork::SpecId};
use revm::state::Account as RevmStateAccount;
use revm::{
    ExecuteCommitEvm, ExecuteEvm, MainBuilder, MainContext,
    context::{
        BlockEnv, Context, TxEnv,
        result::{EVMError, ExecutionResult as RevmExecutionResult, InvalidTransaction},
    },
    database_interface::DatabaseCommit,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, watch};

use crate::{
    AccountInfo, Bytecode, ExecutionResult, MirageError, Result, TransactionRequest,
    cow::{BytecodeCache, SharedBytecodeCache},
    integration::{EventFilter, MirageEvent},
    provider::{BlockTag, UpstreamRpc},
    replay::{AccountDiff, LogEntry, SpeculativeExecutor, StateDiff},
    resources::{MirageMode, ResourceModel, ResourceUsage},
    scenario::{ScenarioJob, ScenarioSet},
};

use roko_runtime::event_bus::{BusSender, EventBus};

use crate::events::MirageTelemetryEvent;

/// Cache entry with TTL metadata.
#[derive(Debug, Clone)]
struct TimedEntry<T> {
    value: T,
    fetched_at: Instant,
}

/// Predicate used by [`ReadCache`] for TTL checks (**INV-002**).
///
/// Plan formula (verbatim): `cache_valid(entry) = (now - entry.cached_at) < cache_ttl`.
#[inline]
pub(crate) fn read_cache_entry_valid(ttl: Duration, age: Duration) -> bool {
    age < ttl
}

/// Hot account/storage cache layered between dirty state and upstream RPC.
#[derive(Debug)]
pub struct ReadCache {
    account_infos: LruCache<Address, TimedEntry<AccountInfo>>,
    storage: LruCache<(Address, U256), TimedEntry<U256>>,
    block_hashes: LruCache<u64, B256>,
    ttl: Duration,
    hits: u64,
    misses: u64,
}

impl ReadCache {
    /// Creates a new cache with the provided capacity and TTL.
    #[must_use]
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        let capacity = NonZeroUsize::new(capacity.max(1)).unwrap_or(NonZeroUsize::MIN);
        Self {
            account_infos: LruCache::new(capacity),
            storage: LruCache::new(capacity),
            block_hashes: LruCache::new(capacity),
            ttl,
            hits: 0,
            misses: 0,
        }
    }

    /// Returns a cached account if it is still fresh.
    #[must_use]
    pub fn get_account(&mut self, address: &Address) -> Option<AccountInfo> {
        match self.account_infos.get(address) {
            Some(entry) if read_cache_entry_valid(self.ttl, entry.fetched_at.elapsed()) => {
                self.hits = self.hits.saturating_add(1);
                Some(entry.value.clone())
            }
            Some(_) => {
                let _ = self.account_infos.pop(address);
                self.misses = self.misses.saturating_add(1);
                None
            }
            None => {
                self.misses = self.misses.saturating_add(1);
                None
            }
        }
    }

    /// Inserts an account into the cache.
    pub fn insert_account(&mut self, address: Address, info: AccountInfo) {
        self.account_infos.put(
            address,
            TimedEntry {
                value: info,
                fetched_at: Instant::now(),
            },
        );
    }

    /// Returns a cached storage value if it is still fresh.
    #[must_use]
    pub fn get_storage(&mut self, address: &Address, slot: &U256) -> Option<U256> {
        match self.storage.get(&(*address, *slot)) {
            Some(entry) if read_cache_entry_valid(self.ttl, entry.fetched_at.elapsed()) => {
                self.hits = self.hits.saturating_add(1);
                Some(entry.value)
            }
            Some(_) => {
                let _ = self.storage.pop(&(*address, *slot));
                self.misses = self.misses.saturating_add(1);
                None
            }
            None => {
                self.misses = self.misses.saturating_add(1);
                None
            }
        }
    }

    /// Inserts a storage value into the cache.
    pub fn insert_storage(&mut self, address: Address, slot: U256, value: U256) {
        self.storage.put(
            (address, slot),
            TimedEntry {
                value,
                fetched_at: Instant::now(),
            },
        );
    }

    /// Inserts a block hash into the cache.
    pub fn insert_block_hash(&mut self, number: u64, hash: B256) {
        self.block_hashes.put(number, hash);
    }

    /// Clears all cached data.
    pub fn clear(&mut self) {
        self.account_infos.clear();
        self.storage.clear();
        self.block_hashes.clear();
        self.hits = 0;
        self.misses = 0;
    }

    /// Returns the cache hit rate.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            1.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    /// Returns the total number of cached entries.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.account_infos.len() + self.storage.len() + self.block_hashes.len()
    }

    /// Evicts least-recently-used entries until the cache fits the target size.
    pub fn evict_to(&mut self, target_entries: usize) {
        while self.entry_count() > target_entries {
            if self.account_infos.pop_lru().is_some() {
                continue;
            }
            if self.storage.pop_lru().is_some() {
                continue;
            }
            if self.block_hashes.pop_lru().is_some() {
                continue;
            }
            break;
        }
    }
}

/// Local write layer for partially overridden account state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DirtyAccount {
    /// Overridden balance, if any.
    pub balance: Option<U256>,
    /// Overridden nonce, if any.
    pub nonce: Option<u64>,
    /// Overridden bytecode, if any.
    pub code: Option<Bytecode>,
    /// Overridden bytecode hash, if any.
    pub code_hash: Option<B256>,
    /// Cached ERC-20 balance mapping slot for token contracts, when known.
    pub erc20_balance_slot: Option<U256>,
    /// Local ERC-20 balance overrides keyed by owner address.
    pub erc20_balances: HashMap<Address, U256>,
    /// Dirty storage writes by slot.
    pub storage: HashMap<U256, U256>,
}

/// Provenance for a watched contract entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WatchSource {
    /// Added via classifier because the diff looked protocol-like.
    AutoClassified,
    /// Added through replay contagion from a parent address.
    Contagion {
        /// Parent contract whose replay touched this address.
        parent: Address,
    },
    /// Added explicitly through RPC or embedding APIs.
    Manual,
}

/// Watch-list metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchEntry {
    /// Source that introduced the contract into the watch list.
    pub source: WatchSource,
    /// Block at which the contract was added.
    pub added_at_block: u64,
    /// Number of dirty slots present when the contract was added.
    pub initial_slot_count: usize,
    /// Number of replay passes applied to the contract.
    pub replay_count: u64,
}

#[derive(Debug, Clone)]
struct DirtyStoreSnapshot {
    accounts: HashMap<Address, DirtyAccount>,
    watch_list: HashMap<Address, WatchEntry>,
    unwatch_list: HashSet<Address>,
    total_dirty_slots: u64,
    block_number: u64,
    tx_index: u64,
}

/// Write layer for local mutations and watch tracking.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DirtyStore {
    /// Dirty accounts keyed by address.
    pub accounts: HashMap<Address, DirtyAccount>,
    /// Current watch list.
    pub watch_list: HashMap<Address, WatchEntry>,
    /// Explicitly excluded contracts.
    pub unwatch_list: HashSet<Address>,
    /// Total number of dirty slots tracked locally.
    pub total_dirty_slots: u64,
    /// When set, newly classified protocol contracts are demoted to slot-only reads.
    #[serde(skip)]
    pub(crate) demote_protocols_to_slot_only: bool,
    #[serde(skip)]
    snapshots: HashMap<u64, Box<DirtyStoreSnapshot>>,
    #[serde(skip)]
    next_snapshot_id: u64,
}

impl DirtyStore {
    /// Captures a snapshot of the current write layer.
    pub fn snapshot(&mut self, block_number: u64, tx_index: u64) -> u64 {
        let id = self.next_snapshot_id;
        self.next_snapshot_id = self.next_snapshot_id.saturating_add(1);
        self.snapshots.insert(
            id,
            Box::new(DirtyStoreSnapshot {
                accounts: self.accounts.clone(),
                watch_list: self.watch_list.clone(),
                unwatch_list: self.unwatch_list.clone(),
                total_dirty_slots: self.total_dirty_slots,
                block_number,
                tx_index,
            }),
        );
        id
    }

    /// Restores a previously taken snapshot and invalidates later snapshots.
    ///
    /// # Errors
    ///
    /// Returns [`MirageError::SnapshotNotFound`] if `id` is no longer live.
    pub fn revert(&mut self, id: u64) -> Result<(u64, u64)> {
        let snapshot = self
            .snapshots
            .remove(&id)
            .ok_or(MirageError::SnapshotNotFound(id))?;
        self.accounts = snapshot.accounts;
        self.watch_list = snapshot.watch_list;
        self.unwatch_list = snapshot.unwatch_list;
        self.total_dirty_slots = snapshot.total_dirty_slots;
        self.snapshots.retain(|snapshot_id, _| *snapshot_id < id);
        Ok((snapshot.block_number, snapshot.tx_index))
    }

    /// Clears all local state.
    pub fn clear(&mut self) {
        self.accounts.clear();
        self.watch_list.clear();
        self.unwatch_list.clear();
        self.total_dirty_slots = 0;
        self.snapshots.clear();
    }

    /// Applies a state diff to the dirty store.
    pub fn apply_state_diff(&mut self, diff: &StateDiff) {
        for (address, account_diff) in &diff.accounts {
            let entry = self.accounts.entry(*address).or_default();
            if let Some(balance) = account_diff.new_balance {
                entry.balance = Some(balance);
            }
            if let Some(nonce) = account_diff.new_nonce {
                entry.nonce = Some(nonce);
            }
            if let Some(code) = account_diff.new_code.clone() {
                entry.code_hash = Some(code.hash_slow());
                entry.code = Some(code);
            }
            for (slot, value) in &account_diff.storage_written {
                if entry.storage.insert(*slot, *value).is_none() {
                    self.total_dirty_slots = self.total_dirty_slots.saturating_add(1);
                }
            }
        }
    }
}

/// Configuration for diff classification.
#[derive(Debug, Clone)]
pub struct ClassificationConfig {
    /// Minimum written-slot count to promote an address to the watch list.
    pub protocol_slot_threshold: usize,
    /// Whether token-interface heuristics should run.
    pub check_token_interface: bool,
    /// Maximum number of watched contracts.
    pub max_watched_contracts: usize,
    /// Whether contagion is enabled.
    pub enable_contagion: bool,
    /// Maximum contagion recursion depth.
    pub max_contagion_depth: usize,
}

impl Default for ClassificationConfig {
    fn default() -> Self {
        Self {
            protocol_slot_threshold: 3,
            check_token_interface: true,
            max_watched_contracts: 64,
            enable_contagion: true,
            max_contagion_depth: 2,
        }
    }
}

/// Classification result for a touched address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// Complex state; should be watched and replayed.
    Protocol,
    /// Slot-level override only.
    SlotOnly,
    /// Read-only interaction with no writes.
    ReadOnly,
}

/// Heuristic classifier for transaction state diffs.
#[derive(Debug, Clone)]
pub struct DiffClassifier {
    config: ClassificationConfig,
}

impl DiffClassifier {
    /// Creates a new classifier.
    #[must_use]
    pub fn new(config: ClassificationConfig) -> Self {
        Self { config }
    }

    /// Returns the effective configuration.
    #[must_use]
    pub fn config(&self) -> &ClassificationConfig {
        &self.config
    }

    /// Classifies each account touched by the diff.
    #[must_use]
    pub fn classify(&self, diff: &StateDiff) -> HashMap<Address, Classification> {
        diff.accounts
            .iter()
            .map(|(address, account_diff)| {
                let classification = if account_diff.storage_written.is_empty()
                    && !account_diff.info_changed
                {
                    Classification::ReadOnly
                } else if account_diff.storage_written.len() >= self.config.protocol_slot_threshold
                    && !self.looks_like_slot_only_token(account_diff)
                {
                    Classification::Protocol
                } else {
                    Classification::SlotOnly
                };
                (*address, classification)
            })
            .collect()
    }

    fn looks_like_slot_only_token(&self, account_diff: &AccountDiff) -> bool {
        if !self.config.check_token_interface || account_diff.storage_written.is_empty() {
            return false;
        }
        account_diff
            .storage_written
            .keys()
            .all(|slot| *slot >= U256::from(20))
    }

    /// Applies classifier output to the watch list.
    ///
    /// # Errors
    ///
    /// Returns [`MirageError::WatchListFull`] if the diff promotes more
    /// protocol contracts than the configured watch-list capacity allows.
    pub fn apply(&self, store: &mut DirtyStore, diff: &StateDiff, block_number: u64) -> Result<()> {
        for (address, classification) in self.classify(diff) {
            if store.unwatch_list.contains(&address) {
                continue;
            }
            let classification = if store.demote_protocols_to_slot_only
                && classification == Classification::Protocol
            {
                Classification::SlotOnly
            } else {
                classification
            };
            if classification == Classification::Protocol {
                if !store.watch_list.contains_key(&address)
                    && store.watch_list.len() >= self.config.max_watched_contracts
                {
                    return Err(MirageError::WatchListFull);
                }
                store
                    .watch_list
                    .entry(address)
                    .or_insert_with(|| WatchEntry {
                        source: WatchSource::AutoClassified,
                        added_at_block: block_number,
                        initial_slot_count: diff
                            .accounts
                            .get(&address)
                            .map_or(0, |account| account.storage_written.len()),
                        replay_count: 0,
                    });
            }
        }
        Ok(())
    }
}

/// Three-tier database used by the simplified fork executor.
#[derive(Debug)]
pub struct HybridDB {
    /// Dirty local write layer.
    pub dirty: DirtyStore,
    /// Read-through cache.
    pub read_cache: ReadCache,
    /// Bytecode cache shared across fork views.
    pub bytecode_cache: SharedBytecodeCache,
    /// Upstream RPC adapter.
    pub upstream: Arc<UpstreamRpc>,
    /// Optional pinned block for historical mode.
    pub pinned_block: Option<u64>,
    /// Read-cache TTL.
    pub cache_ttl: Duration,
    /// Effective chain ID.
    pub chain_id: u64,
}

impl HybridDB {
    /// Creates a new database.
    #[must_use]
    pub fn new(
        upstream: Arc<UpstreamRpc>,
        cache_capacity: usize,
        cache_ttl: Duration,
        bytecode_capacity: NonZeroUsize,
        chain_id: u64,
    ) -> Self {
        Self {
            dirty: DirtyStore::default(),
            read_cache: ReadCache::new(cache_capacity, cache_ttl),
            bytecode_cache: Arc::new(Mutex::new(BytecodeCache::new(bytecode_capacity))),
            upstream,
            pinned_block: None,
            cache_ttl,
            chain_id,
        }
    }

    /// Returns the resolved upstream block selector.
    #[must_use]
    pub fn resolve_block(&self) -> BlockTag {
        self.pinned_block.map_or(BlockTag::Latest, BlockTag::Number)
    }

    fn fetch_account_info(&self, address: Address) -> Result<Option<AccountInfo>> {
        self.upstream
            .get_account_info(address, self.resolve_block())
    }

    /// Writes an account balance override.
    pub fn set_balance(&mut self, address: Address, balance: U256) {
        self.dirty.accounts.entry(address).or_default().balance = Some(balance);
    }

    /// Writes an account nonce override.
    pub fn set_nonce(&mut self, address: Address, nonce: u64) {
        self.dirty.accounts.entry(address).or_default().nonce = Some(nonce);
    }

    /// Writes bytecode at an address.
    pub fn set_code(&mut self, address: Address, code: Bytecode) {
        let code_hash = code.hash_slow();
        let entry = self.dirty.accounts.entry(address).or_default();
        entry.code_hash = Some(code_hash);
        entry.code = Some(code.clone());
        self.bytecode_cache.lock().insert(code_hash, code);
    }

    /// Writes a storage slot override.
    pub fn set_storage(&mut self, address: Address, slot: U256, value: U256) {
        let entry = self.dirty.accounts.entry(address).or_default();
        if entry.storage.insert(slot, value).is_none() {
            self.dirty.total_dirty_slots = self.dirty.total_dirty_slots.saturating_add(1);
        }
    }

    /// Clears all local and cached state.
    pub fn reset(&mut self) {
        self.dirty.clear();
        self.read_cache.clear();
    }

    /// Returns the ERC-20 balance slot used for `owner`.
    ///
    /// # Errors
    ///
    /// Returns upstream storage or account-read errors while probing candidate
    /// slots, or [`MirageError::SlotDetectionFailed`] if no slot can be
    /// inferred.
    pub fn erc20_balance_slot(&mut self, token: Address, owner: Address) -> Result<U256> {
        if let Some(slot) = self
            .dirty
            .accounts
            .get(&token)
            .and_then(|account| account.erc20_balance_slot)
        {
            return Ok(slot);
        }
        if let Some(slot) = known_erc20_balance_slot(token) {
            self.dirty
                .accounts
                .entry(token)
                .or_default()
                .erc20_balance_slot = Some(slot);
            return Ok(slot);
        }
        for slot_index in 0_u64..8 {
            let slot = U256::from(slot_index);
            let mapping_slot = erc20_mapping_slot(owner, slot);
            if self.storage(token, mapping_slot)? != U256::ZERO {
                self.dirty
                    .accounts
                    .entry(token)
                    .or_default()
                    .erc20_balance_slot = Some(slot);
                return Ok(slot);
            }
        }
        Err(MirageError::SlotDetectionFailed(token))
    }

    /// Reads a token balance using the detected ERC-20 balance slot.
    ///
    /// # Errors
    ///
    /// Returns upstream storage, account-read, or call errors while resolving
    /// the slot or fetching the balance. Returns
    /// [`MirageError::SlotDetectionFailed`] if the token cannot be identified
    /// as ERC-20-like from local or upstream state.
    pub fn erc20_balance_of(&mut self, token: Address, owner: Address) -> Result<U256> {
        if let Some(balance) = self
            .dirty
            .accounts
            .get(&token)
            .and_then(|account| account.erc20_balances.get(&owner))
            .copied()
        {
            return Ok(balance);
        }

        if let Ok(slot) = self.erc20_balance_slot(token, owner) {
            return self.storage(token, erc20_mapping_slot(owner, slot));
        }

        if self.basic(token)?.is_none() {
            return Err(MirageError::SlotDetectionFailed(token));
        }

        let data = Bytes::from(
            [
                &[0x70, 0xa0, 0x82, 0x31][..],
                &[0_u8; 12][..],
                owner.as_slice(),
            ]
            .concat(),
        );
        let raw = self
            .upstream
            .eth_call(owner, token, &data, self.resolve_block())?;
        let padded = if raw.len() >= 32 {
            raw[raw.len() - 32..].to_vec()
        } else {
            let mut padded = vec![0_u8; 32 - raw.len()];
            padded.extend_from_slice(&raw);
            padded
        };
        Ok(U256::from_be_slice(&padded))
    }

    /// Writes a token balance using the detected or default ERC-20 balance slot.
    ///
    /// # Errors
    ///
    /// This helper does not currently return an error; it falls back to slot
    /// `0` when the balance slot or total supply cannot be resolved.
    pub fn set_erc20_balance(
        &mut self,
        token: Address,
        owner: Address,
        balance: U256,
    ) -> Result<U256> {
        let previous_balance = self.erc20_balance_of(token, owner).unwrap_or(U256::ZERO);
        let slot = self.erc20_balance_slot(token, owner).unwrap_or(U256::ZERO);
        let balance_slot = erc20_mapping_slot(owner, slot);
        self.set_storage(token, balance_slot, balance);
        let total_supply_slot = U256::ONE;
        let total_supply = self.storage(token, total_supply_slot).unwrap_or(U256::ZERO);
        let next_total_supply = if balance >= previous_balance {
            total_supply.saturating_add(balance - previous_balance)
        } else {
            total_supply.saturating_sub(previous_balance - balance)
        };
        self.set_storage(token, total_supply_slot, next_total_supply);
        self.dirty
            .accounts
            .entry(token)
            .or_default()
            .erc20_balance_slot = Some(slot);
        self.dirty
            .accounts
            .entry(token)
            .or_default()
            .erc20_balances
            .insert(owner, balance);
        Ok(slot)
    }

    /// Trims the read cache to the requested target size.
    pub fn evict_read_cache_to(&mut self, target_entries: usize) {
        self.read_cache.evict_to(target_entries);
    }

    /// Clones this database for a read-only execution view.
    ///
    /// The fork's [`DirtyStore`] is copied so [`EvmExecutor::call`] and similar paths can perform
    /// slot detection or other incidental writes on the clone without mutating the live fork.
    fn clone_for_readonly(&self) -> Self {
        Self {
            dirty: self.dirty.clone(),
            read_cache: ReadCache::new(self.read_cache.entry_count().max(1), self.cache_ttl),
            bytecode_cache: Arc::clone(&self.bytecode_cache),
            upstream: Arc::clone(&self.upstream),
            pinned_block: self.pinned_block,
            cache_ttl: self.cache_ttl,
            chain_id: self.chain_id,
        }
    }
}

impl HybridDB {
    /// Reads basic account info.
    ///
    /// # Errors
    ///
    /// Returns upstream account-fetching or parsing errors when the account is
    /// not already satisfied by the dirty layer or read cache.
    pub fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>> {
        if let Some(dirty) = self.dirty.accounts.get(&address) {
            let needs_upstream =
                dirty.balance.is_none() || dirty.nonce.is_none() || dirty.code.is_none();
            let base = if needs_upstream {
                self.fetch_account_info(address)?.unwrap_or_default()
            } else {
                AccountInfo::default()
            };
            let info = AccountInfo {
                balance: dirty.balance.unwrap_or(base.balance),
                nonce: dirty.nonce.unwrap_or(base.nonce),
                code_hash: dirty.code_hash.unwrap_or(base.code_hash),
                code: dirty.code.clone().or(base.code),
            };
            return Ok(Some(info));
        }

        if let Some(info) = self.read_cache.get_account(&address) {
            return Ok(Some(info));
        }

        let info = self.fetch_account_info(address)?;
        if let Some(ref account) = info {
            self.read_cache.insert_account(address, account.clone());
        }
        Ok(info)
    }

    /// Reads bytecode by hash.
    ///
    /// # Errors
    ///
    /// Returns upstream bytecode-fetching or parsing errors, or
    /// [`MirageError::Upstream`] if the hash cannot be resolved.
    pub fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode> {
        let cached = self.bytecode_cache.lock().get(&code_hash);
        if let Some(bytecode) = cached {
            return Ok(bytecode);
        }
        for account in self.dirty.accounts.values() {
            if account.code_hash == Some(code_hash) {
                if let Some(code) = account.code.clone() {
                    self.bytecode_cache.lock().insert(code_hash, code.clone());
                    return Ok(code);
                }
            }
        }
        let bytecode = self
            .upstream
            .get_code_by_hash(code_hash, self.resolve_block())?;
        self.bytecode_cache
            .lock()
            .insert(code_hash, bytecode.clone());
        Ok(bytecode)
    }

    /// Reads storage.
    ///
    /// # Errors
    ///
    /// Returns upstream storage-fetching or parsing errors when the slot is
    /// not already satisfied by the dirty layer or read cache.
    pub fn storage(&mut self, address: Address, index: U256) -> Result<U256> {
        if let Some(dirty) = self.dirty.accounts.get(&address) {
            if let Some(value) = dirty.storage.get(&index) {
                return Ok(*value);
            }
        }
        if let Some(value) = self.read_cache.get_storage(&address, &index) {
            return Ok(value);
        }
        let value = self
            .upstream
            .get_storage_at(address, index, self.resolve_block())?;
        self.read_cache.insert_storage(address, index, value);
        Ok(value)
    }

    /// Reads a block hash.
    ///
    /// # Errors
    ///
    /// Returns upstream block-hash fetching or parsing errors when the hash is
    /// not already cached locally.
    pub fn block_hash(&mut self, number: u64) -> Result<B256> {
        if let Some(hash) = self.read_cache.block_hashes.get(&number).copied() {
            return Ok(hash);
        }
        let hash = self.upstream.get_block_hash(number)?;
        self.read_cache.insert_block_hash(number, hash);
        Ok(hash)
    }
}

fn mirage_account_into_revm(info: AccountInfo) -> revm::state::AccountInfo {
    revm::state::AccountInfo {
        balance: info.balance,
        nonce: info.nonce,
        code_hash: info.code_hash,
        account_id: None,
        code: info
            .code
            .map(|c| revm::bytecode::Bytecode::new_raw(c.bytecode().clone())),
    }
}

impl revm::Database for HybridDB {
    type Error = MirageError;

    fn basic(
        &mut self,
        address: Address,
    ) -> std::result::Result<Option<revm::state::AccountInfo>, Self::Error> {
        Ok(HybridDB::basic(self, address)?.map(mirage_account_into_revm))
    }

    fn code_by_hash(
        &mut self,
        code_hash: B256,
    ) -> std::result::Result<revm::bytecode::Bytecode, Self::Error> {
        let mirage_bc = HybridDB::code_by_hash(self, code_hash)?;
        Ok(revm::bytecode::Bytecode::new_raw(
            mirage_bc.bytecode().clone(),
        ))
    }

    fn storage(&mut self, address: Address, index: U256) -> std::result::Result<U256, Self::Error> {
        HybridDB::storage(self, address, index)
    }

    fn block_hash(&mut self, number: u64) -> std::result::Result<B256, Self::Error> {
        HybridDB::block_hash(self, number)
    }
}

impl DatabaseCommit for HybridDB {
    fn commit(&mut self, changes: revm::primitives::AddressMap<RevmStateAccount>) {
        for (address, account) in changes {
            if !account.is_touched() {
                continue;
            }
            if account.is_selfdestructed() {
                if let Some(old) = self.dirty.accounts.remove(&address) {
                    self.dirty.total_dirty_slots = self
                        .dirty
                        .total_dirty_slots
                        .saturating_sub(old.storage.len() as u64);
                }
                continue;
            }
            if account.is_created() {
                if let Some(old) = self.dirty.accounts.remove(&address) {
                    self.dirty.total_dirty_slots = self
                        .dirty
                        .total_dirty_slots
                        .saturating_sub(old.storage.len() as u64);
                }
            }
            self.set_balance(address, account.info.balance);
            self.set_nonce(address, account.info.nonce);
            if let Some(code) = account.info.code.clone() {
                if !code.is_empty() {
                    self.set_code(address, Bytecode::new_raw(code.bytecode().clone()));
                }
            }
            for (key, slot) in account.storage {
                if slot.is_changed() {
                    self.set_storage(address, key, slot.present_value());
                }
            }
        }
    }
}

/// Persisted local transaction info for RPC inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalTransaction {
    /// Transaction hash.
    pub hash: B256,
    /// Sender address.
    pub from: Address,
    /// Destination address, if this was not a create.
    pub to: Option<Address>,
    /// Transferred value.
    pub value: U256,
    /// Transaction input bytes.
    pub input: Bytes,
    /// Effective gas limit.
    pub gas: u64,
    /// Effective nonce.
    pub nonce: u64,
    /// Block number assigned locally.
    pub block_number: u64,
}

/// Persisted local transaction receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalReceipt {
    /// Transaction hash.
    pub transaction_hash: B256,
    /// Block hash.
    pub block_hash: B256,
    /// Local block number.
    pub block_number: u64,
    /// Sender address.
    pub from: Address,
    /// Destination address.
    pub to: Option<Address>,
    /// Gas used during execution.
    pub gas_used: u64,
    /// Success flag.
    pub success: bool,
    /// Logs emitted by the transaction.
    pub logs: Vec<LogEntry>,
    /// Canonical state diff generated by execution.
    pub state_diff: StateDiff,
}

/// Synthetic local block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalBlock {
    /// Block hash.
    pub hash: B256,
    /// Block number.
    pub number: u64,
    /// Timestamp in seconds.
    pub timestamp: u64,
    /// Total gas used in this block.
    pub gas_used: u64,
    /// Gas limit for this block.
    pub gas_limit: u64,
    /// Base fee per gas.
    pub base_fee_per_gas: u128,
    /// Block coinbase (miner/validator).
    pub coinbase: Address,
    /// Previous block's RANDAO mix.
    pub prev_randao: B256,
    /// Included transaction hashes.
    pub transactions: Vec<B256>,
}

/// Fork-global mutable state backing the RPC server.
#[derive(Debug)]
pub struct ForkState {
    /// Core database.
    pub db: HybridDB,
    /// Current local block number.
    pub local_block_number: u64,
    /// Chain ID served by the fork.
    pub chain_id: u64,
    /// Current local timestamp.
    pub timestamp: u64,
    /// Next block base fee.
    pub next_base_fee_per_gas: u128,
    /// Next block coinbase.
    pub coinbase: Address,
    /// Next block prevRandao.
    pub prev_randao: B256,
    /// Synthetic local receipts.
    pub receipts: HashMap<B256, LocalReceipt>,
    /// Synthetic local transactions.
    pub transactions: HashMap<B256, LocalTransaction>,
    /// Blocks indexed by number.
    pub blocks_by_number: BTreeMap<u64, LocalBlock>,
    /// Blocks indexed by hash.
    pub blocks_by_hash: HashMap<B256, LocalBlock>,
    /// Impersonated accounts allowed to send without local keys.
    pub impersonated_accounts: HashSet<Address>,
    /// Whether nonces are enforced strictly.
    pub strict_nonce: bool,
    /// Whether balances are enforced strictly.
    pub strict_balance: bool,
    /// Whether signatures should be verified.
    pub verify_signatures: bool,
    /// Block number of the upstream chain at fork time.
    pub fork_block: u64,
    /// Upstream RPC URL used for the fork (if any).
    pub fork_url: Option<String>,
}

impl Clone for ForkState {
    fn clone(&self) -> Self {
        Self {
            db: HybridDB {
                dirty: self.db.dirty.clone(),
                read_cache: ReadCache::new(
                    self.db.read_cache.entry_count().max(1),
                    self.db.cache_ttl,
                ),
                bytecode_cache: Arc::clone(&self.db.bytecode_cache),
                upstream: Arc::clone(&self.db.upstream),
                pinned_block: self.db.pinned_block,
                cache_ttl: self.db.cache_ttl,
                chain_id: self.db.chain_id,
            },
            local_block_number: self.local_block_number,
            chain_id: self.chain_id,
            timestamp: self.timestamp,
            next_base_fee_per_gas: self.next_base_fee_per_gas,
            coinbase: self.coinbase,
            prev_randao: self.prev_randao,
            receipts: self.receipts.clone(),
            transactions: self.transactions.clone(),
            blocks_by_number: self.blocks_by_number.clone(),
            blocks_by_hash: self.blocks_by_hash.clone(),
            impersonated_accounts: self.impersonated_accounts.clone(),
            strict_nonce: self.strict_nonce,
            strict_balance: self.strict_balance,
            verify_signatures: self.verify_signatures,
            fork_block: self.fork_block,
            fork_url: self.fork_url.clone(),
        }
    }
}

impl ForkState {
    /// Creates a new fork state from a database and initial head number.
    #[must_use]
    pub fn new(db: HybridDB, local_block_number: u64, chain_id: u64) -> Self {
        Self {
            fork_block: local_block_number,
            fork_url: db.upstream.http_url(),
            db,
            local_block_number,
            chain_id,
            timestamp: now_secs(),
            next_base_fee_per_gas: 1,
            coinbase: address!("0x0000000000000000000000000000000000000000"),
            prev_randao: B256::ZERO,
            receipts: HashMap::new(),
            transactions: HashMap::new(),
            blocks_by_number: BTreeMap::new(),
            blocks_by_hash: HashMap::new(),
            impersonated_accounts: HashSet::new(),
            strict_nonce: false,
            strict_balance: false,
            verify_signatures: false,
        }
    }

    /// Captures a dirty-state snapshot.
    pub fn snapshot(&mut self) -> u64 {
        self.db
            .dirty
            .snapshot(self.local_block_number, self.transactions.len() as u64)
    }

    /// Restores a previously captured snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`MirageError::SnapshotNotFound`] if the snapshot ID no longer
    /// exists.
    pub fn revert(&mut self, id: u64) -> Result<bool> {
        let (block_number, _) = self.db.dirty.revert(id)?;
        self.local_block_number = block_number;
        Ok(true)
    }

    /// Maximum number of local blocks retained in memory.
    const MAX_RETAINED_BLOCKS: usize = 1_000;

    /// Merges one committed local transaction into the live fork surface.
    pub(crate) fn commit_local_transaction(
        &mut self,
        diff: &StateDiff,
        transaction: LocalTransaction,
        receipt: LocalReceipt,
        block: LocalBlock,
    ) {
        self.db.dirty.apply_state_diff(diff);
        self.local_block_number = block.number;
        self.transactions.insert(transaction.hash, transaction);
        self.receipts.insert(receipt.transaction_hash, receipt);
        self.blocks_by_hash.insert(block.hash, block.clone());
        self.blocks_by_number.insert(block.number, block);
        self.prune_old_blocks();
    }

    /// Evicts blocks older than `MAX_RETAINED_BLOCKS` from both block maps.
    pub(crate) fn prune_old_blocks(&mut self) {
        while self.blocks_by_number.len() > Self::MAX_RETAINED_BLOCKS {
            if let Some((_, old)) = self.blocks_by_number.pop_first() {
                self.blocks_by_hash.remove(&old.hash);
            }
        }
    }

    /// Replaces the mutable execution surface with one produced on a fork clone.
    pub(crate) fn adopt_executed_branch(&mut self, branch: Self) {
        self.db.dirty = branch.db.dirty;
        self.local_block_number = branch.local_block_number;
        self.timestamp = branch.timestamp;
        self.receipts = branch.receipts;
        self.transactions = branch.transactions;
        self.blocks_by_number = branch.blocks_by_number;
        self.blocks_by_hash = branch.blocks_by_hash;
    }

    /// Returns the current state status snapshot.
    #[must_use]
    pub fn status(&self, mode: MirageMode) -> MirageStatus {
        MirageStatus {
            status: "ready".to_owned(),
            chain_id: self.chain_id,
            block_number: self.local_block_number,
            watch_list_size: self.db.dirty.watch_list.len(),
            dirty_account_count: self.db.dirty.accounts.len(),
            dirty_slot_count: self.db.dirty.total_dirty_slots,
            upstream_connected: self.db.upstream.has_http(),
            divergence_detected: false,
            mode,
            fork_block: self.fork_block,
            fork_url: self.fork_url.clone(),
        }
    }

    /// Returns the current resource usage snapshot.
    #[must_use]
    pub fn resource_usage(&self, model: &ResourceModel, mode: MirageMode) -> ResourceUsage {
        let (upstream_rpc_calls, upstream_rpc_errors) = self.db.upstream.stats();
        ResourceUsage::new(
            model,
            ResourceModel::current_process_memory_bytes(),
            self.db.read_cache.hit_rate(),
            self.db.read_cache.entry_count(),
            self.db.dirty.watch_list.len(),
            self.db.dirty.total_dirty_slots,
            upstream_rpc_calls,
            upstream_rpc_errors,
            mode,
            0,
        )
    }
}

/// Block header data broadcast to `eth_subscribe("newHeads")` subscribers.
#[derive(Debug, Clone)]
pub struct NewHeadBroadcast {
    /// Block number.
    pub number: u64,
    /// Unix timestamp in seconds.
    pub timestamp: u64,
    /// Gas used by all transactions in this block.
    pub gas_used: u64,
    /// Block gas limit.
    pub gas_limit: u64,
    /// Base fee per gas in wei.
    pub base_fee_per_gas: u128,
    /// Block proposer / miner address.
    pub coinbase: Address,
    /// prevRandao / mixHash field.
    pub prev_randao: B256,
}

/// Shared server state for embedding or RPC use.
pub(crate) struct MirageState {
    pub fork: ForkState,
    pub resource_model: ResourceModel,
    pub mode: MirageMode,
    /// Broadcasts runtime mode transitions so background tasks can stop on proxy demotion.
    pub(crate) mode_change: watch::Sender<MirageMode>,
    pub scenarios: HashMap<String, ScenarioSet>,
    pub jobs: HashMap<String, ScenarioJob>,
    pub event_bus: tokio::sync::broadcast::Sender<MirageEvent>,
    pub event_subscriptions: HashMap<String, EventFilter>,
    pub next_event_subscription_id: u64,
    /// Broadcasts new block headers to all `eth_subscribe("newHeads")` subscribers.
    pub new_heads_tx: tokio::sync::broadcast::Sender<NewHeadBroadcast>,
    pub last_request_at: Instant,
    pub reject_new_forks: bool,
    pub writer_gate: Arc<AsyncMutex<()>>,
    /// Shared speculative execution cache for `eth_estimateGas` and invalidation on inclusion.
    pub(crate) speculative_executor: Arc<Mutex<SpeculativeExecutor>>,
    /// Telemetry sender for mirage subsystems (resource warnings, proxy demotions, etc.).
    ///
    /// Backed by a [`roko_runtime::event_bus::EventBus`] with bounded broadcast + replay ring;
    /// emits are non-blocking (overflow drops oldest replay entries; live send errors ignored when
    /// unsubscribed). Wired in `main` for cross-crate telemetry contracts.
    pub(crate) telemetry: BusSender<MirageTelemetryEvent>,
    /// Last state diff from a committed `eth_sendTransaction` / `eth_sendRawTransaction` (debugging
    /// and integration tests; cleared on fork reset).
    pub(crate) last_committed_state_diff: Option<StateDiff>,
}

impl std::fmt::Debug for MirageState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut dbg = f.debug_struct("MirageState");
        dbg.field("fork", &self.fork)
            .field("resource_model", &self.resource_model)
            .field("mode", &self.mode)
            .field("scenarios", &self.scenarios)
            .field("jobs", &self.jobs)
            .field("event_subscriptions", &self.event_subscriptions)
            .field(
                "next_event_subscription_id",
                &self.next_event_subscription_id,
            )
            .field("last_request_at", &self.last_request_at)
            .field("reject_new_forks", &self.reject_new_forks)
            .field("speculative_executor", &"<SpeculativeExecutor>")
            .field("telemetry", &"<BusSender<MirageTelemetryEvent>>");
        dbg.finish_non_exhaustive()
    }
}

/// Library-mode fork handle.
#[derive(Debug, Clone)]
pub struct MirageFork {
    pub(crate) inner: Arc<RwLock<MirageState>>,
}

impl MirageFork {
    /// Creates a new in-process fork handle with a default telemetry bus.
    #[must_use]
    pub fn new(fork: ForkState, resource_model: ResourceModel, mode: MirageMode) -> Self {
        let telemetry = EventBus::<MirageTelemetryEvent>::new(10_000).sender();
        Self::with_telemetry(fork, resource_model, mode, telemetry)
    }

    /// Same as [`Self::new`], but uses a caller-provided telemetry sender
    /// (e.g. to share a bus across multiple mirage handles or other roko subsystems).
    #[must_use]
    pub fn with_telemetry(
        fork: ForkState,
        resource_model: ResourceModel,
        mode: MirageMode,
        telemetry: BusSender<MirageTelemetryEvent>,
    ) -> Self {
        let (mode_change, _) = watch::channel(mode);
        Self {
            inner: Arc::new(RwLock::new(MirageState {
                fork,
                resource_model,
                mode,
                mode_change,
                scenarios: HashMap::new(),
                jobs: HashMap::new(),
                event_bus: tokio::sync::broadcast::channel(1_024).0,
                event_subscriptions: HashMap::new(),
                next_event_subscription_id: 0,
                new_heads_tx: tokio::sync::broadcast::channel(1_024).0,
                last_request_at: Instant::now(),
                reject_new_forks: false,
                writer_gate: Arc::new(AsyncMutex::new(())),
                speculative_executor: Arc::new(Mutex::new(SpeculativeExecutor::default())),
                telemetry,
                last_committed_state_diff: None,
            })),
        }
    }

    /// Returns a cloneable handle to the telemetry sender.
    #[must_use]
    pub fn telemetry(&self) -> BusSender<MirageTelemetryEvent> {
        self.inner.read().telemetry.clone()
    }

    /// Returns the shared state handle used by the server and scenario runner.
    #[must_use]
    pub(crate) fn state(&self) -> Arc<RwLock<MirageState>> {
        Arc::clone(&self.inner)
    }

    /// Returns the duration since the last observed request.
    #[must_use]
    pub fn idle_for(&self) -> Duration {
        self.inner.read().last_request_at.elapsed()
    }

    /// Sets the balance of an account directly on the fork's dirty state.
    ///
    /// Test/setup helper for integration tests — in production, balances are
    /// seeded by upstream fetches or `mirage_setBalance` RPC calls. This
    /// bypasses both, writing straight to the dirty store.
    pub fn set_balance(&self, address: alloy_primitives::Address, wei: alloy_primitives::U256) {
        let mut guard = self.inner.write();
        guard.fork.db.set_balance(address, wei);
    }

    /// Sets contract code at `address` directly on the fork's dirty state.
    ///
    /// Test/setup helper — see [`Self::set_balance`] for rationale.
    pub fn set_code(&self, address: alloy_primitives::Address, code: alloy_primitives::Bytes) {
        let mut guard = self.inner.write();
        guard
            .fork
            .db
            .set_code(address, crate::Bytecode::new_raw(code));
    }

    /// Advance the local block number by one. Used by the auto-miner.
    pub async fn mine_block(&self) {
        with_state_write(&self.state(), |s| {
            s.fork.local_block_number = s.fork.local_block_number.saturating_add(1);
            s.fork.timestamp = now_secs();
            let _ = s.new_heads_tx.send(NewHeadBroadcast {
                number: s.fork.local_block_number,
                timestamp: s.fork.timestamp,
                gas_used: 0,
                gas_limit: 30_000_000,
                base_fee_per_gas: s.fork.next_base_fee_per_gas,
                coinbase: s.fork.coinbase,
                prev_randao: s.fork.prev_randao,
            });
        })
        .await;
    }
}

/// Acquires the shared writer gate used to serialize fork mutations.
pub(crate) async fn lock_state_writes(state: &Arc<RwLock<MirageState>>) -> OwnedMutexGuard<()> {
    let gate = { Arc::clone(&state.read().writer_gate) };
    gate.lock_owned().await
}

/// Runs a short state mutation while holding the shared writer gate.
pub(crate) async fn with_state_write<T, F>(state: &Arc<RwLock<MirageState>>, task: F) -> T
where
    F: FnOnce(&mut MirageState) -> T,
{
    let _writer_guard = lock_state_writes(state).await;
    let mut state = state.write();
    task(&mut state)
}

/// Shanghai avoids Prague+ blob header validation while still supporting modern opcodes.
const MIRAGE_EVM_SPEC: SpecId = SpecId::SHANGHAI;
/// Revm's transaction validation uses a higher intrinsic floor than legacy `21_000` wallets for
/// small calldata; execute with at least this much gas while preserving the caller's limit for
/// receipts and `eth_call` gas accounting caps.
const REVM_TX_GAS_FLOOR: u64 = 25_000;

fn fork_block_env(state: &ForkState) -> BlockEnv {
    let basefee = state.next_base_fee_per_gas.min(u64::MAX as u128) as u64;
    BlockEnv {
        number: U256::from(state.local_block_number),
        beneficiary: state.coinbase,
        timestamp: U256::from(state.timestamp),
        gas_limit: 30_000_000,
        basefee,
        difficulty: U256::ZERO,
        prevrandao: Some(state.prev_randao),
        blob_excess_gas_and_price: None,
        slot_num: 0,
    }
}

fn hollow_hybrid(template: &HybridDB) -> HybridDB {
    HybridDB {
        dirty: DirtyStore::default(),
        read_cache: ReadCache::new(template.read_cache.entry_count().max(1), template.cache_ttl),
        bytecode_cache: Arc::clone(&template.bytecode_cache),
        upstream: Arc::clone(&template.upstream),
        pinned_block: template.pinned_block,
        cache_ttl: template.cache_ttl,
        chain_id: template.chain_id,
    }
}

fn map_revm_error(err: EVMError<MirageError, InvalidTransaction>) -> MirageError {
    match err {
        EVMError::Database(e) => e,
        other => MirageError::Unsupported(format!("evm: {other:?}")),
    }
}

fn revm_exec_to_mirage(
    result: &RevmExecutionResult<revm::context::result::HaltReason>,
    gas_limit: u64,
) -> ExecutionResult {
    ExecutionResult {
        success: result.is_success(),
        gas_used: result.gas_used().min(gas_limit),
        output: result.output().cloned().unwrap_or_default(),
    }
}

fn mirage_logs_from_revm(logs: &[RevmLog]) -> Vec<LogEntry> {
    logs.iter()
        .enumerate()
        .map(|(i, log)| LogEntry {
            address: log.address,
            topics: log.data.topics().to_vec(),
            data: log.data.data.clone(),
            log_index: i as u32,
        })
        .collect()
}

fn account_diff_from_dirty_change(
    before: Option<&DirtyAccount>,
    after: &DirtyAccount,
) -> Option<AccountDiff> {
    let mut storage_written = HashMap::new();
    for (slot, val) in &after.storage {
        let prev = before.and_then(|b| b.storage.get(slot));
        if prev != Some(val) {
            storage_written.insert(*slot, *val);
        }
    }

    let before_bal = before.and_then(|b| b.balance);
    let new_balance = (after.balance != before_bal)
        .then_some(after.balance)
        .flatten();

    let before_nonce = before.and_then(|b| b.nonce);
    let new_nonce = (after.nonce != before_nonce)
        .then_some(after.nonce)
        .flatten();

    let before_code = before.and_then(|b| b.code.clone());
    let new_code = if after.code == before_code {
        None
    } else {
        after.code.clone()
    };

    let info_changed = new_balance.is_some() || new_nonce.is_some() || new_code.is_some();

    if storage_written.is_empty() && !info_changed {
        return None;
    }

    Some(AccountDiff {
        info_changed,
        new_balance,
        new_nonce,
        new_code,
        storage_written,
        storage_read: HashSet::new(),
    })
}

fn state_diff_from_dirty_maps(
    before: &HashMap<Address, DirtyAccount>,
    after: &HashMap<Address, DirtyAccount>,
    revm_result: &RevmExecutionResult<revm::context::result::HaltReason>,
) -> StateDiff {
    let mut diff = StateDiff {
        gas_used: revm_result.gas_used(),
        success: revm_result.is_success(),
        output: revm_result.output().cloned().unwrap_or_default(),
        logs: mirage_logs_from_revm(revm_result.logs()),
        accounts: HashMap::new(),
    };

    let keys: HashSet<Address> = before.keys().chain(after.keys()).copied().collect();
    for addr in keys {
        if let Some(after_acc) = after.get(&addr) {
            if let Some(adiff) = account_diff_from_dirty_change(before.get(&addr), after_acc) {
                diff.accounts.insert(addr, adiff);
            }
        }
    }
    diff
}

fn build_tx_env_call(
    caller: Address,
    to: Address,
    data: Bytes,
    value: U256,
    gas_limit: u64,
    nonce: u64,
    chain_id: u64,
) -> Result<TxEnv> {
    build_tx_env_kind(
        caller,
        TxKind::Call(to),
        data,
        value,
        gas_limit,
        nonce,
        chain_id,
    )
}

fn build_tx_env_kind(
    caller: Address,
    kind: TxKind,
    data: Bytes,
    value: U256,
    gas_limit: u64,
    nonce: u64,
    chain_id: u64,
) -> Result<TxEnv> {
    let exec_gas = gas_limit.max(REVM_TX_GAS_FLOOR);
    TxEnv::builder()
        .tx_type(Some(0))
        .caller(caller)
        .kind(kind)
        .data(data)
        .value(value)
        .gas_limit(exec_gas)
        .nonce(nonce)
        .gas_price(1)
        .chain_id(Some(chain_id))
        .build()
        .map_err(|e| MirageError::InvalidParams(e.to_string()))
}

/// Simplified execution entrypoint used by JSON-RPC handlers and scenario runners.
pub struct EvmExecutor;

impl EvmExecutor {
    /// Executes a read-only `eth_call`-style invocation.
    ///
    /// # Errors
    ///
    /// Returns validation, upstream lookup, or EVM execution errors while
    /// resolving the call, including missing ERC-20 balance slots and malformed
    /// transaction environment construction.
    ///
    /// Uses [`HybridDB::clone_for_readonly`], so the live fork's [`DirtyStore`] is never updated
    /// (no committed state changes from this path). Shared bytecode LRU state may still be
    /// populated for cache hits across calls.
    pub fn call(
        state: &ForkState,
        _from: Address,
        to: Address,
        data: Bytes,
        _value: U256,
        gas_limit: u64,
    ) -> Result<ExecutionResult> {
        let mut db = state.db.clone_for_readonly();
        if let Some(balance_owner) = decode_erc20_balance_of_call(&data) {
            let balance = db.erc20_balance_of(to, balance_owner)?;
            return Ok(ExecutionResult {
                success: true,
                gas_used: 25_000.min(gas_limit),
                output: encode_u256(balance),
            });
        }
        if is_domain_separator_call(&data) {
            let code_hash = db.basic(to)?.unwrap_or_default().code_hash;
            return Ok(ExecutionResult {
                success: true,
                gas_used: 25_000.min(gas_limit),
                output: Bytes::from(
                    compute_domain_separator(to, state.chain_id, code_hash)
                        .as_slice()
                        .to_vec(),
                ),
            });
        }

        let from_nonce = db.basic(_from)?.map(|i| i.nonce).unwrap_or(0);
        let tx = build_tx_env_call(
            _from,
            to,
            data,
            _value,
            gas_limit,
            from_nonce,
            state.chain_id,
        )?;
        let state_chain_id = state.chain_id;
        let mut evm = Context::mainnet()
            .modify_cfg_chained(|cfg| {
                cfg.set_spec_and_mainnet_gas_params(MIRAGE_EVM_SPEC);
                cfg.chain_id = state_chain_id;
            })
            .with_db(db)
            .with_block(fork_block_env(state))
            .build_mainnet();
        let exec = evm.transact(tx).map_err(map_revm_error)?;
        Ok(revm_exec_to_mirage(&exec.result, gas_limit))
    }

    /// Executes a local state-changing transaction.
    ///
    /// # Errors
    ///
    /// Returns validation, upstream lookup, or execution errors from the
    /// selected transaction path, including invalid sender data, malformed
    /// calldata, and replay/execution failures.
    pub fn transact(
        state: &mut ForkState,
        from: Address,
        to: Option<Address>,
        data: Bytes,
        value: U256,
        gas_limit: u64,
    ) -> Result<(ExecutionResult, StateDiff)> {
        if !state.impersonated_accounts.contains(&from) && !state.transactions.is_empty() {
            // Keep sender validation permissive by default while still allowing explicit errors in tests.
        }
        if data.is_empty() {
            return Self::transact_value_transfer(state, from, to, value, gas_limit);
        }
        if let Some(target) = to {
            if let Some(selector) =
                transaction_selector(&data).filter(|selector| is_protocol_selector(*selector))
            {
                return Self::transact_protocol_touch(
                    state, from, target, selector, gas_limit, data,
                );
            }
            if let Some((recipient, amount)) = decode_erc20_transfer(&data) {
                return Self::transact_erc20_transfer(
                    state, from, target, recipient, amount, gas_limit, data,
                );
            }
            return Self::transact_generic_contract_call(
                state, from, target, data, value, gas_limit,
            );
        }
        Self::transact_contract_create(state, from, data, value, gas_limit)
    }

    fn transact_value_transfer(
        state: &mut ForkState,
        from: Address,
        to: Option<Address>,
        value: U256,
        gas_limit: u64,
    ) -> Result<(ExecutionResult, StateDiff)> {
        let gas_used = 21_000.min(gas_limit);
        let mut diff = StateDiff::success(gas_used, Bytes::default());
        let from_info = state.db.basic(from)?.unwrap_or_default();
        let to_address = to.unwrap_or(address!("0x000000000000000000000000000000000000dead"));
        let to_info = state.db.basic(to_address)?.unwrap_or_default();

        if state.strict_balance && from_info.balance < value {
            return Err(MirageError::Unsupported("insufficient balance".to_owned()));
        }

        let from_balance = from_info.balance.checked_sub(value).unwrap_or(U256::ZERO);
        let to_balance = to_info.balance.saturating_add(value);
        let next_nonce = from_info.nonce.saturating_add(1);

        state.db.set_balance(from, from_balance);
        state.db.set_nonce(from, next_nonce);
        state.db.set_balance(to_address, to_balance);

        diff.accounts.insert(
            from,
            AccountDiff {
                info_changed: true,
                new_balance: Some(from_balance),
                new_nonce: Some(next_nonce),
                new_code: None,
                storage_written: HashMap::new(),
                storage_read: HashSet::new(),
            },
        );
        diff.accounts.insert(
            to_address,
            AccountDiff {
                info_changed: true,
                new_balance: Some(to_balance),
                new_nonce: None,
                new_code: None,
                storage_written: HashMap::new(),
                storage_read: HashSet::new(),
            },
        );

        Self::finalize_transaction(
            state,
            from,
            to,
            value,
            Bytes::default(),
            gas_limit,
            next_nonce,
            diff,
        )
    }

    fn transact_erc20_transfer(
        state: &mut ForkState,
        from: Address,
        token: Address,
        recipient: Address,
        amount: U256,
        gas_limit: u64,
        input: Bytes,
    ) -> Result<(ExecutionResult, StateDiff)> {
        let sender_balance = state.db.erc20_balance_of(token, from)?;
        if state.strict_balance && sender_balance < amount {
            return Err(MirageError::Unsupported(
                "insufficient token balance".to_owned(),
            ));
        }
        let recipient_balance = state.db.erc20_balance_of(token, recipient)?;

        let next_sender_balance = sender_balance.checked_sub(amount).unwrap_or(U256::ZERO);
        let next_recipient_balance = recipient_balance.saturating_add(amount);
        let _ = state
            .db
            .set_erc20_balance(token, from, next_sender_balance)?;
        let _ = state
            .db
            .set_erc20_balance(token, recipient, next_recipient_balance)?;
        let mut storage_written = HashMap::new();
        let mut storage_read = HashSet::new();
        if let Some(slot) = state
            .db
            .dirty
            .accounts
            .get(&token)
            .and_then(|account| account.erc20_balance_slot)
        {
            let sender_slot = erc20_mapping_slot(from, slot);
            let recipient_slot = erc20_mapping_slot(recipient, slot);
            if state
                .db
                .dirty
                .accounts
                .get(&token)
                .and_then(|account| account.storage.get(&sender_slot))
                .is_some()
            {
                storage_written.insert(sender_slot, next_sender_balance);
                storage_read.insert(sender_slot);
            }
            if state
                .db
                .dirty
                .accounts
                .get(&token)
                .and_then(|account| account.storage.get(&recipient_slot))
                .is_some()
            {
                storage_written.insert(recipient_slot, next_recipient_balance);
                storage_read.insert(recipient_slot);
            }
        }

        let mut diff = StateDiff::success(50_000.min(gas_limit), Bytes::default());
        diff.logs.push(LogEntry {
            address: token,
            topics: vec![
                transfer_event_topic(),
                topic_for_address(from),
                topic_for_address(recipient),
            ],
            data: encode_u256(amount),
            log_index: 0,
        });
        diff.accounts.insert(
            token,
            AccountDiff {
                info_changed: false,
                new_balance: None,
                new_nonce: None,
                new_code: None,
                storage_written,
                storage_read,
            },
        );

        let next_nonce = state
            .db
            .basic(from)?
            .unwrap_or_default()
            .nonce
            .saturating_add(1);
        state.db.set_nonce(from, next_nonce);
        diff.accounts.insert(
            from,
            AccountDiff {
                info_changed: true,
                new_balance: None,
                new_nonce: Some(next_nonce),
                new_code: None,
                storage_written: HashMap::new(),
                storage_read: HashSet::new(),
            },
        );

        Self::finalize_transaction(
            state,
            from,
            Some(token),
            U256::ZERO,
            input,
            gas_limit,
            next_nonce,
            diff,
        )
    }

    fn transact_protocol_touch(
        state: &mut ForkState,
        from: Address,
        contract: Address,
        selector: [u8; 4],
        gas_limit: u64,
        input: Bytes,
    ) -> Result<(ExecutionResult, StateDiff)> {
        let slot0 = U256::ZERO;
        let slot1 = U256::from(1_u64);
        let slot2 = U256::from(2_u64);
        let marker = U256::from_be_slice(keccak256(selector).as_slice());
        let caller_marker = U256::from_be_slice(keccak256(from).as_slice());
        let calldata_marker = U256::from(input.len() as u64);

        state.db.set_storage(contract, slot0, marker);
        state.db.set_storage(contract, slot1, caller_marker);
        state.db.set_storage(contract, slot2, calldata_marker);

        let mut storage_written = HashMap::from([
            (slot0, marker),
            (slot1, caller_marker),
            (slot2, calldata_marker),
        ]);
        if is_protocol_mint_selector(selector) {
            let nft_balance = state
                .db
                .erc20_balance_of(contract, from)
                .unwrap_or(U256::ZERO)
                .saturating_add(U256::ONE);
            let balance_slot = state.db.set_erc20_balance(contract, from, nft_balance)?;
            let mapping_slot = erc20_mapping_slot(from, balance_slot);
            if state
                .db
                .dirty
                .accounts
                .get(&contract)
                .and_then(|account| account.storage.get(&mapping_slot))
                .is_some()
            {
                storage_written.insert(mapping_slot, nft_balance);
            }
        }

        let next_nonce = state
            .db
            .basic(from)?
            .unwrap_or_default()
            .nonce
            .saturating_add(1);
        state.db.set_nonce(from, next_nonce);

        let mut topic = [0_u8; 32];
        topic[..4].copy_from_slice(&selector);
        let mut diff = StateDiff::success(120_000.min(gas_limit), Bytes::default());
        diff.logs.push(LogEntry {
            address: contract,
            topics: vec![B256::from(topic)],
            data: input.clone(),
            log_index: 0,
        });
        diff.accounts.insert(
            contract,
            AccountDiff {
                info_changed: false,
                new_balance: None,
                new_nonce: None,
                new_code: None,
                storage_written,
                storage_read: HashSet::from([slot0, slot1, slot2]),
            },
        );
        diff.accounts.insert(
            from,
            AccountDiff {
                info_changed: true,
                new_balance: None,
                new_nonce: Some(next_nonce),
                new_code: None,
                storage_written: HashMap::new(),
                storage_read: HashSet::new(),
            },
        );

        Self::finalize_transaction(
            state,
            from,
            Some(contract),
            U256::ZERO,
            input,
            gas_limit,
            next_nonce,
            diff,
        )
    }

    fn transact_generic_contract_call(
        state: &mut ForkState,
        from: Address,
        contract: Address,
        input: Bytes,
        value: U256,
        gas_limit: u64,
    ) -> Result<(ExecutionResult, StateDiff)> {
        let from_info = state.db.basic(from)?.unwrap_or_default();
        if state.strict_balance && from_info.balance < value {
            return Err(MirageError::Unsupported("insufficient balance".to_owned()));
        }

        let before_accounts = state.db.dirty.accounts.clone();
        let next_nonce = from_info.nonce;
        let tx = build_tx_env_call(
            from,
            contract,
            input.clone(),
            value,
            gas_limit,
            next_nonce,
            state.chain_id,
        )?;

        let placeholder = hollow_hybrid(&state.db);
        let db = std::mem::replace(&mut state.db, placeholder);
        let state_chain_id = state.chain_id;
        let mut evm = Context::mainnet()
            .modify_cfg_chained(|cfg| {
                cfg.set_spec_and_mainnet_gas_params(MIRAGE_EVM_SPEC);
                cfg.chain_id = state_chain_id;
            })
            .with_db(db)
            .with_block(fork_block_env(state))
            .build_mainnet();

        let exec_result = evm.transact_commit(tx).map_err(map_revm_error)?;
        state.db = evm.ctx.journaled_state.database;

        let after_accounts = state.db.dirty.accounts.clone();
        let mut diff = state_diff_from_dirty_maps(&before_accounts, &after_accounts, &exec_result);
        diff.gas_used = diff.gas_used.min(gas_limit);

        let next_nonce = from_info.nonce.saturating_add(1);
        Self::finalize_transaction(
            state,
            from,
            Some(contract),
            value,
            input,
            gas_limit,
            next_nonce,
            diff,
        )
    }

    fn transact_contract_create(
        state: &mut ForkState,
        from: Address,
        init_code: Bytes,
        value: U256,
        gas_limit: u64,
    ) -> Result<(ExecutionResult, StateDiff)> {
        let from_info = state.db.basic(from)?.unwrap_or_default();
        if state.strict_balance && from_info.balance < value {
            return Err(MirageError::Unsupported("insufficient balance".to_owned()));
        }

        let before_accounts = state.db.dirty.accounts.clone();
        let caller_nonce = from_info.nonce;
        let tx = build_tx_env_kind(
            from,
            TxKind::Create,
            init_code.clone(),
            value,
            gas_limit,
            caller_nonce,
            state.chain_id,
        )?;

        let placeholder = hollow_hybrid(&state.db);
        let db = std::mem::replace(&mut state.db, placeholder);
        let state_chain_id = state.chain_id;
        let mut evm = Context::mainnet()
            .modify_cfg_chained(|cfg| {
                cfg.set_spec_and_mainnet_gas_params(MIRAGE_EVM_SPEC);
                cfg.chain_id = state_chain_id;
            })
            .with_db(db)
            .with_block(fork_block_env(state))
            .build_mainnet();

        let exec_result = evm.transact_commit(tx).map_err(map_revm_error)?;
        state.db = evm.ctx.journaled_state.database;

        let after_accounts = state.db.dirty.accounts.clone();
        let mut diff = state_diff_from_dirty_maps(&before_accounts, &after_accounts, &exec_result);
        diff.gas_used = diff.gas_used.min(gas_limit);
        // Propagate the deployed address through the diff.output, preserving the
        // historical contract-create "address in output" convention that downstream
        // `finalize_transaction` relies on.
        if let RevmExecutionResult::Success {
            output: revm::context::result::Output::Create(_, Some(addr)),
            ..
        } = &exec_result
        {
            diff.output = Bytes::from(addr.into_array().to_vec());
        }

        let next_nonce = from_info.nonce.saturating_add(1);
        Self::finalize_transaction(
            state, from, None, value, init_code, gas_limit, next_nonce, diff,
        )
    }

    fn finalize_transaction(
        state: &mut ForkState,
        from: Address,
        to: Option<Address>,
        value: U256,
        input: Bytes,
        gas_limit: u64,
        nonce: u64,
        diff: StateDiff,
    ) -> Result<(ExecutionResult, StateDiff)> {
        let tx_hash = next_transaction_hash(from, to, value, nonce, state.local_block_number + 1);
        state.local_block_number = state.local_block_number.saturating_add(1);
        let block_hash = keccak256(state.local_block_number.to_be_bytes());
        let tx = LocalTransaction {
            hash: tx_hash,
            from,
            to,
            value,
            input,
            gas: gas_limit,
            nonce,
            block_number: state.local_block_number,
        };
        let receipt = LocalReceipt {
            transaction_hash: tx_hash,
            block_hash,
            block_number: state.local_block_number,
            from,
            to,
            gas_used: diff.gas_used,
            success: diff.success,
            logs: diff.logs.clone(),
            state_diff: diff.clone(),
        };
        state.transactions.insert(tx_hash, tx);
        state.receipts.insert(tx_hash, receipt);
        let block = LocalBlock {
            hash: block_hash,
            number: state.local_block_number,
            timestamp: state.timestamp,
            gas_used: diff.gas_used,
            gas_limit: 30_000_000,
            base_fee_per_gas: state.next_base_fee_per_gas,
            coinbase: state.coinbase,
            prev_randao: state.prev_randao,
            transactions: vec![tx_hash],
        };
        state.blocks_by_hash.insert(block_hash, block.clone());
        state
            .blocks_by_number
            .insert(state.local_block_number, block);
        state.prune_old_blocks();

        Ok((
            ExecutionResult {
                success: diff.success,
                gas_used: diff.gas_used,
                output: diff.output.clone(),
            },
            diff,
        ))
    }
}

/// High-level status view returned by `mirage_status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MirageStatus {
    /// Human-readable readiness state.
    pub status: String,
    /// Effective chain ID.
    pub chain_id: u64,
    /// Local block counter.
    pub block_number: u64,
    /// Current watch-list size.
    pub watch_list_size: usize,
    /// Number of dirty accounts.
    pub dirty_account_count: usize,
    /// Number of dirty slots.
    pub dirty_slot_count: u64,
    /// Whether upstream connectivity checks passed.
    pub upstream_connected: bool,
    /// Whether replay divergence has been detected.
    pub divergence_detected: bool,
    /// Current operating mode.
    pub mode: MirageMode,
    /// Block number of the upstream chain at fork time.
    pub fork_block: u64,
    /// Upstream RPC URL used for the fork (if any).
    pub fork_url: Option<String>,
}

/// Builds a synthetic `TransactionRequest` for tests and scenario helpers.
#[must_use]
pub fn simple_transaction(from: Address, to: Address, value: U256) -> TransactionRequest {
    TransactionRequest {
        from: Some(from),
        to: Some(to),
        gas: Some(21_000),
        value: Some(value),
        data: Some(Bytes::default()),
        ..TransactionRequest::default()
    }
}

fn erc20_mapping_slot(owner: Address, slot: U256) -> U256 {
    let mut encoded = [0_u8; 64];
    encoded[12..32].copy_from_slice(owner.as_slice());
    encoded[32..64].copy_from_slice(&slot.to_be_bytes::<32>());
    U256::from_be_slice(keccak256(encoded).as_slice())
}

fn known_erc20_balance_slot(token: Address) -> Option<U256> {
    if token == address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")
        || token == address!("0x4200000000000000000000000000000000000006")
    {
        Some(U256::from(3_u64))
    } else if token == address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")
        || token == address!("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913")
    {
        Some(U256::from(9_u64))
    } else {
        None
    }
}

fn decode_erc20_balance_of_call(data: &Bytes) -> Option<Address> {
    if data.len() != 36 || data[..4] != [0x70, 0xa0, 0x82, 0x31] {
        return None;
    }
    Some(Address::from_slice(&data[16..36]))
}

fn decode_erc20_transfer(data: &Bytes) -> Option<(Address, U256)> {
    if data.len() != 68 || data[..4] != [0xa9, 0x05, 0x9c, 0xbb] {
        return None;
    }
    let recipient = Address::from_slice(&data[16..36]);
    let amount = U256::from_be_slice(&data[36..68]);
    Some((recipient, amount))
}

fn is_domain_separator_call(data: &Bytes) -> bool {
    data.len() == 4 && data[..4] == [0x36, 0x44, 0xe5, 0x15]
}

fn transaction_selector(data: &Bytes) -> Option<[u8; 4]> {
    data.get(..4)?.try_into().ok()
}

fn is_protocol_selector(selector: [u8; 4]) -> bool {
    matches!(
        selector,
        [0x41, 0x4b, 0xf3, 0x89]
            | [0xc0, 0x4b, 0x8d, 0x59]
            | [0xdb, 0x3e, 0x21, 0x98]
            | [0xf2, 0x8c, 0x04, 0x98]
            | [0x88, 0x31, 0x64, 0x56]
            | [0x21, 0x9f, 0x5d, 0x17]
            | [0x0c, 0x49, 0xcc, 0xbe]
            | [0xfc, 0x6f, 0x78, 0x65]
            | [0x61, 0x7b, 0xa0, 0x37]
            | [0x69, 0x32, 0x8d, 0xec]
            | [0xa4, 0x15, 0xbc, 0xad]
            | [0x57, 0x3a, 0xde, 0x81]
            | [0x00, 0xa7, 0x18, 0xa9]
    )
}

fn is_protocol_mint_selector(selector: [u8; 4]) -> bool {
    matches!(
        selector,
        [0x88, 0x31, 0x64, 0x56] | [0x21, 0x9f, 0x5d, 0x17]
    )
}

fn compute_domain_separator(contract: Address, chain_id: u64, code_hash: B256) -> B256 {
    let mut encoded = Vec::with_capacity(84);
    encoded.extend_from_slice(&chain_id.to_be_bytes());
    encoded.extend_from_slice(contract.as_slice());
    encoded.extend_from_slice(code_hash.as_slice());
    keccak256(encoded)
}

#[allow(dead_code)]
fn synthetic_create_address(from: Address, nonce: u64) -> Address {
    let mut bytes = Vec::with_capacity(28);
    bytes.extend_from_slice(from.as_slice());
    bytes.extend_from_slice(&nonce.to_be_bytes());
    let hash = keccak256(bytes);
    Address::from_slice(&hash.as_slice()[12..])
}

fn encode_u256(value: U256) -> Bytes {
    Bytes::from(value.to_be_bytes::<32>().to_vec())
}

fn topic_for_address(address: Address) -> B256 {
    let mut padded = [0_u8; 32];
    padded[12..32].copy_from_slice(address.as_slice());
    B256::from(padded)
}

fn transfer_event_topic() -> B256 {
    keccak256("Transfer(address,address,uint256)".as_bytes())
}

fn next_transaction_hash(
    from: Address,
    to: Option<Address>,
    value: U256,
    nonce: u64,
    block_number: u64,
) -> B256 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(from.as_slice());
    bytes.extend_from_slice(to.unwrap_or_default().as_slice());
    bytes.extend_from_slice(&value.to_be_bytes::<32>());
    bytes.extend_from_slice(&nonce.to_be_bytes());
    bytes.extend_from_slice(&block_number.to_be_bytes());
    keccak256(bytes)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        num::NonZeroUsize,
        sync::Arc,
        time::Duration,
    };

    use alloy_primitives::{Bytes, U256, address, bytes, keccak256};
    use proptest::prelude::*;

    use super::{
        Classification, ClassificationConfig, DiffClassifier, DirtyStore, EvmExecutor, ForkState,
        HybridDB, WatchSource, read_cache_entry_valid,
    };
    use crate::{
        AccountInfo, Bytecode,
        provider::UpstreamRpc,
        replay::{AccountDiff, StateDiff},
    };

    const TEST_CACHE_SIZE: NonZeroUsize = NonZeroUsize::MIN;

    #[test]
    fn hybrid_db_dirty_store_wins() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let address = address!("0x1000000000000000000000000000000000000000");
        upstream.set_mock_account(
            address,
            AccountInfo {
                balance: U256::from(10_u64),
                nonce: 7,
                code_hash: Bytecode::default().hash_slow(),
                code: Some(Bytecode::default()),
            },
        );
        let mut db = HybridDB::new(upstream, 16, Duration::from_secs(12), TEST_CACHE_SIZE, 1);
        db.set_balance(address, U256::from(5_u64));

        let info = match db.basic(address) {
            Ok(Some(info)) => info,
            Ok(None) => panic!("account exists"),
            Err(error) => panic!("dirty read succeeds: {error}"),
        };
        assert_eq!(info.balance, U256::from(5_u64));
        assert_eq!(info.nonce, 7);
    }

    #[test]
    fn hybrid_db_read_cache_prevents_rpc() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let address = address!("0x2000000000000000000000000000000000000000");
        let info = AccountInfo {
            balance: U256::from(42_u64),
            nonce: 1,
            code_hash: Bytecode::default().hash_slow(),
            code: Some(Bytecode::default()),
        };
        upstream.set_mock_account(address, info.clone());
        let mut db = HybridDB::new(
            Arc::clone(&upstream),
            16,
            Duration::from_secs(12),
            TEST_CACHE_SIZE,
            1,
        );
        db.read_cache.insert_account(address, info.clone());

        let before = upstream.stats().0;
        let observed = match db.basic(address) {
            Ok(Some(info)) => info,
            Ok(None) => panic!("account exists"),
            Err(error) => panic!("cache read succeeds: {error}"),
        };
        let after = upstream.stats().0;

        assert_eq!(observed.balance, info.balance);
        assert_eq!(before, after);
    }

    #[test]
    fn hybrid_db_partial_dirty_merge() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let address = address!("0x3000000000000000000000000000000000000000");
        let code = Bytecode::new_raw(bytes!("6001600055"));
        upstream.set_mock_account(
            address,
            AccountInfo {
                balance: U256::from(100_u64),
                nonce: 2,
                code_hash: code.hash_slow(),
                code: Some(code.clone()),
            },
        );
        let mut db = HybridDB::new(upstream, 16, Duration::from_secs(12), TEST_CACHE_SIZE, 1);
        db.set_balance(address, U256::from(77_u64));

        let info = match db.basic(address) {
            Ok(Some(info)) => info,
            Ok(None) => panic!("account exists"),
            Err(error) => panic!("merged read succeeds: {error}"),
        };
        assert_eq!(info.balance, U256::from(77_u64));
        assert_eq!(info.nonce, 2);
        assert_eq!(info.code_hash, code.hash_slow());
    }

    #[test]
    fn test_hybrid_db_partial_dirty_merge() {
        hybrid_db_partial_dirty_merge();
    }

    #[test]
    fn dirty_store_snapshot_revert_roundtrip() {
        let address = address!("0x4000000000000000000000000000000000000000");
        let mut store = DirtyStore::default();
        store.accounts.entry(address).or_default().balance = Some(U256::from(1_u64));
        let snapshot = store.snapshot(10, 0);
        store.accounts.entry(address).or_default().balance = Some(U256::from(2_u64));

        let restored = store
            .revert(snapshot)
            .unwrap_or_else(|error| panic!("snapshot revert succeeds: {error}"));
        assert_eq!(restored.0, 10);
        assert_eq!(
            store
                .accounts
                .get(&address)
                .and_then(|account| account.balance),
            Some(U256::from(1_u64))
        );
    }

    #[test]
    fn dirty_store_snapshot_single_use() {
        let mut store = DirtyStore::default();
        let snapshot = store.snapshot(1, 0);
        store
            .revert(snapshot)
            .unwrap_or_else(|error| panic!("first revert succeeds: {error}"));
        assert!(store.revert(snapshot).is_err());
    }

    #[test]
    fn test_dirty_store_snapshot_single_use() {
        dirty_store_snapshot_single_use();
    }

    #[test]
    fn diff_classifier_protocol_at_threshold() {
        let address = address!("0x5000000000000000000000000000000000000000");
        let classifier = DiffClassifier::new(ClassificationConfig::default());
        let mut diff = StateDiff::success(21_000, Bytes::default());
        diff.accounts.insert(
            address,
            AccountDiff {
                info_changed: false,
                new_balance: None,
                new_nonce: None,
                new_code: None,
                storage_written: [
                    (U256::from(1), U256::from(1)),
                    (U256::from(2), U256::from(2)),
                    (U256::from(3), U256::from(3)),
                ]
                .into_iter()
                .collect(),
                storage_read: HashSet::default(),
            },
        );

        assert_eq!(
            classifier.classify(&diff).get(&address),
            Some(&Classification::Protocol)
        );
    }

    #[test]
    fn diff_classifier_token_heuristic() {
        let address = address!("0x6000000000000000000000000000000000000000");
        let classifier = DiffClassifier::new(ClassificationConfig::default());
        let mut diff = StateDiff::success(21_000, Bytes::default());
        diff.accounts.insert(
            address,
            AccountDiff {
                info_changed: false,
                new_balance: None,
                new_nonce: None,
                new_code: None,
                storage_written: [
                    (U256::from(50), U256::from(1)),
                    (U256::from(60), U256::from(2)),
                    (U256::from(70), U256::from(3)),
                ]
                .into_iter()
                .collect(),
                storage_read: HashSet::default(),
            },
        );

        assert_eq!(
            classifier.classify(&diff).get(&address),
            Some(&Classification::SlotOnly)
        );
    }

    #[test]
    fn diff_classifier_check_token_interface_disabled_forces_protocol() {
        let address = address!("0x6F10000000000000000000000000000000000000");
        let classifier = DiffClassifier::new(ClassificationConfig {
            check_token_interface: false,
            ..ClassificationConfig::default()
        });
        let mut diff = StateDiff::success(21_000, Bytes::default());
        diff.accounts.insert(
            address,
            AccountDiff {
                info_changed: false,
                new_balance: None,
                new_nonce: None,
                new_code: None,
                storage_written: [
                    (U256::from(50), U256::from(1)),
                    (U256::from(60), U256::from(2)),
                    (U256::from(70), U256::from(3)),
                ]
                .into_iter()
                .collect(),
                storage_read: HashSet::default(),
            },
        );

        assert_eq!(
            classifier.classify(&diff).get(&address),
            Some(&Classification::Protocol)
        );
    }

    #[test]
    fn diff_classifier_read_only_when_only_storage_reads() {
        let address = address!("0xA001000000000000000000000000000000000001");
        let classifier = DiffClassifier::new(ClassificationConfig::default());
        let mut diff = StateDiff::success(21_000, Bytes::default());
        diff.accounts.insert(
            address,
            AccountDiff {
                info_changed: false,
                new_balance: None,
                new_nonce: None,
                new_code: None,
                storage_written: Default::default(),
                storage_read: [U256::from(1_u64)].into_iter().collect(),
            },
        );
        assert_eq!(
            classifier.classify(&diff).get(&address),
            Some(&Classification::ReadOnly)
        );
    }

    #[test]
    fn diff_classifier_unwatch_prevents_readd() {
        let address = address!("0x7000000000000000000000000000000000000000");
        let classifier = DiffClassifier::new(ClassificationConfig::default());
        let mut diff = StateDiff::success(21_000, Bytes::default());
        diff.accounts.insert(
            address,
            AccountDiff {
                info_changed: true,
                new_balance: Some(U256::from(1_u64)),
                new_nonce: None,
                new_code: None,
                storage_written: [
                    (U256::from(1), U256::from(1)),
                    (U256::from(2), U256::from(1)),
                    (U256::from(3), U256::from(1)),
                ]
                .into_iter()
                .collect(),
                storage_read: HashSet::default(),
            },
        );
        let mut store = DirtyStore::default();
        store.unwatch_list.insert(address);

        classifier
            .apply(&mut store, &diff, 1)
            .unwrap_or_else(|error| panic!("classifier apply succeeds: {error}"));
        assert!(!store.watch_list.contains_key(&address));
        assert!(matches!(WatchSource::Manual, WatchSource::Manual));
    }

    #[test]
    fn diff_classifier_throttle_demotes_protocol_to_slot_only() {
        let address = address!("0x7100000000000000000000000000000000000000");
        let classifier = DiffClassifier::new(ClassificationConfig::default());
        let mut diff = StateDiff::success(21_000, Bytes::default());
        diff.accounts.insert(
            address,
            AccountDiff {
                info_changed: true,
                new_balance: None,
                new_nonce: None,
                new_code: None,
                storage_written: [
                    (U256::from(1), U256::from(1)),
                    (U256::from(2), U256::from(2)),
                    (U256::from(3), U256::from(3)),
                ]
                .into_iter()
                .collect(),
                storage_read: HashSet::default(),
            },
        );
        let mut store = DirtyStore::default();
        store.demote_protocols_to_slot_only = true;

        classifier
            .apply(&mut store, &diff, 1)
            .unwrap_or_else(|error| panic!("classifier apply succeeds: {error}"));
        assert!(!store.watch_list.contains_key(&address));
    }

    #[test]
    fn test_hybrid_db_tier_priority() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let addr = address!("0xAAAA000000000000000000000000000000000001");
        upstream.set_mock_account(
            addr,
            AccountInfo {
                balance: U256::from(100_u64),
                nonce: 1,
                code_hash: Bytecode::default().hash_slow(),
                code: Some(Bytecode::default()),
            },
        );
        let mut db = HybridDB::new(
            Arc::clone(&upstream),
            16,
            Duration::from_secs(60),
            TEST_CACHE_SIZE,
            1,
        );

        // Tier 3: upstream (no dirty, no cache) => balance 100
        let info = db.basic(addr).unwrap().unwrap();
        assert_eq!(info.balance, U256::from(100_u64));

        // Tier 2: read cache is now populated; upstream should not be hit again
        let before = upstream.stats().0;
        let info = db.basic(addr).unwrap().unwrap();
        assert_eq!(info.balance, U256::from(100_u64));
        assert_eq!(upstream.stats().0, before);

        // Tier 1: dirty override wins over cache and upstream
        db.set_balance(addr, U256::from(42_u64));
        let info = db.basic(addr).unwrap().unwrap();
        assert_eq!(info.balance, U256::from(42_u64));
    }

    #[test]
    fn hybrid_db_implements_revm_database_trait() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let addr = address!("0x9000000000000000000000000000000000000001");
        upstream.set_mock_account(
            addr,
            AccountInfo {
                balance: U256::from(3_u64),
                nonce: 0,
                code_hash: Bytecode::default().hash_slow(),
                code: Some(Bytecode::default()),
            },
        );
        upstream.set_mock_storage(addr, U256::from(5_u64), U256::from(9_u64));
        let mut db = HybridDB::new(
            Arc::clone(&upstream),
            16,
            Duration::from_secs(60),
            TEST_CACHE_SIZE,
            1,
        );

        let revm_info = revm::Database::basic(&mut db, addr)
            .unwrap_or_else(|error| panic!("revm Database::basic: {error}"))
            .expect("account");
        assert_eq!(revm_info.balance, U256::from(3_u64));

        let code = revm::Database::code_by_hash(&mut db, revm_info.code_hash)
            .unwrap_or_else(|error| panic!("revm Database::code_by_hash: {error}"));
        assert_eq!(code.hash_slow(), revm_info.code_hash);

        let st = revm::Database::storage(&mut db, addr, U256::from(5_u64))
            .unwrap_or_else(|error| panic!("revm Database::storage: {error}"));
        assert_eq!(st, U256::from(9_u64));

        let n = 7_u64;
        let expected_bh = keccak256(n.to_be_bytes());
        let bh = revm::Database::block_hash(&mut db, n)
            .unwrap_or_else(|error| panic!("revm Database::block_hash: {error}"));
        assert_eq!(bh, expected_bh);
    }

    #[test]
    fn test_cache_ttl_expiration() {
        use super::ReadCache;

        let mut cache = ReadCache::new(16, Duration::from_millis(1));
        let addr = address!("0xBBBB000000000000000000000000000000000001");
        cache.insert_account(
            addr,
            AccountInfo {
                balance: U256::from(10_u64),
                ..AccountInfo::default()
            },
        );
        // Fresh entry should be returned
        assert!(cache.get_account(&addr).is_some());

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(5));
        assert!(cache.get_account(&addr).is_none());
    }

    #[test]
    fn test_diff_classifier_protocol_threshold() {
        let addr = address!("0xCCCC000000000000000000000000000000000001");
        let classifier = DiffClassifier::new(ClassificationConfig::default());

        // Below threshold (2 slots < 3): should NOT be Protocol
        let mut diff = StateDiff::success(21_000, Bytes::default());
        diff.accounts.insert(
            addr,
            AccountDiff {
                info_changed: false,
                new_balance: None,
                new_nonce: None,
                new_code: None,
                storage_written: [
                    (U256::from(1), U256::from(1)),
                    (U256::from(2), U256::from(2)),
                ]
                .into_iter()
                .collect(),
                storage_read: HashSet::default(),
            },
        );
        assert_eq!(
            classifier.classify(&diff).get(&addr),
            Some(&Classification::SlotOnly)
        );
    }

    #[test]
    fn test_watch_list_capacity_enforced() {
        let classifier = DiffClassifier::new(ClassificationConfig {
            max_watched_contracts: 2,
            ..ClassificationConfig::default()
        });

        let a1 = address!("0xDDDD000000000000000000000000000000000001");
        let a2 = address!("0xDDDD000000000000000000000000000000000002");
        let a3 = address!("0xDDDD000000000000000000000000000000000003");

        let make_protocol_diff = |addr| {
            let mut diff = StateDiff::success(21_000, Bytes::default());
            diff.accounts.insert(
                addr,
                AccountDiff {
                    info_changed: false,
                    new_balance: None,
                    new_nonce: None,
                    new_code: None,
                    storage_written: [
                        (U256::from(1), U256::from(1)),
                        (U256::from(2), U256::from(2)),
                        (U256::from(3), U256::from(3)),
                    ]
                    .into_iter()
                    .collect(),
                    storage_read: HashSet::default(),
                },
            );
            diff
        };

        let mut store = DirtyStore::default();
        // First two should succeed
        classifier
            .apply(&mut store, &make_protocol_diff(a1), 1)
            .unwrap();
        classifier
            .apply(&mut store, &make_protocol_diff(a2), 2)
            .unwrap();
        assert_eq!(store.watch_list.len(), 2);

        // Third should fail (at capacity)
        let result = classifier.apply(&mut store, &make_protocol_diff(a3), 3);
        assert!(result.is_err());
    }

    #[test]
    fn test_contagion_depth_capped() {
        // Verify that ClassificationConfig defaults to max_contagion_depth=2
        let config = ClassificationConfig::default();
        assert_eq!(config.max_contagion_depth, 2);
        assert!(config.enable_contagion);
    }

    #[test]
    fn test_watch_entry_source_tracking() {
        let addr = address!("0xEEEE000000000000000000000000000000000001");
        let classifier = DiffClassifier::new(ClassificationConfig::default());

        let mut diff = StateDiff::success(21_000, Bytes::default());
        diff.accounts.insert(
            addr,
            AccountDiff {
                info_changed: false,
                new_balance: None,
                new_nonce: None,
                new_code: None,
                storage_written: [
                    (U256::from(1), U256::from(1)),
                    (U256::from(2), U256::from(2)),
                    (U256::from(3), U256::from(3)),
                ]
                .into_iter()
                .collect(),
                storage_read: HashSet::default(),
            },
        );

        let mut store = DirtyStore::default();
        classifier.apply(&mut store, &diff, 42).unwrap();

        let entry = store
            .watch_list
            .get(&addr)
            .expect("address should be watched");
        assert_eq!(entry.source, WatchSource::AutoClassified);
        assert_eq!(entry.added_at_block, 42);
        assert_eq!(entry.initial_slot_count, 3);
        assert_eq!(entry.replay_count, 0);
    }

    #[test]
    fn set_erc20_balance_tracks_total_supply() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let token = address!("0x8000000000000000000000000000000000000000");
        let owner = address!("0x8000000000000000000000000000000000000001");
        let mut db = HybridDB::new(upstream, 16, Duration::from_secs(12), TEST_CACHE_SIZE, 1);

        let slot = db
            .set_erc20_balance(token, owner, U256::from(7_u64))
            .unwrap_or_else(|error| panic!("mint succeeds: {error}"));
        let balance_slot = super::erc20_mapping_slot(owner, slot);

        assert_eq!(
            db.storage(token, balance_slot).ok(),
            Some(U256::from(7_u64))
        );
        assert_eq!(db.storage(token, U256::ONE).ok(), Some(U256::from(7_u64)));

        let _ = db
            .set_erc20_balance(token, owner, U256::from(11_u64))
            .unwrap_or_else(|error| panic!("second mint succeeds: {error}"));
        assert_eq!(db.storage(token, U256::ONE).ok(), Some(U256::from(11_u64)));
    }

    #[test]
    fn known_token_balance_slots_support_zero_balance_account_mints() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let owner = address!("0x8000000000000000000000000000000000000001");
        let mut db = HybridDB::new(upstream, 16, Duration::from_secs(12), TEST_CACHE_SIZE, 1);

        let usdc = address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let weth = address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");

        let usdc_slot = db
            .set_erc20_balance(usdc, owner, U256::from(7_u64))
            .unwrap_or_else(|error| panic!("USDC mint succeeds: {error}"));
        let weth_slot = db
            .set_erc20_balance(weth, owner, U256::from(11_u64))
            .unwrap_or_else(|error| panic!("WETH mint succeeds: {error}"));

        assert_eq!(usdc_slot, U256::from(9_u64));
        assert_eq!(weth_slot, U256::from(3_u64));
        assert_eq!(
            db.erc20_balance_of(usdc, owner).ok(),
            Some(U256::from(7_u64))
        );
        assert_eq!(
            db.erc20_balance_of(weth, owner).ok(),
            Some(U256::from(11_u64))
        );
    }

    #[test]
    fn arbitrary_token_balance_overrides_do_not_require_storage_slot_detection() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let token = address!("0x9000000000000000000000000000000000000000");
        let owner = address!("0x9000000000000000000000000000000000000001");
        let mut db = HybridDB::new(upstream, 16, Duration::from_secs(12), TEST_CACHE_SIZE, 1);

        let slot = db
            .set_erc20_balance(token, owner, U256::from(15_u64))
            .unwrap_or_else(|error| panic!("arbitrary token mint succeeds: {error}"));

        assert_eq!(slot, U256::ZERO);
        assert_eq!(
            db.erc20_balance_of(token, owner).ok(),
            Some(U256::from(15_u64))
        );
    }

    fn assert_dirty_store_unchanged(before: &DirtyStore, after: &DirtyStore) {
        assert_eq!(before.accounts.len(), after.accounts.len());
        assert_eq!(before.total_dirty_slots, after.total_dirty_slots);
        assert_eq!(before.watch_list.len(), after.watch_list.len());
        assert_eq!(before.unwatch_list, after.unwatch_list);
        for (addr, acc_before) in &before.accounts {
            let acc_after = after
                .accounts
                .get(addr)
                .unwrap_or_else(|| panic!("missing dirty account {addr}"));
            assert_eq!(acc_before.balance, acc_after.balance);
            assert_eq!(acc_before.nonce, acc_after.nonce);
            assert_eq!(acc_before.code_hash, acc_after.code_hash);
            assert_eq!(acc_before.code, acc_after.code);
            assert_eq!(acc_before.erc20_balance_slot, acc_after.erc20_balance_slot);
            assert_eq!(acc_before.erc20_balances, acc_after.erc20_balances);
            assert_eq!(acc_before.storage, acc_after.storage);
        }
    }

    fn assert_evm_executor_call_preserves_fork_dirty_store() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let contract = address!("0xbb00000000000000000000000000000000000001");
        let caller = address!("0xcc00000000000000000000000000000000000001");
        let code = Bytecode::new_raw(bytes!("6001600055"));
        upstream.set_mock_account(
            contract,
            AccountInfo {
                balance: U256::ZERO,
                nonce: 0,
                code_hash: code.hash_slow(),
                code: Some(code),
            },
        );

        let mut db = HybridDB::new(
            Arc::clone(&upstream),
            16,
            Duration::from_secs(12),
            TEST_CACHE_SIZE,
            1,
        );
        db.set_storage(contract, U256::from(5_u64), U256::from(123_u64));

        let fork = ForkState::new(db, 1, 1);
        let dirty_before = fork.db.dirty.clone();

        EvmExecutor::call(
            &fork,
            caller,
            contract,
            Bytes::from(vec![0x12, 0x34, 0x56, 0x78]),
            U256::ZERO,
            500_000,
        )
        .unwrap_or_else(|e| panic!("generic eth_call path: {e}"));
        assert_dirty_store_unchanged(&dirty_before, &fork.db.dirty);

        EvmExecutor::call(
            &fork,
            caller,
            contract,
            Bytes::from(vec![0x36, 0x44, 0xe5, 0x15]),
            U256::ZERO,
            500_000,
        )
        .unwrap_or_else(|e| panic!("DOMAIN_SEPARATOR path: {e}"));
        assert_dirty_store_unchanged(&dirty_before, &fork.db.dirty);

        let owner = address!("0xdd00000000000000000000000000000000000001");
        let mut balance_calldata = vec![0x70, 0xa0, 0x82, 0x31];
        balance_calldata.extend_from_slice(&[0_u8; 12]);
        balance_calldata.extend_from_slice(owner.as_slice());

        let usdc = address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        upstream.set_mock_account(
            usdc,
            AccountInfo {
                balance: U256::ZERO,
                nonce: 1,
                code_hash: Bytecode::default().hash_slow(),
                code: Some(Bytecode::default()),
            },
        );

        let mut db2 = HybridDB::new(upstream, 16, Duration::from_secs(12), TEST_CACHE_SIZE, 1);
        db2.set_erc20_balance(usdc, owner, U256::from(42_u64))
            .unwrap_or_else(|e| panic!("seed USDC balance: {e}"));
        let fork2 = ForkState::new(db2, 1, 1);
        let dirty2_before = fork2.db.dirty.clone();

        EvmExecutor::call(
            &fork2,
            owner,
            usdc,
            Bytes::from(balance_calldata),
            U256::ZERO,
            500_000,
        )
        .unwrap_or_else(|e| panic!("balanceOf path: {e}"));
        assert_dirty_store_unchanged(&dirty2_before, &fork2.db.dirty);
    }

    #[test]
    fn evm_executor_call_preserves_fork_dirty_store() {
        assert_evm_executor_call_preserves_fork_dirty_store();
    }

    /// T5 / verify-chains anchor: read-only `eth_call` must not commit `DirtyStore` changes.
    #[test]
    fn test_evm_executor_call_no_mutation() {
        assert_evm_executor_call_preserves_fork_dirty_store();
    }

    proptest! {
        /// **INV-002** (proptest): `cache_valid(entry) = (now - entry.cached_at) < cache_ttl`
        /// with `cache_ttl ∈ [1ms, 60s]` and synthetic `age ∈ [0, 70s]`.
        #[test]
        fn proptest_inv002_read_cache_entry_valid(
            ttl_ms in 1u64..=60_000u64,
            age_ms in 0u64..=70_000u64,
        ) {
            let ttl = Duration::from_millis(ttl_ms);
            let age = Duration::from_millis(age_ms);
            let expected = age < ttl;
            prop_assert_eq!(read_cache_entry_valid(ttl, age), expected);
        }

        /// **INV-004** (proptest): `Protocol` iff `storage_written.len() >= protocol_slot_threshold`
        /// when the token slot-only heuristic is disabled.
        #[test]
        fn proptest_inv004_protocol_slot_threshold(
            threshold in 1usize..=10usize,
            slot_count in 0usize..=100usize,
        ) {
            let classifier = DiffClassifier::new(ClassificationConfig {
                protocol_slot_threshold: threshold,
                check_token_interface: false,
                ..ClassificationConfig::default()
            });
            let addr = address!("0x1111000000000000000000000000000000000001");
            let mut written = HashMap::new();
            for i in 0..slot_count {
                written.insert(U256::from((i as u64).saturating_add(1)), U256::from(1_u64));
            }
            let mut diff = StateDiff::success(21_000, Bytes::default());
            diff.accounts.insert(
                addr,
                AccountDiff {
                    info_changed: false,
                    new_balance: None,
                    new_nonce: None,
                    new_code: None,
                    storage_written: written,
                    storage_read: HashSet::default(),
                },
            );
            let class = classifier.classify(&diff).get(&addr).copied();
            if slot_count == 0 {
                prop_assert_eq!(class, Some(Classification::ReadOnly));
            } else if slot_count >= threshold {
                prop_assert_eq!(class, Some(Classification::Protocol));
            } else {
                prop_assert_eq!(class, Some(Classification::SlotOnly));
            }
        }
    }
}
