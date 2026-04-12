# Crate Map

> **Abstract:** Roko is implemented as an 18+ crate Rust workspace. Each crate maps to one
> or more architectural layers, follows strict downward-only dependency rules, and implements
> one or more of the six Synapse traits. This document provides the complete crate map, layer
> assignments, dependency relationships, test coverage, and the dissolution of the legacy
> `roko-golem` umbrella crate into standalone components.


> **Implementation**: Shipping

**Topic**: [00-architecture](./INDEX.md)
**Prerequisites**: [12-five-layer-taxonomy](./12-five-layer-taxonomy.md), [06-synapse-traits](./06-synapse-traits.md)
**Key sources**:
- `/Users/will/dev/nunchi/roko/refactoring-prd/00-overview.md` — Crate table
- `/Users/will/dev/nunchi/roko/roko/crates/roko-core/src/lib.rs` — Kernel structure
- `/Users/will/dev/nunchi/roko/roko/tmp/prd-migration/context-pack/00-ALWAYS-READ-FIRST.md` — Full crate listing
- `/Users/will/dev/nunchi/roko/roko/tmp/prd-migration/context-pack/01-naming-map.md` — roko-golem dissolution

---

## Abstract

Roko's 18+ crate structure embodies the principle of modular composition: each crate has a
clear responsibility, a well-defined position in the five-layer taxonomy, and strict
dependency constraints. No crate at layer N may depend on a crate at layer N+1 or above.
Cross-cutting concerns (Neuro, Daimon, Dreams) are injected via trait objects rather than
direct imports, preserving the layering invariant.

The workspace has evolved through several naming transitions: from `bardo-*` and `golem-*`
prefixes to the unified `roko-*` namespace. The most significant structural change was the
dissolution of `roko-golem`, an umbrella crate that aggregated cognitive subsystems behind a
`ScaffoldEngine` trait. In the Synapse Architecture, composition happens at the application
layer through config-driven assembly — no umbrella crate is needed because each subsystem
defines its own Synapse trait implementations that compose through the universal Engram type.

This document provides the comprehensive crate map, including each crate's purpose, status,
test coverage, primary Synapse traits implemented, and inter-crate dependencies.

---

## 1. Layer-by-Layer Crate Map

### 1.1 Layer 0: Runtime

Runtime crates provide the foundational infrastructure: event streaming, process supervision,
cancellation, the adaptive clock, and shared primitive types including HDC vectors.

| Crate | Status | Tests | Purpose | Primary Traits |
|---|---|---|---|---|
| `roko-primitives` | Built | — | HDC vectors (10,240-bit), Hamming similarity, tiering, shared types | (Provides types used by Substrate) |
| `roko-runtime` | Built | — | Event bus, `ProcessSupervisor`, cancellation tokens, adaptive clock (Gamma/Theta/Delta) | (Infrastructure for Substrate) |

**`roko-primitives`** provides the Hyperdimensional Computing foundation: 10,240-bit binary
vectors with Bind (XOR), Bundle (majority), and Similarity (Hamming distance) operations.
These vectors enable O(1) similarity search across the knowledge base (Kanerva 2009). The
crate also provides HDC-based fingerprinting for code symbols and documents.

**`roko-runtime`** provides the process lifecycle management layer. `ProcessSupervisor`
tracks spawned agent processes, handles graceful shutdown, and implements the adaptive clock
that manages the three cognitive speeds (Gamma at ~5-15s, Theta at ~75s, Delta at hours).
The event bus enables publish-subscribe communication within a single agent process.

### 1.2 Kernel

The kernel crate is the architectural foundation — every other crate in the workspace
depends on it.

| Crate | Status | Tests | Purpose | Primary Traits |
|---|---|---|---|---|
| `roko-core` | Built | 376 | Engram type (currently `Signal`), 6 Synapse trait definitions, Score, Decay, Kind, Body, ContentHash, Provenance, Query, Budget, Context, Verdict, TickOutcome, loop_tick, OperatingFrequency, config schema | Defines all 6: Substrate, Scorer, Gate, Router, Composer, Policy |

**`roko-core`** is the "one noun, six verbs" made concrete. It defines:

- **Engram** (`Signal` in current code, rename is Tier 0D): The universal content-addressed
  data type with BLAKE3 hashing, 7-axis scoring, four decay variants, lineage DAG, and
  provenance tracking.
- **Six Synapse traits**: The composable interfaces that every capability in Roko implements.
- **`loop_tick`**: The 5-step kernel loop (query → select → compose → verify → persist+policy)
  that executes one cognitive cycle.
