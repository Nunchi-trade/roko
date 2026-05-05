# Dream Cycle as Loop

> Depth for [06-MEMORY.md](../../unified/06-MEMORY.md). How offline knowledge consolidation emerges as a Loop Graph of phase Cells with Trigger-based scheduling and predict-publish-correct feedback.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal/Pulse, demurrage, Kind system, HDC fingerprints), [02-CELL](../../unified/02-CELL.md) (9 protocols, predict-publish-correct, CalibrationTable), [03-GRAPH](../../unified/03-GRAPH.md) (Graph, Loop specialization, feedback edges), [04-EXECUTION](../../unified/04-EXECUTION.md) (Engine, Hot Flow, budget enforcement), [06-MEMORY](../../unified/06-MEMORY.md) (Store, demurrage, tier progression, staging buffer), [07-LEARNING](../../unified/07-LEARNING.md) (L3 knowledge consolidation, predict-publish-correct), [13-TRIGGERS](../../unified/13-TRIGGERS.md) (Trigger protocol, TriggerBinding, push-based scheduling)

**Existing code**: `crates/roko-dreams/src/` (cycle.rs, runner.rs, staging.rs, replay.rs, imagination.rs, hypnagogia.rs, phase2/)

---

## 1. The Dream Problem, Precisely Stated

The existing `DreamCycle` in `crates/roko-dreams/src/cycle.rs` is a 900-line monolithic function. It runs NREM, REM, and Integration as sequential method calls inside a single async block. The three phases cannot be composed, replaced, or extended independently. The scheduling logic in `runner.rs` is a separate state machine with its own idle timers and cron parsing that does not use the Trigger system. The staging buffer in `staging.rs` is an isolated struct with its own persistence, disconnected from the Store protocol.

Three consequences:

1. **No scheduling.** `DreamRunner` exists but nothing calls it at runtime. The Mori lesson applies directly: dreams exist but no trigger calls them. The idle-time trigger, the cron trigger, and the episode-count trigger are all implemented as methods on `DreamSchedulePolicy` but never wired into the event loop.

2. **No composability.** Adding a new consolidation strategy (say, a specialized code-pattern extractor or a cross-domain analogy finder) requires modifying the monolithic cycle function. There is no way to insert a Cell between NREM and REM, or to run an alternative REM implementation for a specific domain.

3. **No feedback.** The `DreamQualityDashboard` is described in source docs but not implemented. Phase budget allocation is static (Hypnagogia 10%, NREM 30%, REM 50%, Integration 0%, Evolution 10%). There is no mechanism for the system to learn which phase produces the best waking improvement and reallocate accordingly.

The redesign addresses all three by expressing the dream cycle as a Loop Graph of pluggable phase Cells, scheduling as a Trigger Cell, the staging buffer as a Store partition, and quality monitoring as a Lens that feeds back into budget allocation.

---

## 2. The Dream Cycle IS a Loop Graph

A Loop is a Graph with a feedback edge from output back to input (see [03-GRAPH.md](../../unified/03-GRAPH.md) S2 `NodeKind::Loop`). The dream cycle is a textbook case: Integration's output (what was promoted, what was rejected, what needs more evidence) feeds back to NREM's input (which episodes to replay next, which clusters to prioritize).

### 2.1 Graph Definition

```toml
[graph]
name = "dream-consolidation"
version = "1.0.0"
loop = true
max_iterations = 10       # max cycles per dream session
convergence = "staging_stable"

[[nodes]]
id = "nrem"
label = "NREM Replay"
cell = "roko:dream-nrem"
protocol = ["Store", "Score"]

[[nodes]]
id = "rem"
label = "REM Imagination"
cell = "roko:dream-rem"
protocol = ["Compose", "Route"]

[[nodes]]
id = "integrate"
label = "Integration"
cell = "roko:dream-integrate"
protocol = ["Store", "Verify"]

[[nodes]]
id = "budget-gate"
label = "Budget Check"
cell = "roko:dream-budget-gate"
protocol = ["Verify"]

[[edges]]
from = "nrem"
to = "rem"
type = "data"

[[edges]]
from = "rem"
to = "integrate"
type = "data"

[[edges]]
from = "integrate"
to = "budget-gate"
type = "data"

# The feedback edge: Integration output feeds back to NREM input
[[edges]]
from = "budget-gate"
to = "nrem"
type = "feedback"
condition = "!budget_exhausted && !staging_stable"
```

### 2.2 Convergence Condition

The loop terminates when any of:
- The staging buffer is stable (no entries changed stage in the last iteration)
- The budget is exhausted (any axis: tokens, cost, duration)
- `max_iterations` reached
- The cancellation token fires (external shutdown)

```rust
/// Convergence check for the dream Loop.
/// Returns true when further iterations would produce no new promotions.
fn staging_stable(prev: &DreamIterationOutput, curr: &DreamIterationOutput) -> bool {
    curr.entries_advanced == 0
        && curr.entries_promoted == 0
        && curr.new_candidates == 0
}
```

This is the same convergence pattern used by L1 parameter tuning (see [07-LEARNING.md](../../unified/07-LEARNING.md) S3): the loop runs until the delta between iterations drops below a threshold.

### 2.3 What Flows Between Cells

Each edge carries Signals. The types are:

```rust
/// NREM output -> REM input.
/// Clustered episode summaries with replay utility scores.
pub struct NremOutput {
    pub clusters: Vec<EpisodeCluster>,
    pub insights: Vec<Signal>,           // Kind::Insight, Transient tier
    pub replay_stats: ReplayStats,
    pub relabeled_episodes: Vec<Signal>,  // hindsight-relabeled failures
}

/// REM output -> Integration input.
/// Counterfactual hypotheses and strategy fragments.
pub struct RemOutput {
    pub hypotheses: Vec<Signal>,         // Kind::StrategyFragment
    pub counterfactuals: Vec<Signal>,    // Kind::Insight with counterfactual lineage
    pub threat_warnings: Vec<Signal>,    // Kind::Warning, short TTL
    pub imagination_stats: ImaginationStats,
}

/// Integration output -> feedback to NREM.
/// What was promoted, what needs more evidence, what to prioritize next.
pub struct IntegrationOutput {
    pub promoted: Vec<Signal>,           // promoted to higher tier
    pub needs_evidence: Vec<SignalRef>,  // replay these episodes again
    pub rejected: Vec<SignalRef>,        // contradicted or redundant
    pub staging_snapshot: StagingSnapshot,
    pub quality: DreamQualityMetrics,
}
```

