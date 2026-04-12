# HDC Counterfactual Synthesis

> **Layer**: L1 Framework (HDC primitives) + Cognitive Cross-Cut (dream counterfactuals)
>
> **Synapse Traits**: `Substrate` (HDC vector storage), `Scorer` (Hamming similarity scoring)
>
> **Crate**: `roko-primitives` (`HdcVector`), `roko-learn` (`hdc_clustering`, `pattern_discovery`), `roko-dreams` (counterfactual application)
>
> **Prerequisites**: [03-rem-imagination.md](03-rem-imagination.md)


> **Implementation**: Scaffold

---

## What HDC Counterfactual Synthesis Is

Hyperdimensional Computing (HDC), also known as Vector Symbolic Architectures (VSA), provides the mathematical substrate for Roko's dream counterfactual operations. Where LLM-based counterfactuals operate on natural language (slow, expensive, creative), HDC counterfactuals operate on 10,240-bit Binary Spatter Code (BSC) vectors (fast, free, mechanical). The two approaches are complementary:

- **LLM counterfactuals** (REM imagination, see [03-rem-imagination.md](03-rem-imagination.md)): Generate semantically rich hypotheses using natural language reasoning. Expensive. ~$0.01 per counterfactual.
- **HDC counterfactuals**: Explore the neighborhood of existing knowledge vectors using sub-microsecond bit operations. Free. ~1,000 counterfactuals per millisecond.

HDC counterfactual synthesis is used during dreams to rapidly scan the agent's knowledge space for unexplored regions, identify potential connections, and generate candidate directions for the slower LLM-based reasoning to explore.

---

## HDC Fundamentals

The full theoretical basis is documented in [Kanerva 2009, Cognitive Computation 1(2), "Hyperdimensional Computing: An Introduction to Computing in Distributed Representation with High-Dimensional Random Vectors"], [Neubert et al. 2022, Proceedings of the IEEE, "An Introduction to Hyperdimensional Computing for Robotics"], and [Kleyko et al. 2022, ACM Computing Surveys 55(6), "A Survey on Hyperdimensional Computing"].

### Core Properties

| Property | Description | Implication |
|----------|-------------|-------------|
| **Dimensionality** | 10,240 bits per vector | Johnson-Lindenstrauss lemma guarantees orthogonality for up to 100K entries with ε=0.1 |
| **Random orthogonality** | Two random BSC vectors are nearly orthogonal with overwhelming probability | Any two unrelated knowledge entries can be stored in the same space without interference |
| **Similarity metric** | Hamming distance (number of differing bits, normalized to [0, 1]) | O(1) comparison via XOR + popcount — sub-microsecond on modern hardware |
| **Binding** | XOR of two vectors produces a vector orthogonal to both | Represents relationships: `bind(A, B)` encodes "A related to B" |
| **Bundling** | Majority vote across multiple vectors | Produces a vector similar to all inputs — a "prototype" or "centroid" |
| **Permutation** | Cyclic bit-shift | Creates a new vector that is related to the original but distinct — used for sequence encoding and counterfactual exploration |

### Existing Implementation

The Roko codebase already has a complete HDC implementation:

```rust
// crates/roko-primitives/src/hdc.rs (formerly bardo-primitives)
pub struct HdcVector { /* 10,240 bits = 1,280 bytes */ }

impl HdcVector {
    /// Create a deterministic vector from a seed.
    pub fn from_seed(seed: &[u8]) -> Self;

    /// Hamming similarity ∈ [0.0, 1.0]. 1.0 = identical, 0.5 = random.
    pub fn similarity(&self, other: &Self) -> f32;

    /// XOR binding: result is orthogonal to both inputs.
    pub fn bind(&self, other: &Self) -> Self;

    /// Majority-vote bundling: result is similar to all inputs.
    pub fn bundle(vectors: &[&Self]) -> Self;

    /// Cyclic bit-shift permutation.
    pub fn permute(&self, shift: usize) -> Self;
}
```

---

## Counterfactual Operations

### 1. Neighborhood Exploration

To explore the neighborhood of a knowledge entry's meaning, the dream engine applies small permutations and checks what other knowledge entries are nearby:

