//! Block/event feed agent (#6).
//!
//! Reads from `state.chain` ring buffers and publishes block data
//! on `feed:chain:blocks` roughly every 2 seconds.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::time::interval;

use super::{FeedAgent, FeedAgentContext, FeedDescriptor};

pub struct ChainWatcherAgent;

impl FeedAgent for ChainWatcherAgent {
    fn agent_id(&self) -> &'static str {
        "chain-watcher"
    }
    fn display_name(&self) -> &'static str {
        "Chain Block Watcher"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["chain", "blocks", "events"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "chain-blocks".into(),
            topic: "feed:chain:blocks".into(),
            name: "Chain Blocks".into(),
            description: "Latest block data from the connected chain".into(),
            kind: "raw".into(),
            rate: "2s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(2));
            let mut last_block: u64 = 0;
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let block = ctx.state.chain.latest_block.read().await;
                        // Only publish when we have real block data — skip null ticks
                        // to avoid polluting the feed log with empty entries.
                        let Some(b) = block.as_ref() else { continue };
                        if b.number <= last_block { continue }
                        last_block = b.number;
                        let payload = json!({
                            "number": b.number,
                            "hash": b.hash,
                            "timestamp": b.timestamp,
                            "gas_used": b.gas_used,
                            "gas_limit": b.gas_limit,
                            "tx_count": b.tx_count,
                            "base_fee_per_gas": b.base_fee_per_gas,
                        });
                        ctx.publish_tick(
                            self.agent_id(),
                            "chain-blocks",
                            "feed:chain:blocks",
                            payload,
                        );
                    }
                }
            }
        })
    }
}
