# Naming and Glossary

> **TL;DR**: This is the canonical vocabulary reference for Roko. It now distinguishes shipping
> code from target-state design terms with explicit status tags, so docs stop describing planned
> architecture as if it already exists. Use this document when writing docs, code comments,
> interfaces, or external material. If another document disagrees, this glossary wins.
>
> **First-time readers**: this chapter is an A-Z lookup. Start here when another architecture
> doc uses a term you do not recognize, then follow the cited home doc for depth.
>
> **Source of this consolidation**: REF34 makes this file the canonical glossary chapter. See
> [tmp/refinements/34-glossary.md](../../tmp/refinements/34-glossary.md).

**Status**: Written

---

## Current Naming Map

Roko's naming story now mixes shipping terminology with target-state terms introduced by the
refinement series. Use the following map to distinguish what is already in the codebase from what
is still planned.

| Canonical term | Status | Use | Avoid |
|---|---|---|---|
| `Roko` | `[shipping]` | Project and framework name | `Bardo` / `Mori` (retired) |
| `Agent` | `[shipping]` | Running process or session | `Golem` (retired) |
| `Engram` | `[shipping]` | Durable record medium | `Signal` (retired durable term) |
| `Pulse` | `[planned]` | Target-state ephemeral transport medium | `Event`, `Envelope`, `Message`, `Signal` (retired wire terms) |
| `Substrate` | `[shipping]` | Storage fabric | legacy storage-only synonyms |
| `EventBus<E>` | `[shipping]` | Current live transport implementation | calling it retired |
| `Bus` | `[planned]` | Target-state transport fabric abstraction | presenting it as already shipped |
| `Topic` | `[planned]` | Target-state Pulse routing handle | `Channel`, `Subject` |
| `TopicFilter` | `[planned]` | Target-state subscription matcher | ad hoc routing filters |
| `Datum` | `[planned]` | Target-state polymorphic `Engram` or `Pulse` input | one-off sum types |
| `PulseSource` | `[planned]` | Target-state lightweight Pulse origin attribution | overloaded provenance terms |
| `Neuro` | `[shipping]` | Durable knowledge cross-cut | `Grimoire` (retired) |
| `Daimon` | `[built]` | Affect cross-cut | old loop-step framing for affect |
| `Dreams` | `[built]` | Delta-speed consolidation cross-cut | treating Dreams as a loop step |
| `Mesh` | `[planned]` | Agent-network layer | `Styx` (retired) |
| `Fleet` | `[planned]` | Agent roster | `Clade` (retired) |
| `StateHub` | `[built]` | Current dashboard/event hub; target-state projection layer over Bus + Substrate | TUI-only framing |
| `TypedContext` | `[planned]` | Structured domain situation payload | free-text-only context matching |
| `Custody` | `[planned]` | Chain-of-custody audit record | informal approval prose |

See also [07-naming](../../tmp/refinements/07-naming.md),
[02-engram-vs-pulse](../../tmp/refinements/02-engram-vs-pulse.md),
[03-bus-as-first-class](../../tmp/refinements/03-bus-as-first-class.md), and
[docs/00-architecture/07b-bus-transport-fabric.md](./07b-bus-transport-fabric.md).

## Conventions

- Terms in **bold** at the start of each entry are the canonical form.
- `Code terms` stay in backticks.
- Status tags use four labels:
  - `[shipping]` = working type, module, or behavior in the current codebase.
  - `[built]` = code exists, but the glossary term overstates how fully it is wired.
  - `[planned]` = target-state design term with no corresponding shipped type or runtime path yet.
  - `[retired]` = historical term deliberately replaced by newer vocabulary.
- Each entry cites a home doc. Refinement proposals use bare filenames in the label and a link
  to `tmp/refinements/...`; canonical architecture chapters use normal doc links.
- `(historical)` marks a retired or legacy term that may still appear in old docs or code.
- `(new)` marks a term introduced by the refinement series.
- Retired terms belong only in explicitly retired, historical, deprecated, legacy, formerly,
  old-name, renamed, or see-also contexts.

## A

**Active inference** — Predict-publish-correct loop carried across operators with `prediction.*`,
`outcome.*`, and `prediction.error.*` Pulses. Home: [10-self-learning-cybernetic-loops](../../tmp/refinements/10-self-learning-cybernetic-loops.md).

**ACT** — Step 4 of the seven-step universal loop: execute the composed Engram as an LLM call,
tool call, or chain call. Produces Pulses and usually a final durable Engram. Home:
[05-loop-retold](../../tmp/refinements/05-loop-retold.md) and
[Universal Cognitive Loop](./09-universal-cognitive-loop.md).

**Agent** — Running process or session that drives the universal loop end to end. Formerly `Golem`.
Home: [docs/00-architecture/12-five-layer-taxonomy.md](./12-five-layer-taxonomy.md).

