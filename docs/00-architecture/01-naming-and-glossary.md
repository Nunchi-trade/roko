# Naming Map and Glossary

> **Abstract:** This document is the authoritative naming map for Roko's kernel vocabulary.
> Use `Engram` for the durable record, `Pulse` for the ephemeral wire medium, `Substrate` for
> storage, `Bus` for transport, `Topic` for routing, `TopicFilter` for subscription matching,
> `Datum` for Engram-or-Pulse operator inputs, and `PulseSource` for transport-time producer
> attribution. When another document disagrees, this glossary wins. See also
> [tmp/refinements/07-naming.md](../../tmp/refinements/07-naming.md),
> [tmp/refinements/09-phase-2-implications.md](../../tmp/refinements/09-phase-2-implications.md),
> [07-substrate-trait.md](./07-substrate-trait.md),
> [07b-bus-transport-fabric.md](./07b-bus-transport-fabric.md), and
> [08-scorer-gate-router-composer-policy.md](./08-scorer-gate-router-composer-policy.md).

> **Implementation**: Shipping

---

## 1. Canonical Naming Decisions

Roko's architecture story is now explicit: two mediums, two fabrics, six operators. The kernel
one-liner is:

> Roko's kernel has two mediums (`Engram` for durable content-addressed and decayed record; `Pulse` for
> ephemeral topic-addressed sequence-bearing transport) moving through two fabrics
> (`Substrate` for storage; `Bus` for transport), acted on by six operators.

The current project vocabulary is:

| Current Name | Use | Notes |
|---|---|---|
| `Roko` | Project and framework name | Use for the overall system and documentation set. |
| `Agent` | Runtime process or session | Use for one autonomous worker or assistant instance. |
| `Fleet` | Agent roster | Use for a named set of agents under one operator or policy surface. |
| `Mesh` | Agent network layer | Use for multi-agent transport and topology, especially Phase 2+ networking. |
| `Neuro` | Durable knowledge cross-cut | Injects into Substrate reads and Composer assembly. |
| `Daimon` | Affect cross-cut | Injects into assessment bias and act gating. |
| `Dreams` | Delta-speed consolidation cross-cut | Produces durable outputs for later cycles. |

This document intentionally does not restate the retired `Signal = Engram` equivalence
disclaimer. `Engram` is the durable name; `Pulse` is the ephemeral sibling medium.

---

## 2. Configuration Files

| Current Path | Use |
|---|---|
| `roko.toml` | Primary user-facing configuration file |
| `.roko/` | Local runtime state, caches, transcripts, and learned artifacts |
| `.roko/learn/` | Learned routing state and policy artifacts |

---

## 3. Crate Names

The naming contract for the kernel crates is:

| Crate | Responsibility |
|---|---|
| `roko-core` | Core kernel vocabulary including `Engram`, `Pulse`, `Topic`, `TopicFilter`, `Datum`, `PulseSource`, `Substrate`, and `Bus` |
| `roko-agent` | Agent runtime, model/tool execution, and live Pulse production |
| `roko-orchestrator` | Plan DAG execution, scheduling, and orchestration topics |
| `roko-neuro` | Durable knowledge management and distillation |
| `roko-daimon` | PAD-vector affect and behavioral modulation |
| `roko-dreams` | Delta-speed replay, synthesis, and consolidation |
| `roko-chain` | Durable chain integration plus chain-facing Bus backends |

User-facing docs should describe those crates in current vocabulary rather than older umbrella
names. When a concept spans multiple crates, describe the concept first and the crate boundary
second.

---

## 4. Crate Dissolution: `roko-golem` (legacy umbrella crate)

The old umbrella crate is not part of the current naming story. Refer to the concrete subsystem
crates directly.

| Legacy Crate or Symbol | Current Replacement | Notes |
|---|---|---|
| `roko-golem` (legacy umbrella crate) | No umbrella replacement | Use the standalone crates directly. |
| `roko-golem/daimon.rs` (legacy path) | `roko-daimon` | Affect belongs to the Daimon cross-cut. |
| `roko-golem/grimoire.rs` (legacy path) | `roko-neuro` | Durable knowledge belongs to Neuro. |
| `roko-golem/dreams.rs` (legacy path) | `roko-dreams` | Delta-speed consolidation belongs to Dreams. |
| `roko-golem/chain_witness.rs` (legacy path) | `roko-chain` | Chain witness behavior belongs in chain-facing crates. |

---

## 5. Core Types

### 5.1 Canonical Kernel Vocabulary

