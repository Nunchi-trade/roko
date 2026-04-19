# Knowledge-Driven Morphogenesis

> Replace task-return-based activation in the Gierer-Meinhardt reaction-diffusion specialisation engine with knowledge concentration gradients from each agent's Neuro store, so agents specialise based on *what they know* rather than *what recently succeeded*.

**Kind**: Innovation
**Status**: Speculative
**Source domain**: Biology (morphogenesis), multi-agent systems, reaction-diffusion systems
**Affects subsystems**: Neuro, Coordination (roko-conductor), Pheromone system
**Last reviewed**: 2026-04-19

---

## The idea

Roko's coordination system already implements Turing reaction-diffusion for agent role specialisation (Gierer-Meinhardt dynamics, α=0.05, β=0.15, μ=0.01). The activation and inhibition signals are based on **task returns** — crude success/failure metrics.

Meanwhile, Neuro maintains rich knowledge with six types, four validation tiers, HDC vectors, and Ebbinghaus decay. This knowledge is **not connected** to the morphogenetic process. Agents specialise based on whether they succeed at tasks, not based on what they know.

The insight: **knowledge concentration should drive specialisation**. An agent with deep Consolidated knowledge about testing should specialise in testing. An agent with rich CausalLink knowledge about performance should specialise in optimisation. The knowledge landscape *is* the morphogenetic field.

Each agent's Neuro store is projected into the 8-dimensional strategy space, producing a "knowledge concentration gradient." Each strategy dimension maps to a tag set:

```
dim 0: refactoring/cleanup/restructure
dim 1: feature/implementation/api
dim 2: testing/assertions/coverage
dim 3: documentation/readme/comment
dim 4: performance/optimisation/benchmark
dim 5: security/auth/validation
dim 6: dependency/upgrade/version
dim 7: architecture/design/pattern
```

Concentration for dimension k = Σ(entry.confidence × tier_multiplier × jaccard(entry.tags, dim_tags[k])) normalised by Neuro store size. Tier multipliers: Transient 0.1 × , Working 0.5 ×, Consolidated 1.0 ×, Persistent 5.0 ×.

High concentration in a dimension drives activation; knowledge shared via pheromones (Wisdom kind, 24 h half-life) drives inhibition — if another agent already has deep knowledge in a niche, don't compete.

Key differences from task-return activation:
- **Proactive** (specialise on what you know) vs. reactive (specialise on what worked)
- **Stable** (decay via Ebbinghaus) vs. volatile (single failure can flip strategy)

## Origin

- **Richardson et al. (2024)** "Learning Spatio-Temporal Patterns with Neural Cellular Automata," *PLOS Computational Biology*. Trains NCAs to learn Turing pattern dynamics — local update rules produce global emergent patterns.
- **Shimizu et al. (2025)** "An Algorithm Applying the Self-Organizing Capabilities of a Reaction-Diffusion Model to Control Active Swarm Robots," *Journal of Intelligent & Robotic Systems*. Reaction-diffusion controlling self-organising modular robots with local-only information.
- **Turing (1952)** "The Chemical Basis of Morphogenesis," *Philosophical Transactions of the Royal Society B*. Foundational: spatial pattern formation via local activation + long-range inhibition.

## Application to Roko

Six integration steps are specified:

1. Add `KnowledgeConcentration` computation to `roko-conductor/src/morphogenesis.rs`.
2. Wire NeuroStore query into concentration engine (read-only cross-crate access).
3. Replace task-return activation with gradient: α × gradient replaces α × returns.
4. Deposit concentration as Wisdom pheromone (`PheromoneKind::Wisdom`, 24 h half-life, Mesh scope).
5. Dashboard strategy heatmap per agent in `roko-cli/src/tui/`.
6. Log specialisation trajectory to `.roko/learn/specialisation.jsonl`.

## Estimated impact

Source states no explicit throughput number. Test criteria: "Two agents with identical knowledge differentiate via inhibition within 100 ticks." "Agent with deep testing knowledge specialises in testing dimension (>3× baseline)."

## Prerequisites

- NeuroStore with tag-indexed entries accessible from `roko-conductor`.
- Pheromone infrastructure with Wisdom kind and configurable half-life.
- 8-dimensional strategy vector per agent (already in morphogenetic state).

## Status

Speculative — idea only; no formal evaluation. Ranked **P2** in the source implementation priority table.

## Risks and objections

- Jaccard similarity between free-form tags and predefined dimension tag sets is noisy; tag quality will determine gradient quality.
- Knowledge concentration is computed over the full Neuro store — expensive if the store is large; needs caching or incremental updates.
- Inhibition via collective pheromones requires the pheromone mesh to be timely; stale pheromones could cause erroneous inhibition.

## Related innovations

- [stigmergic-bandits](./stigmergic-bandits.md) — specialised agents deposit stronger pheromones in their niche, reinforcing the gradient
- [hdc-active-inference](./hdc-active-inference.md) — HDC fingerprints are used to tag knowledge entries that feed concentration
- [dream-token-economy](./dream-token-economy.md) — higher-tier knowledge (earning more dream budget) also dominates concentration gradients
- [witness-world-model](./witness-world-model.md) — causal edges added to world model also contribute to knowledge concentration

## References

- Richardson et al. (2024). Learning Spatio-Temporal Patterns with Neural Cellular Automata. *PLOS Computational Biology*.
- Shimizu et al. (2025). An Algorithm Applying the Self-Organizing Capabilities of a Reaction-Diffusion Model to Control Active Swarm Robots. *Journal of Intelligent & Robotic Systems*.
- Turing (1952). The Chemical Basis of Morphogenesis. *Philosophical Transactions of the Royal Society B* 237(641).
- Gierer & Meinhardt (1972). A theory of biological pattern formation. *Kybernetik* 12(1).
