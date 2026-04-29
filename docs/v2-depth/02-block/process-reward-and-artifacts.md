# Process Reward Models and Artifacts

> Depth for [02-CELL.md](../../unified/02-CELL.md). Process Reward Models as fine-grained Verify Cells that score intermediate reasoning steps. Artifact store for gate evidence. PRMs as a Score+Verify composition producing per-step reward Signals.

---

## 1. Why Process Rewards

Standard verification is binary: code compiles or it does not, tests pass or they fail. Process Reward Models (PRMs) add granularity by scoring the agent's **process** -- each tool call, each file edit, each reasoning turn -- not just the final output.

This matters because:

- An agent 90% of the way to a working solution is more promising than one that made no progress
- Intermediate progress Signals enable early intervention (abandon a failing approach before consuming the full retry budget)
- Per-step rewards provide **10x richer training signal** than final pass/fail outcomes (AgentPRM, arXiv:2502.10325)

Binary verification operates at one timescale: attempt complete, check result. Process rewards operate at a faster timescale: per-turn within an attempt, per-step within a turn.

---

## 2. The Two Dimensions: Promise and Progress

Roko's PRM tracks two orthogonal Signals per agent execution.

### 2.1 Promise

**Promise** estimates how likely the current execution is to eventually succeed, given work done so far. It answers: "Is this approach heading somewhere good?"

| Indicator | Signal source | High Promise | Low Promise |
|---|---|---|---|
| Compile status | Rung 0 verdicts | Making compile-passing edits | Compile error count increasing |
| File targeting | Tool call metadata | Targeting right files | Targeting unrelated files |
| Edit pattern | Historical episode similarity | Matches successful patterns | Matches failed patterns |
| Loop detection | Tool call history | Diverse actions | Same edit repeatedly |

**Intervention on low Promise**: Early termination of the current attempt before the full retry budget is consumed. Better to start fresh with a different prompt or model than to continue down a failing path.

### 2.2 Progress

**Progress** measures whether the agent is advancing across attempts. It answers: "Is this attempt doing better than the previous one?"

| Indicator | Positive Progress | Negative Progress |
|---|---|---|
| Rung advancement | Higher rung than previous attempt | Same or lower rung |
| Error count | Fewer compile errors | Same or more errors |
| Test pass rate | More tests passing | Fewer tests passing |

**Intervention on negative Progress**: Across multiple attempts -> re-planning. The current plan may be fundamentally flawed. Generate a new plan rather than retrying the same approach.

### 2.3 The Score Functions

**Promise** is a weighted combination:

```
Promise(attempt) = 0.4 * rung_fraction
                 + 0.3 * test_pass_rate
                 + 0.2 * error_trend
                 + 0.1 * tool_efficiency
```

Where:
- `rung_fraction` = highest_rung_passed / total_rungs (0.0 to 1.0)
- `test_pass_rate` = tests_passed / total_tests (0.5 if no test gate)
- `error_trend` = 1.0 decreasing, 0.5 stable, 0.0 increasing
- `tool_efficiency` = useful_tool_calls / total_tool_calls

| Promise | Interpretation | Action |
|---|---|---|
| > 0.8 | High: likely to succeed | Continue, possibly reduce retries |
| 0.4-0.8 | Moderate: uncertain | Continue with standard retries |
| 0.2-0.4 | Low: probably failing | Consider early termination |
| < 0.2 | Very low: almost certainly failing | Terminate, try different approach |

**Progress** compares current to previous attempt:

```
Progress(attempt_n) = delta_rung + delta_test_rate + delta_error_count
```

| Progress | Interpretation | Action |
|---|---|---|
| > 0.1 | Advancing | Continue, approach is working |
| -0.1 to 0.1 | Stalling | Escalate complexity, adjust prompt |
| < -0.1 | Regressing | Stop retrying, re-plan |

---

## 3. Data Sources from the Gate Infrastructure

Process rewards consume data already produced by the Verify pipeline. No new data collection is required -- PRMs are a **Lens** (read-only observation) on existing gate data.

### 3.1 Per-Turn Tool Call Metadata