**Agent mesh** — Peer-to-peer layer for inter-agent coordination over transport topics.
Formerly `Styx`. Phase 2+. Home: [09-phase-2-implications](../../tmp/refinements/09-phase-2-implications.md).

**Algedonic signal** — Cross-layer alarm that bypasses normal hierarchy when lower layers are
failing; emitted as a priority Pulse. Home:
[10-self-learning-cybernetic-loops](../../tmp/refinements/10-self-learning-cybernetic-loops.md).

**Annotation** — Typed human-authored Engram attached to another artifact such as an episode,
heuristic, plan, or diff. Home: [30-rich-ux-primitives](../../tmp/refinements/30-rich-ux-primitives.md) and
[Rich UX Primitives](../12-interfaces/23-rich-ux-primitives.md).

**ASSESS** — Step 2 of the seven-step loop: joint `Scorer` and `Router` pass that chooses the
next action and records confidence. Home: [05-loop-retold](../../tmp/refinements/05-loop-retold.md).

**Attestation** [built] — Cryptographic signature over an Engram `ContentHash`. The shipped code
already has `Attestation`, `ChainAttestation`, and sign/verify support; the level taxonomy
(`LocalAgent`, `OrgRole`, `ChainWitness`) is target-state. Home:
[32-safety-sandbox-provenance](../../tmp/refinements/32-safety-sandbox-provenance.md) and
[Provenance and Attestation](./05-provenance-and-attestation.md).

**Authorization** [built] — The current shipping safety layer authorizes actions through
`SafetyLayer::check_pre_execution()`, `AgentContract`, and `AgentWarrant`. The
`authorize(principal, action, target, ctx)` signature is target-state shorthand for that decision
boundary. Home:
[32-safety-sandbox-provenance](../../tmp/refinements/32-safety-sandbox-provenance.md).

## B

**Balance** [planned] *(new)* — An Engram's demurrage-taxed attention credit. Starts at `1.0`, decays over
time, and is restored by reinforcement. Home:
[12-knowledge-demurrage](../../tmp/refinements/12-knowledge-demurrage.md) and
[Attention as Universal Cognitive Currency](./25-attention-as-currency.md).

**BROADCAST** — Step 6b of the seven-step loop, co-equal with `PERSIST`: publish Pulses to the
Bus. Home: [05-loop-retold](../../tmp/refinements/05-loop-retold.md).

**Body** — Shared payload enum used by both Engrams and Pulses so graduation can preserve
identity. Home: [02-engram-vs-pulse](../../tmp/refinements/02-engram-vs-pulse.md) and
[Engram Data Type](./02-engram-data-type.md).

**Bus** [planned] *(promoted)* — Target-state kernel transport trait for Pulses; sibling to
`Substrate`. The current live transport code is `EventBus<E>` in `roko-runtime`. Home:
[03-bus-as-first-class](../../tmp/refinements/03-bus-as-first-class.md) and
[Bus Transport Fabric](./07b-bus-transport-fabric.md).

**`BusReceiver`** [planned] *(new)* — Handle returned by `Bus::subscribe`; yields matching Pulses in
publish order. Home: [03-bus-as-first-class](../../tmp/refinements/03-bus-as-first-class.md).

**BroadcastBus** [planned] *(new)* — Default in-process Bus backend wrapping
`tokio::sync::broadcast`. Home: [08-code-sketches](../../tmp/refinements/08-code-sketches.md).

## C

**c-factor** [built] — Collective-intelligence factor computed continuously from Bus and Substrate
statistics for agent cohorts. Home:
[13-collective-intelligence-c-factor](../../tmp/refinements/13-collective-intelligence-c-factor.md) and
[C-Factor: Collective Intelligence](./14-c-factor-collective-intelligence.md).

**Calibrator** [planned] — Policy that updates a Heuristic's `Calibration` after observing predictions
versus outcomes. Home:
[14-worldview-validation](../../tmp/refinements/14-worldview-validation.md) and
[10-self-learning-cybernetic-loops](../../tmp/refinements/10-self-learning-cybernetic-loops.md).

**Calibration** [planned] — Per-heuristic, per-claim, or per-operator record of trials, confirmations,
violations, Brier score, and Wilson confidence interval. Home:
[14-worldview-validation](../../tmp/refinements/14-worldview-validation.md).

**CascadeRouter** — Existing bandit-based model router that picks a model per turn. Home:
[10-self-learning-cybernetic-loops](../../tmp/refinements/10-self-learning-cybernetic-loops.md).

**ChainBus** [planned] *(Phase 2+)* — Bus backend that maps chain event logs into `chain.*` topics. Home:
[09-phase-2-implications](../../tmp/refinements/09-phase-2-implications.md).

**ChainSubstrate** [planned] — Substrate backend that persists attestations and durable insights on-chain.
Phase 2+. Home: [09-phase-2-implications](../../tmp/refinements/09-phase-2-implications.md).

**Chain witness** — Cryptographic witness over an Engram committed to a blockchain for
cross-deployment trust. Home:
[32-safety-sandbox-provenance](../../tmp/refinements/32-safety-sandbox-provenance.md).

