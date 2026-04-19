# HDC + Active Inference

> Encode Roko's generative world model as a 10,240-bit HDC vector so that belief updating becomes a vector-bundle operation and prediction error becomes Hamming distance.

**Kind**: Innovation
**Status**: Speculative
**Source domain**: Neuroscience, information theory, hyperdimensional computing
**Affects subsystems**: Neuro, Heartbeat/gating, Daimon, Dreams (NREM consolidation)
**Last reviewed**: 2026-04-19

---

## The idea

Roko's Neuro subsystem already encodes knowledge as 10,240-bit Binary Spatter Code (BSC) vectors (XOR binding, majority-vote bundling, cyclic permutation). The heartbeat's dual-process gating approximates active inference's Expected Free Energy to route between T0/T1/T2 tiers. But prediction error is currently a scalar derived from probe anomaly counts and regime drift.

The gap: the agent has no *structured belief representation* that can be updated, compared, or composed. Active inference requires a generative model — a probability distribution over world states the agent updates via sensory prediction errors. HDC vectors are that model.

Encoding the agent's generative model as an HDC vector unifies these two subsystems. Each belief about the world is a role-filler binding in a 10,240-bit BSC vector. Prediction error becomes **Hamming distance** between the predicted observation vector and the actual observation vector. Free energy minimisation becomes **vector update** operations — no matrix algebra, O(160) word operations.

```
Generative model μ: HDC vector encoding current beliefs
Predicted observation ô: decode(μ) via unbinding
Actual observation o: encode current sensory state
Prediction error ε: hamming_distance(ô, o) / 10240
Free energy F ≈ ε + complexity_penalty(μ)
Update: μ' = bundle([μ, weighted_bind(o, learning_rate)])
```

The elegance: free energy is a **scalar derived from Hamming distance**, which Roko already computes in ~50 ns via POPCNT. No new mathematical machinery is needed. The belief vector μ is the agent's world model compressed to 1,280 bytes — persistable, transmissible, and algebraically composable via bundling.

## Origin

Draws on four primary sources:

- **Bybee & Bhatt (2024)** "Modelling Neural Probabilistic Computation Using Vector Symbolic Architectures," *Frontiers in Computational Neuroscience* 18. Shows that VSA operations natively compute marginalisation, entropy, and mutual information; belief updating reduces to vector addition.
- **Heddes et al. (2024)** "Hyperdimensional Computing: A Framework for Stochastic Computation and Symbolic AI," *Journal of Big Data*. Frames HDC as stochastic computing where noise tolerance aligns with approximate Bayesian inference.
- **Renner et al. (2024)** "Brain-Inspired Computational Intelligence via Predictive Coding," arXiv:2308.07870v3. Formalises predictive coding as a general-purpose learning algorithm for distributed architectures.
- **Friston (2010)** "The free-energy principle: a unified brain theory?" *Nature Reviews Neuroscience* 11(2). Foundational formulation: F = E_q[ln q(s) − ln p(o,s)].

## Application to Roko

Seven integration steps are specified:

1. Add `HdcBeliefState` to `bardo-primitives/src/belief.rs` (HDC vector ops already present).
2. Initialise belief from Daimon personality — PAD baseline → μ_prior.
3. Wire into heartbeat perception step, replacing scalar prediction error with HDC prediction error.
4. Feed observation vectors from the 16 T0 probes → observation bundle.
5. Use free energy for tier gating (F replaces anomaly-count formula).
6. Persist belief state across sessions in `.roko/state/beliefs.bin` (1,280 bytes).
7. Dream consolidation (NREM phase) updates μ_prior via slow prior update during Delta.

## Estimated impact

Full inference cycle < 10 μs on M-series Apple Silicon; belief state serialises to exactly 1,280 bytes. No explicit throughput or cost estimate is stated; the source notes: "No other framework has a world model this compact and algebraically composable."

## Prerequisites

- `bardo-primitives` HDC vector operations already in crate.
- 16 T0 probe outputs converted to HDC observation vectors.
- Daimon PAD baseline available for μ_prior initialisation.
- Heartbeat gating refactor to consume free energy scalar rather than anomaly count.

## Status

Speculative — idea only; no formal evaluation. Ranked **P0** in the source implementation priority table.

## Risks and objections

- Majority-vote bundling is lossy; if too many observations are bundled, beliefs may become underdetermined.
- Learning rate α must be carefully calibrated — too high causes instability, too low causes slow adaptation.
- The HDC belief state is a compact *approximation*; for domains requiring precise belief tracking, richer representations may be needed (see Innovation 6: Witness World Model).

## Related innovations

- [witness-world-model](./witness-world-model.md) — richer, graph-structured alternative for mature agents with sufficient history
- [knowledge-morphogenesis](./knowledge-morphogenesis.md) — HDC fingerprints also drive specialisation gradients
- [affect-causal-discovery](./affect-causal-discovery.md) — causal discovery operates over the same episode history that feeds belief updates
- [dream-verification](./dream-verification.md) — REM imagination may update μ_prior via Delta consolidation

## References

- Bybee & Bhatt (2024). Modelling Neural Probabilistic Computation Using Vector Symbolic Architectures. *Frontiers in Computational Neuroscience* 18.
- Heddes et al. (2024). Hyperdimensional Computing: A Framework for Stochastic Computation and Symbolic AI. *Journal of Big Data*.
- Renner et al. (2024). Brain-Inspired Computational Intelligence via Predictive Coding. arXiv:2308.07870v3.
- Friston (2010). The free-energy principle: a unified brain theory? *Nature Reviews Neuroscience* 11(2).
- Kanerva (2009). Hyperdimensional Computing: An Introduction to Computing in Distributed Representation with High-Dimensional Random Vectors.
- Kleyko et al. (2022). A Survey on Hyperdimensional Computing aka Vector Symbolic Architectures.
