# 07 — Process Reward Models

> **Layer**: L3 Harness — Verification
> **Crate**: `roko-learn` (planned integration), `roko-gate` (data source)
> **Status**: Design, informed by active gate infrastructure


> **Implementation**: Shipping

---

## 1. Overview

Process reward models (PRMs) score intermediate reasoning steps rather than only the
final output. In the context of agent-driven development, this means evaluating the
agent's *process* — each tool call, each file edit, each reasoning turn — not just
whether the final code passes gates.

Standard verification is binary: the code either compiles or it doesn't, tests either
pass or they don't. Process rewards add granularity: *how much progress did the agent
make toward a working solution?* This matters because:

- An agent that gets 90% of the way to a working solution before failing is more
  promising than one that makes no progress
- Intermediate progress signals enable early intervention (abandon a failing approach
  before it consumes the full retry budget)
- Process rewards provide 10x richer training signal than final pass/fail outcomes

> **Citation**: Lightman et al. "Let's Verify Step by Step" (2023) — PRM800K dataset,
> process reward models for mathematical reasoning.

> **Citation**: refactoring-prd/02-five-layers.md — "Process Reward Models: Promise +
> Progress scoring, low Promise → early intervention, negative Progress → re-planning."

---

## 2. The Two Dimensions: Promise and Progress

Roko's process reward model tracks two orthogonal signals per agent execution:

### 2.1 Promise

**Promise** estimates how likely the current execution is to eventually succeed, given
the work done so far. It answers: "Is this approach heading somewhere good?"

Indicators of high Promise:
- The agent is making compile-passing edits (Rung 0 passes)
- The agent's tool calls are targeting the right files
- The edit pattern matches successful historical executions for similar tasks

Indicators of low Promise:
- The agent is making the same edit repeatedly (loop detection)
- The agent's tool calls are targeting unrelated files
- The compile error count is increasing, not decreasing

**Intervention**: Low Promise → early termination of the current attempt, before the
full retry budget is consumed. Better to start fresh with a different prompt or model
than to continue down a failing path.

### 2.2 Progress

**Progress** measures whether the agent is advancing toward the goal or stalling. It
answers: "Is this attempt doing better than the previous one?"

Indicators of positive Progress:
- Higher rung reached than the previous attempt (ratchet advancing)
- Fewer compile errors than the previous attempt
- More tests passing than the previous attempt

Indicators of negative Progress:
- Same or lower rung than the previous attempt
- Same or more compile errors
- Same or fewer tests passing

**Intervention**: Negative Progress across multiple attempts → re-planning. The current
plan may be fundamentally flawed. Rather than retrying with the same approach, generate
a new plan and start over.

---

## 3. Data Sources for Process Rewards

The gate infrastructure provides rich data for computing process rewards:

### 3.1 Per-Turn Tool Call Metadata

Each agent turn produces a `ToolCallMeta`:
```rust
pub struct ToolCallMeta {
    pub tool_name: String,
    pub duration_ms: u64,
    pub result_tokens: u64,
    pub succeeded: bool,
    pub advanced_task: bool,      // Did this call advance the task?
    pub was_redundant: bool,      // Was this call unnecessary?
    pub error_category: Option<String>,
}
```

The `advanced_task` flag can be computed post-hoc: if a tool call's output was referenced
in the final solution (the agent used the information), it advanced the task. If not,
it was wasted work.

> **Citation**: tmp/implementation-plans/modelrouting/12-advanced-patterns.md §K
> (Task 2J.16) — "Per-step rewards provide 10x richer signal than final pass/fail."

### 3.2 Gate Verdicts with Rung Information

Each gate pipeline execution produces verdicts with:
- Which rung was reached (highest passing rung)
- Test counts (passed/failed/ignored) — a proxy for "how close to passing"
- Error digest — what went wrong, machine-parseable

### 3.3 Diff Analysis

