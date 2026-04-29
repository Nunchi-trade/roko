# Autocatalytic Compounding

> Depth for [16-autocatalytic-and-cybernetics.md](../../docs/00-architecture/16-autocatalytic-and-cybernetics.md). Redesigns the seven compounding mechanisms as Loops, derives the Kauffman autocatalytic condition, identifies single points of failure in the feedback graph, and shows anti-metrics as Verify Cells.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, demurrage, HDC fingerprints), [02-CELL](../../unified/02-CELL.md) (Loop specialization, Verify protocol, Lens), [03-GRAPH](../../unified/03-GRAPH.md) (Graph wiring), [04-SPECIALIZATIONS](../../unified/04-SPECIALIZATIONS.md) (Loop definition), [10-LEARNING-LOOPS](../../unified/10-LEARNING-LOOPS.md) (L1-L4 taxonomy), [11-MEMORY-AND-KNOWLEDGE](../../unified/11-MEMORY-AND-KNOWLEDGE.md) (Memory, demurrage)

---

## 1. The Compounding Thesis

Roko gets better by using itself. The mechanism is not one loop but seven, tied together so each loop's output feeds the next loop's input. The system compounds.

This is not a metaphor. It is a structural claim about feedback topology. The seven Loops form a connected cycle -- each one produces Signals that are consumed by at least one other. When the cycle is connected, the system is autocatalytic in the Kauffman (1993) sense: the outputs of the reaction network sustain the inputs that keep the reactions going.

The practical consequence: every unit of usage should make the next unit cheaper, faster, or better. If that curve flattens or reverses, a Loop has broken and the feedback graph is no longer connected.

---

## 2. Seven Compounding Loops

Each compounding mechanism is a **Loop** -- a Graph specialization where output feeds back to input (see [04-SPECIALIZATIONS.md](../../unified/04-SPECIALIZATIONS.md)). Loops are defined by their feedback topology, not their content.

```toml
# Loop definition schema (see [03-GRAPH.md](../../unified/03-GRAPH.md) SS3)
# A Loop is a Graph where at least one output port connects
# back to an input port, creating a feedback cycle.

[[loop]]
id = "demurrage-retrieval"
cells = ["query-cell", "demurrage-cell", "reinforce-cell", "store-cell"]
feedback_edge = { from = "store-cell.out", to = "query-cell.context" }

[[loop]]
id = "heuristic-calibration"
cells = ["predict-cell", "outcome-cell", "calibrate-cell"]
feedback_edge = { from = "calibrate-cell.updated_model", to = "predict-cell.model" }
```

### 2.1 Loop 1: Demurrage-Weighted Retrieval

Memory that sits idle is taxed. Memory that is used is reinforced. The result: a self-trimming Store where the retrieval surface converges toward what has actually been useful.

```rust
/// Loop: demurrage x retrieval -> self-trimming Memory.
///
/// Graph topology:
///   query_cell -> score_cell -> retrieve_cell -> reinforce_cell -> store_cell
///                                                                     |
///                                    query_cell.context <----feedback--+
///
/// Each turn through the loop:
///   1. Query Cell selects candidate Signals from Store.
///   2. Score Cell ranks them (HDC similarity + demurrage balance).
///   3. Retrieve Cell surfaces top-k to the Compose pipeline.
///   4. Reinforce Cell applies demurrage: +bonus for retrieved Signals,
///      -tax for idle Signals (see [01-SIGNAL.md](../../unified/01-SIGNAL.md) SS5).
///   5. Store Cell persists updated balances.
///   6. FEEDBACK: next query benefits from sharper balance distribution.
pub struct DemurrageRetrievalLoop {
    flat_tax_per_day: f64,       // 0.01
    retrieved_bonus: f64,        // 0.02
    cited_bonus: f64,            // 0.05
    gated_bonus: f64,            // 0.03
    surprised_bonus: f64,        // 0.15
    min_balance: f64,            // 0.0 (freeze threshold)
}

impl DemurrageRetrievalLoop {
    /// One tick of the demurrage Loop.
    /// Returns: (Signals reinforced, Signals frozen, Signals retrieved).
    pub fn tick(
        &self,
        store: &mut dyn Store,
        retrieved: &[ContentHash],
        cited: &[ContentHash],
        gated: &[ContentHash],
        surprised: &[ContentHash],
        elapsed_days: f64,
    ) -> DemurrageTick {
        let mut reinforced = 0u64;
        let mut frozen = 0u64;

        // Tax all warm-tier Signals
        for signal in store.warm_tier_iter() {
            let tax = self.flat_tax_per_day * elapsed_days;
            signal.balance -= tax;

            // Reinforce used Signals
            if retrieved.contains(&signal.hash) {
                signal.balance += self.retrieved_bonus;
                reinforced += 1;
            }
            if cited.contains(&signal.hash) {
                signal.balance += self.cited_bonus;
                reinforced += 1;
            }
            if gated.contains(&signal.hash) {
                signal.balance += self.gated_bonus;
            }
            if surprised.contains(&signal.hash) {
                signal.balance += self.surprised_bonus;
            }

            // Freeze depleted Signals
            if signal.balance < self.min_balance {
                store.move_to_cold_tier(signal.hash);
                frozen += 1;
            }
        }

        DemurrageTick { reinforced, frozen, retrieved: retrieved.len() as u64 }
    }
}
```

