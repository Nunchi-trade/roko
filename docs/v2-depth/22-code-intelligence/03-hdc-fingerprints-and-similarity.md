# HDC Fingerprints and Similarity

> Depth for HDC fingerprints in code intelligence. Covers the encoding scheme, the Store-native
> similarity index, clone detection, and unification with the system's HDC infrastructure
> (knowledge store, pheromone detection, immune system).

---

## Shared HDC Infrastructure

HDC fingerprints in code intelligence are NOT a standalone system. They use the SAME 10,240-bit
hyperdimensional vectors and algebraic operations used throughout the Roko architecture:

| System | What it fingerprints | Where defined |
|---|---|---|
| **Code intelligence** (this doc) | Source code symbols | `crates/roko-index/src/hdc.rs` |
| **Knowledge store** (neuro) | Knowledge Signals | `crates/roko-neuro/` |
| **Pheromone detection** | Coordination patterns | See [06-MEMORY.md](../../unified/06-MEMORY.md) |
| **Immune system** | Threat patterns | See [16-SECURITY.md](../../unified/16-SECURITY.md) |
| **Episode fingerprinting** | Agent execution traces | `orchestrate.rs` `hdc_fingerprint` field |

All share the same 3-operation algebra from Kanerva (2009):

| Operation | Symbol | Implementation | What it does |
|---|---|---|---|
| **Bind** | XOR | `a[i] ^ b[i]` | Associates two concepts (role-filler binding) |
| **Bundle** | Majority vote | Bit is 1 if > 50% of inputs have it set | Creates set superposition |
| **Permute** | Bit rotation | `rotate_left(n)` | Creates ordered sequences |

This unification means a code symbol fingerprint can be directly compared with a knowledge
Signal fingerprint via Hamming distance. If a code pattern is structurally similar to a known
concept in the knowledge store, the system detects it. This is the basis for
**cross-domain transfer** -- patterns learned in one domain (e.g., gate pipeline structure)
can be recognized when they appear in code (e.g., a similar pipeline pattern in a new crate).

---

## The Encoding Scheme

Each symbol's fingerprint encodes three properties:

```
fingerprint(symbol) = bind(role_vector(kind), bundle(name_vector, context_vector))
```

### Role Vectors

Each `SymbolKind` maps to a deterministic base vector via seed hashing:

```rust
fn role_vector(kind: &SymbolKind) -> [u64; 160] {
    let seed: &[u8] = match kind {
        SymbolKind::Function => b"roko:role:function",
        SymbolKind::Struct   => b"roko:role:struct",
        SymbolKind::Enum     => b"roko:role:enum",
        SymbolKind::Trait    => b"roko:role:trait",
        SymbolKind::Const    => b"roko:role:const",
        SymbolKind::Type     => b"roko:role:type",
        SymbolKind::Module   => b"roko:role:module",
        SymbolKind::Impl     => b"roko:role:impl",
        _                    => b"roko:role:unknown",
    };
    vector_from_seed(seed)  // FNV-1a -> splitmix64 PRNG expansion to 10,240 bits
}
```

Role vectors are near-orthogonal by the high-dimensional quasi-orthogonality property. Two
functions with the same name will have more similar fingerprints than a function and a struct
with the same name.

### Name Encoding via Character Trigrams

Symbol names are encoded using overlapping character trigrams:

```rust
fn encode_name(name: &str) -> [u64; 160] {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() < 3 { return vector_from_seed(name.as_bytes()); }
    let trigrams: Vec<[u64; 160]> = chars.windows(3)
        .map(|w| vector_from_seed(w.iter().collect::<String>().as_bytes()))
        .collect();
    bundle(&trigrams)
}
```

For `process_input`: trigrams `pro`, `roc`, `oce`, `ces`, `ess`, `ss_`, `s_i`, `_in`, `inp`,
`npu`, `put`. Each trigram becomes a 10,240-bit vector, then all are bundled via majority vote.

Properties:
- **Similar names produce similar vectors**: `process_input` and `process_output` share 7 of
  11 trigrams, so name vectors will be similar (Hamming distance << 50%).
- **Order sensitivity**: `abc` and `bca` produce different trigram sets.
- **Short names** (< 3 chars): fall back to direct seed encoding.

### Context Encoding

The context vector captures surrounding source text:

```rust
let ctx_vec = vector_from_seed(context);  // context = file content bytes
```

Currently, context is the entire file content. This provides a "which file is this in" signal.
Future enhancements could use more targeted context: function body, parameter types, doc
comments, neighboring symbol names.

### Composition

```rust
pub fn fingerprint_symbol(symbol: &Symbol, context: &[u8]) -> HdcFingerprint {
    let role_vec = role_vector(&symbol.kind);
    let name_vec = encode_name(&symbol.name);
    let ctx_vec = vector_from_seed(context);
    let combined = bundle(&[name_vec, ctx_vec]);
    HdcFingerprint { bits: bind(&role_vec, &combined) }
}
```

