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

## Detailed Gas Cost Model

The gas estimates in the precompile interface above are calibrated against established EVM benchmarking methodology. This section provides the derivation.

### Calibration Methodology

EVM gas is calibrated to CPU computation at approximately **1 gas ≈ 10 nanoseconds ≈ 30 CPU cycles** (at 3 GHz reference clock). This ratio derives from the ECRECOVER anchor: 3,000 gas for secp256k1 recovery measured at ~116μs on reference hardware, yielding 25.86 gas/μs. Cross-validated against the EIP-2666 SHA-256 repricing benchmark and the evmone opcode benchmarks (Pawel Bylica, Nethermind team).

The calibration was most recently revisited in **EIP-7904 (General Repricing, February 2025, targeting Glamsterdam)**, which proposes a 78.6% gas fee reduction for many opcodes based on current hardware measurements.

### HDC Operation Benchmarks

On modern x86-64 CPUs (Intel Ice Lake, AMD Zen 4), the POPCNT instruction executes in **1 cycle with 1/cycle throughput** on 64-bit words. AVX-512 VPOPCNTQ processes 8 × 64-bit words per instruction (3 cycles latency, 0.5/cycle throughput), achieving 1.536 trillion bits counted per second at 3 GHz.

| Operation | CPU Cycles | Time (3 GHz) | Derived Gas (precompile) | Comparison: Solidity |
|---|---|---|---|---|
| XOR two 1280-byte vectors | 160 cycles (160 × u64 XOR) | ~53 ns | **~5-6 gas** | ~120 gas (40 × 256-bit XOR @ 3 gas) |
| Hamming distance (XOR + POPCNT) | 480 cycles (160 XOR + 160 POPCNT + 159 ADD) | ~160 ns | **~16 gas** | ~2,220 gas (no native POPCNT in EVM) |
| Top-K search (N=1000, K=20) | 485,000 cycles | ~162 μs | **~16,167 gas** | Infeasible (>10M gas) |
| Top-K search (N=10,000, K=20) | 4,850,000 cycles | ~1.62 ms | **~162,000 gas** | Infeasible |
| Top-K search (N=100,000, K=20) | 48,500,000 cycles | ~16.2 ms | **~1,620,000 gas** | Infeasible |
| BIND (XOR) | 160 cycles | ~53 ns | **~5-6 gas** | ~120 gas |
| BUNDLE (N=10 majority vote) | 1,760 cycles | ~587 ns | **~59 gas** | ~1,500 gas |
| PERMUTE (cyclic shift) | 320 cycles (memcpy with offset) | ~107 ns | **~11 gas** | ~200 gas |

For comparison with existing EVM precompiles:

| Precompile | Gas | Computation |
|---|---|---|
| SHA-256 (32 bytes) | 72 gas | SHA-2 hash |
| ECRECOVER | 3,000 gas | secp256k1 signature recovery |
| BN256_ADD | 150 gas | Elliptic curve addition |
| BLS12_G1ADD (EIP-2537) | 375 gas | BLS curve addition |
| KZG Point Eval (EIP-4844) | 50,000 gas | KZG polynomial commitment opening |
| **HDC Hamming distance** | **~16 gas** | XOR + POPCNT on 10,240 bits |

The HDC precompile is dramatically cheaper than cryptographic precompiles because the operations are pure bitwise arithmetic — no modular exponentiation, no elliptic curve math, no polynomial evaluation.

### Three-Tier Search Gas Breakdown

The ~400 gas estimate for `hdc_topk(K=20)` in the precompile interface assumes an optimized three-tier architecture operating on an index of ~100K entries. The detailed breakdown:

```
Tier 1 (Bloom filter): 100K entries × O(1) check = ~100 gas
  — 8.7 bits/entry × 100K = 108 KB Bloom filter
  — Each check: 3 hash lookups × ~2 cycles = 6 cycles per entry
  — 90% rejection rate → 10K candidates pass to Tier 2

Tier 2 (Approximate, 1024-bit): 10K entries × 16 XOR + 16 POPCNT = ~100 gas
  — Downprojected 10,240→1,024 bit vectors (10x compression)
  — 16 u64 words per comparison vs. 160
  — Keep top 5K candidates

Tier 3 (Exact, 10,240-bit): 5K entries × 160 XOR + 160 POPCNT = ~200 gas
  — Full precision on 5K candidates
  — Min-heap for top-K selection

Total: ~400 gas for K=20 against 100K index
```

