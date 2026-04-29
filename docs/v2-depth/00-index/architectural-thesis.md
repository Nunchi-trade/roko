# Architectural Thesis: The Scaffold IS the Product

> Depth for [00-INDEX.md](../../unified/00-INDEX.md), [01-SIGNAL.md](../../unified/01-SIGNAL.md) layers, and the five-layer taxonomy. This doc re-derives the layer architecture as a structural consequence of Cell/Graph composition and protocol conformance, not as arbitrary boundary-drawing. It then extends the model to L5: self-evolution.

---

## 1. The Core Claim, Restated

The empirical case is settled. Given the same LLM, agent performance varies 30-65% depending on harness quality (SWE-bench Verified, Jimenez et al. 2024). Meta-Harness (Lee et al. 2026) shows +7.7 points from harness optimization alone at 4x fewer tokens. FrugalGPT (Chen et al. 2023) matches GPT-4 at 2% cost through routing alone.

The implication: the scaffold is not scaffolding. It is the product. LLM inference is a commodity input; the system wrapping it is the durable asset.

This claim has a structural consequence. If the scaffold compounds value independently of the model, then the scaffold must be:

1. **Composable** -- different domains need different scaffold configurations
2. **Self-observing** -- the scaffold must measure its own contribution to outcome quality
3. **Self-modifying** -- the scaffold must improve from what it observes

These three requirements, formalized, produce the five-layer architecture as a theorem rather than a design choice.

---

## 2. Deriving Layers from Protocol Conformance

The unified spec defines three fundamentals: Signal (durable data), Cell (atomic computation), and Graph (composition of Cells). See [02-CELL.md](../../unified/02-CELL.md) for the Cell specification.

A Cell declares typed I/O and protocol conformance. The nine protocols are: Store, Score, Verify, Route, Compose, React, Observe, Connect, and Trigger. A Cell does not know which layer it lives in. Layers emerge from the *dependency structure* of protocol conformance.

### 2.1 The dependency lattice

Consider which protocols require which others as prerequisites:

```
Trigger  ──requires──>  Bus (to receive Pulses)
React    ──requires──>  Bus (to subscribe to Pulse streams)
Observe  ──requires──>  Store + Bus (to read Signals, subscribe to Pulses)
Compose  ──requires──>  Store + Score (to retrieve and rank Signals under budget)
Route    ──requires──>  Score (to compare candidates)
Verify   ──requires──>  Connect (to call external reality: compiler, test suite, chain)
Score    ──requires──>  Store (to read prior Signals for comparison)
Connect  ──requires──>  (nothing internal -- bridges to external world)
Store    ──requires──>  (nothing -- the bottom)
```

This dependency relation is a partial order. A topological sort of this partial order produces exactly five strata:

```rust
// The five strata derived from protocol dependency:
//
// Stratum 0: Store, Bus           -- data at rest, data in motion
// Stratum 1: Connect, Score       -- external I/O, evaluation
// Stratum 2: Compose, Route       -- assembly under budget, selection among candidates
// Stratum 3: Verify, Observe      -- check against reality, read-only projection
// Stratum 4: React, Trigger       -- reactive policy, event-driven firing

// This maps directly to the five layers:
enum Layer {
    L0Runtime,       // Store + Bus: the two fabrics
    L1Framework,     // Connect + Score: external bridges + evaluation
    L2Scaffold,      // Compose + Route: context engineering + model selection
    L3Harness,       // Verify + Observe: gates + lenses
    L4Orchestration, // React + Trigger: policy + event-driven coordination
}
```

### 2.2 Why this is not arbitrary

The layers are not an organizational convenience. They are the *only* topological sort of the protocol dependency graph (modulo co-stratum ordering). If you try to put Verify below Score, you find that Verify needs Connect (external I/O) which needs Store (to record connection state). If you try to put Compose above Verify, you find that Compose needs Score which needs Store, and Verify also needs Store through Connect -- the dependency arrows force the ordering.

The downward-dependency rule (Layer N depends on Layer N-1, never reverse) is therefore not a policy. It is a structural invariant of the protocol dependency lattice.

### 2.3 Cross-cuts are precisely the protocols that span multiple strata

Memory (Store Cell + demurrage + dreams) touches L0 (storage), L2 (context retrieval), and L4 (dream consolidation policy). The Daimon (PAD affect) touches L1 (routing bias), L2 (composition weighting), and L3 (gate threshold modulation). These are cross-cuts because they implement protocols at multiple strata simultaneously. They are injected via trait objects because any single-stratum placement would violate the dependency lattice.

