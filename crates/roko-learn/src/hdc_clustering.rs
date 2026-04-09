//! K-medoids clustering over 10,240-bit [`HdcVector`]s.
//!
//! Provides a straightforward partitioning-around-medoids (PAM) implementation
//! that uses [`HdcVector::similarity`] as the (dis)similarity metric. The
//! algorithm converts similarity to distance via `d(a,b) = 1 - similarity(a,b)`
//! so that identical vectors have distance 0.
//!
//! # Algorithm
//!
//! 1. **Initialise** — greedy farthest-first seeding: pick the point closest
//!    to the global centroid (lowest total distance) as the first medoid, then
//!    iteratively add the point maximising its minimum distance to all existing
//!    medoids.
//! 2. **Assign** — each point goes to the nearest medoid.
//! 3. **Update** — for each cluster, the member minimising total intra-cluster
//!    distance becomes the new medoid.
//! 4. Repeat 2-3 until medoids stabilise or `max_iterations` is reached.
//!
//! # Example
//!
//! ```
//! use bardo_primitives::HdcVector;
//! use roko_learn::hdc_clustering::{k_medoids, KMedoidsConfig};
//!
//! // Three well-separated seed vectors.
//! let seeds: Vec<HdcVector> = (0..30)
//!     .map(|i| HdcVector::from_seed(format!("group-{}", i / 10).as_bytes()))
//!     .collect();
//!
//! let result = k_medoids(&seeds, &KMedoidsConfig { k: 3, max_iterations: 50 });
//! assert_eq!(result.clusters.len(), 3);
//! ```

use bardo_primitives::HdcVector;

/// Configuration for k-medoids clustering.
#[derive(Debug, Clone)]
pub struct KMedoidsConfig {
    /// Number of clusters to form.
    pub k: usize,
    /// Maximum assignment-update iterations before stopping.
    pub max_iterations: usize,
}

impl Default for KMedoidsConfig {
    fn default() -> Self {
        Self {
            k: 3,
            max_iterations: 100,
        }
    }
}

/// A single cluster produced by [`k_medoids`].
#[derive(Debug, Clone)]
pub struct HdcCluster {
    /// Index of the medoid in the original input slice.
    pub medoid_index: usize,
    /// The medoid vector itself (convenience copy).
    pub medoid: HdcVector,
    /// Indices of all members (including the medoid) in the original input.
    pub members: Vec<usize>,
}

/// Result of a [`k_medoids`] run.
#[derive(Debug, Clone)]
pub struct ClusterResult {
    /// The discovered clusters, one per active medoid.
    pub clusters: Vec<HdcCluster>,
    /// Number of assign-update iterations executed.
    pub iterations: usize,
    /// Whether the algorithm converged (medoids stopped changing) before
    /// hitting `max_iterations`.
    pub converged: bool,
}

