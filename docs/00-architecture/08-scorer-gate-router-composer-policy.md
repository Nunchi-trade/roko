# Scorer, Gate, Router, Composer, Policy — The Five Operational Traits

> **Abstract:** This document provides detailed specifications for the five non-Substrate
> Synapse traits. Each trait is described with its complete Rust signature, design rationale,
> key implementations, and role in the cognitive loop. The Substrate trait is covered
> separately in [07-substrate-trait.md](07-substrate-trait.md).


> **Implementation**: Shipping

---

## 1. Scorer — Rate Engrams

The Scorer trait rates Engrams along multi-dimensional axes (see
[03-score-7-axis-appraisal.md](03-score-7-axis-appraisal.md)). Scorers are pure functions
of `(Engram, Context)` — no side effects, no I/O, no state mutation.

### 1.1 Trait Signature

```rust
pub trait Scorer: Send + Sync {
    fn score(&self, signal: &Signal, ctx: &Context) -> Score;
    fn name(&self) -> &'static str { "unnamed_scorer" }
}
```

### 1.2 Key Implementations

| Scorer | What It Measures | Primary Axis |
|---|---|---|
| `RelevanceScorer` | Semantic match to current goal (via Context.goal) | confidence |
| `RecencyScorer` | How recent the Engram is | confidence (decreasing with age) |
| `ReputationScorer` | Author's historical track record | reputation |
| `CatalyticScorer` | How many downstream Engrams this enabled | utility |
| `KeywordOverlapScorer` | Keyword match between Engram and query | confidence |
| `ToolRelevanceScorer` | How relevant a tool is to the current task | confidence |
| `CompositeScorer` | Weighted combination of multiple Scorers | all axes |

### 1.3 Composition

Multiple Scorers compose into a pipeline via `CompositeScorer`, which applies weighted
combinations using Score arithmetic (addition for evidence aggregation, multiplication for
modifier application).

### 1.4 Scorer as Parameter

The Composer trait takes `&dyn Scorer` as an argument, enabling the same Composer to produce
different outputs based on which scoring function is injected. This is the key composition
point between L2 (Scaffold) scoring and L2 context assembly.

---

## 2. Gate — Verify Against Ground Truth

The Gate trait verifies Engrams against external reality. Gates are the bridge between the
agent's internal representations and the external world.

### 2.1 Trait Signature

```rust
#[async_trait]
pub trait Gate: Send + Sync {
    async fn verify(&self, signal: &Signal, ctx: &Context) -> Verdict;
    fn name(&self) -> &str;
}
```

### 2.2 Verdict Structure

```rust
pub struct Verdict {
    pub passed: bool,           // binary result
    pub reason: String,         // human-readable explanation
    pub gate: String,           // which gate rendered this
    pub score: f32,             // [0..1] numeric score
    pub detail: Option<String>, // stdout, error output, diagnostic
    pub test_count: Option<TestCount>, // structured test results
    pub error_digest: Option<String>,  // unique errors with file/line
    pub duration_ms: u64,       // wall-clock time
}
```

The `is_mostly_passing()` method classifies failed verdicts with high pass rates (>90% of
>20 tests passing) — useful for Policies that need to distinguish "a few tests regressed"
from "compilation is broken."

### 2.3 The 11-Gate Pipeline

The `roko-gate` crate implements a multi-rung gate pipeline:

| Rung | Gate | Async? | What It Checks |
|---|---|---|---|
| 1 | `CompileGate` | Yes | Does `cargo build` succeed? |
| 2 | `TestGate` | Yes | Does `cargo test` pass? |
| 3 | `ClippyGate` | Yes | Does `cargo clippy` pass? |
| 4 | `DiffGate` | Yes | Are file changes within expected bounds? |
| 5 | `FormatGate` | Yes | Is `cargo fmt --check` clean? |
| 6 | `SchemaGate` | No | Does the output match expected JSON schema? |
| 7 | `JudgeGate` | Yes | LLM-as-judge quality assessment |
| 8 | `SimulationGate` | Yes | Transaction simulation via `eth_call` |
| 9 | `BalanceGate` | Yes | Are balances within expected bounds? |
| 10 | `ContentGate` | No | Does content match expected patterns? |
| 11 | `CustomGate` | Yes | Domain-specific verification |

### 2.4 Adaptive Thresholds

Gate thresholds adapt via exponential moving average (EMA) per rung:

```
new_threshold = α × observed_pass_rate + (1 - α) × old_threshold
```

This prevents overly strict gates from blocking all progress and overly lenient gates from
missing regressions. Thresholds persist in `.roko/learn/gate-thresholds.json`.

---

## 3. Router — Select Among Alternatives

The Router trait selects one Engram from a set of candidates and learns from outcomes.

### 3.1 Trait Signature

```rust
pub trait Router: Send + Sync {
    fn select(&self, candidates: &[Signal], ctx: &Context) -> Option<Selection>;
    fn feedback(&self, outcome: &Outcome);
    fn name(&self) -> &str;
}
```

### 3.2 The feedback() Learning Loop

The `feedback()` method is what makes Routers self-improving. After a selection is acted
upon, the Outcome (success, reward, cost, latency) updates the Router's internal model:

```
select(candidates, ctx) → Selection
    ↓ (act on selection)
outcome observed → Outcome { success, reward, cost, latency_ms }
    ↓
feedback(outcome) → internal model updated
    ↓ (next time)
select(candidates, ctx) → better Selection
```

### 3.3 Key Implementations

