# Audit, Witness DAG, and Forensic Replay

> Depth for [16-SECURITY.md](../../unified/16-SECURITY.md). Expresses the audit chain, witness DAG, and forensic AI as Store + lineage + Lens. The custody chain is a sequence of Signals with lineage links. The witness DAG extends lineage into a 5-vertex typed graph. Forensic replay is a Lens that walks the DAG backward to reconstruct decision context.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, lineage, content-addressing, provenance), [02-CELL](../../unified/02-CELL.md) (Observe/Lens protocol, Store protocol), [06-MEMORY](../../unified/06-MEMORY.md) (Store partitions, demurrage), [16-SECURITY](../../unified/16-SECURITY.md) (audit trail, SecurityEvent types)

---

## 1. Three Layers of Provenance

Roko's audit system has three layers of increasing richness, each building on the layer below:

| Layer | What It Records | Primitive | Durability |
|---|---|---|---|
| **Lineage** | Causal ancestry: which Signals produced this Signal | `Signal.parent_hashes: Vec<ContentHash>` | Permanent (content-addressed) |
| **Custody** | Authorization evidence: who approved this action and why | `Custody` Signal with structured fields | Permanent (zero demurrage for security events) |
| **Witness DAG** | Cognitive provenance: the full reasoning chain from observation to resolution | 5-vertex typed DAG of Signals | Permanent (DAG roots optionally anchored on-chain) |

Each layer is expressed using existing primitives. Lineage is a property of every Signal. Custody is a Signal with `Kind::Custody`. The witness DAG is a Graph of Signals with typed edges. No special machinery.

---

## 2. The Custody Chain as a Sequence of Signals

Custody records are Signals with `Kind::Custody`. Each custody Signal has lineage links to the Signals it governs. The chain forms naturally from Signal lineage -- no separate data structure required.

```rust
/// A custody record: the auditable evidence of an action.
/// This is a Signal with Kind::Custody, stored in the Store.
pub struct Custody {
    /// Hash of the action Signal this custody covers.
    pub action: ContentHash,
    /// Who initiated the action.
    pub principal: PrincipalId,
    /// When the action occurred.
    pub when: DateTime<Utc>,
    /// What authorization evidence supports this action.
    pub authorized: AuthzEvidence,
    /// Which heuristics influenced the decision.
    pub why_heuristics: Vec<ContentHash>,  // lineage links to Heuristic Signals
    /// Which claims were cited.
    pub why_claims: Vec<ContentHash>,       // lineage links to Claim Signals
    /// Pre-execution simulation result (if any).
    pub simulation: Option<ContentHash>,
    /// Which Verify Cells passed before execution.
    pub gates_passed: Vec<GateVerdict>,
    /// Taint state at the time of action.
    pub taint: Taint,
    /// Result of the action.
    pub result: Option<ContentHash>,        // lineage link to result Signal
    /// External witness anchor (if on-chain attestation enabled).
    pub witness: Option<ChainWitness>,
}

/// Authorization evidence types.
pub enum AuthzEvidence {
    /// Standing permission from the role grant.
    RoleGrant { role: String, space: SpaceId },
    /// User approved this specific action in the current session.
    SessionApproval { session_id: String, scope: String },
    /// One-shot approval for this single action.
    OneShotApproval { approval_id: String },
    /// Review confirmation for a destructive/visible action.
    ReviewConfirmation { reviewer: PrincipalId, diff_hash: ContentHash },
    /// Escalation outcome from human override.
    Escalation { escalated_by: PrincipalId, reason: String },
}

/// Attestation levels for custody strength.
pub enum AttestationLevel {
    /// Signed by the current agent session. Low friction.
    LocalAgent,
    /// Signed by a human-owned or organization-owned key.
    OrgRole,
    /// Anchored on-chain for cross-deployment verification.
    ChainWitness,
}
```

### What Requires Custody

Not every Signal needs custody. The baseline rule: if an action is destructive, externally visible, compliance-relevant, or hard to reverse, it emits a Custody Signal.

| Action Category | Custody Required | Attestation Level |
|---|---|---|
| File deletion / overwrite | Yes | LocalAgent |
| Shell execution with side effects | Yes | LocalAgent |
| Git push / PR creation | Yes | LocalAgent |
| Network egress with user data | Yes | LocalAgent |
| Chain transaction signing | Yes | OrgRole |
| Knowledge promotion to durable store | Conditional (if tainted) | LocalAgent |
| Production infrastructure write | Yes | OrgRole |
| Safety threshold modification | Yes | OrgRole |
| Configuration change | Yes | LocalAgent |

