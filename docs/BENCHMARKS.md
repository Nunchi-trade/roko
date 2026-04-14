# Benchmarks and evaluation methodology

> Audience: ML researchers, evaluation engineers, contributors building on roko
>
> Scope: How roko measures agent performance, what it measures today, and where
> it maps to external benchmarks. Covers internal metrics, evaluation
> architecture, cost modeling, and continuous monitoring.

---

## 1. Evaluation philosophy

Most agent benchmarks test models in isolation. GPT-4 vs. Claude Sonnet on
SWE-bench. That framing is wrong for systems like roko, and the field has
caught up to this.

The HAL Leaderboard (Kapoor et al., Princeton, ICLR 2026) evaluated 21,730
agent rollouts across model-scaffold-benchmark triples. The finding: the
scaffold contributes as much to performance as the model itself. A weaker model
inside a strong harness routinely beats a stronger model inside a naive loop.
The Meta-Harness paper (Lee et al., 2026) confirmed this with +7.7 accuracy
points on text classification and 4x token reduction from harness improvements
alone.

Roko's evaluation therefore never compares bare models. It compares
**model x scaffold** combinations:

```
(sonnet + roko orchestration)  vs.  (opus + standard bash loop)
(haiku + full 6-rung pipeline) vs.  (sonnet + compile-only gate)
```

This is the only comparison that predicts real-world performance.

### Why LLM-as-judge fails for coding

Three converging results establish that LLM self-evaluation breaks down for
code:

- **Huang et al. (ICLR 2024)**: Self-correction without external feedback
  produces *worse* answers than the initial attempt.
- **Pan et al. (ICML 2024)**: Self-refinement loops optimize for surface
  features (comment quality, variable naming) rather than functional
  correctness.
- **Song et al. (ICLR 2025)**: Self-improvement stalls when verification
  ability is bounded by generation ability -- the same ceiling caps both.

The evaluator must be **structurally different** from the generator. Compilers,
test suites, linters, and blockchain state have no capability ceiling in their
domain. An LLM judge plateaus exactly where the LLM coder plateaus. Roko's
gate pipeline uses deterministic tools for this reason.

---

## 2. The harness pattern

Every coding agent benchmark follows the same architecture. The agent is a pure
function from problem to solution: `fn solve(issue: &str) -> Patch`. The
**harness** surrounds the agent, handles sandboxing, testing, and scoring.

```
Environment setup (docker, deps, correct commit)
         |
         v
    Agent under test
    (reads issue + codebase, produces patch)
         |
         v
Patch application (git apply)
         |
         v
Test execution (cargo test / pytest)
         |
         v
Scoring (pass/fail per test, resolve rate)
```

### The Karpathy autoresearch principle

The separation is strict. In Karpathy's autoresearch framing:

- **`prepare.py`** (read-only, evaluation harness): Downloads data, defines
  scoring. Never modified.
- **`train.py`** (mutable, agent under test): The optimization target. Iterates
  freely but can only observe the score that `prepare.py` emits.

**The generator must never see the evaluator logic.** The moment it can inspect
the scoring function, it exploits the metric surface rather than solving the
task. Pan et al. (2024) call this **spontaneous reward hacking** -- the agent
writes `assert!(true)` to pass tests, adds `#[allow(clippy::all)]` to suppress
warnings, or writes coverage-maximal tests that assert nothing.

### How roko enforces separation

Roko implements the harness pattern through three isolated processes with no
shared mutable state:

| Process | Sees | Cannot see |
|---|---|---|
| **Test generation** (from PRD acceptance criteria) | PRD, existing codebase (read-only) | Implementation agent's prompt or task brief |
| **Implementation** (agent under test) | Task brief, codebase context, episode history | Test source, gate config, acceptance criteria |
| **Gate evaluation** (deterministic pipeline) | Patch from implementation, tests from test generation | N/A -- no agent, no optimization pressure |

The gate pipeline contains no LLM. It is a deterministic sequence of structural
checks: compile, lint, test, symbol resolution, property verification. You
cannot reward-hack a compiler.

