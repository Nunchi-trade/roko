# HDC Algebra and Retrieval

> Depth for [06-MEMORY.md](../../unified/06-MEMORY.md). How hyperdimensional computing operations emerge as Cells, and retrieval as Pipeline Graphs over Store.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, HDC fingerprint, content-addressed identity), [02-CELL](../../unified/02-CELL.md) (Cell, Compose protocol, Score protocol, Store protocol), [03-GRAPH](../../unified/03-GRAPH.md) (Pipeline pattern, Graph), [06-MEMORY](../../unified/06-MEMORY.md) (Store, retrieval, demurrage)

---

## 1. The Problem HDC Solves

Agent systems need to answer a question that hashing cannot: "what do I already know that _resembles_ this?" Content hashes give exact identity. Embedding vectors give approximate similarity. But float embeddings are large (6 KB per vector at 1536-d float32), require GPU-heavy dot products, depend on a specific model version for consistency, and -- critically -- have no native compositionality. You cannot XOR two float embeddings to encode a relationship and then XOR again to recover a constituent. Float embeddings are opaque points in a learned manifold.

Hyperdimensional Computing (HDC) provides an alternative: **10,240-bit binary vectors** where similarity is Hamming distance, composition is bitwise XOR, aggregation is majority vote, and sequencing is cyclic rotation. The entire algebra runs in L1 cache with no floating point. A single comparison costs ~13 ns (XOR + POPCNT on 160 u64 words). 800,000 fingerprints fit in 1 GB of RAM with room to spare.

HDC is not a replacement for dense embeddings. It is a _complementary_ representation optimized for a different regime: structured, compositional, deterministic, and cheap. Where dense embeddings excel at semantic similarity in natural language, HDC excels at structural similarity in typed records -- precisely what a knowledge store full of scored, kinded, provenance-tracked Signals needs.

In the unified vocabulary, HDC is not a library bolted onto the side of Store. It is **infrastructure native to Signal** -- the `hdc_fingerprint` field is a first-class member of every Signal, and the four HDC operations are Cells that compose into Graphs for retrieval, factorization, and encoding.

---

## 2. The Vector

Every Signal carries a 10,240-bit fingerprint:

```rust
pub struct Signal {
    // ... identity, content, scoring, demurrage, lineage ...
    pub hdc_fingerprint: HdcVector,  // 10,240 bits = 160 x u64 = 1,280 bytes
    // ...
}
```

The vector lives in `roko-primitives/src/hdc.rs`:

```rust
pub struct HdcVector {
    bits: [u64; 160],
}
```

### 2.1 Statistical Foundation

Binary Spatter Codes (BSC, Kanerva 2009) have precise mathematical properties at D = 10,240:

| Property | Value | Derivation |
|---|---|---|
| Expected similarity of two random vectors | 0.500 (mu = D/2) | Each bit independent, P(agree) = 0.5 |
| Standard deviation | sigma = sqrt(D)/2 = 50.6 bits | Binomial(D, 0.5) |
| Coefficient of variation | 1/sqrt(D) = 0.00988 | sigma/mu, measure of noise floor width |
| P(random match > 55%) | < 10^-9 | Z = (0.55 - 0.5) * sqrt(D) / 0.5 = 10.12 |
| Bonferroni-safe threshold (100K vocab) | 0.526 (Z = 5.26) | FP rate 7.3e-8 per comparison, < 1% across 100K |
| Johnson-Lindenstrauss bound | D >= 8*ln(N)/epsilon^2 | For N=100K, epsilon=0.1: D >= 9,210. We use 10,240. |

The key threshold is **0.526**. Any pair of vectors with Hamming similarity above 0.526 shares genuine structure with overwhelming probability, even after testing against a vocabulary of 100,000 entries with Bonferroni correction. Below 0.526, the match is indistinguishable from random.

This threshold is a constant in the codebase:

```rust
/// crates/roko-primitives/src/codebook.rs
pub const RESONANCE_THRESHOLD: f32 = 0.526;
```

### 2.2 Vector Generation

Vectors are generated deterministically from byte seeds via FNV-1a hashing followed by splitmix64 expansion:

```rust
impl HdcVector {
    pub fn from_seed(seed: &[u8]) -> Self {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
        for &byte in seed {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        let mut bits = [0u64; 160];
        for word in &mut bits {
            *word = splitmix64(&mut hash);
        }
        Self { bits }
    }
}
```

Same seed produces the same vector, always. This is critical: role vectors like `V_domain`, `V_topic`, `V_kind` are stable across processes, machines, and time. A codebook generated on one node is identical to one generated on another, with no synchronization required.

Random vectors (for Signals whose fingerprint must be unique rather than deterministic) use UUID v4 as the seed source.

---

## 3. Four Operations as Four Cells

The unified vocabulary says: every computation is a Cell (Signals in, Signals out). The four HDC operations are no exception. Each is an atomic Cell with typed I/O, deterministic execution, and zero external dependencies.

### 3.1 BindCell -- XOR Association

Bind creates a vector that encodes the *relationship* between two Signals. The result is dissimilar to both inputs but recoverable by binding again with either input (since XOR is self-inverse).

```rust
/// Cell: HDC Bind (XOR).
///
/// Input: exactly 2 Signals.
/// Output: 1 Signal whose hdc_fingerprint is input[0].hdc XOR input[1].hdc.
///
/// Algebraic properties:
///   bind(a, a) = zero          (self-inverse)
///   bind(a, b) = bind(b, a)   (commutative)
///   bind(bind(a, b), c) = bind(a, bind(b, c))  (associative)
///   bind(a, zero) = a          (identity)
///
/// This is an abelian group under XOR. The group structure means
/// unbinding is free: unbind(key, bound) = bind(key, bound).
pub struct BindCell;

impl Cell for BindCell {
    fn name(&self) -> &str { "hdc.bind" }

    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Compose] }

    fn estimated_cost(&self) -> Option<Cost> {
        Some(Cost::microcents(0))  // pure bitwise, ~1 ns
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        ensure!(input.len() == 2, "BindCell requires exactly 2 input Signals");
        let bound = input[0].hdc_fingerprint.bind(&input[1].hdc_fingerprint);
        let output = Signal::builder(Kind::CausalLink)
            .hdc_fingerprint(bound)
            .source(vec![input[0].ref_(), input[1].ref_()])
            .build();
        Ok(vec![output])
    }
}
```

