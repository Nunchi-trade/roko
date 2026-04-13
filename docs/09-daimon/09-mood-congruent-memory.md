# Mood-Congruent Memory

> How emotional state biases knowledge retrieval: the four-factor scoring model, emotional tags on Engrams, PAD cosine similarity, and the dream-memory-emotion triangle.


> **Implementation**: Built

**Topic**: [Daimon](./INDEX.md)
**Prerequisites**: [01-pad-vector.md](./01-pad-vector.md), [07-15-percent-contrarian-retrieval.md](./07-15-percent-contrarian-retrieval.md)
**Key sources**: `bardo-backup/prd/03-daimon/02-emotion-memory.md`, `bardo-backup/prd/03-daimon/06-dream-daimon.md`

---

## Abstract

Emotional state biases what agents remember. This is not a bug — it's an adaptive mechanism grounded in Bower's (1981) associative network theory and validated by Emotional RAG (2024, arXiv:2410.23041). An anxious agent should retrieve memories of past dangers; a confident agent should retrieve memories of past successes. The four-factor retrieval model integrates emotional congruence as a first-class retrieval signal alongside recency, importance, and semantic relevance.

This document specifies how emotional tags are attached to Engrams (knowledge entries), how the four-factor scoring model computes retrieval priority, how PAD cosine similarity captures emotional direction, and how the dream system interacts with emotional memory through consolidation bias and depotentiation.

---

## Emotional Tags on Engrams

### The EmotionalTag Struct

Every Engram in the Neuro knowledge store gains an optional emotional tag. This tag captures the PAD vector and Plutchik classification at the time the Engram was created:

```rust
/// Emotional context attached to a knowledge entry at creation time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmotionalTag {
    /// PAD vector at the time the Engram was created.
    pub pad: PadVector,

    /// Plutchik label for human-readable annotation.
    /// e.g., "moderate_fear", "mild_joy", "strong_surprise"
    pub emotion: String,

    /// Emotional intensity at encoding time [0.0, 1.0].
    /// Higher intensity means the Engram was encoded under
    /// stronger emotional conditions and receives higher
    /// consolidation priority.
    pub intensity: f32,

    /// Brief description of the appraisal trigger.
    /// e.g., "gate_fail:rung_2:task_abc"
    pub trigger: String,

    /// Mood snapshot at the time of creation.
    /// Captures the broader affective context beyond the
    /// discrete emotion. Used for mood-congruent retrieval:
    /// the mood at encoding is compared with the mood at
    /// retrieval to compute congruence.
    pub mood_snapshot: PadVector,
}
```

The tag is optional — Engrams created before the Daimon is enabled, or during T0 ticks that skip appraisal, have `emotional_tag: None`. The emotional component defaults to a neutral factor (0.5) in the retrieval score for untagged entries.

### Extension to Engram Storage

The Engram storage schema (whether in SQLite, JSONL, or the Substrate) includes affect provenance columns:

```
affect_pleasure     REAL    -- PAD pleasure at discovery
affect_arousal      REAL    -- PAD arousal at discovery
affect_dominance    REAL    -- PAD dominance at discovery
discovery_emotion   TEXT    -- Plutchik label at time of creation
```

These columns enable the four-factor retrieval scoring. They are indexed for efficient filtering (e.g., "all Engrams where arousal > 0.5").

---

## Four-Factor Retrieval Scoring

Every knowledge retrieval scores candidates using four factors, each grounded in a different research tradition:

### Factor 1: Recency (Ebbinghaus 1885)

Memories fade with time. The forgetting curve is exponential:

```
recency = exp(-t / half_life)
```

where `t` is time since last access. Recently accessed entries score higher. Half-life is type-dependent (Warnings: 7 days; Insights: 30 days; Facts: 365 days) and tier-multiplied (Transient: 0.1×; Working: 0.5×; Consolidated: 1.0×; Persistent: 5.0×).

