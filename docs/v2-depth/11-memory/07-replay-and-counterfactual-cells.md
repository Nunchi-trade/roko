# Replay and Counterfactual Cells

> Depth for [06-MEMORY.md](../../unified/06-MEMORY.md). How episode replay and counterfactual reasoning decompose into Score, Route, Compose, and Verify Cells within the dream Loop.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, demurrage, HDC fingerprints, PAD metadata), [02-CELL](../../unified/02-CELL.md) (9 protocols, predict-publish-correct, Verify redesign, EFE routing), [03-GRAPH](../../unified/03-GRAPH.md) (Graph, Loop), [04-EXECUTION](../../unified/04-EXECUTION.md) (Engine, Hot Graph, Flow), [05-AGENT](../../unified/05-AGENT.md) (Dreaming state, CorticalState, somatic markers, CognitiveWorkspace), [06-MEMORY](../../unified/06-MEMORY.md) (Memory specialization, demurrage economics, dream consolidation, HDC operations, AntiKnowledge), [07-LEARNING](../../unified/07-LEARNING.md) (L1-L4 Loop taxonomy, predict-publish-correct)

**Source docs**: `docs/10-dreams/02-nrem-replay.md`, `docs/10-dreams/03-rem-imagination.md`, `docs/10-dreams/06-hdc-counterfactual-synthesis.md`, `docs/10-dreams/04-consolidation-and-staging.md`, `docs/10-dreams/09-threat-simulation.md`

---

## 1. The Dream Graph

The dream cycle is not a special subsystem. It is a Loop Graph -- the same kind of Graph that runs the cognitive pipeline, the routing cascade, and the heuristic calibration loop. Dream execution uses the same Engine ([04-EXECUTION](../../unified/04-EXECUTION.md)), the same Flow lifecycle, and the same budget enforcement as every other Graph in the system.

The prior design treated NREM replay, REM imagination, hindsight relabeling, and consolidation as four sequential phases inside a monolithic `DreamCycle::run`. The unified redesign factors them into composable Cells wired by typed edges. Each Cell has a single protocol responsibility. The Graph topology -- not the code -- determines which phases run and in what order.

```
            +------------------+
            |  Episode Store   |  Store protocol
            +--------+---------+
                     |
                     v
            +------------------+
            |  Replay Score    |  Score protocol (Mattar-Daw utility)
            +--------+---------+
                     |
                     v
            +------------------+
            |  Budget Route    |  Route protocol (NREM/REM/HER budget split)
            +--------+---------+
                     |
              +------+------+
              |      |      |
              v      v      v
         +------+ +-----+ +------+
         | NREM | | REM | | HER  |   Three Compose/Verify sub-graphs
         +--+---+ +--+--+ +--+---+
              |      |      |
              +------+------+
                     |
                     v
            +------------------+
            |  Dedup Verify    |  Verify protocol (HDC deduplication)
            +--------+---------+
                     |
                     v
            +------------------+
            |  Staging Store   |  Store protocol (confidence ladder)
            +--------+---------+
                     |
                     v
            +------------------+
            |  Depotentiation  |  Functor (PAD arousal reduction)
            +--------+---------+
                     |
            feedback edge back to Episode Store
```

```toml
[graph]
name = "dream-cycle"
loop = true
convergence = { max_iterations = 5, min_delta = 0.01 }

[[nodes]]
id = "episode-store"
cell = "roko:episode-store"
protocol = "Store"

[[nodes]]
id = "replay-scorer"
cell = "roko:mattar-daw-scorer"
protocol = "Score"

[[nodes]]
id = "budget-router"
cell = "roko:dream-budget-router"
protocol = "Route"

[[nodes]]
id = "nrem-replay"
cell = "roko:nrem-replay-graph"
protocol = "Compose"

[[nodes]]
id = "rem-imagination"
cell = "roko:rem-imagination-graph"
protocol = "Compose"

[[nodes]]
id = "her-relabeler"
cell = "roko:her-relabel-verify"
protocol = "Verify"

[[nodes]]
id = "dedup-verify"
cell = "roko:hdc-dedup-verify"
protocol = "Verify"

[[nodes]]
id = "staging-store"
cell = "roko:staging-buffer"
protocol = "Store"

[[nodes]]
id = "depotentiation"
cell = "roko:arousal-depotentiation"
protocol = "React"

[[edges]]
from = "episode-store"
to = "replay-scorer"

[[edges]]
from = "replay-scorer"
to = "budget-router"

[[edges]]
from = "budget-router"
to = "nrem-replay"
condition = "routed == 'nrem'"

[[edges]]
from = "budget-router"
to = "rem-imagination"
condition = "routed == 'rem'"

[[edges]]
from = "budget-router"
to = "her-relabeler"
condition = "routed == 'her'"

[[edges]]
from = "nrem-replay"
to = "dedup-verify"

[[edges]]
from = "rem-imagination"
to = "dedup-verify"

[[edges]]
from = "her-relabeler"
to = "dedup-verify"

[[edges]]
from = "dedup-verify"
to = "staging-store"

[[edges]]
from = "staging-store"
to = "depotentiation"

[[edges]]
from = "depotentiation"
to = "episode-store"
```

This is a single Loop. The feedback edge from `depotentiation` back to `episode-store` means each iteration of the dream cycle benefits from the updated arousal metadata and staging buffer state of the previous iteration. When the convergence condition fires (staging buffer delta drops below 0.01, or 5 iterations), the Loop halts and the Agent transitions out of the Dreaming state.

---

## 2. The Replay Score Cell (Mattar-Daw Utility)

Episode replay IS a Store+Score+Route problem. The Score Cell ranks episodes by how much the system would benefit from replaying them. This is the Mattar-Daw utility formula (Mattar & Daw 2018, Nature Neuroscience), decomposed into three independent terms that the Score protocol combines.

### The utility formula

```
Utility(episode) = Gain(episode) * Need(episode) * (1 / SpacingPenalty(episode))
```

**Gain** = `|expected_outcome - actual_outcome|`. High prediction error means high learning value. Gate failures and surprising successes both produce high gain. Normalized to [0.0, 1.0] across the current batch.

**Need** = `hdc_similarity(episode.fingerprint, recent_centroid)`. How relevant is this episode to the agent's current operating context? The recent centroid is the bundled HDC vector of the most recent 50 episodes. High similarity means the agent frequently encounters situations like this one, so learning from it has high expected reuse value. Sub-microsecond per comparison (Kanerva 2009).

**SpacingPenalty** = `1.0 + (replay_count * 0.5 / time_since_last_replay_hours)`. Implements the spacing effect (Cepeda et al. 2006, Psychological Bulletin). Recently replayed episodes are penalized. Never-replayed episodes have penalty 1.0 (no penalty).

### The Score Cell

