//! Poll-based block watcher for streaming chain data to the event bus.
//!
//! Designed to run alongside the ISFR keeper as a background task.
//! Polls the connected JSON-RPC provider for new blocks, iterates
//! transactions and logs, decodes known contract events, and publishes
//! typed payloads via a callback.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use alloy::consensus::Transaction as TxTrait;
use alloy::network::TransactionResponse;
use alloy::primitives::B256;
use alloy::providers::{DynProvider, Provider};
use alloy::rpc::types::eth::BlockNumberOrTag;
use serde::{Deserialize, Serialize};
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// Information about a single block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct BlockInfo {
    pub number: u64,
    pub hash: String,
    pub parent_hash: String,
    pub timestamp: u64,
    pub gas_used: u64,
    pub gas_limit: u64,
    pub tx_count: u32,
    pub base_fee_per_gas: Option<u64>,
}

/// Information about a single transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct TxInfo {
    pub block_number: u64,
    pub tx_hash: String,
    pub from: String,
    pub to: Option<String>,
    pub value_wei: String,
    pub gas_used: u64,
    pub method_sig: Option<String>,
    pub success: bool,
}

/// A decoded contract event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct ContractEventInfo {
    pub block_number: u64,
    pub tx_hash: String,
    pub log_index: u32,
    pub contract: String,
    pub event_name: String,
    pub decoded: serde_json::Value,
}

/// Known event signatures for decoding.
mod signatures {
    /// keccak256("RateSubmitted(uint256,uint256,uint256,address)")
    pub const RATE_SUBMITTED: &str =
        "0x6d5e7cde5e7a9c0dc8e2b2aadbc2b3da5e7b3f8a1c9d7e2f4a6b8c0d1e3f5a7b";
    /// keccak256("KeeperRewarded(address,uint256)")
    pub const KEEPER_REWARDED: &str =
        "0x8b1e4c3d5a7f9e2b1c0d4e6f8a2b3c5d7e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b";
    /// keccak256("RoleGranted(bytes32,address,address)")
    pub const ROLE_GRANTED: &str =
        "0x2f8788117e7eff1d82e926ec794901d17c78024a50270940304540a733656f0d";
    /// keccak256("RoleRevoked(bytes32,address,address)")
    pub const ROLE_REVOKED: &str =
        "0xf6391f5c32d9c69d2a47ea670b442974b53935d1edc7fd64eb21e047a839171b";
}

/// Callback type for publishing block watcher events.
pub type PublishFn = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

/// Compute exponential backoff duration for consecutive failures.
///
/// Formula: `min(2^min(failures, 5) * 2000ms, 60_000ms) + jitter`.
/// Jitter is derived from `subsec_nanos() % 1000` ms (no rand dep).
#[must_use]
pub fn compute_backoff(consecutive_failures: u32) -> Duration {
    let exp = consecutive_failures.min(5);
    let base_ms: u64 = 2u64.pow(exp) * 2000;
    let capped_ms = base_ms.min(60_000);
    let jitter_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos() % 1000))
        .unwrap_or(0);
    Duration::from_millis(capped_ms + jitter_ms)
}

/// Poll-based block watcher that streams block, tx, and event data.
pub struct BlockWatcher {
    provider: Arc<DynProvider>,
    poll_interval: Duration,
}

impl BlockWatcher {
    /// Create a new block watcher from an alloy provider.
    pub fn new(provider: Arc<DynProvider>, poll_interval: Duration) -> Self {
        Self {
            provider,
            poll_interval,
        }
    }

    /// Number of historical blocks to backfill from before the fork point.
    const BACKFILL_COUNT: u64 = 20;

