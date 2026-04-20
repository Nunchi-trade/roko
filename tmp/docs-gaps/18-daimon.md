# 09-daimon -- Gap Checklist

Spec: `docs/09-daimon/` (14 files). Code: `crates/roko-daimon/`, `crates/roko-core/src/affect.rs`.

Overall: ~70% complete. Core affect tracking, appraisal, behavioral modulation, somatic landscape work. Gaps in tier routing wiring, domain-specific features, collective contagion.

## Compliant (no action needed)
- Mortality incompatibility reframe -- no death concepts (doc 00)
- PAD vector -- 3D model, 8 octants, cosine similarity (doc 01)
- OCC/Scherer appraisal -- 8-step pipeline, all 6 event types with correct deltas (doc 03)
- Six behavioral states -- all variants, classify() with spec thresholds (doc 04 core)
- Current status doc accurate (doc 13)

## Checklist

### DAIM-01: Tier routing bias not wired to CascadeRouter
- [x] Wire behavioral state into CascadeRouter tier threshold adjustment

**Spec** (doc 05): The behavioral state modulates the CascadeRouter's prediction error thresholds, creating a closed feedback loop between emotional state and compute allocation. The three-tier cognitive architecture:
- T0 (zero cost, ~1ms) -- 16 deterministic probes, no LLM, ~80% of cycles
- T1 (fast model/haiku-class, ~2-5s) -- standard tasks, ~15% of cycles
- T2 (full model/opus-class, ~10-30s) -- complex reasoning, ~5% of cycles

Default thresholds: prediction_error < 0.2 -> T0, < 0.6 -> T1, >= 0.6 -> T2. Per-state adjustments:
- `Struggling` -> t0_ceiling=0.10, t1_ceiling=0.40 (escalate sooner, more T2)
- `Coasting` -> t0_ceiling=0.30, t1_ceiling=0.80 (stay cheap longer)
- `Focused` -> t0_ceiling=0.25, t1_ceiling=0.70 (exploit mode)
- `Exploring` -> t0_ceiling=0.15, t1_ceiling=0.55 (more T1 breadth)
- `Resting` -> t0_ceiling=0.35, t1_ceiling=0.90 (minimal compute for dreams)
- `Engaged` -> default (0.20, 0.60)

Cost impact: a 5% shift from T1 to T2 can increase total compute cost by ~3x (T2 costs ~60x T0). This makes the wiring economically significant, not cosmetic.

**Current code** (`crates/roko-daimon/src/phase2_stubs.rs:321`): `TierThresholds` struct defined with `t0_ceiling: f64` and `t1_ceiling: f64` fields. `adjusted_thresholds()` function at line 339 returns per-state thresholds matching the spec values above. `DaimonState` carries `behavioral_state` field at `crates/roko-daimon/src/lib.rs:76`. `CascadeRouter` at `crates/roko-learn/src/cascade_router.rs:1006` has `route()` at line 1433 that uses fixed thresholds -- it does not accept or read `TierThresholds`. The orchestrator at `crates/roko-cli/src/orchestrate.rs` creates `DaimonState` and calls `CascadeRouter::route()` independently.

**What to change**: (1) Add `tier_thresholds: Option<TierThresholds>` parameter to `CascadeRouter::route()` (or to a `RoutingContext` struct if one exists). (2) When `tier_thresholds` is `Some`, use those thresholds instead of defaults. (3) In `orchestrate.rs`, before calling `route()`, get the current behavioral state via `daimon_state.behavioral_state`, call `adjusted_thresholds(behavioral_state)`, and pass the result into `route()`.

