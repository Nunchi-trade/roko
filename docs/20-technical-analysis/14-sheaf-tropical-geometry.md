# Sheaf-Theoretic Consistency and Tropical Decision Geometry

> Sheaf theory provides local-to-global consistency guarantees across distributed oracle subsystems. Tropical geometry reveals the piecewise-linear decision boundaries of oracle policies and connects symbolic planning (dynamic programming) with neural computation via the max-plus semiring.


> **Implementation**: Specified

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [06-hyperdimensional-ta](./06-hyperdimensional-ta.md) for HDC encoding, [07-spectral-liquidity-manifolds](./07-spectral-liquidity-manifolds.md) for Riemannian geometry, [12-somatic-ta-and-emergent-multiscale](./12-somatic-ta-and-emergent-multiscale.md) for IIT Phi
**Key sources**: Hansen & Ghrist (2019), Bodnar et al. (2022), Zhang et al. (2018, ICML)

---

## Part I: Sheaf Theory for Oracle Consistency

### Why sheaves for distributed oracles

Roko's 9 TA subsystems (HDC patterns, spectral manifolds, causal discovery, TDA, signal metabolism, adversarial robustness, somatic markers, active inference, resonant patterns) each produce predictions that must be **locally consistent** — the chain oracle's price prediction should cohere with the liquidity manifold's execution cost estimate, which should cohere with the causal model's structural equations.

Sheaf theory (Bredon, 1997; Curry, 2014) provides the mathematical framework for exactly this problem: ensuring local consistency implies global consistency. A cellular sheaf assigns a vector space to each subsystem (its "prediction space") and linear maps between adjacent subsystems (their "consistency constraints"). When the sheaf has vanishing cohomology, local consistency implies global consistency — the system's predictions are guaranteed to be mutually compatible.

This is the mathematical formalization of what IIT Phi (doc 12) measures empirically: the degree to which subsystem predictions form a coherent whole. Sheaf cohomology replaces the brute-force 510-bipartition enumeration with a principled algebraic characterization.

### Cellular sheaves on the oracle graph

```rust
/// A cellular sheaf over the oracle subsystem graph.
///
/// The graph G has:
/// - Vertices v_i: the 9 TA subsystems (each producing predictions)
/// - Edges e_{ij}: pairs of subsystems that must be consistent
///
/// The sheaf assigns:
/// - F(v_i) = ℝ^{d_i}: prediction space of subsystem i
/// - F(e_{ij}): comparison space for consistency between i and j
/// - ρ_{v_i, e_{ij}}: F(v_i) → F(e_{ij}): restriction map
///   (projects subsystem i's prediction into the comparison space)
///
/// (Hansen & Ghrist, 2019, "Toward a Spectral Theory of Cellular Sheaves",
///  Journal of Applied and Computational Topology, 3, 315-358)
pub struct CellularSheaf {
    /// Number of vertices (subsystems).
    pub n_vertices: usize,
    /// Vertex stalks: dimension of each subsystem's prediction space.
    pub vertex_dims: Vec<usize>,
    /// Edges: pairs of vertices that share consistency constraints.
    pub edges: Vec<(usize, usize)>,
    /// Edge stalks: dimension of each comparison space.
    pub edge_dims: Vec<usize>,
    /// Restriction maps: linear maps from vertex stalk to edge stalk.
    /// restriction_maps[e] = (matrix_from_v1, matrix_from_v2)
    pub restriction_maps: Vec<(Vec<Vec<f64>>, Vec<Vec<f64>>)>,
}

/// A section of the sheaf: a choice of prediction from each subsystem.
pub struct SheafSection {
    /// Per-vertex prediction vectors.
    pub vertex_values: Vec<Vec<f64>>,
}

/// The coboundary operator δ: C^0(G, F) → C^1(G, F).
///
/// For a section s ∈ C^0, the coboundary measures inconsistency:
///   (δs)(e_{ij}) = ρ_{v_j, e_{ij}}(s(v_j)) - ρ_{v_i, e_{ij}}(s(v_i))
///
/// A section is CONSISTENT (a global section) iff δs = 0.
/// ||δs||² measures the total inconsistency across all edges.
impl CellularSheaf {
    /// Compute the coboundary of a section.
    pub fn coboundary(&self, section: &SheafSection) -> Vec<Vec<f64>> {
        self.edges.iter().enumerate().map(|(e_idx, &(vi, vj))| {
            let (rho_i, rho_j) = &self.restriction_maps[e_idx];
            let projected_i = mat_vec_mul(rho_i, &section.vertex_values[vi]);
            let projected_j = mat_vec_mul(rho_j, &section.vertex_values[vj]);
            subtract_vec(&projected_j, &projected_i)
        }).collect()
    }

    /// Total inconsistency: ||δs||².
    pub fn inconsistency(&self, section: &SheafSection) -> f64 {
        self.coboundary(section).iter()
            .flat_map(|v| v.iter())
            .map(|x| x * x)
            .sum()
    }
}
```