```rust
/// Scores episodes by Mattar-Daw replay utility.
///
/// Protocol: Score.
/// Input: Vec<Signal> where each Signal wraps an Episode.
/// Output: Vec<Signal> annotated with Score.utility = Mattar-Daw value.
pub struct MattarDawScoreCell {
    pub gain_weight: f64,        // default: 1.0
    pub need_weight: f64,        // default: 1.0
    pub spacing_decay: f64,      // default: 0.5
    pub recent_window: usize,    // default: 50
}

#[async_trait]
impl ScoreProtocol for MattarDawScoreCell {
    async fn score(
        &self,
        signal: &Signal,
        ctx: &ScoreContext,
    ) -> Result<Score> {
        let episode: &Episode = signal.body.downcast_ref()?;

        // Gain: prediction error from gate verdicts
        let gate_count = episode.gate_verdicts.len().max(1) as f64;
        let fail_count = episode.gate_verdicts.iter()
            .filter(|v| !v.passed).count() as f64;
        let gain = (fail_count / gate_count
            + if episode.success { 0.0 } else { 0.5 })
            .clamp(0.0, 1.0);

        // Need: HDC similarity to recent centroid
        let need = match (&episode.hdc_fingerprint, &ctx.attention_focus) {
            (Some(fp), Some(centroid)) => fp.similarity(centroid) as f64,
            _ => 0.5, // no fingerprint available, assume moderate need
        };

        // SpacingPenalty: inverse time-weighted replay count
        let replay_count = ctx.store
            .query_replay_count(&episode.id).await
            .unwrap_or(0) as f64;
        let hours_since = ctx.store
            .hours_since_last_replay(&episode.id).await
            .unwrap_or(f64::MAX);
        let spacing_penalty = 1.0
            + (replay_count * self.spacing_decay / hours_since.max(0.01));
        let spacing_inv = 1.0 / spacing_penalty;

        // Final utility
        let utility = (gain * self.gain_weight)
            * (need * self.need_weight)
            * spacing_inv;

        Ok(Score {
            relevance: need as f32,
            quality: gain as f32,
            confidence: spacing_inv as f32,
            novelty: (1.0 - need) as f32,
            utility: utility as f32,
        })
    }
}
```

### Predict-publish-correct for the Score Cell

The Score Cell is a learner. On each dream cycle, it publishes a Pulse predicting which episodes will produce the highest-value insights. After the dream cycle completes, the staging buffer publishes which insights actually promoted. The CalibrationPolicy ([02-CELL](../../unified/02-CELL.md) S8) joins these Pulses and adjusts the Score Cell's weight parameters.

```
Pulse topic: "dream.replay.predicted_top_k"    // published before replay
Pulse topic: "dream.staging.promoted"           // published after consolidation
Pulse topic: "dream.replay.calibration"         // CalibrationPolicy output
```

This means the Score Cell's gain_weight and need_weight drift toward values that predict actual waking utility. The system learns which episodes are worth replaying, not from a static formula, but from lived experience of which replays led to insights that survived waking validation.

---

## 3. The Budget Route Cell

Replay and counterfactual generation share a single Route Cell that allocates the dream budget between NREM replay, REM imagination, and hindsight relabeling. This is the key unification: the prior design ran phases sequentially with fixed proportions. The unified design treats them as competing consumers of a finite budget, routed by EFE.

### Budget allocation as routing

The Route protocol ([02-CELL](../../unified/02-CELL.md) S2.4) selects among alternatives by expected free energy (EFE). For the dream budget, the three alternatives are:

| Alternative | What it does | Typical budget share |
|---|---|---|
| NREM replay | Consolidate high-surprise episodes | 50% |
| REM imagination | Generate counterfactual hypotheses | 30% |
| HER relabeling | Recover learning signal from failures | 20% |

But these proportions are not fixed. The Route Cell observes the agent's recent learning trajectory and adapts:

```rust
/// Routes dream budget across NREM, REM, and HER sub-graphs.
///
/// Protocol: Route.
/// Input: scored episodes from the Mattar-Daw Score Cell.
/// Output: episodes annotated with `routed` tag indicating sub-graph.
pub struct DreamBudgetRouteCell {
    /// Minimum budget fraction for any sub-graph.
    pub min_fraction: f64,           // default: 0.10
    /// EMA smoothing for budget adaptation.
    pub ema_alpha: f64,              // default: 0.3
    /// Running EMA of per-mode insight promotion rate.
    pub promotion_rates: BTreeMap<DreamMode, f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DreamMode {
    Nrem,
    Rem,
    Her,
}

impl DreamBudgetRouteCell {
    /// Compute budget fractions from historical promotion rates.
    ///
    /// Each mode's share is proportional to its promotion rate
    /// (insights promoted to permanent knowledge / insights generated),
    /// floored at `min_fraction`.
    fn compute_fractions(&self) -> BTreeMap<DreamMode, f64> {
        let total: f64 = self.promotion_rates.values().sum();
        if total < 1e-9 {
            // Cold start: use default proportions
            return BTreeMap::from([
                (DreamMode::Nrem, 0.50),
                (DreamMode::Rem, 0.30),
                (DreamMode::Her, 0.20),
            ]);
        }

        let mut fractions = BTreeMap::new();
        let mut headroom = 1.0;

        for (mode, rate) in &self.promotion_rates {
            let raw = rate / total;
            let floored = raw.max(self.min_fraction);
            fractions.insert(*mode, floored);
            headroom -= floored;
        }

        // Distribute any remaining headroom to the highest performer
        if headroom > 0.0 {
            if let Some(best) = fractions.iter()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .map(|(k, _)| *k)
            {
                *fractions.get_mut(&best).unwrap() += headroom;
            }
        }

        fractions
    }
}

#[async_trait]
impl RouteProtocol for DreamBudgetRouteCell {
    async fn route(
        &self,
        candidates: &[RouteCandidate],
        ctx: &RouteContext,
    ) -> Result<RouteDecision> {
        let fractions = self.compute_fractions();

        // Sort scored episodes by utility (highest first)
        let mut episodes = candidates.to_vec();
        episodes.sort_by(|a, b| b.score.utility.partial_cmp(&a.score.utility)
            .unwrap_or(std::cmp::Ordering::Equal));

        let total = episodes.len();
        let mut decisions = Vec::with_capacity(total);
        let mut cursor = 0;

        for (mode, fraction) in &fractions {
            let count = ((total as f64) * fraction).ceil() as usize;
            let slice_end = (cursor + count).min(total);
            for ep in &episodes[cursor..slice_end] {
                decisions.push(RouteDecision {
                    candidate: ep.clone(),
                    target: match mode {
                        DreamMode::Nrem => "nrem-replay",
                        DreamMode::Rem => "rem-imagination",
                        DreamMode::Her => "her-relabeler",
                    },
                });
            }
            cursor = slice_end;
        }

        Ok(RouteDecision::fan_out(decisions))
    }
}
```

### Learning which mode is most productive

The Route Cell observes the staging buffer's promotion rate per mode and adjusts the EMA:

```
After each dream cycle:
  for mode in [Nrem, Rem, Her]:
    generated = count(staging_buffer entries from this mode this cycle)
    promoted  = count(staging_buffer entries from this mode that reached Promoted)
    rate      = promoted / generated.max(1)
    ema[mode] = ema_alpha * rate + (1 - ema_alpha) * ema[mode]
```

If REM imagination produces many hypotheses but few survive waking validation, its share shrinks. If HER relabeling consistently produces insights that promote, its share grows. The minimum fraction (default 10%) prevents any mode from being starved entirely -- all three modes maintain exploratory coverage.

This is predict-publish-correct applied to the Route Cell itself. The prediction is the budget allocation. The outcome is the promotion rate. The correction is the EMA update.

---

## 4. NREM Replay as a Compose Sub-Graph

NREM replay IS a Compose Cell. It takes scored episodes as input and produces Insight Signals as output. Internally, it is a sub-Graph with its own Route Cell that selects replay mode.

### Four replay modes

| Mode | What | When selected | Reference |
|---|---|---|---|
| Standard Forward | Chronological replay with current knowledge | Default | -- |
| Reverse | Outcome-to-conditions causal chain | High-gain episodes | Ambrose et al. 2016, Science |
| Perturbed | Systematic value/timing/outcome perturbation | 30% random selection | -- |
| Compressed Batch | K-medoids clustering, replay medoids | Backlog > 50 episodes | -- |

### Replay fidelity

Not all episodes are replayed at the same fidelity. The fidelity level determines how closely the replay matches the original episode:

| Fidelity | Fraction | Description | Selection criterion |
|---|---|---|---|
| Exact | 20% | Verbatim replay of the original episode | Anchor memories: arousal > 0.8 |
| Perturbed | 65% | Gaussian noise on parameters, sigma = 0.15 | Default |
| Generative | 15% | Structural template only, floor similarity >= 0.80 | Low-confidence episodes |

Anchor memories -- episodes with arousal above 0.8 in the PAD metadata -- always bypass to Exact fidelity. These are the episodes where something consequential happened (a major gate failure, a breakthrough insight, a novel threat). Perturbing them risks losing the precise conditions that made them significant.

### The NREM sub-Graph

```rust
/// NREM replay as a Compose sub-Graph.
///
/// Contains:
///   - Mode Route Cell (selects forward/reverse/perturbed/compressed)
///   - Fidelity Route Cell (selects exact/perturbed/generative)
///   - Replay Compose Cell (constructs the replay prompt)
///   - Pattern Score Cell (trigram mining + HDC clustering)
///
/// Input: scored episodes from the Budget Route Cell.
/// Output: InsightRecord Signals for the dedup Verify Cell.
pub struct NremReplayGraph;

impl Cell for NremReplayGraph {
    fn name(&self) -> &str { "nrem-replay-graph" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Compose] }
}
```

```toml
[graph]
name = "nrem-replay"

[[nodes]]
id = "mode-router"
cell = "roko:replay-mode-router"
protocol = "Route"

[[nodes]]
id = "fidelity-router"
cell = "roko:replay-fidelity-router"
protocol = "Route"

[[nodes]]
id = "replay-compose"
cell = "roko:replay-prompt-compose"
protocol = "Compose"

[[nodes]]
id = "pattern-scorer"
cell = "roko:cross-episode-pattern-scorer"
protocol = "Score"

[[edges]]
from = "mode-router"
to = "fidelity-router"

[[edges]]
from = "fidelity-router"
to = "replay-compose"

[[edges]]
from = "replay-compose"
to = "pattern-scorer"
```

### Mode Route Cell

```rust
/// Selects replay mode for each episode.
///
/// Protocol: Route.
/// Decision tree:
///   1. If episode backlog > 50: Compressed Batch for the cluster,
///      skip to medoid episodes.
///   2. If episode.gain > 0.7: Reverse (high-gain episodes benefit
///      most from causal-chain analysis).
///   3. If random(0..1) < 0.30: Perturbed (30% perturbation rate).
///   4. Otherwise: Standard Forward.
pub struct ReplayModeRouteCell {
    pub compressed_threshold: usize,  // default: 50
    pub reverse_gain_floor: f64,      // default: 0.7
    pub perturbed_rate: f64,          // default: 0.30
}

#[async_trait]
impl RouteProtocol for ReplayModeRouteCell {
    async fn route(
        &self,
        candidates: &[RouteCandidate],
        ctx: &RouteContext,
    ) -> Result<RouteDecision> {
        if candidates.len() > self.compressed_threshold {
            return self.route_compressed(candidates, ctx).await;
        }

        let mut decisions = Vec::with_capacity(candidates.len());
        for ep in candidates {
            let mode = if ep.score.quality > self.reverse_gain_floor as f32 {
                ReplayMode::Reverse
            } else if ctx.rng.gen_f64() < self.perturbed_rate {
                ReplayMode::Perturbed
            } else {
                ReplayMode::Forward
            };
            decisions.push((ep.clone(), mode));
        }

        Ok(RouteDecision::annotated(decisions))
    }
}

impl ReplayModeRouteCell {
    /// Compressed batch: cluster episodes by HDC fingerprint,
    /// replay only medoids. K-medoids with k = sqrt(n).
    async fn route_compressed(
        &self,
        candidates: &[RouteCandidate],
        _ctx: &RouteContext,
    ) -> Result<RouteDecision> {
        let k = (candidates.len() as f64).sqrt().ceil() as usize;
        let fingerprints: Vec<&HdcVector> = candidates.iter()
            .filter_map(|c| c.signal.hdc_fingerprint.as_ref())
            .collect();

        let clusters = hdc_k_medoids(&fingerprints, k, 50);

        // Select medoid from each cluster
        let medoid_indices: Vec<usize> = clusters.iter()
            .map(|c| c.medoid_index)
            .collect();

        let decisions: Vec<_> = medoid_indices.iter()
            .filter_map(|&i| candidates.get(i))
            .map(|ep| (ep.clone(), ReplayMode::Forward))
            .collect();

        Ok(RouteDecision::annotated(decisions))
    }
}
```

### Perturbation as Compose

The Perturbed replay mode is itself a Compose Cell that takes an episode and produces a modified version with controlled mutations:

```rust
/// Perturbation types applied during Perturbed replay.
pub enum Perturbation {
    /// Numeric parameters shifted by +/-10-50%.
    ValueShift { magnitude: f64 },
    /// Temporal parameters shifted by +/-2x.
    TimingShift { factor: f64 },
    /// Success/failure outcome flipped.
    OutcomeFlip,
    /// Context from an unrelated episode injected.
    ContextInjection { donor_episode: SignalRef },
}

/// Compose Cell: apply perturbations to an episode for robustness testing.
///
/// Input: (episode Signal, perturbation type).
/// Output: perturbed episode Signal with lineage back to original.
pub struct PerturbationComposeCell {
    pub value_shift_range: (f64, f64),  // default: (0.10, 0.50)
    pub timing_shift_max: f64,          // default: 2.0
}
```

