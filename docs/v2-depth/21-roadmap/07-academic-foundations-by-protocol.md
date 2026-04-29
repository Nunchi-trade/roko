# Academic Foundations by Protocol

> Organizes 500+ citations from `docs/21-references/` by which unified protocol or runtime primitive they ground. Every design decision in the Roko spec traces to published research indexed here.

**Depth for**: [28-ROADMAP.md](../../unified/28-ROADMAP.md)
**Sources**: All 25 files in `docs/21-references/` (00 through 25)
**Prerequisites**: [00-INDEX.md](../../unified/00-INDEX.md) (vocabulary), [02-CELL.md](../../unified/02-CELL.md) (protocols)

---

## How to Use This Document

Each section maps to one of the 9 unified protocols (or a cross-cutting concern). Within each section:

- **Key papers** are listed with one-line relevance notes
- **Depth doc reference** shows which depth doc uses the citation
- **Implementation priority** (P0 = already used in runtime, P1 = next quarter, P2 = target state)
- **Runtime primitive** names the Cell, Graph, or Store structure the paper grounds

If you are implementing a Cell that conforms to a particular protocol, start here to understand what research justifies the design choices.

---

## Store Protocol Citations

> `put / get / query / query_similar / prune` -- Persist and retrieve Signals. Content-addressed, demurrage-decayed.

The Store protocol is grounded in memory consolidation, knowledge management, and streaming algorithms. The central empirical finding: **forgetting is optimization, not failure** (Richards & Frankland 2017).

### Memory Systems and Consolidation

| Paper | Relevance | Priority |
|-------|-----------|----------|
| McClelland, McNaughton & O'Reilly (1995). CLS Theory. _Psych. Review_ | Dual-store architecture: fast episodic + slow semantic consolidation. Grounds NeuroStore's episode-to-insight tier promotion. | P0 |
| Ebbinghaus (1885). _Uber das Gedachtnis_ | Negative exponential decay with retrieval-slowing. Directly implemented as per-type demurrage rates. | P0 |
| Richards & Frankland (2017). Persistence and Transience of Memory. _Neuron_ | Forgetting = L1 regularization. Foundational for the entire demurrage architecture. | P0 |
| Gesell (1916). _The Natural Economic Order_ | Demurrage: money decays over time to encourage circulation. The economic metaphor for knowledge decay. | P0 |
| Nader et al. (2000). Reconsolidation after Retrieval. _Nature_ | Retrieved memories become labile and can be updated. Grounds confidence-update-on-retrieval. | P0 |
| Roediger & Karpicke (2006). Test-Enhanced Learning. _Psych. Sci._ | Retrieval strengthens traces (+200% vs passive). Grounds the strength-increment-on-retrieval mechanism. | P0 |
| Cepeda et al. (2006). Distributed Practice. _Psych. Bull._ | Optimal spacing for durable memory. Grounds the 50-tick Curator cycle interval. | P1 |
| Park et al. (2023). Generative Agents. _UIST_ | Four-factor retrieval (recency, importance, relevance, emotional congruence). | P0 |
| Chhikara et al. (2025). Mem0. arXiv:2504.19413 | Two-phase extraction-update: +26% accuracy, 91% lower p95. Validates tiered store. | P1 |
| Xu et al. (2025). A-MEM. arXiv:2502.12110 | Atomic notes with dynamic links: 85-93% token reduction. Grounds bi-temporal metadata. | P1 |
| Arbesman (2012). _Half-Life of Facts_ | Per-domain factual decay rates. Informs per-type half-life calibration. | P1 |
| arXiv:2505.16067 (2025). Self-Degradation of Agent Memory | Naive add-all degrades performance. Grounds mark_verified quality gate. | P0 |