### Sheaf Laplacian — Diffusion with consistency

```rust
/// The sheaf Laplacian L_F = δ^T δ.
///
/// The sheaf Laplacian generalizes the graph Laplacian by incorporating
/// the restriction maps. Where the graph Laplacian diffuses scalar values,
/// the sheaf Laplacian diffuses VECTOR values while preserving
/// consistency structure.
///
/// Spectral properties (Hansen & Ghrist, 2019):
/// - ker(L_F) = H^0(G, F) = space of globally consistent sections
/// - dim(ker(L_F)) = β_0(F) = number of independent consistent predictions
/// - Smallest nonzero eigenvalue λ_1 = "consistency gap" (how far from
///   consistency the best non-trivial section is)
/// - Fiedler-like bound: λ_1 ≥ h²(F)/2 where h(F) is the sheaf Cheeger constant
pub struct SheafLaplacian {
    /// The Laplacian matrix L_F (block matrix, total dim = Σ d_i).
    pub matrix: Vec<Vec<f64>>,
    /// Total dimension.
    pub total_dim: usize,
}

impl CellularSheaf {
    /// Construct the sheaf Laplacian L_F = δ^T δ.
    pub fn laplacian(&self) -> SheafLaplacian {
        let total_dim: usize = self.vertex_dims.iter().sum();
        let mut matrix = vec![vec![0.0; total_dim]; total_dim];

        for (e_idx, &(vi, vj)) in self.edges.iter().enumerate() {
            let (rho_i, rho_j) = &self.restriction_maps[e_idx];
            let offset_i = self.vertex_offset(vi);
            let offset_j = self.vertex_offset(vj);

            // L_F has block structure:
            // L_F[vi,vi] += ρ_i^T ρ_i
            // L_F[vj,vj] += ρ_j^T ρ_j
            // L_F[vi,vj] -= ρ_i^T ρ_j
            // L_F[vj,vi] -= ρ_j^T ρ_i
            add_block(&mut matrix, offset_i, offset_i, &mat_mul_transpose(rho_i, rho_i));
            add_block(&mut matrix, offset_j, offset_j, &mat_mul_transpose(rho_j, rho_j));
            sub_block(&mut matrix, offset_i, offset_j, &mat_mul_transpose(rho_i, rho_j));
            sub_block(&mut matrix, offset_j, offset_i, &mat_mul_transpose(rho_j, rho_i));
        }

        SheafLaplacian { matrix, total_dim }
    }
}
```

### Sheaf cohomology for inconsistency detection