- **OperatingFrequency**: The Gamma/Theta/Delta enum with adaptive scheduling and
  affect-driven frequency selection.
- **Config schema**: The `roko.toml` configuration structure for agent setup.

The 376 tests cover the Engram lifecycle, trait contracts, score arithmetic, decay formulas,
content hashing, query filtering, and loop_tick execution.

### 1.3 Layer 1: Framework

Framework crates provide the building blocks: default trait implementations, LLM backends,
tool routing, model cascade, MCP client, and safety capabilities.

| Crate | Status | Tests | Purpose | Primary Traits |
|---|---|---|---|---|
| `roko-std` | Built | 96 | Default implementations of all 6 Synapse traits, 19 built-in tools (file ops, shell, search, MCP), mock dispatcher for testing | Implements all 6 |
| `roko-agent` | Built | 346 | 5 LLM backend drivers (Anthropic Claude, OpenAI, OpenRouter, Ollama, exec-based), connection pooling, CascadeRouter, MCP client, tool dispatch loop, safety layer (role auth + pre/post checks) | Router (CascadeRouter), Scorer (model selection) |

**`roko-std`** is the "batteries-included" crate. It provides sensible defaults for every
Synapse trait:

- `MemorySubstrate`: In-memory Substrate backed by `BTreeMap<ContentHash, Signal>`.
- `DefaultScorer`: Composite scorer using keyword overlap + format matching.
- `CompileGate`, `TestGate`, `ClippyGate`, `DiffGate`: Built-in verification gates.
- `DefaultRouter`: Score-based selection with configurable confidence threshold.
- `SystemPromptComposer`: Prompt assembly with budget constraints.
- `DefaultPolicy`: Baseline reactive policy.

The 19 built-in tools cover: file read/write/edit, shell execution, search (glob + grep),
MCP tool delegation, web fetch (when available), and testing utilities.

**`roko-agent`** provides the bridge between Roko's Synapse Architecture and external LLM
providers. The 5 backends all implement a common `AgentBackend` trait that abstracts
provider-specific details:

- **Anthropic**: Native Claude API with streaming, tool use, extended thinking.
- **OpenAI**: GPT-4o/GPT-4.1 API.
- **OpenRouter**: Meta-routing across 100+ models via OpenRouter API.
- **Ollama**: Local model execution.
- **Exec-based**: Wraps any CLI tool (e.g., `claude`, `aider`) as an agent backend.

The `CascadeRouter` implements the T0/T1/T2 tier routing: given a prediction error from the
16 T0 probes, it selects the cheapest model tier sufficient for the current task. This is the
primary Router implementation used in production.

### 1.4 Layer 2: Scaffold

Scaffold crates handle context engineering — what the LLM sees and how it is assembled.

| Crate | Status | Tests | Purpose | Primary Traits |
|---|---|---|---|---|
| `roko-compose` | Built | 23 | `SystemPromptBuilder` (6-layer prompt assembly), 9 role-specific templates (implementer, debugger, architect, researcher, planner, reviewer, orchestrator, tester, security), context enrichment pipeline, token budget management | Composer (primary), Scorer (section evaluation) |

**`roko-compose`** implements the Composer trait with a 6-layer prompt assembly pipeline:

1. **Role identity layer**: Who the agent is (from 9 templates)
2. **Domain context layer**: Project structure, language support, build system
3. **Task context layer**: Current task PRD, plan constraints, acceptance criteria
4. **Knowledge layer**: Relevant Neuro entries (injected via `&dyn Substrate`)
5. **Iteration memory layer**: Past attempts, failures, fixes (from episodes)
6. **Safety layer**: Constraints, permissions, forbidden actions

Each layer bids for token budget via the VCG Attention Auction mechanism. Sections are
scored by expected value and ranked by bid. The final composed prompt fits within the model's
context window while maximizing information density.

### 1.5 Layer 3: Harness

Harness crates handle verification — did the agent's output actually work?

| Crate | Status | Tests | Purpose | Primary Traits |
|---|---|---|---|---|
| `roko-gate` | Built | 200 | 11+ gate implementations, 6-rung pipeline (syntax → compile → test → lint → diff → semantic), adaptive threshold EMA, gate verdict aggregation, `is_mostly_passing` classification | Gate (primary) |
| `roko-fs` | Built | 37 | `FileSubstrate`: JSONL-based Engram persistence, append-only log, garbage collection by decay/prune, file-layout conventions | Substrate (file-backed) |