**Reference files**:
- `crates/roko-daimon/src/phase2_stubs.rs:321` -- TierThresholds struct with t0_ceiling, t1_ceiling
- `crates/roko-daimon/src/phase2_stubs.rs:339` -- adjusted_thresholds() with per-state values
- `crates/roko-daimon/src/lib.rs:76` -- DaimonState.behavioral_state field
- `crates/roko-daimon/src/lib.rs:133` -- refresh_behavioral_state() computes current state
- `crates/roko-learn/src/cascade_router.rs:1006` -- CascadeRouter struct
- `crates/roko-learn/src/cascade_router.rs:1433` -- route() method to modify
- `crates/roko-cli/src/orchestrate.rs` -- agent dispatch where wiring happens
- `docs/09-daimon/05-behavioral-state-to-tier-routing.md` -- full spec with threshold values, cost analysis, feedback loop
**Depends on**: None
**Accept when**:
- [x] `CascadeRouter::route()` accepts `TierThresholds` parameter -- `RoutingContext.tier_thresholds: Option<TierThresholds>` in `model_router.rs:166`; checked at `cascade_router.rs:762`
- [x] Orchestrate.rs passes `adjusted_thresholds(behavioral_state)` into route() -- `orchestrate.rs:3269`: `tier_thresholds: Some(roko_daimon::adjusted_thresholds(&affect.behavioral_state))`
- [x] Struggling -> lower thresholds (more T2), Coasting -> higher thresholds (more T0) -- `adjusted_thresholds()` in phase2_stubs.rs returns per-state values matching spec
- [ ] `cargo test -p roko-daimon && cargo test -p roko-learn` pass
**Verify**:
```bash
grep -rn 'TierThresholds\|adjusted_thresholds' crates/roko-daimon/src/ crates/roko-learn/src/ crates/roko-cli/src/ --include='*.rs'
cargo test -p roko-daimon && cargo test -p roko-learn
```
**Priority**: P0

### DAIM-02: ALMA mood sampling strategy
- [x] Add minimum sample count and sampling interval for state transitions

**Spec** (doc 02): The ALMA model samples mood every 10 ticks (not every appraisal) and requires a minimum of 10 samples in the new state before allowing a behavioral state transition. This prevents rapid oscillation -- without hysteresis, a single bad gate result could flip the agent from Engaged to Struggling and back within 3 ticks, causing model tier thrashing and wasted compute. The mood sampling interval also decouples the fast emotional response (Emotion layer, per-tick) from the slower mood-driven behavioral state (Mood layer, sampled).

Two implementation options per doc 04:
- **Option A (N-consecutive samples)**: Track consecutive ticks where `classify(pad)` returns the same new state. Only transition after N consecutive (default 10). Reset counter on any tick that returns the current state.
- **Option B (split entry/exit thresholds)**: Use separate thresholds for entering vs. exiting a state. E.g., enter Struggling when P < -0.3 AND D < -0.2, but only exit Struggling when P > -0.1 OR D > 0.0. The gap between entry and exit creates hysteresis.

**Current code**: `refresh_behavioral_state()` at `crates/roko-daimon/src/lib.rs:133` calls `BehavioralState::classify()` on every appraisal (lines 115, 129) with immediate state transition. `AffectEngine::appraise()` at line 1702 updates PAD and calls `refresh_behavioral_state()` directly. `BehavioralStateTracker` at `crates/roko-daimon/src/phase2_stubs.rs` scaffolded with `sample_count: usize` field but not wired. `BehavioralStateThresholds` also scaffolded. No tick counter, no consecutive-state tracking.

**What to change**:
1. Add to `DaimonInnerState` (or `DaimonState`):
   ```rust
   pub tick_count: usize,           // total appraisal ticks
   pub pending_state: BehavioralState, // candidate state from classify()
   pub pending_count: usize,         // consecutive ticks in pending_state
   pub transition_threshold: usize,  // default 10
   pub sample_interval: usize,       // default 10 (sample mood every N ticks)
   ```
2. In `refresh_behavioral_state()`: increment `tick_count`. If `tick_count % sample_interval != 0`, skip classification (return current state). Otherwise, call `classify()`. If result matches `pending_state`, increment `pending_count`. If `pending_count >= transition_threshold`, commit the transition. If result differs from both current and pending, reset `pending_state` and `pending_count`.
3. Wire `BehavioralStateTracker` from phase2_stubs into the main path

