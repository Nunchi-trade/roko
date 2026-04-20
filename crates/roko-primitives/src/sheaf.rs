//! Cellular sheaves for oracle consistency checking (TA-13).
//!
//! A **cellular sheaf** on a graph `G = (V, E)` assigns a vector space (stalk)
//! to each vertex and a linear restriction map to each edge. The sheaf
//! Laplacian `L_F = delta^T delta` measures local-to-global inconsistency:
//!
//! - `lambda_min(L_F) = 0` means a global section exists (perfect consistency).
//! - `lambda_min(L_F) > 0` means the predictions disagree.
//!
//! # Application
//!
//! When multiple oracles (chain, coding, research) produce predictions, we
//! check whether their predictions form a consistent global section on the
//! sheaf. The eigenvector of the smallest eigenvalue of the Laplacian
//! identifies which oracle is most inconsistent.
//!
//! # References
//!
//! - Hansen, J. & Ghrist, R. (2019). Toward a spectral theory of cellular sheaves.
//! - Curry, J. (2014). Sheaves, cosheaves and applications.
//! - Robinson, M. (2014). *Topological Signal Processing*.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Identifier for a vertex (oracle) in the consistency graph.
pub type NodeId = u32;

/// Identifier for an edge in the consistency graph.
///
/// An edge connects two oracles whose predictions should be compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EdgeId(pub NodeId, pub NodeId);

/// A linear restriction map represented as a dense matrix.
///
/// Maps from a stalk (vector space) at a vertex to the edge space.
/// Stored as row-major `rows x cols` matrix.
#[derive(Debug, Clone, PartialEq)]
pub struct RestrictionMap {
    /// Number of rows (dimension of the edge stalk).
    pub rows: usize,
    /// Number of columns (dimension of the vertex stalk).
    pub cols: usize,
    /// Row-major matrix data.
    pub data: Vec<f64>,
}

impl RestrictionMap {
    /// Create a new restriction map.
    ///
    /// `data` is row-major with dimensions `rows x cols`.
    pub fn new(rows: usize, cols: usize, data: Vec<f64>) -> Self {
        assert_eq!(data.len(), rows * cols, "data length mismatch");
        Self { rows, cols, data }
    }

    /// Create an identity restriction map (projection is identity).
    pub fn identity(dim: usize) -> Self {
        let mut data = vec![0.0; dim * dim];
        for i in 0..dim {
            data[i * dim + i] = 1.0;
        }
        Self {
            rows: dim,
            cols: dim,
            data,
        }
    }

    /// Create a projection restriction map that selects specific components.
    ///
    /// `indices` specifies which components of the vertex stalk to project
    /// onto the edge stalk.
    pub fn projection(vertex_dim: usize, indices: &[usize]) -> Self {
        let edge_dim = indices.len();
        let mut data = vec![0.0; edge_dim * vertex_dim];
        for (row, &col) in indices.iter().enumerate() {
            assert!(
                col < vertex_dim,
                "index {col} out of bounds for dim {vertex_dim}"
            );
            data[row * vertex_dim + col] = 1.0;
        }
        Self {
            rows: edge_dim,
            cols: vertex_dim,
            data,
        }
    }

    /// Apply this restriction map to a vector.
    ///
    /// Returns `M * v` where M is this matrix and v is the input vector.
    pub fn apply(&self, v: &[f64]) -> Vec<f64> {
        assert_eq!(v.len(), self.cols, "vector length mismatch");
        let mut result = vec![0.0; self.rows];
        for i in 0..self.rows {
            for j in 0..self.cols {
                result[i] += self.data[i * self.cols + j] * v[j];
            }
        }
        result
    }

    /// Compute the transpose of this map.
    pub fn transpose(&self) -> RestrictionMap {
        let mut data = vec![0.0; self.cols * self.rows];
        for i in 0..self.rows {
            for j in 0..self.cols {
                data[j * self.rows + i] = self.data[i * self.cols + j];
            }
        }
        RestrictionMap {
            rows: self.cols,
            cols: self.rows,
            data,
        }
    }
}

/// An edge in the sheaf with its restriction maps from both endpoints.
#[derive(Debug, Clone)]
struct SheafEdge {
    /// Source vertex.
    src: NodeId,
    /// Target vertex.
    tgt: NodeId,
    /// Restriction map from source stalk to edge stalk: F(src) -> F(e).
    map_src: RestrictionMap,
    /// Restriction map from target stalk to edge stalk: F(tgt) -> F(e).
    map_tgt: RestrictionMap,
}