```rust
// A cross-cut is a Cell that implements protocols at 2+ strata.
// It MUST be injected as a trait object to avoid circular layer deps.

trait MemoryCrossCut: Store + Score + React {
    // Store at L0: persist Signals with demurrage
    // Score at L1: relevance scoring for retrieval
    // React at L4: dream consolidation policy on episodes
}

// Injection pattern: higher layers receive cross-cuts as &dyn, not
// as direct imports. This preserves the dependency lattice.
fn compose_with_memory(
    composer: &dyn Compose,
    memory: &dyn Store,        // L0 protocol, injected
    scorer: &dyn Score,        // L1 protocol, injected
    budget: &Budget,
) -> Signal { /* ... */ }
```

---

## 3. The Five Layers as Beer's VSM

The derivation above produces the same structure that Beer's Viable System Model (1972) predicts for any viable organization. This is not coincidence -- both derive from the same constraint: a system that must regulate itself needs exactly five recursive subsystems.

| Beer VSM | Derived Layer | Protocol Basis | Function |
|---|---|---|---|
| System 1: Operations | L0 Runtime | Store + Bus | Primary activities: data persistence and transport |
| System 2: Coordination | L1 Framework | Connect + Score | Anti-oscillation: external bridges prevent conflicting model calls |
| System 3: Control | L2 Scaffold | Compose + Route | Resource allocation: token budgets, model selection |
| System 3*: Audit | L3 Harness | Verify + Observe | Quality assurance: gate pipeline, read-only lenses |
| System 4: Intelligence | L4 Orchestration | React + Trigger | Adaptation: plan DAGs, policy reactions, event-driven coordination |

The missing piece in Beer's VSM is System 5: Policy/Identity. In Roko, this role is filled by the Daimon cross-cut (self-model via PAD vector) plus the specification itself (evolvable artifact). See section 5.

---

## 4. Crate Map as Layer Instantiation

Each crate instantiates protocols at a specific layer. The current workspace and target boundaries:

```
L4 Orchestration
  roko-orchestrator  -- React: PlanPhasePolicy, plan state machine
  roko-conductor     -- React: CircuitBreakerPolicy, 10 watchers
  roko-cli           -- Trigger: CLI commands fire Graphs

L3 Harness
  roko-gate          -- Verify: 14 gates, 7-rung pipeline, adaptive thresholds
  roko-fs            -- Store impl: JSONL substrate persistence (also L0)

L2 Scaffold
  roko-compose       -- Compose: SystemPromptBuilder, 9 role templates
                     -- target split: roko-compose-core + roko-templates

L1 Framework
  roko-agent         -- Connect: 5+ LLM backends, MCP client
                     -- Score: ToolRelevanceScorer
                     -- Route: CascadeRouter, LinUCBRouter
  roko-std           -- Score: builtin scorers
                     -- target split: roko-defaults + roko-tools

L0 Runtime / Kernel
  roko-core          -- Store: Substrate trait, Signal types, 6 kernel traits
  roko-runtime       -- Bus: EventBus<E> (current), ProcessSupervisor
  roko-primitives    -- HDC vectors, Hamming similarity
  roko-bus           -- [target] Bus trait, Topic, TopicFilter
  roko-hdc           -- [target] HDC similarity, fingerprinting
  roko-spi           -- [target] Plugin extension contracts

Cross-cuts (injected, not layer-bound)
  roko-neuro         -- Store + Score: knowledge, tier progression, HDC
  roko-daimon        -- Score + React: PAD affect, behavioral states
  roko-dreams        -- React: offline consolidation, hypothesis generation
  roko-learn         -- Score + React: episodes, playbooks, bandits, experiments
```

### 4.1 The one confirmed layer violation

`roko-conductor` (L3/L4) imports `roko-learn` (L2/Cross-cut) directly for circuit-breaker state tracking. The Bus-first fix routes this through `gate.failure.rate` Pulses on the L0 Bus, dissolving the compile-time dependency. See [02-CELL.md](../../unified/02-CELL.md) on how Bus topics replace direct crate coupling.

---

## 5. L5: Self-Evolution

The five-layer model is stable for a system that executes fixed specifications. But the scaffold thesis implies something stronger: the scaffold must improve *itself*. When L4 treats the spec itself as an evolvable artifact, a sixth layer emerges.

### 5.1 What L5 looks like

