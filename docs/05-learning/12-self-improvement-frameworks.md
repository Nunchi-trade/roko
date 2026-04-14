# Self-Improvement Frameworks

> **Sources:** Academic literature survey, legacy research docs, implementation plans
> **Cross-references:** [02-skill-library-voyager](02-skill-library-voyager.md), [04-cascade-router](04-cascade-router.md), [17-adas-and-autocatalytic](17-adas-and-autocatalytic.md)


> **Implementation**: Shipping

---

## Purpose

This document surveys the academic and industrial frameworks that inform Roko's learning architecture. Each framework contributes a specific insight that is implemented (or planned) in the system. The survey is organized from concrete (implemented techniques) to speculative (research directions), with explicit citations for traceability.

---

## Agent Self-Improvement Frameworks

### Reflexion (Shinn et al. 2023)

**Insight:** Agents improve by reflecting on failures in natural language, then using those reflections as additional context in subsequent attempts.

**Roko implementation:** The episode logger captures gate failure signatures. The playbook rule system extracts if-then rules from failure patterns and injects them into subsequent agent prompts. This is a structured form of Reflexion: instead of free-form natural language reflection, Roko extracts typed rules with confidence tracking and trigger matching.

**Key difference:** Reflexion operates within a single task's retry loop. Roko's playbook rules persist across tasks and plans — a failure in plan A prevents the same mistake in plan B.

### ExpeL (Zhao et al. 2023)

**Insight:** Agents should extract generalizable "experiences" (insights) from successful and failed trials, accumulating them into a growing library.

**Roko implementation:** The skill library implements ExpeL-style experience extraction. Successful episodes produce skills (positive experiences); failure patterns produce playbook rules (negative experiences). Both persist across sessions and grow monotonically.

**Key difference:** ExpeL uses natural language experiences without confidence tracking. Roko's playbook rules have bounded confidence dynamics (validate +0.05, contradict −0.10) that automatically prune stale experiences.

### DSPy (Khattab et al. 2023)

**Insight:** Prompt optimization should be treated as a compiler problem: define a program signature, generate prompt variations, evaluate against a metric, and select the best-performing variant.

**Roko implementation:** The prompt experiment system (`ExperimentStore`) implements DSPy-style prompt optimization. Each experiment defines a prompt section, generates variants, assigns variants using UCB1 bandit selection, and evaluates against gate pass rate.

**Key difference:** DSPy optimizes statically (generate many variants, evaluate on a test set, select the winner). Roko optimizes online (bandit-driven variant selection during live execution, continuous evaluation).

### Meta-Harness (concept from self-hosted development)

**Insight:** A system that develops itself should use its own self-improvement mechanisms on its own self-improvement mechanisms. The harness that runs agents should itself be subject to optimization.

**Roko implementation:** This is the autocatalytic thesis described in [17-adas-and-autocatalytic](17-adas-and-autocatalytic.md). Roko uses its own learning loops to optimize the components that implement those learning loops. When Roko modifies `roko-learn` code, the cascade router learns which model works best for `roko-learn` tasks, and the skill library accumulates patterns specific to modifying the learning subsystem.

---

## Model Routing Research

### RouteLLM (Ong et al., ICLR 2025)

**Result:** 85% cost reduction while maintaining quality by routing queries to strong or weak models based on predicted difficulty.

**Approach:** Train a classifier (matrix factorization, BERT, or causal LM) on human preference data to predict which queries need a strong model. Route to weak model unless the classifier predicts the strong model is needed.

**Roko adaptation:** The cascade router's confidence stage implements a simpler version: empirical pass rates per model with confidence intervals, rather than a neural classifier. The LinUCB stage provides context-dependent routing similar to RouteLLM's classifier but using linear contextual bandits instead of neural networks.

### FrugalGPT (Chen et al., arXiv:2305.05176)

**Result:** 98% cost reduction with maintained quality by cascading through models from cheapest to most expensive, stopping when confidence is high enough.

**Approach:** Send the query to the cheapest model first. If the model's confidence (measured by agreement with a scoring model) is below threshold, escalate to the next more expensive model.

