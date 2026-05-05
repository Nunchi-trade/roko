# 11 — Dual-Process Tier Routing

> Sub-doc 11 of **02-agents** · Roko Documentation
>
> This document describes the dual-process cognitive model (System 1 / System 2)
> as applied to Roko's model tier routing, the CascadeRouter, the LinUCB bandit,
> Pareto frontier computation, and anomaly detection.


> **Implementation**: Shipping

---

## The Cognitive Model

Roko's model routing is inspired by dual-process theory from cognitive science
(Kahneman, 2011):

- **System 1** (fast, automatic) — Quick, pattern-matching responses. Low cost,
  low latency. Maps to the Fast model tier (Haiku-class).
- **System 2** (slow, deliberate) — Careful reasoning, multi-step analysis.
  Higher cost, higher quality. Maps to the Premium model tier (Opus-class).

The insight: most agent tasks don't need System 2. Classification, validation,
orchestration overhead, and simple code changes can be handled by fast models.
Only hard debugging, architectural decisions, and complex reasoning need premium
models. Routing everything through premium models wastes money without improving
outcomes.

---

## Three Model Tiers

```rust
pub enum ModelTier {
    Fast,      // Haiku-class: classification, watchers, orchestration
    Standard,  // Sonnet-class: implementation, review (the workhorse)
    Premium,   // Opus/GPT-5-class: architecture, hard debugging
}
```

| Tier | Examples | Typical cost | Use cases |
|---|---|---|---|
| Fast | Claude Haiku, GPT-4o-mini | ~$0.25/M input | Watchers, validators, conductors |
| Standard | Claude Sonnet, GPT-4o | ~$3/M input | Implementation, review, testing |
| Premium | Claude Opus, GPT-5 | ~$15/M input | Architecture, hard debug, audit |

Each role has a default tier (sub-doc 04), but the CascadeRouter can override
it dynamically based on learned performance data.

---

## CascadeRouter

The CascadeRouter implements the dual-process model as a multi-stage
confidence cascade:

```
Task arrives
    │
    ▼
Stage 1: Try Fast model (System 1)
    │
    ├── Confidence ≥ threshold → Accept result, done
    │
    ▼
Stage 2: Try Standard model
    │
    ├── Confidence ≥ threshold → Accept result, done
    │
    ▼
Stage 3: Try Premium model (System 2)
    │
    └── Accept result regardless
```

The "confidence" signal comes from multiple sources:
1. **Gate results** — Did the output pass compile/test/clippy gates?
2. **Self-assessment** — Did the model express uncertainty in its output?
3. **Historical performance** — For similar tasks, which tier succeeded?
4. **Cost-quality tradeoff** — Given the budget, is escalation worth it?

### Confidence computation

The confidence score is a weighted combination of signals:

```
confidence = w1 × gate_pass_rate
           + w2 × (1 - uncertainty_markers)
           + w3 × historical_success_rate
           + w4 × task_complexity_estimate
```

The weights are learned via the LinUCB bandit (see below). The threshold
for accepting a fast-model result depends on the temperament setting
(sub-doc 10): Conservative requires 0.9 confidence, Balanced requires 0.7.

### Persistence

The CascadeRouter persists its state to `.roko/learn/cascade-router.json`.
This means routing decisions improve across sessions — a model that
consistently fails for a task type will be avoided in future runs.

---

## LinUCB Bandit

The CascadeRouter uses a **LinUCB contextual bandit** (Li et al., 2010,
"A contextual-bandit approach to personalized news article recommendation")
to select models within each tier:

### How it works

1. **Context vector** — For each task, compute features: task type, estimated
   complexity, historical performance, role, current budget.
2. **Arm selection** — Each model is an "arm". LinUCB computes an upper
   confidence bound for each arm given the context.
3. **Reward** — After the task completes, the reward signal combines: gate
   pass/fail, token efficiency, wall-clock time, cost.
4. **Update** — The bandit updates its weight matrix for the selected arm.

