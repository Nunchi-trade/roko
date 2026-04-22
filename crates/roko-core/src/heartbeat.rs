//! Heartbeat protocol types shared between `roko-serve` and clients.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Default interval in seconds between heartbeats.
pub const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// Maximum number of heartbeats retained in the ring buffer.
pub const HEARTBEAT_RING_CAPACITY: usize = 500;

/// Payload sent by a heartbeat emitter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatPayload {
    pub sender_id: String,
    pub timestamp: String,
    #[serde(default)]
    pub active_tasks: usize,
    #[serde(default)]
    pub completed_tasks: usize,
    #[serde(default)]
    pub failed_tasks: usize,
    #[serde(default)]
    pub active_agents: usize,
    #[serde(default)]
    pub frequency: f64,
    #[serde(default)]
    pub metrics: HashMap<String, f64>,
}

/// Per-sender aggregated network statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkStats {
    pub sender_id: String,
    pub heartbeat_count: usize,
    pub last_seen: String,
    pub avg_active_tasks: f64,
}

/// Information about a known heartbeat sender.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SenderInfo {
    pub sender_id: String,
    pub first_seen: String,
    pub last_seen: String,
    pub total_heartbeats: usize,
}
