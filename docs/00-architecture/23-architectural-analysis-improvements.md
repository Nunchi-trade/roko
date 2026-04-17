# Architectural Coherence Analysis and Improvements

> **Abstract:** A comprehensive analysis of Roko's current architecture as a v1 snapshot and
> its v2 rewrite path: the six-operator snapshot, five-layer taxonomy, three cognitive speeds,
> Engram/Pulse split, two mediums (durable Engram and ephemeral Pulse), two fabrics (Substrate
> and Bus), and cognitive cross-cuts — evaluated against modern research in cognitive
> architectures (SOAR, ACT-R, LIDA), trait-based systems (Scala typeclasses, Haskell type
> classes), category theory (functors, monoids, natural transformations), and active inference
> (Free Energy Principle, VERSES Genius). This document identifies architectural strengths,
> coherence gaps, layer violations, rewrite boundaries, and proposes improvements grounded in
> academic literature. See also `tmp/refinements/01-critique-one-noun.md`,
> `tmp/refinements/03-bus-as-first-class.md`,
> `tmp/refinements/04-operators-generalized.md`,
> `tmp/refinements/21-from-scratch-redesigns.md`, and
> [01-naming-and-glossary.md](./01-naming-and-glossary.md).

> **Implementation**: Analysis (informing future architectural decisions, including from-scratch sequencing)
> See also `tmp/refinements/20-modularity-composability.md` for the dep graph rewrite boundary.

**Date**: 2026-04-12
**Methodology**: Read all 24 architecture docs, all 22 section INDEX files, STATUS/QUICKSTART/COMPARISON,
analyzed actual Cargo.toml dependency graphs across all 28 crates, read trait definitions in code
(`roko-core/src/traits.rs`), and surveyed recent literature.

---

## 1. Executive Summary

Roko's current architecture is **remarkably coherent** as a v1 snapshot, but the current shape
is not the endpoint. The architecture docs now need to treat the two-medium, two-fabric kernel
as the v2 rewrite path: Engram and Pulse moving through Substrate and Bus. The five-layer
taxonomy has **one confirmed dependency violation** (`roko-conductor` L3/L4 → `roko-learn`
L2/Cross-cut) and **six unclassified crates**. The three cognitive speeds map cleanly to all
three primary domains (coding, chain, research). The current Engram model is genuinely
universal, with edge cases that are handled by existing extension mechanisms (`Kind::Custom`,
`Body::Json`, `tags`).

REF01 is the diagnostic bridge for that shift: it shows that the old "one noun, six verbs"
mnemonic still captures the durable half of the system, but it no longer explains live traffic,
the bus, or the boundary cases that this analysis documents below.

Key findings:
1. **The current operator set is coherent, but v2 planning changes the question.** The
   from-scratch kernel rewrite in REF21 is the clean path once Pulse and Bus semantics are
   first-class.
2. **One dependency violation exists, and the Bus story dissolves it.** `roko-conductor` →
   `roko-learn` breaks the L3→L2 rule only because circuit-breaker state was modeled as a direct
   learning dependency. The Bus-first fix routes that state through `gate.verdict.emitted` and
   `gate.failure.rate` topics on the kernel `Bus` trait, so `roko-conductor` and `roko-learn`
   stay decoupled at compile time.
3. **Category theory provides formal grounding.** The pipeline is a composition of morphisms;
   Score is a monoid; cross-cuts are endofunctors. These aren't metaphors — they're structural
   properties that guarantee composability.
4. **Active inference reframes the Gate.** The Gate should be reconceived as a prediction-error
   detector (not just pass/fail), enabling continuous model updating.
5. **LIDA-style competitive attention** would strengthen the Router by allowing multiple Scorers
   to form competing coalitions, implementing Global Workspace Theory's consciousness spotlight.

---

## 2. Analysis A: Are the Six Synapse Traits Sufficient?

### 2.1 Methodology

Analyzed all 131 trait implementations found in the codebase:
- Substrate: 4 implementations (MemorySubstrate, FileSubstrate, HdcSubstrate, ChainSubstrate)
- Scorer: 7 implementations (SumScorer, MulScorer, ConstScorer, RelevanceScorer, ReputationScorer, ToolRelevanceScorer, NoOpScorer)
- Gate: 33 implementations (ShellGate, PropertyTestGate, GeneratedTestGate, IntegrationGate, WalletGate, TxSimGate, VerifyChainGate, LlmJudgeGate, FactCheckGate, SymbolGate, GatePipeline, OrderedGate, MockGate, NoOpGate, and 19 more)
- Router: 7 implementations (FirstRouter, HighestScoreRouter, RoundRobinRouter, CascadeRouter, LinUCBRouter, WeightedRouter, NoOpRouter)
- Composer: 5 implementations (PromptComposer, ContextPackComposer, PlanComposer, SystemPromptBuilder, NoOpComposer)
- Policy: 4 implementations (EpisodePolicy, ConductorPolicy, PheromonPolicy, NoOpPolicy)

Also searched for TODO/HACK/FIXME/workaround markers near trait usage. Found 8 TODOs, all in
UI/API boundary code (`roko-cli/src/tui/`, `roko-serve/src/routes/`), none in core trait usage.

### 2.2 Boundary Operations

Three operations sit at the boundary of the trait model, and REF04 shows that they are better
understood as `Datum`-aware operators rather than special cases:

| Operation | Current Implementation | Fit Quality |
|---|---|---|
| **Engram transformation** (e.g., summarize, translate) | `Composer::compose(&[Datum], &Budget, &dyn Scorer, ctx)` | Adequate. Composer can ingest either medium, so the same path can absorb mixed Engram/Pulse context without inventing a separate boundary abstraction. |
| **Telemetry emission** (metrics, traces) | `Policy::decide(&[Pulse], ctx)` returning `PolicyOutputs` | Better fit. Policy is intentionally Pulse-native, so live telemetry and reactive follow-ups can emit both Pulses and Engrams directly instead of masquerading as an empty Engram slice. |
| **Batch verification** (verify N Engrams at once) | Loop calling `Gate::verify` N times, or `Gate::verify_stream(&[Pulse], ctx)` for stream windows | Adequate. Gate keeps the durable Engram path while also supporting stream verification over Pulses when the boundary is temporal rather than stored. |

The trait boundary now lines up with the medium split: `Datum` covers polymorphic Scorer and
Composer inputs; Router gains a native Pulse-selection path beside its durable Engram path;
`Gate::verify_stream` covers verification over Pulse windows; `Policy::decide(&[Pulse], ctx)`
and `PolicyOutputs` make stream reaction explicit; and the Bus handles Pulse transport while
Substrate remains the storage path for Engrams. REF01 treats that boundary as evidence for the
two mediums and two fabrics reframing rather than as a harmless quirk.

### 2.3 When a rewrite beats incremental refactor

The interesting question is no longer whether the current trait count can be defended; it is
whether the kernel assumptions should be rewritten cleanly. Incremental refactor wins when the
changed assumption is local, the interface surface is already large, and the new capability can
be layered without changing the medium model.

REF21 argues the opposite here. The current design collapses durable and ephemeral traffic into
one surface, but the v2 path wants two mediums moving through two fabrics: Engram and Pulse over
Substrate and Bus. That is a kernel-level assumption, so stretching the old shape would leave
the architecture split between old and new mental models.

The from-scratch list in `tmp/refinements/21-from-scratch-redesigns.md` spells out the heuristic,
the five candidates, and the sequencing. In this doc's framing, the kernel rewrite is the first
v2 move, not a late optimization.

### 2.4 Could Traits Be Merged?

| Candidate Merge | Argument For | Argument Against | Verdict |
|---|---|---|---|
| Scorer + Router | Both evaluate candidates | Router has `feedback()` (stateful); Scorer is stateless and pure | **No merge** |
| Gate + Scorer | Both assess quality | Gate is async (external I/O); Scorer is sync (pure computation). Gate returns Verdict with rich evidence; Scorer returns Score. | **No merge** |
| Policy + Gate | Both examine outputs | Policy is reactive (many→many, no verdict); Gate is verificatory (one→Verdict). Fundamentally different cardinalities. | **No merge** |
| Scorer + Gate | Could merge into "Assessor" | Different output types, different execution models, different layer assignments | **No merge** |

All merge candidates fail because the traits differ in at least two of: sync/async, stateful/
stateless, input cardinality, output type, and layer assignment.

### 2.5 Comparison to Other Trait-Based Agent Systems

| System | Number of Core Abstractions | Roko Equivalent |
|---|---|---|
| **CoALA** (Sumers et al. 2023) | 5 stores + 3 action types | Roko's 6 traits subsume CoALA's decomposition |
| **LIDA** (Franklin et al. 2016) | Codelets (perception, attention, action, learning) | Each codelet type maps to a trait implementation |
| **Google multi-agent patterns** (2025) | 3 execution primitives (sequential, loop, parallel) | Orchestrator composes trait calls in these patterns |
| **Agent Design Pattern Catalogue** (arXiv:2405.10467) | 18 patterns | Patterns compose from trait implementations; not a competing decomposition |

Roko's trait decomposition is **coarser than codelet architectures** (LIDA may have hundreds
of codelet types) but **finer than framework abstractions** (LangChain's "chain" is coarser
than any single Synapse trait). The six-trait level appears to be the right granularity for
a Rust trait system: fine enough for meaningful composition, coarse enough for human reasoning.

**References:**
- Koopmans et al. (2024). "Agent Design Pattern Catalogue." arXiv:2405.10467
- Google Cloud (2025). "Choose a Design Pattern for Agentic AI Systems."
- Franklin, S. et al. (2016). "LIDA: A Systems-level Architecture." IEEE Trans. AMD 6(1).

---

## 3. Analysis B: Five-Layer Taxonomy Coherence

### 3.1 Dependency Audit

Full Cargo.toml analysis across all 28 crates shows a mostly clean layer story, but the dep graph still has a few pressure points that should be treated as architectural signals rather than isolated mistakes.

#### 3.1.1 Current-state audit

**Clean layers (no violations):**
- **L0** (kernel core, filesystem substrate, standard runtime, runtime support, vector-symbolic primitives): Zero upward deps.
- **L1** (roko-agent, roko-index, roko-lang-*): Depend only on L0. One dev-dependency exception.
- **L2** (roko-compose, roko-learn): Depend on L0 and L1. Clean.
- **L4** (roko-cli, roko-orchestrator): Depend on all layers. Expected for entry points.

**Current-state coupling findings:**
- `roko-agent` reaches into `roko-learn` for persistence-oriented learning hooks, which means the agent side is still writing across a boundary that should be carried by the Bus.
- `roko-cli` imports from almost everything. Some of that is legitimate for the main entry point, but some of it is accidental coupling that makes the binary look flatter than the architecture really is.
- `roko-fs` and `roko-std` are too loosely separated in the docs and their surrounding imports. The storage/runtime boundary is correct in spirit, but the present shape does not make it crisp enough.
- HDC still leaks through `roko-primitives` in places where a focused `roko-hdc` boundary would be cleaner.
- `roko-compose` still keeps templates too close to the compose engine. That makes role/template growth harder than it should be.
- There is no `roko-bus` crate yet, so bus behavior still lives inside runtime code instead of having its own kernel-level boundary.