**Roko adaptation:** The cascade router's fallback mechanism implements this pattern: the `CascadeModel` includes both a primary and a fallback model. If the primary fails (gate failure, timeout), the orchestrator retries with the fallback. The three-stage cascade (Static→Confidence→UCB) is a different dimension of cascading: strategy complexity rather than model cost.

### MixLLM (concept)

**Result:** 97.25% of GPT-4 quality at 24.18% of the cost by mixing outputs from multiple models.

**Roko relevance:** Not directly implemented. Roko routes to a single model per task rather than mixing outputs. However, the collective calibration mechanism (see [15-collective-calibration-31x](15-collective-calibration-31x.md)) achieves a related effect: multiple agents with different models collectively produce better outcomes than any single agent.

### AutoMix (NeurIPS 2024)

**Insight:** Self-verification enables cascading without a separate scoring model. After the cheap model generates a response, ask it to verify its own answer. If self-verification fails, escalate to the expensive model.

**Roko adaptation:** Gate verification serves as Roko's "self-verification": the compile, test, and lint gates provide ground-truth feedback that the response is correct, without requiring a separate scoring model. This is more reliable than LLM self-verification because the gates are deterministic.

### Unified Routing (ETH Zurich, ICLR 2025)

**Insight:** Route across multiple providers simultaneously, considering cost, latency, and quality as a multi-objective optimization problem.

**Roko implementation:** The Pareto frontier computation (see [10-pareto-frontier-pruning](10-pareto-frontier-pruning.md)) and multi-provider health tracking (see [09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md)) implement unified routing across providers. The cascade router considers cost, quality (pass rate), and latency SLA when selecting models.

### Speculative Cascades

**Concept:** Start processing with a cheap model while simultaneously evaluating whether to hand off to an expensive model. If the cheap model's partial output looks promising, continue; otherwise, switch.

**Roko relevance:** Not implemented. Roko processes tasks sequentially (one model attempt at a time) rather than speculatively. Speculative cascading would require streaming gate evaluation, which the current batch-gate pipeline doesn't support.

---

## Production Routing Systems

### LiteLLM

Open-source proxy that standardizes API calls across 100+ LLM providers. Provides routing, fallback, and cost tracking. Roko's `roko-agent` dispatcher serves a similar function but is specialized for agent workloads with gate-based feedback.

### OpenRouter

Commercial routing service that provides unified API access to multiple models. Roko's cascade router draws from OpenRouter's approach of maintaining per-model performance statistics and routing based on empirical quality data.

### Portkey

Production LLM gateway with routing, fallback, and observability. Roko's provider health tracking is inspired by Portkey's circuit breaker patterns.

---

## Self-Improvement Prerequisites

The self-improvement literature consistently identifies prerequisites that Roko satisfies:

### External Verifier Requirement

Huang et al. (ICLR 2024), Song et al. (ICLR 2025), and Pan et al. (ICML 2024) establish that self-improvement requires an external verifier: models cannot reliably improve their own outputs without ground-truth feedback.

**Roko's verifier:** The 11-gate pipeline (compile, test, clippy, diff, etc.) provides deterministic external verification. This is stronger than the weak verifiers (LLM-as-judge) used in most self-improvement research, because gate outcomes are not subject to model bias or hallucination.

### Karpathy Autoresearch Pattern

Andrej Karpathy's autoresearch experiment (700 experiments, 11% speedup, rediscovered RMSNorm) demonstrates that automated experimentation can produce genuine insights, but requires careful metric tracking and experiment isolation.

**Roko implementation:** The prompt experiment system (`ExperimentStore`) implements isolated A/B testing with bandit-driven variant selection. The cascade router provides automated model experimentation. Both produce structured outcome data for analysis.

---

## Context Assembly Optimization

The highest-leverage self-improvement in the legacy system (mori-agents/07-self-improvement.md) was identified as **adaptive context dropping** — learning which prompt sections contribute to gate passes and which waste tokens. This insight directly motivated:

- The `PromptSectionMeta` tracking in efficiency events (section-level token attribution).
- The prompt experiment system (A/B testing prompt section variants).
- The section effectiveness feedback loop (loop 3 in [13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)).

---

## Four Key Metrics for Self-Improvement