### Factor 2: Importance (Shinn et al. 2023)

Entries validated through operational use earn higher confidence. The quality score combines raw confidence with the validation ratio:

```
quality = confidence × (validated / (validated + contradicted + 1))
```

This implements Reflexion's (Shinn et al. 2023) core insight: self-reflection on past performance improves future decisions. Knowledge that predicted correct outcomes is more valuable than knowledge that has never been tested.

### Factor 3: Relevance (Standard RAG)

Cosine similarity between the query embedding and the entry embedding. This is the baseline retrieval signal — how semantically close is this entry to the agent's current task?

### Factor 4: Emotional Congruence (Bower 1981)

PAD cosine similarity between the agent's current emotional state and the entry's affect provenance:

```rust
/// Four-factor retrieval scoring.
///
/// score = w_recency    × recency(Ebbinghaus)
///       + w_importance × quality(Reflexion)
///       + w_relevance  × cosine(query, entry)
///       + w_emotional  × PAD_cosine(current_mood, entry_affect)
///
/// Initial weights: recency: 0.20, importance: 0.25, relevance: 0.35, emotional: 0.20
/// Weights are learned by the self-tuning system over time.
pub fn score_entry(
    entry: &KnowledgeEntry,
    query_embedding: &[f32],
    current_pad: &PadVector,
    current_tick: u64,
    weights: &RetrievalWeights,
) -> f64 {
    let recency = (-((current_tick - entry.last_accessed_at) as f64)
        / weights.recency_half_life).exp();

    let importance = entry.quality_score();

    let relevance = cosine_similarity(query_embedding, &entry.embedding);

    let emotional_congruence = pad_cosine_similarity(current_pad, &entry.affect_pad());

    weights.recency * recency
        + weights.importance * importance
        + weights.relevance * relevance
        + weights.emotional * emotional_congruence
}
```

### Weight Distribution

The initial weights (0.20, 0.25, 0.35, 0.20) assign 35% to semantic relevance, 25% to importance, 20% to recency, and 20% to emotional congruence. These are learned over time — the self-tuning system adjusts weights based on which factor combinations correlate with positive task outcomes.

The 20% emotional weight is significant but not dominant. It's large enough to bias retrieval toward mood-congruent entries (5–30% accuracy boost per Bower 1981) without overwhelming semantic relevance. An emotionally congruent but semantically irrelevant entry won't surface — the 35% relevance weight ensures topical appropriateness.

---

## PAD Cosine Similarity

The PAD cosine similarity function captures the *direction* of emotional state rather than its *magnitude*. Two anxiety states at different intensities are recognized as similar. Two states with the same intensity but different directions (anxiety vs. confidence) are recognized as different.

```rust
/// PAD cosine similarity, mapped to [0, 1].
///
/// Captures the direction of emotional state (quality of emotion)
/// rather than its magnitude (intensity). An anxiety state at
/// intensity 0.3 is similar to anxiety at intensity 0.8.
pub fn pad_cosine_similarity(a: &PadVector, b: &PadVector) -> f64 {
    let dot = a.pleasure * b.pleasure
        + a.arousal * b.arousal
        + a.dominance * b.dominance;

    let mag_a = (a.pleasure.powi(2) + a.arousal.powi(2) + a.dominance.powi(2)).sqrt();
    let mag_b = (b.pleasure.powi(2) + b.arousal.powi(2) + b.dominance.powi(2)).sqrt();

    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.5;  // Neutral mood → neutral similarity
    }

    // Map cosine similarity from [-1, 1] to [0, 1]
    (dot / (mag_a * mag_b) + 1.0) / 2.0
}
```

**Why cosine, not Euclidean?**: Euclidean distance in PAD space conflates emotional direction with emotional intensity. An agent at P: -0.2, A: +0.1, D: -0.1 (mild anxiety) and an agent at P: -0.8, A: +0.5, D: -0.4 (strong anxiety) have high Euclidean distance (0.74) but high cosine similarity (0.99). They're both anxious — the memories encoded under either state are relevant to the other. Cosine similarity captures this relationship.