---

## 3. What roko measures today

Roko records three data streams during execution. Everything described in this
section is implemented and shipping.

### 3.1 Episodes

Every agent turn produces one `Episode` record, appended to
`.roko/episodes.jsonl`:

```rust
Episode {
    id: String,              // UUID v4
    agent_id: String,
    task_id: String,
    plan_id: String,
    role: String,            // "Implementer", "Reviewer", etc.
    model: String,           // "claude-sonnet-4-20250514"
    success: bool,           // gate pass/fail
    iteration: u32,          // retry index within the task
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: f64,
    duration_ms: u64,
    gate_verdicts: Vec<GateVerdict>,
    timestamp: DateTime<Utc>,
}
```

Each `GateVerdict` records the gate name, pass/fail, and a content-hashed error
signature for deduplication. Episodes are the raw substrate for all downstream
learning.

**Source:** `crates/roko-learn/src/episode_logger.rs`

### 3.2 Efficiency events

Per-turn cost and performance data flows to `.roko/learn/efficiency.jsonl`:

```rust
TaskMetric {
    task_id: String,
    plan_id: String,
    role: String,
    complexity_band: String,  // "fast", "standard", "complex"
    model: String,
    gate: String,
    gate_passed: bool,
    iteration: u32,
    cost_usd: f64,
    duration_ms: u64,
    input_tokens: u64,
    output_tokens: u64,
    config_hash: ConfigHash,  // for A/B comparison
    timestamp: DateTime<Utc>,
}
```

Task metrics feed the regression detector, the baseline computation, and the
dashboard.

**Source:** `crates/roko-learn/src/task_metric.rs`, `crates/roko-learn/src/efficiency.rs`

### 3.3 Gate verdicts

The gate pipeline (`crates/roko-gate/src/gate_pipeline.rs`) produces a
structured `Verdict` per task execution:

```rust
Verdict {
    passed: bool,
    gate: String,       // "compile:cargo", "test:cargo", etc.
    reason: String,     // machine-parseable failure summary
    detail: String,     // full gate output with per-gate headers
    duration_ms: u64,
    test_count: TestCount { passed, failed, ignored },
    score: f32,         // for probabilistic gates: CI lower bound
}
```

Verdicts flow to:
- `GateRatchet` -- tracks regression (highest rung reached per plan)
- `AdaptiveThresholds` -- EMA per rung, adjusts retry budget
- `CascadeRouter` -- updates bandit arms based on model + outcome
- `EpisodeLogger` -- records the full trace
- `GateFeedback` -- parses output for agent context on retry

**Source:** `crates/roko-gate/src/gate_pipeline.rs`, `crates/roko-cli/src/orchestrate.rs`

### 3.4 Cascade router observations

The cascade router persists its state to `.roko/learn/cascade-router.json`.
It tracks per-model trial counts, successes, and bandit weights across three
stages:

| Stage | Observations | Algorithm |
|---|---|---|
| Static | < 50 | Hardcoded role-to-model table |
| Confidence | 50--200 | Empirical pass rates + confidence intervals |
| UCB | > 200 | LinUCB contextual bandit (18 dimensions) |

The router produces observations in the form `(model, task_context, outcome)`
triples, which are the signal for cost and quality optimization.

**Source:** `crates/roko-learn/src/cascade_router.rs`

---

## 4. Internal metrics

These are the metrics roko computes from the data streams above.

### 4.1 Gate pass rate

The most direct measure of agent capability:

```
pass_rate = tasks_passing_all_gates / total_tasks
```

Computed overall and per `(role, complexity_band)` slice. A 90% pass rate on
trivial tasks with 20% on complex tasks is a different profile from 60%
uniform. Roko always reports per-slice.

### 4.2 First-try success rate

Pass rate discounts brute-force strategies:

```
first_try_rate = tasks_passing_on_iteration_0 / total_tasks
```

Two agents can both achieve 70% pass rate through different behavioral
strategies. One gets it right on the first attempt 70% of the time. The other
brute-forces through 3--4 retries. First-try rate distinguishes them.

