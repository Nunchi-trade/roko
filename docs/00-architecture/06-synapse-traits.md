# The Six Synapse Traits

> **Abstract:** The Synapse Architecture is built from six composable traits — the "verbs"
> that operate on Engrams. Every capability in Roko — agent spawning, gate verification,
> prompt assembly, model routing, memory retrieval, pheromone reaction, chain participation —
> is an implementation of one of these six traits. This document introduces the trait model,
> explains how traits compose across layers, and provides the complete Rust signatures from
> shipping code.


> **Implementation**: Shipping

---

## 1. The Composition Model

Classical agent frameworks define capabilities through inheritance hierarchies or plugin
registries. A "coding agent" might inherit from "BaseAgent" and override methods. A "gate
plugin" might register with a "gate manager." Each new capability requires understanding
the framework's extension points.

Roko takes a fundamentally different approach: **there are exactly six operations the system
can perform on Engrams**, and every capability is one of those six operations. The six
traits are:

| Trait | Role | Async? | Primary Layer |
|---|---|---|---|
| **Substrate** | Persist and query Engrams | async | L0 Runtime |
| **Scorer** | Rate Engrams along multiple axes | sync | L2 Scaffold |
| **Gate** | Check Engrams against ground truth (returns Verdict) | async | L3 Harness |
| **Router** | Choose best Engram from candidates (+ `feedback()`) | sync | L1 Framework |
| **Composer** | Combine Engrams under budget constraints (takes `&dyn Scorer`) | sync | L2 Scaffold |
| **Policy** | Observe Engram streams, emit new Engrams (batch input) | sync | L3-L4 |

These traits are **distributed across layers** — they do not all live at one level. The
architecture works because traits compose across layers rather than competing with them.

### 1.1 Why Six

The number six is not arbitrary. It emerged from analyzing the complete Roko design corpus
(400+ capabilities across coding agents, chain agents, verification pipelines, context
engineering, learning systems, and orchestration). Every capability, without exception,
reduces to one of these six operations:

1. **Store/retrieve** (Substrate) — every system needs persistence
2. **Evaluate** (Scorer) — every system needs quality assessment
3. **Verify** (Gate) — every system needs external truth checking
4. **Choose** (Router) — every system needs selection among alternatives
5. **Assemble** (Composer) — every system needs to combine information
6. **React** (Policy) — every system needs reactive behavior

This analysis is documented in detail in the original unified primitives design document
(see `roko-progress/12-unified-primitives.md`), which shows how tool dispatch, model routing,
context assembly, verification, conductor watchers, learning, chain participation, and
observability all map to exactly these six trait interfaces.

### 1.2 Why Traits (Not Objects, Not Functions)

Rust traits provide three properties essential to Roko's design:

1. **Static dispatch when possible**: Concrete trait implementations can be monomorphized
   by the compiler, eliminating virtual dispatch overhead for hot paths.
2. **Dynamic dispatch when needed**: `&dyn Trait` enables runtime composition — swapping
   implementations based on configuration or learned preferences.
3. **Bounds checking at compile time**: `Send + Sync` bounds on all traits ensure that
   implementations are safe for concurrent use, which is required because the three
   cognitive speeds (Gamma/Theta/Delta) run on separate async tasks.

---

## 2. Substrate — Store and Query