The bundle preserves both name and context (superposition). The bind tags the result with
the symbol kind (role-filler association).

Similarity behavior:
- Two functions with similar names in similar contexts: HIGH similarity
- A function and a struct with the same name: LOWER similarity (different role vectors)
- Two functions with different names but same context: MODERATE similarity

### File-Level Fingerprints

Entire files can be fingerprinted by bundling all symbol fingerprints:

```rust
pub fn fingerprint_file(source: &SourceFile) -> HdcFingerprint {
    let sym_fps: Vec<[u64; 160]> = source.symbols.iter()
        .map(|sym| fingerprint_symbol(sym, source.content.as_bytes()).bits)
        .collect();
    HdcFingerprint { bits: bundle(&sym_fps) }
}
```

File fingerprints enable file-level similarity search: finding test files for implementation
files, identifying near-duplicate modules, clustering related files.

---

## Similarity Search as Store::query_similar

HDC similarity search implements `Store::query_similar()` from the Store protocol (see
[02-CELL.md](../../unified/02-CELL.md)). Given a query HDC vector, find all Signals within
a Hamming distance threshold.

### Hamming Distance

```rust
fn hamming_distance(a: &[u64; 160], b: &[u64; 160]) -> u32 {
    let mut diff = 0u32;
    for (left, right) in a.iter().zip(b.iter()) {
        diff += (left ^ right).count_ones();  // POPCNT instruction
    }
    diff
}
```

Normalized similarity: `1.0 - (hamming_distance / 10_240.0)`. Range [0.0, 1.0]:
- 1.0 = identical fingerprints
- 0.5 = random (expected for unrelated vectors)
- 0.0 = maximally different (extremely unlikely for random vectors)

### Performance

| Operation | Time | Notes |
|---|---|---|
| `vector_from_seed()` | ~200ns | 160 splitmix64 iterations |
| `encode_name()` (15-char name) | ~3us | 13 trigrams, 13 vectors, 1 bundle |
| `fingerprint_symbol()` | ~5us | role + name + context + bind + bundle |
| `similarity()` | ~50ns | 160 XOR + POPCNT operations |
| Brute-force scan 5K symbols | ~0.25ms | 5000 x 50ns |
| Brute-force scan 50K symbols | ~2.5ms | May need HNSW for larger |

### Verified Behaviors (from test suite)

| Test | What it verifies |
|---|---|
| `identical_symbols_identical_fingerprints` | Same symbol + same context -> similarity 1.0 |
| `similar_names_high_similarity` | `process_input` vs `process_output` -> similarity > 0.5 |
| `different_kinds_lower_similarity` | `Config(Function)` vs `Config(Struct)` -> similarity < 0.9 |
| `completely_different_symbols_low_similarity` | Unrelated symbols -> similarity < 0.7 |
| `self_similarity_is_one` | Any fingerprint vs itself -> exactly 1.0 |
| `comparison_performance_under_1ms` | 10,000 comparisons in < 1ms |

### Planned: HNSW Index

For large indices (50K+ symbols), brute-force O(N) becomes expensive. HNSW (Hierarchical
Navigable Small World) graphs provide approximate nearest-neighbor search in O(log N):

```rust
pub struct HdcIndex {
    fingerprints: Vec<(SymbolId, HdcFingerprint)>,
    hnsw: HnswGraph,
}

impl HdcIndex {
    pub fn query(&self, query: &HdcFingerprint, k: usize) -> Vec<(SymbolId, f64)> {
        // Return top-k most similar symbols in O(log N)
    }
}
```

---

## Clone Detection Pipeline as Score Cells

Code clone detection is a **Pipeline of Score Cells** -- each stage classifies similarity
into clone types. See [03-GRAPH.md](../../unified/03-GRAPH.md) for the Pipeline pattern.

### The Clone Taxonomy (Roy and Cordy 2007)

| Type | Definition | HDC detects | Neural detects |
|---|---|---|---|
| Type-1 | Exact copies (whitespace/comments differ) | Yes (sim > 0.95) | Yes |
| Type-2 | Renamed identifiers/literals | Partially (trigram overlap, sim 0.80--0.95) | Yes |
| Type-3 | Near-miss (statements added/deleted) | Weakly (sim 0.60--0.80) | Yes |
| Type-4 | Semantic clones (same behavior, different syntax) | No (sim < 0.60) | Yes |

### Two-Phase Pipeline

```
Phase 1: HDC candidate generation (fast, O(N) or O(log N) with HNSW)
    |
    v
Phase 2: Neural refinement (slow, only on candidates)
    |
    v
Clone pairs with type classification
```

HDC provides the fast first pass. Neural embeddings (feature-gated, via `fastembed`) confirm
candidates and detect Type-4 clones that HDC misses.

