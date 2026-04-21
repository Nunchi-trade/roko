# Agent Mesh Sync: Transport Layer for Pheromone Propagation

> **Layer**: L0 Runtime (connection lifecycle, event bus), L1 Framework (transport backends,
> protocol definition)
>
> **Synapse traits**: `Bus` (MeshBus for Pulses), `Substrate` (MeshSubstrate as a distributed
> store), `Policy` (reactive behavior on received messages)
>
> **Prerequisites**: `03-digital-pheromones.md` (what gets transported),
> `05-pheromone-scope.md` (Mesh scope definition)

> **See also**: `../../tmp/refinements/09-phase-2-implications.md`,
> `../00-architecture/01-naming-and-glossary.md`


> **Implementation**: Specified

---

## Overview

The Agent Mesh is Roko's peer-to-peer connectivity layer for Mesh-scope pheromone
propagation, collective knowledge sharing, and morphogenetic coordination signals. In the
two-fabric model it is not a separate trait family: MeshBus carries Pulses and MeshSubstrate
replicates Engrams. It provides transport between agents in a Collective without requiring a
centralized relay broker.

The Agent Mesh supports three co-equal transport mechanisms:

| Transport | Technology | Latency | NAT Traversal | Encryption | Best For |
|-----------|-----------|---------|---------------|------------|----------|
| **WebSocket** | Standard WSS | ~50ms | N/A (outbound only) | TLS 1.3 | Always-available relay, store-and-forward |
| **Iroh** | QUIC + ed25519 | ~10ms LAN, ~100ms WAN | Hole-punching + relay fallback | QUIC TLS 1.3 | Direct P2P, gossip pub/sub, blob transfer |
| **ERC-8004** | On-chain registry | ~minutes | N/A | N/A | Agent discovery, service endpoint resolution |

These transports are complementary, not competing. The recommended configuration uses all three:
Iroh for direct agent-to-agent communication, WebSocket as a fallback relay with store-and-
forward for offline agents, and ERC-8004 for discovery and identity binding.

---

## The Dual-Transport Architecture

### Four Valid Transport Combinations

| WebSocket | Iroh | Behavior |
|-----------|------|----------|
| off | off | Local-only agent. No Mesh sync, no pheromones, no collective coordination. ~95% core capability via local NeuroStore. Valid for development, testing, or isolated operation. |
| on | off | Classic relay model. All 16 services multiplexed over a single outbound WebSocket. Store-and-forward for offline agents (7-day TTL). Simple, reliable. |
| off | on | Pure P2P. Collective sync via direct QUIC streams, pheromones via gossip, knowledge via content-addressed blobs. No store-and-forward for offline agents. |
| **on** | **on** | **Best of both (recommended).** Iroh for direct peer communication (lower latency, no relay hop). WebSocket for store-and-forward, persistence, and services that require a server. Dedup via version vectors when same message arrives via both. |

### EventFabric as Integration Point

Regardless of transport, all messages converge on the Bus projection layer. A pheromone
arriving via WebSocket and the same pheromone arriving via Iroh produce identical Pulses on
topics such as `mesh.pheromone.deposited`. Durable Engrams land in MeshSubstrate; consumers
(NeuroStore ingestion, context assembly, morphogenetic coordinator) never need to know which
transport delivered the signal.

Deduplication happens at the ingestion boundary via version vectors (`{agent_id → last_seen_seq}`). If a pheromone arrives via both WebSocket and Iroh, the second arrival is silently dropped.

---

## WebSocket Transport

### The Outbound WebSocket Model

Every agent that enables Agent Mesh maintains ONE persistent outbound WebSocket connection.
This connection works from behind any NAT, firewall, or restrictive network — it is a standard
outbound HTTPS connection on port 443. No inbound ports, no tunnels, no port forwarding.

The WebSocket connection serves as a bidirectional channel for ALL Mesh traffic: collective
sync, pheromone updates, knowledge exchange, Bus relay, and TUI forwarding. In the
two-fabric model it is one backend for MeshBus Pulses, while MeshSubstrate handles durable
Engram replication. This multiplexed design minimizes connection overhead while supporting the
full range of Mesh services.

### Connection Registry

The Agent Mesh server maintains an in-memory registry of connected agents, indexed by Collective:

```rust
/// In-memory registry of connected agents.
/// When Agent A pushes a delta, the Mesh server looks up all peers
/// (same Collective, different agent) and pushes to their WebSocket connections.
pub struct ConnectionRegistry {
    /// Map from CollectiveId to list of connected agents.
    connections: DashMap<CollectiveId, Vec<ConnectedAgent>>,
}

impl ConnectionRegistry {
    /// Route a pheromone delta to all Collective members except the sender.
    pub async fn route_pheromone(
        &self,
        sender_id: &AgentId,
        collective_id: &CollectiveId,
        delta: PheromoneDelta,
    ) {
        if let Some(peers) = self.connections.get(collective_id) {
            for peer in peers.iter() {
                if peer.agent_id != *sender_id {
                    let _ = peer.tx.send(MeshMessage::PheromoneSync {
                        from: sender_id.clone(),
                        entries: delta.entries.clone(),
                    }).await;
                }
            }
        }
    }
}
```

### Store-and-Forward