/// A cellular sheaf on a graph.
///
/// Each vertex has a stalk (vector space dimension) and each edge has
/// restriction maps from both endpoint stalks to a shared edge stalk.
///
/// The sheaf Laplacian quantifies the inconsistency of a set of
/// "local sections" (predictions at each vertex).
#[derive(Debug, Clone)]
pub struct CellularSheaf {
    /// Stalk dimensions: node_id -> dimension of the vector space at that node.
    stalk_dims: HashMap<NodeId, usize>,
    /// Edges with restriction maps.
    edges: Vec<SheafEdge>,
}

impl CellularSheaf {
    /// Create an empty sheaf.
    #[must_use]
    pub fn new() -> Self {
        Self {
            stalk_dims: HashMap::new(),
            edges: Vec::new(),
        }
    }

    /// Add a vertex (oracle) with a given stalk dimension.
    ///
    /// The stalk dimension is the number of prediction components
    /// (e.g., 4 for [price, volume, gas, risk]).
    pub fn add_vertex(&mut self, node: NodeId, dim: usize) {
        self.stalk_dims.insert(node, dim);
    }

    /// Add an edge with restriction maps from both endpoints.
    ///
    /// The restriction maps project from each endpoint's stalk into a
    /// shared comparison space. The consistency check verifies that both
    /// projections agree.
    pub fn add_edge(
        &mut self,
        src: NodeId,
        tgt: NodeId,
        map_src: RestrictionMap,
        map_tgt: RestrictionMap,
    ) {
        assert_eq!(
            map_src.rows, map_tgt.rows,
            "restriction maps must have same edge dimension"
        );
        self.edges.push(SheafEdge {
            src,
            tgt,
            map_src,
            map_tgt,
        });
    }

    /// Add an edge where both restriction maps are identity (same stalk dimension).
    ///
    /// This is the simplest case: both oracles produce predictions in the
    /// same space and we compare them directly.
    pub fn add_identity_edge(&mut self, src: NodeId, tgt: NodeId) {
        let src_dim = self
            .stalk_dims
            .get(&src)
            .copied()
            .expect("source vertex not found");
        let tgt_dim = self
            .stalk_dims
            .get(&tgt)
            .copied()
            .expect("target vertex not found");
        assert_eq!(
            src_dim, tgt_dim,
            "identity edge requires equal stalk dimensions"
        );
        let id = RestrictionMap::identity(src_dim);
        self.add_edge(src, tgt, id.clone(), id);
    }

    /// Number of vertices.
    #[must_use]
    pub fn num_vertices(&self) -> usize {
        self.stalk_dims.len()
    }

    /// Number of edges.
    #[must_use]
    pub fn num_edges(&self) -> usize {
        self.edges.len()
    }

    /// Total dimension of the vertex stalk space (sum of all stalk dims).
    #[must_use]
    pub fn total_vertex_dim(&self) -> usize {
        self.stalk_dims.values().sum()
    }

    /// Ordered list of vertex IDs (sorted for consistent indexing).
    fn ordered_nodes(&self) -> Vec<NodeId> {
        let mut nodes: Vec<NodeId> = self.stalk_dims.keys().copied().collect();
        nodes.sort();
        nodes
    }

    /// Compute the byte offset of a vertex's stalk in the flattened vector.
    fn stalk_offset(&self, nodes: &[NodeId], node: NodeId) -> usize {
        let mut offset = 0;
        for &n in nodes {
            if n == node {
                return offset;
            }
            offset += self.stalk_dims[&n];
        }
        panic!("node {node} not found");
    }

