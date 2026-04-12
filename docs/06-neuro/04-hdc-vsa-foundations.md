# HDC/VSA Foundations

> Hyperdimensional Computing (HDC) and Vector Symbolic Architectures (VSA) provide the mathematical substrate for Neuro's similarity search, knowledge encoding, and cross-domain transfer — using 10,240-bit binary vectors with algebraic operations that run in nanoseconds.


> **Implementation**: Built

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [00-vision-and-grimoire-rename.md](./00-vision-and-grimoire-rename.md) for Neuro context
**Key sources**:
- `bardo-backup/prd/shared/hdc-vsa.md` (BSC algebra, capacity bounds, Rust types)
- `bardo-backup/prd/shared/hdc-fingerprints.md` (three-tier search, transaction encoding)
- `bardo-backup/prd/shared/hdc-applications.md` (memory compression, controlled forgetting)
- `bardo-backup/tmp/agent-chain/04-hdc.md` (HDC math from first principles)
- `refactoring-prd/03-cognitive-subsystems.md` §1 (HDC Encoding section)
- `refactoring-prd/09-innovations.md` §XIII (Cross-Domain Insight Resonance), §XIX.D (False Positive Rate)
- `crates/bardo-primitives/src/hdc.rs` (HdcVector implementation)
- `crates/roko-index/src/hdc.rs` (code symbol fingerprinting)

---

## Abstract

Hyperdimensional Computing (HDC), also called Vector Symbolic Architectures (VSA), represents information as high-dimensional binary vectors and manipulates them with a small set of algebraic operations. The idea rests on a geometric fact: in spaces of dimension D ≥ 8,000, randomly sampled vectors are quasi-orthogonal with overwhelming probability. For D = 10,240 binary vectors (the dimension Roko uses), the probability that two independent vectors share more than 55% of their bits is less than 10⁻⁹. Thousands of distinct concepts can coexist in the same vector space without collision.

HDC is not a replacement for neural network embeddings or traditional database indices. It is a **complementary computational substrate** — a 1,280-byte algebraic representation that unifies knowledge fingerprinting, memory compression, similarity search, cross-domain transfer, and collective consensus under a single set of operations. Neuro uses HDC vectors as optional annotations on `KnowledgeEntry` objects, enabling sub-millisecond similarity queries without any external vector database, GPU, or API calls.

This document covers the mathematical foundations of HDC, the selection of Binary Spatter Codes (BSC) as the specific HDC family, the dimension choice of D = 10,240, the four core algebraic operations, capacity bounds, and the Rust implementation in `roko-primitives` (currently `bardo-primitives`).

---

## What HDC Is

### The Mathematics of Near-Orthogonality

The property that makes HDC work is **concentration of measure**. In a D-dimensional binary space {0,1}^D, the expected Hamming distance between two independently drawn vectors is D/2, with standard deviation √(D)/2. As D grows, the distribution of pairwise distances concentrates tightly around the mean:

```
Expected Hamming distance:     μ = D/2
Standard deviation:            σ = √(D)/2
Coefficient of variation:      CV = σ/μ = 1/√D
```

At D = 10,240:
- CV = 1/√10240 = 0.00988
- 99% of random pairs land within 1% of the expected Hamming distance D/2
- The normalized Hamming similarity (fraction of matching bits) concentrates around 0.5

Random vectors are neither similar nor dissimilar — they are **reliably orthogonal**. This means that when two vectors have similarity significantly above 0.5, there is a genuine structural relationship between them. When similarity is near 0.5, the vectors are unrelated.

Kanerva (2009) formalized these properties and showed that the same geometry underlies neural population codes in the brain. The connection is not metaphorical: place cells in the hippocampus, grid cells in the entorhinal cortex, and sparse codes throughout the cortex all operate in regimes where quasi-orthogonality provides the capacity guarantees that HDC exploits computationally [Kanerva 2009, Cognitive Computation 1(2), 139–159].

### HDC as Computational Substrate

HDC is not a machine learning model. It has no training phase, no gradient computation, no loss function. It is a **representational algebra** — a set of operations on binary vectors that preserve and compose structural relationships. These operations (bind, bundle, permute, similarity) are:

- **Deterministic**: Same input always produces the same output. No randomness in operation.
- **Algebraic**: Operations compose cleanly. Bind distributes over bundle. Permute is a group operation.
- **Efficient**: All operations reduce to bitwise logic (XOR, AND, OR, POPCNT). No floating point.
- **Interpretable**: The result of binding two concepts is a vector that encodes their relationship. The result of bundling multiple vectors is a vector similar to all of them.

For Neuro, HDC provides:
1. **Similarity search**: Find knowledge entries related to a query in ~13ns per comparison
2. **Structured encoding**: Encode typed relationships (cause → effect, role → filler) algebraically
3. **Cross-domain transfer**: Detect structural analogies across domains via vector similarity
4. **Memory compression**: Bundle thousands of entries into a single 1,280-byte summary
5. **Collective consensus**: Privacy-preserving vote aggregation via HDC bundling

---

## Selected System: Binary Spatter Codes (BSC)

### HDC Family Comparison

Four HDC system families dominate the literature. Roko uses BSC exclusively.

| Property | BSC | MAP | HRR | FHRR |
|---|---|---|---|---|
| Vector space | {0,1}^D | {-1,0,+1}^D or Z^D | R^D | C^D (unit circle) |
| Binding | XOR | Multiplication | Circular convolution | Phase addition |
| Binding cost per dim | 1 CPU op | 1 CPU op | O(D) or O(D log D) | 1 complex multiply |
| Unbinding | Exact (XOR is self-inverse) | Exact (ternary mult) | Approximate (correlation) | Approximate (conjugate) |
| Bundle capacity at D=10K | ~1,000 pairs | ~800 pairs | ~100 pairs | ~100 pairs |
| Storage per vector | 1,280 bytes | 10–40 KB | 40 KB (f32) | 80 KB (c64) |
| Best for | Discrete data, hardware speed | Weighted voting, magnitude | Continuous embeddings | Sequence-aware binding |

