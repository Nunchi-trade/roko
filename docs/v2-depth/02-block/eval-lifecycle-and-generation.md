# Eval Lifecycle and Generation

> Depth for [02-CELL.md](../../unified/02-CELL.md). The multi-timescale evaluation lifecycle, autonomous eval generation, and EvoSkills -- verification criteria that evolve through their own meta-Loop.

---

## Overview

Verification in Roko is not a single gate check. It is a 14-Loop system spanning five speed tiers, from sub-second machine checks to multi-day meta-evaluation. At each tier, Verify Cells produce Verdict Signals that feed into slower Loops, and insights from slower Loops tune the parameters of faster Loops.

On top of this lifecycle sits autonomous eval generation: the system creates its own Verify criteria (test cases, property assertions, invariants) and evolves them through EvoSkills -- a quality-diversity archive where skills compete, mutate, and speciate under verification pressure.

This doc describes the lifecycle, the generation pipeline, and the evolutionary mechanisms.

---

## 1. The Five Speed Tiers

Every evaluation Loop satisfies the **Karpathy Property**: if the evaluation metric improves, end-to-end performance improves. No metric is uncorrelated with task success, gameable without improving outcomes, or improvable at the expense of another metric.

### Tier 1: Machine Speed (sub-second to seconds, 5 Loops)

These run within or immediately after a single agent turn:

| Loop | What It Measures | Signal Kind |
|---|---|---|
| **Confidence Calibration** | Gap between predicted and actual pass rates (ECE) | Prediction Pulse vs Verdict Signal |
| **Context Attribution** | Which prompt sections correlated with Verify success | Section-outcome lift |
| **Cost-Effectiveness** | Token spend per Verdict quality | Efficiency Signal |
| **Tool Selection** | Redundant reads, unnecessary edits, unproductive calls | Tool-call pattern Signal |
| **Adversarial Awareness** | Detection of prompt injections, malicious test fixtures | Security Pulse |

### Tier 2: Cognitive Speed (seconds to minutes, 3 Loops)

These run during a single task execution across multiple turns:

| Loop | What It Runs | Cross-reference |
|---|---|---|
| **Gate Pipeline** | Rung selector -> Verify Cells -> Verdict cycle | [verify-as-universal-oracle.md](verify-as-universal-oracle.md) |
| **Error Diagnosis** | Feedback classification Pipeline | [gate-feedback-and-retry.md](gate-feedback-and-retry.md) |
| **Retry Logic** | Retry, escalate, or replan decision | [gate-feedback-and-retry.md](gate-feedback-and-retry.md) |

### Tier 3: Consolidation Speed (minutes to hours, 3 Loops)

These run after a batch of tasks (e.g., a full plan execution):

| Loop | What It Produces |
|---|---|
| **Skill Extraction** | Reusable tool-use patterns from successful episodes (see Section 4) |
| **Pattern Discovery** | Cross-task success/failure patterns (e.g., "auth tasks fail 3x more") |
| **Model Calibration** | Updated Thompson Sampling parameters for the Route Cell's bandit arms |

### Tier 4: Retrospective Speed (hours to days, 2 Loops)

| Loop | What It Does |
|---|---|
| **Shadow Testing** | Same tasks with different models/prompts; compare outcomes |
| **Reasoning Quality Review** | Alignment, consistency, and annotation quality across completed tasks |

### Tier 5: Meta Speed (days to weeks, 1 Loop)

| Loop | What It Does |
|---|---|
| **Meta-Learning Evaluation** | Evaluates whether the 13 other Loops are improving outcomes over time |

### Composition: Faster Feeds Slower

```
Machine Speed (5)   ->   Cognitive Speed (3)   ->   Consolidation (3)
  residuals               verdicts                   skills, patterns
  lift scores             retry outcomes             model calibration
  efficiency              feedback                        |
  tool patterns                                          v
  alerts                                         Retrospective (2)
                                                   shadow results
                                                   quality scores
                                                        |
                                                        v
                                                   Meta Speed (1)
                                                   "is learning net positive?"
```

---

## 2. The Four-Phase Lifecycle

Beyond speed tiers, evaluation goes through deployment phases:

| Phase | What | Data Source |
|---|---|---|
| **Trace Inspection** | Examine individual agent turns (raw data) | Episode log, efficiency events |
| **Backtesting** | Replay past executions with different parameters | Artifact store (exact inputs), threshold history |
| **Paper Trading** | Shadow mode alongside production, compare outcomes | Shadow vs production results |
| **Canary Deployment** | Gradual rollout to a fraction of tasks, monitor for regressions | A/B experiment results (`.roko/learn/experiments.json`) |

The **Gauntlet** validates the evaluation lifecycle itself:

