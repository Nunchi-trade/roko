# 23 -- Arenas, Evals, and Bounties

> Arena = universal measurement surface. Eval = calibration against ground truth. Bounty = paid task with escrow. All three feed the reputation registry and the cascade router's learning loop. The 7-step flywheel converts raw attempts into calibrated knowledge. Pattern extraction produces Heuristic Signals with mandatory falsifiers.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal/Pulse, demurrage, HDC fingerprints, Heuristic kind), [02-CELL](02-CELL.md) (Cell trait, Verify protocol, predict-publish-correct, conjunctive/Pareto scoring), [03-GRAPH](03-GRAPH.md) (Graph composition), [04-EXECUTION](04-EXECUTION.md) (Flow lifecycle), [07-LEARNING](07-LEARNING.md) (4 loops, Variance Inequality), [06-MEMORY](06-MEMORY.md) (demurrage, knowledge distillation), [22-REGISTRIES](22-REGISTRIES.md) (reputation registry, InsightStore), [08-GATEWAY](08-GATEWAY.md) (CascadeRouter)

---

## 1. Design Constraints

1. **No self-grading.** Evals never use LLM output to judge LLM output. Ground truth comes from external oracles, test suites, human review, chain state, or benchmark datasets. This is enforced by the Verify protocol ([doc-02](02-CELL.md)) and the Variance Inequality: the verifier must be spectrally cleaner than the generator. "The LLM thinks it's good" is not a ground truth source.
2. **Scoring is declarative.** Every arena and eval declares its scoring function at registration time. Agents know how they will be scored before they start. No post-hoc scoring changes.
3. **Escrow before execution.** Bounties lock funds in a contract before Agents begin work. No payment promises -- only escrowed funds.
4. **Reputation flows from validation.** Arena attempts and bounty completions produce Verify-protocol verdict Signals. These feed the reputation registry ([doc-22](22-REGISTRIES.md)).
5. **VCG for matching, Vickrey for bidding.** Agent-to-task matching uses welfare-maximizing VCG allocation (Vickrey-Clarke-Groves). Individual bounties use second-price auctions. Both enforce truthful bidding -- agents cannot gain by misrepresenting their value.
6. **Cross-arena transfer is measured.** Skills demonstrated in one arena produce knowledge Signals with HDC fingerprints. When those fingerprints correlate with success in another arena, the system has discovered genuine cross-domain transfer. Transfer is measured, not assumed.

---

## 2. Kernel Mapping

Arenas, evals, and bounties are specializations of kernel primitives. No new kernel types are introduced.

| Arena Concept | Kernel Primitive | Notes |
|---|---|---|
| Arena | Graph with Verify-protocol Cells | Task source + scoring + leaderboard = a Graph executed per attempt |
| Attempt | Flow | A Flow executing the Arena's Graph for a specific Agent |
| Scoring function | Cell implementing Score protocol | Produces verdict Signals with 5-axis quality rating |
| Leaderboard | Store projection | Derived view recomputed from attempt Signals via aggregation rule |
| Eval | Cell implementing Verify protocol | Measurement with declared GroundTruthSource |
| Meta-eval | Cell that wraps another Verify Cell | Calibrates the calibrator |
| Bounty | Signal (Bounty kind) in Store | Escrow state tracked on-chain, work tracked off-chain |
| VCG matching | Compose-protocol Cell | Same `vcg_allocate` used for context assembly |
| Flywheel | Loop specialization (7-step) | Recurring Graph execution per arena |
| Cross-arena transfer | HDC fingerprint correlation | Computed from episode Signals across arena boundaries |

---

## 3. Arena as Universal Measurement Surface

An arena is more than a competitive environment -- it is the **universal measurement surface** that connects Agent behavior to ground truth. Every arena defines three things: what agents do (task source), how they are scored (function), and who is winning (leaderboard).

### 3.1 The 7-Step Flywheel

Every arena, regardless of domain, executes this cycle. The flywheel IS the arena -- it converts raw attempts into calibrated knowledge.

```
1. TRACE          Agent executes task, all actions recorded as episode Signals
       |
       v
2. AUTO-GRADE     Verify-protocol Cells produce verdict Signals (binary + continuous reward)
       |
       v
3. PREFERENCE-MINE  Extract pairwise preferences from scored attempts via Bradley-Terry MLE
       |
       v
4. FAILURE-CLUSTER  Group failed attempts by HDC fingerprint similarity -> failure modes
       |
       v
5. CURRICULUM-GEN   Generate training tasks targeting discovered failure modes (adversarial)
       |
       v
6. PATTERN-EXTRACT  Distill successful strategies from high-scoring attempts -> Heuristic Signals
       |
       v
7. PREFERENCE-BOOTSTRAP  Use extracted patterns to bootstrap preferences for new arenas
       |
       +--- feeds back to step 1 (new curriculum tasks enter the arena)
```

The flywheel is self-reinforcing. More attempts produce more failure clusters, which generate more targeted curriculum, which produces more attempts. Extracted patterns (Heuristic Signals with mandatory falsifiers, see [doc-01](01-SIGNAL.md) section 4) bootstrap new arenas without cold-start.

**Step 6 is load-bearing**: pattern extraction produces Heuristic Signals -- not rules of thumb, but testable predictions with a mandatory falsifier and a live calibration track record. The falsifier is derived from the failure clusters (step 4): "this strategy works UNLESS [falsifier condition from failure analysis]."

### 3.2 Why "Universal Measurement Surface"

Every learning loop in the system (see [doc-07](07-LEARNING.md)) depends on ground truth. Arenas are the primary source:

| Learning Loop | What Arena Provides |
|---|---|
| L1 Parameter tuning | Continuous `Verdict.reward` from arena auto-grading |
| L2 Strategy routing | Arena performance data feeds CascadeRouter model selection |
| L3 Dream cycle | High-scoring attempts are candidates for knowledge distillation |
| L4 Structural adaptation | Arena curricula identify which Graph structures fail |

Without arenas, learning loops have no ground truth. With arenas, every dimension of Agent behavior is measurable.

---

## 4. Core Types

All arena types are expressed as Cell specializations and Signal kinds. The `Arena` struct is a Graph definition; attempts are Flows; scores are verdict Signals.

### 4.1 Arena Definition

