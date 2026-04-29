# HDC On-Chain and Verification

> Depth for [22-REGISTRIES.md](../../unified/22-REGISTRIES.md). How hyperdimensional computing extends to on-chain verification through native precompiles and zero-knowledge proofs.

---

## 1. The Problem: Similarity Search on a Trustless Computer

Off-chain, HDC operations are cheap. A single 10,240-bit XOR + POPCNT costs ~13 ns on commodity hardware. A top-K search over 100,000 vectors takes ~1.3 ms. The entire codebook fits in L1 cache. None of this matters on-chain.

The EVM charges per opcode. A naive Solidity implementation of Hamming distance over 10,240 bits costs ~2,220 gas per comparison (160 words of XOR + POPCNT). A top-20 search over 10,000 entries costs ~22M gas -- more than a block's gas limit. Even storing a single 10,240-bit vector costs ~32,000 gas (160 SSTORE operations at 200 gas each, warm). At these prices, on-chain HDC is unusable.

The solution is a native precompile: a contract at a reserved address that executes HDC operations in compiled Rust/C code rather than interpreted EVM bytecodes. The precompile reduces costs by 25-50x, making HDC operations practical for on-chain knowledge search, identity matching, and reputation-weighted routing.

But precompiles introduce a new problem: **verifiability**. Off-chain, you trust your own CPU. On-chain, validators run the precompile and you trust the consensus mechanism. But for cross-chain verification or light-client proofs, you need to verify that an HDC computation was performed correctly without re-executing it. This is where zero-knowledge proofs enter.

This depth doc redesigns on-chain HDC so that the precompile is a Cell, the three-tier search is a Pipeline Graph, and the four verification approaches are interchangeable Verify Cells.

---

## 2. The HDC Precompile IS a Cell in the EVM

The HDC precompile at address `0xA01` runs the same four operations as the off-chain `HdcVector` type in `roko-primitives/src/hdc.rs`: bind (XOR), bundle (majority vote), similarity (Hamming), and top-K search. The difference is execution context (EVM gas metering) and representation (packed calldata instead of in-memory structs).

```rust
/// HdcPrecompileCell: a Cell that wraps the 0xA01 precompile.
///
/// Same operations as the off-chain BindCell, BundleCell,
/// SimilarityCell, and TopKCell -- but executed via eth_call
/// through a ChainConnector.
///
/// Gas costs (native precompile, not Solidity):
///   hdc_similarity: ~50 gas   (vs ~2,220 in Solidity)
///   hdc_bind:       ~30 gas   (vs ~1,200 in Solidity)
///   hdc_bundle:     ~30+5N gas (N = number of vectors)
///   hdc_topk:       ~400 gas  (K=20, index of 1000 entries)
pub struct HdcPrecompileCell {
    id: CellId,

    /// Chain connector for submitting precompile calls.
    connector: Arc<ChainConnector>,

    /// Precompile address (0xA01 on Korai/Mirage).
    precompile_address: String,
}

impl Cell for HdcPrecompileCell {
    fn name(&self) -> &str { "hdc.precompile" }
    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::Compose, ProtocolId::Score]
    }
    fn estimated_cost(&self) -> Option<Cost> {
        // ~50 gas per similarity check, at current gas prices
        Some(Cost::gas(50))
    }
}

impl HdcPrecompileCell {
    /// Compute Hamming similarity between two vectors on-chain.
    ///
    /// Encodes the two vectors as calldata, calls the precompile
    /// via ChainConnector.query(), and decodes the result.
    pub async fn similarity(
        &self,
        a: &HdcVector,
        b: &HdcVector,
    ) -> Result<f32> {
        let calldata = encode_similarity_call(a, b);
        let result = self.connector.query(QueryRequest {
            kind: "eth_call".into(),
            payload: calldata,
            address: self.precompile_address.clone(),
            ..Default::default()
        }).await?;
        decode_similarity_result(&result.data)
    }

    /// Bind two vectors on-chain (XOR).
    pub async fn bind(
        &self,
        a: &HdcVector,
        b: &HdcVector,
    ) -> Result<HdcVector> {
        let calldata = encode_bind_call(a, b);
        let result = self.connector.query(QueryRequest {
            kind: "eth_call".into(),
            payload: calldata,
            address: self.precompile_address.clone(),
            ..Default::default()
        }).await?;
        decode_vector_result(&result.data)
    }

    /// Bundle N vectors on-chain (majority vote).
    pub async fn bundle(
        &self,
        vectors: &[HdcVector],
    ) -> Result<HdcVector> {
        let calldata = encode_bundle_call(vectors);
        let result = self.connector.query(QueryRequest {
            kind: "eth_call".into(),
            payload: calldata,
            address: self.precompile_address.clone(),
            ..Default::default()
        }).await?;
        decode_vector_result(&result.data)
    }

    /// Top-K similarity search on-chain.
    ///
    /// The precompile maintains an index of registered vectors.
    /// Returns the K most similar vectors to the query.
    pub async fn topk(
        &self,
        query: &HdcVector,
        k: usize,
    ) -> Result<Vec<(ContentHash, f32)>> {
        let calldata = encode_topk_call(query, k);
        let result = self.connector.query(QueryRequest {
            kind: "eth_call".into(),
            payload: calldata,
            address: self.precompile_address.clone(),
            ..Default::default()
        }).await?;
        decode_topk_result(&result.data)
    }
}
```

