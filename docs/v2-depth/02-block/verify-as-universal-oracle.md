# Verify as Universal Oracle

> Depth for [02-CELL.md](../../unified/02-CELL.md). Derives the four simultaneous roles of the Verify protocol (reward function, relabeling oracle, safety boundary, economic attestation), the Goodhart-resistance of conjunctive hard + Pareto soft criteria, the Variance Inequality, Bradley-Terry aggregation, and meta-verification as a Loop.

---

## 1. Why Verify Is Load-Bearing

Verify is the protocol that connects agent computation to external reality. Every other protocol operates on internal representations -- Signals, Scores, Routes, Compositions. Verify is the only protocol that asks: *"Did the thing we produced actually work?"*

This makes Verify simultaneously the most important and most dangerous protocol:

- **Most important** because without it, the system is hallucinating with confidence. Scores, Routes, and Compositions are all internal estimates. Verify is the contact point with ground truth.
- **Most dangerous** because a broken Verify Cell poisons everything downstream. Routing learns from Verdicts. Scoring is calibrated by Verdicts. Composition adapts section weights from Verdicts. If Verify is wrong, the entire feedback loop amplifies the error.

The Verdict type (see [02-CELL.md](../../unified/02-CELL.md) S2.3) is therefore not just a pass/fail bit. It is a structured object that serves four distinct roles simultaneously, derived from its fields.

---

## 2. The Four Roles of Verdict

### 2.1 Role 1: Reward Function

**Derived from**: `Verdict.reward: f64`

The `reward` field is a continuous signal that feeds into the Route protocol's Expected Free Energy (EFE) computation. When a Route Cell selects a candidate (model, agent, tool) and the task later receives a Verdict, the `reward` value updates the candidate's `CandidateHistory.mean_reward` and `reward_variance`.

```rust
/// After a task completes and is verified, the Route Cell
/// updates its beliefs about the selected candidate.
fn update_route_beliefs(
    history: &mut CandidateHistory,
    verdict: &Verdict,
) {
    history.trials += 1;

    // Incremental mean update (Welford's algorithm)
    let delta = verdict.reward - history.mean_reward;
    history.mean_reward += delta / history.trials as f64;

    // Incremental variance update
    let delta2 = verdict.reward - history.mean_reward;
    history.reward_variance += delta * delta2;
}

/// The Route Cell uses mean_reward and reward_variance to compute
/// the EFE for each candidate:
///
///   EFE = pragmatic_value + epistemic_value - cost_term
///
/// where:
///   pragmatic_value = mean_reward (from Verdict.reward history)
///   epistemic_value = f(reward_variance) (high variance = high info gain)
///   cost_term = estimated_cost / budget_remaining
fn compute_efe(
    candidate: &RouteCandidate,
    context: &RouteContext,
) -> f64 {
    let history = candidate.history.as_ref();

    let pragmatic = history.map(|h| h.mean_reward).unwrap_or(0.5);

    // Epistemic value: higher when we are uncertain.
    // Uses the variance of the reward distribution.
    // When variance is high, trying this candidate teaches us something.
    let epistemic = history
        .map(|h| {
            if h.trials < 2 { 1.0 } // never tried = maximum epistemic value
            else {
                let var = h.reward_variance / (h.trials - 1) as f64;
                var.sqrt() // standard deviation as epistemic proxy
            }
        })
        .unwrap_or(1.0);

    // Regime modulates the balance between exploitation and exploration
    let explore_weight = match context.regime {
        Regime::Calm => 0.4,    // explore freely
        Regime::Normal => 0.2,  // balanced
        Regime::Volatile => 0.05, // mostly exploit
        Regime::Crisis => 0.0,  // pure exploitation
    };

    let cost_term = candidate.estimated_cost.to_usd()
        / context.budget_remaining.to_usd().max(0.001);

    pragmatic + explore_weight * epistemic - cost_term
}
```

**Why reward matters**: Without a continuous reward signal, routing degenerates to random selection or fixed rules. The Verdict's `reward` field is the training signal that makes routing adaptive. Over time, the system learns which models handle which task types best, because Verify provides the ground-truth feedback that no internal estimate can replace.

### 2.2 Role 2: Relabeling Oracle

**Derived from**: `Verdict.hard_criteria` + `Verdict.soft_criteria` + `Verdict.evidence`

When a task fails, the Verdict provides structured feedback for trajectory relabeling -- the system can look at the failed attempt and learn *what specifically went wrong* rather than just "it failed."

