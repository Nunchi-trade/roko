//! ISFR composite keeper feed agent (#1).
//!
//! Reads from `state.isfr.current_rate` and publishes the weighted composite
//! on `feed:isfr:composite` every 10 seconds.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::time::interval;

use super::{FeedAgent, FeedAgentContext, FeedDescriptor};

pub struct IsfrKeeperAgent;

impl FeedAgent for IsfrKeeperAgent {
    fn agent_id(&self) -> &'static str {
        "isfr-keeper"
    }
    fn display_name(&self) -> &'static str {
        "ISFR Composite Keeper"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["isfr", "composite", "rate"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "isfr-composite".into(),
            topic: "feed:isfr:composite".into(),
            name: "ISFR Composite Rate".into(),
            description: "Weighted composite interest rate across all sources".into(),
            kind: "composite".into(),
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
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let rate = ctx.state.isfr.current_rate.read().await;
                        let payload = if let Some(r) = rate.as_ref() {
                            json!({
                                "composite_bps": r.composite_bps,
                                "lending_bps": r.lending_bps,
                                "structured_bps": r.structured_bps,
                                "funding_bps": r.funding_bps,
                                "staking_bps": r.staking_bps,
                                "confidence_bps": r.confidence_bps,
                                "source_count": r.readings.len(),
                                "timestamp_ms": r.timestamp_ms,
                            })
                        } else {
                            json!({
                                "composite_bps": null,
                                "lending_bps": null,
                                "structured_bps": null,
                                "funding_bps": null,
                                "staking_bps": null,
                                "confidence_bps": null,
                                "source_count": 0,
                                "timestamp_ms": null,
                                "status": "waiting",
                            })
                        };
                        ctx.publish_tick(
                            self.agent_id(),
                            "isfr-composite",
                            "feed:isfr:composite",
                            payload,
                        );
                    }
                }
            }
        })
    }
}