---

## 5. Spaced Repetition as Predict-Publish-Correct

Spaced repetition IS the predict-publish-correct pattern ([02-CELL](../../unified/02-CELL.md) S8). The system predicts when a piece of knowledge needs review. Reality publishes whether the knowledge is still valid. The calibration updates the review schedule.

### SM-2 as a Score Cell

The SM-2 algorithm (Wozniak & Gorzelanczyk 1994) is a Score Cell that predicts review intervals. Its parameters -- the easiness factor EF and the inter-repetition interval I(n) -- are the Cell's internal state, updated by predict-publish-correct.

```rust
/// SM-2 spaced repetition as a Score Cell.
///
/// For each knowledge Signal in the Store, maintains an easiness factor
/// and repetition count. Produces a Score where utility = urgency of review
/// (1.0 = overdue, 0.0 = not yet due).
///
/// Parameters are canonical SM-2 (Wozniak & Gorzelanczyk 1994):
///   EF_initial = 2.5
///   I(1) = 1 hour
///   I(2) = EF hours
///   I(n) = I(n-1) * EF
///   EF_min = 1.3
pub struct Sm2ScoreCell {
    pub initial_ef: f64,      // 2.5
    pub min_ef: f64,          // 1.3
    pub first_interval_h: f64, // 1.0
}

/// Per-Signal repetition state.
pub struct RepetitionState {
    pub signal_ref: SignalRef,
    pub easiness_factor: f64,
    pub repetition_count: u32,
    pub last_review_at: DateTime<Utc>,
    pub next_review_at: DateTime<Utc>,
}

impl RepetitionState {
    /// Compute the next interval in hours.
    fn next_interval_hours(&self, ef: f64, first_h: f64) -> f64 {
        match self.repetition_count {
            0 => first_h,
            1 => ef * first_h,
            _ => {
                let prev = self.interval_hours();
                prev * ef
            }
        }
    }

    /// Update after a review. grade: 0 (total failure) to 5 (perfect).
    /// SM-2 EF update: EF' = EF + (0.1 - (5 - grade) * (0.08 + (5 - grade) * 0.02))
    fn update(&mut self, grade: u8, now: DateTime<Utc>, first_h: f64, min_ef: f64) {
        let g = grade.min(5) as f64;
        let ef_delta = 0.1 - (5.0 - g) * (0.08 + (5.0 - g) * 0.02);
        self.easiness_factor = (self.easiness_factor + ef_delta).max(min_ef);

        if grade >= 3 {
            self.repetition_count += 1;
        } else {
            // Failed review: reset to first interval
            self.repetition_count = 0;
        }

        let interval_h = self.next_interval_hours(self.easiness_factor, first_h);
        self.last_review_at = now;
        self.next_review_at = now + chrono::Duration::hours(interval_h as i64);
    }
}
```

### The predict-publish-correct Loop

```
1. PREDICT: SM-2 Score Cell publishes a Pulse on "memory.review.due"
   containing the Signals whose review interval has elapsed.

2. REALITY: During the next dream cycle, those Signals are replayed.
   The replay produces a gate verdict (did the knowledge hold up?).

3. CORRECT: The CalibrationPolicy joins the predicted review schedule
   with the actual verdict:
     - grade >= 3 (knowledge held): EF increases, interval stretches.
     - grade < 3 (knowledge failed): EF decreases, interval shrinks.
     - grade == 0 (knowledge contradicted): EF drops toward min,
       Signal flagged for AntiKnowledge check.
```

```toml
[graph]
name = "spaced-repetition-loop"
loop = true

[[nodes]]
id = "sm2-scorer"
cell = "roko:sm2-score"
protocol = "Score"

[[nodes]]
id = "replay-compose"
cell = "roko:replay-prompt-compose"
protocol = "Compose"

[[nodes]]
id = "gate-verify"
cell = "roko:gate-pipeline"
protocol = "Verify"

[[nodes]]
id = "sm2-calibrate"
cell = "roko:sm2-calibration"
protocol = "React"

[[edges]]
from = "sm2-scorer"
to = "replay-compose"
condition = "score.utility > 0.5"

[[edges]]
from = "replay-compose"
to = "gate-verify"

[[edges]]
from = "gate-verify"
to = "sm2-calibrate"

[[edges]]
from = "sm2-calibrate"
to = "sm2-scorer"
```

This Loop runs at delta timescale (per-dream-cycle). The SM-2 scorer predicts which Signals need review. Replay tests them. The gate verdict feeds back to calibrate the easiness factor. Over time, robust knowledge gets reviewed less often; fragile knowledge gets reviewed more often. Knowledge that consistently fails review decays through demurrage and eventually enters cold storage.

The interaction between SM-2 and demurrage is deliberate. Demurrage is passive economic pressure (Gesell 1916): idle Signals decay. SM-2 is active scheduling: the system seeks out Signals that might be stale and tests them. The two mechanisms reinforce each other. A Signal that is both unused (high demurrage drain) and SM-2-overdue (low easiness factor) is doubly pressured toward retirement.

---

## 6. SCM Levels as Three Compose Cells

Pearl's Structural Causal Model levels (L1/L2/L3) are three Compose Cells of increasing sophistication, selected by Route based on available budget and confidence requirements. They are not sequential phases -- the Route Cell picks the appropriate level per-episode based on the episode's characteristics and the remaining dream budget.

### The three levels

```rust
/// Pearl SCM Level 1: Association.
///
/// "What correlates with what?" Statistical pattern matching
/// across episodes. No causation asserted.
///
/// Protocol: Compose.
/// Initial confidence: 0.20.
/// Budget cost: low (pattern matching, no LLM call).
pub struct AssociationComposeCell;

/// Pearl SCM Level 2: Intervention.
///
/// "What would happen if I did X?" Simulates do(X) interventions
/// on a causal model built from observed episodes.
///
/// Protocol: Compose.
/// Initial confidence: 0.25-0.30.
/// Budget cost: medium (requires causal model construction + LLM call).
pub struct InterventionComposeCell {
    /// The causal model built from observed episodes.
    /// Variables: model, task_type, tool_sequence, outcome.
    /// Edges: observed co-occurrence frequency > threshold.
    pub causal_model: CausalModel,
}

/// Pearl SCM Level 3: Counterfactual.
///
/// "What would have happened if conditions were different?"
/// Abduction-action-prediction (Pearl 2009).
///
/// Protocol: Compose.
/// Initial confidence: 0.30.
/// Budget cost: high (abduction + modified model + prediction LLM call).
pub struct CounterfactualComposeCell {
    pub causal_model: CausalModel,
}

/// Pearl SCM Level 3+: Backtracking Counterfactual.
///
/// "What must have been different upstream for the outcome to change?"
/// DeepBC (arXiv:2310.07665, TMLR 2024).
///
/// Protocol: Compose.
/// Initial confidence: 0.25 (lower than L3 due to exogenous reasoning).
/// Budget cost: highest (posterior sampling over exogenous variables).
pub struct BacktrackingComposeCell {
    pub max_backtrack_depth: usize,  // default: 3
    pub posterior_samples: usize,    // default: 5
    pub budget_fraction: f64,        // default: 0.30 of L3 budget
}
```

