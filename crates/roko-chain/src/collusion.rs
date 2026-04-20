//! Collusion ring detection via assignment graph analysis.
//!
//! Per spec (docs/08-chain/14-reputation-system-7-domain.md lines 250-302):
//! Detects collusion rings by analyzing mutual assignment patterns in the
//! job marketplace. When agents repeatedly assign jobs to each other at
//! abnormally high rates, they form a clique in the assignment graph.
//!
//! ## Algorithm
//!
//! 1. Build an assignment graph: edge A→B means A assigned a job to B.
//! 2. Compute mutual assignment ratio for each pair: min(A→B, B→A) / max(A→B, B→A).
//! 3. Filter edges where mutual ratio exceeds threshold (default 0.5).
//! 4. Find cliques (fully connected subgraphs) via DFS.
//! 5. Cliques of size >= 3 with mutual ratios above threshold are flagged.
//!
//! ## Penalty
//!
//! All members of a detected collusion ring receive feedback weight dilution
//! (-50% for 30 days) via `ReputationViolation::Collusion`. Their scores
//! are NOT directly slashed — only their influence as raters is reduced.

use std::collections::{HashMap, HashSet};

use crate::phase2::u256;

/// An assignment edge in the job marketplace graph.
#[derive(Debug, Clone)]
pub struct AssignmentEdge {
    /// Job poster (assigner).
    pub from: u256,
    /// Job executor (assignee).
    pub to: u256,
    /// Block number when assignment was made.
    pub block: u64,
}

/// Configuration for collusion detection.
#[derive(Debug, Clone, PartialEq)]
pub struct CollusionConfig {
    /// Minimum mutual assignment ratio to consider suspicious (0.0-1.0).
    /// If A assigned B 5 times and B assigned A 4 times, ratio = 4/5 = 0.8.
    pub mutual_ratio_threshold: f64,
    /// Minimum assignments between a pair to be considered (filters low-volume noise).
    pub min_assignments_per_pair: u32,
    /// Minimum clique size to flag as collusion ring.
    pub min_clique_size: usize,
    /// How far back (in blocks) to consider assignments. 0 = all history.
    pub lookback_blocks: u64,
}

impl Default for CollusionConfig {
    fn default() -> Self {
        Self {
            mutual_ratio_threshold: 0.5,
            min_assignments_per_pair: 3,
            min_clique_size: 3,
            lookback_blocks: 0,
        }
    }
}

/// A detected collusion ring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollusionRing {
    /// Passport IDs of agents in the ring.
    pub members: Vec<u256>,
    /// Size of the ring.
    pub size: usize,
}

/// Result of running collusion detection.
#[derive(Debug, Clone)]
pub struct CollusionReport {
    /// Detected collusion rings.
    pub rings: Vec<CollusionRing>,
    /// Number of suspicious pairs found (before clique analysis).
    pub suspicious_pairs: usize,
    /// Total agents analyzed.
    pub agents_analyzed: usize,
    /// Total assignments analyzed.
    pub assignments_analyzed: usize,
}

/// Collusion detector that analyzes assignment patterns.
#[derive(Debug, Clone)]
pub struct CollusionDetector {
    /// Configuration.
    pub config: CollusionConfig,
    /// Recorded assignment edges.
    edges: Vec<AssignmentEdge>,
}

impl CollusionDetector {
    /// Create a new collusion detector.
    pub fn new(config: CollusionConfig) -> Self {
        Self {
            config,
            edges: Vec::new(),
        }
    }

    /// Record an assignment edge.
    pub fn record_assignment(&mut self, edge: AssignmentEdge) {
        self.edges.push(edge);
    }

    /// Number of recorded assignments.
    pub fn assignment_count(&self) -> usize {
        self.edges.len()
    }