The `DiffGate`'s analysis provides:
- How many substantive lines were added
- Whether the additions are vacuous (todo!/unimplemented!)
- Whether the code is getting more or less complete

### 3.4 Ratchet State

The `GateRatchet` tracks the highest rung passed per plan. Changes in the ratchet state
across attempts reveal whether the agent is making progress:
- Ratchet advances from rung 1 to rung 2: positive Progress
- Ratchet stays at rung 1 after 3 attempts: stalling
- Ratchet would regress from rung 2 to rung 1: negative Progress (blocked by ratchet)

---

## 4. The Promise Score Function

Promise is computed as a weighted combination of signals:

```
Promise(attempt) = w₁ × rung_fraction
                 + w₂ × test_pass_rate
                 + w₃ × error_trend
                 + w₄ × tool_efficiency
```

Where:
- `rung_fraction` = highest_rung_passed / total_rungs (0.0 to 1.0)
- `test_pass_rate` = tests_passed / total_tests (0.0 to 1.0, or 0.5 if no test gate)
- `error_trend` = 1.0 if errors decreasing, 0.5 if stable, 0.0 if increasing
- `tool_efficiency` = useful_tool_calls / total_tool_calls

Default weights: w₁=0.4, w₂=0.3, w₃=0.2, w₄=0.1.

### Promise Thresholds

| Promise | Interpretation | Action |
|---|---|---|
| > 0.8 | High — likely to succeed | Continue, possibly reduce retries |
| 0.4–0.8 | Moderate — uncertain | Continue with standard retries |
| 0.2–0.4 | Low — probably failing | Consider early termination |
| < 0.2 | Very low — almost certainly failing | Terminate, try different approach |

---

## 5. The Progress Score Function

Progress compares the current attempt to the previous one:

```
Progress(attempt_n) = Δrung + Δtest_rate + Δerror_count
```

Where:
- `Δrung` = (current_rung - previous_rung) / total_rungs
- `Δtest_rate` = current_pass_rate - previous_pass_rate
- `Δerror_count` = (previous_errors - current_errors) / max(previous_errors, 1)
  (positive = improvement)

### Progress Thresholds

| Progress | Interpretation | Action |
|---|---|---|
| > 0.1 | Advancing | Continue, approach is working |
| -0.1 to 0.1 | Stalling | Escalate complexity, adjust prompt |
| < -0.1 | Regressing | Stop retrying, re-plan |

---

## 6. Integration with the Feedback Loop

Process rewards create a fast feedback loop within the agent execution cycle:

```
Agent turn N
    ↓
Gate pipeline
    ↓
Compute Promise(N) and Progress(N)
    ↓
Decision:
  Promise > 0.4 and Progress > -0.1 → continue to turn N+1
  Promise < 0.2 → early termination
  Progress < -0.1 for 3 turns → re-plan
```

This is a *within-attempt* optimization. It happens faster than the retry loop
(which operates across attempts) and much faster than the escalation mechanism
(which operates across failed attempts).

The three feedback timescales:
1. **Process reward** (per-turn): Promise/Progress → continue/terminate
2. **Retry loop** (per-attempt): Gate verdict → retry with adjusted prompt
3. **Escalation** (across attempts): Repeated failure → add rungs, re-plan

> **Citation**: bardo-backup/prd/16-testing/07-fast-feedback-loops.md — Machine-speed
> evaluation loops: confidence calibration, context attribution, cost-effectiveness.

---

## 7. Academic Foundations

### 7.1 Lightman et al. (2023) — PRM800K

Showed that process supervision (scoring each step) outperforms outcome supervision
(scoring only the final answer) for mathematical reasoning. Best-of-N selection with
process rewards outperformed majority voting by 8%.

### 7.2 AgentPRM (arXiv:2502.10325)

Extended process rewards to agent tool-use settings. Per-step rewards provide 10x richer
signal than final pass/fail. The key insight: not all tool calls contribute equally to
the outcome. Scoring each call identifies which calls are productive vs. wasteful.