| Router | Algorithm | Learns From |
|---|---|---|
| `StaticRouter` | Deterministic (config-driven) | Nothing — fixed |
| `LinUCBRouter` | Contextual bandit (LinUCB) | Reward + context features |
| `CascadeRouter` | Multi-stage: confidence → UCB → cost | Reward + cost |
| `WeightedRouter` | Softmax over Scorer outputs | Scorer weights (fixed) |
| `EpsilonGreedyBandit` | ε-greedy with decay | Reward |

### 3.4 CascadeRouter Detail

The CascadeRouter implements the FrugalGPT cascade pattern (Chen et al. 2023,
arXiv:2305.05176):

1. **Confidence check**: If the cheap model's confidence exceeds threshold, use it
2. **UCB exploration**: If uncertain, use LinUCB to select among models
3. **Cost-aware**: Factor in token cost and latency to the reward signal

The CascadeRouter persists its state to `.roko/learn/cascade-router.json` and improves with
every task completion.

---

## 4. Composer — Combine Under Budget

The Composer trait assembles multiple Engrams into a single new Engram under resource
constraints.

### 4.1 Trait Signature

```rust
pub trait Composer: Send + Sync {
    fn compose(
        &self,
        signals: &[Signal],
        budget: &Budget,
        scorer: &dyn Scorer,
        ctx: &Context,
    ) -> Result<Signal>;

    fn name(&self) -> &str;
}
```

### 4.2 Budget Constraints

```rust
pub struct Budget {
    pub max_tokens: Option<usize>,   // token count cap
    pub max_signals: Option<usize>,  // input count cap
    pub max_bytes: Option<usize>,    // output size cap
    pub max_wall_ms: Option<u64>,    // wall-clock time cap
}
```

### 4.3 Key Implementations

| Composer | What It Assembles | Budget Dimensions |
|---|---|---|
| `PromptComposer` | Prompt sections → complete prompt | max_tokens |
| `ContextComposer` | Code files + docs → context pack | max_tokens, max_signals |
| `SystemPromptBuilder` | 6-layer role templates → system prompt | max_tokens |
| `PlanComposer` | Task descriptions → execution plan | max_signals |

### 4.4 The SystemPromptBuilder

The most complex Composer implementation. Assembles system prompts from six layers:

1. **Role description** — what the agent is
2. **Task context** — what it's working on
3. **Tool inventory** — what tools are available
4. **Safety rules** — what it must not do
5. **Knowledge** — relevant heuristics and rules from Neuro
6. **History** — recent episode summaries

Each layer is scored for relevance and included under the token budget. The builder uses
templates from `roko-compose/src/templates/` specific to each agent role.

---

## 5. Policy — React to Streams

The Policy trait watches Engram streams and produces reactive interventions.

### 5.1 Trait Signature

```rust
pub trait Policy: Send + Sync {
    fn decide(&self, stream: &[Signal], ctx: &Context) -> Vec<Signal>;
    fn name(&self) -> &str;
}
```

### 5.2 Key Implementations

| Policy | Trigger | Emits |
|---|---|---|
| `CircuitBreakerPolicy` | Repeated gate failures | Pause/stop Engrams |
| `EpisodeLogPolicy` | Agent turn completion | Episode Engrams |
| `PheromonePolicy` | Market state changes | Pheromone Engrams |
| `HeartbeatPolicy` | Timer tick | Heartbeat metric Engrams |
| `AlertPolicy` | Tool health degradation | ToolHealthDegraded Engrams |
| `EfficiencyPolicy` | Turn completion | Efficiency metric Engrams |
| `ConductorPolicy` | Multiple signals | Conductor decisions |

### 5.3 Policy in the Loop

Policy runs in step 8 (ADAPT) of the cognitive loop — after the gate verdict, after
persistence, the Policy examines what just happened and emits reactions. Reactions are
themselves Engrams, stored in the Substrate, and visible to subsequent loop ticks.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Chen et al. 2023 (arXiv:2305.05176) | FrugalGPT: cascade routing pattern. Foundation for CascadeRouter. |
| Li et al. 2010 | LinUCB: contextual bandit for recommendation. Algorithm behind LinUCBRouter. |
| Auer et al. 2002, Machine Learning 47(2-3) | UCB1: finite-time regret bounds for multi-armed bandits. Theoretical basis for exploration. |
| Scherer 2001, Applied AI 15 | Appraisal theory: multi-dimensional evaluation. Theoretical basis for Scorer design. |
| Ousterhout 2018, A Philosophy of Software Design | Deep modules with simple interfaces. Each trait is a deep module. |

---

## Current Status and Gaps

- **Scorer**: Implemented in `roko-std` (KeywordOverlapScorer, ToolRelevanceScorer) and
  `roko-learn` (FormatBandit as scoring-adjacent). CompositeScorer partially implemented.
- **Gate**: 11 gates in `roko-gate` (200 tests). Adaptive thresholds wired.
- **Router**: CascadeRouter, LinUCBRouter, EpsilonGreedyBandit in `roko-learn` (101 tests).
  StaticRouter in `roko-std`.
- **Composer**: SystemPromptBuilder in `roko-compose` (23 tests). PromptComposer in
  `roko-compose`.
- **Policy**: Efficiency events, episode logging wired in orchestrate.rs. Conductor policies
  in `roko-conductor`.

---

## Cross-References

- [06-synapse-traits.md](06-synapse-traits.md) — Trait overview
- [07-substrate-trait.md](07-substrate-trait.md) — Substrate in depth
- [09-universal-cognitive-loop.md](09-universal-cognitive-loop.md) — How these traits compose
