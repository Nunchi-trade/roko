# Verify Cells and Pipeline

> Depth for [02-CELL.md](../../unified/02-CELL.md). The 11 gate implementations as Verify Cells, the 7-rung pipeline as a Pipeline Graph, and rung selection as a Route Cell that picks minimum viable verification.

This doc covers the concrete verification machinery. For the four simultaneous roles of the Verify protocol (reward function, relabeling oracle, safety boundary, economic attestation), Goodhart-resistance, Variance Inequality, and meta-verification, see [verify-as-universal-oracle.md](verify-as-universal-oracle.md).

---

## 1. Verify Cells: The 11 Implementations

Every gate in roko-gate is a Cell that conforms to the Verify protocol. In the codebase this is the `Gate` trait:

```rust
pub trait Gate: Send + Sync {
    /// Always returns Verdict, never Result<Verdict>.
    /// Gate failure is a verdict, not an error.
    async fn verify(&self, engram: &Engram, ctx: &Context) -> Verdict;
    fn name(&self) -> &str;
}
```

The return type is `Verdict`, not `Result<Verdict>`. This is the most important design decision in the Verify protocol implementation. A compile error, a spawn failure, a timeout -- all are verdicts. Downstream consumers (ratchet, adaptive thresholds, feedback systems, routing) never handle two failure paths. Every Verify Cell satisfies four invariants:

1. **Total function**: Always returns a Verdict. No panics, no hangs. Every Cell enforces a timeout and converts expiry to `Verdict::fail()`.
2. **Deterministic on identical inputs**: Same code + filesystem state produces the same verdict. No randomness (except LlmJudgeGate, which has its own reproducibility constraints).
3. **Side-effect free on source**: Cells read the filesystem and run subprocesses but never modify the source being verified. Build artifacts go to `CARGO_TARGET_DIR` or the ArtifactStore.
4. **Duration tracked**: Every verdict carries `duration_ms` for adaptive threshold and efficiency calculations.

### 1.1 The Verify Cell Catalog

Cells are ordered from cheapest to most expensive. Each maps to a rung in the Pipeline (see S2).

| Cell | Rung | Input | What it verifies | Cost | False positive rate |
|---|---|---|---|---|---|
| **CompileGate** | 0 (Compile) | GatePayload with BuildSystem | Code compiles (`cargo check`, `npm build`, `go build`) | Low (seconds) | 0% |
| **ClippyGate** | 1 (Lint) | GatePayload with BuildSystem | No lint violations (`cargo clippy -D warnings`, `go vet`, `npm lint`) | Low (seconds-minute) | 0% |
| **TestGate** | 2 (Test) | GatePayload with TestSelector | Tests pass. Parses passed/failed/ignored counts per build system | Medium (seconds-15min) | 0% for deterministic tests |
| **SymbolGate** | 3 (Symbol) | Source roots + SymbolManifest | Required symbols exist with correct kind, visibility, module path | Near-zero (file I/O only, no subprocess) | 0% |
| **GeneratedTestGate** | 4 (GeneratedTest) | Agent-generated test files | Auto-generated tests exercise the new code | High | Moderate (generated tests may be incorrect) |
| **PropertyTestGate** | 5 (PropertyTest) | Property specs | Property-based tests (QuickCheck/proptest) over randomized inputs | High | Low |
| **IntegrationGate** | 6 (Integration) | Full environment | Integration tests with external services, DBs, network | Highest (minutes-hours) | Moderate (infra flakiness) |
| **DiffGate** | Pre-pipeline | Git diff text | Non-vacuous changes (rejects `todo!()`, `Ok(())`, empty diffs) | Zero (pure string scan) | 0% |
| **ShellGate** | Any | Program + args | Arbitrary shell command exits 0 | Varies | 0% (deterministic) |
| **LlmJudgeGate** | Auxiliary | Code + spec | LLM-based quality judgment for nuanced properties | Medium | Non-zero (model-dependent) |
| **VerifyChainGate** | Auxiliary | Chain artifacts | Chain verification (Phase 2+) | Low | 0% |

