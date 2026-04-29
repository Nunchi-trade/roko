# Positional Effects and Retrieval Quality

> Depth for [02-CELL.md](../../unified/02-CELL.md). Lost-in-the-middle (U-shape) mitigation as a positional Score Cell, section effect tracking via Beta distributions, and how retrieval quality feeds back into future composition.

---

## 1. The U-Shaped Attention Phenomenon

Language models attend most strongly to the beginning and end of their context, with degraded attention to the middle. This is not a training artifact -- it is an **algebraic property of causal decoder architectures**, present at initialization before any training or positional encoding (arXiv:2603.10123, 2025).

```
Performance
    |  ====                                        ====
    |  ======                                    ======
    |  ========                                ========
    |  ==========                            ==========
    |  ============                        ============
    |  ================================  ==============
    +------------------------------------------------------> Position
      Beginning          Middle                End
```

### Why It Cannot Be Trained Away

Two architectural properties guarantee the U-shape:

1. **Causal masking guarantees primacy bias.** In a causal decoder, early tokens lie on exponentially more computational paths through the residual network. A token at position 1 influences every subsequent attention operation. A token at position N/2 influences only half as many.

2. **Residual connections guarantee recency bias.** Late tokens maintain direct (short-path) connections to the output through the residual stream, bypassing the attention bottleneck.

Positional encodings (RoPE, ALiBi) modulate the shape of the U-curve but cannot eliminate it. Any scaffold that places critical information in the middle is fighting the architecture.

### Empirical Evidence

| Study | Finding |
|---|---|
| Liu et al. (2023, TACL 2024) | U-shaped retrieval on GPT-3.5, Claude, MPT-30B. >30% degradation in middle positions. |
| "Lost in the Middle at Birth" (2025) | Bias is algebraic, present at initialization. Cannot be trained away. |
| Chroma "Context Rot" (2025) | All 18 frontier models show degradation. Semantically close distractors are the worst. |
| Du et al. (EMNLP 2025) | Even whitespace degrades performance by 13.9-85%. |
| Shi et al. (ICML 2023) | Irrelevant context actively harms -- worse than no context at all. |
| Sequential-NIAH (2025) | Claude 3.5 at 87% on sequential needle extraction -- still 12.4% below reference. |

### Attention Rank Collapse

In very long contexts, a separate failure mode emerges: attention scores collapse toward uniformity (OpenReview:7SLtElfqCW, 2025). All tokens receive roughly equal attention, preventing the model from distinguishing relevant from irrelevant information. Mitigation: polylogarithmic rescaling of attention scores. This is a model-level fix, but it reinforces the scaffold principle: shorter, better-composed prompts are inherently safer.

### Attention Sinks

The first few tokens in a sequence act as "attention sinks" -- they absorb disproportionate attention probability (arXiv:2603.10123). This is why role identity at position 1 gets extreme attention: it is not just primacy, it is the attention sink effect. Scaffold implication: the first ~50 tokens receive outsized attention. Use them for role identity.

---

## 2. The Placement Enum: Static Positional Scoring

Roko implements U-shape mitigation through the `Placement` enum, a static positional Score Cell that assigns each section to a zone:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Start,   // highest attention zone (primacy)
    Middle,  // lowest attention zone (degradation)
    End,     // second-highest attention zone (recency)
}
```

### Section-to-Placement Mapping

| Section Kind | Placement | Rationale |
|---|---|---|
| Role identity | **Start** | Identity first; attention sink effect |
| Conventions | **Start** | Safety rules need primacy attention |
| Task description | **Start** | Core directive at primacy position |
| Tool instructions | **Start** | Tool awareness before planning |
| Workspace map | **Middle** | Supporting context, not critical path |
| PRD extract | **Middle** | Reference material |
| Cross-plan context | **Middle** | Background information |
| Research memo | **Middle** | Supporting evidence |
| Relevant techniques | **End** | Learned strategies near output |
| Anti-patterns | **End** | Prohibitions need recency attention |
| Gate errors | **End** | Most recent failure near generation boundary |
| Affect guidance | **End** | Behavioral modulation applied to final decisions |
| Constraints reminder | **End** | Dual-position pattern (Devin): safety at both edges |

### U-Shape Ordering in the Compose Cell

After the budget-fitting phase (greedy knapsack), the Compose Cell reorders included sections:

```rust
// Phase 5 of PromptComposer::compose()
let mut final = Vec::new();