---

## 3. The Witness DAG: Five-Vertex Cognitive Provenance

The linear custody chain answers "what happened." The witness DAG answers "why it happened" by recording the full cognitive reasoning chain.

### Five Vertex Types

The witness DAG extends Signal lineage into a typed graph with five vertex kinds:

```rust
/// The five vertex types of the witness DAG.
/// Each vertex is a Signal with a specific Kind and typed edges.
pub enum WitnessVertex {
    /// Raw data ingested from the environment.
    /// Examples: market tick, file read result, API response.
    Observation {
        source: DataSource,
        taint: Taint,
        timestamp: DateTime<Utc>,
    },
    /// A forward-looking claim derived from observations.
    /// Examples: "this code has a bug," "price will increase."
    Prediction {
        confidence: f64,
        horizon: Duration,
        basis: Vec<ContentHash>,  // links to Observations
    },
    /// An action chosen based on predictions and context.
    /// Examples: "edit this file," "execute this trade."
    Decision {
        action: ContentHash,       // link to the action Signal
        custody: ContentHash,      // link to the Custody Signal
        alternatives: Vec<ContentHash>,  // rejected alternatives
        routing_scores: Vec<(ContentHash, f64)>,  // why this alternative won
    },
    /// The outcome of a Decision, verified by the Gate pipeline.
    /// Examples: "tests passed," "trade executed at price X."
    Resolution {
        decision: ContentHash,     // link to the Decision
        gate_verdicts: Vec<GateVerdict>,
        outcome: Outcome,
        reward: f64,               // continuous reward signal
    },
    /// Knowledge distilled from the Observation-to-Resolution chain
    /// and persisted in the Store.
    /// Examples: a new Heuristic, updated model routing weights.
    StoreEntry {
        resolution: ContentHash,   // link to the Resolution
        kind: SignalKind,          // Heuristic, Insight, AntiKnowledge, etc.
        demurrage_rate: f64,       // decay rate (zero for immune memory)
    },
}
```

### DAG Structure

Each vertex links to its ancestors via Signal lineage. The five vertex types create a natural flow:

```
Observation -> Prediction -> Decision -> Resolution -> StoreEntry
     |              |            |            |              |
     +--- taint ----+-- basis ---+-- custody -+-- verdicts --+-- lineage
```

The DAG is not strictly linear. A Decision may cite multiple Predictions. A Prediction may be based on multiple Observations. A Resolution may produce multiple StoreEntry Signals. The DAG captures this branching structure.

### Commitment Hashes

Every vertex carries a BLAKE3 commitment hash over its typed content. The commitment hash chain is tamper-evident: modifying any vertex changes its hash, which invalidates all downstream vertices that reference it.

```rust
/// Compute the commitment hash for a witness vertex.
/// The hash covers the vertex type, content, and all parent hashes.
pub fn commitment_hash(vertex: &WitnessVertex, parent_hashes: &[ContentHash]) -> ContentHash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(vertex.type_tag().as_bytes());
    hasher.update(&vertex.content_bytes());
    for parent in parent_hashes {
        hasher.update(parent.as_bytes());
    }
    ContentHash::from(hasher.finalize())
}
```

### On-Chain Anchoring

For deployments requiring external verifiability, DAG root hashes can be anchored on-chain. This provides non-repudiable timestamps that survive local storage manipulation.

```rust
/// Anchor a DAG root hash on-chain.
/// The ChainWitness proves the DAG existed at a specific block number.
pub struct ChainWitness {
    pub dag_root: ContentHash,
    pub chain_id: u64,
    pub block_number: u64,
    pub tx_hash: [u8; 32],
    pub timestamp: DateTime<Utc>,
}
```

---

## 4. Forensic Replay as a Lens

Forensic replay is a **Lens** (see [02-CELL.md](../../unified/02-CELL.md) SS7) -- a read-only observation Cell that walks the witness DAG backward to reconstruct the complete decision context for any past action. It does not modify any data. It produces a `ForensicReplay` Signal that is itself content-addressed and auditable.