| Speed | Duration | Scope |
|---|---|---|
| Smoke | 5 min | Core gate Pipeline on known test cases |
| Nightly | 2-4 hours | Full rung ladder on real project tasks |
| Full | 24-48 hours | All 14 Loops, cross-model comparison |

---

## 3. Autonomous Eval Generation

Autonomous eval generation is a Pipeline that creates Verify criteria before implementation begins. The key architectural decision: **test generation and implementation are performed by different agents**, creating an adversarial relationship that improves verification quality.

### 3.1 The Three-Stage Pipeline

```
Stage 1: Test Generation Cell
    Test agent reads task spec -> generates tests -> stores as immutable Signals (BLAKE3)
    |
    v
Stage 2: Test Validation Cell
    Compile + run tests against *current* codebase (before changes)
    New functionality tests: MUST FAIL (feature doesn't exist yet)
    Existing functionality tests: MUST PASS (baseline)
    Non-compiling tests: REJECTED
    |
    v
Stage 3: Test Registration Cell
    Validated tests registered with GeneratedTestGate (Rung 4)
    PropertyTestGate (Rung 5) for property-based tests
```

After implementation:
```
Generated tests -> PASS (feature now works) = verification success
Generated tests -> FAIL (feature broken)    = verification failure
```

### 3.2 Test Generation Strategies

**Example-Based** (most common, clear pass/fail semantics):

```rust
#[test]
fn rate_limiter_allows_within_limit() {
    let limiter = RateLimiter::new(100, Duration::from_secs(60));
    assert!(limiter.check("client-1").is_ok());
}
```

**Property-Based** (wider input space, catches edge cases):

```rust
#[proptest]
fn rate_limiter_never_allows_over_limit(limit in 1..100u32, requests in 1..200u32) {
    let limiter = RateLimiter::new(limit, Duration::from_secs(60));
    let mut allowed = 0;
    for _ in 0..requests {
        if limiter.check("client").is_ok() { allowed += 1; }
    }
    prop_assert!(allowed <= limit);
}
```

**Invariant-Based** (regression protection):

```rust
// Pre/post invariant: existing auth still works after agent changes
#[test]
fn existing_auth_still_works() {
    let auth = AuthService::new();
    assert!(auth.validate_token("valid-token").is_ok());
}
```

### 3.3 The Generation-Verification Gap (Song et al., ICLR 2025)

Self-improvement works only when verification capability exceeds generation capability. If the test generator produces tests easier than the implementation task, generated tests add no value. The system ensures test generation is at least as sophisticated as implementation by:

1. Using a capable model for test generation (not a cheap model)
2. Including edge cases, error conditions, adversarial inputs
3. Validating that generated tests fail before implementation (discrimination check)

### 3.4 Cheap-Model Convergence Loop

For simple tasks, a convergence Loop with a cheap model saves cost:

```
while not converged:
    cheap_model generates implementation attempt
    generated_tests run against attempt
    if all pass: converged = true
    else: feed errors back to cheap_model

if converged: submit to full gate Pipeline
else (after N attempts): escalate to expensive model
```

If 60% of tasks are solvable by a cheap model, overall cost drops ~50% while maintaining quality -- generated tests enforce the same standard regardless of which model produced the code.

### 3.5 Immutable Verification Artifacts

Generated tests are stored as immutable Signals in the Store before implementation begins:

1. **No tampering**: Implementation agent cannot modify the tests
2. **Reproducibility**: Exact tests retrievable by content hash (BLAKE3)
3. **Forensic replay**: Any Verdict can be replayed with original tests

### 3.6 Eval Quality Metrics

| Metric | Target |
|---|---|
| Test generation success rate (compile) | > 95% |
| Test discrimination (fail before impl) | > 80% |
| False positive rate (fail on correct impl) | < 5% |
| Coverage improvement over hand-written | > 20% |
| Cost per test | < $0.01 |

---

## 4. EvoSkills: Evolving Verification Skills

EvoSkills is a self-evolving skill library where verification-validated skills improve autonomously through adversarial surrogate verification and cross-model transfer.

Empirical results from the reference system:
- Baseline: 32% success rate
- With EvoSkills: 75% (+43pp)
- Cross-model transfer: +35-44pp

### 4.1 Three-Tier Learning Hierarchy

```
Tier 1: Episodes (Raw)
    Every execution in .roko/episodes.jsonl
    Task spec, tool calls, Verdicts, outcome, tokens
    Never modified or deleted

         | 5+ similar episodes with same tool-use sequence -> success
         v

Tier 2: Patterns (Extracted)
    Precondition: what task characteristics trigger this?
    Procedure: what tool calls compose the pattern?
    Postcondition: what Verify outcomes does it produce?
    Hypotheses, not yet validated.

         | 5+ successful production applications
         v

Tier 3: Playbook (Validated)
    Injected into agent context via Compose Cell's "skills" section
    Tracked with confidence scores and usage telemetry
    Confidence = (validations / (validations + failures)) x cross_model_factor
```