**Use cases**:
- Encode "cause -> effect" as a single vector (CausalLink Signals).
- Role-filler binding: `bind(V_domain, V_rust)` encodes "domain=Rust".
- Unbinding to probe: given a new cause, `bind(new_cause, causal_link)` recovers the predicted effect.

### 3.2 BundleCell -- Majority-Vote Aggregation

Bundle merges N Signals into a single centroid that is similar to all inputs. This is the "superposition" operation -- the result vector responds to similarity queries for any constituent.

```rust
/// Cell: HDC Bundle (majority vote).
///
/// Input: 1..N Signals.
/// Output: 1 Signal whose hdc_fingerprint is the majority-vote bundle.
///
/// Algebraic properties:
///   bundle(a, b) = bundle(b, a)               (commutative)
///   bundle(a, b, c) ~ bundle(bundle(a, b), c) (approximately associative)
///   bundle(a, a) = a                           (idempotent for odd counts)
///
/// Commutative semigroup. Approximate associativity degrades with
/// depth; for >50 vectors, use BundleAccumulator for numerical stability.
pub struct BundleCell;

impl Cell for BundleCell {
    fn name(&self) -> &str { "hdc.bundle" }

    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Compose] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        ensure!(!input.is_empty(), "BundleCell requires at least 1 input Signal");
        let refs: Vec<&HdcVector> = input.iter()
            .map(|s| &s.hdc_fingerprint)
            .collect();
        let bundled = HdcVector::bundle(&refs);
        let sources = input.iter().map(|s| s.ref_()).collect();
        let output = Signal::builder(Kind::Insight)
            .hdc_fingerprint(bundled)
            .source(sources)
            .build();
        Ok(vec![output])
    }
}
```

For large bundles (>50 vectors), a streaming variant uses `BundleAccumulator` to avoid materializing all vectors at once:

```rust
/// Streaming bundle accumulator. 40 KB heap (10,240 x i32 vote counts).
/// Supports weighted addition and multiplicative decay for temporal biasing.
pub struct BundleAccumulator {
    votes: Vec<i32>,  // 10,240 entries
    pub count: usize,
}

impl BundleAccumulator {
    pub fn add(&mut self, hv: &HdcVector);
    pub fn add_weighted(&mut self, hv: &HdcVector, weight: i32);
    pub fn decay(&mut self, factor: f32);  // multiplicative forgetting
    pub fn finish(&self) -> HdcVector;     // threshold votes at zero
}
```

The `DecayingBundleAccumulator` variant applies automatic temporal decay before each addition, biasing the bundle toward recent vectors. Its half-life is `-(ln 2) / ln(decay_factor)` additions; at `decay_factor = 0.95`, the half-life is ~13.5 additions.

### 3.3 PermuteCell -- Positional Encoding

Permute encodes sequence position by cyclic bit rotation. The rotated vector is near-orthogonal to the original (similarity ~0.5 for any shift > 0), but the rotation is invertible.

```rust
/// Cell: HDC Permute (cyclic bit rotation).
///
/// Input: 1 Signal + position parameter.
/// Output: 1 Signal with hdc_fingerprint rotated left by `positions` bits.
///
/// Properties:
///   permute(a, 0) = a                                  (identity)
///   permute(permute(a, i), j) = permute(a, i + j)     (group under addition mod D)
///   similarity(a, permute(a, k)) ~ 0.5 for k > 0      (near-orthogonal)
pub struct PermuteCell {
    pub positions: usize,
}

impl Cell for PermuteCell {
    fn name(&self) -> &str { "hdc.permute" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        ensure!(input.len() == 1, "PermuteCell requires exactly 1 input Signal");
        let permuted = input[0].hdc_fingerprint.permute(self.positions);
        let output = Signal::builder(input[0].kind.clone())
            .hdc_fingerprint(permuted)
            .source(vec![input[0].ref_()])
            .build();
        Ok(vec![output])
    }
}
```

**Use case**: Encode the ordered sequence of agent turns in an episode. Position-encode each turn, then bundle the positioned vectors. The result fingerprint captures both _what_ happened and _in what order_.

```rust
/// Encode an ordered sequence of Signals into a single fingerprint.
/// Each Signal is permuted by its position index, then all are bundled.
fn encode_sequence(signals: &[Signal]) -> HdcVector {
    let positioned: Vec<HdcVector> = signals.iter().enumerate()
        .map(|(i, s)| s.hdc_fingerprint.permute(i))
        .collect();
    let refs: Vec<&HdcVector> = positioned.iter().collect();
    HdcVector::bundle(&refs)
}
```

### 3.4 SimilarityCell -- Hamming Distance Query

Similarity computes the overlap between two vectors. Unlike the other three operations, it does not produce a vector -- it produces a scalar score.

```rust
/// Cell: HDC Similarity (Hamming).
///
/// Input: exactly 2 Signals.
/// Output: 1 Signal of Kind::Score whose payload contains the similarity value.
///
/// Cost: ~13 ns with AVX-512 (XOR 160 words + POPCNT).
/// In practice bounded by memory latency, not arithmetic.
pub struct SimilarityCell;

impl Cell for SimilarityCell {
    fn name(&self) -> &str { "hdc.similarity" }

    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        ensure!(input.len() == 2, "SimilarityCell requires exactly 2 input Signals");
        let sim = input[0].hdc_fingerprint.similarity(&input[1].hdc_fingerprint);
        let output = Signal::builder(Kind::Score)
            .payload(json!({ "similarity": sim }))
            .confidence(sim as f64)
            .build();
        Ok(vec![output])
    }
}
```

The implementation in `roko-primitives`:

