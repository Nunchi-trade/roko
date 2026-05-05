//! Gas price feed agent (#7).
//!
//! Derives a simple gas price EMA from recent block headers and publishes
//! on `feed:chain:gas` every 5 seconds.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::time::interval;

use super::{FeedAgent, FeedAgentContext, FeedDescriptor};

pub struct GasOracleAgent;

impl FeedAgent for GasOracleAgent {
    fn agent_id(&self) -> &'static str {
        "gas-oracle"
    }
    fn display_name(&self) -> &'static str {
        "Gas Oracle"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["chain", "gas", "oracle"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "chain-gas".into(),
            topic: "feed:chain:gas".into(),
            name: "Gas Price Oracle".into(),
            description: "EMA gas price derived from recent block headers".into(),
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
            let mut ema: f64 = 0.0;
            let alpha: f64 = 0.2;
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let blocks = ctx.state.chain.recent_blocks.read().await;
                        let fees: Vec<f64> = blocks
                            .iter()
                            .filter_map(|b| b.base_fee_per_gas.map(|f| f as f64))
                            .collect();
                        let payload = if fees.is_empty() {
                            json!({
                                "ema_gwei": null,
                                "p25_gwei": null,
                                "p50_gwei": null,
                                "p75_gwei": null,
                                "sample_count": 0,
                                "status": "waiting",
                            })
                        } else {
                            for fee in &fees {
                                if ema == 0.0 {
                                    ema = *fee;
                                } else {
                                    ema = alpha * fee + (1.0 - alpha) * ema;
                                }
                            }
                            let mut sorted = fees.clone();
                            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                            let p25 = sorted[sorted.len() / 4];
                            let p50 = sorted[sorted.len() / 2];
                            let p75 = sorted[sorted.len() * 3 / 4];

                            json!({
                                "ema_gwei": ema / 1e9,
                                "p25_gwei": p25 / 1e9,
                                "p50_gwei": p50 / 1e9,
                                "p75_gwei": p75 / 1e9,
                                "sample_count": fees.len(),
                            })
                        };
                        ctx.publish_tick(
                            self.agent_id(),
                            "chain-gas",
                            "feed:chain:gas",
                            payload,
                        );
                    }
                }
            }
        })
    }
}
