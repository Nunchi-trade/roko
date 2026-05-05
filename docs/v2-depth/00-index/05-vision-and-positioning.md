# Vision and Positioning

> Depth for [EXECUTIVE-SUMMARY.md](../../docs/EXECUTIVE-SUMMARY.md), [VISION-RUN-ANYWHERE.md](../../docs/VISION-RUN-ANYWHERE.md), [COMPARISON.md](../../docs/COMPARISON.md), and [USE-CASES.md](../../docs/USE-CASES.md). Re-derives Roko's vision and competitive positioning through unified primitives. Why Roko is a protocol (not a framework), how it differs from LangGraph/CrewAI/AutoGen, and why persistent identity, self-trimming knowledge, and cryptographic provenance create defensible differentiation.

**Depends on**: [00-INDEX](../../unified/00-INDEX.md) (5 primitives, 9 protocols, 4 patterns), [01-SIGNAL](../../unified/01-SIGNAL.md) (demurrage, content addressing, HDC), [02-CELL](../../unified/02-CELL.md) (predict-publish-correct), [03-GRAPH](../../unified/03-GRAPH.md) (Graph composition, Rack)

---

## 1. What Roko Is

Roko is a **protocol for composable agent computation**. It defines how agents store knowledge, make decisions, verify outputs, and learn from outcomes -- using five primitives (Signal, Cell, Graph, Bus, Store) and nine behavioral protocols.

The design peers are not LangGraph or CrewAI. They are:

| Peer | What they standardized | What Roko standardizes |
|---|---|---|
| **ERC-20** | Token interfaces: any wallet holds any token | Agent interfaces: any Cell connects to any Graph |
| **Stripe** | Payment protocol: one integration, every payment method | Computation protocol: one Cell interface, every cognitive operation |
| **Ethereum** | Execution protocol: trustless, verifiable state transitions | Agent protocol: verifiable, lineage-tracked, self-trimming state transitions |

This distinction matters. A framework (LangGraph, CrewAI) provides an implementation. A protocol provides a **composition standard**. Cells from different authors compose because they speak the same nine protocols. Signals from different systems merge because they share content addressing. Graphs built independently interoperate because they are all Graphs of Cells.

---

## 2. The Three Technical Bottlenecks

Sequoia Capital's "The Agentic Web" (2026) identifies three bottlenecks preventing a functioning agent economy. Roko addresses each through its unified primitives:

### Bottleneck 1: Persistent Identity

Agents need stable identities that persist across sessions, hosts, and restarts. Without identity, there is no reputation, no accountability, no learning that compounds.

**Roko's answer**: ERC-8004 agent identities + HDC fingerprints.

Every Signal is content-addressed (SHA-256). Every agent has a persistent identity whose history is a DAG of Signals in Store. HDC fingerprints (10,240-bit hyperdimensional vectors) encode behavioral patterns -- an agent's "cognitive signature" that persists even when individual Signals decay. This is not just a user ID. It is a verifiable, machine-readable history of what an agent knows and how it behaves.

### Bottleneck 2: Agent Communication Protocol

Agents need to discover each other, negotiate capabilities, and delegate work without prior arrangement. Current approaches (fixed APIs, hardcoded integrations) do not scale.

**Roko's answer**: MCP (tool access) + A2A (agent cards) + Bus (ephemeral coordination) + stigmergic signaling.

The Bus provides ephemeral pub/sub for real-time coordination. MCP provides tool discovery (Linux Foundation standard). A2A provides agent-to-agent task delegation (Google standard). Stigmergic coordination (typed, decaying, scoped pheromones) enables emergent collective behavior without direct messaging.

### Bottleneck 3: Trust Without Face-to-Face

Agents need to evaluate each other's competence and honesty without human intermediation.

**Roko's answer**: ZK proofs over HDC vectors + TraceRank reputation + demurrage-weighted knowledge.

An agent can prove it has certain capabilities (a high gate-pass rate on Rust tasks) without revealing its full history. TraceRank computes reputation from the lineage graph (a verification verdict Signal that references the code Signal it verified creates a provable trust chain). Demurrage ensures stale reputation decays -- an agent that was competent a year ago must demonstrate current competence.

---

## 3. Why Protocol, Not Framework

### The Framework Trap

Frameworks (LangGraph, CrewAI, AutoGen) provide convenience at the cost of composability:

| Property | Framework approach | Protocol approach (Roko) |
|---|---|---|
| Adding a new model | Implement a provider adapter specific to the framework | Any Cell that conforms to the Connect protocol works |
| Adding a new verification step | Write a hook in the framework's callback system | Add a Verify Cell to the Graph. Done. |
| Combining two agents | Framework-specific delegation API | Wire two Agent Spaces into a shared Graph |
| Sharing learned state | Export to framework-specific format | Signal merge -- content-addressed, CRDT-compatible |
| Inspecting decisions | Framework-specific debug tools | Walk Signal lineage in Store (universal) |

