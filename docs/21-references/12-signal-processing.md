# Signal Processing and Time Series

> Academic foundations for spectral decomposition, predictive filtering, and temporal pattern detection in Roko's cognitive clock and monitoring systems.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md)
**Key sources**: `bardo-backup/prd/shared/citations.md` §§30-31

> **Implementation**: Reference

---

## Abstract

Roko agents operate at three cognitive frequencies — Gamma (~5-15s), Theta (~75s), Delta (~hours). The signal processing literature provides the mathematical foundations for spectral decomposition of agent performance data, predictive filtering for state estimation, and multi-scale temporal analysis. These methods underpin the adaptive clock, drift detection, and performance monitoring.

---

## Predictive Processing

- Clark, A. (2013). Whatever Next? Predictive Brains, Situated Agents, and the Future of Cognitive Science. _Behavioral and Brain Sciences_, 36(3), 181-204.
  *Grounds: Predictive processing framework — brains as prediction machines that minimize prediction error. Foundational for Roko's prediction-error-driven T0/T1/T2 tier routing. The agent's cognitive tier is determined by prediction error magnitude.*

---

## Topological Data Analysis

- Carlsson, G. (2009). Topology and Data. _Bulletin of the American Mathematical Society_, 46(2), 255-308.
  *Grounds: TDA foundations — topological methods for data analysis, particularly persistent homology for detecting multi-scale structure.*

- Gidea, M. & Katz, Y. (2018). Topological Data Analysis of Financial Time Series: Landscapes of Crashes. _Physica A_, 491, 820-834.
  *Grounds: Crash detection — persistent homology detects structural changes in financial time series that precede crashes. Applicable to anomaly detection in agent performance metrics.*

- Bauer, U. (2021). Ripser: Efficient Computation of Vietoris-Rips Persistence Barcodes. _Journal of Applied and Computational Topology_, 5, 391-423.
  *Grounds: Efficient TDA — efficient algorithm for computing persistence barcodes. Enables practical TDA on agent performance streams.*

- Perea, J.A. & Harer, J. (2015). Sliding Windows and Persistence: An Application of Topological Methods to Signal Analysis. _Foundations of Computational Mathematics_, 15(3), 799-838.
  *Grounds: Sliding window TDA — topological methods applied to time series via sliding window embeddings. Applicable to detecting periodic patterns in agent behavior.*

---

## Information Theory

- Shannon, C.E. (1948). A Mathematical Theory of Communication. _Bell System Technical Journal_, 27, 379-423 & 623-656.
  *Grounds: Information theory foundation — foundational information theory. Mutual information, entropy, and channel capacity are used throughout Roko for measuring knowledge value and information flow.*

- Cover, T.M. & Thomas, J.A. (2006). _Elements of Information Theory_, 2nd ed. Wiley.
  *Grounds: Information theory reference — comprehensive treatment of information theory including rate-distortion theory, source coding, and channel capacity.*

- Simon, H.A. (1971). Designing Organizations for an Information-Rich World. In _Computers, Communications, and the Public Interest_. Johns Hopkins Press.
  *Grounds: Attention as scarce resource — "a wealth of information creates a poverty of attention." Foundational insight for the VCG attention auction and context budget management.*

- Still, S. et al. (2012). Thermodynamics of Prediction. _Physical Review Letters_, 109(12), 120604.
  *Grounds: Prediction thermodynamics — formal connection between prediction efficiency and thermodynamic cost. The minimum energy required to predict is bounded by mutual information between past and future.*

---

## Computational Irreversibility

- Landauer, R. (1961). Irreversibility and Heat Generation in the Computing Process. _IBM Journal of Research and Development_, 5(3), 183-191.
  *Grounds: Landauer's principle — erasing information has a minimum thermodynamic cost. Theoretical foundation for the claim that knowledge decay (forgetting) is not free but has real computational cost.*

- Bennett, C.H. (1982). The Thermodynamics of Computation — A Review. _International Journal of Theoretical Physics_, 21(12), 905-940.
  *Grounds: Reversible computation — comprehensive review of computational thermodynamics. Provides the theoretical context for why selective forgetting (pruning low-value knowledge) is computationally preferable to accumulation.*

---

## TDA Advances (2024-2025)

- Topological Data Analysis and Topological Deep Learning Beyond Persistent Homology (2025). _Artificial Intelligence Review_, Springer, 2025.
  *Grounds: Beyond persistent homology — comprehensive review of TDA extensions including persistent topological Laplacians that capture both topological invariants and homotopic shape evolution during filtration. Addresses the limitation that standard persistent homology misses geometric evolution. Applicable to detecting subtle structural changes in agent performance metrics that don't involve topological transitions.*

- Persistent Homology-Based Algorithm for Unsupervised Anomaly Detection in Time Series (2025). OpenReview.
  *Grounds: TDA anomaly detection — algorithm using delay embeddings and 1-dimensional persistent homology from distance-to-measure Rips filtration for unsupervised anomaly detection. Competitive with state-of-the-art methods. Applicable to Roko's drift detection in agent performance streams.*

- Multivariate Time-Series Anomaly Detection with Topological Analysis (2024). arXiv:2408.13082.
  *Grounds: Graph-TDA anomaly detection — enhanced GAT modeling higher-order topological features as persistent homology groups under varying graph filtering degrees. Improves accuracy of inter-feature dependency modeling. Applicable to monitoring correlated agent performance metrics.*

- Change Point Detection in Financial Time Series Using TDA (2025). _Systems_, 13(10), 875.
  *Grounds: TDA change points — Takens embedding and sliding window techniques transform time series into high-dimensional topological space for change point detection. Applicable to detecting regime changes in agent execution patterns.*

---

## Cross-References

- See [11-streaming-algorithms.md](./11-streaming-algorithms.md) for ADWIN and online statistics
- See [16-active-inference.md](./16-active-inference.md) for free energy and prediction error
- See topic [00-architecture](../00-architecture/INDEX.md) for the three cognitive frequencies
