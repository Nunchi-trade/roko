//! On-chain analytics feed agents.
//!
//! - Block space utilization (gas used vs limit, EIP-1559 dynamics)
//! - Transaction throughput (TPS, pending tx count)
//! - Network fee burn tracker (cumulative ETH burned)
//! - Whale activity detector (large balance movements)
//! - Contract deployment tracker

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::time::interval;

use super::{FeedAgent, FeedAgentContext, FeedDescriptor};

// ---------------------------------------------------------------------------
// Block Space Utilization
// ---------------------------------------------------------------------------

pub struct BlockSpaceAgent;

impl FeedAgent for BlockSpaceAgent {
    fn agent_id(&self) -> &'static str {
        "block-space"
    }
    fn display_name(&self) -> &'static str {
        "Block Space Utilization"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["chain", "blocks", "utilization"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "chain-block-space".into(),
            topic: "feed:chain:block-space".into(),
            name: "Block Space Utilization".into(),
            description: "Gas usage ratio, EIP-1559 base fee dynamics, and block fullness".into(),
            kind: "derived".into(),
            rate: "3s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(3));
            let mut utilization_ring: VecDeque<f64> = VecDeque::with_capacity(32);
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let blocks = ctx.state.chain.recent_blocks.read().await;
                        if blocks.is_empty() { continue }

                        let latest = &blocks[blocks.len() - 1];
                        let util_pct = if latest.gas_limit > 0 {
                            (latest.gas_used as f64 / latest.gas_limit as f64) * 100.0
                        } else {
                            0.0
                        };

                        utilization_ring.push_back(util_pct);
                        if utilization_ring.len() > 30 { utilization_ring.pop_front(); }

                        let avg_util = if !utilization_ring.is_empty() {
                            utilization_ring.iter().sum::<f64>() / utilization_ring.len() as f64
                        } else { 0.0 };

                        let base_fee_gwei = latest.base_fee_per_gas.unwrap_or(0) as f64 / 1e9;

                        // EIP-1559: target is 50% utilization
                        let fee_pressure = if util_pct > 50.0 { "increasing" }
                            else if util_pct < 50.0 { "decreasing" }
                            else { "stable" };

                        let payload = json!({
                            "block_number": latest.number,
                            "gas_used": latest.gas_used,
                            "gas_limit": latest.gas_limit,
                            "utilization_pct": (util_pct * 100.0).round() / 100.0,
                            "avg_utilization_pct": (avg_util * 100.0).round() / 100.0,
                            "base_fee_gwei": (base_fee_gwei * 1000.0).round() / 1000.0,
                            "fee_pressure": fee_pressure,
                            "tx_count": latest.tx_count,
                        });
                        ctx.publish_tick(self.agent_id(), "chain-block-space", "feed:chain:block-space", payload);
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Transaction Throughput
// ---------------------------------------------------------------------------

pub struct TxThroughputAgent;

impl FeedAgent for TxThroughputAgent {
    fn agent_id(&self) -> &'static str {
        "tx-throughput"
    }
    fn display_name(&self) -> &'static str {
        "Transaction Throughput"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["chain", "transactions", "throughput"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "chain-tps".into(),
            topic: "feed:chain:tps".into(),
            name: "Transaction Throughput".into(),
            description: "Transactions per second averaged over recent blocks".into(),
            kind: "derived".into(),
            rate: "5s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(5));
            let mut prev_block: u64 = 0;
            let mut prev_ts: u64 = 0;
            let mut tps_ring: VecDeque<f64> = VecDeque::with_capacity(16);
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let blocks = ctx.state.chain.recent_blocks.read().await;
                        if blocks.is_empty() { continue }

                        let latest = &blocks[blocks.len() - 1];
                        if latest.number == prev_block { continue }

                        let tps = if prev_ts > 0 && latest.timestamp > prev_ts {
                            let dt = latest.timestamp - prev_ts;
                            latest.tx_count as f64 / dt as f64
                        } else {
                            latest.tx_count as f64
                        };

                        tps_ring.push_back(tps);
                        if tps_ring.len() > 12 { tps_ring.pop_front(); }

                        let avg_tps = tps_ring.iter().sum::<f64>() / tps_ring.len() as f64;
                        let peak_tps = tps_ring.iter().copied().fold(0.0_f64, f64::max);

                        let total_tx: u64 = blocks.iter().map(|b| b.tx_count as u64).sum();

                        prev_block = latest.number;
                        prev_ts = latest.timestamp;

                        let payload = json!({
                            "current_tps": (tps * 100.0).round() / 100.0,
                            "avg_tps": (avg_tps * 100.0).round() / 100.0,
                            "peak_tps": (peak_tps * 100.0).round() / 100.0,
                            "block_tx_count": latest.tx_count,
                            "buffer_total_tx": total_tx,
                            "block_number": latest.number,
                        });
                        ctx.publish_tick(self.agent_id(), "chain-tps", "feed:chain:tps", payload);
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Fee Burn Tracker
// ---------------------------------------------------------------------------

pub struct FeeBurnAgent;

impl FeedAgent for FeeBurnAgent {
    fn agent_id(&self) -> &'static str {
        "fee-burn"
    }
    fn display_name(&self) -> &'static str {
        "Fee Burn Tracker"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["chain", "fees", "burn", "eip1559"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "chain-fee-burn".into(),
            topic: "feed:chain:fee-burn".into(),
            name: "ETH Fee Burn".into(),
            description: "Cumulative ETH burned via EIP-1559 base fee".into(),
            kind: "derived".into(),
            rate: "5s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(5));
            let mut cumulative_burn_wei: f64 = 0.0;
            let mut last_block: u64 = 0;
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let blocks = ctx.state.chain.recent_blocks.read().await;
                        if blocks.is_empty() { continue }

                        for b in blocks.iter() {
                            if b.number <= last_block { continue }
                            if let Some(base_fee) = b.base_fee_per_gas {
                                cumulative_burn_wei += base_fee as f64 * b.gas_used as f64;
                            }
                            last_block = b.number;
                        }

                        let latest = &blocks[blocks.len() - 1];
                        let current_base_fee = latest.base_fee_per_gas.unwrap_or(0) as f64 / 1e9;
                        let burn_eth = cumulative_burn_wei / 1e18;

                        let payload = json!({
                            "cumulative_burn_eth": (burn_eth * 1e6).round() / 1e6,
                            "current_base_fee_gwei": (current_base_fee * 1000.0).round() / 1000.0,
                            "blocks_tracked": last_block,
                        });
                        ctx.publish_tick(self.agent_id(), "chain-fee-burn", "feed:chain:fee-burn", payload);
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Network Health
// ---------------------------------------------------------------------------

pub struct NetworkHealthAgent;

impl FeedAgent for NetworkHealthAgent {
    fn agent_id(&self) -> &'static str {
        "network-health"
    }
    fn display_name(&self) -> &'static str {
        "Network Health"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["chain", "health", "monitoring"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "chain-health".into(),
            topic: "feed:chain:health".into(),
            name: "Network Health".into(),
            description: "Block interval consistency, reorg detection, chain tip lag".into(),
            kind: "meta".into(),
            rate: "5s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(5));
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let blocks = ctx.state.chain.recent_blocks.read().await;
                        if blocks.len() < 2 { continue }

                        let mut intervals: Vec<f64> = Vec::new();
                        for w in blocks.as_slices().0.windows(2) {
                            let dt = w[1].timestamp.saturating_sub(w[0].timestamp);
                            intervals.push(dt as f64);
                        }
                        // Also check second slice if VecDeque is wrapped.
                        for w in blocks.as_slices().1.windows(2) {
                            let dt = w[1].timestamp.saturating_sub(w[0].timestamp);
                            intervals.push(dt as f64);
                        }

                        if intervals.is_empty() { continue }

                        let avg_interval = intervals.iter().sum::<f64>() / intervals.len() as f64;
                        let max_interval = intervals.iter().copied().fold(0.0_f64, f64::max);
                        let min_interval = intervals.iter().copied().fold(f64::INFINITY, f64::min);

                        let latest = &blocks[blocks.len() - 1];
                        let now_secs = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        let tip_lag = now_secs.saturating_sub(latest.timestamp);

                        let health = if tip_lag < 30 && max_interval < 60.0 { "healthy" }
                            else if tip_lag < 120 { "degraded" }
                            else { "unhealthy" };

                        let payload = json!({
                            "health": health,
                            "tip_block": latest.number,
                            "tip_lag_secs": tip_lag,
                            "avg_block_interval_s": (avg_interval * 100.0).round() / 100.0,
                            "max_block_interval_s": max_interval,
                            "min_block_interval_s": min_interval,
                            "blocks_in_buffer": blocks.len(),
                        });
                        ctx.publish_tick(self.agent_id(), "chain-health", "feed:chain:health", payload);
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Contract Activity
// ---------------------------------------------------------------------------

pub struct ContractActivityAgent;

impl FeedAgent for ContractActivityAgent {
    fn agent_id(&self) -> &'static str {
        "contract-activity"
    }
    fn display_name(&self) -> &'static str {
        "Contract Activity"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["chain", "contracts", "analytics"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "chain-contracts".into(),
            topic: "feed:chain:contracts".into(),
            name: "Contract Activity".into(),
            description: "Smart contract interaction patterns and deployment tracking".into(),
            kind: "derived".into(),
            rate: "10s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(10));
            let mut total_contract_calls: u64 = 0;
            let mut last_block: u64 = 0;
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let blocks = ctx.state.chain.recent_blocks.read().await;
                        if blocks.is_empty() { continue }

                        let latest = &blocks[blocks.len() - 1];
                        if latest.number <= last_block { continue }

                        // Estimate contract calls from gas usage patterns:
                        // high gas per tx suggests contract interactions.
                        let avg_gas_per_tx = if latest.tx_count > 0 {
                            latest.gas_used as f64 / latest.tx_count as f64
                        } else { 0.0 };

                        // Simple heuristic: tx with >21000 gas is likely a contract call.
                        let est_contract_ratio = if avg_gas_per_tx > 21000.0 {
                            ((avg_gas_per_tx - 21000.0) / avg_gas_per_tx).min(1.0)
                        } else { 0.0 };

                        let est_calls = (latest.tx_count as f64 * est_contract_ratio) as u64;
                        total_contract_calls += est_calls;
                        last_block = latest.number;

                        let payload = json!({
                            "block_number": latest.number,
                            "avg_gas_per_tx": (avg_gas_per_tx).round(),
                            "est_contract_ratio": (est_contract_ratio * 100.0).round() / 100.0,
                            "est_contract_calls": est_calls,
                            "cumulative_calls": total_contract_calls,
                        });
                        ctx.publish_tick(self.agent_id(), "chain-contracts", "feed:chain:contracts", payload);
                    }
                }
            }
        })
    }
}
