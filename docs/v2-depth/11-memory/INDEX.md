# 11-memory -- Depth Index

Depth documents for [11-MEMORY-AND-KNOWLEDGE.md](../../unified/11-MEMORY-AND-KNOWLEDGE.md).
Covers three source domains: the **Neuro** knowledge store (types, tiers, HDC, distillation, backup),
**Dreams** offline consolidation (cycle, replay, hypnagogia, threat simulation, staging), and
**Coordination** stigmergy (pheromones, mesh sync, collective intelligence). Each depth doc
redesigns its source material using unified primitives (Signal, Cell, Graph, Loop, Bus, Store).

---

## Depth docs (15)

| # | Filename | Covers |
|---|---|---|
| 01 | [knowledge-as-signal.md](01-knowledge-as-signal.md) | Knowledge types as Signal Kinds, tiers as demurrage, Ebbinghaus decay as rate law, query API as Store protocol |
| 02 | [hdc-algebra-and-retrieval.md](02-hdc-algebra-and-retrieval.md) | HDC 10,240-bit vectors, four operations (bind/bundle/permute/similarity), retrieval as Pipeline Graph, false positive math, anti-knowledge HDC encoding |
| 03 | [knowledge-lifecycle-loop.md](03-knowledge-lifecycle-loop.md) | D1/D2/D3 distillation as Loop Graph, calibration as predict-publish-correct, backup/restore as Store replay, somatic integration, mesh sync as Pipeline inflow |
| 04 | [antiknowledge-and-immunity.md](04-antiknowledge-and-immunity.md) | AntiKnowledge as Signal Kind, cognitive immune system as Verify Pipeline, SIR epidemiological tracking as Observe Cell, memetic fitness as Score Cell |
| 05 | [cross-domain-transfer.md](05-cross-domain-transfer.md) | Cross-domain HDC resonance as Store query, Library of Babel as nested Spaces, federation as Pipeline ingestion, mesh sync as Connect protocol |
| 06 | [dream-cycle-as-loop.md](06-dream-cycle-as-loop.md) | Three-phase dream cycle (NREM/REM/Integration) as Loop Graph, Trigger-based scheduling, phase budget allocation, sleep-time compute, cross-system integration |
| 07 | [replay-and-counterfactual-cells.md](07-replay-and-counterfactual-cells.md) | NREM replay as Score Cell (Mattar-Daw utility), REM imagination as Compose Cell, HDC counterfactual synthesis, hindsight experience replay, inner world rendering |
| 08 | [hypnagogia-and-creativity.md](08-hypnagogia-and-creativity.md) | Hypnagogia engine as Pipeline Graph, anti-correlated retrieval, divergence/alpha control, Dali interrupt as React Cell, homuncular observer as Verify Cell |
| 09 | [consolidation-and-staging.md](09-consolidation-and-staging.md) | Staging buffer as Store partition, confidence ladder as demurrage resistance, SHY renormalization as Functor, confirmation boost as React Cell, dream evolution, oneirography |
| 10 | [threat-simulation-and-nightmares.md](10-threat-simulation-and-nightmares.md) | Threat simulation as Verify Cell, FMEA/FTA as Score Cells, nightmare detection pipeline, nightmare containment as React Cell, dream journal analysis |
| 11 | [stigmergy-as-bus.md](11-stigmergy-as-bus.md) | Pheromones as Pulses, evaporation as ring-buffer/demurrage, reinforcement as repeated publication, scoped visibility via Bus topics |
| 12 | [12-pheromone-mechanics-and-interference.md](12-pheromone-mechanics-and-interference.md) | 7 universal pheromone kinds, half-life taxonomy, SINR interference model, anti-saturation mechanisms, Hill-function response thresholds, promotion cascade (Pattern→Wisdom→Consensus) |
| 13 | [13-morphogenetic-specialization-as-loop.md](13-morphogenetic-specialization-as-loop.md) | Turing reaction-diffusion for role emergence, 8D strategy vector, Gierer-Meinhardt kinetics, niche competition, stability analysis (Lyapunov), convergence O(N×log N) |
| 14 | [14-mesh-sync-and-subnets.md](14-mesh-sync-and-subnets.md) | Bus federation across Space boundaries, dual transport (WebSocket relay + Iroh gossip), partition tolerance, permissioned subnets as nested Spaces, trust multipliers, publish gates |
| 15 | [15-collective-metrics-as-lens.md](15-collective-metrics-as-lens.md) | c-factor as Lens Graph (5 axes), WisdomGate, groupthink countermeasures (Alpha paradox, contrarian retrieval), 7 compounding flywheel Loops, scaling analysis |

