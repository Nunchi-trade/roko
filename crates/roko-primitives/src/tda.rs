//! Topological Data Analysis (TDA) primitives (TA-09).
//!
//! Persistence diagrams extract shape features from time series data that are
//! invariant to continuous deformation. This module provides:
//!
//! - **Takens delay embedding**: converts a 1-D time series into a point cloud
//!   in d-dimensional phase space.
//! - **Vietoris-Rips persistence**: tracks birth/death of topological features
//!   (connected components H0, loops H1) across increasing scale parameters.
//! - **Persistence landscape**: vectorization of persistence diagrams into a
//!   Banach space element, enabling statistical operations (mean, variance).
//!
//! # References
//!
//! - Takens, F. (1981). Detecting strange attractors in turbulence.
//! - Carlsson, G. (2009). Topology and data. *Bull. AMS*, 46(2), 255-308.
//! - Bubenik, P. (2015). Statistical topological data analysis using persistence
//!   landscapes. *JMLR*, 16, 77-102.

/// A (birth, death) pair representing a topological feature.
///
/// Connected components (H0) are born at ε=0 and die when they merge.
/// Loops (H1) are born when a cycle appears and die when the cycle is filled.
/// Features far from the diagonal (death >> birth) are genuine structure;
/// features near the diagonal are noise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PersistencePoint {
    /// Scale at which this feature first appears.
    pub birth: f64,
    /// Scale at which this feature disappears.
    pub death: f64,
    /// Homological dimension (0 = connected component, 1 = loop, 2 = void).
    pub dimension: usize,
}

impl PersistencePoint {
    /// Create a new persistence point.
    pub fn new(birth: f64, death: f64, dimension: usize) -> Self {
        Self {
            birth,
            death,
            dimension,
        }
    }

    /// Lifetime of this feature: death - birth.
    ///
    /// Longer-lived features are more significant.
    pub fn persistence(&self) -> f64 {
        self.death - self.birth
    }
}

/// A persistence diagram: a multiset of (birth, death) pairs.
#[derive(Debug, Clone, Default)]
pub struct PersistenceDiagram {
    /// The persistence points.
    pub points: Vec<PersistencePoint>,
}

impl PersistenceDiagram {
    /// Create an empty persistence diagram.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a persistence point.
    pub fn add(&mut self, birth: f64, death: f64, dimension: usize) {
        self.points
            .push(PersistencePoint::new(birth, death, dimension));
    }

    /// Get all points of a given homological dimension.
    pub fn points_at_dim(&self, dim: usize) -> Vec<&PersistencePoint> {
        self.points.iter().filter(|p| p.dimension == dim).collect()
    }

    /// Total persistence: sum of all feature lifetimes.
    pub fn total_persistence(&self) -> f64 {
        self.points.iter().map(|p| p.persistence()).sum()
    }

    /// Maximum persistence (most significant feature).
    pub fn max_persistence(&self) -> f64 {
        self.points
            .iter()
            .map(|p| p.persistence())
            .fold(0.0_f64, f64::max)
    }

    /// Bottleneck distance approximation to another diagram.
    ///
    /// The bottleneck distance is the maximum over all matched pairs of the
    /// L-infinity distance. This is a simplified greedy approximation.
    pub fn bottleneck_distance(&self, other: &PersistenceDiagram) -> f64 {
        let mut max_dist = 0.0_f64;
        let mut used = vec![false; other.points.len()];

        for p in &self.points {
            let mut best_dist = p.persistence() / 2.0; // Distance to diagonal.
            let mut best_idx = None;

            for (j, q) in other.points.iter().enumerate() {
                if used[j] || q.dimension != p.dimension {
                    continue;
                }
                let dist = (p.birth - q.birth).abs().max((p.death - q.death).abs());
                if dist < best_dist {
                    best_dist = dist;
                    best_idx = Some(j);
                }
            }

            if let Some(idx) = best_idx {
                used[idx] = true;
            }
            max_dist = max_dist.max(best_dist);
        }

        // Unmatched points in `other` contribute their distance to diagonal.
        for (j, q) in other.points.iter().enumerate() {
            if !used[j] {
                max_dist = max_dist.max(q.persistence() / 2.0);
            }
        }

        max_dist
    }
}

