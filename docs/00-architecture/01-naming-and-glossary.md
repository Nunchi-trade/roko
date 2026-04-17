# Naming Map and Glossary

> **Abstract:** This document is the authoritative naming map for Roko's kernel vocabulary.
> Use `Engram` for the durable record, `Pulse` for the ephemeral wire medium, `Substrate` for
> storage, `Bus` for transport, `Topic` for routing, `TopicFilter` for subscription matching,
> `Datum` for Engram-or-Pulse operator inputs, `PulseSource` for transport-time producer
> attribution, and `StateHub` for the kernel projection layer that bridges Bus + Substrate to
> consumer surfaces. For the heuristic, falsifier, and worldview vocabulary used in learning
> refinements, see also [tmp/refinements/14-worldview-validation.md](../../tmp/refinements/14-worldview-validation.md). For the paper,
> claim, and replication-ledger vocabulary used in research-to-runtime work, see also
> [tmp/refinements/16-research-to-runtime.md](../../tmp/refinements/16-research-to-runtime.md). For the plugin and SPI extension
> vocabulary used in the five-tier extension architecture, see also
> [tmp/refinements/17-plugin-extension-architecture.md](../../tmp/refinements/17-plugin-extension-architecture.md).
> For the Rust SDK vocabulary used by application authors, agent authors, trait implementors,
> and runtime implementors, see also [tmp/refinements/22-developer-ux-rust.md](../../tmp/refinements/22-developer-ux-rust.md)
> and [../12-interfaces/19-rust-sdk-developer-ux.md](../12-interfaces/19-rust-sdk-developer-ux.md).
> When another document
> disagrees, this glossary wins. See also
> [tmp/refinements/07-naming.md](../../tmp/refinements/07-naming.md),
> [tmp/refinements/11-hyperdimensional-substrate.md](../../tmp/refinements/11-hyperdimensional-substrate.md),
> [tmp/refinements/12-knowledge-demurrage.md](../../tmp/refinements/12-knowledge-demurrage.md),
> [tmp/refinements/14-worldview-validation.md](../../tmp/refinements/14-worldview-validation.md),
> [tmp/refinements/13-collective-intelligence-c-factor.md](../../tmp/refinements/13-collective-intelligence-c-factor.md),
> [tmp/refinements/10-self-learning-cybernetic-loops.md](../../tmp/refinements/10-self-learning-cybernetic-loops.md),
> [tmp/refinements/09-phase-2-implications.md](../../tmp/refinements/09-phase-2-implications.md),
> [tmp/refinements/20-modularity-composability.md](../../tmp/refinements/20-modularity-composability.md),
> [tmp/refinements/23-user-ux-running-agents.md](../../tmp/refinements/23-user-ux-running-agents.md),
> [tmp/refinements/24-deployment-ux.md](../../tmp/refinements/24-deployment-ux.md),
> [tmp/refinements/32-safety-sandbox-provenance.md](../../tmp/refinements/32-safety-sandbox-provenance.md),
> [tmp/refinements/26-statehub-rearchitecture.md](../../tmp/refinements/26-statehub-rearchitecture.md),
> [tmp/refinements/27-realtime-event-surface.md](../../tmp/refinements/27-realtime-event-surface.md),
> [tmp/refinements/28-cli-parity-familiar-workflows.md](../../tmp/refinements/28-cli-parity-familiar-workflows.md),
> [tmp/refinements/29-web-ui-architecture.md](../../tmp/refinements/29-web-ui-architecture.md),
> [tmp/refinements/30-rich-ux-primitives.md](../../tmp/refinements/30-rich-ux-primitives.md),
> [tmp/refinements/25-domain-specific-agents.md](../../tmp/refinements/25-domain-specific-agents.md),
> [../12-interfaces/22-statehub-projection-layer.md](../12-interfaces/22-statehub-projection-layer.md),
> [../12-interfaces/13-web-portal.md](../12-interfaces/13-web-portal.md),
> [../12-interfaces/23-rich-ux-primitives.md](../12-interfaces/23-rich-ux-primitives.md),
> [../12-interfaces/06-websocket-streaming.md](../12-interfaces/06-websocket-streaming.md),
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

