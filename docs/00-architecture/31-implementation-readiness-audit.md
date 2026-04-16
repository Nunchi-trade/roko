# Implementation Readiness Audit

> Generated 2026-04-13. Covers all 21 doc sections (excluding 21-references), 350+ files,
> cross-referenced against 18 crates (~177K LOC, ~3,391 tests).

## Methodology

Every markdown file in each section was read and scored against 6 implementation-readiness criteria:

| Criterion | What it measures |
|---|---|
| **rust_structs** | Quality/completeness of struct, trait, enum definitions with field-level types |
| **pseudocode** | Algorithm pseudocode, Rust code blocks, step-by-step decision logic |
| **config_params** | Configuration parameters with defaults, ranges, rationale |
| **error_handling** | Error types, recovery paths, failure mode specification |
| **integration_wiring** | How components connect to other crates and the CLI entry point |
| **test_criteria** | Observable test conditions, acceptance thresholds, named test cases |

Scale: 0 = absent, 1 = mentioned, 2 = partial, 3 = adequate, 4 = strong, 5 = exemplary.

File classifications:
- **Specified** — Has concrete Rust types, real constants, wiring details; directly implementable
- **Scaffold** — Has design intent, partial types, or pseudocode but gaps remain
- **Concept only** — Primarily theoretical, no concrete implementation material
- **Built** — Code exists in crates and tests pass
- **Wired** — Code exists AND is called from the CLI/orchestration loop

## Rewrite Readiness

This audit measures incremental implementation readiness: whether a section can be finished by filling the gaps in the current shape. REF21 adds a second question: when does the current shape itself become the blocker, and when is a from-scratch rewrite the lower-risk move?

Heuristic: prefer a from-scratch rewrite when at least three of these are true:

1. The current design embeds an assumption the new design must invert.
2. The code surface is small enough to replace without long compatibility debt.
3. The public interface can stay stable or be feature-flagged.
4. The rewrite unlocks capability that incremental work cannot reach cleanly.
5. The current design has few direct users or weak plugin contracts.

In REF21, the five rewrite-track candidates are the roko-core kernel, roko-learn reorganization, Substrate trait rewrite, gate pipeline, and roko-compose engine. Treat them as v2 cutovers, not backlog items: the point is to decide when a clean replacement beats patching the current shape. Terminology follows [01-naming-and-glossary.md](./01-naming-and-glossary.md). See also `tmp/refinements/21-from-scratch-redesigns.md`.

### Risk Management

Rewrite-track work needs stronger governance than ordinary feature work:

1. Feature-flag the new kernel path so the old and new shapes can coexist briefly.
2. Keep a one-shot compatibility reader for existing Engram data rather than a permanent dual-world shim.
3. Record current test outputs before cutover and require parity or explicit justification afterward.
4. Land at most one rewrite per week and allow a bake period before removing the old path.
5. Require a named owner, a written before/after contract, a rollback plan, and two sign-offs for the kernel rewrite.

---

## Section Scorecard

| # | Section | Structs | Pseudo | Config | Errors | Wiring | Tests | Total | Complexity | Crate Status |
|---|---|---|---|---|---|---|---|---|---|---|
| 00 | Architecture | 4 | 4 | 4 | 3 | 3 | 3 | **21/30** | Very High | roko-core: Stable |
| 01 | Orchestration | 5 | 5 | 5 | 5 | 5 | 5 | **30/30** | High | roko-orchestrator: Wired |
| 02 | Agents | 4 | 4 | 4 | 3 | 3 | 3 | **21/30** | High | roko-agent: Stable |
| 03 | Composition | 4 | 5 | 5 | 3 | 4 | 4 | **25/30** | Very High | roko-compose: Wired |
| 04 | Verification | 5 | 4 | 5 | 4 | 5 | 4 | **27/30** | Medium | roko-gate: Wired |
| 05 | Learning | 5 | 5 | 5 | 4 | 5 | 5 | **29/30** | High | roko-learn: Wired |
| 06 | Neuro | 4 | 4 | 5 | 3 | 3 | 3 | **22/30** | High | Core + legacy agent scaffold |
| 07 | Conductor | 5 | 5 | 5 | 5 | 5 | 4 | **29/30** | High | roko-conductor: Wired |
| 08 | Chain | 4 | 3 | 3 | 2 | 3 | 3 | **18/30** | Very High | roko-chain: Scaffold |
| 09 | Daimon | 5 | 4 | 4 | 3 | 3 | 4 | **23/30** | Medium-High | Legacy agent scaffold path |
| 10 | Dreams | 4 | 4 | 5 | 2 | 4 | 4 | **23/30** | High | Legacy agent scaffold path |
| 11 | Safety | 5 | 4 | 5 | 4 | 5 | 4 | **27/30** | High | (in roko-orchestrator) |
| 12 | Interfaces | 4 | 4 | 5 | 3 | 4 | 3 | **23/30** | Very High | roko-cli: Wired (partial) |
| 13 | Coordination | 5 | 5 | 5 | 3 | 5 | 4 | **27/30** | Very High | 0% implemented |
| 14 | Identity/Economy | 5 | 5 | 4 | 3 | 4 | 4 | **25/30** | Very High | 0% implemented |
| 15 | Code Intelligence | 5 | 5 | 3 | 2 | 4 | 5 | **24/30** | Medium | roko-index: Built/Unwired |
| 16 | Heartbeat | 5 | 5 | 5 | 4 | 5 | 5 | **29/30** | Very High | 0% implemented |
| 17 | Lifecycle | 5 | 5 | 5 | 5 | 4 | 5 | **29/30** | High | Partial (ProcessSupervisor) |
| 18 | Tools | 5 | 5 | 5 | 4 | 4 | 5 | **28/30** | High | roko-std: Stable |
| 19 | Deployment | 4 | 5 | 5 | 4 | 3 | 4 | **25/30** | High→Low | Native only |
| 20 | Technical Analysis | 5 | 5 | 4 | 3 | 2 | 5 | **24/30** | Very High | 0% implemented |

