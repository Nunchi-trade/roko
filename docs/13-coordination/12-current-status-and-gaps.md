# Current Status and Gaps: Coordination Implementation State

> **Layer**: All layers (L0–L4) — this sub-doc surveys the implementation status of
> coordination features across the entire stack
>
> **Synapse traits**: All six — coordination touches every trait
>
> **Prerequisites**: All preceding sub-docs in `13-coordination/`


> **Implementation**: Specified

> **See also**: `../../tmp/refinements/09-phase-2-implications.md`,
> `../00-architecture/01-naming-and-glossary.md`

---

## Overview

This sub-doc provides an honest assessment of which coordination features described in the
preceding sub-docs are implemented, which are scaffolded, and which remain unbuilt. The purpose
is to give implementation agents a clear picture of where to focus effort and to prevent
duplicate work.

The assessment is organized by sub-doc, mapping each major feature to its current
implementation status. For REF09, the key test is whether each coordination capability is
described as durable Engrams in Substrate plus live Pulses on the Bus rather than as a special
side channel.

---

## Status Legend

| Status | Meaning |
|--------|---------|
| **Wired** | Code exists, integrated into runtime, tested via CLI |
| **Scaffold** | Struct/trait exists, but logic is stub/incomplete |
| **Design** | Specified in PRD/design docs, no code exists |
| **Gap** | Not yet designed or specified; identified as needed |

---

## 00: Stigmergy Theory

| Feature | Status | Location | Notes |
|---------|--------|----------|-------|
| Engram as content-addressed unit | **Wired** | `roko-core` durable record type | Content-addressed durable record with hash, body, score, parents, and tags is already the right storage primitive for pheromone deposits |
| Substrate trait (store/query) | **Wired** | `roko-core/src/traits.rs` | `Substrate` trait with `store()` and `query()` |
| Scorer trait | **Wired** | `roko-core/src/traits.rs` | `Scorer` trait with `score()` |
| Policy trait | **Wired** | `roko-core/src/traits.rs` | `Policy` trait with `observe()` and `react()` |
| Universal cognitive loop | **Wired** | `roko-cli/src/orchestrate.rs` | query → score → route → compose → act → verify → write → react |
| Stigmergic loop (deposit → propagate → sense → respond) | **Scaffold** | Partially wired via durable record persistence | Durable deposit is partly present, but the full `Substrate.put()` + `Bus.publish()` coordination loop is not yet integrated |

---

## 03: Digital Pheromones

| Feature | Status | Location | Notes |
|---------|--------|----------|-------|
| `Pheromone` struct | **Design** | `refactoring-prd/04-knowledge-and-mesh.md` | Typed coordination view defined in the PRD; intended to sit on top of Engram storage and Bus announcements |
| `PheromoneKind` enum | **Design** | `refactoring-prd/04-knowledge-and-mesh.md` | 7 universal + domain-specific + Custom(String). Not yet in code. |
| Exponential decay function | **Design** | PRD + legacy `bardo-backup/prd/02-mortality/10-clade-ecology.md` | `pheromone_decay()` fully specified with formula; not yet implemented in Roko crates |
| Confirmation mechanics | **Design** | PRD | Half-life extension via confirmation count; anti-spoofing via reputation weighting. Not implemented. |
| Pheromone-enriched context assembly | **Scaffold** | `roko-compose` crate | The `SystemPromptBuilder` exists and supports 6-layer prompts, but does not yet include ambient pheromone summary from shared Substrate or `mesh.pheromone.deposited` Pulses |

---

## 04: Pheromone Kinds

| Feature | Status | Location | Notes |
|---------|--------|----------|-------|
| Universal kinds (Threat, Opportunity, Wisdom) | **Design** | PRD | Not yet in code |
| Domain-specific kinds (Alpha, Pattern, Anomaly, Consensus) | **Design** | PRD | Not yet in code |
| Custom(String) extensibility | **Design** | PRD | Not yet in code |
| Kind-specific decay profiles | **Design** | PRD | Specified in PRD; needs implementation |
| Kind interaction rules (Threat suppression, Pattern→Wisdom promotion) | **Design** | PRD | Not yet in code |

---

## 05: Pheromone Scope