**Sources**: Kanerva (2009) for BSC; Gayler (1998) for MAP; Plate (2003) for HRR; Yeung, Zou, and Imani (2024) for GHRR.

### Why BSC

Roko chose BSC for five reasons:

1. **Exact invertibility**: XOR is its own inverse — `bind(bind(a, b), b) = a` exactly, with no approximation error. This makes unbinding free and exact, a property no other HDC family matches. For Neuro, this means structured queries can decompose a composite knowledge vector back into its constituent parts without information loss.

2. **Storage efficiency**: 1,280 bytes per vector at D = 10,240. A knowledge base of 100,000 entries requires only ~128 MB of HDC storage. By contrast, HRR at D = 10,000 requires 40 KB per vector (40 GB for 100K entries), and FHRR requires 80 KB per vector.

3. **Computation speed**: XOR compiles to a single instruction per 64-bit word. Hamming distance compiles to XOR followed by POPCNT. On a modern CPU with AVX-512 SIMD, comparing two 10,240-bit vectors takes approximately **13 nanoseconds** (XOR 160 u64 words + popcount). This enables brute-force scanning of 100,000 entries in ~1.3 ms — no approximate nearest neighbor index required for most use cases.

4. **Bundle capacity**: D = 10,240 BSC vectors reliably store up to ~1,000 bound pairs in a bundle with >95% retrieval accuracy [Kleyko et al. 2022, ACM Computing Surveys]. This is 10× the capacity of HRR at the same dimension.

5. **Discrete data fit**: Knowledge entries are inherently discrete — they have typed tags, named concepts, structured relationships. BSC's discrete operations (XOR, majority vote) are a natural fit for encoding discrete structured data. HRR's strength (compatibility with continuous-valued data) is not needed here.

### BSC Capacity

Kleyko et al. (2022) report that D = 10,000 BSC vectors reliably store up to ~1,000 bound pairs in a bundle with >95% retrieval accuracy. At D = 16,384, capacity extends to ~2,000 pairs. Thomas et al. (2021) confirm that capacity grows linearly with D and logarithmically with the acceptable error rate.

---

## Dimension: D = 10,240

The Roko implementation uses **D = 10,240 bits = 160 × u64 words = 1,280 bytes** per vector.

Two reasons for this specific number:

1. **Quasi-orthogonality guarantee**: P(|sim| > 0.05 from expected) < 10⁻⁹ for random pairs. The coefficient of variation is 1/√D = 0.00988, meaning 99% of random pairs land within 1% of expected Hamming distance D/2.

2. **SIMD alignment**: 160 words = 5 × 32-word AVX-512 passes or 10 × 16-word AVX2 passes. Clean loop boundaries with no remainder handling. This maximizes throughput on modern x86 processors.

### Johnson-Lindenstrauss Bound

The Johnson-Lindenstrauss lemma (1984) provides a lower bound on the dimension needed to preserve pairwise distances for N points with distortion ε:

```
D ≥ (8 ln N) / ε²
```

For N = 100,000 knowledge entries and ε = 0.1 (10% maximum distortion):

```
D ≥ (8 × ln(100000)) / 0.01 = (8 × 11.51) / 0.01 = 9,210
```

D = 10,240 exceeds this bound, confirming that the chosen dimension is sufficient to distinguish 100,000+ knowledge entries with <10% distance distortion. For N = 1,000,000 entries (a large collective knowledge base), the required dimension is:

```
D ≥ (8 × ln(1000000)) / 0.01 = (8 × 13.82) / 0.01 = 11,052
```

This is slightly above 10,240, suggesting that for very large knowledge bases (>100K entries), the dimension may need to increase to D = 12,288 or D = 16,384. For the current use case (per-agent knowledge bases of <100K entries), D = 10,240 is sufficient.

**Citation**: Johnson, W. B., & Lindenstrauss, J. (1984). "Extensions of Lipschitz mappings into a Hilbert space." *Contemporary Mathematics*, 26, 189–206.

---

## Signal-to-Noise Ratio and Capacity Bounds

For a bundle of K items in D dimensions, the signal-to-noise ratio is:

```
SNR = √(D / K)
```

At D = 10,240:

| K (items bundled) | SNR | Max codebook N at 99% accuracy |
|---|---|---|
| 5 | 45.3 | >100,000 |
| 10 | 32.0 | >50,000 |
| 50 | 14.3 | ~5,000 |
| 100 | 10.1 | ~1,000 |
| 200 | 7.2 | ~200 |
| 500 | 4.5 | ~20 |

For the primary Neuro use case — encoding 5–10 role-filler pairs per knowledge entry — the capacity is enormous (SNR > 30). Even the memory compression use case — bundling hundreds of entry prototypes into a summary vector — stays within workable SNR ranges.

**Safe rule of thumb**: K < 100 items per bundle for reliable retrieval against codebooks of 1,000+ entries.

---

## Performance Characteristics

### Single Comparison

```
XOR 160 u64 words + POPCNT: ~13 ns (x86 AVX-512)
                              ~25–30 ns (ARM NEON)
```

### Brute-Force Scan

| Entries | Time (AVX-512) | Time (ARM NEON) |
|---|---|---|
| 1,000 | ~13 µs | ~30 µs |
| 10,000 | ~130 µs | ~300 µs |
| 100,000 | ~1.3 ms | ~3 ms |
| 1,000,000 | ~13 ms | ~30 ms |

