# 06 — Lost in the Middle: U-Shaped Attention Optimization

> Layer 2 Scaffold — Synapse Architecture
> Status: **Implemented** — Placement enum in `roko-compose::prompt`
> Canonical source: Liu et al., TACL 2024 [arXiv:2307.03172]

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

## 6. Academic Foundations

**Liu et al. (2023), "Lost in the Middle: How Language Models Use Long Contexts"** [TACL 2024, arXiv:2307.03172]. The foundational paper documenting the U-shaped attention curve. Tested on multi-document QA and key-value retrieval across GPT-3.5-turbo, Claude, and MPT-30B. The finding has been replicated across all frontier models tested since.

**LLMLingua** [Jiang et al., EMNLP 2023]. LLMLingua's prompt compression achieves up to 20× compression with minimal performance loss. LongLLMLingua extends this with semantic density ranking — reordering retrieved documents so the most information-dense content is at the edges.

**Selective Context** [Li et al., EMNLP 2023]. Information-theoretic approach to context pruning. 50% context reduction, 36% less memory, 32% faster inference with 0.023 BERTscore drop. Selective Context identifies and removes redundant content, which disproportionately occupies the middle positions.

**Shi et al. (2023), "Large Language Models Can Be Easily Distracted by Irrelevant Context"** [ICML 2023]. Irrelevant context actively harms performance. Combined with the U-shape finding, irrelevant content in the middle causes maximum harm.

**Du et al. (2025)** [EMNLP 2025]. Even whitespace degrades 13.9-85%. Formatting overhead in the middle zone amplifies the attention degradation.

**Chroma (2025), "Context Rot"**. All 18 frontier models tested show degradation as context grows. Claude showed the lowest hallucination rate, attributed to conservative abstention — when the context becomes confusing, Claude flags uncertainty rather than guessing.

**Gist Tokens** [Mu et al., NeurIPS 2023]. Full prompts can be compressed to a few special tokens. This represents the extreme form of the U-shape mitigation: if you can compress supporting context to gist tokens, you eliminate the middle zone entirely.

---

## 7. Current Status and Gaps

| Aspect | Status |
|--------|--------|
| Placement enum (Start/Middle/End) | **Implemented** |
| Section-to-Placement mapping | **Implemented** |
| U-shape ordering in PromptComposer | **Implemented** |
| Constraints at both edges | **Implemented** |
| Semantic density ranking (LongLLMLingua-style) | **Not yet** |
| Attention curve measurement per model | **Not yet** |
| Dynamic placement based on content scoring | **Not yet** |

---

## Cross-References

- [01-prompt-composer.md](01-prompt-composer.md) — Assembly algorithm including U-shape phase
- [05-token-budget-management.md](05-token-budget-management.md) — Budget constraints
- [07-active-inference-context-selection.md](07-active-inference-context-selection.md) — Scoring that feeds placement decisions
- [08-5-stage-assembly-pipeline.md](08-5-stage-assembly-pipeline.md) — Stage 5 format step
- `crates/roko-compose/src/prompt.rs` — Placement enum and ordering logic