```rust
/// Sheaf cohomology H^k(G, F) detects global inconsistencies.
///
/// H^0(G, F) = ker(δ₀) = globally consistent sections
///   dim(H^0) > 0 means consistent global predictions exist.
///
/// H^1(G, F) = ker(δ₁) / im(δ₀) = obstructions to consistency
///   dim(H^1) > 0 means there are inconsistencies that cannot be
///   resolved by adjusting individual subsystem predictions.
///   These indicate STRUCTURAL contradictions in the oracle architecture.
///
/// (Curry, 2014, "Sheaves, Cosheaves and Applications", arXiv:1303.3255)
///
/// Connection to IIT Phi (doc 12):
/// - dim(H^0) large: high integration (subsystems agree)
/// - dim(H^1) large: low integration (structural disagreements)
/// - Sheaf cohomology provides the ALGEBRAIC explanation for
///   why certain bipartitions in the Phi computation lose less
///   information — they correspond to sheaf subcomplexes with
///   low H^1.
pub struct SheafCohomology {
    /// Betti numbers β_k = dim(H^k).
    pub betti_numbers: Vec<usize>,
    /// Basis of H^0 (globally consistent sections).
    pub global_sections: Vec<SheafSection>,
    /// Representatives of H^1 (inconsistency witnesses).
    pub obstruction_cocycles: Vec<Vec<Vec<f64>>>,
}

impl CellularSheaf {
    /// Compute sheaf cohomology via Smith normal form.
    pub fn cohomology(&self) -> SheafCohomology {
        let coboundary_matrix = self.build_coboundary_matrix();
        let snf = smith_normal_form(&coboundary_matrix);

        let beta_0 = snf.null_space_dim();
        let global_sections = snf.null_space_basis()
            .into_iter()
            .map(|v| self.vector_to_section(&v))
            .collect();

        SheafCohomology {
            betti_numbers: vec![beta_0, snf.cokernel_dim()],
            global_sections,
            obstruction_cocycles: snf.cokernel_basis(),
        }
    }
}
```

### Sheaf neural networks for learned consistency

```rust
/// Sheaf neural networks: learn restriction maps from data.
///
/// Instead of hand-coding consistency constraints between TA subsystems,
/// learn them from prediction-outcome pairs.
///
/// Architecture (Bodnar et al., 2022, "Neural Sheaf Diffusion",
///  arXiv:2202.04579):
/// 1. Input: subsystem predictions at vertices
/// 2. Sheaf diffusion: x_{t+1} = x_t - σ · L_F · x_t
///    where L_F is the sheaf Laplacian with LEARNED restriction maps
/// 3. Output: diffused predictions (consistent, denoised)
///
/// The restriction maps ρ_{v,e} are parameterized as small neural networks
/// or linear maps learned via backpropagation.
pub struct SheafNeuralNetwork {
    /// Number of diffusion steps.
    pub n_diffusion_steps: usize,   // default: 5
    /// Diffusion step size.
    pub sigma: f64,                  // default: 0.1
    /// Whether restriction maps are learned or fixed.
    pub learn_restrictions: bool,    // default: true
    /// Hidden dimension for restriction map networks.
    pub restriction_hidden_dim: usize, // default: 32
}
```

---

## Part II: Tropical Geometry for Decision Boundaries

### The max-plus semiring and oracle decisions

Every oracle prediction that selects among discrete outcomes computes a maximum over score functions — this is inherently tropical arithmetic.