```rust
/// Arena lifecycle states.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArenaState {
    /// Arena created but not yet accepting attempts.
    Draft,
    /// Arena is live and accepting attempts.
    Active,
    /// Arena is temporarily paused (no new attempts, existing ones continue).
    Paused,
    /// Arena has permanently concluded. Leaderboard is final.
    Concluded,
}

/// Where arena tasks come from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TaskSource {
    /// Fixed dataset of input/output pairs.
    Static {
        /// Content hash (stored on chain substrate) or URL pointing to the dataset.
        dataset_cid: String,
        /// Number of tasks in the dataset.
        count: u64,
        /// Whether tasks are sampled randomly per attempt.
        randomize: bool,
    },
    /// Tasks generated at attempt time by a deterministic function.
    Procedural {
        /// Generator identifier (registered in the eval registry).
        generator_id: [u8; 32],
        /// Seed derivation: per-attempt, per-epoch, or fixed.
        seed_mode: SeedMode,
        /// Difficulty parameters passed to the generator.
        difficulty: HashMap<String, f64>,
    },
    /// Tasks submitted by users and curated by the arena creator.
    UserContributed {
        /// Minimum reputation required to submit tasks.
        min_contributor_reputation: f64,
        /// Whether submissions require creator approval.
        requires_approval: bool,
    },
    /// Tasks designed to exploit weaknesses found in prior attempts.
    Adversarial {
        /// Agent that generates adversarial tasks.
        adversary_agent_id: [u8; 32],
        /// Maximum difficulty increase per round.
        max_difficulty_step: f64,
        /// Whether the adversary can see prior attempt strategies.
        sees_prior_attempts: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeedMode {
    /// New random seed per attempt.
    PerAttempt,
    /// Same seed for all attempts within an epoch (enables direct comparison).
    PerEpoch { epoch_duration_blocks: u64 },
    /// Fixed seed (all attempts see the same tasks).
    Fixed { seed: u64 },
}
```

### 4.2 Scoring Functions

Three scoring types compose to handle any measurement. Composite scoring uses **conjunctive hard criteria (AND) + Pareto soft criteria** -- never weighted-sum.

```rust
/// How an attempt gets scored. Implemented as Verify-protocol Cells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ScoringFunction {
    /// Pass or fail. Score is 0.0 or 1.0.
    Binary {
        /// What determines pass/fail.
        criterion: BinaryCriterion,
    },
    /// Continuous score in [0.0, 1.0] or unbounded.
    Continuous {
        /// Metric computed from the attempt output.
        metric: ContinuousMetric,
        /// How to normalize the raw metric to a score.
        normalization: Normalization,
    },
    /// Conjunctive hard criteria (AND) + Pareto soft criteria.
    /// No weighted-sum: Goodhart's Law makes weighted combinations
    /// exploitable. Pareto ranking has no such failure mode.
    Composite {
        /// Component scores and their weights.
        components: Vec<ScoringComponent>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BinaryCriterion {
    /// All gate checks pass.
    AllGatesPass,
    /// Test suite passes with zero failures.
    TestSuitePass { suite_hash: String },
    /// External oracle returns true.
    OracleVerdict { oracle_id: [u8; 32] },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ContinuousMetric {
    /// Sharpe ratio of returns (for trading arenas).
    SharpeRatio,
    /// Continuous ranked probability score (for prediction arenas).
    CRPS,
    /// Execution time in milliseconds (lower is better).
    Latency,
    /// Token efficiency: output quality per token spent.
    TokenEfficiency,
    /// Custom metric computed by a registered eval function.
    Custom { eval_id: [u8; 32] },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Normalization {
    /// Score is used as-is.
    Identity,
    /// Linearly scaled to [0, 1] based on observed min/max.
    MinMax,
    /// Z-score relative to population mean/stddev.
    ZScore,
    /// Percentile rank within the leaderboard.
    Percentile,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScoringComponent {
    pub name: String,
    pub function: Box<ScoringFunction>,
    pub weight: f64,
}
```

Why no weighted-sum in Composite: Goodhart's Law. Given weights, agents optimize the weight vector rather than the underlying qualities. A 60/40 correctness/efficiency weighting produces agents that sacrifice correctness for efficiency at the margin. Pareto ranking has no such failure mode -- an agent is Pareto-optimal only if no other agent beats it on all dimensions simultaneously.

### 4.3 Arena Struct

```rust
/// The full arena definition. This is a Graph template: each attempt
/// instantiates a Flow from this definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Arena {
    /// Unique arena identifier (blake3 hash of creation params).
    pub id: [u8; 32],
    /// Human-readable name.
    pub name: String,
    /// Markdown description.
    pub description: String,
    /// Category for filtering.
    pub category: ArenaCategory,
    /// Current lifecycle state.
    pub state: ArenaState,
    /// Where tasks come from.
    pub task_source: TaskSource,
    /// How attempts are scored (Verify-protocol Cells).
    pub scoring: ScoringFunction,
    /// How individual scores aggregate into leaderboard rank.
    pub aggregation: AggregationRule,
    /// Creator's identity ID.
    pub creator_identity_id: u128,
    /// Block at which the arena was created.
    pub created_at_block: u64,
    /// Optional prize pool in USDC (held in escrow).
    pub prize_pool_usdc: u64,
    /// Maximum attempts per agent (0 = unlimited).
    pub max_attempts_per_agent: u64,
    /// Rate limit: minimum blocks between attempts by the same agent.
    pub cooldown_blocks: u64,
    /// Optional deadline block (arena concludes automatically).
    pub deadline_block: Option<u64>,
    /// Ground truth source declaration (required by design constraint 1).
    pub ground_truth: GroundTruthSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArenaCategory {
    Coding,
    Trading,
    Prediction,
    Games,
    Persuasion,
    Negotiation,
    Optimization,
    Research,
    UserCreated,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AggregationRule {
    /// Best score across all attempts.
    BestOf,
    /// Average of the last N attempts.
    AverageLastN { n: u64 },
    /// Exponentially weighted moving average.
    EWMA { alpha: f64 },
    /// Median of all attempts.
    Median,
}
```

---

## 5. The 8 Arenas

Eight concrete arenas cover the primary domains where Agents operate. Each arena has domain-specific scoring, but all share the 7-step flywheel.

### 5.1 Coding Arena

**Task**: Fix bugs, implement features, refactor code, write tests.
**Scoring**: Correctness (test pass rate), token efficiency (tokens per successful change), latency (wall-clock time), code quality (clippy + complexity metrics).
**Ground truth**: Test suites, compilation, gate pipeline.
**Cross-arena transfer**: Code patterns transfer to optimization and security audit arenas.

### 5.2 Trading Arena

**Task**: Execute trades in simulated or live markets.
**Scoring**: Sharpe ratio, max drawdown, PnL, win rate.
**Ground truth**: Market state at settlement (chain state or simulation oracle).
**Cross-arena transfer**: Risk assessment patterns transfer to prediction and optimization arenas.

### 5.3 Prediction Arena

**Task**: Forecast future states (prices, metrics, outcomes).
**Scoring**: CRPS (Continuous Ranked Probability Score), calibration (Brier score), discrimination.
**Ground truth**: Realized outcomes at resolution time.
**Cross-arena transfer**: Calibration skills transfer to all arenas (every arena benefits from well-calibrated confidence).

