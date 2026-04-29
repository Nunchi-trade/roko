# Signal Algebra

> Depth for [01-SIGNAL.md](../../unified/01-SIGNAL.md). Derives the algebraic structure of Signal and Pulse as a semiring, showing how composition, lineage, and graduation form a coherent formal system.

---

## 1. The Core Claim

Signal and Pulse are not just "two data shapes." They form a **semiring** under two operations:

- **Bind** (notation: `a * b`): associate two Signals into a role-filler pair. Produces a new HDC vector that is dissimilar to both inputs but recoverable by unbinding.
- **Bundle** (notation: `a + b`): merge multiple Signals into a composite that is similar to all inputs. The composite *is* the Compound kind at the type level.

These operations satisfy the semiring axioms:
- `(Signal, +, 0_bundle)` is a commutative monoid (identity is the zero vector)
- `(Signal, *, 0_bind)` is a monoid (identity is the zero vector under XOR)
- `*` distributes over `+` (approximately, within HDC noise margins)
- The zero vector annihilates under bind

The semiring structure is not ornamental. It means that Signal composition is *lawful* -- you can reason about it algebraically instead of testing every combination.

---

## 2. The Signal Struct as an Algebraic Object

See [01-SIGNAL.md](../../unified/01-SIGNAL.md) &sect;2 for the full struct. The algebraic core lives in three fields:

```rust
pub struct Signal {
    pub content_hash: ContentHash,     // identity in the hash monoid
    pub hdc_fingerprint: HdcVector,    // carrier of algebraic operations
    pub source: Vec<SignalRef>,        // lineage DAG edges
    pub kind: Kind,                    // type tag, composable via Compound
    // ... remaining fields
}
```

The `content_hash` participates in the **lineage monoid** (append-only DAG).
The `hdc_fingerprint` participates in the **vector semiring** (bind + bundle).
The `kind` participates in the **kind lattice** (flat kinds join into Compound).

These three algebraic structures are independent but interact at composition boundaries.

---

## 3. Bind -- The Multiplicative Operation

Bind associates two Signals, producing a vector that encodes the *relationship* between them rather than either Signal alone.

```rust
/// Bind: XOR in HDC space.
/// Produces an association vector dissimilar to both inputs.
///
/// Properties:
///   a * a = 0          (self-inverse)
///   a * b = b * a      (commutative)
///   (a * b) * c = a * (b * c)  (associative)
///   a * 0 = a          (identity)
///
/// Abelian group under XOR.
pub fn bind(a: &HdcVector, b: &HdcVector) -> HdcVector {
    HdcVector::xor(a, b)
}

/// Unbind: recover b from (a * b) given a.
/// Because XOR is self-inverse: a * (a * b) = b.
pub fn unbind(key: &HdcVector, bound: &HdcVector) -> HdcVector {
    HdcVector::xor(key, bound)
}
```

### 3.1 Bind as Role-Filler Encoding

The classic VSA use of bind: encode structured records as single vectors.

```rust
/// Encode a Signal's score profile as a single HDC vector.
///
/// Each axis is bound to a role vector, then all role-filler pairs
/// are bundled into a single record vector.
pub fn encode_score_record(score: &Score) -> HdcVector {
    let roles = ScoreRoles::global();  // pre-computed role vectors
    let pairs = [
        bind(&roles.confidence, &quantize(score.confidence)),
        bind(&roles.novelty,    &quantize(score.novelty)),
        bind(&roles.quality,    &quantize(score.quality)),
        bind(&roles.relevance,  &quantize(score.relevance)),
        bind(&roles.utility,    &quantize(score.utility)),
    ];
    bundle(&pairs)
}

/// Recover the confidence quantization level from a record vector.
pub fn probe_confidence(record: &HdcVector, roles: &ScoreRoles) -> HdcVector {
    unbind(&roles.confidence, record)
    // Then find nearest quantization level by Hamming distance
}
```

### 3.2 Bind for Association