| Feature | Status | Location | Notes |
|---------|--------|----------|-------|
| `PheromoneScope` enum | **Design** | PRD | Local/Mesh/Global specified; not yet in code |
| Local scope storage | **Wired** (partial) | `roko-fs` crate | `FileSubstrate` stores Engrams locally via JSONL. Could serve as local pheromone store with tagging. |
| Mesh scope propagation | **Design** | PRD | Requires Agent Mesh transport (not implemented) |
| Global scope (Korai chain) | **Design** | PRD | Requires chain integration (deferred to Tier 6) |
| Scope promotion gates | **Design** | PRD | Local → Mesh → Global promotion logic not implemented |
| Trust discounting by scope | **Design** | PRD | Confidence multipliers (0.80 for Mesh, 0.50 for Global) not yet applied |

---

## 06: Agent Mesh Sync

| Feature | Status | Location | Notes |
|---------|--------|----------|-------|
| WebSocket transport | **Design** | Historical mesh transport docs | Intended as one backend for `MeshBus`; not yet implemented in Roko |
| Iroh P2P transport | **Design** | Historical mesh transport docs | Intended as a lower-latency backend for `MeshBus` and `MeshSubstrate`; not implemented |
| ERC-8004 agent discovery | **Design** | PRD | Service endpoint resolution specified; not implemented |
| mDNS local discovery | **Design** | PRD (Iroh provides built-in mDNS) | Would come with Iroh integration |
| Version vector deduplication | **Design** | `bardo-backup/prd/20-styx/03-clade-sync.md` | Lamport/Fidge vector clocks specified; not implemented |
| Store-and-forward for offline agents | **Design** | `bardo-backup/prd/20-styx/03-clade-sync.md` | 7-day TTL store-and-forward specified; not implemented |
| Gossip pub/sub for pheromones | **Design** | PRD | `MeshBus` gossip for topics such as `mesh.pheromone.deposited` is specified; not implemented |
| Content-addressed blob transfer | **Design** | PRD | `MeshSubstrate`-style durable replication over content-addressed transfer is specified; not implemented |

---

## 07: Morphogenetic Specialization

| Feature | Status | Location | Notes |
|---------|--------|----------|-------|
| `MorphogeneticState` struct | **Design** | `bardo-backup/prd/02-mortality/10b-morphogenetic-specialization.md` | Full Rust struct specified in legacy PRD; not yet in Roko crates |
| `MorphogeneticParams` struct | **Design** | Legacy PRD | Parameters (alpha, beta, mu, sigma_noise) specified |
| Reaction-diffusion update rule | **Design** | Legacy PRD | Full `update()` function specified; not implemented |
| `specialization_index()` | **Design** | Legacy PRD | Shannon entropy calculation specified; not implemented |
| `niche_competition()` | **Design** | Legacy PRD | Cosine similarity calculation specified; not implemented |
| Role vector broadcast | **Design** | `bardo-backup/prd/20-styx/03-clade-sync.md` | Piggybacks on batch sync; requires Agent Mesh |
| Niche vacancy alerts | **Design** | Legacy PRD | Immediate push on agent departure; requires Agent Mesh |
| Role conflict alerts | **Design** | Legacy PRD | Threshold-triggered; requires Agent Mesh |

---

## 08: Permissioned Subnets

| Feature | Status | Notes |
|---------|--------|-------|
| Subnet scope variant | **Design** | Specified in this migration; not in legacy PRD |
| Access control (invite/role/reputation) | **Design** | Conceptual design; no specification |
| Publishing gate | **Design** | Conceptual design; no specification |
| Internal reputation | **Design** | Conceptual design; no specification |

**Note**: Permissioned subnets are the least specified of all coordination features. The design
in `08-permissioned-subnets.md` is based on inference from the legacy Styx privacy model
and Buchanan's club goods theory. Implementation would require significant additional design
work.

---

## 09: Stigmergy Scaling

| Feature | Status | Notes |
|---------|--------|-------|
| Scaling analysis | **Design** | This sub-doc is analytical, not implementation |
| Gossip scaling testing | **Gap** | No load testing or scaling benchmarks exist |
| Pheromone field size limits | **Gap** | No empirical data on practical limits |

---

## 10: Exponential Flywheel