### SCM Route Cell

The Route Cell selects which SCM level to apply based on two signals: the episode's Score and the remaining budget.

```rust
/// Routes episodes to the appropriate SCM Compose Cell.
///
/// Protocol: Route.
/// Decision logic:
///   - All episodes get L1 (Association) -- it is free.
///   - Episodes with gain > 0.5: additionally get L2 (Intervention).
///   - Episodes with gain > 0.7 AND remaining budget >= L3 cost:
///     additionally get L3 (Counterfactual).
///   - Of the L3 budget, 30% is allocated to L3+ (Backtracking).
pub struct ScmLevelRouteCell {
    pub l2_gain_threshold: f64,      // default: 0.5
    pub l3_gain_threshold: f64,      // default: 0.7
    pub l3_backtrack_fraction: f64,  // default: 0.30
}

#[async_trait]
impl RouteProtocol for ScmLevelRouteCell {
    async fn route(
        &self,
        candidates: &[RouteCandidate],
        ctx: &RouteContext,
    ) -> Result<RouteDecision> {
        let mut decisions = Vec::new();
        let mut budget_remaining = ctx.budget.remaining();

        for ep in candidates {
            // L1 is always free
            decisions.push((ep.clone(), ScmLevel::Association));

            if ep.score.quality > self.l2_gain_threshold as f32 {
                let l2_cost = self.estimate_l2_cost(ep);
                if budget_remaining >= l2_cost {
                    decisions.push((ep.clone(), ScmLevel::Intervention));
                    budget_remaining -= l2_cost;
                }
            }

            if ep.score.quality > self.l3_gain_threshold as f32 {
                let l3_cost = self.estimate_l3_cost(ep);
                if budget_remaining >= l3_cost {
                    // Standard L3 gets (1 - backtrack_fraction)
                    decisions.push((ep.clone(), ScmLevel::Counterfactual));

                    // L3+ Backtracking gets backtrack_fraction
                    let bt_cost = l3_cost * self.l3_backtrack_fraction;
                    if budget_remaining >= l3_cost + bt_cost {
                        decisions.push((ep.clone(), ScmLevel::Backtracking));
                        budget_remaining -= bt_cost;
                    }

                    budget_remaining -= l3_cost;
                }
            }
        }

        Ok(RouteDecision::annotated(decisions))
    }
}
```

### Why Compose and not a monolithic function

Each SCM level is a separate Compose Cell because:

1. **Independent versioning**: L1 can be updated without touching L3.
2. **Independent budgeting**: Each Cell reports its own `estimated_cost()`, enabling the Route Cell to make informed decisions.
3. **Independent calibration**: Each level's predict-publish-correct Loop tracks its own promotion rate. If L2 Intervention hypotheses rarely promote, the Route Cell learns to allocate less budget to L2.
4. **Composability**: A custom dream Graph can include L1 and L2 but exclude L3, or include only L3 for a "deep counterfactual" mode.

---

## 7. Boden Creativity Modes as Three Compose Cells

Boden's (2004) three creativity modes -- combinational, exploratory, transformational -- are three additional Compose Cells that produce novel strategy hypotheses during REM imagination. The Route Cell selects which mode to apply based on the agent's current cognitive state.

### The three Compose Cells

```rust
/// Boden Combinational: merge patterns from two dissimilar episodes.
///
/// Protocol: Compose.
/// Selection: pairs of episodes with HDC dissimilarity > 0.55.
/// Max pairs per cycle: 5.
/// Output: bisociation hypotheses (Koestler 1964).
pub struct CombinationalComposeCell {
    pub min_dissimilarity: f64,  // default: 0.55
    pub max_pairs: usize,        // default: 5
}

/// Boden Exploratory: push a known heuristic to its boundary conditions.
///
/// Protocol: Compose.
/// Input: existing Heuristic Signal with calibration score.
/// Extreme multiplier: 3.0 (push parameter 3x beyond normal range).
/// Output: boundary condition hypotheses.
pub struct ExploratoryComposeCell {
    pub extreme_multiplier: f64,  // default: 3.0
}

/// Boden Transformational: violate a core assumption and rebuild.
///
/// Protocol: Compose.
/// Input: existing Heuristic Signal.
/// Max assumptions to violate per cycle: 5.
/// Min heuristic confidence for transformation: 0.40.
/// Output: radical strategy shift hypotheses.
pub struct TransformationalComposeCell {
    pub max_assumptions: usize,          // default: 5
    pub min_heuristic_confidence: f64,   // default: 0.40
}
```

### Creativity Route Cell

The Route Cell selects creativity mode based on the agent's CorticalState ([05-AGENT](../../unified/05-AGENT.md)):

```rust
/// Routes to creativity mode based on cognitive state.
///
/// Protocol: Route.
/// Decision logic:
///   - Low novelty in recent episodes (< 0.3 avg): Transformational
///     (the current approach is stale, violate assumptions).
///   - High novelty + high gain (> 0.6 both): Exploratory
///     (new territory, probe boundaries).
///   - Otherwise: Combinational
///     (the default generative mode).
pub struct CreativityModeRouteCell {
    pub transformational_novelty_ceiling: f64,  // default: 0.3
    pub exploratory_novelty_floor: f64,         // default: 0.6
    pub exploratory_gain_floor: f64,            // default: 0.6
}

#[async_trait]
impl RouteProtocol for CreativityModeRouteCell {
    async fn route(
        &self,
        candidates: &[RouteCandidate],
        ctx: &RouteContext,
    ) -> Result<RouteDecision> {
        let avg_novelty = candidates.iter()
            .map(|c| c.score.novelty as f64)
            .sum::<f64>() / candidates.len().max(1) as f64;

        let avg_gain = candidates.iter()
            .map(|c| c.score.quality as f64)
            .sum::<f64>() / candidates.len().max(1) as f64;

        let mode = if avg_novelty < self.transformational_novelty_ceiling {
            CreativityMode::Transformational
        } else if avg_novelty > self.exploratory_novelty_floor
            && avg_gain > self.exploratory_gain_floor
        {
            CreativityMode::Exploratory
        } else {
            CreativityMode::Combinational
        };

        let decisions: Vec<_> = candidates.iter()
            .map(|c| (c.clone(), mode))
            .collect();

        Ok(RouteDecision::annotated(decisions))
    }
}
```

### Observable via Lens

The creativity mode selection is observable through a Lens ([02-CELL](../../unified/02-CELL.md) S9). The Lens publishes which mode was selected on each dream cycle as a Pulse on topic `"dream.creativity.mode_selected"`. Over time, this Lens data reveals which mode produces the highest waking improvement -- measured by the staging buffer promotion rate partitioned by creativity mode.