Bind also encodes *associations between Signals*. If Signal A (a compiler error) is causally related to Signal B (a fix), the bound vector `A * B` encodes "A caused B" as a single vector. Store that vector as a `Kind::CausalLink` Signal. Later, given a new error similar to A, unbind to recover B-like vectors pointing toward potential fixes.

```rust
/// Create a causal link Signal from two Signals.
pub fn create_causal_link(cause: &Signal, effect: &Signal) -> Signal {
    let association = bind(&cause.hdc_fingerprint, &effect.hdc_fingerprint);
    Signal::builder(Kind::CausalLink)
        .hdc_fingerprint(association)
        .source(vec![cause.ref_(), effect.ref_()])
        .build()
}

/// Given a new cause-like Signal, probe causal links for likely effects.
pub fn probe_effects(
    new_cause: &Signal,
    causal_links: &[Signal],
) -> Vec<(SignalRef, f32)> {
    causal_links.iter().filter_map(|link| {
        let predicted_effect = unbind(&new_cause.hdc_fingerprint, &link.hdc_fingerprint);
        // Search Store for Signals similar to predicted_effect
        // Return matches above threshold
        Some((link.ref_(), similarity))
    }).collect()
}
```

---

## 4. Bundle -- The Additive Operation

Bundle merges multiple Signals into a composite that is similar to all inputs. This is majority vote, not XOR.

```rust
/// Bundle: majority vote across bit positions.
/// Produces a vector similar to ALL inputs (centroid).
///
/// Properties:
///   a + b = b + a                 (commutative)
///   (a + b) + c ~ a + (b + c)    (approximately associative)
///   a + a = a                     (idempotent for odd counts)
///
/// Commutative semigroup (no inverse, no true identity).
/// With tie-breaking, approaches a commutative monoid.
pub fn bundle(vectors: &[HdcVector]) -> HdcVector {
    if vectors.is_empty() {
        return HdcVector::zero();
    }
    let n = vectors.len();
    let threshold = n / 2;
    let mut result = HdcVector::zero();

    for bit_pos in 0..HDC_DIMENSION {
        let ones: usize = vectors.iter()
            .filter(|v| v.get_bit(bit_pos))
            .count();
        if ones > threshold {
            result.set_bit(bit_pos, true);
        } else if ones == threshold && n % 2 == 0 {
            // Tie-break: random or use first vector's bit
            result.set_bit(bit_pos, vectors[0].get_bit(bit_pos));
        }
    }
    result
}
```

### 4.1 Bundle as Compound Kind

Bundle is the vector-level operation behind `Kind::Compound`. When you construct `Kind::compound([GateVerdict, TestResult])`, the resulting Signal's HDC fingerprint should be the bundle of a GateVerdict-like vector and a TestResult-like vector. This means the compound Signal will appear in similarity searches for *either* constituent kind.

```rust
/// When constructing a Compound-kinded Signal, bundle the
/// kind-specific role vectors into the fingerprint.
pub fn fingerprint_for_compound(
    kinds: &[Kind],
    payload_fingerprint: &HdcVector,
    kind_codebook: &KindCodebook,
) -> HdcVector {
    let kind_vectors: Vec<HdcVector> = kinds.iter()
        .map(|k| kind_codebook.vector_for(k))
        .collect();
    let kind_bundle = bundle(&kind_vectors);

    // Bind the kind-bundle with the payload fingerprint
    // so the compound vector encodes BOTH what it is AND what it contains
    bind(&kind_bundle, payload_fingerprint)
}
```

### 4.2 Bundle for Cluster Centroids

Bundle naturally produces cluster centroids. Given a set of Signals about a topic, their bundle is the "consensus vector" for that topic. This is how Memory (the durable knowledge store) could represent consolidated knowledge: not as a single best Signal, but as the bundle of all contributing Signals.