```rust
/// Relabeling: given a failed trajectory (the sequence of Signals
/// that led to a rejected output), the Verdict provides enough
/// structured information to construct a corrective example.
///
/// This is hindsight relabeling (Andrychowicz et al. 2017, HER).
/// The failed trajectory becomes a positive example for a different
/// goal: "given this input, do NOT produce output that fails these
/// specific criteria."
fn relabel_failed_trajectory(
    trajectory: &[Signal],
    verdict: &Verdict,
) -> RelabeledExample {
    // Which hard criteria failed? These are the actionable failures.
    let failed_hard: Vec<&CriterionResult> = verdict.hard_criteria
        .iter()
        .filter(|c| !c.passed)
        .collect();

    // What evidence was collected? This is the diagnostic context.
    let diagnostics: Vec<&Evidence> = verdict.evidence
        .iter()
        .filter(|e| matches!(e.kind,
            EvidenceKind::CompileOutput
            | EvidenceKind::TestResult { .. }
            | EvidenceKind::ClippyDiagnostic
            | EvidenceKind::RuntimeTrace
        ))
        .collect();

    // Where on the Pareto front did the soft criteria land?
    // This tells us what tradeoffs the agent was making.
    let soft_profile: Vec<(String, f64)> = verdict.soft_criteria
        .iter()
        .map(|c| (format!("{:?}", c.criterion), c.score))
        .collect();

    RelabeledExample {
        original_trajectory: trajectory.to_vec(),
        failed_criteria: failed_hard.into_iter().cloned().collect(),
        diagnostics: diagnostics.into_iter().cloned().collect(),
        soft_profile,
        corrective_hint: build_corrective_hint(&failed_hard, &diagnostics),
    }
}

/// Build a natural-language corrective hint from structured Verdict data.
/// This hint is injected into the system prompt for the next attempt.
fn build_corrective_hint(
    failed: &[&CriterionResult],
    evidence: &[&Evidence],
) -> String {
    let mut hint = String::from("Previous attempt failed. Specific issues:\n");

    for criterion in failed {
        hint.push_str(&format!(
            "- {:?}: score {:.2} (threshold not met)\n",
            criterion.criterion, criterion.score
        ));

        // Attach relevant evidence
        for ev_ref in &criterion.evidence_refs {
            if let Some(ev) = evidence.iter().find(|e| e.kind.matches_ref(ev_ref)) {
                hint.push_str(&format!("  Evidence: {}\n", ev.content));
            }
        }
    }

    hint
}
```

**Why relabeling matters**: Most agent systems treat failure as a binary retry signal. Structured Verdicts enable learning from failure -- the failed trajectory plus the Verdict's criterion-level detail becomes a corrective example that the Compose protocol injects into the next attempt's context.

### 2.3 Role 3: Safety Boundary

**Derived from**: `Verdict.hard_pass` + `verify_pre()` + `StreamVerdict.continue_execution`

Verify serves as the safety boundary through three enforcement points:

```rust
/// Safety boundary: three enforcement points.
///
/// 1. PRE-ACTION (verify_pre): Veto execution before it starts.
///    If hard_pass is false, the Cell is not executed.
///    This prevents unsafe actions from being attempted.
///
/// 2. MID-STREAM (verify_stream): Terminate execution mid-flight.
///    If continue_execution is false, the Cell is interrupted.
///    This catches safety violations that emerge during execution.
///
/// 3. POST-ACTION (verify_post): Reject the output after execution.
///    If hard_pass is false, the output is not persisted or used.
///    This catches violations in the produced artifact.

/// Pre-action safety check. Called before Cell execution.
async fn safety_pre_check(
    verifier: &dyn VerifyProtocol,
    input: &[Signal],
    plan: &ActionPlan,
    ctx: &VerifyContext,
) -> Result<(), CellError> {
    let verdict = verifier.verify_pre(input, plan, ctx).await?;

    if !verdict.hard_pass {
        // Find the specific safety criterion that failed
        let safety_failures: Vec<_> = verdict.hard_criteria
            .iter()
            .filter(|c| !c.passed && is_safety_criterion(&c.criterion))
            .collect();

        return Err(CellError::PreVerifyVeto { verdict });
    }

    Ok(())
}

fn is_safety_criterion(criterion: &Criterion) -> bool {
    matches!(criterion,
        Criterion::NoSecretLeak
        | Criterion::PermissionsRespected
        | Criterion::SandboxIntact
        | Criterion::InvariantPreserved { .. }
    )
}

/// Mid-stream safety check. Called periodically during execution.
async fn safety_stream_check(
    verifier: &dyn VerifyProtocol,
    partial_output: &[Signal],
    ctx: &VerifyContext,
) -> Result<bool, CellError> {
    let stream_verdict = verifier.verify_stream(partial_output, ctx).await?;

    if !stream_verdict.continue_execution {
        // Log the partial verdict for forensics
        log_safety_interruption(&stream_verdict.partial);
        return Ok(false); // signal the executor to terminate
    }

    Ok(true) // continue execution
}
```

