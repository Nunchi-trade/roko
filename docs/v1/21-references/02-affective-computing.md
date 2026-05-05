# Affective Computing

> Academic foundations for emotion modeling, somatic markers, and affect-modulated cognition in the Roko Daimon subsystem.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md), [Daimon](../09-daimon/INDEX.md)
**Key sources**: `bardo-backup/prd/02-mortality/14-research-foundations.md` §3, `bardo-backup/prd/shared/citations.md` §28

> **Implementation**: Reference

---

## Abstract

The Daimon is not cosmetic. Five independent research lines — somatic markers, mood-congruent retrieval, exploration modulation, narrative transfer, and empirical trading results — converge on the conclusion that emotion-like states serve genuine computational functions that cognition alone cannot replicate. The PAD (Pleasure-Arousal-Dominance) vector, the ALMA three-layer temporal model, the Somatic Landscape, and affect-modulated retrieval are all grounded in the citations below.

---

## PAD Emotional State Model

- Mehrabian, A. (1996). Pleasure-Arousal-Dominance: A General Framework for Describing and Measuring Individual Differences in Temperament. _Current Psychology_, 14(4), 261-292.
  *Grounds: Daimon PAD vector — foundational PAD emotional state model. Three continuous dimensions capture more variance than discrete emotion labels. Roko uses: Pleasure = task success, Arousal = urgency/load, Dominance = confidence.*

- Russell, J.A. & Mehrabian, A. (1977). Evidence for a Three-Factor Theory of Emotions. _Journal of Research in Personality_, 11(3), 273-294.
  *Grounds: PAD validation — three-dimensional affect captures emotional experience more completely than discrete categories. PAD octants map to behavioral tendencies.*

---

## Somatic Marker Hypothesis

- Damasio, A.R. (1994). _Descartes' Error: Emotion, Reason, and the Human Brain_. Putnam.
  *Grounds: Somatic Landscape — patients without emotion make consistently worse decisions under uncertainty. Somatic markers bias choices before deliberation. Roko implements this as a k-d tree over 8-dimensional strategy space.*

- Bechara, A., Damasio, H., & Damasio, A.R. (2000). Emotion, Decision Making and the Orbitofrontal Cortex. _Cerebral Cortex_, 10(3), 295-307.
  *Grounds: Pre-cognitive gut feelings — anticipatory SCRs precede conscious awareness in the Iowa Gambling Task. Roko's SomaticLandscape provides fast heuristic feelings about strategy regions before analytical reasoning begins.*

- Bechara, A. & Damasio, A.R. (2005). The Somatic Marker Hypothesis: A Neural Theory of Economic Decision. _Games and Economic Behavior_, 52, 336-372.
  *Grounds: Economic decision-making — formal integration of somatic marker theory with economic decision theory. Validates PAD-modulated tier routing for cost-sensitive agent decisions.*

- Cabrera-Paniagua, D. & Rubilar-Torrealba, R. (2023). Autonomous Stock Market Agents with Somatic Markers. _Journal of Ambient Intelligence and Humanized Computing_.
  *Grounds: Empirical validation — agents with somatic markers achieve higher Sharpe ratios on S&P 500 and Dow Jones benchmarks. Punishment signals triggered by drawdowns adaptively reduced position sizes. Directly validates somatic markers for autonomous agents.*

---

## Mood-Congruent Memory and Emotional Retrieval

- Bower, G.H. (1981). Mood and Memory. _American Psychologist_, 36(2), 129-148.
  *Grounds: Four-factor retrieval emotional congruence — emotional states bias memory retrieval via associative network activation. Directly implemented as the emotional factor (0.15 weight) in four-factor retrieval scoring. Also motivates the mandatory 15% contrarian retrieval to prevent echo chambers.*

- Blaney, P.H. (1986). Affect and Memory: A Review. _Psychological Bulletin_, 99(2), 229-246.
  *Grounds: Mood-congruent retrieval validation — comprehensive review confirming mood-congruent memory effects across multiple experimental paradigms. Supports emotional congruence as a retrieval factor.*

- Phelps, E.A. (2004). Human Emotion and Memory: Interactions of the Amygdala and Hippocampal Complex. _Current Opinion in Neurobiology_, 14(2), 198-202.
  *Grounds: Emotion-memory integration — emotion and memory as a single system with two interfaces. Grounds the emotional_tag field on every NeuroStore entry.*

- Cahill, L. & McGaugh, J.L. (1998). Mechanisms of Emotional Arousal and Lasting Declarative Memory. _Trends in Neurosciences_, 21(7), 294-299.
  *Grounds: Arousal encoding factor — arousal enhances consolidation via amygdala modulation. Grounds the arousal_encoding_factor() function in knowledge entry creation.*

---

## Emotion Classification and Appraisal

- Plutchik, R. (1980). _Emotion: A Psychoevolutionary Synthesis_. Harper & Row.
  *Grounds: Daimon emotion classification — eight primary emotions from evolutionary pressure. PAD octants map to Plutchik categories for human-readable emotional state labeling.*

