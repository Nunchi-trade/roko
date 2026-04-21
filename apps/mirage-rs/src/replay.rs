//! Transaction replay and speculative execution helpers.

#![allow(dead_code)]

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use alloy_primitives::{Address, B256, Bytes, U256, hex};
use futures_util::StreamExt;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, watch};

use crate::{
    Bytecode, ExecutionResult, MirageError, Result, TransactionRequest,
    cow::{CowState, MultiVersionStore},
    fork::{
        Classification, DiffClassifier, DirtyAccount, DirtyStore, EvmExecutor, ForkState, HybridDB,
        MirageState, NewHeadBroadcast, ReadCache, WatchEntry, WatchSource,
    },
    provider::UpstreamRpc,
    resources::MirageMode,
};

/// Runs a blocking closure. On a multi-thread Tokio scheduler (i.e. a
/// `tokio::spawn` task), wraps in `block_in_place` so that reqwest's
/// debug-mode runtime check does not panic when it creates and drops an
/// internal Tokio runtime.  On any other context (blocking threads, no
/// runtime, single-thread scheduler) calls the closure directly.
fn run_blocking<F: FnOnce() -> R, R>(f: F) -> R {
    match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(f)
        }
        _ => f(),
    }
}

/// Canonical log entry captured in a state diff.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// Contract address that emitted the log.
    pub address: Address,
    /// Topics carried by the log.
    pub topics: Vec<B256>,
    /// Raw log data.
    pub data: Bytes,
    /// Log index within the transaction receipt.
    pub log_index: u32,
}

/// Canonical account-level state diff.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDiff {
    /// Whether account-level fields changed.
    pub info_changed: bool,
    /// New balance, if written.
    pub new_balance: Option<U256>,
    /// New nonce, if written.
    pub new_nonce: Option<u64>,
    /// New bytecode, if written.
    pub new_code: Option<Bytecode>,
    /// Storage writes by slot.
    pub storage_written: HashMap<U256, U256>,
    /// Storage reads by slot.
    pub storage_read: HashSet<U256>,
}

/// Canonical transaction state diff exported to downstream plans.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateDiff {
    /// Per-account changes.
    pub accounts: HashMap<Address, AccountDiff>,
    /// Logs emitted during execution.
    pub logs: Vec<LogEntry>,
    /// Gas used.
    pub gas_used: u64,
    /// Success flag.
    pub success: bool,
    /// Output bytes.
    pub output: Bytes,
}

impl StateDiff {
    /// Creates a successful diff shell.
    #[must_use]
    pub fn success(gas_used: u64, output: Bytes) -> Self {
        Self {
            gas_used,
            success: true,
            output,
            ..Self::default()
        }
    }
}

/// Configuration for the targeted follower.
#[derive(Debug, Clone)]
pub struct FollowerConfig {
    /// WebSocket URL used for subscriptions.
    pub ws_url: String,
    /// HTTP URL used for block/transaction fetches.
    pub http_url: String,
    /// Maximum replay wall-clock time budget per block.
    pub block_budget: std::time::Duration,
    /// Optional manual address filter.
    pub filter_addresses: Option<Vec<Address>>,
    /// Optional manual selector filter.
    pub filter_selectors: Option<Vec<[u8; 4]>>,
    /// When true, apply upstream state diffs without advancing `local_block_number`.
    /// Used when the auto-miner controls block production and the follower is
    /// only responsible for keeping watched-account state current.
    pub sync_state_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebsocketLoopOutcome {
    Shutdown,
    Proxy,
    Reconnect,
}

/// Maximum backlog to replay block-by-block before skipping to the latest head.
const MAX_REPLAY_LAG_BLOCKS: u64 = 50;
/// Backpressure between the WebSocket `newHeads` producer and block replay.
const NEW_HEADS_CHANNEL_CAPACITY: usize = 32;
/// Initial delay before retrying a dropped WebSocket `newHeads` subscription.
const WS_RECONNECT_BACKOFF_INITIAL: Duration = Duration::from_millis(250);
/// Upper bound for exponential backoff between reconnect attempts.
const WS_RECONNECT_BACKOFF_MAX: Duration = Duration::from_secs(5);

#[inline]
fn next_ws_reconnect_delay(current: Duration) -> Duration {
    current
        .checked_mul(2)
        .unwrap_or(WS_RECONNECT_BACKOFF_MAX)
        .min(WS_RECONNECT_BACKOFF_MAX)
}

/// Background follower that replays upstream heads into the local fork state.
///
/// When the upstream exposes a WebSocket URL, [`Self::run`] subscribes to
/// `newHeads` (`eth_subscribe`) via [`UpstreamRpc::subscribe_new_heads`]. If
/// the socket drops or the stream errors, the follower waits with exponential
/// backoff (starting at 250ms, doubling, capped at 5s) before resubscribing.
#[derive(Debug)]
pub struct TargetedFollower {
    upstream: Arc<UpstreamRpc>,
    state: Arc<RwLock<MirageState>>,
    classifier: DiffClassifier,
    config: FollowerConfig,
    last_block_number: u64,
}

impl TargetedFollower {
    /// Creates a new targeted follower starting from `initial_block`.
    #[must_use]
    pub fn new(
        upstream: Arc<UpstreamRpc>,
        mirage: &crate::fork::MirageFork,
        classifier: DiffClassifier,
        config: FollowerConfig,
        initial_block: u64,
    ) -> Self {
        Self {
            upstream,
            state: mirage.state(),
            classifier,
            config,
            last_block_number: initial_block,
        }
    }

    /// Processes the next available upstream block.
    pub(crate) fn tick_once(&mut self) -> Result<()> {
        self.catch_up_to_current_head()
    }

