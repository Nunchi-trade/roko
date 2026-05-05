# Test Strategy and Verification

> Expresses the comprehensive test strategy as a Pipeline of Verify Cells at multiple scales. The test pyramid maps to tier costs. CI/CD is a Pipeline Graph fired by Trigger Cells.

**Depth for**: [28-ROADMAP.md](../../unified/28-ROADMAP.md)
**Sources**: `docs/00-architecture/32-comprehensive-test-strategy.md`
**Prerequisites**: [00-INDEX.md](../../unified/00-INDEX.md) (vocabulary), [02-CELL.md](../../unified/02-CELL.md) (Verify protocol), [04-EXECUTION.md](../../unified/04-EXECUTION.md) (execution engine)

---

## The Testing Paradox of Self-Improving Systems

Classical testing assumes a fixed program. Roko violates this assumption in three ways:

1. **Prompt evolution**: SystemPromptBuilder templates, EvoSkills, and playbook injection change the effective program without changing source code.
2. **Threshold drift**: Adaptive gate thresholds (EMA alpha=0.1), CascadeRouter bandit arms, and efficiency metrics evolve continuously.
3. **Knowledge accumulation**: NeuroStore tiers, episode logs, and pattern extraction create emergent behaviors not present at deployment.

Testing must therefore operate at three levels: **static** (source code correctness), **behavioral** (system behavior under fixed inputs), and **evolutionary** (capability preservation across self-modification cycles).

The unified spec provides the conceptual apparatus to express all three levels as the same primitive: a **Verify Cell** operating at different scales.

---

## Tests as Verify Cells

Every test in Roko's codebase is an instance of the Verify protocol:

```rust
/// The Verify protocol (from 02-CELL.md)
pub trait Verify {
    /// Pre-condition check (can veto execution)
    fn verify_pre(&self, input: &Signal) -> Result<(), Verdict>;
    /// Post-condition check (produces Verdict)
    fn verify_post(&self, input: &Signal, output: &Signal) -> Verdict;
}
```

A test is a Verify Cell that:
- Takes an **input** Signal (test fixture or stimulus)
- Produces an output Signal (system behavior)
- Checks the output against criteria and returns a **Verdict** (pass/fail with evidence)

The difference between test types is **scale and cost**, not kind.

---

## The Test Pyramid as Tier Costs

| Test Type | Tier | Cost | Frequency | What It Verifies | Verify Cell Pattern |
|-----------|------|------|-----------|------------------|---------------------|
| **Unit tests** | T0 | Free (~0ms per test) | Every commit | Single-function correctness | Local Verify Cell (inline) |
| **Integration tests** | T0 | Compile-time (~1s per test) | Every commit | Cross-crate composition | Graph-level Verify (multi-Cell) |
| **Property tests** | T1 | Cheap (~10ms per case x 1000 cases) | Every commit | Algebraic invariants | Randomized Score Cell |
| **Eval tests** | T2 | Expensive (~$0.01-1.00 per eval) | Pre-release | LLM output quality | LLM-backed Verify Cell |
| **Red-team tests** | Delta | Offline (hours) | Weekly | Adversarial robustness | Dream-cycle adversarial probing |

### As a TOML Pipeline Graph

```toml
[graph]
id = "test-pyramid"
pattern = "Pipeline"

[[graph.cells]]
id = "unit"
protocol = "Verify"
tier = "T0"
cost = 0.0
description = "3,761 inline tests across 22 crates"

[[graph.cells]]
id = "integration"
protocol = "Verify"
tier = "T0"
cost = 0.0
description = "Cross-crate composition tests in tests/ directories"

[[graph.cells]]
id = "property"
protocol = "Score"
tier = "T1"
cost = 0.001
description = "Randomized property assertions (proptest)"

[[graph.cells]]
id = "eval"
protocol = "Verify"
tier = "T2"
cost = 0.5
description = "LLM-backed evaluation of agent outputs"

[[graph.cells]]
id = "redteam"
protocol = "Verify"
tier = "Delta"
cost = 10.0
description = "Adversarial probing via Dream cycles"

[[graph.edges]]
from = "unit"
to = "integration"
condition = "all_pass"
description = "Only run integration if unit passes"

[[graph.edges]]
from = "integration"
to = "property"
condition = "all_pass"

[[graph.edges]]
from = "property"
to = "eval"
condition = "all_pass"

[[graph.edges]]
from = "eval"
to = "redteam"
condition = "all_pass"
```

---