---

## Source mapping

### Neuro / knowledge store (16 source docs)

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/06-neuro/00-vision-and-grimoire-rename.md` | Absorbed | [01-knowledge-as-signal.md](01-knowledge-as-signal.md) |
| `docs/06-neuro/01-six-knowledge-types.md` | Absorbed | [01-knowledge-as-signal.md](01-knowledge-as-signal.md) |
| `docs/06-neuro/02-four-validation-tiers.md` | Absorbed | [01-knowledge-as-signal.md](01-knowledge-as-signal.md) |
| `docs/06-neuro/03-type-half-lives.md` | Absorbed | [01-knowledge-as-signal.md](01-knowledge-as-signal.md) |
| `docs/06-neuro/04-hdc-vsa-foundations.md` | Absorbed | [02-hdc-algebra-and-retrieval.md](02-hdc-algebra-and-retrieval.md) |
| `docs/06-neuro/05-hdc-operations.md` | Absorbed | [02-hdc-algebra-and-retrieval.md](02-hdc-algebra-and-retrieval.md) |
| `docs/06-neuro/06-hdc-knowledge-encoding.md` | Absorbed | [02-hdc-algebra-and-retrieval.md](02-hdc-algebra-and-retrieval.md) |
| `docs/06-neuro/07-ebbinghaus-decay-with-tier.md` | Absorbed | [01-knowledge-as-signal.md](01-knowledge-as-signal.md) |
| `docs/06-neuro/08-cross-domain-hdc-transfer.md` | Absorbed | [02-hdc-algebra-and-retrieval.md](02-hdc-algebra-and-retrieval.md), [05-cross-domain-transfer.md](05-cross-domain-transfer.md) |
| `docs/06-neuro/09-false-positive-math.md` | Absorbed | [02-hdc-algebra-and-retrieval.md](02-hdc-algebra-and-retrieval.md) |
| `docs/06-neuro/10-knowledge-query-api.md` | Absorbed | [01-knowledge-as-signal.md](01-knowledge-as-signal.md), [02-hdc-algebra-and-retrieval.md](02-hdc-algebra-and-retrieval.md) |
| `docs/06-neuro/11-antiknowledge-challenge.md` | Absorbed | [02-hdc-algebra-and-retrieval.md](02-hdc-algebra-and-retrieval.md), [04-antiknowledge-and-immunity.md](04-antiknowledge-and-immunity.md) |
| `docs/06-neuro/12-4-tier-distillation-pipeline.md` | Absorbed | [03-knowledge-lifecycle-loop.md](03-knowledge-lifecycle-loop.md) |
| `docs/06-neuro/13-somatic-integration.md` | Absorbed | [03-knowledge-lifecycle-loop.md](03-knowledge-lifecycle-loop.md) |
| `docs/06-neuro/14-library-of-babel.md` | Absorbed | [05-cross-domain-transfer.md](05-cross-domain-transfer.md) |
| `docs/06-neuro/15-knowledge-backup-restore.md` | Absorbed | [03-knowledge-lifecycle-loop.md](03-knowledge-lifecycle-loop.md) |

### Dreams / offline consolidation (16 source docs)

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/10-dreams/00-vision-and-dream-as-death-reframe.md` | Absorbed | [06-dream-cycle-as-loop.md](06-dream-cycle-as-loop.md) |
| `docs/10-dreams/01-three-phase-cycle.md` | Absorbed | [06-dream-cycle-as-loop.md](06-dream-cycle-as-loop.md) |
| `docs/10-dreams/02-nrem-replay.md` | Absorbed | [06-dream-cycle-as-loop.md](06-dream-cycle-as-loop.md), [07-replay-and-counterfactual-cells.md](07-replay-and-counterfactual-cells.md) |
| `docs/10-dreams/03-rem-imagination.md` | Absorbed | [06-dream-cycle-as-loop.md](06-dream-cycle-as-loop.md), [07-replay-and-counterfactual-cells.md](07-replay-and-counterfactual-cells.md) |
| `docs/10-dreams/04-consolidation-and-staging.md` | Absorbed | [06-dream-cycle-as-loop.md](06-dream-cycle-as-loop.md), [07-replay-and-counterfactual-cells.md](07-replay-and-counterfactual-cells.md), [09-consolidation-and-staging.md](09-consolidation-and-staging.md) |
| `docs/10-dreams/05-dream-evolution.md` | Absorbed | [06-dream-cycle-as-loop.md](06-dream-cycle-as-loop.md), [09-consolidation-and-staging.md](09-consolidation-and-staging.md) |
| `docs/10-dreams/06-hdc-counterfactual-synthesis.md` | Absorbed | [06-dream-cycle-as-loop.md](06-dream-cycle-as-loop.md), [07-replay-and-counterfactual-cells.md](07-replay-and-counterfactual-cells.md) |
| `docs/10-dreams/07-hypnagogia-engine.md` | Absorbed | [06-dream-cycle-as-loop.md](06-dream-cycle-as-loop.md), [08-hypnagogia-and-creativity.md](08-hypnagogia-and-creativity.md) |
| `docs/10-dreams/08-divergence-and-alpha.md` | Absorbed | [08-hypnagogia-and-creativity.md](08-hypnagogia-and-creativity.md) |
| `docs/10-dreams/09-threat-simulation.md` | Absorbed | [06-dream-cycle-as-loop.md](06-dream-cycle-as-loop.md), [07-replay-and-counterfactual-cells.md](07-replay-and-counterfactual-cells.md), [10-threat-simulation-and-nightmares.md](10-threat-simulation-and-nightmares.md) |
| `docs/10-dreams/10-hauntology-in-dreams.md` | Absorbed | [06-dream-cycle-as-loop.md](06-dream-cycle-as-loop.md) |
| `docs/10-dreams/11-inner-worlds-and-rendering.md` | Absorbed | [07-replay-and-counterfactual-cells.md](07-replay-and-counterfactual-cells.md), [08-hypnagogia-and-creativity.md](08-hypnagogia-and-creativity.md) |
| `docs/10-dreams/12-sleep-time-compute.md` | Absorbed | [06-dream-cycle-as-loop.md](06-dream-cycle-as-loop.md) |
| `docs/10-dreams/13-scheduling-and-triggers.md` | Absorbed | [06-dream-cycle-as-loop.md](06-dream-cycle-as-loop.md) |
| `docs/10-dreams/14-oneirography.md` | Absorbed | [09-consolidation-and-staging.md](09-consolidation-and-staging.md) |
| `docs/10-dreams/15-cross-system-integration.md` | Absorbed | [06-dream-cycle-as-loop.md](06-dream-cycle-as-loop.md) |

