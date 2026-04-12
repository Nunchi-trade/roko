# HDC Counterfactual Synthesis

> **Layer**: L1 Framework (HDC primitives) + Cognitive Cross-Cut (dream counterfactuals)
>
> **Synapse Traits**: `Substrate` (HDC vector storage), `Scorer` (Hamming similarity scoring)
>
> **Crate**: `roko-primitives` (`HdcVector`), `roko-learn` (`hdc_clustering`, `pattern_discovery`), `roko-dreams` (counterfactual application)
>
> **Prerequisites**: [03-rem-imagination.md](03-rem-imagination.md)

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

## Cross-References

| Document | Relevance |
|----------|-----------|
| [03-rem-imagination.md](03-rem-imagination.md) | LLM-based counterfactuals that HDC counterfactuals complement |
| [05-dream-evolution.md](05-dream-evolution.md) | EVOLUTION phase uses HDC permutation for knowledge recombination |
| [07-hypnagogia-engine.md](07-hypnagogia-engine.md) | Anti-correlated retrieval powers the Thalamic Gate |
| [02-nrem-replay.md](02-nrem-replay.md) | HDC clustering for compressed batch replay |
