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

## Counterfactual Diversity

### Why Single Counterfactuals Are Insufficient

Generating only a single counterfactual for an episode is inadequate for three structural reasons:

1. **Single recourse path**: One counterfactual gives one direction to move — but the real decision boundary may be irregular, and multiple paths of different cost exist. The agent needs options.
2. **Decision boundary geometry is hidden**: A single counterfactual tells you one point on the decision boundary, not its shape. The agent cannot distinguish between a flat boundary (many paths) and a sharp ridge (one narrow path) from a single example.
3. **Sensitivity to perturbations**: A single counterfactual may lie in a low-density region of knowledge space — plausible in theory but unreachable in practice, or unstable under small perturbations. Diverse counterfactuals make it possible to select the most robust option.

### DiCE: Diverse Counterfactual Explanations

**Reference**: Mothilal, Sharma & Tan (2020, FAT*), "Explaining Machine Learning Classifiers through Diverse Counterfactual Explanations."

DiCE generates a set of k counterfactuals that are jointly optimized for proximity to the original and diversity from each other. The loss function is:

```
L = L_yloss + λ_p · L_proximity - λ_d · det(K)
```

Where:
- `L_yloss` = classification loss (counterfactual must flip the decision)
- `L_proximity` = distance from the original episode (weighted by MAD-normalized L1)
- `det(K)` = determinant of the kernel matrix K — maximized to spread counterfactuals apart
- `K_{i,j} = 1 / (1 + dist(c_i, c_j))` for counterfactuals i, j in the generated set

**Default parameters**: λ_p = 0.5, λ_d = 1.0, k = 3–5 counterfactuals.

The kernel matrix K is a Gram matrix: `det(K)` measures the volume spanned by the counterfactual set in feature space. Maximizing the determinant forces counterfactuals to cover different regions of the feature space, producing a geometrically diverse set.

### DPP Sampling

**Determinantal Point Process (DPP)** sampling provides a principled way to select a diverse subset from a larger candidate pool. For a ground set Y = {c_1, ..., c_N} of candidate counterfactuals and a kernel matrix L (where L_{i,j} encodes similarity between c_i and c_j):

```
P(S ⊆ Y) ∝ det(L_S)  for |S| = k
```

`L_S` is the submatrix of L indexed by S. The probability of selecting a subset S is proportional to the volume it spans in the kernel space. Subsets containing highly similar items (small volume, small determinant) are selected with low probability. Diverse subsets (large volume, large determinant) are selected with high probability.

In the HDC context, the kernel matrix is constructed from HDC Hamming similarities:

```rust
fn build_kernel_matrix(counterfactuals: &[HdcVector]) -> Vec<Vec<f64>> {
    let n = counterfactuals.len();
    let mut k = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..n {
            let sim = counterfactuals[i].similarity(&counterfactuals[j]) as f64;
            k[i][j] = if i == j { 1.0 } else { sim };
        }
    }
    k
}
```

### Wachter et al. Original Formulation

**Reference**: Wachter, Mittelstadt & Russell (2017/2018, Harvard JOLT), "Counterfactual Explanations without Opening the Black Box."

The original single-counterfactual formulation minimizes:

```
L(x, x', y', λ) = λ · (f̂(x') - y')² + d(x, x')
```

Where `d(x, x')` is the MAD-weighted L1 distance:

```
d(x, x') = Σ_j |x_j - x'_j| / MAD_j
```

`MAD_j = median(|x_j - median(x_j)|)` is the Median Absolute Deviation of feature j across the dataset. MAD normalization makes the distance scale-invariant — a change of 1 unit in a high-variance feature is penalized less than the same change in a low-variance feature.

DiCE extends this to k counterfactuals by adding the DPP diversity term. In the HDC setting, `d(x, x')` is replaced by Hamming distance between the original episode vector and the counterfactual vector.

### Coverage Metrics

**Coverage** measures the fraction of users (or episodes) for whom at least one counterfactual in the generated set is actionable. A high-diversity set may still fail to cover certain subgroups if all counterfactuals are clustered in one region of the feature space. Coverage is the primary population-level evaluation metric.

### Evaluation Metrics

| Metric | Definition | Target |
|--------|-----------|--------|
| **Validity** | Fraction of CFs that actually flip the decision | > 0.95 |
| **Proximity** | Mean MAD-weighted distance from original | Minimize |
| **Sparsity** | Mean fraction of features changed | Minimize |
| **Diversity (IM1)** | Mean pairwise distance within the CF set | Maximize |
| **Diversity (IM2)** | Determinant of pairwise distance matrix | Maximize |
| **Plausibility** | Fraction of CFs in high-density regions of training data | > 0.70 |
| **Actionability** | Fraction of CFs satisfying domain constraints | > 0.80 |
| **Coverage** | Fraction of episodes with at least one actionable CF | > 0.60 |

