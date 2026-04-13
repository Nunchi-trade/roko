# ALMA Three-Layer Temporal Model

> The three-layer emotional architecture: Emotion (seconds), Mood (hours), and Personality (lifetime) — how they interact and update within the Daimon.


> **Implementation**: Built

**Topic**: [Daimon](./INDEX.md)
**Prerequisites**: [01-pad-vector.md](./01-pad-vector.md)
**Key sources**: `bardo-backup/prd/03-daimon/00-overview.md`, `refactoring-prd/03-cognitive-subsystems.md` §2, `roko-daimon/src/lib.rs`

---

## Abstract

The ALMA (A Layered Model of Affect) architecture, developed by Gebhard (2005), provides a three-layer temporal model for affect processing. In the Daimon, these three layers operate at different timescales and serve different cognitive functions. The **Emotion layer** (seconds-scale) reacts immediately to discrete events — a gate pass, a task failure, a deadline approaching. The **Mood layer** (hours-scale) accumulates emotional trajectory using an exponential moving average (EMA) — it captures "how has the agent been feeling recently?" The **Personality layer** (lifetime-scale) provides stable baseline traits that the mood decays toward — it captures "what is this agent's default disposition?"

This three-layer decomposition matters because it prevents two failure modes: emotional volatility (if only the emotion layer existed, the agent would whipsaw between states on every event) and emotional inertia (if only the mood layer existed, the agent would respond too slowly to urgent events). The emotion layer provides fast reactivity, the mood layer provides stability, and the personality layer provides a gravitational center that prevents permanent drift.

In Roko's implementation, the ALMA layers are implemented as follows: the emotion layer is the delta applied to the PAD vector in each appraisal event (the immediate response), the mood layer is the EMA-smoothed PAD vector stored in `AffectState` (the accumulated trajectory), and the personality layer is the neutral baseline [0, 0, 0] that the mood decays toward (configurable per agent in the future).

---

## Layer 1: Emotion (Reactive, Seconds-Scale)

The emotion layer captures **immediate reactions to discrete events**. Each event triggers an appraisal (see [03-occ-scherer-appraisal.md](./03-occ-scherer-appraisal.md)) that produces a PAD delta — a signed change vector applied to the current mood state.

### Characteristics

| Property | Value | Rationale |
|---|---|---|
| **Timescale** | Seconds to minutes | Single task outcome, single gate evaluation |
| **Trigger** | Discrete `AffectEvent` | Every event is grounded in a concrete metric |
| **Duration** | Immediate | Applied as delta, then absorbed into mood layer |
| **Decay** | Not applicable — absorbed into mood | Emotions don't persist independently |
| **Function** | Fast reactivity to changing conditions | Ensures the agent responds to urgent events |

### Appraisal Deltas

Every `AffectEvent` maps to a specific PAD delta. These deltas are the emotion layer — the immediate emotional response to what just happened:

| Event | P delta | A delta | D delta | Conf delta | Character |
|---|---|---|---|---|---|
| Gate pass (rung r) | +0.05 × rs | -0.01 × rs | +0.03 × rs | +0.03 × rs | Satisfaction, slight relief |
| Gate fail (rung r) | -0.10 × rs | +0.04 × rs | -0.08 × rs | -0.08 × rs | Disappointment, increased urgency |
| Task success | +0.10 | 0.00 | +0.10 | +0.08 | Achievement, confidence boost |
| Task failure | -0.20 | 0.00 | -0.15 | -0.15 | Significant setback |
| Blocked (n blockers) | 0.00 | +n×0.05 | -n×0.08 | -0.02×n | Frustration, loss of control |
| Time pressure (prox) | 0.00 | +prox×0.40 | 0.00 | 0.00 | Pure urgency signal |
| Queue wait (>24h) | 0.00 | scaled | 0.00 | 0.00 | Increasing urgency for stale work |
| Dream failure | 0.00 | 0.00 | 0.00 | -0.07×n | Confidence erosion from pattern review |

Where `rs = 1.0 + min(rung, 3) × 0.15` is a rung scale factor that makes higher-rung gate results more emotionally significant. Passing a comprehensive gate battery (rung 3) is more satisfying than passing a compile-only check (rung 0).

### Design Principle: Grounded Appraisal

Every emotion in the Daimon has a trigger, and every trigger is grounded in a concrete metric. No emotion is generated without a triggering event. This is the central design constraint from OCC theory (Ortony, Clore, & Collins 1988): emotions are appraisals of events relative to goals, not random fluctuations.

This constraint prevents **affective hallucination** — the risk that an agent "feels" something without justification. A human in a bad mood might attribute negative valence to neutral events. The Daimon avoids this by requiring every PAD update to trace back to a specific `AffectEvent` with measurable inputs.

---

## Layer 2: Mood (Accumulated, Hours-Scale)