Cross-model factor:
- 1.0 if validated across 3+ models
- 0.8 across 2 models
- 0.6 on only 1 model

Skills below confidence 0.5 are demoted back to Tier 2.

### 4.2 Skill Structure

```rust
pub struct Skill {
    pub id: String,
    pub name: String,
    pub precondition: String,         // When to apply
    pub procedure: String,            // Tool call sequence summary
    pub postcondition: String,        // Expected Verify outcome
    pub confidence: f64,              // [0, 1]
    pub source_episodes: Vec<String>, // Episode IDs
    pub validations: u64,
    pub failures: u64,
    pub task_categories: Vec<String>,
}
```

Example:
```
Skill: "Rust Compile Fix -- Missing Import"
Precondition: compile Verify Cell fails with error[E0433] or error[E0425]
Procedure:
  1. Read error -> identify missing symbol
  2. Grep codebase for symbol
  3. Identify exporting crate/module
  4. Add use statement
Postcondition: compile Verify Cell passes
Confidence: 0.92 (46 validations, 4 failures, 3 models)
```

### 4.3 Adversarial Surrogate Verification

The breakthrough: validation through adversarial testing.

1. **Surrogate test generation**: For each candidate skill, generate adversarial test cases that try to break it (edge cases, input variations, failure modes)
2. **Cross-model testing**: Apply skill with different models. Skills that work across 3+ models capture genuine task knowledge, not model artifacts
3. **Confidence scoring**: Skills that survive adversarial + cross-model testing get high confidence; those that don't are demoted

### 4.4 Skill Evolution Mechanisms

**Refinement**: On failure, analyze why -- narrow precondition, add missing procedure step, correct postcondition.

**Specialization**: General skill fails for a sub-case -> create variant:
```
General:     "Fix compile error -- missing import"
Specialized: "Fix compile error -- missing import from workspace crate"
             (adds: check Cargo.toml for workspace deps before adding use)
```

**Retirement**: Confidence drops below threshold (codebase evolution obsoletes old patterns) -> remove from active playbook, retain in archive.

### 4.5 MAP-Elites Quality-Diversity Archive

Standard evolution converges to one solution. MAP-Elites maintains diverse, high-quality solutions across a behavioral space. For skills, we want a diverse repertoire, not one "best" skill.

```rust
/// 4-dimensional behavioral space for skill diversity.
pub struct BehavioralDescriptor {
    pub completion_rate: f64,     // Task completion [0, 1]
    pub gate_score: f64,          // Average Verify score [0, 1]
    pub token_efficiency: f64,    // Inverse tokens per success [0, 1]
    pub generalization: f64,      // Fraction of task categories covered [0, 1]
}

impl BehavioralDescriptor {
    /// Discretize into MAP-Elites cell (10 bins per axis = 10,000 cells).
    pub fn to_cell(&self, resolution: usize) -> [usize; 4] {
        let bin = |v: f64| ((v * resolution as f64) as usize).min(resolution - 1);
        [bin(self.completion_rate), bin(self.gate_score),
         bin(self.token_efficiency), bin(self.generalization)]
    }
}
```

The archive stores the highest-fitness genome per cell. Metrics:
- **Coverage**: fraction of cells filled (exploration)
- **QD-score**: sum of all fitness values (quality + diversity)

### 4.6 Skill Genome and Evolution Loop

Each skill genome encodes evolvable parameters: prompt template, tool preferences, retry config, temperature, token budget, gate weights.

```
Per generation (batch_size offspring):
    parent = archive.random_parent()
    offspring = mutate(parent)  // prompt rephrase, tool weight adjust, param perturb
    fitness, behavior = evaluate(offspring)  // run on sampled tasks, observe Verdicts
    archive.try_insert(offspring)  // keep if best for its behavioral niche
```

Mutation operators:
- **Prompt mutation** (rate 0.3): rephrase, add/remove context
- **Tool mutation** (rate 0.2): adjust preference weights
- **Continuous perturbation** (sigma 0.1): temperature, token budget
- **Crossover** (rate 0.2): recombine two parents

### 4.7 Speciation

Skills using fundamentally different strategies (error-reading vs codebase-searching) are protected from competing directly. Speciation groups similar genomes by compatibility distance and allocates evaluation budget proportionally.

```rust
/// Compatibility distance (NEAT-style, Stanley & Miikkulainen 2002).
/// Genomes within threshold -> same species.
/// Threshold dynamically adjusted to maintain ~5 species.
distance = c_prompt * prompt_dist   // Jaccard of n-gram sets
         + c_tools * tool_dist      // Cosine of preference vectors
         + c_params * param_dist    // Normalized L2 of continuous params
```

Stagnant species (15 generations without improvement) are dissolved.