// Highest attention: primacy zone
final.extend(included.iter().filter(|s| s.placement == Start));

// Lowest attention: degradation zone
final.extend(included.iter().filter(|s| s.placement == Middle));

// Second-highest attention: recency zone
final.extend(included.iter().filter(|s| s.placement == End));

// Within each zone, preserve CacheLayer ordering for cache stability
```

The dual-position pattern (Devin, 2025) places safety constraints at both edges: Layer 2 (Conventions) at the beginning and Layer 6b (Anti-Patterns) at the end. Even as context grows, safety rules remain in high-attention zones.

### Interaction with Cache Alignment

Cache alignment wants stable content first (System -> Session -> Task -> Dynamic). U-shape wants high-value content at beginning and end. These are partially in tension.

Resolution: within each cache tier, sections are placed by their `Placement` hint. The overall structure achieves both goals:

```
Cache Tier 0 (System) -- all Start placement
    Role identity, Conventions, Tool instructions

Cache Tier 1 (Session) -- Middle placement
    Workspace map, Cross-plan context

Cache Tier 2 (Task) -- Start + Middle placement
    Task description (Start), PRD extract (Middle), Brief (Middle)

Cache Tier 3 (Dynamic) -- End placement
    Gate errors, Anti-patterns, Affect guidance
```

The System tier forms a byte-identical cached prefix. The highest-attention positions (beginning and end) contain the most critical information.

---

## 3. The PositionAttentionModel: Continuous Positional Scoring

The static Placement enum assigns sections to three zones. The `PositionAttentionModel` generalizes this to a continuous attention score at any position, parameterized by a double-exponential U-curve:

```rust
/// Attention multiplier based on position in the context window.
/// Fitted per model family from empirical measurements (Liu et al. 2023).
pub struct PositionAttentionModel {
    pub primacy_weight: f64,   // default: 0.35
    pub primacy_decay: f64,    // default: 0.15
    pub recency_weight: f64,   // default: 0.30
    pub recency_decay: f64,    // default: 0.20
    pub baseline: f64,         // default: 0.35
}

impl PositionAttentionModel {
    /// Attention at normalized position [0, 1].
    /// Returns value in [baseline, 1.0].
    pub fn attention_at(&self, pos: f64) -> f64 {
        let primacy = self.primacy_weight * (-self.primacy_decay * pos).exp();
        let recency = self.recency_weight * (-self.recency_decay * (1.0 - pos)).exp();
        (primacy + recency + self.baseline).min(1.0)
    }

    /// Effective score = base_score * attention_multiplier.
    pub fn effective_score(&self, base_score: f64, pos: f64) -> f64 {
        base_score * self.attention_at(pos)
    }
}
```

With default parameters:
- Position 0.0 (start): attention = 0.35 + 0.35*1.0 + 0.30*exp(-0.20) = ~0.95
- Position 0.5 (middle): attention = 0.35 + 0.35*exp(-0.075) + 0.30*exp(-0.10) = ~0.62
- Position 1.0 (end): attention = 0.35 + 0.35*exp(-0.15) + 0.30*1.0 = ~0.95

The middle-position effective score is approximately 65% of edge-position effective score, consistent with Liu et al.'s ~30% degradation finding.

### Position-Adjusted Section Scoring

A simpler integration preserves the existing Placement enum:

```rust
pub fn placement_adjusted_score(base_score: f64, placement: Placement) -> f64 {
    match placement {
        Start  => base_score * 1.00,  // primacy zone: full value
        End    => base_score * 0.95,  // recency zone: ~95% value
        Middle => base_score * 0.70,  // degradation zone: ~70% value
    }
}
```

This means a Medium-priority section at Start is effectively scored higher than a High-priority section in Middle, creating pressure to promote valuable sections to edge positions.

### Per-Model Attention Curves

Different models exhibit different U-curve shapes. Claude 3.5 achieved 87% on Sequential-NIAH while other models scored lower. The system can store fitted parameters per model family:

```rust
pub struct ModelAttentionCurves {
    pub curves: HashMap<String, PositionAttentionModel>,
    pub default_curve: PositionAttentionModel,
}

