# 15 -- Telemetry: Lens System and StateHub

> Full observability through the Observe protocol. Lenses focus attention without modifying the subject. StateHub projects Lens output into typed projections consumed by every surface.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal/Pulse duality), [02-CELL](02-CELL.md) (Observe protocol, predict-publish-correct, Lens definition), [05-AGENT](05-AGENT.md) (CorticalState, vitality)

---

## 1. Overview

Every Cell, Graph, Agent, and Space in Roko can be observed without modification. The **Lens** specialization -- a Cell implementing the Observe protocol -- receives read-only lifecycle events and emits observation Signals onto the Bus.

The telemetry system has two layers:

1. **Lenses** -- the raw observation machinery. Cells that watch events and emit structured observation Signals.
2. **StateHub** -- the projection layer. Consumes Lens output and produces typed, versioned projections that every surface (TUI, web, Slack, audit) can subscribe to.

Four principles govern the design:

1. **Observation is passive.** A Lens never mutates the subject. Removing all Lenses from a system changes nothing about its behavior -- only visibility.
2. **Observation is compositional.** Lenses stack (multiple Lenses on one target), chain (a Lens watches another Lens's output), and scope (Cell, Graph, Agent, Space granularity).
3. **Observation uses the same primitives.** Lens output is a Signal. Lens composition is a Graph. Lens configuration is TOML. No special telemetry infrastructure.
4. **Projections are the data contracts.** StateHub projections are the typed interfaces between the telemetry system and the five named surfaces in [doc-20](20-SURFACES.md). Surfaces never read raw Lens output -- they subscribe to projections.

---

## 2. The Observe Protocol

The Observe protocol is the seventh of nine Cell protocols ([doc-02](02-CELL.md)). It is the only protocol that is strictly read-only -- calling `observe()` never modifies the event source.

```rust
pub trait Observe: Cell {
    /// Observe a single event. Emit zero or more observation Signals.
    /// The event is immutable -- Lenses cannot modify what they observe.
    async fn observe(&self, event: &ObservableEvent) -> Result<Vec<Signal>>;

    /// Declare which event types this Lens observes.
    /// The engine only routes matching events to this Lens.
    fn observes(&self) -> &[ObservableEventKind];

    /// The scope this Lens is attached to.
    fn scope(&self) -> LensScope;
}
```

### LensScope

```rust
pub enum LensScope {
    /// Observe a single Cell's events.
    Cell(CellRef),
    /// Observe all Cells within a Graph.
    Graph(GraphRef),
    /// Observe an Agent's full pipeline (all internal Graphs).
    Agent(AgentRef),
    /// Observe everything within a Space (all Agents, Graphs, Cells).
    Space(SpaceId),
    /// Observe a specific other Lens's output (chaining).
    Lens(LensRef),
    /// Global -- observe all events system-wide.
    Global,
}
```

The engine uses `observes()` and `scope()` together to build a routing table at Graph-load time. Events matching both the kind filter and the scope filter are delivered to the Lens. Events outside scope are never delivered -- the Lens does not pay for events it ignores.

---

## 3. Observable Events

```rust
pub enum ObservableEvent {
    // -- Signal lifecycle -------------------------------------------------
    SignalCreated(Signal),
    SignalScored(SignalRef, ScoreResult),
    SignalRouted(SignalRef, RouteResult),
    SignalVerified(SignalRef, Verdict),
    SignalComposed(Vec<SignalRef>, Signal),
    SignalDemurrageApplied(SignalRef, f64),
    SignalPromoted(SignalRef, Tier, Tier),
    SignalPruned(SignalRef),

    // -- Cell lifecycle --------------------------------------------------
    CellStarted { block: CellRef, run: RunId, input_hash: ContentHash },
    CellCompleted { block: CellRef, run: RunId, duration: Duration, cost: Cost },
    CellFailed { block: CellRef, run: RunId, error: CellError },
    CellRetried { block: CellRef, run: RunId, attempt: u32, reason: String },
    CellCancelled { block: CellRef, run: RunId },
    CellPredictionPublished { block: CellRef, prediction: Pulse },
    CellCalibrationReceived { block: CellRef, error: f64 },

    // -- Graph lifecycle --------------------------------------------------
    GraphStarted { graph: GraphRef, run: RunId, input_hash: ContentHash },
    GraphNodeCompleted { graph: GraphRef, run: RunId, node: NodeId, duration: Duration },
    GraphCompleted { graph: GraphRef, run: RunId, duration: Duration, cost: Cost },
    GraphFailed { graph: GraphRef, run: RunId, error: CellError },
    GraphPaused { graph: GraphRef, run: RunId, reason: PauseReason },
    GraphResumed { graph: GraphRef, run: RunId },

    // -- Agent lifecycle --------------------------------------------------
    AgentTick { agent: AgentRef, regime: Regime, prediction_error: f64, vitality: f64 },
    AgentRegimeChange { agent: AgentRef, old: Regime, new_regime: Regime },
    AgentBudgetUpdate { agent: AgentRef, spent: Cost, remaining: Cost, vitality: f64 },
    AgentModeChange { agent: AgentRef, old: AgentMode, new_mode: AgentMode },
    AgentPhaseChange { agent: AgentRef, old: VitalityPhase, new_phase: VitalityPhase },
    AgentStateTransition { agent: AgentRef, old: TypeState, new_state: TypeState },
    AgentSlotUpdate { agent: AgentRef, slot: SlotName, state: SlotState },

    // -- Memory lifecycle -------------------------------------------------
    MemoryRetrieved { query: String, results: usize, duration: Duration },
    MemoryStored { signal: SignalRef, tier: Tier },
    MemoryConsolidated { promoted: usize, demoted: usize, pruned: usize },
    DemurrageApplied { count: usize, total_balance_lost: f64 },

    // -- Verify lifecycle -------------------------------------------------
    VerifyPreResult { block: CellRef, verdict: Verdict, evidence: Vec<EvidenceKind> },
    VerifyPostResult { block: CellRef, verdict: Verdict, reward: f64, evidence: Vec<EvidenceKind> },

    // -- Trigger lifecycle ------------------------------------------------
    TriggerFired { trigger: TriggerRef, graph: GraphRef },
    TriggerArmed { trigger: TriggerRef },
    TriggerDisarmed { trigger: TriggerRef },

    // -- Extension lifecycle ----------------------------------------------
    ExtensionHookCalled { extension: String, hook: String, layer: u8, duration: Duration },
    ExtensionHookFailed { extension: String, hook: String, error: String },
}

pub enum ObservableEventKind {
    SignalLifecycle,
    CellLifecycle,
    GraphLifecycle,
    AgentLifecycle,
    MemoryLifecycle,
    VerifyLifecycle,
    TriggerLifecycle,
    ExtensionLifecycle,
    All,
}
```

A Lens declaring `observes() = &[ObservableEventKind::CellLifecycle]` receives only Cell lifecycle events. A Lens declaring `All` receives everything within its scope.

---

## 4. Built-in Lenses

Roko ships 11 built-in Lenses. Each is a Cell implementing Observe, packaged with the `roko-core` distribution.

### 4.1 CostLens

Tracks USD and token expenditure across Cell executions.

| Property | Value |
|---|---|
| **Observes** | `CellLifecycle`, `GraphLifecycle`, `AgentLifecycle` |
| **Default Scope** | `Graph` |
| **Emits** | `Signal { kind: CostReport }` |

```rust
pub struct CostReportPayload {
    pub target: String,
    pub interval: Duration,
    pub total_usd: f64,
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub model_breakdown: BTreeMap<String, f64>,
    pub cumulative_usd: f64,
    pub budget_remaining: Option<f64>,
    pub vitality: Option<f64>,
}
```

### 4.2 LatencyLens

Measures execution duration with percentile tracking.

| Property | Value |
|---|---|
| **Observes** | `CellLifecycle`, `GraphLifecycle` |
| **Default Scope** | `Graph` |
| **Emits** | `Signal { kind: Observation }` |

```rust
pub struct LatencyPayload {
    pub target: String,
    pub interval: Duration,
    pub count: u64,
    pub p50: Duration,
    pub p95: Duration,
    pub p99: Duration,
    pub mean: Duration,
}
```

### 4.3 QualityLens

Tracks pass/fail rates from Verify-protocol Cells, including pre/post verification with continuous reward.

| Property | Value |
|---|---|
| **Observes** | `VerifyLifecycle`, `SignalLifecycle` |
| **Default Scope** | `Graph` |
| **Emits** | `Signal { kind: Observation }` |

```rust
pub struct QualityPayload {
    pub target: String,
    pub interval: Duration,
    pub total_verifications: u64,
    pub pre_verify_vetoes: u64,
    pub post_verify_passed: u64,
    pub post_verify_failed: u64,
    pub pass_rate: f64,
    pub avg_reward: f64,             // mean Verdict.reward (continuous)
    pub hard_criteria_failures: u64,
    pub rung_breakdown: BTreeMap<String, PassFailCounts>,
}
```

### 4.4 EfficiencyLens

Measures tokens-per-task and cost-per-quality ratios.

| Property | Value |
|---|---|
| **Observes** | `CellLifecycle`, `AgentLifecycle` |
| **Default Scope** | `Agent` |
| **Emits** | `Signal { kind: Observation }` |

```rust
pub struct EfficiencyPayload {
    pub agent: String,
    pub interval: Duration,
    pub tasks_completed: u64,
    pub tokens_per_task: f64,
    pub usd_per_task: f64,
    pub quality_per_usd: f64,
    pub t0_hit_rate: f64,
    pub t1_hit_rate: f64,
    pub t2_hit_rate: f64,
    pub avg_prediction_error: f64,
    pub vitality: f64,
    pub vitality_phase: VitalityPhase,
}
```

### 4.5 ErrorLens

Classifies and aggregates errors across Cell executions.

| Property | Value |
|---|---|
| **Observes** | `CellLifecycle`, `GraphLifecycle`, `ExtensionLifecycle` |
| **Default Scope** | `Graph` |
| **Emits** | `Signal { kind: Observation }` |

```rust
pub struct ErrorPayload {
    pub target: String,
    pub interval: Duration,
    pub total_errors: u64,
    pub by_category: BTreeMap<ErrorCategory, u64>,
    pub by_block: BTreeMap<String, u64>,
    pub retry_count: u64,
    pub retry_success_rate: f64,
    pub error_rate: f64,
}

pub enum ErrorCategory {
    Timeout, CapabilityDenied, External, LogicError, InvalidInput, Cancelled,
}
```

### 4.6 DriftLens

Detects knowledge quality degradation in Memory stores. Tracks demurrage-driven balance changes (Gesell 1916) rather than time-based decay.

| Property | Value |
|---|---|
| **Observes** | `MemoryLifecycle`, `SignalLifecycle` |
| **Default Scope** | `Agent` |
| **Emits** | `Signal { kind: Observation }` |

```rust
pub struct DriftPayload {
    pub memory: String,
    pub interval: Duration,
    pub total_entries: u64,
    pub tier_distribution: BTreeMap<Tier, u64>,
    pub avg_balance: f64,
    pub balance_delta: f64,
    pub promotion_rate: f64,
    pub demotion_rate: f64,
    pub cold_entries: u64,
    pub anti_knowledge_count: u64,
    pub heuristic_calibration_avg: f64,
}
```

### 4.7 BudgetLens

Monitors budget consumption and vitality across Agents and Spaces.

| Property | Value |
|---|---|
| **Observes** | `AgentLifecycle`, `CellLifecycle` |
| **Default Scope** | `Agent` or `Space` |
| **Emits** | `Signal { kind: Alert }` when thresholds crossed |

```rust
pub struct BudgetAlertPayload {
    pub target: String,
    pub budget_total: f64,
    pub budget_spent: f64,
    pub budget_remaining: f64,
    pub vitality: f64,
    pub vitality_phase: VitalityPhase,
    pub projected_exhaustion: Option<DateTime<Utc>>,
    pub burn_rate: f64,
    pub level: AlertLevel,
}
```

### 4.8 TrendLens

Computes statistical trends over time-series observation data.

| Property | Value |
|---|---|
| **Observes** | Any other Lens's output (chaining Lens) |
| **Default Scope** | `Lens` (wraps another Lens) |
| **Emits** | `Signal { kind: Trend }` |

```rust
pub struct TrendPayload {
    pub source_lens: String,
    pub metric: String,
    pub window: Duration,
    pub slope: f64,
    pub ema: f64,
    pub ema_previous: f64,
    pub direction: TrendDirection,
    pub r_squared: f64,
    pub data_points: usize,
}

pub enum TrendDirection { Rising, Falling, Stable }
```

### 4.9 AnomalyLens

Detects statistical outliers in observation streams. Uses rolling window z-score with IQR fallback for non-Gaussian distributions.

| Property | Value |
|---|---|
| **Observes** | Any other Lens's output (chaining Lens) |
| **Default Scope** | `Lens` (wraps another Lens) |
| **Emits** | `Signal { kind: Anomaly }` |

```rust
pub struct AnomalyPayload {
    pub source_lens: String,
    pub metric: String,
    pub observed_value: f64,
    pub expected_value: f64,
    pub deviation: f64,
    pub direction: AnomalyDirection,
    pub severity: AnomalyLevel,
}
```

### 4.10 UsageLens

Tracks usage analytics for marketplace and developer metrics.

| Property | Value |
|---|---|
| **Observes** | `CellLifecycle`, `GraphLifecycle`, `TriggerLifecycle` |
| **Default Scope** | `Space` or `Global` |
| **Emits** | `Signal { kind: Observation }` |

Analytics are aggregated -- individual runs are never exposed to publishers. Spaces can opt out via `telemetry.marketplace = false`.

### 4.11 CollectiveIntelligenceLens (c-factor)

Computes the **c-factor** -- collective intelligence as a runtime observable. Grounded in Woolley et al. (2010, *Science*): 40%+ of group performance variance loads onto a single general factor driven by turn-taking equality and social perceptiveness, not mean IQ.

| Property | Value |
|---|---|
| **Observes** | `AgentLifecycle`, `SignalLifecycle`, `MemoryLifecycle` |
| **Default Scope** | `Space` (measures across all agents in a Space) |
| **Emits** | `Signal { kind: CFactorReport }` |

```rust
pub struct CFactorPayload {
    pub space: String,
    pub interval: Duration,
    pub c_factor: f64,                       // composite score (0.0..=1.0)

    // -- Components -------------------------------------------------------
    pub turn_taking_entropy: f64,            // Shannon entropy of agent turn distribution
    pub peer_prediction_accuracy: f64,       // predict-publish-correct on inter-agent Signals
    pub citation_reciprocity: f64,           // knowledge attribution flow balance
    pub hdc_diversity: f64,                  // spread of HDC fingerprints (avoid collapse)

    // -- Diagnostics ------------------------------------------------------
    pub agent_count: usize,
    pub active_agents: usize,
    pub dominant_agent_share: f64,
    pub knowledge_flow_edges: usize,
    pub avg_agent_vitality: f64,
}
```

---

## 5. C-Factor: Five Sub-Lenses

C-factor is a **covariate** -- an observable diagnostic that correlates with cohort quality -- not an objective to maximize. It is a Lens Cell (observe, compute, publish -- never act). If c-factor is turned into a reward signal, the system will game it: route easy work to well-coordinated cohorts, suppress dissenting Signals, narrow HDC diversity.

The correct architecture: c-factor feeds L4 evolution decisions as one covariate among many.

```rust
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
        let cohort = extract_cohort(&input)?;
        let metrics = CohortMetrics {
            turn_taking_entropy: self.sub_lenses[0].measure(&cohort, ctx).await?,
            peer_prediction_accuracy: self.sub_lenses[1].measure(&cohort, ctx).await?,
            citation_reciprocity: self.sub_lenses[2].measure(&cohort, ctx).await?,
            delivery_rate: self.sub_lenses[3].measure(&cohort, ctx).await?,
            hdc_diversity: self.sub_lenses[4].measure(&cohort, ctx).await?,
        };
        let weights = self.weights.read();
        let c_factor = weights.dot(&metrics);
        let pulse = Signal::pulse(
            Kind::Telemetry,
            topic!("telemetry.cohort.c_factor"),
            CFactorPayload { cohort_id: cohort.id, metrics, c_factor },
        );
        Ok(vec![pulse])
    }
}
```

### 5.1 Turn-Taking Entropy Lens

Measures how evenly a cohort shares the conversational floor. Normalized Shannon entropy of sender share.

```rust
/// H = -sum(p_i * ln(p_i)) / ln(N)
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

        entropy / n.ln()
    }
}
```

**Data source**: Bus Pulse authorship metadata. Every Pulse carries an `author: AuthorId` field. The Lens counts Pulses per author within the cohort window.

### 5.2 Peer Prediction Accuracy Lens

Social perceptiveness -- how well agents predict each other's output. Joins `prediction.*` Pulses with `outcome.*` Pulses.

```rust
/// Metric: 1.0 - mean_squared_error(predictions, outcomes), clamped to [0.0, 1.0].
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

**Data source**: The predict-publish-correct pattern from [07-LEARNING](07-LEARNING.md). Reuses existing Cell prediction infrastructure but measures accuracy across agents.

### 5.3 Citation Reciprocity Lens

Trust calibration: when an agent cites a Signal that later survives Verify, trust increases. When the cited Signal fails Verify, trust decreases.

```rust
/// Metric: fraction of citations where the cited Signal survived its
///         next Verify pass. Weighted by recency (newer citations count more).
pub struct CitationReciprocityLens;

impl CitationReciprocityLens {
    pub fn compute(citations: &[CitationRecord]) -> f64 {
        if citations.is_empty() { return 0.5; }
        let (survived, total) = citations.iter()
            .fold((0.0_f64, 0.0_f64), |(s, t), c| {
                let weight = c.recency_weight();
                let survived_val = if c.downstream_gate_passed { weight } else { 0.0 };
                (s + survived_val, t + weight)
            });
        if total < f64::EPSILON { 0.5 } else { survived / total }
    }
}
```

**Data source**: Store lineage graph. Every Signal records `parent_hashes` ([01-SIGNAL](01-SIGNAL.md)). The Lens walks lineage to find citations and joins with Verify verdicts.

### 5.4 Bus Delivery Rate Lens

Channel openness -- whether intended Pulses actually reach their subscribers.

```rust
/// Metric: confirmed / (confirmed + dropped) over the cohort window.
pub struct DeliveryRateLens;

impl DeliveryRateLens {
    pub fn compute(confirmed: u64, dropped: u64) -> f64 {
        let total = confirmed + dropped;
        if total == 0 { return 1.0; }
        confirmed as f64 / total as f64
    }
}
```

**Data source**: Bus internal instrumentation. The Bus publishes delivery confirmations and drop events on internal topics.

### 5.5 HDC Diversity Lens

Cognitive diversity measured as distance across cohort HDC fingerprint clouds. If every agent's working set converges, the cohort is over-coupled and brittle.

```rust
/// Metric: mean pairwise cosine distance across agent centroid fingerprints.
///   Diversity = 1.0 - mean_pairwise_similarity. Range: [0.0, 1.0].
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

**Data source**: Every Signal carries an HDC fingerprint ([01-SIGNAL](01-SIGNAL.md)). The Lens groups recent Signals by author, computes per-agent centroids via HDC bundle, then measures pairwise distance.

### 5.6 Learned Weight Composition

The five process variables are combined into a single scalar via learned weights fitted online from cohort outcomes.

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
    pub fn dot(&self, m: &CohortMetrics) -> f64 {
        self.turn_taking * m.turn_taking_entropy
            + self.peer_prediction * m.peer_prediction_accuracy
            + self.citation_reciprocity * m.citation_reciprocity
            + self.delivery_rate * m.delivery_rate
            + self.hdc_diversity * m.hdc_diversity
            + self.bias
    }
}

