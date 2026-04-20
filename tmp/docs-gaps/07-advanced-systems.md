# Advanced Systems -- P2 Capability Types Not Built

These are advanced capability types from docs 25-29 that have no code. They represent the cognitive architecture's advanced features. All are P2 priority -- build after P0/P1 work is done.

Each section references the specific doc and lists the types that need to be implemented.

## Checklist

### AS-01: Attention System (doc 25)
- [x] Implement attention-as-currency types

**Spec** (doc 25-attention-as-currency.md): Attention is a scarce resource allocated through market-like mechanisms.
**Types to build**:
- [x] `AttentionToken` -- unit of attention currency
- [x] `AttentionBudget` -- per-agent attention allocation
- [x] `AttentionAuction` -- mechanism for agents to bid for attention
- [x] Related allocation and accounting types
**Where**: `crates/roko-runtime/src/heartbeat_attention.rs`
**Accept when**:
- [x] All types defined and documented
- [x] Config section CS-02 (`[attention]`) exists
- [x] Unit tests for allocation logic (25+ tests: VCG auction, carryover budget, POMDP, bidders, governor)
- [x] `cargo test -p roko-runtime` passes

### AS-02: Cognitive Immune System (doc 26)
- [x] Implement cognitive immune system types

**Spec** (doc 26-cognitive-immune-system.md): System for detecting and quarantining compromised knowledge.
**Types to build**:
- [x] `enum Taint { Hallucination, Contradiction, UnverifiedSource, ToolMisuse, ... }` -- typed taint classification (also TC-02 in `01-type-corrections.md`)
- [x] `Quarantine` -- isolation for tainted engrams (`QuarantineVault` in `roko-core/src/immune.rs`)
- [x] `IncidentLink` -- connects related taint incidents
- [x] Immune response and recovery types (`ImmuneResponse`, `ResponseAction`)
**Where**: `roko-core/src/provenance.rs` (Taint enum) + `roko-core/src/immune.rs` (quarantine/incident)
**Accept when**:
- [x] Typed Taint enum replaces `tainted: bool`
- [x] Quarantine mechanism exists (`QuarantineVault` with screen/quarantine/review/drain_resolved)
- [x] Config section CS-03 (`[immune]`) exists
- [x] `cargo test -p roko-core -- immune` passes (19 tests)

### AS-03: Temporal Knowledge Topology (doc 27)
- [x] Implement temporal reasoning types

**Spec** (doc 27-temporal-knowledge-topology.md): Temporal relations over knowledge state, including Allen interval algebra.
**Types to build**:
- [x] `TemporalRelation` -- relationships between knowledge states
- [x] `KnowledgeEpoch` -- temporal boundary of knowledge validity
- [x] `AllenRelation` enum -- the 13 Allen interval relations (before, after, meets, overlaps, etc.)
- [x] Temporal query and indexing types (`TemporalIndex`, `TemporalInterval`)
**Where**: `crates/roko-neuro/src/temporal.rs`
**Accept when**:
- [x] Allen relations implemented (13 relations with compute/inverse/is_concurrent/is_sequential)
- [x] Temporal queries on knowledge store work (`TemporalIndex::entries_at`, `concurrent_with`, `epoch_at`)
- [x] Config section CS-04 (`[temporal]`) exists
- [x] `cargo test -p roko-neuro -- temporal` passes (25 tests)

### AS-04: Emergent Goal Structures (doc 28)
- [x] Implement goal emergence types

**Spec** (doc 28-emergent-goal-structures.md): Goals emerge from patterns in agent behavior rather than being explicitly programmed.
**Types to build**:
- [x] `GoalSeed` -- initial pattern that may become a goal
- [x] `GoalTree` -- hierarchical goal structure with observe/promote/prune/decay
- [x] Goal scoring, pruning, and promotion logic (`GoalNode::from_seed`, `update_progress`, `should_prune`)
**Where**: `crates/roko-daimon/src/goals.rs`
**Accept when**:
- [x] Goal seeds can be detected from behavior patterns
- [x] Goal tree can be built and pruned (full lifecycle: observe -> promote -> hierarchy -> prune)
- [x] Config section CS-05 (`[goals]`) exists
- [x] `cargo test -p roko-daimon -- goals` passes (13 tests)

### AS-05: Cognitive Energy Model (doc 29)
- [x] Implement cognitive energy types

**Spec** (doc 29-cognitive-energy-model.md): Energy budget model for cognitive operations -- agents have metabolic costs.
**Types to build**:
- [x] `EnergyPool` -- available cognitive energy (spend/replenish/throttle)
- [x] `CognitiveMetabolism` -- energy consumption rates (per-operation + global multiplier)
- [x] Energy accounting and throttling types (`EnergyTransaction`, `EnergyLedger`, `OperationKind`)
**Where**: `crates/roko-runtime/src/energy.rs`
**Accept when**:
- [x] Energy pools track consumption (spend/replenish/adjust with ledger)
- [x] Metabolism rates influence agent behavior (economy/performance modes, throttle_level)
- [x] Config section CS-06 (`[energy]`) exists
- [x] `cargo test -p roko-runtime -- energy` passes (14 tests)