### 4.3 Quality-adjusted pass rate

Penalizes retry-heavy success:

```
qa_pass_rate = sum(1 / iterations_to_pass) / total_tasks
```

A first-attempt pass contributes 1.0. A second-attempt pass contributes 0.5.
A fourth-attempt pass contributes 0.25. A failure contributes 0.0. This
correlates better with real-world cost and latency than raw pass rate.

### 4.4 Cost efficiency

```
cost_efficiency = 1 - (actual_cost / ceiling_cost)
```

Clamped to [0, 1]. The ceiling is the investigation threshold for the
complexity band (see section 5). Cost efficiency measures how far below the
alarm threshold the system operates.

### 4.5 Waste ratio

The highest-leverage optimization metric:

```
waste_ratio = tokens_spent_on_failed_iterations / total_tokens
```

If 60% of tokens go to failed gate attempts, the highest-leverage investment is
better prompts (fewer failures). If waste is under 20%, invest in
parallelization (faster wave scheduling). Between 20% and 60%, balance both.

### 4.6 C-Factor (collective capability factor)

A composite score that captures system-wide health:

```
c_factor = weighted_mean(
    gate_pass_rate,
    cost_efficiency,
    speed_efficiency,
    first_try_rate,
    knowledge_growth,
    turn_taking_equality,
)
```

C-Factor regression -- when the composite drops against a trailing window --
detects subtle multi-dimensional degradation where no single metric breaches
its threshold but the system is collectively worse.

**Source:** `crates/roko-learn/src/cfactor.rs` (computed in
`LearningRuntime::record_completed_run()`)

---

## 5. Cost modeling

### Target costs by complexity band

| Complexity | Target cost/task | Investigation threshold |
|---|---|---|
| Trivial | ~$0.30 | > $0.60 |
| Simple | ~$0.80 | > $1.60 |
| Standard | ~$2.00 | > $4.00 |
| Complex | ~$5.00 | > $10.00 |

The investigation threshold is 2x the target. Breaching it triggers a
regression alert.

### The cost equation

```
total = sum(input_tokens * price + output_tokens * price) * iterations * tasks * plans - cache_savings
```

Cache savings are significant. Roko's prompt caching (system prompt reuse
across tasks within a plan) typically achieves 80%+ cache hit rates. The
cascade router's cache affinity bonus (0.15 score boost for reusing the same
model as the previous task) actively optimizes for this.

### Cost-per-success

The metric that matters for budgeting:

```
cost_per_success = total_cost / tasks_successfully_completed
```

A model that costs $0.05/task with 78% pass rate has a cost-per-success of
$0.064. A model that costs $1.38/task with 71% pass rate has a
cost-per-success of $1.94. The cheaper model is 30x more cost-effective for
simple tasks. The cascade router's Pareto frontier pruning removes models that
are dominated on both cost and quality.

---

## 6. Evaluation methodology

### 6.1 The 80/20 split

Roko's evaluation tasks are split into two pools:

- **Train set (80%)**: Used for prompt modification, context graph optimization,
  and threshold calibration. Developers iterate freely against this set.
- **Held-out set (20%)**: Hidden from developer analysis. Run periodically to
  detect overfitting.

If performance on the held-out set diverges from the train set -- high on
train, low on held-out -- the system is overfitting to the evaluation suite
rather than building genuine capability. Based on the Leaderboard Illusion
finding (Singh et al., NeurIPS 2025): models performing well on isolated sets
without generalizing require rollback.

### 6.2 Per-stratum reporting

Results are always stratified across:

| Dimension | Strata |
|---|---|
| Codebase size | Small (< 10K LOC), Medium (10--50K), Large (50K+) |
| Task complexity | Trivial, Simple, Standard, Complex |
| Language | Rust, TypeScript, Go, Mixed |
| Dependency depth | Shallow (< 5), Deep (5--20), Very deep (20+) |
| Gate failure mode | Compile, Test, Lint, Symbol, Generated, Property, Integration |

