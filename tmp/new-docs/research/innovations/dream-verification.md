# Dream Verification

> Check REM imagination scenarios against formal invariants before staging hypotheses, turning safety violations into AntiKnowledge entries rather than undetected risks.

**Kind**: Innovation
**Status**: Speculative
**Source domain**: Formal methods, safe reinforcement learning, AI planning
**Affects subsystems**: Dreams (REM phase), Gate, Neuro (AntiKnowledge), Safety
**Last reviewed**: 2026-04-19

---

## The idea

Roko's Dreams subsystem already generates counterfactual scenarios during REM imagination: "What if the agent had used a different strategy?" "What happens at the boundary of this heuristic?" Separately, the safety system has a formal verification pipeline (Slither, Echidna, hevm, Certora) for chain-domain smart contracts.

The gap: **no verification occurs during imagination**. REM generates novel strategies and hypotheses but they are evaluated only by LLM reasoning (Sonnet-class model). A dream could produce a "brilliant" optimisation that violates a safety constraint — and this wouldn't be caught until the strategy is executed and gates fire.

The solution: during REM imagination, each counterfactual scenario generates **verification conditions** (VCs) that must hold if the imagined strategy is correct. These VCs are checked against the agent's invariant set before the hypothesis enters the staging buffer. Dreams that violate invariants are not discarded — they become **AntiKnowledge** entries with high confidence, permanently preventing the agent from ever attempting the unsafe strategy.

Six invariant types are defined:
- **TypeInvariant** — function always returns expected type
- **RangeInvariant** — value stays within bounds
- **OrderInvariant** — temporal ordering constraints (CTL: AG(A → AF B))
- **MutexInvariant** — operations never concurrent
- **MonotonicInvariant** — metric never decreases after operation
- **BudgetInvariant** — cumulative cost ≤ budget after sequence

Dream quality score = (verified_count / total_count) × novelty_score. Verified dreams are staged normally; violated dreams produce AntiKnowledge; inconclusive dreams are staged at 50% confidence.

## Origin

- **Hao, Guan et al. (2024)** "SafeDreamer: Safe Reinforcement Learning with World Models," ICLR 2024, arXiv:2307.07176. Integrates Lagrangian safety constraints into Dreamer world model imagination rollouts, verifying safety conditions *during* planning. Achieves near-zero constraint violations.
- **Lee et al. (2025)** "VeriPlan: Integrating Formal Verification and LLMs into End-User Planning," CHI 2025, arXiv:2502.17898. Applies model checking to LLM-generated plans using temporal logic constraints.
- **Hao, Chen, Zhang & Fan (2024)** "Large Language Models Can Solve Real-World Planning Rigorously with Formal Verification Tools," NAACL 2025, arXiv:2404.11891. Formalises planning as constrained satisfiability with SAT solver verification.
- **Dijkstra (1976)** Weakest precondition calculus — foundational method for generating verification conditions from programs.

## Application to Roko

Seven integration steps are specified:

1. Define `Invariant` and `InvariantKind` types in `roko-gate/src/invariant.rs`.
2. Load invariants from `roko.toml` `[invariants]` section, per domain.
3. Learn invariants from gate history via `roko-learn` pattern mining (successful patterns → Range/Order invariants).
4. Wire `VerifiedDreamRunner` into dream cycle replacing plain REM with verified REM.
5. Create AntiKnowledge in `roko-neuro` on critical violations.
6. Log dream quality metrics to `.roko/learn/dream-quality.jsonl`.
7. Learn type invariants from `roko-index` symbol type signatures.

## Estimated impact

Source states: "Verification adds <50ms overhead per scenario (for <20 invariants)." "Dream quality score correlates with staging-to-promotion rate (r > 0.5)" (test criterion). "AntiKnowledge created for critical violations prevents re-proposal in future dreams."

## Prerequisites

- Formal invariant specification language and loader in `roko.toml`.
- Pattern mining from gate history to auto-learn invariants.
- Weakest precondition engine (or lightweight symbolic execution).
- Integration with `roko-gate` invariant infrastructure.

## Status

Speculative — idea only; no formal evaluation. Ranked **P3** in the source implementation priority table (largest effort category).

## Risks and objections

- Weakest precondition computation for arbitrary LLM-generated strategies is undecidable in general; the implementation necessarily approximates.
- Invariant specification is a manual burden; if agents are deployed in novel domains, invariants may be missing.
- False positives in verification could suppress genuinely good dreams, reducing learning signal.
- 50ms overhead per scenario may add up significantly if REM generates many scenarios per cycle.

## Related innovations

- [dream-token-economy](./dream-token-economy.md) — verification results determine dream quality grade, which drives budget allocation
- [affect-causal-discovery](./affect-causal-discovery.md) — REM counterfactuals and causal counterfactuals share the episode substrate
- [hdc-active-inference](./hdc-active-inference.md) — Delta consolidation updates μ_prior based on verified dreams
- [witness-world-model](./witness-world-model.md) — Witness DAG can source invariants from historical causal chains

## References

- Hao, Guan et al. (2024). SafeDreamer: Safe Reinforcement Learning with World Models. ICLR 2024, arXiv:2307.07176.
- Lee et al. (2025). VeriPlan: Integrating Formal Verification and LLMs into End-User Planning. CHI 2025, arXiv:2502.17898.
- Hao, Chen, Zhang & Fan (2024). Large Language Models Can Solve Real-World Planning Rigorously with Formal Verification Tools. NAACL 2025, arXiv:2404.11891.
- Dijkstra (1976). A Discipline of Programming. Prentice-Hall.
- Boden (1990). The Creative Mind: Myths and Mechanisms. (Three creativity modes referenced in Dreams subsystem).
