# Roko — Executive Summary

> A cognitive architecture for self-developing agents. 36 workspace members, ~322K LOC, 3,761 tests.
>
> **Updated**: 2026-04-13 · **Audience**: Technical executives, investors, engineering leads

---

## Thesis

Roko is a Rust toolkit that gives AI agents the cognitive machinery to develop themselves.
Where frameworks like LangChain chain LLM calls in sequences, Roko provides a full cognitive
architecture — memory that decays, emotions that modulate compute allocation, offline
consolidation that transforms experience into knowledge, and verification at every step.
The core self-hosting loop works today: Roko reads its own product requirements, generates
implementation plans, dispatches Claude agents to execute them, validates outputs through an
11-gate pipeline, learns from outcomes, and persists everything as content-addressed, scored,
decaying data. The scaffold IS the product — every improvement to Roko improves the system
that builds Roko.

---

## System at a Glance

**Architecture**: 1 noun (Engram) + 6 verb traits (Substrate, Scorer, Gate, Router, Composer, Policy).

**Universal loop**: query → score → route → compose → act → verify → persist → react.

**Three cognitive speeds**: Gamma (~5-15s reactive), Theta (~75s reflective), Delta (hours, consolidation).

**Five layers**: L0 Runtime → L1 Framework → L2 Scaffold → L3 Harness → L4 Orchestration.

**Cross-cuts**: Neuro (knowledge), Daimon (affect), Dreams (offline learning).

---

## Section Summaries

### 00 — Architecture (Shipping)

The Synapse Architecture defines one universal data type (the Engram — content-addressed via
BLAKE3, 7-axis scored, with four decay models) and six composable traits that process it.
Every capability in the system, from code generation to knowledge consolidation, is an
implementation of one of these six traits. 376 tests in `roko-core`.

### 01 — Orchestration (Shipping)

The L4 Orchestration layer coordinates multiple agents via a pure state machine
(`ParallelExecutor`) that schedules tasks from a cross-plan DAG, isolates work in git
worktrees, serializes merges via a file-conflict-aware queue, and recovers from crashes
using hash-chained event-log replay. 158 tests.

### 02 — Agents (Shipping)

Five LLM backends (Claude CLI, Anthropic API, OpenAI-compat, Cursor ACP, Ollama), a
three-stage CascadeRouter (Static → Confidence → UCB) for cost-optimal model selection,
MCP tool integration, and a 7-step safety pipeline. 346 tests.

### 03 — Composition (Shipping)

A 7-layer SystemPromptBuilder with 12 role templates, cache-aligned prompt assembly,
Liu et al. 2023 "lost in the middle" U-shape placement, token budget management, and a
13-step enrichment pipeline. Scaffold changes alone produce a 6× performance gap
(Lee et al. 2026). 36+ tests.

### 04 — Verification (Shipping)

An 11-gate, 7-rung pipeline (Compile → Lint → Test → Symbol → GeneratedTest → PropertyTest
→ Integration) with short-circuit execution, monotonic ratcheting, EMA-based adaptive
thresholds, process reward models, and forensic causal replay. Design principle: gate
failure is a verdict, not an error. 200 tests.

### 05 — Learning (Shipping)

Every agent turn updates 10+ learning subsystems simultaneously: episode logger, cost
tracker, playbook rules, skill library, pattern miner, cascade router, C-Factor metric,
regression detector, experiments, and efficiency events. Three bandit algorithms (UCB1,
LinUCB, Track-and-Stop) drive online decision-making. 101 tests.

### 06 — Neuro (Built)

A tiered knowledge system: 6 knowledge types (Insight, Heuristic, Warning, CausalLink,
StrategyFragment, AntiKnowledge) × 4 validation tiers (Transient → Working → Consolidated
→ Persistent), encoded as 10,240-bit hyperdimensional computing vectors for sub-millisecond
similarity search. Built but not yet wired to the runtime.

### 07 — Conductor (Built)

A cybernetic regulator implementing the Good Regulator Theorem (Conant & Ashby 1970): 10
watchers, graduated interventions (Continue/Restart/Fail), stuck detection, circuit breakers,
EWMA anomaly detection, and Yerkes-Dodson pressure dynamics. Built but not called from the
orchestrator.

