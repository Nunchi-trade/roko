# Implementation Readiness: Dependency-Ordered Build Sequence

> Depth for [00-INDEX.md](../../unified/00-INDEX.md), the implementation readiness audit, and the consolidated roadmap. This doc derives a build sequence from the integration topology's Cell dependency graph, identifies parallelizable tracks, locates risk cliffs, and adds a Loop that tracks readiness metrics.

---

## 1. Method: Topological Sort of the Cell Dependency Graph

The integration topology (see [integration-topology.md](./integration-topology.md)) defines a typed directed graph. The build sequence is the topological sort of this graph: build the node with zero inbound dependencies first, then build nodes whose dependencies are all satisfied, and so on.

This is not a traditional project plan. It is a *structural derivation* from the architecture. The order is constrained by protocol dependencies, not by team preference.

### 1.1 The dependency DAG

Collapsing the integration graph into build-order dependencies (A depends on B means B must exist before A can be wired):

```
Level 0 (no dependencies):
  Store         -- roko-core: Signal types, Substrate trait
  Bus           -- roko-bus (target) or EventBus<E> in roko-runtime

Level 1 (depends on Level 0):
  Hdc           -- roko-hdc (target) or roko-primitives: depends on Store for fingerprint persistence
  Score         -- roko-core traits: depends on Store for Signal retrieval
  Connect       -- roko-agent: depends on Store for connection state

Level 2 (depends on Level 1):
  Route         -- CascadeRouter: depends on Score for candidate evaluation
  Compose       -- SystemPromptBuilder: depends on Store + Score for context retrieval
  ToolDispatch  -- depends on Connect for LLM backends

Level 3 (depends on Level 2):
  Verify        -- roko-gate: depends on Connect (external subprocess), Compose (evidence assembly)
  Observe       -- Lenses: depends on Store + Bus for projection sources
  CodeIndex     -- roko-index: depends on Score for relevance ranking

Level 4 (depends on Level 3):
  React         -- Policies: depends on Verify (verdicts), Bus (Pulse streams)
  ConductorReact -- depends on Verify (gate failure rates), Bus (live metrics)

Level 5 (depends on Level 4):
  PlanReact     -- Orchestrator: depends on React, Verify, Compose, Connect, Route
  LearnRecord   -- depends on Verify (verdicts), React (episode events)
  TriggerFire   -- depends on PlanReact for Graph activation

Level 6 (depends on Level 5):
  NeuroMemory   -- depends on Store + Score + LearnRecord (episodes to consolidate)
  DaimonAffect  -- depends on Score + React + Verify (PAD updates from verdicts)

Level 7 (depends on Level 6):
  DreamConsolidate -- depends on NeuroMemory + DaimonAffect + LearnRecord

Level 8 (depends on Level 7):
  ChainConnect  -- depends on Connect + Store + Verify for chain-specific backends
  Interfaces    -- depends on everything (CLI, TUI, HTTP are top-level consumers)
```

---

## 2. Build Phases Derived from the Topological Sort

### Phase A: Kernel Stabilization (Levels 0-1)

**What**: Stabilize Store and Bus as the two kernel fabrics. Ensure HDC fingerprinting is available as a Store-level primitive.

**Concrete work**:

```toml
# Phase A deliverables
[phase_a]
# Already built and tested:
store = { crate = "roko-core", status = "stable", tests = 376 }
bus_current = { crate = "roko-runtime", status = "stable", notes = "EventBus<E>" }
hdc = { crate = "roko-primitives", status = "built", tests = 18 }

# Target work:
bus_trait = { crate = "roko-bus", status = "target", notes = "Extract Bus trait from runtime" }
hdc_on_signal = { status = "gap", notes = "HDC fingerprint field on every Signal at write time" }
topic_filter = { status = "target", notes = "TopicFilter for Bus subscription" }
```

**Estimated readiness**: Store is **30/30** (fully specified and implemented). Bus is **partial** (EventBus exists but the kernel Bus trait is target-state). HDC is **built** (vectors work, fingerprinting per-Signal is not yet wired at Store.put()).

