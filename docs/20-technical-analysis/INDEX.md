# Topic 20: Technical Analysis — Universal Oracle Primitives

> TA is NOT chain-only. It is a general-purpose prediction framework with domain-specific instances.

**Prerequisites**: [00-architecture](../00-architecture/INDEX.md) for Synapse Architecture (Engram, 6 traits, 5 layers), [05-learning](../05-learning/INDEX.md) for cybernetic feedback loops and CascadeRouter, [06-neuro](../06-neuro/INDEX.md) for HDC knowledge encoding and tier progression

---

## Overview

Technical analysis (TA) originated as a financial discipline — chart patterns, moving averages, momentum oscillators applied to price data. In Roko, TA is generalized into a set of **universal oracle primitives**: prediction, evaluation, calibration, and feedback loops that operate identically across any domain where an agent interacts with a verifiable external system.

The core insight: code, markets, research, and operations all share the same structural properties that make TA useful — measurable state variables, time series dynamics, feedback loops, pattern recurrence, adversarial dynamics, and external verification. A build time trend is structurally analogous to a price trend. A test failure probability is structurally analogous to a risk assessment. The mathematics is identical; the domain vocabulary changes.

The `Oracle` trait provides the universal interface. Domain-specific implementations (chain, coding, research, custom) handle the details. New domains are added by implementing the Oracle trait — not modifying the kernel.

---

## Sub-documents