---

## The Full Retrieval Pipeline

The complete retrieval pipeline integrates four-factor scoring with contrarian injection:

```
Phase 1: CANDIDATE GENERATION
    HNSW approximate nearest neighbors (3× overfetch)
    → 3 × limit candidates

Phase 2: FOUR-FACTOR RE-RANKING
    Score each candidate with recency × importance × relevance × emotional congruence
    → Sorted by composite score

Phase 3: CONTRARIAN INJECTION (if tracker says inject)
    Invert pleasure and dominance of current PAD
    Search for opposite-valence entries
    Add to candidate set and re-rank
    → Ensures 15% minimum opposite-valence in rolling 200-tick window

Phase 4: RETRIEVAL STRENGTHENING (Testing Effect)
    Mark retrieved entries as accessed
    Increment access count
    → Roediger & Karpicke (2006): retrieval strengthens memory trace

Phase 5: RETURN top-k results
```

### Cross-Emotional Retrieval

The system supports explicit cross-emotional retrieval — deliberately searching for memories encoded under a different emotional state. This is available as a named retrieval mode for specific situations:

| Situation | Cross-Emotional Query | Rationale |
|---|---|---|
| Anxious agent, approaching deadline | Retrieve confident memories | Coping: recall successful strategies under pressure |
| Overconfident agent, novel territory | Retrieve cautious memories | Humility: recall past overconfidence failures |
| Stuck agent, no progress | Retrieve diverse emotional contexts | Divergence: inject variety to break patterns |

---

## Emotional Consolidation Bias

When the Daimon is enabled, the dream consolidation process applies an emotional salience boost. High-arousal experiences are preferentially consolidated, matching the neurobiological pattern where emotional arousal enhances memory consolidation via amygdala-hippocampal interaction (McGaugh 2004):

```rust
pub fn consolidation_priority(episode: &Episode) -> f64 {
    let base_priority = episode.importance * episode.novelty;

    let emotional_tag = match &episode.emotional_tag {
        Some(tag) => tag,
        None => return base_priority,
    };

    // Emotional arousal increases consolidation priority.
    let arousal_boost = emotional_tag.pad.arousal.abs() as f64 * 0.3;
    base_priority * (1.0 + arousal_boost)
}
```

The 0.3 scaling factor ensures emotional salience is *additive* to the existing importance-novelty ranking, not dominant. Among episodes of comparable importance and novelty, emotional intensity breaks the tie.

---

## Emotional Provenance Tracking

When episodes are consolidated into Insights and Heuristics, the emotional provenance transfers:

```rust
pub struct EmotionalProvenance {
    /// Average PAD vector during evidence accumulation.
    pub average_pad: PadVector,

    /// Emotion at first discovery.
    pub discovery_emotion: String,

    /// Narrative arc of validation.
    pub validation_arc: Option<ValidationArc>,

    /// Emotional diversity: Shannon entropy across supporting episodes.
    /// Higher = more reliable (validated across diverse emotional states).
    pub emotional_diversity: f64,
}

pub enum ValidationArc {
    /// Adversity to positive outcome — most transferable knowledge.
    Redemptive,
    /// Initial success to failure — cautionary.
    Contaminating,
    /// Consistent tone — reliable but less narratively rich.
    Stable,
    /// Gradual improvement — successful learning trajectory.
    Progressive,
}
```

### Emotional Diversity as Quality Signal

An Insight validated across diverse emotional states is more reliable than one validated only during a single mood. Emotional diversity is computed as normalized Shannon entropy of emotional labels across supporting episodes:

```rust
pub fn emotional_diversity(supporting_episodes: &[Episode]) -> f64 {
    let mut emotion_counts: HashMap<String, u32> = HashMap::new();
    for ep in supporting_episodes {
        if let Some(ref tag) = ep.emotional_tag {
            *emotion_counts.entry(tag.emotion.clone()).or_insert(0) += 1;
        }
    }

    let total: u32 = emotion_counts.values().sum();
    if total == 0 { return 0.0; }

    let mut entropy = 0.0_f64;
    for &count in emotion_counts.values() {
        let p = count as f64 / total as f64;
        if p > 0.0 {
            entropy -= p * p.log2();
        }
    }

    let max_entropy = (emotion_counts.len() as f64).log2();
    if max_entropy > 0.0 { entropy / max_entropy } else { 0.0 }
}
```

A diversity score of 1.0 means every supporting episode had a different emotional label — maximum diversity, highest reliability signal. A score of 0.0 means all episodes had the same label — potential emotional bias in the validation.

---

## The Dream-Memory-Emotion Triangle

Dreams, memory, and emotion form a three-way interaction:

1. **Emotion → Memory**: Emotional state biases which memories are retrieved (mood-congruent retrieval) and which are consolidated (arousal-based consolidation priority).

2. **Memory → Emotion**: Retrieved memories influence the agent's current emotional state. Retrieving past failures increases anxiety; retrieving past successes increases confidence. This is the appraisal pipeline — events are appraised, and retrieved context shapes how events are interpreted.

3. **Dreams → Memory + Emotion**: Dream consolidation reorganizes memory (promoting, pruning, synthesizing) and reshapes emotion (REM depotentiation reduces arousal on charged memories). Dreams also create new somatic markers from emotionally significant episodes.

This triangle is self-regulating when all three mechanisms operate:
- Mood-congruent retrieval provides adaptive bias (useful in most cases)
- Contrarian retrieval prevents the bias from becoming a trap
- Dream depotentiation cools the fuel that drives the bias
- Consolidation strengthens the most emotionally significant memories

Without dreams, the triangle degenerates: emotional memories accumulate without processing, mood-congruent retrieval creates increasingly strong feedback loops, and the contrarian mechanism becomes the only defense against rumination.

---

## Academic Foundations

- Bower, G.H. (1981). "Mood and Memory." *American Psychologist*, 36(2), 129–148.
- Blaney, P.H. (1986). "Affect and Memory: A Review." *Psychological Bulletin*, 99(2), 229–246.
- Faul, L. & LaBar, K.S. (2022). "Mood-Congruent Memory Revisited." *Psychological Review*.
- Emotional RAG. (2024). "Emotional RAG: Enhancing Role-Playing Agents through Emotional Retrieval." arXiv:2410.23041.
- Ebbinghaus, H. (1885). *Memory: A Contribution to Experimental Psychology*.
- Shinn, N. et al. (2023). "Reflexion: Language Agents with Verbal Reinforcement Learning." *NeurIPS*.
- McGaugh, J.L. (2004). "The Amygdala Modulates the Consolidation of Memories of Emotionally Arousing Experiences." *Annual Review of Neuroscience*, 27.
- Roediger, H.L. & Karpicke, J.D. (2006). "Test-enhanced learning: Taking memory tests improves long-term retention." *Psychological Science*, 17(3), 249–255.
- McAdams, D.P. (2001). "The Psychology of Life Stories." *Review of General Psychology*, 5(2).
- Abelson, R.P. (1963). "Computer Simulation of 'Hot Cognition'." In Tomkins & Messick (Eds.), *Computer Simulation of Personality*. Wiley.

---

## Cross-References

- See [01-pad-vector.md](./01-pad-vector.md) for PAD vector structure
- See [07-15-percent-contrarian-retrieval.md](./07-15-percent-contrarian-retrieval.md) for contrarian injection details
- See [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md) for somatic landscape integration
- See topic [03-dreams](../10-dreams/INDEX.md) for dream consolidation and depotentiation
- See topic [04-knowledge](../06-neuro/INDEX.md) for Neuro knowledge store integration