```rust
/// Stores and queries Engrams.
///
/// All storage backends implement this trait: MemorySubstrate (testing),
/// FileSubstrate (.roko/ persistence), HdcSubstrate (semantic search),
/// ChainSubstrate (shared on-chain state). They are API-identical from a
/// caller's perspective.
///
/// # Idempotence
/// put is idempotent for Engrams with identical content hashes.
///
/// # Concurrency
/// Substrates are Send + Sync. Impls must handle concurrent access.
#[async_trait]
pub trait Substrate: Send + Sync {
    /// Store an Engram. Returns its content hash. Idempotent on content.
    async fn put(&self, signal: Signal) -> Result<ContentHash>;

    /// Retrieve an Engram by content hash. Does not apply decay.
    async fn get(&self, id: &ContentHash) -> Result<Option<Signal>>;

    /// Query for Engrams matching the given filter. Impls may apply decay
    /// when evaluating min_weight and when ordering results.
    async fn query(&self, q: &Query, ctx: &Context) -> Result<Vec<Signal>>;

    /// Remove Engrams whose effective weight (score × decay) has fallen
    /// below threshold at ctx.now_ms. Returns count of pruned Engrams.
    async fn prune(&self, threshold: f32, ctx: &Context) -> Result<usize>;

    /// Optional: total count of stored Engrams (for metrics/health checks).
    async fn len(&self) -> Result<usize> { Ok(0) }

    /// Optional: is the substrate empty?
    async fn is_empty(&self) -> Result<bool> { Ok(self.len().await? == 0) }

    /// Human-readable name for logging/debugging.
    fn name(&self) -> &'static str { "unnamed_substrate" }
}
```

### 2.1 Why Async

Substrate operations are async because storage backends may involve I/O:
- **FileSubstrate** reads/writes JSONL files
- **ChainSubstrate** makes RPC calls to read on-chain state
- **HdcSubstrate** performs similarity search over HDC vectors
- **NetworkSubstrate** (future) queries remote Substrates via the Agent Mesh

Even `MemorySubstrate` (in-memory, used for testing) implements the async interface for
compatibility.

### 2.2 Implementations

| Implementation | Backend | Layer | Use Case |
|---|---|---|---|
| `MemorySubstrate` | In-memory HashMap | L0 | Testing, ephemeral storage |
| `FileSubstrate` | JSONL files in `.roko/` | L0 | Default persistence |
| `HdcSubstrate` | HDC vectors + Hamming search | L0 | Semantic similarity queries |
| `ChainSubstrate` | On-chain state via RPC | L1 | Shared agent state on Korai |

### 2.3 Idempotence

`put()` is idempotent: storing the same Engram twice (same ContentHash) is a no-op. This
is a direct consequence of content-addressed storage — the identity IS the content.

### 2.4 Pruning

`prune()` removes Engrams whose `weight_at(ctx.now_ms) < threshold`. This is how the
system implements automatic memory management. The threshold is configurable; typical values
are 0.01 (aggressive pruning) to 0.001 (conservative).

---

## 3. Scorer — Rate

```rust
/// Rates an Engram along multi-dimensional axes.
///
/// Scorers are pure functions of (Engram, Context). They compose freely:
/// use CompositeScorer to combine several scorers via +/× operations.
///
/// # Examples of Scorers
/// - RelevanceScorer: how well does this Engram match the current goal?
/// - RecencyScorer: how recent is this Engram?
/// - ReputationScorer: how trustworthy is its author?
/// - CatalyticScorer: how many downstream Engrams does this enable?
pub trait Scorer: Send + Sync {
    /// Score an Engram in the given context.
    fn score(&self, signal: &Signal, ctx: &Context) -> Score;

    /// Human-readable name.
    fn name(&self) -> &'static str { "unnamed_scorer" }
}
```

### 3.1 Why Sync

Scorers are synchronous because scoring should be fast — no I/O, no network calls, no
blocking. A Scorer is a pure function: given an Engram and a Context, it produces a Score.
This purity enables aggressive caching and parallelization.

### 3.2 Composition

Multiple Scorers compose into a single scoring pipeline via `CompositeScorer`:

```rust
// Example: composite scoring pipeline
let scorer = CompositeScorer::new()
    .add(RelevanceScorer::new(), 0.4)   // 40% weight
    .add(RecencyScorer::new(), 0.3)     // 30% weight
    .add(ReputationScorer::new(), 0.3); // 30% weight
```

Individual Scorer outputs combine via Score addition (evidence aggregation) or
multiplication (modifier application), depending on the composition strategy.

### 3.3 Scorer as Argument