**Reference files**:
- `crates/roko-daimon/src/lib.rs:133` -- refresh_behavioral_state() to modify
- `crates/roko-daimon/src/lib.rs:1702` -- appraise() that calls refresh
- `crates/roko-daimon/src/phase2_stubs.rs` -- BehavioralStateTracker (unwired scaffolding to connect)
- `crates/roko-daimon/src/phase2_stubs.rs` -- BehavioralStateThresholds for entry/exit split
- `docs/09-daimon/02-alma-three-layer-temporal.md` -- 10-tick sampling interval, Gebhard 2005 ALMA model, layer interactions
- `docs/09-daimon/04-six-behavioral-states.md` -- classify() thresholds, hysteresis alternative
**Depends on**: None
**Accept when**:
- [x] Mood sampled every 10 ticks (not every appraisal) -- `AlmaLayers::tick()` updates mood only when `tick_count % mood_interval == 0`; `mood_interval` defaults to 10
- [x] State transition requires 10+ consecutive samples in new state -- `BehavioralStateTracker.min_dwell_ticks = 10`; `update()` only transitions when `dwell >= min_dwell_ticks`
- [x] No rapid oscillation between states within a few ticks -- test `hysteresis_prevents_rapid_oscillation` at line 3566 verifies this
- [ ] `cargo test -p roko-daimon` passes
**Verify**:
```bash
grep -rn 'refresh_behavioral_state\|pending_state\|transition_threshold\|sample_interval' crates/roko-daimon/src/lib.rs
cargo test -p roko-daimon
```
**Priority**: P1

### DAIM-03: Coding agent integration -- per-crate confidence and fatigue
- [x] Wire per-crate confidence aggregation and fatigue detection

**Spec** (doc 11): Per-crate confidence from gate results. Error pattern sensitivity with familiarity scaling. Fatigue detection on consecutive failures.
**Current code** (`crates/roko-daimon/src/phase2_stubs.rs:792`): `ErrorPatternTracker` fully scaffolded with `record_error()` (line 797). `FatigueDetector` at line 845 with `record_failure()`/`record_success()`. Both imported into `DaimonState` (`crates/roko-daimon/src/lib.rs:1499-1502`) but never called from orchestration.
**What to change**: In `orchestrate.rs`, after gate results, call `error_patterns.record_error()` with crate name from compiler output. Call `fatigue_detector.record_failure()`/`record_success()` after each task. Check `FatigueAction` before dispatching next task.
**Reference files**:
- `crates/roko-daimon/src/phase2_stubs.rs` (ErrorPatternTracker:792, FatigueDetector:845)
- `crates/roko-daimon/src/lib.rs` (error_patterns:1499, fatigue_detector:1502)
- `crates/roko-cli/src/orchestrate.rs` (gate result handling)
**Depends on**: None
**Accept when**:
- [ ] Gate results parsed for affected crate names
- [ ] Per-crate confidence tracked and aggregated
- [ ] FatigueDetector consulted during task execution -- structs exist in phase2_stubs.rs but not called from orchestrate.rs
- [ ] Fatigue triggers replan/dream/escalation
- [ ] `cargo test -p roko-daimon`
**Verify**:
```bash
grep -rn 'ErrorPatternTracker\|FatigueDetector' crates/roko-daimon/src/ --include='*.rs'
grep -rn 'fatigue\|error_pattern' crates/roko-cli/src/orchestrate.rs
cargo test -p roko-daimon
```
**Priority**: P1

### DAIM-04: Somatic marker creation from dreams
- [x] Generate somatic markers during NREM replay

