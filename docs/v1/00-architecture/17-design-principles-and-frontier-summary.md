# Design Principles and Frontier Innovation Summary

> **Abstract:** Roko is guided by seven design principles (P1-P7) that constrain architectural
> decisions and prevent feature drift. REF19 reframes the capability list as a net-new
> innovations catalog with an explicit split between primitive innovations and composed
> innovations. This document preserves the design principles, then explains which capabilities
> are genuinely primitive, which are integrations of prior art, and why the moat comes from
> the composition across Engram, Pulse, Bus, Substrate, HDC fingerprint, demurrage,
> heuristics/falsifiers, c-factor, the replication ledger, and the plugin SPI. See
> [tmp/refinements/19-net-new-innovations.md](../../tmp/refinements/19-net-new-innovations.md)
> for the refinement note that drove this rewrite.


> **Implementation**: Mixed

> **Reality check**: This document mixes shipping architecture with target-state
> frontier claims. Shipping today: Engram/Substrate, the six kernel traits, the
> runtime event bus in its current concrete form, HDC primitives, multi-backend
> orchestration, the gate pipeline, episode logging, and the TUI. Research
> hypotheses or planned primitives: `Pulse`, demurrage, heuristics with explicit
> falsifiers as a shared commons, replication ledger, worldview clusters, and
> the full plugin SPI.
>
> **Actual edge today**: the competitive edge is the working Rust product
> surface, not the full future frontier catalog.

**Topic**: [00-architecture](./INDEX.md)
**Prerequisites**: [00-vision-and-thesis](./00-vision-and-thesis.md), [06-synapse-traits](./06-synapse-traits.md), [12-five-layer-taxonomy](./12-five-layer-taxonomy.md)
**Key sources**:
- `/Users/will/dev/nunchi/roko/refactoring-prd/09-innovations.md` — Historical source for the older frontier list
- `/Users/will/dev/nunchi/roko/refactoring-prd/00-overview.md` — Design principles, overview
- `/Users/will/dev/nunchi/roko/bardo-backup/prd/00-vision/05-manifesto.md` — Original manifesto (7 principles)
- `[tmp/refinements/19-net-new-innovations.md](../../tmp/refinements/19-net-new-innovations.md)` — REF19 source note for the net-new catalog

---

## Abstract

Design principles are the immune system of an architecture — they prevent bad decisions
before they are made. Roko's seven principles (P1 through P7) were extracted from the
patterns that made the original pre-refactor codebase succeed and the patterns that made it
fail. Each principle is a hard constraint: if a proposed change violates a principle, it is
rejected regardless of its other merits.

The net-new catalog is intentionally split into primitive and composed innovations so the
document does not blur architectural primitives with higher-level capability surfaces. The
primitive layer names the kernel invariants; the composed layer shows how those invariants,
plus prior art, produce the surfaces that matter operationally.

This document serves as both a design guide (what principles govern decisions) and a
capability catalog (what is genuinely net-new, what is primitive, and what is composed).

### Status framing

| Category | What belongs here today |
|---|---|
| **Shipping** | Engram/Substrate, the six kernel traits, the current concrete event bus, HDC primitives, gate pipeline, multi-backend orchestration, episode logging, TUI |
| **Research hypotheses / target-state** | Pulse, demurrage, falsifier commons, replication ledger, worldview clusters, full plugin SPI |
| **Prior art integrations** | Many higher-level compositions such as T0 probes, VCG attention, predictive foraging, and cross-domain resonance |

---

## 1. The Seven Design Principles

### P1: Composition Over Configuration

**Statement**: Every capability is a trait implementation. New behaviors emerge from
composing existing traits, not from configuration flags or feature gates.

**Rationale**: Configuration systems grow unboundedly — a dozen flags become a hundred,
interactions between flags become untestable, and users cannot reason about behavior.
Trait composition is bounded by the type system: if it compiles, the composition is valid.

**Application examples**:
- A new verification strategy is a new `Gate` implementation, not a `verification_strategy`
  config key.