```rust
pub struct ToolCallMeta {
    pub tool_name: String,
    pub duration_ms: u64,
    pub result_tokens: u64,
    pub succeeded: bool,
    pub advanced_task: bool,   // computed post-hoc: was this output referenced in the solution?
    pub was_redundant: bool,   // was this call unnecessary?
    pub error_category: Option<String>,
}
```

`advanced_task` is computed by checking if the tool call's output was referenced in the final solution (the agent used the information). If not, it was wasted work. This feeds the `tool_efficiency` component of Promise.

### 3.2 Gate Verdicts with Rung Information

Each pipeline execution produces:
- Which rung was reached (highest passing rung) -> `rung_fraction`
- Test counts (passed/failed/ignored) -> `test_pass_rate`
- Error digest (machine-parseable) -> `error_trend`

### 3.3 DiffGate Analysis

- How many substantive lines were added
- Whether additions are vacuous (`todo!()` / `unimplemented!()`)
- Code completeness trajectory

### 3.4 Ratchet State

Changes in `GateRatchet` across attempts reveal Progress:
- Ratchet advances from rung 1 to 2: positive Progress
- Ratchet stays at rung 1 after 3 attempts: stalling
- Ratchet would regress: negative Progress (blocked by ratchet)

---

## 4. Three Feedback Timescales

Process rewards create a multi-timescale control system. Each timescale addresses a different kind of failure.

```
Fast (per-turn):   Promise score -> continue or terminate this attempt
Medium (per-attempt): Progress score -> adjust routing and prompts
Slow (across plans): Trend across plans -> update model preferences and templates
```

### 4.1 Within-Attempt Optimization

```
Agent turn N
    |
    v
Gate pipeline (if code-modifying step)
    |
    v
Compute Promise(N)
    |
    +-- Promise > 0.4 AND Progress > -0.1 -> continue to turn N+1
    +-- Promise < 0.2 -> early termination
    +-- Progress < -0.1 for 3 turns -> re-plan
```

This is faster than the retry loop (which operates across attempts) and much faster than escalation (which operates across failed attempts).

### 4.2 Cybernetic Signal Routing

| Signal | From | To | Effect |
|---|---|---|---|
| Promise | Gate + ToolCallMeta | Conductor | Early termination on low Promise |
| Progress | Gate (across attempts) | Route Cell | Model re-selection on stalling |
| Progress | Gate (across attempts) | Compose protocol | Prompt adjustment on stalling |
| Promise x Progress | Combined | React Cell | Re-plan on persistent low signals |

---

## 5. Self-Supervised PRM Training

Roko has a unique advantage over academic PRM systems: it generates its own step-level training labels. The gate pipeline is a deterministic oracle. Every intermediate artifact can be verified, producing automated labels without human annotation.

### 5.1 The Self-Supervision Loop

```
Agent execution trace:
  step_1: read file -> artifact_1 (no code change)
  step_2: edit file -> artifact_2 (code changed)
  step_3: edit file -> artifact_3 (code changed)
  step_4: run tests -> artifact_4 (no code change)
  step_5: fix test  -> artifact_5 (code changed)

Gate verification of each intermediate artifact:
  artifact_2: compile PASS, test FAIL -> partial credit
  artifact_3: compile PASS, test 8/10 PASS -> more credit
  artifact_5: compile PASS, test 10/10 PASS -> full credit

Step-level labels:
  step_1: 0.3 (read, neutral but necessary)
  step_2: 0.5 (compiles but tests fail)
  step_3: 0.7 (more tests pass)
  step_4: 0.3 (information-gathering, no code progress)
  step_5: 1.0 (all tests pass)
```

### 5.2 Monte Carlo Step-Level Q-Values

For richer labels than binary gate outcomes, Monte Carlo rollouts estimate the probability that continuing from a given state leads to eventual success:

```rust
pub struct MonteCarloStepLabeler {
    pub num_rollouts: usize,      // default: 8
    pub max_rollout_turns: usize, // default: 10
    pub gate_pipeline: GatePipeline,
    pub agent: Box<dyn AgentBackend>,
}

// For each intermediate step:
//   successes = 0
//   for k in 0..num_rollouts:
//       continuation = agent.continue_from(state, max_turns)
//       verdict = gate_pipeline.verify(continuation.artifact)
//       if verdict.passed: successes += 1
//   q_value = successes / num_rollouts
//   label = "correct" if q_value > 0.5 else "incorrect"
```