### ERC-20 Composability

The analogy is precise. Before ERC-20, every token had a custom interface. Wallets needed per-token adapters. Exchanges needed per-token integrations. After ERC-20, any compliant token works with any compliant wallet.

Before Roko's protocol, every agent system has custom interfaces. Tools, verification, routing, and learning are framework-specific. After Roko's protocol:

- Any Cell conforming to Verify can validate any Signal.
- Any Cell conforming to Route can select among any candidates.
- Any Cell conforming to Compose can assemble context from any Store.
- Any Graph of Cells produces and consumes the same Signal type.

This is what enables a future where third-party Cells (custom verifiers, specialized routers, domain-specific composers) plug into any Roko Graph without modification.

---

## 4. Four Defensible Differentiators

### 4.1 Self-Trimming Knowledge (Demurrage)

Every Signal has a balance that decays over time (Gesell 1916). Knowledge that is not accessed, cited, or validated loses value and eventually gets pruned. This is not garbage collection -- it is **epistemic hygiene**. The system actively forgets what is probably wrong.

Why this matters competitively: every other agent system accumulates knowledge monotonically. Over time, stale information poisons decisions. RAG systems retrieve outdated context. Vector stores grow without bound. Roko is the only system where knowledge quality improves with time because bad knowledge self-destructs.

### 4.2 Persistent Identity (ERC-8004 + HDC)

An agent's identity is not a database row. It is the complete lineage DAG of its Signals in Store, fingerprinted by a 10,240-bit HDC vector that encodes behavioral patterns.

Why this matters competitively: identity enables reputation, which enables trust, which enables an agent economy. Without persistent identity, agents are anonymous -- there is no basis for evaluating their work. With persistent identity, agents develop track records (gate pass rates, cost efficiency, domain expertise) that are verifiable by any third party.

### 4.3 Cryptographic Provenance (Content Addressing + ZK)

Every Signal is content-addressed (SHA-256). Every Signal carries lineage (parent hashes). This creates an immutable audit chain: you can trace any decision back through every Signal that influenced it.

ZK proofs extend this to privacy-preserving attestation. An agent can prove "I have a 92% gate pass rate on Rust implementation tasks over 500+ observations" without revealing the tasks, the code, or the client.

Why this matters competitively: in regulated industries and high-stakes domains, provenance is not optional. Roko provides it structurally (every Signal has it), not as an add-on logging system.

### 4.4 Self-Development Loop

Roko develops itself. The workflow is concrete and operational:

```
Signal(Kind::Idea) -> Signal(Kind::PRD) -> Signal(Kind::Plan) -> Graph execution
  -> Signal(Kind::Code) -> Verify Cell -> Signal(Kind::Verdict) -> Store
```

Each step produces Signals that enter Store. Each Signal is subject to all nine protocols. The system that builds Roko IS Roko. Improvements to the protocol improve the system that extends the protocol.

Why this matters competitively: this is a compound improvement loop. A 10% improvement to the Verify protocol makes all future code 10% more reliable, which makes the next Verify improvement 10% more likely to succeed. No other system has this autocatalytic property.

---

## 5. Use Cases as Graph Templates

Every use case is a **Graph** -- a typed DAG of Cells connected by edges. Use cases differ only in which Cells are instantiated and how the Graph is wired. The runtime is the same.

### Single-Shot Task Execution

```
[Trigger: CLI input]
  -> [Compose Cell: assemble context]
    -> [Route Cell: select model]
      -> [Connect Cell: LLM call]
        -> [Verify Cell: gate pipeline]
          -> [Store Cell: persist result]
```

One Graph. Six Cells. The output is a verified Signal in Store.

### PRD-to-Execution Pipeline

```
[Trigger: PRD published]
  -> [Compose Cell: research + PRD context]
    -> [Connect Cell: plan generation LLM call]
      -> [Store Cell: tasks.toml as Signal]
        -> [Graph: parallel task executor]
          -> per task: [Route -> Connect -> Verify -> Store]
            -> [React Cell: update learning state]
```

Same primitives. Different topology. The PRD is a Signal. The plan is a Signal. Each task result is a Signal. All are content-addressed, lineage-tracked, and subject to demurrage.

### Research Synthesis

```
[Trigger: research command]
  -> [Connect Cell: Perplexity/web search]
    -> [Compose Cell: synthesize results]
      -> [Verify Cell: citation check]
        -> [Store Cell: research report as Signal]
```

