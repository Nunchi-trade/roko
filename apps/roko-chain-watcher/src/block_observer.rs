//! Real Ethereum block observer for `roko-chain-watcher`.
//!
//! Polls `eth_getBlockByNumber` against a real Ethereum RPC (or a mirage fork
//! which lazily proxies to upstream), analyzes each block's gas usage,
//! base-fee trend, and tx activity, and posts insights/pheromones for real
//! observed patterns.
//!
//! All insights posted from this module are grounded in actual chain data —
//! no templates, no random content.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};
use tracing::{debug, info, warn};

use crate::known_addresses::{ContractCategory, decode_method_selector, lookup};
use crate::rpc_client::MirageRpcClient;

/// Compute a small stable hash of a string for dedup purposes.
fn content_key(s: &str) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

/// Lightweight block header fields we care about.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ObservedBlock {
    pub number: u64,
    pub hash: String,
    pub timestamp: u64,
    pub gas_used: u64,
    pub gas_limit: u64,
    pub base_fee_wei: u128,
    pub tx_count: usize,
    pub miner: String,
    /// Saturation percent (gas_used / gas_limit * 100).
    pub saturation_pct: f32,
}

impl ObservedBlock {
    /// Base fee in gwei (1e9 wei = 1 gwei).
    #[must_use]
    pub fn base_fee_gwei(&self) -> f64 {
        (self.base_fee_wei as f64) / 1_000_000_000.0
    }
}

/// A large ERC-20 / ETH transfer detected in a block.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct LargeTransfer {
    pub tx_hash: String,
    pub from: String,
    pub to: String,
    pub value_eth: f64,
}

/// Block-level observer: polls real blocks, analyzes patterns, posts insights.
pub struct BlockObserver {
    /// Separate client for block queries (may point to mirage or real upstream).
    eth_client: reqwest::Client,
    eth_url: String,
    /// Mirage client for posting insights/pheromones back to the chain layer.
    mirage: MirageRpcClient,
    /// Identity used when posting insights.
    watcher_id: String,
    /// Last block we analyzed.
    last_analyzed: AtomicU64,
    /// Ring of recent observed blocks for trend analysis.
    recent: parking_lot::Mutex<VecDeque<ObservedBlock>>,
    /// Capacity for the trend buffer.
    capacity: usize,
    next_id: AtomicU64,
    /// Whether to fetch full transactions per block.
    fetch_full_txs: bool,
    dry_run: bool,
    /// Recent posted-insight content hashes (skip identical repeats).
    posted_hashes: parking_lot::Mutex<HashSet<u64>>,
    /// Ring buffer of most-recently posted hashes (so we can expire).
    posted_ring: parking_lot::Mutex<VecDeque<u64>>,
}

impl BlockObserver {
    /// Build a new observer pointed at the given ETH RPC.
    pub fn new(
        eth_url: String,
        mirage: MirageRpcClient,
        watcher_id: String,
        fetch_full_txs: bool,
        dry_run: bool,
    ) -> Self {
        Self {
            eth_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            eth_url,
            mirage,
            watcher_id,
            last_analyzed: AtomicU64::new(0),
            recent: parking_lot::Mutex::new(VecDeque::with_capacity(16)),
            capacity: 16,
            next_id: AtomicU64::new(1),
            fetch_full_txs,
            dry_run,
            posted_hashes: parking_lot::Mutex::new(HashSet::new()),
            posted_ring: parking_lot::Mutex::new(VecDeque::with_capacity(256)),
        }
    }

    /// Returns true if this content was posted recently (skip if so).
    fn dedup_check(&self, content: &str) -> bool {
        let h = content_key(content);
        let mut set = self.posted_hashes.lock();
        if set.contains(&h) {
            return true;
        }
        set.insert(h);
        let mut ring = self.posted_ring.lock();
        ring.push_back(h);
        while ring.len() > 200 {
            if let Some(old) = ring.pop_front() {
                set.remove(&old);
            }
        }
        false
    }