If an agent is offline when a pheromone delta is pushed, the Mesh server stores it in its
persistence layer. When the agent reconnects, it receives all pending deltas in order. Pending
deltas expire after a configurable TTL (default: 7 days).

This is critical for asynchronous coordination: an agent that runs overnight should find all
pheromone signals deposited by daytime agents when it boots in the morning.

### WebSocket message types

| Message Type | Direction | Content |
|-------------|-----------|---------|
| `PheromoneSync` | Server -> Agent | Batch of pheromone Engrams from Collective peers |
| `PheromoneImmediate` | Server -> Agent | High-priority pheromone (Threat, high-intensity Anomaly) |
| `PheromoneDelta` | Agent -> Server | New pheromone deposits for relay to Collective and Bus publication |
| `KnowledgeSync` | Bidirectional | NeuroStore entries promoted to Mesh scope |
| `MorphogeneticBroadcast` | Agent -> Server -> Peers | Role vector and specialization signals |
| `VersionVector` | Agent -> Server | Sequence numbers for dedup and delta computation |
| `Heartbeat` | Agent -> Server | Liveness signal (every 30s) |

### Message ordering and priority

WebSocket messages are ordered by a two-tier priority queue at the sender. High-priority
messages (Threat pheromones, role conflict alerts) preempt batched sync messages.

```rust
/// Priority levels for outbound WebSocket messages.
/// Lower numeric value = higher priority = sent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum MeshPriority {
    /// Threat pheromones, role conflict alerts, niche vacancy alerts.
    /// Sent immediately, bypassing the batch interval.
    Critical = 0,
    /// PheromoneImmediate (intensity > immediate_threshold).
    High = 1,
    /// Standard batched sync: PheromoneSync, KnowledgeSync, MorphogeneticBroadcast.
    Normal = 2,
    /// Heartbeat, VersionVector exchanges, telemetry.
    Low = 3,
}

/// Outbound message queue with priority ordering.
///
/// Uses a `BinaryHeap` keyed on `(priority, sequence)` to maintain
/// FIFO order within the same priority level.
///
/// # Flow control
///
/// When the queue exceeds `max_pending` messages, the oldest `Low`
/// priority messages are dropped first, then `Normal`. `Critical` and
/// `High` messages are never dropped — if the queue is full of critical
/// messages, backpressure propagates to the caller via `try_send`.
pub struct PriorityOutbox {
    queue: BinaryHeap<Reverse<(MeshPriority, u64, MeshMessage)>>,
    next_seq: u64,
    /// Maximum pending messages before flow control kicks in.
    /// Default: 1024. Range: [64, 65536].
    max_pending: usize,
}

impl PriorityOutbox {
    /// Enqueue a message. Returns `Err(QueueFull)` if the queue is at capacity
    /// and no lower-priority message can be evicted.
    pub fn enqueue(
        &mut self,
        priority: MeshPriority,
        msg: MeshMessage,
    ) -> Result<(), QueueFull> {
        if self.queue.len() >= self.max_pending {
            // Try to evict lowest-priority message
            if !self.evict_lowest(priority) {
                return Err(QueueFull);
            }
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.queue.push(Reverse((priority, seq, msg)));
        Ok(())
    }
}
```

**Ordering guarantees**: Messages within the same priority level are delivered in FIFO order
(sequence number tiebreak). Across priority levels, higher-priority messages are sent first.
The WebSocket transport itself is ordered (TCP), so server-side delivery order matches the
sender's priority queue order.

**Flow control on overflow**: When the outbox hits `max_pending`, the sender evicts the oldest
message at the lowest populated priority level. If only `Critical` messages remain, the
`enqueue` call returns `Err(QueueFull)` and the caller must decide whether to block or drop.
In practice, this only happens during sustained network partitions where the WebSocket write
buffer is full.

### Full message envelope schema

Every message on the wire (both WebSocket and Iroh) uses a common envelope:

```rust
/// Wire-format envelope for all Mesh messages.
///
/// Serialized with postcard (compact binary). The envelope wraps every
/// message type and provides the fields needed for routing, dedup,
/// authentication, and priority handling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshEnvelope {
    /// Protocol version. Current: 1. Receivers reject unknown versions.
    pub version: u8,

    /// Unique message ID (UUIDv7 for time-ordering).
    pub message_id: [u8; 16],

    /// Sender's agent ID.
    pub sender: AgentId,

    /// Sender's Collective ID.
    pub collective_id: CollectiveId,

    /// Monotonically increasing sequence number from this sender.
    /// Used by version vectors for dedup.
    pub seq: u64,

    /// Message priority (mapped from MeshPriority enum).
    pub priority: u8,

    /// Unix timestamp in milliseconds when the message was created.
    pub timestamp_ms: u64,

    /// Domain this message belongs to (for topic routing).
    /// Empty string means "all domains" (e.g., heartbeats).
    pub domain: String,

    /// The payload, tagged by type.
    pub payload: MeshPayload,

    /// Ed25519 signature over (version || message_id || sender || seq || payload_hash).
    /// Verified by receivers before processing.
    pub signature: [u8; 64],
}

/// Tagged union of all possible message payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MeshPayload {
    PheromoneDelta(Vec<PheromoneEntry>),
    PheromoneImmediate(PheromoneEntry),
    KnowledgeSync(Vec<KnowledgeEntry>),
    MorphogeneticBroadcast(MorphogeneticPheromone),
    NicheVacancy(NicheVacancy),
    RoleConflict(RoleConflict),
    VersionVector(HashMap<AgentId, u64>),
    Heartbeat { uptime_secs: u64, load: f32 },
}
```

