# OCC and Scherer Appraisal Theory

> How events become emotions: the appraisal pipeline that converts gate pass/fail, task outcomes, blockers, time pressure, and prediction accuracy into PAD vector updates.


> **Implementation**: Built

**Topic**: [Daimon](./INDEX.md)
**Prerequisites**: [01-pad-vector.md](./01-pad-vector.md), [02-alma-three-layer-temporal.md](./02-alma-three-layer-temporal.md)
**Key sources**: `bardo-backup/prd/03-daimon/01-appraisal.md`, `refactoring-prd/03-cognitive-subsystems.md` §2, `roko-daimon/src/lib.rs`

---

## Abstract

Appraisal theory holds that emotions are not random internal states but structured evaluations of events relative to goals. The OCC model (Ortony, Clore, & Collins 1988) established that emotions arise from appraising events (desirable/undesirable for goals), agents (praiseworthy/blameworthy), and objects (appealing/unappealing). Scherer's Component Process Model (2001) refined this into a sequential checking process: novelty → pleasantness → goal relevance → coping potential → norm compatibility. The Daimon implements a hybrid appraisal pipeline that draws from both theories: every `AffectEvent` is evaluated against the agent's current goals and capabilities, producing a PAD delta that updates the mood state.

The critical design constraint is **grounding**: every emotion must have a trigger, and every trigger must be grounded in a concrete metric. A gate pass is a measurable positive outcome. A task failure is a measurable negative outcome. Time pressure has a measurable deadline proximity in [0.0, 1.0]. No emotion is generated without an event that can be traced to a specific measurement. This constraint prevents the affect system from degenerating into emotional hallucination — the agent cannot "feel anxious" without a concrete reason.

---

## Theoretical Foundation

### OCC Model (Ortony, Clore, Collins 1988)

The OCC model classifies emotions based on what is being appraised:

| Appraisal Focus | Positive Valence | Negative Valence | Agent Mapping |
|---|---|---|---|
| **Events** (consequences for goals) | Joy, Hope, Relief | Distress, Fear, Disappointment | Gate results, task outcomes |
| **Agents** (actions relative to standards) | Pride, Admiration | Shame, Reproach | Self-evaluation of strategy quality |
| **Objects** (attributes of things) | Liking, Attraction | Disliking, Aversion | Code patterns, familiar vs. unfamiliar territory |

For the Daimon, the primary appraisal focus is **events** — did the action produce a good or bad outcome relative to the agent's task goals? Agent-focused appraisals (pride, shame) emerge indirectly through the Dominance dimension: successful strategies increase dominance (pride analog), while failed strategies decrease dominance (shame analog). Object-focused appraisals appear through the somatic landscape — familiar patterns that previously succeeded carry positive valence (attraction analog), while patterns associated with failure carry negative valence (aversion analog).

### Scherer's Component Process Model (2001)

Scherer proposed that appraisal proceeds through sequential checks, each adding information to the emotional evaluation:

| Check | Question | Daimon Implementation |
|---|---|---|
| **Novelty** | Is this event new or expected? | Prediction accuracy — was this outcome predicted? |
| **Intrinsic pleasantness** | Is this event inherently positive or negative? | Gate pass vs. fail, task success vs. failure |
| **Goal relevance** | Does this event matter for my goals? | Always relevant — every event is in the context of an active task |
| **Coping potential** | Can I handle this? | Dominance dimension — high D means "I can handle this" |
| **Norm compatibility** | Does this align with standards? | Code quality gates, style checks, test coverage thresholds |

The Daimon implements Scherer's checks implicitly through the appraisal rule structure. Novelty is captured by prediction error (handled by the Oracle/prediction system, see topic [05-learning](../05-learning/INDEX.md)). Pleasantness maps directly to the Pleasure delta. Goal relevance is assumed (all events arise from active tasks). Coping potential maps to the Dominance delta. Norm compatibility maps to gate rung levels (higher rungs = stricter norm checking).

---

## The Appraisal Pipeline

### Event → Appraisal → PAD Delta → Mood Update

The appraisal pipeline is a deterministic function from events to PAD deltas:

```
AffectEvent arrives
  │
  ▼
Step 1: CLASSIFY — identify event type (gate, task, blocker, time, queue, dream)
  │
  ▼
Step 2: GROUND — verify the event is grounded in a concrete metric
  │    (gate has a boolean pass/fail; task has a boolean success/fail;
  │     time pressure has a [0,1] proximity; blockers have a count)
  │
  ▼
Step 3: SCALE — compute magnitude based on event parameters
  │    (rung_scale = 1.0 + min(rung, 3) × 0.15 for gate events;
  │     blocker_scale = max(1, min(5, n)) for blocked events)
  │
  ▼
Step 4: COMPUTE DELTA — apply appraisal rules to produce PAD delta
  │    (pleasure, arousal, dominance, confidence adjustments)
  │
  ▼
Step 5: DECAY — apply temporal decay to current mood before adding delta
  │    (factor = 0.5 ^ (elapsed_hours / half_life_hours))
  │
  ▼
Step 6: APPLY — add delta to current mood with clamping to [-1, 1]
  │
  ▼
Step 7: PERSIST — autosave updated state to disk
  │
  ▼
Step 8: EMIT — if PAD change exceeds threshold (0.15 Euclidean),
         emit a MoodUpdate event for connected clients
```

