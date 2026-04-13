# Deep Architectural Gaps: Beyond Naming

The docs specify far more than naming changes. This document captures every structural, compositional, and algorithmic gap between the PRD docs and the current code.

---

## 1. CORE TYPE SYSTEM (roko-core)

### Score: 4-axis → 7-axis
- **Code has**: confidence, novelty, utility, reputation
- **Docs add**: precision, salience, coherence (3 extended axes)
- **Formula**: unchanged (`confidence × (1 + novelty) × (1 + utility) × reputation`)
- **Impact**: Score struct extension, scorer implementations, anywhere Score is constructed

### Signal/Engram: Attestation Integration Is Only Partially Closed
- **Code now has**: `attestation: Option<Attestation>` in `roko-core::Engram`, plus core attestation types and serde support
- **Remaining gap**: signing, verification, and chain-witness workflows are still not wired
- **Impact**: chain integration, forensic verification, mesh trust

### Lineage: Partially Populated, Not End-to-End
- `Engram.lineage: Vec<ContentHash>` exists
- `derive()` exists to construct child Engrams
- `roko-compose::PromptComposer` already populates prompt output lineage from kept inputs
- **Remaining gap**: many runtime-emitted Engrams still do not preserve upstream lineage consistently across dispatch, policy, and persistence flows

---

## 2. ORCHESTRATION (roko-orchestrator)

### DAG Optimization Passes — ALL MISSING
The docs specify 4 optimization algorithms for UnifiedTaskDag that don't exist:

| Algorithm | What It Does | Current |
|-----------|-------------|---------|
| **CPM analysis** | Critical Path Method with forward/backward pass | Not implemented |
| **Task fusion** | Merge linear task chains to reduce scheduling overhead | Not implemented |
| **Speculative execution** | Spark-style speculative task execution for slow tasks | Not implemented |
| **Graph partitioning** | METIS-inspired partitioning for parallel execution | Not implemented |

Current `UnifiedTaskDag` only has: `topological_sort()`, `waves()`, `stats()`

### Incremental DAG Computation — MISSING
- `IncrementalDag` struct with dirty/clean propagation
- Durability levels (Low/Medium/High) for selective invalidation
- Build-system-style incremental recomputation

### Dynamic DAG Mutation During Execution — MISSING
```rust
pub enum DagMutation {
    AddTask { ... },      // auto-fix agent adds remediation task
    RemoveTask { ... },   // conductor removes redundant task
    SplitTask { ... },    // conductor splits oversized task
    AddDependency { ... },
    UpdateTaskMetadata { ... },
}
```
Currently DAGs are immutable once created. Docs specify live mutation with consistency invariants.

---

## 3. AGENT SYSTEM (roko-agent)

### Agent Composition Operators — MISSING
- `CompositeAgent` — merges multiple agents via skill library
- `AgentComposition` enum: Pipeline, Parallel, Conditional, MixtureOfAgents
- `MergeStrategy` — concatenate, aggregate, vote, best-of-N
- `SkillSelector` — semantic similarity + transition graph

### Agent Introspection & Metacognition — MISSING
- `AgentIntrospection` — self-inspection capability
- `AgentIdentity` — role, model tier, temperament, capabilities
- `MetacognitiveMonitor` — watches agent for failure patterns
- `Intervention` enum — escalate model, human handoff, abort, inject reflection

### Supervision Strategy (Erlang/OTP) — MISSING
```rust
pub enum SupervisionStrategy {
    OneForOne { max_restarts, within_ms, fallback_tier },
    OneForAll { max_restarts },
    RestForOne { max_restarts },
}
```
Maps Erlang restart strategies to plan execution recovery.

### Capability-Based Security (OCaps) — MISSING
- `AgentWarrant` — cryptographic unforgeable capability tokens
- `Capability` enum — Tool, ReadPath, WritePath, Exec, Network
- Current model is RBAC via `AgentRole`, docs specify OCaps with delegation chains

### Agent Metamorphosis — MISSING
- `MorphableAgent` — dynamic role switching during execution
- `RoleProfile` — clarity, differentiation, alignment scores
- Allowed transitions matrix to prevent unsafe role morphs

### Provider: Automatic Selection — MISSING
- `TaskRequirements` struct — what a task needs from a provider
- `score_model_for_task()` — score provider-model pairs against requirements
- `select_model_for_task()` — automatic selection based on capabilities + cost
- Current: manual model selection via `model_hint` in tasks.toml

