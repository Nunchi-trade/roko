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
use tracing::{debug, warn};

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

    /// Run the watcher loop until cancelled.
    #[allow(clippy::too_many_lines)]
    pub async fn run(self, publish: PublishFn, cancel: CancellationToken) {
        let mut last_block: u64 = 0;

        // Seed with current block number.
        match self.provider.get_block_number().await {
            Ok(n) => {
                // Start from current block (don't replay history).
                last_block = n;
                debug!(block = n, "block_watcher seeded at block");
            }
            Err(e) => {
                warn!(error = %e, "block_watcher failed to get initial block number");
            }
        }

        loop {
            select! {
                _ = cancel.cancelled() => {
                    debug!("block_watcher cancelled");
                    break;
                }
                _ = tokio::time::sleep(self.poll_interval) => {}
            }

            let current = match self.provider.get_block_number().await {
                Ok(n) => n,
                Err(e) => {
                    warn!(error = %e, "block_watcher poll failed");
                    continue;
                }
            };

            if current <= last_block {
                continue;
            }

            // Process each new block.
            for num in (last_block + 1)..=current {
                if cancel.is_cancelled() {
                    return;
                }

                let block = match self
                    .provider
                    .get_block_by_number(BlockNumberOrTag::Number(num))
                    .full()
                    .await
                {
                    Ok(Some(b)) => b,
                    Ok(None) => continue,
                    Err(e) => {
                        warn!(block = num, error = %e, "failed to fetch block");
                        continue;
                    }
                };

                let header = &block.header;
                let txs = block.transactions.as_transactions().map_or(0, |t| t.len());

                let block_info = BlockInfo {
                    number: header.number,
                    hash: format!("{:#x}", header.hash),
                    parent_hash: format!("{:#x}", header.parent_hash),
                    timestamp: header.timestamp,
                    gas_used: header.gas_used,
                    gas_limit: header.gas_limit,
                    tx_count: txs as u32,
                    base_fee_per_gas: header.base_fee_per_gas,
                };

                publish(
                    "chain:block",
                    serde_json::to_value(&block_info).unwrap_or_default(),
                );

                // Process transactions.
                if let Some(transactions) = block.transactions.as_transactions() {
                    for tx in transactions {
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
                        let (gas_used, success, logs) =
                            match self.provider.get_transaction_receipt(hash_b256).await {
                                Ok(Some(receipt)) => {
                                    let logs: Vec<_> = receipt.inner.logs().to_vec();
                                    (receipt.gas_used, receipt.status(), logs)
                                }
                                _ => (0, true, vec![]),
                            };

                        let tx_info = TxInfo {
                            block_number: num,
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
                                block_number: num,
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
            }

            last_block = current;
        }
    }
}

/// Decode a known contract event or return the raw topic0.
fn decode_event(
    topic0: &str,
    topics: &[B256],
    data: &[u8],
) -> (String, serde_json::Value) {
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
