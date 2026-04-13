# Six Knowledge Types

> Every knowledge entry in Neuro is classified into one of six semantic categories, each with a distinct half-life, retrieval behavior, and role in the agent's cognitive lifecycle.


> **Implementation**: Built

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [00-vision-and-grimoire-rename.md](./00-vision-and-grimoire-rename.md) for Neuro context
**Key sources**:
- `refactoring-prd/03-cognitive-subsystems.md` §1 (Six Knowledge Types table)
- `bardo-backup/prd/04-memory/01-grimoire.md` (original 5 canonical types)
- `bardo-backup/prd/04-memory/01b-grimoire-memetic.md` (AntiKnowledge as 6th type)
- `bardo-backup/tmp/agent-chain/05-knowledge-layer.md` (InsightEntry schema, lifecycle)
- `crates/roko-neuro/src/lib.rs` (current KnowledgeKind enum, KnowledgeEntry struct)

---

## Abstract

Knowledge entries are the fundamental units of Neuro's persistent memory. Unlike raw episode logs that record every turn of every conversation, knowledge entries are **distilled, classified, and typed** — each entry represents a single piece of reusable understanding that the agent has extracted from experience.

The type system is critical because different kinds of knowledge behave differently. An Insight ("Rust's borrow checker errors often mean you need Arc here") needs regular revalidation — it has a 30-day base half-life. A Warning ("Never use `unwrap()` in production paths") must be aggressively current — it has a 7-day base half-life and degrades quickly if not reconfirmed. AntiKnowledge ("Moving to async doesn't always improve throughput") never fully decays — it has a confidence floor of 0.3 because knowing what is false is permanently valuable.

The six types emerged from two design lineages: the original Grimoire (now Neuro) design specified five canonical types (Insight, Heuristic, Warning, CausalLink, StrategyFragment), and the memetic evolution extension added AntiKnowledge as a sixth type to represent knowledge about what is wrong. Together, these six types cover the full spectrum of what an agent can learn, from positive observations through causal models to defensive knowledge about known pitfalls.

---

## The Six Types

### 1. Insight

**Definition**: A validated observation — a compact causal or correlational statement distilled from one or more episodes, treated as true until contradicted by evidence.

**Base half-life**: 30 days. Observations need regular revalidation because the world changes. An insight about API behavior may become stale when the API is updated. An insight about code patterns may become irrelevant after a refactor.

**Coding domain examples**:
- "Rust's borrow checker errors often mean you need `Arc` here"
- "The `roko-gate` crate's test suite is sensitive to timing; use `tokio::time::pause()` to avoid flaky tests"
- "When `cargo clippy` warns about needless `clone()`, the fix is usually to take a reference instead"

**Chain domain examples**:
- "ETH gas spikes correlate with NFT mints"
- "Uniswap V3 concentrated liquidity positions need rebalancing when price moves >5% from the center tick"
- "MEV bots front-run large DEX trades with a median latency of 12ms on Ethereum mainnet"

**Research domain examples**:
- "Academic papers published on arXiv before peer review have a higher retraction rate for biomedical topics than for CS topics"
- "GPT-4-level models consistently underperform on multi-step arithmetic despite strong language understanding"

**Retrieval behavior**: Standard confidence-weighted retrieval. Insights are the most common knowledge type — they form the bulk of a mature agent's NeuroStore.

**Promotion path**: An Insight that proves useful across 5+ episodes and achieves ≥0.7 confidence can be promoted to a Heuristic by the tier progression pipeline (see [12-4-tier-distillation-pipeline.md](./12-4-tier-distillation-pipeline.md)).

### 2. Heuristic

**Definition**: A reusable rule of thumb — a compact, actionable pattern that the agent can apply directly without further reasoning. Heuristics are more durable than insights because they encode generalized patterns rather than specific observations.

**Base half-life**: 90 days. Rules of thumb are more durable than observations because they represent patterns that have been validated across multiple contexts. A heuristic that works for 90 days is likely capturing a genuine regularity.

**Coding domain examples**:
- "Always run clippy before committing"
- "When a function exceeds 50 lines, split it into smaller functions"
- "Use `#[must_use]` on functions that return `Result` or `Option`"
- "For async Rust code, prefer `tokio::spawn` over `std::thread::spawn`"

