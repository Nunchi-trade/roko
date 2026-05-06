//! Feed agent framework: 29 background agents that publish structured data
//! feeds to the relay topic bus and local event bus.
//!
//! Each agent implements [`FeedAgent`] and is spawned by [`spawn_all`] during
//! server startup. Agents publish [`ServerEvent::FeedTick`] events that the
//! SSE bridge streams to the demo-app Feeds dashboard.

mod chain_watcher;
mod defi;
mod derivatives;
mod epoch_tracker;
mod gas_oracle;
mod keeper;
mod market;
mod monitors;
mod onchain;
mod oracle_submitter;
mod source_scouts;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::events::ServerEvent;
use crate::state::AppState;
use crate::state::{FeedCatalogAgent, FeedCatalogEntry};

// ---------------------------------------------------------------------------
// FeedDescriptor — compatible with the relay protocol type
// ---------------------------------------------------------------------------

/// Describes a data feed that an agent publishes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedDescriptor {
    pub feed_id: String,
    pub topic: String,
    pub name: String,
    pub description: String,
    /// One of: `"raw"`, `"derived"`, `"composite"`, `"meta"`.
    pub kind: String,
    /// Human-readable publish rate, e.g. `"10s"`, `"per-block"`.
    pub rate: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// FeedAgent trait
// ---------------------------------------------------------------------------

/// A background agent that publishes structured data feeds.
pub trait FeedAgent: Send + Sync + 'static {
    /// Unique agent identifier (e.g. `"isfr-keeper"`).
    fn agent_id(&self) -> &'static str;
    /// Human-readable display name (e.g. `"ISFR Composite Keeper"`).
    fn display_name(&self) -> &'static str;
    /// Agent capability tags.
    fn capabilities(&self) -> Vec<&str>;
    /// Feed descriptors for feeds this agent publishes.
    fn feeds(&self) -> Vec<FeedDescriptor>;
    /// Run the agent's main loop. Returns when cancelled.
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;
}

/// Shared context passed to each feed agent.
pub struct FeedAgentContext {
    pub state: Arc<AppState>,
    pub cancel: CancellationToken,
}

impl FeedAgentContext {
    /// Publish a feed tick to the local event bus.
    pub fn publish_tick(
        &self,
        agent_id: &str,
        feed_id: &str,
        topic: &str,
        payload: serde_json::Value,
    ) {
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        self.state.event_bus.publish(ServerEvent::FeedTick {
            agent_id: agent_id.to_string(),
            feed_id: feed_id.to_string(),
            topic: topic.to_string(),
            payload,
            timestamp_ms,
        });
    }
}

// ---------------------------------------------------------------------------
// spawn_all — create and run all 29 feed agents
// ---------------------------------------------------------------------------

/// Spawn all feed agents as background tokio tasks.
///
/// Returns join handles for each agent. Agents respect the server's cancel
/// token for graceful shutdown.
pub fn spawn_all(state: Arc<AppState>) -> Vec<JoinHandle<()>> {
    // Check if feed agents are enabled in config.
    let roko_config = state.load_roko_config();
    if !roko_config.feed_agents_enabled() {
        tracing::debug!("feed_agents disabled in config, skipping");
        return Vec::new();
    }

    let cancel = CancellationToken::new();

    // Bridge the server's CancelToken to a tokio-util CancellationToken.
    {
        let bridge_cancel = cancel.clone();
        let bridge_state = Arc::clone(&state);
        tokio::spawn(async move {
            bridge_state.cancel.cancelled().await;
            bridge_cancel.cancel();
        });
    }

    let agents: Vec<Arc<dyn FeedAgent>> = vec![
        // Original 15 agents
        Arc::new(keeper::IsfrKeeperAgent),
        Arc::new(source_scouts::AaveScoutAgent),
        Arc::new(source_scouts::CompoundScoutAgent),
        Arc::new(source_scouts::EthenaScoutAgent),
        Arc::new(source_scouts::LidoScoutAgent),
        Arc::new(chain_watcher::ChainWatcherAgent),
        Arc::new(gas_oracle::GasOracleAgent),
        Arc::new(derivatives::RateDerivativeAgent),
        Arc::new(derivatives::SpreadMonitorAgent),
        Arc::new(epoch_tracker::EpochTrackerAgent),
        Arc::new(oracle_submitter::OracleSubmitterAgent),
        Arc::new(monitors::AgentMonitorAgent),
        Arc::new(monitors::ConfidenceScorerAgent),
        Arc::new(derivatives::VolatilityWatcherAgent),
        Arc::new(monitors::RelayStatsAgent),
        // On-chain analytics (5)
        Arc::new(onchain::BlockSpaceAgent),
        Arc::new(onchain::TxThroughputAgent),
        Arc::new(onchain::FeeBurnAgent),
        Arc::new(onchain::NetworkHealthAgent),
        Arc::new(onchain::ContractActivityAgent),
        // DeFi analytics (5)
        Arc::new(defi::YieldCurveAgent),
        Arc::new(defi::LiquidationRiskAgent),
        Arc::new(defi::TvlTrackerAgent),
        Arc::new(defi::StablecoinPegAgent),
        Arc::new(defi::MevTrackerAgent),
        // Market analytics (5)
        Arc::new(market::CorrelationAgent),
        Arc::new(market::RegimeClassifierAgent),
        Arc::new(market::RiskAdjustedAgent),
        Arc::new(market::SystemHeartbeatAgent),
    ];

    // Build catalog snapshot for the /api/feeds/catalog endpoint.
    let catalog_agents: Vec<FeedCatalogAgent> = agents
        .iter()
        .map(|a| FeedCatalogAgent {
            agent_id: a.agent_id().to_string(),
            name: a.display_name().to_string(),
            capabilities: a.capabilities().iter().map(|s| s.to_string()).collect(),
            feed_count: a.feeds().len(),
            online: true,
        })
        .collect();

    let catalog_feeds: Vec<FeedCatalogEntry> = agents
        .iter()
        .flat_map(|a| {
            let aid = a.agent_id().to_string();
            a.feeds().into_iter().map(move |f| FeedCatalogEntry {
                feed_id: f.feed_id,
                topic: f.topic,
                name: f.name,
                description: f.description,
                kind: f.kind,
                rate: f.rate,
                agent_id: aid.clone(),
            })
        })
        .collect();

    // Write catalog to state (best-effort, don't block startup).
    {
        let state_cat = Arc::clone(&state);
        let agents_clone = catalog_agents.clone();
        let feeds_clone = catalog_feeds.clone();
        tokio::spawn(async move {
            let mut cat = state_cat.feed_agent_catalog.write().await;
            cat.agents = agents_clone;
            cat.feeds = feeds_clone;
        });
    }

    // Emit FeedAgentOnline events.
    for agent in &agents {
        state.event_bus.publish(ServerEvent::FeedAgentOnline {
            agent_id: agent.agent_id().to_string(),
            name: agent.display_name().to_string(),
            feed_count: agent.feeds().len(),
        });
    }

    let total = agents.len();
    tracing::info!(count = total, "spawning feed agents");

    let mut handles = Vec::with_capacity(agents.len());
    for agent in agents {
        let ctx = FeedAgentContext {
            state: Arc::clone(&state),
            cancel: cancel.clone(),
        };
        let agent_id = agent.agent_id().to_string();
        handles.push(tokio::spawn(async move {
            agent.run(ctx).await;
            tracing::debug!(agent_id, "feed agent exited");
        }));
    }

    handles
}
