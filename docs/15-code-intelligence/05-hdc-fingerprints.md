# HDC Fingerprints for Structural Similarity

> 10,240-bit hyperdimensional computing vectors that encode code structure into fixed-width binary fingerprints — enabling sub-microsecond similarity search without neural embeddings.


> **Implementation**: Built

**Topic**: [Code Intelligence](./INDEX.md)
**Prerequisites**: [02-symbol-extraction.md](./02-symbol-extraction.md)
**Key sources**: `crates/roko-index/src/hdc.rs`, `bardo-backup/tmp/mori-refactor/10-code-intelligence.md`, `bardo-backup/tmp/death/tools/02-code-index.md`, `bardo-backup/tmp/death/docs/30-index-performance.md`

---

## Abstract

Finding similar code is a core capability for coding agents. When an agent needs to implement a new function, finding existing functions with similar structure accelerates the task. When searching for patterns across a codebase, structural similarity outperforms keyword matching for non-trivial queries.

Neural embeddings (CodeBERT, StarCoder) provide excellent semantic similarity but carry significant costs: GPU inference, model loading latency, and storage overhead for high-dimensional float vectors. For a lightweight code intelligence system that runs on every developer machine, a cheaper alternative is needed.

Hyperdimensional Computing (HDC) provides that alternative. HDC encodes structural properties of code — symbol kind, name, surrounding context — into fixed-width binary vectors using three algebraic operations: bind (XOR), bundle (majority vote), and permute (rotate). The resulting fingerprints support similarity comparison via Hamming distance, which reduces to XOR + popcount — operations that modern CPUs execute in a single instruction cycle.

The `roko-index` crate implements 10,240-bit HDC fingerprints in `hdc.rs`. This document covers the mathematical foundations, the encoding scheme, the implementation details, and the performance characteristics that make HDC practical for real-time code intelligence.

---

## Mathematical Foundations

### Hyperdimensional Computing

HDC (Kanerva 2009) is a computational framework based on the algebraic properties of high-dimensional random vectors. The core insight: in sufficiently high-dimensional spaces (thousands of bits), random vectors are almost certainly near-orthogonal. This means:

1. **Capacity** — A space of D-bit vectors can represent an exponential number of distinct concepts without interference.
2. **Composability** — Vectors can be combined using algebraic operations that preserve the ability to detect component parts.
3. **Robustness** — Small perturbations (noise, missing data) don't destroy the overall structure because similarity is distributed across many bits.

Three operations form the algebra:

| Operation | Symbol | Implementation | Preserves |
|---|---|---|---|
| **Bind** | ⊗ | XOR | Associates two concepts (role-filler binding) |
| **Bundle** | ⊕ | Majority vote | Creates a set-like representation (superposition) |
| **Permute** | ρ | Bit rotation | Creates ordered sequences |

### Why 10,240 bits?

The vector dimensionality D determines the system's capacity and precision:

| D | Capacity (approx.) | Hamming precision | Storage per vector |
|---|---|---|---|
| 1,024 | ~100 items | ±3.1% | 128 bytes |
| 4,096 | ~1,000 items | ±1.6% | 512 bytes |
| **10,240** | **~10,000 items** | **±1.0%** | **1,280 bytes** |
| 65,536 | ~100,000 items | ±0.4% | 8,192 bytes |

The 10,240-bit choice (160 u64 words) balances three concerns:

1. **Sufficient capacity** — ~10,000 distinguishable items supports workspace-scale indexing.
2. **Precision** — ±1.0% Hamming precision distinguishes similar from dissimilar with high confidence.
3. **Performance** — 160 words × 8 bytes = 1,280 bytes per fingerprint. XOR + popcount over 160 words completes in ~50ns on modern hardware.

---

## The Encoding Scheme

### Overview

Each symbol's fingerprint encodes three properties:

```
fingerprint(symbol) = bind(role_vector(kind), bundle(name_vector, context_vector))
```

1. **Role vector** — Deterministic vector derived from the symbol's `SymbolKind`.
2. **Name vector** — Encoded from character trigrams of the symbol name.
3. **Context vector** — Derived from the surrounding source text.

The bind operation (XOR) associates the "what kind of thing is this" (role) with "what does it look and feel like" (name + context). The bundle operation (majority vote) combines name and context into a single composite descriptor.

### Role vectors