**Compounding mechanism**: More usage produces more reinforcement evidence. Better evidence improves the demurrage curve. A sharper demurrage curve makes retrieval more selective. More selective retrieval improves the quality of the next episode. The KPI is median tokens per task (should decrease monotonically for a given difficulty bucket).

### 2.2 Loop 2: Heuristic Calibration

Heuristics only compound if they can be falsified. A heuristic that never sees a counterexample does not get better; it just gets older. The predict-publish-correct pattern ([10-LEARNING-LOOPS.md](../../unified/10-LEARNING-LOOPS.md) SS2) makes falsification structural.

```rust
/// Loop: heuristic predict -> outcome -> calibrate -> better prediction.
///
/// Graph topology:
///   heuristic_cell -> predict_pulse -> outcome_pulse -> calibrate_cell
///        ^                                                    |
///        +--------------------feedback------------------------+
///
/// The calibrate Cell updates the heuristic's confidence interval.
/// Confidence shrinks as evidence accumulates, making the heuristic
/// more precise and more useful as a routing signal.
pub struct HeuristicCalibrationLoop {
    learning_rate: f64,          // EMA rate for confidence update
    min_samples_for_trust: u32,  // minimum observations before routing trusts this
}

impl HeuristicCalibrationLoop {
    pub fn calibrate(
        &self,
        heuristic: &mut Heuristic,
        prediction: f64,
        outcome: f64,
    ) {
        let error = (prediction - outcome).abs();
        // Shrink confidence interval when error is small
        heuristic.confidence_width = heuristic.confidence_width
            * (1.0 - self.learning_rate)
            + error * self.learning_rate;
        heuristic.samples += 1;
    }
}
```

**Compounding mechanism**: Better calibration improves downstream decisions. Better decisions produce better evidence. Better evidence improves calibration further. The KPI is mean confidence interval width per heuristic (should decrease with trials).

### 2.3 Loop 3: HDC Codebook Cleanup

HDC fingerprints turn similarity into a cheap cleanup operation. Every new episode, Verify result, and heuristic adds to the codebook. More episodes improve cleanup quality up to the large capacity of the codebook.

**Compounding mechanism**: More interactions improve codebook organization. Better organization means future queries collapse to the right Signal cluster on the first pass. The KPI is percentage of Compose prompts that hit HDC-clean cache on the first attempt (should asymptote toward 1.0).

### 2.4 Loop 4: C-Factor Feedback

C-factor measures cohort process quality. High c-factor cohorts produce higher-quality output, which produces better learning evidence for routing, demurrage tuning, and heuristic calibration. This is a three-loop reinforcement: c-factor rises, output quality rises, learning quality rises, c-factor rises again.

See [c-factor-as-lens.md](c-factor-as-lens.md) for the full treatment. The constraint: c-factor is a covariate, not the objective.

### 2.5 Loop 5: Playbook Distillation

Episodes compress into playbooks, and playbooks compress into meta-playbooks. That is learning about learning. Once the corpus is large enough, the cost per distilled unit drops while the transfer value per unit rises.

