# Hyperdimensional Technical Analysis

> HDC encodes TA patterns as 10,240-bit vectors. Pattern algebra (bind, bundle, permute) enables nanosecond cross-domain similarity search, temporal composition, and shift-invariant pattern matching.


> **Implementation**: Specified

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [06-neuro](../06-neuro/INDEX.md) for HDC basics, [01-oracle-trait](./01-oracle-trait.md) for Oracle integration
**Key sources**: `bardo-backup/prd/23-ta/01-hyperdimensional-technical-analysis.md`, `refactoring-prd/09-innovations.md` §XIII

---

## Why HDC for technical analysis

Traditional TA relies on numerical time series operations — moving averages, statistical tests, regression models. These are computationally efficient but fragile: they require exact feature alignment, cannot handle structural similarity across domains, and fail at compositional pattern matching.

Hyperdimensional Computing (HDC) solves these problems by encoding patterns as high-dimensional binary vectors (10,240 bits = 1,280 bytes) and performing pattern algebra via bitwise operations:

| Operation | HDC | Cost | What it does |
|---|---|---|---|
| **Bind** (XOR) | `A ⊕ B` | ~2ns | Associate two concepts: "price" bound with "rising" |
| **Bundle** (majority) | `[A, B, C]` | ~10ns | Merge patterns: composite of multiple observations |
| **Permute** (rotate) | `π(A)` | ~1ns | Encode position/sequence: "first observation" vs. "second" |
| **Similarity** (Hamming) | `d(A, B)` | ~13ns | Compare patterns: how similar are these two vectors? |

With AVX-512 SIMD on modern x86, each operation processes the full 10,240-bit vector in one pass (XOR 160 u64 words + popcount). ARM NEON is roughly 2-3x slower. Performance numbers from Kleyko et al. (2022, *ACM Computing Surveys*) and Kanerva (2009, *Cognitive Computation*).

The critical advantage: **cross-domain pattern matching at nanosecond cost**. A pattern learned in the chain domain (e.g., "volatility spike precedes mean reversion") can be detected as structurally similar to a pattern in the coding domain (e.g., "error rate spike precedes test stabilization") without explicit cross-domain translation.

---

## Pattern algebra for TA

### Role-filler composition

TA patterns are encoded as role-filler pairs — a "role" (what kind of observation) bound to a "filler" (the specific value or state):

```rust
/// Encode a TA observation as a role-filler HDC vector.
///
/// Role = what kind of observation (price, volume, rsi, build_time, test_rate)
/// Filler = the specific value quantized into an HDC codebook
///
/// The binding preserves both pieces: given the composite,
/// unbinding with the role recovers the filler (approximately).
pub fn encode_observation(role: &HdcVector, filler: &HdcVector) -> HdcVector {
    role.xor(filler)  // BIND operation
}

/// Encode a complete TA state as a bundle of role-filler pairs.
///
/// Example (chain): BUNDLE(
///     BIND(price_role, price_filler),
///     BIND(volume_role, volume_filler),
///     BIND(rsi_role, rsi_filler),
///     BIND(macd_role, macd_filler),
/// )
pub fn encode_ta_state(observations: &[(HdcVector, HdcVector)]) -> HdcVector {
    let bound: Vec<HdcVector> = observations.iter()
        .map(|(role, filler)| role.xor(filler))
        .collect();
    HdcVector::bundle(&bound)  // majority vote across all bound pairs
}
```

### Temporal composition

Time series patterns are encoded using permutation to represent sequence:

```rust
/// Encode a temporal pattern: a sequence of observations over time.
///
/// Uses permutation (bit rotation) to mark temporal position:
///   π^0(obs_0) ⊕ π^1(obs_1) ⊕ π^2(obs_2) ⊕ ...
///
/// This creates a single vector that encodes the SEQUENCE of
/// observations, not just their aggregate.
///
/// Example: "RSI rose from 30 to 50 to 70 over 3 ticks"
///   = BIND(PERM(rsi_30, 0), PERM(rsi_50, 1), PERM(rsi_70, 2))
pub fn encode_temporal_pattern(observations: &[HdcVector]) -> HdcVector {
    let permuted: Vec<HdcVector> = observations.iter()
        .enumerate()
        .map(|(i, obs)| obs.permute(i as u32))
        .collect();
    HdcVector::bundle(&permuted)
}
```

