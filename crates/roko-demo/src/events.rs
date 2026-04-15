//! Demo event streaming primitives.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::ws_server;

/// One node in the emitted knowledge graph.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnowledgeNode {
    /// Stable graph node id.
    pub id: String,
    /// Human-readable content label.
    pub content: String,
    /// Poster wallet or address.
    pub poster: String,
    /// Current pheromone weight from the board.
    pub pheromone_weight: u64,
    /// Number of confirmations represented in the graph.
    pub confirmations: u64,
}

/// One edge in the emitted knowledge graph.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnowledgeEdge {
    /// Source node or actor id.
    pub from: String,
    /// Destination node id.
    pub to: String,
    /// Relationship kind: posted, confirmed, or queried.
    pub kind: String,
}

/// Structured event emitted by demo scenarios.
#[allow(missing_docs)]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DemoEvent {
    ScenarioStarted { scenario: String },
    RoundStarted { scenario: String, round: u32 },
    RoundCompleted { scenario: String, round: u32, winner: String, output_eth: f64 },
    ScenarioCompleted { scenario: String, rounds: u32, improvement_bps: u32 },
    JobPosted { round: u32, job_id: String, bounty_wei: String, spec: String },
    JobAssigned { round: u32, job_id: String, worker: String, model: String },
    AgentBid { round: u32, worker: String, model: String, expected_output_eth: f64, confidence: f64 },
    ExecutionStarted { round: u32, worker: String, route_steps: usize },
    ExecutionCompleted { round: u32, worker: String, actual_output_eth: f64 },
    ValidationVote { round: u32, validator: String, approve: bool },
    ValidationComplete { round: u32, accepted: bool, validators: Vec<String> },
    FeesDistributed {
        round: u32,
        job_id: String,
        amount_wei: String,
        validator_share_wei: String,
        data_share_wei: String,
        agent_share_wei: String,
        treasury_share_wei: String,
    },
    InsightPosted { round: u32, insight_id: String, poster: String, uri: String },
    InsightConfirmed { round: u32, insight_id: String, confirmer: String, pheromone: u64 },
    KnowledgeQueried { round: u32, worker: String, insights_available: usize },
    CFactorMeasured { round_1_output_eth: f64, round_2_output_eth: f64, improvement_bps: u32 },
    ReputationUpdated { worker: String, reputation: String },
    AgentSlashed {
        worker: String,
        reason_code: u8,
        amount_wei: String,
        new_bond_wei: String,
        new_reputation: String,
    },
    KnowledgeGraphUpdate {
        round: u32,
        nodes: Vec<KnowledgeNode>,
        edges: Vec<KnowledgeEdge>,
    },
    Error { message: String },
}

/// Sink for demo events.
#[async_trait]
pub trait EventEmitter: Send + Sync {
    /// Emit one event.
    async fn emit(&self, event: DemoEvent);
}

/// No-op emitter.
pub struct NullEmitter;

/// Writes newline-delimited JSON to stdout.
pub struct NdjsonEmitter;

/// Broadcasts JSON over WebSocket clients.
pub struct WsEmitter {
    broadcaster: tokio::sync::broadcast::Sender<String>,
}

/// Fan-out over multiple emitters.
pub struct CompositeEmitter {
    emitters: Vec<Arc<dyn EventEmitter>>,
}

/// Pushes cloned events into an mpsc channel for local consumers such as the TUI.
pub struct ChannelEmitter {
    sender: mpsc::Sender<DemoEvent>,
}

impl CompositeEmitter {
    /// Construct a composite emitter.
    pub fn new(emitters: Vec<Arc<dyn EventEmitter>>) -> Self {
        Self { emitters }
    }
}

impl ChannelEmitter {
    /// Construct a channel-backed emitter.
    pub fn new(sender: mpsc::Sender<DemoEvent>) -> Self {
        Self { sender }
    }
}

/// Factory for requested event mode.
pub async fn create_emitter(mode: &str, ws_port: u16) -> anyhow::Result<Arc<dyn EventEmitter>> {
    match mode {
        "none" => Ok(Arc::new(NullEmitter)),
        "ndjson" => Ok(Arc::new(NdjsonEmitter)),
        "ws" => Ok(Arc::new(WsEmitter {
            broadcaster: ws_server::start_ws_server(ws_port).await?,
        })),
        "both" => {
            let ws = Arc::new(WsEmitter {
                broadcaster: ws_server::start_ws_server(ws_port).await?,
            });
            let ndjson = Arc::new(NdjsonEmitter);
            Ok(Arc::new(CompositeEmitter::new(vec![ndjson, ws])))
        }
        other => Err(anyhow::anyhow!("unknown events mode: {other}")),
    }
}

#[async_trait]
impl EventEmitter for NullEmitter {
    async fn emit(&self, _event: DemoEvent) {}
}

#[async_trait]
impl EventEmitter for NdjsonEmitter {
    async fn emit(&self, event: DemoEvent) {
        match serde_json::to_string(&event) {
            Ok(line) => println!("{line}"),
            Err(error) => tracing::warn!("event serialization failed: {error}"),
        }
    }
}

#[async_trait]
impl EventEmitter for WsEmitter {
    async fn emit(&self, event: DemoEvent) {
        match serde_json::to_string(&event) {
            Ok(line) => {
                let _ = self.broadcaster.send(line);
            }
            Err(error) => tracing::warn!("event serialization failed: {error}"),
        }
    }
}

#[async_trait]
impl EventEmitter for CompositeEmitter {
    async fn emit(&self, event: DemoEvent) {
        for emitter in &self.emitters {
            emitter.emit(event.clone()).await;
        }
    }
}

#[async_trait]
impl EventEmitter for ChannelEmitter {
    async fn emit(&self, event: DemoEvent) {
        let _ = self.sender.send(event).await;
    }
}