### Streaming and Online Algorithms

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Bifet & Gavalda (2007). ADWIN. _SIAM_ | Adaptive window for drift detection. Used in model routing. | P0 |
| Vovk, Gammerman & Shafer (2005). Conformal Prediction | Distribution-free prediction intervals. CalibrationTracker bounds. | P1 |
| Guo et al. (2017). Calibration of Neural Networks. _ICML_ | Temperature scaling for calibration. CalibrationTracker bias correction. | P1 |
| Malkov & Yashunin (2020). HNSW. _IEEE TPAMI_ | O(log N) search at 95-99% recall. Production HDC search infrastructure. | P1 |

### Knowledge Lifecycle

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Shuvaev et al. (2024). Genomic Bottleneck. _PNAS_ | Compression IS the regularizer. Grounds tier promotion forcing generalization. | P0 |
| Lenski et al. (2003). Evolutionary Origin of Complex Features. _PNAS_ | Complex features require generational turnover. Grounds Delta-frequency refresh. | P1 |
| Tononi & Cirelli (2014). Synaptic Homeostasis. _Neuron_ | Sleep prunes weak connections. Knowledge decay during idle. | P1 |
| Davis & Zhong (2017). Biology of Forgetting. _Neuron_ | Active forgetting is metabolically expensive (proving it serves a function). | P1 |

**Depth doc references**: [temporal-knowledge-graph.md](temporal-knowledge-graph.md), [04-hdc-pattern-encoding-and-metabolism.md](04-hdc-pattern-encoding-and-metabolism.md)

---

## Score Protocol Citations

> `rate along 5 dimensions` -- Evaluate Signal quality. Calibrated via Beta-Binomial tracker.

Score Cells produce quality evaluations. The research divides into process reward models (step-level verification), calibration theory, and collective scoring.

### Process Reward and Verification

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Lightman et al. (2024). Let's Verify Step by Step. arXiv:2305.20050 | Step-level verification outperforms outcome-only. Grounds per-gate scoring. | P0 |
| Song et al. (2025). Mind the Gap. _ICLR_ | Self-improvement requires verification > generation. The foundational result. | P0 |
| Huang et al. (2024). LLMs Cannot Self-Correct Yet. _ICLR_ | Self-correction without external feedback makes answers worse. External verifiers mandatory. | P0 |
| Pan et al. (2024). Spontaneous Reward Hacking. _ICML_ | Same-model generation + judging leads to reward hacking. Generator-verifier separation. | P0 |
| Wei et al. (2022). Chain-of-Thought. _NeurIPS_ | CoT makes reasoning steps explicit and individually verifiable. | P0 |
| Wang et al. (2023). Self-Consistency. _ICLR_ | Multiple paths + consensus = process verification. | P1 |

### Calibration and Uncertainty

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Guo et al. (2017). Calibration of Neural Networks. _ICML_ | Modern NNs are poorly calibrated; temperature scaling. | P0 |
| Lakshminarayanan et al. (2017). Deep Ensembles. _NeurIPS_ | Ensemble uncertainty estimation. Multi-model confidence in CascadeRouter. | P1 |
| Farquhar et al. (2024). Semantic Entropy. _Nature_ | Semantic entropy detects hallucinations. Confidence estimation in Gate. | P1 |
| Xiong et al. (2023). Can LLMs Express Uncertainty? | LLM confidence elicitation empirics. 7-axis Score calibration. | P1 |

### Collective Scoring

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Woolley et al. (2010). Collective Intelligence Factor. _Science_ | C-Factor: measurable group intelligence. Covariate, not objective. | P0 |
| Hawkins et al. (2017). Thousand Brains Theory. _Frontiers_ | Multiple columns vote on perception. Multi-agent estimation consensus. | P2 |
| Dabney et al. (2018). Distributional RL with Quantile Regression. _AAAI_ | Learn full distribution of returns, not just mean. Richer scoring signals. | P2 |

**Depth doc references**: [03-oracle-as-score-cell.md](03-oracle-as-score-cell.md)

---

## Verify Protocol Citations

> `check -> Verdict` -- Validate correctness, safety, quality. Conjunctive hard + Pareto soft.