- A new context assembly strategy is a new `Composer` implementation, not a
  `context_mode` flag.
- Domain-specific behavior (chain tools, coding tools) is a trait implementation plugged
  into the universal loop, not a `domain` configuration enum.

**Anti-pattern**: `if config.mode == "chain" { ... } else if config.mode == "coding" { ... }`.
This is a sign that domain behavior should be factored into a trait implementation.

### P2: Verify Everything

**Statement**: Every agent output passes through the Gate pipeline before being persisted
or acted upon. No unverified Engrams enter the audit DAG as trusted.

**Rationale**: LLMs hallucinate. Tools fail silently. External data sources lie. Without
systematic verification, errors compound through the lineage DAG. The Gate pipeline is the
firewall between "the agent produced something" and "the system trusts it."

**Application examples**:
- Code output passes through compile gate → test gate → clippy gate → diff gate before
  being accepted.
- Knowledge claims pass through confidence threshold → source verification → consistency
  check before being promoted from Transient to Working tier.
- Chain transactions pass through simulation gate (mirage-rs) → gas estimation → balance
  check before submission.

**Escape hatch**: Engrams can be marked with low confidence and stored as Transient tier
knowledge without full verification, allowing speculative hypotheses to exist in the system
while clearly flagged as unverified.

### P3: Budget-Aware by Default

**Statement**: Every operation has a budget (tokens, time, cost, signals). The system
degrades gracefully when budgets are exhausted, never crashes or produces unbounded output.

**Rationale**: LLM inference is expensive. Context windows are finite. Execution time has
deadlines. A system that does not respect budgets either exceeds cost limits or truncates
output arbitrarily. Roko embeds budget awareness into the core types (Query has a Budget;
Composer takes a Budget parameter).

**Application examples**:
- `Composer.compose()` takes a `Budget { max_tokens, max_signals, max_bytes, max_wall_ms }`
  and assembles context within those constraints.
- `CascadeRouter` selects the cheapest model tier sufficient for the current prediction
  error, not the most capable.
- The orchestrator tracks total cost across a plan execution and can halt when budget is
  exhausted.

### P4: Content-Addressed Everything

**Statement**: Every Engram has a BLAKE3 content hash computed from its identity fields
(kind, body, author, tainted, lineage, tags). The hash is the Engram's identity. Two
Engrams with identical content have identical hashes.

**Rationale**: Content addressing provides three properties simultaneously:
1. **Deduplication**: Identical content is automatically deduplicated.
2. **Integrity verification**: Any modification to an Engram changes its hash, making
   tampering detectable.
3. **Lineage verification**: The audit DAG is a hash chain — replaying the lineage
   verifies that no intermediate Engram was modified.

This is the foundation of the Forensic AI capability — causal replay of agent decisions
with cryptographic verification.

### P5: Decay as Feature, Not Bug

**Statement**: Engrams decay by default. Information that is not reinforced fades. This is
a feature that prevents knowledge hoarding and ensures the system's working memory stays
relevant.

**Rationale**: Without decay, the Substrate grows unboundedly. Old, irrelevant Engrams
dilute search results. The system cannot distinguish between current knowledge and historical
artifacts. Biological memory systems (Ebbinghaus 1885) demonstrate that forgetting is
essential for efficient recall.

**Application examples**:
- Threat pheromones (HalfLife: 2 hours) fade quickly because threats are time-sensitive.
- Working knowledge (Ebbinghaus with strength 0.5) persists for hours to days, long enough
  to be useful within a session.
- Persistent knowledge (strength 5.0×) has effective half-lives of weeks to months, but
  still eventually decays if not reinforced through use.
- `Decay::None` exists for permanent records (e.g., on-chain attestations) but is the
  exception, not the default.

### P6: Self-Model Is Mandatory

**Statement**: Every agent maintains a self-model (the Daimon PAD vector) that tracks its
own cognitive state. This self-model modulates behavior — it is not cosmetic.