### The Eight-Step Pipeline in the Legacy Specification

The original `bardo-backup/prd/03-daimon/01-appraisal.md` specified an 8-step appraisal pipeline that the runtime daimon extension executed every tick:

1. **Collect observation data** — read the latest observation from the heartbeat
2. **Extract appraisal triggers** — identify which events in this tick require appraisal
3. **Apply OCC/Scherer rule-based appraisal** — convert triggers to PAD deltas
4. **Apply Chain-of-Emotion reasoning** — for complex situations, use structured LLM reasoning (optional, T2 only)
5. **Update PAD vector** — apply deltas with clamping
6. **Classify Plutchik emotion** — map updated PAD to nearest Plutchik category
7. **Check somatic markers** — query the somatic landscape for historical valence of similar situations
8. **Write CorticalState** — commit the updated affect state to the shared perception surface

In the new Roko architecture, steps 1-3 are implemented in `AffectEngine::appraise()`. Step 4 (Chain-of-Emotion) is deferred — it requires an LLM call and is only warranted for T2 situations with high ambiguity. Steps 5-6 are handled by `apply_delta()` and `AffectOctant::from_pad()`. Step 7 is specified but not yet wired (see [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md)). Step 8 maps to the persistence mechanism in `autosave()`.

---

## Appraisal Rule Set

### Gate Results

Gate evaluations are the most frequent appraisal trigger. Each gate result (pass or fail) at a specific rung produces a scaled PAD delta:

```rust
AffectEvent::GateResult { plan_id, task_id, passed, rung } => {
    let rung_scale = 1.0 + (rung.min(3) as f64 * 0.15);
    if passed {
        // Satisfaction: pleasure increases, slight arousal relief,
        // dominance and confidence increase
        delta = (0.05 * rs, -0.01 * rs, 0.03 * rs, 0.03 * rs)
    } else {
        // Disappointment: pleasure decreases strongly, arousal increases,
        // dominance and confidence decrease
        delta = (-0.10 * rs, 0.04 * rs, -0.08 * rs, -0.08 * rs)
    }
}
```

**Rung scaling rationale**: A rung-0 gate (compile only) carries less emotional weight than a rung-3 gate (compile + test + clippy + diff review + symbol check + LLM judge). The scale factor ranges from 1.0 (rung 0) to 1.45 (rung 3), making higher-rung outcomes approximately 45% more emotionally significant.

**Asymmetry rationale**: Gate failures have 2× the pleasure impact of gate passes. This follows prospect theory (Kahneman & Tversky 1979) — losses loom larger than gains. For agents, this means a failed gate produces more behavioral change than a passed gate, which matches engineering reality: a broken build demands immediate attention while a clean build is the expected baseline.

### Task Outcomes

Task completion (success or failure) produces the largest emotional impact:

```rust
AffectEvent::TaskOutcome { task_id, succeeded } => {
    if succeeded {
        // Achievement: strong pleasure and dominance boost
        delta = (0.10, 0.00, 0.10, 0.08)
    } else {
        // Significant setback: strong negative pleasure and dominance
        delta = (-0.20, 0.00, -0.15, -0.15)
    }
}
```

Task outcomes do not affect arousal directly. Arousal tracks urgency and load, which are driven by time pressure and blockers, not by success/failure. A successful task doesn't reduce urgency — there may still be many tasks remaining under a tight deadline.

### Blockers

Being blocked on dependencies or safety gates raises arousal (urgency) and lowers dominance (sense of control):

```rust
AffectEvent::Blocked { task_id, blocker_count } => {
    let n = blocker_count.max(1).min(5) as f64;
    delta = (0.0, n * 0.05, -(n * 0.08), -0.02 * n)
}
```

The blocker count is capped at 5 to prevent extreme states from many simultaneous blockers. Each additional blocker has diminishing relative impact but cumulative absolute impact.

### Time Pressure

Deadline proximity is a pure arousal signal:

```rust
AffectEvent::TimePressure { task_id, deadline_proximity } => {
    let proximity = deadline_proximity.clamp(0.0, 1.0);
    delta = (0.0, proximity * 0.40, 0.0, 0.0)
}
```