```rust
impl HdcVector {
    pub fn similarity(&self, other: &Self) -> f32 {
        let mut differing_bits = 0u32;
        for (left, right) in self.bits.iter().zip(other.bits.iter()) {
            differing_bits += (left ^ right).count_ones();
        }
        1.0 - (f32::from(differing_bits as u16) / 10_240.0)
    }
}
```

### 3.5 The Semiring

These four operations form a **semiring over HdcVector**:

- `(HdcVector, bundle, zero_vector)` is a commutative monoid (additive).
- `(HdcVector, bind, zero_vector)` is an abelian group (multiplicative, since XOR is self-inverse).
- Bind distributes over bundle (approximately, within HDC noise margins).
- The zero vector annihilates under bind: `bind(a, zero) = a`.

The semiring structure means that compositions of HDC operations are _lawful_ -- you can reason about them algebraically rather than testing every combination. BindCell followed by BundleCell is a known pattern (role-filler encoding). BundleCell followed by SimilarityCell is a known pattern (centroid query). The Cell abstraction makes these compositions explicit and type-checked at the Graph level.

---

## 4. Encoding as a Compose Cell

Different domains need different encoding strategies. An episode fingerprint binds role vectors for `domain`, `topic`, `kind`, and `content`. A CausalLink fingerprint binds permuted cause and effect vectors. A Heuristic fingerprint encodes the `when` predicates and `then` action.

Rather than hard-coding each encoding scheme, encoding itself is a **Compose Cell** -- a parameterized Cell that accepts a Codebook and a schema, and produces an HdcVector from structured input.

```rust
/// Cell: domain-pluggable HDC encoder.
///
/// Takes a structured Signal and produces an HdcVector by:
/// 1. Looking up role vectors in the Codebook (or allocating them on demand).
/// 2. Looking up filler vectors for each field value.
/// 3. Binding each (role, filler) pair.
/// 4. Bundling all bound pairs into a single record vector.
///
/// The Codebook is a Cell-level parameter, not a global. Different
/// EncodeCell instances use different Codebooks for different domains.
pub struct EncodeCell {
    codebook: Arc<Mutex<Codebook>>,
    schema: EncodingSchema,
}

/// Defines which Signal fields map to which HDC roles.
pub struct EncodingSchema {
    pub role_fields: Vec<RoleField>,
}

pub struct RoleField {
    /// Name of the role (e.g., "domain", "topic", "kind").
    pub role_name: String,
    /// JSON path into the Signal payload to extract the filler value.
    pub payload_path: String,
    /// Whether this field is optional (missing fields are skipped).
    pub optional: bool,
}

impl Cell for EncodeCell {
    fn name(&self) -> &str { "hdc.encode" }

    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Compose] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let signal = &input[0];
        let mut codebook = self.codebook.lock();
        let mut pairs = Vec::new();

        for field in &self.schema.role_fields {
            let role_vec = codebook.get_or_allocate(&field.role_name).clone();

            let filler_value = extract_field(&signal.payload, &field.payload_path);
            match filler_value {
                Some(val) => {
                    let filler_seed = format!("{}:{}", field.role_name, val);
                    let filler_vec = HdcVector::from_seed(filler_seed.as_bytes());
                    pairs.push(role_vec.bind(&filler_vec));
                }
                None if field.optional => continue,
                None => return Err(CellError::MissingField(field.role_name.clone())),
            }
        }

        let refs: Vec<&HdcVector> = pairs.iter().collect();
        let encoded = HdcVector::bundle(&refs);

        let mut output = signal.clone();
        output.hdc_fingerprint = encoded;
        Ok(vec![output])
    }
}
```

### 4.1 Domain Codebooks

A `Codebook` maps symbolic names to deterministic vectors within a domain namespace:

```rust
pub struct Codebook {
    domain: String,
    symbols: HashMap<String, HdcVector>,
}

impl Codebook {
    /// Deterministic: same (domain, name) always produces the same vector.
    pub fn allocate(&mut self, name: impl Into<String>) -> &HdcVector {
        let name = name.into();
        self.symbols.entry(name.clone()).or_insert_with(|| {
            let seed = format!("{}:{}", self.domain, name);
            HdcVector::from_seed(seed.as_bytes())
        })
    }
}
```

The codebase ships with `CodingCodebook` (16 pre-allocated symbols: `compile_error`, `test_failure`, `borrow_check`, `refactor`, etc.). New domains register their own codebooks:

```rust
// Research domain codebook
let mut research = Codebook::new("research");
research.allocate("finding");
research.allocate("citation");
research.allocate("methodology");
research.allocate("hypothesis");

// Trading domain codebook
let mut trading = Codebook::new("trading");
trading.allocate("price_signal");
trading.allocate("volume_spike");
trading.allocate("regime_change");
```

Because codebooks are deterministic (same domain + name = same vector, always), they require no synchronization between nodes. A codebook generated on machine A is identical to one generated on machine B.

### 4.2 The Knowledge Encoding Pattern

The canonical encoding for a knowledge entry (the "role-filler record" pattern from Kanerva 2009):

```rust
/// Encode a knowledge Signal as an HDC record.
///
/// entry_hv = bundle(
///   bind(role_domain,  hv_rust),
///   bind(role_topic,   hv_borrow_checker),
///   bind(role_kind,    hv_insight),
///   bind(role_content, hv_content_fingerprint)
/// )
fn encode_knowledge_entry(
    entry: &Signal,
    codebook: &mut Codebook,
) -> HdcVector {
    let role_domain  = codebook.get_or_allocate("domain").clone();
    let role_topic   = codebook.get_or_allocate("topic").clone();
    let role_kind    = codebook.get_or_allocate("kind").clone();
    let role_content = codebook.get_or_allocate("content").clone();

    let domain_val = extract_domain(entry);
    let topic_val  = extract_topic(entry);

    let pairs = vec![
        role_domain.bind(&HdcVector::from_seed(domain_val.as_bytes())),
        role_topic.bind(&HdcVector::from_seed(topic_val.as_bytes())),
        role_kind.bind(&HdcVector::from_seed(entry.kind.as_str().as_bytes())),
        role_content.bind(&HdcVector::from_seed(&entry.content_hash.0)),
    ];

    let refs: Vec<&HdcVector> = pairs.iter().collect();
    HdcVector::bundle(&refs)
}
```