The naming contract for the current workspace and the REF20 target dep graph is:

| Crate | Responsibility |
|---|---|
| `roko-core` | Core kernel vocabulary including `Engram`, `Pulse`, `Topic`, `TopicFilter`, `Datum`, `PulseSource`, `Substrate`, and `Bus` |
| `roko-bus` | Proposed kernel transport crate that extracts Bus traits, Topic routing helpers, and replay semantics out of `roko-runtime` so transport is a first-class dependency boundary |
| `roko-hdc` | Proposed kernel HDC crate that extracts vector operations, encoders, binding, bundling, and similarity out of `roko-primitives` so consumers depend on a minimal semantic-memory surface |
| `roko-spi` | Stable extension contracts: manifests, capabilities, permissions, and versioned plugin metadata |
| `roko-agent` | Agent runtime, model/tool execution, and live Pulse production |
| `roko-defaults` | Proposed split from `roko-std` that holds default operator implementations without pulling in every builtin tool |
| `roko-tools` | Proposed split from `roko-std` that holds builtin tools as a separately versioned implementation crate |
| `roko-compose-core` | Proposed split from `roko-compose` that holds prompt assembly, layering, and budgeting logic |
| `roko-templates` | Proposed split from `roko-compose` that holds role and domain templates as data-first assets above the compose engine |
| `roko-orchestrator` | Plan DAG execution, scheduling, and orchestration topics |
| `roko-neuro` | Durable knowledge management and distillation |
| `roko-daimon` | PAD-vector affect and behavioral modulation |
| `roko-dreams` | Delta-speed replay, synthesis, and consolidation |
| `roko-chain` | Durable chain integration plus chain-facing Bus backends |
| `roko-plugin` | Plugin discovery/loading surface, manifest ingestion, and legacy event-source framework |
| `roko-extension-abi` | Native ABI bridge for Tier 4 loadable extensions |
| `roko-wasm-host` | WASM host boundary for Tier 5 sandboxed extensions and capability-limited Bus/Substrate access |

User-facing docs should describe those crates in current vocabulary rather than older umbrella
names. When a concept spans multiple crates, describe the concept first and the crate boundary
second.