From the legacy analysis (mori-agents/07-self-improvement.md):

| Metric | Definition | Self-Improvement Lever |
|--------|-----------|----------------------|
| First-attempt pass rate | % tasks passing gates first try | Playbook rules prevent known failures |
| Iterations per plan | Avg iterations to complete | Better model routing, better prompts |
| Cost per plan | Total USD per plan | Model routing, cache optimization |
| Prompt tokens per spawn | Input tokens for initial prompt | Context assembly optimization |

These four metrics form the core of Roko's self-improvement feedback: every learning subsystem ultimately aims to improve one or more of these numbers.

---

## Router-R1 and Speculative Cascades

### Router-R1

A reinforcement-learning-trained router that uses chain-of-thought reasoning to make routing decisions. Unlike RouteLLM's classifier approach, Router-R1 generates an explicit reasoning trace before making the routing decision, enabling interpretable routing logic.

**Roko relevance:** The cascade router's stage transitions (Static → Confidence → UCB) can be seen as a hardcoded reasoning chain. Router-R1 suggests that this chain itself could be learned — an ADAS-level optimization (see [17-adas-and-autocatalytic](17-adas-and-autocatalytic.md)).

### Speculative Cascades

Start processing with a cheap model while simultaneously evaluating whether to hand off to an expensive model. If the cheap model's partial output looks promising, continue; otherwise, switch. This requires streaming gate evaluation, which the current batch-gate pipeline doesn't support.

**Roko relevance:** Not implemented. The current gate pipeline evaluates complete outputs, not streaming partial results. Speculative cascading would require modifications to the gate pipeline architecture (see [04-verification](../04-verification/INDEX.md)).

---

## Unified Routing (ETH Zurich, ICLR 2025)

A comprehensive framework for routing across multiple providers simultaneously, treating cost, latency, and quality as a multi-objective optimization problem. The unified approach considers the entire provider landscape as a single decision space rather than selecting providers independently.

**Key insight:** Provider-level routing (which provider to use) and model-level routing (which model to use) should be solved jointly, because the same model may have different cost, latency, and quality characteristics across providers.

**Roko implementation:** The `ProviderHealthRegistry` + `CascadeRouter` + `LatencyRegistry` together implement a form of unified routing. Provider health filters out degraded providers, the cascade router selects models, and latency statistics inform SLA compliance. However, these operate sequentially rather than jointly — full unified routing would optimize across all three dimensions simultaneously.

---

## Practical Insights from Production

### Adaptive Context Dropping (Highest Leverage)

The legacy analysis (mori-agents/07-self-improvement.md) identified adaptive context dropping as the single highest-leverage self-improvement technique. The insight: most prompt sections in an agent's system prompt are irrelevant to the current task, but they consume tokens and may confuse the agent. Learning which sections to drop (or heavily truncate) for each task type can:

- Reduce prompt size by 30-50% (saving input token costs).
- Improve pass rates by 5-15% (less noise in the prompt).
- Reduce latency by 20-40% (fewer tokens to process).

**Roko implementation:** The `PromptSectionMeta` tracking in efficiency events (per-section token attribution), combined with feedback loop 3 (Section→Scaffold), enables adaptive context dropping. The system tracks which sections correlate with gate passes and adjusts section weights accordingly.

### Warm Pool Optimization

Reusing agent processes (warm starts) instead of spawning fresh processes (cold starts) saves:
- Process startup time (~2-5 seconds per agent spawn).
- KV cache priming (~1000-5000 tokens of system prompt re-processing).
- Memory allocation overhead.

The `AgentEfficiencyEvent.was_warm_start` field tracks warm vs. cold start distribution, enabling measurement of warm pool effectiveness.

---

## Framework Comparison Matrix

