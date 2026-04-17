# Naming Map and Glossary

> **Abstract:** This document provides the authoritative naming map for all Roko
> terminology, mapping legacy names to their current equivalents and defining every
> Roko-specific term, abbreviation, and architectural concept used across the PRD suite.
> When any other document disagrees with a definition here, this glossary is authoritative.
> This serves as a quick-reference companion to the full architecture documentation. See also
> `tmp/refinements/03-bus-as-first-class.md` for the Bus promotion that makes the transport
> fabric a first-class kernel surface beside Substrate,
> `tmp/refinements/04-operators-generalized.md` for the `Datum` and `PolicyOutputs` operator
> `tmp/refinements/05-loop-retold.md` for the seven-step loop vocabulary (`SENSE`,
> `BROADCAST`, and cross-cuts as injections rather than loop steps),
> generalization, and
> `tmp/refinements/09-phase-2-implications.md` for how Chain, Dreams, Coordination, and
> Heartbeat land on the same two-fabric kernel.


> **Implementation**: Shipping

---

## 1. Project and Framework Names

The Roko project has undergone several naming iterations. The following table is the
canonical old-to-new mapping. All documentation uses the new names exclusively (with
parenthetical notes when quoting legacy sources).

| Old Name | New Name | Notes |
|---|---|---|
| Bardo | **Roko** | Overall framework/project name. "Bardo" was the umbrella name for the original ecosystem. |
| Mori | **Roko Orchestrator** | Build/coding orchestration. Often just "orchestrator." The original Mori was a 108K-LOC TypeScript/Rust application for coding agent orchestration. |
| Golem / Golems | **Agent / Agents** | The autonomous entity. "Agent" is the generic term; "Roko agent" when disambiguation is needed. The framework is Roko; individual entities are agents. |
| Grimoire | **Neuro** / `roko-neuro` / **NeuroStore** | Knowledge management subsystem. Persists insights, heuristics, warnings with tier-based decay. |
| Styx | **Agent Mesh** / **Mesh** | P2P relay and permissioned subnets for inter-agent communication and knowledge sharing. |
| GNOS (token) | **KORAI** (mainnet) / **DAEJI** (testnet) | Token names. KORAI is the mainnet token on the Korai chain. DAEJI is the testnet equivalent on the Daeji testnet. |
| Korai | **Korai** | Dedicated EVM chain for agent coordination. Mainnet. |
| Daeji | **Daeji** | Testnet for the Korai chain. |
| Clade (legacy term) | **Fleet** / **Mesh** | Legacy name for cooperating agents. Use **Fleet** for a roster and **Mesh** for the network topology. |
| Signal (legacy code identifier) | **Engram** | Legacy code name still appears in some Rust paths; documentation uses **Engram** for the durable medium and does not keep the old equivalence disclaimer. |
| EventBus<E> (legacy transport name) | **Bus** / `BroadcastBus` | Legacy generic naming. User-facing docs use **Bus** for the kernel transport primitive and `BroadcastBus` for the in-process backend. |
| Envelope<E> (legacy transport wrapper) | **Pulse** | Legacy wrapper name retained only as an implementation detail during migration. |
| Event / Message (legacy transport nouns) | **Pulse** | Use **Pulse** for the ephemeral wire medium. Use `ChatMessage` only for LLM transcript payloads. |
| Channel / Subject (legacy routing nouns) | **Topic** | Use **Topic** for the dot-separated routing handle on the Bus. |

> **Legacy code note:** some Rust identifiers still use `Signal`, but that is a migration
> detail rather than an architectural synonym. The durable medium is **Engram**; the
> ephemeral medium is **Pulse**; the two fabrics are **Substrate** and **Bus**. See
> [06-synapse-traits.md](./06-synapse-traits.md),
> [07-substrate-trait.md](./07-substrate-trait.md), and
> `tmp/refinements/03-bus-as-first-class.md`. REF09 extends that same vocabulary to
> `ChainBus`, `MeshBus`, and `HeartbeatPolicy`; see `tmp/refinements/09-phase-2-implications.md`.

---

## 2. Configuration Files