**Violations:**

| From | To | Type | Severity |
|---|---|---|---|
| `roko-conductor` (L3/L4) | `roko-learn` (L2/Cross-cut) | Direct compile-time dependency | **Medium** |
| `roko-agent` (L1) | `roko-learn` (L2/Cross-cut) | Dev-dependency only | **Low** |

### 3.2 The roko-conductor Violation

**Root cause**: `roko-conductor` imports learning types for circuit-breaker state tracking.
The Conductor needs to react to failure-rate Pulses, but the current implementation reaches
across into `roko-learn` instead of consuming the shared transport primitive and topic boundary.

**Bus-first fix**: publish the circuit-breaker facts on the kernel `Bus` as topics and let both
subsystems subscribe to the same live stream:

- `gate.verdict.emitted` carries the gate result from the verification path.
- `gate.failure.rate` carries the learned failure-rate Pulse that `roko-conductor` needs.

That dissolves the layer violation without introducing a separate `HealthMetrics` trait in
`roko-core`. `roko-conductor` depends on the `Bus` trait and a `TopicFilter`; `roko-learn`
publishes the learned rate as a Pulse; `Gate` can verify Pulse windows directly; `Router` can
select among live Pulse candidates; and `Composer` can fold mixed `Datum` inputs when the
conductor needs a synthesized control packet rather than a stored episode. The compile-time
dependency between `roko-conductor` and `roko-learn` disappears because the shared contract is
now the Bus fabric and its topics, not a direct crate dependency or a bespoke trait object.

REF03 and REF04 together are the load-bearing reframing for this section: the architecture
already contains two mediums and two fabrics, so the conductor problem belongs in topic routing,
Datum-aware operator boundaries, and Pulse stream handling rather than in a bespoke health
interface.

REF20 sharpens that conclusion one level further: this is not just a local fix, it is the kind of dependency-audit failure that becomes trivial once the target dep graph moves bus behavior into `roko-bus` and makes the conductor consume topics instead of learning internals.

### 3.3 Unclassified Crates

Six crates need formal layer assignment:

| Crate | Recommended Classification | Rationale |
|---|---|---|
| `roko-neuro` | **Cross-cut** | Bridges L0-L2 for knowledge; inject via `&dyn Substrate` |
| `roko-daimon` | **Cross-cut** | No upward deps (only roko-core); inject via PAD trait object |
| `roko-dreams` | **Cross-cut** | Bridges Neuro + Daimon at Delta frequency |
| legacy umbrella crate | **Phase 2+ umbrella** | Contains Daimon and Dreams code pending dissolution |
| `roko-chain` | **L1 Domain Plugin** | Analogous to roko-agent for chain domain |
| `roko-plugin` | **L1 Framework** | Plugin SDK extending the tool/agent system |

### 3.4 Proposed Target Dep Graph

REF20 proposes a clean rewrite boundary for the dep graph rather than another round of ad hoc coupling repairs. The target shape is:

- `roko-core` stays the shared type-and-trait nucleus.
- `roko-bus` becomes the kernel transport crate for Bus traits, topics, and in-process broadcast primitives.
- `roko-hdc` becomes the focused hyperdimensional computing crate instead of leaking HDC through `roko-primitives`.
- `roko-spi` holds the extension SPI so plugins do not need to depend on kernel crates directly.
- `roko-std` splits into `roko-defaults` and `roko-tools`.
- `roko-compose` splits into `roko-compose-core` and `roko-templates`.

That target graph makes the kernel boundary explicit: storage, transport, hyperdimensional primitives, and plugin surface are separate crates; implementations sit below them; composition and templates are no longer coupled as one package. In analysis terms, this is the clean rewrite boundary that the current dep graph is trying to imply but has not yet made real.

### 3.5 Layer Taxonomy Completeness

The five layers map cleanly to Beer's VSM:

| VSM System | Layer | Function | Clean? |
|---|---|---|---|
| System 1 (Operations) | L0 Runtime | Process lifecycle, I/O | Yes |
| System 2 (Coordination) | L1 Framework | Prevent conflict between agents | Yes |
| System 3 (Control) | L2 Scaffold | Optimize resource allocation (context, tokens) | Yes |
| System 3* (Audit) | L3 Harness | Verify quality | Yes |
| System 4 (Intelligence) | L4 Orchestration | Plan, adapt, look forward | Yes |
| System 5 (Policy) | L4 + Daimon | Identity, self-model | Partially — System 5 spans L4 and a cross-cut |

The only imperfect mapping is System 5 (Policy/Identity), which spans both the L4 orchestration
layer and the Daimon cross-cut. This is acceptable because Beer's VSM explicitly allows System 5
to draw from multiple subsystems — it is the meta-system that integrates all others.

---

## 4. Analysis C: Three Cognitive Speeds

### 4.1 Domain Mapping Completeness

| Domain | Gamma (~5-15s) | Theta (~75s) | Delta (~hours) | Clean? |
|---|---|---|---|---|
| **Coding** | Compile check, quick fix, cached lookup | Summarize progress, check predictions, update PAD | Dreams replay of failed compilations, knowledge promotion | **Yes** |
| **Chain** | Gas check, balance check, price lookup | Portfolio assessment, hedging check, prediction calibration | MEV incident analysis, strategy consolidation | **Yes** |
| **Research** | Citation lookup, fact check | Research direction assessment, contradiction detection | Cross-domain hypothesis generation, literature synthesis | **Yes** |
| **Orchestration** | Task status check, agent health probe | Plan progress summary, re-planning assessment | Full plan retrospective, skill library update | **Yes** |

