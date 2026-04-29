# Mesh Sync and Subnets

> Depth for [06-MEMORY.md](../../unified/06-MEMORY.md). Bus federation across Space boundaries via dual-transport (WebSocket relay + Iroh gossip), partition-tolerant eventual consistency with version-vector dedup, ERC-8004 agent discovery, permissioned subnets as nested Spaces with Bus partitions, and the Weismann barrier for cross-Space trust.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal/Pulse duality, graduation), [02-CELL](../../unified/02-CELL.md) (Connect, Verify, Score protocols), [03-GRAPH](../../unified/03-GRAPH.md) (Pipeline), [11-CONNECTIVITY](../../unified/11-CONNECTIVITY.md) (relay wire protocol, exoskeleton), [10-GROUPS](../../unified/10-GROUPS.md) (Group, Space, coordination modes), [11-stigmergy-as-bus](11-stigmergy-as-bus.md) (Bus-native stigmergy, scoped visibility), [12-pheromone-mechanics-and-interference](12-pheromone-mechanics-and-interference.md) (kind system, promotion cascade)

---

## 1. The Federation Problem

A single agent running locally needs only a local Bus and local Store. Stigmergy works on one machine with zero network overhead. But when agents form a Group (see [10-GROUPS.md](../../unified/10-GROUPS.md)), their Bus fabrics must connect -- Pulses deposited by one agent must be visible to others in the same Group. This is the **federation problem**: extending Bus and Store across network boundaries while preserving the stigmergic properties (decentralization, decay, eventual consistency).

The solution is not to build a global synchronized database. It is to federate Bus instances across Space boundaries using a dual-transport layer that respects the AP (Available, Partition-tolerant) design point. Pheromone signals are inherently fuzzy -- a missed Pulse is not catastrophic because the same signal will be re-deposited if it is still relevant. This makes eventual consistency not just acceptable but natural.

---

## 2. Dual-Transport Architecture

Two transport mechanisms work in tandem. Neither is primary -- they complement each other:

| Transport | Technology | Latency | Best For |
|-----------|-----------|---------|----------|
| **WebSocket relay** | Standard WSS over TLS 1.3 | ~50ms WAN | Always-available, store-and-forward for offline agents, NAT-friendly |
| **Iroh gossip** | QUIC + ed25519, HyParView + PlumTree | ~10ms LAN, ~100ms WAN | Direct P2P, bounded fan-out, content-addressed blob transfer |

### 2.1 The Connect Cell

The transport layer is expressed as a **Connect Cell** with failover (see [02-CELL.md](../../unified/02-CELL.md) for the Connect protocol):

```rust
/// Connect Cell for mesh transport with dual-backend failover.
///
/// Implements the Connect protocol. Manages lifecycle of both
/// WebSocket and Iroh connections. Routes Pulses through whichever
/// transport is available; prefers Iroh (lower latency) when both work.
struct MeshConnectCell {
    /// WebSocket relay connection state.
    ws: Option<WebSocketConnection>,
    /// Iroh endpoint state.
    iroh: Option<IrohEndpoint>,
    /// Health check interval. Default: 30 seconds.
    health_interval: Duration,
    /// Failover policy: which transport to prefer and when to switch.
    failover: FailoverPolicy,
}

enum FailoverPolicy {
    /// Prefer Iroh when available, fall back to WebSocket.
    PreferIroh,
    /// Use WebSocket only (simpler configuration).
    WebSocketOnly,
    /// Use Iroh only (LAN-only operation).
    IrohOnly,
    /// Use both simultaneously, dedup on receive.
    DualWrite,
}
```

### 2.2 Four Valid Transport Combinations

| WebSocket | Iroh | Behavior |
|-----------|------|----------|
| off | off | Local-only agent. No mesh sync. ~95% core capability via local Store. Valid for development/testing. |
| on | off | Classic relay. All services multiplexed over single outbound WSS. Store-and-forward for offline agents (7-day TTL). |
| off | on | Pure P2P. Sync via direct QUIC streams, pheromones via gossip, knowledge via content-addressed blobs. No store-and-forward. |
| **on** | **on** | **Recommended.** Iroh for direct peer communication. WebSocket for store-and-forward and services requiring a server. Dedup via version vectors. |