At proximity = 1.0 (deadline imminent), arousal jumps by 0.40 — a significant escalation that will trigger tier routing changes (higher arousal → lower T2 threshold → route to stronger models). At proximity = 0.1 (deadline far away), the impact is only 0.04 — a negligible signal.

### Queue Wait

Work that has been waiting in a queue generates increasing arousal:

```rust
AffectEvent::QueueWait { task_id, wait_hours } => {
    let bump = if wait_hours <= 24.0 { 0.0 }
               else if wait_hours > 24.0 * 7.0 { 1.0 }
               else { ((wait_hours - 24.0) / 24.0 * 0.1).clamp(0.0, 1.0) };
    delta = (0.0, bump, 0.0, 0.0)
}
```

No arousal for work less than 24 hours old. After 24 hours, arousal ramps up by 0.1 per day. After 7 days, arousal saturates at maximum — the signal means "do this or drop it."

### Dream Failure

When the dream consolidation process reviews episodes and finds repeated failures in a task type, it lowers confidence without affecting the PAD vector:

```rust
AffectEvent::DreamFailure { task_type, failure_count } => {
    let failures = failure_count.max(1).min(5) as f64;
    let confidence_drop = -(0.07 * failures).min(0.35);
    delta = (0.0, 0.0, 0.0, confidence_drop)
}
```

Dream failures affect confidence, not pleasure or arousal, because dream consolidation is a reflective process (Theta/Delta frequency), not a reactive one (Gamma frequency). The agent isn't "feeling bad" about dream discoveries — it's updating its self-model of capability.

---

## Future Appraisal Triggers

The following triggers are specified in the legacy PRD but not yet implemented in `roko-daimon`:

| Trigger | Source | PAD Effect | Status |
|---|---|---|---|
| **Prediction accuracy** | Oracle/CalibrationTracker | Accurate → +D; Inaccurate → -D, +A | Not implemented (depends on Tier 2J) |
| **Peer comparison** | C-Factor metrics | Outperforming → +P, +D; Underperforming → -P | Not implemented (depends on Tier 2M) |
| **Novel domain entry** | Context assembly detects unfamiliar crate/module | -D (low confidence in new territory) | Not implemented |
| **Repeated pattern success** | Playbook/skill library match | +D (confidence in known approach) | Not implemented |
| **Knowledge contradiction** | AntiKnowledge challenges existing knowledge | -D, +A (uncertainty spike) | Not implemented (depends on Tier 2A) |

---

## Academic Foundations

- Ortony, A., Clore, G.L., & Collins, A. (1988). *The Cognitive Structure of Emotions*. Cambridge University Press.
- Scherer, K.R. (2001). "Appraisal considered as a process of multilevel sequential checking." In Scherer, Schorr, & Johnstone (Eds.), *Appraisal Processes in Emotion*. Oxford University Press.
- Kahneman, D. & Tversky, A. (1979). "Prospect Theory: An Analysis of Decision under Risk." *Econometrica*, 47(2), 263–291.
- Mehrabian, A. (1996). "Pleasure-arousal-dominance: A general framework for describing and measuring individual differences in temperament." *Current Psychology*, 14(4), 261–292.
- Shinn, N. et al. (2023). "Reflexion: Language Agents with Verbal Reinforcement Learning." *NeurIPS*.
- Bechara, A. et al. (1994). "Insensitivity to future consequences following damage to human prefrontal cortex." *Cognition*, 50, 7–15.
- Bechara, A., Damasio, H., Tranel, D., & Damasio, A.R. (1997). "Deciding advantageously before knowing the advantageous strategy." *Science*, 275(5304), 1293–1295.

---

## Current Status and Gaps

**Implemented**: Full appraisal pipeline for gate results, task outcomes, blockers, time pressure, queue waits, and dream failures in `roko-daimon/src/lib.rs`. Rung scaling, asymmetric valence, temporal decay before delta application — all implemented and tested.

**Gaps**: Chain-of-Emotion reasoning (LLM-assisted appraisal for complex/ambiguous situations) not implemented. Somatic marker check (step 7) not wired. Prediction accuracy, peer comparison, novel domain entry, repeated pattern success, and knowledge contradiction triggers not implemented. The legacy 8-step pipeline from `01-appraisal.md` is partially implemented — steps 1-3, 5-6 are done, steps 4, 7-8 are deferred.

---

## Cross-References

- See [01-pad-vector.md](./01-pad-vector.md) for PAD vector structure and octant classification
- See [02-alma-three-layer-temporal.md](./02-alma-three-layer-temporal.md) for how deltas interact with the three temporal layers
- See [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md) for somatic marker check in the pipeline
- See topic [05-learning](../05-learning/INDEX.md) for prediction accuracy as future appraisal trigger
- See topic [04-verification](../04-verification/INDEX.md) for gate rung levels