```rust
fn explore_neighborhood(
    entry: &KnowledgeEntry,
    knowledge_store: &[KnowledgeEntry],
    n_neighbors: usize,
    shift_amounts: &[usize],
) -> Vec<NeighborhoodDiscovery> {
    let mut discoveries = Vec::new();

    for &shift in shift_amounts {
        let shifted = entry.hdc_vector.permute(shift);
        // Find entries closest to the shifted vector
        let mut nearest: Vec<(f32, &KnowledgeEntry)> = knowledge_store
            .iter()
            .filter(|k| k.id != entry.id)
            .map(|k| (shifted.similarity(&k.hdc_vector), k))
            .collect();
        nearest.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

        for (sim, neighbor) in nearest.iter().take(n_neighbors) {
            if *sim > 0.60 {  // configurable threshold
                discoveries.push(NeighborhoodDiscovery {
                    source: entry.id.clone(),
                    neighbor: neighbor.id.clone(),
                    similarity: *sim,
                    shift_amount: shift,
                });
            }
        }
    }

    discoveries
}
```

Typical shift amounts: `[1, 2, 4, 8, 16, 32, 64, 128]`. Each shift explores a different "direction" in the HDC space. Small shifts (1–4) find closely related concepts. Large shifts (64–128) find more distant associations.

### 2. Anti-Correlated Retrieval

The hypnagogia engine (see [07-hypnagogia-engine.md](07-hypnagogia-engine.md)) uses **anti-correlated retrieval** — finding knowledge entries that are maximally *dissimilar* to the current focus. This is achieved by inverting the query vector:

```rust
fn anti_correlated_retrieval(
    focus_vector: &HdcVector,
    knowledge_store: &[KnowledgeEntry],
    n_results: usize,
) -> Vec<&KnowledgeEntry> {
    // Invert the focus vector (XOR with all-ones)
    let anti_focus = focus_vector.bind(&HdcVector::ones());

    // Find entries most similar to the anti-focus
    let mut scored: Vec<(f32, &KnowledgeEntry)> = knowledge_store
        .iter()
        .map(|k| (anti_focus.similarity(&k.hdc_vector), k))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    scored.into_iter().take(n_results).map(|(_, k)| k).collect()
}
```

Anti-correlated retrieval finds the most "opposite" knowledge entries — the entries that share the least structure with the current focus. These maximally dissimilar entries are the raw material for combinational creativity (see [03-rem-imagination.md](03-rem-imagination.md)): combining maximally distant concepts produces the most novel associations.

### 3. Counterfactual Blending

Two knowledge entries can be "blended" by bundling their vectors with different weights to explore the semantic space between them:

```rust
fn counterfactual_blend(
    entry_a: &HdcVector,
    entry_b: &HdcVector,
    blend_ratio: f32,  // 0.0 = pure A, 1.0 = pure B
) -> HdcVector {
    // Create multiple copies weighted by blend ratio
    let a_copies = ((1.0 - blend_ratio) * 10.0) as usize;
    let b_copies = (blend_ratio * 10.0) as usize;

    let mut refs = Vec::new();
    for _ in 0..a_copies { refs.push(entry_a); }
    for _ in 0..b_copies { refs.push(entry_b); }

    HdcVector::bundle(&refs)
}
```

By varying `blend_ratio` from 0.0 to 1.0, the dream engine can sweep across the semantic space between two entries, checking at each point what existing knowledge entries are nearby. This identifies "stepping stones" between concepts — intermediate knowledge entries that connect two seemingly unrelated ideas.

---

## K-Medoids Clustering for Dream Content

The `roko-learn::hdc_clustering` module provides K-medoids clustering over HDC vectors. During dreams, this is used for two purposes:

### Episode Clustering for Compressed Replay

When the episode backlog is large, episodes are clustered by structural similarity. The medoid of each cluster serves as the representative for compressed replay (see [02-nrem-replay.md](02-nrem-replay.md)):

```rust
let result = k_medoids(&episode_vectors, &KMedoidsConfig { k: 6, max_iterations: 50 });
// Replay only the medoid episodes (6 instead of 100+)
```

### Knowledge Space Mapping

Periodically (during EVOLUTION), the agent's entire NeuroStore is clustered to identify knowledge domains:

```rust
let knowledge_vectors: Vec<HdcVector> = knowledge_store
    .iter()
    .map(|e| e.hdc_vector.clone())
    .collect();
let clusters = k_medoids(&knowledge_vectors, &KMedoidsConfig { k: 8, max_iterations: 100 });
```

Each cluster represents a knowledge domain. Clusters with few members represent under-explored areas. Clusters with many high-confidence members represent well-understood domains. The dream engine can direct exploration toward under-represented clusters.

---

## Performance Characteristics

| Operation | Time | Description |
|-----------|------|-------------|
| `HdcVector::from_seed` | ~500 ns | Create a new vector from seed bytes |
| `HdcVector::similarity` | ~200 ns | Hamming distance via XOR + popcount |
| `HdcVector::bind` | ~150 ns | XOR of two 10,240-bit vectors |
| `HdcVector::bundle` (10 vectors) | ~2 µs | Majority vote across 10 vectors |
| `HdcVector::permute` | ~100 ns | Cyclic bit-shift |
| Neighborhood exploration (100 entries, 8 shifts) | ~160 µs | 800 similarity comparisons |
| K-medoids (100 vectors, k=6) | ~5 ms | Full clustering with convergence |
| K-medoids (1000 vectors, k=10) | ~500 ms | Scales quadratically with N |

These times are on modern x86 hardware. The key insight is that HDC operations are **sub-microsecond per comparison**. An agent with 10,000 knowledge entries can search the entire space in ~2 ms. No embedding API, no GPU, no external service.

---

## Why 10,240 Bits?

The dimensionality choice of 10,240 bits is grounded in the Johnson-Lindenstrauss lemma (1984): for N items embedded in a random high-dimensional space, the minimum dimensionality D to preserve pairwise distances with relative error ε is:

```
D ≥ 8 × ln(N) / ε²
```

For N = 100,000 knowledge entries and ε = 0.1 (10% relative error):
```
D ≥ 8 × ln(100,000) / 0.01 = 8 × 11.51 / 0.01 = 9,210
```

10,240 bits provides ~11% headroom above the theoretical minimum. This is a conservative choice that ensures the agent's knowledge space remains well-separated even as the knowledge base grows to 100K entries.

---

## Academic Citations

| Paper | How It Informs HDC Counterfactuals |
|-------|-----------------------------------|
| Kanerva (2009), Cognitive Computation 1(2), "Hyperdimensional Computing" | Core HDC theory: BSC vectors, binding, bundling, permutation |
| Neubert et al. (2022), Proceedings of the IEEE, "Vector Symbolic Architectures" | Comprehensive VSA survey and practical applications |
| Kleyko et al. (2022), ACM Computing Surveys 55(6), "A Survey on HDC" | Theoretical foundations and efficiency analysis |
| Johnson & Lindenstrauss (1984), "Extensions of Lipschitz mappings into a Hilbert space" | Dimensionality requirements for distance preservation |
| Frady et al. (2021), "Resonator networks for factorization and advanced retrieval" | Advanced HDC retrieval techniques |
| Lewis et al. (2020), NeurIPS, arXiv:2005.11401, "Retrieval-Augmented Generation" | RAG for knowledge-intensive NLP — HDC index is a decentralized RAG |

---

## Implementation details

### HdcVector permutation

Permutation performs a cyclic left-shift on the 10,240-bit vector. A shift of `n` moves every bit `n` positions to the left, with overflow wrapping to the right end.

