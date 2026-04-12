# Stigmergy Scaling: How Coordination Costs Grow

> **Layer**: L4 Orchestration (scaling analysis), L0 Runtime (resource management)
>
> **Synapse traits**: All six — scaling affects every trait's performance characteristics
>
> **Prerequisites**: `00-stigmergy-theory.md` (stigmergy fundamentals),
> `06-agent-mesh-sync.md` (transport layer)


> **Implementation**: Specified

---

## Overview

A critical question for any multi-agent coordination mechanism is: **how does coordination
cost scale with the number of agents?** This sub-doc analyzes the scaling properties of
Roko's stigmergic coordination system — the pheromone field, the Agent Mesh transport,
morphogenetic specialization, and the collective intelligence metrics — and shows that
stigmergy provides fundamentally better scaling than direct communication alternatives.

---

## Coordination Cost Models

### Direct Communication: O(N²)

In a direct communication system (message passing, shared blackboard with point-to-point
channels), every agent must potentially communicate with every other agent. For N agents:

- **Point-to-point channels**: N × (N-1) / 2 = O(N²) channels
- **Message volume**: Each agent sends messages to every other agent = O(N²) messages per round
- **Leader election**: Consensus protocols typically require O(N²) or O(N log N) messages

This quadratic scaling makes direct communication impractical for large Collectives. A
100-agent Collective would require ~5,000 communication channels, each consuming bandwidth
and processing time.

### Stigmergy: O(N × M)

In Roko's stigmergic system, coordination cost scales as O(N × M), where:

- **N** = number of agents in the Collective
- **M** = number of distinct pheromone kinds × number of scope levels

Since M is bounded and small (7 built-in kinds × 3 scopes = 21 channels, plus custom kinds),
coordination cost grows **linearly** with agent count.

Each agent performs two operations per stigmergic cycle:

1. **Deposit**: Write one or more pheromone Engrams to the Substrate → O(1) per agent
2. **Sense**: Query the Substrate for relevant pheromones → O(M) per agent, where M is the
   number of pheromone kinds the agent monitors

Total coordination cost per cycle: O(N × M) = O(N) for fixed M.

### Comparison

| Agent Count (N) | Direct Comm O(N²) | Stigmergy O(N × M), M=20 | Ratio |
|----------------|--------------------|-----------------------------|-------|
| 5 | 10 | 100 | 0.1× (direct wins for tiny N) |
| 20 | 190 | 400 | 0.5× |
| 50 | 1,225 | 1,000 | 1.2× (crossover point) |
| 100 | 4,950 | 2,000 | 2.5× |
| 500 | 124,750 | 10,000 | 12.5× |
| 1,000 | 499,500 | 20,000 | 25× |
| 10,000 | 49,995,000 | 200,000 | 250× |

The crossover point (where stigmergy becomes more efficient than direct communication) occurs
at approximately N = 40 for M = 20. Below this threshold, direct communication may be simpler
to implement, but stigmergy's advantages (robustness, asynchrony, minimal agent complexity)
still apply even at small scales.

---

## Pheromone Field Scaling

### Storage Scaling

The pheromone field (aggregate of all active pheromones) scales as:

```
Storage = N_agents × R_deposit × τ_mean / R_gc
```

Where:
- `N_agents` = number of agents depositing
- `R_deposit` = average deposit rate per agent (5–25 per day)
- `τ_mean` = mean pheromone half-life (weighted by kind distribution)
- `R_gc` = garbage collection rate

For a typical Collective of 10 agents:
- Each agent deposits ~15 pheromones/day
- Mean half-life: ~8 hours (weighted average across kinds)
- After 3 half-lives (~24h), intensity < 12.5% (near GC threshold)
- Steady-state active pheromones: ~10 × 15 × 1.0 = **~150 active pheromones**

This is modest. Even at 1,000 agents, the steady-state is ~15,000 active pheromones — easily
manageable with a simple in-memory store or JSONL file.

### Query Scaling

When an agent queries the pheromone field, it scans active pheromones matching its filter:

- **Naive scan**: O(P) where P = total active pheromones. For P = 15,000, this is fast enough
  (~1ms with simple iteration).
