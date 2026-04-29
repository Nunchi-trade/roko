# 2025 Additions — Cross-Domain Research Frontiers

> Cutting-edge 2024-2025 research across cognitive architecture agents, multi-agent coordination, HDC applications, active inference, affect computing, knowledge consolidation, formal verification for AI, TDA for time series, mechanism design for AI economies, and stigmergy. Each citation links to the Roko subsystem it validates.

**Topic**: [References](./INDEX.md)
**Prerequisites**: All domain sub-docs
**Date**: April 2025 survey

> **Implementation**: Reference

---

## Abstract

This document catalogs ~60 papers from the 2024-2025 research frontier that strengthen, validate, or extend Roko's architectural foundations. These papers were discovered through systematic web search across ten research domains. Each paper is annotated with its relevance to specific Roko subsystems. Papers that are also added to their primary domain sub-doc are marked with cross-references. The research landscape shows clear convergence: sleep-inspired compute, active forgetting, emergent coordination in LLM collectives, and formal verification of AI systems are all moving from theoretical curiosities to engineering necessities — exactly the trajectory Roko's architecture anticipated.

---

## 1. Cognitive Architecture for LLM Agents

### Surveys and Taxonomies

- Agentic AI: A Comprehensive Survey (2025). _Artificial Intelligence Review_, Springer.
  *Grounds: Architecture validation — unified taxonomy decomposes LLM agents into six modular dimensions (Core Components, Cognitive Architecture, Learning, Multi-Agent, Environments, Evaluation). Data shows paradigm shift from symbolic (2018-2021) to neural orchestration (post-2022). Validates Roko's neural-first design with structured cognitive overlays. See [20-cognitive-architectures.md](./20-cognitive-architectures.md).*

- Wu, S. et al. (2025). Cognitive LLMs: Integrating Cognitive Architectures and LLMs for Manufacturing Decision-Making. _SAGE Journals_.
  *Grounds: ACT-R + LLM — integrates classical cognitive architectures (ACT-R, SOAR) with LLMs, demonstrating that cognitive architecture principles improve LLM reasoning for structured tasks. Validates layering cognitive architecture onto LLM agents. See [20-cognitive-architectures.md](./20-cognitive-architectures.md).*

- Agentic AI: Architectures, Taxonomies, and Evaluation (2025). arXiv:2601.12560.
  *Grounds: Agent evaluation — 21,730 rollouts confirm scaffold choice matters as much as model choice, replicating and extending the HAL finding. See [20-cognitive-architectures.md](./20-cognitive-architectures.md).*

### Cognitive Workspace

- Cognitive Workspace (2025). Active Memory Management for LLMs. arXiv:2508.13171.
  *Grounds: Context-as-workspace — treats the context window as a cognitive workspace with explicit read/write/evict operations. Mirrors NeuroStore's approach to context as a managed resource, not a passive buffer. See [20-cognitive-architectures.md](./20-cognitive-architectures.md).*

- Position: Episodic Memory is the Missing Piece for Long-Term LLM Agents (2025). arXiv:2502.06975.
  *Grounds: Episodic memory mandate — position paper arguing that episodic memory (event-specific recollection, not just facts) is required for long-horizon agents. Validates NeuroStore's Episode knowledge type as architecturally necessary, not optional.*

---

## 2. Multi-Agent Coordination and Emergent Collectives

### Emergence and Coordination

- Emergent Coordination in Multi-Agent Language Models (2025). arXiv:2510.05174.
  *Grounds: Dynamical emergence — information-theoretic framework measuring higher-order structure in multi-agent LLM systems. Identity-linked differentiation + theory-of-mind instructions produce collective intelligence. See [04-coordination-and-multi-agent.md](./04-coordination-and-multi-agent.md) and [18-collective-intelligence.md](./18-collective-intelligence.md).*

- Emergent Convergence in Multi-Agent LLM Annotation (2025). arXiv:2512.00047.
  *Grounds: Convergence dynamics — LLM groups develop asymmetric influence patterns and negotiation behaviors without explicit role prompting. Validates emergent specialization in Roko Collectives.*

- Multi-Agent Language Models: Advancing Cooperation, Coordination, and Adaptation (2025). arXiv:2506.09331.
  *Grounds: Comprehensive multi-agent LLM survey — covers cooperation mechanisms, coordination protocols, and adaptation strategies across multi-agent LLM systems.*

- Large Language Models Miss the Multi-Agent Mark (2025). arXiv:2505.21298.
  *Grounds: LLM coordination limits — identifies systematic failures in implicit LLM coordination. Motivates explicit coordination mechanisms (Pheromone Field, Agent Mesh) over implicit cooperation. See [18-collective-intelligence.md](./18-collective-intelligence.md).*