| Framework | Input | Output | Learning Signal | Persistence | Roko Equivalent |
|-----------|-------|--------|----------------|-------------|-----------------|
| Reflexion | Failed attempt + reflection prompt | Natural language reflection | Task retry success | Per-task context | Playbook rules |
| ExpeL | Episode batch | Generalized insights | Insight validation rate | Cross-task library | Skill library |
| DSPy | Program signature + test set | Optimized prompt | Test set accuracy | Static compilation | Prompt experiments |
| Voyager | Minecraft task | JavaScript function | Environment feedback | Skill library | Skill library |
| RouteLLM | Query | Strong/weak routing | Human preference | Router model weights | Cascade router |
| FrugalGPT | Query | Model cascade | Scoring model | Cascade config | Cascade router |
| AutoMix | Query | Self-verified cascade | Self-verification | None (online) | Gate pipeline |
| ADAS | Architecture spec | New architecture code | Benchmark evaluation | Archive of designs | (Planned) |

---

## Open Research Questions

Several open questions inform future development:

1. **Can a system improve its own improvement mechanisms?** Meta-Harness suggests yes, but the empirical evidence is limited to Karpathy's autoresearch experiment (11% speedup) and small-scale ADAS results (+14% on ARC). Whether these results transfer to large-scale software engineering is unknown.

2. **Does the external verifier requirement create a ceiling?** Huang et al. (ICLR 2024) show that self-improvement requires external verification. Roko's gate pipeline provides this, but the gates themselves are fixed — they don't improve. A system that improves its verifiers (automatically adding new test cases, discovering new lint rules) would have a higher ceiling.

3. **What is the optimal exploration budget?** All bandit algorithms trade exploration (trying suboptimal options) against exploitation (using the best-known option). The optimal tradeoff depends on the rate of environmental change, which is itself changing. Adaptive exploration budgets (like Thompson Sampling with drift) are theoretically sound but empirically untested in agent systems.

4. **Can cross-project transfer overcome the cold-start problem?** Skills and patterns extracted from project A may accelerate project B, but the transfer rate depends on structural similarity between projects. The HDC fingerprint approach enables fast similarity matching, but the quality of transferred knowledge is untested at scale.

---

## Improvement Measurement: Rigorous Quantification

Self-improvement claims require rigorous measurement. Without principled metrics and experimental controls, apparent improvements may be noise, regression to the mean, or artifacts of changing task distributions. This section specifies the measurement framework.

### Improvement Score Card

```rust
pub struct ImprovementScoreCard {
    /// Time window for comparison.
    pub window: TimeWindow,
    /// Baseline period metrics.
    pub baseline: PeriodMetrics,
    /// Current period metrics.
    pub current: PeriodMetrics,
    /// Statistical significance of observed changes.
    pub significance: SignificanceTests,
    /// Confound analysis.
    pub confounds: Vec<Confound>,
}

pub struct PeriodMetrics {
    /// Episode count in this period.
    pub n_episodes: usize,
    /// Four key metrics from mori-agents/07-self-improvement.md.
    pub first_attempt_pass_rate: f64,
    pub avg_iterations_per_plan: f64,
    pub avg_cost_per_plan_usd: f64,
    pub avg_prompt_tokens_per_spawn: u64,
    /// Extended metrics.
    pub skill_library_size: usize,
    pub playbook_rule_count: usize,
    pub c_factor: f64,
    pub avg_calibration_error: f64,
}

pub struct SignificanceTests {
    /// Two-proportion z-test for pass rate difference.
    pub pass_rate_z_score: f64,
    pub pass_rate_p_value: f64,
    /// Welch's t-test for cost difference.
    pub cost_t_statistic: f64,
    pub cost_p_value: f64,
    /// Mann-Whitney U test for iterations (non-parametric).
    pub iterations_u_statistic: f64,
    pub iterations_p_value: f64,
    /// Is the improvement statistically significant at α = 0.05?
    pub is_significant: bool,
}

pub enum Confound {
    /// Task distribution changed between periods.
    TaskDistributionShift {
        metric: String,
        baseline_distribution: Vec<f64>,
        current_distribution: Vec<f64>,
        kl_divergence: f64,
    },
    /// Model provider updated between periods.
    ProviderUpdate {
        model: String,
        update_timestamp: DateTime<Utc>,
    },
    /// Configuration change between periods.
    ConfigChange {
        key: String,
        old_value: String,
        new_value: String,
    },
    /// Sample size too small for reliable comparison.
    InsufficientSample {
        metric: String,
        n_required: usize,
        n_actual: usize,
    },
}
```