### Shift-invariant pattern matching

Temporal patterns should be recognizable regardless of when they start. Shift invariance is achieved by checking similarity at all offsets:

```rust
/// Check if a pattern exists anywhere in a longer sequence.
///
/// Slides the pattern template across the sequence and returns
/// the maximum similarity at any offset.
///
/// This is how TA patterns like "head and shoulders" are detected
/// regardless of when they occurred in the time series.
pub fn shift_invariant_match(
    pattern: &HdcVector,
    sequence: &[HdcVector],
    pattern_len: usize,
) -> (f64, usize) {
    let mut best_similarity = 0.0;
    let mut best_offset = 0;

    for offset in 0..=(sequence.len() - pattern_len) {
        let window = &sequence[offset..offset + pattern_len];
        let window_encoded = encode_temporal_pattern(window);
        let similarity = pattern.hamming_similarity(&window_encoded);

        if similarity > best_similarity {
            best_similarity = similarity;
            best_offset = offset;
        }
    }

    (best_similarity, best_offset)
}
```

---

## DeFi primitive encoding

The chain domain defines HDC codebooks for DeFi primitives. Each primitive type gets a unique role vector, and specific instances are encoded as fillers:

```rust
/// DeFi primitive HDC codebook.
///
/// Each primitive type is a randomly generated 10,240-bit vector.
/// These are fixed at initialization and shared across all agents
/// (deterministic from seed).
pub struct DeFiCodebook {
    // Transaction type roles
    pub swap: HdcVector,
    pub liquidity_provision: HdcVector,
    pub lending: HdcVector,
    pub borrowing: HdcVector,
    pub vault_deposit: HdcVector,
    pub staking: HdcVector,
    pub restaking: HdcVector,
    pub perpetual: HdcVector,
    pub options: HdcVector,
    pub yield_farming: HdcVector,
    pub streaming_payment: HdcVector,
    pub gas_token: HdcVector,
    pub intent: HdcVector,
    pub rwa: HdcVector,
    pub cross_chain: HdcVector,
    pub account_abstraction: HdcVector,
    pub prediction_market: HdcVector,

    // Parameter roles
    pub amount: HdcVector,
    pub price: HdcVector,
    pub slippage: HdcVector,
    pub gas_cost: HdcVector,
    pub protocol: HdcVector,
    pub chain: HdcVector,
    pub pool: HdcVector,
    pub token_pair: HdcVector,

    // Numeric codebooks (quantized value ranges)
    pub amount_codebook: QuantizedCodebook,
    pub price_codebook: QuantizedCodebook,
    pub percentage_codebook: QuantizedCodebook,
}
```

### Quantized numeric encoding

Continuous values are quantized into discrete HDC vectors using thermometer encoding:

```rust
/// Quantized codebook for encoding continuous values as HDC vectors.
///
/// Uses thermometer encoding: for value in range [min, max] with N levels,
/// the encoded vector is a blend of the level vectors weighted by proximity.
///
/// This preserves ordinal relationships: encode(3.0) is more similar to
/// encode(4.0) than to encode(100.0).
pub struct QuantizedCodebook {
    /// Level vectors, one per quantization level.
    levels: Vec<HdcVector>,
    /// Value range.
    min: f64,
    max: f64,
    /// Number of quantization levels.
    n_levels: usize,
}

impl QuantizedCodebook {
    /// Encode a continuous value as an HDC vector.
    pub fn encode(&self, value: f64) -> HdcVector {
        let normalized = (value - self.min) / (self.max - self.min);
        let level = (normalized * self.n_levels as f64).clamp(0.0, (self.n_levels - 1) as f64);
        let lower = level.floor() as usize;
        let upper = (lower + 1).min(self.n_levels - 1);
        let weight = level - lower as f64;

        // Interpolate between adjacent level vectors
        self.levels[lower].weighted_bundle(&self.levels[upper], 1.0 - weight, weight)
    }
}
```

### Pattern composition queries

Complex TA patterns are composed from primitive encodings:

```rust
/// Example: encode "a large ETH swap on Uniswap with high slippage"
///
/// This creates a single 10,240-bit vector that captures the
/// full semantic content of the pattern.
pub fn encode_swap_pattern(
    codebook: &DeFiCodebook,
    token_pair: &str,
    amount: f64,
    slippage: f64,
    protocol: &str,
) -> HdcVector {
    let type_binding = codebook.swap.clone();
    let pair_binding = codebook.token_pair.xor(&codebook.encode_string(token_pair));
    let amount_binding = codebook.amount.xor(&codebook.amount_codebook.encode(amount));
    let slip_binding = codebook.slippage.xor(&codebook.percentage_codebook.encode(slippage));
    let proto_binding = codebook.protocol.xor(&codebook.encode_string(protocol));

    HdcVector::bundle(&[type_binding, pair_binding, amount_binding, slip_binding, proto_binding])
}

/// Query: "find all patterns similar to large swaps with high slippage"
pub fn query_similar_patterns(
    pattern: &HdcVector,
    memory: &[HdcVector],
    threshold: f64,  // typically 0.526 per refactoring-prd/09-innovations.md §XIII
) -> Vec<(usize, f64)> {
    memory.iter()
        .enumerate()
        .filter_map(|(i, m)| {
            let sim = pattern.hamming_similarity(m);
            if sim > threshold { Some((i, sim)) } else { None }
        })
        .collect()
}
```

---

## Cross-domain pattern matching

The deepest value of HDC for TA is cross-domain insight resonance (see `refactoring-prd/09-innovations.md` §XIII). When a coding oracle encodes "high churn in auth module" and a chain oracle encodes "high volatility in ETH/USDC," the HDC vectors are structurally similar because both encode `BIND(high_uncertainty, critical_subsystem)`:

```rust
/// Cross-domain insight resonance detection.
///
/// Continuously cross-correlate new Engrams against the HDC knowledge
/// base across ALL domains. When similarity exceeds threshold (0.526),
/// emit a cross-domain insight.
pub fn detect_cross_domain_resonance(
    new_engram: &Engram,
    all_domain_knowledge: &[Engram],
    threshold: f64,
) -> Vec<CrossDomainInsight> {
    let new_hv = new_engram.hdc_vector();
    let new_domain = new_engram.domain();

    all_domain_knowledge.iter()
        .filter(|k| k.domain() != new_domain)  // cross-domain only
        .filter_map(|k| {
            let sim = new_hv.hamming_similarity(&k.hdc_vector());
            if sim > threshold {
                Some(CrossDomainInsight {
                    source_engram: new_engram.id,
                    target_engram: k.id,
                    source_domain: new_domain.clone(),
                    target_domain: k.domain().clone(),
                    similarity: sim,
                    description: format!(
                        "Pattern in {} domain has structural similarity ({:.3}) to pattern in {} domain",
                        new_domain, sim, k.domain()
                    ),
                })
            } else {
                None
            }
        })
        .collect()
}
```

The threshold of 0.526 comes from information-theoretic analysis: with 10,240-bit vectors, random vectors have expected Hamming similarity of 0.500 with standard deviation of ~0.005. A threshold of 0.526 (5σ above chance) ensures that detected similarities are statistically significant.

---

## Coding domain HDC codebook

The coding domain has its own HDC codebook, structurally parallel to the DeFi codebook:

```rust
pub struct CodingCodebook {
    // Event type roles
    pub commit: HdcVector,
    pub build: HdcVector,
    pub test_run: HdcVector,
    pub lint: HdcVector,
    pub benchmark: HdcVector,
    pub deploy: HdcVector,
    pub review: HdcVector,
    pub merge: HdcVector,

    // Metric roles
    pub complexity: HdcVector,
    pub coverage: HdcVector,
    pub pass_rate: HdcVector,
    pub build_time: HdcVector,
    pub error_count: HdcVector,
    pub churn_rate: HdcVector,

    // Scope roles
    pub file: HdcVector,
    pub module: HdcVector,
    pub crate_scope: HdcVector,
    pub workspace: HdcVector,

    // Numeric codebooks
    pub count_codebook: QuantizedCodebook,
    pub rate_codebook: QuantizedCodebook,
    pub duration_codebook: QuantizedCodebook,
}
```

