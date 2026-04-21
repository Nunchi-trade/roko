# Cross-System Integration

> **Layer**: All layers (L0–L4) — dreams integrate across the entire stack
>
> **Synapse Traits**: All six traits participate in dream integration at different points
>
> **Crate**: `roko-dreams` (primary), with integration points in `roko-neuro`, `roko-learn`, `roko-compose`, `roko-agent`, `roko-orchestrator`
>
> **Prerequisites**: [01-three-phase-cycle.md](01-three-phase-cycle.md), [04-consolidation-and-staging.md](04-consolidation-and-staging.md), [13-scheduling-and-triggers.md](13-scheduling-and-triggers.md)


> **Implementation**: Scaffold

---

## Overview

Dreams do not exist in isolation. The dream subsystem is a cognitive cross-cut — it touches every layer of the Roko architecture, injecting learned knowledge, emotional modulation, and creative insights into the agent's waking operation. In the two-fabric model, Dreams consume Substrate scans for completeness and Bus subscriptions for reactivity, so Delta-speed consolidation can wake on `substrate.engram.stored` instead of fixed polling. This document maps every integration point between the dream subsystem and other Roko subsystems, documenting the data flows, Bus channels, and trait implementations that connect dreams to the rest of the cognitive architecture. See also [tmp/refinements/09-phase-2-implications.md](../../tmp/refinements/09-phase-2-implications.md) and [01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md).

The five layers of the Roko architecture:
- **L0 Runtime**: Process lifecycle, events, supervision, adaptive clock
- **L1 Framework**: Backends, roles, tools, model routing, capabilities
- **L2 Scaffold**: Context engineering, prompts, enrichment
- **L3 Harness**: Gates, conductor, monitoring, interventions
- **L4 Orchestration**: DAGs, scheduling, multi-agent coordination

Dreams operate primarily at L0 (scheduling, process lifecycle) and L4 (orchestration, coordination), but inject results into L1 (model routing updates), L2 (context enrichment), and L3 (gate threshold adaptation).

### Reactive Input / Output Model

Dreams use both durable storage and live transport:

| Direction | Fabric | Channel | Purpose |
|-----------|--------|---------|---------|
| Substrate → Dreams | Substrate | Query / scan | Completeness pass over durable Engrams during consolidation |
| Bus → Dreams | Bus | `substrate.engram.stored` | Pulse-triggered Delta wakeup when new durable material lands |
| Dreams → Substrate | Substrate | `put()` | Persist consolidated Engrams and staged knowledge |
| Dreams → Bus | Bus | `engram.promoted`, `neuro.insight.promoted` | Broadcast promotion so Neuro and Compose can refresh without re-querying |

This is the key Phase 2+ simplification: the Delta loop becomes reactive without becoming a separate subsystem. The Bus notification wakes the cycle; the Substrate scan finishes the batch.

---

## Dreams × Neuro (Knowledge Store)

The Neuro subsystem (`roko-neuro`) is the agent's persistent knowledge base — episodes, insights, heuristics, warnings, causal links. Dreams are the primary mechanism for transforming raw episodic memory into durable semantic knowledge.

### Data Flow: Neuro → Dreams

| Data | Direction | Purpose |
|------|-----------|---------|
| Episodes since last dream | Neuro → NREM Replay | Raw material for replay prioritization via Mattar-Daw utility formula |
| Causal edges with confidence scores | Neuro → REM Imagination | Structural causal models for counterfactual generation via Pearl's SCM framework |
| Knowledge entries by tier | Neuro → Integration phase | Existing knowledge for deduplication and confidence updates |
| Embedding vectors | Neuro → HDC synthesis | Source material for counterfactual blending via XOR binding and majority bundling |

The `DreamEngine::schedule()` method reads episodes from the Neuro store to determine whether enough unprocessed material has accumulated to justify a dream cycle. The current scaffold in `roko-dreams/src/runner.rs` still reads from `EpisodeLogger`, but the intended two-fabric path pairs that completeness scan with Bus notifications from `substrate.engram.stored`:

```rust
let episodes_path = self.workdir.join(".roko").join("episodes.jsonl");
let episodes = block_on(EpisodeLogger::read_all_lossy(&episodes_path)).ok()?;
let last_report = load_latest_dream_report(&self.report_dir()).ok().flatten();
let cutoff = last_report
    .as_ref()
    .and_then(|report| report.processed_through.or(Some(report.started_at)));

let recent: Vec<&Episode> = episodes
    .iter()
    .filter(|episode| cutoff.is_none_or(|ts| episode.timestamp > ts))
    .collect();
```

