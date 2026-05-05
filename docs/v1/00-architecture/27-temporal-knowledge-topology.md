# Temporal Knowledge Topology

> **Abstract:** Knowledge in Roko exists in time — facts have validity windows, relationships
> evolve, and the order of events matters for causal reasoning. This document specifies a
> temporal knowledge layer that augments Roko's Neuro knowledge store with Allen's interval
> algebra for temporal relation reasoning, an event calculus for tracking fluent changes, and
> a temporal knowledge graph that maintains the full history of knowledge evolution. Under
> REF11, each supporting Engram also carries a first-class HDC fingerprint, so temporal
> consolidation can cluster related episodes by both interval overlap and semantic proximity.
> The result enables agents to answer not just "what is true?" but "what was true when?",
> "what caused what?", and "which temporal patterns are converging into durable categories?"
> See [tmp/refinements/11-hyperdimensional-substrate.md](../../tmp/refinements/11-hyperdimensional-substrate.md),
> [02-engram-data-type.md](./02-engram-data-type.md), and
> [07-substrate-trait.md](./07-substrate-trait.md).

> **Implementation**: Specified

**Topic**: [00-architecture](./INDEX.md)
**Prerequisites**: [02-engram-data-type](./02-engram-data-type.md), [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md), [04-decay-variants](./04-decay-variants.md)
**Key sources**:
- Allen 1983, CACM — Maintaining Knowledge about Temporal Intervals
- Kowalski & Sergot 1986, New Gen. Computing — A Logic-based Calculus of Events
- Rasmussen et al. 2025, arXiv:2501.13956 — Zep/Graphiti: Temporal KG Architecture for Agent Memory
- arXiv:2509.15464 (2025) — Temporal Reasoning with LLMs over Evolving Knowledge Graphs
- arXiv:2401.06072 (2024) — Chain of History: TKG Completion with LLMs
- Lacroix et al. 2020 — Tensor Decomposition for Temporal Knowledge Graph Completion

---

## 1. The Problem: Timeless Knowledge in a Temporal World

Roko's Neuro knowledge store currently treats knowledge as effectively atemporal. An Engram has
a `created_at` timestamp and a `Decay` variant that controls how its weight diminishes over
time. But the knowledge itself — "Rust 1.91 is the minimum version", "alloy requires nightly" —
has no explicit temporal structure. There is no way to express:

- **Validity windows**: "This API endpoint was active from March to June 2025"
- **Temporal ordering**: "The migration happened before the schema change"
- **Temporal overlap**: "While we were on version 2.x, the bug was present"
- **Causal chains**: "Because the CI pipeline broke (event A), the release was delayed (event B)"

This limits the agent's ability to reason about change, history, and causation — all essential
for a self-developing system that must understand its own evolution.

---

## 2. Allen's Interval Algebra

### 2.1 The 13 Relations

Allen (1983) defined 13 mutually exclusive relations between time intervals. Every pair of
temporal intervals satisfies exactly one of these relations (or its inverse):

