# Knowledge Lifecycle Loop

> Depth for [06-MEMORY.md](../../unified/06-MEMORY.md). How knowledge distillation, calibration, backup, and federation emerge as a single Loop Graph with predict-publish-correct at every stage.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Kind, demurrage, HDC fingerprints), [02-CELL](../../unified/02-CELL.md) (Score protocol, Compose protocol, Verify protocol, Store protocol, Observe protocol, Trigger protocol), [03-GRAPH](../../unified/03-GRAPH.md) (Loop pattern, Pipeline pattern, feedback edges), [04-EXECUTION](../../unified/04-EXECUTION.md) (Hot Flow, Engine), [06-MEMORY](../../unified/06-MEMORY.md) (Store, tiers, demurrage, distillation stages), [07-LEARNING](../../unified/07-LEARNING.md) (predict-publish-correct, L1-L4 Loop taxonomy)

---

## 0. Why This Document Exists

The parent spec ([06-MEMORY](../../unified/06-MEMORY.md)) defines the _what_: four tiers, demurrage, D1/D2/D3 distillation, calibration receipts, backup/restore, mesh sync, dream consolidation. This document defines the _how_: a single Loop Graph that unifies all of those mechanisms under predict-publish-correct, running continuously as a Hot Flow rather than in discrete batches.

The central claim: **distillation, calibration, ingestion, backup/restore, and mesh sync are not five separate subsystems.** They are the same Pipeline -- the Knowledge Lifecycle Pipeline -- running at different velocities, consuming different inflow channels, but sharing identical Cell types and the same feedback Loop.

If you understand one stage, you understand them all.

---

## 1. The Pipeline Problem (Status Quo)

Today's codebase (`roko-neuro`) implements distillation as three sequential batch functions:

- **D1** (`TierProgression::extract_insights`): mines recurring patterns from episodes, emits `InsightRecord` Signals at Transient tier.
- **D2** (`TierProgression::promote_heuristics`): clusters Insights with 5+ supporting episodes, emits `HeuristicRule` Signals with when/then clauses.
- **D3** (`TierProgression::compile_playbook`): ranks top Heuristics by confidence, renders `PLAYBOOK.md`.

Three things are wrong with this design:

1. **Batch boundaries are arbitrary.** D1 runs over "the episodes accumulated so far." Nothing defines when "so far" ends and the next batch begins. The distiller either processes everything (expensive, redundant) or must track a watermark externally (fragile, unportable).

2. **Feedback is open-loop.** A Heuristic compiled into the playbook gets used by agents, which produce new episodes, which feed back into D1. But the Pipeline has no structural awareness of this feedback. The calibration replay (`replay_heuristics`) is called manually after the pipeline completes, not as a Pipeline stage.

3. **Ingestion channels are ad-hoc.** Self-distillation, user restore, mesh sync, and dream consolidation each call `KnowledgeStore::ingest` (or `ingest_with_source`) with different confidence discounts hardcoded per caller. There is no single boundary Cell that normalizes all inflows.

The redesign closes all three gaps.

---

## 2. The Loop Graph

The Knowledge Lifecycle is a single **Loop Graph** -- a Graph specialization where the output feeds back to the input (see [03-GRAPH](../../unified/03-GRAPH.md)). The Graph contains seven Cells connected in a Pipeline with a feedback edge from the terminal Cell back to the entry Cell.

```
                          FEEDBACK EDGE
          +---------------------------------------------+
          |                                             |
          v                                             |
    +-----------+     +--------+     +---------+     +--------+
    |  INGEST   |---->| DISTILL|---->| CALIBRATE|--->| PUBLISH|
    | (Score)   |     | (D1/D2)|     | (Verify) |    | (Store)|
    +-----------+     +--------+     +---------+     +--------+
          ^                                             |
          |              +--------+                     |
          +--------------| OBSERVE|<--------------------+
          |              | (Lens) |
          |              +--------+
          |
    +-----+------+
    |   PRUNE    |
    | (Trigger)  |
    +------------+
```

The graph is defined in TOML:

```toml
[graph]
name = "knowledge-lifecycle"
specialization = "loop"

# Convergence: re-evaluate when new Signals arrive, not on a timer.
# The Loop runs continuously; each iteration processes a micro-batch
# of Signals that entered since the last iteration.
[graph.loop]
mode = "event-driven"            # not "periodic"
min_interval = "500ms"           # debounce: wait at least 500ms between iterations
max_batch = 64                   # process at most 64 inbound Signals per tick
convergence = "input-exhausted"  # stop when no new Signals remain in the ingest queue

[[cells]]
id = "ingest"
cell = "roko:knowledge-ingest"
protocol = "Score"               # scores inbound Signals by source-channel trust

[[cells]]
id = "distill"
cell = "roko:knowledge-distill"
protocol = "Compose"             # composes lower-tier Signals into higher-tier Signals

[[cells]]
id = "calibrate"
cell = "roko:knowledge-calibrate"
protocol = "Verify"              # verify heuristics against episode outcomes

[[cells]]
id = "publish"
cell = "roko:knowledge-publish"
protocol = "Store"               # write graduated Signals to durable Memory Store

[[cells]]
id = "observe"
cell = "roko:knowledge-observe"
protocol = "Observe"             # Lens: emit distillation health metrics

[[cells]]
id = "prune"
cell = "roko:knowledge-prune"
protocol = "Trigger"             # fire when stagnation or cold-threshold conditions met

# Edges
[[edges]]
from = "ingest"
to = "distill"

[[edges]]
from = "distill"
to = "calibrate"

[[edges]]
from = "calibrate"
to = "publish"

[[edges]]
from = "publish"
to = "observe"

# Feedback edge: published Signals re-enter the ingest queue as
# potential inputs for the next distillation round.
[[edges]]
from = "publish"
to = "ingest"
feedback = true

# Prune is triggered by the Observe Lens when stagnation is detected.
[[edges]]
from = "observe"
to = "prune"
condition = "stagnation_detected"
```