    /// Compute the coboundary operator delta.
    ///
    /// The coboundary maps vertex sections to edge discrepancies:
    /// `(delta s)_e = F_{e,tgt}(s_tgt) - F_{e,src}(s_src)`
    ///
    /// Returns the coboundary as a dense matrix (total_edge_dim x total_vertex_dim).
    fn coboundary_matrix(&self) -> (Vec<f64>, usize, usize) {
        let nodes = self.ordered_nodes();
        let n_cols = self.total_vertex_dim();

        // Total edge stalk dimension.
        let n_rows: usize = self.edges.iter().map(|e| e.map_src.rows).sum();

        let mut matrix = vec![0.0; n_rows * n_cols];

        let mut row_offset = 0;
        for edge in &self.edges {
            let edge_dim = edge.map_src.rows;
            let src_offset = self.stalk_offset(&nodes, edge.src);
            let tgt_offset = self.stalk_offset(&nodes, edge.tgt);
            let src_dim = self.stalk_dims[&edge.src];
            let tgt_dim = self.stalk_dims[&edge.tgt];

            for i in 0..edge_dim {
                // -F_{e,src}: negate source restriction.
                for j in 0..src_dim {
                    matrix[(row_offset + i) * n_cols + (src_offset + j)] -=
                        edge.map_src.data[i * edge.map_src.cols + j];
                }
                // +F_{e,tgt}: positive target restriction.
                for j in 0..tgt_dim {
                    matrix[(row_offset + i) * n_cols + (tgt_offset + j)] +=
                        edge.map_tgt.data[i * edge.map_tgt.cols + j];
                }
            }

            row_offset += edge_dim;
        }

        (matrix, n_rows, n_cols)
    }

    /// Compute the sheaf Laplacian `L_F = delta^T * delta`.
    ///
    /// Returns a `total_vertex_dim x total_vertex_dim` matrix.
    #[must_use]
    pub fn laplacian(&self) -> (Vec<f64>, usize) {
        let (delta, n_rows, n_cols) = self.coboundary_matrix();

        // L = delta^T * delta (n_cols x n_cols matrix).
        let mut lap = vec![0.0; n_cols * n_cols];
        for i in 0..n_cols {
            for j in 0..n_cols {
                let mut sum = 0.0;
                for k in 0..n_rows {
                    sum += delta[k * n_cols + i] * delta[k * n_cols + j];
                }
                lap[i * n_cols + j] = sum;
            }
        }

        (lap, n_cols)
    }

    /// Compute the inconsistency score for a set of predictions.
    ///
    /// The predictions are provided as a map from node ID to prediction vector.
    /// The score is `||delta s||^2 / ||s||^2` where `s` is the section and
    /// `delta` is the coboundary operator.
    ///
    /// - Score = 0: perfectly consistent.
    /// - Score > 0: inconsistent (higher = more disagreement).
    ///
    /// Returns `None` if the section is zero or nodes are missing.
    #[must_use]
    pub fn inconsistency_score(&self, predictions: &HashMap<NodeId, Vec<f64>>) -> Option<f64> {
        let nodes = self.ordered_nodes();
        let n = self.total_vertex_dim();

        // Flatten predictions into a single vector.
        let mut section = vec![0.0; n];
        for &node in &nodes {
            let pred = predictions.get(&node)?;
            let offset = self.stalk_offset(&nodes, node);
            let dim = self.stalk_dims[&node];
            if pred.len() != dim {
                return None;
            }
            for (i, &val) in pred.iter().enumerate() {
                section[offset + i] = val;
            }
        }

        // Compute ||s||^2.
        let s_norm_sq: f64 = section.iter().map(|x| x * x).sum();
        if s_norm_sq < 1e-15 {
            return None;
        }

        // Compute delta * s.
        let (delta, n_rows, n_cols) = self.coboundary_matrix();
        let mut ds = vec![0.0; n_rows];
        for i in 0..n_rows {
            for j in 0..n_cols {
                ds[i] += delta[i * n_cols + j] * section[j];
            }
        }

        // ||delta s||^2.
        let ds_norm_sq: f64 = ds.iter().map(|x| x * x).sum();

        Some(ds_norm_sq / s_norm_sq)
    }