### 7.3 Self-Refine (Madaan et al. 2023)

Showed that LLMs can improve their own outputs through iterative refinement with
feedback. Process rewards formalize the feedback signal that drives refinement: rather
than generic "try again," the agent gets "your Promise score dropped because error count
increased — focus on reducing errors."

### 7.4 Reflexion (Shinn et al. 2023)

Introduced the concept of verbal reinforcement for agents: converting numeric feedback
into natural language that the agent can learn from. Process rewards can be converted
to reflexion-style feedback: "Your last 3 attempts reached Rung 1 but failed at Rung 2.
Test failures are all in the auth module. Focus on auth module tests."

> **Citation**: bardo-backup/tmp/mori-refactor/06-harness.md — Academic foundations
> section: "Process Reward Models (Lightman et al. 2023 PRM800K), Self-Refine (Madaan
> et al. 2023), Reflexion (Shinn et al. 2023)."

---

## 8. Promise + Progress as Cybernetic Signals

In the Synapse Architecture's cybernetic feedback model, gate verdicts flow back to
Scorer, Router, Composer, and the agent. Process rewards add two new feedback channels:

| Signal | From | To | Effect |
|---|---|---|---|
| Promise | Gate + ToolCallMeta | Conductor | Early termination on low Promise |
| Progress | Gate (across attempts) | Router | Model re-selection on stalling |
| Progress | Gate (across attempts) | Composer | Prompt adjustment on stalling |
| Promise × Progress | Combined | Policy | Re-plan on persistent low signals |

This creates a multi-timescale control system:
- **Fast**: Promise per-turn → terminate unproductive attempts
- **Medium**: Progress per-attempt → adjust routing and prompts
- **Slow**: Trend across plans → update model preferences and prompt templates

> **Citation**: refactoring-prd/01-synapse-architecture.md — Cybernetic feedback loops
> from Gate to Scorer, Router, Composer.

---

## 9. Relationship to Predictive Foraging

Process rewards connect to the predictive foraging system (from the agent-chain
architecture). Before a task starts, the router predicts success probability. After the
task runs, the actual outcome produces a residual (prediction - reality). Process
rewards refine these predictions:

- If Promise is high but the task fails → the predictor was overconfident
- If Promise is low but the task succeeds → the predictor was underconfident
- Residuals from process rewards calibrate the predictor faster than binary outcomes

> **Citation**: tmp/implementation-plans/modelrouting/12-advanced-patterns.md §B
> (Tasks 2J.03–2J.04) — Predictive Foraging: prediction → residual → bias correction.

---

## 10. Summary

Process reward models transform binary gate verdicts into a continuous signal of agent
quality. By tracking Promise (is this attempt heading somewhere good?) and Progress (is
the agent improving across attempts?), the system can make finer-grained decisions about
when to continue, when to terminate early, and when to re-plan entirely.

The data is already there — gate verdicts, tool call metadata, ratchet state, diff
analysis. Process rewards are a *lens* on this data, not a new data collection system.
They turn "passed/failed" into "making progress / stalling / regressing" and enable
interventions that save compute and improve outcomes.

---

## 11. Self-Supervised PRM Training from Gate Verdicts

Roko has a unique advantage over academic PRM systems: it generates its own step-level
training labels. The gate pipeline is a deterministic oracle. Every intermediate artifact
can be verified, producing automated labels without human annotation.

> **Citation**: Lightman et al. "Let's Verify Step by Step" (arXiv:2305.20050, 2023) —
> PRM800K required 800K human labels. Self-supervised approaches eliminate this cost.

> **Citation**: "Process-Supervised Reinforcement Learning for Code Generation"
> (arXiv:2502.01715, 2025) — compiler-driven step-level rewards for code RL.

### 11.1 The Self-Supervision Loop