### LlmBackend: HTTP Implementations — MISSING
- `LlmBackend` trait exists, only `OllamaLlmBackend` implements it
- **Missing**: `OpenAiCompatBackend`, `AnthropicApiBackend`
- These are needed for HTTP API providers to use the ToolLoop (tool-calling loop)

---

## 4. NEURO (Knowledge Management)

### Knowledge Tier Field — IMPLEMENTED
KnowledgeEntry now has a `tier` field:
- Transient (0.1× strength, minutes-hours half-life)
- Working (0.5×, hours-days)
- Consolidated (1.0×, days-weeks)
- Persistent (5.0×, weeks-months)

`effective_half_life = base_half_life × tier_multiplier` is implemented.

### Knowledge Type Reconciliation — IMPLEMENTED
- Canonical `KnowledgeKind` now matches the docs: `Insight`, `Heuristic`, `Warning`, `CausalLink`, `StrategyFragment`, `AntiKnowledge`
- Legacy names (`Fact`, `Procedure`, `Playbook`, `Constraint`) were retired from code and preserved as serde aliases so historical JSONL still deserializes
- Distiller prompts, tier progression, dream synthesis, context assembly, and CLI retrieval were updated to emit/query the PRD-native kinds

### ContextAssembler — IMPLEMENTED / CANONICALIZED
- The canonical `ContextAssembler`, `ContextChunk`, `PadState`, `ContextSource`, and task input primitives now live in `roko-neuro`
- `roko-compose` re-exports the neuro-owned assembler instead of carrying a separate implementation
- `ContextAssembler::compress()` now performs auction-style budget arbitration instead of simple rank-and-truncate: chunks compete under token cost, repeated source families get diminishing returns, and low-marginal-value chunks stop winning before the budget is exhausted
- The remaining retrieval gaps are higher-order behavior: somatic / active-inference modulation beyond the current PAD biasing and the still-missing cross-subsystem VCG auction

### HDC Encoding on Ingest — IMPLEMENTED
- With the `hdc` feature enabled, `KnowledgeStore::ingest()` persists an `hdc_vector` for new entries at write time
- Stored vectors are reused by `MemoryIndex` when present instead of always recomputing from raw content
- Neuro now routes HDC generation through a dedicated encoder path, so type-specific encodings can evolve without further coupling `knowledge_store.rs` to HDC internals

### CausalLink Permutation Binding — IMPLEMENTED
- CausalLinks now encode directional structure through permuted cause/effect role bindings plus an ordered `causal_edge` binding
- Query vectors probe both cause and effect roles, so CausalLinks can be recalled from either side of the relationship
- Structured tags such as `cause:...`, `effect:...`, `domain:...`, `strength:...`, and `condition:...` are used when present, with free-text causal parsing as a fallback

### Tier Promotion State Machine
- `TierProgression` struct exists with D1/D2/D3 stages
- But no automated trigger: gate verdicts should drive promotion/demotion

---

## 5. DAIMON (Affect Engine)

### SomaticLandscape — NOT IMPLEMENTED
- k-d tree over 8D strategy space (complexity, risk, novelty, confidence, time_pressure, scope, reversibility, dependency_depth)
- `SomaticMarker` structs enabling <1ms similarity queries
- Used for "gut feeling" fast pattern matching from past experience

### Behavioral State Classification — IMPLEMENTED
- Shared `BehavioralState` now lives in `roko-core` with explicit `classify(pad, confidence)` logic
- `roko-daimon::AffectState` persists the discrete state directly and refreshes it on decay/appraisal/query
- Routing modulation still remains simple, but the state model itself is no longer implicit

### Mood-Congruent Retrieval — PARTIAL
- `ContextAssembler` now biases retrieval with `PadState` and reserves a contrarian slice of knowledge entries so negative mood does not collapse into pure caution and positive mood does not collapse into pure optimism
- `Engram` now supports optional `EmotionalTag` metadata, so retrieval has a canonical emotional provenance field to build on
- The richer Somatic Landscape design is still missing: there is no 8D somatic marker space and no k-d-tree retrieval path

