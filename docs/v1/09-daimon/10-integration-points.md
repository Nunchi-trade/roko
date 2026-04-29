# Integration Points

> The four ways the Daimon's PAD vector drives other systems: behavioral state selection, tier routing bias, VCG auction bidding, and somatic landscape querying.


> **Implementation**: Built

**Topic**: [Daimon](./INDEX.md)
**Prerequisites**: [04-six-behavioral-states.md](./04-six-behavioral-states.md), [05-behavioral-state-to-tier-routing.md](./05-behavioral-state-to-tier-routing.md), [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md)
**Key sources**: `refactoring-prd/03-cognitive-subsystems.md` §2, `refactoring-prd/09-innovations.md` §II–III, `roko-daimon/src/lib.rs`, `roko-golem/src/daimon.rs`

---

## Abstract

The Daimon's PAD vector is not a display value. It is a control signal that drives four systems simultaneously. Each system reads a different projection of the same PAD state:

1. **Behavioral state selection** — maps the PAD vector to one of six discrete states, used for self-model reporting and TUI display
2. **Tier routing bias** — modulates the CascadeRouter's prediction error thresholds, controlling compute allocation
3. **VCG auction bidding** — biases context window allocation through urgency and affect weight multipliers
4. **Somatic landscape querying** — provides fast heuristic pre-evaluation before analytical reasoning

These are not independent features — they are different consumers of the same signal. Changing the PAD vector changes all four simultaneously. This is the Daimon's architectural contribution: a single emotional state creates coherent behavioral change across the entire cognitive pipeline.

---

## Integration Point 1: Behavioral State Selection

### Mechanism

The PAD vector maps to one of six behavioral states (Engaged, Struggling, Coasting, Exploring, Focused, Resting) through threshold-based classification:

```
PAD → classify_behavioral_state() → BehavioralState
```

### What It Drives

- **Self-model**: The agent's internal representation of its own cognitive state. Used in the `<daimon>` context block injected into LLM prompts via the SystemPromptBuilder.
- **TUI display**: The behavioral state label appears in the dashboard and is reflected in the Spectre creature's visual state.
- **Conversational tone**: Each state maps to a PAD octant, which maps to a conversational style (see `07-runtime-daimon.md`). An Anxious agent hedges extensively; a Confident agent uses definitive language.

### Current Implementation

In `roko-daimon/src/lib.rs`, the `modulate()` method implements behavioral state selection implicitly through PAD threshold checks:

```rust
fn modulate(&self, params: &mut DispatchParams) {
    let state = self.query();

    if state.confidence < 0.30 || state.pad.dominance < -0.25 {
        // Struggling → Escalating
        params.strategy = DispatchStrategy::Escalating;
        params.turn_limit = params.turn_limit.saturating_add(10);
        params.model = promote_model(&params.model);
    } else if state.pad.pleasure > 0.35 && state.confidence > 0.65 {
        // Coasting → Exploratory
        params.strategy = DispatchStrategy::Exploratory;
        params.turn_limit = params.turn_limit.saturating_sub(5);
        params.model = demote_model(&params.model);
    } else if state.pad.pleasure < -0.30 && state.pad.arousal > 0.30 {
        // Struggling → Conservative
        params.strategy = DispatchStrategy::Conservative;
        params.turn_limit = params.turn_limit.saturating_sub(3);
        params.model = demote_model(&params.model);
    } else if state.pad.arousal < -0.20 {
        // Resting → Proactive
        params.strategy = DispatchStrategy::Proactive;
        params.turn_limit = params.turn_limit.saturating_add(5);
    } else {
        // Engaged → Balanced
        params.strategy = DispatchStrategy::Balanced;
    }

    params.effort = params.strategy.effort_label().to_string();
}
```

In `roko-golem/src/daimon.rs`, the `AffectOctant::behavior_modulation()` method provides the parallel implementation using octant-based classification.

---

## Integration Point 2: Tier Routing Bias

### Mechanism

The behavioral state modulates the CascadeRouter's prediction error thresholds:

```
PAD → behavioral_state → adjusted_thresholds(state) → CascadeRouter.select_tier()
```

### What It Drives

- **Model selection**: Which LLM processes this operation (haiku, sonnet, opus)
- **Compute cost**: T2 operations cost approximately 60× T0 operations
- **Response quality**: Stronger models produce better results on complex tasks
- **Latency**: T2 operations take 10–30 seconds vs. T0 at ~1ms

### The VCG Connection

The tier routing bias also indirectly affects VCG bidding (Integration Point 3) because the selected model tier determines how much context the operation can consume. A T2 operation with an opus-class model has a larger context window and can accommodate more VCG auction winners than a T1 operation with a haiku-class model.