---

## 3. Version Vector Deduplication

When the same Pulse arrives via both WebSocket and Iroh (or is re-sent after a reconnection), it must be deduplicated. Each agent maintains a version vector: `{AgentId -> last_seen_seq}`.

```rust
/// Version vector for deduplication across transport backends.
///
/// Each agent increments its sequence number monotonically.
/// Receivers track the highest seen sequence per sender.
/// A Pulse with seq <= last_seen_seq[sender] is a duplicate.
struct VersionVector {
    /// Map from sender AgentId to highest seen sequence number.
    entries: HashMap<AgentId, u64>,
}

impl VersionVector {
    /// Check if a Pulse is new (not yet seen from this sender).
    fn is_new(&self, sender: &AgentId, seq: u64) -> bool {
        match self.entries.get(sender) {
            Some(&last_seen) => seq > last_seen,
            None => true,
        }
    }

    /// Record that we have seen this Pulse.
    fn mark_seen(&mut self, sender: &AgentId, seq: u64) {
        let entry = self.entries.entry(sender.clone()).or_insert(0);
        *entry = (*entry).max(seq);
    }
}
```

**Scaling**: Version vectors grow as O(N) entries, one per agent. For N = 1,000 agents: 1,000 * 16 bytes (8-byte AgentId hash + 8-byte seq) = 16 KB. Version vector exchange on reconnection transfers at most N * 16 bytes of metadata before delta sync begins -- negligible overhead.

---

## 4. WebSocket Relay

### 4.1 The Outbound Model

Every agent maintains ONE persistent outbound WebSocket connection (standard HTTPS on port 443). This works from behind any NAT or firewall. The connection multiplexes all mesh traffic: pheromone sync, knowledge exchange, morphogenetic broadcasts, heartbeats.

### 4.2 Store-and-Forward

If an agent is offline when a Pulse delta is pushed, the relay server stores it. When the agent reconnects, it receives all pending deltas in order. Pending deltas expire after a configurable TTL (default: 7 days).

Queue sizing for typical deployment: 5 agents offline for 24 hours with 25 deposits/day each = 125 pending entries * ~1KB each = ~125 KB. Even with generous TTLs, the queue is modest.

### 4.3 Message Priority

```rust
/// Priority levels for outbound WebSocket messages.
enum MeshPriority {
    /// Threat Pulses, role conflict alerts, niche vacancies.
    /// Sent immediately, bypassing batch interval.
    Critical = 0,
    /// High-intensity pheromone Pulses (> immediate_threshold).
    High = 1,
    /// Standard batched sync: pheromone sync, knowledge sync, morphogenetic.
    Normal = 2,
    /// Heartbeat, version vector exchanges, telemetry.
    Low = 3,
}
```

When the outbox exceeds capacity (default: 1024 messages), the oldest Low priority messages are dropped first, then Normal. Critical and High messages are never dropped -- if the queue is full of critical messages, backpressure propagates to the caller.

---

## 5. Iroh Gossip Transport

### 5.1 HyParView + PlumTree

Iroh-gossip uses HyParView for membership management and PlumTree for efficient broadcast:

- **O(log N) message delivery**: Each agent forwards to O(log N) peers; the gossip tree ensures full coverage.
- **Bounded per-node bandwidth**: Each agent's outgoing bandwidth is `degree * message_rate`, regardless of Group size.
- **Eventual delivery**: Not total ordering, but sufficient for fuzzy pheromone signals.

### 5.2 Topic-Based Subscription

Each (domain, regime) pair gets a deterministic gossip topic:

```
TopicId = blake3("roko/pheromone/{domain}/{regime}")
```

The topic set is bounded: domains * regimes ~ 50 topics maximum. Agents subscribe only to topics for domains they operate in, preventing unnecessary Pulse delivery.

When gossip receivers fall behind (`Event::Lagged`), the message is dropped and logged. Pheromone loss is acceptable -- signals are fuzzy by nature, and a lost Pulse will be re-deposited or become irrelevant before its absence matters.