```rust
/// Allen's 13 temporal interval relations.
///
/// For intervals X = [x_start, x_end] and Y = [y_start, y_end]:
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AllenRelation {
    /// X is entirely before Y: x_end < y_start
    Before,
    /// X meets Y: x_end == y_start (no gap, no overlap)
    Meets,
    /// X overlaps with Y: x_start < y_start < x_end < y_end
    Overlaps,
    /// X starts at the same time as Y but ends earlier: x_start == y_start, x_end < y_end
    Starts,
    /// X is during Y: y_start < x_start, x_end < y_end
    During,
    /// X finishes at the same time as Y but starts later: x_end == y_end, x_start > y_start
    Finishes,
    /// X equals Y: x_start == y_start, x_end == y_end
    Equals,
    /// Inverses (Y relation X)
    After,       // inverse of Before
    MetBy,       // inverse of Meets
    OverlappedBy,// inverse of Overlaps
    StartedBy,   // inverse of Starts
    Contains,    // inverse of During
    FinishedBy,  // inverse of Finishes
}

/// A time interval with nanosecond precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemporalInterval {
    pub start: i64,  // Unix timestamp (nanoseconds)
    pub end: i64,    // Unix timestamp (nanoseconds), or i64::MAX for "ongoing"
}

impl TemporalInterval {
    pub const ONGOING: i64 = i64::MAX;

    pub fn new(start: i64, end: i64) -> Self {
        debug_assert!(start <= end, "interval start must not exceed end");
        Self { start, end }
    }

    /// Determine the Allen relation between self and other.
    pub fn relation_to(&self, other: &Self) -> AllenRelation {
        if self.end < other.start {
            AllenRelation::Before
        } else if self.end == other.start {
            AllenRelation::Meets
        } else if self.start < other.start && self.end > other.start && self.end < other.end {
            AllenRelation::Overlaps
        } else if self.start == other.start && self.end < other.end {
            AllenRelation::Starts
        } else if self.start > other.start && self.end < other.end {
            AllenRelation::During
        } else if self.end == other.end && self.start > other.start {
            AllenRelation::Finishes
        } else if self.start == other.start && self.end == other.end {
            AllenRelation::Equals
        } else if self.start > other.end {
            AllenRelation::After
        } else if self.start == other.end {
            AllenRelation::MetBy
        } else if other.start < self.start && other.end > self.start && other.end < self.end {
            AllenRelation::OverlappedBy
        } else if self.start == other.start && self.end > other.end {
            AllenRelation::StartedBy
        } else if self.start < other.start && self.end > other.end {
            AllenRelation::Contains
        } else if self.end == other.end && self.start < other.start {
            AllenRelation::FinishedBy
        } else {
            unreachable!("all Allen relations are covered")
        }
    }

    /// Does this interval overlap with another?
    pub fn overlaps_with(&self, other: &Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// Duration in nanoseconds (None if ongoing).
    pub fn duration_ns(&self) -> Option<i64> {
        if self.end == Self::ONGOING { None } else { Some(self.end - self.start) }
    }
}
```

### 2.2 Temporal Constraint Network

Allen's algebra supports constraint propagation — if we know that A is before B and B overlaps
C, we can infer the possible relations between A and C. This is implemented as a constraint
network over Engram validity intervals.

```rust
/// Temporal constraint network over Engram validity intervals.
///
/// Stores pairwise Allen relations and propagates constraints
/// when new relations are added.
pub struct TemporalConstraintNetwork {
    /// Adjacency matrix: (hash_a, hash_b) → set of possible Allen relations.
    constraints: HashMap<(ContentHash, ContentHash), AllenRelationSet>,
    /// All known intervals.
    intervals: HashMap<ContentHash, TemporalInterval>,
}

/// Compact set of Allen relations (13 bits, one per relation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllenRelationSet(u16);

impl AllenRelationSet {
    pub const ALL: Self = Self(0x1FFF);  // 13 bits set
    pub const EMPTY: Self = Self(0);

    pub fn singleton(r: AllenRelation) -> Self {
        Self(1 << r as u16)
    }

    pub fn contains(&self, r: AllenRelation) -> bool {
        self.0 & (1 << r as u16) != 0
    }

    pub fn intersect(&self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }
}
```

### 2.3 Constraint Propagation Algorithm

```
ALGORITHM: AllenConstraintPropagation(network, new_constraint(A, B, R))

1. Add R to constraints[(A, B)]
2. Initialize worklist = [(A, B)]
3. While worklist is not empty:
   a. Pop (X, Y) from worklist
   b. For each Z ≠ X, Y with known constraints:
      - Compute transitive: R_xz_new = compose(constraints[(X, Y)], constraints[(Y, Z)])
      - Compute intersection: R_xz = constraints[(X, Z)] ∩ R_xz_new
      - If R_xz is stricter than stored:
        - Update constraints[(X, Z)] = R_xz
        - Add (X, Z) to worklist
      - If R_xz is empty:
        - INCONSISTENCY DETECTED — temporal contradiction
        - Return Err(TemporalContradiction { x: X, y: Y, z: Z })
4. Return Ok(network)

COMPLEXITY: O(N³) worst case, but sparse networks are much faster.
COMPOSITION TABLE: 13×13 table of Allen relation compositions (Allen 1983, Table 2).
```

---

## 3. Event Calculus

### 3.1 Fluents and Events

The event calculus (Kowalski & Sergot 1986) models how the truth of properties ("fluents")
changes in response to events.