Each `SymbolKind` maps to a deterministic base vector:

```rust
fn role_vector(kind: &SymbolKind) -> [u64; WORDS] {
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
    vector_from_seed(seed)
}
```

The `vector_from_seed()` function uses FNV-1a hashing to produce a 64-bit seed, then expands it to 10,240 bits via splitmix64 PRNG:

```rust
fn vector_from_seed(seed: &[u8]) -> [u64; WORDS] {
    let mut state = fnv1a(seed);
    let mut bits = [0u64; WORDS];
    for word in &mut bits {
        *word = splitmix64(&mut state);
    }
    bits
}
```

The deterministic PRNG ensures that the same seed always produces the same vector. Different seeds produce near-orthogonal vectors (by the high-dimensional quasi-orthogonality property).

Role vectors serve as "type tags" in the fingerprint space. Two functions with the same name will have more similar fingerprints than a function and a struct with the same name, because the role vectors for `Function` and `Struct` are near-orthogonal.

### Name encoding via character trigrams

Symbol names are encoded using overlapping character trigrams:

```rust
fn encode_name(name: &str) -> [u64; WORDS] {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() < 3 {
        return vector_from_seed(name.as_bytes());
    }

    let trigrams: Vec<[u64; WORDS]> = chars
        .windows(3)
        .map(|w| {
            let trigram: String = w.iter().collect();
            vector_from_seed(trigram.as_bytes())
        })
        .collect();

    bundle(&trigrams)
}
```

For the name `process_input`:
- Trigrams: `pro`, `roc`, `oce`, `ces`, `ess`, `ss_`, `s_i`, `_in`, `inp`, `npu`, `put`
- Each trigram → deterministic 10,240-bit vector
- All trigram vectors → bundled via majority vote

This encoding has two important properties:

1. **Similar names produce similar vectors** — `process_input` and `process_output` share 7 of 11 trigrams, so their name vectors will be similar (Hamming distance << 50%).

2. **Order sensitivity** — Different orderings of the same characters produce different trigram sets: `abc` and `bca` share only the trigram `bc_` / `_bc`. This captures the structure of names, not just their character bags.

Short names (< 3 characters) fall back to direct seed encoding to avoid empty trigram sets.

### Context encoding

The context vector captures the surrounding source text:

```rust
let ctx_vec = vector_from_seed(context);
```

In the current implementation, the context is the entire file content (passed as a byte slice to `fingerprint_symbol()`). The context vector provides a "which file is this in" signal that helps distinguish identically-named symbols in different files.

Future enhancements could provide more targeted context:
- Function body text only (not the entire file)
- Parameter types and return type
- Doc comment text
- Surrounding symbol names (neighborhood context)

### Composition: bind and bundle

The final fingerprint combines the three components:

```rust
pub fn fingerprint_symbol(symbol: &Symbol, context: &[u8]) -> HdcFingerprint {
    let role_vec = role_vector(&symbol.kind);
    let name_vec = encode_name(&symbol.name);
    let ctx_vec = vector_from_seed(context);
    let combined = bundle(&[name_vec, ctx_vec]);
    HdcFingerprint {
        bits: bind(&role_vec, &combined),
    }
}
```

Step by step:
1. `bundle([name_vec, ctx_vec])` → majority-vote combination of name and context
2. `bind(role_vec, combined)` → XOR association of role with name+context

Why this order? The bundle preserves both name and context information (superposition). The bind tags the result with the symbol kind. This means:
- Two functions with similar names in similar contexts → high similarity
- A function and a struct with the same name → lower similarity (different role vectors)
- Two functions with different names but same context → moderate similarity

### File-level fingerprints

Entire files can also be fingerprinted by bundling all symbol fingerprints:

```rust
pub fn fingerprint_file(source: &SourceFile) -> HdcFingerprint {
    if source.symbols.is_empty() {
        return HdcFingerprint {
            bits: vector_from_seed(source.content.as_bytes()),
        };
    }

    let sym_fps: Vec<[u64; WORDS]> = source.symbols.iter()
        .map(|sym| fingerprint_symbol(sym, source.content.as_bytes()).bits)
        .collect();

    HdcFingerprint { bits: bundle(&sym_fps) }
}
```

File fingerprints enable file-level similarity search: "find files structurally similar to this one." This is useful for:
- Finding test files that correspond to implementation files
- Identifying duplicate or near-duplicate modules
- Clustering related files for workspace organization