**`roko-gate`** provides the verification pipeline that makes Roko's "verify everything"
philosophy concrete. The 6-rung pipeline escalates verification from cheap to expensive:

| Rung | Gate | Cost | What It Checks |
|---|---|---|---|
| 1 | Syntax | ~0ms | Parseable output, valid JSON/TOML/Rust syntax |
| 2 | Compile | ~seconds | `cargo check` / `tsc` / `go build` passes |
| 3 | Test | ~seconds-minutes | `cargo test` / `npm test` / `go test` passes |
| 4 | Lint | ~seconds | `clippy` / `eslint` / `golangci-lint` clean |
| 5 | Diff | ~ms | Output differs meaningfully from input (not no-op) |
| 6 | Semantic | ~seconds | LLM-as-judge or domain-specific semantic check |

Each gate produces a `Verdict` Engram with pass/fail, confidence, reason, test counts, and
error digest. Adaptive thresholds (EMA per rung) learn the expected pass rate over time,
allowing the system to detect anomalous gate failures.

**`roko-fs`** provides the production Substrate: append-only JSONL files with content-hash
indexing. Supports garbage collection by decay (expired Engrams) and explicit prune policies.
File layout follows `.roko/signals.jsonl` with optional per-session partitioning.

### 1.6 Layer 4: Orchestration

Orchestration crates coordinate multi-agent work: planning, scheduling, parallel execution,
and reactive adaptation.

| Crate | Status | Tests | Purpose | Primary Traits |
|---|---|---|---|---|
| `roko-orchestrator` | Built | 158 | Plan DAG execution, parallel task scheduler, merge queue, worktree-based isolation, state persistence + resume, safety validation | Policy (orchestration-level reactive control) |
| `roko-conductor` | Built | — | 10 reactive watchers (file changes, build events, test results, etc.), circuit breaker (exponential backoff on repeated failures), real-time monitoring | Policy (event-driven reactions) |

**`roko-orchestrator`** implements the plan-execute-gate-persist loop that drives Roko's
self-hosting capability:

1. Parse plans from TOML task files
2. Build a DAG of task dependencies
3. Execute tasks in parallel (respecting dependencies)
4. Gate-verify each task output
5. Persist state (snapshot + resume on interruption)
6. Feed gate results back into the learning system

The 158 tests cover DAG construction, parallel execution ordering, state serialization,
resume from interruption, and merge queue conflict detection.

**`roko-conductor`** provides reactive event-driven control. Its 10 watchers monitor the
environment (file system changes, build results, test results, resource usage) and fire
Policy-style interventions when conditions are met. The circuit breaker prevents infinite
retry loops when a persistent error is detected.

### 1.7 Cognitive Cross-Cuts

These crates implement the three cognitive subsystems that are injected across all layers
via trait objects.

| Crate | Status | Tests | Purpose | Primary Traits |
|---|---|---|---|---|
| `roko-learn` | Built | 101 | Episode logging, playbook extraction, skill libraries, epsilon-greedy bandits, CascadeRouter persistence, prompt experiments (A/B), adaptive gate thresholds (EMA) | Substrate (episode storage), Scorer (reward computation), Router (bandit-based selection) |
| `roko-neuro` | Built | — | Knowledge store: 6 knowledge types (Insight, Heuristic, Warning, CausalLink, StrategyFragment, AntiKnowledge), 4 tiers (Transient → Persistent), HDC encoding for similarity search, distillation pipeline | Substrate (knowledge-backed), Scorer (knowledge relevance) |
| `roko-daimon` | Built | — | PAD (Pleasure-Arousal-Dominance) vector, 6 behavioral states (Engaged/Focused/Exploring/Struggling/Coasting/Resting), somatic markers (Damasio), affect-driven frequency selection | Scorer (somatic marker evaluation), Router (affect-biased selection) |
| `roko-dreams` | Scaffold | — | Offline learning: NREM replay (Mattar-Daw), REM imagination (Boden + Pearl SCM), integration staging (0.20→0.70 promotion), hypnagogia engine (Thalamic Gate + Executive Loosener + Dali Interrupt + Homuncular Observer) | Substrate (dream staging), Policy (consolidation decisions) |

### 1.8 Chain

| Crate | Status | Tests | Purpose | Primary Traits |
|---|---|---|---|---|
| `roko-chain` | Built | 52 | `ChainClient` / `ChainWallet` trait abstractions, chain witness (Engram → on-chain attestation), ERC-8004 integration path | Substrate (chain-backed), Gate (chain verification) |