**Claim** [planned] *(new)* — Structured hypothesis distilled from a `Paper` Engram, including falsifier,
context, effect size, and calibration. Home:
[16-research-to-runtime](../../tmp/refinements/16-research-to-runtime.md).

**`claim!` macro** [planned] — Build-time macro resolving a `ClaimId` into a runtime parameter that checks
against the replication ledger. Home:
[16-research-to-runtime](../../tmp/refinements/16-research-to-runtime.md).

**Cohort** [planned] — Set of agents working on a related task and measured together for c-factor. Home:
[13-collective-intelligence-c-factor](../../tmp/refinements/13-collective-intelligence-c-factor.md).

**Cold tier** [planned] — Substrate region for Engrams whose balance has reached the demurrage floor.
Content remains resolvable but moves to slower storage. Home:
[12-knowledge-demurrage](../../tmp/refinements/12-knowledge-demurrage.md) and
[Decay Variants](./04-decay-variants.md).

**Commons** [planned] — Cross-deployment pool of empirically validated heuristics. Home:
[14-worldview-validation](../../tmp/refinements/14-worldview-validation.md) and
[18-competitive-moat](../../tmp/refinements/18-competitive-moat.md).

**COMPOSE** — Step 3 of the seven-step loop: the `Composer` assembles a prompt Engram under a
budget. Home: [05-loop-retold](../../tmp/refinements/05-loop-retold.md).

**Composer** — One of the six operators. Takes a slice of `Datum` and produces an Engram,
usually a prompt. Home:
[04-operators-generalized](../../tmp/refinements/04-operators-generalized.md) and
[Scorer, Gate, Router, Composer, Policy](./08-scorer-gate-router-composer-policy.md).

**Consistency gate** — Stream gate that checks an output against its cited Engram support,
often via HDC similarity. Home:
[11-hyperdimensional-substrate](../../tmp/refinements/11-hyperdimensional-substrate.md).

**ContentHash** — `BLAKE3(kind, body, author, tags)` identifier for an Engram. Home:
[Engram Data Type](./02-engram-data-type.md).

**Context** [shipping] — Existing sidecar state passed to operators today; `TypedContext` is the newer,
structured domain-specific form. Home:
[25-domain-specific-agents](../../tmp/refinements/25-domain-specific-agents.md).

**Custody** [planned] *(new)* — Chain-of-custody record for auditable actions: who approved them, why,
what simulation ran, what result occurred, and what witness exists. Home:
[25-domain-specific-agents](../../tmp/refinements/25-domain-specific-agents.md) and
[32-safety-sandbox-provenance](../../tmp/refinements/32-safety-sandbox-provenance.md).

## D

**Daimon** — Affect cross-cut that maintains PAD state, biases `Scorer`, and gates actions.
Home: [Cognitive Cross-Cuts](./13-cognitive-cross-cuts.md).

**`Datum`** [planned] *(new)* — `enum Datum<'a> { Engram(&'a Engram), Pulse(&'a Pulse) }` used by
polymorphic operators. Home:
[04-operators-generalized](../../tmp/refinements/04-operators-generalized.md) and
[08-code-sketches](../../tmp/refinements/08-code-sketches.md).

**Decay** — Older durable-memory weighting family (`None`, `HalfLife`, `Ttl`, `Ebbinghaus`)
that is being superseded by `balance` plus `demurrage`. Home:
[Decay Variants](./04-decay-variants.md).

**Delta (speed)** — Slowest cognitive speed, used for background consolidation and Dreams.
Home: [Three Cognitive Speeds](./10-three-cognitive-speeds.md).

**Delta (projection)** — Incremental update to a `StateHub` projection's `State`. Home:
[26-statehub-rearchitecture](../../tmp/refinements/26-statehub-rearchitecture.md) and
[StateHub Projection Layer](../12-interfaces/22-statehub-projection-layer.md).

**Demurrage** [planned] *(new)* — Economic memory rule that taxes idle Engram balance over time and
restores weight through reinforcement. Home:
[12-knowledge-demurrage](../../tmp/refinements/12-knowledge-demurrage.md) and
[Attention as Universal Cognitive Currency](./25-attention-as-currency.md).

**Dissonance** — Learning signal emitted when applicable heuristics predict incompatible
outcomes. Home: [14-worldview-validation](../../tmp/refinements/14-worldview-validation.md).

**Domain** — One of the canonical domain bundles such as coding, research, blockchain, data,
ops, or writing. Home: [25-domain-specific-agents](../../tmp/refinements/25-domain-specific-agents.md).

**Dreams** — Delta-speed consolidation cross-cut; not a loop step. Dreams inject durable
results back into Substrate for later cycles. Home:
[Cognitive Cross-Cuts](./13-cognitive-cross-cuts.md) and
[09-phase-2-implications](../../tmp/refinements/09-phase-2-implications.md).