**Risk**: Bus extraction is the single riskiest refactor. If the Bus trait design is wrong, every subsystem migration is wrong. Mitigate by landing the trait with one backend (BroadcastBus wrapping `tokio::sync::broadcast`) and migrating one subsystem at a time.

```rust
// Phase A: minimal Bus trait
#[async_trait]
trait Bus: Send + Sync {
    async fn publish(&self, pulse: Pulse) -> Result<()>;
    async fn subscribe(&self, filter: TopicFilter) -> BusReceiver;
    // replay_since deferred to Phase A.2
}

// Phase A: HDC fingerprint at Store.put()
impl Store for ConcreteStore {
    fn put(&self, mut signal: Signal) -> Result<ContentHash> {
        // Compute HDC fingerprint if not already set
        if signal.hdc_fingerprint.is_none() {
            signal.hdc_fingerprint = Some(hdc_encode(&signal));
        }
        self.inner_put(signal)
    }
}
```

### Phase B: Framework and Evaluation (Levels 2-3)

**What**: Compose, Route, and Verify are the three protocols that produce the "scaffold" value proposition. Phase B makes them Bus-aware and adds the CodeIndex edge.

**Concrete work**:

```toml
[phase_b]
# Already built:
compose = { crate = "roko-compose", status = "wired", tests = 264 }
route = { crate = "roko-agent", status = "wired", tests = 567, notes = "CascadeRouter, LinUCBRouter" }
verify = { crate = "roko-gate", status = "wired", tests = 216, notes = "14 gates, 7 rungs" }

# Phase B additions:
code_index_wiring = { status = "gap", priority = "highest", notes = "Wire roko-index into ContextCompose" }
bus_publish_verdicts = { status = "gap", notes = "GateVerify publishes gate.verdict.emitted Pulses" }
bus_publish_tools = { status = "gap", notes = "ToolDispatch publishes tool.call.* Pulses" }
observe_lenses = { status = "gap", notes = "EdgeHealthLens, SccHealthLens" }
```

**Highest-leverage single edge**: CodeIndex → ContextCompose. The implementation readiness audit calls this out: `roko-index` has 5 files, ~700 LOC, 32 tests, plus 3 language providers (~2,339 LOC, 92 tests). It is a standalone library with *zero consumers*. Wiring it into `ContextCompose` gives agents code-aware context for free.

```rust
// Phase B: wire CodeIndex into ContextCompose
impl Compose for CodeAwareComposer {
    fn compose(
        &self,
        inputs: &[Signal],
        budget: &Budget,
        scorer: &dyn Score,
        ctx: &Context,
    ) -> Signal {
        // 1. Standard context retrieval
        let base_context = self.base_composer.compose(inputs, budget, scorer, ctx);

        // 2. Code-aware enrichment from roko-index
        let code_signals = self.code_index.query_symbols(
            &ctx.current_file,
            &ctx.current_function,
            budget.reserve(0.2),  // reserve 20% of budget for code context
        );

        // 3. Score and merge
        let all_signals = [base_context.as_slice(), code_signals.as_slice()].concat();
        self.base_composer.compose(&all_signals, budget, scorer, ctx)
    }
}
```

**Risk**: Bus migration of GateVerify and ToolDispatch. Both currently use direct function calls. The migration must not break the core execution loop. Mitigate by publishing Pulses *in addition to* direct calls (dual-write), then migrating consumers to Bus subscriptions one at a time.

### Phase C: Learning and Adaptation (Levels 4-5)

**What**: Wire the learning feedback loops. Connect LearnRecord to all sources. Wire DaimonAffect into the orchestrator. Close the affect learning loop.

**Concrete work**:

```toml
[phase_c]
# Already built:
learn = { crate = "roko-learn", status = "wired", tests = 348 }
orchestrate = { crate = "roko-orchestrator", status = "wired", tests = 315 }

# Phase C additions:
learn_from_all_verdicts = { status = "partial", notes = "LearnRecord receives all gate verdict types, not just pass/fail" }
daimon_verdict_loop = { status = "gap", notes = "GateVerify → DaimonAffect via gate.verdict.emitted Pulse" }
neuro_routing = { status = "gap", notes = "NeuroMemory → ModelRoute: knowledge-informed model selection" }
conductor_bus = { status = "gap", notes = "ConductorReact subscribes to Bus instead of importing roko-learn" }
heuristic_extraction = { status = "partial", notes = "Playbook extraction exists; falsifier-bearing Heuristics do not" }
demurrage_wiring = { status = "target", notes = "Replace age-only pruning with balance + reinforcement" }
```

**Dependency chain that must be respected**:
```
HDC fingerprint on Signals (Phase A)
  → Similarity-based Store queries (Phase B)
  → Demurrage with novelty-weighted reinforcement (Phase C)
  → Heuristic calibration with falsifiers (Phase C)
  → c-factor measurement (Phase C, depends on all above)
```

This chain is the critical path from the consolidated roadmap. Attempting c-factor without HDC-backed demurrage produces a metric with no structural backing.

```rust
// Phase C: demurrage with novelty-weighted reinforcement
impl Store for DemurrageStore {
    fn reinforce(&self, hash: &ContentHash, kind: ReinforceKind) -> Result<()> {
        let mut signal = self.get(hash)?;
        let bonus = match kind {
            ReinforceKind::Retrieved => 0.05,
            ReinforceKind::Cited => 0.10,
            ReinforceKind::Gated => 0.15,  // passed verification
            ReinforceKind::Surprised => {
                // Novelty-weighted: rare Signals get larger bonus
                let novelty = self.compute_novelty(&signal)?;
                0.05 + 0.15 * novelty  // range [0.05, 0.20]
            }
            ReinforceKind::AgentQuoted => 0.08,
        };
        signal.balance = (signal.balance + bonus).min(1.0);
        signal.last_reinforced = now();
        self.inner_put(signal)
    }

    fn compute_novelty(&self, signal: &Signal) -> Result<f64> {
        // 1 - max_similarity to top-K neighbors via HDC
        let neighbors = self.query_similar(
            signal.hdc_fingerprint.as_ref().unwrap(),
            5,  // top-5
        )?;
        let max_sim = neighbors.iter()
            .map(|n| hdc_similarity(&signal.hdc_fingerprint.unwrap(), &n.hdc_fingerprint.unwrap()))
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(0.0);
        Ok(1.0 - max_sim)
    }
}
```

### Phase D: Consolidation and Cross-Cuts (Levels 6-7)

**What**: Wire the offline learning loop (Dreams), the knowledge cross-cut (Neuro to Compose), and the affect cross-cut (Daimon to all consumers).

**Concrete work**:

```toml
[phase_d]
# Already built (scaffold):
dreams = { crate = "roko-dreams", status = "scaffold", notes = "DreamRunner exists, no runtime trigger" }
neuro = { crate = "roko-neuro", status = "partial", notes = "Store types spread across crates" }
daimon = { crate = "roko-daimon", status = "built", notes = "PAD + somatic landscape exist" }

# Phase D additions:
dream_trigger = { status = "gap", notes = "Delta-speed trigger from Bus subscription to substrate.signal.stored" }
dream_to_neuro = { status = "gap", notes = "DreamConsolidate → NeuroMemory: consolidated Signals" }
dream_to_daimon = { status = "gap", notes = "DreamConsolidate → DaimonAffect: depotentiation of traumatic markers" }
neuro_to_compose = { status = "partial", notes = "Knowledge entries partially reach ContextCompose" }
daimon_consolidation = { status = "gap", notes = "Merge two Daimon implementations (roko-daimon + legacy scaffold)" }
```

**Risk cliff**: Dreams depends on everything above it in the dependency DAG. If Phase A, B, or C has unresolved issues, Dreams will be blocked. Mitigate by running Dreams in "read-only replay" mode first (read episodes from Store, consolidate, but do not write back) to validate the consolidation logic before wiring the output edges.

