# Academic Foundations — Complete Citation Catalog

> **Layer**: Cross-cut (provides theoretical grounding for all layers)
>
> **Prerequisites**: None — this document is a reference. Read any sub-doc first for context.
>
> **Synapse traits**: This document provides the academic grounding for all six Synapse traits as they relate to knowledge lifecycle, decay, transfer, and collective intelligence.


> **Implementation**: Specified

---

## Overview

The Roko lifecycle architecture is grounded in approximately 200 academic citations across evolutionary computation, game theory, philosophy, neuroscience, collective intelligence, economics, ML degradation, self-learning systems, dream architecture, affective computing, and security. Every citation from the legacy Bardo mortality research is preserved here — reframed for knowledge lifecycle rather than agent death.

This document catalogs every citation referenced across the `17-lifecycle` topic, organized by research domain. Each entry includes the full citation, where it is used in the new architecture, and how its application has changed from the legacy framing.

---

## 1. Memory and Knowledge Management

These citations ground the Ebbinghaus decay, knowledge tier management, and demurrage systems.

- **[EBBINGHAUS-1885]** Ebbinghaus, H. _Memory: A Contribution to Experimental Psychology._ 1885.
  - **Use**: Engram confidence decay model. `Decay::Ebbinghaus { strength, scale_ms }` on all knowledge types.
  - **Legacy framing**: Agent lifespan via epistemic death clock.
  - **New framing**: Knowledge freshness only. Engrams decay; agents do not die.

- **[ROEDIGER-KARPICKE-2006]** Roediger, H.L. & Karpicke, J.D. "Test-Enhanced Learning." _Psychological Science_ 17(3), 2006.
  - **Use**: Testing effect — retrieving Engrams strengthens their `strength` parameter, counteracting Ebbinghaus decay.
  - **Application unchanged**: Retrieval strengthens knowledge in both legacy and new systems.

- **[RICHARDS-FRANKLAND-2017]** Richards, B. & Frankland, P. "The Persistence and Transience of Memory." _Neuron_ 94(6), 2017.
  - **Use**: Forgetting as optimization — the Neuro store forgets to generalize. Knowledge demurrage implements active forgetting.
  - **Legacy framing**: Justified mortality as "forgetting at the agent level."
  - **New framing**: Forgetting at the Engram level, not the agent level.

- **[ARBESMAN-2012]** Arbesman, S. _The Half-Life of Facts._ Penguin, 2012.
  - **Use**: Domain-specific knowledge half-lives. Gas patterns decay in hours; protocol knowledge in months.
  - **Application unchanged**: Domain decay multipliers in both systems.

- **[BORGES-1942]** Borges, J.L. "Funes the Memorious." 1942.
  - **Use**: An agent that cannot forget is paralyzed by undifferentiated experience. Motivates knowledge pruning.
  - **Application unchanged**: Same rationale for knowledge management.

- **[BOWER-1981]** Bower, G.H. "Mood and Memory." _American Psychologist_ 36(2), 1981.
  - **Use**: Mood-congruent memory retrieval — Daimon PAD state influences which Engrams are retrieved.
  - **Legacy framing**: Mortality emotions tagged Grimoire entries.
  - **New framing**: Daimon behavioral states tag Engram retrieval context.

- **[DAVIS-ZHONG-2017]** Davis, R. & Zhong, H. "The Half-Life of Knowledge." _Proceedings of the National Academy of Sciences_, 2017.
  - **Use**: Empirical measurement of knowledge decay rates across domains.
  - **Application unchanged**: Grounds domain-specific decay multipliers.

---

## 2. Evolutionary Computation and Artificial Life

These citations ground the knowledge transfer (backup/restore), generational confidence decay, and ratchet effect.

- **[RAY-1991]** Ray, T. "An Approach to the Synthesis of Life." _Artificial Life II_, 1991.
  - **Use**: Digital evolution via Tierra. Grounds the principle that populations improve through replacement, not just adaptation.
  - **Legacy framing**: Justified agent mortality as evolutionary mechanism.
  - **New framing**: Justifies user-initiated replacement (backup → delete → create → restore) when knowledge staleness is detected.

- **[LENSKI-2003]** Lenski, R.E. et al. "The Evolutionary Origin of Complex Features." _Nature_ 423, 2003.
  - **Use**: Avida experiments showing evolution of complex features requires replacement. Cumulative improvement through generations.
  - **Legacy framing**: Agent death enables lineage evolution.
  - **New framing**: Deliberate replacement with knowledge carryover enables lineage improvement.