## E

**Ebbinghaus** — A forgetting-curve-style decay variant kept as historical context under the
new demurrage framing. Home: [Decay Variants](./04-decay-variants.md).

**Engram** — Roko's durable medium: content-addressed, lineage-bearing, scored, and persisted in
a `Substrate`. Home:
[02-engram-vs-pulse](../../tmp/refinements/02-engram-vs-pulse.md) and
[Engram Data Type](./02-engram-data-type.md).

**EngramBuilder** — Builder for constructing Engrams and attaching derived fields such as
fingerprint, lineage, and score. Home: [Engram Data Type](./02-engram-data-type.md).

**Envelope** *(historical)* — Old generic wrapper around transport payloads. Retired in favor of
`Pulse`. Home: [02-engram-vs-pulse](../../tmp/refinements/02-engram-vs-pulse.md) and
[07-naming](../../tmp/refinements/07-naming.md).

**Episode** — Engram kind recording a full agent turn, including inputs, tool calls, outputs,
and verdicts. Home: [05-learning/INDEX](../05-learning/INDEX.md).

**Event** *(historical)* — Retired as Roko's primary wire type name in favor of `Pulse`.
Colloquial prose may still use it for "something that happened," but not as the canonical type
name. Home: [07-naming](../../tmp/refinements/07-naming.md).

**EventBus** [shipping] — Current live generic broadcast-channel transport abstraction in
`roko-runtime/src/event_bus.rs`. Target-state work may evolve this into a kernel `Bus` trait with
`Pulse` payloads, but `EventBus<E>` is the transport code that ships today. Home:
[03-bus-as-first-class](../../tmp/refinements/03-bus-as-first-class.md).

## F

**Fabric** — Kernel data-movement primitive. Roko has two fabrics: `Substrate` for storage and
`Bus` for transport. Home:
[03-bus-as-first-class](../../tmp/refinements/03-bus-as-first-class.md).

**Falsifier** [planned] — Predicate attached to a `Claim` or `Heuristic` that specifies what observable
would refute it. Home:
[14-worldview-validation](../../tmp/refinements/14-worldview-validation.md) and
[16-research-to-runtime](../../tmp/refinements/16-research-to-runtime.md).

**Fingerprint** [built] *(new)* — `HDC fingerprint` attached to Engrams or related records for
similarity queries, clustering, consensus, and analogy. `HdcVector` exists today, but it is not
attached to every Engram on write. Home:
[11-hyperdimensional-substrate](../../tmp/refinements/11-hyperdimensional-substrate.md) and
[Engram Data Type](./02-engram-data-type.md).

**Fleet** [planned] — Deployment-scoped roster of agents. Formerly `Clade`. Home:
[13-collective-intelligence-c-factor](../../tmp/refinements/13-collective-intelligence-c-factor.md).

## G

**Gamma** — Fastest cognitive speed, usually the turn-level cadence. Home:
[Three Cognitive Speeds](./10-three-cognitive-speeds.md).

**Gate** — One of the six operators. Verifies an Engram or a Pulse window against truth or
policy. Home:
[04-operators-generalized](../../tmp/refinements/04-operators-generalized.md) and
[Scorer, Gate, Router, Composer, Policy](./08-scorer-gate-router-composer-policy.md).

**GateVerdict** — Engram kind produced by a Gate; includes pass/fail, reason, and evidence.
Home: [Scorer, Gate, Router, Composer, Policy](./08-scorer-gate-router-composer-policy.md).

**Golem** *(historical)* — Retired old name for `Agent`.

**Graduation** [planned] *(new)* — Converting a `Pulse` into an `Engram` when durable lineage and audit
matter. Home:
[02-engram-vs-pulse](../../tmp/refinements/02-engram-vs-pulse.md) and
[08-code-sketches](../../tmp/refinements/08-code-sketches.md).

**Grimoire** *(historical)* — Retired old name for `Neuro`.

## H

**Harness** — Deliverable surface and L3 layer concerned with gating, monitoring, and
supervision. Home: [Five-Layer Taxonomy](./12-five-layer-taxonomy.md).

**HDC** — Hyperdimensional Computing: 10,240-bit vectors with bind, bundle, permute, similarity,
and consensus operations. Home:
[11-hyperdimensional-substrate](../../tmp/refinements/11-hyperdimensional-substrate.md).

**`HdcVector`** — Rust type for the underlying HDC vector representation. Home:
[11-hyperdimensional-substrate](../../tmp/refinements/11-hyperdimensional-substrate.md).

**Heartbeat** — Cognitive clock publishing `heartbeat.gamma.tick`,
`heartbeat.theta.tick`, and `heartbeat.delta.tick` Pulses. Home:
[09-phase-2-implications](../../tmp/refinements/09-phase-2-implications.md) and
[16-heartbeat/INDEX](../16-heartbeat/INDEX.md).