**The safety criteria subset** (from [02-CELL.md](../../unified/02-CELL.md) S2.3):

| Criterion | Safety role |
|---|---|
| `NoSecretLeak` | Prevents credential exposure in agent output |
| `PermissionsRespected` | Enforces capability boundary (see S3.2) |
| `SandboxIntact` | Verifies execution stayed within sandbox level |
| `InvariantPreserved` | Custom invariants (e.g., "never delete production data") |

Safety criteria are always hard criteria. They cannot be traded off against quality or efficiency. A single safety failure vetoes the entire action, regardless of how good the other criteria look.

### 2.4 Role 4: Economic Attestation

**Derived from**: `Verdict` as a persisted Signal + `Evidence` chain

When a Verdict passes, it serves as an attestation of work quality. This attestation has economic consequences in the marketplace (see [15-MARKETPLACE-AND-SHARING.md](../../unified/15-MARKETPLACE-AND-SHARING.md)):

```rust
/// Economic attestation: a passing Verdict is a signed quality
/// certificate for the work that produced the verified Signal.
///
/// Reputation flows from verified work:
/// - Agents that produce more passing Verdicts gain reputation
/// - Agents that produce failing Verdicts lose reputation
/// - The Verify Cell itself has reputation (see S5: Variance Inequality)
///
/// On-chain deployments (see [18-ON-CHAIN-REGISTRIES.md])
/// can anchor Verdicts for tamper-proof attestation.
struct EconomicAttestation {
    /// The Verdict itself (persisted as a Signal in Store).
    verdict_ref: SignalRef,

    /// The work being attested (the Signal that was verified).
    work_ref: SignalRef,

    /// The agent that produced the work.
    producer: AgentId,

    /// The Verify Cell that issued the Verdict.
    verifier: CellRef,

    /// Reputation delta: positive for pass, negative for fail.
    reputation_delta: f64,

    /// Cost of verification (charged to the task budget).
    verification_cost: Cost,
}

/// Reputation update from a Verdict.
fn reputation_from_verdict(
    verdict: &Verdict,
    current_reputation: f64,
    verifier_reputation: f64,
) -> f64 {
    // Reputation change is weighted by the verifier's own reputation.
    // A verdict from a high-reputation verifier has more impact.
    let weight = verifier_reputation.clamp(0.0, 1.0);

    if verdict.hard_pass {
        // Passing: reputation increases proportional to reward and verifier weight
        current_reputation + verdict.reward * weight * 0.1
    } else {
        // Failing: reputation decreases proportional to severity
        let severity = verdict.hard_criteria
            .iter()
            .filter(|c| !c.passed)
            .count() as f64;
        current_reputation - severity * weight * 0.05
    }
}
```

---

## 3. Goodhart Resistance: Hard + Pareto Soft

The Verdict type is deliberately structured to resist Goodhart's Law ("When a measure becomes a target, it ceases to be a good measure"). The resistance comes from the structural separation of hard and soft criteria.

### 3.1 The Problem with Weighted Sums

Most evaluation systems collapse multiple criteria into a single scalar:

```text
WRONG: overall_score = 0.3 * correctness + 0.3 * quality + 0.2 * efficiency + 0.2 * safety
```

This fails because:

1. **Substitution**: An agent can compensate for low safety by boosting quality. A weighted sum says "a score of 0.95 overall is good" even if the safety component is 0.0.
2. **Manipulation**: An agent that understands the weights can game the metric by optimizing the most cheaply-improved dimension at the expense of others.
3. **Incomparability**: Quality and safety are not on the same scale. Multiplying them by weights pretends they are fungible when they are not.

### 3.2 The Conjunctive + Pareto Design

The Verdict avoids these failures by structural separation:

```rust
/// Hard criteria: conjunctive (AND). ALL must pass.
/// These are non-negotiable. You cannot compensate for a failed
/// hard criterion by excelling at a soft criterion.
///
/// This is NOT a weighted sum. It is a Boolean AND.
fn hard_pass(verdict: &Verdict) -> bool {
    verdict.hard_criteria.iter().all(|c| c.passed)
}

/// Soft criteria: Pareto front. Multi-objective, NEVER collapsed.
/// A solution dominates another if it is at least as good on all
/// soft criteria and strictly better on at least one.
///
/// Non-dominated solutions form the Pareto front.
/// The system selects from the front, not from a scalar ranking.
fn pareto_front(candidates: &[Verdict]) -> Vec<&Verdict> {
    candidates
        .iter()
        .filter(|v| {
            // v is non-dominated: no other candidate beats it on all soft criteria
            !candidates.iter().any(|other| dominates(other, v))
        })
        .collect()
}

fn dominates(a: &Verdict, b: &Verdict) -> bool {
    // a dominates b if a is >= b on all soft criteria and > on at least one
    let mut at_least_one_strictly_better = false;

    for (ca, cb) in a.soft_criteria.iter().zip(b.soft_criteria.iter()) {
        if ca.score < cb.score {
            return false; // a is worse on this criterion
        }
        if ca.score > cb.score {
            at_least_one_strictly_better = true;
        }
    }

    at_least_one_strictly_better
}
```