### Why event-driven, not periodic

Periodic distillation (run every N minutes) wastes work when there are no new episodes and introduces arbitrary latency when episodes arrive in bursts. Event-driven iteration means: when a new Signal enters the ingest queue (an episode logged, a mesh sync received, a restore initiated), the Loop wakes up, debounces for 500ms to batch nearby arrivals, processes up to 64 Signals, and returns to sleep.

This is a **Hot Flow** -- the Loop is always alive, consuming Signals as they arrive, rather than a cold batch job that must be scheduled externally.

---

## 3. Cell Specifications

### 3.1 Ingest Cell (Score Protocol)

The Ingest Cell is the single boundary through which **all** knowledge enters the system. Every inflow channel -- self-distillation, dream consolidation, mesh sync, user restore, marketplace import, cross-collective federation -- passes through this Cell.

```rust
/// The single boundary for all knowledge inflow.
///
/// Implements Score protocol: assigns a trust-adjusted confidence
/// to each inbound Signal based on its source channel.
pub struct IngestCell {
    /// Per-channel confidence discount.
    channel_discounts: ChannelDiscountTable,
    /// Anti-knowledge repulsion index (HDC).
    anti_knowledge_index: HdcIndex,
    /// Deduplication bloom filter.
    dedup_filter: BloomFilter,
}

/// Confidence discount factors per source channel.
///
/// These are the canonical values. Any document that redeclares
/// source-channel discounts is referencing this table.
pub struct ChannelDiscountTable {
    pub self_distillation: f64,      // 1.00 -- we trust our own eyes
    pub gate_verdict: f64,           // 0.95 -- near-mechanical verification
    pub agent_output: f64,           // 0.80 -- LLM claims need validation
    pub user_restore: f64,           // 0.85 -- trusted but stale
    pub mesh_peer: f64,              // 0.80 -- collective intelligence
    pub marketplace: f64,            // 0.60 -- unknown provenance
    pub cross_collective: f64,       // 0.50 -- adversarial boundary
    pub dream_consolidation: f64,    // 0.70 -- speculative synthesis
}

impl Default for ChannelDiscountTable {
    fn default() -> Self {
        Self {
            self_distillation: 1.00,
            gate_verdict: 0.95,
            agent_output: 0.80,
            user_restore: 0.85,
            mesh_peer: 0.80,
            marketplace: 0.60,
            cross_collective: 0.50,
            dream_consolidation: 0.70,
        }
    }
}
```

The Ingest Cell performs five operations on each inbound Signal:

1. **Channel identification.** Read the Signal's `provenance.source` field and map to a `SourceChannel` variant.
2. **Confidence discount.** Multiply raw confidence by the channel's discount factor: `adjusted = raw * discount`.
3. **Deduplication.** Hash the Signal's content and check the bloom filter. If present, skip or merge (increment `confirmation_count` on the existing entry).
4. **Anti-knowledge repulsion.** Compute HDC similarity against the anti-knowledge index. Apply the three-threshold protocol from [06-MEMORY](../../unified/06-MEMORY.md) SS7: warn at 0.5, discount at 0.7, reject at 0.9.
5. **Tier assignment.** All inbound Signals enter at `Tier::Transient` regardless of source. Even user-restored Persistent Signals restart at Transient. Trust is earned through the pipeline, not imported.

**The Ingest Cell is the same for all inflow channels.** This is the key insight. Dream consolidation output, mesh sync payloads, marketplace imports, and backup restores all flow through the same Score Cell. The only difference is the discount factor applied in step 2.

```rust
impl Cell for IngestCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut output = Vec::with_capacity(input.len());

        for mut signal in input {
            let channel = self.identify_channel(&signal);
            let discount = self.channel_discounts.factor_for(channel);

            // Score protocol: adjust confidence
            signal.confidence = (signal.confidence * discount).clamp(0.0, 1.0);

            // Dedup: merge or skip
            if self.dedup_filter.check(&signal.content_hash) {
                ctx.bus.publish(Pulse::new(
                    "knowledge.ingest.duplicate",
                    &signal.content_hash,
                )).await;
                // Emit merge Signal instead of duplicate
                output.push(Signal::merge_confirmation(signal));
                continue;
            }

            // Anti-knowledge repulsion
            if let Some(action) = self.anti_knowledge_check(&signal) {
                match action {
                    RepulsionAction::Reject => {
                        ctx.bus.publish(Pulse::new(
                            "knowledge.ingest.rejected",
                            json!({
                                "reason": "anti_knowledge_repulsion",
                                "hash": signal.content_hash,
                            }),
                        )).await;
                        continue; // drop the Signal
                    }
                    RepulsionAction::Discount(factor) => {
                        signal.confidence *= factor;
                    }
                    RepulsionAction::Warn => {
                        ctx.bus.publish(Pulse::new(
                            "knowledge.ingest.anti_knowledge_warning",
                            &signal.content_hash,
                        )).await;
                    }
                }
            }

            // Force Transient tier on entry
            signal.tier = Tier::Transient;
            signal.balance = 1.0;

            self.dedup_filter.insert(&signal.content_hash);
            output.push(signal);
        }

        output
    }
}
```