The feedback edge carries `IntegrationOutput`. The `needs_evidence` field tells NREM which episode clusters to prioritize in the next iteration. This is the structural reification of the biological principle: NREM replay is biased toward memories that integration found ambiguous.

---

## 3. Each Phase IS a Cell

### 3.1 NREM Replay Cell (Store + Score)

NREM retrieves episodes from Store (Store protocol) and scores them for replay priority (Score protocol). The existing `select_replay_episodes` and `ReplayUtility` from `replay.rs` become the Score implementation.

```rust
/// NREM Replay: retrieve high-surprise episodes, cluster by HDC similarity,
/// extract patterns into Insight Signals.
///
/// Protocols: Store (query episodes), Score (Mattar-Daw replay utility)
pub struct NremReplayCell {
    id: CellId,
    replay_policy: DreamReplayPolicy,
    mattar_daw: MattarDawConfig,
    max_clusters: usize,
    hdc_similarity_threshold: f32,
}

impl Cell for NremReplayCell {
    fn name(&self) -> &str { "dream-nrem" }
    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::Store, ProtocolId::Score]
    }

    fn estimated_cost(&self) -> Option<Cost> {
        // NREM uses T0 (Haiku/Fast): ~$0.001 per episode
        Some(Cost::per_unit(0.001, "episode"))
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // On first iteration, input is the trigger payload (episode list).
        // On subsequent iterations, input is IntegrationOutput (feedback).
        let priority_refs = extract_priority_refs(&input);

        // 1. Query episodes from Store
        let episodes = ctx.store.query(StoreQuery {
            kind: Some(Kind::Episode),
            min_balance: Some(0.05),  // above cold threshold
            limit: self.replay_policy.max_episodes,
            ..Default::default()
        }).await?;

        // 2. Score each episode with Mattar-Daw utility
        let mut scored: Vec<(Signal, ReplayUtility)> = episodes
            .iter()
            .map(|ep| {
                let novelty = compute_novelty(ep, &episodes);
                let recency = compute_recency(ep);
                let util = ReplayUtility::compute(ep, novelty, recency, &self.mattar_daw);
                (ep.clone(), util)
            })
            .collect();

        // 2a. Boost priority for episodes requested by Integration feedback
        for (ep, util) in &mut scored {
            if priority_refs.contains(&ep.id()) {
                util.utility *= 2.0;  // double priority for feedback-requested replays
            }
        }

        // 3. Select top-k and cluster by HDC similarity
        scored.sort_by(|a, b| b.1.utility.total_cmp(&a.1.utility));
        let selected: Vec<Signal> = scored
            .into_iter()
            .take(self.replay_policy.max_episodes)
            .map(|(ep, _)| ep)
            .collect();

        let clusters = cluster_by_hdc(&selected, self.hdc_similarity_threshold);

        // 4. Extract patterns from clusters -> Insight Signals
        let mut insights = Vec::new();
        for cluster in &clusters {
            if cluster.episodes.len() >= 3 {
                let insight = distill_cluster_to_insight(cluster)?;
                insights.push(insight);
            }
        }

        // 5. Hindsight relabeling: decompose failures into achieved sub-goals
        let relabeled = hindsight_relabel(&selected)?;

        // 6. Predict-publish-correct: predict how many insights will promote
        let prediction = ctx.calibration.predict(
            "nrem.promotion_rate",
            insights.len() as f64 / selected.len().max(1) as f64,
        );
        ctx.bus.publish(Pulse::prediction(
            topic!("dream.nrem.promotion_rate"),
            prediction,
        )).await?;

        // 7. Pack output
        Ok(NremOutput { clusters, insights, replay_stats: stats, relabeled }.into_signals())
    }
}
```

**Why Store + Score**: NREM does two things -- it retrieves (Store protocol: `query` for episodes) and it evaluates (Score protocol: Mattar-Daw utility scoring). These are the two protocols that map cleanly to what NREM actually does. It does not compose new content (that is REM's job) and it does not verify (that is Integration's job).

### 3.2 REM Imagination Cell (Compose + Route)

REM creates new content from existing patterns (Compose protocol) and selects which creativity mode to apply (Route protocol). The existing `imagination.rs` functions become the Compose implementation. The three creativity modes (Combinational, Exploratory, Transformational) become Route targets.

