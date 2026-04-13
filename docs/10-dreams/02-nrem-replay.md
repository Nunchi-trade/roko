# NREM Replay: Utility-Weighted Episode Consolidation

> **Layer**: Cognitive Cross-Cut (L1 Framework agent dispatch, L2 Scaffold context assembly)
>
> **Synapse Traits**: `Substrate` (episode retrieval from NeuroStore), `Scorer` (Mattar-Daw utility formula)
>
> **Crate**: `roko-dreams` — NREM replay logic within `cycle.rs`
>
> **Prerequisites**: [01-three-phase-cycle.md](01-three-phase-cycle.md)


> **Implementation**: Scaffold

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
| Shin et al. (2017), NeurIPS, "Continual Learning with Deep Generative Replay" | Scholar architecture H = ⟨G, S⟩: generator produces synthetic samples from past distributions to prevent catastrophic forgetting |
| Gao et al. (2023), ICML, diffusion-based generative replay | Diffusion model as generator substantially closes the quality gap between generative and exact replay |
| Kurth-Nelson et al. (2023), MEG study | Human hippocampal replay is partial, contains novel shortcuts, and supports compositional inference across non-adjacent episodes |
| Helfrich et al. (2023), Nature Neuroscience | Triple SO-spindle-ripple coupling: spindle onset ~451ms before SO down-state, SWR within 250ms of spindle onset; disruption impairs consolidation |
| Wozniak (1990), SM-2 algorithm | Spaced repetition: EF-based interval scheduling for optimal review timing, basis for SuperMemo and Anki |
| Mnih et al. (2015), Nature, "Human-level control through deep reinforcement learning" | DQN: circular replay buffer of 1M transitions, uniform minibatch sampling of size 32 |
| Schaul et al. (2016), ICLR, "Prioritized Experience Replay" | PER: priority p_i = |δ_i| + ε, sampling P(i) = p_i^α / Σ p_j^α with α=0.6, IS weights with β annealing 0.4→1.0 |
| Andrychowicz et al. (2017), NeurIPS, "Hindsight Experience Replay" | HER: relabeling failed episodes with achieved goals; k=4 virtual goals per transition using "future" strategy |
| Wang & Ross (2019), "Boosting Soft Actor-Critic: Emphasizing Recent Experience without Forgetting the Past" | ERE: c_k = max(N · η^(k·1000/K), c_min) with η=0.996; dynamic buffer shrinkage emphasizing recent experience |

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

## Implementation details

### Gain computation

The `compute_gain` function derives prediction error from the episode's `expected_outcome` and `actual_outcome` fields. Both fields are stored as `f64` values on `Episode` representing a normalized success metric (0.0 = total failure, 1.0 = perfect success).

```rust
/// Compute gain for an episode.
///
/// Uses absolute prediction error, normalized across the batch.
/// Episodes without an expected outcome (first-time task types) receive
/// gain = 1.0 — maximum learning signal.
fn compute_gain(ep: &Episode) -> f64 {
    match ep.expected_outcome {
        Some(expected) => (expected - ep.actual_outcome).abs(),
        None => 1.0, // novel task type — always worth replaying
    }
}

/// Normalize gains to [0.0, 1.0] across a batch.
fn normalize_gains(gains: &mut [f64]) {
    let max = gains.iter().cloned().fold(0.0_f64, f64::max);
    if max > 0.0 {
        for g in gains.iter_mut() {
            *g /= max;
        }
    }
}
```

When the entire batch has zero prediction error (every episode matched expectations), all gains remain 0.0. The batch selection then falls back to Need alone, which is the correct behavior: if nothing surprised you, rehearse the most common patterns.

Cross-scale normalization applies when episodes come from different task types with different outcome scales. Gate pass/fail episodes use binary outcomes (0.0 or 1.0), while performance-scored episodes use continuous values. The normalization pass handles this by operating on the absolute deltas after gain computation, not on the raw outcome values.

### Need computation

Need uses the following HDC feature set to build each episode vector:

| Feature | Encoding | Bits |
|---------|----------|------|
| Task type | `HdcVector::from_seed(task_type.as_bytes())` | 10,240 |
| Model used | `HdcVector::from_seed(model_id.as_bytes())` | 10,240 |
| Tools invoked | Bundle of per-tool seed vectors | 10,240 |
| Outcome class | `HdcVector::from_seed(outcome_class.as_bytes())` | 10,240 |
| Gate results | Bind chain of per-gate vectors | 10,240 |

The episode vector is produced by binding all five feature vectors:

```rust
fn encode_episode(ep: &Episode) -> HdcVector {
    let task_v = HdcVector::from_seed(ep.task_type.as_bytes());
    let model_v = HdcVector::from_seed(ep.model_id.as_bytes());
    let tools_v = HdcVector::bundle(
        &ep.tools_used.iter()
            .map(|t| HdcVector::from_seed(t.as_bytes()))
            .collect::<Vec<_>>()
            .iter()
            .collect::<Vec<_>>(),
    );
    let outcome_v = HdcVector::from_seed(ep.outcome_class.as_bytes());
    let gates_v = ep.gate_results.iter().fold(
        HdcVector::from_seed(b"gate_identity"),
        |acc, g| acc.bind(&HdcVector::from_seed(g.as_bytes())),
    );

    task_v
        .bind(&model_v)
        .bind(&tools_v)
        .bind(&outcome_v)
        .bind(&gates_v)
}
```

The recent centroid bundles the most recent N episodes (default N=50, configurable via `dreams.replay.centroid_window`). The centroid is recomputed at the start of each dream cycle — not cached across cycles — because the recent distribution shifts.

```rust
fn compute_recent_centroid(
    episodes: &[Episode],
    window: usize,
) -> HdcVector {
    let recent = &episodes[episodes.len().saturating_sub(window)..];
    let vectors: Vec<HdcVector> = recent.iter().map(encode_episode).collect();
    HdcVector::bundle(&vectors.iter().collect::<Vec<_>>())
}
```

Need falls to ~0.5 (random baseline) for episodes that share no structural features with recent work. It rises toward 1.0 for episodes that mirror the agent's current activity patterns.

### Spacing penalty

Parameters and their ranges:

| Parameter | Default | Range | Unit | Effect |
|-----------|---------|-------|------|--------|
| `decay_factor` | 0.5 | 0.1 - 2.0 | dimensionless | Higher values penalize recent replays more aggressively |
| `time_since_last_replay` | measured | >0 | hours | Floor clamped to 0.01 to avoid division by zero |
| `replay_count` | tracked | 0+ | integer | Incremented each time the episode enters a replay batch |

For an episode that has never been replayed, the formula yields:

```
SpacingPenalty = 1.0 + (0 * 0.5 / anything) = 1.0
```

No penalty. For an episode replayed twice, 1 hour ago:

```
SpacingPenalty = 1.0 + (2 * 0.5 / 1.0) = 2.0
```

Utility halved. For the same episode 24 hours later:

```
SpacingPenalty = 1.0 + (2 * 0.5 / 24.0) = 1.042
```

Nearly no penalty. The spacing effect works as intended: recently rehearsed episodes are suppressed, but the suppression decays over time.

```rust
fn compute_spacing_penalty(
    ep: &Episode,
    replay_history: &HashMap<String, ReplayRecord>,
) -> f64 {
    match replay_history.get(&ep.id) {
        None => 1.0,
        Some(record) => {
            let hours_since = record
                .last_replayed
                .elapsed()
                .as_secs_f64() / 3600.0;
            let clamped_hours = hours_since.max(0.01); // avoid div-by-zero
            1.0 + (record.count as f64 * SPACING_DECAY / clamped_hours)
        }
    }
}
```

### K-medoids clustering

Dynamic K selection uses the silhouette method. The algorithm tests K values from 2 to `max_k` (default: `(n_episodes as f64).sqrt().ceil() as usize`, capped at 12) and picks the K that maximizes mean silhouette score:

```rust
fn select_k(
    vectors: &[HdcVector],
    max_k: usize,
    max_iterations: usize,
) -> usize {
    let mut best_k = 2;
    let mut best_score = f64::NEG_INFINITY;

    for k in 2..=max_k {
        let result = k_medoids(vectors, &KMedoidsConfig {
            k,
            max_iterations,
        });
        let score = mean_silhouette(vectors, &result.assignments, &result.medoids);
        if score > best_score {
            best_score = score;
            best_k = k;
        }
    }
    best_k
}
```

Convergence criterion: the algorithm stops when no swap of a medoid with a non-medoid reduces the total within-cluster distance. Maximum iterations (default: 50) prevents runaway computation on pathological distributions.

Failure handling: if K-medoids fails to converge within `max_iterations`, the algorithm returns the best assignment found so far. If the input has fewer vectors than K, K is reduced to `vectors.len() - 1`. If only one vector exists, no clustering is performed and the single episode is replayed directly.

### Perturbed replay

Perturbation selection follows a fixed priority order, not random choice:

1. **Value shift** is always applied first (most common, least disruptive)
2. **Timing shift** is applied if the episode has temporal parameters
3. **Outcome flip** is applied to 1 in 3 perturbed replays (configurable via `perturbation_flip_rate`, default: 0.33)
4. **Context injection** is applied to 1 in 5 perturbed replays (configurable via `perturbation_inject_rate`, default: 0.20)