**`roko-chain`** provides the bridge between Roko's Engram-based cognition and on-chain
verifiability. The `ChainClient` trait abstracts blockchain interaction; `ChainWallet`
manages key material and signing. The chain witness module posts Engram attestations on-chain,
enabling the forensic AI capability.

### 1.9 Plugins, Lang, MCP

| Crate | Status | Tests | Purpose |
|---|---|---|---|
| `roko-plugin` | Built | — | Event source framework: file watch, cron scheduling, webhook ingestion |
| `roko-index` | Built | — | Code parsing (tree-sitter), symbol graph construction, HDC fingerprinting for code |
| `roko-lang-rust` | Built | — | Rust language support: Cargo integration, module resolution, type extraction |
| `roko-lang-typescript` | Built | — | TypeScript language support: npm/pnpm integration, type extraction |
| `roko-lang-go` | Built | — | Go language support: module resolution, type extraction |
| `roko-mcp-stdio` | Scaffold | — | MCP server over stdio transport |
| `roko-mcp-github` | Scaffold | — | MCP server for GitHub integration |
| `roko-mcp-slack` | Scaffold | — | MCP server for Slack integration |
| `roko-mcp-scripts` | Scaffold | — | MCP server for script execution |

### 1.10 Applications

| Crate | Status | Tests | Purpose |
|---|---|---|---|
| `roko-cli` | Built | 38 | User-facing binary: all subcommands (init, run, plan, prd, research, status, replay, config, dashboard) |
| `roko-serve` | Scaffold | — | HTTP server + REST API for remote operation |
| `mirage-rs` | Built | 141 | In-process EVM simulator for chain agent testing without real chain interaction |

---

## 2. Crate Dissolution: `roko-golem`

The `roko-golem` crate was an umbrella that aggregated cognitive subsystems behind a
`ScaffoldEngine` trait and a `GolemScaffold` aggregator struct. In the Synapse Architecture,
this aggregation is unnecessary — each subsystem defines its own Synapse trait implementations
that compose through the universal Engram type.

### 2.1 Redistribution Map

| Subsystem | Lines | Source | Destination | Action |
|---|---|---|---|---|
| Daimon | 972 | `roko-golem/daimon.rs` | `roko-daimon` (standalone crate) | Move full implementation |
| Dreams | 43 | `roko-golem/dreams.rs` | `roko-dreams` (standalone crate) | Delete placeholder, expand in roko-dreams |
| Grimoire | 44 | `roko-golem/grimoire.rs` | `roko-neuro` (standalone crate) | Delete placeholder; roko-neuro replaces |
| Chain Witness | 43 | `roko-golem/chain_witness.rs` | `roko-chain` (as `chain_witness` module) | Move |
| Mortality | 44 | `roko-golem/mortality.rs` | **DELETE ENTIRELY** | No mortality in the new architecture |
| Hypnagogia | 42 | `roko-golem/hypnagogia.rs` | `roko-dreams` (as `hypnagogia` module) | Move |
| `ScaffoldEngine` trait | — | `roko-golem/lib.rs` | **DELETE** | Each subsystem has its own trait |
| `GolemScaffold` aggregator | — | `roko-golem/lib.rs` | **DELETE** | Composition at application layer via config |

### 2.2 Composability Principle

After dissolution, any subsystem can pipe to any other through Engrams:

```
Daimon emits Engrams → Neuro stores them
Dreams reads from Neuro → produces new Engrams
Chain posts Engrams on-chain → produces attestation Engrams
Everything flows through the 6 Synapse traits
No umbrella crate needed
```

The application layer (e.g., `roko-cli`) assembles the desired composition via `roko.toml`
configuration, selecting which Substrate, Router, Gate, etc. implementations to use for a
given agent.

---

## 3. Dependency Rules

### 3.1 The Downward-Only Invariant

Dependencies flow strictly downward through the layer hierarchy:

```
L4 (Orchestration) → may depend on L3, L2, L1, L0, Kernel
L3 (Harness)       → may depend on L2, L1, L0, Kernel
L2 (Scaffold)      → may depend on L1, L0, Kernel
L1 (Framework)     → may depend on L0, Kernel
L0 (Runtime)       → may depend on Kernel only
Kernel             → depends on nothing (leaf of dependency tree)
```

Cross-cutting cognitive crates (roko-learn, roko-neuro, roko-daimon, roko-dreams) may
depend on Kernel and L0 but are injected into higher layers via trait objects, never via
direct imports.

### 3.2 Concrete Dependency Graph (Key Edges)