---

## Core Operations

### Bundle (majority vote)

```rust
fn bundle(vectors: &[[u64; WORDS]]) -> [u64; WORDS] {
    let threshold = vectors.len() / 2;
    let mut result = [0u64; WORDS];
    for (word_idx, slot) in result.iter_mut().enumerate() {
        let mut word = 0u64;
        for bit_idx in 0..64 {
            let mut ones = 0usize;
            for vec in vectors {
                ones += ((vec[word_idx] >> bit_idx) & 1) as usize;
            }
            if ones > threshold {
                word |= 1u64 << bit_idx;
            }
        }
        *slot = word;
    }
    result
}
```

The majority vote sets each bit to 1 if more than half of the input vectors have that bit set to 1. This creates a "consensus" vector that is similar to all inputs — the HDC equivalent of a centroid.

For an even number of inputs, ties (exactly 50%) are broken toward 0, introducing a slight bias. This is acceptable because the probability of exact ties decreases exponentially with the number of inputs.

### Bind (XOR)

```rust
fn bind(a: &[u64; WORDS], b: &[u64; WORDS]) -> [u64; WORDS] {
    let mut result = [0u64; WORDS];
    for (slot, (left, right)) in
        result.iter_mut().zip(a.iter().zip(b.iter()))
    {
        *slot = left ^ right;
    }
    result
}
```

XOR is the binding operation because:
- It is its own inverse: `bind(bind(a, b), b) = a`
- It preserves the dimensionality (output is D bits, same as inputs)
- It distributes over bundle: `bind(a, bundle(b, c)) ≈ bundle(bind(a, b), bind(a, c))`

### Hamming distance

```rust
fn hamming_distance(a: &[u64; WORDS], b: &[u64; WORDS]) -> u32 {
    let mut diff = 0u32;
    for (left, right) in a.iter().zip(b.iter()) {
        diff += (left ^ right).count_ones();
    }
    diff
}
```

Hamming distance counts the number of differing bits. On modern x86 CPUs, `count_ones()` compiles to the `POPCNT` instruction, making each word comparison a single cycle.

### Similarity (normalized Hamming)

```rust
impl HdcFingerprint {
    pub fn similarity(&self, other: &Self) -> f64 {
        let dist = hamming_distance(&self.bits, &other.bits);
        1.0 - (f64::from(dist) / TOTAL_BITS as f64)
    }
}
```

Similarity is normalized to [0.0, 1.0]:
- 1.0 = identical fingerprints (0 differing bits)
- 0.5 = random (expected for unrelated vectors in high dimensions)
- 0.0 = maximally different (all bits differ — extremely unlikely for random vectors)

---

## Verified Behaviors

The test suite validates the fingerprint system's core properties:

| Test | What it verifies |
|---|---|
| `identical_symbols_identical_fingerprints` | Same symbol + same context → similarity 1.0 |
| `similar_names_high_similarity` | `process_input` vs `process_output` → similarity > 0.5 |
| `different_kinds_lower_similarity` | `Config(Function)` vs `Config(Struct)` → similarity < 0.9 |
| `completely_different_symbols_low_similarity` | `parse_config(Function)` vs `Color(Enum)` → similarity < 0.7 |
| `fingerprint_file_deterministic` | Same file → identical fingerprints (no randomness) |
| `fingerprint_file_empty_symbols` | File with no symbols → content-based fingerprint (no panic) |
| `self_similarity_is_one` | Any fingerprint vs itself → exactly 1.0 |
| `short_name_encoding` | Single-character names → valid fingerprint (no panic) |
| `comparison_performance_under_1ms` | 10,000 comparisons in < 1ms (< 100ns each) |

---

## Performance Characteristics

### Computation costs

| Operation | Time | Notes |
|---|---|---|
| `vector_from_seed()` | ~200ns | 160 splitmix64 iterations |
| `encode_name()` (typical 15-char name) | ~3μs | 13 trigrams → 13 vectors → 1 bundle |
| `fingerprint_symbol()` | ~5μs | role + name + context + bind + bundle |
| `fingerprint_file()` (10 symbols) | ~50μs | 10 symbol fingerprints + 1 bundle |
| `similarity()` | ~50ns | 160 XOR + POPCNT operations |

