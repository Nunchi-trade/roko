# Coordination: Stigmergy, Pheromones, and Collective Intelligence

> **Topic**: 13-coordination
>
> **Scope**: Multi-agent coordination in Roko — stigmergy theory, digital pheromones,
> pheromone types and scoping, Agent Mesh transport, morphogenetic specialization,
> exponential flywheel mechanisms, collective intelligence metrics, and current
> implementation status.

---

## Summary

This topic covers how Roko agents coordinate without direct communication, using **stigmergy**
— indirect coordination through environmental modification. In the two-fabric model, the
environment is a multi-layered pheromone field composed of typed Engrams in shared Substrates
and matching Pulses on the Bus. These digital pheromones carry coordination signals (threats,
opportunities, patterns, wisdom) that propagate through three scopes (Local, Mesh, Global)
via the Agent Mesh, now framed as MeshBus + MeshSubstrate rather than a separate trait family.

On top of the pheromone field, **morphogenetic specialization** uses Turing reaction-diffusion
dynamics to produce emergent role differentiation in Collectives of initially identical agents.
Activation (profitable strategies reinforce slowly via experience) and inhibition (Collective-
wide pheromone signals propagate fast via the Agent Mesh) satisfy Turing's instability
condition, causing stable specialist patterns to emerge without central role assignment.

The **exponential flywheel** describes ten mechanisms (autocatalytic knowledge networks,
superlinear scaling, Reed's Law, knowledge distillation cascades, evolutionary dynamics,
collective calibration, cross-domain resonance, niche construction, information-theoretic
compounding, transactive memory) that produce superlinear growth in collective intelligence.
The **C-Factor** and composite **C-Score** metrics quantify coordination effectiveness.

---

## Prerequisites

- Familiarity with Roko's Synapse Architecture (6 traits: Substrate, Scorer, Gate, Router,
  Composer, Policy) — see `00-architecture/`
- Understanding of the Engram type (content-addressed, scored, decaying unit of cognition) —
  see `00-architecture/`
- Basic awareness of the 5-layer taxonomy (L0 Runtime through L4 Orchestration) — see
  `00-architecture/`

---

## Table of Contents

