# Current Status and Gaps

> What's built, what's scaffolded, what's specified but unimplemented, and which legacy files were deliberately skipped.


> **Implementation**: Built

**Topic**: [Daimon](./INDEX.md)
**Prerequisites**: All previous sub-docs
**Key sources**: `roko-daimon/src/lib.rs`, `roko-golem/src/daimon.rs`, `refactoring-prd/07-implementation-priorities.md`

---

## Abstract

The Daimon affect engine has moved the shared emotional vocabulary into `roko-core`, while `roko-daimon` now owns the live PAD state, explicit behavioral-state classification, baseline behavioral modulation, and a real somatic landscape for coding-task routing. The largest remaining gaps are no longer the existence of a fast affective path, but the richer PRD control surfaces layered on top of it: collective contagion, deeper VCG bidding/pricing integration, domain-extensible strategy spaces, and several frontier appraisal triggers. This document catalogs the exact state of each component, references the implementation priority tiers, and lists the legacy source files that were deliberately skipped during migration.

---

## Implemented Components

### roko-daimon (standalone crate)

**File**: `crates/roko-daimon/src/lib.rs`

| Component | Status | Description |
|---|---|---|
| Shared `PadVector` struct | **Complete** | Canonical type lives in `roko-core`, with clamp/delta/decay/similarity helpers |
| `AffectState` struct | **Complete** | PAD + confidence + explicit behavioral_state + updated_at timestamp |
| `AffectEvent` enum | **Complete** | 6 variants: GateResult, TaskOutcome, Blocked, TimePressure, QueueWait, DreamFailure |
| `DaimonState` struct | **Complete** | Wraps AffectState + half_life_hours + persistence_path |
| `AffectEngine` trait | **Complete** | 4 methods: appraise(), query(), modulate(), persist() |
| Appraisal rules | **Complete** | All 6 event types with correct PAD deltas, rung scaling, asymmetric valence |
| Temporal decay | **Complete** | Exponential decay: `factor = 0.5 ^ (elapsed_hours / half_life_hours)` |
| Behavioral state classification | **Complete** | Explicit `BehavioralState::classify(pad, confidence)` stored on affect state |
| Behavioral modulation | **Complete** | Model promotion/demotion (haiku↔sonnet↔opus), turn limit adjustment, strategy selection keyed off behavioral state |
| `SomaticLandscape` | **Partial** | Persisted `SomaticMarker` store backed by a `kiddo` k-d tree over the 8D coding strategy space, plus dream-time depotentiation of high-intensity markers |
| Somatic query + modulation | **Partial** | `query_somatic()` and `modulate_with_strategy()` blend nearby and contrarian markers to bias dispatch before task execution; strong matches now emit explicit runtime events |
| Somatic persistence / restore | **Complete** | Marker payloads persist with Daimon state, and `load_or_new()` rebuilds the in-memory index |
| Persistence | **Complete** | Atomic file write (write to .tmp, rename) with auto-save on appraise |
| Load/restore | **Complete** | `load_or_new()` loads from disk or creates fresh neutral state |
| `DispatchStrategy` enum | **Complete** | 5 variants with effort labels: Conservative, Balanced, Exploratory, Escalating, Proactive |
| `DispatchParams` struct | **Complete** | model + turn_limit + strategy + effort |
| `queue_wait_arousal()` | **Complete** | Public function for queue-wait arousal computation |
| `EmotionalTag` generation | **Partial** | Daimon derives emotional tags, the orchestrator stamps conductor engrams and episodes with them, and Neuro now preserves both emotional tags and derived emotional provenance metadata during consolidation and direct knowledge emission |
| Tests | **Complete** | Appraisal, persistence, modulation, behavioral-state, emotional-tag, and somatic-landscape coverage |

### Removed legacy affect implementation

The old `roko-golem/src/daimon.rs` path has been removed from the active codebase. The migration outcome is narrower than the historical per-task design: `roko-daimon` is now the canonical affect engine, with a single persisted affect state plus a persisted somatic landscape used to bias dispatch. The remaining work is feature depth, not crate consolidation.

---

## Scaffolded Components

