# C-Factor as Lens

> Depth for [14-c-factor-collective-intelligence.md](../../docs/00-architecture/14-c-factor-collective-intelligence.md). Redesigns c-factor as a Lens Cell that computes collective intelligence from Bus and Store statistics, shows the five process variables as concrete Lens Cells, and addresses what happens when c-factor is optimized directly.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, demurrage), [02-CELL](../../unified/02-CELL.md) (Cell, Lens, React protocol, Verify protocol), [03-GRAPH](../../unified/03-GRAPH.md) (Graph wiring), [09-TELEMETRY](../../unified/09-TELEMETRY.md) (Lens data feeds), [10-LEARNING-LOOPS](../../unified/10-LEARNING-LOOPS.md) (L1-L4 loop taxonomy)

---

## 1. C-Factor Is a Lens, Not a Reward

C-factor measures collective intelligence across a cohort of agents. The key design decision: c-factor is a **covariate** -- an observable diagnostic that correlates with cohort quality -- not an objective to maximize. This distinction is load-bearing.

In unified terms, c-factor is a **Lens Cell** (see [02-CELL.md](../../unified/02-CELL.md) SS7). It observes Bus traffic and Store content, computes a scalar, and publishes it as a Pulse. It does not take action. It does not gate anything by itself. It is a sensor, not an actuator.

Why this matters: if you turn c-factor into a reward signal, the system will game it. It will route easy work to well-coordinated cohorts, suppress dissenting Signals to reduce entropy, and narrow the HDC diversity of its working set. You get a high c-factor and a brittle system.

The correct architecture: c-factor feeds L4 evolution decisions as one covariate among many. The React protocol Cells that actually intervene on cohort process check c-factor AND outcome quality together.

```rust
/// C-factor is a Lens Cell. It observes, computes, publishes. It does not act.
///
/// See [02-CELL.md](../../unified/02-CELL.md) SS7 for the Lens pattern:
/// Lens Cells subscribe to Bus topics, compute derived metrics,
/// and publish the result as Pulses on telemetry topics.
pub struct CollectiveIntelligenceLens {
    id: CellId,
    /// The five sub-lenses that compute process variables.
    sub_lenses: [Box<dyn Lens>; 5],
    /// Learned weights for combining process variables into scalar.
    weights: RwLock<CohortWeights>,
    /// Online learner that updates weights from cohort outcomes.
    learner: CohortWeightsLearner,
    /// Rolling window of cohort measurements.
    history: VecDeque<(CohortId, f64, Instant)>,
}

impl Cell for CollectiveIntelligenceLens {
    fn id(&self) -> CellId { self.id }
    fn name(&self) -> &str { "collective-intelligence-lens" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Observe] }
    fn capabilities(&self) -> &Capabilities {
        // Read-only: Bus subscription + Store query. No write, no shell, no LLM.
        Capabilities::read_only()
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // 1. Extract cohort membership from input Signals
        let cohort = extract_cohort(&input)?;

        // 2. Compute five process variables via sub-lenses
        let metrics = CohortMetrics {
            turn_taking_entropy: self.sub_lenses[0].measure(&cohort, ctx).await?,
            peer_prediction_accuracy: self.sub_lenses[1].measure(&cohort, ctx).await?,
            citation_reciprocity: self.sub_lenses[2].measure(&cohort, ctx).await?,
            delivery_rate: self.sub_lenses[3].measure(&cohort, ctx).await?,
            hdc_diversity: self.sub_lenses[4].measure(&cohort, ctx).await?,
        };

        // 3. Compute learned scalar
        let weights = self.weights.read();
        let c_factor = weights.dot(&metrics);

        // 4. Publish as Pulse on telemetry topic
        let pulse = Signal::pulse(
            Kind::Telemetry,
            topic!("telemetry.cohort.c_factor"),
            CFactorPayload { cohort_id: cohort.id, metrics, c_factor },
        );

        Ok(vec![pulse])
    }
}
```

---

## 2. The Five Process Variable Lenses

