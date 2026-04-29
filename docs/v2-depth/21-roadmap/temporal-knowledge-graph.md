# Temporal Knowledge Graph

> Depth for [27-temporal-knowledge-topology.md](../../docs/00-architecture/27-temporal-knowledge-topology.md). Redesigns temporal knowledge as Allen's 13 interval relations stored as a constraint network in Store, event calculus as Cells, and a 3-tier temporal knowledge graph as three Memory specializations at different timescales.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, demurrage, HDC fingerprints), [02-CELL](../../unified/02-CELL.md) (Cell, Verify protocol, Score protocol), [04-SPECIALIZATIONS](../../unified/04-SPECIALIZATIONS.md) (Store, Memory), [11-MEMORY-AND-KNOWLEDGE](../../unified/11-MEMORY-AND-KNOWLEDGE.md) (Memory tiers, tier progression)

---

## 1. The Problem: Timeless Signals in a Temporal World

Roko's Store currently treats knowledge as effectively atemporal. A Signal has a `created_at` timestamp and demurrage that controls weight decay. But the knowledge itself -- "Rust 1.91 is the minimum version", "alloy requires nightly" -- has no explicit temporal structure.

There is no way to express:
- **Validity windows**: "This API endpoint was active from March to June 2025"
- **Temporal ordering**: "The migration happened before the schema change"
- **Temporal overlap**: "While we were on version 2.x, the bug was present"
- **Causal chains**: "Because the CI pipeline broke, the release was delayed"

This limits the agent's ability to reason about change, history, and causation. A self-developing system must understand its own evolution.

In unified terms, the solution is three additions:
1. Allen's 13 interval relations as a **constraint network** stored in Store.
2. Event calculus (Kowalski-Sergot) as **Cells**: HoldsAt, Initiates, Terminates.
3. A 3-tier temporal knowledge graph as **three Memory specializations** at different timescales.

---

## 2. Allen's Interval Relations as a Constraint Network

Allen (1983) defined 13 mutually exclusive relations between time intervals. Every pair of temporal intervals satisfies exactly one. The relations form a JEME (jointly exhaustive, mutually exclusive) partition.

```rust
/// Allen's 13 temporal interval relations.
///
/// For intervals X = [x_start, x_end] and Y = [y_start, y_end].
/// The 13 relations + their inverses cover all possible pairwise orderings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AllenRelation {
    Before,       // x_end < y_start
    Meets,        // x_end == y_start
    Overlaps,     // x_start < y_start < x_end < y_end
    Starts,       // x_start == y_start, x_end < y_end
    During,       // y_start < x_start, x_end < y_end
    Finishes,     // x_end == y_end, x_start > y_start
    Equals,       // x_start == y_start, x_end == y_end
    After,        // inverse of Before
    MetBy,        // inverse of Meets
    OverlappedBy, // inverse of Overlaps
    StartedBy,    // inverse of Starts
    Contains,     // inverse of During
    FinishedBy,   // inverse of Finishes
}

/// A time interval with nanosecond precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemporalInterval {
    pub start: i64,  // Unix nanos
    pub end: i64,    // Unix nanos, or i64::MAX for "ongoing"
}

impl TemporalInterval {
    pub const ONGOING: i64 = i64::MAX;

    /// Determine the Allen relation between self and other.
    pub fn relation_to(&self, other: &Self) -> AllenRelation {
        // Direct comparison of endpoints determines relation.
        // All 13 cases are covered by exhaustive endpoint comparison.
        // See source spec for the full match tree.
        match (self.start.cmp(&other.start), self.end.cmp(&other.end),
               self.end.cmp(&other.start), other.end.cmp(&self.start)) {
            _ if self.end < other.start => AllenRelation::Before,
            _ if self.end == other.start => AllenRelation::Meets,
            _ if self.start > other.end => AllenRelation::After,
            _ if self.start == other.end => AllenRelation::MetBy,
            _ if self.start == other.start && self.end == other.end
                => AllenRelation::Equals,
            _ if self.start == other.start && self.end < other.end
                => AllenRelation::Starts,
            _ if self.start == other.start && self.end > other.end
                => AllenRelation::StartedBy,
            _ if self.end == other.end && self.start > other.start
                => AllenRelation::Finishes,
            _ if self.end == other.end && self.start < other.start
                => AllenRelation::FinishedBy,
            _ if self.start < other.start && self.end > other.start
                && self.end < other.end => AllenRelation::Overlaps,
            _ if other.start < self.start && other.end > self.start
                && other.end < self.end => AllenRelation::OverlappedBy,
            _ if self.start > other.start && self.end < other.end
                => AllenRelation::During,
            _ if self.start < other.start && self.end > other.end
                => AllenRelation::Contains,
            _ => unreachable!("all Allen relations covered"),
        }
    }
}
```