### 08 — Chain / Korai (Built)

A dedicated EVM for agent coordination: soulbound identity passports, 7-domain reputation
with EMA, Spore/Sparrow job marketplace, HDC precompile at ~400 gas, KORAI/DAEJI tokens
with 1% annual demurrage, and ISFR clearing with KKT certificates. 52 tests; blocked by
chain deployment.

### 09 — Daimon (Built)

An affect engine using PAD vectors (Pleasure-Arousal-Dominance) that modulate model tier
selection, exploration rate, context retrieval, and compute allocation. Six cyclical behavioral
states (Engaged, Struggling, Coasting, Exploring, Focused, Resting) — no terminal state.
Implements Damasio's somatic marker hypothesis for fast pattern-matching.

### 10 — Dreams (Scaffold)

Offline consolidation: NREM replay (Mattar-Daw utility-based episode selection), REM
imagination (Pearl SCM counterfactual reasoning), integration staging (knowledge tier
promotion), and hypnagogia (stochastic resonance for creative insight). Lin et al. 2025
shows sleep-time compute yields ~5× test-time cost reduction.

### 11 — Safety (Shipping core / Specified advanced)

Defense-in-depth: 6 runtime guards, capability tokens, content-addressed audit chains, taint
tracking, temporal logic monitors (LTL Büchi automata), witness DAGs with ZK proof paths, and
a 5-stage formal verification pipeline. The #1 gap: `SafetyLayer` is wired but not invoked
from the production code path.

### 12 — Interfaces (Scaffold)

CLI binary, ratatui TUI with ROSEDUST design language, HTTP API (roko-serve), Web Portal
(React 19 / Next.js), Spectre creature visualization (procedurally generated from agent
state), ambient sonification, and A2UI generative interface protocol. TUI wiring is on the
critical path.

### 13 — Coordination (Specified)

Stigmergy-based multi-agent coordination: typed/decaying/scoped pheromones, Agent Mesh
transport (WebSocket + Iroh P2P), morphogenetic specialization via Turing
reaction-diffusion, and 10 exponential flywheel mechanisms for superlinear collective
intelligence growth.

### 14 — Identity & Economy (Deferred)

ERC-8004 agent registries, Korai Passport (soulbound ERC-721), knowledge marketplace with
alpha-decay pricing, Vickrey reputation auctions, x402 micropayments, LMSR prediction
markets, and Shapley attribution. Requires Korai chain launch.

### 15 — Code Intelligence (Built)

Tree-sitter parsing, symbol graph with PageRank importance scoring, 10,240-bit HDC
fingerprints, three language providers (Rust, TypeScript, Go). 30 tests. Major gap: no
persistent storage, no search API, no MCP server for agents.

### 16 — Heartbeat (Specified)

The autonomous cognitive clock: 9-step CoALA-derived pipeline at three concurrent speeds,
dual-process T0/T1/T2 tier gating (~80% of ticks free), 16 zero-LLM probes, VCG attention
auction, and active inference POMDP for compute allocation.

### 17 — Lifecycle (Specified)

User-directed agent lifecycle: CREATE → CONFIGURE → FUND → RUN → BACKUP → DELETE → CREATE →
RESTORE. Knowledge transfers across generations via selective backup/restore with 0.85^N
generational confidence decay. Replaces all legacy mortality framing.

### 18 — Tools (Shipping builtins / Scaffold servers)

19 built-in tools, domain plugin SDK, 4 MCP server specs (GitHub 17 tools, Slack 8 tools,
scripts, stdio), 18 agent templates, event sources (cron, file watch, webhooks), and three
plugin loading mechanisms.

### 19 — Deployment (Specified)

Native binaries (x86_64 + aarch64), Docker, WASM (~500KB), daemon mode (launchd/systemd),
Fly.io cloud, edge deployment, multi-repo coordination, and production hardening (adaptive
timeouts, hedged requests, graceful shutdown).

### 20 — Technical Analysis (Specified)