```rust
/// Lens: creativity mode selection and outcome tracking.
pub struct CreativityModeLens {
    /// Counts by mode over trailing 30 dream cycles.
    pub selections: BTreeMap<CreativityMode, u32>,
    /// Promotion rate by mode (promoted / generated).
    pub promotion_rates: BTreeMap<CreativityMode, f64>,
    /// Waking improvement by mode: average gate score increase
    /// in tasks that used promoted insights from this mode.
    pub waking_improvement: BTreeMap<CreativityMode, f64>,
}
```

The waking_improvement metric closes the loop: the Route Cell can consult the Lens to verify that its creativity mode selection is actually producing downstream benefit. If Transformational mode is selected frequently but Transformational hypotheses never improve waking gate scores, the Route Cell can learn to prefer other modes.

---

## 8. Hindsight Experience Replay as a Verify Cell

Hindsight Experience Replay (HER; Andrychowicz et al. 2017, NIPS) IS a Verify Cell. It takes a failed trajectory (a Signal that did not pass its gate) and produces a reframed successful Signal with lower confidence. The Verify protocol ([02-CELL](../../unified/02-CELL.md) S2.3) already has the right shape: it examines input and output, produces a Verdict, and can emit new Signals as evidence.

### The relabeling operation

When a trajectory fails, HER asks: "What goal *was* achieved, even if the original goal was not?" The answer is the highest gate that the trajectory passed before the final failure.

```rust
/// Hindsight Experience Replay as a Verify Cell.
///
/// Protocol: Verify.
/// Input: failed episode Signals (gate failure).
/// Output: relabeled Signals with achieved sub-goals as the new goal,
///   at initial confidence 0.45.
///
/// For each failed episode, generates k=4 virtual goals (Andrychowicz et al. 2017):
///   1. The highest passing gate becomes the "achieved goal."
///   2. The trajectory up to that gate becomes a "successful" trajectory.
///   3. The relabeled Signal enters the staging buffer at confidence 0.45.
///   4. k-1 additional virtual goals are sampled from intermediate states.
pub struct HerVerifyCell {
    pub virtual_goals_per_episode: usize,  // default: 4
    pub initial_confidence: f64,           // default: 0.45
}

#[async_trait]
impl VerifyProtocol for HerVerifyCell {
    async fn verify_pre(
        &self,
        _input: &[Signal],
        _plan: &ActionPlan,
        _ctx: &VerifyContext,
    ) -> Result<Verdict> {
        // HER does not gate pre-execution; it only relabels post-hoc.
        Ok(Verdict::pass())
    }

    async fn verify_post(
        &self,
        input: &[Signal],
        _output: &[Signal],
        ctx: &VerifyContext,
    ) -> Result<Verdict> {
        let mut relabeled = Vec::new();

        for signal in input {
            let episode: &Episode = match signal.body.downcast_ref() {
                Some(ep) => ep,
                None => continue,
            };

            // Only process failed episodes
            if episode.success { continue; }

            // Find the highest passing gate (the "achieved goal")
            let passing_gates: Vec<&GateVerdict> = episode.gate_verdicts.iter()
                .filter(|v| v.passed)
                .collect();

            if passing_gates.is_empty() { continue; }

            // Primary virtual goal: highest passing gate
            let achieved = passing_gates.last().unwrap();
            relabeled.push(self.relabel_for_goal(
                signal,
                achieved,
                self.initial_confidence,
            ));

            // Additional virtual goals: sample from intermediate states
            let additional_count = (self.virtual_goals_per_episode - 1)
                .min(passing_gates.len().saturating_sub(1));
            for gate in passing_gates.iter().take(additional_count) {
                relabeled.push(self.relabel_for_goal(
                    signal,
                    gate,
                    self.initial_confidence * 0.85,
                ));
            }
        }

        Ok(Verdict {
            hard_pass: true,
            reward: 0.0,
            criteria: vec![],
            evidence: relabeled,
            explanation: format!(
                "HER relabeled {} failed episodes into {} virtual successes",
                input.iter().filter(|s| !s.body.downcast_ref::<Episode>()
                    .map_or(true, |ep| ep.success)).count(),
                relabeled.len(),
            ),
        })
    }
}

impl HerVerifyCell {
    /// Relabel a failed episode as a success for the given achieved gate.
    fn relabel_for_goal(
        &self,
        original: &Signal,
        achieved_gate: &GateVerdict,
        confidence: f64,
    ) -> Signal {
        let mut relabeled = original.clone();

        // Rewrite the goal to the achieved sub-goal
        relabeled.metadata.insert(
            "her_original_goal".into(),
            original.metadata.get("goal").cloned()
                .unwrap_or_default(),
        );
        relabeled.metadata.insert(
            "her_achieved_goal".into(),
            serde_json::Value::String(achieved_gate.gate_name.clone()),
        );
        relabeled.metadata.insert(
            "her_relabeled".into(),
            serde_json::Value::Bool(true),
        );

        // Set confidence to the HER initial level
        relabeled.score.confidence = confidence as f32;

        // Lineage: the relabeled Signal traces back to the original
        relabeled.lineage.push(original.id.clone());

        relabeled
    }
}
```

### Why Verify and not Compose

HER is a Verify Cell because it is fundamentally an evaluation operation: it examines a trajectory and judges which parts succeeded. The relabeled Signals are the Verdict's evidence, which is exactly how the Verify protocol ([02-CELL](../../unified/02-CELL.md) S2.3) models the relationship between evaluation and output. HER does not generate new content -- it reframes existing content under a different success criterion.

The initial confidence of 0.45 is deliberately below the staging buffer's Validated threshold (0.50). HER-relabeled episodes must still survive waking validation before they are trusted. This prevents the system from trusting synthetic successes that have never been tested in reality.

### Recovery rate

Andrychowicz et al. (2017) showed that HER recovers useful learning signal from at least 45% of otherwise-discarded failure episodes. In the Roko context, a trajectory that failed the test gate but passed the compile gate contains genuine evidence that the code was syntactically valid. That evidence is worth preserving, even if the overall goal was not achieved.

---

## 9. Emotional Depotentiation as a Functor

Emotional depotentiation IS a Functor applied during REM -- it modifies the PAD metadata on replayed Signals without changing the Graph topology. Functors are Signal endofunctors ([05-AGENT](../../unified/05-AGENT.md) S7): they transform Signals passing through an edge.

### The depotentiation operation

Walker & van der Helm (2009, Psychological Bulletin) demonstrated that REM sleep reduces the emotional intensity of memories. The informational content is preserved; the arousal charge is diminished.