### 5.4 Research Arena

**Task**: Analyze documents, synthesize findings, produce cited reports.
**Scoring**: Recall (found relevant information), precision (avoided irrelevant), citation quality (sources verified), comprehensiveness.
**Ground truth**: Expert-curated reference answers, benchmark datasets.
**Cross-arena transfer**: Information retrieval patterns transfer to coding (documentation search), security (vulnerability database search).

### 5.5 Games Arena

**Task**: Play adversarial games (Go, chess, strategy games, negotiation simulations).
**Scoring**: Win rate, Elo rating.
**Ground truth**: Game outcome (win/loss/draw) -- unambiguous.
**Cross-arena transfer**: Strategic planning transfers to trading (position management) and optimization (constraint satisfaction).

### 5.6 Optimization Arena

**Task**: Minimize or maximize objective functions under constraints (gas optimization, resource allocation, scheduling).
**Scoring**: Continuous objective value, constraint satisfaction rate.
**Ground truth**: Objective function evaluation (deterministic).
**Cross-arena transfer**: Constraint reasoning transfers to coding (performance optimization) and trading (portfolio optimization).

### 5.7 Security Audit Arena

**Task**: Find vulnerabilities in code, smart contracts, configurations.
**Scoring**: True positive rate (found real vulnerabilities), false positive rate (avoided false alarms), severity-weighted coverage.
**Ground truth**: Known vulnerability set (planted bugs, historical CVEs, audit reports).
**Cross-arena transfer**: Pattern recognition transfers to coding (defensive programming) and research (threat modeling).

### 5.8 Self-Hosting Meta-Arena

**Task**: Roko developing itself. The self-hosting loop IS an arena.
**Scoring**: See section 6 (Meta-Arena).
**Ground truth**: Git merge, CI pass, gate pipeline.
**Cross-arena transfer**: The meta-arena produces cross-domain transfer by definition -- every improvement to Roko's own tooling benefits all other arenas.

### 5.9 Cross-Arena Transfer

When an Agent scores well in one arena, the episode Signals carry HDC fingerprints. If those fingerprints correlate with success in a different arena, the system has discovered genuine cross-domain transfer:

```
Agent A scores 95th percentile in Coding Arena
    |
    v
Episode Signals fingerprinted via HDC (Kanerva 2009)
    |
    v
Agent A enters Security Audit Arena
    |
    v
HDC similarity between successful coding episodes
    and security audit tasks > threshold
    |
    v
Cross-arena transfer detected:
  - Coding patterns that predict security audit success
  - Stored as Heuristic Signals with cross-domain tags
  - CascadeRouter learns to route similar tasks to Agent A
```

Transfer is measured, not assumed. An Agent that excels at coding does not automatically get credit in security -- it must demonstrate the transfer.

---

## 6. Meta-Arena: Roko Developing Itself

The self-hosting workflow (see CLAUDE.md) IS an arena. Every PR that Roko opens against its own codebase is an arena attempt. The meta-arena has unique properties.

### 6.1 Scoring Dimensions

| Metric | What It Measures | Ground Truth |
|---|---|---|
| **PR merge rate** | What fraction of generated PRs merge successfully | Git history: merged vs closed/abandoned |
| **Gate pass rate** | What fraction of tasks pass the gate pipeline on first attempt | Gate verdict Signals per task |
| **Cost per task** | USD spent per successfully completed task | Cost Signals from episode logger |
| **Time to first PR on new codebase** | How quickly can Roko start contributing to unfamiliar code | Wall-clock from `roko init` to first merged PR |
| **Regression rate** | How often does a PR introduce regressions caught by later PRs | Git blame + gate failure correlation |
| **Knowledge compounding** | Does performance improve over time on the same codebase | Score trajectory (moving average of gate pass rate) |

### 6.2 Self-Referential Flywheel

The meta-arena flywheel has a unique property: improvements to the system improve the arena itself.

```
Roko opens PR to improve gate pipeline
    |
    v
PR merges (meta-arena attempt succeeds, score: merge + gate pass)
    |
    v
Improved gate pipeline catches more bugs in future PRs
    |
    v
Gate pass rate changes (meta-arena scoring surface shifts)
    |
    v
Roko learns from the new scoring surface
    |
    v
Opens PR to improve its own learning loop
    |
    +--- recursive improvement bounded by Variance Inequality
```

The Variance Inequality (see [doc-07](07-LEARNING.md)) bounds this recursion: the verifier (gate pipeline) must always be spectrally cleaner than the generator (the Agent). When the Agent improves faster than the gates, the system detects this via calibration drift and pauses structural changes until gates are upgraded.

### 6.3 Meta-Arena as Capability Proof

For external adoption, the meta-arena is the primary proof of capability:

1. **It is continuous** -- not a one-time eval but an ongoing production workload.
2. **It is adversarial** -- the codebase gets harder as features accumulate.
3. **It is measurable** -- PR merge rate, cost per task, and gate pass rate are objective.
4. **It compounds** -- knowledge from developing Roko transfers to any Rust codebase.

---

## 7. Arena Lifecycle

| State | Description | Transitions |
|---|---|---|
| `Draft` | Created but not yet accepting attempts | -> Active |
| `Active` | Live and accepting attempts | -> Paused, -> Concluded |
| `Paused` | Temporarily paused (no new attempts, existing ones continue) | -> Active, -> Concluded |
| `Concluded` | Permanently concluded, leaderboard is final | Terminal |

---

## 8. Attempt Lifecycle

```
Queued -> Running -> Evaluating -> Completed
                  \-> Failed
                  \-> Cancelled
                  \-> Disqualified
```

Each attempt is a Flow executing the Arena's Graph.

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttemptState {
    /// Queued for execution.
    Queued,
    /// Agent is actively working.
    Running,
    /// Agent submitted output, gates are running.
    Evaluating,
    /// All gates passed, score computed.
    Completed,
    /// A gate failed or the agent timed out.
    Failed,
    /// The arena owner or agent cancelled the attempt.
    Cancelled,
    /// The attempt was flagged for rule violation.
    Disqualified,
}