For per-agent knowledge bases (typically <100K entries), brute-force scan is fast enough that no approximate nearest neighbor (ANN) index is needed. For collective knowledge bases on-chain (potentially millions of entries), a three-tier search strategy is used (see [06-hdc-knowledge-encoding.md](./06-hdc-knowledge-encoding.md)).

**Citation for performance claims**: Kleyko, D., Rachkovskij, D. A., Osipov, E., & Rahimi, A. (2022). "A Survey on Hyperdimensional Computing: Theory, Architecture, and Applications." *ACM Computing Surveys*, 54(6). Performance figures are for XOR + popcount on x86-64 with SIMD; ARM NEON estimates are 2–3× slower based on instruction throughput ratios.

### Storage

| Items | HDC Storage | Equivalent Neural Embeddings (768-dim float32) |
|---|---|---|
| 1,000 | 1.28 MB | 3.07 MB |
| 10,000 | 12.8 MB | 30.7 MB |
| 100,000 | 128 MB | 307 MB |

HDC vectors are ~2.4× more storage-efficient than 768-dimensional float32 embeddings (the typical OpenAI embedding size). At higher neural embedding dimensions (1536-dim), HDC is ~4.8× more efficient.

---

## The Rust Implementation

### HdcVector (`roko-primitives/src/hdc.rs`)

The current implementation in `bardo-primitives` (to be renamed `roko-primitives`) provides the core `HdcVector` struct:

```rust
/// 10,240-bit binary sparse distributed vector.
///
/// Three core operations: XOR bind, majority-vote bundle, Hamming similarity.
/// All operations are CPU-cache-friendly bit manipulation — no floating point,
/// no matrix multiply, no GPU required.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HdcVector {
    bits: [u64; 160],
}
```

Key properties of the implementation:

1. **Copy semantics**: `HdcVector` is `Copy` — stack copies are acceptable at 1,280 bytes for short-lived computations. For persistent collections, heap-allocate (`Box<HdcVector>` or `Vec<HdcVector>`).

2. **Deterministic seeding**: `HdcVector::from_seed(bytes)` produces the same vector for the same input, using FNV-1a hashing followed by splitmix64 expansion. This ensures reproducibility across process restarts — the same concept always maps to the same vector.

3. **Serialization**: Full serde support serializing as 1,280 raw bytes. rkyv feature flag enables zero-copy deserialization from memory-mapped files.

4. **Zero dependencies on external services**: No vector database, no embedding API, no GPU. Everything runs locally on the CPU.

### Convenience Functions

```rust
/// Compute a deterministic HDC fingerprint for any serializable value.
pub fn fingerprint(value: &impl serde::Serialize) -> HdcVector {
    let seed = serde_json::to_vec(value).unwrap_or_default();
    HdcVector::from_seed(&seed)
}

/// Compute a deterministic HDC fingerprint for raw text.
pub fn text_fingerprint(text: &str) -> HdcVector {
    HdcVector::from_seed(text.as_bytes())
}
```

These functions provide simple entry points for converting arbitrary data into HDC vectors. `fingerprint()` works with any serde-serializable struct; `text_fingerprint()` works directly with string content.

---

## Relationship to Neural Network Embeddings

HDC vectors and neural network embeddings (e.g., OpenAI embeddings, sentence-transformers) serve overlapping but distinct purposes:

| Property | HDC (BSC) | Neural Embeddings |
|---|---|---|
| Encoding method | Deterministic algebraic composition | Learned neural network |
| Similarity metric | Hamming distance (bitwise) | Cosine similarity (float) |
| Composability | Full algebra (bind, bundle, permute) | No algebraic structure |
| Determinism | Identical input → identical vector | May vary across model versions |
| External dependency | None (runs locally, no API) | Requires embedding model/API |
| Cost | Zero (pure computation) | Per-token API cost or GPU compute |
| Semantic understanding | Structural/compositional | Semantic/contextual |
| Cross-domain transfer | Natural (structural analogy) | Limited (domain-specific training) |

Neuro uses HDC for **structural similarity** — detecting when two knowledge entries encode similar relationships, regardless of domain. Neural embeddings are better for **semantic similarity** — detecting when two text passages discuss similar topics. The two are complementary: a future enhancement could use neural embeddings for initial retrieval and HDC for structural re-ranking.

---

## Academic Foundations

### Core HDC/VSA References

- Kanerva, P. (2009). "Hyperdimensional Computing: An Introduction to Computing in Distributed Representation with High-Dimensional Random Vectors." *Cognitive Computation*, 1(2), 139–159.
- Kleyko, D., Rachkovskij, D. A., Osipov, E., & Rahimi, A. (2022). "A Survey on Hyperdimensional Computing: Theory, Architecture, and Applications." *ACM Computing Surveys*, 54(6).
- Thomas, A., Dasgupta, S., & Bhatt, T. (2021). "A Theoretical Perspective on Hyperdimensional Computing." *Journal of Artificial Intelligence Research*, 72, 215–249.
- Plate, T. A. (2003). *Holographic Reduced Representations: Distributed Representation for Cognitive Structures*. CSLI Publications. (HRR capacity proofs)
- Gayler, R. W. (1998). "Multiplicative-Additive-Permute representations and the binding problem." *Proceedings of the 20th Cognitive Science Conference*.
- Frady, E. P., Kleyko, D., & Sommer, F. T. (2020). "A Theory of Sequence Indexing and Working Memory in Recurrent Neural Networks." *Neural Computation*, 32(12), 2275–2325. (Resonator networks)

### Dimension and Capacity

