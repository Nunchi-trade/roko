# Missing Loops and Calibration

> Depth for [07-LEARNING.md](../../unified/07-LEARNING.md). The eight cybernetic feedback Loops that connect learning subsystems, their cross-Loop interaction matrix, collective calibration via the 31.6x heuristic (CLT-inspired upper bound), and predictive foraging as a meta-Loop that turns prediction error into a learning signal.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, Bus), [02-CELL](../../unified/02-CELL.md) (Cell, React, Observe, Verify, Route, Compose protocols), [04-EXECUTION](../../unified/04-EXECUTION.md) (Loop specialization), [07-LEARNING](../../unified/07-LEARNING.md) (L1-L4 taxonomy, predict-publish-correct), [autocatalytic-compounding.md](autocatalytic-compounding.md) (Kauffman condition), [c-factor-as-lens.md](c-factor-as-lens.md) (C-factor computation)

**Source docs**: [13-8-missing-feedback-loops.md](../../docs/05-learning/13-8-missing-feedback-loops.md), [15-collective-calibration-31x.md](../../docs/05-learning/15-collective-calibration-31x.md), [16-predictive-foraging.md](../../docs/05-learning/16-predictive-foraging.md)

---

## 1. Eight Feedback Loops

The learning system was built in layers: episodes first, then patterns, then bandits, then routing. Each layer works independently, but they did not originally talk to each other. The eight missing Loops are the inter-layer connections that close the cybernetic circuit -- Signals flowing from one subsystem's output to another subsystem's input, creating self-regulating behavior.

Each Loop connects a **source** (where the Signal originates) to a **target** (where it should influence decisions) through a **transform** (how the Signal becomes action). In unified vocabulary, each Loop is a Graph with a feedback edge, containing at minimum an Observe Cell (source), a transform Cell, and a React Cell (target injection).

```
Eight Feedback Loops
====================

1. Health   -> Routing      Provider circuit breaker -> candidate filter
2. Conductor -> Routing     System load signals -> routing cost bias
3. Section  -> Scaffold     Section effectiveness -> prompt weights
4. Failure  -> Replanning   Gate failure patterns -> plan revision
5. Skills   -> Prompts      Skill library matches -> prompt injection
6. Cost     -> Routing      Budget pressure -> model tier constraint
7. Latency  -> Reward       Response latency -> bandit reward signal
8. Experiments -> Static    Experiment winners -> static routing table
```

### Loop 1: Health -> Routing

```rust
/// Observe Cell: reads ProviderHealthRegistry circuit breaker state.
/// React Cell: filters CascadeRouter candidate set.
///
/// When a provider's circuit breaker is Open, all that provider's models
/// are excluded from the candidate set before scoring.
fn health_to_routing(
    provider: &str,
    registry: &ProviderHealthRegistry,
) -> bool {
    // < 1ms: HashMap lookup
    matches!(registry.state(provider), CircuitState::Closed | CircuitState::HalfOpen)
}
```

**Status**: Wired. The cascade router calls `is_available()` during candidate scoring.
**Failure mode if broken**: Routes to degraded provider -> timeouts -> wasted budget -> cascading failures.

### Loop 2: Conductor -> Routing

```rust
/// Observe Cell: reads system load snapshot (CPU, memory, active agents, queue depth).
/// React Cell: biases CascadeRouter toward cheaper models under load.
fn conductor_to_routing(
    load: &SystemLoadSnapshot,
    max_agents: u32,
) -> RoutingBias {
    let utilization = load.active_agents as f64 / max_agents as f64;
    if utilization > 0.8 {
        RoutingBias::PreferCheaper { cost_weight_multiplier: 1.5 }
    } else if utilization < 0.3 {
        RoutingBias::AllowExpensive { quality_weight_multiplier: 1.2 }
    } else {
        RoutingBias::Neutral
    }
}
```

**Status**: Wired (pressure heuristic). `RoutingContext` carries conductor pressure derived from active agent count and queue depth.
**Failure mode if broken**: Expensive models during high load -> resource exhaustion -> spawn failures.

### Loop 3: Section -> Scaffold

This is the highest-leverage self-improvement Loop. It tracks which prompt sections correlate with gate success and adjusts section weights accordingly.