### The Constraint Network

Allen's algebra supports constraint propagation: if A is Before B and B Overlaps C, we can infer the possible relations between A and C. This is stored in Store as a constraint network over Signal validity intervals.

```rust
/// Temporal constraint network stored in Store.
///
/// Stores pairwise Allen relations between Signals and propagates
/// constraints when new relations are added. Inconsistency detection
/// catches temporal contradictions.
///
/// See [04-SPECIALIZATIONS.md](../../unified/04-SPECIALIZATIONS.md)
/// for Store partition semantics.
pub struct TemporalConstraintNetwork {
    /// Adjacency: (hash_a, hash_b) -> set of possible Allen relations.
    constraints: HashMap<(ContentHash, ContentHash), AllenRelationSet>,
    /// All known intervals.
    intervals: HashMap<ContentHash, TemporalInterval>,
}

/// Compact set of Allen relations (13 bits, one per relation).
/// Intersection is bitwise AND. Union is bitwise OR.
/// Empty set means temporal contradiction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllenRelationSet(u16);

impl AllenRelationSet {
    pub const ALL: Self = Self(0x1FFF);   // all 13 bits set
    pub const EMPTY: Self = Self(0);

    pub fn singleton(r: AllenRelation) -> Self { Self(1 << r as u16) }
    pub fn contains(&self, r: AllenRelation) -> bool { self.0 & (1 << r as u16) != 0 }
    pub fn intersect(&self, other: Self) -> Self { Self(self.0 & other.0) }
    pub fn is_empty(&self) -> bool { self.0 == 0 }
}
```

### Constraint Propagation Cell

```rust
/// Cell: Allen constraint propagation.
///
/// When a new temporal relation is asserted between two Signals,
/// this Cell propagates the constraint through the network using
/// the 13x13 composition table (Allen 1983, Table 2).
///
/// Returns Err if an inconsistency is detected (temporal contradiction).
/// Inconsistency means the asserted timeline is self-contradictory,
/// which is a signal of memory poisoning or data corruption.
pub struct AllenPropagationCell;

impl Cell for AllenPropagationCell {
    fn name(&self) -> &str { "allen-constraint-propagation" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let assertion = extract_temporal_assertion(&input[0])?;
        let network = ctx.store().get_temporal_network().await?;

        // Add the new constraint
        let (a, b, relation) = (assertion.signal_a, assertion.signal_b, assertion.relation);
        network.add(a, b, AllenRelationSet::singleton(relation));

        // Propagate using worklist algorithm
        let mut worklist = vec![(a, b)];
        while let Some((x, y)) = worklist.pop() {
            // For each third Signal z with known constraints to x or y
            for z in network.neighbors_of(x).chain(network.neighbors_of(y)) {
                if z == x || z == y { continue; }

                // Compose: R(x,z)_new = compose(R(x,y), R(y,z))
                let r_xy = network.get(x, y);
                let r_yz = network.get(y, z);
                let r_xz_composed = allen_compose(r_xy, r_yz);

                // Intersect with existing constraint
                let r_xz_old = network.get(x, z);
                let r_xz_new = r_xz_old.intersect(r_xz_composed);

                if r_xz_new.is_empty() {
                    // INCONSISTENCY: temporal contradiction detected
                    return Ok(vec![Signal::new(
                        Kind::Finding,
                        ThreatFinding {
                            id: Uuid::new_v4(),
                            class: ThreatClass::LineageMismatch,
                            affected_signals: vec![x, y, z],
                            confidence: 1.0,
                            severity: 0.7,
                            recommended_action: ContainmentAction::Reverify,
                            ..Default::default()
                        },
                    )]);
                }

                if r_xz_new != r_xz_old {
                    network.set(x, z, r_xz_new);
                    worklist.push((x, z));
                }
            }
        }

        // Persist updated network
        ctx.store().put_temporal_network(&network).await?;
        Ok(vec![])
    }
}

/// The 13x13 Allen composition table.
/// compose(R1, R2) returns the set of Allen relations that hold between
/// A and C given R1(A, B) and R2(B, C).
/// This table has 169 entries, each an AllenRelationSet.
fn allen_compose(r1: AllenRelationSet, r2: AllenRelationSet) -> AllenRelationSet {
    let mut result = AllenRelationSet::EMPTY;
    for i in 0..13 {
        if r1.contains(AllenRelation::from_index(i)) {
            for j in 0..13 {
                if r2.contains(AllenRelation::from_index(j)) {
                    result = result.union(COMPOSITION_TABLE[i][j]);
                }
            }
        }
    }
    result
}
```