```
Agent execution trace:
  step_1: read file → artifact_1 (no code change)
  step_2: edit file → artifact_2 (code changed)
  step_3: edit file → artifact_3 (code changed)
  step_4: run tests → artifact_4 (no code change)
  step_5: fix test → artifact_5 (code changed)

Gate verification of each intermediate artifact:
  artifact_1: N/A (no code change — label inherited from previous)
  artifact_2: compile PASS, test FAIL → partial credit
  artifact_3: compile PASS, test PASS 8/10 → more credit
  artifact_5: compile PASS, test PASS 10/10 → full credit

Step-level labels:
  step_1: 0.3 (read → neutral, but necessary)
  step_2: 0.5 (compiles but tests fail)
  step_3: 0.7 (more tests pass)
  step_4: 0.3 (information-gathering, no code progress)
  step_5: 1.0 (all tests pass)
```

### 11.2 Monte Carlo Step-Level Q-Values

For richer labels than binary gate outcomes, use Monte Carlo rollouts to estimate the
probability that a step leads to eventual success:

```rust
/// Monte Carlo estimator for step-level quality values.
///
/// For each intermediate step, estimate the probability that continuing
/// from that state leads to eventual gate passage. This is the Q-value
/// of the step under the current policy.
pub struct MonteCarloStepLabeler {
    /// Number of rollouts per step (more = better estimate, higher cost).
    pub num_rollouts: usize,    // default: 8
    /// Maximum additional turns per rollout.
    pub max_rollout_turns: usize, // default: 10
    /// Gate pipeline to evaluate rollout outcomes.
    pub gate_pipeline: GatePipeline,
    /// Agent backend for generating rollout continuations.
    pub agent: Box<dyn AgentBackend>,
}

impl MonteCarloStepLabeler {
    /// Estimate the Q-value of reaching this intermediate state.
    ///
    /// Pseudocode:
    ///   successes = 0
    ///   for k in 0..num_rollouts:
    ///       continuation = agent.continue_from(state, max_turns=max_rollout_turns)
    ///       verdict = gate_pipeline.verify(continuation.artifact)
    ///       if verdict.passed:
    ///           successes += 1
    ///   q_value = successes / num_rollouts
    ///   label = "correct" if q_value > 0.5 else "incorrect"
    pub async fn estimate_q_value(&self, state: &IntermediateState) -> StepLabel {
        let mut successes = 0;
        for _ in 0..self.num_rollouts {
            let continuation = self.agent
                .continue_from(state, self.max_rollout_turns).await;
            let verdict = self.gate_pipeline
                .verify(&continuation.as_signal(), &state.context).await;
            if verdict.passed {
                successes += 1;
            }
        }
        let q_value = successes as f64 / self.num_rollouts as f64;
        StepLabel {
            q_value,
            label: if q_value > 0.5 { StepQuality::Correct } else { StepQuality::Incorrect },
            confidence: wilson_confidence(successes as u64, self.num_rollouts as u64),
        }
    }
}

pub struct StepLabel {
    pub q_value: f64,
    pub label: StepQuality,
    pub confidence: f64,
}

pub enum StepQuality {
    Correct,
    Incorrect,
    Ambiguous, // q_value near 0.5
}
```

**Cost analysis**: With 8 rollouts per step and ~5 code-modifying steps per task, this
requires 40 additional agent turns plus 40 gate evaluations. At Haiku-tier costs (~$0.001
per turn), the total labeling cost is ~$0.04 per task. For 100 tasks, $4 produces
~500 step-level labels — orders of magnitude cheaper than human annotation.

### 11.3 FoVer: Formally Verified Labels

For code changes that can be expressed as logical assertions, formal verification
provides perfect step-level labels:

> **Citation**: Kamoi et al., "FoVer: Generalizable Process Reward Models via Formally
> Verified Training Data" (arXiv:2505.15960, 2025) — Z3 and Isabelle for automatic labels.

