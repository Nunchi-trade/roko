# Cognitive Cross-Cuts: Neuro, Daimon, Dreams

> **Abstract:** Three cognitive subsystems — Neuro (knowledge), Daimon (motivation), and
> Dreams (offline learning) — are injected across multiple architectural layers rather than
> living at any single level. These cross-cuts provide the self-improving capabilities that
> distinguish Roko from static agent frameworks. This document specifies each subsystem's
> role, data structures, integration points, and theoretical foundations.


> **Implementation**: Shipping

---

## 1. Why Cross-Cuts

In a strictly layered architecture, where does knowledge management live? It is needed at
L2 (context engineering needs knowledge to enrich prompts), L3 (gates need knowledge to
calibrate thresholds), and L4 (the orchestrator needs knowledge to plan better). Forcing
knowledge into any single layer would require upward dependencies, violating the layering
principle.

The solution: cross-cutting concerns are injected via trait objects. The knowledge subsystem
(Neuro) implements the `Substrate` trait. Any layer that needs knowledge receives a
`&dyn Substrate` pointing to the NeuroStore. No layer needs to import Neuro directly — it
receives the trait object via dependency injection.

This pattern applies to all three cognitive cross-cuts: Neuro, Daimon, and Dreams.

---

## 2. Neuro — Knowledge Management

`roko-neuro` provides persistent, tier-based knowledge management with HDC encoding for
similarity search.

### 2.1 Six Knowledge Types

| Type | Purpose | Example |
|---|---|---|
| **Insight** | A general observation that proved useful | "This codebase uses builder pattern extensively" |
| **Heuristic** | A procedural rule extracted from experience | "When tests fail with E0599, check trait imports first" |
| **Warning** | A known pitfall or anti-pattern | "Never use --no-verify with this repo's hooks" |
| **CausalLink** | A cause-effect relationship | "Upgrading alloy requires rustc 1.91+" |
| **StrategyFragment** | A reusable strategic approach | "For large refactors, use worktrees for parallel branches" |
| **AntiKnowledge** | Explicitly falsified knowledge | "Hypothesis X was tested and disproved" |

AntiKnowledge is particularly important: it prevents the system from re-exploring dead ends.
When a hypothesis is falsified, it is stored as AntiKnowledge so that future agents do not
waste time on the same idea.

### 2.2 Four Knowledge Tiers

Knowledge progresses through four tiers with different retention characteristics:

| Tier | Strength Multiplier | Effective Half-Life | Promotion Criteria |
|---|---|---|---|
| **Transient** | 0.1× | Minutes to hours | Created on first observation |
| **Working** | 0.5× | Hours to days | Referenced in 2+ successful ticks |
| **Consolidated** | 1.0× | Days to weeks | Validated by gate verdicts or prediction outcomes |
| **Persistent** | 5.0× | Weeks to months | Repeatedly validated across multiple sessions |

Tier promotion happens during the Dreams consolidation cycle (Delta frequency). Knowledge
that proves useful is promoted; knowledge that fails to prove itself decays naturally via
Ebbinghaus forgetting (see [04-decay-variants.md](04-decay-variants.md)).

### 2.3 HDC Encoding

Knowledge entries are encoded as 10,240-bit Hyperdimensional Computing (HDC) vectors
(Kanerva 2009, Cognitive Computation 1(2)) for O(1) similarity search:

- **Bind** (XOR): Combines two concepts into a bound pair
- **Bundle** (majority): Combines multiple vectors, preserving similarity to all inputs
- **Similarity** (Hamming distance): Measures overlap between vectors

HDC encoding enables the Cross-Domain Insight Resonance feature (see
[17-design-principles-and-frontier-summary.md](17-design-principles-and-frontier-summary.md)):
knowledge from one domain can be retrieved when it is structurally similar to a query from
a different domain, even if the domains share no vocabulary.

### 2.4 Integration Points