```rust
/// A fluent is a time-varying property of the system.
///
/// Examples:
///   "rust_version(1.91)" — the current Rust version is 1.91
///   "ci_passing(true)" — CI pipeline is currently passing
///   "feature_enabled(dark_mode)" — dark mode feature is enabled
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fluent {
    /// Unique identifier for this fluent.
    pub id: FluentId,
    /// Human-readable name.
    pub name: String,
    /// Current value (JSON-encoded for flexibility).
    pub value: serde_json::Value,
    /// Validity interval: when this fluent holds.
    pub valid: TemporalInterval,
    /// The event that initiated this fluent value.
    pub initiated_by: Option<EventId>,
    /// The event that terminated this fluent value (None if ongoing).
    pub terminated_by: Option<EventId>,
}

/// An event is a point-in-time occurrence that initiates or terminates fluents.
///
/// Examples:
///   Event("rustup update stable") initiates Fluent("rust_version(1.91)")
///   Event("CI failure") terminates Fluent("ci_passing(true)")
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalEvent {
    pub id: EventId,
    /// When the event occurred (point in time, not interval).
    pub timestamp: i64,
    /// Human-readable description.
    pub description: String,
    /// The Engram that recorded this event (if any).
    pub engram_hash: Option<ContentHash>,
    /// Fluents initiated by this event.
    pub initiates: Vec<FluentId>,
    /// Fluents terminated by this event.
    pub terminates: Vec<FluentId>,
    /// Causal predecessors: events that caused this event.
    pub caused_by: Vec<EventId>,
}

/// Unique identifiers.
pub type FluentId = Uuid;
pub type EventId = Uuid;
```

### 3.2 Event Calculus Axioms

The three core axioms, implemented as query predicates:

```rust
/// Event calculus query engine.
pub struct EventCalculus {
    pub events: BTreeMap<i64, Vec<TemporalEvent>>,  // ordered by timestamp
    pub fluents: HashMap<FluentId, Vec<Fluent>>,     // history of each fluent
}

impl EventCalculus {
    /// HoldsAt(fluent, time) — is the fluent true at the given time?
    ///
    /// A fluent holds at time T if:
    ///   ∃ event E where E.timestamp ≤ T and E initiates fluent
    ///   AND ¬∃ event E' where E.timestamp < E'.timestamp ≤ T and E' terminates fluent
    pub fn holds_at(&self, fluent_id: FluentId, time: i64) -> bool {
        self.fluents.get(&fluent_id)
            .map(|history| {
                history.iter().any(|f| {
                    f.valid.start <= time
                        && (f.valid.end == TemporalInterval::ONGOING || f.valid.end > time)
                })
            })
            .unwrap_or(false)
    }

    /// Initiates(event, fluent, time) — does this event start the fluent?
    pub fn initiates(&self, event_id: EventId, fluent_id: FluentId) -> bool {
        self.events.values()
            .flatten()
            .any(|e| e.id == event_id && e.initiates.contains(&fluent_id))
    }

    /// Terminates(event, fluent, time) — does this event end the fluent?
    pub fn terminates(&self, event_id: EventId, fluent_id: FluentId) -> bool {
        self.events.values()
            .flatten()
            .any(|e| e.id == event_id && e.terminates.contains(&fluent_id))
    }

    /// CausedBy(event_a, event_b) — transitive causal chain.
    pub fn caused_by(&self, effect: EventId, cause: EventId) -> bool {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(effect);
        while let Some(current) = queue.pop_front() {
            if current == cause { return true; }
            if !visited.insert(current) { continue; }
            if let Some(event) = self.find_event(current) {
                queue.extend(event.caused_by.iter().copied());
            }
        }
        false
    }

    fn find_event(&self, id: EventId) -> Option<&TemporalEvent> {
        self.events.values().flatten().find(|e| e.id == id)
    }
}
```

---

## 4. Temporal Knowledge Graph

### 4.1 Three-Tier Architecture (Inspired by Graphiti/Zep)

Rasmussen et al. (2025) demonstrated a three-tier temporal KG architecture that outperforms
flat memory systems. Roko adapts this for its Engram-based knowledge:

```rust
/// Temporal Knowledge Graph with three tiers.
///
/// Tier 1: Episode Layer — raw Engram sequences with bundled fingerprints (what happened)
/// Tier 2: Entity Layer — extracted entities with temporal properties and evolving centroids (what exists)
/// Tier 3: Community Layer — HDC-backed clusters of related entities (what patterns exist)
pub struct TemporalKnowledgeGraph {
    /// Tier 1: Episodes (ordered sequences of Engrams).
    pub episodes: Vec<TemporalEpisode>,
    /// Tier 2: Temporal entities extracted from episodes.
    pub entities: HashMap<EntityId, TemporalEntity>,
    /// Tier 3: Community clusters (updated during Delta consolidation).
    pub communities: Vec<TemporalCommunity>,
    /// Allen constraint network over entity validity intervals.
    pub temporal_constraints: TemporalConstraintNetwork,
    /// Event calculus engine for fluent/event reasoning.
    pub event_calculus: EventCalculus,
}

/// A temporal episode: a sequence of Engrams within a time window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalEpisode {
    pub id: Uuid,
    pub interval: TemporalInterval,
    pub engram_hashes: Vec<ContentHash>,
    /// Bundle of the episode's member Engram fingerprints.
    pub fingerprint: Option<HdcVector>,
    /// Summary (generated during Theta reflection).
    pub summary: Option<String>,
    /// Causal links to other episodes.
    pub causal_links: Vec<Uuid>,
}

/// A temporal entity: something with identity that persists through time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalEntity {
    pub id: EntityId,
    pub name: String,
    /// Kind of entity (file, function, crate, concept, agent, etc.).
    pub kind: EntityKind,
    /// Properties that change over time (modeled as fluents).
    pub properties: Vec<FluentId>,
    /// Relationships to other entities, each with a validity interval.
    pub relationships: Vec<TemporalRelationship>,
    /// Running centroid from the supporting Engram fingerprints.
    pub fingerprint: Option<HdcVector>,
    /// First observed.
    pub created: i64,
    /// Last observed (or ONGOING).
    pub last_seen: i64,
}

/// A relationship between entities that has a temporal validity window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalRelationship {
    pub source: EntityId,
    pub target: EntityId,
    pub relation_type: RelationType,
    pub valid: TemporalInterval,
    /// Confidence in this relationship (degrades if not re-observed).
    pub confidence: f64,
    /// Engram that established this relationship.
    pub evidence: ContentHash,
}

pub type EntityId = Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntityKind {
    File, Function, Struct, Trait, Crate, Module,
    Concept, Agent, Task, Plan, Prd,
    ExternalApi, Dependency, Configuration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelationType {
    DependsOn, Implements, Tests, Calls, Contains,
    CreatedBy, ModifiedBy, CausedBy, Supersedes,
    ConflictsWith, Relates,
}
```

### 4.2 Temporal Community Detection

During Delta consolidation, the TKG clusters entities into temporal communities — groups
of entities that are tightly related within overlapping time windows.

```rust
/// A temporal community: a cluster of entities that co-exist in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalCommunity {
    pub id: Uuid,
    pub entities: Vec<EntityId>,
    /// The temporal intersection: when all entities in this community co-exist.
    pub active_interval: TemporalInterval,
    /// Bundle-center used for similarity-driven promotion.
    pub fingerprint: Option<HdcVector>,
    /// Community summary (LLM-generated during Delta).
    pub summary: Option<String>,
    /// Stability score: how long this community has existed unchanged.
    pub stability: f64,
}
```

### 4.3 HDC Fingerprints Across the Temporal Tiers

Allen relations explain *when* two records relate. HDC fingerprints explain *how close in
meaning* those records are. REF11 makes that second signal native by carrying a deterministic
10,240-bit fingerprint on every Engram rather than in an optional side index. The temporal
topology uses those fingerprints at each tier:

- **Tier 1 episodes** bundle the fingerprints of their member Engrams, producing an episode
  centroid that can be compared with other episodes in constant time.
- **Tier 2 entities** maintain a running centroid over the Engrams that mention or update the
  entity, so "the same thing evolving through time" becomes a similarity query instead of a
  fragile string join.