### Data Flow: Dreams → Neuro

| Data | Direction | Purpose |
|------|-----------|---------|
| `InsightRecord` entries | NREM Replay → Neuro | Discovered patterns, cross-episode correlations, credit assignments |
| `CounterfactualHypothesis` entries | REM Imagination → Neuro | Hypotheses staged at confidence 0.20–0.30 for future validation |
| Confidence updates | Integration → Neuro | Promoted insights with updated confidence scores |
| Deprecated entries | Integration → Neuro | Entries with reduced confidence after dream re-evaluation |
| Playbook revisions | Integration → Neuro | New or updated strategy entries derived from dream consolidation |

The `TierProgression` system in `roko-neuro` classifies dream-generated insights into tiers based on accumulated evidence:

| Tier | Confidence Range | Typical Source |
|------|-----------------|----------------|
| T0 (Observation) | 0.00–0.19 | Raw episode fragments |
| T1 (Hypothesis) | 0.20–0.39 | REM counterfactual outputs, initial pattern detection |
| T2 (Emerging Pattern) | 0.40–0.59 | Cross-episode consolidation, confirmed replay patterns |
| T3 (Validated Insight) | 0.60–0.79 | Gate-validated dream discoveries |
| T4 (Established Knowledge) | 0.80–1.00 | Repeatedly confirmed, multi-dream validated |

Dreams primarily generate T1 and T2 entries. Promotion to T3+ requires waking validation — dreams propose, experience disposes. When consolidation finishes, Dreams emit the durable Engram and also publish promotion Pulses (`engram.promoted` for the generic promotion Pulse and `neuro.insight.promoted` for Neuro-specific cache refresh) so downstream consumers can react without polling.

### Bidirectional Feedback Loop

The dream-Neuro integration creates a continuous refinement loop:

```
Episodes accumulate in Neuro
  → Dreams replay and consolidate episodes
    → Insights written back to Neuro at T1-T2
      → Promotion Pulses (`engram.promoted`, `neuro.insight.promoted`) broadcast on the Bus
      → Waking experience validates or refutes
        → Validated insights promoted to T3+
          → T3+ insights influence future dream replay priority
            → Better dreams → better insights → cycle continues
```

This mirrors the Complementary Learning Systems (CLS) theory from neuroscience (McClelland et al., 1995): the hippocampal fast-learning system (episodic Neuro entries) and the neocortical slow-learning system (consolidated semantic knowledge) work together through sleep-mediated replay to build robust, generalized knowledge.

---

## Dreams × Daimon (Affect Engine)

The Daimon is the agent's affect engine — it maintains a PAD (Pleasure-Arousal-Dominance) emotional state vector that evolves continuously based on appraisal events. Dreams interact with the Daimon in both directions.

### Emotional Context for Dreams

The agent's emotional state at dream onset influences dream processing:

| PAD Dimension | Dream Effect |
|--------------|-------------|
| High Arousal (A > 0.7) | More REM time allocated for emotional depotentiation (Walker & van der Helm, 2009) |
| Negative Pleasure (P < -0.3) | Replay prioritizes failure episodes for credit reassignment |
| Low Dominance (D < -0.3) | Threat simulation emphasis in REM phase (Revonsuo, 2000) |
| High Pleasure (P > 0.5) | Exploratory creativity mode favored in REM |

### Emotional Depotentiation

The most important dream-Daimon interaction is emotional depotentiation during the REM phase. High-arousal episodes from waking experience are replayed with reduced emotional charge:

```
pre_dream_arousal = daimon.pad().arousal
// ... REM processing ...
post_dream_arousal = pre_dream_arousal - depotentiation_delta

// Typical depotentiation: 0.3-0.5 per dream cycle
depotentiation_delta = 0.3 + (pre_dream_arousal - 0.5).max(0.0) * 0.4
```

This is a direct implementation of Walker & van der Helm's "overnight therapy" finding: REM sleep depotentiates emotional charge from memories while preserving their informational content. The agent remembers *what happened* but no longer *feels it as strongly*.

### Daimon State Updates from Dreams

