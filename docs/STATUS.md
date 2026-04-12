# Roko Implementation Status

> **Last updated**: 2026-04-12
>
> Single source of truth for what's implemented vs. specified across the Roko system.
> For naming conventions, see [`00-architecture/01-naming-and-glossary.md`](00-architecture/01-naming-and-glossary.md).
> For the crate map, see [`00-architecture/15-crate-map.md`](00-architecture/15-crate-map.md).

---

## Status Tiers

| Tier | Meaning |
|------|---------|
| **Shipping** | End-to-end wired, tested, used in self-hosting workflow. CLI-accessible. |
| **Built** | Code exists, compiles, has tests — but not yet called from the runtime or CLI. |
| **Scaffold** | Struct/trait stubs exist. No meaningful implementation. |
| **Specified** | Described in PRD docs only. No code. |
| **Deferred** | Intentionally postponed (Phase 2+, chain-dependent, or research-only). |

---

## Master Status Matrix

| # | Section | Tier | Primary Crate(s) | Status Doc |
|---|---------|------|-------------------|------------|
| 00 | [Architecture](00-architecture/INDEX.md) | **Shipping** | `roko-core` (376 tests) | — |
| 01 | [Orchestration](01-orchestration/INDEX.md) | **Shipping** | `roko-orchestrator` (158 tests), `roko-cli` | — |
| 02 | [Agents](02-agents/INDEX.md) | **Shipping** | `roko-agent` (346 tests) | [15-status-gaps.md](02-agents/15-status-gaps.md) |
| 03 | [Composition](03-composition/INDEX.md) | **Shipping** | `roko-compose` (23 tests) | [13-current-status-and-gaps.md](03-composition/13-current-status-and-gaps.md) |
| 04 | [Verification](04-verification/INDEX.md) | **Shipping** | `roko-gate` (200 tests), `roko-fs` (37 tests) | — |
| 05 | [Learning](05-learning/INDEX.md) | **Shipping** | `roko-learn` (101 tests) | — |
| 06 | [Neuro](06-neuro/INDEX.md) | **Built** | `roko-neuro` | [16-current-status-and-gaps.md](06-neuro/16-current-status-and-gaps.md) |
| 07 | [Conductor](07-conductor/INDEX.md) | **Built** | `roko-conductor` | — |
| 08 | [Chain](08-chain/INDEX.md) | **Built** | `roko-chain` (52 tests) | — |
| 09 | [Daimon](09-daimon/INDEX.md) | **Built** | `roko-daimon` | [13-current-status-and-gaps.md](09-daimon/13-current-status-and-gaps.md) |
| 10 | [Dreams](10-dreams/INDEX.md) | **Scaffold** | `roko-dreams` | [16-implementation-status.md](10-dreams/16-implementation-status.md) |
| 11 | [Safety](11-safety/INDEX.md) | **Shipping** (core) / **Specified** (advanced) | `roko-agent` (safety layer) | — |
| 12 | [Interfaces](12-interfaces/INDEX.md) | **Scaffold** | `roko-cli` (text dashboard) | — |
| 13 | [Coordination](13-coordination/INDEX.md) | **Specified** | — | [12-current-status-and-gaps.md](13-coordination/12-current-status-and-gaps.md) |
| 14 | [Identity & Economy](14-identity-economy/INDEX.md) | **Deferred** | — | — |
| 15 | [Code Intelligence](15-code-intelligence/INDEX.md) | **Built** | `roko-index`, `roko-lang-*` | [10-current-status-and-gaps.md](15-code-intelligence/10-current-status-and-gaps.md) |
| 16 | [Heartbeat](16-heartbeat/INDEX.md) | **Specified** | — | — |
| 17 | [Lifecycle](17-lifecycle/INDEX.md) | **Specified** | — | — |
| 18 | [Tools](18-tools/INDEX.md) | **Shipping** (builtins) / **Scaffold** (MCP servers) | `roko-std` (96 tests) | — |
| 19 | [Deployment](19-deployment/INDEX.md) | **Specified** | — | — |
| 20 | [Technical Analysis](20-technical-analysis/INDEX.md) | **Specified** | — | — |
| 21 | [References](21-references/INDEX.md) | N/A (bibliography) | — | — |

---

## Detailed Breakdown

### Shipping (end-to-end wired, CLI-accessible)

These components form the working self-hosting loop: `roko prd` → `roko plan run` → gate → persist → resume.

| Component | Crate | Tests | CLI Command |
|-----------|-------|-------|-------------|
| Signal/Engram type + 6 Synapse traits | `roko-core` | 376 | — (kernel) |
| Plan DAG executor + parallel scheduling | `roko-orchestrator` | 158 | `roko plan run` |
| 5 LLM backends + CascadeRouter + MCP | `roko-agent` | 346 | `roko run` |
| 6-layer SystemPromptBuilder + 9 templates | `roko-compose` | 23 | — (used by orchestrator) |
| 11 gates + 6-rung pipeline + adaptive thresholds | `roko-gate` | 200 | — (used by orchestrator) |
| JSONL FileSubstrate + GC | `roko-fs` | 37 | — (storage layer) |
| Episodes + playbooks + bandits + experiments | `roko-learn` | 101 | — (feedback loops) |
| 19 built-in tools (file, shell, search, MCP) | `roko-std` | 96 | — (tool dispatch) |
| ProcessSupervisor + event bus + cancellation | `roko-runtime` | — | — (infra) |
| Safety layer (role auth + pre/post checks) | `roko-agent` | — | — (integrated) |
| PRD lifecycle (idea/draft/plan) | `roko-cli` | 38 | `roko prd` |
| Research agent (topic/enhance) | `roko-cli` | — | `roko research` |
| Session persistence + resume | `roko-cli` | — | `roko plan run --resume` |
| Efficiency events + cascade router persist | `roko-learn` | — | — (auto) |
| Configuration management | `roko-cli` | — | `roko config` |

