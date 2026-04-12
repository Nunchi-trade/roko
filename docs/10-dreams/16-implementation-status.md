# Implementation Status and Roadmap

> **Layer**: All layers (L0–L4)
>
> **Crate**: `roko-dreams` (primary), `roko-golem` (legacy scaffold, to be dissolved), `roko-learn` (supporting infrastructure)
>
> **Prerequisites**: [00-vision-and-dream-as-death-reframe.md](00-vision-and-dream-as-death-reframe.md)

---

## Current Code Status

The dream subsystem exists across two crates with different maturity levels. This document maps what is implemented, what is scaffolded, and what remains to be built.

### roko-dreams Crate

**Location**: `/Users/will/dev/nunchi/roko/roko/crates/roko-dreams/`

The `roko-dreams` crate contains the primary dream implementation with two modules:

#### `runner.rs` — Dream Runner Facade (Implemented)

The public facade wrapping dream-cycle implementation with a scheduling and consolidation API. This is the most complete piece of the dream subsystem.

**Implemented structs and traits**:

| Type | Status | Description |
|------|--------|-------------|
| `DreamRunner` | **Implemented** | Public facade with `new()`, `replay_insights()`, `latest_report()`, `consolidate_now()`, `schedule_next()` |
| `DreamEngine` trait | **Implemented** | Trait with `replay()`, `consolidate()`, `schedule()` methods |
| `DreamLoopConfig` | **Implemented** | Configuration: `auto_dream`, `idle_threshold_mins`, `min_episodes_for_dream`, `agent` |
| `DreamAgentConfig` | **Implemented** | Agent backend config: `command`, `args`, `model`, `bare_mode`, `effort`, `timeout_ms`, `env` |
| `DreamReviewAgent` | **Implemented** | Enum dispatching to `ClaudeCliAgent` or `ExecAgent` for dream inference |

**Key implemented behaviors**:

1. **Scheduling logic** (`DreamEngine::schedule()`): Reads episodes from `EpisodeLogger`, filters to episodes after the last dream report's `processed_through` timestamp, checks against `min_episodes_for_dream` threshold, computes idle time against `idle_threshold_mins`, returns `Some(Duration::ZERO)` for immediate fire or `Some(Duration)` for future scheduling.

2. **Consolidation pipeline** (`consolidate_async()`): Creates `EpisodeLogger`, `KnowledgeStore`, `PlaybookStore`, and `AgentDispatcher`, then runs `DreamCycle::run()`.

3. **Report persistence**: Dream reports are persisted as `dream-{timestamp_ms}.json` files in `.roko/dreams/`. The `latest_report()` method scans the directory for the most recent report file.

4. **Agent dispatch**: Supports both `ClaudeCliAgent` (for Claude CLI-based dream inference) and `ExecAgent` (for arbitrary command execution). Default model: `claude-opus-4-6` (configurable).

#### `cycle.rs` — Dream Cycle Engine (Implemented)

The core dream cycle implementation with the three-phase processing pipeline.

**Implemented types**:

| Type | Status | Description |
|------|--------|-------------|
| `DreamCycle` | **Implemented** | Core cycle engine with `new()` and `run()` |
| `DreamCycleReport` | **Implemented** | Report struct with timestamps, insights, and processing metadata |
| `AgentDispatcher` trait | **Implemented** | Trait for dispatching dream inference to agent backends |

#### `lib.rs` — Re-exports

```rust
pub mod cycle;
pub mod runner;

pub use cycle::{AgentDispatcher, DreamCycle, DreamCycleReport};
pub use roko_golem::{DreamsEngine, GolemSubsystemId, GolemSubsystemSummary, ScaffoldEngine};
pub use runner::{
    DreamAgentConfig, DreamConfig, DreamEngine, DreamLoopConfig,
    DreamReport, DreamRunner, Episode, Insight,
};
```

Note the re-exports from `roko_golem` — these are legacy scaffold types that will be removed when `roko-golem` is dissolved.

### roko-golem Crate (Legacy Scaffold — To Be Dissolved)

**Location**: `/Users/will/dev/nunchi/roko/roko/crates/roko-golem/`

The `roko-golem` crate contains placeholder scaffolds for several cognitive subsystems. The dream-related scaffolds are:

#### `dreams.rs` — Placeholder (43 lines)

```rust
pub struct DreamsEngine {
    id: String,
}

impl DreamsEngine {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

impl ScaffoldEngine for DreamsEngine {
    fn summary(&self) -> GolemSubsystemSummary {
        GolemSubsystemSummary {
            id: GolemSubsystemId::Dreams,
            label: "Dreams".into(),
            one_liner: "Offline consolidation & insight replay".into(),
        }
    }

    fn replay(&self) -> &str {
        "dreams-replay-stub"
    }
}
```