```rust
/// Observe Cell: reads PromptSectionMeta + gate outcomes from efficiency events.
/// Score Cell: computes conditional pass rate per section.
/// React Cell: adjusts section priority weights in the Compose pipeline.
pub struct SectionEffectivenessTracker {
    stats: HashMap<String, SectionStats>,
}

pub struct SectionStats {
    pub included_count: u32,
    pub included_pass_count: u32,
    pub excluded_count: u32,
    pub excluded_pass_count: u32,
}

impl SectionStats {
    /// Positive = section helps. Negative = section hurts.
    pub fn effectiveness_delta(&self) -> f64 {
        let inc_rate = self.included_pass_count as f64
            / self.included_count.max(1) as f64;
        let exc_rate = self.excluded_pass_count as f64
            / self.excluded_count.max(1) as f64;
        inc_rate - exc_rate
    }
}
```

**Status**: Wired for live orchestration path. Section inclusion/drop metadata flows into efficiency events; `LearningRuntime` persists a section-effectiveness registry.
**Impact**: Adaptive context assembly can reduce prompt size by 30-50% while improving pass rates. The highest-leverage improvement available.

### Loop 4: Failure -> Replanning

```rust
/// Observe Cell: watches consecutive gate failures for a task.
/// Score Cell: analyzes failure patterns (same error? different errors?).
/// React Cell: triggers replanning -- decompose, change approach, or escalate.
pub enum FailureRecommendation {
    /// Same error with multiple models -> fundamental approach problem.
    Decompose { suggested_split: Vec<String> },
    /// Varied errors -> approach needs revision.
    ChangeApproach { reason: String },
    /// Many failures, high cost -> escalate to human.
    HumanReview { context: String },
    /// Task may be impossible given current capabilities.
    Skip { reason: String },
}
```

**Status**: Wired for orchestrator path. Gate failures trigger strategy-specific replan flows.
**Failure mode if broken**: Retries same failing task indefinitely -> budget burn.

### Loop 5: Skills -> Prompts

```rust
/// Observe Cell: queries SkillLibrary by task file paths, tags, HDC similarity.
/// Compose Cell: injects matched skills into agent prompt (priority 3, max 500 tokens).
///
/// The 100th modification to a crate is dramatically cheaper than the 1st
/// because the skill library has accumulated the crate's patterns.
fn skills_to_prompt(
    task: &TaskDef,
    library: &SkillLibrary,
    max_skills: usize,     // 3
    min_confidence: f64,   // 0.50
    max_tokens: usize,     // 500
) -> Vec<SkillInjection> {
    let mut matches = library.search_by_task(task);
    matches.retain(|m| m.confidence >= min_confidence);
    matches.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
    matches.truncate(max_skills);
    // Budget-aware: stop adding when token budget exhausted
    matches.into_iter()
        .scan(max_tokens, |budget, skill| {
            if skill.tokens <= *budget {
                *budget -= skill.tokens;
                Some(skill.into())
            } else {
                None
            }
        })
        .collect()
}
```

**Status**: Wired for live prompt composition. Skills rendered into a dedicated `skill-library` prompt section.

### Loop 6: Cost -> Routing

```rust
/// Observe Cell: tracks cumulative cost (per-task, per-session, per-day).
/// Verify Cell: compares against budget limits.
/// React Cell: constrains CascadeRouter to cheaper models when budget is tight.
pub enum BudgetRoutingAction {
    Continue,
    Downgrade { max_cost_per_m: f64 },  // only models cheaper than this
    Block,
    HardStop,
}

fn cost_to_routing(task_pct: f64, session_pct: f64, day_pct: f64) -> BudgetRoutingAction {
    let max_pct = task_pct.max(session_pct).max(day_pct);
    match max_pct {
        p if p >= 1.0  => BudgetRoutingAction::HardStop,
        p if p >= 0.95 => BudgetRoutingAction::Block,
        p if p >= 0.80 => BudgetRoutingAction::Downgrade { max_cost_per_m: 0.50 },
        _              => BudgetRoutingAction::Continue,
    }
}
```

**Status**: Wired. `BudgetGuardrail` checks spend before dispatch; can block or force cheaper tier.

### Loop 7: Latency -> Reward

