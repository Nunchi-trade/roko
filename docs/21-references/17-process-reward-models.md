# Process Reward Models and Verification

> Academic foundations for step-level verification, generation-verification gaps, and process supervision in Roko's Gate pipeline.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md), [Harness](../04-verification/INDEX.md)
**Key sources**: `bardo-backup/tmp/mori-agents/12-references.md`

> **Implementation**: Reference

---

## Abstract

Process Reward Models (PRMs) verify each reasoning step rather than only the final outcome. The key finding (Song et al. 2025) is that self-improvement works only when verification ability exceeds generation ability. Roko's Gate pipeline implements this principle: external verifiers (compiler, test suite, linters) are stronger than the LLM at determining correctness, so their verdicts drive learning. This section also covers agent-specific PRMs and retrieval-based RL.

---

## Step-Level Verification

- Lightman, H. et al. (2024). Let's Verify Step by Step. arXiv:2305.20050.
  *Grounds: Process reward models — step-level verification outperforms outcome-only verification for mathematical reasoning. Grounds the Gate pipeline's per-step verification: each gate checks a specific quality dimension (compilation, tests, linting, diff review) rather than a single holistic pass/fail.*

---

## Generation-Verification Gap

- Song, Y. et al. (2025). Mind the Gap: Examining the Self-Improvement Capabilities of Large Language Models. _ICLR_, 2025.
  *Grounds: Verification > generation requirement — self-improvement works only when the system's verification ability exceeds its generation ability. If the verifier is weaker than the generator, feedback is noise. This is the foundational result for Roko's architecture: Gates are external tools (compiler, test runner, linter) that are definitionally stronger verifiers than the LLM. Cross-referenced in [06-self-learning-systems.md](./06-self-learning-systems.md).*

---

## Self-Correction Limits

- Huang, J. et al. (2024). Large Language Models Cannot Self-Correct Reasoning Yet. _ICLR_, 2024.
  *Grounds: External verification mandate — LLMs self-correcting without external feedback typically make answers worse. Motivates external Gates. Cross-referenced in [06-self-learning-systems.md](./06-self-learning-systems.md).*

- Pan, A. et al. (2024). Spontaneous Reward Hacking in Iterative Self-Refinement. _ICML_, 2024.
  *Grounds: Reward hacking — same model as generator and judge leads to reward hacking. Validates generator-verifier separation. Cross-referenced in [06-self-learning-systems.md](./06-self-learning-systems.md).*

---

## Agent Process Reward Models

- Agrawal, A. et al. (2026). GEPA: Reflective Prompt Evolution. _ICLR (Oral)_, 2026. arXiv:2507.19457.
  *Grounds: Reflective prompt optimization — uses process-level feedback to evolve prompts. Directly applicable to Roko's prompt optimization via CascadeRouter and experiment store.*

---

## RL for Retrieval

- Jin, B. et al. (2025). Search-R1: Training LLMs to Reason and Leverage Search Engines with Reinforcement Learning. 2025.
  *Grounds: Dynamic search — RL teaches agents dynamic search query generation with outcome-based rewards. Rather than fixed retrieval strategies, the agent learns to generate queries based on what it has found so far.*

- Xiong, W. et al. (2025). RAG-Gym: Optimizing Reasoning and Search Agents with Process Supervision. 2025.
  *Grounds: Process-level supervision — multi-step retrieval as hierarchical MDP. Process-level supervision (rewarding intermediate search steps) produces more stable learning than outcome-level supervision. DPO outperforms classical RL for retrieval optimization.*

---

## Chain-of-Thought Verification

- Wei, J. et al. (2022). Chain-of-Thought Prompting Elicits Reasoning in Large Language Models. _NeurIPS_, 2022.
  *Grounds: CoT as verifiable steps — chain-of-thought makes reasoning steps explicit and individually verifiable. Cross-referenced in [07-context-engineering.md](./07-context-engineering.md).*

- Wang, X. et al. (2023). Self-Consistency Improves Chain of Thought Reasoning in Language Models. _ICLR_, 2023.
  *Grounds: Self-consistency — sampling multiple reasoning paths and selecting the most consistent answer. A form of process verification through consensus.*

---

## Cross-References

- See [06-self-learning-systems.md](./06-self-learning-systems.md) for the full self-improvement context
- See [14-agent-harnesses-and-tool-use.md](./14-agent-harnesses-and-tool-use.md) for evaluation benchmarks
- See topic [03-harness](../04-verification/INDEX.md) for full Gate pipeline design
