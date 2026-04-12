# HDC/VSA Foundations

> Hyperdimensional Computing (HDC) and Vector Symbolic Architectures (VSA) provide the mathematical substrate for Neuro's similarity search, knowledge encoding, and cross-domain transfer — using 10,240-bit binary vectors with algebraic operations that run in nanoseconds.

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
- `BundleAccumulator` (designed in spec but not in current `bardo-primitives`; vote tracking for incremental bundling)
- `ItemMemory` / codebook (designed in spec; not implemented)
- `ResonatorNetwork` (Frady et al. 2020; designed in spec; not implemented)
- `DecayingBundleAccumulator` (designed in spec; vote decay for controlled forgetting)
- AVX-512/AVX2 SIMD intrinsics (current implementation uses scalar loops; auto-vectorization may apply)
- Three-tier search (Bloom filter → approximate → exact)
- On-chain HDC precompile for Korai

---

## Cross-references

- See [05-hdc-operations.md](./05-hdc-operations.md) for detailed coverage of each HDC operation
- See [06-hdc-knowledge-encoding.md](./06-hdc-knowledge-encoding.md) for how knowledge entries are encoded as HDC vectors
- See [08-cross-domain-hdc-transfer.md](./08-cross-domain-hdc-transfer.md) for structural analogy detection
- See [09-false-positive-math.md](./09-false-positive-math.md) for similarity threshold selection
- See topic [15-code-intelligence](../15-code-intelligence/INDEX.md) for HDC-based code symbol fingerprinting
- See topic [08-chain](../08-chain/INDEX.md) for on-chain HDC precompile design