- **Indexed scan**: O(log P) using kind-indexed or scope-indexed data structures. Worthwhile
  for P > 100,000.
- **Approximate scan**: O(1) using Bloom filters to skip irrelevant scopes. Used for
  cross-Collective queries.

For most Roko deployments (Collective sizes 2–50), naive scanning is sufficient. The pheromone
field never grows large enough to require sophisticated indexing.

### Decay as Natural GC

The exponential decay of pheromones provides automatic garbage collection. Unlike a growing
log or append-only store, the pheromone field has a natural steady-state size determined by
the balance between deposit rate and decay rate. This means:

1. **No unbounded growth**: The field size is bounded by `N × R_deposit × τ_mean`.
2. **Self-healing**: If the field becomes noisy (too many low-quality deposits), the noise
   decays away. Only confirmed, high-quality signals persist.
3. **No compaction needed**: Unlike database tables that require periodic compaction, the
   pheromone field self-compacts through decay.

---

## Transport Scaling

### WebSocket Relay

The WebSocket relay scales with the number of connected agents and the message rate:

| Factor | Scaling | Analysis |
|--------|---------|----------|
| Connections | O(N) | One persistent connection per agent |
| Fan-out | O(N) per message | Each deposit relayed to N-1 Collective members |
| Bandwidth | O(N × R_deposit × S_msg) | R_deposit = deposit rate, S_msg = message size |
| Latency | O(1) | Constant relay latency regardless of N |

For a 50-agent Collective with 15 deposits/day/agent, each message ~1KB:
- Total messages: 50 × 15 = 750/day
- Fan-out: 750 × 49 = 36,750 deliveries/day = ~0.4/second
- Bandwidth: 36,750 × 1KB = ~36 MB/day

This is trivially handleable by a single relay server. WebSocket scaling becomes a concern only
at >10,000 agents, at which point sharding by Collective becomes necessary.

### Iroh Gossip

Iroh-gossip uses HyParView + PlumTree for epidemic broadcast, which provides:

- **O(log N) message delivery**: Each agent forwards to O(log N) peers; the gossip tree
  ensures full coverage.
- **Bounded fan-out**: Each agent's outgoing bandwidth is bounded by `degree × R_message`,
  regardless of Collective size.
- **Eventual delivery**: Not total ordering, but eventual delivery is sufficient for fuzzy
  pheromone signals.

| Factor | Scaling | Analysis |
|--------|---------|----------|
| Connections per node | O(log N) | HyParView maintains ~log(N) active peers |
| Message delivery | O(N × log N) total network | Each message traverses log(N) hops × N recipients |
| Bandwidth per node | O(R_deposit × log N × S_msg) | Bounded per-node bandwidth |
| Latency | O(log N) hops | Logarithmic propagation delay |

Gossip is more efficient than relay for large Collectives (N > 50) because the per-node
bandwidth is bounded logarithmically rather than linearly.

### Comparison: Relay vs Gossip

| Collective Size | Relay Bandwidth/Node | Gossip Bandwidth/Node | Winner |
|----------------|---------------------|-----------------------|--------|
| 5 | 4 × 1KB/msg = 4KB/msg | 2 × 1KB/msg = 2KB/msg | Gossip |
| 50 | 49 × 1KB/msg = 49KB/msg | 6 × 1KB/msg = 6KB/msg | Gossip |
| 500 | 499 × 1KB/msg = 499KB/msg | 9 × 1KB/msg = 9KB/msg | Gossip |
| 5,000 | 4,999 × 1KB/msg = ~5MB/msg | 12 × 1KB/msg = 12KB/msg | Gossip |

At all Collective sizes, gossip provides better per-node bandwidth scaling. However, the
relay provides store-and-forward for offline agents and requires no per-agent configuration,
making it simpler for small deployments.

---

## Morphogenetic Scaling

### Role Vector Propagation

Each agent broadcasts its 8-dimensional role vector (64 bytes + metadata) every 50 ticks.
The total bandwidth for morphogenetic coordination:

```
Morphogenetic bandwidth = N × (64 bytes / 50 ticks) × tick_rate
```

For a 20-agent Collective at 4 ticks/minute:
- Bandwidth: 20 × 64 × (4/50) = 102 bytes/minute = **~6 KB/hour**