### Built (compiles, has code, not fully wired to runtime)

| Component | Crate | Tests | Gap |
|-----------|-------|-------|-----|
| HDC vectors (10,240-bit) + fingerprinting | `roko-primitives` | — | Used by roko-index, not yet by runtime |
| Knowledge store (6 types × 4 tiers) | `roko-neuro` | — | Struct exists; not wired to orchestrator knowledge injection |
| PAD vector + 6 behavioral states + somatic markers | `roko-daimon` | — | Struct exists; not wired to tier routing |
| 10 reactive watchers + circuit breaker | `roko-conductor` | — | Built but not called from orchestrate.rs |
| Chain client + wallet + witness | `roko-chain` | 52 | Needs Korai chain deployment |
| Tree-sitter parsing + symbol graph + PageRank | `roko-index` | — | Built; MCP server not wired |
| Rust/TypeScript/Go language support | `roko-lang-*` | — | Built; used by roko-index |
| EVM simulator | `mirage-rs` | 141 | Chain-domain testing tool; works standalone |

### Scaffold (stubs only)

| Component | Crate | Gap |
|-----------|-------|-----|
| Dream engine (NREM/REM/integration) | `roko-dreams` | Runner + cycle facades exist; core algorithms unimplemented |
| MCP servers (GitHub, Slack, Scripts, Stdio) | `roko-mcp-*` | Crate stubs, no implementation |
| HTTP server + REST API | `roko-serve` | Crate exists, no routes |
| Text dashboard (TUI) | `roko-cli` | Renders text pages, no interactive terminal UI |

### Specified (PRD docs only, no code)

| Component | Section | Key Docs |
|-----------|---------|----------|
| Heartbeat cognitive loop (Gamma/Theta/Delta) | 16 | 9-step pipeline, dual process, attention auction |
| Agent mesh + pheromone gossip | 13 | Stigmergy, pheromone kinds, mesh sync |
| Morphogenetic specialization | 13 | Reaction-diffusion agent differentiation |
| Technical analysis oracles (7 frontier methods) | 20 | HDC-TA, spectral manifolds, causal discovery, TDA |
| Temporal logic safety monitors | 11 | Büchi automata, LTL/CTL verification |
| Witness DAG + ZK proofs | 11 | Content-addressed audit DAG, plonky2 |
| Formal verification pipeline | 11 | Heimdall, Slither, Echidna, HEVM |
| Cognitive kernel safety | 11 | Namespace isolation, signal delivery |
| Active inference compute allocation | 16 | EFE estimation, LinUCB bandits |
| Plugin SDK + event sources | 18 | Domain plugin automation |
| Agent lifecycle (birth → retirement) | 17 | Full lifecycle state machine |
| Deployment (cloud, bare-metal, hybrid) | 19 | Infrastructure patterns |

### Deferred (Phase 2+)

| Component | Section | Why Deferred |
|-----------|---------|--------------|
| Identity & economy layer | 14 | Requires Korai chain launch |
| ERC-8004 registries | 08 | Chain-dependent |
| x402 micropayments | 08 | Chain-dependent |
| Reputation system (7-domain) | 08 | Chain-dependent |
| Sonification / audio interface | 12 | Research/experimental |
| Generational evolution (agent lineages) | 17 | Requires stable mesh + economy |

---

## Test Coverage Summary

| Crate | Tests | Layer |
|-------|-------|-------|
| `roko-core` | 376 | Kernel |
| `roko-agent` | 346 | L1 Framework |
| `roko-gate` | 200 | L3 Harness |
| `roko-orchestrator` | 158 | L4 Orchestration |
| `mirage-rs` | 141 | L1 Framework (chain testing) |
| `roko-learn` | 101 | Cross-cut |
| `roko-std` | 96 | L1 Framework |
| `roko-chain` | 52 | L1 Framework |
| `roko-cli` | 38 | L4 Application |
| `roko-fs` | 37 | L3 Harness |
| `roko-compose` | 23 | L2 Scaffold |
| **Total** | **1,568** | |

---

## Critical Path to Full Self-Hosting

The self-hosting loop works today (`prd` → `plan run` → gate → persist → resume`). Three capabilities would close the remaining gaps:

1. **Interactive TUI** (Section 12) — Wire `ratatui` into the text dashboard scaffold. Currently `roko dashboard` outputs plain text.

2. **Automatic plan generation** (Section 01) — Trigger `prd plan` automatically when a PRD is published, removing the manual step.

3. **Feedback loop** (Section 05) — Failed task gates feed back into the plan generator for automatic re-planning, closing the learn-from-failure cycle.

After these three, Roko can fully self-host: read its own PRDs, generate plans, execute them, validate results, learn from failures, and iterate — without human intervention beyond initial PRD creation.

---

## How to Read This Document

- **If you're new**: Start with [QUICKSTART.md](QUICKSTART.md), then return here for orientation.
- **If you're implementing**: Find your target section in the matrix above. Check the tier. If "Shipping," you're extending working code. If "Built," you're wiring existing code into the runtime. If "Specified," you're building from the PRD spec.
- **If you're debugging**: The test count column tells you where coverage exists. Crates with `—` for tests are the riskiest to modify.
- **If you're reviewing**: Cross-reference with section-specific status docs (linked in the matrix) for detailed gap analysis.
