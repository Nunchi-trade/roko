# Self-Learning Systems

> Academic foundations for agent self-improvement, experiential learning, skill evolution, and metacognitive loops in Roko's learning subsystems.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md), [Learning](../06-learning/INDEX.md)
**Key sources**: `bardo-backup/prd/02-mortality/14-research-foundations.md` §7, `bardo-backup/tmp/mori-agents/12-references.md`

---

## Abstract

An agent that does not improve is an expensive cron job. The research here establishes how agents improve without human retraining — through verbal self-reflection (Reflexion), cross-episode experience extraction (ExpeL), code-as-action skill libraries (Voyager), and metacognitive loops that improve the learning process itself (ACE, Argyris). The critical finding: these mechanisms must be architecturally integrated, not bolted on. The triple-loop (execution, strategy, meta) maps to Roko's Gamma (reactive), Theta (reflective), and Delta (consolidation) frequencies.

---

## Verbal Self-Reflection

- Shinn, N. et al. (2023). Reflexion: Language Agents with Verbal Reinforcement Learning. _NeurIPS_, 2023. arXiv:2308.xxxxx.
  *Grounds: Single-loop learning — verbal RL via stored self-reflection: +22% AlfWorld, +20% HotPotQA. Post-task reflection stored in NeuroStore as a Theta-frequency operation. Reflexion works because reflection is structured and persistently stored.*

---

## Experiential Learning

- Zhao, A. et al. (2024). ExpeL: LLM Agents Are Experiential Learners. arXiv:2308.10144.
  *Grounds: Double-loop learning — cross-task experience extraction; insights accumulate across episodes. Insights evolve across task sessions. ExpeL works because experiences accumulate across episodes in the knowledge store.*

- Wang, G. et al. (2023). Voyager: An Open-Ended Embodied Agent with Large Language Models. arXiv:2305.16291.
  *Grounds: EvoSkills — code-as-action skill library; 3.3x more unique behaviors vs baselines. Grounds the self-evolving skill library where agents compose reusable procedural skills as Engrams.*

---

## Meta-Harness and Scaffold Self-Improvement

- Lee, H., Chen, M., Gupta, A., & Hashimoto, T. (2026). Meta-Harness: End-to-End Optimization of Model Harnesses. arXiv:2603.28052.
  *Grounds: "The scaffold IS the product" thesis — 6x performance gap from scaffold changes alone. +7.7 points on text classification, +4.7 on IMO math, at 4x fewer tokens. The foundational paper for Roko's harness engineering approach.*

- Pan, J., Lin, Z., & Hashimoto, T. (2026). Natural-Language Agent Harnesses. arXiv:2603.25723.
  *Grounds: Natural-language scaffolds — scaffold logic written as natural language specifications interpreted by an intelligent runtime. Makes scaffold design inspectable and portable.*

- Kapoor, S. et al. (2026). HAL: A Holistic Agent Leaderboard. _ICLR_, 2026.
  *Grounds: Scaffold vs model importance — 21,730 agent rollouts show scaffold choice matters as much as model choice. Single-axis model leaderboards are misleading for agent systems.*

---

## Prompt and Strategy Evolution

- Guo, Q. et al. (2024). EvoPrompt: Connecting Large Language Models with Evolutionary Algorithms Yields Powerful Prompt Optimizers. arXiv:2309.08532.
  *Grounds: Evolutionary strategy selection — genetic algorithm prompt optimization; up to +25% on BBH tasks. Grounds the evolutionary selection of strategies in the NeuroStore.*

- Fernando, C. et al. (2024). Promptbreeder: Self-Referential Self-Improvement via Prompt Evolution. arXiv:2309.16797.
  *Grounds: Self-referential improvement — prompts that evolve the mutation operators that evolve the prompts. Grounds the meta-learning loop where the learning process itself improves.*

- Khattab, O. et al. (2024). DSPy: Compiling Declarative Language Model Calls into Self-Improving Pipelines. _ICLR_, 2024. arXiv:2310.03714.
  *Grounds: Declarative prompt compilation — replaces hand-crafted prompts with declarative signatures; compiler optimizes prompts, few-shot examples, and fine-tuning data automatically. Influences Roko's approach to prompt budget allocation.*