```rust
/// Observe Cell: reads LatencyRegistry (EWMA per model, P50/P95/P99).
/// Score Cell: computes latency reward component relative to SLA.
/// React Cell: incorporates into composite bandit reward.
fn latency_to_reward(ewma_ms: f64, sla_ms: f64) -> f64 {
    if ewma_ms <= sla_ms * 0.5 {
        1.0  // well within SLA
    } else if ewma_ms <= sla_ms {
        // linear decay from 1.0 to 0.5 as latency approaches SLA
        1.0 - 0.5 * ((ewma_ms - sla_ms * 0.5) / (sla_ms * 0.5))
    } else {
        // beyond SLA: penalty proportional to overshoot
        (0.5 * sla_ms / ewma_ms).max(0.0)
    }
}

/// Composite reward for bandit update.
fn composite_reward(quality: f64, cost_reward: f64, latency_reward: f64) -> f64 {
    0.60 * quality + 0.25 * cost_reward + 0.15 * latency_reward
}
```

**Status**: Wired. Runtime feedback computes routing reward with observed latency.

### Loop 8: Experiments -> Static

```rust
/// Observe Cell: reads ExperimentStore for concluded experiments.
/// Verify Cell: checks statistical significance (chi-squared or z-test).
/// React Cell: updates static routing table or prompt config with winner.
fn experiments_to_static(
    conclusion: &ExperimentConclusion,
    min_delta: f64,     // 0.05 (5% improvement required)
    max_p_value: f64,   // 0.05
    min_samples: usize, // 50
) -> Option<ConfigUpdate> {
    if conclusion.delta < min_delta
        || conclusion.p_value > max_p_value
        || conclusion.sample_size < min_samples
    {
        return None;  // not significant enough
    }
    Some(ConfigUpdate {
        key: format!("prompt.{}.variant", conclusion.section_name),
        new_value: conclusion.winner_variant.clone(),
        requires_review: true,  // human must approve config changes
    })
}
```

**Status**: Wired (persisted winner + router sync). Concluded experiments return their winner on future assignments.

---

## 2. Cross-Loop Interaction Matrix

The eight Loops do not operate independently. They interact, and interactions can cause oscillation if not managed by the stability mechanisms described in [drift-and-stability.md](drift-and-stability.md).

| Source Loop | Affected Loop | Interaction |
|---|---|---|
| 1 (Health->Routing) | 6 (Cost->Routing) | Provider failure forces fallback to more expensive provider |
| 2 (Conductor->Routing) | 7 (Latency->Reward) | High system load increases latency, penalizing reward signals |
| 3 (Section->Scaffold) | 5 (Skills->Prompts) | Section weight changes may truncate skill injection section |
| 4 (Failure->Replan) | 6 (Cost->Routing) | Replanning creates new tasks, increasing session cost |
| 6 (Cost->Routing) | 1 (Health->Routing) | Cost-forced downgrade to cheap provider may hit rate limits |
| 7 (Latency->Reward) | 2 (Conductor->Routing) | Latency-optimal routing may increase system load |
| 8 (Experiments->Static) | 3 (Section->Scaffold) | Experiment winner changes section content, resetting effectiveness data |

### Interaction-Aware Scheduling

To prevent cascading oscillation, updates are scheduled by priority tier:

```
Priority 1 (every episode):    Loop 1 (Health), Loop 6 (Cost)
    Safety-critical: prevent provider failures and budget overruns

Priority 2 (every 5 episodes): Loop 7 (Latency), Loop 2 (Conductor)
    Performance: optimize for speed and resource utilization

Priority 3 (every 20 episodes): Loop 3 (Section), Loop 5 (Skills)
    Learning: adjust prompt composition based on accumulated evidence

Priority 4 (every 50 episodes): Loop 4 (Failure->Replan), Loop 8 (Experiments)
    Strategic: structural changes with high confidence requirements
```

Safety-critical Loops always run before learning Loops, preventing a scenario where a learning-driven change causes a safety-critical failure.

---

## 3. Collective Calibration: The 31.6x Heuristic

The 31.6x figure is a CLT-inspired heuristic upper bound, not a theorem. It models collective accuracy as:

```
accuracy(t) = 1 - 1 / sqrt(N * t)
```