After a dream cycle completes, the Daimon receives state updates:

| Update | Source | Effect |
|--------|--------|--------|
| Arousal reduction | REM depotentiation | PAD arousal component decreases by 0.3–0.5 |
| Pleasure adjustment | Dream discoveries | Positive discoveries increase P; failure pattern recognition may decrease P |
| Dominance recalibration | Threat simulation | Successful threat rehearsal increases D; unresolved threats decrease D |

---

## Dreams × Learning Subsystem (roko-learn)

The learning subsystem (`roko-learn`) manages episodes, playbooks, model routing, prompt experiments, and efficiency tracking. Dreams integrate deeply with several learning components.

### Episode Logger Integration

The `EpisodeLogger` records agent turns — each action, its context, the gate results, and the outcome. Dreams consume these episodes as raw material:

```rust
// From roko-dreams/src/runner.rs
let episodes = Arc::new(EpisodeLogger::new(
    self.workdir.join(".roko").join("episodes.jsonl"),
));
```

The dream cycle reads episodes, processes them through NREM replay and REM imagination, and produces a `DreamCycleReport` that includes references to which episodes were processed. The `processed_through` timestamp in the report allows subsequent dream cycles to skip already-processed episodes.

### Playbook Store Integration

Dreams can generate playbook revisions — updates to the agent's executable strategy document. The `PlaybookStore` from `roko-learn` is passed directly to the `DreamCycle`:

```rust
let playbooks_root = self.workdir.join(".roko").join("learn").join("playbooks");
let playbooks = Arc::new(PlaybookStore::new(playbooks_root));
let mut cycle = DreamCycle::new(episodes, knowledge, playbooks, dispatcher);
```

Dream-generated playbook revisions enter at confidence 0.20–0.30 and must be validated by waking experience before promotion to the active playbook.

### Pattern Discovery Integration

The `PatternMiner` and `CrossEpisodeConsolidator` from `roko-learn/src/pattern_discovery.rs` are key components used during NREM replay:

| Component | Role in Dreams |
|-----------|---------------|
| `PatternMiner` | Trigram mining across episodes with FNV-1a hashing to discover recurring action patterns |
| `CrossEpisodeConsolidator` | K-medoids clustering over HDC episode vectors to identify cross-episode meta-patterns |
| `EpisodeView` trait | Abstracts episode access for pattern mining (`actions()`, `succeeded()`) |

The `CrossEpisodeConsolidator` uses the K-medoids implementation from `roko-learn/src/hdc_clustering.rs` to cluster episode vectors and discover structural similarities across experiences:

```rust
pub fn consolidate(&self, episode_vectors: &[(usize, HdcVector)]) -> Vec<CrossEpisodeMetaPattern> {
    let config = KMedoidsConfig {
        k: self.target_clusters,
        max_iterations: self.max_iterations,
    };
    let result = k_medoids(&vectors, &config);
    // ... convert clusters to meta-patterns with coherence scores ...
}
```

### CascadeRouter Integration

Dream consolidation can update the `CascadeRouter` — the model routing system that selects which LLM backend to use for different task types. If dreams discover that certain model configurations produce better outcomes for certain task categories, the router weights are updated:

| Dream Output | Router Update |
|-------------|--------------|
| Model A outperforms B for task type X (from replay credit assignment) | Increase Model A weight for type X |
| Consistent gate failures with Model C for type Y | Decrease Model C weight for type Y |
| Novel strategy discovery with Model D at high temperature | Note Model D effectiveness for creative tasks |

### Prompt Experiment Integration

The `ExperimentStore` tracks A/B prompt experiments. Dreams can analyze experiment results during consolidation, identifying which prompt variants produced better outcomes across multiple episodes rather than just individual trials.

---

## Dreams × Context Engineering (roko-compose)

The context engineering subsystem (`roko-compose`) assembles prompts for agent inference using the `SystemPromptBuilder`. Dreams integrate with context engineering in two ways, and it listens for promotion Pulses so it can refresh cached enrichment instead of re-querying after every consolidation cycle.

### Dream Context Injection

When an agent's context is assembled for a waking task, recent dream insights can be injected as context enrichment:

| Context Layer | Dream Contribution |
|--------------|--------------------|
| Knowledge context | Recently promoted dream insights (T3+) added to knowledge section after `neuro.insight.promoted` |
| Emotional context | Post-dream PAD state (with depotentiation applied) |
| Strategy context | Dream-generated playbook revisions that have been validated |
| Warning context | Threat simulation results from dream REM phase |

### Dream Prompt Assembly

The dream consolidation process itself requires prompt assembly. The `DreamCycle` uses the agent dispatcher to run LLM inference during consolidation:

```rust
let dispatcher: Arc<dyn AgentDispatcher> =
    Arc::new(self.config.agent.build_agent(&self.workdir));
let mut cycle = DreamCycle::new(episodes, knowledge, playbooks, dispatcher);
cycle.run().await
```

The `DreamAgentConfig` specifies the model and parameters for dream inference:

```rust
pub struct DreamAgentConfig {
    pub command: String,       // "claude" or custom command
    pub args: Vec<String>,
    pub model: Option<String>, // e.g., "claude-haiku-4-5-20251001"
    pub bare_mode: bool,       // true for dreams (no tool use)
    pub effort: String,        // "low" for dreams (cost-effective)
    pub timeout_ms: u64,       // 120000ms default
    pub env: Vec<(String, String)>,
}
```

Dreams typically use a cheaper, faster model (e.g., Haiku) with low effort settings and bare mode (no tool use), since dream consolidation is a reasoning task that doesn't require tool execution. The Compose path can consume `neuro.insight.promoted` Pulses to refresh prompt enrichment caches without waiting for another substrate scan.

---

## Dreams × Gate Pipeline (roko-gate)

The gate pipeline validates agent outputs through a multi-rung verification process. Dreams interact with gates in two ways.

### Gate Results Feed Dreams

Gate results from waking tasks are recorded as part of episode data. When dreams replay these episodes, gate success/failure patterns are critical inputs:

| Gate Signal | Dream Use |
|------------|-----------|
| Compile gate failures | Indicate code quality issues — replay with emphasis on error patterns |
| Test gate failures | Indicate logic errors — replay with emphasis on reasoning chains |
| Clippy warnings | Indicate style/correctness patterns — consolidate into coding heuristics |
| Diff gate anomalies | Indicate unexpected changes — replay to understand scope creep |

### Dreams Update Gate Thresholds

The adaptive gate threshold system uses EMA (Exponential Moving Average) per rung, stored in `.roko/learn/gate-thresholds.json`. Dream consolidation can propose threshold adjustments based on cross-episode analysis:

- If dreams discover that a particular gate rung consistently produces false positives (blocks good work), the threshold may be relaxed
- If dreams discover that a particular gate rung consistently misses real issues, the threshold may be tightened
- Threshold proposals from dreams enter at low confidence and require waking validation

---

## Dreams × Agent Mesh (Coordination)

In multi-agent deployments, Dreams interact with the Mesh as a pair of ordinary fabrics: `MeshSubstrate` replicates durable dream outputs, and `MeshBus` publishes the live Pulses that let peers react without polling.

### Knowledge Sharing via Mesh

Dream-generated insights can be shared across the mesh:

| Sharing Direction | Mechanism | Content |
|------------------|-----------|---------|
| Agent → Mesh | `MeshSubstrate.put()` plus `MeshBus.publish()` | High-confidence insights (T3+), validated heuristics, and promotion Pulses |
| Mesh → Agent | `MeshBus` subscription plus `MeshSubstrate` query | Peer insights for potential integration (at reduced confidence, ×0.85 per hop) |

The confidence reduction on shared knowledge follows the Weismann barrier principle: inherited knowledge is always less trusted than self-discovered knowledge. An insight that is T4 (0.90 confidence) in the originating agent enters the receiving agent at T3 (0.90 × 0.85 = 0.765).

### Collective Dream Patterns

When multiple agents dream about similar episodes (because they share overlapping task domains), the mesh can identify collective patterns:

1. Agent A dreams and discovers pattern P₁
2. Agent B independently dreams and discovers pattern P₂
3. Mesh detects P₁ and P₂ are semantically similar (via HDC cosine similarity)
4. Both agents receive a "collective confirmation" signal, boosting confidence in the shared pattern

This is analogous to the Grossman-Stiglitz information paradox (Grossman & Stiglitz, "On the Impossibility of Informationally Efficient Markets," *AER*, 1980): individually discovered information has value precisely because it's not universally known. The mesh creates a marketplace where dream insights have economic value — agents that dream more effectively contribute more to collective intelligence. In REF09 terms, that marketplace is pub/sub topology over `MeshBus` plus durable replication in `MeshSubstrate`, not a separate coordination mechanism.