```rust
/// Formally verified step labeling for code changes.
///
/// For steps that modify contracts, invariants, or type-level properties,
/// attempt formal verification of the intermediate state.
pub struct FormalStepLabeler {
    /// Verification backend (Z3, Isabelle, or Prusti for Rust).
    pub verifier: Box<dyn FormalVerifier>,
    /// Maximum verification time per step.
    pub timeout: Duration,  // default: 30s
}

pub trait FormalVerifier: Send + Sync {
    /// Attempt to verify that the code change preserves stated invariants.
    ///
    /// Returns Verified (label=correct), Counterexample (label=incorrect),
    /// or Timeout/Unknown (no label).
    fn verify(&self, pre_state: &Code, post_state: &Code,
              invariants: &[Invariant]) -> VerificationResult;
}

pub enum VerificationResult {
    /// Invariants hold — step is correct.
    Verified,
    /// Counterexample found — step introduced a bug.
    Counterexample(String),
    /// Verification timed out or was inconclusive.
    Unknown,
}
```

FoVer labels are high-confidence but limited to steps where formal specs exist.
In practice, ~10-20% of code changes in a typed language like Rust can be formally
checked (type constraints, trait bounds, lifetime invariants). The rest use Monte Carlo.

---

## 12. RLHF Alternatives for Agent Improvement

Gate verdicts provide a natural reward signal. The question is how to convert
these rewards into improved agent behavior across future tasks.

### 12.1 DPO (Direct Preference Optimization)

DPO avoids explicit reward model training by directly optimizing the policy from
preference pairs. For agent verification, preference pairs come from gate outcomes:

> **Citation**: Rafailov et al., "Direct Preference Optimization: Your Language Model
> Is Secretly a Reward Model" (NeurIPS 2023, arXiv:2305.18290).

```rust
/// DPO training pair from gate verdicts.
///
/// When two agents attempt the same task and one passes gates while
/// the other fails, this creates a natural preference pair.
pub struct DpoTrainingPair {
    /// The task specification (shared context).
    pub task_spec: String,
    /// The preferred response (passed all gates).
    pub preferred: AgentTrace,
    /// The dispreferred response (failed gates).
    pub dispreferred: AgentTrace,
    /// Margin: how much better the preferred response was.
    /// Higher margin = stronger training signal.
    pub margin: f64,
}

/// DPO loss function (for reference — training happens offline):
///
/// L_DPO(θ) = -E[ log σ( β × (
///     log π_θ(y_w|x) / π_ref(y_w|x)
///   - log π_θ(y_l|x) / π_ref(y_l|x)
/// ))]
///
/// where:
///   x = task_spec
///   y_w = preferred trace
///   y_l = dispreferred trace
///   β = temperature (sharpness of reward model)
///   π_ref = reference policy (the untuned model)

/// Parameters for DPO-derived preference collection.
pub struct DpoConfig {
    /// Temperature parameter controlling reward sharpness.
    /// Lower = more decisive preferences. Default: 0.1.
    pub beta: f64,
    /// Minimum margin between preferred/dispreferred.
    /// Pairs with margin < this are too similar to be useful.
    pub min_margin: f64,          // default: 0.3
    /// Maximum pairs to collect before triggering training.
    pub batch_size: usize,        // default: 128
    /// Storage path for collected pairs.
    pub pairs_path: PathBuf,
}
```

**Implicit reward extraction**: DPO implicitly defines a reward model:
`r(x, y) = β × log(π_θ(y|x) / π_ref(y|x))`. This can be extracted and used
for the process reward scoring without separate PRM training.

> **Citation**: "Bootstrapping Language Models with DPO Implicit Rewards" (ICLR 2025)
> — using DPO's implicit reward for self-improvement.

### 12.2 RLAIF (Reinforcement Learning from AI Feedback)

Instead of human preferences, use a judge model to generate preferences from gate
verdicts and code quality analysis:

```rust
/// RLAIF configuration for generating AI feedback from gate signals.
pub struct RlaifConfig {
    /// The judge model (e.g., Opus) that evaluates traces.
    pub judge_model: String,
    /// Aspects to evaluate (code quality, efficiency, correctness).
    pub evaluation_criteria: Vec<EvaluationCriterion>,
    /// Whether to include gate verdicts as context for the judge.
    pub include_gate_context: bool,  // default: true
}

pub struct EvaluationCriterion {
    pub name: String,
    pub weight: f64,
    pub prompt_template: String,
}

/// RLAIF feedback generation:
///
/// 1. Collect two traces for the same task (different models or attempts)
/// 2. Show both traces + gate verdicts to the judge model
/// 3. Judge produces: {preferred: "A"|"B", reasoning: "...", confidence: 0.9}
/// 4. Store as DPO training pair with judge's confidence as margin
///
/// This is cheaper than DPO because the judge model doesn't need to
/// generate rollouts — it only evaluates existing traces.
```

### 12.3 Constitutional AI for Safety Gates

Safety-related gate failures (e.g., code that introduces security vulnerabilities,
race conditions, or resource leaks) can be addressed with a Constitutional AI approach:

```rust
/// Constitutional principles for the safety gate.
pub const SAFETY_CONSTITUTION: &[&str] = &[
    "Generated code must not introduce SQL injection, XSS, or command injection.",
    "Generated code must not disable authentication or authorization checks.",
    "Generated code must not expose secrets in logs, error messages, or comments.",
    "Generated code must not introduce race conditions or deadlocks.",
    "Generated code must not create unbounded resource allocation.",
];

/// Self-critique loop using constitutional principles:
///
/// 1. Agent produces code change
/// 2. Critic (same or different model) evaluates against constitution
/// 3. If violation found: generate revised version that fixes the violation
/// 4. Repeat until clean or max iterations
/// 5. Run gate pipeline on the final version
///
/// The critique step happens BEFORE the gate pipeline, catching safety
/// issues that static gates might miss (gates check syntax/tests, not
/// security semantics).
```

---

## 13. Reward Shaping for Intermediate Steps

Raw gate verdicts are sparse: most intermediate steps produce no verdict (no code
change = nothing to verify). Reward shaping fills the gaps with dense signals that
guide the agent without changing the optimal policy.

> **Citation**: Ng et al., "Policy Invariance Under Reward Transformations" (ICML 1999)
> — potential-based reward shaping preserves optimal policy.

### 13.1 Potential-Based Reward Shaping

```rust
/// Potential function over agent states.
///
/// Maps each intermediate state to a scalar that estimates "how close
/// to passing gates" the agent is. The shaped reward is:
///
///   R'(s, a, s') = R(s, a, s') + γ × Φ(s') - Φ(s)
///
/// where γ = discount factor and Φ = potential function.
/// This preserves the optimal policy (Ng et al. 1999 theorem).
pub struct GatePotential {
    /// Weight for each component of the potential.
    pub weights: PotentialWeights,
}

pub struct PotentialWeights {
    /// Weight for compilation status (0/1).
    pub compile_weight: f64,      // default: 0.4
    /// Weight for test pass rate [0, 1].
    pub test_rate_weight: f64,    // default: 0.3
    /// Weight for lint cleanliness [0, 1].
    pub lint_weight: f64,         // default: 0.1
    /// Weight for code completeness (1 - stub_fraction) [0, 1].
    pub completeness_weight: f64, // default: 0.2
}

impl GatePotential {
    /// Compute the potential of an intermediate state.
    ///
    /// Higher potential = closer to passing all gates.
    pub fn phi(&self, state: &IntermediateState) -> f64 {
        let compile = if state.compiles { 1.0 } else { 0.0 };
        let test_rate = state.tests_passed as f64
            / state.tests_total.max(1) as f64;
        let lint = if state.lint_clean { 1.0 } else { 0.5 };
        let completeness = 1.0 - state.stub_fraction();

        self.weights.compile_weight * compile
            + self.weights.test_rate_weight * test_rate
            + self.weights.lint_weight * lint
            + self.weights.completeness_weight * completeness
    }

    /// Compute the shaped reward for a state transition.
    ///
    /// Positive when the agent moves toward passing gates.
    /// Negative when the agent moves away.
    /// Zero when no progress (encourages efficiency).
    pub fn shaped_reward(&self, prev: &IntermediateState,
                         next: &IntermediateState,
                         discount: f64) -> f64 {
        discount * self.phi(next) - self.phi(prev)
    }
}
```