### Storage costs

| Metric | Value |
|---|---|
| Bytes per fingerprint | 1,280 (160 × 8 bytes) |
| ~5,000 symbol fingerprints | ~6.25 MB |
| ~122,000 symbol fingerprints (large workspace) | ~150 MB |
| With rkyv zero-copy snapshot | Same (binary format, no overhead) |

### Comparison with neural embeddings

| Property | HDC (10,240-bit) | Dense embedding (384-dim float) |
|---|---|---|
| Vector size | 1,280 bytes | 1,536 bytes |
| Computation | ~5μs (CPU only) | ~10ms (GPU) or ~100ms (CPU) |
| Similarity operation | ~50ns (XOR+POPCNT) | ~500ns (dot product) |
| Quality for structural similarity | Good | Excellent |
| Quality for semantic similarity | Limited | Excellent |
| Model dependency | None | Requires fastembed/BGE-small |
| Incremental update cost | ~5μs per symbol | ~10ms per symbol |

HDC excels for structural similarity (same kind, similar names, similar context) and is 200×–20,000× faster than neural embeddings. Neural embeddings excel for semantic similarity ("what does this code do?") but require GPU infrastructure. The planned design uses both: HDC for fast structural matching, embeddings for semantic refinement.

---

## Code Clone Detection: HDC + Neural Hybrid

### The clone taxonomy

Code clones (Roy and Cordy 2007) fall into four types:

| Type | Definition | HDC detects | Neural detects |
|---|---|---|---|
| Type-1 | Exact copies (whitespace/comments differ) | Yes (similarity ~0.95+) | Yes |
| Type-2 | Renamed identifiers/literals | Partially (trigram overlap) | Yes |
| Type-3 | Near-miss (statements added/deleted/modified) | Weakly (context diverges) | Yes |
| Type-4 | Semantic clones (same behavior, different syntax) | No | Yes (embeddings) |

HDC fingerprints reliably detect Type-1 clones and partially detect Type-2 (names share trigrams). For Type-3 and Type-4, neural embeddings are needed.

### Neural code embedding models

The planned embedding layer supports pluggable models:

| Model | Parameters | Embed dim | Context | Strengths |
|---|---|---|---|---|
| CodeBERT (Feng et al. 2020) | 125M | 768 | 512 tok | Bimodal NL+PL; clone detection F1 >96% on Type-1/2/3 |
| UniXcoder (Guo et al. 2022) | 125M | 768 | 512 tok | AST-aware cross-modal; best Type-4 via comment alignment |
| StarCoder2 (Lozhkov et al. 2024) | 3B–15B | varies | 16K tok | Fill-in-the-middle; long context for full-function comparison |
| CodeSage (Zhang et al. 2024) | 130M–1.3B | 1024 | varies | Contrastive learning; outperforms ada-002 by 41% on code-to-code search |
| Jina Code v2 (2024) | 161M | 768 | 8K tok | 30+ languages; text-to-code, code-to-code, code-to-completion modes |

CodeSage's two-stage training is notable: (1) MLM + identifier deobfuscation (predicting original names from minified code), then (2) contrastive learning with hard negatives. CodeSage v2 adds consistency filtering (removing low-quality training pairs) and Matryoshka Representation Learning (truncatable embeddings with minimal loss).

### Hybrid clone detection pipeline

The planned approach uses HDC as a fast first pass and neural embeddings for refinement:

```rust
/// Planned: Hybrid clone detection pipeline
pub struct CloneDetector {
    hdc_index: HdcIndex,
    embedding_model: Option<EmbeddingModel>,
    config: CloneConfig,
}

pub struct CloneConfig {
    /// HDC similarity threshold for candidate generation. Range: 0.55..0.80.
    pub hdc_candidate_threshold: f64,
    /// Neural similarity threshold for confirmed clones. Range: 0.70..0.95.
    pub neural_confirm_threshold: f64,
    /// Maximum candidates from HDC pass. Range: 50..500.
    pub max_candidates: usize,
    /// Minimum clone size (tokens). Range: 20..200.
    pub min_clone_tokens: usize,
}

impl Default for CloneConfig {
    fn default() -> Self {
        Self {
            hdc_candidate_threshold: 0.6,
            neural_confirm_threshold: 0.85,
            max_candidates: 100,
            min_clone_tokens: 30,
        }
    }
}

impl CloneDetector {
    /// Two-phase clone detection: HDC candidate gen → neural confirmation.
    pub fn detect_clones(
        &self,
        query: &SymbolId,
    ) -> Vec<ClonePair> {
        // Phase 1: Fast HDC candidate generation (brute-force or HNSW)
        let candidates = self.hdc_index.query(
            &self.hdc_index.fingerprint(query),
            self.config.max_candidates,
        );

        // Phase 2: Neural refinement (if model available)
        if let Some(model) = &self.embedding_model {
            let query_emb = model.embed(query);
            candidates.into_iter()
                .filter(|(id, _)| {
                    let emb = model.embed(id);
                    cosine_similarity(&query_emb, &emb) >= self.config.neural_confirm_threshold
                })
                .map(|(id, hdc_sim)| ClonePair {
                    source: query.clone(),
                    target: id,
                    clone_type: classify_clone_type(hdc_sim),
                    hdc_similarity: hdc_sim,
                })
                .collect()
        } else {
            // HDC-only mode: lower confidence, no Type-4 detection
            candidates.into_iter()
                .filter(|(_, sim)| *sim >= self.config.hdc_candidate_threshold)
                .map(|(id, sim)| ClonePair {
                    source: query.clone(),
                    target: id,
                    clone_type: classify_clone_type(sim),
                    hdc_similarity: sim,
                })
                .collect()
        }
    }
}

#[derive(Clone, Debug)]
pub struct ClonePair {
    pub source: SymbolId,
    pub target: SymbolId,
    pub clone_type: CloneType,
    pub hdc_similarity: f64,
}

#[derive(Clone, Debug)]
pub enum CloneType {
    Type1,  // HDC sim > 0.95
    Type2,  // HDC sim 0.80..0.95
    Type3,  // HDC sim 0.60..0.80
    Type4,  // HDC sim < 0.60 but neural sim > threshold
}

fn classify_clone_type(hdc_sim: f64) -> CloneType {
    if hdc_sim > 0.95 { CloneType::Type1 }
    else if hdc_sim > 0.80 { CloneType::Type2 }
    else if hdc_sim > 0.60 { CloneType::Type3 }
    else { CloneType::Type4 }
}
```

### PPR re-ranking for clone results

Personalized PageRank (see [04-pagerank-symbol-importance.md](./04-pagerank-symbol-importance.md)) can re-rank clone results by structural proximity: candidates that are both similar AND structurally adjacent to the query (sharing callers, callees, or type dependencies) are ranked higher. This suppresses false positives from textually similar but architecturally unrelated code.

### Test criteria for clone detection

- Type-1 clones (identical symbols) have HDC similarity > 0.95
- Type-2 clones (renamed identifiers) have HDC similarity > 0.80
- Neural refinement rejects candidates with low semantic similarity
- Clone detection without embedding model falls back to HDC-only mode
- Minimum clone size filter excludes trivially small fragments
- PPR re-ranking boosts structurally adjacent clones above distant ones

---

## Planned Enhancements

### HNSW index for fast nearest-neighbor search

For large indices, brute-force comparison (O(N) per query) becomes expensive. HNSW (Hierarchical Navigable Small World) graphs provide approximate nearest-neighbor search in O(log N) time:

```rust
// Planned: HNSW index over HDC fingerprints
pub struct HdcIndex {
    fingerprints: Vec<(SymbolId, HdcFingerprint)>,
    hnsw: HnswGraph,  // From an HNSW library
}

impl HdcIndex {
    pub fn query(&self, query: &HdcFingerprint, k: usize) -> Vec<(SymbolId, f64)> {
        // Return top-k most similar symbols
    }
}
```

Benchmark data from the legacy design (131,654 QPS for search) indicates HNSW can support real-time similarity queries even for large workspaces.

### Reciprocal Rank Fusion (RRF) with other search strategies

HDC similarity is one of five planned search strategies. Reciprocal Rank Fusion combines multiple ranked lists:

```
RRF_score(d) = Σ 1 / (k + rank_i(d))
```

Where `k` is a constant (typically 60) and `rank_i(d)` is the rank of document `d` in strategy `i`. This gives a single combined ranking from keyword, structural, HDC, embedding, and graph-based results.

### Content-aware fingerprinting

Currently, context is the entire file content. Future versions could use more targeted context:

```rust
// Planned: Rich context encoding
pub fn fingerprint_symbol_rich(
    symbol: &Symbol,
    body: &str,           // Function body or type definition
    doc_comment: &str,    // Associated doc comment
    param_types: &[&str], // Parameter types for functions
    return_type: &str,    // Return type for functions
) -> HdcFingerprint {
    let role_vec = role_vector(&symbol.kind);
    let name_vec = encode_name(&symbol.name);
    let body_vec = vector_from_seed(body.as_bytes());
    let doc_vec = vector_from_seed(doc_comment.as_bytes());
    let type_vecs: Vec<_> = param_types.iter()
        .map(|t| vector_from_seed(t.as_bytes()))
        .collect();
    let ret_vec = vector_from_seed(return_type.as_bytes());

    let combined = bundle(&[
        name_vec, body_vec, doc_vec,
        bundle(&type_vecs), ret_vec,
    ]);
    HdcFingerprint { bits: bind(&role_vec, &combined) }
}
```

This richer encoding would capture not just what a symbol is named and where it lives, but what it does (body), what it accepts (parameters), and what it returns.

---

## Academic Foundations

- **Hyperdimensional Computing**: Kanerva (2009), "Hyperdimensional Computing: An Introduction to Computing in Distributed Representation with High-Dimensional Random Vectors." *Cognitive Computation* 1(2):139–159. The foundational paper establishing the mathematical framework for HDC.
- **code2vec**: Alon, Zilberstein, Levy, and Brody (2019), "code2vec: Learning Distributed Representations of Code." *POPL*. Demonstrated that code structure can be captured in fixed-width vectors. HDC fingerprints achieve a similar goal without neural network training.
- **StarCoder**: Li et al. (2023), "StarCoder: May the Source Be with You!" arXiv:2305.06161. State-of-the-art code embedding model. Represents the neural embedding alternative to HDC for semantic similarity.
- **CodeBERT**: Feng, Guo, Tang, et al. (2020), "CodeBERT: A Pre-Trained Model for Programming and Natural Languages." *EMNLP*. Pre-trained model for code understanding, providing dense embeddings that complement HDC's structural fingerprints.
- **Holographic reduced representations**: Plate (2003), *Holographic Reduced Representations*. Oxford University Press. Foundational work on distributed representations using binding and bundling operations — the theoretical predecessor of modern HDC.
- **Cross-domain insight resonance**: Roko design innovation. Uses HDC structural analogy (threshold 0.526) to detect cross-domain structural similarities — e.g., a pattern in the gate pipeline that mirrors a pattern in the knowledge store.

---

## Current Status and Gaps

### Built

- `HdcFingerprint` struct with 10,240-bit vectors (160 u64 words)
- `fingerprint_symbol()` combining role, name trigrams, and context
- `fingerprint_file()` bundling all symbol fingerprints
- `similarity()` via normalized Hamming distance
- Deterministic PRNG (splitmix64) and hash (FNV-1a) for reproducibility
- Core operations: `bind`, `bundle`, `hamming_distance`, `vector_from_seed`, `encode_name`, `role_vector`
- Comprehensive test suite: identity, similarity gradients, performance bounds

### Missing

- HNSW index for fast nearest-neighbor search
- Content-aware fingerprinting (body, doc comments, parameter types)
- Dense embedding integration (fastembed/BGE-small-en-v1.5)
- Reciprocal Rank Fusion combining HDC with other search strategies
- Fingerprint persistence (rkyv snapshots, SQLite storage)
- Fingerprint cache with content-hash invalidation
- Permute operation (bit rotation) for sequence encoding
- Cross-domain similarity analysis

---

## Cross-References

- See [02-symbol-extraction.md](./02-symbol-extraction.md) for the symbols that fingerprints encode
- See [06-context-assembly-from-code.md](./06-context-assembly-from-code.md) for how HDC similarity drives context retrieval
- See [08-index-db-scaling.md](./08-index-db-scaling.md) for persistent fingerprint storage
- See [09-snapshot-optimization.md](./09-snapshot-optimization.md) for rkyv zero-copy fingerprint snapshots
- See topic [00-architecture](../00-architecture/INDEX.md) for the Engram scoring axes that HDC similarity maps to
- See topic [06-neuro](../06-neuro/INDEX.md) for HDC encoding in the knowledge management system