This is negligible relative to pheromone and knowledge sync traffic.

### Inhibition Computation

Each agent computes inhibition pressure from aggregated role vectors — O(N × STRATEGY_DIMS)
per update cycle:

| Collective Size | Inhibition Computation | Time (est.) |
|----------------|----------------------|-------------|
| 5 | 40 multiply-adds | ~1 μs |
| 50 | 400 multiply-adds | ~10 μs |
| 500 | 4,000 multiply-adds | ~100 μs |
| 5,000 | 40,000 multiply-adds | ~1 ms |

Even at 5,000 agents, the morphogenetic update is computationally trivial.

### Convergence Time Scaling

Convergence to stable specialist patterns scales approximately as O(N × log N):

| Collective Size | Convergence Time (ticks) | Wall Time (at 4 ticks/min) |
|----------------|-------------------------|---------------------------|
| 2 | ~500 | ~2 hours |
| 5 | ~800 | ~3.3 hours |
| 10 | ~1,200 | ~5 hours |
| 20 | ~1,800 | ~7.5 hours |
| 50 | ~3,000 | ~12.5 hours |

Convergence time grows sub-linearly with Collective size because larger Collectives have more
diverse noise vectors, which speeds up symmetry breaking [Turing 1952].

---

## Knowledge Sync Scaling

### Sync Volume

Knowledge sync (NeuroStore entries promoted to Mesh scope) scales linearly with agent count:

| Scenario | Entries/day/agent | 5-agent Collective | 50-agent Collective |
|----------|------------------|--------------------|---------------------|
| Quiet | 5–10 | 25–50 entries/day | 250–500 entries/day |
| Active | 15–25 | 75–125 entries/day | 750–1,250 entries/day |
| Volatile | 25–50 | 125–250 entries/day | 1,250–2,500 entries/day |

### Deduplication Overhead

Version vectors grow as O(N) entries (one entry per agent). For N = 1,000:
- Vector size: 1,000 × 16 bytes (8-byte AgentId + 8-byte seq) = **16 KB**
- Dedup check: O(1) per message (hash map lookup)

Version vector exchange on reconnection transfers at most N × 16 bytes of metadata before
delta sync begins — negligible overhead.

### Store-and-Forward Scaling

The WebSocket relay's store-and-forward queue grows with the number of offline agents and
the deposit rate:

```
Queue size = N_offline × R_deposit × TTL
```

For 5 agents offline for 24 hours with 25 deposits/day each:
- Queue: 5 × 25 × 1 = 125 pending entries × ~1KB each = **~125 KB**

Even with generous TTLs (7 days), the store-and-forward queue is modest.

---

## Scaling Limits

### Theoretical Limits

| Factor | Practical Limit | Bottleneck |
|--------|----------------|-----------|
| Agents per Collective | ~10,000 | Gossip fan-out latency |
| Pheromone field size | ~1M active pheromones | In-memory storage |
| Kind diversity | ~100 custom kinds | Configuration complexity |
| Morphogenetic convergence | ~50 agents | Convergence time > 12 hours |
| WebSocket relay connections | ~50,000 | Single relay server capacity |

### Mitigation Strategies

When Collective size approaches limits:

1. **Partition into sub-Collectives**: Use permissioned subnets (see `08-permissioned-subnets.md`) to create smaller coordination groups within the larger Collective.
2. **Hierarchical pheromone aggregation**: Sub-Collective pheromone fields are summarized and propagated to the parent Collective, reducing per-agent bandwidth.
3. **Gossip over relay**: Switch from WebSocket relay to Iroh gossip for Collectives > 50 agents.
4. **Morphogenetic partitioning**: For Collectives > 50 agents, partition the morphogenetic system by domain or project.

---

## Stigmergy vs Alternatives at Scale

### Comparison with Consensus Protocols

| Property | Stigmergy | BFT Consensus (e.g., PBFT) | Raft/Paxos |
|----------|-----------|---------------------------|-----------|
| Message complexity | O(N × M) | O(N²) | O(N) |
| Fault tolerance | N-1 (any agent can fail) | N/3 (Byzantine) | N/2 (crash) |
| Consistency | Eventual | Strong | Strong |
| Latency | O(log N) gossip hops | O(N) rounds | O(log N) |
| Suitable for | Fuzzy coordination, soft state | State machine replication | Leader election, log replication |