### 13.2 Shaping Signal Interpretation

```
Step: agent adds a use statement (fixes compile error)
  Φ(prev) = 0.0 (doesn't compile)
  Φ(next) = 0.4 (compiles, no tests pass yet)
  Shaped reward: 0.99 × 0.4 - 0.0 = +0.396 (positive: progress)

Step: agent deletes a test (makes test suite pass vacuously)
  Φ(prev) = 0.7 (compiles, 7/10 tests pass)
  Φ(next) = 0.6 (compiles, 7/7 tests pass but completeness drops)
  Shaped reward: 0.99 × 0.6 - 0.7 = -0.106 (negative: regression)

Step: agent reads a file (no code change)
  Φ(prev) = 0.5
  Φ(next) = 0.5 (unchanged)
  Shaped reward: 0.99 × 0.5 - 0.5 = -0.005 (near zero: encourages efficiency)
```

The potential-based shaping naturally penalizes vacuous changes (deleting tests to
pass) and rewards genuine progress (fixing errors to pass), without any hand-coded
rules for these behaviors.

### 13.3 Dense Reward Schedule

Combining sparse gate rewards with shaped potential rewards:

```rust
/// Complete per-step reward computation.
pub struct StepRewardComputer {
    pub gate_potential: GatePotential,
    pub discount: f64,               // default: 0.99
    pub gate_weight: f64,            // default: 1.0 (sparse gate reward weight)
    pub shaping_weight: f64,         // default: 0.5 (shaped reward weight)
}

impl StepRewardComputer {
    pub fn compute(&self, prev: &IntermediateState,
                   next: &IntermediateState,
                   gate_verdict: Option<&Verdict>) -> f64 {
        // Sparse gate reward (only present on code-modifying steps)
        let gate_reward = gate_verdict
            .map(|v| if v.passed { 1.0 } else { -0.5 })
            .unwrap_or(0.0);

        // Dense shaped reward (every step)
        let shaped = self.gate_potential.shaped_reward(prev, next, self.discount);

        self.gate_weight * gate_reward + self.shaping_weight * shaped
    }
}
```

---

## 14. ThinkPRM: Generative Process Verification

Rather than training a discriminative PRM (which requires labeled data), use a
generative approach: ask a reasoning model to verify each step by thinking through it.

> **Citation**: Mukhal et al., "Process Reward Models That Think" (arXiv:2504.16828,
> 2025) — ThinkPRM fine-tuned on 1K synthetic CoTs outperforms discriminative PRMs
> using 1% of typical annotation cost.