| Feature | Status | Notes |
|---------|--------|-------|
| Autocatalytic knowledge networks | **Design** | Enabled by Substrate+Scorer; no explicit detection |
| Superlinear scaling measurement | **Gap** | No benchmarking framework exists |
| Knowledge distillation pipeline | **Scaffold** (partial) | Pheromone promotion (Pattern→Wisdom→Consensus) specified but not implemented |
| Cross-domain insight detection | **Design** | HDC similarity threshold (0.526) specified; HDC fingerprints built in `bardo-primitives` but not wired into pheromone system |

---

## 11: Collective Intelligence Metrics

| Feature | Status | Location | Notes |
|---------|--------|----------|-------|
| C-Factor estimation | **Design** | `bardo-backup/tmp/agent-chain/proving-collective-intelligence.md` | Formula specified; not implemented |
| Composite C-Score | **Design** | This migration | Four diagnostic signals specified; not implemented |
| Turn-taking equality metric | **Design** | This migration | Shannon entropy of deposits; not implemented |
| Knowledge flow rate metric | **Design** | This migration | Deposit-to-confirmation latency; not implemented |
| Cross-domain transfer metric | **Design** | This migration | Cross-domain sensing fraction; not implemented |
| Emergent coordination metric | **Design** | This migration | Stigmergic vs orchestrated fraction; not implemented |
| Dashboard integration | **Scaffold** | `roko-cli` dashboard subcommand | Text-mode dashboard exists but does not display C-Factor or C-Score |
| A/B testing framework | **Design** | This migration | Clustered standard errors specified; not implemented |

---

## Priority Ordering for Implementation

Based on the dependency graph and the self-hosting workflow:

### Tier 1: Foundation (Prerequisites for All Coordination)

1. **`Pheromone` struct and `PheromoneKind` enum** — Core type system. Everything else depends
   on this. Add to `roko-core`.
2. **`PheromoneScope` enum** — Scope system. Add to `roko-core`.
3. **`pheromone_decay()` function** — Decay mechanics. Add to `roko-core` or `roko-fs`.
4. **Pheromone storage in FileSubstrate** — Extend `roko-fs` to store and query pheromone
   Engrams with kind/scope/intensity filtering.

### Tier 2: Local Stigmergy (Single Agent Benefits)