Verify is load-bearing: it serves simultaneously as reward function, relabeling oracle, safety boundary, and economic attestation.

### Security and Safety Architecture

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Debenedetti et al. (2025). CaMeL. arXiv | Capability-based machine learning. Separates control from data flow. | P0 |
| Dennis & Van Horn (1966). Capability-Based Security. _CACM_ | Unforgeable capability tokens. Tool permission model. | P0 |
| Cohen (1987). Computer Viruses. _Comp. & Security_ | Perfect malicious detection undecidable. Defense must be structural. | P0 |
| Orseau & Armstrong (2016). Safely Interruptible Agents. _UAI_ | Off-policy learning for safe shutdown. Agent lifecycle management. | P0 |
| Bai et al. (2022). Constitutional AI. arXiv:2212.08073 | Harmlessness from AI feedback. Policy trait constitutional constraints. | P1 |
| OWASP (2025). Top 10 for LLM Applications | Memory poisoning ranked high. Decay as structural defense. | P0 |
| OWASP (2025). Agentic Security Top 10 | Confused deputy, privilege escalation, tool misuse. Safety layer design. | P0 |

### Formal Verification

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Position: Formal Methods for Safe AI (2025). _ICML_ | Model checking, theorem proving for AI. Validates structural safety. | P1 |
| Towards Guaranteed Safe AI (2024). arXiv:2405.06624 | World model + safety spec + verifier = quantitative guarantees. Maps to NeuroStore + Policy + Gate. | P1 |
| Berkenkamp et al. (2017). Safe Model-based RL. _NeurIPS_ | Lyapunov stability for safe exploration. | P2 |
| Alshiekh et al. (2018). Safe RL via Shielding. _AAAI_ | Runtime shields override unsafe actions. Analogous to Gate pipeline. | P1 |

### Provenance and Attestation

| Paper | Relevance | Priority |
|-------|-----------|----------|
| C2PA. Content Provenance Standard | Cryptographic proof of origin. Attestation field on Signals. | P1 |
| W3C. DIDs v1.0 | Decentralized identifiers. ERC-8004 agent identity. | P2 |
| Merkle (1987). Digital Signatures. _CRYPTO_ | Content-addressed verification trees. BLAKE3 content hashing. | P0 |

### Regulatory Compliance

| Paper | Relevance | Priority |
|-------|-----------|----------|
| EU AI Act (2024). Regulation 2024/1689 | Risk classification, transparency, human oversight. Forensic AI audit trail. | P1 |
| MiFID II. Directive 2014/65/EU | Algo trading record-keeping. Lineage DAG + episode logs satisfy requirement. | P2 |
| SOX (2002). Sarbanes-Oxley Act | Audit trails for financial reporting. Content-addressed Signal DAG. | P2 |
| GDPR (2016). Regulation 2016/679 | Right to be forgotten. Knowledge decay as structural data minimization. | P1 |

**Depth doc references**: [05-causal-discovery-and-adversarial-robustness.md](05-causal-discovery-and-adversarial-robustness.md)

---

## Route Protocol Citations

> `select among candidates` -- Choose Cell/model/path for task. EFE: epistemic + pragmatic - cost.

Routing is grounded in active inference (the principled answer to "what should I attend to?") and mechanism design (the principled answer to "how do I allocate scarce resources?").

### Active Inference and Free Energy

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Friston (2006). Free Energy Principle. _J. Physiol.-Paris_ | All self-organizing systems minimize variational free energy. Foundational. | P0 |
| Friston et al. (2015). Active Inference and Epistemic Value. _Cog. Neurosci._ | EFE = pragmatic_value + epistemic_value. Resolves explore-exploit. | P0 |
| Millidge et al. (2021). Whence the Expected Free Energy? _Neural Comp._ | Naive EFE discourages exploration. Essential corrective for implementation. | P0 |
| Parr, Pezzulo & Friston (2022). _Active Inference_ textbook | Complete mathematical framework. Primary implementation reference. | P0 |
| Shafiei et al. (2025). DR-FREE. _Nature Comms._ | Robust active inference under model uncertainty. Tier routing when domain model is imperfect. | P1 |
| Koudahl et al. (2024). Active Inference for Multi-LLM. arXiv:2412.10425 | Cognitive layer above LLMs adjusting prompts through information-seeking. | P1 |
| Itti & Baldi (2005). Bayesian Surprise. _NeurIPS_ | Surprise = KL(posterior, prior). Identical to epistemic EFE component. | P0 |