When a crate name is part of the target dep graph rather than the current workspace, label it as
proposed or target-state rather than implying it already ships. REF20 uses that distinction to
keep the docs aligned with repository reality while still documenting the intended landing zone.

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
| `StateHub` | Kernel projection layer | Hydrates named projections from Substrate, folds Bus deltas, and serves typed views to consumers. |
| `Topic` | Routing handle | Dot-separated lowercase identifier such as `gate.verdict.emitted`. |
| `TopicFilter` | Subscription and replay selector | Declarative matcher for Bus consumers. |
| `Datum<'a>` | Either-medium operator input | `enum Datum<'a> { Engram(&'a Engram), Pulse(&'a Pulse) }`. |
| `PulseSource` | Transport-time producer attribution | Lightweight source identifier carried on a Pulse before graduation. |
| `BusReceiver` | Subscriber handle | Delivers matching Pulses in publish order with bounded replay state. |
| `u64` sequence id | Bus ordering primitive | The default sequence identifier for Pulse ordering. |
| `Heuristic` | Learned rule of thumb | Structured precondition-plus-prediction knowledge with calibration from lived episodes. |
| `Falsifier` | Counterexample or test signal | A deliberate challenge that can violate a heuristic and force recalibration. |
| `Worldview` | Heuristic cluster | A co-citation cluster of heuristics that holds up in a shared domain. |
| `Plugin` | Loadable extension package | A discoverable extension bundle described by a manifest and loaded at a specific tier. |
| `SPI` | Stable plugin interface | The shared contract layer for plugin discovery, capabilities, permissions, and versioning. |
| `Manifest` | Extension descriptor | Declarative metadata that identifies a plugin, its tier, and its permissions. |
| `dep graph` | Planned crate dependency graph | The target import topology that keeps kernel crates narrow, implementation crates swappable, and higher layers decoupled through traits and fabrics. |
| `Surface` | User interaction rendering | One of CLI, TUI, Chat, or Web presenting the same verbs over shared state. |
| `Verb set` | Cross-surface action vocabulary | The shared `ask`, `plan`, `do`, `watch`, `inspect`, `replay`, `learn`, `tune`, and `connect` contract. |
| `Session` | Cross-surface continuity artifact | Named, replayable unit of user-visible work spanning prompts, checkpoints, and progress streams. |
| `Domain profile` | Installable domain bundle | A tier-2 plugin bundle that packages domain-specific tools, roles, gates, heuristics, and templates for coding, research, blockchain, data/ML, ops, writing, or third-party domains. |
| `Profile bundle` | Versioned install unit for a domain profile | The discoverable package installed through `roko plugin install @roko/<name>-profile`. |
| `Profile` | Deployment-shape config bundle | A named default set for laptop, single-server, container, clustered, or edge runtime behavior. Use `domain profile` when the bundle customizes agent behavior rather than deployment shape. |
| `State portability` | Signed import/export workflow | The rule that `Substrate` state, queue state, and deployment config can move between shapes as one auditable archive. |
| `SecretStore` | Secret backend trait | Layered secret resolver behind OS keychain, vault-style stores, and role-scoped credential injection. |
| `TenantCtx` | Tenant-scoped request context | Identity, role, quota, and audit information propagated through auth, `Substrate`, and control-plane spans. |
| `TypedContext` | Structured situation payload | Domain-tagged key/value context passed to composers, gates, and heuristics so matching happens on typed fields instead of free-text parsing. |
| `Custody` | Chain-of-custody record | Auditable record of who approved an action, which heuristics and claims influenced it, what simulation ran, and what witness or result was observed. |
| `Attestation` | Cryptographic commitment on an Engram | Signature over a durable ContentHash plus timestamp and attestation level for later verification. |
| `AttestationLevel` | Audit commitment tier | `LocalAgent`, `OrgRole`, or `ChainWitness`, chosen by kind and action criticality. |
| `Taint` | Untrusted-input marker | One-way safety label such as `UserInput`, `ExternalFetch`, `ThirdPartyPlugin`, or `LegacyImport` that propagates until explicit human sign-off. |
| `Projection` | Named live-updating view | A typed `State` plus `Delta` fold over Bus + Substrate that consumers query and subscribe to through StateHub. |
| `Telemetry surface` | Operator-facing observability contract | The combined logs, metrics, traces, event streams, and replay affordances that make a run inspectable without changing kernel behavior. |
| `Cursor` | Realtime resume token | Opaque position marker carried on projection and stream replies so clients can resume after reconnect. |
| `Realtime surface` | External streaming contract | Shared `query`, `subscribe`, and `publish` vocabulary carried over WebSocket, SSE, or optional gRPC. |
| `Cost meter` | Spend projection and dashboard feed | Named projection and CLI/UI surface exposing per-session, per-task, per-role, and per-model spend plus remaining budget. |
| `Slash command` | Interactive shell shortcut | Familiar `/<verb>` entry such as `/edit` or `/watch` that maps onto the same canonical CLI verbs. |
| `Diff-first review` | Proposed-edit presentation style | Show hunks before apply, preserve per-hunk accept/reject/edit decisions in the transcript, and expose `explain` on demand. |
| `Transcript` | Session interaction log | Human-readable or structured record of prompts, outputs, approvals, budgets, and replay metadata. |
| `Budget line` | Visible cost state | Prompt or status-surface rendering of current turn or session spend versus configured limit. |
| `Reasoning stream` | Live thought-sidecar rendering | Collapsible stream of `agent.reasoning` Pulses that keeps long-running work legible without replacing the main answer. |
| `Tool-call banner` | Action-status affordance | Compact rendering of one tool invocation with status, output drill-down, rerun, and explanation hooks. |
| `Gate badge` | Verification status chip | Clickable or focusable pass/warn/fail indicator backed by gate evidence rather than decorative status. |
| `Heuristic footnote` | Inline heuristic citation | Numbered annotation that reveals which heuristic shaped a response, plus calibration and provenance. |
| `Uncertainty bar` | Confidence rendering | Visual or textual confidence indicator attached to a decision, often used to trigger explicit approval. |
| `Replay scrubber` | Episode timeline control | Time-based affordance that rewinds transcript, diff, and projection views together over a recorded episode. |
| `Progressive disclosure` | Layered reveal pattern | Summary-first presentation that expands to reasoning, heuristics, trace, or cost only when requested. |
| `Spatial memory` | Layout consistency discipline | Stable panel placement, navigation paths, and shortcut meaning that lets operators build muscle memory. |

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
| `Roko Chat` | Conversational surface over the shared session and Bus stream |
| `Roko Portal` | Stable chapter and product name for the first-party web UI over StateHub, the realtime surface, and the control plane |
| `Web UI` | Browser rendering of the shared verb set, sessions, and projection state; the current first-party scope is `Home`, `Chat`, `Plans`, `Beliefs`, and `Settings` |
| `HTTP API` | Programmatic control plane |
| `Realtime surface` | Shared transport contract for live Pulse delivery to clients and observers |

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
| `HDC fingerprint` | Deterministic 10,240-bit `HdcVector` carried on each Engram for native similarity, clustering, consensus, and analogy. |
| `roko-bus` | Proposed kernel crate for the transport fabric. |
| `roko-hdc` | Proposed kernel crate for hyperdimensional operations and similarity. |
| `roko-defaults` | Proposed crate for default operator implementations split out of `roko-std`. |
| `roko-tools` | Proposed crate for builtin tools split out of `roko-std`. |
| `roko-compose-core` | Proposed crate for compose engine mechanics split out of `roko-compose`. |
| `roko-templates` | Proposed crate for role and domain template packs split out of `roko-compose`. |
| `SDK` | Four-layer Rust developer surface spanning one-liner, builder, trait impl, and runtime impl entry points over the same kernel. |
| `Domain profile` | Installable bundle for one work domain such as coding, research, blockchain, data/ML, ops, or writing. |
| `TypedContext` | Structured `domain + fields` shape that lets gates, heuristics, and templates match on typed context rather than parsing summaries. |
| `Custody` | Shared audit record for consequential actions across domains, including approvals, simulations, results, and optional chain witness material. |
| `Attestation` | Cryptographic signature record attached to selected Engrams so auditors can verify who committed to a durable record and at what level. |
| `AttestationLevel` | Commitment tier used by attested Engrams: `LocalAgent`, `OrgRole`, or `ChainWitness`. |
| `Taint` | Durable safety label recording whether an Engram originated from user input, external fetches, third-party plugins, or legacy imports. |
| `one-liner` | Fastest Rust entry point, typically `roko::run(...)`, for demos, scripts, and first-run success. |
| `builder` | Daily-driver authoring surface that validates at `.build()` and hides kernel details by default. |
| `trait impl` | Custom component surface for `Substrate`, `Bus`, operators, translators, and provider adapters. |
| `runtime impl` | Host integration surface for browser, edge, embedded, or distributed runtimes. |
| `cargo roko` | Cargo-native developer workflow for scaffold, replay, explain, benchmark, and heuristic inspection. |
| `Paper` | Durable research source Engram that seeds claims, heuristics, and replication tracking. |
| `Claim` | Testable hypothesis distilled from a Paper, with context, falsifier, and calibration. |
| `Replication Ledger` | Per-claim record of paper effect, observed effect, trial count, divergence, and replication status. |
| `Demurrage` | The durable-memory holding cost that continuously taxes idle Engram balance while reinforcement refunds useful retrieval, citation, gate survival, and surprise. |
| `Balance` | The attention-credit carried by an Engram under demurrage; Scorer and Composer read effective weight from balance rather than a standalone freshness field. |
| `Cold tier` | A colder Substrate tier that keeps content-addressability and lineage intact after an Engram's balance reaches the configured floor. |
| `Heuristic` | A first-class learned claim with explicit preconditions, a prediction, and calibration updates from actual outcomes. |
| `Falsifier` | An observed challenge or counterexample that can refute, refine, or narrow a heuristic. |
| `Worldview` | A domain-shaped cluster of heuristics that co-occur and reinforce one another under lived experience. |
| `MeshBus` | Bus backend for collective pub/sub topics such as `mesh.pheromone.deposited`. |
| `MeshSubstrate` | Shared durable Engram backend for mesh replication, collective knowledge, and pheromone deposits. |
| `HeartbeatPolicy` | Runtime policy that publishes `heartbeat.gamma.tick`, `heartbeat.theta.tick`, and `heartbeat.delta.tick` Pulses. |
| `Extension tier` | One of the five extension loading classes: prompts, profiles, declarative tools/MCP, native Rust, or WASM sandboxing. |
| `c-factor` | Continuously measured cohort-process metric learned from turn-taking entropy, peer prediction accuracy, citation reciprocity, delivery rate, and HDC diversity. |
| `Compounding loop` | A feedback loop where each successful cycle improves the next cycle's cost, latency, or quality rather than merely repeating it. |
| `Superlinear return` | A scaling regime where accumulated usage, connected deployments, or added plugins produce more than proportional capability or efficiency gains. |
| `Heuristic commons` | The cross-deployment pool of exportable heuristics and calibration data that bootstraps new Roko installations. |
| `Synapse Architecture` | The architecture story of two mediums, two fabrics, and six operators. |
| `Surface` | One of the four user renderings: CLI, TUI, Chat, or Web. |
| `Verb set` | The unified cross-surface action vocabulary: `ask`, `plan`, `do`, `watch`, `inspect`, `replay`, `learn`, `tune`, `connect`. |
| `Session` | The user-facing continuity object whose state can be resumed, watched, exported, shared, and replayed across surfaces. |
| `Profile` | Deployment-shape preset that bundles backend, auth, observability, and storage defaults for laptop, single-server, container, clustered, or edge operation. |
| `State portability` | Export/import contract for moving `Substrate` state, queue state, and config between deployment shapes as a signed archive. |
| `SecretStore` | Swappable secret-resolution backend behind env, config, OS keychain, and external secret managers. |
| `TenantCtx` | Per-request tenant and role envelope used for scoped storage, budget enforcement, and audit labeling. |
| `Slash command` | Familiar interactive `/<verb>` form that maps to the same CLI action vocabulary rather than creating a second interface model. |
| `Diff-first review` | The rule that code changes are shown as hunks before apply, with per-hunk approval and optional explainability. |
| `Transcript` | Durable session log carrying prompts, outputs, approvals, budget state, and replay metadata across CLI, TUI, Chat, and Web. |
| `Budget line` | The visible spend summary shown during interactive and status flows so routing and approvals remain legible to the operator. |
| `Safety spine` | The orthogonal enforcement story joining authorization, sandboxing, taint, attestation, custody, and audit tooling across all layers. |
| `Component library` | Shared browser widgets and review primitives, such as `@roko/ui`, used by the first-party web UI and extension pages. |

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
| `prediction.*` | Operator predictions published before downstream reality resolves them |
| `outcome.*` | Reality-side or verification-side Pulses that close a prediction loop |
| `prediction.error.*` | Joined residuals, drift, and high-surprise signals derived from prediction/outcome pairs |
| `calibration.*` | Operator calibration updates emitted by learning policies |
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
| `Demurrage` | Economic memory rule that charges idle Engrams a holding cost and rewards useful durable knowledge with reinforcement bonuses. |
| `Dreams` | Delta-speed consolidation cross-cut that writes durable results back to storage. |
| `Engram` | Durable cognitive record stored in a Substrate and identified by content hash. |
| `Balance` | Per-Engram attention credit under demurrage; when it falls to the floor, the Engram becomes a cold-tier candidate. |
| `c-factor` | Learned scalar summarizing collective process quality for a cohort; computed from Bus plus Substrate statistics and used as a diagnostic covariate rather than a standalone objective. |
| `Paper` | Research Engram that carries the cited source and its durable metadata. |
| `Claim` | Structured, testable restatement of a Paper's result with a falsifier and calibration state. |
| `Replication Ledger` | Living record that compares a paper's reported effect against observed results in this deployment. |
| `Fleet` | Roster of agents under shared coordination or ownership. |
| `HDC fingerprint` | Per-Engram 10,240-bit hyperdimensional vector used for `query_similar`, clustering, consensus, and analogy. |
| `Mesh` | Agent-network layer for multi-agent communication. |
| `Neuro` | Durable knowledge cross-cut that influences storage reads and composition. |
| `SDK` | The four-layer Rust developer surface that lets Rust users start with a one-liner and descend only as far as builder, trait impl, or runtime impl work requires. |
| `Pulse` | Ephemeral transport record published on a Bus and retained only as long as the stream requires. |
| `PulseSource` | Lightweight producer identity carried on a Pulse. |
| `Taint` | One-way safety classification on an Engram indicating untrusted provenance and the need for additional checks before action. |
| `Custody` | Durable chain-of-custody Engram for auditable actions, including authorization evidence, cited heuristics and claims, gate verdicts, and optional witness data. |
| `Attestation` | Signature metadata proving that a principal committed to a specific durable ContentHash at a given level. |
| `AttestationLevel` | Verification tier for an attested Engram: local session, human role, or chain witness. |
| `StateHub` | Kernel projection layer that turns Bus pulses and Substrate history into named consumer-facing views. |
| `Telemetry surface` | Combined logs, metrics, traces, projection streams, and replay tools that let operators inspect a run and the observability stack itself. |
| `Cursor` | Opaque resume token carried by the realtime surface so clients can continue from a known stream position. |
| `Cost meter` | Projection-backed spend view used by CLI, TUI, and web surfaces for budget-versus-burn visibility. |
| `Slash command` | Interactive `/<verb>` alias such as `/edit`, `/run`, or `/watch` that resolves to the same underlying Roko actions. |
| `Diff-first review` | Operator-facing review mode where code changes appear as hunks before apply and can be accepted, rejected, or edited individually. |
| `Transcript` | Persisted session log that captures prompts, outputs, approvals, budgets, and replay metadata for later resume or audit. |
| `Budget line` | Prompt or status readout showing current spend against configured limits for the turn or session. |
| `Prediction Error` | The residual between predicted and observed outcomes, published as `prediction.error.*` when it becomes a first-class runtime signal. |
| `Profile` | Deployment-shape configuration preset selected by name rather than by building a different binary. |
| `SecretStore` | Secret backend abstraction that keeps layered credential resolution off the main config path. |
| `Substrate` | Storage fabric for Engrams and durable retrieval. |
| `Synapse Architecture` | The kernel framing of two mediums, two fabrics, six operators, five layers, three speeds, and three cross-cuts. |
| `State portability` | Ability to export and import durable state, queue state, and config between laptop, single-server, container, clustered, and edge deployments. |
| `TenantCtx` | Auth-derived tenant context used by multi-tenant control planes to scope storage, quotas, and telemetry. |
| `Topic` | Routing handle for Pulses on the Bus. |
| `TopicFilter` | Declarative matcher for Topic-based subscription and replay. |

