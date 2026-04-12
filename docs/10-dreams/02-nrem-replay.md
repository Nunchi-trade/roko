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

## Cross-References

| Document | Relevance |
|----------|-----------|
| [01-three-phase-cycle.md](01-three-phase-cycle.md) | Overview of the three-phase dream cycle and NREM's place within it |
| [03-rem-imagination.md](03-rem-imagination.md) | REM phase that processes NREM outputs for creative recombination |
| [04-consolidation-and-staging.md](04-consolidation-and-staging.md) | Integration phase that evaluates replay outputs |
| [06-hdc-counterfactual-synthesis.md](06-hdc-counterfactual-synthesis.md) | HDC vector operations used for similarity computation and clustering |
| [../03-neuro/INDEX.md](../06-neuro/INDEX.md) | NeuroStore where replay outputs are persisted |