This is a pure placeholder — no actual dream logic. All real implementation lives in `roko-dreams`.

#### `hypnagogia.rs` — Placeholder (43 lines)

```rust
pub struct HypnagogiaEngine {
    _id: String,
}

impl HypnagogiaEngine {
    pub fn new(id: impl Into<String>) -> Self {
        Self { _id: id.into() }
    }
}

impl ScaffoldEngine for HypnagogiaEngine {
    fn summary(&self) -> GolemSubsystemSummary {
        GolemSubsystemSummary {
            id: GolemSubsystemId::Hypnagogia,
            label: "Hypnagogia".into(),
            one_liner: "Sleep-onset creativity via noise injection".into(),
        }
    }

    fn interrupt(&self) -> &str {
        "hypnagogia-interrupt-stub"
    }
}
```

Also a pure placeholder. The Hypnagogia engine is fully designed (see [07-hypnagogia-engine.md](07-hypnagogia-engine.md)) but not yet implemented.

### roko-learn Crate (Supporting Infrastructure — Implemented)

**Location**: `/Users/will/dev/nunchi/roko/roko/crates/roko-learn/`

Several `roko-learn` modules provide infrastructure that dreams depend on:

#### `pattern_discovery.rs` — Pattern Mining (978 lines, Implemented)

| Component | Status | Description |
|-----------|--------|-------------|
| `PatternMiner` | **Implemented** | Trigram mining across episodes with FNV-1a hashing |
| `EpisodeView` trait | **Implemented** | Abstraction for episode access (`actions()`, `succeeded()`) |
| `Pattern` struct | **Implemented** | Pattern with id, signature, description, support_count, confidence |
| `CrossEpisodeConsolidator` | **Implemented** | K-medoids clustering over HDC episode vectors |
| `CrossEpisodeMetaPattern` | **Implemented** | Meta-pattern with bundle_vector, medoid_vector, coherence |
| `episode_vector()` | **Implemented** | Encodes structural features into HDC vectors |

Comprehensive test suite: 12+ tests including synthetic pattern recovery.

#### `hdc_clustering.rs` — K-Medoids Clustering (498 lines, Implemented)

| Component | Status | Description |
|-----------|--------|-------------|
| `KMedoidsConfig` | **Implemented** | Configuration: k (cluster count), max_iterations |
| `k_medoids()` | **Implemented** | Greedy farthest-first seeding → assign → update loop |
| `HdcCluster` | **Implemented** | Cluster with medoid_index, medoid vector, member indices |
| `ClusterResult` | **Implemented** | Result with clusters, iterations, convergence flag |

Comprehensive test suite: 9 tests including synthetic 3-cluster recovery.

#### `episode_logger.rs` — Episode Logging (Implemented)

Records agent turns as episodes in `.roko/episodes.jsonl`. The `EpisodeLogger` is used directly by `DreamRunner` to read episodes for dream scheduling and consolidation.

#### `playbook.rs` — Playbook Store (Implemented)

Manages playbook revisions in `.roko/learn/playbooks/`. The `PlaybookStore` is passed to `DreamCycle` for dream-generated strategy updates.

### roko-neuro Crate (Knowledge Store — Implemented)

**Location**: `/Users/will/dev/nunchi/roko/roko/crates/roko-neuro/`

| Component | Status | Description |
|-----------|--------|-------------|
| `KnowledgeStore` | **Implemented** | Persistent knowledge storage used by dream consolidation |
| `TierProgression` | **Implemented** | Insight classification into T0–T4 tiers |
| `InsightRecord` | **Implemented** | Dream-generated insight data structure |

---

## Implementation Plan Items (§G: Dreams — Offline Learning)

The implementation plan at `/Users/will/dev/nunchi/roko/roko/tmp/implementation-plans/12a-cognitive-layer.md` §G defines the dream subsystem roadmap:

### G1: Episode Replay Scheduler

**Status**: Partially implemented

The scheduling logic exists in `DreamEngine::schedule()` in `runner.rs`. What remains:

| Item | Status |
|------|--------|
| Idle-time trigger (primary) | **Implemented** — checks `idle_threshold_mins` against episode timestamps |
| Scheduled trigger (secondary) | **Not implemented** — `scheduled_interval_hours` config exists in spec but not in code |
| Manual trigger (CLI) | **Not implemented** — `roko dream run` CLI command not yet wired |
| Intensive consolidation mode | **Not implemented** — backlog high/low water marks not coded |