| Term | Canonical Use | Notes |
|---|---|---|
| `Engram` | Durable record medium | Content-addressed, lineage-tracked, decayed, scored, and persisted in a Substrate. |
| `Pulse` | Ephemeral transport medium | Topic-addressed, sequence-bearing, ring-buffered, and not persisted by default. |
| `Substrate` | Storage fabric | Persists Engrams and supports durable queries. |
| `Bus` | Transport fabric | Publishes, subscribes, and replays Pulses by Topic. |
| `Topic` | Routing handle | Dot-separated lowercase identifier such as `gate.verdict.emitted`. |
| `TopicFilter` | Subscription and replay selector | Declarative matcher for Bus consumers. |
| `Datum<'a>` | Either-medium operator input | `enum Datum<'a> { Engram(&'a Engram), Pulse(&'a Pulse) }`. |
| `PulseSource` | Transport-time producer attribution | Lightweight source identifier carried on a Pulse before graduation. |
| `BusReceiver` | Subscriber handle | Delivers matching Pulses in publish order with bounded replay state. |
| `u64` sequence id | Bus ordering primitive | The default sequence identifier for Pulse ordering. |

### 5.2 Prominent Retired and Avoided Names

Every retired term appears below with its current replacement. Outside explicitly retired or
legacy contexts like this table, do not use these names in new prose.

| Retired or Legacy Form | Use Instead | Reason |
|---|---|---|
| `Signal` (retired durable-record name) | `Engram` | The durable medium keeps the Engram name; do not reclaim `Signal` for a different concept. |
| `Signal` (retired ephemeral candidate name) | `Pulse` | The ephemeral medium keeps the `Pulse` name; do not reuse `Signal` for the wire type. |
| `Signal = Engram` (retired equivalence disclaimer) | Delete the disclaimer | The architecture now distinguishes durable `Engram` from ephemeral `Pulse`. |
| `SignalBuilder` (legacy builder name) | `EngramBuilder` | Builder naming should match the durable medium. |
| `EventBus<E>` (deprecated transport trait name) | `Bus` | The transport trait is the Bus; backend names stay specific. |
| `Envelope<E>` (legacy wrapper name) | `Pulse` | Envelope can remain an internal implementation detail, not the user-facing type. |
| `Event` (retired primary wire-type name) | `Pulse` | Too generic and collides with Rust ecosystem imports. |
| `Message` (retired primary wire-type name) | `Pulse` or `ChatMessage` | Use `Pulse` for transport and `ChatMessage` only for LLM transcripts. |
| `Channel` (legacy routing noun) | `Topic` | Bus routing uses Topics. |
| `Subject` (legacy routing noun) | `Topic` | Bus routing uses Topics. |
| `Grimoire` (retired cross-cut name) | `Neuro` | Durable knowledge is the Neuro cross-cut. |
| `Styx` (retired umbrella name) | `Mesh` and `Korai` | Use `Mesh` for the agent network and `Korai` for the chain. |
| `Clade` (retired roster name) | `Fleet` | Use Fleet for a roster and Mesh for the network. |
| `Bardo` (retired project name) | `Roko` | The framework name is Roko. |
| `Mori` (retired project or product name) | `Roko` | Use `Roko` in architecture prose; name the orchestrator surface directly only when needed. |
| `Golem` (retired runtime entity name) | `Agent` | Runtime workers are agents. |
| `mortal` / `death` / `reincarnation` (retired lifecycle framing) | Remove the framing | Use resource, custody, budget, or export/import language instead. |

### 5.3 Naming Rules

1. Use `Engram` when the object must be durable, auditable, or lineage-bearing.
2. Use `Pulse` when the object exists to move through a `Bus` and may be discarded afterward.
3. Use `Topic` for Pulse routing keys and `TopicFilter` for matching logic.
4. Use `Datum` only when an operator truly accepts either medium.
5. Use `PulseSource` for lightweight producer attribution and `Provenance` for durable Engram
   attribution after graduation.
6. Keep retired names confined to explicit retirement tables, migration notes, or historical
   references.

---

## 6. Interface Names

| Current Interface | Use |
|---|---|
| `Roko CLI` | Command-line entry point and scripting surface |
| `Roko TUI` | Terminal dashboard and interactive console |
| `Roko Portal` | Web dashboard and browser surface |
| `HTTP API` | Programmatic control plane |
| `WebSocket` / `SSE` surfaces | Live Pulse delivery to clients and observers |

---

## 7. Token Details

| Token | Network | Notes |
|---|---|---|
| `KORAI` | Korai mainnet | Mainnet token name. |
| `DAEJI` | Daeji testnet | Testnet token name. |

---