**Compounding mechanism**: The system no longer relearns the same structure from scratch. It learns the reusable shape of the work. The KPI is retroactive improvements per week from the Delta consolidation cycle.

### 2.6 Loop 6: Cross-Deployment Heuristic Commons

Once heuristics can be imported across deployments, each deployment contributes to a shared commons. The marginal cost of sharing is low, but the marginal value to other deployments is high. The system becomes more valuable as the install base grows.

**Compounding mechanism**: Each deployment contributes once but benefits many times. The KPI is first-task-after-install to success minutes (should decrease as commons grows).

### 2.7 Loop 7: Plugin Ecosystem

Plugins make capability portable. Each new plugin increases the value of Roko to users who need that capability, and each new user increases the value of building a plugin. Classic two-sided market.

**Compounding mechanism**: Network effects. The KPI is unique plugin count and unique plugin users.

---

## 3. The Feedback Graph

The seven Loops are not independent. They feed each other. The feedback graph shows which Loops are inputs to which other Loops.

```
                +----> [L4: c-factor] ----+
                |                         |
                |                         v
[L1: demurrage] <--+-- [L2: heuristic] --+--> [L5: playbook]
     |              |       ^                      |
     v              |       |                      v
[L3: HDC cleanup] --+-------+              [L6: commons]
                                                   |
                                                   v
                                           [L7: plugin ecosystem]
```

Edges in detail:

| From | To | What flows |
|---|---|---|
| L1 demurrage | L3 HDC cleanup | Sharper balance distribution improves codebook organization |
| L1 demurrage | L4 c-factor | Better retrieval produces better cohort outcomes |
| L2 heuristic | L1 demurrage | Calibrated heuristics inform which Signals to reinforce |
| L2 heuristic | L4 c-factor | Better predictions improve peer-prediction accuracy |
| L3 HDC cleanup | L2 heuristic | Cleaner codebook improves similarity-based prediction |
| L3 HDC cleanup | L5 playbook | Cleaner fingerprints improve distillation quality |
| L4 c-factor | L1 demurrage | High-c-factor cohorts produce Signals worth reinforcing |
| L4 c-factor | L2 heuristic | c-factor as context feature improves routing decisions |
| L5 playbook | L6 commons | Distilled playbooks are the currency of the commons |
| L6 commons | L1 demurrage | Imported heuristics populate the Store with high-value Signals |
| L6 commons | L7 plugin | Richer commons attract more users, justifying more plugins |
| L7 plugin | L6 commons | More plugins increase the value of sharing heuristics across them |

---

## 4. The Autocatalytic Condition (Kauffman)

A reaction network is autocatalytic when every reaction's inputs are produced by some other reaction in the network. The system sustains itself without external injection of raw materials.

In Loop terms: the system compounds when the feedback graph forms a **connected cycle** -- every Loop has at least one input that comes from another Loop, and there are no orphan Loops that receive nothing from the network.

```rust
/// Check the Kauffman autocatalytic condition on the feedback graph.
///
/// The condition holds when:
/// 1. Every Loop has at least one input edge from another Loop.
/// 2. The graph is strongly connected (every Loop is reachable from
///    every other Loop through directed edges).
///
/// If any Loop is orphaned (no incoming edges from another Loop),
/// the system has a growth bottleneck at that point.
pub fn check_autocatalytic(graph: &FeedbackGraph) -> AutocatalyticStatus {
    // Check condition 1: no orphan loops
    let orphans: Vec<LoopId> = graph.loops.iter()
        .filter(|l| graph.incoming_edges(l.id).is_empty())
        .map(|l| l.id)
        .collect();

    if !orphans.is_empty() {
        return AutocatalyticStatus::Broken {
            orphans,
            reason: "Loops with no incoming feedback cannot compound".into(),
        };
    }

    // Check condition 2: strong connectivity (Tarjan's algorithm)
    let sccs = tarjan_scc(&graph.adjacency);
    if sccs.len() == 1 && sccs[0].len() == graph.loops.len() {
        AutocatalyticStatus::Connected
    } else {
        AutocatalyticStatus::Fragmented {
            components: sccs,
            reason: "Feedback graph is not strongly connected; \
                     some Loops cannot reach all others".into(),
        }
    }
}

pub enum AutocatalyticStatus {
    /// All Loops form a single strongly connected component.
    Connected,
    /// Some Loops have no incoming edges.
    Broken { orphans: Vec<LoopId>, reason: String },
    /// Loops form multiple disconnected components.
    Fragmented { components: Vec<Vec<LoopId>>, reason: String },
}
```

