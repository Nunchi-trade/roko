# Witness DAG World Model

> Extract a causal world model directly from the Witness DAG by statistically analysing Prediction→Resolution chains, so the agent's history of cognitive events becomes its generative model for active inference.

**Kind**: Innovation
**Status**: Speculative
**Source domain**: Causal inference, active inference, epistemology
**Affects subsystems**: Witness DAG (roko-core), Heartbeat/gating, Learning (roko-learn), Dreams (Delta consolidation)
**Last reviewed**: 2026-04-19

---

## The idea

Roko's Witness DAG records cryptographic commitments of every cognitive event: Observations, Predictions, Decisions, Resolutions, and NeuroEntries. It is currently used for **forensic analysis** — post-facto auditing, hallucination detection, provenance queries.

The insight from active inference: an agent's **generative model** (its beliefs about how the world works) is precisely the structure that predicts future observations from past actions. The Witness DAG already *contains* this structure — the edges from Observations to Predictions to Decisions to Resolutions trace exactly the causal chains the agent believes in. The DAG doesn't just *record* history; it **is** the world model.

Extraction process:
1. Collect all Prediction vertices with their Resolution outcomes.
2. Group by prediction type (hash(prediction_category + observation_types)).
3. For each group: compute accuracy = correct_resolutions / total_predictions.
4. Create causal edge: Observation_type → Outcome_type with weight = accuracy × num_observations.
5. Prune edges with accuracy < 0.3 (likely spurious).
6. Detect confounders via conditional independence testing.
7. Compute Expected Free Energy for action selection: G(a) = −pragmatic_value − epistemic_value.

This complements Innovation 1 (HDC beliefs): use HDC for fast (~8 μs) per-tick inference, the Witness-derived model for deeper (~50 ms) periodic reflection. The Witness model is richer and graph-structured; HDC beliefs are compact (1,280 bytes) and real-time.

## Origin

- **Richens & Everitt (2024)** "Robust Agents Learn Causal World Models," ICLR 2024 (Oral), arXiv:2402.10877. Proves any agent achieving bounded regret under distributional shifts must have learned an approximate causal DAG. Behavioural history implicitly encodes a DAG converging to the true causal graph for optimal agents.
- **Gkountouras et al. (2024)** "Language Agents Meet Causality — Bridging LLMs and Causal World Models," arXiv:2410.19923. Causal world model as learned DAG providing LLMs a structured interface for interventional reasoning.
- **Deng et al. (2025)** "A Roadmap Towards Improving Multi-Agent RL with Causal Discovery and Inference," arXiv:2503.17803. Learning structural causal models from multi-agent interaction histories.
- **Conant & Ashby (1970)** "Every Good Regulator of a System Must Be a Model of That System." The Good Regulator Theorem: effective control requires an internal model isomorphic to the controlled system.

## Application to Roko

Seven integration steps are specified:

1. Add `CausalWorldModel` to `roko-core/src/world_model.rs` (adjacent to Witness DAG).
2. Extract model during Theta reflection (every 500 ticks).
3. Use EFE for action selection in heartbeat step 3 (ATTEND), replacing heuristic routing.
4. Calibration monitoring in `roko-learn/src/regression.rs` — staleness → `CognitiveSignal::Explore`.
5. Persist model to `.roko/state/world-model.json`.
6. Cross-agent model sharing via Mesh pheromone (Wisdom kind).
7. Dream replay enriches model: Delta consolidation strengthens edges, REM adds novel ones.

## Estimated impact

Source states: "Model reconstruction < 50ms for DAG with 10,000 vertices." "Calibration accuracy improves monotonically over 1,000 predictions (convergence)" (test criterion). "Confounder detection removes spurious correlation when true cause is identified."

## Prerequisites

- Witness DAG must contain sufficient Prediction→Resolution pairs (minimum 10 per prediction type).
- Statistical independence testing infrastructure.
- Witness DAG query interface to extract pairs efficiently.
- Periodic Theta reflection hook in the orchestration loop.

## Status

Speculative — idea only; no formal evaluation. Ranked **P2** in the source implementation priority table (large effort).

## Risks and objections

- Causal discovery from observational data is fundamentally limited without interventions — the Markov equivalence class cannot always be fully oriented.
- With 10 minimum observations per edge, early extraction may produce mostly unoriented or spurious edges.
- Model staleness detection (20-prediction window) may trigger excessive exploration during environment transitions.
- Cross-agent model sharing via pheromones could propagate incorrect causal beliefs across the collective.

## Related innovations

- [hdc-active-inference](./hdc-active-inference.md) — HDC beliefs are the fast-path complement; Witness model is the deep-reflection path
- [affect-causal-discovery](./affect-causal-discovery.md) — shared causal inference substrate; affect model and world model can share edges
- [dream-verification](./dream-verification.md) — Dream replay enriches both verification invariants and world model edges
- [stigmergic-bandits](./stigmergic-bandits.md) — EFE from world model could replace UCB for trail selection

## References

- Richens & Everitt (2024). Robust Agents Learn Causal World Models. ICLR 2024 (Oral), arXiv:2402.10877.
- Gkountouras et al. (2024). Language Agents Meet Causality. arXiv:2410.19923.
- Deng et al. (2025). A Roadmap Towards Improving Multi-Agent RL with Causal Discovery and Inference. arXiv:2503.17803.
- Conant & Ashby (1970). Every Good Regulator of a System Must Be a Model of That System. *International Journal of Systems Science* 1(2).
- Friston (2010). The free-energy principle: a unified brain theory? *Nature Reviews Neuroscience* 11(2).