| Old | New |
|---|---|
| `golem.toml` | `roko.toml` |

---

## 3. Crate Names

| Old Crate | New Crate | Notes |
|---|---|---|
| All `golem-*` | `roko-*` | Mechanical rename. |
| All `bardo-*` | `roko-*` | Mechanical rename. Note: `bardo-runtime` and `bardo-primitives` still use old names in the current codebase. Rename is planned. |
| `mori-index` | `roko-index` | Code parsing, symbol graphs, HDC fingerprints. |
| `mori-context` | `roko-compose` + `roko-index` | Context features moved to `roko-compose`; code intelligence moved to `roko-index`. |
| `mori-mcp` | `roko-mcp-{stdio,github,slack,scripts}` | Split into per-transport MCP server crates. |
| `bardo-terminal` | `roko-cli` | The terminal UI is in `roko-cli`, with a separate TUI scaffold. |
| `roko-golem` | **DISSOLVED** | See Crate Dissolution section below. |

---

## 4. Crate Dissolution: `roko-golem`

The `roko-golem` crate has been dissolved. Its subsystems are redistributed to standalone
crates following the composability principle: any subsystem can pipe to any other through
Engrams and the six Synapse traits. No umbrella crate is needed.

| Subsystem | Original Location | New Location | Notes |
|---|---|---|---|
| Daimon (972 lines, fully implemented) | `roko-golem/daimon.rs` | `roko-daimon` | Full PAD vector implementation, behavioral states, somatic markers. |
| Dreams (43 lines, placeholder) | `roko-golem/dreams.rs` | `roko-dreams` | Placeholder deleted; `roko-dreams` is the expanded implementation. |
| Grimoire (44 lines, placeholder) | `roko-golem/grimoire.rs` | `roko-neuro` | Placeholder deleted; `roko-neuro` is the replacement with tier-based knowledge management. |
| Chain Witness (43 lines, placeholder) | `roko-golem/chain_witness.rs` | `roko-chain` as `chain_witness` module | Moved. |
| Mortality (44 lines, placeholder) | `roko-golem/mortality.rs` | **DELETED ENTIRELY** | No mortality system in the new architecture. Resource constraints (budget, confidence, time) replace mortality clocks. |
| Hypnagogia (42 lines, placeholder) | `roko-golem/hypnagogia.rs` | `roko-dreams` as `hypnagogia` module | Moved to the Dreams crate. |
| `ScaffoldEngine` trait | `roko-golem/lib.rs` | **DELETED** | Each subsystem defines its own trait. No umbrella needed. |
| `GolemScaffold` aggregator | `roko-golem/lib.rs` | **DELETED** | Composition happens at the application layer via configuration. |

After dissolution, `roko-golem` is removed from workspace members in `Cargo.toml`.

**Composability principle**: Any subsystem can pipe to any other. Daimon emits Engrams →
Neuro stores them. Dreams reads from Neuro → produces new Engrams. Chain posts Engrams
on-chain. Everything flows through the six Synapse traits.

---

## 5. Core Types

| Old Name | New Name | Notes |
|---|---|---|
| `Signal` (legacy architectural noun) | **Engram** | The canonical architectural term is **Engram**. Use the legacy code name only when pointing at current source identifiers. |
| `Signal` (legacy Rust type name) | `Signal` (for now) | Legacy type name still exists in code. The architectural term remains **Engram** until the code rename lands. |
| `SignalBuilder` (legacy builder name) | **EngramBuilder** | Builder pattern for Engram construction. Code currently uses the legacy builder identifier. |
| `signal.rs` | `engram.rs` | Source file rename. |
| `EventBus<E>` (legacy trait name) | **Bus** | The transport fabric is documented as the **Bus** trait; `BroadcastBus` is the default in-process backend. |
| `Envelope<E>` (legacy wire type) | **Pulse** | The transport datum is a **Pulse**, not a user-facing envelope type. |
| `channel` / `subject` (legacy routing nouns) | **Topic** | Bus routing uses dot-separated **Topic** strings and `TopicFilter` expressions. |
| "1 noun, 6 verbs" | **Synapse Architecture** | The current architecture story is two mediums, two fabrics, and six operators rather than a single-medium mnemonic. |