### Improvement Attribution

When improvement is detected, attribution identifies which learning subsystem caused it:

```
Improvement detected: pass rate 0.62 → 0.78 (+26%, p < 0.01)

Attribution analysis:
    1. Check if model routing changed → router selected opus more often (+12% of change)
    2. Check if new playbook rules were promoted → 3 new rules matched failing tasks (+8%)
    3. Check if skill library grew → 5 new skills for this task category (+4%)
    4. Check if prompt experiments concluded → "concise" variant won (+2%)
    Residual (unexplained): 0%

Most impactful subsystem: Cascade router (model selection improvement)
```

### Controlled Experiments via Holdout

The gold standard for measuring improvement is a controlled experiment: randomly assign tasks to a "learning" group (all subsystems active) and a "holdout" group (learning frozen at baseline state).

```rust
pub struct ImprovementExperiment {
    /// Experiment identifier.
    pub id: String,
    /// Start timestamp.
    pub started_at: DateTime<Utc>,
    /// Treatment: current learning configuration.
    pub treatment_config: LearningConfig,
    /// Control: frozen baseline configuration.
    pub control_config: LearningConfig,
    /// Assignment: hash(task_id) % 100 < treatment_pct → treatment.
    pub treatment_pct: u8,  // default: 80 (80% treatment, 20% holdout)
    /// Results accumulator.
    pub treatment_results: PeriodMetrics,
    pub control_results: PeriodMetrics,
    /// Minimum tasks before concluding.
    pub min_tasks: usize,  // default: 100
}
```

The holdout design ensures that observed improvements are caused by learning rather than external factors (easier task mix, model provider updates, codebase maturation).

### Monotonicity Tracking

Self-improvement should be monotonic: the system should get better over time, not oscillate. Monotonicity is tracked via the C-Factor trend:

```
C-Factor time series:
    0.48, 0.51, 0.53, 0.55, 0.54, 0.57, 0.61, 0.63, 0.65, 0.68
    ← monotonically increasing (with small perturbations)

Monotonicity score = fraction of steps where C(t) > C(t-1)
    = 8/9 = 0.89 (high monotonicity)

If monotonicity < 0.60 over 20+ episodes:
    → Learning system is not converging
    → Investigate: oscillation? regression? environmental shift?
```

---

## Improvement Safety: Preventing Harmful Self-Modification

A self-improving system can improve in harmful directions: optimizing for pass rate by generating trivially passing code, optimizing for cost by producing low-quality outputs, or modifying its own safety checks to avoid gate failures. Improvement safety mechanisms prevent these failure modes.

### Safety Invariants

```rust
pub struct SafetyInvariants {
    /// Gate pipeline must never be disabled or bypassed.
    pub gates_enabled: bool,
    /// Minimum gate count (at least compile + test).
    pub min_gate_count: usize,  // default: 2
    /// Gate thresholds must never drop below absolute floor.
    pub gate_threshold_floor: f64,  // default: 0.30
    /// Playbook rules cannot override safety-critical gates.
    pub safety_gates_immutable: Vec<String>,  // ["compile", "test"]
    /// Maximum model downgrade depth (prevent cascading to weakest model).
    pub max_downgrade_steps: u32,  // default: 2
    /// Self-modification detection: alert if learning modifies learning code.
    pub self_modification_alert: bool,  // default: true
}

pub enum SafetyViolation {
    /// A gate was disabled or its threshold dropped below floor.
    GateWeakened { gate: String, old_threshold: f64, new_threshold: f64 },
    /// A playbook rule attempts to override a safety-critical gate.
    SafetyGateOverride { rule_id: String, gate: String },
    /// Model selection cascaded below the minimum quality threshold.
    ExcessiveDowngrade { target_model: String, downgrade_depth: u32 },
    /// Learning subsystem is modifying its own code paths.
    SelfModification { modified_crate: String, modifier_task: String },
    /// Output quality metrics declined while pass rate increased (gaming gates).
    GateGaming { pass_rate_delta: f64, quality_delta: f64 },
    /// Cost optimization produced outputs below minimum quality.
    QualityFloor { task_id: String, quality_score: f64, threshold: f64 },
}
```