```
roko-cli ─────────→ roko-orchestrator ──→ roko-gate ──→ roko-core
    │                     │                   │              ↑
    ├──→ roko-agent ──────┤                   │              │
    │         │           │                   │              │
    │         └──→ roko-compose ──→ roko-core │              │
    │                     │                   │              │
    ├──→ roko-learn ──────┘                   │              │
    │         │                               │              │
    │         └──→ roko-fs ───────────────────┘              │
    │                                                        │
    ├──→ roko-std ──→ roko-core ─────────────────────────────┘
    │
    └──→ roko-runtime ──→ roko-primitives
```

### 3.3 Test Count Summary

| Crate | Tests | Coverage Focus |
|---|---|---|
| roko-core | 376 | Engram lifecycle, trait contracts, score arithmetic, decay, hashing, loop_tick |
| roko-agent | 346 | Backend drivers, connection pooling, CascadeRouter, safety layer, tool dispatch |
| roko-gate | 200 | Gate pipeline, adaptive thresholds, verdict aggregation, mostly_passing |
| roko-orchestrator | 158 | DAG execution, parallel scheduling, state persistence, resume |
| mirage-rs | 141 | EVM simulation, contract interaction, gas estimation |
| roko-learn | 101 | Episodes, playbooks, bandits, experiments, efficiency events |
| roko-std | 96 | Default trait impls, built-in tools, mock dispatcher |
| roko-chain | 52 | ChainClient/ChainWallet, chain witness |
| roko-cli | 38 | Subcommand parsing, config loading, integration smoke tests |
| roko-fs | 37 | JSONL persistence, GC, layout conventions |
| roko-compose | 23 | Prompt assembly, template rendering, budget enforcement |
| **Total** | **~1,568** | |

---

## 4. Legacy Crate Names

For reference, here is the complete old→new crate naming map:

| Old Name | New Name | Notes |
|---|---|---|
| `bardo-primitives` | `roko-primitives` | HDC vectors, shared types |
| `bardo-runtime` | `roko-runtime` | Event bus, supervision |
| `golem-core` | `roko-core` | Kernel |
| `mori-index` | `roko-index` | Code parsing, symbol graphs |
| `mori-context` | Split: `roko-compose` + `roko-index` | Context features → compose; code intelligence → index |
| `mori-mcp` | `roko-mcp-{stdio,github,slack,scripts}` | Split into transport-specific crates |
| `bardo-terminal` | `roko-cli` | Terminal UI scaffold in roko-cli |
| `roko-golem` | **DISSOLVED** | See Section 2 above |

All `golem-*` and `bardo-*` crate references in legacy documents should be translated to
their `roko-*` equivalents.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Ousterhout 2018, "A Philosophy of Software Design" | Module depth principle: deep modules with narrow interfaces. Applied to Synapse trait design. |
| Parnas 1972, "On the criteria to be used in decomposing systems into modules" | Information hiding: each crate hides implementation behind trait interfaces |
| Beer 1972, Brain of the Firm | Viable System Model: recursive subsystem organization maps to layer hierarchy |

---

## Current Status and Gaps

- **18+ crates built**: The workspace compiles and passes ~1,568 tests across all crates.
  Requires rustc 1.91+ for alloy dependencies.
- **roko-golem dissolution**: Specified but not yet executed as a code migration. The Daimon
  implementation (972 lines) exists in both roko-golem and roko-daimon. The authoritative
  version should be the standalone `roko-daimon` crate.
- **MCP crates**: Scaffolded but not fully implemented. `roko-mcp-stdio` has basic transport;
  the others are stubs.
- **roko-serve**: Scaffolded. HTTP API not yet wired.
- **roko-dreams**: Scaffolded. Three-phase cycle specified but not shipping.
- **Signal → Engram rename**: The core Rust type is still named `Signal`. Rename to `Engram`
  is Tier 0D in the implementation plan. All PRD documentation uses "Engram" but code samples
  reference `Signal` with explanatory comments.

---

## Cross-References

- See [02-engram-data-type](./02-engram-data-type.md) for the Engram struct that all crates process
- See [06-synapse-traits](./06-synapse-traits.md) for the trait definitions that crates implement
- See [12-five-layer-taxonomy](./12-five-layer-taxonomy.md) for the layer assignments
- See [01-naming-and-glossary](./01-naming-and-glossary.md) for the complete old→new naming map
- See topic [17-lifecycle](../17-lifecycle/INDEX.md) for the roko-golem dissolution plan