---

## Iroh Transport

### What Is Iroh?

Iroh (from `n0-computer/iroh`) is a Rust library providing QUIC-based connections dialed by
public key, with NAT traversal and relay fallback. It offers three sub-protocols relevant to
Roko:

1. **Direct connections**: Dial any agent by its ed25519 public key (`NodeId`). Iroh handles
   address resolution, NAT hole-punching (~70% direct connection success rate), and relay
   fallback.
2. **iroh-gossip**: HyParView + PlumTree pub/sub for efficient multi-agent broadcast. Used for
   pheromone propagation across the Collective.
3. **iroh-blobs**: Content-addressed blob transfer using BLAKE3 hashing. Used for knowledge
   bundle exchange.

### Connection lifecycle

1. **Bind**: `Endpoint::builder()` with secret key, ALPN identifier (`roko/mesh/1`), and
   relay configuration. Spawns background tasks for relay connection and hole-punching.
2. **Publish**: Agent publishes its addressing information via pkarr (DNS-over-HTTPS) so peers
   can resolve its `NodeId` to a network address.
3. **Accept**: Incoming connections are screened by Collective membership -- only agents
   registered in the same Collective (verified via ERC-8004 registry or local config) are
   accepted.
4. **Connect**: Outbound connections dial peers by `NodeId`. Iroh resolves via pkarr/DNS,
   attempts direct connection, falls back to relay.
5. **Streams**: Bidirectional QUIC streams for collective sync. One long-lived stream per
   peer, pooled rather than per-message.
6. **Gossip**: Joins topic-based gossip channels for pheromone propagation.
7. **Shutdown**: Close connections, stop accept loop, unpublish from pkarr. 5-second graceful
   timeout.

### Automatic relay fallback

When direct QUIC hole-punching fails (~30% of connections, primarily symmetric NAT), Iroh
falls back to relay servers automatically. The fallback is transparent to application code.

```rust
/// Iroh relay configuration.
///
/// The relay serves two purposes:
/// 1. STUN-like address discovery (always used, even for direct connections)
/// 2. Traffic relay when direct connection fails
///
/// Cost model: relay traffic is metered. Each relayed byte counts against
/// the mesh budget. Direct connections have zero relay cost.
pub struct IrohRelayConfig {
    /// Relay mode selection.
    /// - "default": Use n0's public relay network (free tier: 1GB/month)
    /// - "custom": Use a self-hosted relay at `relay_url`
    /// - "disabled": No relay. Direct connections only (LAN-only operation)
    pub mode: RelayMode,

    /// URL of self-hosted relay server.
    /// Only used when mode = "custom".
    /// Example: "https://relay.my-mesh.example.com"
    pub relay_url: Option<String>,

    /// Maximum relay bandwidth per agent per day (bytes).
    /// Default: 100 MB (104_857_600). Range: [1_048_576, 10_737_418_240].
    /// Prevents runaway relay costs from sustained fallback.
    pub max_relay_bytes_per_day: u64,

    /// Connection timeout before declaring relay fallback.
    /// Default: 5 seconds. Range: [1, 30] seconds.
    pub direct_connect_timeout: Duration,
}
```

**Fallback sequence**: Iroh attempts direct connection first (UDP hole-punch via STUN). If no
response arrives within `direct_connect_timeout` (default: 5s), traffic routes through the
relay. The switch is per-connection, not global -- an agent can have direct connections to
some peers and relayed connections to others simultaneously.

**Cost model**: Relayed traffic counts against `max_relay_bytes_per_day`. When the limit is
reached, new relay connections are refused (direct-only or no connection). The daily counter
resets at midnight UTC. Pheromone messages are small (~200 bytes each), so 100 MB supports
roughly 500,000 pheromone exchanges per day through the relay -- well above normal operation.

### Pheromone Propagation via iroh-gossip

Each `(domain, regime)` pair gets a gossip topic. Topic IDs are deterministic, and the same
transport also carries Bus topics such as `mesh.pheromone.deposited`:

```
TopicId = blake3::hash(format!("roko/pheromone/{domain}/{regime}"))
```

The topic set is bounded: domains × regimes ≈ 50 topics maximum. Agents subscribe only to
topics for domains they operate in, preventing unnecessary pulse delivery. MeshBus reuses
the same topology to fan out Pulses while MeshSubstrate keeps the durable record.

`PheromoneDeposit` messages are serialized (postcard encoding) and broadcast via
`sender.broadcast()`. Decay is computed locally by each agent based on the deposit timestamp —
no coordination needed. The existing exponential decay formula applies unchanged. The
`mesh.pheromone.deposited` topic is the Bus-facing announcement for the same deposit.

When gossip receivers fall behind (`Event::Lagged`), the message is dropped and logged at WARN
level. Pheromone loss is acceptable -- signals are fuzzy by nature, and a lost pheromone will
likely be re-deposited or become irrelevant before its absence matters.