```rust
/// Tropical semiring: (ℝ ∪ {-∞}, ⊕, ⊗) where:
///   a ⊕ b = max(a, b)     (tropical addition)
///   a ⊗ b = a + b          (tropical multiplication)
///
/// Key insight (Zhang et al., 2018, ICML):
/// A ReLU neural network computes a tropical rational function.
/// The decision boundary of max(f₁(x), f₂(x)) is a TROPICAL
/// HYPERSURFACE — a piecewise-linear codimension-1 set in input space.
///
/// For oracle policies:
///   prediction = argmax_k score_k(observation)
///   The boundary where score_i = score_j is a tropical hyperplane.
///   The arrangement of ALL boundaries forms a tropical polytope
///   whose combinatorial type characterizes the oracle's behavior.
pub struct TropicalPolynomial {
    /// Coefficients c_i and exponent vectors a_i.
    /// f(x) = max_i (c_i + a_i · x)
    pub terms: Vec<TropicalTerm>,
}

pub struct TropicalTerm {
    /// Constant coefficient.
    pub coefficient: f64,
    /// Exponent vector (linear coefficients in max-plus).
    pub exponents: Vec<f64>,
}

impl TropicalPolynomial {
    /// Evaluate the tropical polynomial at a point.
    /// f(x) = max_i (c_i + a_i · x)
    pub fn evaluate(&self, x: &[f64]) -> f64 {
        self.terms.iter()
            .map(|t| t.coefficient + dot(&t.exponents, x))
            .fold(f64::NEG_INFINITY, f64::max)
    }

    /// Find the tropical hypersurface (decision boundary).
    /// The hypersurface is the set of points where the maximum
    /// is achieved by at least two terms simultaneously.
    pub fn hypersurface_test(&self, x: &[f64]) -> bool {
        let values: Vec<f64> = self.terms.iter()
            .map(|t| t.coefficient + dot(&t.exponents, x))
            .collect();
        let max_val = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let count = values.iter().filter(|&&v| (v - max_val).abs() < 1e-10).count();
        count >= 2  // on the hypersurface iff 2+ terms tie for maximum
    }
}
```

### Tropical convexity for prediction regions

```rust
/// Tropical convex hull of oracle prediction prototypes.
///
/// The tropical convex hull of points p₁, ..., pₙ is:
///   tconv(p₁,...,pₙ) = { max_i(c_i + p_i) : c_i ∈ ℝ, ⊕_i c_i = max(c_i) = 0 }
///
/// Tropical convexity has properties different from classical convexity:
/// - Tropical line segments are piecewise-linear paths
/// - Tropical convex sets can be non-convex in the classical sense
/// - The tropical convex hull of d+1 points in ℝ^d is a tropical polytope
///   whose combinatorial type classifies the point configuration
///
/// Application: each oracle's prediction space is tropically convex.
/// The tropical convex hull of successful prediction prototypes defines
/// the oracle's "competence region" in a way that respects the
/// max-plus structure of the prediction mechanism.
///
/// (Develin & Sturmfels, 2004; developments survey: arXiv:2405.17005)
pub struct TropicalConvexHull {
    /// Generator points.
    pub generators: Vec<Vec<f64>>,
    /// Dimension of the ambient space.
    pub dim: usize,
}

impl TropicalConvexHull {
    /// Test membership in the tropical convex hull.
    ///
    /// A point x is in tconv(p₁,...,pₙ) iff there exist c₁,...,cₙ ∈ ℝ
    /// with max(c_i) = 0 such that x_j = max_i(c_i + p_{i,j}) for all j.
    ///
    /// Solvable as a linear feasibility problem in O(n·d) variables.
    pub fn contains(&self, x: &[f64]) -> bool {
        // Formulate as: find c s.t. max(c) = 0 and
        // for all j: x_j = max_i(c_i + generators[i][j])
        // This is a tropical linear system.
        tropical_feasibility(&self.generators, x)
    }
}
```

### Tropical attention for symbolic-neural fusion

