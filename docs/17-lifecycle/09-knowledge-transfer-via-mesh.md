# Knowledge Transfer via Agent Mesh

> **Layer**: L4 Orchestration (multi-agent coordination) + L1 Framework (Substrate replication)
>
> **Prerequisites**: `docs/09-mesh/INDEX.md` (Agent Mesh architecture), `docs/03-neuro/INDEX.md` (Neuro store, Engram format)
>
> **Synapse traits**: Substrate (shared knowledge store queried across agents), Policy (governs sharing thresholds and publication rules), Scorer (scores shared Engrams from other agents), Gate (validates received Engrams before adoption)

---

## Overview

While backup/restore (docs 05-08) handles knowledge transfer between a deleted agent and its successor, the Agent Mesh enables **live knowledge sharing between running agents**. This is the replacement for the legacy "Clade/Styx" system — agents in the same Collective (a group of agents under common ownership or shared purpose) can exchange Engrams in real time through the Mesh relay.

The Mesh provides three knowledge sharing modes:

1. **Collective sync**: Bidirectional sync between agents in the same Collective
2. **P2P Engram sharing**: Direct agent-to-agent knowledge transfer via Mesh
3. **Public knowledge feeds**: Subscribe to curated Engram streams from other Collectives

---

## Collective Knowledge Sharing

A Collective is a group of agents organized under a common owner or purpose. In the legacy system, this was called a "Clade" — a term that carried biological lineage connotations. In Roko, a Collective is simply a coordination group with shared knowledge.

### Configuration

```toml
[mesh]
enabled = true
relay_url = "wss://mesh.roko.dev/v1/ws"
collective_id = "my-team"

[mesh.sharing]
# What to share
share_types = ["insight", "warning", "causal_link"]  # Knowledge types to share
min_share_confidence = 0.5                             # Only share above this threshold
share_on_gate_pass = true                              # Share Engrams that pass gates

# How much to trust received knowledge
received_confidence_discount = 0.7                     # Multiply incoming confidence by 0.7
max_received_per_hour = 100                            # Rate limit on incoming Engrams

# Sharing frequency
sync_interval_secs = 300                               # Sync every 5 minutes
```

### Sharing Protocol

The Collective sharing protocol uses a version-vector-based delta sync mechanism. Each agent maintains a version vector tracking the highest sequence number received from each peer (Lamport 1978, Fidge 1988):

```rust
/// Version vector for delta sync.
/// Each entry maps an agent ID to the highest sequence number
/// received from that agent.
pub type VersionVector = HashMap<String, u64>;

/// Sync delta: Engrams that the peer hasn't seen yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncDelta {
    /// Source agent ID.
    pub source_agent: String,
    /// Engrams to sync (only those with seq > peer's last_seen).
    pub engrams: Vec<SharedEngram>,
    /// Source agent's current version vector.
    pub version_vector: VersionVector,
    /// Timestamp of this sync.
    pub timestamp: u64,
}

/// An Engram packaged for sharing across the Mesh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedEngram {
    /// The Engram content and metadata.
    pub engram: BackupEngram,
    /// Sequence number (monotonically increasing per source agent).
    pub seq: u64,
    /// Sharing provenance: who shared this and when.
    pub shared_by: String,
    pub shared_at: u64,
    /// Optional: attestation (Ed25519 signature + optional ChainAttestation).
    pub attestation: Option<Attestation>,
}
```

### Bloom Filter Discovery

Before requesting full Engram content, agents exchange Bloom filters to discover which knowledge exists across the Collective. This prevents redundant transfers:

1. Agent A sends its Bloom filter (covering all Engram hashes in its Neuro store) to the Mesh relay
2. Agent B receives A's Bloom filter and checks which of its Engrams are not in A's set
3. B sends only the novel Engrams to A

This is the same mechanism used in the legacy "Styx Bloom gossip" system, now operating over the Mesh relay.

---

## Daimon-Driven Sharing Thresholds

The Daimon's PAD (Pleasure-Arousal-Dominance) state modulates sharing behavior. This replaces the legacy system where mortality pressure drove sharing — in Roko, cognitive performance drives sharing:

### Arousal-Driven Sharing

High arousal (from resource pressure, deadline proximity, or task difficulty) increases sharing frequency and lowers the confidence threshold:

```rust
/// Compute sharing threshold modulated by Daimon PAD state.
pub fn sharing_threshold(
    base_threshold: f64,
    pad: &PADVector,
) -> f64 {
    // High arousal → lower threshold (share more)
    // High dominance → higher threshold (share selectively)
    let arousal_modifier = -pad.arousal * 0.15;   // ±0.15
    let dominance_modifier = pad.dominance * 0.10;  // ±0.10

    (base_threshold + arousal_modifier + dominance_modifier).clamp(0.1, 0.9)
}
```

### Behavioral State Sharing Patterns

| Behavioral State | Sharing Behavior | Rationale |
|-----------------|-----------------|-----------|
| **Engaged** | Standard: share at base threshold | Operating normally |
| **Struggling** | Increased: lower threshold by 15% | Need help, share problems and partial findings |
| **Coasting** | Reduced: raise threshold by 10% | Low urgency, share only high-quality Engrams |
| **Exploring** | Increased: lower threshold by 20% | Discovering new knowledge, share hypotheses |
| **Focused** | Reduced: raise threshold by 15% | Deep work, avoid distraction from incoming Engrams |
| **Resting** | Minimal: share only Warnings | In Dream consolidation, not actively producing |

This table replaces the legacy mortality-phase sharing patterns (camel/lion/child). The behavioral states are cyclical and reversible, not terminal, but they produce equivalent sharing dynamics — an agent under pressure shares more, an agent at ease shares selectively.

---

## Receiving Knowledge from Mesh

When an agent receives Engrams from the Mesh, they go through the same quarantine → validate → adopt pipeline used for backup restore (see `08-selective-restore.md`), with additional Mesh-specific checks:

### Mesh Reception Pipeline

```rust
/// Process incoming Engrams from Mesh.
pub fn process_mesh_engrams(
    neuro: &mut NeuroStore,
    incoming: Vec<SharedEngram>,
    config: &MeshReceiveConfig,
) -> MeshReceiveReport {
    let mut report = MeshReceiveReport::default();

    for shared in incoming {
        // 1. Rate limiting
        if report.received_this_hour >= config.max_received_per_hour {
            report.rate_limited += 1;
            continue;
        }

        // 2. Attestation verification (if present)
        if let Some(ref attestation) = shared.attestation {
            if !verify_attestation(attestation, &shared.engram) {
                report.attestation_failed += 1;
                continue;
            }
        }

        // 3. Reputation check (if reputation system is enabled)
        if let Some(reputation) = get_agent_reputation(&shared.shared_by) {
            if reputation < config.min_sender_reputation {
                report.reputation_filtered += 1;
                continue;
            }
        }

        // 4. Apply confidence discount
        let mut engram = shared.engram;
        engram.score.confidence *= config.received_confidence_discount;

        // 5. Quarantine and validate (same as restore pipeline)
        let validation = validate_engram(&engram, neuro);
        if validation == ValidationStatus::Rejected {
            report.rejected += 1;
            continue;
        }

        // 6. Add provenance
        engram.provenance.push(ProvenanceEntry::MeshReceived {
            from_agent: shared.shared_by.clone(),
            via_collective: config.collective_id.clone(),
            timestamp: shared.shared_at,
        });

        // 7. Adopt
        neuro.insert(engram);
        report.adopted += 1;
    }

    report
}
```

### Confidence Discount

Received Engrams always have their confidence multiplied by `received_confidence_discount` (default: 0.7). This is the Mesh equivalent of generational confidence decay — knowledge from another agent is treated with appropriate skepticism until independently validated.

The legacy system used the Weismann barrier metaphor (Heard & Martienssen 2014) — somatic cells (individual experience) are separated from germ cells (inherited knowledge) by a barrier that strips most acquired marks. The same principle applies: shared knowledge arrives at reduced confidence and must be independently validated to earn higher tiers.

---

## P2P Knowledge Transfer

Beyond Collective sync, agents can share knowledge directly via peer-to-peer Mesh connections:

```rust
/// Request specific Engrams from a peer agent.
pub struct KnowledgeRequest {
    /// Target agent ID.
    pub target_agent: String,
    /// Query: what knowledge are we looking for?
    pub query: KnowledgeQuery,
    /// Maximum Engrams to receive.
    pub max_results: u32,
    /// Optional: offer to share in return (reciprocity).
    pub offer: Option<Vec<String>>,  // Engram hashes to offer
}

pub enum KnowledgeQuery {
    /// Search by keyword/topic.
    Semantic(String),
    /// Search by HDC similarity (10,240-bit BSC vector).
    HdcSimilarity { vector: Vec<u8>, threshold: f64 },
    /// Search by knowledge type.
    ByType(EngramKind),
    /// Search by tag.
    ByTag(String),
}
```

P2P transfer is useful for:
- Targeted knowledge acquisition (agent A asks agent B for specific expertise)
- Cross-domain insight resonance (agents in different domains discover structural analogies via HDC similarity)
- Cooperative task completion (agents share task-relevant knowledge during collaborative work)

---

## Four-Tier Gossip Architecture

The Mesh uses a four-tier gossip architecture for knowledge propagation:

| Tier | Protocol | Latency | Scope | Use |
|------|----------|---------|-------|-----|
| 1. GossipSub v1.1 | Publish/subscribe | Milliseconds | Immediate Collective | Hot Engrams: warnings, urgent insights |
| 2. Simulation Layer | Structured exchange | Seconds-minutes | Extended Collective | Cross-validated findings, hypothesis testing |
| 3. Aggregation Layer | TEE-protected | Per epoch | Cross-Collective | Aggregated statistics, anonymized patterns |
| 4. Canonical Event Bus | Block-finalized | Per block | All agents | Consensus knowledge, verified facts |

For chain-domain agents, Tier 4 operates on the Korai chain with 400ms block times. For non-chain agents, Tier 4 is optional and replaced by Mesh-consensus mechanisms.

---

## Stigmergy: Indirect Coordination

The Mesh supports stigmergic coordination — agents indirectly coordinate by modifying their shared knowledge environment, rather than through direct message passing. This is grounded in Grassé's observation of termite coordination (Grassé 1959): individual termites deposit pheromones that modify the environment, and subsequent termites respond to the modified environment rather than to each other.

In Roko, the "pheromones" are typed Engrams with specific decay profiles:

| Engram subtype | Decay profile | Purpose |
|---------------|--------------|---------|
| Threat | Fast (Alpha) | Immediate danger warnings |
| Opportunity | Moderate (Pattern) | Discovered opportunities |
| Wisdom | Slow (Consensus) | Validated long-term knowledge |
| Anomaly | Variable (Anomaly) | Unusual patterns requiring investigation |

Agents reading the shared Neuro space respond to the accumulated Engram patterns — a concentration of Threat Engrams in a domain triggers increased caution across the Collective, without any agent explicitly commanding the others. This is the generalized stigmergy from the legacy system, extended beyond DeFi to any domain.

---

## C-Factor: Collective Intelligence Metric

The C-Factor measures whether a Collective of agents performs better than the sum of its parts:

```
C-Factor = Collective Performance / Sum(Individual Performances)
```

A C-Factor > 1.0 indicates superlinear collective intelligence — the Collective knows more than any individual member. This metric is tracked across Mesh-connected Collectives and drives the calibration heuristic for knowledge sharing parameters.

The 31.6× Collective Calibration target (1/sqrt(N×t) heuristic, with caveats) suggests that well-coordinated Collectives can achieve substantial collective intelligence amplification — though empirical validation of these targets remains an open research question (see `docs/17-lifecycle/12-academic-foundations.md`).

---

## Permissioned Subnets

Operators can create permissioned subnets — private Collectives with restricted membership:

```toml
[mesh]
collective_id = "company-private"
collective_type = "permissioned"
# Only agents with these operator addresses can join
allowed_operators = ["0x...", "0x..."]
# Knowledge shared within this subnet is never propagated to public Mesh
private = true
```

This enables company-internal Collectives where proprietary knowledge remains within organizational boundaries. The Mesh relay enforces access control based on agent attestation and operator signatures.

---

## Related Topics

- `docs/09-mesh/INDEX.md` — Full Agent Mesh architecture
- `docs/17-lifecycle/08-selective-restore.md` — Offline knowledge transfer via backup/restore
- `docs/03-neuro/INDEX.md` — Neuro store, Engram format
- `docs/17-lifecycle/12-academic-foundations.md` — Stigmergy, collective intelligence citations