**Heuristic** [built] *(new)* — First-class learning rule or heuristic record. The shipped code
has `HeuristicRule` and related learning logic, but not the full Engram variant described here.
Home:
[14-worldview-validation](../../tmp/refinements/14-worldview-validation.md).

**Holographic** — HDC property where partial information still retrieves the whole and damage
degrades gracefully. Home:
[11-hyperdimensional-substrate](../../tmp/refinements/11-hyperdimensional-substrate.md).

## I

**Identity fingerprint** — HDC vector characterizing an agent's recent Engrams for team
discovery, routing diversity, and similarity-aware coordination. Home:
[11-hyperdimensional-substrate](../../tmp/refinements/11-hyperdimensional-substrate.md).

**Intrinsic motivation** — Policy bias toward high prediction-error regions where the system can
still learn. Home:
[10-self-learning-cybernetic-loops](../../tmp/refinements/10-self-learning-cybernetic-loops.md).

## K

**Kernel** — The narrow set of core types and traits that every other crate depends on:
`Engram`, `Pulse`, `Substrate`, `Bus`, `Scorer`, `Gate`, `Router`, `Composer`, and `Policy`.
Home: [04-operators-generalized](../../tmp/refinements/04-operators-generalized.md).

**Kind** — Semantic category enum for Engrams and Pulses such as `Plan`, `Task`, `Episode`,
`GateVerdict`, `Heuristic`, and `Paper`. Home:
[Engram Data Type](./02-engram-data-type.md).

**Korai** — Chain integration layer. Historically bundled under `Styx`; now kept distinct from
the mesh. Home: [09-phase-2-implications](../../tmp/refinements/09-phase-2-implications.md).

## L

**Layer (L0-L4)** — Five-layer taxonomy: Runtime, Framework, Scaffold, Harness, and
Orchestration, with strictly downward dependencies. Home:
[Five-Layer Taxonomy](./12-five-layer-taxonomy.md).

**Lineage** — `Vec<ContentHash>` on an Engram pointing to its parents in the durable audit DAG.
Home: [Engram Data Type](./02-engram-data-type.md).

**`loop_tick`** — Universal cognitive loop function, revised to the seven-step framing. Home:
[05-loop-retold](../../tmp/refinements/05-loop-retold.md) and
[Universal Cognitive Loop](./09-universal-cognitive-loop.md).

## M

**MCP** — Model Context Protocol for tool integration over stdio or HTTP. Home:
[17-plugin-extension-architecture](../../tmp/refinements/17-plugin-extension-architecture.md) and
[18-tools/INDEX](../18-tools/INDEX.md).

**MetaGate** — Gate that runs against the agent's self-model rather than only external outputs.
Home: [10-self-learning-cybernetic-loops](../../tmp/refinements/10-self-learning-cybernetic-loops.md).

**Mesh** — Agent-network layer for multi-agent routing and coordination. Formerly `Styx` as the
umbrella term. Home:
[09-phase-2-implications](../../tmp/refinements/09-phase-2-implications.md).

**MultiBus** [planned] *(new)* — Bus backend composing multiple backends behind one interface. Home:
[03-bus-as-first-class](../../tmp/refinements/03-bus-as-first-class.md).

## N

**Neuro** — Durable knowledge cross-cut covering storage, distillation, and tier progression.
Formerly `Grimoire`. Home:
[Cognitive Cross-Cuts](./13-cognitive-cross-cuts.md) and
[06-neuro/INDEX](../06-neuro/INDEX.md).

**Novelty** [planned] — `1 - max(similarity)` over top-K HDC neighbors; used by demurrage reinforcement to
reward uniqueness as well as reuse. Home:
[12-knowledge-demurrage](../../tmp/refinements/12-knowledge-demurrage.md).

## O

**Operator** — One of the six kernel verb traits: `Scorer`, `Gate`, `Router`, `Composer`,
`Policy`, plus the fabric traits `Substrate` and `Bus` as storage and transport operators.
Home: [04-operators-generalized](../../tmp/refinements/04-operators-generalized.md).

**Orchestrator** — Layer-4 subsystem that runs plans, dispatches tasks, and enforces merge
queues. Home: [01-orchestration/INDEX](../01-orchestration/INDEX.md).

**Outcome Pulse** [planned] *(new)* — Pulse on an `outcome.*` topic that closes the loop on a prior
prediction Pulse. Home:
[10-self-learning-cybernetic-loops](../../tmp/refinements/10-self-learning-cybernetic-loops.md).

## P

**PAD vector** — Pleasure-Arousal-Dominance affective state maintained by `Daimon`. Home:
[09-daimon/INDEX](../09-daimon/INDEX.md).

**Paper** [planned] *(new)* — Engram kind representing an academic paper together with DOI, authors,
abstract, fingerprint, and extracted claims. Home:
[16-research-to-runtime](../../tmp/refinements/16-research-to-runtime.md).

**PERSIST** — Step 6a of the seven-step loop: persist an Engram to `Substrate`. Home:
[05-loop-retold](../../tmp/refinements/05-loop-retold.md).