### G2: Episode Re-evaluation

**Status**: Partially implemented via `TierProgression::analyze()`

The `replay_insights()` method in `DreamRunner` calls `TierProgression::analyze()` to classify episodes into insight tiers. Full re-evaluation with Mattar-Daw utility scoring is not yet implemented.

### G3: Mistake Identification

**Status**: Not implemented

Automated identification of failure patterns across episodes. The `PatternMiner` infrastructure exists but is not yet wired into the dream cycle for mistake-specific mining.

### G4: Heuristic Strengthening

**Status**: Partially implemented

Confidence updates for validated heuristics exist in the `TierProgression` system. Dream-specific strengthening (increasing confidence of dream-generated insights that receive waking validation) is not yet wired.

### G5: Counterfactual Simulation

**Status**: Not implemented

REM-phase counterfactual generation using Pearl's SCM framework. The design is fully specified (see [03-rem-imagination.md](03-rem-imagination.md)) but no code exists.

### G6: Cross-Episode Consolidation

**Status**: Implemented (infrastructure)

The `CrossEpisodeConsolidator` in `roko-learn/src/pattern_discovery.rs` provides K-medoids clustering over HDC episode vectors. This infrastructure is ready but not yet called from the dream cycle.

### G7: Novel Strategy Generation

**Status**: Not implemented

Creative strategy generation using Boden's three modes (combinational, exploratory, transformational). The design is specified in [03-rem-imagination.md](03-rem-imagination.md).

### G8: Dream Report Persistence

**Status**: Implemented

`DreamCycleReport` is serialized to JSON and persisted in `.roko/dreams/`. The `load_latest_dream_report()` function retrieves the most recent report for scheduling decisions.

---

## roko-golem Dissolution Plan

Per the naming map (`context-pack/01-naming-map.md`), `roko-golem` is to be dissolved. Dream-related modules move to `roko-dreams`:

| Current Location | Target Location | Content |
|-----------------|----------------|---------|
| `roko-golem/src/dreams.rs` | **Delete** | Pure placeholder; real implementation already in `roko-dreams` |
| `roko-golem/src/hypnagogia.rs` | `roko-dreams/src/hypnagogia.rs` | Move and implement per [07-hypnagogia-engine.md](07-hypnagogia-engine.md) |
| `roko-golem` `ScaffoldEngine` trait | **Dissolve** | Replace with proper `DreamEngine` trait (already exists in `roko-dreams`) |
| `roko-dreams/src/lib.rs` re-exports from `roko-golem` | **Remove** | Drop `DreamsEngine`, `GolemSubsystemId`, `GolemSubsystemSummary`, `ScaffoldEngine` re-exports |

After dissolution:
- `roko-dreams` depends on `roko-learn`, `roko-neuro`, `roko-agent` — not on `roko-golem`
- The `DreamEngine` trait in `roko-dreams/src/runner.rs` becomes the canonical dream interface
- `ScaffoldEngine` is fully replaced by `DreamEngine`

---

## Implementation Roadmap

### Phase 1: Core Dream Loop (Current)

| Item | Status | Priority |
|------|--------|----------|
| DreamRunner facade | **Done** | — |
| DreamEngine trait | **Done** | — |
| Scheduling logic (idle trigger) | **Done** | — |
| Episode reading from EpisodeLogger | **Done** | — |
| DreamCycleReport persistence | **Done** | — |
| Agent dispatch (Claude/Exec) | **Done** | — |
| K-medoids clustering | **Done** | — |
| Pattern mining | **Done** | — |

### Phase 2: Complete Dream Cycle

| Item | Status | Priority |
|------|--------|----------|
| Scheduled trigger (fixed interval) | Not started | High |
| CLI commands (`roko dream run/report/history`) | Not started | High |
| Intensive consolidation mode (backlog) | Not started | Medium |
| Mattar-Daw utility scoring for replay | Not started | High |
| Wire PatternMiner into dream cycle | Not started | High |
| Wire CrossEpisodeConsolidator into dream cycle | Not started | High |
| Mistake identification from failure patterns | Not started | Medium |

### Phase 3: REM and Creativity

| Item | Status | Priority |
|------|--------|----------|
| Pearl SCM counterfactual generation | Not started | Medium |
| Boden's three creativity modes | Not started | Medium |
| Emotional depotentiation | Not started | Medium |
| Threat simulation (Revonsuo) | Not started | Low |
| Hypnagogia engine (4 layers) | Not started | Low |

### Phase 4: Integration and Feedback