### Rust Structs

```rust
pub struct CounterfactualDiversityConfig {
    /// Number of diverse counterfactuals to generate per episode.
    pub k: usize,                          // default: 5, range: 2-10
    /// Proximity weight (pulls CFs toward original).
    pub proximity_weight: f64,             // default: 0.5, range: 0.1-2.0
    /// Diversity weight (spreads CFs apart via DPP).
    pub diversity_weight: f64,             // default: 1.0, range: 0.1-5.0
    /// Minimum pairwise HDC distance between counterfactuals.
    pub min_pairwise_distance: f32,        // default: 0.15, range: 0.05-0.40
    /// Maximum feature changes per counterfactual (sparsity constraint).
    pub max_features_changed: usize,       // default: 3, range: 1-10
}

pub struct CounterfactualSet {
    pub original_episode: String,
    pub counterfactuals: Vec<CounterfactualHypothesis>,
    pub diversity_score: f64,
    pub coverage_score: f64,
    pub mean_proximity: f64,
    pub mean_sparsity: f64,
}
```

### Algorithm: Diverse CF Generation Using DPP

```
GENERATE-DIVERSE-COUNTERFACTUALS(episode, config):

  Input: episode (HDC vector + feature dict), config: CounterfactualDiversityConfig
  Output: CounterfactualSet

  1. Generate candidate pool (M = k * 10 candidates):
     FOR i = 1..M:
       shift ← random_element([1, 2, 4, 8, 16, 32, 64])
       candidate_vec ← episode.hdc_vector.permute(shift)
       // Optionally blend with a randomly retrieved anti-correlated entry
       IF rand() < 0.3:
         anti_entry ← anti_correlated_retrieval(episode.hdc_vector, 1)[0]
         blend_ratio ← Uniform(0.1, 0.5)
         candidate_vec ← counterfactual_blend(candidate_vec, anti_entry.hdc_vector, blend_ratio)
       candidates.push(candidate_vec)

  2. Filter by validity (decision flip):
     valid_candidates ← [c for c in candidates if flips_decision(episode, c)]

  3. Filter by sparsity (max features changed):
     sparse_candidates ← [c for c in valid_candidates
                           if count_feature_changes(episode, c) <= config.max_features_changed]

  4. If |sparse_candidates| <= k:
     RETURN CounterfactualSet from all sparse_candidates

  5. DPP selection — greedy MAP inference:
     Build kernel matrix K where K[i][j] = similarity(sparse_candidates[i], sparse_candidates[j])
     selected ← []
     WHILE |selected| < k AND sparse_candidates not empty:
       // Greedy: add the candidate that maximizes det(L_{selected ∪ {candidate}})
       best ← argmax over remaining candidates of det_increment(selected, candidate, K)
       selected.push(best)
       // Enforce minimum pairwise distance constraint
       sparse_candidates ← [c for c in sparse_candidates
                             if similarity(c, best) < (1.0 - config.min_pairwise_distance)]

  6. Compute metrics:
     diversity_score ← det(K_selected) ^ (1/k)
     mean_proximity ← mean(hamming_distance(episode.hdc_vector, c) for c in selected)
     mean_sparsity ← mean(count_feature_changes(episode, c) / total_features for c in selected)

  7. RETURN CounterfactualSet {
       original_episode: episode.id,
       counterfactuals: selected,
       diversity_score,
       coverage_score: 0.0,  // computed separately at population level
       mean_proximity,
       mean_sparsity,
     }
```

### Academic Citations

| Paper | Relevance |
|-------|-----------|
| Mothilal, Sharma & Tan (2020, FAT*), "Explaining Machine Learning Classifiers through Diverse Counterfactual Explanations" | DiCE loss function, DPP determinant term, diversity metrics IM1/IM2 |
| Wachter, Mittelstadt & Russell (2017/2018, Harvard JOLT), "Counterfactual Explanations without Opening the Black Box" | Original CF formulation: MAD-weighted L1 + λ proximity-validity tradeoff |
| Kulesza & Taskar (2012, Foundations and Trends in ML), "Determinantal Point Processes for Machine Learning" | DPP theory: P(S) ∝ det(L_S), greedy MAP inference |
| Verma et al. (2020, ICLR workshop), "Counterfactual Explanations for Machine Learning: A Review" | Validity/proximity/sparsity/diversity evaluation taxonomy |

### Test Criteria

