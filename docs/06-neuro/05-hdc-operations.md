# HDC Operations: Bind, Bundle, Permute, Similarity

> The four algebraic operations of Binary Spatter Codes — XOR bind, majority-vote bundle, cyclic-shift permute, and Hamming similarity — form a complete algebra for encoding, composing, and querying knowledge in Neuro.

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [04-hdc-vsa-foundations.md](./04-hdc-vsa-foundations.md) for HDC context and dimension choice
**Key sources**:
- `bardo-backup/prd/shared/hdc-vsa.md` §4 (BSC algebra)
- `bardo-backup/prd/shared/hdc-applications.md` (episode compression, controlled forgetting)
- `crates/bardo-primitives/src/hdc.rs` (Rust implementation)
- `crates/roko-index/src/hdc.rs` (code symbol encoding example)

---

## Abstract

The Binary Spatter Code (BSC) algebra consists of four operations that, together, can encode arbitrarily complex structured data into 10,240-bit vectors. These operations are not arbitrary choices — each has specific algebraic properties that make it suitable for a particular role in knowledge representation:

- **Bind (XOR)** associates two concepts into a new vector orthogonal to both. It encodes typed relationships: "Rust in the language role."
- **Bundle (majority vote)** superimposes multiple vectors into an aggregate similar to all inputs. It encodes set membership: "this entry is about Rust AND async AND performance."
- **Permute (cyclic shift)** encodes position or sequence order, breaking the commutativity of bind. It enables "A then B" to differ from "B then A."
- **Similarity (Hamming distance)** measures the structural relationship between two vectors on a [0, 1] scale.

Every downstream application in Neuro — knowledge encoding, cross-domain transfer, episode compression, structured queries — is a composition of these four primitives. This document covers each operation's algebraic properties, Rust implementation, and role in knowledge representation.

---

## 1. Bind (XOR)

### Definition

Binding associates two hypervectors into a new vector that is quasi-orthogonal to both inputs. It encodes a **relationship** between two concepts:

```
bind(A, B) = A ⊕ B    (componentwise XOR)
```

The result `A ⊕ B` is a new vector that represents the association "A in the context of B" (or equivalently, "B in the context of A"). Critically, the result is quasi-orthogonal to both A and B individually — it is a genuinely new representation, not a blend.

### Algebraic Properties

| Property | Formula | Significance |
|---|---|---|
| **Self-inverse** | bind(bind(A, B), B) = A | No separate unbind needed; XOR is its own inverse |
| **Commutative** | bind(A, B) = bind(B, A) | Role-filler binding is symmetric unless permute is used |
| **Associative** | bind(A, bind(B, C)) = bind(bind(A, B), C) | Multi-way binding can be done in any order |
| **Distributes over bundle** | bind(A, bundle(B, C)) = bundle(bind(A, B), bind(A, C)) | Structured queries work: unbinding a role from a bundled record retrieves the filler |

The distributivity property is what makes structured queries possible. If you have a bundled record:

```
record = bundle(bind(role_language, hv_rust), bind(role_topic, hv_async))
```

You can query "what language?" by unbinding the role:

```
answer = bind(record, role_language)  →  approximately hv_rust
```

The answer is approximately `hv_rust` because bundle introduces some noise, but the similarity is high enough to identify it via nearest-neighbor lookup in the codebook.

### Rust Implementation

```rust
// From bardo-primitives/src/hdc.rs
impl HdcVector {
    /// Binds two vectors using XOR. Involution: `bind(bind(a, b), b) == a`.
    #[must_use]
    pub fn bind(&self, other: &Self) -> Self {
        let mut bits = [0u64; 160];
        for (slot, (left, right)) in bits.iter_mut().zip(self.bits.iter().zip(other.bits.iter())) {
            *slot = left ^ right;
        }
        Self { bits }
    }
}
```

**Performance**: 160 XOR operations on u64 words. With scalar code: ~5 ns. With AVX-512 auto-vectorization: ~2 ns. The operation is embarrassingly parallel — each word is independent.

### Use in Neuro

Binding is used to encode **typed relationships** in knowledge entries:

```
// Encode "borrow_checker is a concept in the Rust domain"
bind(role_domain, hv_rust) ⊕ bind(role_concept, hv_borrow_checker)
```

For CausalLinks, binding with permuted role vectors encodes directionality:

```
// Encode "high_complexity CAUSES more_review"
bind(permute(role_cause, 1), hv_high_complexity) ⊕ bind(permute(role_effect, 2), hv_more_review)
```

---

## 2. Bundle (Majority Vote)

### Definition

Bundling superimposes multiple hypervectors into a single aggregate. The result is **similar to all inputs** — it is a "set union" in hypervector space. For BSC, bundling counts votes per bit position and sets each output bit to the majority value:

```
bundle(A, B, C)[i] = majority(A[i], B[i], C[i])
```

Where `majority(bits)` returns 1 if more than half the input bits are 1, and 0 otherwise. Ties (exactly half 1s and half 0s) break to 0 for determinism.

### Properties

| Property | Details |
|---|---|
| **Similarity preservation** | sim(bundle(A, B), A) ≈ sim(bundle(A, B), B) > 0.5 |
| **Capacity** | SNR = √(D/K) for K bundled items; K < 100 for reliable retrieval at D=10,240 |
| **NOT associative** | bundle(bundle(A, B), C) ≠ bundle(A, bundle(B, C)) because majority vote is not associative over binary vectors |
| **Requires accumulator** | Because bundling is not associative, incremental bundling requires a vote accumulator that tracks per-bit vote counts as integers |

The non-associativity of bundling is important: you cannot incrementally bundle binary vectors by XOR or any binary operation. You must maintain integer vote counts and threshold at the end. This is why the design includes a `BundleAccumulator` type.

### Rust Implementation

```rust
// From bardo-primitives/src/hdc.rs
impl HdcVector {
    /// Bundles a slice of vectors using majority vote (tie → 0).
    #[must_use]
    pub fn bundle(vectors: &[&Self]) -> Self {
        if vectors.is_empty() {
            return Self::zeros();
        }
        let len = vectors.len();
        let mut bits = [0u64; 160];
        for (word_index, slot) in bits.iter_mut().enumerate() {
            let mut word = 0u64;
            for bit_index in 0..64 {
                let mut ones = 0usize;
                for vector in vectors {
                    ones += ((vector.bits[word_index] >> bit_index) & 1) as usize;
                }
                if ones * 2 > len {
                    word |= 1u64 << bit_index;
                }
            }
            *slot = word;
        }
        Self { bits }
    }
}
```

**Performance**: O(D × K) where K is the number of vectors being bundled. For K = 10 vectors: ~800 ns. For K = 100: ~8 µs. The operation is memory-bound — it iterates through each input vector's bits.

### BundleAccumulator (Designed, Not Yet Implemented)

For incremental bundling (adding vectors one at a time), the design specifies a `BundleAccumulator` that maintains per-bit integer vote counts:

```rust
// From design spec (shared/hdc-vsa.md) — not yet in codebase
pub struct BundleAccumulator {
    votes: Vec<i32>,    // HDC_BITS = 10,240 entries
    pub count: usize,
}

impl BundleAccumulator {
    pub fn new() -> Self {
        BundleAccumulator {
            votes: vec![0i32; 10_240],
            count: 0,
        }
    }

    /// Add a vector: +1 for set bits, -1 for unset bits.
    pub fn add(&mut self, hv: &HdcVector) { /* ... */ }

    /// Add with scalar weight (equivalent to adding `weight` times).
    pub fn add_weighted(&mut self, hv: &HdcVector, weight: i32) { /* ... */ }

    /// Collapse votes into a binary vector (majority vote, ties → 0).
    pub fn finish(&self) -> HdcVector { /* ... */ }

    /// Apply exponential decay to all vote counts (controlled forgetting).
    pub fn decay(&mut self, factor: f32) { /* ... */ }
}
```

The accumulator uses 40 KB of memory (10,240 × 4 bytes). It should be heap-allocated, not stack-allocated, in performance-critical paths.

The `decay()` method enables **controlled forgetting**: by multiplying all vote counts by a decay factor (e.g., 0.95), older items lose influence in the bundle while newer items retain full weight. This mirrors the Ebbinghaus decay model at the vector level.

### Use in Neuro

Bundling is used for:

1. **Knowledge entry encoding**: Bundle multiple role-filler bindings into a single entry vector:
   ```
   entry_hv = bundle(
       bind(role_domain, hv_rust),
       bind(role_topic, hv_borrow_checker),
       bind(role_type, hv_insight),
       bind(role_content, hv_content_fingerprint)
   )
   ```

2. **Episode compression**: Bundle prototypes of related episodes into a summary vector that can be queried for any of the constituent episodes.