```rust
/// Tropical attention: attention mechanism native to max-plus semiring.
///
/// Standard attention: softmax(QK^T / √d) V
/// Tropical attention: max-plus(Q ⊗ K^T) ⊗ V
///
/// Where ⊗ is tropical matrix multiplication:
///   (A ⊗ B)_{ij} = max_k(A_{ik} + B_{kj})
///
/// Key property (arXiv:2505.17190, 2025):
/// Tropical attention directly approximates dynamic programming
/// algorithms (shortest paths, Viterbi, CKY parsing).
/// This creates a principled bridge between:
/// - Symbolic planning (DAG executor in roko-orchestrator)
/// - Neural scoring (oracle prediction networks)
///
/// Application to Roko:
/// When the plan DAG executor chooses which task to execute next,
/// the scoring function is naturally a tropical polynomial.
/// Tropical attention learns optimal task selection from execution history.
pub struct TropicalAttention {
    /// Query projection dimension.
    pub d_k: usize,                 // default: 64
    /// Value projection dimension.
    pub d_v: usize,                 // default: 64
    /// Number of tropical attention heads.
    pub n_heads: usize,             // default: 4
    /// Temperature for soft-max approximation (→0 recovers exact max).
    pub temperature: f64,            // default: 0.1
}

impl TropicalAttention {
    /// Tropical attention forward pass.
    ///
    /// Given queries Q, keys K, values V:
    ///   Attention(Q,K,V) = softmax(Q ⊗ K^T / τ) · V
    ///
    /// where ⊗ is tropical matmul and τ → 0 recovers exact max-plus.
    pub fn forward(
        &self,
        queries: &[Vec<f64>],
        keys: &[Vec<f64>],
        values: &[Vec<f64>],
    ) -> Vec<Vec<f64>> {
        // Tropical matmul: (Q ⊗ K^T)_{ij} = max_k(Q_{ik} + K_{jk})
        let scores: Vec<Vec<f64>> = queries.iter().map(|q| {
            keys.iter().map(|k| {
                q.iter().zip(k.iter())
                    .map(|(qi, ki)| qi + ki)
                    .fold(f64::NEG_INFINITY, f64::max)
            }).collect()
        }).collect();

        // Soft-max approximation with temperature
        let weights = softmax_2d(&scores, self.temperature);

        // Weighted sum of values
        mat_mul(&weights, values)
    }
}
```

### Tropical robustness analysis

```rust
/// Tropical geometry reveals adversarial vulnerability structure.
///
/// Zhang et al. (2018, ICML) and subsequent work (arXiv:2402.00576, 2024)
/// show that:
/// 1. Decision boundaries of ReLU-based oracles are tropical hypersurfaces
/// 2. Adversarial examples live on or near these hypersurfaces
/// 3. The NUMBER of linear regions in a tropical polynomial
///    correlates with adversarial robustness (more regions → more robust)
///
/// This connects to certified robustness (doc 11):
/// - Lipschitz constant L of a tropical polynomial is the maximum
///   slope across all linear regions
/// - Certification radius R = margin / L
/// - Tropical geometry provides an exact characterization of L
///   (not just an upper bound as in spectral norm methods)
pub struct TropicalRobustnessAnalyzer {
    /// The oracle's prediction function as a tropical polynomial.
    pub policy: TropicalPolynomial,
}

impl TropicalRobustnessAnalyzer {
    /// Count the number of linear regions (combinatorial complexity).
    pub fn count_linear_regions(&self) -> usize {
        // The number of linear regions equals the number of cells
        // in the tropical hyperplane arrangement.
        self.policy.terms.len()  // upper bound; exact count requires arrangement computation
    }

    /// Compute exact Lipschitz constant from the tropical polynomial.
    ///
    /// L = max over all linear regions of ||gradient||
    /// For a tropical polynomial f(x) = max_i(c_i + a_i·x),
    /// the gradient in region i is a_i, so L = max_i ||a_i||.
    pub fn exact_lipschitz(&self) -> f64 {
        self.policy.terms.iter()
            .map(|t| norm(&t.exponents))
            .fold(0.0f64, f64::max)
    }

    /// Find the minimum distance from a point to the tropical hypersurface.
    /// This is the EXACT adversarial perturbation distance
    /// (not a bound — the true minimum distance to a decision boundary).
    pub fn distance_to_boundary(&self, x: &[f64]) -> f64 {
        let values: Vec<f64> = self.policy.terms.iter()
            .map(|t| t.coefficient + dot(&t.exponents, x))
            .collect();
        let mut sorted = values.clone();
        sorted.sort_by(|a, b| b.partial_cmp(a).unwrap());

        if sorted.len() < 2 {
            return f64::MAX;
        }

        // Distance to boundary ≈ (best score - second best) / ||gradient_diff||
        let margin = sorted[0] - sorted[1];
        let best_idx = values.iter().position(|&v| (v - sorted[0]).abs() < 1e-12).unwrap();
        let second_idx = values.iter().position(|&v| (v - sorted[1]).abs() < 1e-12).unwrap();
        let grad_diff = subtract_vec(
            &self.policy.terms[best_idx].exponents,
            &self.policy.terms[second_idx].exponents,
        );
        let grad_norm = norm(&grad_diff);
        if grad_norm < 1e-12 { f64::MAX } else { margin / grad_norm }
    }
}
```