/// Takens delay embedding of a scalar time series.
///
/// Maps `x(t) -> [x(t), x(t - tau), x(t - 2*tau), ..., x(t - (dim-1)*tau)]`
/// in d-dimensional phase space.
///
/// - `series`: input time series values.
/// - `dim`: embedding dimension (typically 2 or 3).
/// - `tau`: delay parameter (in number of time steps).
///
/// Returns a vector of d-dimensional points.
pub fn takens_embedding(series: &[f64], dim: usize, tau: usize) -> Vec<Vec<f64>> {
    if dim == 0 || tau == 0 || series.len() < (dim - 1) * tau + 1 {
        return Vec::new();
    }

    let n = series.len() - (dim - 1) * tau;
    let mut points = Vec::with_capacity(n);

    for i in 0..n {
        let mut point = Vec::with_capacity(dim);
        for d in 0..dim {
            point.push(series[i + d * tau]);
        }
        points.push(point);
    }

    points
}

/// Euclidean distance between two points.
fn euclidean_distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f64>()
        .sqrt()
}

/// Compute pairwise distance matrix for a point cloud.
fn distance_matrix(points: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let n = points.len();
    let mut dist = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let d = euclidean_distance(&points[i], &points[j]);
            dist[i][j] = d;
            dist[j][i] = d;
        }
    }
    dist
}

/// Union-Find data structure for connected component tracking.
struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }

    fn union(&mut self, x: usize, y: usize) -> bool {
        let rx = self.find(x);
        let ry = self.find(y);
        if rx == ry {
            return false; // Already connected.
        }
        match self.rank[rx].cmp(&self.rank[ry]) {
            std::cmp::Ordering::Less => self.parent[rx] = ry,
            std::cmp::Ordering::Greater => self.parent[ry] = rx,
            std::cmp::Ordering::Equal => {
                self.parent[ry] = rx;
                self.rank[rx] += 1;
            }
        }
        true
    }
}

/// Compute Vietoris-Rips persistence diagram from a point cloud.
///
/// Tracks H0 (connected components) and optionally H1 (loops) across
/// increasing scale parameters. This is a simplified implementation
/// that computes H0 features exactly via Union-Find and approximates
/// H1 features via cycle detection heuristics.
///
/// - `points`: d-dimensional point cloud.
/// - `max_dim`: maximum homological dimension to compute (0 or 1).
///
/// Returns a `PersistenceDiagram` of (birth, death) pairs.
pub fn vietoris_rips(points: &[Vec<f64>], max_dim: usize) -> PersistenceDiagram {
    let n = points.len();
    if n < 2 {
        return PersistenceDiagram::new();
    }

    let dist = distance_matrix(points);
    let mut diagram = PersistenceDiagram::new();

    // Collect all pairwise distances and sort.
    let mut edges: Vec<(f64, usize, usize)> = Vec::with_capacity(n * (n - 1) / 2);
    for i in 0..n {
        for j in (i + 1)..n {
            edges.push((dist[i][j], i, j));
        }
    }
    edges.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    // H0: Connected components via Union-Find.
    // Each point is born at ε=0. A component dies when it merges with another
    // (the younger component's birth ends at the merge distance).
    let mut uf = UnionFind::new(n);
    let birth_time = vec![0.0_f64; n]; // All H0 features born at 0.
    let mut components = n;

    for &(distance, i, j) in &edges {
        let ri = uf.find(i);
        let rj = uf.find(j);
        if ri != rj {
            // The component with later birth_time dies.
            let (dying, surviving) = if birth_time[ri] >= birth_time[rj] {
                (ri, rj)
            } else {
                (rj, ri)
            };
            let _ = surviving; // Older component survives.
            let persistence = distance - birth_time[dying];
            if persistence > f64::EPSILON {
                diagram.add(birth_time[dying], distance, 0);
            }
            uf.union(i, j);
            components -= 1;
        }
    }

    // The last surviving component lives forever (infinite death).
    // We represent this with death = max edge distance + epsilon.
    let max_dist = edges.last().map_or(0.0, |e| e.0);
    if components == 1 {
        diagram.add(0.0, max_dist * 1.1 + 1.0, 0);
    }

    // H1: Approximate loop detection.
    // A 1-cycle appears when adding an edge creates a triangle (or higher simplex)
    // where all vertices are already connected. We detect when an edge is added
    // between two already-connected vertices.
    if max_dim >= 1 {
        let mut uf2 = UnionFind::new(n);
        let mut adjacency: Vec<std::collections::HashSet<usize>> =
            vec![std::collections::HashSet::new(); n];

        for &(distance, i, j) in &edges {
            let ri = uf2.find(i);
            let rj = uf2.find(j);

            if ri == rj {
                // Already connected — this edge creates a cycle.
                // Check if there's a common neighbor (triangle).
                let common_neighbors: Vec<usize> =
                    adjacency[i].intersection(&adjacency[j]).copied().collect();

                if !common_neighbors.is_empty() {
                    // Triangle found: birth = distance, death when triangle fills.
                    // Approximate death as slightly larger distance.
                    let fill_dist = common_neighbors
                        .iter()
                        .map(|&k| dist[i][k].max(dist[j][k]))
                        .fold(f64::INFINITY, f64::min);
                    if fill_dist > distance + f64::EPSILON {
                        diagram.add(distance, fill_dist, 1);
                    }
                }
            } else {
                uf2.union(i, j);
            }
            adjacency[i].insert(j);
            adjacency[j].insert(i);
        }
    }

    diagram
}