```rust
/// REM Imagination: generate counterfactuals, hypotheses, and threat warnings.
///
/// Protocols: Compose (synthesize new Signals from clusters),
///            Route (select creativity mode per cluster)
pub struct RemImaginationCell {
    id: CellId,
    imagination_modes: Vec<ImaginationMode>,
    max_counterfactuals_per_cluster: usize,
    threat_rehearsal_enabled: bool,
    trust_region_radius: f64,
}

impl Cell for RemImaginationCell {
    fn name(&self) -> &str { "dream-rem" }
    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::Compose, ProtocolId::Route]
    }

    fn estimated_cost(&self) -> Option<Cost> {
        // REM uses T1 (Sonnet/Standard): ~$0.01 per counterfactual
        Some(Cost::per_unit(0.01, "counterfactual"))
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let nrem_output = NremOutput::from_signals(&input)?;

        let mut hypotheses = Vec::new();
        let mut counterfactuals = Vec::new();
        let mut threat_warnings = Vec::new();

        for cluster in &nrem_output.clusters {
            // Route: select creativity mode based on cluster properties
            let mode = self.select_mode(cluster, ctx).await?;

            match mode {
                ImaginationMode::Combinational => {
                    // Merge patterns from two episodes within cluster
                    let cf = self.synthesize_combinational(cluster, ctx).await?;
                    counterfactuals.extend(cf);
                }
                ImaginationMode::Exploratory => {
                    // Extend pattern into a neighboring domain
                    let hyp = self.synthesize_exploratory(cluster, ctx).await?;
                    hypotheses.extend(hyp);
                }
                ImaginationMode::Transformational => {
                    // Invert an assumption from a successful pattern
                    let hyp = self.synthesize_transformational(cluster, ctx).await?;
                    hypotheses.extend(hyp);
                }
            }
        }

        // Threat rehearsal sub-phase
        if self.threat_rehearsal_enabled {
            let threats = enumerate_threats(&nrem_output.insights);
            for threat in threats {
                let warning = Signal::new(Kind::Warning, threat.into_body());
                warning.set_ttl(Duration::from_secs(3600)); // 1h TTL
                ctx.bus.publish(Pulse::from_signal(
                    topic!("dream.threat.warning"),
                    &warning,
                )).await?;
                threat_warnings.push(warning);
            }
        }

        // Predict-publish-correct: predict hypothesis diversity
        let diversity = hdc_diversity(&hypotheses);
        ctx.calibration.predict("rem.hypothesis_diversity", diversity);
        ctx.bus.publish(Pulse::prediction(
            topic!("dream.rem.hypothesis_diversity"),
            diversity,
        )).await?;

        Ok(RemOutput { hypotheses, counterfactuals, threat_warnings, .. }.into_signals())
    }

    async fn select_mode(
        &self,
        cluster: &EpisodeCluster,
        ctx: &CellContext,
    ) -> Result<ImaginationMode, CellError> {
        // Route protocol: use cluster properties to select creativity mode.
        // High-failure clusters -> Transformational (invert failing assumption)
        // Cross-domain clusters -> Combinational (merge diverse patterns)
        // Single-domain clusters -> Exploratory (extend into neighbors)
        let failure_rate = cluster.failure_rate();
        let domain_count = cluster.distinct_domains();

        if failure_rate > 0.6 {
            Ok(ImaginationMode::Transformational)
        } else if domain_count > 1 {
            Ok(ImaginationMode::Combinational)
        } else {
            Ok(ImaginationMode::Exploratory)
        }
    }
}
```

**Why Compose + Route**: REM creates things that did not exist before (Compose: assemble new Signals from components) and it chooses how to create them (Route: select among alternative creativity strategies). This is the dual of NREM: where NREM reads and evaluates existing knowledge, REM writes and directs new knowledge.

### 3.3 Integration Cell (Store + Verify)

Integration persists validated outputs (Store protocol) and validates them against existing knowledge (Verify protocol). The existing `StagingBuffer` becomes the mechanism, but now operating on a Store partition rather than an isolated struct.

```rust
/// Integration: validate dream outputs, promote through staging, persist to Store.
///
/// Protocols: Store (persist promoted Signals), Verify (validate against
///            existing knowledge for redundancy and contradiction)
pub struct IntegrationCell {
    id: CellId,
    redundancy_threshold: f32,        // HDC similarity for dedup (0.90)
    min_confirmations_for_d2: usize,  // 5 for Heuristic promotion
    max_playbooks_per_cycle: usize,   // 12
}

impl Cell for IntegrationCell {
    fn name(&self) -> &str { "dream-integrate" }
    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::Store, ProtocolId::Verify]
    }

    fn estimated_cost(&self) -> Option<Cost> {
        // Integration is pure computation: no LLM calls. $0.
        Some(Cost::zero())
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let rem_output = RemOutput::from_signals(&input)?;
        let staging = ctx.store.partition("dream:staging");

        // --- D1: Episodes to Insights ---
        // Add new candidates to staging at Raw stage
        let mut new_candidates = 0;
        for hypothesis in &rem_output.hypotheses {
            staging.put(hypothesis.clone().with_metadata(
                "staging_stage", "raw",
            )).await?;
            new_candidates += 1;
        }
        for counterfactual in &rem_output.counterfactuals {
            staging.put(counterfactual.clone().with_metadata(
                "staging_stage", "raw",
            )).await?;
            new_candidates += 1;
        }

        // Advance Raw -> Replayed for entries whose source episodes
        // appeared in this replay batch
        let replayed_ids = collect_episode_ids(&input);
        let raw_entries = staging.query(StoreQuery {
            metadata_eq: vec![("staging_stage", "raw")],
            ..Default::default()
        }).await?;
        let mut entries_advanced = 0;
        for entry in raw_entries {
            if replayed_ids.contains(&entry.source_episode_id()) {
                staging.update_metadata(&entry.id(), "staging_stage", "replayed").await?;
                entries_advanced += 1;
            }
        }

        // --- Verify: redundancy and contradiction checks ---
        let existing = ctx.store.query(StoreQuery {
            kind: Some(Kind::Insight),
            min_tier: Some(Tier::Consolidated),
            ..Default::default()
        }).await?;

        let replayed_entries = staging.query(StoreQuery {
            metadata_eq: vec![("staging_stage", "replayed")],
            ..Default::default()
        }).await?;

        for entry in replayed_entries {
            let dominated = existing.iter().any(|ex| {
                entry.hdc_fingerprint()
                    .zip(ex.hdc_fingerprint())
                    .map(|(a, b)| a.similarity(b) >= self.redundancy_threshold)
                    .unwrap_or(false)
            });
            if !dominated {
                staging.update_metadata(&entry.id(), "staging_stage", "validated").await?;
                entries_advanced += 1;
            }
        }

        // --- D2: Insights to Heuristics ---
        // Promote Validated entries with enough confirmations
        let validated = staging.query(StoreQuery {
            metadata_eq: vec![("staging_stage", "validated")],
            ..Default::default()
        }).await?;

        let mut promoted = Vec::new();
        let mut needs_evidence = Vec::new();
        let mut rejected = Vec::new();

        for entry in validated {
            let confirmations = entry.confirmation_count();
            if confirmations >= self.min_confirmations_for_d2 {
                // Promote to Consolidated tier in main Store
                let mut promoted_signal = entry.clone();
                promoted_signal.set_tier(Tier::Consolidated);
                promoted_signal.set_balance(1.0);  // fresh balance
                ctx.store.put(promoted_signal.clone()).await?;
                staging.update_metadata(&entry.id(), "staging_stage", "promoted").await?;
                promoted.push(promoted_signal);
            } else {
                // Needs more evidence: tell NREM to prioritize source episodes
                needs_evidence.push(entry.id());
            }
        }

        // --- D3: Heuristics to Playbooks ---
        // Top heuristics by calibration score become PLAYBOOK.md entries
        // (only when promoted count exceeds threshold)

        // --- Predict-publish-correct ---
        let promotion_rate = promoted.len() as f64
            / (promoted.len() + needs_evidence.len() + rejected.len()).max(1) as f64;
        ctx.bus.publish(Pulse::outcome(
            topic!("dream.nrem.promotion_rate"),
            promotion_rate,
        )).await?;

        let quality = DreamQualityMetrics {
            promotion_rate,
            hypothesis_diversity: hdc_diversity(&rem_output.hypotheses),
            entries_advanced,
            entries_promoted: promoted.len(),
            new_candidates,
        };

        // Publish quality as Pulse for the DreamQualityLens
        ctx.bus.publish(Pulse::from_signal(
            topic!("dream.quality"),
            &Signal::new(Kind::Telemetry, quality.clone()),
        )).await?;

        Ok(IntegrationOutput {
            promoted,
            needs_evidence,
            rejected,
            staging_snapshot: staging.snapshot().await?,
            quality,
        }.into_signals())
    }
}
```

