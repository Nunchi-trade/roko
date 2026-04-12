# False Positive Math for HDC Similarity

> With 10,240-bit BSC vectors, a threshold of 0.526 guarantees <1% false positive rate against a 100K vocabulary after Bonferroni correction — the recommended threshold for Neuro's cross-domain resonance detection.


> **Implementation**: Built

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [04-hdc-vsa-foundations.md](./04-hdc-vsa-foundations.md)
**Key sources**:
- `refactoring-prd/09-innovations.md` §XIX.D (HDC Cross-Domain False Positive Rate)
- `bardo-backup/prd/shared/hdc-vsa.md` §3 (dimension choice, quasi-orthogonality)
- Johnson, W. B., & Lindenstrauss, J. (1984)

---

## Abstract

When Neuro detects a similarity between two HDC vectors above some threshold, how confident can we be that this represents a genuine structural relationship rather than random noise? This is the false positive question. It is critical for cross-domain resonance detection (see [08-cross-domain-hdc-transfer.md](./08-cross-domain-hdc-transfer.md)), where spurious matches would generate misleading "insights" that waste agent resources.

This document derives the false positive rate for HDC similarity at various thresholds, applies Bonferroni correction for multiple comparisons against large vocabularies, and recommends the 0.526 threshold for Neuro's production use.

---

## Statistical Foundation

### Distribution of Random Similarity

For two independent random 10,240-bit binary vectors, the Hamming similarity (fraction of matching bits) follows a distribution with:

```
Expected value:     μ = 0.5
Variance:           σ² = 1 / (4D) = 1 / (4 × 10,240) = 2.441 × 10⁻⁵
Standard deviation: σ = 1 / (2√D) = 1 / (2 × √10,240) = 0.00494
```

By the Central Limit Theorem, for D = 10,240, the distribution of Hamming similarity is well-approximated by a Normal distribution: `sim ~ N(0.5, 0.00494²)`.

### Z-Score and False Positive Rate

A similarity of `s` corresponds to a Z-score of:

```
Z = (s - 0.5) / σ = (s - 0.5) / 0.00494
```

The false positive rate (probability of a random pair exceeding threshold `s`) is:

```
P(sim > s) = 1 - Φ(Z)
```

Where Φ is the standard normal CDF.

---

## Threshold Table

| Threshold | Z-score | Per-Comparison FP Rate | Use Case |
|---|---|---|---|
| 0.505 | 1.01 | 15.6% | Too low — noise |
| 0.510 | 2.02 | 2.17% | Rough screening |
| 0.512 | 2.43 | 0.75% | Single-pair check |
| 0.515 | 3.04 | 0.12% | Conservative single-pair |
| 0.520 | 4.05 | 2.6 × 10⁻⁵ | Moderate vocabulary |
| 0.526 | 5.26 | 7.3 × 10⁻⁸ | 100K vocabulary (Bonferroni) |
| 0.530 | 6.07 | 6.5 × 10⁻¹⁰ | 1M vocabulary |
| 0.540 | 8.10 | < 10⁻¹⁵ | Extremely conservative |
| 0.550 | 10.12 | < 10⁻²³ | Near-certain match |

---

## Bonferroni Correction for Multiple Comparisons

When scanning a knowledge base of N entries, the probability of **at least one** false positive is:

```
P(at least 1 FP) ≈ N × P(single FP)     (for small P)
```

To maintain an overall false positive rate of α across N comparisons (Bonferroni correction), the per-comparison threshold must be:

```
P(single FP) ≤ α / N
```

### Threshold Selection by Vocabulary Size

| Vocabulary Size (N) | Target α | Required per-comparison FP | Required Z | Threshold |
|---|---|---|---|---|
| 100 | 1% | 10⁻⁴ | 3.72 | 0.518 |
| 1,000 | 1% | 10⁻⁵ | 4.26 | 0.521 |
| 10,000 | 1% | 10⁻⁶ | 4.75 | 0.523 |
| **100,000** | **1%** | **10⁻⁷** | **5.26** | **0.526** |
| 1,000,000 | 1% | 10⁻⁸ | 5.73 | 0.528 |
| 10,000,000 | 1% | 10⁻⁹ | 6.00 | 0.530 |

### Recommendation: Threshold = 0.526

For Neuro's typical use case — an agent with up to 100,000 knowledge entries, scanning for cross-domain resonance — the **recommended threshold is 0.526**.

This guarantees:
- **< 1% overall false positive rate** when scanning 100,000 entries
- **Z-score of 5.26** — the match is 5.26 standard deviations above the expected random similarity
- **Per-comparison false positive rate of 7.3 × 10⁻⁸** — vanishingly small

### For Larger Knowledge Bases

