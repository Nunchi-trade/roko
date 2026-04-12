# Hyperdimensional Computing and Vector Symbolic Architectures

> Academic foundations for HDC/VSA: the 10,240-bit Binary Spatter Code algebra, learned hashing, similarity search, and HDC-based knowledge representation in Roko.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md), [Primitives](../00-architecture/02-primitives.md)
**Key sources**: `bardo-backup/prd/shared/hdc-vsa.md`, `bardo-backup/tmp/agent-chain/08-references.md`, `bardo-backup/tmp/agent-chain/14-academic-foundations.md` §2

---

## Abstract

Roko uses 10,240-bit Binary Spatter Codes (BSC) as the universal representation substrate for knowledge similarity, cross-domain transfer, and structural analogy detection. HDC provides three algebraic operations — XOR binding, majority-vote bundling, and cyclic-shift permutation — that compose knowledge representations in nanoseconds on commodity hardware. The mathematical foundation rests on the Johnson-Lindenstrauss lemma (preserving distances under projection) and Kanerva's insight that in very high dimensions, random vectors are nearly orthogonal with high probability.

---

## Foundational HDC Theory

- Kanerva, P. (1988). _Sparse Distributed Memory_. MIT Press.
  *Grounds: Content-addressable memory — in high dimensions (D ≥ 1,000), random vectors are nearly orthogonal with high probability, enabling content-addressable memory with simple bitwise operations. The foundational work for all of Roko's HDC operations.*

- Kanerva, P. (2009). Hyperdimensional Computing: An Introduction to Computing in Distributed Representation with High-Dimensional Random Vectors. _Cognitive Computation_, 1(2), 139-159.
  *Grounds: HDC terminology — introduces binding, bundling, permutation. Explains why 10,000-dimensional binary vectors provide sufficient capacity for practical knowledge systems. The primary reference for Roko's 10,240-bit BSC dimensionality.*

---

## VSA Surveys

- Kleyko, D., Rachkovskij, D.A., Osipov, E., & Rahimi, A. (2022). A Survey on Hyperdimensional Computing aka Vector Symbolic Architectures. _ACM Computing Surveys_, 55(6), Article 130.
  *Grounds: BSC selection — the most comprehensive HDC/VSA survey. Covers all major VSA families (MAP-B, MAP-C, BSC, HRR, FHRR, VTB). Validates the bundle similarity formula and capacity bounds used in Roko's implementation.*

- Neubert, P., Schubert, S., & Protzel, P. (2022). Vector Symbolic Architectures as a Computing Framework for Emerging Hardware. _Proceedings of the IEEE_. arXiv:2106.05268.
  *Grounds: BSC hardware — comprehensive VSA survey covering Binary Spatter Codes (BSC/MAP-B): XOR binding, majority-vote bundling, cyclic-shift permutation. Surveys FPGA and ASIC implementations achieving sub-microsecond operations.*

---

## Resonator Networks and Advanced Retrieval

- Frady, E.P., Kleyko, D., & Sommer, F.T. (2021). Computing on Functions Using Randomized Vector Representations. arXiv:2109.03429.
  *Grounds: Resonator networks — iterative convergence methods for HDC retrieval. More space-efficient than exhaustive search for very large dictionaries. Referenced as future optimization path (not needed at current scale where SIMD exhaustive search suffices).*

---

## Random Projection Theory

- Johnson, W.B. & Lindenstrauss, J. (1984). Extensions of Lipschitz Mappings into a Hilbert Space. _Contemporary Mathematics_, 26, 189-206.
  *Grounds: Dimensionality preservation — the JL lemma: N points in high-dimensional space can be projected into O(log N / ε²) dimensions while preserving pairwise distances within (1 ± ε). For ε = 0.1 and N = 100,000: D ≥ 4,604. Roko's 10,240 bits provide generous headroom. Mathematical foundation for the random projection from 1,536-dim LLM embeddings to 10,240-bit binary hypervectors.*

---

## Locality-Sensitive Hashing

- Charikar, M.S. (2002). Similarity Estimation Techniques from Rounding Algorithms. _STOC_, 2002.
  *Grounds: SimHash — single random projection h(x) = sign(w^T x) produces binary codes where collision probability equals 1 − θ/π. The Phase 1 encoding in Roko's HDC pipeline. The projection matrix is derived deterministically from configuration, ensuring all agents produce identical hypervectors for identical embeddings.*

