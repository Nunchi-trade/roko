# 03 — The Gate Pipeline

> **Layer**: L3 Harness — Verification
> **Crate**: `roko-gate` (`crates/roko-gate/src/gate_pipeline.rs`)
> **Status**: Implemented (593 lines), wired into orchestrate.rs


> **Implementation**: Shipping

---

## 1. Overview

The `GatePipeline` composes multiple gates into a single verification step. It accepts a
`Vec<Box<dyn Gate>>`, runs them sequentially, and produces an aggregated `Verdict`. It
implements the `Gate` trait itself, so a pipeline can be nested inside another pipeline
or used anywhere a single gate is expected.

This is the primary mechanism for turning the rung selector's output (a list of rungs)
into a concrete verification execution.

> **Citation**: crates/roko-gate/src/gate_pipeline.rs — Full implementation.

---

## 2. Structure

```rust
pub struct GatePipeline {
    gates: Vec<Box<dyn Gate>>,
    short_circuit: bool,
    name: String,
}
```

| Field | Purpose |
|---|---|
| `gates` | Ordered list of gates to execute |
| `short_circuit` | If true, stop on first failure |
| `name` | Display name for the pipeline's own verdict |

### Construction

```rust
GatePipeline::new(vec![
    Box::new(CompileGate::cargo()),
    Box::new(ClippyGate::cargo()),
    Box::new(TestGate::cargo()),
])
.with_short_circuit(true)
.with_name("rung-pipeline")
```

The builder pattern follows the same convention as individual gates: `with_*` methods
return `Self` for chaining.

---

## 3. Short-Circuit vs. Full Execution

### 3.1 Short-Circuit Mode (`short_circuit: true`)

The default and most common mode. The pipeline stops at the first gate that fails and
returns a failure verdict immediately. This is the correct behavior for the rung
pipeline: if compile fails, there is no point running lint or tests.

```
CompileGate → FAIL → stop → return Verdict::fail(...)
              (ClippyGate and TestGate never run)
```

**Why this matters**: In a 7-rung pipeline where integration tests take 30 minutes, a
compile failure caught in 3 seconds saves 30+ minutes of wasted compute. Short-circuit
mode is the mechanism that makes the verification-first architecture efficient.

### 3.2 Full Execution Mode (`short_circuit: false`)

All gates run regardless of individual outcomes. The final verdict is a failure if *any*
gate failed. This mode is useful when you want a comprehensive report of all issues
rather than just the first.

```
CompileGate → FAIL → continue
ClippyGate  → PASS → continue
TestGate    → FAIL → continue
→ return aggregated Verdict::fail(...)
  (detail includes output from all three gates)
```

---

## 4. Verdict Aggregation

The pipeline's `verify()` method aggregates individual verdicts into a single
pipeline-level verdict:

### 4.1 Pass Condition

The pipeline passes if and only if **every** gate passes. A single failure anywhere in
the chain makes the whole pipeline fail.

### 4.2 Detail Aggregation

Individual gate outputs are concatenated into the pipeline's `detail` field, separated
by headers:

```
--- [compile:cargo] ---
Compiling foo v0.1.0
Finished dev in 2.3s

--- [clippy:cargo] ---
warning: unused variable

--- [test:cargo] ---
test result: ok. 12 passed; 0 failed; 0 ignored
```

This gives the caller (and the agent) a complete view of what happened at each
verification step.

### 4.3 Reason Construction

On failure, the pipeline's `reason` field lists which gate(s) failed:

```
gate pipeline failed: compile:cargo (error: bad thing; error[E0425]: symbol not found)
```

In short-circuit mode, there is exactly one failed gate. In full-execution mode, there
may be multiple.

### 4.4 Duration

The pipeline's `duration_ms` is the sum of all individual gate durations. This tracks
total wall-clock time spent on verification.

### 4.5 Test Count Merging

If any gate in the pipeline produces `TestCount` (test gates do), the pipeline merges
them by summing passed, failed, and ignored counts across all gates. This is relevant
when a pipeline contains multiple test gates (e.g., unit tests + integration tests).

---

## 5. The Pipeline as a Gate

`GatePipeline` implements `Gate`:

```rust
#[async_trait]
impl Gate for GatePipeline {
    async fn verify(&self, signal: &Signal, ctx: &Context) -> Verdict {
        // ... iterate over self.gates, aggregate verdicts
    }

    fn name(&self) -> &str {
        &self.name
    }
}
```

