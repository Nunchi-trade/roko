# NREM Replay: Utility-Weighted Episode Consolidation

> **Layer**: Cognitive Cross-Cut (L1 Framework agent dispatch, L2 Scaffold context assembly)
>
> **Synapse Traits**: `Substrate` (episode retrieval from NeuroStore), `Scorer` (Mattar-Daw utility formula)
>
> **Crate**: `roko-dreams` — NREM replay logic within `cycle.rs`
>
> **Prerequisites**: [01-three-phase-cycle.md](01-three-phase-cycle.md)

---

## What NREM Replay Does

NREM (Non-Rapid Eye Movement) replay is the first phase of every dream cycle. It takes accumulated episodes from the agent's episode log and replays them — not verbatim, but with controlled mutations — to consolidate memory and extract patterns. The goal is threefold:

1. **Strengthen useful memories**: Episodes that contain valuable patterns get reinforced in NeuroStore. Their associated knowledge entries gain confidence.
2. **Weaken irrelevant memories**: Episodes that contain no actionable patterns enter temporal decay. Their associated knowledge entries lose confidence.
3. **Extract cross-episode patterns**: Structural similarities between unrelated episodes are discovered through HDC clustering and trigram mining.

The biological analogy is hippocampal sharp-wave ripples during slow-wave sleep (stages N2–N3). Ji & Wilson (2007, Nature Neuroscience) showed that these ripples replay compressed versions of waking experiences, and the replay is not passive — it actively reorganizes memory.

---

## The Mattar-Daw Utility Formula

Episode selection for replay is governed by the utility formula from Mattar & Daw (2018, Nature Neuroscience, "Prioritized memory access explains planning and hippocampal replay"):

```
Utility(episode) = Gain(episode) × Need(episode) × (1 / SpacingPenalty(episode))
```

### Gain

Gain measures how much the agent's behavior would improve by better processing this episode. It is computed as the **prediction error** — the magnitude of the difference between what the agent expected and what actually happened:

```
Gain(episode) = |expected_outcome - actual_outcome|
```

For episodes that ended in failure (a gate rejection, a task that produced incorrect output), the gain is inherently high because the agent's predictions were wrong. For episodes that succeeded exactly as predicted, the gain is low — there is nothing new to learn.

Gain is normalized to [0.0, 1.0] across the current episode batch.

### Need

Need measures how often the agent encounters situations structurally similar to this episode. It is computed using HDC similarity between the episode's vector representation and the centroid of the most recent N episodes:

```
Need(episode) = HDC_similarity(episode_vector, recent_centroid)
```

Where `episode_vector` is a 10,240-bit BSC (Binary Spatter Code) vector encoding the episode's structural features (task type, model used, tools invoked, outcome), and `recent_centroid` is the bundled vector of the most recent 50 episodes. High need means the agent frequently encounters situations like this one, so learning from it has high expected value.

The HDC similarity operation uses Hamming distance and runs in sub-microsecond time per comparison (Kanerva 2009, Cognitive Computation 1(2)). This means the Need computation scales linearly with episode count and is never a bottleneck.

### Spacing Penalty

The spacing penalty implements the well-established **spacing effect** from memory research (Cepeda et al. 2006, Psychological Bulletin). Recently replayed episodes are penalized to prevent over-rehearsal:

```
SpacingPenalty(episode) = 1.0 + (replay_count × decay_factor / time_since_last_replay)
```

Where:
- `replay_count` is the number of times this episode has been replayed in previous dream cycles
- `decay_factor` is a configurable parameter (default: 0.5)
- `time_since_last_replay` is measured in hours since the episode was last replayed

An episode replayed recently gets a high spacing penalty (reducing its utility). An episode replayed long ago gets a low spacing penalty (increasing its utility). An episode never replayed gets a spacing penalty of 1.0 (no penalty).

### Batch Selection

The top-K episodes by utility score are selected for the current replay batch. K is configurable (default: 10 episodes per dream cycle). The selection uses a priority queue:

```rust
fn select_replay_batch(
    episodes: &[Episode],
    recent_centroid: &HdcVector,
    replay_history: &HashMap<String, ReplayRecord>,
    batch_size: usize,
) -> Vec<&Episode> {
    let mut scored: Vec<(f64, &Episode)> = episodes
        .iter()
        .map(|ep| {
            let gain = compute_gain(ep);
            let need = compute_need(ep, recent_centroid);
            let spacing = compute_spacing_penalty(ep, replay_history);
            let utility = gain * need * (1.0 / spacing);
            (utility, ep)
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
    scored.into_iter().take(batch_size).map(|(_, ep)| ep).collect()
}
```

---

## Replay Modes

Once episodes are selected, they are replayed using one of four modes:

### 1. Standard Forward Replay