---

## See Also

- [02-engram-data-type.md](./02-engram-data-type.md) for the durable record medium
- [06-synapse-traits.md](./06-synapse-traits.md) for operator boundaries across the two mediums
- [07-substrate-trait.md](./07-substrate-trait.md) for the storage fabric
- [07b-bus-transport-fabric.md](./07b-bus-transport-fabric.md) for the transport fabric
- [08-scorer-gate-router-composer-policy.md](./08-scorer-gate-router-composer-policy.md) for `Datum`-aware operator signatures
- [../12-interfaces/19-rust-sdk-developer-ux.md](../12-interfaces/19-rust-sdk-developer-ux.md) for the four-layer Rust SDK vocabulary and developer-facing entry points
- [tmp/refinements/14-worldview-validation.md](../../tmp/refinements/14-worldview-validation.md) for the heuristic, falsifier, and worldview refinement
- [tmp/refinements/16-research-to-runtime.md](../../tmp/refinements/16-research-to-runtime.md) for the paper, claim, and replication ledger pipeline
- [tmp/refinements/07-naming.md](../../tmp/refinements/07-naming.md) for the canonical naming proposal
- [tmp/refinements/11-hyperdimensional-substrate.md](../../tmp/refinements/11-hyperdimensional-substrate.md) for the fingerprint-first HDC proposal
- [tmp/refinements/33-observability-telemetry.md](../../tmp/refinements/33-observability-telemetry.md) for the consolidated observability and telemetry contract
- [../12-interfaces/21-user-ux-running-agents.md](../12-interfaces/21-user-ux-running-agents.md) for the four surfaces and unified verb set
- [tmp/refinements/23-user-ux-running-agents.md](../../tmp/refinements/23-user-ux-running-agents.md) for the user-UX refinement proposal
- [tmp/refinements/22-developer-ux-rust.md](../../tmp/refinements/22-developer-ux-rust.md) for the Rust SDK framing and developer-UX proposal
- [tmp/refinements/28-cli-parity-familiar-workflows.md](../../tmp/refinements/28-cli-parity-familiar-workflows.md) for slash-command, diff-first, and transcript terminology in the CLI parity proposal
- [../19-deployment/INDEX.md](../19-deployment/INDEX.md) for deployment profiles, five shapes, and state portability
- [../19-deployment/10-secret-management.md](../19-deployment/10-secret-management.md) for layered secret resolution and shared-server credential handling
- [tmp/refinements/24-deployment-ux.md](../../tmp/refinements/24-deployment-ux.md) for the deployment-UX refinement proposal
- [tmp/refinements/32-safety-sandbox-provenance.md](../../tmp/refinements/32-safety-sandbox-provenance.md) for the shared vocabulary around custody, sandboxing, taint, attestation, and the safety spine
- [tmp/refinements/26-statehub-rearchitecture.md](../../tmp/refinements/26-statehub-rearchitecture.md) for the StateHub projection-layer proposal
- [tmp/refinements/27-realtime-event-surface.md](../../tmp/refinements/27-realtime-event-surface.md) for the shared realtime transport proposal
- [../12-interfaces/06-websocket-streaming.md](../12-interfaces/06-websocket-streaming.md) for the interface-facing realtime surface contract
- [../12-interfaces/22-statehub-projection-layer.md](../12-interfaces/22-statehub-projection-layer.md) for the interface-facing projection-layer contract
- [../12-interfaces/23-rich-ux-primitives.md](../12-interfaces/23-rich-ux-primitives.md) for reasoning streams, heuristic footnotes, uncertainty bars, replay scrubbers, and the shared interface primitive vocabulary
