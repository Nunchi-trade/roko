# Dreams and Offline Learning

> Academic foundations for offline consolidation, creative hypothesis generation, and sleep-time compute in the Roko Dreams subsystem.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md), [Dreams](../10-dreams/INDEX.md)
**Key sources**: `bardo-backup/prd/02-mortality/14-research-foundations.md` §4, `bardo-backup/prd/shared/citations.md` §28

> **Implementation**: Reference

---

## Abstract

The brain dedicates 25-33% of its runtime to a state that prevents environmental interaction — an enormous evolutionary cost that must confer proportional benefits. The benefits are well-characterized: memory consolidation, emotional depotentiation, counterfactual hypothesis generation, and catastrophic forgetting prevention. For Roko agents, idle periods are not waste — they are budget for offline cognitive work. The Dreams subsystem implements three-phase consolidation: NREM replay (Mattar-Daw prioritized access), REM imagination (Boden creativity modes + Pearl SCM counterfactuals + Walker emotional depotentiation), and integration staging.

---

## Hypnagogia and Creative Insight

- Lacaux, C., Andrillon, T., Arnulf, I., & Oudiette, D. (2021). Sleeping on a Problem: Catching the Creative Spark During Sleep Onset. _Science Advances_, 7(50), eabj5866.
  *Grounds: Hypnagogia engine — 83% of subjects discovered hidden rules during brief N1 (sleep onset) exposure vs. 30% staying awake. The foundational result for Roko's hypnagogia engine: the Alpha Convergence Problem (all agents converging to similar solutions) is solved by forcing divergent exploration during idle periods.*

- Lacaux, C. et al. (2024). Sleep Onset Is Not a One-Way Trip: A Comprehensive Review of the N1 Stage. _Trends in Neurosciences_, 47(4), 273-288.
  *Grounds: N1 stage dynamics — comprehensive review establishing the N1 stage as a unique cognitive state with creativity-enhancing properties distinct from both wakefulness and deeper sleep.*

- Haar Horowitz, A. et al. (2020). Dormio: A Targeted Dream Incubation Device. _Consciousness and Cognition_, 83, 102938.
  *Grounds: Targeted dream incubation — MIT Dormio project demonstrated targeted dream incubation at sleep onset. 43% creativity boost via controlled N1 intervention.*

- Haar Horowitz, A., Cunningham, T.J., Maes, P., & Stickgold, R. (2023). Targeted Dream Incubation at Sleep Onset Increases Post-Sleep Creativity. _Scientific Reports_, 13, 7319.
  *Grounds: Creativity boost validation — confirmed 43% creativity boost from targeted dream incubation in a controlled study. Validates the hypnagogia engine's targeted knowledge recombination approach.*

---

## World Models and Imagined Trajectories

- Hafner, D. et al. (2025). DreamerV3: Mastering Diverse Domains through World Models. Working paper.
  *Grounds: REM counterfactual generation — agents trained on imagined trajectories outperform across 150+ tasks. World model dreaming. For Roko, REM-phase consolidation generates synthetic scenarios from the causal model.*

- Ha, D. & Schmidhuber, J. (2018). World Models. arXiv:1803.10122.
  *Grounds: Dream-based training — controller trained entirely inside dreams achieves competitive performance. Demonstrates that dreaming multiplies learning from scarce experience. For Roko, idle-time knowledge recombination creates new hypotheses without real execution.*

---

## Sleep-Time Compute

- Lin, B. et al. (2025). Sleep-time Compute: Beyond Inference Scaling at Test-Time. arXiv:2504.13171.
  *Grounds: Delta-frequency scheduling — dual-agent architecture: Sleeper Agent precomputes during downtime, Serve Agent handles live interactions. ~5× test-time compute reduction, up to 18% accuracy gain. For Roko, dream cycles are sleep-time compute — agents that process experiences during low-activity periods execute fewer expensive T2 inference calls during active work.*

---

## Wake-Sleep Learning

- WSCL (2024). Wake-Sleep Consolidated Learning. arXiv:2401.08623.
  *Grounds: Three-phase dreaming — Complementary Learning Systems three-phase cycle (wake/NREM/REM): 38% reduction in catastrophic forgetting, 17.6% increase in zero-shot transfer. Directly grounds the three-phase dream architecture.*

---

## Replay and Experience Prioritization

- Wagner, U. et al. (2004). Sleep Inspires Insight. _Nature_, 427, 352-355.
  *Grounds: Dream-based insight generation — sleep is 2.6× more likely to produce insight on hidden rule problems. Dream cycles produce genuine insight, not just consolidation.*

- Zhao, J. et al. (2024). BTP: Towards Efficient and Reliable Experience Replay for LLM-Based Agents. arXiv:2410.12236.
  *Grounds: Replay prioritization — P2Value (Possibility and Pass-rate Prioritized Value) for prioritized experience replay in LLM agents. Dream replay prioritizes informative failures.*