- Johnson, W. B., & Lindenstrauss, J. (1984). "Extensions of Lipschitz mappings into a Hilbert space." *Contemporary Mathematics*, 26, 189–206.
- Yeung, E., Zou, T., & Imani, M. (2024). "Generalized Holographic Reduced Representations." (GHRR with non-commutative binding)

### Neuroscience Connection

- Kanerva, P. (1988). *Sparse Distributed Memory*. MIT Press.
- Neubert, P., Schubert, S., & Protzel, P. (2019). "An Introduction to Hyperdimensional Computing for Robotics." *KI - Künstliche Intelligenz*, 33, 319–330. (Place cell analogy)

---

## Implementation Details

### BundleAccumulator with vote tracking

`BundleAccumulator` maintains per-bit integer vote counts for incremental bundling. Because majority vote is not associative over binary vectors, you cannot incrementally bundle by XOR. The accumulator tracks the running tally and collapses to a binary vector on demand.

```rust
use crate::HdcVector;

const HDC_BITS: usize = 10_240;

/// Incremental majority-vote accumulator for HDC bundling.
///
/// Stores per-bit vote counts as i32. Each `add()` contributes +1 (bit set)
/// or -1 (bit unset) per position. `finish()` thresholds at zero to produce
/// the final binary vector.
///
/// Memory: 40 KB (10,240 x 4 bytes). Heap-allocate in hot paths.
pub struct BundleAccumulator {
    /// Per-bit vote tally. Positive = majority 1, negative = majority 0.
    votes: Vec<i32>,
    /// Number of vectors added so far.
    pub count: usize,
}

impl BundleAccumulator {
    /// Create a fresh accumulator with all votes at zero.
    pub fn new() -> Self {
        Self {
            votes: vec![0i32; HDC_BITS],
            count: 0,
        }
    }

    /// Add a vector to the accumulator.
    ///
    /// For each bit position: +1 if the bit is set, -1 if unset.
    /// Cost: O(D) = 10,240 iterations over the bit array.
    pub fn add(&mut self, hv: &HdcVector) {
        self.count += 1;
        for word_idx in 0..160 {
            let word = hv.bits[word_idx];
            for bit in 0..64 {
                let pos = word_idx * 64 + bit;
                if (word >> bit) & 1 == 1 {
                    self.votes[pos] += 1;
                } else {
                    self.votes[pos] -= 1;
                }
            }
        }
    }

    /// Add a vector with integer weight.
    ///
    /// Equivalent to calling `add()` `weight` times, but in a single pass.
    /// Negative weights invert the contribution (subtract instead of add).
    ///
    /// Use cases:
    /// - Weighted consensus (trusted agents get weight > 1)
    /// - Recency weighting (recent entries get higher weight)
    /// - Undo (weight = -1 reverses a previous `add`)
    pub fn add_weighted(&mut self, hv: &HdcVector, weight: i32) {
        self.count += weight.unsigned_abs() as usize;
        for word_idx in 0..160 {
            let word = hv.bits[word_idx];
            for bit in 0..64 {
                let pos = word_idx * 64 + bit;
                if (word >> bit) & 1 == 1 {
                    self.votes[pos] += weight;
                } else {
                    self.votes[pos] -= weight;
                }
            }
        }
    }

    /// Collapse votes to a binary vector via majority threshold.
    ///
    /// Bit i = 1 if votes[i] > 0, else 0. Ties (votes[i] == 0) break to 0
    /// for determinism. Does not consume the accumulator — you can continue
    /// adding vectors after calling `finish()`.
    pub fn finish(&self) -> HdcVector {
        let mut bits = [0u64; 160];
        for word_idx in 0..160 {
            let mut word = 0u64;
            for bit in 0..64 {
                let pos = word_idx * 64 + bit;
                if self.votes[pos] > 0 {
                    word |= 1u64 << bit;
                }
            }
            bits[word_idx] = word;
        }
        HdcVector { bits }
    }

    /// Apply exponential decay to all vote counts.
    ///
    /// Multiplies every vote by `factor` (typically 0.90-0.99) and truncates
    /// toward zero. This implements controlled forgetting: older contributions
    /// lose influence while newer ones retain full weight.
    ///
    /// After decay, the accumulator's `count` is NOT adjusted — it still
    /// reflects the total number of `add()` calls. Use `count` for bookkeeping
    /// only; the votes themselves determine the output.
    ///
    /// # Panics
    /// Panics if `factor` is negative.
    pub fn decay(&mut self, factor: f32) {
        assert!(factor >= 0.0, "decay factor must be non-negative");
        for vote in self.votes.iter_mut() {
            *vote = (*vote as f32 * factor) as i32;
        }
    }
}
```

**Configuration parameters**:

| Parameter | Default | Range | Notes |
|---|---|---|---|
| `HDC_BITS` | 10,240 | 8,192 - 16,384 | Must match `HdcVector` dimension |
| Decay factor | 0.95 | 0.80 - 0.99 | Lower = faster forgetting. 0.95 halves influence after ~14 decays |
| Weight range | -100 to 100 | arbitrary i32 | Extreme weights skew the bundle; keep below count/2 |

**Error handling**: `add()` and `add_weighted()` cannot fail — they operate on fixed-size arrays. `decay()` panics on negative factor (programming error, not runtime condition). `finish()` is pure and infallible.

**Test criteria**:
- `finish()` on an empty accumulator returns `HdcVector::zeros()`
- `add()` followed by `finish()` with a single vector returns that vector exactly
- `add()` three copies of A and two copies of B: `finish()` returns A (majority wins)
- `add_weighted(A, 3)` followed by `add_weighted(B, 2)`: same result as above
- `decay(0.0)` followed by `finish()` returns `HdcVector::zeros()`
- `decay(1.0)` does not change the output of `finish()`