Each Woolley process variable becomes a concrete Lens Cell. These are the sub-lenses wired into the `CollectiveIntelligenceLens`. Each one subscribes to Bus topics and queries Store, computes a single scalar in `[0.0, 1.0]`, and publishes its result on a sub-topic.

### 2.1 Turn-Taking Entropy Lens

Turn-taking equality measures how evenly a cohort shares the conversational floor. If one agent dominates, the cohort is not truly collective.

```rust
/// Lens Cell: turn-taking entropy across a cohort.
///
/// Subscribes to: Bus Pulses on "agent.turn.*" for the cohort window.
/// Publishes to:  "telemetry.cohort.turn_taking"
///
/// Metric: normalized Shannon entropy of sender share.
///   H = -sum(p_i * ln(p_i)) / ln(N)
///   where p_i = turns by agent i / total turns, N = cohort size.
///   H = 1.0 means perfect equality. H -> 0.0 means one agent dominates.
pub struct TurnTakingEntropyLens;

impl TurnTakingEntropyLens {
    pub fn compute(turns_per_agent: &[u64]) -> f64 {
        let total: u64 = turns_per_agent.iter().sum();
        if total == 0 { return 0.0; }

        let n = turns_per_agent.len() as f64;
        if n <= 1.0 { return 1.0; }

        let entropy: f64 = turns_per_agent.iter()
            .filter(|&&t| t > 0)
            .map(|&t| {
                let p = t as f64 / total as f64;
                -p * p.ln()
            })
            .sum();

        // Normalize by maximum possible entropy (uniform distribution)
        entropy / n.ln()
    }
}
```

**Data source**: Bus Pulse authorship metadata. Every Pulse carries an `author: AuthorId` field. The Lens counts Pulses per author within the cohort window.

### 2.2 Peer Prediction Accuracy Lens

Social perceptiveness is the ability to predict what another agent will say or do next. In runtime terms, agents publish `prediction.*` Pulses and the Bus carries `outcome.*` Pulses. The Lens joins them.

```rust
/// Lens Cell: peer prediction accuracy.
///
/// Subscribes to: "prediction.{agent_id}" and "outcome.{agent_id}"
/// Publishes to:  "telemetry.cohort.peer_prediction"
///
/// Metric: 1.0 - mean_squared_error(predictions, outcomes)
///   clamped to [0.0, 1.0].
pub struct PeerPredictionLens;

impl PeerPredictionLens {
    pub fn compute(predictions: &[f64], outcomes: &[f64]) -> f64 {
        if predictions.is_empty() { return 0.5; } // no data = maximum uncertainty
        let mse: f64 = predictions.iter().zip(outcomes)
            .map(|(p, o)| (p - o).powi(2))
            .sum::<f64>() / predictions.len() as f64;
        (1.0 - mse).clamp(0.0, 1.0)
    }
}
```

**Data source**: The predict-publish-correct pattern from [10-LEARNING-LOOPS.md](../../unified/10-LEARNING-LOOPS.md) SS2. Every Cell already publishes predictions and receives corrections. The peer prediction Lens reuses this infrastructure but measures accuracy across agents, not within a single Cell.

### 2.3 Citation Reciprocity Lens

Trust calibration in runtime terms: when an agent cites a Signal that later survives Verify, trust increases. When the cited Signal fails Verify, trust decreases.

```rust
/// Lens Cell: citation reciprocity and downstream survival.
///
/// Subscribes to: Store lineage events (Signal cited/retrieved)
///                Verify outcomes (gate verdicts on cited Signals)
/// Publishes to:  "telemetry.cohort.citation_reciprocity"
///
/// Metric: fraction of citations where the cited Signal survived its
///         next Verify pass. Weighted by recency (newer citations count more).
pub struct CitationReciprocityLens;

impl CitationReciprocityLens {
    pub fn compute(citations: &[CitationRecord]) -> f64 {
        if citations.is_empty() { return 0.5; }
        let (survived, total) = citations.iter()
            .fold((0.0_f64, 0.0_f64), |(s, t), c| {
                let weight = c.recency_weight(); // exponential decay by age
                let survived_val = if c.downstream_gate_passed { weight } else { 0.0 };
                (s + survived_val, t + weight)
            });
        if total < f64::EPSILON { 0.5 } else { survived / total }
    }
}
```

