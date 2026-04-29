# 02 — Cell and Protocols

> The universal computation unit. Signals in, Signals out. Declares typed I/O, capabilities, and protocol conformance. Every Cell is a learner via predict-publish-correct (Friston 2006).

**Subsumes**: Module, Tool, Gate, Router, Composer, Scorer, Policy, Substrate, Connector, Operator.

---

## 1. Cell Trait

A Cell is atomic computation with an identity, typed inputs and outputs, declared capabilities, protocol conformance, and cost estimation. Every first-class primitive in Roko is a Cell or composed of Cells.

```rust
/// The universal computation unit.
///
/// A Cell declares what it consumes, what it produces, what protocols it
/// speaks, and what capabilities it requires. The runtime uses these
/// declarations for type-checking edges, capability intersection, cost
/// budgeting, and protocol dispatch.
pub trait Cell: Send + Sync + 'static {
    /// Stable identifier. Content-addressed from (name, version, author).
    fn id(&self) -> CellId;

    /// Human-readable name.
    fn name(&self) -> &str;

    /// Semantic version.
    fn version(&self) -> Version;

    /// Typed input schema. `None` means "accepts any Signal."
    fn input_schema(&self) -> Option<&TypeSchema>;

    /// Typed output schema. `None` means "produces any Signal."
    fn output_schema(&self) -> Option<&TypeSchema>;

    /// Capabilities this Cell requires to run (see S3).
    fn capabilities(&self) -> &Capabilities;

    /// Which of the 9 protocols this Cell conforms to.
    fn protocols(&self) -> &[ProtocolId];

    /// Estimated cost to execute once, in USD-equivalent microcents.
    /// Used by the Route protocol for EFE cost terms and by Compose
    /// for budget-constrained assembly. Returns `None` if cost is
    /// input-dependent and cannot be estimated statically.
    fn estimated_cost(&self) -> Option<Cost>;

    /// Estimated wall-clock time to execute once.
    fn estimated_duration(&self) -> Option<Duration>;

    /// Execute the Cell. Consumes input Signals, produces output Signals.
    /// The runtime supplies a `CellContext` with Bus access, Store handle,
    /// budget remaining, trace context, and cancellation token.
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError>;
}
```

### CellId

```rust
/// Content-addressed Cell identity.
/// Deterministic: same (name, version, author) -> same CellId.
pub struct CellId(pub ContentHash);

impl CellId {
    pub fn compute(name: &str, version: &Version, author: &Author) -> Self {
        let mut hasher = sha2::Sha256::new();
        hasher.update(name.as_bytes());
        hasher.update(version.to_string().as_bytes());
        hasher.update(author.to_string().as_bytes());
        Self(ContentHash(hasher.finalize().into()))
    }
}
```

### CellContext

```rust
/// Runtime context provided to every Cell execution.
pub struct CellContext {
    /// Access to Bus for publishing Pulses (predictions, lifecycle events).
    pub bus: Arc<dyn Bus>,

    /// Access to Store for reading/writing Signals.
    pub store: Arc<dyn Store>,

    /// Remaining budget for this execution scope (Graph or Agent).
    pub budget_remaining: Cost,

    /// Distributed trace context for observability.
    pub trace: TraceContext,

    /// Cancellation token -- checked between steps.
    pub cancel: CancellationToken,

    /// The Cell's own calibration table (for predict-publish-correct).
    pub calibration: Arc<CalibrationTable>,

    /// Current Agent identity if running inside an Agent scope.
    pub agent: Option<AgentId>,

    /// Run ID if running inside a Flow.
    pub run_id: Option<RunId>,

    /// Lock-free shared perception surface. Defined in [05-AGENT](05-AGENT.md) S4.
    /// Available inside Agent Hot Graphs; `None` for standalone Flows.
    pub cortical: Option<Arc<CorticalState>>,
}
```

---

## 2. The Nine Protocols