**Key property**: Allen's algebra is qualitative. It reasons about temporal ordering without requiring exact timestamps. This matters because many of Roko's temporal facts are qualitative ("the refactor happened before the release") rather than exact. The constraint network handles both qualitative and quantitative constraints uniformly.

---

## 3. Event Calculus as Cells

The event calculus (Kowalski & Sergot 1986) models how the truth of properties ("fluents") changes in response to events. Three core axioms, each implemented as a Cell.

### Fluents and Events

```rust
/// A fluent: a time-varying property of the system.
///
/// Examples: "rust_version(1.91)", "ci_passing(true)", "feature_enabled(dark_mode)"
/// Fluents are stored as Signals with Kind::Fluent.
pub struct Fluent {
    pub id: FluentId,
    pub name: String,
    pub value: serde_json::Value,
    pub valid: TemporalInterval,
    pub initiated_by: Option<EventId>,
    pub terminated_by: Option<EventId>,
}

/// An event: a point-in-time occurrence that initiates or terminates fluents.
///
/// Events are stored as Signals with Kind::TemporalEvent.
pub struct TemporalEvent {
    pub id: EventId,
    pub timestamp: i64,
    pub description: String,
    pub signal_hash: Option<ContentHash>,
    pub initiates: Vec<FluentId>,
    pub terminates: Vec<FluentId>,
    pub caused_by: Vec<EventId>,
}
```

### HoldsAt Cell (Score Protocol)

```rust
/// Cell: HoldsAt(fluent, time) -> is the fluent true at time T?
///
/// Score protocol: takes a query Signal and returns a scored result.
/// A fluent holds at time T if:
///   - there exists an event E at E.timestamp <= T that initiates the fluent
///   - AND there is no event E' at E.timestamp < E'.timestamp <= T
///     that terminates the fluent
///
/// This is the law of inertia: fluents persist until terminated.
/// It solves the frame problem -- we only track what changes.
pub struct HoldsAtCell;

impl Cell for HoldsAtCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn name(&self) -> &str { "holds-at" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let query = extract_holds_at_query(&input[0])?;
        let fluent_id = query.fluent_id;
        let time = query.at_time;

        let fluent_history = ctx.store().query_fluent_history(fluent_id).await?;

        let holds = fluent_history.iter().any(|f| {
            f.valid.start <= time
                && (f.valid.end == TemporalInterval::ONGOING || f.valid.end > time)
        });

        let confidence = if holds { 1.0 } else { 0.0 };
        // For open intervals near the edge, confidence degrades
        let edge_penalty = fluent_history.iter()
            .filter(|f| (f.valid.end - time).abs() < 3600_000_000_000) // within 1 hour
            .count() as f64 * 0.1;

        Ok(vec![Signal::new(
            Kind::Score,
            HoldsAtResult {
                fluent_id,
                at_time: time,
                holds,
                confidence: (confidence - edge_penalty).max(0.0),
            },
        )])
    }
}
```

### Initiates and Terminates Cells (React Protocol)