**Data source**: Store lineage graph. Every Signal records `parent_hashes` (see [01-SIGNAL.md](../../unified/01-SIGNAL.md) SS2). The Lens walks lineage to find citations and joins them with Verify verdicts.

### 2.4 Bus Delivery Rate Lens

Channel openness measures whether intended Pulses actually reach their subscribers. A cohort with excellent local reasoning can still fail if the Bus drops traffic, backpressure saturates, or capability restrictions prevent delivery.

```rust
/// Lens Cell: Bus delivery confirmation rate.
///
/// Subscribes to: "bus.delivery.confirmed" and "bus.delivery.dropped"
/// Publishes to:  "telemetry.cohort.delivery_rate"
///
/// Metric: confirmed / (confirmed + dropped) over the cohort window.
pub struct DeliveryRateLens;

impl DeliveryRateLens {
    pub fn compute(confirmed: u64, dropped: u64) -> f64 {
        let total = confirmed + dropped;
        if total == 0 { return 1.0; } // no traffic = no drops
        confirmed as f64 / total as f64
    }
}
```

**Data source**: Bus internal instrumentation. The Bus publishes delivery confirmations and drop events on internal topics. This is where deployment observability matters -- the metric should show up alongside normal service health (see [09-TELEMETRY.md](../../unified/09-TELEMETRY.md)).

### 2.5 HDC Diversity Lens

Cognitive diversity is measured as distance across cohort HDC fingerprint clouds. If every agent's working set converges to the same region, the cohort is over-coupled and brittle.

```rust
/// Lens Cell: HDC diversity across cohort Signals.
///
/// Subscribes to: Store events for new Signals authored by cohort agents
/// Publishes to:  "telemetry.cohort.hdc_diversity"
///
/// Metric: mean pairwise cosine distance across agent centroid fingerprints.
///   Diversity = 1.0 - mean_pairwise_similarity.
///   Range: [0.0, 1.0]. Higher = more diverse.
pub struct HdcDiversityLens;

impl HdcDiversityLens {
    pub fn compute(agent_centroids: &[HdcVector]) -> f64 {
        let n = agent_centroids.len();
        if n < 2 { return 0.0; }

        let mut total_similarity = 0.0;
        let mut pairs = 0u64;
        for i in 0..n {
            for j in (i + 1)..n {
                total_similarity += hdc_cosine_similarity(
                    &agent_centroids[i], &agent_centroids[j],
                );
                pairs += 1;
            }
        }
        let mean_similarity = total_similarity / pairs as f64;
        1.0 - mean_similarity
    }
}
```

**Data source**: Every Signal carries an HDC fingerprint (see [01-SIGNAL.md](../../unified/01-SIGNAL.md) SS4). The Lens groups recent Signals by author, computes per-agent centroids via HDC bundle, then measures pairwise distance.

---

## 3. The C-Factor Scalar: Learned Composition

The five process variables are combined into a single scalar via learned weights. The weights are fitted online from cohort outcomes, not declared by fiat.

