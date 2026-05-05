//! On-chain oracle submission agent (#11).
//!
//! Monitors epoch transitions and publishes submission status on
//! `feed:chain:oracle`. Does NOT perform actual submission (that's handled
//! by the ISFR keeper's publish callback); this agent reports status.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::time::interval;

use super::{FeedAgent, FeedAgentContext, FeedDescriptor};

pub struct OracleSubmitterAgent;

impl FeedAgent for OracleSubmitterAgent {
    fn agent_id(&self) -> &'static str {
        "oracle-submitter"
    }
    fn display_name(&self) -> &'static str {
        "Oracle Submitter"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["chain", "oracle", "submission"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "chain-oracle".into(),
            topic: "feed:chain:oracle".into(),
            name: "Oracle Submission Status".into(),
            description: "On-chain rate submission status per epoch".into(),
            kind: "meta".into(),
            rate: "per-epoch".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(10));
            let mut last_epoch: u64 = u64::MAX;
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let epoch = ctx.state.isfr.current_epoch.load(std::sync::atomic::Ordering::Relaxed);
                        let epoch_changed = epoch != last_epoch;
                        if epoch_changed {
                            last_epoch = epoch;
                        }

                        let has_oracle = ctx.state.isfr.contract_addresses.read().await
                            .as_ref()
                            .and_then(|a| a.isfr_oracle.as_ref())
                            .is_some();
                        let has_wallet = ctx.state.load_roko_config()
                            .chain.wallet_key.as_ref()
                            .is_some_and(|k| !k.is_empty());

                        let payload = json!({
                            "epoch": epoch,
                            "epoch_changed": epoch_changed,
                            "oracle_configured": has_oracle,
                            "wallet_configured": has_wallet,
                            "status": if has_oracle && has_wallet { "submitting" } else { "skipped" },
                        });
                        ctx.publish_tick(
                            self.agent_id(),
                            "chain-oracle",
                            "feed:chain:oracle",
                            payload,
                        );
                    }
                }
            }
        })
    }
}