Aggregate metrics hide failures. A system that reports "pass rate improved from
65% to 75%" may have achieved this by routing more tasks to high-affordance
code. Per-stratum reporting prevents that.

### 6.3 Cross-validation across codebases

Single-codebase evaluation overfits to the codebase's structure. The validation
suite includes tasks drawn from:

- Roko itself (177K LOC, Rust, deep dependency graph)
- External open-source projects (varying languages, sizes, structure quality)
- Synthetic repositories designed to stress specific capabilities

The synthetic repos include intentionally adversarial cases: poorly documented
code, tangled dependencies, missing tests, ambiguous specifications.

### 6.4 The 2x2x2 factorial design

Three binary factors, eight configurations, ten seeds each:

| Factor | Off | On |
|---|---|---|
| Context engine | Raw file dump | Full 6-layer SystemPromptBuilder |
| Gate pipeline | Compile-only (rung 0) | Full pipeline with adaptive thresholds |
| Parallel execution | Sequential, single agent | Full DAG scheduler |

ANOVA decomposition isolates each component's contribution and interaction
effects. If `Context + Gates` outperforms `Context alone + Gates alone`, the
components synergize. If it underperforms the sum, they interfere.

### 6.5 Overfitting detection

**PBO (probability of backtest overfitting)**: Partition the task set into N
complementary pairs. For each partition, pick the best configuration on the
in-sample half, measure on the out-of-sample half. PBO = the fraction of
partitions where in-sample-optimal underperforms median. Threshold: PBO < 0.50.

**Deflated Sharpe Ratio**: Adjusts observed performance for the number of
configurations tested:

```
DSR = (SR_observed - E[max(SR) under null]) / std[max(SR)]
```

Corrects for selection bias, non-normal distributions, skewness. Threshold:
DSR > 2.0 (approximately p < 0.05 after multiple-testing adjustment).

---

## 7. The gate pipeline as evaluation infrastructure

Roko's gate pipeline is not just a CI system. It is the primary evaluation
instrument. Understanding its structure matters for interpreting all metrics.

### 7.1 Pipeline structure

The `GatePipeline` composes `Vec<Box<dyn Gate>>` into a single verification
step. It implements the `Gate` trait itself, enabling nesting.

```rust
GatePipeline::new(vec![
    Box::new(CompileGate::cargo()),
    Box::new(ClippyGate::cargo()),
    Box::new(TestGate::cargo()),
])
.with_short_circuit(true)
```

Short-circuit mode stops at the first failure. In a pipeline where integration
tests take 30 minutes, a compile failure caught in 3 seconds saves that time.

### 7.2 Progressive verification

Verification depth increases in phases:

| Phase | Gates | Cost | Purpose |
|---|---|---|---|
| Smoke | Compile only | ~3s | Does it parse? |
| Lint | Compile + clippy + diff | ~8s | Is it clean? |
| Test | Full test suite | ~60s | Does it work? |
| Property | Property tests (256 cases) | ~120s | Does it generalize? |
| Deep | PBT (10K) + fuzz (30s) + integration | ~180s | Is it robust? |

Short-circuiting at Smoke saves up to 180s per doomed attempt. For a 50-task
plan averaging 3 attempts per task, the savings compound to hours.

### 7.3 Verdict aggregation

The pipeline passes if and only if every gate passes. Individual gate outputs
are concatenated with per-gate headers:

```
--- [compile:cargo] ---
Compiling foo v0.1.0
Finished dev in 2.3s

--- [clippy:cargo] ---
warning: unused variable

--- [test:cargo] ---
test result: ok. 12 passed; 0 failed; 0 ignored
```

Duration is the sum of individual gate durations. Test counts are merged across
all gates.

**Source:** `crates/roko-gate/src/gate_pipeline.rs` (593 lines, wired into
`orchestrate.rs`)

---

## 8. Adaptive thresholds and learning

### 8.1 Per-rung EMA

Every gate execution updates a per-rung exponential moving average:

```rust
ema_pass_rate = 0.1 * new_observation + 0.9 * ema_pass_rate
```