### Gate Gaming Detection

The most insidious failure mode is "gate gaming": the system learns to produce outputs that pass gates without actually solving the task. Detection:

```
Gate gaming indicators:
    1. Pass rate increases while downstream quality decreases
       (code passes tests but has bugs discovered later)
    2. Output complexity decreases (shorter, simpler code that
       technically passes but doesn't handle edge cases)
    3. Test coverage decreases while test pass rate increases
       (trivial tests that always pass)
    4. Diff size shrinks toward zero (minimal changes that pass gates
       but don't address the task requirements)
```

```rust
pub struct GateGamingDetector {
    /// Window of recent episodes for analysis.
    pub window_size: usize,  // default: 50
    /// Alert if pass rate increases by >10% while quality score decreases by >5%.
    pub pass_quality_divergence_threshold: f64,  // default: 0.05
    /// Alert if average diff size drops below this fraction of baseline.
    pub min_diff_size_fraction: f64,  // default: 0.30
    /// Alert if output token count drops below this fraction of baseline.
    pub min_output_fraction: f64,  // default: 0.40
}
```

### Constitutional Constraints

Inspired by Constitutional AI (Bai et al. 2022), the self-improvement system operates under constitutional constraints — inviolable rules that no learning subsystem can override:

```toml
# In roko.toml [safety] section
[safety.constitution]
# Learning cannot disable gates
gates_immutable = true
# Learning cannot modify the safety module itself
self_modification_forbidden_crates = ["roko-gate", "roko-agent/safety"]
# Model selection must always include at least one high-quality option
min_quality_model_tier = "standard"
# Budget optimization cannot reduce quality below floor
quality_floor = 0.50
# All self-modifications require human review
self_mod_requires_review = true
```

### Improvement Velocity Limits

Even beneficial improvements should be rate-limited to prevent cascade failures:

```rust
pub struct ImprovementVelocityLimits {
    /// Maximum playbook rule changes per day.
    pub max_rule_changes_per_day: u32,  // default: 10
    /// Maximum routing table changes per day.
    pub max_routing_changes_per_day: u32,  // default: 20
    /// Maximum prompt experiment conclusions per day.
    pub max_experiment_conclusions_per_day: u32,  // default: 5
    /// Cooldown after a safety violation (minutes).
    pub safety_violation_cooldown_minutes: u32,  // default: 60
    /// Maximum C-Factor change per episode (damping).
    pub max_cfactor_delta: f64,  // default: 0.02
}
```

These limits prevent a scenario where a false positive in the improvement pipeline triggers a cascade of changes that collectively degrade the system. By limiting the rate of change, the system has time to detect and recover from individual bad decisions.

### Connection to AI Safety Research

The improvement safety framework draws on three lines of research:

1. **Constitutional AI (Bai et al. 2022):** Inviolable rules that constrain self-improvement. Roko's constitutional constraints are the safety analogue.

2. **Scalable oversight (Amodei et al. 2016):** As systems become more capable, human oversight must scale. Roko's `self_mod_requires_review` flag ensures human-in-the-loop for self-referential changes.

3. **Reward hacking (Skalse et al. 2022):** Optimizing for a proxy metric (gate pass rate) can diverge from the true objective (correct code). Gate gaming detection explicitly monitors for this divergence.

4. **Self-play safety (Silver et al. 2017; OpenAI Five 2019):** Self-play can discover exploits in the reward function. Roko's holdout experiment design provides a control group that detects if the "improved" system is actually gaming rather than improving.

---

## Relationship to Other Documents

- **[02-skill-library-voyager](02-skill-library-voyager.md)** — Implements Voyager-style skill accumulation (Wang et al. 2023).
- **[04-cascade-router](04-cascade-router.md)** — Implements RouteLLM/FrugalGPT-inspired cascading.
- **[01-playbook-system](01-playbook-system.md)** — Implements Reflexion/ExpeL-style experience extraction.
- **[13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)** — Maps these frameworks to specific cybernetic feedback loops.
- **[17-adas-and-autocatalytic](17-adas-and-autocatalytic.md)** — Extends self-improvement to meta-level architecture search (ADAS) and autocatalytic growth.