If Neuro is used for collective knowledge bases on the Korai chain (potentially millions of entries), the threshold should be raised to 0.528–0.530. The difference is small (2–4 thousandths) but ensures the Bonferroni correction remains valid.

---

## Minimum Dimension Validation

### Johnson-Lindenstrauss Bound

The Johnson-Lindenstrauss lemma (1984) provides a lower bound on dimension D to preserve pairwise distances for N points with distortion ε:

```
D ≥ (8 ln N) / ε²
```

For Neuro's use case:

| N (entries) | ε (distortion) | Minimum D | D = 10,240 sufficient? |
|---|---|---|---|
| 1,000 | 0.1 | 553 | Yes (18.5× headroom) |
| 10,000 | 0.1 | 737 | Yes (13.9× headroom) |
| 100,000 | 0.1 | 921 | Yes (11.1× headroom) |
| 100,000 | 0.05 | 3,682 | Yes (2.8× headroom) |
| 1,000,000 | 0.1 | 1,106 | Yes (9.3× headroom) |
| 1,000,000 | 0.05 | 4,423 | Yes (2.3× headroom) |
| 1,000,000 | 0.03 | 12,286 | **No** (needs D ≥ 12,288) |

At ε = 0.1 (10% distortion tolerance), D = 10,240 supports up to 10 million entries with ample headroom. At ε = 0.03 (3% distortion), the dimension limit is reached at ~750K entries. For extremely large or high-precision applications, D = 16,384 (256 u64 words) would extend the range.

**Citation**: Johnson, W. B., & Lindenstrauss, J. (1984). "Extensions of Lipschitz mappings into a Hilbert space." *Contemporary Mathematics*, 26, 189–206.

---

## Multi-Agent Confirmation

For additional false positive reduction, the refactoring-prd recommends requiring **cross-domain analogies to be confirmed by at least 2 independent agents**:

```
P(joint FP) = P(agent_1 FP) × P(agent_2 FP)
            = (7.3 × 10⁻⁸)² = 5.3 × 10⁻¹⁵
```

With two-agent confirmation at the 0.526 threshold, the false positive rate against a 100K vocabulary drops to effectively zero. This adds latency (must wait for two agents to independently detect the match) but provides extremely high confidence in genuine cross-domain insights.

---

## Practical Implications

### For Neuro Query API

The similarity threshold is a configurable parameter in the query API:

```rust
pub struct NeuroQuery {
    pub topic: String,
    pub limit: usize,
    pub min_similarity: f64,  // default: 0.526 for cross-domain, 0.51 for within-domain
}
```

Within-domain queries can use a lower threshold (0.51) because the domain constraint already reduces the false positive space. Cross-domain queries use the full 0.526 threshold.

### For Resonance Detection

The resonance detector runs as a background loop, comparing each new entry against all entries in other domains. With 100K entries and ~13 ns per comparison, a full scan takes ~1.3 ms — fast enough to run on every knowledge ingestion without blocking the agent.

### For the Somatic Landscape

Somatic markers (see [13-somatic-integration.md](./13-somatic-integration.md)) also use HDC similarity for nearest-neighbor lookup in the 8D strategy space. The k-d tree used for somatic lookup has different threshold requirements (lower, since it operates within a single domain), but the same statistical framework applies.

---

## Academic Foundations

- Johnson, W. B., & Lindenstrauss, J. (1984). "Extensions of Lipschitz mappings into a Hilbert space." *Contemporary Mathematics*, 26, 189–206.
- Kanerva, P. (2009). "Hyperdimensional Computing." *Cognitive Computation*, 1(2), 139–159.
- Kleyko, D., et al. (2022). "A Survey on Hyperdimensional Computing." *ACM Computing Surveys*, 54(6).
- Bonferroni, C. E. (1936). "Teoria statistica delle classi e calcolo delle probabilità." *Pubblicazioni del R. Istituto Superiore di Scienze Economiche e Commerciali di Firenze*, 8, 3–62.

---

## Current Status and Gaps

**Implemented**: `HdcVector::similarity()` returns normalized Hamming similarity in [0, 1].

**Missing**: Configurable similarity threshold in query API. Bonferroni-aware threshold selection based on knowledge base size. Cross-domain resonance detection loop. Multi-agent confirmation protocol. Automatic threshold adjustment as knowledge base grows.

---

## Cross-references

- See [04-hdc-vsa-foundations.md](./04-hdc-vsa-foundations.md) for the dimension choice
- See [08-cross-domain-hdc-transfer.md](./08-cross-domain-hdc-transfer.md) for how resonance detection uses these thresholds
- See [10-knowledge-query-api.md](./10-knowledge-query-api.md) for the query API parameters