**Rationale**: The Good Regulator Theorem (Conant & Ashby 1970) proves that every good
regulator must contain a model of the system it regulates. An agent without self-awareness
cannot adaptively allocate cognitive resources, cannot detect when it is struggling, and
cannot decide when to escalate to a stronger model.

**Application examples**:
- An agent with declining Pleasure (task failures) and rising Arousal (increasing urgency)
  transitions from Engaged to Struggling state, shortening the Theta reflection cycle.
- An agent with stable Pleasure and low Arousal transitions to Coasting state, extending
  Gamma runs and preferring T0/T1 tiers.
- Somatic markers (positive/negative valence from past experiences) bias Router selection
  before analytical reasoning engages.

### P7: Observable by Default

**Statement**: Every Engram, every trait invocation, every gate verdict, every tier
selection is observable. The system is transparent to its operators at every level of
abstraction.

**Rationale**: Opaque agent systems are undeployable in regulated environments and
undebuggable in development. Roko's content-addressed lineage DAG, provenance tracking,
and taint propagation provide full observability without requiring additional instrumentation.

**Application examples**:
- `roko replay <hash>` walks the lineage DAG backward from any Engram to its root inputs.
- Gate verdicts include the gate name, score, reason, test counts, and error digest.
- CascadeRouter logs every tier selection decision with confidence and cost.
- Daimon PAD vector changes are recorded as Engrams, creating a visible emotional history.

---

## 2. The Fourteen Frontier Innovations

REF19 replaces the older frontier list with a tighter distinction: some items are target-state
primitive innovations, meaning proposed architectural primitives or invariants; others are
integrated innovations, meaning composed capabilities built from those primitives plus prior
art. That split matters because it keeps the novelty claim honest. See
[tmp/refinements/19-net-new-innovations.md](../../tmp/refinements/19-net-new-innovations.md)
for the source note.

### 2.1 Summary Table

| # | Primitive innovation | What is net-new here | Closest prior art | Why it matters |
|---|---|---|---|---|
| 1 | **Engram / Pulse / Bus / Substrate split** | A clean separation between durable record, ephemeral transport, transport fabric, and storage fabric. | Event sourcing, actor systems, message buses, hexagonal architecture | This gives the kernel a stable vocabulary and boundary model for both durability and live traffic. |
| 2 | **HDC fingerprint** | A deterministic 10,240-bit fingerprint on each Engram for similarity, clustering, consensus, and analogy. | Hyperdimensional computing, sparse distributed memory, embedding search | Semantic locality becomes a first-class runtime primitive instead of a side index. |
| 3 | **Demurrage** | A continuous holding cost on idle Engrams with reinforcement for useful reuse and retrieval. | Gesell demurrage, forgetting curves, memory decay models | The durable medium stays selective instead of turning into dead weight. |
| 4 | **Heuristics / falsifiers commons** | Heuristics live with explicit falsifiers, recalibration, and exportable calibration history. | Scientific method, calibration logs, prediction markets | Belief revision becomes auditable and shared across deployments instead of local and informal. |
| 5 | **c-factor** | A cohort-process scalar learned from Bus and Substrate evidence, not declared by fiat. | Collective-intelligence research, team-process metrics | Group quality becomes measurable and actionable without collapsing into a single hard objective. |
| 6 | **Replication ledger** | A durable record of replications, failures, and outcome conditions for claims and heuristics. | Provenance graphs, experimental registries, audit logs | Later agents inherit tested knowledge instead of starting from scratch. |
| 7 | **Plugin SPI** | A stable plugin service provider interface for extensions, domain surfaces, and local workflows. | Rust trait-based plugins, ports/adapters, extension-point design | The architecture stays open without becoming configuration soup. |

### 2.2 Innovation Details