### 3.2 Distill Cell (Compose Protocol)

The Distill Cell implements D1 and D2 as a single Compose operation that takes lower-tier Signals and produces higher-tier Signals. D3 (playbook compilation) is a projection -- not a separate stage -- handled by the Publish Cell.

```rust
/// Composes episodes into insights, insights into heuristics.
///
/// D1 and D2 are the same operation at different tiers:
///   - D1: episodes (Kind::Episode) with 3+ co-occurring patterns -> Insight
///   - D2: insights (Kind::Insight) with 5+ confirmations, 2+ contexts -> Heuristic
///
/// The Distill Cell does NOT call an LLM in the hot path. It uses
/// HDC clustering and PatternMiner (deterministic, <1ms per batch).
/// LLM-backed distillation (via the Distiller struct) runs as a
/// separate async task triggered by the Observe Lens when the
/// pattern-based pipeline produces ambiguous clusters.
pub struct DistillCell {
    d1_config: D1Config,
    d2_config: D2Config,
}

pub struct D1Config {
    /// Minimum co-occurring episodes to form an Insight.
    pub min_support: usize,         // default: 3
    /// Minimum confidence for the pattern to be promoted.
    pub min_confidence: f64,        // default: 0.30
    /// HDC similarity threshold for clustering.
    pub hdc_threshold: f64,         // default: 0.70
}

pub struct D2Config {
    /// Minimum supporting Insights for Heuristic promotion.
    pub min_insights: usize,        // default: 5
    /// Minimum confidence after aggregation.
    pub min_confidence: f64,        // default: 0.70
    /// Minimum distinct contexts (agents, tasks, domains).
    pub min_contexts: usize,        // default: 2
    /// Reject if any supporting Insight contradicts another.
    pub reject_on_contradiction: bool, // default: true
}

impl Cell for DistillCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Compose] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut output = Vec::new();

        // Partition input by kind
        let (episodes, insights, pass_through): (Vec<_>, Vec<_>, Vec<_>) =
            partition_by_kind(input);

        // D1: episodes -> insights
        if episodes.len() >= self.d1_config.min_support {
            let patterns = PatternMiner::new(self.d1_config.min_support)
                .mine(&episodes);
            for pattern in patterns {
                if pattern.confidence >= self.d1_config.min_confidence {
                    let insight = Signal::new_insight(
                        pattern.summary(),
                        pattern.confidence,
                        pattern.source_episodes(),
                    );
                    // Publish prediction: "this pattern will hold"
                    ctx.bus.publish(Pulse::prediction(
                        "distill.d1",
                        &insight.content_hash,
                        pattern.confidence,
                    )).await;
                    output.push(insight);
                }
            }
        }

        // D2: insights -> heuristics
        let all_insights: Vec<_> = insights.into_iter()
            .chain(output.iter().filter(|s| s.kind == Kind::Insight).cloned())
            .collect();

        let clusters = hdc_cluster(&all_insights, self.d2_config.hdc_threshold);
        for cluster in clusters {
            if cluster.len() < self.d2_config.min_insights { continue; }
            let contexts = distinct_contexts(&cluster);
            if contexts < self.d2_config.min_contexts { continue; }
            if self.d2_config.reject_on_contradiction
                && has_contradiction(&cluster) { continue; }

            let heuristic = compose_heuristic(&cluster);
            ctx.bus.publish(Pulse::prediction(
                "distill.d2",
                &heuristic.content_hash,
                heuristic.confidence,
            )).await;
            output.push(heuristic);
        }

        // Pass through Signals that are already at their target tier
        output.extend(pass_through);
        output
    }
}
```

**D1 and D2 are the same operation at different tiers.** This is not a metaphor. Both take N Signals of tier T, cluster by HDC similarity, check support and confidence thresholds, and emit a single Signal at tier T+1. The only differences are the threshold constants and the output Kind. A future D0 (raw observations to episodes) and D4 (heuristics to worldviews) would follow the same pattern.

### 3.3 Calibrate Cell (Verify Protocol)

The Calibrate Cell is where predict-publish-correct becomes concrete for knowledge. Every Heuristic Signal carries a prediction: "when these conditions hold, this outcome follows." The Calibrate Cell joins that prediction against actual episode outcomes and updates the Heuristic's calibration record.