```rust
pub struct CohortWeights {
    pub turn_taking: f64,
    pub peer_prediction: f64,
    pub citation_reciprocity: f64,
    pub delivery_rate: f64,
    pub hdc_diversity: f64,
    pub bias: f64,
}

pub struct CohortMetrics {
    pub turn_taking_entropy: f64,
    pub peer_prediction_accuracy: f64,
    pub citation_reciprocity: f64,
    pub delivery_rate: f64,
    pub hdc_diversity: f64,
}

impl CohortWeights {
    /// Linear combination. The simplest possible composition.
    pub fn dot(&self, m: &CohortMetrics) -> f64 {
        self.turn_taking * m.turn_taking_entropy
            + self.peer_prediction * m.peer_prediction_accuracy
            + self.citation_reciprocity * m.citation_reciprocity
            + self.delivery_rate * m.delivery_rate
            + self.hdc_diversity * m.hdc_diversity
            + self.bias
    }
}

/// Online learner for CohortWeights.
///
/// Subscribes to: cohort completion Pulses (topic "cohort.completed")
///                with outcome quality attached.
/// Updates weights via gradient step toward observed outcome quality.
pub struct CohortWeightsLearner {
    learning_rate: f64,
    window: VecDeque<(CohortMetrics, f64)>, // (metrics, outcome_quality)
    max_window: usize,
}

impl CohortWeightsLearner {
    /// One gradient step. Called when a cohort completes and outcome is known.
    pub fn update(
        &self,
        weights: &mut CohortWeights,
        metrics: &CohortMetrics,
        outcome_quality: f64,
    ) {
        let predicted = weights.dot(metrics);
        let error = outcome_quality - predicted;

        weights.turn_taking += self.learning_rate * error * metrics.turn_taking_entropy;
        weights.peer_prediction += self.learning_rate * error * metrics.peer_prediction_accuracy;
        weights.citation_reciprocity += self.learning_rate * error * metrics.citation_reciprocity;
        weights.delivery_rate += self.learning_rate * error * metrics.delivery_rate;
        weights.hdc_diversity += self.learning_rate * error * metrics.hdc_diversity;
        weights.bias += self.learning_rate * error;
    }
}
```

The operational split is clear:
- **CohortMetrics** explain (why did this cohort perform well or poorly?).
- **CohortWeights** adapt (which process variables matter most in this deployment?).
- **c-factor** reports (the scalar published to dashboards and telemetry).
- **c-score** predicts (the fitted model behind the scalar).

---

## 4. The C-Factor Loop: Measure, Gate, Evolve

C-factor participates in the L4 evolution loop ([10-LEARNING-LOOPS.md](../../unified/10-LEARNING-LOOPS.md) SS6). The rule: only evolve structural changes that increase c-factor. But c-factor alone does not trigger evolution -- it must coincide with improved task outcomes.

```
C-Factor Loop (operates at L4 timescale -- per-approval)
==========================================================

1. CollectiveIntelligenceLens publishes c-factor for each cohort window.
2. Outcome Signals arrive (task completions, gate verdicts, quality scores).
3. CohortWeightsLearner joins c-factor with outcomes, updates weights.
4. L4 StructuralAdaptation proposes a change (new routing policy,
   different cohort composition, adjusted turn-taking temperature).
5. The change is applied to an observation window.
6. Post-change c-factor AND outcome quality are measured.
7. If BOTH improved or held stable: the change is retained.
   If c-factor rose but outcomes fell: REJECT (Goodhart violation).
   If c-factor fell but outcomes rose: RETAIN (c-factor was over-indexed).
   If both fell: REJECT.
```

The critical constraint in step 7 is the Goodhart guard. C-factor rising while outcomes fall is the signal that the system is gaming the metric. The AND condition prevents this.

---

## 5. Anti-Groupthink Primitives as React Cells

Optimizing for cohort cohesion can overshoot into groupthink. Three structural countermeasures are implemented as **React protocol Cells** (see [02-CELL.md](../../unified/02-CELL.md) SS5). React Cells subscribe to Signals and emit corrective actions.

### 5.1 Devil's Advocate React Cell

```rust
/// React Cell: injects an opposing viewpoint when consensus is too uniform.
///
/// Trigger: c-factor HDC diversity component drops below threshold
///          while turn-taking entropy remains high (agreement, not domination).
/// Action:  Emit a Pulse containing an explicit counterargument to the
///          current consensus, synthesized from minority-lineage Signals.
pub struct DevilsAdvocateReact {
    diversity_floor: f64,     // default: 0.25
    consensus_ceiling: f64,   // when turn-taking entropy > this AND diversity < floor
}

impl Cell for DevilsAdvocateReact {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::React] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let metrics = extract_cohort_metrics(&input)?;
        if metrics.hdc_diversity < self.diversity_floor
            && metrics.turn_taking_entropy > self.consensus_ceiling
        {
            // Consensus without diversity = groupthink risk.
            // Find Signals with lineage outside the majority cluster.
            let minority_signals = ctx.store().query(
                Query::hdc_far_from(metrics.majority_centroid, 0.5)
            ).await?;

            let counter_pulse = Signal::pulse(
                Kind::Coordination,
                topic!("cohort.devils_advocate"),
                DevilsAdvocatePayload {
                    minority_evidence: minority_signals,
                    reason: "Diversity below threshold during apparent consensus",
                },
            );
            Ok(vec![counter_pulse])
        } else {
            Ok(vec![])
        }
    }
}
```