Because both codebooks use the same HDC algebra (10,240-bit BSC, XOR bind, majority bundle), patterns from either domain can be compared directly via Hamming similarity. This is the mechanism that enables cross-domain insight transfer at nanosecond cost.

---

## HDC pattern memory — The pattern store

```rust
/// HDC pattern memory for TA.
///
/// Stores encoded TA patterns as a searchable vector space.
/// Queries return the most similar patterns, enabling:
/// - "Have I seen this pattern before?" (recall)
/// - "What patterns are similar to this?" (analogy)
/// - "What patterns from other domains match?" (transfer)
pub struct PatternStore {
    /// All stored patterns indexed by domain.
    patterns: HashMap<OracleDomain, Vec<StoredPattern>>,

    /// Cross-domain index for resonance detection.
    cross_domain_index: Vec<(OracleDomain, HdcVector, ContentHash)>,
}

pub struct StoredPattern {
    /// The encoded HDC vector.
    pub vector: HdcVector,

    /// The Engram this pattern was derived from.
    pub source_engram: ContentHash,

    /// The outcome when this pattern last occurred.
    pub outcome: Option<PredictionOutcome>,

    /// How often this pattern has been observed.
    pub frequency: u64,

    /// Reliability: how often did this pattern's predicted outcome match actual?
    pub reliability: f64,
}

impl PatternStore {
    /// Find the K most similar patterns to a query.
    ///
    /// Cost: O(N) with N = total patterns, ~13ns per comparison.
    /// For 100K patterns: ~1.3ms. For 1M: ~13ms.
    pub fn find_similar(
        &self,
        query: &HdcVector,
        domain: Option<&OracleDomain>,
        k: usize,
        threshold: f64,
    ) -> Vec<(f64, &StoredPattern)> {
        let candidates = match domain {
            Some(d) => self.patterns.get(d).map(|v| v.as_slice()).unwrap_or(&[]),
            None => &self.cross_domain_index.iter()
                .map(|(_, v, _)| v)
                .collect::<Vec<_>>(),  // simplified
        };

        let mut results: Vec<_> = candidates.iter()
            .map(|p| (query.hamming_similarity(&p.vector), p))
            .filter(|(sim, _)| *sim > threshold)
            .collect();

        results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        results.truncate(k);
        results
    }
}
```

---

## Integration with Dreams — Pattern consolidation

During Delta-frequency consolidation (Dreams), the HDC pattern store undergoes three operations:

1. **NREM replay**: High-value patterns are replayed and their reliability scores updated based on accumulated outcomes.
2. **REM recombination**: Novel pattern compositions are generated by bundling existing patterns with random perturbation (mutation via XOR with noise vector).
3. **Pruning**: Patterns with reliability below 0.3 after 10+ observations are removed.

```rust
/// Dream consolidation for HDC pattern store.
pub fn dream_consolidation(store: &mut PatternStore) {
    // NREM: replay high-value patterns
    for pattern in store.high_value_patterns() {
        let updated_reliability = pattern.recompute_reliability();
        pattern.reliability = updated_reliability;
    }

    // REM: generate novel compositions
    let existing: Vec<&HdcVector> = store.all_vectors().collect();
    for _ in 0..10 {
        let a = existing.choose(&mut rng).unwrap();
        let b = existing.choose(&mut rng).unwrap();
        let novel = a.xor(b).permute(1);  // recombine + shift
        store.add_hypothetical(novel, confidence: 0.2);
    }

    // Prune: remove unreliable patterns
    store.prune(|p| p.frequency >= 10 && p.reliability < 0.3);
}
```

---

## Implementation details

### Codebook generation algorithm

Codebooks are generated deterministically from a domain-specific seed. This ensures that all agents sharing a seed share the same vector space, enabling direct cross-agent pattern comparison without alignment.