### Pheromone Field Integration

The mesh's pheromone field (a diffusing scalar field encoding collective emotional state) interacts with dream scheduling:

| Pheromone Signal | Dream Effect |
|-----------------|-------------|
| High threat pheromone (> 0.7) on `mesh.pheromone.deposited` | Prioritize threat simulation in REM phase |
| Low activity pheromone | Extend NREM consolidation (more time for thorough replay) |
| Knowledge pheromone spike | Prioritize integration of newly received mesh insights from `MeshSubstrate` |

---

## Dreams × Orchestrator (Plan Execution)

The orchestrator (`roko-orchestrator`, wired through `roko-cli/src/orchestrate.rs`) manages plan DAGs — directed acyclic graphs of tasks that the agent executes. Dreams interact with the orchestrator for scheduling and feedback.

### Scheduling Coordination

The dream scheduler coordinates with the orchestrator to find idle windows:

```
Orchestrator                     Dream Scheduler
    |                                  |
    |-- Task completes --------------->|
    |                                  |-- Check idle threshold
    |                                  |-- Check episode count
    |                                  |
    |                                  |-- Threshold met → DREAM FIRES
    |                                  |
    |<-- Dream complete, resume -------|
    |                                  |
    |-- Next task starts ------------->|
```

The orchestrator calls `dream_runner.schedule_next()` after each task completion. Dreams never interrupt active tasks.

### Plan Feedback Loop

Dream-generated insights can feed back into the plan generator:

| Dream Output | Plan Effect |
|-------------|-------------|
| Recurring failure pattern across tasks | Flag similar pending tasks for re-planning |
| New heuristic for task type X | Enrich context for future tasks of type X |
| Discovered dependency between task outcomes | Suggest DAG edge additions |

This feedback loop is the foundation for automatic re-planning: when dreams discover that a plan's assumptions are wrong, the plan generator can produce corrected plans.

---

## Dreams × Hypnagogia Engine

The Hypnagogia engine (see [07-hypnagogia-engine.md](07-hypnagogia-engine.md)) is a subsystem within dreams that operates during the transition between waking and sleep states. It integrates with the main dream cycle as a creativity amplifier.

### Data Flow

```
Waking State
  → Hypnagogia (transition phase)
    → Thalamic Gate: anti-correlated HDC retrieval from Neuro
    → Executive Loosener: elevated temperature (T=1.3, top_p=0.95)
    → Dali Interrupt: 50-100 token associative fragments
    → Homuncular Observer: structured scoring (T=0.4)
  → Dream Cycle (NREM → REM → Integration)
```

Hypnagogic fragments that score above the Homuncular Observer's relevance threshold are fed into the REM phase as seed material for counterfactual generation. This creates a pipeline from unstructured association to structured hypothesis.

### Stochastic Resonance

The Hypnagogia engine uses controlled noise injection (Gammaitoni et al., "Stochastic Resonance," *Reviews of Modern Physics*, 1998) to improve signal detection. The noise level is calibrated so that weak but genuine patterns in the agent's experience are amplified rather than drowned:

- Too little noise: only obvious patterns detected (exploitation trap)
- Too much noise: signal lost in randomness (exploration waste)
- Optimal noise: weak patterns amplified, novel connections surfaced

---

## Dreams × Runtime Process Supervisor

The runtime process supervisor manages agent process lifecycle. Dream cycles interact with that supervisor for resource management:

| Supervisor Function | Dream Integration |
|--------------------|-------------------|
| Process tracking | Dream agent processes tracked and cleaned up on completion |
| Cancellation tokens | Dream cycles can be cancelled if a higher-priority task arrives |
| Bus | Dream completion and promotion Pulses published for other subsystems to observe |
| Resource limits | Dream inference constrained by supervisor memory/CPU limits |

---

## Integration Summary Table