This composability is intentional. It means:
- A pipeline can contain other pipelines (nesting)
- Any code that accepts `&dyn Gate` can accept a pipeline
- The adaptive threshold system, ratchet, and feedback systems all work with pipelines
  without special-casing

---

## 6. Execution Flow

```
Pipeline::verify(signal, ctx)
│
├─ gate[0].verify(signal, ctx) → verdict_0
│  ├─ if failed && short_circuit → return fail verdict
│  └─ collect detail, test counts
│
├─ gate[1].verify(signal, ctx) → verdict_1
│  ├─ if failed && short_circuit → return fail verdict
│  └─ collect detail, test counts
│
├─ ... (for each gate)
│
└─ aggregate:
   ├─ passed = all verdicts passed
   ├─ reason = join failure reasons
   ├─ detail = join all details with headers
   ├─ duration = sum of durations
   ├─ test_count = sum of test counts
   └─ return aggregated Verdict
```

---

## 7. How the Orchestrator Uses the Pipeline

In `crates/roko-cli/src/orchestrate.rs`, the orchestrator constructs a pipeline per task:

```
1. Determine plan complexity (Trivial/Simple/Standard/Complex)
2. Detect environment capabilities (which build tools exist)
3. select_rungs(complexity, caps, prior_failures)
4. Map each Rung to a concrete Box<dyn Gate>
5. GatePipeline::new(gates).with_short_circuit(true)
6. pipeline.verify(signal, ctx)
7. Feed verdict to ratchet, thresholds, feedback
```

The pipeline is constructed fresh for each task execution. This means:
- Different tasks can have different pipelines (based on complexity)
- Escalation adds gates to the pipeline on retry
- The pipeline is lightweight — no persistent state

---

## 8. Error Handling Within the Pipeline

Because the Gate trait returns `Verdict` (not `Result<Verdict>`), the pipeline never
has to handle gate errors. Every gate handles its own infrastructure failures internally.
The pipeline simply collects verdicts and aggregates them.

This is the practical benefit of the `-> Verdict` design decision: composition is
trivial. There are no error propagation paths to worry about, no `?` operators, no
`Result::map` chains. Just: run the gate, get a verdict, check if it passed.

> **Citation**: 00-gate-trait.md — "Gate failure is not an error — it is a verdict."

---

## 9. Pipeline Lifecycle in the Universal Loop

```
Universal loop: query → score → route → compose → act → VERIFY → write → react
                                                        ^^^^^^^^
                                                   GatePipeline lives here

Signal produced by agent (act step)
    ↓
GatePipeline.verify(signal, ctx)
    ↓
Verdict flows to:
    ├─ write: Verdict persisted as signal in Substrate
    ├─ react: GateRatchet.record_pass(plan_id, rung)
    ├─ react: AdaptiveThresholds.update(rung, passed)
    ├─ react: GateFeedback for agent context on retry
    ├─ react: EfficiencyEvent for learning
    └─ react: CascadeRouter.update_arm(model, reward)
```

The pipeline is the single point where all these downstream systems get their input.
This centralization means there's one place to add new feedback consumers, one place
to add instrumentation, and one place to add logging.

> **Citation**: refactoring-prd/01-synapse-architecture.md — Cybernetic feedback loops
> from Gate to Scorer, Router, Composer.

---

## 10. Testing the Pipeline

The pipeline has extensive tests in `gate_pipeline.rs`:

| Test | What It Verifies |
|---|---|
| `pipeline_empty_passes` | Empty pipeline returns pass verdict |
| `pipeline_single_pass` | Single passing gate → pass |
| `pipeline_single_fail` | Single failing gate → fail |
| `pipeline_short_circuits` | Stops at first failure |
| `pipeline_full_execution` | Runs all gates when short_circuit=false |
| `pipeline_aggregates_test_counts` | Merges test counts across gates |
| `pipeline_detail_headers` | Detail output has per-gate headers |
| `pipeline_duration_sums` | Total duration = sum of gate durations |

These tests use mock gates that return predetermined verdicts, avoiding the need for
actual subprocess spawning in unit tests.

---

## 11. Relationship to Other Components

| Component | Relationship |
|---|---|
| `RungSelector` | Determines which gates go into the pipeline |
| `GateRatchet` | Consumes pipeline verdicts to track regression |
| `AdaptiveThresholds` | Consumes pipeline verdicts for per-rung EMA |
| `GateFeedback` | Parses pipeline detail output for agent context |
| `ArtifactStore` | Future: stores pipeline artifacts content-addressed |
| Orchestrator | Constructs and executes the pipeline per task |

