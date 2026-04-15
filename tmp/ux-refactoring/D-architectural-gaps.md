# Section D: Architectural Gaps

Source: `tmp/integrate-prds/08-DEEP-ARCHITECTURAL-GAPS.md`
44 items across 12 subsystems. 7 implemented, 10 partial, 27 not implemented.

---

## D.01 — Score: 7-Axis Expansion

**Status**: DONE
**Priority**: —
**Files**: `crates/roko-core/src/score.rs`

Score model already has: confidence, novelty, utility, reputation, precision, salience, coherence. No work needed.

---

## D.02 — Engram Attestation Integration

**Status**: PARTIAL
**Priority**: P2
**Estimated LOC**: ~80
**Dependencies**: D.44 (chain infrastructure)

### Files to modify

- `crates/roko-core/src/attestation.rs` — Signing + verification logic
- `crates/roko-core/src/engram.rs` — `attestation: Option<Attestation>` field (exists)
- `crates/roko-chain/src/` — Chain witness workflows

### Context

`Attestation` type and `Option<Attestation>` field on `Engram` exist. Missing: signing, verification, and chain-witness workflows. Needed for forensic verification and mesh trust.

### Implementation details

1. In `attestation.rs`, implement:
   - `fn sign(engram: &Engram, key: &SigningKey) -> Attestation` — Ed25519 signature over engram content hash
   - `fn verify(engram: &Engram, attestation: &Attestation) -> bool` — verify signature against public key
2. In `roko-chain`, add chain witness:
   - `fn witness_on_chain(attestation: &Attestation, chain_client: &ChainClient) -> TxHash` — write attestation hash to chain
   - `fn verify_on_chain(attestation: &Attestation, chain_client: &ChainClient) -> bool` — check chain for matching hash
3. Wire into orchestrate.rs: optionally attest gate-passed engrams

### Verify command

```bash
cargo build -p roko-core -p roko-chain 2>&1 | tail -5
cargo test -p roko-core --lib -- attestation 2>&1 | tail -10
```

---

## D.03 — Lineage End-to-End

**Status**: PARTIAL
**Priority**: P2
**Estimated LOC**: ~60
**Dependencies**: None

### Files to modify

- `crates/roko-core/src/engram.rs` — `lineage: Vec<ContentHash>` (exists)
- `crates/roko-cli/src/orchestrate.rs` — Preserve lineage in dispatch/policy/persistence flows
- `crates/roko-agent/src/dispatcher/mod.rs` — Pass upstream hashes through dispatch

### Context

`Engram.lineage` and `derive()` exist. `PromptComposer` populates lineage from kept inputs. But many runtime-emitted Engrams still do not preserve upstream lineage consistently across dispatch, policy, and persistence flows.

### Implementation details

1. Audit all `Engram::new()` call sites in orchestrate.rs, dispatcher, and policy modules
2. For each: ensure `lineage` is populated with upstream engram hashes
3. In dispatcher: when creating response engrams, include input engram hash in lineage
4. In gate pipeline: when creating gate verdict engrams, include task engram hash
5. In persistence: ensure lineage is serialized to JSONL

### Verify command

```bash
cargo test --workspace -- lineage 2>&1 | tail -10
# Check: grep 'lineage' .roko/signals.jsonl | head -5 | jq '.lineage | length'
# Should be > 0 for derived engrams
```

---

## D.04 — Knowledge Tier Field

**Status**: DONE
**Priority**: —
**Files**: `crates/roko-neuro/src/knowledge_store.rs`

`KnowledgeEntry` has `tier` field (Transient/Working/Consolidated/Persistent) with `effective_half_life = base_half_life * tier_multiplier`. No work needed.

---

## D.05 — Knowledge Type Reconciliation

**Status**: DONE
**Priority**: —
**Files**: `crates/roko-neuro/src/knowledge_store.rs`

Canonical `KnowledgeKind`: Insight, Heuristic, Warning, CausalLink, StrategyFragment, AntiKnowledge. Legacy names preserved as serde aliases. No work needed.

---

## D.06 — ContextAssembler Canonicalization

**Status**: DONE
**Priority**: —
**Files**: `crates/roko-neuro/src/context.rs`

Auction-style budget arbitration implemented. Chunks compete under token cost with diminishing returns. No work needed.

---

## D.07 — HDC Encoding on Ingest

**Status**: DONE
**Priority**: —
**Files**: `crates/roko-neuro/src/knowledge_store.rs`

With `hdc` feature, `KnowledgeStore::ingest()` persists `hdc_vector`. No work needed.

---

## D.08 — CausalLink Permutation Binding

**Status**: DONE
**Priority**: —
**Files**: `crates/roko-neuro/src/hdc.rs`

Directional structure through permuted cause/effect role bindings. No work needed.

---

## D.09 — Tier Promotion State Machine

**Status**: PARTIAL
**Priority**: P2
**Estimated LOC**: ~40
**Dependencies**: None

### Files to modify

- `crates/roko-neuro/src/tier_progression.rs` — `TierProgression` (exists, D1/D2/D3 stages)
- `crates/roko-cli/src/orchestrate.rs` — Wire gate verdicts to tier promotion

### Context

`TierProgression` struct exists with D1/D2/D3 stages but no automated trigger. Gate verdicts should drive promotion/demotion of knowledge entries.

### Implementation details

1. In `tier_progression.rs`, add `fn evaluate_promotion(entry: &KnowledgeEntry, verdicts: &[GateVerdict]) -> Option<KnowledgeTier>`:
   - If entry used in 3+ successful tasks → promote (Transient→Working→Consolidated)
   - If entry associated with 2+ gate failures → demote
   - If entry unchanged for 2× half-life → mark for expiry review
2. In `orchestrate.rs`, after gate pipeline:
   - Collect knowledge entries used in this task
   - Call `evaluate_promotion()` for each
   - Apply tier changes to `KnowledgeStore`

### Verify command

```bash
cargo build -p roko-neuro -p roko-cli 2>&1 | tail -5
cargo test -p roko-neuro --lib -- tier_progression 2>&1 | tail -10
```

---

## D.10 — SomaticLandscape Completion

**Status**: PARTIAL
**Priority**: P2
**Estimated LOC**: ~60
**Dependencies**: None

### Files to modify

- `crates/roko-daimon/src/lib.rs` — Somatic landscape (kiddo k-d tree exists)

### Context

`SomaticLandscape` with kiddo k-d tree over 8D strategy space exists. Live task outcomes recorded from roko-cli, queried pre-dispatch. Missing: non-coding/native strategy-space extractors beyond the centralized role-aware config path, plus broader multi-surface use of somatic signals beyond current dispatch/prompt/retrieval path.

### Implementation details

1. Add domain-specific strategy-space extractors:
   - `fn extract_strategy_point(task: &Task, context: &TaskContext) -> [f64; 8]`
   - Extract: complexity (LOC estimate), risk (files touched), novelty (new vs existing), confidence (model tier), time_pressure (deadline proximity), scope (crate count), reversibility (test coverage), dependency_depth (DAG depth)
2. Wire somatic signals into additional decision surfaces:
   - Gate threshold adjustment: high-risk somatic markers → stricter gates
   - Retry strategy: emotionally tagged failures influence retry vs replan decision
