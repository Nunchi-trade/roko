//! DeFi analytics feed agents.
//!
//! - Yield curve (term structure across ISFR sources)
//! - Liquidation risk monitor
//! - Protocol TVL tracker
//! - Stablecoin peg monitor
//! - MEV opportunity tracker

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::time::interval;

use super::{FeedAgent, FeedAgentContext, FeedDescriptor};

// ---------------------------------------------------------------------------
// Yield Curve Agent
// ---------------------------------------------------------------------------

pub struct YieldCurveAgent;

impl FeedAgent for YieldCurveAgent {
    fn agent_id(&self) -> &'static str {
        "yield-curve"
    }
    fn display_name(&self) -> &'static str {
        "DeFi Yield Curve"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["defi", "yield", "curve", "analytics"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "defi-yield-curve".into(),
            topic: "feed:defi:yield-curve".into(),
            name: "Yield Curve".into(),
            description:
                "Term structure of DeFi yields across lending, staking, and structured products"
                    .into(),
            kind: "composite".into(),
            rate: "30s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(30));
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let rate = ctx.state.isfr.current_rate.read().await;
                        let sources = ctx.state.isfr.sources.read().await;

                        if sources.is_empty() { continue }

                        // Build yield curve: sort sources by rate (proxy for duration risk).
                        let mut curve_points: Vec<serde_json::Value> = sources.iter().map(|s| {
                            json!({
                                "name": s.name,
                                "class": s.class,
                                "rate_bps": s.last_rate_bps,
                                "weight": s.weight,
                                "health": s.health,
                            })
                        }).collect();
                        curve_points.sort_by(|a, b| {
                            let ra = a["rate_bps"].as_u64().unwrap_or(0);
                            let rb = b["rate_bps"].as_u64().unwrap_or(0);
                            ra.cmp(&rb)
                        });

                        let composite = rate.as_ref().map(|r| r.composite_bps).unwrap_or(0);
                        let spread = if curve_points.len() >= 2 {
                            let lo = curve_points.first().and_then(|p| p["rate_bps"].as_u64()).unwrap_or(0);
                            let hi = curve_points.last().and_then(|p| p["rate_bps"].as_u64()).unwrap_or(0);
                            hi.saturating_sub(lo)
                        } else { 0 };

                        let payload = json!({
                            "composite_bps": composite,
                            "curve_spread_bps": spread,
                            "curve_points": curve_points,
                            "source_count": sources.len(),
                        });
                        ctx.publish_tick(self.agent_id(), "defi-yield-curve", "feed:defi:yield-curve", payload);
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Liquidation Risk Monitor
// ---------------------------------------------------------------------------

pub struct LiquidationRiskAgent;

impl FeedAgent for LiquidationRiskAgent {
    fn agent_id(&self) -> &'static str {
        "liquidation-risk"
    }
    fn display_name(&self) -> &'static str {
        "Liquidation Risk Monitor"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["defi", "liquidation", "risk"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "defi-liquidation-risk".into(),
            topic: "feed:defi:liquidation-risk".into(),
            name: "Liquidation Risk".into(),
            description: "Estimated liquidation risk levels from rate and volatility data".into(),
            kind: "derived".into(),
            rate: "20s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(20));
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let history = ctx.state.isfr.rate_history.read().await;
                        let sources = ctx.state.isfr.sources.read().await;

                        if history.len() < 3 { continue }

                        // Calculate rate volatility as a liquidation risk proxy.
                        let rates: Vec<f64> = history.iter().map(|r| r.composite_bps as f64).collect();
                        let n = rates.len() as f64;
                        let mean = rates.iter().sum::<f64>() / n;
                        let variance = rates.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
                        let stddev = variance.sqrt();
                        let cv = if mean > 0.0 { stddev / mean } else { 0.0 };

                        // Risk tiers based on coefficient of variation.
                        let risk_level = if cv > 0.15 { "high" }
                            else if cv > 0.05 { "medium" }
                            else { "low" };

                        let stale_count = sources.iter().filter(|s| s.health != "live").count();
                        let data_quality = if sources.is_empty() { "unknown" }
                            else if stale_count == 0 { "excellent" }
                            else if stale_count <= sources.len() / 2 { "fair" }
                            else { "poor" };

                        let payload = json!({
                            "risk_level": risk_level,
                            "rate_volatility_bps": (stddev * 100.0).round() / 100.0,
                            "coefficient_of_variation": (cv * 10000.0).round() / 10000.0,
                            "mean_rate_bps": (mean * 100.0).round() / 100.0,
                            "data_quality": data_quality,
                            "stale_sources": stale_count,
                            "samples": history.len(),
                        });
                        ctx.publish_tick(self.agent_id(), "defi-liquidation-risk", "feed:defi:liquidation-risk", payload);
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Protocol TVL Tracker
// ---------------------------------------------------------------------------

pub struct TvlTrackerAgent;

impl FeedAgent for TvlTrackerAgent {
    fn agent_id(&self) -> &'static str {
        "tvl-tracker"
    }
    fn display_name(&self) -> &'static str {
        "Protocol TVL Tracker"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["defi", "tvl", "protocol"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "defi-tvl".into(),
            topic: "feed:defi:tvl".into(),
            name: "Protocol TVL".into(),
            description: "Estimated total value locked across monitored protocols".into(),
            kind: "composite".into(),
            rate: "30s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(30));
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let sources = ctx.state.isfr.sources.read().await;
                        if sources.is_empty() { continue }

                        // Derive synthetic TVL from source weights and rate data.
                        // In a real deployment this would query on-chain balances;
                        // here we use a weighted model based on ISFR source data.
                        let mut protocol_tvls: Vec<serde_json::Value> = Vec::new();
                        let mut total_tvl_est: f64 = 0.0;

                        for s in sources.iter() {
                            // Synthetic TVL: higher rate × higher weight → more capital.
                            // Base of $100M per source, scaled by weight.
                            let rate_bps = s.last_rate_bps.unwrap_or(0);
                            let base_tvl = 100_000_000.0 * s.weight;
                            let rate_factor = 1.0 + (rate_bps as f64 / 10_000.0);
                            let est_tvl = base_tvl * rate_factor;
                            total_tvl_est += est_tvl;

                            protocol_tvls.push(json!({
                                "name": s.name,
                                "class": s.class,
                                "est_tvl_usd": (est_tvl).round(),
                                "weight": s.weight,
                                "rate_bps": rate_bps,
                            }));
                        }

                        let payload = json!({
                            "total_est_tvl_usd": total_tvl_est.round(),
                            "protocol_count": sources.len(),
                            "protocols": protocol_tvls,
                        });
                        ctx.publish_tick(self.agent_id(), "defi-tvl", "feed:defi:tvl", payload);
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Stablecoin Peg Monitor
// ---------------------------------------------------------------------------

pub struct StablecoinPegAgent;

impl FeedAgent for StablecoinPegAgent {
    fn agent_id(&self) -> &'static str {
        "stablecoin-peg"
    }
    fn display_name(&self) -> &'static str {
        "Stablecoin Peg Monitor"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["defi", "stablecoin", "peg", "monitoring"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "defi-stablecoin-peg".into(),
            topic: "feed:defi:stablecoin-peg".into(),
            name: "Stablecoin Peg".into(),
            description: "Simulated peg deviation for major stablecoins".into(),
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
            let mut tick_count: u64 = 0;
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        tick_count += 1;

                        // Derive peg deviations from gas volatility as a proxy for
                        // market stress (in production, we'd read DEX prices).
                        let blocks = ctx.state.chain.recent_blocks.read().await;
                        let stress = if blocks.len() >= 2 {
                            let fees: Vec<f64> = blocks.iter()
                                .filter_map(|b| b.base_fee_per_gas.map(|f| f as f64))
                                .collect();
                            if fees.len() >= 2 {
                                let mean = fees.iter().sum::<f64>() / fees.len() as f64;
                                let var = fees.iter().map(|f| (f - mean).powi(2)).sum::<f64>() / fees.len() as f64;
                                (var.sqrt() / mean.max(1.0)).min(1.0)
                            } else { 0.0 }
                        } else { 0.0 };

                        // Synthetic peg: tight around $1, widens with stress.
                        let base_dev = 0.0001; // 1 bps normal deviation
                        let stress_dev = stress * 0.005; // up to 50 bps under stress

                        // Deterministic pseudo-random per stablecoin using tick count.
                        let usdc_dev = base_dev + stress_dev * ((tick_count % 7) as f64 / 7.0);
                        let usdt_dev = base_dev + stress_dev * ((tick_count % 11) as f64 / 11.0);
                        let dai_dev = base_dev * 1.5 + stress_dev * ((tick_count % 13) as f64 / 13.0);

                        let payload = json!({
                            "usdc": { "price": 1.0 - usdc_dev + usdc_dev * 2.0 * ((tick_count % 3) as f64 / 3.0), "deviation_bps": (usdc_dev * 10000.0).round() },
                            "usdt": { "price": 1.0 - usdt_dev + usdt_dev * 2.0 * ((tick_count % 5) as f64 / 5.0), "deviation_bps": (usdt_dev * 10000.0).round() },
                            "dai": { "price": 1.0 - dai_dev + dai_dev * 2.0 * ((tick_count % 7) as f64 / 7.0), "deviation_bps": (dai_dev * 10000.0).round() },
                            "market_stress": (stress * 10000.0).round() / 10000.0,
                            "overall_peg_health": if stress < 0.1 { "stable" } else if stress < 0.3 { "minor_deviation" } else { "stress" },
                        });
                        ctx.publish_tick(self.agent_id(), "defi-stablecoin-peg", "feed:defi:stablecoin-peg", payload);
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// MEV Opportunity Tracker
// ---------------------------------------------------------------------------

pub struct MevTrackerAgent;

impl FeedAgent for MevTrackerAgent {
    fn agent_id(&self) -> &'static str {
        "mev-tracker"
    }
    fn display_name(&self) -> &'static str {
        "MEV Opportunity Tracker"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["chain", "mev", "analytics"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "chain-mev".into(),
            topic: "feed:chain:mev".into(),
            name: "MEV Opportunities".into(),
            description: "Estimated MEV extraction opportunities from gas and tx patterns".into(),
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
            let mut cumulative_est_mev: f64 = 0.0;
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let blocks = ctx.state.chain.recent_blocks.read().await;
                        if blocks.is_empty() { continue }

                        let latest = &blocks[blocks.len() - 1];

                        // MEV estimation heuristic: blocks with high gas utilization
                        // and many transactions suggest more sandwich/arb opportunities.
                        let util = if latest.gas_limit > 0 {
                            latest.gas_used as f64 / latest.gas_limit as f64
                        } else { 0.0 };

                        let base_fee_eth = latest.base_fee_per_gas.unwrap_or(0) as f64 / 1e18;
                        // Estimated MEV per block: function of utilization × base fee.
                        let est_mev_eth = util * base_fee_eth * latest.tx_count as f64 * 0.01;
                        cumulative_est_mev += est_mev_eth;

                        let opportunity = if util > 0.9 && latest.tx_count > 10 { "high" }
                            else if util > 0.5 { "medium" }
                            else { "low" };

                        let payload = json!({
                            "opportunity_level": opportunity,
                            "block_utilization_pct": (util * 100.0).round() / 100.0,
                            "est_mev_eth": (est_mev_eth * 1e8).round() / 1e8,
                            "cumulative_est_mev_eth": (cumulative_est_mev * 1e6).round() / 1e6,
                            "block_tx_count": latest.tx_count,
                            "block_number": latest.number,
                        });
                        ctx.publish_tick(self.agent_id(), "chain-mev", "feed:chain:mev", payload);
                    }
                }
            }
        })
    }
}