All domains map cleanly. The key insight: the three speeds are **domain-agnostic** because they
are defined in terms of the universal cognitive loop, not domain-specific operations. Any operation
that can be expressed as `query → score → route → compose → act → verify → persist → react` can
run at any of the three speeds.

### 4.2 Comparison to Classical Architectures

| Architecture | Number of Speeds | Roko Equivalent |
|---|---|---|
| **SOAR** | 1 (~50ms decision cycle) | Roughly Gamma, with impasses escalating to deeper reasoning |
| **ACT-R** | 1 (~50ms production fire) | Roughly Gamma; no explicit reflective or consolidation speed |
| **LIDA** | 1 (~260-390ms cognitive cycle) | Roughly Gamma; deliberation is a subphase, not a separate speed |
| **SOFAI** | 2 (Fast/Slow) | Fast ≈ Gamma, Slow ≈ Theta; no Delta equivalent |
| **Roko** | 3 (Gamma/Theta/Delta) | Extends dual-process with offline consolidation |

Roko's three speeds are a genuine architectural innovation. The Delta speed (offline consolidation
via Dreams) has no direct analog in established cognitive architectures. It is inspired by sleep
neuroscience (McClelland et al. 1995, CLS theory) rather than cognitive architecture tradition.

### 4.3 Speed Interaction Model

The three speeds are not independent — they interact through the cross-cuts:

```
Gamma ticks produce episodes → stored in Substrate
    │
    ├── Theta reads recent episodes → summarizes → updates Daimon PAD
    │       │
    │       └── PAD changes may trigger speed escalation or consolidation
    │
    └── Delta reads accumulated episodes → Dreams replay → Neuro promotion
            │
            └── Promoted knowledge available to next Gamma tick
```

This is a **hierarchical prediction error cascade**: Gamma handles immediate surprises, Theta
handles accumulated pattern changes, and Delta handles deep structural learning. Each speed's
output feeds the next speed's input, creating the autocatalytic loop described in
16-autocatalytic-and-cybernetics.md.

---

## 5. Analysis D: Engram Universality and Edge Cases

### 5.1 What the Universal Type Handles Well

| Data Category | Engram Representation | Fit |
|---|---|---|
| LLM output | `Kind::AgentOutput`, `Body::Text(response)` | Excellent |
| Gate verdict | `Kind::GateVerdict`, `Body::Json(verdict_data)` | Excellent |
| Code file | `Kind::PromptSection`, `Body::Text(code)` | Good |
| Binary artifact | `Kind::Custom("artifact")`, `Body::Bytes(data)` | Good |
| Prediction | `Kind::Prediction`, `Body::Json(claim)` | Excellent |
| Metric | `Kind::Metric`, `Body::Json(metric_data)` | Excellent |
| Pheromone | `Kind::Pheromone`, `Body::Json(pheromone)` + `Decay::HalfLife` | Excellent |

### 5.2 Edge Cases That Stress the Type

| Edge Case | Problem | Current Handling | Adequacy |
|---|---|---|---|
| **Large binary blobs** (e.g., model weights) | Engram struct held in memory | `Body::Bytes` exists but no streaming | Adequate for current use; streaming needed at scale |
| **Structured multi-part data** (e.g., PR with title + body + files) | Single Body can't hold structured parts | `Body::Json` with nested structure | Adequate but verbose |
| **Cross-Engram relationships** (e.g., "this gate verdict is about that agent output") | Lineage is a Vec of parent hashes | Lineage + tags (`"target_id": hash`) | Adequate |
| **Real-time streaming data** (e.g., live price feed) | Engram is a snapshot, not a stream | Create new Engrams per tick with Decay::TTL | Adequate; TTL handles ephemerality |
| **Confidential data** (e.g., API keys in context) | Provenance.tainted exists but no encryption | Taint flag + scrub policy | Adequate for current threat model |

### 5.3 Comparison to Agent Data Protocol (ADP)

The Agent Data Protocol (arXiv:2510.24702) addresses exactly the same problem: universal data
representation for agent systems. ADP unifies all agent data into Trajectory objects composed of
Actions (API calls, code execution, text exchange) and Observations (text, web).

| Dimension | ADP | Roko Engram |
|---|---|---|
| **Universal type** | Trajectory | Engram |
| **Identity** | Sequential index | Content-addressed (BLAKE3 hash) |
| **Quality assessment** | None | 4-axis Score (confidence, novelty, utility, reputation) |
| **Temporal dynamics** | None | Four Decay variants (None, HalfLife, TTL, Ebbinghaus) |
| **Trust tracking** | None | Provenance (author, trust, tainted, session) |
| **Composition** | Concatenation | Composer trait with budget constraints |
| **Complexity reduction** | O(D+A) vs O(D×A) | Same: universal type enables O(D+A) integration |

Roko's Engram is strictly richer than ADP's Trajectory: it adds scoring, decay, provenance,
content-addressing, and lineage tracking. The ADP paper validates the core insight that a
universal type reduces integration complexity from multiplicative to additive.

### 5.4 VSA/HDC Algebraic Extension

The vector-symbolic primitives crate provides 10,240-bit Hyperdimensional Computing vectors.
These could extend the Engram with algebraic operations:

```rust
// Potential extension: Engram algebraic operations
impl Engram {
    /// Bind two Engrams: creates an association (XOR in HDC space)
    pub fn bind(&self, other: &Engram) -> Engram { /* ... */ }

    /// Bundle Engrams: creates a superposition (majority vote in HDC space)
    pub fn bundle(engrams: &[Engram]) -> Engram { /* ... */ }

    /// Permute: creates a sequential ordering (cyclic shift in HDC space)
    pub fn permute(&self, position: usize) -> Engram { /* ... */ }
}
```