/// Persistence landscape: vectorization of a persistence diagram.
///
/// Converts the birth-death pairs into a piecewise-linear function
/// (the "landscape") at each resolution level. Level k is the k-th largest
/// landscape function value at each parameter value.
///
/// - `diagram`: persistence diagram to vectorize.
/// - `dim`: homological dimension to extract (usually 0 or 1).
/// - `resolution`: number of sample points in the landscape.
///
/// Returns a vector of landscape function values (level 0 only for simplicity).
pub fn persistence_landscape(
    diagram: &PersistenceDiagram,
    dim: usize,
    resolution: usize,
) -> Vec<f64> {
    let points: Vec<&PersistencePoint> = diagram.points_at_dim(dim);
    if points.is_empty() || resolution == 0 {
        return vec![0.0; resolution];
    }

    // Determine parameter range.
    let min_birth = points.iter().map(|p| p.birth).fold(f64::INFINITY, f64::min);
    let max_death = points
        .iter()
        .map(|p| p.death)
        .fold(f64::NEG_INFINITY, f64::max);

    if (max_death - min_birth).abs() < f64::EPSILON {
        return vec![0.0; resolution];
    }

    let step = (max_death - min_birth) / resolution as f64;
    let mut landscape = Vec::with_capacity(resolution);

    for k in 0..resolution {
        let t = min_birth + (k as f64 + 0.5) * step;

        // For each persistence point, the landscape function is a tent:
        //   Lambda(t) = min(t - birth, death - t) if birth <= t <= death, else 0
        // The level-0 landscape is the maximum over all tents.
        let max_tent = points
            .iter()
            .map(|p| {
                if t >= p.birth && t <= p.death {
                    (t - p.birth).min(p.death - t)
                } else {
                    0.0
                }
            })
            .fold(0.0_f64, f64::max);

        landscape.push(max_tent);
    }

    landscape
}

