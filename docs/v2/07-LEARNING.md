# 07 -- Learning Loops

> Four cybernetic loops at increasing timescales. Seven compounding mechanisms that make each unit of usage improve the next. C-factor as a Lens. All expressed as Loop Graphs of Cells processing Signals through Bus and Store.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal/Pulse, demurrage, HDC fingerprints), [02-CELL](02-CELL.md) (9 protocols, predict-publish-correct, Verify redesign, EFE routing), [03-GRAPH](03-GRAPH.md) (Graph wiring, TOML definition), [04-EXECUTION](04-EXECUTION.md) (Loop specialization, convergence conditions), [05-AGENT](05-AGENT.md) (EFE gating, vitality, somatic markers, CorticalState), [06-MEMORY](06-MEMORY.md) (Store, demurrage economics, Heuristics, AntiKnowledge, Resonator Networks)

**Pattern**: Loop (feedback edge from output back to input). Every structure in this document is a Loop Graph -- a Graph specialization where output feeds back to input.

---

## 1. Overview

Roko learns through four feedback loops, each operating at a different timescale with a different level of autonomy. All four are implemented as **Loop** specializations -- Graphs that feed output back to input -- using the same primitives as every other part of the system.

Learning is not a separate subsystem. It emerges from the **predict-publish-correct** pattern ([02-CELL](02-CELL.md)): every Cell publishes its prediction as a Pulse, reality publishes the outcome, a CalibrationPolicy joins them and computes error, and the Cell subscribes to its own error topic. This pattern is structural -- it uses the same Bus that carries heartbeats and gate verdicts. (Friston 2006, active inference made structural.)

| Loop | Name | Timescale | Autonomy | What It Adjusts |
|---|---|---|---|---|
| L1 | Parameter Tuning | Gamma (per-tick) | Fully automatic | Continuous params within declared bounds |
| L2 | Strategy Routing | Theta (per-task) | Fully automatic | Selection among pre-approved alternatives via EFE |
| L3 | Knowledge Consolidation | Delta (per-session) | Automatic + auditable | Compression of episodes into durable knowledge + hindsight relabeling |
| L4 | Structural Adaptation | Manual (per-approval) | Requires human approval | Changes to system structure, spec amendments |

Each loop is bounded by explicit safety constraints. Lower loops (L1, L2) operate within tight, pre-declared ranges. Higher loops (L3, L4) have broader scope but stricter oversight.

```
                            Increasing scope -->
                            Increasing timescale -->
                            Increasing oversight -->

    +------------+  +-----------------+  +----------------------+  +------------------------+
    | L1: Param  |  | L2: Strategy    |  | L3: Knowledge        |  | L4: Structural         |
    | Tuning     |  | Routing         |  | Consolidation        |  | Adaptation             |
    |            |  |                 |  |                      |  |                        |
    | gamma      |  | theta           |  | delta                |  | manual                 |
    | per-tick   |  | per-task        |  | per-session          |  | per-approval           |
    | automatic  |  | automatic       |  | auto + audit         |  | human approval         |
    +------------+  +-----------------+  +----------------------+  +------------------------+
```

---

## 2. Predict-Publish-Correct: The Structural Mechanism

Every operator in Roko is a learner. This is not a metaphor -- it is the literal mechanism by which all four loops update (Friston 2006, active inference).

### 2.1 The Pattern

```
Cell publishes prediction    ->    Pulse on "prediction.{operator}"
Reality publishes outcome    ->    Pulse on "outcome.{operator}"
CalibrationPolicy joins them ->    Pulse on "calibration.{operator}.updated"
Cell subscribes to error     ->    Updates its internal model
```

### 2.2 How It Works Per Loop

| Loop | What Predicts | What Is the Outcome | What Updates |
|---|---|---|---|
| **L1** | Gate threshold predicts pass/fail boundary | Actual gate verdict | EMA moves threshold toward observed optimal |
| **L2** | Router predicts best model for context | Task quality + cost + latency | EFE posterior updates belief about model quality per context |
| **L3** | Insight predicts task success when applied | Downstream gate pass/fail when Insight was in context | Demurrage balance refreshed on success, drained on failure |
| **L4** | Structural proposal predicts quality improvement | Observation window quality metrics | Proposal history informs future proposal generation |

### 2.3 Why Structural, Not Bolted-On

The predict-publish-correct pattern uses the same Bus that carries agent heartbeats, the same Pulse type that carries streaming output, and the same topic taxonomy that routes lifecycle events. There is no learning-specific infrastructure -- the Bus IS the learning fabric. This means:

1. **Every new Cell automatically participates in learning** -- it predicts, publishes, and can subscribe to corrections without any learning-specific code.
2. **Learning is observable** -- Lens Cells attach to `prediction.*` and `calibration.*` topics to track learning dynamics.
3. **Learning is auditable** -- graduated Signals from predictions and outcomes carry full lineage.

---

## 3. Loop 1: Parameter Tuning (Gamma)

**Timescale**: Per-tick (100ms to 2s)
**Autonomy**: Fully automatic
**Safety**: Parameters adjust only within declared `ParamRange` bounds
**Pattern**: Loop Graph with EMA feedback

### 3.1 Loop Graph Definition

```toml
[graph]
name = "l1-parameter-tuning"
loop = true
max_iterations = 10000
min_interval = "100ms"

[[nodes]]
id = "observe"
cell = "roko:param-observer"
protocol = "Observe"

[[nodes]]
id = "predict"
cell = "roko:param-predictor"
protocol = "Score"

[[nodes]]
id = "compare"
cell = "roko:param-comparator"
protocol = "Verify"

[[nodes]]
id = "adjust"
cell = "roko:param-adjuster"
protocol = "React"

[[edges]]
from = "observe"
to = "predict"

[[edges]]
from = "predict"
to = "compare"

[[edges]]
from = "compare"
to = "adjust"

[[edges]]
from = "adjust"
to = "observe"
condition = "NOT converged"
```

### 3.2 What It Adjusts

L1 tunes continuous-valued parameters that have declared safe ranges:

```rust
pub struct ParamRange {
    pub min: f64,
    pub max: f64,
    pub step: Option<f64>,
    pub default: f64,
    pub learning_rate: f64,
}
```

| Parameter | ParamRange | Learning Signal |
|---|---|---|
| Gate thresholds per rung | `[0.3, 0.95]` step 0.01 | Pass rate EMA over window |
| Prompt experiment weights | `[0.0, 1.0]` | Experiment outcome tracking |
| Model temperature per task type | `[0.0, 1.5]` step 0.05 | Gate pass/fail on model output |
| Adaptive clock regime thresholds | `[0.1, 0.8]` | Prediction error distribution |
| T0/T1/T2 EFE cost weights | `[0.01, 10.0]` | Agent efficiency (T0 hit rate vs quality) |
| Compose budget allocation weights | `[0.0, 1.0]` | Downstream quality of composed output |

### 3.3 EMA Update

L1 uses exponential moving average with directional adjustment:

```
new_value = current_value + learning_rate * (observed_optimal - current_value)
clamped to [param_range.min, param_range.max]
```

This is the predict-publish-correct pattern applied to continuous parameters: the current value is the prediction, the gate outcome is the observation, and the EMA update is the correction.

### 3.4 Safety: Auto-Rollback

L1 maintains a rolling quality window. If quality drops below a configured threshold after parameter adjustment:

1. Revert to the previous parameter value.
2. Halve the learning rate for this parameter.
3. Emit an `Alert(Warning)` Signal via the BudgetLens.
4. Log the rollback in `.roko/learn/param-rollbacks.jsonl`.

After `max_rollbacks` consecutive rollbacks, the parameter freezes at its current value and an `Alert(Critical)` Signal is emitted recommending human review.

```rust
pub struct L1SafetyBounds {
    pub param_ranges: BTreeMap<String, ParamRange>,
    pub max_adjustment_per_tick: f64,
    pub quality_window: usize,
    pub quality_floor: f64,
    pub max_consecutive_rollbacks: usize,
}
```

**Invariant**: After every L1 adjustment, `param_range.min <= value <= param_range.max`. Violation is a logic error that halts the loop.

---

## 4. Loop 2: Strategy Routing (Theta) -- EFE

**Timescale**: Per-task (750ms to 16s)
**Autonomy**: Fully automatic
**Safety**: Finite set of pre-approved alternatives only
**Pattern**: Loop Graph with Bayesian posterior feedback

### 4.1 Loop Graph Definition

```toml
[graph]
name = "l2-strategy-routing"
loop = true
max_iterations = 100000
min_interval = "750ms"

[[nodes]]
id = "context"
cell = "roko:routing-context"
protocol = "Observe"

[[nodes]]
id = "efe-select"
cell = "roko:efe-selector"
protocol = "Route"

[[nodes]]
id = "dispatch"
cell = "roko:strategy-dispatch"
protocol = "React"

[[nodes]]
id = "outcome"
cell = "roko:outcome-observer"
protocol = "Observe"

[[nodes]]
id = "update"
cell = "roko:posterior-updater"
protocol = "Score"

[[edges]]
from = "context"
to = "efe-select"

[[edges]]
from = "efe-select"
to = "dispatch"

[[edges]]
from = "dispatch"
to = "outcome"

[[edges]]
from = "outcome"
to = "update"

[[edges]]
from = "update"
to = "context"
condition = "session_active"
```

### 4.2 What It Adjusts

L2 selects between existing, pre-approved alternatives using **Expected Free Energy (EFE)** (Friston 2006), replacing the previous LinUCB bandit. Each alternative is evaluated by how much it reduces the agent's uncertainty (epistemic value) while advancing its goals (pragmatic value), conditioned on the current regime.

```rust
pub struct StrategySet {
    pub alternatives: Vec<StrategyAlternative>,
    pub efe_model: EFEModel,
    pub regime_context: Regime,
}

pub struct EFEModel {
    pub beliefs: BTreeMap<AlternativeId, BayesianPosterior>,
    pub regime_factors: BTreeMap<Regime, Vec<f64>>,
    pub exploration_budget: f64,
}
```

### 4.3 EFE vs LinUCB

| Property | EFE | LinUCB (previous) |
|---|---|---|
| **Model type** | Bayesian generative | Linear contextual bandit |
| **Exploration** | Information gain (epistemic value) -- principled | UCB confidence bound -- heuristic |
| **Cost awareness** | Native -- cost enters the free energy | External constraint only |
| **Regime conditioning** | First-class -- different priors per regime | Context feature (indirect) |
| **Timescale separation** | T0/T1/T2 naturally emerge from EFE bound evaluation | Separate gating logic required |

Each timescale corresponds to a different free-energy lower bound:

| Timescale | EFE Bound | What It Means |
|---|---|---|
| **T0 (gamma)** | Expected free energy under reflex policy | "Can I handle this without thinking?" |
| **T1 (theta)** | Expected free energy under lightweight model | "Can a quick analysis resolve this?" |
| **T2 (delta)** | Expected free energy under full deliberation | "Do I need deep reasoning here?" |

The gate decision ([05-AGENT](05-AGENT.md)) IS the L2 routing decision -- there is no separate gating system. The same EFE computation selects the tier and, within T1/T2, selects the specific model.

### 4.4 Regime Conditioning

The EFE model receives the current `regime: Regime` as a context signal from the CorticalState ([05-AGENT](05-AGENT.md)). Different regimes shift the prior beliefs:

- **Crisis regime**: epistemic value weighted higher (need information to resolve crisis)
- **Calm regime**: pragmatic value weighted higher (environment is stable, optimize for goals)
- **Volatile regime**: cost weighted higher (uncertain environment, avoid expensive mistakes)

### 4.5 CascadeRouter

The CascadeRouter is the concrete implementation of L2 strategy routing for model selection. It maintains Bayesian posteriors per model per domain, conditioned on regime. The EFE computation selects the optimal model for each task.

### 4.6 The `feedback()` Method

The Route protocol includes a `feedback()` method that is the learning signal for L2:

```rust
pub trait Route: Cell {
    async fn route(&self, candidates: &[Signal], ctx: &RouteContext) -> Result<RouteResult>;
    async fn feedback(&self, choice: &SignalRef, outcome: &Signal) -> Result<()>;
}
```

After each task completes, the orchestrator calls `feedback()` with the selected alternative and the outcome. The EFE model updates its Bayesian posteriors.

### 4.7 Safety: Pre-Approved Alternatives Only

L2 can never introduce a new alternative. It can only select among the set declared in configuration. Adding a new model or a new failure strategy requires human action (L4).

```rust
pub struct L2SafetyBounds {
    pub alternatives: Vec<String>,
    pub exploration_budget: f64,
    pub min_observations: usize,
    pub degradation_threshold: f64,
    pub max_efe_cost_ratio: f64,
}
```

**Invariant**: L2 never selects an alternative not in the `alternatives` set.

---

## 5. Loop 3: Knowledge Consolidation (Delta)

**Timescale**: Per-session (60s to 10m)
**Autonomy**: Automatic execution, auditable results
**Safety**: New knowledge starts Transient; promotion requires Verify passage; AntiKnowledge prevents known-bad rediscovery
**Pattern**: Loop Graph with four-phase dream cycle

### 5.1 Loop Graph Definition

```toml
[graph]
name = "l3-dream-consolidation"
loop = true
max_iterations = 100
min_interval = "60s"

[[nodes]]
id = "collect"
cell = "roko:episode-collector"
protocol = "Observe"

[[nodes]]
id = "nrem"
cell = "roko:nrem-replay"
protocol = "Score"

[[nodes]]
id = "hindsight"
cell = "roko:hindsight-relabeler"
protocol = "Score"

[[nodes]]
id = "rem"
cell = "roko:rem-imagination"
protocol = "Compose"

[[nodes]]
id = "integrate"
cell = "roko:knowledge-integrator"
protocol = "Store"

[[edges]]
from = "collect"
to = "nrem"

[[edges]]
from = "nrem"
to = "hindsight"

[[edges]]
from = "hindsight"
to = "rem"

[[edges]]
from = "rem"
to = "integrate"

[[edges]]
from = "integrate"
to = "collect"
condition = "session_active"
```