5. **Pheromone-enriched context assembly** — Extend `roko-compose` SystemPromptBuilder to
   include ambient pheromone summary in agent prompts. This provides value even for single
   agents (local pheromones guide the agent's own future actions).
6. **Local pheromone deposit/sense in orchestrate.rs** — Wire pheromone deposit after gate
   results (Threat on failure, Opportunity on success) and sense before task dispatch.

### Tier 3: Collective Coordination (Multi-Agent Benefits)

7. **Agent Mesh transport (WebSocket)** — Minimum viable Mesh scope implementation. Could
   start with a simple WebSocket relay for Collective sync.
8. **Version vector deduplication** — Required for reliable Mesh sync.
9. **Morphogenetic specialization** — Add `MorphogeneticState`, `MorphogeneticParams`, and
   `update()` to enable emergent role differentiation.
10. **Pheromone confirmation mechanics** — Half-life extension via independent confirmation.

### Tier 4: Metrics and Optimization

11. **C-Factor estimation** — Instrument the orchestrator to measure collective intelligence.
12. **Dashboard integration** — Display C-Factor and C-Score in `roko dashboard`.
13. **Adaptive optimization** — Use metrics to auto-tune coordination parameters.

### Tier 5: Advanced Features (Post Self-Hosting)

14. **Iroh P2P transport** — Direct agent-to-agent communication.
15. **Permissioned subnets** — Private Mesh scopes.
16. **Global scope (Korai chain integration)** — On-chain pheromone publishing.
17. **Cross-domain insight detection** — HDC-based similarity matching.
18. **A/B testing framework** — Rigorous evaluation of coordination mechanisms.

---

## Existing Code Assets

Several existing crates contain relevant code that could be leveraged:

| Crate | Relevant Code | Status | How to Leverage |
|-------|--------------|--------|-----------------|
| `roko-core` | `Signal` struct (= Engram), 6 Synapse traits | **Wired** | Extend with pheromone types |
| `roko-fs` | `FileSubstrate` (JSONL store, GC) | **Wired** | Add pheromone-specific queries |
| `roko-compose` | `SystemPromptBuilder` (6-layer prompts) | **Wired** | Add pheromone enrichment layer |
| `roko-learn` | Efficiency events, cascade router, experiments | **Wired** | Use as instrumentation for C-Factor |
| `roko-gate` | 11 gates, adaptive thresholds | **Wired** | Gate results → pheromone deposits |
| `roko-conductor` | 10 watchers, circuit breaker | **Wired** | Monitor pheromone field health |
| `bardo-primitives` | HDC fingerprints, tiering | **Built** (not wired) | Cross-domain similarity detection |
| `roko-golem` | Chain witness, daimon, dreams | **Phase 2+** | Global scope, chain integration |

---

## Key Gaps and Open Questions

### Gap 1: No Pheromone Type System in Code

The `PheromoneKind` and `PheromoneScope` enums exist only in PRD documents, not in the
codebase. This is the single most impactful gap — without these types, no coordination feature
can be implemented.

### Gap 2: No Agent Mesh Transport

Neither WebSocket relay nor Iroh P2P is implemented. Without transport, Mesh-scope
coordination is impossible. This blocks all multi-agent coordination features.

### Gap 3: No Morphogenetic Implementation

The reaction-diffusion dynamics are fully specified (parameters, update rule, convergence
analysis) but not implemented in any crate. This means Collectives of identical agents will
produce redundant work.

### Gap 4: No Collective Intelligence Measurement

The C-Factor and C-Score are defined but not instrumented. Without measurement, there is no
way to know if coordination is working or to tune parameters.

### Open Question 1: Pheromone Kind Extensibility

Should domain plugins register custom pheromone kinds at startup, or should the `Custom(String)`
variant be sufficient? Registration would enable kind-specific routing and scoring optimizations
but adds configuration complexity.

**Decision**: Start with `Custom(String)` (simpler, no registration needed). Add registration
only if performance profiling shows that kind-specific optimizations are needed.

### Open Question 2: Mesh Transport Priority

Should WebSocket relay or Iroh P2P be implemented first? WebSocket is simpler (standard HTTP
infrastructure) but centralized. Iroh is decentralized but requires more implementation effort.

**Decision**: Start with WebSocket relay (simpler, faster to implement, provides
store-and-forward). Add Iroh later for direct P2P performance and decentralization benefits.

### Open Question 3: Morphogenetic Strategy Dimensions

The legacy PRD uses 8 domain-specific dimensions (momentum, mean_reversion, lp, risk, etc.)
from the DeFi domain. Roko needs domain-agnostic dimensions. The migration proposes:
depth, breadth, execution, verification, time_horizon, exploration, exploitation, coordination.

**Decision**: Use the domain-agnostic 8 dimensions as default. Allow domain plugins to
override via `roko.toml` configuration.

---

## Summary

The coordination layer is the **most specified but least implemented** part of Roko's
architecture. The theoretical foundations are strong (extensive academic citations, detailed
Rust struct definitions, convergence analysis), but the codebase has:

- **0 pheromone types** in code (all in PRD)
- **0 transport implementations** for Mesh scope
- **0 morphogenetic code** (all in PRD)
- **0 collective intelligence metrics** (all in PRD)

The foundation crates (`roko-core`, `roko-fs`, `roko-compose`, `roko-learn`, `roko-gate`) are
wired and provide the infrastructure on which coordination features can be built. The
implementation plan follows a tier progression from types (Tier 1) through local stigmergy
(Tier 2), collective coordination (Tier 3), metrics (Tier 4), and advanced features (Tier 5).

---

## References

(See individual sub-docs for topic-specific references.)

---

## Cross-References

- All preceding sub-docs (00–11) — this sub-doc surveys their implementation status
- `INDEX.md` — Table of contents for the full coordination topic
- `../../tmp/refinements/09-phase-2-implications.md` — Phase 2+ two-fabric framing for Dreams, Mesh, and coordination
- `../00-architecture/01-naming-and-glossary.md` — canonical Bus, Pulse, Topic, MeshBus, and MeshSubstrate vocabulary