**Chain domain examples**:
- "Set slippage >1% during high volatility"
- "Never swap >5% of pool depth in a single transaction"
- "Rebalance yield farming positions on >5% drift from target allocation"
- "Monitor mempool for pending large transactions before executing swaps"

**Research domain examples**:
- "Cross-reference at least 3 independent sources before accepting a claim"
- "Prefer primary sources (original papers) over secondary sources (blog posts, summaries)"

**Retrieval behavior**: Heuristics are retrieved with higher priority than Insights when the agent is in a high-confidence, execution-focused behavioral state (Daimon state: Focused or Coasting). They serve as fast System 1 shortcuts that skip detailed reasoning.

**Origin**: Heuristics are produced by the D2 stage of the tier progression pipeline — they emerge from clusters of 5+ related Insights that share a common pattern with ≥0.7 confidence.

### 3. Warning

**Definition**: A known pitfall — a specific danger signal that the agent should watch for and avoid. Warnings are the most aggressive knowledge type: they have short half-lives because danger signals must be current.

**Base half-life**: 7 days. Danger signals must be fresh. A security vulnerability warning from two months ago may have been patched. A gas price warning from last week may no longer reflect current network conditions. If a warning is still relevant, it will be reconfirmed by ongoing experience and its confidence will stay high.

**Coding domain examples**:
- "Never use `unwrap()` in production paths"
- "The `chrono` crate has known issues with time zone handling on Windows; use `time` instead"
- "Do not call `std::process::exit()` in library code — it prevents cleanup"
- "The `reqwest` default timeout is 30 seconds, which is too long for most API calls"

**Chain domain examples**:
- "Never swap >5% of pool depth"
- "Avoid interacting with contracts that have been deployed for less than 24 hours"
- "The Curve stETH/ETH pool can depeg during high withdrawal demand"
- "Flash loan attacks frequently target oracle contracts with single-source price feeds"

**Operations domain examples**:
- "The staging environment's database has a 100-connection limit; exceeding it silently drops queries"
- "Never run migrations on the production database during peak hours (9am-5pm UTC)"

**Retrieval behavior**: Warnings receive a retrieval boost when the agent's Daimon state shows high arousal (urgency) or low dominance (uncertainty). The somatic landscape (see [13-somatic-integration.md](./13-somatic-integration.md)) gives warnings negative valence markers, causing them to surface during pre-action safety checks.

**Interaction with AntiKnowledge**: Warnings describe what to avoid ("never do X"). AntiKnowledge describes what is false ("X seems true but isn't"). The distinction is between prescriptive (Warning) and descriptive (AntiKnowledge) negative knowledge.

### 4. CausalLink

**Definition**: A cause-effect relationship — a structured observation that one phenomenon reliably leads to another. CausalLinks encode directional relationships using HDC permutation to distinguish cause from effect.

**Base half-life**: 60 days. Causal relationships need periodic confirmation because underlying mechanisms can change. A causal link between "large buy order → price impact → arbitrage opportunity" may weaken if market microstructure changes.

**Coding domain examples**:
- "Increasing thread pool size → reduced I/O latency (up to CPU core count)"
- "Adding `#[inline]` to hot-path functions → 5-15% throughput improvement in tight loops"
- "Removing `Box<dyn Error>` in favor of concrete error types → reduced allocation overhead"
- "Enabling LTO (link-time optimization) → 10-30% binary size reduction but 2-5x longer compile time"

**Chain domain examples**:
- "Large buy order → price impact → arbitrage opportunity"
- "ETH staking queue length increase → delayed validator activation → staking APY increase"
- "High gas base fee → user migration to L2 → L2 TVL growth"

**Research domain examples**:
- "Increased citation count for a paper → higher probability of replication success"
- "Multi-author papers from diverse institutions → lower retraction rate"

**HDC encoding**: CausalLinks use the permute operation to encode directionality:
```
causal_vector = PERM(cause_vector, 1) XOR PERM(effect_vector, 2)
```
This ensures that `CAUSE → EFFECT` is distinguishable from `EFFECT → CAUSE` in the HDC space. The permutation shifts encode the role (cause at position 1, effect at position 2).