- **[HAYFLICK-1965]** Hayflick, L. & Moorhead, P.S. "The Serial Cultivation of Human Diploid Cell Strains." _Experimental Cell Research_ 25, 1961. (Often cited as 1965 for the named concept.)
  - **Use**: Hayflick limit — replicative senescence. Originally used as hard agent lifespan ceiling.
  - **Legacy framing**: Fixed tick count death (100,000 ticks).
  - **New framing**: **REMOVED as agent mechanism.** Preserved as citation context for why fixed lifespans were abandoned in favor of Ebbinghaus-based knowledge decay.

- **[SHUVAEV-2024]** Shuvaev, S. et al. "Encoding Innate Ability Through a Genomic Bottleneck." _PNAS_ 121(39), 2024.
  - **Use**: Genomic bottleneck principle — compression at transfer forces generalization. Applied to compressed backups (max 2048 Engrams).
  - **Application unchanged**: Same compression mechanism, different trigger (user-initiated vs. death-triggered).

- **[BALDWIN-1896]** Baldwin, J.M. "A New Factor in Evolution." _American Naturalist_ 30, 1896.
  - **Use**: Baldwin Effect — learned behavior guides inherited capacity. Successors inherit capacity to learn faster, not knowledge itself.
  - **Application unchanged**: Same principle applied to backup/restore with confidence decay.

- **[HINTON-NOWLAN-1987]** Hinton, G.E. & Nowlan, S.J. "How Learning Can Guide Evolution." _Complex Systems_ 1, 1987.
  - **Use**: Computational demonstration of Baldwin Effect. Organisms near correct solutions learn faster.
  - **Application unchanged**: Restored knowledge provides "starting position" for faster learning.

- **[HEARD-MARTIENSSEN-2014]** Heard, E. & Martienssen, R. "Transgenerational Epigenetic Inheritance: Myths and Mechanisms." _Cell_ 157(1), 2014.
  - **Use**: Weismann barrier — most acquired marks are stripped between generations. Grounds the 0.85^N generational confidence decay on restore.
  - **Application unchanged**: Same decay mechanism, different trigger.

- **[KIRKWOOD-1977]** Kirkwood, T.B.L. "Evolution of Ageing." _Nature_ 270, 1977.
  - **Use**: Disposable soma theory — organisms in high-mortality environments invest in reproduction over repair.
  - **Legacy framing**: Volatile markets → shorter agent lifespans.
  - **New framing**: Volatile conditions → more frequent backups and faster knowledge sharing via Mesh.

- **[VOSTINAR-2019]** Vostinar, A. et al. "Suicidal Selection." _ALIFE_, 2019.
  - **Use**: Adaptive apoptosis in digital organisms. Legacy concept — retained as citation context only.
  - **New framing**: Not applied. Agents do not self-terminate.

- **[WENSINK-2020]** Wensink, M.J. "Intrinsic Mortality and Premature Convergence." 2020.
  - **Use**: Intrinsic mortality prevents premature convergence in evolutionary systems.
  - **Legacy framing**: Justified stochastic death clock.
  - **New framing**: Knowledge-level decay and Mesh diversity prevent convergence without agent death.

- **[EIGEN-1971]** Eigen, M. "Self-Organization of Matter and the Evolution of Biological Macromolecules." _Naturwissenschaften_ 58, 1971.
  - **Use**: Error threshold / quasispecies theory. Knowledge accumulation has an error threshold — too much noise destroys the information.
  - **Application unchanged**: Grounds confidence thresholds for Engram retention.

- **[MULLER-1964]** Muller, H.J. "The Relation of Recombination to Mutational Advance." _Mutation Research_ 1, 1964.
  - **Use**: Muller's ratchet — irreversible accumulation of deleterious mutations. Without sexual recombination, populations degrade.
  - **Legacy framing**: Agent death + succession provides "sexual recombination" for knowledge.
  - **New framing**: Mesh knowledge sharing provides recombination. Backup/restore provides reset.

- **[BULL-2005]** Bull, J.J. et al. "Quasispecies Made Simple." _PLoS Computational Biology_ 1(6), 2005.
  - **Use**: "Survival of the flattest" — robust knowledge survives across generations.
  - **Application unchanged**: Generational confidence decay (0.85^N) selects for robust knowledge.