```rust
/// Verifies heuristic predictions against reality.
///
/// This IS predict-publish-correct for knowledge:
///   - PREDICT: the Heuristic's when/then clause
///   - PUBLISH: the episode's actual outcome
///   - CORRECT: update calibration record, adjust confidence
pub struct CalibrateCell;

impl Cell for CalibrateCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut output = Vec::new();

        for signal in input {
            match signal.kind {
                Kind::Heuristic => {
                    let heuristic = signal.payload::<HeuristicPayload>()?;
                    let recent_episodes = ctx.store()
                        .query_episodes_matching(&heuristic.when)
                        .await?;

                    let mut receipts = Vec::new();
                    for episode in &recent_episodes {
                        let predicted = heuristic.then_holds_for(episode);
                        let actual = episode.success;
                        let action = calibration_action(predicted, actual);

                        receipts.push(CalibrationReceipt {
                            episode_ref: episode.signal_ref(),
                            action,
                            timestamp: Utc::now(),
                        });

                        // Publish correction on Bus
                        ctx.bus.publish(Pulse::new(
                            &format!(
                                "calibration.knowledge.{}",
                                signal.content_hash
                            ),
                            json!({
                                "predicted": predicted,
                                "actual": actual,
                                "action": action.as_str(),
                                "brier_delta": (predicted as u8 as f64
                                    - actual as u8 as f64).powi(2),
                            }),
                        )).await;
                    }

                    if receipts.is_empty() {
                        output.push(signal);
                        continue;
                    }

                    // Update the Heuristic's calibration
                    let mut updated = signal.clone();
                    let payload = updated.payload_mut::<HeuristicPayload>()?;
                    for receipt in &receipts {
                        payload.calibration.predictions += 1;
                        if receipt.action == CalibrationAction::Confirm {
                            payload.calibration.correct += 1;
                        }
                        payload.calibration.score =
                            payload.calibration.correct as f64
                            / payload.calibration.predictions as f64;
                        payload.receipts.push(receipt.clone());
                    }

                    // Brier score update (exponential moving average)
                    let batch_brier = compute_batch_brier(&receipts);
                    payload.calibration.brier_score = ema(
                        payload.calibration.brier_score,
                        batch_brier,
                        0.1,
                    );

                    // Tier progression decision
                    let decision = tier_decision(
                        &updated,
                        &receipts,
                    );
                    match decision {
                        TierDecision::Promote(tier) => {
                            updated.tier = tier;
                        }
                        TierDecision::Demote(tier) => {
                            updated.tier = tier;
                        }
                        TierDecision::Retire => {
                            // Spawn refined children with narrower
                            // when-clauses before retiring
                            let children = refine_heuristic(
                                &updated, &receipts,
                            );
                            output.extend(children);
                            updated.deprecated = true;
                        }
                        TierDecision::NoChange => {}
                    }

                    output.push(updated);
                }
                // Non-heuristic Signals pass through uncalibrated
                _ => output.push(signal),
            }
        }

        output
    }
}

fn calibration_action(predicted: bool, actual: bool) -> CalibrationAction {
    match (predicted, actual) {
        (true, true) => CalibrationAction::Confirm,
        (false, false) => CalibrationAction::Confirm,
        (true, false) => CalibrationAction::Violate,
        (false, true) => CalibrationAction::Violate,
    }
}
```

### 3.4 Publish Cell (Store Protocol)

The Publish Cell writes graduated Signals to durable Memory Store and compiles projections (D3 playbook is a projection of the Heuristic store, not a separate artifact).

```rust
/// Writes Signals to durable Store. Compiles projections.
///
/// D3 happens here: the playbook is a *projection* of the
/// Heuristic store, recompiled on each publish cycle.
/// The Heuristic is the source of truth; PLAYBOOK.md is a view.
pub struct PublishCell {
    store: Arc<dyn Store>,
    playbook_limit: usize,      // default: 12
}

impl Cell for PublishCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Store] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut published = Vec::new();
        let mut heuristics_changed = false;

        for signal in input {
            self.store.put(&signal).await?;
            ctx.bus.publish(Pulse::new(
                "knowledge.published",
                &signal.content_hash,
            )).await;

            if signal.kind == Kind::Heuristic {
                heuristics_changed = true;
            }

            published.push(signal);
        }

        // D3: recompile playbook projection when heuristics change
        if heuristics_changed {
            let top_heuristics = self.store
                .query_by_kind(Kind::Heuristic)
                .await?
                .into_iter()
                .filter(|s| !s.deprecated)
                .sorted_by(|a, b| {
                    b.payload::<HeuristicPayload>()
                        .map(|p| p.calibration.score)
                        .unwrap_or(0.0)
                        .partial_cmp(
                            &a.payload::<HeuristicPayload>()
                                .map(|p| p.calibration.score)
                                .unwrap_or(0.0)
                        )
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .take(self.playbook_limit)
                .collect::<Vec<_>>();

            let playbook_md = compile_playbook_markdown(&top_heuristics);
            self.store.put(&Signal::new(
                Kind::Projection,
                Body::text(playbook_md),
            )).await?;

            ctx.bus.publish(Pulse::new(
                "knowledge.playbook.updated",
                json!({ "heuristic_count": top_heuristics.len() }),
            )).await;
        }

        published
    }
}
```

### 3.5 Observe Cell (Lens)

The Observe Cell is a read-only Lens that watches distillation health without modifying the pipeline. It emits diagnostic Pulses on the Bus.