This would make Engram a proper Vector Symbolic Architecture element, enabling compositional
knowledge representation directly at the type level. Currently, HDC operations exist in the
vector-symbolic primitives crate but are not exposed on the Engram struct.

**Reference**: Kleyko, D. et al. (2022). "A Survey on Hyperdimensional Computing."
Artificial Intelligence Review 56.

---

## 6. Analysis E: Cross-Cut Isolation

### 6.1 Trait Object Injection Pattern

The cross-cuts (Neuro, Daimon, Dreams) are injected via trait objects, which is the correct
Rust pattern for cross-cutting concerns. Analysis of the actual injection points:

| Cross-Cut | Injection Mechanism | Trait Used | Isolation Quality |
|---|---|---|---|
| **Neuro** | `&dyn Substrate` (NeuroStore implements Substrate) | Substrate | **Good**: consumers don't know they're accessing knowledge vs. generic storage |
| **Daimon** | PAD vector passed as context/config values | Custom structs | **Adequate**: not trait-object injected; uses direct struct access |
| **Dreams** | Delta-frequency timer triggers DreamRunner | Scheduled execution | **Adequate**: runs independently but reads/writes directly to Neuro |

### 6.2 Isolation Gaps

| Gap | Impact | Recommendation |
|---|---|---|
| Daimon is not injected via trait object | L0/L1 code must import `roko-daimon` types directly | Define an `AffectModel` trait in `roko-core` with `fn pad(&self) -> PadVector` and `fn behavioral_state(&self) -> BehavioralState` |
| Dreams directly imports roko-neuro and roko-learn | Creates hidden coupling between cross-cuts | Dreams should receive `&dyn Substrate` and `&dyn EpisodeStore` trait objects |
| Arbitration protocol not yet implemented | Cross-cut conflicts resolved ad hoc | Implement the VCG arbitration described in 13-cognitive-cross-cuts.md Section 6 |

### 6.3 Functorial Composition Properties

As analyzed in the categorical framework (Section 10 of 06-synapse-traits.md), cross-cuts
form endofunctors on the Engram category. For the functorial composition to be correct,
the following diagram must commute:

```
                  Neuro
Pipeline ─────────────────→ Enriched Pipeline
    │                              │
    │ Daimon                       │ Daimon
    │                              │
    ▼                              ▼
Modulated Pipeline ───────→ Enriched + Modulated Pipeline
                  Neuro
```

That is: enriching with knowledge and then modulating with affect must produce the same
result as modulating first and then enriching. The arbitration protocol (priority hierarchy +
VCG tiebreaker) ensures this commutativity by defining a canonical resolution order.

---

## 7. Analysis F: Category Theory Perspectives

### 7.1 The Engram Category (Eng)

**Objects**: Types in the pipeline — `Vec<Engram>`, `Engram`, `Score`, `Selection`, `Verdict`

**Morphisms**: Trait operations, parameterized by `Context`:
- `query_ctx : 1 → Vec<Engram>` (Substrate)
- `score_ctx : Engram → Score` (Scorer)
- `select_ctx : Vec<Engram> → Option<Selection>` (Router)
- `compose_ctx : (Vec<Engram>, Budget) → Engram` (Composer)
- `verify_ctx : Engram → Verdict` (Gate)
- `decide_ctx : Vec<Engram> → Vec<Engram>` (Policy)

**Identity morphisms**: NoOp implementations (NoOpScorer, NoOpRouter, NoOpComposer, etc.)

**Composition**: Pipeline steps compose via standard function composition. The pipeline
`query >> select >> compose >> verify >> persist >> decide` is an arrow in the category
of "Eng-valued computations."

### 7.2 Score as a Commutative Monoid

```
(Score, +, Score::ZERO)     — additive identity: {confidence: 0, novelty: 0, utility: 0, reputation: 0}
(Score, ×, Score::NEUTRAL)  — multiplicative identity: {confidence: 1, novelty: 0, utility: 0, reputation: 1}
```

Both operations are:
- **Associative**: (a + b) + c = a + (b + c)
- **Commutative**: a + b = b + a
- **Have identity**: a + 0 = a, a × 1 = a

The multiplicative monoid is particularly important for the effective score formula:
`effective = confidence × (1 + novelty) × (1 + utility) × reputation`

This is a **monoid homomorphism** from the product monoid (Score, ×) to the positive reals
(ℝ⁺, ×). Monoid homomorphisms preserve composition, which means composing Scores and then
computing effective is the same as computing effective on each and multiplying.

### 7.3 Verdict as a Filtered Monoid

Verdicts form a monoid under sequential composition (pipeline of gates):

```
verdict₁ ∘ verdict₂ = {
    passed: verdict₁.passed && verdict₂.passed,
    score: min(verdict₁.score, verdict₂.score),
    // other fields merged
}
```

This is a **filtered monoid**: the `passed` field acts as a filter, and once any gate fails
(passed = false), the pipeline short-circuits. This is the categorical dual of the Maybe monad
— composition stops on the first failure.

### 7.4 Pipeline as Kleisli Composition

The full pipeline involves effects (async I/O, failure, state) and can be modeled as Kleisli
composition in a monad:

```
Pipeline = Substrate.query >=> Router.select >=> Composer.compose >=> Gate.verify >=> Substrate.put >=> Policy.decide
```

Where `>=>` is Kleisli composition in the `Result<T, RokoError>` monad. Each step may fail,
and failure short-circuits the pipeline (like the Verdict monoid, but at the pipeline level).

### 7.5 Functorial Cross-Cuts (Formal)

Define the cross-cut functors:

```
N : Eng → Eng    (Neuro: enrich with knowledge)
D : Eng → Eng    (Daimon: modulate with affect)
R : Eng → Eng    (Dreams: consolidate with replay)
```

The claim: N, D, and R are endofunctors. This requires:
1. **Identity preservation**: N(id) = id (enriching a no-op produces a no-op)
2. **Composition preservation**: N(f ∘ g) = N(f) ∘ N(g) (enriching a pipeline = enriching each step)

Both hold because cross-cuts inject additional information without changing the pipeline
structure. The NoOp implementations serve as witnesses for identity preservation.

**Natural transformations** between cross-cuts:
```
η : N → D    (knowledge outcomes update affect)
ε : D → N    (affect biases knowledge retrieval)
```

These form an adjunction if the arbitration protocol correctly resolves conflicts — the
priority hierarchy (Daimon > Neuro > Dreams) establishes the adjunction's unit and counit.

### 7.6 Implications

The categorical analysis reveals that Roko's composability is not accidental — it is a
structural property of the architecture. The six traits are morphisms in a category; Score
is a monoid; cross-cuts are endofunctors. Any new feature that preserves these categorical
properties will compose correctly with existing code. Any feature that violates them
(e.g., a Gate that mutates shared state outside the Substrate) will break composition.

**Design rule derived from category theory**: Every new trait implementation must:
1. Accept and return types that are objects in the Engram category
2. Preserve the monoidal structure of Score (no Score that breaks associativity)
3. Be implementable as a natural transformation on the pipeline (no hidden side effects)

---

## 8. Novel Proposals

### 8.1 Competitive Attention (LIDA-Inspired Router Enhancement)

**Current state**: Router receives candidates scored by a single Scorer, selects one.

**Proposal**: Implement LIDA-style competitive attention where multiple Scorers run
concurrently, form coalitions, and compete for the Router's selection:

```rust
pub struct CompetitiveRouter {
    scorers: Vec<Box<dyn Scorer>>,
    coalition_threshold: f32,
    inner_router: Box<dyn Router>,
}

impl Router for CompetitiveRouter {
    fn select(&self, candidates: &[Engram], ctx: &Context) -> Option<Selection> {
        // 1. Each scorer independently scores all candidates
        let score_matrix: Vec<Vec<Score>> = self.scorers.iter()
            .map(|s| candidates.iter().map(|c| s.score(c, ctx)).collect())
            .collect();

        // 2. Form coalitions: scorers that agree on the top candidate
        let coalitions = form_coalitions(&score_matrix, self.coalition_threshold);

        // 3. Strongest coalition's top candidate wins
        let winning_coalition = coalitions.into_iter()
            .max_by_key(|c| c.members.len())?;

        // 4. Inner router selects from the winning coalition's candidates
        let coalition_candidates: Vec<Engram> = winning_coalition.top_candidates
            .iter()
            .filter_map(|&idx| candidates.get(idx).cloned())
            .collect();

        self.inner_router.select(&coalition_candidates, ctx)
    }
}
```

**Theoretical basis**: LIDA's attention codelets (Franklin et al. 2016) demonstrate that
competitive attention produces more robust selection than single-scorer evaluation. The
"consciousness spotlight" (Global Workspace Theory, Baars 1988) is the winning coalition
that broadcasts its content to all subsystems.

**Integration**: CompetitiveRouter implements the Router trait, so it plugs directly into
`loop_tick` without any changes to the pipeline structure.

### 8.2 Gradient Gate Feedback (Active Inference Enhancement)

**Current state**: Gate returns binary pass/fail Verdict. Learning uses the boolean.

**Proposal**: Use the Gate's confidence score (verdict.score ∈ [0,1]) as a continuous
learning feedback value, not just the boolean:

```rust
// After gate verification in loop_tick
let verdict = gate.verify(&composed, ctx).await;

// Continuous feedback to Router (not just success/failure)
let outcome = Outcome {
    selection: selection.clone(),
    success: verdict.passed,
    reward: verdict.score,  // Use continuous score, not binary
    cost: Some(inference_cost),
    latency_ms: Some(elapsed.as_millis() as u64),
};
router.feedback(&outcome);

// Active inference: high surprise → generate a learning update
if verdict.score < 0.3 {
    // High prediction error → create insight for Neuro
    let insight = Engram::builder()
        .kind(Kind::Insight)
        .body(Body::Json(json!({
            "gate": verdict.gate,
            "error": verdict.reason,
            "context": ctx.goal,
        })))
        .provenance(Provenance::trusted("gate-learner"))
        .build();
    substrate.put(insight).await?;
}
```

**Theoretical basis**: Active inference (Friston 2010) frames verification as free energy
minimization. The Gate's confidence score is a direct measure of prediction error. Using it
as a continuous learning feedback value (not binary) enables gradient-based model updating.

**Integration**: This enhancement modifies `loop_tick` behavior but preserves its signature.
All existing trait implementations continue to work.

### 8.3 Hierarchical Pipeline Composition

**Current state**: Each cognitive speed runs the same `loop_tick` with different parameters.

**Proposal**: Formalize the relationship between speeds as monoid homomorphisms:

```rust
/// A pipeline is a configured loop_tick — a closure over trait implementations.
pub struct Pipeline {
    substrate: Arc<dyn Substrate>,
    scorer: Arc<dyn Scorer>,
    router: Arc<dyn Router>,
    composer: Arc<dyn Composer>,
    gate: Arc<dyn Gate>,
    policy: Arc<dyn Policy>,
}

impl Pipeline {
    /// Fold multiple pipeline outputs into a single pipeline input.
    /// This is the monoid homomorphism that connects cognitive speeds.
    pub fn fold_outcomes(outcomes: &[TickOutcome]) -> Query {
        // Theta folds Gamma outcomes; Delta folds Theta outcomes
        let hashes: Vec<ContentHash> = outcomes.iter()
            .flat_map(|o| o.written.iter().cloned())
            .collect();
        Query::by_lineage(hashes)
    }
}

/// Gamma → Theta → Delta as a composed pipeline
pub fn hierarchical_tick(
    gamma: &Pipeline,
    theta: &Pipeline,
    gamma_outcomes: &[TickOutcome],
) -> impl Future<Output = Result<TickOutcome>> {
    let query = Pipeline::fold_outcomes(gamma_outcomes);
    theta.tick(&query)
}
```

**Theoretical basis**: If `TickOutcome` forms a monoid (under concatenation of written hashes),
then `fold_outcomes` is a monoid homomorphism. Category theory guarantees that this fold
composes correctly, meaning Theta's processing of Gamma outcomes is well-defined regardless
of how many Gamma ticks produced how many outcomes.

---

## 9. Inconsistencies Found and Resolutions

### 9.1 Documentation Inconsistencies

| Location | Issue | Resolution |
|---|---|---|
| STATUS.md vs. 15-crate-map.md | STATUS says "18+ Rust crates"; crate map shows 28 | Clarify: 18 primary crates + MCP/demo/utility crates |
| 12-five-layer-taxonomy.md | Lists `roko-fs` as L3 Harness | `roko-fs` is L0 Runtime (implements FileSubstrate). Mislabeled. |
| 06-synapse-traits.md | Says "4 Substrate implementations" | Actually 4 are spec'd; 2 are shipped (Memory, File) |
| TUI status | STATUS.md says "Scaffold"; QUICKSTART.md shows `roko dashboard` as working command | Both correct: command exists, outputs text; "Scaffold" means no interactive ratatui UI |
| 02-engram-data-type.md | References "7-axis appraisal" | Code has 4 axes (Score struct). 3 extended axes specified but not implemented. |

### 9.2 Code-Documentation Mismatches

| Aspect | Documentation | Code | Impact |
|---|---|---|---|
| **Data type name** | "Engram" | current kernel struct name | None (see 01-naming-and-glossary.md) |
| **Score axes** | 7 (4 stable + 3 extended) | 4 (confidence, novelty, utility, reputation) | Medium — documentation overpromises |
| **Attestation field** | Specified in Engram docs | Not in current kernel struct | Low — Phase 2+ feature |
| **Conductor layer** | Documented as L3 or L4 | Depends on roko-learn (L2) — actual layer unclear | Medium — layer violation |

### 9.3 `roko-fs` Layer Assignment