| # | File | Title | Lines | Summary |
|---|------|-------|-------|---------|
| 00 | [00-stigmergy-theory.md](00-stigmergy-theory.md) | Stigmergy Theory | ~470 | Grassé 1959, formal definition, two forms (sematectonic/marker-based), why stigmergy over direct communication, scalability/robustness/asynchrony analysis, Grossman-Stiglitz paradox resolution, **information-theoretic analysis** (stigmergic channel capacity, entropy rate, transfer entropy, design implications) |
| 01 | [01-stigmergy-beyond-termites.md](01-stigmergy-beyond-termites.md) | Stigmergy Beyond Termites | ~280 | Biological examples (ants, bees, bacteria, spiders), human stigmergy (Wikipedia, science, cities, OSS), computational stigmergy (ACO, PSO, swarm robotics), software engineering (code smells, niche construction, information foraging), Constructal Law, self-organized criticality |
| 02 | [02-git-as-stigmergy.md](02-git-as-stigmergy.md) | Git as Stigmergy | ~250 | Repository as stigmergic environment, sematectonic (code structure) and marker-based (commits, branches, CI) signals, multi-agent worktree model, base+overlay pattern, declared contracts, pheromone traces in codebases |
| 03 | [03-digital-pheromones.md](03-digital-pheromones.md) | Digital Pheromones | ~490 | Two-fabric pheromone framing, typed Engrams plus Bus Pulses, decay formula, confirmation mechanics, anti-spoofing via reputation, pheromone field operations, pheromone-enriched context assembly, complete lifecycle, **pheromone interference model** (SINR framework, cross-kind interference matrix, anti-saturation mechanisms, mitigation strategies) |
| 04 | [04-pheromone-kinds.md](04-pheromone-kinds.md) | Pheromone Kinds | ~450 | PheromoneKind enum (Threat/Opportunity/Wisdom/Alpha/Pattern/Anomaly/Consensus/Custom), three-tier taxonomy, kind interactions, Alpha paradox, Pattern→Wisdom promotion, Consensus stability, **pheromone-driven task allocation** (response threshold model, Hill function, emergent division of labor) |
| 05 | [05-pheromone-scope.md](05-pheromone-scope.md) | Pheromone Scope | ~260 | PheromoneScope enum (Local/Mesh/Global), three-level hierarchy, Constructal Law connection, trust discounting, scope promotion, cross-scope composition, permissioned subnets preview |
| 06 | [06-agent-mesh-sync.md](06-agent-mesh-sync.md) | Agent Mesh Sync | ~470 | Dual-transport architecture (WebSocket + Iroh), Bus projection integration, MeshBus + MeshSubstrate split, connection registry, store-and-forward, iroh-gossip pheromone propagation, iroh-blobs knowledge exchange, ERC-8004 discovery, failure modes, security model, **partition tolerance** (AP design, partition-aware morphogenetics, post-partition reconciliation, Byzantine agent detection) |
| 07 | [07-morphogenetic-specialization.md](07-morphogenetic-specialization.md) | Morphogenetic Specialization | ~660 | Turing 1952 reaction-diffusion, strategy concentration vector (8 dimensions), Gierer-Meinhardt kinetics, update rule with activation/inhibition/decay/noise, niche competition, role coordination messages, resource pressure modulation, convergence analysis, **Turing pattern stability analysis** (linear stability, pitchfork bifurcation, Hopf oscillatory instability, Lyapunov stability monitoring, pattern selection), DeLanda assemblage theory |
| 08 | [08-permissioned-subnets.md](08-permissioned-subnets.md) | Permissioned Subnets | ~240 | Private Mesh scopes, access control models (invite/role/reputation), internal reputation, opt-in publishing, scope boundary enforcement, organizational patterns, club goods theory |
| 09 | [09-stigmergy-scaling.md](09-stigmergy-scaling.md) | Stigmergy Scaling | ~250 | O(N×M) vs O(N²) analysis, pheromone field scaling, transport scaling (relay vs gossip), morphogenetic scaling, knowledge sync scaling, practical limits, comparison with consensus and pub-sub |
| 10 | [10-exponential-flywheel.md](10-exponential-flywheel.md) | Exponential Flywheel | ~280 | 10 mechanisms for superlinear growth: autocatalytic networks (Kauffman), superlinear scaling (West/Bettencourt β≈1.15), Reed's Law (2^N), distillation cascades (Hinton), evolutionary dynamics, collective calibration (31.6×), cross-domain resonance (HDC 0.526), niche construction, information-theoretic compounding, transactive memory |
| 11 | [11-collective-intelligence-metrics.md](11-collective-intelligence-metrics.md) | Collective Intelligence Metrics | ~640 | C-Factor (Woolley et al. 2010) from Bus statistics, composite C-Score (4 diagnostic signals), turn-taking equality, knowledge flow rate, cross-domain transfer, emergent coordination, information-theoretic metrics, **collective pathology detection** (information cascades/herding, groupthink/premature consensus, echo chambers, cascading hallucinations, pheromone deadlock, composite pathology dashboard), A/B testing with clustered standard errors, dashboard integration |
| 12 | [12-current-status-and-gaps.md](12-current-status-and-gaps.md) | Current Status and Gaps | ~250 | Per-feature implementation status (Wired/Scaffold/Design/Gap), 5-tier implementation priority, existing code assets, key gaps (no pheromone types, no transport, no morphogenetic code, no metrics), open questions and decisions |

---

## Cross-References

- `00-architecture/` — Engram struct, Synapse Architecture, 5-layer taxonomy
- `01-cognitive-loop/` — The universal loop that pheromones plug into
- `03-neuro/` — NeuroStore as the local pheromone Substrate
- `05-agent-types/` — Agent role system that morphogenetic specialization refines
- `08-chain/` — Korai chain as the Global pheromone scope
- `09-safety/` — Safety constraints on pheromone propagation
- `10-learning/` — Efficiency events and adaptive tuning that feed coordination metrics
- `00-architecture/01-naming-and-glossary.md` — Shared vocabulary for Bus, Pulse, MeshBus, and MeshSubstrate
- `00-architecture/24-cross-section-integration-map.md` — Bus-based integration map for the two-fabric model
- `../../tmp/refinements/09-phase-2-implications.md` — Phase 2+ implications for chain, dreams, mesh, coordination, and heartbeat