| Layer | How Neuro Is Used |
|---|---|
| L2 Scaffold | Composer queries NeuroStore for relevant knowledge to include in prompts |
| L3 Harness | Gate thresholds informed by historical knowledge about pass rates |
| L4 Orchestration | Planner queries for heuristics about task decomposition |
| Cognitive (Dreams) | Dreams reads from and writes to NeuroStore during consolidation |

---

## 3. Daimon — Motivation and Focus

`roko-daimon` provides the agent's self-model: a PAD (Pleasure-Arousal-Dominance) vector
that modulates tier routing, context bidding, and risk tolerance.

### 3.1 PAD Vector

The PAD model (Mehrabian & Russell 1974; Russell & Mehrabian 1977, Journal of Personality
and Social Psychology 35(4)) represents emotional state as three orthogonal dimensions:

| Dimension | Range | What It Represents |
|---|---|---|
| **Pleasure** (P) | [-1, 1] | Task success vs. failure. Positive when things are going well. |
| **Arousal** (A) | [-1, 1] | Urgency and load. High when there is surprise or pressure. |
| **Dominance** (D) | [-1, 1] | Confidence and control. High when the agent feels capable. |

The PAD vector is NOT a personality — it is a dynamic state that changes continuously based
on recent outcomes, gate verdicts, prediction accuracy, and task load.

### 3.2 Six Behavioral States

The PAD vector maps to six behavioral states (a simplification of Plutchik's emotion wheel,
Plutchik 2001, American Scientist 89(4)):

| State | PAD Region | Behavior |
|---|---|---|
| **Engaged** | P+, A moderate, D+ | Productive work. Standard Theta cadence. |
| **Focused** | P+, A low, D+ | Deep work. Extended Gamma runs, fewer Theta interruptions. |
| **Exploring** | P neutral, A+, D neutral | Curious. Higher exploration rate, more T2 escalation. |
| **Struggling** | P-, A+, D- | Difficulty. Shortened Theta cadence, more frequent reflection. |
| **Coasting** | P neutral, A-, D+ | Easy work. Extended Gamma, T0-heavy. |
| **Resting** | P neutral, A-, D neutral | Idle. Delta consolidation mode. |

These states are cyclical — there is no terminal state. The agent cycles between states
based on task outcomes and environmental changes. This replaces the legacy mortality phases
(Thriving → Terminal) which had a final death destination.

### 3.3 Somatic Markers

Damasio's somatic marker hypothesis (Damasio 1994, Descartes' Error, Putnam) proposes that
emotional signals from past experience bias decision-making before conscious deliberation.
In Roko, somatic markers are implemented as score modifiers that the Daimon applies to
Router selections:

- An agent that previously failed when using a particular tool has a negative somatic marker
  for that tool — the Router will prefer alternatives.
- An agent that succeeded with a particular approach has a positive somatic marker — the
  Router will prefer repeating it.

These markers implement "gut feelings" computationally: fast heuristic signals that guide
decision-making before analytical reasoning (T2) engages.

### 3.4 Integration Points

| Layer | How Daimon Is Used |
|---|---|
| L0 Runtime | Adaptive clock uses PAD for frequency selection (anxious → shorter Theta) |
| L1 Framework | Router uses PAD for tier escalation (low confidence → T2) |
| L2 Scaffold | Composer uses PAD for context bidding (high arousal → include more safety context) |
| Cognitive (Dreams) | Dreams use PAD for emotional depotentiation during REM phase |

---

## 4. Dreams — Offline Learning

`roko-dreams` provides offline learning during idle time (Delta frequency). The Dreams cycle
is inspired by sleep neuroscience: the two-stage model of memory consolidation (CLS theory,
McClelland et al. 1995, Psychological Review 102(3)) and the active inference model of
dreaming (Walker & van der Helm 2009, Annual Review of Clinical Psychology 5).

### 4.1 Three-Phase Cycle