LinUCB balances exploration (trying new models to learn their performance)
with exploitation (using the model that's historically best for this context).
The exploration parameter is controlled by temperament: Exploratory temperament
sets a high exploration parameter, causing the bandit to try more models.

### Pareto frontier pruning

Before the bandit selects a model, a Pareto frontier computation prunes the
candidate set. Models are evaluated on two dimensions:

1. **Quality** — Historical gate pass rate for the task type
2. **Cost** — Price per million tokens

Models that are dominated (worse on both dimensions than another model) are
removed from consideration. This prevents the bandit from exploring obviously
bad options.

Implementation plan `modelrouting/2G.10` describes the Pareto computation.
Implementation plan `modelrouting/2G.11` describes applying Pareto pruning
to the LinUCB exploration set.

---

## Thompson Sampling

For the confidence-threshold decision (escalate or accept?), the CascadeRouter
uses Thompson sampling over the weighted confidence signals:

```
For each tier t:
    sample θ_t ~ Beta(successes_t, failures_t)
    adjusted_confidence = θ_t × raw_confidence
```

This introduces beneficial randomness: even when the fast model's average
confidence is below threshold, it occasionally gets a chance (when the sampled
θ is high), allowing the system to discover that the fast model has improved
for certain task types.

---

## Anomaly Detection

Implementation plan `modelrouting/2G.12` introduces an `AnomalyDetector`
that monitors model performance for unusual patterns:

```rust
pub struct AnomalyDetector {
    // Tracks per-model running statistics
    // Flags when a model's recent performance deviates significantly
    // from its historical baseline
}
```

The detector watches for:
- **Sudden quality drops** — A model that was passing gates 90% of the time
  drops to 50% (provider degradation, model update).
- **Latency spikes** — Response times exceed 2× the rolling average.
- **Cost anomalies** — Token usage significantly higher than expected for
  the task type.

When an anomaly is detected, the router temporarily de-prioritizes the
affected model and fires an alert to the Monitor role.

Implementation plan `modelrouting/2G.16` wires the AnomalyDetector into
the dispatch pipeline.

---

## Three Cognitive Speeds

The dual-process model maps to three cognitive speeds in Roko's execution:

| Speed | Latency | Description | Example |
|---|---|---|---|
| **Gamma** | ~5-15s | Fast reflexive response | File read, simple classification |
| **Theta** | ~75s | Standard deliberation | Implementation, code review |
| **Delta** | Hours | Deep reasoning | Architecture, research, complex debug |

The CascadeRouter uses these speeds as a prior: Gamma tasks start at Fast
tier, Theta tasks start at Standard, Delta tasks start at Premium. The
bandit can override these starting points based on learned performance.

---

## Active Inference Connection

The model routing system is theoretically grounded in the Free Energy
Principle (Friston, 2010). The CascadeRouter's behavior can be interpreted
as minimizing expected free energy:

- **Epistemic value** — Exploration (trying new models) reduces uncertainty
  about model performance, lowering expected free energy.
- **Pragmatic value** — Exploitation (using known-good models) directly
  achieves task objectives.
- **Confidence threshold** — The threshold acts as a precision parameter:
  high precision (Conservative) demands more evidence before accepting,
  low precision (Exploratory) accepts with less evidence.

This connection is documented in refactoring PRD §01-synapse-architecture
and provides the theoretical basis for why the bandit approach works:
it naturally balances the explore-exploit tradeoff in a principled way.

---

## Research Context

The model routing approach draws on several research directions:

1. **RouteLLM** (2024) — Binary classifier for cheap/expensive model routing.
   Roko extends this to a multi-tier cascade with contextual bandits.
2. **MixLLM** (2024) — Mixed model serving with learned routing policies.
3. **FrugalGPT** (Chen et al., 2023) — Cost-efficient LLM serving via
   cascading and caching. The cascade structure is similar.
4. **AutoMix** (2024) — Automatic model mixing based on query difficulty.
5. **Router-R1** (2025) — RL-trained router that learns per-query routing.

Implementation plan `modelrouting/11-research-context.md` provides full
citations and comparative analysis for each approach.

Roko's contribution is combining these approaches into a unified system:
Pareto pruning (from multi-objective optimization) → LinUCB selection (from
contextual bandits) → Thompson sampling for confidence (from Bayesian
decision theory) → anomaly detection for robustness.

---

## Dual-Process Theory 2.0: Recent Advances

The classical System 1/System 2 dichotomy has been refined by cognitive
science research (De Neys & Pennycook, 2019; De Neys, 2018):

### Competing Intuitions Model

People can process logical principles *intuitively*, without deliberation.
This challenges the classic view that logical reasoning is exclusively
System 2. The revised model proposes **multiple types of intuitions**: some
are logical and reliable, others are heuristic and less reliable. These
competing intuitions can differ in **activation strength** — when heuristic
and logical intuitions have similar activation, deliberation (System 2) is
more likely to intervene.

**Mapping to Roko:** The CascadeRouter's "confidence signal" is the
activation strength. When the Fast tier's confidence is high (strong
intuition), accept immediately. When confidence is uncertain (competing
intuitions), escalate to Standard or Premium tier (deliberation).