### EmotionalTag on Engrams — PARTIAL
```rust
pub struct EmotionalTag {
    pub pad: PadVector,
    pub intensity: f32,
    pub trigger: String,
    pub mood_snapshot: PadVector,
}
```
- `roko-core::Engram` now carries `Option<EmotionalTag>`
- `roko-cli` currently emits conductor engrams with live Daimon emotional metadata and preserves tags when deriving conductor signals
- Broader propagation across episodes, Neuro distillation, and retrieval weighting is still incomplete

### VCG Auction for Context Budget — PARTIAL
- Inside Neuro, context chunks now compete via an auction-style allocator with token-cost awareness and a marginal-value stopping rule
- The full Vickrey-Clarke-Groves mechanism is still not wired across subsystems: Neuro, Daimon, iteration memory, code intelligence, playbooks, research, task context, and oracles are not yet bidding in one shared market

### Daimon → CascadeRouter Integration — PARTIAL
- Affect state already modulates dispatch through `DispatchParams` and `RoutingContext.affect_confidence`
- CascadeRouter already biases toward stronger models at low affect confidence
- **Remaining gap**: deeper behavioral-state-aware routing is still not explicit, and Daimon state is not modeled as a first-class routing policy object

---

## 6. DREAMS (Offline Consolidation)

### NREM Replay Modes — SCAFFOLD ONLY
Four replay modes specified, not implemented:
- Random, Consequence (high-reward), Causal (failure chains), Hypothetical (what-if)
- Mattar-Daw utility formula for replay prioritization

### REM Imagination — NOT IMPLEMENTED
- Pearl structural causal models for counterfactual reasoning
- GIRL trust-region constraints on counterfactual plausibility
- Boden 3-mode creativity (combinational, exploratory, transformational)

### Hypnagogia Engine — NOT IMPLEMENTED
- Thalamic Gate, Executive Loosener, Dali Interrupt, Homuncular Observer
- Four-layer system for sleep-onset creativity via stochastic resonance
- Currently exists as placeholder in roko-golem, not moved to roko-dreams

### Threat Simulation — NOT IMPLEMENTED
- FMEA/FTA systematic threat enumeration
- Severity assessment (CVSS/DREAD-style scoring)

### Sleep-Time Compute — NOT IMPLEMENTED
- Lin et al. 2025 mechanism for budget-aware dream scheduling
- Documented ~5× reduction in test-time cost with 13-18% accuracy improvement

### Dream Scheduling — NOT IMPLEMENTED
- Three trigger types: idle (gap > threshold), scheduled (cron), manual
- Frequency adaptation based on learning signal quality

---

## 7. LEARNING SYSTEM (roko-learn)

### EWC Regularizer — NOT IMPLEMENTED
- Elastic Weight Consolidation for bandit arms
- Prevents catastrophic forgetting when updating model routing weights

### Episode Importance Scoring — NOT IMPLEMENTED
- Surprisal + Novelty + Difficulty + Information Gain + Diversity
- Currently all episodes weighted equally in pattern discovery

### Curriculum Learning — NOT IMPLEMENTED
- `DifficultyModel` with task ordering (EasyFirst/HardFirst/Interleaved/Adaptive)
- Agent gets progressively harder tasks as skills improve

### Learning Rate Scheduling — NOT IMPLEMENTED
- Per-subsystem phase multipliers (cold/warm/mature rates)
- Currently all subsystems learn at same constant rate

### Meta-Learning for Tool Use — NOT IMPLEMENTED
- `ToolUsageProfile` tracking tool sequences correlated with success
- Tool sequence patterns per (role, task_category)

### Episode Tiering — NOT IMPLEMENTED
- Hot (recent, full detail), Warm (zstd compressed), Cold (HDC superposition)
- Currently single flat `episodes.jsonl`

---

## 8. COORDINATION (Entirely Missing)

### Pheromone System — 0 CODE
- `Pheromone` struct, `PheromoneKind` (7 types), `PheromoneScope` (3 levels)
- Exponential decay: `intensity × e^(-λt)`
- Reputation-weighted confirmation extends half-life
- Pheromone-enriched context composition

### Agent Mesh Transport — 0 CODE
- WebSocket relay + Iroh P2P
- Version vector deduplication (Lamport/Fidge clocks)

### Morphogenetic Specialization — 0 CODE
- Turing reaction-diffusion dynamics for emergent role differentiation
- Strategy vectors, niche competition heuristics

### C-Factor Collective Intelligence — 0 CODE
- Composite C-Score from 4 diagnostics
- Collective pathology detection (cascades, groupthink, echo chambers, deadlock, hallucination)