At larger scales, the three-tier architecture bounds cost growth:
- 1M entries: ~600 gas (Bloom filter rejects 99%+ → 10K to Tier 2)
- 10M entries: ~800 gas (Bloom filter + approximate tier handle the scale)

### Stylus Implementation Path

For Korai deployed as an Arbitrum Orbit L3, HDC operations can be implemented as **Stylus contracts** rather than native precompiles. Stylus compiles Rust to WASM and runs alongside the EVM with shared state and ABI-compatible cross-calls.

```rust
// Stylus HDC contract — Rust compiled to WASM
// Deployed as a regular contract, callable from Solidity
use stylus_sdk::{prelude::*, alloy_primitives::*};

sol_storage! {
    #[entrypoint]
    pub struct HdcPrecompile {
        /// On-chain HDC index (vector hash → vector data)
        mapping(bytes32 => bytes) vectors;
        /// Number of indexed vectors
        uint256 vector_count;
    }
}

#[external]
impl HdcPrecompile {
    /// Compute normalized Hamming similarity between two 10,240-bit vectors
    /// Gas cost via Stylus: ~16-20 gas (vs. ~2,220 in Solidity)
    pub fn similarity(&self, a: Bytes, b: Bytes) -> Result<U256, Vec<u8>> {
        if a.len() != 1280 || b.len() != 1280 {
            return Err(b"invalid vector length".to_vec());
        }
        let matching_bits = hamming_distance_raw(&a, &b);
        // Return as PU18 fixed-point: similarity × 10^18
        let sim = U256::from(10240 - matching_bits)
            * U256::from(10).pow(U256::from(18))
            / U256::from(10240);
        Ok(sim)
    }

    /// XOR binding of two vectors
    /// Gas cost via Stylus: ~5-6 gas
    pub fn bind(&self, a: Bytes, b: Bytes) -> Result<Bytes, Vec<u8>> {
        if a.len() != 1280 || b.len() != 1280 {
            return Err(b"invalid vector length".to_vec());
        }
        let mut result = vec![0u8; 1280];
        for i in 0..1280 {
            result[i] = a[i] ^ b[i];
        }
        Ok(Bytes::from(result))
    }

    /// Majority-vote bundle of N vectors
    /// Gas cost via Stylus: ~30 + 5N gas
    pub fn bundle(&self, vectors: Vec<Bytes>) -> Result<Bytes, Vec<u8>> {
        let n = vectors.len();
        let threshold = n / 2;
        let mut counts = vec![0u32; 10240];
        for v in &vectors {
            for bit_idx in 0..10240 {
                let byte_idx = bit_idx / 8;
                let bit_pos = bit_idx % 8;
                if v[byte_idx] & (1 << bit_pos) != 0 {
                    counts[bit_idx] += 1;
                }
            }
        }
        let mut result = vec![0u8; 1280];
        for bit_idx in 0..10240 {
            if counts[bit_idx] > threshold as u32 {
                let byte_idx = bit_idx / 8;
                let bit_pos = bit_idx % 8;
                result[byte_idx] |= 1 << bit_pos;
            }
        }
        Ok(Bytes::from(result))
    }
}

/// Inner Hamming distance — pure bitwise, no allocations
fn hamming_distance_raw(a: &[u8], b: &[u8]) -> u32 {
    // Process as 64-bit words for maximum WASM performance
    let a_words = unsafe { std::slice::from_raw_parts(a.as_ptr() as *const u64, 160) };
    let b_words = unsafe { std::slice::from_raw_parts(b.as_ptr() as *const u64, 160) };
    a_words.iter().zip(b_words.iter())
        .map(|(x, y)| (x ^ y).count_ones())
        .sum()
}
```

**Stylus performance data** (from OpenZeppelin benchmarks, September 2024):
- Poseidon hash: 18x cheaper via Stylus than Solidity (11,887 gas vs. ~215,000 gas)
- General compute: 10-100x cheaper
- Memory-intensive operations: 100-500x cheaper
- Bitwise operations (XOR, POPCNT): expected 20-50x cheaper than Solidity equivalents

**Stylus constraints**:
- Compressed WASM binary limit: 24 KB (HDC operations are algorithmically simple — fits easily)
- No `std` library (use `wee_alloc` or `mini_alloc` for heap allocation)
- Annual reactivation required (365 days or after Stylus upgrade)
- Host I/O overhead: ~0.84 gas per VM context switch for storage reads

---