- Wang, X. et al. (2024). Prioritized Generative Replay. arXiv:2410.18082.
  *Grounds: Synthetic scenario generation — conditional diffusion generates new transitions near high-value experience regions. REM creates synthetic scenarios, not just replays.*

- Van de Ven, G.M., Siegelmann, H.T., & Tolias, A.S. (2020). Brain-Inspired Replay for Continual Learning with Artificial Neural Networks. _Nature Communications_, 11, 4069.
  *Grounds: Continual learning via replay — brain-inspired replay prevents catastrophic forgetting in artificial neural networks. Validates experience replay as the primary anti-forgetting mechanism.*

---

## Creativity Theory

- Boden, M.A. (2004). _The Creative Mind: Myths and Mechanisms_. 2nd ed. Routledge.
  *Grounds: REM creativity modes — three creativity modes: exploratory (within existing rules), combinational (new combinations of existing elements), and transformational (changing the rules). Roko's REM phase implements combinational creativity via HDC recombination and transformational creativity via causal model intervention.*

---

## Causal Models for Counterfactuals

- Pearl, J. (2009). _Causality: Models, Reasoning, and Inference_. 2nd ed. Cambridge University Press.
  *Grounds: REM counterfactual generation — structural causal models enable counterfactual reasoning: "what would have happened if I had chosen differently?" The agent builds a causal graph of its domain, then generates counterfactuals by intervening on causal variables during REM dreaming. Cross-referenced in [01-memory-consolidation.md](./01-memory-consolidation.md).*

---

## Hauntology and Experiential Traces

- Derrida, J. (1993). _Specters of Marx: The State of the Debt, the Work of Mourning and the New International_. Routledge (English translation 1994).
  *Grounds: Hypnagogia engine uniqueness — each agent is "differently haunted" by its own experiential traces. Derrida's hauntology provides the theoretical frame for why recombining different experiential histories produces different creative outputs, solving the Alpha Convergence Problem.*

---

## Generative Agents and Reflection

- Park, J.S. et al. (2023). Generative Agents: Interactive Simulacra of Human Behavior. _UIST_, 2023. arXiv:2304.03442.
  *Grounds: Memory + reflection architecture — the reflection mechanism in Generative Agents (periodic synthesis of higher-order observations from raw episodes) maps to the Theta-frequency reflection cycle. Cross-referenced in [01-memory-consolidation.md](./01-memory-consolidation.md).*

---

## Sleep-Inspired Architectures for LLMs (2025)

- Language Models Need Sleep (2025). Learning to Self-Modify and Consolidate Memories. OpenReview.
  *Grounds: LLM sleep paradigm — introduces a sleep paradigm with two stages: Memory Consolidation (RL-based Knowledge Seeding for parameter expansion) and Dreaming (self-improvement via synthetic data generation). Directly validates Roko's NREM/REM two-phase dream architecture at the LLM level.*

- Tutuncuoglu, B.T. (2025). NeuroDream: A Sleep-Inspired Memory Consolidation Framework for Artificial Neural Networks. SSRN:5377250.
  *Grounds: Dream-phase framework — explicit dream phase where models disconnect from input and engage in internally generated simulations from stored latent embeddings. Up to 38% reduction in forgetting, 17.6% increase in zero-shot transfer. Validates Roko's REM counterfactual generation producing genuine knowledge recombination.*

- LightMem (2025). Lightweight and Efficient Memory-Augmented Generation. arXiv:2510.18866.
  *Grounds: Sleep-time memory update — offline consolidation procedure decouples memory management from online inference; up to 10.9% accuracy improvement with 117x token reduction. Validates Delta-frequency consolidation operating asynchronously from Gamma-frequency execution.*

- SleepGate (2025). Learning to Forget: Sleep-Inspired Memory Consolidation for Resolving Proactive Interference in LLMs. arXiv:2603.14517.
  *Grounds: Active learned forgetting — brain's solution to proactive interference is an active, learned curation process. Validates NeuroStore's Curator as an active forgetting agent, not a passive decay mechanism.*

- CosmoCore (2025). Affective Dream-Replay Reinforcement Learning for Code Generation. arXiv:2510.18895.
  *Grounds: Affect-modulated dreams — combines affective states with dream-replay RL for code generation. Validates the intersection of Daimon (affect) and Dreams (offline consolidation) subsystems.*

---

## Cross-References

- See [01-memory-consolidation.md](./01-memory-consolidation.md) for Wilson & McNaughton 1994, Mattar & Daw 2018, and other replay citations
- See [02-affective-computing.md](./02-affective-computing.md) for Walker & van der Helm 2009 emotional depotentiation
- See [13-philosophy.md](./13-philosophy.md) for Derrida 1993 in the philosophy context
- See topic [10-dreams](../10-dreams/INDEX.md) for full Dreams subsystem design