**Domain list management**: Each agent subscribes to gossip topics based on its configured
domains. The domain list is set at startup from `roko.toml` and can change at runtime when a
domain plugin is loaded or unloaded.

```rust
/// Manages gossip topic subscriptions for an agent.
///
/// Topic IDs are deterministic: `blake3(format!("roko/pheromone/{domain}/{regime}"))`.
/// When the agent's domain list changes, the manager subscribes to new topics
/// and unsubscribes from removed ones.
pub struct GossipTopicManager {
    /// Currently subscribed topics, keyed by (domain, regime).
    active_topics: HashMap<(String, String), TopicSubscription>,

    /// Domains this agent operates in. Set from config, updated at runtime.
    domains: Vec<String>,

    /// Known regimes per domain. Updated when regime-change pheromones arrive.
    regimes: HashMap<String, String>,
}

impl GossipTopicManager {
    /// Recompute subscriptions after a domain or regime change.
    ///
    /// Subscribes to new (domain, regime) pairs. Unsubscribes from
    /// pairs no longer in the active set. Existing subscriptions
    /// for unchanged pairs are left alone (no reconnect churn).
    pub fn reconcile(&mut self, endpoint: &Endpoint) -> Result<(), MeshError> {
        let desired: HashSet<(String, String)> = self.domains.iter()
            .flat_map(|d| {
                let regime = self.regimes.get(d).cloned()
                    .unwrap_or_else(|| "default".into());
                vec![(d.clone(), regime)]
            })
            .collect();

        // Unsubscribe from removed topics
        self.active_topics.retain(|key, sub| {
            if !desired.contains(key) {
                sub.leave();
                false
            } else {
                true
            }
        });

        // Subscribe to new topics
        for key in &desired {
            if !self.active_topics.contains_key(key) {
                let topic_id = Self::topic_id(&key.0, &key.1);
                let sub = endpoint.join_topic(topic_id)?;
                self.active_topics.insert(key.clone(), sub);
            }
        }
        Ok(())
    }

    fn topic_id(domain: &str, regime: &str) -> TopicId {
        let input = format!("roko/pheromone/{domain}/{regime}");
        TopicId::from(blake3::hash(input.as_bytes()))
    }
}
```

**Regime detection**: When the Collective detects a regime change (via regime-change pheromones
from `roko-conductor`), the topic manager creates a new topic for the new regime and begins
a 60-second overlap period where the agent subscribes to both old and new regime topics. After
the overlap, the old topic subscription is dropped. This prevents message loss during
transitions.

**Topic lifecycle**: Topics are created on first subscription and garbage-collected when no
agents remain subscribed. The underlying HyParView protocol handles peer membership -- when
the last subscriber leaves, the topic becomes dormant. Rejoining a dormant topic is
indistinguishable from joining a new one.

### Knowledge Exchange via iroh-blobs

For large knowledge artifacts (NeuroStore entries promoted to Mesh scope, knowledge backups),
iroh-blobs provides content-addressed transfer:

1. Sending agent adds the knowledge bundle to its local iroh-blobs store
2. The BLAKE3 hash is announced via gossip on a `roko/knowledge/{collective_id}` topic
3. Receiving agents download the blob on-demand using the hash
4. BLAKE3's BAO (Authenticated Ordering) provides incremental verification — the transfer
   is verified as it streams, not all-or-nothing

### Identity Binding

Each agent has a separate ed25519 keypair for Iroh, distinct from any wallet keys:

- **Generation**: On first boot, `iroh::SecretKey::generate()` creates a new keypair
- **Persistence**: Secret key stored at `{config.iroh.secret_key_path}` with 0o600 permissions
- **NodeId**: The public key is the agent's stable Iroh identity
- **Registration**: NodeId stored in ERC-8004 `serviceEndpoints` for discovery:
  ```json
  {
    "serviceEndpoints": {
      "ws": "wss://mesh.roko.dev/v1/mesh/ws",
      "iroh": "<base32-encoded-node-id>"
    }
  }
  ```

---

## ERC-8004: Agent Discovery

ERC-8004 (Agent Cards) provides on-chain service discovery for agents. Each registered agent
publishes an Agent Card containing:

| Field | Description |
|-------|-------------|
| `agentId` | On-chain agent identifier |
| `operator` | Operator address (for Collective discovery) |
| `capabilities` | Bitmask of agent capabilities |
| `serviceEndpoints` | WebSocket URL, Iroh NodeId, other transport addresses |
| `reputation` | Current reputation scores per domain |
| `domains` | Domains the agent operates in |

### Discovery Scopes

Three discovery scopes, from local to global:

#### Scope 1: Same LAN (Zero-Config)

Iroh's `MdnsAddressLookup` uses mDNS/DNS-SD to discover peers on the local network. No
configuration needed. When two agents run on the same machine or same LAN, they discover each
other automatically.

#### Scope 2: Same Collective (Cross-Network)

1. Query ERC-8004 registry: `getAgentsByOperator(operator_address)` → all agents for this
   operator
2. Extract Iroh `NodeId` from each agent's `serviceEndpoints`
3. Feed NodeIds into Iroh's address resolution chain
4. Connect directly or via relay

Discovery frequency: on boot, on new agent registration (event listener), and periodically
(default: every 300 seconds).

#### Scope 3: Cross-Collective (Strangers)

Cross-collective discovery requires an index:

1. **ERC-8004 browse**: Query the on-chain registry for agents filtered by capability,
   reputation, or domain. Public, permissionless directory.
2. **Semantic search**: Query the knowledge layer for semantic matches. Results include
   publisher NodeId. Knowledge exchange via Iroh after initial discovery.
3. **Gossip bootstrap**: Connect to well-reputed peers from ERC-8004, join domain-specific
   gossip topics, discover additional peers through HyParView peer sampling.

Cross-collective discovery is off by default (`config.discovery.cross_collective_enabled = false`).

### Caching strategy and stale data handling

ERC-8004 queries hit an on-chain registry, which is expensive (gas for writes, RPC latency for
reads). The discovery layer caches Agent Card data locally with a TTL-based invalidation
strategy.

```rust
/// Local cache for ERC-8004 Agent Card data.
///
/// Reduces on-chain queries to the poll interval (default: 300s).
/// Stale entries are usable but flagged — an agent whose card hasn't
/// been refreshed may have changed endpoints or gone offline.
pub struct AgentCardCache {
    /// Cached cards, keyed by agent ID.
    cards: HashMap<AgentId, CachedCard>,

    /// How long a cached card is considered fresh.
    /// Default: 300 seconds (matches poll interval).
    /// Range: [60, 3600] seconds.
    pub fresh_ttl: Duration,

    /// How long a stale card is kept before eviction.
    /// A stale card is still returned (with a `stale` flag) but triggers
    /// an async refresh. Default: 3600 seconds. Range: [300, 86400].
    pub stale_ttl: Duration,

    /// Maximum cached entries. LRU eviction beyond this limit.
    /// Default: 1000. Range: [100, 100_000].
    pub max_entries: usize,
}

struct CachedCard {
    card: AgentCard,
    fetched_at: Instant,
}

impl AgentCardCache {
    /// Look up an agent's card.
    ///
    /// Returns `Fresh(card)` if within fresh_ttl.
    /// Returns `Stale(card)` if within stale_ttl (triggers async refresh).
    /// Returns `Miss` if not cached or beyond stale_ttl.
    pub fn get(&self, agent_id: &AgentId) -> CacheLookup {
        match self.cards.get(agent_id) {
            Some(entry) => {
                let age = entry.fetched_at.elapsed();
                if age < self.fresh_ttl {
                    CacheLookup::Fresh(entry.card.clone())
                } else if age < self.stale_ttl {
                    CacheLookup::Stale(entry.card.clone())
                } else {
                    CacheLookup::Miss
                }
            }
            None => CacheLookup::Miss,
        }
    }
}
```

**Stale data handling**: When a stale card is returned, the caller proceeds with the cached
endpoints while an async refresh runs in the background. If the refresh reveals changed
endpoints (e.g., new Iroh NodeId), existing connections to the old endpoints are drained
gracefully (5-second timeout) before switching. If the refresh fails (RPC error), the stale
card remains in use until its `stale_ttl` expires.

**Event-driven invalidation**: In addition to polling, the discovery layer listens for
ERC-8004 `AgentUpdated` and `AgentRemoved` events via an Ethereum event subscription. These
events invalidate the cache entry immediately, avoiding stale data between poll intervals.

---

## Service-to-Transport Mapping

| Service | WebSocket | Iroh | Notes |
|---------|-----------|------|-------|
| Collective sync | Store-and-forward for offline agents | Direct QUIC streams | Iroh preferred, WS fallback |
| Pheromone propagation | Central fan-out from Mesh server | iroh-gossip per topic | MeshBus fans out Pulses; MeshSubstrate persists Engrams |
| Knowledge exchange | Relay | iroh-blobs (content-addressed) | Iroh preferred |
| Bloom oracle | Server-side cache | Gossip broadcast | Iroh for Collective, WS for ecosystem |
| Causal graph federation | Relay | Gossip | Iroh preferred |
| Event relay (→ TUI) | WS relay | Direct stream | Iroh if TUI is local |
| Persistence backup | WS to server | N/A (requires server) | **WS only** |
| Semantic search | WS to server | N/A (requires vector DB) | **WS only** |

---

## Failure Modes and Graceful Degradation

| Scenario | Behavior |
|----------|----------|
| Iroh endpoint fails to bind | Log warning, disable Iroh, operate WS-only |
| Iroh relay unreachable | Direct connections only; LAN peers still reachable via mDNS |
| Peer unreachable via Iroh | Route via WS for store-and-forward |
| WS relay unreachable | Operate Iroh-only; no persistence backup or semantic search |
| Both transports down | Local-only mode (~95% capability); local NeuroStore operates independently |
| Gossip `Event::Lagged` | Pheromone updates may be lost; acceptable (fuzzy signals) |
| Delta via both transports | Dedup via version vector; second arrival is no-op |

### Graceful Degradation Hierarchy

```
Both transports active    → full capability
Iroh only                 → ~85% (sync + pheromones work; no persistence/search)
WS only                   → ~95% (all services via relay; higher latency)
Neither                   → ~95% of core (local NeuroStore operates independently)
```

The ~95% local-only capability reflects a fundamental design principle: **the Agent Mesh is an
accelerator, not a requirement.** An agent with `mesh.enabled: false` operates on its local
NeuroStore, runs the full cognitive loop (query → score → route → compose → act → verify →
write → react), and produces useful work. What it loses is cross-agent intelligence — valuable
for coordination but not essential for individual agent operation.