### 1.2 ShellGate as Foundation Cell

ShellGate is the simplest Verify Cell and the building block for CompileGate, ClippyGate, and TestGate. Its `verify()` implements the canonical subprocess pattern:

```rust
// Pseudocode for the pattern all subprocess-spawning Verify Cells follow
async fn verify(&self, engram: &Engram, ctx: &Context) -> Verdict {
    let start = Instant::now();

    // 1. Extract payload from Signal body for working_dir, extra_env
    let payload = parse_gate_payload(engram);

    // 2. Build command: program, args, cwd, env, kill_on_drop(true)
    let mut cmd = Command::new(&self.program);
    cmd.args(&self.args)
       .current_dir(&payload.working_dir)
       .kill_on_drop(true);

    // 3. Run with timeout
    let result = tokio::time::timeout(
        Duration::from_millis(self.timeout_ms),
        cmd.output(),
    ).await;

    let elapsed = start.elapsed().as_millis() as u64;

    // 4. Three outcomes, all produce Verdict (never Result)
    match result {
        Err(_) => Verdict::fail(&self.name, format!("timed out after {}ms", self.timeout_ms))
            .with_duration(elapsed),
        Ok(Err(io_err)) => Verdict::fail(&self.name, format!("spawn failed: {io_err}"))
            .with_duration(elapsed),
        Ok(Ok(output)) => {
            let combined = format_stdout_stderr(&output);
            if output.status.success() {
                Verdict::pass(&self.name).with_detail(combined).with_duration(elapsed)
            } else {
                Verdict::fail(&self.name, summarize_errors(&combined))
                    .with_detail(combined)
                    .with_duration(elapsed)
            }
        }
    }
}
```

Timeouts by Cell: CompileGate 10min, TestGate 15min, ClippyGate 5min, ShellGate 5min.

### 1.3 SymbolGate: Zero-Cost Structural Verification

SymbolGate is unique: no subprocesses, no LLM calls. It parses Rust source files with a lightweight single-pass line-based extractor and verifies that every symbol in a SymbolManifest exists with the correct kind, visibility, and module path. It catches the most common agent failure mode ("I was told to create `pub struct RateLimiter` and did not") at effectively zero cost.

Five mismatch categories: `MISSING`, `WRONG_VIS`, `WRONG_KIND`, `WRONG_PATH`, `AMBIGUOUS`. Each gives the agent actionable feedback.

### 1.4 DiffGate: Vacuous-Implementation Rejection

DiffGate solves a specific failure mode: agents that "pass" gates by producing vacuous implementations. Without it, an agent can replace function bodies with `todo!()` to make compile and lint happy while doing no work.

A diff is rejected when: (a) zero added lines, (b) non-whitespace added lines below threshold, or (c) every substantive line matches a forbidden token (`todo!()`, `unimplemented!()`, `Ok(())`, etc.). The analysis is pure -- no I/O, no subprocess.

### 1.5 LlmJudgeGate: The Only Non-Deterministic Verify Cell

LlmJudgeGate consults a model rather than a deterministic tool. It is used when properties are too nuanced for automated checking (e.g., "does this implementation match the PRD's intent?"). It is the only Verify Cell that violates the determinism invariant, and the Variance Inequality (see [verify-as-universal-oracle.md](verify-as-universal-oracle.md)) applies especially here: the judge model must be spectrally cleaner than the generator.

---

## 2. The 7-Rung Pipeline as a Pipeline Graph

The 7-rung pipeline is a Pipeline Graph (see [03-GRAPH.md](../../unified/03-GRAPH.md)) -- a linear chain of Verify Cells where each can reject (short-circuit) or pass through to the next. Rungs are numbered 0-6, cheapest to most expensive.

