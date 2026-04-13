# Somatic Landscape Integration

> The Somatic Landscape integrates Damasio's somatic marker hypothesis into Neuro's retrieval pipeline — a k-d tree over an 8-dimensional strategy space that provides fast emotional heuristics for knowledge selection, with mandatory 15% contrarian retrieval to prevent confirmation bias.


> **Implementation**: Built

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [01-six-knowledge-types.md](./01-six-knowledge-types.md), [06-hdc-knowledge-encoding.md](./06-hdc-knowledge-encoding.md)
**Key sources**:
- `refactoring-prd/09-innovations.md` §III (SomaticLandscape struct, 8D strategy space)
- `refactoring-prd/09-innovations.md` §XIX.F (8D axis definitions per domain)
- `refactoring-prd/03-cognitive-subsystems.md` §2 (Daimon as Somatic Marker)
- `bardo-backup/prd/04-memory/02-emotional-memory.md` (PAD model, mood-congruent retrieval, Bower 1981)

---

## Abstract

Antonio Damasio's somatic marker hypothesis (1994) proposes that emotions provide fast heuristic signals — "gut feelings" — that guide decision-making before conscious deliberation. When a person encounters a situation similar to a past experience, the body generates somatic markers (physiological responses) that were associated with that experience's outcome. These markers serve as rapid, pre-analytical filters: approach (positive marker) or avoid (negative marker).

Roko implements this as a **Somatic Landscape** — a k-d tree over an 8-dimensional strategy space where each point carries an emotional valence derived from past outcomes. Before acting, the agent queries the landscape: "What does this region of strategy space *feel like*?" Nearby positive markers signal confidence (use cheaper models, move faster). Nearby negative markers signal caution (escalate to stronger models, slow down, request review).

The integration between the Somatic Landscape and Neuro's knowledge retrieval is now split across two layers: `roko-daimon` owns the situation-specific somatic landscape and uses it to bias dispatch before work begins, while `roko-neuro::ContextAssembler` applies PAD-biased retrieval, arousal-shaped scope, and a mandatory contrarian slice when selecting knowledge. The architecture is affective end-to-end, but the final direct fusion of somatic scores into Neuro's knowledge ranking is still pending.

---

## The SomaticLandscape Struct

```rust
// From refactoring-prd/09-innovations.md §III (design spec)
pub struct SomaticLandscape {
    tree: KdTree<f64, SomaticMarker, 8>,
}

pub struct SomaticMarker {
    pub strategy_coords: [f64; 8],  // 8D strategy space
    pub valence: f64,               // positive or negative (-1 to +1)
    pub intensity: f64,             // how strong the feeling (0 to 1)
    pub episodes: Vec<ContentHash>, // which episodes formed this marker
}
```

### How It Works

1. **Before acting**: The agent maps its current situation to 8D strategy coordinates
2. **Query landscape**: Find nearest neighbors in the k-d tree (< 1ms)
3. **Aggregate valence**: Compute the weighted mean valence of nearby markers
4. **Route accordingly**:
   - Strong negative valence → caution → stronger model (System 2 routing)
   - Strong positive valence → confidence → cheaper model (System 1 routing)
   - Neutral or mixed → standard routing
5. **After acting**: Record the outcome as a new somatic marker at the current coordinates

### The 8-Dimensional Strategy Space

The 8 dimensions are domain-configurable. Each represents a continuous [0, 1] dimension of the current strategy:

**Coding domain (default)**:

| Dim | Name | Low (0.0) | High (1.0) |
|---|---|---|---|
| 1 | Complexity | Simple, single-function change | Complex, multi-file refactor |
| 2 | Risk | No existing tests could break | Core infrastructure change |
| 3 | Novelty | Familiar pattern, done before | Completely new territory |
| 4 | Confidence | Uncertain about approach | High confidence from past experience |
| 5 | Time pressure | No deadline, explore freely | Urgent, budget nearly exhausted |
| 6 | Scope | Single file, isolated change | Cross-crate, many dependents |
| 7 | Reversibility | Easily reverted (one commit) | Hard to undo (database migration) |
| 8 | Dependency depth | Leaf code, no downstream consumers | Core trait, many implementors |