### 5.2 What It Does

L3 compresses raw episodes (agent turns, gate results, cost reports) into durable knowledge (Insights, Heuristics, CausalLinks, StrategyFragments). This is the dream cycle -- offline processing that runs between or alongside active work. L3 also performs **hindsight relabeling**, recovering value from failed trajectories.

### 5.3 Four Phases

#### Phase 1: NREM Replay

Cluster high prediction-error episodes and extract patterns. The system replays recent episodes, focusing on those with high prediction error (PE > 0.30) -- situations where the agent was surprised. Clustering by HDC similarity groups related surprises. For each cluster, a pattern extractor (LLM-driven Cell) produces an Insight Signal describing the common structure.

**Output**: `Signal { kind: Insight, tier: Transient }` with pattern description, cluster size, avg PE, supporting episode refs, and HDC fingerprint.

#### Phase 2: Hindsight Relabeling

Failed trajectories are decomposed into sub-goals, and achieved sub-goals are relabeled as positive episodes.

```
Failed trajectory (original goal: "implement auth + tests")
    |
    v
Sub-goal extraction: "auth implemented" (achieved), "tests written" (failed)
    |
    v
Relabel: trajectory is SUCCESSFUL for "implement auth"
    |
    v
Episode relabeled with achieved sub-goal -> enters NREM replay as positive data
```

**Why this matters**: Without hindsight relabeling, any trajectory that fails its overall gate is discarded. But most failed trajectories contain useful partial work -- code that compiled but did not pass tests, research that found relevant sources but did not synthesize them. Hindsight relabeling recovers this value.

**Recovery rate**: Recovers useful learning signal from at least 45% of otherwise-discarded episodes. The key insight is that "failure" is always relative to a specific goal -- the same trajectory may be a success for a different, simpler goal.

**Sub-goal extraction**: The system uses the task's dependency graph and intermediate gate results to identify which sub-goals were achieved.

#### Phase 3: REM Imagination

Generate counterfactual scenarios from high-value Insights. For each new Insight, the system generates counterfactual scenarios: "If the agent had used model X instead of model Y, what would have happened?" An LLM-driven Cell simulates the alternative and evaluates the projected outcome.

**Output**: `Signal { kind: StrategyFragment, tier: Transient }` with condition/action clauses, projected improvement, confidence, and lineage.

**Threat rehearsal** runs as a sub-phase: the system enumerates plausible threat scenarios from recent episodes and generates Warning Signals (ephemeral, published on Bus with short TTL).

#### Phase 4: Integration

Promote validated Insights and StrategyFragments through knowledge tiers. Tier promotion uses the demurrage model ([06-MEMORY](06-MEMORY.md)): Signals with high balance (actively used, recently reinforced) get promoted; Signals with low balance get demoted.

```
Transient Insights/Strategies
    |
    v
Verify protocol (gate check)
    |
    +-- Passed (3+ confirmations) -> Promote to Working
    +-- Passed (5+ across contexts) -> Promote to Consolidated
    +-- Failed -> Demote one tier, possibly create AntiKnowledge
```

### 5.4 AntiKnowledge Creation

When a validated Insight is later disproven (gate failures using knowledge derived from it), the system creates an AntiKnowledge Signal that actively repels future knowledge in the same HDC region. See [06-MEMORY](06-MEMORY.md).

### 5.5 Safety Bounds

```rust
pub struct L3SafetyBounds {
    pub max_entries_per_cycle: usize,
    pub verification_required: bool,
    pub min_confirmations_d1_d2: usize,   // 3+ to promote Transient -> Working
    pub min_confirmations_d2_d3: usize,   // 5+ to promote Working -> Consolidated
    pub anti_knowledge_threshold: f64,    // HDC similarity for rejection (0.9)
    pub max_tier_auto: Tier,              // highest tier reachable without human
    pub hindsight_min_subgoals: usize,    // min sub-goals achieved to relabel
}
```

**Invariant**: No knowledge entry reaches Persistent tier without human or consortium approval.

---

## 6. Loop 4: Structural Adaptation

**Timescale**: Per-approval (unbounded -- waits for human)
**Autonomy**: Requires human approval for every change
**Safety**: `RecursiveSafetyMonitor`, pre-change snapshot, auto-rollback on quality regression, c-factor gate

L4 is the only loop that modifies the system's own structure. It encompasses three mechanisms that share the same approval workflow.

### What L4 can change

```rust
/// The kinds of structural changes L4 can propose.
/// Each requires human approval before application.
pub enum StructuralChangeKind {
    /// Add, remove, or reconfigure a model in the CascadeRouter alternatives set.
    ModelConfig { model: String, action: ModelAction },

    /// Modify a Graph's topology (add/remove nodes, change edges).
    GraphTopology { graph: GraphRef, diff: GraphDiff },

    /// Add or remove a Cell from the CellRegistry.
    CellRegistration { cell: CellRef, action: RegistryAction },

    /// Modify Extension chain ordering or add/remove Extensions.
    ExtensionChain { agent: AgentId, diff: ExtensionDiff },

    /// Amend a specification document (spec-as-artifact).
    SpecAmendment { document: String, diff: String },

    /// Modify ParamRange bounds (widen or narrow L1's operating range).
    ParamRangeBounds { param: String, new_range: ParamRange },

    /// Modify Safety bounds (RecursiveSafetyMonitor thresholds).
    SafetyBounds { monitor: String, change: SafetyBoundsDiff },
}

impl StructuralChangeKind {
    /// Nesting depth: how many layers of structural change this affects.
    /// Used by RecursiveSafetyMonitor to enforce max_depth.
    pub fn depth(&self) -> u32 {
        match self {
            Self::ModelConfig { .. } => 1,         // leaf: single config change
            Self::ParamRangeBounds { .. } => 1,    // leaf: single param range
            Self::CellRegistration { .. } => 1,    // leaf: single Cell
            Self::ExtensionChain { .. } => 2,      // affects Agent + all its ticks
            Self::GraphTopology { .. } => 2,       // affects Graph + all Flows using it
            Self::SpecAmendment { .. } => 3,       // affects spec + all implementations
            Self::SafetyBounds { .. } => 3,        // affects safety + all loops it guards
        }
    }
}
```

### 6.1 HGM Metaproductivity

**Hierarchical Generative Model (HGM)** scores configuration variants by their descendant performance, not just direct performance. A configuration that produces high-quality children (through L3 knowledge consolidation) is scored higher than one that performs well in isolation (Minsky 1986, cf. *Society of Mind* on heterogeneous specialist interaction).

```
Configuration A        Configuration B
    |                      |
    +-- Child A1 (0.8)     +-- Child B1 (0.9)
    +-- Child A2 (0.7)     +-- Child B2 (0.3)
    +-- Child A3 (0.9)     +-- Child B3 (0.4)

    Score: 0.80             Score: 0.53
    A wins despite no single child beating B1
```