### Averages by criterion

| Criterion | Mean | Min | Sections at Min |
|---|---|---|---|
| rust_structs | 4.6 | 4 | 00, 02, 03, 06, 08, 10, 12, 19 |
| pseudocode | 4.5 | 3 | 08 |
| config_params | 4.6 | 3 | 08, 15 |
| **error_handling** | **3.4** | **2** | **08, 10, 15** |
| integration_wiring | 3.9 | 2 | 20 |
| test_criteria | 4.1 | 3 | 00, 02, 06, 08, 12 |

**Universal weakness: error handling** (mean 3.4). The codebase consistently specifies the happy path
with mathematical precision but under-specifies failure modes. Error enums exist sporadically but are
not systematic.

---

## Per-Section Detail

### 00 — Architecture (32 files, 21/30)

| Classification | Files |
|---|---|
| Specified | 00–09, 10, 11, 12, 13, 15, 18, 20, 21 (18 files) |
| Scaffold | 14, 16, 17, 19, 22–29 (12 files) |
| Concept only | 30 (1 file) |

**Strengths:** Engram/Score/Decay/Provenance/Kind/Body/ContentHash data types are the most fully specified
layer in the codebase. The cognitive loop (09) and five-layer taxonomy (12) have tight spec-code alignment.
60+ config params in RokoConfig schema with validation rules.

**Critical gaps:**
- Kernel readiness is now a rewrite-track decision: the current architecture is coherent as v1, but REF21 asks whether the next kernel should be replaced cleanly rather than patched in place
- Files 25–29 (Attention Currency, Cognitive Immune, Temporal Topology, Emergent Goals, Energy Model) have
  dense specifications but zero shipping code and no test criteria
- Cross-section integration map (doc 24) identifies 20 missing wiring points

**Crate reality:** roko-core is Stable with 610 tests across 59 files (~6,500 LOC). Core types (`Engram`,
6 Synapse traits, `Kind`, `Body`, `Score`, `Config`) are complete and well-tested.

---

### 01 — Orchestration (15 files, 30/30)

| Classification | Files |
|---|---|
| Specified | 00–11, 13 (13 files) |
| Scaffold | 12 (1 file) |

**Best-specified section in the entire codebase.** Every major component has concrete Rust code, test criteria,
config constants, error types, and integration wiring. Real file line counts confirm specs match code
(dag.rs=760, executor/mod.rs=719, recovery.rs=1,075).

**Strengths:** Snapshot/recovery (atomic writes, BLAKE3 hash chains, 3-level integrity verification), plan
state machine (every transition enumerated with guard conditions), CRDT merge semantics for future distributed use.

**Gaps:** CRDT/HLC not yet wired. Saga pattern specified but not built. Plan template `instantiate()`/`compose()`
unverified.

**Crate reality:** roko-orchestrator is Wired (23 files, ~3,000 LOC, 315 tests). `ParallelExecutor`,
`UnifiedTaskDag`, `ExecutorSnapshot`, `PlanStateMachine` all called from `orchestrate.rs`. Safety subsystem
(taint propagation, capability tokens, loop guard, audit chain) implemented and tested.

---

### 02 — Agents (17 files, 21/30)

| Classification | Files |
|---|---|
| Specified | 00–03, 05–07, 09, 11, 13, 14 (11 files) |
| Scaffold | 04, 08, 10, 12, 15 (5 files) |

**Most honest gap analysis of any section.** Doc 15 now correctly frames the agent gap as partial,
not absolute: the shared ToolLoop/Safety path is live for OpenAI-compatible providers, Gemini compat
models, Anthropic API, Perplexity search-grounded chat, and Gemini-native non-grounding tool-capable
models, but it is not yet universal across every backend family; role prompts are still thin, and
some known-protocol subprocess or specialty creation sites remain outside the factory path.

**Critical gaps:**
- ToolDispatcher is not yet universal across runtime paths — the 7-step safety pipeline exists and is live on routed HTTP provider families, but known-protocol subprocess paths and specialty endpoints still bypass it; the remaining no-config subprocess fallbacks are centralized in the factory instead of scattered across call sites
- Role prompts average ~20 tokens; Meta-Harness research shows harness quality dominates model quality
- LlmBackend coverage is incomplete — routed tool-loop support now also covers Gemini compat models, Anthropic API, Perplexity search-grounded chat, and Gemini-native non-grounding tool-capable models, but Gemini grounding/code-execution and Perplexity deep-research still use dedicated paths
- Temperament system fully specified but not propagated to runtime