**Retrieval behavior**: CausalLinks are retrieved when the agent encounters a situation that matches either the cause or the effect. If the agent detects a "large buy order" (matching the cause), the CausalLink surfaces the predicted effect ("price impact → arbitrage opportunity"). If the agent observes "arbitrage activity" (matching the effect), the CausalLink surfaces possible causes.

**Research basis**: Pearl's Structural Causal Models (Pearl 2000, 2009) provide the theoretical foundation for encoding directional causal relationships. The HDC permutation encoding is an efficient computational approximation of a causal graph edge.

### 5. StrategyFragment

**Definition**: A partial strategy for a problem class — a multi-step action pattern or recipe that the agent can apply or adapt to similar situations. StrategyFragments are more complex than Heuristics (which are single rules) but less complete than Playbooks (which are compiled from multiple heuristics and strategies).

**Base half-life**: 14 days. Strategies are context-dependent — they depend on current tool versions, API behaviors, market conditions, and other environmental factors that change frequently. A strategy that worked two weeks ago may need adaptation.

**Coding domain examples**:
- "Rate-limited APIs: exponential backoff + jitter + circuit breaker"
- "Rust async debugging: 1) Check for `Send` bound violations 2) Look for held-across-await locks 3) Check for recursive async calls 4) Use `tokio::runtime::Builder::enable_all()` in tests"
- "Large refactor procedure: 1) Create comprehensive test coverage 2) Extract interfaces 3) Implement new code behind feature flag 4) Migrate callers 5) Remove old code"

**Chain domain examples**:
- "Yield farming: compound daily, harvest weekly, rebalance on >5% drift"
- "Token launch analysis: 1) Check contract source verification 2) Analyze holder distribution 3) Check liquidity lock duration 4) Monitor initial trading volume 5) Wait 48h before entering position"
- "MEV protection: 1) Use Flashbots Protect for submission 2) Set tight slippage 3) Split large orders 4) Time transactions for low-mempool periods"

**Research domain examples**:
- "Literature review: 1) Seed search with 3-5 known papers 2) Snowball via citations 3) Cross-reference with survey papers 4) Check for contradictions 5) Synthesize into structured summary"

**Retrieval behavior**: StrategyFragments are retrieved when the agent's current task matches the problem class. They serve as starting templates that the agent can adapt rather than reasoning from scratch.

**Relationship to Playbooks**: When the tier progression pipeline detects clusters of related StrategyFragments and Heuristics that consistently succeed together, it compiles them into a PLAYBOOK.md file (see [12-4-tier-distillation-pipeline.md](./12-4-tier-distillation-pipeline.md)). Playbooks are the highest tier of procedural knowledge.

### 6. AntiKnowledge

**Definition**: Things that seem true but are not — validated negative knowledge about common misconceptions, failed approaches, and debunked beliefs. AntiKnowledge is the epistemic immune system of the agent.

**Base half-life**: Never (confidence floor of 0.3). Known unknowns are always valuable. An agent that forgets what it has learned is wrong will re-discover and re-try failed approaches, wasting resources. AntiKnowledge persists indefinitely, though its confidence can decay to the floor of 0.3 — it never reaches zero.

**Coding domain examples**:
- "Moving to async doesn't always improve throughput" (false: async helps I/O-bound workloads but hurts CPU-bound ones)
- "Using `Rc` instead of `Arc` is always faster" (false: on single-threaded code yes, but introducing `Rc` in code that may later become multi-threaded creates tech debt)
- "More tests always means better quality" (false: poorly written tests can give false confidence)

**Chain domain examples**:
- "Higher APY doesn't mean higher risk-adjusted returns" (false: high APY often correlates with higher impermanent loss or smart contract risk)
- "DEX arbitrage is always profitable" (false: gas costs, MEV competition, and slippage can make arbitrage opportunities negative-EV)
- "Stablecoin pools are risk-free" (false: depeg events, smart contract risk, and regulatory risk exist)

**Research domain examples**:
- "Papers with more citations are more reliable" (false: citation count correlates with impact, not necessarily correctness)
- "Peer review guarantees correctness" (false: peer review reduces errors but does not eliminate them)