### 3.3 Proof Sketch: Goodhart Resistance

**Claim**: The conjunctive-hard + Pareto-soft structure resists Goodhart's Law under the following conditions:

1. Hard criteria have binary thresholds that correspond to real-world invariants (compiles or does not, tests pass or fail, sandbox intact or breached).
2. Soft criteria are multi-dimensional and never collapsed to a scalar.
3. No criterion's score is a function of another criterion's score (independence).

**Sketch**:

- **Hard criteria cannot be gamed by substitution**: Because hard criteria are conjunctive, failing any one is a hard fail. An agent that tries to substitute quality for safety still fails the safety criterion. There is no weighted-sum escape route.

- **Soft criteria resist collapse gaming**: Because soft criteria produce a Pareto front rather than a scalar ranking, an agent cannot improve its ranking by inflating one cheap dimension. Moving along the Pareto front (improving dimension A at the cost of dimension B) does not dominate the previous position -- it is a lateral move, not an improvement.

- **The combination resists the "goodest" pathology**: In a weighted-sum world, the optimizer finds the cheapest way to inflate the scalar. In the conjunctive + Pareto world, the optimizer must (a) pass all hard criteria -- no shortcuts, and (b) find a non-dominated position on the soft Pareto front -- no single-dimension inflation.

**Limitation**: This does not resist an adversary who can manipulate the *criteria themselves* (e.g., redefining what "compiles" means). The defense against that is the Variance Inequality (S4): the verifier must be harder to fool than the generator.

---

## 4. The Variance Inequality

The Variance Inequality is the central safety property of the Verify protocol:

**Statement**: The Verify Cell ensemble must have lower variance on ground-truth benchmarks than the generator Cell it is judging. Formally:

```text
Var[verifier(x) - truth(x)] < Var[generator(x) - truth(x)]
```

In words: the verifier's noise must be smaller than the generator's noise. A verifier that is noisier than the generator adds uncertainty rather than resolving it.

### 4.1 Why This Matters

Consider the alternative: a noisy verifier. If the verifier has high variance:

- Some good outputs get rejected (false negatives)
- Some bad outputs get accepted (false positives)
- The reward signal to routing is noisy, slowing learning
- Safety boundaries become unreliable

The Variance Inequality ensures that verification is *spectrally cleaner* than generation -- it resolves more uncertainty than it introduces.

### 4.2 Ensuring the Inequality

Three structural mechanisms enforce the Variance Inequality:

```rust
/// 1. DISJOINT-FAMILY PANELS
///
/// Judges are drawn from disjoint model families. Correlated errors
/// within a family cancel out when aggregated across families.
/// 3 judges from different families > 5 from the same family.
struct DisjointPanel {
    judges: Vec<JudgeCell>,
    families: Vec<ModelFamily>,
}

impl DisjointPanel {
    fn new(judges: Vec<JudgeCell>) -> Result<Self> {
        let families: Vec<_> = judges.iter().map(|j| j.family()).collect();

        // Enforce disjointness: no two judges from the same family
        let unique_families: HashSet<_> = families.iter().collect();
        if unique_families.len() < families.len() {
            return Err(PanelError::DuplicateFamily);
        }

        Ok(Self { judges, families })
    }
}

/// 2. NO SELF-JUDGMENT
///
/// A Cell never verifies its own output. The generator and verifier
/// must be different Cells, ideally from different model families.
/// This prevents the "LLM judging itself" failure mode where the
/// generator and verifier share systematic biases.
fn validate_no_self_judgment(
    generator: &CellRef,
    verifier: &CellRef,
) -> Result<()> {
    if generator.id == verifier.id {
        return Err(VerifyError::SelfJudgment {
            cell: generator.name.clone(),
        });
    }
    Ok(())
}

/// 3. CALIBRATION BENCHMARKS
///
/// Periodically, Verify Cells are tested against known ground-truth
/// benchmarks. The measured variance is compared to the generator's
/// variance on the same benchmarks. If the Inequality is violated,
/// the verifier is flagged for replacement.
struct VarianceCheck {
    verifier_variance: f64,
    generator_variance: f64,
    benchmark_size: usize,
}

impl VarianceCheck {
    fn inequality_holds(&self) -> bool {
        self.verifier_variance < self.generator_variance
    }

    fn margin(&self) -> f64 {
        self.generator_variance - self.verifier_variance
    }
}
```

### 4.3 Spectral Cleanliness

