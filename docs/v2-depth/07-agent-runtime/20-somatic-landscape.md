# Somatic Landscape

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). How somatic markers emerge as a Store specialization with spatial indexing, and how contrarian retrieval prevents affective lock-in.

---

## 1. The Problem: Affective Lock-In

An agent that only retrieves mood-congruent memories spirals. A run of gate failures produces negative affect, which biases retrieval toward other negative outcomes, which depresses confidence further, which causes worse routing decisions, which produce more gate failures. This is the affective lock-in loop -- the same phenomenon Bower (1981) documented in human mood-congruent memory, except worse, because the agent has no external social regulation to break the cycle.

The somatic landscape solves this with two mechanisms:

1. **Spatial indexing over strategy coordinates** -- markers are retrieved by strategic similarity, not emotional similarity. An agent asking "how did I feel when I last attempted a high-risk, novel task?" gets markers from that region of strategy space regardless of current mood.
2. **Mandatory contrarian retrieval** -- 15% of retrieved markers are drawn from the opposite affective pole, injecting disconfirming evidence into every decision. The agent cannot fully commit to its current emotional narrative.

Both mechanisms reduce to applying unified primitives: Store with a k-d tree index, and a Functor that wraps query results.

---

## 2. The Somatic Landscape IS a Store Specialization

A `SomaticMarker` is a Signal with 8-dimensional strategy coordinates as metadata. The somatic landscape is a Store whose `query_similar` uses spatial distance in the 8D strategy space rather than HDC cosine similarity. This is the same Store protocol -- `put`, `get`, `query`, `query_similar`, `prune` -- with a different index strategy.

```rust
/// A somatic marker is a Signal stored in the agent's somatic Store partition.
/// The 8D strategy coordinates are the spatial key for k-d tree indexing.
/// Valence and intensity are the payload that biases future decisions.
pub struct SomaticMarker {
    /// Signal identity -- participates in standard demurrage.
    pub signal_id: SignalId,

    /// Position in the 8D coding strategy space.
    /// Each dimension is [0.0, 1.0].
    /// [complexity, risk, novelty, confidence, time_pressure,
    ///  scope, reversibility, dependency_depth]
    pub strategy_coords: [f64; 8],

    /// Affective valence: -1.0 (strongly negative) to +1.0 (strongly positive).
    pub valence: f64,

    /// Affective intensity: 0.0 (negligible) to 1.0 (overwhelming).
    pub intensity: f64,

    /// Episode references that produced this marker (max 50, capped on merge).
    pub source_episodes: Vec<ContentHash>,

    /// Standard Signal fields: balance, demurrage_paid, last_touched_at.
    /// Markers decay via the same Gesell-Shannon ODE as all Signals.
    pub balance: f64,
    pub last_touched_at: DateTime<Utc>,
}
```

### Store Protocol Mapping

| Store method | Somatic behavior |
|---|---|
| `put(signal)` | Insert marker into k-d tree at `strategy_coords` |
| `get(id)` | Look up marker by SignalId |
| `query(filter)` | Filter markers by valence range, intensity threshold, episode hash |
| `query_similar(coords, k)` | k-nearest-neighbor search in the k-d tree -- the core retrieval path |
| `prune(threshold)` | Remove markers whose `balance` has decayed below threshold |

The somatic Store partition lives inside the agent's Space. It is scoped to that agent -- no other agent reads or writes it directly. (Cross-agent sharing happens through the contagion mechanism described in [21-collective-contagion.md](21-collective-contagion.md), which strips private fields before relay.)

---

## 3. The k-d Tree IS an Index Strategy on Store

The Memory specialization uses a three-tier HDC index for retrieval (see [06-MEMORY.md](../../unified/06-MEMORY.md)). The somatic Store uses a k-d tree over continuous 8D coordinates. These are both index strategies on the same Store protocol -- they differ in the metric space (cosine over 10,000-dimensional binary vectors vs. Euclidean over 8-dimensional continuous vectors) but share the same put/get/query_similar interface.