**Chain domain**:

| Dim | Name | Low (0.0) | High (1.0) |
|---|---|---|---|
| 1 | Volatility | Stable market conditions | Extreme price swings |
| 2 | Exposure | Minimal position size | Large relative to portfolio |
| 3 | Liquidity | Deep, well-traded market | Thin order book |
| 4 | Correlation | Uncorrelated to existing positions | Highly correlated |
| 5 | Leverage | No leverage | High leverage |
| 6 | Time horizon | Long-term hold | Short-term trade |
| 7 | Slippage risk | Large pool, minimal slippage | Small pool, high slippage |
| 8 | Counterparty risk | Established protocol | New, unaudited contract |

---

## Mood-Congruent Retrieval

### Bower's Theory (1981)

Gordon Bower's mood-congruent memory research demonstrated that people preferentially recall memories whose emotional tone matches their current mood. Happy people recall happy memories more easily; anxious people recall threatening memories more easily.

In Neuro, mood-congruent retrieval is implemented as a **retrieval bias** based on the Daimon's current PAD (Pleasure-Arousal-Dominance) state:

```
retrieval_weight = base_weight × (1 + 0.15 × mood_congruence)
```

Where `mood_congruence` is the dot product between the entry's emotional valence and the Daimon's current PAD vector (normalized to [-1, 1]). The 0.15 coefficient means mood congruence contributes at most 15% to the retrieval weight — significant enough to bias selection but not dominant.

**Effect**: When the Daimon's Pleasure dimension is low (task failing, errors accumulating), the retrieval pipeline biases toward entries with negative valence — Warnings, AntiKnowledge, past failure analyses. This is the computational equivalent of "I have a bad feeling about this" — the agent's negative affect activates cautionary knowledge.

### Mandatory 15% Contrarian Retrieval

To prevent emotional echo chambers — where negative mood retrieves only negative knowledge, reinforcing the negative mood — Neuro enforces **mandatory 15% contrarian retrieval** (Bower 1981):

```
For each knowledge retrieval batch (e.g., top 20 entries):
    - 85% selected by standard retrieval score (confidence × decay × similarity × mood)
    - 15% (at least 3 entries) selected from the OPPOSITE valence
```

If the Daimon is in a negative state, 15% of retrieved entries are forced to be positive-valence (successful strategies, confirmed patterns). If the Daimon is in a positive state, 15% are forced to be negative-valence (warnings, failure analyses).

This prevents two failure modes:
1. **Panic lock-in**: Negative mood → only negative knowledge → more negative mood → spiral
2. **Overconfidence**: Positive mood → only positive knowledge → blind to risks → failure

---

## PAD Vector Integration

The Daimon PAD vector (see topic [09-daimon](../09-daimon/INDEX.md)) drives several aspects of Neuro's retrieval:

| PAD Dimension | Low Value Effect on Retrieval | High Value Effect on Retrieval |
|---|---|---|
| **Pleasure** | Bias toward Warnings and AntiKnowledge | Bias toward Heuristics and StrategyFragments |
| **Arousal** | Retrieve broadly (exploration mode) | Retrieve narrowly (focus on urgent entries) |
| **Dominance** | Bias toward exploratory/research entries | Bias toward execution-focused entries |

### Arousal Encoding

The Yerkes-Dodson law (1908) describes an inverted-U relationship between arousal and performance: moderate arousal is optimal, while both very low and very high arousal impair performance.

In Neuro, arousal modulates **retrieval scope**:
- **Low arousal** → broad retrieval (more entries, more diverse, lower similarity threshold)
- **Moderate arousal** → balanced retrieval (standard settings)
- **High arousal** → narrow retrieval (fewer entries, higher similarity threshold, focus on the most relevant)

```
effective_limit = base_limit × (1 + 0.5 × (1 - |arousal - 0.5| × 2))
// At moderate arousal (0.5): limit × 1.5 (broadest)
// At extreme arousal (0 or 1): limit × 1.0 (narrowest)
```