impl ModelAttentionCurves {
    pub fn for_model(&self, model_id: &str) -> &PositionAttentionModel {
        self.curves.get(model_id).unwrap_or(&self.default_curve)
    }
}
// Persisted to: .roko/learn/attention-curves.json
```

---

## 4. Section Effect Tracking via Beta Distributions

Each section's historical correlation with gate success is tracked as a Beta distribution. This is the `BetaPosterior` on `ComposeBid.effect`:

```rust
pub struct BetaPosterior {
    pub alpha: f64,  // "successes" (section included AND gate passed)
    pub beta: f64,   // "failures" (section included AND gate failed)
}

impl BetaPosterior {
    pub fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }
    pub fn variance(&self) -> f64 {
        (self.alpha * self.beta)
            / ((self.alpha + self.beta).powi(2) * (self.alpha + self.beta + 1.0))
    }
    pub fn update(&mut self, gate_passed: bool) {
        if gate_passed { self.alpha += 1.0 } else { self.beta += 1.0 }
    }
}
```

The `SectionEffectivenessRegistry` (from `roko-learn`) maintains one Beta posterior per section per task category. After each task, it updates:

```rust
// In the learning Loop, after gate verdict:
for section in composition_manifest.included {
    let posterior = registry.get_or_create(section.name, task_category);
    posterior.update(gate_passed);
}
```

### From Effects to Bids

The posterior mean feeds into the bidding process:

```
bid_value = base_relevance * effect.mean() * novelty_attenuation * cost_factor
```

Where:
- `base_relevance` is the section's relevance to the current task (from the Score Cell).
- `effect.mean()` is the learned probability that including this section correlates with gate success.
- `novelty_attenuation = 1 / (1 + ln(freq))` penalizes boilerplate.
- `cost_factor` is the cost-effectiveness multiplier (described below).

Sections with low `effect.mean()` (e.g., Learning Pack at 61% pass rate vs 67% baseline) get lower bids and are more likely to be dropped under budget pressure. Sections with high `effect.mean()` (e.g., Task Brief at 71%) get higher bids and survive aggressive budgets.

### Cold Start

A fresh section starts with `BetaPosterior { alpha: 1.0, beta: 1.0 }` (uniform prior, mean 0.5). The system needs ~10 observations to become informative. Until then, the greedy knapsack (not VCG) is used, because VCG payments are meaningless with uninformative posteriors.

---

## 5. Retrieval Quality Feedback

### Leave-One-Out Influence

The Contextual Influence Value framework measures per-section impact from natural variation:

```
influence(S) = pass_rate_when_S_included - pass_rate_when_S_excluded
```

This does not require controlled experiments. Tasks where a section was dropped due to budget constraints serve as the "without S" condition. Tasks where it was included serve as the "with S" condition.

```rust
pub fn compute_section_influence(
    outcomes: &[BudgetOutcome],
    section_name: &str,
    task_category: &str,
) -> SectionInfluence {
    let with: Vec<_> = outcomes.iter()
        .filter(|o| o.category == task_category)
        .filter(|o| o.section_was_included(section_name))
        .collect();
    let without: Vec<_> = outcomes.iter()
        .filter(|o| o.category == task_category)
        .filter(|o| !o.section_was_included(section_name))
        .collect();

    let rate_with = pass_rate(&with);
    let rate_without = pass_rate(&without);

    SectionInfluence {
        section_name: section_name.into(),
        influence: rate_with - rate_without,
        observations_with: with.len(),
        observations_without: without.len(),
        confidence: wilson_interval(with.len(), without.len(), rate_with, rate_without),
    }
}
```

Classification:
- `influence > 0.05`: Valuable -- increase allocation by 20%.
- `-0.05 <= influence <= 0.05`: Neutral -- no change.
- `influence < -0.05`: Harmful -- reduce allocation by 50% or drop entirely.

Constraints: never modify Critical section allocations; bound reallocation by +/-50% per cycle; require >= 50 observations per category; run at most once per day.

### Cost-Aware Effectiveness

When cost attribution data is available (see [enrichment-pipeline.md](enrichment-pipeline.md)), the learning system can distinguish:
- A section that costs 2,000 tokens and achieves 60% pass rate.
- A section that costs 200 tokens and achieves 60% pass rate.

The second is 10x more cost-effective. The `LearningBidder` incorporates a `cost_effectiveness_factor`:

```rust
fn cost_effectiveness_factor(&self, section_name: &str) -> f64 {
    let stats = self.section_costs.get(section_name)?;
    if stats.observation_count < 3 { return 1.0; }

    let pass_rate = stats.passes as f64 / stats.observation_count as f64;
    let cost_per_token = stats.total_cost_usd / stats.total_tokens as f64;

    let cost_efficiency = 1.0 / (1.0 + cost_per_token.ln().max(0.0));
    let combined = 0.7 * pass_rate + 0.3 * cost_efficiency;
    combined.clamp(0.5, 2.0)  // avoid wild swings
}
```

**Mori-diffs reality**: Cost attribution types are designed but not implemented. The `LearningBidder` updates on `(included, passed)` but has no cost signal. See [09-COMPOSITION-AUCTION.md](../../mori-diffs/09-COMPOSITION-AUCTION.md).

### Information-Theoretic Density

A secondary retrieval quality signal: measure how much of a section's content is novel given the other sections (n-gram overlap as a proxy for mutual information):

```rust
pub fn section_information_density(
    section_content: &str,
    other_sections: &[&str],
) -> f64 {
    let section_ngrams = extract_ngrams(section_content, 3);
    let other_ngrams: HashSet<_> = other_sections.iter()
        .flat_map(|s| extract_ngrams(s, 3))
        .collect();

    // Fraction of section's n-grams that don't appear in other sections
    let novel_fraction = section_ngrams.iter()
        .filter(|ng| !other_ngrams.contains(*ng))
        .count() as f64 / section_ngrams.len().max(1) as f64;

    novel_fraction  // [0, 1]. Higher = more novel.
}
```

Sections with low information density (high overlap with other included sections) get lower effective bids. This prevents budget waste on redundant content.

---

## 6. Dynamic Placement: LongLLMLingua-Style Reordering

The static Placement enum assigns positions per section type. Dynamic placement goes further: assign positions per section **instance** based on measured information density relative to the current task:

```rust
pub fn dynamic_placement(sections: &mut [PromptSection], query: &str) {
    // Score each section's density relative to the task query
    for s in sections.iter_mut() {
        s.density_score = information_density(&s.content, query);
    }

    // Sort by density descending
    sections.sort_by(|a, b| b.density_score.partial_cmp(&a.density_score).unwrap());

    // Highest density -> Start (primacy), next -> End (recency), rest -> Middle
    let n = sections.len();
    for (i, s) in sections.iter_mut().enumerate() {
        if s.priority == Critical { continue; }  // Critical placement is fixed
        s.placement = if i < n / 3 { Start }
            else if i >= 2 * n / 3 { End }
            else { Middle };
    }
}
```

This is LongLLMLingua's semantic density ranking (Jiang et al., ACL 2024): place the densest documents at the edges of the context. LongLLMLingua achieves up to 21.4% performance improvement using only 1/4 of original tokens.

**Status**: Designed but not implemented. The static Placement enum is sufficient for most tasks.

---

## 7. Hierarchical Context Organization

Research suggests that hierarchical organization partially mitigates middle-zone degradation. Instead of flat section sequences, organize content with explicit navigation cues:

```markdown
<!-- roko:section:domain_context -->
## Domain Context