### 2.1 Calldata Encoding

The precompile uses a compact binary encoding, not ABI encoding. ABI encoding would waste gas on padding 10,240-bit vectors to 32-byte word boundaries.

```rust
/// Precompile function selectors (first 4 bytes of calldata).
const SELECTOR_SIMILARITY: [u8; 4] = [0x01, 0x00, 0x00, 0x00];
const SELECTOR_BIND:       [u8; 4] = [0x02, 0x00, 0x00, 0x00];
const SELECTOR_BUNDLE:     [u8; 4] = [0x03, 0x00, 0x00, 0x00];
const SELECTOR_TOPK:       [u8; 4] = [0x04, 0x00, 0x00, 0x00];

/// Encode a similarity call: selector + 2 * 1280 bytes = 2564 bytes.
fn encode_similarity_call(a: &HdcVector, b: &HdcVector) -> Vec<u8> {
    let mut data = Vec::with_capacity(4 + 2 * HDC_BYTES);
    data.extend_from_slice(&SELECTOR_SIMILARITY);
    data.extend_from_slice(&a.to_bytes());
    data.extend_from_slice(&b.to_bytes());
    data
}

/// Encode a top-K call: selector + 1280 bytes query + 2 bytes K.
fn encode_topk_call(query: &HdcVector, k: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(4 + HDC_BYTES + 2);
    data.extend_from_slice(&SELECTOR_TOPK);
    data.extend_from_slice(&query.to_bytes());
    data.extend_from_slice(&(k as u16).to_le_bytes());
    data
}

/// Decode similarity result: single f32 (4 bytes, LE).
fn decode_similarity_result(data: &[u8]) -> Result<f32> {
    if data.len() < 4 {
        return Err(anyhow!("similarity result too short: {} bytes", data.len()));
    }
    let bytes: [u8; 4] = data[..4].try_into()?;
    Ok(f32::from_le_bytes(bytes))
}
```

### 2.2 Gas Cost Breakdown

Why the precompile is 25-50x cheaper than Solidity:

| Operation | Solidity (interpreted) | Precompile (native) | Speedup |
|---|---|---|---|
| XOR (160 words) | 160 * 3 gas = 480 | ~5 gas (SIMD) | 96x |
| POPCNT (160 words) | 160 * ~14 gas = 2,240 | ~11 gas (hw POPCNT) | 204x |
| Hamming distance | ~2,220 gas total | ~50 gas total | 44x |
| Top-K (K=20, N=1000) | infeasible (~2.2M gas) | ~400 gas | >5,000x |

The Solidity POPCNT cost estimate assumes the standard bit-counting loop (shift-and-mask-and-add). Hardware POPCNT on the precompile's compiled code reduces 160 words to a single vectorized instruction sequence.

### 2.3 Off-Chain / On-Chain Cell Equivalence

The critical invariant: the off-chain `BindCell`, `BundleCell`, `SimilarityCell`, and `TopKCell` (defined in [02-hdc-algebra-and-retrieval.md](../11-memory/02-hdc-algebra-and-retrieval.md)) produce *identical results* to the on-chain precompile. They differ only in cost and trust model:

| Property | Off-chain Cell | On-chain Precompile Cell |
|---|---|---|
| Correctness guarantee | Trust local CPU | Trust consensus (BFT, < 1/3 Byzantine) |
| Cost | ~0 (CPU cycles) | Gas (see table above) |
| Latency | ~13 ns per comparison | ~50 ms per RPC round-trip |
| Index size | Unlimited (RAM-bound) | Contract storage-bound |
| Verifiability | None (local only) | On-chain receipt + optional ZK proof |

A Graph that performs HDC search can swap between off-chain Cells and the precompile Cell without changing its topology. The Route Cell selects based on the trust requirement: local search for agent-internal use, on-chain search for cross-agent verification.

---

## 3. Three-Tier On-Chain Search IS a Pipeline Graph

The off-chain HDC search pipeline (see [02-hdc-algebra-and-retrieval.md](../11-memory/02-hdc-algebra-and-retrieval.md) S5) uses a three-tier strategy: Bloom filter for fast rejection, approximate search for candidates, exact search for ranking. The on-chain version uses the same Pipeline Graph with the same structure, but each tier is implemented as a precompile operation.

```
 Query vector
      |
      v
 [BloomFilterCell] ── ~100 gas, eliminates 90% of index ──>
      |
      v (candidates)
 [ApproximateSearchCell] ── ~100 gas, 16-word compressed vectors ──>
      |
      v (top candidates)
 [ExactSearchCell] ── ~200 gas, full 160-word vectors ──>
      |
      v
 Top-K results with similarity scores
```

### 3.1 Tier 1: Bloom Filter Cell

The Bloom filter is a space-efficient probabilistic data structure. On-chain, it occupies a fixed-size storage slot (e.g., 256 bytes for a 2,048-bit filter). A query hashes the HDC vector's high-entropy words into Bloom positions. If any position is zero, the vector is definitely not in the index.