**Pheromone** — Engram kind used for stigmergic coordination between agents. Home:
[09-phase-2-implications](../../tmp/refinements/09-phase-2-implications.md).

**Plan** — Engram kind representing a structured multi-task plan with DAG edges. Home:
[01-orchestration/INDEX](../01-orchestration/INDEX.md).

**Playbook** — Engram kind storing a distilled reusable action sequence. Home:
[05-learning/INDEX](../05-learning/INDEX.md).

**Plugin** [planned] — Third-party extension package. Tiers span prompts, profiles, manifests, native,
and WASM. Home:
[17-plugin-extension-architecture](../../tmp/refinements/17-plugin-extension-architecture.md).

**Policy** — One of the six operators; reacts to streams of Pulses and emits new Pulses plus
Engrams. Home:
[04-operators-generalized](../../tmp/refinements/04-operators-generalized.md) and
[Scorer, Gate, Router, Composer, Policy](./08-scorer-gate-router-composer-policy.md).

**`PolicyOutputs`** [planned] *(new)* — Return type of `Policy::decide` containing `{ pulses, engrams }`.
Home: [04-operators-generalized](../../tmp/refinements/04-operators-generalized.md).

**Prediction Pulse** [planned] *(new)* — Pulse on a `prediction.*` topic emitted when an operator makes a
decision that should later be checked against reality. Home:
[10-self-learning-cybernetic-loops](../../tmp/refinements/10-self-learning-cybernetic-loops.md).

**PRD** — Product Requirements Document represented in `.roko/prd/` as a work item's lifecycle
directory. Home: [12-interfaces/INDEX](../12-interfaces/INDEX.md).

**Principal** — User, agent, or plugin subject to an authorization decision. Home:
[32-safety-sandbox-provenance](../../tmp/refinements/32-safety-sandbox-provenance.md).

**Projection** [planned] *(new)* — Named, typed, live-updating view on Bus and Substrate with
`State`, `Delta`, and a fold function. The current `StateHub` exists, but this typed projection
model is still target-state. Home:
[26-statehub-rearchitecture](../../tmp/refinements/26-statehub-rearchitecture.md) and
[StateHub Projection Layer](../12-interfaces/22-statehub-projection-layer.md).

**Profile** — Named bundle of defaults. Use the bare term for deployment shapes such as
`laptop`, `single-server`, `container`, `clustered`, or `edge`; say `domain profile` when the
bundle customizes agent behavior instead. Home:
[24-deployment-ux](../../tmp/refinements/24-deployment-ux.md) and
[25-domain-specific-agents](../../tmp/refinements/25-domain-specific-agents.md).

**Provenance** — Full author, trust, taint, and attestation record on an Engram. Home:
[Provenance and Attestation](./05-provenance-and-attestation.md).

**Pulse** [planned] *(new)* — Target-state ephemeral medium: typed, sequence-numbered,
topic-addressed, ring-buffered, and not persisted by default. Lives on a `Bus` and may graduate
to an Engram.
Home: [02-engram-vs-pulse](../../tmp/refinements/02-engram-vs-pulse.md).

**`PulseSource`** [planned] *(new)* — Lightweight origin attribution on every Pulse, usually
`{ component, agent_id }`. Home:
[08-code-sketches](../../tmp/refinements/08-code-sketches.md).

## Q

**`query_similar`** [planned] *(new)* — Substrate method that returns Engrams within an HDC radius of a
query fingerprint. Home:
[11-hyperdimensional-substrate](../../tmp/refinements/11-hyperdimensional-substrate.md) and
[Substrate Trait](./07-substrate-trait.md).

## R

**REACT** — Step 7 of the seven-step loop: `Policy::decide` emits follow-on Pulses and Engrams.
Home: [05-loop-retold](../../tmp/refinements/05-loop-retold.md).

**Reinforcement** [planned] *(new)* — Bonus applied to an Engram's balance when it is cited, retrieved,
gated, surprising, or agent-quoted. Home:
[12-knowledge-demurrage](../../tmp/refinements/12-knowledge-demurrage.md).

**`ReinforceKind`** [planned] *(new)* — Enum of reinforcement causes such as `Cited`, `Retrieved`,
`Gated`, `Surprised`, and `AgentQuoted`. Home:
[12-knowledge-demurrage](../../tmp/refinements/12-knowledge-demurrage.md).

**Replication ledger** [planned] *(new)* — Per-claim record of paper-reported effect versus observed
effect, confidence interval, and replication status. Home:
[16-research-to-runtime](../../tmp/refinements/16-research-to-runtime.md).

**Role** — Composition template plus tool allow-list and gate defaults. Home:
[12-interfaces/21-user-ux-running-agents.md](../12-interfaces/21-user-ux-running-agents.md).

**Router** — One of the six operators; picks among candidates. Home:
[04-operators-generalized](../../tmp/refinements/04-operators-generalized.md).