```rust
/// Read-only observation of distillation pipeline health.
///
/// Monitors:
///   - Throughput: Signals processed per iteration
///   - Promotion rate: fraction of Signals that gained a tier
///   - Calibration drift: average Brier score trend
///   - Stagnation: time since last promotion event
///   - Balance distribution: histogram of balance across tiers
pub struct ObserveCell {
    stagnation_threshold: Duration,   // default: 24 hours
    calibration_drift_window: usize,  // default: 100 observations
}

impl Cell for ObserveCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Observe] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let stats = compute_pipeline_stats(&input);

        // Emit health metrics as Pulses (Lens pattern: observe, don't modify)
        ctx.bus.publish(Pulse::new(
            "knowledge.health.throughput",
            json!({
                "signals_processed": stats.total,
                "promotions": stats.promotions,
                "demotions": stats.demotions,
                "rejections": stats.rejections,
            }),
        )).await;

        // Stagnation detection
        if stats.time_since_last_promotion > self.stagnation_threshold {
            ctx.bus.publish(Pulse::new(
                "knowledge.health.stagnation",
                json!({
                    "hours_since_promotion": stats.time_since_last_promotion
                        .as_secs() / 3600,
                    "total_heuristics": stats.heuristic_count,
                    "avg_calibration": stats.avg_calibration,
                }),
            )).await;

            // Emit a trigger Signal for the Prune Cell
            return Ok(vec![Signal::new(
                Kind::Trigger,
                Body::json(json!({ "action": "stagnation_review" })),
            )]);
        }

        // Calibration drift: if average Brier score is worsening,
        // promotion thresholds may need tuning (L1 parameter loop)
        if stats.brier_trend > 0.05 {
            ctx.bus.publish(Pulse::new(
                "knowledge.health.calibration_drift",
                json!({
                    "brier_trend": stats.brier_trend,
                    "window": self.calibration_drift_window,
                }),
            )).await;
        }

        // Lens: pass through without modification
        Ok(input)
    }
}
```

### 3.6 Prune Cell (Trigger Protocol)

The Prune Cell fires when the Observe Lens detects stagnation or when demurrage has driven entries below the cold threshold.

```rust
/// Triggered when knowledge store needs maintenance.
///
/// Three prune modes:
///   - Cold archival: balance < COLD_THRESHOLD -> cold storage
///   - Stagnation shake: randomly demote lowest-calibration heuristics
///     to force re-evaluation
///   - Expiry review: entries older than 2x half-life get re-evaluated
pub struct PruneCell {
    cold_threshold: f64,             // default: 0.05
    stagnation_demote_fraction: f64, // default: 0.10 (demote bottom 10%)
}

impl Cell for PruneCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Trigger] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let trigger = input.first()
            .and_then(|s| s.payload::<TriggerPayload>().ok());

        match trigger.map(|t| t.action.as_str()) {
            Some("stagnation_review") => {
                // Shake the store: demote bottom N% of heuristics
                // to force them back through the pipeline.
                // This prevents the store from ossifying.
                let heuristics = ctx.store()
                    .query_by_kind(Kind::Heuristic)
                    .await?;
                let demote_count = (heuristics.len() as f64
                    * self.stagnation_demote_fraction) as usize;
                let to_demote = heuristics.iter()
                    .sorted_by_calibration_ascending()
                    .take(demote_count.max(1));

                let mut demoted = Vec::new();
                for signal in to_demote {
                    let mut s = signal.clone();
                    s.tier = demote_tier(s.tier);
                    ctx.bus.publish(Pulse::new(
                        "knowledge.prune.stagnation_demote",
                        &s.content_hash,
                    )).await;
                    demoted.push(s);
                }
                Ok(demoted) // re-enter pipeline via feedback edge
            }
            _ => {
                // Default: cold-threshold archival
                let cold = ctx.store()
                    .query_below_balance(self.cold_threshold)
                    .await?;
                for signal in &cold {
                    ctx.store().archive_to_cold(signal).await?;
                    ctx.bus.publish(Pulse::new(
                        "knowledge.prune.archived",
                        &signal.content_hash,
                    )).await;
                }
                Ok(vec![])
            }
        }
    }
}
```

---

## 4. Backup/Restore as Store Protocol

Backup and restore are not special operations. They are `Store::get_many` and the Ingest Cell, respectively.

### Backup

```rust
/// Backup = Store::get_many with an export filter.
///
/// The backup file is a JSONL stream of Signals, identical in format
/// to the internal store. No transformation. No lossy compression.
/// The backup IS the store, filtered.
pub async fn backup(
    store: &dyn Store,
    filter: &ExportFilter,
    dest: &Path,
) -> Result<BackupManifest> {
    let entries = store.get_many(filter).await?;
    let header = BackupHeader {
        version: 1,
        created_at: Utc::now(),
        entry_count: entries.len(),
        source_path: store.path().to_string_lossy().into(),
    };

    let mut writer = BufWriter::new(File::create(dest)?);
    writeln!(writer, "{}", serde_json::to_string(&header)?)?;
    for entry in &entries {
        writeln!(writer, "{}", serde_json::to_string(entry)?)?;
    }

    Ok(BackupManifest { header, path: dest.to_owned() })
}
```

### Restore

Restore feeds the backup file through the **same Ingest Cell** that handles all other inflow. The only difference is the source channel (`user_restore`, discount 0.85x).