    /// Identify the most inconsistent oracle (vertex).
    ///
    /// Computes the coboundary and returns the node whose predictions
    /// contribute most to the total inconsistency. This is measured by the
    /// per-vertex contribution to `||delta s||^2`.
    ///
    /// Returns `(node_id, contribution_fraction)` or `None` if all consistent.
    #[must_use]
    pub fn most_inconsistent(
        &self,
        predictions: &HashMap<NodeId, Vec<f64>>,
    ) -> Option<(NodeId, f64)> {
        let nodes = self.ordered_nodes();
        let n = self.total_vertex_dim();

        // Flatten predictions.
        let mut section = vec![0.0; n];
        for &node in &nodes {
            let pred = predictions.get(&node)?;
            let offset = self.stalk_offset(&nodes, node);
            for (i, &val) in pred.iter().enumerate() {
                section[offset + i] = val;
            }
        }

        // Compute Laplacian contribution per vertex: (L_F s)_v . s_v
        let (lap, dim) = self.laplacian();
        if dim == 0 {
            return None;
        }

        // L * s
        let mut ls = vec![0.0; dim];
        for i in 0..dim {
            for j in 0..dim {
                ls[i] += lap[i * dim + j] * section[j];
            }
        }

        // Per-vertex contribution: sum of (L*s)_i * s_i over stalk indices.
        let mut vertex_contributions: Vec<(NodeId, f64)> = Vec::new();
        let mut total_contribution = 0.0;

        for &node in &nodes {
            let offset = self.stalk_offset(&nodes, node);
            let d = self.stalk_dims[&node];
            let mut contrib = 0.0;
            for i in 0..d {
                contrib += ls[offset + i] * section[offset + i];
            }
            contrib = contrib.abs();
            total_contribution += contrib;
            vertex_contributions.push((node, contrib));
        }

        if total_contribution < 1e-15 {
            return None;
        }

        // Return the vertex with highest contribution.
        vertex_contributions
            .into_iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(node, c)| (node, c / total_contribution))
    }

    /// Compute the smallest eigenvalue of the sheaf Laplacian via power iteration
    /// on `(L - lambda_max I)` (inverse iteration approximation).
    ///
    /// Since direct eigendecomposition of small matrices (typically 4-12 dimensional)
    /// is sufficient, we use the Rayleigh quotient iteration approach.
    ///
    /// Returns the smallest eigenvalue. A value near 0 means the predictions
    /// are nearly consistent.
    #[must_use]
    pub fn min_eigenvalue(&self) -> f64 {
        let (lap, dim) = self.laplacian();
        if dim == 0 {
            return 0.0;
        }

        smallest_eigenvalue(&lap, dim, 200, 1e-10)
    }
}