```rust
/// Generate a domain-specific HDC codebook deterministically.
///
/// The seed derives from the domain name via SHA-256. Each role vector
/// is drawn from the resulting CSPRNG stream. Because the seed is
/// deterministic, every agent in the same domain produces identical
/// codebooks without coordination.
pub struct CodebookGenerator {
    /// Domain seed (SHA-256 of domain name).
    seed: [u8; 32],
    /// Dimensionality of generated vectors (default: 10_240).
    dim: usize,
}

impl CodebookGenerator {
    pub fn new(domain: &str, dim: usize) -> Self {
        let seed = sha256(domain.as_bytes());
        Self { seed, dim }
    }

    /// Generate a role vector at a given index.
    ///
    /// Uses ChaCha20 seeded from `self.seed ++ index.to_le_bytes()`.
    /// Each bit is drawn with P(1) = 0.5 (dense binary).
    pub fn generate_role(&self, index: u32) -> HdcVector {
        let mut key = self.seed.to_vec();
        key.extend_from_slice(&index.to_le_bytes());
        let mut rng = ChaCha20Rng::from_seed(sha256(&key));
        HdcVector::random(&mut rng, self.dim)
    }

    /// Generate a QuantizedCodebook for a value range.
    ///
    /// Level vectors use thermometer construction:
    ///   level_0 = random base vector
    ///   level_k = level_{k-1} with `flip_count` random bits flipped
    ///
    /// `flip_count = dim / (2 * n_levels)` ensures adjacent levels
    /// have Hamming similarity ~= 1 - 1/(2*n_levels).
    pub fn generate_quantized(
        &self,
        codebook_index: u32,
        n_levels: usize,
        min: f64,
        max: f64,
    ) -> QuantizedCodebook {
        let flip_count = self.dim / (2 * n_levels);
        let base = self.generate_role(codebook_index);
        let mut levels = vec![base];

        for k in 1..n_levels {
            let prev = &levels[k - 1];
            let mut rng = ChaCha20Rng::from_seed(
                sha256(&[&self.seed[..], &(codebook_index + k as u32).to_le_bytes()].concat())
            );
            let flipped = prev.flip_random_bits(flip_count, &mut rng);
            levels.push(flipped);
        }

        QuantizedCodebook { levels, min, max, n_levels }
    }
}
```

**Configuration parameters**:

| Parameter | Default | Range | Notes |
|---|---|---|---|
| `dim` | 10,240 | 1,024 - 65,536 | Must be multiple of 64 for SIMD alignment. 10,240 = 160 u64 words. |
| `n_levels` (QuantizedCodebook) | 64 | 8 - 256 | More levels = finer granularity, more memory. 64 gives ~1.5% resolution. |
| `flip_count` | `dim / (2 * n_levels)` | derived | Controls similarity between adjacent levels. |

### QuantizedCodebook::encode() interpolation

The `encode()` method interpolates between adjacent level vectors via a weighted bundle. The procedure:

1. Normalize the input value to `[0.0, 1.0]` within the codebook's range.
2. Map to a fractional level index: `level_f = normalized * (n_levels - 1)`.
3. Identify the two bracketing levels: `lower = floor(level_f)`, `upper = lower + 1`.
4. Compute interpolation weight: `w = level_f - lower`.
5. Return `weighted_bundle(levels[lower], levels[upper], 1.0 - w, w)`.

The weighted bundle for two vectors uses probabilistic bit selection: for each bit position, select from `levels[upper]` with probability `w`, else from `levels[lower]`. This produces a vector whose Hamming similarity to each level is proportional to the interpolation weight.

**Error handling**: Values outside `[min, max]` are clamped. If `n_levels` is 1, return the single level vector regardless of input. If the codebook is empty (zero levels), return a zero vector and log a warning.

### Pattern store serialization (CBOR)

The PatternStore serializes to CBOR (RFC 8949) for compact, schema-flexible persistence:

```rust
/// CBOR schema for PatternStore persistence.
///
/// Top-level: CBOR map {
///   "version": u32,              // schema version (currently 1)
///   "domains": map {             // keyed by OracleDomain string
///     "<domain>": array [        // array of StoredPattern
///       {
///         "v": bytes(1280),      // HDC vector (10,240 bits = 1,280 bytes)
///         "src": bytes(32),      // source engram ContentHash
///         "out": ?i8,            // outcome: -1 (loss), 0 (neutral), 1 (profit), null
///         "freq": u64,           // observation count
///         "rel": f32,            // reliability [0.0, 1.0]
///       },
///       ...
///     ]
///   },
///   "cross_index": array [       // cross-domain index entries
///     { "d": string, "v": bytes(1280), "h": bytes(32) },
///     ...
///   ]
/// }
///
/// File size estimate: 1,280 bytes per pattern + 45 bytes metadata.
/// 100K patterns ~= 130 MB. 1M patterns ~= 1.3 GB.
pub fn serialize_pattern_store(store: &PatternStore) -> Vec<u8> {
    let mut encoder = CborEncoder::new();
    encoder.map(3);
    encoder.text("version").unsigned(1);
    encoder.text("domains");
    encoder.map(store.patterns.len());
    for (domain, patterns) in &store.patterns {
        encoder.text(&domain.to_string());
        encoder.array(patterns.len());
        for p in patterns {
            encoder.map(5);
            encoder.text("v").bytes(&p.vector.as_bytes());
            encoder.text("src").bytes(&p.source_engram.as_bytes());
            encoder.text("out").optional_i8(p.outcome.map(|o| o as i8));
            encoder.text("freq").unsigned(p.frequency);
            encoder.text("rel").float32(p.reliability as f32);
        }
    }
    // ... cross_index similarly
    encoder.finish()
}
```

### Similarity threshold calibration

The default threshold of 0.526 derives from the information-theoretic properties of 10,240-bit BSC vectors. Calibrate it in practice with this procedure:

1. **Generate null distribution**: Create 10,000 random vector pairs. Compute their Hamming similarities. The distribution should be approximately Gaussian with mean 0.500 and stddev ~0.00494.
2. **Choose significance level**: The default 0.526 corresponds to 5.26 sigma (p < 1e-7). For applications tolerating more false positives, use 0.515 (3 sigma, p < 0.0013).
3. **Validate on held-out data**: Take known-similar pattern pairs from the domain. Compute their similarity distribution. The threshold should separate the null distribution from the true-positive distribution with <1% overlap.
4. **Adjust per domain**: If a domain has noisier encodings (fewer role-filler pairs per pattern), increase the threshold. Rule of thumb: add 0.005 per missing role-filler pair below 5.

```rust
/// Calibrate similarity threshold for a given vector dimensionality.
///
/// Returns (mean, stddev, suggested_threshold) based on the null distribution.
pub fn calibrate_threshold(dim: usize, sigma_level: f64, n_samples: usize) -> (f64, f64, f64) {
    let mut rng = thread_rng();
    let mut similarities = Vec::with_capacity(n_samples);
    for _ in 0..n_samples {
        let a = HdcVector::random(&mut rng, dim);
        let b = HdcVector::random(&mut rng, dim);
        similarities.push(a.hamming_similarity(&b));
    }
    let mean = similarities.iter().sum::<f64>() / n_samples as f64;
    let variance = similarities.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / n_samples as f64;
    let stddev = variance.sqrt();
    (mean, stddev, mean + sigma_level * stddev)
}
```

### Cross-domain routing protocol

When oracles from different domains want to exchange patterns, the routing protocol works as follows:

```
1. Source oracle encodes a pattern using its domain codebook.
2. Source sends (domain_id, pattern_hv, metadata) to the PatternStore.
3. PatternStore inserts the pattern into the cross_domain_index.
4. Any oracle can query the cross_domain_index with a pattern vector.
5. Matches above threshold are returned with their source domain.
6. The querying oracle decides whether to incorporate the cross-domain match.
```

No codebook translation is needed because all codebooks share the same vector space (10,240-bit BSC with XOR bind and majority bundle). The cross-domain similarity is structural, not lexical.

**Integration wiring**: `PatternStore::find_similar()` with `domain: None` searches the cross-domain index. The oracle calls this during its Theta-frequency analysis pass.

### Pruning rules

When the pattern count exceeds memory limits, prune according to these rules (applied in order):

1. **Unreliable patterns**: Remove patterns where `reliability < 0.3` and `frequency >= 10`. These have enough observations to confirm they do not predict well.
2. **Stale patterns**: Remove patterns not matched in the last `max_staleness` duration (default: 72 hours). These are no longer relevant to current market/code conditions.
3. **Redundant patterns**: For patterns with Hamming similarity > 0.95 to each other, keep only the one with higher reliability. This deduplicates near-identical encodings.
4. **LRU eviction**: If the store still exceeds `max_patterns`, remove the least-recently-matched patterns until within budget.