The Composer trait takes `&dyn Scorer` as a parameter, enabling the Composer to evaluate
candidates using whatever scoring function is appropriate for the context. This is a key
composition point: the same Composer can produce different outputs depending on which Scorer
is injected.

---

## 4. Gate — Verify

```rust
/// Verifies an Engram against ground truth, producing a Verdict.
///
/// Gates are the bridge to external reality: compile, run tests, simulate
/// transactions, check balances, validate schemas. A gate that returns
/// passed = true is a claim that the Engram is correct in some domain.
///
/// # Async by default
/// Gates typically invoke subprocesses, HTTP calls, or chain RPCs.
#[async_trait]
pub trait Gate: Send + Sync {
    /// Verify the Engram and return a verdict.
    async fn verify(&self, signal: &Signal, ctx: &Context) -> Verdict;

    /// Human-readable name (appears in verdicts).
    fn name(&self) -> &str;
}
```

### 4.1 Why Async

Gates invoke external processes:
- **CompileGate** runs `cargo build`
- **TestGate** runs `cargo test`
- **ClippyGate** runs `cargo clippy`
- **SimulationGate** simulates transactions via `eth_call`
- **DiffGate** checks that file changes are within expected bounds

All of these involve subprocess spawning and I/O, requiring async.

### 4.2 Verdict (Not Boolean)

Gates return a `Verdict`, not a boolean. The Verdict struct carries evidence:

```rust
pub struct Verdict {
    pub passed: bool,
    pub reason: String,
    pub gate: String,
    pub score: f32,           // [0..1] — for thresholding
    pub detail: Option<String>,
    pub test_count: Option<TestCount>,
    pub error_digest: Option<String>,
    pub duration_ms: u64,
}
```

This rich output enables downstream Policies to make intelligent decisions: retry with
different input, escalate to a human, adjust the approach based on specific failure modes.

### 4.3 Gate Pipeline

The `roko-gate` crate implements an 11-gate pipeline organized into 6 rungs:

| Rung | Gate | What It Checks |
|---|---|---|
| 1 | CompileGate | Does the code compile? |
| 2 | TestGate | Do tests pass? |
| 3 | ClippyGate | Lint warnings? |
| 4 | DiffGate | Are file changes within bounds? |
| 5 | FormatGate | Is the code formatted? |
| 6 | Custom gates | Domain-specific verification |

Gates have adaptive thresholds (EMA per rung) that adjust based on observed pass rates.

---

## 5. Router — Select

```rust
/// Selects one Engram from many candidates.
///
/// Routers are the decision-making layer: which model to call, which backend
/// to use, which gate to run next, which bounty to claim. They learn via
/// feedback() so they improve with experience.
///
/// # Implementations
/// - StaticRouter — deterministic choice (config-driven)
/// - LinUCBRouter — contextual bandit
/// - CascadeRouter — multi-stage confidence → UCB
/// - WeightedRouter — softmax over scorers
pub trait Router: Send + Sync {
    /// Select one Engram from candidates. None = no selection made.
    fn select(&self, candidates: &[Signal], ctx: &Context) -> Option<Selection>;

    /// Learn from a selection's actual outcome.
    fn feedback(&self, outcome: &Outcome);

    /// Human-readable name (appears in selections).
    fn name(&self) -> &str;
}
```

### 5.1 The feedback() Method

The `feedback()` method is what makes Routers learn. After a selection is acted upon, the
outcome (success/failure, reward, cost, latency) is fed back to the Router. This enables:

- **CascadeRouter**: Updates contextual bandit weights based on reward
- **LinUCBRouter**: Updates the LinUCB algorithm's confidence bounds
- **EpsilonGreedyBandit**: Updates arm reward estimates

The feedback loop is the core learning mechanism for model routing — the system improves its
model selection over time without any manual tuning.

### 5.2 Selection Output

```rust
pub struct Selection {
    pub chosen: ContentHash,
    pub confidence: f32,
    pub router: String,
    pub reasoning: Option<String>,
}
```

