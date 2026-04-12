# Current Status and Gaps

> What's built, what's scaffolded, what's specified but unimplemented, and which legacy files were deliberately skipped.

**Topic**: [Daimon](./INDEX.md)
**Prerequisites**: All previous sub-docs
**Key sources**: `roko-daimon/src/lib.rs`, `roko-golem/src/daimon.rs`, `refactoring-prd/07-implementation-priorities.md`

---

## Abstract

The Daimon affect engine exists in two parallel implementations (`roko-daimon` and `roko-golem/daimon.rs`) with significant overlap and some divergence. The core PAD vector, appraisal pipeline, and behavioral modulation are functional. The somatic landscape, collective contagion, VCG bidding integration, and several appraisal triggers are specified but not built. This document catalogs the exact state of each component, references the implementation priority tiers, and lists the legacy source files that were deliberately skipped during migration.

---

## Implemented Components

### roko-daimon (standalone crate)

**File**: `crates/roko-daimon/src/lib.rs` (569 lines)

| Component | Status | Description |
|---|---|---|
| `PadVector` struct | **Complete** | Three f64 fields (pleasure, arousal, dominance), clamped to [-1, 1] |
| `AffectState` struct | **Complete** | PAD + confidence + updated_at timestamp |
| `AffectEvent` enum | **Complete** | 6 variants: GateResult, TaskOutcome, Blocked, TimePressure, QueueWait, DreamFailure |
| `DaimonState` struct | **Complete** | Wraps AffectState + half_life_hours + persistence_path |
| `AffectEngine` trait | **Complete** | 4 methods: appraise(), query(), modulate(), persist() |
| Appraisal rules | **Complete** | All 6 event types with correct PAD deltas, rung scaling, asymmetric valence |
| Temporal decay | **Complete** | Exponential decay: `factor = 0.5 ^ (elapsed_hours / half_life_hours)` |
| Behavioral modulation | **Complete** | Model promotion/demotion (haiku↔sonnet↔opus), turn limit adjustment, strategy selection |
| Persistence | **Complete** | Atomic file write (write to .tmp, rename) with auto-save on appraise |
| Load/restore | **Complete** | `load_or_new()` loads from disk or creates fresh neutral state |
| `DispatchStrategy` enum | **Complete** | 5 variants with effort labels: Conservative, Balanced, Exploratory, Escalating, Proactive |
| `DispatchParams` struct | **Complete** | model + turn_limit + strategy + effort |
| `queue_wait_arousal()` | **Complete** | Public function for queue-wait arousal computation |
| Tests | **Complete** | 3 tests: appraise/persist, escalation, demotion |

### roko-golem/daimon.rs (per-task affect engine)

**File**: `crates/roko-golem/src/daimon.rs` (972 lines)

| Component | Status | Description |
|---|---|---|
| `AffectState` struct | **Complete** | 4 f64 fields (pleasure, arousal, dominance, confidence) + timestamp |
| `AffectEngine` struct | **Complete** | Per-task HashMap<String, AffectState> + half_life + persistence |
| `AffectOctant` enum | **Complete** | 8 octants with `from_pad()` classification and `label()` |
| `AffectBehaviorModulation` struct | **Complete** | 7 behavioral parameters (strategy, exploration_rate, prefer_proven_playbooks, model_tier_escalation, extra_retries, trigger_dream_cycles, run_maintenance_tasks) |
| 5 behavior factory methods | **Complete** | balanced(), anxious(), confident(), angry(), bored() |
| Per-task appraisal methods | **Complete** | on_task_success(), on_task_failure(), on_dream_failure(), on_gate_pass(), on_gate_fail(), on_time_pressure(), on_blocked(), queue_wait_arousal() |
| Signal emission | **Complete** | Emits affect signals to JSONL when confidence drops below 0.3 or valence crosses extremes |
| Atomic persistence | **Complete** | save_to() with tmp+rename pattern |
| Tests | **Complete** | 13 tests covering all appraisal methods, octant classification, behavior modulation, decay, persistence |

### Overlap Between the Two Implementations