Alpha = 0.1 gives an effective memory of ~10 observations. This tracks shifts
in pass rates as the project evolves -- a new project has low rates, rates
climb as issues are fixed, a major refactor temporarily drops them.

### 8.2 Retry budget

The EMA drives retry allocation:

| Pass rate | Suggested retries |
|---|---|
| ~100% | 1 (almost always passes) |
| ~50% | 3 (coin flip) |
| ~0% | 5 (almost never passes) |

For unknown rungs or those with fewer than 5 observations, the default is 3.

### 8.3 Skip advisory

If a rung has passed 20+ consecutive times, the system advises skipping it. The
advisory is not enforced -- the orchestrator can honor it 90% of the time and
periodically verify. A failure resets the consecutive pass counter.

### 8.4 Statistical process control

Beyond EMA, roko implements three SPC methods for formal anomaly detection:

- **CUSUM** (cumulative sum): Detects small, sustained shifts that EMA smooths
  over. A gate drifting from 90% to 80% over 20 runs accumulates signal.
- **EWMA control chart**: Adds formal upper/lower control limits to the EMA.
  When the EMA crosses a limit, the gate is flagged as out-of-control.
- **BOCPD** (Bayesian online change point detection): Provides a probabilistic
  answer to "did the gate's fundamental behavior change?" Triggers baseline
  recalibration after regime shifts.

These three methods work in a hierarchy:

```
Observation (pass/fail)
    |
    +-- EMA update: smoothed pass rate
    +-- CUSUM update: sustained shift detection
    +-- EWMA control chart: formal anomaly detection
    +-- BOCPD update: regime change detection
             |
             +-- change point? -> recalibrate all detectors
```

**Persistence:** `.roko/learn/gate-thresholds.json` (atomic tempfile + rename)

**Source:** `crates/roko-gate/src/adaptive_threshold.rs` (215 lines)

---

## 9. Continuous monitoring

### 9.1 Regression detection

The regression detector compares recent metrics against a historical baseline.
It operates per `(role, complexity_band)` slice:

| Metric | Threshold | Severity |
|---|---|---|
| Pass rate drop | > 15% | Alert |
| Cost increase | > 20% | Alert |
| Duration increase | > 30% | Warning |
| Iterations increase | > 25% | Warning |

The asymmetry between Alert and Warning reflects priority: pass rate and cost
regressions demand investigation. Duration and iteration increases may reflect
a harder task mix.

The detector runs inside `LearningRuntime::record_completed_run()`. It reads
all `TaskMetric` records, splits into baseline (all except the latest 20) and
current (latest 20), and compares.

### 9.2 C-Factor regression

In addition to per-metric detection, the C-Factor composite score has its own
regression check. This catches multi-dimensional degradation where pass rate
drops slightly, cost rises slightly, and speed decreases slightly -- none
individually alarming, but collectively significant.

### 9.3 Drift alerts

Drift manifests in two forms:

**Data drift**: The task distribution shifts. More complex tasks, different
codebases, new languages. The regression detector accounts for this via
per-slice analysis, but aggregate metrics may still mislead.

**Model drift**: Provider updates change model behavior. The cascade router
detects this through its bandit feedback: a model that was Pareto-optimal last
week may become dominated after a provider update. When the router's
calibration error (ECE) exceeds 0.10, it triggers automatic Platt scaling
recalibration.

### 9.4 The immortal baseline

One configuration is permanently frozen: initial model, initial prompts, rungs
0 + 2 only, raw context. It runs alongside every evaluation batch. All
improvement claims are measured against this baseline. If the current system
does not beat the immortal baseline by DSR > 2.0, accumulated complexity has
not produced genuine improvement.

The immortal baseline also detects environment drift -- new Rust versions, API
changes, hardware differences -- because its configuration never changes.

### 9.5 Process reward models

Process rewards score intermediate reasoning steps, not just the final output.
Roko tracks two orthogonal signals per agent execution:

**Promise** estimates how likely the current execution is to succeed:

```
promise = 0.4 * rung_fraction
        + 0.3 * test_pass_rate
        + 0.2 * error_trend
        + 0.1 * tool_efficiency
```