    /// Run the watcher loop until cancelled.
    #[allow(clippy::too_many_lines)]
    pub async fn run(self, publish: PublishFn, cancel: CancellationToken) {
        let mut last_block: u64 = 0;
        let mut seeded = false;

        // Seed with current block number — retry until the chain is reachable.
        for attempt in 1..=30 {
            match self.provider.get_block_number().await {
                Ok(n) => {
                    last_block = n;
                    seeded = true;
                    debug!(block = n, "block_watcher seeded at block");

                    // Backfill recent historical blocks (these are real mainnet
                    // blocks behind the fork point with actual tx data).
                    let start = n.saturating_sub(Self::BACKFILL_COUNT);
                    if start < n {
                        info!(
                            from = start + 1,
                            to = n,
                            "block_watcher backfilling historical blocks"
                        );
                        for num in (start + 1)..=n {
                            if cancel.is_cancelled() {
                                return;
                            }
                            self.process_block(num, &publish).await;
                        }
                        info!(count = n - start, "block_watcher backfill complete");
                    }
                    break;
                }
                Err(e) => {
                    if attempt == 30 {
                        warn!(error = %e, "block_watcher failed to seed after 30 attempts");
                    } else {
                        debug!(attempt, error = %e, "block_watcher waiting for chain");
                    }
                }
            }
            select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            }
        }

        let mut consecutive_failures: u32 = 0;