**CausalLink encoding** uses permutation to distinguish cause from effect:

```rust
/// CausalLink = bind(permute(cause_role, 1), cause_hv)
///          XOR bind(permute(effect_role, 2), effect_hv)
fn encode_causal_link(cause: &Signal, effect: &Signal) -> HdcVector {
    let cause_role  = HdcVector::from_seed(b"causal:cause");
    let effect_role = HdcVector::from_seed(b"causal:effect");

    let cause_part  = cause_role.permute(1).bind(&cause.hdc_fingerprint);
    let effect_part = effect_role.permute(2).bind(&effect.hdc_fingerprint);

    cause_part.bind(&effect_part)
}
```

---

## 5. Three-Tier Search as a Pipeline Graph

The Store protocol declares `query_similar`:

```rust
/// From roko-core/src/traits.rs
async fn query_similar(
    &self,
    fp: &HdcVector,
    radius: f32,
    limit: usize,
    ctx: &Context,
) -> Result<Vec<(ContentHash, f32)>>;
```

A brute-force scan works at small scale (10K vectors in <1 ms), but does not scale to millions of Signals. The three-tier search is a **Pipeline Graph** -- three Cells wired in sequence, each more expensive but more precise, with early termination when results are sufficient.

```
                     Pipeline Graph: hdc.search
  ┌──────────────┐      ┌──────────────────┐      ┌──────────────┐
  │  BloomFilter  │─hit─>│  Approximate     │─hit─>│  Exact       │
  │  Cell         │      │  Similarity Cell │      │  Similarity  │
  │  (8 hashes,  │      │  (first 2,048    │      │  Cell         │
  │   1M bits)   │      │   bits only)     │      │  (full 10,240│
  │              │      │                  │      │   bits)       │
  └──────┬───────┘      └──────┬───────────┘      └──────┬───────┘
         │ miss                │ miss                     │
         ▼                     ▼                          ▼
      skip entry           skip entry              emit ranked result
```

### 5.1 Tier 1: Bloom Filter Cell

The Bloom filter provides a fast exclusion test. If the filter says "no match," the entry is skipped with certainty. If it says "possible match," the entry advances to Tier 2.

```rust
/// Cell: Bloom filter pre-screen for HDC similarity.
///
/// Uses 8 independent hash functions over a 1M-bit array (125 KB).
/// False positive rate at 100K entries: ~2.1% (acceptable because
/// Tier 2 filters them out).
///
/// The filter does NOT store vectors. It stores digests of the
/// vector's high-order bits, so it answers "could this vector be
/// similar?" rather than "is this vector present?"
pub struct BloomFilterCell {
    filter: BloomFilter,
    hash_count: usize,     // 8
    bit_count: usize,      // 1_000_000
}

impl BloomFilterCell {
    /// Insert a vector's fingerprint digest into the filter.
    pub fn insert(&mut self, hv: &HdcVector) {
        // Hash the first 256 bits (4 u64 words) with 8 independent
        // hash functions. This is enough to distinguish vectors that
        // share high-order structure from random noise.
        let prefix = &hv.as_words()[..4];
        for i in 0..self.hash_count {
            let idx = bloom_hash(prefix, i) % self.bit_count;
            self.filter.set(idx);
        }
    }

    /// Test whether a query vector _could_ match any stored vector.
    pub fn might_contain(&self, hv: &HdcVector) -> bool {
        let prefix = &hv.as_words()[..4];
        (0..self.hash_count).all(|i| {
            let idx = bloom_hash(prefix, i) % self.bit_count;
            self.filter.get(idx)
        })
    }
}
```

**Cost**: O(1) per entry. 125 KB memory. The entire 100K-entry Bloom filter fits in L2 cache.

### 5.2 Tier 2: Approximate Similarity Cell

Candidates that pass the Bloom filter are compared using only the first 2,048 bits (32 u64 words, 256 bytes) of each vector. This is 5x cheaper than full comparison but captures ~80% of the discriminative information (the Johnson-Lindenstrauss bound holds at D=2,048 for vocabularies up to ~5K).

```rust
/// Cell: approximate HDC similarity using truncated vectors.
///
/// Compares only the first `prefix_bits` of each vector.
/// Default prefix: 2,048 bits (32 words).
///
/// Candidates below `threshold - margin` are rejected.
/// Candidates above `threshold + margin` are accepted directly.
/// Candidates in the margin band advance to Tier 3 (exact).
pub struct ApproximateSimilarityCell {
    prefix_words: usize,    // 32 (2,048 bits)
    threshold: f32,         // 0.526 (RESONANCE_THRESHOLD)
    margin: f32,            // 0.02 (uncertainty band)
}

impl Cell for ApproximateSimilarityCell {
    fn name(&self) -> &str { "hdc.approx_similarity" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let query = &input[0];
        let candidates = &input[1..];
        let mut results = Vec::new();

        for candidate in candidates {
            let approx_sim = hamming_prefix(
                &query.hdc_fingerprint,
                &candidate.hdc_fingerprint,
                self.prefix_words,
            );

            if approx_sim >= self.threshold + self.margin {
                // Confident match: accept without exact verification
                results.push((candidate.ref_(), approx_sim, Confidence::High));
            } else if approx_sim >= self.threshold - self.margin {
                // Uncertain: needs exact verification in Tier 3
                results.push((candidate.ref_(), approx_sim, Confidence::NeedsExact));
            }
            // Below threshold - margin: rejected
        }

        // Emit results as Signals for the next Pipeline stage
        Ok(pack_search_results(results))
    }
}

fn hamming_prefix(a: &HdcVector, b: &HdcVector, words: usize) -> f32 {
    let bits = words * 64;
    let mut diff = 0u32;
    for i in 0..words {
        diff += (a.as_words()[i] ^ b.as_words()[i]).count_ones();
    }
    1.0 - (diff as f32 / bits as f32)
}
```

**Cost**: O(prefix_words) per candidate. At 32 words, ~3 ns per comparison.

### 5.3 Tier 3: Exact Similarity Cell