L5 is the layer where the system's own architecture is a Signal in its own Store. The specification documents are not external to the system -- they are machine-parseable, agent-readable, and proposable-for-amendment.

```rust
// L5: The spec as a runtime artifact
struct SpecSignal {
    kind: Kind::Specification,
    body: Body::Toml(spec_content),
    version: SemVer,
    provenance: Provenance {
        author: AgentId | HumanId,
        attestation: Option<Attestation>,
        // Every spec change has verifiable authorship
    },
    lineage: Vec<SignalRef>,  // which prior spec version this derives from
}

// L5 Cells:
// - SpecProposalCell: React protocol, watches episodes + gate verdicts,
//   proposes spec amendments when pattern quality degrades
// - SpecVerifyCell: Verify protocol, checks proposed amendments against
//   invariants (does the new spec still produce the same layer derivation?)
// - SpecApprovalCell: React protocol, human-in-the-loop approval gate
//   (the system cannot modify its own verification pipeline unsupervised)

// The Variance Inequality constrains L5:
// The verifier of a spec change must be "spectrally cleaner" than
// the proposer. An agent cannot approve its own spec amendments.
```

### 5.2 The recursive VSM

Beer's VSM is explicitly recursive: each System 1 operation can itself be a viable system with its own five subsystems. L5 makes this recursion concrete:

```
L5 (Meta-policy) observes L4 → proposes spec changes
L4 (Orchestration) executes plans under current spec
L3 (Harness) verifies execution against spec constraints
L2 (Scaffold) assembles context using spec-defined templates
L1 (Framework) connects to models using spec-defined routing
L0 (Runtime) persists/transports using spec-defined protocols
```

L5 does not add new protocols. It reuses React (watch episodes, propose changes), Verify (check amendments against invariants), and Store (persist spec versions with lineage). The novelty is that the *target* of these protocols is the specification itself, not domain data.

### 5.3 Constraints that prevent runaway self-modification

Three structural constraints keep L5 bounded:

1. **Variance Inequality**: The verifier of a spec change must be heterogeneous from the proposer. No LLM judges its own spec amendments.

2. **Lexicographic corrigibility** (Nayebi 5-head): deference > switch > truth > impact > task. Even at L5, the deference head always dominates task performance. The system cannot modify its corrigibility ordering.

3. **Human-in-the-loop gate**: L5 spec changes pass through a human approval Verify Cell that sits *outside the modifiable surface*. The system can propose, but cannot unilaterally enact.

```rust
// L5 constraint enforcement
fn verify_spec_amendment(
    proposed: &SpecSignal,
    current: &SpecSignal,
    verifier: &dyn Verify,  // MUST be heterogeneous from proposer
    human_gate: &dyn Verify, // human-in-the-loop, outside modifiable surface
) -> Verdict {
    // 1. Structural invariant: does the amended spec still derive the
    //    same 5-layer topology from protocol dependencies?
    let structural = verifier.check(proposed, Criterion::LayerDerivation);

    // 2. Corrigibility invariant: does the amended spec preserve the
    //    lexicographic ordering of the 5 corrigibility heads?
    let corrigibility = verifier.check(proposed, Criterion::CorrigibilityOrder);

    // 3. Human approval: non-bypassable external gate
    let human = human_gate.check(proposed, Criterion::HumanApproval);

    // All three must pass. Conjunctive, not weighted-sum.
    Verdict::conjunctive(&[structural, corrigibility, human])
}
```

---

## 6. Cybernetic Loops

The architecture is not static. Each layer has Lenses (Observe protocol) that watch its health, and Loops (Graph with feedback edges) that tune its parameters.

### 6.1 Per-layer health Lenses

| Layer | Lens | What It Observes | Published Pulse Topic |
|---|---|---|---|
| L0 Runtime | `StoreHealthLens` | Signal count, query latency, GC pressure, Bus delivery rate | `lens.store.health`, `lens.bus.health` |
| L1 Framework | `ConnectHealthLens` | LLM error rates, MCP availability, routing latency | `lens.connect.health` |
| L2 Scaffold | `ComposeHealthLens` | Token budget utilization, section effect scores, context hit rate | `lens.compose.health` |
| L3 Harness | `VerifyHealthLens` | Gate pass rates (per rung), adaptive threshold drift, false positive rate | `lens.verify.health` |
| L4 Orchestration | `ReactHealthLens` | Plan completion rate, task failure clustering, policy reaction latency | `lens.react.health` |
| L5 Meta | `SpecHealthLens` | Spec amendment proposal rate, amendment acceptance rate, post-amendment quality delta | `lens.spec.health` |