1. **DPP diversity**: generated sets of k=5 CFs have mean pairwise Hamming distance > `min_pairwise_distance`.
2. **Validity**: all CFs in the set have similarity below the original episode's decision threshold (they are in a different region of knowledge space).
3. **Sparsity**: no CF changes more than `max_features_changed` features from the original.
4. **Kernel matrix symmetry**: `K[i][j] == K[j][i]` for all i, j; diagonal entries are 1.0.
5. **Greedy selection monotonicity**: adding each successive CF to the selected set increases the determinant of the kernel submatrix.
6. **Coverage**: across 100 episodes, at least 60% have at least one CF within proximity threshold of the original.
7. **Empty candidate pool**: if no valid candidates survive validity + sparsity filtering, return a set with 0 counterfactuals without panic.
8. **Metric ranges**: `diversity_score` ∈ [0, 1]; `mean_proximity` ∈ [0, 1]; `mean_sparsity` ∈ [0, 1].

---

## Counterfactual Plausibility Scoring

A counterfactual that is diverse and proximate may still be useless if it represents an unreachable or incoherent state — a point in HDC space that no real episode would ever occupy. Plausibility scoring filters out these "off-manifold" counterfactuals.

### FACE: Feasible Actionable Counterfactuals via Density-Weighted Paths

**Reference**: Poyiadzi, Sokol, Santos-Rodriguez, De Bie & Flach (2020, AIES), "FACE: Feasible and Actionable Counterfactual Explanations."

FACE replaces the direct L1 proximity measure with a **density-weighted shortest path** cost. The idea: a CF reachable only through low-density regions of knowledge space requires traversing states that are rare or impossible. The path cost penalizes such routes:

```
D_{f,γ} = ∫_γ f(γ(t)) · |γ'(t)| dt
```

Where:
- `γ(t)` is a path from the original episode to the counterfactual in feature space
- `|γ'(t)|` is the speed of traversal (arc length element)
- `f(γ(t)) = 1 / density(γ(t))` — low-density regions cost more to traverse

The integral measures cumulative cost along the path, weighted by the inverse of the local density. A CF reachable through high-density regions (many real examples nearby at each step) has low FACE cost. A CF only reachable through sparse, rarely-observed states has high FACE cost.

**Graph approximation**: FACE approximates the continuous integral with a graph where nodes are training episodes and edges are weighted by `1 / (density_A + density_B)` for adjacent nodes A, B. Shortest paths in this graph approximate the integral. In the HDC context, nodes are NeuroStore entries and edge weights are derived from local KDE estimates.

### Density-Based Plausibility: KDE Scoring

Kernel Density Estimation scores how likely a counterfactual is relative to the distribution of known episodes:

```
p̂(x') = (1 / (N · h^d)) Σ_{i=1}^{N} K((x' - x_i) / h)
```

Where:
- `x_i` are the N training episodes
- `h` is the bandwidth (smoothing parameter)
- `d` is the feature dimension
- `K` is the kernel function (Gaussian in continuous space; approximate via HDC similarity in vector space)

In the HDC setting, the continuous KDE is approximated as:

```rust
fn hdc_kde_score(candidate: &HdcVector, store: &[HdcVector], bandwidth: f32) -> f64 {
    let n = store.len() as f64;
    let sum: f64 = store.iter()
        .map(|x_i| {
            let sim = candidate.similarity(x_i) as f64;
            // Gaussian kernel: K(u) = exp(-u²/2), u = (1 - sim) / bandwidth
            let u = (1.0 - sim) / bandwidth as f64;
            (-u * u / 2.0).exp()
        })
        .sum();
    sum / n
}
```

A high KDE score indicates the counterfactual lies in a densely populated region of knowledge space — similar to many known episodes. A low score indicates an outlier.

### Local Outlier Factor

**LOF** (Breunig et al. 2000, SIGMOD) measures the local density deviation of a point compared to its k-nearest neighbors:

```
LOF_k(x') = (1/k) Σ_{o ∈ N_k(x')} [lrd_k(o) / lrd_k(x')]
```

Where `lrd_k(x') = 1 / (mean reachability distance of x' from its k-nearest neighbors)`.

- `LOF_k(x') ≈ 1.0`: point has similar density to its neighbors (not an outlier)
- `LOF_k(x') >> 1.0`: point is in a lower-density region than its neighbors (outlier)

Counterfactuals with `LOF > 2.0` (default threshold) are rejected as outliers — they lie in regions of knowledge space that are significantly less dense than the surrounding area, indicating they are off-manifold.

### VAE-Based Plausibility

A Variational Autoencoder trained on the episode distribution can detect off-manifold counterfactuals via reconstruction error: if the VAE cannot reconstruct a candidate well, the candidate is not on the learned manifold. In the HDC setting, this is approximated by checking whether the candidate vector can be expressed as a bundle of known episode vectors with reasonable weight magnitudes. High reconstruction error → low plausibility.

### Causal Consistency

**Reference**: Karimi, Barthe, Balle & Valera (2021, FAccT), "Algorithmic Recourse: from Counterfactual Explanations to Interventions."

Karimi et al. distinguish between:
- **Counterfactual explanation**: tells you where to end up (the CF point)
- **Algorithmic recourse**: tells you what actions to take (the intervention sequence)