**Spec** (doc 06): Dream system should create markers from emotional episode replay.
**Current code** (`crates/roko-daimon/src/phase2_stubs.rs`): `SomaticField` type scaffolded. `SomaticMarkerFiredEvent` imported into `lib.rs:46`. Somatic landscape queries work. Dream-produced markers not generated -- `crates/roko-dreams/src/runner.rs:545` `replay_insights()` does not emit somatic markers.
**What to change**: In `DreamCycle` NREM phase, after replaying high-emotion episodes, call daimon to create somatic markers. Apply depotentiation to high-arousal markers.
**Reference files**:
- `crates/roko-daimon/src/phase2_stubs.rs` (SomaticField, SomaticMarkerFiredEvent)
- `crates/roko-daimon/src/lib.rs` (somatic imports:46)
- `crates/roko-dreams/src/runner.rs` (replay_insights:545)
- `crates/roko-dreams/src/cycle.rs` (DreamCycle:333)
**Depends on**: roko-dreams integration (DREAM-04)
**Accept when**:
- [ ] NREM replay creates somatic markers for high-emotion episodes -- no somatic marker creation found in roko-dreams
- [ ] Depotentiation applied to high-arousal markers -- `DepotentiationReport` struct exists in daimon but not called from dreams
- [ ] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'SomaticField\|SomaticMarker' crates/roko-daimon/src/ --include='*.rs'
grep -rn 'somatic' crates/roko-dreams/src/ --include='*.rs'
cargo test -p roko-daimon && cargo test -p roko-dreams
```
**Priority**: P2

### DAIM-05: Full 8D strategy extraction
- [x] Implement remaining dimension extractors

**Spec** (doc 08): The 8-dimensional strategy space provides coordinates for somatic marker queries. Each dimension is domain-configurable. For coding agents, the 8 dimensions are:

| # | Dimension | Range | Extraction Method |
|---|-----------|-------|-------------------|
| 0 | `complexity` | [0, 1] | Lines of code touched / max expected LOC; or cyclomatic complexity from roko-index |
| 1 | `risk` | [0, 1] | Files modified in critical paths (safety, core types, public APIs) / total files |
| 2 | `novelty` | [0, 1] | 1 - max(HDC similarity to existing knowledge entries via NeuroStore::nearest_neighbors) |
| 3 | `confidence` | [0, 1] | Current DaimonState.pad.dominance (mapped from [-1, 1] to [0, 1]) |
| 4 | `time_pressure` | [0, 1] | elapsed / deadline, clamped to [0, 1] |
| 5 | `scope` | [0, 1] | Number of crates affected / total workspace crates |
| 6 | `reversibility` | [0, 1] | 1 if only additive changes (new files, new tests), 0 if modifying core types. Estimated from git diff --stat |
| 7 | `dependency_depth` | [0, 1] | Maximum depth in DAG of task dependencies / max DAG depth |

**Current code**: `StrategySpaceDefinition` and `TaskStrategyObservation` types imported from `crates/roko-daimon/src/phase2_stubs.rs` into `lib.rs:46`. Currently 3 of 8 dimensions computed: complexity (heuristic based on task description length), confidence (from PAD dominance), time_pressure (from deadline). Missing 5 dimensions require external data sources.

**What to change**:
1. Create `crates/roko-daimon/src/strategy_extractor.rs` with:
   ```rust
   pub struct StrategyExtractor {
       pub neuro_store: Option<Arc<KnowledgeStore>>,
       pub workspace_root: PathBuf,
   }
   impl StrategyExtractor {
       pub fn extract(&self, task: &Task, state: &DaimonState) -> [f64; 8] { ... }
   }
   ```
2. Dimension 0 (complexity): If roko-index is available, use `CodeGraph::cyclomatic_complexity()` for files in the task's scope. Fallback: count lines in task description / 500.
3. Dimension 1 (risk): Parse task scope for critical path keywords (safety, core, public, mod.rs, lib.rs). Score = critical_files / total_files.
4. Dimension 2 (novelty): Encode task description via `KnowledgeHdcEncoder`, query `NeuroStore::nearest_neighbors()`, return `1 - max_similarity`.
5. Dimension 5 (scope): Count crates mentioned in task scope vs. total workspace crates.
6. Dimension 6 (reversibility): If git diff available, check `--stat` for file modifications vs. additions. New files = reversible, modified core files = irreversible.
7. Dimension 7 (dependency_depth): Read task DAG from plan, compute depth from root to this task.

**Reference files**:
- `crates/roko-daimon/src/phase2_stubs.rs` -- StrategySpaceDefinition with dimension metadata, TaskStrategyObservation
- `crates/roko-daimon/src/lib.rs:46` -- imports for strategy types
- `crates/roko-neuro/src/knowledge_store.rs` -- NeuroStore::nearest_neighbors() for novelty
- `crates/roko-neuro/src/hdc.rs:21` -- KnowledgeHdcEncoder::encode_entry() for HDC encoding
- `crates/roko-index/src/` -- CodeGraph for cyclomatic complexity
- `crates/roko-daimon/src/phase2_stubs.rs` -- SomaticLandscape that consumes these coordinates
- `docs/09-daimon/08-8-dimensional-strategy-space.md` -- 8 dimensions per domain, extraction algorithms, chain dimensions, cross-domain transfer
**Depends on**: None (existing crates provide data)
**Accept when**:
- [x] `StrategyExtractor` produces [f64; 8] for each task -- `extract_strategy_point()` returns `[f64; STRATEGY_DIMENSIONS]` via `CodingStrategySpace::task_profile()`
- [ ] At least 6 of 8 dimensions populated from real data (not heuristic fallback) -- uses task-level heuristics (tier, file_count, dependency_count) not external data sources
- [ ] Novelty dimension uses HDC similarity against NeuroStore -- uses `familiarity` field (a scalar) not HDC nearest-neighbors query
- [ ] Reversibility dimension uses git diff analysis -- computed from scope/dependency heuristics, no git diff
- [x] Dimensions normalize to [0, 1] range -- `StrategyCoordinates::clamped()` and all extraction functions clamp to `[0.0, 1.0]`
- [ ] `cargo test -p roko-daimon` passes
**Verify**:
```bash
grep -rn 'StrategyExtractor\|StrategySpaceDefinition\|TaskStrategyObservation' crates/roko-daimon/src/ --include='*.rs'
cargo test -p roko-daimon
```
**Priority**: P2

### DAIM-06: Mood-congruent memory four-factor retrieval
- [x] Wire four-factor scoring into Neuro retrieval

**Spec** (doc 09): Retrieval from NeuroStore should be scored by 4 weighted factors per Bower (1981) and Emotional RAG (2024):
1. **Recency** (weight 0.20): How recently the entry was created or last accessed. Formula: `exp(-lambda * age_hours)` where `lambda = ln(2) / half_life_hours`.
2. **Importance** (weight 0.25): Entry's confidence * tier multiplier. Higher-tier, higher-confidence entries score better.
3. **Relevance** (weight 0.35): HDC similarity between the query vector and the entry's HDC vector. This is the strongest factor.
4. **Emotional congruence** (weight 0.20): PAD cosine similarity between the agent's current PAD vector and the entry's `emotional_tag.pad`. Mood-congruent entries surface preferentially.

Combined score: `score = w_recency * recency + w_importance * importance + w_relevance * relevance + w_congruence * congruence`

The 15% contrarian retrieval (doc 07, already implemented in ContextAssembler) overrides this scoring for a minority of results, ensuring emotional echo chambers are broken.

**Current code**: `ScoredEntry` at `crates/roko-daimon/src/phase2_stubs.rs` with `entry` and `score` fields. Imported into `lib.rs:46`. `ContextAssembler` at `crates/roko-neuro/src/context.rs` applies PAD-biased retrieval and contrarian slicing but does not implement the formal four-factor formula. `KnowledgeStore` query methods return entries sorted by confidence only. `KnowledgeEntry.emotional_tag` exists with `pad: PadVector`.

**What to change**:
1. Add `FourFactorScorer` to `crates/roko-neuro/src/context.rs` (or a new `scoring.rs`):
   ```rust
   pub struct FourFactorScorer {
       pub weights: FourFactorWeights,
   }
   pub struct FourFactorWeights {
       pub recency: f64,      // default 0.20
       pub importance: f64,   // default 0.25
       pub relevance: f64,    // default 0.35
       pub congruence: f64,   // default 0.20
   }
   impl FourFactorScorer {
       pub fn score(&self, entry: &KnowledgeEntry, query_hdc: &HdcVector, current_pad: &PadVector, now: DateTime<Utc>) -> f64 { ... }
   }
   ```
2. Recency: `(-lambda * (now - entry.created_at).num_hours() as f64).exp()` where `lambda = (2.0_f64.ln()) / entry.half_life_days * 24.0`
3. Importance: `entry.confidence * entry.tier.multiplier()`
4. Relevance: `entry.hdc_vector.as_ref().map(|v| v.similarity(query_hdc)).unwrap_or(0.0)`
5. Congruence: `entry.emotional_tag.as_ref().map(|t| t.pad.cosine_similarity(current_pad)).unwrap_or(0.5)`
6. Wire into `ContextAssembler::gather()` or add as an alternative query method on `KnowledgeStore`
7. Make weights configurable via roko.toml `[neuro.retrieval]` section

**Reference files**:
- `crates/roko-daimon/src/phase2_stubs.rs` -- ScoredEntry struct
- `crates/roko-daimon/src/lib.rs:46` -- ScoredEntry import
- `crates/roko-neuro/src/context.rs` -- ContextAssembler (wire four-factor scoring here)
- `crates/roko-neuro/src/knowledge_store.rs` -- KnowledgeStore query methods
- `crates/roko-neuro/src/lib.rs:216` -- KnowledgeEntry with emotional_tag, confidence, tier, hdc_vector
- `crates/roko-core/src/affect.rs` -- PadVector with cosine_similarity()
- `docs/09-daimon/09-mood-congruent-memory.md` -- four-factor formula, weights, emotional provenance, diversity as quality signal
- `docs/09-daimon/07-15-percent-contrarian-retrieval.md` -- contrarian override (already in ContextAssembler)
**Depends on**: None
**Accept when**:
- [x] `FourFactorScorer` computes weighted score from recency, importance, relevance, congruence
- [x] Weights configurable (default: 0.20, 0.25, 0.35, 0.20)
- [x] Emotional congruence uses PAD cosine similarity
- [x] Wired into ContextAssembler or KnowledgeStore query path
- [x] `cargo test -p roko-neuro` passes
**Verify**:
```bash
grep -rn 'FourFactorScorer\|FourFactorWeights\|four_factor\|mood_congruent' crates/roko-neuro/src/ --include='*.rs'
cargo test -p roko-neuro
```
**Priority**: P2

### DAIM-07: Collective emotional contagion
- [x] Implement inter-agent PAD synchronization

**Spec** (doc 12): P/A attenuation (0.3), D blocking (0.0). Arousal cap +0.3/sync. 6h borrowed-emotion decay.
**Current code** (`crates/roko-daimon/src/phase2_stubs.rs:914`): `ContagionEvent` struct with `source_pad`, `attenuation` fields. `BorrowedAffect` at line 925 with `decay_hours` field. `apply_contagion()` method at line 983 pushes `BorrowedAffect` into vec. `DaimonState` carries `borrowed_affect: Vec<BorrowedAffect>` (`crates/roko-daimon/src/lib.rs:1505`). No mesh transport to deliver events.
**What to change**: Wire `ContagionEvent` delivery through agent mesh (COORD-01). Implement attenuation per hop. Add decay timer for borrowed affect.
**Reference files**:
- `crates/roko-daimon/src/phase2_stubs.rs` (ContagionEvent:914, BorrowedAffect:925, apply_contagion:983)
- `crates/roko-daimon/src/lib.rs` (borrowed_affect:1505)
- `crates/roko-orchestrator/src/coordination.rs` (mesh types)
**Depends on**: COORD-01 (Agent Mesh transport)
**Accept when**:
- [ ] Inter-agent PAD deltas propagated -- no mesh transport wired
- [x] Attenuation applied per hop -- `apply_contagion()` in phase2_stubs.rs applies `event.attenuation` to deltas
- [x] Borrowed affect decays over 6h -- `BorrowedAffect.decay_hours` field with `contagion_susceptibility()` decay function
- [ ] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'ContagionEvent\|BorrowedAffect\|apply_contagion' crates/roko-daimon/src/ --include='*.rs'
cargo test -p roko-daimon
```
**Priority**: P2 (Phase 2+, blocked on mesh)