---

## 6. Interface Names

| Old | New |
|---|---|
| Bardo Sanctum | **Roko Portal** (web dashboard) |
| bardo-terminal / Bardo | **Roko TUI** (terminal dashboard) |
| Mori TUI | **Roko TUI** |

---

## 7. Token Details

| Token | Chain | Notes |
|---|---|---|
| **KORAI** | Korai (mainnet) | 1% annual demurrage. Replaces GNOS. |
| **DAEJI** | Daeji (testnet) | Testnet equivalent of KORAI. |

When a legacy document mentions "GNOS token," the correct new name is "KORAI token" (or
"DAEJI token" if explicitly about testnet). When it mentions "golem chain," the correct
new name is "Korai chain."

---

## 8. Subsystem Names — Kept Unchanged

These names are preserved from the original architecture with no rename needed:

| Name | What It Is |
|---|---|
| **Mirage** / **mirage-rs** | In-process EVM simulator for transaction testing. |
| **Heartbeat** | The agent's cognitive loop — one complete seven-step cycle of SENSE → ASSESS → COMPOSE → ACT → VERIFY → PERSIST/BROADCAST → REACT, run at Gamma, Theta, or Delta speed. |
| **CoALA** | Cognitive Architecture for Language Agents (Sumers et al. 2023). The theoretical framework underlying the universal cognitive loop. |
| **Pheromone system** | Stigmergic coordination: agents leave decaying signals in shared substrate for indirect communication (Grassé 1959, Parunak et al. 2007). |
| **Sleepwalker** | Reduced-capability sleep mode for agents during low-activity periods. |
| **Oneirography** | Recording and analysis of Dreams cycle outputs. |
| **Hypnagogia** | The transitional state between waking and sleeping cognition, implemented as a creative hypothesis generator. |
| **ALMA** | Three-layer affect model (Gebhard 2005) informing the Daimon's emotional processing. |
| **Somatic markers** | Damasio's somatic marker hypothesis (Damasio 1994): emotional signals from past experience that bias decision-making. |
| **Library of Babel** | Cross-collective knowledge sharing — named after Borges' infinite library concept. |
| **Bazaar** | Commerce primitives for agent-to-agent economic interaction. |
| **MPP** | Machine Payment Protocol — protocol for agent-initiated micropayments. |
| **Testament** | Knowledge transfer between agents (repurposed from "death inheritance" to "knowledge export/import"). |
| **Portal** | Interface concept for user interaction surfaces. |
| **Spectre** | The procedurally generated creature visualization — a dot-cloud entity whose body encodes cognitive state. |
| **ROSEDUST** | Dark-only design language (rose on void-black). The visual identity system for all Roko interfaces. |

---

## 9. New Names (Not in Legacy Sources)

These terms are introduced in the new architecture and do not appear in legacy documents:

| Term | Definition |
|---|---|
| **Engram** | The core data type — a content-addressed, scored, decaying, lineage-tracked unit of cognition. Replaces "Signal" as the architectural noun. |
| **Pulse** | The ephemeral medium. A typed, topic-addressed, sequence-numbered wire record carried on a Bus and graduated to an Engram only when durable lineage matters. |
| **Bus** | The transport fabric and kernel primitive. Kernel trait for publishing, subscribing, and replaying Pulses across topic streams without becoming a persistence surface. |
| **Topic** | Dot-separated routing handle for Pulses on a Bus, such as `gate.verdict.emitted` or `agent.msg.chunk`. |
| **TopicFilter** | Declarative Bus subscription filter with `Exact`, `Glob`, `AnyOf`, `All`, `And`, `Or`, and `Not` forms. |
| **BusReceiver** | Subscriber handle returned by `Bus::subscribe()`. Delivers Pulses in publish order and carries sequence state for bounded replay. |
| **Datum** | Polymorphic operator input: `enum Datum<'a> { Engram(&'a Engram), Pulse(&'a Pulse) }`. Used when operators need to accept either medium without inventing a new trait family. |
| **Synapse Architecture** | Roko's architectural story: two mediums (Engram and Pulse), two fabrics (Substrate and Bus), and six operators. Replaces the older single-medium mnemonic. |
| **Five Layers** | Runtime (L0) / Framework (L1) / Scaffold (L2) / Harness (L3) / Orchestration (L4). |
| **Cognitive Cross-Cuts** | Neuro / Daimon / Dreams — subsystems injected across multiple layers. |
| **C-Factor** | Collective intelligence ratio metric (Woolley et al. 2010, Science 330). |
| **C-Score** | Composite optimization metric for collective performance. |
| **Three Cognitive Speeds** | Gamma (~5-15s reactive) / Theta (~75s reflective) / Delta (hours consolidation). |
| **Seven-Step Loop** | The canonical universal loop: SENSE → ASSESS → COMPOSE → ACT → VERIFY → PERSIST/BROADCAST → REACT. Cross-cuts inject into the operators around this loop; they are not extra sequential steps. |
| **16 T0 Probes** | Zero-LLM diagnostic probes that suppress ~80% of ticks to T0 (no inference call). |
| **VCG Attention Auction** | Vickrey-Clarke-Groves mechanism for truthful context budget allocation. |
| **Somatic Landscape** | k-d tree over 8D strategy space with 15% contrarian retrieval for diversity. |
| **Hypnagogia Engine** | Creative hypothesis generator: Thalamic Gate + Executive Loosener + Dali Interrupt + Homuncular Observer. |
| **Cognitive Kernel Primitives** | Namespaces, cognitive signals, scheduling, syscalls — the "operating system" layer. |
| **Korai Passport** | ERC-721 soulbound agent identity on the Korai chain. |
| **Spore / Sparrow** | Job market protocols for agent task discovery and claiming. |
| **ISFR** | Intersubjective Fact Registry — decentralized fact verification. |
| **Valhalla** | Privacy layer using TEE, PSI (Private Set Intersection), and ZK proofs. |

---

## 10. Glossary of Architectural Terms

### A

| Term | Definition |
|---|---|
| **A2A** | Agent-to-Agent protocol. Google's open standard for inter-agent communication. |
| **A2UI** | Agent-to-User-Interface. Agents generate their own UI components in ROSEDUST. |
| **Active Inference** | Framework from neuroscience (Friston 2010): self-organizing systems minimize prediction error through perception and action. Roko uses this as a conceptual framework for attention and tier routing. |
| **ADAS** | Automated Design of Agentic Systems (Hu et al. ICLR 2025). Meta-agent that searches the space of agent architectures. |
| **Adaptive Clock** | The scheduler that manages three cognitive speeds (Gamma/Theta/Delta) and adjusts cadence based on affect state and task characteristics. |
| **Agent Mesh** | P2P communication and knowledge sharing between Roko agents. Replaces "Styx." |
| **Attestation** | Cryptographic proof of Engram origin — Ed25519 signature with optional chain attestation. |

### B

| Term | Definition |
|---|---|
| **Body** | The typed payload of an Engram. Variants: Empty (marker), Text (UTF-8), Json (structured), Bytes (binary). |
| **Bus** | Kernel transport trait and fabric for Pulses. Exposes publish, subscribe, replay, sequencing, and ring-buffer health without becoming a persistence surface. See also [07-substrate-trait.md](./07-substrate-trait.md) and `tmp/refinements/03-bus-as-first-class.md`. |
| **Budget** | Resource constraints for Composer operations: max_tokens, max_signals, max_bytes, max_wall_ms. |

### C