### Cost Impact Summary

| Transition | Cost Change | When It Happens |
|---|---|---|
| Engaged → Struggling | ~3.5× increase | Sustained failures lower confidence below 0.30 |
| Engaged → Coasting | ~0.4× decrease | Sustained successes raise pleasure above 0.35 |
| Engaged → Resting | ~0.5× decrease | Low arousal (idle periods, post-dream) |
| Struggling → Engaged | ~0.3× decrease | Recovery through successful outcomes |
| Coasting → Engaged | ~2.5× increase | Encountering harder problems |

See [05-behavioral-state-to-tier-routing.md](./05-behavioral-state-to-tier-routing.md) for full threshold tables.

---

## Integration Point 3: VCG Auction Bidding

### Mechanism

The VCG (Vickrey-Clarke-Groves) auction allocates the limited context window among competing subsystems. The Daimon biases bidding through two multipliers:

```
bid = expected_value × urgency × affect_weight

where:
  urgency = 1 + arousal × 0.5
  affect_weight = 1 + 0.3 × abs(pleasure - 0.5)
```

### How Affect Modulates Bidding

**High arousal → increased urgency**: When the agent is under pressure (high arousal from time pressure, blockers, or failures), the urgency multiplier amplifies bids for safety-related and task-critical context sections. At arousal = 0.8, urgency = 1.4 — a 40% boost. This ensures that urgent situations receive richer context.

**Extreme pleasure (positive or negative) → increased affect weight**: When pleasure is far from neutral (0.5 in the [0,1] mapping), the affect weight increases. This means both very positive and very negative emotional states increase the weight of emotionally-relevant context:
- High pleasure (P = 0.8): affect_weight = 1 + 0.3 × |0.8 - 0.5| = 1.09. Modest boost — success doesn't dramatically change context allocation.
- Low pleasure (P = -0.3): affect_weight = 1 + 0.3 × |-0.3 - 0.5| = 1.24. Stronger boost — failure increases the weight of diagnostic context.

### Per-Subsystem Bidding Behavior

The Daimon's PAD state biases specific subsystems' bids:

| Subsystem | High Arousal Effect | Low Dominance Effect | Low Pleasure Effect |
|---|---|---|---|
| **Neuro** (knowledge) | Safety knowledge prioritized | Exploratory knowledge boosted | Warning knowledge boosted |
| **Daimon** (affect context) | Bid increases (own state is urgent) | Bid increases (needs self-model) | Bid increases (needs emotional context) |
| **Iteration memory** | Boost for recent failures | Neutral | **Strong boost** — past failure context critical |
| **Code intelligence** | Boost for safety-critical code paths | Neutral | Neutral |
| **Playbook rules** | Proven playbooks prioritized | Novel playbooks boosted | Conservative playbooks prioritized |
| **Research artifacts** | Neutral | **Strong boost** — research needed | Neutral |
| **Task context** | PRD sections with deadlines prioritized | Neutral | Neutral |
| **Oracle predictions** | Predictions with high error prioritized | Neutral | Neutral |

### VCG Truthfulness Guarantee

The VCG mechanism uses second-price payment: each winning bidder pays the bid of the next-highest loser. This creates truthful incentives — subsystems cannot benefit from inflating their bids because they pay based on others' bids, not their own. The Daimon's affect modulation changes the *actual value* of context (not just the bid), so it doesn't undermine truthfulness.

**Citation**: Vickrey (1961), Clarke (1971), Groves (1973). Applied to attention allocation following the context engineering framework (Karpathy 2025).

---

## Integration Point 4: Somatic Landscape Querying

### Mechanism

Before selecting an action, the agent queries the somatic landscape with the proposed strategy's 8D coordinates:

```
Strategy coords → SomaticLandscape.query() → SomaticSignal → pre-analytical bias
```

### What It Drives

- **Pre-filter on strategy space**: The somatic signal narrows the action space before analytical reasoning begins
- **Model tier suggestion**: Strong negative valence → suggest T2; strong positive valence → suggest T0/T1
- **Review scrutiny**: Negative valence increases the gate rung level for the resulting output
- **Exploration vs. exploitation**: Negative valence → prefer proven playbooks; positive valence → permit exploration

### Timing in the Cognitive Pipeline

The somatic query is the fastest decision signal, executing before everything else:

```
1. SOMATIC QUERY (< 1ms)        — k-d tree nearest neighbor
2. PREDICTION ERROR PROBES (< 5ms) — 16 deterministic probes
3. TIER SELECTION (~0ms)         — threshold comparison
4. CONTEXT ASSEMBLY (~10ms)      — VCG auction, knowledge retrieval
5. MODEL INFERENCE (~2-30s)      — LLM call at selected tier
```