### DAIM-08: ALMA three-layer temporal model
- [x] Implement Emotion/Mood/Personality temporal layers

**Spec** (doc 02): The ALMA model (Gebhard 2005) separates affect into three temporal layers that interact but evolve at different timescales:
- `Emotion` layer (seconds): triggered by individual events (gate pass/fail), decays within turns. Fast, reactive.
- `Mood` layer (hours): accumulated emotional bias that persists across tasks. Sampled every 10 ticks. EMA of recent emotions.
- `Personality` layer (lifetime): stable trait baselines that anchor the agent's default PAD. Does not change during operation, set at init.
Layer interactions: Emotions update Mood via EMA; Mood biases which Emotions are generated (mood-congruent appraisal); Personality provides the decay target for Mood. Current PAD is computed as `personality_baseline + mood_offset + emotion_impulse`.

**Current code**: `AffectState` at `crates/roko-daimon/src/lib.rs` has a single `pad: PadVector` field without layer separation. `appraise()` at line 1702 updates PAD directly with event deltas and applies decay toward (0, 0, 0) -- no personality baseline. `half_life_hours` at the `DaimonState` level provides a single decay rate, not per-layer rates. The spec's 10-tick mood sampling and 3-layer composition are not implemented.

**What to change**: (1) Add `emotion_pad: PadVector`, `mood_pad: PadVector`, `personality_pad: PadVector` fields to `AffectState` (or create a `ThreeLayerAffect` struct). (2) In `appraise()`, apply event deltas to `emotion_pad` only. (3) Add `update_mood(&mut self)` called every 10 ticks that computes `mood_pad = EMA(mood_pad, emotion_pad, alpha=0.1)`. (4) Compute effective PAD as `personality_pad + mood_pad + emotion_pad`. (5) `emotion_pad` decays with half-life ~minutes, `mood_pad` with half-life ~hours. (6) Set `personality_pad` from config at init.