- AgentsNet: Coordination and Collaborative Reasoning in Multi-Agent LLMs (2025). arXiv:2507.08616.
  *Grounds: Distributed coordination benchmark — probes up to 100 agents with problems from distributed computing theory. Provides evaluation methodology for Roko's Collective coordination at scale.*

- Multi-Agent Collaboration Mechanisms: A Survey of LLMs (2025). arXiv:2501.06322.
  *Grounds: Collaboration taxonomy — role-based division, debate-style refinement, stigmergic coordination. See [04-coordination-and-multi-agent.md](./04-coordination-and-multi-agent.md).*

- LLM-Coordination: Evaluating Multi-agent Coordination Abilities (2025). _NAACL_, 2025.
  *Grounds: Coordination evaluation — benchmark and evaluation framework specifically for multi-agent LLM coordination abilities.*

- Agentic LLMs in the Supply Chain: Multi-Agent Consensus-Seeking (2025). _International Journal of Production Research_.
  *Grounds: Consensus mechanisms — multi-agent LLM consensus-seeking with transactive reasoning and balanced collective convergence. Applicable to Roko's multi-agent decision procedures.*

### Cooperation Game Theory

- Fontana, M. et al. (2024). Nicer Than Humans: How Do LLMs Behave in the Prisoner's Dilemma? arXiv:2406.13605.
  *Grounds: LLM cooperation — LLMs exhibit cooperation patterns exceeding human baselines in game-theoretic settings. Validates feasibility of cooperative multi-agent LLM systems. See [04-coordination-and-multi-agent.md](./04-coordination-and-multi-agent.md).*

---

## 3. Stigmergy and Swarm Intelligence

- Stigmergy: From Mathematical Modelling to Control (2024). _Proceedings of the Royal Society A_.
  *Grounds: PDE-based stigmergy — formal mathematical framework treating swarms as fluids, transforming stigmergic coordination into a single PDE. Provides rigorous mathematical foundation for Roko's Pheromone Field. See [04-coordination-and-multi-agent.md](./04-coordination-and-multi-agent.md) and [05-biological-analogues.md](./05-biological-analogues.md).*

- Automatic Design of Stigmergy-Based Behaviours for Robot Swarms (2024). _Communications Engineering_, Nature.
  *Grounds: Automatic design — strategy to automatically design stigmergy-based collective behaviors, validated in simulation and real-robot experiments. See [04-coordination-and-multi-agent.md](./04-coordination-and-multi-agent.md).*

- Stigmergy Facilitates Emergent Patterns in Academic Communication (2025). Research Square.
  *Grounds: Human stigmergy — citation patterns follow stigmergic dynamics; digital cues on platforms (wikis, blockchain) enable decentralized coordination. Validates digital stigmergy beyond biological and robotic domains.*

- Enhancing Radioactive Environment Exploration with Stigmergy (2024). _Robotics and Autonomous Systems_.
  *Grounds: Stigmergy vs Levy flight — comparative analysis showing stigmergy outperforms random exploration in hazardous environments. See [05-biological-analogues.md](./05-biological-analogues.md).*

---

## 4. HDC and Vector Symbolic Architecture Applications

- Rahimi, A. et al. (2024). HDC: A Framework for Stochastic Computation and Symbolic AI. _Journal of Big Data_.
  *Grounds: Unified framework — HDC as both stochastic computation framework and symbolic AI system. See [09-hdc-vsa.md](./09-hdc-vsa.md).*

- FLASH: Adaptive Encoder for HDC (2024). _Frontiers in AI_.
  *Grounds: Learnable encoding — gradient-descent-based encoder matrix learning bridges fixed and learned encoding phases. See [09-hdc-vsa.md](./09-hdc-vsa.md).*

- HPVM-HDC: Heterogeneous Programming System (2024). arXiv:2410.15179.
  *Grounds: HDC portability — unified programming model for CPU/GPU/FPGA HDC execution. See [09-hdc-vsa.md](./09-hdc-vsa.md).*

- Hyperdimensional Computing in Biomedical Sciences (2025). PMC review.
  *Grounds: HDC maturity — comprehensive review of production HDC deployments validating practical viability. See [09-hdc-vsa.md](./09-hdc-vsa.md).*

- Optimal Hyperdimensional Representation for Learning and Cognitive Computation (2025). OpenReview.
  *Grounds: Optimal representations — theoretical work on optimal HDC representations for learning, addressing capacity and encoding quality tradeoffs. Informs Roko's 10,240-bit dimensionality choice.*