```rust
/// Restore = read JSONL + feed through Ingest Cell with
/// SourceChannel::UserRestore (0.85x confidence discount).
///
/// Restored Signals restart at Tier::Transient regardless of their
/// backed-up tier. They must earn their way back through the pipeline.
pub async fn restore(
    pipeline: &KnowledgeLifecycleLoop,
    backup_path: &Path,
) -> Result<RestoreReport> {
    let reader = BufReader::new(File::open(backup_path)?);
    let mut lines = reader.lines();

    // Skip header
    let header_line = lines.next()
        .ok_or(anyhow!("empty backup file"))??;
    let header: BackupHeader = serde_json::from_str(&header_line)?;

    let mut signals = Vec::with_capacity(header.entry_count);
    for line in lines {
        let line = line?;
        if line.trim().is_empty() { continue; }
        let mut signal: Signal = serde_json::from_str(&line)?;
        // Tag provenance so Ingest Cell applies correct discount
        signal.provenance.source = "user_restore".into();
        signal.provenance.restored_from = Some(backup_path.to_string_lossy().into());
        signals.push(signal);
    }

    // Feed through the pipeline's ingest queue
    pipeline.submit(signals).await?;

    Ok(RestoreReport {
        header,
        submitted: signals.len(),
    })
}
```

This design eliminates the four-step BACKUP-DELETE-CREATE-RESTORE ceremony. Restore is just "submit Signals to the pipeline with a different provenance tag." The pipeline handles deduplication, anti-knowledge repulsion, tier reset, and confidence discounting -- the same operations it performs for every other inflow channel.

---

## 5. Confidence Discounting as a Functor

The channel discount is a **Functor**: a structure-preserving map applied at the ingestion boundary. It transforms the confidence field of every Signal passing through the Ingest Cell without altering the Signal's content, lineage, or kind.

```rust
/// A Functor over Signal confidence, parameterized by source channel.
///
/// Functors are composable: applying mesh_peer discount (0.80) followed
/// by cross_collective discount (0.50) yields a combined discount of 0.40.
/// This is how multi-hop federation naturally accumulates trust loss.
pub struct ConfidenceFunctor {
    factor: f64,
}

impl ConfidenceFunctor {
    pub fn for_channel(channel: SourceChannel) -> Self {
        Self { factor: channel.discount_factor() }
    }

    /// Apply the functor to a Signal. Structure-preserving:
    /// only the confidence field changes.
    pub fn apply(&self, signal: &mut Signal) {
        signal.confidence = (signal.confidence * self.factor).clamp(0.0, 1.0);
    }

    /// Compose two functors. The combined factor is the product.
    pub fn compose(self, other: Self) -> Self {
        Self { factor: (self.factor * other.factor).clamp(0.0, 1.0) }
    }
}
```

Multi-hop federation composes functors naturally. A Signal that travels from Collective A (cross_collective: 0.50) through Mesh Peer B (mesh_peer: 0.80) arrives at the local Ingest Cell with a combined discount of `0.50 * 0.80 = 0.40`. Each hop applies its functor; the composition is multiplication. No special federation logic is needed -- the Ingest Cell sees a single SourceChannel and applies its discount. The provenance chain records the composition path for auditability.

---

## 6. Heuristic Calibration as Predict-Publish-Correct

The heuristic lifecycle is the clearest instance of predict-publish-correct in the knowledge system:

```
PREDICT:  Heuristic says "when X, then Y" (published as Pulse on prediction.heuristic.{id})
PUBLISH:  Episode outcome records whether Y actually happened (Pulse on outcome.heuristic.{id})
CORRECT:  Calibrate Cell joins prediction and outcome, updates Brier score, emits receipt
          (Pulse on calibration.heuristic.{id}.updated)
```

This is not just logging. The correction Pulse feeds back to the Heuristic through the Loop's feedback edge:

1. Calibrate Cell updates the Heuristic's `CalibrationRecord`.
2. Publish Cell writes the updated Heuristic to Store.
3. Observe Lens tracks calibration drift across all Heuristics.
4. On next pipeline iteration, the updated confidence influences which Heuristics survive D2 promotion thresholds and which get retired.

### Heuristic versioning

When a Heuristic is refined (its when-clause narrows), the Calibrate Cell spawns child Heuristics with `parent_heuristic` references. The parent is deprecated, not deleted. This creates a version DAG:

```
heuristic_001 (deprecated)
  +-- heuristic_001a ("when refactoring Rust code, run clippy")
  +-- heuristic_001b ("when refactoring TypeScript code, run eslint")
```

The version DAG is stored as Signal lineage -- the standard `source[]` mechanism from [01-SIGNAL](../../unified/01-SIGNAL.md). No special versioning infrastructure is needed. The DAG is queryable via the standard HDC similarity search: child Heuristics have high similarity to their parent (they share most of the HDC fingerprint content).

---

## 7. Worldviews as Emergent Graphs

Worldviews are not created. They are discovered.

The Observe Lens periodically computes a **co-activation matrix** over Heuristics: how often pairs of Heuristics appear together in successful gate evaluations. Clusters in this matrix are worldviews.