| # | File | Title | Lines | Summary |
|---|---|---|---|---|
| 00 | [00-vision-ta-generalized.md](./00-vision-ta-generalized.md) | TA as Universal Oracle Primitives | ~230 | Vision document. Why generalize TA. Structural analogy argument. Cross-domain HDC transfer. Where oracles fit in the Synapse Architecture. |
| 01 | [01-oracle-trait.md](./01-oracle-trait.md) | The Oracle Trait | ~380 | Full Rust trait signature. `predict()`, `evaluate()`. OracleQuery, Prediction, PredictionAccuracy structs. PredictionStore, ResidualCorrector, CalibrationTracker. Integration with all 6 Synapse traits. |
| 02 | [02-chain-oracles.md](./02-chain-oracles.md) | Chain Oracles | ~300 | ChainOracle implementation. Traditional TA (MA, RSI, Bollinger, MACD). DeFi-native indicators (concentrated liquidity, lending, funding rates, yield curves, on-chain options). MEV detection. 8 T0 chain probes. Mirage-rs integration. |
| 03 | [03-coding-oracles.md](./03-coding-oracles.md) | Coding Oracles | ~320 | CodingOracle implementation. Build time prediction, test failure probability, complexity drift, dependency risk, performance regression. 6 T0 coding probes. Tech debt feedback loops. roko-index integration. |
| 04 | [04-research-oracles.md](./04-research-oracles.md) | Research Oracles | ~280 | ResearchOracle implementation. Source reliability, completeness assessment, contradiction detection, replication probability, citation momentum. p-hacking detection. Charnov stopping rule for research. |
| 05 | [05-witness-as-ta-generalized.md](./05-witness-as-ta-generalized.md) | The Witness Pipeline | ~280 | Generalized witness trait. Chain, coding, research witness implementations. Triage pipeline (MIDAS-R, DDSketch). CorticalState shared signal bus. Three cognitive speeds in the witness. |
| 06 | [06-hyperdimensional-ta.md](./06-hyperdimensional-ta.md) | Hyperdimensional TA | ~280 | HDC pattern algebra for TA. Role-filler composition. Temporal encoding via permutation. Shift-invariant matching. DeFi and coding codebooks. Cross-domain resonance detection (threshold 0.526). Pattern store with Dreams consolidation. |
| 07 | [07-spectral-liquidity-manifolds.md](./07-spectral-liquidity-manifolds.md) | Spectral Liquidity Manifolds | ~300 | Riemannian geometry for DeFi execution costs. Metric tensor (slippage + gas + time + opportunity). Christoffel symbols. Geodesics as optimal execution paths. Ricci scalar as market stability indicator. Parallel transport for cross-protocol pattern transfer. Fréchet mean. Spectral decomposition. |
| 08 | [08-adaptive-signal-metabolism.md](./08-adaptive-signal-metabolism.md) | Adaptive Signal Metabolism | ~280 | Signals as living organisms. Hebbian learning (Oja's rule). Replicator dynamics (Taylor & Jonker). Fisher's fundamental theorem. Speciation, fitness landscapes (Sewall Wright). Red Queen dynamic. SignalRegistry ecosystem. |
| 09 | [09-causal-microstructure-discovery.md](./09-causal-microstructure-discovery.md) | Causal Microstructure Discovery | ~280 | Pearl's causal hierarchy (3 levels). Structural causal models. do-operator. PC algorithm (Spirtes/Glymour/Scheines). Granger causality with 4 DeFi extensions. Interventional discovery via mirage-rs. Dream-based counterfactuals. Backdoor criterion. |
| 10 | [10-predictive-geometry-and-resonant-patterns.md](./10-predictive-geometry-and-resonant-patterns.md) | Predictive Geometry & Resonant Patterns | ~320 | TDA persistence diagrams and landscapes (Bubenik). Topology-to-trajectory mapping. Resonant patterns as organisms with HDC genomes. Reproductive algebra. VCG auction competition. Lotka-Volterra dynamics. Price equation. |
| 11 | [11-adversarial-signal-robustness.md](./11-adversarial-signal-robustness.md) | Adversarial Signal Robustness | ~300 | Adversarial signal decomposition. HDC prototype matching (~10ns). Robust statistics (trimmed mean, Hodges-Lehmann, MAD, rank transform). Signal cross-validation. Red-team dreaming algorithm. Domain-specific attack prototypes. |
| 12 | [12-somatic-ta-and-emergent-multiscale.md](./12-somatic-ta-and-emergent-multiscale.md) | Somatic TA & Emergent Multiscale Intelligence | ~320 | Somatic markers (Damasio) as HDC bindings. PAD encoding. Somatic retrieval (~63ns). 15% contrarian retrieval (Bower). IIT Phi over 9 TA subsystems (510 bipartitions). MIB diagnostic. PID synergy detection (Williams & Beer). |
| 13 | [13-predictive-foraging-and-active-inference.md](./13-predictive-foraging-and-active-inference.md) | Predictive Foraging & Active Inference | ~330 | The complete prediction-resolution-calibration loop. PredictionClaim, ResidualCorrector (~50ns), CalibrationTracker. Active inference POMDP (90 states). EFE decomposition (pragmatic + epistemic - ambiguity). Charnov MVT stopping rule. Thompson Sampling for oracle selection. Collective calibration on Korai. |
| 14 | [14-sheaf-tropical-geometry.md](./14-sheaf-tropical-geometry.md) | Sheaf-Theoretic Consistency & Tropical Decision Geometry | ~450 | Cellular sheaves for oracle consistency (Hansen & Ghrist). Sheaf Laplacian, cohomology for inconsistency detection. Sheaf neural networks (Bodnar et al.). Tropical semiring (max-plus). Tropical polynomials as oracle decisions. Tropical convexity. Tropical attention (symbolic-neural fusion). Tropical robustness (exact adversarial distances). Tropical VCG auctions. |

---

## Key concepts

| Concept | Description | Where defined |
|---|---|---|
| **Oracle trait** | Universal prediction interface: `predict()` + `evaluate()` | [01-oracle-trait.md](./01-oracle-trait.md) |
| **Prediction** | Value + confidence + interval + horizon + lineage | [01-oracle-trait.md](./01-oracle-trait.md) |
| **PredictionStore** | Lifecycle management: register → track → resolve → feedback | [01-oracle-trait.md](./01-oracle-trait.md) |
| **ResidualCorrector** | Bias elimination at ~50ns per correction | [13-predictive-foraging-and-active-inference.md](./13-predictive-foraging-and-active-inference.md) |
| **CalibrationTracker** | Per-(model, category) accuracy statistics | [13-predictive-foraging-and-active-inference.md](./13-predictive-foraging-and-active-inference.md) |
| **CorticalState** | Shared atomic signal bus for T0 probes | [05-witness-as-ta-generalized.md](./05-witness-as-ta-generalized.md) |
| **T0 Probes** | 16 zero-LLM probes (8 chain + 6 coding + 2 universal) | [02-chain-oracles.md](./02-chain-oracles.md), [03-coding-oracles.md](./03-coding-oracles.md) |
| **HDC pattern algebra** | 10,240-bit BSC vectors: bind (XOR), bundle (majority), permute (rotate) | [06-hyperdimensional-ta.md](./06-hyperdimensional-ta.md) |
| **Somatic markers** | HDC bindings between patterns and PAD affect states | [12-somatic-ta-and-emergent-multiscale.md](./12-somatic-ta-and-emergent-multiscale.md) |
| **Spectral manifold** | Riemannian geometry over liquidity cost landscape | [07-spectral-liquidity-manifolds.md](./07-spectral-liquidity-manifolds.md) |
| **Active inference** | Factorized POMDP (90 states), EFE for context selection | [13-predictive-foraging-and-active-inference.md](./13-predictive-foraging-and-active-inference.md) |
| **Replicator dynamics** | Fitness-proportionate signal evolution | [08-adaptive-signal-metabolism.md](./08-adaptive-signal-metabolism.md) |
| **Causal discovery** | Pearl's SCM + PC algorithm + Granger + mirage-rs interventions | [09-causal-microstructure-discovery.md](./09-causal-microstructure-discovery.md) |
| **Persistence landscapes** | Banach-space elements from TDA for trajectory prediction | [10-predictive-geometry-and-resonant-patterns.md](./10-predictive-geometry-and-resonant-patterns.md) |
| **Red-team dreaming** | Adversarial self-simulation during Delta Dreams | [11-adversarial-signal-robustness.md](./11-adversarial-signal-robustness.md) |
| **IIT Phi** | Integrated information metric over 9 TA subsystems | [12-somatic-ta-and-emergent-multiscale.md](./12-somatic-ta-and-emergent-multiscale.md) |
| **Conformal prediction** | Distribution-free prediction sets with coverage guarantees | [01-oracle-trait.md](./01-oracle-trait.md) |
| **Oracle composition** | Weighted ensemble, conformal aggregation, recalibration | [01-oracle-trait.md](./01-oracle-trait.md) |
| **Fisher-Rao metric** | Information-geometric Riemannian metric on oracle parameter space | [07-spectral-liquidity-manifolds.md](./07-spectral-liquidity-manifolds.md) |
| **Natural gradient** | Coordinate-free optimization on statistical manifolds | [07-spectral-liquidity-manifolds.md](./07-spectral-liquidity-manifolds.md) |
| **NOTEARS/SDCD** | Continuous optimization for DAG structure learning | [09-causal-microstructure-discovery.md](./09-causal-microstructure-discovery.md) |
| **DAG-GNN** | Neural causal discovery with GNN encoder | [09-causal-microstructure-discovery.md](./09-causal-microstructure-discovery.md) |
| **Persistence images** | Stable vector representation of persistent homology | [10-predictive-geometry-and-resonant-patterns.md](./10-predictive-geometry-and-resonant-patterns.md) |
| **Certified robustness** | Randomized smoothing, Lipschitz bounds, IBP | [11-adversarial-signal-robustness.md](./11-adversarial-signal-robustness.md) |
| **Cellular sheaves** | Local-to-global consistency via sheaf Laplacian and cohomology | [14-sheaf-tropical-geometry.md](./14-sheaf-tropical-geometry.md) |
| **Tropical polynomials** | Max-plus algebra for piecewise-linear oracle decisions | [14-sheaf-tropical-geometry.md](./14-sheaf-tropical-geometry.md) |
| **Tropical attention** | Symbolic-neural fusion via max-plus attention mechanism | [14-sheaf-tropical-geometry.md](./14-sheaf-tropical-geometry.md) |

---

## Citation index

All academic citations used across this topic's sub-documents:

| Citation | Used in |
|---|---|
| Friston, K. (2010). "The free-energy principle." *Nature Reviews Neuroscience*, 11(2), 127-138. | 00, 01, 05, 13 |
| Conant, R. C., & Ashby, W. R. (1970). "Every good regulator of a system." *IJSS*, 1(2), 89-97. | 00, 01, 13 |
| Sumers, T. R., et al. (2023). "Cognitive Architectures for Language Agents." arXiv:2309.02427. | 00, 01 |
| Lee, S., et al. (2026). "Meta-Harness." arXiv:2603.28052. | 00, 13 |
| Chen, L., et al. (2023). "FrugalGPT." arXiv:2305.05176. | 00, 01, 02, 03, 05, 13 |
| Kleyko, D., et al. (2022). "A Survey on Hyperdimensional Computing." *ACM Computing Surveys*, 54(6). | 00, 01, 03, 04, 06, 08, 11, 12 |
| Kanerva, P. (2009). "Hyperdimensional Computing." *Cognitive Computation*, 1(2), 139-159. | 00, 06 |
| Pirolli, P., & Card, S. (1999). "Information foraging." *Psychological Review*, 106(4), 643-675. | 00, 04, 13 |
| Charnov, E. L. (1976). "Optimal foraging: the marginal value theorem." *TPB*, 9, 129-136. | 00, 04, 05, 13 |
| Ousterhout, J. (2018). *A Philosophy of Software Design*. | 01, 03 |
| Vickrey, W. (1961). "Counterspeculation, Auctions." *Journal of Finance*, 16(1). | 01, 02, 10 |
| Pearl, J. (2009). *Causality*. 2nd ed. Cambridge University Press. | 02, 09, 11 |
| Masson, C., et al. (2019). "DDSketch." *PVLDB*, 12(12), 2195-2205. | 02, 05 |
| Wilder, J. W. (1978). *New Concepts in Technical Trading Systems*. | 02 |
| Garman, M. B., & Klass, M. J. (1980). "Estimation of Security Price Volatilities." *JoB*, 53(1). | 02 |
| McCabe, T. J. (1976). "A Complexity Measure." *IEEE TSE*, SE-2(4). | 03 |
| Lehman, M. M. (1980). "Programs, Life Cycles, and Laws of Software Evolution." *IEEE*, 68(9). | 03 |
| Nagappan, N., & Ball, T. (2005). "Relative Code Churn Measures." *ICSE 2005*. | 03 |
| Open Science Collaboration. (2015). "Reproducibility of psychological science." *Science*, 349(6251). | 04 |
| Simmons, J. P., et al. (2011). "False-Positive Psychology." *Psychological Science*, 22(11). | 04 |
| Ioannidis, J. P. A. (2005). "Why Most Published Research Findings Are False." *PLoS Medicine*, 2(8). | 04 |
| Bhatia, S., et al. (2020). "MIDAS." *AAAI 2020*. | 05 |
| McClelland, J. L., et al. (1995). "Complementary learning systems." *Psychological Review*, 102(3). | 05 |
| Plate, T. A. (1995). "Holographic Reduced Representations." *IEEE TNN*, 6(3). | 06 |
| Frady, E. P., et al. (2018). "Sequence Indexing." *Neural Computation*, 30(6). | 06 |
| Rachkovskij, D. A. (2001). "Binary Sparse Distributed Codes." *IEEE TKDE*, 13(2). | 06 |
| Lacaux, C., et al. (2021). "Sleep onset is a creative sweet spot." *Science Advances*, 7(50). | 06 |
| Amari, S., & Nagaoka, H. (2000). *Methods of Information Geometry*. | 07 |
| do Carmo, M. P. (1992). *Riemannian Geometry*. | 07 |
| Pennec, X. (2006). "Intrinsic Statistics on Riemannian Manifolds." *JMIV*, 25(1). | 07 |
| Taylor, P. D., & Jonker, L. B. (1978). "Evolutionary Stable Strategies." *Math. Biosci.*, 40(1-2). | 08 |
| Fisher, R. A. (1930). *The Genetical Theory of Natural Selection*. | 08 |
| Wright, S. (1932). "Roles of Mutation, Inbreeding, Crossbreeding, and Selection." | 08 |
| Van Valen, L. (1973). "A New Evolutionary Law." *Evolutionary Theory*, 1. | 08 |
| Oja, E. (1982). "Simplified neuron model." *J. Math. Biol.*, 15(3). | 08 |
| Hebb, D. O. (1949). *The Organization of Behavior*. | 08 |
| Spirtes, P., et al. (2000). *Causation, Prediction, and Search*. 2nd ed. MIT Press. | 09 |
| Granger, C. W. J. (1969). "Causal Relations by Econometric Models." *Econometrica*, 37(3). | 09 |
| Pearl, J. (2019). "Seven tools of causal inference." *CACM*, 62(3). | 09 |
| Bubenik, P. (2015). "Statistical TDA using Persistence Landscapes." *JMLR*, 16(3). | 10 |
| Takens, F. (1981). "Detecting strange attractors in turbulence." *LNM*, 898. | 10 |
| Price, G. R. (1970). "Selection and Covariance." *Nature*, 227. | 10 |
| Lotka, A. J. (1925). *Elements of Physical Biology*. | 10 |
| Carlsson, G. (2009). "Topology and Data." *Bulletin of the AMS*, 46(2). | 10 |
| Huber, P. J. (1964). "Robust Estimation." *Annals of Math. Stat.*, 35(1). | 11 |
| Hodges, J. L., & Lehmann, E. L. (1963). "Estimates of Location Based on Rank Tests." | 11 |
| Hampel, F. R. (1974). "The Influence Curve." *JASA*, 69(346). | 11 |
| Damasio, A. R. (1994). *Descartes' Error*. Putnam. | 12 |
| Mehrabian, A., & Russell, J. A. (1974). *An Approach to Environmental Psychology*. | 12 |
| Bower, G. H. (1981). "Mood and Memory." *American Psychologist*, 36(2). | 12 |
| Kahneman, D. (2011). *Thinking, Fast and Slow*. | 12 |
| Tononi, G. (2004). "Information integration theory." *BMC Neuroscience*, 5(42). | 12 |
| Williams, P. L., & Beer, R. D. (2010). "Nonnegative decomposition." arXiv:1004.2515. | 12 |
| Thompson, W. R. (1933). "On the Likelihood." *Biometrika*, 25(3-4). | 13 |
| Raj, V., & Kalyani, S. (2017). "Taming Non-stationary Bandits." arXiv:1707.09727. | 13 |
| Murphy, A. H. (1973). "A New Vector Partition of the Probability Score." *J. Applied Meteorology*, 12(4). | 01 |
| Vovk, V., Gammerman, A., & Shafer, G. (2005). *Algorithmic Learning in a Random World*. Springer. | 01 |
| Angelopoulos, A. N., & Bates, S. (2023). "Conformal Prediction: A Gentle Introduction." arXiv:2107.07511. | 01 |
| Naeini, M. P., et al. (2015). "Obtaining Well Calibrated Probabilities Using Bayesian Binning." *AAAI 2015*. | 01 |
| Guo, C., et al. (2017). "On Calibration of Modern Neural Networks." *ICML 2017*. | 01 |
| Cesa-Bianchi, N., & Lugosi, G. (2006). *Prediction, Learning, and Games*. Cambridge University Press. | 01 |
| Amari, S. (1998). "Natural Gradient Works Efficiently in Learning." *Neural Computation*, 10(2). | 07 |
| Amari, S. (2016). *Information Geometry and Its Applications*. Springer. | 07 |
| Čencov, N. N. (1982). *Statistical Decision Rules and Optimal Inference*. AMS. | 07 |
| Villani, C. (2009). *Optimal Transport: Old and New*. Springer. | 07 |
| Martens, J., & Grosse, R. (2015). "Optimizing Neural Networks with K-FAC." *ICML 2015*. | 07 |
| Zheng, X., et al. (2018). "DAGs with NO TEARS." *NeurIPS 2018*. | 09 |
| Yu, Y., et al. (2019). "DAG-GNN: Structure Learning with Graph Neural Networks." *ICML 2019*. | 09 |
| Nazaret, A., et al. (2024). "Stable Differentiable Causal Discovery." *ICML 2024*, PMLR 235. | 09 |
| Bello, K., et al. (2022). "DAGMA: Learning DAGs via M-matrices." *NeurIPS 2022*. | 09 |
| Adams, H., et al. (2017). "Persistence Images." *JMLR*, 18(8), 1-35. | 10 |
| Bauer, U. (2021). "Ripser: Efficient Computation of Vietoris-Rips Persistence Barcodes." *JACT*, 5(1). | 10 |
| Cohen-Steiner, D., et al. (2007). "Stability of Persistence Diagrams." *DCG*, 37(1). | 10 |
| Cohen, J. M., et al. (2019). "Certified Adversarial Robustness via Randomized Smoothing." *ICML 2019*. | 11 |
| Gowal, S., et al. (2018). "Interval Bound Propagation." arXiv:1810.12715. | 11 |
| Steinhardt, G., et al. (2017). "Certified Defenses for Data Poisoning Attacks." *NeurIPS 2017*. | 11 |
| Hansen, J., & Ghrist, R. (2019). "Toward a Spectral Theory of Cellular Sheaves." *JACT*, 3. | 14 |
| Bodnar, C., et al. (2022). "Neural Sheaf Diffusion." arXiv:2202.04579. | 14 |
| Zhang, L., Naitzat, G., & Lim, L.-H. (2018). "Tropical Geometry of Deep Neural Networks." *ICML 2018*. | 14 |
| Tran, N. M., & Yu, J. (2019). "Product-Mix Auctions and Tropical Geometry." *MOR*, 44(4). | 14 |
| Alfarra, M., et al. (2024). "Tropical Decision Boundaries Are Robust." arXiv:2402.00576. | 14 |
| Gebhart, T., et al. (2023). "Knowledge Sheaves." *PMLR 206*. | 14 |

---

## Cross-topic references

| Topic | Relationship |
|---|---|
| [00-architecture](../00-architecture/INDEX.md) | Synapse Architecture, Engram struct, 6 traits, 5 layers, Universal Cognitive Loop |
| [05-learning](../05-learning/INDEX.md) | CascadeRouter (LinUCB, Thompson Sampling), adaptive gate thresholds, cybernetic feedback loops |
| [06-neuro](../06-neuro/INDEX.md) | HDC encoding (10,240-bit BSC), knowledge tier progression, cross-domain transfer |
| [07-daimon](../09-daimon/INDEX.md) | PAD vector, behavioral states, somatic landscape, affect modulation of oracle behavior |
| [08-dreams](../10-dreams/INDEX.md) | NREM replay, REM counterfactuals, hypnagogia, offline consolidation of prediction patterns |
| [09-innovations](../20-technical-analysis/INDEX.md) | T0 probes, VCG auction, somatic landscape, collective calibration, predictive foraging |

---

## Generation notes

- **Generated by**: Claude Opus 4.6, PRD migration batch
- **Source material**: `refactoring-prd/03-cognitive-subsystems.md` §4, `refactoring-prd/09-innovations.md` §I/II/III/VII/XIX, `refactoring-prd/01-synapse-architecture.md`, `bardo-backup/prd/23-ta/*` (11 legacy files), `bardo-backup/tmp/agent-chain/10-predictive-foraging.md`, `tmp/implementation-plans/modelrouting/12-advanced-patterns.md`
- **Naming map applied**: golem→agent, grimoire→neuro, bardo→roko, Signal→Engram, GNOS→KORAI, clade→collective, Styx→Agent Mesh
- **Reframe rules applied**: mortality→resource management, death clocks→budget limits, succession→backup/restore
- **Citation count**: 76 unique academic citations across 15 sub-documents
- **Total lines**: ~11,000 across 16 files
- **2025-04-13 enhancement**: Deep research pass adding oracle calibration/composition (conformal prediction, Brier decomposition), information geometry (Fisher-Rao, natural gradient, α-connections, Wasserstein), continuous DAG learning (NOTEARS, DAG-GNN, SDCD, DAGMA), advanced TDA (persistence images, Ripser, vectorization methods), certified adversarial robustness (randomized smoothing, Lipschitz bounds, IBP), sheaf theory (cellular sheaves, Laplacian, cohomology), and tropical geometry (max-plus semiring, tropical attention, tropical VCG auctions). 28 new citations from 2017-2025 research.