Low promise (< 0.2) triggers early termination. Better to start fresh than
continue a failing path.

**Progress** measures whether the agent advances across attempts:

```
progress = delta_rung + delta_test_rate + delta_error_count
```

Negative progress across three turns triggers re-planning. The current plan may
be fundamentally flawed.

These signals create a multi-timescale control system:
- Per-turn: promise/progress drives continue/terminate
- Per-attempt: gate verdict drives retry with adjusted prompt
- Across attempts: repeated failure drives rung escalation and re-planning

**Source:** `crates/roko-learn/src/prm.rs` (design stage, data sourced from
existing gate infrastructure)

---

## 10. External benchmark positioning

Roko's internal evaluation is primary. External benchmarks provide calibration
against the field.

### 10.1 SWE-bench

SWE-bench (Jimenez et al., 2024) measures resolution of real GitHub issues.
Harness quality accounts for most performance variance between agent systems --
the same model can score 25% or 85% depending on the harness. Roko's
architecture is designed with this finding in mind: the six crate layers
(core, agent, orchestrator, gate, compose, learn) provide harness
infrastructure while the model is a pluggable component.

SWE-bench relevance to roko: high. Both systems resolve real-world code issues
from natural language specifications. Roko's gate pipeline maps directly to
SWE-bench's test-based evaluation.

### 10.2 MBPP and HumanEval

MBPP (Mostly Basic Programming Problems) and HumanEval measure code generation
from function docstrings. These benchmarks are simpler than roko's operating
environment (full codebase context, multi-file changes, dependency graphs) but
useful for isolating raw generation quality independent of harness
effectiveness.

Relevance to roko: moderate. Useful for baseline model comparison but not for
evaluating orchestration, context management, or iterative refinement -- the
capabilities that differentiate roko from a bare model.

### 10.3 GT-Score composite

For cross-system comparison, the GT-Score (Sheppert, 2026) provides a
standardized composite:

```
GT = 0.40 * resolve_rate
   + 0.25 * cost_efficiency
   + 0.20 * time_efficiency
   + 0.15 * quality_score
```

Where:
- `resolve_rate` = fraction passing all rungs
- `cost_efficiency` = 1 - (actual / ceiling), clamped [0, 1]
- `time_efficiency` = 1 - (actual_wall / SLA), clamped [0, 1]
- `quality_score` = composite of correctness, security, readability,
  idiomaticness, performance, maintainability

Reported per-task with 95% CI via bootstrap resampling.

---

## 11. EVM-specific verification

Blockchain infrastructure provides natively deterministic evaluation. Unlike
generic integration testing that requires mock layers, EVM verification
evaluates precisely against simulated on-chain state with zero-impact
boundaries.

### 11.1 Verification pipeline

| Stage | Weight | Metric |
|---|---|---|
| `forge build` | Pass/fail | Compiler verification |
| `forge test` | 0.30 | Fuzz-constrained test pass rate |
| Anvil simulation | 0.20 | Mainnet-fork execution trace correctness |
| Slither | 0.15 | Zero CVE detection |
| Gas benchmark | 0.15 | Computational bounding within limits |

### 11.2 Deterministic test patterns

EVM tests are inherently reproducible. Given the same block state, the same
transaction produces the same result. Roko leverages this by constructing
`forge build -> anvil simulate -> slither analyze -> gas bench` test matrices
dynamically based on PRD detection.

The three-level evaluation framework applies directly:

**Level 0 (binary)**: Compiles? Tests pass? Deploys to anvil? Transaction
executes?

**Level 1 (property-based)**: For a slippage hook -- small swaps pass, large
swaps capped, violations revert, bidirectional, zero-safe, no overflow, gas
efficient, access controlled, no reentrancy.

**Level 2 (reference comparison)**: Deploy both reference and candidate
implementations. Run identical randomized scenarios (seed = 42). Compare
outputs within 0.1% tolerance.

```
composite = 0.3 * L0 + 0.3 * L1 + 0.4 * L2
```