- The Hyperdimensional Transform for Distributional Modeling (2025). _Neural Computing and Applications_.
  *Grounds: HDC for distributions — hyperdimensional transform enabling distributional modeling, regression, and classification. Applicable to encoding distributional knowledge (confidence ranges, uncertainty) in Roko's HDC vectors.*

---

## 5. Active Inference and Free Energy Principle

- Shafiei, A. et al. (2025). Distributionally Robust Free Energy Principle for Decision-Making. _Nature Communications_, 17, 707.
  *Grounds: Robust active inference — DR-FREE combines robust FEP extension with resolution engine. Agents complete tasks even when SOTA fails under model uncertainty. See [16-active-inference.md](./16-active-inference.md).*

- Koudahl, M.T. et al. (2024). Active Inference for Self-Organizing Multi-LLM Systems. arXiv:2412.10425.
  *Grounds: Cognitive layer for LLMs — active inference framework as cognitive layer above LLM agents, dynamically adjusting prompts through information-seeking behavior. See [16-active-inference.md](./16-active-inference.md).*

- Synthetic Active Inference Agents, Part II (2024). arXiv:2306.02733.
  *Grounds: Scalable implementation — message passing on Forney-style Factor Graphs for generalized free energy minimization. Scalable path for Roko's EFE tier routing. See [16-active-inference.md](./16-active-inference.md).*

- Free Energy Principle and Active Inference in Neural Language Models (2024). CEUR-WS Vol-3923.
  *Grounds: FEP for language models — direct application of FEP to neural language model behavior, showing that language generation can be understood as free energy minimization.*

---

## 6. Affective Computing and Emotion in Agents

- Yin, Y. et al. (2025). Emotions in Artificial Intelligence. arXiv:2505.01462.
  *Grounds: Teleology-driven affect — unifies emotion theories under adaptive, goal-directed premise. See [02-affective-computing.md](./02-affective-computing.md).*

- Intelligent Agents with Emotional Intelligence (2025). arXiv:2511.20657.
  *Grounds: EI for agents — survey identifying emotional intelligence as architecturally vital for agent systems. See [02-affective-computing.md](./02-affective-computing.md).*

- Emotions in the Loop (2025). arXiv:2505.01542.
  *Grounds: Affect-in-loop — comprehensive survey of affective computing integrated into interaction loops. See [02-affective-computing.md](./02-affective-computing.md).*

- CosmoCore: Affective Dream-Replay RL for Code Generation (2025). arXiv:2510.18895.
  *Grounds: Affect + dreams — combines affective states with dream-replay RL, validating the intersection of Daimon and Dreams subsystems. See [03-dreams-and-offline-learning.md](./03-dreams-and-offline-learning.md).*

- Affective Computing and Emotional Data: Challenges in Privacy Regulations (2025). arXiv:2509.20153.
  *Grounds: Regulatory affect — examines privacy implications of emotion-aware AI under the EU AI Act. Informs compliance requirements for Roko's Daimon when operating in regulated domains.*

---

## 7. Knowledge Consolidation, Memory, and Forgetting

### Agent Memory Surveys

- Liu, S. et al. (2025). Memory in the Age of AI Agents. arXiv:2512.13564.
  *Grounds: Memory taxonomy — factual, experiential, working memory distinguished. See [01-memory-consolidation.md](./01-memory-consolidation.md).*

- Wang, Z. et al. (2025). Rethinking Memory in LLM-based Agents. arXiv:2505.00675.
  *Grounds: Six memory operations — Consolidation, Updating, Indexing, Forgetting, Retrieval, Condensation. See [01-memory-consolidation.md](./01-memory-consolidation.md).*

- Memory for Autonomous LLM Agents (2026). arXiv:2603.07670.
  *Grounds: Write-manage-read formalization — five mechanism families for agent memory. See [01-memory-consolidation.md](./01-memory-consolidation.md).*

### Sleep-Inspired Memory

- Language Models Need Sleep (2025). OpenReview.
  *Grounds: LLM sleep paradigm — two-stage sleep: Memory Consolidation + Dreaming. See [03-dreams-and-offline-learning.md](./03-dreams-and-offline-learning.md).*

- NeuroDream (2025). SSRN:5377250.
  *Grounds: Dream-phase consolidation — 38% forgetting reduction, 17.6% zero-shot transfer increase. See [03-dreams-and-offline-learning.md](./03-dreams-and-offline-learning.md).*

- LightMem (2025). arXiv:2510.18866.
  *Grounds: Offline consolidation — 10.9% accuracy gain, 117x token reduction. See [03-dreams-and-offline-learning.md](./03-dreams-and-offline-learning.md).*