---

## Key Academic Citations Across This Topic

| Citation | Sub-Doc(s) | Concept |
|----------|-----------|---------|
| Grassé 1959, *Insectes Sociaux* | 00, 01, 02, 03 | Original stigmergy in termites |
| Turing 1952, *Phil. Trans. Royal Society B* | 07 | Reaction-diffusion morphogenesis |
| Gierer & Meinhardt 1972, *Kybernetik* | 07 | Activator-inhibitor formalization |
| Dorigo, Maniezzo & Colorni 1996, *IEEE SMC-B* | 00, 01 | Ant Colony Optimization |
| Parunak, Brueckner & Sauter 2005, *E4MAS* | 00, 01, 03 | Digital pheromones |
| Theraulaz & Bonabeau 1999, *Artificial Life* | 00, 01 | History of stigmergy |
| Woolley et al. 2010, *Science* 330(6004) | 11 | Collective intelligence factor |
| Nealson, Platt & Hastings 1970, *J. Bacteriology* | 01, 03 | Quorum sensing |
| Bettencourt et al. 2007, *PNAS* | 10 | Superlinear urban scaling β≈1.15 |
| Kauffman 1993, *Origins of Order* | 07, 10 | Autocatalytic networks, edge of chaos |
| Reed 2001, *Harvard Business Review* | 09, 10 | Reed's Law (V~2^N) |
| Odling-Smee, Laland & Feldman 2003 | 01, 10 | Niche construction |
| Bejan 1997, *Int. J. Heat and Mass Transfer* | 01, 05 | Constructal Law |
| Bak, Tang & Wiesenfeld 1987, *PRL* | 01 | Self-organized criticality |
| Hölldobler & Wilson 2008 | 00, 01, 03 | Superorganism theory |
| Hinton, Vinyals & Dean 2015, arXiv:1503.02531 | 10 | Knowledge distillation |
| Lamport 1978, *CACM* | 00, 05, 06 | Time, clocks, vector ordering |
| Fidge 1988, *ACSC* | 00, 05, 06 | Vector clock formalization |
| Grossman & Stiglitz 1980, *AER* | 00 | Information paradox |
| Kanerva 2009, *Cognitive Computation* | 10 | Hyperdimensional computing |
| Buchanan 1965, *Economica* | 08 | Club goods theory |
| Shannon 1948, *Bell System Technical Journal* | 00, 07, 10, 11 | Information theory |
| Pirolli & Card 1999, *Psychological Review* | 01 | Information foraging |
| Gibson 1979 | 01, 02 | Affordance theory |
| Heylighen 2016, *Cognitive Systems Research* | 01 | Universal coordination mechanism |
| Wilson 1971, *The Insect Societies* | 00, 04 | Social insect pheromone types |
| DeLanda 2006, *A New Philosophy of Society* | 07 | Assemblage theory |
| Ruan et al. 2025 | 11 | SwarmBench evaluation |
| Surowiecki 2004, *Wisdom of Crowds* | 10, 11 | Collective intelligence foundations |
| Wegner 1987, *Theories of Group Behavior* | 10 | Transactive memory |
| Schreiber 2000, *Physical Review Letters* 85(2) | 00 | Transfer entropy (causal information flow) |
| Langton 1990, *Physica D* 42(1-3) | 00 | Computation at edge of chaos |
| Tse & Viswanath 2005, *Fundamentals of Wireless Communication* | 03 | SINR framework for interference modeling |
| Bonabeau, Theraulaz & Deneubourg 1998, *Bull. Math. Biology* | 04 | Response threshold model for task allocation |
| Brewer 2012, *IEEE Computer* 45(2) | 06 | CAP theorem and AP design |
| Cross & Hohenberg 1993, *Reviews of Modern Physics* 65(3) | 07 | Pattern formation, bifurcation analysis |
| Bikhchandani, Hirshleifer & Welch 1992, *JPE* 100(5) | 11 | Information cascades and herding |
| Janis 1972, *Victims of Groupthink* | 11 | Groupthink pathology |
| Bénabou 2013, *Review of Economic Studies* 80(2) | 11 | Rational groupthink in organizations |
| Newman 2006, *PNAS* 103(23) | 11 | Network modularity and community detection |
| Sunstein 2017, *#Republic* | 11 | Echo chambers in information networks |

