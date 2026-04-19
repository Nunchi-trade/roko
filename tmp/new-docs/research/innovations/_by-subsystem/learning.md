# Subsystem: Learning (roko-learn)

Innovations that interact with the learning subsystem — bandits, cost tracking, episode history, and skill evolution.

| Slug | Interaction |
|---|---|
| [affect-causal-discovery](../affect-causal-discovery.md) | Adds PAD snapshots to `Episode` records; runs PC algorithm structure learning during Theta reflection. |
| [stigmergic-bandits](../stigmergic-bandits.md) | Adds `StigmergicBandit` to `roko-learn`; wraps existing UCB1/LinUCB/Thompson Sampling with pheromone-backed trail selection. |
| [witness-world-model](../witness-world-model.md) | `CausalWorldModel` calibration monitoring in `roko-learn/src/regression.rs`; staleness detection triggers `CognitiveSignal::Explore`. |
| [dream-token-economy](../dream-token-economy.md) | Adds `DreamBudget` with `dream_economy.rs`; tracks dream cycle costs and knowledge promotion revenue. |