```
Rung 0: CompileGate ──pass──> Rung 1: ClippyGate ──pass──> Rung 2: TestGate ──pass──>
Rung 3: SymbolGate ──pass──> Rung 4: GeneratedTestGate ──pass──> Rung 5: PropertyTestGate
──pass──> Rung 6: IntegrationGate ──pass──> VERIFIED
    \                \                \
     fail             fail             fail
      ↓                ↓                ↓
   SHORT-CIRCUIT    SHORT-CIRCUIT    SHORT-CIRCUIT
```

### 2.1 GatePipeline: The Sequential Combinator

```rust
pub struct GatePipeline {
    gates: Vec<Box<dyn Gate>>,
    short_circuit: bool,  // default: true -- stop on first failure
    name: String,
}
```

GatePipeline implements `Gate` itself (fractal composition -- a Pipeline of Pipelines is just a Pipeline). In short-circuit mode, a compile failure in 3 seconds saves a 15-minute test run. In full-execution mode, all gates run regardless of individual outcomes, producing a comprehensive report.

**Verdict aggregation**:
- Pass condition: ALL gates pass (conjunctive -- matches the hard-criteria design from [verify-as-universal-oracle.md](verify-as-universal-oracle.md))
- Detail: concatenated per-gate output with `--- [gate_name] ---` headers
- Duration: sum of all gate durations
- Test counts: merged by summing passed/failed/ignored across all test gates
- Reason on failure: lists which gate(s) failed

### 2.2 Verdict Lattice

Verdicts form a bounded lattice:

```
Skip < Warn < Pass < Fail

identity:  Skip  (no effect)
absorber:  Fail  (dominates any merge)
merge(v1, v2) = max(v1, v2)
```

This gives a monoid `(Verdict, merge, Skip)` that every combinator preserves.

### 2.3 Gate Composition Algebra

Beyond sequential composition, the algebra defines:

| Combinator | Topology | Use case |
|---|---|---|
| **Sequential** | g1 then g2 (short-circuit on fail) | Standard rung pipeline |
| **Parallel** | Run all concurrently, merge verdicts | Independent gates within a rung (Symbol + Diff) |
| **Fallback** | Try g1; if fail, try g2 instead | Degraded environments (no clippy -> grep-lint) |
| **Voting** | Run N gates, pass if >= K pass (quorum) | LLM judge panels |
| **Threshold** | Pass only if score exceeds minimum | Converting continuous scores to binary verdicts |

Parallel duration is `max(durations)` not `sum(durations)` -- gates run concurrently. Example compositions:

```rust
// Progressive pipeline with parallel groups
Sequential(vec![
    CompileGate,
    Parallel(vec![ClippyGate, DiffGate]),  // independent, run concurrently
    TestGate,
    Parallel(vec![SymbolGate, GeneratedTestGate]),
    PropertyTestGate,
    IntegrationGate,
])

// LLM judge panel: 3 judges, pass if 2+ agree
Voting { gates: vec![Judge1, Judge2, Judge3], quorum: 2 }
```

### 2.4 Probabilistic Verify Cells

Standard Verify Cells return binary pass/fail. Probabilistic Cells return a confidence interval `[lower, upper]`, essential for property-based tests and fuzz tests where results are inherently statistical.

The Wilson score interval is used (not the naive Wald interval) because it is well-calibrated even at small sample sizes:

```rust
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

For pipeline compatibility, probabilistic verdicts convert to standard Verdicts using the **lower bound of the CI as the score** -- the most conservative estimate.

Sequential stopping rules let property tests exit early when the outcome is statistically clear, rather than running a fixed iteration count.

### 2.5 Progressive Delivery Pipeline

Adapted from canary deployment (Argo Rollouts), a progressive pipeline increases verification depth in phases with automatic rollback:

| Phase | Blast Radius | Gates | Cost | Purpose |
|---|---|---|---|---|
| Smoke | 1% | Compile only | ~3s | Does it parse? |
| Lint | 5% | Compile + Clippy + Diff | ~8s | Is it clean? |
| Test | 25% | Full test suite | ~60s | Does it work? |
| Property | 50% | Property tests (256 cases) | ~120s | Does it generalize? |
| Deep | 100% | PBT (10K) + Fuzz (30s) + Integration | ~180s | Is it robust? |

Short-circuiting at Smoke saves up to 180s per doomed attempt. For a plan with 50 tasks averaging 3 attempts each, the savings compound to hours.

---

## 3. Rung Selection as a Route Cell

Not every task needs every gate. A one-line rename does not need property-based testing. Rung selection is functionally a Route Cell: given task complexity and confidence, it selects the minimum viable verification set.

### 3.1 Inputs

Three inputs determine which rungs to run:

| Input | Type | What it carries |
|---|---|---|
| **Plan complexity** | `PlanComplexity` enum | Trivial / Simple / Standard / Complex |
| **Rung capabilities** | `RungCaps` struct | Which gates are available in this environment |
| **Prior failures** | `u32` | How many prior attempts failed (drives escalation) |

### 3.2 Complexity-to-Rung Mapping

```
Trivial  -> max rung 0 (Compile only)
Simple   -> max rung 1 (Compile + Lint)
Standard -> max rung 3 (Compile + Lint + Test + Symbol)
Complex  -> max rung 6 (all available)
```

The selector collects all rungs <= maximum that are available in `RungCaps`, sorted ascending (cheapest first).

### 3.3 Escalation on Failure

When a plan fails a gate, complexity escalates:

```rust
impl PlanComplexity {
    pub fn escalate(&self) -> Self {
        match self {
            Self::Trivial  => Self::Simple,
            Self::Simple   => Self::Standard,
            Self::Standard => Self::Complex,
            Self::Complex  => Self::Complex,  // already maximal
        }
    }
}
```

Escalation is applied `prior_failures` times (capped at Complex). This captures the heuristic: if easy checks catch a problem, the change is more complex than initially classified and deeper verification is warranted.

### 3.4 The Selection Algorithm

```rust
pub fn select_rungs(
    complexity: PlanComplexity,
    caps: &RungCaps,
    prior_failures: u32,
) -> Vec<Rung> {
    // 1. Escalate complexity by prior_failures levels
    let effective = (0..prior_failures).fold(complexity, |c, _| c.escalate());

    // 2. Map to maximum rung
    let max_rung = match effective {
        PlanComplexity::Trivial  => 0,
        PlanComplexity::Simple   => 1,
        PlanComplexity::Standard => 3,
        PlanComplexity::Complex  => 6,
    };

    // 3. Collect available rungs <= max, sorted ascending
    ALL_RUNGS.iter()
        .filter(|r| r.as_u8() <= max_rung && caps.has_rung(**r))
        .copied()
        .collect()
}
```

**Example**: complexity=Simple, caps={compile:true, lint:true, test:true, symbol:false}, prior_failures=1. Escalate Simple by 1 -> Standard. Max rung 3. Available rungs <= 3 = [Compile(0), Lint(1), Test(2)] (Symbol excluded because unavailable). Result: `[Compile, Lint, Test]`.

### 3.5 Separation of Concerns

The rung selector produces `Vec<Rung>`. The orchestrator maps each to a concrete `Box<dyn Gate>` and feeds them into a GatePipeline. The selector knows nothing about gate implementations; the pipeline knows nothing about complexity. Each evolves independently.

```
select_rungs(complexity, caps, failures)
    -> Vec<Rung>
    -> map each Rung to Box<dyn Gate>
    -> GatePipeline::new(gates).with_short_circuit(true)
    -> pipeline.verify(signal, ctx)
    -> aggregated Verdict
