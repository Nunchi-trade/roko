# Stigmergic Bandits

> Treat each pheromone trail as a bandit arm so that new agents inherit the collective's exploration history instead of cold-starting, and collective reward signals replace per-agent trial-and-error.

**Kind**: Innovation
**Status**: Speculative
**Source domain**: Swarm intelligence, reinforcement learning, multi-armed bandits
**Affects subsystems**: Learning (roko-learn), Pheromone/coordination (roko-conductor), CascadeRouter
**Last reviewed**: 2026-04-19

---

## The idea

Roko has two parallel selection mechanisms that solve explore/exploit using completely different paradigms:

- **Bandits** (UCB1, LinUCB, Thompson Sampling): Per-agent, internal. Each agent maintains its own reward estimates and exploration bonuses. No inter-agent learning. Cold starts everywhere.
- **Pheromones** (stigmergy): Inter-agent, external. Agents deposit and sense environmental traces. Rich spatial structure. But no formal optimality guarantees — decay rates are hand-tuned, not learned.

The synthesis: treat each pheromone trail as a **bandit arm**. The arm's estimated reward is the trail's current intensity (decayed). The exploration bonus is a UCB-style term based on how recently the trail was last reinforced. New agents inherit the collective's exploration history via the pheromone field instead of cold-starting.

The Pheromone-Bandit correspondence:

| Bandit Concept | Pheromone Analog |
|---|---|
| Arm | Pheromone trail (kind + scope + location) |
| Estimated reward | Trail intensity × depositor reputation |
| Pull count | Number of reinforcements |
| Exploration bonus | C × √(ln(total_pulls) / trail_pulls) |
| Reward update | Deposit (success) or anti-deposit (failure) |
| Arm creation | New trail deposited by any agent |
| Arm elimination | Trail intensity decays below threshold |

UCB score blends local experience (70%) with pheromone intensity (30%), plus a UCB1 exploration bonus. Pure exploration (ε-greedy, ε=0.05) creates new trails. Anti-pheromone on failure doubles the decay rate.

## Origin

- **Chari et al. (2025)** "Pheromone-based Learning of Optimal Reasoning Paths" (ACO-ToT), arXiv:2501.19278. LLM "ants" deposit pheromone on reasoning paths; mixture-of-experts scoring combines pheromone with expertise. Outperforms standard CoT on GSM8K, ARC, MATH.
- **Li, Zhu et al. (2024)** "PooL: Pheromone-inspired Communication Framework for Large-Scale MARL," arXiv:2202.09722. Pheromones as RL agent outputs, achieving higher rewards with lower communication cost.
- **Dorigo & Stützle (2004)** *Ant Colony Optimization*. Foundational framework with convergence guarantees under ergodicity assumptions.
- **Auer, Cesa-Bianchi & Fischer (2002)** "Finite-time Analysis of the Multiarmed Bandit Problem," *Machine Learning* 47(2-3). UCB1 achieves O(√(KT ln T)) regret — optimal up to logarithmic factors.

## Application to Roko

Six integration steps are specified:

1. Add `StigmergicBandit` to `roko-learn/src/stigmergic.rs` (adjacent to existing bandits module).
2. Wrap `CascadeRouter` trail selection with `StigmergicBandit` (models as trails + arms).
3. Cold-start new agents from pheromone field (inherit field rewards at 50% discount).
4. Deposit pheromone on task completion in `roko-cli/src/orchestrate.rs` after gate verdict.
5. Anti-pheromone on gate failure (double decay rate).
6. Dashboard trail intensity heatmap in `roko-cli/src/tui/`.

## Estimated impact

Source states: "Stigmergic bandit achieves lower cumulative regret than isolated UCB1 (≥20% improvement)" (test criterion). "New agent inherits reward estimates from 10 existing trails within 1 tick."

## Prerequisites

- Existing pheromone infrastructure with active trails, deposit/decay, and intensity queries.
- Existing bandits module in `roko-learn` for local reward tracking.
- `CascadeRouter` refactored to accept pluggable trail selection.

## Status

Speculative — idea only; no formal evaluation. Ranked **P1** in the source implementation priority table.

## Risks and objections

- The blending of local (70%) and pheromone (30%) reward is a fixed heuristic; optimal blend ratio is task- and domain-dependent.
- Anti-pheromone (double decay) is an aggressive penalty; a single false failure could prematurely suppress a good trail.
- Trail intensity can be manipulated by agents that deposit pheromone on low-reward trails (free-riding / adversarial agents).
- UCB1 regret bounds require stationary reward distributions; pheromone rewards are non-stationary by design.

## Related innovations

- [knowledge-morphogenesis](./knowledge-morphogenesis.md) — specialised agents deposit stronger pheromones, reinforcing specialisation gradients
- [hdc-active-inference](./hdc-active-inference.md) — free energy can gate when to follow trails vs. explore
- [witness-world-model](./witness-world-model.md) — EFE action selection could replace UCB for trail selection

## References

- Chari et al. (2025). Pheromone-based Learning of Optimal Reasoning Paths. arXiv:2501.19278.
- Li, Zhu et al. (2024). PooL: Pheromone-inspired Communication Framework for Large-Scale MARL. arXiv:2202.09722.
- Dorigo & Stützle (2004). *Ant Colony Optimization*. MIT Press.
- Auer, Cesa-Bianchi & Fischer (2002). Finite-time Analysis of the Multiarmed Bandit Problem. *Machine Learning* 47(2-3).