Metaproductivity measures a configuration's ability to **produce good descendants**, not just to be good itself. This selects for generative capacity.

### 6.2 CycleQD with HDC Behavioral Characterizations

**Quality-Diversity (QD)** search explores the space of system configurations, maintaining a diverse archive of high-quality variants. **CycleQD** adds a cyclical schedule that alternates between exploration (add new variants) and exploitation (refine best variants).

Behavioral characterizations use HDC fingerprints: each configuration variant is fingerprinted by its behavioral signature (which tasks it handles well, which gate rungs it passes, which models it selects). HDC similarity between fingerprints determines archive placement -- similar behaviors occupy the same archive cell, diverse behaviors occupy different cells.

### 6.3 Verify-as-Reward for Self-Play

The Verify protocol ([02-CELL](02-CELL.md)) serves as the reward function for L4 self-play:

1. A candidate agent runs with the proposed configuration.
2. A verifier agent (distinct from the candidate) evaluates the output using the Verify protocol.
3. The continuous reward (`Verdict.reward`) becomes the fitness signal for the evolutionary archive.

**Variance Inequality**: The verifier must be spectrally cleaner than the generator. In practice: the verifier uses a different model than the agent being evaluated, and verification Cells sit outside the modifiable surface. L4 cannot change the Verify Cells it is evaluated by.

### 6.4 C-Factor Gate

L4 only evolves configurations that increase genuine collective intelligence. The c-factor (section 9) -- computed from turn-taking entropy (Shannon entropy), peer prediction accuracy, citation reciprocity, and HDC diversity (Woolley et al. 2010) -- gates every structural change:

```
Proposed configuration change
    |
    v
Apply to sandbox environment
    |
    v
Run evaluation period
    |
    v
Measure c-factor (before and after)
    |
    +-- c-factor increased -> Proposal eligible for human review
    +-- c-factor decreased -> Auto-rejected (logged for analysis)
```

### 6.5 Spec-as-Artifact

The specification itself (these documents) is a mutable artifact in the L4 evolutionary archive:

1. **Readable by agents at startup** -- injected into system prompt context.
2. **Queryable as MCP tools** -- agents can `query_spec("what is demurrage?")`.
3. **Evolvable through L4** -- structural proposals can include spec amendments.
4. **Signed under ERC-8004** -- each spec version has verifiable provenance.

Spec amendments follow the same proposal workflow but with an additional constraint: the amended spec must score at least as well as the current spec on CMP (Comprehension-Maintainability-Precision) metrics.

### 6.6 RecursiveSafetyMonitor

All L4 proposals pass through the `RecursiveSafetyMonitor` before reaching the human:

```rust
pub struct RecursiveSafetyMonitor {
    pub max_depth: u32,                // max nesting of structural changes (default: 3)
    pub max_rate: Rate,                // max proposals per time window (default: 5/hour)
    pub quality_floor: f64,            // reject if system quality below this
    pub caveat_threshold: usize,       // max caveats before auto-reject (default: 3)
    pub c_factor_floor: f64,           // reject if c-factor drops below this
    pub snapshot_required: bool,       // always snapshot before applying (default: true)
}
```

### 6.7 Approval Workflow

```
L4 generates proposal
    |
    v
RecursiveSafetyMonitor checks bounds
    |
    +-- Rejected (bounds violated) -> Log, notify operator
    |
    v
c-factor gate (sandbox evaluation)
    |
    +-- c-factor decreased -> Auto-rejected
    |
    v
Proposal Signal emitted (kind: StructuralProposal)
    |
    v
Human reviews via dashboard / CLI / API
    |
    +-- Rejected -> Log rejection reason, feed to L3 as lesson
    +-- Approved -> Execute change
    |       |
    |       v
    |   Snapshot current state
    |       |
    |       v
    |   Apply structural change
    |       |
    |       v
    |   Monitor quality for observation_window
    |       |
    |       +-- Quality maintained -> Confirm change
    |       +-- Quality regressed -> Auto-rollback to snapshot
    |
    +-- Deferred -> Park for later review
```

### 6.8 Safety Bounds

```rust
pub struct L4SafetyBounds {
    pub require_human_approval: bool,
    pub recursive_safety_monitor: RecursiveSafetyMonitor,
    pub snapshot_before_change: bool,
    pub auto_rollback_on_regression: bool,
    pub observation_window: Duration,
    pub rollback_threshold: f64,
    pub c_factor_gate: bool,
    pub variance_inequality: bool,
    pub max_structural_changes_per_day: usize,
}
```

**Invariant**: No structural change is applied without a snapshot. No structural change persists if quality regresses beyond threshold or c-factor decreases.

---

## 7. Autonomy Levels

Roko defines six autonomy levels (Stafford Beer, Viable System Model; adapted for agent systems). Each learning loop operates at specific levels, and the operator can configure the maximum allowed level per Space.

| Level | Name | What the System Can Do | Loop |
|---|---|---|---|
| 0 | **Observe** | Read-only observation, emit Signals | All loops read |
| 1 | **Suggest** | Propose changes, wait for approval | L4 default |
| 2 | **Tune** | Adjust parameters within ParamRange | L1 |
| 3 | **Select** | Choose among pre-approved alternatives via EFE | L2 |
| 4 | **Consolidate** | Create, promote, demote knowledge entries | L3 |
| 5 | **Execute** | Apply structural changes (after approval) | L4 (with approval) |

### Configuration

```toml
[space.autonomy]
max_level = 4
l1_enabled = true
l2_enabled = true
l3_enabled = true
l4_enabled = false

[space.autonomy.l3]
auto_promote = true
max_tier = "consolidated"
dream_schedule = "0 */6 * * *"

[space.autonomy.l4]
require_approval = true
max_proposals_per_hour = 3
auto_rollback = true
observation_window = "1h"
c_factor_floor = 0.4
```

---

## 8. Seven Compounding Loops

Beyond the four formal learning loops, seven compounding mechanisms connect learning to runtime behavior. Each creates a virtuous cycle and is expressed as a **Loop Graph**. Together they form the autocatalytic core (Kauffman 1993).

### 8.1 Loop C1: Demurrage-Weighted Retrieval

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
///      -tax for idle Signals (see 01-SIGNAL.md).
///   5. Store Cell persists updated balances.
///   6. FEEDBACK: next query benefits from sharper balance distribution.
/// Demurrage constants are defined in DemurrageConfig ([06-MEMORY](06-MEMORY.md) S3).
/// This struct references them; do not duplicate values here.
pub struct DemurrageRetrievalLoop {
    /// Values from DemurrageConfig in 06-MEMORY S3.
    pub config: Arc<DemurrageConfig>,
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