3. **Memory consolidation**: During Dreams, bundle related knowledge entries to detect clusters and emergent patterns.

4. **Collective consensus**: In the Agent Mesh, bundle multiple agents' independently computed vectors to find areas of agreement (high vote counts) and disagreement (low vote counts).

---

## 3. Permute (Cyclic Shift)

### Definition

Permutation applies a cyclic bit rotation by `k` positions, producing a vector that is quasi-orthogonal to the original:

```
permute(A, k) = cyclic_left_shift(A, k)
```

Each distinct shift count produces a quasi-orthogonal vector: `sim(permute(A, 0), permute(A, 1)) ≈ 0.5`. This property enables encoding **position** or **sequence order**.

### Properties

| Property | Details |
|---|---|
| **Group operation** | permute(permute(A, j), k) = permute(A, j+k) |
| **Invertible** | permute(permute(A, k), D-k) = A |
| **Quasi-orthogonality** | permute(A, k) is quasi-orthogonal to A for k ≥ 1 |
| **Preserves similarity structure** | sim(permute(A, k), permute(B, k)) = sim(A, B) |

### Rust Implementation

```rust
// From bardo-primitives/src/hdc.rs
impl HdcVector {
    /// Rotates bits left by `n` positions (cyclic permutation).
    #[must_use]
    pub fn permute(&self, n: usize) -> Self {
        let bits_len = self.bits.len() * 64;
        let n = n % bits_len;
        if n == 0 {
            return *self;
        }
        let word_shift = n / 64;
        let bit_shift = n % 64;
        let mut bits = [0u64; 160];
        for (index, slot) in bits.iter_mut().enumerate() {
            let src0 = (index + 160 - word_shift) % 160;
            *slot = if bit_shift == 0 {
                self.bits[src0]
            } else {
                let src1 = (src0 + 159) % 160;
                (self.bits[src0] << bit_shift) | (self.bits[src1] >> (64 - bit_shift))
            };
        }
        Self { bits }
    }
}
```

**Performance**: 160 shift+OR operations: ~10 ns. Slightly slower than bind due to the conditional logic for cross-word bit shifting.

### Use in Neuro

Permute is used to encode **directionality** in CausalLinks and **sequence order** in StrategyFragments:

**CausalLink encoding**:
```
// "cause" is at position 1, "effect" is at position 2
causal_hv = bind(permute(hv_cause, 1), permute(hv_effect, 2))
```

This ensures that `CAUSE → EFFECT` has a different vector than `EFFECT → CAUSE`. Without permute, bind's commutativity would make them identical.

**Sequence encoding** (standard scheme):
```
// Encode the sequence [step1, step2, step3] in a StrategyFragment
seq_hv = bundle(
    permute(hv_step1, 0),
    permute(hv_step2, 1),
    permute(hv_step3, 2)
)
```

To query "what is the second step?", unbind position 1:
```
answer = bind(seq_hv, permute(identity, 1))  →  approximately hv_step2
```

---

## 4. Similarity (Hamming Distance)

### Definition

Similarity between two BSC vectors is the fraction of matching bits:

```
sim(A, B) = 1 - hamming_distance(A, B) / D
```

Where `hamming_distance(A, B)` counts the number of differing bit positions.

### Interpretation

| Range | Meaning |
|---|---|
| 1.0 | Identical vectors |
| > 0.526 | Meaningful relationship (< 1% FP against 100K vocabulary, Bonferroni corrected) |
| > 0.52 | Meaningful relationship (single-pair check) |
| 0.48 – 0.52 | Noise band (quasi-orthogonal, no relationship) |
| < 0.48 | Meaningful dissimilarity (anti-correlated) |
| 0.0 | Bitwise complement |

The threshold of **0.526** is recommended for Neuro's cross-domain resonance detection against large knowledge bases. See [09-false-positive-math.md](./09-false-positive-math.md) for the derivation.

### Rust Implementation

```rust
// From bardo-primitives/src/hdc.rs
impl HdcVector {
    /// Returns the Hamming similarity in the range `[0, 1]`.
    pub fn similarity(&self, other: &Self) -> f32 {
        let mut differing_bits = 0u32;
        for (left, right) in self.bits.iter().zip(other.bits.iter()) {
            differing_bits += (left ^ right).count_ones();
        }
        let differing_bits = u16::try_from(differing_bits).unwrap_or(u16::MAX);
        1.0_f32 - (f32::from(differing_bits) / 10_240.0_f32)
    }
}
```

