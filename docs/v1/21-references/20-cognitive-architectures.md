# Cognitive Architectures

> Academic foundations for cognitive agent architectures, dual-process theory, and computational cognitive science that inform Roko's Synapse Architecture.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md)
**Key sources**: `bardo-backup/prd/shared/citations.md` §3, `bardo-backup/tmp/agent-chain/08-references.md`

> **Implementation**: Reference

---

## Abstract

Roko's universal cognitive loop is not an ad-hoc design but an implementation of established cognitive architecture principles. CoALA (Sumers et al. 2023) provides the 9-step pipeline. Kahneman's System 1/System 2 grounds the dual-process T0/T1/T2 cascade. CLARION's dual-level architecture validates the combination of explicit (declarative) and implicit (procedural) knowledge. ACT-R and SOAR provide decades of validated cognitive architecture design that inform Roko's approach.

---

## CoALA: Cognitive Architectures for Language Agents

- Sumers, T.R., Yao, S., Narasimhan, K., & Griffiths, T.L. (2023). Cognitive Architectures for Language Agents (CoALA). arXiv:2309.02427.
  *Grounds: 9-step cognitive pipeline — defines the CoALA framework: perceive → retrieve → reason → act → learn. Roko's universal loop (PERCEIVE → EVALUATE → ATTEND → INTEGRATE → ACT → VERIFY → PERSIST → ADAPT → META-COGNIZE) extends CoALA with explicit verification (Gate) and meta-cognition (Daimon).*

- Sumers, T.R. et al. (2024). Cognitive Architectures for Language Agents. _Transactions on Machine Learning Research_, 2024.
  *Grounds: CoALA journal version — extended treatment of CoALA with additional analysis and updated cognitive architecture taxonomy.*

---

## Dual-Process Theory

- Kahneman, D. (2011). _Thinking, Fast and Slow_. Farrar, Straus and Giroux.
  *Grounds: System 1/System 2 — dual-process theory: System 1 (fast, automatic, heuristic) and System 2 (slow, deliberate, analytical). Roko implements this as T0 (no LLM, ~80% of ticks), T1 (fast model, ~15%), T2 (full model, ~5%). The routing is not manual but emerges from prediction error.*

- Kahneman, D. & Tversky, A. (1979). Prospect Theory: An Analysis of Decision Under Risk. _Econometrica_, 47(2), 263-292.
  *Grounds: Decision under uncertainty — prospect theory describes how people evaluate losses and gains asymmetrically. Informs the Daimon's asymmetric response to negative vs. positive outcomes.*

- Kahneman, D. (1973). _Attention and Effort_. Prentice-Hall.
  *Grounds: Attention allocation — attention as a scarce computational resource. Grounds the VCG attention auction where subsystems compete for limited context budget.*

---

## Classical Cognitive Architectures

- Anderson, J.R. (1993). _Rules of the Mind_. Lawrence Erlbaum Associates.
  *Grounds: ACT-R — Adaptive Control of Thought-Rational. Distinguishes declarative memory (facts) and procedural memory (skills). Roko's NeuroStore knowledge types map to ACT-R memory types: Insight/CausalLink = declarative; Heuristic/StrategyFragment = procedural.*

- Laird, J.E., Newell, A., & Rosenbloom, P.S. (1987). SOAR: An Architecture for General Intelligence. _Artificial Intelligence_, 33(1), 1-64.
  *Grounds: SOAR — problem solving through search in problem spaces with learning from experience (chunking). The SOAR cycle (propose → decide → apply → learn) maps to Roko's compose → act → verify → adapt.*

- Sun, R. (2002). Duality of the Mind: A Bottom-Up Approach Toward Cognition. Lawrence Erlbaum Associates.
  *Grounds: CLARION — dual-level architecture combining explicit (top-level) and implicit (bottom-level) knowledge. Validates Roko's combination of explicit knowledge (NeuroStore entries) and implicit knowledge (HDC vectors, somatic markers).*

---

## Cognitive Load and Rational Inattention

- Sims, C.A. (2003). Implications of Rational Inattention. _Journal of Monetary Economics_, 50(3), 665-690.
  *Grounds: Rational inattention — rational finite-capacity agents optimally ignore some information. Resource constraints shape attention allocation. Grounds the VCG auction mechanism and the T0 suppression strategy.*

---

## Cognitive Workspace

- Cognitive Workspace (2025). Active Memory Management for LLMs. arXiv:2508.13171.
  *Grounds: Active memory — active memory management treating the context window as a cognitive workspace with explicit read/write/evict operations. Informs the Composer's context management.*

---

## Episodic Memory for LLMs

- Fountas, Z. et al. (2025). EM-LLM: Human-Inspired Episodic Memory for Infinite Context LLMs. _ICLR_, 2025.
  *Grounds: Episodic memory — human-inspired episodic memory enabling infinite effective context through memory retrieval and integration. Informs NeuroStore's episodic knowledge management.*

---

## Complementary Learning Systems

- McClelland, J.L., McNaughton, B.L., & O'Reilly, R.C. (1995). Why There Are Complementary Learning Systems in the Hippocampus and Neocortex. _Psychological Review_, 102(3), 419-457.
  *Grounds: Dual memory systems — fast hippocampal learning and slow neocortical consolidation. Cross-referenced in [01-memory-consolidation.md](./01-memory-consolidation.md).*

---

## Agentic AI Architecture Surveys (2025)

- Agentic AI: A Comprehensive Survey of Architectures, Applications, and Future Directions (2025). _Artificial Intelligence Review_, Springer, 2025.
  *Grounds: Agentic AI taxonomy — unified taxonomy decomposes LLM-based agents into six modular dimensions: Core Components (perception, memory, action, profiling), Cognitive Architecture (planning, reflection), Learning, Multi-Agent Systems, Environments, and Evaluation. Validates Roko's modular architecture (6 Synapse traits + Daimon + Dreams + NeuroStore).*

- Wu, S. et al. (2025). Cognitive LLMs: Toward Human-Like Artificial Intelligence by Integrating Cognitive Architectures and Large Language Models. _SAGE Journals_, 2025.
  *Grounds: Cognitive LLM integration — integrates classical cognitive architectures (ACT-R, SOAR) with modern LLMs for manufacturing decision-making. Validates Roko's approach of layering cognitive architecture principles onto LLM-based agents rather than treating the LLM as a standalone reasoner.*

- Agentic AI: Architectures, Taxonomies, and Evaluation of LLM Agents (2025). arXiv:2601.12560.
  *Grounds: Agent evaluation taxonomy — systematic evaluation framework for agentic AI systems. Data shows clear paradigm shift: symbolic/hybrid cognitive architectures dominated 2018-2021, while neural orchestration frameworks dominate post-2022. Validates Roko's neural-first design with structured cognitive overlays.*

---

## Cross-References

- See [01-memory-consolidation.md](./01-memory-consolidation.md) for CLS and memory systems
- See [02-affective-computing.md](./02-affective-computing.md) for PAD and somatic markers
- See [15-cybernetics-and-vsm.md](./15-cybernetics-and-vsm.md) for cybernetic regulation
- See [16-active-inference.md](./16-active-inference.md) for active inference
