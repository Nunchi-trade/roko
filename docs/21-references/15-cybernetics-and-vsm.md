# Cybernetics and Viable System Model

> Academic foundations for cybernetic control theory, the Viable System Model, autopoiesis, and feedback-driven adaptation in Roko's architecture.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md)
**Key sources**: `bardo-backup/prd/shared/citations.md` §6, `bardo-backup/tmp/agent-chain/08-references.md`

---

## Abstract

Roko's architecture is fundamentally cybernetic: a system that regulates itself through feedback loops operating at multiple timescales. The Viable System Model (Beer 1972) provides the recursive organizational template. The Good Regulator Theorem (Conant & Ashby 1970) grounds the requirement that agents must model their domain. Ashby's Law of Requisite Variety determines the minimum complexity of the agent's internal model. These are not metaphors — they are engineering constraints that the Synapse Architecture satisfies.

---

## Cybernetic Foundations

- Wiener, N. (1948). _Cybernetics: Or Control and Communication in the Animal and the Machine_. MIT Press.
  *Grounds: Cybernetic framework — the foundational work on feedback-based control in machines and organisms. Roko's cognitive loop is a cybernetic feedback system: perceive → act → verify → adapt.*

- Ashby, W.R. (1956). _An Introduction to Cybernetics_. Chapman & Hall.
  *Grounds: Requisite variety — the Law of Requisite Variety: a controller must have at least as many states as the system it controls. Determines the minimum complexity of Roko's internal model for any given domain.*

---

## Good Regulator Theorem

- Conant, R.C. & Ashby, W.R. (1970). Every Good Regulator of a System Must Be a Model of That System. _International Journal of Systems Science_, 1(2), 89-97.
  *Grounds: World model requirement — any good regulator must contain a model of the system it regulates. Roko agents must build and maintain a world model of their domain. This is why NeuroStore knowledge, Daimon affect state, and prediction tracking are architectural requirements, not optional features.*

---

## Viable System Model

- Beer, S. (1972). _The Brain of the Firm_. Allen Lane.
  *Grounds: Five-system recursion — cybernetic framework with five recursively nested subsystems for organizational viability. Roko maps to VSM: System 1 (Operations) = individual agent execution; System 2 (Coordination) = pheromone field / Agent Mesh; System 3 (Control) = Gate pipeline + budget management; System 4 (Intelligence) = NeuroStore + HDC index; System 5 (Identity) = configuration + policy.*

- Beer, S. (1984). The Viable System Model: Its Provenance, Development, Methodology and Pathology. _Journal of the Operational Research Society_, 35(1), 7-25.
  *Grounds: VSM methodology — formal presentation of VSM methodology and diagnostic pathology. Provides the diagnostic framework for identifying organizational dysfunction in agent Collectives.*

---

## Autopoiesis

- Maturana, H.R. & Varela, F.J. (1980). _Autopoiesis and Cognition_. D. Reidel.
  *Grounds: Self-producing systems — autopoiesis: a system that produces itself. Roko agents are autopoietic in the cybernetic sense — they produce the knowledge that sustains their own operation through the cognitive loop.*

- Varela, F.J. (1991). Organism: A Meshwork of Selfless Selves. In _Organism and the Origins of Self_. Springer.
  *Grounds: Network selfhood — organisms as meshworks of autonomous subsystems. Grounds the Collective as a meshwork of autonomous agents with emergent collective behavior.*

---

## OODA Loop and Decision Cycles

- Boyd, J. (1987). Patterns of Conflict. Unpublished.
  *Grounds: OODA loop — Observe-Orient-Decide-Act decision cycle. The OODA loop is a cybernetic control loop; Roko's 9-step cognitive loop extends it with explicit verification (Gate) and learning (Policy) steps.*

---

## Feedback and Control

- Maxwell, J.C. (1868). On Governors. _Proceedings of the Royal Society_.
  *Grounds: Governor theory — the first mathematical analysis of feedback control. The conceptual ancestor of all cybernetic regulation, including Roko's adaptive clock that adjusts cognitive frequency based on performance.*

- Powers, W.T. (1973). _Behavior: The Control of Perception_. Aldine.
  *Grounds: Perceptual control — behavior controls perception, not output. Roko agents act to bring their perceived state in line with their reference state (goals), not to produce specific outputs.*

- Sterling, P. (2012). Allostasis: A Model of Predictive Regulation. _Physiology & Behavior_.
  *Grounds: Predictive regulation — allostasis as anticipatory regulation (predicting needs before they arise) vs. homeostasis (reacting to deviations). Grounds the predictive foraging system where agents anticipate information needs.*

- Cannon, W.B. (1932). _The Wisdom of the Body_. W.W. Norton.
  *Grounds: Homeostasis — the concept of maintaining internal stability through feedback. The simplest form of the regulatory principle that Roko implements at multiple timescales.*

---

## Triple-Loop Learning

- Argyris, C. & Schön, D. (1978). _Organizational Learning_. Addison-Wesley.
  *Grounds: Triple-loop learning — single-loop (fix errors in execution), double-loop (change the strategy), triple-loop (change the learning process). Maps to Gamma/Theta/Delta. Cross-referenced in [06-self-learning-systems.md](./06-self-learning-systems.md).*

- Tosey, P., Visser, M., & Saunders, M.N.K. (2012). The Origins and Conceptualizations of 'Triple-Loop' Learning. _Management Learning_, 43(3), 291-307.
  *Grounds: Triple-loop review — critical review of triple-loop learning origins and conceptualizations. Provides nuanced understanding of when each loop operates.*

---

## Second-Order Cybernetics

- Von Foerster, H. (1979). Cybernetics of Cybernetics. In _Communication and Control in Society_. Gordon and Breach.
  *Grounds: Observer inclusion — second-order cybernetics includes the observer in the system. Roko's meta-cognition step (Step 9: Daimon.assess()) is second-order: the agent observes itself observing.*

- Bateson, G. (1972). _Steps to an Ecology of Mind_. Ballantine.
  *Grounds: Ecology of mind — levels of learning and logical types. The distinction between learning (changing behavior) and learning-to-learn (changing the learning process) grounds Roko's multi-level adaptation.*

---

## Adaptive Markets

- Lo, A.W. (2004). The Adaptive Markets Hypothesis. _Journal of Portfolio Management_, 30(5), 15-29.
  *Grounds: Adaptive efficiency — markets are not perfectly efficient but adaptively efficient through evolutionary dynamics. Validates the premise that agent strategies must continuously adapt.*

---

## Reflexivity

- Soros, G. (1987). _The Alchemy of Finance_. Simon & Schuster.
  *Grounds: Reflexivity — participants' expectations affect the fundamentals they're trying to predict. Agents that publish predictions alter the system they're predicting. Grounds the need for contrarian retrieval and prediction calibration.*

---

## Cross-references

- See [04-coordination-and-multi-agent.md](./04-coordination-and-multi-agent.md) for stigmergic coordination
- See [16-active-inference.md](./16-active-inference.md) for free energy as cybernetic principle
- See [20-cognitive-architectures.md](./20-cognitive-architectures.md) for CoALA as cognitive cybernetics