### Emotional Decay

Somatic markers have their own decay rate, separate from knowledge entry decay:

- **Emotional half-life**: 3 days (from `02-emotional-memory.md`)
- **Knowledge half-life**: Type-dependent (7–365 days)

Emotions fade faster than knowledge. An agent may remember that "using Arc fixes borrow checker errors" (knowledge, 30-day Insight half-life) long after it has forgotten how frustrated it felt during the debugging session (emotion, 3-day half-life). This mirrors the human experience: we remember facts longer than we remember feelings.

The Walker & van der Helm (2009) SFSR (Sleep to Forget, Sleep to Remember) model explains this: during REM sleep (or the Dreams consolidation cycle), emotional charge is depotentiated while factual content is consolidated. Each Dreams cycle reduces emotional intensity by 0.3–0.5 on the valence scale, while the associated knowledge entry retains its content and confidence.

---

## Academic Foundations

- Damasio, A. R. (1994). *Descartes' Error: Emotion, Reason, and the Human Brain*. Putnam. (Somatic marker hypothesis)
- Bower, G. H. (1981). "Mood and Memory." *American Psychologist*, 36(2), 129–148. (Mood-congruent retrieval, 15% contrarian)
- Yerkes, R. M., & Dodson, J. D. (1908). "The relation of strength of stimulus to rapidity of habit-formation." *Journal of Comparative Neurology and Psychology*, 18(5), 459–482.
- Walker, M. P., & van der Helm, E. (2009). "Overnight therapy? The role of sleep in emotional brain processing." *Psychological Bulletin*, 135(5), 731–748. (SFSR model, emotional depotentiation during sleep)
- Mehrabian, A. (1996). "Pleasure-Arousal-Dominance: A general framework for describing and measuring individual differences in temperament." *Current Psychology*, 14, 261–292. (PAD model)
- Plutchik, R. (1980). *Emotion: A Psychoevolutionary Synthesis*. Harper & Row. (Emotional classification)
- Gadanho, S. C. (2003). "Learning behavior-selection by emotions and cognition in a multi-goal robot task." *Journal of Machine Learning Research*, 4, 385–412. (Computational somatic markers)

---

## Current Status and Gaps

**Implemented**:
- `roko-daimon` now has a persisted `SomaticLandscape` backed by `kiddo`, with `SomaticMarker` and `SomaticSignal` types over the coding-domain 8D strategy space
- `roko-cli` projects task execution context into `StrategyCoordinates`, records live task outcomes into the somatic landscape, and queries that landscape before dispatch
- `roko-neuro::ContextAssembler` applies mood-congruent retrieval bias, a mandatory contrarian slice, and arousal-shaped allocation pressure during context assembly
- `KnowledgeEntry` has `hdc_vector` and emotional provenance fields for similarity-based and affect-aware retrieval
- Behavioral states are defined and fed into routing and prompting

**Still missing**:
- Domain-configurable 8D axis definitions beyond the current coding-task projection
- Direct use of somatic scores inside Neuro knowledge ranking rather than only Daimon routing
- Emotional decay (3-day half-life) separate from knowledge decay
- SFSR emotional depotentiation during Dreams (Walker & van der Helm 2009)
- Flashbulb memory encoding for high-arousal events
- Emotional contagion between agents in a collective (0.3 attenuation factor per hop)
- Integration between somatic markers and the VCG attention auction bidding

---

## Cross-References

- See [01-six-knowledge-types.md](./01-six-knowledge-types.md) for how types interact with emotional retrieval
- See [07-ebbinghaus-decay-with-tier.md](./07-ebbinghaus-decay-with-tier.md) for knowledge decay (distinct from emotional decay)
- See [10-knowledge-query-api.md](./10-knowledge-query-api.md) for the retrieval API that integrates somatic markers
- See topic [09-daimon](../09-daimon/INDEX.md) for the Daimon PAD vector that drives somatic integration
- See topic [10-dreams](../10-dreams/INDEX.md) for emotional depotentiation during Dreams