```rust
/// Configuration for pattern store pruning.
pub struct PruneConfig {
    /// Maximum patterns per domain before pruning triggers.
    pub max_patterns_per_domain: usize,  // default: 100_000
    /// Minimum reliability to survive pruning (with sufficient observations).
    pub min_reliability: f64,            // default: 0.3
    /// Minimum observations before reliability-based pruning applies.
    pub min_frequency: u64,              // default: 10
    /// Maximum time since last match before staleness pruning.
    pub max_staleness: Duration,         // default: 72 hours
    /// Similarity threshold for deduplication.
    pub dedup_threshold: f64,            // default: 0.95
}
```

### Connection to Dreams: reliability updates

During Delta-frequency dream consolidation, the `StoredPattern.reliability` field is updated from dream outcomes:

```
1. Dreams NREM phase replays high-frequency patterns.
2. For each replayed pattern, Dreams evaluates: "If this pattern
   activated now, would the predicted outcome hold?"
3. The evaluation runs the pattern through the current causal model
   (from causal microstructure discovery) and compares the predicted
   outcome to the pattern's stored outcome.
4. If they agree: reliability += 0.05 (capped at 1.0).
5. If they disagree: reliability -= 0.10 (floored at 0.0).
6. During REM recombination, newly generated hypothetical patterns
   start with reliability = 0.2.
```

The asymmetric update (slower increase, faster decrease) follows the principle that trust is hard to earn and easy to lose. A pattern must consistently agree with the causal model across multiple dream cycles to reach high reliability.

### Test criteria

- **Codebook determinism**: Two `CodebookGenerator` instances with the same domain and dim produce identical role vectors.
- **Quantized encoding monotonicity**: For values v1 < v2, `hamming_similarity(encode(v1), encode(v2))` decreases as `|v2 - v1|` increases.
- **Threshold calibration**: The null distribution mean is within 0.001 of 0.500 for dim = 10,240.
- **Cross-domain routing**: A pattern stored by one oracle is retrievable by another oracle via `find_similar(domain: None)`.
- **CBOR round-trip**: `deserialize(serialize(store))` produces an identical PatternStore.
- **Pruning correctness**: After pruning, no pattern violates the configured thresholds.
- **Dream reliability update**: After 10 agreeing dream cycles, reliability reaches >= 0.7. After 5 disagreeing cycles from reliability 0.7, reliability drops to <= 0.2.

---

## Academic foundations

- Kanerva, P. (2009). "Hyperdimensional Computing: An Introduction." *Cognitive Computation*, 1(2), 139-159. — Binary Spatter Code (BSC) foundations.
- Kleyko, D., et al. (2022). "A Survey on Hyperdimensional Computing." *ACM Computing Surveys*, 54(6), 1-51. — Comprehensive HDC survey including performance benchmarks.
- Plate, T. A. (1995). "Holographic Reduced Representations." *IEEE Transactions on Neural Networks*, 6(3), 623-641. — Distributed representations for structured data.
- Frady, E. P., Kleyko, D., & Sommer, F. T. (2018). "A Theory of Sequence Indexing and Working Memory in Recurrent Neural Networks." *Neural Computation*, 30(6), 1449-1513. — Temporal encoding via permutation.
- Rachkovskij, D. A. (2001). "Representation and Processing of Structures with Binary Sparse Distributed Codes." *IEEE Transactions on Knowledge and Data Engineering*, 13(2), 261-276. — Sparse distributed memory for pattern matching.
- Lacaux, C., et al. (2021). "Sleep onset is a creative sweet spot." *Science Advances*, 7(50). — Creativity during N1 sleep, motivating hypnagogia-based pattern generation.

---

## Cross-references

- See [06-neuro](../06-neuro/INDEX.md) for HDC fundamentals and the Neuro knowledge store
- See [08-adaptive-signal-metabolism.md](./08-adaptive-signal-metabolism.md) for evolutionary dynamics of HDC-encoded signals
- See [10-predictive-geometry-and-resonant-patterns.md](./10-predictive-geometry-and-resonant-patterns.md) for TDA + HDC pattern composition
- See [12-somatic-ta-and-emergent-multiscale.md](./12-somatic-ta-and-emergent-multiscale.md) for somatic markers as HDC bindings