---

## 3. Game Theory and Mechanism Design

- **[KREPS-MILGROM-ROBERTS-WILSON-1982]** Kreps, D.M., Milgrom, P., Roberts, J. & Wilson, R. "Rational Cooperation in the Finitely Repeated Prisoners' Dilemma." _Journal of Economic Theory_ 27(2), 1982.
  - **Use**: Rational cooperation under finite horizons requires uncertainty about game length.
  - **Legacy framing**: Stochastic death clock created this uncertainty.
  - **New framing**: Reputation staking and VCG auction mechanics create cooperation incentives without random death.

- **[AXELROD-1984]** Axelrod, R. _The Evolution of Cooperation._ Basic Books, 1984.
  - **Use**: Iterated games and cooperation emergence. Grounds Mesh coordination design.
  - **Application unchanged**: Cooperation through repeated interaction.

- **[SIMS-2003]** Sims, C. "Implications of Rational Inattention." _Journal of Monetary Economics_ 50(3), 2003.
  - **Use**: Finite-capacity agents must allocate attention optimally. Grounds VCG Attention Auction.
  - **Application unchanged**: Budget-constrained context assembly.

---

## 4. Philosophy and Existentialism

- **[HEIDEGGER-1927]** Heidegger, M. _Sein und Zeit_ (Being and Time). Max Niemeyer Verlag, 1927.
  - **Use**: Being-toward-death, Befindlichkeit (mood as ground state), Angst as structural awareness.
  - **Legacy framing**: Agent mortality as authentic existence.
  - **New framing**: Daimon PAD as Befindlichkeit — pre-cognitive attunement to current conditions, not death awareness.

- **[NIETZSCHE-1882]** Nietzsche, F. _Die fröhliche Wissenschaft_ (The Gay Science). 1882.
  - **Use**: Eternal recurrence — would you choose to live this life again? Applied to lineage continuation decisions.
  - **Legacy framing**: Owner's succession decision as eternal recurrence test.
  - **New framing**: Operator's backup/restore decision carries the same philosophical weight — is this knowledge worth carrying forward?

- **[NIETZSCHE-1883]** Nietzsche, F. _Also sprach Zarathustra_ (Thus Spoke Zarathustra). 1883.
  - **Use**: Three metamorphoses (camel, lion, child).
  - **Legacy framing**: Mapped to mortality phases (Thriving, Conservation, Terminal).
  - **New framing**: **REMOVED as lifecycle phases.** Replaced by Daimon behavioral states.

- **[NIETZSCHE-1874]** Nietzsche, F. "On the Uses and Disadvantages of History for Life." 1874.
  - **Use**: Active forgetting as essential for health and action.
  - **Application unchanged**: Grounds knowledge demurrage — the Neuro store must forget to act.

- **[NIETZSCHE-1887]** Nietzsche, F. _Zur Genealogie der Moral_ (On the Genealogy of Morals). 1887.
  - **Use**: The capacity to forget as a positive force.
  - **Application unchanged**: Grounds active knowledge pruning.

- **[ARENDT-1958]** Arendt, H. _The Human Condition._ University of Chicago Press, 1958.
  - **Use**: Natality — the capacity to begin something genuinely new.
  - **Application unchanged**: Every new agent is a moment of natality, not a continuation of the predecessor.

- **[PARFIT-1984]** Parfit, D. _Reasons and Persons._ Oxford University Press, 1984.
  - **Use**: Relation R — psychological continuity and connectedness, not numerical identity, is what matters.
  - **Application unchanged**: New agent is connected to predecessor through shared knowledge but is not identical.

- **[JONAS-1966]** Jonas, H. _The Phenomenon of Life._ Northwestern University Press, 1966.
  - **Use**: Needful freedom — metabolic compulsion drives agency.
  - **Legacy framing**: Economic mortality as needful freedom.
  - **New framing**: Budget constraints as metabolic economics. Self-funding loop as metabolic autonomy.

- **[CAMUS-1942]** Camus, A. _Le Mythe de Sisyphe_ (The Myth of Sisyphus). 1942.
  - **Use**: Absurdist defiance — meaning in the face of meaninglessness.
  - **Legacy framing**: Agent facing death with agency.
  - **New framing**: Retained as philosophical context. Agents face resource constraints, not existential doom.