        // Tax all warm-tier Signals using DemurrageConfig values from 06-MEMORY S3
        for signal in store.warm_tier_iter() {
            let tax = self.config.flat_tax_per_day * elapsed_days;
            signal.balance -= tax;

            // Reinforce used Signals
            if retrieved.contains(&signal.hash) {
                signal.balance += self.config.retrieved_bonus;
                reinforced += 1;
            }
            if cited.contains(&signal.hash) {
                signal.balance += self.config.cited_bonus;
                reinforced += 1;
            }
            if gated.contains(&signal.hash) {
                signal.balance += self.config.gated_bonus;
            }
            if surprised.contains(&signal.hash) {
                signal.balance += self.config.surprised_bonus;
            }

            // Freeze depleted Signals
            if signal.balance < self.config.min_balance {
                store.move_to_cold_tier(signal.hash);
                frozen += 1;
            }
        }

        DemurrageTick { reinforced, frozen, retrieved: retrieved.len() as u64 }
    }
}
```

**Compounding mechanism**: More usage produces more reinforcement evidence. Better evidence improves the demurrage curve. A sharper demurrage curve makes retrieval more selective. More selective retrieval improves the quality of the next episode. The KPI is median tokens per task (should decrease monotonically for a given difficulty bucket).

### 8.2 Loop C2: Heuristic Calibration

Heuristics only compound if they can be falsified. A heuristic that never sees a counterexample does not get better; it just gets older. The predict-publish-correct pattern (section 2) makes falsification structural.

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

### 8.3 Loop C3: HDC Codebook Cleanup

HDC fingerprints turn similarity into a cheap cleanup operation. Every new episode, Verify result, and heuristic adds to the codebook. Resonator Networks ([06-MEMORY](06-MEMORY.md)) periodically factorize bundled HDC vectors to identify constituent patterns learned separately. When a bundle's constituents all exist independently at higher tiers, the bundle is pruned (Frady et al. 2020).

```rust
/// HDC codebook cleanup loop. Prunes redundant bundled vectors when their
/// constituents exist independently at equal or higher tiers.
pub struct HdcCodebookCleanupLoop {
    /// Minimum cosine similarity for a bundle to be considered redundant.
    pub redundancy_threshold: f64,    // default: 0.92
    /// Minimum tier of all constituents for a bundle to be prunable.
    pub min_constituent_tier: Tier,   // default: Working
    /// Maximum vectors to process per tick (bound cleanup cost).
    pub max_per_tick: usize,          // default: 100
}

impl HdcCodebookCleanupLoop {
    /// One tick: factorize bundles, prune redundant ones.
    /// Returns count of pruned vectors.
    pub fn tick(&self, store: &mut dyn Store) -> u64 {
        let bundles = store.query_bundled_vectors(self.max_per_tick);
        let mut pruned = 0u64;
        for bundle in bundles {
            let constituents = resonator_factorize(&bundle.vector);
            let all_independent = constituents.iter().all(|c|
                store.has_independent(c, self.min_constituent_tier)
            );
            if all_independent {
                store.prune_vector(bundle.hash);
                pruned += 1;
            }
        }
        pruned
    }
}
```

**Compounding mechanism**: More interactions improve codebook organization. Better organization means future queries collapse to the right Signal cluster on the first pass. The KPI is percentage of Compose prompts that hit HDC-clean cache on the first attempt (should asymptote toward 1.0).

### 8.4 Loop C4: C-Factor Feedback

C-factor measures cohort process quality (section 9). High c-factor cohorts produce higher-quality output, which produces better learning evidence for routing, demurrage tuning, and heuristic calibration. This is a three-loop reinforcement: c-factor rises, output quality rises, learning quality rises, c-factor rises again.

The constraint: c-factor is a **covariate, not the objective**. See section 9.7 for Goodhart defense.

```rust
/// C-factor feedback loop. Observes cohort c-factor, correlates with outcomes,
/// updates CohortWeights for the CollectiveIntelligenceLens.
pub struct CFactorFeedbackLoop {
    /// Learning rate for weight updates.
    pub learning_rate: f64,           // default: 0.01
    /// Minimum cohort completions before updating weights.
    pub min_observations: usize,      // default: 5
    /// The learner that updates weights from outcomes.
    pub learner: CohortWeightsLearner,
}

impl CFactorFeedbackLoop {
    /// One tick: correlate latest cohort c-factor with task outcomes,
    /// update weights. Returns whether weights changed.
    pub fn tick(
        &self,
        weights: &mut CohortWeights,
        metrics: &CohortMetrics,
        outcome_quality: f64,
    ) -> bool {
        self.learner.update(weights, metrics, outcome_quality);
        true
    }
}
```

### 8.5 Loop C5: Playbook Distillation

Episodes compress into playbooks, and playbooks compress into meta-playbooks. That is learning about learning. Once the corpus is large enough, the cost per distilled unit drops while the transfer value per unit rises.

```rust
/// Playbook distillation loop. Compresses recent episodes into reusable
/// playbook entries, and playbooks into meta-playbooks.
pub struct PlaybookDistillationLoop {
    /// Minimum episodes with the same error signature to form a playbook.
    pub min_cluster_size: usize,      // default: 3
    /// HDC similarity threshold for clustering episodes.
    pub cluster_threshold: f64,       // default: 0.75
    /// Maximum playbooks to distill per tick.
    pub max_per_tick: usize,          // default: 10
}

impl PlaybookDistillationLoop {
    /// One tick: cluster recent episodes, extract playbook entries.
    /// Returns count of new playbook entries created.
    pub fn tick(&self, episodes: &[Episode], store: &mut dyn Store) -> u64 {
        let clusters = hdc_cluster(episodes, self.cluster_threshold);
        let mut created = 0u64;
        for cluster in clusters.iter().filter(|c| c.len() >= self.min_cluster_size) {
            let playbook = extract_playbook_entry(cluster);
            store.put_playbook(playbook);
            created += 1;
        }
        created
    }
}
```

**Compounding mechanism**: The system no longer relearns the same structure from scratch. It learns the reusable shape of the work. The KPI is retroactive improvements per week from the Delta consolidation cycle.

### 8.6 Loop C6: Cross-Deployment Heuristic Commons

Once heuristics can be imported across deployments, each deployment contributes to a shared commons. The marginal cost of sharing is low, but the marginal value to other deployments is high. Knowledge Signals published on-chain create a shared commons. A heuristic learned by deployment A and validated by deployment B gets promoted faster than one validated only locally.

```rust
/// Cross-deployment heuristic commons loop. Exports local validated heuristics
/// to shared storage and imports externally validated ones.
pub struct HeuristicCommonsLoop {
    /// Minimum confidence for a heuristic to be exported.
    pub export_confidence_threshold: f64,   // default: 0.8
    /// Maximum heuristics to import per tick.
    pub max_import_per_tick: usize,         // default: 20
    /// Trust discount for externally sourced heuristics.
    pub external_trust_discount: f64,       // default: 0.5
}