| Term | Definition |
|---|---|
| **C-Factor** | Collective intelligence ratio: `C-Factor = Collective Performance / Sum(Individual Performances)`. Values > 1.0 indicate superlinear intelligence. (Woolley et al. 2010) |
| **C-Score** | Composite optimization metric: `gate_pass×0.3 + cost_efficiency×0.2 + speed×0.15 + first_try_rate×0.25 + knowledge_growth×0.1`. |
| **CascadeRouter** | Multi-stage model routing: confidence threshold → contextual bandit (LinUCB) → cost-aware selection. Persisted to `.roko/learn/cascade-router.json`. |
| **ChainBus** | Bus backend that turns Korai or Daeji log streams into topic-addressed Pulses such as `chain.deposit.emitted`. It is the transport sibling to `ChainSubstrate`, not a replacement for durable on-chain storage. |
| **ChainSubstrate** | Substrate backend for durable on-chain Engrams such as attestations, transactions, insights, bounties, and pheromones. Queries durable chain state; live chain notifications belong on `ChainBus`. |
| **Cognitive Loop** | The universal seven-step loop: SENSE → ASSESS → COMPOSE → ACT → VERIFY → PERSIST/BROADCAST → REACT. It draws from CoALA but treats cross-cuts such as Neuro, Daimon, and Dreams as injected concerns rather than sequential loop steps. |
| **Collective** | A group of cooperating agents. Replaces "Clade." |
| **Composer** | Synapse trait. Combines `Datum` inputs into one durable Engram under Budget constraints. Synchronous. Takes `&dyn Scorer`. |
| **ContentHash** | 32-byte BLAKE3 digest identifying an Engram. Computed from kind + body + author + tags. Score and decay are excluded. |
| **Context** | The shared runtime environment passed to every trait method. Carries time (`now_ms`), goal, session, and extension attributes. |

### D

| Term | Definition |
|---|---|
| **Datum** | Polymorphic operator input: `enum Datum<'a> { Engram(&'a Engram), Pulse(&'a Pulse) }`. Used by Scorer and Composer where the same operator should accept either medium. |
| **Daimon** | Motivation and focus subsystem (`roko-daimon`). Maintains PAD (Pleasure-Arousal-Dominance) vectors, six behavioral states, and somatic markers. The agent's self-model. |
| **Decay** | Time-based weight reduction for Engrams. Variants: None (permanent), HalfLife (exponential), Ttl (hard cutoff), Ebbinghaus (psychological forgetting curve). |
| **Delta** | Consolidation cognitive speed. Hours timescale. Dreams: replay, synthesis, pruning. Knowledge tier promotion. |
| **Dreams** | Offline learning subsystem (`roko-dreams`). Three-phase cycle: NREM replay + REM imagination + integration staging. |
| **Dual-Process** | Kahneman's System 1/System 2 mapped to Roko: T0 (direct tool call, no LLM) / T1 (fast model) / T2 (full model, deep reasoning). |

### E

| Term | Definition |
|---|---|
| **EFE** | Expected Free Energy. `G(π) = −Pragmatic Value − Epistemic Value`. Drives tier routing without hyperparameters. |
| **Engram** | The universal data type of Roko. Content-addressed (BLAKE3), scored (7-axis), decaying, lineage-tracked. Every piece of information in the system is an Engram. Currently named `Signal` in code; rename is Tier 0D. |
| **EngramBuilder** | Builder pattern for constructing Engrams. Currently named `SignalBuilder` in code. |
| **Episode** | A recorded sequence of agent actions and their outcomes. Stored as Engrams with `Kind::Episode`. |
| **EvoSkills** | Self-evolving skill libraries via adversarial surrogate verification (Zhao et al. 2024). |

### F-G

| Term | Definition |
|---|---|
| **Forensic AI** | Content-addressed causal replay — the ability to trace any decision back to its inputs via lineage chains. Regulatory pre-compliance moat. |
| **Gamma** | Reactive cognitive speed. ~5-15 second timescale. One complete loop tick: tool calls, LLM inference, verification. |
| **Gate** | Synapse trait. Verifies an Engram against ground truth and can optionally verify a Pulse window through a `verify_stream` path. Asynchronous. The bridge to external reality. |

### H-K

