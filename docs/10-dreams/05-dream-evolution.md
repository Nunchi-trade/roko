# Dream Evolution: The Fourth Phase

> **Layer**: Cognitive Cross-Cut (L2 Scaffold knowledge recombination)
>
> **Synapse Traits**: `Scorer` (memetic fitness scoring), `Policy` (strategy evolution policy)
>
> **Crate**: `roko-dreams` (planned — not yet implemented)
>
> **Prerequisites**: [01-three-phase-cycle.md](01-three-phase-cycle.md), [04-consolidation-and-staging.md](04-consolidation-and-staging.md)


> **Implementation**: Scaffold

---

## The EVOLUTION Phase

Beyond the three core phases (NREM Replay, REM Imagination, Integration), the dream system includes a fourth phase: **EVOLUTION**. This phase operates on promoted knowledge entries — those that have already passed through the staging buffer and been validated by waking experience — and applies evolutionary selection pressures to generate higher-order strategies.

EVOLUTION is not triggered every dream cycle. It fires when the agent has accumulated a sufficient body of promoted knowledge (configurable threshold, default: 20 promoted entries since the last EVOLUTION cycle). The phase is computationally expensive and produces high-level strategic recombinations that reshape the agent's approach.

---

## Three Operations

### 1. Memetic Selection

Heuristics and strategies in NeuroStore compete for survival. EVOLUTION evaluates each promoted entry against the agent's recent performance:

- **High-fitness heuristics** (their presence correlated with successful episodes) receive a confidence boost and have their half-life extended by 1.5×.
- **Low-fitness heuristics** (their presence correlated with failed episodes, or they were never referenced in any episode) receive a confidence penalty and begin accelerated decay.
- **Neutral heuristics** (no correlation with success or failure) are left unchanged.

The fitness function:

```
fitness(heuristic) = success_rate_when_referenced / success_rate_when_not_referenced
```

A fitness > 1.0 means the heuristic is correlated with success. A fitness < 1.0 means the heuristic is correlated with failure. A fitness ≈ 1.0 means the heuristic has no effect.

This implements a simplified version of the memetic evolution described by Dawkins (1976, The Selfish Gene): ideas (memes) compete for replication within the agent's cognitive architecture. Successful memes survive; unsuccessful ones die.

### 2. Strategy Evolution via Imagined Returns

EVOLUTION takes pairs of high-fitness heuristics and combines them to produce candidate super-strategies:

```
Heuristic A (fitness 1.8): "{heuristic_a}"
Heuristic B (fitness 2.1): "{heuristic_b}"

These heuristics are both high-fitness. They both correlate with success.
What happens if you combine them into a single compound strategy?
Under what conditions would the compound be better than either alone?
Under what conditions would they conflict?
```

The compound strategies enter the staging buffer at confidence 0.30 (the maximum for dream-generated hypotheses). They represent the agent's best current thinking — validated heuristics combined in novel ways.

### 3. Knowledge Recombination

EVOLUTION applies Wright's (1932, Proceedings of the Sixth International Congress of Genetics) shifting balance theory: knowledge entries are randomly paired and recombined using HDC vector operations to explore the neighborhood of existing strategies.

The recombination uses HDC permutation — a cyclic bit-shift on the 10,240-bit BSC vector that represents the knowledge entry's content:

```rust
let recombined = HdcVector::bundle(&[
    &entry_a.hdc_vector,
    &entry_b.hdc_vector.permute(shift_amount),
]);
```

The permuted bundle creates a vector that is related to both parent entries but distinct from either. The nearest neighbors of this recombined vector in NeuroStore identify potentially relevant knowledge entries that the agent has not yet connected. These connections are surfaced as candidate insights.

This implements the "dream seed" concept: HDC vectors serve as seeds for knowledge exploration. Each permutation explores a slightly different region of the agent's knowledge space, analogous to genetic mutation in biological evolution.

---

## The Dream-Prediction Feedback Loop

EVOLUTION closes a critical feedback loop: dreams generate predictions, waking experience validates them, and the validation results feed back into future dreams.

```
DREAM → Generate hypothesis H with predicted outcome P
WAKE → Observe actual outcome O
DREAM → Compare P vs O
  If P ≈ O: boost confidence in H, reinforce the heuristics that generated H
  If P ≠ O: reduce confidence in H, weaken the heuristics that generated H
DREAM → Next EVOLUTION considers updated fitness scores
```

This is the **predictive foraging** mechanism from `agent-chain/10-predictive-foraging.md`: agents make falsifiable predictions, and incorrect predictions decay the knowledge that produced them. EVOLUTION's memetic selection operates on the fitness scores that this feedback loop produces.

---

## Implementation Status

The EVOLUTION phase is **not yet implemented** in `roko-dreams`. The implementation plan (§G8: "Novel strategy generation") lists it as a future item. The design is stable and the HDC primitives required (permutation, bundling, similarity) are all available in `roko-primitives` and `roko-learn`.

---

## Academic Citations

| Paper | How It Informs EVOLUTION |
|-------|-------------------------|
| Dawkins (1976), The Selfish Gene | Memetic evolution: ideas compete for survival within cognitive architectures |
| Wright (1932), "The roles of mutation, inbreeding, crossbreeding, and selection in evolution" | Shifting balance theory: exploration of adaptive landscapes via recombination |
| Kanerva (2009), Cognitive Computation 1(2), "Hyperdimensional Computing" | HDC permutation operations for knowledge recombination |
| Simonton (2010), "Creative thought as blind-variation and selective-retention" | BVSR theory applied to strategy evolution |

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [04-consolidation-and-staging.md](04-consolidation-and-staging.md) | Promoted entries that EVOLUTION operates on |
| [06-hdc-counterfactual-synthesis.md](06-hdc-counterfactual-synthesis.md) | HDC operations used for knowledge recombination |
| [../03-neuro/INDEX.md](../06-neuro/INDEX.md) | NeuroStore where evolved strategies are persisted |