### 5.3 Content-Addressed Knowledge Exchange

For large knowledge artifacts (Signals promoted to mesh scope), iroh-blobs provides content-addressed transfer:

1. Sending agent adds the knowledge bundle to its local iroh-blobs store
2. The BLAKE3 hash is announced via gossip on a `roko/knowledge/{group_id}` topic
3. Receiving agents download the blob on-demand using the hash
4. BLAKE3's BAO (Authenticated Ordering) provides incremental verification

---

## 6. ERC-8004 Agent Discovery

ERC-8004 (Agent Cards) provides on-chain service discovery. Each registered agent publishes an Agent Card containing:

| Field | Description |
|-------|-------------|
| `agentId` | On-chain agent identifier |
| `operator` | Operator address (for Group discovery) |
| `capabilities` | Bitmask of agent capabilities |
| `serviceEndpoints` | WebSocket URL, Iroh NodeId |
| `reputation` | Per-domain reputation scores |
| `domains` | Domains the agent operates in |

### 6.1 Three Discovery Scopes

| Scope | Mechanism | Configuration |
|-------|-----------|---------------|
| **Same LAN** | Iroh mDNS/DNS-SD zero-config discovery | Automatic |
| **Same Group** | ERC-8004 `getAgentsByOperator()` -> extract Iroh NodeIds | On boot + periodic (300s) |
| **Cross-Group** | ERC-8004 browse by capability/reputation/domain | Off by default (`cross_collective_enabled = false`) |

---

## 7. Permissioned Subnets

A permissioned subnet is a **nested Space with a Bus partition** (see [03-GRAPH.md](../../unified/03-GRAPH.md) for the Space pattern). While the standard Group scope makes Pulses visible to all members, subnets add access control -- restricting visibility to specific agents within a Group.

### 7.1 Subnet as Nested Space

```
Space Hierarchy:
+-- Global (all agents)
+-- Group(GroupId) (standard -- all Group members)
|   +-- Subnet("engineering") (permissioned -- engineering agents only)
|   +-- Subnet("research") (permissioned -- research agents only)
|   +-- Subnet("security") (permissioned -- security agents + admins)
+-- Local(AgentId) (agent-private)
```

Subnets do not replace the standard Group scope -- they add a finer-grained layer within it. An agent in the "engineering" subnet can still see Group-scope Pulses from all members, but Pulses deposited at subnet scope are visible only to other subnet members.

### 7.2 Three Access Control Models

```toml
# Invite-based: explicit agent list
[mesh.subnets.security-team]
access_model = "invite"
members = ["agent-security-lead", "agent-security-scanner"]

# Role-based: predicate on agent properties
[mesh.subnets.engineering]
access_model = "role"
role_predicate = "agent_type == 'coding' OR agent_type == 'testing'"

# Reputation-based: meritocratic admission
[mesh.subnets.elite-research]
access_model = "reputation"
min_reputation = 0.85
domain = "research"
```

### 7.3 The Weismann Barrier

Knowledge inherited from other Spaces (as opposed to self-generated) is discounted by trust multipliers. This is the **Weismann barrier** -- analogous to the biological separation between germline and soma that prevents acquired traits from being inherited directly.

The trust discount is expressed as a **Score Cell** that adjusts Signal weight by provenance:

```rust
/// Score Cell that applies Weismann barrier trust multipliers.
///
/// Signals from different provenance zones receive different trust weights.
/// This prevents low-trust information from contaminating high-trust stores.
struct WeismannBarrierScore {
    /// Trust multiplier for self-generated Signals. Default: 1.0.
    self_trust: f64,
    /// Trust multiplier for same-Group Signals. Default: 0.80.
    collective_trust: f64,
    /// Trust multiplier for cross-Group Signals. Default: 0.60.
    cross_collective_trust: f64,
    /// Trust multiplier for anonymous/unknown-provenance Signals. Default: 0.50.
    anonymous_trust: f64,
}

impl WeismannBarrierScore {
    fn trust_multiplier(&self, provenance: &Provenance) -> f64 {
        match provenance {
            Provenance::Self_ => self.self_trust,
            Provenance::SameGroup(_) => self.collective_trust,
            Provenance::CrossGroup(_) => self.cross_collective_trust,
            Provenance::Anonymous => self.anonymous_trust,
        }
    }
}
```