**Why Store + Verify**: Integration does two things -- it persists knowledge that passes validation (Store: `put` promoted Signals) and it validates candidates against existing knowledge (Verify: redundancy check, contradiction check, confirmation count). The Verify protocol is structurally appropriate because it produces Verdicts with evidence, and those Verdicts feed back into the staging buffer's confidence ladder.

---

## 4. Dream Scheduling IS a Trigger Cell

The scheduling problem is: when should the dream Loop fire? Three event sources must converge:

1. **Idle gap**: no waking Flows have executed for `idle_threshold` minutes
2. **Episode pressure**: unprocessed episode count exceeds threshold
3. **Cron schedule**: periodic timer (e.g., every 4 hours)

In the unified architecture, this is a Trigger Cell. Trigger Cells are push-based: they subscribe to Bus topics and fire when conditions are met (see [13-TRIGGERS.md](../../unified/13-TRIGGERS.md) S1).

### 4.1 Trigger Cell Definition

```rust
/// Dream scheduling Trigger.
///
/// Watches three event sources and fires the dream Loop Graph
/// when any trigger condition is met.
///
/// Protocol: Trigger (arm/disarm)
pub struct DreamTriggerCell {
    id: CellId,
    config: DreamScheduleConfig,
    /// Handle to the armed idle watcher.
    idle_handle: Option<TriggerHandle>,
    /// Handle to the armed cron timer.
    cron_handle: Option<TriggerHandle>,
    /// Episode count at last dream.
    last_dream_episode_count: AtomicUsize,
    /// Timestamp of last dream completion.
    last_dream_completed: RwLock<Option<DateTime<Utc>>>,
}

pub struct DreamScheduleConfig {
    /// Idle threshold before dream may fire.
    pub idle_threshold: Duration,           // default: 15 minutes
    /// Minimum episodes since last dream.
    pub min_episodes: usize,                // default: 5
    /// Cron expression for periodic dreams.
    pub cron: Option<String>,               // default: "0 */4 * * *" (every 4h)
    /// High-water mark for intensive mode.
    pub intensive_high_water: usize,        // default: 50
    /// Low-water mark to exit intensive mode.
    pub intensive_low_water: usize,         // default: 10
    /// Manual trigger enabled.
    pub manual_enabled: bool,               // default: true
    /// Bus topics that can trigger dreams.
    pub bus_trigger_topics: Vec<Topic>,
    /// Minimum score for bus-triggered dreams.
    pub bus_trigger_min_score: f64,         // default: 0.7
    /// Minimum interval between any two dreams.
    pub cooldown: Duration,                 // default: 10 minutes
}

#[async_trait]
impl TriggerProtocol for DreamTriggerCell {
    async fn arm(
        &self,
        binding: &TriggerBinding,
        bus: Arc<dyn Bus>,
    ) -> Result<TriggerHandle> {
        // 1. Subscribe to idle events on Bus
        bus.subscribe(topic!("agent.idle"), {
            let config = self.config.clone();
            let last_completed = self.last_dream_completed.clone();
            move |pulse| {
                let idle_duration = pulse.payload::<IdleEvent>()?.duration;
                let since_last = last_completed.read()
                    .map(|t| Utc::now() - t)
                    .unwrap_or(chrono::Duration::max_value());

                if idle_duration >= config.idle_threshold
                    && since_last >= config.cooldown.into()
                {
                    bus.publish(Pulse::trigger_fired(
                        topic!("trigger:dream:fired"),
                        DreamTrigger::Idle,
                    ))?;
                }
                Ok(())
            }
        }).await?;

        // 2. Subscribe to episode completion events on Bus
        bus.subscribe(topic!("episode.completed"), {
            let config = self.config.clone();
            let counter = self.last_dream_episode_count.clone();
            move |_pulse| {
                let current = counter.fetch_add(1, Ordering::Relaxed) + 1;
                if current >= config.intensive_high_water {
                    // Intensive mode: fire immediately
                    bus.publish(Pulse::trigger_fired(
                        topic!("trigger:dream:fired"),
                        DreamTrigger::EpisodeCount,
                    ))?;
                } else if current >= config.min_episodes {
                    // Normal mode: mark eligible (idle trigger will pick up)
                }
                Ok(())
            }
        }).await?;

        // 3. Arm cron timer if configured
        if let Some(cron_expr) = &self.config.cron {
            let schedule = Schedule::from_str(cron_expr)?;
            // The Engine's timer facility handles the cron tick
            bus.subscribe(topic!("cron.tick"), move |pulse| {
                if schedule.includes(&pulse.timestamp()) {
                    bus.publish(Pulse::trigger_fired(
                        topic!("trigger:dream:fired"),
                        DreamTrigger::Scheduled,
                    ))?;
                }
                Ok(())
            }).await?;
        }

        // 4. Subscribe to high-value Bus events (optional reactive trigger)
        for topic in &self.config.bus_trigger_topics {
            bus.subscribe(topic.clone(), {
                let min_score = self.config.bus_trigger_min_score;
                move |pulse| {
                    if pulse.score() >= min_score {
                        bus.publish(Pulse::trigger_fired(
                            topic!("trigger:dream:fired"),
                            DreamTrigger::BusPulse {
                                engram_hash: pulse.id().to_string(),
                            },
                        ))?;
                    }
                    Ok(())
                }
            }).await?;
        }

        Ok(TriggerHandle {
            id: self.id.into(),
            binding: binding.clone(),
            armed_at: Utc::now(),
            state: TriggerState::Armed,
        })
    }

    async fn disarm(&self, handle: TriggerHandle) -> Result<()> {
        // Unsubscribe all Bus subscriptions
        Ok(())
    }
}
```