### Autonomous Daemon

```
[Trigger Cell: cron/webhook/file-watch]
  -> [Route Cell: select appropriate Graph template]
    -> [Graph: selected workflow (task execution, PR review, etc.)]
      -> [React Cell: publish results, update metrics]
```

The point: there is no special-case code for different use cases. Each is a Graph of the same Cells wired differently. Adding a new use case means defining a new Graph topology in TOML, not writing new code.

---

## 6. The Five-Shape Deployment Model

"Run anywhere" means one binary, config-selected deployment shape. The unified primitives are identical across all shapes -- what changes is the Store backend, Bus backend, and export sinks.

### The Five Shapes

| Shape | Store | Bus | Agent dispatch | Gate execution | Typical user |
|---|---|---|---|---|---|
| **Laptop** | SQLite (local file) | in-memory ring | Subprocess (Claude CLI) | Local (cargo, npm, go) | Solo developer |
| **Single-server** | SQLite (persistent) | in-memory ring | Subprocess or API | Local | Self-hosted team |
| **Container** | Postgres / SQLite | NATS or in-memory | API (Anthropic, OpenAI) | Local or delegated | Kubernetes deployment |
| **Clustered** | Postgres (shared) | NATS (distributed) | API (load-balanced) | Distributed (A2A) | Production multi-node |
| **Edge** | in-memory (volatile) | none | API (forwarded to core) | Remote (delegated) | CDN worker, IoT |

### Same Graph, Different Backends

```toml
# Laptop shape
profile = "laptop"
# [No other config needed -- defaults handle everything]

# Clustered shape -- same binary, different config
profile = "clustered"

[profile.clustered]
substrate = { kind = "postgres", url = "${DATABASE_URL}" }
bus = { kind = "nats", url = "${NATS_URL}" }
auth = "oidc"
```

The Cells do not know which shape they run in. A Verify Cell checks code correctness identically on a laptop and in a Kubernetes pod. A Route Cell selects models identically on a single server and in a clustered deployment. The shape is invisible to the computation.

### Binary Size by Shape

| Shape | Features included | Approximate size |
|---|---|---|
| Full (laptop/server) | All crates, TUI, gate runners | ~15 MB (native) |
| Container | No TUI, API-only dispatch | ~10 MB (native) |
| Edge/WASM | Core only (routing + scoring + HDC) | ~500 KB (gzipped WASM) |

---

## 7. Competitive Comparison by Capability Axis

| Axis | **Roko** | **LangGraph** | **CrewAI** | **AutoGen** | **SWE-Agent** |
|---|---|---|---|---|---|
| **Core abstraction** | Protocol (Signal + Cell + Graph) | State machine (nodes + edges) | Crew (agents + tasks) | Conversation (agents + messages) | Single agent + ACI |
| **Composability** | Any Cell + any Graph (ERC-20 model) | Within LangGraph only | Within CrewAI only | Within AutoGen only | None |
| **Knowledge persistence** | Store with demurrage, HDC, 6 types x 4 tiers | External vector store | None | None | None |
| **Temporal dynamics** | 4 decay models, knowledge half-lives | None | None | None | None |
| **Verification** | 9 protocols, Verify Cell, 7-rung pipeline | Optional callbacks | None | None | SWE-bench eval |
| **Learning** | predict-publish-correct (structural), 10+ subsystems | None | None | None | None |
| **Self-improvement** | Autocatalytic loop (PRD -> plan -> execute -> gate -> learn) | None | None | None | None |
| **Deployment shapes** | 5 shapes, same binary | Cloud-hosted | Python process | Python process | Python process |
| **Safety** | Capability tokens, taint tracking, temporal monitors | None built-in | None built-in | None built-in | Container sandbox |
| **Provenance** | Content-addressed lineage DAG on every datum | None | None | None | None |
| **Affect model** | PAD vectors, somatic markers, 6 states | None | None | None | None |
| **Offline learning** | Dream cycle (NREM/REM/integration) | None | None | None | None |
| **Language** | Rust (performance, safety guarantees) | Python | Python | Python | Python |
| **Multi-target** | Native + WASM from same source | Python only | Python only | Python only | Python only |

### Where Alternatives Win

| Alternative | Where it is better | Why |
|---|---|---|
| LangGraph | Python ecosystem integration, rapid prototyping | If you need Python libraries and do not need verification |
| CrewAI | Simple role-based delegation for demos | If "team of agents" is the UX you want |
| AutoGen | Multi-turn conversation orchestration | If your use case is conversational, not computational |
| SWE-Agent | Benchmark performance (SWE-bench focus) | If you are optimizing a single metric on a benchmark |