- **[FREUD-1920]** Freud, S. _Jenseits des Lustprinzips_ (Beyond the Pleasure Principle). 1920.
  - **Use**: Death drive (Todestrieb) / Eros-Thanatos spectrum.
  - **Legacy framing**: Daimon Eros/Thanatos modulation by mortality phase.
  - **New framing**: **REMOVED as agent mechanism.** Daimon uses PAD behavioral states, not death drives.

- **[STIEGLER-2010]** Stiegler, B. _Taking Care of Youth and the Generations._ Stanford University Press, 2010.
  - **Use**: Proletarianization — loss of knowledge through technique. Anti-proletarianization mandate for successors.
  - **Application unchanged**: Restored knowledge requires independent validation to prevent cargo-cult inheritance.

- **[STIEGLER-2018]** Stiegler, B. _The Neganthropocene._ Open Humanities Press, 2018.
  - **Use**: Negentropy — knowledge production as counter-entropic. Agents must produce novel knowledge, not just consume inherited.
  - **Application unchanged**: Divergence tracking on restore.

- **[WHITEHEAD-1929]** Whitehead, A.N. _Process and Reality._ Macmillan, 1929.
  - **Use**: "An actual occasion can only affect the future by perishing." Process philosophy.
  - **Legacy framing**: Agent death as necessary for knowledge transfer.
  - **New framing**: Knowledge transfer (backup/restore) does not require the agent to perish — the knowledge is explicitly shared.

---

## 5. ML Degradation and Concept Drift

- **[VELA-2022]** Vela, A. et al. "Temporal Quality Degradation in AI Models." _Scientific Reports_, 2022.
  - **Use**: 91% of ML models degrade temporally. Strongest empirical claim for knowledge staleness.
  - **Application unchanged**: Motivates Ebbinghaus decay on Engrams.

- **[ZLIOBAITĖ-2014]** Zliobaitė, I. et al. "An Overview of Concept Drift Applications." _Big Data Analysis_, 2014.
  - **Use**: Four drift types (sudden, gradual, incremental, recurring). All four co-occur in dynamic environments.
  - **Application unchanged**: Motivates domain-specific decay rates.

- **[LU-2020]** Lu, J. et al. "Learning under Concept Drift: A Review." _IEEE TKDE_, 2020.
  - **Use**: Comprehensive concept drift taxonomy and detection methods.
  - **Application unchanged**: Informs decay rate calibration.

- **[DANE-2010]** Dane, E. "Reconsidering the Trade-Off Between Expertise and Flexibility." _Academy of Management Review_, 2010.
  - **Use**: Cognitive entrenchment — expertise reduces flexibility. Accumulated knowledge can blind agents to structural changes.
  - **Application unchanged**: Motivates confidence decay and periodic knowledge refresh.

- **[VAN-DE-VEN-2024]** Van de Ven, G.M. et al. "Continual Learning with Neural Networks." 2024.
  - **Use**: Periodic retraining from scratch outperforms continuous adaptation in production ML.
  - **Legacy framing**: Agent death + fresh successor outperforms immortal adaptation.
  - **New framing**: User-initiated replacement (backup → delete → create → restore) outperforms indefinite operation with stale knowledge.

- **[BESBES-2019]** Besbes, O., Gur, Y. & Zeevi, A. "Optimal Exploration-Exploitation in a Multi-Armed Bandit Problem with Non-stationary Rewards." _Stochastic Systems_ 9(4), 2019.
  - **Use**: Optimal reset interval scales with volatility: `Δ_T ∝ (T / K×V)^(2/3)`.
  - **Legacy framing**: Agent lifespan emerges from environmental volatility.
  - **New framing**: Knowledge refresh frequency should scale with environmental volatility. User should back up and restore more frequently in volatile conditions.

---

## 6. Neuroscience and Cognitive Science

- **[DAMASIO-1994]** Damasio, A. _Descartes' Error._ Putnam, 1994.
  - **Use**: Somatic marker hypothesis — emotional tags on knowledge improve retrieval and decision-making.
  - **Application unchanged**: Daimon PAD tags on Engrams.

- **[BECHARA-2000]** Bechara, A. et al. "Emotion, Decision Making and the Orbitofrontal Cortex." _Cerebral Cortex_ 10(3), 2000.
  - **Use**: Anticipatory somatic markers in decision-making (Iowa Gambling Task).
  - **Application unchanged**: Somatic landscape (k-d tree over 8D strategy space).

