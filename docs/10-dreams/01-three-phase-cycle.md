# The Three-Phase Dream Cycle

> **Layer**: Cognitive Cross-Cut (L0 scheduling + L1 agent dispatch + L2 context assembly)
>
> **Synapse Traits**: `Scorer` (Mattar-Daw utility scoring), `Gate` (staging buffer validation), `Composer` (Engram assembly from dream outputs)
>
> **Crate**: `roko-dreams` — `cycle.rs`, `runner.rs`
>
> **Prerequisites**: [00-vision-and-dream-as-death-reframe.md](00-vision-and-dream-as-death-reframe.md)

---

## Overview

Every dream cycle in Roko consists of three sequential phases, inspired by biological mammalian sleep:

1. **NREM Replay** — Replaying past experiences with minor mutations to consolidate memory
2. **REM Imagination** — Generating counterfactual scenarios and novel strategies through creative recombination
3. **Integration** — Evaluating dream outputs, updating the staging buffer, and promoting validated hypotheses to permanent knowledge

These phases are not metaphorical labels. Each phase implements a distinct computational process with its own academic foundations, model requirements, and output types. The three-phase structure is grounded in the Complementary Learning Systems (CLS) theory of McClelland et al. (1995) and the specific consolidation mechanisms documented by Diekelmann & Born (2010, Psychological Review).

---

## Phase 1: NREM Replay

**Purpose**: Strengthen significant memories, weaken irrelevant ones, and extract cross-episode patterns.

**Biological basis**: During non-REM sleep (particularly slow-wave sleep stages N2-N3), the hippocampus replays compressed versions of recent experiences at accelerated timescales. Sharp-wave ripples coordinate the transfer of episodic memories from hippocampus to neocortex (Buzsáki 1989; Ji & Wilson 2007). The replay is not veridical — it includes minor mutations that test whether patterns hold under perturbation.

**Computational implementation**: The NREM replay phase selects episodes from the episode log using the Mattar & Daw (2018, Nature Neuroscience) utility formula:

```
Utility(episode) = Gain × Need × (1 / spacing_penalty)
```

Where:
- **Gain** measures how much the agent's behavior would improve by better processing this episode. Episodes where the outcome was surprising (the agent's prediction error was large) have high gain.
- **Need** measures how often the agent encounters situations similar to this episode. Episodes from frequently encountered task types have high need.
- **Spacing penalty** implements spacing effects (Cepeda et al. 2006) — recently replayed episodes are penalized to prevent over-rehearsal.

The selected episodes are replayed with two types of mutation:

| Mutation Type | Description | Research Basis | Frequency |
|---------------|-------------|----------------|-----------|
| **Perturbation** | Key values within the episode are shifted within a plausible range (e.g., a parameter that was `0.30` becomes `0.25`) | Perturbed replay from hippocampal studies | 30% of replays |
| **Bidirectional replay** | Episodes are replayed both forward (cause→effect) and backward (effect→cause) to strengthen causal associations in both directions | Ambrose et al. (2016), Reverse Replay | All replays |

The replay phase uses a **Haiku-class model** (cheap, fast) because it primarily involves pattern matching and comparison against existing knowledge, not creative generation.

> **Full detail**: [02-nrem-replay.md](02-nrem-replay.md)

---

## Phase 2: REM Imagination

**Purpose**: Generate novel strategies and counterfactual hypotheses through creative recombination of existing knowledge.

**Biological basis**: During REM sleep, the brain enters a state of high-level cortical activation without sensory input. The prefrontal cortex (executive control) is suppressed while associative cortex is active, enabling novel combinations of memories that would be inhibited during waking (Hobson & Schredl 2011). Walker & van der Helm (2009, Psychological Bulletin) demonstrated that REM sleep specifically reduces the emotional charge of traumatic memories — "overnight therapy."

**Computational implementation**: The REM phase operates in three creativity modes, following Boden's (2004) taxonomy:

| Creativity Mode | Operation | Prompt Structure | Example |
|----------------|-----------|-----------------|---------|
| **Combinational** | Combine elements from unrelated episodes to discover unexpected similarities | "Given episode A about [topic X] and episode B about [topic Y], what structural patterns do they share?" | Noticing that gas price spikes and governance vote deadlines share the same timing pattern |
| **Exploratory** | Traverse the boundaries of existing strategy spaces, pushing parameters to extremes | "Your current heuristic says [X]. What happens at the extreme of this rule? When would it break?" | Testing whether a "always retry failed tasks 3 times" heuristic still works at 10 retries |
| **Transformational** | Violate fundamental assumptions of existing strategies to generate genuinely novel approaches | "What if [core assumption] were false? What strategy would you use instead?" | Imagining that compilation errors are actually test failures — what would the response be? |

The REM phase also implements **counterfactual reasoning** via Pearl's (2009) three-level structural causal model (SCM) framework:

1. **Association** (Level 1): What correlates with what in the episode data?
2. **Intervention** (Level 2): What would happen if the agent had taken a different action?
3. **Counterfactual** (Level 3): Given what actually happened, what would have happened if conditions had been different?

Byrne's (2005, The Rational Imagination) "fault lines" guide which counterfactuals the agent explores first:
- **Controllable actions**: Things the agent could have done differently (highest priority)
- **Recent actions**: Temporally proximate decisions (second priority)
- **Abnormal actions**: Decisions that deviated from the agent's usual patterns (third priority)

Epstude & Roese (2008, Personality and Social Psychology Review) provide the functional theory: **upward counterfactuals** ("what if I had done better?") drive self-improvement, while **downward counterfactuals** ("what if I had done worse?") serve as rehearsal for future threats.

**Emotional depotentiation**: During REM processing, Walker & van der Helm (2009) showed that the emotional charge of memories decreases by 0.3–0.5 units per cycle (on a 0–1 arousal scale). This is implemented as a direct update to the Daimon's PAD arousal dimension for each episode processed:

```
post_dream_arousal = pre_dream_arousal - depotentiation_delta
depotentiation_delta ∈ [0.3, 0.5] per cycle
```

The REM phase uses a **Sonnet-class model** (more capable, more expensive) because creative recombination requires genuine reasoning rather than pattern matching.

> **Full detail**: [03-rem-imagination.md](03-rem-imagination.md)

---

## Phase 3: Integration

**Purpose**: Evaluate the outputs from NREM replay and REM imagination, stage validated hypotheses, and promote them to permanent knowledge.

**Biological basis**: The integration phase corresponds to the brief waking periods between sleep cycles (interspersed micro-arousals) and the consolidation that occurs during the transition between NREM and REM. During these transitions, the brain evaluates which memories have been sufficiently strengthened and which should decay (Stickgold & Walker 2013).

**Computational implementation**: Integration is a **pure computation phase** — no LLM call is required. It operates on the outputs from NREM and REM:

### The Staging Buffer

Dream-generated hypotheses enter a SQLite staging buffer at confidence level 0.20–0.30. This is the "maybe" zone — the hypothesis is interesting enough to record but not validated enough to act on.

```sql
CREATE TABLE staged_hypotheses (
    id TEXT PRIMARY KEY,
    content TEXT NOT NULL,
    confidence REAL DEFAULT 0.25,
    source_phase TEXT NOT NULL,        -- 'nrem' or 'rem'
    source_episodes TEXT,              -- JSON array of episode IDs
    created_at INTEGER NOT NULL,       -- Unix timestamp
    last_validated_at INTEGER,
    validation_count INTEGER DEFAULT 0,
    status TEXT DEFAULT 'staged'       -- staged | validated | promoted | expired | refuted
);
```

### Confidence Ladder

Hypotheses climb a confidence ladder through waking validation:

| Confidence Range | Status | What Happens |
|-----------------|--------|--------------|
| 0.20–0.30 | **Staged** | Hypothesis just entered from a dream. No action taken yet. |
| 0.30–0.50 | **Partially Validated** | Some waking evidence supports the hypothesis. Confidence boosted by 0.10 per confirmation. |
| 0.50–0.70 | **Strongly Supported** | Multiple confirmations. The agent begins to act on this hypothesis tentatively. |
| ≥ 0.70 | **Promoted** | Hypothesis is written to permanent NeuroStore as a validated insight or heuristic. |