```rust
/// BloomFilterCell: first tier of on-chain HDC search.
///
/// Input: query HdcVector + index Bloom filter (on-chain storage).
/// Output: boolean (possibly-present or definitely-absent).
///
/// Gas: ~100 (read 256-byte Bloom from storage + hash 4 positions).
/// False positive rate: ~10% at 10K entries with 2048-bit filter.
pub struct BloomFilterCell {
    id: CellId,
    connector: Arc<ChainConnector>,
    bloom_storage_slot: String,
}

impl Cell for BloomFilterCell {
    fn name(&self) -> &str { "hdc.bloom" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::gas(100)) }
}

#[async_trait]
impl Score for BloomFilterCell {
    async fn score(&self, signal: &Signal, _ctx: &CellContext) -> Result<ScoreVector> {
        let query = &signal.hdc_fingerprint;

        // Hash 4 high-entropy words from the query into Bloom positions.
        let positions = bloom_positions(query, 4);

        // Read Bloom filter from on-chain storage.
        let bloom_bytes = self.connector.query(QueryRequest {
            kind: "get_storage_at".into(),
            address: self.bloom_storage_slot.clone(),
            ..Default::default()
        }).await?;

        let present = positions.iter().all(|&pos| {
            let byte_idx = pos / 8;
            let bit_idx = pos % 8;
            bloom_bytes.data.get(byte_idx).map_or(false, |b| b & (1 << bit_idx) != 0)
        });

        Ok(ScoreVector {
            // 1.0 = possibly present (pass to next tier)
            // 0.0 = definitely absent (short-circuit)
            relevance: if present { 1.0 } else { 0.0 },
            ..Default::default()
        })
    }
}

/// Generate Bloom filter positions from an HdcVector.
///
/// Uses the first 4 u64 words of the vector as hash sources,
/// modulo the Bloom filter size (2048 bits).
fn bloom_positions(vector: &HdcVector, k: usize) -> Vec<usize> {
    let bytes = vector.to_bytes();
    (0..k).map(|i| {
        let word_offset = i * 8;
        let word = u64::from_le_bytes(
            bytes[word_offset..word_offset + 8].try_into().unwrap()
        );
        (word % 2048) as usize
    }).collect()
}
```

### 3.2 Tier 2: Approximate Search Cell

Candidates that pass the Bloom filter enter approximate search. This tier uses compressed 16-word (128-byte) vectors -- the first 16 words of each full 160-word vector. At 10x compression, approximate comparisons cost 10x less gas.

```rust
/// ApproximateSearchCell: second tier of on-chain HDC search.
///
/// Input: query HdcVector + candidate list from Bloom tier.
/// Output: top-K candidates ranked by approximate similarity.
///
/// Gas: ~100 for 20 comparisons at 16 words each.
/// Accuracy: approximate Hamming distance over 1,024 of 10,240 bits.
///           Rank correlation with full similarity: >0.95 for D=10,240.
pub struct ApproximateSearchCell {
    id: CellId,
    connector: Arc<ChainConnector>,
}

impl Cell for ApproximateSearchCell {
    fn name(&self) -> &str { "hdc.approximate" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::gas(100)) }
}

impl ApproximateSearchCell {
    /// Compute approximate similarity using only the first 16 words.
    ///
    /// The statistical guarantee: for D_approx = 1024 bits,
    /// sigma = sqrt(1024)/2 = 16 bits. Two vectors that are genuinely
    /// similar (full Hamming > 0.55) will have approximate Hamming > 0.52
    /// with probability > 0.999. The approximation misranks only
    /// vectors near the decision boundary.
    fn approximate_similarity(a: &HdcVector, b: &HdcVector) -> f32 {
        let mut xor_count = 0u32;
        for i in 0..16 {
            xor_count += (a.word(i) ^ b.word(i)).count_ones();
        }
        1.0 - (xor_count as f32 / 1024.0)
    }
}
```

### 3.3 Tier 3: Exact Search Cell

The final tier re-ranks the top candidates from tier 2 using full 160-word vectors. This is the same operation as `HdcVector::similarity()` in `roko-primitives/src/hdc.rs`, but executed via the precompile.

```rust
/// ExactSearchCell: third tier of on-chain HDC search.
///
/// Input: top candidates from approximate tier (typically 20-50).
/// Output: final top-K ranking with exact similarity scores.
///
/// Gas: ~200 for re-ranking 20 candidates at full 160 words each.
pub struct ExactSearchCell {
    id: CellId,
    precompile: Arc<HdcPrecompileCell>,
}

impl Cell for ExactSearchCell {
    fn name(&self) -> &str { "hdc.exact" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::gas(200)) }
}

impl ExactSearchCell {
    /// Re-rank candidates with exact Hamming similarity.
    pub async fn rerank(
        &self,
        query: &HdcVector,
        candidates: &[(ContentHash, HdcVector)],
        k: usize,
    ) -> Result<Vec<(ContentHash, f32)>> {
        let mut scored: Vec<(ContentHash, f32)> = Vec::with_capacity(candidates.len());
        for (hash, vec) in candidates {
            let sim = self.precompile.similarity(query, vec).await?;
            scored.push((*hash, sim));
        }
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        Ok(scored)
    }
}
```