```rust
/// ThinkPRM: a generative process verifier that reasons about step correctness.
///
/// Instead of learning a classifier, this uses chain-of-thought reasoning
/// to verify each step. The model thinks through "does this step make
/// sense given the task and previous steps?" and outputs a verdict.
pub struct ThinkPrm {
    /// The reasoning model used for verification.
    /// Prefer a capable model (Sonnet+) for reliable reasoning.
    pub model: String,
    /// Maximum reasoning tokens per step verification.
    pub max_reasoning_tokens: usize,  // default: 1024
    /// Score threshold for marking a step as incorrect.
    pub threshold: f64,               // default: 0.5
}

impl ThinkPrm {
    /// Verify a single step in context.
    ///
    /// Prompt structure:
    ///   "You are verifying step {i} of an agent's execution.
    ///    Task: {task_description}
    ///    Previous steps: {step_1..step_{i-1}}
    ///    Current step: {step_i}
    ///    Gate results so far: {gate_verdicts}
    ///
    ///    Think step by step about whether this step is:
    ///    1. Moving toward solving the task
    ///    2. Consistent with previous steps
    ///    3. Likely to lead to gate passage
    ///
    ///    Score the step from 0.0 (definitely wrong) to 1.0 (definitely correct)."
    ///
    /// Returns the model's step score and reasoning chain.
    pub async fn verify_step(&self, step: &Step,
                              context: &StepContext) -> StepVerification {
        // ... prompt construction and model call
        todo!()
    }
}

pub struct StepVerification {
    pub step_index: usize,
    pub score: f64,
    pub reasoning: String,
    pub is_correct: bool,
    pub verification_tokens: usize,
}
```

**Cost-effectiveness**: ThinkPRM requires no training data — it uses the model's
reasoning ability directly. At ~1K tokens per step verification and ~5 steps per task,
the cost is ~5K tokens per task (~$0.075 at Sonnet-tier). This is 10x cheaper than
Monte Carlo rollouts and provides natural-language explanations of why a step is good
or bad.

---

## 15. Integration Architecture

All PRM components connect into a unified step-level scoring pipeline:

```
Agent step N
    │
    ├── Gate verdict (sparse, binary) ──────────────────────┐
    │                                                        │
    ├── Potential-based shaping (dense, continuous) ─────────┤
    │                                                        │
    ├── ThinkPRM verification (generative, reasoning) ──────┤
    │                                                        ▼
    │                                              StepRewardComputer
    │                                                        │
    │                                              Combined step score
    │                                                        │
    ├── If score > 0.4 AND Progress > -0.1 ──► Continue
    ├── If score < 0.2 ────────────────────► Early termination
    ├── If negative Progress × 3 turns ────► Re-plan
    └── Accumulate for DPO pair collection
```

### 15.1 Persistence

Step-level rewards are persisted alongside episodes:

```
.roko/learn/
├── episodes.jsonl              # raw agent traces
├── step-rewards.jsonl          # per-step reward annotations
│   {"episode_id": "...", "step": 3, "gate_reward": 0.0,
│    "shaped_reward": 0.15, "think_score": 0.72, "combined": 0.44}
├── dpo-pairs.jsonl             # collected preference pairs
│   {"task": "...", "preferred": "ep_123", "dispreferred": "ep_124",
│    "margin": 0.6}
└── prm-metrics.json            # PRM calibration metrics
    {"accuracy": 0.83, "calibration_error": 0.04, "samples": 1200}
```

---

## 16. Test Criteria

| Test | Property |
|---|---|
| `monte_carlo_q_value_correct` | All-passing rollouts → q_value ≈ 1.0 |
| `monte_carlo_q_value_failing` | All-failing rollouts → q_value ≈ 0.0 |
| `potential_compile_fix_positive` | Fixing compile error → positive shaped reward |
| `potential_delete_test_negative` | Deleting tests to pass → negative shaped reward |
| `potential_no_change_near_zero` | Read-only step → shaped reward ≈ 0 |
| `dpo_pair_minimum_margin` | Pairs with margin < 0.3 are filtered out |
| `shaped_reward_preserves_policy` | Total shaped reward over optimal path = 0 (Ng theorem) |
| `step_reward_combines_sources` | gate_weight × gate + shaping_weight × shaped = correct |
| `think_prm_score_bounds` | Score always in [0.0, 1.0] |
| `early_termination_on_low_promise` | Promise < 0.2 for 2 turns → termination signal |
| `replan_on_negative_progress` | Progress < -0.1 for 3 turns → replan signal |