| Term | Definition |
|---|---|
| **HDC** | Hyperdimensional Computing. 10,240-bit vectors using XOR bind, majority bundle, Hamming similarity. Used for semantic similarity in knowledge retrieval. (Kanerva 2009, Cognitive Computation 1(2); Plate 2003; Frady et al. 2018) |
| **HeartbeatPolicy** | Policy-owned clock publisher that emits `heartbeat.gamma.tick`, `heartbeat.theta.tick`, and `heartbeat.delta.tick` Pulses on the Bus. REF09 frames heartbeat as a Bus producer rather than a special orchestration mechanism. |
| **Hypnagogia** | The creative hypothesis generator in `roko-dreams`. Four components: Thalamic Gate (stimulus filter), Executive Loosener (constraint relaxation), Dali Interrupt (capture mechanism), Homuncular Observer (coherence filter). |
| **Kind** | The semantic type of an Engram. `#[non_exhaustive]` enum with variants for agent runtime, verification, tasks, context, routing, memory, observability, chain participation, and `Custom(String)` for extensions. |
| **KORAI** | Mainnet token on the Korai chain. 1% annual demurrage. Replaces "GNOS." |
| **Korai** | Dedicated EVM chain for agent coordination. |

### L-N

| Term | Definition |
|---|---|
| **Lineage** | A vector of ContentHashes tracking the parent Engrams from which a new Engram was derived. Forms an audit DAG. |
| **MeshBus** | Bus backend for inter-agent transport over NATS or libp2p-style meshes. Carries Pulses such as `mesh.pheromone.deposited` without becoming the durable coordination store. |
| **MeshSubstrate** | Shared Substrate backend for durable multi-agent Engrams replicated across the mesh. Holds the pheromone and shared-knowledge records that `MeshBus` announces. |
| **Neuro** | Knowledge management subsystem (`roko-neuro`). Six knowledge types (Insight, Heuristic, Warning, CausalLink, StrategyFragment, AntiKnowledge). Four tiers (Transient, Working, Consolidated, Persistent). HDC encoding for similarity. |
| **NeuroStore** | The storage backend for the Neuro subsystem. |

### O-P

| Term | Definition |
|---|---|
| **Outcome** | Feedback about a prior Router selection. Carries success/failure, reward, cost, and latency. Used by Routers to learn via bandit algorithms. |
| **Pheromone** | A stigmergic Engram with time-based decay. Three types: THREAT (2h half-life), OPPORTUNITY (4h), WISDOM (24h). Agents deposit pheromones into shared Substrates and announce fresh deposits on Bus topics such as `mesh.pheromone.deposited`. |
| **Policy** | Synapse trait. Watches Pulse streams and emits `PolicyOutputs` in response. Policies publish live reactions as Pulses and persist summaries, verdict records, or graduated artifacts as Engrams. |
| **PolicyOutputs** | Policy return type with two channels: `pulses: Vec<Pulse>` for Bus publication and `engrams: Vec<Engram>` for Substrate persistence. |
| **Pulse** | Ephemeral medium carried by the Bus. Pulses are topic-addressed, sequence-numbered, ring-buffered for short replay, and not persisted by default. |
| **Provenance** | Who produced an Engram and how trustworthy they are. Fields: author (String), trust ([0,1]), tainted (bool), session (Option). |

### Q-R

| Term | Definition |
|---|---|
| **Query** | Filter specification for Substrate lookups. Fields: kinds, author, session, since_ms, until_ms, min_weight, tags, limit. |
| **ROSEDUST** | Dark-only design system. Rose on void-black. The visual identity for all Roko interfaces. |
| **Router** | Synapse trait. Selects among Engram candidates for durable retrieval or Pulse candidates for live routing. Has `feedback()` for learning from outcomes and may expose separate `select_engram` and `select_pulse` entry points. |

### S

| Term | Definition |
|---|---|
| **Score** | Multi-dimensional quality assessment of an Engram. 4 stable axes: confidence [0,1], novelty [0,1], utility [0,∞), reputation [0,∞). 3 extended axes (planned): precision [0,1], salience [0,1], coherence [0,1]. Effective score formula: `confidence × (1 + novelty) × (1 + utility) × reputation`. |
| **Scorer** | Synapse trait. Rates either medium, usually through monomorphic `score_engram` and `score_pulse` methods plus a thin `score(Datum, Context)` dispatcher. Synchronous. |
| **Selection** | Output of a Router: the chosen Engram or Pulse candidate, plus confidence, router name, and optional reasoning. |
| **Spectre** | Procedurally generated dot-cloud creature per agent. Body encodes cognitive state; eyes encode emotion; clarity encodes prediction accuracy. |
| **Substrate** | Storage fabric and kernel trait for durable Engrams. Multiple backends include MemorySubstrate, FileSubstrate, HdcSubstrate, and ChainSubstrate. |
| **Synapse Architecture** | The compositional model underlying Roko: two mediums moving through two fabrics, coordinated by six operators across five layers. |