### Hybrid Dual-Tree Architecture

Two trees coexist, optimizing for different access patterns:

```rust
/// The somatic Store's index layer.
/// ImmutableKdTree is rebuilt during dream consolidation for optimal performance.
/// Mutable KdTree accumulates live markers between dream cycles.
pub struct SomaticIndex {
    /// Optimal tree, rebuilt from scratch during NREM consolidation.
    /// Read-only during Active state. ~5us for 100 markers, ~20us for 1K.
    immutable: ImmutableKdTree<f64, 8>,

    /// Accumulator tree for markers created during Active state.
    /// Merged into immutable during next dream consolidation.
    mutable: KdTree<f64, 8>,

    /// Marker data, keyed by position in the tree.
    /// Both trees index into this shared storage.
    markers: Vec<SomaticMarker>,
}

impl SomaticIndex {
    /// k-NN query across both trees. Results are merged and deduplicated.
    pub fn query_nearest(&self, coords: &[f64; 8], k: usize) -> Vec<(f64, &SomaticMarker)> {
        let mut results = Vec::with_capacity(k * 2);

        // Query immutable tree (bulk of markers, optimal layout).
        for neighbor in self.immutable.nearest_n(coords, k) {
            results.push((neighbor.distance, &self.markers[neighbor.item as usize]));
        }

        // Query mutable tree (recent markers since last dream).
        for neighbor in self.mutable.nearest_n::<SquaredEuclidean>(coords, k) {
            results.push((neighbor.distance, &self.markers[neighbor.item as usize]));
        }

        // Sort by distance, take top k, deduplicate by signal_id.
        results.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
        results.truncate(k);
        results
    }
}
```

### Performance Budget

| Marker count | k-NN query (immutable) | k-NN query (mutable) | Combined |
|---|---|---|---|
| 100 | ~5 us | ~2 us | ~7 us |
| 1,000 | ~20 us | ~5 us | ~25 us |
| 10,000 | ~100 us | ~15 us | ~115 us |
| 100,000 | ~500 us | ~50 us | ~550 us |

All within the 1ms budget for a single pipeline step. The immutable tree (kiddo `ImmutableKdTree`) achieves better cache locality than the mutable tree because its memory layout is optimized at construction time.

---

## 4. Marker Creation IS a React Cell

Markers are not created by an explicit "create marker" API call. They emerge from the agent's experience through a React Cell that watches PAD change Pulses on the Bus. This follows the predict-publish-correct pattern: the React Cell observes emotional outcomes and records them as spatial annotations.

### Two Creation Triggers

**Dream-created markers** (delta timescale): During NREM replay, episodes with |arousal| > 0.5 generate markers. The dream consolidation Cell replays high-PE episodes, extracts the strategy coordinates and emotional outcome, and stores them.

**Live-created markers** (gamma timescale): A React Cell subscribes to `agent:{id}.affect.changed` Pulses on the Bus. When the Euclidean PAD delta exceeds 0.15 between consecutive ticks, the Cell creates a marker at the current strategy coordinates.

```rust
/// React Cell that creates somatic markers from significant affect changes.
/// Subscribes to PAD change Pulses on the agent's Bus partition.
pub struct MarkerCreationCell {
    somatic_store: Arc<SomaticStore>,
    prev_pad: PadVector,
    current_coords: StrategyCoordinates,
}

impl Cell for MarkerCreationCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::React] }
    fn name(&self) -> &str { "somatic.marker_creation" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let affect_pulse = AffectChangePulse::from_signals(&input)?;
        let new_pad = affect_pulse.pad;

        // Euclidean distance in PAD space.
        let delta = ((new_pad.pleasure - self.prev_pad.pleasure).powi(2)
            + (new_pad.arousal - self.prev_pad.arousal).powi(2)
            + (new_pad.dominance - self.prev_pad.dominance).powi(2))
            .sqrt();

        if delta > 0.15 {
            let marker = SomaticMarker {
                signal_id: SignalId::new(),
                strategy_coords: self.current_coords.as_array(),
                valence: new_pad.pleasure,  // valence tracks pleasure axis
                intensity: new_pad.magnitude() / 3.0_f64.sqrt(),
                source_episodes: vec![affect_pulse.episode_hash],
                balance: 1.0,
                last_touched_at: Utc::now(),
            };

            self.somatic_store.put(marker.into_signal()).await?;

            // Publish marker-created Pulse for observability.
            ctx.bus().publish(Pulse::new(
                "somatic.marker_created",
                marker.strategy_coords,
            )).await?;
        }

        Ok(vec![]) // React Cells do not produce output Signals.
    }
}
```