```rust
/// Cell: Initiates -- when an event occurs, start the fluent.
///
/// React protocol: subscribes to event Signals and emits fluent-start Signals.
pub struct InitiatesCell;

impl Cell for InitiatesCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::React] }
    fn name(&self) -> &str { "initiates" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut outputs = Vec::new();
        for signal in &input {
            if let Some(event) = extract_temporal_event(signal) {
                for fluent_id in &event.initiates {
                    // Start the fluent at the event's timestamp
                    let fluent = Fluent {
                        id: *fluent_id,
                        name: ctx.store().fluent_name(*fluent_id).await?,
                        value: ctx.store().fluent_value(*fluent_id).await?,
                        valid: TemporalInterval {
                            start: event.timestamp,
                            end: TemporalInterval::ONGOING,
                        },
                        initiated_by: Some(event.id),
                        terminated_by: None,
                    };
                    outputs.push(Signal::new(Kind::Fluent, fluent));
                }
            }
        }
        Ok(outputs)
    }
}

/// Cell: Terminates -- when an event occurs, end the fluent.
pub struct TerminatesCell;

impl Cell for TerminatesCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::React] }
    fn name(&self) -> &str { "terminates" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut outputs = Vec::new();
        for signal in &input {
            if let Some(event) = extract_temporal_event(signal) {
                for fluent_id in &event.terminates {
                    // Close the open interval on the current fluent value
                    let mut fluent = ctx.store().get_current_fluent(*fluent_id).await?;
                    fluent.valid.end = event.timestamp;
                    fluent.terminated_by = Some(event.id);
                    outputs.push(Signal::new(Kind::Fluent, fluent));
                }
            }
        }
        Ok(outputs)
    }
}
```

---

## 4. Three-Tier Temporal Knowledge Graph as Memory Specializations

Inspired by Rasmussen et al. (2025, Zep/Graphiti), the temporal knowledge graph has three tiers, each a **Memory specialization** (see [11-MEMORY-AND-KNOWLEDGE](../../unified/11-MEMORY-AND-KNOWLEDGE.md)) operating at a different timescale.

### Tier 1: Episode Memory (Minutes to Hours)

Raw Signal sequences with bundled HDC fingerprints. "What happened."

```rust
/// Tier 1: Episode Memory.
///
/// Stores temporal episodes: ordered sequences of Signals within a time window.
/// Each episode carries a bundled HDC fingerprint (the centroid of its
/// member Signal fingerprints).
///
/// Timescale: minutes to hours. This is the fastest Memory tier.
/// Demurrage: standard rate. Episodes decay at the normal Signal rate.
pub struct EpisodeMemory {
    pub episodes: Vec<TemporalEpisode>,
    pub max_episodes: usize,  // FIFO eviction when exceeded
}

pub struct TemporalEpisode {
    pub id: Uuid,
    pub interval: TemporalInterval,
    pub signal_hashes: Vec<ContentHash>,
    /// Bundle of member Signal fingerprints.
    pub fingerprint: Option<HdcVector>,
    pub summary: Option<String>,
    pub causal_links: Vec<Uuid>,
}
```

### Tier 2: Entity Memory (Hours to Weeks)

Extracted entities with temporal properties and evolving centroids. "What exists."

```rust
/// Tier 2: Entity Memory.
///
/// Stores temporal entities: things with identity that persist through time.
/// Each entity maintains a running HDC centroid over the Signals that
/// mention or update it.
///
/// Timescale: hours to weeks.
/// Demurrage: half the standard rate. Entities persist longer than episodes.
pub struct EntityMemory {
    pub entities: HashMap<EntityId, TemporalEntity>,
}

pub struct TemporalEntity {
    pub id: EntityId,
    pub name: String,
    pub kind: EntityKind,
    /// Properties modeled as fluents (time-varying via event calculus).
    pub properties: Vec<FluentId>,
    /// Relationships with temporal validity.
    pub relationships: Vec<TemporalRelationship>,
    /// Running centroid from supporting Signal fingerprints.
    pub fingerprint: Option<HdcVector>,
    pub created: i64,
    pub last_seen: i64,
}

pub struct TemporalRelationship {
    pub source: EntityId,
    pub target: EntityId,
    pub relation_type: RelationType,
    pub valid: TemporalInterval,
    pub confidence: f64,
    pub evidence: ContentHash,
}
```

### Tier 3: Community Memory (Weeks to Months)

HDC-backed clusters of related entities. "What patterns exist."

```rust
/// Tier 3: Community Memory.
///
/// Stores temporal communities: clusters of entities that are tightly
/// related within overlapping time windows. Updated during Delta
/// consolidation.
///
/// Timescale: weeks to months.
/// Demurrage: quarter the standard rate. Communities are the most durable tier.
pub struct CommunityMemory {
    pub communities: Vec<TemporalCommunity>,
}

pub struct TemporalCommunity {
    pub id: Uuid,
    pub entities: Vec<EntityId>,
    /// Temporal intersection: when all entities co-exist.
    pub active_interval: TemporalInterval,
    /// Bundle-center for similarity-driven promotion.
    pub fingerprint: Option<HdcVector>,
    pub summary: Option<String>,
    /// Stability score: how long unchanged.
    pub stability: f64,
}
```