Level 2 gets highest weight because behavioral equivalence to a reference
implementation is the strongest correctness signal available without formal
verification.

---

## 12. Affordance-weighted evaluation

Not all code is equally easy to modify. A well-tested, well-documented, loosely
coupled module with stable interfaces is fundamentally more amenable to agent
modification than a tangled, untested monolith. Evaluating agents without
accounting for the target code's modifiability conflates agent skill with
codebase quality.

### 12.1 AffordanceScore

Drawing from Gibson's (1979) ecological psychology -- an affordance is a
property of the environment that enables action:

| Component | Range | What it measures |
|---|---|---|
| Extensibility | [0, 1] | Trait-based design, clear extension points |
| Test coverage | [0, 1] | Line and branch coverage from existing suites |
| Documentation | [0, 1] | Doc comments, module-level docs, examples |
| Coupling | [0, 1] | Inverse of fan-in + fan-out |
| Recent stability | [0, 1] | Inverse of churn rate over 30 days |
| Size | [0, 1] | Inverse of LOC |

Weighted composite:

```
affordance = 0.25 * extensibility
           + 0.20 * test_coverage
           + 0.15 * documentation
           + 0.20 * coupling
           + 0.10 * recent_stability
           + 0.10 * size
```

### 12.2 Stratification by affordance

| Band | Score range | Expected pass rate | Investigate if below |
|---|---|---|---|
| High | > 0.7 | > 85% | 80% |
| Medium | 0.3--0.7 | > 55% | 45% |
| Low | < 0.3 | > 25% | 15% |

A system that reports "pass rate improved 65% to 75%" but achieved this by
routing more tasks to high-affordance code has not actually improved. Affordance
stratification exposes this.

### 12.3 Niche construction

Track the mean AffordanceScore across the codebase over time:

```
niche_trend = mean_affordance(t) - mean_affordance(t - window)
```

**Positive niche construction** (trend > 0): The agents are leaving the
codebase better than they found it. Tests added, documentation improved,
coupling decreased. Future tasks become easier -- the self-hosting virtuous
cycle.

**Negative niche construction** (trend < 0): The agents are degrading the
codebase. Technical debt accumulates, coverage drops, files grow. Future tasks
become harder.

Report niche construction trend per plan. A plan whose net affordance delta is
negative delivered its feature at the cost of future modifiability.

### 12.4 Cost prediction from affordance

Token cost correlates inversely with affordance. Agents spend more tokens
exploring low-affordance code (reading more files, trying more approaches,
backtracking):

```
predicted_cost(task) = base_cost(complexity) * (1 / affordance(target_files))
```

This enables better model routing. Low-affordance tasks justify the cost of
Opus (better at handling ambiguity). High-affordance tasks can be routed to
Haiku (efficient at following clear patterns).

---

## 13. Chaos engineering

Distributed systems engineering learned decades ago that you cannot test
resilience by hoping nothing breaks. Agent systems need the same discipline.

### 13.1 Fault injection menu

| Injection | Effect | Frequency |
|---|---|---|
| Process kill | SIGKILL to a random agent mid-turn | 1 per plan |
| Compile warning injection | `#[deprecated]` attribute in a random source file | 2 per plan |
| Gate delay | 5-second artificial delay on one gate rung | 3 per plan |
| Worktree corruption | Garbage bytes in a non-critical file | 1 per plan |
| Network partition | Simulated API timeout for one LLM call | 1 per plan |
| Resource exhaustion | Temporary file descriptor limit | 1 per plan |

Each injection is logged with timestamp, target agent, injection type, and the
system's response. The implementing agent is not told that chaos is active.

### 13.2 Chaos metrics

**Recovery time**: How long until productive work resumes after injection.

```
recovery_time = timestamp_next_successful_gate - timestamp_injection
```

Recovery time should decrease over runs. Flat or increasing recovery indicates
the learning mechanisms are not capturing failure patterns.

**Learning response**: Did the system capture the failure pattern?

```
learning_response = 1 if pattern_captured_in_playbook else 0
```