Candidates that survive Tiers 1 and 2 are compared using all 10,240 bits. This is the SimilarityCell from section 3.4, applied to filtered candidates.

```rust
/// Cell: exact HDC similarity over full 10,240-bit vectors.
///
/// Only processes candidates that passed Bloom + approximate stages.
/// Emits ranked (ContentHash, f32) pairs above the threshold.
pub struct ExactSimilarityCell {
    threshold: f32,   // 0.526
    limit: usize,     // max results
}

impl Cell for ExactSimilarityCell {
    fn name(&self) -> &str { "hdc.exact_similarity" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let query = &input[0];
        let candidates = &input[1..];
        let mut scored: Vec<(SignalRef, f32)> = candidates.iter()
            .map(|c| {
                let sim = query.hdc_fingerprint.similarity(&c.hdc_fingerprint);
                (c.ref_(), sim)
            })
            .filter(|(_, sim)| *sim >= self.threshold)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        scored.truncate(self.limit);
        Ok(pack_ranked_results(scored))
    }
}
```

### 5.4 The Pipeline Graph

The three tiers compose into a Pipeline Graph:

```rust
/// Construct the three-tier HDC search Pipeline.
fn build_search_pipeline(config: &SearchConfig) -> Graph {
    let bloom = Node::cell(BloomFilterCell::new(config.bloom_bits, config.bloom_hashes));
    let approx = Node::cell(ApproximateSimilarityCell {
        prefix_words: config.approx_prefix_words,
        threshold: config.threshold,
        margin: config.approx_margin,
    });
    let exact = Node::cell(ExactSimilarityCell {
        threshold: config.threshold,
        limit: config.max_results,
    });

    Graph::pipeline("hdc.search", vec![bloom, approx, exact])
}
```

### 5.5 Hot-Swappable Search at Runtime

Because the search is a Graph (data, not compiled code), the pipeline stages can be replaced at runtime without restarting:

```rust
/// Swap the search pipeline's approximate stage to use a different
/// prefix length. Takes effect on the next query.
async fn tune_search_precision(
    graph_registry: &mut GraphRegistry,
    new_prefix_words: usize,
) {
    let mut pipeline = graph_registry.get_mut("hdc.search").unwrap();
    pipeline.replace_node("hdc.approx_similarity", Node::cell(
        ApproximateSimilarityCell {
            prefix_words: new_prefix_words,
            threshold: RESONANCE_THRESHOLD,
            margin: 0.02,
        },
    ));
}
```

This enables the system to adapt its search strategy based on observed query patterns:
- High miss rate at Tier 2? Widen the margin or increase prefix length.
- Bloom filter saturated? Rebuild with larger bit array.
- Under time pressure? Skip Tier 2 and go directly Bloom -> Exact.

---

## 6. ResonatorNetwork as a Loop

Resonator Networks (Frady et al. 2020) solve the inverse problem: given a bundled vector `B = bundle(bind(R1,F1), bind(R2,F2), ...)`, recover the original (role, filler) pairs. This is factorization of a superposition.

The algorithm is iterative:
1. Initialize each factor estimate randomly from its codebook.
2. For each factor, unbind all other current estimates from the bundle to isolate the target factor's contribution.
3. Find the nearest codebook entry to the isolated contribution.
4. Replace the estimate with the nearest entry.
5. Repeat until convergence or max iterations.

This is a **Loop** in the unified vocabulary: a Graph whose output feeds back into its input.

```rust
/// Cell: Resonator Network factorization.
///
/// Implements the iterative factorization algorithm as a Loop:
/// the output of each iteration feeds back as input to the next.
///
/// Convergence: similarity between successive estimates < convergence_threshold.
/// Max iterations: 50 (default). Typical: 5-20.
pub struct ResonatorCell {
    /// Per-role codebooks: maps role name -> candidate filler vectors.
    codebooks: BTreeMap<String, Vec<(String, HdcVector)>>,
    max_iterations: usize,
    convergence_threshold: f32,
}

impl Cell for ResonatorCell {
    fn name(&self) -> &str { "hdc.resonate" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let bundle = &input[0].hdc_fingerprint;
        let roles: Vec<&str> = self.codebooks.keys().map(|s| s.as_str()).collect();
        let n = roles.len();

        // Initialize: random codebook entries per role
        let mut estimates: Vec<HdcVector> = self.codebooks.values()
            .map(|entries| entries[0].1.clone())  // deterministic init: first entry
            .collect();

        for iteration in 0..self.max_iterations {
            let prev_estimates = estimates.clone();

            for i in 0..n {
                // Unbind all OTHER estimates from the bundle
                let mut residual = *bundle;
                for (j, est) in estimates.iter().enumerate() {
                    if j != i {
                        residual = residual.bind(est);
                    }
                }

                // Find nearest codebook entry to the residual
                let codebook = &self.codebooks[roles[i]];
                let (best_name, best_vec, best_sim) = codebook.iter()
                    .map(|(name, vec)| (name, vec, residual.similarity(vec)))
                    .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap())
                    .unwrap();

                estimates[i] = *best_vec;

                // Publish iteration progress as Pulse (observable via Lens)
                ctx.bus.publish(Pulse::new(
                    format!("hdc.resonate.iteration.{}.role.{}", iteration, roles[i]),
                    json!({
                        "iteration": iteration,
                        "role": roles[i],
                        "best_match": best_name,
                        "similarity": best_sim,
                    }),
                )).await;
            }

            // Check convergence: all estimates stable?
            let converged = estimates.iter().zip(prev_estimates.iter())
                .all(|(curr, prev)| curr.similarity(prev) > 1.0 - self.convergence_threshold);

            if converged {
                break;
            }
        }

        // Package results: (role, filler_name, confidence) triples
        let results: Vec<serde_json::Value> = roles.iter().enumerate()
            .map(|(i, role)| {
                let codebook = &self.codebooks[*role];
                let (name, _, sim) = codebook.iter()
                    .map(|(n, v)| (n.as_str(), v, estimates[i].similarity(v)))
                    .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap())
                    .unwrap();
                json!({ "role": role, "filler": name, "confidence": sim })
            })
            .collect();

        Ok(vec![Signal::builder(Kind::Finding)
            .payload(json!({ "factorization": results }))
            .source(vec![input[0].ref_()])
            .build()])
    }
}
```