### Dual-Process and Cascade Routing

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Kahneman (2011). _Thinking, Fast and Slow_ | System 1 / System 2. T0/T1/T2 cascade. | P0 |
| Chen, Zaharia & Zou (2023). FrugalGPT. arXiv:2305.05176 | 98% cost reduction via intelligent routing. T0/T1/T2 cascade. | P0 |
| Ong et al. (2024). RouteLLM. arXiv:2406.18665 | Preference-based routing. CascadeRouter training. | P0 |
| Yoshida et al. (2024). System 1 to System 2 Distillation | Distilling slow into fast. Dual-process T0/T1/T2 architecture. | P1 |
| Hansen & Zilberstein (2001). Anytime Algorithms. _AI_ | Progressively better results; stop at cheapest sufficient tier. | P1 |

### Market Microstructure for Routing

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Peters (2019). Ergodicity Problem. _Nature Physics_ | Log-wealth maximization for non-ergodic agents. Kelly criterion for routing budget. | P2 |
| Kelly (1956). Information Rate. _Bell System Tech. J._ | Optimal bet sizing. Position sizing in Route decisions. | P2 |
| Lo (2004). Adaptive Markets. _J. Portfolio Mgmt._ | Markets adaptively efficient via evolutionary dynamics. Strategies must adapt. | P1 |

**Depth doc references**: [03-oracle-as-score-cell.md](03-oracle-as-score-cell.md), [06-advanced-geometry-and-integration.md](06-advanced-geometry-and-integration.md)

---

## Compose Protocol Citations

> `assemble under budget -> Signal` -- Combine context for LLM calls. VCG auction with section effect tracking.

Compose is grounded in context engineering (how to fill the window), mechanism design (how to allocate budget), and attention theory.

### Context Engineering

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Zhang et al. (2026). ACE: Agentic Context Engineering. _ICLR_ | Generator-Reflector-Curator cycle. +10.6% AppWorld. Compose-verify-persist. | P0 |
| Samsung Research (2025). CSO. arXiv:2511.03728 | 6x reduction, 10-25x growth rate reduction. Structured context compression. | P0 |
| Kang et al. (2025). ACON. arXiv:2510.00615 | 26-54% peak token reduction. Failure-driven compression. | P1 |
| Liu et al. (2024). Lost in the Middle. _TACL_ | U-shaped attention. Highest-priority at start and end. | P0 |
| Du et al. (2025). Context Length Hurts. _EMNLP_ | Even whitespace degrades. Aggressive compression mandate. | P0 |
| Shi et al. (2023). Distracted by Irrelevant Context. _ICML_ | Irrelevant context actively degrades. Quality filtering required. | P0 |
| Joren et al. (2025). Sufficient Context. _ICLR_ | Insufficient context = 6x worse. Quality gate on context. | P0 |
| Lewis et al. (2020). RAG. _NeurIPS_ | Retrieval-augmented generation. Per-tick context assembly. | P0 |
| Lindenbauer et al. (2025). Observation Masking. _NeurIPS_ | Halves cost while matching quality. T0 suppression pattern. | P1 |

### Attention and Budget Allocation

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Sims (2003). Rational Inattention. _J. Monetary Econ._ | Finite-capacity agents optimally ignore some information. VCG auction motivation. | P0 |
| Simon (1971). Attention in Information-Rich World | "Wealth of information creates poverty of attention." Context budget. | P0 |
| Kahneman (1973). _Attention and Effort_ | Attention as scarce resource. VCG competition for limited budget. | P0 |
| Nemhauser, Wolsey & Fisher (1978). Submodular Maximization. _Math. Prog._ | Greedy (1-1/e) approximation. Context selection is submodular. | P1 |

