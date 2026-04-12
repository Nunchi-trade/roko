# Vision and Mortality Incompatibility

> The Daimon is a cognitive performance affect engine — NOT a mortality anxiety system. This document explains what Daimon is, what it is not, and why the new architecture explicitly removes mortality framing.


> **Implementation**: Built

**Topic**: [Daimon](./INDEX.md)
**Prerequisites**: [00-architecture](../00-architecture/INDEX.md) for Engrams, Synapse traits, universal cognitive loop
**Key sources**: `refactoring-prd/03-cognitive-subsystems.md` §2, `refactoring-prd/08-translation-guide.md` §INCOMPATIBLE: Emotion Mapped to Mortality, `refactoring-prd/07-implementation-priorities.md` §Tier 2D/2E

---

## Abstract

The Daimon (`roko-daimon`) is Roko's affect engine — a subsystem that models the agent's internal cognitive state using a PAD (Pleasure-Arousal-Dominance) vector (Mehrabian 1996, Current Psychology 14(4)). It provides a fast System 1 routing mechanism that modulates tier routing, context bidding, risk tolerance, and behavioral state. The Daimon is architecturally structural: it is not decoration, not cosmetic personality, and not mortality anxiety. It is the mechanism by which agents adapt their compute investment, exploration rate, and decision strategy based on their accumulated experience.

The Daimon exists because emotion — properly understood as appraisal-driven motivational state — is a necessary control signal for intelligent systems. Damasio's somatic marker hypothesis (1994) demonstrated that patients with prefrontal cortex damage who lost emotional processing became unable to make effective decisions despite retaining full intellectual capacity. The same principle applies to agents: without an affect layer, an agent that has failed five consecutive tasks treats the sixth identically to the first. The Daimon ensures that accumulated failure lowers pleasure and confidence, biasing the agent toward stronger models, more cautious strategies, and eventually re-planning — precisely the adaptive behavior that distinguishes intelligent systems from brittle ones.

This document establishes the critical architectural boundary: the Daimon in Roko tracks **cognitive performance**, not mortality. Every reference to death, dying, mortality phases, terminal states, or existential anxiety from the legacy architecture is explicitly rejected.

---

## What Daimon IS

The Daimon is a **cognitive performance affect engine** with four concrete functions:

### 1. PAD State Tracking

The Daimon maintains a three-dimensional PAD (Pleasure-Arousal-Dominance) vector, each dimension in [-1.0, 1.0], updated by concrete appraisal triggers:

| Dimension | Low Value Meaning | High Value Meaning | Agent Interpretation |
|---|---|---|---|
| **Pleasure** | Tasks failing, unexpected errors, gate failures | Tasks succeeding, clean passes, accurate predictions | Outcome quality trajectory |
| **Arousal** | Idle, routine tasks, no pressure | High-stakes, urgent, many failures, deadline proximity | Cognitive load and urgency |
| **Dominance** | Uncertain, exploring, low confidence, stuck | Confident, executing known patterns, in control | Confidence in approach |

The PAD model originates from Mehrabian (1996, Current Psychology 14(4)), with the foundational psychometric work in Russell & Mehrabian (1977). The three dimensions are orthogonal and their combination defines 8 octant states (see [01-pad-vector.md](./01-pad-vector.md)).

### 2. Behavioral State Classification

The PAD vector maps to six named behavioral states that are **cyclical and non-terminal**:

- **Engaged** — balanced PAD, standard operation
- **Struggling** — low Pleasure, high Arousal → escalate model, re-plan, request help
- **Coasting** — high Pleasure, low Arousal → use cheaper models, take on more tasks
- **Exploring** — low Dominance → research mode, exploration budget
- **Focused** — high Dominance, high Pleasure → exploit known patterns, maximum speed
- **Resting** — low Arousal, low Dominance → offline learning, consolidation (Dreams)

These states cycle freely: an agent can transition from any state to any other state based on task outcomes. There is no terminal state. There is no death destination. The cycle is: work → succeed or fail → adapt → work again.

### 3. Compute Allocation Control

The Daimon's behavioral state directly modulates the CascadeRouter's tier routing. This is the concrete mechanism by which affect controls compute investment:

- **Struggling agents** have a lower T2 trigger threshold → route to deep reasoning (stronger models) sooner
- **Coasting agents** have a higher T2 trigger threshold → stay on cheap models longer
- **Focused agents** exploit T0/T1 cached strategies → minimal compute waste
- **Exploring agents** allocate T2 budget for research → invest in knowledge acquisition

This is not sentiment analysis applied to an agent's chat messages. It is a control signal that directly modulates infrastructure decisions (which model to call, how many turns to allow, whether to re-plan).

### 4. Somatic Fast-Path Decision Making

Before analytical reasoning, the Daimon queries a k-d tree of past strategy outcomes (the Somatic Landscape) to provide sub-millisecond "gut feeling" heuristics. This implements Damasio's somatic marker hypothesis in software: emotions mark past decisions with valence, and that valence biases future similar decisions without requiring full re-analysis. See [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md) for the full mechanism.

---

## What Daimon is NOT

### NOT Mortality Anxiety

The legacy architecture (`bardo-backup/prd/03-daimon/`) framed the Daimon as a **mortality-aware affect system**. Agents experienced "economic anxiety" as their budget depleted, "epistemic vertigo" as their knowledge decayed, and "stochastic dread" as random death approached. The affect system was coupled to three mortality clocks, and the emotional trajectory moved inexorably toward a terminal state.

**This framing is explicitly removed.** The new Roko architecture has no natural death, no mortality clocks, no terminal state, no thanatopsis phase, no death protocol, no succession mechanism.

The translation guide (`refactoring-prd/08-translation-guide.md` §INCOMPATIBLE: Emotion Mapped to Mortality) states:

> **Old**: Daimon PAD vector had mortality-specific mappings. Fear increased as death approached. Joy was highest in "Thriving" phase. Mortality directly modulated emotional state.
>
> **New**: Daimon PAD vector tracks cognitive performance. Pleasure = task success. Arousal = urgency/load. Dominance = confidence. No mortality input.
>
> **Action**: Keep PAD vector math, Plutchik classification, somatic markers. Remove mortality as an input signal. Add task performance, gate results, and prediction accuracy as input signals instead.

### NOT Cosmetic Personality

Many agent frameworks add "personality" as a prompt injection — a static system message that says "You are friendly and helpful" or "You are analytical and precise." The Daimon is architecturally different. It does not inject personality descriptions; it modulates infrastructure parameters. The difference between a cosmetic personality and the Daimon is the difference between telling someone to "act confident" versus actually measuring their success rate and adjusting their decision-making strategy based on accumulated outcomes.

### NOT Vitality Phases

The legacy architecture had five behavioral phases: Thriving → Stable → Conservation → Declining → Terminal. These were mortality-driven: as the agent's vitality composite decreased, it moved through phases toward inevitable death. The new architecture replaces these with six **cyclical** behavioral states that respond to task outcomes, not to a countdown timer. An agent in the Struggling state that succeeds on its next task transitions to Engaged or Focused — there is no one-way progression toward death.

### NOT an Input for Mortality Clocks

The legacy system had three mortality clocks (economic, epistemic, stochastic) that fed into the Daimon. In the new architecture, there are no mortality clocks. Urgency comes from external deadlines and budget constraints, not from intrinsic finite lifespan. The mechanism is identical — resource scarcity drives prioritization — but the source is practical (deadlines, budgets, task queues) rather than existential (approaching death).

---

## The Reframe Rationale

The research citations that support the Daimon are preserved without exception. Mehrabian's PAD model (1996), Damasio's somatic markers (1994), OCC appraisal theory (Ortony, Clore, & Collins 1988), Scherer's component process model (2001), Bower's mood-congruent memory (1981), Blaney's review (1986), Walker & van der Helm's emotional depotentiation in REM sleep (2009), Plutchik's emotion wheel (1980) — all of these are domain-agnostic findings about how affect influences cognition and decision-making. None of them require a mortality framing.

The mortality framing was a design choice in the legacy architecture, not a requirement of the research. Damasio's patients were not dying; they had prefrontal cortex damage that disrupted their emotional processing, leading to poor decision-making despite intact analytical capacity. The somatic marker hypothesis applies to any decision-making system, not specifically to dying systems.

The reframe preserves the mechanism (affect modulates behavior) and changes the input signals (task outcomes instead of mortality clocks) and removes the terminal destination (cyclical states instead of a death trajectory).