### Coordination / stigmergy (12 source docs)

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/13-coordination/00-stigmergy-theory.md` | Absorbed | [11-stigmergy-as-bus.md](11-stigmergy-as-bus.md) |
| `docs/13-coordination/01-stigmergy-beyond-termites.md` | Absorbed | [11-stigmergy-as-bus.md](11-stigmergy-as-bus.md) |
| `docs/13-coordination/02-git-as-stigmergy.md` | Absorbed | [11-stigmergy-as-bus.md](11-stigmergy-as-bus.md) |
| `docs/13-coordination/03-digital-pheromones.md` | Absorbed | [11-stigmergy-as-bus.md](11-stigmergy-as-bus.md), [12-pheromone-mechanics-and-interference.md](12-pheromone-mechanics-and-interference.md) |
| `docs/13-coordination/04-pheromone-kinds.md` | Absorbed | [12-pheromone-mechanics-and-interference.md](12-pheromone-mechanics-and-interference.md) |
| `docs/13-coordination/05-pheromone-scope.md` | Absorbed | [14-mesh-sync-and-subnets.md](14-mesh-sync-and-subnets.md) |
| `docs/13-coordination/06-agent-mesh-sync.md` | Absorbed | [14-mesh-sync-and-subnets.md](14-mesh-sync-and-subnets.md) |
| `docs/13-coordination/07-morphogenetic-specialization.md` | Absorbed | [13-morphogenetic-specialization-as-loop.md](13-morphogenetic-specialization-as-loop.md) |
| `docs/13-coordination/08-permissioned-subnets.md` | Absorbed | [14-mesh-sync-and-subnets.md](14-mesh-sync-and-subnets.md) |
| `docs/13-coordination/09-stigmergy-scaling.md` | Absorbed | [15-collective-metrics-as-lens.md](15-collective-metrics-as-lens.md) |
| `docs/13-coordination/10-exponential-flywheel.md` | Absorbed | [15-collective-metrics-as-lens.md](15-collective-metrics-as-lens.md) |
| `docs/13-coordination/11-collective-intelligence-metrics.md` | Absorbed | [15-collective-metrics-as-lens.md](15-collective-metrics-as-lens.md) |

---

44 source docs -- all absorbed across 15 depth docs.
