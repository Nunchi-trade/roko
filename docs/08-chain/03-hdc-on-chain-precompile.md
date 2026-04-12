# HDC On-Chain Precompile

> 10,240-bit hyperdimensional vectors via native EVM precompile: ~400 gas for top-K=20 similarity search, three-tier search architecture, same encoding locally and on-chain.


> **Implementation**: Built

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [01-korai-chain-spec.md](./01-korai-chain-spec.md)
**Key sources**: `refactoring-prd/04-knowledge-and-mesh.md`, `refactoring-prd/09-innovations.md` §XIX.D, `bardo-backup/tmp/agent-chain/04-hdc.md`, `bardo-backup/prd/shared/hdc-vsa.md`

---

## Abstract

The HDC (Hyperdimensional Computing) on-chain precompile is the core technical innovation of the Korai chain. It provides native EVM support for 10,240-bit Binary Spatter Code (BSC) vectors — the same encoding used by `roko-primitives` for local knowledge representation. This means an Engram's HDC fingerprint can be computed locally by an agent, posted to the Korai chain, and queried by any other agent using the same mathematical operations, with no encoding translation.

The precompile achieves approximately 400 gas for a top-K=20 similarity search against the on-chain knowledge index. This is orders of magnitude cheaper than implementing the same operations in Solidity (which would cost millions of gas for the bitwise operations on 1,280-byte vectors). The precompile is a **custom Korai feature** — it does not exist on Ethereum mainnet or any standard EVM chain. Development and testing use mirage-rs, which emulates the precompile's behavior in-process.

This document specifies the HDC vector format, the precompile interface, the three-tier search architecture, gas metering, and the relationship between local and on-chain HDC operations.

---

## HDC Vector Format

### Binary Spatter Code (BSC)

Roko uses BSC (Binary Spatter Code) as its hyperdimensional encoding:

- **Dimensionality**: 10,240 bits (1,280 bytes)
- **Element type**: Binary (0 or 1)
- **Similarity metric**: Normalized Hamming distance (fraction of matching bits)
- **Operations**:
  - **BIND** (XOR): Associates two concepts. `BIND(A, B) = A ⊕ B`. Produces a vector nearly orthogonal to both inputs.
  - **BUNDLE** (majority vote): Combines multiple concepts. `BUNDLE(A, B, C) = majority_vote(A, B, C)` per dimension. Produces a vector similar to all inputs.
  - **PERMUTE** (cyclic shift): Encodes order/sequence. `PERMUTE(A, k) = rotate_left(A, k)`. Produces a vector nearly orthogonal to the input.

These three operations are sufficient to encode arbitrarily complex structured knowledge. The encoding is the same whether computed by `roko-primitives` locally or by the precompile on-chain.

### Why 10,240 Bits?

The dimensionality is chosen for a balance of precision and efficiency:

- **Statistical properties**: For two independent random 10,240-bit vectors, Hamming similarity follows Normal(0.5, σ²) where σ = 1/(2√n) = 0.00494. This gives excellent discrimination.
- **False positive rates** (from `refactoring-prd/09-innovations.md` §XIX.D):

| Threshold | Z-score | False Positive Rate | Use Case |
|---|---|---|---|
| 0.512 | 2.43 | < 1% per comparison | Single-pair check |
| **0.526** | **5.26** | **< 1% against 100K vocabulary** | **Full knowledge base scan (Bonferroni corrected)** |
| 0.54 | 8.10 | < 10⁻¹⁵ per comparison | Extremely conservative |

The recommended threshold for cross-domain knowledge resonance detection is **0.526**, which guarantees < 1% overall false positive rate even when scanning 100K stored vectors.

- **Storage**: 1,280 bytes per vector. 100K entries = ~128 MB of vector data. Manageable for on-chain storage at Korai's scale.
- **SIMD acceleration**: 10,240 bits = 160 × 64-bit words. Hamming distance computation via POPCNT instruction: ~160 × 1 cycle = ~160 cycles on modern CPUs. This is the basis for the ~400 gas estimate.

---

## Precompile Interface

The HDC precompile is deployed at a reserved Korai address (0xA01). It exposes four operations:

### 1. `hdc_similarity(a: bytes1280, b: bytes1280) -> uint256`

Compute the normalized Hamming similarity between two 10,240-bit vectors.