| Feature | roko-daimon | roko-golem/daimon.rs | Notes |
|---|---|---|---|
| PAD vector | `PadVector` (3 fields) | 4 inline f64 fields | Same math, different struct |
| Appraisal rules | Via `AffectEvent` enum match | Via named methods (on_gate_pass, etc.) | Same deltas, different API surface |
| Behavioral modulation | `modulate()` on DispatchParams | `behavior_modulation()` on AffectOctant | Different output types |
| Octant classification | Not present | `AffectOctant::from_pad()` | roko-golem only |
| Per-task tracking | Not present (single global state) | `HashMap<String, AffectState>` | roko-golem only |
| Signal emission | Not present | Emits to JSONL | roko-golem only |
| Persistence | Atomic file write | Atomic file write | Same pattern |
| Half-life | 4.0 hours | 4.0 hours | Same value |

### Consolidation Plan (Tier 0C)

The plan (from `refactoring-prd/07-implementation-priorities.md` Tier 0C) is to dissolve `roko-golem` and move affect logic into `roko-daimon`. The consolidated crate should:

1. Keep `roko-daimon`'s clean `AffectEngine` trait interface
2. Add `roko-golem`'s per-task tracking (`HashMap<String, AffectState>`)
3. Add `roko-golem`'s octant classification (`AffectOctant::from_pad()`)
4. Add `roko-golem`'s behavior modulation struct (`AffectBehaviorModulation`)
5. Add `roko-golem`'s signal emission
6. Remove `roko-golem/src/daimon.rs` after migration

---

## Scaffolded Components

These exist as types or stubs but are not wired into the runtime:

| Component | Where | What Exists | What's Missing |
|---|---|---|---|
| DispatchStrategy effort labels | roko-daimon | Enum + `effort_label()` method | Not read by any consumer |
| Daimon context block | roko-compose | Template slot exists | Daimon doesn't write to it |
| CascadeRouter threshold adjustment | roko-learn | Router exists, thresholds exist | Router doesn't read Daimon thresholds |

---

## Specified but Not Implemented

These are fully specified in the legacy PRDs and/or `refactoring-prd` but have no code:

### Implementation Plan Reference (12a-cognitive-layer.md)

| Task | Description | Priority |
|---|---|---|
| F1 | `PadVector` struct and `AffectState` | **Done** |
| F2 | 8 octant states from PAD sign | **Done** (roko-golem) |
| F3 | `AffectEvent` enum and `AffectEngine::appraise()` | **Done** |
| F4 | Temporal decay (exponential, 4h half-life) | **Done** |
| F5 | Behavior modulation table | **Done** (both crates) |
| F6 | Affect signatures on episodes | **Not done** — episodes don't carry EmotionalTag |
| F7 | Affect → SystemPromptBuilder | **Not done** — Daimon doesn't write to prompt context |
| F8 | Affect → CascadeRouter | **Not done** — router doesn't read Daimon thresholds |
| F9 | Persistence (autosave + load) | **Done** |

### Unimplemented Features by Category

**Somatic Landscape**:
- `SomaticLandscape` struct with k-d tree over 8D strategy space
- `SomaticMarker` struct with valence, intensity, episode provenance
- Pre-action query protocol (nearest-neighbor search)
- Marker creation from significant outcomes
- Marker consolidation during dream cycles
- `SomaticMarkerFired` event emission
- 15% contrarian retrieval within somatic queries
- Strategy space computer (`StrategySpaceComputer` trait)
- 8D dimension computation for coding domain
- `kiddo` crate dependency

**Emotional Memory Integration**:
- `EmotionalTag` on Engrams
- Four-factor retrieval scoring (recency × importance × relevance × emotional congruence)
- PAD cosine similarity for retrieval scoring
- Emotional provenance tracking on consolidated knowledge
- Emotional diversity as quality signal
- Consolidation priority boost for high-arousal episodes
- Mind wandering mechanism (spontaneous retrieval every ~200 ticks)

**Collective Contagion**:
- Inter-agent emotional contagion protocol
- P/A attenuation (0.3), D blocking (0.0)
- Arousal cap per sync cycle (+0.3)
- Unidirectional propagation enforcement
- 6-hour borrowed-emotion decay
- Somatic field aggregation across mesh
- C-Factor measurement

