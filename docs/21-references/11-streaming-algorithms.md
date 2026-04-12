# Streaming Algorithms and Online Statistics

> Academic foundations for probabilistic data structures, adaptive windowing, online estimation, and streaming computation used in Roko's real-time monitoring.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md)
**Key sources**: `bardo-backup/prd/shared/citations.md` §§10-11

---

## Abstract

Roko agents process continuous data streams at Gamma frequency (~5-15s ticks). Streaming algorithms provide constant-memory, single-pass computation: approximate counting (HyperLogLog), frequency estimation (Count-Min Sketch), set membership (Bloom filters), and change detection (ADWIN). These are the computational primitives behind T0 probes and real-time monitoring.

---

## Adaptive Windowing

- Bifet, A. & Gavaldà, R. (2007). Learning from Time-Changing Data with Adaptive Windowing. _SIAM_, 2007.
  *Grounds: ADWIN — adaptive window algorithm that automatically detects distribution change and adjusts window size. Used in Roko's drift detection for model routing — when prediction accuracy drifts, the window shrinks to respond faster.*

---

## Calibration and Uncertainty Estimation

- Guo, C. et al. (2017). On Calibration of Modern Neural Networks. _ICML_, 2017. arXiv:1706.04599.
  *Grounds: Calibration measurement — modern neural networks are poorly calibrated; temperature scaling is a simple fix. Grounds the CalibrationTracker's bias correction.*

- Lakshminarayanan, B., Pritzel, A., & Blundell, C. (2017). Simple and Scalable Predictive Uncertainty Estimation using Deep Ensembles. _NeurIPS_, 2017. arXiv:1612.01474.
  *Grounds: Ensemble uncertainty — deep ensembles provide well-calibrated uncertainty estimates. Informs multi-model confidence aggregation in Roko's CascadeRouter.*

- Vovk, V., Gammerman, A., & Shafer, G. (2005). _Algorithmic Learning in a Random World_. Springer.
  *Grounds: Conformal prediction — distribution-free prediction intervals with guaranteed coverage. Provides theoretical foundation for prediction confidence bounds in Roko.*

- Farquhar, S. et al. (2024). Detecting Hallucinations in Large Language Models Using Semantic Entropy. _Nature_, 630.
  *Grounds: Hallucination detection — semantic entropy detects hallucinations by measuring consistency across sampled completions. Informs confidence estimation in Roko's Gate pipeline.*

- Xiong, M. et al. (2023). Can LLMs Express Their Uncertainty? An Empirical Evaluation of Confidence Elicitation in LLMs. arXiv:2306.13063.
  *Grounds: LLM confidence — empirical evaluation of how well LLMs express their own uncertainty. Informs the confidence scoring in Roko's 7-axis Score.*

---

## Distributional Reinforcement Learning

- Dabney, W. et al. (2018). Distributional Reinforcement Learning with Quantile Regression. _AAAI_, 2018.
  *Grounds: Quantile regression — distributional RL that learns the full distribution of returns, not just the mean. Provides richer signals for Roko's learning systems.*

- Dabney, W. et al. (2020). A Distributional Code for Value in Dopamine-Based Reinforcement Learning. _Nature_, 577.
  *Grounds: Neuroscientific validation — dopamine neurons encode distributional (not just expected) value, validating distributional RL as neurobiologically plausible.*

---

## Anytime Algorithms

- Hansen, E.A. & Zilberstein, S. (2001). Monitoring and Control of Anytime Algorithms. _Artificial Intelligence_, 126(1-2).
  *Grounds: Anytime computation — algorithms that produce progressively better results with more compute time and can be interrupted at any point. Grounds the T0/T1/T2 cascade where the agent stops at the cheapest sufficient tier.*

---

## Cross-references

- See [12-signal-processing.md](./12-signal-processing.md) for Kalman filtering and spectral methods
- See [16-active-inference.md](./16-active-inference.md) for free energy-based uncertainty
- See topic [00-architecture](../00-architecture/INDEX.md) for T0 probe architecture