### 4.2 Trigger Binding

The Trigger fires the dream Loop Graph. This is expressed as a `TriggerBinding` in the system config:

```toml
[[triggers]]
name = "dream-consolidation"
cell = "roko:dream-trigger"
graph = "dream-consolidation"
cooldown = "10m"

[triggers.config]
idle_threshold = "15m"
min_episodes = 5
cron = "0 */4 * * *"
intensive_high_water = 50
intensive_low_water = 10
bus_trigger_topics = ["gate.verdict.emitted", "episode.completed"]
bus_trigger_min_score = 0.7
```

When the Trigger fires, the Engine starts a Flow from the `dream-consolidation` Graph. The trigger payload becomes the input Signal to the first Cell (NREM).

### 4.3 Intensive Mode

When episode count exceeds `intensive_high_water` (default 50), the system enters intensive mode: back-to-back dream cycles until episode count drops below `intensive_low_water` (default 10). This is expressed as a property of the Trigger, not the Loop:

```rust
/// After a dream cycle completes, check if intensive mode should continue.
fn should_continue_intensive(
    trigger: &DreamTriggerCell,
    report: &DreamCycleReport,
) -> bool {
    let unprocessed = trigger.last_dream_episode_count.load(Ordering::Relaxed);
    unprocessed > trigger.config.intensive_low_water
}
```

The Trigger publishes another `trigger:dream:fired` Pulse immediately when intensive mode is active, causing the Engine to start another Flow. This avoids special-casing within the Loop itself -- the Loop does not know about intensive mode. It runs one cycle and exits. The Trigger decides whether to fire again.

---

## 5. The Staging Buffer IS a Store Partition

The existing `StagingBuffer` in `staging.rs` is a `Vec<StagingEntry>` with its own `save`/`load` methods. In the redesign, it becomes a **Store partition** with stricter demurrage.

### 5.1 Partition Semantics

```rust
/// The staging buffer is a Store partition with the topic prefix "dream:staging".
/// Entries in this partition have stricter demurrage than the main Store:
/// - Raw entries: 7-day expiry (vs. 30-day for normal Insights)
/// - Validated entries: 14-day expiry
/// - Promoted entries: removed from staging, live in main Store
///
/// The partition shares the same Store interface, so all Store operations
/// (query, query_similar, prune) work on staging entries.
pub fn staging_partition(store: &Arc<dyn Store>) -> Arc<dyn Store> {
    store.partition("dream:staging")
}
```

### 5.2 Stricter Demurrage

Staging entries pay higher demurrage than main Store entries. This creates economic pressure: dream outputs that do not validate quickly are garbage collected, preventing hallucinated insights from accumulating.

| Stage | Flat tax (r) | Exp decay (beta) | Effective lifetime |
|---|---|---|---|
| Raw | 0.10 | 0.15 | ~7 days |
| Replayed | 0.05 | 0.08 | ~14 days |
| Validated | 0.02 | 0.03 | ~30 days |
| Promoted | N/A (moves to main Store) | N/A | Main Store rates apply |

Compare with the main Store's Insight rates (`r=0.01, beta=0.02, ~30 days`). Staging is 3-10x faster decay. Unvalidated dream outputs must earn their place or disappear.

### 5.3 Confidence Ladder as Balance Thresholds

The existing four-stage confidence ladder (Raw 0.20, Replayed 0.30, Validated 0.50, Promoted 0.70) maps to balance thresholds on the Signal. Stage transitions happen when verification passes AND balance exceeds the threshold:

```rust
/// Confidence stages as balance thresholds on the Signal.
/// Advancing a stage requires both:
/// 1. Passing the stage's Verify check
/// 2. Having balance >= the stage's threshold
pub const STAGE_THRESHOLDS: &[(ConfidenceStage, f64)] = &[
    (ConfidenceStage::Raw, 0.20),
    (ConfidenceStage::Replayed, 0.30),
    (ConfidenceStage::Validated, 0.50),
    (ConfidenceStage::Promoted, 0.70),
];
```

This means that even if a dream output passes verification, it still needs sufficient balance (i.e., active use -- retrieved, cited, gate-passed) to promote. The demurrage economics from [06-MEMORY.md](../../unified/06-MEMORY.md) S3 apply uniformly.

---

## 6. Sleep-Time Compute IS a Hot Flow

The existing `DreamComputeBudget` allocates a fraction of daily inference budget to dreaming. In unified terms, dream consolidation runs as a **Hot Flow** -- a Flow managed by the Engine with explicit priority below waking Flows.

### 6.1 Priority Scheduling

```rust
/// Dream Flows run at priority level below waking Flows.
/// The Engine's scheduler ensures that waking work always preempts dreaming.
///
/// Priority levels:
/// - CRITICAL: safety checks, circuit breaker interventions
/// - HIGH: waking task execution (plan runner)
/// - NORMAL: learning loops (L1, L2), routine operations
/// - LOW: dream consolidation, cold storage archival
/// - BACKGROUND: telemetry export, index rebuilds
pub const DREAM_FLOW_PRIORITY: FlowPriority = FlowPriority::Low;
```

When a waking Flow needs resources (LLM tokens, API concurrency slots), the Engine pauses the dream Flow. When waking work completes, the dream Flow resumes. This is native to the Engine's concurrency model -- no special handling needed.

### 6.2 Budget as Cost Verify Cell

The `budget-gate` node in the Loop Graph (section 2.1) is a Verify Cell that checks remaining budget before allowing the feedback edge to fire:

```rust
/// Budget gate: check whether the dream has remaining budget to continue.
///
/// Protocol: Verify (verify_pre checks budget, verify_post is a no-op)
pub struct DreamBudgetGateCell {
    id: CellId,
    budget: DreamComputeBudget,
}

#[async_trait]
impl VerifyProtocol for DreamBudgetGateCell {
    async fn verify_pre(
        &self,
        _input: &[Signal],
        _plan: &ActionPlan,
        ctx: &VerifyContext,
    ) -> Result<Verdict> {
        let remaining = ctx.budget_remaining();
        let phase_budget = self.budget.total_dream_budget_usd();
        let fraction = remaining.as_usd() / phase_budget;

        Ok(Verdict {
            hard_pass: fraction > 0.0,
            reward: fraction,  // continuous signal for budget awareness
            hard_criteria: vec![CriterionResult {
                criterion: Criterion::WithinBudget { max_cost: Cost::usd(phase_budget) },
                passed: fraction > 0.0,
                score: fraction,
                evidence_refs: vec![],
            }],
            ..Default::default()
        })
    }

    async fn verify_post(
        &self,
        _input: &[Signal],
        _output: &[Signal],
        _ctx: &VerifyContext,
    ) -> Result<Verdict> {
        Ok(Verdict::pass())
    }
}
```

### 6.3 Cost Model Per Phase

Each phase Cell declares its estimated cost (section 3). The Engine aggregates costs across the Loop:

| Phase | Model Tier | Cost per Unit | Unit |
|---|---|---|---|
| NREM Replay | T0 (Fast/Haiku) | ~$0.001 | per episode |
| REM Imagination | T1 (Standard/Sonnet) | ~$0.01 | per counterfactual |
| Integration | None (pure computation) | $0 | per cycle |

For a typical cycle processing 30 episodes with 10 counterfactuals: `30 * $0.001 + 10 * $0.01 + $0 = $0.13`. With `dream_fraction=0.15` and `daily_budget=$10`, the dream budget is $1.50/day, allowing ~11 cycles.

---

## 7. DreamQualityDashboard IS a Lens

The DreamQualityDashboard described in source material becomes a Lens Cell. Lens Cells observe Bus traffic and Store content, compute derived metrics, and publish them as Pulses (see [02-CELL.md](../../unified/02-CELL.md) for the Lens pattern).

### 7.1 Lens Cell

```rust
/// DreamQualityLens: observe dream cycle outcomes, compute quality metrics,
/// publish as Pulses for phase allocation tuning.
///
/// Protocol: Observe
pub struct DreamQualityLens {
    id: CellId,
    /// Rolling window of dream cycle metrics (last 20 cycles).
    history: RwLock<VecDeque<DreamQualityMetrics>>,
}

pub struct DreamQualityMetrics {
    /// Fraction of staging entries that promote per cycle.
    /// Healthy range: 0.15 - 0.35.
    pub promotion_rate: f64,
    /// HDC diversity of generated hypotheses. Higher = more creative.
    pub hypothesis_diversity: f64,
    /// Fraction of promoted insights that improve waking task success.
    pub waking_improvement: f64,
    /// Fraction of dreams that produce contradictions or regressions.
    pub nightmare_rate: f64,
    /// Dollar cost per promoted insight.
    pub cost_efficiency: f64,
    /// Timestamp.
    pub measured_at: DateTime<Utc>,
}

impl DreamQualityMetrics {
    /// Healthy ranges for each metric.
    pub fn health_check(&self) -> Vec<(String, HealthStatus)> {
        vec![
            ("promotion_rate".into(), range_check(self.promotion_rate, 0.15, 0.35)),
            ("hypothesis_diversity".into(), min_check(self.hypothesis_diversity, 0.3)),
            ("nightmare_rate".into(), max_check(self.nightmare_rate, 0.10)),
            ("cost_efficiency".into(), max_check(self.cost_efficiency, 0.50)),
        ]
    }
}

impl Cell for DreamQualityLens {
    fn name(&self) -> &str { "dream-quality-lens" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Observe] }
    fn capabilities(&self) -> &Capabilities { Capabilities::read_only() }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let metrics = DreamQualityMetrics::from_signals(&input)?;

        // Add to rolling window
        let mut history = self.history.write();
        history.push_back(metrics.clone());
        if history.len() > 20 {
            history.pop_front();
        }

        // Compute trend via linear regression over last 20 cycles
        let trend = DreamTrend {
            promotion_rate_slope: linear_regression(
                &history.iter().map(|m| m.promotion_rate).collect::<Vec<_>>()
            ),
            cost_efficiency_slope: linear_regression(
                &history.iter().map(|m| m.cost_efficiency).collect::<Vec<_>>()
            ),
            waking_improvement_slope: linear_regression(
                &history.iter().map(|m| m.waking_improvement).collect::<Vec<_>>()
            ),
        };

        // Publish quality + trend as Pulse
        ctx.bus.publish(Pulse::from_signal(
            topic!("telemetry.dream.quality"),
            &Signal::new(Kind::Telemetry, (metrics, trend)),
        )).await?;

        Ok(vec![])
    }
}
```

### 7.2 Phase Allocation IS a Route Cell That Adapts

The static phase budget allocation (Hypnagogia 10%, NREM 30%, REM 50%, Integration 0%, Evolution 10%) becomes an adaptive Route Cell. It subscribes to the DreamQualityLens output and adjusts allocations based on which phase produces the best waking improvement.

```rust
/// Phase budget allocation as an adaptive Route Cell.
///
/// Subscribes to dream quality metrics and adjusts phase allocations
/// based on which phase's outputs produce the best waking improvement.
///
/// Protocol: Route
pub struct PhaseAllocationRouter {
    id: CellId,
    /// Current phase allocations (mutable, learned).
    allocations: RwLock<PhaseAllocations>,
    /// Per-phase EMA of waking improvement attributable to that phase.
    phase_improvement_ema: RwLock<HashMap<DreamPhaseKind, f64>>,
    /// Learning rate for allocation adjustment.
    learning_rate: f64,      // default: 0.05
    /// Minimum allocation for any phase (floor to prevent starvation).
    min_allocation: f64,     // default: 0.05
}

impl PhaseAllocationRouter {
    /// Update allocations based on observed quality metrics.
    ///
    /// The key insight: track which phase's outputs ultimately promote
    /// and improve waking performance. Allocate more budget to phases
    /// that produce better results.
    pub async fn update_from_quality(
        &self,
        metrics: &DreamQualityMetrics,
        phase_attribution: &PhaseAttribution,
    ) {
        let mut ema = self.phase_improvement_ema.write();
        let alpha = self.learning_rate;

        // Update per-phase improvement EMA
        for (phase, improvement) in &phase_attribution.per_phase_improvement {
            let current = ema.entry(*phase).or_insert(0.0);
            *current = alpha * improvement + (1.0 - alpha) * *current;
        }

        // Recompute allocations proportional to improvement EMA
        let total_improvement: f64 = ema.values().sum::<f64>().max(1e-9);
        let mut alloc = self.allocations.write();

        for (phase, improvement) in ema.iter() {
            let raw_fraction = improvement / total_improvement;
            let clamped = raw_fraction.max(self.min_allocation);
            match phase {
                DreamPhaseKind::Nrem => alloc.nrem = clamped,
                DreamPhaseKind::Rem => alloc.rem = clamped,
                DreamPhaseKind::Hypnagogia => alloc.hypnagogia = clamped,
                DreamPhaseKind::Evolution => alloc.evolution = clamped,
                DreamPhaseKind::Integration => {} // always 0 (no LLM cost)
            }
        }

        // Renormalize to sum to 1.0
        alloc.normalize();
    }
}
```