### Where Roko Wins

| Requirement | Why Roko |
|---|---|
| Agents that get better over time | predict-publish-correct + demurrage + dream consolidation |
| Cost optimization at volume | CascadeRouter + adaptive thresholds + A/B experiments |
| Audit trail for every decision | Content-addressed lineage (structural, not optional) |
| Production deployment beyond a laptop | 5 deployment shapes, same binary, config-selected |
| Verification of every output | 7-rung pipeline, adaptive EMA thresholds, gate-as-cognition |
| Self-developing systems | PRD -> plan -> execute -> gate -> learn (operational today) |
| Long-running reliability | Demurrage prevents knowledge rot; Conductor prevents stuck agents |

---

## 8. The Network Effect Thesis

A single Roko agent is useful. N Roko agents sharing learned state are qualitatively different.

### How It Works

Every instance produces learning data:
- Routing observations: which model succeeds on which task type
- Gate thresholds: pass rates per verification rung
- Skill discoveries: successful tool-use patterns
- Cost data: actual spend per model per provider

With Merkle-CRDT sync (content-addressed + conflict-free), instances merge this data without coordination. A new user's agent starts with the collective experience of all existing users.

### Why Unified Primitives Enable This

The merge works because Signals are content-addressed. Two instances that independently discover the same skill produce the same Signal hash -- automatic deduplication. Two instances that observe different routing outcomes produce Signals that merge via G-Counter CRDT -- observations sum across instances.

This is impossible in a framework that uses opaque internal state. It is natural in a protocol where every datum is a content-addressed Signal with typed merge semantics.

---

## What This Enables

1. **Protocol-level interoperability**: Third-party Cells compose with first-party Cells by construction. The standard is the interface, not the implementation.
2. **Defensible differentiation**: Demurrage, HDC fingerprints, content-addressed lineage, and the self-development loop are not features -- they are structural properties that competitors cannot bolt on.
3. **Network effects from day one**: Merkle-CRDT sync means every new user benefits from every existing user's learning. This compounds.
4. **Use-case extensibility without code**: New workflows are new Graph topologies, not new code. TOML-defined, discoverable, composable.
5. **Deployment flexibility**: Same binary runs on a developer laptop and in a Kubernetes cluster. The protocol does not care about infrastructure.

## Feedback Loops

- **Self-development loop**: Roko improves itself -> better Cells -> better verification -> higher confidence in self-development -> more aggressive self-development.
- **Network learning loop**: More users -> more routing observations -> better model selection for everyone -> lower cost -> more users.
- **Knowledge decay loop**: Bad knowledge decays -> retrieval quality improves -> agent outputs improve -> more knowledge passes Verify -> Store quality improves.
- **Reputation loop**: Agent performs well -> reputation increases -> more task delegation -> more data -> better learning -> performs better.

## Open Questions

1. **Protocol governance**: If Roko is a protocol, who decides when the protocol evolves? ERC-20 has the EIP process. What is Roko's equivalent?
2. **Ecosystem bootstrap**: A protocol is only valuable with multiple implementations. How does Roko reach critical mass? Self-development is the initial use case, but what drives adoption beyond the creator?
3. **Trust transitivity**: If Agent A trusts Agent B, and Agent B trusts Agent C, does Agent A trust Agent C? The lineage DAG can verify direct relationships. Transitive trust requires additional machinery (web-of-trust or delegation chains).
4. **Cross-protocol Signal interop**: Roko Signals are SHA-256 content-addressed. If another system uses BLAKE3 or Keccak-256, can Signals interoperate? Content addressing is protocol-specific unless a canonicalization standard exists.
5. **Economic sustainability**: The brain/skill marketplace is the obvious monetization path. But marketplace economics require critical mass. What is the minimum viable network for the marketplace to function?

## Implementation Tasks

| Task | Where | Effort | Priority |
|---|---|---|---|
| Validate `roko-core` compiles to `wasm32-unknown-unknown` | `roko-core` | S | High |
| Implement platform abstraction traits (StateStore, LlmClient, GateRunner) | `roko-core` | L | High |
| Define CRDT types for routing observations, gate thresholds, skills | `roko-learn` | L | Medium |
| Implement Merkle-CRDT sync protocol (reconciliation, merge) | new crate | XL | Medium |
| Implement brain export/import commands | `roko-cli` | M | Medium |
| Publish A2A Agent Card from `roko serve` | `roko-serve` | M | Medium |
| Build `roko-wasm` crate (browser target with IndexedDB Store) | new crate | XL | Low |
| Implement edge deployment profile (minimal WASM build) | build config | M | Low |
| Document the protocol specification as a standalone standard | docs | L | Low |