The episode is replayed in chronological order. The agent reviews what happened and extracts insights using its current knowledge state:

```
Given this episode from {timestamp}:
  Task: {task_description}
  Actions taken: {action_sequence}
  Outcome: {success/failure}
  Gate results: {gate_verdicts}

With your current knowledge, would you do anything differently?
What patterns do you notice that you might not have noticed at the time?
```

Standard forward replay is the baseline mode. It catches cases where the agent's knowledge has improved since the episode occurred and the old actions are now suboptimal.

### 2. Reverse Replay

The episode is replayed backward — from outcome to initial conditions. This strengthens causal associations in the backward direction ("this outcome was caused by these conditions"):

```
An episode ended with this outcome: {outcome}
Working backward, what conditions and decisions led to this result?
What was the earliest decision point where a different choice would have changed the outcome?
```

Reverse replay is based on Ambrose et al. (2016, Science, "Reverse replay of hippocampal place cells"), which showed that backward replay during sleep strengthens goal-to-start causal chains, enabling better planning.

### 3. Perturbed Replay

In 30% of replays (configurable), key values within the episode are systematically perturbed to test robustness. The perturbations are not random — they shift values within a plausible range:

| Perturbation Type | Description | Magnitude |
|-------------------|-------------|-----------|
| **Value shift** | Numeric parameters are shifted by ±10-50% | Within the observed range from the episode log |
| **Timing shift** | Temporal parameters are shifted forward/backward | ±2× the original duration |
| **Outcome flip** | The outcome is reversed (success→failure or vice versa) | Binary flip |
| **Context injection** | Context from an unrelated episode is injected | Random episode from a different cluster |

The purpose of perturbed replay is to test whether extracted patterns are robust or fragile. If a pattern only holds under the exact original conditions, it is fragile and should receive lower confidence. If the pattern holds under perturbation, it is robust and should receive higher confidence.

```
This episode originally had these values: {original_values}
Now imagine these values were slightly different: {perturbed_values}
Would the same outcome have occurred? Would you have taken the same actions?
What does this tell you about how robust your strategy is?
```

### 4. Compressed Batch Replay

When the episode backlog is large (>50 episodes since the last dream), the replay phase uses compressed batch mode. Instead of replaying individual episodes, it groups them by HDC cluster and replays the cluster centroid:

1. Compute HDC vectors for all unprocessed episodes
2. Run K-medoids clustering (`roko-learn::hdc_clustering::k_medoids`)
3. For each cluster, replay the medoid episode as representative of the group
4. Patterns extracted from the medoid apply to all cluster members

This reduces the number of LLM calls while preserving the structural diversity of the replay. A batch of 100 episodes might produce 4–6 clusters, requiring only 4–6 replay calls instead of 100.

---

## Cross-Episode Pattern Discovery

In addition to per-episode replay, the NREM phase runs two pattern-discovery operations across the entire replay batch:

### Trigram Mining

The `PatternMiner` from `roko-learn::pattern_discovery` identifies recurring three-action sequences across episodes:

```rust
let mut miner = PatternMiner::new(
    min_support,      // e.g., 2 — trigram must appear in ≥2 episodes
    min_confidence,   // e.g., 0.5 — trigram must appear in ≥50% of episodes
);
for episode in &replay_batch {
    miner.ingest_episode(episode);
}
let patterns = miner.discover();
```

A discovered trigram like `read → edit → test` (appearing in 8 of 10 episodes) becomes a candidate heuristic: "the pattern of reading, editing, then testing is a reliable workflow." Trigrams that appear with high confidence in successful episodes but low confidence in failed episodes are especially informative.

### HDC Cross-Episode Consolidation

The `CrossEpisodeConsolidator` from `roko-learn::pattern_discovery` discovers structural meta-patterns across unrelated episodes:

```rust
let consolidator = CrossEpisodeConsolidator::new(
    target_clusters,    // e.g., 4
    min_cluster_size,   // e.g., 2
    max_iterations,     // e.g., 50
    min_coherence,      // e.g., 0.55
);
let report = consolidator.discover(&episodes);
```

This uses K-medoids clustering over HDC episode vectors. Each cluster represents a group of structurally similar episodes — episodes that share task type, model, outcome pattern, and tool usage regardless of their specific content. The cluster's bundle vector captures what they have in common.

A meta-pattern like "all episodes using the `code-implementer` template with `claude-sonnet` on `implementation` tasks succeed at the `compile` gate but fail at the `test` gate" is a cross-episode structural insight that would be difficult to notice from individual episode review.

---

## Replay Output Format

Each replay produces an `InsightRecord` that is passed to the Integration phase:

```rust
pub struct InsightRecord {
    /// Unique identifier.
    pub id: String,
    /// The extracted insight or pattern.
    pub content: String,
    /// Confidence score (0.0–1.0).
    pub confidence: f64,
    /// Source episode IDs that contributed to this insight.
    pub source_episodes: Vec<String>,
    /// The replay mode that produced this insight.
    pub replay_mode: ReplayMode,
    /// Whether the insight confirms or contradicts existing knowledge.
    pub relation_to_existing: InsightRelation,
    /// HDC vector encoding of the insight content.
    pub hdc_vector: Option<HdcVector>,
}

pub enum ReplayMode {
    Forward,
    Reverse,
    Perturbed,
    CompressedBatch,
}

pub enum InsightRelation {
    /// Confirms an existing knowledge entry (boosts confidence).
    Confirms(String),
    /// Contradicts an existing knowledge entry (reduces confidence).
    Contradicts(String),
    /// Novel — no existing knowledge entry is similar enough.
    Novel,
}
```

---

## Emotional Modulation of Replay

The Daimon's PAD vectors influence replay in two ways:

### 1. Somatic Marker Prioritization

Episodes encoded with high emotional intensity (high absolute arousal, regardless of valence) receive a replay priority boost. This implements Damasio's (1994) somatic marker hypothesis — emotions mark experiences as significant, and significant experiences should be replayed preferentially:

```
replay_priority_boost = |arousal_at_encoding| × somatic_weight
```

Where `somatic_weight` is configurable (default: 0.3). An episode encoded with arousal 0.9 gets a 0.27 priority boost.

### 2. Mood-Congruent Recall

The current mood state biases replay toward mood-congruent episodes. A negative-mood agent replays more failure episodes; a positive-mood agent replays more success episodes. This is biologically grounded (Blaney 1986, Psychological Bulletin) but deliberately attenuated in Roko to prevent rumination spirals:

```
mood_congruence = similarity(current_pad, episode_pad) × attenuation_factor
```

Where `attenuation_factor` is 0.15 (a weak bias). The agent's mood gently influences what it dreams about, but does not dominate the utility-based selection.

---

## Academic Citations

| Paper | How It Informs NREM Replay |
|-------|---------------------------|
| Mattar & Daw (2018), Nature Neuroscience, "Prioritized memory access explains planning and hippocampal replay" | Core utility formula for episode selection |
| Ji & Wilson (2007), Nature Neuroscience, "Coordinated memory replay in the visual cortex and hippocampus during sleep" | Biological basis for compressed replay during slow-wave sleep |
| Ambrose et al. (2016), Science, "Reverse replay of hippocampal place cells is uniquely associated with period of reward" | Biological basis for bidirectional (including reverse) replay |
| Cepeda et al. (2006), Psychological Bulletin, "Distributed practice in verbal recall tasks" | Spacing effect: distributed replay produces better retention than massed replay |
| Buzsáki (1989), Neuroscience, "Two-stage model of memory trace formation" | Sharp-wave ripple mechanism for hippocampal-to-cortical transfer |
| Diekelmann & Born (2010), Psychological Review, "The memory function of sleep" | Comprehensive review of sleep's role in memory consolidation |
| Damasio (1994), Descartes' Error | Somatic marker hypothesis: emotional tagging of experiences guides decision-making |
| Blaney (1986), Psychological Bulletin | Mood-congruent memory recall |
| Kanerva (2009), Cognitive Computation 1(2), "Hyperdimensional Computing" | HDC: 10,240-bit BSC vectors for sub-microsecond similarity comparison |
| Park et al. (2023), UIST, arXiv:2304.03442, "Generative Agents" | Periodic reflection cycles for experience synthesis |
| McClelland et al. (1995), Psychological Review | Complementary Learning Systems: fast episodic + slow semantic memory |

---

## Configuration

```toml
[dreams.replay]
# Maximum episodes to replay per dream cycle
batch_size = 10
# Fraction of replays that use perturbation
perturbation_rate = 0.30
# Spacing effect decay factor
spacing_decay = 0.5
# Somatic marker weight for replay priority
somatic_weight = 0.3
# Mood congruence attenuation factor
mood_attenuation = 0.15
# Minimum utility score for replay inclusion
min_utility = 0.1
```

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [01-three-phase-cycle.md](01-three-phase-cycle.md) | Overview of the three-phase dream cycle and NREM's place within it |
| [03-rem-imagination.md](03-rem-imagination.md) | REM phase that processes NREM outputs for creative recombination |
| [04-consolidation-and-staging.md](04-consolidation-and-staging.md) | Integration phase that evaluates replay outputs |
| [06-hdc-counterfactual-synthesis.md](06-hdc-counterfactual-synthesis.md) | HDC vector operations used for similarity computation and clustering |
| [../03-neuro/INDEX.md](../03-neuro/INDEX.md) | NeuroStore where replay outputs are persisted |