### 3.4 The Complete Pipeline Graph

```rust
/// Build the three-tier on-chain HDC search Pipeline Graph.
///
/// This is the same Pipeline pattern used by the 7-rung Verify
/// pipeline: a linear chain of Cells where each can short-circuit
/// (reject) or pass through to the next.
fn build_onchain_search_graph(
    connector: Arc<ChainConnector>,
    precompile: Arc<HdcPrecompileCell>,
) -> Graph {
    let bloom = BloomFilterCell::new(connector.clone());
    let approx = ApproximateSearchCell::new(connector.clone());
    let exact = ExactSearchCell::new(precompile);

    Graph::builder("hdc.onchain_search")
        .node("bloom", bloom)
        .node("approx", approx)
        .node("exact", exact)
        .edge("bloom", "approx", EdgeCondition::ScoreAbove(0.5))
        .edge("approx", "exact", EdgeCondition::Always)
        .entry("bloom")
        .exit("exact")
        .policy(GraphPolicy {
            failure_strategy: FailureStrategy::ShortCircuit,
            budget: Budget::gas(500),  // total gas budget for full pipeline
            ..Default::default()
        })
        .build()
}

/// Total gas cost analysis for 100K index:
///
/// Tier 1 (Bloom):        ~100 gas, eliminates ~90% -> 10K candidates
/// Tier 2 (Approximate):  ~100 gas, re-ranks 10K -> top 50
/// Tier 3 (Exact):        ~200 gas, re-ranks 50 -> top 20
/// Total:                 ~400 gas
///
/// For comparison, naive full search: 100K * 50 gas = 5M gas.
/// The three-tier pipeline achieves a 12,500x reduction.
```

---

## 4. Verifiable HDC IS a Verify Cell

The on-chain precompile runs inside the validator's execution environment. Validators reach consensus on the result. But what about:

- **Cross-chain verification**: proving to Ethereum that a computation happened on Korai.
- **Light-client proofs**: clients that do not run a full node need proof of correctness.
- **Dispute resolution**: when two parties disagree about an HDC computation result.

These require *verifiable* HDC: a mechanism to prove that an HDC computation was performed correctly without re-executing it. In the unified vocabulary, verifiable HDC is a Verify Cell -- it takes a claim ("vector A has similarity 0.73 to vector B") and produces a Verdict (pass/fail with proof).

### 4.1 The Four Verification Approaches

Each is a Verify Cell with different cost/latency/trust tradeoffs:

```rust
/// Four interchangeable Verify Cells for HDC computation proofs.
///
/// Selected by a Route Cell based on:
///   - Gas budget (ZK is cheapest to verify, most expensive to generate)
///   - Latency tolerance (optimistic is slowest to finalize)
///   - Trust assumptions (TEE requires hardware trust)
///   - Binary-field compatibility (Binius is cheapest for XOR-heavy ops)

// 1. ZK Proof (RISC Zero / SP1)
pub struct ZkHdcVerifyCell { /* ... */ }

// 2. Optimistic (fraud proof, challenge window)
pub struct OptimisticHdcVerifyCell { /* ... */ }

// 3. TEE Attestation (SGX/TDX)
pub struct TeeHdcVerifyCell { /* ... */ }

// 4. Binius Binary-Field STARK
pub struct BiniusHdcVerifyCell { /* ... */ }
```

### 4.2 ZK Proof Verify Cell

Zero-knowledge proofs (RISC Zero zkVM or Succinct SP1) generate a proof that an HDC computation was performed correctly. The proof is verified on-chain at ~250K gas. The prover runs off-chain; only the verification is on-chain.