### Mechanism Design for Context

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Vickrey (1961). Second-Price Auctions. _J. Finance_ | Truthful bidding. Foundational for VCG attention auction. | P0 |
| Clarke (1971). Multipart Pricing. _Public Choice_ | Multi-item truthful allocation. Simultaneous context sections. | P0 |
| Groves (1973). Incentives in Teams. _Econometrica_ | Truthful revelation in teams. Subsystems reveal true valuation. | P0 |
| Milgrom (2004). _Putting Auction Theory to Work_ | Applied auction design. Efficient attention auction implementation. | P1 |
| Duetting et al. (2024). Mechanism Design for LLMs. _WWW_ | Token-level auctions for multi-LLM output. VCG at token granularity. | P2 |

**Depth doc references**: [emergent-goals-and-energy.md](emergent-goals-and-energy.md)

---

## React Protocol Citations

> `watch Pulses -> emit Signals/Pulses` -- Real-time event response. Operates on ephemeral Bus stream.

React Cells watch the ephemeral Bus and respond. The research grounds cybernetic feedback, self-learning, and adaptive control.

### Cybernetics and Feedback Control

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Wiener (1948). _Cybernetics_ | Feedback-based control. The cognitive loop as cybernetic system. | P0 |
| Ashby (1956). _Introduction to Cybernetics_ | Requisite variety: controller complexity >= system complexity. | P0 |
| Conant & Ashby (1970). Good Regulator Theorem. _IJSS_ | Every good regulator must model its system. World model requirement. | P0 |
| Beer (1972). _Brain of the Firm_ | VSM: five recursively nested subsystems. System 1-5 mapping to Roko. | P0 |
| Boyd (1987). OODA Loop | Observe-Orient-Decide-Act. Roko's loop extends with Gate and Policy. | P0 |
| Maxwell (1868). On Governors. _Proc. Royal Soc._ | First mathematical feedback control analysis. Adaptive clock. | P1 |
| Powers (1973). _Behavior: Control of Perception_ | Behavior controls perception, not output. Goal-directed action. | P1 |
| Sterling (2012). Allostasis. _Physiol. & Behavior_ | Predictive regulation (anticipating needs). Predictive foraging. | P1 |

### Self-Learning and Triple-Loop

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Argyris & Schon (1978). _Organizational Learning_ | Triple-loop: fix/change-strategy/change-learning. Gamma/Theta/Delta. | P0 |
| Shinn et al. (2023). Reflexion. _NeurIPS_ | Verbal RL via stored self-reflection. +22% AlfWorld. Theta-frequency. | P0 |
| Zhao et al. (2024). ExpeL. arXiv:2308.10144 | Cross-task experience extraction. Double-loop learning. | P0 |
| Wang et al. (2023). Voyager. arXiv:2305.16291 | Code-as-action skill library. 3.3x behaviors. EvoSkills. | P1 |
| Lee et al. (2026). Meta-Harness. arXiv:2603.28052 | 6x gap from scaffold changes. "The scaffold IS the product." | P0 |
| SAMULE (2025). Multi-level Reflection. _EMNLP_ | Across-trajectory reflection outperforms single. Theta-frequency validation. | P1 |

### Biological Adaptation

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Charnov (1976). Marginal Value Theorem. _Theor. Pop. Bio._ | When to leave a depleting resource patch. Task switching decision. | P1 |
| Kauffman (1993). _Origins of Order_ | Self-organized criticality. Adaptive clock targets edge of chaos. | P2 |
| Yerkes & Dodson (1908). Arousal-Performance. _J. Comp. Neurol._ | Inverted-U performance curve. Daimon arousal modulation. | P1 |
| Von Foerster (1979). Cybernetics of Cybernetics | Second-order: observer in the system. Meta-cognition step. | P2 |

