# Somatic Markers (Damasio)

> Emotional memory as a fast heuristic — the k-d tree over the 8-dimensional strategy space that lets agents make sub-millisecond decisions before analytical reasoning engages.

**Topic**: [Daimon](./INDEX.md)
**Prerequisites**: [01-pad-vector.md](./01-pad-vector.md), [08-8-dimensional-strategy-space.md](./08-8-dimensional-strategy-space.md)
**Key sources**: `refactoring-prd/09-innovations.md` §III, `refactoring-prd/03-cognitive-subsystems.md` §2, `bardo-backup/prd/03-daimon/01-appraisal.md`, `bardo-backup/prd/03-daimon/02-emotion-memory.md`

---

## Abstract

Damasio's somatic marker hypothesis (1994) proposes that emotions mark past experiences with "gut feelings" that speed future decisions. When a person encounters a situation similar to one they've experienced before, their body generates a somatic response — a flush of anxiety, a sense of confidence, a feeling of unease — before conscious reasoning engages. This System 1 response narrows the decision space, directing analytical (System 2) attention toward promising options and away from dangerous ones.

The Daimon implements this as a **k-d tree over the 8-dimensional strategy space**. Before the agent selects an action, it queries the somatic landscape: "What does this region of strategy space *feel like*?" If nearby markers carry strong negative valence, the agent routes to stronger models and increases review scrutiny. If nearby markers carry strong positive valence, the agent proceeds with confidence on cheaper models. This query takes less than 1 millisecond — it is the fastest decision signal in the entire cognitive architecture, operating before the prediction error probes, before the tier router, and before analytical model inference.

The system implements mandatory 15% contrarian retrieval to prevent the somatic landscape from becoming an echo chamber that reinforces confirmation bias (Bower 1981).

---

## Theoretical Foundation

### The Somatic Marker Hypothesis

Damasio (1994) proposed the somatic marker hypothesis based on observations of patients with ventromedial prefrontal cortex damage. These patients retained normal IQ and logical reasoning ability but lost the ability to make advantageous decisions in real-world situations — they could reason about options but couldn't feel which options were dangerous.

The key findings from the Iowa Gambling Task (Bechara et al. 1994, 1997):

1. **Normal subjects** develop anticipatory skin conductance responses (somatic markers) before reaching for disadvantageous card decks — they "feel" the danger before they can articulate it.
2. **Patients with vmPFC damage** never develop these anticipatory responses. They can explain the logic of the task but continue choosing badly.
3. **The somatic response precedes conscious awareness** — subjects show physiological markers of danger before they report any knowledge of which decks are bad.

**Implication for agents**: An agent without somatic markers must reason through every decision from first principles. An agent with somatic markers has a fast pre-filter that narrows the search space before expensive reasoning begins. This is the computational analogy: somatic markers provide O(log n) approximate evaluation before O(n) exact evaluation.

### Why Not Just Use the PAD Vector?

The PAD vector tracks the agent's current mood — a global emotional state. Somatic markers are different: they are **situation-specific emotional memories**. The PAD vector says "I feel anxious right now." A somatic marker says "The last time I was in a situation like *this specific one*, it went badly."

The distinction matters because the same agent mood can produce different decisions depending on the strategic context. An anxious agent encountering a familiar successful pattern should proceed with caution but not retreat. An anxious agent encountering a pattern associated with past failures should escalate immediately. The somatic landscape provides the situation-specific signal that the global PAD vector cannot.

---

## The Somatic Landscape

### Data Structure

The somatic landscape is a k-d tree over the 8-dimensional strategy space. Each node in the tree is a somatic marker — a record of a past strategy's emotional outcome:

```rust
pub struct SomaticLandscape {
    tree: KdTree<f64, SomaticMarker, 8>,
}

pub struct SomaticMarker {
    /// Coordinates in the 8D strategy space.
    /// Dimensions are domain-configurable (see 08-8-dimensional-strategy-space.md).
    pub strategy_coords: [f64; 8],

    /// Emotional valence: positive (+1) = this worked well;
    /// negative (-1) = this went badly. The sign determines
    /// whether the marker promotes or inhibits the strategy.
    pub valence: f64,

    /// Emotional intensity: how strong the feeling was (0 to 1).
    /// Higher intensity means the marker fires more strongly
    /// when a similar situation is encountered.
    pub intensity: f64,

    /// Content hashes of the episodes that formed this marker.
    /// Provides provenance: which specific experiences created
    /// this gut feeling?
    pub episodes: Vec<ContentHash>,
}
```

The k-d tree provides efficient nearest-neighbor queries in 8 dimensions. For a landscape with N markers, nearest-neighbor search is O(log N) average case. With the `kiddo` crate (specified in the legacy infrastructure doc), 10,000 markers produce query times under 100 microseconds.

### Marker Creation

Somatic markers are created by the dream consolidation process and by significant live events:

**Dream-created markers**: During NREM replay, the dream engine processes past episodes and identifies patterns. Episodes with strong emotional charge (|arousal| > 0.5) that have clear strategy coordinates are distilled into somatic markers. The marker's valence comes from the episode's pleasure dimension; the intensity from the arousal dimension.

**Live-created markers**: When a task outcome produces a PAD delta exceeding the emission threshold (0.15 Euclidean), the appraisal engine creates a somatic marker at the current strategy coordinates. Live markers have higher initial intensity than dream markers because they haven't been depotentiated.

```rust
impl SomaticLandscape {
    /// Create a marker from a task outcome.
    pub fn record_outcome(
        &mut self,
        strategy_coords: [f64; 8],
        pleasure: f64,
        arousal: f64,
        episode_hash: ContentHash,
    ) {
        let marker = SomaticMarker {
            strategy_coords,
            valence: pleasure,  // positive outcome → positive valence
            intensity: arousal.abs().min(1.0),
            episodes: vec![episode_hash],
        };
        self.tree.add(&strategy_coords, marker);
    }
}
```

### Marker Consolidation

Multiple markers in the same region of strategy space consolidate over time. When two markers have strategy coordinates within Euclidean distance 0.5, the dream engine merges them:

```rust
fn consolidate_markers(a: &SomaticMarker, b: &SomaticMarker) -> SomaticMarker {
    // Weighted average: more intense markers dominate
    let total_intensity = a.intensity + b.intensity;
    let w_a = a.intensity / total_intensity;
    let w_b = b.intensity / total_intensity;

    let mut coords = [0.0; 8];
    for i in 0..8 {
        coords[i] = w_a * a.strategy_coords[i] + w_b * b.strategy_coords[i];
    }

    SomaticMarker {
        strategy_coords: coords,
        valence: w_a * a.valence + w_b * b.valence,
        intensity: total_intensity.min(1.0),
        episodes: [a.episodes.clone(), b.episodes.clone()].concat(),
    }
}
```

Consolidation prevents unbounded growth of the k-d tree while preserving the aggregate emotional signal. A region with many positive markers consolidates into a single strong positive marker. A region with mixed markers consolidates into a weak marker, reflecting genuine ambiguity.

---

## Querying the Somatic Landscape

### Pre-Action Query

Before selecting an action, the agent queries the somatic landscape with the proposed strategy's coordinates:

```rust
impl SomaticLandscape {
    /// Query the somatic landscape for emotional valence near a strategy.
    ///
    /// Returns a SomaticSignal with the aggregate valence and intensity
    /// of nearby markers, plus the mandatory contrarian component.
    pub fn query(
        &self,
        strategy_coords: &[f64; 8],
        k: usize,          // number of nearest neighbors (default: 5)
        contrarian_k: usize, // contrarian neighbors (default: 1)
    ) -> SomaticSignal {
        // Phase 1: Find k nearest neighbors
        let neighbors = self.tree.nearest(strategy_coords, k, &squared_euclidean);

        // Phase 2: Compute weighted valence
        let mut total_valence = 0.0;
        let mut total_weight = 0.0;
        for (dist_sq, marker) in &neighbors {
            let distance_weight = 1.0 / (1.0 + dist_sq); // inverse distance
            let weight = distance_weight * marker.intensity;
            total_valence += weight * marker.valence;
            total_weight += weight;
        }
        let congruent_valence = if total_weight > 0.0 {
            total_valence / total_weight
        } else {
            0.0  // no markers nearby → neutral
        };

        // Phase 3: Mandatory contrarian retrieval (15%)
        let contrarian = self.query_contrarian(strategy_coords, congruent_valence, contrarian_k);

        // Phase 4: Blend (85% congruent, 15% contrarian)
        let blended_valence = 0.85 * congruent_valence + 0.15 * contrarian.valence;

        SomaticSignal {
            valence: blended_valence,
            intensity: total_weight.min(1.0),
            neighbor_count: neighbors.len(),
            contrarian_count: contrarian.count,
        }
    }
}
```

### Response to Somatic Signal

The somatic signal modulates the agent's behavior before analytical reasoning:

| Signal | Agent Response |
|---|---|
| Strong negative valence (< -0.5) | Route to T2 (deep reasoning), increase review scrutiny, activate Conservative strategy |
| Weak negative valence (-0.5 to -0.2) | Increase prediction error threshold modestly, prefer proven playbooks |
| Neutral (-0.2 to 0.2) | No somatic bias — let prediction error and tier router decide |
| Weak positive valence (0.2 to 0.5) | Slight model demotion, prefer cached strategies |
| Strong positive valence (> 0.5) | Route to T0/T1, exploit known patterns, minimal review overhead |

### Latency Budget

The somatic query must complete within 1 millisecond to serve as a pre-analytical heuristic. With `kiddo`'s k-d tree:

| Landscape Size | 5-NN Query Time | Budget |
|---|---|---|
| 100 markers | ~5 µs | Well within budget |
| 1,000 markers | ~20 µs | Within budget |
| 10,000 markers | ~100 µs | Within budget |
| 100,000 markers | ~500 µs | Within budget |

The k-d tree is rebuilt during dream consolidation (offline), not during live queries. Live insertions use `kiddo`'s incremental insert, which is O(log N).

---

## Somatic Events

When a somatic marker fires strongly (|valence| > 0.3 and intensity > 0.5), the system emits a `SomaticMarkerFired` event:

```rust
pub struct SomaticMarkerFiredEvent {
    /// Description of the situation that triggered the marker.
    pub situation: String,
    /// Valence of the fired marker.
    pub valence: f64,
    /// Which episodes formed this marker.
    pub source_episodes: Vec<ContentHash>,
    /// Strategy parameter that was modified.
    pub strategy_param: String,
}
```

This event is consumed by:
- The TUI (particle effect: `somatic_flash`, 500ms duration)
- The episode logger (records that a somatic marker influenced this decision)
- The emotional provenance tracker (tracks which decisions were somatic-guided vs. analytically-derived)

---

## Interaction with Other Daimon Components

### Somatic Markers and the PAD Vector

Somatic markers and the PAD vector operate at different timescales and granularities:

| Property | PAD Vector | Somatic Markers |
|---|---|---|
| Scope | Global mood | Situation-specific |
| Timescale | Seconds to hours (ALMA layers) | Persistent (until dream consolidation) |
| Creation | Every appraisal event | Significant outcomes only |
| Query cost | O(1) — direct field read | O(log N) — k-d tree search |
| Decay | Exponential (4h half-life) | Slow (dream-managed) |

The two systems are complementary. The PAD vector provides the current emotional context for the decision. The somatic landscape provides historical emotional context for similar decisions. Together, they implement Damasio's full somatic marker framework: current feeling + remembered feeling → decision bias.

### Somatic Markers and Dream Consolidation

Dreams maintain the somatic landscape through three operations:

1. **Creation**: New markers from emotionally significant episodes
2. **Consolidation**: Merging nearby markers to prevent unbounded growth
3. **Depotentiation**: Reducing intensity of markers with high arousal (Walker & van der Helm 2009) — a marker that originally fired at intensity 0.9 may be reduced to 0.6 after dreaming, reflecting emotional processing

The dream engine's consolidation pass over the somatic landscape is part of the NREM phase. It runs at Delta frequency (idle time) and is not latency-sensitive.

---

## Academic Foundations

- Damasio, A.R. (1994). *Descartes' Error: Emotion, Reason, and the Human Brain*. Putnam.
- Bechara, A., Damasio, A.R., Damasio, H., & Anderson, S.W. (1994). "Insensitivity to future consequences following damage to human prefrontal cortex." *Cognition*, 50, 7–15.
- Bechara, A., Damasio, H., Tranel, D., & Damasio, A.R. (1997). "Deciding advantageously before knowing the advantageous strategy." *Science*, 275(5304), 1293–1295.
- Bower, G.H. (1981). "Mood and Memory." *American Psychologist*, 36(2), 129–148.
- Walker, M.P. & van der Helm, E. (2009). "Overnight therapy? The role of sleep in emotional brain processing." *Psychological Bulletin*, 135(5), 731–748.
- Kahneman, D. (2011). *Thinking, Fast and Slow*. Farrar, Straus and Giroux.

---

## Current Status and Gaps

**Specified**: Full `SomaticLandscape` and `SomaticMarker` structs in `refactoring-prd/09-innovations.md` §III. Query protocol, contrarian retrieval, response mapping, event emission.

**Not implemented**: The somatic landscape is not yet built in `roko-daimon`. The legacy `golem-daimon` crate specified `kiddo` as the k-d tree dependency, but no code exists for the landscape data structure or query methods. This is the Daimon's largest implementation gap.

**Dependencies**: Requires the 8-dimensional strategy space definition (see [08-8-dimensional-strategy-space.md](./08-8-dimensional-strategy-space.md)) and dream consolidation infrastructure (see topic [03-dreams](../03-dreams/INDEX.md)).

---

## Cross-references

- See [07-15-percent-contrarian-retrieval.md](./07-15-percent-contrarian-retrieval.md) for contrarian retrieval mechanism
- See [08-8-dimensional-strategy-space.md](./08-8-dimensional-strategy-space.md) for strategy space dimensions
- See [09-mood-congruent-memory.md](./09-mood-congruent-memory.md) for how somatic markers relate to mood-congruent retrieval
- See [10-integration-points.md](./10-integration-points.md) for somatic landscape as integration point
- See topic [03-dreams](../03-dreams/INDEX.md) for dream consolidation of somatic markers