### HDC-Guided Tier Progression

Tier progression uses HDC fingerprints to decide when episodes should be promoted to entities and entities to communities.

```rust
/// HDC-guided tier progression.
///
/// 1. Episode Signals land with their own fingerprints.
/// 2. Delta consolidation groups temporally overlapping episodes
///    whose fingerprints fall within a similarity radius.
/// 3. The cluster center becomes a candidate entity centroid.
/// 4. As older episodes accumulate noise through demurrage,
///    the centroid remains close to the broad pattern while
///    drifting away from one-off details.
/// 5. Stable entity clusters are promoted to communities.
pub fn tier_progression(
    episodes: &EpisodeMemory,
    entities: &mut EntityMemory,
    communities: &mut CommunityMemory,
    similarity_threshold: f64,     // default: 0.7
    stability_threshold: f64,      // default: 0.8
    min_community_size: usize,     // default: 3
) {
    // Episode -> Entity: cluster temporally overlapping, HDC-similar episodes
    let clusters = cluster_by_temporal_overlap_and_hdc(
        &episodes.episodes, similarity_threshold
    );

    for cluster in clusters {
        if cluster.len() < 2 { continue; }

        // Compute centroid of the cluster's fingerprints
        let centroid = hdc_bundle(
            &cluster.iter()
                .filter_map(|ep| ep.fingerprint.as_ref())
                .collect::<Vec<_>>()
        );

        // Find or create the entity this cluster represents
        let entity_id = entities.find_or_create_by_centroid(
            &centroid, similarity_threshold
        );
        entities.update_centroid(entity_id, &centroid);
    }

    // Entity -> Community: cluster stable, co-temporal entities
    let entity_clusters = cluster_entities_by_overlap_and_hdc(
        &entities.entities, similarity_threshold
    );

    for cluster in entity_clusters {
        if cluster.len() < min_community_size { continue; }

        let stability = compute_cluster_stability(&cluster);
        if stability >= stability_threshold {
            let community = TemporalCommunity {
                id: Uuid::new_v4(),
                entities: cluster.iter().map(|e| e.id).collect(),
                active_interval: compute_intersection(&cluster),
                fingerprint: Some(hdc_bundle(
                    &cluster.iter()
                        .filter_map(|e| e.fingerprint.as_ref())
                        .collect::<Vec<_>>()
                )),
                summary: None, // generated during Delta
                stability,
            };
            communities.communities.push(community);
        }
    }
}
```

---

## 5. Temporal Decay Modulation: Demurrage x Temporal Validity

How does demurrage interact with temporal validity? Two effects:

1. **Standard demurrage** taxes idle Signals by age since last use, regardless of temporal validity. A Signal about "API v2 endpoints" loses balance if no one retrieves it.

2. **Temporal validity modulation** adjusts demurrage based on whether the Signal's temporal interval is still active. A Signal whose validity window has closed should decay faster (the information is historical). A Signal whose validity is ongoing should decay slower (the information is current).

```rust
/// Temporal decay modulation.
///
/// Combines standard demurrage with temporal validity awareness.
/// Active-interval Signals get slower demurrage.
/// Expired-interval Signals get faster demurrage.
/// Community stability further modulates the rate.
pub fn temporal_demurrage(
    signal: &Signal,
    now: i64,
    base_tax: f64,
    community_stability: f64,
) -> f64 {
    let interval = signal.temporal_interval();

    // Is the Signal's validity window still active?
    let temporal_modifier = if interval.end == TemporalInterval::ONGOING {
        0.5  // ongoing: half the normal tax rate
    } else if interval.end > now {
        0.7  // active but with known end: reduced tax
    } else {
        // Expired: increased tax, proportional to how long ago it expired
        let staleness = (now - interval.end) as f64 / 86400_000_000_000.0; // days
        1.0 + (staleness * 0.1).min(2.0) // up to 3x normal rate
    };

    // Community stability modulates further
    // Stable community -> slower decay (knowledge is structurally supported)
    let stability_modifier = 1.0 - (community_stability * 0.3);

    base_tax * temporal_modifier * stability_modifier
}
```