The somatic query can preempt tier selection: if the somatic signal is strongly negative, it can force T2 before the prediction error probes even run. This is the System 1 fast path — the agent "feels" that this situation is dangerous before it has analytically assessed why.

### Interaction with PAD Vector

The somatic landscape and the PAD vector are complementary:

| Property | PAD Vector | Somatic Landscape |
|---|---|---|
| Question | "How do I feel right now?" | "How did I feel last time in a situation like *this*?" |
| Scope | Global mood | Situation-specific memory |
| Update frequency | Every appraisal event | Dream consolidation + significant live events |
| Query cost | O(1) | O(log N) |
| Affects | All four integration points | Pre-analytical bias only |

Both signals feed into the decision pipeline, but the somatic signal is more specific. The PAD vector says "I'm anxious." The somatic signal says "The last time I was in *this particular region of strategy space*, it went badly." The combination produces richer behavioral modulation than either signal alone.

---

## Integration Map

```
                    PAD Vector
                   ╱    │    ╲
                  ╱     │     ╲
                 ╱      │      ╲
                ╱       │       ╲
               ╱        │        ╲
    Behavioral      Tier         VCG        Somatic
     State         Routing      Auction     Landscape
       │             │            │            │
       ▼             ▼            ▼            ▼
   Self-model    CascadeRouter  Context     Pre-filter
   TUI display   Model select   assembly    Fast bias
   Tone map      Cost control   Token alloc  Strategy
                                             ranking
```

All four paths read the same PAD state. A change to pleasure, arousal, or dominance cascades through all four integration points simultaneously, producing coherent behavioral change.

---

## Event Emission

When the PAD state changes significantly (Euclidean delta > 0.15 from last emitted state), the system emits events:

| Event | Trigger | Consumers |
|---|---|---|
| `MoodUpdate` | PAD Euclidean delta > 0.15 | TUI, episode logger, connected clients |
| `DaimonAppraisal` | Every appraisal completes | TUI, episode logger, metrics |
| `SomaticMarkerFired` | Strong somatic marker match ( \|valence\| > 0.3 and intensity > 0.5 ) | WebSocket / server event consumers now receive it directly; TUI / episode-log consumers can subscribe |
| `EmotionalShift` | Dominant Plutchik emotion changes | TUI, notification system |

The 0.15 threshold prevents event flooding. A shift from Relaxed to Anxious (PAD distance ~1.2) always emits. Micro-fluctuations within the same octant (distance ~0.05) do not.

---

## Current Status and Gaps

**Implemented**:
- Integration Point 1 (behavioral state): Fully implemented in `roko-daimon`, with shared `BehavioralState` in `roko-core` and live state persisted on `AffectState`.
- Integration Point 2 (tier routing): Live Daimon PAD / behavioral state now feeds both `CascadeRouter` selection bias and `SystemPromptBuilder` affect guidance in the orchestration path.
- Integration Point 3 (VCG auction): Partially implemented — orchestration now passes live PAD state into `PromptComposer`, and the shared prompt auction applies urgency / affect weighting plus per-bidder PAD modulation and diagnostic externality payments.
- Integration Point 4 (somatic landscape): Partially implemented — `roko-daimon` owns the live 8D k-d-tree-backed landscape, task execution projects strategies into it, and the resulting somatic signal feeds routing, prompting, runtime events, and dream maintenance.
- Signal emission: MoodUpdate events are emitted when PAD change exceeds threshold (roko-golem). Affect signals emitted to JSONL (roko-golem).

**Still missing**:
- Integration Point 3 (VCG auction): exact welfare-maximizing settlement, broader bidder production, and richer fairness policy.
- Integration Point 4 (somatic landscape): true domain-native strategy extractors, collective contagion, and broader cross-surface coupling.
- Full emotional-memory propagation: `EmotionalTag` now reaches live conductor engrams, episode logs, and Neuro distillation inputs, but retrieval weighting and consolidation policy are still incomplete.

---

## Cross-References

- See [04-six-behavioral-states.md](./04-six-behavioral-states.md) for behavioral state definitions
- See [05-behavioral-state-to-tier-routing.md](./05-behavioral-state-to-tier-routing.md) for tier routing details
- See [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md) for somatic landscape
- See [11-coding-agent-integration.md](./11-coding-agent-integration.md) for coding-specific integration
- See topic [05-learning](../05-learning/INDEX.md) for CascadeRouter internals