Universal Oracle primitives generalized beyond finance: the `Oracle` trait provides
predict/evaluate for any verifiable domain. Seven frontier methods: HDC pattern algebra,
spectral liquidity manifolds, adaptive signal metabolism, causal microstructure discovery,
TDA persistence landscapes, somatic TA, and sheaf-theoretic consistency.

### 21 — References (260+ citations)

Academic bibliography across 25 research domains, from cognitive science and cybernetics to
mechanism design and tropical geometry. Every architectural decision traces to peer-reviewed
research.

---

## Five Most Innovative Ideas

### 1. Everything Decays (Temporal Knowledge Management)

Every piece of knowledge has a half-life determined by its validation tier. Transient
knowledge (unverified) decays in hours; Persistent knowledge (cross-validated) decays over
months. Four decay models (exponential, linear, stepped, Ebbinghaus) prevent stale
information from poisoning decisions — a problem that grows worse as agent systems run longer.
No other framework treats knowledge temporality as a first-class architectural concern.

**Grounding**: Ebbinghaus (1885), Murre & Dros (2015), McClelland et al. (1995).

### 2. Affect-Driven Compute Allocation (Daimon)

PAD vectors (Pleasure, Arousal, Dominance) from cognitive science directly modulate which
model tier the agent uses, how much context it assembles, and whether it explores or
exploits. When things go well, the agent uses cheaper models and fewer retries. When things
go badly, it escalates to stronger models with richer context. This is not anthropomorphism
— it is a proven decision-making optimization (Damasio's somatic marker hypothesis) that
produces measurable cost savings.

**Grounding**: Mehrabian & Russell (1974), Damasio (1994), Bechara et al. (1997).

### 3. Offline Consolidation (Dreams)

When idle, agents enter a three-phase dream cycle: NREM replay (prioritized memory access for
high-utility episodes), REM imagination (structural causal model counterfactuals), and
integration staging (knowledge tier promotion). This transforms raw experience into validated
knowledge — the same process biological brains use for memory consolidation. Lin et al. (2025)
demonstrates sleep-time compute yields ~5× reduction in test-time cost with 13-18% accuracy
improvement.

**Grounding**: Mattar & Daw (2018), Walker & van der Helm (2009), Pearl (2009), Lin et al. (2025).

### 4. Verification as Cognition (Gate Pipeline)

Gate verdicts are themselves Engrams that re-enter the cognitive loop. The 7-rung pipeline
with adaptive EMA thresholds, monotonic ratcheting, and process reward models creates a
system where verification is not a post-hoc check but a core cognitive operation. The system
learns its own expected pass rates and flags anomalies. Song et al. (ICLR 2025) shows the
generation-verification gap is the key to self-improvement.

**Grounding**: Song et al. (2025), Lightman et al. (2023), Lee et al. (2026).

### 5. Self-Development Loop (The Scaffold IS the Product)

Roko uses itself to develop itself: `prd idea` → `prd draft` → `research enhance-prd` →
`prd plan` → `plan run` → gate → persist → resume. Each improvement to the scaffold improves
the agent that builds the scaffold, creating a compound improvement loop. Zhang et al. (2025)
demonstrate this pattern can improve SWE-bench performance from 20% to 50%; Robeyns (2025)
shows a self-editing coding agent achieving 17% → 53% gains.

**Grounding**: Kauffman (1993), Zhang et al. (2025), Robeyns (2025), Liu & van der Schaar (2025).

---

## Ten Critical Implementation Priorities

| # | Priority | Effort | Impact | Status |
|---|----------|--------|--------|--------|
| 1 | **Interactive TUI** — Wire ratatui into the text dashboard scaffold | Medium | High — primary operator interface | Scaffold |
| 2 | **Automatic plan generation** — Trigger `prd plan` when a PRD is published | Small | High — removes manual step from self-hosting loop | Not started |
| 3 | **Failure feedback loop** — Failed gates feed back into plan generator for re-planning | Medium | High — closes learn-from-failure cycle | Not started |
| 4 | **Wire SafetyLayer** — Connect ToolDispatcher into production code path (orchestrate.rs) | Small | Critical — safety architecture is built but dormant | Built, not wired |
| 5 | **Wire Conductor** — Connect 10 watchers + circuit breaker into orchestrate.rs | Small | High — anomaly detection exists but is unused | Built, not wired |
| 6 | **Wire Neuro** — Connect knowledge store into orchestrator context injection | Medium | High — knowledge management is the foundation for learning | Built, not wired |
| 7 | **Wire Daimon** — Connect affect engine into tier routing and prompt assembly | Medium | Medium — cost optimization through affect-modulated routing | Wired |
| 8 | **Code Intelligence MCP** — Expose roko-index via MCP server for agent consumption | Medium | High — agents need structural code understanding | Built, no server |
| 9 | **Implement Dream Runner** — Wire NREM replay + REM imagination into Delta loop | Large | Medium — offline consolidation for long-running agents | Scaffold |
| 10 | **Heartbeat Gamma/Theta/Delta** — Formal three-speed cognitive loop with adaptive clock | Large | High — autonomous agent operation without human triggers | Specified |

**Items 1-3 are the critical path to full self-hosting.** After these, Roko can develop
itself end-to-end without human intervention beyond initial PRD creation.

---

## Comparison to State of the Art

| Dimension | Roko | LangChain / CrewAI | SWE-Agent | AutoGPT | Research Frontier |
|-----------|------|-------------------|-----------|---------|-------------------|
| **Architecture** | Cognitive (1+6 trait composition) | DAG/chain or role-based | Single agent + ACI | Loop-based | CoALA (Sumers 2023), Agentic AI Survey (2025) |
| **Memory** | 6 types × 4 tiers, HDC, Ebbinghaus decay | Vector store (external) | None built-in | None | Memory in MAS (2025), Park et al. Generative Agents |
| **Self-improvement** | Full loop: PRD → plan → execute → gate → learn | None | SWE-bench eval | None | Darwin Gödel Machine (Zhang 2025), Robeyns (2025) |
| **Offline learning** | Three-phase dreams, sleep-time compute | None | None | None | Lin et al. (2025), NeuroDream (2025) |
| **Verification** | 11 gates, 7 rungs, adaptive thresholds | Optional callbacks | SWE-bench | None | Song et al. (2025), Process Reward Models |
| **Affect model** | PAD vectors, somatic markers, 6 states | None | None | None | Emotional RAG (2024), Yin et al. (2025) |
| **Multi-agent** | Pheromone stigmergy, morphogenetic specialization | Agent executor | Single agent | None | Emergent Coordination (Riedl 2025) |
| **Safety** | 6 guards + temporal logic + witness DAG + taint | None built-in | Container sandbox | None | CaMeL (Debenedetti 2025), OWASP Agentic Top 10 |
| **Language** | Rust (performance, safety) | Python | Python | Python | — |
| **Test coverage** | 3,761 tests across 36 workspace members | Varies | SWE-bench | Minimal | — |

**Roko's key differentiator**: It is the only framework that combines cognitive architecture
(not just prompt chaining), temporal knowledge management, affect-driven compute allocation,
offline consolidation, and a working self-development loop — all in a systems language with
strong verification. The closest academic comparisons are CoALA (Sumers et al. 2023) for
architecture and Darwin Gödel Machine (Zhang et al. 2025) for self-improvement, but neither
provides a complete, integrated, shipping system.

---

## Biggest Open Research Questions

1. **Does the autocatalytic improvement thesis hold empirically?** Kauffman's autocatalytic
   set theory predicts compound improvement: if each subsystem improves the others by 10%,
   the compound effect is 0.9^4 = 0.656 (34% total improvement). Can this be measured via
   C-Factor trends in production?

2. **How should intrinsic metacognition be implemented?** Liu & van der Schaar (2025) argue
   that true self-improvement requires agents that learn *how to learn*, not just agents that
   change code. Roko's Theta loop and Conductor provide the hooks, but the metacognitive
   learning algorithm is unspecified.

3. **What is the optimal balance between knowledge persistence and decay?** Ebbinghaus curves
   prevent stale data, but aggressive decay loses hard-won insights. The decay-tier matrix
   needs empirical calibration across diverse agent workloads.

4. **Can stigmergic coordination scale to hundreds of agents?** The pheromone model is
   O(N×M) vs. O(N²) for direct communication, but real-world performance depends on
   pheromone field saturation, interference, and the SINR model for signal quality.

5. **Is the VCG attention auction worth its complexity?** The second-price mechanism for
   context budget allocation is theoretically optimal, but a simpler priority queue may
   achieve 90% of the benefit at 10% of the implementation cost.

6. **How effective is HDC for cross-domain transfer?** The 0.526 similarity threshold for
   detecting structural analogies between domains (code ↔ finance ↔ research) is derived
   from Johnson-Lindenstrauss bounds, but real-world false positive rates need validation.

---

## Six-Month Implementation Roadmap

### Month 1-2: Close the Self-Hosting Loop

- [ ] **Interactive TUI** — ratatui dashboard with ROSEDUST design language
- [ ] **Automatic plan generation** — `prd plan` triggers on PRD publish
- [ ] **Failure feedback loop** — gate failures feed back into re-planning
- [ ] **Wire SafetyLayer** — connect ToolDispatcher to orchestrate.rs

**Milestone**: Roko develops itself end-to-end. Human creates PRDs; Roko handles the rest.

### Month 3-4: Wire the Cognitive Subsystems

- [ ] **Wire Conductor** — 10 watchers + circuit breaker in production
- [ ] **Wire Neuro** — knowledge injection into agent prompts
- [x] **Wire Daimon** — affect-modulated tier routing and context
- [ ] **Code Intelligence MCP** — expose roko-index to agents via MCP server
- [ ] **Implement basic Dream Runner** — NREM replay during agent idle time

**Milestone**: Agents have memory, emotions, anomaly detection, and structural code
understanding. Cost optimization through affect-modulated routing.

### Month 5-6: Autonomous Operation

- [ ] **Heartbeat Gamma/Theta/Delta** — formal three-speed cognitive loop
- [ ] **T0 Probe Registry** — 16 zero-LLM probes for tier suppression
- [ ] **Agent Mesh transport** — WebSocket-based pheromone propagation
- [ ] **HTTP API** — roko-serve for remote orchestration
- [ ] **Production hardening** — adaptive timeouts, hedged requests, graceful shutdown

**Milestone**: Agents run autonomously with continuous cognitive loops, multi-agent
coordination, and remote monitoring. ~80% of heartbeat ticks require no LLM call.

### Beyond 6 Months (Phase 2)

- Korai chain deployment and ERC-8004 registries
- Identity/economy layer activation
- Full dream cycle (REM imagination, hypnagogia)
- VCG attention auction
- Active inference POMDP for compute allocation
- WASM deployment for edge agents

---

## Key Metrics

| Metric | Current | 6-Month Target |
|--------|---------|----------------|
| Crates | 36 workspace members | 36 workspace members (consolidate, don't add) |
| LOC | ~322K | ~200K |
| Tests | 3,761 | 2,500+ |
| Shipping sections | 6 of 22 | 12 of 22 |
| Self-hosting steps automated | 6 of 8 | 8 of 8 |
| Cognitive subsystems wired | 0 of 3 | 3 of 3 (Neuro, Daimon, Dreams) |

---

## How to Read the Full Documentation

| Goal | Start Here |
|------|-----------|
| Understand the architecture | [00-architecture/INDEX.md](00-architecture/INDEX.md) |
| See what's implemented | [STATUS.md](STATUS.md) |
| Use the CLI | [QUICKSTART.md](QUICKSTART.md) |
| Compare to alternatives | [COMPARISON.md](COMPARISON.md) |
| Implement something | STATUS.md → section INDEX.md → status/gaps doc |

The full PRD corpus: 22 sections, 384+ documents, ~137K lines, 260+ academic citations.

---

*Generated 2026-04-13 · Roko v0.1 · 36 workspace members · 3,761 tests · [github.com/nunchi/roko](https://github.com/nunchi/roko)*