### 6.1 Why Resonator Networks Matter for Store

1. **Knowledge deduplication**: Two Signals with high HDC similarity could be true duplicates (same structured content) or false positives (different content that happens to share bits). Factorization distinguishes them -- true duplicates produce the same role-filler decomposition.

2. **Constituent extraction**: An episode fingerprint encodes tools used, errors encountered, domain, outcome. Factorization recovers _which_ tools, _which_ errors, etc. This powers "what went wrong in episodes that look like this one?"

3. **Cross-domain transfer**: The `detect_cross_domain_resonance` function in `codebook.rs` finds patterns that structurally match across different codebooks. Factorization reveals _which substructure_ is shared, enabling transfer learning: "the retry pattern from networking also applies to database operations."

4. **Dream consolidation**: During L3 integration (phase 4 of dream cycles), Resonator Networks factorize bundled episode fingerprints to identify overlapping patterns across episodes. Patterns that appear in 5+ independent episodes are promoted to Heuristic Signals.

### 6.2 Observability via Lens

Because each iteration publishes a Pulse on the Bus, the Resonator Network's convergence is observable in real time. An Observe Cell (a Lens) can track:

- Number of iterations to convergence (indicates problem difficulty)
- Per-role confidence at convergence (indicates factorization quality)
- Divergent runs that hit `max_iterations` without converging (indicates either an ambiguous bundle or a codebook gap)

```rust
/// Lens: observe Resonator Network convergence.
/// Publishes summary metrics after each factorization.
pub struct ResonatorLens;

impl Cell for ResonatorLens {
    fn name(&self) -> &str { "lens.resonator" }

    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Observe] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Subscribe to hdc.resonate.iteration.* Pulses
        let mut iterations = 0;
        let mut min_confidence = 1.0_f32;

        // Aggregate iteration Pulses into summary metrics
        // ...

        ctx.bus.publish(Pulse::new(
            "lens.resonator.summary",
            json!({
                "iterations": iterations,
                "min_confidence": min_confidence,
                "converged": iterations < 50,
            }),
        )).await;

        Ok(vec![])
    }
}
```

---

## 7. ItemMemory as a Store Specialization

`ItemMemory` in `roko-primitives` is a named codebook with brute-force nearest-neighbor lookup:

```rust
pub struct ItemMemory {
    entries: HashMap<String, HdcVector>,
}

impl ItemMemory {
    pub fn insert(&mut self, name: impl Into<String>, hv: HdcVector);
    pub fn insert_seeded(&mut self, name: &str);  // deterministic from name
    pub fn nearest(&self, query: &HdcVector) -> Option<(&str, f32)>;
    pub fn top_k(&self, query: &HdcVector, k: usize) -> Vec<(&str, f32)>;
}
```

In the unified vocabulary, ItemMemory is a **Store specialization** -- a Store-protocol Cell whose `query_similar` is implemented via HDC comparison rather than text search:

```rust
/// ItemMemory as a Store Cell.
///
/// Provides the Store protocol (put, get, query, query_similar, prune)
/// with HDC-native similarity search. This is the foundational
/// building block for domain-specific knowledge stores.
impl Store for ItemMemory {
    async fn put(&self, signal: Signal) -> Result<()> {
        self.insert(signal.id.to_string(), signal.hdc_fingerprint);
        Ok(())
    }

    async fn query_similar(
        &self,
        fp: &HdcVector,
        radius: f32,
        limit: usize,
        _ctx: &Context,
    ) -> Result<Vec<(ContentHash, f32)>> {
        let results = self.top_k(fp, limit)
            .into_iter()
            .filter(|(_, sim)| *sim >= radius)
            .map(|(name, sim)| (ContentHash::from_str(name).unwrap(), sim))
            .collect();
        Ok(results)
    }
}
```

The `PatternStore` in `codebook.rs` extends this with labeled patterns, observation counts, and source-domain tracking:

```rust
pub struct PatternStore {
    patterns: Vec<StoredPattern>,
}

pub struct StoredPattern {
    pub label: String,
    pub fingerprint: HdcVector,
    pub observation_count: u64,
    pub source_domain: String,
}

impl PatternStore {
    pub fn query_similar(&self, probe: &HdcVector, threshold: f32) -> Vec<(&str, f32)>;
    pub fn nearest(&self, probe: &HdcVector) -> Option<(&str, f32)>;
}
```

This is the pattern store used by `detect_cross_domain_resonance` to find structural similarities across domain boundaries -- a coding pattern that also appears in research, or a testing pattern that also appears in deployment.

---

## 8. HDC-Powered Store Queries

With HDC fingerprints as first-class Signal fields, the Store protocol gains similarity queries that work without any external embedding service.

### 8.1 query_similar on Store

The full implementation flow:

```
Client calls store.query_similar(probe_fingerprint, 0.526, 20, ctx)
  |
  v
Store routes to HDC search Pipeline Graph:
  |
  +--> BloomFilterCell: exclude impossible matches (O(1) per entry)
  |
  +--> ApproximateSimilarityCell: compare first 2,048 bits (~3 ns/entry)
  |
  +--> ExactSimilarityCell: full 10,240-bit comparison (~13 ns/entry)
  |
  v
Return ranked Vec<(ContentHash, f32)> above threshold
```

### 8.2 Probing Causal Links

Given a new error Signal, probe causal links to predict likely fixes:

```rust
/// Query: "what effects have followed causes similar to this?"
async fn predict_effects(
    store: &dyn Store,
    new_cause: &Signal,
    ctx: &Context,
) -> Vec<(Signal, f32)> {
    // 1. Find causal link Signals similar to the cause
    let causal_links = store.query(
        &Query::new().kind(Kind::CausalLink),
        ctx,
    ).await.unwrap();

    // 2. Unbind the cause from each link to predict the effect
    let mut predictions = Vec::new();
    for link in &causal_links {
        let predicted_effect_fp = new_cause.hdc_fingerprint.bind(&link.hdc_fingerprint);

        // 3. Find Signals similar to the predicted effect
        let matches = store.query_similar(
            &predicted_effect_fp,
            RESONANCE_THRESHOLD,
            5,
            ctx,
        ).await.unwrap();

        for (hash, sim) in matches {
            if let Some(signal) = store.get(&hash, ctx).await.ok().flatten() {
                predictions.push((signal, sim));
            }
        }
    }

    predictions.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
    predictions.dedup_by(|a, b| a.0.content_hash == b.0.content_hash);
    predictions
}
```

### 8.3 AntiKnowledge Enforcement

When a new Signal is ingested, its HDC fingerprint is checked against all AntiKnowledge entries:

```rust
/// Check a new Signal against AntiKnowledge entries in Store.
/// Returns the action to take: Accept, Discount, or Reject.
async fn check_anti_knowledge(
    store: &dyn Store,
    new_signal: &Signal,
    ctx: &Context,
) -> AntiKnowledgeAction {
    let anti_entries = store.query(
        &Query::new().kind(Kind::AntiKnowledge),
        ctx,
    ).await.unwrap_or_default();

    let mut worst_sim = 0.0_f32;
    for anti in &anti_entries {
        let sim = new_signal.hdc_fingerprint.similarity(&anti.hdc_fingerprint);
        worst_sim = worst_sim.max(sim);
    }

    match worst_sim {
        s if s >= 0.9 => AntiKnowledgeAction::Reject,
        s if s >= 0.7 => AntiKnowledgeAction::Discount(0.5),  // halve initial balance
        s if s >= 0.5 => AntiKnowledgeAction::Warn,
        _ => AntiKnowledgeAction::Accept,
    }
}
```

---

## 9. Cybernetic Loops

### 9.1 Index Health Lens

An Observe Cell monitors the HDC index's statistical health:

```rust
/// Lens: HDC index health metrics.
///
/// Observes:
/// - Mean pairwise similarity (should be ~0.5 for well-distributed vectors)
/// - Bloom filter false positive rate (should be < 5%)
/// - Codebook utilization (symbols allocated vs. symbols actively used)
/// - Approximate-to-exact promotion rate (should be < 20%)
pub struct HdcIndexHealthLens {
    sample_size: usize,  // 1000 (random sample for pairwise stats)
}

impl Cell for HdcIndexHealthLens {
    fn name(&self) -> &str { "lens.hdc_index_health" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let all_vectors = ctx.store.sample_fingerprints(self.sample_size).await?;

        // Mean pairwise similarity: should be ~0.5
        let mut sum = 0.0_f64;
        let mut count = 0u64;
        for i in 0..all_vectors.len() {
            for j in (i + 1)..all_vectors.len() {
                sum += all_vectors[i].similarity(&all_vectors[j]) as f64;
                count += 1;
            }
        }
        let mean_sim = if count > 0 { sum / count as f64 } else { 0.5 };

        // Detect drift: mean similarity > 0.52 indicates distribution collapse
        let healthy = (mean_sim - 0.5).abs() < 0.02;

        ctx.bus.publish(Pulse::new("lens.hdc_index.health", json!({
            "mean_pairwise_similarity": mean_sim,
            "sample_size": self.sample_size,
            "healthy": healthy,
            "drift_warning": !healthy,
        }))).await;

        Ok(vec![])
    }
}
```

### 9.2 Threshold Tuning Loop

The similarity threshold (default 0.526) is derived from the Bonferroni-corrected false positive rate against 100K entries. But as the vocabulary grows, the optimal threshold changes. A Loop Cell tunes it:

```rust
/// Loop Cell: adaptive similarity threshold.
///
/// Observes false positive rate from search results (fraction of
/// returned results that were not actually relevant, as judged by
/// downstream gate outcomes). Adjusts threshold via EMA.
pub struct ThresholdTuningLoop {
    current_threshold: f32,    // starts at 0.526
    ema_alpha: f32,            // 0.05 (slow adaptation)
    min_threshold: f32,        // 0.51 (cannot go below noise floor)
    max_threshold: f32,        // 0.60 (cannot be so strict nothing matches)
}

impl ThresholdTuningLoop {
    /// Update threshold based on observed false positive rate.
    /// Called after each batch of search results is evaluated.
    pub fn update(&mut self, observed_fp_rate: f32, vocabulary_size: usize) {
        // Bonferroni-corrected theoretical FP rate
        let z_target = 5.26;  // P < 7.3e-8 per comparison
        let sigma = (10_240.0_f32).sqrt() / 2.0;
        let theoretical_threshold = 0.5 + z_target * sigma / 10_240.0;

        // Blend theoretical with observed
        let target = if observed_fp_rate > 0.01 {
            // Too many false positives: raise threshold
            self.current_threshold + self.ema_alpha * 0.005
        } else if observed_fp_rate < 0.001 && self.current_threshold > theoretical_threshold {
            // Too strict: lower toward theoretical
            self.current_threshold - self.ema_alpha * 0.002
        } else {
            self.current_threshold
        };

        self.current_threshold = target.clamp(self.min_threshold, self.max_threshold);
    }
}
```

### 9.3 Codebook Growth Management

When a codebook grows too large, the probability of accidental near-matches increases (birthday paradox in Hamming space). The system monitors codebook size and takes corrective action:

```rust
/// When codebook exceeds this size, trigger compression.
const CODEBOOK_GROWTH_THRESHOLD: usize = 50_000;

/// React Cell: codebook growth management.
///
/// Monitors codebook size. When it exceeds the threshold:
/// 1. Identify least-used symbols (lowest observation_count).
/// 2. Bundle groups of related low-use symbols into composite entries.
/// 3. Retire original entries (demote to cold storage).
///
/// This is the HDC equivalent of vocabulary compression.
pub struct CodebookGrowthReactor {
    threshold: usize,
    compression_target: f32,  // 0.7 (reduce to 70% of current size)
}
```

---

## 10. The Encoding Functor

