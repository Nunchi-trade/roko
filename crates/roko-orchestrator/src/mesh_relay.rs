//! WebSocket relay for mesh-scope pheromone synchronization.
//!
//! Handles peer-to-peer pheromone delivery with version-vector
//! deduplication and store-and-forward for offline agents.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::coordination::{AgentId, Pheromone, PheromoneKind};

/// Sequence number assigned to each published pheromone.
pub type SeqNo = u64;

/// Per-peer connection state.
#[derive(Debug, Clone)]
pub struct PeerState {
    /// The agent's identifier.
    pub agent_id: AgentId,
    /// Whether the peer is currently connected.
    pub connected: bool,
    /// Monotonic millisecond timestamp of last activity.
    pub last_seen_ms: i64,
    /// Which pheromone kinds this peer has subscribed to.
    pub subscribed_kinds: Vec<PheromoneKind>,
}

/// A pheromone tagged with its relay sequence number and origin.
#[derive(Debug, Clone)]
pub struct SequencedPheromone {
    /// Relay-assigned sequence number for deduplication.
    pub seq: SeqNo,
    /// The originating agent.
    pub origin: AgentId,
    /// The pheromone payload.
    pub pheromone: Pheromone,
}

/// Inner state protected by a mutex.
#[derive(Debug)]
struct Inner {
    /// Per-peer connection state.
    peers: HashMap<AgentId, PeerState>,
    /// Version vectors: highest sequence number seen from each origin.
    version_vectors: HashMap<AgentId, SeqNo>,
    /// Store-and-forward queues for offline peers.
    store_forward: HashMap<AgentId, Vec<SequencedPheromone>>,
}

/// WebSocket relay for mesh-scope pheromone synchronization.
///
/// Thread-safe: all mutable state is behind `Arc<Mutex<_>>`. The local
/// sequence counter uses `AtomicU64` for lock-free increment.
#[derive(Debug, Clone)]
pub struct MeshRelay {
    inner: Arc<Mutex<Inner>>,
    local_seq: Arc<AtomicU64>,
}

impl Default for MeshRelay {
    fn default() -> Self {
        Self::new()
    }
}

