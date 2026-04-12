# HDC Operations: Bind, Bundle, Permute, Similarity

> The four algebraic operations of Binary Spatter Codes — XOR bind, majority-vote bundle, cyclic-shift permute, and Hamming similarity — form a complete algebra for encoding, composing, and querying knowledge in Neuro.


> **Implementation**: Built

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

## Implementation Details: BundleAccumulator Methods

### `add()` — unweighted vector addition

```rust
impl BundleAccumulator {
    /// Add a vector to the accumulator with unit weight.
    ///
    /// For each of the 10,240 bit positions:
    ///   - bit == 1 → votes[pos] += 1
    ///   - bit == 0 → votes[pos] -= 1
    ///
    /// This encoding (bipolar: +1/-1 instead of 1/0) ensures that the zero
    /// crossing of the vote tally corresponds exactly to the majority threshold.
    ///
    /// Time: O(D) = ~10,240 iterations. Memory: no allocation.
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
}
```

The bipolar encoding (+1/-1) is not arbitrary. It centers the vote distribution at zero, which means:
- Positive votes → majority of added vectors had a 1 at this position
- Negative votes → majority had a 0
- Zero → exact tie, resolved to 0 in `finish()` for determinism

This is algebraically equivalent to counting ones and comparing to count/2, but eliminates the division in the threshold step.

### `add_weighted()` — scalar-weighted addition

```rust
impl BundleAccumulator {
    /// Add a vector with integer weight.
    ///
    /// Equivalent to calling `add()` abs(weight) times, but in a single O(D) pass.
    /// Negative weights subtract (undo a previous contribution or down-weight).
    ///
    /// Use cases:
    /// - Recency weighting: recent vectors get weight 3, older get weight 1
    /// - Trust weighting: verified agents get weight 2, unverified get weight 1
    /// - Undo: weight = -1 reverses a prior `add()`
    ///
    /// The `count` field increments by abs(weight), tracking total contribution
    /// magnitude (not the number of `add_weighted` calls).
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
}
```

**Weight bounds**: No hard limit on weight magnitude, but weights above `count / 2` can dominate the bundle. A weight of 100 when only 10 vectors have been added makes that single vector control the output. Keep weights proportional to the expected bundle size.

### `finish()` — collapse to binary vector

```rust
impl BundleAccumulator {
    /// Collapse the accumulated votes into a binary HdcVector.
    ///
    /// For each bit position:
    ///   - votes[pos] > 0  → bit = 1 (majority voted 1)
    ///   - votes[pos] <= 0 → bit = 0 (majority voted 0, ties break to 0)
    ///
    /// This method does NOT consume or reset the accumulator. You can call
    /// `finish()`, inspect the result, then continue adding more vectors
    /// and call `finish()` again.
    ///
    /// Time: O(D). No allocation (writes into stack array).
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
}
```

**Tie-breaking**: Ties (votes == 0) break to 0, not to random. This ensures determinism: the same sequence of `add()` calls always produces the same output from `finish()`. The BSC literature (Kleyko et al. 2022) notes that random tie-breaking would preserve the statistical properties of the bundle, but determinism matters more for reproducibility in Neuro.

### `decay()` — controlled forgetting

```rust
impl BundleAccumulator {
    /// Apply exponential decay to all vote counts.
    ///
    /// Multiplies each vote by `factor` and truncates toward zero.
    /// Typical values: 0.90 (aggressive), 0.95 (moderate), 0.99 (gentle).
    ///
    /// The decay formula for a vote after N decay calls:
    ///   vote_effective = vote_original * factor^N
    ///
    /// Half-life in number of decay calls:
    ///   half_life = -ln(2) / ln(factor)
    ///
    /// | factor | half_life |
    /// |--------|-----------|
    /// | 0.90   | 6.6       |
    /// | 0.95   | 13.5      |
    /// | 0.99   | 69.0      |
    ///
    /// After decay, small votes (|vote| < 1) truncate to 0, creating a natural
    /// noise floor that cleans up stale contributions.
    pub fn decay(&mut self, factor: f32) {
        assert!(factor >= 0.0, "decay factor must be non-negative");
        for vote in self.votes.iter_mut() {
            *vote = (*vote as f32 * factor) as i32;
        }
    }
}
```

**When to call decay**: Call `decay()` periodically between `add()` calls to implement temporal weighting. Two patterns:

1. **Fixed schedule**: Call `decay()` every N additions. The bundle represents a sliding window of approximately `half_life * N` vectors.
2. **Time-based**: Call `decay()` at regular wall-clock intervals (e.g., every hour). Vectors added during busy periods get down-weighted relative to vectors added during quiet periods.

Neuro uses pattern 2 during the Dreams consolidation cycle: decay runs once per Dreams session, so the bundle naturally emphasizes knowledge from recent sessions.

---

## Implementation Details: ResonatorNetwork Factor Decomposition

The full resonator network algorithm (Frady et al. 2020) iteratively recovers the factors of a composite HDC vector. Here is the complete iteration loop with convergence detection.

### Algorithm