```rust
impl HdcVector {
    /// Cyclic left-shift permutation.
    ///
    /// Shift direction: LEFT (toward MSB). Bits shifted past the MSB
    /// wrap around to the LSB. This matches the convention in Kanerva (2009)
    /// where permutation encodes sequence position.
    ///
    /// The inverse of `permute(n)` is `permute(DIMENSION - n)`.
    pub fn permute(&self, shift: usize) -> Self {
        let effective_shift = shift % Self::DIMENSION; // 10,240
        if effective_shift == 0 {
            return self.clone();
        }

        let mut result = Self::zeros();

        // For each u64 word in the internal representation:
        // shift bits left by effective_shift, OR in the wrapped bits.
        // Implementation operates on the underlying [u64; 160] array.
        let word_shift = effective_shift / 64;
        let bit_shift = effective_shift % 64;

        for i in 0..Self::N_WORDS {
            let src_word = (i + Self::N_WORDS - word_shift) % Self::N_WORDS;
            if bit_shift == 0 {
                result.words[i] = self.words[src_word];
            } else {
                let prev_word = (src_word + Self::N_WORDS - 1) % Self::N_WORDS;
                result.words[i] = (self.words[src_word] << bit_shift)
                    | (self.words[prev_word] >> (64 - bit_shift));
            }
        }

        result
    }

    /// Inverse permutation: undo a previous `permute(shift)`.
    pub fn unpermute(&self, shift: usize) -> Self {
        self.permute(Self::DIMENSION - (shift % Self::DIMENSION))
    }
}
```

Key properties:
- `permute(0)` returns the original vector (identity).
- `permute(n).unpermute(n)` returns the original vector (inverse).
- `permute(n)` produces a vector nearly orthogonal to the original for `n >= 1`. Similarity between a vector and its permuted version is approximately 0.50 (random baseline) for any non-zero shift.
- Permutation is O(N/64) where N is the vector dimension. For 10,240 bits this is 160 word operations — approximately 100 ns.

Wraparound semantics: bits shifted past position 0 (leftmost) reappear at position 10,239 (rightmost). This is a closed group operation — no information is lost, and the operation is perfectly invertible.

### Weighted bundling

The `counterfactual_blend` function uses replication-based weighting. For binary vectors, there is no concept of "multiply by 0.7" — a bit is either 0 or 1. The replication method approximates continuous weights through vote counting:

```rust
fn counterfactual_blend(
    entry_a: &HdcVector,
    entry_b: &HdcVector,
    blend_ratio: f32,  // 0.0 = pure A, 1.0 = pure B
) -> HdcVector {
    // Scale to integer copies. Higher resolution = better precision.
    let total_copies = 10; // configurable, range: 4 - 100
    let b_copies = (blend_ratio * total_copies as f32).round() as usize;
    let a_copies = total_copies - b_copies;

    let mut refs: Vec<&HdcVector> = Vec::with_capacity(total_copies);
    for _ in 0..a_copies { refs.push(entry_a); }
    for _ in 0..b_copies { refs.push(entry_b); }

    HdcVector::bundle(&refs)
}
```

Precision analysis for binary vectors with float `blend_ratio`:

| `total_copies` | Effective resolution | Max blend error | Bundle time (2 vectors) |
|-----------------|---------------------|-----------------|-------------------------|
| 4 | 0.25 steps | 12.5% | ~0.5 us |
| 10 | 0.10 steps | 5.0% | ~1.0 us |
| 20 | 0.05 steps | 2.5% | ~2.0 us |
| 100 | 0.01 steps | 0.5% | ~10.0 us |

The default of 10 copies gives 10% resolution — sufficient for dream exploration where the sweep is coarse. For fine-grained knowledge space mapping (during EVOLUTION), increase to 20-100.

The fundamental precision loss: `bundle` uses majority vote. With an even total and a 50/50 split, ties are broken by a deterministic rule (bit position parity). This introduces a slight bias toward one input, but the bias is uniformly distributed across bit positions and averages out across multiple blend operations.

### Anti-correlated retrieval

`HdcVector::ones()` returns a vector with all 10,240 bits set to 1:

```rust
impl HdcVector {
    /// All-ones vector (1,280 bytes of 0xFF).
    ///
    /// XOR with ones inverts every bit, producing the "opposite" vector.
    /// For any vector V: V.bind(ones()) has similarity ~0.0 to V
    /// (each bit is flipped, so Hamming distance is maximal).
    pub fn ones() -> Self {
        let mut v = Self::zeros();
        for word in v.words.iter_mut() {
            *word = u64::MAX;
        }
        v
    }
}
```

Cost analysis:

| Operation | Cost | Notes |
|-----------|------|-------|
| `HdcVector::ones()` construction | ~20 ns | Fills 160 u64 words with MAX |
| `focus.bind(&ones())` (inversion) | ~150 ns | XOR all 160 words |
| Nearest-neighbor scan (N entries) | N * 200 ns | One similarity comparison per entry |
| **Total anti-correlated retrieval** | ~170 ns + N * 200 ns | For 1,000 entries: ~200 us |