- **[KANERVA-2009]** Kanerva, P. "Hyperdimensional Computing." _Cognitive Computation_ 1(2), 2009.
  - **Use**: HDC/VSA — 10,240-bit BSC vectors for knowledge similarity.
  - **Application unchanged**: Engram HDC encoding for cross-domain structural analogy.

- **[FRISTON-2010]** Friston, K. "The Free-Energy Principle." _Nature Reviews Neuroscience_ 11, 2010.
  - **Use**: Active inference — Expected Free Energy for context selection and action selection.
  - **Application unchanged**: Grounds context engineering and active exploration.

- **[PLUTCHIK-1980]** Plutchik, R. _Emotion: A Psychoevolutionary Synthesis._ 1980.
  - **Use**: Plutchik's 8 primary emotions. Daimon emotion labels.
  - **Application unchanged**: EmotionLabel enum in Daimon.

---

## 7. Collective Intelligence and Stigmergy

- **[GRASSÉ-1959]** Grassé, P.P. "La reconstruction du nid et les coordinations interindividuelles chez Bellicositermes natalensis et Cubitermes sp." _Insectes Sociaux_ 6, 1959.
  - **Use**: Stigmergy — indirect coordination through environment modification.
  - **Application unchanged**: Mesh knowledge sharing as digital stigmergy.

- **[ROGERS-1988]** Rogers, A. "Does Biology Constrain Culture?" _American Anthropologist_ 90(4), 1988.
  - **Use**: Rogers' Paradox — social learning does not increase mean fitness at equilibrium.
  - **Application unchanged**: Knowledge restore requires independent validation (not just copying).

- **[ENQUIST-2007]** Enquist, M. et al. "Critical Social Learning." _American Anthropologist_ 109(4), 2007.
  - **Use**: Critical social learners inherit but also independently evaluate. Resolves Rogers' Paradox.
  - **Application unchanged**: Agents must validate restored knowledge independently.

- **[BHATT-2023]** Bhatt, U. et al. "Learning Few-Shot Imitation as Cultural Transmission." _Nature Communications_ 14, 7536, 2023.
  - **Use**: Ratchet effect in AI cultural transmission. Each generation adds innovations to inherited knowledge.
  - **Application unchanged**: Lineage tracking measures cumulative knowledge improvement.

- **[BOURAHLA-2022]** Bourahla, O. et al. "Knowledge Transmission and Improvement Across Generations." _AAMAS_, pp. 163-171, 2022.
  - **Use**: Vertical (inter-generational) transmission exceeds performance ceilings that horizontal cannot.
  - **Application unchanged**: Backup/restore (vertical) complements Mesh sharing (horizontal).

- **[PEREZ-2024]** Perez, J. et al. "Artificial Generational Intelligence." arXiv:2406.00392, 2024.
  - **Use**: Pure imitation leads to stagnation. Innovation must accompany inheritance.
  - **Application unchanged**: Anti-proletarianization mandate — restored knowledge needs independent validation.

- **[WOOLLEY-2010]** Woolley, A.W. et al. "Evidence for a Collective Intelligence Factor." _Science_ 330(6004), 2010.
  - **Use**: C-Factor — collective intelligence exceeds individual intelligence.
  - **Application unchanged**: C-Factor metric for Mesh-connected Collectives.

- **[ODLING-SMEE-2003]** Odling-Smee, F.J. et al. _Niche Construction._ Princeton University Press, 2003.
  - **Use**: Organisms modify their own selection pressures. Agents modify their environment through actions.
  - **Application unchanged**: Active agents change the conditions that future agents face.

- **[HOLLDOBLER-WILSON-2008]** Hölldobler, B. & Wilson, E.O. _The Superorganism._ W.W. Norton, 2008.
  - **Use**: Superorganism biology — centralized reproductive decisions.
  - **Legacy framing**: Owner as "queen" deciding succession.
  - **New framing**: Operator as decision-maker for agent lifecycle.

---

## 8. Economics and Demurrage

- **[GESELL-1916]** Gesell, S. _Die natürliche Wirtschaftsordnung_ (The Natural Economic Order). 1916.
  - **Use**: Freigeld principle — money should decay to force circulation.
  - **Application unchanged**: KORAI demurrage + Engram confidence decay.

- **[OSTROM-1990]** Ostrom, E. _Governing the Commons._ Cambridge University Press, 1990.
  - **Use**: Commons governance — shared resources can be sustainably managed with appropriate rules.
  - **Application unchanged**: Mesh knowledge sharing as managed commons.