```
Input:
  composite: HdcVector  (z = bind(x1, x2, ..., xF))
  codebooks: [ItemMemory; F]  (one per factor)
  config: ResonatorConfig { max_iterations, convergence_threshold, early_termination_sim }

Output:
  factors: [String; F]  (best-matching codebook entry per factor)
  similarities: [f32; F]
  converged: bool

Procedure:
  1. Initialize: estimate[i] = arbitrary entry from codebook[i] for each i
  2. prev_similarities = [0.0; F]
  3. For iteration = 1 to max_iterations:
     a. For each factor i = 0..F:
        i.   other_product = BIND(estimate[0], ..., estimate[i-1], estimate[i+1], ..., estimate[F-1])
        ii.  cleanup_signal = BIND(composite, other_product)  // unbind all other factors
        iii. (best_name, best_sim) = codebook[i].nearest(cleanup_signal)
        iv.  estimate[i] = codebook[i].get(best_name)
        v.   current_similarities[i] = best_sim

     b. Early termination check:
        If ALL current_similarities[i] > early_termination_sim:
          Return (factors, similarities, converged=true)

     c. Convergence check (after iteration >= 2):
        max_delta = max(|current_similarities[i] - prev_similarities[i]|) for all i
        If max_delta < convergence_threshold:
          Return (factors, similarities, converged=true)

     d. prev_similarities = current_similarities

  4. Return (factors, similarities, converged=false)  // did not converge
```

### Convergence properties

Frady et al. (2020) prove that resonator networks minimize reconstruction error monotonically under certain conditions:
- Codebook entries are quasi-orthogonal (guaranteed for BSC at D = 10,240)
- Number of factors F < sqrt(D / log(N_max)) where N_max is the largest codebook size
- For D = 10,240 and N_max = 1000: F < sqrt(10240 / 6.9) ≈ 38 factors

In practice, convergence is fast:

| Factors | Codebook size | Typical iterations | Success rate (D=10,240) |
|---|---|---|---|
| 2 | 100 | 5-10 | >99% |
| 3 | 100 | 10-20 | >98% |
| 5 | 100 | 15-30 | >95% |
| 5 | 1,000 | 20-40 | >90% |
| 8 | 100 | 25-50 | >85% |

### Early termination criteria

Two conditions trigger early exit:

1. **All factors high-confidence**: Every factor's similarity to its best codebook match exceeds `early_termination_sim` (default 0.9). The network has found a strong decomposition and further iteration will not improve it.

2. **Similarity plateau**: The maximum change in any factor's similarity between consecutive iterations falls below `convergence_threshold` (default 0.001). The network has settled into a fixed point.

If neither condition is met within `max_iterations`, the result is returned with `converged: false`. The caller should treat the result with skepticism — the similarities array indicates which factors were recovered reliably and which were not.

### Error handling

- Empty codebook list: returns empty result, `converged: true` (vacuously)
- Codebook with 0 entries for one factor: that factor returns `"<unknown>"` with similarity 0.0
- Non-convergence: `converged: false` with partial results. Caller checks per-factor similarities
- The algorithm never panics or returns `Err`. All failure modes are represented in the `ResonatorResult` struct

### Integration wiring

Wire into `NeuroStore` for structured query decomposition:

```rust
// In roko-neuro/src/store.rs
impl NeuroStore {
    /// Decompose an entry's HDC vector into its constituent role-filler pairs.
    pub fn decompose_entry(
        &self,
        entry_hv: &HdcVector,
        role_codebook: &ItemMemory,
        filler_codebooks: &[&ItemMemory],
    ) -> ResonatorResult {
        let network = ResonatorNetwork::new(ResonatorConfig::default());
        // Each role-filler binding is one factor pair
        // The composite is the bundle of all bindings
        network.decompose(entry_hv, &[role_codebook, filler_codebooks[0]])
    }
}
```

### Test criteria

- 2-factor decomposition: compose `z = bind(A, B)` from known codebook entries, recover A and B with similarity > 0.95
- 5-factor decomposition: compose `z = bind(A, B, C, D, E)`, recover all five with similarity > 0.8
- Convergence speed: 2-factor case converges in < 15 iterations
- Non-convergent case: random vector not composed from codebook entries returns `converged: false`
- Determinism: same inputs produce same outputs across runs

---

## Current Status and Gaps

**Implemented**: `bind()`, `bundle()`, `permute()`, `similarity()`, `from_seed()`, `to_bytes()`/`from_bytes()`, serde, rkyv zero-copy, `fingerprint()`/`text_fingerprint()`.

**Missing**: `BundleAccumulator` (incremental bundling with vote tracking — designed above), `ItemMemory` (concept codebook — designed in [04-hdc-vsa-foundations.md](./04-hdc-vsa-foundations.md)), `ResonatorNetwork` (factor decomposition — designed above), `DecayingBundleAccumulator` (controlled forgetting — designed in [04-hdc-vsa-foundations.md](./04-hdc-vsa-foundations.md)), SIMD intrinsics (strategy in [04-hdc-vsa-foundations.md](./04-hdc-vsa-foundations.md)).

---

## Cross-references

- See [04-hdc-vsa-foundations.md](./04-hdc-vsa-foundations.md) for the mathematical foundations and dimension choice
- See [06-hdc-knowledge-encoding.md](./06-hdc-knowledge-encoding.md) for how these operations encode knowledge entries
- See [08-cross-domain-hdc-transfer.md](./08-cross-domain-hdc-transfer.md) for how structural analogy works
- See [09-false-positive-math.md](./09-false-positive-math.md) for the similarity threshold derivation
