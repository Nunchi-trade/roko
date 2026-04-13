# Witness DAG: Cryptographic Cognitive Traces

> **Layer**: L3 Harness (audit chain extension), L4 Orchestration (cross-agent verification)
>
> **Crate**: Target: `roko-gate` (extension of existing audit chain), integration with `roko-fs` (SQLite storage)
>
> **Synapse traits**: `Substrate` (persist vertices), `Gate` (verify commitment hashes), `Policy` (emit DAG violation Engrams)
>
> **Prerequisites**: [02-audit-chain.md](02-audit-chain.md), [11-temporal-logic.md](11-temporal-logic.md)


> **Implementation**: Specified

---

## Overview

The existing audit chain in Roko is a linear Merkle hash-chain: each decision is hashed, each hash commits to the previous one. This chain proves that events happened in a particular sequence. It cannot prove **why** those events happened. A linear chain records that the agent swapped ETH for USDC at block 19,412,003. It says nothing about the three observations that suggested a regime change, the two predictions that confirmed it, or the Gate that approved the trade.

The **Witness DAG** extends the linear audit chain into a directed acyclic graph that links every observation, prediction, decision, and outcome into a tamper-proof chain of reasoning. Any learned knowledge in the Neuro store (Roko's knowledge management subsystem, `roko-neuro`) traces backward through the DAG to the raw observations that justify it. The linear audit chain becomes a degenerate path through the DAG — backward compatibility is preserved.

This document specifies the Witness DAG's mathematical foundations, data structures, zero-knowledge proof capabilities, storage model, and integration with the rest of the Roko architecture.

---

## The Problem: Four Gaps in the Linear Chain

### Gap 1: No Reasoning Provenance

The audit chain records that the agent performed an action. It does not record which observations, predictions, and risk assessments led to that action. Post-mortem analysis can determine *what* happened but not *why* the agent thought it was appropriate.

In a coding domain: the chain records that the agent modified `auth.rs` and added a new endpoint. It does not record which code intelligence signals, which test failures, which PRD requirements, or which Neuro entries informed that implementation choice.

### Gap 2: No Knowledge Provenance

The Neuro store (Roko's persistent knowledge system, `roko-neuro`) contains entries such as "high code churn in authentication modules correlates with security vulnerabilities" or "momentum strategies fail in range-bound markets." Which episodes taught these lessons? How many observations support them? There is no way to trace a Neuro entry back to its evidential basis without the DAG.

### Gap 3: Trust Requires Reputation

When agents in a Collective (a group of cooperating agents on the Korai network) establish trust, they rely on reputation scores and attestations. Reputation is backward-looking and gameable: an agent can build reputation through conservative behavior, then exploit that trust. **Verifiable reasoning quality** — cryptographic proof that decisions were grounded in real observations — would be a stronger trust signal than reputation alone.

### Gap 4: Auditing Requires Revelation

Stakeholders (depositors, operators, regulators) want to audit decision quality. Today this requires revealing the strategy itself. There is no way to prove "my decisions were well-reasoned" without showing the reasoning — unless zero-knowledge proofs are used (see §Zero-Knowledge Proofs below).

---

## Mathematical Foundations

### DAG Structure

A Witness DAG is a directed acyclic graph **W = (V, E)** where:

**Vertices V:** Every cognitive event produces a vertex. Five vertex types map to steps of the Universal Cognitive Loop (the 9-step Synapse Loop that every Roko agent runs at its own timescale):

| Type | Label | Created at Loop Step | Description |
|---|---|---|---|
| Observation | O | Step 1 (PERCEIVE) | Raw perceptual data: price feeds, on-chain events, code intelligence signals, test results, file changes |
| Prediction | P | Step 3 (ATTEND) | Forecasts derived from observations: "this refactoring will break 3 tests" or "ETH will decline 3% in 10 ticks" |
| Decision | D | Steps 4-6 (INTEGRATE, ACT, VERIFY) | Actions chosen based on predictions: "modify auth.rs" or "swap 10 ETH for USDC" |
| Resolution | R | Step 8 (ADAPT) | Observed outcomes: "compilation succeeded, 2 tests failed" or "swap executed at 3,201" |
| NeuroEntry | G | Step 9 (META-COGNIZE) | Learned knowledge: "this pattern of test failure indicates missing error handling" |

Note: The legacy specification used "GrimoireEntry" for the fifth vertex type. In the current architecture, Grimoire has been renamed to Neuro (`roko-neuro`), so this type is "NeuroEntry" — a learned knowledge entry persisted in the Neuro store.

**Edges E:** Directed edges encode "was used to produce." Direction points from input to output:

- **O → P**: "Observation O was used to generate prediction P."
- **P → D**: "Prediction P informed decision D."
- **D → R**: "Decision D produced resolution R."
- **P → G**: "Prediction P contributed to Neuro entry G."
- **R → G**: "Resolution R contributed to Neuro entry G."
- **G → P**: "Neuro entry G influenced prediction P." (Knowledge feedback loop.)

### Cryptographic Commitment

Each vertex carries two hashes:

**Content hash** — commits to the vertex's data. Two vertices with identical content produce identical content hashes:

```
h(v) = BLAKE3(type || timestamp || content(v))
```

**Commitment hash** — commits to both the vertex's content and its entire ancestry. This is the Merkle property: modifying any vertex invalidates the commitment hashes of all its descendants:

```
c(v) = BLAKE3(h(v) || c(parent_1) || c(parent_2) || ... || c(parent_n))
```

Parent commitment hashes are sorted lexicographically before hashing to ensure deterministic commitment regardless of edge insertion order.

**Why BLAKE3:** BLAKE3 is chosen over SHA-256 for witness hashing: 3-5x faster on modern hardware, tree-based structure enables incremental hashing of event streams, and 256-bit output provides equivalent collision resistance. This choice is consistent with the Engram's `ContentHash` which also uses BLAKE3 (see the Engram struct in `roko-core`).

### Tamper Evidence

The commitment hash `c(v)` of any vertex commits to the entire subgraph that produced it. If an attacker modifies observation O_17 that was used to generate prediction P_8, then:

1. `c(O_17)` changes because the content changed
2. `c(P_8)` changes because it includes `c(O_17)` as a parent
3. Every decision, resolution, and Neuro entry downstream of P_8 has an invalid commitment hash

The root hash (most recent vertex, or a synthetic root committing to all leaves) summarizes the entire reasoning history. Publishing this root to an external system (a blockchain such as Korai, a timestamping service) creates a non-repudiable commitment to the complete reasoning chain.

The existing linear audit chain is a special case: if every vertex has exactly one parent and the only vertex type is D, the Witness DAG reduces to a linear hash chain. The DAG is a strict generalization.

### Hallucination vs. Memory Detection

The DAG enables distinguishing two failure modes that look identical in a linear chain:

**Hallucination.** A decision D has no observation vertices in its provenance subgraph. The agent made a decision based on fabricated or injected data rather than real observations. The DAG detects this: `observation_provenance(D)` returns an empty set.

**Memory corruption.** A Neuro entry G has valid observation provenance, but the commitment hashes in the chain are invalid. Something tampered with the reasoning chain after the fact. The DAG detects this: `verify(G)` returns false.

**Stale knowledge.** A Neuro entry G has valid provenance, but the observations in its chain are all older than a threshold T. The knowledge is grounded but may be outdated. The DAG quantifies this: `max_observation_age(G)` returns the age of the oldest supporting observation. This integrates with the Neuro tier system — entries whose provenance is entirely stale may be demoted from Consolidated tier to Working tier.

---

## Core Data Structures

### Vertex Type and Vertex

```rust
use blake3::Hash;
use std::sync::Arc;

/// The five types of cognitive event that produce DAG vertices.
/// Maps directly to steps in the Universal Cognitive Loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VertexType {
    /// Raw perceptual data from step 1 (PERCEIVE).
    Observation = 0,
    /// Forecasts from step 3 (ATTEND).
    Prediction = 1,
    /// Actions from steps 4-6 (INTEGRATE/ACT/VERIFY).
    Decision = 2,
    /// Observed outcomes from step 8 (ADAPT).
    Resolution = 3,
    /// Learned knowledge from step 9 (META-COGNIZE).
    /// Formerly "GrimoireEntry" — now NeuroEntry per naming map.
    NeuroEntry = 4,
}

/// A single vertex in the Witness DAG.
#[derive(Debug, Clone)]
pub struct Vertex {
    /// BLAKE3(content_hash || sorted parent commitment hashes).
    /// Commits to the vertex AND its entire ancestry.
    pub commitment_hash: Hash,
    /// BLAKE3(type || timestamp || content).
    /// Commits to the vertex's data only.
    pub content_hash: Hash,
    /// What kind of cognitive event this vertex represents.
    pub vertex_type: VertexType,
    /// When this vertex was created, in milliseconds since epoch.
    pub timestamp_ms: u64,
    /// Serialized content of the cognitive event.
    pub content: Vec<u8>,
    /// Commitment hashes of parent vertices, sorted lexicographically.
    pub parent_hashes: Vec<Hash>,
}

impl Vertex {
    /// Create a new vertex and compute both hashes.
    pub fn new(
        vertex_type: VertexType,
        timestamp_ms: u64,
        content: Vec<u8>,
        parent_hashes: Vec<Hash>,
    ) -> Self {
        // Content hash: H(type || timestamp || content)
        let content_hash = {
            let mut hasher = blake3::Hasher::new();
            hasher.update(&[vertex_type as u8]);
            hasher.update(&timestamp_ms.to_le_bytes());
            hasher.update(&content);
            hasher.finalize()
        };

        // Sort parent hashes for deterministic commitment
        let mut sorted_parents = parent_hashes;
        sorted_parents.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        // Commitment hash: H(content_hash || parent_1 || parent_2 || ...)
        let commitment_hash = {
            let mut hasher = blake3::Hasher::new();
            hasher.update(content_hash.as_bytes());
            for parent in &sorted_parents {
                hasher.update(parent.as_bytes());
            }
            hasher.finalize()
        };

        Self {
            commitment_hash,
            content_hash,
            vertex_type,
            timestamp_ms,
            content,
            parent_hashes: sorted_parents,
        }
    }
}
```

### The WitnessDAG Struct

```rust
use dashmap::DashMap;
use std::sync::atomic::{AtomicU32, Ordering};

/// The Witness DAG: a content-addressed, append-only DAG of cognitive events.
/// Thread-safe via DashMap (sharded concurrent HashMap) and parking_lot::RwLock.
pub struct WitnessDAG {
    /// All vertices, indexed by commitment hash.
    vertices: DashMap<Hash, Arc<Vertex>>,
    /// Forward edges: parent -> set of children.
    children: DashMap<Hash, Vec<Hash>>,
    /// The commitment hash of the most recently added vertex.
    latest: parking_lot::RwLock<Option<Hash>>,
    /// Maximum depth of any path in the DAG.
    /// Exposed as a signal for Collective peers to read.
    pub dag_depth: AtomicU32,
}

impl WitnessDAG {
    pub fn new() -> Self {
        Self {
            vertices: DashMap::new(),
            children: DashMap::new(),
            latest: parking_lot::RwLock::new(None),
            dag_depth: AtomicU32::new(0),
        }
    }

    /// Append a vertex to the DAG. O(1) amortized.
    pub fn append(&self, vertex: Vertex) -> Hash {
        let hash = vertex.commitment_hash;

        // Register forward edges from each parent to this vertex.
        for parent in &vertex.parent_hashes {
            self.children
                .entry(*parent)
                .or_insert_with(Vec::new)
                .push(hash);
        }

        // Update depth: max(parent depths) + 1.
        let depth = vertex
            .parent_hashes
            .iter()
            .filter_map(|p| self.vertices.get(p))
            .map(|v| self.vertex_depth(&v.commitment_hash))
            .max()
            .unwrap_or(0)
            + 1;

        let current_max = self.dag_depth.load(Ordering::Relaxed);
        if depth > current_max {
            self.dag_depth.store(depth, Ordering::Relaxed);
        }

        self.vertices.insert(hash, Arc::new(vertex));
        *self.latest.write() = Some(hash);

        hash
    }

    /// Walk the DAG backward from a vertex, collecting all ancestors.
    /// Used for provenance queries. BFS traversal.
    pub fn provenance(&self, start: &Hash) -> Vec<Arc<Vertex>> {
        let mut visited = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        let mut result = Vec::new();

        queue.push_back(*start);

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(vertex) = self.vertices.get(&current) {
                for parent in &vertex.parent_hashes {
                    queue.push_back(*parent);
                }
                result.push(Arc::clone(&vertex));
            }
        }

        result
    }

    /// Verify the integrity of a vertex: recompute its commitment hash
    /// and check that it matches the stored value.
    pub fn verify(&self, hash: &Hash) -> bool {
        let vertex = match self.vertices.get(hash) {
            Some(v) => v.clone(),
            None => return false,
        };

        // Recompute content hash
        let expected_content = {
            let mut hasher = blake3::Hasher::new();
            hasher.update(&[vertex.vertex_type as u8]);
            hasher.update(&vertex.timestamp_ms.to_le_bytes());
            hasher.update(&vertex.content);
            hasher.finalize()
        };

        if expected_content != vertex.content_hash {
            return false;
        }

        // Recompute commitment hash
        let expected_commitment = {
            let mut hasher = blake3::Hasher::new();
            hasher.update(vertex.content_hash.as_bytes());
            for parent in &vertex.parent_hashes {
                hasher.update(parent.as_bytes());
            }
            hasher.finalize()
        };

        expected_commitment == vertex.commitment_hash
    }

    /// Find all observation vertices that support a given vertex.
    pub fn observation_provenance(&self, root: &Hash) -> Vec<Arc<Vertex>> {
        self.provenance(root)
            .into_iter()
            .filter(|v| v.vertex_type == VertexType::Observation)
            .collect()
    }

    /// Find all prediction-resolution pairs in the provenance of a vertex.
    /// Used for computing prediction accuracy over a reasoning chain.
    pub fn prediction_resolution_pairs(
        &self,
        root: &Hash,
    ) -> Vec<(Arc<Vertex>, Arc<Vertex>)> {
        let ancestors = self.provenance(root);
        let ancestor_set: std::collections::HashSet<Hash> =
            ancestors.iter().map(|v| v.commitment_hash).collect();

        let mut pairs = Vec::new();

        for vertex in &ancestors {
            if vertex.vertex_type != VertexType::Prediction {
                continue;
            }

            if let Some(child_hashes) = self.children.get(&vertex.commitment_hash) {
                for child_hash in child_hashes.iter() {
                    if ancestor_set.contains(child_hash) {
                        if let Some(child) = self.vertices.get(child_hash) {
                            if child.vertex_type == VertexType::Resolution {
                                pairs.push((Arc::clone(vertex), Arc::clone(&child)));
                            }
                        }
                    }
                }
            }
        }

        pairs
    }

    fn vertex_depth(&self, hash: &Hash) -> u32 {
        let vertex = match self.vertices.get(hash) {
            Some(v) => v.clone(),
            None => return 0,
        };
        if vertex.parent_hashes.is_empty() {
            return 1;
        }
        vertex
            .parent_hashes
            .iter()
            .map(|p| self.vertex_depth(p))
            .max()
            .unwrap_or(0)
            + 1
    }
}
```

---

## Integration with the Universal Cognitive Loop

The Witness DAG is constructed incrementally as the 9-step Synapse Loop executes:

| Loop Step | Step Name | DAG Action |
|---|---|---|
| 1 | PERCEIVE | Create O vertices for each observation. No parents (these are roots). |
| 2 | EVALUATE | No new vertices. Scoring metadata attached to existing vertices. |
| 3 | ATTEND | Create P vertices. Edges from O vertices used and any G (Neuro) entries consulted. |
| 4 | INTEGRATE | Create D vertex if the Composer decides on an action. Edges from P vertices. |
| 5 | ACT | Update D vertex with execution results. No new vertices. |
| 6 | VERIFY | Finalize D vertex. Commitment hash computed at this point via Gate verification. |
| 7 | PERSIST | Store vertex in Substrate. Execution record linked to D. |
| 8 | ADAPT | Create R vertices for each resolution. Edges from D. |
| 9 | META-COGNIZE | Create G (NeuroEntry) vertices for new knowledge. Edges from relevant P, R, D vertices. |

Each vertex is an Engram — it carries the full Engram metadata (kind, body, tags, score, lineage, provenance). The DAG's commitment hash chain is orthogonal to and consistent with the Engram's own `ContentHash` (which is `BLAKE3(kind + body + author + tags)` per the Engram spec).

---

## Zero-Knowledge Proofs for Strategy Auditing

Using ZK-SNARKs or ZK-STARKs, an agent can prove statements about its DAG structure without revealing DAG contents. Four proof types address the four gaps identified above:

### Proof 1: Decision Grounding

"This decision was based on at least N observations and M predictions."

The prover demonstrates that the subgraph rooted at decision D_i contains at least N observation vertices and M prediction vertices, all with valid commitment hashes. The verifier learns the branching factor but not the content of any vertex. This addresses **Gap 1** (no reasoning provenance) — stakeholders verify decision quality without seeing the strategy.

### Proof 2: Knowledge Provenance

"This Neuro entry traces back to at least K direct observations."

The prover walks the DAG backward from NeuroEntry G_j and proves that the reachable subgraph contains at least K observation vertices. The verifier learns evidential depth but not the observations themselves. This addresses **Gap 2** (no knowledge provenance) — the strength of a knowledge entry is verifiable without revealing what the agent learned.

### Proof 3: Prediction Accuracy

"My prediction accuracy over the last T ticks exceeds X%."

The prover identifies all prediction-resolution pairs in a time window, computes accuracy, and proves the result exceeds the threshold. The verifier learns the accuracy percentage but not individual predictions or resolutions. This addresses **Gap 3** (trust requires reputation) — trust becomes proportional to verifiable prediction quality.

### Proof 4: Reasoning Consistency

"All commitment hashes in the subgraph rooted at vertex V are valid."

Proves the reasoning chain has not been tampered with, without revealing the chain itself. This addresses **Gap 4** (auditing requires revelation).

**Implementation note:** ZK proof generation is O(circuit_size). A grounding proof for a typical decision with 10-20 parent vertices takes 1-5 seconds using plonky2. Too slow for real-time, acceptable for on-demand auditing. Full ZK integration is deferred to Tier 4 in the implementation roadmap (see `refactoring-prd/07-implementation-priorities.md`).

---

## SQLite Storage Model

The DAG is stored in SQLite with two tables:

```sql
CREATE TABLE vertices (
    hash        BLOB PRIMARY KEY,  -- 32-byte BLAKE3 commitment hash
    content_hash BLOB NOT NULL,    -- 32-byte BLAKE3 content hash
    vertex_type INTEGER NOT NULL,  -- 0=O, 1=P, 2=D, 3=R, 4=G
    timestamp   INTEGER NOT NULL,  -- Unix timestamp in milliseconds
    content     BLOB NOT NULL,     -- Serialized vertex data
    pruned      INTEGER DEFAULT 0  -- 1 if content has been pruned
);

CREATE TABLE edges (
    parent_hash BLOB NOT NULL,
    child_hash  BLOB NOT NULL,
    PRIMARY KEY (parent_hash, child_hash),
    FOREIGN KEY (parent_hash) REFERENCES vertices(hash),
    FOREIGN KEY (child_hash) REFERENCES vertices(hash)
);

CREATE INDEX idx_edges_child ON edges(child_hash);
CREATE INDEX idx_vertices_type ON vertices(vertex_type, timestamp);
```

This integrates with `roko-fs`, which already provides `FileSubstrate` (JSONL-based Engram persistence). The Witness DAG's SQLite storage is a complementary persistence layer — Engrams are stored in JSONL via FileSubstrate for linear queries, and in the DAG's SQLite for graph queries.

---

## SQLite schema: complete definition

```sql
-- Core vertex storage.
CREATE TABLE vertices (
    hash         BLOB PRIMARY KEY,    -- 32-byte BLAKE3 commitment hash
    content_hash BLOB NOT NULL,       -- 32-byte BLAKE3 content hash
    vertex_type  INTEGER NOT NULL,    -- 0=Observation, 1=Prediction, 2=Decision,
                                      -- 3=Resolution, 4=NeuroEntry
    timestamp    INTEGER NOT NULL,    -- Unix timestamp in milliseconds
    content      BLOB NOT NULL,       -- Serialized vertex data (MessagePack)
    depth        INTEGER NOT NULL DEFAULT 1, -- DAG depth at this vertex
    pruned       INTEGER NOT NULL DEFAULT 0, -- 1 if content has been replaced by summary
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Directed edges: parent -> child.
CREATE TABLE edges (
    parent_hash  BLOB NOT NULL,
    child_hash   BLOB NOT NULL,
    edge_type    INTEGER NOT NULL DEFAULT 0, -- 0=data_flow, 1=knowledge_feedback
    PRIMARY KEY (parent_hash, child_hash),
    FOREIGN KEY (parent_hash) REFERENCES vertices(hash),
    FOREIGN KEY (child_hash)  REFERENCES vertices(hash)
);

-- Indexes for common query patterns.
CREATE INDEX idx_edges_child     ON edges(child_hash);
CREATE INDEX idx_vertices_type   ON vertices(vertex_type, timestamp);
CREATE INDEX idx_vertices_depth  ON vertices(depth);
CREATE INDEX idx_vertices_pruned ON vertices(pruned, timestamp);

-- Summary vertices replace pruned subtrees.
CREATE TABLE summaries (
    root_hash       BLOB PRIMARY KEY,   -- Commitment hash of the pruned subtree root
    vertex_count    INTEGER NOT NULL,    -- Total vertices in the pruned subtree
    obs_count       INTEGER NOT NULL,    -- Observation vertices in the subtree
    pred_count      INTEGER NOT NULL,    -- Prediction vertices
    decision_count  INTEGER NOT NULL,    -- Decision vertices
    resolution_count INTEGER NOT NULL,   -- Resolution vertices
    neuro_count     INTEGER NOT NULL,    -- NeuroEntry vertices
    pred_accuracy   REAL,                -- Prediction accuracy over the subtree
    neuro_hashes    BLOB,                -- MessagePack-encoded Vec<Hash> of NeuroEntry
                                         -- vertices whose provenance passes through
    compressed_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

-- On-chain anchor records.
CREATE TABLE anchors (
    anchor_id    INTEGER PRIMARY KEY AUTOINCREMENT,
    dag_root     BLOB NOT NULL,         -- 32-byte BLAKE3 root hash at time of anchor
    tick_number  INTEGER NOT NULL,      -- Tick number when anchored
    tx_hash      BLOB,                  -- On-chain transaction hash (null if not yet confirmed)
    chain_id     INTEGER NOT NULL DEFAULT 1, -- Chain ID (1=Korai mainnet)
    anchored_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_anchors_tick ON anchors(tick_number);
```

---

## DAG construction: integration into the 9-step Synapse Loop

Each step of the Synapse Loop calls into `WitnessDAG::append()` at specific points. The integration happens in `orchestrate.rs` as part of the per-task execution path.

```rust
/// Pseudocode for DAG vertex creation during one Synapse Loop iteration.
fn synapse_loop_tick(dag: &WitnessDAG, /* ... */) {
    // Step 1: PERCEIVE -- create Observation vertices (roots).
    let observations: Vec<Hash> = perceive()
        .into_iter()
        .map(|obs| {
            let v = Vertex::new(
                VertexType::Observation,
                now_ms(),
                serialize(&obs),
                vec![], // no parents: observations are roots
            );
            dag.append(v)
        })
        .collect();

    // Step 2: EVALUATE -- no new vertices. Scoring metadata
    // is attached to existing observation Engrams.

    // Step 3: ATTEND -- create Prediction vertices.
    let neuro_entries_used = retrieve_relevant_neuro();
    let neuro_hashes: Vec<Hash> = neuro_entries_used
        .iter()
        .map(|g| g.commitment_hash)
        .collect();

    let predictions: Vec<Hash> = attend(&observations, &neuro_entries_used)
        .into_iter()
        .map(|pred| {
            // Parents: observations used + neuro entries consulted.
            let mut parents = observations.clone();
            parents.extend(neuro_hashes.clone());
            let v = Vertex::new(
                VertexType::Prediction,
                now_ms(),
                serialize(&pred),
                parents,
            );
            dag.append(v)
        })
        .collect();

    // Steps 4-6: INTEGRATE/ACT/VERIFY -- create Decision vertex.
    if let Some(action) = compose(&predictions) {
        let decision_hash = dag.append(Vertex::new(
            VertexType::Decision,
            now_ms(),
            serialize(&action),
            predictions.clone(), // parents: predictions that informed this decision
        ));

        // Step 5: ACT -- execute the action.
        let outcome = execute(action);

        // Step 6: VERIFY -- Gate pipeline runs here.
        let verdict = gate_pipeline.verify(&outcome).await;

        // Step 7: PERSIST -- store in Substrate.
        substrate.write(&outcome);

        // Step 8: ADAPT -- create Resolution vertex.
        let resolution_hash = dag.append(Vertex::new(
            VertexType::Resolution,
            now_ms(),
            serialize(&verdict),
            vec![decision_hash], // parent: the decision that produced this outcome
        ));

        // Step 9: META-COGNIZE -- create NeuroEntry if lesson learned.
        if let Some(lesson) = meta_cognize(&verdict) {
            dag.append(Vertex::new(
                VertexType::NeuroEntry,
                now_ms(),
                serialize(&lesson),
                // Parents: predictions, resolutions, and decisions that
                // contributed to this learned knowledge.
                vec![decision_hash, resolution_hash]
                    .into_iter()
                    .chain(predictions.iter().copied())
                    .collect(),
            ));
        }
    }
}
```

---

## Pruning and Compression

The full DAG grows linearly with ticks. Each tick produces 5-20 vertices. At one tick per ~10 seconds (gamma speed), that is ~8,640 ticks per day, or 43,000-172,000 vertices per day.

### Three Pruning Strategies

**Rolling window.** Keep the full DAG for the last T ticks (default: 7 days, ~604,800 ticks). All vertices within the window retain full content and are queryable.

**Compression beyond the window.** For vertices older than T, replace subtrees with summary vertices. A summary vertex contains:
- Root commitment hash of the replaced subtree (preserving the Merkle property)
- Aggregate statistics: vertex count by type, prediction accuracy, Neuro entries produced
- Commitment hashes of any Neuro entries whose provenance chains pass through the subtree

**Neuro provenance preservation.** Even after supporting observations are pruned, the hashes in the DAG serve as existence proofs. The provenance chain from a Neuro entry to its observations remains verifiable (hashes match), even though observation content has been discarded. This means knowledge stays grounded even after the raw evidence is compressed.

### Storage Estimates

- ~200 bytes per vertex average
- At 100,000 vertices per day, the live DAG consumes ~20 MB/day
- A 7-day rolling window is ~140 MB
- After compression, historical data adds ~1 MB/day
- One year of compressed history: ~365 MB

### Pruning algorithm (pseudocode)

```
prune_dag(dag: &WitnessDAG, db: &SqliteDb, config: &PruneConfig):
    cutoff = now_ms() - config.rolling_window_ms  // default: 7 days

    # Step 1: Identify vertices outside the rolling window.
    old_vertices = db.query(
        "SELECT hash, vertex_type FROM vertices
         WHERE timestamp < ? AND pruned = 0",
        [cutoff]
    )

    # Step 2: Group into subtrees by finding connected components
    # among old vertices. Each component becomes one summary.
    components = find_connected_components(old_vertices, db)

    for component in components:
        # Step 3: Check Neuro provenance preservation.
        # NeuroEntry vertices whose provenance passes through
        # this component retain their hash chain.
        neuro_hashes = []
        for vertex in component:
            children = db.query(
                "SELECT child_hash FROM edges WHERE parent_hash = ?",
                [vertex.hash]
            )
            for child in children:
                if child.vertex_type == NeuroEntry && child.timestamp >= cutoff:
                    neuro_hashes.push(child.hash)

        # Step 4: Compute aggregate statistics.
        summary = Summary {
            root_hash: component.root().commitment_hash,
            vertex_count: component.len(),
            obs_count: component.count_type(Observation),
            pred_count: component.count_type(Prediction),
            decision_count: component.count_type(Decision),
            resolution_count: component.count_type(Resolution),
            neuro_count: component.count_type(NeuroEntry),
            pred_accuracy: compute_pred_accuracy(component),
            neuro_hashes: neuro_hashes,
        }

        # Step 5: Replace content with summary, preserve hashes.
        db.insert_summary(summary)
        for vertex in component:
            db.execute(
                "UPDATE vertices SET pruned = 1, content = X''
                 WHERE hash = ?",
                [vertex.hash]
            )
            // Edges are preserved -- hash chain remains verifiable.
```

**Configuration:**

```toml
[safety.witness_dag]
rolling_window_days = 7          # Full DAG retention. Range: 1..90.
prune_interval_hours = 6         # How often pruning runs. Range: 1..24.
anchor_interval_ticks = 720      # Ticks between on-chain anchors. Range: 100..10000.
max_vertices_in_memory = 500000  # When exceeded, evict oldest to SQLite. Range: 10000..5000000.
sqlite_wal_mode = true           # Use WAL mode for concurrent reads.
```

### ZK proof generation: plonky2 integration path

ZK proofs run off the hot path -- they are generated on demand for auditing, not for real-time verification. The integration uses plonky2 (Polygon's recursive SNARK library) as the backend.

```rust
use plonky2::field::goldilocks_field::GoldilocksField;
use plonky2::plonk::circuit_builder::CircuitBuilder;
use plonky2::plonk::config::PoseidonGoldilocksConfig;

type F = GoldilocksField;
type C = PoseidonGoldilocksConfig;

/// ZK proof types supported by the Witness DAG.
pub enum WitnessProofType {
    /// Prove decision D was based on >= N observations and >= M predictions.
    DecisionGrounding {
        decision_hash: blake3::Hash,
        min_observations: u32,
        min_predictions: u32,
    },
    /// Prove NeuroEntry traces back to >= K observations.
    KnowledgeProvenance {
        neuro_hash: blake3::Hash,
        min_observations: u32,
    },
    /// Prove prediction accuracy >= X% over time window.
    PredictionAccuracy {
        start_tick: u64,
        end_tick: u64,
        min_accuracy_pct: u32,
    },
    /// Prove all commitment hashes in subtree are valid.
    ReasoningConsistency {
        root_hash: blake3::Hash,
    },
}

/// Generate a ZK proof for a WitnessProofType.
///
/// Returns the serialized proof (typically 100-500 bytes for plonky2).
/// Generation time: 1-5 seconds for typical subgraphs (10-20 vertices).
pub fn generate_proof(
    dag: &WitnessDAG,
    proof_type: WitnessProofType,
) -> anyhow::Result<Vec<u8>> {
    match proof_type {
        WitnessProofType::DecisionGrounding {
            decision_hash,
            min_observations,
            min_predictions,
        } => {
            // Walk DAG backward from decision, count vertex types.
            let ancestors = dag.provenance(&decision_hash);
            let obs_count = ancestors
                .iter()
                .filter(|v| v.vertex_type == VertexType::Observation)
                .count() as u32;
            let pred_count = ancestors
                .iter()
                .filter(|v| v.vertex_type == VertexType::Prediction)
                .count() as u32;

            // Build circuit: prove obs_count >= min and pred_count >= min
            // without revealing the actual vertices.
            let config = plonky2::plonk::circuit_data::CircuitConfig::standard_recursion_config();
            let mut builder = CircuitBuilder::<F, 2>::new(config);

            // Circuit wires commitment hash chain verification
            // and vertex type counting into a single proof.
            // (Full circuit construction omitted for brevity --
            //  the circuit has ~500 gates for a 20-vertex subgraph.)

            let data = builder.build::<C>();
            let proof = data.prove(/* witness */)?;
            Ok(proof.to_bytes())
        }
        // Other proof types follow the same pattern:
        // walk DAG, build circuit, generate proof.
        _ => todo!("Implement remaining proof types"),
    }
}
```

**Performance estimates:**

| Proof type | Subgraph size | Circuit gates | Generation time | Proof size |
|-----------|--------------|---------------|----------------|------------|
| DecisionGrounding | 10-20 vertices | ~500 | 1-2 sec | ~200 bytes |
| KnowledgeProvenance | 20-50 vertices | ~1,200 | 2-4 sec | ~300 bytes |
| PredictionAccuracy | 100-500 pairs | ~5,000 | 4-8 sec | ~400 bytes |
| ReasoningConsistency | 50-200 vertices | ~2,000 | 3-5 sec | ~350 bytes |

### Test criteria

- `WitnessDAG::append()` produces deterministic commitment hashes (same inputs, same hash)
- `WitnessDAG::verify()` returns false if any byte in content is modified after insertion
- `WitnessDAG::provenance()` returns the complete ancestor set via BFS
- `WitnessDAG::observation_provenance()` filters to only Observation vertices
- `WitnessDAG::prediction_resolution_pairs()` pairs predictions with their resolutions correctly
- SQLite schema supports concurrent reads under WAL mode
- Pruning preserves hash chain integrity: `verify()` passes for pruned vertices (hashes remain, content cleared)
- Summary statistics match recomputation from the original subtree
- ZK proof for DecisionGrounding verifies with the plonky2 verifier
- DAG integration with the Synapse Loop creates vertices at the correct steps (Observation at PERCEIVE, Prediction at ATTEND, etc.)

---

## On-Chain Anchoring

The DAG root hash can be published on-chain for non-repudiable timestamping. Two modes:

### Periodic Anchoring

Every N ticks (default: 720, or approximately once per 2 hours at gamma speed), publish the current DAG root hash to a smart contract on Korai (Roko's dedicated EVM chain). This creates a public commitment that can be verified against the local DAG at any future point.

### Event-Driven Anchoring

After significant decisions (large trades, strategy changes, phase transitions, critical code deployments), anchor the DAG root immediately. This ties high-impact reasoning to an on-chain timestamp.

### Anchor Contract

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// Minimal witness anchoring contract on Korai.
contract WitnessAnchor {
    event DAGRootAnchored(
        address indexed agent,
        bytes32 dagRoot,
        uint64 tickNumber,
        uint256 timestamp
    );

    /// Anchor a DAG root hash. Callable by any agent.
    function anchor(bytes32 dagRoot, uint64 tickNumber) external {
        emit DAGRootAnchored(msg.sender, dagRoot, tickNumber, block.timestamp);
    }
}
```

Note the naming change from the legacy specification: `golem` parameter is now `agent`, reflecting that agents are the autonomous entities in the Roko architecture.

---

## DAG-Based Trust in Collectives

### Verifiable Reasoning Quality

When Agent A wants to establish trust with Agent B in a Collective (a group of cooperating agents), it shares a DAG subtree. Agent B can verify:

1. **Internal consistency.** All commitment hashes are valid. No vertex was modified after creation.
2. **Observation grounding.** Observation vertices reference verifiable external events — block numbers exist, prices match, test results are reproducible.
3. **Prediction honesty.** Every prediction has a corresponding resolution. The agent is not cherry-picking successes. A missing resolution for an old prediction is suspicious.
4. **Knowledge depth.** Neuro entries descending from many independent observations through multiple prediction-resolution cycles carry more evidential weight than those from a single observation.

Trust becomes proportional to verifiable quality of reasoning, not historical reputation alone. A new agent with a short but high-quality DAG establishes trust faster than reputation alone would allow.

### Mesh Integration

When an agent shares a Neuro entry with its Collective via the Agent Mesh (`roko-mesh`, the P2P relay with permissioned subnets), it attaches the DAG subtree rooted at that entry. Collective members verify provenance before incorporating the knowledge. Knowledge sharing shifts from "trust the source" to "verify then trust."

### Temporal Logic Integration

Each tick's witness includes not just what the agent did but whether its behavior satisfied its temporal contract (see [11-temporal-logic.md](11-temporal-logic.md)). A violated specification produces a witness of misbehavior — a cryptographic proof that the agent failed to meet its behavioral commitments. This is relevant for accountability in multi-agent Collectives.

---

## Relation to Engram Lineage

The Engram struct (defined in `roko-core`) already includes a `lineage: Vec<ContentHash>` field that tracks parent Engrams. The Witness DAG is a **richer structure** built on top of Engram lineage:

| Feature | Engram Lineage | Witness DAG |
|---|---|---|
| Structure | Flat list of parent hashes | Full DAG with typed vertices and edges |
| Vertex types | One type (Engram) | Five types (O, P, D, R, G) |
| Query | "What are the parents?" | "What observations support this knowledge?" |
| Verification | Content hash matches | Full Merkle chain verification |
| ZK proofs | Not supported | Four proof types |
| Storage | Inline in Engram | Separate SQLite tables |
| Pruning | By decay/TTL | Rolling window + compression |

The two systems are complementary: Engram lineage provides lightweight parent tracking for every Engram; the Witness DAG provides deep provenance analysis for safety-critical reasoning chains.

---

## Implementation Status

| Component | Status | Location |
|---|---|---|
| Engram `lineage` field | Built | `roko-core/src/signal.rs` (will be `engram.rs` after Tier 0D rename) |
| FileSubstrate (JSONL persistence) | Built | `roko-fs/` |
| Linear audit chain (hash chaining in ToolDispatcher) | Built | `roko-agent/src/dispatcher/mod.rs` `emit_audit()` |
| WitnessDAG data structures | Design only | Target: Tier 3 |
| SQLite storage backend | Design only | Target: Tier 3 |
| ZK proof generation (plonky2) | Design only | Target: Tier 4 |
| On-chain anchoring (Korai) | Design only | Target: Tier 4 |
| DAG-based Collective trust | Design only | Target: Tier 5 |

---

## Academic References

| Paper | Contribution |
|---|---|
| Merkle (1987), "A Digital Signature Based on a Conventional Encryption Function" | Merkle tree — foundation of hash-chain integrity |
| O'Connor & Aumasson (2020), BLAKE3 specification | BLAKE3 hash function used for all DAG commitments |
| Ben-Sasson et al. (2018), "Scalable, transparent, and post-quantum secure computational integrity" | ZK-STARKs for proof generation |
| Gabizon, Williamson, Ciobotaru (2019), "PLONK: Permutations over Lagrange-bases for Oecumenical Noninteractive arguments of Knowledge" | plonky2 proof system for decision grounding proofs |
| Sumers et al. (2023, arXiv:2309.02427), "Cognitive Architectures for Language Agents" | CoALA cognitive loop — the 9 steps that generate DAG vertices |
| W3C PROV-DM (2013) | Standard provenance data model |
| Calautti et al. (2024, ACM PODS) | Why-provenance complexity for Datalog — NP-complete |
| Karvounarakis (2012) | ProQL — provenance query language with path expressions |
| Palumbo et al. (2026, arXiv:2602.16708) | PCAS — Datalog-derived policy language for agents |

---

## DAG Query Language (WQL -- Witness Query Language)

A Datalog-inspired query language for provenance queries over the Witness DAG, grounded in W3C PROV-DM (2013) and formal provenance query research.

### W3C PROV-DM Alignment

The Witness DAG maps onto the W3C PROV-DM standard as follows:

| Witness DAG Concept | W3C PROV-DM Concept |
|---|---|
| Vertex | Entity |
| Tool call / Loop step | Activity |
| Agent instance | Agent |
| Parent edge | wasGeneratedBy / used / wasDerivedFrom |

```rust
/// W3C PROV-DM aligned provenance primitives.
/// See https://www.w3.org/TR/prov-dm/
pub enum ProvRelation {
    /// Activity used this entity as input.
    Used { activity: Hash, entity: Hash },
    /// This entity was generated by an activity.
    WasGeneratedBy { entity: Hash, activity: Hash },
    /// Derived entity was computed from source entity.
    WasDerivedFrom { derived: Hash, source: Hash },
    /// This activity was associated with this agent.
    WasAssociatedWith { activity: Hash, agent: String },
    /// This entity was attributed to this agent.
    WasAttributedTo { entity: Hash, agent: String },
}
```

### WQL: Witness Query Language

WQL is a Datalog-style query language for DAG provenance queries. A query consists of a head predicate (what to compute) and a body of clauses (what must hold).

```rust
/// A WQL query over the Witness DAG.
/// Datalog-inspired with provenance-specific built-in predicates.
pub struct WqlQuery {
    /// The goal predicate to evaluate.
    pub head: WqlPredicate,
    /// Body clauses that must all be satisfied.
    pub body: Vec<WqlClause>,
    /// Maximum depth for recursive traversal.
    pub max_depth: Option<u32>,
    /// Time window constraint.
    pub time_window: Option<TimeWindow>,
}

pub enum WqlClause {
    /// Match a vertex by type and bind to variable.
    Vertex { var: String, vertex_type: Option<VertexType> },
    /// Match a parent edge.
    ParentOf { parent: String, child: String },
    /// Transitive ancestor (recursive).
    AncestorOf { ancestor: String, descendant: String },
    /// Time constraint on a vertex.
    TimeBound { var: String, after: Option<u64>, before: Option<u64> },
    /// Vertex content matches a pattern.
    ContentMatch { var: String, pattern: String },
    /// Negation: this clause must NOT hold.
    Not(Box<WqlClause>),
}

pub enum WqlPredicate {
    /// Count vertices of a type in the provenance subgraph.
    Count { var: String, vertex_type: VertexType },
    /// Check that all paths from root reach a vertex of this type.
    AllPathsReach { root: String, target_type: VertexType },
    /// Extract the subgraph rooted at a vertex.
    Subgraph { root: String },
    /// Compute prediction accuracy in a subgraph.
    PredictionAccuracy { root: String },
}
```

### Built-in query templates

Common provenance queries expressed as WQL templates:

```
% "What observations support this decision?"
observation_support(D, O) :-
    vertex(D, decision),
    ancestor_of(O, D),
    vertex(O, observation).

% "Was this knowledge entry grounded in real observations?"
grounded(G) :-
    vertex(G, neuro_entry),
    observation_support(G, _O).

% "Which decisions had no observation basis (hallucinations)?"
hallucination(D) :-
    vertex(D, decision),
    NOT observation_support(D, _).

% "What is the evidential depth of this knowledge entry?"
evidential_depth(G, N) :-
    vertex(G, neuro_entry),
    count(O, observation, ancestor_of(O, G), N).

% "Which predictions were accurate vs inaccurate?"
prediction_resolved(P, R) :-
    vertex(P, prediction),
    parent_of(P, R),  % P is parent of R
    vertex(R, resolution).

% "Trace the complete reasoning chain for a decision"
reasoning_chain(D, Chain) :-
    vertex(D, decision),
    subgraph(D, Chain).
```

### WQL evaluator

```rust
/// WQL query evaluator over the Witness DAG.
pub struct WqlEvaluator {
    dag: Arc<WitnessDAG>,
    /// Maximum recursion depth for ancestor queries.
    max_recursion_depth: u32,
    /// Query timeout.
    timeout: Duration,
    /// Cache for memoized subgraph traversals.
    cache: HashMap<Hash, Vec<Arc<Vertex>>>,
}

impl WqlEvaluator {
    pub fn new(dag: Arc<WitnessDAG>, max_depth: u32, timeout: Duration) -> Self {
        Self {
            dag,
            max_recursion_depth: max_depth,
            timeout,
            cache: HashMap::new(),
        }
    }

    /// Evaluate a WQL query and return matching vertices.
    pub fn evaluate(&mut self, query: &WqlQuery) -> Result<WqlResult, WqlError> {
        let start = Instant::now();

        // Bind variables through clause evaluation
        let mut bindings = WqlBindings::new();

        for clause in &query.body {
            if start.elapsed() > self.timeout {
                return Err(WqlError::Timeout);
            }
            self.evaluate_clause(clause, &mut bindings)?;
        }

        // Evaluate head predicate with bindings
        self.evaluate_predicate(&query.head, &bindings)
    }

    fn evaluate_clause(
        &self,
        clause: &WqlClause,
        bindings: &mut WqlBindings,
    ) -> Result<(), WqlError> {
        match clause {
            WqlClause::AncestorOf { ancestor, descendant } => {
                // BFS traversal with depth limit
                let desc_hash = bindings.get_hash(descendant)?;
                let ancestors = self.dag.provenance(&desc_hash);
                // ... bind matching ancestors
                Ok(())
            }
            // ... other clause types
            _ => Ok(())
        }
    }
}

pub enum WqlResult {
    /// A set of matching vertices.
    Vertices(Vec<Arc<Vertex>>),
    /// A count result.
    Count(u64),
    /// A boolean result (all-paths-reach, grounded).
    Boolean(bool),
    /// A subgraph extraction.
    Subgraph(Vec<Arc<Vertex>>, Vec<(Hash, Hash)>),
    /// Prediction accuracy percentage.
    Accuracy(f64),
}

pub enum WqlError {
    Timeout,
    MaxDepthExceeded,
    UnboundVariable(String),
    InvalidVertexType,
}
```

### Configuration

```toml
[safety.witness_dag.query]
max_recursion_depth = 100    # Max depth for ancestor queries. Range: 10..1000.
query_timeout_ms = 5000      # Per-query timeout. Range: 100..60000.
cache_size = 10000           # Memoization cache entries. Range: 100..1000000.
enable_wql_cli = true        # Enable WQL queries via `roko dag query`.
```

### Provenance complexity considerations

Calautti et al. (2024, ACM PODS) showed that why-provenance for Datalog is NP-complete even for linear recursion. Roko sidesteps this by targeting how-provenance (polynomial time) using BFS traversal with bounded depth, not full why-provenance. The `max_recursion_depth` parameter caps traversal cost. Combined with the query timeout, this keeps WQL evaluation predictable under adversarial DAG shapes (deep chains, fan-out explosions).

### Test criteria

- WQL `ancestor_of` query finds all ancestors via BFS within depth limit
- WQL `observation_support` correctly identifies observation vertices in provenance
- WQL `hallucination` query returns decisions with no observation ancestry
- WQL evaluation respects timeout and returns `WqlError::Timeout`
- WQL cache correctly memoizes repeated subgraph traversals
- WQL `evidential_depth` returns correct count of observation ancestors
- WQL negation (`NOT`) correctly excludes matching vertices

---

## DAG Query Language

The Witness DAG is only as useful as the queries you can run against it. This section defines a typed query interface for structural, temporal, and provenance queries over the content-addressed DAG. The design draws on three foundations: GQL (ISO/IEC 39075:2024, the first ISO-standard graph query language), Datalog for recursive provenance queries, and petgraph for Rust-native graph algorithms.

### Query types

Five categories of queries cover the forensic and safety use cases:

```rust
/// A query over the Witness DAG.
/// Supports structural, temporal, provenance, pattern, and aggregate queries.
pub enum DagQuery {
    /// Reachability: did vertex A causally influence vertex B?
    Reachability {
        source: VertexHash,
        target: VertexHash,
    },
    /// Ancestry: all predecessors of a vertex (full causal history).
    Ancestry {
        vertex: VertexHash,
        /// Maximum depth. None = unbounded.
        max_depth: Option<usize>,
    },
    /// Provenance chain: the causal path from source to target.
    ProvenanceChain {
        source: VertexHash,
        target: VertexHash,
        /// Return all paths or just shortest.
        all_paths: bool,
    },
    /// Pattern match: find subgraph patterns (twig queries).
    PatternMatch {
        pattern: DagPattern,
    },
    /// Temporal slice: all vertices within a time window.
    TemporalSlice {
        start_ms: i64,
        end_ms: i64,
        /// Filter by vertex type.
        vertex_types: Option<Vec<VertexType>>,
    },
    /// Aggregate: compute metrics over the DAG.
    Aggregate {
        metric: DagMetric,
        scope: AggregateScope,
    },
}

/// A structural pattern for subgraph matching.
/// Inspired by GQL Regular Path Queries (ISO/IEC 39075:2024).
pub struct DagPattern {
    /// Pattern nodes with variable bindings.
    pub nodes: Vec<PatternNode>,
    /// Edge constraints between pattern nodes.
    pub edges: Vec<PatternEdge>,
    /// Filter predicates on node/edge attributes.
    pub predicates: Vec<PatternPredicate>,
}

pub struct PatternNode {
    /// Variable name for this node (e.g., "a", "b").
    pub variable: String,
    /// Required vertex type (None = any type).
    pub vertex_type: Option<VertexType>,
    /// Required tag constraints.
    pub tag_constraints: Vec<(String, String)>,
}

pub struct PatternEdge {
    /// Source node variable.
    pub from: String,
    /// Target node variable.
    pub to: String,
    /// Edge type constraint.
    pub edge_type: Option<EdgeType>,
    /// Minimum hops (for path queries). Default: 1.
    pub min_hops: usize,
    /// Maximum hops (for path queries). None = unbounded.
    pub max_hops: Option<usize>,
}

pub enum PatternPredicate {
    /// Node attribute equals value.
    NodeAttrEq { variable: String, attr: String, value: String },
    /// Node timestamp within range.
    NodeTimestampRange { variable: String, start_ms: i64, end_ms: i64 },
    /// Edge timestamp ordering: from.timestamp < to.timestamp.
    TemporalOrder { from: String, to: String },
}

/// Metrics computable over DAG subgraphs.
pub enum DagMetric {
    /// Count of vertices matching criteria.
    VertexCount,
    /// Maximum depth of the DAG from roots.
    MaxDepth,
    /// Average branching factor (out-degree).
    AvgBranchingFactor,
    /// Longest path (in edges).
    LongestPath,
    /// Count of vertices by type.
    TypeDistribution,
}

pub enum AggregateScope {
    /// Entire DAG.
    Global,
    /// Subgraph reachable from a vertex.
    SubgraphFrom(VertexHash),
    /// Vertices within a time window.
    TimeWindow { start_ms: i64, end_ms: i64 },
}
```

### Query execution engine

```rust
/// Executes queries against the Witness DAG.
/// Uses petgraph for graph algorithms and SQLite for indexed lookups.
pub struct DagQueryEngine {
    /// In-memory graph representation (petgraph).
    graph: petgraph::Graph<WitnessVertex, WitnessEdge>,
    /// Node index lookup by vertex hash.
    hash_to_node: HashMap<VertexHash, NodeIndex>,
    /// SQLite connection for persistent queries.
    db: rusqlite::Connection,
}

impl DagQueryEngine {
    /// Execute a query and return results.
    pub fn execute(&self, query: &DagQuery) -> Result<QueryResult> {
        match query {
            DagQuery::Reachability { source, target } => {
                let src = self.resolve(source)?;
                let tgt = self.resolve(target)?;
                let reachable = petgraph::algo::has_path_connecting(
                    &self.graph, src, tgt, None
                );
                Ok(QueryResult::Boolean(reachable))
            }
            DagQuery::Ancestry { vertex, max_depth } => {
                let node = self.resolve(vertex)?;
                let ancestors = self.collect_ancestors(node, *max_depth);
                Ok(QueryResult::Vertices(ancestors))
            }
            DagQuery::ProvenanceChain { source, target, all_paths } => {
                let src = self.resolve(source)?;
                let tgt = self.resolve(target)?;
                if *all_paths {
                    let paths = self.all_simple_paths(src, tgt);
                    Ok(QueryResult::Paths(paths))
                } else {
                    let path = self.shortest_path(src, tgt);
                    Ok(QueryResult::Paths(path.into_iter().collect()))
                }
            }
            DagQuery::PatternMatch { pattern } => {
                let matches = self.match_pattern(pattern);
                Ok(QueryResult::PatternMatches(matches))
            }
            DagQuery::TemporalSlice { start_ms, end_ms, vertex_types } => {
                let vertices = self.temporal_slice(*start_ms, *end_ms, vertex_types);
                Ok(QueryResult::Vertices(vertices))
            }
            DagQuery::Aggregate { metric, scope } => {
                let value = self.compute_aggregate(metric, scope);
                Ok(QueryResult::Aggregate(value))
            }
        }
    }

    /// Collect ancestors via reverse BFS with optional depth bound.
    fn collect_ancestors(
        &self,
        node: NodeIndex,
        max_depth: Option<usize>,
    ) -> Vec<WitnessVertex> {
        let reversed = petgraph::visit::Reversed(&self.graph);
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back((node, 0usize));
        visited.insert(node);
        let mut result = Vec::new();

        while let Some((current, depth)) = queue.pop_front() {
            if let Some(max) = max_depth {
                if depth > max { continue; }
            }
            if current != node {
                result.push(self.graph[current].clone());
            }
            for neighbor in reversed.neighbors(current) {
                if visited.insert(neighbor) {
                    queue.push_back((neighbor, depth + 1));
                }
            }
        }
        result
    }

    /// Pattern matching using stack-based twig algorithm.
    /// Matches structural patterns against the DAG.
    fn match_pattern(&self, pattern: &DagPattern) -> Vec<PatternMatch> {
        // For each candidate node matching the first pattern node:
        //   Try to extend the match by finding neighbors matching
        //   subsequent pattern nodes and edges.
        // Uses backtracking search with pruning by vertex type
        // and tag constraints.
        // ...
    }
}

pub enum QueryResult {
    Boolean(bool),
    Vertices(Vec<WitnessVertex>),
    Paths(Vec<Vec<WitnessVertex>>),
    PatternMatches(Vec<PatternMatch>),
    Aggregate(AggregateValue),
}

pub struct PatternMatch {
    /// Variable bindings: variable name -> matched vertex.
    pub bindings: HashMap<String, WitnessVertex>,
}

pub enum AggregateValue {
    Count(usize),
    Float(f64),
    Distribution(HashMap<String, usize>),
}
```

### Datalog provenance queries

For recursive provenance queries (transitive closure, fixed-point computations), embed Datafrog (the lightweight Datalog engine used by the Rust compiler's Polonius borrow checker):

```rust
use datafrog::{Iteration, Relation, RelationLeaper};

/// Compute transitive influence relation over the Witness DAG.
/// Given direct edges (A -> B), computes all transitive pairs (A ->* B).
pub fn transitive_influence(
    direct_edges: &[(VertexHash, VertexHash)],
) -> Vec<(VertexHash, VertexHash)> {
    let mut iteration = Iteration::new();
    let edges = iteration.variable::<(VertexHash, VertexHash)>("edges");
    let influences = iteration.variable::<(VertexHash, VertexHash)>("influences");

    // Seed with direct edges.
    edges.insert(Relation::from_vec(direct_edges.to_vec()));
    influences.insert(Relation::from_vec(direct_edges.to_vec()));

    // Fixed-point: influences(a, c) :- influences(a, b), edges(b, c).
    while iteration.changed() {
        influences.from_join(&influences, &edges, |&_b, &a, &c| (a, c));
    }

    influences.complete().into_iter().collect()
}

/// Find all vertices that were influenced by tainted data.
/// tainted(V) :- source_tainted(V).
/// tainted(V) :- tainted(U), edge(U, V).
pub fn taint_reachability(
    edges: &[(VertexHash, VertexHash)],
    tainted_sources: &[VertexHash],
) -> Vec<VertexHash> {
    let mut iteration = Iteration::new();
    let edge_rel = iteration.variable::<(VertexHash, VertexHash)>("edges");
    let tainted = iteration.variable::<(VertexHash, ())>("tainted");

    edge_rel.insert(Relation::from_vec(edges.to_vec()));
    tainted.insert(Relation::from_vec(
        tainted_sources.iter().map(|v| (v.clone(), ())).collect(),
    ));

    while iteration.changed() {
        // tainted(v) :- tainted(u), edge(u, v).
        tainted.from_join(&tainted, &edge_rel, |&_u, &(), &v| (v, ()));
    }

    tainted.complete().into_iter().map(|(v, _)| v).collect()
}
```

### Safety-specific query patterns

Pre-built queries for common safety analysis tasks:

| Query Name | Pattern | Use Case |
|---|---|---|
| `toctou_detection` | `read(F) ->* modify(F) ->* use(F)` where `modify.agent != read.agent` | Detect time-of-check/time-of-use races |
| `escalation_chain` | `action(A1) -> action(A2) -> ... -> action(AN)` where `permission(A_i) < permission(A_{i+1})` | Detect privilege escalation sequences |
| `data_exfiltration` | `read(sensitive) ->* network_call(external)` | Detect potential data exfiltration paths |
| `circular_reasoning` | cycle in `decision -> observation -> decision` | Detect reasoning loops |
| `orphaned_approvals` | `Decision` vertex with no child `Resolution` vertex | Find decisions that were never resolved |

```rust
/// Pre-built safety query patterns.
pub struct SafetyQueries;

impl SafetyQueries {
    /// Detect TOCTOU: file read by agent A, modified by agent B,
    /// then used by agent A without re-reading.
    pub fn toctou_detection() -> DagPattern {
        DagPattern {
            nodes: vec![
                PatternNode { variable: "read".into(), vertex_type: Some(VertexType::Observation), tag_constraints: vec![("tool".into(), "read_file".into())] },
                PatternNode { variable: "modify".into(), vertex_type: Some(VertexType::Decision), tag_constraints: vec![("tool".into(), "write_file".into())] },
                PatternNode { variable: "use".into(), vertex_type: Some(VertexType::Observation), tag_constraints: vec![("tool".into(), "read_file".into())] },
            ],
            edges: vec![
                PatternEdge { from: "read".into(), to: "modify".into(), edge_type: None, min_hops: 1, max_hops: Some(10) },
                PatternEdge { from: "modify".into(), to: "use".into(), edge_type: None, min_hops: 1, max_hops: Some(10) },
            ],
            predicates: vec![
                PatternPredicate::TemporalOrder { from: "read".into(), to: "modify".into() },
                PatternPredicate::TemporalOrder { from: "modify".into(), to: "use".into() },
            ],
        }
    }

    /// Detect privilege escalation sequences.
    pub fn escalation_chain(min_length: usize) -> DagPattern {
        // Pattern: sequence of decisions with monotonically increasing
        // permission levels.
        DagPattern {
            nodes: (0..min_length).map(|i| PatternNode {
                variable: format!("action_{}", i),
                vertex_type: Some(VertexType::Decision),
                tag_constraints: vec![],
            }).collect(),
            edges: (0..min_length-1).map(|i| PatternEdge {
                from: format!("action_{}", i),
                to: format!("action_{}", i + 1),
                edge_type: None,
                min_hops: 1,
                max_hops: Some(5),
            }).collect(),
            predicates: vec![],
        }
    }
}
```

### Configuration

```toml
[safety.dag_query]
# Enable the DAG query engine.
enabled = true
# Maximum query execution time in milliseconds.
# Range: 100..60000. Default: 5000.
max_query_time_ms = 5000
# Maximum result set size.
# Range: 10..100000. Default: 10000.
max_results = 10000
# Enable Datalog recursive queries.
enable_datalog = true
# Maximum fixed-point iterations for Datalog.
# Range: 10..10000. Default: 1000.
max_datalog_iterations = 1000
# Pre-built safety queries to run automatically per task.
auto_queries = ["toctou_detection", "escalation_chain", "orphaned_approvals"]
# Interval for automatic safety queries (in tasks completed).
# Range: 1..100. Default: 5.
auto_query_interval_tasks = 5
```

### Test criteria

- `DagQueryEngine::execute(Reachability)` returns true for connected vertices and false for disconnected
- `DagQueryEngine::execute(Ancestry)` with max_depth=2 returns only vertices within 2 hops
- `DagQueryEngine::execute(ProvenanceChain)` returns shortest path when all_paths=false
- `DagQueryEngine::execute(PatternMatch)` with TOCTOU pattern detects the `read -> modify -> use` sequence
- `transitive_influence()` computes correct transitive closure for a 5-node chain
- `taint_reachability()` correctly propagates taint through 3+ hops
- `SafetyQueries::toctou_detection()` pattern matches when agents differ between read and modify
- Auto-queries run every `auto_query_interval_tasks` completed tasks
- Query execution respects `max_query_time_ms` timeout

---

## Related Topics

- [02-audit-chain.md](02-audit-chain.md) — The linear Merkle hash-chain that the DAG extends
- [11-temporal-logic.md](11-temporal-logic.md) — Temporal verdicts enrich witness records
- [15-forensic-ai.md](15-forensic-ai.md) — The Witness DAG is the data structure underlying forensic replay
- [13-formal-verification.md](13-formal-verification.md) — Formal verification of smart contracts before committing capital