### 7.4 Publish Gates

Moving knowledge from a narrower scope to a broader scope requires passing through a Verify Cell gate:

| Transition | Gate | Requirements |
|-----------|------|-------------|
| Subnet -> Group | Publishing gate | >= 2 subnet member confirmations + optional human approval |
| Group -> Global | Promotion gate | >= 4 Group member confirmations + minimum reputation |

```toml
[mesh.subnets.engineering.publishing]
auto_publish = false  # Require explicit approval
min_confirmations = 2
require_human_approval = true
publishable_kinds = ["Wisdom", "Consensus", "Pattern"]
restricted_kinds = ["Threat", "Alpha"]  # Stay private
```

The publishing gate enforces an information boundary: subnet-private Pulses cannot leak to broader scopes without passing through the gate. This is implemented at the transport layer -- the relay refuses to forward subnet-scoped messages to non-members.

```rust
/// Verify Cell for scope boundary enforcement.
///
/// Pre-condition: sender is a member of the source subnet.
/// Checks: target scope is not broader than source (unless publishing
/// gate is satisfied), restricted kinds never leave the subnet.
fn verify_scope_boundary(
    sender: &AgentId,
    pulse: &PheromonePulse,
    target_scope: &Scope,
    subnet_config: &SubnetConfig,
) -> Verdict {
    // Check membership
    if !subnet_config.is_member(sender) {
        return Verdict::Reject("not a subnet member");
    }
    // Check scope broadening
    if target_scope.is_broader_than(&pulse.scope) {
        if !subnet_config.publishing_gate_satisfied(pulse) {
            return Verdict::Reject("publishing gate not met");
        }
        if subnet_config.is_restricted_kind(&pulse.kind) {
            return Verdict::Reject("restricted kind cannot leave subnet");
        }
    }
    Verdict::Accept
}
```

### 7.5 Club Goods Economics