| # | Integrated innovation | Built from | Closest prior art | Why it matters |
|---|---|---|---|---|
| 1 | **16 T0 Probes** | Probe registry + router + budgets + prediction-error aggregation | Cascade routing, FrugalGPT-style model selection | Zero-LMM-cost ticks become the default, not the exception. |
| 2 | **VCG Attention Auction** | Budgeting + competing subsystems + truthful allocation | Vickrey-Clarke-Groves auctions, attention economics | Context allocation becomes explicit and testable instead of ad hoc. |
| 3 | **Somatic Landscape** | PAD/self-model + HDC neighborhood search + strategy memory | Somatic marker hypothesis, nearest-neighbor decision support | Affect becomes a routing signal rather than a cosmetic state label. |
| 4 | **Hypnagogia Engine** | HDC recall + episodic recombination + guarded partial completions | Dormio-style hypnagogia research, creative recombination systems | It forces divergent hypothesis generation when single-model convergence would flatten alpha. |
| 5 | **Predictive Foraging** | Heuristics + falsifiers + retrieval prediction + stopping rules | Marginal Value Theorem, information foraging theory | Retrieval becomes self-correcting and budget-aware. |
| 6 | **Collective calibration loop** | c-factor + replication ledger + verified outcomes + shared heuristics | Calibration tracking, team learning loops | The system learns from each cohort instead of resetting at deployment boundaries. |
| 7 | **Forensic AI** | Engram lineage + Bus history + Gate verdicts + replay tooling | Audit trails, causal reconstruction, provenance systems | Decisions can be replayed with the evidence that actually existed at the time. |
| 8 | **EvoSkills / ADAS family** | Plugin SPI + gates + replication ledger + trait search | Adversarial skill verification, meta-agent architecture search | Skills and architectures can improve through selection pressure instead of manual curation alone. |
| 9 | **Cross-Domain Insight Resonance** | HDC fingerprint + heuristic commons + diverse evidence | Analogy engines, HDC similarity search | Structural similarity across domains becomes a low-latency capability. |
| 10 | **Generative Interfaces (A2UI)** | Plugin SPI + design system + runtime descriptions | Schema-driven UI generation, generated admin surfaces | Agents can expose usable interfaces without hand-built screens for every workflow. |
| 11 | **Knowledge Futures Market** | Replication ledger + plugin SPI + chain settlement + verified delivery | Prediction markets, escrowed work contracts | Research demand can be directed toward verifiable output instead of vague planning. |

The honest reading is that the integrated items are mostly compositions. Their novelty is not
that each subpiece is unprecedented in isolation; their novelty is that the same architecture
can wire them together so the outputs of one become the inputs of the next.

---

## 3. Innovation Interconnection Map

The moat is not any one primitive innovation by itself. The moat is the composition across
Engram, Pulse, Bus, Substrate, HDC fingerprint, demurrage, heuristics/falsifiers, c-factor,
the replication ledger, and the plugin SPI. Those pieces reinforce one another:

REF31 turns that composition claim into an explicit architecture artifact: see
[34-synergy-integration-map](./34-synergy-integration-map.md) and
[tmp/refinements/31-synergy-integration-map.md](../../tmp/refinements/31-synergy-integration-map.md)
for the 10-primitive matrix and the named mechanisms behind this moat framing.

| Primitive / invariant | Composed capability it enables | Moat effect |
|---|---|---|
| Engram / Pulse / Bus / Substrate | Forensic AI, live coordination, durable replay | Separating durable record from ephemeral transport makes the system auditable without freezing runtime traffic. |
| HDC fingerprint | Somatic Landscape, Hypnagogia, Cross-Domain Insight Resonance | Similarity becomes a shared computation surface instead of a bespoke index in each feature. |
| Demurrage | Predictive Foraging, memory hygiene, selective persistence | The durable medium stays live, which prevents the commons from turning into dead weight. |
| Heuristics / falsifiers | Collective calibration, VCG routing, EvoSkills | Learned rules stay falsifiable, exportable, and revisable instead of becoming folklore. |
| c-factor | Shared calibration, cohort-aware routing, team diagnostics | Group process becomes measurable and transportable across deployments. |
| Replication ledger | Reuse of verified claims, failure replay, evidence inheritance | Every deployment can start from accumulated evidence rather than from scratch. |
| Plugin SPI | Domain extensions, ADAS search space, user-specific workflows | The platform remains open-ended without giving up architectural control. |

