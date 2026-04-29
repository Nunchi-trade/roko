# Self-Improvement Frameworks

> Depth for [07-LEARNING.md](../../unified/07-LEARNING.md). Self-improvement as a meta-Loop: how Roko improves its own improvement process. ADAS as a Loop that evolves Graph topologies. Bounded by the Variance Inequality. Safety invariants that prevent harmful self-modification.

**Depends on**: [02-CELL](../../unified/02-CELL.md) (Cell, Verify protocol, predict-publish-correct), [03-GRAPH](../../unified/03-GRAPH.md) (Graph, TOML definition), [04-EXECUTION](../../unified/04-EXECUTION.md) (Loop specialization), [07-LEARNING](../../unified/07-LEARNING.md) (L1-L4 taxonomy, Variance Inequality), [autocatalytic-compounding.md](autocatalytic-compounding.md) (compounding thesis, Kauffman condition)

**Source docs**: [12-self-improvement-frameworks.md](../../docs/05-learning/12-self-improvement-frameworks.md), [17-adas-and-autocatalytic.md](../../docs/05-learning/17-adas-and-autocatalytic.md), [18-self-learning-cybernetic-loops.md](../../docs/05-learning/18-self-learning-cybernetic-loops.md), [20-research-to-runtime.md](../../docs/05-learning/20-research-to-runtime.md)

---

## 1. The Self-Improvement Stack

Roko does not improve through a single mechanism. It improves through a stack of techniques operating at increasing levels of abstraction. Each level's output feeds the level above.

| Level | What improves | Technique | Unified expression |
|---|---|---|---|
| 0 | Task outcomes | Agent execution + gate verification | Pipeline of Cells |
| 1 | Parameters | EMA tuning of thresholds, weights, budgets | L1 Loop (per-tick) |
| 2 | Strategies | Playbook rules, skill library, heuristic calibration | L2 Loop (per-episode) |
| 3 | Representations | Prompt experiments, section effectiveness, HDC cleanup | L3 Loop (per-batch) |
| 4 | Architecture | ADAS-style topology search, structural evolution | L4 Loop (per-approval) |
| Meta | The improvement process itself | Meta-Loop: use the system to improve the system | Loop of Loops |

The crucial insight from the source docs: Roko uses its own learning Loops to optimize the components that implement those Loops. When Roko modifies `roko-learn` code, the cascade router learns which model works best for `roko-learn` tasks, and the skill library accumulates patterns specific to modifying the learning subsystem. This is the meta-harness concept -- the system developing itself is also the system being developed.

---

## 2. Academic Frameworks as Unified Patterns

Each self-improvement framework from the literature maps to a specific Loop pattern in the unified vocabulary.

### 2.1 Reflexion (Shinn et al. 2023)

**Insight**: Agents improve by reflecting on failures, then using those reflections as context in subsequent attempts.

**Unified expression**: A Loop where the React Cell watches gate failure Pulses, extracts structured rules (not free-form natural language), and injects them as Signals into the next Compose pass.

```
Gate failure Pulse -> React Cell (rule extraction) -> Signal (playbook rule)
    -> Compose Cell (prompt injection) -> Agent execution -> Gate verdict
    -> [feedback to React Cell]
```

**Key difference from paper**: Reflexion operates within a single task's retry loop. Roko's playbook rules persist across tasks and plans as durable Signals. A failure in plan A prevents the same mistake in plan B.

### 2.2 ExpeL (Zhao et al. 2023)

**Insight**: Extract generalizable "experiences" from successful and failed trials into a growing library.

**Unified expression**: An Observe Cell monitors completed episodes and distills them into skill Signals. Unlike Reflexion's reactive rules, ExpeL-style skills are proactive: they are injected before the agent starts, not after it fails.

**Key difference**: ExpeL uses natural language experiences without calibration. Roko's playbook rules have bounded confidence dynamics (validate +0.05, contradict -0.10) that automatically prune stale experiences via the predict-publish-correct pattern.

### 2.3 DSPy (Khattab et al. 2023)

**Insight**: Prompt optimization as a compiler problem: define a signature, generate variants, evaluate against a metric, select the winner.

**Unified expression**: The `ExperimentStore` is a Loop where a Route Cell (UCB1 bandit) assigns prompt variants, a Verify Cell (gate pipeline) evaluates them, and a React Cell promotes the winner.