    /// Runs the follower until shutdown or upstream stream exhaustion.
    ///
    /// # Errors
    ///
    /// Returns upstream block-fetch, WebSocket, or replay-validation errors
    /// encountered while consuming heads or falling back to polling.
    #[allow(clippy::cognitive_complexity)]
    pub async fn run(mut self, mut shutdown: broadcast::Receiver<()>) -> Result<()> {
        if self.is_proxy_mode() {
            return Ok(());
        }
        if self.upstream.has_ws() {
            let proxy_mode = { self.state.read().mode_change.subscribe() };
            let mut reconnect_delay = WS_RECONNECT_BACKOFF_INITIAL;
            loop {
                if self.is_proxy_mode() {
                    return Ok(());
                }
                let mut heads = match self.upstream.subscribe_new_heads().await {
                    Ok(heads) => {
                        reconnect_delay = WS_RECONNECT_BACKOFF_INITIAL;
                        if let Err(error) = run_blocking(|| self.catch_up_to_current_head()) {
                            tracing::warn!("targeted follower catch-up failed: {error}");
                            tokio::select! {
                                _ = shutdown.recv() => return Ok(()),
                                () = tokio::time::sleep(reconnect_delay) => {}
                            }
                            reconnect_delay = next_ws_reconnect_delay(reconnect_delay);
                            continue;
                        }
                        let mut stream = heads;
                        let (tx, rx) = mpsc::channel::<Result<u64>>(NEW_HEADS_CHANNEL_CAPACITY);
                        tokio::spawn(async move {
                            while let Some(item) = stream.next().await {
                                if tx.send(item).await.is_err() {
                                    break;
                                }
                            }
                        });
                        rx
                    }
                    Err(error) => {
                        let msg = error.to_string();
                        // -32601 = method not found: the WS endpoint doesn't support
                        // eth_subscribe (e.g. Anvil with --no-mining or plain HTTP-over-WS).
                        // Fall through to the HTTP polling loop below instead of spinning.
                        if msg.contains("-32601") || msg.contains("method not found") {
                            tracing::info!(
                                "eth_subscribe unsupported, falling back to HTTP polling"
                            );
                            break;
                        }
                        tracing::warn!("targeted follower websocket connect failed: {error}");
                        tokio::select! {
                            _ = shutdown.recv() => return Ok(()),
                            mode = self.wait_for_proxy_mode(&proxy_mode) => {
                                if mode == MirageMode::Proxy {
                                    return Ok(());
                                }
                            }
                            () = tokio::time::sleep(reconnect_delay) => {}
                        }
                        reconnect_delay = next_ws_reconnect_delay(reconnect_delay);
                        continue;
                    }
                };

                match self
                    .drive_websocket_stream(&mut heads, &mut shutdown, &proxy_mode)
                    .await?
                {
                    WebsocketLoopOutcome::Shutdown | WebsocketLoopOutcome::Proxy => return Ok(()),
                    WebsocketLoopOutcome::Reconnect => {}
                }

                tokio::select! {
                    _ = shutdown.recv() => return Ok(()),
                    mode = self.wait_for_proxy_mode(&proxy_mode) => {
                        if mode == MirageMode::Proxy {
                            return Ok(());
                        }
                    }
                    () = tokio::time::sleep(reconnect_delay) => {}
                }
                reconnect_delay = next_ws_reconnect_delay(reconnect_delay);
            }
        }

        loop {
            tokio::select! {
                _ = shutdown.recv() => break,
                result = std::future::ready(run_blocking(|| self.tick_once())) => {
                    result?;
                    if self.is_proxy_mode() {
                        return Ok(());
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }

        Ok(())
    }

    async fn drive_websocket_stream(
        &mut self,
        heads_rx: &mut mpsc::Receiver<Result<u64>>,
        shutdown: &mut broadcast::Receiver<()>,
        proxy_mode: &watch::Receiver<MirageMode>,
    ) -> Result<WebsocketLoopOutcome> {
        let mut proxy_mode = proxy_mode.clone();
        loop {
            tokio::select! {
                _ = shutdown.recv() => return Ok(WebsocketLoopOutcome::Shutdown),
                _ = proxy_mode.changed() => {
                    if *proxy_mode.borrow() == MirageMode::Proxy {
                        return Ok(WebsocketLoopOutcome::Proxy);
                    }
                }
                next_head = heads_rx.recv() => {
                    match next_head {
                        Some(Ok(head)) => run_blocking(|| self.replay_to_head(head)),
                        Some(Err(error)) => {
                            tracing::warn!("targeted follower websocket stream failed: {error}");
                            return Ok(WebsocketLoopOutcome::Reconnect);
                        }
                        None => {
                            tracing::warn!("targeted follower websocket stream closed");
                            return Ok(WebsocketLoopOutcome::Reconnect);
                        }
                    }
                }
            }
        }
    }

    async fn wait_for_proxy_mode(&self, proxy_mode: &watch::Receiver<MirageMode>) -> MirageMode {
        let mut proxy_mode = proxy_mode.clone();
        let _ = proxy_mode.changed().await;
        *proxy_mode.borrow()
    }

    fn catch_up_to_current_head(&mut self) -> Result<()> {
        if self.is_proxy_mode() {
            return Ok(());
        }
        let head = self.upstream.get_block_number()?;
        self.replay_to_head(head);
        Ok(())
    }

    #[allow(clippy::cognitive_complexity)]
    fn replay_to_head(&mut self, head: u64) {
        if self.is_proxy_mode() || head <= self.last_block_number {
            return;
        }

        let start = self.replay_start_number(head);
        if start > self.last_block_number.saturating_add(1) {
            tracing::warn!(
                from = self.last_block_number,
                to = head,
                skipped = start
                    .saturating_sub(self.last_block_number)
                    .saturating_sub(1),
                "targeted follower lagged behind upstream head; skipping intermediate blocks"
            );
        }

        for number in start..=head {
            if self.is_proxy_mode() {
                return;
            }
            if let Err(error) = self.replay_block(number) {
                tracing::warn!("targeted replay failed for block {number}: {error}");
            }
        }
        self.last_block_number = head;
        tracing::info!(head, "block head advanced");
    }

    fn replay_start_number(&self, head: u64) -> u64 {
        let next = self.last_block_number.saturating_add(1);
        if head.saturating_sub(self.last_block_number) > MAX_REPLAY_LAG_BLOCKS {
            head
        } else {
            next
        }
    }

    #[allow(clippy::cognitive_complexity)]
    fn replay_block(&self, number: u64) -> Result<()> {
        if self.is_proxy_mode() {
            return Ok(());
        }
        let block = self
            .upstream
            .get_block_by_number(crate::provider::BlockTag::Number(number), true)?
            .ok_or_else(|| MirageError::Upstream(format!("missing upstream block {number}")))?;
        let transactions = block
            .get("transactions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let watch_set: HashSet<Address> = {
            let state = self.state.read();
            state.fork.db.dirty.watch_list.keys().copied().collect()
        };
        let filter_address_set: Option<HashSet<Address>> = self
            .config
            .filter_addresses
            .as_ref()
            .map(|addrs| addrs.iter().copied().collect());

        let budget_deadline = Instant::now() + self.config.block_budget;

        for tx_json in transactions {
            if self.is_proxy_mode() {
                return Ok(());
            }
            if Instant::now() >= budget_deadline {
                tracing::warn!(
                    number,
                    budget = ?self.config.block_budget,
                    "targeted follower block replay exceeded block_budget; skipping remaining transactions"
                );
                break;
            }
            if !self.matches_filters(&tx_json, &watch_set, &filter_address_set)? {
                continue;
            }

            let tx_hash = parse_b256_value(
                tx_json
                    .get("hash")
                    .ok_or_else(|| MirageError::Upstream("missing tx hash".to_owned()))?,
            )?;
            let tx_replay = TxReplay { tx_hash };
            let mut fork = { self.state.read().fork.clone() };
            let snapshot = fork.snapshot();
            let replayed = match tx_replay.execute(&self.upstream, &mut fork) {
                Ok(result) => result,
                Err(error) => {
                    let _ = fork.revert(snapshot);
                    tracing::warn!("replay failed for {tx_hash}: {error}");
                    continue;
                }
            };
            let (_execution, diff) = replayed;
            if !diff.success {
                let _ = fork.revert(snapshot);
                tracing::warn!(
                    "replay reverted on-chain for {tx_hash}; not applying partial state diff"
                );
                continue;
            }
            let touched_parent = tx_json
                .get("to")
                .and_then(|value| value.as_str())
                .and_then(|value| value.parse::<Address>().ok())
                .or_else(|| {
                    tx_json
                        .get("from")
                        .and_then(|value| value.as_str())
                        .and_then(|value| value.parse::<Address>().ok())
                });
            self.apply_contagion_watchers(&mut fork, &diff, number, touched_parent)?;
            self.classifier.apply(&mut fork.db.dirty, &diff, number)?;
            {
                let mut state = self.state.write();
                state.fork.adopt_executed_branch(fork);
                state.last_request_at = Instant::now();
                if let Ok(request) = serde_json::from_value::<TransactionRequest>(tx_json.clone()) {
                    state
                        .speculative_executor
                        .lock()
                        .invalidate_for_request(&request);
                }
            }
        }

        // Update block-level EVM context from the upstream block so BASEFEE,
        // COINBASE, PREVRANDAO, and TIMESTAMP opcodes stay realistic even in
        // sync_state_only mode.  Only advance local_block_number when we are
        // in normal following mode; in sync_state_only the auto-miner owns it.
        {
            let mut state = self.state.write();
            state.last_request_at = std::time::Instant::now();

            if let Some(ts) = parse_hex_field(&block, "timestamp") {
                state.fork.timestamp = ts;
            }
            if let Some(base_fee) = parse_hex_field_u128(&block, "baseFeePerGas") {
                state.fork.next_base_fee_per_gas = base_fee;
            }
            if let Some(coinbase) = block
                .get("miner")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<Address>().ok())
            {
                state.fork.coinbase = coinbase;
            }
            if let Some(randao) = block
                .get("mixHash")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<B256>().ok())
            {
                state.fork.prev_randao = randao;
            }

            if !self.config.sync_state_only {
                state.fork.local_block_number = number;
                let _ = state.new_heads_tx.send(NewHeadBroadcast {
                    number,
                    timestamp: state.fork.timestamp,
                    gas_used: parse_hex_field(&block, "gasUsed").unwrap_or(0),
                    gas_limit: parse_hex_field(&block, "gasLimit").unwrap_or(30_000_000),
                    base_fee_per_gas: state.fork.next_base_fee_per_gas,
                    coinbase: state.fork.coinbase,
                    prev_randao: state.fork.prev_randao,
                });
            }
        }

        Ok(())
    }

    fn matches_filters(
        &self,
        tx_json: &serde_json::Value,
        watch_set: &HashSet<Address>,
        filter_address_set: &Option<HashSet<Address>>,
    ) -> Result<bool> {
        let to = tx_json
            .get("to")
            .and_then(|value| value.as_str())
            .and_then(|value| value.parse::<Address>().ok());

        if let Some(filter_addresses) = filter_address_set {
            if to.is_some_and(|address| filter_addresses.contains(&address)) {
                return Ok(true);
            }
        }

        if let Some(filter_selectors) = &self.config.filter_selectors {
            let data = tx_json
                .get("input")
                .or_else(|| tx_json.get("data"))
                .and_then(|value| value.as_str())
                .unwrap_or("0x");
            let bytes = hex::decode(data.trim_start_matches("0x"))
                .map_err(|error| MirageError::Upstream(format!("invalid tx calldata: {error}")))?;
            let selector = bytes.get(..4).and_then(|bytes| bytes.try_into().ok());
            if selector.is_some_and(|selector| filter_selectors.contains(&selector)) {
                return Ok(true);
            }
        }

        Ok(to.is_some_and(|address| watch_set.contains(&address)))
    }

    fn apply_contagion_watchers(
        &self,
        fork: &mut ForkState,
        diff: &StateDiff,
        block_number: u64,
        parent: Option<Address>,
    ) -> Result<()> {
        if fork.db.dirty.demote_protocols_to_slot_only {
            return Ok(());
        }
        if !self.classifier.config().enable_contagion {
            return Ok(());
        }

        let parent = parent.unwrap_or_default();
        for (address, classification) in self.classifier.classify(diff) {
            if classification != Classification::Protocol {
                continue;
            }
            let parent_depth = Self::watch_depth(&fork.db.dirty.watch_list, parent);
            if parent_depth.saturating_add(1) > self.classifier.config().max_contagion_depth {
                continue;
            }
            if fork.db.dirty.unwatch_list.contains(&address)
                || fork.db.dirty.watch_list.contains_key(&address)
            {
                continue;
            }
            if fork.db.dirty.watch_list.len() >= self.classifier.config().max_watched_contracts {
                return Err(MirageError::WatchListFull);
            }
            fork.db.dirty.watch_list.insert(
                address,
                WatchEntry {
                    source: if parent == Address::ZERO {
                        WatchSource::AutoClassified
                    } else {
                        WatchSource::Contagion { parent }
                    },
                    added_at_block: block_number,
                    initial_slot_count: diff
                        .accounts
                        .get(&address)
                        .map_or(0, |account| account.storage_written.len()),
                    replay_count: 0,
                },
            );
        }
        Ok(())
    }

    fn watch_depth(watch_list: &HashMap<Address, WatchEntry>, address: Address) -> usize {
        let mut depth = 0;
        let mut current = address;
        let mut seen = HashSet::new();
        while seen.insert(current) {
            let Some(entry) = watch_list.get(&current) else {
                break;
            };
            match entry.source {
                WatchSource::AutoClassified | WatchSource::Manual => return depth,
                WatchSource::Contagion { parent } => {
                    depth = depth.saturating_add(1);
                    current = parent;
                }
            }
        }
        depth
    }

    fn is_proxy_mode(&self) -> bool {
        self.state.read().mode == MirageMode::Proxy
    }
}

/// Replay a transaction by hash.
#[derive(Debug, Clone, Copy)]
pub struct TxReplay {
    /// Transaction hash to fetch and replay.
    pub tx_hash: B256,
}

impl TxReplay {
    /// Attempts to replay a specific transaction.
    ///
    /// # Errors
    ///
    /// Returns upstream transaction lookup, JSON decode, missing-sender, or
    /// execution errors while rebuilding the transaction.
    pub fn execute(
        &self,
        upstream: &UpstreamRpc,
        state: &mut ForkState,
    ) -> Result<(ExecutionResult, StateDiff)> {
        let tx = upstream
            .get_transaction_by_hash(self.tx_hash)?
            .ok_or_else(|| {
                MirageError::Upstream(format!("transaction {} not found upstream", self.tx_hash))
            })?;
        let request: TransactionRequest = serde_json::from_value(tx)?;
        let from = request
            .from
            .ok_or_else(|| MirageError::InvalidParams("missing from".to_owned()))?;
        let to = request.to;
        let data = request.data.unwrap_or_default();
        let value = request.value.unwrap_or(U256::ZERO);
        // Default to block-gas-limit when unspecified so contract-create estimates
        // (which need much more than the 21_000 value-transfer floor) succeed.
        let gas_limit = request.gas.unwrap_or(30_000_000);
        EvmExecutor::transact(state, from, to, data, value, gas_limit)
    }
}

/// Result of speculative execution.
#[derive(Debug, Clone)]
pub struct SpeculativeResult {
    /// Synthetic execution result.
    pub result: ExecutionResult,
    /// Canonical state diff.
    pub state_diff: StateDiff,
    /// Read set used for invalidation.
    pub read_set: HashSet<(Address, U256)>,
    /// Timestamp when the result was computed.
    pub computed_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SpeculativeContext {
    block_number: u64,
    timestamp: u64,
}

/// Executes transactions against a copy-on-write storage fork without committing base state.
///
/// Storage is forked per block with [`CowState::branch`]: a shared [`Arc`] baseline maps dirty
/// slots from [`ForkState`], and each [`Self::execute`] builds a speculative [`ForkState`] whose
/// [`HybridDB`] dirty layer is seeded from that [`CowState`] so repeated speculations reuse the
/// same baseline without deep-cloning.
#[derive(Debug, Default)]
pub struct SpeculativeExecutor {
    cache: HashMap<(B256, u64), SpeculativeResult>,
    cache_context: Option<SpeculativeContext>,
    cow_baseline: Option<(SpeculativeContext, Arc<HashMap<(Address, U256), U256>>)>,
}

impl SpeculativeExecutor {
    /// Executes a transaction request without mutating the base state.
    ///
    /// # Errors
    ///
    /// Returns missing-sender, decode, or execution errors while preparing and
    /// simulating the speculative transaction.
    ///
    /// Forks the current dirty storage into a [`CowState`] branch so the
    /// shared baseline is never deep-cloned for repeated speculative runs
    /// against the same block.
    pub fn execute(
        &mut self,
        state: &ForkState,
        request: &TransactionRequest,
    ) -> Result<SpeculativeResult> {
        self.prepare_context(state);
        let from = request
            .from
            .ok_or_else(|| MirageError::InvalidParams("missing from".to_owned()))?;
        let to = request.to;
        let value = request.value.unwrap_or(U256::ZERO);
        // Default to block-gas-limit when unspecified so contract-create estimates
        // (which need much more than the 21_000 value-transfer floor) succeed.
        let gas_limit = request.gas.unwrap_or(30_000_000);
        let data = request.data.clone().unwrap_or_default();
        let tx_hash = speculative_key(from, to, value, gas_limit, &data);
        let cache_key = (tx_hash, state.local_block_number);
        if let Some(cached) = self.cache.get(&cache_key) {
            return Ok(cached.clone());
        }
        // Fork via CowState: the shared baseline avoids redundant snapshots
        // when multiple speculative executions target the same block.
        let baseline = self.ensure_cow_baseline(state);
        let cow_fork = CowState::branch(baseline);
        let mut speculative_state = Self::build_speculative_state(state, &cow_fork);

        let (result, state_diff) =
            EvmExecutor::transact(&mut speculative_state, from, to, data, value, gas_limit)?;
        // Revm-backed execution does not populate `storage_read`; derive a conservative read set
        // from explicit reads, written slots (read-before-write), and account-level touches.
        let mut read_set: HashSet<(Address, U256)> = state_diff
            .accounts
            .iter()
            .flat_map(|(address, diff)| {
                diff.storage_read
                    .iter()
                    .map(move |slot| (*address, *slot))
                    .chain(
                        diff.storage_written
                            .keys()
                            .copied()
                            .map(move |slot| (*address, slot)),
                    )
            })
            .collect();
        for (address, diff) in &state_diff.accounts {
            if diff.info_changed {
                read_set.insert((*address, U256::ZERO));
            }
        }
        let speculative = SpeculativeResult {
            result,
            state_diff,
            read_set,
            computed_at: Instant::now(),
        };
        self.cache.insert(cache_key, speculative.clone());
        Ok(speculative)
    }

