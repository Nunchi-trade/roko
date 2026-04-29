# 21-roadmap — Depth Index

Depth for [21-ROADMAP.md](../../unified/21-ROADMAP.md)

---

## Source docs (44)

### Technical analysis (15)

| Source doc | Status |
|---|---|
| `docs/20-technical-analysis/00-vision-ta-generalized.md` | Absorbed (03) |
| `docs/20-technical-analysis/01-oracle-trait.md` | Absorbed (03) |
| `docs/20-technical-analysis/02-chain-oracles.md` | Absorbed (03) |
| `docs/20-technical-analysis/03-coding-oracles.md` | Absorbed (03) |
| `docs/20-technical-analysis/04-research-oracles.md` | Absorbed (03) |
| `docs/20-technical-analysis/05-witness-as-ta-generalized.md` | Absorbed (03, 04) |
| `docs/20-technical-analysis/06-hyperdimensional-ta.md` | Absorbed (04) |
| `docs/20-technical-analysis/07-spectral-liquidity-manifolds.md` | Absorbed (06) |
| `docs/20-technical-analysis/08-adaptive-signal-metabolism.md` | Absorbed (04) |
| `docs/20-technical-analysis/09-causal-microstructure-discovery.md` | Absorbed (05) |
| `docs/20-technical-analysis/10-predictive-geometry-and-resonant-patterns.md` | Absorbed (04, 06) |
| `docs/20-technical-analysis/11-adversarial-signal-robustness.md` | Absorbed (05) |
| `docs/20-technical-analysis/12-somatic-ta-and-emergent-multiscale.md` | Absorbed (06) |
| `docs/20-technical-analysis/13-predictive-foraging-and-active-inference.md` | Absorbed (03) |
| `docs/20-technical-analysis/14-sheaf-tropical-geometry.md` | Absorbed (06) |

### Academic references (25)

| Source doc | Status |
|---|---|
| `docs/21-references/00-lifecycle-and-finite-agency.md` | Absorbed (07, 08) |
| `docs/21-references/01-memory-consolidation.md` | Absorbed (07, 08) |
| `docs/21-references/02-affective-computing.md` | Absorbed (07) |
| `docs/21-references/03-dreams-and-offline-learning.md` | Absorbed (07, 08) |
| `docs/21-references/04-coordination-and-multi-agent.md` | Absorbed (07) |
| `docs/21-references/05-biological-analogues.md` | Absorbed (07, 08) |
| `docs/21-references/06-self-learning-systems.md` | Absorbed (07, 08) |
| `docs/21-references/07-context-engineering.md` | Absorbed (07) |
| `docs/21-references/08-security-and-provenance.md` | Absorbed (07) |
| `docs/21-references/09-hdc-vsa.md` | Absorbed (07) |
| `docs/21-references/10-market-microstructure.md` | Absorbed (07) |
| `docs/21-references/11-streaming-algorithms.md` | Absorbed (07) |
| `docs/21-references/12-signal-processing.md` | Absorbed (07) |
| `docs/21-references/13-philosophy.md` | Absorbed (07) |
| `docs/21-references/14-agent-harnesses-and-tool-use.md` | Absorbed (07, 08) |
| `docs/21-references/15-cybernetics-and-vsm.md` | Absorbed (07) |
| `docs/21-references/16-active-inference.md` | Absorbed (07, 08) |
| `docs/21-references/17-process-reward-models.md` | Absorbed (07, 09) |
| `docs/21-references/18-collective-intelligence.md` | Absorbed (07) |
| `docs/21-references/19-regulatory-compliance.md` | Absorbed (07) |
| `docs/21-references/20-cognitive-architectures.md` | Absorbed (07) |
| `docs/21-references/21-mechanism-design.md` | Absorbed (07) |
| `docs/21-references/22-protocol-standards.md` | Absorbed (07) |
| `docs/21-references/23-generational-and-evolutionary.md` | Absorbed (07, 08) |
| `docs/21-references/24-additions-2025.md` | Absorbed (07) |
| `docs/21-references/25-research-to-runtime.md` | Absorbed (08) |

### Architecture cross-cuts (5)

| Source doc | Status |
|---|---|
| `docs/00-architecture/27-temporal-knowledge-topology.md` | Done |
| `docs/00-architecture/28-emergent-goal-structures.md` | Done |
| `docs/00-architecture/29-cognitive-energy-model.md` | Done |
| `docs/00-architecture/32-comprehensive-test-strategy.md` | Absorbed (09) |
| `docs/00-architecture/33-refactor-plan-phases.md` | Pending |

---

## Depth docs