"Spectrally cleaner" means the verifier's error power spectrum has less energy in the frequencies that matter. In practice:

- The generator may produce outputs that swing wildly in quality across different task types (high variance).
- The verifier should have more consistent accuracy across the same task types (lower variance).
- The difference in variance is the "spectral cleanliness margin." A larger margin means more reliable verification.

---

## 5. Bradley-Terry Aggregation for Subjective Criteria

Objective criteria (compiles, tests pass) have unambiguous ground truth. Subjective criteria (code quality, relevance, consistency) do not. For these, Verify uses pairwise comparison aggregated via Bradley-Terry maximum likelihood estimation.

### 5.1 The Problem with Absolute Scoring

LLMs asked to rate code quality on a 1-10 scale exhibit well-known instabilities:

- **Anchoring**: the first number chosen biases subsequent ratings
- **Scale drift**: what counts as "7/10" changes across contexts
- **Position bias**: early items in a list receive different scores than late items

Pairwise comparison avoids these issues: "Is A better than B?" is a simpler, more stable judgment than "Rate A on a scale of 1-10."

### 5.2 The Bradley-Terry Model

Given candidates `{c_1, ..., c_n}`, each with a latent strength `pi_i > 0`, the probability that `c_i` beats `c_j` in a pairwise comparison is:

```text
P(c_i beats c_j) = pi_i / (pi_i + pi_j)
```

Given a set of observed pairwise comparisons, the maximum likelihood estimates of `pi_i` are found by iterative proportional fitting:

```rust
/// Bradley-Terry MLE via iterative algorithm (Zermelo 1929, Bradley-Terry 1952).
///
/// Given pairwise comparisons, estimate strength parameters.
/// Convergence is guaranteed for connected comparison graphs
/// (every candidate reachable from every other via comparison chains).
fn bradley_terry_mle(
    comparisons: &[PairwiseJudgment],
    candidates: &[SignalRef],
    max_iterations: usize,
    tolerance: f64,
) -> BradleyTerryResult {
    let n = candidates.len();
    let mut strengths = vec![1.0f64; n]; // initial: all equal

    // Index mapping
    let idx: HashMap<&SignalRef, usize> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| (c, i))
        .collect();

    // Count wins and total comparisons per candidate
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
            PairwiseWinner::Tie => {
                wins[i] += 0.5;
                wins[j] += 0.5;
            }
        }
    }

    let mut convergence = f64::MAX;

    for _ in 0..max_iterations {
        let old_strengths = strengths.clone();

        for i in 0..n {
            let denominator: f64 = (0..n)
                .filter(|&j| j != i)
                .map(|j| totals[i][j] / (strengths[i] + strengths[j]))
                .sum();

            if denominator > 0.0 {
                strengths[i] = wins[i] / denominator;
            }
        }

        // Normalize so strengths sum to n (arbitrary but stable)
        let sum: f64 = strengths.iter().sum();
        for s in &mut strengths {
            *s *= n as f64 / sum;
        }

        // Check convergence
        convergence = strengths
            .iter()
            .zip(old_strengths.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f64, f64::max);

        if convergence < tolerance {
            break;
        }
    }

    // Convert to log-scale strengths (more interpretable)
    let log_strengths: Vec<(SignalRef, f64)> = candidates
        .iter()
        .zip(strengths.iter())
        .map(|(c, &s)| (c.clone(), s.ln()))
        .collect();

    BradleyTerryResult {
        strengths: log_strengths,
        convergence,
        comparisons: comparisons.len(),
    }
}
```

### 5.3 Panel Design

For each subjective criterion, a panel of judges performs pairwise comparisons. The panel design ensures the Variance Inequality:

```rust
/// Design a judgment panel for a subjective criterion.
///
/// Rules:
/// 1. At least 3 judges from disjoint model families
/// 2. No judge from the same family as the generator
/// 3. Each pair of candidates compared at least once per judge
/// 4. Comparison graph must be connected (for BT convergence)
fn design_panel(
    candidates: &[SignalRef],
    generator_family: &ModelFamily,
    available_judges: &[JudgeCell],
) -> Result<JudgmentPlan> {
    // Filter: no judge from the generator's family
    let eligible: Vec<_> = available_judges
        .iter()
        .filter(|j| j.family() != generator_family)
        .collect();

    // Select disjoint families
    let mut selected = Vec::new();
    let mut used_families = HashSet::new();

    for judge in &eligible {
        if !used_families.contains(&judge.family()) {
            selected.push((*judge).clone());
            used_families.insert(judge.family());
        }
        if selected.len() >= 3 {
            break;
        }
    }

    if selected.len() < 3 {
        return Err(PanelError::InsufficientDiversity {
            available_families: used_families.len(),
            required: 3,
        });
    }

    // Generate comparison schedule: round-robin among candidates
    let mut comparisons = Vec::new();
    for i in 0..candidates.len() {
        for j in (i + 1)..candidates.len() {
            for judge in &selected {
                comparisons.push(ComparisonTask {
                    candidate_a: candidates[i].clone(),
                    candidate_b: candidates[j].clone(),
                    judge: judge.cell_ref(),
                });
            }
        }
    }

    Ok(JudgmentPlan { comparisons, judges: selected })
}
```