The structural lesson from REF19 is that competitors can copy a feature surface, but they do
not automatically inherit the linked primitives that make the surface self-reinforcing. The
two-mediums / two-fabrics kernel matters here: `Engram` and `Pulse` separate durable record
from ephemeral transport; `Substrate` and `Bus` separate storage from transport; the seven-step
loop then reuses those primitives on every turn so learning, verification, persistence, and
broadcast compound instead of resetting.

P1 keeps the system compositional, P2 and P7 make it auditable, P3 keeps accumulation bounded,
P4 and P5 keep memory durable but selective, and P6 keeps the policy layer able to model
itself well enough to exploit the evidence the loop returns. That is why the moat is a curve
of reinforcement, not a single feature claim.

### 3.1 Structural Moat Synthesis

The moat thesis in
[tmp/refinements/18-competitive-moat.md](../../tmp/refinements/18-competitive-moat.md)
still applies, but REF19 sharpens it: architectural coherence, a heuristic commons, a plugin
ecosystem, a replication ledger, and kernel-level correctness are not separate advantages.
They are the same advantage viewed at different scales. Each one makes the next one easier to
build, verify, and retain.

---

## 4. Empirical Validation Status

Each innovation has different validation requirements:

| Innovation | Validation status | What would validate it |
|---|---|---|
| Engram / Pulse / Bus / Substrate split | Core architecture | Round-trip fidelity across live transport, durable storage, replay, and graduation boundaries. |
| HDC fingerprint | Implemented in primitives | Similarity search quality, collision behavior, and clustering usefulness on real Engrams. |
| Demurrage | Specified and partially wired | Retention curves that keep useful knowledge live while trimming dead weight. |
| Heuristics / falsifiers commons | Specified | Falsifier closure rates, calibration improvement, and reuse across deployments. |
| c-factor | Specified | Correlation with cohort outcome quality without becoming a gaming target. |
| Replication ledger | Architecture supports it | A complete record of replications, failures, and provenance-linked outcomes. |
| Plugin SPI | Specified | Stable third-party extension loading with clear capability boundaries. |
| 16 T0 Probes | Theoretical/spec'd | Measure T0 suppression rate on real workloads. |
| VCG Attention Auction | Algorithm specified | A/B test: VCG vs. fixed-priority context selection. |
| Somatic Landscape | Cited foundations | A/B test: agents with vs. without somatic markers. |
| Hypnagogia Engine | Cited foundations | Generate hypotheses and have domain experts score genuine vs. spurious novelty. |
| Predictive Foraging | Specified | Measure prediction accuracy and retrieval efficiency over time. |
| Collective calibration loop | Heuristic model | Compare calibration speed and transfer across cohorts and deployments. |
| Forensic AI | Architecture supports it | Reconstruct a real incident from the ledger and verify completeness. |
| EvoSkills / ADAS | Published results | Replicate the improvement loop inside the Gate pipeline. |
| Cross-Domain Insight Resonance | Theoretical | Generate cross-domain insights and measure false positive rate. |
| Generative Interfaces | Specified | Build a prototype A2UI renderer with the design system. |
| Knowledge Futures Market | Deferred (P3) | Depends on Korai chain deployment. |

---

## 5. Theoretical Foundations of the Design Principles

### 5.1 Information Hiding (Parnas 1972)

Parnas's paper "On the Criteria to Be Used in Decomposing Systems into Modules" (CACM 15(12))
established the foundational criterion: **"Begin with a list of difficult design decisions or
design decisions which are likely to change."** Each decision becomes the secret of exactly
one module. The interface reveals as little as possible.

Each Synapse trait passes the Parnas test:
- `Scorer` hides the scoring algorithm (neural model, heuristic, or lookup table)
- `Gate` hides the verification mechanism (subprocess, API call, simulation)
- `Router` hides the selection algorithm (static, bandit, cascade)
- `Composer` hides the assembly strategy (template, VCG auction, priority queue)