### Tropical VCG auction theory

```rust
/// Tropical geometry in the VCG attention auction.
///
/// The VCG auction used by the Composer (doc 00, doc 10) for context
/// allocation is a product-mix auction. Recent work shows that
/// product-mix auctions are fundamentally tropical-geometric objects:
///
/// - Bidder valuations form tropical polynomials
/// - Competitive equilibrium prices lie on tropical hypersurfaces
/// - The set of Walrasian equilibria is a tropical polytope
///
/// (Baldwin & Klemperer, 2019, "Understanding Preferences: 'Demand Types',
///  and the Existence of Equilibrium with Indivisibilities";
///  Tran & Yu, 2019, "Product-Mix Auctions and Tropical Geometry", MOR)
///
/// This means Roko's VCG context auction has a tropical structure:
/// the equilibrium prices for context window slots are solutions
/// to a tropical linear system. Computing them via tropical methods
/// is O(n·k) where n = bidders and k = slots, faster than general
/// VCG computation.
pub struct TropicalAuction {
    /// Bidder valuations as tropical polynomials.
    pub bidder_valuations: Vec<TropicalPolynomial>,
    /// Number of available slots.
    pub n_slots: usize,
}

impl TropicalAuction {
    /// Find competitive equilibrium prices via tropical linear algebra.
    pub fn equilibrium_prices(&self) -> Vec<f64> {
        // Solve the tropical linear system for equilibrium
        // Uses tropical Cramer's rule (Richter-Gebert et al., 2005)
        tropical_linear_solve(&self.bidder_valuations, self.n_slots)
    }
}
```

---

## Integration: Sheaf + Tropical + Existing Architecture

### Connecting sheaf consistency to IIT Phi

The sheaf Laplacian eigenvalues provide a principled replacement for the brute-force Phi computation (doc 12):

```
IIT Phi (doc 12):           Sheaf cohomology (this doc):
510 bipartitions enumerated  →  dim(H^1(G, F)) computed via Smith normal form
O(2^9) cost                  →  O(d^3) where d = total stalk dimension
Phi = min(ΔI) over partitions →  β_1 = number of independent obstructions
```

When β_1 = 0 (no obstructions), the TA subsystems are guaranteed to have a globally consistent prediction — a stronger statement than "Phi is high."

### Connecting tropical geometry to adversarial robustness

Tropical analysis (this doc) complements the certified robustness methods (doc 11):

```
Certified robustness (doc 11):        Tropical analysis (this doc):
Randomized smoothing: statistical     →  Exact boundary distance: deterministic
Lipschitz bounds: upper bound on L    →  Exact L from tropical polynomial structure
IBP: interval over-approximation      →  Exact linear region enumeration
```

Tropical methods provide EXACT adversarial distances (not bounds), but only for piecewise-linear oracle functions. For neural oracles, the tropical analysis applies to the last layer (softmax/argmax) exactly.

---

## Configuration parameters

| Parameter | Default | Range | Notes |
|---|---|---|---|
| Sheaf: `n_diffusion_steps` | 5 | 1-20 | More steps = smoother but may over-smooth |
| Sheaf: `sigma` (diffusion rate) | 0.1 | 0.01-1.0 | Higher = faster convergence, risk of instability |
| Sheaf: `restriction_hidden_dim` | 32 | 8-128 | Capacity of learned restriction maps |
| Tropical: `temperature` | 0.1 | 0.01-1.0 | Lower = closer to exact max-plus |
| Tropical: `n_heads` | 4 | 1-8 | Tropical attention heads |