- **Tier 3 communities** are formed when temporal overlap and HDC similarity both exceed
  threshold. This avoids clustering two co-temporal but semantically unrelated episodes just
  because they happened near each other.

That combination is what lets the Neuro cross-cut graduate repeated episodes into broader
category Engrams. Temporal overlap supplies the candidate window; HDC similarity supplies the
semantic bundle center.

### 4.4 HDC-Guided Tier Progression

The tier progression loop in Neuro already distinguishes episodes, entities, and durable
knowledge. HDC fingerprints make the promotion path explicit:

1. Episode Engrams land with their own fingerprints.
2. Delta consolidation groups temporally overlapping episodes whose fingerprints fall within a
   similarity radius.
3. The cluster center becomes a candidate semantic Engram: less specific than any single
   episode, but still anchored to the contributing lineage.
4. As older episodes accumulate noise through decay or demurrage-driven fuzzing, the centroid
   remains close to the broad pattern while drifting away from one-off details.

This is the temporal side of REF11's claim that memory should become more categorical over
time instead of merely disappearing. The topology chapter therefore treats HDC clustering as a
promotion primitive, not just a retrieval convenience.

---

## 5. Temporal Queries

### 5.1 Query Language Extensions

The temporal layer adds new query capabilities to the Neuro knowledge store:

```rust
/// Temporal query extensions.
pub enum TemporalQuery {
    /// What was true at time T?
    PointQuery {
        fluent_pattern: String,
        at_time: i64,
    },
    /// What was true during interval [start, end]?
    IntervalQuery {
        fluent_pattern: String,
        during: TemporalInterval,
    },
    /// What changed between times T1 and T2?
    DiffQuery {
        entity_pattern: String,
        from: i64,
        to: i64,
    },
    /// What caused event E? (transitive causal chain)
    CausalQuery {
        effect_event: EventId,
        max_depth: usize,
    },
    /// What entities have Allen relation R to entity X?
    AllenQuery {
        reference: ContentHash,
        relation: AllenRelation,
    },
    /// What will likely be true at future time T? (extrapolation)
    PredictionQuery {
        fluent_pattern: String,
        at_future_time: i64,
        confidence_threshold: f64,
    },
}
```

### 5.2 Temporal Retrieval Algorithm

```
ALGORITHM: TemporalRetrieval(query, tkg)

CASE PointQuery(pattern, T):
  1. Find all fluents matching pattern
  2. For each fluent, check HoldsAt(fluent, T)
  3. Return matching fluent values with their confidence

CASE DiffQuery(pattern, T1, T2):
  1. Snapshot_1 = all fluents matching pattern that HoldAt(T1)
  2. Snapshot_2 = all fluents matching pattern that HoldAt(T2)
  3. Added = Snapshot_2 - Snapshot_1
  4. Removed = Snapshot_1 - Snapshot_2
  5. Changed = {f | f.value(T1) ≠ f.value(T2) and f in both snapshots}
  6. Return TemporalDiff { added, removed, changed }

CASE CausalQuery(effect, max_depth):
  1. BFS backward through event.caused_by links
  2. At each hop, collect (event, depth, confidence)
  3. Confidence = product of edge confidences along path
  4. Stop at max_depth or when confidence < 0.01
  5. Return causal chain as DAG

CASE PredictionQuery(pattern, T_future, threshold):
  1. Get fluent history for pattern
  2. Fit trend model (linear regression on value × time)
  3. Extrapolate to T_future with confidence interval
  4. Return prediction if confidence > threshold
```

---

## 6. Integration with Decay Variants

The temporal layer enriches Roko's existing Decay system by adding context-aware decay:

| Decay Variant | Temporal Enhancement |
|---|---|
| **HalfLife** | Half-life scaled by temporal community stability (stable communities decay slower) |
| **TTL** | TTL extended if fluent is still valid (re-observation resets timer) |
| **Ebbinghaus** | Spaced repetition intervals derived from temporal access pattern |
| **None** | No change (axioms and definitions don't decay) |

REF11 adds a second effect alongside scalar decay: the effective HDC fingerprint can become
progressively noisier as an Engram ages. Old records still match their broad category, but
their exact episodic neighborhood fades. Temporal community detection should therefore prefer
fresh exact fingerprints for episode recall and bundled centroids for long-horizon semantic
promotion.

```rust
/// Temporal decay modulation.
pub fn modulated_decay(
    base_decay: &Decay,
    entity: &TemporalEntity,
    community_stability: f64,
) -> Decay {
    match base_decay {
        Decay::HalfLife { half_life_secs } => {
            // Stable community → slower decay
            let modulated_hl = (*half_life_secs as f64 * (1.0 + community_stability)).round() as u64;
            Decay::HalfLife { half_life_secs: modulated_hl }
        }
        Decay::Ttl { expires_at } => {
            // If entity was recently re-observed, extend TTL
            if entity.last_seen > *expires_at - 3600 {
                Decay::Ttl { expires_at: entity.last_seen + 7200 }
            } else {
                base_decay.clone()
            }
        }
        other => other.clone(),
    }
}
```

---

## 7. Configuration

```toml
[temporal]
# Enable temporal knowledge layer.
enabled = true

[temporal.intervals]
# Default validity duration for new Engrams without explicit end time (seconds).
default_validity_secs = 86400  # 24 hours
# Precision for interval comparisons (nanoseconds).
comparison_epsilon_ns = 1_000_000  # 1ms

[temporal.constraint_network]
# Maximum entities in constraint network before pruning oldest.
max_entities = 10_000
# Propagation iteration limit (prevent infinite loops in dense graphs).
max_propagation_iterations = 1_000

[temporal.event_calculus]
# Maximum causal chain depth for CausalQuery.
max_causal_depth = 20
# Minimum confidence for causal chain links.
min_causal_confidence = 0.01

[temporal.tkg]
# Maximum episodes retained (FIFO eviction).
max_episodes = 5_000
# Community detection frequency (every N Delta cycles).
community_detection_interval = 3
# Minimum community size (entities).
min_community_size = 3
# Community stability threshold for promotion.
stability_threshold = 0.7

[temporal.prediction]
# Minimum data points for trend extrapolation.
min_data_points = 5
# Maximum extrapolation horizon (seconds into the future).
max_horizon_secs = 604800  # 7 days
```

---

## 8. Integration Wiring

### 8.1 Into the Universal Cognitive Loop

| Loop Step | Temporal Integration |
|---|---|
| 1. SENSE | `TemporalQuery` augments `Substrate.query()` and `query_similar()` with time-aware retrieval. |
| 2. ASSESS | Score is boosted for temporally relevant Engrams and HDC-near prior episodes. |
| 3. COMPOSE | Temporal context is injected as "as of [timestamp], the following holds...". |
| 4. ACT | Agents emit outputs that can reference historical state and predicted next-state. |
| 5. VERIFY | Gates check temporal consistency and semantic continuity against supporting fingerprints. |
| 6. PERSIST / BROADCAST | New Engrams create `TemporalEvent`s while Pulses publish timeline updates for live consumers. |
| 7. REACT | Policies update the temporal constraint network and promotion queues based on fresh evidence. |

### 8.2 Into Existing Crates

| Crate | Integration Point | Change |
|---|---|---|
| `roko-core` | `Engram` struct | Add optional `valid: TemporalInterval` field alongside the HDC fingerprint metadata |
| `roko-neuro` | `NeuroStore` | Wrap in `TemporalKnowledgeGraph`; use HDC centroids for temporal clustering and promotion |
| `roko-learn` | `EpisodeLogger` | Episodes become `TemporalEpisode` with interval |
| `roko-gate` | `GatePipeline` | Add `TemporalConsistencyGate` |
| `roko-dreams` | Delta cycle | Community detection + HDC cluster bundling + temporal pruning |
| `roko-compose` | Context assembly | Include temporal context section |
| `roko-index` | Symbol graph | Symbol validity intervals plus per-symbol fingerprints for temporal analogy |

---

## 9. Test Criteria

| Test | What It Validates | Type |
|---|---|---|
| `test_allen_all_13_relations` | Each of 13 relations correctly computed | Unit |
| `test_allen_exhaustive_coverage` | Every pair of intervals maps to exactly one relation | Property |
| `test_constraint_propagation_transitive` | A before B, B before C → A before C | Unit |
| `test_constraint_inconsistency_detected` | A before B and B before A → error | Unit |
| `test_holds_at_basic` | Fluent initiated at T1, queried at T2 > T1: holds | Unit |
| `test_holds_at_terminated` | Fluent terminated at T2, queried at T3 > T2: not holds | Unit |
| `test_causal_chain_transitive` | A caused B, B caused C → A caused C | Unit |
| `test_causal_chain_max_depth` | Chain stops at max_depth | Unit |
| `test_tkg_episode_creation` | Engram sequence creates a TemporalEpisode | Integration |
| `test_tkg_entity_extraction` | Repeated Engram references create TemporalEntity | Integration |
| `test_tkg_community_detection` | Co-temporal entities clustered by overlap + fingerprint similarity | Integration |
| `test_tkg_cluster_promotion_centroid` | Stable episode cluster yields a promoted category Engram fingerprint | Integration |
| `test_diff_query_detects_changes` | DiffQuery between T1 and T2 finds added/removed/changed | Unit |
| `test_prediction_linear_trend` | Linear fluent value extrapolates correctly | Unit |
| `test_modulated_decay_stability` | Stable community → slower decay | Unit |
| `test_temporal_consistency_gate` | New Engram contradicting timeline is rejected | Integration |

---

## 10. Theoretical Foundations

### 10.1 Allen's Interval Algebra (Allen 1983)

The 13 relations form a jointly exhaustive and mutually exclusive (JEME) partition of all
possible pairwise temporal relationships. The composition table (13×13 = 169 entries, each a
subset of the 13 relations) enables constraint propagation — discovering implied temporal
relationships from explicitly stated ones.

**Key property for Roko**: Allen's algebra is *qualitative* — it reasons about temporal
ordering without requiring exact timestamps. This matters because many of Roko's temporal
facts are qualitative ("the refactor happened before the release") rather than exact.

### 10.2 Event Calculus (Kowalski & Sergot 1986)

The event calculus provides a formal logic for reasoning about actions and their effects on the
world. The three core axioms (Initiates, Terminates, HoldsAt) plus the law of inertia (fluents
persist until terminated) give a complete framework for tracking how the system's state evolves.

**Key property for Roko**: The event calculus handles the **frame problem** — the question of
what *doesn't* change when something happens. By the law of inertia, only explicitly terminated
fluents change. This prevents the combinatorial explosion of tracking every unchanged property.

### 10.3 Temporal Knowledge Graphs (Rasmussen et al. 2025)

The Zep/Graphiti architecture demonstrated that temporal KGs with a three-tier hierarchy
(episodes, entities, communities) outperform flat memory systems on deep retrieval tasks
(94.8% vs. 93.4% on Deep Memory Retrieval benchmark, with 90% latency reduction).

arXiv:2509.15464 (2025) showed that LLMs augmented with evolving KGs improved temporal
reasoning from 18.6% to 37.0% accuracy on benchmark tasks, matching models 80× their size.

---

## Cross-References

- [01-naming-and-glossary](./01-naming-and-glossary.md) — Canonical naming for HDC fingerprint and related terms
- [02-engram-data-type](./02-engram-data-type.md) — Engram struct that gains temporal intervals and HDC fingerprints
- [07-substrate-trait](./07-substrate-trait.md) — Native similarity queries over fingerprinted Engrams
- [04-decay-variants](./04-decay-variants.md) — Decay system modulated by temporal stability
- [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md) — Neuro knowledge tiers enhanced by TKG
- [18-decay-tier-matrix](./18-decay-tier-matrix.md) — Decay-tier interactions with temporal modulation
- [25-attention-as-currency](./25-attention-as-currency.md) — Temporal recency as auction bid modifier
- [26-cognitive-immune-system](./26-cognitive-immune-system.md) — Temporal consistency as immune check
- [28-emergent-goal-structures](./28-emergent-goal-structures.md) — Temporal patterns trigger goal formation
- [Topic 05: Learning](../05-learning/INDEX.md) — Episode logging becomes temporal episodes
- [Topic 06: Neuro](../06-neuro/INDEX.md) — Knowledge store wrapped in TKG
- [Topic 15: Code Intelligence](../15-code-intelligence/INDEX.md) — Symbol graph gains temporal validity
- `see tmp/refinements/11-hyperdimensional-substrate.md`