| Depth doc | Source | What it adds |
|---|---|---|
| [temporal-knowledge-graph.md](temporal-knowledge-graph.md) | `27-temporal-knowledge-topology.md` | Allen's 13 interval relations as constraint network in Store, event calculus (HoldsAt/Initiates/Terminates) as Cells, 3-tier temporal Memory (Episode/Entity/Community), HDC-guided tier progression, temporal demurrage modulation |
| [emergent-goals-and-energy.md](emergent-goals-and-energy.md) | `28-emergent-goal-structures.md`, `29-cognitive-energy-model.md` | Unified goal-energy model, goal emergence Cell, intrinsic motivation Score Cell with ZPD, five energy zones as type-state, EFE goal selection Route Cell, goal conflict arbitration, energy-affect bidirectional Loop, somatic markers |
| [03-oracle-as-score-cell.md](03-oracle-as-score-cell.md) | `01-oracle-trait.md`, `02-chain-oracles.md`, `03-coding-oracles.md`, `04-research-oracles.md`, `13-predictive-foraging.md` | Oracle as Score Cell with predict-publish-correct, ResidualCorrector as Functor (~50ns bias elimination), CalibrationTracker as Store Cell, three domain oracle specializations (chain/coding/research), conformal prediction as Verify Cell, oracle composition with VCG auction, complete Oracle Loop Graph |
| [04-hdc-pattern-encoding-and-metabolism.md](04-hdc-pattern-encoding-and-metabolism.md) | `06-hyperdimensional-ta.md`, `08-adaptive-signal-metabolism.md`, `10-predictive-geometry.md` | Patterns as Signals in Store (Kind::Pattern), role-filler BIND + temporal PERMUTE + composite BUNDLE, quantized codebooks with thermometer construction, cross-domain resonance at 0.526 threshold, replicator equation as demurrage economics, Oja's rule Hebbian learning, Fisher's fundamental theorem as Loop observable, Red Queen pressure via demurrage, Dream consolidation (NREM replay + REM recombination + pruning) |
| [05-causal-discovery-and-adversarial-robustness.md](05-causal-discovery-and-adversarial-robustness.md) | `09-causal-microstructure-discovery.md`, `11-adversarial-signal-robustness.md` | Pearl's 3-level causal hierarchy as Cell taxonomy (L1=Store query, L2=Connect Cell, L3=Dream REM), SCM as Graph of Score Cells, PC algorithm as Score Cell, Granger causality with 4 DeFi extensions, intervention testing via Connect Cell (mirage-rs), counterfactual reasoning in Dream cycle, 5-layer immune system Pipeline, HDC prototype matching (~10ns), robust statistics (trimmed mean/MAD), red-team dreaming self-immunization, certified robustness via randomized smoothing |
| [06-advanced-geometry-and-integration.md](06-advanced-geometry-and-integration.md) | `07-spectral-liquidity-manifolds.md`, `10-predictive-geometry.md`, `12-somatic-ta.md`, `14-sheaf-tropical-geometry.md` | Research-stage concepts as target-state Cells with graduation paths. Spectral manifolds as Route Cell (naive=lookup table, target=Riemannian geodesic). TDA as Signal metadata (persistence diagrams, Takens embedding, landscapes). Sheaf consistency as Verify Cell (naive=pairwise checks, target=Laplacian spectral analysis). Tropical geometry as Route Cell (naive=decision tree, target=tropical polynomial boundaries). IIT Phi as Observe Cell. PID synergy detection. |
| [07-academic-foundations-by-protocol.md](07-academic-foundations-by-protocol.md) | All 25 `docs/21-references/` files | 500+ citations organized by which unified protocol they ground: Store (memory consolidation, streaming), Score (PRMs, calibration, collective scoring), Verify (security, formal methods, provenance, compliance), Route (active inference, dual-process, market microstructure), Compose (context engineering, mechanism design, attention), React (cybernetics, self-learning, biological adaptation), Agent specialization (cognitive architectures, affective computing, philosophy), Coordination (stigmergy, collective intelligence, evolutionary), HDC/Signal (BSC algebra, random projection, hashing), External protocols (MCP, A2A, ERC-8004, x402) |
| [08-research-to-runtime-bridge.md](08-research-to-runtime-bridge.md) | `docs/21-references/25-research-to-runtime.md`, all reference files | Five detailed bridges: active inference -> 4-signal EFE approximation; replicator dynamics -> demurrage + retrieve-to-reinforce; Turing patterns -> morphogenetic pheromone field; somatic markers -> PAD x decision k-d tree; conformal prediction -> CalibrationTracker EMA. Each bridge: academic foundation, runtime pseudocode, fidelity-lost table, higher-fidelity implementation path, graduation criterion. Research-to-Runtime Pipeline as a Loop Graph (Paper -> Hypothesis -> Implementation -> Evaluation -> Calibration). |
| [09-test-strategy-and-verification.md](09-test-strategy-and-verification.md) | `docs/00-architecture/32-comprehensive-test-strategy.md` | Tests as Verify Cells at multiple scales. Test pyramid mapped to tier costs: unit (T0, free), integration (T0, compile-time), property (T1, cheap), eval (T2, expensive), red-team (Delta, offline). CI/CD as Pipeline Graph fired by Trigger Cells. Three property-test categories (algebraic, stateful, metamorphic). Self-immunization via Dream-cycle adversarial probing. Capability preservation tests for evolutionary stability. Quality metrics (3,761 tests, target 5,000). Observability contract verification. |