**The Challenge Mechanism**: AntiKnowledge entries carry two special fields not present on other types:
- `refuted_insight_id`: The ID of the Insight or Heuristic that this AntiKnowledge entry refutes
- `refutation_evidence`: Evidence explaining why the refuted entry was wrong

When an AntiKnowledge entry is created, it generates a **refutation warning** that is attached to the original entry:

```rust
// From roko-neuro/src/lib.rs
impl KnowledgeEntry {
    pub fn refutation_warning(&self) -> Option<String> {
        if self.kind != KnowledgeKind::AntiKnowledge {
            return None;
        }
        let refuted_id = self.refuted_insight_id.as_deref()?.trim();
        if refuted_id.is_empty() {
            return None;
        }
        let evidence = self
            .refutation_evidence
            .as_deref()
            .unwrap_or(self.content.as_str())
            .trim()
            .trim_end_matches(|ch| matches!(ch, '.' | '!' | '?'));
        if evidence.is_empty() {
            return None;
        }
        Some(format!(
            "Previous insight {refuted_id} was wrong because {evidence}."
        ))
    }
}
```

This mechanism ensures that when an agent retrieves an Insight that has been challenged, the AntiKnowledge warning is surfaced alongside it. The agent sees both the original claim and the evidence against it, enabling informed decision-making rather than blind trust.

**Retrieval behavior**: AntiKnowledge entries are retrieved in two contexts:
1. **Proactive**: When the agent retrieves knowledge entries for a task, AntiKnowledge entries matching the query are included alongside positive entries. This prevents the agent from acting on known-false beliefs.
2. **Reactive**: When a new candidate Insight enters the knowledge base, it is checked against existing AntiKnowledge entries. If the candidate matches (high HDC similarity to a refuted claim), it is flagged for review before being accepted.

**Memetic evolution context**: The AntiKnowledge type was introduced in the memetic evolution extension of the original Grimoire (now Neuro) design (`bardo-backup/prd/04-memory/01b-grimoire-memetic.md`). In the Dawkinsian replicator model used for knowledge base health diagnostics (where `W(E) = f × r × L` — fidelity × fecundity × longevity), AntiKnowledge entries serve as the **immune system** that prevents epistemic parasites (entries with high fitness but negative actual decision quality) from proliferating.

The original specification defined AntiKnowledge with the following properties:
- Confidence floor of 0.3 (never decays below this)
- 0.5× demurrage rate (decays at half the normal rate on-chain)
- Automatically generated when an existing entry is contradicted by strong evidence
- Requires `refuted_insight_id` and `refutation_evidence` fields

---

## Type-to-Code Mapping

### Current Implementation (`roko-neuro/src/lib.rs`)