### 5.2 Outsider Injection React Cell

```rust
/// React Cell: routes some work to an agent with zero lineage overlap.
///
/// Trigger: cohort HDC centroids converge (mean pairwise distance < threshold)
///          over 3+ consecutive measurement windows.
/// Action:  Emit a routing hint Signal that the Route protocol should
///          assign the next task to an agent outside the current cohort.
pub struct OutsiderInjectionReact {
    convergence_threshold: f64,   // default: 0.15
    convergence_window: usize,    // default: 3
    history: Mutex<VecDeque<f64>>,
}
```

### 5.3 Minority Report Preservation React Cell

```rust
/// React Cell: applies softer demurrage to dissenting Signals.
///
/// Trigger: a Signal is marked as dissenting (its HDC fingerprint is
///          far from the cohort centroid AND it received low citation count).
/// Action:  Emit a demurrage override Signal that reduces the decay rate
///          on the dissenting Signal, keeping it alive longer.
///
/// Rationale: dissenting Signals are the system's hedge against monoculture.
/// If they decay at the normal rate, the majority view wins by attrition,
/// not by evidence. Softer demurrage gives minority views time to be proven
/// right or wrong on their merits.
pub struct MinorityReportReact {
    distance_threshold: f64,       // HDC distance from centroid. default: 0.6
    demurrage_discount: f64,       // fraction of normal rate. default: 0.3
}
```

These three React Cells form a structural defense against c-factor maximization turning into monoculture. They are not rhetorical devices -- they are runtime policies wired into the Graph that processes cohort telemetry.

---

## 6. WisdomGate as a Verify Cell

Before a consensus artifact is finalized, it should pass a **WisdomGate** -- a Verify protocol Cell that encodes the conditions under which group consensus is meaningful rather than merely loud.

```rust
/// Verify Cell: consensus quality gate.
///
/// Maps the four classical conditions (Surowiecki 2004) to runtime checks:
///   1. Diversity of opinion     -> HDC diversity above threshold
///   2. Independence             -> lineage overlap below threshold
///   3. Decentralization         -> sender concentration below threshold
///   4. Aggregation              -> explicit aggregation method applied
pub struct WisdomGate {
    min_hdc_diversity: f64,      // default: 0.3
    max_lineage_overlap: f64,    // default: 0.5
    max_sender_share: f64,       // default: 0.4 (no agent > 40% of turns)
    aggregation: AggregationMethod,
}

pub enum AggregationMethod {
    /// HDC bundle: component-wise majority vote across fingerprints.
    Bundle,
    /// HDC bind: preserves order/provenance in the composition.
    Bind,
    /// Weighted bundle: trust-weighted composition.
    WeightedBundle { trust_source: String },
    /// Cleanup to codebook: snap to nearest known concept.
    CodebookCleanup { codebook: String },
}

impl Cell for WisdomGate {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let consensus = extract_consensus_candidate(&input)?;

        let diversity_ok = consensus.hdc_diversity >= self.min_hdc_diversity;
        let independence_ok = consensus.lineage_overlap <= self.max_lineage_overlap;
        let decentralized_ok = consensus.max_sender_share <= self.max_sender_share;
        let aggregated = consensus.aggregation_applied;

        let passed = diversity_ok && independence_ok && decentralized_ok && aggregated;

        let verdict = Signal::new(
            Kind::Verdict,
            WisdomVerdict {
                passed,
                diversity: consensus.hdc_diversity,
                overlap: consensus.lineage_overlap,
                concentration: consensus.max_sender_share,
                reason: if !passed {
                    format!(
                        "Failed: diversity={diversity_ok}, independence={independence_ok}, \
                         decentralized={decentralized_ok}, aggregated={aggregated}"
                    )
                } else {
                    "All four Surowiecki conditions met".into()
                },
            },
        );
        Ok(vec![verdict])
    }
}
```