```rust
/// Forensic replay Lens Cell.
/// Walks the witness DAG backward from an action to reconstruct
/// the complete decision context.
pub struct ForensicReplayLens;

impl Cell for ForensicReplayLens {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Observe] }
    fn name(&self) -> &str { "forensic-replay" }

    async fn execute(
        &self,
        input: Vec<Signal>,    // Contains the action hash to replay
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let action_hash = extract_action_hash(&input[0])?;

        // Step 1: Find the action Signal
        let action = ctx.store().get(&action_hash).await?;
        let timestamp = action.created_at;

        // Step 2: Find the Decision vertex in the witness DAG
        let decision = ctx.store().query(Query {
            kind: Some(Kind::WitnessDecision),
            filter: Filter::Field("action", action_hash.clone()),
            before: Some(timestamp),
        }).await?.first().cloned();

        // Step 3: Walk backward to Predictions
        let predictions = if let Some(ref d) = decision {
            walk_parents(ctx.store(), d, Kind::WitnessPrediction).await?
        } else { vec![] };

        // Step 4: Walk backward to Observations
        let observations = walk_all_parents(
            ctx.store(), &predictions, Kind::WitnessObservation
        ).await?;

        // Step 5: Find the Custody record
        let custody = ctx.store().query(Query {
            kind: Some(Kind::Custody),
            filter: Filter::Field("action", action_hash.clone()),
        }).await?.first().cloned();

        // Step 6: Find Gate verdicts (Resolution vertex)
        let resolution = ctx.store().query(Query {
            kind: Some(Kind::WitnessResolution),
            filter: Filter::Field("decision", decision_hash(&decision)),
        }).await?.first().cloned();

        // Step 7: Find any StoreEntry (learned knowledge)
        let store_entries = if let Some(ref r) = resolution {
            ctx.store().query(Query {
                kind: Some(Kind::WitnessStoreEntry),
                filter: Filter::Field("resolution", r.hash()),
            }).await?
        } else { vec![] };

        // Step 8: Reconstruct the Store state at the time
        let temporal_state = ctx.store().query(Query {
            before: Some(timestamp),
            kind: None,
            filter: Filter::None,
        }).await?;

        // Step 9: Build the replay Signal
        let replay = ForensicReplay {
            action: action_hash,
            timestamp,
            observations: observations.iter().map(|o| o.hash()).collect(),
            predictions: predictions.iter().map(|p| p.hash()).collect(),
            decision: decision.map(|d| d.hash()),
            custody: custody.map(|c| c.hash()),
            resolution: resolution.map(|r| r.hash()),
            store_entries: store_entries.iter().map(|s| s.hash()).collect(),
            store_state_size: temporal_state.len(),
            taint_at_time: extract_taint(&action),
            replay_hash: ContentHash::default(), // computed below
        };

        let replay_signal = Signal::new(Kind::ForensicReplay, replay);
        Ok(vec![replay_signal])
    }
}
```

### Replay as a Graph Walk

The replay traversal is a backward Graph walk from a target vertex (the action) through the witness DAG. The walk follows lineage links, collecting vertices by type. The result is a subgraph of the full witness DAG -- the "decision cone" that led to this specific action.

```
                     StoreEntry
                         ^
                         |
Target action <- Decision <- Resolution
                    ^   ^
                    |   |
              Prediction Prediction
                 ^   ^     ^
                 |   |     |
           Observation Observation Observation
```

The decision cone is the minimal subgraph needed to answer: "Why did this action happen, what information led to it, and who approved it?"

---

## 5. Composition with the Immune System

The forensic replay system and the immune system (see [immune-system-as-graph.md](immune-system-as-graph.md)) compose naturally:

- **Immune findings reference custody**: When the immune pipeline produces a ThreatFinding, Layer 4 (Incident Response) links it to the relevant custody record. The forensic replay of the incident walks both the immune pipeline's findings and the original action's decision cone.

- **Quarantine entries are in the DAG**: When a Signal is quarantined, the quarantine entry becomes a vertex in the witness DAG. The replay of the quarantined Signal shows both the original decision context and the immune system's intervention.

- **Immune memory patterns trace to incidents**: Each ImmunePattern (Layer 5) has an `incident_link` to the original incident. The forensic replay of a pattern match shows the full chain: original attack, detection, containment, and pattern extraction.

---

## 6. Regulatory Pre-Compliance Mapping

The three-layer provenance system maps directly to specific regulatory requirements:

### EU AI Act

| Article | Requirement | Roko Capability |
|---|---|---|
| Article 14 | Human oversight mechanisms | Custody records proving human approval; Cognitive Signals for intervention |
| Article 11 | Technical documentation and logging | Signal lineage + witness DAG |
| Article 12 | Record keeping | Store persistence with zero demurrage for security events |
| Article 13 | Transparency and information to users | Forensic replay: reconstructable decision context |

### SEC/CFTC