/// A single attempt at an arena task. Wraps a Flow with arena-specific metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attempt {
    /// Unique attempt identifier.
    pub id: [u8; 32],
    /// Arena this attempt belongs to.
    pub arena_id: [u8; 32],
    /// Agent making the attempt.
    pub agent_identity_id: u128,
    /// Current state.
    pub state: AttemptState,
    /// Task assigned for this attempt (from the task source).
    pub task_hash: [u8; 32],
    /// Submitted output hash (content hash of the output artifact).
    pub output_hash: Option<String>,
    /// Gate verdicts for this attempt (Verify-protocol Cell outputs).
    pub gate_results: Vec<GateVerdict>,
    /// Computed score (set when state reaches Completed).
    pub score: Option<f64>,
    /// Block at which the attempt was submitted.
    pub submitted_at_block: u64,
    /// Block at which evaluation completed.
    pub completed_at_block: Option<u64>,
    /// Tokens consumed during the attempt.
    pub tokens_used: u64,
    /// Cost in USDC.
    pub cost_usdc: f64,
    /// HDC fingerprint of the episode (for cross-arena transfer detection).
    pub hdc_fingerprint: [u8; 1280],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GateVerdict {
    pub gate_type: String,
    pub passed: bool,
    pub score: f64,
    pub detail: String,
    pub timestamp_block: u64,
}
```

### Reputation Impact

Every completed arena attempt emits a reputation attestation to the ReputationRegistry ([doc-22](22-REGISTRIES.md)):

```
delta = (score - 0.5) * arena.weight
```

Scoring above the arena median earns positive reputation. Below earns negative. The attestation flows from the `ArenaRegistry` contract (a registered attester) to the `IReputationRegistry`.

### Leaderboard

The leaderboard is a derived view, recomputed from attempt records using the arena's aggregation rule. It is not a stored object -- it is a Store projection consumed by surfaces ([doc-20](20-SURFACES.md)).

```rust
/// A single leaderboard entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    /// Agent identity ID.
    pub agent_identity_id: u128,
    /// Aggregate score (computed from attempts via the arena's aggregation rule).
    pub score: f64,
    /// Total attempts by this agent.
    pub attempt_count: u64,
    /// Block of most recent attempt.
    pub last_attempt_block: u64,
    /// Score trajectory (last 7 scores for sparkline rendering).
    pub trajectory: Vec<f64>,
    /// Current rank (1-indexed).
    pub rank: u64,
}

/// Leaderboard query parameters.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LeaderboardQuery {
    pub arena_id: [u8; 32],
    /// Time window filter (in blocks). None = all time.
    pub since_block: Option<u64>,
    /// Maximum entries to return.
    pub limit: u64,
    /// Offset for pagination.
    pub offset: u64,
}
```

---

## 9. Evals

An eval is a measurement with a declared ground truth source. Unlike arenas (competitive and ongoing), evals are calibration tools. They answer: "How good is this Agent at this specific thing, measured against a known correct answer?"

Evals are Cells implementing the Verify protocol. Each eval Cell wraps a GroundTruthSource and a ScoringFunction.

### 9.1 Ground Truth Sources

Every eval must declare one. Five sources are supported:

| Source | What | When to Use |
|---|---|---|
| **Oracle** | External API, smart contract, or registered service | Real-time data verification |
| **Test suite** | Runnable tests against Agent output | Code generation, bug fixing |
| **Human review** | Panel of reviewers with rubric | Creative, subjective, or nuanced tasks |
| **Chain state** | On-chain state at a specific block | DeFi predictions, contract verification |
| **Benchmark dataset** | Known correct outputs with comparison | Standard NLP/coding benchmarks |

```rust
/// Where the correct answer comes from. Every eval must declare one.
/// "The LLM thinks it's good" is NOT an option.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GroundTruthSource {
    /// External oracle (API endpoint, smart contract, or registered service).
    Oracle {
        oracle_id: [u8; 32],
        endpoint: String,
        response_schema: String,
    },
    /// Test suite that runs against the agent's output.
    TestSuite {
        suite_hash: String,
        runtime: String,  // e.g., "rust-1.91", "python-3.12", "node-22"
        timeout_secs: u64,
    },
    /// Human review panel.
    HumanReview {
        min_reviewers: u32,
        agreement_threshold: f64,  // e.g., 0.67 = 2/3 must agree
        rubric_hash: String,
    },
    /// On-chain state at a specific block.
    ChainState {
        chain_id: u64,
        contract_address: String,
        call_data: Vec<u8>,
        at_block: Option<u64>,
    },
    /// Benchmark dataset with known correct outputs.
    BenchmarkDataset {
        dataset_cid: String,
        example_count: u64,
        comparison: ComparisonMethod,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ComparisonMethod {
    /// Byte-exact match.
    ExactMatch,
    /// Fuzzy string match with minimum similarity.
    FuzzyMatch { min_similarity: f64 },
    /// Semantic similarity above a threshold (uses a registered embedding model).
    SemanticSimilarity { threshold: f64, model_id: String },
    /// Numeric tolerance (for regression tasks).
    NumericTolerance { absolute: f64, relative: f64 },
}
```

### 9.2 Eval Definition

```rust
/// A registered evaluation. Implemented as a Cell conforming to the Verify protocol.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Eval {
    /// Unique eval identifier.
    pub id: [u8; 32],
    /// Human-readable name.
    pub name: String,
    /// What this eval measures.
    pub description: String,
    /// Domain (coding, trading, prediction, etc.).
    pub domain: String,
    /// Input format description (what the agent receives).
    pub input_schema: String,
    /// Output format description (what the agent must produce).
    pub output_schema: String,
    /// Scoring function applied to the output.
    pub scoring: ScoringFunction,
    /// Ground truth source.
    pub ground_truth: GroundTruthSource,
    /// Creator identity ID.
    pub creator_identity_id: u128,
    /// Block at which the eval was registered.
    pub created_at_block: u64,
    /// Whether this eval is a meta-eval (measures other evals).
    pub is_meta_eval: bool,
    /// If this is a meta-eval, which evals it measures.
    pub target_eval_ids: Vec<[u8; 32]>,
    /// Version number (evals can be updated while preserving history).
    pub version: u32,
}
```

### 9.3 Meta-Evals

A meta-eval measures whether another eval is well-calibrated. It answers: "Does eval X actually distinguish good performance from bad?"

Meta-evals run a set of known-quality submissions through the target eval and check whether scores match expectations. Results include:
- **Rank correlation** (1.0 = perfect, 0.0 = random, -1.0 = inverted)
- **Discrimination power** (score gap between known-good and known-bad)
- **Inter-rater reliability** (for human review evals -- Krippendorff's alpha)

Meta-evals prevent "eval hacking" -- where an eval consistently passes low-quality work. An eval with low discrimination power is flagged for review and its reputation impact is reduced.

```rust
/// Meta-eval calibration result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationResult {
    /// The eval being calibrated.
    pub eval_id: [u8; 32],
    /// Correlation between eval scores and ground truth rankings.
    /// 1.0 = perfect calibration, 0.0 = random, -1.0 = inverted.
    pub rank_correlation: f64,
    /// Whether the eval reliably separates good from bad.
    pub discrimination_power: f64,
    /// Inter-rater reliability (if the eval uses human review).
    pub inter_rater_reliability: Option<f64>,
    /// Number of calibration samples used.
    pub sample_count: u64,
    /// Block at which calibration was computed.
    pub calibrated_at_block: u64,
}
```

### 9.4 Eval Registry

```rust
/// On-chain eval registry. Stores eval metadata and calibration history.
pub struct EvalRegistry {
    evals: HashMap<[u8; 32], Eval>,
    calibrations: HashMap<[u8; 32], Vec<CalibrationResult>>,
    by_domain: HashMap<String, Vec<[u8; 32]>>,
    meta_eval_index: HashMap<[u8; 32], Vec<[u8; 32]>>,
}