```rust
/// Emotional depotentiation Functor.
///
/// Pattern: Functor (Signal endofunctor).
/// Applied to: Signals passing through the depotentiation node
///   in the dream Graph.
/// Effect: reduces the arousal dimension of the Signal's PAD vector.
///
/// delta ∈ [0.3, 0.5] per dream cycle (Walker & van der Helm 2009).
pub struct DepotentiationFunctor {
    pub delta_min: f64,  // default: 0.3
    pub delta_max: f64,  // default: 0.5
}

impl DepotentiationFunctor {
    /// Apply depotentiation to a Signal's PAD metadata.
    ///
    /// The delta is proportional to the current arousal:
    /// high-arousal memories get larger reductions.
    pub fn apply(&self, signal: &mut Signal) {
        if let Some(pad) = signal.metadata.get_mut("pad") {
            if let Some(arousal) = pad.get_mut("arousal") {
                let current = arousal.as_f64().unwrap_or(0.0);
                // Delta scales with current arousal: high arousal = larger reduction
                let delta = self.delta_min
                    + (self.delta_max - self.delta_min) * current.abs();
                let new_arousal = (current - delta).max(-1.0);
                *arousal = serde_json::Value::from(new_arousal);
            }
        }
    }
}
```

### Why a Functor and not a Cell

Depotentiation does not produce new Signals. It modifies existing Signals in transit. This is the defining characteristic of a Functor ([05-AGENT](../../unified/05-AGENT.md) S7): an operation applied to Signals flowing through an edge that does not change the Graph structure. Every Signal that passes through the depotentiation edge gets its arousal reduced.

The Functor is positioned after the staging buffer in the dream Graph. This means depotentiation applies to all outputs from the dream cycle -- both NREM insights and REM hypotheses -- before they flow back to the episode store via the feedback edge.

### Two purposes

1. **Reduces rumination**: high-arousal failure episodes (gate rejections, compilation errors) have their emotional charge reduced. The agent remembers the lesson but is not paralyzed by the memory of past failures.

2. **Preserves the lesson, removes the sting**: the informational content of the Signal (what happened, what was learned) is untouched. Only the PAD arousal dimension changes. This is the Complementary Learning Systems principle (McClelland et al. 1995): fast emotional response is decoupled from slow semantic knowledge.

---

## 10. Deduplication as a Verify Cell

Before insights and hypotheses enter the staging buffer, a Verify Cell performs HDC-based deduplication. This prevents the dream cycle from flooding the staging buffer with near-identical entries.

```rust
/// HDC deduplication Verify Cell.
///
/// Protocol: Verify.
/// Three-stage check:
///   1. Self-dedup: if new entry has HDC similarity > 0.85 to another
///      entry in the same batch, discard the lower-confidence duplicate.
///   2. Existing-knowledge check: if an existing Store entry with
///      confidence > 0.50 has HDC similarity > 0.80 to the new entry,
///      discard the new entry (unless it contradicts, in which case retain
///      at confidence 0.25).
///   3. AntiKnowledge check: if the new entry has HDC similarity > 0.50
///      to an AntiKnowledge Signal, apply the repulsion thresholds
///      from 06-MEMORY.md S7.
pub struct HdcDedupVerifyCell {
    pub self_dedup_threshold: f32,        // default: 0.85
    pub existing_threshold: f32,          // default: 0.80
    pub existing_min_confidence: f64,     // default: 0.50
    pub contradiction_confidence: f64,    // default: 0.25
}

#[async_trait]
impl VerifyProtocol for HdcDedupVerifyCell {
    async fn verify_post(
        &self,
        _input: &[Signal],
        output: &[Signal],
        ctx: &VerifyContext,
    ) -> Result<Verdict> {
        let mut accepted = Vec::new();
        let mut rejected_count = 0;

        // Stage 1: self-dedup within the batch
        let mut batch_fingerprints: Vec<(usize, &HdcVector)> = output.iter()
            .enumerate()
            .filter_map(|(i, s)| s.hdc_fingerprint.as_ref().map(|fp| (i, fp)))
            .collect();

        let mut discard_indices = BTreeSet::new();
        for i in 0..batch_fingerprints.len() {
            if discard_indices.contains(&i) { continue; }
            for j in (i+1)..batch_fingerprints.len() {
                if discard_indices.contains(&j) { continue; }
                let sim = batch_fingerprints[i].1
                    .similarity(batch_fingerprints[j].1);
                if sim > self.self_dedup_threshold {
                    // Discard the lower-confidence entry
                    let (idx_i, idx_j) = (
                        batch_fingerprints[i].0,
                        batch_fingerprints[j].0,
                    );
                    if output[idx_i].score.confidence
                        < output[idx_j].score.confidence
                    {
                        discard_indices.insert(idx_i);
                    } else {
                        discard_indices.insert(idx_j);
                    }
                    rejected_count += 1;
                }
            }
        }

        // Stage 2 + 3: check against existing store
        for (i, signal) in output.iter().enumerate() {
            if discard_indices.contains(&i) { continue; }

            if let Some(fp) = &signal.hdc_fingerprint {
                // Check existing knowledge
                let existing = ctx.store
                    .query_similar(fp, self.existing_threshold, 1).await?;

                if let Some((existing_ref, sim)) = existing.first() {
                    let existing_signal = ctx.store
                        .get(&existing_ref.id).await?;

                    if let Some(existing) = existing_signal {
                        if existing.score.confidence
                            > self.existing_min_confidence as f32
                            && *sim > self.existing_threshold
                        {
                            // Check if it contradicts
                            if signal.metadata.get("contradicts").is_some() {
                                let mut kept = signal.clone();
                                kept.score.confidence =
                                    self.contradiction_confidence as f32;
                                accepted.push(kept);
                            } else {
                                rejected_count += 1;
                            }
                            continue;
                        }
                    }
                }
            }

            accepted.push(signal.clone());
        }

        Ok(Verdict {
            hard_pass: true,
            reward: 0.0,
            criteria: vec![],
            evidence: accepted,
            explanation: format!(
                "Dedup: {} accepted, {} rejected as duplicates",
                output.len() - rejected_count,
                rejected_count,
            ),
        })
    }
}
```

---

## 11. Prioritized Experience Replay Integration

The Mattar-Daw utility formula in section 2 naturally implements Prioritized Experience Replay (Schaul et al. 2016, ICLR). The mapping:

| PER concept | Mattar-Daw component | Role |
|---|---|---|
| Priority p_i | Gain * Need | Episodes with high prediction error AND high policy relevance are replayed first |
| Importance Sampling weights | 1 / SpacingPenalty | Corrects for the bias introduced by non-uniform sampling |
| Annealing beta | SpacingPenalty time decay | As time since last replay increases, the spacing penalty shrinks toward 1.0 (uniform) |

The SpacingPenalty serves the same role as the IS weight correction in PER: it prevents the replay distribution from collapsing onto a small set of repeatedly-replayed episodes. Episodes that have been replayed recently are downweighted, which is equivalent to the IS correction that prevents high-priority episodes from dominating training.