```rust
pub struct CloneDetector {
    hdc_index: HdcIndex,
    embedding_model: Option<EmbeddingModel>,  // feature-gated
}

impl CloneDetector {
    pub fn detect_clones(&self, query: &SymbolId) -> Vec<ClonePair> {
        // Phase 1: Fast HDC candidates (sim > 0.6)
        let candidates = self.hdc_index.query(&fingerprint, max_candidates);
        // Phase 2: Neural refinement (if model available)
        // Phase 3: Classify by HDC similarity threshold
    }
}
```

### Composition with Other Systems

Clone detection composes with:

- **Immune system** (see [16-SECURITY.md](../../unified/16-SECURITY.md)): duplicated code
  patterns are a form of "infection" -- the immune system uses HDC fingerprints to detect
  and quarantine repeated antipatterns.
- **Knowledge store** (see [06-MEMORY.md](../../unified/06-MEMORY.md)): cross-domain transfer
  via HDC similarity. When a code pattern matches a known knowledge Signal with sim > 0.526
  (the cross-domain insight resonance threshold), the system detects a structural analogy
  between domains.
- **Learning loops**: clone detection results feed the heuristic calibration loop. Detected
  clones become heuristics: "when modifying function X, also check function Y (clone pair)."

---

## HDC vs Dense Embeddings

| Property | HDC (10,240-bit) | Dense embedding (384-dim float) |
|---|---|---|
| Vector size | 1,280 bytes | 1,536 bytes |
| Computation | ~5us (CPU only) | ~10ms (GPU) or ~100ms (CPU) |
| Similarity op | ~50ns (XOR+POPCNT) | ~500ns (dot product) |
| Structural similarity | Good | Excellent |
| Semantic similarity | Limited | Excellent |
| Model dependency | None | Requires fastembed/BGE-small |
| Incremental update | ~5us per symbol | ~10ms per symbol |

HDC is 200x--20,000x faster than neural embeddings and requires no GPU. Neural embeddings
capture semantic meaning that HDC misses. The planned design uses both: HDC for fast structural
matching (always on), embeddings for semantic refinement (feature-gated behind `embedding`
feature flag).

---

## What This Enables

1. **Sub-microsecond similarity search** -- 50ns per comparison enables real-time "find similar"
   for interactive agents.
2. **Clone detection without external models** -- HDC fingerprints detect Type-1/2 clones with
   zero dependencies, preventing the #1 mistake (duplicate implementations).
3. **Cross-system pattern matching** -- the same HDC vectors used in knowledge store, pheromone
   detection, and immune system enable detecting structural analogies between code and concepts.
4. **Incremental fingerprint updates** -- re-fingerprinting a changed symbol takes ~5us, not
   ~10ms. Fingerprints stay current as the agent modifies code.

## Feedback Loops

- **Clone detection accuracy calibration**: detected clones are verified by agents (true positive
  or false positive). The HDC similarity threshold adjusts: if many false positives at 0.6,
  raise to 0.65. Beta-Binomial calibration per clone type.
- **Cross-domain transfer learning**: when a code pattern's HDC fingerprint matches a knowledge
  Signal and the resulting insight leads to a gate pass, the cross-domain threshold is reinforced.
  Over time, the system learns which structural analogies are productive.
- **Fingerprint quality tracking**: if agents consistently find that HDC similarity correlates
  with actual code similarity (measured by task outcomes), the fingerprint encoding is
  validated. If not, context vector encoding should be made richer (body text, param types).

## Open Questions

1. Should the permute operation (bit rotation) be used for encoding parameter order? Currently
   only bind and bundle are used. Permute would capture "first parameter is X, second is Y"
   ordering, but adds encoding complexity.
2. What is the right HDC dimensionality for code? 10,240 bits handles ~10K distinguishable
   items. Enterprise workspaces with 500K+ symbols may need 65,536 bits (8KB per vector).
   Is the capacity/performance tradeoff worth it?
3. Should file-level fingerprints use weighted bundling (weight by PageRank of each symbol)
   rather than uniform bundling? This would make file fingerprints dominated by important
   symbols rather than giving equal weight to helpers and core types.

## Implementation Tasks

| Task | File paths | Priority |
|---|---|---|
| Add HNSW index for fast nearest-neighbor | `crates/roko-index/src/hdc.rs` | Tier 2 |
| Add content-aware fingerprinting (body, doc, params) | `crates/roko-index/src/hdc.rs` | Tier 2 |
| Add dense embedding integration (feature-gated) | `crates/roko-index/Cargo.toml`, new file | Tier 2 |
| Add RRF combining HDC + embedding results | `crates/roko-index/src/` (new file) | Tier 2 |
| Add fingerprint persistence (SQLite) | `crates/roko-index/src/sqlite.rs` | Tier 1 |
| Add rkyv snapshot for fingerprints (zero-copy) | `crates/roko-index/src/` (new file) | Tier 2 |
| Add clone detection pipeline | `crates/roko-index/src/` (new file) | Tier 2 |
| Add permute operation for sequence encoding | `crates/roko-index/src/hdc.rs` | Tier 3 |
| Wire HDC fingerprints to neuro store for cross-domain | `crates/roko-neuro/`, `crates/roko-index/` | Tier 3 |