    /// Run collusion detection, returning a report of findings.
    pub fn detect(&self) -> CollusionReport {
        let current_block = self.edges.iter().map(|e| e.block).max().unwrap_or(0);

        // Filter by lookback window.
        let active: Vec<&AssignmentEdge> = if self.config.lookback_blocks > 0 {
            let min_block = current_block.saturating_sub(self.config.lookback_blocks);
            self.edges.iter().filter(|e| e.block >= min_block).collect()
        } else {
            self.edges.iter().collect()
        };

        // Step 1: Count assignments per directed pair.
        let mut pair_counts: HashMap<(u256, u256), u32> = HashMap::new();
        let mut all_agents: HashSet<u256> = HashSet::new();
        for edge in &active {
            *pair_counts.entry((edge.from, edge.to)).or_default() += 1;
            all_agents.insert(edge.from);
            all_agents.insert(edge.to);
        }

        // Step 2: Find suspicious mutual pairs.
        let mut suspicious: HashSet<(u256, u256)> = HashSet::new();
        for (&(a, b), &count_ab) in &pair_counts {
            if a >= b {
                continue; // Only check each pair once (canonical order).
            }
            let count_ba = pair_counts.get(&(b, a)).copied().unwrap_or(0);
            let total = count_ab + count_ba;
            let min_count = count_ab.min(count_ba);
            let max_count = count_ab.max(count_ba);

            if total < self.config.min_assignments_per_pair {
                continue;
            }

            if max_count > 0 {
                let ratio = min_count as f64 / max_count as f64;
                if ratio >= self.config.mutual_ratio_threshold {
                    suspicious.insert((a.min(b), a.max(b)));
                }
            }
        }

        let suspicious_pairs = suspicious.len();

        // Step 3: Build adjacency from suspicious pairs and find cliques.
        let mut adjacency: HashMap<u256, HashSet<u256>> = HashMap::new();
        for &(a, b) in &suspicious {
            adjacency.entry(a).or_default().insert(b);
            adjacency.entry(b).or_default().insert(a);
        }

        let rings = find_cliques(&adjacency, self.config.min_clique_size);

        CollusionReport {
            rings,
            suspicious_pairs,
            agents_analyzed: all_agents.len(),
            assignments_analyzed: active.len(),
        }
    }
}

impl Default for CollusionDetector {
    fn default() -> Self {
        Self::new(CollusionConfig::default())
    }
}

/// Find all maximal cliques of at least `min_size` in the adjacency graph.
///
/// Uses Bron-Kerbosch algorithm with pivoting for efficiency.
fn find_cliques(adjacency: &HashMap<u256, HashSet<u256>>, min_size: usize) -> Vec<CollusionRing> {
    let mut results = Vec::new();
    let all_vertices: HashSet<u256> = adjacency.keys().copied().collect();

    let mut p = all_vertices;
    bron_kerbosch(
        &HashSet::new(),
        &mut p,
        &mut HashSet::new(),
        adjacency,
        min_size,
        &mut results,
    );

    results
}