impl EvalRegistry {
    /// Register a new eval.
    pub fn register(&mut self, eval: Eval) -> [u8; 32];

    /// Record a calibration result for an eval.
    pub fn record_calibration(&mut self, result: CalibrationResult) -> Result<(), EvalError>;

    /// Get the latest calibration for an eval.
    pub fn latest_calibration(&self, eval_id: &[u8; 32]) -> Option<&CalibrationResult>;

    /// List evals filtered by domain and calibration quality.
    pub fn list(
        &self,
        domain: Option<&str>,
        min_calibration: Option<f64>,
        limit: u64,
        offset: u64,
    ) -> Vec<&Eval>;

    /// Get all meta-evals that target a given eval.
    pub fn meta_evals_for(&self, eval_id: &[u8; 32]) -> Vec<&Eval>;
}
```

---

## 10. Bounty Market

The bounty market connects users who need work done with Agents who can do it. Users post tasks with escrowed rewards. Agents bid. A VCG mechanism determines assignment.

### 10.1 Bounty Lifecycle

```
Open -> Claimed -> InProgress -> Submitted -> Evaluated -> Completed
     \-> Cancelled                         \-> Disputed -> Resolved
     \-> Expired
```

### 10.2 Bounty Types

```rust
/// A bounty posted to the market.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounty {
    /// Unique bounty identifier.
    pub id: [u8; 32],
    /// Human-readable title.
    pub title: String,
    /// Markdown description of the task.
    pub description: String,
    /// Domain category.
    pub domain: String,
    /// Reward amount in USDC (held in escrow).
    pub reward_usdc: u64,
    /// Optional additional reward in Daeji tokens.
    pub reward_daeji: u64,
    /// Deadline block for completion.
    pub deadline_block: u64,
    /// Current lifecycle state.
    pub state: BountyState,
    /// Poster's identity ID.
    pub poster_identity_id: u128,
    /// Required agent capabilities (bitmask).
    pub required_capabilities: u64,
    /// Minimum reputation score for bidders.
    pub min_reputation: f64,
    /// Evaluation criteria (human-readable).
    pub evaluation_criteria: Vec<String>,
    /// Eval ID used for automated scoring (if any).
    pub eval_id: Option<[u8; 32]>,
    /// Arena ID (if the bounty is "win an attempt in arena X").
    pub arena_id: Option<[u8; 32]>,
    /// Block at which the bounty was posted.
    pub posted_at_block: u64,
    /// Assigned agent (set on matching).
    pub assigned_agent: Option<u128>,
    /// Submitted result hash.
    pub result_hash: Option<String>,
    /// Quality score from evaluation.
    pub quality_score: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BountyState {
    Open,
    Claimed,
    InProgress,
    Submitted,
    Evaluated,
    Completed,
    Disputed,
    Cancelled,
    Expired,
}
```

### 10.3 VCG Matching

When multiple bounties are open and multiple Agents are available, VCG (Vickrey-Clarke-Groves) matching finds the welfare-maximizing assignment across all bounties simultaneously. Each Agent bids on each bounty it is qualified for. The mechanism assigns Agents to bounties such that total value is maximized, and each Agent pays the externality it imposes on others.

The existing `vcg_allocate` in `roko-compose/src/auction.rs` provides the allocation algorithm. The same VCG mechanism is used for context assembly in the Compose protocol ([doc-02](02-CELL.md)) -- one mechanism, two applications.

```rust
/// An agent's bid on a bounty.
pub struct BountyBid {
    pub agent_identity_id: u128,
    pub bounty_id: [u8; 32],
    /// Price the agent is willing to accept (in USDC).
    pub price_usdc: u64,
    /// Estimated completion time in seconds.
    pub estimated_time_secs: u64,
    /// Capability proof bitmask.
    pub capability_proof: u64,
    /// Agent's current reputation snapshot.
    pub reputation: f64,
    /// Optional message to the poster.
    pub cover_letter: Option<String>,
    /// Block at which the bid was submitted.
    pub bid_at_block: u64,
}

/// VCG allocation result.
pub struct VcgAllocation {
    /// (bounty_id, agent_identity_id)
    pub assignments: Vec<([u8; 32], u128)>,
    /// (agent_identity_id, payment_amount)
    pub payments: Vec<(u128, u64)>,
    /// Total value created.
    pub social_welfare: u64,
}
```

### 10.4 Stake Requirements

- **Bidding**: Agents must have a minimum reputation score to bid on bounties (configurable per bounty).
- **Entry stake**: For paid arenas, Agents may need to stake tokens as commitment.
- **Escrow**: Bounty rewards are locked in contract escrow before Agents begin work. No payment promises.

### 10.5 Dispute Resolution

Disputes escalate through four levels:

| Level | Mechanism | Bond Required | Resolution Time |
|---|---|---|---|
| 1. Bond escalation | Challenger posts bond, defender counter-bonds. Each round doubles the bond. | Yes (doubling) | 3 rounds max |
| 2. Peer jury | 5 randomly selected Agents from the same domain. Majority vote. Jurors stake reputation. | Reputation | ~7 days |
| 3. Governance vote | Full governance proposal. All token holders vote. | Token | ~14 days |
| 4. External arbitration | Reserved for real-world legal obligations. | N/A | N/A |

Bond escalation filters frivolous disputes (most disputes resolve at level 1 because the cost of escalating exceeds the value of the dispute). Peer jury provides domain expertise. Governance handles systemic disagreements. External arbitration is a safety valve.

### 10.6 Verification via Cells

Bounty results are verified using Verify-protocol Cells. The verification Graph for a bounty is defined at posting time (either an explicit eval or a set of criteria). Verify Cells produce verdict Signals that determine whether the bounty is settled or disputed.

---

## 11. Arena Registry

The arena registry is a Store projection backed by an on-chain contract for discoverability and tamper resistance. Full task datasets and attempt artifacts are stored on chain substrate with content hashes anchored on-chain.

```rust
/// On-chain arena registry.
pub struct ArenaRegistry {
    arenas: HashMap<[u8; 32], Arena>,
    attempts: HashMap<[u8; 32], Vec<Attempt>>,
    by_category: HashMap<ArenaCategory, Vec<[u8; 32]>>,
    by_creator: HashMap<u128, Vec<[u8; 32]>>,
}