impl HeuristicCommonsLoop {
    /// One tick: export validated locals, import relevant externals.
    /// Returns (exported, imported) counts.
    pub fn tick(
        &self,
        local_store: &mut dyn Store,
        commons: &dyn CommonsStore,
    ) -> (u64, u64) {
        let exported = self.export_validated(local_store, commons);
        let imported = self.import_relevant(local_store, commons);
        (exported, imported)
    }
}
```

**Compounding mechanism**: Each deployment contributes once but benefits many times. The KPI is first-task-after-install to success minutes (should decrease as commons grows).

### 8.7 Loop C7: Plugin Ecosystem

Plugins make capability portable. Each new plugin increases the value of Roko to users who need that capability, and each new user increases the value of building a plugin. Classic two-sided market (Khattab et al. 2023, DSPy-style optimization).

```rust
/// Plugin ecosystem feedback loop. Tracks plugin usage and correlates with
/// task outcomes to surface high-value plugins.
pub struct PluginEcosystemLoop {
    /// Minimum uses before a plugin's impact is assessed.
    pub min_uses: usize,                    // default: 10
    /// Window for measuring plugin impact.
    pub impact_window: Duration,            // default: 7 days
}

impl PluginEcosystemLoop {
    /// One tick: compute plugin impact scores from recent outcomes.
    /// Returns updated plugin rankings.
    pub fn tick(&self, usage: &PluginUsageLog, outcomes: &[Episode]) -> Vec<PluginRanking> {
        usage.plugins_with_min_uses(self.min_uses)
            .map(|p| PluginRanking {
                plugin_id: p.id,
                usage_count: p.uses,
                impact_score: correlate_with_outcomes(p, outcomes, self.impact_window),
            })
            .collect()
    }
}
```

**Compounding mechanism**: Network effects. The KPI is unique plugin count and unique plugin users.

---

## 9. C-Factor as Lens

C-factor measures collective intelligence across a cohort of agents. It is a **Lens Cell** ([02-CELL](02-CELL.md)) -- it observes, computes, publishes. It does not take action. It does not gate anything by itself. It is a sensor, not an actuator.

**Key design decision**: c-factor is a **covariate** -- an observable diagnostic that correlates with cohort quality -- not an objective to maximize. If you turn c-factor into a reward signal, the system will game it (section 9.7).

### 9.1 CollectiveIntelligenceLens

```rust
/// C-factor is a Lens Cell. It observes, computes, publishes. It does not act.
///
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

### 9.2 The Five Sub-Lenses

Each Woolley (2010) process variable becomes a concrete Lens Cell. Each subscribes to Bus topics and queries Store, computes a single scalar in `[0.0, 1.0]`, and publishes its result on a sub-topic.

#### TurnTakingEntropyLens

Turn-taking equality measures how evenly a cohort shares the conversational floor.

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

#### PeerPredictionLens

Social perceptiveness -- the ability to predict what another agent will say or do next. Agents publish `prediction.*` Pulses and the Bus carries `outcome.*` Pulses. The Lens joins them.

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

**Data source**: The predict-publish-correct pattern (section 2). The peer prediction Lens reuses this infrastructure but measures accuracy across agents, not within a single Cell.

#### CitationReciprocityLens

Trust calibration: when an agent cites a Signal that later survives Verify, trust increases. When the cited Signal fails Verify, trust decreases.

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

**Data source**: Store lineage graph. Every Signal records `parent_hashes` ([01-SIGNAL](01-SIGNAL.md)). The Lens walks lineage to find citations and joins them with Verify verdicts.

#### DeliveryRateLens

Channel openness -- whether intended Pulses actually reach their subscribers.

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

**Data source**: Bus internal instrumentation. The Bus publishes delivery confirmations and drop events on internal topics.

#### HdcDiversityLens

Cognitive diversity -- distance across cohort HDC fingerprint clouds.

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

**Data source**: Every Signal carries an HDC fingerprint ([01-SIGNAL](01-SIGNAL.md)). The Lens groups recent Signals by author, computes per-agent centroids via HDC bundle, then measures pairwise distance.

### 9.3 Learned Composition

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

The operational split:
- **CohortMetrics** explain (why did this cohort perform well or poorly?).
- **CohortWeights** adapt (which process variables matter most in this deployment?).
- **c-factor** reports (the scalar published to dashboards and telemetry).
- **c-score** predicts (the fitted model behind the scalar).

### 9.4 The C-Factor Loop: Measure, Gate, Evolve

C-factor participates in the L4 evolution loop (section 6). The rule: only evolve structural changes that increase c-factor. But c-factor alone does not trigger evolution -- it must coincide with improved task outcomes.

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

### 9.5 WisdomGate as Verify Cell

Before a consensus artifact is finalized, it should pass a **WisdomGate** -- a Verify protocol Cell that encodes the conditions under which group consensus is meaningful rather than merely loud (Surowiecki 2004).

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

### 9.6 Anti-Groupthink React Cells

Optimizing for cohort cohesion can overshoot into groupthink. Three structural countermeasures are implemented as **React protocol Cells** ([02-CELL](02-CELL.md)). React Cells subscribe to Signals and emit corrective actions.

#### Devil's Advocate React Cell

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

#### Outsider Injection React Cell

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

#### Minority Report Preservation React Cell

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

### 9.7 What Happens When Someone Optimizes C-Factor Directly?

**Failure mode 1: Easy-task routing.** The system routes easy tasks to cohorts with high c-factor, inflating the metric while the hard work goes unaddressed. Detection: compare c-factor with task difficulty distribution. If high-c-factor cohorts only see trivial tasks, the metric is gamed.

**Failure mode 2: Dissent suppression.** The system penalizes agents that lower turn-taking entropy by disagreeing. This raises c-factor but kills the diversity that makes c-factor meaningful. Detection: HDC diversity trending downward while c-factor rises. The WisdomGate should start failing.

**Failure mode 3: Prediction collusion.** Agents learn to predict each other accurately by converging to the same outputs, not by actually modeling each other. Detection: peer prediction accuracy rises while outcome diversity falls.

**Structural defense**: the AND condition in the L4 loop (section 9.4). C-factor must rise (or hold) together with outcome quality. Combined with the three anti-groupthink React Cells, this creates a system that is structurally resistant to c-factor Goodharting.

### 9.8 Cohort Extraction

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

---

## 10. The Feedback Graph

The seven compounding Loops are not independent. They feed each other. The feedback graph shows which Loops are inputs to which other Loops.

```
                +----> [C4: c-factor] ----+
                |                         |
                |                         v
[C1: demurrage] <--+-- [C2: heuristic] --+--> [C5: playbook]
     |              |       ^                      |
     v              |       |                      v
[C3: HDC cleanup] --+-------+              [C6: commons]
                                                   |
                                                   v
                                           [C7: plugin ecosystem]
```

### Edge Detail