**VCG Auction Integration**:
- Urgency multiplier from arousal: `urgency = 1 + arousal × 0.5`
- Affect weight from pleasure extremes: `affect_weight = 1 + 0.3 × abs(pleasure - 0.5)`
- Per-subsystem bid modulation based on PAD state

**Coding Agent Integration**:
- Per-crate confidence aggregation
- Error pattern tracker with familiarity scoring
- Fatigue detection (consecutive failure monitoring)
- Error category extraction from gate output

**Future Appraisal Triggers**:
- Prediction accuracy (depends on Tier 2J)
- Peer comparison (depends on Tier 2M)
- Novel domain entry
- Repeated pattern success
- Knowledge contradiction (depends on Tier 2A)

**Dream-Daimon Bridge**:
- Emotional load computation for dream urgency
- Mood-biased replay content selection
- REM depotentiation (arousal *= 0.70 per cycle for A > 0.5)
- Dream outcome appraisal (validated/refuted/novel/threat)
- DreamEmotionalMetadata on dream-produced Engrams

---

## Skipped Legacy Files

The following legacy source files were deliberately skipped during migration because they contain mortality-specific content that is incompatible with the new architecture:

| File | Reason Skipped | Citations Extracted |
|---|---|---|
| `bardo-backup/prd/03-daimon/04-mortality-daimon.md` | Mortality-specific emotional mapping. Three mortality emotions (Economic Anxiety, Epistemic Vertigo, Stochastic Dread) are incompatible with the new architecture which removes death framing. | Heidegger 1927 (Befindlichkeit) noted but not used — too philosophical for the engineering spec. |
| `bardo-backup/prd/03-daimon/05-death-daimon.md` | Death protocol emotional processing. Thanatopsis, emotional life review, death testament — all removed in the new architecture. | None used. |

**What was preserved from these files**: The general principle that resource scarcity creates emotional pressure is preserved — but the source of scarcity is now budget/deadline/confidence constraints, not existential mortality. See [00-vision-and-mortality-incompatibility.md](./00-vision-and-mortality-incompatibility.md) for the full reframing rationale.

---

## Implementation Priority Path

Based on `refactoring-prd/07-implementation-priorities.md`:

| Tier | Tasks | Status |
|---|---|---|
| **0C** | Dissolve roko-golem, consolidate affect logic into roko-daimon | Not started |
| **2D** | Daimon PAD tracking (F1-F5, F9) — core appraisal and modulation | **Complete** |
| **2E** | Behavioral modulation (F5) — behavioral states and dispatch strategy | **Complete** |
| **2D+** | Affect on episodes (F6), affect→SystemPromptBuilder (F7), affect→CascadeRouter (F8) | Not started |
| **2G** | Somatic landscape, 8D strategy space, k-d tree | Not started |
| **2H** | Emotional memory integration (EmotionalTag, four-factor retrieval) | Not started |
| **2I** | Dream-daimon bridge (emotional load, depotentiation) | Not started |
| **2M** | Collective contagion, somatic field, C-Factor | Not started |

### Recommended Next Steps

1. **Consolidate crates** (Tier 0C): Move roko-golem/daimon.rs into roko-daimon. This unblocks all subsequent work by providing a single canonical implementation.

2. **Wire affect→SystemPromptBuilder** (F7): Inject the Daimon's state into the agent's system prompt. This is the lowest-effort integration point with the highest behavioral impact — the LLM sees the emotional context and adjusts its reasoning accordingly.

3. **Wire affect→CascadeRouter** (F8): Pass adjusted tier thresholds to the CascadeRouter. This creates the compute allocation feedback loop.

4. **Add EmotionalTag to episodes** (F6): Tag episodes with PAD state at creation time. This enables mood-congruent retrieval and emotional consolidation bias when those systems are built.

---

## Cross-references

- See [00-vision-and-mortality-incompatibility.md](./00-vision-and-mortality-incompatibility.md) for why mortality files are skipped
- See [10-integration-points.md](./10-integration-points.md) for the four integration points and their wiring status
- See all other sub-docs for per-feature implementation status