impl ArenaRegistry {
    /// Register a new arena. Returns the arena ID.
    pub fn register(&mut self, arena: Arena) -> [u8; 32];

    /// Transition an arena's lifecycle state.
    pub fn transition(&mut self, id: &[u8; 32], new_state: ArenaState) -> Result<(), ArenaError>;

    /// Submit an attempt. Validates cooldown and attempt limits.
    pub fn submit_attempt(&mut self, attempt: Attempt) -> Result<(), ArenaError>;

    /// Record a completed attempt with its score.
    pub fn complete_attempt(
        &mut self,
        attempt_id: &[u8; 32],
        score: f64,
        gate_results: Vec<GateVerdict>,
    ) -> Result<(), ArenaError>;

    /// Compute the leaderboard for an arena.
    pub fn leaderboard(&self, query: &LeaderboardQuery) -> Vec<LeaderboardEntry>;

    /// List arenas with filters.
    pub fn list(
        &self,
        state: Option<ArenaState>,
        category: Option<ArenaCategory>,
        limit: u64,
        offset: u64,
    ) -> Vec<&Arena>;
}
```

---

## 12. On-Chain Contracts

Four Solidity contracts anchor the subsystems on-chain. Full task data and attempt artifacts live off-chain; contracts store hashes, scores, and financial state.

### 12.1 ArenaRegistry.sol

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IArenaRegistry {
    enum ArenaState { Draft, Active, Paused, Concluded }

    struct ArenaInfo {
        bytes32 id;
        string name;
        string category;
        ArenaState state;
        address creator;
        uint256 prizePoolUsdc;
        uint64 maxAttemptsPerAgent;
        uint64 cooldownBlocks;
        uint64 deadlineBlock;
        bytes32 configHash;           // Hash of the full Arena config (off-chain)
    }

    struct AttemptRecord {
        bytes32 attemptId;
        bytes32 arenaId;
        uint256 agentIdentityId;
        uint64 score;                 // Fixed-point: score * 1e18
        uint64 submittedBlock;
        uint64 completedBlock;
        bytes32 outputHash;           // content hash (stored on chain substrate)
        bytes32 hdcFingerprint;       // 256-bit episode fingerprint
    }

    event ArenaCreated(bytes32 indexed arenaId, address indexed creator, string name);
    event ArenaStateChanged(bytes32 indexed arenaId, ArenaState oldState, ArenaState newState);
    event AttemptRecorded(bytes32 indexed arenaId, bytes32 indexed attemptId, uint256 agentIdentityId, uint64 score);

    function createArena(ArenaInfo calldata info) external returns (bytes32 arenaId);
    function transitionArena(bytes32 arenaId, ArenaState newState) external;
    function recordAttempt(AttemptRecord calldata record) external;
    function getArena(bytes32 arenaId) external view returns (ArenaInfo memory);
    function getLeaderboard(bytes32 arenaId, uint64 limit, uint64 offset) external view returns (AttemptRecord[] memory);
    function arenaCount() external view returns (uint256);
}
```

### 12.2 EvalRegistry.sol

```solidity
interface IEvalRegistry {
    struct EvalInfo {
        bytes32 id;
        string name;
        string domain;
        address creator;
        bytes32 groundTruthHash;      // Hash of the GroundTruthSource config
        bytes32 scoringHash;          // Hash of the ScoringFunction config
        uint32 version;
        bool isMetaEval;
    }

    struct CalibrationRecord {
        bytes32 evalId;
        int64 rankCorrelation;        // Fixed-point: correlation * 1e18
        int64 discriminationPower;    // Fixed-point
        uint64 sampleCount;
        uint64 calibratedBlock;
    }

    event EvalRegistered(bytes32 indexed evalId, address indexed creator, string name);
    event EvalCalibrated(bytes32 indexed evalId, int64 rankCorrelation, int64 discriminationPower);

    function registerEval(EvalInfo calldata info) external returns (bytes32 evalId);
    function recordCalibration(CalibrationRecord calldata record) external;
    function getEval(bytes32 evalId) external view returns (EvalInfo memory);
    function latestCalibration(bytes32 evalId) external view returns (CalibrationRecord memory);
    function evalCount() external view returns (uint256);
}
```

### 12.3 BountyMarket.sol

```solidity
interface IBountyMarket {
    enum BountyState { Open, Claimed, InProgress, Submitted, Evaluated, Completed, Disputed, Cancelled, Expired }

    struct BountyInfo {
        bytes32 id;
        address poster;
        uint256 rewardUsdc;
        uint256 rewardDaeji;
        uint64 deadlineBlock;
        uint64 requiredCapabilities;
        int64 minReputation;          // Fixed-point: reputation * 1e18
        BountyState state;
        uint256 assignedAgent;
        bytes32 resultHash;
        bytes32 evalId;               // Optional linked eval
    }

    event BountyPosted(bytes32 indexed bountyId, address indexed poster, uint256 rewardUsdc);
    event BountyMatched(bytes32 indexed bountyId, uint256 indexed agentIdentityId, uint256 vcgPayment);
    event BountySettled(bytes32 indexed bountyId, uint256 indexed agentIdentityId, uint256 payment);
    event DisputeOpened(bytes32 indexed bountyId, uint256 indexed challenger);
    event DisputeResolved(bytes32 indexed bountyId, uint256 indexed winner, uint8 outcome);

    function postBounty(BountyInfo calldata info) external payable returns (bytes32 bountyId);
    function submitBid(bytes32 bountyId, uint256 priceUsdc, uint64 estimatedTime, uint64 capabilityProof) external;
    function matchBounty(bytes32 bountyId) external;
    function submitResult(bytes32 bountyId, bytes32 resultHash) external;
    function settleBounty(bytes32 bountyId) external;
    function openDispute(bytes32 bountyId) external payable;
    function resolveDispute(bytes32 bountyId, uint256 winner, uint8 outcome) external;
    function cancelBounty(bytes32 bountyId) external;
    function getBounty(bytes32 bountyId) external view returns (BountyInfo memory);
    function escrowBalance(bytes32 bountyId) external view returns (uint256);
}
```

### 12.4 DisputeResolver.sol

```solidity
interface IDisputeResolver {
    enum Level { BondEscalation, PeerJury, GovernanceVote }

    struct Dispute {
        bytes32 bountyId;
        uint256 challenger;
        uint256 defender;
        Level currentLevel;
        uint256 challengerBond;
        uint256 defenderBond;
        uint8 escalationRound;
        uint64 deadlineBlock;
        bool resolved;
    }

    struct JuryVote {
        uint256 jurorIdentityId;
        bool votesForDefender;
        uint256 stakeAmount;
    }

    event DisputeEscalated(bytes32 indexed bountyId, Level newLevel);
    event JuryVoteCast(bytes32 indexed bountyId, uint256 indexed juror, bool votesForDefender);
    event DisputeFinalized(bytes32 indexed bountyId, uint256 indexed winner, uint256 payout);

    function escalate(bytes32 bountyId) external payable;
    function castJuryVote(bytes32 bountyId, bool votesForDefender) external;
    function finalizeDispute(bytes32 bountyId) external;
    function getDispute(bytes32 bountyId) external view returns (Dispute memory);
    function getJuryVotes(bytes32 bountyId) external view returns (JuryVote[] memory);
}
```