impl Default for CellularSheaf {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Eigenvalue computation (power method on shifted matrix)
// ---------------------------------------------------------------------------

/// Compute the smallest eigenvalue of a symmetric PSD matrix.
///
/// Uses the inverse power method: repeatedly solving `(L - mu I)^{-1} x = y`
/// where mu is a shift close to 0. For small matrices we do a direct approach.
///
/// For the sheaf Laplacian (which is always PSD), the smallest eigenvalue
/// is >= 0.
fn smallest_eigenvalue(matrix: &[f64], dim: usize, max_iter: usize, tol: f64) -> f64 {
    if dim == 1 {
        return matrix[0];
    }

    // First, find an estimate of the largest eigenvalue via power iteration.
    let lambda_max = largest_eigenvalue(matrix, dim, max_iter, tol);
    if lambda_max.abs() < tol {
        return 0.0;
    }

    // Compute B = lambda_max * I - L, then find largest eigenvalue of B.
    // lambda_min(L) = lambda_max(L) - lambda_max(B).
    let mut shifted = vec![0.0; dim * dim];
    for i in 0..dim {
        for j in 0..dim {
            shifted[i * dim + j] = -matrix[i * dim + j];
        }
        shifted[i * dim + i] += lambda_max;
    }

    let lambda_max_shifted = largest_eigenvalue(&shifted, dim, max_iter, tol);
    let lambda_min = lambda_max - lambda_max_shifted;

    // Clamp to non-negative (sheaf Laplacian is PSD).
    lambda_min.max(0.0)
}

/// Compute the largest eigenvalue of a symmetric matrix via power iteration.
fn largest_eigenvalue(matrix: &[f64], dim: usize, max_iter: usize, tol: f64) -> f64 {
    // Start with a non-zero vector.
    let mut v: Vec<f64> = (0..dim).map(|i| 1.0 / ((i + 1) as f64)).collect();
    normalize(&mut v);

    let mut eigenvalue = 0.0;

    for _ in 0..max_iter {
        // w = M * v
        let mut w = vec![0.0; dim];
        for i in 0..dim {
            for j in 0..dim {
                w[i] += matrix[i * dim + j] * v[j];
            }
        }

        // Rayleigh quotient: lambda = v^T w / v^T v = v^T w (since v is normalized).
        let new_eigenvalue: f64 = v.iter().zip(w.iter()).map(|(a, b)| a * b).sum();

        normalize(&mut w);
        v = w;

        if (new_eigenvalue - eigenvalue).abs() < tol {
            return new_eigenvalue;
        }
        eigenvalue = new_eigenvalue;
    }

    eigenvalue
}

/// Normalize a vector to unit length.
fn normalize(v: &mut [f64]) {
    let norm: f64 = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm > 1e-15 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_simple_sheaf() -> CellularSheaf {
        // Three oracles, each producing 2D predictions.
        // Edges compare all pairs.
        let mut sheaf = CellularSheaf::new();
        sheaf.add_vertex(0, 2); // Oracle A: 2D predictions
        sheaf.add_vertex(1, 2); // Oracle B: 2D predictions
        sheaf.add_vertex(2, 2); // Oracle C: 2D predictions
        sheaf.add_identity_edge(0, 1);
        sheaf.add_identity_edge(1, 2);
        sheaf.add_identity_edge(0, 2);
        sheaf
    }

    // --- Sheaf structure ---

    #[test]
    fn sheaf_construction() {
        let sheaf = make_simple_sheaf();
        assert_eq!(sheaf.num_vertices(), 3);
        assert_eq!(sheaf.num_edges(), 3);
        assert_eq!(sheaf.total_vertex_dim(), 6);
    }

    #[test]
    fn restriction_map_identity_applies_correctly() {
        let id = RestrictionMap::identity(3);
        let v = vec![1.0, 2.0, 3.0];
        let result = id.apply(&v);
        assert_eq!(result, v);
    }

    #[test]
    fn restriction_map_projection() {
        // Project 4D vector onto components 0 and 2.
        let proj = RestrictionMap::projection(4, &[0, 2]);
        let v = vec![10.0, 20.0, 30.0, 40.0];
        let result = proj.apply(&v);
        assert_eq!(result, vec![10.0, 30.0]);
    }

    #[test]
    fn restriction_map_transpose() {
        let m = RestrictionMap::new(2, 3, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let mt = m.transpose();
        assert_eq!(mt.rows, 3);
        assert_eq!(mt.cols, 2);
        // Row 0 of transpose = col 0 of original.
        assert_eq!(mt.data[0], 1.0); // mt[0][0]
        assert_eq!(mt.data[1], 4.0); // mt[0][1]
        assert_eq!(mt.data[2], 2.0); // mt[1][0]
        assert_eq!(mt.data[3], 5.0); // mt[1][1]
    }

    // --- Laplacian ---

    #[test]
    fn laplacian_dimensions() {
        let sheaf = make_simple_sheaf();
        let (lap, dim) = sheaf.laplacian();
        assert_eq!(dim, 6); // 3 vertices * 2D stalks
        assert_eq!(lap.len(), 36); // 6x6 matrix
    }

    #[test]
    fn laplacian_is_symmetric() {
        let sheaf = make_simple_sheaf();
        let (lap, dim) = sheaf.laplacian();
        for i in 0..dim {
            for j in 0..dim {
                assert!(
                    (lap[i * dim + j] - lap[j * dim + i]).abs() < 1e-12,
                    "L[{i}][{j}] = {}, L[{j}][{i}] = {}",
                    lap[i * dim + j],
                    lap[j * dim + i]
                );
            }
        }
    }

    #[test]
    fn laplacian_is_positive_semidefinite() {
        let sheaf = make_simple_sheaf();
        let lambda_min = sheaf.min_eigenvalue();
        assert!(
            lambda_min >= -1e-8,
            "sheaf Laplacian should be PSD, lambda_min = {lambda_min}"
        );
    }

    // --- Consistency checking ---

    #[test]
    fn consistent_predictions_score_zero() {
        let sheaf = make_simple_sheaf();

        // All oracles agree: [1.0, 2.0].
        let mut predictions = HashMap::new();
        predictions.insert(0, vec![1.0, 2.0]);
        predictions.insert(1, vec![1.0, 2.0]);
        predictions.insert(2, vec![1.0, 2.0]);

        let score = sheaf.inconsistency_score(&predictions).unwrap();
        assert!(
            score < 1e-10,
            "consistent predictions should have score ≈ 0: {score}"
        );
    }

    #[test]
    fn inconsistent_predictions_positive_score() {
        let sheaf = make_simple_sheaf();

        // Oracle C disagrees with A and B.
        let mut predictions = HashMap::new();
        predictions.insert(0, vec![1.0, 2.0]);
        predictions.insert(1, vec![1.0, 2.0]);
        predictions.insert(2, vec![5.0, -3.0]); // Disagrees!

        let score = sheaf.inconsistency_score(&predictions).unwrap();
        assert!(
            score > 0.1,
            "inconsistent predictions should have positive score: {score}"
        );
    }

    #[test]
    fn most_inconsistent_identifies_outlier() {
        let sheaf = make_simple_sheaf();

        // Oracle C is the outlier.
        let mut predictions = HashMap::new();
        predictions.insert(0, vec![1.0, 1.0]);
        predictions.insert(1, vec![1.0, 1.0]);
        predictions.insert(2, vec![10.0, 10.0]); // Way off.

        let (node, fraction) = sheaf.most_inconsistent(&predictions).unwrap();
        assert_eq!(node, 2, "Oracle C (node 2) should be most inconsistent");
        assert!(
            fraction > 0.3,
            "outlier should have high contribution fraction: {fraction}"
        );
    }

    #[test]
    fn consistent_predictions_most_inconsistent_is_none() {
        let sheaf = make_simple_sheaf();

        let mut predictions = HashMap::new();
        predictions.insert(0, vec![1.0, 2.0]);
        predictions.insert(1, vec![1.0, 2.0]);
        predictions.insert(2, vec![1.0, 2.0]);

        // All consistent => no inconsistent vertex.
        assert!(sheaf.most_inconsistent(&predictions).is_none());
    }

    // --- Eigenvalue computation ---

    #[test]
    fn min_eigenvalue_consistent_sheaf() {
        let sheaf = make_simple_sheaf();
        let lambda = sheaf.min_eigenvalue();
        // The min eigenvalue of the Laplacian should be 0 if the sheaf
        // has a nontrivial kernel (which identity-edge sheaves do).
        // For a complete graph with identity maps, the kernel is the
        // "constant section" space.
        assert!(
            lambda < 0.1,
            "min eigenvalue should be near 0 for complete sheaf: {lambda}"
        );
    }

    #[test]
    fn min_eigenvalue_single_vertex() {
        let mut sheaf = CellularSheaf::new();
        sheaf.add_vertex(0, 3);
        // No edges -> Laplacian is zero.
        let (lap, dim) = sheaf.laplacian();
        assert_eq!(dim, 3);
        assert!(lap.iter().all(|&x| x.abs() < 1e-15));
    }

    // --- Projection restriction maps ---

    #[test]
    fn sheaf_with_projection_maps() {
        // Oracle A: 4D predictions [price, volume, gas, risk]
        // Oracle B: 2D predictions [price, risk]
        // Edge compares the overlapping components.
        let mut sheaf = CellularSheaf::new();
        sheaf.add_vertex(0, 4);
        sheaf.add_vertex(1, 2);

        // Project A onto [price, risk] (indices 0, 3).
        let map_a = RestrictionMap::projection(4, &[0, 3]);
        // B's full prediction is already in [price, risk].
        let map_b = RestrictionMap::identity(2);
        sheaf.add_edge(0, 1, map_a, map_b);

        // Consistent: A's price and risk match B.
        let mut preds = HashMap::new();
        preds.insert(0, vec![10.0, 20.0, 30.0, 40.0]); // price=10, risk=40
        preds.insert(1, vec![10.0, 40.0]); // price=10, risk=40

        let score = sheaf.inconsistency_score(&preds).unwrap();
        assert!(
            score < 1e-10,
            "projected consistent predictions should score ≈ 0: {score}"
        );

        // Inconsistent: B's risk disagrees.
        preds.insert(1, vec![10.0, 0.0]); // risk disagrees
        let score = sheaf.inconsistency_score(&preds).unwrap();
        assert!(
            score > 0.01,
            "projected inconsistency should be positive: {score}"
        );
    }

    #[test]
    fn inconsistency_increases_with_disagreement() {
        let sheaf = make_simple_sheaf();

        let mut p1 = HashMap::new();
        p1.insert(0, vec![1.0, 1.0]);
        p1.insert(1, vec![1.0, 1.0]);
        p1.insert(2, vec![2.0, 2.0]); // Small disagreement.

        let mut p2 = HashMap::new();
        p2.insert(0, vec![1.0, 1.0]);
        p2.insert(1, vec![1.0, 1.0]);
        p2.insert(2, vec![10.0, 10.0]); // Large disagreement.

        let s1 = sheaf.inconsistency_score(&p1).unwrap();
        let s2 = sheaf.inconsistency_score(&p2).unwrap();
        assert!(
            s2 > s1,
            "larger disagreement should produce higher score: {s1} vs {s2}"
        );
    }
}