The most significant documentation inconsistency: `roko-fs` is listed under L3 Harness in
the layer taxonomy (Section 5, line 116: "roko-fs — JSONL substrate persistence, garbage
collection, file layout") but functionally it is an L0 Runtime crate. It implements
`FileSubstrate`, which is a `Substrate` trait implementation — and Substrate is assigned to L0.

**Resolution**: Move `roko-fs` to L0 Runtime in the documentation. Its sole purpose is
persistent storage of Engrams, which is the canonical L0 responsibility.

---

## 10. Prioritized Improvements

### 10.1 High Priority (Architectural Integrity)

| # | Improvement | Effort | Impact |
|---|---|---|---|
| 1 | Fix roko-conductor → roko-learn dependency violation | Small | Restores layer integrity |
| 2 | Classify 6 unclassified crates in taxonomy | Small | Completes architectural map |
| 3 | Fix roko-fs layer assignment (L3 → L0) in docs | Trivial | Corrects documentation |
| 4 | Align Score documentation (7-axis → 4-axis current, 7-axis planned) | Small | Prevents confusion |

### 10.2 Medium Priority (Architectural Enhancement)

| # | Improvement | Effort | Impact |
|---|---|---|---|
| 5 | Implement gradient Gate feedback (Section 8.2) | Medium | Continuous learning from gate confidence |
| 6 | Define AffectModel trait in roko-core for Daimon injection | Small | Proper cross-cut isolation |
| 7 | Formalize Pipeline as composable unit (Section 8.3) | Medium | Enables hierarchical speed composition |
| 8 | Implement cross-cut arbitration protocol | Medium | Resolves Daimon↔Neuro↔Dreams conflicts |

### 10.3 Low Priority (Architectural Innovation)

| # | Improvement | Effort | Impact |
|---|---|---|---|
| 9 | CompetitiveRouter (LIDA-inspired, Section 8.1) | Large | More robust attention/selection |
| 10 | VSA/HDC operations on Engram struct (Section 5.4) | Large | Compositional knowledge representation |
| 11 | Formal category theory verification of pipeline laws | Large | Mathematical guarantees of composability |

### 10.4 From-scratch rewrite candidates as the v2 path

REF21 is the alternative to indefinite incremental patching. Use it when the design embeds the
wrong assumption, the interface surface is small enough to restabilize, and the new shape unlocks
capabilities that the current architecture cannot reach cleanly.

For this codebase, the decisive assumption is the medium model. The v2 path is the from-scratch
kernel rewrite that separates Engram from Pulse and places both on the right fabrics: Substrate
for storage and Bus for transport. Once that foundation is in place, the remaining candidates
sequence naturally: substrate after kernel, then learning and composition, then gates if the
incremental path still leaves too much surface tension.

Terminology follows [01-naming-and-glossary.md](./01-naming-and-glossary.md).

Read `tmp/refinements/21-from-scratch-redesigns.md` for the full candidate list, the rewrite
heuristic, and the week-by-week sequencing.

---

## 11. Academic References

### Cognitive Architectures
- Sumers, T. et al. (2023). "Cognitive Architectures for Language Agents (CoALA)." arXiv:2309.02427.
- Franklin, S. et al. (2016). "LIDA: A Systems-level Architecture for Cognition, Emotion, and Learning." IEEE Trans. Autonomous Mental Development 6(1).
- Laird, J. E. (2012). "The Soar Cognitive Architecture." MIT Press.
- Anderson, J. R. (2007). "How Can the Human Mind Occur in the Physical Universe?" Oxford University Press.
- Laird, J. E. (2022). "Analysis and Comparison of ACT-R and Soar." arXiv:2201.09305.
- Baars, B. J. (1988). "A Cognitive Theory of Consciousness." Cambridge University Press.

### Active Inference and Free Energy
- Friston, K. (2010). "The free-energy principle: a unified brain theory?" Nature Reviews Neuroscience 11(2).
- Parr, T. et al. (2024). "Active Inference: The Free Energy Principle in Mind, Brain, and Behavior." arXiv:2402.14460.
- VERSES AI (2025). "Genius: Renormalizing Generative Models." [verses.ai](https://www.verses.ai/active-inference-research)
- Champion, T. et al. (2022). "pymdp: A Python library for active inference." arXiv:2201.03904.
- Devillers, B. et al. (2024). "An Embodied Agent Inspired by Global Workspace Theory." Frontiers in Computational Neuroscience.

### Dual-Process and Multi-Speed
- Fabiano, F. et al. (2025). "SOFAI: A multi-component cognitive architecture for intelligent systems." npj Artificial Intelligence.
- Kahneman, D. (2011). "Thinking, Fast and Slow." Farrar, Straus and Giroux.
- Sun, R. (2002). "Duality of the Mind." Lawrence Erlbaum Associates. (CLARION architecture)

### Category Theory and Software Composition
- Milewski, B. (2014). "Category Theory for Programmers." [bartoszmilewski.com](https://bartoszmilewski.com/2014/10/28/category-theory-for-programmers-the-preface/)
- Seemann, M. (2017). "From Design Patterns to Category Theory." [blog.ploeh.dk](https://blog.ploeh.dk/2017/10/04/from-design-patterns-to-category-theory/)
- Clarke, B. et al. (2020). "Profunctor Optics: A Categorical Update." arXiv:2001.07488.
- Gonzalez, G. (2012). "The Functor Design Pattern." [haskellforall.com](https://haskellforall.com/2012/09/the-functor-design-pattern.html)

### Agent Systems and Design Patterns
- Koopmans, A. et al. (2024). "Agent Design Pattern Catalogue." arXiv:2405.10467.
- Google Cloud (2025). "Eight Multi-Agent Design Patterns." InfoQ.
- Phan-Ba, R. et al. (2025). "Agent Data Protocol (ADP)." arXiv:2510.24702.

### Cybernetics and Self-Organization
- Ashby, W. R. (1956). "An Introduction to Cybernetics." Chapman & Hall.
- Conant, R. C. & Ashby, W. R. (1970). "Every Good Regulator of a System Must Be a Model of That System." Int. J. Systems Science 1(2).
- Beer, S. (1972). "Brain of the Firm." Allen Lane.
- Kauffman, S. (1993). "The Origins of Order." Oxford University Press.

### Hyperdimensional Computing
- Kanerva, P. (2009). "Hyperdimensional Computing." Cognitive Computation 1(2).
- Kleyko, D. et al. (2022). "A Survey on Hyperdimensional Computing." Artificial Intelligence Review 56.
- Frady, E. P. et al. (2018). "Neural computation with HDC vectors."

### Global Workspace Theory
- Dehaene, S. et al. (2025). "GW-Dreamer: Multimodal Global Workspace + Dreamer." arXiv:2502.21142.
- VanRullen, R. & Bhatt, A. (2025). "Functional Advantages of the Selection-Broadcast Cycle." arXiv:2505.13969.

### Memory and Sleep
- McClelland, J. et al. (1995). "Complementary Learning Systems." Psychological Review 102(3).
- Mattar, M. & Daw, N. (2018). "Prioritized Memory Access." Nature Neuroscience 21.
- Walker, M. & van der Helm, E. (2009). "Sleep and Emotional Memory." Annual Review of Clinical Psychology 5.
- Lacaux, C. et al. (2021). "Hypnagogia and Creative Insights." Science Advances 7(50).

---

## 12. Conclusion

Roko's current architecture is architecturally sound and theoretically well-grounded as v1, but
it should no longer be read as the final shape. The current operator model handles the present
surface with only minor boundary awkwardness, while REF21 defines the v2 rewrite path around the
two-medium/two-fabric kernel. The five-layer taxonomy is clean with one fixable dependency
violation. The three cognitive speeds are a genuine innovation extending classical dual-process
theory. The Engram model is still universal, validated by comparison to the Agent Data Protocol.

The architecture's deepest strength is its **categorical composability**: the pipeline is a
morphism composition, Score is a monoid, and cross-cuts are endofunctors. Those properties are
exactly why incremental refactor works for some improvements and why a from-scratch kernel
rewrite is justified for the ones that invert the medium model.

The most impactful improvement would be **gradient gate feedback** (Section 8.2), which
connects Roko's existing Gate pipeline to active inference's prediction-error minimization
framework, enabling continuous learning from every verification attempt rather than binary
pass/fail outputs.

Read together, the §2.2 telemetry boundary and the §3.2 conductor violation are the strongest
signals in this analysis that the kernel wants the two-medium / two-fabric vocabulary from
`tmp/refinements/01-critique-one-noun.md`, not just a renamed durable record.