Causal consistency requires that the counterfactual be reachable via a valid sequence of interventions in the causal graph. Using Pearl's do-calculus, a causally consistent CF must satisfy all structural equations when the intervened variables are set to their CF values. In the HDC setting, causal consistency is checked by verifying that the feature changes implied by the CF do not violate known causal dependencies (encoded in the `CausalGraph` struct).

### Composite Plausibility

The three components are combined into a single composite plausibility score:

```
composite_plausibility =
    path_weight · (1 / (1 + FACE_cost)) +
    density_weight · KDE_score +
    causal_weight · causal_consistency_score
```

Default weights: `path_weight = 0.30`, `density_weight = 0.40`, `causal_weight = 0.30`. These sum to 1.0. Density receives the highest weight because it is the most directly computable and least assumption-dependent measure.

### Rust Structs

```rust
pub struct PlausibilityScorer {
    /// Minimum density score for CF acceptance (KDE-based).
    pub min_density_score: f64,            // default: 0.30, range: 0.10-0.70
    /// LOF threshold: CFs with LOF above this are rejected as outliers.
    pub max_lof: f64,                      // default: 2.0, range: 1.5-5.0
    /// Whether to check causal consistency against the CausalGraph.
    pub check_causal_consistency: bool,    // default: true
    /// HDC manifold proximity threshold.
    pub manifold_proximity_threshold: f32, // default: 0.55, range: 0.40-0.70
    /// Weight for path-based plausibility (FACE-style).
    pub path_weight: f64,                  // default: 0.30
    /// Weight for density-based plausibility.
    pub density_weight: f64,               // default: 0.40
    /// Weight for causal consistency.
    pub causal_weight: f64,                // default: 0.30
}

pub struct PlausibilityReport {
    pub counterfactual_id: String,
    pub density_score: f64,
    pub lof_score: f64,
    pub causal_consistency: bool,
    pub manifold_proximity: f32,
    pub composite_plausibility: f64,
    pub accepted: bool,
}
```

### Academic Citations

| Paper | Relevance |
|-------|-----------|
| Poyiadzi, Sokol, Santos-Rodriguez, De Bie & Flach (2020, AIES), "FACE: Feasible and Actionable Counterfactual Explanations" | Density-weighted shortest path cost; graph approximation |
| Breunig, Kriegel, Ng & Sander (2000, SIGMOD), "LOF: Identifying Density-Based Local Outliers" | LOF outlier detection; local reachability density |
| Karimi, Barthe, Balle & Valera (2021, FAccT), "Algorithmic Recourse: from Counterfactual Explanations to Interventions" | Distinction between CF explanations and actionable recourse; causal consistency via Pearl's framework |
| Pearl (2009), "Causality: Models, Reasoning, and Inference" | Structural causal models; do-calculus for intervention consistency |
| Kingma & Welling (2013), arXiv:1312.6114, "Auto-Encoding Variational Bayes" | VAE reconstruction error as off-manifold indicator |

### Test Criteria

1. **KDE score range**: all returned scores are in [0.0, 1.0].
2. **LOF threshold enforcement**: a point isolated from all neighbors (LOF >> 2.0) is rejected when `max_lof = 2.0`.
3. **KDE high density**: a counterfactual identical to a known episode scores near 1.0 on KDE.
4. **KDE low density**: a counterfactual constructed as the XOR of two random unrelated vectors scores near 0.0 on KDE.
5. **Causal consistency flag**: a CF that inverts a feature that is a known effect (not a cause) is flagged as causally inconsistent when `check_causal_consistency = true`.
6. **Composite weight sum**: `path_weight + density_weight + causal_weight == 1.0` (enforced at config construction).
7. **PlausibilityReport.accepted**: set to `true` iff `composite_plausibility >= min_density_score` AND `lof_score <= max_lof` AND (not `check_causal_consistency` OR `causal_consistency == true`).
8. **Manifold proximity**: a CF vector with similarity < `manifold_proximity_threshold` to all NeuroStore entries is flagged as off-manifold regardless of other scores.
9. **LOF with k=1**: single-entry NeuroStore returns LOF = 1.0 for all candidates (no neighborhood to compare against); no panic.
10. **FACE path weight**: increasing `path_weight` lowers the composite score for CFs that require traversal through low-density regions.

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [03-rem-imagination.md](03-rem-imagination.md) | LLM-based counterfactuals that HDC counterfactuals complement |
| [05-dream-evolution.md](05-dream-evolution.md) | EVOLUTION phase uses HDC permutation for knowledge recombination |
| [07-hypnagogia-engine.md](07-hypnagogia-engine.md) | Anti-correlated retrieval powers the Thalamic Gate |
| [02-nrem-replay.md](02-nrem-replay.md) | HDC clustering for compressed batch replay |