```

---

## 4. The ArtifactStore: Content-Addressed Gate Evidence

The ArtifactStore is an append-only, content-addressed store for gate output. Every artifact is identified by its BLAKE3 hash. The store deduplicates automatically.

```rust
pub struct ArtifactStore {
    items: HashMap<ContentHash, Vec<u8>>,  // BLAKE3 hash -> bytes
}
```

Three operations: `store(bytes) -> hash`, `get(hash) -> Option<bytes>`, `contains(hash) -> bool`. No delete, no update, no clear. Immutability is a design constraint:

- **No accidental loss**: An artifact backing a verdict cannot disappear
- **Audit trail**: verdict -> artifact hash -> artifact content is always intact
- **Forensic replay**: Given a hash, retrieve the exact artifact for causal replay

BLAKE3 over SHA-256 because: 5-15x faster on modern hardware, streaming support without full-input buffering, keyed mode for per-session namespacing.

Content addressing appears consistently across the system: ArtifactStore (gate outputs), Signal (content bodies), FileSubstrate (JSONL storage) -- all use BLAKE3. Any artifact or Signal can be cross-referenced by hash.

**Future**: Persistent filesystem layout with two-character prefix directories (`.roko/artifacts/ab/ab3f8c1d2e...`) + JSONL manifest for metadata + GC for artifacts older than configurable threshold with no active references.

---

## 5. The Verification-First Architecture

The rung system embodies a core insight from the GVU framework (Song et al., ICLR 2025):

**Cheap verification gates that run first prevent expensive retries. The returns to stronger verification compound, while the returns to stronger generation plateau.**

The cost savings compound:
- Compile failure caught in 3s saves a 15-minute test run
- Lint failure caught in 10s saves the same 15-minute test run
- A plan that passes on first attempt at Trivial complexity uses only the compile gate -- sub-second verification

In GVU terms, compilers and test suites are oracle verifiers (zero false positive rate for the properties they check). Roko invests in a rich Verify Cell ecosystem (11 cells, 7 rungs) rather than solely better prompts because **verification quality matters more than generation quality**.

### 5.1 Escalation Creates a Monotonically Advancing Frontier

Escalation (forward: adds rungs on failure) and ratcheting (backward: blocks regression on prior passes -- see [ratcheting-and-adaptive-thresholds.md](ratcheting-and-adaptive-thresholds.md)) work together:

```
Attempt 1: Trivial -> [Compile]
  Compile PASS (ratchet records rung 0)
  Lint not run (not in rung set)

Attempt 2 (after other task triggers escalation): Simple -> [Compile, Lint]
  Compile must still pass (ratchet enforces)
  Lint PASS (ratchet records rung 1)
  Test not run

Attempt 3: Standard -> [Compile, Lint, Test, Symbol]
  Compile must still pass (ratchet enforces)
  Lint must still pass (ratchet enforces)
  Test PASS (ratchet records rung 2)
  ...