---

## 6. How Verify Learns: Predict-Publish-Correct on Its Own Verdicts

Verify Cells are learners, like all Cells (see [02-CELL.md](../../unified/02-CELL.md) S3.10). But Verify has a unique learning challenge: it is judging the system's outputs, yet it must also judge *itself*. The predict-publish-correct Loop applied to Verify works as follows:

### 6.1 The Prediction Step

Before verifying, the Verify Cell publishes a prediction about what its own Verdict will be:

```rust
/// Before verification, predict the outcome.
/// This prediction will later be compared to reality.
async fn verify_with_prediction(
    verifier: &dyn VerifyProtocol,
    input: &[Signal],
    output: &[Signal],
    ctx: &VerifyContext,
    bus: &dyn Bus,
) -> Result<Verdict> {
    // Step 1: Predict. Before running verification, estimate the result.
    let prediction = VerifyPrediction {
        block_id: verifier.id(),
        predicted_pass: estimate_pass_probability(verifier, input, output, ctx),
        predicted_reward: estimate_reward(verifier, input, output, ctx),
        predicted_duration: verifier.estimated_duration(),
    };

    // Publish prediction on Bus
    bus.publish(Pulse {
        topic: Topic::from(format!("prediction.{}", verifier.id())),
        body: Body::json(&prediction),
        ..Default::default()
    }).await?;

    // Step 2: Actually verify.
    let verdict = verifier.verify_post(input, output, ctx).await?;

    // Step 3: Publish outcome (the actual Verdict).
    bus.publish(Pulse {
        topic: Topic::from(format!("outcome.{}", verifier.id())),
        body: Body::json(&verdict),
        ..Default::default()
    }).await?;

    Ok(verdict)
}
```

### 6.2 The Calibration Step

A CalibrationPolicy (a React Cell) subscribes to both `prediction.{verifier_id}` and `outcome.{verifier_id}`, joins them by lineage, and computes the prediction error:

```rust
/// CalibrationPolicy for Verify Cells.
/// Subscribes to prediction/outcome pairs and updates the
/// Verify Cell's calibration table.
#[async_trait]
impl ReactProtocol for VerifyCalibrationPolicy {
    async fn react(
        &self,
        pulses: &[Pulse],
        ctx: &ReactContext,
    ) -> Result<ReactOutput> {
        // Join predictions and outcomes by lineage_hint
        let predictions = filter_by_topic(pulses, "prediction.*");
        let outcomes = filter_by_topic(pulses, "outcome.*");

        let mut updates = Vec::new();

        for outcome in outcomes {
            if let Some(prediction) = find_matching_prediction(
                &predictions, outcome
            ) {
                let pred: VerifyPrediction = parse_body(&prediction.body);
                let verdict: Verdict = parse_body(&outcome.body);

                // Prediction error: how wrong was the prediction?
                let pass_error = if verdict.hard_pass {
                    1.0 - pred.predicted_pass
                } else {
                    pred.predicted_pass
                };

                let reward_error = (verdict.reward - pred.predicted_reward).abs();

                updates.push(CalibrationUpdate {
                    block_id: pred.block_id.clone(),
                    prediction: serde_json::to_value(&pred).unwrap(),
                    outcome: serde_json::to_value(&verdict).unwrap(),
                    error: (pass_error + reward_error) / 2.0,
                    context_key: None,
                });
            }
        }

        // Publish calibration updates
        let mut output_pulses = Vec::new();
        for update in &updates {
            output_pulses.push(Pulse {
                topic: Topic::from(format!(
                    "calibration.{}.updated",
                    update.block_id
                )),
                body: Body::json(update),
                ..Default::default()
            });
        }

        Ok(ReactOutput {
            pulses: output_pulses,
            signals: vec![], // calibration updates are ephemeral
        })
    }

    fn subscription(&self) -> TopicFilter {
        TopicFilter::Or(
            Box::new(TopicFilter::Glob("prediction.*".into())),
            Box::new(TopicFilter::Glob("outcome.*".into())),
        )
    }
}
```

---

## 7. Meta-Verification: When the Verifier Is Wrong

The deepest question about Verify: what happens when the Verify Cell itself is wrong? Who verifies the verifier?

### 7.1 The Problem

A systematic bias in the Verify Cell propagates through the entire system:

- **False positives** (accepting bad work): routing learns that bad strategies work, composition includes bad patterns, reputation inflates.
- **False negatives** (rejecting good work): routing learns that good strategies fail, agents are penalized for correct work, throughput collapses.

### 7.2 Meta-Verification as a Loop

The solution is to treat meta-verification as a Loop (see [10-LEARNING-LOOPS.md](../../unified/10-LEARNING-LOOPS.md)) -- a Graph with a feedback edge:

```rust
/// Meta-verification Loop.
///
/// Periodically, a sample of Verdicts is re-evaluated by a different
/// Verify Cell ensemble (the "meta-panel"). Disagreements between
/// the original verdict and the meta-panel's verdict indicate
/// verifier bias.
struct MetaVerifyLoop {
    /// The primary Verify Cells being audited.
    primary_verifiers: Vec<CellRef>,

    /// The meta-panel: different Cell ensemble for re-evaluation.
    meta_panel: DisjointPanel,

    /// Sampling rate: fraction of Verdicts to re-evaluate.
    sample_rate: f64,

    /// Disagreement threshold: if meta-panel disagrees more than this,
    /// flag the primary verifier for investigation.
    disagreement_threshold: f64,
}

impl MetaVerifyLoop {
    /// Run one cycle of meta-verification.
    async fn cycle(
        &self,
        store: &dyn StoreProtocol,
        bus: &dyn Bus,
    ) -> Result<MetaVerifyReport> {
        // 1. Sample recent Verdicts from Store
        let recent_verdicts = store.query(StoreQuery {
            kinds: Some(vec![Kind::from("verdict")]),
            limit: Some(100),
            ..Default::default()
        }).await?;

        let sample_size = (recent_verdicts.len() as f64 * self.sample_rate) as usize;
        let sample = reservoir_sample(&recent_verdicts, sample_size);

        // 2. For each sampled Verdict, retrieve the original input/output
        //    and re-verify with the meta-panel
        let mut disagreements = 0;

        for verdict_signal in &sample {
            let original_verdict: Verdict = parse_body(&verdict_signal.body);
            let original_input = fetch_lineage(store, verdict_signal).await?;

            // Meta-panel re-verifies
            let meta_verdicts = self.meta_panel.verify_all(&original_input).await?;
            let meta_consensus = aggregate_verdicts(&meta_verdicts);

            // Check for disagreement
            if meta_consensus.hard_pass != original_verdict.hard_pass {
                disagreements += 1;

                // Publish disagreement for investigation
                bus.publish(Pulse {
                    topic: Topic::from("meta.verify.disagreement"),
                    body: Body::json(&MetaDisagreement {
                        original: original_verdict.clone(),
                        meta: meta_consensus.clone(),
                        signal_ref: verdict_signal.id.clone(),
                    }),
                    ..Default::default()
                }).await?;
            }
        }

        let disagreement_rate = disagreements as f64 / sample_size as f64;

        Ok(MetaVerifyReport {
            sample_size,
            disagreements,
            disagreement_rate,
            threshold_exceeded: disagreement_rate > self.disagreement_threshold,
        })
    }
}
```

### 7.3 The Regress Problem

Meta-verification raises the obvious regress: who verifies the meta-panel? The answer is that the regress terminates at ground truth:

1. **Level 0**: Generator produces output.
2. **Level 1**: Primary Verify Cell checks output against objective criteria (compiles, tests pass). These criteria have unambiguous ground truth. No regress needed.
3. **Level 2**: Meta-panel spot-checks primary Verdicts. Disagreements on objective criteria indicate verifier bugs (not judgment calls). Disagreements on subjective criteria are resolved by human review (Level 3).
4. **Level 3**: Human review. The final oracle. Used sparingly (expensive, slow) but provides the ground truth that calibrates all lower levels.

The regress is finite because objective criteria have computable ground truth (run the compiler, run the tests), and subjective criteria terminate at human judgment.

---

## 8. The Four-Role Verdict Dispatch

Putting it all together, here is the dispatch logic for consuming a Verdict in its four roles simultaneously:

```rust
/// Dispatch a Verdict into its four simultaneous roles.
///
/// This is called after every verification. The Verdict feeds:
/// 1. Route: update candidate beliefs (reward function)
/// 2. Learn: relabel failed trajectories (oracle)
/// 3. Safety: enforce boundaries (if pre-check veto)
/// 4. Reputation: update economic attestation
async fn dispatch_verdict(
    verdict: &Verdict,
    task: &TaskContext,
    store: &dyn StoreProtocol,
    bus: &dyn Bus,
) -> Result<()> {
    // Role 1: Reward function -> Route protocol
    // Publish verdict for routing feedback
    bus.publish(Pulse {
        topic: Topic::from(if verdict.hard_pass {
            "verify.verdict.passed"
        } else {
            "verify.verdict.failed"
        }),
        body: Body::json(&RouteRewardPayload {
            task_type: task.task_type.clone(),
            candidate: task.selected_candidate.clone(),
            reward: verdict.reward,
            regime: task.regime,
        }),
        ..Default::default()
    }).await?;

    // Role 2: Relabeling oracle -> Learning subsystem
    if !verdict.hard_pass {
        let relabeled = relabel_failed_trajectory(
            &task.trajectory,
            verdict,
        );
        // Persist the relabeled example for future Compose context
        let signal = Signal::from_relabeled(&relabeled);
        store.put(signal).await?;
    }

    // Role 3: Safety boundary (already enforced at pre/stream/post)
    // Log for audit trail
    if verdict.hard_criteria.iter().any(|c| !c.passed && is_safety_criterion(&c.criterion)) {
        bus.publish(Pulse {
            topic: Topic::from("safety.verdict.violation"),
            body: Body::json(&SafetyViolation {
                task: task.task_id.clone(),
                failed_criteria: verdict.hard_criteria
                    .iter()
                    .filter(|c| !c.passed)
                    .cloned()
                    .collect(),
            }),
            ..Default::default()
        }).await?;
    }

    // Role 4: Economic attestation -> Reputation
    let attestation = EconomicAttestation {
        verdict_ref: store.put(Signal::from_verdict(verdict)).await?,
        work_ref: task.output_ref.clone(),
        producer: task.agent_id.clone(),
        verifier: task.verifier_ref.clone(),
        reputation_delta: if verdict.hard_pass {
            verdict.reward * 0.1
        } else {
            -0.05 * verdict.hard_criteria.iter().filter(|c| !c.passed).count() as f64
        },
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

---

## What This Enables

1. **Closed-loop learning**: Verify provides the reward signal that makes routing, scoring, and composition adaptive. Without Verify, the system is open-loop: it generates but never checks. With Verify, every generation improves the next.

2. **Structured failure analysis**: Relabeling turns failures into learning opportunities. Instead of "retry and hope," the system injects specific corrective hints from Verdict evidence. This is especially powerful for code tasks where compile errors, test failures, and clippy diagnostics are machine-parseable.

3. **Safety boundaries that cannot be traded off**: Hard criteria are conjunctive. No amount of quality makes up for a safety violation. This is enforced at the type level (hard_pass is separate from soft criteria), not as a tuning parameter.

4. **Self-improving verification**: Predict-publish-correct applied to Verify means the system tracks how often its verifiers are right. Meta-verification catches systematic bias before it poisons downstream learning.

5. **Economic trust**: Verdicts as attestations create a portable reputation system. Agents, Cells, and models build track records that are anchored in verified outcomes, not self-reported metrics.

---

## Feedback Loops

- **Adaptive gate thresholds**: The hard-criterion thresholds are not fixed. They are updated via EMA on historical pass rates. If a criterion passes too easily (>95% pass rate), the threshold tightens. If it fails too often (<50% pass rate), the system investigates (but does not automatically loosen -- that would reward declining quality).

- **Verifier rotation**: When meta-verification detects bias, the primary verifier is rotated out and replaced with a different Cell from the meta-panel. The rotated-out verifier enters a recalibration period where its Verdicts are shadowed (published but not used for routing) until its disagreement rate drops.

- **Criterion evolution**: New criteria can be added to the Criterion enum as the system encounters new failure modes. For example, if agents start producing code that compiles but has infinite loops, a `Terminates` criterion can be added without changing the Verdict structure.

---

## Open Questions

1. **Incentive alignment under adversarial generators**: The current model assumes generators are cooperative (they try to produce good output). In a marketplace setting, a generator might try to game the verifier. How does the Variance Inequality hold against an adversarial generator that specifically targets the verifier's blind spots? The disjoint-family panel helps, but a determined adversary might find correlated weaknesses across families.

2. **Subjective criteria ground truth**: Bradley-Terry aggregation gives a ranking, but rankings are relative. When the entire candidate set is poor, BT will still rank them and declare a winner. Should there be an absolute quality floor below which BT results are discarded? This would require a hybrid system: absolute thresholds for a minimum quality bar, BT for ranking above the bar.

3. **Verification cost scaling**: As the number of criteria and judges grows, verification cost grows quadratically (O(candidates^2 * judges * criteria) for BT). Can we prune the comparison graph (e.g., Swiss-system tournament pairing) while maintaining BT convergence guarantees?

4. **Causal attribution from Verdicts**: When a Verdict fails with multiple failed criteria, which criterion was the root cause? The current structure lists all failed criteria but does not model causality between them (e.g., "compile failure caused test failure"). A causal DAG over criteria would enable more precise relabeling, but adds complexity to the Verdict type.