**Current status**: Loops L1-L5 are strongly connected via the edges in section 3. L6 (commons) and L7 (plugin) are phase-2 and not yet connected in the running system. The autocatalytic condition holds for the inner five Loops but not yet for the full seven.

---

## 5. KPI Panel as Lens Cells

Each KPI is a concrete Lens Cell that publishes its measurement to the telemetry Bus.

```rust
/// KPI Lens Cells for the compounding dashboard.
///
/// Each publishes a Pulse on "telemetry.compounding.{kpi_name}"
/// at the cadence appropriate for its timescale.
pub enum CompoundingKpi {
    /// Mean time to first successful PR on a new codebase.
    /// Measures: all seven Loops together.
    /// Expected curve: steep initial drop, then continued decline.
    TimeToFirstPr,

    /// Median tokens per task, bucketed by difficulty.
    /// Measures: L1 (demurrage), L3 (HDC), L5 (playbook).
    /// Expected curve: monotonic decrease.
    MedianTokensPerTask,

    /// Percentage of Compose prompts hitting HDC-clean cache first try.
    /// Measures: L3 (HDC cleanup).
    /// Expected curve: asymptote toward 1.0.
    HdcCacheHitRate,

    /// Mean calibration confidence interval width per heuristic.
    /// Measures: L2 (heuristic calibration).
    /// Expected curve: decrease with trials.
    MeanCalibrationWidth,

    /// Percentage of heuristics sourced from commons.
    /// Measures: L6 (cross-deployment commons).
    /// Expected curve: increase, then stabilize.
    CommonsHeuristicFraction,

    /// C-factor on randomly sampled cohorts.
    /// Measures: L4 (c-factor feedback).
    /// Expected curve: stable or rising.
    CFactorSampled,

    /// Dream-cycle retroactive improvements per week.
    /// Measures: L5 (playbook distillation).
    /// Expected curve: growth with corpus size.
    DeltaImprovementsPerWeek,

    /// Unique plugin count.
    /// Measures: L7 (plugin ecosystem).
    UniquePlugins,

    /// First-task-after-install to success minutes.
    /// Measures: L6 (cross-deployment commons).
    /// Expected curve: decreases as commons grows.
    FirstTaskMinutes,
}
```

---

## 6. Anti-Metrics as Verify Cells

Three numbers should NOT increase. If they do, the system is accumulating complexity without compounding value. Each anti-metric is a **Verify protocol Cell** that emits a failing verdict when the anti-metric drifts upward.

```rust
/// Verify Cell: warm-tier Signal count stability.
///
/// The warm tier should stabilize, not grow without bound.
/// If it grows monotonically, demurrage is not trimming effectively.
pub struct WarmTierStabilityVerify {
    /// Maximum acceptable growth rate per week (as fraction of current count).
    max_growth_rate: f64, // default: 0.05 (5% per week)
    /// Measurement window in days.
    window_days: u32,     // default: 7
}

impl Cell for WarmTierStabilityVerify {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let history = extract_warm_tier_history(&input)?;
        let growth_rate = compute_growth_rate(&history, self.window_days);

        let passed = growth_rate <= self.max_growth_rate;
        Ok(vec![Signal::verdict(
            "warm-tier-stability",
            passed,
            format!("Growth rate: {growth_rate:.3} (max: {:.3})", self.max_growth_rate),
        )])
    }
}

/// Verify Cell: unconfirmed heuristic count.
///
/// Heuristics with fewer than 3 confirmations should not grow indefinitely.
/// If they do, the calibrator is generating heuristics faster than it tests them.
pub struct UnconfirmedHeuristicVerify {
    min_confirmations: u32,   // default: 3
    max_unconfirmed: u64,     // default: 100
}

impl Cell for UnconfirmedHeuristicVerify {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let count = count_heuristics_below_threshold(
            ctx.store(), self.min_confirmations
        ).await?;
        let passed = count <= self.max_unconfirmed;
        Ok(vec![Signal::verdict(
            "unconfirmed-heuristic-count",
            passed,
            format!("Unconfirmed: {count} (max: {})", self.max_unconfirmed),
        )])
    }
}

/// Verify Cell: mean lineage depth.
///
/// Lineage depth per response should not drift upward unless the extra
/// lineage is improving answer quality. Unbounded lineage growth means
/// the system is citing citations of citations without adding value.
pub struct LineageDepthVerify {
    max_mean_depth: f64,      // default: 5.0
    quality_correlation_min: f64, // default: 0.1
}

impl Cell for LineageDepthVerify {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let (mean_depth, quality_corr) = compute_lineage_stats(&input)?;

        // Fail if depth is high AND it is not correlated with quality
        let passed = mean_depth <= self.max_mean_depth
            || quality_corr >= self.quality_correlation_min;

        Ok(vec![Signal::verdict(
            "lineage-depth",
            passed,
            format!("Mean depth: {mean_depth:.1}, quality correlation: {quality_corr:.3}"),
        )])
    }
}
```