where N = agents, t = calibration rounds. For N=10, t=100: sqrt(1000) = 31.6, giving accuracy = 0.968.

### Why This Is a Heuristic, Not a Theorem

Five assumptions are required and frequently violated:

1. **Independence**: Agents' errors must be independent. Agents using the same model make correlated errors, reducing effective N.
2. **Stationarity**: Target distribution must not change during calibration. Codebase and model providers shift continuously.
3. **Optimal aggregation**: Formula assumes majority voting or Bayesian averaging. Roko uses sequential execution with feedback, not parallel voting.
4. **Finite-sample convergence**: CLT is asymptotic. For small N and t, the approximation is loose.
5. **Homogeneous quality**: Formula assumes equal-quality agents. Poor agents add noise, not signal.

**Practical expectation: 3-10x improvement from collective calibration, not 31.6x.** The 31.6x is the idealized upper bound under perfect conditions.

### C-Factor as the Measurement

The C-Factor (Collective Capability Factor) is the practical implementation. It combines multiple components into a weighted scalar, computed every 50 episodes:

| Component | Weight | What it measures |
|---|---|---|
| gate_pass_rate | 0.20 | Primary success metric |
| cost_efficiency | 0.15 | Budget sustainability |
| first_try_rate | 0.15 | Efficiency of approach |
| speed | 0.10 | Throughput |
| knowledge_growth | 0.10 | Learning velocity |
| turn_taking_equality | 0.05 | Collaboration quality |
| Others (distributed) | 0.25 | Secondary indicators |

Each component is normalized to [0.0, 1.0] relative to the system's own baseline (first 10 plans). A C-Factor of 0.8 means the system is performing significantly better than its initial configuration, regardless of what that initial configuration was.

### Leave-One-Out Contributions

C-Factor includes per-agent contribution scores: recompute C-Factor without each agent's episodes. If `contribution_score > 0`, the agent raises collective quality. If negative, it drags the collective down.

This feeds back into routing: agents with negative contributions get routed to stronger models. Agents with positive contributions can use cheaper models without quality loss.

See [c-factor-as-lens.md](c-factor-as-lens.md) for the full Lens Cell implementation, anti-groupthink React Cells, and WisdomGate Verify Cell.

---

## 4. Predictive Foraging: Prediction Error as Learning Signal

Predictive foraging turns every orchestrator decision into a falsifiable prediction. The key insight: **prediction errors are more informative than raw outcomes**. A task that fails is one data point. A task predicted to succeed with 90% confidence that fails is a strong signal of miscalibration.

### Four Prediction Types

| Prediction | Source | Outcome | Calibration target |
|---|---|---|---|
| Duration | Baseline stats for (role, complexity_band) | Actual wall time | Duration accuracy |
| Complexity | Static task analysis | Iterations, files touched | Complexity band accuracy |
| Gate outcome | Per-model pass rate adjusted by features | Pass/fail verdict | Brier score (target: 0) |
| Merge conflict | File overlap between concurrent tasks | Conflict / no conflict | False positive rate |

### CalibrationTracker

For probabilistic predictions, calibration is measured as the Brier score:

```
Brier score = (1/N) * sum( (predicted_probability - actual_outcome)^2 )
```

Perfectly calibrated: Brier = 0. Always-50% predictor: Brier = 0.25. Lower is better.

### Arithmetic Corrector

When systematic bias is detected, a simple correction fixes it:

```
correction_factor = actual_mean / predicted_mean
corrected_prediction = raw_prediction * correction_factor
```

This runs in ~50 nanoseconds per prediction. Despite its simplicity, it captures the dominant source of miscalibration (systematic bias). Per-category correction factors emerge over time:

```
config_modification:    correction = 0.733  (overconfident)
test_scaffolding:       correction = 1.05   (slightly underconfident)
cross_crate_refactor:   correction = 0.62   (very overconfident)
```

### Higher-Order Learning

Prediction errors create a hierarchy of learning signals:

```
Level 0: Task outcome (pass/fail)
    -> Level 1: Was the prediction correct? (calibration error)
    -> Level 2: Is the predictor systematically biased? (calibration drift)
    -> Level 3: Are the features informative? (feature importance)
```