**Runtime** — Layer-0 subsystem containing the process supervisor, cancellation, `Bus`, and
`Substrate`. Home: [Five-Layer Taxonomy](./12-five-layer-taxonomy.md).

## S

**Scaffold** — Deliverable surface and L2 layer where contexts and composed work products live.
Home: [Five-Layer Taxonomy](./12-five-layer-taxonomy.md).

**Score** — Seven-axis appraisal attached to an Engram by the `Scorer`. Home:
[Score: 7-Axis Appraisal](./03-score-7-axis-appraisal.md).

**Scorer** — One of the six operators; computes score for any `Datum`. Home:
[04-operators-generalized](../../tmp/refinements/04-operators-generalized.md).

**SENSE** — Step 1 of the seven-step loop: perceive from `Substrate`, `Bus`, and external I/O.
Home: [05-loop-retold](../../tmp/refinements/05-loop-retold.md).

**Session** — Bounded run of agent interaction, resumable across CLI, TUI, Chat, and Web.
Home:
[23-user-ux-running-agents](../../tmp/refinements/23-user-ux-running-agents.md) and
[User UX Running Agents](../12-interfaces/21-user-ux-running-agents.md).

**Signal** *(historical)* — Retired old name for `Engram` as the durable record. The stale
"Signal = Engram" disclaimer is retired and should not appear in new prose. Home:
[07-naming](../../tmp/refinements/07-naming.md).

**SPI** — Service Provider Interface for plugin extension points. Home:
[17-plugin-extension-architecture](../../tmp/refinements/17-plugin-extension-architecture.md).

**Stigmergy** — Indirect coordination via shared environment, implemented with `Pheromone`
Engrams and `mesh.pheromone.*` Pulses. Home:
[09-phase-2-implications](../../tmp/refinements/09-phase-2-implications.md).

**StateHub** [built] *(promoted)* — Current dashboard/event hub that can broadcast state updates.
The typed, filterable projection layer described here is target-state rather than fully wired
today. Home:
[26-statehub-rearchitecture](../../tmp/refinements/26-statehub-rearchitecture.md) and
[StateHub Projection Layer](../12-interfaces/22-statehub-projection-layer.md).

**Styx** *(historical)* — Retired old umbrella term that split into `Mesh` and `Korai`.

**Substrate** — Kernel storage trait for durable Engrams. Home:
[Substrate Trait](./07-substrate-trait.md) and
[03-bus-as-first-class](../../tmp/refinements/03-bus-as-first-class.md).

**Swarm** — Collective of agents subscribed to the same topic set. Home:
[09-phase-2-implications](../../tmp/refinements/09-phase-2-implications.md).

## T

**Taint** [built] — Metadata indicating untrusted input origin that propagates through derived
Engrams until explicit review or approval. The current code ships `Provenance.tainted: bool`; the
richer enum described in the refinements is target-state. Home:
[32-safety-sandbox-provenance](../../tmp/refinements/32-safety-sandbox-provenance.md) and
[Cognitive Immune System](./26-cognitive-immune-system.md).

**Theta** — Middle cognitive speed, usually plan-level cadence. Home:
[Three Cognitive Speeds](./10-three-cognitive-speeds.md).

**Topic** [planned] *(new)* — Routing handle for Pulses. Dot-separated lowercase strings such as
`gate.verdict.emitted`. Home:
[03-bus-as-first-class](../../tmp/refinements/03-bus-as-first-class.md),
[07-naming](../../tmp/refinements/07-naming.md), and
[Bus Transport Fabric](./07b-bus-transport-fabric.md).

**`TopicFilter`** [planned] *(new)* — Declarative subscription matcher with variants such as `Exact`,
`Glob`, `AnyOf`, `All`, `And`, `Or`, and `Not`. Home:
[03-bus-as-first-class](../../tmp/refinements/03-bus-as-first-class.md) and
[Bus Transport Fabric](./07b-bus-transport-fabric.md).

**Trust score** — Per-agent-pair or per-topic reputation measure used during collective routing
and c-factor analysis. Home:
[13-collective-intelligence-c-factor](../../tmp/refinements/13-collective-intelligence-c-factor.md).

**TypedContext** [planned] *(new)* — Structured domain situation data, usually
`{ domain, fields: BTreeMap<Key, Value> }`, so gates and heuristics match on typed predicates
instead of free text. Home:
[25-domain-specific-agents](../../tmp/refinements/25-domain-specific-agents.md).

## U

**Undo** — Three-level reversal mechanism: ephemeral edits, short-term command undo, and
long-term replay-based revert. Home:
[23-user-ux-running-agents](../../tmp/refinements/23-user-ux-running-agents.md).

**Universal loop** — Seven-step cognitive loop: `SENSE`, `ASSESS`, `COMPOSE`, `ACT`, `VERIFY`,
`PERSIST`, `BROADCAST`, and `REACT`, with `PERSIST` and `BROADCAST` co-equal in step 6. Home:
[05-loop-retold](../../tmp/refinements/05-loop-retold.md) and
[Universal Cognitive Loop](./09-universal-cognitive-loop.md).

