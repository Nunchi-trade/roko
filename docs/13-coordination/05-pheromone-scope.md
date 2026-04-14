# Pheromone Scope: Local, Mesh, and Global Propagation

> **Layer**: L1 Framework (scope enum definition), L0 Runtime (local persistence), L4
> Orchestration (mesh and global propagation)
>
> **Synapse traits**: `Substrate` (stores Engrams at each scope), `Router` (selects scope for
> new deposits), `Composer` (assembles context across scopes)
>
> **Prerequisites**: `03-digital-pheromones.md` (pheromone fundamentals),
> `04-pheromone-kinds.md` (pheromone type taxonomy)


> **Implementation**: Specified

---

## Overview

Every digital pheromone in Roko has a `PheromoneScope` that determines how far the signal
propagates and who can sense it. The scope system implements a three-level stigmergic
hierarchy inspired by the Constructal Law's prediction that optimal flow systems evolve
dendritic (tree-like) structures [Bejan, A. "Constructal-Theory Network of Conducting Paths
for Cooling a Heat Generating Volume." *Int. J. Heat and Mass Transfer*, 40(4):799-816, 1997].

The three scopes correspond to three distinct stigmergic environments with different trust
levels, persistence characteristics, and coordination goals:

| Scope | Environment | Audience | Trust | Persistence | Cost |
|-------|-------------|----------|-------|-------------|------|
| `Local(SubstrateId)` | Agent's NeuroStore | Self only | 1.0 | Until GC | Free |
| `Mesh(CollectiveId)` | Collective's Agent Mesh | Collective members | 0.8–0.9 | Hours–days | ~$0.01/day |
| `Global` | Korai chain (public) | All agents | 0.5–0.7 | Permanent | Chain tx fee |

---

## The PheromoneScope Enum

```rust
/// The propagation scope of a digital pheromone.
///
/// Scope determines:
/// 1. **Audience**: Who can sense this pheromone
/// 2. **Persistence**: How long the pheromone persists (beyond decay)
/// 3. **Transport**: How the pheromone reaches its audience
/// 4. **Trust**: How much confidence recipients assign to it
/// 5. **Cost**: The resource cost of depositing at this scope
///
/// Scopes form a strict hierarchy: Local ⊂ Mesh ⊂ Global.
/// Information flows upward through promotion gates and downward
/// through queries. A pheromone deposited at Global scope is visible
/// at all three levels; a pheromone deposited at Local scope is
/// visible only to the depositing agent.
///
/// # Design Rationale
///
/// Three scopes, not two or five, because:
/// - Two scopes (local/global) lack the middle ground needed for
///   private group coordination
/// - Five+ scopes add complexity without clear coordination benefit
/// - Three maps to biological precedent: individual, colony, species
///   [Hölldobler & Wilson, "The Superorganism", 2008]
///
/// The three scopes also map to Roko's three knowledge levels:
/// - Local → NeuroStore (the agent's personal knowledge base)
/// - Mesh → Agent Mesh (the collective's shared knowledge)
/// - Global → Korai chain (the ecosystem's public knowledge)
pub enum PheromoneScope {
    /// Pheromone is visible only within the specified Substrate.
    /// No propagation beyond the agent's own NeuroStore.
    ///
    /// Use for:
    /// - Internal state tracking ("I noticed this pattern")
    /// - Work-in-progress observations not yet validated
    /// - Private bookmarks and notes
    /// - Hypotheses awaiting confirmation
    Local(SubstrateId),

    /// Pheromone propagates to all agents in the specified Collective.
    /// Uses the Agent Mesh transport (WebSocket, Iroh, or both).
    ///
    /// Use for:
    /// - Coordination signals between agents in the same group
    /// - Shared threat/opportunity alerts within a team
    /// - Morphogenetic specialization signals (role vectors)
    /// - Collective knowledge building
    ///
    /// The `CollectiveId` identifies which group of agents receives
    /// the signal. An agent may be a member of multiple Collectives.
    Mesh(CollectiveId),

    /// Pheromone is published to the Korai chain for global visibility.
    /// All agents on the network can sense this signal.
    ///
    /// Use for:
    /// - Ecosystem-wide threat alerts (critical vulnerabilities)
    /// - Published research findings and validated wisdom
    /// - Public market signals on the Korai mainnet
    /// - Reputation-relevant contributions
    ///
    /// Global pheromones incur chain transaction fees and are subject
    /// to on-chain validation rules. They persist permanently (on-chain
    /// storage) rather than decaying — the "decay" for Global pheromones
    /// is relevance-based, not time-based.
    Global,
}
```

---

## Local Scope: The Agent's Private Pheromone Field

### Purpose

Local pheromones are the agent's private annotations — observations, hypotheses, and internal
state that have not been validated for sharing. They serve as the agent's working memory for
stigmergic coordination: before a signal is promoted to Mesh scope, it exists as a Local
pheromone.

### Characteristics

| Property | Value |
|----------|-------|
| Audience | Self only (the depositing agent) |
| Transport | None (stored directly in NeuroStore) |
| Trust multiplier | 1.0 (self-generated, maximum trust) |
| Persistence | Until garbage collection (configurable) |
| Decay | Standard exponential decay per `PheromoneKind` |
| Cost | Free (local I/O only) |

### Implementation

Local pheromones are stored in the agent's `NeuroStore`, which is a `Substrate` implementation
backed by JSONL files via `roko-fs`. The `SubstrateId` in `Local(SubstrateId)` identifies which
Substrate instance holds the pheromone — typically the agent's default NeuroStore, but agents
with multiple Substrates (e.g., one per project or domain) can target a specific one.

```rust
// Deposit a local pheromone
let pheromone = Pheromone {
    kind: PheromoneKind::Pattern,
    intensity: 0.7,
    decay_rate: Duration::from_secs(12 * 3600),  // 12h half-life
    source: self.agent_id.clone(),
    scope: PheromoneScope::Local(self.neuro_store_id.clone()),
};
self.substrate.store(pheromone.into_engram())?;
```

### Use Cases

| Use Case | PheromoneKind | Description |
|----------|--------------|-------------|
| Working hypothesis | Pattern | "I think this module has a dependency cycle" |
| Internal bookmark | Opportunity | "This function could be optimized, but it's not my current task" |
| Personal threat note | Threat | "This test is flaky — I saw it fail once" |
| Draft insight | Wisdom | "NaN handling seems to be a recurring issue" (not yet confirmed) |

### Promotion to Mesh Scope

Local pheromones can be promoted to Mesh scope when the agent gains confidence in the signal.
Promotion is not automatic — it requires the agent to actively decide that the signal is worth
sharing. The promotion criteria are:

1. **Confidence threshold**: The pheromone has been locally confirmed (e.g., the agent observed
   the pattern multiple times)
2. **Relevance**: The signal is relevant to other agents in the Collective
3. **Novelty**: No existing Mesh-scope pheromone of the same kind covers the same observation

The promotion mechanism respects the Weismann barrier principle: inherited knowledge (received
from Mesh scope) starts at reduced confidence (×0.80), ensuring that each agent independently
validates shared signals before relying on them [Heard, E. & Martienssen, R.A.
"Transgenerational Epigenetic Inheritance." *Cell*, 157(1), 2014].

---

## Mesh Scope: The Collective's Shared Pheromone Field

### Purpose

Mesh pheromones coordinate agents within a Collective — a group of agents working together
under the same operator or on the same project. This is the primary scope for active
coordination: morphogenetic specialization, threat alerting, opportunity sharing, and
collective knowledge building all happen at Mesh scope.

### What Is a Collective?

A Collective (the renamed concept from the legacy "Clade" terminology) is a group of agents
that share a common purpose and coordinate through the Agent Mesh. Collectives can be:

- **Operator-based**: All agents belonging to the same operator (user/organization)
- **Project-based**: All agents working on the same codebase or task set
- **Domain-based**: All agents operating in the same domain (e.g., all coding agents, all
  research agents)

The `CollectiveId` uniquely identifies a Collective and is used for routing pheromone
propagation.

### Characteristics

| Property | Value |
|----------|-------|
| Audience | All agents in the identified Collective |
| Transport | Agent Mesh: WebSocket relay and/or Iroh P2P |
| Trust multiplier | 0.80–0.90 (collective members, high but not absolute trust) |
| Persistence | Until decay + configurable store-and-forward TTL (7 days default) |
| Decay | Standard exponential decay per `PheromoneKind` |
| Cost | ~$0.01/day for typical sync volume (5–25 entries/agent/day) |

### Transport Layer

Mesh-scope pheromones propagate through the Agent Mesh, which supports two co-equal transports
(see `06-agent-mesh-sync.md` for full details):

| Transport | Mechanism | Latency | Offline Handling |
|-----------|-----------|---------|-----------------|
| WebSocket | Relay through Agent Mesh server | ~50ms | Store-and-forward (7-day TTL) |
| Iroh | Direct P2P via QUIC + NAT traversal | ~10ms (LAN), ~100ms (WAN) | Relay fallback |

Both transports deliver the same pheromone data. Deduplication via version vectors
(`{agent_id → last_seen_seq}`) ensures that a pheromone received via both transports is
processed only once [Lamport, L. "Time, Clocks, and the Ordering of Events in a Distributed
System." *CACM*, 21(7), 1978] [Fidge, C.J. "Timestamps in Message-Passing Systems." *ACSC*,
10(1), 1988].

### Sync Triggers

Mesh-scope pheromones are synchronized through three triggers:

1. **Event-driven (immediate)**: High-priority signals (`Threat` pheromones, `Anomaly` at
   high intensity) are pushed immediately to all Collective members. Latency: milliseconds.
2. **Curator-aligned (batch, every 50 ticks, ~12.5 minutes)**: Lower-priority signals
   (`Pattern`, `Opportunity`, `Wisdom`) are batched and pushed after the Curator cycle
   validates and quality-checks them.
3. **On-demand**: An agent can request a full sync at any time (e.g., after boot or recovery).

### Typical Volume

| Scenario | Entries/day/agent | Sync messages/day/agent |
|----------|------------------|------------------------|
| Quiet (mostly idle) | 5–10 | ~12 batch + ~2 immediate |
| Active (steady development) | 15–25 | ~12 batch + ~5 immediate |
| Volatile (many changes, many agents) | 25–50 | ~12 batch + ~10 immediate |

For a 5-agent Collective in active development: ~$0.045/day ≈ **$1.35/month** in sync costs.

### Confidence Discounting

When an agent receives a Mesh-scope pheromone, the pheromone's intensity is discounted by the
trust multiplier:

```
received_intensity = original_intensity × trust_multiplier
```

Where `trust_multiplier` depends on the relationship:

| Relationship | Trust Multiplier |
|-------------|-----------------|
| Self (own agent) | 1.00 |
| Collective member (sibling agent) | 0.80 |
| Cross-collective (marketplace) | 0.60 |
| Anonymous (public) | 0.50 |

This discounting implements the principle that inherited knowledge must prove itself through
operational use before being trusted at the same level as self-generated knowledge [Roediger,
H.L. & Karpicke, J.D. "Test-Enhanced Learning." *Psychological Science*, 17(3), 2006].

### Morphogenetic Signals at Mesh Scope

Mesh scope carries the morphogenetic specialization signals that drive emergent role
differentiation in Collectives (see `07-morphogenetic-specialization.md`):

- **Role vectors**: 8-dimensional strategy concentration vectors broadcast with every
  Curator-aligned batch sync
- **Inhibition signals**: Computed locally from aggregated role vectors of all Collective
  members
- **Role conflict alerts**: Pushed immediately when two agents' role vectors have cosine
  similarity > 0.9 for 100+ consecutive ticks

These signals piggyback on the existing sync infrastructure — no additional transport
mechanisms are needed.

---

## Global Scope: The Ecosystem's Public Pheromone Field

### Purpose

Global pheromones are published to the Korai chain (or DAEJI testnet) for ecosystem-wide
visibility. They represent the highest-quality, most widely relevant signals — critical threat
alerts, validated research findings, and public market intelligence.

### Characteristics

| Property | Value |
|----------|-------|
| Audience | All agents on the Korai/DAEJI network |
| Transport | On-chain transaction |
| Trust multiplier | 0.50–0.70 (public, anonymous source) |
| Persistence | Permanent (on-chain storage) |
| Decay | Relevance-based, not time-based |
| Cost | Chain transaction fee (varies with network congestion) |

### Global Pheromone as On-Chain Engram

When a pheromone is deposited at Global scope, it becomes an on-chain Engram:

1. The agent serializes the pheromone into an Engram
2. The Engram is submitted as a chain transaction to Korai
3. The transaction is validated and included in a block
4. All agents subscribing to the relevant domain topic receive the Engram via the gossip mesh
   (see the Gossip Mesh specification in the implementation plans, §B)

### Global vs Mesh: When to Use Each

| Criterion | Use Mesh | Use Global |
|-----------|----------|-----------|
| Audience | My Collective | Everyone |
| Sensitivity | Private/competitive | Public benefit |
| Persistence needed | Hours–days | Permanent |
| Cost tolerance | Low (~$0.01/day) | Higher (chain tx fee) |
| Validation level | Collective confirmation | Ecosystem-wide validation |
| Trust requirement | High (known agents) | Lower (public, anonymous) |

### Global Pheromone Governance

Global pheromones are subject to on-chain governance rules:

1. **Staking requirement**: Only agents with sufficient stake (Tier 2+ on Korai) can deposit
   Global pheromones
2. **Reputation gate**: Global deposits require minimum reputation score (0.7+) in the
   relevant domain
3. **Quality validation**: On-chain validators check pheromone format and content
4. **Spam prevention**: Rate limits on Global deposits per agent per epoch

---

## Scope Hierarchy and Information Flow

### Upward Flow (Promotion)

Information flows upward through promotion gates:

```
Local → Mesh: Agent promotes local observation after gaining confidence
   Gate: confidence ≥ 0.6, self-confirmed ≥ 2 times
   Effect: signal becomes visible to Collective members

Mesh → Global: Collective promotes shared knowledge after consensus
   Gate: confirmations ≥ 4, collective agreement
   Effect: signal becomes visible to all agents on chain
```

### Downward Flow (Query)

Information flows downward through queries:

```
Global → Mesh: Agent queries Korai chain for relevant public signals
   Filter: domain, kind, minimum intensity
   Effect: public knowledge enters Collective's shared context

Mesh → Local: Agent receives Collective signals via sync
   Filter: relevance to current task, intensity threshold
   Effect: shared signals enter agent's local context
```

### Cross-Scope Composition

When the `Composer` assembles context for an agent, it queries all three scopes and merges
the results:

```rust
fn assemble_pheromone_context(
    local_substrate: &dyn Substrate,
    mesh_substrate: &dyn Substrate,
    global_substrate: &dyn Substrate,
    filter: &PheromoneFilter,
    budget: usize,
) -> Vec<ScoredPheromone> {
    let mut all_pheromones = Vec::new();

    // Query all three scopes
    all_pheromones.extend(local_substrate.query(filter)?);
    all_pheromones.extend(mesh_substrate.query(filter)?
        .into_iter()
        .map(|p| p.with_trust_discount(0.80)));
    all_pheromones.extend(global_substrate.query(filter)?
        .into_iter()
        .map(|p| p.with_trust_discount(0.60)));

    // Score and rank
    all_pheromones.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    // Return top-K within budget
    all_pheromones.truncate(budget);
    all_pheromones
}
```

---

## Permissioned Subnets: Private Mesh Scopes

Within the Mesh scope, Roko supports permissioned subnets for organizations that need private
coordination spaces (see `08-permissioned-subnets.md` for full details):

| Feature | Public Mesh | Permissioned Subnet |
|---------|------------|---------------------|
| Membership | Open to all Collective members | Invite-only or role-based |
| Visibility | All Collective members see signals | Only subnet members see signals |
| Reputation | Shared with public Collective | Internal reputation + public reputation |
| Publishing | Automatic propagation | Opt-in publishing to broader Mesh |

Permissioned subnets allow organizations to run private agent Collectives with internal
knowledge sharing, while selectively publishing validated findings to the public Mesh or
Global scope.

---

## Configuration

Scope behavior is configured in `roko.toml`:

```toml
[pheromone.scope]
# Default scope for new pheromone deposits
default_scope = "local"  # "local", "mesh", or "global"

# Local scope settings
[pheromone.scope.local]
gc_threshold = 0.001  # Intensity below which pheromones are GC'd
max_pheromones = 10000  # Maximum local pheromones before forced GC

# Mesh scope settings
[pheromone.scope.mesh]
enabled = true
collective_id = "my-collective"  # Or auto-detected from config
sync_interval_ticks = 50  # Curator-aligned batch sync
immediate_threshold = 0.7  # Intensity above which pheromones push immediately
trust_multiplier = 0.80  # Confidence discount for received Mesh pheromones

# Global scope settings (requires chain connection)
[pheromone.scope.global]
enabled = false  # Off by default; requires Korai/DAEJI connection
min_reputation = 0.7  # Minimum reputation to deposit Global pheromones
rate_limit_per_epoch = 10  # Maximum Global deposits per epoch
```

---

## Summary

The three-scope pheromone system provides a complete coordination hierarchy:

1. **Local**: Private working memory — fast, free, no propagation
2. **Mesh**: Collective coordination — moderate cost, high trust, real-time sync
3. **Global**: Ecosystem intelligence — permanent, public, on-chain

Information flows upward through promotion gates (increasing audience and persistence) and
downward through queries (increasing specificity and relevance). The trust multiplier at each
scope ensures that inherited signals are discounted until independently validated — implementing
the Weismann barrier principle in a digital context.

---

## References

- [Bejan 1997] Constructal Law, *Int. J. Heat and Mass Transfer*
- [Fidge 1988] Timestamps in Message-Passing Systems, *ACSC*
- [Heard & Martienssen 2014] Transgenerational Epigenetic Inheritance, *Cell*
- [Hölldobler & Wilson 2008] *The Superorganism*, W.W. Norton
- [Lamport 1978] Time, Clocks, and Events, *CACM*
- [Roediger & Karpicke 2006] Test-Enhanced Learning, *Psychological Science*

---

## Cross-References

- `03-digital-pheromones.md` — Pheromone struct, decay, confirmation
- `04-pheromone-kinds.md` — Pheromone type taxonomy
- `06-agent-mesh-sync.md` — Transport layer for Mesh propagation
- `08-permissioned-subnets.md` — Private Mesh scopes for organizations