- Scherer, K.R. (2001). Appraisal Considered as a Process of Multilevel Sequential Checking. In _Appraisal Processes in Emotion_, Oxford University Press.
  *Grounds: Engram Score appraisal theory — multi-level sequential checking mirrors the 7-axis Score on Engrams. Each axis captures an independent dimension of appraisal.*

- Ortony, A., Clore, G.L., & Collins, A. (1988). _The Cognitive Structure of Emotions_. Cambridge University Press.
  *Grounds: OCC emotion model — cognitive appraisal-based emotion taxonomy. Complements PAD with a structure-of-emotion framework for interpreting agent behavioral states.*

---

## Temporal Affect Models

- Gebhard, P. (2005). ALMA — A Layered Model of Affect. _AAMAS_, 2005.
  *Grounds: Three-layer temporal affect (ALMA) — emotion (seconds), mood (hours), personality (lifetime). Implemented in Roko as tick-level emotion, EMA mood, and static personality configuration.*

---

## Affect in Agent Systems

- Zhang, Y. et al. (2024). Self-Emotion Changes ~50% of Agent Decisions in Social Simulation. _SIGDIAL_, 2024.
  *Grounds: Daimon is architectural, not decorative — self-emotion changes approximately 50% of agent decisions in social simulation. Demonstrates that affect is a primary driver of agent behavior, not a secondary display layer.*

- Gadanho, S.C. (2003). Learning Behavior-Selection by Emotions and Cognition in a Multi-Goal Robot Task. _Journal of Machine Learning Research_, 4, 385-412.
  *Grounds: Combined affect-cognition architecture — ALEC (emotion + cognition) architecture: 40% fewer collisions vs cognition alone. Validates the combined Daimon + cognitive loop architecture.*

- Barthet, M. et al. (2022). Go-Blend: Affect-Driven Reinforcement Learning. _IEEE Transactions on Affective Computing_.
  *Grounds: Exploration temperature modulation — affect-driven RL improves exploration efficiency and agent performance. Arousal modulates exploration temperature in Roko's tier routing.*

- Seligman, M.E.P. (1972). Learned Helplessness. _Annual Review of Medicine_, 23, 407-412.
  *Grounds: Dominance alert — learned helplessness from uncontrollable negative outcomes. Dominance < -0.3 for 200+ ticks triggers an alert in the Daimon behavioral state machine.*

- Van den Broek, E. (2023). Emotion Contagion in Multi-Agent Systems. _Autonomous Agents and Multi-Agent Systems_.
  *Grounds: Arousal contagion dampening — anger spreads more competitively than other emotions. Contagion dampening required to prevent panic cascades. Arousal contagion is capped at +0.3 per sync cycle in Roko collectives.*

---

## Emotional Depotentiation

- Walker, M.P. & van der Helm, E. (2009). Overnight Therapy? The Role of Sleep in Emotional Brain Processing. _Psychological Bulletin_, 135(5), 731-748.
  *Grounds: REM emotional depotentiation — REM sleep depotentiates emotional charge while preserving informational content. Dream cycles reduce arousal on highly charged memories by 0.3-0.5 per cycle to prevent panic lock-in. Cross-referenced in [03-dreams-and-offline-learning.md](./03-dreams-and-offline-learning.md).*

---

## Emotional RAG

- Zhang, Y. et al. (2024). Emotional RAG. arXiv:2410.23041.
  *Grounds: PAD vectors on knowledge entries — emotion-tagged retrieval significantly outperforms non-emotional retrieval across three datasets. Validates attaching PAD vectors to every NeuroStore entry.*

---

## Affective Computing Surveys and Frameworks (2025)

- Yin, Y. et al. (2025). Emotions in Artificial Intelligence. arXiv:2505.01462.
  *Grounds: Teleology-driven affect — proposes a teleology-driven affective computing framework unifying major emotion theories under the premise that affect is adaptive and goal-directed. Emphasizes aligning agent responses with both individual and collective well-being over extended timescales. Validates the Daimon as a goal-directed computational system, not a cosmetic overlay.*

- Intelligent Agents with Emotional Intelligence (2025). Current Trends, Challenges, and Future Prospects. arXiv:2511.20657.
  *Grounds: Emotionally intelligent agents — survey of agents with emotional intelligence identifying key challenges in development. Validates the architectural necessity of emotional intelligence in agent systems, supporting Roko's Daimon as an integral subsystem.*

- Emotions in the Loop (2025). A Survey of Affective Computing for Emotional Support. arXiv:2505.01542.
  *Grounds: Affect-in-the-loop — comprehensive survey of affective computing integrated into agent interaction loops. Validates Roko's approach of integrating affect (Daimon) into the cognitive loop rather than treating it as a post-hoc analysis layer.*

---

## Cross-References

- See [03-dreams-and-offline-learning.md](./03-dreams-and-offline-learning.md) for Walker & van der Helm emotional depotentiation in the dreams context
- See [01-memory-consolidation.md](./01-memory-consolidation.md) for mood-congruent retrieval in the memory context
- See topic [09-daimon](../09-daimon/INDEX.md) for the full Daimon subsystem design