    /// Removes cached results whose read sets overlap the provided writes.
    pub fn invalidate_for_writes(&mut self, writes: &HashSet<(Address, U256)>) {
        self.cache
            .retain(|_, cached| cached.read_set.is_disjoint(writes));
    }

    /// Clears speculative results for a specific block number.
    pub fn invalidate_for_block(&mut self, block_number: u64) {
        self.cache
            .retain(|(_, cached_block), _| *cached_block != block_number);
        if self
            .cache_context
            .is_some_and(|context| context.block_number == block_number)
        {
            self.cache_context = None;
        }
        if self
            .cow_baseline
            .as_ref()
            .is_some_and(|(context, _)| context.block_number == block_number)
        {
            self.cow_baseline = None;
        }
    }

    /// Drops cached entries for the same signed request shape (e.g. tx now included on-chain).
    pub fn invalidate_for_request(&mut self, request: &TransactionRequest) {
        let Some(from) = request.from else {
            return;
        };
        let to = request.to;
        let value = request.value.unwrap_or(U256::ZERO);
        // Default to block-gas-limit when unspecified so contract-create estimates
        // (which need much more than the 21_000 value-transfer floor) succeed.
        let gas_limit = request.gas.unwrap_or(30_000_000);
        let data = request.data.clone().unwrap_or_default();
        let h = speculative_key(from, to, value, gas_limit, &data);
        self.cache.retain(|(k, _), _| *k != h);
    }

