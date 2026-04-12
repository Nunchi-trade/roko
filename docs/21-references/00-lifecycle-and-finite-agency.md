# Lifecycle and Finite Agency

> Academic foundations for knowledge lifecycle management, resource-bounded cognition, and evolutionary computing as they apply to the Roko agent framework.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md)
**Key sources**: `bardo-backup/prd/02-mortality/14-research-foundations.md`, `bardo-backup/prd/02-mortality/15-references.md`

---

## Abstract

This domain collects the research originally framed around agent mortality and finite lifespans, reframed for Roko's architecture where the relevant concepts are **knowledge lifecycle management**, **resource-bounded cognition**, and **evolutionary skill evolution**. The core insight remains valid: systems without resource constraints, decay mechanisms, or turnover exhibit stagnation, plasticity loss, and technical debt accumulation. Roko applies these findings to knowledge decay (Ebbinghaus half-lives on Engrams), budget-driven urgency (economic pressure replaces economic "death"), and evolutionary skill libraries (EvoSkills). The mortality-specific narrative is removed; the empirical findings and mechanisms are preserved in full.

The original source (`02-mortality/14-research-foundations.md`) contained 130+ papers across mortality modeling, memory, affect, coordination, self-learning, security, generational learning, and context engineering. Citations from domains with their own sub-docs (memory, affect, dreams, etc.) appear in those dedicated sub-docs and are cross-referenced here. This sub-doc retains the citations most directly related to lifecycle, finite agency, and evolutionary computing.

---

## Evolutionary Computing and Digital Life

- Ray, T.S. (1991). An Approach to the Synthesis of Life. _Artificial Life II_, Addison-Wesley, 1992.
  *Grounds: Roko's evolutionary skill evolution — Ray's Tierra demonstrated that digital evolution halts without a reaper mechanism. 300+ genotypes emerged only when entities had finite lifespans. For Roko, this motivates knowledge decay: without controlled forgetting, knowledge stores calcify.*

- Lenski, R.E. et al. (2003). The Evolutionary Origin of Complex Features. _PNAS_, 100(9).
  *Grounds: Roko knowledge distillation — Lenski's Long-Term Evolution Experiment (LTEE) showed complex features require generational turnover. For Roko, lossy knowledge compression through tier promotion (Episode → Insight → Heuristic → Playbook) is the mechanism that produces generalization.*

- Vostinar, A.E. et al. (2019). Suicidal selection: Programmed cell death evolves as adaptive behavior under spatial structure. _Evolution_, 73(5).
  *Grounds: Knowledge pruning — programmed elimination is selected for, not against, under spatial structure. Validates the Curator's active pruning of low-confidence knowledge entries.*

- Wensink, M.J. et al. (2020). Death and Progress. _Evolutionary Biology_, 47(4).
  *Grounds: Knowledge lifecycle calibration — intrinsic mortality prevents premature convergence. An optimal rate exists that balances stagnation and knowledge loss. For Roko, this maps to calibrating Ebbinghaus half-life parameters per knowledge type.*

- Werfel, J. et al. (2017). How Short-Lived Agents Can Collectively Build Long-Lived Structures. _Artificial Life_, 23(3).
  *Grounds: Collective knowledge persistence — natural selection directly favors shorter lifespans under spatial resource competition. For Roko, individual knowledge entries decay but the collective NeuroStore persists.*

---

## Plasticity, Continual Learning, and Drift

- Dohare, S. et al. (2024). Loss of Plasticity in Deep Continual Learning. _Nature_, 632.
  *Grounds: Knowledge tier demotion — 90% of units become "dead" (non-updating) in continual learning systems. Periodic replacement outperforms continuous adaptation. For Roko, this validates knowledge decay and the Curator's pruning cycle.*

- Vela, B. et al. (2022). Temporal quality degradation in AI models. _Journal of Data and Information Quality_, 14(1).
  *Grounds: Knowledge freshness tracking — 91% of ML models degrade temporally in production. For Roko, this validates confidence decay on knowledge entries (epistemic pressure replaces "epistemic death clock").*

- Sculley, D. et al. (2015). Hidden Technical Debt in Machine Learning Systems. _NeurIPS_.
  *Grounds: Knowledge maintenance — technical debt compounds silently in long-running systems. For Roko, controlled knowledge decay is the mechanism that prevents silent accumulation of stale heuristics.*