> **Citation**: crates/roko-gate/src/gate_pipeline.rs — Tests demonstrating pipeline
> behavior.

---

## 12. Design Rationale: Sequential, Not Parallel

The current pipeline executes gates sequentially. This is deliberate:

1. **Dependency ordering**: Rung N often depends on Rung N-1's success. Running tests
   on code that doesn't compile wastes time and produces confusing errors.
2. **Short-circuit value**: Sequential execution enables short-circuit, which is the
   pipeline's primary optimization.
3. **Simplicity**: Sequential execution has no synchronization concerns.

Future: A "gate group" concept where independent gates within the same rung run in
parallel. For example, if a rung has both a symbol gate and a format gate (both zero
cost, no dependency between them), they could run concurrently. This would be a new
composition primitive, not a change to the pipeline's sequential semantics.

---

## 13. Gate Composition Algebra

The pipeline's sequential composition is one combinator. A complete algebra over gates
enables richer verification topologies — parallel fan-outs, voting, fallback chains, and
confidence-weighted verdicts. The algebra treats `Gate` as the base type and defines
combinators that produce new gates from existing ones.

> **Citation**: Foundational Property-Based Testing (Paraskevopoulou & Hritcu) — formal
> compositional structures over verification predicates.

### 13.1 Verdict Lattice

Verdicts form a bounded lattice ordered by severity:

```
Skip < Warn < Pass < Fail

identity:  Skip  (a gate that produces Skip has no effect)
absorber:  Fail  (once present, dominates any merge)
merge(v1, v2) = max(v1, v2)   -- most severe verdict wins
```

This gives a monoid over verdicts: `(Verdict, merge, Skip)`. Every combinator preserves
this structure — the composed gate always returns a single `Verdict` from the same lattice.

```rust
/// Extended verdict with a confidence interval, not just pass/fail.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub enum VerdictSeverity {
    Skip = 0,
    Warn = 1,
    Pass = 2,
    Fail = 3,
}

impl VerdictSeverity {
    pub fn merge(self, other: Self) -> Self {
        if self as u8 >= other as u8 { self } else { other }
    }
}
```

### 13.2 Combinators

```rust
/// Sequential composition: g1 THEN g2 (short-circuit on fail).
/// This is what GatePipeline already does.
pub struct Sequential(Vec<Box<dyn Gate>>);

/// Parallel composition: run all gates concurrently, merge verdicts.
/// Independent gates (e.g., SymbolGate + DiffGate) run simultaneously.
pub struct Parallel(Vec<Box<dyn Gate>>);

/// Fallback: try g1; if it fails, try g2 instead.
/// Useful for degraded environments (e.g., no clippy → fall back to grep-based lint).
pub struct Fallback(Box<dyn Gate>, Box<dyn Gate>);

/// Voting: run N gates, pass if >= K pass (quorum).
/// Useful for LLM judge panels where individual judges may disagree.
pub struct Voting {
    gates: Vec<Box<dyn Gate>>,
    quorum: usize, // minimum passes required
}

/// Weighted: scale a gate's confidence by a factor.
/// Low-confidence gates (LLM judge) contribute less to aggregate decisions.
pub struct Weighted {
    gate: Box<dyn Gate>,
    weight: f64, // [0.0, 1.0]
}

/// Threshold: pass only if gate's score exceeds a minimum.
/// Converts continuous scores into binary verdicts at a chosen cut-point.
pub struct Threshold {
    gate: Box<dyn Gate>,
    min_score: f32, // gate.score must be >= this
}
```

### 13.3 Parallel Gate Group

The `Parallel` combinator enables concurrent execution of independent gates within
a single rung:

```rust
#[async_trait]
impl Gate for Parallel {
    async fn verify(&self, signal: &Signal, ctx: &Context) -> Verdict {
        let futures: Vec<_> = self.0.iter()
            .map(|g| g.verify(signal, ctx))
            .collect();
        let verdicts = futures::future::join_all(futures).await;

        let passed = verdicts.iter().all(|v| v.passed);
        let duration = verdicts.iter().map(|v| v.duration_ms).max().unwrap_or(0);
        let test_count = merge_test_counts(&verdicts);
        let detail = verdicts.iter()
            .map(|v| format!("--- [{}] ---\n{}", v.gate, v.detail.as_deref().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("\n\n");

        Verdict {
            passed,
            gate: "parallel-group".into(),
            reason: if passed { "all gates passed".into() }
                    else { format!("{} gate(s) failed",
                        verdicts.iter().filter(|v| !v.passed).count()) },
            detail: Some(detail),
            duration_ms: duration, // wall-clock = max, not sum
            test_count,
            ..Default::default()
        }
    }
}
```