**Depth doc references**: [emergent-goals-and-energy.md](emergent-goals-and-energy.md), [04-hdc-pattern-encoding-and-metabolism.md](04-hdc-pattern-encoding-and-metabolism.md)

---

## Agent Specialization Citations

> Cognitive architectures, affective computing, lifecycle/agency, philosophy -- What makes an Agent more than a generic Cell.

### Cognitive Architecture

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Sumers et al. (2023). CoALA. arXiv:2309.02427 | 9-step cognitive pipeline. Universal loop extends CoALA with Gate + Daimon. | P0 |
| Anderson (1993). ACT-R. _Rules of the Mind_ | Declarative/procedural memory. NeuroStore knowledge types. | P1 |
| Laird, Newell & Rosenbloom (1987). SOAR. _AI_ | Problem solving + chunking. Propose-decide-apply-learn maps to compose-act-verify-adapt. | P1 |
| Sun (2002). CLARION | Dual-level (explicit + implicit). NeuroStore entries + HDC vectors + somatic markers. | P1 |
| Maturana & Varela (1980). _Autopoiesis and Cognition_ | Self-producing systems. Agents produce knowledge sustaining their operation. | P2 |

### Affective Computing (Daimon)

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Mehrabian (1996). PAD Model. _Current Psychology_ | Three continuous dimensions (Pleasure, Arousal, Dominance). Daimon state vector. | P0 |
| Damasio (1994). _Descartes' Error_ | Somatic markers: emotion biases decision before deliberation. SomaticLandscape. | P0 |
| Bechara et al. (2000). Emotion and Decision. _Cerebral Cortex_ | Pre-cognitive gut feelings in Iowa Gambling Task. Fast heuristic feelings. | P0 |
| Gebhard (2005). ALMA. _AAMAS_ | Three-layer temporal affect: emotion/mood/personality. Tick/EMA/static. | P0 |
| Zhang et al. (2024). Self-Emotion Changes ~50% of Decisions. _SIGDIAL_ | Affect is primary driver, not display layer. Daimon is architectural. | P0 |
| Bower (1981). Mood and Memory. _Am. Psych._ | Mood-congruent retrieval. Emotional factor (0.15 weight) in retrieval. | P1 |
| Walker & van der Helm (2009). Overnight Therapy. _Psych. Bull._ | REM depotentiates emotional charge. Dream cycles reduce arousal. | P1 |
| Seligman (1972). Learned Helplessness | Dominance < -0.3 for 200+ ticks triggers alert. | P0 |

### Philosophy of Agency

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Jonas (1966). _Phenomenon of Life_ | Needful freedom: metabolism as freedom-through-necessity. Economic burn rate. | P0 |
| Heidegger (1927). _Being and Time_ | Temporal urgency from finite horizon. Budget creates prioritization. | P1 |
| Derrida (1993). _Specters of Marx_ | Hauntology: each agent differently haunted. Solves Alpha Convergence. | P1 |
| Varela, Thompson & Rosch (1991). _Embodied Mind_ | Enactive cognition: agents construct cognitive world through interaction. | P2 |
| Popper (1972). _Objective Knowledge_ | Knowledge evolves through conjecture and refutation. AntiKnowledge type. | P0 |
| Whitehead (1929). _Process and Reality_ | Reality as process. Signal DAG as actual occasion chain. | P2 |
| Camus (1942). _Myth of Sisyphus_ | Perseverance under uncertainty. Try-fail-learn-retry loop. | P2 |

**Depth doc references**: [emergent-goals-and-energy.md](emergent-goals-and-energy.md)

---

## Coordination Citations

> Multi-agent, stigmergy, generational/evolutionary -- How Cells in a Group or collective coordinate.

