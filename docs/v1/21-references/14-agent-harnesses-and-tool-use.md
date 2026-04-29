# Agent Harnesses and Tool Use

> Academic foundations for agent scaffolding, harness engineering, tool interfaces, and coding agent systems that inform Roko's framework and harness layers.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md), [Framework](../00-architecture/INDEX.md), [Harness](../04-verification/INDEX.md)
**Key sources**: `bardo-backup/tmp/mori-agents/12-references.md`, `bardo-backup/prd/shared/citations.md` §3

> **Implementation**: Reference

---

## Abstract

The Roko thesis — "the scaffold IS the product" — is grounded in empirical evidence that the same LLM performs 6x better or worse depending on its surrounding harness code. This section collects the agent scaffolding research: Meta-Harness (Lee et al. 2026), SWE-agent (Yang et al. 2024), coding agent evaluation (SWE-bench), and practical patterns for multi-agent orchestration. The core finding is that scaffold choice matters as much as model choice for agent performance.

---

## Harness Engineering

- Lee, H., Chen, M., Gupta, A., & Hashimoto, T. (2026). Meta-Harness: End-to-End Optimization of Model Harnesses. arXiv:2603.28052.
  *Grounds: Core thesis — 6x performance gap from scaffold changes alone. +7.7 points text classification, +4.7 points IMO math, at 4x fewer tokens. An agent reads its own scaffold history, proposes improvements, benchmarks them, and iterates. The foundational paper for Roko's approach. Cross-referenced in [06-self-learning-systems.md](./06-self-learning-systems.md).*

- Pan, J., Lin, Z., & Hashimoto, T. (2026). Natural-Language Agent Harnesses. arXiv:2603.25723.
  *Grounds: Natural-language scaffolds — scaffold logic as natural language specifications interpreted by an intelligent runtime. Cross-referenced in [06-self-learning-systems.md](./06-self-learning-systems.md).*

- Kapoor, S. et al. (2026). HAL: A Holistic Agent Leaderboard. _ICLR_, 2026.
  *Grounds: Scaffold importance — 21,730 agent rollouts show scaffold choice matters as much as model choice. Cross-referenced in [06-self-learning-systems.md](./06-self-learning-systems.md).*

---

## Agent-Computer Interfaces

- Yang, J. et al. (2024). SWE-agent: Agent-Computer Interfaces Enable Automated Software Engineering. _NeurIPS_, 2024.
  *Grounds: ACI design — Agent-Computer Interfaces purpose-built for LLM agents. How an agent interacts with its environment (tool design, output formatting, feedback structure) matters as much as reasoning. Influences Roko's tool permissions, structured error digests, and role-specific feedback formatting.*

- Gauthier, P. (2024). Aider: AI Pair Programming in Your Terminal. aider.chat.
  *Grounds: Repository map — pioneered practical patterns for AI-assisted code editing: edit format negotiation, repository map construction, and diff-based output parsing. Influences Roko's workspace map generation.*

---

## Multi-Agent Orchestration

- Anthropic (2024). Building Effective Agents. anthropic.com.
  *Grounds: Composition over complexity — keep individual agents simple, compose through a controller. Directly influenced Roko's architecture: each agent role does one thing, the orchestrator composes them into pipelines.*

---

## Model Routing

- Chen, L., Zaharia, M., & Zou, J. (2023). FrugalGPT: How to Use Large Language Models While Reducing Cost and Improving Performance. arXiv:2305.05176.
  *Grounds: 16 T0 probes — cascade architectures can achieve up to 98% cost reduction while matching top-model quality. The key is intelligent routing. Grounds Roko's T0/T1/T2 cascade.*

- Ong, I., Almahairi, A., & Manning, C.D. (2024). RouteLLM: Learning to Route LLMs with Preference Data. arXiv:2406.18665.
  *Grounds: Preference-based routing — learn routing decisions from preference data. Informs the CascadeRouter's training on historical task outcomes.*

- Yoshida, S., Nishida, K., & Okazaki, N. (2024). System 1 to System 2 Distillation for Efficient Tool-Using Agents. arXiv:2407.xxxxx.
  *Grounds: Dual-process agents — distilling System 2 (slow, deliberate) capabilities into System 1 (fast, automatic) operation. Directly grounds Roko's dual-process T0/T1/T2 architecture.*

---

## Cognitive Architecture Integration

- Sumers, T.R., Yao, S., Narasimhan, K., & Griffiths, T.L. (2023). Cognitive Architectures for Language Agents (CoALA). arXiv:2309.02427.
  *Grounds: 9-step cognitive loop — defines the CoALA framework mapping to Roko's universal loop. Cross-referenced in [20-cognitive-architectures.md](./20-cognitive-architectures.md).*

- Park, J.S. et al. (2023). Generative Agents: Interactive Simulacra of Human Behavior. _UIST_, 2023. arXiv:2304.03442.
  *Grounds: Memory + reflection — memory, retrieval, and reflection architecture producing emergent social behaviors. Cross-referenced in [01-memory-consolidation.md](./01-memory-consolidation.md).*

---

## Evaluation and Benchmarks

- Jimenez, C.E. et al. (2024). SWE-bench: Can Language Models Resolve Real-World GitHub Issues? _ICLR_, 2024.
  *Grounds: Agent benchmark — 2,294 real GitHub issues from 12 Python repositories. The gold standard for coding agent evaluation.*

- Shahul Es, S. et al. (2024). RAGAS: Automated Evaluation of Retrieval Augmented Generation. _EACL_, 2024.
  *Grounds: RAG evaluation — three metrics (faithfulness, answer relevance, context relevance) for automated RAG evaluation without human annotations.*

- Saad-Falcon, J. et al. (2024). ARES: An Automated Evaluation Framework for RAG Systems. _NAACL_, 2024.
  *Grounds: Statistical RAG evaluation — Prediction-Powered Inference provides statistically valid RAG evaluation from ~300 human labels with confidence intervals.*

- Liu, X. et al. (2024). AgentBench: Evaluating LLMs as Agents. _ICLR_, 2024.
  *Grounds: Multi-environment evaluation — tests agents across eight environments. Agent performance varies dramatically across environments, supporting environment-specific scaffold design.*

---

## Cross-References

- See [06-self-learning-systems.md](./06-self-learning-systems.md) for self-improvement mechanisms
- See [07-context-engineering.md](./07-context-engineering.md) for context assembly optimization
- See [20-cognitive-architectures.md](./20-cognitive-architectures.md) for CoALA and cognitive frameworks
