//! Per-source ISFR scout agents (#2-5).
//!
//! Each agent reads from `state.isfr.sources` and publishes individual
//! source health data on its own topic.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::time::interval;

use super::{FeedAgent, FeedAgentContext, FeedDescriptor};

macro_rules! scout_agent {
    ($struct_name:ident, $agent_id:expr, $display:expr, $source_name:expr, $feed_id:expr, $topic:expr) => {
        pub struct $struct_name;

        impl FeedAgent for $struct_name {
            fn agent_id(&self) -> &'static str {
                $agent_id
            }
            fn display_name(&self) -> &'static str {
                $display
            }
            fn capabilities(&self) -> Vec<&str> {
                vec!["isfr", "source", "scout"]
            }
            fn feeds(&self) -> Vec<FeedDescriptor> {
                vec![FeedDescriptor {
                    feed_id: $feed_id.into(),
                    topic: $topic.into(),
                    name: format!("{} Source Feed", $display),
                    description: format!("Per-source rate data from {}", $source_name),
                    kind: "raw".into(),
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
                                let sources = ctx.state.isfr.sources.read().await;
                                let payload = if let Some(src) = sources.iter().find(|s| s.name == $source_name) {
                                    json!({
                                        "source": src.name,
                                        "class": src.class,
                                        "weight": src.weight,
                                        "last_rate_bps": src.last_rate_bps,
                                        "health": src.health,
                                        "last_poll_ms": src.last_poll_ms,
                                    })
                                } else {
                                    json!({
                                        "source": $source_name,
                                        "class": null,
                                        "weight": null,
                                        "last_rate_bps": null,
                                        "health": "unavailable",
                                        "last_poll_ms": null,
                                        "status": "waiting",
                                    })
                                };
                                ctx.publish_tick(
                                    self.agent_id(),
                                    $feed_id,
                                    $topic,
                                    payload,
                                );
                            }
                        }
                    }
                })
            }
        }
    };
}

scout_agent!(AaveScoutAgent, "isfr-scout-aave", "Aave Scout", "aave", "isfr-aave", "feed:isfr:aave");
scout_agent!(CompoundScoutAgent, "isfr-scout-compound", "Compound Scout", "compound", "isfr-compound", "feed:isfr:compound");
scout_agent!(EthenaScoutAgent, "isfr-scout-ethena", "Ethena Scout", "ethena", "isfr-ethena", "feed:isfr:ethena");
scout_agent!(LidoScoutAgent, "isfr-scout-lido", "Lido Scout", "lido", "isfr-lido", "feed:isfr:lido");