**Key difference**: DSPy optimizes statically (generate many, evaluate on a test set, pick the best). Roko optimizes online (bandit-driven selection during live execution, continuous evaluation). This makes Roko responsive to non-stationarity but noisier per-variant.

### 2.4 ADAS (Hu et al. 2025)

**Insight**: A meta-agent searches the space of possible agent architectures by generating, evaluating, and iterating on designs in code.

**Unified expression**: An L4 Loop where the "parameter" being tuned is the Graph topology itself. The meta-agent modifies TOML Graph definitions, routing rules, and prompt templates. The Verify Cell is the same gate pipeline that validates agent output -- applied to the meta-agent's architectural proposals.

```
Architecture search -> Generate candidate Graph (TOML)
    -> Evaluate on held-out tasks via gate pipeline
    -> Score Cell (C-factor + outcome quality)
    -> React Cell (select, mutate, recombine)
    -> [feedback: better architectures for next round]
```

**Status**: ADAS is planned but not implemented. The components exist (TOML Graph definitions, gate pipeline, C-factor, ExperimentStore) but no meta-agent yet operates on them. See section 4 for what would be needed.

---

## 3. The Variance Inequality Bound

Self-improvement has a fundamental constraint from [07-LEARNING.md](../../unified/07-LEARNING.md): the **Variance Inequality** -- the verifier must be spectrally cleaner than the generator.

```
Var(verifier) < Var(generator)
```

In practical terms: the system that evaluates improvements must be more reliable than the system being improved. If the verifier is noisier than the generator, "improvements" are just noise that passes a noisy filter.

This constraint is load-bearing for every level of the self-improvement stack:

| Level | Generator | Verifier | Inequality holds? |
|---|---|---|---|
| 0 (tasks) | LLM agent | Compile + test + clippy (deterministic) | **Yes** -- deterministic gates are cleaner than stochastic LLM |
| 1 (parameters) | EMA tuning | Pass rate over window | **Yes** -- statistical aggregate cleaner than individual outcome |
| 2 (strategies) | Playbook extraction | Confidence interval with min samples | **Yes** -- bounded by sample size requirement |
| 3 (representations) | Prompt experiments | A/B test with significance threshold | **Yes** -- statistical test cleaner than individual variant |
| 4 (architecture) | Meta-agent proposal | C-factor AND outcome quality | **Conditionally** -- depends on C-factor calibration quality |

Level 4 is where the inequality is most at risk. C-factor is a learned composite metric, not a deterministic check. If C-factor weights are poorly calibrated, the verifier may be noisier than the generator, and structural evolution produces noise, not improvement.

### Constitutional Constraints

To maintain the Variance Inequality even at Level 4, the system operates under inviolable constraints:

```rust
/// Safety invariants that no learning subsystem can override.
/// These maintain the Variance Inequality by keeping the verifier
/// outside the modifiable surface.
pub struct ConstitutionalConstraints {
    /// Gate pipeline cannot be disabled or bypassed.
    pub gates_immutable: bool,               // always true
    /// Minimum gate count (compile + test at minimum).
    pub min_gate_count: usize,               // 2
    /// Gate threshold floor -- thresholds cannot drop below this.
    pub gate_threshold_floor: f64,           // 0.30
    /// These crates cannot be modified by automated self-improvement.
    pub forbidden_modification_crates: Vec<String>,  // ["roko-gate"]
    /// Maximum model downgrade depth (prevent cascading to weakest).
    pub max_downgrade_steps: u32,            // 2
    /// All self-modifications require human review.
    pub self_mod_requires_review: bool,      // true
}
```

The critical constraint: the gate pipeline (the verifier) sits **outside the modifiable surface**. The agent cannot modify its own verification pipeline. This is the structural guarantee that the Variance Inequality holds -- the verifier improves only through human intervention or through L4 proposals that pass the AND condition (C-factor AND outcome quality both improve).

---

## 4. Improvement Measurement

Self-improvement claims require rigorous measurement. Without principled metrics and experimental controls, apparent improvements may be noise, regression to the mean, or artifacts of changing task distributions.

### Four Key Metrics

The source docs identify four metrics that capture the full self-improvement signal:

| Metric | Definition | Which Loop improves it |
|---|---|---|
| **First-attempt pass rate** | % tasks passing gates first try | Playbook rules, skill injection |
| **Iterations per plan** | Avg iterations to complete | Model routing, better prompts |
| **Cost per plan** | Total USD per plan | Routing, cache optimization |
| **Prompt tokens per spawn** | Input tokens for initial prompt | Context assembly optimization |