### Phase E: Domain and Interface (Level 8)

**What**: Wire domain-specific subsystems (Chain, Code Intelligence in full) and all interface surfaces (CLI, TUI, HTTP, WebSocket).

**Concrete work**:

```toml
[phase_e]
chain_integration = { status = "scaffold", notes = "roko-chain has traits and mocks; needs alloy wiring" }
serve_bus = { status = "gap", notes = "roko-serve subscribes to Bus for live updates" }
tui_bus = { status = "gap", notes = "TUI subscribes to Bus for real-time dashboard" }
domain_profiles = { status = "target", notes = "Packaged domain-specific configurations" }
plugin_spi = { status = "target", notes = "5-tier extension model" }
```

---

## 3. Parallelization Analysis

Which phases can overlap?

```
Phase A: Kernel Stabilization        [MUST be first]
         |
         v
Phase B: Framework & Evaluation      [starts after Phase A Store+Bus stable]
         |
         +---> Phase C: Learning     [starts after Phase B Verify wired to Bus]
         |
         +---> Phase E.interfaces:   [can start after Phase B Observe wired]
         |     (CLI, TUI, HTTP
         |      subscribe to Bus)
         |
         v
Phase D: Cross-Cuts                  [starts after Phase C Learning wired]
         |
         v
Phase E.domain: Domain Integration   [starts after Phase D Neuro wired]
```

**Maximum parallelism**: After Phase A, three tracks can run concurrently:
1. Phase B (framework + evaluation)
2. Phase E.interfaces (interface Bus subscriptions)
3. CodeIndex wiring (independent of Bus migration)

After Phase B, two tracks can run concurrently:
1. Phase C (learning)
2. Remaining Phase E.interfaces

Phase D is strictly sequential on Phase C. Phase E.domain is strictly sequential on Phase D.

**For the actual team shape (1 developer + AI agents)**: Most tracks serialize. The practical order is A → B → C → D → E, with CodeIndex wiring as the highest-leverage detour that can happen anytime after Phase A.

---

## 4. Risk Cliffs

A risk cliff is a point where accumulated technical debt, architectural assumptions, or untested integration creates a sudden failure risk.

### 4.1 Risk cliff 1: Bus trait design (Phase A)

**What**: The Bus trait is the most consequential API surface in the system. Every subsystem will depend on it. If the initial design is wrong, the migration cost is proportional to the number of subsystems already migrated.

**When**: End of Phase A, when the first non-trivial subsystem migrates to Bus.

**Mitigation**:
```rust
// Start with the minimal trait and one backend
#[async_trait]
trait Bus: Send + Sync {
    async fn publish(&self, pulse: Pulse) -> Result<()>;
    async fn subscribe(&self, filter: TopicFilter) -> BusReceiver;
    // NO replay_since yet -- add in Phase A.2 after initial migration validates
}

// One backend: in-process broadcast
struct BroadcastBus {
    sender: broadcast::Sender<Pulse>,
}

// Migrate ONE subsystem first (GateVerify → LearnRecord via Bus)
// Wait for one plan execution cycle to validate
// Then migrate the next subsystem
```

### 4.2 Risk cliff 2: Demurrage rate tuning (Phase C)

**What**: Demurrage rates that are too aggressive will cold-archive useful knowledge. Rates that are too conservative will let the Store bloat. The correct rates depend on usage patterns that do not exist yet (because the learning loop is being built in Phase C).

**When**: Middle of Phase C, when demurrage first runs against real data.

**Mitigation**: Start with conservative rates (slow decay), observe with a `DemurrageHealthLens`, and tune using the Theta-cadence calibration Loop.

