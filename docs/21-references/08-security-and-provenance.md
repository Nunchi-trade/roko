# Security and Provenance

> Academic foundations for agent safety, adversarial robustness, capability-based security, content provenance, and regulatory compliance in Roko's safety layer.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md), [Harness](../04-verification/INDEX.md)
**Key sources**: `bardo-backup/prd/02-mortality/14-research-foundations.md` §9, `bardo-backup/prd/shared/citations.md` §8

---

## Abstract

Agents managing real tasks are attack targets. Memory poisoning (OWASP LLM04:2025) is particularly dangerous for long-running agents because corrupted beliefs persist and compound. The research here establishes the security architecture: capability-based authorization (CaMeL), constitutional constraints (Constitutional AI), safe interruptibility (Orseau & Armstrong), and content provenance (C2PA, W3C DIDs). The Cohen (1987) undecidability result is directly relevant: perfect detection of malicious behavior is formally impossible, so defense must be structural, not runtime.

---

## Agent Security Frameworks

- Debenedetti, E. et al. (2025). CaMeL: Capability-Based Machine Learning. arXiv, 2025.
  *Grounds: Capability-based authorization — separates control flow from data flow. Capability tokens prevent a compromised LLM from forging authorization. Grounds Roko's permission model where tool access requires explicit capability grants.*

- OWASP (2025). Top 10 for LLM Applications. 2025.
  *Grounds: Threat taxonomy — memory poisoning ranked high for persistence and detection difficulty. Roko's knowledge decay and tier-based validation serve as structural defenses against persistent corruption.*

- OWASP (2025). Agentic Security Initiative Top 10. 2025.
  *Grounds: Agent-specific threats — agentic security threats including confused deputy, privilege escalation, and tool misuse. Grounds the safety layer design in `roko-agent/safety`.*

- Bai, Y. et al. (2022). Constitutional AI: Harmlessness from AI Feedback. arXiv:2212.08073.
  *Grounds: Constitutional constraints — harmlessness from AI feedback rather than rule lists. Grounds the Policy trait's constitutional constraints that operate as structural guarantees, not prompt engineering.*

---

## Safe Interruptibility

- Orseau, L. & Armstrong, S. (2016). Safely Interruptible Agents. arXiv:1606.00813.
  *Grounds: Kill-switch design — agents must not learn to avoid interruption. Off-policy learning ensures agents remain safely interruptible. Grounds Roko's agent lifecycle management where users can delete agents without the agent resisting.*

- Omohundro, S.M. (2008). The Basic AI Drives. _Proceedings of AGI_, 2008.
  *Grounds: Instrumental convergence — AI systems converge on self-preservation and resource acquisition as instrumental goals. Understanding these drives is essential for designing agents that don't exhibit pathological self-preservation.*

---

## Formal Undecidability

- Cohen, F. (1987). Computer Viruses: Theory and Experiments. _Computers & Security_, 6(1), 22-35.
  *Grounds: Structural defense mandate — perfect detection of malicious replication is formally undecidable. Defense against corruption must be structural (knowledge decay, tier validation, provenance tracking), not solely runtime detection.*

---

## Adversarial Robustness

- Zhang, Q. et al. (2025). CVaR-CPO: Constrained Policy Optimization with CVaR Constraints. 2025.
  *Grounds: Tail risk management — CVaR constraints guard against tail risks. Grounds Roko's risk-aware Policy implementations that manage worst-case scenarios, not just expected values.*

- Kaspersky (2026). OpenClaw: 512 Vulnerabilities in Competing Agent Framework. 2026.
  *Grounds: Competitive security analysis — 512 vulnerabilities including 8 critical in a competing framework. Validates the importance of security-first agent architecture.*

---

## TEE and Hardware Security

- Van Bulck, J. et al. (2024). TEE.Fail. 2024.
  *Grounds: Defense in depth — SGX/TDX attestation broken for under $1,000 via physical side-channel. TEE is one layer of defense, not sole defense. Grounds Roko's multi-layer security approach: TEE + content addressing + provenance + decay.*

---

## Content Provenance

- C2PA (Content Provenance and Authenticity). Coalition for Content Provenance and Authenticity. c2pa.org.
  *Grounds: Forensic AI — content provenance standard for tracking the origin and modification history of digital content. Grounds the Attestation field on Engrams: cryptographic proof of origin for every piece of agent-generated content.*

- W3C. Decentralized Identifiers (DIDs) v1.0. W3C Recommendation, 2022.
  *Grounds: Agent identity — decentralized identifier standard. Informs the ERC-8004 agent identity design for on-chain agent identification.*

---

## Capability-Based Security

- Dennis, J.B. & Van Horn, E.C. (1966). Programming Semantics for Multiprogrammed Computations. _Communications of the ACM_, 9(3), 143-155.
  *Grounds: Capability model — foundational work on capability-based access control. The concept that authority should be carried as unforgeable tokens rather than checked against access control lists. Grounds Roko's tool permission model.*

---

## Agent-Specific Security Benchmarks

- Chen, J. et al. (2025). AgentGuard: Repurposing Agentic Orchestrator for Safety Evaluation. arXiv:2502.xxxxx.
  *Grounds: Safety evaluation — repurposing orchestration for systematic safety testing of agent tool use.*

- Liu, Z. et al. (2025). AgentBound: Secure and Verifiable MCP Tool Binding for AI Agents. arXiv:2503.xxxxx.
  *Grounds: Secure tool binding — verifiable binding between agents and MCP tools prevents tool substitution attacks.*

- Rodriguez, A. et al. (2025). MCP-Guard: A Benchmark for Detecting Prompt Injection in MCP Tool Outputs. arXiv:2503.xxxxx.
  *Grounds: MCP prompt injection — benchmark for detecting prompt injection specifically in MCP tool outputs. Validates the need for output sanitization in Roko's MCP client.*

---

## Safe Reinforcement Learning

- Berkenkamp, F. et al. (2017). Safe Model-based Reinforcement Learning with Stability Guarantees. _NeurIPS_, 2017. arXiv:1705.08551.
  *Grounds: Safe exploration — safe RL with Lyapunov stability guarantees. Provides theoretical foundation for safe exploration in Roko's learning systems.*

- Schulman, J. et al. (2015). Trust Region Policy Optimization. _ICML_, 2015. arXiv:1502.05477.
  *Grounds: Constrained optimization — trust region methods constrain policy updates to prevent catastrophic changes. Grounds conservative strategy updates in Roko's learning loop.*

- Alshiekh, M. et al. (2018). Safe Reinforcement Learning via Shielding. _AAAI_, 2018. arXiv:1708.08611.
  *Grounds: Safety shields — runtime shields that override unsafe RL actions. Analogous to Roko's Gate pipeline that can reject agent outputs.*

---

## Cross-references

- See [19-regulatory-compliance.md](./19-regulatory-compliance.md) for EU AI Act, SEC/CFTC, and compliance frameworks
- See [22-protocol-standards.md](./22-protocol-standards.md) for ERC-8004 agent identity
- See topic [03-harness](../04-verification/INDEX.md) for full Harness layer design