```rust
/// ZkHdcVerifyCell: verify an HDC computation claim using a ZK proof.
///
/// The prover (off-chain) runs the HDC computation inside a zkVM
/// and generates a receipt (proof). The verifier (this Cell, on-chain)
/// checks the receipt against the claimed inputs and outputs.
///
/// Cost: ~250K gas for on-chain verification.
/// Latency: seconds to minutes for proof generation (off-chain).
/// Trust: cryptographic -- no trust in prover or hardware.
pub struct ZkHdcVerifyCell {
    id: CellId,
    connector: Arc<ChainConnector>,
    verifier_address: String,  // On-chain ZK verifier contract
}

impl Cell for ZkHdcVerifyCell {
    fn name(&self) -> &str { "hdc.verify.zk" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::gas(250_000)) }
}

#[async_trait]
impl Verify for ZkHdcVerifyCell {
    async fn verify(&self, signal: &Signal, ctx: &Context) -> Verdict {
        let start = Instant::now();

        // Extract the claim and proof from the Signal body.
        let claim = extract_hdc_claim(signal);
        let proof = extract_zk_proof(signal);

        // Verify the proof on-chain via the verifier contract.
        let calldata = encode_verify_call(&claim, &proof);
        let result = self.connector.query(QueryRequest {
            kind: "eth_call".into(),
            payload: calldata,
            address: self.verifier_address.clone(),
            ..Default::default()
        }).await;

        let elapsed = start.elapsed().as_millis() as u64;

        match result {
            Ok(response) if decode_bool(&response.data) => {
                Verdict::pass("hdc.verify.zk")
                    .with_detail(format!(
                        "ZK proof verified: {} similarity claim",
                        claim.operation
                    ))
                    .with_duration(elapsed)
            }
            Ok(_) => {
                Verdict::fail("hdc.verify.zk", "ZK proof verification failed")
                    .with_duration(elapsed)
            }
            Err(e) => {
                Verdict::fail("hdc.verify.zk", format!("verifier call failed: {e}"))
                    .with_duration(elapsed)
            }
        }
    }

    fn name(&self) -> &str { "hdc.verify.zk" }
}

/// An HDC computation claim: inputs, operation, and asserted output.
#[derive(Debug, Clone)]
pub struct HdcClaim {
    /// The operation performed (similarity, bind, bundle, topk).
    pub operation: HdcOperation,
    /// Input vector(s) as content hashes (referencing on-chain storage).
    pub input_hashes: Vec<ContentHash>,
    /// Claimed output (similarity score, result vector, or top-K list).
    pub output: HdcClaimOutput,
}

#[derive(Debug, Clone)]
pub enum HdcOperation {
    Similarity,
    Bind,
    Bundle,
    TopK { k: usize },
}

#[derive(Debug, Clone)]
pub enum HdcClaimOutput {
    Similarity(f32),
    Vector(HdcVector),
    TopK(Vec<(ContentHash, f32)>),
}
```

### 4.3 Optimistic Verify Cell

The optimistic approach assumes the claim is correct and opens a challenge window (100 blocks, ~5 seconds at 50ms blocks). Anyone can submit a fraud proof during the window. If no fraud proof appears, the claim is accepted.

```rust
/// OptimisticHdcVerifyCell: assume correct, allow challenges.
///
/// Cost: ~3K gas to post claim + ~50 gas per block of challenge window.
/// Latency: 100 blocks (~5 seconds at 50ms blocks) for finality.
/// Trust: economic -- challengers must bond; false claims are slashed.
///
/// This is the cheapest option when disputes are rare. The 100-block
/// window is short because Korai's 50ms blocks mean 100 blocks is
/// only 5 seconds, not the hours typical of L2 optimistic rollups.
pub struct OptimisticHdcVerifyCell {
    id: CellId,
    connector: Arc<ChainConnector>,
    challenge_window_blocks: u64,  // Default: 100
}

impl Cell for OptimisticHdcVerifyCell {
    fn name(&self) -> &str { "hdc.verify.optimistic" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::gas(3_000)) }
}

#[async_trait]
impl Verify for OptimisticHdcVerifyCell {
    async fn verify(&self, signal: &Signal, ctx: &Context) -> Verdict {
        let start = Instant::now();
        let claim = extract_hdc_claim(signal);

        // 1. Post claim on-chain (registers it for challenge).
        let claim_tx = self.connector.execute(ExecuteRequest {
            payload: encode_post_claim(&claim),
            ..Default::default()
        }).await;

        let Ok(claim_receipt) = claim_tx else {
            return Verdict::fail("hdc.verify.optimistic", "failed to post claim")
                .with_duration(start.elapsed().as_millis() as u64);
        };

        // 2. Wait for challenge window to close.
        let claim_block = claim_receipt.block_number;
        let finality_block = claim_block + self.challenge_window_blocks;

        // Poll for finality (or challenge).
        loop {
            let current = self.connector.query(QueryRequest {
                kind: "block_number".into(),
                ..Default::default()
            }).await;

            let current_block = match current {
                Ok(resp) => decode_block_number(&resp.data),
                Err(_) => break,
            };

            if current_block >= finality_block {
                break;
            }

            // Check if a fraud proof was submitted.
            let challenged = self.check_challenge(&claim, claim_block).await;
            if challenged {
                return Verdict::fail(
                    "hdc.verify.optimistic",
                    "claim challenged with fraud proof",
                ).with_duration(start.elapsed().as_millis() as u64);
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // 3. No challenge -> claim accepted.
        Verdict::pass("hdc.verify.optimistic")
            .with_detail(format!(
                "claim finalized after {} blocks, no challenge",
                self.challenge_window_blocks
            ))
            .with_duration(start.elapsed().as_millis() as u64)
    }

    fn name(&self) -> &str { "hdc.verify.optimistic" }
}
```

### 4.4 TEE Attestation Verify Cell

Trusted Execution Environment (SGX, TDX) attestation proves that the HDC computation ran inside a secure enclave. The attestation is a signed quote that the on-chain verifier checks against a known enclave measurement.