The danger is "interface leakage" — when a shared `Datum` shape exposes internal
representation details through the trait interface, coupling all traits through one
overloaded representation.

### 5.2 Module Depth (Ousterhout 2018)

Ousterhout defines module depth = benefit / interface cost. Deep modules have large
implementations behind small interfaces. Each Synapse trait has a 1-3 method interface hiding
arbitrarily complex implementations — this is maximally deep design.

**Information leakage** occurs when a design decision appears in multiple trait interfaces.
If two traits both need to know the prompt template format, the template format has leaked.
Fix: introduce a new module whose secret is the template format.

### 5.3 Clean Architecture / Hexagonal Architecture

Martin's Clean Architecture (2017) and Cockburn's Hexagonal Architecture (2005) converge on
the same principle: **dependencies point inward**. Domain types (Engram, Score, Kind) are in
the center. Infrastructure (Substrate backends, LLM APIs) is on the outside. The Substrate
does not define what an Engram is; the Engram defines what the Substrate must store.

Hexagonal Architecture adds the **port/adapter** distinction that Roko implements exactly:
- **Ports** = Synapse traits (Substrate, Scorer, Gate, Router, Composer, Policy)
- **Adapters** = Concrete implementations (FileSubstrate, CompileGate, CascadeRouter)

Multiple adapters per port is the norm: MemorySubstrate for testing, FileSubstrate for
production, ChainSubstrate for on-chain state.

### 5.4 Algebraic Effects as Theoretical Ideal

Algebraic effects (Plotkin & Power 2001; Bauer & Pretnar 2015; Leijen 2017) represent the
theoretical ideal that Rust's trait system approximates. In an effect system, each capability
is an **effect declaration** (not a trait). Handlers provide interpretations (not impl blocks).
Effect rows replace generic bounds.

The 6-trait system is an approximation of a 6-effect system constrained by Rust's lack of
native effect polymorphism. The key insight: handlers can be stacked (a caching handler wraps
a neural-model handler), enabling compositional interpretation without trait inheritance.

Rust's `async`/`await` is already a special-cased algebraic effect. The Koka language (Leijen
2014) implements general algebraic effects with row-typed effect polymorphism. If Rust gains
higher-kinded types or effect polymorphism, the Synapse traits could be reframed as effects.

### 5.5 Functional Core / Imperative Shell (Bernhardt 2012)

The Synapse Architecture naturally implements this pattern:
- **Functional core** (pure, immutable): Scorer, Router, Composer, Policy — all operate on
  immutable `Datum` inputs and return new values. No I/O, no state mutation.
- **Imperative shell** (effectful): Substrate (persistence), Agent dispatch (LLM calls) —
  the only impure operations.

This creates a clean testing boundary: functional core traits can be tested with pure
inputs/outputs and no mocking. Imperative shell traits require integration testing.

---

## 6. Anti-Patterns Catalog: Designs Explicitly Rejected

Based on the MAST taxonomy of multi-agent system failures (Cemri et al. 2025, arXiv:2503.13657)
and empirical agent failure analysis.

### 6.1 The God Agent Anti-Pattern

**Rejected**: A single agent instructed to perform all roles simultaneously (coder, reviewer,
tester, architect). MAST finding: 41.8% of multi-agent failures stem from specification and
system design issues; role ambiguity is the leading cause.

**Roko's design**: Agent roles are explicit (`RoleSystemPromptSpec` in `roko-compose`). Each
role has defined responsibilities AND exclusions. Nine role templates provide clear behavioral
boundaries.

### 6.2 Hallucination Amplification Loop

**Rejected**: Pipeline where Agent B takes Agent A's unverified output as trusted input. Error
amplifies at each step through conformity bias and chain-of-thought amplification (OWASP ASI08:
"Cascading Failures in Agentic AI").

**Roko's design**: P2 (Verify Everything) structurally prevents this. Every output passes
through the Gate pipeline before being persisted or used as input. The lineage DAG traces
every Engram to its verified sources.

### 6.3 Unbounded Context Sharing