```rust
/// A Worldview is an emergent Graph of co-activated Heuristics.
///
/// Worldviews are not stored as first-class Signals. They are
/// computed on demand from the co-activation matrix. The matrix
/// IS the worldview; the named cluster is a projection.
pub struct WorldviewGraph {
    /// Co-activation counts: (heuristic_a, heuristic_b) -> count.
    pub co_activations: BTreeMap<(ContentHash, ContentHash), u64>,
    /// Minimum co-activation count to consider an edge.
    pub min_edge_weight: u64,        // default: 5
    /// Minimum cluster size to name a worldview.
    pub min_cluster_size: usize,     // default: 3
}

impl WorldviewGraph {
    /// Discover worldview clusters using connected-component analysis
    /// on the co-activation graph, filtered by edge weight threshold.
    pub fn discover(&self) -> Vec<WorldviewCluster> {
        let graph = self.build_adjacency(self.min_edge_weight);
        let components = connected_components(&graph);

        components.into_iter()
            .filter(|c| c.len() >= self.min_cluster_size)
            .map(|heuristic_ids| {
                let avg_calibration = mean_calibration(&heuristic_ids);
                let domain_fingerprint = hdc_bundle(
                    &heuristic_ids.iter()
                        .filter_map(|id| id.hdc_vector.as_ref())
                        .collect::<Vec<_>>()
                );
                WorldviewCluster {
                    heuristic_ids,
                    avg_calibration,
                    domain_fingerprint,
                    coherence: intra_cluster_similarity(&heuristic_ids),
                }
            })
            .collect()
    }
}
```

**Why emergent, not declared:** Declared worldviews require someone to decide which Heuristics belong together. Emergent worldviews discover this from usage data. If agents consistently use Heuristics A, B, and C together and those contexts pass gates, the co-activation matrix records the pattern. The worldview emerges without anyone naming it.

**Rival worldviews** appear when the co-activation graph has multiple components in the same domain HDC neighborhood. The component with the highest average calibration score is the "main" worldview; the next-highest is the "challenger." The 15% mandatory contrarian retrieval slot ([06-MEMORY](../../unified/06-MEMORY.md) SS6) always draws from the challenger, preventing cognitive monoculture.

---

## 8. Unified Ingestion Pipeline

Dreams, mesh sync, backup/restore, marketplace imports, and self-distillation all use the same Pipeline. The only difference is the provenance tag, which the Ingest Cell maps to a source channel for confidence discounting.

| Inflow Channel | Provenance Tag | Source Channel | Discount |
|---|---|---|---|
| Self-distillation (D1/D2 output) | `"tier-progression:d1"` | `SelfDistillation` | 1.00 |
| Dream consolidation | `"dream:integration"` | `DreamConsolidation` | 0.70 |
| Mesh peer sync | `"mesh:{peer_id}"` | `MeshPeer` | 0.80 |
| User restore | `"user_restore"` | `UserRestore` | 0.85 |
| Marketplace import | `"marketplace:{listing_id}"` | `Marketplace` | 0.60 |
| Cross-collective federation | `"collective:{collective_id}"` | `CrossCollective` | 0.50 |
| Verify verdict | `"gate:{gate_id}"` | `GateVerdict` | 0.95 |
| Agent reflection | `"agent:{agent_id}"` | `AgentOutput` | 0.80 |

This table IS the ingestion pipeline. There is no separate "4-stage ingestion" (QUARANTINE-CONSENSUS-SKILL_SANDBOX-ADOPT). Those four stages are emergent properties of the tier system:

- **QUARANTINE** = Transient tier (all inbound Signals start here).
- **CONSENSUS** = promotion from Transient to Working (3+ Verify passes required).
- **SKILL SANDBOX** = Working tier (Signal is used in context packs but not yet trusted).
- **ADOPT** = promotion to Consolidated (5+ independent confirmations from different agents/contexts).

The four stages are not implementation code -- they are tier progression thresholds. The Pipeline does not need to know about them.

---

## 9. Cybernetic Loops

### Loop 1: Promotion Threshold Tuning (L1, Gamma timescale)

The promotion thresholds (3 gate passes for Transient-to-Working, 5 confirmations for Working-to-Consolidated) are not constants. They are L1 parameters tuned by the standard parameter tuning Loop ([07-LEARNING](../../unified/07-LEARNING.md) SS3).

```rust
/// L1 parameter: minimum gate passes for Transient -> Working promotion.
pub struct PromotionThresholdParam {
    pub current: usize,
    pub range: ParamRange<usize>,  // [2, 10]
    pub ema_alpha: f64,            // 0.05
}
```

When the Observe Lens detects that promotion rate is too low (stagnation) or too high (noise flooding higher tiers), the L1 loop adjusts thresholds. Predict-publish-correct: the threshold predicts "Signals promoted at this threshold will pass subsequent gates"; reality is whether they do; correction adjusts the threshold via EMA.

### Loop 2: Model Routing for Distillation (L2, Theta timescale)

When the Distill Cell encounters ambiguous clusters that need LLM interpretation (the `Distiller` struct from `roko-neuro`), the L2 model routing Loop selects which model to use. Small, cheap models handle clear-cut patterns; expensive models handle ambiguous ones. The CascadeRouter makes this decision per-distillation-call, learning from the quality of distillation output.

### Loop 3: Knowledge Consolidation (L3, Delta timescale)

The Knowledge Lifecycle Loop itself IS L3. It runs continuously (Hot Flow) rather than periodically, but its effective timescale is delta (per-session) because it takes multiple sessions for episodes to accumulate enough support for D1 promotion.

### Self-tuning: What happens when the store stagnates