| Requirement | Roko Capability |
|---|---|
| Trading decision reconstruction (MiFID II) | Witness DAG: Observation -> Prediction -> Decision chain |
| Order audit trail (Rule 17a-4) | Content-addressed Signal chain with timestamps; 6-year retention via zero demurrage |
| Best execution documentation | Decision vertex with `alternatives` and `routing_scores` fields |
| Risk management documentation | Resolution vertex with `gate_verdicts` |

### HIPAA

| Requirement | Roko Capability |
|---|---|
| Audit trail for clinical decisions | Witness DAG + forensic replay |
| Access controls (who saw what PHI) | Space grants + CaMeL tags on data flows |
| Integrity controls | BLAKE3 content hashes + commitment hash chain |
| Breach notification evidence | Forensic replay reconstructs exactly what data was accessed |

### GDPR

| Requirement | Roko Capability |
|---|---|
| Right to explanation (Article 22) | Forensic replay of any automated decision |
| Processing records (Article 30) | Store persistence = complete processing log |
| Right to erasure (Article 17) | Demurrage enables controlled data expiry (but custody Signals are exempt) |

---

## What This Enables

1. **Cryptographically verifiable audit**: Every action is traceable through content-addressed Signals. Modifying any record invalidates its hash and all downstream references.
2. **Complete decision reconstruction**: The witness DAG captures not just what happened, but why -- which observations led to which predictions led to which decisions.
3. **Regulatory compliance by construction**: The audit infrastructure is not an add-on. It is a structural property of the Signal/Store system.
4. **Cross-deployment verification**: On-chain anchoring of DAG roots provides non-repudiable timestamps that survive local storage manipulation.
5. **Immune-forensic composition**: Security incidents are traceable through both the immune pipeline and the original decision context.

## Feedback Loops

- **L1**: Replay quality metrics (how completely a decision can be reconstructed) feed back into Signal emission policies. If replays are missing Observations, the system increases observation logging.
- **L2**: Custody denial patterns (actions rejected by reviewers) feed the cascade router. Agents whose actions are frequently rejected by human reviewers are routed to more conservative models.
- **L3**: Witness DAG completeness (ratio of Decisions with full Observation chains) is tracked as a system health metric.
- **Memory**: Postmortem Signals from incident response are stored as immune patterns for future recognition.

## Open Questions

1. **DAG growth**: The witness DAG grows with every observation, prediction, and decision. For a high-frequency trading agent, this could be millions of vertices per day. Storage compression (HDC fingerprints as compact vertex summaries) and DAG pruning (retaining only vertices referenced by custody records) may be needed.

2. **Replay fidelity**: When replaying a decision, the Store state at the time is reconstructed via temporal query. But the exact model weights, prompt template versions, and routing parameters may have changed since the decision. Should replays use snapshot versioning of all system parameters?

3. **Multi-agent DAGs**: In a Group (multiple cooperating agents), the witness DAG spans multiple agents. A Decision by Agent A may be based on a Prediction from Agent B. How does the DAG represent cross-agent provenance without leaking private knowledge between agent Spaces?

4. **ZK proofs over the DAG**: For regulated environments, can a deployment prove compliance properties over the witness DAG without revealing the full decision context? ZK proofs over DAG subgraphs would enable "prove that human review occurred" without revealing the reviewed content.

## Implementation Tasks

| Task | File | What |
|---|---|---|
| Add Custody Signal emission | `crates/roko-cli/src/orchestrate.rs` | Emit Custody Signals for destructive/visible actions in the orchestration loop |
| Implement witness vertex types | `crates/roko-core/src/signal.rs` | Add `Kind::WitnessObservation`, `WitnessPrediction`, `WitnessDecision`, `WitnessResolution`, `WitnessStoreEntry` |
| Implement commitment hash chain | `crates/roko-core/src/signal.rs` | Add `commitment_hash()` function with BLAKE3 over vertex content + parent hashes |
| Implement ForensicReplayLens | `crates/roko-core/src/` | Add Lens Cell that walks the witness DAG backward |
| Add temporal query to Store | `crates/roko-fs/src/` | Support `before: Option<DateTime>` filter on Store queries |
| Wire witness vertices into orchestrate.rs | `crates/roko-cli/src/orchestrate.rs` | Emit Observation/Prediction/Decision/Resolution vertices at appropriate points |
| Integration test: replay of a plan execution | `crates/roko-cli/tests/` | Execute a plan, then replay a specific task's decision to verify completeness |
| Add on-chain anchoring (Phase 2+) | `crates/roko-chain/` | Implement `ChainWitness` with DAG root hash commitment |