## V

**Verdict** — Output of a Gate, always materialized as a `GateVerdict` Engram so the durable
audit DAG is preserved. Home:
[04-operators-generalized](../../tmp/refinements/04-operators-generalized.md).

**VERIFY** — Step 5 of the seven-step loop: Gate or stream gate verifies an Engram or Pulse
window and emits a verdict. Home: [05-loop-retold](../../tmp/refinements/05-loop-retold.md).

## W

**Watchdog** [planned] — Policy subscribed to a Claim's falsifier predicate across episodes so the
replication ledger updates automatically. Home:
[16-research-to-runtime](../../tmp/refinements/16-research-to-runtime.md).

**Wilson CI** [planned] — Wilson score interval used as a confidence bound in calibration. Home:
[14-worldview-validation](../../tmp/refinements/14-worldview-validation.md).

**WisdomGate** — Gate enforcing Surowiecki's four conditions before a consensus Engram is
finalized. Home:
[13-collective-intelligence-c-factor](../../tmp/refinements/13-collective-intelligence-c-factor.md).

**Worldview** [planned] *(new)* — Co-citation cluster of mutually supporting heuristics that dominates a
domain-fingerprinted region of situations. Home:
[14-worldview-validation](../../tmp/refinements/14-worldview-validation.md).

**Witness** — See chain witness. Home:
[32-safety-sandbox-provenance](../../tmp/refinements/32-safety-sandbox-provenance.md).

## Retired / Deprecated Terms

These terms may appear in historical code or older docs, but they are retired and should not be
used in new work except in explicitly retired, deprecated, historical, legacy, old-name, or
formerly-marked contexts.

| Old | Replaced by | Reason |
|---|---|---|
| `Signal` (retired durable term) | `Engram` | Durable-record rename already landed |
| `Signal` (retired ephemeral candidate) | `Pulse` | Do not reuse `Signal` for the wire medium |
| `Envelope<E>` (historical wrapper name) | `Pulse` | Internal wrapper name leaked into architecture prose |
| `Message` (retired wire term) | `Pulse` for transport, `ChatMessage` for LLM transcripts | `Message` is overloaded |
| `Event` (retired wire term) | `Pulse` | Too generic and collides with framework vocabulary |
| `Bardo`, `Mori` (retired project codenames) | `Roko` | Retired project codenames |
| `Golem` (retired runtime-entity name) | `Agent` | Retired runtime-entity name |
| `Styx` (retired umbrella term) | `Mesh` + `Korai` | One umbrella term split into two clearer concepts |
| `Grimoire` (retired knowledge-cross-cut name) | `Neuro` | Retired knowledge-cross-cut name |
| `Clade` (retired roster term) | `Fleet` | `Fleet` is the conventional roster term |
| `Signal = Engram` disclaimer (retired) | remove the disclaimer | `Engram` and `Pulse` are distinct mediums |
| retired lifecycle framing like `mortal`, `death`, or `reincarnation` | remove the framing | Use custody, export/import, resource, or budget language instead |

`EventBus<E>` is intentionally not in the retired table. It is the live transport implementation in
the codebase today. Use `EventBus<E>` when referring to current code and `Bus` only when discussing
the target-state abstraction proposed by the refinement docs.

## Terms Deliberately Not Defined Here

Some words still use ordinary engineering meaning rather than a formal Roko-specific definition:

- `session` in the OIDC or HTTP sense
- `task` in the general async-runtime sense
- `model` when the text clearly means an LLM, not a runtime or mental model
- `cost` when the text simply means currency spend

If any of those starts behaving like a technical term in architecture prose, promote it into
this glossary in the same change.

## Maintenance

- Every new technical term introduced in a refinement or architecture doc should add a glossary
  entry in the same change.
- Retiring a term moves it into the retired table with a reason.
- Cross-references elsewhere in `docs/` should use the spellings in this chapter.
- Review this chapter whenever a new primitive, cross-cut, interface surface, or safety concept
  becomes load-bearing.

## See Also

- [Vision and Core Thesis](./00-vision-and-thesis.md)
- [Engram Data Type](./02-engram-data-type.md)
- [Substrate Trait](./07-substrate-trait.md)
- [Bus Transport Fabric](./07b-bus-transport-fabric.md)
- [Universal Cognitive Loop](./09-universal-cognitive-loop.md)
- [Cognitive Cross-Cuts](./13-cognitive-cross-cuts.md)
- [StateHub Projection Layer](../12-interfaces/22-statehub-projection-layer.md)
- [07-naming](../../tmp/refinements/07-naming.md)
- [31-synergy-integration-map](../../tmp/refinements/31-synergy-integration-map.md)
- [34-glossary](../../tmp/refinements/34-glossary.md)