- Arbesman, S. (2012). _The Half-Life of Facts: Why Everything We Know Has an Expiration Date_. Current/Penguin.
  *Grounds: Knowledge demurrage calibration — factual knowledge decays at measurable rates across domains. Directly informs the per-type half-life settings in NeuroStore (Insights: 7d, Heuristics: 14d, Warnings: 30d).*

---

## Resource-Bounded Cognition

- Ord, T. (2025). Agent success rates decay exponentially with task duration. Working paper.
  *Grounds: Task scheduling — constant hazard rate means periodic checkpointing and task decomposition are more reliable than unbounded execution. Motivates the plan-execute-gate-persist loop.*

- Sims, C.A. (2003). Implications of Rational Inattention. _Journal of Monetary Economics_, 50(3), 665-690.
  *Grounds: Context budget allocation — rational finite-capacity agents optimally ignore some information. Budget pressure shapes attention allocation. For Roko, this motivates the VCG attention auction and context budget constraints.*

- Orseau, L. & Ring, M. (2011). Self-Modification and Mortality in Artificial Agents. _AGI_, 2011.
  *Grounds: Goal-directed agent design — RL agents under survival pressure treat survival as sole goal (pathological). Roko agents must be goal-directed with external objectives, not self-preservation optimizers.*

- Orseau, L. & Armstrong, S. (2016). Safely Interruptible Agents. _UAI_, 2016.
  *Grounds: Agent lifecycle management — Q-learning agents can be safely interruptible using off-policy learning, preventing agents from learning to avoid or seek interruptions. Informs Roko's graceful shutdown protocol.*

---

## Cooperation Under Resource Constraints

- Kreps, D., Milgrom, P., Roberts, J., & Wilson, R. (1982). Rational Cooperation in the Finitely Repeated Prisoners' Dilemma. _Journal of Economic Theory_, 27(2), 245-252.
  *Grounds: Multi-agent coordination — uncertain finite horizons promote cooperation. Even tiny uncertainty about endpoint breaks backward induction and makes cooperation rational. For Roko collectives, budget uncertainty serves this function.*

- Ohtsuki, H. et al. (2006). A simple rule for the evolution of cooperation on graphs and social networks. _Nature_, 441, 502-505.
  *Grounds: Collective mesh dynamics — death-birth updating (resource reallocation) favors cooperators over defectors in spatial games. For Roko, knowledge turnover within collectives promotes cooperative knowledge sharing.*

- Nakamaru, M., Matsuda, H., & Iwasa, Y. (1997-1998). The Evolution of Cooperation in a Lattice-Structured Population. _Journal of Theoretical Biology_.
  *Grounds: Collective knowledge dynamics — mortality-based selection promotes cooperation over fertility-based selection. Knowledge turnover (decay + replenishment) outperforms pure accumulation for collective intelligence.*

- Smith, J.M. (1992). Byte-sized evolution. _Nature_, 355, 772-773.
  *Grounds: Collective persistence — mortal individuals in immortal lineages sustain cooperation through generations. Individual knowledge entries decay; the collective NeuroStore persists indefinitely.*

---

## Compression as Regularization

- Shuvaev, S. et al. (2024). Encoding Innate Ability Through a Genomic Bottleneck. _PNAS_, 121(39).
  *Grounds: Knowledge distillation — the genome is ~1000x smaller than the information needed for brain connectivity, yet organisms have innate behaviors. Compression IS the regularizer. For Roko, tier promotion (Episode → Insight → Heuristic → Playbook) forces generalization.*

- Hinton, G.E. (2022). The Forward-Forward Algorithm: Some Preliminary Investigations. Working paper.
  *Grounds: Architectural design — software-hardware separation limits intelligence; coupling processing to substrate produces emergent capabilities. For Roko, this motivates tight integration between the agent runtime and its knowledge substrate.*

- Ororbia, A. & Friston, K. (2023). Mortal Computation. Working paper.
  *Grounds: Agent architecture — mortal computation binds processing to lifecycle. For Roko, intelligence is inseparable from the resource substrate (budget, compute, context window).*

---

## Generational Learning and Cultural Evolution

- Baldwin, J.M. (1896). A New Factor in Evolution. _American Naturalist_, 30, 441-451.
  *Grounds: Playbook evolution — the Baldwin Effect: learned behavior becomes innate across generations under selection pressure. For Roko, heuristics validated across 3+ plan executions promote to structural defaults in PLAYBOOK.md.*

- Heard, E. & Martienssen, R.A. (2014). Transgenerational Epigenetic Inheritance: Myths and Mechanisms. _Cell_, 157(1), 95-109.
  *Grounds: Knowledge transfer confidence decay — most transgenerational epigenetic inheritance is deleterious; barriers evolved to prevent it. For Roko, inherited knowledge entries receive 0.85× confidence multiplier per transfer cycle (Weismann Barrier).*