The Selection identifies which Engram was chosen, how confident the Router is in that choice,
and optionally why. This is logged for observability and used by the feedback loop.

### 5.3 Outcome Feedback

```rust
pub struct Outcome {
    pub selection: Selection,
    pub success: bool,
    pub reward: f32,
    pub cost: Option<f32>,
    pub latency_ms: Option<u64>,
}
```

Outcomes feed back into the Router. Cost-aware Routers (like CascadeRouter) use the cost
field to balance quality against expense, implementing the FrugalGPT cascade pattern
(Chen et al. 2023, arXiv:2305.05176).

---

## 6. Composer — Combine

```rust
/// Combines multiple Engrams into one new Engram under a Budget.
///
/// Composers are the assembly layer: prompts from sections, context packs
/// from fragments, transactions from operations, plans from tasks, bounties
/// from sub-bounties. Output respects budget constraints.
pub trait Composer: Send + Sync {
    /// Combine input Engrams into a new composed Engram.
    /// The composer may use the scorer to rank/select inputs under budget.
    fn compose(
        &self,
        signals: &[Signal],
        budget: &Budget,
        scorer: &dyn Scorer,
        ctx: &Context,
    ) -> Result<Signal>;

    /// Human-readable name.
    fn name(&self) -> &str;
}
```

### 6.1 Budget Constraints

```rust
pub struct Budget {
    pub max_tokens: Option<usize>,
    pub max_signals: Option<usize>,
    pub max_bytes: Option<usize>,
    pub max_wall_ms: Option<u64>,
}
```

Every Composer must respect the Budget. This is how Roko handles the fundamental constraint
of LLM context windows — the Composer selects which Engrams to include and which to drop
based on the Scorer's ranking, under the token/byte/count budget.

### 6.2 Scorer Injection

The Composer takes `&dyn Scorer` as a parameter, enabling different scoring strategies for
different composition tasks. A prompt Composer might use a RelevanceScorer to prioritize
context that matches the current goal, while a knowledge Composer might use a RecencyScorer
to prioritize fresh information.

### 6.3 Implementations

| Composer | What It Assembles |
|---|---|
| `PromptComposer` | Assembles prompt sections into a complete prompt under token budget |
| `ContextComposer` | Builds context packs from code files, documentation, and history |
| `SystemPromptBuilder` | 6-layer prompt assembly with role templates |
| `PlanComposer` | Combines task descriptions into execution plans |

---

## 7. Policy — React

```rust
/// Watches a stream of Engrams and emits new Engrams in response.
///
/// Policies are the reactive/behavioral layer: conductor watchers, circuit
/// breakers, episode logging, pheromone reactions, heartbeat emission,
/// promotion to chain, sentinel detection. They run continuously over the
/// Engram stream and may produce zero, one, or many output Engrams per tick.
pub trait Policy: Send + Sync {
    /// Examine the recent Engram stream and produce new Engrams (interventions).
    fn decide(&self, stream: &[Signal], ctx: &Context) -> Vec<Signal>;

    /// Human-readable name.
    fn name(&self) -> &str;
}
```

### 7.1 Batch Input

Unlike Scorers (which operate on a single Engram) or Gates (which verify a single Engram),
Policies receive a batch of recent Engrams — the "stream." This enables pattern detection
across multiple Engrams:

- **CircuitBreakerPolicy**: Monitors for repeated gate failures, emits a pause Engram
- **EpisodeLogPolicy**: Collects agent turns and gate results, emits Episode Engrams
- **PheromonePolicy**: Watches for market events, emits pheromone signals
- **HeartbeatPolicy**: Emits periodic heartbeat Engrams for health monitoring
- **AlertPolicy**: Watches for tool health degradation, emits ToolHealthDegraded Engrams

### 7.2 Zero to Many Outputs

A Policy may produce any number of output Engrams per invocation:
- **Zero**: Nothing notable happened in the stream
- **One**: A single intervention (e.g., circuit breaker trip)
- **Many**: Multiple reactions (e.g., episode log + metric update + alert)