/// Online learner: gradient step toward observed outcome quality.
pub struct CohortWeightsLearner {
    learning_rate: f64,
    window: VecDeque<(CohortMetrics, f64)>,
    max_window: usize,
}

impl CohortWeightsLearner {
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

### 5.7 C-Factor Components Summary

| Component | What it measures | Why it matters |
|---|---|---|
| **Turn-taking entropy** | Shannon entropy of agent turn distribution | High entropy = balanced participation. Low = one agent dominates. |
| **Peer prediction accuracy** | How well agents predict each other's output | High accuracy = shared mental model. Measured via predict-publish-correct on inter-agent Signals. |
| **Citation reciprocity** | Balance of knowledge attribution flow | Reciprocal citation = genuine knowledge exchange. One-way flow = parasitic consumption. |
| **Bus delivery rate** | Whether Pulses actually reach subscribers | Low rate = infrastructure bottleneck or capability restrictions prevent communication. |
| **HDC diversity** | Spread of episode fingerprints across agents | High diversity = agents explore different solution regions. Low = redundant work. |

### 5.8 C-Factor as L4 Gate

The c-factor is not just a metric -- it is a gate. L4 structural adaptation ([doc-07](07-LEARNING.md)) only evolves configurations that increase genuine collective intelligence. A configuration that improves individual metrics but decreases c-factor is rejected. This prevents optimization pressure from collapsing agent diversity. The c-factor is a **covariate, not an objective** -- optimizing c directly can be gamed (Woolley et al. 2010).

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

---

## 6. Lens Composition

Lenses compose in three ways: stacking, chaining, and scoping. All three are configured via TOML and resolved at Graph-load time.

### 6.1 Stacking (multiple Lenses on same target)

Multiple Lenses observe the same Cell, Graph, Agent, or Space simultaneously. Each receives the same events independently. There is no ordering dependency between stacked Lenses.

### 6.2 Chaining (Lens observes another Lens's output)

A Lens can observe another Lens's output Signals rather than raw lifecycle events. This enables derived metrics.

```
CostLens ---> TrendLens (cost $) ---> AnomalyLens
  emits          emits                   emits
CostReport     Trend                   Anomaly
 Signals       Signals                 Signals
```

Chaining uses `LensScope::Lens(ref)`. The engine ensures delivery order: upstream Lens output is delivered to downstream Lens before the next event cycle.

### 6.3 Scoping (Cell, Graph, Agent, Space levels)

| Scope | Receives events from | Aggregation |
|---|---|---|
| `Cell` | Single Cell only | Per-invocation metrics |
| `Graph` | All Cells within one Graph | Aggregated across nodes |
| `Agent` | All Graphs within one Agent | Cross-pipeline view |
| `Space` | All Agents within one Space | Full workspace view |
| `Global` | Everything in the system | System-wide overview |

### Composition Rules

1. **Independence**: Stacked Lenses do not affect each other. Failure in one does not interrupt others.
2. **Ordering**: Chained Lenses are topologically sorted. Cycles are rejected at Graph-load time.
3. **Scope narrowing**: A Lens at Graph scope does not receive events from Cells in other Graphs.
4. **Scope widening**: A Lens at Space scope receives events from all contained Agents/Graphs/Cells.
5. **Cross-scope chaining**: A Graph-scoped CostLens can chain into a Space-scoped TrendLens.

---

## 7. StateHub -- The Projection Layer

StateHub is the universal projection layer between Lens output and all consumer surfaces. It subscribes to Lens observation Signals, maintains typed projection state, and publishes versioned snapshots that surfaces subscribe to.

### Why StateHub

Without StateHub, every surface (TUI, web dashboard, Slack bot, audit trail) would independently parse raw Lens output, compute its own aggregations, and maintain its own state. This leads to inconsistency (surfaces disagree on numbers) and duplication (same computation done N times for N surfaces).

StateHub centralizes the computation once and projects the result to all consumers through typed, versioned contracts.

### Core Design

```rust
pub struct StateHub {
    pub projections: BTreeMap<ProjectionId, ProjectionState>,
    pub subscribers: Vec<SurfaceSubscription>,
    pub bus: BusHandle,
}

pub struct ProjectionState {
    pub id: ProjectionId,
    pub schema: TypeSchema,
    pub version: u64,                    // monotonic, incremented on update
    pub data: Value,                     // typed payload matching schema
    pub updated_at: DateTime<Utc>,
    pub source_lenses: Vec<LensRef>,
}
```

### 7 Core Projections

| Projection | Type | Source Lenses | Consumers |
|---|---|---|---|
| `cohort_health` | `CohortHealthProjection` | EfficiencyLens, ErrorLens, BudgetLens | TUI (F1), Web dashboard |
| `active_tasks` | `ActiveTasksProjection` | QualityLens, LatencyLens | TUI (F2: Tasks), Workbench surface |
| `gate_pipeline` | `GatePipelineProjection` | QualityLens | TUI (F3: Gates), Audit trail |
| `cost_meter` | `CostMeterProjection` | CostLens, BudgetLens | TUI (F5: Telemetry), Web |
| `knowledge_health` | `KnowledgeHealthProjection` | DriftLens | TUI (F6: Knowledge), Web |
| `c_factor` | `CFactorProjection` | CollectiveIntelligenceLens | TUI (F7), L4 gate, Web |
| `agent_vitality` | `AgentVitalityProjection` | BudgetLens, EfficiencyLens | TUI (F4: Agents), Slack alerts |

### Projection Schemas

```rust
pub struct CohortHealthProjection {
    pub agent_count: usize,
    pub active_count: usize,
    pub avg_vitality: f64,
    pub avg_pass_rate: f64,
    pub total_spend_usd: f64,
    pub error_rate: f64,
    pub t0_hit_rate: f64,
    pub regime_distribution: BTreeMap<Regime, usize>,
}

pub struct ActiveTasksProjection {
    pub tasks: Vec<TaskSnapshot>,
    pub queued: usize,
    pub completed_last_hour: usize,
    pub avg_task_duration: Duration,
}

pub struct GatePipelineProjection {
    pub rungs: Vec<RungSnapshot>,
    pub overall_pass_rate: f64,
    pub avg_reward: f64,
    pub hard_criteria_fail_rate: f64,
}

pub struct CostMeterProjection {
    pub total_usd: f64,
    pub budget_remaining: f64,
    pub burn_rate_usd_per_hour: f64,
    pub projected_exhaustion: Option<DateTime<Utc>>,
    pub model_breakdown: BTreeMap<String, f64>,
    pub cost_trend: TrendDirection,
}

pub struct KnowledgeHealthProjection {
    pub total_entries: u64,
    pub tier_distribution: BTreeMap<Tier, u64>,
    pub avg_balance: f64,
    pub cold_entries: u64,
    pub heuristic_count: u64,
    pub heuristic_avg_calibration: f64,
    pub anti_knowledge_count: u64,
}

pub struct CFactorProjection {
    pub c_factor: f64,
    pub components: CFactorComponents,
    pub trend: TrendDirection,
    pub agent_diversity: f64,
}

pub struct AgentVitalityProjection {
    pub agents: Vec<AgentVitalitySnapshot>,
}

pub struct AgentVitalitySnapshot {
    pub name: String,
    pub vitality: f64,
    pub phase: VitalityPhase,
    pub regime: Regime,
    pub slots_active: usize,
    pub slots_total: usize,
    pub tasks_completed: u64,
    pub current_task: Option<String>,
}
```

### Projection Updates

StateHub updates projections on every Lens observation Signal. Each update:

1. Applies the observation to the relevant projection(s).
2. Increments the projection `version`.
3. Publishes a Pulse on `statehub:{projection_id}:updated` with the new version number.
4. Subscribed surfaces receive the Pulse and fetch the updated projection.

Surfaces can subscribe at different resolutions:

| Resolution | Use case |
|---|---|
| `100ms` | Real-time TUI sparklines |
| `1s` | Web dashboard charts |
| `10s` | Low-bandwidth monitoring |
| `60s` | Slack notifications, historical review |

### StateHub as Data Contracts for Surfaces

The five named surfaces in [doc-20](20-SURFACES.md) consume StateHub projections:

| Surface | Projections consumed |
|---|---|
| **Workbench** | `active_tasks`, `gate_pipeline`, `cost_meter` |
| **Agent Inbox** | `agent_vitality`, `cohort_health` |
| **Generative Canvas** | `active_tasks`, `gate_pipeline` |
| **Stigmergy Minimap** | `c_factor`, `cohort_health`, `knowledge_health` |
| **Autonomy Slider** | `agent_vitality`, `c_factor` |

Surfaces never read raw Lens output. The projection schemas are the stable API between telemetry and UX.

---

## 8. Dashboard Plumbing

The dashboard (TUI, web, or visual editor) receives Lens output via StateHub projections, streamed over WebSocket or SSE.

### WebSocket Protocol

The control plane (`roko serve` on :6677) exposes a WebSocket endpoint for live telemetry:

```
GET /ws/telemetry
```

Client subscription message:

```json
{
    "subscribe": {
        "projections": ["cohort_health", "cost_meter", "c_factor"],
        "resolution": "1s"
    }
}
```

Server streams projection updates:

```json
{
    "projection": "cost_meter",
    "version": 1423,
    "timestamp": "2026-04-25T10:30:00Z",
    "data": {
        "total_usd": 0.0234,
        "budget_remaining": 0.9766,
        "burn_rate_usd_per_hour": 0.047,
        "cost_trend": "stable"
    }
}
```

### SSE Endpoint

For clients that cannot use WebSocket:

```
GET /api/statehub/stream?projections=cohort_health,cost_meter&resolution=1s
```

Returns `text/event-stream` with JSON payloads matching the WebSocket format.

### REST API

```
GET /api/telemetry/{lens-id}/history?from=...&to=...&resolution=60s
GET /api/statehub/{projection-id}
GET /api/statehub/{projection-id}/history?from=...&to=...
```

### TUI Rendering

The ratatui dashboard renders StateHub projections directly. Each F-key tab maps to one or more projections (e.g., F5: Telemetry uses `cost_meter` + `gate_pipeline`; F7 uses `c_factor`).

---

## 9. TOML Configuration

### Attaching Lenses to a Graph

```toml
[graph]
name = "plan-executor"

[[lenses]]
name = "cost-monitor"
block = "roko:cost-lens@^1.0"
scope = "graph"
[lenses.params]
interval = "60s"
budget_warn_pct = 0.80

[[lenses]]
name = "quality-monitor"
block = "roko:quality-lens@^1.0"
scope = "graph"
[lenses.params]
pass_rate_warn = 0.70
```

### Chaining Lenses

```toml
[[lenses]]
name = "cost-monitor"
block = "roko:cost-lens@^1.0"
scope = "graph"

[[lenses]]
name = "cost-trend"
block = "roko:trend-lens@^1.0"
scope = "lens:cost-monitor"
[lenses.params]
metric = "total_usd"
window = "600s"

[[lenses]]
name = "cost-anomaly"
block = "roko:anomaly-lens@^1.0"
scope = "lens:cost-trend"
[lenses.params]
sigma_moderate = 3.0
```

### StateHub Configuration

```toml
[statehub]
enabled = true
projection_interval = "1s"
history_retention = "7d"

[[statehub.projections]]
id = "cohort_health"
sources = ["efficiency-lens", "error-lens", "budget-lens"]

[[statehub.projections]]
id = "c_factor"
sources = ["collective-intelligence-lens"]
```

---

## 10. Performance

Lenses must not degrade the observed system. The overhead budget is strict.

### Overhead Budget

| Constraint | Limit | Enforcement |
|---|---|---|
| Per-event Lens invocation | < 1% of observed Cell's execution time | Runtime circuit breaker |
| Total Lens overhead per Cell | < 5% (all stacked Lenses combined) | Aggregated timing check |
| Memory per Lens | < 10 MB rolling window | OOM guard per Lens |
| Bus backpressure | Drop-oldest for observation topics | Prevents slow Lens from blocking |

### Circuit Breaker

If a Lens consistently exceeds its overhead budget:

1. **First violation**: Log warning, continue.
2. **3 consecutive violations**: Reduce invocation rate (sample 50% of events).
3. **10 consecutive violations**: Disable Lens, emit Alert(Critical) on system Bus, notify operator.
4. **Recovery**: Operator re-enables via config or API. Lens starts in sampled mode.

### Async Processing

Lenses run asynchronously -- they do not block the observed Cell's execution path. The engine captures the event (cheap clone or Arc), dispatches to the Lens async task pool, and continues Cell execution immediately.

---

## 11. Acceptance Criteria

| # | Criterion | Verification |
|---|---|---|
| T-1 | Observe trait compiles with `observe()`, `observes()`, `scope()` | `cargo check` |
| T-2 | ObservableEvent covers all lifecycle events in this spec | Enum variant count matches spec |
| T-3 | CostLens emits CostReport Signals per configured interval | Integration test |
| T-4 | LatencyLens computes correct p50/p95/p99 from duration samples | Unit test |
| T-5 | QualityLens tracks pass rate per rung and avg continuous reward | Integration test |
| T-6 | EfficiencyLens computes tokens-per-task with vitality phase | Unit test |
| T-7 | ErrorLens classifies errors by category | Unit test |
| T-8 | DriftLens detects balance decrease from demurrage | Integration test |
| T-9 | BudgetLens emits Alert at configured thresholds with vitality | Unit test |
| T-10 | TrendLens computes correct EMA and slope | Unit test |
| T-11 | AnomalyLens flags outliers at configured sigma | Unit test |
| T-12 | CollectiveIntelligenceLens computes c-factor from 5 sub-lenses | Integration test |
| T-13 | c-factor decreases when one agent dominates turns | Unit test |
| T-14 | Turn-taking entropy = 1.0 for uniform distribution, < 0.5 for single-dominant | Unit test |
| T-15 | Peer prediction accuracy degrades gracefully with no data (returns 0.5) | Unit test |
| T-16 | Citation reciprocity weighted by recency | Unit test |
| T-17 | HDC diversity = 0.0 for identical centroids, > 0.5 for orthogonal | Unit test |
| T-18 | CohortWeightsLearner converges toward outcome quality | Integration test |
| T-19 | Stacking: 3 Lenses on same Graph all receive events | Integration test |
| T-20 | Chaining: TrendLens receives CostLens output | Integration test |
| T-21 | Scope isolation: Graph Lens does not see events from other Graphs | Integration test |
| T-22 | StateHub updates projection on Lens observation Signal | Integration test |
| T-23 | StateHub projection version increments monotonically | Unit test |
| T-24 | Surface subscription at 1s resolution receives coalesced updates | Integration test |
| T-25 | StateHub projections match typed schemas | Schema validation test |
| T-26 | Dashboard WS streams StateHub projections at configured resolution | Integration test |
| T-27 | SSE endpoint delivers same data as WebSocket | Integration test |
| T-28 | Circuit breaker disables slow Lens after 10 violations | Unit test |
| T-29 | Lens overhead stays under 1% of Cell execution time | Benchmark |
| T-30 | C-factor AND outcome both must improve for L4 change retention | Integration test |

---

## 12. Cross-References

| Topic | Document | Section |
|---|---|---|
| Signal/Pulse duality | [doc-01](01-SIGNAL.md) | SS1-3 |
| Observe protocol definition | [doc-02](02-CELL.md) | SS3.7 |
| Predict-publish-correct | [doc-02](02-CELL.md) | SS3.10 |
| Verify redesign (continuous reward) | [doc-02](02-CELL.md) | SS3.3 |
| Vitality and behavioral phases | [doc-05](05-AGENT.md) | SS3 |
| CorticalState | [doc-05](05-AGENT.md) | SS4 |
| L4 c-factor gate | [doc-07](07-LEARNING.md) | SS5 |
| Demurrage model | [doc-06](06-MEMORY.md) | SS3 |
| Five named surfaces | [doc-20](20-SURFACES.md) | -- |
| Immune system AnomalyLens | [doc-16](16-SECURITY.md) | SS3 |