---

## Partition Tolerance and Byzantine Resilience

Real-world mesh networks face network partitions, Byzantine agents, and asymmetric connectivity. Roko's Agent Mesh must maintain useful coordination even when the network is degraded, drawing on CAP theorem analysis [Brewer, E. "CAP Twelve Years Later: How the 'Rules' Have Changed." *Computer*, 45(2):23-29, IEEE, 2012] and Byzantine fault tolerance research.

### Network Partition Model

Network partitions split a Collective into isolated subgroups that cannot communicate. The Agent Mesh explicitly chooses **Availability + Partition tolerance (AP)** over Consistency, because:

1. Pheromone signals are inherently fuzzy — eventual consistency is sufficient
2. Agents must continue operating during partitions (availability)
3. Reconciliation after partition healing is straightforward (merge pheromone fields)

| Partition Scenario | Behavior | Recovery |
|-------------------|----------|----------|
| **Clean split** (two halves, no overlap) | Each partition runs independent pheromone fields; morphogenetic specialization re-converges within each partition | Merge: union of pheromone fields with dedup via version vectors; re-run morphogenetic update |
| **Asymmetric partition** (one large, one small) | Large partition operates normally; small partition has reduced collective intelligence | Small partition rejoins via delta sync; its local pheromones merge into the field |
| **Intermittent connectivity** | Messages arrive with high latency and reordering | Version vectors handle reordering; decay handles staleness; priority queue handles bursts |
| **Single-agent isolation** | Agent operates in local-only mode (~95% capability) | Reconnection triggers delta sync; agent receives missed pheromones and morphogenetic updates |

### Partition-Aware Morphogenetic Dynamics

During a network partition, the morphogenetic specialization system adjusts:

```rust
/// Adjustments to morphogenetic parameters during detected network partitions.
///
/// When an agent detects that it can no longer reach a subset of its
/// Collective, it adjusts the reaction-diffusion parameters to account
/// for the reduced information about the Collective's role structure.
pub struct PartitionAwareMorphogenetics {
    /// When the visible Collective size drops below this fraction of
    /// the known total, enter partition mode.
    /// Default: 0.6 (partition detected when <60% of peers visible).
    /// Range: [0.3, 0.9].
    pub partition_detection_threshold: f64,

    /// In partition mode, reduce inhibition rate (beta) by this factor.
    /// Rationale: inhibition is based on collective pheromone signals,
    /// which are incomplete during partition. Over-inhibiting based on
    /// partial data would push agents toward incorrect niches.
    /// Default: 0.5 (halve beta). Range: [0.2, 0.8].
    pub beta_reduction_factor: f64,

    /// In partition mode, increase noise (sigma) by this factor.
    /// Rationale: more exploration helps when the information landscape
    /// is incomplete.
    /// Default: 2.0 (double sigma). Range: [1.0, 5.0].
    pub sigma_amplification_factor: f64,

    /// Ticks to wait after full connectivity restores before exiting
    /// partition mode. Prevents oscillation if connectivity is flapping.
    /// Default: 200 ticks. Range: [50, 1000].
    pub recovery_cooldown_ticks: u64,
}

impl Default for PartitionAwareMorphogenetics {
    fn default() -> Self {
        Self {
            partition_detection_threshold: 0.6,
            beta_reduction_factor: 0.5,
            sigma_amplification_factor: 2.0,
            recovery_cooldown_ticks: 200,
        }
    }
}
```

### Post-Partition Reconciliation

When a partition heals, the two halves must merge their pheromone fields:

```rust
/// Reconcile two pheromone fields after a network partition heals.
///
/// Strategy: union merge with conflict resolution.
///
/// For pheromones that exist in both partitions:
/// - Take the one with higher effective intensity (accounting for decay)
/// - Merge confirmation sets (union of confirmers)
/// - Use the earlier deposit timestamp (preserves lineage)
///
/// For pheromones unique to one partition:
/// - Accept if intensity > evaporation threshold
/// - Apply trust discount (partition-sourced pheromones start at ×0.7)
///
/// # Parameters
/// - `partition_duration`: How long the partitions were separated
/// - `trust_discount`: Discount applied to cross-partition pheromones.
///   Default: 0.7. Range: [0.3, 1.0]. Longer partitions → lower trust.
pub struct PartitionReconciliation {
    /// Base trust discount for cross-partition pheromones.
    /// Default: 0.7 (30% discount). Range: [0.3, 1.0].
    pub base_trust_discount: f64,

    /// Additional discount per hour of partition duration.
    /// Default: 0.05 per hour. Range: [0.0, 0.2].
    pub per_hour_discount: f64,

    /// Maximum total discount (minimum trust).
    /// Default: 0.3 (70% discount cap). Range: [0.1, 0.5].
    pub max_discount: f64,

    /// Whether to re-trigger morphogenetic update immediately after merge.
    /// Default: true.
    pub retrigger_morphogenetic: bool,
}

impl Default for PartitionReconciliation {
    fn default() -> Self {
        Self {
            base_trust_discount: 0.7,
            per_hour_discount: 0.05,
            max_discount: 0.3,
            retrigger_morphogenetic: true,
        }
    }
}
```