This unification means the Score Cell does not need separate PER and Mattar-Daw implementations. The Mattar-Daw utility formula IS the prioritization function, and the spacing penalty IS the IS weight correction.

---

## 12. The Complete Dream Budget Model

The Budget Route Cell (section 3) allocates across NREM/REM/HER. Within each mode, sub-Route Cells allocate across specific operations. The total budget is a tree:

```
Dream budget (from Engine's BudgetTracker)
  |
  +-- NREM (50% default, adaptive)
  |     +-- Forward replay: 70% of NREM budget
  |     +-- Reverse replay: mode-routed, high-gain episodes
  |     +-- Perturbed: 30% random
  |     +-- Compressed: automatic when backlog > 50
  |
  +-- REM (30% default, adaptive)
  |     +-- SCM levels:
  |     |     +-- L1 Association: free (all episodes)
  |     |     +-- L2 Intervention: gain > 0.5
  |     |     +-- L3 Counterfactual: gain > 0.7
  |     |     +-- L3+ Backtracking: 30% of L3 budget
  |     +-- Boden modes:
  |           +-- Combinational / Exploratory / Transformational
  |           +-- Route by CorticalState (section 7)
  |
  +-- HER (20% default, adaptive)
        +-- k=4 virtual goals per failed episode
        +-- Confidence: 0.45 initial
```

Each level reports its cost to the parent Route Cell via `Cell::estimated_cost()`. The Route Cell uses these estimates for EFE-based allocation. The predict-publish-correct Loop on each Route Cell adjusts the budget split based on observed promotion rates.

---

## What This Enables

1. **Composable dream architectures**: Because each phase is a Cell in a Graph, the dream cycle can be customized per-agent. A code agent might use heavy NREM replay (high-value gate verdicts) and light REM (less need for creative recombination). A research agent might invert this. The Graph topology, not the code, controls the balance.

2. **Budget-aware creativity**: The Route Cell that allocates between NREM, REM, and HER learns which mode is most productive for this agent's workload. An agent whose failures are mostly near-misses (many passing gates before a final failure) will naturally allocate more budget to HER. An agent whose failures are fundamental (no gates pass) will allocate more to REM counterfactual reasoning.

3. **Observable dream quality**: Every Cell publishes its predictions and outcomes. The Lens on the creativity mode Route Cell (section 7) provides a direct measurement of which creative strategy is producing the most waking improvement. This is not introspection -- it is measurement.

4. **SM-2 composing with demurrage**: The spaced repetition Loop (section 5) and demurrage create dual pressure on knowledge quality. Active scheduling (SM-2 predicts when to review) combines with passive economics (demurrage taxes idle knowledge). The combination is stronger than either alone: SM-2 catches knowledge that is about to become stale; demurrage catches knowledge that SM-2 missed.

5. **Incremental adoption**: Each Cell can be implemented independently. The existing `roko-dreams` crate already has `replay.rs`, `imagination.rs`, `staging.rs`, and `threat.rs`. The path forward is to wrap each in a Cell adapter and wire them with the Graph topology defined here, rather than rewriting from scratch.

---

## Feedback Loops

Five predict-publish-correct Loops operate within the dream Graph:

| Loop | Prediction | Outcome | Correction target |
|---|---|---|---|
| **Replay Score calibration** | "These episodes will produce the most valuable insights" | Which insights actually promoted from staging | Mattar-Daw weight parameters (gain_weight, need_weight) |
| **Budget Route adaptation** | "This NREM/REM/HER split will maximize promotions" | Promotion rate by mode | Mode budget fractions (EMA) |
| **SM-2 scheduling** | "This Signal needs review at time T" | Did the Signal pass review? | Easiness factor and interval |
| **SCM level routing** | "This episode needs L2/L3 counterfactual depth" | Did the hypothesis promote? | Gain thresholds for L2/L3 routing |
| **Creativity mode selection** | "Transformational/Exploratory/Combinational is optimal now" | Waking improvement by mode (Lens) | Novelty/gain thresholds for mode selection |

All five Loops use the same CalibrationPolicy ([02-CELL](../../unified/02-CELL.md) S8) -- they differ only in what is predicted, what counts as outcome, and which parameters are adjusted.

The cross-loop interaction is the most important property. The Budget Route Loop (row 2) observes the outputs of the other four Loops: if SM-2 scheduling is catching most stale knowledge on its own, the Budget Route may reduce NREM's share and increase REM's. If creativity mode selection consistently favors Combinational, the Budget Route knows that the agent is in a cross-pollination phase and can allocate more REM budget. The Loops are not isolated -- they share the Bus.

---

## Open Questions

1. **Dream interruption semantics**: What happens when an Agent receives a high-priority waking task mid-dream? The current design treats the dream Graph as a standard Flow that can be paused and resumed via the Engine ([04-EXECUTION](../../unified/04-EXECUTION.md)). But mid-dream interruption means the staging buffer may contain partially-processed entries. Should the staging buffer flush partial entries (losing work) or retain them for the next dream cycle (risking staleness)?

2. **Multi-agent dream sharing**: When two Agents dream about overlapping domains, their staging buffers may contain complementary hypotheses. Should there be a cross-agent staging buffer merge during Integration? The pheromone mechanism ([06-MEMORY](../../unified/06-MEMORY.md) S11) provides the channel (Wisdom pheromone Pulses on the Bus), but the timing is unclear: should the merge happen during REM (when both agents are dreaming) or during waking (when each agent encounters the other's pheromones)?

3. **HER confidence calibration**: The initial confidence of 0.45 for HER-relabeled episodes is a constant. Should it be adaptive? An HER relabeling where the achieved gate is the test gate (close to the original goal) is arguably higher-confidence than one where the achieved gate is the compile gate (far from the original goal). A distance-weighted confidence could improve the staging buffer's signal-to-noise ratio.

4. **Depotentiation overshoot**: Walker & van der Helm's delta range [0.3, 0.5] per cycle was calibrated for biological sleep, where cycles are 90 minutes. Roko's dream cycles are ~5 minutes. Should the delta be proportionally reduced, or is the per-cycle framing the right unit regardless of wall-clock duration? Over-depotentiation could eliminate useful somatic markers; under-depotentiation leaves the agent anxious.

5. **Backtracking counterfactual grounding**: L3+ Backtracking reasons about unobserved exogenous variables. In biological cognition, this is grounded by embodied experience. In Roko, the "exogenous" variables are things like "what if the CI server had been configured differently?" -- counterfactuals about the environment that the agent cannot observe. How should the system bound the plausibility of exogenous hypotheses when it has no ground truth about the exogenous state?

6. **Creativity mode cycle detection**: If the Creativity Mode Route Cell oscillates (Combinational -> Transformational -> Combinational -> ...) without producing promotions, is the issue the mode selection or the underlying input quality? A damping mechanism (e.g., minimum dwell time per mode) could prevent thrashing, but might also delay adaptation. The right answer may be a hysteresis threshold on the Route Cell's novelty/gain parameters.