This creates a second-order feedback loop: dreams produce knowledge, knowledge improves waking performance, waking performance metrics feed back into phase allocation, phase allocation changes which dreams run next. The timescale is slow (delta: per-session), which provides stability.

---

## 8. Pluggable Phase Cells

Because each phase is a Cell in a Graph, new consolidation strategies can be added by inserting Cells into the Loop. The Graph is defined in TOML, so this is a configuration change, not a code change.

### 8.1 Example: Code Pattern Extractor

A specialized Cell that runs between NREM and REM, extracting code-specific patterns (common error sequences, successful refactoring moves, tool usage patterns):

```toml
# Insert a code pattern extractor between NREM and REM
[[nodes]]
id = "code-patterns"
label = "Code Pattern Extractor"
cell = "roko:dream-code-patterns"
protocol = ["Score", "Compose"]

# Rewire edges
[[edges]]
from = "nrem"
to = "code-patterns"
type = "data"

[[edges]]
from = "code-patterns"
to = "rem"
type = "data"
```

### 8.2 Example: Cross-Domain Analogy Finder

A Cell that runs in parallel with REM, searching for structural analogies across domains using Resonator Network factorization (see [06-MEMORY.md](../../unified/06-MEMORY.md) S8):

```toml
# Parallel analogy finder alongside REM
[[nodes]]
id = "analogy"
label = "Cross-Domain Analogy"
cell = "roko:dream-analogy"
protocol = ["Compose"]

[[nodes]]
id = "merge"
label = "Merge REM + Analogy"
kind = "fan-in"
merge = "union"

[[edges]]
from = "nrem"
to = "rem"
type = "data"

[[edges]]
from = "nrem"
to = "analogy"
type = "data"

[[edges]]
from = "rem"
to = "merge"
type = "data"

[[edges]]
from = "analogy"
to = "merge"
type = "data"

[[edges]]
from = "merge"
to = "integrate"
type = "data"
```

### 8.3 Example: Micro-Consolidation

For short idle gaps (1-5 minutes), a full dream cycle is too expensive. A single-Cell Graph provides lightweight consolidation:

```rust
/// Micro-consolidation: a single-Cell dream for short idle gaps.
/// Replays the top-3 highest-utility episodes, advances staging
/// entries that can be advanced, runs GC. No REM, no counterfactuals.
pub struct MicroConsolidationCell {
    id: CellId,
    max_replays: usize,  // default: 3
}

impl Cell for MicroConsolidationCell {
    fn name(&self) -> &str { "dream-micro" }
    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::Store, ProtocolId::Score]
    }

    fn estimated_cost(&self) -> Option<Cost> {
        // 3 episodes at T0: ~$0.003
        Some(Cost::usd(0.003))
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Abbreviated NREM: top-3 episodes only
        let episodes = ctx.store.query(StoreQuery {
            kind: Some(Kind::Episode),
            sort: SortBy::Custom("replay_utility"),
            limit: self.max_replays,
            ..Default::default()
        }).await?;

        // Score and extract any immediate patterns
        let insights = quick_pattern_extract(&episodes)?;

        // Advance staging entries that match
        let staging = ctx.store.partition("dream:staging");
        advance_staging_from_replayed(&staging, &episodes).await?;

        // Run GC on staging
        staging.prune(0.05).await?;

        Ok(insights)
    }
}
```

This is wired to a separate Trigger with a lower idle threshold:

```toml
[[triggers]]
name = "micro-consolidation"
cell = "roko:dream-trigger"
graph = "dream-micro"
cooldown = "1m"

[triggers.config]
idle_threshold = "2m"
min_episodes = 1
```

The point: the three-phase structure (NREM/REM/Integration) is not architecturally necessary. It is one Loop Graph among many possible configurations. A single adaptive Cell (micro-consolidation), a five-phase extended cycle (with Hypnagogia and Evolution), or a domain-specific pipeline can all coexist as different Graph definitions sharing the same primitive Cells.

---

## 9. The Complete Wiring

```
                    +-------------------+
                    | DreamTriggerCell  |  <-- watches Bus: idle, episodes, cron
                    | (Trigger protocol)|
                    +--------+----------+
                             |
                             | trigger:dream:fired Pulse
                             v
                    +--------+----------+
                    |   Engine.start()  |  <-- starts Flow at LOW priority
                    +--------+----------+
                             |
                             v
               +-------------+-------------+
               |   Dream Loop Graph        |
               |                           |
               |   +-------+    +------+   |
               |   | NREM  |--->| REM  |   |
               |   | Store |    |Compose|   |
               |   | Score |    | Route|   |
               |   +---+---+    +--+---+   |
               |       ^           |       |
               |       |           v       |
               |   +---+---+  +---+----+   |
               |   |Budget |<-|Integrate|  |
               |   | Gate  |  | Store  |   |
               |   |Verify |  | Verify |   |
               |   +-------+  +--------+   |
               |                           |
               +---------------------------+
                             |
                             | dream.quality Pulse
                             v
               +-------------+-------------+
               | DreamQualityLens          |
               | (Observe protocol)        |
               +-------------+-------------+
                             |
                             | telemetry.dream.quality Pulse
                             v
               +-------------+-------------+
               | PhaseAllocationRouter     |
               | (Route protocol)          |
               +---------------------------+
                             |
                             | updates PhaseAllocations
                             v
                    (feeds into next dream cycle's
                     budget distribution)
```