### Promotion to Permanent Knowledge

When a hypothesis reaches confidence ≥ 0.70, the integration phase promotes it:

1. The hypothesis is written to NeuroStore as a `KnowledgeEntry` with `source: "dream"` provenance
2. If the hypothesis represents a new heuristic (actionable rule), it is also written to `PLAYBOOK.md`
3. The staging buffer entry is updated to `status: 'promoted'`
4. A `DreamOutcomeEvent` is emitted for downstream listeners

### Expiration

Hypotheses that are not validated within a configurable window (default: 5,000 ticks / ~3.5 days) expire:

1. The staging buffer entry is updated to `status: 'expired'`
2. The hypothesis enters normal temporal decay in NeuroStore if it was already partially stored
3. No action is taken — the hypothesis was not confirmed by waking experience

### What Integration Produces

| Output | Description | Destination |
|--------|-------------|-------------|
| Promoted insights | Validated hypotheses that became permanent knowledge | NeuroStore (`Substrate`) |
| Updated confidence scores | Existing knowledge entries whose confidence was adjusted by dream replay | NeuroStore |
| Emotional depotentiation | Reduced arousal scores for processed episodes | Daimon PAD update |
| Meta-patterns | Cross-episode structural similarities discovered by HDC clustering | NeuroStore |
| Dream report | A structured `DreamCycleReport` summarizing the cycle | `.roko/dreams/dream-{timestamp}.json` |

> **Full detail**: [04-consolidation-and-staging.md](04-consolidation-and-staging.md)

---

## Dream Cycle State Machine

The dream cycle progresses through a deterministic state machine:

```
IDLE → NREM_REPLAY → REM_IMAGINATION → INTEGRATION → IDLE
```

Each state transition is logged. The agent cannot be interrupted mid-phase (the current phase runs to completion before any transition). Between full dream cycles, the agent may enter a micro-consolidation mode where only the most urgent single replay is processed — this handles the case where the agent is briefly idle between tasks but not idle enough for a full dream cycle.

```rust
pub enum DreamPhase {
    /// No dream in progress. Agent is in waking mode.
    Idle,
    /// NREM replay: re-processing past episodes with mutations.
    NremReplay {
        episodes_to_replay: usize,
        episodes_replayed: usize,
    },
    /// REM imagination: counterfactual generation and creative recombination.
    RemImagination {
        counterfactuals_to_generate: usize,
        counterfactuals_generated: usize,
    },
    /// Integration: evaluating outputs, staging, promoting.
    Integration {
        hypotheses_to_evaluate: usize,
        hypotheses_evaluated: usize,
    },
}
```

---

## Resource Allocation Across Phases

Each phase has different computational requirements:

| Phase | Model Tier | Context Window | Typical Duration | Cost Profile |
|-------|-----------|----------------|------------------|-------------|
| **NREM Replay** | Haiku-class (T0) | Minimal — episode data + current heuristics | 60–120 seconds for a batch of 10 episodes | Low (~$0.001/episode) |
| **REM Imagination** | Sonnet-class (T1) | Full — needs broad knowledge context for creative recombination | 120–300 seconds for 3–5 counterfactuals | Medium (~$0.01/counterfactual) |
| **Integration** | None (pure computation) | N/A — operates on structured data only | < 5 seconds | Negligible |

The asymmetry is deliberate. NREM replay is cheap pattern matching — a fast, inexpensive model suffices. REM imagination requires genuine reasoning and creativity — a more capable model is worth the cost. Integration is arithmetic and database operations — no model needed at all.

The CascadeRouter (see `crates/roko-learn/src/cascade_router.rs`) handles model selection for each phase. During dreaming, the agent signals `SIGPAUSE` to any Gamma-frequency processes, allowing the Delta-frequency dream cycle to consume the compute budget.

