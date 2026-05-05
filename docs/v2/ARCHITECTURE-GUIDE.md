# Roko v2 Architecture Guide

**Who this is for**: Engineers reading the codebase for the first time. If you
already know the system well, jump to the section you need using the table of
contents. If this is your first time here, read the first three sections before
anything else.

---

## Table of Contents

1. [What is Roko?](#1-what-is-roko)
2. [Core Mental Model: 1 Noun + 9 Operations](#2-core-mental-model-1-noun--9-operations)
3. [The Universal Loop](#3-the-universal-loop)
4. [Crate Map](#4-crate-map)
5. [Protocol Traits (Core Six)](#5-protocol-traits-core-six)
6. [Foundation Service Traits](#6-foundation-service-traits)
7. [Supporting Protocol Traits](#7-supporting-protocol-traits)
8. [The Cell Supertrait](#8-the-cell-supertrait)
9. [WorkflowEngine and PipelineStateV2](#9-workflowengine-and-pipelinestatev2)
10. [EffectDriver Pattern](#10-effectdriver-pattern)
11. [Agent Dispatch and ToolDispatcher](#11-agent-dispatch-and-tooldispatcher)
12. [Gate Pipeline Architecture](#12-gate-pipeline-architecture)
13. [CascadeRouter: 3-Stage Model Selection](#13-cascaderouter-3-stage-model-selection)
14. [Learning Architecture](#14-learning-architecture)
15. [Knowledge Store Architecture](#15-knowledge-store-architecture)
16. [Dream Consolidation Cycle](#16-dream-consolidation-cycle)
17. [DaimonPolicy Affect Engine](#17-daimonpolicy-affect-engine)
18. [SystemPromptBuilder 9-Layer Assembly](#18-systempromptbuilder-9-layer-assembly)
19. [Conductor: Reactive Intelligence Layer](#19-conductor-reactive-intelligence-layer)
20. [RuntimeEvent System](#20-runtimeevent-system)
21. [HDC Primitives](#21-hdc-primitives)
22. [Safety Architecture](#22-safety-architecture)
23. [Extension System](#23-extension-system)
24. [Data Flow: Signal Through the System](#24-data-flow-signal-through-the-system)
25. [Appendix: Key File Index](#appendix-key-file-index)

---

## 1. What is Roko?

Roko is a Rust toolkit for building agents that build themselves.

The core idea: the same system that executes software tasks can also read
requirements, generate implementation plans, run agents to carry them out,
validate the results, learn from failures, and iterate — autonomously, in a
loop. The goal is a system sophisticated enough to develop itself.

Roko is not a chat wrapper or a thin LLM client. It is a full orchestration
runtime: 18 crates, a typed event bus, a multi-stage gate pipeline, a
self-improving model router, a durable knowledge store, and an affect engine
that adjusts agent behavior based on recent history. Every component exists
because the self-hosting loop needed it.

If you want to understand Roko, start with the question: "what does it take
for a system to reliably execute a software task, validate the result, and
improve over time?" Every architectural decision in this document is an answer
to that question.

---

## 2. Core Mental Model: 1 Noun + 9 Operations

The entire design reduces to one data type and nine operations:

```
Engram            -- the universal datum: addressable, decaying, scored, traced

  Store           -- persist and retrieve Engrams (FileSubstrate, HdcSubstrate, ChainSubstrate)
  Score           -- rate along multi-dimensional axes (relevance, recency, reputation)
  Verify          -- check against ground truth (compile, test, clippy, LLM-judge)
  Route           -- select one candidate from many (CascadeRouter, LinUCB, StaticRouter)
  Compose         -- combine under budget (PromptComposer, ContextAssembler)
  React           -- watch streams and emit interventions (Conductor watchers, Policy)
  Bus             -- publish/subscribe transport for ephemeral Pulses
  ColdStore       -- archival store for aged-out Engrams
  Observe/Connect/Trigger  -- peripheral protocol traits (Cell-based extensions)
```

### What is an Engram?

Think of an `Engram` like a Git commit for a piece of knowledge or an agent
output: it is content-addressed (its identity is a BLAKE3 hash of what it
contains), immutable once created, and it carries metadata about where it came
from and when.

Unlike a Git commit, an Engram also has:
- A **decay** curve (knowledge has a half-life; a `Warning` decays in hours, an
  `Insight` in weeks)
- A **score** (multi-dimensional: confidence, novelty, utility, reputation)
- An **emotional tag** (the affect state when it was created)
- **Lineage** (parent content hashes, forming an auditable DAG)
- An **HDC fingerprint** (a 10,240-bit vector for semantic similarity lookup)

Everything in Roko — every agent output, every gate verdict, every piece of
knowledge, every task definition — is an Engram. This uniformity is what makes
the universal loop possible.

<details>
<summary>Full Engram struct (roko-core/src/engram.rs)</summary>

```rust
pub struct Engram {
    /// Content-addressed identity (BLAKE3 hash of kind+body+author+tags).
    pub id: ContentHash,
    /// HDC fingerprint for semantic similarity lookup.
    pub fingerprint: Option<HdcFingerprint>,
    /// What kind of engram this is (Task, GateVerdict, Episode, Prompt, ...).
    pub kind: Kind,
    /// The payload (Text, Json, Binary, Empty).
    pub body: Body,
    /// Unix milliseconds when first emitted.
    pub created_at_ms: i64,
    /// How this engram's weight decays over time (None, HalfLife, Exponential).
    pub decay: Decay,
    /// Producer attribution and trust.
    pub provenance: Provenance,
    /// Quality score at emission time (confidence, novelty, utility, reputation).
    pub score: Score,
    /// ContentHashes of parent Engrams (forms a DAG for audit and C-factor metrics).
    pub lineage: Vec<ContentHash>,
    /// Arbitrary string metadata (BTreeMap for stable hashing).
    pub tags: BTreeMap<String, String>,
    /// Optional cryptographic proof of origin.
    pub attestation: Option<Attestation>,
    /// Optional PAD-based emotional metadata.
    pub emotional_tag: Option<EmotionalTag>,
}
```

**Identity and Hashing**

Content hash covers: `kind + body + author + taint + lineage + tags`. It does
NOT cover score, decay, timestamp, attestation, or emotional metadata — those
can change without changing what the engram fundamentally is.

```rust
pub fn content_hash(&self) -> ContentHash {
    // BLAKE3 over kind.identity_key() | body.canonical_bytes()
    //                | author | taint | lineage | tags(sorted)
}
```

**Effective Weight**

```rust
pub fn weight_at(&self, now_ms: i64) -> f32 {
    let age = now_ms - self.created_at_ms;
    self.score.effective() * self.decay.apply(age)
}
```

**Construction**

```rust
let engram = Engram::builder(Kind::Task)
    .body(Body::text("implement login"))
    .tag("priority", "high")
    .decay(Decay::HalfLife { half_life_ms: 86_400_000 })
    .build();
```

**HDC Operations**

Engrams support hyperdimensional computing operations when fingerprints are set:

```rust
pub fn bind(&self, other: &Engram) -> Option<HdcVector>        // XOR binding
pub fn bundle(engrams: &[Engram]) -> Option<HdcVector>         // majority bundle
pub fn at_position(&self, position: usize) -> Option<HdcVector> // positional permutation
```

</details>

### Pulse vs. Engram

Not everything needs to be persisted. `Pulse` is the ephemeral counterpart —
published on the `Bus` for immediate reactions. Pulses that are worth keeping
get promoted to `Engram`s via `Store::put`. Think of Pulses as in-flight
events and Engrams as the durable record.

```rust
pub fn from_pulse_synthetic(p: &Pulse) -> Self  // single pulse -> Engram
pub fn from_pulses(pulses: &[Pulse]) -> Self     // batch of pulses -> summary Engram
```

### Why this design?

Every capability in the system — agent spawning, gate verification, prompt
assembly, model routing, memory retrieval, affect modulation, chain participation
— is an implementation of one of the nine traits applied to Engrams.

This uniformity has a practical payoff: when you understand these nine
operations, you understand the whole system. When adding a feature, identify
which verb it belongs to. If it does not map cleanly to one of the traits,
reconsider the design.

---

## 3. The Universal Loop

Source: `crates/roko-core/src/loop_tick.rs`

Every operation in Roko follows the same shape. Here is the loop in plain
English:

1. **Query** the store for candidate Engrams (what do we know that's relevant?)
2. **Score** the candidates (which are the most valuable right now?)
3. **Route** to a selection (pick the best one)
4. **Compose** it into an output Engram (assemble, under budget)
5. **Verify** the result (did it pass the gate?)
6. **Write back** if it passed, then **React** (policy fires side effects)

```
candidates = substrate.query(q, ctx)
    ↓
selection = router.select(candidates, ctx)
    ↓
composed  = composer.compose([selection], budget, scorer, ctx)
    ↓
verdict   = gate.verify(composed, ctx)
    ↓
if passed: substrate.put(composed) + policy.decide(stream, ctx)
```

The loop is parameterized entirely by trait implementations. The same
`loop_tick` call trains the scaffold optimizer, picks a model, runs a gate,
assembles a prompt, or claims a bounty — only the concrete impls change.

<details>
<summary>TickConfig and TickOutcome structs</summary>

```rust
pub struct TickConfig {
    pub max_turns: Option<u64>,       // limit iterations
    pub timeout_secs: Option<u64>,    // wall-clock limit
    pub budget_usd: Option<f64>,      // cost ceiling
    pub verbose: bool,
}

pub struct TickOutcome {
    pub candidates_examined: usize,
    pub composed: Option<Engram>,
    pub verdict: Option<Verdict>,
    pub emitted: Vec<Engram>,         // from policy.decide()
    pub stored_hash: Option<ContentHash>,
}
```

</details>

### Walking through a real execution

To make this concrete, here is what happens when `roko plan run` executes a
single task:

1. `PlanRunner` (orchestrate.rs) picks a pending task from the DAG.
2. `CascadeRouter.select()` chooses a model based on the task's role and
   complexity — this is the **Route** step applied to model selection.
3. `SystemPromptBuilder.build()` assembles a 9-layer system prompt by
   **composing** role identity, conventions, domain context, task details,
   relevant playbooks, and current affect state into a single Engram.
4. `ContextAssembler` queries `KnowledgeStore` and injects relevant knowledge
   entries — past insights, anti-patterns, strategy fragments.
5. `DaimonState.pre_dispatch()` computes a `DispatchModulation` that adjusts
   model temperature and turn budget based on recent success/failure history.
6. The agent runs (`dispatch_agent()`), makes tool calls, and produces output.
7. `GatePipeline.run_rung()` **verifies** the output: compile, lint, test,
   symbol check — whatever rungs are appropriate for this task's complexity.
8. If gates pass, the episode is **written back** to `episodes.jsonl`.
9. **React**: `CascadeRouter.feedback()` updates the model arm reward,
   `DaimonState.on_outcome()` updates the affect state, `KnowledgeAdmissionStore`
   considers admitting a new knowledge entry.

Every step is an instance of one of the nine operations. That is the loop.

---

## 4. Crate Map

Here is the full system in one diagram. The arrows show dependency direction
(bottom depends on top). Crates at the bottom are the entry points; crates at
the top are foundations.

```
                         ┌─────────────────────────┐
                         │     roko-primitives      │
                         │  HdcVector, TierRouter,  │
                         │  PadVector, InferenceTier│
                         └───────────┬─────────────┘
                                     │
                         ┌───────────▼─────────────┐
                         │       roko-core          │
                         │  Engram, 9 traits,       │
                         │  foundation traits,      │
                         │  config, tool, runtime   │
                         │  events, signals, jobs   │
                         └──┬──────┬──────┬────────┘
                            │      │      │
           ┌────────────────┘      │      └──────────────┐
           │                       │                      │
  ┌────────▼───────┐    ┌──────────▼────────┐   ┌───────▼───────┐
  │   roko-fs      │    │   roko-runtime    │   │ roko-learn    │
  │  FileSubstrate │    │  EffectDriver,    │   │ EpisodeLogger,│
  │  RokoLayout    │    │  WorkflowEngine,  │   │ CascadeRouter,│
  │  JSONL storage │    │  PipelineStateV2, │   │ Playbooks,    │
  └────────────────┘    │  ProcessSupervisor│   │ Anomaly,      │
                        │  event_bus        │   │ SkillLibrary  │
                        └──────────┬────────┘   └──────┬────────┘
                                   │                    │
       ┌───────────────────────────┼────────────────────┤
       │                           │                    │
┌──────▼──────┐  ┌─────────────────▼──────┐  ┌────────▼───────┐
│ roko-agent  │  │   roko-gate            │  │  roko-neuro    │
│ 5 backends  │  │  7-rung pipeline,      │  │  KnowledgeStore│
│ ToolDisp.,  │  │  AdaptiveThresholds,   │  │  ContextAssemb.│
│ SafetyLayer,│  │  SPC detectors,        │  │  TierProgress. │
│ MCP passth. │  │  CompileGate,TestGate, │  │  Admission     │
└──────┬──────┘  │  ClippyGate,LLMJudge  │  └──────┬─────────┘
       │         └────────────────────────┘         │
       │                                            │
┌──────▼──────────────────────────────────┐ ┌──────▼─────────┐
│             roko-compose                │ │  roko-dreams   │
│  SystemPromptBuilder (9 layers),        │ │  DreamCycle,   │
│  PromptComposer, AttentionBidder,       │ │  Hypnagogia,   │
│  EnrichmentPipeline, ContextProvider    │ │  Imagination,  │
└─────────────────────────────────────────┘ │  Rehearsal     │
                                            └──────┬─────────┘
                                                   │
┌──────────────────────────────────────────────────▼──────────┐
│                      roko-conductor                          │
│  Conductor, CircuitBreaker, 10 Watchers,                     │
│  DiagnosisEngine, StuckDetector, YerkesDodson                │
└──────────────────────────────────────────┬──────────────────┘
                                           │
                            ┌──────────────▼──────────┐
                            │        roko-daimon       │
                            │  DaimonState, AlmaLayers │
                            │  SomaticMarkers,         │
                            │  AffectPolicy adapter    │
                            └──────────────┬───────────┘
                                           │
                 ┌─────────────────────────▼──────────────────┐
                 │             roko-orchestrator               │
                 │  ParallelExecutor, UnifiedTaskDag,          │
                 │  EventLog, WorktreeManager, PheromoneStore  │
                 └────────────────────┬───────────────────────┘
                                      │
          ┌───────────────────────────▼───────────────────────┐
          │                      roko-cli                      │
          │  orchestrate.rs (PlanRunner), dashboard TUI,       │
          │  all subcommands (prd, plan, agent, research, ...) │
          └───────────────────────────────────────────────────┘
```

Additional crates (parallel, not in the main execution stack):

```
  roko-serve          HTTP control plane (~85 routes on :6677)
  roko-agent-server   Per-agent HTTP sidecar (13 routes)
  roko-std            19 builtin tools, StaticToolRegistry, SumScorer
  roko-chain          Chain witness primitives (Phase 2+)
  roko-index          Code-intelligence indexer
  roko-mcp-code       Code-intelligence MCP server
  roko-lang-{rust,typescript,go}  Language analyzers
```

### Reading the map

- `roko-core` is the vocabulary: types and trait definitions. Nothing in it
  does any I/O. It has no runtime dependencies.
- `roko-primitives` is pre-core math: HDC vectors, PAD affect, tier routing.
- The middle tier (`roko-fs`, `roko-runtime`, `roko-learn`) implements the
  infrastructure: file storage, the workflow state machine, the learning
  subsystems.
- `roko-agent`, `roko-gate`, `roko-neuro`, `roko-compose` are the operational
  core: LLM dispatch, verification, knowledge, prompt assembly.
- `roko-conductor`, `roko-daimon`, `roko-dreams` layer on reactive oversight,
  affect modulation, and offline consolidation.
- `roko-orchestrator` and `roko-cli` are the entry points that wire everything
  together.

---

## 5. Protocol Traits (Core Six)

Source: `crates/roko-core/src/traits.rs`

These six traits define the complete operational surface of Roko. Every
capability is an implementation of one of these traits. If you add a feature
and it does not fit into one of these six verbs, reconsider the design.

### 5.1 Store — Persist and Retrieve

**What it does in plain English**: A Store is an addressable database of
Engrams. You put things in (by content hash), get them back, and query by
filter or semantic similarity. Implementations range from an in-memory hash
map (for tests) to a JSONL file on disk to an HDC-indexed semantic store.

<details>
<summary>Store trait signature</summary>

```rust
#[async_trait]
pub trait Store: Send + Sync {
    /// Store an engram. Returns its content hash. Idempotent on content.
    async fn put(&self, engram: Engram) -> Result<ContentHash>;

    /// Retrieve an engram by content hash. Does not apply decay.
    async fn get(&self, id: &ContentHash) -> Result<Option<Engram>>;

    /// Query for engrams matching the given filter. Impls may apply decay
    /// when evaluating min_weight and ordering results.
    async fn query(&self, q: &Query, ctx: &Context) -> Result<Vec<Engram>>;

    /// Query by HDC similarity against a fingerprint.
    async fn query_similar(
        &self,
        fp: &HdcVector,
        radius: f32,
        limit: usize,
        ctx: &Context,
    ) -> Result<Vec<(ContentHash, f32)>>;

    /// Remove engrams whose effective weight has fallen below threshold.
    async fn prune(&self, threshold: f32, ctx: &Context) -> Result<usize>;

    async fn len(&self) -> Result<usize>;
    async fn is_empty(&self) -> Result<bool>;
    fn name(&self) -> &'static str;
}
```

</details>

**Implementations**: `MemorySubstrate` (testing), `FileSubstrate` (JSONL on
`.roko/`), `HdcSubstrate` (semantic search), `ChainSubstrate` (on-chain shared
state, Phase 2+).

---

### 5.2 Score — Rate Along Dimensions

**What it does in plain English**: A Scorer takes an Engram and returns a
numeric rating. Scores are multi-dimensional (confidence, novelty, utility,
reputation) and context-dependent. Different scorers weight things differently:
`RecencyScorer` favors newer engrams, `ReputationScorer` favors high-trust
sources. The composite `SumScorer` blends multiple scorers.

<details>
<summary>Score trait signature</summary>

```rust
pub trait Score: Send + Sync {
    /// Score an engram in the given context. Pure function.
    fn score(&self, engram: &Engram, ctx: &Context) -> ScoreValue;

    fn score_engram(&self, engram: &Engram, ctx: &Context) -> ScoreValue;
    fn score_pulse(&self, p: &Pulse, ctx: &Context) -> ScoreValue;
    fn score_datum(&self, datum: Datum<'_>, ctx: &Context) -> ScoreValue;
    fn name(&self) -> &'static str;
}
```

</details>

The `ScoreValue` struct carries four dimensions: `confidence`, `novelty`,
`utility`, and `reputation`. `Score::effective()` returns a single f32
combining them.

**Implementations**: `RelevanceScorer`, `RecencyScorer`, `ReputationScorer`,
`CatalyticScorer`, `SumScorer` (roko-std), `CompositeScorer`.

---

### 5.3 Verify — Check Against Ground Truth

**What it does in plain English**: A Verifier takes an Engram and returns a
Verdict — pass or fail, with output. This is how the gate pipeline works: each
gate is a `Verify` implementation that runs `cargo compile`, `cargo test`, a
diff check, or an LLM judge, and returns whether the output is acceptable.

<details>
<summary>Verify trait signature</summary>

```rust
#[async_trait]
pub trait Verify: Send + Sync {
    /// Verify the engram and return a verdict.
    async fn verify(&self, engram: &Engram, ctx: &Context) -> Verdict;

    /// Verify a batch of ephemeral pulses by promoting them to a synthetic engram.
    async fn verify_stream(&self, pulses: &[Pulse], ctx: &Context) -> Verdict;

    /// Human-readable name (appears in verdicts).
    fn name(&self) -> &str;
}
```

</details>

**Implementations**: See Section 12 (Gate Pipeline) for the full list.

---

### 5.4 Route — Select One Candidate

**What it does in plain English**: A Router takes a list of candidates and
picks one. At the model routing level, the candidates are models and the
Router implements the learning loop that decides which model to use. At the
knowledge level, the Router picks which knowledge entry to surface.

<details>
<summary>Route trait signature</summary>

```rust
pub trait Route: Send + Sync {
    /// Select one engram from the candidates. None = no selection made.
    fn select(&self, candidates: &[Engram], ctx: &Context) -> Option<Selection>;

    fn select_engram(&self, candidates: &[Engram], ctx: &Context) -> Option<Selection>;
    fn select_pulse(&self, candidates: &[Pulse], ctx: &Context) -> Option<Selection>;

    /// Learn from a selection's actual outcome (for bandit updates).
    fn feedback(&self, outcome: &Outcome);

    fn name(&self) -> &str;
}
```

</details>

**Implementations**: `StaticRouter` (config-driven), `LinUCBRouter` (contextual
bandit), `CascadeRouter` (3-stage: Static→Confidence→UCB), `WeightedRouter`
(softmax over scorers).

---

### 5.5 Compose — Combine Under Budget

**What it does in plain English**: A Composer takes multiple input Engrams and
assembles them into a single output Engram, respecting a token budget. This is
how prompt assembly works: `PromptComposer` takes role identity, conventions,
task context, knowledge entries, and gate feedback, and assembles them into a
single system prompt that fits within the model's context window.

<details>
<summary>Compose trait signature</summary>

```rust
pub trait Compose: Send + Sync {
    /// Combine input engrams into a new composed engram.
    fn compose(
        &self,
        engrams: &[Engram],
        budget: &Budget,
        scorer: &dyn Score,
        ctx: &Context,
    ) -> Result<Engram>;

    /// Compose from a polymorphic mix of engrams and pulses.
    fn compose_datums(
        &self,
        datums: &[Datum<'_>],
        budget: &Budget,
        scorer: &dyn Score,
        ctx: &Context,
    ) -> Result<Engram>;

    fn name(&self) -> &str;
}
```

</details>

**Implementations**: `PromptComposer` (prompt assembly), `ContextAssembler`
(knowledge context packs), task plan combiners.

---

### 5.6 React — Watch Streams and Emit Interventions

**What it does in plain English**: A Policy watches the stream of recent
Engrams and decides whether to intervene. The `Conductor` is the primary
implementation: it runs 10 watchers over the stream and emits `ConductorDecision`
events when something is wrong (agent stuck, budget exceeded, quality degrading).
This is the reactive oversight layer.

<details>
<summary>React trait signature</summary>

```rust
pub trait React: Send + Sync {
    /// Examine the recent engram stream and produce new engrams (interventions).
    fn decide(&self, stream: &[Engram], ctx: &Context) -> Vec<Engram>;

    /// Examine both persisted engrams and ephemeral pulses.
    fn decide_with_pulses(
        &self,
        engrams: &[Engram],
        pulses: &[Pulse],
        ctx: &Context,
    ) -> PolicyOutputs;

    fn name(&self) -> &str;
}
```

</details>

`PolicyOutputs` contains both `engrams: Vec<Engram>` (to persist) and
`pulses: Vec<Pulse>` (to publish on the Bus).

**Implementations**: `Conductor` (composite of 10 watchers), `CircuitBreaker`,
individual watcher impls (stuck detection, anomaly, budget, etc.).

---

## 6. Foundation Service Traits

Source: `crates/roko-core/src/foundation.rs`

These are the service contracts between the WorkflowEngine and its
infrastructure providers. Every impl is injected at runtime — the engine is
pure. This is what makes the WorkflowEngine testable: swap in mock
implementations of these traits and you can drive the full pipeline without
touching a real LLM or filesystem.

### 6.1 ModelCaller

**What it does**: The contract between the engine and LLM backends. Takes a
structured request (model, messages, budget, routing hints) and returns a
response (content, usage, stop reason). All five LLM backends implement this.

<details>
<summary>ModelCaller structs and trait</summary>

```rust
pub struct ModelCallRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub role: Option<String>,
    pub caller: Option<String>,       // "cli" | "serve" | "research" | "dreams"
    pub run_id: Option<String>,
    pub prompt_section_ids: Vec<String>,
    pub knowledge_ids: Vec<String>,
    pub budget: Option<TokenBudget>,
    pub budget_remaining: Option<f64>,
    pub routing_hints: Vec<String>,
    pub cache_policy: CachePolicy,    // Default | Bypass | ForceRefresh
}

pub struct ModelCallResponse {
    pub content: String,
    pub model: String,
    pub usage: TokenUsage,            // input_tokens, output_tokens, cost_usd
    pub stop_reason: Option<String>,
    pub request_id: Option<String>,
}

#[async_trait]
pub trait ModelCaller: Send + Sync {
    async fn call(&self, req: ModelCallRequest) -> Result<ModelCallResponse>;
}
```

</details>

Implemented by: `roko-agent` backends (Claude CLI, Claude API, Gemini, Codex,
Ollama, Perplexity, OpenAI-compat).

---

### 6.2 PromptAssembler

**What it does**: Assembles a system prompt from a role spec. The 9-layer
`SystemPromptBuilder` in `roko-compose` implements this. See Section 18 for
the assembly details.

<details>
<summary>PromptAssembler structs and trait</summary>

```rust
pub struct PromptSpec {
    pub role: Option<String>,
    pub task: Option<String>,
    pub workdir: Option<PathBuf>,
    pub gate_feedback: Vec<String>,
    pub anti_patterns: Vec<String>,
}

#[async_trait]
pub trait PromptAssembler: Send + Sync {
    async fn assemble(&self, spec: PromptSpec) -> Result<String>;
    fn last_prompt_section_ids(&self) -> Vec<String>;
    fn last_knowledge_ids(&self) -> Vec<String>;
}
```

</details>

---

### 6.3 FeedbackSink

**What it does**: The funnel through which every model call, gate result, and
completed workflow feeds into the learning subsystems. The `FeedbackService`
in `roko-learn` implements this and fans events to `CascadeRouter`,
`EpisodeLogger`, `AdaptiveThresholds`, and more.

<details>
<summary>FeedbackSink event types and trait</summary>

```rust
pub enum FeedbackEvent {
    ModelCall {
        run_id: Option<String>,
        request_id: Option<String>,
        prompt_section_ids: Vec<String>,
        knowledge_ids: Vec<String>,
        model: Option<String>,
        role: String,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        latency_ms: u64,
        success: bool,
    },
    GateResult {
        run_id: String,
        gate_name: String,
        passed: bool,
        duration_ms: u64,
    },
    WorkflowComplete {
        event_type: String,
        run_id: String,
        model: Option<String>,
        success: bool,
        total_cost_usd: f64,
        total_tokens: u64,
        duration_ms: u64,
    },
}

#[async_trait]
pub trait FeedbackSink: Send + Sync {
    async fn record(&self, event: FeedbackEvent) -> Result<()>;
    async fn flush(&self) -> Result<()>;
}
```

</details>

---

### 6.4 GateRunner

**What it does**: The contract the WorkflowEngine uses to run gates without
knowing about the gate pipeline's internals. `GatePipeline` in `roko-gate`
implements this. See Section 12.

<details>
<summary>GateRunner structs and trait</summary>

```rust
pub struct GateConfig {
    pub workdir: PathBuf,
    pub enabled_gates: Vec<String>,   // ["compile", "test", "clippy", ...]
    pub shell_gates: Vec<ShellGateCommand>,
    pub max_rung: Option<u8>,
}

pub struct GateVerdict {
    pub gate_name: String,
    pub passed: bool,
    pub skipped: bool,
    pub skip_reason: Option<String>,
    pub output: String,
    pub duration_ms: u64,
}

pub struct GateReport {
    pub verdicts: Vec<GateVerdict>,
    // helpers: all_passed(), first_failure(), failure_summary()
}

#[async_trait]
pub trait GateRunner: Send + Sync {
    async fn run_gates(&self, config: GateConfig) -> Result<GateReport>;
}
```

</details>

---

### 6.5 EventConsumer

**What it does**: A one-way sink for `RuntimeEvent` values emitted by the
workflow engine. Adapters include: the TUI bridge (renders events to the
dashboard), the SSE broadcaster (pushes to HTTP clients), and the JSONL logger.

```rust
pub trait EventConsumer: Send + Sync {
    /// Called for each event emitted by the workflow engine. Must be non-blocking.
    fn consume(&self, event: &RuntimeEvent);
}
```

---

### 6.6 EffectExecutor (Low-level Effect)

**What it does**: The low-level effect abstraction used by the `EffectDriver`.
Translates `Effect` enum variants (SpawnAgent, RunGates, Commit, Checkpoint)
into real I/O, and returns `EffectOutcome`.

<details>
<summary>Effect, EffectOutcome, and EffectExecutor</summary>

```rust
pub enum Effect {
    SpawnAgent {
        run_id: String, role: String, model: String,
        system_prompt: String, user_prompt: String, workdir: PathBuf,
    },
    RunGates { run_id: String, config: GateConfig },
    Commit { run_id: String, workdir: PathBuf, message: String },
    Checkpoint { run_id: String, state_json: String, path: PathBuf },
}

pub enum EffectOutcome {
    AgentDone { agent_id: String, output: String, tokens_used: u64,
                cost_usd: f64, files_changed: Vec<String> },
    GatesDone { report: GateReport },
    CommitDone { hash: String, message: String },
    CheckpointDone { path: String },
    Failed { error: String },
}

#[async_trait]
pub trait EffectExecutor: Send + Sync {
    async fn execute(&self, effect: Effect) -> Result<EffectOutcome>;
}
```

</details>

---

### 6.7 AffectPolicy

**What it does**: DaimonPolicy is like an emotional thermostat. It reads the
system's recent success/failure history, maintains a multi-timescale emotional
state (fast emotion, medium mood, slow temperament), and adjusts the behavior
of every agent dispatch based on that state. If agents have been failing
repeatedly, the thermostat notices and dials down exploration. If things are
going smoothly, it stays lean.

<details>
<summary>AffectPolicy structs and trait</summary>

```rust
pub struct AffectContext {
    pub behavioral_state: BehavioralState,
    pub pad: [f32; 3],                // [Pleasure, Arousal, Dominance]
    pub emotional_tag: Option<String>,
}

pub struct DispatchModulation {
    pub tier_bias: f32,               // -1.0 (cheapest) to +1.0 (most capable)
    pub turn_limit_factor: f32,       // 1.0 = no change
    pub exploration_rate: f32,        // 0.0 to 1.0
}

#[async_trait]
pub trait AffectPolicy: Send + Sync {
    fn pre_dispatch(&self, task_id: &str, role: &str) -> AffectContext;
    fn on_task_outcome(&mut self, task_id: &str, succeeded: bool,
                       tokens_used: u64, cost_usd: f64);
    fn on_gate_result(&mut self, gate_name: &str, passed: bool,
                      rung: u8, confidence: f64);
    fn modulate_dispatch(&self, role: &str, params: &mut DispatchModulation);
    fn behavioral_state(&self) -> BehavioralState;
    async fn persist(&self) -> Result<()>;
}
```

</details>

Canonical implementation: `DaimonPolicy` (roko-daimon). No-op: `NoOpAffectPolicy`.

---

## 7. Supporting Protocol Traits

Source: `crates/roko-core/src/traits.rs`

### 7.1 Bus — Publish/Subscribe for Ephemeral Pulses

**What it does**: The Bus is a broadcast channel for `Pulse` values — things
that need immediate reactive handling but may not need to be persisted.
Subscribers filter by topic. The sequence number enables gap detection and
ordered replay.

<details>
<summary>Bus trait</summary>

```rust
pub trait Bus: Send + Sync {
    type Receiver: Send;
    fn publish(&self, pulse: Pulse) -> Result<u64>;       // returns sequence number
    fn subscribe(&self, filter: TopicFilter) -> Result<Self::Receiver>;
}
```

</details>

**Implementation**: `PulseBus` wraps `EventBus<Pulse>` with topic filtering.

---

### 7.2 ColdStore — Archival for Aged-Out Engrams

**What it does**: When an Engram's effective weight falls below a threshold
(because it is old and low-scored), it moves from the hot `Store` to a
`ColdStore`. Cold storage is compressed, rarely-accessed, and does not
participate in live queries. Engrams can be thawed back on demand.

<details>
<summary>ColdStore trait and migration flow</summary>

```rust
#[async_trait]
pub trait ColdStore: Send + Sync {
    async fn archive(&self, engram: Engram) -> Result<ContentHash>;
    async fn archive_batch(&self, engrams: Vec<Engram>) -> Result<usize>;
    async fn thaw(&self, id: &ContentHash) -> Result<Option<Engram>>;
    async fn contains(&self, id: &ContentHash) -> Result<bool>;
    async fn archived_count(&self) -> Result<usize>;
    async fn storage_bytes(&self) -> Result<u64>;
    async fn purge_before(&self, epoch_ms: i64) -> Result<usize>;
    fn name(&self) -> &'static str;
}
```

Migration flow:
```
Store (hot) ──age_out()──► ColdStore (cold/archive)
              ◄──thaw()──
```

</details>

**Implementation**: `ArchiveColdSubstrate` in roko-fs (compressed JSONL archives).

---

### 7.3 Observe, Connect, Trigger (Cell-Based Extensions)

These three traits extend the `Cell` supertrait for peripheral integrations:
external data sources (`Observe`), network connections (`Connect`), and
scheduled triggers (`Trigger`).

<details>
<summary>Observe, Connect, Trigger trait signatures</summary>

```rust
pub trait Observe: Cell {
    fn observe(&self) -> Vec<Engram>;
}

pub trait Connect: Cell {
    fn connect(&self) -> Result<()>;
    fn health(&self) -> bool;
    fn disconnect(&self) -> Result<()>;
}

pub trait Trigger: Cell {
    fn arm(&self) -> Result<()>;
    fn disarm(&self) -> Result<()>;
}
```

</details>

---

## 8. The Cell Supertrait

Source: `crates/roko-cell.rs`

Every protocol implementation must be a `Cell`. This gives the execution engine
identity, cost estimation, and protocol introspection. Think of it as the
common interface that lets the system ask any component "who are you, how
expensive are you, and what can you do?"

<details>
<summary>Cell trait signature</summary>

```rust
pub trait Cell: Send + Sync + 'static {
    fn cell_id(&self) -> &str;
    fn cell_name(&self) -> &str;
    fn cell_version(&self) -> CellVersion { (0, 1, 0) }
    fn protocols(&self) -> &[&str] { &[] }
    fn estimated_cost(&self) -> Option<f64> { None }
    fn estimated_duration(&self) -> Option<Duration> { None }
}
```

</details>

---

## 9. WorkflowEngine and PipelineStateV2

Sources:
- `crates/roko-runtime/src/pipeline_state.rs`
- `crates/roko-runtime/src/workflow_engine.rs`

### The core idea

The WorkflowEngine is the execution engine for a single task. Given a task
prompt and a set of services, it runs the agent, validates the output, handles
failures, and commits the result. It coordinates the `PipelineStateV2` state
machine with the `EffectDriver` that executes real I/O.

> **Why pure state machine + effect driver?** Because it makes the engine
> testable — you can feed events to `PipelineStateV2` and verify the outputs
> without ever spawning a real agent. The state machine has zero side effects.
> Every decision it makes is expressed as a `PipelineOutput` action that the
> `EffectDriver` then executes. Swap the driver for a mock and the entire
> workflow logic is unit-testable.

The separation looks like this:

```
PipelineStateV2       -- PURE state machine, zero side effects
EffectDriver          -- executes the actions returned by the state machine
WorkflowEngine        -- ties them together in a run loop
```

### Workflow Templates

Three built-in configurations cover the common cases:

```rust
pub struct WorkflowConfig {
    pub has_strategy: bool,
    pub has_review: bool,
    pub max_iterations: u32,
    pub max_autofix_attempts: u32,
}

impl WorkflowConfig {
    pub fn express() -> Self   // implement -> gate -> commit
    pub fn standard() -> Self  // implement -> gate -> review -> commit
    pub fn full() -> Self      // strategy -> implement -> gate -> review -> commit
}
```

### Phase State Machine

The state machine moves a task through phases. Here is the complete transition
diagram:

<details>
<summary>Full state machine transition diagram</summary>

```
Pending
  │ Start
  ▼
Strategizing ─────────────────────────┐
  │ StrategyComplete(brief)           │ StrategySkipped
  │                                   │
  ▼                                   ▼
Implementing ◄──────────────────────────────────────────────────┐
  │ AgentCompleted                    │ ReviewRevise / ReviewRej │
  │                                   │                          │
  ▼           AgentFailed             ▼                          │
Gating ───────────────────────► Halted         Reviewing ───────┘
  │                                             ▲│ ReviewApproved
  │ GatesPassed                                 ││
  │ ┌─ no review                     GatesPassed┘│
  │ └─ has_review ──────────────────────────────►│
  │                                              │
  │ GateFailed                                   │
  ├──► AutoFixing ──► (retry gates or Implementing or Halted)
  │
  └─ iterations exhausted ──► Halted

Reviewing ──────────────────────────────────────────► Committing
                                                          │
                                                   CommitDone
                                                          │
                                                        Complete

(UserCancel → Cancelled from any phase)
(ResourceExhausted → Halted from any phase)
```

</details>

<details>
<summary>PipelineInput events (what drives the state machine)</summary>

```rust
pub enum PipelineInput {
    Start,
    StrategyComplete { brief: String },
    StrategySkipped,
    AgentCompleted { output: String, files_changed: u32 },
    AgentFailed { error: String },
    GatesPassed,
    GateFailed { gate: String, output: String },
    ReviewApproved { summary: String },
    ReviewRejected { reason: String },
    ReviewUnclear { summary: String },
    ReviewRevise { findings: Vec<String> },
    CommitDone { hash: String },
    CommitFailed { error: String },
    UserCancel,
    ResourceExhausted { reason: String },
}
```

</details>

<details>
<summary>PipelineOutput actions (what the state machine returns)</summary>

```rust
pub enum PipelineOutput {
    SpawnStrategist { prompt: String },
    SpawnImplementer { prompt: String, context: Option<String> },
    SpawnAutoFixer { error_output: String },
    RunGates,
    SpawnReviewer { diff_context: Option<String> },
    Commit,
    Done { outcome: WorkflowOutcome },
    Halt { reason: String },
}
```

</details>

### Checkpointing

The state machine serializes to JSON for resumption:

```rust
let json = sm.checkpoint()?;             // → JSON string
let sm = PipelineStateV2::from_checkpoint(&json)?; // restore exact state
```

This is how `roko plan run --resume .roko/state/executor.json` works. The
executor snapshots state after every task completion, so if the process crashes
mid-plan, you can resume from the last checkpoint.

### WorkflowEngine

<details>
<summary>WorkflowEngine struct and run loop</summary>

```rust
pub struct WorkflowEngine {
    services: EffectServices,
    consumers: Vec<Arc<dyn EventConsumer>>,
}

impl WorkflowEngine {
    pub async fn run(&self, config: WorkflowRunConfig) -> Result<WorkflowRunReport>;
    pub async fn run_with_cancel(
        &self,
        config: WorkflowRunConfig,
        token: CancelToken,
    ) -> Result<WorkflowRunReport>;
}
```

The run loop:
1. Creates `PipelineStateV2` from config.
2. Creates `EffectDriver` from services.
3. Loop: `sm.step(input)` → `effect_driver.execute(output)` → new input → repeat.
4. Each iteration checks `CancelToken` for cooperative cancellation.
5. Returns `WorkflowRunReport` with gate outcomes, timing, tokens, cost.

</details>

---

## 10. EffectDriver Pattern

Source: `crates/roko-runtime/src/effect_driver.rs`

The `EffectDriver` bridges the pure state machine to real infrastructure.

When the state machine emits `PipelineOutput::SpawnImplementer`, the
EffectDriver executes it by: assembling the system prompt, applying affect
modulation, calling the model, recording feedback, and returning
`PipelineInput::AgentCompleted` or `::AgentFailed`.

<details>
<summary>EffectServices and EffectDriver structs</summary>

```rust
pub struct EffectServices {
    pub default_model: String,
    pub model_caller: Arc<dyn ModelCaller>,
    pub prompt_assembler: Arc<dyn PromptAssembler>,
    pub feedback_sink: Arc<dyn FeedbackSink>,
    pub gate_runner: Arc<dyn GateRunner>,
    pub affect_policy: Option<Arc<Mutex<dyn AffectPolicy>>>,
}

pub struct EffectDriver {
    services: EffectServices,
    run_id: String,
    workdir: PathBuf,
    feedback_totals: Mutex<WorkflowFeedbackTotals>,
}
```

</details>

The `spawn_agent` method sequence (called for `SpawnImplementer`,
`SpawnStrategist`, `SpawnAutoFixer`, `SpawnReviewer`):

```
1. Compute DispatchModulation from AffectPolicy.pre_dispatch()
2. Clamp turn_limit_factor in [MIN=0.25, MAX=2.0]
3. Adjust temperature: BASE(0.2) + exploration_rate * 0.6 + tier_bias * 0.1
4. Choose CachePolicy::Bypass if exploration_rate > 0.5
5. Assemble system prompt via PromptAssembler.assemble(spec)
6. Call ModelCaller.call(req)
7. Record FeedbackEvent.ModelCall via FeedbackSink
8. Notify AffectPolicy.on_task_outcome()
9. Return PipelineInput::AgentCompleted or ::AgentFailed
```

The key design constraint: the `EffectDriver` and `PipelineStateV2` have no
shared mutable state. The state machine is the only source of truth about what
phase the workflow is in.

---

## 11. Agent Dispatch and ToolDispatcher

Source: `crates/roko-agent/src/dispatcher/mod.rs`

### The ToolDispatcher Pipeline

When an agent makes a tool call during its turn, every call passes through a
fixed 8-stage pipeline. Think of it like a middleware stack:

```
1. validate    -- JSON schema check against registry def
2. resolve     -- look up ToolDef in registry by canonical name
3. authorize   -- def.permission.satisfied_by(&role_perms)
4. tool_selector -- profile-based allow/deny check (TOOL-03)
5. hook_chain  -- sequential safety hooks, first rejection short-circuits
6. handler     -- HandlerResolver.resolve(name) → ToolHandler.execute()
                  raced against ctx.timeout + CancelToken
7. truncate    -- cap Ok content at DEFAULT_MAX_RESULT_BYTES (16,384)
8. result_cache -- optionally cache deterministic tool results
```

<details>
<summary>Batch dispatch and HandlerResolver</summary>

**Batch Dispatch**

```rust
pub async fn dispatch_batch(
    &self,
    calls: Vec<ToolCall>,
    ctx: &ToolContext,
) -> Vec<ToolResult>;
```

Calls are partitioned by `ToolConcurrency`: `Parallel` tools run via
`join_all`; `Serial` tools run sequentially to preserve shell-state ordering
and avoid write-write races.

**HandlerResolver (Pluggable)**

```rust
pub trait HandlerResolver: Send + Sync {
    fn resolve(&self, name: &str) -> Option<Arc<dyn ToolHandler>>;
}
```

The builtin resolver is `roko_std::tool::handlers::handler_for`. Custom MCP
backends provide their own. This keeps `roko-agent` free of `roko-std`
dependency.

**ToolDispatcher struct**

```rust
pub struct ToolDispatcher {
    registry: Arc<dyn ToolRegistry>,
    resolver: Arc<dyn HandlerResolver>,
    max_result_bytes: usize,         // default 16,384
    safety: Option<SafetyLayer>,
    tool_cache: Option<Mutex<ToolResultCache>>,
    hook_chain: Option<SafetyHookChain>,
    tool_selector: Option<ToolSelector>,
}
```

</details>

### Agent Backends

Five LLM backends share a common `Agent` trait interface:

| Backend | Use case |
|---|---|
| Claude CLI | Default: spawns `claude` subprocess, streams JSON |
| Claude API | Direct API calls, streaming, prompt caching |
| Gemini | Google Gemini via REST, includes free-tier shadow runner |
| Perplexity | Web-search-grounded research queries |
| OpenAI-compat | Any OpenAI-compatible endpoint (Ollama, Codex, local) |

---

## 12. Gate Pipeline Architecture

Source: `crates/roko-gate/src/lib.rs`, `rung_selector.rs`, `adaptive_threshold.rs`

The gate pipeline is like a CI/CD pipeline for AI output. An agent produces
changes; the gate pipeline runs them through a sequence of checks and either
approves or rejects. Unlike a static CI pipeline, this one adapts: it skips
rungs that are consistently passing, adjusts retry budgets based on historical
pass rates, and detects statistical shifts in quality.

### 7-Rung Pipeline

The canonical pipeline is selected based on plan complexity
(`PlanComplexity`: Trivial, Simple, Moderate, Complex, Critical). Harder tasks
run more rungs:

| Rung | Index | Gates | Trigger |
|------|-------|-------|---------|
| Compile | 0 | `CompileGate` | All plans |
| Lint | 1 | `ClippyGate` | Simple+ |
| Test | 2 | `TestGate` | Moderate+ |
| Symbol | 3 | `SymbolGate` | Moderate+ |
| GeneratedTest | 4 | `GeneratedTestGate` + `VerifyChainGate` | Complex+ |
| PropertyTest | 5 | `PropertyTestGate` + `FactCheckGate` | Complex+ |
| Integration | 6 | `LlmJudgeGate` + `IntegrationGate` | Critical |

### Standalone Gates (6 Gates)

Invoked outside the rung pipeline for specific scenarios:

- `DiffGate` — post-task diff analysis
- `CodeExecutionGate` — sandboxed code execution
- `ShellGate` — arbitrary shell command verification
- `BenchmarkRegressionGate` — performance regression detection
- `FormatCheckGate` — code formatting (rustfmt, prettier)
- `SecurityScanGate` — security scanning

### Gate Combinators

```rust
pub struct ParallelGate<G>(Vec<G>);   // run gates in parallel, collect all verdicts
pub struct VotingGate<G>(Vec<G>);     // majority-vote across inner gates
pub struct FallbackGate<G>(Vec<G>);   // try in order, first non-error wins
```

### Adaptive Thresholds

Each rung tracks its own statistical history using EMA and CUSUM. The system
uses this history to make smarter decisions: if a rung has passed 20 times in
a row, skip it. If the pass rate suddenly drops, alert the Conductor.

<details>
<summary>RungStats and AdaptiveThresholds structs</summary>

```rust
pub struct RungStats {
    pub ema_pass_rate: f64,           // starts at 0.5 (neutral)
    pub total_observations: u64,
    pub consecutive_passes: u32,
    pub cusum_high: f64,              // detects upward shifts in pass rate
    pub cusum_low: f64,               // detects downward shifts
    pub cusum_shift_detected: bool,
}

pub struct AdaptiveThresholds {
    // per_rung: HashMap<u32, RungStats>
    // SPC detectors: CUSUM + EWMA + BOCPD (roko-gate/spc.rs)
    // Hotelling's T-squared for multi-gate joint anomaly (roko-gate/hotelling.rs)
}
```

EMA update formula (alpha = 0.1):
```
ema_pass_rate = 0.9 * ema_pass_rate + 0.1 * (1.0 if passed else 0.0)
```

</details>

Key decisions driven by adaptive thresholds:

- **Retry budget**: `suggested_retries(rung)` → 1..5 based on EMA pass rate
- **Skip decision**: when `consecutive_passes >= 20`, suggest skipping the rung
- **CUSUM alerts**: trigger replan or conductor intervention on detected shifts

### SPC Detectors

<details>
<summary>Statistical Process Control detector details</summary>

```
CUSUM      -- Cumulative Sum, detects sustained mean shift
EWMA       -- Exponentially Weighted Moving Average control chart
BOCPD      -- Bayesian Online Change Point Detection (for structural shifts)
```

All three fire `SpcAlert` events that feed into the `Conductor` watcher pipeline.

</details>

### Gate Failure Classification

<details>
<summary>FailureClass and GateFailureAction enums</summary>

```rust
pub enum FailureClass {
    CompileError { error_code: String, category: ErrorCategory },
    TestFailure { test_name: String, failure_kind: GateFailureKind },
    LintError { rule: String },
    SymbolMismatch { expected: Vec<String>, found: Vec<String> },
    // ...
}

pub enum GateFailureAction {
    Retry,          // transient, likely to pass on retry
    FixRequired,    // agent must change code
    Escalate,       // abort current attempt, escalate to conductor
    Skip,           // gate not applicable to this change
}
```

</details>

---

## 13. CascadeRouter: 3-Stage Model Selection

Source: `crates/roko-learn/src/cascade_router.rs`

CascadeRouter is like an A/B testing framework that learns. It starts simple
(use the configured model for each role), accumulates observations, builds
confidence, and eventually graduates to a full contextual bandit that learns
which model produces the best outcomes for which kinds of tasks. The three
stages correspond to how much data you have:

| Stage | Name | Observations | Strategy |
|-------|------|--------------|----------|
| 1 | Static | < 50 | Hardcoded role → model table |
| 2 | Confidence | 50–200 | Empirical pass rates + confidence interval |
| 3 | UCB | > 200 | Full `LinUCB` contextual bandit |

<details>
<summary>CascadeRouter struct</summary>

```rust
pub struct CascadeRouter {
    linucb: LinUCBRouter,                           // Stage 3 bandit
    confidence_stats: Mutex<HashMap<String, ModelStats>>, // Stage 2 stats
    pareto_frontier: Mutex<ParetoFrontierState>,    // cost-quality Pareto frontier
    role_table: Mutex<HashMap<AgentRole, String>>,  // Stage 1 static table
    model_slugs: Vec<String>,                       // available arms
    stage_tracking: Mutex<StageTracking>,           // current stage + history
    free_tier_shadow_runner: Option<Arc<dyn ShadowModelRunner>>,
}
```

</details>

### Stage 1 — Static Routing

Uses a hardcoded `role → model_slug` table. The table is configurable from
`roko.toml` under `[cascade_router]`. Transitions to Stage 2 once 50
observations accumulate.

### Stage 2 — Confidence-Based

<details>
<summary>Stage 2 selection logic</summary>

Tracks `ModelStats` per model:
- Pass rate (EMA)
- Observation count
- Confidence interval width

Selects the model whose lower confidence interval bound is highest. Falls back
to static when confidence intervals are too wide. Transitions to Stage 3 after
200 observations.

</details>

### Stage 3 — LinUCB Contextual Bandit

<details>
<summary>LinUCB feature vector and reward computation</summary>

`LinUCBRouter` uses a `CONTEXT_DIM`-dimensional feature vector built from:
- Task category and complexity
- Domain
- Operating frequency
- Daimon behavioral state
- CFactor regression signal
- Pareto frontier position

`compute_routing_reward_v2` aggregates: pass rate, cost, latency, C-factor,
budget pressure, and Pareto adjustments into a scalar reward.

</details>

### Supporting Systems

**Pareto frontier**: Periodically recomputed from `ModelObservation` records.
Down-weights dominated models during UCB exploration.

**DaimonPolicy influence**: Behavioral state shifts the tier bias:
- `Struggling` → prefer cheaper tiers (reduce escalation risk)
- `Coasting` → stay lean
- `Focused` → prefer higher capability

**Temperature adjustment**: `exploration_rate` from `AffectPolicy` modulates
model call temperature in `EffectDriver`.

**Persistence**: State serializes to `.roko/learn/cascade-router.json`. The
router does not lose its learning between runs.

---

## 14. Learning Architecture

Source: `crates/roko-learn/src/lib.rs`

The learning system is a collection of independent subsystems that consume the
signal stream from the orchestrator and agents, persist durable records, and
surface reusable knowledge back to the composer/router feedback loop.

Every task completion fans out through `FeedbackSink.record()` into all the
subsystems below. Nothing is thrown away.

### 14.1 Episode Logger

An episode is a complete record of one agent task execution: what task it ran,
what model it used, what gates it passed or failed, how much it cost.

<details>
<summary>Episode struct</summary>

```rust
pub struct Episode {
    pub id: String,
    pub task_id: String,
    pub plan_id: String,
    pub role: String,
    pub model: String,
    pub prompt_sections: Vec<String>,
    pub gate_verdicts: Vec<GateVerdict>,
    pub usage: Usage,                  // tokens, cost, latency
    pub hdc_fingerprint: Option<HdcVector>,
    pub outcome: String,               // "success" | "failure"
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}
```

</details>

Persisted to `.roko/episodes.jsonl`. The `hdc_fingerprint` per episode enables
semantic clustering during dream consolidation (Section 16).

### 14.2 CFactor (Catalyst Factor)

C-factor measures how many downstream engrams an engram enabled. It answers
the question: "did this particular task unlock a lot of subsequent work, or
was it a dead end?"

<details>
<summary>C-factor formula and uses</summary>

```
C-factor = (downstream_count - baseline) / baseline_stddev
```

High C-factor → this engram was unusually catalytic. Used to:
- Adjust routing rewards in `compute_routing_reward_v2`
- Trigger replan on detected C-factor regressions
- Influence Daimon behavioral state classification

</details>

### 14.3 Anomaly Detection

Three detectors running continuously:

- `RunawayLoopDetector` — detects repeated identical tool calls
- `CostSpikeDetector` — detects sudden cost increases
- `QualityDegradationDetector` — detects declining gate pass rates

Each emits `Anomaly` events consumed by the `Conductor`.

### 14.4 Budget Guardrails

```rust
pub struct BudgetGuardrail {
    // tracks per-plan and fleet-wide spend
    // emits BudgetAction::Warn | Throttle | Abort when limits are hit
}
```

### 14.5 Playbook Store

Reusable task patterns extracted from successful episodes. Queried at dispatch
time and injected into Layer 6 of the system prompt. When an agent faces a
task similar to one it has done before, its playbook is in the prompt.

<details>
<summary>Playbook struct</summary>

```rust
pub struct Playbook {
    pub id: String,
    pub role: String,
    pub description: String,
    pub steps: Vec<PlaybookStep>,
    pub success_rate: f64,
    pub avg_cost_usd: f64,
}
```

</details>

### 14.6 Model Experiment Store

<details>
<summary>A/B experiment structure</summary>

A/B experiments over prompt sections and models:

```rust
pub struct ModelExperimentStore {
    // experiments: Vec<ModelExperiment>
    // Each experiment: control vs treatment, assignment by hash
    // Outcome recording: per-experiment EMA of rewards
}
```

Results feed back into `CascadeRouter` arm selection.

</details>

### 14.7 Skill Library

<details>
<summary>Skill struct</summary>

Structured skills that agents can invoke and improve:

```rust
pub struct Skill {
    pub id: String,
    pub name: String,
    pub domain: String,
    pub description: String,
    pub success_count: u32,
    pub failure_count: u32,
}
```

</details>

### 14.8 Error Pattern Store

Persistent storage of gate failure patterns, enabling learned retry strategies.
When the system sees a failure it has seen before, it knows whether recovering
was successful and how many tokens it took.

<details>
<summary>GateFailureObservation struct</summary>

```rust
pub struct GateFailureObservation {
    pub gate_name: String,
    pub error_signature: String,     // structural fingerprint
    pub task_context: TaskContext,
    pub recovery_succeeded: bool,
    pub tokens_to_recover: u64,
}
```

</details>

### 14.9 Routing Decision Log

<details>
<summary>RoutingDecisionLog struct</summary>

Every routing decision is logged for later analysis and bandit training:

```rust
pub struct RoutingDecisionLog {
    pub decision_id: String,
    pub role: String,
    pub selected_model: String,
    pub stage: CascadeStage,
    pub candidates: Vec<CandidateEntry>,
    pub reward: Option<f64>,         // filled in after task completes
}
```

</details>

---

## 15. Knowledge Store Architecture

Source: `crates/roko-neuro/src/lib.rs`

The durable knowledge store (`KnowledgeStore`/`NeuroStore`) is long-term
memory. It is separate from the Engram substrate: knowledge entries are
distilled, validated observations extracted from multiple episodes, not raw
signals. An Engram might record "this task failed with E0308"; a knowledge
entry records "type mismatches in trait impls often come from lifetime
parameter omissions."

### KnowledgeKind Taxonomy

<details>
<summary>KnowledgeKind enum</summary>

```rust
pub enum KnowledgeKind {
    Insight,          // compact causal observation from multiple episodes
    Heuristic,        // lightweight rule of thumb or learned tendency
    AntiKnowledge,    // negative knowledge: what to avoid, what has failed
    Warning,          // cautionary note about a recurring failure mode
    CausalLink,       // causal relationship between two observations
    StrategyFragment, // reusable approach fragment for larger plans
}
```

</details>

### Decay Half-Lives

Different knowledge kinds have different durability. A `Warning` is urgent and
specific; it decays in hours. An `Insight` is durable wisdom; it decays in
weeks. On-chain storage has shorter effective lifetimes because of block-based
accounting.

<details>
<summary>Decay half-life table</summary>

| Kind | Off-chain half-life | On-chain half-life |
|------|--------------------|--------------------|
| Insight | 30 days | ~7 days (1.5M blocks) |
| Heuristic | 90 days | ~15 days (3.2M blocks) |
| Warning | 1 hour | ~3 minutes (90 blocks) |
| CausalLink | 60 days | ~15 days |
| StrategyFragment | 14 days | ~15 days |
| AntiKnowledge | 30 days (default) | ~15 days |

Block rate: 1 block per 2 seconds (`BLOCKS_PER_DAY = 43,200`).

</details>

### KnowledgeTier

Entries advance through tiers as they accumulate validation. Higher tier =
slower decay and higher priority in retrieval.

<details>
<summary>KnowledgeTier enum and progression</summary>

```rust
pub enum KnowledgeTier {
    Transient,      // multiplier: 0.1x (decays very fast)
    Working,        // multiplier: 0.5x
    Consolidated,   // multiplier: 1.0x (base rate)
    Persistent,     // multiplier: 5.0x (extremely durable)
}
```

Tier progression is driven by `TierProgression` which promotes entries based
on validation count, cross-episode consistency, and C-factor contribution.

</details>

<details>
<summary>KnowledgeEntry struct</summary>

```rust
pub struct KnowledgeEntry {
    pub id: String,
    pub kind: KnowledgeKind,
    pub source: Option<String>,
    pub content: String,
    pub confidence: f64,             // [0.0, 1.0]
    pub tier: KnowledgeTier,
    pub validation_count: u32,
    pub half_life_days: f64,
    pub emotional_provenance: Option<EmotionalProvenance>,
    // ... timestamps, tags, balance (demurrage)
}
```

</details>

### Admission Control

New candidates are evaluated before entering the durable store. The store does
not accept everything — it filters, deduplicates, and resolves conflicts.

<details>
<summary>Admission pipeline</summary>

```
1. Duplicate detection (content hash + semantic similarity threshold)
2. Confidence threshold gate (default: 0.4)
3. Conflict detection with existing entries (opposing claims)
4. Capacity enforcement (LRU eviction when store is full)
```

Outcomes: `Admitted`, `Rejected(reason)`, `Merged(existing_id)`.

</details>

### Four-Factor Retrieval (Daimon Integration)

Knowledge retrieval uses a learnable four-factor scoring model. The emotional
factor means that knowledge discovered in a high-stress context gets weighted
differently when retrieved in a different affect state — the system accounts
for context.

<details>
<summary>Four-factor retrieval formula and default weights</summary>

```
score = w_recency    * recency_factor(Ebbinghaus)
      + w_importance  * importance_factor(confidence * validation_ratio)
      + w_relevance   * semantic_similarity(query, entry)
      + w_emotional   * PAD_cosine(current_mood, entry_affect)
```

Default weights: recency=0.20, importance=0.25, relevance=0.35,
emotional=0.20. Weights are online-learnable via gradient descent on retrieval
quality.

</details>

### Emotional Provenance

<details>
<summary>EmotionalProvenance struct</summary>

```rust
pub struct EmotionalProvenance {
    pub average_pad: PadVector,
    pub discovery_emotion: String,    // coarse PAD label
    pub validation_arc: Option<ValidationArc>,  // Redemptive|Contaminating|Stable|Progressive
    pub emotional_diversity: f64,     // normalized Shannon entropy of emotion labels
}
```

Knowledge validated under diverse emotional conditions gets a diversity bonus
in retrieval scoring.

</details>

---

## 16. Dream Consolidation Cycle

Source: `crates/roko-dreams/src/`

Dream consolidation is like sleeping on a problem. While agents are active,
they accumulate episodes. Offline, the dream cycle processes those episodes
to extract durable knowledge, identify recurring patterns, simulate
counterfactual scenarios, and generate routing advice.

The dream cycle is the primary mechanism by which the system improves over
time. Without it, episodes accumulate but are never distilled into reusable
knowledge.

### Four Phases

```
Phase 1: Hypnagogia
  ├── HypnagogiaEngine clusters recent episodes by plan/task shape
  ├── ThalamicGate filters low-quality clusters
  ├── ExecutiveLoosener relaxes strict structural constraints for exploration
  ├── HomuncularObserver identifies self-referential patterns
  └── DaliInterrupt handles liminal interrupts between consolidation cycles

Phase 2: Dream Cycle Core
  ├── CrossEpisodeConsolidator finds recurring structural patterns
  ├── TierProgression promotes validated knowledge entries
  ├── CFactor regression analysis (trailing 7-day window)
  └── Performance stall detection (triggers decomposition strategy change)

Phase 3: Imagination
  ├── synthesize_hypotheses() — generate counterfactual scenarios
  ├── imagine() — simulate hypothetical execution paths
  └── counterfactual_episode() — evaluate "what if different model?"

Phase 4: Rehearsal and Routing Advice
  ├── rehearse_threats() — simulate known failure modes
  ├── generate_routing_advice() — produce DreamRoutingAdvice for CascadeRouter
  └── save_dream_routing_advice() — persist to .roko/learn/dream-routing.json
```

<details>
<summary>DreamCycleReport struct</summary>

```rust
pub struct DreamCycleReport {
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub total_episodes: usize,
    pub processed_episodes: usize,
    pub analysis: TierProgressionReport,
    pub cfactor_regression: Option<CFactorRegression>,
    pub clusters: Vec<DreamClusterReport>,
    pub cross_episode_report: Option<CrossEpisodeConsolidationReport>,
    pub routing_recommendations: usize,
    pub knowledge_entries_written: usize,
    pub playbooks_created: usize,
}
```

</details>

### StagingBuffer and Confidence Stages

Episodes enter a `StagingBuffer` before full consolidation. They progress
through confidence stages based on validation count and cross-episode
corroboration:

```rust
pub enum ConfidenceStage {
    Candidate,      // just observed
    Provisional,    // seen twice
    Established,    // corroborated across multiple episodes
    Consolidated,   // promoted to KnowledgeStore
}
```

### DreamRunner Configuration

<details>
<summary>DreamLoopConfig and DreamAgentConfig</summary>

```rust
pub struct DreamLoopConfig {
    pub interval_secs: u64,          // how often to run
    pub min_episodes_per_cycle: usize,
    pub max_episodes_per_cycle: usize,
    pub budget: DreamBudget,         // token and time limits
}

pub struct DreamAgentConfig {
    pub model: String,
    pub max_tokens: u32,
}
```

</details>

---

## 17. DaimonPolicy Affect Engine

Sources:
- `crates/roko-daimon/src/lib.rs`
- `crates/roko-core/src/affect.rs`

DaimonPolicy is the affect engine that modulates agent behavior based on
the system's emotional state. Think of it as an emotional thermostat: it reads
recent success/failure history, maintains a multi-timescale internal state,
and adjusts every subsequent agent dispatch based on what it sees.

If agents have been failing repeatedly (high arousal, low pleasure), the
thermostat notices the `Struggling` state and dials down exploration, prefers
cheaper models, and reduces turn budgets. If things are going smoothly
(`Coasting`), it stays lean. If it is in a productive groove (`Focused`), it
may prefer more capable models to capitalize on momentum.

### PAD Vector

The Pleasure-Arousal-Dominance (PAD) model represents emotional state as a
point in 3D space:

<details>
<summary>PadVector struct</summary>

```rust
pub struct PadVector {
    pub pleasure: f64,    // [-1.0, 1.0]: valence (negative ↔ positive)
    pub arousal: f64,     // [-1.0, 1.0]: activation (calm ↔ excited)
    pub dominance: f64,   // [-1.0, 1.0]: control (submissive ↔ dominant)
}
```

</details>

### Three-Layer ALMA Model (Gebhard 2005)

The Daimon maintains three temporal layers. Each layer has a different time
constant: emotion reacts immediately to stimuli, mood is a rolling average of
emotion, and temperament is a slow-moving baseline. This is the same
multi-timescale model used in human affect research.

<details>
<summary>AlmaLayers struct and update equations</summary>

```rust
pub struct AlmaLayers {
    pub emotion: PadVector,           // fast: tau=0.1, reacts immediately
    pub mood: PadVector,              // medium: tau=0.5, running average
    pub temperament: PadVector,       // slow: tau=0.9, stable baseline

    pub mood_interval: u64,           // update mood every N ticks (default 10)
    pub temperament_interval: u64,    // update temperament every N ticks (default 100)
}

// Effective affect = 0.5*emotion + 0.3*mood + 0.2*temperament
pub fn effective_affect(&self) -> PadVector { ... }
```

Update equations:
```
emotion = (1 - tau_e) * emotion + tau_e * stimulus    # EMA
mood    = (1 - tau_m) * mood    + tau_m * emotion     # at mood_interval ticks
temp    = (1 - tau_t) * temp    + tau_t * mood        # at temperament_interval ticks
```

</details>

### BehavioralState Classification

<details>
<summary>BehavioralState classification rules</summary>

```rust
pub enum BehavioralState {
    Engaged,    // baseline active state
    Struggling, // repeated failure / uncertainty; escalate or conserve
    Coasting,   // succeeding cheaply; stay lean
    Exploring,  // learning / uncertain but not failing
    Focused,    // confident exploitation of known-good patterns
    Resting,    // low-demand consolidation mode
}
```

Classification from PAD + confidence:
```
confidence < 0.30 or dominance < -0.25
  or (pleasure < -0.30 and arousal > 0.30)  →  Struggling

pleasure > 0.35 and confidence > 0.65       →  Coasting
dominance > 0.30 and pleasure > 0.25        →  Focused
arousal < -0.20                             →  Resting
dominance < 0.10 and pleasure > -0.20       →  Exploring
otherwise                                   →  Engaged
```

</details>

### Somatic Markers

<details>
<summary>Somatic marker lookup via k-d tree</summary>

Somatic markers encode situation→response associations learned from past
outcomes. The `KdTree` allows fast nearest-neighbor lookup in 8-dimensional
strategy space:

```rust
pub struct DaimonState {
    pub affect: AffectState,
    somatic_tree: KdTree<f64, STRATEGY_DIMENSIONS>,  // kiddo k-d tree
    // ...
}
```

When dispatching a task, the somatic system looks up the nearest stored
situation and applies the associated emotional bias to retrieval weights and
dispatch modulation.

</details>

### Four-Factor Retrieval Weights

<details>
<summary>RetrievalWeights and update rule</summary>

```rust
pub struct RetrievalWeights {
    pub recency: f64,       // default 0.20
    pub importance: f64,    // default 0.25
    pub relevance: f64,     // default 0.35
    pub emotional: f64,     // default 0.20
}

// score = w_r*recency + w_i*importance + w_v*relevance + w_e*emotional
// Online update via gradient descent on retrieval quality outcomes
```

</details>

### DispatchModulation

The Daimon fills `DispatchModulation` before every task dispatch. The
`EffectDriver` applies it:

```
tier_bias > 0         →  prefer more capable (expensive) model
tier_bias < 0         →  prefer cheaper model
turn_limit_factor     →  multiply default turn budget
exploration_rate      →  increases temperature, may bypass cache
```

### AffectEvent Pipeline

<details>
<summary>AffectEvent enum</summary>

```rust
pub enum AffectEvent {
    TaskCompleted { succeeded: bool, tokens_used: u64, cost_usd: f64 },
    GateVerdict { gate_name: String, passed: bool, rung: u8, confidence: f64 },
    BudgetPressure { fraction_remaining: f64 },
    KnowledgeHit { confidence: f64 },
    SomaticFired { marker_id: String, similarity: f64 },
}
```

Each event is appraised into a PAD stimulus, which updates the ALMA emotion
layer via EMA.

</details>

---

## 18. SystemPromptBuilder 9-Layer Assembly

Source: `crates/roko-compose/src/system_prompt_builder.rs`

The `SystemPromptBuilder` constructs cache-aligned, role-specific system
prompts from composable fragments. Think of it as a layered document assembler:
each layer adds different content targeting a different cache tier. Stable
content (role identity, conventions) goes in the prefix that can be cached
across many calls; volatile content (current task, gate feedback) goes at the
end.

### The 9 Layers

| Layer | Content | Cache Tier |
|-------|---------|------------|
| 1 | Role identity: who am I, what is my job | System (stable) |
| 2 | Conventions: project coding standards | System (semi-stable) |
| 3 | Domain context: project-specific knowledge | Session (semi-stable) |
| 3b | Relevant assembled context (ContextProvider) | Session (semi-stable) |
| 3c | Active pheromone/stigmergic signals | Session (semi-stable) |
| 4 | Task context: current task details | Task (volatile) |
| 4b | Prior gate failure feedback (retry context) | Dynamic |
| 5 | Tool instructions: available tools | System (stable) |
| 6 | Relevant techniques: playbooks + skills + tool hints | Task (volatile) |
| 7 | Anti-patterns: what NOT to do | Task (volatile) |
| 8 | Affect guidance: emotional tone and focus | Dynamic |

<details>
<summary>SystemPromptBuilder struct fields</summary>

```rust
pub struct SystemPromptBuilder {
    role_identity: String,            // Layer 1
    conventions: Option<String>,      // Layer 2
    domain: Option<String>,           // Layer 3
    context: Option<String>,          // Layer 3b
    pheromones: Vec<ContextChunk>,    // Layer 3c
    task: Option<String>,             // Layer 4
    gate_feedback: Vec<String>,       // Layer 4b
    tools: Option<String>,            // Layer 5
    relevant_skills: Vec<Skill>,      // Layer 6
    relevant_playbooks: Vec<Playbook>,// Layer 6
    tool_hints: Option<String>,       // Layer 6b
    anti_patterns: Vec<String>,       // Layer 7
    affect_state: Option<PadState>,   // Layer 8
    temperament: Option<Temperament>, // role behavior dial
    cache_markers: bool,              // insert alignment markers between tiers
    token_budget: Option<usize>,      // enforce token cap
    budget_profile: Option<PromptBudget>, // per-layer section caps
    section_effectiveness: Option<SectionEffectivenessConfig>, // learned section priority
}
```

</details>

### Builder API

```rust
SystemPromptBuilder::new("You are an implementer...")
    .with_conventions("Use snake_case, thiserror for errors")
    .with_domain("Roko context: 18-crate orchestration toolkit")
    .with_task("Implement the gate ratchet in roko-gate")
    .with_gate_feedback(vec!["Previous compile: E0308 type mismatch on line 42"])
    .with_tools("MCP tools: Read, Write, Bash, Grep, Glob")
    .with_relevant_playbooks(playbooks)
    .with_relevant_skills(skills)
    .with_anti_patterns(vec!["Never call unwrap() in library crates"])
    .with_affect(pad_state)
    .with_cache_markers(true)
    .build();
```

### Section Effectiveness Registry

<details>
<summary>How section effectiveness is learned</summary>

```rust
pub struct SectionEffectivenessRegistry {
    // per (role, section_id): correlation between inclusion and positive outcomes
    // used to reorder layers and suppress low-signal sections
}
```

A section's priority is boosted if its presence correlates with gate passes
and task success, and reduced if it correlates with token waste.

</details>

### Cache Alignment

Layers 1+2+5 form the prefix-cacheable "system" tier. A cache alignment
marker is inserted at each tier boundary so the provider's prompt cache
can reuse the stable prefix across calls with different task contexts.

<details>
<summary>Cache boundary layout</summary>

```
[CACHE_BOUNDARY: system]
Layer 1: role identity
Layer 2: conventions
Layer 5: tools
[CACHE_BOUNDARY: session]
Layer 3: domain
Layer 3c: pheromones
[CACHE_BOUNDARY: task]
Layer 4: task
Layer 6: techniques
Layer 7: anti-patterns
Layer 4b: gate feedback
Layer 8: affect
```

</details>

---

## 19. Conductor: Reactive Intelligence Layer

Source: `crates/roko-conductor/src/lib.rs`

The Conductor watches signal streams and decides when to intervene: restart
an agent, change model, or abort a plan. It is a `React` implementation
composed of 10 independent watchers that each inspect the Engram stream for
different problems.

> **Why 10 pure watchers?** Because each watcher is testable in isolation.
> A watcher is a function from `&[Engram]` to `Vec<Engram>`. You can feed it
> a known sequence of events and verify that it fires at the right moment.
> No mocking, no side effects.

### Architecture

```
Engram stream
     │
     ├─── Watcher 1 (StuckDetector)      ┐
     ├─── Watcher 2 (AnomalyDetector)    │ all pure functions:
     ├─── Watcher 3 (CircuitBreaker)     │ &[Engram] -> Vec<Engram>
     ├─── Watcher 4 (BudgetGuardrail)    │
     ├─── Watcher 5 (HealthMonitor)      │ No side effects
     ├─── Watcher 6 (PatternDetector)    │
     ├─── Watcher 7 (SelfHealing)        │
     ├─── Watcher 8 (YerkesDodson)       │
     ├─── Watcher 9 (DiagnosisEngine)    │
     └─── Watcher 10 (FederationLayer)   ┘
              │
              ▼
     InterventionPolicy (WorstSeverityPolicy | BanditPolicy)
              │
              ▼
     ConductorDecision
       ├── Continue
       ├── Restart { reason, new_model }
       ├── Abort { reason }
       ├── ChangeModel { from, to }
       └── Escalate { severity, context }
```

### Circuit Breaker

<details>
<summary>CircuitBreaker struct and proactive trip signal</summary>

```rust
pub struct CircuitBreaker {
    // per-plan failure budget
    // Holt (double exponential smoothing) forecaster for proactive tripping
    pub state: CircuitBreakerState,  // Closed | Open | HalfOpen
}
```

`ProactiveTripSignal` fires when the Holt forecaster predicts imminent failure
budget exhaustion — trips the circuit before the budget is actually hit.

</details>

### StuckDetector

<details>
<summary>StuckDetector and MetaCognitionHook</summary>

```rust
pub struct StuckDetector {
    // ActivityEntry ring buffer
    // StuckKind: ToolLoop | OutputLoop | EmptyTurn | ProgressStall
}

pub struct MetaCognitionHook {
    // fires when StuckDetector signals
    // actions: PromptWithContext | SwitchTool | Escalate | Abort
}
```

</details>

### DiagnosisEngine

<details>
<summary>DiagnosisEngine enums</summary>

```rust
pub enum ErrorCategory {
    CompileError,
    TestFailure,
    BudgetExhaustion,
    InfiniteLoop,
    ModelCapacity,
    NetworkError,
    // ...
}

pub struct DiagnosisResult {
    pub category: ErrorCategory,
    pub confidence: f64,
    pub suggested_intervention: SuggestedIntervention,
    pub root_cause_candidates: Vec<String>,
}
```

</details>

### Federation Hierarchy

The Conductor is hierarchical. Each layer escalates to the next when local
policy cannot resolve the problem:

```
L1: Per-turn watcher (StuckDetector, AnomalyDetector)
L2: Per-task conductor (CircuitBreaker, HealthMonitor)
L3: Per-plan coordinator (PlanRevision, recovery policies)
L4: Fleet conductor (cross-plan resource allocation)
```

### Yerkes-Dodson Pressure-Performance

<details>
<summary>YerkesDodson watcher</summary>

```rust
pub struct YerkesDodson {
    // models inverted-U relationship between arousal and performance
    // low arousal (Resting): suboptimal but stable
    // optimal arousal (Engaged/Focused): peak performance
    // high arousal (Struggling): performance degrades
}
```

Used to calibrate when to inject challenge (increase arousal) vs. when to
de-escalate (reduce pressure).

</details>

---

## 20. RuntimeEvent System

Source: `crates/roko-core/src/runtime_event.rs`, `crates/roko-runtime/src/event_bus.rs`

`RuntimeEvent` is the typed event stream emitted by the workflow engine. It
is consumed by three separate sinks simultaneously: the TUI (renders to the
dashboard), SSE endpoints (pushes to HTTP clients), and the JSONL logger.

<details>
<summary>RuntimeEvent enum variants</summary>

```rust
pub enum RuntimeEvent {
    WorkflowStarted { run_id: String, prompt: String },
    PhaseTransition { run_id: String, phase: String },
    AgentThinking { run_id: String, role: String },
    AgentCompleted { run_id: String, role: String, output_len: usize },
    GateRunning { run_id: String, gate: String },
    GatePassed { run_id: String, gate: String, duration_ms: u64 },
    GateFailed { run_id: String, gate: String, output: String },
    TokensUsed { run_id: String, input: u64, output: u64, cost_usd: f64 },
    WorkflowComplete { run_id: String, outcome: WorkflowOutcome },
    ConductorIntervention { run_id: String, decision: String },
    // ... (non-exhaustive)
}
```

</details>

The `RuntimeEventBus` is a `tokio::broadcast` channel. The TUI subscribes via
`StateHub` (push-based dashboard). SSE subscribers receive events as
newline-delimited JSON.

---

## 21. HDC Primitives

Source: `crates/roko-primitives/src/hdc.rs`

Roko uses 10,240-bit hyperdimensional computing (HDC) vectors for semantic
similarity, episode clustering, and anti-noise fingerprinting. HDC is a form
of computation where meaning is encoded in high-dimensional binary vectors,
and operations like XOR (binding) and majority vote (bundling) preserve
semantic relationships.

The key property: two HDC vectors computed from similar inputs have high
Hamming similarity. Two vectors from unrelated inputs are nearly orthogonal.
This is what enables episode clustering during dream consolidation: episodes
with similar task shapes produce similar fingerprints.

<details>
<summary>HDC vector constants, operations, and codebook</summary>

```rust
pub const HDC_BITS: usize = 10_240;
pub const HDC_BYTES: usize = HDC_BITS / 8; // 1,280 bytes

pub struct HdcVector([u8; HDC_BYTES]);
```

Core HDC operations:

| Operation | Method | Semantics |
|-----------|--------|-----------|
| Binding | `a.bind(&b)` | XOR: encodes association |
| Bundling | `HdcVector::bundle(&refs)` | Majority vote: encodes set membership |
| Permutation | `v.permute(n)` | Cyclic shift: encodes position/role |
| Similarity | `a.hamming_similarity(&b)` | 0.0–1.0: measures relatedness |
| Seeding | `HdcVector::from_seed(bytes)` | Deterministic from any byte string |

**Episode fingerprinting**: Each completed episode gets an `HdcVector`
computed from its task description, gate outcomes, and role. Episodes with
similar fingerprints cluster together during dream consolidation.

**Codebook** (`crates/roko-primitives/src/codebook.rs`): Deterministic symbol
allocation, role-filler binding, pattern store, and cross-domain resonance
detection.

**TierRouter**: Maps `(InferenceTier, vitality) → model_name`:
```rust
pub enum InferenceTier { T0, T1, T2 }
```

</details>

---

## 22. Safety Architecture

Source: `crates/roko-agent/src/safety/`

### SafetyLayer

Attached to `ToolDispatcher`. Runs before and after every tool invocation:

```
Pre-execution:
  1. Role authorization (def.permission check)
  2. AgentContract check (YAML-defined safety contracts per agent)
  3. Provenance validation

Post-execution:
  1. Output scrubbing (ScrubPolicy: redact secrets, PII)
  2. Audit logging (Custody chain)
  3. Taint propagation
```

<details>
<summary>AgentContract, CustodyLogger, and ScrubPolicy</summary>

**AgentContract**

```yaml
# contracts/<agent-role>.yaml
allowed_tools: [Read, Write, Bash, Grep, Glob]
denied_patterns:
  - "rm -rf /"
  - "git push --force"
max_file_size_bytes: 10485760
```

Falls back to permissive default when YAML is missing.

**Custody Chain**

Every tool invocation is logged to the Custody audit chain:

```rust
pub struct CustodyLogger {
    // append-only JSONL at .roko/custody.jsonl
    // ForensicReplay can reconstruct causal chain from any content hash
}
```

**ScrubPolicy**

```rust
pub struct ScrubPolicy {
    // patterns: Vec<Regex> for secrets, keys, tokens
    // replaces matches with "[REDACTED]"
    // applied to all tool outputs before they reach the agent
}
```

</details>

---

## 23. Extension System

Source: `crates/roko-core/src/extension.rs`

`ExtensionChain` allows plugging in custom behavior at named hook points.
Extensions are called synchronously at task lifecycle events and can modify
context, record metrics, or trigger external notifications.

<details>
<summary>ExtensionChain and Extension trait</summary>

```rust
pub struct ExtensionChain {
    // hooks: HashMap<HookPoint, Vec<Box<dyn Extension>>>
}

pub trait Extension: Send + Sync {
    fn hook_points(&self) -> &[HookPoint];
    fn on_task_start(&self, ctx: &mut TaskContext) -> Result<()>;
    fn on_gate_result(&self, verdict: &GateVerdict) -> Result<()>;
    fn on_task_complete(&self, episode: &Episode) -> Result<()>;
}
```

Hook points: `TaskStart`, `GateResult`, `TaskComplete`, `PlanRevision`,
`ModelSelected`, `KnowledgeAdmitted`.

</details>

---

## 24. Data Flow: Signal Through the System

This section follows a request end-to-end to show how all the pieces connect.

### 24.1 Plan Execution Flow

```
roko plan run plans/
       │
       ▼
orchestrate.rs (PlanRunner)
  ├── discover_plans()        load tasks.toml files
  ├── ParallelExecutor        build UnifiedTaskDag
  └── For each task:
       │
       ├── CascadeRouter.select()     choose model
       ├── SystemPromptBuilder.build() assemble 9-layer prompt
       ├── ContextAssembler           inject knowledge
       ├── DaimonState.pre_dispatch() get AffectContext
       │
       ├── dispatch_agent() ──────────────────────────────────────────┐
       │    └── ToolDispatcher                                         │
       │         └── [validate → authorize → hook_chain → handler]    │
       │                                                               │
       ├── GatePipeline.run_rung()    verify changes                  │
       │    ├── select_rungs()        choose rungs for complexity      │
       │    ├── [Compile, Lint, Test, Symbol, ...]                     │
       │    └── AdaptiveThresholds.record()   update EMA, CUSUM       │
       │                                                               │
       ├── EpisodeLogger.record()     .roko/episodes.jsonl            │
       ├── CascadeRouter.feedback()   update bandit arm               │
       ├── DaimonState.on_outcome()   update PAD affect               │
       ├── ErrorPatternStore.record() if gate failed                  │
       └── KnowledgeAdmissionStore()  admit success entry             │
                                                                       │
                                         Agent turn loop ─────────────┘
                                           (see 24.2)
```

### 24.2 Agent Turn Loop

```
Agent receives system prompt + task prompt
  │
  ▼
[LLM call → tool calls → LLM call → ...]
  │
  Each tool call:
  ├── ToolDispatcher.dispatch(call, ctx)
  │    ├── validate schema
  │    ├── authorize by role
  │    ├── run safety hook chain
  │    ├── execute handler (Read/Write/Bash/Grep/...)
  │    ├── truncate result to 16KB
  │    └── log to CustodyLogger
  │
  └── Result appended to conversation
        │
        ▼
  LLM generates next message or stops
```

### 24.3 Learning Feedback Loop

```
Completed task
  │
  ├── EpisodeLogger             append episode to JSONL
  ├── FeedbackSink.record()     fan into learning subsystems
  │    ├── CascadeRouter        update model arm reward
  │    ├── AdaptiveThresholds   update rung EMA
  │    ├── LatencyRegistry      update rolling P50/P99
  │    ├── CostRecord           update cost DB
  │    └── SectionEffectiveness update per-section correlation
  │
  └── (offline) DreamRunner
       ├── HypnagogiaEngine     cluster episodes
       ├── TierProgression      promote validated knowledge
       ├── PlaybookStore        extract reusable patterns
       └── generate_routing_advice()  advise CascadeRouter
```

### 24.4 Knowledge Retrieval at Dispatch Time

```
Before dispatching agent for task T:
  │
  ├── KnowledgeStore.query(task_context)
  │    └── four-factor scoring: recency + importance + relevance + emotional
  │
  ├── PlaybookStore.query(role, domain)
  │    └── match by role + domain embedding similarity
  │
  ├── ErrorPatternStore.query(task_context)
  │    └── anti-knowledge: patterns to avoid
  │
  └── All injected into Layer 6 (techniques) of system prompt
```

### 24.5 Cross-Crate Signal Flow

```
                    ┌──────────────┐
                    │   roko-cli   │
                    │ orchestrate  │
                    └──────┬───────┘
                           │ dispatch_agent_with()
           ┌───────────────▼──────────────────┐
           │            roko-agent             │
           │  ToolDispatcher + SafetyLayer     │
           │  5 LLM backends                  │
           └───────┬────────────────┬──────────┘
                   │ outcomes       │ tool calls
           ┌───────▼──────┐  ┌─────▼──────────┐
           │  roko-learn  │  │    roko-std     │
           │  EpisodeLog  │  │  19 builtin     │
           │  Cascade     │  │  tool handlers  │
           │  Playbooks   │  └────────────────┘
           └───────┬──────┘
                   │ distillation
           ┌───────▼──────┐
           │  roko-neuro  │
           │  Knowledge   │
           │  Store       │
           └───────┬──────┘
                   │ consolidation
           ┌───────▼──────┐
           │ roko-dreams  │
           │  DreamCycle  │
           └──────────────┘
                   │ routing advice
                   └──────────────► CascadeRouter (roko-learn)
```

---

## Appendix: Key File Index

| Component | Path |
|-----------|------|
| Core traits (6 verbs) | `crates/roko-core/src/traits.rs` |
| Foundation service traits | `crates/roko-core/src/foundation.rs` |
| Engram type | `crates/roko-core/src/engram.rs` |
| Universal loop | `crates/roko-core/src/loop_tick.rs` |
| Affect primitives | `crates/roko-core/src/affect.rs` |
| Cell supertrait | `crates/roko-core/src/cell.rs` |
| PipelineStateV2 | `crates/roko-runtime/src/pipeline_state.rs` |
| WorkflowEngine | `crates/roko-runtime/src/workflow_engine.rs` |
| EffectDriver | `crates/roko-runtime/src/effect_driver.rs` |
| PlanRunner (orchestrate) | `crates/roko-cli/src/orchestrate.rs` |
| ToolDispatcher | `crates/roko-agent/src/dispatcher/mod.rs` |
| Gate pipeline | `crates/roko-gate/src/gate_pipeline.rs` |
| Adaptive thresholds | `crates/roko-gate/src/adaptive_threshold.rs` |
| CascadeRouter | `crates/roko-learn/src/cascade_router.rs` |
| LinUCB router | `crates/roko-learn/src/model_router.rs` |
| EpisodeLogger | `crates/roko-learn/src/episode_logger.rs` |
| SystemPromptBuilder | `crates/roko-compose/src/system_prompt_builder.rs` |
| DaimonState | `crates/roko-daimon/src/lib.rs` |
| DreamCycle | `crates/roko-dreams/src/cycle.rs` |
| KnowledgeEntry types | `crates/roko-neuro/src/lib.rs` |
| Conductor | `crates/roko-conductor/src/conductor.rs` |
| CircuitBreaker | `crates/roko-conductor/src/circuit_breaker.rs` |
| StuckDetector | `crates/roko-conductor/src/stuck_detection.rs` |
| HdcVector | `crates/roko-primitives/src/hdc.rs` |
| TierRouter | `crates/roko-primitives/src/tier.rs` |
| HTTP control plane | `crates/roko-serve/src/routes/` |
| Per-agent sidecar | `crates/roko-agent-server/src/` |
| Builtin tools | `crates/roko-std/src/tools/` |
| Runtime event bus | `crates/roko-runtime/src/event_bus.rs` |