**Performance**: 160 XOR + POPCNT operations: ~13 ns on x86 with SIMD auto-vectorization. This is the critical inner loop for knowledge retrieval — every similarity query executes this path.

**Note**: The `u16::try_from(differing_bits)` conversion is a safety clamp. At D = 10,240, the maximum Hamming distance is 10,240, which fits in a u16 (max 65,535). The `unwrap_or(u16::MAX)` fallback handles potential overflow gracefully.

### Zero-Copy Similarity (rkyv)

With the `rkyv` feature flag, similarity can be computed directly against memory-mapped archived vectors without deserialization:

```rust
#[cfg(feature = "rkyv")]
pub fn similarity_archived(&self, archived: &ArchivedHdcVector) -> f32 {
    let mut differing_bits = 0u32;
    for (left, right) in self.bits.iter().zip(archived.bits.iter()) {
        let right_u64: u64 = (*right).into();
        differing_bits += (left ^ right_u64).count_ones();
    }
    let differing_bits = u16::try_from(differing_bits).unwrap_or(u16::MAX);
    1.0_f32 - (f32::from(differing_bits) / 10_240.0_f32)
}
```

On little-endian platforms, the archived representation of `[u64; 160]` is identical to the in-memory layout, so this reads directly from the mmap'd buffer with no deserialization overhead. This enables scanning large on-disk knowledge bases without loading all vectors into memory.

---

## ResonatorNetwork (Designed, Not Yet Implemented)

Resonator networks (Frady et al. 2020, Neural Computation 32(12)) solve the inverse problem: given a composite hypervector `z = bind(x₁, x₂, ..., xF)` and codebooks for each factor, recover the original factors.

The algorithm works through iterated projection:
1. Initialize each factor estimate (e.g., to the codebook centroid)
2. For each factor i: bind all other current estimates, unbind from the composite, project onto the nearest codebook entry
3. Repeat until convergence (typically 10–50 iterations)

The network's dynamics are energy-minimizing: each step reduces the reconstruction error `‖z − bind(est₁, est₂, ..., estF)‖`. Frady et al. show that resonator networks dramatically outperform alternating least squares and gradient-based methods on the factorization task, especially when multiple factors are present.

For Neuro, resonator networks would enable **decomposing observed patterns back into constituent fields**: "what domain AND topic AND type produced this knowledge entry?" This capability is designed but not yet implemented in the codebase.

---

## Academic Foundations

- Kanerva, P. (2009). "Hyperdimensional Computing." *Cognitive Computation*, 1(2), 139–159. (BSC algebra formalization)
- Kleyko, D., et al. (2022). "A Survey on Hyperdimensional Computing." *ACM Computing Surveys*, 54(6). (Capacity bounds, performance benchmarks)
- Frady, E. P., Kleyko, D., & Sommer, F. T. (2020). "A Theory of Sequence Indexing and Working Memory in Recurrent Neural Networks." *Neural Computation*, 32(12), 2275–2325. (Resonator networks)
- Plate, T. A. (2003). *Holographic Reduced Representations*. CSLI Publications. (Bundle capacity proofs)
- Thomas, A., Dasgupta, S., & Bhatt, T. (2021). "A Theoretical Perspective on Hyperdimensional Computing." *JAIR*, 72. (Capacity scaling)

---

## Current Status and Gaps

**Implemented**: `bind()`, `bundle()`, `permute()`, `similarity()`, `from_seed()`, `to_bytes()`/`from_bytes()`, serde, rkyv zero-copy, `fingerprint()`/`text_fingerprint()`.

**Missing**: `BundleAccumulator` (incremental bundling with vote tracking), `ItemMemory` (concept codebook), `ResonatorNetwork` (factor decomposition), `DecayingBundleAccumulator` (controlled forgetting), SIMD intrinsics (relies on auto-vectorization).

---

## Cross-references

- See [04-hdc-vsa-foundations.md](./04-hdc-vsa-foundations.md) for the mathematical foundations and dimension choice
- See [06-hdc-knowledge-encoding.md](./06-hdc-knowledge-encoding.md) for how these operations encode knowledge entries
- See [08-cross-domain-hdc-transfer.md](./08-cross-domain-hdc-transfer.md) for how structural analogy works
- See [09-false-positive-math.md](./09-false-positive-math.md) for the similarity threshold derivation