3. Add `fn somatic_summary(&self) -> SomaticSummary` for TUI display

### Verify command

```bash
cargo build -p roko-daimon 2>&1 | tail -5
cargo test -p roko-daimon --lib -- somatic 2>&1 | tail -10
```

---

## D.11 — Behavioral State Classification

**Status**: DONE
**Priority**: —
**Files**: `crates/roko-core/src/affect.rs`, `crates/roko-daimon/src/lib.rs`

Shared `BehavioralState` in `roko-core` with `classify(pad, confidence)`. Wired into SystemPromptBuilder and CascadeRouter. No work needed.

---

## D.12 — Mood-Congruent Retrieval Completion

**Status**: PARTIAL
**Priority**: P2
**Estimated LOC**: ~30
**Dependencies**: None

### Files to modify

- `crates/roko-neuro/src/context.rs` — ContextAssembler PAD biasing (exists)

### Context

ContextAssembler biases retrieval with PadState, scores chunk-level emotional congruence, reserves contrarian slice. Missing: broader coordination with domain-native strategy-space extractors and cross-subsystem VCG market.

### Implementation details

1. Ensure somatic marker hints from `roko-daimon` are consumed by `ContextAssembler` when ranking chunks (partially wired)
2. Add `fn apply_somatic_bias(chunks: &mut [ContextChunk], markers: &[SomaticMarker])`:
   - Boost chunks that match somatic marker patterns
   - Ensure contrarian slice still applies (don't let somatic bias override it)
3. Log somatic influence to `.roko/learn/context-tuning.jsonl`

### Verify command

```bash
cargo build -p roko-neuro 2>&1 | tail -5
cargo test -p roko-neuro --lib -- context 2>&1 | tail -10
```

---

## D.13 — EmotionalTag Completion

**Status**: PARTIAL
**Priority**: P2
**Estimated LOC**: ~30
**Dependencies**: None

### Files to modify

- `crates/roko-core/src/engram.rs` — `EmotionalTag` (exists)
- `crates/roko-neuro/src/distiller.rs` — Emotional provenance in distillation

### Context

`EmotionalTag` on Engram exists. Distilled knowledge entries carry `EmotionalProvenance`. Retrieval-time weighting uses tags. Missing: somatic-landscape-backed recall and broader consolidation policy.

### Implementation details

1. In distiller, when consolidating knowledge entries:
   - Query somatic landscape for markers matching entry's emotional provenance
   - Weight consolidation priority by somatic marker intensity
2. In knowledge store, add emotional diversity metric to retrieval:
   - Prefer knowledge sets with diverse emotional origins (not all from same mood)
3. Add consolidation policy: entries with strong emotional tags and high somatic marker matches get priority consolidation

### Verify command

```bash
cargo build -p roko-neuro 2>&1 | tail -5
cargo test -p roko-neuro --lib -- distiller 2>&1 | tail -10
```

---

## D.14 — VCG Auction Completion

**Status**: PARTIAL
**Priority**: P2
**Estimated LOC**: ~60
**Dependencies**: None

### Files to modify

- `crates/roko-compose/src/budget.rs` — Auction allocator (greedy exists)
- `crates/roko-compose/src/context_provider.rs` — Bidder population

### Context

Greedy allocator exists with subsystem bidders (Neuro, Daimon, IterationMemory, etc.). Missing: exact welfare-maximizing knapsack with fairness controls, and several bidders (Oracles, richer iteration memory) are sparsely populated.

### Implementation details

1. In `budget.rs`:
   - Replace greedy allocator with knapsack-based approach for small bidder counts (< 20 items)
   - Keep greedy as fallback for large item counts
   - Add fairness constraint: no single bidder can win > 40% of total budget
2. Populate sparse bidders:
   - `Oracles`: wire prediction calibration data from `roko-learn/src/prediction.rs`
   - `IterationMemory`: include last 3 agent turns (currently only last 1)

### Verify command

```bash
cargo build -p roko-compose 2>&1 | tail -5
cargo test -p roko-compose --lib -- budget 2>&1 | tail -10
```

---

## D.15 — Daimon → CascadeRouter Completion

**Status**: PARTIAL
**Priority**: P2
**Estimated LOC**: ~30
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/cascade_router.rs` — Daimon policy consumption (partially wired)

### Context

Affect state modulates dispatch via `DispatchParams`, routing receives `RoutingContext.daimon_policy`. CascadeRouter uses Daimon policy for low-confidence escalation and behavioral-state tier biasing. Missing: other runtime decision layers not unified around policy.

### Implementation details

1. Unify Daimon policy consumption across:
   - Gate threshold selection (high-anxiety → stricter gates)
   - Retry strategy selection (behavioral state influences retry vs replan)
   - Tool allowlist adjustment (risk-averse state → fewer dangerous tools)
2. Add `DaimonPolicy::influence_gates(&self, thresholds: &mut GateThresholds)` method
3. Wire into gate pipeline in orchestrate.rs

### Verify command

```bash
cargo build -p roko-learn 2>&1 | tail -5
cargo test -p roko-learn --lib -- cascade_router 2>&1 | tail -10
```

---

## D.16 — Provider Auto-Selection: Universal

**Status**: PARTIAL
**Priority**: P1
**Estimated LOC**: ~40
**Dependencies**: None

### Files to modify

- `crates/roko-agent/src/dispatcher/mod.rs` — Ensure all dispatch paths use provider selection
- `crates/roko-agent/src/tool_loop/backends/mod.rs` — Backend-specific selection

### Context

`score_model_for_task()` and `select_model_for_task()` exist. `orchestrate.rs` derives task requirements and re-ranks models. But secondary runtimes still use narrower heuristics or fixed models.

### Implementation details

1. Audit all agent dispatch paths (Claude CLI, Gemini native, Ollama, OpenAI compat)
2. Ensure each path calls `select_model_for_task()` before dispatch
3. For paths that currently hardcode model: replace with provider-aware selection
4. Add fallback: if selection fails, use configured default model

### Verify command

```bash
cargo build -p roko-agent 2>&1 | tail -5
cargo test -p roko-agent --lib -- select_model 2>&1 | tail -10
```

---

## D.17 — LLM Backend HTTP Completeness

**Status**: PARTIAL
**Priority**: P1
**Estimated LOC**: ~60
**Dependencies**: None

### Files to modify

- `crates/roko-agent/src/gemini/native.rs` — Gemini native path
- `crates/roko-agent/src/tool_loop/backends/gemini_native.rs` — Gemini tool loop

### Context

Ollama and OpenAI-compatible backends have production HTTP implementations. Anthropic API and Perplexity flow through shared tool loops. Gemini-native request families still use a dedicated path. Non-chat specialty endpoints (embeddings, async deep-research) remain adapter-specific by design.

### Implementation details

1. Gemini native path: ensure tool dispatch flows through shared `ToolDispatcher` chain
2. Verify Gemini native requests receive safety layer checks
3. For specialty endpoints (embeddings, deep-research): document that adapter-specific paths are intentional, add safety validation at adapter boundary
4. Add integration test that sends a tool-bearing request through each backend and verifies safety layer is hit

### Verify command

```bash
cargo build -p roko-agent 2>&1 | tail -5
cargo test -p roko-agent --lib -- gemini 2>&1 | tail -10
```

---

## D.18 — DAG Optimization: CPM Analysis

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~80
**Dependencies**: None

### Files to modify

- `crates/roko-orchestrator/src/dag.rs` — `UnifiedTaskDag` (has `topological_sort()`, `waves()`, `stats()`)

### Context

Docs specify Critical Path Method with forward/backward pass for task scheduling. Currently DAG only has topological sort and wave computation.

### Implementation details

1. Add to `UnifiedTaskDag`:
   - `fn critical_path(&self) -> Vec<TaskId>` — forward pass (earliest start) + backward pass (latest start), return tasks with zero slack
   - `fn earliest_start(&self, task: TaskId) -> Duration` — max(predecessor earliest_finish)
   - `fn latest_start(&self, task: TaskId) -> Duration` — min(successor latest_start) - duration
   - `fn slack(&self, task: TaskId) -> Duration` — latest_start - earliest_start
2. Use task `estimated_duration` from tasks.toml (or heuristic from LOC estimate)
3. Expose via `roko plan show <id>` with `--critical-path` flag

### Verify command

```bash
cargo build -p roko-orchestrator 2>&1 | tail -5
cargo test -p roko-orchestrator --lib -- critical_path 2>&1 | tail -10
```

---

## D.19 — DAG Optimization: Task Fusion

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~60
**Dependencies**: D.18

### Files to modify

- `crates/roko-orchestrator/src/dag.rs`

### Context

Merge linear task chains to reduce scheduling overhead. If A→B→C with no other dependencies, fuse into single task.

### Implementation details

1. Add `fn fuse_linear_chains(&mut self) -> usize`:
   - Find nodes with exactly 1 predecessor and 1 successor
   - If both predecessor and successor have no other connections, merge
   - Combine task specs, sum LOC estimates
   - Return number of fusions performed
2. Only fuse tasks with same model tier requirement
3. Apply before execution, log fusions

### Verify command

```bash
cargo test -p roko-orchestrator --lib -- fuse 2>&1 | tail -10
```

---

## D.20 — DAG Optimization: Speculative Execution

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~80
**Dependencies**: None

### Files to modify

- `crates/roko-orchestrator/src/executor/mod.rs`

### Context

Spark-style speculative execution for slow tasks. If a task exceeds 2× expected duration, start a backup execution with a different model.

### Implementation details

1. Add speculative execution config: `speculative_threshold_multiplier: f64` (default 2.0)
2. When a task exceeds threshold:
   - Launch backup task with next-tier model
   - First to complete wins; cancel the other
3. Track speculative executions in state for dashboard display
4. Only speculate if budget allows (check cost projections)

### Verify command

```bash
cargo build -p roko-orchestrator 2>&1 | tail -5
cargo test -p roko-orchestrator --lib -- speculative 2>&1 | tail -10
```

---

## D.21 — DAG: Incremental Computation

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~100
**Dependencies**: None

### Files to modify

- `crates/roko-orchestrator/src/dag.rs` — Add `IncrementalDag`

### Context

Build-system-style incremental recomputation. Dirty/clean propagation with durability levels for selective invalidation.

### Implementation details

1. Add `IncrementalDag` struct:
   ```rust
   pub struct IncrementalDag {
       dag: UnifiedTaskDag,
       dirty: HashSet<TaskId>,
       durability: HashMap<TaskId, Durability>,
   }
   pub enum Durability { Low, Medium, High }
   ```
2. `fn mark_dirty(&mut self, task: TaskId)` — propagate dirty flag to all dependents
3. `fn clean_set(&self) -> HashSet<TaskId>` — tasks that don't need re-execution
4. `fn recompute_plan(&mut self) -> Vec<TaskId>` — return only dirty tasks in execution order
5. Use durability to decide what survives across re-plans

### Verify command

```bash
cargo test -p roko-orchestrator --lib -- incremental 2>&1 | tail -10
```

---

## D.22 — DAG: Dynamic Mutation

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~80
**Dependencies**: None

### Files to modify

- `crates/roko-orchestrator/src/dag.rs` — Add mutation API
- `crates/roko-orchestrator/src/executor/mod.rs` — Handle mutations during execution

### Context

DAGs are currently immutable once created. Docs specify live mutation with consistency invariants.

### Implementation details

1. Add `DagMutation` enum:
   ```rust
   pub enum DagMutation {
       AddTask { task: Task, depends_on: Vec<TaskId> },
       RemoveTask { task_id: TaskId },
       SplitTask { task_id: TaskId, into: Vec<Task> },
       AddDependency { from: TaskId, to: TaskId },
       UpdateTaskMetadata { task_id: TaskId, metadata: TaskMetadata },
   }
   ```
2. Add `fn apply_mutation(&mut self, mutation: DagMutation) -> Result<()>` to `UnifiedTaskDag`
3. Invariant checks: no cycles after mutation, no mutation of completed tasks
4. In executor: accept mutation channel, apply between wave boundaries

### Verify command

```bash
cargo test -p roko-orchestrator --lib -- mutation 2>&1 | tail -10
```

---

## D.23 — Agent Composition Operators

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~120
**Dependencies**: None

### Files to modify

- `crates/roko-agent/src/` — New module for composition

### Context

Docs specify `CompositeAgent`, `AgentComposition` enum (Pipeline, Parallel, Conditional, MixtureOfAgents), `MergeStrategy`, and `SkillSelector`. None exist.

### Implementation details

1. Create `crates/roko-agent/src/composition.rs`:
   ```rust
   pub enum AgentComposition {
       Pipeline(Vec<Box<dyn Agent>>),
       Parallel(Vec<Box<dyn Agent>>, MergeStrategy),
       Conditional { condition: Box<dyn Fn(&Task) -> usize>, branches: Vec<Box<dyn Agent>> },
       MixtureOfAgents { agents: Vec<Box<dyn Agent>>, aggregator: Box<dyn Agent> },
   }
   pub enum MergeStrategy { Concatenate, Aggregate, Vote, BestOfN }
   ```
2. Implement `CompositeAgent` that wraps `AgentComposition` and implements `Agent` trait
3. `Pipeline`: output of agent N becomes input of agent N+1
4. `Parallel`: run all agents concurrently, merge results via strategy
5. `Vote`: majority vote on categorical outputs

### Verify command

```bash
cargo build -p roko-agent 2>&1 | tail -5
cargo test -p roko-agent --lib -- composition 2>&1 | tail -10
```

---

## D.24 — Agent Introspection & Metacognition

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~100
**Dependencies**: None

### Files to modify

- `crates/roko-agent/src/` — New module for introspection

### Context

Self-inspection capability: `AgentIntrospection`, `AgentIdentity`, `MetacognitiveMonitor`, `Intervention` enum (escalate model, human handoff, abort, inject reflection).

### Implementation details

1. Create `crates/roko-agent/src/introspection.rs`:
   - `AgentIdentity { role, model_tier, temperament, capabilities }`
   - `MetacognitiveMonitor` — watches agent output for failure patterns:
     - Repeated tool calls with same args (stuck)
     - Contradictory statements within N turns
     - Confidence dropping below threshold
   - `Intervention` enum: `EscalateModel`, `HumanHandoff`, `Abort`, `InjectReflection(String)`
2. `MetacognitiveMonitor::check(turns: &[Turn]) -> Option<Intervention>` — analyze recent turns
3. Wire into tool loop: check after each turn, apply intervention if triggered

### Verify command

```bash
cargo build -p roko-agent 2>&1 | tail -5
cargo test -p roko-agent --lib -- introspection 2>&1 | tail -10
```

---

## D.25 — Supervision Strategy (Erlang/OTP)

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~80
**Dependencies**: None

### Files to modify

- `crates/roko-runtime/src/process.rs` — Add supervision strategies

### Context

Map Erlang restart strategies to plan execution recovery: OneForOne, OneForAll, RestForOne.

### Implementation details

1. Add to `roko-runtime`:
   ```rust
   pub enum SupervisionStrategy {
       OneForOne { max_restarts: u32, within_ms: u64, fallback_tier: String },
       OneForAll { max_restarts: u32 },
       RestForOne { max_restarts: u32 },
   }
   ```
2. `OneForOne`: restart only the failed agent, escalate tier after N failures
3. `OneForAll`: if one agent fails, restart all agents in the plan wave
4. `RestForOne`: restart the failed agent and all agents started after it
5. Wire into `ProcessSupervisor` as configurable strategy per plan

### Verify command

```bash
cargo build -p roko-runtime 2>&1 | tail -5
cargo test -p roko-runtime --lib -- supervision 2>&1 | tail -10
```

---

## D.26 — Capability-Based Security (OCaps)

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~100
**Dependencies**: None

### Files to modify

- `crates/roko-agent/src/safety/` — Add capability token system

### Context

Current model is RBAC via `AgentRole`. Docs specify OCaps with delegation chains: unforgeable `AgentWarrant` tokens, `Capability` enum (Tool, ReadPath, WritePath, Exec, Network).

### Implementation details

1. Create `crates/roko-agent/src/safety/capabilities.rs`:
   ```rust
   pub struct AgentWarrant {
       pub id: [u8; 32],
       pub capabilities: Vec<Capability>,
       pub issuer: String,
       pub expires_at: Option<u64>,
       pub delegate_depth: u8, // max delegation chain length
   }
   pub enum Capability {
       Tool(String),
       ReadPath(PathBuf),
       WritePath(PathBuf),
       Exec(String),
       Network { host: String, port: u16 },
   }
   ```
2. `fn check_capability(warrant: &AgentWarrant, required: &Capability) -> bool`
3. `fn delegate(warrant: &AgentWarrant, subset: &[Capability]) -> Result<AgentWarrant>` — create child warrant with reduced capabilities
4. Wire into `ToolDispatcher`: check warrant before tool execution

### Verify command

```bash
cargo build -p roko-agent 2>&1 | tail -5
cargo test -p roko-agent --lib -- capabilities 2>&1 | tail -10
```

---

## D.27 — Agent Metamorphosis

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~60
**Dependencies**: None

### Files to modify

- `crates/roko-agent/src/` — New module for role switching

### Context

Dynamic role switching during execution. `MorphableAgent`, `RoleProfile`, allowed transitions matrix.

### Implementation details

1. Create `crates/roko-agent/src/metamorphosis.rs`:
   - `RoleProfile { role, clarity, differentiation, alignment }`
   - `MorphableAgent` wrapping an agent with dynamic role state
   - Allowed transitions: `HashMap<AgentRole, Vec<AgentRole>>` (e.g., implementer can morph to reviewer but not to strategist)
   - `fn morph(&mut self, new_role: AgentRole) -> Result<()>` — check transition is allowed, update system prompt
2. Wire: allow conductor to request role morph when task category changes

### Verify command

```bash
cargo build -p roko-agent 2>&1 | tail -5
cargo test -p roko-agent --lib -- metamorphosis 2>&1 | tail -10
```

---

## D.28 — NREM Replay Modes

**Status**: SCAFFOLD
**Priority**: P2
**Estimated LOC**: ~120
**Dependencies**: None

### Files to modify

- `crates/roko-dreams/src/runner.rs` — Dream runner (scaffold exists)
- `crates/roko-dreams/src/cycle.rs` — Sleep cycle phases

### Context

Four replay modes specified: Random, Consequence (high-reward), Causal (failure chains), Hypothetical (what-if). Mattar-Daw utility formula for replay prioritization. Currently only scaffold exists.

### Implementation details

1. In `runner.rs`, implement 4 replay modes:
   - `Random`: sample episodes uniformly
   - `Consequence`: prioritize episodes with highest absolute reward (success or failure)
   - `Causal`: follow failure chains — find root cause episodes that led to cascading failures
   - `Hypothetical`: modify episode parameters (different model, different tool) and re-evaluate
2. Mattar-Daw utility: `U(s) = |reward(s)| × novelty(s) × recency_decay(s)`
3. Replay produces knowledge entries: distilled patterns from replayed episodes
4. Wire into dream scheduling (D.33)

### Verify command

```bash
cargo build -p roko-dreams 2>&1 | tail -5
cargo test -p roko-dreams --lib -- replay 2>&1 | tail -10
```

---

## D.29 — REM Imagination

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~150
**Dependencies**: D.28

### Files to modify

- `crates/roko-dreams/src/` — New module

### Context

Pearl structural causal models for counterfactual reasoning. GIRL trust-region constraints. Boden 3-mode creativity (combinational, exploratory, transformational).

### Implementation details

1. Create `crates/roko-dreams/src/imagination.rs`:
   - `CounterfactualQuery { episode_id, intervention: (variable, new_value) }`
   - `fn imagine(query: &CounterfactualQuery, model: &CausalModel) -> Outcome`
   - Trust region: counterfactual must be within plausibility bounds
2. Three creativity modes:
   - Combinational: merge patterns from different episodes
   - Exploratory: extend known patterns to new domains
   - Transformational: invert assumptions of successful patterns
3. Output: hypothetical knowledge entries for validation in next wake cycle

### Verify command

```bash
cargo build -p roko-dreams 2>&1 | tail -5
cargo test -p roko-dreams --lib -- imagination 2>&1 | tail -10
```

---

## D.30 — Hypnagogia Engine

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~100
**Dependencies**: D.28

### Files to modify

- `crates/roko-dreams/src/hypnagogia.rs` — Exists (moved from roko-golem)

### Context

4-layer system for sleep-onset creativity via stochastic resonance: Thalamic Gate, Executive Loosener, Dali Interrupt, Homuncular Observer.

### Implementation details

1. In `hypnagogia.rs`, implement 4 layers:
   - `ThalamicGate`: filter incoming signals by relevance + add noise (stochastic resonance)
   - `ExecutiveLoosener`: relax constraints on associative search (wider HDC neighborhood)
   - `DaliInterrupt`: periodically inject random knowledge entries to break fixation
   - `HomuncularObserver`: evaluate creativity of generated associations, keep promising ones
2. Pipeline: gate → loosen → interrupt → observe → emit creative associations
3. Output: candidate insights for dream consolidation

### Verify command

```bash
cargo build -p roko-dreams 2>&1 | tail -5
cargo test -p roko-dreams --lib -- hypnagogia 2>&1 | tail -10
```

---

## D.31 — Threat Simulation

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~80
**Dependencies**: None

### Files to modify

- `crates/roko-dreams/src/` — New module

### Context

FMEA/FTA systematic threat enumeration. CVSS/DREAD-style severity assessment for pre-emptive risk identification.

### Implementation details

1. Create `crates/roko-dreams/src/threat.rs`:
   - `ThreatScenario { description, likelihood, impact, detection_difficulty }`
   - `fn enumerate_threats(plan: &Plan) -> Vec<ThreatScenario>` — systematic enumeration
   - FMEA: for each task, what can fail? What's the effect? How likely?
   - Severity: `score = likelihood × impact × (1 - detection_probability)`
2. Generate mitigation recommendations for high-severity threats
3. Emit threats as Warning-type knowledge entries

### Verify command

```bash
cargo build -p roko-dreams 2>&1 | tail -5
cargo test -p roko-dreams --lib -- threat 2>&1 | tail -10
```

---

## D.32 — Sleep-Time Compute

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~60
**Dependencies**: D.28

### Files to modify

- `crates/roko-dreams/src/runner.rs` — Budget-aware dream scheduling

### Context

Lin et al. 2025 mechanism: ~5× reduction in test-time cost with 13-18% accuracy improvement via budget-aware offline processing.

### Implementation details

1. Add budget tracking to dream runner:
   - `DreamBudget { max_tokens, max_cost_usd, max_duration_s }`
   - Track consumed budget per dream cycle
   - Stop dreaming when budget exhausted
2. Prioritize high-value replay (Mattar-Daw utility) within budget
3. Track value generated per token spent during dreams
4. Adaptive scheduling: increase dream budget when dreams produce high-lift knowledge

### Verify command

```bash
cargo build -p roko-dreams 2>&1 | tail -5
cargo test -p roko-dreams --lib -- budget 2>&1 | tail -10
```

---

## D.33 — Dream Scheduling

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~50
**Dependencies**: D.28

### Files to modify

- `crates/roko-dreams/src/runner.rs` — Trigger logic
- `crates/roko-cli/src/orchestrate.rs` — Wire dream triggers

### Context

Three trigger types: idle (gap > threshold), scheduled (cron), manual. Frequency adaptation based on learning signal quality.

### Implementation details

1. Add trigger types:
   - Idle: if no task dispatched for > 5 minutes, start dream cycle
   - Scheduled: configurable cron expression in `roko.toml`
   - Manual: `roko dream` CLI command
2. Frequency adaptation:
   - After each dream cycle, measure knowledge quality (tier promotions, skill extractions)
   - If quality high: decrease interval (dream more often)
   - If quality low: increase interval (dream less often)
3. Wire idle trigger in orchestrate.rs between plan waves

### Verify command

```bash
cargo build -p roko-dreams -p roko-cli 2>&1 | tail -5
cargo run -p roko-cli -- dream --help 2>&1 | head -5
```

---

## D3 Batch Reconciliation (2026-04-15)

This summary supersedes the stale per-item status lines below for `D.34-D.54`.

- `D.34` DONE: `roko-learn` now has `EwcRegularizer` and logs knowledge-preservation dampening during routing updates.
- `D.35` DONE: `episode_logger` now computes episode importance and replay tiers from surprisal/novelty/difficulty/information-gain/diversity.
- `D.36` DONE: `curriculum.rs` adds `CurriculumStrategy`, `DifficultyModel`, and task reordering helpers.
- `D.37` DONE: `LearningRateSchedule` now modulates router exploration across cold/warm/mature phases.
- `D.38` PARTIAL: meta-learning remains routed through existing runtime feedback, experiments, and playbooks; no standalone tool-sequence miner landed.
- `D.39` PARTIAL: episode tiering is implemented as logical replay priority, not a hot/warm/cold storage migration.
- `D.40` PARTIAL: pheromones are now represented and transported as `Kind::Pheromone` engrams, but there is still no dedicated core `PheromoneField` with decay/confirmation state.
- `D.41` PARTIAL: mesh transport remains out of scope for this write boundary; D3 only lands local pheromone transport and prompt-enrichment seams.
- `D.42` DONE: `cfactor.rs` now detects collective pathologies such as cascade/groupthink/echo-chamber style failures.
- `D.43` DONE: CLI heartbeat theta reflection is wired through `heartbeat.rs` and `orchestrate.rs`, with persisted snapshots and conductor signals.
- `D.44` DONE: delta heartbeat now gates dream consolidation and only triggers dream work when the system is idle and due.
- `D.45` PARTIAL: adaptive cadence uses the existing `roko-core` operating-frequency scheduler instead of introducing a separate conductor-only clock stack.
- `D.46` DONE: 16 zero-LLM heartbeat probes are implemented in the CLI heartbeat path and persisted with each snapshot.
- `D.47` PARTIAL: universal safety work remains primarily in `roko-agent`; D3 did not extend backend safety wiring there.
- `D.48` PARTIAL: daemon/heartbeat lifecycle flushing improved, but the full 8-step agent deletion lifecycle in `roko-runtime` did not land.
- `D.49` PARTIAL: existing daemon mode now flushes heartbeat artifacts on shutdown.
- `D.50` PARTIAL: explicitly reconciled as future work; no `roko-wasm` crate was added in this batch.
- `D.51` DONE: compose/context assembly now ingest pheromone signals and render an active-signal layer.
- `D.52` DONE: `ActiveInferenceScorer` and goal-aware composition routing are now wired into `roko-compose`.
- `D.53` PARTIAL: no dedicated morphogenetic specialization module landed; current specialization is expressed through somatic strategy-space, routing, and pheromone-aware composition.
- `D.54` PARTIAL: `active_inference.rs` is wired into tier selection, but as a pragmatic selector rather than the full 90-state POMDP from the source doc.

## D.34 — EWC Regularizer

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~50
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/bandits.rs` — Add EWC to bandit updates

### Context

Elastic Weight Consolidation prevents catastrophic forgetting when updating model routing weights. Currently bandit arms update without regularization.

### Implementation details

1. Add `EwcRegularizer` to `bandits.rs`:
   - Track Fisher information matrix diagonal (per-arm variance of reward gradient)
   - On update: `new_weight = argmin(loss + λ/2 × Σ F_i × (θ_i - θ*_i)²)`
   - `λ` controls regularization strength (default 0.1)
2. Apply to `CascadeRouter` arm updates
3. Track when regularization prevents a weight change (log as "knowledge preservation event")

### Verify command

```bash
cargo build -p roko-learn 2>&1 | tail -5
cargo test -p roko-learn --lib -- ewc 2>&1 | tail -10
```

---

## D.35 — Episode Importance Scoring

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~40
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/episode_logger.rs` — Add importance scoring

### Context

Currently all episodes weighted equally in pattern discovery. Need: Surprisal + Novelty + Difficulty + Information Gain + Diversity scoring.

### Implementation details

1. Add `fn importance_score(episode: &Episode, history: &[Episode]) -> f64`:
   - Surprisal: how unexpected was the outcome? (1 - P(outcome | context))
   - Novelty: how different from recent episodes? (HDC distance to cluster centroids)
   - Difficulty: task complexity proxy (LOC, model tier used)
   - Information gain: did this episode change routing weights significantly?
   - Diversity: does this episode add variety to the episode corpus?
2. Score formula: `importance = 0.3×surprisal + 0.25×novelty + 0.2×difficulty + 0.15×info_gain + 0.1×diversity`
3. Use importance in dream replay prioritization (D.28) and pattern discovery

### Verify command

```bash
cargo build -p roko-learn 2>&1 | tail -5
cargo test -p roko-learn --lib -- importance 2>&1 | tail -10
```

---

## D.36 — Curriculum Learning

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~60
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/` — New module

### Context

`DifficultyModel` with task ordering (EasyFirst/HardFirst/Interleaved/Adaptive). Agent gets progressively harder tasks as skills improve.

### Implementation details

1. Create `crates/roko-learn/src/curriculum.rs`:
   ```rust
   pub enum CurriculumStrategy {
       EasyFirst,
       HardFirst,
       Interleaved,
       Adaptive { success_threshold: f64 },
   }
   pub struct DifficultyModel {
       strategy: CurriculumStrategy,
       skill_levels: HashMap<String, f64>, // per-category skill
   }
   ```
2. `fn reorder_tasks(tasks: &[Task], model: &DifficultyModel) -> Vec<Task>`:
   - EasyFirst: sort by estimated difficulty ascending
   - Adaptive: start easy, increase difficulty when success rate > threshold
3. Wire into DAG executor as optional reordering pass

### Verify command

```bash
cargo build -p roko-learn 2>&1 | tail -5
cargo test -p roko-learn --lib -- curriculum 2>&1 | tail -10
```

---

## D.37 — Learning Rate Scheduling

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~30
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/cascade_router.rs` — Per-subsystem learning rates

### Context

Currently all subsystems learn at same constant rate. Need per-subsystem phase multipliers (cold/warm/mature rates).

### Implementation details

1. Add `LearningRateSchedule`:
   ```rust
   pub struct LearningRateSchedule {
       cold_rate: f64,  // first N observations (high, explore)
       warm_rate: f64,  // N to M observations (medium)
       mature_rate: f64, // M+ observations (low, exploit)
       cold_threshold: usize,
       warm_threshold: usize,
   }
   ```
2. Apply per-subsystem: routing, gate thresholds, prompt experiments each get their own schedule
3. Persist observation counts alongside router state

### Verify command

```bash
cargo build -p roko-learn 2>&1 | tail -5
cargo test -p roko-learn --lib -- learning_rate 2>&1 | tail -10
```

---

## D.38 — Meta-Learning for Tool Use

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~60
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/` — New module

### Context

Track tool sequences correlated with success. Extract per-(role, task_category) tool usage patterns.

### Implementation details

1. Create `crates/roko-learn/src/tool_meta.rs`:
   - `ToolUsageProfile { role, task_category, tool_sequences: Vec<ToolSequence>, success_rate: f64 }`
   - `ToolSequence { tools: Vec<String>, frequency: usize, avg_success: f64 }`
2. `fn record_tool_usage(episode: &Episode, profile: &mut ToolUsageProfile)` — extract tool call sequence from episode
3. `fn recommend_tools(role: &str, category: &str, profiles: &[ToolUsageProfile]) -> Vec<String>` — return tools most correlated with success
4. Wire into prompt builder: include recommended tools in system prompt
5. Persist profiles to `.roko/learn/tool-profiles.json`

### Verify command

```bash
cargo build -p roko-learn 2>&1 | tail -5
cargo test -p roko-learn --lib -- tool_meta 2>&1 | tail -10
```

---

## D.39 — Episode Tiering

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~80
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/episode_logger.rs` — Tiered storage

### Context

Currently single flat `episodes.jsonl`. Need: Hot (recent, full detail), Warm (zstd compressed), Cold (HDC superposition).

### Implementation details

1. Add episode storage tiers:
   - Hot: `.roko/episodes.jsonl` — last 7 days, full JSON, random access
   - Warm: `.roko/episodes-warm/` — 7-30 days, zstd compressed per day
   - Cold: `.roko/episodes-cold.hdc` — 30+ days, HDC superposition vectors only
2. `fn compact_episodes()`:
   - Move episodes older than 7 days to warm tier (compress with zstd)
   - Move episodes older than 30 days to cold tier (compute HDC superposition)
3. `fn query_episodes(filter: &EpisodeFilter) -> Vec<Episode>`:
   - Hot: direct read
   - Warm: decompress + filter
   - Cold: HDC similarity search (approximate)
4. Run compaction on dream trigger or daily schedule

### Verify command

```bash
cargo build -p roko-learn 2>&1 | tail -5
cargo test -p roko-learn --lib -- episode_tier 2>&1 | tail -10
```

---

## D.40 — Pheromone System

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~150
**Dependencies**: None

### Files to modify

- `crates/roko-core/src/` — New module (or new crate)

### Context

Entire pheromone subsystem is 0 code. Docs specify: `Pheromone` struct, `PheromoneKind` (7 types), `PheromoneScope` (3 levels), exponential decay, reputation-weighted confirmation, pheromone-enriched context composition.

### Implementation details

1. Create pheromone module:
   ```rust
   pub struct Pheromone {
       pub kind: PheromoneKind,
       pub scope: PheromoneScope,
       pub intensity: f64,
       pub deposited_at: u64,
       pub deposited_by: String,
       pub content: String,
       pub decay_rate: f64,
   }
   pub enum PheromoneKind { Opportunity, Threat, Knowledge, Success, Failure, Resource, Warning }
   pub enum PheromoneScope { Task, Plan, Global }
   ```
2. Exponential decay: `current_intensity = initial × e^(-λ × elapsed_seconds)`
3. `PheromoneField`:
   - `fn deposit(&mut self, pheromone: Pheromone)`
   - `fn sense(&self, scope: PheromoneScope, kinds: &[PheromoneKind]) -> Vec<&Pheromone>` — return active pheromones above threshold
   - `fn confirm(&mut self, id: PheromoneId, confirmer: &str)` — reputation-weighted confirmation extends half-life
   - `fn decay(&mut self)` — remove expired pheromones
4. Persist to `.roko/pheromones.jsonl`
5. Wire into ContextAssembler: add pheromone summary layer to system prompt

### Verify command

```bash
cargo build -p roko-core 2>&1 | tail -5
cargo test -p roko-core --lib -- pheromone 2>&1 | tail -10
```

---

## D.41 — Agent Mesh Transport

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~200
**Dependencies**: D.40

### Files to modify

- `crates/roko-agent/src/` — New module for mesh communication

### Context

WebSocket relay + Iroh P2P. Version vector deduplication (Lamport/Fidge clocks). Enables multi-agent coordination beyond single-machine orchestration.

### Implementation details

1. Create mesh transport module:
   - `MeshTransport` trait: `send(target: AgentId, message: MeshMessage)`, `recv() -> MeshMessage`
   - `WebSocketRelay` implementation: connect to central relay server
   - `IrohP2P` implementation: direct peer-to-peer via Iroh
2. Version vector deduplication:
   - Each message carries a vector clock
   - Receivers reject messages already seen (idempotent delivery)
3. Message types: PheromoneDeposit, KnowledgeShare, TaskCoordination, Heartbeat

### Verify command

```bash
cargo build -p roko-agent 2>&1 | tail -5
cargo test -p roko-agent --lib -- mesh 2>&1 | tail -10
```

---

## D.42 — C-Factor Collective Intelligence

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~80
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/cfactor.rs` — Extend with collective diagnostics

### Context

`CFactor` struct exists with 10 sub-metrics and `compute()`. Missing: collective pathology detection (cascades, groupthink, echo chambers, deadlock, hallucination).

### Implementation details

1. Add pathology detection to `cfactor.rs`:
   ```rust
   pub enum CollectivePathology {
       Cascade { trigger_agent: String, affected_count: usize },
       Groupthink { diversity_score: f64 }, // diversity < 0.3
       EchoChamber { repeated_knowledge_pct: f64 },
       Deadlock { blocked_agents: Vec<String> },
       Hallucination { ungrounded_claims: Vec<String> },
   }
   pub fn detect_pathologies(episodes: &[Episode]) -> Vec<CollectivePathology>
   ```
2. Cascade: agent A's failure causes B and C to fail (trace through lineage)
3. Groupthink: all agents selecting same model/approach (low strategy diversity)
4. Echo chamber: same knowledge entries reused across agents without verification
5. Report pathologies in `roko status` output

### Verify command

```bash
cargo build -p roko-learn 2>&1 | tail -5
cargo test -p roko-learn --lib -- pathology 2>&1 | tail -10
```

---

## D.43 — Heartbeat: Theta Loop

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~80
**Dependencies**: None

### Files to modify

- `crates/roko-cli/src/orchestrate.rs` — Add periodic reflection loop
- `crates/roko-conductor/src/` — MetaCognitionHook (exists but not called periodically)

### Context

Current orchestration is a simplified Gamma (reactive task execution). Theta loop (~75s period) provides periodic reflection: assess progress, detect drift, adjust strategy.

### Implementation details

1. Add `ThetaLoop` struct:
   - Period: configurable, default 75 seconds
   - Each tick: read recent episodes, run `MetaCognitionHook`, assess plan progress
   - If progress stalled: emit Warning pheromone, suggest re-prioritization
   - If model costs spiking: trigger cost→routing feedback (E.06)
   - If success rate dropping: trigger confidence check
2. Run as background task alongside plan execution
3. Output: `ThetaReport { progress_pct, drift_detected, cost_rate, recommendations }`
4. Wire into orchestrate.rs as `tokio::spawn`ed background loop

### Verify command

```bash
cargo build -p roko-cli 2>&1 | tail -5
cargo test -p roko-conductor --lib -- metacognition 2>&1 | tail -10
```

---

## D.44 — Heartbeat: Delta Loop (Dreams Integration)

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~40
**Dependencies**: D.28, D.33

### Files to modify

- `crates/roko-dreams/src/runner.rs` — Integration with heartbeat
- `crates/roko-cli/src/orchestrate.rs` — Wire delta loop

### Context

Delta loop (hours-scale) runs dream consolidation. Dreams crate exists but not integrated into heartbeat cycle.

### Implementation details

1. Wire dream runner into orchestrate.rs as periodic (hourly) background task
2. On trigger: run NREM replay → REM imagination → knowledge consolidation
3. Report dream results: new knowledge entries, tier promotions, creative hypotheses
4. Pause during active plan execution (only run in idle periods)

### Verify command

```bash
cargo build -p roko-dreams -p roko-cli 2>&1 | tail -5
```

---

## D.45 — Adaptive Clock

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~120
**Dependencies**: D.43

### Files to modify

- `crates/roko-conductor/src/` — New module

### Context

`CorticalState` (32-signal atomic struct), `CognitiveSignal` (8 typed interrupts), `FrequencyScheduler` (adapts tick rate). None exist.

### Implementation details

1. Create `crates/roko-conductor/src/adaptive_clock.rs`:
   - `CorticalState`: 32 f32 values representing current system state (cost rate, success rate, load, etc.)
   - `CognitiveSignal`: interrupts that override normal scheduling (deadline approaching, budget exceeded, error spike)
   - `FrequencyScheduler`: adjusts Gamma/Theta/Delta tick rates based on regime detection
2. Regime detection: idle → reactive → active → intensive
3. Tick rate adjustment: idle=slow (30s gamma, 5min theta), intensive=fast (5s gamma, 30s theta)

### Verify command

```bash
cargo build -p roko-conductor 2>&1 | tail -5
cargo test -p roko-conductor --lib -- adaptive_clock 2>&1 | tail -10
```

---

## D.46 — T0 Probes (16 Zero-LLM Probes)

**Status**: NOT DONE
**Priority**: P1
**Estimated LOC**: ~200
**Dependencies**: None

### Files to modify

- `crates/roko-conductor/src/` — New module for probes

### Context

16 zero-cost probes that suppress LLM calls ~80% of the time. Each probe checks a condition without any LLM call and returns whether the system needs to act.

### Implementation details

1. Create `crates/roko-conductor/src/probes.rs`:
   ```rust
   pub trait T0Probe: Send + Sync {
       fn name(&self) -> &str;
       fn check(&self, state: &SystemState) -> ProbeResult;
   }
   pub enum ProbeResult { NoAction, TriggerGamma(String), TriggerTheta(String) }
   ```
2. Implement 16 probes:
   - `config_changed` — watch roko.toml mtime
   - `gate_failed_recently` — check last N gate verdicts
   - `file_modified` — watch workspace files (notify crate)
   - `test_count_delta` — compare test count to last known
   - `compile_error_new` — check `cargo check` exit code
   - `budget_threshold` — check remaining budget vs spent
   - `confidence_dropping` — rolling window of model confidence scores
   - `prediction_violation` — check if predictions were falsified
   - `tool_health_degraded` — ping critical tools (MCP servers, APIs)
   - `pheromone_detected` — check pheromone field for actionable signals
   - `task_deadline_near` — check task deadlines vs current time
   - `idle_timeout` — trigger if no activity for threshold period
   - `knowledge_stale` — check knowledge entry ages vs half-lives
   - `dependency_changed` — check Cargo.lock/package.json changes
   - `metric_anomaly` — Z-score check on key metrics
   - `heartbeat_timeout` — check last successful heartbeat time
3. `ProbeRunner::run_all() -> Vec<ProbeResult>` — run all probes, return actionable results
4. Wire: run probes before every Gamma tick; skip LLM dispatch if all probes return NoAction

### Verify command

```bash
cargo build -p roko-conductor 2>&1 | tail -5
cargo test -p roko-conductor --lib -- probe 2>&1 | tail -10
```

---

## D.47 — Safety Integration: Universal

**Status**: PARTIAL
**Priority**: P1
**Estimated LOC**: ~40
**Dependencies**: None

### Files to modify

- `crates/roko-agent/src/safety/mod.rs` — Safety layer
- `crates/roko-agent/src/claude_cli_agent.rs` — Wire safety checks
- `crates/roko-agent/src/tool_loop/backends/gemini_native.rs` — Wire safety checks

### Context

6 safety guards fully implemented (~1355 lines, 50+ tests). Routed HTTP/tool-loop paths now reach `ToolDispatcher`. Known-protocol subprocess branches (Claude CLI) and specialty endpoints (Gemini native, embeddings, async deep-research) still bypass.

### Implementation details

1. In `claude_cli_agent.rs`: before passing tool calls to Claude CLI subprocess, validate through `ToolDispatcher` safety layer
2. In `gemini_native.rs`: wrap tool dispatch through shared safety chain
3. For embeddings/deep-research: add boundary validation (input sanitization, output scrubbing) even if full safety chain is overkill
4. Add integration test: verify that a denied tool call (e.g., `rm -rf /`) is blocked regardless of backend

### Verify command

```bash
cargo build -p roko-agent 2>&1 | tail -5
cargo test -p roko-agent --lib -- safety 2>&1 | tail -10
```

---

## D.48 — Agent Deletion Lifecycle

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~60
**Dependencies**: None

### Files to modify

- `crates/roko-runtime/src/process.rs` — Shutdown protocol
- `crates/roko-neuro/src/knowledge_store.rs` — Knowledge backup with confidence decay

### Context

8-step clean shutdown specified in docs, not fully implemented. Knowledge backup/restore with 0.85^N confidence decay.

### Implementation details

1. Implement 8-step shutdown:
   - Signal intent, drain active tasks, flush state, backup knowledge, deregister from registry, archive episodes, emit shutdown engram, terminate process
2. Knowledge backup: serialize agent's knowledge entries with `confidence *= 0.85^depth` decay
3. Knowledge restore: when new agent inherits, load with decayed confidence
4. Wire into ProcessSupervisor's agent removal path

### Verify command

```bash
cargo build -p roko-runtime -p roko-neuro 2>&1 | tail -5
```

---

## D.49 — Daemon Mode

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: See F.09
**Dependencies**: None

Covered by F.09 (TUI & Interfaces section). Cross-reference only.

---

## D.50 — WASM Deployment

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~100
**Dependencies**: None

### Files to modify

- `crates/roko-wasm/` — **NEW CRATE**

### Context

Target modules: Engram, Score, Router, Composer. Enable running core roko logic in browser/edge environments.

### Implementation details

1. Create `roko-wasm` crate with `wasm32-unknown-unknown` target
2. Re-export core types with `#[wasm_bindgen]`:
   - `Engram::new()`, `Engram::derive()`, `Engram::content_hash()`
   - `Score::compute()`, `Score::confidence()`
   - `CascadeRouter::select()` (sync, no I/O)
   - `PromptComposer::assemble()` (sync, no I/O)
3. Exclude async types (Gate, Substrate) — these require I/O
4. Build with `wasm-pack build --target web`

### Verify command

```bash
wasm-pack build crates/roko-wasm --target web 2>&1 | tail -5
```

---

## D.51 — Compose: Pheromone Enrichment

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~30
**Dependencies**: D.40

### Files to modify

- `crates/roko-compose/src/system_prompt_builder.rs` — Add pheromone summary layer
- `crates/roko-compose/src/context_provider.rs` — Query pheromone field

### Context

SystemPromptBuilder assembles 6-7 layers but no pheromone summary layer. ContextProvider doesn't query pheromone field for threat/opportunity signals.

### Implementation details

1. In `context_provider.rs`:
   - Add `fn pheromone_context(field: &PheromoneField, scope: PheromoneScope) -> Vec<ContextChunk>`
   - Query active pheromones, format as context chunks
   - Include: threats (high priority), opportunities, warnings
2. In `system_prompt_builder.rs`:
   - Add `.with_pheromones(chunks: &[ContextChunk])` method
   - Insert after knowledge layer, before task brief
   - Format: `## Active Signals\n- [Threat] ...\n- [Opportunity] ...`

### Verify command

```bash
cargo build -p roko-compose 2>&1 | tail -5
cargo test -p roko-compose --lib -- pheromone 2>&1 | tail -10
```

---

## D.52 — Compose: Active Inference Scoring

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~80
**Dependencies**: None

### Files to modify

- `crates/roko-compose/src/scorer.rs` — Add EFE-based scoring strategy

### Context

Composer takes scorer as parameter (correct). But no Expected Free Energy (EFE) based scoring strategy exists.

### Implementation details

1. Add `ActiveInferenceScorer` implementing `Scorer` trait:
   ```rust
   // EFE formula: G = E_Q[ln Q(s') - ln P(s', o')]
   // Decomposes into:
   //   pragmatic_value: how well does this context serve the goal?
   //   epistemic_value: how much information does this context provide?
   pub struct ActiveInferenceScorer {
       goal_embeddings: Vec<f32>, // HDC vector of current goal
       prior_beliefs: HashMap<String, f64>, // P(topic) from knowledge store
   }
   ```
2. Score = pragmatic_value + epistemic_value
3. Pragmatic: cosine similarity between context HDC vector and goal HDC vector
4. Epistemic: information gain = KL divergence between prior and posterior
5. Automatically balances exploration/exploitation with zero hyperparameters

### Verify command

```bash
cargo build -p roko-compose 2>&1 | tail -5
cargo test -p roko-compose --lib -- active_inference 2>&1 | tail -10
```

---

## D.53 — Morphogenetic Specialization

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~100
**Dependencies**: D.40, D.41

### Files to modify

- `crates/roko-agent/src/` — New module

### Context

Turing reaction-diffusion dynamics for emergent role differentiation. Strategy vectors, niche competition heuristics. Agents specialize based on local environment signals.

### Implementation details

1. Create morphogenetic module:
   - `StrategyVector` — 8D vector representing agent's specialization
   - `fn differentiate(agents: &[StrategyVector], pheromones: &PheromoneField) -> Vec<StrategyVector>`:
     - Activator: success in a domain strengthens that dimension
     - Inhibitor: proximity to another agent's specialization weakens overlapping dimensions
     - Diffusion: pheromone signals influence all agents in scope
   - Niche competition: agents naturally avoid duplicating each other's strengths
2. Output: updated role hints for each agent, fed into system prompt

### Verify command

```bash
cargo build -p roko-agent 2>&1 | tail -5
cargo test -p roko-agent --lib -- morphogenetic 2>&1 | tail -10
```

---

## D.54 — Active Inference POMDP Router

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~120
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/` — New module

### Context

Factorized 90-state POMDP for tier selection. Currently using heuristic thresholds in CascadeRouter.

### Implementation details

1. Create `crates/roko-learn/src/active_inference.rs`:
   - 90-state POMDP: 3 task difficulties × 3 agent skills × 10 confidence levels
   - Actions: select model tier (T0, T1, T2)
   - Observations: gate verdict (pass/fail), cost, latency
   - Belief update: Bayesian filtering over hidden state
   - Policy: minimize expected free energy (EFE)
2. `fn select_tier(belief: &BeliefState, task: &TaskRequirements) -> ModelTier`
3. Wire as alternative to CascadeRouter (configurable in roko.toml)

### Verify command

```bash
cargo build -p roko-learn 2>&1 | tail -5
cargo test -p roko-learn --lib -- active_inference 2>&1 | tail -10
```