| Phase | Inspiration | What Happens |
|---|---|---|
| **NREM Replay** | Slow-wave sleep replay (Mattar & Daw 2018, Nature Neuroscience 21) | Replay recent episodes, weighted by prediction error magnitude. Extract patterns. |
| **REM Imagination** | REM sleep creativity (Boden 2004, The Creative Mind) | Generate novel hypotheses via HDC recombination. Counterfactual reasoning via Pearl's SCM (Pearl 2009, Causality). Emotional depotentiation (Walker & van der Helm 2009). |
| **Integration Staging** | Memory consolidation (Lacaux et al. 2021, Science Advances 7(50)) | Validate dream outputs against existing knowledge. Promote to NeuroStore if confidence exceeds threshold (0.20 → 0.70 promotion). |

### 4.2 NREM Replay Details

During NREM replay, the agent re-examines recent episodes prioritized by their prediction
error — episodes where the outcome differed most from what the agent predicted are replayed
first. This follows Mattar & Daw's gain model of hippocampal replay: replay what is most
useful for future decisions, not what was most recent.

### 4.3 REM Imagination Details

REM imagination generates novel hypotheses by:

1. **HDC recombination**: Taking knowledge vectors from different domains and combining them
   via majority bundling to find structural analogies.
2. **Counterfactual generation**: Using Pearl's Structural Causal Model to ask "what if?"
   questions about past episodes.
3. **Emotional depotentiation**: Reducing the emotional charge of negative experiences
   (Walker & van der Helm 2009) so that the agent can learn from failures without being
   biased against similar future opportunities.

### 4.4 Hypnagogia Engine

The hypnagogia engine generates creative hypotheses during the transition between active
work and consolidation. Four components:

| Component | Role |
|---|---|
| **Thalamic Gate** | Filters incoming stimuli, allowing only high-novelty signals through |
| **Executive Loosener** | Relaxes constraint satisfaction thresholds, enabling unusual associations |
| **Dali Interrupt** | Captures fleeting insights before they fade (named after Dalí's nap technique) |
| **Homuncular Observer** | Coherence filter that evaluates whether the generated hypothesis is worth testing |

This addresses the **Alpha Convergence Problem**: without creative divergence, an agent's
knowledge converges to a local optimum. The hypnagogia engine provides the "random restart"
that exploration/exploitation algorithms need — but with structure, not pure randomness.

### 4.5 Integration Points

| Layer | How Dreams Is Used |
|---|---|
| L0 Runtime | Delta frequency triggers Dreams cycle during idle time |
| Cognitive (Neuro) | Dreams reads episodes from and writes consolidated knowledge to NeuroStore |
| Cognitive (Daimon) | Dreams uses PAD for emotional depotentiation; updates PAD after consolidation |

---

## 5. The Cross-Cut Interaction Model

The three cross-cuts interact bidirectionally:

```
Daimon ←→ Neuro
  PAD biases knowledge retrieval    │  Knowledge outcomes update PAD
  High arousal → safety knowledge   │  Validated knowledge → pleasure increase
  Low confidence → cautious recall  │  Falsified knowledge → dominance decrease

Daimon ←→ Dreams
  PAD triggers Dreams (low arousal → Delta)  │  Dreams depotentiate negative PAD
  PAD modulates dream intensity              │  Dreams update behavioral state

Neuro ←→ Dreams
  Neuro provides episodes for replay  │  Dreams produce consolidated knowledge
  Neuro's tiers guide replay priority │  Dreams promote/demote knowledge tiers
```

This triangular interaction creates the self-improving cognitive system: the agent
experiences (Neuro records), reflects (Daimon assesses), consolidates (Dreams synthesizes),
and the cycle continues.

---

## 6. Cross-cut arbitration protocol

When two or more cross-cuts produce conflicting signals for the same decision, an arbitration protocol resolves the conflict.

### 6.1 Priority hierarchy

Cross-cuts have a fixed priority ordering:

| Priority | Cross-cut | Rationale |
|---|---|---|
| 1 (highest) | **Daimon** (safety) | Safety constraints override all other concerns |
| 2 | **Neuro** (knowledge) | Factual knowledge overrides learned preferences |
| 3 (lowest) | **Dreams** (hypotheses) | Dream-generated hypotheses are speculative |

When Daimon's safety constraints conflict with a Neuro heuristic, Daimon wins. When Neuro's validated knowledge conflicts with a Dreams hypothesis, Neuro wins.

### 6.2 Conflict scenarios and resolutions

**Scenario 1: Daimon vs Neuro -- risk tolerance**

Daimon's PAD vector indicates low dominance (low confidence), so it wants to escalate to T2 (slow, deliberate reasoning). Neuro has a Persistent heuristic that says "this task type always succeeds at T0 (fast, automatic)."

Resolution: Daimon wins. The agent escalates to T2. Safety-driven caution overrides historical success patterns because the PAD vector reflects the current state, which may differ from historical conditions.

```
fn resolve_tier_conflict(daimon: TierRecommendation, neuro: TierRecommendation) -> Tier {
    // Daimon's safety assessment always overrides
    if daimon.safety_critical {
        return daimon.tier;
    }
    // Otherwise, use the more cautious recommendation
    daimon.tier.max(neuro.tier) // higher tier = more deliberate
}
```

**Scenario 2: Neuro vs Dreams -- contradictory knowledge**

Neuro has a Consolidated knowledge entry: "alloy requires rustc 1.91+." Dreams generated a hypothesis during REM imagination: "alloy might work with rustc 1.85 using feature flags."

Resolution: Neuro wins. Consolidated knowledge (validated by gate verdicts) overrides Dream hypotheses (speculative). The Dreams hypothesis is stored as a candidate for testing but does not influence the current task.

```
fn resolve_knowledge_conflict(neuro: &KnowledgeEntry, dream: &DreamHypothesis) -> Action {
    if neuro.tier >= KnowledgeTier::Consolidated {
        // Validated knowledge wins; queue dream for future testing
        Action::UseNeuro { queue_dream: true }
    } else {
        // Low-tier neuro knowledge: dream may be worth testing
        Action::TestDream { fallback: neuro }
    }
}
```

**Scenario 3: Daimon vs Dreams -- emotional state**

Daimon is in the Struggling state (P-, A+, D-) and wants shorter Theta cadence (more frequent reflection). Dreams has just completed consolidation and wants to transition to Resting (low arousal, consolidation mode).

Resolution: Daimon wins. Active task performance takes priority over consolidation scheduling. Dreams consolidation is deferred until Daimon's state stabilizes.

### 6.3 VCG auction as tiebreaker

When the priority hierarchy does not cleanly resolve a conflict (two signals at the same priority, or a borderline case), the system falls back to a VCG (Vickrey-Clarke-Groves) attention auction.

Each cross-cut bids for influence on the decision. The bid is the cross-cut's confidence in its recommendation:

```
fn vcg_tiebreak(bids: &[(CrossCut, f32, Action)]) -> Action {
    // Sort by bid value (confidence) descending
    let mut sorted = bids.to_vec();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    // Winner pays the second-highest bid (VCG mechanism)
    // This incentivizes truthful confidence reporting
    let winner = &sorted[0];
    let second_price = if sorted.len() > 1 { sorted[1].1 } else { 0.0 };

    // The "payment" is logged as attention cost for the winning cross-cut
    log_attention_cost(winner.0, second_price);

    winner.2.clone()
}
```

The VCG mechanism ensures truthful reporting: a cross-cut gains nothing by inflating its confidence, because the price it pays is determined by the second-highest bid, not its own.

This tiebreaker is invoked only when:
- Two cross-cuts are at the same priority level
- Both have confidence > 0.5
- The conflict affects a Router or Composer decision (not safety)

### 6.4 Arbitration configuration

| Parameter | Default | Range | Description |
|---|---|---|---|
| `safety_always_wins` | true | bool | Daimon safety override cannot be disabled |
| `vcg_min_confidence` | 0.5 | 0.1 - 0.9 | Minimum confidence to participate in VCG |
| `knowledge_tier_threshold` | Consolidated | Transient - Persistent | Minimum Neuro tier to override Dreams |
| `dream_testing_enabled` | true | bool | Whether Dreams hypotheses are queued for testing |

### 6.5 Integration wiring

The arbitration protocol lives at the Router level (L1 Framework), where cross-cut signals converge:

```
Cross-cut signals arrive at Router:
    Daimon.recommend_tier(task) -> TierRecommendation
    Neuro.query_relevant(task)  -> Vec<KnowledgeEntry>
    Dreams.recent_hypotheses()  -> Vec<DreamHypothesis>
        |
        v
    Arbitrator.resolve(daimon, neuro, dreams) -> Decision
        |
        v
    Router uses Decision to select model and parameters
```

### 6.6 Test criteria

1. Daimon safety override always wins regardless of Neuro confidence.
2. Consolidated Neuro knowledge overrides Dreams hypothesis.
3. Transient Neuro knowledge does not override Dreams hypothesis with confidence > 0.8.
4. VCG tiebreaker selects the higher-confidence signal.
5. VCG payment equals the second-highest bid (not the winner's bid).
6. Arbitration logs include the conflict type, participants, and resolution.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Kanerva 2009, Cognitive Computation 1(2) | HDC: hyperdimensional computing for similarity search. |
| Plate 2003, Holographic Reduced Representation | HRR: holographic encoding for knowledge representation. |
| Frady et al. 2018 | Neural computation with HDC vectors. |
| Kleyko et al. 2022, Artificial Intelligence Review | Survey of HDC applications. |
| Mehrabian & Russell 1974 | PAD model: Pleasure-Arousal-Dominance emotional space. |
| Russell & Mehrabian 1977, JPSP 35(4) | Empirical validation of the PAD dimensional model. |
| Damasio 1994, Descartes' Error | Somatic marker hypothesis: emotion biases decision-making. |
| Plutchik 2001, American Scientist 89(4) | Emotion wheel: mapping complex emotions to dimensional space. |
| Gebhard 2005 | ALMA: three-layer affect model for computational emotion. |
| Scherer 2001, Applied AI 15 | Appraisal theory of emotion. |
| McClelland et al. 1995, Psychological Review 102(3) | Complementary Learning Systems theory (CLS): hippocampal-neocortical consolidation. |
| Mattar & Daw 2018, Nature Neuroscience 21 | Prioritized replay: replay what is most useful for future decisions. |
| Walker & van der Helm 2009, Annual Review of Clinical Psychology 5 | REM sleep emotional depotentiation. |
| Lacaux et al. 2021, Science Advances 7(50) | Hypnagogia: creative insights during sleep onset. |
| Boden 2004, The Creative Mind | Computational creativity: exploratory, combinational, transformational. |
| Pearl 2009, Causality, CUP | Structural Causal Models for counterfactual reasoning. |

---

## Current Status and Gaps

- **Neuro**: `roko-neuro` built. Knowledge types and tier system defined. HDC encoding via
  `roko-primitives`. Integration with prompt assembly wired.
- **Daimon**: `roko-daimon` built (972 lines, fully implemented). PAD vector, behavioral
  states, somatic markers. Integration with operating frequency selection wired.
- **Dreams**: `roko-dreams` scaffolded but not fully implemented. Three-phase cycle specified.
  Hypnagogia engine specified. NREM replay and REM imagination not yet shipping.
- **Gap**: Cross-cut interaction (Daimon ↔ Neuro ↔ Dreams) not yet fully wired.

---

## Cross-References

- [04-decay-variants.md](04-decay-variants.md) — Ebbinghaus decay in knowledge tiers
- [10-three-cognitive-speeds.md](10-three-cognitive-speeds.md) — Delta frequency triggers Dreams
- [11-dual-process-and-active-inference.md](11-dual-process-and-active-inference.md) — Daimon drives tier routing
- [12-five-layer-taxonomy.md](12-five-layer-taxonomy.md) — Cross-cuts injected across layers