These exist as types or stubs but are not wired into the runtime:

| Component | Where | What Exists | What's Missing |
|---|---|---|---|
| DispatchStrategy effort labels | roko-daimon | Enum + `effort_label()` method | Still not exposed as a first-class runtime control surface |

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
| F6 | Affect signatures on episodes | **Partial** — Engrams, episodes, Neuro distillation, and direct knowledge emission now carry emotional tags plus derived emotional provenance; the remaining gap is the fuller somatic-landscape path and broader cross-subsystem weighting |
| F7 | Affect → SystemPromptBuilder | **Done** — live Daimon PAD now feeds affect guidance in the system prompt |
| F8 | Affect → CascadeRouter | **Done** — live Daimon behavioral state and confidence now arrive as a first-class `DaimonPolicy` in routing decisions |
| F9 | Persistence (autosave + load) | **Done** |

### Unimplemented Features by Category

**Somatic Landscape**:
- Strategy-space abstractions beyond the current coding-task coordinate projection
- External configuration for alternate domain axis sets
- Direct use of somatic scores inside cross-subsystem context bidding, not just the current dispatch/prompt/retrieval surfaces

**Emotional Memory Integration**:
- Retrieval-time use of `EmotionalTag` is now partially implemented in `ContextAssembler`
- Four-factor retrieval scoring is partially implemented locally in Neuro; the richer cross-subsystem variant is still missing
- PAD cosine similarity is now used for retrieval congruence scoring
- Emotional provenance now transfers onto consolidated knowledge entries
- Emotional diversity and heuristic validation arcs now persist on `KnowledgeEntry.emotional_provenance` and contribute a small retrieval reliability boost
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
- Partially implemented end-to-end: orchestration now passes live PAD state into `PromptComposer`, and the shared prompt auction applies those multipliers plus per-bidder PAD modulation before selecting winners
- Diagnostic externality payments are now emitted on the composed prompt as an approximate VCG accounting layer

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
| **0C** | Dissolve roko-golem, consolidate affect logic into roko-daimon | **Complete** |
| **2D** | Daimon PAD tracking (F1-F5, F9) — core appraisal and modulation | **Complete** |
| **2E** | Behavioral modulation (F5) — behavioral states and dispatch strategy | **Complete** |
| **2D+** | Affect on episodes (F6), affect→SystemPromptBuilder (F7), affect→CascadeRouter (F8) | Mostly complete; routing now has an explicit `DaimonPolicy`, and the remaining gap is somatic-landscape-backed retrieval plus broader cross-subsystem weighting |
| **2G** | Somatic landscape, 8D strategy space, k-d tree | Partial; persisted axis registration plus a shared `StrategySpaceComputer` are now in place, and non-coding domains use role-aware label projection, but true domain-native extractors and VCG coupling are still missing |
| **2H** | Emotional memory integration (EmotionalTag, four-factor retrieval) | Partial |
| **2I** | Dream-daimon bridge (emotional load, depotentiation) | Partial; dream replay now depotentiates arousal/high-intensity markers and synthesizes somatic markers from replayed episodes, but richer emotional maintenance policy is still missing |
| **2M** | Collective contagion, somatic field, C-Factor | Not started |

### Recommended Next Steps

1. **Deepen somatic-landscape semantics**: add true domain-native coordinate extractors on top of the current role-aware projection and tighten cross-subsystem use of the now-centralized strategy-space computer.

2. **Finish emotional-memory scoring**: retrieval weighting now uses emotional provenance and emotional diversity in Neuro; the remaining work is direct somatic-landscape-backed knowledge selection and consolidation priority.

3. **Deepen VCG affect bidding**: broaden bidder production beyond the currently populated runtime sections and add richer fairness / pricing policy on top of the PAD-modulated shared auction.

4. **Layer in collective contagion and frontier appraisal triggers** once the single-agent affect path is fully exploited.

---

## Cross-References

- See [00-vision-and-mortality-incompatibility.md](./00-vision-and-mortality-incompatibility.md) for why mortality files are skipped
- See [10-integration-points.md](./10-integration-points.md) for the four integration points and their wiring status
- See all other sub-docs for per-feature implementation status