---

## Test criteria

- **Sheaf consistency**: For a section with δs = 0, `inconsistency()` returns 0.0 within f64 epsilon.
- **Laplacian positive semidefiniteness**: All eigenvalues of L_F are ≥ 0.
- **Cohomology dimension**: For a connected graph with trivial sheaf (all ρ = identity), β_0 = 1.
- **Tropical evaluation**: For f(x) = max(3 + 2x, 1 + 4x), f(1) = max(5, 5) = 5.
- **Tropical hypersurface detection**: At x=1 in the above example, `hypersurface_test` returns true.
- **Exact Lipschitz**: For f(x) = max(2x, 4x), exact_lipschitz() = 4.0.
- **Tropical attention**: With temperature → 0, output converges to the value vector with highest tropical score.
- **Sheaf-IIT agreement**: When β_1 = 0, Phi > 0 (global consistency implies integration).

---

## Academic foundations

- Hansen, J., & Ghrist, R. (2019). "Toward a Spectral Theory of Cellular Sheaves." *Journal of Applied and Computational Topology*, 3, 315-358. — Sheaf Laplacian spectral theory.
- Bodnar, C., et al. (2022). "Neural Sheaf Diffusion: A Topological Perspective on Heterophily and Oversmoothing in GNNs." arXiv:2202.04579. — Learned sheaf neural networks.
- Curry, J. (2014). "Sheaves, Cosheaves and Applications." arXiv:1303.3255. — Computational sheaf cohomology.
- Bredon, G. E. (1997). *Sheaf Theory*. 2nd ed. Springer. — Standard reference.
- Robinson, M. (2014). *Topological Signal Processing*. Springer. — Sheaves for signal processing.
- Gebhart, T., Schrater, P., & Hylton, A. (2023). "Knowledge Sheaves: A Sheaf-Theoretic Framework for Knowledge Graph Embedding." *PMLR 206*. — Knowledge representation via sheaves.
- Zhang, L., Naitzat, G., & Lim, L.-H. (2018). "Tropical Geometry of Deep Neural Networks." *ICML 2018*. — Tropical decision boundaries.
- Alfarra, M., et al. (2024). "Tropical Decision Boundaries for Neural Networks Are Robust Against Adversarial Attacks." arXiv:2402.00576. — Tropical adversarial robustness.
- Tran, N. M., & Yu, J. (2019). "Product-Mix Auctions and Tropical Geometry." *Mathematics of Operations Research*, 44(4). — Tropical auction theory.
- Maragos, P. (2024). "Tropical Geometry for Machine Learning and Optimization." *ICASSP 2024 Tutorial*. — Comprehensive tropical ML survey.
- arXiv:2505.17190 (2025). "Tropical Attention: Neural Algorithmic Reasoning for Combinatorial Algorithms." — Tropical attention mechanism.
- Develin, M., & Sturmfels, B. (2004). "Tropical Convexity." *Documenta Mathematica*, 9, 1-27. — Tropical convex hull foundations.

---

## Cross-References

- See [06-hyperdimensional-ta.md](./06-hyperdimensional-ta.md) for HDC pattern encoding that sheaf sections encode
- See [07-spectral-liquidity-manifolds.md](./07-spectral-liquidity-manifolds.md) for Riemannian geometry complementing information geometry
- See [10-predictive-geometry-and-resonant-patterns.md](./10-predictive-geometry-and-resonant-patterns.md) for TDA (topology from different angle than sheaves)
- See [11-adversarial-signal-robustness.md](./11-adversarial-signal-robustness.md) for certified robustness methods complemented by tropical analysis
- See [12-somatic-ta-and-emergent-multiscale.md](./12-somatic-ta-and-emergent-multiscale.md) for IIT Phi replaced by sheaf cohomology
- See [13-predictive-foraging-and-active-inference.md](./13-predictive-foraging-and-active-inference.md) for VCG auction with tropical structure