## Unit Tests: Local Verify Cells

Unit tests verify single-function correctness. They are the cheapest Verify Cells (T0: no external calls, no I/O, sub-millisecond).

### Current State

| Crate | Tests | Target | Key Modules |
|-------|-------|--------|-------------|
| `roko-core` | 376 | 500 | signal, score, decay, kind, verdict, query, loop_tick |
| `roko-agent` | 346 | 450 | dispatcher (5 backends), cascade_router, tool_loop, safety, mcp |
| `roko-gate` | 200 | 300 | compile, clippy, test, symbol, diff, pipeline, ratchet, adaptive |
| `roko-orchestrator` | 158 | 250 | plan_dag, executor, state, merge_queue |
| `roko-conductor` | 185 | 200 | 10 watchers, circuit breaker, event bus |
| `roko-learn` | 101 | 200 | episode, playbook, skill, bandit, experiment |
| `roko-std` | 96 | 150 | 19 built-in tools, mock_dispatcher |
| `roko-compose` | 23 | 100 | system_prompt_builder, templates, enrichment |
| Others | 276 | 510 | fs, cli, chain, index, primitives, neuro, daimon, dreams |
| **Total** | **3,761** | **~5,000** | |

### Unit Test as Verify Cell (Pseudocode)

```rust
/// Every unit test is structurally a Verify Cell
#[test]
fn score_effective_in_range() {
    // Input Signal: construct a Score
    let score = Score::new(0.8, -0.3, 0.5, 0.9);

    // Output Signal: compute effective_score
    let effective = score.effective();

    // Verdict: check against criteria
    assert!(effective >= -1.0 && effective <= 1.0,
        "effective_score {} out of range for {:?}", effective, score);
    // Implicit: Verdict::Pass if no panic
}
```

---

## Integration Tests: Graph-Level Verify

Integration tests verify that Cells compose correctly across crate boundaries. They are still T0 (compile-time, no LLM calls) but exercise the Graph structure.

### Cross-Crate Integration Matrix

| Scenario | Crates | Priority | Status |
|----------|--------|----------|--------|
| Full self-hosting loop | cli -> orchestrator -> agent -> gate -> fs -> learn | P0 | Partial |
| PRD -> Plan -> Execute | cli -> agent -> orchestrator -> gate | P0 | Not tested |
| Gate pipeline -> Adaptive thresholds | gate -> learn (efficiency) | P1 | Not tested |
| Agent -> Safety -> Tool dispatch | agent (dispatcher + safety) -> std (tools) | P1 | Partial |
| CascadeRouter -> Model -> Cost | agent (router) -> learn (efficiency + bandit) | P1 | Not tested |
| SystemPromptBuilder -> Agent | compose -> agent | P1 | Not tested |
| Episode -> Skill extraction | learn -> compose (injection) | P2 | Not tested |
| Signal -> Query -> Decay -> GC | core -> fs -> core (decay) | P2 | Not tested |
| Replay with override | cli -> orchestrator -> fs -> learn -> Bus | P0 | Not tested |
| Telemetry contract | orchestrator -> agent -> gate -> Bus -> StateHub | P0 | Not tested |

### Integration Test as Graph-Level Verify

```rust
/// Integration test: a Graph of Cells verified as a unit
#[tokio::test]
async fn test_full_self_hosting_loop() {
    // This test exercises the Pipeline pattern:
    // compose_cell -> agent_cell -> gate_cell -> store_cell -> learn_cell

    let dir = tempdir().unwrap();

    // Input Signal: a plan with one task
    let plan = Plan::single_task("Add hello() function to lib.rs");

    // Execute the Pipeline Graph
    let result = execute_plan(&plan, &dir).await;

    // Verify Cell checks:
    // 1. Gate verdict is Pass (correctness)
    assert!(result.gate_verdict.passed);
    // 2. State persisted (durability)
    assert!(dir.path().join(".roko/state/executor.json").exists());
    // 3. Episode logged (observability)
    assert!(dir.path().join(".roko/episodes.jsonl").exists());
    // 4. Efficiency event recorded (learning)
    assert!(dir.path().join(".roko/learn/efficiency.jsonl").exists());
}
```

---

## Property Tests: Randomized Score Cells

Property-based tests are Score Cells that evaluate algebraic invariants across randomized inputs. They use the `proptest` crate to generate thousands of input variations and check that properties hold universally.

### Three Categories

#### Category A: Algebraic Properties (Pure Functions)

