# Collective Intelligence

> Academic foundations for group intelligence, C-Factor measurement, superlinear scaling, and turn-taking equality in Roko's Collective system.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md), [Agent Mesh](../13-coordination/INDEX.md)
**Key sources**: `bardo-backup/prd/shared/citations.md`, `refactoring-prd/09-innovations.md` §VI

---

## Abstract

Roko's C-Factor metric — `Collective Performance / Sum(Individual Performances)` — measures whether a group of agents exhibits superlinear intelligence (C-Factor > 1.0). The research here establishes that collective intelligence is real and measurable (Woolley et al. 2010), that it depends more on social sensitivity and turn-taking equality than on individual ability, and that network effects can produce superlinear value scaling (Metcalfe, Reed).

---

## Collective Intelligence Factor

- Woolley, A.W., Chabris, C.F., Pentland, A., Hashmi, N., & Malone, T.W. (2010). Evidence for a Collective Intelligence Factor in the Performance of Human Groups. _Science_, 330(6004), 686-688.
  *Grounds: C-Factor — established that groups have a measurable collective intelligence factor (c) analogous to individual IQ (g). The c factor correlates more with social sensitivity, equality of turn-taking, and proportion of women than with average or maximum individual intelligence. Roko's C-Factor metric is directly inspired by this work. Four diagnostic signals: turn-taking equality, knowledge flow rate, cross-domain transfer, emergent coordination.*

---

## Network Effects and Scaling

- Metcalfe, R. (1995). Metcalfe's Law. As described in various subsequent analyses.
  *Grounds: Superlinear knowledge scaling — Metcalfe's Law: network value grows O(N²) with N nodes. N agents produce O(N) insights, but total network value grows O(N²) because each agent benefits from all others' contributions. Grounds the network flywheel hypothesis.*

- Reed, D.P. (1999). That Sneaky Exponential — Beyond Metcalfe's Law to the Power of Community Building.
  *Grounds: Group-forming networks — Reed's Law: group-forming networks grow as 2^N. More conservative than Metcalfe but still superlinear. Informs the value of permissioned subnets where agents form working groups.*

---

## Collective Calibration

- Central Limit Theorem (classical). Error in estimating a population mean scales as 1/sqrt(n) for independent observations.
  *Grounds: 31.6× calibration heuristic — at N=1,000 agents: sqrt(1000) = 31.6× faster calibration (theoretical upper bound under independence assumption). This is a Nunchi-derived scaling heuristic inspired by CLT, not a published theorem. Actual speedup will be less due to correlation, distribution shift, and coordination overhead.*

---

## Swarm Intelligence

- Holland, J.H. (1995). _Hidden Order: How Adaptation Builds Complexity_. Addison-Wesley.
  *Grounds: Complex adaptive systems — adaptation builds complexity through simple rules. Grounds the emergence of collective intelligence from simple agent behaviors (emit pheromones, decay knowledge, confirm findings).*

---

## Knowledge Flow and Information Economics

- Hayek, F.A. (1945). The Use of Knowledge in Society. _American Economic Review_, 35(4), 519-530.
  *Grounds: Distributed knowledge — knowledge is distributed and cannot be centralized. The price system aggregates dispersed information. Roko's pheromone field and KORAI token serve analogous aggregation functions.*

- Arrow, K.J. (1962). Economic Welfare and the Allocation of Resources for Invention. _NBER_.
  *Grounds: Information as public good — information has public good properties (non-rivalrous, partially excludable). Grounds the knowledge sharing economics in Agent Mesh.*

---

## Representation Engineering

- Turner, A. et al. (2024). Activation Addition: Steering Language Models Without Optimization. arXiv:2308.10248.
  *Grounds: Steering vectors — activation addition steers model behavior without fine-tuning. Potential mechanism for collective knowledge injection into agent behavior.*

- Zou, A. et al. (2023). Representation Engineering: A Top-Down Approach to AI Transparency. arXiv:2310.01405.
  *Grounds: Representation control — top-down approach to understanding and controlling LLM representations. Informs transparency mechanisms in Roko's observability layer.*

---

## Thousand Brains Theory

- Hawkins, J., Ahmad, S., & Cui, Y. (2017). A Theory of How Columns in the Neocortex Enable Learning the Structure of the World. _Frontiers in Neural Circuits_.
  *Grounds: Distributed consensus — each cortical column learns complete predictive models. Multiple columns vote on perception. Grounds the multi-agent voting mechanism where agents share estimates and reach consensus.*

- Clay, V., Leadholm, P., & Hawkins, J. (2024). The Thousand Brains Project. arXiv, 2024.
  *Grounds: Monty implementation — first practical implementation of Thousand Brains Theory with learning modules, cortical messaging protocol, and explicit voting mechanism.*

---

## Cross-references

- See [04-coordination-and-multi-agent.md](./04-coordination-and-multi-agent.md) for stigmergic coordination
- See [05-biological-analogues.md](./05-biological-analogues.md) for superorganism theory
- See [15-cybernetics-and-vsm.md](./15-cybernetics-and-vsm.md) for cybernetic coordination