### T-V

| Term | Definition |
|---|---|
| **T0 / T1 / T2** | Inference tiers. T0: no LLM call (direct tool call, ~80% of ticks). T1: fast model, shallow reasoning (~15%). T2: full model, deep reasoning (~5%). Routing emerges from active inference (EFE). |
| **Theta** | Reflective cognitive speed. ~75 second timescale. Summarize recent work, update Daimon state, check predictions. |
| **TickOutcome** | The result of one `loop_tick` invocation. In the generalized operator model it spans both durable persistence and live publication: candidates examined, composed Engram, Verdict, emitted Pulses, emitted Engrams, and written ContentHashes. |
| **Topic** | Dot-separated routing handle for Pulses. Topics name transport intent rather than storage identity, for example `mesh.pheromone.deposited` or `heartbeat.gamma.tick`. |
| **TopicFilter** | Declarative matcher for Bus subscriptions and replay queries. Supports exact, glob, set, and boolean composition. |
| **Verdict** | Output of a Gate. Contains: passed (bool), reason (String), gate name, score [0,1], optional detail, optional TestCount, optional error_digest, duration_ms. |

---

## 11. Naming Application Rules

1. **When quoting legacy sources verbatim**: Keep the old name in the quote but add a
   parenthetical: "(formerly Grimoire, now Neuro)."
2. **When paraphrasing**: Use the new name directly.
3. **Code samples**: Use new crate names (`roko-primitives`, `roko-runtime`, etc.) even if
   the current Rust code still uses old names. Note the current name in a comment.
4. **Struct/type names**: Say "Engram" in prose. In Rust code snippets, use `Signal` and
   add a comment like `// will be renamed to Engram in Tier 0D`.
5. **File paths**: All `bardo-*` → `roko-*`, all `mori-*` → `roko-*`.
6. **Never say**: "Golem SDK" → say "Agent SDK" or "Roko SDK".
7. **Never say**: "Mori + Golem" → say "Roko framework with coding and chain domain plugins."

---

## Academic Foundations

| Citation | Relevance |
|---|---|
| Woolley et al. 2010, Science 330(6004) | C-Factor: collective intelligence metric. |
| Sumers et al. 2023 (arXiv:2309.02427) | CoALA: cognitive architecture framework. |
| Friston 2010, Nature Reviews Neuroscience 11(2) | Free Energy Principle, active inference. |
| Kanerva 2009, Cognitive Computation 1(2) | Hyperdimensional Computing foundations. |
| Grassé 1959 | Stigmergy: indirect coordination via environmental signals. |
| Damasio 1994, Descartes' Error | Somatic markers: emotion biasing decision-making. |
| Mehrabian & Russell 1974 | PAD model: Pleasure-Arousal-Dominance emotional space. |
| Gebhard 2005 | ALMA: three-layer affect model. |
| Kahneman 2011, Thinking, Fast and Slow | Dual-process theory: System 1 / System 2. |

---

## Cross-References

- [00-vision-and-thesis.md](00-vision-and-thesis.md) — Why the architecture exists
- [02-engram-data-type.md](02-engram-data-type.md) — Full Engram specification
- [02b-pulse-ephemeral-event.md](02b-pulse-ephemeral-event.md) — Pulse as the ephemeral medium
- [03-score-7-axis-appraisal.md](03-score-7-axis-appraisal.md) — Score details
- [06-synapse-traits.md](06-synapse-traits.md) — The six Synapse traits
- [07-substrate-trait.md](07-substrate-trait.md) — Substrate as the storage fabric beside Bus
- [07b-bus-transport-fabric.md](07b-bus-transport-fabric.md) — Bus trait, Topics, and replay semantics