| From | To | What Flows |
|---|---|---|
| C1 demurrage | C3 HDC cleanup | Sharper balance distribution improves codebook organization |
| C1 demurrage | C4 c-factor | Better retrieval produces better cohort outcomes |
| C2 heuristic | C1 demurrage | Calibrated heuristics inform which Signals to reinforce |
| C2 heuristic | C4 c-factor | Better predictions improve peer-prediction accuracy |
| C3 HDC cleanup | C2 heuristic | Cleaner codebook improves similarity-based prediction |
| C3 HDC cleanup | C5 playbook | Cleaner fingerprints improve distillation quality |
| C4 c-factor | C1 demurrage | High-c-factor cohorts produce Signals worth reinforcing |
| C4 c-factor | C2 heuristic | c-factor as context feature improves routing decisions |
| C5 playbook | C6 commons | Distilled playbooks are the currency of the commons |
| C6 commons | C1 demurrage | Imported heuristics populate the Store with high-value Signals |
| C6 commons | C7 plugin | Richer commons attract more users, justifying more plugins |
| C7 plugin | C6 commons | More plugins increase the value of sharing heuristics across them |

---

## 11. The Autocatalytic Condition (Kauffman 1993)

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

**Current status**: Loops C1-C5 are strongly connected via the edges in section 10. C6 (commons) and C7 (plugin) are phase-2 and not yet connected in the running system. The autocatalytic condition holds for the inner five Loops but not yet for the full seven.

---

## 12. KPI Panel as Lens Cells

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
    /// Measures: C1 (demurrage), C3 (HDC), C5 (playbook).
    /// Expected curve: monotonic decrease.
    MedianTokensPerTask,

    /// Percentage of Compose prompts hitting HDC-clean cache first try.
    /// Measures: C3 (HDC cleanup).
    /// Expected curve: asymptote toward 1.0.
    HdcCacheHitRate,

    /// Mean calibration confidence interval width per heuristic.
    /// Measures: C2 (heuristic calibration).
    /// Expected curve: decrease with trials.
    MeanCalibrationWidth,

    /// Percentage of heuristics sourced from commons.
    /// Measures: C6 (cross-deployment commons).
    /// Expected curve: increase, then stabilize.
    CommonsHeuristicFraction,

    /// C-factor on randomly sampled cohorts.
    /// Measures: C4 (c-factor feedback).
    /// Expected curve: stable or rising.
    CFactorSampled,

    /// Dream-cycle retroactive improvements per week.
    /// Measures: C5 (playbook distillation).
    /// Expected curve: growth with corpus size.
    DeltaImprovementsPerWeek,

    /// Unique plugin count.
    /// Measures: C7 (plugin ecosystem).
    UniquePlugins,

    /// First-task-after-install to success minutes.
    /// Measures: C6 (cross-deployment commons).
    /// Expected curve: decreases as commons grows.
    FirstTaskMinutes,
}
```

---

## 13. Anti-Metrics as Verify Cells

Three numbers should NOT increase. If they do, the system is accumulating complexity without compounding value. Each anti-metric is a **Verify protocol Cell** that emits a failing verdict when the anti-metric drifts upward.

### 13.1 Warm-Tier Signal Count Stability

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
```

### 13.2 Unconfirmed Heuristic Count

```rust
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
```

### 13.3 Mean Lineage Depth

