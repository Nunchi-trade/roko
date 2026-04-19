# Subsystem: Dreams

Innovations that interact with the Dreams subsystem (roko-dreams) — NREM replay, REM imagination, Delta consolidation, and dream scheduling.

| Slug | Interaction |
|---|---|
| [hdc-active-inference](../hdc-active-inference.md) | Delta/NREM consolidation slowly updates μ_prior (the prior belief vector) based on dream-verified knowledge. |
| [affect-causal-discovery](../affect-causal-discovery.md) | REM Phase 2 counterfactuals feed the causal inference engine; counterfactual PAD states are evaluated using the discovered causal model. |
| [dream-verification](../dream-verification.md) | Wraps the REM runner with `VerifiedDreamRunner`; generates and checks verification conditions for each counterfactual scenario before staging. |
| [knowledge-morphogenesis](../knowledge-morphogenesis.md) | Logs specialisation trajectories to `.roko/learn/specialisation.jsonl` for dream replay analysis. |
| [code-somatic-markers](../code-somatic-markers.md) | Dreams Integration phase merges nearby somatic markers created from gate verdicts. |
| [dream-token-economy](../dream-token-economy.md) | Gates dream cycle invocation via `can_afford()`; budget is updated after each cycle; phase allocation (NREM/REM/Integration) is driven by quality grade. |
| [witness-world-model](../witness-world-model.md) | Delta consolidation strengthens world model edges; REM adds novel causal edges to the model. |