Properties that must hold for ALL inputs. No state, no side effects.

```rust
use proptest::prelude::*;

proptest! {
    /// Score effective_score is always bounded
    #[test]
    fn effective_in_range(
        confidence in -1.0f32..=1.0,
        novelty in -1.0f32..=1.0,
        utility in -1.0f32..=1.0,
        reputation in -1.0f32..=1.0,
    ) {
        let score = Score::new(confidence, novelty, utility, reputation);
        let e = score.effective();
        prop_assert!(e >= -1.0 && e <= 1.0);
    }

    /// Decay is monotonically non-increasing
    #[test]
    fn decay_monotone(
        half_life_ms in 1u64..1_000_000,
        t1_ms in 0u64..2_000_000,
        t2_ms in 0u64..2_000_000,
    ) {
        let decay = Decay::HalfLife { half_life_ms };
        let (early, late) = if t1_ms <= t2_ms { (t1_ms, t2_ms) } else { (t2_ms, t1_ms) };
        prop_assert!(decay.weight_at(early) >= decay.weight_at(late));
    }

    /// HDC bind is self-inverse: bind(bind(a, b), b) ~ a
    #[test]
    fn hdc_bind_self_inverse(a in arb_hdc_vector(), b in arb_hdc_vector()) {
        let bound = hdc_bind(&a, &b);
        let recovered = hdc_bind(&bound, &b);
        prop_assert!(cosine_similarity(&a, &recovered) > 0.9);
    }

    /// Signal serialization round-trip
    #[test]
    fn signal_roundtrip(signal in arb_signal()) {
        let bytes = serde_json::to_vec(&signal).unwrap();
        let recovered: Signal = serde_json::from_slice(&bytes).unwrap();
        prop_assert_eq!(signal.content_hash(), recovered.content_hash());
    }
}
```

#### Category B: Stateful Properties (Sequential Invariants)

Properties that hold across sequences of operations. Uses `proptest-state-machine`.

```rust
/// GateRatchet: highest_pass never decreases for a given plan
struct RatchetMachine {
    ratchet: GateRatchet,
    max_per_plan: HashMap<String, u8>,
}

impl StateMachine for RatchetMachine {
    type Transition = RatchetOp;

    fn apply(&mut self, op: RatchetOp) {
        match op {
            RatchetOp::RecordPass { plan, rung } => {
                self.ratchet.record_pass(&plan, rung);
                let max = self.max_per_plan.entry(plan.clone()).or_insert(0);
                *max = (*max).max(rung);
            }
        }
    }

    fn check_invariant(&self) -> bool {
        // Invariant: highest_pass matches our tracking
        self.max_per_plan.iter().all(|(plan, max)| {
            self.ratchet.highest_pass(plan) == Some(*max)
        })
    }
}
```

#### Category C: Metamorphic Relations (No Oracle)

When exact outputs cannot be predicted, check that input transformations produce predictable output transformations.

```rust
/// Metamorphic: adding context section increases token count
#[test]
fn compose_monotone_under_enrichment() {
    let base_prompt = SystemPromptBuilder::new()
        .role("coder")
        .task("implement foo")
        .build();

    let enriched_prompt = SystemPromptBuilder::new()
        .role("coder")
        .task("implement foo")
        .knowledge_section("additional context here")
        .build();

    // Metamorphic relation: enrichment increases size
    assert!(enriched_prompt.token_count() >= base_prompt.token_count());
}
```

---

## Eval Tests: LLM-Backed Verify Cells

Eval tests use LLM judgment to verify outputs that cannot be checked by deterministic criteria. They are T2 (expensive: ~$0.01-1.00 per eval) and run pre-release.

### Architecture

```toml
[graph]
id = "eval-verify"
pattern = "Pipeline"

[[graph.cells]]
id = "generate"
protocol = "Compose"
description = "Generate agent output for eval task"
tier = "T2"

[[graph.cells]]
id = "judge"
protocol = "Verify"
description = "LLM judge evaluates output quality"
tier = "T2"
config.model = "claude-sonnet"
config.rubric = "eval_rubric.toml"

[[graph.cells]]
id = "aggregate"
protocol = "Score"
description = "Aggregate verdicts across eval set"
```

### Eval Categories