### ItemMemory codebook

`ItemMemory` is a codebook that maps named concepts to their HDC vectors. It provides nearest-neighbor lookup: given an unknown vector, find the closest named concept.

```rust
use crate::HdcVector;
use std::collections::HashMap;

/// A codebook mapping named concepts to HDC vectors.
///
/// Supports insertion, exact lookup by name, and nearest-neighbor search
/// by similarity. For codebooks under ~10K entries, brute-force search
/// runs in <130us and no index is needed.
pub struct ItemMemory {
    /// Maps concept name to its deterministic HDC vector.
    entries: HashMap<String, HdcVector>,
    /// Cached sorted keys for deterministic iteration order.
    sorted_keys: Vec<String>,
    /// Whether sorted_keys needs rebuilding.
    dirty: bool,
}

impl ItemMemory {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            sorted_keys: Vec::new(),
            dirty: false,
        }
    }

    /// Insert a named concept. Generates its vector via `HdcVector::from_seed()`.
    /// Overwrites if the name already exists.
    pub fn insert(&mut self, name: &str, hv: HdcVector) {
        self.entries.insert(name.to_string(), hv);
        self.dirty = true;
    }

    /// Insert a concept with a deterministic seed-based vector.
    pub fn insert_seeded(&mut self, name: &str) {
        let hv = HdcVector::from_seed(name.as_bytes());
        self.insert(name, hv);
    }

    /// Look up a concept by exact name.
    pub fn get(&self, name: &str) -> Option<&HdcVector> {
        self.entries.get(name)
    }

    /// Find the K nearest concepts to the query vector.
    ///
    /// Returns (name, similarity) pairs sorted by descending similarity.
    /// Brute-force scan: O(N) where N = codebook size.
    pub fn top_k(&self, query: &HdcVector, k: usize) -> Vec<(&str, f32)> {
        let mut scored: Vec<(&str, f32)> = self.entries.iter()
            .map(|(name, hv)| (name.as_str(), query.similarity(hv)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        scored
    }

    /// Find the single nearest concept. Returns None if the codebook is empty.
    pub fn nearest(&self, query: &HdcVector) -> Option<(&str, f32)> {
        self.top_k(query, 1).into_iter().next()
    }

    /// Number of concepts in the codebook.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the codebook is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
```

**Codebook generation**: Role codebooks are built at startup by seeding standard role names:

```rust
fn build_role_codebook() -> ItemMemory {
    let mut codebook = ItemMemory::new();
    for role in &[
        "role:domain", "role:topic", "role:type", "role:content", "role:tag",
        "role:risk_factor", "role:response", "role:pattern", "role:severity",
        "role:temporal", "role:confidence",
    ] {
        codebook.insert_seeded(role);
    }
    codebook
}
```

Domain concept codebooks are built incrementally as new concepts appear during knowledge ingestion. The `dirty` flag tracks when `sorted_keys` needs rebuilding for deterministic serialization.

**Test criteria**:
- `insert_seeded("rust")` followed by `get("rust")` returns `Some(hv)` matching `HdcVector::from_seed(b"rust")`
- `nearest()` on a query identical to a codebook entry returns that entry with similarity 1.0
- `top_k(query, 3)` returns exactly 3 entries in descending similarity order
- `nearest()` on an empty codebook returns `None`

### ResonatorNetwork (Frady et al. 2020)

Resonator networks solve the factor decomposition problem: given a composite vector `z = bind(x1, x2, ..., xF)` and a codebook for each factor, recover the original factors through iterated projection.

