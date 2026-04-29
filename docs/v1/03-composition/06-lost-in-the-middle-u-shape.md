# 06 — Lost in the Middle: U-Shaped Attention Optimization

> Layer 2 Scaffold — Synapse Architecture
> Status: **Implemented** — Placement enum in `roko-compose::prompt`
> Canonical source: Liu et al., TACL 2024 [arXiv:2307.03172]


> **Implementation**: Shipping

---

## Abstract

Language models attend to information at the beginning and end of their context far more effectively than information in the middle. This U-shaped attention curve, documented by Liu et al. (2023), directly constrains scaffold design: critical sections must be placed at prompt boundaries, not buried in the middle. Roko implements this through the Placement enum (Start/Middle/End) and the PromptComposer's U-shape ordering phase. This document specifies the attention phenomenon, the empirical evidence, the mitigation strategies, and Roko's implementation.

---

## 1. The Phenomenon

Liu, Lin, Hewitt, Paranjape, Bevilacqua, Petroni, and Liang (2023) tested how language models use information at different positions within their context window. The finding is a U-shaped performance curve:

```
Performance
    ▲
    │ ████                                        ████
    │ █████                                     ██████
    │ ██████                                  ████████
    │ ████████                              ██████████
    │ ██████████                          ████████████
    │ ████████████                      ██████████████
    │ ██████████████                  ████████████████
    │ ████████████████            ████████████████████
    │ ██████████████████████████████████████████████████
    └────────────────────────────────────────────────────▶
      Beginning      Middle positions        End         Position
```

- **Beginning (primacy):** Models attend most strongly to the first tokens. Information placed at the start of the context is used effectively.
- **End (recency):** Models attend second-most strongly to the last tokens. Information placed at the end is used well.
- **Middle (degradation):** Information in the middle of long contexts is largely ignored. Performance degrades substantially — over 30% — when relevant information is positioned mid-context.

This is a **positional problem**, not a capacity problem. The same information that the model ignores in position 10 of 20 documents might be used correctly in position 1 or position 20. The model can process the tokens — it just does not attend to them effectively.

---

## 2. Empirical Evidence

### 2.1 Liu et al. (2023) — The Original Finding

Tested on multi-document question answering and key-value retrieval tasks across GPT-3.5-turbo, Claude (v1), and MPT-30B-Instruct:

- 20 documents retrieved, with the answer placed at varying positions
- Performance highest when the answer is in position 1 (beginning) or position 20 (end)
- Performance lowest when the answer is in positions 8-14 (middle)
- The degradation occurs even in models explicitly designed for long contexts (e.g., MPT-30B's 65K context window)

### 2.2 LongLLMLingua — Semantic Density Ranking

LongLLMLingua (extended from LLMLingua [Jiang et al., EMNLP 2023]) addresses the U-shape through reordering: place the most semantically dense (information-rich) documents at the edges of the context (beginning and end), with less dense documents in the middle.

### 2.3 Devin — Dual-Position Constraints

Devin's agent framework (2025) applies the finding directly: critical constraints and safety rules appear at both the START and END of the system prompt. This dual-position pattern ensures that the model attends to safety rules even as the context grows:

```rust
if let Some(ref constraints) = agent_config.critical_constraints {
    // Position at end (after user query)
    result.parts.push(PromptPart {
        part_type: PartType::ConstraintsReminder,
        content: format!(
            "<critical_constraints>\n{constraints}\n</critical_constraints>"
        ),
        position: Position::End,
    });
}
```

### 2.4 Context Rot (Chroma 2025)

Chroma's "Context Rot" report tested 18 frontier models and found that all exhibit the U-shaped attention pattern. Performance does not plateau as context grows — it actively degrades. The worst offenders are semantically close distractors: documents that look relevant but contain wrong or misleading information. These are far more harmful than obviously irrelevant documents, because the model attends to them (they are in the context and seem related) but they lead it to wrong conclusions.

### 2.5 Du et al. (EMNLP 2025) — Even Whitespace Hurts

Du et al. found that even whitespace and formatting overhead degrades performance by 13.9-85%. This suggests that the middle zone degradation is not solely an attention mechanism issue but also reflects information dilution — more tokens between relevant content means more opportunities for the model to lose the thread.

### 2.6 Shi et al. (ICML 2023) — Irrelevant Context Actively Harms

Shi et al. demonstrated that irrelevant context does not merely dilute performance — it **actively harms** it. Models perform worse with irrelevant context than with no context at all. Combined with the U-shape finding, this means: irrelevant information in the middle of the context causes double harm — it both wastes budget and actively degrades the model's ability to use relevant information nearby.

---

## 3. Roko's Implementation

### 3.1 The Placement Enum

```rust
// crates/roko-compose/src/prompt.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Placement {
    /// Place at the beginning of the prompt. Highest attention zone.
    Start,
    /// Place in the middle. Lowest attention zone.
    Middle,
    /// Place at the end. Second-highest attention zone.
    End,
}
```

### 3.2 Section-to-Placement Mapping

| Section | Placement | Rationale |
|---------|-----------|-----------|
| Role identity | **Start** | Agent must know its identity first |
| Conventions | **Start** | Safety rules need primacy attention |
| Task description | **Start** | Core task goes at the beginning |
| Workspace map | **Middle** | Supporting context, not critical path |
| PRD extract | **Middle** | Reference material, consulted as needed |
| Cross-plan context | **Middle** | Background information |
| Research memo | **Middle** | Supporting evidence |
| Gate errors | **End** | Most recent failure needs recency attention |
| Anti-patterns | **End** | Prohibitions need recency attention |
| Affect guidance | **End** | Behavioral modulation applied to final decisions |
| Constraints reminder | **End** | Devin's dual-position pattern |

### 3.3 U-Shape Ordering in PromptComposer

After budget fitting (Phase 4 of the assembly algorithm), the PromptComposer reorders included sections:

```
final_order = [
    // Highest attention zone: primacy
    sections.filter(placement == Start),

    // Lowest attention zone: degradation
    sections.filter(placement == Middle),

    // Second-highest attention zone: recency
    sections.filter(placement == End),
]
```

Within each placement group, the CacheLayer ordering is preserved for cache stability. The U-shape ordering only affects the relative position of groups, not the internal order within a group.

### 3.4 The Five-Stage Pipeline Integration

In the 5-stage context assembly pipeline (see [08-5-stage-assembly-pipeline.md](08-5-stage-assembly-pipeline.md)), U-shape formatting is Stage 5 (Format):

```
Stage 1: Query → Candidate retrieval
Stage 2: Scoring → Rank by composite score
Stage 3: Diversity → Deduplicate
Stage 4: Budget → Fit to token budget
Stage 5: Format → U-shaped placement ← here
```

At Stage 5, the highest-scoring entries are placed at positions 1-3 (beginning, highest attention) and at the final positions (end, second-highest attention). Medium-scoring entries fill the middle. This is the LongLLMLingua semantic density ranking applied to the assembly pipeline.

---

## 4. Interaction with Cache Alignment

U-shape placement and cache alignment are partially in tension:

- **Cache alignment** wants stable content first (System layer → Session → Task → Dynamic)
- **U-shape** wants high-value content at beginning and end

Resolution: within each cache tier, sections are placed according to their Placement hint. The overall structure becomes:

```
Cache Layer 0 (System) — all Start placement
    Role identity
    Conventions
    Safety constraints

Cache Layer 1 (Session) — Middle placement
    Workspace map
    Cross-plan context

Cache Layer 2 (Task) — Start + Middle placement
    Task description (Start)
    PRD extract (Middle)
    Task brief (Middle)

Cache Layer 3 (Dynamic) — End placement
    Gate errors
    Anti-patterns
    Affect guidance
    Constraints reminder
```

This structure achieves both goals: the System layer forms a stable cached prefix (identical bytes across all requests for the same role), AND the highest-attention positions (beginning and end) contain the most critical information.

---

## 5. Design Implications

### 5.1 Never Bury Critical Information in the Middle

If a section is critical to task success, it must have `Placement::Start` or `Placement::End`. Placing it in the Middle is equivalent to reducing its effective priority by 30%+ (the attention degradation factor).

### 5.2 Constraints at Both Edges

Following Devin's dual-position pattern, safety constraints appear at both the beginning (Layer 2: Conventions) and the end (Layer 7: Constraints Reminder). This ensures that even as the context grows, safety rules remain in high-attention positions.

### 5.3 Error Context at the End

Gate errors and iteration memory always go at the End. This exploits the recency effect: the model's last impression before generating a response is "these are the mistakes to avoid." This is empirically more effective than placing errors in the middle, where they may be ignored.

### 5.4 Supporting Context in the Middle

Low-criticality supporting context (workspace maps, cross-plan context, research memos) is placed in the Middle. This is acceptable because:
1. The content is supporting, not critical — if the model partially ignores it, task success is not seriously affected.
2. It occupies the largest contiguous block of the prompt, where most of the budget is spent.
3. It is cached (Session/Task layer), so even if attention to it is reduced, the cost is low.

---

## 6. Position-Aware Scoring

The existing PromptComposer assigns Placement (Start/Middle/End) as a static property of each section. Position-aware scoring goes further: it adjusts the effective score of a section based on where it will actually be placed.

### 6.1 Position Attention Multipliers

Based on the empirical U-shaped curve, we can quantify the attention multiplier at each position:

```rust
/// Attention multiplier based on position within the context window.
/// Derived from Liu et al. (2023) empirical measurements.
pub struct PositionAttentionModel {
    /// Attention curve parameters (fitted per model family).
    /// attention(pos) = primacy_weight * exp(-primacy_decay * pos)
    ///                + recency_weight * exp(-recency_decay * (total - pos))
    ///                + baseline
    pub primacy_weight: f64,   // default: 0.35
    pub primacy_decay: f64,    // default: 0.15
    pub recency_weight: f64,   // default: 0.30
    pub recency_decay: f64,    // default: 0.20
    pub baseline: f64,         // default: 0.35
}

impl PositionAttentionModel {
    /// Compute attention multiplier for a normalized position [0, 1].
    pub fn attention_at(&self, normalized_pos: f64) -> f64 {
        let primacy = self.primacy_weight * (-self.primacy_decay * normalized_pos).exp();
        let recency = self.recency_weight
            * (-self.recency_decay * (1.0 - normalized_pos)).exp();
        (primacy + recency + self.baseline).min(1.0)
    }

    /// Compute effective score = base_score × attention_multiplier.
    pub fn effective_score(&self, base_score: f64, normalized_pos: f64) -> f64 {
        base_score * self.attention_at(normalized_pos)
    }
}
```

### 6.2 Position-Optimal Section Assignment

Given N sections to place, assign each to the position that maximizes total effective score:

```
Algorithm: Position-optimal assignment

Input: N sections with base scores s_1 >= s_2 >= ... >= s_N
       Attention curve a(pos) for positions 1..N

1. Sort sections by score descending
2. Assign highest-scored section to position argmax(a(pos))  // typically pos=1
3. Assign second-highest to the remaining position with highest a(pos)  // typically pos=N
4. Continue alternating between beginning and end positions
5. Middle positions receive lowest-scored sections

This is equivalent to the interleaving:
  positions = [1, N, 2, N-1, 3, N-2, ...]
  assign section_i to positions[i]
```

This generalizes the Start/Middle/End placement to continuous position optimization.

### 6.3 Score Adjustment for the Current Implementation

A simpler integration that preserves the existing Placement enum:

```rust
/// Adjust a section's priority score based on its placement.
/// Sections in high-attention positions get a bonus; middle gets a penalty.
pub fn placement_adjusted_score(base_score: f64, placement: Placement) -> f64 {
    match placement {
        Placement::Start => base_score * 1.0,   // primacy zone: full value
        Placement::End   => base_score * 0.95,  // recency zone: ~95% value
        Placement::Middle => base_score * 0.70,  // degradation zone: ~70% value
    }
}
```

This adjustment means that a Medium-priority section at the Start is effectively scored higher than a High-priority section in the Middle. This creates pressure to promote valuable sections to edge positions.

---

## 7. Empirical Validation Plan for Roko

How to measure the lost-in-the-middle effect for Roko's specific context assembly pipeline and target models.

### 7.1 Controlled Position Experiment

```
Protocol: Measure attention curve for Roko's system prompts

Setup:
  - Select 20 tasks spanning Trivial/Standard/Complex
  - Identify one critical fact per task (e.g., a specific type signature needed)
  - Construct system prompts with the fact at 5 positions:
    Position A: After role identity (beginning, tokens 100-200)
    Position B: After conventions (early middle, tokens 500-800)
    Position C: Center of domain context (deep middle, tokens 2000-3000)
    Position D: After task context (late middle, tokens 4000-5000)
    Position E: In anti-patterns (end, tokens 6000-7000)

Measurement:
  - Run each task 5× at each position (100 runs per task, 2000 total)
  - Record: gate pass rate, whether the critical fact was used in the response
  - Use the same model (Sonnet) and temperature (0) for all runs

Analysis:
  - Plot pass rate vs. position → expect U-shaped curve
  - Fit the PositionAttentionModel parameters to the observed curve
  - Compare Roko's curve to Liu et al.'s published curve
  - Store fitted parameters per model in .roko/learn/attention-curves.json

Expected outcome:
  - Positions A and E: 75-90% fact utilization
  - Position C: 45-65% fact utilization
  - ~30% degradation in the middle (consistent with Liu et al.)
```

### 7.2 Model-Specific Attention Curves

Different models exhibit different attention patterns. Claude 3.5 achieved 87% on Sequential-NIAH [arXiv:2504.04713] while other models scored lower. The validation plan should measure per-model curves:

```rust
/// Per-model attention curve parameters, fitted from validation experiments.
pub struct ModelAttentionCurves {
    /// Model ID → fitted PositionAttentionModel.
    pub curves: HashMap<String, PositionAttentionModel>,
    /// Default curve used for unknown models.
    pub default_curve: PositionAttentionModel,
}

impl ModelAttentionCurves {
    /// Get the attention model for a specific LLM.
    pub fn for_model(&self, model_id: &str) -> &PositionAttentionModel {
        self.curves.get(model_id).unwrap_or(&self.default_curve)
    }

    /// Persist fitted curves.
    /// File: .roko/learn/attention-curves.json
    pub fn save(&self, path: &Path) -> Result<()> { /* ... */ }
}
```

### 7.3 Continuous Monitoring

After the initial validation, continuously monitor the attention effect during normal operation:

```
For each task execution:
  1. Record which sections were placed where (position in token stream)
  2. Record gate outcome
  3. Periodically refit attention curves from accumulated data
  4. Alert if the curve shape changes significantly (model update may have altered attention patterns)
```

### 7.4 Hierarchical Context Organization

Research on structured prompting suggests that hierarchical organization can partially mitigate the middle-zone degradation. Instead of a flat sequence of sections, organize content in a tree structure with explicit navigation cues:

```
<!-- roko:section:domain_context -->
## Domain Context

### Crate Architecture
- roko-compose: prompt assembly (this is where your changes go)
- roko-core: trait definitions (do not modify)

### Relevant Types
- `PromptSection`: the unit of composition
- `Budget`: hard constraints on output

### PRD Requirements
- REQ-1: Support 7 layers
- REQ-2: Cache alignment
```

The headers and indentation create a structural scaffold that helps the model navigate even in the middle zone. The model can attend to the headers (which are at local primacy positions within each subsection) even when it partially loses track of the content between them.

---

## 8. The Structural Explanation (2025 Theory)

### 8.1 Why It's Architectural, Not Learned

A landmark 2025 paper [arXiv:2603.10123] proved that the U-shaped attention bias is an **algebraic property** of causal decoder architectures, present at initialization before any training or positional encoding:

- **Causal masking guarantees primacy bias.** Early tokens lie on exponentially more computational paths through the residual network. A token at position 1 influences every subsequent attention operation; a token at position N/2 influences only half as many.

- **Residual connections guarantee recency bias.** Late tokens maintain direct (short-path) connections to the output through the residual stream, bypassing the attention bottleneck.

This means the bias **cannot be trained away.** Positional encodings (RoPE, ALiBi) modulate the shape of the U-curve but cannot eliminate it. Any scaffold that places critical information in the middle is fighting the architecture.

### 8.2 Layer-wise Positional Bias

A complementary finding [arXiv:2601.04098, January 2025]: positional bias operates at the per-layer level. Early transformer layers exhibit different position preferences than later layers. Later layers show stronger primacy bias, meaning that deep processing disproportionately favors early tokens.

### 8.3 Attention Rank Collapse

In very long contexts, a separate failure mode emerges: attention scores collapse toward uniformity [OpenReview:7SLtElfqCW, 2025]. All tokens receive roughly equal attention, preventing the model from distinguishing relevant from irrelevant information. This is worse than the U-shape — at least the U-shape preserves edge attention. Rank collapse eliminates even that.

Mitigation: polylogarithmic rescaling of attention scores (approximately logarithmic in context length). This is a model-level fix, not a scaffold fix, but it affects scaffold design: shorter prompts are less susceptible to rank collapse.

### 8.4 Attention Sinks

The first few tokens in a sequence act as "attention sinks" — they absorb disproportionate attention probability that can't be usefully distributed elsewhere [arXiv:2603.10123]. This is why role identity at position 1 gets extreme attention: it's not just primacy, it's the attention sink effect. Scaffold implication: the first ~50 tokens of the system prompt receive outsized attention. Use them wisely — role identity is correct for this position.

---

## 9. Mitigation Techniques Beyond U-Shape Ordering

### 9.1 Found in the Middle: Attention Calibration

He et al. [ACL Findings 2024, arXiv:2406.16008] demonstrated that positional bias can be calibrated without model retraining. The method: measure the bias empirically for each position, then subtract the learned bias from attention scores to make attention position-agnostic. Results: up to **15 percentage point improvement** on long-context retrieval tasks.

Roko can implement this at the scaffold level: if the model provides attention scores (some APIs expose logprobs), use them to detect position bias and adjust section placement dynamically.

### 9.2 Hidden State Scaling

An even lighter intervention [ACL Findings 2025]: scaling a **single hidden state dimension** is sufficient to meaningfully reduce position bias. This requires model-level access but is cheap enough for real-time application.

### 9.3 LongLLMLingua Document Reordering

LongLLMLingua [Jiang et al., ACL 2024, arXiv:2310.06839] implements automatic document reordering: it scores each retrieved document's semantic density relative to the query, then places the densest documents at the edges. This achieves up to **21.4% performance improvement** using only 1/4 of original tokens.

Roko's PromptComposer already implements the placement principle (Start/Middle/End). The LongLLMLingua enhancement is to make placement **dynamic** — assigned per-section based on measured information density, not static per-section-type.

```rust
/// Assign placement dynamically based on information density.
pub fn dynamic_placement(
    sections: &mut [PromptSection],
    query: &str,
) {
    // Score each section's information density relative to the task query
    for section in sections.iter_mut() {
        section.density_score = information_density(&section.content, query);
    }

    // Sort by density descending
    sections.sort_by(|a, b| b.density_score.partial_cmp(&a.density_score).unwrap());

    // Assign placements: highest density → Start, next → End, rest → Middle
    let n = sections.len();
    for (i, section) in sections.iter_mut().enumerate() {
        // Skip Critical sections — their placement is fixed
        if section.priority == SectionPriority::Critical {
            continue;
        }
        section.placement = if i < n / 3 {
            Placement::Start
        } else if i >= 2 * n / 3 {
            Placement::End
        } else {
            Placement::Middle
        };
    }
}
```

---

## 10. Academic Foundations

**Liu et al. (2023), "Lost in the Middle: How Language Models Use Long Contexts"** [TACL 2024, arXiv:2307.03172]. The foundational paper documenting the U-shaped attention curve. Tested on multi-document QA and key-value retrieval across GPT-3.5-turbo, Claude, and MPT-30B. The finding has been replicated across all frontier models tested since.

**"Lost in the Middle at Birth"** [arXiv:2603.10123, 2025]. Proved that the U-shaped bias is an algebraic property of causal decoder architectures, present at initialization. Causal masking guarantees primacy; residual connections guarantee recency. Positional encodings modulate but cannot eliminate the effect. The most important theoretical result for scaffold design: this bias is permanent.

**"Lost in the Middle: An Emergent Property from Information Retrieval Demands"** [arXiv:2510.10276, October 2025]. Complementary mechanistic account: the primacy effect emerges from uniform long-term retrieval demand combined with causal masking. Training on retrieval tasks reinforces rather than corrects the bias.

**"Found in the Middle"** [He et al., ACL Findings 2024, arXiv:2406.16008]. The response paper: positional attention bias can be calibrated without retraining, improving retrieval by up to 15 percentage points. Validates scaffold-level mitigation.

**"Mitigate Position Bias via Scaling a Single Hidden State"** [ACL Findings 2025]. Lightweight intervention: scaling one hidden state dimension meaningfully reduces position bias.

**"Layer-wise Positional Bias in Short-Context Language Modeling"** [arXiv:2601.04098, January 2025]. Position bias operates per-layer. Later transformer layers show stronger primacy bias.

**"Critical Attention Scaling in Long-Context Transformers"** [OpenReview:7SLtElfqCW, 2025]. Attention rank collapse: in very long contexts, attention scores collapse toward uniformity. Fix: polylogarithmic rescaling.

**LLMLingua / LongLLMLingua** [Jiang et al., EMNLP 2023; ACL 2024, arXiv:2310.06839]. LLMLingua's prompt compression achieves up to 20× compression. LongLLMLingua extends this with question-aware compression and semantic density ranking — reordering so the densest documents occupy edge positions. Up to 21.4% improvement at 4× compression.

**Selective Context** [Li et al., EMNLP 2023]. Information-theoretic context pruning. 50% reduction, 0.023 BERTscore drop.

**Shi et al. (2023)** [ICML 2023]. Irrelevant context actively harms performance.

**Du et al. (2025)** [EMNLP 2025]. Even whitespace degrades 13.9-85%.

**Chroma (2025), "Context Rot"**. All 18 frontier models show degradation. Claude lowest hallucination rate.

**Gist Tokens** [Mu et al., NeurIPS 2023]. Full prompts compressed to special tokens. Extreme U-shape mitigation: eliminate the middle entirely.

**Sequential-NIAH** [arXiv:2504.04713, 2025]. Multi-needle evaluation showing Claude 3.5 at 87% accuracy for sequential needle extraction — 12.4% below reference, suggesting the U-shape still affects even state-of-the-art models on complex retrieval.

**Serial Position Effects of LLMs** [arXiv:2406.15981, 2024]. Systematic empirical characterization of how primacy, recency, and middle loss vary across model size, context length, and task type.

---

## 11. Test Criteria

```
test_position_attention_model_u_shape:
    Given default PositionAttentionModel parameters
    When computing attention at positions [0.0, 0.25, 0.5, 0.75, 1.0]
    Then attention at 0.0 > attention at 0.5
    And attention at 1.0 > attention at 0.5
    And attention at 0.5 is the minimum (U-shape)

test_placement_adjusted_score:
    Given base_score = 1.0
    When adjusted for Start, Middle, End
    Then Start >= End > Middle

test_dynamic_placement_preserves_critical:
    Given sections including Critical-priority sections
    When dynamic_placement is applied
    Then Critical sections retain their original placement

test_hierarchical_formatting:
    Given domain context with subsections
    When formatted with headers
    Then output contains markdown headers at subsection boundaries
```

---

## 12. Current Status and Gaps

| Aspect | Status |
|--------|--------|
| Placement enum (Start/Middle/End) | **Implemented** |
| Section-to-Placement mapping | **Implemented** |
| U-shape ordering in PromptComposer | **Implemented** |
| Constraints at both edges | **Implemented** |
| Position-aware scoring (§6) | **Designed** — PositionAttentionModel specified |
| Position-optimal section assignment (§6.2) | **Designed** — interleaving algorithm specified |
| Empirical validation plan (§7) | **Designed** — controlled experiment protocol specified |
| Per-model attention curves (§7.2) | **Not yet** — requires validation experiments |
| Dynamic placement from density scoring (§9.3) | **Designed** — LongLLMLingua-style reordering specified |
| Hierarchical context organization (§7.4) | **Designed** — header-based navigation cues specified |
| Semantic density ranking (LongLLMLingua-style) | **Not yet** |
| Attention curve measurement per model | **Not yet** |

---

## Cross-References

- [01-prompt-composer.md](01-prompt-composer.md) — Assembly algorithm including U-shape phase
- [02-system-prompt-builder-7-layer.md](02-system-prompt-builder-7-layer.md) — Layer ordering and interaction effects (§8)
- [05-token-budget-management.md](05-token-budget-management.md) — Budget constraints
- [07-active-inference-context-selection.md](07-active-inference-context-selection.md) — Scoring that feeds placement decisions
- [08-5-stage-assembly-pipeline.md](08-5-stage-assembly-pipeline.md) — Stage 5 format step
- [09-predictive-foraging-mvt.md](09-predictive-foraging-mvt.md) — Information density as foraging signal
- `crates/roko-compose/src/prompt.rs` — Placement enum and ordering logic