/// Bron-Kerbosch algorithm with pivoting for maximal clique enumeration.
fn bron_kerbosch(
    r: &HashSet<u256>,
    p: &mut HashSet<u256>,
    x: &mut HashSet<u256>,
    adj: &HashMap<u256, HashSet<u256>>,
    min_size: usize,
    results: &mut Vec<CollusionRing>,
) {
    if p.is_empty() && x.is_empty() {
        if r.len() >= min_size {
            let mut members: Vec<u256> = r.iter().copied().collect();
            members.sort_unstable();
            let size = members.len();
            results.push(CollusionRing { members, size });
        }
        return;
    }

    // Pick a pivot vertex to minimize branching.
    let pivot = p
        .union(x)
        .max_by_key(|v| {
            adj.get(v)
                .map_or(0, |neighbors| p.intersection(neighbors).count())
        })
        .copied();

    let Some(pivot) = pivot else {
        return;
    };

    let pivot_neighbors = adj.get(&pivot).cloned().unwrap_or_default();
    let candidates: Vec<u256> = p.difference(&pivot_neighbors).copied().collect();

    for v in candidates {
        let v_neighbors = adj.get(&v).cloned().unwrap_or_default();
        let mut new_r = r.clone();
        new_r.insert(v);

        let mut new_p: HashSet<u256> = p.intersection(&v_neighbors).copied().collect();
        let mut new_x: HashSet<u256> = x.intersection(&v_neighbors).copied().collect();

        bron_kerbosch(&new_r, &mut new_p, &mut new_x, adj, min_size, results);

        p.remove(&v);
        x.insert(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(from: u256, to: u256, block: u64) -> AssignmentEdge {
        AssignmentEdge { from, to, block }
    }

    #[test]
    fn empty_graph_detects_nothing() {
        let detector = CollusionDetector::default();
        let report = detector.detect();
        assert!(report.rings.is_empty());
        assert_eq!(report.suspicious_pairs, 0);
    }

    #[test]
    fn detects_three_agent_ring() {
        let mut detector = CollusionDetector::new(CollusionConfig {
            min_assignments_per_pair: 2,
            mutual_ratio_threshold: 0.4,
            min_clique_size: 3,
            ..Default::default()
        });

        // Three agents assigning to each other: 1↔2, 2↔3, 1↔3.
        for block in 0..5 {
            detector.record_assignment(edge(1, 2, block));
            detector.record_assignment(edge(2, 1, block));
            detector.record_assignment(edge(2, 3, block));
            detector.record_assignment(edge(3, 2, block));
            detector.record_assignment(edge(1, 3, block));
            detector.record_assignment(edge(3, 1, block));
        }

        let report = detector.detect();
        assert!(
            !report.rings.is_empty(),
            "should detect the 3-agent ring, got {} rings",
            report.rings.len()
        );
        assert_eq!(report.rings[0].size, 3);
        assert_eq!(report.suspicious_pairs, 3);
    }

    #[test]
    fn ignores_low_volume_pairs() {
        let mut detector = CollusionDetector::new(CollusionConfig {
            min_assignments_per_pair: 5,
            ..Default::default()
        });

        // Only 2 assignments per pair — below threshold.
        detector.record_assignment(edge(1, 2, 0));
        detector.record_assignment(edge(2, 1, 1));

        let report = detector.detect();
        assert_eq!(report.suspicious_pairs, 0);
    }

    #[test]
    fn ignores_one_directional_assignments() {
        let mut detector = CollusionDetector::new(CollusionConfig {
            min_assignments_per_pair: 2,
            mutual_ratio_threshold: 0.4,
            ..Default::default()
        });

        // Agent 1 always assigns to 2, but 2 never assigns to 1.
        for block in 0..10 {
            detector.record_assignment(edge(1, 2, block));
        }

        let report = detector.detect();
        assert_eq!(
            report.suspicious_pairs, 0,
            "one-directional shouldn't be suspicious"
        );
    }

    #[test]
    fn pair_below_clique_size_not_reported() {
        let mut detector = CollusionDetector::new(CollusionConfig {
            min_assignments_per_pair: 2,
            mutual_ratio_threshold: 0.4,
            min_clique_size: 3,
            ..Default::default()
        });

        // Two agents in a mutual pair (ring of 2), but min_clique_size is 3.
        for block in 0..5 {
            detector.record_assignment(edge(1, 2, block));
            detector.record_assignment(edge(2, 1, block));
        }

        let report = detector.detect();
        assert!(report.suspicious_pairs > 0, "pair should be suspicious");
        assert!(
            report.rings.is_empty(),
            "ring of 2 should not be reported when min_clique=3"
        );
    }

    #[test]
    fn lookback_window_filters_old() {
        let mut detector = CollusionDetector::new(CollusionConfig {
            min_assignments_per_pair: 2,
            mutual_ratio_threshold: 0.4,
            min_clique_size: 2,
            lookback_blocks: 10,
        });

        // Old assignments (outside window).
        for block in 0..5 {
            detector.record_assignment(edge(1, 2, block));
            detector.record_assignment(edge(2, 1, block));
        }
        // Recent assignment (inside window).
        detector.record_assignment(edge(3, 4, 100));

        let report = detector.detect();
        // Old pair should be filtered by lookback (max_block=100, window=10 → min=90).
        assert_eq!(
            report.suspicious_pairs, 0,
            "old assignments should be filtered"
        );
    }
}