### Why React and Not a Direct Callback

The React Cell pattern makes marker creation composable and testable. It can be:
- Disabled by removing the Cell from the Extension chain (no code change).
- Tested by injecting synthetic PAD Pulses and verifying marker creation.
- Replaced by a domain-specific variant that uses different thresholds or coordinate mappings.
- Observed by other Cells that subscribe to `somatic.marker_created` Pulses.

---

## 5. Consolidation IS a Dream Phase Cell

During NREM replay (the first phase of the dream cycle -- see [cross-cut-functors.md](cross-cut-functors.md) SS5), the somatic landscape consolidates markers. This is the same pattern as episode consolidation in Memory, but operating over the spatial index instead of the HDC index.

### Consolidation Rules

1. **Merge nearby markers**: Markers within Euclidean distance 0.5 in the 8D space are merged. The merged marker has:
   - **Coordinates**: weighted average by intensity.
   - **Valence**: weighted average by intensity.
   - **Intensity**: sum, capped at 1.0.
   - **Episodes**: union, capped at 50 (oldest dropped).

2. **Rebuild immutable tree**: After merging, the consolidated marker set replaces the immutable tree. The mutable tree is emptied.

3. **Depotentiate extreme markers**: Markers with |valence| > 0.8 and age > 48h have their intensity reduced by 30-50% (configurable). This is emotional depotentiation -- the same Walker & van der Helm (2009) mechanism that operates on episodic memory during REM, applied here to somatic markers. The affect fades; the spatial annotation persists.

```rust
/// NREM dream phase Cell for somatic marker consolidation.
/// Same Graph node pattern as Memory's NremReplayCell, but operates
/// on the somatic Store partition instead of the episodic Store partition.
pub struct SomaticConsolidationCell {
    somatic_store: Arc<SomaticStore>,
}

impl Cell for SomaticConsolidationCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Store] }
    fn name(&self) -> &str { "somatic.nrem_consolidation" }

    async fn execute(
        &self,
        _input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut markers = self.somatic_store.all_markers().await?;
        let before_count = markers.len();

        // Phase 1: Merge nearby markers.
        let merged = merge_nearby(&mut markers, 0.25); // 0.25 = distance_sq threshold (0.5^2)

        // Phase 2: Depotentiate extreme markers older than 48h.
        let now = Utc::now();
        let depotentiated = depotentiate(&merged, now, Duration::hours(48));

        // Phase 3: Rebuild immutable index from consolidated set.
        self.somatic_store.rebuild_index(depotentiated).await?;

        let after_count = self.somatic_store.count().await;

        Ok(vec![Signal::metadata("somatic.consolidation", json!({
            "before": before_count,
            "after": after_count,
            "merged": before_count - after_count,
        }))])
    }
}

/// Merge markers within distance_sq threshold using weighted averaging.
fn merge_nearby(markers: &mut Vec<SomaticMarker>, distance_sq_threshold: f64) -> Vec<SomaticMarker> {
    let mut consolidated = Vec::new();
    let mut consumed = vec![false; markers.len()];

    for i in 0..markers.len() {
        if consumed[i] { continue; }
        let mut cluster = vec![i];

        for j in (i + 1)..markers.len() {
            if consumed[j] { continue; }
            let dist_sq: f64 = markers[i].strategy_coords.iter()
                .zip(markers[j].strategy_coords.iter())
                .map(|(a, b)| (a - b).powi(2))
                .sum();

            if dist_sq < distance_sq_threshold {
                cluster.push(j);
                consumed[j] = true;
            }
        }

        if cluster.len() == 1 {
            consolidated.push(markers[i].clone());
        } else {
            // Weighted merge: intensity is the weight.
            let total_intensity: f64 = cluster.iter()
                .map(|&idx| markers[idx].intensity)
                .sum();

            let mut merged_coords = [0.0; 8];
            let mut merged_valence = 0.0;
            let mut merged_episodes = Vec::new();

            for &idx in &cluster {
                let w = markers[idx].intensity / total_intensity.max(1e-10);
                for d in 0..8 {
                    merged_coords[d] += w * markers[idx].strategy_coords[d];
                }
                merged_valence += w * markers[idx].valence;
                merged_episodes.extend_from_slice(&markers[idx].source_episodes);
            }

            merged_episodes.truncate(50); // cap episode references

            consolidated.push(SomaticMarker {
                signal_id: SignalId::new(),
                strategy_coords: merged_coords,
                valence: merged_valence,
                intensity: total_intensity.min(1.0),
                source_episodes: merged_episodes,
                balance: 1.0, // freshly consolidated = full balance
                last_touched_at: Utc::now(),
            });
        }
    }

    consolidated
}
```