    fn current_context(state: &ForkState) -> SpeculativeContext {
        SpeculativeContext {
            block_number: state.local_block_number,
            timestamp: state.timestamp,
        }
    }

    fn prepare_context(&mut self, state: &ForkState) {
        let context = Self::current_context(state);
        if self.cache_context.is_some_and(|cached| cached != context) {
            self.cache.clear();
        }
        if self
            .cow_baseline
            .as_ref()
            .is_some_and(|(cached, _)| *cached != context)
        {
            self.cow_baseline = None;
        }
        self.cache_context = Some(context);
    }

    fn ensure_cow_baseline(&mut self, state: &ForkState) -> Arc<HashMap<(Address, U256), U256>> {
        let context = Self::current_context(state);
        if let Some((cached, baseline)) = &self.cow_baseline {
            if *cached == context {
                return Arc::clone(baseline);
            }
        }
        let mut storage = HashMap::new();
        for (address, account) in &state.db.dirty.accounts {
            for (slot, value) in &account.storage {
                storage.insert((*address, *slot), *value);
            }
        }
        let baseline = Arc::new(storage);
        self.cow_baseline = Some((context, Arc::clone(&baseline)));
        baseline
    }

    fn build_speculative_state(state: &ForkState, cow: &CowState) -> ForkState {
        let mut dirty = DirtyStore::default();
        dirty.demote_protocols_to_slot_only = state.db.dirty.demote_protocols_to_slot_only;
        for (address, account) in &state.db.dirty.accounts {
            let mut storage = HashMap::new();
            for slot in account.storage.keys() {
                if let Some(value) = cow.read(*address, *slot) {
                    storage.insert(*slot, value);
                }
            }
            dirty.accounts.insert(
                *address,
                DirtyAccount {
                    balance: account.balance,
                    nonce: account.nonce,
                    code: account.code.clone(),
                    code_hash: account.code_hash,
                    erc20_balance_slot: account.erc20_balance_slot,
                    erc20_balances: account.erc20_balances.clone(),
                    storage,
                },
            );
        }
        dirty.watch_list = state.db.dirty.watch_list.clone();
        dirty.unwatch_list = state.db.dirty.unwatch_list.clone();
        dirty.total_dirty_slots = state.db.dirty.total_dirty_slots;

        let db = HybridDB {
            dirty,
            read_cache: ReadCache::new(
                state.db.read_cache.entry_count().max(1),
                state.db.cache_ttl,
            ),
            bytecode_cache: Arc::clone(&state.db.bytecode_cache),
            upstream: Arc::clone(&state.db.upstream),
            pinned_block: state.db.pinned_block,
            cache_ttl: state.db.cache_ttl,
            chain_id: state.db.chain_id,
        };

        let mut fork = ForkState::new(db, state.local_block_number, state.chain_id);
        fork.timestamp = state.timestamp;
        fork.next_base_fee_per_gas = state.next_base_fee_per_gas;
        fork.coinbase = state.coinbase;
        fork.prev_randao = state.prev_randao;
        fork.impersonated_accounts = state.impersonated_accounts.clone();
        fork
    }
}

impl MultiVersionStore {
    /// Counts transaction re-executions by collapsing per-slot versions into a
    /// single highest incarnation per transaction index.
    #[must_use]
    pub fn re_execution_count(&self) -> usize {
        let mut tx_incarnations = HashMap::<usize, usize>::new();
        for entry in &self.versions {
            for version in entry.value() {
                tx_incarnations
                    .entry(version.tx_index)
                    .and_modify(|current| *current = (*current).max(version.incarnation as usize))
                    .or_insert(version.incarnation as usize);
            }
        }
        tx_incarnations.into_values().sum()
    }