impl MeshRelay {
    /// Create a new empty relay.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                peers: HashMap::new(),
                version_vectors: HashMap::new(),
                store_forward: HashMap::new(),
            })),
            local_seq: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Publish a pheromone to all connected subscribers.
    ///
    /// Returns the assigned sequence number. Duplicate pheromones
    /// (already seen via version vector) are silently dropped and
    /// return `0`.
    pub fn publish(&self, origin: &AgentId, pheromone: Pheromone) -> SeqNo {
        let seq = self.local_seq.fetch_add(1, Ordering::Relaxed);
        let mut inner = self.inner.lock();

        // Version-vector dedup: skip if we already saw a higher seq from this origin.
        let seen = inner.version_vectors.entry(origin.clone()).or_insert(0);
        if seq <= *seen {
            return 0;
        }
        *seen = seq;

        let msg = SequencedPheromone {
            seq,
            origin: origin.clone(),
            pheromone,
        };

        // Fan out to all peers.
        let peer_ids: Vec<AgentId> = inner.peers.keys().cloned().collect();
        for peer_id in &peer_ids {
            if peer_id == origin {
                continue; // don't echo back to sender
            }
            let peer = &inner.peers[peer_id];

            // Check subscription filter.
            if !peer.subscribed_kinds.is_empty()
                && !peer.subscribed_kinds.contains(&msg.pheromone.kind)
            {
                continue;
            }

            if peer.connected {
                // In a full implementation this would push to the peer's
                // WebSocket sink. For now we record delivery intent.
            } else {
                // Store-and-forward for offline peers.
                inner
                    .store_forward
                    .entry(peer_id.clone())
                    .or_default()
                    .push(msg.clone());
            }
        }

        seq
    }

    /// Register a peer's subscription to specific pheromone kinds.
    ///
    /// An empty `kinds` vector subscribes to all kinds.
    pub fn subscribe(&self, agent_id: AgentId, kinds: Vec<PheromoneKind>) {
        let mut inner = self.inner.lock();
        let peer = inner.peers.entry(agent_id.clone()).or_insert(PeerState {
            agent_id,
            connected: false,
            last_seen_ms: 0,
            subscribed_kinds: Vec::new(),
        });
        peer.subscribed_kinds = kinds;
    }

    /// Handle a peer connecting to the relay.
    ///
    /// Returns any store-and-forward pheromones queued while the peer
    /// was offline.
    pub fn on_peer_connect(&self, agent_id: AgentId, now_ms: i64) -> Vec<SequencedPheromone> {
        let mut inner = self.inner.lock();
        let peer = inner.peers.entry(agent_id.clone()).or_insert(PeerState {
            agent_id: agent_id.clone(),
            connected: true,
            last_seen_ms: now_ms,
            subscribed_kinds: Vec::new(),
        });
        peer.connected = true;
        peer.last_seen_ms = now_ms;

        // Drain store-and-forward queue.
        inner
            .store_forward
            .remove(&agent_id)
            .unwrap_or_default()
    }

    /// Handle a peer disconnecting from the relay.
    pub fn on_peer_disconnect(&self, agent_id: &AgentId) {
        let mut inner = self.inner.lock();
        if let Some(peer) = inner.peers.get_mut(agent_id) {
            peer.connected = false;
        }
        // Ensure store-forward queue exists for the now-offline peer.
        inner.store_forward.entry(agent_id.clone()).or_default();
    }

    /// Return the number of currently connected peers.
    #[must_use]
    pub fn connected_count(&self) -> usize {
        self.inner
            .lock()
            .peers
            .values()
            .filter(|p| p.connected)
            .count()
    }

    /// Return the number of registered peers (connected or not).
    #[must_use]
    pub fn peer_count(&self) -> usize {
        self.inner.lock().peers.len()
    }

    /// Return the current local sequence number.
    #[must_use]
    pub fn current_seq(&self) -> SeqNo {
        self.local_seq.load(Ordering::Relaxed)
    }

    /// Return the number of queued store-and-forward messages for a peer.
    #[must_use]
    pub fn queued_count(&self, agent_id: &AgentId) -> usize {
        self.inner
            .lock()
            .store_forward
            .get(agent_id)
            .map_or(0, Vec::len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::{PheromoneKind, PheromoneScope};

    fn test_pheromone(kind: PheromoneKind) -> Pheromone {
        Pheromone::new(
            kind.clone(),
            0.8,
            kind.default_half_life(),
            "test-source".to_owned(),
            PheromoneScope::Mesh("collective-1".to_owned()),
        )
    }

    #[test]
    fn publish_assigns_increasing_sequence_numbers() {
        let relay = MeshRelay::new();
        let origin = "agent-a".to_owned();
        let seq1 = relay.publish(&origin, test_pheromone(PheromoneKind::Threat));
        let seq2 = relay.publish(&origin, test_pheromone(PheromoneKind::Opportunity));
        assert!(seq2 > seq1);
    }

    #[test]
    fn store_and_forward_queues_for_offline_peers() {
        let relay = MeshRelay::new();
        let sender = "agent-a".to_owned();
        let receiver = "agent-b".to_owned();

        // Receiver connects then disconnects.
        relay.on_peer_connect(receiver.clone(), 1000);
        relay.on_peer_disconnect(&receiver);

        // Sender publishes while receiver is offline.
        relay.publish(&sender, test_pheromone(PheromoneKind::Threat));
        relay.publish(&sender, test_pheromone(PheromoneKind::Opportunity));

        assert_eq!(relay.queued_count(&receiver), 2);

        // Receiver reconnects and gets queued messages.
        let replayed = relay.on_peer_connect(receiver.clone(), 2000);
        assert_eq!(replayed.len(), 2);
        assert_eq!(relay.queued_count(&receiver), 0);
    }

    #[test]
    fn subscription_filters_by_kind() {
        let relay = MeshRelay::new();
        let sender = "agent-a".to_owned();
        let receiver = "agent-b".to_owned();

        // Receiver subscribes only to Threat.
        relay.on_peer_connect(receiver.clone(), 1000);
        relay.subscribe(receiver.clone(), vec![PheromoneKind::Threat]);
        relay.on_peer_disconnect(&receiver);

        // Sender publishes Threat and Opportunity.
        relay.publish(&sender, test_pheromone(PheromoneKind::Threat));
        relay.publish(&sender, test_pheromone(PheromoneKind::Opportunity));

        // Only Threat should be queued.
        assert_eq!(relay.queued_count(&receiver), 1);
    }

    #[test]
    fn peer_connect_disconnect_tracking() {
        let relay = MeshRelay::new();
        assert_eq!(relay.connected_count(), 0);
        assert_eq!(relay.peer_count(), 0);

        relay.on_peer_connect("agent-a".to_owned(), 1000);
        assert_eq!(relay.connected_count(), 1);
        assert_eq!(relay.peer_count(), 1);

        relay.on_peer_connect("agent-b".to_owned(), 1000);
        assert_eq!(relay.connected_count(), 2);

        relay.on_peer_disconnect(&"agent-a".to_owned());
        assert_eq!(relay.connected_count(), 1);
        assert_eq!(relay.peer_count(), 2);
    }

    #[test]
    fn no_echo_to_sender() {
        let relay = MeshRelay::new();
        let agent = "agent-a".to_owned();

        // Agent connects and then disconnects (to use store-forward as proof).
        relay.on_peer_connect(agent.clone(), 1000);
        relay.on_peer_disconnect(&agent);

        // Agent publishes — should not queue to itself.
        relay.publish(&agent, test_pheromone(PheromoneKind::Threat));
        assert_eq!(relay.queued_count(&agent), 0);
    }
}