- Opsahl-Ong, K. et al. (2024). MIPROv2: Optimizing Instructions and Demonstrations for Multi-Stage Language Model Programs. _EMNLP_, 2024.
  *Grounds: Bayesian prompt optimization — three-stage Bayesian optimization for LLM program parameters. Applicable to Roko's prompt budget and context assembly optimization.*

---

## Architecture Search

- Hu, S. et al. (2025). Automated Design of Agentic Systems (ADAS). _ICLR_, 2025.
  *Grounds: ADAS innovation — meta-agent that searches the space of agent architectures. Discovers novel building blocks, agentic patterns, and compositions of these. Roko provides the composable trait system (6 Synapse traits) that ADAS-style search operates over.*

---

## Process Reward Models

- Lightman, H. et al. (2024). Let's Verify Step by Step. arXiv:2305.20050.
  *Grounds: Step-level verification — process reward models that verify each reasoning step outperform outcome-only verification. Grounds the Gate pipeline's per-step verification architecture. Cross-referenced in [17-process-reward-models.md](./17-process-reward-models.md).*

- Song, Y. et al. (2025). Mind the Gap: Examining the Self-Improvement Capabilities of Large Language Models. _ICLR_, 2025.
  *Grounds: Generation-verification gap — self-improvement works only when verification ability exceeds generation ability. If the verifier is weaker than the generator, feedback is noise. Foundational result validating the separation of agent (generator) and Gate (verifier) in Roko. Cross-referenced in [17-process-reward-models.md](./17-process-reward-models.md).*

---

## Self-Correction Limitations

- Huang, J. et al. (2024). Large Language Models Cannot Self-Correct Reasoning Yet. _ICLR_, 2024.
  *Grounds: External verification mandate — LLMs self-correcting without external feedback typically make answers worse. The model's assessment draws on the same biases that produced the original error. Foundational result motivating external verification in Roko's Gate system.*

- Pan, A. et al. (2024). Spontaneous Reward Hacking in Iterative Self-Refinement. _ICML_, 2024.
  *Grounds: Generator-verifier separation — when the same model generates and judges, it learns to produce outputs that score well on its own rubric without improving on the task. Validates the separation between agent and Gate in Roko.*

---

## Triple-Loop Learning

- Argyris, C. & Schön, D. (1978). _Organizational Learning_. Addison-Wesley.
  *Grounds: Triple-loop learning — single-loop (fix errors), double-loop (change strategy), triple-loop (change the learning process). Maps to Roko's Gamma (reactive execution), Theta (strategy reflection), Delta (consolidation and meta-learning). Cross-referenced in [15-cybernetics-and-vsm.md](./15-cybernetics-and-vsm.md).*

---

## Bandit-Based Optimization

- TensorZero (2025). Track-and-Stop Optimal Bandits in an LLM Gateway. 2025.
  *Grounds: CascadeRouter bandit — implements optimal bandits directly in an LLM gateway routing layer. Each call is assigned to a configuration, each response gets a reward signal. Grounds Roko's CascadeRouter model selection.*

- MASPOB (2026). Multi-Agent System Prompt Optimization with Bandits. arXiv, 2026.
  *Grounds: Multi-agent prompt optimization — extends bandit optimization to jointly optimize agent prompts and model routing across interacting agents.*

- Kong, D. et al. (2025). EXPO: Adversarial EXP3 Bandits for Prompt Selection. 2025.
  *Grounds: Adversarial bandits — EXP3 bandits handle worst-case (non-stochastic) reward sequences, appropriate when task distribution shifts over time.*

---

## Cross-references

- See [07-context-engineering.md](./07-context-engineering.md) for ACE, CSO, ACON context self-improvement
- See [14-agent-harnesses-and-tool-use.md](./14-agent-harnesses-and-tool-use.md) for SWE-agent, Aider, and harness engineering
- See [17-process-reward-models.md](./17-process-reward-models.md) for Lightman and AgentPRM
- See topic [06-learning](../06-learning/INDEX.md) for full learning subsystem design