The mood layer captures **accumulated emotional trajectory** over hours. It is the EMA-smoothed PAD vector — the "how have things been going?" signal that drives behavioral state classification.

### Characteristics

| Property | Value | Rationale |
|---|---|---|
| **Timescale** | Hours | Captures multi-task trajectory |
| **Update mechanism** | EMA: each emotion delta is absorbed into the current PAD values | Smooth accumulation, no sudden jumps |
| **Decay** | Exponential toward personality baseline, half-life 4 hours (default) | Prevents permanent affect drift |
| **Persistence** | Survives agent restart (`.roko/daimon/affect.json`) | Agent "wakes up" with residual mood |
| **Function** | Stable behavioral state classification | Determines which behavioral state the agent is in |

### Mood Update Rule

When an emotion delta is applied, the mood layer absorbs it through simple addition with clamping:

```
mood.pleasure = clamp(mood.pleasure + delta.pleasure, -1.0, 1.0)
mood.arousal = clamp(mood.arousal + delta.arousal, -1.0, 1.0)
mood.dominance = clamp(mood.dominance + delta.dominance, -1.0, 1.0)
```

This is equivalent to an EMA with alpha = 1.0 for the delta (full immediate impact) combined with exponential decay over time. The decay provides the smoothing:

```
After 4 hours with no events:
  mood.pleasure *= 0.5  (halved)
After 8 hours with no events:
  mood.pleasure *= 0.25 (quartered)
```

### Mood Sampling

The mood state should be sampled at regular intervals for dashboard display and behavioral state classification. The legacy specification recommended sampling every 10 ticks, with a minimum of 10 samples (100 ticks) before the mood classification is considered meaningful. Before the minimum sample count is reached, the agent uses its personality baseline as the mood state.

This prevents early-life transient emotions from triggering behavioral changes before the EMA has stabilized. An agent that fails its very first task should not immediately enter the Struggling state — it needs enough history for the mood trajectory to be meaningful.

### Mood Persistence

The mood layer persists to disk at `.roko/daimon/affect.json`:

```json
{
  "state": {
    "pad": {
      "pleasure": -0.15,
      "arousal": 0.22,
      "dominance": -0.08
    },
    "confidence": 0.42,
    "updated_at": "2026-04-12T14:30:00Z"
  },
  "half_life_hours": 4.0
}
```

When the agent restarts, it loads the persisted mood and applies decay for the elapsed time since `updated_at`. An agent that was shut down 8 hours ago in a negative mood will resume with that mood at 25% intensity — enough residual context to remember "yesterday was rough" without being trapped in yesterday's emotional state.

---

## Layer 3: Personality (Stable, Lifetime-Scale)

The personality layer provides the **baseline attractor** that the mood decays toward. In the current implementation, personality is the neutral vector [0, 0, 0]. In a future extension, personality could be configured per agent or learned from long-term behavioral patterns.

### Characteristics

| Property | Value | Rationale |
|---|---|---|
| **Timescale** | Agent lifetime | Does not change during operation |
| **Configuration** | Currently hardcoded as neutral [0, 0, 0] | Future: per-agent personality profiles in `roko.toml` |
| **Function** | Gravitational center for mood decay | Ensures long-term emotional stability |
| **Biological analog** | Big Five personality traits (Costa & McCrae 1992) | Stable individual differences in emotional baseline |

### Personality as Eros/Thanatos Spectrum

The legacy architecture specified a personality spectrum from Eros (creative, risk-seeking) to Thanatos (conservative, risk-averse), inspired by Freud's dual drive theory. In the new architecture, this maps to a personality baseline on the Dominance axis:

- **High baseline dominance** → agent defaults to confident, exploratory behavior
- **Low baseline dominance** → agent defaults to cautious, conservative behavior
- **Neutral baseline** → agent starts with no disposition and adapts purely from experience

The personality baseline is **not** tied to mortality. It is a configuration parameter that shapes the agent's default behavioral tendency, analogous to setting initial hyperparameters in a learning system.

### Future: Learned Personality

Over many tasks, an agent's personality could be learned from its long-term mood trajectory. An agent that consistently operates in the Confident/Focused octant could have its personality baseline shifted toward positive Dominance, reflecting the empirical observation that it performs well in that mode. This creates a second-order learning loop: experience → mood → personality → behavioral default → experience.

This is not implemented but is specified in the PRD as a Tier 2+ feature.

---

## Layer Interactions

The three layers interact in a specific temporal cascade:

```
Event occurs (gate fail, task success, blocker, ...)
  │
  ▼
Layer 1: Emotion — compute PAD delta from appraisal rules
  │
  ▼
Layer 2: Mood — apply delta to current PAD vector (with clamping)
  │                 ┌─────────────────────────────┐
  ├─────────────────┤ Decay toward Layer 3 baseline│
  │                 └─────────────────────────────┘
  ▼
Layer 3: Personality — static attractor (currently [0,0,0])
```