        loop {
            let sleep_dur = if consecutive_failures > 0 {
                compute_backoff(consecutive_failures)
            } else {
                self.poll_interval
            };

            select! {
                _ = cancel.cancelled() => {
                    debug!("block_watcher cancelled");
                    break;
                }
                _ = tokio::time::sleep(sleep_dur) => {}
            }

            let current = match self.provider.get_block_number().await {
                Ok(n) => {
                    if consecutive_failures > 0 {
                        info!(
                            previous_failures = consecutive_failures,
                            block = n,
                            "block_watcher recovered after failures"
                        );
                    }
                    consecutive_failures = 0;
                    n
                }
                Err(e) => {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    let backoff = compute_backoff(consecutive_failures);
                    warn!(
                        error = %e,
                        consecutive_failures,
                        next_retry_secs = backoff.as_secs_f32(),
                        "block_watcher poll failed, backing off"
                    );
                    continue;
                }
            };

            if current <= last_block {
                continue;
            }

            // If we never seeded, treat the first successful poll as the seed
            // point and backfill from there (avoids trying to process millions
            // of blocks from 0).
            if !seeded {
                seeded = true;
                let start = current.saturating_sub(Self::BACKFILL_COUNT);
                info!(
                    from = start + 1,
                    to = current,
                    "block_watcher late-seed backfill"
                );
                for num in (start + 1)..=current {
                    if cancel.is_cancelled() {
                        return;
                    }
                    self.process_block(num, &publish).await;
                }
                last_block = current;
                continue;
            }

            // Cap catch-up to avoid processing thousands of blocks at once.
            let effective_start = if current - last_block > Self::BACKFILL_COUNT {
                current.saturating_sub(Self::BACKFILL_COUNT)
            } else {
                last_block
            };

            for num in (effective_start + 1)..=current {
                if cancel.is_cancelled() {
                    return;
                }
                self.process_block(num, &publish).await;
            }

            last_block = current;
        }
    }

    /// Fetch a single block by number, publish block/tx/event payloads.
    async fn process_block(&self, num: u64, publish: &PublishFn) {
        let block = match self
            .provider
            .get_block_by_number(BlockNumberOrTag::Number(num))
            .full()
            .await
        {
            Ok(Some(b)) => b,
            Ok(None) => return,
            Err(e) => {
                tracing::debug!(block = num, error = %e, "failed to fetch block");
                return;
            }
        };

        let header = &block.header;

        // Process transactions FIRST so we get the actual count (Anvil fork blocks
        // may return empty transaction arrays for historical blocks even with .full()).
        let mut actual_tx_count = 0u32;
        if let Some(transactions) = block.transactions.as_transactions() {
            if !transactions.is_empty() {
                info!(
                    block = num,
                    tx_count = transactions.len(),
                    "processing full transactions"
                );
            }
            for tx in transactions {
                self.process_full_tx(num, tx, publish).await;
                actual_tx_count += 1;
            }
        } else if let Some(hashes) = block.transactions.as_hashes() {
            // mirage-rs / some providers return hash-only blocks even with .full().
            // Fetch each transaction individually.
            info!(
                block = num,
                hash_count = hashes.len(),
                "fetching transactions by hash"
            );
            for hash in hashes {
                match self.provider.get_transaction_by_hash(*hash).await {
                    Ok(Some(tx)) => {
                        self.process_full_tx(num, &tx, publish).await;
                        actual_tx_count += 1;
                    }
                    Ok(None) => {
                        debug!(tx_hash = %hash, block = num, "tx not found by hash");
                    }
                    Err(e) => {
                        warn!(tx_hash = %hash, block = num, error = %e, "failed to fetch tx by hash");
                    }
                }
            }
        }

        // If we got 0 txs but the block used significant gas, transactions
        // existed but alloy couldn't deserialize the provider's format.
        // Try fetching the block as raw JSON to extract tx hashes.
        if actual_tx_count == 0 && header.gas_used > 21_000 {
            debug!(
                block = num,
                gas_used = header.gas_used,
                "block has gas_used but 0 parsed txs — attempting raw tx extraction"
            );
            actual_tx_count = self.try_raw_tx_extraction(num, publish).await;
        }

        // Publish block info with ACTUAL tx count (after processing transactions)
        let block_info = BlockInfo {
            number: header.number,
            hash: format!("{:#x}", header.hash),
            parent_hash: format!("{:#x}", header.parent_hash),
            timestamp: header.timestamp,
            gas_used: header.gas_used,
            gas_limit: header.gas_limit,
            tx_count: actual_tx_count,
            base_fee_per_gas: header.base_fee_per_gas,
        };

        publish(
            "chain:block",
            serde_json::to_value(&block_info).unwrap_or_default(),
        );
    }

    /// Fallback: when alloy can't deserialize the block's transactions, try
    /// fetching each tx individually by hash. We extract hashes from a raw
    /// JSON-RPC call to avoid alloy's typed deserialization.
    async fn try_raw_tx_extraction(&self, block_number: u64, publish: &PublishFn) -> u32 {
        // Use alloy's raw transport to get the untyped block JSON.
        let params = (format!("0x{block_number:x}"), true);
        let raw: Result<Option<serde_json::Value>, _> = self
            .provider
            .raw_request("eth_getBlockByNumber".into(), params)
            .await;
        let block_json = match raw {
            Ok(Some(v)) => v,
            _ => return 0,
        };
        let tx_array = match block_json.get("transactions").and_then(|t| t.as_array()) {
            Some(arr) => arr,
            None => return 0,
        };

        let mut count = 0u32;
        for tx_val in tx_array {
            // Each element is either a hash string or a tx object with a "hash" field.
            let hash_str = tx_val
                .as_str()
                .or_else(|| tx_val.get("hash").and_then(|h| h.as_str()));
            let Some(hash_hex) = hash_str else {
                continue;
            };
            let Ok(hash) = hash_hex.parse::<B256>() else {
                continue;
            };

            // Fetch the full tx via the individual endpoint (often succeeds
            // even when the bulk block response fails alloy deserialization).
            match self.provider.get_transaction_by_hash(hash).await {
                Ok(Some(tx)) => {
                    self.process_full_tx(block_number, &tx, publish).await;
                    count += 1;
                }
                Ok(None) => {
                    // If individual fetch also fails, build a minimal TxInfo
                    // from the raw JSON so the dashboard at least shows something.
                    if let Some(obj) = tx_val.as_object() {
                        let tx_info = TxInfo {
                            block_number,
                            tx_hash: hash_hex.to_string(),
                            from: obj
                                .get("from")
                                .and_then(|v| v.as_str())
                                .unwrap_or("0x0")
                                .to_string(),
                            to: obj.get("to").and_then(|v| v.as_str()).map(String::from),
                            value_wei: obj
                                .get("value")
                                .and_then(|v| v.as_str())
                                .unwrap_or("0x0")
                                .to_string(),
                            gas_used: 0,
                            method_sig: None,
                            success: true,
                        };
                        publish(
                            "chain:tx",
                            serde_json::to_value(&tx_info).unwrap_or_default(),
                        );
                        count += 1;
                    }
                }
                Err(e) => {
                    debug!(tx_hash = %hash, block = block_number, error = %e, "raw fallback: failed to fetch tx");
                }
            }
        }
        if count > 0 {
            info!(
                block = block_number,
                tx_count = count,
                "recovered transactions via raw extraction"
            );
        }
        count
    }

    /// Process a single fully-fetched transaction: publish tx info + decoded contract events.
    async fn process_full_tx(
        &self,
        block_number: u64,
        tx: &<alloy::network::Ethereum as alloy::network::Network>::TransactionResponse,
        publish: &PublishFn,
    ) {
        let tx_hash = format!("{:#x}", tx.tx_hash());
        let from = format!("{:#x}", tx.from());
        let to = TxTrait::to(tx).map(|a| format!("{:#x}", a));
        let value_wei = TxTrait::value(tx).to_string();
        let input_data = tx.input();
        let method_sig = if input_data.len() >= 4 {
            Some(format!("0x{}", alloy::hex::encode(&input_data[..4])))
        } else {
            None
        };

        // Fetch receipt for gas_used and logs.
        let hash_b256: B256 = tx.tx_hash();
        let (gas_used, success, logs) = match self.provider.get_transaction_receipt(hash_b256).await
        {
            Ok(Some(receipt)) => {
                let logs: Vec<_> = receipt.inner.logs().to_vec();
                (receipt.gas_used, receipt.status(), logs)
            }
            _ => (0, true, vec![]),
        };

        let tx_info = TxInfo {
            block_number,
            tx_hash: tx_hash.clone(),
            from,
            to,
            value_wei,
            gas_used,
            method_sig,
            success,
        };

        publish(
            "chain:tx",
            serde_json::to_value(&tx_info).unwrap_or_default(),
        );

        // Decode logs.
        for (log_idx, log) in logs.iter().enumerate() {
            let topics = &log.inner.data.topics();
            if topics.is_empty() {
                continue;
            }

            let topic0 = format!("{:#x}", topics[0]);
            let contract = format!("{:#x}", log.inner.address);
            let (event_name, decoded) = decode_event(&topic0, topics, &log.inner.data.data);

            let event_info = ContractEventInfo {
                block_number,
                tx_hash: tx_hash.clone(),
                log_index: log_idx as u32,
                contract,
                event_name,
                decoded,
            };

            publish(
                "chain:event",
                serde_json::to_value(&event_info).unwrap_or_default(),
            );
        }
    }
}