A system that fails the same way twice has not learned.

**Cascade impact**: Did the injection propagate beyond its target?

```
cascade_impact = agents_affected / total_agents
```

Consistently above 0.3 indicates hidden sequential dependencies in the DAG.

### 13.3 Antifragility index

```
antifragility = performance_under_chaos / performance_without_chaos
```

- < 1.0: Fragile. Chaos degrades performance.
- = 1.0: Robust. Chaos has no effect.
- \> 1.0: Antifragile. Chaos triggers learning that raises the baseline.

---

## 14. Evolutionary metrics

These track whether the system improves over longer time horizons.

### 14.1 Baldwin effect score

```
baldwin = fraction of learned optimizations committed as permanent code
```

Target: > 0.3 at 90 days. Thirty percent of runtime learnings (playbook rules,
routing preferences, prompt patterns) should become permanent templates.

### 14.2 Ratchet score

```
ratchet = 1 - (regression_events / total_gate_passes)
```

Target: > 0.95. Fewer than 5% of passes should be followed by regression.

### 14.3 Information gain per mechanism

| Mechanism | Target (bits/kiloTick) | Below target means |
|---|---|---|
| Adaptive thresholds | > 0.05 | Thresholds are static -- remove overhead |
| CascadeRouter | > 0.10 | Routing not learning -- check reward signal |
| Prompt experiments | > 0.08 | Experiments not differentiating -- increase variants |
| Playbook rules | > 0.03 | Not capturing patterns -- check extraction |

If a mechanism is not learning (information gain below target), it adds
overhead without value. Either fix the input signal or disable it.

---

## 15. Summary of data paths

Everything in this document traces back to data roko already collects:

| Data stream | Location | Consumers |
|---|---|---|
| Episodes | `.roko/episodes.jsonl` | Pattern discovery, skill extraction, clustering |
| Task metrics | `.roko/learn/task-metrics.jsonl` | Baselines, regression detection, dashboard |
| Efficiency events | `.roko/learn/efficiency.jsonl` | Cost analysis, waste ratio, prompt attribution |
| Gate thresholds | `.roko/learn/gate-thresholds.json` | Retry budget, skip advisory, SPC |
| Cascade router state | `.roko/learn/cascade-router.json` | Model routing, Pareto pruning, calibration |
| Experiment state | `.roko/learn/experiments.json` | A/B prompt experiments |

The evaluation methodology described here is not a separate system. It is a
lens on the data these subsystems already produce.

---

## Citations

1. Kapoor et al. (2026). "HAL: A Benchmark for Evaluating LLM Agents." ICLR.
   -- Scaffold-aware evaluation, 21,730 agent rollouts.
2. Lee et al. (2026). "Meta-Harness: Harness Engineering for LLM Agents."
   arXiv:2603.28052. -- +7.7 accuracy, 4x token reduction.
3. Jimenez et al. (2024). "SWE-bench: Can Language Models Resolve Real-World
   GitHub Issues?" -- External benchmark context.
4. Huang et al. (2024). "Large Language Models Cannot Self-Correct Reasoning
   Yet." ICLR. -- Self-correction fails without external feedback.
5. Pan et al. (2024). "Spontaneous Reward Hacking in Self-Refinement." ICML.
   -- LLM self-evaluation attacks surface features.
6. Song et al. (2025). GVU Framework. ICLR. -- Verification quality bounds
   self-improvement.
7. Lightman et al. (2023). "Let's Verify Step by Step." -- Process reward
   models for step-level evaluation.
8. Singh et al. (2025). "The Leaderboard Illusion." NeurIPS. -- Overfitting
   detection in agent evaluation.
9. Sheppert (2026). GT-Score composite. -- Standardized agent quality metric.
10. Bailey et al. (2015). PBO: Probability of Backtest Overfitting. --
    Combinatorial cross-validation for configuration selection.
11. Gibson (1979). The Ecological Approach to Visual Perception. --
    Affordance theory.
12. Karpathy. Autoresearch pattern. -- Separation of generator from evaluator.