```rust
// Conservative initial demurrage rates
const INITIAL_DEMURRAGE_RATES: &[(&str, f64)] = &[
    ("Heuristic", 0.001),    // very slow: 50% in ~700 hours
    ("Episode", 0.01),       // moderate: 50% in ~70 hours
    ("Insight", 0.005),      // slow: 50% in ~140 hours
    ("AgentOutput", 0.05),   // fast: 50% in ~14 hours
];

// DemurrageHealthLens observes:
// - What fraction of cold-archived Signals are later retrieved? (too aggressive if high)
// - What is the Store growth rate? (too conservative if accelerating)
// - What is the mean balance of actively-retrieved Signals? (healthy range: 0.3-0.8)
```

### 4.3 Risk cliff 3: Dreams integration (Phase D)

**What**: Dreams depends on every prior phase. If Phase A (Store), Phase B (Verify), or Phase C (Learn) has unresolved issues, Dreams will surface them all simultaneously. Dreams is also the subsystem with the weakest error handling (2/5 in the readiness audit).

**When**: Start of Phase D, when DreamRunner first runs against real episode data.

**Mitigation**: Run Dreams in read-only mode first. Do not write consolidated Signals back to Store until the consolidation logic is validated against known-good episodes.

### 4.4 Risk cliff 4: The orchestrator hub migration (Phase B → C)

**What**: The orchestrator (PlanReact) is the highest-degree node in the integration graph. Migrating it from direct function calls to Bus subscriptions changes the entire system's coordination model. A bug in this migration breaks the core execution loop.

**When**: Phase B/C boundary, when the orchestrator starts subscribing to Bus topics instead of holding direct channels.

**Mitigation**: Dual-write period. The orchestrator keeps its direct channels AND subscribes to Bus topics. A consistency checker validates that both paths produce the same events. Only after N=100 consistent plan executions, remove the direct channels.

---

## 5. Readiness Tracking Loop

A Loop (Graph with feedback edge) tracks implementation readiness metrics:

```rust
struct ReadinessTrackingLoop {
    // Observe: check which edges in the system graph are wired
    integration_lens: DisconnectionLens,
    // Observe: check test pass rates per phase
    test_lens: TestHealthLens,
    // React: publish readiness metrics
    policy: ReadinessPolicy,
}

struct ReadinessMetrics {
    // Phase-level readiness
    phase_a_readiness: f64,  // fraction of Phase A edges wired
    phase_b_readiness: f64,
    phase_c_readiness: f64,
    phase_d_readiness: f64,
    phase_e_readiness: f64,

    // System-level readiness
    integration_ratio: f64,  // fraction of nodes reachable from PlanReact
    scc_count: usize,        // number of strongly-connected components
    test_pass_rate: f64,     // workspace-wide test pass rate
    wired_edge_count: usize, // edges with status == Wired
    total_edge_count: usize, // total edges in target graph

    // Risk indicators
    longest_untested_path: Vec<SystemNode>,  // longest chain of edges with no integration test
    highest_degree_node: SystemNode,          // node with most edges (coupling indicator)
    stalled_sccs: Vec<String>,               // SCCs that have stopped cycling
}

impl React for ReadinessPolicy {
    fn react(&self, pulses: &[Pulse]) -> ReactOutput {
        let metrics = self.compute_readiness(pulses);

        // Publish readiness metrics
        let readiness_pulse = Pulse::new(
            "readiness.metrics",
            PulseBody::Readiness(metrics.clone()),
        );

        // Alert on risk cliffs
        let mut alerts = vec![];
        if metrics.phase_a_readiness < 1.0 && metrics.phase_b_readiness > 0.0 {
            alerts.push(Pulse::new(
                "readiness.alert.premature_phase_b",
                PulseBody::Alert("Phase B started before Phase A complete".into()),
            ));
        }
        if metrics.integration_ratio < 0.5 {
            alerts.push(Pulse::new(
                "readiness.alert.low_integration",
                PulseBody::Alert(format!(
                    "Only {:.0}% of system nodes are reachable",
                    metrics.integration_ratio * 100.0,
                )),
            ));
        }

        ReactOutput {
            pulses: [vec![readiness_pulse], alerts].concat(),
            signals: vec![Signal::new(
                Kind::Observation,
                Body::Json(serde_json::to_value(&metrics).unwrap()),
            )],
        }
    }
}
```