The inversion is exact, not approximate. Every bit is flipped. The resulting anti-focus vector has similarity 0.0 to the original focus vector (maximum Hamming distance = all bits differ). Entries similar to the anti-focus are, by construction, dissimilar to the original focus.

For large knowledge stores (>10,000 entries), a pre-built locality-sensitive hash (LSH) index can reduce the scan from O(N) to O(log N), but the sub-millisecond performance of brute-force scan makes this unnecessary for current scale targets.

### K-medoids clustering

#### Automatic K selection

K is selected automatically using the silhouette method when the caller does not specify a fixed K:

```rust
pub struct KMedoidsConfig {
    /// Fixed K, or None for automatic selection.
    pub k: Option<usize>,
    /// Maximum K to test during automatic selection.
    pub max_k: usize,             // default: 12, range: 2 - 20
    /// Maximum iterations per K-medoids run.
    pub max_iterations: usize,    // default: 50, range: 10 - 200
    /// Convergence tolerance: stop when total cost decreases by less than this.
    pub convergence_epsilon: f64, // default: 1e-4
}

fn auto_select_k(
    vectors: &[HdcVector],
    config: &KMedoidsConfig,
) -> usize {
    let max_k = config.max_k.min(vectors.len() - 1);
    let mut best_k = 2;
    let mut best_silhouette = f64::NEG_INFINITY;

    for k in 2..=max_k {
        let result = k_medoids_inner(vectors, k, config.max_iterations, config.convergence_epsilon);
        let silhouette = mean_silhouette_score(vectors, &result);
        if silhouette > best_silhouette {
            best_silhouette = silhouette;
            best_k = k;
        }
    }

    best_k
}
```

The silhouette score for each point measures how well-separated its cluster is:

```
silhouette(i) = (b(i) - a(i)) / max(a(i), b(i))

where:
  a(i) = mean distance from point i to all other points in the same cluster
  b(i) = mean distance from point i to all points in the nearest other cluster
```

Silhouette ranges from -1.0 (wrong cluster) to +1.0 (well-separated). A mean silhouette above 0.5 indicates good clustering. Below 0.25 suggests the data has no clear cluster structure.

#### Initialization