---

## 9. HEARTBEAT (Gamma/Theta/Delta)

### Only Gamma Exists
- Current orchestration loop is a simplified Gamma (reactive task execution)
- **Theta loop** (periodic reflection, ~75s): MetaCognitionHook exists but not called periodically
- **Delta loop** (consolidation, hours): Dreams crate exists but not integrated into heartbeat

### Adaptive Clock — NOT IMPLEMENTED
- `CorticalState` (32-signal atomic struct) — shared perception surface
- `CognitiveSignal` (8 typed interrupts) — cross-cut dispatch mechanism
- `FrequencyScheduler` — adapts tick rate based on regime detection
- None of these exist in code

### T0 Probes — 0 OF 16 IMPLEMENTED
16 zero-LLM probes that suppress LLM calls ~80% of the time:
1. config_changed, 2. gate_failed_recently, 3. file_modified, 4. test_count_delta,
5. compile_error_new, 6. budget_threshold, 7. confidence_dropping, 8. prediction_violation,
9. tool_health_degraded, 10. pheromone_detected, 11. task_deadline_near, 12. idle_timeout,
13. knowledge_stale, 14. dependency_changed, 15. metric_anomaly, 16. heartbeat_timeout

### Active Inference POMDP — NOT IMPLEMENTED
- Factorized 90-state POMDP for tier selection
- Currently using heuristic thresholds in CascadeRouter

---

## 10. SAFETY — BUILT BUT DORMANT

### Critical Integration Gap
6 safety guards fully implemented (~1355 lines, 50+ tests), **never called from orchestrate.rs**:
- BashPolicy: deny patterns (rm -rf, sudo, etc.)
- GitPolicy: force push, hard reset, branch deletion blocks
- NetworkPolicy: RFC1918, link-local, loopback denial
- PathPolicy: worktree sandbox via canonicalization
- ScrubPolicy: 9 regex patterns (API keys, JWTs, etc.)
- RateLimiter: sliding-window per (role, tool)

`ToolDispatcher` is imported in orchestrate.rs but never instantiated. Safety is completely dormant.

---

## 11. COMPOSE — MISSING INTEGRATIONS

### No Pheromone Enrichment
- SystemPromptBuilder assembles 6-7 layers but no pheromone summary layer
- ContextProvider doesn't query pheromone field for threat/opportunity signals

### No Affect-Biased Context
- Daimon PAD state exists independently but doesn't bias context selection
- No mood-congruent retrieval in ContextAssembler

### No Active Inference Scoring
- Composer takes scorer as parameter (correct)
- But no EFE-based (Expected Free Energy) scoring strategy implementation

---

## 12. LIFECYCLE & DEPLOYMENT — MINIMAL

### Agent Deletion — INCOMPLETE
- 8-step clean shutdown specified in docs, not fully implemented
- Knowledge backup/restore with 0.85^N confidence decay — not implemented

### Daemon Mode — NOT IMPLEMENTED
- launchd (macOS) and systemd (Linux) unit file generation specified
- `sd_notify` watchdog integration specified
- No implementation

### WASM Deployment — NOT IMPLEMENTED
- Target modules specified (Engram, Score, Router, Composer)
- `roko-wasm` crate doesn't exist

---

## SUMMARY: Refactoring Scope by Category

| Category | Items | Type |
|----------|-------|------|
| **Naming/Mechanical** | 5 | Crate renames, type renames, metadata updates |
| **Type System Extensions** | 4 | 7-axis score, attestation, emotional tags, knowledge tiers |
| **Missing Algorithms** | 12 | CPM, task fusion, speculative exec, NREM replay, REM imagination, EWC, curriculum learning, etc. |
| **Missing Subsystems** | 6 | Pheromone field, Agent Mesh, Morphogenetics, T0 Probes, Heartbeat Theta/Delta, Adaptive Clock |
| **Missing Integrations** | 8 | Safety→orchestrator, Daimon→CascadeRouter, Neuro→Compose, Dreams→Heartbeat, etc. |
| **Missing Agent Patterns** | 5 | Composition, introspection, metamorphosis, OCaps, supervision strategies |
| **Missing Infrastructure** | 4 | HTTP LlmBackends, daemon mode, WASM, lifecycle management |

Total: ~44 distinct implementation gaps beyond naming changes.