```rust
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

## 14. Demurrage x HDC Complete Compounding Loop

The full demurrage x HDC self-trimming Memory Loop, end to end.

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

## 15. What Breaks the Autocatalytic Cycle?

Single points of failure in the feedback graph. If any Loop stops producing its output, the downstream Loops lose their input and the cycle breaks.

| Failure Point | What Breaks | Detection | Recovery |
|---|---|---|---|
| **Demurrage miscalibration** | Tax too high -> Store empties. Tax too low -> Store bloats, retrieval degrades. | Anti-metric Verify Cells (section 13) | L1 auto-rollback restores previous tax rate |
| **Heuristic stagnation** | No falsifiers -> heuristics stop improving -> routing degrades | Mean calibration width plateaus | Importance sampling: deliberately test boundary cases |
| **HDC codebook saturation** | Codebook full of noise -> cleanup produces noise | HDC cache hit rate stops rising | Cold-tier eviction of low-balance codebook entries |
| **C-factor Goodharting** | C-factor gamed -> routing serves easy work -> real outcomes degrade | c-factor AND outcome divergence | The AND condition in L4 (section 9.4) |
| **Playbook staleness** | Playbooks not updated -> obsolete advice -> gate failures | Delta improvements per week drops to zero | Demurrage on playbook Signals; stale playbooks lose balance |
| **Commons poisoning** | Imported heuristics are wrong -> corrupts local calibration | First-task-after-install INCREASES | Quarantine imported heuristics until locally validated |
| **Bus partition** | Bus delivery drops -> Loops cannot communicate -> cycle fragments | Delivery rate Lens drops below threshold | Circuit breaker + Bus health alert |

The most dangerous failure mode is **silent degradation**: a Loop continues to produce output, but the output quality declines slowly. The anti-metric Verify Cells are designed to catch this by monitoring the numbers that should NOT increase.

---

## 16. Cybernetic Foundations

| Cybernetic Principle | Unified Mapping |
|---|---|
| Ashby's Law of Requisite Variety | The number of Loop types must match the number of failure modes. Seven Loops for seven sources of improvement. |
| Conant-Ashby (Good Regulator) | The KPI panel (section 12) IS the system's model of its own learning dynamics. Self-regulation requires self-observation. |
| Beer's Viable System Model | The Loops operate at three timescales (L1-L2 at gamma/theta, L3-L5 at delta, L6-L7 at deployment) matching Beer's recursive viability. |
| Kauffman's Autocatalytic Sets | The connected cycle condition (section 11). The system compounds when every Loop's inputs are produced by the network. |

---

## 17. Cross-Loop Safety

1. **No circular self-amplification**: L4 cannot approve its own proposals. L3 knowledge about "L1 should be more aggressive" is a suggestion, not a command.
2. **Monotonic quality**: Global quality metric tracked. If it drops below a floor, all loops above L0 pause.
3. **Audit trail**: Every loop action logged as Episode Signal with lineage.
4. **Variance Inequality**: The verifier is always spectrally cleaner than the generator. L4 proposals evaluated by Cells outside the modifiable surface.

---

## 18. Implementation as Graphs

Each loop is a Graph (specifically, a Loop specialization). The loops are literally defined as TOML Graphs with feedback edges (see sections 3.1, 4.1, 5.1 for concrete definitions).

### Loop Graph Conventions

| Convention | Requirement |
|---|---|
| `loop = true` | Declares the Graph as a Loop specialization |
| Feedback edge | At least one edge from downstream node back to upstream node |
| Convergence condition | Feedback edge has a `condition` (e.g., `NOT converged`, `session_active`) |
| Max iterations | `max_iterations` field prevents infinite loops |
| Min interval | `min_interval` between iterations prevents busy-looping |

### Why Graphs?

1. **Composability**: Loops use the same Cells, scoring, routing, and verification as task Graphs.
2. **Observability**: Lens Cells attach to Loop Graphs. CostLens tracks how much learning costs.
3. **Resumability**: Loop Graphs checkpoint like any other Flow. A crashed dream cycle resumes.
4. **Testability**: Loop Graphs are testable with the same infrastructure.

---

## 19. Acceptance Criteria

| # | Criterion | Verification |
|---|---|---|
| LL-1 | L1 adjusts gate threshold within ParamRange after gate outcome | Integration test |
| LL-2 | L1 auto-rollback triggers when quality drops below floor | Unit test |
| LL-3 | L1 freezes parameter after max_rollbacks | Unit test |
| LL-4 | L2 EFE model selects model given context and regime | Unit test |
| LL-5 | L2 Router.feedback() updates Bayesian posteriors | Unit test |
| LL-6 | L2 falls back to default when all alternatives degrade | Unit test |
| LL-7 | L2 never selects an alternative outside the configured set | Unit test |
| LL-8 | L2 regime conditioning shifts selection -- Crisis biases T2 | Unit test |
| LL-9 | L3 NREM clusters high-PE episodes and produces Insights | Integration test |
| LL-10 | L3 hindsight relabeling recovers sub-goals from failed trajectory | Integration test |
| LL-11 | L3 hindsight recovery rate >= 45% of discarded episodes | Statistical test over 100 episodes |
| LL-12 | L3 REM generates StrategyFragments from Insights | Integration test |
| LL-13 | L3 promotes Insight from Transient to Working after 3 confirmations | Integration test |
| LL-14 | L3 creates AntiKnowledge when Insight is disproven | Integration test |
| LL-15 | L3 four-phase order: NREM -> Hindsight -> REM -> Integration | Integration test |
| LL-16 | L4 proposal passes through RecursiveSafetyMonitor | Unit test |
| LL-17 | L4 c-factor gate rejects proposals that decrease c-factor | Integration test |
| LL-18 | L4 Variance Inequality enforced: verifier != generator model | Unit test |
| LL-19 | L4 metaproductivity scores by descendant performance | Unit test |
| LL-20 | L4 CycleQD maintains diverse archive via HDC fingerprints | Integration test |
| LL-21 | L4 Verify-as-reward uses continuous Verdict.reward | Integration test |
| LL-22 | L4 spec-as-artifact: spec queryable as MCP tool | Integration test |
| LL-23 | L4 spec amendment requires CMP scoring + human approval | Workflow test |
| LL-24 | L4 snapshots state before structural change | Integration test |
| LL-25 | L4 auto-rollback triggers on quality regression | Integration test |
| LL-26 | Autonomy levels enforced: L3 cannot perform L4 actions | Unit test |
| LL-27 | All loop actions logged as Episode Signals with lineage | Integration test |
| LL-28 | Global quality floor pauses all loops when breached | Integration test |
| LL-29 | Predict-publish-correct pattern observable on Bus topics | Integration test |
| LL-30 | Each loop representable as a Loop Graph (TOML) | Graph load test |
| LL-31 | C-factor Lens publishes scalar on telemetry Bus | Integration test |
| LL-32 | Five sub-lenses compute independently and combine via learned weights | Unit test |
| LL-33 | CohortWeightsLearner converges on synthetic cohort data | Statistical test |
| LL-34 | WisdomGate rejects consensus when Surowiecki conditions unmet | Unit test |
| LL-35 | Devil's Advocate fires when diversity < 0.25 and entropy high | Unit test |
| LL-36 | Outsider Injection fires after 3 consecutive convergent windows | Unit test |
| LL-37 | Minority Report reduces demurrage on dissenting Signals | Unit test |
| LL-38 | Goodhart guard rejects changes where c-factor rises but outcomes fall | Integration test |
| LL-39 | Anti-metric Verify Cells fire on warm-tier growth > 5%/week | Unit test |
| LL-40 | Kauffman condition check detects orphan Loops | Unit test |
| LL-41 | Kauffman condition check detects fragmented graph | Unit test |
| LL-42 | KPI Lens Cells publish on correct telemetry topics | Integration test |
| LL-43 | Demurrage x HDC tick reinforces retrieved + gated Signals | Unit test |
| LL-44 | Demurrage x HDC tick freezes depleted Signals to cold tier | Unit test |

---

## 20. Citations

| Citation | Reference |
|---|---|
| Friston 2006 | Karl Friston, "A free energy principle for the brain," *Journal of Physiology - Paris*, 100(1-3), 70-87, 2006. Active inference and predict-publish-correct. |
| Kauffman 1993 | Stuart Kauffman, *The Origins of Order*, Oxford University Press, 1993. Autocatalytic sets and self-organization. |
| Woolley et al. 2010 | Anita Woolley et al., "Evidence for a collective intelligence factor in the performance of human groups," *Science*, 330(6004), 686-688, 2010. C-factor. |
| Surowiecki 2004 | James Surowiecki, *The Wisdom of Crowds*, Doubleday, 2004. Four conditions for wise crowds. |
| Gesell 1916 | Silvio Gesell, *The Natural Economic Order*, 1916. Demurrage as holding cost. |
| Minsky 1986 | Marvin Minsky, *The Society of Mind*, Simon & Schuster, 1986. Heterogeneous specialist interaction. |
| Beer 1972 | Stafford Beer, *Brain of the Firm*, Allen Lane, 1972. Viable System Model. |
| Frady et al. 2020 | E. P. Frady et al., "A theory of sequence indexing and working memory in recurrent neural networks," *Neural Computation*, 2020. Resonator Networks. |
| Khattab et al. 2023 | Omar Khattab et al., "DSPy: Compiling declarative language model calls into self-improving pipelines," 2023. Optimization through usage data. |
| Kanerva 2009 | Pentti Kanerva, "Hyperdimensional computing: An introduction to computing in distributed representation with high-dimensional random vectors," *Cognitive Computation*, 2009. |
| Jonas 1966 | Hans Jonas, *The Phenomenon of Life*, Northwestern University Press, 1966. Mortality as motivator. |

---

## 21. Cross-References

| Topic | Document | Section |
|---|---|---|
| Signal/Pulse duality, demurrage | [01-SIGNAL](01-SIGNAL.md) | SS1-6 |
| Predict-publish-correct | [02-CELL](02-CELL.md) | Protocols |
| Verify redesign (continuous reward, evidence) | [02-CELL](02-CELL.md) | Verify |
| EFE Route protocol | [02-CELL](02-CELL.md) | Route |
| EFE gating in agent pipeline | [05-AGENT](05-AGENT.md) | Cognitive loop |
| Vitality phases | [05-AGENT](05-AGENT.md) | Lifecycle |
| CognitiveWorkspace (VCG, section effects) | [05-AGENT](05-AGENT.md) | Compose |
| Demurrage model | [06-MEMORY](06-MEMORY.md) | Demurrage |
| Heuristic lifecycle | [06-MEMORY](06-MEMORY.md) | Heuristics |
| Resonator Networks | [06-MEMORY](06-MEMORY.md) | HDC |
| AntiKnowledge | [06-MEMORY](06-MEMORY.md) | AntiKnowledge |
| CascadeRouter fallback chain | [08-GATEWAY](08-GATEWAY.md) | SS10 |
| CaMeL IFC, 5-head corrigibility | [16-SECURITY](16-SECURITY.md) | -- |
| On-chain knowledge commons | [22-REGISTRIES](22-REGISTRIES.md) | -- |