### AS-06: Demurrage trait (doc 04, 18)
- [x] Implement Demurrage as a trait

**Spec** (doc 04-decay-variants.md, doc 18-decay-tier-matrix.md): Time-based value decay formalized as a trait.
**Current code**: Decay enum exists but Demurrage as a separate trait does not.
**Accept when**:
- [x] `trait Demurrage` exists -- `crates/roko-core/src/demurrage.rs:15` with balance/demurrage_rate/tick/replenish/is_depleted methods
- [ ] Integrates with Decay system -- Demurrage trait and Decay enum coexist but do not directly reference each other; they address different concerns (Decay=engram policy, Demurrage=value/attention balance)
- [x] Config section CS-01 (`[demurrage]`) exists -- `DemurrageConfig` at schema.rs:2799, `[demurrage]` section in hot_reload.rs
- [ ] `cargo test --workspace`
**Priority**: P1 (closer to core than other AS items)

### AS-07: BayesianConfidenceUpdater (doc 11)
- [x] Implement Bayesian confidence updating

**Spec** (doc 11-dual-process-and-active-inference.md): Active inference with Bayesian confidence updates.
**Current code**: `roko_learn::bayesian_confidence::BayesianConfidenceUpdater` implements a Beta-Binomial conjugate model. Supports uniform/informative priors, single/batch/weighted observations, merge, credible intervals, variance tracking. 11 unit tests verify prior behavior, update mechanics, interval narrowing, and evidence merging.
**Accept when**:
- [x] `BayesianConfidenceUpdater` type exists
- [x] Can update confidence based on evidence
- [x] `cargo test --workspace`

### AS-08: ColdSubstrate trait (doc 07)
- [x] Implement archival substrate

**Spec** (doc 07-substrate-trait.md): ColdSubstrate for aged-out engrams that no longer need hot-path access.
**Accept when**:
- [x] `trait ColdSubstrate` exists alongside `Substrate` (in `roko-core/src/traits.rs`)
- [x] Migration path from hot to cold storage (`ArchiveColdSubstrate` in `roko-fs/src/cold_substrate.rs` + `SubstrateMigrator`)
- [x] `cargo test -p roko-fs -- cold_substrate` passes (8 tests)

### AS-09: Bus backends (doc 07b)
- [x] Implement Bus backend variants (3 of 4; ChainBus deferred to chain phase)

**Spec** (doc 07b): Multiple Bus implementations for different transport needs.
**Types to build**:
- [x] `BroadcastBus` -- in-process broadcast (no replay)
- [x] `MemoryBus` -- in-memory with bounded replay ring
- [x] `MultiBus` -- fan-out to multiple backends via `BusErased` trait
- [ ] `ChainBus` -- chain-witnessed transport (deferred: requires Korai chain layer)
**Where**: `crates/roko-core/src/bus_backends.rs`
**Depends on**: K-02 (Bus trait) -- satisfied
**Accept when**:
- [x] Each backend implements `Bus` trait
- [x] `cargo test -p roko-core -- bus_backends` passes (10 tests)

### AS-10: Target crate splits (doc 15)
- [ ] Split crates as specified in crate map

**Spec** (doc 15-crate-map.md): Target crate structure includes:
- [ ] `roko-bus` -- standalone Bus fabric crate
- [ ] `roko-hdc` -- standalone HDC crate (currently in roko-primitives)
- [ ] `roko-spi` -- Service Provider Interface
- [ ] `roko-defaults` -- split from roko-std (defaults)
- [ ] `roko-tools` -- split from roko-std (tools)
**Accept when**:
- [ ] Each crate exists as a workspace member
- [ ] Original code moved, not duplicated
- [ ] `cargo build --workspace`

### AS-11: ReinforceKind enum
- [x] Implement typed reinforcement categories

**Spec** (implied by learning/feedback docs): Typed enum for reinforcement signal kinds.
**Current code**: `roko_learn::reinforce_kind::ReinforceKind` is a `#[non_exhaustive]` enum with 16 variants covering gates, routing, prompts, skills, playbooks, conductor interventions, dream hypotheses, and cost efficiency. Includes `ReinforceSignal` wrapper with timestamp, task/plan/agent context. `is_positive()`/`is_negative()`/`label()`/`reward_value()` methods. 6 unit tests verify polarity, construction, and serde roundtrip.
**Accept when**:
- [x] `enum ReinforceKind` exists with meaningful variants
- [x] Used in feedback/learning paths