Perturbation ranges are justified by the episode log's observed variance:

```rust
fn compute_perturbation_range(
    episodes: &[Episode],
    field: &str,
) -> (f64, f64) {
    let values: Vec<f64> = episodes
        .iter()
        .filter_map(|ep| ep.numeric_field(field))
        .collect();
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let std_dev = (values.iter()
        .map(|v| (v - mean).powi(2))
        .sum::<f64>() / values.len() as f64)
        .sqrt();
    (mean - 2.0 * std_dev, mean + 2.0 * std_dev)
}
```

Perturbations stay within 2 standard deviations of the observed range. This prevents generating implausible scenarios while still testing robustness.

Perturbations are applied independently. Each perturbation type is a separate axis. A single perturbed replay may combine a value shift with a timing shift, but outcome flip and context injection are applied alone (they change the episode's semantics too much to combine with other perturbation types).

### Insight consolidation

Insights from replay are written to NeuroStore through the `InsightConsolidator`:

```rust
pub struct InsightConsolidator {
    neuro_store: Arc<NeuroStore>,
    min_confidence: f64,       // default: 0.30
    merge_threshold: f32,      // default: 0.80 (HDC similarity)
    max_insights_per_cycle: usize, // default: 20
}

impl InsightConsolidator {
    /// Process replay insights and write to NeuroStore.
    ///
    /// Returns the number of new entries created and existing entries updated.
    pub fn consolidate(
        &self,
        insights: Vec<InsightRecord>,
    ) -> Result<ConsolidationReport, DreamError> {
        let mut created = 0;
        let mut updated = 0;

        for insight in insights {
            if insight.confidence < self.min_confidence {
                continue; // below threshold — discard
            }

            // Check for existing similar entry
            match self.find_similar(&insight) {
                Some(existing) => {
                    // Merge: update confidence using EMA
                    let new_confidence = existing.confidence * 0.7
                        + insight.confidence * 0.3;
                    self.neuro_store.update_confidence(
                        &existing.id,
                        new_confidence,
                    )?;
                    updated += 1;
                }
                None => {
                    // Novel insight — create new entry
                    let entry = KnowledgeEntry {
                        id: generate_id(),
                        content: insight.content,
                        confidence: insight.confidence,
                        hdc_vector: insight.hdc_vector.unwrap_or_else(
                            || encode_insight_text(&insight.content)
                        ),
                        source: InsightSource::DreamReplay {
                            episode_ids: insight.source_episodes,
                            mode: insight.replay_mode,
                        },
                        created_at: Utc::now(),
                    };
                    self.neuro_store.insert(entry)?;
                    created += 1;
                }
            }
        }

        Ok(ConsolidationReport { created, updated })
    }

    fn find_similar(&self, insight: &InsightRecord) -> Option<KnowledgeEntry> {
        let vector = insight.hdc_vector.as_ref()?;
        self.neuro_store
            .nearest_neighbors(vector, 1)
            .into_iter()
            .next()
            .filter(|entry| entry.hdc_vector.similarity(vector) > self.merge_threshold)
    }
}
```

Confidence scoring rules:

| Source | Initial confidence | Rationale |
|--------|-------------------|-----------|
| Forward replay, confirmed pattern | 0.50 | Known pattern, re-observed |
| Forward replay, new insight | 0.35 | New observation from known data |
| Reverse replay | 0.40 | Causal chain analysis has moderate reliability |
| Perturbed replay, robust finding | 0.55 | Survived perturbation — higher trust |
| Perturbed replay, fragile finding | 0.25 | Did not survive perturbation — flag for review |
| Compressed batch, cluster-level | 0.45 | Represents multiple episodes but coarser signal |

HDC encoding for textual insights uses trigram hashing:

```rust
fn encode_insight_text(text: &str) -> HdcVector {
    let trigrams: Vec<&str> = text
        .as_bytes()
        .windows(3)
        .map(|w| std::str::from_utf8(w).unwrap_or(""))
        .collect();

    let trigram_vectors: Vec<HdcVector> = trigrams
        .iter()
        .enumerate()
        .map(|(i, trigram)| {
            let base = HdcVector::from_seed(trigram.as_bytes());
            base.permute(i) // position-encode via permutation
        })
        .collect();

    HdcVector::bundle(&trigram_vectors.iter().collect::<Vec<_>>())
}
```

This produces a position-sensitive encoding: the same words in different order yield different vectors. The permutation step encodes position, while the trigram seeds encode content.

### Error handling

| Error condition | Handling |
|-----------------|----------|
| Episode log is empty | Skip NREM phase, log warning, proceed to REM |
| HDC encoding fails (empty episode fields) | Use a random vector (degrades to Need ~0.5) |
| K-medoids does not converge | Use best-so-far assignment |
| LLM replay call fails | Retry once with backoff; on second failure, skip episode and log error |
| NeuroStore write fails | Buffer insights in memory, retry at end of cycle; if still failing, write to `.roko/dreams/pending_insights.jsonl` for next cycle |
| All episodes below `min_utility` | Skip NREM phase, log diagnostic |

### Integration wiring

NREM replay connects to the runtime through `DreamCycle::run_nrem()` in `roko-dreams/src/cycle.rs`:

```
orchestrate.rs
  └─ DreamCycle::run()            // entry point from plan executor
       └─ DreamCycle::run_nrem()  // NREM phase
            ├─ EpisodeLog::recent(window)     // fetch episodes from .roko/episodes.jsonl
            ├─ encode_episodes()              // HDC vectors for all episodes
            ├─ compute_recent_centroid()      // centroid of recent N
            ├─ select_replay_batch()          // utility-ranked selection
            ├─ for each episode:
            │    ├─ choose_replay_mode()      // forward/reverse/perturbed
            │    ├─ LlmProvider::generate()   // agent call for insight extraction
            │    └─ InsightRecord::new()      // capture output
            ├─ PatternMiner::discover()       // trigram mining across batch
            ├─ CrossEpisodeConsolidator::discover()  // HDC clustering
            └─ InsightConsolidator::consolidate()    // write to NeuroStore
```

### Test criteria

1. **Gain normalization**: batch of episodes with known outcomes produces gains in [0.0, 1.0]. Episodes with `None` expected outcome get gain 1.0.
2. **Need computation**: an episode with identical features to the recent centroid yields Need > 0.9. An episode with no shared features yields Need ~0.5.
3. **Spacing penalty**: replaying an episode twice in quick succession doubles the penalty. After 24 hours the penalty drops below 1.05.
4. **Batch selection**: top-K selection returns episodes in descending utility order. Ties are broken by episode timestamp (newer first).
5. **K-medoids convergence**: 100 random episode vectors with 4 planted clusters yields 4 clusters with silhouette > 0.6.
6. **Perturbation bounds**: all perturbed values fall within 2 standard deviations of the observed range.
7. **Insight consolidation**: inserting a near-duplicate insight (HDC similarity > 0.80) updates the existing entry's confidence instead of creating a new one.
8. **Error recovery**: empty episode log skips NREM without panic. LLM failure skips the individual episode and logs the error.
9. **End-to-end**: a dream cycle with 20 synthetic episodes produces at least 3 insights written to NeuroStore.

---

## Replay Fidelity

Replay fidelity refers to how faithfully an episode is reproduced during the replay phase. Biological and computational evidence converges on the same insight: exact replay is not always optimal, and sometimes controlled deviation from the original experience produces better generalization.

### The Fidelity Spectrum

Three replay modes sit on a spectrum from high fidelity to high generativity:

| Mode | Description | When to Use |
|------|-------------|-------------|
| **Exact** | Episode replayed verbatim, no modifications | Anchor memories; high-stakes patterns that must not drift |
| **Perturbed** | Controlled noise applied within observed variance | Standard generalization; robustness testing |
| **Generative** | Synthetic episode preserving structural features, varying surface details | Creative exploration; filling in experience gaps |

### Biological Evidence

Hippocampal replay is not veridical. Kurth-Nelson et al. (2023, MEG study) showed that human hippocampal replay during rest sequences is partial, sometimes contains novel shortcuts not experienced during learning, and supports compositional inference — the ability to infer relationships between episodes that were never directly experienced together. The brain does not simply play back a recording; it reconstructs, recombines, and extrapolates.

Temporal compression is equally striking: waking experiences spanning seconds to minutes are replayed within 100–300 ms sharp-wave ripple (SWR) events, achieving compression ratios of 6× to 20× (Ji & Wilson 2007). This compression is not arbitrary — it preserves causal structure while discarding moment-to-moment detail.

### Deep Generative Replay

Shin et al. (2017, NeurIPS, "Continual Learning with Deep Generative Replay") introduced the Scholar architecture H = ⟨G, S⟩, where a Generator G produces synthetic samples from previous distributions, and a Solver S trains on a mixture of real new data and generated old data. This architecture avoids catastrophic forgetting without storing raw episodes — the generator maintains a compressed distributional model of the past.

Roko's generative replay mode is inspired by this: rather than storing every episode indefinitely, the HDC vector of an episode serves as its structural template, and the generative mode synthesizes a new episode that matches the structural signature while varying surface content.

The diffusion-based generative replay approach of Gao et al. (2023, ICML) substantially closes the quality gap between generative and exact replay by using a diffusion model as the generator G. The structural similarity floor parameter controls how close the synthetic episode must be to the original HDC template.

### Fidelity Configuration

```rust
pub enum ReplayFidelity {
    /// Replay the episode as-is, no modifications.
    Exact,
    /// Apply controlled perturbations within observed variance.
    /// Perturbation magnitude controlled by `perturbation_sigma`.
    Perturbed { perturbation_sigma: f64 },
    /// Generate a synthetic episode that preserves structural features
    /// but varies surface details. Uses HDC vector as structural template.
    Generative { structural_similarity_floor: f32 },
}

pub struct ReplayFidelityConfig {
    /// Default fidelity mode for standard replay.
    pub default_mode: ReplayFidelity,       // default: Perturbed { perturbation_sigma: 0.15 }
    /// Fraction of replays that use exact mode (for anchor memories).
    pub exact_fraction: f64,                // default: 0.20, range: 0.0-0.50
    /// Fraction that use generative mode (for creative exploration).
    pub generative_fraction: f64,           // default: 0.10, range: 0.0-0.30
    /// Minimum temporal compression ratio (replay duration / original duration).
    pub min_compression_ratio: f64,         // default: 0.05 (20x compression)
    /// Maximum compression ratio.
    pub max_compression_ratio: f64,         // default: 0.17 (6x compression)
}
```

The `exact_fraction` and `generative_fraction` are enforced at the batch level: across each dream cycle's replay batch, exactly `batch_size × exact_fraction` episodes are replayed using exact mode, and `batch_size × generative_fraction` use generative mode. The remainder use the configured `default_mode` (typically `Perturbed`).

Anchor memories — episodes tagged with high somatic weight (arousal > 0.8 at encoding) — bypass the normal fidelity distribution and always use `Exact` mode. Their structural integrity is preserved regardless of the batch-level fidelity settings.

### Test Criteria for Fidelity

1. **Mode distribution**: across a batch of 20 episodes, the exact/perturbed/generative split matches the configured fractions within ±1 episode.
2. **Perturbation sigma**: `Perturbed { perturbation_sigma: 0.15 }` produces perturbed values with standard deviation within 5% of 0.15 × original_value.
3. **Structural similarity floor**: `Generative { structural_similarity_floor: 0.80 }` produces synthetic episodes with HDC similarity ≥ 0.80 to the source episode's vector.
4. **Anchor bypassing**: an episode with arousal > 0.8 is always assigned `Exact` mode regardless of batch-level fractions.
5. **Compression ratio**: replayed episode processing time falls within `[min_compression_ratio, max_compression_ratio]` × original episode wall-clock duration.

---

## Replay Scheduling

Optimal replay requires knowing not just *which* episodes to replay, but *when* to replay them and *how often*. The spacing effect (Cepeda et al. 2006) shows that distributed practice produces better retention than massed practice; the EVB framework (Mattar & Daw 2018) provides the normative answer for when to replay vs. collect new experience.

### The EVB Decision: Replay vs. Act

The Expected Value of Backup (EVB) answers: given a finite budget of compute, should the agent replay existing episodes or gather new experience? Mattar & Daw's framework says replay is optimal when:

```
EVB(best_candidate_episode) > V(collecting_new_experience)
```

In practice, Roko computes this at the start of each dream cycle. If the current EVB of the highest-utility episode exceeds the expected information gain from the next real task, the dream cycle proceeds. If the agent has rich unexplored tasks, it may defer the dream cycle.

### Spaced Repetition: SM-2 Algorithm

The SM-2 algorithm (Wozniak 1990, the basis for SuperMemo and Anki) provides a principled approach to scheduling individual episode reviews. Each episode accumulates SM-2 state:

```
Initial easiness factor:  EF₀ = 2.5
Interval after first replay:  I₁ = 1 hour
Interval after second replay: I₂ = EF₀ hours

For subsequent replays (n ≥ 3):
  I(n) = I(n-1) × EF

After each replay, update easiness factor:
  EF_new = EF + (0.1 - (5 - q) × (0.08 + (5 - q) × 0.02))

Where q is replay quality (0–5 scale derived from insight confidence):
  q = 5: perfect recall, high-confidence insight produced
  q = 4: correct with minor difficulty
  q = 3: correct with significant difficulty
  q = 2: incorrect, but easy to recall upon seeing the answer
  q = 1: incorrect, but easy to recall
  q = 0: complete failure

Minimum EF is 1.3. If EF falls below 1.3, reset interval to I₁.
```

The quality score q is derived from the insights produced by the replay. A replay that produces a high-confidence, novel insight scores high (q=4 or 5). A replay that produces no new insight and confirms what the agent already knows scores lower (q=2 or 3). A replay whose episode content has drifted so far from current relevance that no actionable insight emerges scores q=1.

### Spindle-Ripple Coupling

Biological sleep exhibits a triple oscillation coupling that gates replay windows: slow oscillations (SO, <1 Hz) set up and down states; sleep spindles (12–16 Hz) relay hippocampal signals to cortex; sharp-wave ripples (80–200 Hz) carry the actual replay content. This creates discrete, rhythmic replay windows rather than continuous replay.

Helfrich et al. (2023, Nature Neuroscience) characterized the precise timing: spindle onset precedes the SO down-state by ~451 ms, and the SWR occurs within 250 ms of spindle onset. This triple coupling is not merely correlational — disrupting the coupling impairs memory consolidation.

Roko's implementation does not model oscillations directly (it operates at the task level, not the millisecond level), but the principle carries over: replay occurs in discrete scheduled windows, not continuously. The `immediate_fraction`, `spaced_fraction`, and `exploration_fraction` parameters carve the replay budget into distinct purposes, analogous to the biological coupling that serves different consolidation functions.

### Scheduling Configuration

```rust
pub struct ReplayScheduleConfig {
    /// Spaced repetition easiness factor (SM-2 algorithm).
    pub initial_easiness: f64,          // default: 2.5, range: 1.3-5.0
    /// Minimum replay interval in hours.
    pub min_interval_hours: f64,        // default: 0.5
    /// Maximum replay interval in hours.
    pub max_interval_hours: f64,        // default: 168.0 (1 week)
    /// Quality threshold below which interval resets.
    pub quality_reset_threshold: f64,   // default: 0.3
    /// Fraction of replay budget for immediate post-experience replay.
    pub immediate_fraction: f64,        // default: 0.40
    /// Fraction for spaced review of older episodes.
    pub spaced_fraction: f64,           // default: 0.40
    /// Fraction for exploration of rarely-replayed episodes.
    pub exploration_fraction: f64,      // default: 0.20
}

pub struct EpisodicSpacingTracker {
    /// Per-episode tracking of replay history with SM-2 parameters.
    entries: HashMap<String, SpacingEntry>,
}

pub struct SpacingEntry {
    pub episode_id: String,
    pub easiness_factor: f64,
    pub interval_hours: f64,
    pub replay_count: u32,
    pub last_quality: f64,
    pub next_review_at: DateTime<Utc>,
}
```

### Adaptive Scheduling Algorithm

```
function schedule_replay_batch(episodes, budget, tracker, now):
    immediate = []
    spaced = []
    exploration = []

    for episode in episodes:
        entry = tracker.get(episode.id)
        if entry is None:
            // Never replayed — candidate for immediate bucket
            immediate.append(episode)
        else if now >= entry.next_review_at:
            // Due for spaced review
            spaced.append(episode)
        else if entry.replay_count <= 1:
            // Rarely replayed — candidate for exploration
            exploration.append(episode)

    // Sort each bucket by utility score (Mattar-Daw)
    immediate.sort_by_utility()
    spaced.sort_by_utility()
    exploration.sort_by_utility()

    // Allocate budget according to fractions
    n_immediate = floor(budget × immediate_fraction)
    n_spaced    = floor(budget × spaced_fraction)
    n_explore   = budget - n_immediate - n_spaced

    return (
        immediate[:n_immediate] +
        spaced[:n_spaced] +
        exploration[:n_explore]
    )

function update_spacing_entry(entry, quality):
    q_scaled = quality × 5.0  // normalize [0,1] quality to [0,5] SM-2 scale
    ef_delta = 0.1 - (5 - q_scaled) × (0.08 + (5 - q_scaled) × 0.02)
    entry.easiness_factor = max(1.3, entry.easiness_factor + ef_delta)

    if quality < quality_reset_threshold:
        entry.interval_hours = min_interval_hours
    else:
        entry.interval_hours = clamp(
            entry.interval_hours × entry.easiness_factor,
            min_interval_hours,
            max_interval_hours
        )

    entry.replay_count += 1
    entry.last_quality = quality
    entry.next_review_at = now + entry.interval_hours
```

### Recent Connections: FOREVER and MSSR

The FOREVER framework (2025) proposes aligning replay with model-time (optimizer steps) rather than wall-clock time. The insight is that the relevant decay variable is not elapsed time but the number of gradient updates the model has undergone since the episode was encoded — because each update shifts the model's internal representations and potentially invalidates cached patterns. Roko does not currently track optimizer steps (it uses LLM APIs rather than training), but the principle suggests that episode utility should degrade proportionally to how many new episodes the agent has processed since the original episode, not just how much time has elapsed.

The MSSR (2025) framework uses the entropy of the replay buffer — H = -Σ p_i log p_i over the priority distribution — as a scheduling signal. When buffer entropy is high (priorities are uniformly distributed), all episodes are equally interesting and the agent should gather new experience rather than replay. When entropy is low (one or a few episodes dominate), replay is urgent. This provides a natural EVB-like criterion that does not require an explicit model of new-experience value.

### Prioritized Experience Replay Connection

Schaul et al. (2016, "Prioritized Experience Replay") formalize the priority-based sampling that Roko's utility formula approximates:

```
Sampling probability: P(i) = p_i^α / Σ p_j^α

Where:
  p_i = |δ_i| + ε    (priority = TD error magnitude + small constant)
  ε   = 0.01          (ensures all transitions have non-zero probability)
  α   = 0.6           (controls how much prioritization is used; 0 = uniform)

Importance sampling correction:
  w_i = (1/N · 1/P(i))^β

Where β anneals from 0.4 to 1.0 over training to correct for bias
introduced by non-uniform sampling.
```

In Roko's formulation: `p_i ≈ Gain(episode) × Need(episode)` and the spacing penalty implements the bias correction analogous to importance sampling weights — episodes replayed too frequently have their effective priority reduced.

### Test Criteria for Scheduling

1. **SM-2 interval growth**: after 3 replays with quality 0.9, the interval has grown by at least EF₀² = 6.25× from the initial interval.
2. **Quality reset**: a replay with quality below `quality_reset_threshold` resets the interval to `min_interval_hours` regardless of prior history.
3. **Budget allocation**: across a batch of 10 episodes, the immediate/spaced/exploration split matches the configured fractions within ±1 episode.
4. **EF floor**: easiness factor never falls below 1.3 regardless of how many low-quality replays occur.
5. **Due detection**: episodes whose `next_review_at` is in the past are always placed in the spaced bucket, not the exploration bucket.
6. **Priority sampling**: with α=0.6, an episode with p_i=0.9 is sampled at least 3× more often than an episode with p_i=0.1 across 1,000 sampling trials.

---

## Deep RL Experience Replay Connections

Roko's dream replay system re-derives many of the principles discovered empirically by the deep reinforcement learning community. This section makes those connections explicit — both for intellectual grounding and to identify which RL replay techniques have not yet been incorporated.

### DQN: The Foundation

Mnih et al. (2015, "Human-level control through deep reinforcement learning", Nature) introduced experience replay as a core component of DQN:

- **Replay buffer**: circular buffer of 1,000,000 transitions `(s, a, r, s')`, discarding oldest when full
- **Sampling**: uniform random sampling of minibatches of size 32
- **Purpose**: breaks temporal correlations in the training stream; allows each transition to contribute to multiple gradient updates

Roko's analog: the `.roko/episodes.jsonl` log is the replay buffer. The Mattar-Daw utility formula replaces uniform sampling with principled prioritization.

### PER: Priority Matters

Schaul et al. (2016, "Prioritized Experience Replay", ICLR) replaced uniform sampling with priority-weighted sampling:

```
Priority:          p_i = |δ_i| + ε
                   where δ_i is the TD error and ε = 0.01

Sampling:          P(i) = p_i^α / Σ p_j^α
                   with α = 0.6

IS correction:     w_i = (1/N · 1/P(i))^β
                   with β annealing from 0.4 to 1.0
```

Roko mapping:

| PER Concept | Roko Equivalent |
|-------------|-----------------|
| TD error `\|δ_i\|` | `Gain(episode)` — prediction error |
| Priority `p_i` | `Gain × Need` product |
| Uniform baseline `ε` | `min_utility = 0.1` floor |
| IS weight `w_i` | Inverse of spacing penalty |
| α exponent | Implicit in the linear `Gain × Need` formula (α=1) |
| β annealing | Not yet implemented; future work |

The key insight PER contributes: IS weights must correct for the bias introduced by non-uniform sampling. Roko's spacing penalty partially achieves this by suppressing over-replayed episodes, but explicit IS weight computation would be a clean addition.

### HER: Relabeling Failure as Success

Andrychowicz et al. (2017, "Hindsight Experience Replay", NeurIPS) addressed the sparse-reward problem: when an agent almost always fails, it has almost nothing to learn from. HER's insight — relabel failed episodes with the goal that was actually achieved, turning failures into successes for a different (virtual) goal.

HER parameters:
- **k = 4** virtual goals per real transition
- **Strategy**: "future" — goals sampled from the remainder of the same episode
- **Effect**: transforms a buffer of near-complete failures into a rich signal about what the agent is capable of achieving

Roko mapping — when a task fails its intended gate, replay that episode with the gate it *did* pass as the "achieved goal":

```
Original episode:
  Intended gate: test (FAIL)
  Passed gates:  compile (PASS), clippy (PASS)

HER relabeling:
  Virtual goal: "produce code that compiles and passes clippy"
  Outcome:      SUCCESS (the agent achieved this goal)
  Insight:      The agent can reliably produce compile-clean, lint-clean code,
                but struggles with correctness.
```

This transforms every test-gate failure into a successful `compile+clippy` episode. The agent learns: "I am reliably competent at structural correctness; my gap is behavioral correctness." Without HER relabeling, these failures simply reinforce "I failed" without indicating where the agent's actual capability boundary lies.

HER replay in Roko uses `k=4` virtual relabelings per failed episode, with the "achieved goal" defined as the highest gate rung that passed. The relabeled episodes are tagged with `InsightRelation::HindsightRelabeled` and receive a confidence of 0.45 — moderate confidence, since the relabeled goal was not the intended objective.

### ERE: Emphasize Recent Experience

Wang & Ross (2019, "Boosting Soft Actor-Critic: Emphasizing Recent Experience without Forgetting the Past") introduced ERE, which interpolates between PER's priority-based sampling and a recency bias:

```
Effective buffer size for the k-th gradient step of episode K:
  c_k = max(N · η^(k × 1000/K), c_min)

Where:
  N     = total buffer size
  η     = 0.996 (controls how fast the window shrinks)
  K     = total gradient steps per episode
  c_min = minimum buffer size (e.g., 2,500)
```

At the start of training on a new episode, the effective buffer size is N (full buffer). As gradient steps accumulate, the effective size shrinks toward c_min, emphasizing recent experience. This prevents forgetting while still allowing learning from the current episode.

Roko mapping: the `centroid_window` parameter (default: 50) implements a softer version of this. Recent episodes contribute fully to the centroid used for Need computation; older episodes are outside the window and have their Need computed against a centroid that does not include them. ERE's η decay could replace the fixed window with a dynamic one.

### Mapping Table

| Deep RL Concept | Algorithm | Roko Dream Equivalent |
|-----------------|-----------|----------------------|
| Replay buffer | DQN | `.roko/episodes.jsonl` |
| Minibatch size | DQN | `batch_size` (default: 10) |
| Uniform sampling | DQN | Replaced by utility-weighted selection |
| TD error priority | PER | `Gain(episode)` |
| IS weight correction | PER | Spacing penalty (approximate) |
| α exponent (priority sharpness) | PER | Implicit linear formula |
| β annealing (bias correction) | PER | Not yet implemented |
| Goal relabeling | HER | Gate-based hindsight relabeling |
| k virtual goals | HER | k=4 relabelings per failed episode |
| Recency emphasis | ERE | `centroid_window` (fixed) |
| η decay parameter | ERE | Could replace `centroid_window` |
| Circular buffer eviction | DQN | Episode log GC via `roko-fs` |
| Structural clustering | N/A (Roko-native) | K-medoids over HDC vectors |
| Generative replay | Scholar (Shin 2017) | `ReplayFidelity::Generative` |

### Test Criteria for RL Connections

1. **HER relabeling**: a failed episode with passing gates {compile, clippy} produces exactly k=4 relabeled virtual episodes, all tagged `HindsightRelabeled`, all with the achieved goal set to the highest passing gate.
2. **HER confidence**: relabeled episodes receive initial confidence 0.45 ± 0.05.
3. **PER sampling distribution**: with priorities [0.9, 0.5, 0.1] and α=0.6, the sampling probabilities satisfy P(0.9)/P(0.1) ≥ 4.0.
4. **ERE window**: with `centroid_window = 50`, Need computation for an episode outside the window uses a centroid that does not include that episode's vector.
5. **Buffer eviction**: when the episode log exceeds the configured maximum, the oldest episodes are evicted first (FIFO), not the lowest-priority ones.
6. **HER gate mapping**: a fully failed episode (no gates pass) produces 0 relabeled virtual episodes (nothing to relabel as achieved).

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [01-three-phase-cycle.md](01-three-phase-cycle.md) | Overview of the three-phase dream cycle and NREM's place within it |
| [03-rem-imagination.md](03-rem-imagination.md) | REM phase that processes NREM outputs for creative recombination |
| [04-consolidation-and-staging.md](04-consolidation-and-staging.md) | Integration phase that evaluates replay outputs |
| [06-hdc-counterfactual-synthesis.md](06-hdc-counterfactual-synthesis.md) | HDC vector operations used for similarity computation and clustering |
| [../03-neuro/INDEX.md](../06-neuro/INDEX.md) | NeuroStore where replay outputs are persisted |
