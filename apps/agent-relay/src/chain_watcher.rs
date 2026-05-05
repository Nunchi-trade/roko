//! Chain event watcher — subscribes to new blocks via WebSocket RPC
//! and publishes them to the relay's [`TopicBus`].
//!
//! When `--rpc-ws-url` is provided to the relay binary, a background task
//! polls `eth_blockNumber` every 2 seconds and emits a `new_block`
//! [`TopicEnvelope`] on the topic `chain:{chain_id}`.  All agents
//! subscribed to that topic receive the block notification.
//!
//! Future: replace polling with alloy's `eth_subscribe("newHeads")` for
//! instant notifications.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::protocol::{RelayOutboundFrame, TopicEnvelope};
use crate::state::RelayState;

/// Configuration for the chain watcher background task.
pub struct ChainWatcherConfig {
    /// WebSocket RPC URL (e.g. `ws://localhost:8545`).
    /// The watcher converts this to an HTTP URL for JSON-RPC polling.
    pub rpc_ws_url: String,
    /// Chain ID used to name the topic (`chain:{chain_id}`).
    pub chain_id: String,
}

/// Start the chain watcher as a background task.
///
/// Subscribes to `eth_blockNumber` every 2 seconds and publishes each new
/// block header as a [`TopicEnvelope`] on `chain:{chain_id}`.
///
/// The task exits cleanly when `cancel` is triggered.
pub async fn start_chain_watcher(
    config: ChainWatcherConfig,
    state: Arc<RelayState>,
    cancel: CancellationToken,
) {
    let topic = format!("chain:{}", config.chain_id);
    info!(
        rpc_url = %config.rpc_ws_url,
        %topic,
        "chain watcher starting",
    );

    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
    let mut last_block: u64 = 0;

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!("chain watcher shutting down");
                break;
            }
            _ = interval.tick() => {
                match poll_block_number(&config.rpc_ws_url).await {
                    Ok(block_num) if block_num > last_block => {
                        let envelope = TopicEnvelope::new(
                            &topic,
                            "new_block",
                            serde_json::json!({
                                "block_number": block_num,
                                "chain_id": &config.chain_id,
                            }),
                        )
                        .with_publisher("chain-watcher");

                        let (seq, subscribers) = state.bus.publish(envelope.clone());

                        // Fan out to all topic subscribers.
                        for sub_id in &subscribers {
                            let frame = RelayOutboundFrame::TopicMessage {
                                topic: envelope.topic.clone(),
                                msg_type: envelope.msg_type.clone(),
                                payload: envelope.payload.clone(),
                                publisher_id: envelope.publisher_id.clone(),
                                seq,
                            };
                            state.send_to_agent(sub_id, frame);
                        }

                        info!(
                            block_number = block_num,
                            subscribers = subscribers.len(),
                            %topic,
                            "published new_block",
                        );
                        last_block = block_num;
                    }
                    Ok(_) => {} // No new block yet.
                    Err(e) => {
                        warn!(error = %e, rpc_url = %config.rpc_ws_url, "failed to poll block number");
                    }
                }
            }
        }
    }
}

/// Poll `eth_blockNumber` via HTTP JSON-RPC.
///
/// Converts the `ws://` / `wss://` URL to `http://` / `https://` so we can
/// use a plain POST request without an open WebSocket connection.
async fn poll_block_number(rpc_url: &str) -> anyhow::Result<u64> {
    let http_url = rpc_url
        .replace("ws://", "http://")
        .replace("wss://", "https://");

    let client = reqwest::Client::new();
    let resp = client
        .post(&http_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_blockNumber",
            "params": [],
            "id": 1,
        }))
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    let hex = resp["result"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no 'result' field in eth_blockNumber response"))?;

    let num = u64::from_str_radix(hex.trim_start_matches("0x"), 16)
        .map_err(|e| anyhow::anyhow!("invalid block number hex '{hex}': {e}"))?;

    Ok(num)
}