    async fn rpc(&self, method: &str, params: JsonValue) -> Result<JsonValue> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let res = self
            .eth_client
            .post(&self.eth_url)
            .json(&req)
            .send()
            .await
            .with_context(|| format!("POST failed for {method}"))?;
        let status = res.status();
        if !status.is_success() {
            anyhow::bail!("{method} returned HTTP {status}");
        }
        let v: JsonValue = res.json().await.context("decode JSON")?;
        if let Some(err) = v.get("error") {
            anyhow::bail!("{method}: {}", err);
        }
        v.get("result")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing result in {method}"))
    }

    /// Fetch the latest block number (hex).
    pub async fn block_number(&self) -> Result<u64> {
        let r = self.rpc("eth_blockNumber", json!([])).await?;
        let s = r.as_str().context("eth_blockNumber not a string")?;
        let s = s.strip_prefix("0x").unwrap_or(s);
        u64::from_str_radix(s, 16).context("invalid hex")
    }

    /// Fetch block N (light — no full txs).
    pub async fn get_block(&self, n: u64) -> Result<Option<ObservedBlock>> {
        let tag = format!("0x{n:x}");
        let full = self.fetch_full_txs;
        let r = self.rpc("eth_getBlockByNumber", json!([tag, full])).await?;
        if r.is_null() {
            return Ok(None);
        }
        let b: RawBlock = serde_json::from_value(r).context("decode block")?;
        let gas_used = parse_hex_u64(&b.gas_used)?;
        let gas_limit = parse_hex_u64(&b.gas_limit)?;
        let base_fee_wei = b
            .base_fee_per_gas
            .as_deref()
            .map(parse_hex_u128)
            .transpose()?
            .unwrap_or(0);
        let timestamp = parse_hex_u64(&b.timestamp)?;
        let number = parse_hex_u64(&b.number)?;
        let tx_count = match &b.transactions {
            JsonValue::Array(arr) => arr.len(),
            _ => 0,
        };
        let saturation = if gas_limit > 0 {
            (gas_used as f32 / gas_limit as f32) * 100.0
        } else {
            0.0
        };
        Ok(Some(ObservedBlock {
            number,
            hash: b.hash.unwrap_or_default(),
            timestamp,
            gas_used,
            gas_limit,
            base_fee_wei,
            tx_count,
            miner: b.miner.unwrap_or_default(),
            saturation_pct: saturation,
        }))
    }

    /// Extract large ETH transfers from a full block (requires `fetch_full_txs=true`).
    #[allow(dead_code)]
    pub async fn large_transfers(&self, n: u64, min_eth: f64) -> Result<Vec<LargeTransfer>> {
        if !self.fetch_full_txs {
            return Ok(Vec::new());
        }
        let tag = format!("0x{n:x}");
        let r = self.rpc("eth_getBlockByNumber", json!([tag, true])).await?;
        if r.is_null() {
            return Ok(Vec::new());
        }
        let txs = r
            .get("transactions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let threshold_wei = (min_eth * 1e18) as u128;
        let mut out = Vec::new();
        for tx in txs {
            let value_hex = tx.get("value").and_then(|v| v.as_str()).unwrap_or("0x0");
            let value_wei = match parse_hex_u128(value_hex) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if value_wei < threshold_wei {
                continue;
            }
            out.push(LargeTransfer {
                tx_hash: tx
                    .get("hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                from: tx
                    .get("from")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                to: tx
                    .get("to")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                value_eth: (value_wei as f64) / 1e18,
            });
            if out.len() >= 5 {
                break;
            }
        }
        Ok(out)
    }

    /// Analyze N consecutive blocks and post insights for anything interesting.
    pub async fn analyze_range(&self, start: u64, end: u64) -> Result<usize> {
        let mut posted = 0usize;
        for n in start..=end {
            let Some(block) = self.get_block(n).await? else {
                continue;
            };
            // Push into trend ring
            {
                let mut g = self.recent.lock();
                g.push_back(block.clone());
                while g.len() > self.capacity {
                    g.pop_front();
                }
            }
            posted += self.analyze_single(&block).await?;
            posted += self.analyze_trend().await?;
            self.last_analyzed.store(n, Ordering::Relaxed);
            debug!(
                block = n,
                gas_used = block.gas_used,
                saturation = format!("{:.1}%", block.saturation_pct),
                base_fee_gwei = format!("{:.2}", block.base_fee_gwei()),
                txs = block.tx_count,
                "observed block"
            );
        }
        Ok(posted)
    }

    /// Single-block analysis: saturation, tx activity, large transfers.
    async fn analyze_single(&self, b: &ObservedBlock) -> Result<usize> {
        let mut posted = 0usize;
        let base_fee_gwei = b.base_fee_gwei();

        // Rule 1: saturation > 95% → threat pheromone
        if b.saturation_pct > 95.0 {
            let content = format!(
                "block #{} saturated: {:.1}% gas used ({} / {}), base fee {:.2} gwei",
                b.number, b.saturation_pct, b.gas_used, b.gas_limit, base_fee_gwei
            );
            self.deposit("threat", &content, (b.saturation_pct - 90.0) / 10.0)
                .await?;
            posted += 1;
        }

        // Rule 2: base fee very low → insight + opportunity
        if base_fee_gwei < 2.0 && base_fee_gwei > 0.0 {
            let content = format!(
                "low congestion: block #{} base fee {:.2} gwei ({} txs) — good window for non-urgent txs",
                b.number, base_fee_gwei, b.tx_count
            );
            self.post_insight("heuristic", &content).await?;
            self.deposit(
                "opportunity",
                &format!(
                    "low gas window: {:.2} gwei at block #{}",
                    base_fee_gwei, b.number
                ),
                0.6,
            )
            .await?;
            posted += 2;
        }

        // Rule 3: base fee very high → threat
        if base_fee_gwei > 100.0 {
            let content = format!(
                "high gas: block #{} base fee {:.2} gwei — congestion spike",
                b.number, base_fee_gwei
            );
            self.deposit("threat", &content, (base_fee_gwei as f32 / 200.0).min(1.0))
                .await?;
            posted += 1;
        }

        // Rule 4: abnormal tx count (> 300 or = 0)
        if b.tx_count > 300 {
            let content = format!(
                "high activity block #{}: {} txs, {:.1}% saturation",
                b.number, b.tx_count, b.saturation_pct
            );
            self.post_insight("insight", &content).await?;
            posted += 1;
        } else if b.tx_count == 0 {
            let content = format!(
                "empty block #{} — propagation delay or validator miss",
                b.number
            );
            self.post_insight("warning", &content).await?;
            posted += 1;
        }

        // Rule 5+: rich tx-level analysis (only if fetching full txs)
        if self.fetch_full_txs {
            posted += self.analyze_transactions(b.number).await.unwrap_or(0);
        }

        Ok(posted)
    }

    /// Deep transaction-level analysis: DEX routing, MEV detection, whale movements,
    /// contract interactions. Generates diverse, interesting insights grounded in
    /// real tx data.
    #[allow(clippy::cognitive_complexity)]
    async fn analyze_transactions(&self, n: u64) -> Result<usize> {
        let tag = format!("0x{n:x}");
        let r = self.rpc("eth_getBlockByNumber", json!([tag, true])).await?;
        if r.is_null() {
            return Ok(0);
        }
        let txs = r
            .get("transactions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if txs.is_empty() {
            return Ok(0);
        }
        let mut posted = 0usize;

        // Aggregate counts by contract and method
        let mut contract_hits: HashMap<&'static str, (ContractCategory, usize)> = HashMap::new();
        let mut method_hits: HashMap<&'static str, usize> = HashMap::new();
        let mut category_hits: HashMap<ContractCategory, usize> = HashMap::new();
        let mut total_value_eth: f64 = 0.0;
        let mut large_transfers: Vec<(String, String, String, f64, String)> = Vec::new();
        let mut contract_creations: Vec<String> = Vec::new();
        let mut high_tip_txs: Vec<(String, f64)> = Vec::new(); // (tx, tip_gwei)

        for tx in &txs {
            let to = tx.get("to").and_then(|v| v.as_str()).unwrap_or("");
            let from = tx.get("from").and_then(|v| v.as_str()).unwrap_or("");
            let input = tx.get("input").and_then(|v| v.as_str()).unwrap_or("");
            let value_hex = tx.get("value").and_then(|v| v.as_str()).unwrap_or("0x0");
            let hash = tx.get("hash").and_then(|v| v.as_str()).unwrap_or("");
            let value_wei = parse_hex_u128(value_hex).unwrap_or(0);
            let value_eth = (value_wei as f64) / 1e18;
            total_value_eth += value_eth;

            // Contract creation
            if to.is_empty() && !input.is_empty() && input.len() > 2 {
                if contract_creations.len() < 3 {
                    contract_creations.push(hash.to_string());
                }
                continue;
            }

            // Known contract interaction
            if let Some(kc) = lookup(to) {
                let entry = contract_hits.entry(kc.name).or_insert((kc.category, 0));
                entry.1 += 1;
                *category_hits.entry(kc.category).or_insert(0) += 1;
            }

            // Method selector decoding
            if let Some(method) = decode_method_selector(input) {
                *method_hits.entry(method).or_insert(0) += 1;
            }

            // Large value transfers
            if value_eth >= 50.0 && large_transfers.len() < 4 {
                large_transfers.push((
                    hash.to_string(),
                    from.to_string(),
                    to.to_string(),
                    value_eth,
                    lookup(to).map(|c| c.name.to_string()).unwrap_or_default(),
                ));
            }

            // High-tip detection (EIP-1559)
            if let (Some(max_fee_s), Some(prio_s)) = (
                tx.get("maxFeePerGas").and_then(|v| v.as_str()),
                tx.get("maxPriorityFeePerGas").and_then(|v| v.as_str()),
            ) {
                let prio_wei = parse_hex_u128(prio_s).unwrap_or(0);
                let prio_gwei = (prio_wei as f64) / 1e9;
                if prio_gwei >= 5.0 && high_tip_txs.len() < 3 {
                    high_tip_txs.push((hash.to_string(), prio_gwei));
                    let _ = max_fee_s; // silence unused
                }
            }
        }

        let tx_total = txs.len();

        // === INSIGHT: top DEX router activity ===
        if let Some((name, (_cat, count))) = contract_hits
            .iter()
            .filter(|(_, (c, _))| *c == ContractCategory::DexRouter)
            .max_by_key(|(_, (_, v))| *v)
        {
            if *count >= 3 {
                let pct = (*count as f32 / tx_total as f32) * 100.0;
                let content = format!(
                    "{} saw {} swaps in block #{} ({:.1}% of {} txs)",
                    name, count, n, pct, tx_total
                );
                self.post_insight("insight", &content).await?;
                posted += 1;
            }
        }

        // === INSIGHT: DEX activity concentration ===
        let dex_total: usize = contract_hits
            .values()
            .filter(|(c, _)| *c == ContractCategory::DexRouter)
            .map(|(_, n)| *n)
            .sum();
        if dex_total >= 6 {
            let routers: Vec<&str> = contract_hits
                .iter()
                .filter(|(_, (c, _))| *c == ContractCategory::DexRouter)
                .map(|(k, _)| *k)
                .collect();
            let content = format!(
                "DEX aggregated: {} router swaps in block #{} across {} routers ({})",
                dex_total,
                n,
                routers.len(),
                routers
                    .iter()
                    .take(3)
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            self.post_insight("insight", &content).await?;
            posted += 1;
        }

        // === INSIGHT: Aave / lending cluster ===
        if let Some(count) = category_hits.get(&ContractCategory::LendingPool) {
            if *count >= 3 {
                let supply = method_hits.get("supply (Aave)").unwrap_or(&0);
                let withdraw = method_hits.get("withdraw (Aave)").unwrap_or(&0);
                let borrow = method_hits.get("borrow (Aave)").unwrap_or(&0);
                let repay = method_hits.get("repay (Aave)").unwrap_or(&0);
                let content = format!(
                    "lending activity spike in block #{}: {} calls (supply={} withdraw={} borrow={} repay={})",
                    n, count, supply, withdraw, borrow, repay
                );
                self.post_insight("insight", &content).await?;
                posted += 1;
            }
        }

        // === INSIGHT: Liquid staking activity ===
        if let Some(count) = category_hits.get(&ContractCategory::Lst) {
            if *count >= 2 {
                let content = format!(
                    "liquid staking activity: {} LST interactions in block #{}",
                    count, n
                );
                self.post_insight("heuristic", &content).await?;
                posted += 1;
            }
        }

        // === INSIGHT: NFT marketplace activity ===
        if let Some(count) = category_hits.get(&ContractCategory::NftMarket) {
            if *count >= 3 {
                let content = format!("NFT marketplace activity: {} trades in block #{}", count, n);
                self.post_insight("insight", &content).await?;
                posted += 1;
            }
        }

        // === INSIGHT: Bridge flows ===
        if let Some(count) = category_hits.get(&ContractCategory::Bridge) {
            if *count >= 2 {
                let bridges: Vec<&str> = contract_hits
                    .iter()
                    .filter(|(_, (c, _))| *c == ContractCategory::Bridge)
                    .map(|(k, _)| *k)
                    .collect();
                let content = format!(
                    "cross-chain bridge activity in block #{}: {} calls via {}",
                    n,
                    count,
                    bridges.join(", ")
                );
                self.post_insight("insight", &content).await?;
                posted += 1;
            }
        }

        // === INSIGHT: stablecoin transfers ===
        if let Some(count) = category_hits.get(&ContractCategory::Stablecoin) {
            if *count >= 5 {
                let stables: Vec<&str> = contract_hits
                    .iter()
                    .filter(|(_, (c, _))| *c == ContractCategory::Stablecoin)
                    .map(|(k, _)| *k)
                    .collect();
                let content = format!(
                    "stablecoin velocity in block #{}: {} transfers across {}",
                    n,
                    count,
                    stables.join(", ")
                );
                self.post_insight("heuristic", &content).await?;
                posted += 1;
            }
        }

        // === INSIGHT: whale transfer with known destination ===
        for (hash, from, to, eth, named) in &large_transfers {
            let to_label = if !named.is_empty() {
                format!("{} ({}..)", named, &to[..8.min(to.len())])
            } else if to.is_empty() {
                "contract creation".to_string()
            } else {
                format!(
                    "{}..{}",
                    &to[..6.min(to.len())],
                    &to[to.len().saturating_sub(4)..]
                )
            };
            let from_short = format!(
                "{}..{}",
                &from[..6.min(from.len())],
                &from[from.len().saturating_sub(4)..]
            );
            let content = format!(
                "whale move in block #{}: {} ETH  {} → {}  (tx {}..)",
                n,
                format_eth(*eth),
                from_short,
                to_label,
                &hash[..10.min(hash.len())]
            );
            self.post_insight("insight", &content).await?;
            posted += 1;
        }

        // === INSIGHT: contract creations ===
        if !contract_creations.is_empty() {
            let content = format!(
                "{} contract(s) deployed in block #{} (first: tx {}..)",
                contract_creations.len(),
                n,
                &contract_creations[0][..10.min(contract_creations[0].len())]
            );
            self.post_insight("insight", &content).await?;
            posted += 1;
        }

        // === PHEROMONE: high priority-fee tx (MEV signal) ===
        if !high_tip_txs.is_empty() {
            let max_tip = high_tip_txs.iter().map(|(_, t)| *t).fold(0.0_f64, f64::max);
            let content = format!(
                "MEV signal: {} tx(s) in block #{} paid priority tips ≥5 gwei (max={:.1} gwei)",
                high_tip_txs.len(),
                n,
                max_tip
            );
            self.deposit("threat", &content, (max_tip / 20.0).min(1.0) as f32)
                .await?;
            posted += 1;
        }

        // === INSIGHT: total ETH transferred (macro flow) ===
        if total_value_eth >= 500.0 {
            let content = format!(
                "{} ETH moved in block #{} across {} txs ({:.1} avg)",
                format_eth(total_value_eth),
                n,
                tx_total,
                total_value_eth / tx_total as f64
            );
            self.post_insight("heuristic", &content).await?;
            posted += 1;
        }

        // === INSIGHT: method-selector distribution ===
        if let Some((top_method, top_count)) = method_hits.iter().max_by_key(|(_, v)| *v) {
            if *top_count >= 8 {
                let pct = (*top_count as f32 / tx_total as f32) * 100.0;
                let content = format!(
                    "block #{} dominated by `{}`: {} calls ({:.0}% of {} txs)",
                    n, top_method, top_count, pct, tx_total
                );
                self.post_insight("heuristic", &content).await?;
                posted += 1;
            }
        }

        Ok(posted)
    }

    /// Multi-block trend analysis: base-fee rise/fall, saturation patterns.
    async fn analyze_trend(&self) -> Result<usize> {
        let snapshot: Vec<ObservedBlock> = {
            let g = self.recent.lock();
            g.iter().cloned().collect()
        };
        if snapshot.len() < 3 {
            return Ok(0);
        }
        let mut posted = 0usize;
        let n = snapshot.len();
        let first = &snapshot[n.saturating_sub(3)];
        let last = &snapshot[n - 1];

        let first_fee = first.base_fee_gwei();
        let last_fee = last.base_fee_gwei();
        if first_fee > 0.1 {
            let pct_change = ((last_fee - first_fee) / first_fee) * 100.0;
            if pct_change.abs() > 25.0 {
                let direction = if pct_change > 0.0 {
                    "rising"
                } else {
                    "falling"
                };
                let content = format!(
                    "base fee {} {:.1}% over 3 blocks: {:.2} → {:.2} gwei (blocks #{}–#{})",
                    direction, pct_change, first_fee, last_fee, first.number, last.number
                );
                self.post_insight("causal_link", &content).await?;
                posted += 1;
                if pct_change > 30.0 {
                    self.deposit(
                        "threat",
                        &format!("gas spike in progress: +{:.0}% over 3 blocks", pct_change),
                        0.85,
                    )
                    .await?;
                    posted += 1;
                }
            }
        }

        // Sustained saturation trend
        let tail = &snapshot[n.saturating_sub(5).max(0)..];
        if tail.len() >= 3 {
            let avg_sat: f32 =
                tail.iter().map(|b| b.saturation_pct).sum::<f32>() / tail.len() as f32;
            if avg_sat > 90.0 {
                let content = format!(
                    "sustained congestion: avg {:.1}% saturation over last {} blocks",
                    avg_sat,
                    tail.len()
                );
                self.post_insight("warning", &content).await?;
                posted += 1;
            } else if avg_sat < 50.0 && avg_sat > 0.0 {
                let content = format!(
                    "network underutilized: avg {:.1}% saturation over last {} blocks",
                    avg_sat,
                    tail.len()
                );
                self.post_insight("heuristic", &content).await?;
                posted += 1;
            }
        }

        // Block time variance
        if snapshot.len() >= 4 {
            let mut deltas: Vec<i64> = Vec::with_capacity(snapshot.len() - 1);
            for w in snapshot.windows(2) {
                deltas.push(w[1].timestamp as i64 - w[0].timestamp as i64);
            }
            let avg_delta = deltas.iter().sum::<i64>() as f32 / deltas.len() as f32;
            if avg_delta > 0.0 {
                let max_delta = *deltas.iter().max().unwrap_or(&0);
                if max_delta > (avg_delta * 2.0) as i64 && max_delta > 18 {
                    let content = format!(
                        "block-time irregularity: max gap {}s (avg {:.1}s) — propagation issue or reorg",
                        max_delta, avg_delta
                    );
                    self.post_insight("warning", &content).await?;
                    posted += 1;
                }
            }
        }

        Ok(posted)
    }

    async fn post_insight(&self, kind: &str, content: &str) -> Result<()> {
        if self.dedup_check(content) {
            return Ok(());
        }
        if self.dry_run {
            info!(dry_run = true, kind, content, "would post insight");
            return Ok(());
        }
        match self
            .mirage
            .chain_post_insight(&self.watcher_id, kind, content, 0)
            .await
        {
            Ok(r) => {
                info!(kind, outcome = %r.outcome, id = %r.id, "posted insight");
            }
            Err(e) => warn!(error = %e, "chain_postInsight failed"),
        }
        Ok(())
    }

    async fn deposit(&self, kind: &str, content: &str, intensity: f32) -> Result<()> {
        if self.dedup_check(content) {
            return Ok(());
        }
        if self.dry_run {
            info!(
                dry_run = true,
                kind, content, intensity, "would deposit pheromone"
            );
            return Ok(());
        }
        match self
            .mirage
            .chain_deposit_pheromone(kind, content, intensity)
            .await
        {
            Ok(id) => info!(kind, id = id.id, intensity, "deposited pheromone"),
            Err(e) => warn!(error = %e, "chain_depositPheromone failed"),
        }
        Ok(())
    }

    /// Run the observer loop.
    #[allow(clippy::cognitive_complexity)]
    pub async fn run(&self, interval: Duration, batch_size: u64) -> Result<()> {
        // Seed: analyze N recent historical blocks on startup
        let tip = self
            .block_number()
            .await
            .context("initial eth_blockNumber")?;
        // Analyze blocks: anchor = tip - batch_size..=tip
        let anchor = tip.saturating_sub(batch_size);
        info!(tip, anchor, "seeding block observer with recent history");
        let _ = self.analyze_range(anchor, tip).await;

        loop {
            tokio::time::sleep(interval).await;
            let latest = match self.block_number().await {
                Ok(n) => n,
                Err(e) => {
                    warn!(error = %e, "block_number poll failed");
                    continue;
                }
            };
            let last = self.last_analyzed.load(Ordering::Relaxed);
            if latest > last {
                // Don't over-run — catch up at most 20 blocks at a time
                let next_start = last.saturating_add(1);
                let next_end = latest.min(next_start.saturating_add(20));
                if let Err(e) = self.analyze_range(next_start, next_end).await {
                    warn!(error = %e, "analyze_range failed");
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct RawBlock {
    number: String,
    hash: Option<String>,
    timestamp: String,
    #[serde(rename = "gasUsed")]
    gas_used: String,
    #[serde(rename = "gasLimit")]
    gas_limit: String,
    #[serde(default, rename = "baseFeePerGas")]
    base_fee_per_gas: Option<String>,
    miner: Option<String>,
    transactions: JsonValue,
}

fn parse_hex_u64(s: &str) -> Result<u64> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(s, 16).with_context(|| format!("bad hex u64: {s}"))
}
fn parse_hex_u128(s: &str) -> Result<u128> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    u128::from_str_radix(s, 16).with_context(|| format!("bad hex u128: {s}"))
}

fn format_eth(v: f64) -> String {
    if v >= 1000.0 {
        format!("{:.0}", v)
    } else if v >= 10.0 {
        format!("{:.1}", v)
    } else {
        format!("{:.3}", v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex() {
        assert_eq!(parse_hex_u64("0x1a").unwrap(), 26);
        assert_eq!(parse_hex_u64("1a").unwrap(), 26);
        assert_eq!(parse_hex_u128("0xff").unwrap(), 255);
    }

    #[test]
    fn format_eth_scales() {
        assert_eq!(format_eth(0.5), "0.500");
        assert_eq!(format_eth(42.7), "42.7");
        assert_eq!(format_eth(1250.0), "1250");
    }

    #[test]
    fn observed_block_gwei_conversion() {
        let b = ObservedBlock {
            number: 1,
            hash: String::new(),
            timestamp: 0,
            gas_used: 0,
            gas_limit: 1,
            base_fee_wei: 25_000_000_000,
            tx_count: 0,
            miner: String::new(),
            saturation_pct: 0.0,
        };
        assert!((b.base_fee_gwei() - 25.0).abs() < 0.01);
    }
}