Stigmergy is not a replacement for consensus — Global-scope pheromones on the Korai chain
still use consensus for finality. But for the vast majority of coordination tasks (intra-
Collective pheromone propagation, morphogenetic signals, knowledge sharing), stigmergy's
eventual consistency is sufficient and its linear scaling is far superior.

### Comparison with Publish-Subscribe

| Property | Stigmergy (Roko) | Pub-Sub (e.g., Kafka, NATS) |
|----------|------------------|-----------------------------|
| State persistence | Yes (pheromones decay but persist) | Depends (Kafka yes, NATS no) |
| Automatic staleness management | Yes (exponential decay) | No (requires manual TTL or compaction) |
| Confirmation semantics | Built-in (pheromone confirmation) | Not built-in (application-level) |
| Agent simplicity | Deposit + sense only | Publish + subscribe + ack + offset management |
| Broker dependency | None (P2P gossip) or optional (relay) | Required (broker is SPOF) |

---

## Empirical Scaling Data

### Simulated Collective Coordination

From the legacy architecture's simulation results (referenced in
`bardo-backup/prd/02-mortality/10-clade-ecology.md`):

```
Scaling: O(domains × signal_types)
  - 5 domains × 3 signal types × 10 golems = 150 pheromone channels
  - Each channel has independent exponential decay
  - Memory: ~50KB per active pheromone field
  - CPU: <1ms per field update (sense + decay computation)
```

The simulation confirmed linear scaling for Collectives up to 100 agents, with no measurable
degradation in coordination quality (measured by C-Factor, see
`11-collective-intelligence-metrics.md`).

### Reed's Law Implications

Reed's Law states that the value of a network grows as O(2^N) for group-forming networks
[Reed, D.P. "The Law of the Pack." *Harvard Business Review*, 2001]. For Roko Collectives:

- Each subset of agents can potentially form a productive sub-group
- The number of such subsets grows exponentially with N
- But coordination cost grows only linearly (O(N × M))

This means the **value-to-cost ratio grows exponentially** with Collective size — precisely the
property needed for the exponential flywheel (see `10-exponential-flywheel.md`).

---

## Summary

Roko's stigmergic coordination scales linearly with agent count for all primary operations:

| Operation | Scaling | Notes |
|-----------|---------|-------|
| Pheromone deposit | O(1) per agent | Constant cost per deposit |
| Pheromone sensing | O(M) per agent | M = bounded kind count |
| Pheromone transport | O(N) relay, O(N log N) gossip | Gossip preferred for N > 50 |
| Morphogenetic update | O(N × 8) per agent | 8 strategy dimensions |
| Knowledge sync | O(N) total entries | Linear in agent count |
| Version vector dedup | O(1) per message | Hash map lookup |
| Store-and-forward | O(N_offline × R_deposit × TTL) | Bounded by TTL |

The key insight: by choosing stigmergy over direct communication, Roko avoids the O(N²)
scaling wall that limits most multi-agent systems. This enables Collectives of hundreds or
thousands of agents to coordinate effectively — a prerequisite for the exponential flywheel
mechanisms described in `10-exponential-flywheel.md`.

---

## References

- [Bejan 1997] Constructal Law, *Int. J. Heat and Mass Transfer*
- [Dorigo, Maniezzo & Colorni 1996] ACO, *IEEE SMC-B*
- [Grassé 1959] Termite mound stigmergy, *Insectes Sociaux*
- [Parunak 1997] Engineering from natural MAS, *Ann. Oper. Res.*
- [Reed 2001] The Law of the Pack, *Harvard Business Review*
- [Turing 1952] Chemical Basis of Morphogenesis, *Phil. Trans. Royal Society B*

---

## Related Sub-Docs

- `00-stigmergy-theory.md` — Why stigmergy scales better than alternatives
- `06-agent-mesh-sync.md` — Transport layer scaling characteristics
- `10-exponential-flywheel.md` — How linear coordination cost enables exponential value
- `11-collective-intelligence-metrics.md` — Measuring scaling effectiveness