### 6.2 Layer boundary Loops

The boundary between adjacent layers is itself a tunable parameter. A Loop (Graph with feedback edge) adjusts where responsibility shifts:

```rust
// Example: the L2/L3 boundary Loop
// When Verify (L3) rejects too many Compose (L2) outputs,
// the Loop tightens the Compose budget or enriches context.

struct ComposeVerifyBoundaryLoop {
    // Observe: watch gate pass rates for compose-originated Signals
    lens: VerifyHealthLens,
    // React: when pass rate drops below threshold, adjust compose params
    policy: BoundaryAdjustmentPolicy,
    // Feedback edge: adjusted params flow back to Compose cells
    feedback_topic: Topic,  // "boundary.l2_l3.adjustment"
}

impl Graph for ComposeVerifyBoundaryLoop {
    fn edges(&self) -> Vec<Edge> {
        vec![
            // lens observes verify outcomes
            Edge::new(self.lens.id(), self.policy.id(), SignalType::GateMetrics),
            // policy publishes adjustment
            Edge::new(self.policy.id(), self.lens.id(), SignalType::Adjustment),
            // ^^^ feedback edge: output feeds back to input
        ]
    }
}
```

### 6.3 The autocatalytic cycle, formalized

The compound improvement math (Kauffman 1993) is not metaphorical. Each layer's Loop improves its own stratum, and the improvement compounds through the dependency lattice:

```
Compose (L2) improves prompts
  → Verify (L3) receives better inputs, catches subtler errors
  → React (L4) extracts richer heuristics from higher-quality episodes
  → Score (L1) uses better heuristics for routing decisions
  → Compose (L2) has better-scored context to work with
  → cycle continues
```

If each layer contributes independent improvement factor `alpha_i`, the compound effect is `1 - prod(1 - alpha_i)`. For five layers each contributing 10%: `1 - 0.9^5 = 0.41` -- 41% compound improvement from 10% individual gains.

---

## 7. What This Enables

1. **Layer replacement without cascade failure** -- swap `roko-gate` (L3) without touching L0, L1, L2, or L4, because the layer boundary is a structural invariant of the protocol dependency lattice
2. **Formal layer derivation** -- the five layers are theorems, not opinions; new protocols can be added and the topology re-derived mechanically
3. **L5 self-evolution** -- the specification becomes a first-class runtime artifact, proposable-for-amendment under structural constraints
4. **Cybernetic regulation** -- every layer has a health Lens and boundary-tuning Loop, making the architecture self-stabilizing

## 8. Feedback Loops

| Loop | Input | Output | Cadence |
|---|---|---|---|
| Layer health monitoring | Per-layer Lens Pulses | Health score Signals persisted in Store | Gamma (every tick) |
| Boundary adjustment | Gate pass rate drift | Compose budget / enrichment parameter changes | Theta (plan-level) |
| Autocatalytic compound | Cross-layer improvement signals | Updated heuristics, thresholds, routing weights | Theta/Delta |
| L5 spec evolution | Accumulated episode quality trends | Spec amendment proposals (human-gated) | Delta (infrequent) |

## 9. Open Questions

1. **Layer derivation automation**: Can the protocol dependency lattice be extracted from Rust trait bounds at compile time, making the layer assignment machine-checkable? The information is present in `where` clauses but not currently harvested.

2. **Cross-cut injection overhead**: Passing cross-cuts as `&dyn Trait` objects introduces dynamic dispatch. For hot paths (Gamma-speed scoring), is the vtable indirection measurable? Preliminary: ~2ns per vtable call vs ~8us for HDC inference, so likely noise.

3. **L5 spec versioning semantics**: When a spec amendment changes the protocol dependency graph (e.g., adding a new protocol), the layer derivation changes. How should the system handle the transition period where two layer topologies coexist? Feature flags? Parallel Graphs?

4. **Empirical validation of compound improvement**: The `1 - prod(1 - alpha_i)` formula assumes independence between layer improvements. In practice, improvements are correlated (better context helps better gates). The actual compound factor could be higher or lower. Needs measurement on real self-hosting runs.

5. **Missing Lens**: There is no `BusSaturationLens` that watches for topic congestion (too many Pulses per topic per second). Bus backpressure is a potential failure mode that no current Lens observes.