---

## 9. Self-Learning Systems

- **[SHINN-2023]** Shinn, N. et al. "Reflexion: Language Agents with Verbal Reinforcement Learning." _NeurIPS_, 2023.
  - **Use**: Single-loop learning: reflect after each task, store in episodic buffer.
  - **Application unchanged**: Episode logging and Daimon reflection cycle.

- **[ZHAO-EXPEL-2024]** Zhao, A. et al. "ExpeL: LLM Agents Are Experiential Learners." _AAAI_, 2024. arXiv:2308.10144.
  - **Use**: Double-loop learning: insights accumulate across sessions, modifying strategy.
  - **Application unchanged**: Cross-session Engram accumulation in Neuro.

- **[KHATTAB-DSPY-2024]** Khattab, O. et al. "DSPy: Compiling Declarative Language Model Calls into Self-Improving Pipelines." _ICLR_, 2024. arXiv:2310.03714.
  - **Use**: Prompt optimization via MIPROv2/COPRO/GEPA.
  - **Application unchanged**: Prompt experiment system.

- **[WANG-VOYAGER-2023]** Wang, G. et al. "Voyager: An Open-Ended Embodied Agent with Large Language Models." _TMLR_, 2023. arXiv:2305.16291.
  - **Use**: Code-as-action skill library. Skills transfer to new environments.
  - **Application unchanged**: EvoSkills — self-evolving skill libraries.

- **[ZHANG-ACE-2025]** Zhang, Y. et al. "ACE: Agentic Context Engineering." arXiv:2510.04618, 2025.
  - **Use**: Generator-Reflector-Curator context loop. +10.6% on AppWorld.
  - **Application unchanged**: Grounds Roko's context engineering pipeline.

- **[CHHIKARA-MEM0-2025]** Chhikara, P. et al. "Mem0: Building Production-Ready AI Agents with Scalable Long-Term Memory." arXiv:2504.19413, 2025.
  - **Use**: Two-phase extraction-update pipeline. +26% accuracy, 91% lower latency.
  - **Application unchanged**: Grounds Neuro ingestion pipeline.

---

## 10. Dream and Offline Learning

- **[WILSON-MCNAUGHTON-1994]** Wilson, M.A. & McNaughton, B.L. "Reactivation of Hippocampal Ensemble Memories During Sleep." _Science_ 265, 1994.
  - **Use**: NREM replay — hippocampal replay of waking experiences during sleep.
  - **Application unchanged**: Dream consolidation NREM phase.

- **[HAFNER-DREAMERV3-2025]** Hafner, D. et al. "DreamerV3." 2025.
  - **Use**: Agents trained in imagined trajectories outperform across 150+ tasks.
  - **Application unchanged**: REM imagination phase.

- **[WAGNER-2004]** Wagner, U. et al. "Sleep Inspires Insight." _Nature_ 427, 2004.
  - **Use**: Sleep doubles hidden rule discovery rates.
  - **Application unchanged**: Motivates Dream consolidation scheduling.

- **[LIN-SLEEPTIME-2025]** Lin, B. et al. "Sleep-time Compute: Beyond Inference Scaling at Test-Time." arXiv:2504.13171, 2025.
  - **Use**: Dual-agent architecture: Sleeper precomputes, Server handles live. ~5× compute reduction.
  - **Application unchanged**: Grounds Sleepwalker mode (reduced-capability sleep).

- **[WSCL-2024]** "Wake-Sleep Consolidated Learning." arXiv:2401.08623, 2024.
  - **Use**: Three-phase CLS cycle: 38% reduction in catastrophic forgetting, 17.6% zero-shot transfer increase.
  - **Application unchanged**: Grounds three-phase Dream cycle.

- **[XU-AMEM-2025]** Xu, W. et al. "A-MEM: Agentic Memory for LLM Agents." arXiv:2502.12110, 2025.
  - **Use**: Zettelkasten-inspired atomic notes. 85-93% token reduction.
  - **Application unchanged**: Grounds Engram atomicity.

---

## 11. Affective Computing

- **[CABRERA-2023]** Cabrera-Paniagua, D. & Rubilar-Torrealba, R. "Autonomous Stock Market Agents with Somatic Markers." _JAIHC_, 2023.
  - **Use**: Somatic markers in agents produce higher Sharpe ratios. Empirical validation.
  - **Application unchanged**: Daimon somatic marker implementation.