```rust
use crate::{HdcVector, ItemMemory};

/// Configuration for resonator network convergence.
pub struct ResonatorConfig {
    /// Maximum iterations before giving up. Default: 50.
    pub max_iterations: usize,
    /// Minimum similarity improvement between iterations to continue.
    /// Below this threshold, the network has converged. Default: 0.001.
    pub convergence_threshold: f32,
    /// Early termination: stop if all factor similarities exceed this. Default: 0.9.
    pub early_termination_sim: f32,
}

impl Default for ResonatorConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            convergence_threshold: 0.001,
            early_termination_sim: 0.9,
        }
    }
}

/// Result of a resonator network decomposition.
pub struct ResonatorResult {
    /// Recovered factor names (one per codebook, in order).
    pub factors: Vec<String>,
    /// Similarity of each recovered factor to the best codebook match.
    pub similarities: Vec<f32>,
    /// Number of iterations until convergence.
    pub iterations: usize,
    /// Whether the network converged within max_iterations.
    pub converged: bool,
}

/// Resonator network for factoring composite HDC vectors.
///
/// Given composite z = bind(x1, x2, ..., xF) and codebooks C1, C2, ..., CF,
/// recovers the original factors x1, ..., xF through iterated projection.
///
/// Algorithm (Frady et al. 2020):
///   1. Initialize each factor estimate to a random codebook entry
///   2. For each factor i:
///      a. Compute the "clean-up" signal: bind z with all other current estimates
///      b. Project the clean-up signal onto codebook Ci (nearest neighbor)
///      c. Update estimate_i to the projection result
///   3. Repeat until convergence or max_iterations
pub struct ResonatorNetwork {
    config: ResonatorConfig,
}

impl ResonatorNetwork {
    pub fn new(config: ResonatorConfig) -> Self {
        Self { config }
    }

    /// Decompose a composite vector into its constituent factors.
    ///
    /// `composite`: the vector to decompose (z = bind(x1, ..., xF))
    /// `codebooks`: one ItemMemory per factor, in binding order
    ///
    /// Returns the best-matching entry from each codebook.
    pub fn decompose(
        &self,
        composite: &HdcVector,
        codebooks: &[&ItemMemory],
    ) -> ResonatorResult {
        let f = codebooks.len();
        if f == 0 {
            return ResonatorResult {
                factors: vec![],
                similarities: vec![],
                iterations: 0,
                converged: true,
            };
        }

        // Step 1: Initialize estimates to first codebook entry (or random)
        let mut estimates: Vec<HdcVector> = codebooks.iter()
            .map(|cb| {
                cb.top_k(&HdcVector::from_seed(b"init"), 1)
                    .first()
                    .map(|(name, _)| cb.get(name).copied().unwrap())
                    .unwrap_or_else(HdcVector::zeros)
            })
            .collect();

        let mut prev_sims = vec![0.0f32; f];
        let mut iterations = 0;
        let mut converged = false;

        // Step 2-3: Iterate until convergence
        for iter in 0..self.config.max_iterations {
            iterations = iter + 1;
            let mut all_above_threshold = true;

            for i in 0..f {
                // Bind all OTHER factor estimates together
                let mut other_product = HdcVector::ones();
                for (j, est) in estimates.iter().enumerate() {
                    if j != i {
                        other_product = other_product.bind(est);
                    }
                }

                // Unbind from composite to get clean-up signal for factor i
                let cleanup = composite.bind(&other_product);

                // Project onto codebook i (nearest neighbor)
                if let Some((best_name, best_sim)) = codebooks[i].nearest(&cleanup) {
                    estimates[i] = codebooks[i].get(best_name).copied()
                        .unwrap_or_else(HdcVector::zeros);
                    prev_sims[i] = best_sim;

                    if best_sim < self.config.early_termination_sim {
                        all_above_threshold = false;
                    }
                }
            }

            // Early termination: all factors recovered with high confidence
            if all_above_threshold {
                converged = true;
                break;
            }

            // Convergence check: similarity improvement below threshold
            // (checked after at least 2 iterations)
            if iter > 0 {
                let max_delta: f32 = prev_sims.iter()
                    .zip(prev_sims.iter())
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0, f32::max);
                if max_delta < self.config.convergence_threshold {
                    converged = true;
                    break;
                }
            }
        }

        // Collect final results
        let mut factors = Vec::with_capacity(f);
        let mut similarities = Vec::with_capacity(f);
        for (i, est) in estimates.iter().enumerate() {
            if let Some((name, sim)) = codebooks[i].nearest(est) {
                factors.push(name.to_string());
                similarities.push(sim);
            } else {
                factors.push("<unknown>".to_string());
                similarities.push(0.0);
            }
        }

        ResonatorResult {
            factors,
            similarities,
            iterations,
            converged,
        }
    }
}
```

**Configuration parameters**:

| Parameter | Default | Range | Notes |
|---|---|---|---|
| `max_iterations` | 50 | 10 - 100 | More factors need more iterations. 50 handles up to ~8 factors |
| `convergence_threshold` | 0.001 | 0.0001 - 0.01 | Similarity delta below which iteration stops |
| `early_termination_sim` | 0.9 | 0.8 - 0.95 | All factors above this similarity triggers early exit |

**Error handling**: Returns `converged: false` if the network fails to converge within `max_iterations`. Callers check `converged` and `similarities` to decide whether to trust the result. Empty codebooks produce empty results without error.

**Integration path**: Wire into `NeuroStore::query_structured()` for decomposing retrieved entry vectors into their constituent role-filler pairs.

**Test criteria**:
- Compose `z = bind(hv_rust, hv_async)`, decompose with two codebooks each containing 100 random entries plus the target. Recovered factors match `hv_rust` and `hv_async`
- Convergence within 20 iterations for 2-factor decomposition
- Returns `converged: false` when given a vector not composed from any codebook entries
- Empty codebook list returns empty result with `converged: true`

### DecayingBundleAccumulator with temporal weighting

`DecayingBundleAccumulator` extends `BundleAccumulator` with automatic per-addition decay. Each call to `add()` first decays existing votes, then adds the new vector. This produces a bundle weighted toward recent additions without manual decay calls.

```rust
/// Bundle accumulator with automatic temporal decay.
///
/// Every `add()` first multiplies existing votes by `decay_factor`,
/// then adds the new vector's contribution. The result is a recency-weighted
/// bundle where recent vectors dominate and old vectors fade.
///
/// The effective half-life (in additions) is:
///   half_life = -ln(2) / ln(decay_factor)
///
/// At decay_factor = 0.95: half_life ~ 13.5 additions
/// At decay_factor = 0.99: half_life ~ 69 additions
pub struct DecayingBundleAccumulator {
    votes: Vec<f32>,       // Use f32 for smooth decay (vs i32 in BundleAccumulator)
    pub count: usize,
    decay_factor: f32,
}

impl DecayingBundleAccumulator {
    /// Create a new decaying accumulator.
    ///
    /// # Panics
    /// Panics if `decay_factor` is not in (0.0, 1.0].
    pub fn new(decay_factor: f32) -> Self {
        assert!(
            decay_factor > 0.0 && decay_factor <= 1.0,
            "decay_factor must be in (0.0, 1.0], got {decay_factor}"
        );
        Self {
            votes: vec![0.0f32; 10_240],
            count: 0,
            decay_factor,
        }
    }

    /// Add a vector with automatic decay of prior votes.
    ///
    /// 1. Multiply all existing votes by decay_factor
    /// 2. Add +1.0 for set bits, -1.0 for unset bits
    pub fn add(&mut self, hv: &HdcVector) {
        self.count += 1;
        // Decay existing votes
        for vote in self.votes.iter_mut() {
            *vote *= self.decay_factor;
        }
        // Add new contribution
        for word_idx in 0..160 {
            let word = hv.bits[word_idx];
            for bit in 0..64 {
                let pos = word_idx * 64 + bit;
                if (word >> bit) & 1 == 1 {
                    self.votes[pos] += 1.0;
                } else {
                    self.votes[pos] -= 1.0;
                }
            }
        }
    }

    /// Collapse votes to a binary vector. Threshold at 0.0; ties break to 0.
    pub fn finish(&self) -> HdcVector {
        let mut bits = [0u64; 160];
        for word_idx in 0..160 {
            let mut word = 0u64;
            for bit in 0..64 {
                let pos = word_idx * 64 + bit;
                if self.votes[pos] > 0.0 {
                    word |= 1u64 << bit;
                }
            }
            bits[word_idx] = word;
        }
        HdcVector { bits }
    }

    /// Current decay factor.
    pub fn decay_factor(&self) -> f32 {
        self.decay_factor
    }

    /// Effective half-life in number of additions.
    pub fn half_life(&self) -> f32 {
        -(2.0_f32.ln()) / self.decay_factor.ln()
    }
}
```