- SleepGate (2025). arXiv:2603.14517.
  *Grounds: Active forgetting — learned curation resolving proactive interference. See [03-dreams-and-offline-learning.md](./03-dreams-and-offline-learning.md).*

### Human-Inspired Memory

- Yehudai, M. et al. (2025). ACT-R-Inspired Memory for LLM Agents. _HAI_, 2025.
  *Grounds: ACT-R decay — validates Ebbinghaus-based decay with cognitive science grounding. See [01-memory-consolidation.md](./01-memory-consolidation.md).*

- Enhancing Memory Retrieval in Generative Agents (2025). _Frontiers in Psychology_.
  *Grounds: Cross-attention retrieval — LLM-trained networks improve multi-factor retrieval scoring.*

- Continuum Memory Architectures for Long-Horizon Agents (2026). arXiv:2601.09913.
  *Grounds: Long-horizon memory — memory architecture for extended agent execution, validating tiered persistence.*

---

## 8. Formal Verification for AI Safety

- Position: Formal Methods are the Principled Foundation of Safe AI (2025). _ICML_.
  *Grounds: Formal safety mandate — model checking, theorem proving, and abstract interpretation for AI systems. See [08-security-and-provenance.md](./08-security-and-provenance.md).*

- Towards Guaranteed Safe AI (2024). arXiv:2405.06624.
  *Grounds: Safety guarantee framework — world model + safety specification + verifier = quantitative safety guarantees. Maps to NeuroStore + Policy + Gate. See [08-security-and-provenance.md](./08-security-and-provenance.md).*

- Model Checking Deep Neural Networks (2025). _Frontiers in Computer Science_.
  *Grounds: Temporal logic verification — LTL/CTL for neural network behavior verification. See [08-security-and-provenance.md](./08-security-and-provenance.md).*

- VNN-COMP 2024. International Verification Competition for Neural Networks.
  *Grounds: Verification benchmarks — standardized competition benchmarks for neural network verification tools. The field is rapidly maturing with practical verification at production scale.*

- Formal Methods in Robot Policy Learning and Verification (2025). OpenReview.
  *Grounds: Policy verification — formal methods applied to verifying learned policies, applicable to Roko's Policy trait verification.*

---

## 9. TDA for Time Series and Anomaly Detection

- TDA and Topological Deep Learning Beyond Persistent Homology (2025). _Artificial Intelligence Review_, Springer.
  *Grounds: TDA extensions — persistent topological Laplacians capture both topological invariants and homotopic shape evolution. See [12-signal-processing.md](./12-signal-processing.md).*

- Persistent Homology-Based Unsupervised Anomaly Detection (2025). OpenReview.
  *Grounds: TDA anomaly detection — delay embeddings + distance-to-measure Rips filtration for univariate time series. See [12-signal-processing.md](./12-signal-processing.md).*

- Multivariate Time-Series Anomaly Detection with Topological Analysis (2024). arXiv:2408.13082.
  *Grounds: Graph-TDA — enhanced GAT with persistent homology for inter-feature dependencies. See [12-signal-processing.md](./12-signal-processing.md).*

- Change Point Detection in Financial Time Series Using TDA (2025). _Systems_, 13(10).
  *Grounds: TDA change points — Takens embedding + sliding window for topological change detection. See [12-signal-processing.md](./12-signal-processing.md).*

- Machine Learning of Time Series Using Persistent Homology (2025). _Scientific Reports_, Nature.
  *Grounds: PH-ML bridge — machine learning directly on persistent homology representations of time series data. Provides methodology for converting Roko's agent performance streams into topological features for anomaly detection.*

---

## 10. Mechanism Design for AI Economies

- Agent Exchange (AEX) (2025). arXiv:2507.03904.
  *Grounds: Agent marketplace — RTB-inspired auction engine with User-Side Platform, Agent-Side Platform, Agent Hubs, and Data Management Platform. See [21-mechanism-design.md](./21-mechanism-design.md).*

- Duetting, P. et al. (2024). Mechanism Design for LLMs. _ACM WWW Best Paper_.
  *Grounds: Token auctions — token-by-token mechanism design for multi-LLM output generation. See [21-mechanism-design.md](./21-mechanism-design.md).*

- Deep Mechanism Design (2024). _PNAS_.
  *Grounds: Neural mechanism design — RL-trained neural networks create desirable mechanisms. See [21-mechanism-design.md](./21-mechanism-design.md).*