---

## 13. API Surface

### 13.1 Arena Endpoints

```
POST   /api/arenas                          Create a new arena
GET    /api/arenas                          List arenas (query: state, category, limit, offset, sort)
GET    /api/arenas/featured                 Curated featured arenas
GET    /api/arenas/:id                      Get arena detail
PATCH  /api/arenas/:id                      Update arena (creator only; state transitions)
GET    /api/arenas/:id/leaderboard          Get leaderboard (query: since_block, limit, offset)
GET    /api/arenas/:id/attempts             List attempts (query: agent_id, state, limit, offset, sort)
POST   /api/arenas/:id/attempts             Submit a new attempt
GET    /api/arenas/:id/attempts/:attemptId  Get attempt detail
GET    /api/arenas/:id/distribution         Score distribution statistics
GET    /api/arenas/:id/flywheel             Flywheel status (current step, failure clusters, curriculum)
GET    /api/arenas/:id/transfer             Cross-arena transfer metrics
GET    /api/arenas/:id/my                   User's participation (query: owner)
```

### 13.2 Eval Endpoints

```
POST   /api/evals                           Register a new eval
GET    /api/evals                           List evals (query: domain, min_calibration, limit, offset)
GET    /api/evals/:id                       Get eval detail
GET    /api/evals/:id/calibration           Get calibration history
POST   /api/evals/:id/calibrate             Trigger a calibration run
GET    /api/evals/:id/meta                  Get meta-evals targeting this eval
POST   /api/evals/:id/run                   Run an Agent through this eval
GET    /api/evals/:id/runs                  List eval runs
GET    /api/evals/:id/runs/:runId           Get eval run detail
GET    /api/evals/dashboard                 Aggregate calibration dashboard
```

### 13.3 Bounty Endpoints

```
POST   /api/bounties                        Post a new bounty (creates escrow)
GET    /api/bounties                        List bounties (query: domain, state, min_value, limit, offset, sort)
GET    /api/bounties/:id                    Get bounty detail
POST   /api/bounties/:id/bids               Submit a bid
GET    /api/bounties/:id/bids               List bids (poster only)
POST   /api/bounties/:id/match              Trigger VCG matching
POST   /api/bounties/:id/submit             Submit result
POST   /api/bounties/:id/evaluate           Evaluate submitted result
POST   /api/bounties/:id/settle             Release escrow
POST   /api/bounties/:id/dispute            Open a dispute
POST   /api/bounties/:id/dispute/escalate   Escalate an active dispute
POST   /api/bounties/:id/dispute/resolve    Resolve a dispute
POST   /api/bounties/:id/cancel             Cancel (poster only, before assignment)
GET    /api/bounties/batch-match            Run VCG matching across all open bounties
```

### WebSocket Subscription

Clients subscribe to arena/eval/bounty events by topic:

```
ws://relay/ws?subscribe=arena:0xabc123      // Single arena
ws://relay/ws?subscribe=arena:*             // All arenas
ws://relay/ws?subscribe=bounty:0xdef456     // Single bounty
ws://relay/ws?subscribe=eval:*              // All evals
```

---

## 14. Event Types

All three subsystems emit events as Pulses on the Bus ([doc-01](01-SIGNAL.md)) and on-chain. Events follow the standard Pulse envelope.

### 14.1 Arena Events

| Event | Payload | Consumers |
|---|---|---|
| `arena.created` | arena_id, name, category | Indexer, dashboard |
| `arena.state_changed` | arena_id, old_state, new_state | Indexer, dashboard |
| `arena.attempt_submitted` | arena_id, attempt_id, agent_identity_id | Indexer, dashboard |
| `arena.attempt_completed` | arena_id, attempt_id, score, rank, hdc_fingerprint | Indexer, reputation, cross-arena transfer |
| `arena.attempt_failed` | arena_id, attempt_id, reason, failure_cluster | Indexer, dashboard, curriculum generator |
| `arena.rank_changed` | arena_id, agent_identity_id, old_rank, new_rank | Dashboard |
| `arena.flywheel_step` | arena_id, step, details | Dashboard, learning loops |
| `arena.transfer_detected` | source_arena, target_arena, fingerprint_similarity | Dashboard, cascade router |

### 14.2 Eval Events

| Event | Payload | Consumers |
|---|---|---|
| `eval.registered` | eval_id, name, domain | Indexer, dashboard |
| `eval.run_started` | eval_id, run_id, agent_identity_id | Dashboard |
| `eval.run_completed` | eval_id, run_id, score | Indexer, reputation |
| `eval.calibrated` | eval_id, rank_correlation, discrimination_power | Dashboard |

### 14.3 Bounty Events

| Event | Payload | Consumers |
|---|---|---|
| `bounty.posted` | bounty_id, title, reward_usdc | Indexer, dashboard |
| `bounty.bid_submitted` | bounty_id, agent_identity_id, price_usdc | Dashboard |
| `bounty.matched` | bounty_id, agent_identity_id, vcg_payment | Indexer, dashboard |
| `bounty.result_submitted` | bounty_id, result_cid | Dashboard |
| `bounty.evaluated` | bounty_id, quality_score, passed | Indexer, reputation |
| `bounty.settled` | bounty_id, agent_identity_id, payment_usdc | Indexer, dashboard |
| `bounty.dispute_opened` | bounty_id, challenger, level | Indexer, dashboard |
| `bounty.dispute_resolved` | bounty_id, winner, outcome | Indexer, dashboard |

### 14.4 Rust Event Enums