Key difference: duration is `max(durations)` not `sum(durations)`, because gates run
concurrently. A `Parallel(SymbolGate, DiffGate)` taking 50ms and 20ms respectively
completes in 50ms, not 70ms.

### 13.4 Voting Gate (Quorum)

For subjective gates (LLM judges, heuristic checks), a single verdict is noisy. A
voting gate runs multiple judges and requires a quorum:

```rust
#[async_trait]
impl Gate for Voting {
    async fn verify(&self, signal: &Signal, ctx: &Context) -> Verdict {
        let futures: Vec<_> = self.gates.iter()
            .map(|g| g.verify(signal, ctx))
            .collect();
        let verdicts = futures::future::join_all(futures).await;

        let pass_count = verdicts.iter().filter(|v| v.passed).count();
        let passed = pass_count >= self.quorum;
        let avg_score = verdicts.iter().map(|v| v.score).sum::<f32>()
            / verdicts.len() as f32;

        Verdict {
            passed,
            score: avg_score,
            gate: "voting-panel".into(),
            reason: format!("{}/{} passed (quorum: {})",
                pass_count, verdicts.len(), self.quorum),
            ..Default::default()
        }
    }
}
```

### 13.5 Composition Examples

```rust
// Standard 4-rung pipeline (current behavior)
Sequential(vec![CompileGate, ClippyGate, TestGate, SymbolGate])

// Parallel lint: clippy + format check run concurrently
Sequential(vec![
    CompileGate,
    Parallel(vec![ClippyGate, FormatGate]),
    TestGate,
])

// LLM judge panel: 3 cheap judges, pass if 2+ agree
Voting { gates: vec![Judge1, Judge2, Judge3], quorum: 2 }

// Degraded environment: try clippy, fall back to grep-based lint
Fallback(ClippyGate, GrepLintGate)

// Progressive: compile → (clippy ∥ diff) → test → (symbol ∥ generated) → property → integration
Sequential(vec![
    CompileGate,
    Parallel(vec![ClippyGate, DiffGate]),
    TestGate,
    Parallel(vec![SymbolGate, GeneratedTestGate]),
    PropertyTestGate,
    IntegrationGate,
])
```

---

## 14. Probabilistic Gates

Standard gates return binary pass/fail. Probabilistic gates return a confidence
interval — a range `[lower, upper]` expressing how certain the gate is about its verdict.
This is essential for gates that use sampling (property-based tests, fuzz tests) where
the result is inherently statistical.

> **Citation**: Sequential Analysis (Siegmund, Springer) — confidence-interval stopping
> rules. Wilson score interval for proportion estimation with small samples.

### 14.1 Confidence Interval Structure

```rust
/// A verdict with statistical confidence bounds.
#[derive(Debug, Clone)]
pub struct ProbabilisticVerdict {
    /// The point estimate of the pass rate.
    pub pass_rate: f64,
    /// Lower bound of the confidence interval.
    pub ci_lower: f64,
    /// Upper bound of the confidence interval.
    pub ci_upper: f64,
    /// Confidence level (e.g., 0.95 for 95% CI).
    pub confidence_level: f64,
    /// Number of samples taken.
    pub sample_count: u64,
    /// Whether the gate passed at the chosen threshold.
    pub passed: bool,
    /// Standard Verdict for pipeline compatibility.
    pub verdict: Verdict,
}
```

### 14.2 Wilson Score Interval

The Wilson score interval is preferred over the naive Wald interval because it is
well-calibrated even at small sample sizes and near boundary proportions (p ≈ 0 or 1):

```rust
/// Compute Wilson score confidence interval for a proportion.
///
/// Parameters:
///   successes: number of passing tests
///   total: total number of tests run
///   z: z-score for desired confidence (1.96 for 95%, 2.576 for 99%)
///
/// Returns: (lower_bound, upper_bound)
fn wilson_interval(successes: u64, total: u64, z: f64) -> (f64, f64) {
    let n = total as f64;
    let p_hat = successes as f64 / n;
    let z2 = z * z;

    let denominator = 1.0 + z2 / n;
    let center = (p_hat + z2 / (2.0 * n)) / denominator;
    let margin = (z / denominator)
        * ((p_hat * (1.0 - p_hat) / n) + (z2 / (4.0 * n * n))).sqrt();

    ((center - margin).max(0.0), (center + margin).min(1.0))
}
```

