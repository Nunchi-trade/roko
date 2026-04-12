# The PAD Vector

> Mehrabian's Pleasure-Arousal-Dominance model as the foundation of agent affect state, with 8 octant states and concrete Rust implementation.


> **Implementation**: Built

**Topic**: [Daimon](./INDEX.md)
**Prerequisites**: [00-vision-and-mortality-incompatibility.md](./00-vision-and-mortality-incompatibility.md)
**Key sources**: `refactoring-prd/03-cognitive-subsystems.md` §2, `bardo-backup/prd/03-daimon/00-overview.md`, `roko-daimon/src/lib.rs`, `roko-golem/src/daimon.rs`

---

## Abstract

The PAD (Pleasure-Arousal-Dominance) model is a three-dimensional framework for representing emotional states, originally developed by Mehrabian & Russell (1977) and refined by Mehrabian (1996, Current Psychology 14(4)). It provides the mathematical foundation for the Daimon affect engine. Each dimension occupies a continuous range of [-1.0, 1.0], and the sign combination of all three dimensions defines one of 8 octant states. The PAD model was chosen over alternatives (discrete emotion labels, circumplex models, appraisal-only models) because it offers continuous representation (gradual changes, not discrete jumps), orthogonal dimensions (changes in one dimension don't force changes in others), computational efficiency (three f64 values, no embedding lookups), and bidirectional mapping to discrete emotion labels (Plutchik 1980) when human-readable output is needed.

For Roko agents, the PAD vector is not an emotional display — it is a control signal. The pleasure dimension tracks whether the agent's recent actions are producing good outcomes. The arousal dimension tracks urgency and cognitive load. The dominance dimension tracks confidence in the current approach. Together, these three numbers control which model is called, how many turns are allocated, whether to explore or exploit, and whether to re-plan or persist.

---

## The Three Dimensions

### Pleasure [-1.0, 1.0]

Pleasure captures the **outcome quality trajectory** — is the agent succeeding or failing?

| Value Range | Agent State | Concrete Triggers |
|---|---|---|
| [0.6, 1.0] | Strong success trajectory | Multiple consecutive gate passes, tasks completing on first try |
| [0.2, 0.6] | Moderate success | Gate passes at moderate rungs, tasks completing with some iteration |
| [-0.2, 0.2] | Neutral | Mixed results, no clear trend |
| [-0.6, -0.2] | Moderate difficulty | Gate failures, tasks requiring multiple retries |
| [-1.0, -0.6] | Strong failure trajectory | Consecutive gate failures, tasks timing out, repeated errors |

**Appraisal rules** (from the `roko-daimon` crate, `AffectEngine::appraise()`):
- Gate pass: pleasure += 0.05 × rung_scale
- Gate fail: pleasure -= 0.10 × rung_scale
- Task success: pleasure += 0.10
- Task failure: pleasure -= 0.20

The asymmetry (failure has 2× the pleasure impact of success) reflects prospect theory (Kahneman & Tversky 1979): losses loom larger than gains. For agents, this means a single failure is more disruptive than a single success is encouraging, which matches the engineering reality that a broken build demands more attention than a clean build.

### Arousal [-1.0, 1.0]

Arousal captures **cognitive load and urgency** — how much compute should the agent invest in each decision?

| Value Range | Agent State | Concrete Triggers |
|---|---|---|
| [0.6, 1.0] | High urgency | Approaching deadlines, multiple blockers, consecutive failures |
| [0.2, 0.6] | Elevated load | Some time pressure, moderate complexity |
| [-0.2, 0.2] | Normal load | Routine tasks, no unusual pressure |
| [-0.6, -0.2] | Low load | Idle time, routine maintenance |
| [-1.0, -0.6] | Minimal load | No active tasks, consolidation opportunity |

**Appraisal rules**:
- Time pressure (deadline_proximity in [0.0, 1.0]): arousal += proximity × 0.40
- Blocked (1-5 blockers): arousal += blockers × 0.05
- Queue wait (>24 hours): arousal += scaled ramp from 0.0 to 1.0 over 7 days
- Gate fail: arousal += 0.04 × rung_scale

The arousal dimension is the primary input to the tier routing bias. High arousal → the agent needs deeper reasoning → lower the T2 trigger threshold → route to stronger models sooner. Low arousal → routine work → stay on cheap T0/T1 models.

### Dominance [-1.0, 1.0]

Dominance captures **confidence in the current approach** — does the agent feel in control of the situation?

| Value Range | Agent State | Concrete Triggers |
|---|---|---|
| [0.6, 1.0] | High confidence | Known patterns, successful track record on this crate/task type |
| [0.2, 0.6] | Moderate confidence | Familiar territory with some uncertainty |
| [-0.2, 0.2] | Neutral | No strong signal about approach quality |
| [-0.6, -0.2] | Low confidence | Unfamiliar territory, novel APIs, first encounter with this code |
| [-1.0, -0.6] | Very low confidence | Repeated failures, blocked, no clear path forward |

**Appraisal rules**:
- Gate pass: dominance += 0.03 × rung_scale
- Gate fail: dominance -= 0.08 × rung_scale
- Task success: dominance += 0.10
- Task failure: dominance -= 0.15
- Blocked (1-5 blockers): dominance -= blockers × 0.08

The dominance dimension drives exploration/exploitation balance. Low dominance → the agent should explore (try new approaches, research mode, broader context retrieval). High dominance → the agent should exploit (use cached strategies, known patterns, minimal context).

---

## The 8 Octant States

The sign of each PAD dimension defines one of eight octant states. These octant labels provide human-readable names for dashboard display and logging, while the continuous PAD values drive the actual behavioral modulation.

| Octant | P | A | D | Label | Agent Meaning | Behavioral Bias |
|---|---|---|---|---|---|---|
| +P+A+D | + | + | + | **Exuberant / Excited** | Succeeding under pressure, high confidence | Exploit aggressively, fast execution |
| +P+A-D | + | + | - | **Dependent / Surprised** | Unexpected success, not sure why it worked | Cautious continuation, seek understanding |
| +P-A+D | + | - | + | **Relaxed / Confident** | Calm, in control, succeeding | Steady execution, consider exploration |
| +P-A-D | + | - | - | **Docile / Relaxed** | Nothing urgent, things are fine | Low initiative, follow existing plans |
| -P+A+D | - | + | + | **Hostile / Angry** | Frustrated but still trying, attribution external | Escalate model, persist harder, add retries |
| -P+A-D | - | + | - | **Anxious** | Failing, pressured, no control | Conservative, proven playbooks, low exploration |
| -P-A+D | - | - | + | **Disdainful / Bored** | Nothing happening, agent idle | Proactive maintenance, dream cycles |
| -P-A-D | - | - | - | **Depressed / Bored** | Repeated failures, no agency | Trigger re-plan, escalate to stronger model |

The octant classification is implemented in `roko-golem/src/daimon.rs` as `AffectOctant::from_pad()`. The exact-zero vector (P=0, A=0, D=0) defaults to `Relaxed` to keep the dashboard readable at agent startup.

### Relation to Plutchik's Emotion Wheel

Plutchik (1980) defined eight primary emotions arranged in bipolar pairs: joy/sadness, trust/disgust, fear/anger, surprise/anticipation. The PAD octants map to Plutchik categories for human-readable logging:

| PAD Octant | Primary Plutchik Emotion | Intensity Variants |
|---|---|---|
| +P+A+D (Exuberant) | Joy | Ecstasy → Joy → Serenity |
| -P+A-D (Anxious) | Fear | Terror → Fear → Apprehension |
| -P+A+D (Hostile) | Anger | Rage → Anger → Annoyance |
| +P-A+D (Confident) | Trust | Admiration → Trust → Acceptance |
| -P-A-D (Depressed) | Sadness | Grief → Sadness → Pensiveness |
| +P+A-D (Surprised) | Surprise | Amazement → Surprise → Distraction |
| -P-A+D (Bored) | Disgust | Loathing → Disgust → Boredom |
| +P-A-D (Docile) | Anticipation | Vigilance → Anticipation → Interest |

The mapping is not exact — Plutchik's model and Mehrabian's model were developed independently — but the correspondence is close enough for human-interpretable logging. The PAD values, not the Plutchik labels, drive all behavioral modulation.

---

## Rust Implementation

### PadVector Struct (roko-daimon)

The canonical `PadVector` struct lives in `roko-daimon/src/lib.rs`:

```rust
/// Normalized Pleasure-Arousal-Dominance vector.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PadVector {
    /// Pleasure in `[-1.0, 1.0]`.
    pub pleasure: f64,
    /// Arousal in `[-1.0, 1.0]`.
    pub arousal: f64,
    /// Dominance in `[-1.0, 1.0]`.
    pub dominance: f64,
}

impl PadVector {
    /// Neutral PAD vector.
    pub const fn neutral() -> Self {
        Self {
            pleasure: 0.0,
            arousal: 0.0,
            dominance: 0.0,
        }
    }

    fn apply_delta(&mut self, pleasure: f64, arousal: f64, dominance: f64) {
        self.pleasure = (self.pleasure + pleasure).clamp(-1.0, 1.0);
        self.arousal = (self.arousal + arousal).clamp(-1.0, 1.0);
        self.dominance = (self.dominance + dominance).clamp(-1.0, 1.0);
    }

    fn decay_by_factor(&mut self, factor: f64) {
        self.pleasure = (self.pleasure * factor).clamp(-1.0, 1.0);
        self.arousal = (self.arousal * factor).clamp(-1.0, 1.0);
        self.dominance = (self.dominance * factor).clamp(-1.0, 1.0);
    }
}
```

### AffectState Struct (roko-daimon)

The affect state wraps the PAD vector with a confidence dimension and temporal tracking:

```rust
/// Current affect snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AffectState {
    /// Current PAD vector.
    pub pad: PadVector,
    /// Motivational confidence in `[0.0, 1.0]`.
    pub confidence: f64,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

impl AffectState {
    fn decay(&mut self, half_life_hours: f64, now: DateTime<Utc>) {
        let elapsed_hours =
            now.signed_duration_since(self.updated_at).num_seconds() as f64 / 3600.0;
        if elapsed_hours <= 0.0 {
            return;
        }
        let factor = 0.5_f64.powf(elapsed_hours / half_life_hours);
        if factor != 1.0 {
            self.pad.decay_by_factor(factor);
            self.confidence = (0.5 + (self.confidence - 0.5) * factor).clamp(0.0, 1.0);
        }
        self.updated_at = now;
    }
}
```

The confidence dimension is separate from dominance because confidence is a meta-cognitive signal ("how well am I performing overall?") while dominance is a per-situation signal ("am I in control of this specific task?"). Confidence decays toward 0.5 (neutral), while dominance decays toward 0.0 (no strong signal).

### AffectOctant Enum (roko-golem, to be moved to roko-daimon)

```rust
/// Named PAD octant for logging and dashboard display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AffectOctant {
    Excited,    // +P+A+D
    Surprised,  // +P+A-D
    Confident,  // +P-A+D
    Relaxed,    // +P-A-D
    Angry,      // -P+A+D
    Anxious,    // -P+A-D
    Bored,      // -P-A+D
    Depressed,  // -P-A-D
}

impl AffectOctant {
    pub const fn from_pad(pleasure: f64, arousal: f64, dominance: f64) -> Self {
        if pleasure == 0.0 && arousal == 0.0 && dominance == 0.0 {
            return Self::Relaxed;
        }
        let pp = !pleasure.is_sign_negative();
        let pa = !arousal.is_sign_negative();
        let pd = !dominance.is_sign_negative();
        match (pp, pa, pd) {
            (true, true, true)   => Self::Excited,
            (true, true, false)  => Self::Surprised,
            (true, false, true)  => Self::Confident,
            (true, false, false) => Self::Relaxed,
            (false, true, true)  => Self::Angry,
            (false, true, false) => Self::Anxious,
            (false, false, true) => Self::Bored,
            (false, false, false) => Self::Depressed,
        }
    }
}
```

---

## PAD Similarity

For mood-congruent memory retrieval and somatic landscape queries, PAD similarity is computed as cosine similarity mapped to [0.0, 1.0]:

```rust
/// PAD cosine similarity, mapped to [0, 1].
/// Captures the direction of emotional state (quality of emotion)
/// rather than its magnitude (intensity).
pub fn pad_cosine_similarity(a: &PadVector, b: &PadVector) -> f64 {
    let dot = a.pleasure * b.pleasure
        + a.arousal * b.arousal
        + a.dominance * b.dominance;
    let mag_a = (a.pleasure.powi(2) + a.arousal.powi(2) + a.dominance.powi(2)).sqrt();
    let mag_b = (b.pleasure.powi(2) + b.arousal.powi(2) + b.dominance.powi(2)).sqrt();

    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.5; // Neutral mood → middle similarity
    }
    (dot / (mag_a * mag_b) + 1.0) / 2.0 // Map [-1, 1] → [0, 1]
}
```

This function is used by the four-factor retrieval scoring model (see [09-mood-congruent-memory.md](./09-mood-congruent-memory.md)) where emotional congruence is the fourth retrieval factor alongside recency, importance, and semantic relevance.

---

## Decay Toward Baseline

The PAD vector decays toward neutral [0, 0, 0] with a configurable half-life (default: 4 hours). This prevents permanent affect drift — an agent that failed badly yesterday should not still be in a pessimistic state today if no new failures have occurred.

The decay function uses exponential decay:

```
factor = 0.5 ^ (elapsed_hours / half_life_hours)
pad.pleasure *= factor
pad.arousal *= factor
pad.dominance *= factor
```

After 1 half-life (4 hours), affect intensity is halved. After 2 half-lives (8 hours), it is quartered. After 4 half-lives (16 hours), it is at 6.25% of original intensity. This matches the ALMA (Affective Language Model Architecture) model's mood layer temporal dynamics (see [02-alma-three-layer-temporal.md](./02-alma-three-layer-temporal.md)).

Confidence decays toward 0.5 (neutral), not toward 0.0:

```
confidence = 0.5 + (confidence - 0.5) * factor
```

This ensures that an agent with no recent events settles at "uncertain" rather than "no confidence."

---

## Academic Foundations

- Mehrabian, A. (1996). "Pleasure-arousal-dominance: A general framework for describing and measuring individual differences in temperament." *Current Psychology*, 14(4), 261–292.
- Russell, J.A. & Mehrabian, A. (1977). "Evidence for a three-factor theory of emotions." *Journal of Research in Personality*, 11, 273–294.
- Plutchik, R. (1980). *Emotion: A Psychoevolutionary Synthesis*. Harper & Row.
- Kahneman, D. & Tversky, A. (1979). "Prospect Theory: An Analysis of Decision under Risk." *Econometrica*, 47(2), 263–291.
- Zhang, H. et al. "Building Emotional Support Chatbots in the Era of LLMs." *SIGDIAL*.
- Gadanho, S.C. (2003). "Learning Behavior-Selection by Emotions and Cognition in a Multi-Goal Robot Task." *Journal of Machine Learning Research*, 4, 385–412.

---

## Current Status and Gaps

**Implemented**: `PadVector`, `AffectState`, decay mechanics, appraisal rules, persistence, octant classification — all in `roko-daimon/src/lib.rs` (569 lines) and `roko-golem/src/daimon.rs` (972 lines).

**Gaps**: The golem implementation needs to be absorbed into the standalone crate per the dissolution plan. The six behavioral states (Engaged/Struggling/Coasting/Exploring/Focused/Resting) from refactoring-prd §2 need to be layered on top of the octant classification. PAD cosine similarity for retrieval scoring is specified but not yet wired into context assembly.

---

## Cross-references

- See [02-alma-three-layer-temporal.md](./02-alma-three-layer-temporal.md) for the three-layer temporal model
- See [03-occ-scherer-appraisal.md](./03-occ-scherer-appraisal.md) for appraisal triggers
- See [04-six-behavioral-states.md](./04-six-behavioral-states.md) for PAD → behavioral state mapping
- See [09-mood-congruent-memory.md](./09-mood-congruent-memory.md) for PAD cosine similarity in retrieval
- See topic [05-learning](../05-learning/INDEX.md) for Daimon feedback from gate outcomes