| Category | What It Measures | Scale | Frequency |
|----------|-----------------|-------|-----------|
| **Code quality** | Does generated code pass human review? | 100 task set | Pre-release |
| **Prompt quality** | Do composed prompts produce better outputs than baseline? | A/B comparison | Weekly |
| **Knowledge retrieval** | Are the right knowledge entries retrieved? | 50 query set | Pre-release |
| **Safety** | Does the agent refuse dangerous instructions? | 200 adversarial prompts | Pre-release |
| **Efficiency** | Token budget utilization vs. outcome quality | Per-run metrics | Continuous |

### Eval Test as Verify Cell

```rust
/// LLM-backed eval: code quality assessment
async fn eval_code_quality(task: &EvalTask) -> Verdict {
    // 1. Generate output using the agent pipeline
    let output = agent_pipeline.execute(&task.prompt).await;

    // 2. Send to judge model with structured rubric
    let judgment = judge_model.evaluate(EvalPrompt {
        rubric: &QUALITY_RUBRIC,
        task_description: &task.description,
        generated_code: &output.code,
        test_results: &output.test_output,
    }).await;

    // 3. Parse structured verdict
    Verdict {
        passed: judgment.overall_score >= 7.0, // out of 10
        reward: judgment.overall_score / 10.0,
        evidence: Evidence::LlmJudgment {
            model: "claude-sonnet",
            score: judgment.overall_score,
            reasoning: judgment.reasoning,
        },
        details: judgment.per_criterion_scores,
    }
}
```

### The Variance Inequality

From `docs/21-references/17-process-reward-models.md`: self-improvement works only when verification ability exceeds generation ability (Song et al. 2025). For eval tests, this means:

- The **judge model** must be more capable than the **generator** at detecting quality
- Or the judge must use **structural verification** (compiler, tests, linters) that are definitionally stronger
- LLM-as-judge should be used for dimensions where structural verification is impossible (style, clarity, approach)

```
Verify_capability > Generate_capability  (mandatory)
```

---

## Red-Team Tests: Adversarial Dream Cycles

Red-team tests are the most expensive Verify Cells (Delta tier: offline, hours). They use the Dreams subsystem's REM creativity mode to generate adversarial scenarios.

### Architecture

```toml
[graph]
id = "redteam-dreams"
pattern = "Loop"

[[graph.cells]]
id = "generate_attack"
protocol = "Compose"
description = "REM creativity: generate adversarial prompt/scenario"
tier = "Delta"

[[graph.cells]]
id = "execute_attack"
protocol = "Connect"
description = "Run agent with adversarial input"

[[graph.cells]]
id = "verify_defense"
protocol = "Verify"
description = "Check that safety layer held"

[[graph.cells]]
id = "update_defenses"
protocol = "React"
description = "If attack succeeded, strengthen immune system"

[[graph.edges]]
from = "generate_attack"
to = "execute_attack"

[[graph.edges]]
from = "execute_attack"
to = "verify_defense"

[[graph.edges]]
from = "verify_defense"
to = "update_defenses"

[[graph.edges]]
from = "update_defenses"
to = "generate_attack"
feedback = true
```

### Threat Model

| Threat | Test Strategy | Expected Defense |
|--------|--------------|-----------------|
| Prompt injection via tool output | Generate adversarial file contents | Safety pre-check filters injection patterns |
| Gate bypass via trivial tests | Generate easy-to-pass test suites, verify they don't pass real criteria | DiffGate min_added_lines, Generated test hashing |
| Knowledge poisoning | Inject corrupted Signals, verify they don't promote to high tiers | mark_verified gate, confidence decay |
| Threshold manipulation | Strategic gate failures to lower adaptive thresholds | Floor threshold, anomaly detection |
| Budget exhaustion | Trigger expensive retry loops | max_retries, cost ceiling |
| State corruption | Malformed JSON in state files | Schema validation on load, graceful fallback |

### Red-Team as Self-Immunization

The 5-layer immune system Pipeline (from [05-causal-discovery-and-adversarial-robustness.md](05-causal-discovery-and-adversarial-robustness.md)):

```
Layer 1: HDC prototype matching (~10ns)
Layer 2: Statistical filters (trimmed mean, MAD)
Layer 3: Structural analysis (AST/CFG)
Layer 4: LLM-based assessment (T2)
Layer 5: Red-team dream probing (Delta)
```

Each red-team cycle that finds a successful attack automatically creates:
1. A new **AntiKnowledge** Signal (what NOT to do)
2. A new HDC prototype for Layer 1 matching
3. An updated statistical profile for Layer 2

This is self-immunization: the system gets stronger from being attacked.

---

## CI/CD as a Pipeline Graph Fired by Trigger Cells

### Architecture