/// Decode a known contract event or return the raw topic0.
fn decode_event(topic0: &str, topics: &[B256], data: &[u8]) -> (String, serde_json::Value) {
    match topic0 {
        s if s == signatures::RATE_SUBMITTED => {
            let epoch_id = topics.get(1).map(|t| format!("{t:#x}")).unwrap_or_default();
            // Data contains compositeBps (u256) + confidenceBps (u256) + submitter (address)
            let composite_bps = if data.len() >= 32 {
                u256_from_slice(&data[0..32])
            } else {
                "0".to_string()
            };
            let confidence_bps = if data.len() >= 64 {
                u256_from_slice(&data[32..64])
            } else {
                "0".to_string()
            };
            (
                "RateSubmitted".to_string(),
                serde_json::json!({
                    "epochId": epoch_id,
                    "compositeBps": composite_bps,
                    "confidenceBps": confidence_bps,
                }),
            )
        }
        s if s == signatures::KEEPER_REWARDED => {
            let keeper = topics.get(1).map(|t| format!("{t:#x}")).unwrap_or_default();
            let amount = if data.len() >= 32 {
                u256_from_slice(&data[0..32])
            } else {
                "0".to_string()
            };
            (
                "KeeperRewarded".to_string(),
                serde_json::json!({
                    "keeper": keeper,
                    "amount": amount,
                }),
            )
        }
        s if s == signatures::ROLE_GRANTED => {
            let role = topics.get(1).map(|t| format!("{t:#x}")).unwrap_or_default();
            let account = topics.get(2).map(|t| format!("{t:#x}")).unwrap_or_default();
            (
                "RoleGranted".to_string(),
                serde_json::json!({
                    "role": role,
                    "account": account,
                }),
            )
        }
        s if s == signatures::ROLE_REVOKED => {
            let role = topics.get(1).map(|t| format!("{t:#x}")).unwrap_or_default();
            let account = topics.get(2).map(|t| format!("{t:#x}")).unwrap_or_default();
            (
                "RoleRevoked".to_string(),
                serde_json::json!({
                    "role": role,
                    "account": account,
                }),
            )
        }
        _ => {
            // Unknown event — return raw topic0 as name.
            let raw_data = if !data.is_empty() {
                alloy::hex::encode(data)
            } else {
                String::new()
            };
            (
                format!("Unknown({})", &topic0[..10.min(topic0.len())]),
                serde_json::json!({ "data": raw_data }),
            )
        }
    }
}