```

Each attempt can only move the verification frontier forward. The system starts cheap and escalates only when warranted.

### 5.2 Rung Ordering vs. Rung Cost

Rung numbers encode a logical ordering, not a strict cost ordering:

| Rung | Typical cost | Notes |
|---|---|---|
| 0 Compile | 1-10s | Incremental builds faster |
| 1 Lint | 2-60s | Clippy can be slow on large codebases |
| 2 Test | 5s-15min | Depends on test count |
| 3 Symbol | 10-100ms | Pure file I/O, cheapest by wall-clock |
| 4 GeneratedTest | 30s-5min | Generation + execution |
| 5 PropertyTest | 10s-10min | Depends on iterations |
| 6 Integration | 1min-1hr | Depends on infrastructure |

Symbol (Rung 3) is actually cheaper than Compile (Rung 0) by wall-clock, but logically sits after Test because its value is in catching issues that compile + test already cover.

---

## 6. Mori-Diffs Reality

The mori-diffs analysis ([14-FAILURE-RETRY.md](../../mori-diffs/14-FAILURE-RETRY.md)) identifies a critical gap in the current pipeline: **the structured `GateFailureClassification` is lost between gate and orchestrator**.

Today's state:
- `roko-gate/src/compile_errors.rs` has a full classification stack: 11 `FailureClass` variants, `GateFailureAction` enum (Retry, NeedsReplan, Blocked, NeedsHuman), structured `GateFailureClassification`
- This classification reaches the orchestrator only as a rendered string in `verdict.error_digest`. The structured data (recommended action, failure class) is lost
- The orchestrator's retry decision is binary: pass or fail, with gate_failure_count vs retry_budget

The ideal design (from the mori-diffs) introduces `FailureKind` (Transient, Permanent, Resource, Structural) at the task level, mapped from the gate-level `FailureClass`, with per-kind retry policies (cooldowns, prompt augmentation, verify script regeneration). See the mori-diffs for the full specification.

---

## What This Enables

1. **Minimum-viable verification**: Rung selection ensures the system never runs more verification than needed. A trivial rename gets a sub-second compile check. A new subsystem gets the full battery. Compute is allocated where it matters.

2. **Fractal composition**: GatePipeline implements Gate. A pipeline can contain other pipelines. Any code that accepts `&dyn Gate` works with a pipeline, a single gate, a voting panel, or a parallel group. No special-casing.

3. **Verification cost scales with risk**: The progressive delivery pipeline spends 3 seconds on most tasks and reserves minutes-to-hours verification for genuinely complex changes. Cost savings compound across hundreds of tasks in a plan.

4. **Content-addressed forensics**: The ArtifactStore makes every verification outcome reproducible. Given a verdict hash, you can retrieve the exact gate output and replay the verification.

5. **Language-agnostic Verify Cells**: Adding a new language requires a `BuildSystem` variant and per-variant `check_args()`/`test_args()`/`lint_args()`. No changes to the Gate trait, the pipeline, the rung selector, or any downstream consumer.

---

## Feedback Loops

- **Verdict -> Route Cell**: Gate verdicts update `CandidateHistory.mean_reward` for the model/agent that produced the output. Routing learns which models handle which task types.
- **Verdict -> Compose protocol**: Failed verdicts produce corrective hints injected into retry prompts (relabeling oracle role from [verify-as-universal-oracle.md](verify-as-universal-oracle.md)).
- **Verdict -> Ratchet**: Passed rungs are recorded monotonically. Regression is blocked.
- **Verdict -> Adaptive thresholds**: EMA per rung adjusts retry budgets and skip advisories (see [ratcheting-and-adaptive-thresholds.md](ratcheting-and-adaptive-thresholds.md)).
- **Verdict -> Efficiency events**: Per-gate timing data feeds `.roko/learn/efficiency.jsonl`.
- **Escalation -> Rung selection**: Prior failures escalate complexity, which adds rungs to the pipeline on retry.

---

## Open Questions

1. **Gate parallelism within rungs**: The current pipeline is strictly sequential. Independent gates within a rung (Symbol + Diff) could run in parallel via the Parallel combinator. The infrastructure exists in spec form but is not wired in orchestrate.rs. When should this be activated?

2. **SymbolGate ordering**: Symbol (Rung 3) is cheaper than Compile (Rung 0) by wall-clock but ranked after Test. Should there be a "pre-pipeline" set of zero-cost Verify Cells (Symbol, Diff) that always run first regardless of rung selection?

3. **Structured classification passthrough**: The mori-diffs identify that `GateFailureClassification` is serialized to JSON in `error_digest` and must be re-parsed by the orchestrator. Adding a typed `classification: Option<GateFailureClassification>` field to Verdict would eliminate this lossy round-trip. The tradeoff is coupling `roko-core` to `roko-gate` types.

4. **Probabilistic gate integration**: The Wilson interval, sequential stopping, and FuzzGate are specified but not yet wired into the rung pipeline. How should probabilistic verdicts interact with the ratchet (which expects binary pass/fail)?
