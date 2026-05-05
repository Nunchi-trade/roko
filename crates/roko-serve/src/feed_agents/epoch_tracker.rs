//! Epoch countdown feed agent (#10).
//!
//! Reads the keeper's current epoch counter and config, then publishes
//! epoch info on `feed:isfr:epochs` every 5 seconds.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::time::interval;

use super::{FeedAgent, FeedAgentContext, FeedDescriptor};

pub struct EpochTrackerAgent;

impl FeedAgent for EpochTrackerAgent {
    fn agent_id(&self) -> &'static str {
        "isfr-epoch-tracker"
    }
    fn display_name(&self) -> &'static str {
        "Epoch Tracker"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["isfr", "epoch", "countdown"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "isfr-epochs".into(),
            topic: "feed:isfr:epochs".into(),
            name: "Epoch Countdown".into(),
            description: "ISFR keeper epoch counter and countdown".into(),
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
                        let epoch = ctx.state.isfr.current_epoch.load(std::sync::atomic::Ordering::Relaxed);
                        let config = ctx.state.load_roko_config();
                        let epoch_duration = config.isfr.epoch_duration_secs;
                        let keeper_running = ctx.state.isfr.keeper_running.load(std::sync::atomic::Ordering::Relaxed);

                        let payload = json!({
                            "current_epoch": epoch,
                            "epoch_duration_secs": epoch_duration,
                            "keeper_running": keeper_running,
                        });
                        ctx.publish_tick(
                            self.agent_id(),
                            "isfr-epochs",
                            "feed:isfr:epochs",
                            payload,
                        );
                    }
                }
            }
        })
    }
}