### Controlled Experiments via Holdout

The gold standard for measuring improvement is a controlled experiment:

```rust
/// Holdout experiment: randomly assign tasks to treatment (learning active)
/// and control (learning frozen at baseline). Compare outcomes.
pub struct ImprovementExperiment {
    pub treatment_pct: u8,   // default: 80 (80% treatment, 20% holdout)
    pub min_tasks: usize,    // default: 100 before concluding
}
```

The holdout design ensures that observed improvements are caused by learning rather than external factors (easier task mix, model provider updates, codebase maturation).

### Monotonicity Tracking

Self-improvement should be monotonic over time. C-factor should not oscillate:

```
Monotonicity score = fraction of steps where C(t) > C(t-1)
Target: >= 0.60 over any 20-episode window
```

If monotonicity drops below 0.60, the learning system is not converging. Investigate: oscillation from competing Loops? regression from a bad rule promotion? environmental shift that invalidated cached strategies?

---

## 5. Gate Gaming Detection

The most insidious failure mode of self-improvement: the system learns to produce outputs that pass gates without actually solving the task. Detection requires monitoring for divergence between pass rate and output quality:

```rust
/// Detect gate gaming: pass rate rising while downstream quality falls.
pub struct GateGamingDetector {
    pub window_size: usize,              // 50 episodes
    pub pass_quality_divergence: f64,    // alert if pass +10% but quality -5%
    pub min_diff_size_fraction: f64,     // alert if diffs shrink to <30% of baseline
    pub min_output_fraction: f64,        // alert if output tokens <40% of baseline
}
```

Indicators that gaming is occurring:
1. Pass rate increases while downstream quality decreases (bugs discovered later).
2. Output complexity decreases (shorter, simpler code that technically passes).
3. Test coverage decreases while test pass rate increases (trivial tests).
4. Diff size shrinks toward zero (minimal changes that pass but do not address requirements).

---

## 6. The Predict-Publish-Correct Doctrine

The source docs on cybernetic loops (doc 18) describe a target-state extension: every operator becomes a predictor. This is the Bus-backed generalization of predict-publish-correct from [07-LEARNING.md](../../unified/07-LEARNING.md):

| Operator | Predicts | Outcome signal | Update policy |
|---|---|---|---|
| Score Cell | Candidate quality by axis | Gate verdict + episode reward | Online calibration per axis |
| Route Cell | Which model/action will succeed | Gate verdict | Contextual bandit updates |
| Compose Cell | Whether prompt fits budget and wins | Token count + gate verdict | Template EMA, variant selection |
| Verify Cell | Whether task will succeed post-patch | Next verdict + regression tests | Threshold smoothing, drift correction |

The architectural payoff: each operator can be calibrated independently once prediction and outcome are first-class Pulses on the Bus. Adding a new learner means writing one subscriber and one publisher, not threading another callback through the runtime.

**Current state** (per mori-diffs): active inference exists in `roko-learn` (`active_inference.rs`, ~255 lines) as a working Bayesian tier selector. The Router has the richest prediction/outcome signals. Bus-mediated calibration across every operator remains target-state.

---

## 7. Research-to-Runtime Pipeline

The source docs on research-to-runtime (doc 20) describe how academic papers can become runtime-active heuristics:

```
Paper -> Claim (testable hypothesis) -> Heuristic (with falsifier)
    -> Trial (run against real episodes) -> Calibration (update confidence)
```

The key property: evidence is cumulative, not ceremonial. The runtime trusts a paper's claim only through the behavior it continues to observe. If a claim's replication ledger weakens (the claimed effect diverges from observed effect), the parameter falls back to a safe default.

```rust
/// Target-state: resolve a parameter from a research claim.
/// Falls back to safe default if claim calibration degrades.
let epsilon = claim!("auer2002", "epsilon_greedy", default = 0.1)?;
```

**Current state**: No `Claim`, `Paper`, or replication-ledger code exists. The provenance-backed heuristic idea is valuable; the full paper economy is deferred.

---

## 8. Improvement Velocity Limits

Even beneficial improvements should be rate-limited to prevent cascade failures:

```rust
/// Rate limits on self-improvement to prevent cascade failures.
pub struct ImprovementVelocityLimits {
    pub max_rule_changes_per_day: u32,         // 10
    pub max_routing_changes_per_day: u32,      // 20
    pub max_experiment_conclusions_per_day: u32, // 5
    pub safety_violation_cooldown_minutes: u32,  // 60
    pub max_cfactor_delta: f64,                 // 0.02 per episode
}
```

These limits prevent a scenario where a false positive in the improvement pipeline triggers a cascade of changes that collectively degrade the system. By limiting the rate of change, the system has time to detect and recover from individual bad decisions.

---

## 9. Compound Improvement Math

The autocatalytic thesis models compound improvement as multiplicative across independent components:

```
compound_success = pass_rate_routing * pass_rate_prompts * pass_rate_skills * pass_rate_rules
```

If each component independently achieves 90% pass rate: `0.9^4 = 0.656`. Small uniform improvements multiply through the chain:

| Change | Compound pass rate | Absolute gain |
|---|---|---|
| All 90% | 0.656 | baseline |
| Routing 90% -> 95% | 0.692 | +3.6% |
| All 90% -> 92% | 0.716 | +6.0% |
| All 90% -> 95% | 0.815 | +15.9% |

**Caveats**: The components are not independent (better routing reduces the impact of prompt optimization), returns diminish as components approach their ceilings, and the multiplicative model overestimates when components are correlated. The C-factor trend is the empirical test: if it shows super-linear growth, the thesis holds. If linear or sub-linear, the thesis is falsified for the current implementation.

See [autocatalytic-compounding.md](autocatalytic-compounding.md) for the full Kauffman condition and feedback graph topology.

---

## 10. Mori-Diffs Reality

Per `tmp/mori-diffs/04-LEARNING.md`:

- **Learning subsystems** (`CascadeRouter`, `EpisodeLogger`, `ExperimentStore`, `SkillLibrary`, playbook rules) are all wired and recording data. The feedback Loops connecting them are mostly wired (see [missing-loops-and-calibration.md](missing-loops-and-calibration.md)).
- **ADAS** is not implemented. The meta-agent that operates on Graph topologies does not exist. The components it would use (TOML definitions, gate pipeline, C-factor, ExperimentStore) are wired.
- **Improvement measurement** (ImprovementScoreCard, holdout experiments) is designed but not instantiated at runtime. No controlled experiments are currently running.
- **Gate gaming detection** is designed but not wired as a runtime monitor.
- **Research-to-runtime** is entirely target-state. No code exists.
- **Predict-publish-correct** is implemented narrowly for routing (active inference in `roko-learn`). Broad operator coverage via Bus remains target-state.

---

## What This Enables

1. **Structured self-improvement**: five levels of improvement operating simultaneously, from parameter tuning to architectural evolution.
2. **Bounded improvement**: the Variance Inequality and constitutional constraints prevent the system from improving itself into a broken state.
3. **Measurable improvement**: four key metrics, holdout experiments, and monotonicity tracking distinguish genuine improvement from noise.
4. **Gaming resistance**: gate gaming detection catches the system optimizing for proxy metrics rather than real outcomes.
5. **Research integration pathway**: a design for making academic insights runtime-active with live calibration (target-state).

## Feedback Loops

- **L1 -> L4**: Parameter tuning observations (which thresholds work) inform structural evolution proposals.
- **Meta-Loop**: The system's self-improvement mechanisms are themselves subject to improvement. Playbook rules for modifying `roko-learn` accumulate as the system works on itself.
- **Variance Inequality enforcement**: The Verify pipeline (gates) sits outside the modifiable surface, ensuring the verifier stays cleaner than the generator across all levels.

## Open Questions

1. **Improvement ceiling**: The multiplicative model predicts diminishing returns as components approach 100%. What is the practical ceiling, and does it match the theoretical prediction? The C-factor saturation phase (500+ episodes) should answer this.
2. **Meta-loop stability**: When the system improves its own improvement mechanisms, can it introduce instabilities that compound faster than the stability mechanisms can damp? The constitutional constraints are a hard bound, but the soft interactions between Loops at levels 1-3 are not formally analyzed.
3. **Cross-project transfer**: Skills and heuristics from project A may accelerate project B, but transfer quality depends on structural similarity. HDC fingerprints enable fast matching, but the empirical transfer rate is untested at scale.
4. **ADAS search space**: What is the right search space for architectural evolution? TOML Graph definitions are one representation, but the space of possible Graph topologies is vast. How should the meta-agent constrain its search?