Stagnation means no promotions for `stagnation_threshold` duration (default: 24 hours). The Observe Lens detects this and triggers the Prune Cell in `stagnation_review` mode. The Prune Cell demotes the bottom 10% of Heuristics by calibration score, forcing them back through the pipeline. This is the system's answer to knowledge ossification: periodically shake the tree and see what falls.

If stagnation persists after shaking, the Observe Lens emits a `knowledge.health.chronic_stagnation` Pulse that the L4 structural adaptation Loop can pick up. L4 may propose lowering promotion thresholds, expanding the set of monitored episode types, or adjusting the LLM distillation prompt.

---

## 10. What This Enables

1. **Continuous distillation.** Knowledge is distilled as episodes arrive, not in batch jobs. Latency from observation to actionable heuristic drops from "whenever someone remembers to run distillation" to "within seconds of episode completion."

2. **Unified ingestion.** All knowledge sources -- self, peers, backups, marketplace -- flow through the same Ingest Cell. Adding a new source channel requires one line in the `ChannelDiscountTable`, not a new subsystem.

3. **Self-calibrating knowledge.** Heuristics that make bad predictions lose confidence automatically. No manual review required for retirement. The system's knowledge quality improves with usage, not just with curation.

4. **Emergent worldviews.** Coherent sets of heuristics are discovered from usage patterns, not declared by humans. Rival worldviews prevent cognitive monoculture.

5. **Composable trust.** Multi-hop federation "just works" because confidence discounting is a composable Functor. A Signal from a peer-of-a-peer arrives with `discount_a * discount_b` confidence, no special logic required.

6. **Anti-ossification.** The stagnation shake mechanism prevents the knowledge store from becoming a static archive. Knowledge that stops being validated is demoted and must re-earn its tier.

7. **Observable pipeline.** The Observe Lens provides continuous health metrics without modifying the pipeline. Dashboards, alerts, and L1/L4 tuning loops all consume the same Bus topics.

---

## 11. Feedback Loops

| Loop | What Observes | What Adjusts | Timescale |
|---|---|---|---|
| **Ingest -> Distill -> Calibrate -> Publish -> Ingest** | Published Signals re-enter as higher-tier inputs for next distillation round | Tier of each Signal | Delta (seconds to hours) |
| **Calibrate -> Bus -> Heuristic confidence** | Episode outcomes vs. heuristic predictions | Brier score, calibration.score, tier | Theta (per-task) |
| **Observe -> L1 Param Tuning** | Promotion rate, calibration drift | D1/D2 promotion thresholds | Gamma (per-tick) |
| **Observe -> Prune -> Pipeline** | Stagnation duration | Demote bottom 10% of heuristics | Delta (per-session) |
| **Publish -> Agent context -> Episode -> Ingest** | Playbook used in agent prompts | Episode quality (closes the full loop from knowledge to action to knowledge) | Theta (per-task) |
| **Worldview co-activation -> Contrarian retrieval** | Co-activation matrix | Which worldview serves the 15% contrarian slot | Delta (recomputed per dream cycle) |

---

## 12. Open Questions

1. **HDC vector staleness.** When a Heuristic's when-clause is refined (narrowed), should its HDC fingerprint be recomputed? Currently the fingerprint is set at creation time and never updated. A refined heuristic with an outdated fingerprint may cluster incorrectly.

2. **Dream vs. hot distillation.** The Hot Flow design runs distillation continuously. Dream consolidation ([06-MEMORY](../../unified/06-MEMORY.md) SS9) runs offline during idle periods. Should dreams produce a different kind of insight than hot distillation (e.g., counterfactual StrategyFragments that hot distillation cannot produce)? Or should dreams be retired in favor of the unified pipeline?

3. **Adversarial mesh ingestion.** The Ingest Cell applies a flat discount per source channel. A mesh peer that sends well-calibrated knowledge for 100 iterations and then sends poisoned knowledge on iteration 101 will have its poison accepted at 0.80x confidence. Is per-peer reputation tracking needed? If so, should it live in the Ingest Cell or as a separate Score Cell upstream?

4. **Functor composition auditing.** Multi-hop federation composes confidence functors by multiplication. After 5 hops at 0.80x each, confidence is `0.80^5 = 0.33`. Is there a floor below which federation becomes useless? Should there be a `min_federated_confidence` that rejects Signals whose accumulated discount is too severe?

5. **Stagnation threshold tuning.** The 24-hour default stagnation threshold is arbitrary. In a high-throughput environment (100+ episodes/hour), stagnation should be detected much sooner. In a low-throughput environment (10 episodes/day), 24 hours may be too aggressive. Should the threshold be a function of episode arrival rate?

6. **Playbook as projection vs. artifact.** The current design treats PLAYBOOK.md as a projection recompiled on every Heuristic change. Should the playbook also be versioned as a Signal in Store (with its own demurrage and tier)? This would allow agents to query "what the playbook looked like at time T" for retrospective analysis.

7. **CalibrationAction coverage.** The current codebase defines five calibration actions (Confirm, Violate, Refine, Generalize, Refute) but the Calibrate Cell above only uses Confirm and Violate. When should Refine and Generalize fire? The parent spec says "evidence refines scope" and "evidence broadens applicability" but does not specify the trigger conditions. This needs concrete predicates.