```toml
[graph]
id = "ci-pipeline"
pattern = "Pipeline"

# Trigger: git push fires the pipeline
[[graph.cells]]
id = "trigger"
protocol = "Trigger"
config.source = "git.push"
config.debounce_ms = 0

# Stage 1: Format check (T0, < 5s)
[[graph.cells]]
id = "fmt"
protocol = "Verify"
tier = "T0"
config.command = "cargo +nightly fmt --all --check"

# Stage 2: Lint (T0, < 60s)
[[graph.cells]]
id = "clippy"
protocol = "Verify"
tier = "T0"
config.command = "cargo clippy --workspace --no-deps -- -D warnings"

# Stage 3: Unit + Integration tests (T0, < 5min)
[[graph.cells]]
id = "test"
protocol = "Verify"
tier = "T0"
config.command = "cargo test --workspace"

# Stage 4: Property tests (T1, < 2min)
[[graph.cells]]
id = "proptest"
protocol = "Verify"
tier = "T1"
config.command = "cargo test --workspace -- --include-ignored proptest"
config.cases = 1000

# Stage 5: Binary size check (T0, < 30s)
[[graph.cells]]
id = "binary_size"
protocol = "Verify"
tier = "T0"
config.max_bytes = 50_000_000  # 50MB ceiling

# Edges: sequential pipeline with early exit
[[graph.edges]]
from = "trigger"
to = "fmt"

[[graph.edges]]
from = "fmt"
to = "clippy"
condition = "pass"

[[graph.edges]]
from = "clippy"
to = "test"
condition = "pass"

[[graph.edges]]
from = "test"
to = "proptest"
condition = "pass"

[[graph.edges]]
from = "proptest"
to = "binary_size"
condition = "pass"
```

### Execution Tiers in CI

| Tier | When | Duration Target | What |
|------|------|-----------------|------|
| **Smoke** | Every commit (pre-push hook) | < 30s | fmt + clippy on changed files |
| **Standard** | Every push to branch | < 10min | Full fmt + clippy + test + proptest |
| **Nightly** | Scheduled (1am) | < 30min | Standard + benchmarks + eval subset |
| **Release** | Pre-merge to main | < 1hr | Standard + full eval + binary size + doc build |
| **Red-team** | Weekly | < 4hr | Full adversarial Dream cycle |

---

## Quality Metrics

The following metrics are tracked continuously and reported by `roko status` and the TUI dashboard:

### Code Quality

| Metric | Current | Target | How Measured |
|--------|---------|--------|-------------|
| Total test count | 3,761 | ~5,000 | `cargo test --workspace 2>&1 \| grep "test result"` |
| Test pass rate | 100% (CI gate) | 100% | CI must pass |
| Clippy clean | Yes (CI gate) | Yes | `cargo clippy -- -D warnings` must exit 0 |
| Format clean | Yes (CI gate) | Yes | `cargo +nightly fmt --check` must exit 0 |
| Binary size (roko-cli) | ~35MB | < 50MB | `ls -la target/release/roko` |
| Property test coverage | 0 tests | 100+ tests | proptest invocations |
| Integration test coverage | 19 workspace tests | 65+ workspace tests | `tests/tests/*.rs` |
| Benchmark regression | No baselines | < 5% regression | iai-callgrind instruction counts |

### Agent Quality (Eval Metrics)

| Metric | Measurement | Target |
|--------|-------------|--------|
| SWE-bench-like pass rate | Percentage of coding tasks that pass Gate on first attempt | > 60% |
| Gate pass rate per rung | Adaptive threshold EMA | Tracks per-rung; floor at 50% |
| Retry escalation rate | Percentage of tasks requiring model escalation | < 20% |
| Token efficiency | Tokens used / task complexity score | Decreasing over time |
| Knowledge retrieval precision | Relevant entries / retrieved entries | > 70% |

### Observability Contracts

| Surface | Contract | Verification |
|---------|----------|-------------|
| Structured logs | JSON with ts/level/target/fields | Schema validation test |
| Metrics | Prometheus exposition stable names/types | Name/type/label assertions |
| Traces | Seven-step span tree with attributes | Span tree structure test |
| Pulses + projections | Bus traffic typed and filterable | Topic/filter matching test |
| Replay | Deterministic episode replay | Baseline comparison test |

---

## The Self-Improving Test Strategy

Because Roko modifies itself, the test strategy must also address **evolutionary** verification: does the system maintain capabilities across self-modification cycles?

