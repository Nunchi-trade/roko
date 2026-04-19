# Subsystem: Coordination / Pheromones

Innovations that interact with the coordination system (roko-conductor) — reaction-diffusion specialisation, pheromone field, and multi-agent dynamics.

| Slug | Interaction |
|---|---|
| [knowledge-morphogenesis](../knowledge-morphogenesis.md) | Replaces task-return activation in `MorphogeneticState::update()` with knowledge concentration gradients; deposits concentration as Wisdom pheromone (24 h half-life, Mesh scope). |
| [stigmergic-bandits](../stigmergic-bandits.md) | Wraps `CascadeRouter` trail selection with `StigmergicBandit`; deposits Opportunity pheromone on success, anti-pheromone (double decay) on failure. |