---

## Generation Notes

- **Sub-docs produced**: 13 (00–12) plus this INDEX.md
- **Total line count**: ~9,900 lines across all sub-docs (enhanced from ~3,540)
- **Key legacy sources consulted**:
  - `refactoring-prd/04-knowledge-and-mesh.md` (canonical pheromone structs, scope model)
  - `refactoring-prd/02-five-layers.md` (stigmergy section, layer taxonomy)
  - `refactoring-prd/09-innovations.md` (14 frontier innovations)
  - `refactoring-prd/05-agent-types.md` (niche construction, coding agent pheromone traces)
  - legacy source: `bardo-backup/tmp/agent-chain/03-stigmergy.md` (full stigmergy spec)
  - legacy source: `bardo-backup/tmp/agent-chain/09-exponential-flywheels.md` (10 flywheel mechanisms)
  - legacy source: `bardo-backup/tmp/agent-chain-new/02-coordination-theory.md` (7 scientific traditions)
  - legacy source: `bardo-backup/tmp/agent-chain/proving-collective-intelligence.md` (C-Factor, evidence levels)
  - legacy source: `bardo-backup/prd/02-mortality/10-clade-ecology.md` (pheromone field, decay function)
  - legacy source: `bardo-backup/prd/02-mortality/10b-morphogenetic-specialization.md` (Turing R-D, full Rust structs)
  - legacy source: `bardo-backup/prd/09-economy/04-coordination.md` (ERC coordination, pheromone signals)
  - legacy source: `bardo-backup/prd/13-runtime/06-collective-intelligence.md` (pheromone field operations)
  - legacy source: `bardo-backup/prd/20-styx/00-architecture.md` (16-service Agent Mesh architecture)
  - legacy source: `bardo-backup/prd/20-styx/03-clade-sync.md` (sync protocol, morphogenetic messages)
  - legacy source: `bardo-backup/prd/20-styx/07-p2p-transport.md` (Iroh integration, dual-transport model)
  - legacy source: `bardo-backup/tmp/mori-refactor/18-agent-ecology.md` (niche construction, affordances)
  - legacy source: `bardo-backup/tmp/death/tools/c05-multi-agent.md` (worktree model, base+overlay)
  - `tmp/implementation-plans/12b-chain-layer.md` (gossip mesh, ISFR)
- **Legacy naming map applied**: legacy terms were remapped to current names such as Roko, Agent, Neuro/NeuroStore, Agent Mesh, KORAI/DAEJI, Collective, and Engram.
- **Legacy reframe rules applied**: end-of-life framing and vitality phases were removed in favor of backup/restore, resource exhaustion, and `resource_pressure_scalar`.
- **Judgment calls**:
  - Chose domain-agnostic strategy dimensions for morphogenetic specialization (depth/breadth/
    execution/verification/time_horizon/exploration/exploitation/coordination) instead of
    legacy DeFi-specific dimensions
  - Generalized the legacy death-framed "bloodstain network" to "knowledge backup/restore" per reframe
    rules
  - Decided WebSocket before Iroh for implementation priority (simpler, faster to implement)
  - Used `Custom(String)` extensibility rather than registration for pheromone kinds (simpler)
  - Classified NicheVacancy messaging as agent "departure" rather than the legacy "death" framing
- **Unresolved tensions**:
  - The Pheromone struct exists only in PRD; implementing it requires deciding whether to
    extend the existing legacy code type `Signal` or create a separate type
  - The legacy Styx architecture has 16 services; Roko's Agent Mesh scope is unclear on how
    many of these survive the migration (this migration documents the coordination subset)
  - Cross-domain insight detection (HDC threshold 0.526) depends on the legacy code path `bardo-primitives`, which
    is built but not wired; the integration path is not fully specified