**Rejected**: Agents sharing a single growing context window that amplifies both errors and
distraction. MAST FM-1.4: loss of conversation history occurs when context exceeds the window.

**Roko's design**: P3 (Budget-Aware) limits context via `Budget { max_tokens, max_signals,
max_bytes, max_wall_ms }`. The VCG Attention Auction allocates context optimally rather than
growing it unboundedly.

### 6.4 Implicit Termination

**Rejected**: Termination condition is "when the task is done" without an operationalized check.
MAST FM-1.5: 8.2% of failures are premature termination or failure to terminate.

**Roko's design**: Every task has a machine-readable completion criterion checked by the Gate
pipeline. Budget thresholds (`warn_threshold`, `block_threshold`) provide hard stops.
`max_turns` bounds every agent invocation.

### 6.5 Self-Verification

**Rejected**: An agent verifying its own output. MAST FM-3.3: 9.1% of failures involve
incorrect verification — agents rationalizing their own output as correct.

**Roko's design**: Gates are structurally separate from the producing agent. `CompileGate`
runs an external subprocess. `TestGate` runs the test suite. `JudgeGate` uses a separate
LLM invocation. No agent evaluates its own work.

### 6.6 Configuration Over Composition

**Rejected**: `if config.mode == "chain" { ... } else if config.mode == "coding" { ... }`.
Domain behavior controlled by config flags rather than trait implementations.

**Roko's design**: P1 (Composition Over Configuration). Domain behavior is a trait
implementation plugged into the universal loop. New domains add trait implementations;
no core changes needed.

### 6.7 Verification as Afterthought

**Rejected**: Adding verification at the end of a pipeline as an optional step that can be
skipped for speed.

**Roko's design**: P2 places verification in the seven-step loop as a load-bearing phase,
not an appendable afterthought. Gate verdicts are Engrams in the lineage DAG; skipping them
leaves an audit gap that is visible to any DAG traversal.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Parnas 1972, CACM 15(12) | Information hiding: modules hide design decisions likely to change. |
| Ousterhout 2018, Yaknyam Press | Module depth: simple interfaces, powerful implementations. |
| Martin 2017, Prentice Hall | Clean Architecture: dependencies point inward. |
| Cockburn 2005, Technical Report | Hexagonal Architecture: ports (traits) and adapters (impls). |
| Plotkin & Power 2001, FoSSaCS, LNCS 2030 | Algebraic effects: adequacy for effect algebras. |
| Bauer & Pretnar 2015, JLAMP 84(1) | Programming with algebraic effects and handlers. |
| Leijen 2017, POPL | Row-typed algebraic effects: efficient compilation of effect systems. |
| Bernhardt 2012, SCNA | Functional Core / Imperative Shell pattern. |
| Cemri et al. 2025, arXiv:2503.13657 | MAST: 14 failure modes across 150 multi-agent system traces. |
| OWASP ASI08 2026 | Cascading Failures in Agentic AI: error amplification taxonomy. |
| Evans 2003, Addison-Wesley | Domain-Driven Design: ubiquitous language, bounded contexts. |
| Hewitt et al. 1973, IJCAI | Actor Model: concurrent computation via message passing. |
| Chen et al. 2023, arXiv:2305.05176 | FrugalGPT: cascade cost optimization, up to 98% reduction. |
| Vickrey 1961; Clarke 1971; Groves 1973 | VCG mechanism: truthful bidding for resource allocation. |
| Damasio 1994, Descartes' Error | Somatic marker hypothesis: emotion biases decisions. |
| Lacaux et al. 2021, Science Advances 7(50) | Hypnagogia: 83% hidden rule discovery in N1. |
| Hu et al. 2025, ICLR | ADAS: meta-agent architecture search, +14% ARC Challenge. |
| Kanerva 2009, Cognitive Computation 1(2) | HDC: hyperdimensional computing for similarity search. |
| Kleyko et al. 2022, Artificial Intelligence Review | Survey of HDC applications. |
| Charnov 1976 | Marginal Value Theorem: optimal foraging stopping rule. |
| Conant & Ashby 1970, IJSS 1(2) | Good Regulator Theorem. |
| Lee et al. 2026, arXiv:2603.28052 | Meta-Harness: harness optimization alone produces outsized gains. |

---

## Current Status and Gaps

- **Design principles (P1-P7)**: Documented and enforced in architectural review. Not
  formally encoded as automated checks; a linter for principle violations would be useful.
- **Engram / Pulse / Bus / Substrate split**: `Engram` and `Substrate` ship. The current event
  bus exists in runtime code, but `Pulse` and a generic kernel `Bus` trait are target-state.
- **HDC fingerprint**: HDC primitives ship, but there is not yet an HDC fingerprint field on
  every `Engram`.
- **Demurrage**: Target-state only. Standard decay exists; the demurrage model does not.
- **Heuristics / falsifiers commons**: Target-state only. Shared falsifier-aware calibration is
  not yet a live runtime surface.
- **c-factor**: Partial. It exists as a routing/coordination signal, but not yet at the full
  scope described here.
- **Replication ledger**: Not yet built as a first-class runtime subsystem.
- **Plugin SPI**: Target-state. The current `roko-plugin` crate is a narrow event-source and
  feedback SDK, not the full multi-tier extension platform described above.
- **16 T0 Probes**: Specified but not implemented. The probe registry (`Vec<Box<dyn Probe>>`)
  is straightforward; individual probes need domain-specific implementations.
- **VCG Attention Auction**: Specified with bid computation formulas. Not yet wired into the
  shipping Composer; current context selection uses score-based ranking.
- **Somatic Landscape**: `roko-daimon` has PAD vector and behavioral states. The k-d tree
  somatic landscape is specified but not built.
- **Hypnagogia Engine**: Specified in `roko-dreams` as scaffold. Not shipping.
- **Predictive Foraging**: CalibrationTracker specified in `roko-learn`. Prediction
  registration is wired; the stopping rule is not yet implemented.
- **Collective calibration loop**: Heuristic model only. Requires broader verified-outcome
  infrastructure before it can become a stable system behavior.
- **Forensic AI**: Content-addressed lineage DAG exists. Causal replay tooling (`roko replay`)
  exists for simple cases. Full forensic reconstruction is not yet built.
- **EvoSkills / ADAS**: Skill-library evolution and meta-search are still not fully wired.
  Both depend on stable trait compositions and stronger verifier isolation.
- **Cross-Domain Insight Resonance**: HDC vectors are built in `roko-primitives`. Cross-domain
  detection is not yet wired.
- **Generative Interfaces**: Specified. Depends on the ROSEDUST design system, which is not
  yet fully built.
- **Knowledge Futures Market**: Deferred as a P3 feature. Depends on Korai chain deployment.

---

## Cross-References

- See [00-vision-and-thesis](./00-vision-and-thesis.md) for the core thesis underlying these principles
- See [04-decay-variants](./04-decay-variants.md) for P5 (Decay as Feature) details
- See [05-provenance-and-attestation](./05-provenance-and-attestation.md) for P4 (Content-Addressed) and P7 (Observable) details
- See [11-dual-process-and-active-inference](./11-dual-process-and-active-inference.md) for T0/T1/T2 tier routing and probe logic
- See [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md) for Daimon, Dreams, and Neuro
- See [14-c-factor-collective-intelligence](./14-c-factor-collective-intelligence.md) for c-factor, collective calibration, and heuristic learning
- See [16-autocatalytic-and-cybernetics](./16-autocatalytic-and-cybernetics.md) for the autocatalytic improvement theory and feedback loops
- See topic [08-chain](../08-chain/INDEX.md) for chain settlement, ledgered verification, and knowledge-market primitives
- See topic [10-dreams](../10-dreams/INDEX.md) for Hypnagogia and Dreams engine details
- See [tmp/refinements/19-net-new-innovations.md](../../tmp/refinements/19-net-new-innovations.md) for the REF19 primitive/composed split