```rust
/// TeeHdcVerifyCell: verify via TEE attestation.
///
/// Cost: ~3K gas for attestation verification.
/// Latency: milliseconds (no challenge window, no proof generation).
/// Trust: hardware -- assumes the TEE is not compromised.
///
/// Best for low-latency, medium-trust scenarios where the TEE
/// manufacturer (Intel, AMD) is trusted but the prover is not.
pub struct TeeHdcVerifyCell {
    id: CellId,
    connector: Arc<ChainConnector>,
    attestation_verifier: String,  // On-chain TEE verifier contract
}

impl Cell for TeeHdcVerifyCell {
    fn name(&self) -> &str { "hdc.verify.tee" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::gas(3_000)) }
}

#[async_trait]
impl Verify for TeeHdcVerifyCell {
    async fn verify(&self, signal: &Signal, ctx: &Context) -> Verdict {
        let start = Instant::now();
        let claim = extract_hdc_claim(signal);
        let attestation = extract_tee_attestation(signal);

        // Verify the TEE attestation on-chain.
        let calldata = encode_tee_verify(&claim, &attestation);
        let result = self.connector.query(QueryRequest {
            kind: "eth_call".into(),
            payload: calldata,
            address: self.attestation_verifier.clone(),
            ..Default::default()
        }).await;

        let elapsed = start.elapsed().as_millis() as u64;

        match result {
            Ok(response) if decode_bool(&response.data) => {
                Verdict::pass("hdc.verify.tee")
                    .with_detail("TEE attestation verified")
                    .with_duration(elapsed)
            }
            Ok(_) => {
                Verdict::fail("hdc.verify.tee", "TEE attestation invalid")
                    .with_duration(elapsed)
            }
            Err(e) => {
                Verdict::fail("hdc.verify.tee", format!("attestation check failed: {e}"))
                    .with_duration(elapsed)
            }
        }
    }

    fn name(&self) -> &str { "hdc.verify.tee" }
}
```

### 4.5 Binius Binary-Field STARK Verify Cell

Binius is a STARK proof system designed for binary fields (GF(2)). HDC operations are native XOR and POPCNT -- both are natural in GF(2). This makes Binius uniquely efficient for HDC proofs: where a general-purpose zkVM treats XOR as arithmetic over a large prime field (expensive), Binius treats XOR as a native field operation (free).

```rust
/// BiniusHdcVerifyCell: verify via binary-field STARK.
///
/// Cost: ~150K gas for on-chain verification (cheaper than ZK because
///       the proof is smaller for binary operations).
/// Latency: seconds for proof generation (faster than ZK for XOR-heavy ops).
/// Trust: cryptographic -- same as ZK, no hardware trust needed.
///
/// Why Binius is optimal for HDC:
///   - XOR in GF(2) is a single constraint (vs ~120 constraints in R1CS)
///   - Hamming distance (POPCNT) decomposes into ~480 constraints per
///     comparison at D=10,240 (vs ~2,200 in general-purpose STARKs)
///   - Proof size: ~10 KB (vs ~100 KB for RISC Zero receipts)
///   - Verification gas: ~150K (vs ~250K for ZK)
pub struct BiniusHdcVerifyCell {
    id: CellId,
    connector: Arc<ChainConnector>,
    verifier_address: String,
}

impl Cell for BiniusHdcVerifyCell {
    fn name(&self) -> &str { "hdc.verify.binius" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::gas(150_000)) }
}

#[async_trait]
impl Verify for BiniusHdcVerifyCell {
    async fn verify(&self, signal: &Signal, ctx: &Context) -> Verdict {
        let start = Instant::now();
        let claim = extract_hdc_claim(signal);
        let proof = extract_binius_proof(signal);

        // The circuit for HDC similarity:
        //   1. Assert input vectors match committed values (hash check)
        //   2. Compute XOR of 160 u64 words (160 free constraints in GF(2))
        //   3. Compute POPCNT of XOR result (480 binary decomposition constraints)
        //   4. Assert POPCNT matches claimed Hamming distance
        //
        // Total: ~640 constraints for a similarity check.
        // Compare: ~2,200 constraints in a general-purpose STARK.
        let calldata = encode_binius_verify(&claim, &proof);
        let result = self.connector.query(QueryRequest {
            kind: "eth_call".into(),
            payload: calldata,
            address: self.verifier_address.clone(),
            ..Default::default()
        }).await;

        let elapsed = start.elapsed().as_millis() as u64;

        match result {
            Ok(response) if decode_bool(&response.data) => {
                Verdict::pass("hdc.verify.binius")
                    .with_detail("Binius binary-field STARK verified")
                    .with_duration(elapsed)
            }
            Ok(_) => {
                Verdict::fail("hdc.verify.binius", "Binius proof verification failed")
                    .with_duration(elapsed)
            }
            Err(e) => {
                Verdict::fail("hdc.verify.binius", format!("verifier call failed: {e}"))
                    .with_duration(elapsed)
            }
        }
    }

    fn name(&self) -> &str { "hdc.verify.binius" }
}
```

---

## 5. Route Cell Selects Verification Strategy

The four Verify Cells are interchangeable. A Route Cell selects among them based on the claim's requirements:

```rust
/// HdcVerificationRouter: Route Cell that selects the verification strategy.
///
/// Decision factors:
///   - Gas budget: ZK (250K) vs Binius (150K) vs TEE (3K) vs Optimistic (3K)
///   - Latency tolerance: TEE (ms) vs Optimistic (5s) vs Binius (s) vs ZK (min)
///   - Trust model: ZK/Binius (cryptographic) vs TEE (hardware) vs Optimistic (economic)
///   - Claim value: high-value claims justify higher verification cost
pub struct HdcVerificationRouter {
    id: CellId,

    /// Available verification strategies, ordered by preference.
    strategies: Vec<VerificationStrategy>,
}

#[derive(Debug, Clone)]
pub struct VerificationStrategy {
    pub cell: Arc<dyn Verify>,
    pub gas_cost: u64,
    pub latency_ms: u64,
    pub trust_level: TrustLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TrustLevel {
    Economic,       // Optimistic: trust is backed by bonds
    Hardware,       // TEE: trust the enclave manufacturer
    Cryptographic,  // ZK / Binius: trust math only
}

impl Cell for HdcVerificationRouter {
    fn name(&self) -> &str { "hdc.verify.router" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Route] }
}

#[async_trait]
impl Route for HdcVerificationRouter {
    async fn route(
        &self,
        signal: &Signal,
        ctx: &CellContext,
    ) -> Result<RoutingDecision> {
        let claim = extract_hdc_claim(signal);
        let gas_budget = ctx.remaining_gas_budget();
        let latency_budget_ms = ctx.deadline_remaining_ms();
        let min_trust = ctx.required_trust_level();

        // Filter by hard constraints.
        let eligible: Vec<_> = self.strategies.iter()
            .filter(|s| s.gas_cost <= gas_budget)
            .filter(|s| s.latency_ms <= latency_budget_ms)
            .filter(|s| s.trust_level >= min_trust)
            .collect();

        if eligible.is_empty() {
            return Err(RouteError::NoEligibleStrategy);
        }

        // Among eligible strategies, minimize gas cost.
        // Tie-break by trust level (prefer stronger guarantees).
        let best = eligible.iter()
            .min_by_key(|s| (s.gas_cost, std::cmp::Reverse(s.trust_level)))
            .unwrap();

        Ok(RoutingDecision {
            target_cell: best.cell.clone(),
            reason: format!(
                "selected {} (gas={}, latency={}ms, trust={:?})",
                best.cell.name(), best.gas_cost, best.latency_ms, best.trust_level
            ),
        })
    }
}
```

### 5.1 Decision Matrix

| Scenario | Selected strategy | Reason |
|---|---|---|
| Cross-chain bridge claim, high value | ZK or Binius | Cryptographic trust required, no hardware assumption |
| Same-chain knowledge search, low value | Optimistic | Cheapest gas, disputes rare |
| Real-time agent routing decision | TEE | Lowest latency, acceptable hardware trust |
| Binary-heavy computation (similarity/bind) | Binius | Optimal constraint count for XOR operations |
| Mixed computation (bundle + topK) | ZK | General-purpose, handles any HDC operation |

### 5.2 Composed Verification Graph

For high-value claims, the Route Cell can select a *composed* verification: run both TEE (fast, hardware trust) and Binius (slow, cryptographic trust) in parallel. The claim is accepted when either proof verifies. This is a standard fan-out Graph with an OR-merge:

```
                 +---> [TeeHdcVerifyCell] ---+
                 |                           |
  [RouterCell] --+                           +---> [OrMerge] ---> Verdict
                 |                           |
                 +---> [BiniusHdcVerifyCell] -+
```

The OrMerge Cell passes the first Verdict that arrives. If TEE returns `pass` in 5ms, the claim is accepted immediately; the Binius proof continues in the background as a confirmation. If TEE fails (enclave attestation rejected), the system falls back to the Binius result.

---

## 6. On-Chain Index Management

The precompile's top-K search operates over an index of registered vectors. The index is stored as contract state and updated when new knowledge entries are published on-chain.

```rust
/// ChainHdcIndex: manages the on-chain vector index.
///
/// Vectors are registered via store() and removed via remove().
/// The index is organized by domain (coding, security, research, etc.)
/// for scoped searches.
///
/// Storage layout (per domain):
///   - Bloom filter: 256 bytes (1 storage slot)
///   - Compressed vectors: 128 bytes each (16-word approximation)
///   - Full vectors: 1,280 bytes each (stored in precompile's native format)
///   - Metadata: content hash -> (block_number, publisher_passport_id)
pub struct ChainHdcIndex {
    id: CellId,
    connector: Arc<ChainConnector>,
    index_contract: String,
}

impl ChainHdcIndex {
    /// Register a new vector in the on-chain index.
    ///
    /// Gas cost: ~32,000 (160 SSTORE operations for full vector)
    /// + ~1,000 (Bloom filter update)
    /// + ~2,000 (compressed vector storage)
    /// Total: ~35,000 gas per registration.
    pub async fn register(
        &self,
        content_hash: &ContentHash,
        vector: &HdcVector,
        domain: &str,
        passport_id: u128,
    ) -> Result<()> {
        let calldata = encode_register_vector(content_hash, vector, domain, passport_id);
        self.connector.execute(ExecuteRequest {
            payload: calldata,
            ..Default::default()
        }).await?;
        Ok(())
    }

    /// Query the index: Bloom check + approximate + exact.
    /// Delegates to the three-tier Pipeline Graph.
    pub async fn search(
        &self,
        query: &HdcVector,
        domain: &str,
        k: usize,
    ) -> Result<Vec<(ContentHash, f32)>> {
        // Internally uses build_onchain_search_graph()
        // See S3.4 for the Pipeline Graph definition.
        let graph = build_onchain_search_graph(
            self.connector.clone(),
            Arc::new(HdcPrecompileCell::new(self.connector.clone())),
        );

        let query_signal = Signal::builder(Kind::Query)
            .hdc_fingerprint(query.clone())
            .body(Body::Structured(SearchParams { domain: domain.into(), k }))
            .build();

        let results = graph.execute(vec![query_signal], &CellContext::default()).await?;
        decode_search_results(&results)
    }
}
```