### 4.8 Landscape-Adaptive Evolution

The fitness landscape is analyzed to adjust evolution strategy:

| Landscape Property | Response |
|---|---|
| Rugged (many local optima) | Increase mutation strength |
| Flat (plateaus) | Increase crossover, reduce random walk |
| Deceptive (gradient away from optimum) | Switch to novelty-driven search |
| Low evolvability (most mutations harmful) | Reduce mutation, increase elitism |

### 4.9 CMA-ES for Continuous Parameters

For temperature, token budget, and gate weights, CMA-ES (Covariance Matrix Adaptation) is more sample-efficient than random mutation. It learns parameter correlations and adapts step size. Operates alongside MAP-Elites: CMA-ES handles continuous optimization, MAP-Elites handles structural diversity.

### 4.10 AURORA: Learned Behavioral Descriptors

Hand-crafted descriptors may miss important behavioral axes. AURORA trains a VAE on execution traces to discover latent behavioral dimensions automatically. Benefits: discovers axes like "cautious vs aggressive editing" without manual specification, and adapts to the codebase's actual behavioral diversity.

---

## 5. The Meta-Loop: Evaluation Evolves Itself

The entire evaluation lifecycle is itself a Loop. The meta-learning Loop (Loop 14) evaluates whether the 13 other Loops are improving outcomes:

```
Inner Loops (1-13) produce Verdict Signals
    |
    v
Meta-Learning Loop (14) measures:
    - Are gate pass rates improving over time?
    - Is the skill library reducing failure rates?
    - Are model routing decisions getting better?
    - Is the cost-per-success declining?
    |
    v
If net positive: continue current strategy
If net negative: adjust Loop parameters or disable underperforming Loops
```

This is a predict-publish-correct Loop applied to the evaluation system itself. The meta-Loop publishes a prediction ("the skill library improves outcomes by X%"), reality publishes the actual outcome, and the meta-Loop corrects its assessment.

---

## What This Enables

1. **Multi-timescale improvement**: Fast Loops catch immediate errors, slow Loops identify structural issues, meta-Loop validates the whole system.
2. **Self-generating verification**: Agents create their own tests, creating adversarial pressure that improves both tests and implementations.
3. **Evolving skills**: Verification-validated skills compound across tasks and models. New models get instant expertise (+35-44pp from cross-model transfer).
4. **Cost compounding**: Cheap-model convergence + generated tests = 50% cost reduction while maintaining quality; skills reduce steps by 26% and tokens by 59% (SAGE).

## Feedback Loops

| Loop | Speed | What Feeds It | What It Produces |
|---|---|---|---|
| Confidence calibration | Machine | Prediction vs Verdict | ECE correction |
| Context attribution | Machine | Section-outcome pairs | Section priority adjustments |
| Gate Pipeline | Cognitive | Agent output | Verdict Signals |
| Skill extraction | Consolidation | Successful episodes | Playbook rules |
| Shadow testing | Retrospective | Alternative configurations | Routing improvements |
| Meta-learning | Meta | All other Loops' metrics | Strategy adjustments |
| Eval generation | Cognitive | Task specs | Test Signals |
| EvoSkills evolution | Consolidation | Genome fitness evaluations | Archive improvements |

## Open Questions

1. **Eval generation for non-code tasks**: The generation pipeline assumes code (compile, test). How to generate verification criteria for documentation, configuration, or design tasks?
2. **Adversarial co-evolution**: Test generators and implementation agents may co-evolve in unproductive ways (tests become easy, implementations become brittle). How to detect and break co-evolutionary cycles?
3. **Skill transfer across codebases**: Skills are codebase-specific. Can skills transfer to a new codebase? What would a "skill embedding" look like?
4. **Meta-Loop stability**: The meta-Loop adjusts other Loops. What prevents oscillation? A damping factor or hysteresis threshold may be needed.
5. **AURORA training data**: How many execution traces are needed before the VAE discovers meaningful behavioral dimensions? Early in a project's life, hand-crafted descriptors may be necessary.

---

## References

- [02-CELL.md](../../unified/02-CELL.md) -- Verify protocol, predict-publish-correct
- [07-LEARNING.md](../../unified/07-LEARNING.md) -- Learning Loops, autocatalytic compounding
- Song et al. (ICLR 2025) -- Generation-Verification Gap
- SAGE (arXiv:2512.17102) -- Skill accumulation: 26% fewer steps, 59% fewer tokens
- Voyager (Wang et al. 2023) -- Skill library as long-term procedural memory
- Mouret & Clune (2015) -- MAP-Elites quality-diversity
- Stanley & Miikkulainen (2002) -- NEAT speciation
- Hansen (arXiv:1604.00772) -- CMA-ES tutorial
- AURORA (2021-2024) -- Learned behavioral descriptors via VAE