---

## 6. Current State Assessment

Mapping the implementation readiness audit scores to the phase structure:

| Phase | Key Crates | Audit Score | Test Count | Status |
|---|---|---|---|---|
| **A: Kernel** | roko-core, roko-runtime, roko-primitives | 21/30 | 376 + ~50 | Store stable; Bus partial; HDC-per-Signal gap |
| **B: Framework** | roko-agent, roko-compose, roko-gate, roko-index | 21 + 25 + 27 + 24 = 97/120 | 567 + 264 + 216 + 92 | Core loop wired; CodeIndex unwired; Bus publishing missing |
| **C: Learning** | roko-learn, roko-conductor, roko-orchestrator | 29 + 29 + 30 = 88/90 | 348 + 130 + 315 | Best-specified subsystems; conductor layer violation; demurrage target-state |
| **D: Cross-cuts** | roko-neuro, roko-daimon, roko-dreams | 22 + 23 + 23 = 68/90 | 18 + ~20 + ~10 | Weakest section; Dreams scaffold-only; Daimon dual-implementation |
| **E: Domain+Interface** | roko-chain, roko-cli, roko-serve | 18 + 23 + (not scored) | 10 + 38 + ~50 | Chain deferred; CLI wired; serve built but not Bus-connected |

**Key insight**: Phase C (Learning) has the highest audit scores (29-30/30) but depends on Phase A (Bus) and Phase B (Verify on Bus) which have lower scores. The readiness is inverted: the most-specified subsystems cannot be fully wired until the less-specified kernel is stabilized. This is the architectural reason the kernel must come first.

---

## 7. What This Enables

1. **Structurally-derived build order** -- the phase sequence comes from the Cell dependency graph, not from feature prioritization meetings
2. **Parallelization visibility** -- the DAG shows exactly which tracks can overlap and which must serialize
3. **Risk cliff awareness** -- four specific risk cliffs are identified with concrete mitigation strategies
4. **Automated readiness tracking** -- the ReadinessTrackingLoop continuously observes integration ratio, SCC health, and phase completeness

## 8. Feedback Loops

| Loop | Input | Output | Cadence |
|---|---|---|---|
| Readiness tracking | Integration graph edge status | Phase readiness percentages, alerts | Theta (daily) |
| Risk cliff monitoring | Phase boundary conditions | Risk alerts when a phase starts before its predecessor completes | Gamma (per-commit) |
| Test health | cargo test results | Per-crate test pass rates, regressions | Gamma (per-commit) |
| Integration ratio trend | DisconnectionLens output | Monotonicity check: ratio should never decrease | Theta (weekly) |

## 9. Open Questions

1. **Phase A duration estimate**: The Bus trait extraction is the gating item. How long does it take to design a Bus trait that survives Phase B migration? Conservative estimate: 2-3 weeks for trait design + one backend + one subsystem migration. Risk: the design may need revision after the first migration reveals assumptions.

2. **Dual-write overhead**: During the orchestrator migration (Phase B/C boundary), the system dual-writes to both direct channels and Bus. What is the overhead? For in-process BroadcastBus, the cost is a `clone()` per Pulse (~100ns). For persistent Bus backends (future), the cost could be significant.

3. **CodeIndex wiring scope**: Should CodeIndex provide context for all languages or start with Rust only? The readiness audit shows three language providers (Rust, TypeScript, Go) with 92 total tests. Starting with Rust-only is lower risk and validates the wiring pattern.

4. **Dreams error handling**: The readiness audit scores Dreams error handling at 2/5 (weakest in the codebase). Phase D cannot safely run Dreams against real data without improving error handling first. Should error handling improvement be a Phase C deliverable (before Dreams wiring) or a Phase D prerequisite?

5. **Team shape vs parallelism**: The roadmap assumes 5-7 engineers for concurrent tracks. With 1 developer + AI agents, nearly all tracks serialize. Should the phase structure change for the actual team shape, or should the same phases simply take longer? The dependency order is structural and does not change; only elapsed time changes.