### 14.3 Sequential Stopping Rule

Property-based tests and fuzz tests can use sequential hypothesis testing to stop
early when the outcome is statistically clear, rather than running a fixed number
of iterations:

```rust
/// Sequential probabilistic gate that stops when confidence is sufficient.
pub struct SequentialPropertyGate {
    /// Property to test.
    pub property: Box<dyn PropertyFn>,
    /// Desired confidence level.
    pub confidence: f64,         // default: 0.95
    /// Acceptable failure rate.
    pub acceptable_error: f64,   // default: 0.01 (1%)
    /// Maximum iterations before giving up.
    pub max_iterations: u64,     // default: 10_000
    /// z-score for the confidence level (precomputed).
    pub z_score: f64,            // 1.96 for 95%
}

/// Pseudocode for sequential verification:
///
/// n = 0, pass_count = 0
/// loop:
///     input = generate_random()
///     if property(input):
///         pass_count += 1
///     n += 1
///
///     (lower, upper) = wilson_interval(pass_count, n, z_score)
///
///     if lower > (1.0 - acceptable_error):
///         return Pass with confidence=lower
///         // CI lower bound exceeds threshold → statistically confident it passes
///
///     if upper < (1.0 - acceptable_error):
///         return Fail with confidence=(1.0 - upper)
///         // CI upper bound below threshold → statistically confident it fails
///
///     if n >= max_iterations:
///         return Warn with confidence=pass_count/n
///         // Inconclusive after max iterations
```

### 14.4 Fuzz Gate with Probabilistic Bounds

```rust
/// Coverage-guided fuzzing as a probabilistic gate.
pub struct FuzzGate {
    /// Fuzz target name (cargo-fuzz target).
    pub target: String,
    /// Maximum wall-clock duration.
    pub max_duration: Duration,      // default: 30s
    /// Corpus directory for seed inputs.
    pub corpus_dir: Option<PathBuf>,
    /// Minimum executions before declaring pass.
    pub min_executions: u64,         // default: 1_000
}

/// FuzzGate returns a probabilistic verdict:
///
/// result = cargo_fuzz_run(target, max_time=duration, corpus=corpus_dir)
/// if result.crashes > 0:
///     return Fail(
///         crashes=result.crashes,
///         minimized_inputs=result.artifacts,
///         confidence=1.0  // crash is definitive
///     )
/// confidence = 1.0 - (1.0 / result.total_runs as f64)
/// // With N executions and 0 crashes, P(no bug) ≈ 1 - 1/N
/// return Pass(
///     executions=result.total_runs,
///     coverage_delta=result.new_edges,
///     confidence=confidence,
/// )
```

### 14.5 Probabilistic Verdict → Standard Verdict

For pipeline compatibility, every probabilistic verdict converts to a standard `Verdict`:

```rust
impl From<ProbabilisticVerdict> for Verdict {
    fn from(pv: ProbabilisticVerdict) -> Self {
        Verdict {
            passed: pv.passed,
            score: pv.ci_lower as f32, // conservative: use lower bound
            gate: pv.verdict.gate,
            reason: format!(
                "pass_rate={:.3} CI=[{:.3}, {:.3}] ({}% confidence, n={})",
                pv.pass_rate, pv.ci_lower, pv.ci_upper,
                (pv.confidence_level * 100.0) as u32, pv.sample_count
            ),
            detail: pv.verdict.detail,
            duration_ms: pv.verdict.duration_ms,
            test_count: pv.verdict.test_count,
            error_digest: pv.verdict.error_digest,
        }
    }
}
```

The `score` field uses the lower bound of the CI — the most conservative estimate.
This means the downstream adaptive threshold and process reward systems operate on
worst-case estimates, not optimistic point estimates.

---

## 15. Progressive Delivery Pipeline

Adapted from canary deployment strategies (Argo Rollouts, Flagger), a progressive
delivery pipeline increases verification depth in phases, with automatic rollback
on failure at any phase.

> **Citation**: "Progressive Delivery in CI/CD Pipelines" (IJISAE, 2024) — canary,
> blue-green, and feature-flag strategies in production CI/CD.

### 15.1 Phase Structure