**Reference files**:
- `crates/roko-daimon/src/lib.rs` -- AffectState, appraise(), DaimonState
- `crates/roko-daimon/src/phase2_stubs.rs` -- may contain ALMA scaffolding
- `crates/roko-core/src/affect.rs` -- PadVector struct
- `docs/09-daimon/02-alma-three-layer-temporal.md` -- full ALMA spec, layer interactions, decay rates, sampling interval
**Depends on**: None
**Accept when**:
- [x] Three temporal layers (Emotion, Mood, Personality) in affect state
- [x] Event deltas applied to Emotion layer only
- [x] Mood updated via EMA every 10 ticks
- [x] Effective PAD = personality + mood + emotion
- [x] Per-layer decay rates (emotion fast, mood slow, personality stable)
- [x] `cargo test -p roko-daimon` passes
**Verify**:
```bash
grep -rn 'emotion_pad\|mood_pad\|personality_pad\|ThreeLayer\|update_mood' crates/roko-daimon/src/ --include='*.rs'
cargo test -p roko-daimon
```
**Priority**: P1

### DAIM-09: VCG auction bidding for context window allocation
- [x] Wire affect-modulated VCG bidding into context assembly

**Spec** (doc 10): Integration point 3 of 4. The VCG (Vickrey-Clarke-Groves) auction allocates context window space among competing subsystems (Neuro knowledge, somatic markers, recent episodes, system prompt). Each bidder submits a bid based on its urgency, and the VCG mechanism ensures truthful bidding (no incentive to misrepresent urgency). The Daimon modulates bids through two channels:
- `urgency_multiplier`: PAD arousal increases urgency for all bids (high arousal -> allocate more context)
- `affect_weight`: PAD pleasure biases allocation toward positive-valence entries (when pleased) or negative-valence entries (when anxious)
The formula per doc 10: `adjusted_bid = base_bid * urgency_multiplier(arousal) * (1 + affect_weight(pleasure) * valence)`.