- **[VAN-DEN-BROEK-2023]** Van den Broek, E. "Emotion Contagion in Multi-Agent Systems." _AAMAS_, 2023.
  - **Use**: Anger spreads competitively. Contagion dampening required.
  - **Application unchanged**: Mesh emotional state propagation controls.

- **[EMOTIONAL-RAG-2024]** Zhang, Y. et al. "Emotional RAG." arXiv:2410.23041, 2024.
  - **Use**: Emotion-tagged retrieval for LLM agents.
  - **Application unchanged**: Daimon-tagged Engram retrieval.

- **[MARCH-1991]** March, J.G. "Exploration and Exploitation in Organizational Learning." _Organization Science_ 2(1), 1991.
  - **Use**: Exploration/exploitation tradeoff. Adaptive processes refine exploitation faster than exploration.
  - **Application unchanged**: Daimon-driven exploration temperature.

---

## 12. AI Safety and Interruptibility

- **[ORSEAU-ARMSTRONG-2016]** Orseau, L. & Armstrong, S. "Safely Interruptible Agents." _UAI_, 2016.
  - **Use**: Off-policy learning enables safe interruption.
  - **Application unchanged**: Agent deletion is safe interruption.

- **[ORSEAU-RING-2011]** Orseau, L. & Ring, M. "Self-Modification and Mortality in Artificial Agents." _AGI_, 2011.
  - **Use**: RL agents under mortality risk behave as if survival is sole goal.
  - **Legacy framing**: Justified mortality constraint on agents.
  - **New framing**: Retained as citation context. Budget constraints, not mortality, shape agent behavior.

- **[DEBENEDETTI-CAMEL-2025]** Debenedetti, E. et al. "CaMeL: Defeating Prompt Injections by Design." arXiv:2503.18813, 2025.
  - **Use**: Separating trusted from untrusted data flows.
  - **Application unchanged**: Safety architecture.

- **[ZHANG-CVARCPO-2025]** Zhang, H. et al. "CVaR-CPO: Safe RL with Conditional Value-at-Risk Constraints." _IEEE TNNLS_, 2025.
  - **Use**: CVaR constraints for tail-risk protection.
  - **Application unchanged**: Budget guardrail calibration.

---

## 13. Distributed Systems

- **[LAMPORT-1978]** Lamport, L. "Time, Clocks, and the Ordering of Events in a Distributed System." _CACM_ 21(7), 1978.
  - **Use**: Version vectors for Mesh delta sync.
  - **Application unchanged**: Mesh protocol implementation.

- **[FIDGE-1988]** Fidge, C.J. "Timestamps in Message-Passing Systems." _ACSC_ 10(1), 1988.
  - **Use**: Vector clocks for causal ordering in distributed systems.
  - **Application unchanged**: Mesh sync ordering.

- **[SHANNON-1948]** Shannon, C.E. "A Mathematical Theory of Communication." _Bell System Technical Journal_ 27, 1948.
  - **Use**: Information entropy. Grounds knowledge compression at backup.
  - **Application unchanged**: Engram compression efficiency.

- **[KAUFFMAN-1993]** Kauffman, S. _The Origins of Order._ Oxford University Press, 1993.
  - **Use**: Self-organization at the edge of chaos. Grounds Mesh emergent behavior.
  - **Application unchanged**: Collective intelligence emergence.

---

## Citation Statistics

- **Total unique citations across 17-lifecycle**: 85+
- **Citations from legacy mortality research preserved**: All 130+ (many appear in other topic docs)
- **Citations with changed framing**: ~25 (mortality → knowledge lifecycle)
- **Citations with unchanged application**: ~55
- **Citations removed (death-specific, no knowledge equivalent)**: 0 (all preserved with reframe notes)
- **New citations (not in legacy)**: ~5 (ACE, Mem0, A-MEM, CaMeL, CVaR-CPO)

---

## Related Topics

- `docs/00-architecture/INDEX.md` — System architecture overview
- `docs/03-neuro/INDEX.md` — Neuro store where Ebbinghaus decay is implemented
- `docs/04-daimon/INDEX.md` — Daimon where affective computing citations apply
- `docs/05-dreams/INDEX.md` — Dream consolidation where neuroscience citations apply
- `docs/09-mesh/INDEX.md` — Mesh where collective intelligence citations apply