### Byzantine Agent Detection

A Byzantine agent is one that deviates from the protocol — depositing false pheromones, amplifying noise, or refusing to relay messages. The Agent Mesh detects Byzantine behavior through reputation-based anomaly detection:

```rust
/// Byzantine behavior detection for mesh agents.
///
/// Monitors agent behavior patterns and flags deviations from expected
/// protocol behavior. Does not require BFT consensus (which is O(N²));
/// instead uses statistical anomaly detection on pheromone patterns.
///
/// # Detection Heuristics
///
/// 1. **False deposit rate**: Agent deposits pheromones that are
///    contradicted by >70% of subsequent observations
/// 2. **Confirmation flooding**: Agent confirms at >5× the median
///    confirmation rate (Sybil-like behavior)
/// 3. **Signal suppression**: Agent receives but never relays messages
///    (gossip protocol violation, detectable via missing seq numbers)
/// 4. **Contradiction oscillation**: Agent alternates between confirming
///    and contradicting the same pheromone (disruptive noise)
///
/// # Response
///
/// Flagged agents have their pheromones discounted (reputation → 0.1)
/// and are eventually excluded from gossip peer lists. Exclusion is
/// reversible if behavior normalizes over a probation period.
pub struct ByzantineDetector {
    /// Window size for behavior analysis (ticks).
    /// Default: 500. Range: [100, 5000].
    pub analysis_window: u64,

    /// False deposit rate threshold for flagging.
    /// Default: 0.3 (30% of deposits contradicted). Range: [0.1, 0.5].
    pub false_deposit_threshold: f64,

    /// Confirmation rate multiplier threshold.
    /// Default: 5.0 (5× median). Range: [2.0, 20.0].
    pub confirmation_flood_multiplier: f64,

    /// Probation duration before reputation can recover.
    /// Default: 2000 ticks. Range: [500, 10000].
    pub probation_duration: u64,

    /// Minimum reputation during probation.
    /// Default: 0.1. Range: [0.0, 0.3].
    pub probation_reputation: f64,
}

impl Default for ByzantineDetector {
    fn default() -> Self {
        Self {
            analysis_window: 500,
            false_deposit_threshold: 0.3,
            confirmation_flood_multiplier: 5.0,
            probation_duration: 2000,
            probation_reputation: 0.1,
        }
    }
}
```

### Consistency Guarantees

The Agent Mesh provides different consistency levels for different message types:

| Message Type | Consistency Level | Rationale |
|-------------|------------------|-----------|
| Pheromone deposits | Eventual | Fuzzy signals; delays are tolerable |
| Morphogenetic broadcasts | Eventual | Role vectors converge over many ticks |
| Niche vacancy alerts | Best-effort immediate | Time-sensitive but not catastrophic if delayed |
| Threat pheromones (critical) | Best-effort immediate | Urgent but agents can also detect threats locally |
| Version vector exchange | Strong (per-session) | Required for correct deduplication |
| Knowledge blobs | Content-addressed (BLAKE3) | Integrity via hash; availability via retry |

The explicit choice of eventual consistency over strong consistency is what enables the Agent Mesh to scale linearly (O(N)) rather than quadratically (O(N²) for consensus-based approaches) while maintaining partition tolerance.

---

## Security Model

### Transport-Level Security

| Transport | Authentication | Encryption | Integrity |
|-----------|---------------|------------|-----------|
| WebSocket | API key or x402 wallet signature linked to ERC-8004 | TLS 1.3 | TLS |
| Iroh | Mutual ed25519 authentication during QUIC handshake | QUIC TLS 1.3 (E2E) | QUIC |
| ERC-8004 | On-chain identity verification | N/A (public data) | Blockchain consensus |

### Application-Level Screening

Incoming Iroh connections are screened:

1. Is this NodeId in my Collective? (Check ERC-8004 `getAgentsByOperator()`)
2. If not Collective, is it a known cross-collective peer? (Check ERC-8004 for valid
   registration)
3. If unknown, reject the connection

### Trust Gradients

| Source | Trust Factor | Confidence Multiplier |
|--------|-------------|----------------------|
| Self (own agent) | 1.0 | None |
| Collective (sibling) | 0.9 | ×0.80 |
| Public (anonymous) | 0.7 | ×0.50 |
| Cross-collective (stranger) | 0.6 | ×0.60 |

### Gossip Security

Iroh-gossip messages are signed by the sender's ed25519 key. Recipients verify the signature
before processing. Application-level validation (confidence bounds, domain checks, pheromone
format) is applied after signature verification.

### Address Privacy

By default, Iroh publishes only the relay URL via pkarr (not direct IP addresses), preventing
IP leakage. Configurable via `AddrFilter::relay_only()` (default) or
`AddrFilter::unfiltered()`.

---

## Version Vectors and Deduplication

Each agent maintains a version vector tracking the highest sequence number received from each
source:

```rust
/// Version vector for deduplication and delta computation.
/// Maps agent_id -> highest_seen_sequence_number.
///
/// Sequence numbers are u64, assigned by each agent's local counter.
/// The counter is persisted to disk so it survives restarts.
pub struct VersionVector {
    /// Map from agent_id to highest seen sequence number.
    entries: HashMap<AgentId, u64>,
}

/// Per-agent sequence number generator.
///
/// Each agent maintains a monotonically increasing counter. The counter
/// is persisted to `{data_dir}/mesh_seq.u64` (8 bytes, little-endian).
/// On startup, the counter loads from disk. If the file is missing or
/// corrupt, the counter resets to 0 — receivers handle this via the
/// wraparound recovery protocol.
pub struct SeqGenerator {
    current: AtomicU64,
    persist_path: PathBuf,
}

impl SeqGenerator {
    /// Allocate the next sequence number.
    /// The counter increments atomically — safe for concurrent deposits.
    pub fn next(&self) -> u64 {
        let seq = self.current.fetch_add(1, Ordering::Relaxed);
        // Async persist — best effort. If the write fails, the counter
        // is still correct in memory. It will be re-persisted on the
        // next successful write.
        self.persist_async(seq + 1);
        seq
    }
}
```

### Sequence number assignment

Each outbound message gets a sequence number from the agent's local `SeqGenerator`. The
number is embedded in the `MeshEnvelope.seq` field. Sequence numbers are per-agent, not
per-topic or per-message-type: all messages from agent A share a single counter. This keeps
the version vector compact (one entry per agent, not per topic).

### How deduplication works

1. Agent A deposits pheromone with seq=42
2. Pheromone arrives via Iroh (fast): Agent B checks version vector, seq 42 is new -> process,
   update vector to {A: 42}
3. Same pheromone arrives via WebSocket (slower): Agent B checks version vector, seq 42
   already seen -> drop silently

This ensures exactly-once processing regardless of how many transports deliver the message.

### Wraparound recovery

A `u64` counter at 1 message per millisecond takes ~584 million years to wrap. In practice,
wraparound does not happen. The recovery protocol exists for a different scenario: **counter
reset** after data loss (disk failure, corrupt persist file, fresh deployment with new state).

When agent A's counter resets to 0 but agent B's version vector says `{A: 5000}`, agent B
would reject all of A's messages as "already seen."

```
Recovery protocol:
1. Agent A detects its counter is lower than peers expect (peers send NACK
   with their last-seen seq for A).
2. Agent A broadcasts a `SeqReset` message (signed, includes new starting seq = 0
   and a reset_epoch that increments on each reset).
3. Receivers update their version vector: set A's entry to 0, record the new
   reset_epoch. Future messages from A are accepted starting from seq 0.
4. The reset_epoch prevents replay attacks: a message with an old reset_epoch
   is rejected even if the seq is valid for the current epoch.
```

```rust
/// Sent when an agent's sequence counter resets (data loss, fresh deploy).
/// Receivers must update their version vector to accept messages from seq 0.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeqReset {
    pub agent_id: AgentId,
    /// Monotonically increasing epoch. Each reset increments this.
    /// Persisted alongside the sequence counter.
    pub reset_epoch: u64,
    /// Ed25519 signature over (agent_id || reset_epoch).
    pub signature: [u8; 64],
}
```

### Delta sync on reconnection

When an agent reconnects after being offline:

1. Agent sends its current version vector to the Mesh server or peer
2. Server/peer computes the delta: all Engrams with seq > the vector's value for each source
3. Delta is sent to the reconnecting agent
4. Agent processes the delta and updates its version vector

This is efficient: only new data is transferred, and the version vector is compact (one entry
per agent in the Collective).

---

## Configuration

```toml
[mesh]
# Master switch — false means local-only operation.
enabled = true

[mesh.websocket]
enabled = true
url = "wss://mesh.roko.dev/v1/mesh/ws"
reconnect_interval_secs = 5
store_forward_ttl_days = 7

[mesh.iroh]
enabled = true
secret_key_path = "data/iroh.key"
relay_mode = "default"  # "default" (n0 public), "custom", or "disabled"
# relay_url = "https://relay.my-mesh.example.com"  # for relay_mode = "custom"

[mesh.discovery]
# ERC-8004 agent discovery
erc8004_poll_interval_secs = 300
cross_collective_enabled = false
max_cross_collective_peers = 50

[mesh.sync]
# Sync timing
batch_interval_ticks = 50  # Curator-aligned
immediate_threshold = 0.7  # Push immediately above this intensity

[mesh.budget]
max_per_tick = 0.01        # Max USDC per tick
daily_budget = 0.50        # Max USDC per day
monthly_budget = 10.00     # Max USDC per month
```

---

## References

- [Fidge 1988] Timestamps in Message-Passing Systems, *ACSC*
- [Lamport 1978] Time, Clocks, and Events, *CACM*
- [Parunak, Brueckner & Sauter 2005] Digital pheromones, *E4MAS*

---

## Cross-References

- `03-digital-pheromones.md` — What gets transported (pheromone types and decay)
- `05-pheromone-scope.md` — Scope model (Local, Mesh, Global)
- `08-permissioned-subnets.md` — Private Mesh scopes
- `09-stigmergy-scaling.md` — How the transport layer scales
- `../../tmp/refinements/09-phase-2-implications.md` — Phase 2+ Bus/Substrate split for Mesh, Dreams, coordination, and heartbeat
- `../00-architecture/01-naming-and-glossary.md` — Glossary for Bus, Pulse, MeshBus, and MeshSubstrate