## 8. Subsystem Names — Kept Unchanged

These names remain current and do not need renaming:

| Name | What It Is |
|---|---|
| `Heartbeat` | The cognitive clock and three-speed cadence |
| `Mirage` | Local EVM simulation environment |
| `Korai` | Chain network and ecosystem name |
| `Daeji` | Testnet network name |
| `Spectre` | Visual representation layer |
| `Portal` | User-facing portal concept |

---

## 9. New Names (Not in Legacy Sources)

The following names are load-bearing additions in the current architecture:

| Term | Definition |
|---|---|
| `Pulse` | Ephemeral sibling medium to `Engram`, carried on the `Bus` and graduated only when durable lineage matters. |
| `Bus` | First-class transport fabric paired with `Substrate` in the kernel. |
| `Topic` | Dot-separated routing namespace for Pulses. |
| `TopicFilter` | Declarative matcher used by subscriptions and replay queries. |
| `Datum` | Either-medium enum used by generalized operators. |
| `PulseSource` | Lightweight source attribution on a Pulse before durable provenance exists. |
| `BusReceiver` | Subscriber handle that yields matching Pulses in order. |
| `ChainBus` | Bus backend that maps chain logs into `chain.*` Pulses while `ChainSubstrate` handles durable on-chain Engrams. |
| `MeshBus` | Bus backend for collective pub/sub topics such as `mesh.pheromone.deposited`. |
| `MeshSubstrate` | Shared durable Engram backend for mesh replication, collective knowledge, and pheromone deposits. |
| `HeartbeatPolicy` | Runtime policy that publishes `heartbeat.gamma.tick`, `heartbeat.theta.tick`, and `heartbeat.delta.tick` Pulses. |
| `Synapse Architecture` | The architecture story of two mediums, two fabrics, and six operators. |

### 9.1 Topic Namespace Guidance

Canonical Topic strings should be lowercase and dot-separated. Example prefixes include:

| Prefix | Meaning |
|---|---|
| `orchestration.*` | Plan and task lifecycle |
| `agent.*` | Agent turn, chunk, and session events |
| `gate.*` | Gate verdicts and pipeline state |
| `safety.*` | Approvals, taint, custody, and permissions |
| `conductor.*` | Runtime health and breaker signals |
| `heartbeat.*` | Cognitive clock ticks and timing telemetry |
| `substrate.*` | Durable storage lifecycle events |
| `chain.*` | Phase 2+ chain forwarding topics |
| `mesh.*` | Phase 2+ multi-agent mesh topics |

Use owned prefixes for third-party extensions rather than publishing into shared system
prefixes without coordination.

---

## 10. Glossary of Architectural Terms

| Term | Definition |
|---|---|
| `Bus` | Kernel transport trait for publishing, subscribing, and bounded replay of Pulses. |
| `BusReceiver` | Subscription handle returned by the Bus for ordered Pulse delivery. |
| `Datum` | Either-medium enum used when operators accept either `Engram` or `Pulse`. |
| `Daimon` | Affect cross-cut that biases assessment and gates action. |
| `Dreams` | Delta-speed consolidation cross-cut that writes durable results back to storage. |
| `Engram` | Durable cognitive record stored in a Substrate and identified by content hash. |
| `Fleet` | Roster of agents under shared coordination or ownership. |
| `Mesh` | Agent-network layer for multi-agent communication. |
| `Neuro` | Durable knowledge cross-cut that influences storage reads and composition. |
| `Pulse` | Ephemeral transport record published on a Bus and retained only as long as the stream requires. |
| `PulseSource` | Lightweight producer identity carried on a Pulse. |
| `Substrate` | Storage fabric for Engrams and durable retrieval. |
| `Synapse Architecture` | The kernel framing of two mediums, two fabrics, six operators, five layers, three speeds, and three cross-cuts. |
| `Topic` | Routing handle for Pulses on the Bus. |
| `TopicFilter` | Declarative matcher for Topic-based subscription and replay. |

---

## See Also

- [02-engram-data-type.md](./02-engram-data-type.md) for the durable record medium
- [06-synapse-traits.md](./06-synapse-traits.md) for operator boundaries across the two mediums
- [07-substrate-trait.md](./07-substrate-trait.md) for the storage fabric
- [07b-bus-transport-fabric.md](./07b-bus-transport-fabric.md) for the transport fabric
- [08-scorer-gate-router-composer-policy.md](./08-scorer-gate-router-composer-policy.md) for `Datum`-aware operator signatures
- [tmp/refinements/07-naming.md](../../tmp/refinements/07-naming.md) for the canonical naming proposal