**Configuration parameters**:

| Parameter | Default | Range | Effect |
|---|---|---|---|
| `decay_factor` | 0.95 | 0.80 - 0.99 | 0.80 = aggressive forgetting (~3.1 half-life), 0.99 = slow (~69 half-life) |

**Use case**: Episode memory compression. As an agent processes episodes, each episode's vector is added to a `DecayingBundleAccumulator`. The resulting bundle represents "what the agent has been working on recently" with smooth recency weighting. Older episodes fade naturally without explicit pruning.

**Test criteria**:
- Adding the same vector 100 times produces that exact vector from `finish()`
- Adding A then B with decay_factor=0.0001: `finish()` returns approximately B (A fully decayed)
- `half_life()` for decay_factor=0.95 returns approximately 13.5
- Constructor panics on decay_factor=0.0 and decay_factor=1.5

### SIMD intrinsics strategy

The current `HdcVector` implementation uses scalar loops that LLVM auto-vectorizes on x86-64. Explicit SIMD intrinsics provide guaranteed performance across compilers and targets.

**Strategy**: Explicit AVX-512 with fallback chain.

```
Tier 1: AVX-512 (512-bit)  — 160 words / 8 = 20 iterations per operation
Tier 2: AVX2 (256-bit)     — 160 words / 4 = 40 iterations per operation
Tier 3: Scalar (64-bit)    — 160 iterations per operation (current)
```

**Implementation approach**:

```rust
#[cfg(target_arch = "x86_64")]
mod simd {
    use std::arch::x86_64::*;

    /// XOR-bind two vectors using AVX-512.
    /// 20 iterations over 512-bit lanes vs 160 scalar iterations.
    #[target_feature(enable = "avx512f")]
    pub unsafe fn bind_avx512(a: &[u64; 160], b: &[u64; 160], out: &mut [u64; 160]) {
        let a_ptr = a.as_ptr() as *const __m512i;
        let b_ptr = b.as_ptr() as *const __m512i;
        let out_ptr = out.as_mut_ptr() as *mut __m512i;
        for i in 0..20 {
            let va = _mm512_loadu_si512(a_ptr.add(i));
            let vb = _mm512_loadu_si512(b_ptr.add(i));
            let result = _mm512_xor_si512(va, vb);
            _mm512_storeu_si512(out_ptr.add(i), result);
        }
    }

    /// Hamming distance via AVX-512 VPOPCNT (Ice Lake+).
    #[target_feature(enable = "avx512vpopcntdq")]
    pub unsafe fn hamming_avx512(a: &[u64; 160], b: &[u64; 160]) -> u32 {
        let mut total = _mm512_setzero_si512();
        let a_ptr = a.as_ptr() as *const __m512i;
        let b_ptr = b.as_ptr() as *const __m512i;
        for i in 0..20 {
            let va = _mm512_loadu_si512(a_ptr.add(i));
            let vb = _mm512_loadu_si512(b_ptr.add(i));
            let xored = _mm512_xor_si512(va, vb);
            let popcnt = _mm512_popcnt_epi64(xored);
            total = _mm512_add_epi64(total, popcnt);
        }
        // Horizontal sum of 8 x u64 lanes
        let stored: [u64; 8] = std::mem::transmute(total);
        stored.iter().sum::<u64>() as u32
    }
}
```

**Runtime detection**: Use `std::is_x86_feature_detected!()` at init time to select the fastest available path. Cache the decision in a static `AtomicU8` flag (0 = scalar, 1 = AVX2, 2 = AVX-512).

**Why not rely on auto-vectorization alone**: LLVM's auto-vectorizer handles the XOR loop well but struggles with the majority-vote `bundle()` loop (conditional per-bit logic with counters). Explicit SIMD for `bind()` and `similarity()` guarantees performance; `bundle()` stays scalar until profiling shows it is a bottleneck.

**Expected speedups**:

| Operation | Scalar | AVX2 | AVX-512 |
|---|---|---|---|
| `bind()` | ~5 ns | ~2 ns | ~1 ns |
| `similarity()` | ~13 ns | ~6 ns | ~2 ns |
| `bundle(10)` | ~800 ns | ~800 ns* | ~800 ns* |

*Bundle is dominated by per-bit counting logic, not XOR. SIMD helps less here.

### Three-tier search algorithm

The three-tier search strategy (Bloom, approximate, exact) reduces query time over large knowledge bases from O(N) brute force to approximately O(N^0.1) expected work.

