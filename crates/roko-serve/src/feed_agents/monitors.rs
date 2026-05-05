//! Meta monitoring agents (#12, #13, #15).

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::time::interval;

use super::{FeedAgent, FeedAgentContext, FeedDescriptor};

// ---------------------------------------------------------------------------
// #12 — Agent Monitor
// ---------------------------------------------------------------------------

pub struct AgentMonitorAgent;

impl FeedAgent for AgentMonitorAgent {
    fn agent_id(&self) -> &'static str {
        "agent-monitor"
    }
    fn display_name(&self) -> &'static str {
        "Agent Monitor"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["meta", "monitoring", "agents"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "meta-agents".into(),
            topic: "feed:meta:agents".into(),
            name: "Agent Monitor".into(),
            description: "Agent count and heartbeat status".into(),
            kind: "meta".into(),
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
                        let agent_count = ctx.state.agent_count.load(std::sync::atomic::Ordering::Relaxed);
                        let heartbeats_len = ctx.state.heartbeats.read().await.len();
                        let catalog = ctx.state.feed_agent_catalog.read().await;
                        let feed_agent_count = catalog.agents.len();
                        let total_feeds = catalog.feeds.len();
                        drop(catalog);

                        let payload = json!({
                            "relay_agent_count": agent_count,
                            "heartbeat_buffer_size": heartbeats_len,
                            "feed_agent_count": feed_agent_count,
                            "total_feeds": total_feeds,
                        });
                        ctx.publish_tick(
                            self.agent_id(),
                            "meta-agents",
                            "feed:meta:agents",
                            payload,
                        );
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// #13 — Confidence Scorer
// ---------------------------------------------------------------------------

pub struct ConfidenceScorerAgent;

impl FeedAgent for ConfidenceScorerAgent {
    fn agent_id(&self) -> &'static str {
        "isfr-confidence"
    }
    fn display_name(&self) -> &'static str {
        "Confidence Scorer"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["isfr", "confidence", "health"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "isfr-confidence".into(),
            topic: "feed:isfr:confidence".into(),
            name: "Source Confidence".into(),
            description: "Per-source health decomposition and confidence score".into(),
            kind: "derived".into(),
            rate: "15s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(15));
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let sources = ctx.state.isfr.sources.read().await;
                        if sources.is_empty() {
                            continue;
                        }
                        let total = sources.len();
                        let live = sources.iter().filter(|s| s.health == "live").count();
                        let stale = sources.iter().filter(|s| s.health == "stale").count();
                        let offline = sources.iter().filter(|s| s.health == "offline").count();
                        let confidence_pct = if total > 0 { (live as f64 / total as f64) * 100.0 } else { 0.0 };

                        let source_details: Vec<serde_json::Value> = sources.iter().map(|s| {
                            json!({
                                "name": s.name,
                                "health": s.health,
                                "weight": s.weight,
                                "last_rate_bps": s.last_rate_bps,
                            })
                        }).collect();

                        let payload = json!({
                            "confidence_pct": confidence_pct,
                            "total_sources": total,
                            "live": live,
                            "stale": stale,
                            "offline": offline,
                            "sources": source_details,
                        });
                        ctx.publish_tick(
                            self.agent_id(),
                            "isfr-confidence",
                            "feed:isfr:confidence",
                            payload,
                        );
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// #15 — Relay Stats Agent
// ---------------------------------------------------------------------------

pub struct RelayStatsAgent;

impl FeedAgent for RelayStatsAgent {
    fn agent_id(&self) -> &'static str {
        "relay-stats"
    }
    fn display_name(&self) -> &'static str {
        "Relay Stats"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["meta", "relay", "stats"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "meta-relay".into(),
            topic: "feed:meta:relay".into(),
            name: "Relay Stats".into(),
            description: "Relay connection health and throughput stats".into(),
            kind: "meta".into(),
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
                        let connection = {
                            let rh = ctx.state.relay_health.read();
                            serde_json::to_value(&rh.connection).unwrap_or_default()
                        };

                        let catalog = ctx.state.feed_agent_catalog.read().await;
                        let msgs_per_sec = catalog.messages_per_sec;
                        let total_feeds = catalog.feeds.len();
                        drop(catalog);

                        let payload = json!({
                            "relay_connection": connection,
                            "total_feeds": total_feeds,
                            "est_messages_per_sec": msgs_per_sec,
                        });
                        ctx.publish_tick(
                            self.agent_id(),
                            "meta-relay",
                            "feed:meta:relay",
                            payload,
                        );
                    }
                }
            }
        })
    }
}