---

## 7. Pseudocode: One Complete Compounding Loop

The demurrage x HDC -> self-trimming Memory Loop, end to end.

```rust
/// Complete tick of the demurrage x HDC compounding Loop.
///
/// This is the inner loop that makes Memory self-trimming:
///   1. Query Store with HDC similarity.
///   2. Score candidates by (HDC distance * demurrage balance).
///   3. Retrieve top-k and pass to Compose.
///   4. After the episode completes, reinforce or tax.
///   5. Freeze depleted Signals.
///   6. Update codebook with episode fingerprints.
///   7. FEEDBACK: next query hits a sharper retrieval surface.
pub async fn demurrage_hdc_tick(
    store: &mut dyn Store,
    query_fingerprint: &HdcVector,
    episode_result: &EpisodeResult,
    config: &DemurrageConfig,
) -> CompoundingTick {
    // Step 1: HDC similarity query
    let candidates = store.query_similar(query_fingerprint, 100).await;

    // Step 2: Score by (similarity * balance)
    let mut scored: Vec<(ContentHash, f64)> = candidates.iter()
        .map(|(hash, similarity)| {
            let balance = store.get_balance(hash);
            (*hash, similarity * balance)
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    // Step 3: Retrieve top-k
    let top_k = &scored[..scored.len().min(10)];
    let retrieved_hashes: Vec<ContentHash> = top_k.iter().map(|(h, _)| *h).collect();

    // Step 4: Reinforce used Signals based on episode outcome
    let mut reinforced = 0u64;
    let mut frozen = 0u64;

    for (hash, _) in &scored {
        if retrieved_hashes.contains(hash) {
            // Retrieved and episode succeeded -> reinforce
            if episode_result.gate_passed {
                store.adjust_balance(hash, config.retrieved_bonus + config.gated_bonus);
                reinforced += 1;
            } else {
                // Retrieved but episode failed -> no bonus, just the retrieval bump
                store.adjust_balance(hash, config.retrieved_bonus * 0.5);
            }
        }

        // Step 5: Tax all warm-tier Signals
        let elapsed_days = episode_result.elapsed.as_secs_f64() / 86400.0;
        store.adjust_balance(hash, -(config.flat_tax_per_day * elapsed_days));

        if store.get_balance(hash) < config.min_balance {
            store.move_to_cold_tier(*hash);
            frozen += 1;
        }
    }

    // Step 6: Update codebook with new episode fingerprint
    if let Some(fp) = &episode_result.fingerprint {
        store.codebook_insert(fp);
    }

    CompoundingTick {
        retrieved: retrieved_hashes.len() as u64,
        reinforced,
        frozen,
        codebook_size: store.codebook_size(),
    }
}
```

---

## 8. What Breaks the Autocatalytic Cycle?

Single points of failure in the feedback graph. If any Loop stops producing its output, the downstream Loops lose their input and the cycle breaks.