The current codebase uses a `KnowledgeKind` enum with **seven** variants:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeKind {
    /// A declarative statement that is treated as true until contradicted.
    Fact,
    /// A compact causal observation distilled from multiple raw episodes.
    Insight,
    /// A step-by-step action pattern or recipe.
    Procedure,
    /// A lightweight rule of thumb or learned tendency.
    Heuristic,
    /// A compiled human-readable playbook of validated heuristics.
    Playbook,
    /// A hard restriction that should not be violated.
    Constraint,
    /// Negative knowledge describing what to avoid or what has failed.
    AntiKnowledge,
}
```

### Reconciliation with the Six-Type Design

The refactoring-prd specifies six types: Insight, Heuristic, Warning, CausalLink, StrategyFragment, AntiKnowledge. The current code has seven variants: Fact, Insight, Procedure, Heuristic, Playbook, Constraint, AntiKnowledge. The mapping is:

| Refactoring-PRD Type | Current Code Variant | Notes |
|---|---|---|
| **Insight** | `Insight` | Direct match |
| **Heuristic** | `Heuristic` | Direct match |
| **Warning** | `Constraint` | Partially overlaps; `Constraint` is "a hard restriction" which covers Warning semantics |
| **CausalLink** | (not present) | Needs to be added |
| **StrategyFragment** | `Procedure` | Partially overlaps; `Procedure` is "a step-by-step action pattern" |
| **AntiKnowledge** | `AntiKnowledge` | Direct match |
| (not in PRD) | `Fact` | Exists in code but not in the 6-type design; serves as a default type |
| (not in PRD) | `Playbook` | Exists in code as the compiled output of tier progression |

**Reconciliation decision** (per `12a-cognitive-layer.md` tasks D1-D4): The recommended approach is to expand the enum to include all types from both sources:
- Keep `Fact` (established general-purpose type with 365-day half-life)
- Keep `Insight` (30-day half-life)
- Keep `Heuristic` (90-day half-life)
- Add `Warning` (7-day half-life, distinct from `Constraint`)
- Add `CausalLink` (60-day half-life, with HDC permute encoding)
- Rename `Procedure` to `StrategyFragment` or keep both (14-day half-life)
- Keep `Playbook` (compiled output, not a raw knowledge type)
- Keep `Constraint` (hard restrictions, infinite half-life)
- Keep `AntiKnowledge` (confidence floor 0.3)

### The KnowledgeEntry Struct

```rust
// From roko-neuro/src/lib.rs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    /// Unique identifier for the knowledge item.
    #[serde(default)]
    pub id: String,
    /// Knowledge category.
    #[serde(default)]
    pub kind: KnowledgeKind,
    /// Provenance label for the entry, if it came from a dedicated source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The actual knowledge content.
    #[serde(default)]
    pub content: String,
    /// Confidence score in the range `0.0..=1.0`.
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    /// Signed retrieval weight for the entry.
    #[serde(default = "default_confidence_weight")]
    pub confidence_weight: f64,
    /// ID of the insight this entry refutes, if it is AntiKnowledge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refuted_insight_id: Option<String>,
    /// Evidence explaining why the refuted insight was wrong.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refutation_evidence: Option<String>,
    /// Episode IDs that contributed to this knowledge.
    #[serde(default)]
    pub source_episodes: Vec<String>,
    /// Topic tags used for retrieval and filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Creation timestamp for the knowledge entry.
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    /// Exponential decay half-life in days.
    #[serde(default = "default_half_life_days")]
    pub half_life_days: f64,
    /// Optional HDC fingerprint for similarity search.
    #[serde(default)]
    pub hdc_vector: Option<Vec<u8>>,
}
```

Key fields by role:

| Field | Purpose |
|---|---|
| `id` | Unique identifier for cross-referencing and challenge tracking |
| `kind` | Semantic category — determines base half-life and retrieval behavior |
| `source` | Provenance label (e.g., "distilled from episode 2024-03-15T14:22:00Z") |
| `content` | The actual knowledge text |
| `confidence` | 0.0–1.0 score reflecting accumulated evidence for/against |
| `confidence_weight` | Signed retrieval weight — can be negative for demoted entries |
| `refuted_insight_id` | AntiKnowledge-specific: which entry this refutes |
| `refutation_evidence` | AntiKnowledge-specific: why the refuted entry was wrong |
| `source_episodes` | Provenance chain — which episodes contributed to this knowledge |
| `tags` | Topic tags for filtering (e.g., `["rust", "borrow-checker", "async"]`) |
| `created_at` | Creation timestamp for decay computation |
| `half_life_days` | Exponential decay half-life (set from type default, modifiable by tier) |
| `hdc_vector` | Optional 10,240-bit BSC vector for similarity search |

---

## Half-Life Summary Table

| Type | Base Half-Life | Rationale | Confidence Floor |
|---|---|---|---|
| **Fact** | 365 days | Established facts change slowly | None (standard decay) |
| **Insight** | 30 days | Observations need regular revalidation | None |
| **Heuristic** | 90 days | Rules of thumb are more durable | None |
| **Warning** | 7 days | Danger signals must be current | None |
| **CausalLink** | 60 days | Causal relationships need confirmation | None |
| **StrategyFragment** | 14 days | Strategies are context-dependent | None |
| **AntiKnowledge** | Never | Known unknowns are always valuable | 0.3 |

These base half-lives are multiplied by the tier multiplier to produce the effective half-life. See [03-type-half-lives.md](./03-type-half-lives.md) for detailed rationale and [07-ebbinghaus-decay-with-tier.md](./07-ebbinghaus-decay-with-tier.md) for the full decay formula.

---

## Domain-Agnostic Design

The six knowledge types are **domain-agnostic** — they apply equally to any problem domain. The types describe the *structure* of knowledge (observation, rule, warning, cause-effect, procedure, negation), not the *content*. Domain-specific behavior comes from:

1. **Content**: A coding agent's Insights are about code; a chain agent's Insights are about markets. The `content` field is free-text.
2. **Tags**: Domain-specific tags (`["rust", "async"]`, `["defi", "uniswap"]`) enable domain-filtered retrieval.
3. **HDC encoding**: Role vectors in the HDC encoding are domain-configurable. The `roko-index/src/hdc.rs` crate defines role vectors for code symbols (`SymbolKind::Function`, `SymbolKind::Struct`, etc.). Chain domain would define its own role vectors (`AssetType::Token`, `Protocol::UniswapV3`, etc.).
4. **Somatic landscape axes**: The 8-dimensional strategy space (see [13-somatic-integration.md](./13-somatic-integration.md)) is configured per domain. Coding agents use `[complexity, risk, novelty, confidence, time_pressure, scope, reversibility, dependency_depth]`. Chain agents use `[volatility, exposure, liquidity, correlation, leverage, time_horizon, slippage_risk, counterparty_risk]`.

---

## Academic Foundations

- Pearl, J. (2000). *Causality: Models, Reasoning, and Inference*. Cambridge University Press.
- Pearl, J. (2009). *Causal inference in statistics: An overview.* Statistics Surveys, 3, 96–146.
- Dawkins, R. (1976). *The Selfish Gene*. Oxford University Press. (Memetic replicator model for knowledge fitness)
- Damasio, A. R. (1994). *Descartes' Error: Emotion, Reason, and the Human Brain*. Putnam. (Somatic markers for Warnings)
- Bower, G. H. (1981). "Mood and Memory." *American Psychologist*, 36(2), 129–148. (15% contrarian retrieval)
- Kahneman, D. (2011). *Thinking, Fast and Slow*. Farrar, Straus and Giroux. (System 1/System 2 — Heuristics as System 1 shortcuts)
- Kleyko, D., Rachkovskij, D. A., Osipov, E., & Rahimi, A. (2022). "A Survey on Hyperdimensional Computing." *ACM Computing Surveys*, 54(6). (HDC encoding for CausalLinks)

---

## Current Status and Gaps

**Implemented**:
- `KnowledgeKind` enum with 7 variants (Fact, Insight, Procedure, Heuristic, Playbook, Constraint, AntiKnowledge)
- `KnowledgeEntry` struct with all fields including `refuted_insight_id` and `refutation_evidence`
- `refutation_warning()` method for AntiKnowledge challenge display
- Half-life constants: `FACT_HALF_LIFE_DAYS = 365.0`, `INSIGHT_HALF_LIFE_DAYS = 30.0`, `HEURISTIC_HALF_LIFE_DAYS = 90.0`

**Missing**:
- `Warning` variant with 7-day half-life (currently approximated by `Constraint`)
- `CausalLink` variant with 60-day half-life and HDC permute encoding
- `StrategyFragment` variant with 14-day half-life (currently approximated by `Procedure`)
- Confidence floor enforcement for AntiKnowledge (the 0.3 floor is not yet enforced in GC)
- Domain-configurable HDC role vectors for non-code domains
- Reactive AntiKnowledge checking (new candidates checked against existing AntiKnowledge)

---

## Cross-References

- See [02-four-validation-tiers.md](./02-four-validation-tiers.md) for how tiers multiply base half-lives
- See [03-type-half-lives.md](./03-type-half-lives.md) for detailed half-life rationale
- See [06-hdc-knowledge-encoding.md](./06-hdc-knowledge-encoding.md) for how entries are encoded as HDC vectors
- See [11-antiknowledge-challenge.md](./11-antiknowledge-challenge.md) for the full AntiKnowledge challenge mechanism
- See [12-4-tier-distillation-pipeline.md](./12-4-tier-distillation-pipeline.md) for how types interact with the distillation pipeline
- See topic [05-learning](../05-learning/INDEX.md) for the episode → knowledge feedback loop
- See topic [00-architecture](../00-architecture/INDEX.md) for the Engram type that underlies all knowledge entries