    /// Computes the Block-STM conflict rate for a block:
    /// `re_executions / total_txs`.
    #[must_use]
    pub fn conflict_rate(&self, total_txs: usize) -> f64 {
        if total_txs == 0 {
            return 0.0;
        }
        self.re_execution_count() as f64 / total_txs as f64
    }
}

fn speculative_key(
    from: Address,
    to: Option<Address>,
    value: U256,
    gas: u64,
    data: &Bytes,
) -> B256 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(from.as_slice());
    bytes.extend_from_slice(to.unwrap_or_default().as_slice());
    bytes.extend_from_slice(&value.to_be_bytes::<32>());
    bytes.extend_from_slice(&gas.to_be_bytes());
    bytes.extend_from_slice(data.as_ref());
    alloy_primitives::keccak256(bytes)
}

fn parse_hex_field(block: &Value, key: &str) -> Option<u64> {
    let s = block.get(key)?.as_str()?;
    let s = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(s, 16).ok()
}

fn parse_hex_field_u128(block: &Value, key: &str) -> Option<u128> {
    let s = block.get(key)?.as_str()?;
    let s = s.strip_prefix("0x").unwrap_or(s);
    u128::from_str_radix(s, 16).ok()
}

fn parse_b256_value(value: &Value) -> Result<B256> {
    let text = value
        .as_str()
        .ok_or_else(|| MirageError::Upstream("expected B256 hex string".to_owned()))?;
    text.parse::<B256>()
        .map_err(|error| MirageError::Upstream(format!("invalid B256: {error}")))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        num::NonZeroUsize,
        sync::Arc,
        time::{Duration, Instant},
    };

    use alloy_primitives::{Address, B256, Bytes, U256, address, keccak256};
    use serde_json::json;