**Crate reality:** roko-agent is Stable/Wired (97 files, ~9,500 LOC, 567 tests). Five LLM backends
(Claude CLI, Anthropic API, Gemini, Perplexity, Ollama/OpenAI-compat). Safety layer (bash, git, path,
network, rate_limit, scrub) is complete. MCP bridge wired. The ToolDispatcher gap is a wiring issue, not
a code absence issue.

---

### 03 — Composition (15 files, 25/30)

| Classification | Files |
|---|---|
| Specified | 00–06 (7 files) |
| Scaffold | 07, 08, 11, 12, 13 (5 files) |
| Concept only | 09, 10 (2 files) |

**Best mathematical specification.** Every scoring formula has explicit weights, every stopping rule has
concrete thresholds, every config has a default with rationale.

**Strengths:** VCG Attention Auction (doc 10) covers truthful bidding proofs, PoA bounds, greedy welfare
guarantees (Dantzig 1957), collusion detection. MVT Predictive Foraging (doc 09) has gain curve equations
and multi-patch binary search. Claims 83% cost reduction / 71%→94% gate pass rate backed by algorithms.

**Critical gaps:**
- Active inference EFE scorer is the highest-leverage unbuilt feature (static SectionScorer is only impl)
- VCG auction: 9/9 implementation items "Not yet"
- PAD persistence resets every session
- MVT stopping rule not applied to context assembler gather loop

**Crate reality:** roko-compose is Wired (39 files, ~4,500 LOC, 264 tests). 9 prompt templates, 6-layer
SystemPromptBuilder, token budget arithmetic, enrichment pipeline all present and called from `orchestrate.rs`.

---

### 04 — Verification (15 files, 27/30)

| Classification | Files |
|---|---|
| Specified | 00–06, 08, 15 (8 files) |
| Scaffold | 07, 09–12 (5 files) |

**Strengths:** Gate trait design principle ("returns `Verdict` not `Result<Verdict>`") is architecturally
clean. ArtifactStore with BLAKE3 content-addressing fully specified. Adaptive thresholds show EWMA, CUSUM,
and BOCPD alternatives with constants.

**Gaps:** Autonomous eval generation (doc 10), EvoSkills (doc 11), forensic replay (doc 12) are scaffold.
Process Reward Model (doc 07) has weights but no model implementation.

**Crate reality:** roko-gate is Wired (22 files, ~2,800 LOC, 216 tests). 11 gate types as real `Gate`
trait impls. `GatePipeline`, `AdaptiveThresholds`, `GateFeedback` all wired.

---

### 05 — Learning (19 files, 29/30)

| Classification | Files |
|---|---|
| Specified | 00–11, 14, 15 (14 files) |
| Scaffold | 12, 13, 16 (3 files) |
| Concept only | 17 (1 file) |

**Strengths:** `AgentEfficiency` telemetry record (28 fields) is the richest data structure in the codebase — backbone
for all learning subsystems. Cascade router 3-stage progression (Static → Confidence → UCB) with observation
thresholds is fully implementation-ready. 31.6× collective calibration derivation is mathematically grounded.

**Gaps:** The feedback-loop docs are stale: all eight "missing" loops now have at least one real code path, but several are still narrower than the PRD ideal or live at the orchestration layer instead of the most canonical subsystem boundary. The remaining work is depth, canonical placement, and consistency, not zero-to-one wiring. ADAS autocatalytic cycle (doc 17) is theoretical only.

**Crate reality:** roko-learn is Wired (36 files, ~5,000 LOC, 348 tests). Cascade router persists to disk.
Episode logger is append-only JSONL with HDC fingerprinting. Prompt experiments, adaptive gate thresholds,
cfactor, anomaly detection, drift tracking all implemented.

---

### 06 — Neuro (18 files, 22/30)

| Classification | Files |
|---|---|
| Specified | 00, 05, 09 (3 files) |
| Scaffold | 02, 03, 04, 06, 07, 10, 11, 12 (8 files) |
| Concept only | 08, 13, 14, 15 (4 files) |

**Weakest section in the audit.** The weakness concentrates in error handling (3), integration wiring (3),
and test criteria (3). Approximately half the neuro subsystem is design-only.

**Strengths:** HDC implementation (`HdcVector`, XOR bind ~5ns, Hamming similarity ~13ns) rigorously
benchmarked. False positive threshold derivation (Z-score 5.26, Bonferroni for 100K vocabulary, threshold
0.526) is exceptional. Ebbinghaus decay with tier multiplier has 4 worked examples.

**Critical gaps:**
- Tier field / PRD-native types have landed, directional causal encoding is now implemented, and the remaining lag is mostly in higher-order retrieval/runtime features plus status docs
- `ContextAssembler` base retrieval is implemented and now uses an internal auction-style allocator; the remaining gap is cross-subsystem allocation rather than the local retrieval path itself
- Somatic integration is now partial rather than absent: `roko-daimon` has a real 8D k-d-tree-backed somatic landscape wired into routing, runtime events, dream maintenance, and centralized strategy-space projection, while `ContextAssembler` already provides PAD-biased retrieval, direct somatic re-ranking, and a contrarian slice; the remaining work is true domain-native extractors plus richer cross-subsystem bidding/pricing
- Cross-domain HDC transfer entirely unimplemented
- Half-life constants now match the PRD for CausalLink (60d) and StrategyFragment (14d)