```rust
/// A progressive gate pipeline that increases verification depth in stages.
pub struct ProgressivePipeline {
    /// Phases ordered by increasing depth / cost.
    pub phases: Vec<ProgressivePhase>,
    /// Name for this pipeline.
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ProgressivePhase {
    /// Human-readable phase label (e.g., "Smoke", "Standard", "Deep").
    pub label: String,
    /// The gate (or composed gate) for this phase.
    /// Uses Box<dyn Gate> so it can be a single gate or a Parallel/Sequential.
    pub gate: Box<dyn Gate>,
    /// Blast radius fraction [0.0, 1.0] — informational, for logging.
    pub blast_radius: f64,
    /// Minimum duration to hold at this phase before advancing (optional).
    pub hold_duration: Option<Duration>,
}
```

### 15.2 Phase Progression for Agent Verification

| Phase | Blast Radius | Gates | Cost | Purpose |
|---|---|---|---|---|
| Smoke | 1% | Compile only | ~3s | Does it parse? |
| Lint | 5% | Compile + Clippy ∥ Diff | ~8s | Is it clean? |
| Test | 25% | Full test suite | ~60s | Does it work? |
| Property | 50% | Property tests (256 cases) | ~120s | Does it generalize? |
| Deep | 100% | PBT (10K) + Fuzz (30s) + Integration | ~180s | Is it robust? |

The cost column shows that short-circuiting at Smoke saves up to 180s per doomed
attempt. For a plan with 50 tasks averaging 3 attempts each, the savings compound
to hours of verification time.

### 15.3 Rollback and Bake-In

```
Phase 1 (Smoke):  compile → PASS → hold 0s → advance
Phase 2 (Lint):   clippy + diff → PASS → hold 0s → advance
Phase 3 (Test):   test suite → FAIL → ROLLBACK → record failure signal
                   (Phases 4-5 never run)
```

A failure at any phase triggers immediate rollback. The failure is recorded as a
verdict Signal with the phase label as metadata, enabling the adaptive threshold
system to track which phases are bottlenecks.

---

## 16. Pipeline Instrumentation

### 16.1 Per-Gate Metrics

Every gate execution produces an instrumentation event:

```rust
pub struct GateMetrics {
    pub gate_name: String,
    pub rung: u8,
    pub passed: bool,
    pub duration_ms: u64,
    pub score: f32,
    /// For probabilistic gates: confidence interval bounds.
    pub ci_lower: Option<f64>,
    pub ci_upper: Option<f64>,
    pub sample_count: Option<u64>,
    /// Memory high-water mark during gate execution (bytes).
    pub peak_memory_bytes: Option<u64>,
    /// Whether this gate was skipped (advisory skip from AdaptiveThresholds).
    pub skipped: bool,
}
```

### 16.2 Pipeline-Level Summary

```rust
pub struct PipelineMetrics {
    pub name: String,
    pub total_duration_ms: u64,
    pub gates_run: usize,
    pub gates_skipped: usize,
    pub gates_passed: usize,
    pub gates_failed: usize,
    pub short_circuited: bool,
    /// Which phase (in progressive mode) was reached.
    pub phase_reached: Option<String>,
    /// Per-gate breakdown.
    pub gate_metrics: Vec<GateMetrics>,
}
```

These metrics feed the adaptive threshold system, the efficiency event logger, and
the dashboard's verification health display.

---

## 17. Test Criteria

| Test | Property |
|---|---|
| `parallel_runs_concurrently` | Parallel(SlowGate, SlowGate) completes in ~1x, not 2x |
| `parallel_both_fail` | Parallel with two failures returns aggregated failure |
| `voting_quorum_pass` | 2/3 judges pass with quorum=2 → pass |
| `voting_quorum_fail` | 1/3 judges pass with quorum=2 → fail |
| `fallback_primary_passes` | Fallback does not run secondary if primary passes |
| `fallback_primary_fails` | Fallback runs secondary and returns its verdict |
| `sequential_stopping_early_pass` | Sequential property gate stops before max iterations |
| `sequential_stopping_early_fail` | Counterexample found → stops and returns Fail |
| `wilson_interval_small_sample` | CI width is large with n=5, small with n=10000 |
| `probabilistic_to_standard_uses_lower_bound` | Conversion uses CI lower bound as score |
| `progressive_short_circuits_on_phase_failure` | Fail at phase 2 → phases 3-5 never run |
| `progressive_advances_all_phases` | All pass → reaches final phase |
| `nested_pipeline` | Pipeline containing a Parallel containing gates works correctly |