K-medoids uses BUILD initialization (the PAM algorithm's standard initialization), not k-means++. The difference matters because K-medoids operates on Hamming distance, not Euclidean distance, and BUILD is specifically designed for arbitrary distance metrics:

```
BUILD initialization:
1. Select the point that minimizes total distance to all other points as first medoid
2. For each subsequent medoid:
   a. For each non-medoid point, compute the reduction in total cost if it were added
   b. Select the point that produces the largest cost reduction
```

This is O(N^2 * K) which is acceptable for the episode counts involved (typically <1,000).

#### Distance metric

Hamming distance, computed as `1.0 - similarity`:

```rust
fn hamming_distance(a: &HdcVector, b: &HdcVector) -> f64 {
    1.0 - a.similarity(b) as f64
}
```

This gives a distance in [0.0, 1.0] where 0.0 = identical and 0.5 = random. Values above 0.5 indicate anti-correlation.

#### Convergence

The SWAP phase iterates until one of these conditions is met:

1. No swap reduces total cost (global optimum for the current K)
2. Total cost reduction in the last iteration is below `convergence_epsilon`
3. `max_iterations` is reached

```rust
fn k_medoids_inner(
    vectors: &[HdcVector],
    k: usize,
    max_iterations: usize,
    epsilon: f64,
) -> KMedoidsResult {
    let mut medoids = build_initialize(vectors, k);
    let mut assignments = assign_to_nearest(vectors, &medoids);
    let mut total_cost = compute_total_cost(vectors, &medoids, &assignments);

    for _iter in 0..max_iterations {
        let mut improved = false;

        for m in 0..k {
            for candidate in non_medoids(vectors.len(), &medoids) {
                // Try swapping medoid m with candidate
                let mut trial_medoids = medoids.clone();
                trial_medoids[m] = candidate;
                let trial_assignments = assign_to_nearest(vectors, &trial_medoids);
                let trial_cost = compute_total_cost(vectors, &trial_medoids, &trial_assignments);

                if trial_cost < total_cost - epsilon {
                    medoids = trial_medoids;
                    assignments = trial_assignments;
                    total_cost = trial_cost;
                    improved = true;
                }
            }
        }

        if !improved {
            break; // converged
        }
    }

    KMedoidsResult { medoids, assignments, total_cost }
}
```

### Error handling

| Error condition | Handling |
|-----------------|----------|
| Input vector count < K | Reduce K to `vectors.len() - 1` |
| Single input vector | Return it as the sole cluster; skip clustering |
| Empty input | Return empty result |
| K-medoids does not converge within `max_iterations` | Return best result found so far; log warning with iteration count and final cost |
| All vectors are identical | Returns K=1 clustering regardless of requested K |
| `blend_ratio` outside [0.0, 1.0] | Clamp to valid range |
| `ones()` called on non-standard dimension | Compile-time guarantee via const generic `DIMENSION` |

### Integration wiring

HDC counterfactual operations are called from multiple points in the dream cycle:

```
DreamCycle::run()
  ├─ run_hypnagogia()
  │    └─ thalamic_gate_retrieval()        // anti-correlated retrieval
  │         └─ HdcVector::bind(&ones())    // vector inversion
  │         └─ NeuroStore::nearest_neighbors()
  │
  ├─ run_nrem()
  │    ├─ encode_episodes()                // episode -> HdcVector
  │    ├─ compute_recent_centroid()        // bundling
  │    ├─ k_medoids()                      // compressed batch replay
  │    └─ CrossEpisodeConsolidator::discover()
  │         └─ k_medoids()                 // pattern clustering
  │
  ├─ run_rem()
  │    ├─ combinational_creativity()
  │    │    └─ HdcVector::similarity()     // dissimilarity check for pair selection
  │    ├─ counterfactual_blend()           // weighted bundling for space exploration
  │    │    └─ HdcVector::bundle()
  │    └─ explore_neighborhood()           // permutation-based exploration
  │         └─ HdcVector::permute()
  │
  └─ run_evolution()                       // periodic knowledge recombination
       └─ k_medoids()                      // knowledge domain mapping
       └─ HdcVector::permute()             // knowledge recombination
```

### Test criteria

1. **Permutation invertibility**: for any vector V and shift N, `V.permute(N).unpermute(N) == V`.
2. **Permutation orthogonality**: `V.similarity(V.permute(1))` is within [0.48, 0.52] for random V.
3. **Ones inversion**: `V.bind(HdcVector::ones()).similarity(V)` is within [0.0, 0.02] for random V.
4. **Anti-correlated retrieval**: given a knowledge store with one entry matching the focus and one opposite, anti-correlated retrieval returns the opposite entry.
5. **Blend continuity**: `counterfactual_blend(A, B, 0.0).similarity(A) > 0.95` and `counterfactual_blend(A, B, 1.0).similarity(B) > 0.95`.
6. **Blend interpolation**: for `ratio` in {0.2, 0.4, 0.6, 0.8}, the blended vector's similarity to A decreases monotonically as ratio increases.
7. **K-medoids with planted clusters**: 4 groups of 25 vectors each (generated from 4 distinct seeds with small perturbations) produces K=4 with silhouette > 0.6.
8. **K-medoids convergence**: algorithm terminates in fewer than `max_iterations` for well-separated clusters.
9. **Auto K selection**: given data with 3 clear clusters, `auto_select_k` returns K=3.
10. **Edge cases**: single vector returns single cluster. Empty input returns empty result. K > N reduces K.

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [03-rem-imagination.md](03-rem-imagination.md) | LLM-based counterfactuals that HDC counterfactuals complement |
| [05-dream-evolution.md](05-dream-evolution.md) | EVOLUTION phase uses HDC permutation for knowledge recombination |
| [07-hypnagogia-engine.md](07-hypnagogia-engine.md) | Anti-correlated retrieval powers the Thalamic Gate |
| [02-nrem-replay.md](02-nrem-replay.md) | HDC clustering for compressed batch replay |