| Item | Status | Priority |
|------|--------|----------|
| Dream → gate threshold updates | Not started | Medium |
| Dream → CascadeRouter updates | Not started | Medium |
| Dream → playbook revisions | Not started | Medium |
| Mesh knowledge sharing of dream insights | Not started | Low |
| Dream feedback into plan generator | Not started | Low |

### Phase 5: Oneirography (Domain Extension)

| Item | Status | Priority |
|------|--------|----------|
| Dream image generation pipeline | Not started | Low |
| Self-appraisal system | Not started | Low |
| Affect-reactive auctions | Not started | Low |
| Extended art forms | Not started | Low |
| Steganographic encoding | Not started | Low |

---

## Key Dependencies

| Dependency | Required By | Status |
|-----------|------------|--------|
| `roko-learn::EpisodeLogger` | Dream scheduling, episode replay | **Available** |
| `roko-learn::PatternMiner` | NREM pattern discovery | **Available** (not wired) |
| `roko-learn::CrossEpisodeConsolidator` | NREM cross-episode consolidation | **Available** (not wired) |
| `roko-learn::hdc_clustering::k_medoids` | Dream content clustering | **Available** |
| `roko-neuro::KnowledgeStore` | Knowledge read/write during dreams | **Available** |
| `roko-neuro::TierProgression` | Insight tier classification | **Available** |
| `roko-agent::ClaudeCliAgent` | Dream inference backend | **Available** |
| `roko-agent::ExecAgent` | Dream inference backend (fallback) | **Available** |
| `bardo-runtime::ProcessSupervisor` | Dream process lifecycle | **Available** |
| Daimon (affect engine) | PAD vectors for emotional context | **Not yet implemented** |
| HDC vectors (`bardo-primitives`) | Counterfactual synthesis | **Built** (not called from dreams) |

---

## Academic References Driving Implementation

The implementation plan items map to specific academic foundations:

| Plan Item | Academic Basis |
|-----------|---------------|
| G1 (Replay Scheduler) | Lin et al. 2025 — sleep-time compute; Walker 2009 — sleep scheduling |
| G2 (Re-evaluation) | Mattar & Daw, "Prioritized memory access explains planning and hippocampal replay," *Nature Neuroscience*, 2018 |
| G3 (Mistake Identification) | Epstude & Roese, "The Functional Theory of Counterfactual Thinking," *PSPR*, 2008 |
| G5 (Counterfactual Simulation) | Pearl, "Causality," Cambridge University Press, 2009; Byrne, "The Rational Imagination," MIT Press, 2005 |
| G6 (Cross-Episode Consolidation) | McClelland et al., "Why There Are Complementary Learning Systems," *Psychological Review*, 1995 |
| G7 (Novel Strategy Generation) | Boden, "The Creative Mind: Myths and Mechanisms," Routledge, 2004 |
| Hypnagogia | Gammaitoni et al., "Stochastic Resonance," *Reviews of Modern Physics*, 1998 |
| Emotional depotentiation | Walker & van der Helm, "Overnight Therapy?" *Psychological Bulletin*, 2009 |
| Threat simulation | Revonsuo, "The reinterpretation of dreams," *Behavioral and Brain Sciences*, 2000 |

---

## Open Questions

1. **Dream cycle interruption**: Should a dream cycle be interruptible if a high-priority task arrives? Current design says no (dreams never interrupt tasks, tasks never interrupt dreams), but this may need revisiting for time-sensitive domains.

2. **Multi-agent dream coordination**: When multiple agents in a mesh dream simultaneously, should they coordinate to avoid redundant consolidation? Or is independent consolidation valuable for divergence?

3. **Dream depth vs. breadth**: Should a dream cycle process many episodes shallowly or few episodes deeply? The current batch_size=10 is arbitrary. Optimal batch sizing may depend on episode complexity.

4. **Hypnagogia temperature tuning**: The T=1.3 for the Executive Loosener and T=0.4 for the Homuncular Observer are from the legacy spec. These may need empirical tuning per domain.

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [00-vision-and-dream-as-death-reframe.md](00-vision-and-dream-as-death-reframe.md) | Architectural vision and reframe context |
| [01-three-phase-cycle.md](01-three-phase-cycle.md) | Cycle structure that implementation must follow |
| [07-hypnagogia-engine.md](07-hypnagogia-engine.md) | Hypnagogia design for Phase 3 implementation |
| [12-sleep-time-compute.md](12-sleep-time-compute.md) | Compute budget constraints on implementation |
| [13-scheduling-and-triggers.md](13-scheduling-and-triggers.md) | Scheduling spec that G1 partially implements |
| [15-cross-system-integration.md](15-cross-system-integration.md) | Integration points that Phase 4 must wire |