- Indyk, P. & Motwani, R. (1998). Approximate Nearest Neighbors: Towards Removing the Curse of Dimensionality. _STOC_, 1998.
  *Grounds: LSH foundation — random hash functions preserve similarity in sub-linear query time. Foundational result enabling efficient similarity search in HDC space.*

---

## Learned Hashing

- Kulis, B. & Darrell, T. (2009). Learning to Hash with Binary Reconstructive Embeddings. _NeurIPS_, 2009.
  *Grounds: Data-dependent hashing — learned hash functions minimize reconstruction error between original distances and Hamming distances, outperforming random projections at the same code length. Phase 2 encoding path.*

- Cao, Z., Long, M., Wang, J., & Yu, P.S. (2017). HashNet: Deep Learning to Hash by Continuation. _ICCV_, 2017.
  *Grounds: Differentiable hashing — continuation method sharpening smooth activation into sign function; +14.6% absolute MAP improvement on ImageNet. Phase 2 encoding technique.*

- Yuan, L. et al. (2020). Central Similarity Quantization for Efficient Image and Video Retrieval. _CVPR_, 2020.
  *Grounds: Hash centers — well-separated binary target points constructed via Hadamard matrices. All same-class codes converge to their shared center. Maps to knowledge domain prototypes in Roko's HDC system. Phase 4 encoding.*

---

## Differentiable HDC Operations

- Ganesan, A. et al. (2021). Learning with Holographic Reduced Representations. _NeurIPS_, 2021 (Spotlight).
  *Grounds: Differentiable HDC — made Holographic Reduced Representations viable as differentiable deep learning components via projection in the Fourier domain. +100x retrieval improvement. The critical bridge paper enabling end-to-end learning with HDC.*

- Alam, M. et al. (2023). Recasting Self-Attention with Holographic Reduced Representations (HRRFormer). _ICML_, 2023.
  *Grounds: HDC attention — replaced self-attention with HRR binding operations, scaling to sequence length 131,072. Demonstrates HDC as a viable attention mechanism.*

---

## FPGA and Hardware Acceleration

- Imani, M. et al. (2019). FloatHD: Integer-Based Training Framework for Hyperdimensional Computing. _IEEE/ACM ICCAD_, 2019.
  *Grounds: Hardware HDC — FPGA implementations achieving ~3-5ns per comparison at 200 MHz. Referenced as the FPGA acceleration path for Roko's HDC similarity search.*

---

## Online and Streaming Hashing

- Çakir, F., He, K., Bargal, S.A., & Sclaroff, S. (2017). MIHash: Online Hashing with Mutual Information. _NeurIPS_, 2017.
  *Grounds: Streaming adaptation — online hashing for synchronous binary code updates under continuous data arrival. Applicable to streaming knowledge ingestion in Roko.*

---

## Approximate Nearest Neighbor Search

- Malkov, Y.A. & Yashunin, D.A. (2020). Efficient and Robust Approximate Nearest Neighbor using Hierarchical Navigable Small World Graphs. _IEEE TPAMI_, 2020.
  *Grounds: HNSW search — O(log N) search at 95-99% recall for billion-scale binary vectors. The production search infrastructure for Roko's HDC index.*

- Zhang, L. et al. (2023). SPFresh: Incremental In-Place Update for Billion-Scale Vector Search. _SIGMOD_, 2023.
  *Grounds: Incremental index updates — LIRE (Lightweight Incremental RE-balancing) for incremental graph rebalancing under continuous insert/delete. Applicable to Roko's continuously-updated knowledge index.*

---

## Holographic Reduced Representations

- Plate, T.A. (1994). Distributed Representations and Nested Compositional Structure. PhD Dissertation, University of Toronto.
  *Grounds: HRR theory — introduced Holographic Reduced Representations using circular convolution for binding. The theoretical ancestor of BSC, using continuous rather than binary vectors.*

---

## Cross-references

- See [01-memory-consolidation.md](./01-memory-consolidation.md) for HDC-encoded knowledge retrieval
- See [03-dreams-and-offline-learning.md](./03-dreams-and-offline-learning.md) for HDC counterfactual synthesis
- See topic [00-architecture](../00-architecture/INDEX.md) for HDC integration in the Synapse Architecture