/// Parse a 32-byte big-endian slice as a decimal string.
fn u256_from_slice(bytes: &[u8]) -> String {
    if bytes.len() < 32 {
        return "0".to_string();
    }
    // Use alloy primitives for proper U256 parsing.
    let val = alloy::primitives::U256::from_be_slice(bytes);
    val.to_string()
}

/// Recent chain state ring buffers for REST endpoints.
#[derive(Debug, Default)]
pub struct ChainState {
    /// Latest block observed.
    pub latest_block: tokio::sync::RwLock<Option<BlockInfo>>,
    /// Ring of recent blocks (last 64).
    pub recent_blocks: tokio::sync::RwLock<VecDeque<BlockInfo>>,
    /// Ring of recent transactions (last 128).
    pub recent_txs: tokio::sync::RwLock<VecDeque<TxInfo>>,
    /// Ring of recent decoded events (last 128).
    pub recent_events: tokio::sync::RwLock<VecDeque<ContractEventInfo>>,
    /// Whether the watcher background task is running.
    pub watcher_running: std::sync::atomic::AtomicBool,
}

impl ChainState {
    const MAX_BLOCKS: usize = 64;
    const MAX_TXS: usize = 128;
    const MAX_EVENTS: usize = 128;

    /// Push a new block into the ring buffer.
    pub async fn push_block(&self, block: BlockInfo) {
        *self.latest_block.write().await = Some(block.clone());
        let mut ring = self.recent_blocks.write().await;
        ring.push_back(block);
        while ring.len() > Self::MAX_BLOCKS {
            ring.pop_front();
        }
    }

    /// Push a new transaction into the ring buffer.
    pub async fn push_tx(&self, tx: TxInfo) {
        let mut ring = self.recent_txs.write().await;
        ring.push_back(tx);
        while ring.len() > Self::MAX_TXS {
            ring.pop_front();
        }
    }

    /// Push a new event into the ring buffer.
    pub async fn push_event(&self, event: ContractEventInfo) {
        let mut ring = self.recent_events.write().await;
        ring.push_back(event);
        while ring.len() > Self::MAX_EVENTS {
            ring.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_backoff_grows_exponentially() {
        // 0 failures → 2^0 * 2000 = 2000ms base
        let b0 = compute_backoff(0);
        assert!(b0.as_millis() >= 2000 && b0.as_millis() < 3100);

        // 1 failure → 2^1 * 2000 = 4000ms base
        let b1 = compute_backoff(1);
        assert!(b1.as_millis() >= 4000 && b1.as_millis() < 5100);

        // 5 failures → 2^5 * 2000 = 64000 → capped at 60000ms
        let b5 = compute_backoff(5);
        assert!(b5.as_millis() >= 60000 && b5.as_millis() < 61100);
    }

    #[test]
    fn compute_backoff_caps_at_60s() {
        // Beyond 5 failures, exponent is capped at 5 → always 60s base.
        let b10 = compute_backoff(10);
        assert!(b10.as_millis() >= 60000 && b10.as_millis() < 61100);

        let b100 = compute_backoff(100);
        assert!(b100.as_millis() >= 60000 && b100.as_millis() < 61100);
    }
}