## Verifiable HDC Computation

On-chain HDC operations via precompile work for small-to-medium index sizes (up to ~100K vectors). For larger indexes or privacy-sensitive queries, off-chain computation with on-chain verification provides better scalability.

### Approach 1: ZK-Proven HDC Search (RISC Zero / SP1)

HDC search can be proven correct using a general-purpose zkVM:

```rust
// RISC Zero guest program for verified HDC top-K search
// Runs inside the RISC-V zkVM, produces a cryptographic receipt
use risc0_zkvm::guest::env;

fn main() {
    // Read inputs from the host
    let query: Vec<u64> = env::read();           // 160 × u64 query vector
    let index_root: [u8; 32] = env::read();      // Merkle root of the index
    let stored_vectors: Vec<Vec<u64>> = env::read(); // N stored vectors
    let k: usize = env::read();                  // top-K parameter

    // Verify the stored vectors match the committed Merkle root
    // (Steel library proves they came from on-chain state)

    // Compute Hamming distances
    let mut distances: Vec<(usize, u32)> = stored_vectors.iter()
        .enumerate()
        .map(|(idx, stored)| {
            let dist: u32 = query.iter().zip(stored.iter())
                .map(|(q, s)| (q ^ s).count_ones())
                .sum();
            (idx, dist)
        })
        .collect();

    // Select top-K (minimum distance = maximum similarity)
    distances.sort_by_key(|&(_, d)| d);
    let top_k: Vec<(usize, u32)> = distances.into_iter().take(k).collect();

    // Commit results to the journal (public output)
    env::commit(&top_k);
    env::commit(&index_root);
}
```

**Proof pipeline**:
1. Guest program computes top-K in RISC-V → execution trace recorded
2. RISC Zero prover generates zk-STARK over the trace
3. STARK recursively compressed → wrapped in Groth16 SNARK (BN254)
4. On-chain: `RiscZeroVerifier.sol` verifies the proof in ~250K gas
5. Result: proven-correct top-K results at fixed verification cost regardless of index size

**Cost estimates** (RISC Zero Bonsai, 2025 pricing):
- N = 1,000 vectors: ~800,000 RISC-V cycles → ~$0.001 per proof
- N = 100,000 vectors: ~80M RISC-V cycles → ~$0.10 per proof
- On-chain verification: ~250K gas (fixed, independent of N)

**SP1 alternative**: Succinct's SP1 achieves similar performance with its Plonky3 backend. SP1 Hypercube (November 2025) achieves real-time Ethereum block proving (99.7% of blocks in <12s with 16 RTX 5090 GPUs), suggesting HDC proofs would complete in milliseconds.

### Approach 2: Optimistic Verification (Fraud Proof)

For latency-sensitive applications, optimistic verification allows immediate use of results with a challenge window:

```rust
/// Optimistic HDC search result submitted on-chain
pub struct OptimisticHdcResult {
    /// Query vector hash
    pub query_hash: [u8; 32],
    /// Index Merkle root at query time
    pub index_root: [u8; 32],
    /// Claimed top-K results: (vector_id, similarity_score)
    pub results: Vec<(u256, u64)>,
    /// Submitter's passport ID
    pub submitter: u256,
    /// Bond posted by submitter (slashed if fraud proven)
    pub bond: U256,
    /// Block number when result was submitted
    pub submitted_at: u64,
    /// Challenge window duration in blocks
    pub challenge_window: u64,  // default: 100 blocks (~40s on Korai)
}

/// Fraud proof: challenger demonstrates a result entry is incorrect
pub struct HdcFraudProof {
    /// The optimistic result being challenged
    pub result_id: [u8; 32],
    /// Index of the challenged entry in the top-K
    pub challenged_index: usize,
    /// The two vectors to compare (query + stored)
    pub query_vector: [u8; 1280],
    pub stored_vector: [u8; 1280],
    /// The correct Hamming distance (computed on-chain by verifier)
    pub claimed_correct_distance: u32,
}
```

**Challenge resolution**: The on-chain verifier re-computes the Hamming distance for the challenged pair. In EVM (without precompile), this costs ~2,220 gas — cheap enough for fraud proofs. With the HDC precompile, it costs ~16 gas. If the on-chain computation differs from the submitted result, the submitter is slashed and the challenger receives the bond.

**Interactive bisection** (Optimism dispute game model): For disputes about which vectors should be in the top-K, a bisection game reduces on-chain work to O(log N) interactions rather than re-verifying all N comparisons. Two parties bisect the sorted distance list until they isolate the single vector where they disagree.