### Default-Interventionist vs. Parallel-Competitive

Two competing models explain how the dual processes interact:

| Model | Architecture | Key property |
|---|---|---|
| **Default-Interventionist** | Sequential: System 1 first, System 2 monitors | System 2 only intervenes when needed |
| **Parallel-Competitive** | Parallel: both systems run simultaneously | Systems compete for behavioral control |
| **Hybrid Two-Stage** | Shallow monitoring always active, deep processing on conflict | Best of both approaches |

The **Hybrid Two-Stage model** best maps to Roko's CascadeRouter:
1. A "shallow analytic monitoring process" (confidence estimation) is always
   active.
2. An "optional deeper processing stage" (model escalation) activates only
   when conflict is detected (low confidence, gate failure).

### Triple-Process Theory: Type 3 Metacognition

Evans (2019) and Vieira et al. (2022) propose a **Type 3 metacognitive
process** that sits above both System 1 and System 2 as a regulatory
mechanism. Houdé proposed a metacognitive "System 3" capable of *inhibiting*
System 1 to enable System 2.

**Mapping to Roko:** Type 3 = **meta-routing** (routing the router).
The metacognitive layer decides *whether* to engage the CascadeRouter's
learned model selection or to use a simple heuristic.

---

## Mixture of Experts Connection

MoE routing within a single model (choosing experts per token) is
architecturally analogous to model-level routing (choosing between LLMs per
query). The algorithmic principles transfer directly:

| MoE concept | Model routing equivalent |
|---|---|
| Gating network | CascadeRouter |
| Expert | Individual model (Haiku, Sonnet, Opus) |
| Top-K selection | Cascade stages (try Fast, then Standard, then Premium) |
| Load balancing | Rate limit awareness + cost budget distribution |
| Expert collapse | Model monoculture (always routing to one model) |
| Sparse activation | Only invoking the cheapest sufficient model |

### Expert Choice Routing

Zhou et al. (2022, arXiv:2202.09368) inverted the MoE selection: instead of
tokens choosing experts, **experts choose tokens**. Applied to model routing,
this means the CascadeRouter could assign tasks to models based on each
model's self-assessed suitability rather than a central router's prediction.

### Avoiding Expert Collapse

A key MoE challenge where only a small subset of experts receive the majority
of inputs. In model routing, this manifests as **model monoculture** — always
routing to one familiar model. The LinUCB exploration parameter and Thompson
sampling already address this, but additional mechanisms can help:

```rust
/// Anti-collapse mechanisms for the CascadeRouter.
pub struct CollapseAvoidance {
    /// Minimum exploration rate: fraction of tasks routed to non-default models
    /// even when the default model appears optimal (default: 0.05 = 5%).
    pub min_exploration_rate: f64,
    /// Recency weighting: geometric forgetting factor for sufficient statistics.
    /// Smaller = forget faster, adapt to model changes sooner (default: 0.95).
    pub geometric_forgetting: f64,
    /// Maximum consecutive uses of the same model before forced exploration
    /// (default: 20).
    pub max_consecutive_same_model: usize,
    /// Diversity bonus: reward models that haven't been used recently
    /// (default: 0.1 bonus per 100 tasks since last use).
    pub diversity_bonus_per_100: f64,
}
```

---

## Routing Feedback Loops

How routing decisions improve over time — the learning mechanisms that make
the CascadeRouter better with each task.

### Online Bandit Learning

The LinUCB bandit updates its confidence bounds after every routing decision:

```
For each task:
  1. Observe context x (task type, estimated complexity, role, budget)
  2. Select model a = argmax(θ_a · x + α × sqrt(x' · A_a^(-1) · x))
  3. Observe reward r (gate pass/fail, tokens, latency, cost)
  4. Update: A_a += x · x', b_a += r · x
```

**PILOT** (arXiv:2508.21141, 2025) extends this with offline human preference
data as a prior, creating a shared embedding space for queries and LLMs that
is initially learned from offline preference data and refined through online
bandit feedback.

### Exponential Moving Average Adaptation

The CascadeRouter uses EMA for rapid adaptation to shifts in model pricing
and quality:

```rust
/// EMA-based adaptation for routing statistics.
pub struct EmaStats {
    /// Smoothing factor (default: 0.05 — slow adaptation).
    pub alpha: f64,
    /// Per-model running statistics.
    pub model_stats: HashMap<String, ModelRunningStats>,
}

pub struct ModelRunningStats {
    /// EMA of gate pass rate.
    pub pass_rate: f64,
    /// EMA of average latency (ms).
    pub latency_ms: f64,
    /// EMA of cost per task (USD).
    pub cost_per_task: f64,
    /// EMA of token efficiency (useful output tokens / total tokens).
    pub token_efficiency: f64,
    /// Count of observations.
    pub observation_count: u64,
}
```

### ParetoBandit: Budget-Aware Online Routing

ParetoBandit (2025) is the first open-source adaptive router that
simultaneously enforces dollar-denominated budgets, adapts online to shifts
in pricing and quality, and onboards new models at runtime. It uses an online
primal-dual budget pacer and geometric forgetting on sufficient statistics.

---

## Meta-Routing: Routing the Router

When should the system use the learned CascadeRouter vs. a simple heuristic?

### The Cost of Routing

| Router type | Overhead | Quality | Best for |
|---|---|---|---|
| **Static heuristic** | ~0 ms | Low | Known task types, stable model fleet |
| **kNN similarity** | ~1 ms | Medium | Warm-start, moderate model diversity |
| **LinUCB bandit** | ~2 ms | High | Online learning, model exploration |
| **RL-trained (Router-R1)** | ~50 ms | Highest | Complex multi-round decisions |
| **LLM-as-router** | ~500 ms | Variable | Very complex routing decisions |

**Key finding:** A well-tuned k-Nearest Neighbors approach often *matches
or outperforms* state-of-the-art learned routers (arXiv:2505.12601, 2025).
The locality properties of model performance in embedding space enable simple
non-parametric methods to achieve strong routing with lower sample complexity.

### Hierarchical Meta-Routing

```rust
/// Meta-routing: select the routing strategy based on task characteristics.
/// The meta-router is itself the Type 3 metacognitive process.
pub struct MetaRouter {
    /// Heuristic router: instant, no overhead.
    heuristic: HeuristicRouter,
    /// Learned router: LinUCB bandit with online adaptation.
    learned: CascadeRouter,
    /// kNN router: fast similarity-based routing.
    knn: KnnRouter,
    /// Meta-policy: when to use which router.
    policy: MetaRoutingPolicy,
}

pub struct MetaRoutingPolicy {
    /// Use heuristic if task matches a known pattern with >0.9 confidence.
    pub heuristic_confidence_threshold: f64,
    /// Use kNN if we have >100 observations for this task type.
    pub knn_observation_threshold: u64,
    /// Use learned router otherwise (exploration phase).
    /// Fall back to heuristic if budget is critically low.
    pub budget_fallback_threshold_usd: f64,
}

impl MetaRouter {
    pub fn route(&self, task: &Engram, ctx: &RoutingContext) -> ModelSelection {
        // 1. Check if task matches a known heuristic pattern
        if let Some(model) = self.heuristic.try_route(task) {
            if self.policy.heuristic_confidence_threshold <= self.heuristic.confidence(task) {
                return model;
            }
        }

        // 2. Check if kNN has enough observations
        let obs_count = self.knn.observation_count_for(task);
        if obs_count >= self.policy.knn_observation_threshold {
            return self.knn.route(task);
        }

        // 3. Use learned router (exploration)
        self.learned.route(task, ctx)
    }
}
```

### Cascade Routing Unification

Dekoninck et al. (2025, arXiv:2410.10347, ICLR 2025) unified routing
(single model chosen per query) and cascading (sequential models until
satisfactory answer) into a single framework: **cascade routing**. This
iteratively picks the best model — can skip models, reorder them, run only as
few as needed. Improves performance by up to 8% on RouterBench and 14% on
SWE-Bench.

**Mapping to Roko:** The CascadeRouter already implements a cascade. The
unification insight is that the cascade should be **dynamic** — not always
Fast → Standard → Premium, but potentially Fast → Premium (skipping Standard)
or Standard → Fast (de-escalating) based on the task and model fleet.

---

## Latest Routing Research (2025–2026)

### Router-R1: RL-Trained Multi-Round Router

Chen et al. (2025, arXiv:2506.09033, NeurIPS 2025). Router instantiated as a
capable LLM. Interleaves "think" actions (internal deliberation) with "route"
actions (dynamic model invocation). Integrates each response into evolving
context for multi-round routing. Open-sourced model weights on HuggingFace.

### xRouter: Cost-Aware RL Orchestration