| Failure point | What breaks | Detection | Recovery |
|---|---|---|---|
| **Demurrage miscalibration** | Tax too high -> Store empties. Tax too low -> Store bloats, retrieval degrades. | Anti-metric Verify Cells (section 6) | L1 auto-rollback restores previous tax rate |
| **Heuristic stagnation** | No falsifiers -> heuristics stop improving -> routing degrades | Mean calibration width plateaus | Importance sampling: deliberately test boundary cases |
| **HDC codebook saturation** | Codebook full of noise -> cleanup produces noise | HDC cache hit rate stops rising | Cold-tier eviction of low-balance codebook entries |
| **C-factor Goodharting** | C-factor gamed -> routing serves easy work -> real outcomes degrade | c-factor AND outcome divergence | The AND condition in L4 (see c-factor-as-lens section 4) |
| **Playbook staleness** | Playbooks not updated -> obsolete advice -> gate failures | Delta improvements per week drops to zero | Demurrage on playbook Signals; stale playbooks lose balance |
| **Commons poisoning** | Imported heuristics are wrong -> corrupts local calibration | First-task-after-install INCREASES | Quarantine imported heuristics until locally validated |
| **Bus partition** | Bus delivery drops -> Loops cannot communicate -> cycle fragments | Delivery rate Lens drops below threshold | Circuit breaker + Bus health alert |

The most dangerous failure mode is **silent degradation**: a Loop continues to produce output, but the output quality declines slowly. The anti-metric Verify Cells are designed to catch this by monitoring the numbers that should NOT increase.

---

## 9. Cybernetic Foundations in Unified Terms

| Cybernetic principle | Unified mapping |
|---|---|
| Ashby's Law of Requisite Variety | The number of Loop types must match the number of failure modes the system faces. Seven Loops for seven sources of improvement. |
| Conant-Ashby (Good Regulator) | The KPI panel (section 5) IS the system's model of its own learning dynamics. Self-regulation requires self-observation. |
| Beer's Viable System Model | The Loops operate at three timescales (L1-L2 at gamma/theta, L3-L5 at delta, L6-L7 at deployment) matching Beer's recursive viability. |
| Kauffman's Autocatalytic Sets | The connected cycle condition (section 4). The system compounds when every Loop's inputs are produced by the network. |

---

## What This Enables

1. **Superlinear returns**: each unit of usage improves the system for the next unit, not just the current one.
2. **Self-trimming Memory**: demurrage ensures the Store stays indexed toward useful Signals, not historical noise.
3. **Structural falsification**: heuristics improve because they are continuously tested, not because they are assumed correct.
4. **Autocatalytic health monitoring**: the Kauffman condition check detects when the feedback graph fragments.
5. **Anti-metric defense**: the system monitors for complexity accumulation and raises Verify failures before compounding inverts.

## Feedback Loops

The feedback topology IS the subject of this document. The meta-feedback: the KPI panel and anti-metric Verify Cells measure whether the Loops themselves are compounding. If the KPIs plateau or the anti-metrics rise, the system knows its own learning has stalled.

- **L1 within Loops**: gate threshold EMA tunes the Verify Cells that monitor anti-metrics.
- **L3 across Loops**: Delta consolidation reviews which Loops contributed to recent improvements and adjusts demurrage on their associated Signals.
- **L4 on the cycle**: structural evolution proposals are evaluated against the full KPI panel, not individual Loop metrics.

## Open Questions

1. **Bootstrapping**: the autocatalytic cycle needs initial inputs to start. What is the minimum viable corpus for the compounding to kick in? The first-task-after-install KPI answers this empirically, but there is no theoretical lower bound yet.
2. **Loop coupling strength**: should all edges in the feedback graph have equal weight, or should some Loops be more tightly coupled than others? Tight coupling speeds compounding but increases fragility.
3. **Negative feedback Loops**: the current architecture emphasizes positive feedback (compounding). Are there places where negative feedback (dampening) is needed to prevent runaway? The anti-metrics are a partial answer, but they are monitors, not controllers.
4. **Cross-workspace compounding**: if Roko manages multiple workspaces, do the Loops compound independently per workspace or across workspaces? Shared heuristic commons suggests cross-workspace, but demurrage and c-factor are workspace-local.
5. **Diminishing returns envelope**: Kauffman's theory predicts that autocatalytic sets eventually reach a steady state. What does that steady state look like for Roko? Is there a ceiling on compounding, and if so, what determines it?