### Stigmergic Coordination

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Grasse (1959). Stigmergy. _Insectes Sociaux_ | Coordination through environmental traces without direct communication. Foundational. | P0 |
| Dorigo & Gambardella (1997). Ant Colony Optimization. _IEEE Trans. Evol. Comp._ | Pheromone deposit/evaporation. Confirmation-based half-life extension. | P0 |
| Parunak et al. (2002). Digital Pheromones. _AAMAS_ | Time-decaying digital signals. Pheromone decay + reinforcement. | P0 |
| Stigmergy: Mathematical Modelling (2024). _Proc. Royal Soc. A_ | PDE-based framework treating swarms as fluids. Rigorous Pheromone Field foundation. | P1 |
| Simard (2012). Mycorrhizal Networks | Underground relay without direct communication. Agent Mesh topology. | P1 |

### Collective Intelligence

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Woolley et al. (2010). C-Factor. _Science_ | Measurable collective intelligence. Four diagnostic signals. | P0 |
| Metcalfe (1995). Network value ~ O(N^2) | Superlinear knowledge scaling. | P1 |
| Reed (1999). Group-forming networks ~ 2^N | Permissioned subnets value. | P2 |
| Hayek (1945). Use of Knowledge in Society. _AER_ | Distributed knowledge, price system aggregates. Pheromone field. | P1 |
| Holland (1995). _Hidden Order_ | Adaptation builds complexity from simple rules. Emergent collective intelligence. | P2 |
| Emergent Coordination in Multi-Agent LLMs (2025). arXiv:2510.05174 | Identity-linked differentiation produces collective intelligence. | P1 |

### Evolutionary and Generational

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Ray (1991). Tierra. _Artificial Life II_ | Digital evolution halts without resource pressure. Knowledge decay necessity. | P0 |
| Baldwin (1896). A New Factor in Evolution. _Am. Nat._ | Learned behavior becomes structural. Heuristics promoted to Persistent tier. | P0 |
| Price (1970). Selection and Covariance. _Nature_ | Universal selection equation. Knowledge persistence = covariance with Gate success. | P1 |
| Fisher (1930). Fundamental Theorem. _Genetical Theory_ | Rate of improvement = variance in fitness. Diversity drives improvement. | P1 |
| Taylor & Jonker (1978). Replicator Dynamics. _Math. Biosci._ | Strategy frequency dynamics. Demurrage as replicator equation. | P1 |
| Dawkins (1976). _Selfish Gene_ | Memes: units of cultural transmission. Signals are agent-ecosystem memes. | P2 |

**Depth doc references**: [04-hdc-pattern-encoding-and-metabolism.md](04-hdc-pattern-encoding-and-metabolism.md), [05-causal-discovery-and-adversarial-robustness.md](05-causal-discovery-and-adversarial-robustness.md)

---

## HDC / Signal Citations

> Hyperdimensional computing foundations -- The 10,240-bit substrate for similarity, transfer, and analogy.

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Kanerva (1988). _Sparse Distributed Memory_ | Content-addressable memory in high dimensions. Foundational. | P0 |
| Kanerva (2009). Hyperdimensional Computing. _Cog. Comp._ | Binding, bundling, permutation. 10,240-bit BSC dimensionality. | P0 |
| Kleyko et al. (2022). VSA Survey. _ACM Comp. Surveys_ | Comprehensive. Validates BSC bundle similarity formula. | P0 |
| Johnson & Lindenstrauss (1984). JL Lemma. _Contemp. Math._ | Distance preservation under random projection. 10,240 bits = generous headroom. | P0 |
| Charikar (2002). SimHash. _STOC_ | Binary codes where collision probability = 1 - theta/pi. Phase 1 encoding. | P0 |
| Frady, Kleyko & Sommer (2021). Resonator Networks | Iterative HDC retrieval. Future optimization path. | P2 |
| Ganesan et al. (2021). Differentiable HRR. _NeurIPS Spotlight_ | End-to-end learning with HDC. +100x retrieval. Bridge paper. | P2 |
| Plate (1994). HRR Dissertation | Holographic Reduced Representations. Theoretical ancestor of BSC. | P2 |
| Rahimi et al. (2024). HDC Framework. _J. Big Data_ | Unified stochastic computation + symbolic AI. Validates general-purpose substrate. | P1 |
| FLASH (2024). Learnable Encoder. _Frontiers in AI_ | Gradient-descent encoder. Bridges Phase 1 fixed and Phase 2 learned. | P2 |