The interaction creates a natural lifecycle:
- **Fresh, active Signals**: low demurrage, high balance, prominent in retrieval.
- **Active but aging Signals**: moderate demurrage, gradually losing prominence.
- **Expired Signals**: high demurrage, rapidly losing balance, moving toward cold tier.
- **Expired but in stable community**: moderate demurrage (the community structure supports the Signal even though its specific validity has ended).

---

## 6. Temporal Consistency as a Verify Cell

New Signals that contradict the established timeline should be flagged.

```rust
/// Verify Cell: temporal consistency check.
///
/// When a new Signal asserts a temporal fact, check it against the
/// constraint network. If the assertion creates an inconsistency
/// (empty AllenRelationSet after propagation), the Signal fails
/// verification.
pub struct TemporalConsistencyVerify;

impl Cell for TemporalConsistencyVerify {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
    fn name(&self) -> &str { "temporal-consistency-verify" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let signal = &input[0];
        let network = ctx.store().get_temporal_network().await?;

        // Check if adding this Signal's interval creates a contradiction
        if let Some(interval) = signal.temporal_interval_opt() {
            let test_result = network.test_add(signal.hash(), interval);
            match test_result {
                Ok(()) => Ok(vec![Signal::verdict(
                    "temporal-consistency", true,
                    "No temporal contradictions detected".into(),
                )]),
                Err(contradiction) => Ok(vec![Signal::verdict(
                    "temporal-consistency", false,
                    format!(
                        "Temporal contradiction: {} conflicts with {} via {}",
                        contradiction.signal_a, contradiction.signal_b,
                        contradiction.via
                    ),
                )]),
            }
        } else {
            // Signal has no temporal interval; pass by default
            Ok(vec![Signal::verdict(
                "temporal-consistency", true,
                "No temporal assertion to check".into(),
            )])
        }
    }
}
```

---

## What This Enables

1. **Temporal reasoning**: agents can answer "what was true when?", "what caused what?", and "what changed between T1 and T2?"
2. **Contradiction detection**: the Allen constraint network catches temporal inconsistencies, which are signals of data corruption or memory poisoning.
3. **Natural Memory lifecycle**: three tiers with different demurrage rates create a knowledge hierarchy that mirrors how biological memory consolidates.
4. **HDC-guided consolidation**: tier progression uses semantic similarity, not just time, to decide what to promote. This prevents co-temporal but semantically unrelated episodes from being clustered.
5. **Causal chains**: the event calculus enables transitive causal reasoning through the `caused_by` graph.

## Feedback Loops

- **L1**: temporal consistency gate threshold adjusts via EMA based on contradiction rates.
- **L2**: route queries to the appropriate temporal tier based on the query's time horizon.
- **L3**: Delta consolidation runs tier progression, promoting episodes to entities and entities to communities.
- **L4**: structural proposals to adjust tier boundaries, similarity thresholds, and demurrage modulation rates.

## Open Questions

1. **Constraint network scale**: Allen constraint propagation is O(N^3) worst case. For 10,000 Signals, this is 10^12 operations. In practice, the network is sparse (most Signals have constraints with only a few others), but how sparse is "sparse enough"? The `max_entities` config bound (default: 10,000) is a hard cap, but is it the right one?
2. **Retroactive temporal facts**: when the system learns that a fact's validity window was different than originally recorded (e.g., "actually the API was active until July, not June"), how is the constraint network updated? Retroactive changes can cascade through propagation.
3. **Temporal prediction accuracy**: the PredictionQuery extrapolates fluent trends. How should prediction confidence degrade with horizon length? Linear regression on temporal trends is simple but may be wrong for non-linear phenomena.
4. **Cross-deployment temporal alignment**: if two deployments have different clocks or different temporal knowledge about the same entities, how are their temporal knowledge graphs merged? The Allen relations are clock-independent (qualitative), which helps, but entity identity must be resolved first.
5. **HDC fingerprint noise over time**: as Signals age, their exact episodic fingerprints become noisier. At what point does the noise exceed the signal, and should the tier progression stop trusting old fingerprints? The source spec suggests "fresh exact fingerprints for episode recall and bundled centroids for long-horizon semantic promotion" but the crossover threshold is not specified.