---

## 7. What Happens When Someone Optimizes C-Factor Directly?

This section exists because the question matters. C-factor is designed as a covariate, but someone will inevitably try to turn it into an objective.

**Failure mode 1: Easy-task routing.** The system routes easy tasks to cohorts with high c-factor, inflating the metric while the hard work goes unaddressed. Detection: compare c-factor with task difficulty distribution. If high-c-factor cohorts only see trivial tasks, the metric is gamed.

**Failure mode 2: Dissent suppression.** The system penalizes agents that lower turn-taking entropy by disagreeing. This raises c-factor but kills the diversity that makes c-factor meaningful. Detection: HDC diversity trending downward while c-factor rises. The WisdomGate should start failing.

**Failure mode 3: Prediction collusion.** Agents learn to predict each other accurately by converging to the same outputs, not by actually modeling each other. Detection: peer prediction accuracy rises while outcome diversity falls. The system is right about what everyone will say because everyone says the same thing.

**Structural defense**: the AND condition in the L4 loop (section 4). C-factor must rise (or hold) together with outcome quality. If the metric moves without the outcomes, the change is rejected. Combined with the three anti-groupthink React Cells, this creates a system that is structurally resistant to c-factor Goodharting.

---

## 8. Cohort Extraction

A cohort is the unit of c-factor measurement. It is defined as a set of agents working on a shared plan, task family, or parent episode during a bounded window.

```rust
pub struct Cohort {
    pub id: CohortId,
    /// Agents in this cohort.
    pub agents: Vec<AuthorId>,
    /// The plan, task family, or episode that binds this cohort.
    pub binding: CohortBinding,
    /// Measurement window.
    pub window: TimeWindow,
}

pub enum CohortBinding {
    Plan(PlanId),
    TaskFamily(String),
    ParentEpisode(EpisodeId),
}
```

Cohort extraction is the main practical gap. It requires consistent task-family labeling so the Lens can group agents into measurement units. Without stable cohort boundaries, c-factor measures noise.

---

## What This Enables

1. **Cohort-level observability**: operators see process quality, not just task outcomes.
2. **Structural groupthink defense**: three React Cells prevent consensus from collapsing into monoculture.
3. **Evidence-gated evolution**: L4 changes are retained only when c-factor AND outcomes both improve.
4. **Learned weight calibration**: the system discovers which process variables matter most for its specific workload.
5. **WisdomGate for consensus**: group decisions must meet the four Surowiecki conditions before being finalized.

## Feedback Loops

- **L1**: gate threshold EMA on WisdomGate pass rates adjusts diversity/overlap thresholds.
- **L2**: CascadeRouter uses c-factor as a context feature for model routing (not reward).
- **L3**: Delta consolidation includes cohort c-factor in episode metadata for replay analysis.
- **L4**: structural evolution proposals are tested against c-factor AND outcome covariance.
- **Weight learning**: the CohortWeightsLearner continuously adjusts which process variables predict outcome quality.

## Open Questions

1. **Cross-deployment c-factor**: can c-factor weights transfer across deployments, or are they deployment-specific? The heuristic commons would need a mechanism for sharing learned weights.
2. **Dynamic cohort boundaries**: should cohorts be fixed at plan start, or should they evolve as agents join and leave? Fixed boundaries are simpler to measure but miss runtime recomposition.
3. **Causal direction**: does high c-factor cause good outcomes, or do easy problems produce both high c-factor and good outcomes? The AND condition helps but does not resolve the causal question.
4. **Human-in-the-loop cohorts**: when a human is part of the cohort, how should their turns be weighted? Humans have different turn-taking patterns than agents.
5. **Bus delivery rate as confounder**: delivery rate depends on infrastructure quality, not cohort process. Should it be deconfounded before entering the c-factor computation?