---

## Legacy Files Skipped

The following legacy source files are **not** carried forward into the new architecture:

| File | Reason |
|---|---|
| `bardo-backup/prd/03-daimon/04-mortality-daimon.md` | Mortality-specific emotions (economic anxiety, epistemic vertigo, stochastic dread). Citations extracted only. |
| `bardo-backup/prd/03-daimon/05-death-daimon.md` | Death protocol emotional processing. Skipped entirely. |
| `bardo-backup/prd/02-mortality/*` | Mortality clocks, vitality phases, thanatopsis, succession. All removed. |

**Citations extracted from skipped files**: Heidegger's Befindlichkeit (1927), Jonas's "needful freedom" — these philosophical foundations are acknowledged as informing the original design but are not required for the new cognitive performance framing.

---

## Academic Foundations

- Mehrabian, A. (1996). "Pleasure-arousal-dominance: A general framework for describing and measuring individual differences in temperament." *Current Psychology*, 14(4), 261–292.
- Russell, J.A. & Mehrabian, A. (1977). "Evidence for a three-factor theory of emotions." *Journal of Research in Personality*, 11, 273–294.
- Damasio, A.R. (1994). *Descartes' Error: Emotion, Reason, and the Human Brain*. Putnam.
- Ortony, A., Clore, G.L., & Collins, A. (1988). *The Cognitive Structure of Emotions*. Cambridge University Press.
- Scherer, K.R. (2001). "Appraisal considered as a process of multilevel sequential checking." In Scherer, Schorr, & Johnstone (Eds.), *Appraisal Processes in Emotion*. Oxford University Press.
- Plutchik, R. (1980). *Emotion: A Psychoevolutionary Synthesis*. Harper & Row.
- Bower, G.H. (1981). "Mood and Memory." *American Psychologist*, 36(2), 129–148.
- Walker, M.P. & van der Helm, E. (2009). "Overnight therapy? The role of sleep in emotional brain processing." *Psychological Bulletin*, 135(5), 731–748.
- Blaney, P.H. (1986). "Affect and memory: A review." *Psychological Bulletin*, 99(2), 229–246.

---

## Current Status and Gaps

The `roko-daimon` crate contains a working standalone affect engine (569 lines) with:
- `PadVector` struct with neutral(), apply_delta(), decay_by_factor()
- `AffectState` with PAD + confidence + temporal decay
- `AffectEngine` trait with appraise(), query(), modulate(), persist()
- `DaimonState` implementing the trait with full appraisal rules for gate results, task outcomes, blockers, time pressure, queue waits, and dream failures
- `DispatchStrategy` enum (Conservative, Balanced, Exploratory, Escalating, Proactive)
- `DispatchParams` modulation (model promotion/demotion, turn limit adjustment)
- Persistence to `.roko/daimon/affect.json`

The `roko-golem/src/daimon.rs` (972 lines) contains a more elaborate per-task affect engine with:
- `AffectOctant` enum (8 named octants from PAD signs)
- `AffectBehaviorModulation` struct with strategy, exploration_rate, model_tier_escalation
- Signal emission for significant state changes (confidence drops, valence extremes)

Per the `roko-golem` dissolution plan (Tier 0C), the golem implementation should be absorbed into `roko-daimon`. The behavioral state classification (6 states from refactoring-prd §2) is not yet wired — the current code uses 8 octants from the PAD model directly, plus 5 dispatch strategies. The six behavioral states from the refactoring PRD (Engaged, Struggling, Coasting, Exploring, Focused, Resting) map to PAD regions and need to be added as a higher-level classification on top of the octants.

See `tmp/implementation-plans/12a-cognitive-layer.md` §F (F1–F9) for the full wiring plan.

---

## Cross-references

- See [01-pad-vector.md](./01-pad-vector.md) for the full PAD model specification
- See [04-six-behavioral-states.md](./04-six-behavioral-states.md) for the cyclical behavioral states
- See [13-current-status-and-gaps.md](./13-current-status-and-gaps.md) for implementation status
- See topic [00-architecture](../00-architecture/INDEX.md) for the universal loop META-COGNIZE step
- See topic [02-agents](../02-agents/INDEX.md) for behavioral state → tier routing
- See topic [10-dreams](../10-dreams/INDEX.md) for dream-daimon reframed