Each level produces a distinct update: Level 0 updates the bandit arm, Level 1 updates the correction factor, Level 2 triggers predictor retraining, Level 3 informs system design improvements.

### Foraging Strategy

Optimal foraging theory (MacArthur & Pianka 1966) allocates effort proportional to expected return:

| Predicted outcome | Strategy |
|---|---|
| High pass probability, low cost | Quick: use cheapest model |
| High pass probability, high cost | Standard: optimize for cost |
| Low pass probability, low cost | Speculative: try cheap model first |
| Low pass probability, high cost | Careful: invest in thorough prompting |

The cascade router implements this through C-Factor-driven bias: high-confidence tasks get cheaper models, low-confidence tasks get stronger models.

---

## 5. Mori-Diffs Reality

Per `tmp/mori-diffs/04-LEARNING.md`:

- **Loops 1, 2, 6, 7**: All wired. Health, conductor pressure, cost guardrails, and latency all feed into routing decisions.
- **Loop 3 (Section->Scaffold)**: Wired for live orchestration path. Section metadata flows into efficiency events; learned lift signals reweight priorities. Broader coverage outside the orchestrator path is a remaining gap.
- **Loop 4 (Failure->Replan)**: Wired for orchestrator path. Gate failures trigger replan flows. Tighter coupling with `roko prd plan` and richer failure analysis are remaining gaps.
- **Loop 5 (Skills->Prompts)**: Wired for orchestration-layer injection. Deeper integration inside `SystemPromptBuilder` itself is a remaining gap.
- **Loop 8 (Experiments->Static)**: Wired (persisted winner + router sync). Optional materialization back into human-edited config files is a remaining gap.
- **Collective calibration**: C-Factor computation exists in `roko-learn/src/cfactor.rs` and is wired. Leave-one-out contributions inform dispatch bias.
- **Predictive foraging**: CalibrationTracker exists in `prediction.rs`. Basic predictions are wired for routing. Full four-type prediction coverage and per-category correction are partially implemented.

---

## What This Enables

1. **Self-regulating learning**: Eight Loops connect subsystems so deviations from optimal behavior trigger automatic corrective signals, following Ashby's Law of Requisite Variety.
2. **Collective improvement**: Calibrated agent collectives achieve 3-10x the throughput of individual agents through shared observation and joint recalibration.
3. **Prediction-driven efficiency**: Calibrated predictions route resources to where they will have the most impact, avoiding both over-investment in easy tasks and under-investment in hard ones.
4. **Interaction-aware scheduling**: Priority-tiered update scheduling prevents safety-critical Loops from being disrupted by slower learning Loops.

## Feedback Loops

- **Loop-of-Loops**: The interaction matrix creates second-order feedback. Loop 1 disrupts Loop 6, Loop 6 disrupts Loop 1 -- the priority scheduling is itself a stability mechanism (see [drift-and-stability.md](drift-and-stability.md)).
- **C-Factor as meta-signal**: C-Factor measures the aggregate effect of all eight Loops. A rising C-Factor means the Loops are collectively working. A falling C-Factor means at least one Loop is broken or counterproductive.
- **Prediction error as attention signal**: High prediction error in a category directs the system's learning attention to that category -- a form of curiosity where the system learns where its models are weakest and spends effort there first.

## Open Questions

1. **Loop interdependence depth**: The interaction matrix shows direct interactions. Are there indirect interactions (Loop A affects Loop B which affects Loop C) that create longer feedback chains? How deep do these chains go, and does depth increase oscillation risk?
2. **Calibration cold start**: The arithmetic corrector needs ~50 observations per category to produce reliable correction factors. During the cold-start period, should the system trust raw predictions (overconfident but consistent) or default to 50% (calibrated but uninformative)?
3. **C-Factor weighting**: The default component weights (0.20 pass rate, 0.15 cost, etc.) are set by judgment. Should they be learned from deployment data via the CohortWeightsLearner? If so, how do you avoid overfitting the weights to the first few plans?
4. **Collective calibration with heterogeneous models**: The 31.6x heuristic assumes equal-quality agents. In practice, agents use different models (opus, sonnet, haiku). Does heterogeneity help (diversity reduces correlation) or hurt (weak models add noise)?