**Crate reality:** Neuro primitives has `HdcVector` (3 files, ~500 LOC, 18 tests). The knowledge store
types are spread across roko-core and the legacy agent scaffold without consolidation.

---

### 07 — Conductor (17 files, 29/30)

| Classification | Files |
|---|---|
| Specified | 00–07, 10, 11, 13, 14 (12 files) |
| Scaffold | 08, 09, 12, 15 (4 files) |

**Highest production-readiness.** Every threshold constant traces to a production failure issue number
(MAX_GHOST_TURNS=3 → Issue #9, MAX_PLAN_FAILURES=2 → Issues #3/#16). 21 production failures mapped to
conductor mechanisms.

**Strengths:** OODA loop mapping to actual code is complete. Process supervision exceptional (cgroups vs
pgrep, bottom-up kill ordering, setsid isolation). `SelfHealingConductor` specifies 4 failure modes with
detection and recovery.

**Gaps:** `ConductorBandit` built but not wired into `evaluate()`. Cognitive control cues (Pause/Resume/Reprioritize)
missing from `ConductorDecision`. L3/L4 federation architecture design-only.

**Crate reality:** roko-conductor is Wired (19 files, ~2,200 LOC, ~130 tests). All 10 watchers are real
`Policy` impls. `DiagnosisEngine` categorizes errors. `CircuitBreaker` tracks per-plan failures. Called from
`orchestrate.rs`.

---

### 08 — Chain (26 files, 18/30)

| Classification | Files |
|---|---|
| Specified | 00–03 (4 files) |
| Scaffold | 04–07, 10 (5 files) |
| Concept only | 08–09, 11–25 (17 files) |

**Lowest-scoring section.** Everything beyond `ChainClient`/`ChainWallet` traits and `mirage-rs` is
Tier 6 (fully deferred). Error handling (2) is the weakest criterion across the entire audit.

**Strengths:** mirage-rs (in-process EVM fork, 141 tests) is a genuinely rare capability. PolicyCage
smart contract has strong safety reasoning.

**Crate reality:** roko-chain is Scaffold (10 files, ~1,200 LOC, ~10 tests). Trait definitions and mocks
complete. `alloy_impl` behind feature flag. Phase 2+.

---

### 09 — Daimon (15 files, 23/30)

| Classification | Files |
|---|---|
| Specified | 00–08, 12, 13 (11 files) |
| Scaffold | 09–11 (3 files) |

**Most implementation-ready affective computing spec.** Many files have copy-pasteable Rust code. OCC/Scherer
appraisal theory correctly applied. ALMA three-layer model (emotion 2s / mood 4h / personality 720h) is
elegantly designed.

**Critical gaps:**
- Two parallel implementations (`roko-daimon` 569 LOC + the legacy scaffold copy of `daimon.rs` 972 LOC) need consolidation
- Somatic landscape is now partially implemented: `roko-daimon` has the 8D k-d-tree-backed store, runtime events, and dream-time consolidation/depotentiation; domain-extensible axis sets are still missing
- CascadeRouter now reads live Daimon behavioral state and biases model-tier selection; the remaining affect gaps are domain-extensible strategy spaces and broader cross-surface bidder production
- VCG context allocation is now partial: bidder-aware shared prompt composition exists, PAD urgency / affect weighting is wired into the live auction, and diagnostic externality payments are emitted; the remaining gap is exact welfare maximization / fairness policy plus fuller bidder coverage

**Crate reality:** Code split across the legacy agent scaffold and roko-core affect types.

---

### 10 — Dreams (19 files, 23/30)

| Classification | Files |
|---|---|
| Specified | 00, 01, 04, 05, 08, 09, 10, 12–15 (11 files) |
| Scaffold | 02, 03, 06, 07, 11, 16, 17 (7 files) |

**Strengths:** DreamRunner/DreamCycle/PatternMiner are genuinely implemented. Config completeness is
best-in-class (every parameter has default + range). 30+ academic papers cited. 15-cross-system-integration.md
is a masterclass in integration documentation.

**Critical gaps:**
- Mattar-Daw utility scoring (core of NREM replay prioritization) not implemented
- REM imagination (counterfactual generation via Pearl SCM) not implemented
- HDC counterfactual synthesis not implemented
- Error handling is weakest criterion (2/5) — implicit failure modes only

**Crate reality:** DreamRunner exists in the legacy scaffold. Core loop works but computational
heart (Mattar-Daw, REM, HDC counterfactuals, hypnagogia) is unimplemented.

---

### 11 — Safety (18 files, 27/30)

| Classification | Files |
|---|---|
| Specified | 00–09, 14–16 (13 files) |
| Scaffold | 10–13 (4 files) |

**Best integration documentation.** Doc 00 maps every safety guard to its location in the 7-step
ToolDispatcher pipeline. Doc 16 provides exact file locations, line counts, and three resolution options
for the critical wiring gap.

**Critical gap:** SafetyLayer + ToolDispatcher are built and wired to each other, but ToolDispatcher
is never invoked from orchestrate.rs. All six guards (BashPolicy, GitPolicy, NetworkPolicy, PathPolicy,
ScrubPolicy, RateLimiter) are dormant in the production code path.

**Crate reality:** Safety types live in roko-orchestrator's safety sub-module. Built and tested. The gap
is a wiring decision (subprocess interception vs settings passthrough vs in-process API dispatch).

---

### 12 — Interfaces (20 files, 23/30)

| Classification | Files |
|---|---|
| Specified | 04, 07–12 (7 files) |
| Scaffold | 00–03, 05, 06, 13–17 (12 files) |
| Concept only | 18 (1 file) |

**Strengths:** ROSEDUST design system (OKLab math, APCA contrast, 256-color quantization) is
production-grade. TUI ELM architecture (`Model`, `UpdateMsg`, `update()`, `view()`) ready to implement.
Spectre physics (Verlet, Gray-Scott, SDF) specified at direct-implementation level.

**Gaps:** Spectre not built. Web Portal not started. Sonification JavaScript-only. WebSocket bidirectional
agent control not built.

**Crate reality:** roko-cli TUI is under active development (40+ files in `tui/`). Full ratatui + crossterm
integration. F1-F7 tab system, modals, views, widgets, postfx pipeline, atmosphere animations. Recent commits
show background thread architecture for zero-I/O rendering.

---

### 13 — Coordination (14 files, 27/30)

| Classification | Files |
|---|---|
| Specified | All 14 files |

**Most academically rigorous section.** 40+ paper citations correctly applied. Every mechanism grounded in
science (Turing 1952, Kauffman 1993, Woolley 2010, Dorigo 1996). Complete Rust structs for `Pheromone`,
`PheromoneKind` (8 variants), `PheromoneScope`, `MorphogeneticState`, `MeshEnvelope`, `ByzantineDetector`,
5 pathology detectors.

**Critical reality:** **Zero code in the codebase.** Status doc explicitly states: "0 pheromone types in code,
0 transport implementations, 0 morphogenetic code, 0 collective intelligence metrics." All design exists in
docs only.

**Tier 1 tasks** (pheromone types in `roko-core`) are a few hours of work and provide immediate single-agent
value. Full multi-agent coordination (Tier 3) requires the Agent Mesh transport.

---

### 14 — Identity/Economy (17 files, 25/30)

| Classification | Files |
|---|---|
| Specified | 01–15 (15 files) |
| Concept only | 00 (pitch doc) |

**Strengths:** Solidity contracts complete enough to deploy (IdentityRegistry, ReputationRegistry,
ValidationRegistry). Economic mechanism proofs (Vickrey incentive compatibility, LMSR market maker) are
rigorous. W3C DID/VC integration specified to the standard.

**Reality:** Entire section is Deferred (Tier 5-6). No Rust code in any existing crate. Requires a new
L3 blockchain (Korai chain on Base). Phase 3+.

---

### 15 — Code Intelligence (12 files, 24/30)

| Classification | Files |
|---|---|
| Built | 01–05 (5 files) |
| Scaffold | 06–09 (4 files) |
| Specified | 00, 10 (2 files) |

**Only section with significant built and tested code** (roko-index: 5 files, ~700 LOC, 32 tests +
3 lang providers: ~2,339 LOC combined, 92 tests). `LanguageProvider` trait clean and extensible. HDC
fingerprints provide sub-microsecond similarity search. PageRank is correct and converges.

**Critical gap:** `roko-index` is a standalone library with no consumer. Not called from `orchestrate.rs`
or any `ContextProvider`. No `CodeIndex` trait. No search API. No error handling (functions return structs
directly, no `Result`).

**Highest-leverage near-term opportunity:** Wiring roko-index + lang providers into the context assembly
pipeline would give agents code-aware context for free.

---

### 16 — Heartbeat (14 files, 29/30)

| Classification | Files |
|---|---|
| Specified | All 13 content files |

**Most mathematically rigorous section.** T0 probe system (16 probes, ~80% tick suppression) specified
with exact cost budgets. VCG attention auction has formal truthfulness proof. `CorticalState<const N: usize>`
const generics are elegant.

**Reality:** No existing Rust implementation. Every file is a spec requiring a new crate/module.

---

### 17 — Lifecycle (14 files, 29/30)

| Classification | Files |
|---|---|
| Specified | All 13 content files |

**Strengths:** PhantomData type-state provisioning eliminates entire classes of runtime errors at
compile time. Error handling is the strongest of any section (5/5) — full taxonomies for
`LifecycleError`, `ProvisioningError`, `FundingError`, `BackupError`.

**Gaps:** Korai funding integration deferred. Type-state provisioning not wired to `roko init`.
GitOps depends on daemon mode (Section 19 scaffold).

---

### 18 — Tools (19 files, 28/30)

| Classification | Files |
|---|---|
| Shipping | 00–02, 04, 05, 07, 09 (7 files) |
| Built | 03 (1 file) |
| Scaffold | 08, 10–13 (5 files) |
| Specified | 06, 14–16 (4 files) |

**Strengths:** `Capability<T>` token system is security-by-construction (unforgeable, single-use,
compile-time). MCP client is actually built. 18 agent templates immediately usable. 4-layer tool testing
with 66 eval tests.

**Gaps:** MCP servers scaffold. Plugin SDK specified but not in any crate. WASM plugins designed but
no `wasm32` target validated.

**Crate reality:** roko-std is Stable (33 files, ~3,500 LOC, ~120 tests). 16 builtin tools, role profiles,
mock dispatcher. Golden tool tests verify schema stability.

---

### 19 — Deployment (15 files, 25/30)

| Classification | Files |
|---|---|
| Working | 01 (native builds) |
| Specified | All others (12 files) |

**Strengths:** Status doc is exceptionally honest. Production hardening (adaptive timeouts, retry with
full jitter) partially implemented in roko-agent. Subscription config schema elegant.

**Gaps:** All deployment infrastructure (Dockerfiles, fly.toml, daemon mode, roko-serve) exists only
as documentation. Zero actual files created. But Tier 1 (Docker, deploy scripts) could be created
in 2-3 days.

---

### 20 — Technical Analysis (16 files, 24/30)

| Classification | Files |
|---|---|
| Specified | All 14 content files |

**Most intellectually ambitious section.** Covers RSI/MACD → HDC → Riemannian manifolds → TDA → sheaf
cohomology → tropical geometry in a coherent design. Cross-domain isomorphism argument is compelling:
the same 6 mathematical structures appear in chain, coding, and research oracles.

**Critical gap:** Integration wiring (2/5) — the weakest criterion in the entire audit. Oracle trait does
not exist in any crate. Error types for `Result<Prediction>` unspecified. None of ChainOracle/CodingOracle/
ResearchOracle exist in the codebase.

---

## Crate Implementation Status

Cross-referencing docs against actual crate code:

| Crate | Files | LOC | Tests | CLI Wired? | Maturity |
|---|---|---|---|---|---|
| roko-core | 59 | ~6,500 | 610 | Yes (kernel) | **Stable** |
| roko-agent | 97 | ~9,500 | 567 | Yes | **Stable/Wired** |
| roko-orchestrator | 23 | ~3,000 | 315 | Yes | **Wired** |
| roko-gate | 22 | ~2,800 | 216 | Yes | **Wired** |
| roko-compose | 39 | ~4,500 | 264 | Yes | **Wired** |
| roko-conductor | 19 | ~2,200 | ~130 | Yes | **Wired** |
| roko-learn | 36 | ~5,000 | 348 | Yes | **Wired** |
| roko-cli | 101 | ~12,000 | ~300 | Entry point | **Stable/Wired** |
| roko-fs | 12 | ~1,800 | ~60 | Yes | **Stable** |
| roko-std | 33 | ~3,500 | ~120 | Yes | **Stable** |
| `roko-runtime` | 6 | ~900 | ~12 | Yes | **Stable** |
| `roko-primitives` | 3 | ~500 | 18 | Yes | **Stable** |
| roko-index | 5 | ~700 | 32 | **No** | Built/Unwired |
| roko-lang-rust | 1 | ~820 | 37 | **No** | Built/Unwired |
| roko-lang-typescript | 1 | ~918 | 31 | **No** | Built/Unwired |
| roko-lang-go | 1 | ~601 | 24 | **No** | Built/Unwired |
| Legacy agent scaffold | 7 | ~600 | 3 | **No** | Scaffold |
| roko-chain | 10 | ~1,200 | ~10 | **No** | Scaffold |

**Key finding:** 12 of 18 crates are Stable/Wired. The remaining 6 break into two categories:
- **Built/Unwired** (roko-index + 3 lang providers): Complete, tested code with no consumer
- **Scaffold** (legacy agent scaffold, roko-chain): Phase 2+ placeholder code

---

## Prioritized Gap List

### Rewrite-track decisions

These are not ordinary backlog items. They are choice points that change the interpretation of the rest of the audit.

| # | Candidate | What changes | Unlocks | Cost | Audit stance |
|---|---|---|---|---|---|
| R1 | roko-core kernel | Replace the single-medium kernel with Engram + Pulse, add Bus as a first-class fabric, and generalize the operator surface | Coherent v2 kernel, cleaner dependency graph, unified operator signatures | 2–3 weeks, ~15 consumer crates | Yes |
| R2 | roko-learn reorganization | Split the monolith into episode, playbook, bandit, experiment, and heuristic crates; make learning subscription-driven | Cleaner dep graph, independent strategy swaps, easier plugin contributions | Significant churn, ~3K lines, ~2 weeks | Yes, after R1 |
| R3 | Substrate trait rewrite | Expand storage from get/put/delete/list to put/get/query/scan/freeze/thaw | HDC similarity query, cold tier semantics, uniform storage backends | ~1 week | Yes |
| R4 | Gate pipeline | Replace the state machine with pure gates plus composition combinators | Third-party gates, easier tests, inspectable composition data | 1–2 weeks | Maybe; schedule only if the payoff is clear |
| R5 | roko-compose engine | Replace the fixed template builder with a query-driven compose pipeline | Situation-specific prompts, HDC retrieval, template-as-data composition | ~2 weeks | Yes, after R3 |

The core distinction is readiness type: these candidates are rewrite-ready, not merely gap-ready. The audit below still matters, but it no longer tells the whole story for the affected sections.

### Tier 0 — Critical Path to Self-Hosting

| # | Gap | Section | Complexity | Impact | Blocking |
|---|---|---|---|---|---|
| G1 | **Wire ToolDispatcher into orchestrate.rs** | 02, 11 | Medium | Safety coverage jumps 30%→100% | Safety layer is dead code |
| G2 | **Wire roko-index into ContextProvider** | 15 | Medium | Code-aware context assembly | Agents lack code intelligence |
| G3 | **Register lang providers in detect_polyglot** | 15 | Low | Multi-language symbol extraction | roko-index has no lang support |
| G4 | **Expand role prompts from ~20 to ~2,000 tokens** | 02 | Medium | Largest single performance lever | Harness quality < model quality |
| G5 | **Implement somatic landscape + VCG affect bidding** | 09 | Medium | Complete Daimon control loop | Affect still lacks fast somatic lookup and full auction integration |

### Tier 1 — High-Leverage Improvements

| # | Gap | Section | Complexity | Impact |
|---|---|---|---|---|
| G6 | Implement active inference EFE scorer | 03 | Medium-High | Replace static SectionScorer |
| G7 | 8 missing feedback loops (wiring recipes) | 05 | Medium (~495 LOC) | Close the learning loop |
| G8 | Normalize kernel vocabulary across the architecture docs | 00 | Low (find/replace) | Eliminate spec/code terminology drift |
| G9 | Wire PAD persistence to `.roko/daimon/affect.json` | 03, 09 | Low-Medium | Cross-session emotional continuity |
| G10 | Add tier field to `KnowledgeEntry` | 06 | Low | Enable tier-weighted decay |
| G11 | Fix half-life constants (30d defaults → spec values) | 06 | Low | Correct knowledge decay rates |
| G12 | Consolidate `roko-daimon` + the legacy scaffold copy of `daimon.rs` | 09 | Medium | Prerequisite for Daimon features |
| G13 | Close safety critical integration gap | 11 | Medium | Activate all 6 safety guards |
| G14 | Create Dockerfiles + fly.toml | 19 | Low (2-3 days) | Enable cloud deployment |
| G15 | Implement Mattar-Daw utility scoring | 10 | Medium | Enable NREM replay prioritization |

### Tier 2 — Feature Enrichment

| # | Gap | Section | Complexity | Impact |
|---|---|---|---|---|
| G16 | VCG Attention Auction | 03, 16 | High | Optimal context allocation |
| G17 | MVT Predictive Foraging | 03 | Medium-High | Reduce unnecessary source queries |
| G18 | Somatic landscape k-d tree | 09 | High | Affect-aware strategy retrieval |
| G19 | Pheromone types in roko-core | 13 | Low | Foundation for coordination |
| G20 | CodeIndex trait + search API | 15 | Medium | Unified code intelligence API |
| G21 | Oracle trait + PredictionStore | 20 | Medium | Enable prediction loop |
| G22 | MCP servers (GitHub, Slack) | 18 | Medium | Service integration |
| G23 | Daemon mode (roko daemon install) | 19 | High | Background operation |
| G24 | CorticalState + T0 probes | 16 | High | ~80% tick suppression |
| G25 | Plugin SDK (hooks, Integration) | 18 | High | Third-party extensibility |

### Tier 3 — Advanced / Phase 2+

| # | Gap | Section | Complexity | Impact |
|---|---|---|---|---|
| G26 | Agent Mesh transport (P2P) | 13 | Very High | Multi-agent coordination |
| G27 | REM counterfactual generation (Pearl SCM) | 10 | Very High | Causal reasoning in dreams |
| G28 | Causal discovery + mirage-rs integration | 20 | Very High | Intervention-based prediction |
| G29 | Sheaf/tropical geometry | 20 | Very High | Algebraic foundation for robustness |
| G30 | Korai chain deployment (L3 on Base) | 08, 14 | Very High | On-chain agent economy |
| G31 | Spectre creature visualization | 12 | Very High | Embodied agent visualization |
| G32 | 4-level conductor federation | 07 | Very High | Distributed safety oversight |
| G33 | Witness DAG with ZK proofs | 11 | Very High | Verifiable execution history |

---

## Estimated Implementation Effort

### By section (person-weeks for remaining work)

| Section | Core Wiring | Advanced Features | Total |
|---|---|---|---|
| 00 Architecture | 1 (kernel cutover decision) | 8 (docs 25-29) | 9 |
| 01 Orchestration | 0 (done) | 4 (CRDT, saga) | 4 |
| 02 Agents | 2 (G1, G4) | 6 (HTTP backends, temperament) | 8 |
| 03 Composition | 1 (G6 base) | 8 (VCG, MVT, HDC dedup) | 9 |
| 04 Verification | 0 (done) | 6 (eval gen, EvoSkills, forensic) | 6 |
| 05 Learning | 2 (G7) | 4 (ADAS, TrackAndStop) | 6 |
| 06 Neuro | 2 (G10, G11) | 12 (somatic, cross-domain, library) | 14 |
| 07 Conductor | 0 (done) | 6 (cognitive control cues, L3/L4) | 6 |
| 08 Chain | 0 (deferred) | 24+ (full DeFi stack) | 24+ |
| 09 Daimon | 2 (G5, G12) | 6 (somatic landscape, contrarian) | 8 |
| 10 Dreams | 1 (G15) | 10 (REM, HDC counterfactual, hypnagogia) | 11 |
| 11 Safety | 1 (G13) | 12 (witness DAG, CaMeL, formal) | 13 |
| 12 Interfaces | 3 (TUI completion) | 16 (Spectre, Portal, A2UI) | 19 |
| 13 Coordination | 1 (G19) | 16 (mesh, morphogenetic, pathology) | 17 |
| 14 Identity/Economy | 0 (deferred) | 24+ (blockchain + DeFi) | 24+ |
| 15 Code Intelligence | 2 (G2, G3, G20) | 4 (SQLite, MCP server) | 6 |
| 16 Heartbeat | 0 | 12 (POMDP, VCG, probes) | 12 |
| 17 Lifecycle | 1 | 4 (type-state pipeline) | 5 |
| 18 Tools | 1 (G22) | 8 (plugin SDK, WASM) | 9 |
| 19 Deployment | 1 (G14) | 6 (daemon, roko-serve) | 7 |
| 20 Technical Analysis | 0 | 18+ (oracles, causal, sheaf) | 18+ |

### Summary

| Category | Effort | ROI |
|---|---|---|
| **Tier 0 (G1-G5)** | ~4 person-weeks | Immediate — unblocks self-hosting quality |
| **Tier 1 (G6-G15)** | ~8 person-weeks | High — closes feedback loops, fixes data integrity |
| **Tier 2 (G16-G25)** | ~20 person-weeks | Medium — feature enrichment |
| **Tier 3 (G26-G33)** | ~50+ person-weeks | Long-term — Phase 2+ advanced capabilities |

These estimates assume incremental delivery. The rewrite-track candidates above are cutover efforts, not ordinary backlog items, so their budgets should be read as sequencing and governance costs rather than feature-closure costs.

---

## Documentation Quality Observations

### Systemic strengths

1. **Self-aware status docs.** Sections 02, 06, 09, 10, 12, 13, 15, 19 each contain a "current status and
   gaps" document with honest inventories. These are the most valuable documents in the corpus.

2. **Academic grounding.** Every section cites primary research. The citations are correctly applied to
   design decisions (not decorative). Best: 13-coordination (40+ papers), 10-dreams (30+ papers),
   05-learning (UCB1, LinUCB, Thompson sampling with proper equations).

3. **Config completeness.** 17 of 21 sections score 4+ on config_params. `roko.toml` schema coverage is
   extensive with validation rules, env var mappings, and TOML blocks.

4. **Mathematical precision.** Sections 01, 03, 05, 07, 13, 14, 16, 20 have formulas specified at
   paper-quality precision with worked examples.

### Systemic weaknesses

1. **Error handling under-specified.** Mean 3.4/5. The happy path is always precise; failure modes are
   often implicit. Only sections 01, 07, 17 score 5/5 on errors.

2. **Integration wiring gaps.** The "built but not wired" pattern appears in: ToolDispatcher (02/11),
   roko-index (15), lang providers (15), ConductorBandit (07), CascadeRouter→Daimon (09), PAD
   persistence (03/09). This is the codebase's central failure mode.

3. **Test criteria for advanced features.** Core components have good test specs. Advanced features
   (VCG auction, MVT foraging, sheaf geometry, causal discovery) often have no test criteria at all.

4. **Phase boundary ambiguity.** Some sections mix immediately implementable features with Phase 2+
   aspirations without clear separation. Sections 08 and 14 handle this well (explicit Tier labels);
   sections 06 and 12 less so. REF21 adds a second ambiguity: some sections are not just mixed-phase,
   they are candidates for from-scratch rewrite decisions, so the audit needs both incremental and
   rewrite-track reading modes.

---

## Recommended Execution Order

REF21 splits the roadmap into two tracks. The incremental order below applies only if the team explicitly declines the rewrite-track candidates for the affected subsystems.

### Rewrite track

1. Week 1: roko-bus extraction.
2. Weeks 2-4: roko-core kernel rewrite.
3. Weeks 4-5: Substrate trait rewrite.
4. Week 5: docs rewrite for the architecture chapter.
5. Weeks 6-7: roko-learn reorganization.
6. Weeks 8-9: roko-compose engine rewrite.
7. Week 10: gate pipeline rewrite, if still worth it.

### Incremental track

```
Week 1-2:  G1 (ToolDispatcher wiring) + G2/G3 (code intelligence wiring) + G5 (Daimon→CascadeRouter)
Week 2-3:  G4 (role prompt expansion) + G8 (kernel vocabulary cleanup) + G10/G11 (knowledge entry fixes)
Week 3-4:  G7 (8 feedback loops) + G9 (PAD persistence) + G14 (Docker/Fly)
Week 4-6:  G6 (EFE scorer) + G12 (daimon consolidation) + G13 (safety gap closure)
Week 6-8:  G15 (Mattar-Daw) + G19 (pheromone types) + G20 (CodeIndex trait)
Week 8-12: G16 (VCG auction) + G21 (Oracle trait) + G22 (MCP servers) + G24 (T0 probes)
```

After weeks 1-4 on the incremental track, roko reaches **full self-hosting with safety**: agents have code-aware context, safety guards are active, behavioral state routes to appropriate models, knowledge decay is correct, and the learning loop is closed.

After weeks 4-8 on the incremental track, roko reaches **intelligent self-hosting**: active inference scores context, daimon modulates behavior, dreams prioritize by utility, and coordination primitives exist.

After weeks 8-12 on the incremental track, roko reaches **optimal self-hosting**: attention is auctioned, predictions feed back into routing, and probes suppress 80% of unnecessary computation.