---

## What This Enables

1. **Scheduling is finally wired.** The DreamTriggerCell uses the same Trigger infrastructure as every other event-driven process in Roko. It subscribes to Bus topics, publishes `trigger:dream:fired` Pulses, and the Engine starts Flows. No special scheduling code. The Mori lesson (dreams exist but no trigger calls them) is structurally prevented: the Trigger is a declared Cell in the system config.

2. **Phases are pluggable.** Adding a new consolidation strategy means defining a new Cell and editing the Graph TOML. The code-pattern extractor, cross-domain analogy finder, and micro-consolidation examples (section 8) each took 10-30 lines of TOML to wire. No modification to existing phase Cells.

3. **Budget allocation adapts.** The PhaseAllocationRouter learns from waking improvement attributable to each phase. If REM imagination consistently produces insights that improve waking performance, REM gets more budget. If NREM replay is sufficient, NREM's allocation grows. The floor constraint (5% minimum per phase) prevents any phase from starving.

4. **Dreams during active execution.** Micro-consolidation (section 8.3) runs during short idle gaps at trivial cost. This closes the gap between the existing 15-minute idle threshold and the sub-minute gaps that occur between task dispatches.

5. **Sleep-time compute amortization.** Because dreams run as Low-priority Flows, they naturally fill idle compute capacity. The Engine's priority scheduler ensures waking work is never delayed. The 5x reduction in test-time compute predicted by Lin (2025) follows from the amortization: dream-generated heuristics are available at zero marginal cost to all subsequent waking tasks.

6. **Convergence is observable.** The Loop's convergence condition (staging stable) and the DreamQualityLens's metrics are published as Pulses. Any Observe Cell can track dream health. The TUI can display dream quality in a dedicated tab. The HTTP control plane can expose dream metrics at `/api/dreams/quality`.

---

## Feedback Loops

| Loop | Source | Target | Signal | Timescale |
|---|---|---|---|---|
| **Phase->Integration->NREM** | Integration `needs_evidence` | NREM replay priority | `IntegrationOutput.needs_evidence: Vec<SignalRef>` | Per-iteration (within one dream) |
| **DreamQuality->PhaseAllocation** | DreamQualityLens | PhaseAllocationRouter | `telemetry.dream.quality` Pulse | Per-dream-cycle (delta) |
| **WakingOutcome->DreamQuality** | Waking gate verdicts referencing dream-sourced knowledge | DreamQualityLens `waking_improvement` | `gate.verdict.emitted` Pulse with lineage to staging entries | Per-task (theta) |
| **Staging->Demurrage->GC** | Store demurrage tick | Staging partition | Balance decay on staging entries; prune at cold threshold | Per-demurrage-interval (hours) |
| **Budget->Trigger->Cooldown** | Budget exhaustion in budget-gate | DreamTriggerCell cooldown | Loop exits, Trigger respects cooldown before refiring | Per-dream-cycle |
| **NREM predict->Integration outcome** | NREM publishes prediction on `dream.nrem.promotion_rate` | Integration publishes outcome on same topic | CalibrationPolicy joins prediction+outcome, updates NREM's CalibrationTable | Per-iteration (predict-publish-correct) |
| **REM predict->WakingVerify outcome** | REM publishes prediction on `dream.rem.hypothesis_diversity` | Waking Verify outcomes for hypothesis-sourced knowledge | CalibrationPolicy joins with lag (hypothesis must survive staging + waking use) | Per-session (delayed feedback) |

---

## Open Questions

1. **Is the three-phase structure optimal?** The redesign preserves NREM/REM/Integration as the default Loop, but micro-consolidation (section 8.3) shows a single-Cell alternative works for short gaps. Should the system start with micro-consolidation and escalate to full three-phase only when episode pressure is high? Or should the PhaseAllocationRouter be able to allocate 0% to a phase, effectively collapsing the Loop?

2. **Phase attribution is hard.** The PhaseAllocationRouter needs to know which phase produced the knowledge that improved waking performance. But a promoted insight may have been initiated by NREM replay, refined by REM imagination, and validated by Integration. How should credit be assigned? Shapley values across the phase sequence? Or simpler: attribute to the phase that created the raw candidate?

3. **Dream-to-dream feedback latency.** Integration's `needs_evidence` feeds back within a single dream session (sub-minute latency). But the DreamQualityLens's waking improvement metric takes days to observe (a promoted insight must be retrieved, used in a waking task, and gate-verified). This means the PhaseAllocationRouter updates on a much slower timescale than the within-dream Loop. Is there a faster proxy for waking improvement that could tighten this loop?

4. **Interaction with L2 routing.** The CascadeRouter (L2) selects model tiers for waking tasks. Dream phases also select model tiers (NREM=T0, REM=T1). Should the CascadeRouter's learned preferences influence dream model selection? If L2 discovers that a particular model excels at counterfactual reasoning, should REM use that model instead of the default T1?

5. **Multi-agent dream sharing.** The current design is per-agent. When multiple agents dream independently, they may discover the same patterns. Should dream outputs be published on a shared Bus topic so that one agent's Integration can consume another agent's NREM insights? This connects to the stigmergy coordination in [06-MEMORY.md](../../unified/06-MEMORY.md) S12 -- dream outputs are a form of pheromone.

6. **Dream interruption.** If a high-priority waking task arrives during a dream, the Engine pauses the dream Flow. But should the dream state be preserved for resumption, or should it be discarded? The Flow/Activity split ([04-EXECUTION.md](../../unified/04-EXECUTION.md) S6) provides the mechanism for snapshotting, but the question is whether a partially-completed dream cycle has value. If NREM completed but REM was interrupted, are the NREM insights worth staging?

7. **Convergence speed.** The Loop's max_iterations is set to 10 (section 2.1). In practice, how many iterations does it take for the staging buffer to stabilize? If convergence is typically 2-3 iterations, the budget allocated for 10 is wasted. If convergence is often not reached at 10, the cap is too low. Empirical measurement is needed once the Loop is wired.
