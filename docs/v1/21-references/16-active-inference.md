# Active Inference and Free Energy Principle

> Academic foundations for active inference, expected free energy, Bayesian surprise, and predictive processing in Roko's context selection and action selection.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md)
**Key sources**: `bardo-backup/tmp/agent-chain/14-academic-foundations.md` §4, `bardo-backup/prd/shared/citations.md` §4

> **Implementation**: Reference

---

## Abstract

Active inference provides the principled answer to "how should an agent decide what to attend to?" Expected Free Energy (EFE) decomposes into pragmatic value (goal achievement) and epistemic value (information gain), resolving the exploration-exploitation dilemma without ad-hoc hyperparameters. Roko uses EFE for both context selection (which knowledge to retrieve) and action selection (which cognitive tier to invoke). This is not a metaphor — the mathematics directly determine retrieval ranking and tier routing.

---

## Free Energy Principle

- Friston, K. (2006). A Free Energy Principle for the Brain. _Journal of Physiology–Paris_, 100(1-3), 70-87.
  *Grounds: FEP foundation — all self-organizing systems minimize variational free energy (the gap between internal model and reality). The foundational principle for Roko's prediction-error-driven cognition.*

- Friston, K. (2010). The Free-Energy Principle: A Unified Brain Theory? _Nature Reviews Neuroscience_, 11(2), 127-138.
  *Grounds: Unified theory — the FEP as a unifying principle for brain function. Free energy minimization unifies perception, action, and learning under a single mathematical framework.*

---

## Expected Free Energy and Exploration

- Friston, K., Rigoli, F., Ognibene, D., Mathys, C., Fitzgerald, T., & Pezzulo, G. (2015). Active Inference and Epistemic Value. _Cognitive Neuroscience_, 6(4), 187-214.
  *Grounds: EFE decomposition — Expected Free Energy = pragmatic_value + epistemic_value. Resolves exploration-exploitation without hyperparameters. In Roko, high-epistemic-value knowledge gets prioritized when the agent is uncertain; high-pragmatic-value knowledge dominates when confident.*

- Millidge, B., Tschantz, A., & Buckley, C.L. (2021). Whence the Expected Free Energy? _Neural Computation_, 33(2), 447-482.
  *Grounds: EFE analysis — critical analysis showing naïve extension of free energy into the future actually discourages exploration. The decomposition requires careful formulation. Essential corrective for proper EFE implementation.*

---

## Active Inference Textbook

- Parr, T., Pezzulo, G., & Friston, K. (2022). _Active Inference: The Free Energy Principle in Mind, Brain, and Behavior_. MIT Press.
  *Grounds: Comprehensive treatment — the first comprehensive textbook on active inference. Provides the complete mathematical framework for implementing active inference in artificial agents.*

---

## Bayesian Surprise

- Itti, L. & Baldi, P. (2005). Bayesian Surprise Attracts Human Attention. _NeurIPS_, 2005.
  *Grounds: Novelty detection — Bayesian surprise as KL divergence between posterior and prior beliefs. Formally identical to the epistemic value component of EFE. Active inference agents naturally seek knowledge with the highest Bayesian surprise — artifacts that would most change the agent's current beliefs.*

---

## Active Inference for LLMs

- Koudahl, M.T. et al. (2024). Active Inference for Self-Organizing Multi-LLM Systems. arXiv, 2024.
  *Grounds: Multi-LLM active inference — applies active inference to balance exploration and exploitation across prompt combinations in multi-LLM systems.*

- BED-LLM (2025). Bayesian Experimental Design for LLMs. Oxford. arXiv, 2025.
  *Grounds: Sequential information gathering — formulates interactive information gathering as sequential Bayesian experimental design using expected information gain.*

---

## Implementation Libraries

- Heins, C., Millidge, B., Demekas, D. et al. (2022). pymdp: A Python Library for Active Inference on Discrete State Spaces. _Journal of Open Source Software_.
  *Grounds: Active inference implementation — JAX-accelerated open-source library for active inference. Provides reference implementations for the discrete state-space active inference used in Roko's tier routing.*

---

## Predictive Processing

- Rao, R.P. & Ballard, D.H. (1999). Predictive Coding in the Visual Cortex: A Functional Interpretation of Some Extra-Classical Receptive-Field Effects. _Nature Neuroscience_, 2(1), 79-87.
  *Grounds: Predictive coding — cortical computation as hierarchical predictive coding. Top-down predictions are compared with bottom-up signals; only prediction errors propagate. Grounds Roko's prediction-error-driven cognitive tier routing.*

- Clark, A. (2013). Whatever Next? Predictive Brains, Situated Agents, and the Future of Cognitive Science. _Behavioral and Brain Sciences_, 36(3), 181-204.
  *Grounds: Predictive processing framework — cross-referenced in [12-signal-processing.md](./12-signal-processing.md).*

---

## Dopaminergic Prediction Errors

- Doya, K. (2002). Metalearning and Neuromodulation. _Neural Networks_, 15(4-6), 495-506.
  *Grounds: Neuromodulation — different neuromodulators (dopamine, serotonin, norepinephrine, acetylcholine) control different aspects of learning. Maps to the 7-axis Score: different scoring axes modulate different learning signals.*

- Rescorla, R.A. & Wagner, A.R. (1972). A Theory of Pavlovian Conditioning. In _Classical Conditioning II_. Appleton-Century-Crofts.
  *Grounds: Prediction error learning — learning driven by the discrepancy between expected and actual outcomes. The simplest form of the prediction error that drives Roko's T0 probes and CalibrationTracker.*

---

## Distributionally Robust Active Inference (2025)

- Shafiei, A., Jesawada, H., Friston, K., & Russo, G. (2025). Distributionally Robust Free Energy Principle for Decision-Making. _Nature Communications_, 17, 707.
  *Grounds: Robust active inference — DR-FREE model instills robustness by design, combining a robust extension of the free energy principle with a resolution engine. Agents complete tasks even when state-of-the-art models fail under training-environment ambiguity. Directly applicable to Roko's tier routing under model uncertainty — the agent should route robustly even when the domain model is imperfect.*

---

## Active Inference for LLM Systems (2024-2025)

- Koudahl, M.T. et al. (2024). Active Inference for Self-Organizing Multi-LLM Systems: A Bayesian Thermodynamic Approach to Adaptation. arXiv:2412.10425.
  *Grounds: Multi-LLM active inference — cognitive layer above LLM-based agents dynamically adjusts prompts and search strategies through principled information-seeking behavior. Validates Roko's active-inference-driven context assembly: the agent selects which knowledge to retrieve by minimizing expected free energy.*

- Synthetic Active Inference Agents (2024). Realising Synthetic Active Inference Agents, Part II: Variational Message Updates. arXiv:2306.02733.
  *Grounds: Scalable active inference — message passing on free-form Forney-style Factor Graphs minimizes generalized free energy objectives. Provides a scalable implementation path for Roko's EFE-based tier routing beyond toy state spaces.*

---

## Cross-References

- See [12-signal-processing.md](./12-signal-processing.md) for predictive processing
- See [15-cybernetics-and-vsm.md](./15-cybernetics-and-vsm.md) for cybernetic regulation
- See [20-cognitive-architectures.md](./20-cognitive-architectures.md) for CoALA and dual-process cognition