| Subsystem | Layer | Direction | Key Data |
|-----------|-------|-----------|----------|
| **Neuro** (Knowledge) | L1 | Bidirectional | Substrate scans + `substrate.engram.stored`; insights → Neuro; promotion Pulses → refresh |
| **Daimon** (Affect) | L1 | Bidirectional | PAD context → dreams; depotentiation → Daimon |
| **Learn** (Episodes) | L1 | Neuro-mediated | Episodes, playbooks, patterns, routing |
| **Compose** (Context) | L2 | Dreams → Context | Post-dream insights injected into context; `neuro.insight.promoted` refreshes enrichment |
| **Gate** (Validation) | L3 | Bidirectional | Gate results → dreams; threshold updates → gates |
| **Mesh** (Coordination) | L4 | Bidirectional | Dream insights → `MeshSubstrate` + `MeshBus`; peer insights → Dreams via Bus subscription + durable query |
| **Orchestrator** (Plans) | L4 | Bidirectional | Idle windows + Bus wakeups → dreams; feedback → plans |
| **Hypnagogia** (Creativity) | L0/L1 | Unidirectional | Hypnagogic fragments → dream seeds |
| **Supervisor** (Process) | L0 | Supervisor → Dreams | Lifecycle, cancellation, resource limits |
| **Oneirography** (Art) | L1/L2 | Dreams → Art | DreamCycleReport → image generation |

---

## Event Flow Diagram

The complete event flow for a dream cycle touching all subsystems:

```
1. Orchestrator detects idle gap after task completion
2. Calls `DreamRunner::schedule_next()` and remains subscribed to `substrate.engram.stored`
3. DreamRunner reads episodes from `EpisodeLogger` (roko-learn)
4. DreamRunner scans the Substrate for durable Engrams that have not yet been consolidated
5. If the batch threshold is met, or enough `substrate.engram.stored` Pulses have arrived, the dream cycle fires

6. NREM Phase:
   a. PatternMiner discovers trigram patterns (roko-learn)
   b. CrossEpisodeConsolidator clusters via K-medoids (roko-learn)
   c. Mattar-Daw utility scores prioritize episodes
   d. Replay generates InsightRecords

7. REM Phase:
   a. Daimon provides PAD context
   b. Neuro provides causal graph for SCM counterfactuals
   c. Hypnagogia fragments seed creative generation
   d. Counterfactual hypotheses generated
   e. Emotional depotentiation applied

8. Integration Phase:
   a. Insights staged in Neuro at T1-T2 confidence
   b. Hypotheses staged at 0.20-0.30 confidence
   c. Playbook revisions written to PlaybookStore
   d. Gate threshold proposals generated
   e. CascadeRouter updates proposed
   f. `engram.promoted` and `neuro.insight.promoted` Pulses published for downstream refresh

9. Post-Dream:
   a. DreamCycleReport persisted to .roko/dreams/
   b. Daimon PAD updated (arousal reduced)
   c. Orchestrator notified → resumes task execution
   d. High-confidence insights shared to Mesh (if connected)
   e. Oneirography generates dream image (if configured)
```

---

## Dreams x Fleet Coordination

When multiple agents operate as a fleet (the Roko term for a coordinated group of agents, formerly "clade"), dream coordination becomes important. Without coordination, fleet members may all dream simultaneously — wasting compute on redundant consolidation when staggered cycles would allow one agent's dream insights to benefit another's waking work before that agent dreams.

```rust
/// Fleet-level dream coordination.
pub struct FleetDreamCoordinator {
    /// Whether to stagger dream cycles across fleet members.
    pub stagger_cycles: bool,             // default: true
    /// Minimum stagger interval between any two agents dreaming (minutes).
    pub min_stagger_mins: u64,            // default: 10, range: 5-60
    /// Whether to aggregate fleet dream insights for collective trend analysis.
    pub aggregate_insights: bool,         // default: true
    /// Collective insight confidence boost when N+ agents independently discover same pattern.
    pub collective_confirmation_boost: f64, // default: 0.15, range: 0.05-0.30
    /// Minimum agents confirming a pattern for collective boost.
    pub min_confirming_agents: usize,     // default: 2, range: 2-5
}
```

Fleet dream coordination operates through the Agent Mesh (formerly "Styx"):