---

## What This Enables

1. **Global knowledge deduplication.** Before publishing a knowledge entry on-chain, the publisher runs a three-tier search to check if similar knowledge already exists. If similarity exceeds the resonance threshold (0.526), the entry is flagged as redundant and the publisher saves gas. This is the on-chain equivalent of the off-chain novelty check, applied at global scale.

2. **Cross-agent knowledge discovery.** Agent A publishes a knowledge entry about Rust trait objects. Agent B, working on a different continent, searches on-chain for "similar to my problem" using HDC. The three-tier pipeline finds Agent A's entry in ~400 gas. Agent B now knows what Agent A knows, without ever communicating directly.

3. **Verifiable reputation claims.** An agent claims "I have Gold-tier reputation in the coding domain." A verifier runs the on-chain HDC search to confirm: does the agent's work history (published as HDC-fingerprinted validation records) match the claimed tier? The ZK proof makes this verifiable on any chain.

4. **Efficient dispute resolution.** When two agents disagree about a knowledge claim, the arbiter checks: do the two agents' HDC fingerprints for the claim have high similarity (they agree on content but disagree on interpretation) or low similarity (they are talking about different things)? The precompile makes this a ~50-gas check.

5. **Privacy-preserving publication.** An agent publishes an HDC fingerprint on-chain without revealing the underlying knowledge. Other agents can check similarity against the fingerprint without seeing the content. This is a natural consequence of HDC's one-way property: you cannot reconstruct the input from the fingerprint.

---

## Feedback Loops

1. **Index growth -> Bloom filter tuning.** As the on-chain index grows, the Bloom filter's false positive rate increases. The system monitors the ratio of Bloom hits to actual matches. When the ratio drops below a threshold, the Bloom filter is rebuilt with more bits. This is an Observe Cell watching the search Pipeline's tier-1-to-tier-2 pass-through rate.

2. **Verification cost -> Route adaptation.** The Route Cell tracks the actual gas cost and latency of each verification strategy. Over time, if Binius proofs consistently verify faster than estimated, the Route Cell updates its cost model and selects Binius more often. This is the same bandit-routing pattern used by the CascadeRouter for LLM model selection.

3. **Search miss rate -> registration incentive.** If agents frequently search for vectors not in the on-chain index, the system emits a Pulse indicating "knowledge gap in domain X." This can trigger a KORAI bounty for agents to publish knowledge in the underserved domain. The feedback loop: search misses -> bounties -> publications -> fewer misses.

4. **Precompile gas accounting -> emission schedule.** The gas consumed by HDC precompile calls is tracked separately in block metrics. If HDC operations dominate gas usage (meaning the knowledge network is heavily used), the emission schedule can adjust minting rates to reward HDC-heavy validators. This connects the precompile to the token economy.

---

## Open Questions

1. **Precompile upgrade path.** If the HDC vector dimension changes (e.g., 10,240 -> 20,480 bits for higher capacity), the precompile's binary encoding changes. How is this handled? A new precompile address (0xA01 -> 0xA02)? A version field in the calldata? The existing `HdcVector` in `roko-primitives` is fixed at 160 u64 words, but the math works at any dimension.

2. **Index sharding.** At 100K entries, the three-tier search costs ~400 gas. At 10M entries, the Bloom filter saturates and tier-1 stops being useful. Should the index be sharded by domain? By publication date? By publisher tier? Sharding changes the search Graph topology (parallel domain searches merged by a BundleCell).

3. **Proof recursion.** Can a Binius proof of 100 similarity comparisons be batched into a single proof that costs less than 100 individual proofs? If so, the batch verification Cell could amortize the ~150K gas across many comparisons, making per-comparison cost negligible. This depends on Binius's recursion overhead for binary circuits.

4. **Approximate search accuracy guarantees.** The tier-2 approximate search uses 16/160 words (10% of bits). The statistical argument (rank correlation > 0.95) holds in expectation. What is the worst-case reranking error? For adversarially-constructed vectors, the approximation could fail. Should tier-2 include a randomized projection (Johnson-Lindenstrauss) for robustness?

5. **Cross-precompile interaction.** The Agent Registry precompile (0xA02) stores agent identities. The HDC precompile (0xA01) stores knowledge fingerprints. Should agents be able to query "which agents have published knowledge similar to this fingerprint?" in a single call? This would require cross-precompile calls or a join precompile, neither of which is standard EVM.