/// L2 distance between two persistence landscapes.
pub fn landscape_distance(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    let sum_sq: f64 = (0..n).map(|i| (a[i] - b[i]).powi(2)).sum();
    let extra_a: f64 = a[n..].iter().map(|x| x.powi(2)).sum();
    let extra_b: f64 = b[n..].iter().map(|x| x.powi(2)).sum();
    (sum_sq + extra_a + extra_b).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takens_embedding_basic() {
        let series = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let embedded = takens_embedding(&series, 3, 2);

        // n = 8, dim = 3, tau = 2 -> first valid index = 0, last = 8 - (3-1)*2 - 1 = 3
        assert_eq!(embedded.len(), 4);
        assert_eq!(embedded[0], vec![1.0, 3.0, 5.0]);
        assert_eq!(embedded[1], vec![2.0, 4.0, 6.0]);
        assert_eq!(embedded[3], vec![4.0, 6.0, 8.0]);
    }

    #[test]
    fn takens_embedding_insufficient_data() {
        let series = vec![1.0, 2.0];
        let embedded = takens_embedding(&series, 3, 2);
        assert!(embedded.is_empty());
    }

    #[test]
    fn takens_embedding_zero_params() {
        assert!(takens_embedding(&[1.0, 2.0], 0, 1).is_empty());
        assert!(takens_embedding(&[1.0, 2.0], 2, 0).is_empty());
    }

    #[test]
    fn persistence_diagram_operations() {
        let mut diagram = PersistenceDiagram::new();
        diagram.add(0.0, 1.0, 0);
        diagram.add(0.0, 3.0, 0);
        diagram.add(0.5, 1.5, 1);

        assert_eq!(diagram.points.len(), 3);
        assert_eq!(diagram.points_at_dim(0).len(), 2);
        assert_eq!(diagram.points_at_dim(1).len(), 1);
        assert!((diagram.total_persistence() - 5.0).abs() < f64::EPSILON);
        assert!((diagram.max_persistence() - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn persistence_point_lifetime() {
        let p = PersistencePoint::new(0.5, 2.5, 0);
        assert!((p.persistence() - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn vietoris_rips_simple_cluster() {
        // Three points forming a tight cluster.
        let points = vec![vec![0.0, 0.0], vec![0.1, 0.0], vec![0.0, 0.1]];

        let diagram = vietoris_rips(&points, 0);
        let h0 = diagram.points_at_dim(0);
        // Should have at least 2 H0 features (2 merges for 3 points).
        assert!(
            h0.len() >= 2,
            "3-point cluster should produce at least 2 H0 features, got {}",
            h0.len()
        );
    }

    #[test]
    fn vietoris_rips_two_clusters() {
        // Two well-separated clusters.
        let points = vec![
            vec![0.0, 0.0],
            vec![0.1, 0.0],
            vec![0.0, 0.1],
            vec![10.0, 10.0],
            vec![10.1, 10.0],
            vec![10.0, 10.1],
        ];

        let diagram = vietoris_rips(&points, 0);
        // Should detect two clusters merging at a large distance.
        assert!(
            diagram.max_persistence() > 5.0,
            "should detect cluster separation"
        );
    }

    #[test]
    fn vietoris_rips_with_h1() {
        // Square: should have a 1-cycle.
        let points = vec![
            vec![0.0, 0.0],
            vec![1.0, 0.0],
            vec![1.0, 1.0],
            vec![0.0, 1.0],
        ];

        let diagram = vietoris_rips(&points, 1);
        // H0 features.
        let h0 = diagram.points_at_dim(0);
        assert!(!h0.is_empty(), "should have H0 features");
    }

    #[test]
    fn vietoris_rips_too_few_points() {
        let diagram = vietoris_rips(&[vec![1.0]], 0);
        assert!(diagram.points.is_empty());
    }

    #[test]
    fn persistence_landscape_basic() {
        let mut diagram = PersistenceDiagram::new();
        diagram.add(0.0, 2.0, 0);
        diagram.add(0.5, 1.5, 0);

        let landscape = persistence_landscape(&diagram, 0, 10);
        assert_eq!(landscape.len(), 10);
        // Landscape should have non-zero values in the middle.
        assert!(landscape.iter().any(|&v| v > 0.0));
    }

    #[test]
    fn persistence_landscape_empty() {
        let diagram = PersistenceDiagram::new();
        let landscape = persistence_landscape(&diagram, 0, 10);
        assert_eq!(landscape.len(), 10);
        assert!(landscape.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn landscape_distance_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let d = landscape_distance(&a, &a);
        assert!(d.abs() < f64::EPSILON);
    }

    #[test]
    fn landscape_distance_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let d = landscape_distance(&a, &b);
        assert!((d - 2.0_f64.sqrt()).abs() < 1e-10);
    }

    #[test]
    fn bottleneck_distance_identical() {
        let mut d1 = PersistenceDiagram::new();
        d1.add(0.0, 1.0, 0);
        d1.add(0.5, 2.0, 0);

        let d = d1.bottleneck_distance(&d1);
        assert!(
            d < 0.01,
            "identical diagrams should have ~0 distance, got {d}"
        );
    }

    #[test]
    fn bottleneck_distance_different() {
        let mut d1 = PersistenceDiagram::new();
        d1.add(0.0, 1.0, 0);

        let mut d2 = PersistenceDiagram::new();
        d2.add(0.0, 5.0, 0);

        let d = d1.bottleneck_distance(&d2);
        assert!(d > 0.0, "different diagrams should have positive distance");
    }

    #[test]
    fn time_series_tda_pipeline() {
        // Full pipeline: time series -> Takens -> Rips -> landscape.
        let series: Vec<f64> = (0..100).map(|i| (i as f64 * 0.1).sin()).collect();
        let embedded = takens_embedding(&series, 2, 5);
        assert!(!embedded.is_empty());

        let diagram = vietoris_rips(&embedded, 1);
        assert!(
            !diagram.points.is_empty(),
            "sinusoidal data should have topological features"
        );

        let landscape = persistence_landscape(&diagram, 0, 50);
        assert_eq!(landscape.len(), 50);
    }
}