**Cost analysis**: 8 rollouts x ~5 code-modifying steps = 40 agent turns + 40 gate evaluations. At Haiku-tier costs (~$0.001/turn), labeling costs ~$0.04 per task. For 100 tasks, $4 produces ~500 step-level labels -- orders of magnitude cheaper than the 800K human labels in PRM800K (Lightman et al. 2023).

### 5.3 FoVer: Formally Verified Labels

For code changes expressible as logical assertions, formal verification (Z3, Isabelle, or Prusti for Rust) provides perfect step-level labels (Kamoi et al., arXiv:2505.15960, 2025). Returns Verified (correct), Counterexample (incorrect), or Unknown (no label).

In practice, ~10-20% of code changes in a typed language like Rust can be formally checked (type constraints, trait bounds, lifetime invariants). The rest use Monte Carlo.

---

## 6. Reward Shaping for Dense Feedback

Raw gate verdicts are sparse: most intermediate steps produce no verdict (no code change = nothing to verify). Reward shaping fills the gaps with dense signals that guide the agent without changing the optimal policy.

### 6.1 Potential-Based Reward Shaping

Per Ng et al. (ICML 1999), potential-based shaping preserves the optimal policy:

```
R'(s, a, s') = R(s, a, s') + gamma * Phi(s') - Phi(s)
```

where `Phi` is a potential function over agent states:

```rust
pub struct GatePotential {
    pub compile_weight: f64,      // default: 0.4
    pub test_rate_weight: f64,    // default: 0.3
    pub lint_weight: f64,         // default: 0.1
    pub completeness_weight: f64, // default: 0.2
}

impl GatePotential {
    pub fn phi(&self, state: &IntermediateState) -> f64 {
        let compile = if state.compiles { 1.0 } else { 0.0 };
        let test_rate = state.tests_passed as f64 / state.tests_total.max(1) as f64;
        let lint = if state.lint_clean { 1.0 } else { 0.5 };
        let completeness = 1.0 - state.stub_fraction();

        self.compile_weight * compile
            + self.test_rate_weight * test_rate
            + self.lint_weight * lint
            + self.completeness_weight * completeness
    }

    pub fn shaped_reward(&self, prev: &IntermediateState,
                         next: &IntermediateState, discount: f64) -> f64 {
        discount * self.phi(next) - self.phi(prev)
    }
}
```

### 6.2 Shaping Signal Interpretation

```
Step: agent adds use statement (fixes compile error)
  Phi(prev)=0.0, Phi(next)=0.4 -> shaped reward +0.396 (progress)

Step: agent deletes a test (makes test suite pass vacuously)
  Phi(prev)=0.7, Phi(next)=0.6 -> shaped reward -0.106 (regression)

Step: agent reads a file (no code change)
  Phi(prev)=0.5, Phi(next)=0.5 -> shaped reward -0.005 (encourages efficiency)
```

The shaping naturally penalizes vacuous changes (deleting tests to pass) and rewards genuine progress (fixing errors to pass), without hand-coded rules. This connects directly to the DiffGate's vacuous-implementation rejection (see [verify-cells-and-pipeline.md](verify-cells-and-pipeline.md) S1.4).

### 6.3 Combined Step Reward

```rust
pub struct StepRewardComputer {
    pub gate_potential: GatePotential,
    pub discount: f64,          // default: 0.99
    pub gate_weight: f64,       // default: 1.0 (sparse gate reward)
    pub shaping_weight: f64,    // default: 0.5 (shaped reward)
}

impl StepRewardComputer {
    pub fn compute(&self, prev: &IntermediateState,
                   next: &IntermediateState,
                   gate_verdict: Option<&Verdict>) -> f64 {
        let gate_reward = gate_verdict
            .map(|v| if v.passed { 1.0 } else { -0.5 })
            .unwrap_or(0.0);
        let shaped = self.gate_potential.shaped_reward(prev, next, self.discount);
        self.gate_weight * gate_reward + self.shaping_weight * shaped
    }
}
```

---

## 7. ThinkPRM: Generative Process Verification

Rather than training a discriminative PRM (which requires labeled data), ThinkPRM (Mukhal et al., arXiv:2504.16828, 2025) uses a generative approach: ask a reasoning model to verify each step by thinking through it.

