# Agent Mesh Sync: Transport Layer for Pheromone Propagation

> **Layer**: L0 Runtime (connection lifecycle, event bus), L1 Framework (transport backends,
> protocol definition)
>
> **Synapse traits**: `Substrate` (the Agent Mesh as a distributed store), `Policy` (reactive
> behavior on received messages)
>
> **Prerequisites**: `03-digital-pheromones.md` (what gets transported),
> `05-pheromone-scope.md` (Mesh scope definition)

---

## Overview

The Agent Mesh (renamed from "Styx" in the legacy architecture) is Roko's peer-to-peer
connectivity layer for Mesh-scope pheromone propagation, collective knowledge sharing, and
morphogenetic coordination signals. It provides transport between agents in a Collective
(renamed from "Clade") without requiring a centralized message broker.

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

Regardless of transport, all messages converge on the EventFabric — Roko's internal event bus.
A pheromone arriving via WebSocket and the same pheromone arriving via Iroh produce identical
events. Consumers (NeuroStore ingestion, context assembly, morphogenetic coordinator) never
know which transport delivered a message.

Deduplication happens at the ingestion boundary via version vectors (`{agent_id → last_seen_seq}`). If a pheromone arrives via both WebSocket and Iroh, the second arrival is silently dropped.

---

## WebSocket Transport

### The Outbound WebSocket Model

Every agent that enables Agent Mesh maintains ONE persistent outbound WebSocket connection.
This connection works from behind any NAT, firewall, or restrictive network — it is a standard
outbound HTTPS connection on port 443. No inbound ports, no tunnels, no port forwarding.

The WebSocket connection serves as a bidirectional channel for ALL Mesh traffic: collective
sync, pheromone updates, knowledge exchange, event relay, and TUI event forwarding. This
multiplexed design minimizes connection overhead while supporting the full range of Mesh
services.

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

### WebSocket Message Types

| Message Type | Direction | Content |
|-------------|-----------|---------|
| `PheromoneSync` | Server → Agent | Batch of pheromone Engrams from Collective peers |
| `PheromoneImmediate` | Server → Agent | High-priority pheromone (Threat, high-intensity Anomaly) |
| `PheromoneDelta` | Agent → Server | New pheromone deposits for relay to Collective |
| `KnowledgeSync` | Bidirectional | NeuroStore entries promoted to Mesh scope |
| `MorphogeneticBroadcast` | Agent → Server → Peers | Role vector and specialization signals |
| `VersionVector` | Agent → Server | Sequence numbers for dedup and delta computation |
| `Heartbeat` | Agent → Server | Liveness signal (every 30s) |

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

### Connection Lifecycle

1. **Bind**: `Endpoint::builder()` with secret key, ALPN identifier (`roko/mesh/1`), and
   relay configuration. Spawns background tasks for relay connection and hole-punching.
2. **Publish**: Agent publishes its addressing information via pkarr (DNS-over-HTTPS) so peers
   can resolve its `NodeId` to a network address.
3. **Accept**: Incoming connections are screened by Collective membership — only agents
   registered in the same Collective (verified via ERC-8004 registry or local config) are
   accepted.
4. **Connect**: Outbound connections dial peers by `NodeId`. Iroh resolves via pkarr/DNS,
   attempts direct connection, falls back to relay.
5. **Streams**: Bidirectional QUIC streams for collective sync. One long-lived stream per
   peer, pooled rather than per-message.
6. **Gossip**: Joins topic-based gossip channels for pheromone propagation.
7. **Shutdown**: Close connections, stop accept loop, unpublish from pkarr. 5-second graceful
   timeout.

### Pheromone Propagation via iroh-gossip

Each `(domain, regime)` pair gets a gossip topic. Topic IDs are deterministic:

```
TopicId = blake3::hash(format!("roko/pheromone/{domain}/{regime}"))
```

The topic set is bounded: domains × regimes ≈ 50 topics maximum. Agents subscribe only to
topics for domains they operate in, preventing unnecessary message delivery.

`PheromoneDeposit` messages are serialized (postcard encoding) and broadcast via
`sender.broadcast()`. Decay is computed locally by each agent based on the deposit timestamp —
no coordination needed. The existing exponential decay formula applies unchanged.

When gossip receivers fall behind (`Event::Lagged`), the message is dropped and logged at WARN
level. Pheromone loss is acceptable — signals are fuzzy by nature, and a lost pheromone will
likely be re-deposited or become irrelevant before its absence matters.

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

---

## Service-to-Transport Mapping

| Service | WebSocket | Iroh | Notes |
|---------|-----------|------|-------|
| Collective sync | Store-and-forward for offline agents | Direct QUIC streams | Iroh preferred, WS fallback |
| Pheromone propagation | Central fan-out from Mesh server | iroh-gossip per topic | Gossip scales better |
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
/// Maps agent_id → highest_seen_sequence_number.
type VersionVector = HashMap<AgentId, u64>;
```

### How Deduplication Works

1. Agent A deposits pheromone with seq=42
2. Pheromone arrives via Iroh (fast): Agent B checks version vector, seq 42 is new → process,
   update vector to {A: 42}
3. Same pheromone arrives via WebSocket (slower): Agent B checks version vector, seq 42
   already seen → drop silently

This ensures exactly-once processing regardless of how many transports deliver the message.

### Delta Sync on Reconnection

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

## Related Sub-Docs

- `03-digital-pheromones.md` — What gets transported (pheromone types and decay)
- `05-pheromone-scope.md` — Scope model (Local, Mesh, Global)
- `08-permissioned-subnets.md` — Private Mesh scopes
- `09-stigmergy-scaling.md` — How the transport layer scales