/// Run k-medoids clustering over a slice of [`HdcVector`]s.
///
/// Returns an empty [`ClusterResult`] when `vectors` is empty or `k == 0`.
/// If `k >= vectors.len()`, every point becomes its own medoid.
pub fn k_medoids(vectors: &[HdcVector], config: &KMedoidsConfig) -> ClusterResult {
    let n = vectors.len();
    let k = config.k;

    if n == 0 || k == 0 {
        return ClusterResult {
            clusters: Vec::new(),
            iterations: 0,
            converged: true,
        };
    }

    let k = k.min(n);

    // --- Precompute pairwise distance matrix (upper-triangle; symmetric). ---
    // Distance = 1.0 - similarity. Stored as full n×n for O(1) lookup.
    let dist = precompute_distances(vectors);

    // --- BUILD phase: greedy farthest-first seeding. ---
    let mut medoid_indices = seed_medoids(&dist, n, k);

    // --- Alternating assign / update loop. ---
    let mut assignments = vec![0usize; n];
    let mut converged = false;
    let mut iterations = 0;

    for _ in 0..config.max_iterations {
        iterations += 1;

        // Assign each point to the nearest medoid.
        assign(&dist, &medoid_indices, &mut assignments);

        // Update medoids: within each cluster pick the member that minimises
        // total distance to every other member.
        let mut changed = false;
        for (cluster_id, medoid_idx) in medoid_indices.iter_mut().enumerate() {
            let new_medoid = find_best_medoid(&dist, &assignments, cluster_id);
            if new_medoid != *medoid_idx {
                *medoid_idx = new_medoid;
                changed = true;
            }
        }

        if !changed {
            converged = true;
            break;
        }
    }

    // Final assignment after the last update.
    assign(&dist, &medoid_indices, &mut assignments);

    // Build output clusters.
    let mut clusters: Vec<HdcCluster> = medoid_indices
        .iter()
        .map(|&mi| HdcCluster {
            medoid_index: mi,
            medoid: vectors[mi],
            members: Vec::new(),
        })
        .collect();

    for (point_idx, &cluster_id) in assignments.iter().enumerate() {
        clusters[cluster_id].members.push(point_idx);
    }

    ClusterResult {
        clusters,
        iterations,
        converged,
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Flat n×n distance matrix stored in row-major order.
fn precompute_distances(vectors: &[HdcVector]) -> Vec<f32> {
    let n = vectors.len();
    let mut dist = vec![0.0_f32; n * n];
    for i in 0..n {
        for j in (i + 1)..n {
            let d = 1.0 - vectors[i].similarity(&vectors[j]);
            dist[i * n + j] = d;
            dist[j * n + i] = d;
        }
    }
    dist
}

/// Greedy farthest-first medoid seeding.
///
/// First medoid: the point with the smallest total distance to all others
/// (the "most central" point). Subsequent medoids: the point whose minimum
/// distance to any already-chosen medoid is largest.
fn seed_medoids(dist: &[f32], n: usize, k: usize) -> Vec<usize> {
    // First medoid: minimise total distance.
    let first = (0..n)
        .min_by(|&a, &b| {
            let sum_a: f32 = (0..n).map(|j| dist[a * n + j]).sum();
            let sum_b: f32 = (0..n).map(|j| dist[b * n + j]).sum();
            sum_a
                .partial_cmp(&sum_b)
                .unwrap_or(core::cmp::Ordering::Equal)
        })
        .unwrap_or(0);

    let mut medoids = vec![first];
    let mut min_dist_to_medoid = vec![f32::MAX; n];

    // Update min distances for the first medoid.
    for i in 0..n {
        min_dist_to_medoid[i] = dist[i * n + first];
    }

    while medoids.len() < k {
        // Pick the point farthest from any existing medoid.
        let next = (0..n)
            .filter(|i| !medoids.contains(i))
            .max_by(|&a, &b| {
                min_dist_to_medoid[a]
                    .partial_cmp(&min_dist_to_medoid[b])
                    .unwrap_or(core::cmp::Ordering::Equal)
            })
            .unwrap_or(0);

        medoids.push(next);

        // Update min distances.
        for i in 0..n {
            let d = dist[i * n + next];
            if d < min_dist_to_medoid[i] {
                min_dist_to_medoid[i] = d;
            }
        }
    }

    medoids
}

/// Assign every point to its nearest medoid.
fn assign(dist: &[f32], medoids: &[usize], assignments: &mut [usize]) {
    let n = assignments.len();
    for i in 0..n {
        let mut best_cluster = 0;
        let mut best_dist = f32::MAX;
        for (ci, &mi) in medoids.iter().enumerate() {
            let d = dist[i * n + mi];
            if d < best_dist {
                best_dist = d;
                best_cluster = ci;
            }
        }
        assignments[i] = best_cluster;
    }
}

/// Within a cluster, find the member that minimises total distance to all
/// other members of the same cluster.
fn find_best_medoid(dist: &[f32], assignments: &[usize], cluster_id: usize) -> usize {
    let n = assignments.len();
    let members: Vec<usize> = (0..n).filter(|&i| assignments[i] == cluster_id).collect();

    if members.is_empty() {
        return 0;
    }

    members
        .iter()
        .copied()
        .min_by(|&a, &b| {
            let cost_a: f32 = members.iter().map(|&m| dist[a * n + m]).sum();
            let cost_b: f32 = members.iter().map(|&m| dist[b * n + m]).sum();
            cost_a
                .partial_cmp(&cost_b)
                .unwrap_or(core::cmp::Ordering::Equal)
        })
        .unwrap_or(members[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use bardo_primitives::HdcVector;

    #[test]
    fn empty_input_returns_empty() {
        let result = k_medoids(
            &[],
            &KMedoidsConfig {
                k: 3,
                max_iterations: 10,
            },
        );
        assert!(result.clusters.is_empty());
        assert_eq!(result.iterations, 0);
        assert!(result.converged);
    }

    #[test]
    fn k_zero_returns_empty() {
        let vecs = vec![HdcVector::from_seed(b"a")];
        let result = k_medoids(
            &vecs,
            &KMedoidsConfig {
                k: 0,
                max_iterations: 10,
            },
        );
        assert!(result.clusters.is_empty());
        assert!(result.converged);
    }

    #[test]
    fn single_vector_single_cluster() {
        let vecs = vec![HdcVector::from_seed(b"only")];
        let result = k_medoids(
            &vecs,
            &KMedoidsConfig {
                k: 1,
                max_iterations: 10,
            },
        );
        assert_eq!(result.clusters.len(), 1);
        assert_eq!(result.clusters[0].members.len(), 1);
        assert_eq!(result.clusters[0].members[0], 0);
        assert!(result.converged);
    }

    #[test]
    fn k_exceeds_n_caps_to_n() {
        let vecs: Vec<HdcVector> = (0..4)
            .map(|i| HdcVector::from_seed(format!("pt-{i}").as_bytes()))
            .collect();
        let result = k_medoids(
            &vecs,
            &KMedoidsConfig {
                k: 100,
                max_iterations: 50,
            },
        );
        // Should have exactly 4 clusters (one per point).
        assert_eq!(result.clusters.len(), 4);
        for c in &result.clusters {
            assert_eq!(c.members.len(), 1);
        }
    }

    #[test]
    fn identical_vectors_cluster_together() {
        // 10 copies of the same vector should all end up in 1 cluster when k=1.
        let v = HdcVector::from_seed(b"same");
        let vecs = vec![v; 10];
        let result = k_medoids(
            &vecs,
            &KMedoidsConfig {
                k: 1,
                max_iterations: 10,
            },
        );
        assert_eq!(result.clusters.len(), 1);
        assert_eq!(result.clusters[0].members.len(), 10);
        assert!(result.converged);
    }

    #[test]
    fn synthetic_three_cluster_recovery() {
        // Build 3 tight groups of 10 vectors each, well-separated.
        // Group A: seeds "alpha-0" .. "alpha-9"  bundled around "alpha"
        // Group B: seeds "bravo-0" .. "bravo-9"  bundled around "bravo"
        // Group C: seeds "charlie-0".."charlie-9" bundled around "charlie"
        //
        // Each group member is constructed by binding a common group seed with a
        // per-member noise seed, so members within a group share structure.
        let group_seeds = [b"alpha".as_slice(), b"bravo", b"charlie"];
        let mut vecs = Vec::new();
        let mut expected_groups: Vec<Vec<usize>> = vec![Vec::new(); 3];

        for (gi, &gs) in group_seeds.iter().enumerate() {
            let base = HdcVector::from_seed(gs);
            for mi in 0..10 {
                // Create a member by bundling the base with a small perturbation.
                // Use bundle of [base, base, noise] so base dominates (2 vs 1).
                let noise_seed = format!("{}-noise-{mi}", String::from_utf8_lossy(gs));
                let noise = HdcVector::from_seed(noise_seed.as_bytes());
                let member = HdcVector::bundle(&[&base, &base, &noise]);
                let idx = vecs.len();
                vecs.push(member);
                expected_groups[gi].push(idx);
            }
        }

        assert_eq!(vecs.len(), 30);

        // Verify groups are internally similar and externally dissimilar.
        let intra_sim = vecs[expected_groups[0][0]].similarity(&vecs[expected_groups[0][1]]);
        let inter_sim = vecs[expected_groups[0][0]].similarity(&vecs[expected_groups[1][0]]);
        assert!(
            intra_sim > inter_sim,
            "intra-group similarity ({intra_sim}) should exceed inter-group ({inter_sim})"
        );

        let result = k_medoids(
            &vecs,
            &KMedoidsConfig {
                k: 3,
                max_iterations: 50,
            },
        );

        assert_eq!(result.clusters.len(), 3);

        // Every cluster should have exactly 10 members.
        let mut sizes: Vec<usize> = result.clusters.iter().map(|c| c.members.len()).collect();
        sizes.sort();
        assert_eq!(sizes, vec![10, 10, 10]);

        // Each cluster should map to exactly one expected group. Build a
        // confusion check: for each cluster, count how many members come from
        // each expected group. The dominant group should cover all 10.
        for cluster in &result.clusters {
            let mut group_counts = [0usize; 3];
            for &member in &cluster.members {
                for (gi, group) in expected_groups.iter().enumerate() {
                    if group.contains(&member) {
                        group_counts[gi] += 1;
                    }
                }
            }
            let max_count = *group_counts.iter().max().unwrap_or(&0);
            assert_eq!(
                max_count, 10,
                "cluster should perfectly recover one group, got counts {group_counts:?}"
            );
        }
    }

    #[test]
    fn similar_vectors_exceed_threshold_and_cluster_together() {
        let base = HdcVector::from_seed(b"similar-base");
        let similar = HdcVector::bundle(&[
            &base,
            &base,
            &HdcVector::from_seed(b"similar-base-noise"),
        ]);

        let similarity = base.similarity(&similar);
        assert!(
            similarity > 0.7,
            "expected similar vectors to exceed the 0.7 threshold, got {similarity}"
        );

        let result = k_medoids(
            &[base, similar],
            &KMedoidsConfig {
                k: 1,
                max_iterations: 10,
            },
        );

        assert_eq!(result.clusters.len(), 1);
        assert_eq!(result.clusters[0].members, vec![0, 1]);
    }

    #[test]
    fn convergence_flag_set_when_stable() {
        // With identical vectors, should converge in 1 iteration.
        let v = HdcVector::from_seed(b"converge");
        let vecs = vec![v; 5];
        let result = k_medoids(
            &vecs,
            &KMedoidsConfig {
                k: 2,
                max_iterations: 100,
            },
        );
        assert!(result.converged);
        assert!(result.iterations <= 2);
    }

    #[test]
    fn deterministic_across_runs() {
        let vecs: Vec<HdcVector> = (0..20)
            .map(|i| HdcVector::from_seed(format!("det-{i}").as_bytes()))
            .collect();
        let cfg = KMedoidsConfig {
            k: 4,
            max_iterations: 50,
        };
        let r1 = k_medoids(&vecs, &cfg);
        let r2 = k_medoids(&vecs, &cfg);
        assert_eq!(r1.clusters.len(), r2.clusters.len());
        for (c1, c2) in r1.clusters.iter().zip(r2.clusters.iter()) {
            assert_eq!(c1.medoid_index, c2.medoid_index);
            assert_eq!(c1.members, c2.members);
        }
    }

    #[test]
    fn all_points_assigned_exactly_once() {
        let vecs: Vec<HdcVector> = (0..15)
            .map(|i| HdcVector::from_seed(format!("assign-{i}").as_bytes()))
            .collect();
        let result = k_medoids(
            &vecs,
            &KMedoidsConfig {
                k: 3,
                max_iterations: 50,
            },
        );
        let mut all_members: Vec<usize> = result
            .clusters
            .iter()
            .flat_map(|c| c.members.iter().copied())
            .collect();
        all_members.sort();
        let expected: Vec<usize> = (0..15).collect();
        assert_eq!(all_members, expected);
    }
}