```rust
pub struct ThinkPrm {
    pub model: String,                  // prefer Sonnet+ for reliable reasoning
    pub max_reasoning_tokens: usize,    // default: 1024
    pub threshold: f64,                 // default: 0.5
}

// Prompt structure:
// "You are verifying step {i} of an agent's execution.
//  Task: {task_description}
//  Previous steps: {step_1..step_{i-1}}
//  Current step: {step_i}
//  Gate results so far: {gate_verdicts}
//
//  Score the step from 0.0 (definitely wrong) to 1.0 (definitely correct)."
```

**Cost-effectiveness**: ~1K tokens per step verification, ~5 steps per task = ~5K tokens per task (~$0.075 at Sonnet-tier). This is 10x cheaper than Monte Carlo rollouts and provides natural-language explanations of why a step is good or bad.

ThinkPRM is a Score+Verify composition: it scores each step (Score protocol) using reasoning-based verification (Verify protocol) and produces per-step reward Signals. This makes it a Cell conforming to both protocols.

---

## 8. RLHF Alternatives for Agent Improvement

Gate verdicts provide a natural reward signal. The question is how to convert these rewards into improved agent behavior across future tasks.

### 8.1 DPO (Direct Preference Optimization)

When two agents attempt the same task and one passes gates while the other fails, this creates a natural preference pair. DPO (Rafailov et al., NeurIPS 2023) directly optimizes from these pairs without explicit reward model training.

```rust
pub struct DpoTrainingPair {
    pub task_spec: String,
    pub preferred: AgentTrace,      // passed all gates
    pub dispreferred: AgentTrace,   // failed gates
    pub margin: f64,                // how much better preferred was
}

pub struct DpoConfig {
    pub beta: f64,           // temperature (default: 0.1)
    pub min_margin: f64,     // pairs below this are too similar (default: 0.3)
    pub batch_size: usize,   // trigger training (default: 128)
    pub pairs_path: PathBuf,
}
```

DPO implicitly defines a reward model: `r(x,y) = beta * log(pi_theta(y|x) / pi_ref(y|x))`, which can be extracted for process reward scoring without separate PRM training.

### 8.2 RLAIF (AI Feedback)

Use a judge model to generate preferences from gate verdicts and code quality analysis. Cheaper than DPO because the judge evaluates existing traces rather than generating rollouts.

### 8.3 Constitutional AI for Safety Gates

Safety-related gate failures (security vulnerabilities, race conditions, resource leaks) use a Constitutional AI approach: self-critique against safety principles before the gate pipeline runs.

```rust
pub const SAFETY_CONSTITUTION: &[&str] = &[
    "Generated code must not introduce SQL injection, XSS, or command injection.",
    "Generated code must not disable authentication or authorization checks.",
    "Generated code must not expose secrets in logs, error messages, or comments.",
    "Generated code must not introduce race conditions or deadlocks.",
    "Generated code must not create unbounded resource allocation.",
];
```

The critique step happens BEFORE the gate pipeline, catching safety issues that static gates (which check syntax/tests, not security semantics) might miss.

---

## 9. Integration Architecture

All PRM components connect into a unified step-level scoring pipeline:

```
Agent step N
    |
    +-- Gate verdict (sparse, binary) ----------------+
    |                                                  |
    +-- Potential-based shaping (dense, continuous) ---+
    |                                                  |
    +-- ThinkPRM verification (generative, reasoning) -+
    |                                                  v
    |                                         StepRewardComputer
    |                                                  |
    |                                         Combined step score
    |                                                  |
    +-- score > 0.4 AND Progress > -0.1 ------> Continue
    +-- score < 0.2 --------------------------> Early termination
    +-- negative Progress x 3 turns ----------> Re-plan
    +-- Accumulate for DPO pair collection
```

### 9.1 Persistence

Step-level rewards persist alongside episodes:

```
.roko/learn/
  episodes.jsonl              # raw agent traces
  step-rewards.jsonl          # per-step reward annotations
  dpo-pairs.jsonl             # collected preference pairs
  prm-metrics.json            # PRM calibration metrics
```

---

## 10. The Artifact Store as Evidence Chain