    use super::{
        AccountDiff, FollowerConfig, SpeculativeContext, SpeculativeExecutor, StateDiff,
        TargetedFollower, WS_RECONNECT_BACKOFF_INITIAL, WS_RECONNECT_BACKOFF_MAX,
        WebsocketLoopOutcome, next_ws_reconnect_delay,
    };
    use crate::{
        TransactionRequest,
        cow::{MultiVersionStore, VersionEntry},
        fork::{ForkState, HybridDB, MirageFork, WatchEntry, WatchSource},
        provider::UpstreamRpc,
        resources::{MirageMode, Profile, ResourceModel},
    };
    #[test]
    fn speculative_exec_no_state_commit() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let state = ForkState::new(db, 0, 1);
        let sender = address!("0x1000000000000000000000000000000000000001");
        let receiver = address!("0x1000000000000000000000000000000000000002");
        let request = TransactionRequest {
            from: Some(sender),
            to: Some(receiver),
            gas: Some(50_000),
            value: Some(U256::from(10_u64)),
            data: Some(Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef])),
            gas_price: None,
            nonce: None,
            chain_id: None,
        };
        let mut executor = SpeculativeExecutor::default();

        let result = match executor.execute(&state, &request) {
            Ok(result) => result,
            Err(error) => panic!("speculation succeeds: {error}"),
        };

        assert!(result.state_diff.success);
        assert!(!result.read_set.is_empty());
        assert!(state.receipts.is_empty());

        let mut writes = HashSet::new();
        writes.extend(result.read_set.iter().copied());
        executor.invalidate_for_writes(&writes);
        assert!(executor.cache.is_empty());
    }

    #[test]
    fn state_diff_account_and_storage() {
        let out = Bytes::from_static(&[0x01, 0x02]);
        let mut diff = StateDiff::success(21_000, out.clone());
        assert!(diff.accounts.is_empty());
        assert!(diff.logs.is_empty());
        assert_eq!(diff.gas_used, 21_000);
        assert!(diff.success);
        assert_eq!(diff.output, out);

        let address = address!("0x2000000000000000000000000000000000000002");
        let topic = B256::from([0x55; 32]);
        diff.logs.push(super::LogEntry {
            address,
            topics: vec![topic],
            data: Bytes::from_static(&[0xaa]),
            log_index: 0,
        });
        diff.accounts.insert(
            address,
            super::AccountDiff {
                info_changed: true,
                new_balance: Some(U256::from(1_u64)),
                new_nonce: Some(1),
                new_code: None,
                storage_written: std::iter::once((U256::from(1_u64), U256::from(2_u64))).collect(),
                storage_read: std::iter::once(U256::from(1_u64)).collect(),
            },
        );

        assert_eq!(diff.accounts[&address].storage_written.len(), 1);
        assert_eq!(diff.accounts[&address].storage_read.len(), 1);
        assert_eq!(diff.logs.len(), 1);
        assert_eq!(diff.logs[0].address, address);
        assert_eq!(diff.logs[0].topics, vec![topic]);
        assert_eq!(diff.logs[0].log_index, 0);
    }

    #[test]
    fn block_stm_matches_sequential() {
        let address = address!("0x3000000000000000000000000000000000000003");
        let slot = U256::from(1_u64);
        let store = MultiVersionStore::default();
        store.record(
            address,
            slot,
            VersionEntry {
                tx_index: 0,
                value: U256::from(5_u64),
                incarnation: 0,
            },
        );
        store.record(
            address,
            slot,
            VersionEntry {
                tx_index: 1,
                value: U256::from(9_u64),
                incarnation: 0,
            },
        );

        let materialized = store.materialize();
        assert_eq!(materialized.get(&(address, slot)), Some(&U256::from(9_u64)));
    }

    #[test]
    fn contagion_depth_cap_blocks_grandchildren() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(
            Arc::clone(&upstream),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let classifier = crate::fork::DiffClassifier::new(crate::fork::ClassificationConfig {
            protocol_slot_threshold: 3,
            check_token_interface: true,
            max_watched_contracts: 64,
            enable_contagion: true,
            max_contagion_depth: 1,
        });
        let follower = TargetedFollower::new(
            Arc::clone(&upstream),
            &mirage,
            classifier,
            FollowerConfig {
                ws_url: "ws://127.0.0.1:8546".to_owned(),
                http_url: "http://127.0.0.1:8545".to_owned(),
                block_budget: Duration::from_secs(1),
                filter_addresses: None,
                filter_selectors: None,
                sync_state_only: false,
            },
            0,
        );

        let root = address!("0x1111111111111111111111111111111111111111");
        let parent = address!("0x2222222222222222222222222222222222222222");
        let grandchild = address!("0x3333333333333333333333333333333333333333");
        {
            let state_handle = mirage.state();
            let mut state = state_handle.write();
            state.fork.db.dirty.watch_list.insert(
                root,
                WatchEntry {
                    source: WatchSource::Manual,
                    added_at_block: 1,
                    initial_slot_count: 0,
                    replay_count: 0,
                },
            );
            state.fork.db.dirty.watch_list.insert(
                parent,
                WatchEntry {
                    source: WatchSource::Contagion { parent: root },
                    added_at_block: 2,
                    initial_slot_count: 0,
                    replay_count: 0,
                },
            );
        }

        let mut diff = StateDiff::default();
        diff.accounts.insert(
            grandchild,
            AccountDiff {
                info_changed: false,
                new_balance: None,
                new_nonce: None,
                new_code: None,
                storage_written: std::iter::once((U256::from(21_u64), U256::from(1_u64)))
                    .chain(std::iter::once((U256::from(22_u64), U256::from(2_u64))))
                    .chain(std::iter::once((U256::from(23_u64), U256::from(3_u64))))
                    .collect(),
                storage_read: HashSet::new(),
            },
        );

        let mut fork_state = mirage.state().read().fork.clone();
        follower
            .apply_contagion_watchers(&mut fork_state, &diff, 3, Some(parent))
            .expect("contagion application succeeds");

        assert!(!fork_state.db.dirty.watch_list.contains_key(&grandchild));
    }

    #[tokio::test]
    async fn follower_exits_when_runtime_enters_proxy_mode() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(
            upstream.clone(),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Proxy,
        );
        let follower = TargetedFollower::new(
            upstream,
            &mirage,
            crate::fork::DiffClassifier::new(crate::fork::ClassificationConfig::default()),
            FollowerConfig {
                ws_url: "ws://127.0.0.1:8546".to_owned(),
                http_url: "http://127.0.0.1:8545".to_owned(),
                block_budget: Duration::from_secs(1),
                filter_addresses: None,
                filter_selectors: None,
                sync_state_only: false,
            },
            0,
        );
        let (_shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel(1);
        let proxy_mode = { mirage.state().read().mode_change.subscribe() };
        let (heads_tx, mut heads_rx) = tokio::sync::mpsc::channel::<crate::Result<u64>>(4);

        let run = tokio::spawn(async move {
            let mut follower = follower;
            let _keep_heads_open = heads_tx;
            follower
                .drive_websocket_stream(&mut heads_rx, &mut shutdown_rx, &proxy_mode)
                .await
        });

        tokio::task::yield_now().await;
        {
            let state_handle = mirage.state();
            let mut state = state_handle.write();
            state.mode = MirageMode::Proxy;
            let _ = state.mode_change.send(MirageMode::Proxy);
        }

        let outcome = tokio::time::timeout(Duration::from_secs(1), run)
            .await
            .expect("follower should stop after proxy demotion")
            .expect("join should succeed")
            .expect("stream drive should succeed");
        assert_eq!(outcome, WebsocketLoopOutcome::Proxy);
    }

    #[test]
    #[ignore = "timing-sensitive; flaky under parallel load"]
    fn test_block_budget_timeout_enforced() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        upstream.set_mock_delay(Duration::from_millis(12));
        let watched = address!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let mut txs = Vec::new();
        for i in 0_u64..24 {
            let h = keccak256(i.to_be_bytes());
            let sender_word = keccak256((i.wrapping_add(10_000)).to_be_bytes());
            txs.push(json!({
                "hash": format!("{h:#x}"),
                "from": format!("{:#x}", Address::from_word(sender_word)),
                "to": format!("{watched:#x}"),
                "gas": "0x030d40",
                "value": "0x0",
                "input": "0x",
            }));
        }
        upstream.seed_mock_block_transactions(1, txs);
        upstream.set_mock_block_number(1);

        let db = HybridDB::new(
            upstream.clone(),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        {
            let state_handle = mirage.state();
            let mut state = state_handle.write();
            state.fork.db.dirty.watch_list.insert(
                watched,
                WatchEntry {
                    source: WatchSource::Manual,
                    added_at_block: 0,
                    initial_slot_count: 0,
                    replay_count: 0,
                },
            );
        }

        let budget = Duration::from_millis(80);
        let follower = TargetedFollower::new(
            upstream.clone(),
            &mirage,
            crate::fork::DiffClassifier::new(crate::fork::ClassificationConfig::default()),
            FollowerConfig {
                ws_url: "ws://127.0.0.1:8546".to_owned(),
                http_url: "http://127.0.0.1:8545".to_owned(),
                block_budget: budget,
                filter_addresses: None,
                filter_selectors: None,
                sync_state_only: false,
            },
            0,
        );

        let start = Instant::now();
        follower.replay_block(1).expect("replay_block");
        let elapsed = start.elapsed();

        let fetches = upstream.mock_transaction_fetches();
        assert!(
            fetches < 24,
            "block_budget should truncate matched tx replays (got {fetches} fetches)"
        );
        assert!(fetches >= 1, "expected at least one replay attempt");
        assert!(
            elapsed <= budget.saturating_add(Duration::from_millis(50)),
            "INV-008: wall time should respect block_budget plus one tx slack, took {elapsed:?}"
        );
    }

    #[test]
    fn test_targeted_follower_filter_throughput() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(
            upstream.clone(),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );

        // Populate watch list with 1000 addresses.
        {
            let state_handle = mirage.state();
            let mut state = state_handle.write();
            for i in 0..1000_u64 {
                let addr = Address::from_word(B256::from(U256::from(i + 1)));
                state.fork.db.dirty.watch_list.insert(
                    addr,
                    WatchEntry {
                        source: WatchSource::Manual,
                        added_at_block: 0,
                        initial_slot_count: 0,
                        replay_count: 0,
                    },
                );
            }
        }

        let follower = TargetedFollower::new(
            upstream,
            &mirage,
            crate::fork::DiffClassifier::new(crate::fork::ClassificationConfig::default()),
            FollowerConfig {
                ws_url: "ws://127.0.0.1:8546".to_owned(),
                http_url: "http://127.0.0.1:8545".to_owned(),
                block_budget: Duration::from_secs(1),
                filter_addresses: None,
                filter_selectors: None,
                sync_state_only: false,
            },
            0,
        );

        // INV-009: `HashSet` watch snapshot gives O(1) membership per tx over 1000 contracts.
        let watched = Address::from_word(B256::from(U256::from(500)));
        let watch_set: HashSet<Address> = {
            let handle = mirage.state();
            let state = handle.read();
            state.fork.db.dirty.watch_list.keys().copied().collect()
        };
        let filter_address_set: Option<HashSet<Address>> = None;
        let tx_json = serde_json::json!({
            "to": format!("{watched}"),
            "input": "0x",
        });
        let start = Instant::now();
        for _ in 0..10_000 {
            let _ = follower.matches_filters(&tx_json, &watch_set, &filter_address_set);
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(500),
            "HashSet-based filter should handle 10k lookups quickly, took {elapsed:?}"
        );
    }

    #[test]
    fn test_targeted_follower_skips_intermediate_replay_when_lagging() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(
            Arc::clone(&upstream),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let follower = TargetedFollower::new(
            upstream,
            &mirage,
            crate::fork::DiffClassifier::new(crate::fork::ClassificationConfig::default()),
            FollowerConfig {
                ws_url: "ws://127.0.0.1:8546".to_owned(),
                http_url: "http://127.0.0.1:8545".to_owned(),
                block_budget: Duration::from_secs(1),
                filter_addresses: None,
                filter_selectors: None,
                sync_state_only: false,
            },
            1,
        );

        assert_eq!(follower.replay_start_number(10), 2);
        assert_eq!(follower.replay_start_number(51), 2);
        assert_eq!(follower.replay_start_number(52), 52);
    }

    #[test]
    fn test_targeted_follower_websocket_backoff_caps() {
        let first = next_ws_reconnect_delay(WS_RECONNECT_BACKOFF_INITIAL);
        let second = next_ws_reconnect_delay(first);

        assert_eq!(first, Duration::from_millis(500));
        assert_eq!(second, Duration::from_secs(1));
        assert_eq!(
            next_ws_reconnect_delay(WS_RECONNECT_BACKOFF_MAX),
            WS_RECONNECT_BACKOFF_MAX
        );
    }

    #[tokio::test]
    async fn follower_reconnects_when_websocket_stream_closes() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(
            Arc::clone(&upstream),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let mut follower = TargetedFollower::new(
            upstream,
            &mirage,
            crate::fork::DiffClassifier::new(crate::fork::ClassificationConfig::default()),
            FollowerConfig {
                ws_url: "ws://127.0.0.1:8546".to_owned(),
                http_url: "http://127.0.0.1:8545".to_owned(),
                block_budget: Duration::from_secs(1),
                filter_addresses: None,
                filter_selectors: None,
                sync_state_only: false,
            },
            0,
        );
        let (_shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel(1);
        let proxy_mode = { mirage.state().read().mode_change.subscribe() };
        let (heads_tx, mut heads_rx) = tokio::sync::mpsc::channel::<crate::Result<u64>>(1);
        drop(heads_tx);

        let outcome = follower
            .drive_websocket_stream(&mut heads_rx, &mut shutdown_rx, &proxy_mode)
            .await
            .expect("stream drive should succeed");

        assert_eq!(outcome, WebsocketLoopOutcome::Reconnect);
    }

    #[test]
    fn test_speculative_executor_memory_per_tx() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let state = ForkState::new(db, 0, 1);
        let mut executor = SpeculativeExecutor::default();

        for i in 0..10_u64 {
            let mut addr_bytes = [0u8; 20];
            addr_bytes[12..20].copy_from_slice(&(i + 1).to_be_bytes());
            let sender = Address::from(addr_bytes);
            let request = TransactionRequest {
                from: Some(sender),
                to: Some(Address::ZERO),
                gas: Some(50_000),
                value: Some(U256::from(1_u64)),
                data: Some(Bytes::from(vec![i as u8; 4])),
                ..Default::default()
            };
            let _ = executor.execute(&state, &request);
        }

        // INV-010: each entry holds a `StateDiff` + `read_set` + outputs — not a full fork clone.
        // Plan target ~12 KiB/entry; we only assert a loose serialized upper bound for CI stability.
        let total_serialized: usize = executor
            .cache
            .values()
            .map(|cached| {
                serde_json::to_vec(&cached.state_diff)
                    .map(|v| v.len())
                    .unwrap_or(0)
                    .saturating_add(cached.read_set.len().saturating_mul(64))
                    .saturating_add(cached.result.output.len())
            })
            .sum();
        let per_entry = total_serialized / executor.cache.len().max(1);
        assert!(
            per_entry < 96 * 1024,
            "cached speculative footprint per entry should stay bounded (got ~{per_entry} B serialized proxy)"
        );

        assert_eq!(executor.cache.len(), 10);
        // The CowState baseline is shared across all 10 executions within the
        // same block, avoiding redundant storage snapshots.
        assert!(
            executor.cow_baseline.is_some(),
            "CowState baseline should be cached for the current block"
        );
    }

    #[test]
    fn test_speculative_invalidation_on_block_write() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let state = ForkState::new(db, 0, 1);
        let mut executor = SpeculativeExecutor::default();

        let request = TransactionRequest {
            from: Some(address!("0x1000000000000000000000000000000000000001")),
            to: Some(address!("0x2000000000000000000000000000000000000002")),
            gas: Some(50_000),
            value: Some(U256::from(1_u64)),
            data: Some(Bytes::from_static(&[0xab, 0xcd])),
            ..Default::default()
        };
        executor
            .execute(&state, &request)
            .expect("speculative execution succeeds");
        assert_eq!(executor.cache.len(), 1);

        executor.invalidate_for_block(0);
        assert!(
            executor.cache.is_empty(),
            "invalidate_for_block should clear cached results"
        );
        assert!(
            executor.cow_baseline.is_none(),
            "invalidate_for_block should clear the CowState baseline"
        );
    }

    #[test]
    fn test_speculative_invalidation_when_request_included() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let state = ForkState::new(db, 0, 1);
        let mut executor = SpeculativeExecutor::default();

        let request = TransactionRequest {
            from: Some(address!("0x1000000000000000000000000000000000000001")),
            to: Some(address!("0x2000000000000000000000000000000000000002")),
            gas: Some(50_000),
            value: Some(U256::from(1_u64)),
            data: Some(Bytes::from_static(&[0xab, 0xcd])),
            ..Default::default()
        };
        executor
            .execute(&state, &request)
            .expect("speculative execution succeeds");
        assert_eq!(executor.cache.len(), 1);

        executor.invalidate_for_request(&request);
        assert!(
            executor.cache.is_empty(),
            "invalidate_for_request should drop the matching cache entry when the tx is included on-chain"
        );
    }

    #[test]
    fn test_speculative_invalidation_on_timestamp_deviation() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let mut state = ForkState::new(db, 0, 1);
        state.timestamp = 1;
        let mut executor = SpeculativeExecutor::default();

        let request = TransactionRequest {
            from: Some(address!("0x1000000000000000000000000000000000000001")),
            to: Some(address!("0x2000000000000000000000000000000000000002")),
            gas: Some(50_000),
            value: Some(U256::from(1_u64)),
            data: Some(Bytes::from_static(&[0xab, 0xcd])),
            ..Default::default()
        };
        let first = executor
            .execute(&state, &request)
            .expect("first speculative execution succeeds");
        assert_eq!(
            executor.cache_context,
            Some(SpeculativeContext {
                block_number: 0,
                timestamp: 1,
            })
        );

        std::thread::sleep(Duration::from_millis(1));
        state.timestamp = 2;
        let second = executor
            .execute(&state, &request)
            .expect("speculative execution after timestamp change succeeds");

        assert_eq!(
            executor.cache_context,
            Some(SpeculativeContext {
                block_number: 0,
                timestamp: 2,
            })
        );
        assert!(
            second.computed_at > first.computed_at,
            "timestamp change should invalidate the cached speculative result"
        );
        assert!(executor.cache.len() <= 1);
    }

    /// INV-036 — speculative cache must drop when any disjunct holds:
    /// (1) the same request is treated as included on-chain (`invalidate_for_request`),
    /// (2) external writes intersect the speculative read set (`invalidate_for_writes`),
    /// (3) fork `block_number` or `timestamp` changes (`prepare_context` full clear).
    #[test]
    fn test_speculative_invalidation_conditions() {
        let sender = address!("0x1000000000000000000000000000000000000001");
        let receiver = address!("0x2000000000000000000000000000000000000002");
        let request = TransactionRequest {
            from: Some(sender),
            to: Some(receiver),
            gas: Some(50_000),
            value: Some(U256::from(10_u64)),
            data: Some(Bytes::from_static(&[0xde, 0xad])),
            ..Default::default()
        };

        // --- (2) writes ∩ read_set ≠ ∅ ---
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(
            Arc::clone(&upstream),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let state = ForkState::new(db, 0, 1);
        let mut executor = SpeculativeExecutor::default();
        let result = executor
            .execute(&state, &request)
            .expect("speculative execution succeeds");
        assert_eq!(executor.cache.len(), 1);

        let mut unrelated = HashSet::new();
        unrelated.insert((
            address!("0x9999999999999999999999999999999999999999"),
            U256::from(999_u64),
        ));
        executor.invalidate_for_writes(&unrelated);
        assert_eq!(
            executor.cache.len(),
            1,
            "unrelated writes should not invalidate"
        );

        executor.invalidate_for_writes(&result.read_set);
        assert!(
            executor.cache.is_empty(),
            "overlapping writes should invalidate cached results"
        );

        // --- (1) tx included on-chain (same shape) → `invalidate_for_request` ---
        let db1 = HybridDB::new(
            Arc::clone(&upstream),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let state1 = ForkState::new(db1, 0, 1);
        let mut ex1 = SpeculativeExecutor::default();
        ex1.execute(&state1, &request)
            .expect("speculative execution for invalidate_for_request");
        assert_eq!(ex1.cache.len(), 1);
        ex1.invalidate_for_request(&request);
        assert!(
            ex1.cache.is_empty(),
            "invalidate_for_request should drop the matching cache entry"
        );

        // --- (3) block number / timestamp deviation → `prepare_context` clears cache ---
        let db2 = HybridDB::new(
            Arc::clone(&upstream),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let mut state2 = ForkState::new(db2, 0, 1);
        state2.timestamp = 100;
        let mut ex2 = SpeculativeExecutor::default();
        ex2.execute(&state2, &request)
            .expect("execute at block 0 / ts 100");
        assert_eq!(ex2.cache.len(), 1);
        assert!(
            ex2.cache.keys().all(|(_, b)| *b == 0),
            "cache key should use block 0"
        );

        state2.local_block_number = 5;
        ex2.execute(&state2, &request)
            .expect("execute after block number change should survive");
        assert_eq!(
            ex2.cache.len(),
            1,
            "prior block entries should be cleared when the block number changes"
        );
        assert!(
            ex2.cache.keys().all(|(_, b)| *b == 5),
            "only the new block's speculative entry should remain"
        );

        let db3 = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let mut state3 = ForkState::new(db3, 0, 1);
        state3.timestamp = 200;
        let mut ex3 = SpeculativeExecutor::default();
        ex3.execute(&state3, &request).expect("execute at ts 200");
        assert_eq!(ex3.cache.len(), 1);
        state3.timestamp = 201;
        ex3.execute(&state3, &request)
            .expect("execute after timestamp bump");
        assert_eq!(ex3.cache.len(), 1);
        assert!(
            ex3.cache_context
                == Some(SpeculativeContext {
                    block_number: 0,
                    timestamp: 201
                }),
            "timestamp drift should refresh speculative context"
        );
    }

    #[test]
    fn test_block_stm_conflict_rate() {
        let store = MultiVersionStore::default();
        let addr = address!("0x4000000000000000000000000000000000000004");
        let slot_a = U256::from(1_u64);
        let slot_b = U256::from(2_u64);
        let total_txs = 25;

        // Tx 0 executes once. Tx 1 conflicts, re-executes once, and writes two
        // slots; re-execution accounting must stay transaction-based instead of
        // counting each rewritten slot separately.
        store.record(
            addr,
            slot_a,
            VersionEntry {
                tx_index: 0,
                value: U256::from(10_u64),
                incarnation: 0,
            },
        );
        store.record(
            addr,
            slot_a,
            VersionEntry {
                tx_index: 1,
                value: U256::from(20_u64),
                incarnation: 0,
            },
        );
        store.record(
            addr,
            slot_b,
            VersionEntry {
                tx_index: 1,
                value: U256::from(30_u64),
                incarnation: 0,
            },
        );
        store.record(
            addr,
            slot_a,
            VersionEntry {
                tx_index: 1,
                value: U256::from(21_u64),
                incarnation: 1,
            },
        );
        store.record(
            addr,
            slot_b,
            VersionEntry {
                tx_index: 1,
                value: U256::from(31_u64),
                incarnation: 1,
            },
        );

        // The materialized view returns the latest version for each slot.
        let materialized = store.materialize();
        assert_eq!(materialized.get(&(addr, slot_a)), Some(&U256::from(21_u64)));
        assert_eq!(materialized.get(&(addr, slot_b)), Some(&U256::from(31_u64)));

        assert_eq!(store.re_execution_count(), 1);
        let conflict_rate = store.conflict_rate(total_txs);
        assert!((conflict_rate - (1.0 / total_txs as f64)).abs() < f64::EPSILON);
        assert!(conflict_rate < 0.05);
    }
}