**Depth doc references**: [04-hdc-pattern-encoding-and-metabolism.md](04-hdc-pattern-encoding-and-metabolism.md)

---

## External Protocol Citations

> Protocol standards (MCP, A2A, ERC-8004, x402), blockchain, micropayments -- The exoskeleton.

| Paper | Relevance | Priority |
|-------|-----------|----------|
| Bryan (2024). ERC-8004: Agent Identity. EIPs | Soulbound NFT with capabilities. Agent identity standard. | P2 |
| Cloudflare/Linux Foundation (2025). x402 | HTTP 402 micropayments. Sub-second USDC settlement. Self-funding agents. | P2 |
| Anthropic (2024). MCP Specification | Tool interaction protocol. roko-agent MCP client. | P0 |
| Google (2025). A2A Protocol | Agent-to-agent communication. Mesh wire protocol. | P2 |
| ERC-4337. Account Abstraction | Smart contract wallets. Agent transaction execution. | P2 |
| Gesell (1916). Demurrage on KORAI token | 1% annual demurrage mirrors knowledge decay. | P2 |
| Ostrom (1990). _Governing the Commons_ | Shared resource governance without central authority. Knowledge commons. | P2 |
| Goldwasser, Micali & Rackoff (1985). ZK Proofs | Zero-knowledge proofs. Privacy-preserving knowledge verification. | P2 |
| Benet (2014). IPFS. arXiv:1407.3561 | Content-addressed P2P storage. BLAKE3 addressing follows same principle. | P1 |
| Szabo (1997). Smart Contracts. _First Monday_ | Self-enforcing digital agreements. Policy trait as computational contracts. | P2 |

---

## What This Enables

1. **Principled implementation**: Every Cell implementor can trace their design to published research.
2. **Literature-driven code review**: "Which paper justifies this design choice?" becomes answerable.
3. **Research debt tracking**: Papers at P2 that should be P0 indicate implementation gaps.
4. **Replication pipeline input**: Papers listed here feed into the Research-to-Runtime pipeline (see [08-research-to-runtime-bridge.md](08-research-to-runtime-bridge.md)).

## Feedback Loops

- **Score protocol calibration** feeds back to validate Score citations (do PRMs work for this domain?)
- **Store demurrage** produces empirical decay curves that can be compared to Ebbinghaus predictions
- **Route EFE** produces empirical explore-exploit balance that validates active inference papers
- **React triple-loop** timing (Gamma/Theta/Delta) can be calibrated against Argyris organizational learning

## Open Questions

1. How many of the P2 papers have runtime approximations that could graduate to P1?
2. Should the replication ledger track citation usage frequency (which papers actually get consulted at runtime)?
3. Can the CascadeRouter bandit arms be mapped 1:1 to the routing papers (FrugalGPT arm, RouteLLM arm, etc.)?
4. What is the minimum paper set for a new domain plugin (e.g., healthcare) to be safely deployable?

## Implementation Tasks

| Task | Path | Priority |
|------|------|----------|
| Add `paper_id` field to Heuristic knowledge type | `crates/roko-neuro/src/types.rs` | P1 |
| Wire replication ledger to Gate outcomes | `crates/roko-cli/src/orchestrate.rs` | P2 |
| Add citation count to `roko learn all` output | `crates/roko-cli/src/learn.rs` | P2 |
| Implement paper Signal Kind | `crates/roko-core/src/kind.rs` | P2 |
| Create starter-kit paper Signals for 12 foundational papers | `.roko/research/starter-kit/` | P1 |