The ArtifactStore (see [verify-cells-and-pipeline.md](verify-cells-and-pipeline.md) S4) provides the evidence chain for process rewards. Every intermediate artifact can be retrieved by BLAKE3 hash and re-verified:

```
step_3: edit file -> artifact_3 (hash: ab3f8c...)
    |
    +-- ArtifactStore.get(ab3f8c...) -> exact bytes
    +-- GatePipeline.verify(artifact_3) -> Verdict (reproducible)
    +-- StepRewardComputer.compute(state_2, state_3, verdict) -> reward
```

This makes process rewards auditable. Any step reward can be traced back to: the exact artifact (content-addressed), the exact gate verdicts (reproducible), and the exact reward computation (deterministic function of inputs).

For forensic replay (see [02-CELL.md](../../unified/02-CELL.md) S2.3 role 2: relabeling oracle), the combination of ArtifactStore + step rewards + DPO pairs provides a complete record of what happened, why each step scored as it did, and how this information feeds back into future agent behavior.

---

## What This Enables

1. **Per-step learning signal**: Instead of waiting for a final binary verdict, the system gets continuous feedback every turn. This is the difference between "retry and hope" and "know specifically what is working and what is not."

2. **Early termination**: Low Promise triggers termination before the retry budget is exhausted. For a task with 5 retries at 15 minutes each, catching a hopeless approach after 2 turns saves up to 73 minutes.

3. **Structured re-planning**: Negative Progress across attempts provides evidence that the current plan is flawed, not just unlucky. The evidence (specific rung failures, error trends) informs the re-planning agent about what to change.

4. **Self-supervised training data**: The gate pipeline generates step-level labels automatically. No human annotation needed. 100 tasks at $0.04 each produces 500 labels for $4 -- competitive with human annotation at orders of magnitude less cost.

5. **Dense reward for efficient behavior**: Potential-based shaping penalizes vacuous changes and rewards genuine progress without hand-coded rules. The discount factor encourages efficiency -- idle steps have slightly negative reward.

---

## Feedback Loops

- **Step reward -> Early termination**: Low combined score for 2+ consecutive turns -> terminate attempt. Saves compute on hopeless approaches.
- **DPO pairs -> Model improvement**: Accumulated preference pairs from gate outcomes feed offline training. Models improve on exactly the kinds of tasks they encounter in production.
- **ThinkPRM reasoning -> Agent context**: The natural-language reasoning from ThinkPRM can be injected into retry prompts, giving the agent not just "your step scored 0.3" but "your step scored 0.3 because the edit targets a module that is unrelated to the failing test."
- **Shaped reward -> Routing**: Steps with consistently negative shaped reward for a given model/task-type combination update the Route Cell's beliefs about that candidate.
- **Artifact store -> Replay**: Content-addressed artifacts enable exact replay of any step's verification, providing ground-truth calibration for the PRM.

---

## Open Questions

1. **Monte Carlo rollout cost**: 8 rollouts per step x 5 steps = 40 additional agent turns per task. For expensive models (Opus-tier), this cost may exceed the benefit. Should rollout count adapt to model cost? Haiku: 16 rollouts, Opus: 2 rollouts.

2. **ThinkPRM vs. gate verdicts**: When ThinkPRM and gate verdicts disagree (ThinkPRM says step is correct, gate says it fails), which should take precedence? The Variance Inequality (see [verify-as-universal-oracle.md](verify-as-universal-oracle.md)) suggests the gate (deterministic oracle) should dominate the ThinkPRM (model-based, noisier). But ThinkPRM may catch semantic issues that gates miss.

3. **Reward hacking via shaping**: The shaped reward uses `completeness = 1 - stub_fraction`. An agent that writes large blocks of mostly-correct but partially-incorrect code gets high completeness reward before gate failure. Should the completeness signal be gated by compile status (only count completeness for code that compiles)?

4. **DPO pair quality**: Not all preference pairs are equally informative. Two attempts that both fail at Rung 2 with similar test failures produce a low-margin pair that may be noise rather than signal. The `min_margin = 0.3` filter helps, but is this threshold well-calibrated? Should it adapt to the distribution of observed margins?

5. **Constitutional AI integration point**: The safety constitution self-critique currently runs before the gate pipeline. Should it also run after, catching safety issues in the agent's final output that emerged during the verification loop itself?