The four HDC operations are natural transformations over the Signal type. Viewed categorically, encoding is a **Functor** from the category of structured data (JSON payloads with typed fields) to the category of HDC vectors (with bind/bundle/permute operations).

```
EncodeCell: StructuredSignal -> HdcVector

Properties:
  encode(A compose B) = encode(A) bind encode(B)    [composition preserved]
  encode(A merge B)   = encode(A) bundle encode(B)  [aggregation preserved]
  encode(sequence)    = permute-then-bundle          [order preserved]
```

This functorial property is what makes HDC encoding composable: the encoding of a compound Signal is the composition of its parts' encodings. You do not need to re-encode from scratch when a Signal is extended or combined.

Different domains plug into this functor by providing:
1. A **Codebook** (role vector assignments for their domain).
2. An **EncodingSchema** (which fields map to which roles).
3. Optionally, a **custom EncodeCell** for domain-specific encoding logic.

---

## What This Enables

1. **Similarity as infrastructure, not integration.** Every Signal has an HDC fingerprint from birth. `query_similar` works on any Store without external services, API keys, or model version dependencies.

2. **Compositional retrieval.** Because HDC operations are algebraic (semiring), you can compose queries: "find Signals similar to the _combination_ of this error AND that domain" by binding the two query vectors before searching. No special multi-vector query API needed.

3. **Causal probing.** CausalLink Signals encode cause-effect pairs as bound vectors. Given a new cause, unbinding predicts the effect, and Store search finds concrete Signals matching the prediction. This powers "what fix worked last time we saw this error?"

4. **Cross-domain resonance.** PatternStore comparison across codebooks reveals when structurally identical patterns appear in different domains. A retry pattern in networking and a retry pattern in database operations share structure that HDC makes measurable.

5. **Factorization for debugging.** ResonatorNetwork decomposes a complex episode fingerprint into its constituent roles and fillers. "This failed episode involved domain=rust, error_type=borrow_check, tool=cargo_clippy" -- extracted from a single 1,280-byte vector.

6. **Hot-swappable search.** The three-tier Pipeline Graph can be reconfigured at runtime -- wider Bloom filters, longer prefixes, adjusted thresholds -- without restarting the Store or re-indexing.

7. **Deterministic everywhere.** Same seed produces the same vector, always. No model version dependency. No embedding API drift. Codebooks are reproducible across machines with no synchronization.

---

## Feedback Loops

| Loop | What Observes | What Tunes | Timescale |
|---|---|---|---|
| **Threshold Tuning** | False positive rate from downstream gate outcomes | `RESONANCE_THRESHOLD` via EMA | Per-batch (minutes) |
| **Bloom Rebuild** | Bloom false positive rate from Tier 1 -> Tier 2 promotion rate | Bloom filter bit count and hash count | Per-hour |
| **Codebook Compression** | Codebook size, symbol usage counts | Vocabulary via bundling low-use symbols | Per-day |
| **Index Health Lens** | Mean pairwise similarity of sampled vectors | Alerts on distribution collapse (mean > 0.52) | Per-query-batch |
| **Prefix Length Tuning** | Approximate-to-exact promotion rate | `prefix_words` in ApproximateSimilarityCell | Per-hour |
| **Decay Integration** | Signal balance decay and reinforcement events | DecayingBundleAccumulator factor | Per-dream-cycle |

The loops are nested: the Threshold Tuning Loop operates per-batch, the Bloom Rebuild operates per-hour (informed by accumulated Threshold Tuning data), and the Codebook Compression operates per-day (informed by accumulated usage data). Each loop is a Graph that can be inspected, paused, or overridden via the control plane.

---

## Open Questions

1. **Is 10,240 bits optimal, or should dimensionality be adaptive?** The Johnson-Lindenstrauss bound says D >= 9,210 for N=100K at epsilon=0.1. At N=1M, the bound rises to ~11,000. Should the system support variable-width vectors (e.g., 8,192 for small stores, 16,384 for large ones)? The cost is complexity in the comparison path and storage heterogeneity. The benefit is tighter noise margins at scale.

2. **Online learning of role vectors.** Currently, role vectors are deterministic (seeded from the domain:name string). But a role like "domain" might benefit from adaptation: after observing that Rust and Go Signals cluster differently than Rust and TypeScript, the role vector for "domain" could be tuned to better separate the actually-observed categories. This is equivalent to online metric learning in HDC space. The risk is losing determinism and cross-node reproducibility.

3. **HDC drift as vocabulary evolves.** When new concepts are added to a codebook, the average pairwise similarity shifts. The Bloom filter and threshold were calibrated for the original vocabulary. Should the system periodically recalibrate? The Index Health Lens detects drift, but the corrective action (recalibrate vs. rebuild vs. accept) is not yet defined.

4. **Dense-HDC hybrid (DualEncoder).** The source material mentions a `DualEncoder` that blends HDC and dense embeddings with an alpha parameter (default 0.6). When should the system prefer HDC-only vs. hybrid? HDC excels at structural similarity (typed records, role-filler patterns); dense embeddings excel at semantic similarity (natural language meaning). A Route Cell that selects the encoder based on Signal kind would let both coexist.

5. **Resonator Network failure modes.** When factorization hits `max_iterations` without converging, the bundle may be ambiguous (multiple valid decompositions) or novel (no codebook entry matches). Should the system emit an AntiKnowledge Signal ("this bundle cannot be decomposed") or a Finding Signal ("codebook is missing a concept")? The latter creates a feedback loop where failed factorizations drive codebook expansion.

6. **Privacy-preserving HDC (PP-HDC).** After role-filler binding, the individual filler vectors are not recoverable from the bundled record without the role vectors. This provides one-way encoding similar to hashing. But is this sufficient for privacy guarantees in multi-tenant deployments? The threat model (who has access to role codebooks?) determines whether PP-HDC is meaningful or theater.

7. **Three-tier search bypass.** For stores under 10K entries, brute-force scan (< 1 ms) beats the three-tier pipeline (which has coordination overhead). Should the Pipeline Graph detect store size and short-circuit to brute-force? This is a Route decision, but it means the search Graph must be self-aware of Store cardinality.