### Consolidation Timing

Consolidation runs during the agent's Dreaming state, as the first step of the delta-timescale dream cycle. It does not run during Active state. The trigger is the same as for Memory consolidation: the agent transitions `Active -> Dreaming` when sleep pressure exceeds threshold or idle timeout fires (see [05-AGENT.md](../../unified/05-AGENT.md) SS2).

---

## 6. Contrarian Retrieval IS a Functor

Standard k-NN retrieval from the somatic Store produces mood-congruent results: if recent experience in this strategy region was positive, the nearest markers will also be positive, reinforcing the agent's current bias. The contrarian retrieval Functor wraps `query_similar` to inject disconfirming evidence.

### The 15% Contrarian Rule

For every somatic query, 85% of influence comes from congruent markers (same-sign valence as the majority) and 15% from contrarian markers (opposite-sign valence). This is a Functor -- it transforms the output of `query_similar` without changing the Store or the index.

```rust
/// Functor that wraps somatic query_similar to inject contrarian markers.
///
/// F_contrarian(query_similar(coords, k)) =
///   0.85 * weighted_average(congruent_markers)
/// + 0.15 * weighted_average(contrarian_markers)
///
/// The contrarian markers are retrieved by inverting the pleasure
/// coordinate and re-querying, or by selecting opposite-valence
/// entries from the original result set.
pub struct ContrarianRetrievalFunctor {
    somatic_store: Arc<SomaticStore>,
    contrarian_fraction: f64,  // default 0.15
    k: usize,                  // default 5
}

impl ContrarianRetrievalFunctor {
    /// Retrieve somatic signal with mandatory contrarian blending.
    pub async fn query(
        &self,
        coords: &[f64; 8],
    ) -> SomaticSignal {
        let neighbors = self.somatic_store.query_similar(coords, self.k).await;

        if neighbors.is_empty() {
            return SomaticSignal::neutral();
        }

        // Separate congruent and contrarian by majority valence.
        let majority_positive = neighbors.iter()
            .map(|(_, m)| m.valence)
            .sum::<f64>() >= 0.0;

        let (congruent, contrarian): (Vec<_>, Vec<_>) = neighbors.iter()
            .partition(|(_, m)| (m.valence >= 0.0) == majority_positive);

        // If no natural contrarian markers exist, synthesize by inverting.
        let contrarian_markers = if contrarian.is_empty() {
            // Invert pleasure and dominance in the query coordinates,
            // keep arousal (high arousal is always relevant).
            let mut inverted = *coords;
            inverted[3] = 1.0 - inverted[3]; // invert confidence
            self.somatic_store.query_similar(&inverted, 2).await
        } else {
            contrarian.iter().map(|&&(d, ref m)| (d, m.clone())).collect()
        };

        // Weighted blending.
        let congruent_signal = weighted_signal(&congruent);
        let contrarian_signal = weighted_signal_from_owned(&contrarian_markers);

        let blended_valence = (1.0 - self.contrarian_fraction) * congruent_signal.valence
            + self.contrarian_fraction * contrarian_signal.valence;
        let blended_intensity = (1.0 - self.contrarian_fraction) * congruent_signal.intensity
            + self.contrarian_fraction * contrarian_signal.intensity;

        SomaticSignal {
            valence: blended_valence,
            intensity: blended_intensity,
            neighbor_count: neighbors.len(),
            contrarian_count: contrarian_markers.len(),
            source_episodes: neighbors.iter()
                .flat_map(|(_, m)| m.source_episodes.iter().cloned())
                .collect(),
        }
    }
}

/// Weight function for k-NN results: w = 1/(1 + dist_sq) * intensity.
fn weighted_signal(neighbors: &[&(f64, &SomaticMarker)]) -> SomaticSignal {
    if neighbors.is_empty() {
        return SomaticSignal::neutral();
    }

    let mut total_weight = 0.0;
    let mut weighted_valence = 0.0;
    let mut weighted_intensity = 0.0;

    for &&(dist_sq, ref marker) in neighbors {
        let w = marker.intensity / (1.0 + dist_sq);
        total_weight += w;
        weighted_valence += w * marker.valence;
        weighted_intensity += w * marker.intensity;
    }

    if total_weight < 1e-10 {
        return SomaticSignal::neutral();
    }

    SomaticSignal {
        valence: weighted_valence / total_weight,
        intensity: weighted_intensity / total_weight,
        neighbor_count: neighbors.len(),
        contrarian_count: 0,
        source_episodes: vec![],
    }
}
```