Qian et al. (2025, arXiv:2510.08439, Salesforce). Built on Qwen2.5-7B,
selects among 20+ external LLMs. Cost-aware reward: no success yields no
reward; on success, cheaper is better. Reaches 80–90% of GPT-5 accuracy at
<1/5 the cost.

### IRT-Router: Psychometric Routing

Song et al. (2025, arXiv:2506.01048, ACL 2025). Borrows Item Response Theory
from psychometrics: each LLM is a "test-taker" with latent multidimensional
ability; each query is a "question" with latent difficulty. Superior in
cold-start scenarios across 20 LLMs and 12 datasets.

### BEST-Route: Test-Time Compute Allocation

Ding et al. (2025, arXiv:2506.22716, ICML 2025). Selects both the **model**
and the **number of responses to sample** based on query difficulty. For
small models, generating multiple responses and selecting the best can enhance
quality while remaining cheaper than a single large-model response. Up to
60% cost reduction with <1% performance degradation. Open-sourced by Microsoft.

### Prefill-Based Routing

Varshney & Surla (2026, arXiv:2603.20895). Uses LLM internal activations
during prefill as predictive signal for model correctness. The prefill
computation already happens, so routing overhead is near-zero. Can approximate
closed-source model capabilities using open-weights encoders.

### Per-Query Difficulty Estimation

RADAR (arXiv:2509.25426, 2025) uses Item Response Theory to jointly model
query difficulty and configuration ability. Routes queries with higher
difficulty to model-budget pairs with higher ability. Formulates selection
as multi-objective optimization at the Pareto frontier. Matches 90% of
o4-mini performance at 10% of cost on out-of-domain queries.

---

## Citations

1. Kahneman, D. (2011). "Thinking, Fast and Slow." — Dual-process theory.
2. De Neys, W. & Pennycook, G. (2019). "Logic, Fast and Slow: Advances in
   Dual-Process Theorizing." Current Directions in Psychological Science.
   — Competing intuitions, activation strength.
3. De Neys, W. (Ed.) (2018). "Dual Process Theory 2.0." Routledge. — Revised
   framework.
4. Evans, J. (2019). "Type 3" metacognitive process concept. — Triple-process
   theory.
5. Li, L. et al. (2010). "A contextual-bandit approach to personalized news
   article recommendation." WWW 2010. — LinUCB algorithm.
6. Chen, L. et al. (2023). "FrugalGPT." — Cascade routing.
7. Friston, K. (2010). "The free-energy principle: a unified brain theory?"
   — Active inference basis.
8. Chen, Z. et al. (2025). "Router-R1: Teaching LLMs Multi-Round Routing via
   RL." NeurIPS 2025. arXiv:2506.09033. — RL-trained router.
9. Qian, C. et al. (2025). "xRouter: Cost-Aware LLMs Orchestration via RL."
   Salesforce. arXiv:2510.08439. — 80–90% GPT-5 accuracy at <1/5 cost.
10. Ong, I. et al. (2025). "RouteLLM: Learning to Route LLMs with Preference
    Data." ICLR 2025. arXiv:2406.18665. — 85% cost reduction on MT Bench.
11. Dekoninck, J. et al. (2025). "A Unified Approach to Routing and Cascading."
    ICLR 2025. arXiv:2410.10347. — +14% on SWE-Bench.
12. arXiv:2505.12601 (2025). "Rethinking Predictive Modeling for LLM Routing:
    When Simple kNN Beats Complex Learned Routers." — kNN competitive.
13. Song, J. et al. (2025). "IRT-Router." ACL 2025. arXiv:2506.01048.
    — Psychometric routing.
14. Ding, Y. et al. (2025). "BEST-Route." ICML 2025. arXiv:2506.22716.
    — Test-time compute allocation. 60% cost reduction.
15. Zhou, Y. et al. (2022). "Mixture-of-Experts with Expert Choice Routing."
    arXiv:2202.09368. — Expert choice routing.
16. arXiv:2508.21141 (2025). "PILOT: Preference-Prior Informed LinUCB."
    — Offline priors + online bandits.
17. arXiv:2509.25426 (2025). "RADAR." — IRT + multi-objective optimization.
18. Varshney, T. & Surla, A. (2026). "LLM Router: Prefill is All You Need."
    arXiv:2603.20895. — Near-zero overhead routing.
19. Implementation plans `modelrouting/2G.10` through `modelrouting/2G.17`.
20. Implementation plan `modelrouting/11-research-context.md`.
21. Refactoring PRD §02-five-layers — Dual-Process Tier Router specification.
22. `.roko/learn/cascade-router.json` — Persisted routing state.