Every Cell conforms to one or more of 9 protocols. Each protocol is an `async_trait` with well-typed signatures. All protocols support **predict-publish-correct** (see [S8](#8-predict-publish-correct)).

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProtocolId {
    Store,
    Score,
    Verify,
    Route,
    Compose,
    React,
    Observe,
    Connect,
    Trigger,
}
```

---

### 2.1 Store Protocol

Persisted storage of Signals. Defined in [doc-01](01-SIGNAL.md) S14.

```rust
#[async_trait]
pub trait StoreProtocol: Cell {
    /// Persist a Signal, returning its reference.
    async fn put(&self, signal: Signal) -> Result<SignalRef>;

    /// Retrieve by identity.
    async fn get(&self, id: &SignalId) -> Result<Option<Signal>>;

    /// Structured query (kind, tags, time range, score thresholds).
    async fn query(&self, query: StoreQuery) -> Result<Vec<Signal>>;

    /// HDC similarity search. Returns (ref, distance) pairs.
    async fn query_similar(
        &self,
        fingerprint: &HdcVector,
        radius: f32,
        limit: usize,
    ) -> Result<Vec<(SignalRef, f32)>>;

    /// Remove Signals below balance threshold. Returns prune report.
    async fn prune(&self, threshold: f64) -> Result<PruneReport>;
}
```

---

### 2.2 Score Protocol

Rate a Signal along 5 dimensions (relevance, quality, confidence, novelty, utility). Score Cells are learners: they predict quality, publish predictions, and receive corrections from gate verdicts via calibration.

```rust
#[async_trait]
pub trait ScoreProtocol: Cell {
    /// Rate a Signal, producing a 5-dimensional Score.
    async fn score(
        &self,
        signal: &Signal,
        context: &ScoreContext,
    ) -> Result<Score>;
}

/// Context for scoring decisions.
pub struct ScoreContext {
    /// Recent Signals in the same topic for relative scoring.
    pub neighbors: Vec<SignalRef>,

    /// The query or task that prompted this scoring.
    pub query: Option<String>,

    /// HDC fingerprint of the current attention focus.
    pub attention_focus: Option<HdcVector>,
}
```

---

### 2.3 Verify Protocol

The load-bearing protocol. Serves four roles simultaneously: **reward function** (continuous `Verdict.reward: f64`), **relabeling oracle** (hindsight relabeling of failed trajectories), **safety boundary** (`verify_pre()` can veto execution), and **economic attestation** (reputation flows from verified work). See Design Principle 5 in [doc-00](00-INDEX.md).

```rust
/// Context for verification decisions.
pub struct VerifyContext {
    /// Access to Store for querying prior verdicts and evidence.
    pub store: Arc<dyn Store>,

    /// Current Agent identity, if running inside an Agent scope.
    pub agent: Option<AgentId>,

    /// The Agent's Space (capability boundary for verification actions).
    pub space: Option<Arc<Space>>,

    /// Cancellation token -- long-running verifications check this.
    pub cancel: CancellationToken,
}

#[async_trait]
pub trait VerifyProtocol: Cell {
    /// Pre-action check. Called BEFORE a Cell executes.
    /// Returns `Verdict` -- if `hard_pass` is false, execution is vetoed.
    async fn verify_pre(
        &self,
        input: &[Signal],
        plan: &ActionPlan,
        ctx: &VerifyContext,
    ) -> Result<Verdict>;

    /// Post-action check. Called AFTER a Cell produces output.
    /// Returns `Verdict` with evidence, reward, and criteria evaluation.
    async fn verify_post(
        &self,
        input: &[Signal],
        output: &[Signal],
        ctx: &VerifyContext,
    ) -> Result<Verdict>;

    /// Streaming check. Called periodically during long-running execution.
    /// Enables early termination if a hard criterion is violated mid-stream.
    async fn verify_stream(
        &self,
        partial_output: &[Signal],
        ctx: &VerifyContext,
    ) -> Result<StreamVerdict>;
}
```

#### Verdict

```rust
/// The output of a Verify check. Contains a continuous reward alongside
/// binary pass/fail, typed evidence, and separate hard/soft criteria.
pub struct Verdict {
    /// Continuous reward signal for learning. Domain-specific scale.
    /// Routing uses this to update EFE estimates. Episode logging records it.
    pub reward: f64,

    /// Conjunctive hard criteria -- ALL must pass. Binary AND.
    /// If any hard criterion fails, the Verdict is a hard fail regardless
    /// of soft criteria or reward.
    pub hard_pass: bool,
    pub hard_criteria: Vec<CriterionResult>,

    /// Pareto soft criteria -- multi-objective, NEVER weighted-sum.
    /// Soft criteria produce a Pareto front; dominated solutions are
    /// discarded. No single-scalar collapse (Goodhart-resistant).
    pub soft_criteria: Vec<CriterionResult>,

    /// Typed evidence collected during verification.
    /// Evidence is separate from Criterion -- a Criterion references
    /// Evidence by kind, but Evidence exists independently.
    pub evidence: Vec<Evidence>,

    /// Wall-clock time spent verifying.
    pub duration: Duration,

    /// Optional explanation for human review.
    pub explanation: Option<String>,
}

pub struct StreamVerdict {
    /// Whether to continue execution.
    pub continue_execution: bool,

    /// Partial verdict (may be updated by verify_post).
    pub partial: Verdict,
}
```

#### Criterion

```rust
/// A single verification criterion. Criteria are either hard (conjunctive)
/// or soft (Pareto). The distinction is structural, not a weight.
pub struct CriterionResult {
    pub criterion: Criterion,
    pub passed: bool,
    pub score: f64,            // 0.0..=1.0 for this specific criterion
    pub evidence_refs: Vec<EvidenceRef>,
}

/// The 19 criterion kinds.
#[non_exhaustive]
pub enum Criterion {
    // -- Correctness --
    Compiles,
    TestsPassing,
    TypeSafe,
    NoRegressions,

    // -- Quality --
    ClippyClean,
    FormattingClean,
    CoverageAbove { threshold: f64 },
    ComplexityBelow { threshold: f64 },
    DiffReasonable { max_lines: usize },

    // -- Safety --
    NoSecretLeak,
    PermissionsRespected,
    SandboxIntact,
    InvariantPreserved { invariant: String },

    // -- Economic --
    WithinBudget { max_cost: Cost },
    WithinDeadline { max_duration: Duration },

    // -- Semantic --
    RelevantToTask,
    ConsistentWithContext,
    NoDuplication,

    // -- Custom --
    Custom { name: String, description: String },
}
```

#### Evidence

Evidence is typed separately from Criterion. A Criterion references Evidence but does not own it. This separation enables: reuse of evidence across criteria, independent evidence collection, and evidence aggregation across multiple verification passes.

```rust
/// Typed verification evidence. 19 kinds.
pub struct Evidence {
    pub kind: EvidenceKind,
    pub content: Value,
    pub collected_at: DateTime<Utc>,
    pub collector: CellRef,
}

#[non_exhaustive]
pub enum EvidenceKind {
    CompileOutput,
    TestResult { suite: String, passed: u32, failed: u32 },
    ClippyDiagnostic,
    DiffStats { insertions: u32, deletions: u32, files: u32 },
    CoverageReport { line_pct: f64, branch_pct: f64 },
    RuntimeTrace,
    MemoryProfile,
    SecurityScan,
    LlmJudgment { model: String, prompt_hash: ContentHash },
    HumanReview { reviewer: String },
    BenchmarkResult { metric: String, value: f64, unit: String },
    TypeCheckOutput,
    SandboxLog,
    CostReport { total: Cost, breakdown: Vec<(String, Cost)> },
    SchemaValidation,
    RegressionDiff,
    PermissionAudit,
    InvariantCheck { invariant: String },
    Custom { name: String },
}
```

#### The Four Roles of Verdict

The Verdict type serves four distinct roles simultaneously, derived from its fields:

**Role 1: Reward Function** (`Verdict.reward`). The continuous reward feeds into the Route protocol's Expected Free Energy (EFE) computation, updating `CandidateHistory.mean_reward` and `reward_variance` via Welford's incremental algorithm. Without this, routing degenerates to random selection.

```rust
fn update_route_beliefs(history: &mut CandidateHistory, verdict: &Verdict) {
    history.trials += 1;
    let delta = verdict.reward - history.mean_reward;
    history.mean_reward += delta / history.trials as f64;
    let delta2 = verdict.reward - history.mean_reward;
    history.reward_variance += delta * delta2;
}
```

**Role 2: Relabeling Oracle** (`Verdict.hard_criteria` + `evidence`). When a task fails, the Verdict provides structured feedback for trajectory relabeling -- hindsight relabeling (Andrychowicz et al. 2017, HER). The failed trajectory + criterion-level detail becomes a corrective example injected into the next attempt's context.

```rust
fn relabel_failed_trajectory(trajectory: &[Signal], verdict: &Verdict) -> RelabeledExample {
    let failed_hard: Vec<&CriterionResult> = verdict.hard_criteria
        .iter().filter(|c| !c.passed).collect();
    let diagnostics: Vec<&Evidence> = verdict.evidence
        .iter().filter(|e| matches!(e.kind,
            EvidenceKind::CompileOutput | EvidenceKind::TestResult { .. }
            | EvidenceKind::ClippyDiagnostic | EvidenceKind::RuntimeTrace
        )).collect();
    RelabeledExample {
        original_trajectory: trajectory.to_vec(),
        failed_criteria: failed_hard.into_iter().cloned().collect(),
        diagnostics: diagnostics.into_iter().cloned().collect(),
        soft_profile: verdict.soft_criteria.iter()
            .map(|c| (format!("{:?}", c.criterion), c.score)).collect(),
        corrective_hint: build_corrective_hint(&failed_hard, &diagnostics),
    }
}
```

**Role 3: Safety Boundary** (`Verdict.hard_pass` + `verify_pre()` + `StreamVerdict.continue_execution`). Three enforcement points: PRE-ACTION (veto before execution), MID-STREAM (terminate mid-flight), POST-ACTION (reject output). Safety criteria are always hard criteria -- they cannot be traded off.

| Criterion | Safety role |
|---|---|
| `NoSecretLeak` | Prevents credential exposure in agent output |
| `PermissionsRespected` | Enforces capability boundary (see S3.2) |
| `SandboxIntact` | Verifies execution stayed within sandbox level |
| `InvariantPreserved` | Custom invariants (e.g., "never delete production data") |

**Role 4: Economic Attestation** (Verdict as persisted Signal + Evidence chain). Passing Verdicts serve as signed quality certificates. Reputation flows from verified work: agents producing passing Verdicts gain reputation weighted by verifier reputation; failing Verdicts decrease reputation.

```rust
fn reputation_from_verdict(
    verdict: &Verdict, current_reputation: f64, verifier_reputation: f64,
) -> f64 {
    let weight = verifier_reputation.clamp(0.0, 1.0);
    if verdict.hard_pass {
        current_reputation + verdict.reward * weight * 0.1
    } else {
        let severity = verdict.hard_criteria.iter()
            .filter(|c| !c.passed).count() as f64;
        current_reputation - severity * weight * 0.05
    }
}
```

#### Goodhart Resistance: Hard + Pareto Soft

The Verdict structure resists Goodhart's Law through structural separation:

- **Hard criteria**: conjunctive (AND). ALL must pass. Cannot compensate for safety with quality.
- **Soft criteria**: Pareto front. Multi-objective, NEVER collapsed to scalar.

```rust
/// Hard pass is Boolean AND. No weighted-sum escape route.
fn hard_pass(verdict: &Verdict) -> bool {
    verdict.hard_criteria.iter().all(|c| c.passed)
}

/// Pareto front: non-dominated solutions only.
fn pareto_front(candidates: &[Verdict]) -> Vec<&Verdict> {
    candidates.iter().filter(|v| {
        !candidates.iter().any(|other| dominates(other, v))
    }).collect()
}

fn dominates(a: &Verdict, b: &Verdict) -> bool {
    let mut strictly_better = false;
    for (ca, cb) in a.soft_criteria.iter().zip(b.soft_criteria.iter()) {
        if ca.score < cb.score { return false; }
        if ca.score > cb.score { strictly_better = true; }
    }
    strictly_better
}
```

**Proof sketch**: Hard criteria cannot be gamed by substitution (conjunctive). Soft criteria resist collapse gaming (Pareto front -- lateral moves, not improvements). The combination forces the optimizer to (a) pass all hard criteria with no shortcuts, and (b) find a non-dominated Pareto position with no single-dimension inflation.

#### The Variance Inequality

The central safety property of Verify:

```
Var[verifier(x) - truth(x)] < Var[generator(x) - truth(x)]
```

The verifier ensemble must have lower variance on ground-truth benchmarks than the generator. A noisier verifier adds uncertainty rather than resolving it. Three structural mechanisms enforce this:

1. **Disjoint-family panels**: Judges from disjoint model families. Correlated errors cancel across families. 3 from different families > 5 from the same.
2. **No self-judgment**: A Cell never verifies its own output. Generator and verifier must be different Cells, ideally different families.
3. **Calibration benchmarks**: Periodically test Verify Cells against known ground truth. Violated inequality flags the verifier for replacement.

```rust
struct VarianceCheck {
    verifier_variance: f64,
    generator_variance: f64,
    benchmark_size: usize,
}

impl VarianceCheck {
    fn inequality_holds(&self) -> bool {
        self.verifier_variance < self.generator_variance
    }
}
```

#### Pairwise Bradley-Terry Judges

For subjective criteria (code quality, relevance, consistency), Verify uses **pairwise comparison** aggregated via Bradley-Terry maximum likelihood estimation (Bradley & Terry 1952). This avoids the well-known instability of absolute Likert-scale LLM judgments (anchoring, scale drift, position bias).

```rust
pub struct PairwiseJudgment {
    pub judge: CellRef,
    pub candidate_a: SignalRef,
    pub candidate_b: SignalRef,
    pub winner: PairwiseWinner,
    pub criterion: Criterion,
    pub confidence: f64,
    pub reasoning: Option<String>,
}

pub enum PairwiseWinner { A, B, Tie }

pub struct BradleyTerryResult {
    pub strengths: Vec<(SignalRef, f64)>,  // log-scale, higher = better
    pub convergence: f64,                  // should be < 1e-6
    pub comparisons: usize,
}
```

The BT model: `P(c_i beats c_j) = pi_i / (pi_i + pi_j)`. MLE via iterative proportional fitting (Zermelo 1929). Convergence guaranteed for connected comparison graphs.

```rust
fn bradley_terry_mle(
    comparisons: &[PairwiseJudgment],
    candidates: &[SignalRef],
    max_iterations: usize,
    tolerance: f64,
) -> BradleyTerryResult {
    let n = candidates.len();
    let mut strengths = vec![1.0f64; n];
    let idx: HashMap<&SignalRef, usize> = candidates.iter().enumerate()
        .map(|(i, c)| (c, i)).collect();

    let mut wins = vec![0.0f64; n];
    let mut totals = vec![vec![0.0f64; n]; n];

    for cmp in comparisons {
        let i = idx[&cmp.candidate_a];
        let j = idx[&cmp.candidate_b];
        totals[i][j] += 1.0;
        totals[j][i] += 1.0;
        match cmp.winner {
            PairwiseWinner::A => wins[i] += 1.0,
            PairwiseWinner::B => wins[j] += 1.0,
            PairwiseWinner::Tie => { wins[i] += 0.5; wins[j] += 0.5; }
        }
    }

    let mut convergence = f64::MAX;
    for _ in 0..max_iterations {
        let old = strengths.clone();
        for i in 0..n {
            let denom: f64 = (0..n).filter(|&j| j != i)
                .map(|j| totals[i][j] / (strengths[i] + strengths[j])).sum();
            if denom > 0.0 { strengths[i] = wins[i] / denom; }
        }
        let sum: f64 = strengths.iter().sum();
        for s in &mut strengths { *s *= n as f64 / sum; }
        convergence = strengths.iter().zip(old.iter())
            .map(|(a, b)| (a - b).abs()).fold(0.0f64, f64::max);
        if convergence < tolerance { break; }
    }

    BradleyTerryResult {
        strengths: candidates.iter().zip(strengths.iter())
            .map(|(c, &s)| (c.clone(), s.ln())).collect(),
        convergence,
        comparisons: comparisons.len(),
    }
}
```

**Panel design**: At least 3 judges from disjoint model families; no judge from the generator's family; each pair compared at least once per judge; comparison graph must be connected (for BT convergence).

---

### 2.4 Route Protocol

Select among candidate Cells or models for a given task. Uses **Expected Free Energy** (EFE, Friston 2006) rather than LinUCB, incorporating both epistemic value (information gain from trying uncertain candidates) and pragmatic value (expected task reward).

```rust
#[async_trait]
pub trait RouteProtocol: Cell {
    async fn route(
        &self,
        candidates: &[RouteCandidate],
        context: &RouteContext,
    ) -> Result<RouteResult>;
}

pub struct RouteCandidate {
    pub id: CellRef,
    pub name: String,
    pub estimated_cost: Cost,
    pub estimated_duration: Duration,
    pub capabilities: Capabilities,
    pub history: Option<CandidateHistory>,
}

pub struct CandidateHistory {
    pub trials: u32,
    pub mean_reward: f64,
    pub reward_variance: f64,
    pub mean_cost: Cost,
}

pub struct RouteContext {
    pub regime: Regime,
    pub vitality: f64,          // remaining_budget / initial_budget
    pub complexity: f64,        // 0.0..=1.0
    pub urgency: f64,           // 0.0 = no deadline, 1.0 = imminent
    pub budget_remaining: Cost,
    pub context_signals: Vec<SignalRef>,
}

pub enum Regime {
    Calm,       // explore freely
    Normal,     // standard EFE balance
    Volatile,   // reduce exploration
    Crisis,     // pure exploitation
}

pub struct RouteResult {
    pub selected: CellRef,
    pub efe_score: f64,
    pub pragmatic_value: f64,
    pub epistemic_value: f64,
    pub cost_term: f64,
    pub alternatives: Vec<(CellRef, f64)>,
}
```

**EFE computation**:

```rust
fn compute_efe(candidate: &RouteCandidate, context: &RouteContext) -> f64 {
    let history = candidate.history.as_ref();
    let pragmatic = history.map(|h| h.mean_reward).unwrap_or(0.5);
    let epistemic = history.map(|h| {
        if h.trials < 2 { 1.0 }
        else { (h.reward_variance / (h.trials - 1) as f64).sqrt() }
    }).unwrap_or(1.0);

    let explore_weight = match context.regime {
        Regime::Calm => 0.4,
        Regime::Normal => 0.2,
        Regime::Volatile => 0.05,
        Regime::Crisis => 0.0,
    };

    let cost_term = candidate.estimated_cost.to_usd()
        / context.budget_remaining.to_usd().max(0.001);

    pragmatic + explore_weight * epistemic - cost_term
}
```

**Why EFE instead of LinUCB**: EFE subsumes LinUCB by also modeling the *value of reducing uncertainty*. An agent that has never tried a cheap model on simple tasks has high epistemic value for that pairing, even if the pragmatic expectation is uncertain. EFE naturally produces the progressive cascade (T0 pattern-match -> T1 cheap model -> T2 expensive model) without hand-coded tier thresholds.

---

### 2.5 Compose Protocol

Assemble multiple Signals into a single output Signal under a budget constraint. Uses a **VCG auction** (Vickrey 1961, Clarke 1971, Groves 1973) with 8+ context bidders.

```rust
#[async_trait]
pub trait ComposeProtocol: Cell {
    async fn compose(
        &self,
        bids: Vec<ComposeBid>,
        budget: &ComposeBudget,
        ctx: &ComposeContext,
    ) -> Result<ComposeResult>;
}

pub struct ComposeBid {
    pub bidder: BidderId,
    pub section: ComposeSection,
    pub value: f64,          // truthful under VCG
    pub token_cost: u32,
    pub effect: BetaPosterior,
}

pub struct ComposeSection {
    pub name: String,
    pub content: String,
    pub kind: ComposeSectionKind,
    pub source_signals: Vec<SignalRef>,
}

pub enum ComposeSectionKind {
    SystemInstruction, TaskDescription, CodeContext, ResearchContext,
    EpisodeHistory, HeuristicGuidance, ToolDocumentation, SafetyConstraint,
    Custom(String),
}

pub struct ComposeBudget {
    pub max_tokens: u32,
    pub max_cost: Cost,
    pub required_sections: Vec<ComposeSectionKind>,
}

pub struct BetaPosterior {
    pub alpha: f64,
    pub beta: f64,
}

impl BetaPosterior {
    pub fn mean(&self) -> f64 { self.alpha / (self.alpha + self.beta) }
    pub fn variance(&self) -> f64 {
        (self.alpha * self.beta)
            / ((self.alpha + self.beta).powi(2) * (self.alpha + self.beta + 1.0))
    }
}

pub struct ComposeResult {
    pub composed: Signal,
    pub accepted: Vec<BidderId>,
    pub payments: Vec<(BidderId, f64)>,  // VCG externality costs
    pub total_tokens: u32,
    pub total_cost: Cost,
}
```

**8+ built-in context bidders**:

| Bidder | What it bids | Source |
|---|---|---|
| `TaskBidder` | Task description, acceptance criteria | Current plan task |
| `CodeBidder` | Relevant code files, definitions | roko-index / tree-sitter |
| `ResearchBidder` | Research findings, citations | roko-research artifacts |
| `EpisodeBidder` | Prior episodes on similar tasks | roko-learn episodes |
| `HeuristicBidder` | Relevant heuristics with calibration | roko-neuro |
| `ToolBidder` | Tool documentation for enabled tools | MCP tool manifests |
| `SafetyBidder` | Safety constraints, contract terms | Agent contract YAML |
| `NeuroBidder` | Distilled knowledge entries | roko-neuro knowledge store |

**Novelty attenuation**: `effective_value = stated_value * (1 / (1 + ln(freq)))`. Common boilerplate gradually loses bid strength, making room for novel context.

---

### 2.6 React Protocol

Watch the Pulse stream and emit Signals and/or Pulses in response. **Breaking change from v1**: React operates on Pulses (ephemeral), not Signals.

```rust
#[async_trait]
pub trait ReactProtocol: Cell {
    async fn react(
        &self,
        pulses: &[Pulse],
        ctx: &ReactContext,
    ) -> Result<ReactOutput>;

    /// Topic filter -- which Pulses this React Cell wants to see.
    fn subscription(&self) -> TopicFilter;
}

pub struct ReactContext {
    pub bus: Arc<dyn Bus>,
    pub store: Arc<dyn Store>,
    pub agent: Option<AgentId>,
    pub cancel: CancellationToken,
}

pub struct ReactOutput {
    pub pulses: Vec<Pulse>,
    pub signals: Vec<Signal>,
}
```

---

### 2.7 Observe Protocol

Read-only observation producing observation Signals. Used by Lenses for telemetry, StateHub projections, and c-factor computation.

```rust
#[async_trait]
pub trait ObserveProtocol: Cell {
    /// Observe the current state and produce observation Signals.
    /// Read-only: Observe Cells MUST NOT mutate state.
    async fn observe(&self, ctx: &ObserveContext) -> Result<Vec<Signal>>;
}

pub struct ObserveContext {
    pub bus: Arc<dyn Bus>,
    pub store: Arc<dyn Store>,
    pub query: Option<String>,
    pub time_range: Option<TimeRange>,
}
```

---

### 2.8 Connect Protocol

Lifecycle-managed connection to external systems.

```rust
#[async_trait]
pub trait ConnectProtocol: Cell {
    async fn connect(&self, config: &ConnectConfig) -> Result<ConnectionHandle>;
    async fn query(&self, handle: &ConnectionHandle, query: Value) -> Result<Value>;
    async fn execute(&self, handle: &ConnectionHandle, command: Value) -> Result<Value>;
    async fn disconnect(&self, handle: ConnectionHandle) -> Result<()>;
    async fn health(&self, handle: &ConnectionHandle) -> Result<ConnectionHealth>;
}

pub struct ConnectionHandle {
    pub id: ConnectionId,
    pub protocol: String,
    pub connected_at: DateTime<Utc>,
}

pub enum ConnectionHealth {
    Healthy,
    Degraded { reason: String },
    Disconnected,
}
```

---

### 2.9 Trigger Protocol

Listen for events and fire Graphs. Fully specified in [doc-13](13-TRIGGERS.md).

```rust
#[async_trait]
pub trait TriggerProtocol: Cell {
    async fn arm(&self, binding: &TriggerBinding) -> Result<TriggerHandle>;
    async fn disarm(&self, handle: TriggerHandle) -> Result<()>;
    async fn poll(&self, handle: &TriggerHandle) -> Result<Vec<TriggerEvent>>;
}
```

---

## 3. Cross-Cutting Concerns

### 3.1 TypeSchema

Every Cell declares its input and output schema. TypeSchema enables compile-time edge validation in Graphs ([doc-03](03-GRAPH.md)) -- mismatched types are caught when the Graph is loaded, not at runtime.

```rust
pub enum TypeSchema {
    Any,
    OfKind(Kind),
    JsonSchema(serde_json::Value),
    OneOf(Vec<TypeSchema>),
    AllOf(Vec<TypeSchema>),
    ArrayOf(Box<TypeSchema>),
    Record(BTreeMap<String, TypeSchema>),
}

impl TypeSchema {
    pub fn is_compatible(&self, other: &TypeSchema) -> bool {
        match (self, other) {
            (_, TypeSchema::Any) => true,
            (TypeSchema::Any, _) => false,
            (TypeSchema::OfKind(a), TypeSchema::OfKind(b)) => a == b,
            (TypeSchema::OneOf(variants), target) => {
                variants.iter().all(|v| v.is_compatible(target))
            }
            (source, TypeSchema::OneOf(variants)) => {
                variants.iter().any(|v| source.is_compatible(v))
            }
            _ => false,
        }
    }
}
```

**TypeSchema as a preorder**: `Any >= OfKind(k) >= JsonSchema(s) >= Record({...})` and `>= ArrayOf(s')`. `OneOf([a, b])` is compatible with target `T` iff EVERY variant is compatible with T (conservative). `AllOf([a, b])` is compatible iff ANY component is (any suffices). This is the subtyping lattice -- fail-closed by design.

**Why `OneOf` uses `all` (not `any`)**: The `all` rule is intentional and conservative. A `OneOf` source can emit ANY of its variants at runtime. If we used `any` (at least one variant is compatible), the remaining incompatible variants would cause runtime type errors on the edges that accepted them. The `all` rule guarantees that no matter which variant the source emits, the target can always accept it. This trades expressiveness for safety: an `OneOf([Text, Image])` source cannot connect to a target that only accepts `Text`. To handle such cases, use a `Branch` node to route variants to type-compatible targets.

### 3.2 Capabilities -- Three-Layer Intersection

Capabilities are fail-closed: a Cell only runs if the intersection of three capability sets is non-empty.

```rust
pub struct Capabilities {
    pub declared: CapabilitySet,   // Cell author
    pub granted: CapabilitySet,    // Agent operator
    pub permitted: CapabilitySet,  // Space policy
}

impl Capabilities {
    /// Effective = intersection of all three layers.
    /// This is the pullback in the category of capability sets.
    pub fn effective(&self) -> CapabilitySet {
        self.declared.intersection(&self.granted).intersection(&self.permitted)
    }
}

pub struct CapabilitySet(BTreeSet<Capability>);

#[non_exhaustive]
pub enum Capability {
    ReadFile,
    WriteFile,
    Execute { sandbox: SandboxLevel },
    Network { allow_list: Vec<String> },
    LlmCall { models: Vec<String> },
    StoreRead,
    StoreWrite,
    BusPublish { topics: Vec<TopicFilter> },
    BusSubscribe { topics: Vec<TopicFilter> },
    HumanEscalation,
    SpawnAgent,
    ModifyPlan,
    Custom(String),
}

pub enum SandboxLevel {
    None,       // no shell access
    Readonly,   // read-only shell
    Sandboxed,  // sandboxed shell (default)
    Full,       // unrestricted (requires explicit grant)
}
```

**Capability intersection as pullback**: Given `D` (declared), `G` (granted), `P` (permitted), effective = `D intersect G intersect P`. Properties: (1) Fail-closed -- missing from ANY layer means denied. (2) Monotone -- adding caps to one layer never removes effective caps. (3) Composable -- for a pipeline `A -> B`, effective caps = `eff(A) intersect eff(B)`. **Capability escalation through composition is impossible.**

```rust
fn pipeline_capabilities(cells: &[&dyn Cell]) -> CapabilitySet {
    cells.iter()
        .map(|c| c.capabilities().effective())
        .reduce(|acc, cap| acc.intersection(&cap))
        .unwrap_or_default()
}
```

### 3.3 Five Implementation Tiers

Cells range from zero-code configuration to native Rust, forming a **Spectral Package Interface** (SPI).

| Tier | Name | Cell Defined By | Capabilities | Target Audience |
|---|---|---|---|---|
| T0 | Prompts | System prompt text + role config | LlmCall only | Domain experts |
| T1 | Config | TOML parameters on existing Cells | Varies by base Cell | Power users |
| T2 | Declarative Tools | JSON/TOML tool manifests + MCP | Tool execution | Developers |
| T3 | WASM | Compiled WASM module (any source language) | Sandboxed compute | Plugin developers |
| T4 | Native Rust | `impl Cell for MyCell` | Full capability set | Core developers |

```toml
# T0 example: a code-review Cell defined entirely by prompt
[cell.code-reviewer]
tier = "prompt"
system_prompt = """
You are a senior code reviewer. Focus on:
1. Correctness
2. Readability
3. Performance
Provide specific, actionable feedback.
"""
input_schema = { kind = "Diff" }
output_schema = { kind = "Markdown" }
protocols = ["score"]

# T1 example: configuring an existing Route Cell
[cell.conservative-router]
tier = "config"
base = "builtin://efe-router"
[cell.conservative-router.params]
exploration_rate = 0.05
regime_override = "volatile"
max_cost_per_call = 0.10
```

### 3.4 Cell Lifecycle Events as Pulses

Every Cell execution emits lifecycle Pulses on Bus. These are the sole source of runtime observability.

| Event | Topic | Graduates? |
|---|---|---|
| Cell execution started | `cell.{id}.started` | No (noise) |
| Cell execution completed | `cell.{id}.completed` | Batch (per-run summary) |
| Cell execution failed | `cell.{id}.failed` | Yes (forensic) |
| Cell prediction published | `prediction.{id}` | No (calibration input) |
| Cell cost charged | `cost.charged` | Yes (accounting) |
| Cell capability denied | `safety.capability.denied` | Yes (audit) |

### 3.5 Protocol Conformance Declaration

```rust
pub struct RegisteredCell {
    pub block: Arc<dyn Cell>,
    pub protocols: Vec<ProtocolConformance>,
}

pub struct ProtocolConformance {
    pub protocol: ProtocolId,
    pub version: Version,
    pub verified: bool,
}
```

### 3.6 Cost Estimation

```rust
/// Cost in USD-equivalent microcents (1 microcent = $0.00000001).
/// Integer arithmetic avoids floating-point rounding in accounting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cost(pub u64);

impl Cost {
    pub const ZERO: Cost = Cost(0);
    pub fn from_usd(dollars: f64) -> Self { Cost((dollars * 100_000_000.0) as u64) }
    pub fn to_usd(&self) -> f64 { self.0 as f64 / 100_000_000.0 }
}
```

### 3.7 CellError

```rust
pub enum CellError {
    SchemaViolation { expected: TypeSchema, got: Kind },
    CapabilityDenied { capability: Capability, layer: String },
    BudgetExhausted { spent: Cost, limit: Cost },
    Timeout { elapsed: Duration, limit: Duration },
    Cancelled,
    UpstreamFailure { block: CellRef, error: Box<CellError> },
    PreVerifyVeto { verdict: Verdict },
    Internal(anyhow::Error),
}
```

### 3.8 CellRef

```rust
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct CellRef {
    pub id: CellId,
    pub name: String,
    pub version: Version,
}
```

### 3.9 Protocol Composition

A single Cell can implement multiple protocols. A `CodeReviewCell` implementing both Score and Verify lives in the **product category** `Score x Verify`:

```rust
pub struct CodeReviewCell { /* ... */ }

impl Cell for CodeReviewCell {
    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::Score, ProtocolId::Verify]
    }
    // Input schema must accept the UNION of both protocols' inputs.
    // Output depends on which protocol is invoked on a given edge.
    fn input_schema(&self) -> Option<&TypeSchema> {
        Some(&self.combined_input_schema)
    }
    fn output_schema(&self) -> Option<&TypeSchema> {
        Some(&self.combined_output_schema)
    }
    // ...
}

#[async_trait]
impl ScoreProtocol for CodeReviewCell {
    async fn score(&self, signal: &Signal, ctx: &ScoreContext) -> Result<Score> {
        todo!()
    }
}

#[async_trait]
impl VerifyProtocol for CodeReviewCell {
    async fn verify_pre(&self, _input: &[Signal], _plan: &ActionPlan,
        _ctx: &VerifyContext) -> Result<Verdict> { todo!() }
    async fn verify_post(&self, input: &[Signal], output: &[Signal],
        ctx: &VerifyContext) -> Result<Verdict> { todo!() }
    async fn verify_stream(&self, _partial: &[Signal],
        _ctx: &VerifyContext) -> Result<StreamVerdict> {
        Ok(StreamVerdict { continue_execution: true, partial: Verdict::default() })
    }
}
```

Categorically: two projection functors `pi_score: Score x Verify -> Score` and `pi_verify: Score x Verify -> Verify`. Multi-protocol Cells must satisfy all declared protocol contracts (enforced at registration via `ProtocolConformance`).

---

## 4. Protocol Algebra

The 9 protocols form a category **Cell** where:

- **Objects** are Cells (carrying typed I/O schemas, capability sets, protocol conformance).
- **Morphisms** are typed Signal/Pulse flows between Cells. A morphism `f: A -> B` exists when `A.output_schema` is compatible with `B.input_schema`.
- **Composition** is sequential piping: `g . f: A -> C` feeds A's output into B, then B's into C.
- **Identity** is the trivial passthrough Cell.

### 4.1 Protocol Morphism Table

Not all protocol-to-protocol compositions are meaningful:

```text
Source -> Target         Legal?    Morphism type
--------------------------------------------------------------------
Store  -> Score          Yes       Query -> Rate
Store  -> Verify         Yes       Retrieve -> Check
Store  -> Route          Yes       Retrieve -> Select
Store  -> Compose        Yes       Retrieve -> Assemble
Store  -> React          No        (Store is pull, React is push)
Store  -> Observe        Yes       Retrieve -> Read
Store  -> Connect        No        (Store is internal)
Store  -> Trigger        No        (Store is passive)

Score  -> Route          Yes       Rate -> Select (scored ranking)
Score  -> Verify         Yes       Rate -> Check (quality predicts pass)
Score  -> Compose        Yes       Rate -> Budget allocation
Score  -> Score          Yes       Cascade scoring

Verify -> React          Yes       Verdict -> React (reward signal)
Verify -> Route          Yes       Verdict -> Select (learn from outcomes)
Verify -> Score          Yes       Verdict -> Recalibrate (correction)
Verify -> Compose        No        (Verdict is terminal, not material)
Verify -> Store          Yes       Verdict -> Persist (audit)

Route  -> Compose        Yes       Selection -> Assemble context
Route  -> Verify         Yes       Selection -> Check (pre-verify)
Route  -> Connect        Yes       Selection -> Dispatch (tool/model)

Compose -> Verify        Yes       Assembled -> Check (the golden path)
Compose -> Connect       Yes       Assembled -> Send (prompt -> LLM)
Compose -> Store         Yes       Assembled -> Persist

React  -> Store          Yes       Graduate (Pulse -> Signal)
React  -> React          Yes       Chain reactions

Observe -> Score         Yes       Observation -> Rate
Observe -> React         Yes       Observation -> React (telemetry)
Observe -> Route         Yes       Observation -> Select (regime detection)

Connect -> Verify        Yes       External result -> Check
Connect -> Store         Yes       External result -> Persist
Connect -> React         Yes       External event -> React

Trigger -> React         Yes       Fire event -> React
Trigger -> Compose       Yes       Fire event -> Assemble context
Trigger -> Route         Yes       Fire event -> Select handler
```

### 4.2 Adjacency Matrix

```rust
/// Protocol adjacency matrix. True = legal composition.
const PROTOCOL_ADJACENCY: [[bool; 9]; 9] = {
    //          Store Score Verfy Route Comps React Obsrv Conct Trigr
    /* Store */ [true, true, true, true, true, false,true, false,false],
    /* Score */ [false,true, true, true, true, false,false,false,false],
    /* Verfy */ [true, true, false,true, false,true, false,false,false],
    /* Route */ [false,false,true, false,true, false,false,true, false],
    /* Comps */ [true, false,true, false,false,false,false,true, false],
    /* React */ [true, false,false,false,false,true, false,false,false],
    /* Obsrv */ [false,true, false,true, false,true, false,false,false],
    /* Conct */ [true, false,true, false,false,true, false,false,false],
    /* Trigr */ [false,false,false,true, true, true, false,false,false],
};

fn protocol_composable(src: &[ProtocolId], tgt: &[ProtocolId]) -> bool {
    src.iter().any(|s| tgt.iter().any(|t|
        PROTOCOL_ADJACENCY[*s as usize][*t as usize]))
}
```

Checked at Graph load time. Violations are static errors.

### 4.3 Natural Transformations Between Protocols

Several protocol-to-protocol relationships are natural transformations:

**Score => Verify** (the most important): Every Score Cell's 5-dimensional output lifts systematically to a Verdict:

```rust
fn score_to_verdict(score: &Score, thresholds: &VerifyThresholds) -> Verdict {
    let hard_criteria = vec![
        CriterionResult {
            criterion: Criterion::RelevantToTask,
            passed: score.relevance >= thresholds.min_relevance,
            score: score.relevance, evidence_refs: vec![] },
        CriterionResult {
            criterion: Criterion::ClippyClean,
            passed: score.quality >= thresholds.min_quality,
            score: score.quality, evidence_refs: vec![] },
    ];
    let hard_pass = hard_criteria.iter().all(|c| c.passed);

    // Reward = geometric mean (penalizes any zero dimension more than arithmetic)
    let reward = (score.relevance * score.quality * score.confidence
        * score.novelty * score.utility).powf(1.0 / 5.0);

    Verdict { reward, hard_pass, hard_criteria, soft_criteria: vec![],
        evidence: vec![], duration: Duration::ZERO, explanation: None }
}
```

**Why natural**: The transformation commutes with Cell composition. Score-then-verify or verify-directly yields the same boundary (given same thresholds).

**Verify => React**: Every Verdict becomes a Pulse on Bus, enabling the predict-publish-correct Loop.

**Store <=> React** (Graduation and Projection): dual natural transformations forming an adjunction (see S6).

### 4.4 Complete Edge Validation

```rust
fn validate_edge(
    source: &RegisteredCell, target: &RegisteredCell, space: &SpacePolicy,
) -> Result<EdgeValidation, EdgeError> {
    // 1. Type compatibility (S3.1 preorder)
    let type_compat = match (source.block.output_schema(), target.block.input_schema()) {
        (_, None) => TypeCompat::AnyAccepted,
        (None, Some(expected)) => return Err(EdgeError::TypeMismatch {
            source: source.block.name().into(), target: target.block.name().into(),
            expected: expected.clone(), got: TypeSchema::Any }),
        (Some(out), Some(inp)) => {
            if out.is_compatible(inp) { TypeCompat::Compatible }
            else { return Err(EdgeError::TypeMismatch {
                source: source.block.name().into(), target: target.block.name().into(),
                expected: inp.clone(), got: out.clone() }) }
        }
    };

    // 2. Protocol adjacency (S4.2 matrix)
    if !protocol_composable(source.block.protocols(), target.block.protocols()) {
        return Err(EdgeError::ProtocolIncompatible {
            source_protocols: source.block.protocols().to_vec(),
            target_protocols: target.block.protocols().to_vec() });
    }

    // 3. Capability pullback (S3.2 intersection)
    let pipeline_caps = source.block.capabilities().effective()
        .intersection(&target.block.capabilities().effective());
    if !space.permits_all(&pipeline_caps) {
        return Err(EdgeError::CapabilityDenied {
            required: pipeline_caps, permitted: space.permitted_capabilities().clone() });
    }

    // 4. Cost estimation
    let estimated_cost = match (source.block.estimated_cost(), target.block.estimated_cost()) {
        (Some(a), Some(b)) => Some(Cost(a.0 + b.0)), _ => None };

    Ok(EdgeValidation { type_compat, proto_compat: true,
        effective_capabilities: pipeline_caps, estimated_cost })
}
```

### 4.5 Categorical Summary

```text
Category Cell:
  Objects   = { C | C : Cell, registered with protocol conformance }
  Morphisms = { f : A -> B | validate_edge(A, B, space) = Ok(_) }
  Identity  = PassthroughCell (output_schema = input_schema, all protocols)
  Compose   = g . f defined when validate_edge(A, B) and validate_edge(B, C) both hold

Functors:
  Store : Cell -> Set     (forgetful: Cell -> its stored Signals)
  Bus   : Cell -> Stream  (forgetful: Cell -> its published Pulses)
  Adjunction: Bus -| Store via graduation/projection (S6)

Natural transformations:
  eta_sv : Score => Verify        (score_to_verdict)
  eta_vr : Verify => React        (verdict_to_pulse)
  eta_rs : React => Store         (graduation)
  eta_sr : Store => React         (projection)

Free monad:
  CellProgram<A> = Free(ProtocolF, A)
  Interpretation: CellProgram<A> -> Flow<A> via the execution engine
```

### 4.6 Protocol Algebra Laws

| Law | Statement | Enables |
|---|---|---|
| Store idempotence | `put(put(signal)) = put(signal)` | Eliminate redundant writes |
| Score monotonicity | `score(compose([a,b])) >= min(score(a), score(b))` | Early rejection |
| Verify conjunctivity | `verify(a AND b) = verify(a) AND verify(b)` | Parallel verification |
| React commutativity | `react([p1,p2]) = react([p2,p1])` (for commutative Cells) | Pulse reordering for batching |

---

## 5. The Free Monad Over Protocol

The 9 protocols define an algebra. The **free monad** over this algebra is the type of all possible Cell programs before any interpretation (execution). This separates *description* from *execution*.

### 5.1 The Protocol Functor

```rust
enum ProtocolF<A> {
    Put(Signal, Box<dyn FnOnce(SignalRef) -> A>),
    Query(StoreQuery, Box<dyn FnOnce(Vec<Signal>) -> A>),
    Score(Signal, ScoreContext, Box<dyn FnOnce(Score) -> A>),
    Verify(Vec<Signal>, VerifyContext, Box<dyn FnOnce(Verdict) -> A>),
    Route(Vec<RouteCandidate>, RouteContext, Box<dyn FnOnce(RouteResult) -> A>),
    Compose(Vec<ComposeBid>, ComposeBudget, Box<dyn FnOnce(ComposeResult) -> A>),
    React(Vec<Pulse>, Box<dyn FnOnce(ReactOutput) -> A>),
    Observe(ObserveContext, Box<dyn FnOnce(Vec<Signal>) -> A>),
    Connect(ConnectionHandle, Value, Box<dyn FnOnce(Value) -> A>),
    Trigger(TriggerBinding, Box<dyn FnOnce(Vec<TriggerEvent>) -> A>),
}
```

### 5.2 The Free Monad

```rust
enum CellProgram<A> {
    Pure(A),
    Step(ProtocolF<CellProgram<A>>),
}

impl<A> CellProgram<A> {
    fn and_then<B>(self, f: impl FnOnce(A) -> CellProgram<B>) -> CellProgram<B> {
        match self {
            CellProgram::Pure(a) => f(a),
            CellProgram::Step(step) => {
                CellProgram::Step(step.map_continuation(|prog| prog.and_then(f)))
            }
        }
    }
}

/// Convenience constructors -- each wraps a ProtocolF variant into a Step.
impl CellProgram<Vec<Signal>> {
    fn query(q: StoreQuery) -> Self {
        CellProgram::Step(ProtocolF::Query(q, Box::new(CellProgram::Pure)))
    }
}

impl CellProgram<Score> {
    fn score_all(signals: Vec<Signal>) -> CellProgram<Vec<(Signal, Score)>> {
        // Wraps ProtocolF::Score for each signal, collecting results
        CellProgram::Step(ProtocolF::Score(
            signals[0].clone(), ScoreContext::default(),
            Box::new(|_score| CellProgram::Pure(vec![])), // simplified
        ))
    }
}

impl CellProgram<RouteResult> {
    fn route(scored: Vec<(Signal, Score)>) -> Self {
        CellProgram::Step(ProtocolF::Route(
            vec![], RouteContext::default(),
            Box::new(CellProgram::Pure),
        ))
    }
}

impl CellProgram<ComposeResult> {
    fn compose(selected: RouteResult, budget: ComposeBudget) -> Self {
        CellProgram::Step(ProtocolF::Compose(
            vec![], budget,
            Box::new(CellProgram::Pure),
        ))
    }
}

impl CellProgram<Verdict> {
    fn verify(composed: ComposeResult) -> Self {
        CellProgram::Step(ProtocolF::Verify(
            vec![], VerifyContext::default(),
            Box::new(CellProgram::Pure),
        ))
    }

    /// The standard Cell pipeline (pseudocode -- actual types require
    /// intermediate CellProgram adapters for heterogeneous chaining).
    fn standard_pipeline(query: StoreQuery, budget: ComposeBudget) -> CellProgram<Verdict> {
        CellProgram::query(query)
            .and_then(|signals| CellProgram::score_all(signals))
            .and_then(|scored| CellProgram::route(scored))
            .and_then(|selected| CellProgram::compose(selected, budget))
            .and_then(|composed| CellProgram::verify(composed))
    }
}
```

### 5.3 Why Free Monads Matter

1. **Static analysis**: Inspect `CellProgram` structure before executing to determine resource needs, verify capabilities, estimate cost.
2. **Interpretation swapping**: Same program interpreted by real executor, mock executor, or cost estimator.
3. **Optimization**: Compiler pass can fuse adjacent Store operations, batch Score calls, eliminate redundant Verify checks.

---

## 6. Store-Bus Duality

Store and Bus are dual -- two views of the same information flow:

| Store (pull) | Bus (push) |
|---|---|
| Consumer initiates (`query`) | Producer initiates (`publish`) |
| Durable (survives restart) | Ephemeral (bounded ring, then gone) |
| Identity is content hash | Identity is sequence number |
| Supports similarity (`query_similar`) | Supports topic routing (`TopicFilter`) |
| Retention is decay-based (demurrage) | Retention is capacity-based (ring eviction) |
| Medium: Signal | Medium: Pulse |
| Concurrency: read-many, write-serialized | Concurrency: write-many, read-many (broadcast) |

### 6.1 The Store-Bus Adjunction

Graduation (`F: Pul -> Sig`) and Projection (`G: Sig -> Pul`) form an adjunction `F -| G`:

```
Unit:   eta_P : P -> G(F(P))     "graduate then project back"
Counit: eps_S : F(G(S)) -> S     "project then graduate back"
```

Unit: graduate a Pulse to Signal, then project as notification Pulse. Result: a "cleaned" Pulse with SignalRef and content hash.

Counit: project a Signal as Pulse, then downstream Cell graduates it. Idempotent on content-addressed Signals.

```
Hom_Bus(Pulses, G(Signals))  ~=  Hom_Store(F(Pulses), Signals)
```

Subscribing to store-write notifications on Bus is equivalent to querying Store for recently graduated Signals.

### 6.2 Graduation Policy as a React Cell

```rust
struct GraduationPolicy {
    subscription: TopicFilter,
    criteria: GraduationCriteria,
}

struct GraduationCriteria {
    sample_rate: Option<usize>,
    min_body_size: Option<usize>,
    always_graduate: Vec<TopicFilter>,
    never_graduate: Vec<TopicFilter>,
    graduate_on_failure: bool,
    batch_threshold: Option<usize>,
}

#[async_trait]
impl ReactProtocol for GraduationPolicy {
    async fn react(&self, pulses: &[Pulse], ctx: &ReactContext) -> Result<ReactOutput> {
        let mut graduated_signals = Vec::new();
        for pulse in pulses {
            if self.should_graduate(pulse) {
                let signal = Signal::from_pulse(pulse);
                ctx.store.put(signal.clone()).await?;
                graduated_signals.push(signal);
            }
        }
        Ok(ReactOutput { pulses: vec![], signals: graduated_signals })
    }

    fn subscription(&self) -> TopicFilter { self.subscription.clone() }
}
```

### 6.3 Consistency Guarantees

1. **Store-first**: graduation writes to Store before publishing downstream Pulses. Prevents phantom notifications.
2. **Projection-best-effort**: projection Pulse after Store write is best-effort. Signal is safe in Store regardless.
3. **Idempotent graduation**: same Pulse twice produces same SignalRef (content-addressed).
4. **Ring eviction is not data loss**: graduated content is in Store. Un-graduated was deemed ephemeral.

### 6.4 The Decision Tree: Store or Bus?

```
Does the information need to survive process restart?
  YES --> STORE (Signal)
  NO  --> Is it a notification about something durable?
            YES --> BUS (Pulse) with lineage_hint to the Signal
            NO  --> Is it ephemeral coordination traffic?
                      YES --> BUS (Pulse) on appropriate topic
                      NO  --> Probably not worth persisting or transporting
```

### 6.5 Backpressure Strategies

| Strategy | Implementation | Used for |
|---|---|---|
| Ring capacity sizing | `bus.ring_capacity = 16384` in roko.toml | Burst absorption |
| Sampling | `SamplingReactCell<R>`: every N-th Pulse | High-throughput feeds |
| Windowed aggregation | `WindowedReactCell<R>`: collect for window, batch | Metrics, telemetry |
| Priority eviction | `PriorityRingBuffer`: low-priority evicted first | Mixed-criticality traffic |

### 6.6 Distributed Bus

```text
Single-process:   BroadcastBus (tokio broadcast + VecDeque ring)
Multi-process:    NatsBus / KafkaBus (topic -> subject/partition mapping)
Cross-datacenter: MultiBus (fan-in from multiple backends)
On-chain:         ChainBus (topics -> contract event logs)
```

All backends implement the same `Bus` trait. `MultiBus` aggregates multiple backends into a single Bus surface.

---

## 7. Verify as Universal Oracle

Verify is the protocol that connects agent computation to external reality. Every other protocol operates on internal representations. Verify asks: *"Did the thing we produced actually work?"*

### 7.1 The Four-Role Verdict Dispatch

After every verification, the Verdict feeds all four roles simultaneously:

```rust
async fn dispatch_verdict(
    verdict: &Verdict, task: &TaskContext,
    store: &dyn StoreProtocol, bus: &dyn Bus,
) -> Result<()> {
    // Role 1: Reward function -> Route protocol
    bus.publish(Pulse {
        topic: Topic::from(if verdict.hard_pass {
            "verify.verdict.passed" } else { "verify.verdict.failed" }),
        body: Body::json(&RouteRewardPayload {
            task_type: task.task_type.clone(),
            candidate: task.selected_candidate.clone(),
            reward: verdict.reward, regime: task.regime }),
        ..Default::default()
    }).await?;

    // Role 2: Relabeling oracle -> Learning subsystem
    if !verdict.hard_pass {
        let relabeled = relabel_failed_trajectory(&task.trajectory, verdict);
        store.put(Signal::from_relabeled(&relabeled)).await?;
    }

    // Role 3: Safety boundary (already enforced at pre/stream/post)
    if verdict.hard_criteria.iter().any(|c| !c.passed && is_safety_criterion(&c.criterion)) {
        bus.publish(Pulse {
            topic: Topic::from("safety.verdict.violation"),
            body: Body::json(&SafetyViolation {
                task: task.task_id.clone(),
                failed_criteria: verdict.hard_criteria.iter()
                    .filter(|c| !c.passed).cloned().collect() }),
            ..Default::default()
        }).await?;
    }

    // Role 4: Economic attestation -> Reputation
    let attestation = EconomicAttestation {
        verdict_ref: store.put(Signal::from_verdict(verdict)).await?,
        work_ref: task.output_ref.clone(),
        producer: task.agent_id.clone(),
        verifier: task.verifier_ref.clone(),
        reputation_delta: if verdict.hard_pass { verdict.reward * 0.1 }
            else { -0.05 * verdict.hard_criteria.iter()
                .filter(|c| !c.passed).count() as f64 },
        verification_cost: task.verification_cost,
    };
    bus.publish(Pulse {
        topic: Topic::from("reputation.update"),
        body: Body::json(&attestation),
        ..Default::default()
    }).await?;

    Ok(())
}
```

### 7.2 Meta-Verification: When the Verifier Is Wrong

A systematic bias in Verify propagates through the entire system. Meta-verification treats this as a Loop:

```rust
struct MetaVerifyLoop {
    primary_verifiers: Vec<CellRef>,
    meta_panel: DisjointPanel,
    sample_rate: f64,
    disagreement_threshold: f64,
}

impl MetaVerifyLoop {
    async fn cycle(&self, store: &dyn StoreProtocol, bus: &dyn Bus)
        -> Result<MetaVerifyReport> {
        let recent = store.query(StoreQuery {
            kinds: Some(vec![Kind::from("verdict")]),
            limit: Some(100), ..Default::default() }).await?;
        let sample_size = (recent.len() as f64 * self.sample_rate) as usize;
        let sample = reservoir_sample(&recent, sample_size);

        let mut disagreements = 0;
        for verdict_signal in &sample {
            let original: Verdict = parse_body(&verdict_signal.body);
            let original_input = fetch_lineage(store, verdict_signal).await?;
            let meta_verdicts = self.meta_panel.verify_all(&original_input).await?;
            let meta_consensus = aggregate_verdicts(&meta_verdicts);
            if meta_consensus.hard_pass != original.hard_pass {
                disagreements += 1;
                bus.publish(Pulse {
                    topic: Topic::from("meta.verify.disagreement"),
                    body: Body::json(&MetaDisagreement {
                        original: original.clone(), meta: meta_consensus.clone(),
                        signal_ref: verdict_signal.id.clone() }),
                    ..Default::default()
                }).await?;
            }
        }

        Ok(MetaVerifyReport {
            sample_size, disagreements,
            disagreement_rate: disagreements as f64 / sample_size as f64,
            threshold_exceeded: (disagreements as f64 / sample_size as f64) > self.disagreement_threshold,
        })
    }
}
```

### 7.3 The Regress Termination

The regress terminates at ground truth:

1. **Level 0**: Generator produces output.
2. **Level 1**: Primary Verify checks against objective criteria (compiles, tests). Unambiguous ground truth. No regress.
3. **Level 2**: Meta-panel spot-checks primary Verdicts. Disagreements on objective criteria = verifier bugs. Disagreements on subjective criteria resolved by human (Level 3).
4. **Level 3**: Human review. Final oracle. Used sparingly.

---

## 8. Predict-Publish-Correct

Every Cell is a learner. This is the structural learning pattern (Friston 2006, active inference made structural).

**The pattern**:
1. Before acting, the Cell publishes a **prediction** Pulse on `prediction.{block_id}`.
2. The Cell executes and produces output.
3. Reality (gate verdicts, downstream results) publishes an **outcome** Pulse on `outcome.{block_id}`.
4. A `CalibrationPolicy` (React Cell) subscribes to both topics, joins by `lineage_hint`, computes error, publishes `calibration.{block_id}.updated`.
5. The Cell subscribes to its own calibration topic and adjusts parameters.

```rust
pub struct CalibrationTable {
    pub mean_error: f64,
    pub error_variance: f64,
    pub updates: u64,
    pub context_errors: BTreeMap<String, f64>,
    pub last_updated: DateTime<Utc>,
}

pub struct CalibrationUpdate {
    pub block_id: CellRef,
    pub prediction: Value,
    pub outcome: Value,
    pub error: f64,
    pub context_key: Option<String>,
}
```

**Verify's own calibration**: Before verifying, a Verify Cell predicts its own outcome. After, the actual Verdict is published. The CalibrationPolicy computes prediction error. A Verify Cell that consistently over- or under-predicts its own verdicts has its internal parameters adjusted.

**Why predict-publish-correct instead of a learning subsystem**: Learning is not a separate module -- it emerges from the same pub/sub fabric that carries heartbeats and gate verdicts. Every Cell improves by construction. The CalibrationPolicy is itself a React Cell. The system learns using its own primitives (Design Principle 7).

---

## 9. TOML Registration

Cells can be registered via TOML for tiers T0-T2:

```toml
[[blocks]]
name = "code-reviewer"
tier = "prompt"
version = "0.1.0"
protocols = ["score", "verify"]
system_prompt = "..."

[[blocks]]
name = "conservative-router"
tier = "config"
version = "0.1.0"
base = "builtin://efe-router"
[blocks.params]
exploration_rate = 0.05

[[blocks]]
name = "github-connector"
tier = "declarative"
version = "0.1.0"
protocols = ["connect"]
mcp_server = "github"
```

---

## 10. Feedback Loops

1. **Score => Verify calibration**: The Score-to-Verdict transformation uses thresholds updated via EMA on historical verdicts. As the system learns which score ranges predict passes, the transformation becomes tighter.

2. **Protocol adjacency expansion**: New edges can be added to the adjacency matrix from configuration, allowing learned composition rules.

3. **Free monad optimization**: The execution engine tracks which fusion/batching optimizations improve performance. Degrading optimizations are rolled back via predict-publish-correct.

4. **Verifier rotation**: When meta-verification detects bias, the primary verifier is rotated out and replaced. The rotated-out verifier enters recalibration (shadowed Verdicts until disagreement rate drops).

5. **Adaptive gate thresholds**: Hard-criterion thresholds updated via EMA on historical pass rates. Too-easy (>95%) tightens. Too-hard (<50%) investigated (but not auto-loosened).

6. **Criterion evolution**: New criteria can be added to the Criterion enum as new failure modes emerge (e.g., `Terminates` for infinite loops).

7. **Graduation policy refinement**: Before graduating, the policy predicts "this Pulse will be queried within 1 hour." After an hour, reality checked. Criteria tightened or loosened via predict-publish-correct.

8. **Backpressure adaptation**: Sampling and windowing parameters adjusted based on subscriber lag.

---

## 11. Citations

| Concept | Citation |
|---|---|
| Active inference, free energy principle | Friston, K. (2006). A free energy principle for the brain. *J. Physiology-Paris*, 100(1-3), 70-87. |
| VCG auction mechanism | Vickrey, W. (1961). *J. Finance*, 16(1). Clarke, E. (1971). *Public Choice*, 11. Groves, T. (1973). *Econometrica*, 41(4). |
| Bradley-Terry pairwise comparison | Bradley, R. A., & Terry, M. E. (1952). *Biometrika*, 39(3/4), 324-345. |
| EFE for routing | Friston, K. et al. (2015). Active inference and epistemic value. *Cognitive Neuroscience*, 6(4). |
| Prospect theory (somatic markers) | Kahneman, D., & Tversky, A. (1979). *Econometrica*, 47(2), 263-292. |
| Hyperdimensional computing | Kanerva, P. (2009). *Cognitive Computation*, 1(2), 139-159. |
| Goodhart's Law | Goodhart, C. A. E. (1984). *Monetary Theory and Practice*, 91-121. |
| LinUCB (replaced by EFE) | Li, L. et al. (2010). *WWW 2010*, 661-670. |
| Hindsight relabeling | Andrychowicz, M. et al. (2017). Hindsight experience replay. *NeurIPS*. |
| Category theory / free monads | Mac Lane, S. (1971). *Categories for the Working Mathematician*. Springer. |

---

## 12. Acceptance Criteria

| Criterion | Verification |
|---|---|
| `Cell` trait compiles with `id`, `input_schema`, `output_schema`, `capabilities`, `protocols`, `estimated_cost`, `execute` | Compile check |
| All 9 protocol traits compile with full type signatures | Compile check |
| `Verdict` has `reward: f64`, `hard_pass: bool`, `hard_criteria`, `soft_criteria`, `evidence` | Compile check |
| Hard criteria are conjunctive: single hard fail -> overall fail | Unit test |
| Soft criteria produce Pareto front, not weighted sum | Unit test with 3+ criteria showing non-dominated set |
| Evidence is typed separately from Criterion (19 `EvidenceKind` variants) | Compile check |
| `PairwiseJudgment` and `BradleyTerryResult` compile | Compile check |
| Bradley-Terry MLE converges for connected comparison graph | Unit test |
| Route uses EFE: `pragmatic + epistemic - cost` decomposition | Unit test showing exploration of uncertain candidates |
| `RouteContext` includes `regime: Regime` and `vitality: f64` | Compile check |
| Regime-dependent exploration weights | Unit test: Crisis -> 0 exploration |
| Compose uses VCG auction with `ComposeBid.value` and `ComposeBudget` | Unit test: truthful bidding dominates lying |
| Section effect tracked via `BetaPosterior` updated from gate verdicts | Unit test: alpha increments on pass, beta on fail |
| 8 built-in bidders registered | Integration test |
| Novelty attenuation in Compose: `1/(1+ln(freq))` | Unit test |
| React operates on `Pulse` (not `Signal`), returns `ReactOutput { pulses, signals }` | Compile check |
| `TypeSchema::is_compatible` rejects mismatched edges | Unit test |
| TypeSchema preorder: `Any >= OfKind >= JsonSchema >= Record` | Unit test chain |
| Capabilities three-layer intersection: denied in any layer -> denied overall | Unit test |
| Pipeline capability intersection: `eff(A->B) = eff(A) intersect eff(B)` | Unit test |
| 5 implementation tiers: T0 prompt Cell loads from TOML | Integration test |
| Cell lifecycle events published as Pulses on Bus | Integration test |
| `CalibrationTable` receives updates from `CalibrationPolicy` | Integration test |
| Predict-publish-correct: prediction -> outcome -> calibration update | Integration test on Bus |
| `Cost` integer arithmetic: no floating-point rounding | Unit test |
| `CellError::PreVerifyVeto` carries the `Verdict` that vetoed | Compile check |
| Protocol composition: Cell implementing 2+ protocols dispatches correctly | Integration test |
| Protocol adjacency matrix: illegal edges rejected at Graph load time | Unit test |
| Complete edge validation: type + protocol + capability check | Integration test |
| Score => Verify natural transformation: geometric mean reward | Unit test |
| Verify => React natural transformation: verdict Pulse on correct topic | Unit test |
| Store-Bus adjunction: graduation + projection round-trip idempotent | Integration test |
| Graduation policy: `always_graduate` > `never_graduate` > defaults | Unit test |
| Store-first consistency: Store write before Bus notification | Integration test |
| Idempotent graduation: same Pulse twice -> same SignalRef | Unit test |
| Variance Inequality: verifier ensemble lower variance than generator | Evaluation test |
| Disjoint-family panels: 3 from different families > 5 from same | Evaluation test |
| No self-judgment: generator != verifier enforced | Unit test |
| Meta-verification: disagreement detection and reporting | Integration test |
| Backpressure: sampling React Cell processes every N-th Pulse | Unit test |
| Free monad: `CellProgram::standard_pipeline` compiles | Compile check |
| Four-role verdict dispatch: all four roles receive Verdict | Integration test |