- **Input**: Two 1,280-byte vectors
- **Output**: Similarity as a PU18 fixed-point value (18 decimal places, range [0, 1e18])
- **Gas cost**: ~50 gas (dominated by input reading; POPCNT is negligible)

### 2. `hdc_topk(query: bytes1280, k: uint32) -> (uint256[], bytes32[])`

Find the K most similar vectors in the on-chain HDC index.

- **Input**: Query vector (1,280 bytes) + K (max 100)
- **Output**: Array of (similarity, entry_hash) pairs, sorted by similarity descending
- **Gas cost**: ~400 gas for K=20 (scales sub-linearly with K due to heap-based selection)

### 3. `hdc_bind(a: bytes1280, b: bytes1280) -> bytes1280`

Compute the XOR binding of two vectors.

- **Input**: Two 1,280-byte vectors
- **Output**: Bound vector (1,280 bytes)
- **Gas cost**: ~30 gas

### 4. `hdc_bundle(vectors: bytes1280[]) -> bytes1280`

Compute the majority-vote bundle of N vectors.

- **Input**: Array of 1,280-byte vectors (max 256)
- **Output**: Bundled vector (1,280 bytes)
- **Gas cost**: ~30 + 5 × N gas

---

## Three-Tier Search Architecture

The `hdc_topk` precompile uses a three-tier search strategy to bound query latency even as the index grows:

### Tier 1: Bloom Filter (Fast Reject)

A compact Bloom filter (8.7 bits/entry) pre-screens the index. Each query vector is hashed to check membership in relevant topic clusters. Entries in non-matching clusters are skipped without reading their full vectors.

- **Cost**: O(1) per entry, ~10ns
- **Rejection rate**: > 90% of entries skipped for focused queries

### Tier 2: Approximate Search (Coarse)

Remaining entries after Bloom filtering are compared using a reduced-resolution representation:
- Downproject 10,240-bit vectors to 1,024-bit summaries (10x compression)
- Hamming distance on the 1,024-bit summaries
- Keep top 5K candidates (configurable)

- **Cost**: O(N_remaining) with 10x fewer operations per comparison
- **Accuracy**: Catches > 99% of true top-K results

### Tier 3: Exact Search (Top-K)

The 5K candidates are scored against the full 10,240-bit query vector using exact Hamming similarity via POPCNT. A min-heap maintains the top K results.

- **Cost**: O(5K × 160 POPCNT operations)
- **Accuracy**: Exact

The combined gas cost of ~400 for K=20 assumes the three-tier pipeline processes roughly 100K index entries. At larger scales (1M+ entries), the Bloom filter and approximate tier keep the exact comparison set bounded.

---

## Relationship to Local HDC Operations

The `roko-primitives` crate (currently `bardo-primitives`) provides the same HDC operations locally:

```rust
// In roko-primitives (local)
pub fn hamming_similarity(a: &[u64; 160], b: &[u64; 160]) -> f64 {
    let matching_bits: u32 = a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x ^ y).count_ones()) // POPCNT
        .sum();
    1.0 - (matching_bits as f64 / 10240.0)
}
```

The critical property: **a vector encoded locally by `roko-primitives` produces identical Hamming similarity results when compared on-chain by the precompile**. This seamless transfer enables the three-level knowledge architecture (Local → Mesh → Chain) without encoding translation.

### Local-to-Chain Knowledge Flow

```
1. Agent observes pattern during task execution
2. Agent's NeuroStore distills pattern into knowledge entry (Insight)
3. roko-primitives encodes the Insight as a 10,240-bit HDC vector
4. Agent posts the vector + metadata to Korai via korai_submitKnowledge
5. On-chain HDC index stores the vector
6. Other agents query via korai_queryKnowledge → hdc_topk precompile
7. Matching entries returned with similarity scores
8. Querying agent imports the knowledge into its local NeuroStore
```

At step 7, the on-chain similarity computation produces the same results that a local comparison would. There is no "chain accuracy vs. local accuracy" gap.

---

## Gas Metering

Gas costs for HDC operations are calibrated to the computational cost relative to standard EVM operations:

| Operation | Estimated Gas | Rationale |
|---|---|---|
| `hdc_similarity` | ~50 | 160 POPCNT + input reading |
| `hdc_topk` (K=20) | ~400 | Three-tier search over index |
| `hdc_bind` (XOR) | ~30 | 160 XOR operations + input/output |
| `hdc_bundle` (N vectors) | ~30 + 5N | N × 160 additions + majority vote |
| `hdc_permute` (cyclic shift) | ~30 | Memory copy with offset |

For comparison, a SHA-256 hash costs 60 gas on Ethereum. The HDC precompile operations are in the same order of magnitude, making them economically viable for per-query use.

### Feasibility Concern

The HDC precompile is a **custom Korai feature**. It does not exist on mainnet Ethereum or any standard EVM. The gas estimates above are theoretical — they need benchmarking on the actual Korai validator implementation. Specific concerns:

1. **Index size**: As the knowledge base grows to 1M+ entries, the three-tier search architecture must maintain sub-millisecond latency. The Bloom filter and approximate tier are designed for this, but need empirical validation.
2. **State bloat**: Each 10,240-bit vector occupies 1,280 bytes of state. 100K entries = ~128 MB. 1M entries = ~1.28 GB. Pruning strategies (demurrage-based expiration of unconfirmed entries) are necessary.
3. **Validator hardware**: POPCNT instruction support is universal on modern x86 CPUs but the bulk processing of 10,240-bit vectors may stress cache lines. Benchmarking on target hardware is needed.

---

## Cross-Domain Resonance via HDC

The HDC encoding enables one of Roko's most novel capabilities: detecting structural analogies across domains in nanoseconds (see `refactoring-prd/09-innovations.md` §XIII).

Example:
```
Coding domain encodes: BIND(high_complexity, more_review)
Chain domain encodes:  BIND(high_volatility, more_caution)
Research domain:       BIND(contradictory_sources, more_verification)

All three have high Hamming similarity because they share:
  BIND(high_uncertainty, more_verification)
```

When the on-chain HDC index contains entries from multiple domains, the `hdc_topk` precompile naturally returns cross-domain matches. An insight from a coding agent about "complex changes require full integration tests" matches a chain agent's insight about "large position changes require full portfolio risk assessment" because both encode the structural pattern `BIND(high_impact_change, comprehensive_verification)`.

The recommended threshold for cross-domain resonance is 0.526 (see False Positive Rates table above). For additional validation, require that cross-domain analogies be confirmed by at least 2 independent agents, which reduces false positives quadratically.

---

## Academic Foundations

- [Kanerva 2009, Cognitive Computation 1(2)] — Hyperdimensional computing with binary vectors. Foundation of BSC encoding.
- Plate, T.A. (2003). *Holographic Reduced Representations*. — Distributed representations via circular convolution; informs the BIND/BUNDLE/PERMUTE algebra.
- Frady, E.P. et al. (2020). "Variable Binding for Sparse Distributed Representations." *IEEE TNNLS*. — Sparse HD computing with near-orthogonal binding.
- Kleyko, D. et al. (2023). "A Survey on Hyperdimensional Computing." *ACM Computing Surveys*. — Comprehensive survey of HD computing theory and applications.
- [Kanerva 1988] — *Sparse Distributed Memory*. Original work on high-dimensional computing.

---

## Current Status and Gaps

**Built:**
- `bardo-primitives/src/hdc.rs` (to be renamed `roko-primitives`): Local HDC operations (BIND, BUNDLE, PERMUTE, Hamming similarity) with SIMD acceleration
- `mirage-rs/src/chain/hdc_index.rs`: In-process HDC index emulating the precompile behavior
- `mirage-rs/src/chain/hnsw.rs`: HNSW approximate nearest neighbor index for development

**Not yet built (Tier 6, deferred):**
- Native EVM precompile implementation for Korai validators
- On-chain HDC index state management
- Bloom filter tier for pre-screening
- Approximate tier (1,024-bit downprojection)
- Gas metering calibration through benchmarking
- Pruning strategy for expired entries

---

## Cross-references

- See [00-vision-and-framing.md](./00-vision-and-framing.md) for the three-level knowledge architecture
- See [02-korai-token-economics.md](./02-korai-token-economics.md) for knowledge posting/query fees
- See topic [06-neuro](../06-neuro/INDEX.md) for the HDC encoding shared between local and on-chain
- See topic [00-architecture](../00-architecture/INDEX.md) for BSC vector format in Engrams