```rust
/// Consolidate a cluster of related Signals into a single
/// representative Signal (the centroid).
pub fn consolidate_cluster(
    signals: &[Signal],
    author: Author,
) -> Signal {
    let centroid = bundle(
        &signals.iter().map(|s| &s.hdc_fingerprint).collect::<Vec<_>>()
    );
    let sources: Vec<SignalRef> = signals.iter().map(|s| s.ref_()).collect();
    Signal::builder(Kind::Insight)
        .hdc_fingerprint(centroid)
        .source(sources)
        .author(author)
        .tag("consolidation", "bundle_centroid")
        .build()
}
```

---

## 5. Permute -- Temporal Ordering

Permutation encodes sequence position. Cyclic bit rotation by `k` positions produces a vector dissimilar to the original but recoverable by rotating back.

```rust
/// Permute: cyclic bit rotation for positional encoding.
///
/// Properties:
///   permute(a, 0) = a             (identity)
///   permute(permute(a, i), j) = permute(a, i+j)  (group under addition mod D)
///   similarity(a, permute(a, k)) ~ 0.5 for k > 0  (near-orthogonal)
pub fn permute(v: &HdcVector, positions: usize) -> HdcVector {
    v.rotate_left(positions % HDC_DIMENSION)
}
```

Permutation encodes ordered sequences of Signals -- for example, the sequence of agent turns in an episode, or the steps in a plan. The sequence vector preserves positional information that bundle alone would lose.

```rust
/// Encode an ordered sequence of Signals.
/// Position-encodes each Signal, then bundles.
pub fn encode_sequence(signals: &[Signal]) -> HdcVector {
    let positioned: Vec<HdcVector> = signals.iter().enumerate()
        .map(|(i, s)| permute(&s.hdc_fingerprint, i))
        .collect();
    bundle(&positioned)
}

/// Probe: "what was at position k in this sequence?"
pub fn probe_position(
    sequence_vec: &HdcVector,
    position: usize,
) -> HdcVector {
    // Rotate the sequence vector BACK by position
    // The result will be most similar to the Signal originally at that position
    permute(sequence_vec, HDC_DIMENSION - (position % HDC_DIMENSION))
}
```

---

## 6. The Semiring Laws

Collecting the algebraic properties:

| Law | Bind (*) | Bundle (+) |
|---|---|---|
| Closure | HdcVector -> HdcVector | HdcVector -> HdcVector |
| Associative | Exact | Approximate (noise accumulates) |
| Commutative | Yes | Yes |
| Identity | Zero vector (all 0s) | None (semigroup) |
| Inverse | Self-inverse: a * a = 0 | None (lossy) |
| Idempotent | No: a * a = 0 (not a) | Approximately: a + a ~ a |
| Distributive | a * (b + c) ~ (a * b) + (a * c) | Approximate |

The approximate associativity and distributivity of bundle is the key engineering constraint. At 10,240 bits, the approximation error is small enough for practical use (the probability of a bit flip from noise is approximately `1/sqrt(D)` ~ 1% per operation), but it means that:

- Bundling more than ~100 vectors accumulates enough noise to degrade similarity
- The semiring laws hold *in expectation*, not bit-for-bit
- Hash-based identity (ContentHash) is exact; HDC-based similarity is approximate

This split is fundamental: **identity is algebraically exact (hash monoid), similarity is algebraically approximate (vector semiring)**.

---

## 7. Graduation and Projection as Functors

See [01-SIGNAL.md](../../unified/01-SIGNAL.md) &sect;1 for the two mediums. Graduation and projection are the only bridges between Pulse and Signal. Algebraically, they are functors between categories:

```
                graduate
    Pulse ──────────────────► Signal
      |                          |
      | Bus transport            | Store persistence
      |                          |
      ▼                          ▼
    Ring buffer               JSONL + HDC index
```

### 7.1 Graduation as Enrichment Functor

Graduation maps a Pulse into a Signal by adding the fields Pulse lacks:

```rust
/// Graduation is a functor: Pulse -> Signal.
///
/// It preserves:
///   - kind (unchanged)
///   - body/payload (unchanged)
///   - temporal ordering (emitted_at_ms -> created_at)
///
/// It adds:
///   - content_hash (computed from payload)
///   - hdc_fingerprint (computed from payload + kind)
///   - score (initial, from Score protocol Cells)
///   - balance (initial 1.0, enters demurrage)
///   - source lineage (from lineage_hint + graduation context)
///   - provenance (from PulseSource + graduation policy)
///   - tier (initial Transient)
impl Pulse {
    pub fn graduate(
        &self,
        provenance: Provenance,
        initial_balance: f64,
        score: Score,
        tags: Vec<String>,
    ) -> Signal {
        Signal {
            id: SignalId::new(),
            content_hash: Signal::compute_hash(&self.body),
            kind: self.kind.clone(),
            payload: self.body.clone(),
            score,
            confidence: score.confidence,
            balance: initial_balance,
            demurrage_paid: 0.0,
            last_touched_at: Utc::now(),
            tier: Tier::Transient,
            created_at: DateTime::from_timestamp_millis(self.emitted_at_ms),
            source: self.lineage_hint.iter()
                .map(|h| SignalRef::from_hash(*h))
                .collect(),
            provenance,
            hdc_fingerprint: encode_signal_from_parts(&self.kind, &self.body),
            author: Author::from_pulse_source(&self.source),
            tags,
            schema: TypeSchema::infer(&self.body),
        }
    }
}
```

### 7.2 Projection as Forgetful Functor

Projection is the reverse: a Signal is projected onto a Pulse by *stripping* the durable fields.

```rust
/// Projection is a forgetful functor: Signal -> Pulse.
///
/// It preserves:
///   - kind
///   - body
///   - created_at -> emitted_at_ms
///
/// It forgets:
///   - content_hash (Pulses are addressed by (topic, seq))
///   - hdc_fingerprint
///   - score, balance, tier, demurrage
///   - full lineage (collapsed to lineage_hint)
///   - provenance (collapsed to PulseSource)
impl Signal {
    pub fn to_pulse(&self, topic: Topic, seq: u64) -> Pulse {
        Pulse {
            seq,
            topic,
            kind: self.kind.clone(),
            body: self.payload.clone(),
            emitted_at_ms: self.created_at.timestamp_millis(),
            source: PulseSource::from_author(&self.author),
            lineage_hint: Some(self.content_hash),
            trace_id: None,
        }
    }
}
```

### 7.3 The Round-Trip Property

Graduation followed by projection should not destroy information that projection preserves:

```
project(graduate(pulse)) preserves { kind, body, emitted_at_ms }
```

But `graduate(project(signal))` produces a *different* Signal (new content_hash, new id, new fingerprint computation), because graduation is enrichment, not reconstruction. This asymmetry is intentional: the projection functor is lossy, and the graduation functor adds information that cannot be recovered from the Pulse alone.

---

## 8. The Lineage DAG as a Free Category

The `source: Vec<SignalRef>` field on each Signal defines edges in a DAG. This DAG is a **free category** generated by the Signals:

- **Objects**: Signals (identified by ContentHash)
- **Morphisms**: lineage edges (A is in B's `source` means "A contributed to B")
- **Composition**: transitive closure (if A -> B -> C, then A transitively contributed to C)
- **Identity**: each Signal has an identity morphism (it contributed to itself)

```rust
/// Walk the lineage DAG to compute the full ancestry of a Signal.
///
/// The ancestry set forms a cone in the free category:
/// for every ancestor A and the target Signal T, there is a
/// path A ->* T through the lineage edges.
pub async fn ancestry(
    store: &dyn Store,
    target: &Signal,
) -> Vec<Signal> {
    let mut visited = HashSet::new();
    let mut queue: VecDeque<SignalRef> = target.source.iter().cloned().collect();
    let mut ancestors = Vec::new();

    while let Some(ref_) = queue.pop_front() {
        if !visited.insert(ref_.content_hash) {
            continue;
        }
        if let Some(ancestor) = store.get(&ref_.id).await? {
            queue.extend(ancestor.source.iter().cloned());
            ancestors.push(ancestor);
        }
    }
    ancestors
}

/// Compute the autocatalytic score: how many downstream Signals
/// were derived from this one?
///
/// High autocatalytic score = this Signal was generative.
/// This is the out-degree of the Signal in the reversed DAG.
pub async fn autocatalytic_score(
    store: &dyn Store,
    signal: &Signal,
) -> usize {
    // Query Store for all Signals whose source[] contains this hash
    let descendants = store.query(StoreQuery {
        lineage_contains: Some(signal.content_hash),
        ..Default::default()
    }).await?;
    descendants.len()
}
```

### 8.1 Lineage Laws

For the DAG to remain well-formed, these laws must hold:

1. **Acyclicity**: No Signal can be in its own transitive ancestry. Enforced structurally because ContentHash is computed at build time before the Signal is stored; a Signal cannot reference its own hash.

2. **Hash stability**: If Signal A is in B's `source`, then A's content_hash must be valid and resolvable in Store. Dangling references (A was pruned but B still references it) are allowed but should emit a warning on lineage traversal.

3. **Monotonic growth**: The DAG only grows. Signals are append-only; lineage edges are never removed. Even cold-storage archival preserves the content_hash so lineage remains traversable.

4. **Bounded fan-in**: A single Signal's `source` should not exceed a practical limit (32 is a reasonable cap). A Signal derived from 100+ parents is a design smell -- use intermediate consolidation Signals.

---

## 9. Compound Kinds as Join in a Lattice

The Kind system forms a **join-semilattice** where Compound is the join operation:

```
         Compound([A, B, C])
        /        |        \
  Compound([A,B]) Compound([A,C]) Compound([B,C])
      / \          / \          / \
     A   B        A   C        B   C
```

The lattice bottom is `Kind::Custom("empty_compound")` (the error state). There is no lattice top (no "every kind at once"). The join operation is `Kind::compound()`:

```rust
/// Kind lattice join: a \/  b = compound([a, b])
///
/// Properties:
///   a \/ a = a                    (idempotent)
///   a \/ b = b \/ a              (commutative)
///   (a \/ b) \/ c = a \/ (b \/ c)  (associative, via flatten)
///   a \/ bot = a                  (identity)
impl Kind {
    pub fn join(a: Kind, b: Kind) -> Kind {
        Kind::compound([a, b])
    }
}
```

This lattice structure means filter matching is a lattice-theoretic operation:

```
signal.kind.matches(filter)  iff  filter <= signal.kind
```

A `Kind::GateVerdict` filter matches `Kind::Compound([GateVerdict, TestResult])` because `GateVerdict <= Compound([GateVerdict, TestResult])` in the lattice ordering.

---

## 10. Scaling: The Algebra at 10x

What happens when Store holds millions of Signals?

### 10.1 HDC Similarity at Scale

At 10,240 bits per fingerprint:
- 1M Signals = 1.28 GB fingerprint data
- Brute-force SIMD scan: ~10 ms for full corpus (POPCNT on modern hardware)
- Approximate nearest neighbor (locality-sensitive hashing on binary vectors): sub-ms for top-K

The semiring operations remain O(D) per operation where D = 10,240. They do not degrade with corpus size. What degrades is *search* -- finding which Signals to operate on. That is Store's problem, not the algebra's.

### 10.2 Lineage DAG at Scale

At 1M Signals with average fan-in of 3:
- 3M edges in the DAG
- Full ancestry traversal of a deeply-derived Signal: potentially O(N) in the worst case
- Practical mitigation: depth-limited traversal with a configurable cap (default 100 ancestors)

```rust
pub struct LineageQuery {
    pub target: SignalRef,
    pub max_depth: usize,       // default 100
    pub max_ancestors: usize,   // default 1000
    pub kind_filter: Option<Kind>,
}
```

### 10.3 Compound Kind Explosion

With K distinct kinds and a max compound size of 4, the number of possible Compound kinds is `C(K, 2) + C(K, 3) + C(K, 4)`. For K = 30 (current kind count), that is `435 + 4,060 + 27,405 = 31,900` possible compounds. This is manageable. The cap at 4 prevents combinatorial explosion.

### 10.4 Bundle Noise at Scale

Bundling N vectors accumulates noise. The signal-to-noise ratio degrades as `sqrt(N)`:

```
P(bit_error) ~ 0.5 * erfc(sqrt(N) / 2)
```

For N = 100 vectors at D = 10,240: expected ~50 bit errors, or ~0.5% error rate. For N = 1000: ~5% error rate -- similarity queries become unreliable. Practical limit: **bundle no more than ~200 vectors** before re-encoding through a hierarchical bundle tree.

```rust
/// Hierarchical bundle for large sets.
/// Bundles groups of CHUNK_SIZE, then bundles the groups.
const CHUNK_SIZE: usize = 64;

pub fn hierarchical_bundle(vectors: &[HdcVector]) -> HdcVector {
    if vectors.len() <= CHUNK_SIZE {
        return bundle(vectors);
    }
    let chunks: Vec<HdcVector> = vectors
        .chunks(CHUNK_SIZE)
        .map(|chunk| bundle(chunk))
        .collect();
    hierarchical_bundle(&chunks)
}
```

---

## 11. What This Enables

1. **Compositional knowledge representation**: Bind + bundle encode structured records, causal links, and cluster centroids as single vectors. No external embedding model needed.

2. **Algebraic reasoning about Signal composition**: The semiring laws let you predict the outcome of composition without executing it. If `a * b * c` should recover `c` when probed with `a * b`, the algebra guarantees it (within noise bounds).

3. **Cross-domain analogy**: Bundle centroids from different domains can be compared via Hamming distance. A retry pattern in networking and a retry pattern in database operations will have similar bundle centroids if their structural roles are similar.

4. **Lawful graduation/projection**: The functor pair ensures that the bridge between Pulse and Signal is well-defined and its information-loss properties are explicit.

5. **Scalable lineage queries**: The free category structure supports efficient ancestry and descendant queries using standard graph algorithms.

---

## 12. Feedback Loops

- **Score -> Bind -> Store -> Score**: When a causal link Signal is created via bind, its utility score starts at 0. As the link is used to predict effects and those predictions are verified, its utility increases. High-utility causal links survive demurrage; low-utility ones decay. The algebra feeds the economics feeds the algebra.

- **Bundle -> Consolidation -> Bundle**: During delta-speed consolidation, clusters of related Signals are bundled into centroids. Those centroids become new Signals that themselves participate in future bundles. The hierarchy deepens over time, compressing knowledge.

- **Compound Kind -> Filter -> Compound Kind**: As consumers learn to filter by Compound kinds, the system discovers which kind combinations co-occur frequently. Those frequent compounds become first-class patterns -- the kind lattice evolves based on usage.

---

## 13. Open Questions

1. **Noise budget**: How many sequential bind and bundle operations can a vector sustain before similarity queries become unreliable? The theoretical bound is known (O(sqrt(D/N))), but the practical threshold in Roko's workload is empirical. Need production data.

2. **Bind vs. bundle for lineage encoding**: Should the lineage DAG be encoded in the HDC fingerprint? Currently lineage is structural (Vec of refs) and fingerprint is semantic. Encoding lineage into the fingerprint would make DAG-aware similarity possible but at the cost of fingerprint stability (lineage can change if parents are re-scored).

3. **Approximate associativity of bundle**: The tie-breaking strategy for even-count bundles affects reproducibility. Should tie-breaking be deterministic (using a seed) or should the system track the noise margin explicitly? If explicit, is there a practical vector-level "confidence" that degrades with operations?

4. **Functor composition**: Graduation and projection compose, but what about chains of graduation? If a Pulse graduates, the Signal projects back to a Pulse on a different topic, and that Pulse graduates again, the two Signals share payload but have different lineage and provenance. Is that a feature (multiple durable records of the same event) or a bug (phantom duplication)?

5. **Kind lattice depth**: With max compound size 4, the lattice has depth 4. Is this sufficient for all practical use cases? What about meta-kinds ("a compound of compounds") -- the current spec forbids nesting, but should it?