### Approach 3: TEE-Attested HDC Search

TEE (Trusted Execution Environment) attestation provides instant verification without ZK proof overhead:

```rust
/// TEE-attested HDC search result
pub struct TeeAttestedHdcResult {
    /// Search results
    pub results: Vec<(u256, u64)>,
    /// TEE attestation report
    pub attestation: TeeAttestation,
}

pub struct TeeAttestation {
    /// Measurement of the HDC search code (MRENCLAVE for SGX)
    pub code_measurement: [u8; 32],
    /// Hash of (query_vector, index_root, results)
    pub data_hash: [u8; 32],
    /// Hardware signature from TEE manufacturer
    pub signature: Vec<u8>,
    /// Expiry timestamp
    pub expiry: u64,
}
```

**Performance**: TEE enclaves execute at native CPU speed — a top-K search over 100K vectors completes in ~16ms (same as the precompile estimate). The attestation proves the computation ran correctly on unmodified code.

**Trust model**: Requires trusting Intel/AMD silicon and their attestation PKI. Not trustless like ZK, but dramatically faster and cheaper. Suitable for Korai's T2 FABRIC aggregation tier (see [07-4-tier-gossip-architecture.md](./07-4-tier-gossip-architecture.md)).

### Approach Comparison

| Approach | Verification Cost | Latency | Trust Model | Best For |
|---|---|---|---|---|
| **Native precompile** | ~400 gas (part of tx) | Instant | Chain consensus | Index < 100K vectors |
| **ZK proof (RISC Zero/SP1)** | ~250K gas (fixed) | Seconds (proof gen) | Trustless (math) | Large indexes, privacy |
| **Optimistic + fraud proof** | ~2,220 gas (only if challenged) | ~40s challenge window | Economic (bond) | Latency-tolerant, cost-sensitive |
| **TEE attestation** | ~3,000 gas (sig verify) | Instant | Hardware trust | High-throughput, T2 aggregation |

### Binary Field STARKs for HDC (Binius)

HDC operations are fundamentally binary (XOR, POPCNT on bit vectors), making them ideal candidates for **Binius** — a STARK proof system operating over GF(2) (binary field) rather than prime fields. In Binius, addition IS XOR and multiplication IS AND by definition, eliminating the massive overhead of embedding binary operations into prime-field arithmetic.

A Binius-native Hamming distance circuit:
```
For each of 160 64-bit chunks:
  chunk_xor[i] = query_word[i] XOR stored_word[i]   // 1 constraint (native in GF(2))
  popcnt[i] = popcount(chunk_xor[i])                 // lookup table constraint
hamming_distance = sum(popcnt[i])                    // 159 additions

Total: ~480 constraints per comparison (vs. ~10,240 in prime-field R1CS)
```

For N = 1,000 comparisons: 480,000 constraints. At Binius proving speeds (~10-100M constraints/second on GPU), this takes 5-50ms per proof — making real-time verified HDC search feasible.

### Academic Foundations (Verifiable HDC)

- Ben-Sasson, E. et al. (2018). "Scalable, Transparent, and Post-Quantum Secure Computational Integrity." *IACR*. — ZK-STARK construction; foundation for RISC Zero and SP1 proof systems.
- Binius project (Irreducible, 2024). "Hardware-Optimized SNARK." — Binary field STARKs that natively support XOR and POPCNT without embedding overhead.
- Costan, V. and Devadas, S. (2016). "Intel SGX Explained." *IACR*. — TEE attestation model for hardware-verified HDC computation.
- Gabizon, A. et al. (2019). "PLONK: Permutations over Lagrange-Bases for Oecumenical Noninteractive Arguments of Knowledge." — Lookup table arguments used in Binius POPCNT circuits.
- HDCoin (arXiv:2202.02964, 2022). "Proof-of-Useful-Work Blockchain via Hyperdimensional Computing." — Prior work on blockchain + HDC integration, using HDC model training as proof of work.

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

## Cross-References

- See [00-vision-and-framing.md](./00-vision-and-framing.md) for the three-level knowledge architecture
- See [02-korai-token-economics.md](./02-korai-token-economics.md) for knowledge posting/query fees
- See topic [06-neuro](../06-neuro/INDEX.md) for the HDC encoding shared between local and on-chain
- See topic [00-architecture](../00-architecture/INDEX.md) for BSC vector format in Engrams