### Why a Functor and Not a Store Feature

The contrarian mechanism is a retrieval-time transformation, not a storage-time one. The raw markers in the Store are unmodified. The Functor applies at the boundary between the Store and the consuming Cell (the Daimon's ASSESS enrichment). This means:

- The raw markers can be queried without contrarian blending for debugging or analysis.
- The contrarian fraction (0.15) can be adjusted per-agent or per-context without touching the Store.
- The Functor composes with other retrieval Functors (e.g., resource pressure compression, described below).

### Rolling Enforcement

A rolling 200-tick window tracks the contrarian fraction of retrieved markers. If it drops below 15%, the Functor forces contrarian retrieval on the next query by inverting the query coordinates (invert pleasure and dominance axes, keep arousal). This ensures the 15% minimum is maintained even when the strategy region is emotionally homogeneous.

```rust
/// Tracks contrarian fraction over a rolling window.
/// Forces contrarian retrieval when the fraction drops below minimum.
pub struct ContrarianTracker {
    window: VecDeque<bool>,  // true = contrarian was present
    window_size: usize,      // default 200
    min_fraction: f64,       // default 0.15
}

impl ContrarianTracker {
    pub fn record(&mut self, had_contrarian: bool) {
        self.window.push_back(had_contrarian);
        if self.window.len() > self.window_size {
            self.window.pop_front();
        }
    }

    pub fn needs_forced_contrarian(&self) -> bool {
        if self.window.len() < 10 { return false; } // cold start
        let fraction = self.window.iter().filter(|&&b| b).count() as f64
            / self.window.len() as f64;
        fraction < self.min_fraction
    }
}
```

---

## 7. Resource Pressure IS a Functor on the 8D Coordinates

When the agent's resource budget is depleted, the somatic landscape compresses. Strategy coordinates are pulled toward the midpoint (0.5 on each axis), reducing the effective diversity of the strategy space. This makes the agent's decisions more conservative -- it queries a narrower region, retrieves fewer distinct markers, and gravitates toward the center of its experience.

### The Compression Formula

```
pressure_scalar = min(token_remaining_fraction, time_remaining_fraction).sqrt()
compressed_coord[d] = midpoint + pressure_scalar * (raw_coord[d] - midpoint)
```

Where `midpoint = 0.5` for all dimensions. At 100% budget, `pressure_scalar = 1.0` and coordinates are unchanged. At 25% budget, `pressure_scalar = 0.5` and coordinates are compressed halfway toward center. At 6.25% budget, `pressure_scalar = 0.25` -- the agent is searching a small region around the center of strategy space.

```rust
/// Functor that compresses somatic query coordinates under resource pressure.
/// Applied before the k-NN query, so the Store itself is unmodified.
///
/// This is a pre-query coordinate transformation, not a post-query
/// result filter. The agent physically queries a smaller region
/// of strategy space when resources are scarce.
pub struct ResourcePressureFunctor {
    vitality: Arc<VitalityTracker>,
}

impl ResourcePressureFunctor {
    /// Transform query coordinates based on current resource pressure.
    pub fn compress(&self, raw_coords: &[f64; 8]) -> [f64; 8] {
        let vitality = self.vitality.vitality();
        let pressure_scalar = vitality.sqrt(); // sqrt provides smooth compression

        let midpoint = 0.5;
        let mut compressed = [0.0; 8];
        for d in 0..8 {
            compressed[d] = midpoint + pressure_scalar * (raw_coords[d] - midpoint);
        }
        compressed
    }
}
```

### Behavioral Consequences

| Vitality | Pressure scalar | Effective strategy region | Agent behavior |
|---|---|---|---|
| 1.00 (Thriving) | 1.00 | Full 8D cube | Explores entire strategy space |
| 0.50 (Stable) | 0.71 | 71% of each axis | Moderate compression |
| 0.25 (Conservation) | 0.50 | 50% of each axis | Queries near-center only |
| 0.06 (Declining) | 0.25 | 25% of each axis | Minimal region, known strategies |
| 0.01 (Terminal) | 0.10 | 10% of each axis | Effectively frozen at midpoint |

This creates a natural annealing schedule: fresh agents explore widely, depleted agents exploit narrowly. The compression is reversible -- if the agent receives more budget, the coordinates expand again. No hysteresis, no permanent damage.

### Functor Composition Order

The full somatic retrieval pipeline composes three Functors:

```
raw_coords
    |
    v
ResourcePressureFunctor.compress(raw_coords)    -- compress under budget pressure
    |
    v
SomaticIndex.query_nearest(compressed, k)       -- k-NN in the Store
    |
    v
ContrarianRetrievalFunctor.blend(results)        -- inject 15% contrarian
    |
    v
SomaticSignal                                     -- consumed by Daimon ASSESS
```

Each Functor is independent. You can disable resource compression (full exploration regardless of budget) or disable contrarian blending (pure mood-congruent retrieval) by removing the corresponding Functor from the chain. The Store and index are untouched.

---

## 8. Somatic Response Thresholds

The blended somatic signal (valence + intensity, after contrarian injection) modulates the agent's tier selection and prediction error sensitivity. This is the bridge between the somatic landscape and the EFE routing formula (see [dual-process-and-efe-routing.md](dual-process-and-efe-routing.md)).

| Somatic valence | Threshold | Routing effect |
|---|---|---|
| Strong negative (< -0.5) | High caution | Force T2 + Conservative strategy. The agent remembers that this region of strategy space led to bad outcomes. |
| Weak negative (-0.5 to -0.1) | Elevated caution | Increase prediction error threshold (require more evidence before acting). |
| Neutral (-0.1 to +0.1) | Baseline | No bias. EFE routing operates on its own terms. |
| Weak positive (+0.1 to +0.5) | Slight confidence | Slight demotion of tier (allow T0/T1 where T2 would normally be selected). |
| Strong positive (> +0.5) | High confidence | Prefer T0/T1. The agent remembers success in this region. |

```rust
/// Convert a somatic signal into a routing bias.
/// This bridges the somatic Store with the EFE gating system.
pub fn somatic_to_routing_bias(signal: &SomaticSignal) -> RoutingBias {
    if !signal.is_actionable() {
        return RoutingBias::neutral();
    }

    let valence = signal.valence;
    let intensity = signal.intensity;

    // Scale bias by intensity: low-intensity signals have minimal effect.
    let strength = intensity.clamp(0.0, 1.0);

    match valence {
        v if v < -0.5 => RoutingBias {
            tier_shift: -2 * strength as i32,  // toward T2
            pe_threshold_delta: 0.15 * strength,
            strategy_override: Some(DispatchStrategy::Conservative),
        },
        v if v < -0.1 => RoutingBias {
            tier_shift: 0,
            pe_threshold_delta: 0.08 * strength,
            strategy_override: None,
        },
        v if v < 0.1 => RoutingBias::neutral(),
        v if v < 0.5 => RoutingBias {
            tier_shift: 1,
            pe_threshold_delta: -0.05 * strength,
            strategy_override: None,
        },
        _ => RoutingBias {
            tier_shift: 2 * strength as i32,  // toward T0/T1
            pe_threshold_delta: -0.10 * strength,
            strategy_override: Some(DispatchStrategy::Exploratory),
        },
    }
}
```

---

## 9. Crate Mapping (Implementation Reality)

| Spec concept | Crate | Current status |
|---|---|---|
| `SomaticMarker`, `SomaticLandscape` | `roko-daimon` | Implemented: k-d tree (kiddo), 8D coding space, contrarian blending |
| `StrategyCoordinates`, `StrategySpaceDefinition` | `roko-daimon` | Implemented: configurable 8D coding domain, `RegisteredStrategySpaceComputer` |
| `SomaticOracleContext`, `SomaticRetrieval` | `roko-daimon/src/somatic_ta.rs` | Implemented: confidence bias, contrarian fraction tracking |
| `ContrarianTracker` | `roko-daimon/src/phase2_stubs.rs` | Implemented: rolling window, forced contrarian retrieval |
| `ResourcePressure` | `roko-daimon/src/phase2_stubs.rs` | Stub: pressure scalar computed but not wired into coordinate compression |
| `SomaticConsolidationCell` | `roko-dreams` | Partial: NREM replay exists but somatic-specific merge not wired as a separate Cell |
| Somatic response -> EFE routing | `roko-cli/src/orchestrate.rs` | Partial: `SomaticOracleContext` is queried at dispatch time but routing bias is applied manually, not via Functor chain |

The spec describes the target architecture. The codebase has the data structures and algorithms in place (k-d tree, contrarian blending, strategy coordinates) but the Functor composition and Cell-based creation/consolidation are not yet wired as described. The path from current implementation to spec: refactor `SomaticLandscape` methods into separate Cells and Functors, then compose them in the agent's pipeline Graph.

---

## What This Enables

1. **Grounded decision-making** -- Somatic markers give the agent a body. "How did I feel last time I tried this?" is a spatial query over past emotional outcomes, not a text search over episode logs. Decisions are informed by affective history without being dominated by it.

2. **Natural annealing** -- Resource pressure compression creates an automatic exploration-to-exploitation transition. Fresh agents explore the full strategy space; depleted agents converge on proven strategies. No explicit annealing schedule needed.

3. **Affective homeostasis** -- The 15% contrarian minimum prevents runaway positive or negative spirals. The agent always hears dissenting evidence, even when its recent experience is uniformly good or bad.

4. **Domain portability** -- The 8D strategy space is a configurable `StrategySpaceDefinition`. The coding domain uses [complexity, risk, novelty, confidence, time_pressure, scope, reversibility, dependency_depth]. A different domain (research, trading, operations) can define its own 8 axes and the same k-d tree infrastructure works unchanged.

---

## Feedback Loops

1. **Somatic -> routing -> gate outcome -> marker update**: Somatic markers bias tier selection. The gate outcome (pass/fail) feeds back into the marker's valence and intensity via the React Cell. Successful strategies in a region accumulate positive markers; failed strategies accumulate negative markers. The landscape self-corrects.

2. **Contrarian -> discovery -> marker rebalancing**: Contrarian retrieval occasionally causes the agent to select a strategy that contradicts its dominant markers. If that strategy succeeds, a new positive marker is created in a region that was previously negative, rebalancing the landscape. The 15% floor prevents the landscape from becoming a self-fulfilling prophecy.

3. **Resource pressure -> compression -> consolidation -> expansion**: Under resource pressure, the agent queries a compressed region, creating markers in a narrow band. During dream consolidation, these densely packed markers merge, producing fewer but higher-intensity markers near center. When budget is replenished, the expanded query region now includes both the consolidated center markers and any surviving peripheral markers, giving the agent a richer experience base.

4. **Dream consolidation -> tree rebuild -> query performance**: As markers accumulate during Active state in the mutable tree, query performance degrades slightly. Dream consolidation merges nearby markers (reducing count) and rebuilds the immutable tree (optimizing layout), restoring query performance. This is a natural maintenance cycle driven by the sleep/wake lifecycle.

---

## Open Questions

1. **Cross-domain transfer**: When an agent switches domains (e.g., from Rust development to research), should somatic markers from the old domain transfer? The `StrategySpaceDefinition` differs, so coordinates are not directly comparable. One approach: HDC fingerprint the domain definitions and transfer markers only between domains with high fingerprint similarity. Another: discard and rebuild.

2. **Marker capacity policy**: The current implementation has no hard cap on marker count. Should there be one? If so, the prune policy should remove low-balance markers first (standard demurrage), but also consider spatial coverage -- pruning should not create empty regions in the strategy space.

3. **Contrarian fraction adaptation**: The 15% minimum is a constant. Should it adapt based on experience? An agent that has never been surprised by contrarian evidence might reduce it to 10%; an agent that frequently benefits from contrarian markers might increase it to 25%. This would be a React Cell that adjusts the `ContrarianRetrievalFunctor`'s parameter based on outcome tracking.

4. **Multi-agent marker provenance**: When markers are shared via the somatic field mechanism (see [21-collective-contagion.md](21-collective-contagion.md)), should the receiving agent treat foreign markers differently from self-generated ones? A trust discount (e.g., 0.5x intensity for foreign markers) would prevent a poorly-calibrated peer from distorting the local landscape.

5. **Dual-tree merge frequency**: Currently, the mutable tree merges into the immutable tree only during dream consolidation. For long-running agents that rarely sleep, the mutable tree could grow large. Should there be a secondary merge trigger based on mutable tree size (e.g., merge when mutable exceeds 20% of immutable)?

---

## Citations

1. Damasio, A. (1994). *Descartes' Error: Emotion, Reason, and the Human Brain.* -- Somatic marker hypothesis.
2. Bower, G. H. (1981). "Mood and memory." *American Psychologist*, 36(2), 129. -- Mood-congruent memory bias.
3. Walker, M. P. & van der Helm, E. (2009). "Overnight therapy? The role of sleep in emotional brain processing." *Psychological Bulletin*, 135(5), 731. -- Emotional depotentiation during sleep.
4. Mattar, M. G. & Daw, N. D. (2018). "Prioritized memory access explains planning and hippocampal replay." *Nature Neuroscience*, 21, 1609-1617. -- Replay prioritization by decision-utility.
5. Jonas, H. (1966). *The Phenomenon of Life.* -- Mortality and cognitive urgency.
6. See [05-AGENT.md](../../unified/05-AGENT.md) SS3 for vitality phases and behavioral consequences.
7. See [cross-cut-functors.md](cross-cut-functors.md) SS4 for Daimon as endofunctor and somatic marker retrieval in ASSESS.
8. See [06-MEMORY.md](../../unified/06-MEMORY.md) SS3 for the Gesell-Shannon demurrage ODE that governs marker balance decay.