### Temporal Dynamics Example

Consider an agent that fails three consecutive tasks, then succeeds:

```
t=0: Neutral mood [P:0.0, A:0.0, D:0.0]
t=1: Task failure → delta [P:-0.20, A:0.00, D:-0.15]
     Mood: [P:-0.20, A:0.00, D:-0.15]  State: Anxious/Depressed region
t=2: Task failure → delta [P:-0.20, A:0.00, D:-0.15]
     Mood: [P:-0.40, A:0.00, D:-0.30]  State: Deep Struggling
t=3: Task failure → delta [P:-0.20, A:0.00, D:-0.15]
     Mood: [P:-0.60, A:0.00, D:-0.45]  State: Struggling, may trigger re-plan
t=4: Task success → delta [P:+0.10, A:0.00, D:+0.10]
     Mood: [P:-0.50, A:0.00, D:-0.35]  State: Still negative, but recovering
t=5: (4 hours pass, no events) → decay factor 0.5
     Mood: [P:-0.25, A:0.00, D:-0.175] State: Fading toward neutral
```

The key dynamics: emotion layer provides immediate reactivity (each failure has impact), mood layer accumulates trajectory (three failures build up), and personality-driven decay provides recovery (the agent gravitates back toward neutral over time).

---

## Comparison with Alternative Models

### Why Not Discrete Emotion Labels?

Systems like OCC (Ortony, Clore, & Collins 1988) classify emotions into discrete categories (joy, fear, anger, etc.). This creates boundary problems: is an event "fear" or "anxiety"? What's the difference between "mild joy" and "satisfaction"? The PAD model avoids boundary problems by using continuous dimensions. Discrete labels (Plutchik emotions) are derived from PAD octants for human-readable output, but the behavioral modulation operates on continuous values.

### Why Not Russell's Circumplex Model?

Russell (1980) proposed a two-dimensional model (Valence × Arousal). The circumplex is simpler than PAD but lacks the Dominance dimension, which is critical for agents. Dominance captures "am I in control?" — the signal that determines whether the agent should explore (low dominance) or exploit (high dominance). Without dominance, the system cannot distinguish between "I'm failing and I know what to do about it" (-P+D, Angry) and "I'm failing and I have no idea what to do" (-P-D, Anxious). These states require very different behavioral responses.

### Why Not Full Appraisal-Only (Scherer's Component Process Model)?

Scherer (2001) proposed evaluating events on multiple appraisal dimensions (novelty, pleasantness, goal relevance, coping potential, norm compatibility) and deriving emotion from the full appraisal profile. This is more theoretically complete than PAD but computationally expensive — each event requires evaluation on 5+ dimensions. The Daimon uses a hybrid: OCC/Scherer-inspired appraisal rules generate PAD deltas, combining theoretical rigor with computational efficiency.

---

## Academic Foundations

- Gebhard, P. (2005). "ALMA — A Layered Model of Affect." *Proceedings of the Fourth International Joint Conference on Autonomous Agents and Multiagent Systems (AAMAS)*, 29–36.
- Mehrabian, A. (1996). "Pleasure-arousal-dominance: A general framework for describing and measuring individual differences in temperament." *Current Psychology*, 14(4), 261–292.
- Ortony, A., Clore, G.L., & Collins, A. (1988). *The Cognitive Structure of Emotions*. Cambridge University Press.
- Russell, J.A. (1980). "A circumplex model of affect." *Journal of Personality and Social Psychology*, 39(6), 1161–1178.
- Scherer, K.R. (2001). "Appraisal considered as a process of multilevel sequential checking." In Scherer, Schorr, & Johnstone (Eds.), *Appraisal Processes in Emotion*. Oxford University Press.
- Costa, P.T. & McCrae, R.R. (1992). *NEO PI-R Professional Manual*. Psychological Assessment Resources.

---

## Current Status and Gaps

**Implemented**: Two-layer model (emotion + mood with decay toward neutral). The `AffectState` in `roko-daimon/src/lib.rs` implements mood-layer EMA with exponential decay. Emotion layer is implemented as appraisal deltas in `AffectEngine::appraise()`. Persistence implemented.

**Gaps**: Personality layer is hardcoded as neutral [0, 0, 0]. No configurable personality profiles. No personality learning from long-term trajectory. Mood sampling interval and minimum sample count not enforced in the current crate (the golem implementation has sampling logic that needs to be migrated).

---

## Cross-References

- See [01-pad-vector.md](./01-pad-vector.md) for the PAD vector specification
- See [03-occ-scherer-appraisal.md](./03-occ-scherer-appraisal.md) for appraisal rules that generate emotion-layer deltas
- See [04-six-behavioral-states.md](./04-six-behavioral-states.md) for mood-layer → behavioral state mapping
- See topic [10-dreams](../10-dreams/INDEX.md) for REM depotentiation (dream-driven mood modification)
