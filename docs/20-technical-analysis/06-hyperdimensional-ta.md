# Hyperdimensional Technical Analysis

> HDC encodes TA patterns as 10,240-bit vectors. Pattern algebra (bind, bundle, permute) enables nanosecond cross-domain similarity search, temporal composition, and shift-invariant pattern matching.

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