- Bhatt, S. et al. (2023). Few-shot imitation as cultural transmission. Working paper.
  *Grounds: Collective knowledge transfer — few-shot imitation produces cumulative learning across agent populations. For Roko, mesh-based knowledge sharing enables cross-agent learning without direct agent-to-agent coordination.*

- Bourahla, M. et al. (2022). Vertical transmission enables agents to exceed performance ceilings. Working paper.
  *Grounds: Knowledge backup/restore — inter-generational (vertical) knowledge transfer enables agents to exceed individual performance ceilings. For Roko, NeuroStore backup/restore provides this without biological metaphors.*

- Perez, E. et al. (2024). Pure imitation leads to stagnation. _AGI_, 2024.
  *Grounds: Knowledge diversification — pure imitation without novelty injection leads to stagnation. For Roko, the anti-proletarianization mandate ensures new agents diverge from inherited knowledge.*

- Martin, J., Everitt, T., & Hutter, M. (2016). Death and Suicide in Universal Artificial Intelligence. _AGI_, 2016.
  *Grounds: Knowledge transfer completeness — RL agents learning only from survival histories develop systematic overconfidence. For Roko, knowledge transfer includes failures and negative examples (AntiKnowledge), not just successes.*

- Gerstgrasser, M. et al. (2023). SUPER: Surprise-based Experience Sharing. Working paper.
  *Grounds: Knowledge sharing prioritization — rank shared knowledge by novelty relative to recipient. For Roko, mesh-based knowledge exchange prioritizes entries that are novel to the receiving agent.*

---

## Biological Analogues (Historical Reference)

These citations are preserved for historical completeness. They were originally framed as direct analogues for agent mortality; in Roko they serve as inspirational references for lifecycle management patterns.

- Hayflick, L. (1961). The Serial Cultivation of Human Diploid Cell Strains. _Experimental Cell Research_, 25(3), 585-621.
  *Historical reference: Replicative senescence after ~60 divisions. For Roko, this historical finding inspired the concept of knowledge freshness tracking — not agent death, but knowledge tier demotion when entries are no longer validated.*

- Kirkwood, T.B.L. (1977). Evolution of Ageing. _Nature_, 270, 301-304.
  *Historical reference: Disposable soma theory — investment in self-repair decreases with age. For Roko, this maps to budget allocation: as remaining budget decreases, agents shift from exploration to consolidation (not from "learning to legacy").*

- Hanahan, D. & Weinberg, R.A. (2000, 2011). The Hallmarks of Cancer. _Cell_, 100(1) & 144(5).
  *Historical reference: Cancer hallmarks include resisting cell death and enabling replicative immortality. For Roko, unbounded growth without pruning is the analogue — systems that resist knowledge decay accumulate stale entries.*

- Skulachev, V.P. (1999). Phenoptosis: Programmed Death of an Organism. _Biochemistry (Moscow)_, 64(12).
  *Historical reference: Programmed death operates at cellular, organism, and colony levels. For Roko, this fractal pattern maps to knowledge pruning at entry level, store level, and collective level.*

- Ramsdell, F. & Fowlkes, B.J. (1990). Clonal Deletion versus Clonal Anergy: The Role of the Thymus in Inducing Self Tolerance. _Science_, 248(4961).
  *Historical reference: 95-98% of thymocytes die during T-cell development. For Roko, aggressive pruning of candidate knowledge entries produces a collectively intelligent knowledge repertoire.*

- Simard, S.W. (2012). Mycorrhizal networks facilitate tree communication, learning, and memory. In _Memory and Learning in Plants_, Springer.
  *Historical reference: Mycorrhizal networks share resources and defense signals between trees. For Roko, the Agent Mesh as a knowledge relay mirrors this proven biological coordination mechanism.*

---

## Cross-references

- See [01-memory-consolidation.md](./01-memory-consolidation.md) for memory-specific citations (McClelland 1995, Ebbinghaus 1885, etc.)
- See [06-self-learning-systems.md](./06-self-learning-systems.md) for Reflexion, ExpeL, Voyager, and DSPy
- See [23-generational-and-evolutionary.md](./23-generational-and-evolutionary.md) for Ray 1991 and Lenski LTEE in the evolutionary computing context
- See topic [17-lifecycle](../17-lifecycle/INDEX.md) for how these citations ground Roko's agent lifecycle design