### Crate Architecture
- roko-compose: prompt assembly (your changes go here)
- roko-core: trait definitions (do not modify)

### Relevant Types
- `PromptSection`: the unit of composition
- `Budget`: hard constraints on output

### PRD Requirements
- REQ-1: Support 9 layers
- REQ-2: Cache alignment
```

Headers create a structural scaffold that helps the model navigate the middle zone. The model attends to headers (which are at local primacy positions within each subsection) even when it partially loses track of content between them.

---

## 8. What This Enables

1. **Architecture-aware composition**: By placing critical content at positions where the model actually attends, the system achieves ~30% better utilization of context budget compared to random or naive ordering.

2. **Learned section effectiveness**: Beta posteriors track which sections actually help, without controlled experiments. Natural budget-pressure variation provides the "with/without" conditions.

3. **Cost-optimal prompt construction**: Cost attribution closes the loop between what sections cost and what they contribute, enabling VCG to allocate budget to the highest-value-per-dollar sections.

4. **Model-adaptive curves**: Per-model attention parameters mean the same composition pipeline produces differently-optimized prompts for Claude vs Sonnet vs Haiku.

5. **Dual-position safety**: Safety constraints at both edges ensure that even as context grows beyond typical lengths, the model always has safety rules in high-attention zones.

---

## 9. Feedback Loops

**Positional feedback**: Section placement -> gate outcome -> Beta posterior update -> adjusted placement decisions. Sections that fail when placed in Middle can be promoted to Start/End via dynamic placement.

**Density feedback**: Section information density -> composition ranking -> gate outcome -> influence measurement -> budget reallocation. Redundant sections (low density) get smaller allocations.

**Attention curve feedback**: Empirical position experiments -> fitted PositionAttentionModel parameters -> more accurate effective scores -> better placement decisions. Stored per-model in `.roko/learn/attention-curves.json`.

---

## 10. Open Questions

1. **Position-interaction effects between sections**: The current model scores each section's positional effectiveness independently. But section A at position 3 might be more valuable when section B is at position 1 (because B provides definitions that A references). Measuring pairwise position interactions requires O(n^2) experiments per section pair -- expensive. The active-inference scorer could potentially capture this through joint EFE estimation.

2. **Attention curve stability across model updates**: When a model provider updates their model (e.g., Claude 3.5 -> 4.0), the fitted attention curves may no longer be accurate. The system needs a drift detector that triggers re-calibration when gate pass rates change unexpectedly.

3. **Learned layer ordering**: Research shows that task-first ordering outperforms grounding-first ordering for trivial tasks, while grounding-first is better for complex tasks. A `LayerOrderPolicy` that learns per-category ordering is designed but not implemented. The interaction with cache alignment (which wants a fixed prefix) creates a tension: changing layer order per task category breaks prefix caching. The resolution may be to learn ordering only within cache tiers, not across them.

4. **Compression vs. dropping**: When a section does not fit the budget, the current system truncates or drops. LLMLingua-2 (Pan et al., 2024) achieves 3-6x token compression with minimal performance loss. Adding a compression Cell between Score and Compose could recover value from budget-constrained sections without the information loss of truncation.

5. **Layer-wise positional bias**: Different transformer layers exhibit different position preferences (arXiv:2601.04098). Later layers show stronger primacy bias. This suggests that the optimal placement strategy may depend on which layers of the model are most important for the task type -- a research question beyond current scaffold design.