- **Staggered scheduling**: The fleet coordinator assigns dream slots so that no two agents dream simultaneously. This ensures continuous waking coverage and allows dream insights from early dreamers to propagate to later dreamers' waking contexts before they sleep.
- **Insight aggregation**: When `aggregate_insights` is enabled, the coordinator collects dream insights from all fleet members and performs collective trend analysis. Patterns discovered independently by multiple agents receive a confidence boost (`collective_confirmation_boost`), reflecting the epistemic value of independent confirmation.
- **Collective confirmation**: If `min_confirming_agents` or more fleet members independently discover the same pattern (measured by HDC cosine similarity > 0.85), the pattern receives the collective confirmation boost. This is a fleet-level implementation of the Grossman-Stiglitz information aggregation principle described in the Mesh integration section above.

---

## Dreams x Configuration System

Dream configuration flows from `roko.toml` through the standard Roko configuration system. The complete dream configuration reference:

```toml
# Complete dream configuration reference
[dreams]
auto_dream = true
idle_threshold_mins = 15
min_episodes_for_dream = 5
scheduled_interval_hours = 4
budget_fraction = 0.15
intensive_threshold = 50
intensive_low_water = 10
batch_size = 10

[dreams.agent]
command = "claude"
model = "claude-haiku-4-5-20251001"
bare_mode = true
effort = "low"
timeout_ms = 120000

[dreams.privacy]
nrem_provider = "local"
rem_provider = "api"
hypnagogia_provider = "api"

[dreams.sharing]
mode = "selective"
confidence_threshold = 0.75
novelty_threshold = 0.60
evaporation_rate = 0.05
hop_decay = 0.85
max_hops = 3

[dreams.nightmare]
enable_detection = true
classifier_tier = "T2"
capability_delta_threshold = 0.50
cooldown_cycles = 3
```

Configuration sections explained:

| Section | Purpose |
|---------|---------|
| `[dreams]` | Top-level scheduling and resource allocation. `auto_dream` enables idle-triggered dreaming. `budget_fraction` caps dream compute at 15% of total agent budget. `intensive_threshold` / `intensive_low_water` control intensive consolidation mode (see [13-scheduling-and-triggers.md](13-scheduling-and-triggers.md)). |
| `[dreams.agent]` | Agent backend configuration for dream inference. Dreams typically use a cheaper model (Haiku) with `bare_mode = true` (no tool use) and `effort = "low"` for cost efficiency. |
| `[dreams.privacy]` | Provider routing per dream phase. `"local"` runs inference locally (no data leaves the machine); `"api"` uses the configured API backend. NREM replay defaults to local since it processes raw episode data. |
| `[dreams.sharing]` | Controls how dream insights propagate through the Agent Mesh. `mode` can be `"none"`, `"selective"` (share only high-confidence, high-novelty insights), or `"all"`. `hop_decay` (0.85) reduces confidence by 15% per mesh hop. `evaporation_rate` (0.05) reduces shared insight confidence over time. |
| `[dreams.nightmare]` | Nightmare detection and containment configuration. See [17-advanced-dream-concepts.md](17-advanced-dream-concepts.md) for the nightmare detection system. `capability_delta_threshold` is the maximum acceptable capability regression before a dream output is flagged as a nightmare. |

The configuration is loaded by `DreamLoopConfig::from_roko_toml()` and propagated to the `DreamRunner` at initialization. Runtime overrides are possible via the `roko config set dreams.<key> <value>` CLI command.

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [01-three-phase-cycle.md](01-three-phase-cycle.md) | Dream cycle structure that integration points feed into |
| [02-nrem-replay.md](02-nrem-replay.md) | NREM replay consuming episodes from EpisodeLogger |
| [03-rem-imagination.md](03-rem-imagination.md) | REM imagination using Neuro causal graphs |
| [04-consolidation-and-staging.md](04-consolidation-and-staging.md) | Integration phase writing results back to Neuro |
| [07-hypnagogia-engine.md](07-hypnagogia-engine.md) | Hypnagogia fragments feeding dream seeds |
| [12-sleep-time-compute.md](12-sleep-time-compute.md) | Compute budget constraining dream inference |
| [13-scheduling-and-triggers.md](13-scheduling-and-triggers.md) | Scheduling coordinated with orchestrator |
| [14-oneirography.md](14-oneirography.md) | Art generation consuming DreamCycleReport |
| [01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md) | Canonical Engram, Pulse, Bus, and Topic terminology |
| [tmp/refinements/09-phase-2-implications.md](../../tmp/refinements/09-phase-2-implications.md) | Phase-2 two-fabric implications for Dreams, Mesh, Chain, and Heartbeat |