---

## Concurrent Execution

Dream phases execute sequentially within a single dream cycle, but multiple aspects of NREM replay can run concurrently:

- Multiple episodes can be replayed in parallel (each replay is independent)
- Replays do not share state until the aggregation step at the end of NREM
- REM imagination is sequential (each counterfactual builds on previous context)
- Integration is a single-threaded batch operation

The `DreamCycle` implementation in `crates/roko-dreams/src/cycle.rs` manages this concurrency.

---

## Dream Scheduling

The dream scheduler determines when and how often to fire dream cycles:

```rust
impl DreamRunner {
    pub fn schedule(&self) -> Option<Duration> {
        if !self.config.auto_dream {
            return None;
        }

        // Count episodes since last dream
        let recent_episodes = count_episodes_since_last_dream();
        if recent_episodes < self.config.min_episodes_for_dream {
            return None;
        }

        // Check idle threshold
        let last_activity = most_recent_episode_timestamp();
        let idle_duration = now - last_activity;
        if idle_duration >= self.config.idle_threshold_mins {
            return Some(Duration::ZERO); // Dream now
        }

        // Schedule for when idle threshold will be reached
        let remaining = self.config.idle_threshold_mins - idle_duration;
        Some(remaining)
    }
}
```

Key scheduling parameters:

| Parameter | Default | Description |
|-----------|---------|-------------|
| `auto_dream` | `true` | Whether idle-triggered dreaming is enabled |
| `idle_threshold_mins` | `15` | Minutes of inactivity before triggering a dream |
| `min_episodes_for_dream` | `5` | Minimum unprocessed episodes required before a dream can fire |

The scheduler interacts with the L4 Orchestration layer: the plan executor knows when the agent is between tasks and can signal the dream scheduler that an idle period is beginning.

---

## Dream Cycle Output: The DreamCycleReport

Every completed dream cycle produces a `DreamCycleReport` — a structured JSON document persisted to `.roko/dreams/`:

```rust
pub struct DreamCycleReport {
    /// When the dream cycle started.
    pub started_at: DateTime<Utc>,
    /// When the dream cycle completed.
    pub completed_at: DateTime<Utc>,
    /// Timestamp of the most recent episode processed.
    pub processed_through: Option<DateTime<Utc>>,
    /// Number of episodes replayed during NREM.
    pub episodes_replayed: usize,
    /// Number of counterfactuals generated during REM.
    pub counterfactuals_generated: usize,
    /// Insights extracted during the cycle.
    pub insights: Vec<InsightRecord>,
    /// Patterns discovered via cross-episode consolidation.
    pub patterns: Vec<PatternRecord>,
    /// Hypotheses staged for future validation.
    pub staged_hypotheses: usize,
    /// Hypotheses promoted to permanent knowledge.
    pub promoted_hypotheses: usize,
    /// Confidence updates applied to existing knowledge.
    pub confidence_updates: usize,
    /// Emotional depotentiation summary.
    pub depotentiation: DepotentiationSummary,
}
```

Reports are stored as `dream-{unix_timestamp_ms}.json` in `.roko/dreams/`. The `DreamRunner::latest_report()` method retrieves the most recent one.

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [02-nrem-replay.md](02-nrem-replay.md) | Full detail on Mattar-Daw utility scoring, bidirectional replay, perturbed replay |
| [03-rem-imagination.md](03-rem-imagination.md) | Full detail on Pearl SCM, Boden creativity modes, emotional depotentiation |
| [04-consolidation-and-staging.md](04-consolidation-and-staging.md) | Full detail on staging buffer, confidence ladder, promotion mechanics |
| [05-dream-evolution.md](05-dream-evolution.md) | The fourth phase (EVOLUTION): memetic selection and strategy evolution |
| [12-sleep-time-compute.md](12-sleep-time-compute.md) | Computational economics and budget allocation |
| [13-scheduling-and-triggers.md](13-scheduling-and-triggers.md) | Detailed scheduling logic and trigger conditions |