---

## 8. Trait × Layer Map

Traits are distributed across the five architectural layers:

```
Layer 4: Orchestration  │ Policy (state machine transitions, plan reactions)
Layer 3: Harness        │ Gate (verification), Policy (conductor watchers)
Layer 2: Scaffold       │ Scorer (relevance), Composer (prompt assembly)
Layer 1: Framework      │ Router (model selection), Scorer (tool relevance)
Layer 0: Runtime        │ Substrate (persistence)
```

Key insight: the same trait can have implementations at different layers. A Scorer at L1
(tool relevance for dispatch) is different from a Scorer at L2 (context relevance for prompt
assembly), but both implement the same `Scorer` trait. This is how composition works across
layers.

---

## 9. Composability Example

A complete cognitive tick composes all six traits:

```rust
pub async fn loop_tick(
    substrate: &dyn Substrate,
    scorer: &dyn Scorer,
    router: &dyn Router,
    composer: &dyn Composer,
    gate: &dyn Gate,
    policy: &dyn Policy,
    query: &Query,
    budget: &Budget,
    ctx: &Context,
) -> Result<TickOutcome> {
    // 1. Query the substrate for candidates.
    let candidates = substrate.query(query, ctx).await?;

    // 2. Router selects one candidate.
    let Some(selection) = router.select(&candidates, ctx) else {
        return Ok(TickOutcome { /* empty */ });
    };

    // 3. Composer builds a new Engram from the selection.
    let chosen = candidates.iter().find(|s| s.id == selection.chosen).cloned();
    let composed = composer.compose(&[chosen.unwrap()], budget, scorer, ctx)?;

    // 4. Gate verifies the composition.
    let verdict = gate.verify(&composed, ctx).await;

    // 5. If passed, persist and run policy reaction.
    if verdict.passed {
        substrate.put(composed.clone()).await?;
        let reactions = policy.decide(std::slice::from_ref(&composed), ctx);
        for r in reactions {
            substrate.put(r).await?;
        }
    }

    Ok(TickOutcome { /* populated */ })
}
```

Different agents use different implementations of each trait, but the loop structure is
universal. This is the Synapse Architecture: one loop, six extension points, infinite
combinations.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Sumers et al. 2023 (arXiv:2309.02427) | CoALA: cognitive architecture components map to these six traits. |
| Gamma et al. 1994, Design Patterns | Strategy pattern: traits as interchangeable algorithms. |
| Ousterhout 2018, A Philosophy of Software Design | Deep modules: simple interfaces with powerful implementations. Each trait is a deep module. |
| Chen et al. 2023 (arXiv:2305.05176) | FrugalGPT: cascade routing as Router + feedback. |

---

## Current Status and Gaps

- **Implemented**: All six traits defined in `roko-core/src/traits.rs`. Concrete
  implementations in `roko-std` (96 tests). Universal loop in `roko-core/src/loop_tick.rs`.
- **Implemented**: CascadeRouter, LinUCBRouter, StaticRouter in `roko-learn`.
- **Implemented**: 11 gates in `roko-gate` with adaptive thresholds.
- **Implemented**: SystemPromptBuilder as a Composer in `roko-compose`.
- **Gap**: CompositeScorer for combining multiple Scorers (specified, partially implemented).
- **Gap**: ChainSubstrate for on-chain Engram storage (specified, not built).

---

## Cross-References

- [02-engram-data-type.md](02-engram-data-type.md) — The Engrams these traits operate on
- [07-substrate-trait.md](07-substrate-trait.md) — Substrate in depth
- [08-scorer-gate-router-composer-policy.md](08-scorer-gate-router-composer-policy.md) — Other five traits in depth
- [09-universal-cognitive-loop.md](09-universal-cognitive-loop.md) — How traits compose in the loop
- [12-five-layer-taxonomy.md](12-five-layer-taxonomy.md) — Which layer each trait lives at