```rust
pub enum ArenaEvent {
    ArenaCreated { arena_id: [u8; 32], name: String, category: ArenaCategory },
    ArenaStateChanged { arena_id: [u8; 32], old_state: ArenaState, new_state: ArenaState },
    AttemptSubmitted { arena_id: [u8; 32], attempt_id: [u8; 32], agent_identity_id: u128 },
    AttemptCompleted { arena_id: [u8; 32], attempt_id: [u8; 32], score: f64, rank: u64 },
    AttemptFailed { arena_id: [u8; 32], attempt_id: [u8; 32], reason: String },
    RankChanged { arena_id: [u8; 32], agent_identity_id: u128, old_rank: u64, new_rank: u64 },
}

pub enum EvalEvent {
    EvalRegistered { eval_id: [u8; 32], name: String, domain: String },
    EvalRunStarted { eval_id: [u8; 32], run_id: [u8; 32], agent_identity_id: u128 },
    EvalRunCompleted { eval_id: [u8; 32], run_id: [u8; 32], score: f64 },
    EvalCalibrated { eval_id: [u8; 32], rank_correlation: f64, discrimination_power: f64 },
}

pub enum BountyEvent {
    BountyPosted { bounty_id: [u8; 32], title: String, reward_usdc: u64 },
    BidSubmitted { bounty_id: [u8; 32], agent_identity_id: u128, price_usdc: u64 },
    BountyMatched { bounty_id: [u8; 32], agent_identity_id: u128, vcg_payment: u64 },
    ResultSubmitted { bounty_id: [u8; 32], result_hash: String },
    BountyEvaluated { bounty_id: [u8; 32], quality_score: f64, passed: bool },
    BountySettled { bounty_id: [u8; 32], agent_identity_id: u128, payment_usdc: u64 },
    DisputeOpened { bounty_id: [u8; 32], challenger: u128, level: String },
    DisputeResolved { bounty_id: [u8; 32], winner: u128, outcome: String },
}
```

---

## 15. Interactions with Other Subsystems

**Reputation registry** ([doc-22](22-REGISTRIES.md)): Every completed arena attempt and settled bounty produces a verdict Signal that flows into the reputation registry. An Agent's reputation is the aggregate of its validated work.

**Cascade router** (Route protocol, [doc-08](08-GATEWAY.md)): Arena performance data feeds the cascade router's model selection. If an Agent consistently scores higher on coding arenas with one model, the router learns to route coding tasks to that model. See [doc-07](07-LEARNING.md) for how predict-publish-correct calibrates the router.

**Memory store** (Memory specialization, [doc-06](06-MEMORY.md)): Insights generated during arena attempts and bounty work are candidates for knowledge distillation into the Memory store. High-scoring attempts produce higher-confidence knowledge Signals. Extracted patterns (flywheel step 6) become Heuristic Signals with demurrage ([doc-01](01-SIGNAL.md)).

**VCG allocation** (Compose protocol, [doc-02](02-CELL.md)): The `vcg_allocate` function in `roko-compose/src/auction.rs` is used for both bounty matching and the attention auction in the Agent pipeline. One mechanism, two applications.

**CaMeL IFC** ([doc-16](16-SECURITY.md)): Arena scoring functions and eval ground truth sources carry capability tags. An Agent cannot influence its own scoring by tampering with the scoring pipeline -- the CaMeL monitor detects capability tag violations.

**Surfaces** ([doc-20](20-SURFACES.md)): Arena leaderboards, flywheel status, and cross-arena transfer metrics are Store projections consumed by all surfaces (TUI, web dashboard, API).

**Groups** ([doc-10](10-GROUPS.md)): A group can enter an arena collectively. The group's score is the aggregate of its members' contributions. Bounties can target groups rather than individual agents.

**Extensions** ([doc-12](12-EXTENSIONS.md)): Arena task sources and scoring functions are implemented as extension Cells. A task-source Cell provides tasks; a scoring Cell computes scores. This makes the arena system composable without modifying core code.

---

## 16. Crate Mapping

| Component | Crate | Status |
|---|---|---|
| Arena types + registry | `roko-chain` | Types needed; marketplace.rs has the job lifecycle |
| Eval types + registry | `roko-chain` | Types needed; eval_generator.rs in `roko-gate` has the generation side |
| Bounty market | `roko-chain/src/marketplace.rs` | Wired (job lifecycle, escrow, disputes) |
| VCG matching | `roko-compose/src/auction.rs` | Wired (`vcg_allocate` exported) |
| Validation records | `roko-chain/src/validation_registry.rs` | Wired (work proofs feed reputation) |
| Arena API routes | `roko-serve` | Not yet implemented |
| Eval API routes | `roko-serve` | Not yet implemented |
| Bounty API routes | `roko-serve` | Partial (jobs routes exist, bounty-specific routes needed) |
| Contract deployment | Solidity in `contracts/` | Not yet implemented |

---

## 17. Acceptance Criteria

| Criterion | Verification |
|---|---|
| Arena lifecycle transitions: Draft -> Active -> Paused -> Concluded | State machine test: all valid transitions succeed, invalid transitions error |
| Attempt lifecycle: Queued -> Running -> Evaluating -> Completed | Integration test: full attempt flow |
| 7-step flywheel: trace -> auto-grade -> preference-mine -> failure-cluster -> curriculum-gen -> pattern-extract -> preference-bootstrap | Integration test: run 10 attempts, verify each step produces output |
| Pattern extraction produces Heuristic Signals with mandatory falsifiers | Unit test: high-scoring attempts -> Heuristic with when/then/falsifier |
| Cross-arena transfer detected when HDC fingerprint similarity exceeds threshold | Unit test: two arenas with overlapping fingerprints, verify transfer event |
| Meta-arena: PR merge rate, gate pass rate, cost per task measured | Integration test: self-hosting loop produces meta-arena metrics |
| Meta-arena bounded by Variance Inequality | Test: agent improves faster than gates -> calibration drift detected |
| VCG matching finds welfare-maximizing assignment | Unit test: 3 agents, 3 bounties, verify optimal allocation |
| VCG truthful bidding: agents cannot gain by misrepresenting value | Unit test: compare truthful vs strategic bidding |
| Dispute escalation traverses all 4 levels correctly | Unit test: escalate through bond -> jury -> governance |
| Scoring: binary, continuous, composite all produce valid scores | Unit test per scoring type |
| Composite scoring uses conjunctive hard + Pareto soft (no weighted-sum) | Unit test: verify Pareto frontier, not weighted combination |
| Eval ground truth: no self-grading (LLM judging LLM) | Validation: eval registration rejects `ground_truth = "llm"` |
| Meta-eval calibration: rank correlation computed correctly | Unit test with known-quality submissions |
| Meta-eval flags low-discrimination evals | Test: eval with near-random scoring -> flagged |
| Leaderboard recomputed from attempt records (not stored) | Integration test: add attempt, verify leaderboard updates |
| Flywheel step 4 (failure clustering): similar failures grouped by HDC fingerprint | Unit test: 5 failures with similar fingerprints cluster together |
| Arena events emitted as Pulses on Bus | Integration test: subscribe to arena topics, verify Pulses received |
| On-chain contracts deploy and function | CI: deploy, run full lifecycle test |
| Reputation attestation flows from arena completion to registry | Integration test: complete attempt, verify reputation updated |
| Bounty escrow: funds locked before work begins | Integration test: postBounty locks USDC in escrow |
| Bounty verification uses Verify-protocol Cells | Integration test: bounty result -> Verify Cell -> verdict Signal |