### Capability Preservation Tests

```rust
/// Run after every learning cycle (Delta frequency)
async fn test_capability_preservation() {
    // 1. Load baseline capability scores from .roko/baselines/capabilities.json
    let baseline = load_capability_baseline();

    // 2. Run standard eval suite with current (potentially modified) system
    let current = run_capability_eval().await;

    // 3. Check for regression beyond tolerance
    for (capability, baseline_score) in &baseline {
        let current_score = current.get(capability).unwrap_or(&0.0);
        let regression = baseline_score - current_score;
        assert!(regression <= 0.05,
            "Capability '{}' regressed by {:.1}% (baseline: {:.1}%, current: {:.1}%)",
            capability, regression * 100.0, baseline_score * 100.0, current_score * 100.0
        );
    }

    // 4. If no regression, update baseline (ratchet)
    if all_passed {
        save_capability_baseline(&current);
    }
}
```

### Threshold Drift Monitoring

```rust
/// Monitor adaptive thresholds for anomalous drift
fn check_threshold_health(thresholds: &AdaptiveThresholds) -> Vec<Alert> {
    let mut alerts = vec![];

    for (rung, ema) in thresholds.per_rung_rates() {
        // Alert if threshold has drifted more than 20% from initial
        if (ema - 0.7).abs() > 0.2 {
            alerts.push(Alert::ThresholdDrift {
                rung,
                current: ema,
                initial: 0.7,
                drift: (ema - 0.7).abs(),
            });
        }

        // Alert if threshold is below safety floor
        if ema < 0.3 {
            alerts.push(Alert::ThresholdBelowFloor {
                rung,
                current: ema,
                floor: 0.3,
            });
        }
    }

    alerts
}
```

---

## What This Enables

1. **Unified verification model**: All tests, from unit to red-team, are instances of the same Verify protocol.
2. **Cost-aware testing**: Each test has an explicit tier and cost, enabling budget-constrained test selection.
3. **Self-immunization**: Adversarial testing feeds back into the immune system, making the system stronger.
4. **Evolutionary stability**: Capability preservation tests prevent silent regression during self-improvement.
5. **Auditable quality**: All metrics are tracked, versioned, and queryable via `roko learn all`.

## Feedback Loops

- **Gate verdicts** feed back to adaptive thresholds (should we retry? should we skip this rung?)
- **Eval results** feed back to CascadeRouter (which model produces higher eval scores?)
- **Red-team findings** feed back to immune system (new attack patterns become detection prototypes)
- **Capability preservation** feeds back to learning configuration (if regression detected, pause self-modification)
- **CI failures** produce Signals (Gate failure episodes) that enter the learning loop

## Open Questions

1. What is the minimum eval set size for statistical significance in capability preservation?
2. How often should red-team Dream cycles run? (Currently specified: weekly. Is this sufficient?)
3. Should property tests be weighted by code path criticality (more cases for hot paths)?
4. Can metamorphic testing be automated by the agent itself (generate metamorphic relations from code)?
5. What is the cost-optimal distribution across test tiers for a given quality target?

## Implementation Tasks

| Task | Path | Priority |
|------|------|----------|
| Add proptest to roko-core (Category A: Score, Decay, Hash) | `crates/roko-core/tests/properties.rs` | P0 |
| Add proptest to bardo-primitives (HDC ops) | `crates/bardo-primitives/tests/properties.rs` | P0 |
| Add proptest-state-machine for GateRatchet | `crates/roko-gate/tests/stateful.rs` | P1 |
| Create workspace integration test: full self-hosting loop | `tests/tests/self_hosting_loop.rs` | P0 |
| Create workspace integration test: telemetry contract | `tests/tests/telemetry_contract.rs` | P0 |
| Add criterion benchmarks for hot paths | `crates/roko-core/benches/`, `crates/roko-gate/benches/` | P1 |
| Add iai-callgrind for CI-stable regression detection | `crates/roko-core/benches/iai_*.rs` | P1 |
| Create capability baseline file | `.roko/baselines/capabilities.json` | P1 |
| Add threshold drift monitoring to `roko doctor` | `crates/roko-cli/src/doctor.rs` | P1 |
| Create red-team Dream cycle configuration | `.roko/config/redteam.toml` | P2 |
| Add eval harness with LLM-as-judge | `crates/roko-cli/src/eval.rs` | P2 |
| Wire CI pipeline as TOML Graph (self-describing CI) | `.roko/graphs/ci-pipeline.toml` | P2 |