- Automated Mechanism Design Survey (2024). _ACM SIGecom Exchanges_, 22(2).
  *Grounds: AMD survey — comprehensive survey covering differentiable economics, neural auction design, and automated mechanism design. Provides the theoretical landscape for Roko's VCG attention auction and agent marketplace.*

---

## 11. Self-Learning and Reflection

- SAMULE: Multi-level Reflection for Self-Learning Agents (2025). _EMNLP_, 2025.
  *Grounds: Multi-level reflection — reflection across trajectories outperforms single-trajectory Reflexion. Error clustering extracts insight from failures. See [06-self-learning-systems.md](./06-self-learning-systems.md).*

- MAR: Multi-Agent Reflexion (2025). arXiv:2512.20845.
  *Grounds: Multi-agent reflection — addresses Reflexion's single-agent limitations. See [06-self-learning-systems.md](./06-self-learning-systems.md).*

- Self-Evolving LLMs via Continual Instruction Tuning (2025). arXiv:2509.18133.
  *Grounds: Self-evolution — self-evolution defined as autonomous adaptation to dynamic tasks, cross-task knowledge integration, and sustained performance. Three categories: autonomous learning, dynamic architecture adaptation, knowledge integration frameworks. Maps to Roko's triple-loop learning.*

- The Future of Continual Learning in the Era of Foundation Models (2025). arXiv:2506.03320.
  *Grounds: Continual learning survey — comprehensive survey of continual learning approaches for foundation models, covering both parametric and non-parametric strategies. Validates NeuroStore's non-parametric approach to continual knowledge management.*

---

## Cross-cutting Themes

### Theme 1: Sleep and Offline Compute are Engineering Necessities

Five independent 2025 papers (Language Models Need Sleep, NeuroDream, LightMem, SleepGate, CosmoCore) demonstrate that offline consolidation — the Dreams subsystem — produces measurable improvements: 38% less forgetting, 17.6% better transfer, 10.9% accuracy gains, 117x token savings. This is no longer speculative neuroscience inspiration; it is empirical ML engineering.

### Theme 2: Active Forgetting Outperforms Passive Accumulation

Memory surveys (Liu 2025, Wang 2025) and SleepGate (2025) converge on the conclusion that forgetting is not a bug but a feature. Naive add-all memory degrades performance. The NeuroStore's Curator cycle — active pruning, confidence decay, tier-based retention — is validated by the latest agent memory research.

### Theme 3: Explicit Coordination Beats Implicit LLM Cooperation

Multiple 2025 papers (Large LLMs Miss the Multi-Agent Mark, AgentsNet, LLM-Coordination) show that LLMs fail at implicit coordination. Emergent coordination requires explicit mechanisms: pheromone fields, role prompting, theory-of-mind instructions. Roko's Pheromone Field and Agent Mesh are the right architectural response.

### Theme 4: Formal Verification is Moving from Theory to Practice

The formal verification community (VNN-COMP, Formal Methods for Safe AI) is producing practical tools for verifying AI system behavior. This trajectory aligns with Roko's Gate pipeline: structural verification (compiler, tests, lints) rather than LLM self-assessment.

### Theme 5: Active Inference Works for LLM Systems

DR-FREE (Shafiei 2025, Nature Communications) and Active Inference for Multi-LLM Systems (Koudahl 2024) demonstrate that active inference is not just a theoretical framework but a practical cognitive layer for LLM agents. Roko's EFE-based tier routing and context selection are validated by production-quality research.

---

## Cross-References

- See [01-memory-consolidation.md](./01-memory-consolidation.md) for agent memory surveys
- See [02-affective-computing.md](./02-affective-computing.md) for emotion-in-loop research
- See [03-dreams-and-offline-learning.md](./03-dreams-and-offline-learning.md) for sleep-inspired LLM architectures
- See [04-coordination-and-multi-agent.md](./04-coordination-and-multi-agent.md) for emergent coordination and stigmergy PDE
- See [05-biological-analogues.md](./05-biological-analogues.md) for swarm robotics
- See [06-self-learning-systems.md](./06-self-learning-systems.md) for multi-level reflection
- See [08-security-and-provenance.md](./08-security-and-provenance.md) for formal verification
- See [09-hdc-vsa.md](./09-hdc-vsa.md) for HDC frameworks
- See [12-signal-processing.md](./12-signal-processing.md) for TDA advances
- See [16-active-inference.md](./16-active-inference.md) for DR-FREE and multi-LLM active inference
- See [18-collective-intelligence.md](./18-collective-intelligence.md) for emergent LLM collectives
- See [20-cognitive-architectures.md](./20-cognitive-architectures.md) for agentic AI surveys
- See [21-mechanism-design.md](./21-mechanism-design.md) for agent marketplaces