```rust
use crate::HdcVector;

/// Three-tier search index for large HDC vector collections.
///
/// Tier 1: Bloom filter with LSH for fast rejection (~95% of entries eliminated)
/// Tier 2: Reduced-precision Hamming on first 2,048 bits (~90% of survivors eliminated)
/// Tier 3: Full 10,240-bit exact comparison on final candidates
pub struct ThreeTierIndex {
    /// LSH-based Bloom filter. Each vector is hashed into `num_hash_functions`
    /// buckets using random hyperplane projections.
    bloom: BloomFilter,
    /// All stored vectors, indexed by insertion order.
    vectors: Vec<HdcVector>,
    /// Configuration.
    config: ThreeTierConfig,
}

pub struct ThreeTierConfig {
    /// Number of LSH hash functions for the Bloom filter. Default: 8.
    pub num_hash_functions: usize,
    /// Bloom filter size in bits. Default: 1,048,576 (128 KB).
    pub bloom_bits: usize,
    /// Number of u64 words to compare in Tier 2. Default: 32 (2,048 bits).
    pub approx_words: usize,
    /// Tier 2 similarity threshold (candidates below this are pruned). Default: 0.51.
    pub approx_threshold: f32,
}

impl Default for ThreeTierConfig {
    fn default() -> Self {
        Self {
            num_hash_functions: 8,
            bloom_bits: 1 << 20,    // 1M bits = 128 KB
            approx_words: 32,       // 2,048 bits = 20% of full vector
            approx_threshold: 0.51,
        }
    }
}

struct BloomFilter {
    bits: Vec<u64>,
    num_hashes: usize,
    /// Random hyperplanes for LSH. Each hyperplane is an HdcVector.
    /// A vector's hash for hyperplane h = popcount(vector AND h) > D/2 ? 1 : 0
    hyperplanes: Vec<HdcVector>,
}
```

**Algorithm pseudocode**:

```
fn top_k(query: &HdcVector, k: usize) -> Vec<(usize, f32)>:
    // Tier 1: Bloom filter
    hash = lsh_hash(query, hyperplanes)
    candidates = bloom_lookup(hash)  // indices of potential matches
    // Expected: ~5-10% of total entries survive

    // Tier 2: Approximate similarity (first 32 words only)
    survivors = []
    for idx in candidates:
        approx_sim = hamming_similarity_partial(query, vectors[idx], approx_words)
        if approx_sim >= approx_threshold:
            survivors.push(idx)
    // Expected: ~0.5-1% of total entries survive

    // Tier 3: Exact top-K on survivors
    scored = []
    for idx in survivors:
        exact_sim = query.similarity(&vectors[idx])
        scored.push((idx, exact_sim))
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1))
    scored.truncate(k)
    scored
```

**Configuration parameters**:

| Parameter | Default | Range | Trade-off |
|---|---|---|---|
| `num_hash_functions` | 8 | 4 - 16 | More = fewer false positives but higher Bloom FP rate |
| `bloom_bits` | 1M | 256K - 16M | Larger = lower FP rate but more memory |
| `approx_words` | 32 | 16 - 80 | More = better pruning but slower Tier 2 |
| `approx_threshold` | 0.51 | 0.50 - 0.53 | Higher = more aggressive pruning, risk of missing true positives |

**Error handling**: The search never errors. Bloom filter false positives are handled by Tier 2/3. If all entries are pruned by Tier 2, the result is an empty vec. The caller should fall back to brute-force if the result set is unexpectedly empty and the query is known to have matches.

**Test criteria**:
- Top-1 on a 10K index with the query vector present returns that vector with similarity 1.0
- Results match brute-force for all queries above threshold 0.526
- Tier 1 eliminates >90% of entries on a 100K random vector set
- Tier 2 eliminates >80% of Tier 1 survivors
- Index with 0 entries returns empty results without panic

---

## Current Status and Gaps

**Implemented**:
- `HdcVector` struct with `[u64; 160]` storage (10,240 bits)
- `bind()` (XOR), `bundle()` (majority vote), `permute()` (cyclic shift), `similarity()` (Hamming)
- `from_seed()` for deterministic vector generation (FNV-1a + splitmix64)
- `to_bytes()` / `from_bytes()` for serialization (1,280 bytes LE)
- `fingerprint()` and `text_fingerprint()` convenience functions
- serde support (serialize as raw bytes)
- rkyv feature flag for zero-copy deserialization
- `HdcFingerprint` for code symbols in `roko-index/src/hdc.rs`
- K-medoids (PAM) clustering over `HdcVector` in `roko-learn/src/hdc_clustering.rs`

**Missing**:
- `BundleAccumulator` (designed above; not in current `bardo-primitives`)
- `ItemMemory` / codebook (designed above; not implemented)
- `ResonatorNetwork` (Frady et al. 2020; designed above; not implemented)
- `DecayingBundleAccumulator` (designed above; vote decay for controlled forgetting)
- AVX-512/AVX2 SIMD intrinsics (strategy above; current implementation uses scalar loops)
- Three-tier search (designed above; Bloom filter, approximate, exact)
- On-chain HDC precompile for Korai

---

## Cross-references

- See [05-hdc-operations.md](./05-hdc-operations.md) for detailed coverage of each HDC operation
- See [06-hdc-knowledge-encoding.md](./06-hdc-knowledge-encoding.md) for how knowledge entries are encoded as HDC vectors
- See [08-cross-domain-hdc-transfer.md](./08-cross-domain-hdc-transfer.md) for structural analogy detection
- See [09-false-positive-math.md](./09-false-positive-math.md) for similarity threshold selection
- See topic [15-code-intelligence](../15-code-intelligence/INDEX.md) for HDC-based code symbol fingerprinting
- See topic [08-chain](../08-chain/INDEX.md) for on-chain HDC precompile design