Permissioned subnets implement what economists call "club goods" (Buchanan 1965) -- goods that are excludable (non-members cannot access) but non-rivalrous (one member's use does not diminish value for others).

- **Excludability**: Subnet Pulses are visible only to members (enforced by access control)
- **Non-rivalrousness**: One agent sensing a Pulse does not diminish its availability

This structure incentivizes collective knowledge production within subnets: members benefit from shared knowledge without the free-rider problem that affects pure public goods (global scope). The opt-in publishing mechanism allows subnets to selectively convert club goods into public goods when collective benefit outweighs competitive advantage.

---

## 8. Transport Scaling

### 8.1 Relay vs Gossip

| Group Size | Relay Bandwidth/Node | Gossip Bandwidth/Node | Winner |
|------------|---------------------|-----------------------|--------|
| 5 | 4 * 1KB = 4KB/msg | 2 * 1KB = 2KB/msg | Gossip |
| 50 | 49 * 1KB = 49KB/msg | 6 * 1KB = 6KB/msg | Gossip |
| 500 | 499 * 1KB = 499KB/msg | 9 * 1KB = 9KB/msg | Gossip |
| 5,000 | 4,999 * 1KB ~ 5MB/msg | 12 * 1KB = 12KB/msg | Gossip |

At all Group sizes, gossip provides better per-node bandwidth scaling. However, the relay provides store-and-forward for offline agents and requires no per-agent configuration. The recommended deployment uses both.

### 8.2 Partition Tolerance

The AP design means that during a network partition:
- Agents on each side of the partition continue operating independently
- Pheromone Pulses accumulate locally on each side
- On reconnection, version vectors drive delta sync
- Duplicate Pulses are silently dropped
- The pheromone field reconverges naturally because old Pulses have decayed during the partition

No conflict resolution protocol is needed. Eventual consistency with decay is the correct semantics for fuzzy coordination signals.

---

## What This Enables

1. **Zero-config local operation**: A single agent or local Group works without any network configuration. The Bus and Store operate locally with identical semantics.
2. **Transparent scale-up**: Adding network connectivity (WebSocket, Iroh, or both) extends the Bus across machines without changing the coordination model. Cells that work locally work identically over the mesh.
3. **Partition tolerance by design**: Network partitions are expected, not exceptional. The AP design with version-vector dedup handles them gracefully.
4. **Information access control**: Subnets provide organizational control over knowledge flow without sacrificing the stigmergic coordination model.
5. **Trust-weighted knowledge federation**: The Weismann barrier prevents low-trust information from contaminating high-trust stores, enabling safe cross-Group knowledge sharing.

## Feedback Loops

1. **Transport quality Loop**: Bus delivery rate (see [15-collective-metrics-as-lens.md](15-collective-metrics-as-lens.md)) feeds back into transport selection. If WebSocket drops too many messages, the system prefers Iroh. If Iroh latency degrades, it falls back to WebSocket.
2. **Trust calibration Loop**: Weismann barrier multipliers can be adjusted based on observed Signal quality from each provenance zone. If cross-Group Signals consistently validate well, the trust multiplier increases.
3. **Subnet membership Loop**: Reputation-based subnets automatically adjust their membership as agent reputation changes. Agents that lose reputation lose access; agents that gain reputation gain access.

## Open Questions

1. **Should subnets support nested subnets?** The current design has a flat subnet structure within a Group. Nested subnets (subnet within subnet) would add complexity but might be needed for large organizations.
2. **How should relay server selection work for global scope?** Multiple relay servers may exist. Load balancing, geographic routing, and relay selection are not yet specified.
3. **What happens when iroh-gossip topics become very sparse?** With few subscribers, HyParView membership may degrade. A minimum membership count or bootstrap mechanism may be needed.
4. **Should the Weismann barrier multipliers be per-agent or per-zone?** The current design uses per-zone multipliers. Per-agent multipliers (based on individual track record) would be more granular but harder to manage.

## Implementation Tasks

1. **Implement `MeshConnectCell` in `roko-runtime`**: `crates/roko-runtime/src/mesh.rs` -- dual-transport Connect Cell with failover policy, health checking, connection lifecycle.
2. **Implement version vector dedup**: `crates/roko-runtime/src/dedup.rs` -- `VersionVector` struct, `is_new()`/`mark_seen()` methods, persistence to `.roko/state/version-vectors.json`.
3. **Add subnet scope to Bus**: `crates/roko-core/src/bus.rs` -- extend topic addressing to include subnet partition, enforce scope boundaries on publish.
4. **Implement `WeismannBarrierScore`**: `crates/roko-core/src/scoring.rs` -- trust multiplier Score Cell, provenance classification, configurable multipliers in `roko.toml`.
5. **Implement publishing gate Verify Cell**: `crates/roko-gate/src/publishing.rs` -- confirmation count check, restricted kind enforcement, optional human approval integration.
6. **Add subnet configuration to `roko.toml` parsing**: `crates/roko-core/src/config.rs` -- parse `[mesh.subnets.*]` sections, validate access models, propagate to Bus configuration.
7. **Wire Iroh gossip into Bus**: `crates/roko-runtime/src/iroh_transport.rs` -- gossip topic management, reconciliation on domain/regime changes, `Event::Lagged` handling.
8. **Wire ERC-8004 discovery**: `crates/roko-runtime/src/discovery.rs` -- periodic Agent Card queries, NodeId extraction, connection bootstrapping for Connect Cell.

---

## References

- Buchanan, J.M. 1965, "An Economic Theory of Clubs", *Economica*
- Ostrom, E. 1990, *Governing the Commons*, Cambridge University Press
- Grossman & Stiglitz 1980, "Informationally Efficient Markets", *AER*
- Parunak 1997, "Engineering from natural MAS", *Ann. Oper. Res.*
- Tse & Viswanath 2005, *Fundamentals of Wireless Communication*, Cambridge University Press