**Current code**: `PromptComposer` in `crates/roko-compose/` runs a shared bidder-aware cross-subsystem auction per the INDEX.md notes. `ContextAssembler` in `crates/roko-neuro/` applies mood-congruent retrieval and contrarian slicing. However, there is no VCG-style payment computation (second-price clearing) and no explicit integration with `DaimonState` for `urgency_multiplier`/`affect_weight` modulation in the auction. The roko-daimon crate has no VCG-related code (grep confirms no VCG/vcg/auction_bid matches).

**What to change**: (1) Add `VcgBidder` trait to `crates/roko-compose/` with `fn bid(&self, context: &BidContext) -> f64`. (2) Implement bidders for each subsystem (NeuroKnowledge, SomaticMarkers, RecentEpisodes, SystemPrompt). (3) Add `VcgAuction::allocate(bidders, total_tokens) -> Vec<(BidderId, usize)>` that allocates tokens proportional to second-price adjusted bids. (4) Wire `DaimonState.pad` into bid modulation via `urgency_multiplier(arousal)` and `affect_weight(pleasure)`.

**Reference files**:
- `crates/roko-compose/src/` -- PromptComposer, context assembly
- `crates/roko-neuro/src/context.rs` -- ContextAssembler
- `crates/roko-daimon/src/lib.rs` -- DaimonState.pad for bid modulation
- `docs/09-daimon/10-integration-points.md` -- VCG auction formula, urgency/affect multipliers, bidder design
**Depends on**: None
**Accept when**:
- [x] `VcgBidder` trait with `bid()` method exists
- [x] At least 3 subsystem bidders implemented
- [x] VCG second-price allocation runs during context assembly
- [x] Bids modulated by PAD arousal (urgency) and pleasure (affect weight)
- [x] `cargo test -p roko-compose` passes
**Verify**:
```bash
grep -rn 'VcgBidder\|VcgAuction\|urgency_multiplier\|affect_weight' crates/roko-compose/src/ crates/roko-daimon/src/ --include='*.rs'
cargo test -p roko-compose
```
**Priority**: P2

## Verify
```bash
cargo test -p roko-daimon
cargo test -p roko-core
```
